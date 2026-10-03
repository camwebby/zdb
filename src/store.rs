//! Local SQLite store for history, workspaces and saved queries. WAL mode and a busy
//! timeout let several zdb instances share one file.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub struct Store {
    db: Connection,
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub project: String,
    pub env: String,
    pub level: String,
    pub sql: String,
    pub at: i64,
    pub duration_ms: i64,
    pub rows: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Workspace {
    pub env: String,
    pub tabs: Vec<TabState>,
    pub current: usize,
    #[serde(default = "default_split")]
    pub split: u16,
}

fn default_split() -> u16 {
    40
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TabState {
    pub name: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub cursor: (usize, usize),
    /// Set for table tabs: the qualified table name.
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default)]
    pub filters: Vec<String>,
    #[serde(default)]
    pub sort: Vec<(String, bool)>,
    #[serde(default)]
    pub row_limit: Option<usize>,
    #[serde(default)]
    pub saved: Option<String>,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct SavedQuery {
    pub name: String,
    pub sql: String,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let db = Connection::open(path)?;
        Self::init(db)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Store> {
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        let _ = db.pragma_update(None, "journal_mode", "wal");
        db.execute_batch(
            "create table if not exists history (
                id integer primary key, project text not null, env text not null, level text not null,
                sql text not null, at integer not null, duration_ms integer not null, rows integer, error text);
             create index if not exists history_at on history(at desc);
             create table if not exists workspaces (project text primary key, data text not null, updated_at integer not null);
             create table if not exists saved_queries (project text not null, name text not null, sql text not null,
                updated_at integer not null, primary key (project, name));
             create table if not exists kv (key text primary key, value text not null);",
        )?;
        Ok(Store { db })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_history(
        &self,
        project: &str,
        env: &str,
        level: &str,
        sql: &str,
        duration_ms: i64,
        rows: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.db.execute(
            "insert into history (project, env, level, sql, at, duration_ms, rows, error) values (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![project, env, level, sql, now(), duration_ms, rows, error],
        )?;
        Ok(())
    }

    pub fn history(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut st = self.db.prepare(
            "select id, project, env, level, sql, at, duration_ms, rows, error from history order by at desc, id desc limit ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| {
            Ok(HistoryEntry {
                project: r.get(1)?,
                env: r.get(2)?,
                level: r.get(3)?,
                sql: r.get(4)?,
                at: r.get(5)?,
                duration_ms: r.get(6)?,
                rows: r.get(7)?,
                error: r.get(8)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn save_workspace(&self, project: &str, ws: &Workspace) -> Result<()> {
        self.db.execute(
            "insert into workspaces (project, data, updated_at) values (?1, ?2, ?3)
             on conflict(project) do update set data = excluded.data, updated_at = excluded.updated_at",
            params![project, serde_json::to_string(ws)?, now()],
        )?;
        Ok(())
    }

    pub fn workspace(&self, project: &str) -> Result<Option<Workspace>> {
        let data: Option<String> =
            self.db.query_row("select data from workspaces where project = ?1", [project], |r| r.get(0)).optional()?;
        Ok(data.and_then(|d| serde_json::from_str(&d).ok()))
    }

    pub fn save_query(&self, project: &str, name: &str, sql: &str) -> Result<()> {
        self.db.execute(
            "insert into saved_queries (project, name, sql, updated_at) values (?1, ?2, ?3, ?4)
             on conflict(project, name) do update set sql = excluded.sql, updated_at = excluded.updated_at",
            params![project, name, sql, now()],
        )?;
        Ok(())
    }

    pub fn saved_queries(&self, project: &str) -> Result<Vec<SavedQuery>> {
        let mut st = self.db.prepare("select name, sql from saved_queries where project = ?1 order by updated_at desc")?;
        let rows = st.query_map([project], |r| Ok(SavedQuery { name: r.get(0)?, sql: r.get(1)? }))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn delete_query(&self, project: &str, name: &str) -> Result<()> {
        self.db.execute("delete from saved_queries where project = ?1 and name = ?2", params![project, name])?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.db.query_row("select value from kv where key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.db.execute(
            "insert into kv (key, value) values (?1, ?2) on conflict(key) do update set value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_workspace_saved() {
        let s = Store::in_memory().unwrap();
        s.add_history("shop", "prod", "prod", "select 1", 4, Some(1), None).unwrap();
        s.add_history("shop", "staging", "staging", "select 2", 4, None, Some("boom")).unwrap();
        let h = s.history(10).unwrap();
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].sql, "select 2");
        assert_eq!(h[0].error.as_deref(), Some("boom"));

        let ws = Workspace {
            env: "staging".into(),
            tabs: vec![TabState { name: "untitled-1".into(), text: "select 1".into(), ..Default::default() }],
            current: 0,
            split: 40,
        };
        s.save_workspace("shop", &ws).unwrap();
        assert_eq!(s.workspace("shop").unwrap(), Some(ws));
        assert_eq!(s.workspace("none").unwrap(), None);

        s.save_query("shop", "active users", "select 1").unwrap();
        s.save_query("shop", "active users", "select 2").unwrap();
        let q = s.saved_queries("shop").unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].sql, "select 2");

        s.set("last", "shop").unwrap();
        assert_eq!(s.get("last").as_deref(), Some("shop"));
    }
}
