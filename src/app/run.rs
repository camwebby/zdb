//! Running SQL: picking statements, parameters, safety checks, background jobs and
//! folding their events back into tab state. Also schema, structure, count and export.

use super::overlay::{Overlay, Prompt, PromptKind};
use super::*;
use crate::config::Level;
use crate::db::{self, ConnectParams, Sink, quote_qualified};
use crate::secrets::{self, Lookup};
use crate::sql::{self, Kind};
use std::io::Write as _;

pub enum NeedPassword {
    Prompt { store: bool, label: String },
    NoConnection,
    Invalid(String),
}

impl App {
    /// Connection parameters for the current environment, or what's missing.
    pub fn connect_params(&self) -> Result<ConnectParams, NeedPassword> {
        let env = self.env_config().ok_or(NeedPassword::NoConnection)?;
        let parts = UrlParts::parse(&env.url).map_err(|e| NeedPassword::Invalid(e.to_string()))?;
        let driver = parts.driver().ok_or_else(|| NeedPassword::Invalid("unknown driver".into()))?;
        let password = match self.passwords.get(&self.key()) {
            Some(p) => Some(p.clone()),
            None => match secrets::lookup(&self.project, env, &parts) {
                Lookup::Found(p) => Some(p),
                Lookup::NotNeeded => None,
                Lookup::Prompt { store_in_keychain } => {
                    return Err(NeedPassword::Prompt {
                        store: store_in_keychain,
                        label: format!("Password for {}", parts.display()),
                    });
                }
            },
        };
        Ok(ConnectParams { driver, parts, password, read_only: self.read_only(), ssh: env.ssh.clone() })
    }

    /// Get connection params, or prompt for a password and retry `then` afterwards.
    pub fn with_params(&mut self, then: impl FnOnce(&mut App, ConnectParams) + 'static) {
        match self.connect_params() {
            Ok(p) => then(self, p),
            Err(NeedPassword::NoConnection) => {
                self.toast_err("no connection · Space c n adds one");
            }
            Err(NeedPassword::Invalid(e)) => self.toast_err(e),
            Err(NeedPassword::Prompt { store, label }) => {
                let key = self.key();
                let (project, env) = (self.project.clone(), self.env.clone());
                self.overlay = Some(Overlay::Prompt(Prompt::new(
                    PromptKind::Password { store, key, project, env, then: Some(Box::new(move |app: &mut App| app.with_params(then))) },
                    &label,
                    "",
                )));
            }
        }
    }

    // ---- picking statements ----

    pub fn run_current(&mut self, all: bool, purpose: Purpose) {
        if !self.has_connection() {
            self.toast_err("no connection · Space c n adds one, or start zdb with a URL");
            return;
        }
        if self.tab().run.is_some() {
            self.toast("a query is already running · ^C cancels it");
            return;
        }
        let idx = self.cur;
        if self.tabs[idx].is_table() && !self.tabs[idx].editor_visible() {
            self.refresh_table(idx);
            return;
        }
        let tab = &self.tabs[idx];
        let text = tab.editor.text();
        let mut picked: Vec<(String, usize)> = Vec::new();
        if let Some(r) = tab.editor.selection_range() {
            let sel = &text[r.clone()];
            for s in sql::split(sel, self.settings.blank_line_splits) {
                picked.push((sel[s.range.clone()].to_string(), r.start + s.range.start));
            }
        } else {
            let stmts = tab.statements(self.settings.blank_line_splits);
            let list = &stmts.2;
            if all {
                for s in list {
                    picked.push((text[s.range.clone()].to_string(), s.range.start));
                }
            } else if let Some(i) = sql::stmt_at(list, tab.editor.cursor_offset()) {
                let s = &list[i];
                picked.push((text[s.range.clone()].to_string(), s.range.start));
            }
        }
        if picked.is_empty() {
            self.toast("nothing to run");
            return;
        }
        if matches!(purpose, Purpose::Explain { .. }) {
            picked.truncate(1);
        }
        self.with_param_values(picked, move |app, stmts| app.check_and_run(stmts, purpose));
    }

    /// Prompt for :name / $1 parameters if any, then continue with substituted statements.
    fn with_param_values(&mut self, picked: Vec<(String, usize)>, then: impl FnOnce(&mut App, Vec<(String, String, usize)>) + 'static) {
        let mut names: Vec<String> = Vec::new();
        for (s, _) in &picked {
            for p in sql::params(s) {
                if !names.contains(&p) {
                    names.push(p);
                }
            }
        }
        if names.is_empty() {
            let v = picked.into_iter().map(|(s, o)| (s.clone(), s, o)).collect();
            then(self, v);
            return;
        }
        let remembered = self.tab().params.clone();
        let values: Vec<String> = names.iter().map(|n| remembered.get(n).cloned().unwrap_or_default()).collect();
        self.overlay = Some(Overlay::Params(overlay::ParamForm::new(
            names,
            values,
            Box::new(move |app: &mut App, vals: BTreeMap<String, String>| {
                app.tab_mut().params.extend(vals.clone());
                let v = picked
                    .into_iter()
                    .map(|(s, o)| {
                        let exec = sql::substitute(&s, &|n| vals.get(n).cloned());
                        (exec, s, o)
                    })
                    .collect();
                then(app, v);
            }),
        )));
    }

    /// Statement checks, then confirm if needed, then run.
    fn check_and_run(&mut self, stmts: Vec<(String, String, usize)>, purpose: Purpose) {
        let level = self.level();
        let prod = level == Level::Prod;
        let ro = self.read_only();
        let mut flagged: Vec<String> = Vec::new();
        let mut typed = false;
        let mut reason = "";
        for (exec, _, _) in &stmts {
            let info = sql::classify(exec);
            let analyze = matches!(purpose, Purpose::Explain { analyze: true });
            let writes = info.is_write() || (analyze && sql::classify(exec).is_write());
            if ro && writes {
                let msg = format!("{} is read-only · Space c w allows writes", self.env);
                self.tab_mut().push_msg(MsgKind::Error, format!("blocked: {}", sql::trimmed(exec)));
                self.toast_err(msg);
                return;
            }
            if analyze && level != Level::Local {
                flagged.push(exec.clone());
                reason = "EXPLAIN ANALYZE executes the statement";
                continue;
            }
            if info.unfiltered_write() {
                flagged.push(exec.clone());
                typed |= prod;
                reason = if info.kind == Kind::Update { "UPDATE without WHERE changes every row" } else { "DELETE without WHERE removes every row" };
            } else if matches!(info.kind, Kind::Drop | Kind::Truncate) {
                flagged.push(exec.clone());
                typed |= prod;
                reason = "this cannot be undone";
            } else if info.kind == Kind::Alter && prod {
                flagged.push(exec.clone());
                reason = "schema change on production";
            }
        }
        let go = move |app: &mut App| app.dispatch_statements(stmts, purpose);
        if flagged.is_empty() {
            go(self);
        } else {
            let require = if typed { Some(self.env.clone()) } else { None };
            let sql = flagged.iter().map(|s| format!("{};", sql::trimmed(s))).collect::<Vec<_>>().join("\n");
            self.confirm("Confirm", reason.to_string(), sql, require, Box::new(go));
        }
    }

    fn row_cap(&self) -> Option<usize> {
        let n = self.tab().row_limit.unwrap_or(self.settings.row_limit);
        if n == 0 { None } else { Some(n) }
    }

    fn dispatch_statements(&mut self, stmts: Vec<(String, String, usize)>, purpose: Purpose) {
        let cap = self.row_cap();
        let driver = self.driver();
        if driver == DriverKind::Sqlite && matches!(purpose, Purpose::Explain { analyze: true }) {
            self.toast_err("SQLite does not support EXPLAIN ANALYZE · use EXPLAIN");
            return;
        }
        let job: Vec<JobStmt> = stmts
            .into_iter()
            .map(|(exec, orig, off)| {
                let (exec, cap) = match purpose {
                    Purpose::Explain { analyze } => (explain_sql(driver, &exec, analyze), None),
                    _ => match cap.and_then(|c| sql::apply_cap(&exec, c)) {
                        Some(capped) => (capped, cap),
                        None => (sql::trimmed(&exec).to_string(), None),
                    },
                };
                let label = match purpose {
                    Purpose::Explain { analyze: true } => "explain analyze".to_string(),
                    Purpose::Explain { .. } => "explain".to_string(),
                    _ => sql::result_label(&orig),
                };
                JobStmt { exec, orig, offset: Some(off), label, cap, purpose, server_sort: false }
            })
            .collect();
        let i = self.cur;
        self.start_job(i, job, false);
    }

    /// Start a job on a tab's session. Results are replaced unless the job is silent.
    pub fn start_job(&mut self, tab_idx: usize, stmts: Vec<JobStmt>, transactional: bool) {
        let tab_id = self.tabs[tab_idx].id;
        self.with_params(move |app, params| {
            let Some(idx) = app.tabs.iter().position(|t| t.id == tab_id) else { return };
            let job = app.alloc_id();
            let env = app.key();
            let env_name = app.env.clone();
            let silent = stmts.iter().all(|s| matches!(s.purpose, Purpose::Silent | Purpose::Commit | Purpose::Delete | Purpose::Insert));
            let t = &mut app.tabs[idx];
            if !silent {
                t.results.clear();
                t.messages.clear();
                t.cur_result = 0;
                t.stale_from = None;
            }
            t.error = None;
            t.run = Some(RunState { job, started: Instant::now(), stmts: stmts.clone(), cur: 0, cancelling: false, any_error: false, total_rows: 0 });
            let slot = t.slot.clone();
            let canceller = t.canceller.clone();
            let tx = app.tx.clone();
            let want_ro = params.read_only;
            let _ = env_name;
            tokio::spawn(job_task(slot, canceller, params, env, want_ro, stmts, transactional, tx, tab_id, job));
            app.ensure_schema();
        });
    }

    // ---- schema ----

    pub fn ensure_schema(&mut self) {
        let key = self.key();
        if !self.has_connection() || self.schemas.contains_key(&key) {
            return;
        }
        self.load_schema();
    }

    pub fn load_schema(&mut self) {
        let key = self.key();
        let Ok(params) = self.connect_params() else { return };
        // a refresh keeps the old schema usable until the new one arrives
        if !matches!(self.schemas.get(&key), Some(SchemaState::Ready(_))) {
            self.schemas.insert(key.clone(), SchemaState::Loading);
        }
        let slot = self.meta.entry(key.clone()).or_insert_with(new_slot).clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = async {
                let mut g = slot.lock().await;
                ensure_session(&mut g, &params, &key, params.read_only).await?;
                db::schema::load(g.session.as_mut().unwrap()).await
            }
            .await;
            let _ = tx.send(Msg::Schema { env: key, res: res.map_err(|e| e.message) });
        });
    }

    fn meta_task<T: Send + 'static>(
        &mut self,
        work: impl for<'a> FnOnce(&'a mut Session) -> futures::future::BoxFuture<'a, Result<T, DbError>> + Send + 'static,
        done: impl FnOnce(Result<T, String>) -> Msg + Send + 'static,
    ) {
        let key = self.key();
        let slot = self.meta.entry(key.clone()).or_insert_with(new_slot).clone();
        let tx = self.tx.clone();
        self.with_params(move |_app, params| {
            tokio::spawn(async move {
                let res = async {
                    let mut g = slot.lock().await;
                    ensure_session(&mut g, &params, &key, params.read_only).await?;
                    work(g.session.as_mut().unwrap()).await
                }
                .await;
                let _ = tx.send(done(res.map_err(|e| e.message)));
            });
        });
    }

    // ---- table tabs ----

    pub fn open_table(&mut self, name: &str, filter: Option<String>) {
        let id = self.alloc_id();
        let mut tab = Tab::new(id, name, "", self.settings.vim);
        tab.table = Some(TableTab { name: name.to_string(), filters: filter.into_iter().collect(), sort: vec![], show_sql: false, structure: false });
        self.tabs.push(tab);
        self.cur = self.tabs.len() - 1;
        self.focus = Focus::Results;
        let i = self.cur;
        self.refresh_table(i);
    }

    /// The SELECT a table tab runs: filters become WHERE, newest primary key first by default.
    pub fn browse_sql(&self, t: &TableTab, limit: Option<usize>) -> String {
        let d = self.driver();
        let schema = self.schema();
        let (sch, name) = match schema.as_ref().and_then(|s| s.table(&t.name)) {
            Some(ti) => (ti.schema.clone(), ti.name.clone()),
            None => match t.name.split_once('.') {
                Some((a, b)) => (a.to_string(), b.to_string()),
                None => (String::new(), t.name.clone()),
            },
        };
        let mut s = format!("select * from {}", quote_qualified(d, &sch, &name));
        if !t.filters.is_empty() {
            s.push_str(" where ");
            let cols: Vec<String> = schema
                .as_ref()
                .and_then(|sc| {
                    sc.table(&t.name)
                        .and_then(|ti| sc.columns.get(&ti.display()))
                })
                .map(|cs| cs.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default();
            s.push_str(&t.filters.iter().map(|f| format!("({})", filter_sql(d, f, &cols))).collect::<Vec<_>>().join(" and "));
        }
        let order: Vec<String> = if !t.sort.is_empty() {
            t.sort.iter().map(|(c, desc)| { format!("{}{}", db::quote_ident(d, c), if *desc { " desc" } else { "" })
                }).collect()
        } else {
            schema.map(|s| s.primary_key(&t.name)).unwrap_or_default().iter().map(|c| format!("{} desc", db::quote_ident(d, c))).collect()
        };
        if !order.is_empty() {
            s.push_str(" order by ");
            s.push_str(&order.join(", "));
        }
        if let Some(n) = limit {
            s.push_str(&format!(" limit {}", n + 1));
        }
        s
    }

    pub fn refresh_table(&mut self, idx: usize) {
        let Some(t) = self.tabs[idx].table.clone() else { return };
        if !self.has_connection() {
            return;
        }
        if t.structure {
            self.load_structure(idx);
            return;
        }
        // wait for the schema so the default order can use the primary key
        if matches!(self.schemas.get(&self.key()), None | Some(SchemaState::Loading)) {
            self.ensure_schema();
            if matches!(self.schemas.get(&self.key()), Some(SchemaState::Loading)) {
                self.tabs[idx].last = Some((true, "loading schema…".into()));
                return;
            }
        }
        let limit = self.settings.browse_rows;
        let exec = self.browse_sql(&t, Some(limit));
        let orig = self.browse_sql(&t, None);
        self.tabs[idx].editor.set_text(&format!("{orig};"));
        let stmt = JobStmt { exec, orig, offset: None, label: t.name.rsplit('.').next().unwrap_or(&t.name).to_string(), cap: Some(limit), purpose: Purpose::Browse, server_sort: true };
        self.start_job(idx, vec![stmt], false);
    }

    pub fn load_structure(&mut self, idx: usize) {
        let Some(t) = self.tabs[idx].table.clone() else { return };
        let Some(schema) = self.schema() else {
            self.ensure_schema();
            self.toast("loading schema…");
            return;
        };
        let Some(info) = schema.table(&t.name).cloned() else {
            self.toast_err(format!("{} not found in schema", t.name));
            return;
        };
        let cols = schema.columns.get(&info.display()).cloned().unwrap_or_default();
        let tab_id = self.tabs[idx].id;
        self.meta_task(
            move |s| Box::pin(async move { db::schema::structure(s, &info, &cols).await }),
            move |res| Msg::Structure { tab: tab_id, res },
        );
    }

    pub fn count_rows(&mut self, table: &str) {
        let d = self.driver();
        let schema = self.schema();
        let q = match schema.as_ref().and_then(|s| s.table(table)) {
            Some(t) => quote_qualified(d, &t.schema, &t.name),
            None => table.to_string(),
        };
        let name = table.to_string();
        self.meta_task(
            move |s| {
                Box::pin(async move {
                    let rows = s.rows(&format!("select count(*) from {q}")).await?;
                    Ok(rows.first().and_then(|r| r[0].clone()).unwrap_or_default())
                })
            },
            move |res| Msg::Count { table: name, res },
        );
    }

    // ---- edits ----

    pub fn edit_statements(&self) -> Result<Vec<String>, String> {
        let tab = self.tab();
        let t = tab.table.as_ref().ok_or("not a table tab")?;
        let view = tab.results.first().ok_or("no data")?;
        let schema = self.schema().ok_or("schema not loaded")?;
        let pk = schema.primary_key(&t.name);
        if pk.is_empty() {
            return Err(format!("{} has no primary key", t.name));
        }
        let d = self.driver();
        let info = schema.table(&t.name).ok_or("table not in schema")?;
        let q = quote_qualified(d, &info.schema, &info.name);
        let mut by_row: BTreeMap<usize, Vec<(usize, Option<String>)>> = BTreeMap::new();
        for ((r, c), v) in &view.grid.edits {
            by_row.entry(*r).or_default().push((*c, v.clone()));
        }
        let mut out = Vec::new();
        for (r, cols) in by_row {
            let sets: Vec<String> = cols
                .iter()
                .map(|(c, v)| { format!("{} = {}", db::quote_ident(d, &view.rs.cols[*c].name), db::literal(d, view.rs.cols[*c].kind, v.as_deref()))
                })
                .collect();
            let wh = pk_where(d, view, &pk, r)?;
            out.push(format!("update {q} set {} where {wh}", sets.join(", ")));
        }
        Ok(out)
    }

    /// One `delete … where <pk>` per data row, for the rows the caller picked.
    pub fn delete_statements(&self, rows: &[usize]) -> Result<Vec<String>, String> {
        let tab = self.tab();
        let t = tab.table.as_ref().ok_or("not a table tab")?;
        let view = tab.results.first().ok_or("no data")?;
        let schema = self.schema().ok_or("schema not loaded")?;
        let pk = schema.primary_key(&t.name);
        if pk.is_empty() {
            return Err(format!("{} has no primary key", t.name));
        }
        let d = self.driver();
        let info = schema.table(&t.name).ok_or("table not in schema")?;
        let q = quote_qualified(d, &info.schema, &info.name);
        rows.iter().map(|r| Ok(format!("delete from {q} where {}", pk_where(d, view, &pk, *r)?))).collect()
    }

    /// After an insert: reload and show the new row, or bring the form back with the error.
    fn finish_insert(&mut self, idx: usize, ok: bool) {
        let Some(mut form) = self.insert_draft.take() else { return };
        if ok {
            self.toast(format!("Inserted 1 row into {}", form.table));
            let t = &mut self.tabs[idx];
            // the default order is newest first, so the new row is at the top
            let default_order = t.table.as_ref().is_some_and(|x| x.sort.is_empty());
            t.restore_cursor = t.results.first().map(|v| if default_order { (0, v.grid.col, 0) } else { (v.grid.row, v.grid.col, v.grid.top) });
            self.refresh_table(idx);
        } else if self.overlay.is_none() && self.tabs.get(self.cur).is_some_and(|t| t.id == form.tab_id) {
            form.error = self.tabs[idx].last.as_ref().map(|(_, m)| m.clone());
            self.overlay = Some(Overlay::Insert(Box::new(form)));
        }
    }

    pub fn run_insert(&mut self, form: overlay::InsertForm) {
        let sql = form.statement();
        self.insert_draft = Some(form);
        let stmt = JobStmt { exec: sql.clone(), orig: sql, offset: None, label: "insert".into(), cap: None, purpose: Purpose::Insert, server_sort: false };
        let i = self.cur;
        self.start_job(i, vec![stmt], false);
    }

    pub fn run_delete(&mut self, stmts: Vec<String>) {
        let job: Vec<JobStmt> = stmts
            .into_iter()
            .map(|s| JobStmt { exec: s.clone(), orig: s, offset: None, label: "delete".into(), cap: None, purpose: Purpose::Delete, server_sort: false })
            .collect();
        let i = self.cur;
        self.start_job(i, job, true);
    }

    pub fn commit_edits(&mut self) {
        if self.read_only() {
            self.toast_err(format!("{} is read-only · Space c w allows writes", self.env));
            return;
        }
        let stmts = match self.edit_statements() {
            Ok(s) => s,
            Err(e) => {
                self.toast_err(e);
                return;
            }
        };
        let job: Vec<JobStmt> = stmts
            .into_iter()
            .map(|s| JobStmt { exec: s.clone(), orig: s, offset: None, label: "update".into(), cap: None, purpose: Purpose::Commit, server_sort: false })
            .collect();
        let i = self.cur;
        self.start_job(i, job, true);
    }

    // ---- export ----

    pub fn start_export(&mut self, fmt: char, path: String) {
        let Some((sql_text, table)) = self.export_source() else {
            self.toast_err("nothing to export · run a query first");
            return;
        };
        let path = expand_tilde(&path);
        let driver = self.driver();
        let tx = self.tx.clone();
        let (ctx, crx) = tokio::sync::oneshot::channel();
        self.export = Some(Export { rows: 0, path: path.clone(), cancel: ctx });
        self.with_params(move |_app, params| {
            tokio::spawn(async move {
                let res = async {
                    let file = std::fs::File::create(&path).map_err(|e| DbError::msg(format!("{path}: {e}")))?;
                    let mut sink = FileSink::new(file, fmt, driver, table, tx.clone());
                    let mut s = Session::connect(&params).await?;
                    let run = s.run(&sql_text, &mut sink);
                    tokio::select! {
                        r = run => { r?; }
                        _ = crx => { return Err(DbError::msg("export cancelled")); }
                    }
                    sink.finish().map_err(|e| DbError::msg(e.to_string()))?;
                    Ok(sink.rows)
                }
                .await;
                let _ = tx.send(Msg::Export(match res {
                    Ok(rows) => ExportEv::Done { rows, path },
                    Err(e) => ExportEv::Failed(e.message),
                }));
            });
        });
    }

    /// SQL to re-run for export (no row cap) and a table name for INSERTs.
    pub fn export_source(&self) -> Option<(String, String)> {
        let tab = self.tab();
        if let Some(t) = &tab.table {
            return Some((self.browse_sql(t, None), t.name.clone()));
        }
        let v = tab.view()?;
        if v.stmt.orig.is_empty() {
            return None;
        }
        let table = sql::table_refs(&v.stmt.orig).first().map(|(t, _)| t.clone()).unwrap_or_else(|| v.label.clone());
        Some((sql::trimmed(&v.stmt.orig).to_string(), table))
    }

    // ---- messages from tasks ----

    pub fn on_msg(&mut self, m: Msg) {
        match m {
            Msg::Job { tab, job, ev } => self.on_job(tab, job, ev),
            Msg::Schema { env, res } => {
                let ok = res.is_ok();
                let st = match res {
                    Ok(s) => SchemaState::Ready(Arc::new(s)),
                    Err(e) => SchemaState::Failed(e),
                };
                if let SchemaState::Failed(e) = &st
                    && env == self.key() {
                        self.toast_err(format!("schema: {e}"));
                    }
                self.schemas.insert(env.clone(), st);
                if ok && env == self.key() {
                    // table tabs waiting for the schema
                    for i in 0..self.tabs.len() {
                        if self.tabs[i].is_table() && self.tabs[i].run.is_none() && self.tabs[i].results.is_empty() && i == self.cur {
                            self.refresh_table(i);
                        }
                    }
                    self.attach_keys_all();
                }
                if env == self.key() {
                    // an open palette picks up the tables right away
                    if let Some(Overlay::Palette(mut p)) = self.overlay.take() {
                        p.items = self.palette_items();
                        p.filter();
                        self.overlay = Some(Overlay::Palette(p));
                    }
                }
            }
            Msg::Structure { tab, res } => {
                let schema = self.schema();
                let Some(t) = self.tab_by_id(tab) else { return };
                match res {
                    Ok(st) => {
                        let name = t.table.as_ref().map(|x| x.name.clone()).unwrap_or_default();
                        t.structure = structure_views(&name, schema.as_deref(), st);
                        t.cur_result = 0;
                    }
                    Err(e) => t.push_msg(MsgKind::Error, e),
                }
            }
            Msg::Count { table, res } => match res {
                Ok(n) => self.toast(format!("{table} · {} rows", crate::grid::group_digits(&n))),
                Err(e) => self.toast_err(e),
            },
            Msg::Export(ev) => match ev {
                ExportEv::Progress(n) => {
                    if let Some(e) = &mut self.export {
                        e.rows = n;
                    }
                }
                ExportEv::Done { rows, path } => {
                    self.export = None;
                    self.toast(format!("Exported {} rows to {path}", crate::grid::group_digits(&rows.to_string())));
                }
                ExportEv::Failed(e) => {
                    self.export = None;
                    self.toast_err(format!("export failed: {e}"));
                }
            },
            Msg::Test { res } => {
                self.test_result = Some(res);
            }
        }
    }

    fn attach_keys_all(&mut self) {
        let Some(schema) = self.schema() else { return };
        let group = self.settings.group_digits;
        for t in self.tabs.iter_mut() {
            let table = t.table.as_ref().map(|x| x.name.clone());
            for v in t.results.iter_mut() {
                let had = v.rs.keys.iter().any(|k| k.0 || k.1.is_some());
                attach_keys(v, &schema, table.as_deref());
                let has = v.rs.keys.iter().any(|k| k.0 || k.1.is_some());
                if has && !had && v.fitted && matches!(v.body, ResultBody::Grid) {
                    // header badges arrived after the widths were fitted
                    for c in 0..v.rs.cols.len() {
                        let need = crate::grid::fit_width(&v.rs, c, group, 0, crate::grid::MAX_WIDTH);
                        if let Some(w) = v.grid.widths.get_mut(c) {
                            *w = (*w).max(need);
                        }
                    }
                }
            }
        }
    }

    fn on_job(&mut self, tab_id: u64, job: u64, ev: JobEv) {
        let project = self.project.clone();
        let env = self.env.clone();
        let level = self.level().label();
        let schema = self.schema();
        let group = self.settings.group_digits;
        let pending_sort = if matches!(ev, JobEv::Columns(_)) { self.pending_sort.take() } else { None };
        let Some(idx) = self.tabs.iter().position(|t| t.id == tab_id) else { return };
        let t = &mut self.tabs[idx];
        let Some(run) = &mut t.run else { return };
        if run.job != job {
            return;
        }
        match ev {
            JobEv::Connected(v) => {
                t.push_msg(MsgKind::Info, format!("connected to {project} · {env} · {v}"));
            }
            JobEv::Start(i) => {
                run.cur = i;
                let stmt = run.stmts[i].clone();
                if !matches!(stmt.purpose, Purpose::Silent | Purpose::Commit | Purpose::Delete | Purpose::Insert) {
                    t.results.push(ResultView::new(stmt));
                }
            }
            JobEv::Columns(cols) => {
                let table = t.table.as_ref().map(|x| x.name.clone());
                if let Some(v) = t.results.last_mut().filter(|v| !v.done) {
                    v.rs = ResultSet::new(cols);
                    if let Some(s) = &schema {
                        attach_keys(v, s, table.as_deref());
                    }
                    if v.stmt.purpose == Purpose::Browse {
                        if let Some(tt) = &t.table {
                            v.grid.sort = tt.sort.iter().filter_map(|(n, d)| { v.rs.cols.iter().position(|c| &c.name == n).map(|i| (i, *d))
                                }).collect();
                        }
                        if let Some((_, (_, col))) = pending_sort {
                            v.grid.col = col.min(v.rs.cols.len().saturating_sub(1));
                        }
                    } else if let Some((sort, (_, col))) = pending_sort.filter(|_| v.stmt.server_sort) {
                        v.grid.sort = sort;
                        v.grid.col = col;
                    }
                }
            }
            JobEv::Rows(rows) => {
                run.total_rows += rows.len() as u64;
                if let Some(v) = t.results.last_mut().filter(|v| !v.done) {
                    for r in rows {
                        v.rs.push(r);
                    }
                    if !v.fitted && v.rs.rows > 0 {
                        v.grid.fit_widths(&v.rs, group, 200);
                        v.fitted = true;
                    }
                }
            }
            JobEv::Notice(n) => t.push_msg(MsgKind::Notice, n),
            JobEv::Done { idx: i, done, ms } => {
                let stmt = run.stmts[i].clone();
                let info = sql::classify(&stmt.exec);
                match info.kind {
                    Kind::Begin => t.in_tx = true,
                    Kind::Commit | Kind::Rollback => t.in_tx = false,
                    _ => {}
                }
                let rows = if done.returned_rows { Some(done.rows_returned as i64) } else { done.rows_affected.map(|n| n as i64) };
                if !matches!(stmt.purpose, Purpose::Browse | Purpose::Silent) {
                    let _ = self.store.add_history(&project, &env, level, &stmt.orig, ms as i64, rows, None);
                }
                let t = &mut self.tabs[idx];
                let has_view = t.results.last().is_some_and(|v| !v.done && v.stmt.exec == stmt.exec);
                if done.returned_rows && has_view {
                    let v = t.results.last_mut().unwrap();
                    v.done = true;
                    v.ms = ms;
                    v.rs.complete = true;
                    if let Some(c) = v.stmt.cap {
                        v.rs.truncate(c);
                    }
                    if !v.fitted || v.rs.rows <= 200 {
                        v.grid.fit_widths(&v.rs, group, 200);
                        v.fitted = true;
                    }
                    if let Purpose::Explain { analyze } = v.stmt.purpose {
                        v.body = explain_body(&v.rs, analyze);
                    } else if !v.grid.sort.is_empty() && v.stmt.purpose != Purpose::Browse && !v.stmt.server_sort {
                        v.grid.apply_local_sort(&v.rs);
                    }
                    if v.stmt.purpose == Purpose::Browse
                        && let Some((row, col, top)) = t.restore_cursor.take() {
                            v.grid.row = row;
                            v.grid.col = col;
                            v.grid.top = top;
                        }
                    v.grid.clamp(&v.rs);
                } else {
                    if has_view {
                        t.results.pop();
                    }
                    let tag = match done.rows_affected {
                        Some(n) if !info.verb.is_empty() => { format!("{} {n}", info.verb.to_uppercase())
                        },
                        _ => info.verb.to_uppercase(),
                    };
                    t.push_msg(MsgKind::Info, format!("{tag} · {ms} ms"));
                }
            }
            JobEv::Error { idx: i, err, ms } => {
                run.any_error = true;
                let stmt = run.stmts.get(i).cloned();
                if let Some(stmt) = &stmt
                    && !matches!(stmt.purpose, Purpose::Browse | Purpose::Silent) {
                        let _ = self.store.add_history(&project, &env, level, &stmt.orig, ms as i64, None, Some(&err.message));
                    }
                let t = &mut self.tabs[idx];
                if t.results.last().is_some_and(|v| !v.done) {
                    t.results.pop();
                }
                let mut text = err.message.clone();
                if let Some(d) = &err.detail {
                    text = format!("{text} ({d})");
                }
                t.push_msg(MsgKind::Error, text.clone());
                t.last = Some((false, err.message.clone()));
                if let Some(stmt) = stmt
                    && let Some(off) = stmt.offset {
                        t.error = Some(locate_error(&t.editor, &stmt, off, &err));
                    }
                if t.error.is_none() || !t.editor_visible() {
                    self.toast_err(err.message);
                }
            }
            JobEv::Finished { ms } => {
                let run = t.run.take().unwrap();
                let commit = run.stmts.iter().any(|s| s.purpose == Purpose::Commit);
                let deleted = run.stmts.iter().filter(|s| s.purpose == Purpose::Delete).count();
                let inserted = run.stmts.iter().any(|s| s.purpose == Purpose::Insert);
                let ddl = run.stmts.iter().any(|s| { matches!(sql::classify(&s.exec).kind, Kind::Create | Kind::Alter | Kind::Drop)
                });
                if !run.any_error {
                    let n_views = t.results.len();
                    t.last = Some((
                        true,
                        match t.results.last() {
                            Some(v) if matches!(v.body, ResultBody::Grid) => {
                                format!("{} rows · {}", crate::grid::group_digits(&v.rs.rows.to_string()), fmt_ms(ms))
                            }
                            _ => match t.messages.last() {
                                Some(m) if n_views == 0 => m.text.clone(),
                                _ => fmt_ms(ms),
                            },
                        },
                    ));
                    if t.results.is_empty() && !t.messages.is_empty() && !commit {
                        t.cur_result = 0; // messages sub-tab
                    }
                    if commit
                        && let Some(v) = t.results.first_mut() {
                            let edits = std::mem::take(&mut v.grid.edits);
                            let n = edits.len();
                            for ((r, c), val) in edits {
                                if let Some(col) = v.rs.data.get_mut(c)
                                    && let Some(cell) = col.get_mut(r) {
                                        *cell = val.map(|s| s.into_boxed_str());
                                    }
                            }
                            t.edit_order.clear();
                            self.toast(format!("Committed {n} change{}", if n == 1 { "" } else { "s" }));
                        }
                } else if commit {
                    self.toast_err("commit failed · rolled back · edits kept");
                }
                let t = &mut self.tabs[idx];
                if t.cur_result > t.views().len() {
                    t.cur_result = 0;
                }
                if ddl && self.schemas.contains_key(&self.key()) {
                    self.load_schema();
                }
                if deleted > 0 && !run.any_error {
                    self.toast(format!("Deleted {deleted} row{}", if deleted == 1 { "" } else { "s" }));
                    let t = &mut self.tabs[idx];
                    t.restore_cursor = t.results.first().map(|v| (v.grid.row, v.grid.col, v.grid.top));
                    self.refresh_table(idx);
                }
                if inserted {
                    self.finish_insert(idx, !run.any_error);
                }
            }
        }
    }
}

/// `pk = value and …` identifying a data row by its primary key.
fn pk_where(d: DriverKind, view: &ResultView, pk: &[String], row: usize) -> Result<String, String> {
    let mut wh = Vec::new();
    for k in pk {
        let ci = view.rs.cols.iter().position(|c| &c.name == k).ok_or(format!("primary key {k} not in result"))?;
        wh.push(match view.rs.get(row, ci) {
            Some(v) => format!("{} = {}", db::quote_ident(d, k), db::literal(d, view.rs.cols[ci].kind, Some(v))),
            None => format!("{} is null", db::quote_ident(d, k)),
        });
    }
    Ok(wh.join(" and "))
}

pub fn fmt_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn explain_sql(d: DriverKind, stmt: &str, analyze: bool) -> String {
    let s = sql::trimmed(stmt);
    match (d, analyze) {
        (DriverKind::Postgres, false) => format!("explain (format json) {s}"),
        (DriverKind::Postgres, true) => format!("explain (analyze, format json) {s}"),
        (DriverKind::Sqlite, _) => format!("explain query plan {s}"),
        (DriverKind::Mysql, false) => format!("explain format=tree {s}"),
        (DriverKind::Mysql, true) => format!("explain analyze {s}"),
    }
}

fn explain_body(rs: &ResultSet, analyze: bool) -> ResultBody {
    if rs.cols.len() == 4 && rs.cols[0].name == "id" && rs.cols[1].name == "parent" && rs.cols[3].name == "detail" {
        return ResultBody::Text((0..rs.rows).filter_map(|r| rs.get(r, 3).map(String::from)).collect());
    }
    let text: Vec<&str> = (0..rs.rows).filter_map(|r| rs.get(r, 0)).collect();
    let joined = text.join("\n");
    if joined.trim_start().starts_with('[') {
        match crate::explain::parse_pg_json(&joined, analyze) {
            Ok(p) => ResultBody::Plan(p),
            Err(e) => ResultBody::Text(vec![e]),
        }
    } else {
        let p = crate::explain::parse_mysql_tree(&joined, analyze);
        if p.nodes.is_empty() { ResultBody::Text(joined.lines().map(String::from).collect()) } else { ResultBody::Plan(p) }
    }
}

fn attach_keys(v: &mut ResultView, schema: &Schema, table: Option<&str>) {
    let source = table.map(String::from).or_else(|| {
        let refs = sql::table_refs(&v.stmt.orig);
        if refs.len() == 1 { Some(refs[0].0.clone()) } else { None }
    });
    let Some(source) = source else { return };
    let cols = schema.columns_of(&source);
    if cols.is_empty() {
        return;
    }
    v.rs.keys = v
        .rs
        .cols
        .iter()
        .map(|c| match cols.iter().find(|x| x.name == c.name) {
            Some(x) => (x.pk, x.fk.clone()),
            None => (false, None),
        })
        .collect();
}

/// Map a server error position onto the editor: the token at that spot, on its line.
fn locate_error(ed: &crate::editor::Editor, stmt: &JobStmt, offset: usize, err: &DbError) -> EditorError {
    let orig = &stmt.orig;
    let pos = err.position.map(|p| { orig.char_indices().nth(p).map(|(b, _)| b).unwrap_or(orig.len())
        }).or_else(|| {
        // MySQL: "... near 'snippet' at line N"
        let m = &err.message;
        let i = m.find("near '")? + 6;
        let j = m[i..].rfind("' at line")? + i;
        let snippet = &m[i..j];
        let probe: String = snippet.chars().take(20).collect();
        if probe.is_empty() { None } else { orig.find(&probe) }
    })
    .or_else(|| {
        // "Unknown column 'emial' in 'field list'", "Table 'shop.x' doesn't exist"
        let m = &err.message;
        let i = m.find('\'')? + 1;
        let j = m[i..].find('\'')? + i;
        let name = m[i..j].rsplit('.').next()?;
        if name.is_empty() {
            return None;
        }
        let lower = orig.to_lowercase();
        let needle = name.to_lowercase();
        let mut from = 0;
        while let Some(p) = lower[from..].find(&needle).map(|p| p + from) {
            let before = lower[..p].chars().next_back();
            let after = lower[p + needle.len()..].chars().next();
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if !word(before) && !word(after) {
                return Some(p);
            }
            from = p + needle.len();
        }
        None
    });
    let at = offset + pos.unwrap_or(0);
    let (line, col) = ed.pos_of(at);
    let len = match pos {
        Some(p) => orig[p..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').count().max(1),
        None => ed.line_len(line).saturating_sub(col).max(1),
    };
    EditorError { line, col, len, message: err.message.clone() }
}

fn structure_views(name: &str, schema: Option<&Schema>, st: crate::db::schema::Structure) -> Vec<ResultView> {
    use crate::db::{ColKind, ColumnMeta};
    let text = |n: &str| ColumnMeta { name: n.into(), type_name: "text".into(), kind: ColKind::Text };
    let mut cols_view = ResultView::text("columns", vec![]);
    let mut rs = ResultSet::new(vec![text("column"), text("type"), text("null"), text("key")]);
    if let Some(s) = schema {
        for c in s.columns_of(name) {
            let key = match (&c.pk, &c.fk) {
                (true, _) => "pk".to_string(),
                (_, Some((t, col))) => format!("→ {t}.{col}"),
                _ => String::new(),
            };
            rs.push(vec![Some(c.name.clone().into()), Some(c.type_name.clone().into()), Some(if c.nullable { "null" } else { "not null" }.into()), Some(key.into())]);
        }
    }
    rs.complete = true;
    cols_view.rs = rs;
    cols_view.body = ResultBody::Grid;
    let mut idx = ResultView::text("indexes", vec![]);
    let mut rs = ResultSet::new(vec![text("index"), text("definition")]);
    for (n, d) in st.indexes {
        rs.push(vec![Some(n.into()), Some(d.into())]);
    }
    rs.complete = true;
    idx.rs = rs;
    idx.body = ResultBody::Grid;
    let mut fk = ResultView::text("foreign keys", vec![]);
    let mut rs = ResultSet::new(vec![text("name"), text("definition")]);
    for (n, d) in st.foreign_keys {
        rs.push(vec![Some(n.into()), Some(d.into())]);
    }
    rs.complete = true;
    fk.rs = rs;
    fk.body = ResultBody::Grid;
    let create = ResultView::text("create", st.create.lines().map(String::from).collect());
    let mut out = vec![cols_view, idx, fk, create];
    for v in out.iter_mut() {
        if matches!(v.body, ResultBody::Grid) {
            v.grid.fit_widths(&v.rs, false, 500);
            v.grid.pinned = 0;
            v.fitted = true;
        }
    }
    out
}

pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(h) = dirs::home_dir() {
            return h.join(rest).to_string_lossy().into_owned();
        }
    p.to_string()
}

async fn ensure_session(slot: &mut Slot, params: &ConnectParams, env: &str, want_ro: bool) -> Result<bool, DbError> {
    if slot.env != env || slot.session.as_ref().is_some_and(|s| s.is_closed()) {
        slot.session = None;
    }
    let mut fresh = false;
    if slot.session.is_none() {
        let s = Session::connect(params).await?;
        slot.session = Some(s);
        slot.env = env.to_string();
        slot.read_only = params.read_only;
        fresh = true;
    }
    if slot.read_only != want_ro {
        slot.session.as_mut().unwrap().set_read_only(want_ro).await?;
        slot.read_only = want_ro;
    }
    Ok(fresh)
}

#[allow(clippy::too_many_arguments)]
async fn job_task(
    slot: SlotRef,
    canceller: Arc<std::sync::Mutex<Option<Canceller>>>,
    params: ConnectParams,
    env: String,
    want_ro: bool,
    stmts: Vec<JobStmt>,
    transactional: bool,
    tx: UnboundedSender<Msg>,
    tab: u64,
    job: u64,
) {
    let t0 = Instant::now();
    let send = |ev: JobEv| {
        let _ = tx.send(Msg::Job { tab, job, ev });
    };
    let mut g = slot.lock().await;
    match ensure_session(&mut g, &params, &env, want_ro).await {
        Ok(fresh) => {
            let s = g.session.as_ref().unwrap();
            *canceller.lock().unwrap() = Some(s.canceller());
            if fresh {
                send(JobEv::Connected(s.server_version().to_string()));
            }
        }
        Err(e) => {
            send(JobEv::Error { idx: 0, err: e, ms: t0.elapsed().as_millis() as u64 });
            send(JobEv::Finished { ms: t0.elapsed().as_millis() as u64 });
            return;
        }
    }
    let sess = g.session.as_mut().unwrap();
    if transactional
        && let Err(e) = sess.rows("begin").await {
            send(JobEv::Error { idx: 0, err: e, ms: 0 });
            send(JobEv::Finished { ms: t0.elapsed().as_millis() as u64 });
            return;
        }
    let mut failed = false;
    for (i, st) in stmts.iter().enumerate() {
        send(JobEv::Start(i));
        let started = Instant::now();
        let mut sink = UiSink { tx: tx.clone(), tab, job, buf: Vec::new(), last: Instant::now(), sent: 0 };
        let res = sess.run(&st.exec, &mut sink).await;
        sink.flush_now();
        let ms = started.elapsed().as_millis() as u64;
        match res {
            Ok(done) => {
                if transactional && done.rows_affected == Some(0) {
                    send(JobEv::Error { idx: i, err: DbError::msg("row changed or was deleted since it was loaded (0 rows affected)"), ms });
                    failed = true;
                    break;
                }
                send(JobEv::Done { idx: i, done, ms });
            }
            Err(e) => {
                send(JobEv::Error { idx: i, err: e, ms });
                failed = true;
                break;
            }
        }
    }
    if transactional {
        let r = if failed { sess.rows("rollback").await } else { sess.rows("commit").await };
        if let Err(e) = r {
            send(JobEv::Error { idx: stmts.len().saturating_sub(1), err: e, ms: 0 });
        }
    }
    if sess.is_closed() {
        g.session = None;
    }
    send(JobEv::Finished { ms: t0.elapsed().as_millis() as u64 });
}

/// Batches streamed rows to the UI: small first batch so the first screen draws at once.
struct UiSink {
    tx: UnboundedSender<Msg>,
    tab: u64,
    job: u64,
    buf: Vec<Vec<Cell>>,
    last: Instant,
    sent: usize,
}

impl UiSink {
    fn flush_now(&mut self) {
        if !self.buf.is_empty() {
            self.sent += self.buf.len();
            let rows = std::mem::take(&mut self.buf);
            let _ = self.tx.send(Msg::Job { tab: self.tab, job: self.job, ev: JobEv::Rows(rows) });
        }
        self.last = Instant::now();
    }
}

impl Sink for UiSink {
    fn columns(&mut self, cols: Vec<ColumnMeta>) {
        let _ = self.tx.send(Msg::Job { tab: self.tab, job: self.job, ev: JobEv::Columns(cols) });
    }
    fn row(&mut self, row: Vec<Cell>) {
        self.buf.push(row);
        let first = self.sent == 0 && self.buf.len() >= 100;
        if first || self.buf.len() >= 2000 {
            self.flush_now();
        }
    }
    fn notice(&mut self, msg: String) {
        let _ = self.tx.send(Msg::Job { tab: self.tab, job: self.job, ev: JobEv::Notice(msg) });
    }
    fn flush(&mut self) {
        if self.last.elapsed() > Duration::from_millis(50) {
            self.flush_now();
        }
    }
}

/// Streams rows straight to a file for export.
struct FileSink {
    out: std::io::BufWriter<std::fs::File>,
    fmt: char,
    driver: DriverKind,
    table: String,
    cols: Vec<ColumnMeta>,
    rows: u64,
    tx: UnboundedSender<Msg>,
    err: Option<std::io::Error>,
}

impl FileSink {
    fn new(f: std::fs::File, fmt: char, driver: DriverKind, table: String, tx: UnboundedSender<Msg>) -> FileSink {
        FileSink { out: std::io::BufWriter::new(f), fmt, driver, table, cols: vec![], rows: 0, tx, err: None }
    }
    fn write(&mut self, s: &str) {
        if self.err.is_none()
            && let Err(e) = self.out.write_all(s.as_bytes()) {
                self.err = Some(e);
            }
    }
    fn finish(&mut self) -> std::io::Result<()> {
        if self.fmt == 'j' {
            self.write(if self.rows == 0 { "[]\n" } else { "\n]\n" });
        }
        if let Some(e) = self.err.take() {
            return Err(e);
        }
        self.out.flush()
    }
}

impl Sink for FileSink {
    fn columns(&mut self, cols: Vec<ColumnMeta>) {
        if self.fmt == 'c' {
            let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
            let t = crate::copy::Table { names, kinds: vec![], rows: vec![] };
            let header = crate::copy::render(crate::copy::Format::Csv, &t, self.driver, "");
            self.write(&header);
        }
        self.cols = cols;
    }
    fn row(&mut self, row: Vec<Cell>) {
        let vals: Vec<Option<&str>> = row.iter().map(|c| c.as_deref()).collect();
        let t = crate::copy::Table {
            names: self.cols.iter().map(|c| c.name.as_str()).collect(),
            kinds: self.cols.iter().map(|c| c.kind).collect(),
            rows: vec![vals],
        };
        let line = match self.fmt {
            'c' => crate::copy::render(crate::copy::Format::CsvNoHeader, &t, self.driver, ""),
            'j' => {
                let mut m = serde_json::Map::new();
                for (i, v) in t.rows[0].iter().enumerate() {
                    m.insert(t.names[i].to_string(), crate::copy::json_value(t.kinds[i], *v));
                }
                let obj = serde_json::to_string(&serde_json::Value::Object(m)).unwrap_or_default();
                format!("{}  {obj}", if self.rows == 0 { "[\n" } else { ",\n" })
            }
            _ => { crate::copy::render(crate::copy::Format::Insert, &t, self.driver, &self.table) + "\n"
            },
        };
        self.write(&line);
        self.rows += 1;
        if self.rows.is_multiple_of(5000) {
            let _ = self.tx.send(Msg::Export(ExportEv::Progress(self.rows)));
        }
    }
}

/// A filter chip as SQL. `~word` chips (typed without an operator) search every column.
pub fn filter_sql(d: crate::config::DriverKind, f: &str, cols: &[String]) -> String {
    let Some(word) = f.strip_prefix('~') else {
        return f.to_string();
    };
    if cols.is_empty() {
        return "false".into();
    }
    let pat = crate::sql::quote_literal(&format!(
        "%{}%",
        word.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    ));
    cols.iter()
        .map(|c| {
            let q = db::quote_ident(d, c);
            match d {
                crate::config::DriverKind::Postgres => format!("{q}::text ilike {pat}"),
                _ => format!("cast({q} as char) like {pat}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

/// True when filter text has no comparison, so it is meant as a plain search.
pub fn is_bare_search(text: &str) -> bool {
    let lower = format!(" {} ", text.to_lowercase());
    let ops = [
        "=",
        "<",
        ">",
        "~",
        "!",
        " is ",
        " like ",
        " ilike ",
        " in ",
        " in(",
        " between ",
        " not ",
        "(",
        " exists",
    ];
    !ops.iter().any(|o| lower.contains(o))
}
