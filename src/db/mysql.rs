//! MySQL / MariaDB via mysql_async, text protocol, streaming rows one at a time.
//! Cancel opens a second connection and issues KILL QUERY for this session's id.

use super::tunnel::Tunnel;
use super::{Cell, ColKind, ColumnMeta, ConnectParams, DbError, Sink, StmtDone};
use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, SslOpts, Value};

pub struct MySession {
    conn: Conn,
    opts: Opts,
    pub version: String,
    _tunnel: Option<Tunnel>,
}

#[derive(Clone)]
pub struct MyCancel {
    opts: Opts,
    id: u32,
}

impl MyCancel {
    pub async fn cancel(&self) -> Result<(), DbError> {
        let mut c = Conn::new(self.opts.clone()).await.map_err(from_my)?;
        c.query_drop(format!("KILL QUERY {}", self.id)).await.map_err(from_my)?;
        let _ = c.disconnect().await;
        Ok(())
    }
}

pub fn from_my(e: mysql_async::Error) -> DbError {
    match e {
        mysql_async::Error::Server(s) => DbError { message: s.message.clone(), position: None, detail: Some(format!("error {} ({})", s.code, s.state)) },
        other => DbError::msg(other.to_string()),
    }
}

impl MySession {
    pub async fn connect(p: &ConnectParams, tunnel: Option<Tunnel>) -> Result<MySession, DbError> {
        let parts = &p.parts;
        let (host, port) = match &tunnel {
            Some(t) => ("127.0.0.1".to_string(), t.local_port),
            None => (if parts.host.is_empty() { "127.0.0.1".into() } else { parts.host.clone() }, parts.port_or_default()),
        };
        let user = if parts.user.is_empty() { "root".to_string() } else { parts.user.clone() };
        let mut b = OptsBuilder::default()
            .ip_or_hostname(host)
            .tcp_port(port)
            .user(Some(user))
            .pass(p.password.clone())
            .db_name(if parts.database.is_empty() { None } else { Some(parts.database.clone()) })
            .prefer_socket(false);
        let mode = parts.param("ssl-mode").or_else(|| parts.param("sslmode")).unwrap_or_default().to_ascii_lowercase();
        match mode.as_str() {
            "required" | "require" => b = b.ssl_opts(SslOpts::default().with_danger_accept_invalid_certs(true).with_danger_skip_domain_validation(true)),
            "verify_ca" | "verify-ca" => b = b.ssl_opts(SslOpts::default().with_danger_skip_domain_validation(true)),
            "verify_identity" | "verify-full" => b = b.ssl_opts(SslOpts::default()),
            _ => {}
        }
        let opts: Opts = b.into();
        let conn = Conn::new(opts.clone()).await.map_err(from_my)?;
        let (a, bb, c) = conn.server_version();
        let mut s = MySession { conn, opts, version: format!("MySQL {a}.{bb}.{c}"), _tunnel: tunnel };
        if let Ok(rows) = s.conn.query::<String, _>("select version()").await
            && let Some(v) = rows.first() {
                s.version = if v.to_lowercase().contains("mariadb") { format!("MariaDB {}", v.split('-').next().unwrap_or(v)) } else { format!("MySQL {v}") };
            }
        Ok(s)
    }

    pub fn canceller(&self) -> MyCancel {
        MyCancel { opts: self.opts.clone(), id: self.conn.id() }
    }

    pub async fn run(&mut self, sql: &str, sink: &mut dyn Sink) -> Result<StmtDone, DbError> {
        let mut result = self.conn.query_iter(sql).await.map_err(from_my)?;
        let mut done = StmtDone::default();
        let mut binary: Vec<bool> = Vec::new();
        if let Some(cols) = result.columns()
            && !cols.is_empty() {
                let metas: Vec<ColumnMeta> = cols
                    .iter()
                    .map(|c| {
                        let (type_name, kind) = describe(c.column_type(), c.flags(), c.character_set(), c.column_length());
                        ColumnMeta { name: c.name_str().to_string(), type_name, kind }
                    })
                    .collect();
                binary = metas.iter().map(|m| m.kind == ColKind::Bytes).collect();
                done.returned_rows = true;
                sink.columns(metas);
            }
        while let Some(row) = result.next().await.map_err(from_my)? {
            let vals = row.unwrap();
            let cells: Vec<Cell> = vals.into_iter().enumerate().map(|(i, v)| cell(v, binary.get(i) == Some(&true))).collect();
            done.rows_returned += 1;
            sink.row(cells);
            if done.rows_returned % 64 == 0 {
                sink.flush();
            }
        }
        if !done.returned_rows {
            done.rows_affected = Some(result.affected_rows());
        }
        let info = result.info().to_string();
        result.drop_result().await.map_err(from_my)?;
        if !info.is_empty() {
            sink.notice(info);
        }
        if self.conn.get_warnings() > 0
            && let Ok(rows) = self.conn.query::<(String, u32, String), _>("show warnings").await {
                for (lvl, code, msg) in rows {
                    sink.notice(format!("{}: {msg} ({code})", lvl.to_lowercase()));
                }
            }
        sink.flush();
        Ok(done)
    }
}

fn cell(v: Value, binary: bool) -> Cell {
    match v {
        Value::NULL => None,
        Value::Bytes(b) => Some(if binary {
            let mut s = String::with_capacity(2 + b.len() * 2);
            s.push_str("0x");
            for x in b {
                s.push_str(&format!("{x:02x}"));
            }
            s.into()
        } else {
            String::from_utf8_lossy(&b).into_owned().into()
        }),
        other => Some(other.as_sql(true).trim_matches('\'').to_string().into()),
    }
}

fn describe(t: ColumnType, flags: ColumnFlags, charset: u16, len: u32) -> (String, ColKind) {
    use ColumnType::*;
    let unsigned = flags.contains(ColumnFlags::UNSIGNED_FLAG);
    let u = |s: &str| if unsigned { format!("{s} unsigned") } else { s.to_string() };
    let binary = charset == 63;
    match t {
        MYSQL_TYPE_TINY if len == 1 => ("tinyint(1)".into(), ColKind::Int),
        MYSQL_TYPE_TINY => (u("tinyint"), ColKind::Int),
        MYSQL_TYPE_SHORT => (u("smallint"), ColKind::Int),
        MYSQL_TYPE_INT24 => (u("mediumint"), ColKind::Int),
        MYSQL_TYPE_LONG => (u("int"), ColKind::Int),
        MYSQL_TYPE_LONGLONG => (u("bigint"), ColKind::Int),
        MYSQL_TYPE_YEAR => ("year".into(), ColKind::Int),
        MYSQL_TYPE_FLOAT => ("float".into(), ColKind::Float),
        MYSQL_TYPE_DOUBLE => ("double".into(), ColKind::Float),
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => ("decimal".into(), ColKind::Decimal),
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => ("date".into(), ColKind::Date),
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => ("time".into(), ColKind::Time),
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => ("datetime".into(), ColKind::Timestamp),
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => ("timestamp".into(), ColKind::Timestamp),
        MYSQL_TYPE_JSON => ("json".into(), ColKind::Json),
        MYSQL_TYPE_BIT => ("bit".into(), ColKind::Bytes),
        MYSQL_TYPE_ENUM => ("enum".into(), ColKind::Text),
        MYSQL_TYPE_SET => ("set".into(), ColKind::Text),
        MYSQL_TYPE_GEOMETRY => ("geometry".into(), ColKind::Bytes),
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB => {
            if binary { ("blob".into(), ColKind::Bytes) } else { ("text".into(), ColKind::Text) }
        }
        MYSQL_TYPE_VAR_STRING | MYSQL_TYPE_VARCHAR => {
            if binary { ("varbinary".into(), ColKind::Bytes) } else { ("varchar".into(), ColKind::Text) }
        }
        MYSQL_TYPE_STRING => {
            if flags.contains(ColumnFlags::ENUM_FLAG) {
                ("enum".into(), ColKind::Text)
            } else if binary {
                ("binary".into(), ColKind::Bytes)
            } else {
                ("char".into(), ColKind::Text)
            }
        }
        _ => ("?".into(), ColKind::Text),
    }
}
