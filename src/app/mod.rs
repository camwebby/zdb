//! Application state. Events become actions, actions update this state, and ui::draw
//! renders it. Database work runs in tokio tasks that report back through `Msg`.

pub mod input;
pub mod overlay;
pub mod run;

use crate::config::{Connections, DriverKind, EnvConfig, Level, Settings, UrlParts};
use crate::copy::Clipboard;
use crate::db::{Canceller, Cell, ColumnMeta, DbError, Schema, Session, StmtDone};
use crate::editor::Editor;
use crate::explain::Plan;
use crate::grid::{GridState, ResultSet};
use crate::keys::Keymap;
use crate::store::{Store, TabState, Workspace};
use crate::theme::{Glyphs, Theme};
use overlay::Overlay;
use ratatui::layout::Rect;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

pub type Deferred = Box<dyn FnOnce(&mut App)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Results,
    Sidebar,
    Inspector,
}

impl Focus {
    pub fn label(self, table_tab: bool) -> &'static str {
        match self {
            Focus::Editor => "EDITOR",
            Focus::Results if table_tab => "TABLE",
            Focus::Results => "RESULTS",
            Focus::Sidebar => "SIDEBAR",
            Focus::Inspector => "INSPECTOR",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Query,
    Explain { analyze: bool },
    Browse,
    Commit,
    Delete,
    Silent,
}

pub enum Msg {
    Job { tab: u64, job: u64, ev: JobEv },
    Schema { env: String, res: Result<Schema, String> },
    Structure { tab: u64, res: Result<crate::db::schema::Structure, String> },
    Count { table: String, res: Result<String, String> },
    Export(ExportEv),
    Test { res: Result<String, String> },
}

pub enum JobEv {
    Connected(String),
    Start(usize),
    Columns(Vec<ColumnMeta>),
    Rows(Vec<Vec<Cell>>),
    Notice(String),
    Done { idx: usize, done: StmtDone, ms: u64 },
    Error { idx: usize, err: DbError, ms: u64 },
    Finished { ms: u64 },
}

pub enum ExportEv {
    Progress(u64),
    Done { rows: u64, path: String },
    Failed(String),
}

#[derive(Clone)]
pub struct JobStmt {
    /// SQL sent to the server (after caps, params, wrappers).
    pub exec: String,
    /// SQL as the user wrote it.
    pub orig: String,
    /// Byte offset of `orig` in the editor text, for error underlines.
    pub offset: Option<usize>,
    pub label: String,
    pub cap: Option<usize>,
    pub purpose: Purpose,
    pub server_sort: bool,
}

pub enum ResultBody {
    Grid,
    Plan(Plan),
    Text(Vec<String>),
}

pub struct ResultView {
    pub label: String,
    pub rs: ResultSet,
    pub grid: GridState,
    pub body: ResultBody,
    pub stmt: JobStmt,
    pub ms: u64,
    pub done: bool,
    pub fitted: bool,
}

impl ResultView {
    pub fn new(stmt: JobStmt) -> ResultView {
        ResultView {
            label: stmt.label.clone(),
            rs: ResultSet::default(),
            grid: GridState::default(),
            body: ResultBody::Grid,
            stmt,
            ms: 0,
            done: false,
            fitted: false,
        }
    }
    pub fn text(label: &str, lines: Vec<String>) -> ResultView {
        let stmt = JobStmt {
            exec: String::new(),
            orig: String::new(),
            offset: None,
            label: label.into(),
            cap: None,
            purpose: Purpose::Silent,
            server_sort: false,
        };
        let mut v = ResultView::new(stmt);
        v.body = ResultBody::Text(lines);
        v.done = true;
        v
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Info,
    Notice,
    Error,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub kind: MsgKind,
    pub text: String,
}

pub struct RunState {
    pub job: u64,
    pub started: Instant,
    pub stmts: Vec<JobStmt>,
    pub cur: usize,
    pub cancelling: bool,
    pub any_error: bool,
    pub total_rows: u64,
}

#[derive(Debug, Clone)]
pub struct EditorError {
    pub line: usize,
    pub col: usize,
    pub len: usize,
    pub message: String,
}

pub struct Slot {
    pub session: Option<Session>,
    pub read_only: bool,
    pub env: String,
}

pub type SlotRef = Arc<tokio::sync::Mutex<Slot>>;

pub fn new_slot() -> SlotRef {
    Arc::new(tokio::sync::Mutex::new(Slot { session: None, read_only: false, env: String::new() }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableTab {
    pub name: String,
    pub filters: Vec<String>,
    pub sort: Vec<(String, bool)>,
    pub show_sql: bool,
    pub structure: bool,
}

pub struct Tab {
    pub id: u64,
    pub name: String,
    pub table: Option<TableTab>,
    pub editor: Editor,
    pub results: Vec<ResultView>,
    /// Index into results; `results.len()` means the messages sub-tab.
    pub cur_result: usize,
    pub messages: Vec<Message>,
    pub run: Option<RunState>,
    pub slot: SlotRef,
    pub canceller: Arc<std::sync::Mutex<Option<Canceller>>>,
    pub in_tx: bool,
    pub row_limit: Option<usize>,
    pub saved: Option<String>,
    pub params: BTreeMap<String, String>,
    pub error: Option<EditorError>,
    pub stale_from: Option<String>,
    pub last: Option<(bool, String)>,
    pub edit_order: Vec<(usize, usize)>,
    /// Cursor (row, col, top) to put back once the table reload after a delete finishes.
    pub restore_cursor: Option<(usize, usize, usize)>,
    pub structure: Vec<ResultView>,
    stmt_cache: std::cell::RefCell<(u64, bool, Vec<crate::sql::Stmt>)>,
}

impl Tab {
    pub fn new(id: u64, name: &str, text: &str, vim: bool) -> Tab {
        let mut editor = Editor::new(text);
        editor.set_vim(vim);
        Tab {
            id,
            name: name.to_string(),
            table: None,
            editor,
            results: Vec::new(),
            cur_result: 0,
            messages: Vec::new(),
            run: None,
            slot: new_slot(),
            canceller: Arc::new(std::sync::Mutex::new(None)),
            in_tx: false,
            row_limit: None,
            saved: None,
            params: BTreeMap::new(),
            error: None,
            stale_from: None,
            last: None,
            edit_order: Vec::new(),
            restore_cursor: None,
            structure: Vec::new(),
            stmt_cache: std::cell::RefCell::new((u64::MAX, false, Vec::new())),
        }
    }

    pub fn is_table(&self) -> bool {
        self.table.is_some()
    }

    pub fn editor_visible(&self) -> bool {
        self.table.as_ref().is_none_or(|t| t.show_sql)
    }

    pub fn views(&self) -> &Vec<ResultView> {
        if self.table.as_ref().is_some_and(|t| t.structure) { &self.structure } else { &self.results }
    }

    pub fn views_mut(&mut self) -> &mut Vec<ResultView> {
        if self.table.as_ref().is_some_and(|t| t.structure) { &mut self.structure } else { &mut self.results }
    }

    pub fn view(&self) -> Option<&ResultView> {
        self.views().get(self.cur_result)
    }

    pub fn view_mut(&mut self) -> Option<&mut ResultView> {
        let i = self.cur_result;
        self.views_mut().get_mut(i)
    }

    pub fn on_messages(&self) -> bool {
        self.cur_result >= self.views().len()
    }

    pub fn statements(&self, blank_lines: bool) -> std::cell::Ref<'_, (u64, bool, Vec<crate::sql::Stmt>)> {
        {
            let mut c = self.stmt_cache.borrow_mut();
            if c.0 != self.editor.generation || c.1 != blank_lines {
                *c = (self.editor.generation, blank_lines, crate::sql::split(&self.editor.text(), blank_lines));
            }
        }
        self.stmt_cache.borrow()
    }

    pub fn pending_edits(&self) -> usize {
        self.results.first().map(|v| v.grid.edits.len()).unwrap_or(0)
    }

    pub fn to_state(&self) -> TabState {
        TabState {
            name: self.name.clone(),
            text: self.editor.text(),
            cursor: self.editor.cursor(),
            table: self.table.as_ref().map(|t| t.name.clone()),
            filters: self.table.as_ref().map(|t| t.filters.clone()).unwrap_or_default(),
            sort: self.table.as_ref().map(|t| t.sort.clone()).unwrap_or_default(),
            row_limit: self.row_limit,
            saved: self.saved.clone(),
            params: self.params.clone(),
        }
    }

    pub fn push_msg(&mut self, kind: MsgKind, text: impl Into<String>) {
        self.messages.push(Message { kind, text: text.into() });
        if self.messages.len() > 500 {
            self.messages.drain(..100);
        }
    }
}

pub enum SchemaState {
    Loading,
    Ready(Arc<Schema>),
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub items: Vec<(String, &'static str)>,
    pub sel: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Sidebar {
    pub filter: crate::app::overlay::LineInput,
    pub filtering: bool,
    pub sel: usize,
    pub scroll: usize,
}

#[derive(Debug, Clone, Default)]
pub struct UiLayout {
    pub top: Rect,
    pub env_span: (u16, u16),
    pub tabbar: Rect,
    pub tab_spans: Vec<(usize, u16, u16)>,
    pub editor: Rect,
    pub results_rule: Rect,
    pub result_spans: Vec<(usize, u16, u16)>,
    pub grid: Rect,
    pub status: Rect,
    pub sidebar: Rect,
    pub sidebar_items_y: u16,
    pub inspector: Rect,
    pub gutter: u16,
}

pub struct SpaceMenu {
    pub path: String,
    pub opened: Instant,
}

pub struct Toast {
    pub text: String,
    pub until: Instant,
    pub error: bool,
}

pub struct Export {
    pub rows: u64,
    pub path: String,
    pub cancel: tokio::sync::oneshot::Sender<()>,
}

pub struct App {
    pub settings: Settings,
    pub theme: Theme,
    pub glyphs: Glyphs,
    pub keymap: Keymap,
    pub conns: Connections,
    pub store: Store,
    pub project: String,
    pub env: String,
    pub tabs: Vec<Tab>,
    pub cur: usize,
    pub focus: Focus,
    pub overlay: Option<Overlay>,
    pub space: Option<SpaceMenu>,
    pub sidebar_open: bool,
    pub sidebar: Sidebar,
    pub inspector_open: bool,
    pub inspector_scroll: usize,
    pub zen: bool,
    pub hide_editor: bool,
    pub hide_results: bool,
    pub split: u16,
    pub schemas: HashMap<String, SchemaState>,
    pub meta: HashMap<String, SlotRef>,
    pub unlocked: HashMap<String, Instant>,
    pub passwords: HashMap<String, String>,
    pub toast: Option<Toast>,
    pub tx: UnboundedSender<Msg>,
    pub clipboard: Clipboard,
    pub quit: bool,
    pub next_id: u64,
    pub complete: Option<Completion>,
    pub chord: Option<char>,
    pub layout: UiLayout,
    pub external: Option<External>,
    pub export: Option<Export>,
    pub scratch_unsaved: bool,
    pub drag: Option<Drag>,
    pub size: (u16, u16),
    pub last_input: Instant,
    pub spinner: usize,
    pub test_result: Option<Result<String, String>>,
    pub last_click: Option<(Instant, u16, u16)>,
    pub quit_after_save: bool,
    pub quit_confirmed_edits: bool,
    /// Sort to apply to the grid when a server-side re-sort finishes: (sort, cursor).
    pub pending_sort: Option<PendingSort>,
}

/// Column sort plus the cursor (row, col) to restore.
pub type PendingSort = (Vec<(usize, bool)>, (usize, usize));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Split,
    Grid,
}

/// Work that needs the terminal released (an external editor).
pub enum External {
    EditBuffer,
    EditFile(std::path::PathBuf),
}

pub fn env_key(project: &str, env: &str) -> String {
    format!("{project}/{env}")
}

impl App {
    pub fn new(settings: Settings, conns: Connections, store: Store, tx: UnboundedSender<Msg>) -> App {
        let (theme, theme_err) = Theme::load();
        let (keymap, key_err) = Keymap::load(settings.ascii);
        let glyphs = Glyphs::new(settings.ascii);
        let mut app = App {
            glyphs,
            theme,
            keymap,
            conns,
            store,
            project: String::new(),
            env: String::new(),
            tabs: Vec::new(),
            cur: 0,
            focus: Focus::Editor,
            overlay: None,
            space: None,
            sidebar_open: false,
            sidebar: Sidebar::default(),
            inspector_open: false,
            inspector_scroll: 0,
            zen: false,
            hide_editor: false,
            hide_results: false,
            split: 40,
            schemas: HashMap::new(),
            meta: HashMap::new(),
            unlocked: HashMap::new(),
            passwords: HashMap::new(),
            toast: None,
            tx,
            clipboard: Clipboard::new(),
            quit: false,
            next_id: 1,
            complete: None,
            chord: None,
            layout: UiLayout::default(),
            external: None,
            export: None,
            scratch_unsaved: false,
            drag: None,
            size: (80, 24),
            last_input: Instant::now(),
            spinner: 0,
            test_result: None,
            last_click: None,
            quit_after_save: false,
            quit_confirmed_edits: false,
            pending_sort: None,
            settings,
        };
        if let Some(e) = theme_err.or(key_err) {
            app.toast_err(e);
        }
        app
    }

    // ---- accessors ----

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.cur]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.cur]
    }

    pub fn tab_by_id(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn env_config(&self) -> Option<&EnvConfig> {
        self.conns.find(&self.project, &self.env)
    }

    pub fn level(&self) -> Level {
        self.env_config().map(|e| e.level()).unwrap_or(Level::Local)
    }

    pub fn has_connection(&self) -> bool {
        self.env_config().is_some()
    }

    pub fn driver(&self) -> DriverKind {
        self.env_config()
            .and_then(|e| UrlParts::parse(&e.url).ok())
            .and_then(|p| p.driver())
            .unwrap_or(DriverKind::Postgres)
    }

    pub fn key(&self) -> String {
        env_key(&self.project, &self.env)
    }

    /// Writes blocked: read-only environment that has not been unlocked (or whose unlock expired).
    pub fn read_only(&self) -> bool {
        match self.env_config() {
            Some(e) if e.read_only() => !self.writes_unlocked(),
            _ => false,
        }
    }

    pub fn writes_unlocked(&self) -> bool {
        match self.unlocked.get(&self.key()) {
            Some(t) => t.elapsed() < Duration::from_secs(self.settings.prod_unlock_idle_minutes * 60),
            None => false,
        }
    }

    pub fn schema(&self) -> Option<Arc<Schema>> {
        match self.schemas.get(&self.key()) {
            Some(SchemaState::Ready(s)) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some(Toast { text: s.into(), until: Instant::now() + Duration::from_millis(self.settings.toast_ms), error: false });
    }

    pub fn toast_err(&mut self, s: impl Into<String>) {
        self.toast = Some(Toast { text: s.into(), until: Instant::now() + Duration::from_millis(self.settings.toast_ms * 2), error: true });
    }

    pub fn narrow(&self) -> bool {
        self.size.0 < 100
    }

    /// The inspector docks beside the results on wide terminals and floats otherwise.
    pub fn inspector_docked(&self) -> bool {
        self.size.0 >= 120 && !self.zen
    }

    // ---- workspaces ----

    pub fn new_tab_name(&self) -> String {
        let mut n = 1;
        loop {
            let name = format!("untitled-{n}");
            if !self.tabs.iter().any(|t| t.name == name) {
                return name;
            }
            n += 1;
        }
    }

    pub fn add_query_tab(&mut self, name: Option<&str>, text: &str) -> usize {
        let id = self.alloc_id();
        let name = name.map(String::from).unwrap_or_else(|| self.new_tab_name());
        let mut tab = Tab::new(id, &name, text, self.settings.vim);
        tab.editor.doc_end(false);
        self.tabs.push(tab);
        self.cur = self.tabs.len() - 1;
        self.focus = Focus::Editor;
        self.complete = None;
        self.cur
    }

    pub fn workspace(&self) -> Workspace {
        Workspace { env: self.env.clone(), tabs: self.tabs.iter().map(|t| t.to_state()).collect(), current: self.cur, split: self.split }
    }

    pub fn save_workspace(&mut self) {
        if self.project.is_empty() {
            return;
        }
        let ws = self.workspace();
        let _ = self.store.save_workspace(&self.project, &ws);
        let _ = self.store.set("last_project", &self.project);
    }

    /// Switch to a project (restoring its workspace) and optionally a specific environment.
    pub fn open_project(&mut self, project: &str, env: Option<&str>) {
        if !self.project.is_empty() {
            self.save_workspace();
        }
        let Some(p) = self.conns.project(project).cloned() else {
            self.toast_err(format!("no project named {project}"));
            return;
        };
        self.project = p.name.clone();
        let ws = self.store.workspace(&p.name).ok().flatten();
        self.tabs.clear();
        self.cur = 0;
        let mut target_env = env.map(String::from).or_else(|| ws.as_ref().map(|w| w.env.clone()));
        if !target_env.as_ref().is_some_and(|e| p.envs.iter().any(|x| &x.name == e)) {
            // default: the first non-production environment
            target_env = p.envs.iter().find(|e| e.level() != Level::Prod).or(p.envs.first()).map(|e| e.name.clone());
        }
        self.env = target_env.unwrap_or_default();
        if let Some(ws) = ws {
            self.split = ws.split.clamp(15, 85);
            for t in &ws.tabs {
                self.restore_tab(t);
            }
            self.cur = ws.current.min(self.tabs.len().saturating_sub(1));
        }
        if self.tabs.is_empty() {
            self.add_query_tab(None, "");
        }
        self.focus = if self.tab().editor_visible() { Focus::Editor } else { Focus::Results };
        self.sidebar.sel = 0;
        // table tabs reload their data on demand
        for i in 0..self.tabs.len() {
            if self.tabs[i].is_table() && i == self.cur {
                self.refresh_table(i);
            }
        }
        let _ = self.store.set("last_project", &self.project);
    }

    fn restore_tab(&mut self, t: &TabState) {
        let id = self.alloc_id();
        let mut tab = Tab::new(id, &t.name, &t.text, self.settings.vim);
        tab.editor.set_cursor(t.cursor.0, t.cursor.1);
        tab.row_limit = t.row_limit;
        tab.saved = t.saved.clone();
        tab.params = t.params.clone();
        if let Some(name) = &t.table {
            tab.table = Some(TableTab { name: name.clone(), filters: t.filters.clone(), sort: t.sort.clone(), show_sql: false, structure: false });
        }
        self.tabs.push(tab);
    }

    /// Switch environment within the project; tabs stay, results go stale.
    pub fn switch_env(&mut self, env: &str) {
        if env == self.env {
            return;
        }
        let Some(_) = self.conns.find(&self.project, env) else { return };
        let open_tx: Vec<String> = self.tabs.iter().filter(|t| t.in_tx).map(|t| t.name.clone()).collect();
        if !open_tx.is_empty() {
            let env = env.to_string();
            self.confirm(
                "Open transaction",
                format!("{} has an open transaction on {}. Switching rolls it back.", open_tx.join(", "), self.env),
                String::new(),
                None,
                Box::new(move |app: &mut App| {
                    for t in app.tabs.iter_mut() {
                        t.in_tx = false;
                    }
                    app.do_switch_env(&env)
                }),
            );
            return;
        }
        self.do_switch_env(env);
    }

    fn do_switch_env(&mut self, env: &str) {
        let old = std::mem::replace(&mut self.env, env.to_string());
        for t in self.tabs.iter_mut() {
            if t.run.is_some() {
                if let Some(c) = t.canceller.lock().unwrap().clone() {
                    tokio::spawn(async move {
                        let _ = c.cancel().await;
                    });
                }
                t.run = None;
            }
            if !t.results.is_empty() {
                t.stale_from = Some(old.clone());
            }
            t.slot = new_slot();
            *t.canceller.lock().unwrap() = None;
        }
        let level = self.level();
        self.toast(format!("{} · {}{}", self.project, self.env, if level == Level::Prod { " · read-only until Space c w" } else { "" }));
        if self.tab().is_table() {
            let i = self.cur;
            self.refresh_table(i);
        }
    }

    pub fn confirm(&mut self, title: &str, body: String, sql: String, require: Option<String>, then: Deferred) {
        self.overlay = Some(Overlay::Confirm(overlay::Confirm {
            title: title.to_string(),
            target: format!("{} · {}", self.project, self.env),
            body,
            sql,
            require,
            typed: overlay::LineInput::default(),
            prod: self.level() == Level::Prod,
            then: Some(then),
            on_cancel: None,
            scroll: 0,
        }));
    }

    // ---- timing ----

    /// When the loop should wake up even without input.
    pub fn next_deadline(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut d: Option<Duration> = None;
        let mut take = |x: Duration| d = Some(d.map_or(x, |y| y.min(x)));
        if let Some(t) = &self.toast {
            take(t.until.saturating_duration_since(now));
        }
        if let Some(s) = &self.space {
            let reveal = s.opened + Duration::from_millis(self.settings.space_menu_delay_ms);
            if reveal > now {
                take(reveal - now);
            }
        }
        if self.tabs.iter().any(|t| t.run.is_some()) || self.export.is_some() || self.schemas.values().any(|s| matches!(s, SchemaState::Loading)) {
            take(Duration::from_millis(100));
        }
        if let Some(t) = self.unlocked.get(&self.key()) {
            let exp = *t + Duration::from_secs(self.settings.prod_unlock_idle_minutes * 60);
            if exp > now {
                take(exp - now);
            }
        }
        d
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        if self.toast.as_ref().is_some_and(|t| t.until <= now) {
            self.toast = None;
        }
        self.spinner = self.spinner.wrapping_add(1);
        let key = self.key();
        if let Some(t) = self.unlocked.get(&key)
            && t.elapsed() >= Duration::from_secs(self.settings.prod_unlock_idle_minutes * 60) {
                self.unlocked.remove(&key);
                self.toast(format!("writes locked again on {} after {} idle minutes", self.env, self.settings.prod_unlock_idle_minutes));
            }
    }

    /// Any input counts as activity for the production unlock timer.
    pub fn touch(&mut self) {
        self.last_input = Instant::now();
        let key = self.key();
        if self.writes_unlocked() {
            self.unlocked.insert(key, Instant::now());
        }
    }

    pub fn reload_config(&mut self) {
        if let Ok(s) = Settings::load() {
            let vim_changed = s.vim != self.settings.vim;
            self.settings = s;
            if vim_changed {
                let v = self.settings.vim;
                for t in self.tabs.iter_mut() {
                    t.editor.set_vim(v);
                }
            }
        }
        let (km, e1) = Keymap::load(self.settings.ascii);
        self.keymap = km;
        let (th, e2) = Theme::load();
        self.theme = th;
        self.glyphs = Glyphs::new(self.settings.ascii);
        match e1.or(e2) {
            Some(e) => self.toast_err(e),
            None => self.toast("settings reloaded"),
        }
    }
}
