//! Postgres via tokio-postgres. Each statement is prepared first (to learn column types,
//! cheaply and without side effects), then run with the simple protocol so every type
//! arrives as text and rows stream in as the server sends them.

use super::tunnel::Tunnel;
use super::{Cell, ColKind, ColumnMeta, ConnectParams, DbError, Sink, StmtDone};
use futures::StreamExt;
use postgres_native_tls::MakeTlsConnector;
use tokio::sync::mpsc;
use tokio_postgres::{AsyncMessage, CancelToken, Client, Config, SimpleQueryMessage, config::SslMode};

pub struct PgSession {
    client: Client,
    notices: mpsc::UnboundedReceiver<String>,
    tls: MakeTlsConnector,
    pub version: String,
    in_tx: bool,
    _tunnel: Option<Tunnel>,
}

#[derive(Clone)]
pub struct PgCancel {
    token: CancelToken,
    tls: MakeTlsConnector,
}

impl PgCancel {
    pub async fn cancel(&self) -> Result<(), DbError> {
        self.token.cancel_query(self.tls.clone()).await.map_err(from_pg)
    }
}

pub fn from_pg(e: tokio_postgres::Error) -> DbError {
    if let Some(db) = e.as_db_error() {
        let position = match db.position() {
            Some(tokio_postgres::error::ErrorPosition::Original(p)) => Some((*p as usize).saturating_sub(1)),
            _ => None,
        };
        let mut detail = db.detail().map(str::to_string);
        if let Some(h) = db.hint() {
            detail = Some(match detail {
                Some(d) => format!("{d} · hint: {h}"),
                None => format!("hint: {h}"),
            });
        }
        return DbError { message: db.message().to_string(), position, detail };
    }
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        msg = format!("{msg}: {s}");
        src = s.source();
    }
    DbError::msg(msg)
}

fn tls_connector(mode: &str) -> Result<MakeTlsConnector, DbError> {
    let mut b = native_tls::TlsConnector::builder();
    if !matches!(mode, "verify-full" | "verify-ca") {
        // libpq semantics: require/prefer encrypt without verifying the certificate
        b.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
    } else if mode == "verify-ca" {
        b.danger_accept_invalid_hostnames(true);
    }
    let c = b.build().map_err(|e| DbError::msg(format!("TLS setup failed: {e}")))?;
    Ok(MakeTlsConnector::new(c))
}

impl PgSession {
    pub async fn connect(p: &ConnectParams, tunnel: Option<Tunnel>) -> Result<PgSession, DbError> {
        let parts = &p.parts;
        let mut cfg = Config::new();
        match &tunnel {
            Some(t) => {
                cfg.host("127.0.0.1").port(t.local_port);
            }
            None => {
                let host = if parts.host.is_empty() { default_host() } else { parts.host.clone() };
                cfg.host(&host).port(parts.port_or_default());
            }
        }
        let user = if parts.user.is_empty() { std::env::var("USER").unwrap_or_else(|_| "postgres".into()) } else { parts.user.clone() };
        cfg.user(&user);
        cfg.dbname(if parts.database.is_empty() { &user } else { &parts.database });
        if let Some(pw) = &p.password {
            cfg.password(pw);
        }
        cfg.application_name("zdb");
        let mode = parts.param("sslmode").unwrap_or_else(|| "prefer".into());
        cfg.ssl_mode(match mode.as_str() {
            "disable" | "allow" => SslMode::Disable,
            "require" | "verify-ca" | "verify-full" => SslMode::Require,
            _ => SslMode::Prefer,
        });
        let tls = tls_connector(&mode)?;
        let (client, mut conn) = cfg.connect(tls.clone()).await.map_err(from_pg)?;
        let (ntx, nrx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut stream = futures::stream::poll_fn(move |cx| conn.poll_message(cx));
            while let Some(m) = stream.next().await {
                match m {
                    Ok(AsyncMessage::Notice(n)) => {
                        let _ = ntx.send(format!("{}: {}", n.severity().to_lowercase(), n.message()));
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        let mut s = PgSession { client, notices: nrx, tls, version: String::new(), in_tx: false, _tunnel: tunnel };
        if let Ok(rows) = s.client.simple_query("show server_version").await {
            for m in rows {
                if let SimpleQueryMessage::Row(r) = m {
                    let v = r.get(0).unwrap_or("").to_string();
                    s.version = format!("PostgreSQL {}", v.split_whitespace().next().unwrap_or(&v));
                }
            }
        }
        Ok(s)
    }

    pub fn canceller(&self) -> PgCancel {
        PgCancel { token: self.client.cancel_token(), tls: self.tls.clone() }
    }

    pub fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    pub async fn run(&mut self, sql: &str, sink: &mut dyn Sink) -> Result<StmtDone, DbError> {
        let types: Option<Vec<ColumnMeta>> = match self.client.prepare(sql).await {
            Ok(st) => Some(
                st.columns()
                    .iter()
                    .map(|c| {
                        let t = c.type_().name().to_string();
                        ColumnMeta { name: c.name().to_string(), kind: kind_for(&t), type_name: t }
                    })
                    .collect(),
            ),
            // Inside a transaction a failed prepare has already aborted it, so report that error.
            Err(e) if self.in_tx => return Err(from_pg(e)),
            Err(_) => None,
        };
        let res = self.run_simple(sql, types, sink).await;
        while let Ok(n) = self.notices.try_recv() {
            sink.notice(n);
        }
        if res.is_ok() {
            match crate::sql::classify(sql).kind {
                crate::sql::Kind::Begin => self.in_tx = true,
                crate::sql::Kind::Commit | crate::sql::Kind::Rollback => self.in_tx = false,
                _ => {}
            }
        }
        res
    }

    async fn run_simple(&mut self, sql: &str, types: Option<Vec<ColumnMeta>>, sink: &mut dyn Sink) -> Result<StmtDone, DbError> {
        let stream = self.client.simple_query_raw(sql).await.map_err(from_pg)?;
        futures::pin_mut!(stream);
        let mut done = StmtDone::default();
        let mut bools: Vec<bool> = Vec::new();
        while let Some(msg) = stream.next().await {
            match msg.map_err(from_pg)? {
                SimpleQueryMessage::RowDescription(cols) => {
                    let metas = match &types {
                        Some(t) if t.len() == cols.len() => t.clone(),
                        _ => cols.iter().map(|c| ColumnMeta { name: c.name().to_string(), type_name: "text".into(), kind: ColKind::Text }).collect(),
                    };
                    bools = metas.iter().map(|m| m.kind == ColKind::Bool).collect();
                    done.returned_rows = true;
                    sink.columns(metas);
                }
                SimpleQueryMessage::Row(r) => {
                    let row: Vec<Cell> = (0..r.len())
                        .map(|i| {
                            r.get(i).map(|v| {
                                if bools.get(i) == Some(&true) {
                                    match v {
                                        "t" => "true".into(),
                                        "f" => "false".into(),
                                        _ => v.into(),
                                    }
                                } else {
                                    v.into()
                                }
                            })
                        })
                        .collect();
                    done.rows_returned += 1;
                    sink.row(row);
                    if done.rows_returned % 64 == 0 {
                        while let Ok(n) = self.notices.try_recv() {
                            sink.notice(n);
                        }
                        sink.flush();
                    }
                }
                SimpleQueryMessage::CommandComplete(n)
                    if !done.returned_rows => {
                        done.rows_affected = Some(n);
                    }
                _ => {}
            }
        }
        sink.flush();
        Ok(done)
    }
}

fn default_host() -> String {
    for d in ["/tmp", "/var/run/postgresql", "/run/postgresql"] {
        if std::path::Path::new(d).join(".s.PGSQL.5432").exists() {
            return d.to_string();
        }
    }
    "localhost".into()
}

pub fn kind_for(t: &str) -> ColKind {
    match t {
        "int2" | "int4" | "int8" | "oid" | "xid" => ColKind::Int,
        "float4" | "float8" => ColKind::Float,
        "numeric" | "money" => ColKind::Decimal,
        "bool" => ColKind::Bool,
        "json" | "jsonb" => ColKind::Json,
        "date" => ColKind::Date,
        "time" | "timetz" | "interval" => ColKind::Time,
        "timestamp" | "timestamptz" => ColKind::Timestamp,
        "uuid" => ColKind::Uuid,
        "bytea" => ColKind::Bytes,
        _ => ColKind::Text,
    }
}
