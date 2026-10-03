//! SQLite work runs on blocking workers so queries do not stall the UI.
use super::{Cell, ColKind, ColumnMeta, ConnectParams, DbError, Sink, StmtDone};
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use std::sync::Arc;

pub type SqliteCancel = Arc<rusqlite::InterruptHandle>;

pub struct SqliteSession {
    conn: Option<Connection>,
    cancel: SqliteCancel,
    pub version: String,
}

enum Event {
    Columns(Vec<ColumnMeta>),
    Row(Vec<Cell>),
}

impl SqliteSession {
    pub async fn connect(p: &ConnectParams) -> Result<Self, DbError> {
        let path = p.parts.database.clone();
        let conn = tokio::task::spawn_blocking(move || {
            // Opening a mistyped path must not silently create an empty database.
            let conn = if path == ":memory:" {
                Connection::open_in_memory()
            } else {
                Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            }?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            conn.execute_batch("pragma foreign_keys = on")?;
            Ok::<_, rusqlite::Error>(conn)
        }).await.map_err(|e| DbError::msg(e.to_string()))?.map_err(from_sqlite)?;
        let cancel = Arc::new(conn.get_interrupt_handle());
        Ok(Self { conn: Some(conn), cancel, version: format!("SQLite {}", rusqlite::version()) })
    }

    pub fn canceller(&self) -> SqliteCancel { self.cancel.clone() }

    pub async fn run(&mut self, sql: &str, sink: &mut dyn Sink) -> Result<StmtDone, DbError> {
        let conn = self.conn.take().ok_or_else(|| DbError::msg("SQLite connection unavailable"))?;
        let sql = sql.to_string();
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let worker = tokio::task::spawn_blocking(move || {
            let result = (|| {
                let mut stmt = conn.prepare(&sql).map_err(from_sqlite)?;
                let mut done = StmtDone::default();
                if stmt.column_count() == 0 {
                    let count = stmt.execute([]).map_err(from_sqlite)?;
                    done.rows_affected = Some(count as u64);
                } else {
                    done.returned_rows = true;
                    let mut metas: Vec<_> = stmt.columns().iter().map(|c| {
                        let ty = c.decl_type().unwrap_or("").to_string();
                        ColumnMeta { name: c.name().to_string(), kind: kind(&ty), type_name: ty }
                    }).collect();
                    let mut rows = stmt.query([]).map_err(from_sqlite)?;
                    let mut sent_columns = false;
                    while let Some(row) = rows.next().map_err(from_sqlite)? {
                        let mut cells = Vec::with_capacity(metas.len());
                        for (i, meta) in metas.iter_mut().enumerate() {
                            let value = row.get_ref(i).map_err(from_sqlite)?;
                            if !sent_columns && meta.type_name.is_empty() {
                                let (ty, k) = match value {
                                    ValueRef::Integer(_) => ("integer", ColKind::Int),
                                    ValueRef::Real(_) => ("real", ColKind::Float),
                                    ValueRef::Blob(_) => ("blob", ColKind::Bytes),
                                    _ => ("text", ColKind::Text),
                                };
                                meta.type_name = ty.into();
                                meta.kind = k;
                            }
                            cells.push(cell(value));
                        }
                        if !sent_columns {
                            let _ = tx.blocking_send(Event::Columns(metas.clone()));
                            sent_columns = true;
                        }
                        if tx.blocking_send(Event::Row(cells)).is_err() { break; }
                        done.rows_returned += 1;
                    }
                    if !sent_columns { let _ = tx.blocking_send(Event::Columns(metas)); }
                }
                Ok(done)
            })();
            (conn, result)
        });
        let mut count = 0;
        while let Some(event) = rx.recv().await {
            match event {
                Event::Columns(cols) => sink.columns(cols),
                Event::Row(row) => { sink.row(row); count += 1; if count % 64 == 0 { sink.flush(); } }
            }
        }
        let (conn, result) = worker.await.map_err(|e| DbError::msg(e.to_string()))?;
        self.conn = Some(conn);
        sink.flush();
        result
    }
}

fn from_sqlite(e: rusqlite::Error) -> DbError { DbError::msg(e.to_string()) }

fn kind(ty: &str) -> ColKind {
    let ty = ty.to_ascii_uppercase();
    if ty.contains("INT") { ColKind::Int }
    else if ty.contains("CHAR") || ty.contains("CLOB") || ty.contains("TEXT") { ColKind::Text }
    else if ty.contains("BLOB") { ColKind::Bytes }
    else if ty.contains("REAL") || ty.contains("FLOA") || ty.contains("DOUB") { ColKind::Float }
    else { ColKind::Text }
}

fn cell(v: ValueRef<'_>) -> Cell {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(n) => Some(n.to_string().into()),
        ValueRef::Real(n) => Some(n.to_string().into()),
        ValueRef::Text(s) => Some(String::from_utf8_lossy(s).into_owned().into()),
        ValueRef::Blob(b) => Some(format!("0x{}", b.iter().map(|b| format!("{b:02x}")).collect::<String>()).into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::{DriverKind, UrlParts}, db::{Collect, Session, schema}};

    async fn session() -> Session {
        Session::connect(&ConnectParams { driver: DriverKind::Sqlite, parts: UrlParts::parse("sqlite://:memory:").unwrap(), password: None, read_only: false, ssh: None }).await.unwrap()
    }

    #[tokio::test]
    async fn queries_schema_and_read_only() {
        let mut s = session().await;
        s.rows("create table parent (id integer primary key)").await.unwrap();
        s.rows("create table child (id integer primary key, parent_id integer references parent, value text, payload blob)").await.unwrap();
        s.rows("create index child_value on child(value)").await.unwrap();
        s.rows("create view child_view as select * from child").await.unwrap();
        s.rows("insert into parent values (1)").await.unwrap();
        let mut sink = Collect::default();
        let done = s.run("insert into child values (1, 1, NULL, X'00ff')", &mut sink).await.unwrap();
        assert_eq!(done.rows_affected, Some(1));
        let done = s.run("select * from child", &mut sink).await.unwrap();
        assert_eq!(done.rows_returned, 1);
        assert_eq!(sink.rows[0][2], None);
        assert_eq!(sink.rows[0][3].as_deref(), Some("0x00ff"));
        assert_eq!(sink.cols[0].kind, ColKind::Int);
        let schema = schema::load(&mut s).await.unwrap();
        assert_eq!(schema.primary_key("child"), vec!["id"]);
        assert_eq!(schema.columns_of("child")[1].fk, Some(("parent".into(), "id".into())));
        assert_eq!(schema.table("child_view").unwrap().kind, schema::TableKind::View);
        let structure = schema::structure(&mut s, schema.table("child").unwrap(), schema.columns_of("child")).await.unwrap();
        assert!(structure.create.contains("CREATE TABLE"));
        assert_eq!(structure.indexes[0].0, "child_value");
        assert_eq!(structure.foreign_keys.len(), 1);
        s.rows("begin").await.unwrap();
        s.rows("update child set value = 'temporary' where id = 1").await.unwrap();
        s.rows("rollback").await.unwrap();
        assert_eq!(s.rows("select value from child").await.unwrap()[0][0], None);
        s.set_read_only(true).await.unwrap();
        assert!(s.rows("delete from child").await.is_err());
        s.set_read_only(false).await.unwrap();
        s.rows("update child set value = 'saved' where id = 1").await.unwrap();
        assert!(s.rows("insert into child values (2, 99, NULL, NULL)").await.is_err());
        assert!(!s.rows("explain query plan select * from child where id = 1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancellation_and_reuse() {
        let mut s = session().await;
        let cancel = s.canceller();
        let interrupt = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            cancel.cancel().await.unwrap();
        });
        assert!(s.rows("with recursive n(x) as (values(1) union all select x+1 from n) select sum(x) from n").await.is_err());
        interrupt.await.unwrap();
        assert_eq!(s.rows("select 42").await.unwrap()[0][0].as_deref(), Some("42"));
    }

    #[tokio::test]
    async fn file_connections_persist_and_missing_files_fail() {
        let path = std::env::temp_dir().join(format!("zdb-sqlite-{}-{}.db", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let params = ConnectParams { driver: DriverKind::Sqlite, parts: UrlParts { scheme: "sqlite".into(), database: path.to_str().unwrap().into(), ..Default::default() }, password: None, read_only: false, ssh: None };
        assert!(Session::connect(&params).await.is_err());
        assert!(!path.exists());
        drop(Connection::open(&path).unwrap());
        let mut s = Session::connect(&params).await.unwrap();
        s.rows("create table persisted (id integer)").await.unwrap();
        s.rows("insert into persisted values (7)").await.unwrap();
        drop(s);
        let mut s = Session::connect(&params).await.unwrap();
        assert_eq!(s.rows("select id from persisted").await.unwrap()[0][0].as_deref(), Some("7"));
        drop(s);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn url_round_trip() {
        for url in ["sqlite:///tmp/my%20db.sqlite", "sqlite://relative/db.sqlite", "sqlite://:memory:"] {
            let parts = UrlParts::parse(url).unwrap();
            assert_eq!(UrlParts::parse(&parts.build()).unwrap(), parts);
        }
        assert_eq!(UrlParts::parse("sqlite:///tmp/test.db").unwrap().database, "/tmp/test.db");
        assert!(UrlParts::parse("sqlite://").is_err());
    }
}
