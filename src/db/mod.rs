//! Driver layer. The UI talks to `Session` and receives `DbEvent`s; Postgres and MySQL
//! differences (protocols, cancel, catalog queries, quoting) stay below this line.

pub mod mysql;
pub mod pg;
pub mod schema;
pub mod sqlite;
pub mod tunnel;

use crate::config::{DriverKind, UrlParts};
use std::time::Duration;

pub use schema::Schema;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColKind {
    Int,
    Float,
    Decimal,
    Bool,
    Text,
    Json,
    Date,
    Time,
    Timestamp,
    Uuid,
    Bytes,
}

impl ColKind {
    pub fn numeric(self) -> bool {
        matches!(self, ColKind::Int | ColKind::Float | ColKind::Decimal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
    pub kind: ColKind,
}

pub type Cell = Option<Box<str>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError {
    pub message: String,
    /// 0-based character offset into the SQL that was sent, when the server reports one.
    pub position: Option<usize>,
    pub detail: Option<String>,
}

impl DbError {
    pub fn msg(m: impl Into<String>) -> DbError {
        DbError { message: m.into(), position: None, detail: None }
    }
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Receives the output of one statement as it streams in.
pub trait Sink: Send {
    fn columns(&mut self, cols: Vec<ColumnMeta>);
    fn row(&mut self, row: Vec<Cell>);
    fn notice(&mut self, _msg: String) {}
    /// Called often; a sink can flush batches here.
    fn flush(&mut self) {}
}

#[derive(Debug, Clone, Default)]
pub struct StmtDone {
    pub rows_affected: Option<u64>,
    pub rows_returned: u64,
    pub returned_rows: bool,
}

pub struct ConnectParams {
    pub driver: DriverKind,
    pub parts: UrlParts,
    pub password: Option<String>,
    pub read_only: bool,
    pub ssh: Option<String>,
}

#[allow(clippy::large_enum_variant)] // one per tab; boxing buys nothing
pub enum Session {
    Pg(pg::PgSession),
    My(mysql::MySession),
    Sqlite(sqlite::SqliteSession),
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Canceller {
    Pg(pg::PgCancel),
    My(mysql::MyCancel),
    Sqlite(sqlite::SqliteCancel),
}

impl Canceller {
    pub async fn cancel(&self) -> Result<(), DbError> {
        match self {
            Canceller::Pg(c) => c.cancel().await,
            Canceller::My(c) => c.cancel().await,
            Canceller::Sqlite(c) => { c.interrupt(); Ok(()) },
        }
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

impl Session {
    pub async fn connect(p: &ConnectParams) -> Result<Session, DbError> {
        if p.driver == DriverKind::Sqlite {
            if p.ssh.as_ref().is_some_and(|s| !s.trim().is_empty()) { return Err(DbError::msg("SQLite does not support SSH tunnels")); }
            let mut s = Session::Sqlite(sqlite::SqliteSession::connect(p).await?);
            if p.read_only { s.set_read_only(true).await?; }
            return Ok(s);
        }
        let tunnel = match &p.ssh {
            Some(target) if !target.trim().is_empty() => {
                let host = if p.parts.host.is_empty() { "localhost".to_string() } else { p.parts.host.clone() };
                Some(tunnel::Tunnel::open(target, &host, p.parts.port_or_default()).await?)
            }
            _ => None,
        };
        let fut = async {
            match p.driver {
                DriverKind::Postgres => pg::PgSession::connect(p, tunnel).await.map(Session::Pg),
                DriverKind::Mysql => mysql::MySession::connect(p, tunnel).await.map(Session::My),
                DriverKind::Sqlite => unreachable!(),
            }
        };
        let mut s = tokio::time::timeout(CONNECT_TIMEOUT, fut)
            .await
            .map_err(|_| DbError::msg(format!("connection timed out after {}s", CONNECT_TIMEOUT.as_secs())))??;
        if p.read_only {
            s.set_read_only(true).await?;
        }
        Ok(s)
    }

    pub fn driver(&self) -> DriverKind {
        match self {
            Session::Pg(_) => DriverKind::Postgres,
            Session::My(_) => DriverKind::Mysql,
            Session::Sqlite(_) => DriverKind::Sqlite,
        }
    }

    pub fn server_version(&self) -> &str {
        match self {
            Session::Pg(s) => &s.version,
            Session::My(s) => &s.version,
            Session::Sqlite(s) => &s.version,
        }
    }

    pub fn canceller(&self) -> Canceller {
        match self {
            Session::Pg(s) => Canceller::Pg(s.canceller()),
            Session::My(s) => Canceller::My(s.canceller()),
            Session::Sqlite(s) => Canceller::Sqlite(s.canceller()),
        }
    }

    pub async fn run(&mut self, sql: &str, sink: &mut dyn Sink) -> Result<StmtDone, DbError> {
        match self {
            Session::Pg(s) => s.run(sql, sink).await,
            Session::My(s) => s.run(sql, sink).await,
            Session::Sqlite(s) => s.run(sql, sink).await,
        }
    }

    /// Run a catalog query and collect text rows.
    pub async fn rows(&mut self, sql: &str) -> Result<Vec<Vec<Option<String>>>, DbError> {
        let mut sink = Collect::default();
        self.run(sql, &mut sink).await?;
        Ok(sink.rows.into_iter().map(|r| r.into_iter().map(|c| c.map(|s| s.into_string())).collect()).collect())
    }

    pub async fn set_read_only(&mut self, ro: bool) -> Result<(), DbError> {
        let sql = match (self.driver(), ro) {
            (DriverKind::Postgres, true) => "set session characteristics as transaction read only",
            (DriverKind::Postgres, false) => "set session characteristics as transaction read write",
            (DriverKind::Mysql, true) => "set session transaction read only",
            (DriverKind::Mysql, false) => "set session transaction read write",
            (DriverKind::Sqlite, true) => "pragma query_only = on",
            (DriverKind::Sqlite, false) => "pragma query_only = off",
        };
        self.rows(sql).await.map(|_| ())
    }

    pub fn is_closed(&self) -> bool {
        match self {
            Session::Pg(s) => s.is_closed(),
            Session::My(_) => false,
            Session::Sqlite(_) => false,
        }
    }
}

#[derive(Default)]
pub struct Collect {
    pub cols: Vec<ColumnMeta>,
    pub rows: Vec<Vec<Cell>>,
    pub notices: Vec<String>,
}

impl Sink for Collect {
    fn columns(&mut self, cols: Vec<ColumnMeta>) {
        self.cols = cols;
    }
    fn row(&mut self, row: Vec<Cell>) {
        self.rows.push(row);
    }
    fn notice(&mut self, msg: String) {
        self.notices.push(msg);
    }
}

/// SQL dialect helpers used when the app generates SQL.
pub fn quote_ident(d: DriverKind, name: &str) -> String {
    let simple = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !crate::sql::is_keyword(name);
    if simple {
        return name.to_string();
    }
    match d {
        DriverKind::Postgres | DriverKind::Sqlite => format!("\"{}\"", name.replace('"', "\"\"")),
        DriverKind::Mysql => format!("`{}`", name.replace('`', "``")),
    }
}

/// Quote a possibly schema-qualified name (`public.orders`).
pub fn quote_qualified(d: DriverKind, schema: &str, name: &str) -> String {
    if schema.is_empty() { quote_ident(d, name) } else { format!("{}.{}", quote_ident(d, schema), quote_ident(d, name)) }
}

/// Literal for a cell value of the given kind, for generated UPDATE/INSERT statements.
pub fn literal(d: DriverKind, kind: ColKind, v: Option<&str>) -> String {
    let Some(v) = v else { return "NULL".into() };
    if d == DriverKind::Sqlite && kind == ColKind::Bytes
        && let Some(hex) = v.strip_prefix("0x").filter(|h| h.len() % 2 == 0 && h.bytes().all(|c| c.is_ascii_hexdigit())) {
            return format!("X'{hex}'");
        }
    match kind {
        ColKind::Int | ColKind::Float | ColKind::Decimal if v.parse::<f64>().is_ok() => v.to_string(),
        ColKind::Bool if matches!(v, "true" | "false") => v.to_string(),
        _ => match d {
            DriverKind::Postgres | DriverKind::Sqlite => crate::sql::quote_literal(v),
            DriverKind::Mysql => format!("'{}'", v.replace('\\', "\\\\").replace('\'', "''")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote_ident(DriverKind::Postgres, "orders"), "orders");
        assert_eq!(quote_ident(DriverKind::Postgres, "Order"), "\"Order\"");
        assert_eq!(quote_ident(DriverKind::Postgres, "select"), "\"select\"");
        assert_eq!(quote_ident(DriverKind::Mysql, "a`b"), "`a``b`");
        assert_eq!(literal(DriverKind::Postgres, ColKind::Int, Some("42")), "42");
        assert_eq!(literal(DriverKind::Postgres, ColKind::Int, Some("4x")), "'4x'");
        assert_eq!(literal(DriverKind::Mysql, ColKind::Text, Some("it's \\")), "'it''s \\\\'");
        assert_eq!(literal(DriverKind::Mysql, ColKind::Text, None), "NULL");
    }
}
