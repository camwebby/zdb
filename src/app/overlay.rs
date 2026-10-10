//! Modal overlays (layer 1 and 2): palette, environment popup, help, confirmations,
//! one-line prompts, the parameter form, the connection form and the edit review.

use super::*;
use crate::config::{EnvConfig, Level, UrlParts};
use crate::keys::{Action, COMMANDS};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as MatchConfig, Matcher, Utf32Str};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineInput {
    pub text: String,
    pub cursor: usize,
}

impl LineInput {
    pub fn new(s: &str) -> LineInput {
        LineInput { text: s.to_string(), cursor: s.chars().count() }
    }
    fn byte(&self, c: usize) -> usize {
        self.text.char_indices().nth(c).map(|(b, _)| b).unwrap_or(self.text.len())
    }
    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = s.chars().count();
    }
    pub fn insert(&mut self, s: &str) {
        let b = self.byte(self.cursor);
        let s = s.replace(['\n', '\r'], " ");
        self.text.insert_str(b, &s);
        self.cursor += s.chars().count();
    }
    /// Returns true if the key edited or moved within the input.
    pub fn handle(&mut self, k: &KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Char('u') if ctrl => {
                let b = self.byte(self.cursor);
                self.text.replace_range(..b, "");
                self.cursor = 0;
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.text.chars().count(),
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Char('b') if alt => self.word_left(),
            KeyCode::Char('f') if alt => self.word_right(),
            KeyCode::Backspace if alt || ctrl => self.delete_word(),
            KeyCode::Char(c) if !ctrl && !alt => self.insert(&c.to_string()),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let a = self.byte(self.cursor - 1);
                    let b = self.byte(self.cursor);
                    self.text.replace_range(a..b, "");
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.text.chars().count() {
                    let a = self.byte(self.cursor);
                    let b = self.byte(self.cursor + 1);
                    self.text.replace_range(a..b, "");
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            _ => return false,
        }
        true
    }
    fn word_left(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut c = self.cursor;
        while c > 0 && !chars[c - 1].is_alphanumeric() {
            c -= 1;
        }
        while c > 0 && chars[c - 1].is_alphanumeric() {
            c -= 1;
        }
        self.cursor = c;
    }
    fn word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut c = self.cursor;
        while c < chars.len() && !chars[c].is_alphanumeric() {
            c += 1;
        }
        while c < chars.len() && chars[c].is_alphanumeric() {
            c += 1;
        }
        self.cursor = c;
    }
    fn delete_word(&mut self) {
        let end = self.cursor;
        self.word_left();
        let a = self.byte(self.cursor);
        let b = self.byte(end);
        self.text.replace_range(a..b, "");
    }
    pub fn at_end(&self) -> bool {
        self.cursor >= self.text.chars().count()
    }
}

// ---------------- palette ----------------

#[derive(Debug, Clone, PartialEq)]
pub enum PData {
    Conn { project: String, env: String },
    Table(String),
    Cmd(Action),
    Saved { name: String, sql: String },
    History { sql: String },
    Info,
}

#[derive(Debug, Clone)]
pub struct PItem {
    pub prefix: char,
    pub label: String,
    pub hint: String,
    pub hint_level: Option<Level>,
    pub data: PData,
}

pub struct Palette {
    pub input: LineInput,
    pub items: Vec<PItem>,
    pub shown: Vec<usize>,
    pub sel: usize,
    pub scroll: usize,
    pub needs_rebuild: bool,
}

impl Palette {
    pub fn filter(&mut self) {
        let text = self.input.text.clone();
        let (prefix, query) = match text.chars().next() {
            Some(c @ ('@' | '#' | '>' | '/' | '!')) => (Some(c), text[1..].trim().to_string()),
            _ => (None, text.trim().to_string()),
        };
        let candidates: Vec<usize> = (0..self.items.len())
            .filter(|i| match prefix {
                Some(p) => self.items[*i].prefix == p,
                None => true,
            })
            .collect();
        if query.is_empty() {
            self.shown = candidates;
        } else {
            let mut m = Matcher::new(MatchConfig::DEFAULT);
            let pat = Pattern::parse(&query, CaseMatching::Ignore, Normalization::Smart);
            let mut buf = Vec::new();
            let mut scored: Vec<(usize, u32)> = candidates
                .into_iter()
                .filter_map(|i| {
                    let it = &self.items[i];
                    let hay = if matches!(it.data, PData::Conn { .. }) { format!("{} {}", it.label, it.hint) } else { it.label.clone() };
                    pat.score(Utf32Str::new(&hay, &mut buf), &mut m).map(|s| (i, s))
                })
                .collect();
            // stable: equal scores keep recency order
            scored.sort_by_key(|x| std::cmp::Reverse(x.1));
            self.shown = scored.into_iter().map(|(i, _)| i).collect();
        }
        self.sel = 0;
        self.scroll = 0;
    }
}

impl App {
    pub fn open_palette(&mut self, prefix: &str) {
        let mut p = Palette { input: LineInput::new(prefix), items: self.palette_items(), shown: vec![], sel: 0, scroll: 0, needs_rebuild: false };
        p.filter();
        self.overlay = Some(Overlay::Palette(p));
        self.space = None;
        self.ensure_schema();
    }

    pub fn palette_items(&mut self) -> Vec<PItem> {
        let mut items = Vec::new();
        // connections, current project first
        let mut projects = self.conns.projects.clone();
        projects.sort_by_key(|p| p.name != self.project);
        for p in &projects {
            for e in &p.envs {
                if p.name == self.project && e.name == self.env {
                    continue;
                }
                items.push(PItem {
                    prefix: '@',
                    label: format!("{} · {}", p.name, e.name),
                    hint: UrlParts::parse(&e.url).map(|u| u.host).unwrap_or_default(),
                    hint_level: Some(e.level()),
                    data: PData::Conn { project: p.name.clone(), env: e.name.clone() },
                });
            }
        }
        match self.schemas.get(&self.key()) {
            Some(SchemaState::Ready(s)) => {
                for t in &s.tables {
                    let hint = match (t.kind, t.rows_estimate) {
                        (crate::db::schema::TableKind::View, _) => "view".to_string(),
                        (_, Some(n)) => { format!("~{} rows", crate::grid::group_digits(&n.to_string()))
                        },
                        _ => String::new(),
                    };
                    items.push(PItem { prefix: '#', label: t.display(), hint, hint_level: None, data: PData::Table(t.display()) });
                }
            }
            Some(SchemaState::Loading) => items.push(PItem { prefix: '#', label: "loading schema…".into(), hint: String::new(), hint_level: None, data: PData::Info }),
            Some(SchemaState::Failed(e)) => items.push(PItem { prefix: '#', label: format!("schema failed: {e}"), hint: String::new(), hint_level: None, data: PData::Info }),
            None => {}
        }
        for c in COMMANDS.iter().filter(|c| !c.title.is_empty()) {
            let label = if c.action == Action::AllowWrites && self.writes_unlocked() { "connection: lock writes" } else { c.title };
            items.push(PItem { prefix: '>', label: label.to_string(), hint: self.keymap.hint(c.action), hint_level: None, data: PData::Cmd(c.action) });
        }
        for q in self.store.saved_queries(&self.project).unwrap_or_default() {
            items.push(PItem {
                prefix: '/',
                label: q.name.clone(),
                hint: q.sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(30).collect(),
                hint_level: None,
                data: PData::Saved { name: q.name, sql: q.sql },
            });
        }
        let levels: HashMap<String, Level> = self
            .conns
            .projects
            .iter()
            .flat_map(|p| { p.envs.iter().map(move |e| (env_key(&p.name, &e.name), e.level()))
            })
            .collect();
        for h in self.store.history(1000).unwrap_or_default() {
            let one_line = h.sql.split_whitespace().collect::<Vec<_>>().join(" ");
            let rows = match (&h.error, h.rows) {
                (Some(_), _) => "error".to_string(),
                (_, Some(n)) => format!("{} rows", crate::grid::group_digits(&n.to_string())),
                _ => String::new(),
            };
            items.push(PItem {
                prefix: '!',
                label: one_line,
                hint: format!("{} · {} · {} · {}", h.env, fmt_date(h.at), crate::app::run::fmt_ms(h.duration_ms.max(0) as u64), rows),
                hint_level: levels.get(&env_key(&h.project, &h.env)).copied().or(Some(match h.level.as_str() {
                    "prod" => Level::Prod,
                    "staging" => Level::Staging,
                    _ => Level::Local,
                })),
                data: PData::History { sql: h.sql },
            });
        }
        items
    }

    pub fn palette_key(&mut self, mut p: Palette, k: KeyEvent) {
        if p.needs_rebuild {
            p.items = self.palette_items();
            p.needs_rebuild = false;
            p.filter();
        }
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return,
            KeyCode::Up | KeyCode::BackTab => p.sel = p.sel.saturating_sub(1),
            KeyCode::Char('p') if k.modifiers.contains(KeyModifiers::CONTROL) => { p.sel = p.sel.saturating_sub(1)
            },
            KeyCode::Down | KeyCode::Tab => { p.sel = (p.sel + 1).min(p.shown.len().saturating_sub(1))
            },
            KeyCode::Char('n') if k.modifiers.contains(KeyModifiers::CONTROL) => { p.sel = (p.sel + 1).min(p.shown.len().saturating_sub(1))
            },
            KeyCode::PageDown => p.sel = (p.sel + 10).min(p.shown.len().saturating_sub(1)),
            KeyCode::PageUp => p.sel = p.sel.saturating_sub(10),
            KeyCode::Enter => {
                if let Some(it) = p.shown.get(p.sel).map(|i| p.items[*i].clone()) {
                    self.palette_choose(it, alt);
                }
                return;
            }
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(PData::Saved { name, .. }) = p.shown.get(p.sel).map(|i| p.items[*i].data.clone()) {
                    match self.store.delete_query(&self.project, &name) {
                        Ok(()) => {
                            self.toast(format!("deleted saved query {name}"));
                            p.items = self.palette_items();
                            p.filter();
                            p.sel = p.sel.min(p.shown.len().saturating_sub(1));
                        }
                        Err(e) => self.toast_err(e.to_string()),
                    }
                }
            }
            KeyCode::Char('k') | KeyCode::Char('o') | KeyCode::Char('e')
                if k.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                // the same key again closes it
                return;
            }
            _ => {
                if p.input.handle(&k) {
                    p.filter();
                }
            }
        }
        if p.sel < p.scroll {
            p.scroll = p.sel;
        } else if p.sel >= p.scroll + 10 {
            p.scroll = p.sel + 1 - 10;
        }
        self.overlay = Some(Overlay::Palette(p));
    }

    fn palette_choose(&mut self, it: PItem, alt: bool) {
        match it.data {
            PData::Conn { project, env } => {
                if project == self.project {
                    self.switch_env(&env);
                } else {
                    self.open_project(&project, Some(&env));
                }
            }
            PData::Table(name) => {
                if alt && self.tab().editor_visible() {
                    let d = self.driver();
                    let q = name.split('.').map(|p| crate::db::quote_ident(d, p)).collect::<Vec<_>>().join(".");
                    self.tab_mut().editor.insert_str(&q);
                    self.focus = Focus::Editor;
                } else {
                    self.open_table(&name, None);
                }
            }
            PData::Cmd(a) => self.do_action(a),
            PData::Saved { name, sql } => {
                self.add_query_tab(Some(&name), &sql);
                self.tab_mut().saved = Some(name);
            }
            PData::History { sql } => {
                self.add_query_tab(None, &sql);
            }
            PData::Info => {}
        }
    }

    // ---------------- environment popup ----------------

    pub fn env_popup_key(&mut self, mut sel: usize, k: KeyEvent) {
        let envs: Vec<String> = self.conns.project(&self.project).map(|p| p.envs.iter().map(|e| e.name.clone()).collect()).unwrap_or_default();
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Char('e') if k.modifiers.contains(KeyModifiers::CONTROL) => return,
            KeyCode::Up | KeyCode::Char('k') => sel = sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => sel = (sel + 1).min(envs.len().saturating_sub(1)),
            KeyCode::Enter => {
                if let Some(e) = envs.get(sel) {
                    self.switch_env(&e.clone());
                }
                return;
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let n = c.to_digit(10).unwrap() as usize;
                if n >= 1
                    && let Some(e) = envs.get(n - 1) {
                        self.switch_env(&e.clone());
                        return;
                    }
            }
            _ => {}
        }
        self.overlay = Some(Overlay::Env(sel));
    }

    // ---------------- prompts ----------------

    pub fn prompt_key(&mut self, mut p: Prompt, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => {
                if matches!(p.kind, PromptKind::Find)
                    && let Some(v) = self.tab_mut().view_mut() {
                        v.grid.find = None;
                    }
                return;
            }
            KeyCode::Char('c') if ctrl => return,
            KeyCode::Enter => {
                self.prompt_submit(p);
                return;
            }
            KeyCode::Char('n') if ctrl && matches!(p.kind, PromptKind::EditCell { .. }) => {
                p.null = !p.null;
            }
            KeyCode::Char('s') if ctrl && matches!(p.kind, PromptKind::Password { .. }) => {
                if let PromptKind::Password { store, .. } = &mut p.kind {
                    *store = !*store;
                }
            }
            KeyCode::Tab => {
                if let Some(c) = p.completions.get(p.comp_idx).cloned() {
                    p.apply_completion(&c);
                    p.comp_idx = (p.comp_idx + 1) % p.completions.len().max(1);
                    if matches!(p.kind, PromptKind::ExportPath { .. }) {
                        p.completions = path_completions(&p.input.text);
                        p.comp_idx = 0;
                    }
                }
            }
            _ => {
                if p.input.handle(&k) {
                    p.null = false;
                    self.prompt_changed(&mut p);
                }
            }
        }
        self.overlay = Some(Overlay::Prompt(p));
    }

    fn prompt_changed(&mut self, p: &mut Prompt) {
        match &p.kind {
            PromptKind::Find => {
                let q = p.input.text.clone();
                if let Some(v) = self.tab_mut().view_mut() {
                    v.grid.run_find(&v.rs, &q);
                }
            }
            PromptKind::Filter => {
                let word: String = p.input.text.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<Vec<_>>().into_iter().rev().collect();
                let names: Vec<String> = self.tab().view().map(|v| v.rs.cols.iter().map(|c| c.name.clone()).collect()).unwrap_or_default();
                p.completions = if word.is_empty() { vec![] } else { names.into_iter().filter(|n| { n.to_lowercase().starts_with(&word.to_lowercase()) && *n != word
                        }).collect() };
                p.comp_idx = 0;
            }
            PromptKind::ExportPath { .. } => {
                p.completions = path_completions(&p.input.text);
                p.comp_idx = 0;
            }
            _ => {}
        }
    }

    fn prompt_submit(&mut self, p: Prompt) {
        let text = p.input.text.clone();
        match p.kind {
            PromptKind::Password { store, key, project, env, then } => {
                if store && !text.is_empty()
                    && let Err(e) = crate::secrets::keychain_set(&project, &env, &text) {
                        self.toast_err(format!("{e} · password kept for this session"));
                    }
                self.passwords.insert(key, text);
                if let Some(f) = then {
                    f(self);
                }
            }
            PromptKind::Find => {
                if let Some(v) = self.tab_mut().view_mut() {
                    v.grid.run_find(&v.rs, &text);
                    let n = v.grid.find.as_ref().map(|f| f.matches.len()).unwrap_or(0);
                    if n == 0 && !text.is_empty() {
                        self.toast(format!("no match for {text}"));
                    }
                }
            }
            PromptKind::Filter => {
                if text.trim().is_empty() {
                    return;
                }
                let i = self.cur;
                if let Some(t) = &mut self.tabs[i].table {
                    let text = text.trim();
                    t.filters.push(if crate::app::run::is_bare_search(text) {
                        format!("~{text}")
                    } else {
                        text.to_string()
                    });
                }
                self.refresh_table(i);
            }
            PromptKind::EditCell { row, col } => {
                let val = if p.null { None } else { Some(text) };
                let t = self.tab_mut();
                if let Some(v) = t.results.first_mut() {
                    let original = v.rs.get(row, col).map(String::from);
                    if original == val {
                        v.grid.edits.remove(&(row, col));
                        t.edit_order.retain(|x| *x != (row, col));
                    } else {
                        v.grid.edits.insert((row, col), val);
                        t.edit_order.retain(|x| *x != (row, col));
                        t.edit_order.push((row, col));
                    }
                }
            }
            PromptKind::SaveQuery => {
                let name = text.trim().to_string();
                if name.is_empty() {
                    return;
                }
                let sql = self.tab().editor.text();
                match self.store.save_query(&self.project, &name, &sql) {
                    Ok(()) => {
                        let t = self.tab_mut();
                        t.name = name.clone();
                        t.saved = Some(name.clone());
                        let msg = format!("Saved {name} to {}", self.project);
                        self.toast(msg);
                    }
                    Err(e) => self.toast_err(e.to_string()),
                }
            }
            PromptKind::ExportPath { fmt } => {
                if !text.trim().is_empty() {
                    self.start_export(fmt, text.trim().to_string());
                }
            }
        }
    }

    // ---------------- confirm ----------------

    pub fn confirm_key(&mut self, mut c: Confirm, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                if let Some(f) = c.on_cancel.take() {
                    f(self);
                }
                return;
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return,
            KeyCode::Enter => {
                let ok = match &c.require {
                    Some(r) => c.typed.text.trim() == r,
                    None => true,
                };
                if ok {
                    if let Some(f) = c.then.take() {
                        f(self);
                    }
                    return;
                }
                self.toast_err(format!("type {} to confirm", c.require.clone().unwrap_or_default()));
            }
            KeyCode::Up => c.scroll = c.scroll.saturating_sub(1),
            KeyCode::Down => c.scroll += 1,
            _ => {
                if c.require.is_some() {
                    c.typed.handle(&k);
                }
            }
        }
        self.overlay = Some(Overlay::Confirm(c));
    }

    // ---------------- params ----------------

    pub fn params_key(&mut self, mut f: ParamForm, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return,
            KeyCode::Tab | KeyCode::Down => f.idx = (f.idx + 1) % f.values.len(),
            KeyCode::BackTab | KeyCode::Up => f.idx = (f.idx + f.values.len() - 1) % f.values.len(),
            KeyCode::Enter => {
                if f.idx + 1 < f.values.len() {
                    f.idx += 1;
                } else {
                    let vals: BTreeMap<String, String> = f.names.iter().cloned().zip(f.values.iter().map(|v| v.text.clone())).collect();
                    if let Some(then) = f.then.take() {
                        then(self, vals);
                    }
                    return;
                }
            }
            _ => {
                f.values[f.idx].handle(&k);
            }
        }
        self.overlay = Some(Overlay::Params(f));
    }

    // ---------------- insert ----------------

    pub fn insert_key(&mut self, mut f: InsertForm, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let n = f.fields.len();
        f.error = None;
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Char('c') if ctrl => return,
            KeyCode::Char('s') if ctrl => return self.submit_insert(f),
            KeyCode::Enter if f.idx + 1 == n => return self.submit_insert(f),
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down => f.idx = (f.idx + 1) % n,
            KeyCode::BackTab | KeyCode::Up => f.idx = (f.idx + n - 1) % n,
            KeyCode::Char('n') if ctrl => f.fields[f.idx].cycle_blank(),
            _ => {
                let field = &mut f.fields[f.idx];
                if field.input.handle(&k) && !field.input.text.is_empty() {
                    field.blank = Blank::Default;
                }
            }
        }
        self.overlay = Some(Overlay::Insert(Box::new(f)));
    }

    fn submit_insert(&mut self, mut f: InsertForm) {
        if let Some(i) = f.missing() {
            f.error = Some(format!("{} is required", f.fields[i].name));
            f.idx = i;
            self.overlay = Some(Overlay::Insert(Box::new(f)));
            return;
        }
        // production asks for the environment name, like every other write
        if self.level() == Level::Prod {
            let back = f.clone();
            let body = format!("this inserts a row into {}", f.table);
            let sql = format!("{};", f.statement());
            self.confirm("Insert", body, sql, Some(self.env.clone()), Box::new(move |app: &mut App| app.run_insert(f)));
            if let Some(Overlay::Confirm(c)) = &mut self.overlay {
                c.on_cancel = Some(Box::new(move |app: &mut App| app.overlay = Some(Overlay::Insert(Box::new(back)))));
            }
            return;
        }
        self.run_insert(f);
    }

    // ---------------- review ----------------

    pub fn review_key(&mut self, mut r: Review, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                self.commit_edits();
                return;
            }
            KeyCode::Up | KeyCode::Char('k') => r.scroll = r.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => r.scroll += 1,
            KeyCode::Char('y') => {
                let text = r.sql.iter().map(|s| format!("{s};")).collect::<Vec<_>>().join("\n");
                match self.clipboard.set(&text) {
                    Ok(()) => self.toast(format!("Copied {} statements", r.sql.len())),
                    Err(e) => self.toast_err(format!("clipboard: {e}")),
                }
            }
            _ => {}
        }
        self.overlay = Some(Overlay::Review(r));
    }

    // ---------------- help ----------------

    pub fn help_key(&mut self, mut h: Help, k: KeyEvent) {
        if h.searching {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => h.searching = false,
                _ => {
                    h.query.handle(&k);
                    h.scroll = 0;
                }
            }
            self.overlay = Some(Overlay::Help(h));
            return;
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => return,
            KeyCode::Char('/') => h.searching = true,
            KeyCode::Up | KeyCode::Char('k') => h.scroll = h.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => h.scroll += 1,
            KeyCode::PageUp => h.scroll = h.scroll.saturating_sub(10),
            KeyCode::PageDown | KeyCode::Char(' ') => h.scroll += 10,
            _ => {}
        }
        self.overlay = Some(Overlay::Help(h));
    }

    pub fn open_help(&mut self, all: bool) {
        let entries = crate::app::input::help_entries(self, if all { None } else { Some(self.focus) });
        self.overlay = Some(Overlay::Help(Help { entries, query: LineInput::default(), searching: false, scroll: 0, title: if all { "all keys".into() } else { format!("keys · {}", self.focus.label(self.tab().is_table()).to_lowercase()) } }));
        self.space = None;
    }

    // ---------------- connection form ----------------

    pub fn open_conn_form(&mut self, edit: bool) {
        let mut f = ConnForm::new();
        if edit {
            if let Some(e) = self.env_config().cloned() {
                f.fill_from(&self.project, &e);
                f.replacing = Some((self.project.clone(), self.env.clone()));
            } else {
                self.toast("no connection to edit · creating a new one");
            }
        } else if !self.project.is_empty() && self.project != crate::config::SCRATCH {
            f.fields[PROJECT].set(&self.project);
        }
        f.projects = self.conns.projects.iter().map(|p| p.name.clone()).filter(|n| n != crate::config::SCRATCH).collect();
        f.envs = self.conns.projects.iter().flat_map(|p| p.envs.iter().map(|e| e.name.clone())).collect();
        f.envs.sort();
        f.envs.dedup();
        for d in ["local", "staging", "prod"] {
            if !f.envs.iter().any(|e| e == d) {
                f.envs.push(d.into());
            }
        }
        self.test_result = None;
        self.overlay = Some(Overlay::Form(Box::new(f)));
        self.space = None;
    }

    pub fn form_key(&mut self, mut f: Box<ConnForm>, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let order = f.visible();
        let pos = order.iter().position(|i| *i == f.focus).unwrap_or(0);
        match k.code {
            KeyCode::Esc => return,
            KeyCode::Char('c') if ctrl => return,
            KeyCode::Char('s') if ctrl => {
                if self.form_save(&mut f) {
                    return;
                }
            }
            KeyCode::Tab | KeyCode::Down => f.focus = order[(pos + 1) % order.len()],
            KeyCode::BackTab | KeyCode::Up => { f.focus = order[(pos + order.len() - 1) % order.len()]
            },
            KeyCode::Enter | KeyCode::Char(' ') if matches!(f.focus, ADVANCED | READONLY | TEST | SAVE) =>
            { match f.focus {
                ADVANCED => f.advanced = !f.advanced,
                READONLY => {
                    f.read_only = Some(!f.effective_read_only());
                }
                TEST => {
                    self.form_test(&f);
                }
                _ => {
                    if self.form_save(&mut f) {
                        return;
                    };
                    }
                }
            },
            KeyCode::Enter => f.focus = order[(pos + 1) % order.len()],
            KeyCode::Right if matches!(f.focus, PROJECT | ENV) && f.fields[f.focus].at_end() => {
                if let Some(s) = f.suggestion() {
                    f.fields[f.focus].set(&s);
                }
            }
            _ => {
                if f.focus < FIELD_COUNT && f.fields[f.focus].handle(&k) {
                    f.error = None;
                    if f.focus == URL {
                        f.split_url();
                    } else if matches!(f.focus, HOST | PORT | USER | DATABASE | SSL) {
                        f.rebuild_url();
                    }
                }
            }
        }
        self.overlay = Some(Overlay::Form(f));
    }

    fn form_test(&mut self, f: &ConnForm) {
        let (env, password) = match f.to_env() {
            Ok(x) => x,
            Err(e) => {
                self.test_result = Some(Err(e));
                return;
            }
        };
        let parts = UrlParts::parse(&env.url).unwrap();
        let project = f.fields[PROJECT].text.trim().to_string();
        let pw = if password.is_empty() {
            match crate::secrets::lookup(&project, &env, &parts) {
                crate::secrets::Lookup::Found(p) => Some(p),
                _ => self.passwords.get(&env_key(&project, &env.name)).cloned(),
            }
        } else {
            Some(password)
        };
        let params = crate::db::ConnectParams { driver: parts.driver().unwrap(), parts, password: pw, read_only: false, ssh: env.ssh.clone() };
        self.test_result = Some(Ok("testing…".into()));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let res = crate::db::Session::connect(&params).await.map(|s| s.server_version().to_string()).map_err(|e| e.message);
            let _ = tx.send(Msg::Test { res });
        });
    }

    /// Returns true when saved (the form closes).
    fn form_save(&mut self, f: &mut ConnForm) -> bool {
        let (env, password) = match f.to_env() {
            Ok(x) => x,
            Err(e) => {
                f.error = Some(e);
                return false;
            }
        };
        let project = f.fields[PROJECT].text.trim().to_string();
        if !password.is_empty() {
            if env.password_source() == "keychain"
                && let Err(e) = crate::secrets::keychain_set(&project, &env.name, &password) {
                    f.error = Some(format!("{e} · choose another password source under advanced"));
                    f.advanced = true;
                    return false;
                }
            self.passwords.insert(env_key(&project, &env.name), password);
        }
        let replacing = f.replacing.clone();
        let was_scratch = self.project == crate::config::SCRATCH;
        self.conns.upsert(&project, env.clone(), replacing.as_ref().map(|(a, b)| (a.as_str(), b.as_str())));
        if let Some((op, oe)) = &replacing
            && (op, oe) != (&project, &env.name) {
                crate::secrets::keychain_delete(op, oe);
            }
        if let Err(e) = self.conns.save() {
            f.error = Some(format!("could not write connections.toml: {e}"));
            return false;
        }
        self.scratch_unsaved = false;
        // a changed connection needs fresh sessions and schema
        let key = env_key(&project, &env.name);
        self.schemas.remove(&key);
        self.meta.remove(&key);
        if was_scratch {
            // keep the scratch tabs: move them to the new project
            self.conns.projects.retain(|p| p.name != crate::config::SCRATCH);
            self.project = project.clone();
            self.env = env.name.clone();
            for t in self.tabs.iter_mut() {
                t.slot = new_slot();
            }
            self.save_workspace();
        } else if self.project != project || self.env != env.name || replacing.is_some() {
            if self.project == project {
                for t in self.tabs.iter_mut() {
                    t.slot = new_slot();
                }
                self.env.clear();
                self.switch_env(&env.name);
            } else {
                self.open_project(&project, Some(&env.name));
            }
        }
        self.toast(format!("saved {project} · {}", env.name));
        if self.quit_after_save {
            self.quit_now();
        }
        true
    }
}

fn fmt_date(ts: i64) -> String {
    let now = crate::store::now();
    let d = now - ts;
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86400 {
        format!("{}h ago", d / 3600)
    } else if d < 86400 * 14 {
        format!("{}d ago", d / 86400)
    } else {
        // days since epoch → yyyy-mm-dd
        let days = ts / 86400;
        let (y, m, dd) = civil(days);
        format!("{y:04}-{m:02}-{dd:02}")
    }
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn path_completions(text: &str) -> Vec<String> {
    let expanded = super::run::expand_tilde(text);
    let (dir, prefix) = match expanded.rfind('/') {
        Some(i) => (expanded[..=i].to_string(), expanded[i + 1..].to_string()),
        None => ("./".to_string(), expanded.clone()),
    };
    let Ok(rd) = std::fs::read_dir(if dir.is_empty() { "/" } else { &dir }) else { return vec![] };
    let mut out: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let shown_dir = match text.rfind('/') {
                Some(i) => text[..=i].to_string(),
                None => String::new(),
            };
            Some(format!("{shown_dir}{name}{}", if is_dir { "/" } else { "" }))
        })
        .collect();
    out.sort();
    out
}

pub enum Overlay {
    Palette(Palette),
    Env(usize),
    Help(Help),
    Confirm(Confirm),
    Prompt(Prompt),
    Params(ParamForm),
    Form(Box<ConnForm>),
    Review(Review),
    Insert(Box<InsertForm>),
}

pub struct Help {
    pub entries: Vec<(String, String, String)>,
    pub query: LineInput,
    pub searching: bool,
    pub scroll: usize,
    pub title: String,
}

impl Help {
    pub fn visible(&self) -> Vec<&(String, String, String)> {
        let q = self.query.text.to_lowercase();
        self.entries
            .iter()
            .filter(|(k, d, s)| { q.is_empty() || k.to_lowercase().contains(&q) || d.to_lowercase().contains(&q) || s.to_lowercase().contains(&q)
            })
            .collect()
    }
}

pub struct Confirm {
    pub title: String,
    pub target: String,
    pub body: String,
    pub sql: String,
    pub require: Option<String>,
    pub typed: LineInput,
    pub prod: bool,
    pub then: Option<Deferred>,
    pub on_cancel: Option<Deferred>,
    pub scroll: usize,
}

pub enum PromptKind {
    Password { store: bool, key: String, project: String, env: String, then: Option<Deferred> },
    Find,
    Filter,
    EditCell { row: usize, col: usize },
    SaveQuery,
    ExportPath { fmt: char },
}

pub struct Prompt {
    pub kind: PromptKind,
    pub label: String,
    pub input: LineInput,
    pub completions: Vec<String>,
    pub comp_idx: usize,
    pub null: bool,
}

impl Prompt {
    pub fn new(kind: PromptKind, label: &str, initial: &str) -> Prompt {
        Prompt { kind, label: label.to_string(), input: LineInput::new(initial), completions: vec![], comp_idx: 0, null: false }
    }
    pub fn masked(&self) -> bool {
        matches!(self.kind, PromptKind::Password { .. })
    }
    pub fn inline(&self) -> bool {
        matches!(self.kind, PromptKind::Find | PromptKind::Filter | PromptKind::EditCell { .. })
    }
    fn apply_completion(&mut self, c: &str) {
        match self.kind {
            PromptKind::ExportPath { .. } => self.input.set(c),
            _ => {
                // replace the trailing identifier
                let t = &self.input.text;
                let n = t.chars().rev().take_while(|c| c.is_alphanumeric() || *c == '_').count();
                let keep: String = t.chars().take(t.chars().count() - n).collect();
                self.input.set(&format!("{keep}{c}"));
            }
        }
    }
}

pub struct ParamForm {
    pub names: Vec<String>,
    pub values: Vec<LineInput>,
    pub idx: usize,
    pub then: Option<ParamsThen>,
}

pub type ParamsThen = Box<dyn FnOnce(&mut App, BTreeMap<String, String>)>;

impl ParamForm {
    pub fn new(names: Vec<String>, values: Vec<String>, then: ParamsThen) -> ParamForm {
        ParamForm { names, values: values.iter().map(|v| LineInput::new(v)).collect(), idx: 0, then: Some(then) }
    }
}

/// What an empty insert field means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blank {
    /// Leave the column out so the database fills it in.
    Default,
    Null,
    EmptyString,
}

#[derive(Debug, Clone)]
pub struct InsertField {
    pub name: String,
    pub type_name: String,
    pub kind: crate::db::ColKind,
    pub pk: bool,
    /// Referenced `table.column`.
    pub fk: Option<String>,
    pub nullable: bool,
    pub default: Option<String>,
    pub input: LineInput,
    pub blank: Blank,
}

impl InsertField {
    /// Not null with nothing to fall back on, so the user has to supply a value.
    pub fn required(&self) -> bool {
        !self.nullable && self.default.is_none()
    }

    /// `None` leaves the column out of the INSERT, `Some(None)` writes NULL.
    pub fn value(&self) -> Option<Option<String>> {
        if !self.input.text.is_empty() {
            return Some(Some(self.input.text.clone()));
        }
        match self.blank {
            Blank::Default => None,
            Blank::Null => Some(None),
            Blank::EmptyString => Some(Some(String::new())),
        }
    }

    /// Ctrl+N: step an empty field through default → NULL → '' (text columns only).
    fn cycle_blank(&mut self) {
        let text = self.kind == crate::db::ColKind::Text;
        if !self.input.text.is_empty() {
            self.input.set("");
            self.blank = Blank::Null;
            return;
        }
        self.blank = match self.blank {
            Blank::Default => Blank::Null,
            Blank::Null if text => Blank::EmptyString,
            _ => Blank::Default,
        };
    }
}

#[derive(Clone)]
pub struct InsertForm {
    pub tab_id: u64,
    pub table: String,
    pub quoted: String,
    pub driver: crate::config::DriverKind,
    pub fields: Vec<InsertField>,
    pub idx: usize,
    pub scroll: std::cell::Cell<usize>,
    pub error: Option<String>,
}

impl InsertForm {
    pub fn statement(&self) -> String {
        let d = self.driver;
        let (mut cols, mut vals) = (Vec::new(), Vec::new());
        for f in &self.fields {
            if let Some(v) = f.value() {
                cols.push(crate::db::quote_ident(d, &f.name));
                vals.push(crate::db::literal(d, f.kind, v.as_deref()));
            }
        }
        match (cols.is_empty(), d) {
            (true, crate::config::DriverKind::Mysql) => format!("insert into {} () values ()", self.quoted),
            (true, _) => format!("insert into {} default values", self.quoted),
            _ => format!("insert into {} ({}) values ({})", self.quoted, cols.join(", "), vals.join(", ")),
        }
    }

    /// The first required field that has no value.
    pub fn missing(&self) -> Option<usize> {
        self.fields.iter().position(|f| f.required() && !matches!(f.value(), Some(Some(_))))
    }
}

pub struct Review {
    pub sql: Vec<String>,
    pub scroll: usize,
}

pub const URL: usize = 0;
pub const HOST: usize = 1;
pub const PORT: usize = 2;
pub const USER: usize = 3;
pub const PASSWORD: usize = 4;
pub const DATABASE: usize = 5;
pub const PROJECT: usize = 6;
pub const ENV: usize = 7;
pub const SSL: usize = 8;
pub const SSH: usize = 9;
pub const PWSRC: usize = 10;
pub const FIELD_COUNT: usize = 11;
pub const ADVANCED: usize = 100;
pub const READONLY: usize = 101;
pub const TEST: usize = 102;
pub const SAVE: usize = 103;

pub const FIELD_LABELS: [&str; FIELD_COUNT] = ["url", "host", "port", "user", "password", "database", "project", "environment", "sslmode", "ssh tunnel", "password from"];

pub struct ConnForm {
    pub fields: Vec<LineInput>,
    pub focus: usize,
    pub advanced: bool,
    pub read_only: Option<bool>,
    pub replacing: Option<(String, String)>,
    pub projects: Vec<String>,
    pub envs: Vec<String>,
    pub error: Option<String>,
}

impl ConnForm {
    pub fn new() -> ConnForm {
        let mut f = ConnForm { fields: vec![LineInput::default(); FIELD_COUNT], focus: URL, advanced: false, read_only: None, replacing: None, projects: vec![], envs: vec![], error: None };
        f.fields[PWSRC].set("keychain");
        f
    }

    pub fn visible(&self) -> Vec<usize> {
        let sqlite = self.fields[URL].text.trim().starts_with("sqlite://");
        let mut v = if sqlite { vec![URL, DATABASE, PROJECT, ENV, ADVANCED] } else { vec![URL, HOST, PORT, USER, PASSWORD, DATABASE, PROJECT, ENV, ADVANCED] };
        if self.advanced {
            if sqlite { v.push(READONLY); } else { v.extend([SSL, SSH, PWSRC, READONLY]); }
        }
        v.extend([TEST, SAVE]);
        v
    }

    pub fn level(&self) -> Level {
        Level::infer(self.fields[ENV].text.trim())
    }

    pub fn effective_read_only(&self) -> bool {
        self.read_only.unwrap_or(self.level() == Level::Prod)
    }

    pub fn suggestion(&self) -> Option<String> {
        let (list, text) = match self.focus {
            PROJECT => (&self.projects, &self.fields[PROJECT].text),
            ENV => (&self.envs, &self.fields[ENV].text),
            _ => return None,
        };
        if text.is_empty() {
            return None;
        }
        list.iter().find(|n| n.to_lowercase().starts_with(&text.to_lowercase()) && *n != text).cloned()
    }

    pub fn split_url(&mut self) {
        if let Ok(p) = UrlParts::parse(&self.fields[URL].text) {
            self.fields[HOST].set(&p.host);
            self.fields[PORT].set(&p.port);
            self.fields[USER].set(&p.user);
            self.fields[DATABASE].set(&p.database);
            if !p.password.is_empty() {
                self.fields[PASSWORD].set(&p.password);
            }
            self.fields[SSL].set(&p.param("sslmode").or_else(|| p.param("ssl-mode")).unwrap_or_default());
        }
    }

    fn rebuild_url(&mut self) {
        let scheme = UrlParts::parse(&self.fields[URL].text).map(|p| p.scheme).unwrap_or_else(|_| {
            let t = &self.fields[URL].text;
            t.split("://").next().filter(|s| crate::config::driver_for_scheme(s).is_some()).unwrap_or("postgres").to_string()
        });
        let ssl = self.fields[SSL].text.trim().to_string();
        let key = if scheme.starts_with("mysql") || scheme == "mariadb" { "ssl-mode" } else { "sslmode" };
        let p = UrlParts {
            scheme,
            user: self.fields[USER].text.trim().to_string(),
            password: String::new(),
            host: self.fields[HOST].text.trim().to_string(),
            port: self.fields[PORT].text.trim().to_string(),
            database: self.fields[DATABASE].text.trim().to_string(),
            query: if ssl.is_empty() { String::new() } else { format!("{key}={ssl}") },
        };
        self.fields[URL].set(&p.build());
    }

    fn fill_from(&mut self, project: &str, e: &EnvConfig) {
        self.fields[URL].set(&e.url);
        self.split_url();
        self.fields[PROJECT].set(if project == crate::config::SCRATCH { "" } else { project });
        self.fields[ENV].set(if project == crate::config::SCRATCH { "local" } else { &e.name });
        self.fields[SSH].set(e.ssh.as_deref().unwrap_or(""));
        self.fields[PWSRC].set(e.password_source());
        self.read_only = e.read_only;
        if e.ssh.is_some() || e.password.is_some() || e.read_only.is_some() {
            self.advanced = true;
        }
    }

    /// Validated EnvConfig and the password typed (if any).
    pub fn to_env(&self) -> Result<(EnvConfig, String), String> {
        let url_text = self.fields[URL].text.trim();
        if url_text.is_empty() {
            return Err("enter a URL like postgres://user@host:5432/db".into());
        }
        let mut parts = UrlParts::parse(url_text).map_err(|e| e.to_string())?;
        let mut password = self.fields[PASSWORD].text.clone();
        if !parts.password.is_empty() {
            password = std::mem::take(&mut parts.password);
        }
        let project = self.fields[PROJECT].text.trim();
        let env = self.fields[ENV].text.trim();
        if project.is_empty() {
            return Err("give the connection a project name".into());
        }
        if env.is_empty() {
            return Err("give the connection an environment name (local, staging, prod…)".into());
        }
        let src = self.fields[PWSRC].text.trim();
        let valid_src = matches!(src, "" | "keychain" | "prompt" | "pgpass" | "mycnf" | "none") || src.starts_with("env:");
        if !valid_src {
            return Err("password from: keychain, prompt, env:VAR, pgpass, mycnf or none".into());
        }
        let level = Level::infer(env);
        let ro = self.read_only.filter(|r| *r != (level == Level::Prod));
        Ok((
            EnvConfig {
                name: env.to_string(),
                url: parts.build(),
                level: None,
                read_only: ro,
                password: if src.is_empty() || src == "keychain" { None } else { Some(src.to_string()) },
                ssh: Some(self.fields[SSH].text.trim().to_string()).filter(|s| !s.is_empty()),
            },
            password,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20729), (2026, 10, 3));
    }

    #[test]
    fn form_url_sync() {
        let mut f = ConnForm::new();
        f.fields[URL].set("postgres://app:pw@db.staging:5432/shop");
        f.split_url();
        assert_eq!(f.fields[HOST].text, "db.staging");
        assert_eq!(f.fields[PASSWORD].text, "pw");
        f.fields[DATABASE].set("shop2");
        f.rebuild_url();
        assert_eq!(f.fields[URL].text, "postgres://app@db.staging:5432/shop2");
        f.fields[PROJECT].set("shop-api");
        f.fields[ENV].set("prod");
        assert!(f.effective_read_only());
        let (e, pw) = f.to_env().unwrap();
        assert_eq!(pw, "pw");
        assert_eq!(e.url, "postgres://app@db.staging:5432/shop2");
        assert_eq!(e.level(), Level::Prod);
        assert!(e.read_only());
    }
}
