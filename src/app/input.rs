//! Keyboard and mouse input: routing to overlays, the Space menu, the editor and panes,
//! and every action's behaviour.

use super::overlay::{LineInput, Overlay, Prompt, PromptKind, Review};
use super::*;
use crate::config::{Level, MenuMode};
use crate::copy::Format;
use crate::editor::Outcome;
use crate::grid::{SelKind, Selection};
use crate::keys::{Action, COMMANDS, Ctx, menu};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

impl App {
    pub fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        self.touch();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);

        if let Some(o) = self.overlay.take() {
            match o {
                Overlay::Palette(p) => self.palette_key(p, k),
                Overlay::Env(sel) => self.env_popup_key(sel, k),
                Overlay::Help(h) => self.help_key(h, k),
                Overlay::Confirm(c) => self.confirm_key(c, k),
                Overlay::Prompt(p) => self.prompt_key(p, k),
                Overlay::Params(f) => self.params_key(f, k),
                Overlay::Form(f) => self.form_key(f, k),
                Overlay::Review(r) => self.review_key(r, k),
            }
            return;
        }

        if let Some(c) = self.chord.take()
            && c == 'x' && ctrl && k.code == KeyCode::Char('e') {
                self.do_action(Action::ExternalEditor);
                return;
            }

        if self.space.is_some() {
            self.menu_key(k);
            return;
        }

        // completion popup gets first say over navigation keys
        if self.focus == Focus::Editor && self.complete.is_some() {
            match k.code {
                KeyCode::Tab => {
                    self.accept_completion();
                    return;
                }
                KeyCode::Esc => {
                    self.complete = None;
                    return;
                }
                KeyCode::Up => {
                    let c = self.complete.as_mut().unwrap();
                    c.sel = c.sel.checked_sub(1).unwrap_or(c.items.len().saturating_sub(1));
                    return;
                }
                KeyCode::Down => {
                    let c = self.complete.as_mut().unwrap();
                    c.sel = (c.sel + 1) % c.items.len().max(1);
                    return;
                }
                _ => {}
            }
        }

        if self.focus == Focus::Editor && ctrl && k.code == KeyCode::Char('x') {
            self.chord = Some('x');
            return;
        }

        // macOS terminals send Cmd+Left/Right as ^A/^E, so the editor keeps them as
        // line start/end; switch environment from the editor with the palette or ^O
        if self.focus == Focus::Editor && ctrl && matches!(k.code, KeyCode::Char('a' | 'e')) && self.tab().editor_visible() {
            return self.editor_key(k);
        }

        if let Some(a) = self.keymap.lookup(Ctx::Global, &k) {
            self.do_action(a);
            return;
        }

        match self.focus {
            Focus::Editor => self.editor_key(k),
            Focus::Sidebar => self.sidebar_key(k),
            Focus::Inspector => self.inspector_key(k),
            Focus::Results => self.pane_key(k),
        }
    }

    fn editor_key(&mut self, k: KeyEvent) {
        if !self.tab().editor_visible() {
            self.focus = Focus::Results;
            return self.pane_key(k);
        }
        let vim_normal = self.tab().editor.vim.is_some() && self.tab().editor.vim_mode() != Some(crate::editor::VimMode::Insert);
        let before = self.tab().editor.generation;
        let out = self.tab_mut().editor.handle_key(k);
        match out {
            Outcome::Leave => self.leave_editor(),
            Outcome::NotHandled if k.code == KeyCode::Esc => self.leave_editor(),
            Outcome::NotHandled if vim_normal && k.code == KeyCode::Char(' ') => self.open_space_menu(""),
            _ => {}
        }
        let changed = self.tab().editor.generation != before;
        if changed {
            self.tab_mut().error = None;
            let typed = matches!(k.code, KeyCode::Char(c) if c.is_alphanumeric() || c == '_' || c == '.');
            if typed || (self.complete.is_some() && k.code == KeyCode::Backspace) {
                self.update_completion();
            } else {
                self.complete = None;
            }
        } else if !matches!(k.code, KeyCode::Char(_)) {
            self.complete = None;
        }
    }

    fn leave_editor(&mut self) {
        self.complete = None;
        self.focus = Focus::Results;
    }

    fn focus_editor(&mut self) {
        if self.tab().editor_visible() {
            self.hide_editor = false;
            self.focus = Focus::Editor;
        } else {
            self.toast("no editor in a table tab · Space t q shows its SQL");
        }
    }

    // ---------------- Space menu ----------------

    pub fn open_space_menu(&mut self, path: &str) {
        if self.settings.space_menu == MenuMode::Off && path.is_empty() {
            // menu hidden, but the keys still work
        }
        self.space = Some(SpaceMenu { path: path.to_string(), opened: Instant::now() });
    }

    pub fn space_visible(&self) -> bool {
        match (&self.space, &self.settings.space_menu) {
            (None, _) => false,
            (Some(_), MenuMode::Off) => false,
            (Some(_), MenuMode::Instant) => true,
            (Some(s), MenuMode::Delay) => s.opened.elapsed() >= Duration::from_millis(self.settings.space_menu_delay_ms),
        }
    }

    fn menu_key(&mut self, k: KeyEvent) {
        let path = self.space.as_ref().unwrap().path.clone();
        match k.code {
            KeyCode::Esc => {
                self.space = None;
            }
            KeyCode::Backspace => {
                let mut p = path.clone();
                p.pop();
                if path.is_empty() {
                    self.space = None;
                } else {
                    self.space = Some(SpaceMenu { path: p, opened: Instant::now() - Duration::from_secs(1) });
                }
            }
            KeyCode::Char(c) => {
                let (_, items) = menu(&path);
                if let Some(it) = items.iter().find(|it| it.key == c) {
                    if let Some(sub) = it.submenu {
                        let visible_already = self.space_visible();
                        let opened = if visible_already { Instant::now() - Duration::from_secs(1) } else { self.space.as_ref().unwrap().opened };
                        self.space = Some(SpaceMenu { path: sub.to_string(), opened });
                    } else if let Some(a) = it.action {
                        self.space = None;
                        self.do_action(a);
                    }
                    return;
                }
                self.space = None;
                self.toast(format!("no action on {c} here · ? lists keys"));
            }
            _ => self.space = None,
        }
    }

    // ---------------- panes ----------------

    fn pane_key(&mut self, k: KeyEvent) {
        // gd follows a foreign key from the cell under the cursor before `g` moved it
        if k.code == KeyCode::Char('d') && k.modifiers.is_empty()
            && let Some(pos) = self.tab_mut().view_mut().and_then(|v| v.grid.before_g.take()) {
                if let Some(v) = self.tab_mut().view_mut() {
                    v.grid.row = pos.0;
                    v.grid.col = pos.1;
                }
                self.do_action(Action::FollowFk);
                return;
            }
        if let Some(v) = self.tab_mut().view_mut() {
            v.grid.before_g = None;
        }
        if self.grid_select_key(k) {
            return;
        }
        if let Some(a) = self.keymap.lookup(Ctx::Pane, &k) {
            if a == Action::Top {
                let pos = self.tab().view().map(|v| (v.grid.row, v.grid.col));
                self.do_action(a);
                if let Some(v) = self.tab_mut().view_mut() {
                    v.grid.before_g = pos;
                }
                return;
            }
            self.do_action(a);
        }
    }

    fn sidebar_key(&mut self, k: KeyEvent) {
        if self.sidebar.filtering {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => self.sidebar.filtering = false,
                _ => {
                    self.sidebar.filter.handle(&k);
                    self.sidebar.sel = 0;
                }
            }
            return;
        }
        let items = self.sidebar_items();
        let n = items.len();
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.sidebar.sel = self.sidebar.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.sidebar.sel = (self.sidebar.sel + 1).min(n.saturating_sub(1)),
            KeyCode::Char('g') => self.sidebar.sel = 0,
            KeyCode::Char('G') => self.sidebar.sel = n.saturating_sub(1),
            KeyCode::Char('/') => self.sidebar.filtering = true,
            KeyCode::Enter | KeyCode::Char('s') => {
                let structure = k.code == KeyCode::Char('s');
                if let Some(it) = items.get(self.sidebar.sel).cloned() {
                    self.sidebar_open_item(it, structure);
                }
            }
            KeyCode::Esc => {
                if !self.sidebar.filter.text.is_empty() {
                    self.sidebar.filter = LineInput::default();
                } else {
                    self.focus = Focus::Results;
                    if self.narrow() {
                        self.sidebar_open = false;
                    }
                }
            }
            _ => {
                if let Some(a) = self.keymap.lookup(Ctx::Pane, &k)
                    && matches!(a, Action::SpaceMenu | Action::Help | Action::FocusEditor | Action::NextPane | Action::PrevPane) {
                        self.do_action(a);
                    }
            }
        }
    }

    fn sidebar_open_item(&mut self, it: SidebarItem, structure: bool) {
        match it {
            SidebarItem::Table(name) => {
                self.open_table(&name, None);
                if structure {
                    self.do_action(Action::Structure);
                }
            }
            SidebarItem::Saved(name, sql) => {
                self.add_query_tab(Some(&name), &sql);
                self.tab_mut().saved = Some(name);
            }
            SidebarItem::Header(_) => {}
        }
        if self.narrow() {
            self.sidebar_open = false;
        }
    }

    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        let q = self.sidebar.filter.text.to_lowercase();
        let m = |s: &str| q.is_empty() || s.to_lowercase().contains(&q);
        let mut out = Vec::new();
        if let Some(s) = self.schema() {
            let tables: Vec<_> = s.tables.iter().filter(|t| t.kind == crate::db::schema::TableKind::Table && m(&t.display())).collect();
            let views: Vec<_> = s.tables.iter().filter(|t| t.kind == crate::db::schema::TableKind::View && m(&t.display())).collect();
            if !tables.is_empty() {
                out.push(SidebarItem::Header("tables"));
                out.extend(tables.iter().map(|t| SidebarItem::Table(t.display())));
            }
            if !views.is_empty() {
                out.push(SidebarItem::Header("views"));
                out.extend(views.iter().map(|t| SidebarItem::Table(t.display())));
            }
        }
        let saved: Vec<_> = self.store.saved_queries(&self.project).unwrap_or_default().into_iter().filter(|q| m(&q.name)).collect();
        if !saved.is_empty() {
            out.push(SidebarItem::Header("saved queries"));
            out.extend(saved.into_iter().map(|q| SidebarItem::Saved(q.name, q.sql)));
        }
        out
    }

    fn inspector_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.inspector_scroll = self.inspector_scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.inspector_scroll += 1,
            KeyCode::PageUp => self.inspector_scroll = self.inspector_scroll.saturating_sub(10),
            KeyCode::PageDown => self.inspector_scroll += 10,
            KeyCode::Char('n') => self.grid_move(1, 0),
            KeyCode::Char('p') => self.grid_move(-1, 0),
            KeyCode::Esc | KeyCode::Enter => {
                self.focus = Focus::Results;
                if !self.inspector_docked() || k.code == KeyCode::Enter {
                    self.inspector_open = false;
                }
            }
            _ => {
                if let Some(a) = self.keymap.lookup(Ctx::Pane, &k)
                    && matches!(a, Action::SpaceMenu | Action::Help | Action::FocusEditor | Action::NextPane | Action::PrevPane | Action::CopyMenu) {
                        self.do_action(a);
                    }
            }
        }
    }

    fn cycle_pane(&mut self, forward: bool) {
        let mut panes = vec![Focus::Results];
        if self.tab().editor_visible() {
            panes.insert(0, Focus::Editor);
        }
        if self.sidebar_open {
            panes.insert(0, Focus::Sidebar);
        }
        if self.inspector_open {
            panes.push(Focus::Inspector);
        }
        let i = panes.iter().position(|p| *p == self.focus).unwrap_or(0);
        let n = panes.len();
        self.focus = panes[if forward { (i + 1) % n } else { (i + n - 1) % n }];
    }

    fn body_rows(&self) -> usize {
        self.layout.grid.height.saturating_sub(3).max(1) as usize
    }

    // ---------------- actions ----------------

    pub fn do_action(&mut self, a: Action) {
        use Action::*;
        let page = self.body_rows();
        match a {
            Palette => self.open_palette(""),
            GoToTable => self.open_palette("#"),
            SwitchConnection => self.open_palette("@"),
            SwitchEnv => {
                if self.conns.project(&self.project).is_some() {
                    let sel = self.conns.project(&self.project).unwrap().envs.iter().position(|e| e.name == self.env).unwrap_or(0);
                    self.overlay = Some(Overlay::Env(sel));
                } else {
                    self.toast("no project · Space c n adds a connection");
                }
            }
            RunStatement => self.run_current(false, Purpose::Query),
            RunAll => self.run_current(true, Purpose::Query),
            Explain => self.run_current(false, Purpose::Explain { analyze: false }),
            ExplainAnalyze => self.run_current(false, Purpose::Explain { analyze: true }),
            Cancel => self.cancel(),
            NewTab => {
                self.add_query_tab(None, "");
            }
            CloseTab => self.close_tab(),
            NextTab | PrevTab => {
                let n = self.tabs.len();
                let i = if a == NextTab { (self.cur + 1) % n } else { (self.cur + n - 1) % n };
                self.switch_tab(i);
            }
            Tab(n) => {
                let i = n as usize - 1;
                if i < self.tabs.len() {
                    self.switch_tab(i);
                }
            }
            ToggleSidebar | ViewSidebar => {
                self.sidebar_open = !self.sidebar_open;
                if self.sidebar_open {
                    self.focus = Focus::Sidebar;
                    self.ensure_schema();
                } else if self.focus == Focus::Sidebar {
                    self.focus = Focus::Results;
                }
            }
            Save => self.save(),
            Quit => self.request_quit(),
            GrowEditor => self.split = (self.split + 5).min(85),
            ShrinkEditor => self.split = self.split.saturating_sub(5).max(15),
            SpaceMenu => self.open_space_menu(""),
            Help => self.open_help(false),
            AllKeys => self.open_help(true),
            FocusEditor => if self.focus == Focus::Editor { self.leave_editor() } else { self.focus_editor() },
            NextPane => self.cycle_pane(true),
            PrevPane => self.cycle_pane(false),
            Up => self.grid_move(-1, 0),
            Down => self.grid_move(1, 0),
            Left => self.grid_move(0, -1),
            Right => self.grid_move(0, 1),
            PageUp => self.grid_move(-(page as isize), 0),
            PageDown => self.grid_move(page as isize, 0),
            Top => self.grid_move(isize::MIN / 2, 0),
            Bottom => self.grid_move(isize::MAX / 2, 0),
            FirstCol => self.grid_move(0, isize::MIN / 2),
            LastCol => self.grid_move(0, isize::MAX / 2),
            Sort => self.sort(false),
            SortAdd => self.sort(true),
            SelectCells => self.select(SelKind::Cells),
            SelectRows => self.select(SelKind::Rows),
            SelectCol => self.select(SelKind::Cols),
            CopyMenu => self.open_space_menu("y"),
            Copy(c) => self.copy(c),
            Find => {
                if self.tab().view().is_some() {
                    self.overlay = Some(Overlay::Prompt(Prompt::new(PromptKind::Find, "/", "")));
                }
            }
            FindNext | FindPrev => {
                let moved = self.tab_mut().view_mut().map(|v| v.grid.find_step(a == FindNext)).unwrap_or(false);
                if !moved {
                    self.toast("no search · / finds in loaded rows");
                }
            }
            FitColumn | Narrow | Widen => {
                let group = self.settings.group_digits;
                if let Some(v) = self.tab_mut().view_mut() {
                    let c = v.grid.col;
                    if c < v.grid.widths.len() {
                        let w = v.grid.widths[c];
                        v.grid.widths[c] = match a {
                            FitColumn => crate::grid::fit_width(&v.rs, c, group, usize::MAX, 200),
                            Narrow => w.saturating_sub(2).max(3),
                            _ => (w + 2).min(200),
                        };
                    }
                }
            }
            Inspect | ViewInspector => {
                self.inspector_open = !self.inspector_open;
                self.inspector_scroll = 0;
                if self.inspector_open && (!self.inspector_docked() || a == ViewInspector) {
                    self.focus = Focus::Inspector;
                } else if !self.inspector_open && self.focus == Focus::Inspector {
                    self.focus = Focus::Results;
                }
            }
            LoadAll => self.load_all(),
            Filter => {
                if self.tab().is_table() {
                    self.overlay = Some(Overlay::Prompt(Prompt::new(PromptKind::Filter, "filter", "")));
                } else {
                    self.toast("filters work in table tabs · / finds in loaded rows");
                }
            }
            ClearFilters => {
                let i = self.cur;
                if let Some(t) = &mut self.tabs[i].table {
                    t.filters.clear();
                    self.refresh_table(i);
                }
            }
            EditCell => self.edit_cell(),
            UndoEdit => {
                let t = self.tab_mut();
                if let Some(key) = t.edit_order.pop()
                    && let Some(v) = t.results.first_mut() {
                        v.grid.edits.remove(&key);
                    }
            }
            NextResult | PrevResult => {
                let t = self.tab_mut();
                let n = t.views().len() + 1;
                t.cur_result = if a == NextResult { (t.cur_result + 1) % n } else { (t.cur_result + n - 1) % n };
            }
            Escape => {
                let mut cleared = false;
                if let Some(v) = self.tab_mut().view_mut()
                    && (v.grid.sel.is_some() || v.grid.find.is_some()) {
                        v.grid.sel = None;
                        v.grid.find = None;
                        cleared = true;
                    }
                if !cleared && self.inspector_open {
                    self.inspector_open = false;
                }
            }
            Begin => self.run_text("begin", "begin"),
            Commit => self.run_text("commit", "commit"),
            Rollback => self.run_text("rollback", "rollback"),
            RowLimit(n) => {
                self.tab_mut().row_limit = Some(n);
                self.toast(if n == 0 { "row limit off for this tab".to_string() } else { format!("row limit {} for this tab", crate::grid::group_digits(&n.to_string())) });
            }
            NewConnection => self.open_conn_form(false),
            EditConnection => self.open_conn_form(true),
            AllowWrites => self.toggle_writes(),
            RefreshSchema => {
                let k = self.key();
                self.schemas.remove(&k);
                self.load_schema();
                self.toast("refreshing schema…");
            }
            Disconnect => {
                let k = self.key();
                for t in self.tabs.iter_mut() {
                    t.slot = new_slot();
                    t.in_tx = false;
                }
                self.meta.remove(&k);
                self.toast(format!("disconnected from {} · {}", self.project, self.env));
            }
            OpenTable => self.open_palette("#"),
            Structure | Data => {
                let i = self.cur;
                if let Some(t) = &mut self.tabs[i].table {
                    t.structure = a == Structure;
                    self.tabs[i].cur_result = 0;
                    if a == Structure {
                        self.load_structure(i);
                    } else if self.tabs[i].results.is_empty() {
                        self.refresh_table(i);
                    }
                    self.focus = Focus::Results;
                } else if let Some(name) = self.table_under_cursor() {
                    self.open_table(&name, None);
                    if a == Structure {
                        self.do_action(Structure);
                    }
                } else {
                    self.toast("open a table first · ^P");
                }
            }
            ShowSql => {
                let i = self.cur;
                if let Some(t) = &mut self.tabs[i].table {
                    t.show_sql = !t.show_sql;
                    self.focus = if t.show_sql { Focus::Editor } else { Focus::Results };
                } else {
                    self.toast("show SQL applies to table tabs");
                }
            }
            CountRows => match self.current_table() {
                Some(t) => self.count_rows(&t),
                None => self.toast("open a table first · ^P"),
            },
            Truncate => self.truncate(),
            ViewEditor => {
                self.hide_editor = !self.hide_editor;
                if self.hide_editor && self.focus == Focus::Editor {
                    self.focus = Focus::Results;
                }
                if self.hide_editor {
                    self.hide_results = false;
                }
            }
            ViewResults => {
                self.hide_results = !self.hide_results;
                if self.hide_results {
                    self.hide_editor = false;
                    self.focus = Focus::Editor;
                }
            }
            Zen => self.zen = !self.zen,
            Density => {
                self.settings.density = if self.settings.density == crate::config::Density::Compact { crate::config::Density::Comfortable } else { crate::config::Density::Compact };
                let _ = self.settings.save();
            }
            KeyHints => {
                self.settings.key_hints = !self.settings.key_hints;
                let _ = self.settings.save();
            }
            Export(fmt) => {
                let ext = match fmt {
                    'j' => "json",
                    's' => "sql",
                    _ => "csv",
                };
                let base = self.export_source().map(|(_, t)| t.rsplit('.').next().unwrap_or("export").to_string()).unwrap_or_else(|| "export".into());
                self.overlay = Some(Overlay::Prompt(Prompt::new(PromptKind::ExportPath { fmt }, &format!("export {ext} to"), &format!("{base}.{ext}"))));
            }
            History => self.open_palette("!"),
            SavedQueries => self.open_palette("/"),
            FormatSql => {
                if self.tab().editor_visible() {
                    let text = self.tab().editor.text();
                    let (r, _) = self.tab().editor.cursor();
                    let formatted = crate::sql::format(&text);
                    let ed = &mut self.tab_mut().editor;
                    ed.set_text(formatted.trim_end());
                    ed.set_cursor(r, 0);
                    self.toast("formatted");
                }
            }
            OpenKeymap => self.edit_config_file(crate::keys::Keymap::path(), crate::keys::Keymap::template()),
            OpenSettings => {
                let s = toml::to_string_pretty(&self.settings).unwrap_or_default();
                self.edit_config_file(crate::config::Settings::path(), s)
            }
            OpenTheme => self.edit_config_file(
                crate::config::config_dir().join("theme.toml"),
                "# Colours: ANSI names (red, lightblue, darkgray…), 0-255, or #rrggbb.\n# accent = \"cyan\"\n# local = \"gray\"\n# staging = \"yellow\"\n# prod = \"red\"\n# prod_fg = \"white\"\n# error = \"red\"\n# pending = \"yellow\"\n# row = \"darkgray\"\n# selection = \"blue\"\n# keyword = \"blue\"\n# string = \"green\"\n# number = \"magenta\"\n# comment = \"darkgray\"\n# function = \"cyan\"\n".into(),
            ),
            ToggleVim => {
                self.settings.vim = !self.settings.vim;
                let v = self.settings.vim;
                for t in self.tabs.iter_mut() {
                    t.editor.set_vim(v);
                }
                let _ = self.settings.save();
                self.toast(if v { "vim mode on" } else { "vim mode off" });
            }
            ExternalEditor => {
                if self.tab().editor_visible() {
                    self.external = Some(External::EditBuffer);
                }
            }
            FollowFk => self.follow_fk(),
        }
    }

    fn edit_config_file(&mut self, path: std::path::PathBuf, template: String) {
        if !path.exists() {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            let _ = std::fs::write(&path, template);
        }
        self.external = Some(External::EditFile(path));
    }

    fn grid_move(&mut self, dr: isize, dc: isize) {
        let rows = self.body_rows();
        let width = self.layout.grid.width;
        let Some(v) = self.tab_mut().view_mut() else { return };
        match &v.body {
            ResultBody::Grid => {
                v.grid.move_by(&v.rs, dr, dc);
                v.grid.ensure_visible(rows, width);
            }
            _ => {
                // text views scroll
                v.grid.top = (v.grid.top as isize + dr).max(0) as usize;
            }
        }
        self.inspector_scroll = 0;
    }

    /// Spreadsheet-style selection: Shift+arrows extend a cell range (or a row range when
    /// rows are selected), Shift+Space selects the row, ^A selects every loaded row.
    fn grid_select_key(&mut self, k: KeyEvent) -> bool {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let Some(v) = self.tab_mut().view_mut() else { return false };
        if !matches!(v.body, ResultBody::Grid) || v.rs.cols.is_empty() {
            return false;
        }
        let here = (v.grid.row, v.grid.col);
        match k.code {
            KeyCode::Char(' ') if shift => {
                v.grid.sel = match v.grid.sel {
                    Some(s) if s.kind == SelKind::Rows => None,
                    _ => Some(Selection { anchor: here, kind: SelKind::Rows }),
                };
                true
            }
            KeyCode::Char('a') if ctrl => {
                let last = v.rs.rows.saturating_sub(1);
                v.grid.sel = Some(Selection { anchor: (0, here.1), kind: SelKind::Rows });
                v.grid.row = last;
                let rows = self.body_rows();
                let width = self.layout.grid.width;
                if let Some(v) = self.tab_mut().view_mut() {
                    v.grid.ensure_visible(rows, width);
                }
                true
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right if shift => {
                if v.grid.sel.is_none() {
                    v.grid.sel = Some(Selection { anchor: here, kind: SelKind::Cells });
                }
                let (dr, dc) = match k.code {
                    KeyCode::Up => (-1, 0),
                    KeyCode::Down => (1, 0),
                    KeyCode::Left => (0, -1),
                    _ => (0, 1),
                };
                self.grid_move(dr, dc);
                true
            }
            _ => false,
        }
    }

    fn select(&mut self, kind: SelKind) {
        if let Some(v) = self.tab_mut().view_mut() {
            v.grid.sel = match v.grid.sel {
                Some(s) if s.kind == kind => None,
                _ => Some(Selection { anchor: (v.grid.row, v.grid.col), kind }),
            };
        }
    }

    fn sort(&mut self, add: bool) {
        let i = self.cur;
        let Some(v) = self.tabs[i].view() else { return };
        if !matches!(v.body, ResultBody::Grid) || v.rs.cols.is_empty() {
            return;
        }
        let col = v.grid.col;
        let col_name = v.rs.cols[col].name.clone();
        // table tabs and capped results sort on the server
        if let Some(t) = &mut self.tabs[i].table {
            if t.structure {
                let v = self.tabs[i].view_mut().unwrap();
                v.grid.sort_cycle_local(col, add);
                return;
            }
            cycle_named(&mut t.sort, &col_name, add);
            self.refresh_table(i);
            self.toast("sorted on server");
            return;
        }
        let v = self.tabs[i].view_mut().unwrap();
        if v.rs.limited || v.stmt.server_sort {
            let mut sort = v.grid.sort.clone();
            cycle_idx(&mut sort, col, add);
            let mut stmt = v.stmt.clone();
            let order: Vec<(usize, bool)> = sort.iter().map(|(c, d)| (c + 1, *d)).collect();
            stmt.exec = crate::sql::wrap_sorted(&stmt.orig, &order, stmt.cap);
            stmt.server_sort = !order.is_empty();
            if order.is_empty() {
                stmt.exec = match stmt.cap.and_then(|c| crate::sql::apply_cap(&stmt.orig, c)) {
                    Some(s) => s,
                    None => crate::sql::trimmed(&stmt.orig).to_string(),
                };
            }
            let pos = (v.grid.row, v.grid.col);
            self.start_job(i, vec![stmt], false);
            self.pending_sort = Some((sort, pos));
            self.toast("sorted on server");
        } else {
            v.grid.sort_cycle(&v.rs, col, add);
        }
    }

    fn load_all(&mut self) {
        let i = self.cur;
        let Some(v) = self.tabs[i].view() else { return };
        if !v.rs.limited {
            self.toast("all rows are loaded");
            return;
        }
        if let Some(t) = self.tabs[i].table.clone() {
            let exec = self.browse_sql(&t, None);
            let stmt = JobStmt { exec: exec.clone(), orig: exec, offset: None, label: v.label.clone(), cap: None, purpose: Purpose::Browse, server_sort: true };
            self.start_job(i, vec![stmt], false);
            return;
        }
        let mut stmt = v.stmt.clone();
        let order: Vec<(usize, bool)> = v.grid.sort.iter().map(|(c, d)| (c + 1, *d)).collect();
        stmt.cap = None;
        stmt.exec = if stmt.server_sort { crate::sql::wrap_sorted(&stmt.orig, &order, None) } else { crate::sql::trimmed(&stmt.orig).to_string() };
        self.start_job(i, vec![stmt], false);
    }

    fn run_text(&mut self, sql: &str, label: &str) {
        let i = self.cur;
        let stmt = JobStmt { exec: sql.into(), orig: sql.into(), offset: None, label: label.into(), cap: None, purpose: Purpose::Silent, server_sort: false };
        self.start_job(i, vec![stmt], false);
    }

    fn toggle_writes(&mut self) {
        let Some(e) = self.env_config() else { return };
        if !e.read_only() {
            self.toast(format!("{} already allows writes", self.env));
            return;
        }
        let k = self.key();
        if self.writes_unlocked() {
            self.unlocked.remove(&k);
            self.toast(format!("writes locked on {}", self.env));
        } else {
            self.unlocked.insert(k, Instant::now());
            self.toast(format!("writes allowed on {} · {} until quit or {} idle minutes", self.project, self.env, self.settings.prod_unlock_idle_minutes));
        }
    }

    fn cancel(&mut self) {
        if let Some(e) = self.export.take() {
            let _ = e.cancel.send(());
            self.toast("export cancelled");
            return;
        }
        let t = self.tab_mut();
        match &mut t.run {
            Some(r) => {
                r.cancelling = true;
                if let Some(c) = t.canceller.lock().unwrap().clone() {
                    tokio::spawn(async move {
                        let _ = c.cancel().await;
                    });
                }
                self.toast("cancelling… rows already received are kept");
            }
            None => {
                let q = self.keymap.hint(Action::Quit);
                self.toast(format!("nothing running · {q} quits"));
            }
        }
    }

    fn switch_tab(&mut self, i: usize) {
        self.cur = i;
        self.complete = None;
        if !self.tab().editor_visible() && self.focus == Focus::Editor {
            self.focus = Focus::Results;
        }
        if self.tab().is_table() && self.tab().results.is_empty() && self.tab().run.is_none() {
            self.refresh_table(i);
        }
    }

    fn close_tab(&mut self) {
        let t = self.tab();
        let mut why = Vec::new();
        if t.in_tx {
            why.push("has an open transaction (it will roll back)");
        }
        if t.pending_edits() > 0 {
            why.push("has unsaved edits");
        }
        if !why.is_empty() {
            let name = t.name.clone();
            self.confirm("Close tab", format!("{name} {}.", why.join(" and ")), String::new(), None, Box::new(|app: &mut App| app.do_close_tab()));
            return;
        }
        self.do_close_tab();
    }

    fn do_close_tab(&mut self) {
        let t = self.tabs.remove(self.cur);
        if t.run.is_some()
            && let Some(c) = t.canceller.lock().unwrap().clone() {
                tokio::spawn(async move {
                    let _ = c.cancel().await;
                });
            }
        if self.tabs.is_empty() {
            self.add_query_tab(None, "");
        }
        self.cur = self.cur.min(self.tabs.len() - 1);
        let i = self.cur;
        self.switch_tab(i);
    }

    fn save(&mut self) {
        if self.tab().pending_edits() > 0 {
            match self.edit_statements() {
                Ok(sql) => self.overlay = Some(Overlay::Review(Review { sql, scroll: 0 })),
                Err(e) => self.toast_err(e),
            }
            return;
        }
        if self.tab().is_table() {
            self.toast("no pending edits");
            return;
        }
        if self.project.is_empty() || self.project == crate::config::SCRATCH {
            self.toast("save the connection to a project first · Space c d");
            return;
        }
        let initial = self.tab().saved.clone().unwrap_or_else(|| if self.tab().name.starts_with("untitled-") { String::new() } else { self.tab().name.clone() });
        self.overlay = Some(Overlay::Prompt(Prompt::new(PromptKind::SaveQuery, "save query as", &initial)));
    }

    pub fn request_quit(&mut self) {
        let open: Vec<String> = self.tabs.iter().filter(|t| t.in_tx).map(|t| t.name.clone()).collect();
        if !open.is_empty() {
            self.confirm(
                "Quit",
                format!("Open transaction in {}. Quitting rolls it back.", open.join(", ")),
                String::new(),
                None,
                Box::new(|app: &mut App| {
                    for t in app.tabs.iter_mut() {
                        t.in_tx = false;
                    }
                    app.request_quit()
                }),
            );
            return;
        }
        let edits = self.tabs.iter().filter(|t| t.pending_edits() > 0).count();
        if edits > 0 && !self.quit_confirmed_edits {
            self.confirm(
                "Quit",
                format!("{edits} tab{} with unsaved edits. Quit anyway?", if edits == 1 { "" } else { "s" }),
                String::new(),
                None,
                Box::new(|app: &mut App| {
                    app.quit_confirmed_edits = true;
                    app.request_quit()
                }),
            );
            return;
        }
        if self.scratch_unsaved {
            self.scratch_unsaved = false;
            self.confirm(
                "Save connection?",
                "This scratch connection isn't saved. enter save it to a project · esc quit without saving".into(),
                String::new(),
                None,
                Box::new(|app: &mut App| {
                    app.quit_after_save = true;
                    app.open_conn_form(true);
                }),
            );
            if let Some(Overlay::Confirm(c)) = &mut self.overlay {
                c.on_cancel = Some(Box::new(|app: &mut App| app.quit_now()));
            }
            return;
        }
        self.quit_now();
    }

    pub fn quit_now(&mut self) {
        self.save_workspace();
        self.quit = true;
    }

    fn current_table(&self) -> Option<String> {
        self.tab().table.as_ref().map(|t| t.name.clone()).or_else(|| self.table_under_cursor())
    }

    fn table_under_cursor(&self) -> Option<String> {
        if self.focus == Focus::Sidebar
            && let Some(SidebarItem::Table(n)) = self.sidebar_items().get(self.sidebar.sel) {
                return Some(n.clone());
            }
        None
    }

    fn truncate(&mut self) {
        let Some(t) = self.current_table() else {
            self.toast("open a table first · ^P");
            return;
        };
        if self.read_only() {
            self.toast_err(format!("{} is read-only · Space c w allows writes", self.env));
            return;
        }
        let d = self.driver();
        let q = match self.schema().and_then(|s| s.table(&t).cloned()) {
            Some(ti) => crate::db::quote_qualified(d, &ti.schema, &ti.name),
            None => t.clone(),
        };
        let sql = format!("truncate table {q}");
        let require = if self.level() == Level::Prod { Some(self.env.clone()) } else { None };
        let s2 = sql.clone();
        self.confirm(
            "Truncate",
            "this removes every row and cannot be undone".into(),
            format!("{sql};"),
            require,
            Box::new(move |app: &mut App| {
                let i = app.cur;
                let stmt = JobStmt { exec: s2.clone(), orig: s2, offset: None, label: "truncate".into(), cap: None, purpose: Purpose::Silent, server_sort: false };
                app.start_job(i, vec![stmt], false);
            }),
        );
    }

    fn edit_cell(&mut self) {
        let Some(t) = self.tab().table.clone() else {
            self.toast("editing works in table tabs · ^P opens one");
            return;
        };
        if t.structure {
            return;
        }
        let pk = self.schema().map(|s| s.primary_key(&t.name)).unwrap_or_default();
        if pk.is_empty() {
            self.toast(format!("{} has no primary key, so its rows are read-only here", t.name));
            return;
        }
        let Some(v) = self.tab().results.first() else { return };
        if v.rs.rows == 0 {
            return;
        }
        let row = v.grid.data_row(v.grid.row);
        let col = v.grid.col;
        let current = v.grid.cell_value(&v.rs, v.grid.row, col).map(String::from);
        let name = v.rs.cols[col].name.clone();
        let mut p = Prompt::new(PromptKind::EditCell { row, col }, &format!("edit {name}"), current.as_deref().unwrap_or(""));
        p.null = current.is_none();
        self.overlay = Some(Overlay::Prompt(p));
    }

    fn follow_fk(&mut self) {
        let d = self.driver();
        let Some(v) = self.tab().view() else { return };
        let col = v.grid.col;
        let Some((_, Some((table, rcol)))) = v.rs.keys.get(col).cloned() else {
            self.toast("not a foreign key column");
            return;
        };
        let val = v.grid.cell_value(&v.rs, v.grid.row, col).map(String::from);
        let Some(val) = val else {
            self.toast("null · nothing to follow");
            return;
        };
        let lit = crate::db::literal(d, v.rs.cols[col].kind, Some(&val));
        self.open_table(&table, Some(format!("{} = {lit}", crate::db::quote_ident(d, &rcol))));
    }

    // ---------------- copy ----------------

    pub fn copy(&mut self, c: char) {
        let Some(fmt) = Format::from_key(c) else { return };
        let d = self.driver();
        let tab = self.tab();
        let table_name = tab.table.as_ref().map(|t| t.name.clone());
        let Some(v) = tab.view() else {
            // messages sub-tab: copy the messages text
            let text: Vec<String> = tab.messages.iter().map(|m| m.text.clone()).collect();
            if text.is_empty() {
                self.toast("nothing to copy");
            } else {
                let s = text.join("\n");
                self.set_clipboard(&s, format!("Copied {} messages", text.len()));
            }
            return;
        };
        match &v.body {
            ResultBody::Text(lines) => {
                let s = lines.join("\n");
                self.set_clipboard(&s, format!("Copied {} lines", lines.len()));
                return;
            }
            ResultBody::Plan(p) => {
                let pre = crate::explain::prefixes(&p.nodes, self.settings.ascii);
                let s = p.nodes.iter().zip(pre).map(|(n, pre)| format!("{pre}{}", n.label)).collect::<Vec<_>>().join("\n");
                self.set_clipboard(&s, format!("Copied plan ({} nodes)", p.nodes.len()));
                return;
            }
            ResultBody::Grid => {}
        }
        if v.rs.cols.is_empty() {
            self.toast("nothing to copy");
            return;
        }
        let (rows, cols): (Vec<usize>, Vec<usize>) = match v.grid.selected(&v.rs) {
            Some((r, c)) => (r.collect(), c.collect()),
            None if fmt == Format::Values => (vec![v.grid.row], vec![v.grid.col]),
            None if fmt == Format::InList => ((0..v.rs.rows).collect(), vec![v.grid.col]),
            None if fmt == Format::Names => (vec![], (0..v.rs.cols.len()).collect()),
            None => ((0..v.rs.rows).collect(), (0..v.rs.cols.len()).collect()),
        };
        let table = crate::copy::Table {
            names: cols.iter().map(|c| v.rs.cols[*c].name.as_str()).collect(),
            kinds: cols.iter().map(|c| v.rs.cols[*c].kind).collect(),
            rows: rows.iter().map(|r| cols.iter().map(|c| v.grid.cell_value(&v.rs, *r, *c)).collect()).collect(),
        };
        let tn = table_name.unwrap_or_else(|| crate::sql::table_refs(&v.stmt.orig).first().map(|(t, _)| t.clone()).unwrap_or_else(|| v.label.clone()));
        let text = crate::copy::render(fmt, &table, d, &tn);
        let what = match fmt {
            Format::Names => format!("{} column names", cols.len()),
            Format::Values if rows.len() == 1 && cols.len() == 1 => "1 value".to_string(),
            _ => format!("{} row{} × {} col{}", rows.len(), if rows.len() == 1 { "" } else { "s" }, cols.len(), if cols.len() == 1 { "" } else { "s" }),
        };
        let label = match fmt {
            Format::Values => String::new(),
            other => format!(" as {}", other.label()),
        };
        self.set_clipboard(&text, format!("Copied {what}{label}"));
    }

    fn set_clipboard(&mut self, text: &str, msg: String) {
        match self.clipboard.set(text) {
            Ok(()) => self.toast(msg),
            Err(e) => self.toast_err(format!("clipboard: {e}")),
        }
    }

    // ---------------- autocomplete ----------------

    pub fn update_completion(&mut self) {
        let Some(schema) = self.schema() else {
            self.complete = None;
            self.ensure_schema();
            return;
        };
        let tab = self.tab();
        let (word, qual) = tab.editor.word_before_cursor();
        let text = tab.editor.text();
        let stmts = tab.statements(self.settings.blank_line_splits);
        let cur_off = tab.editor.cursor_offset();
        let stmt_text = crate::sql::stmt_at(&stmts.2, cur_off).map(|i| text[stmts.2[i].full.clone()].to_string()).unwrap_or_default();
        drop(stmts);
        let refs = crate::sql::table_refs(&stmt_text);
        let lw = word.to_lowercase();
        let starts = |s: &str| s.to_lowercase().starts_with(&lw) && s != word;
        let mut items: Vec<(String, &'static str)> = Vec::new();
        if let Some(q) = qual {
            // alias.col, table.col or schema.table
            let target = refs.iter().find(|(t, a)| a.as_deref() == Some(q.as_str()) || t == &q || t.rsplit('.').next() == Some(q.as_str())).map(|(t, _)| t.clone()).unwrap_or(q.clone());
            for c in schema.columns_of(&target) {
                if lw.is_empty() || starts(&c.name) {
                    items.push((c.name.clone(), "column"));
                }
            }
            for t in schema.tables.iter().filter(|t| t.schema == q) {
                if lw.is_empty() || starts(&t.name) {
                    items.push((t.name.clone(), "table"));
                }
            }
        } else {
            if word.is_empty() {
                self.complete = None;
                return;
            }
            // previous word decides: after FROM/JOIN only tables
            let before: String = text[..cur_off].chars().rev().skip(word.chars().count()).collect::<String>().chars().rev().collect();
            let prev = before.split_whitespace().last().unwrap_or("").to_lowercase();
            let tables_only = matches!(prev.as_str(), "from" | "join" | "into" | "update" | "table");
            let table_name = |t: &crate::db::schema::TableInfo| if t.schema.is_empty() || t.schema == "public" { t.name.clone() } else { t.display() };
            if !tables_only {
                for (t, _) in &refs {
                    for c in schema.columns_of(t) {
                        if starts(&c.name) && !items.iter().any(|(n, _)| *n == c.name) {
                            items.push((c.name.clone(), "column"));
                        }
                    }
                }
            }
            for t in &schema.tables {
                let n = table_name(t);
                if starts(&n) || starts(&t.name) {
                    items.push((n, if t.kind == crate::db::schema::TableKind::View { "view" } else { "table" }));
                }
            }
            if !tables_only {
                for k in crate::sql::KEYWORDS {
                    if starts(k) && k.len() > 2 {
                        items.push((k.to_string(), "keyword"));
                    }
                }
                for f in crate::sql::FUNCTIONS {
                    if starts(f) {
                        items.push((format!("{f}("), "function"));
                    }
                }
            }
        }
        items.truncate(50);
        self.complete = if items.is_empty() { None } else { Some(Completion { items, sel: 0 }) };
    }

    fn accept_completion(&mut self) {
        let Some(c) = self.complete.take() else { return };
        if let Some((text, _)) = c.items.get(c.sel) {
            let d = self.driver();
            let t = if text.ends_with('(') || text.contains('.') { text.clone() } else { crate::db::quote_ident(d, text) };
            let t = if crate::sql::is_keyword(text) { text.clone() } else { t };
            self.tab_mut().editor.complete_word(&t);
        }
    }

    // ---------------- mouse ----------------

    pub fn on_mouse(&mut self, m: MouseEvent) {
        if !self.settings.mouse {
            return;
        }
        self.touch();
        let (x, y) = (m.column, m.row);
        let inside = |r: Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.overlay.is_some() {
                    // clicks outside an overlay close it (except forms and confirms)
                    if matches!(self.overlay, Some(Overlay::Palette(_)) | Some(Overlay::Env(_)) | Some(Overlay::Help(_))) {
                        self.overlay = None;
                    }
                    return;
                }
                self.space = None;
                let double = self.last_click.is_some_and(|(t, cx, cy)| t.elapsed() < Duration::from_millis(400) && cx == x && cy == y);
                self.last_click = Some((Instant::now(), x, y));
                let l = self.layout.clone();
                if inside(l.top) {
                    if x >= l.env_span.0 && x < l.env_span.1 {
                        self.do_action(Action::SwitchEnv);
                    }
                } else if inside(l.tabbar) {
                    if let Some((i, _, _)) = l.tab_spans.iter().find(|(_, a, b)| x >= *a && x < *b) {
                        self.switch_tab(*i);
                    }
                } else if inside(l.sidebar) {
                    self.focus = Focus::Sidebar;
                    if y >= l.sidebar_items_y {
                        let i = (y - l.sidebar_items_y) as usize + self.sidebar.scroll;
                        let items = self.sidebar_items();
                        if i < items.len() {
                            self.sidebar.sel = i;
                            if double || matches!(items[i], SidebarItem::Table(_) | SidebarItem::Saved(..)) && double {
                                self.sidebar_open_item(items[i].clone(), false);
                            }
                        }
                    }
                } else if inside(l.inspector) {
                    self.focus = Focus::Inspector;
                } else if inside(l.editor) {
                    if self.tab().editor_visible() {
                        self.focus = Focus::Editor;
                        let t = self.tab_mut();
                        let row = (y - l.editor.y) as usize + t.editor.scroll;
                        let col = (x.saturating_sub(l.editor.x + l.gutter)) as usize + t.editor.hscroll;
                        t.editor.anchor = None;
                        t.editor.set_cursor(row, col);
                        self.complete = None;
                    }
                } else if inside(l.results_rule) {
                    if let Some((i, _, _)) = l.result_spans.iter().find(|(_, a, b)| x >= *a && x < *b) {
                        self.tab_mut().cur_result = *i;
                    } else if !self.zen {
                        self.drag = Some(Drag::Split);
                    }
                    self.focus = Focus::Results;
                } else if inside(l.grid) {
                    self.focus = Focus::Results;
                    self.grid_click(x, y, m.modifiers.contains(KeyModifiers::SHIFT), double);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => match self.drag {
                Some(Drag::Split) => {
                    let total = self.layout.editor.height + self.layout.grid.height + 1;
                    let top = self.layout.editor.y;
                    if total > 4 && y > top {
                        let pct = ((y - top) as u32 * 100 / total as u32) as u16;
                        self.split = pct.clamp(10, 90);
                    }
                }
                Some(Drag::Grid) => {
                    if let Some((r, c)) = self.grid_cell_at(x, y)
                        && let Some(v) = self.tab_mut().view_mut() {
                            if v.grid.sel.is_none() {
                                v.grid.sel = Some(Selection { anchor: (v.grid.row, v.grid.col), kind: SelKind::Cells });
                            }
                            v.grid.row = r;
                            v.grid.col = c;
                        }
                }
                None => {}
            },
            MouseEventKind::Up(_) => self.drag = None,
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d: isize = if m.kind == MouseEventKind::ScrollDown { 3 } else { -3 };
                let l = self.layout.clone();
                if inside(l.editor) && self.tab().editor_visible() {
                    let t = self.tab_mut();
                    let max = t.editor.lines.len().saturating_sub(1);
                    t.editor.scroll = (t.editor.scroll as isize + d).clamp(0, max as isize) as usize;
                } else if inside(l.sidebar) {
                    let n = self.sidebar_items().len();
                    self.sidebar.sel = (self.sidebar.sel as isize + d).clamp(0, n.saturating_sub(1) as isize) as usize;
                } else if inside(l.inspector) {
                    self.inspector_scroll = (self.inspector_scroll as isize + d).max(0) as usize;
                } else if inside(l.grid) {
                    let rows = self.body_rows();
                    if let Some(v) = self.tab_mut().view_mut() {
                        let max = v.rs.rows.saturating_sub(rows);
                        v.grid.top = (v.grid.top as isize + d).clamp(0, max as isize) as usize;
                        if matches!(v.body, ResultBody::Grid) {
                            v.grid.row = v.grid.row.clamp(v.grid.top, (v.grid.top + rows).saturating_sub(1).max(v.grid.top));
                            v.grid.clamp(&v.rs);
                        }
                    }
                }
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
                let d = if m.kind == MouseEventKind::ScrollRight { 1 } else { -1 };
                if inside(self.layout.grid) {
                    self.grid_move(0, d);
                }
            }
            _ => {}
        }
    }

    fn grid_cell_at(&self, x: u16, y: u16) -> Option<(usize, usize)> {
        let v = self.tab().view()?;
        let l = &v.grid.layout;
        if y < l.body_y || y >= l.body_y + l.body_rows {
            return None;
        }
        let row = v.grid.top + (y - l.body_y) as usize;
        if row >= v.rs.rows {
            return None;
        }
        let col = l.cols.iter().find(|(_, cx, w)| x >= *cx && x < cx + w + 2).map(|(c, _, _)| *c)?;
        Some((row, col))
    }

    fn grid_click(&mut self, x: u16, y: u16, shift: bool, double: bool) {
        let Some(v) = self.tab().view() else { return };
        let l = v.grid.layout.clone();
        if y == l.header_y || y == l.header_y + 1 {
            if let Some((c, _, _)) = l.cols.iter().find(|(_, cx, w)| x >= *cx && x < cx + w + 2) {
                let c = *c;
                if let Some(v) = self.tab_mut().view_mut() {
                    v.grid.col = c;
                }
                self.sort(shift);
            }
            return;
        }
        if let Some((r, c)) = self.grid_cell_at(x, y) {
            let v = self.tab_mut().view_mut().unwrap();
            if shift {
                if v.grid.sel.is_none() {
                    v.grid.sel = Some(Selection { anchor: (v.grid.row, v.grid.col), kind: SelKind::Cells });
                }
            } else {
                v.grid.sel = None;
            }
            v.grid.row = r;
            v.grid.col = c;
            self.drag = Some(Drag::Grid);
            if double {
                self.do_action(Action::Inspect);
            }
        }
    }
}

trait LocalSort {
    fn sort_cycle_local(&mut self, col: usize, add: bool);
}

impl LocalSort for crate::grid::GridState {
    fn sort_cycle_local(&mut self, col: usize, add: bool) {
        let mut s = self.sort.clone();
        cycle_idx(&mut s, col, add);
        self.sort = s;
    }
}

fn cycle_idx(sort: &mut Vec<(usize, bool)>, col: usize, add: bool) {
    match sort.iter().position(|(c, _)| *c == col) {
        Some(i) if !sort[i].1 => sort[i].1 = true,
        Some(i) => {
            sort.remove(i);
        }
        None if add => sort.push((col, false)),
        None => *sort = vec![(col, false)],
    }
    if !add {
        sort.retain(|(c, _)| *c == col);
    }
}

fn cycle_named(sort: &mut Vec<(String, bool)>, col: &str, add: bool) {
    match sort.iter().position(|(c, _)| c == col) {
        Some(i) if !sort[i].1 => sort[i].1 = true,
        Some(i) => {
            sort.remove(i);
        }
        None if add => sort.push((col.to_string(), false)),
        None => *sort = vec![(col.to_string(), false)],
    }
    if !add {
        sort.retain(|(c, _)| c == col);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SidebarItem {
    Header(&'static str),
    Table(String),
    Saved(String, String),
}

fn describe(a: Action) -> &'static str {
    use Action::*;
    match a {
        Palette => "command palette",
        GoToTable => "go to table",
        SwitchConnection => "switch connection",
        SwitchEnv => "switch environment",
        RunStatement => "run statement at cursor (or selection)",
        RunAll => "run all statements",
        Cancel => "cancel running query",
        NewTab => "new tab",
        CloseTab => "close tab",
        ToggleSidebar => "toggle sidebar",
        Save => "save query to project · review edits",
        Quit => "quit",
        GrowEditor => "move split down",
        ShrinkEditor => "move split up",
        SpaceMenu => "menu",
        Help => "help for this pane",
        FocusEditor => "editor ⇄ results",
        NextPane => "next pane",
        PrevPane => "previous pane",
        Up => "up",
        Down => "down",
        Left => "left",
        Right => "right",
        PageUp => "page up",
        PageDown => "page down",
        Top => "top (gd follows a foreign key)",
        Bottom => "bottom",
        FirstCol => "first column",
        LastCol => "last column",
        Sort => "sort: ascending → descending → off",
        SortAdd => "add secondary sort",
        SelectCells => "select cell range",
        SelectRows => "select rows",
        SelectCol => "select column",
        CopyMenu => "copy…",
        Find => "find in loaded rows",
        FindNext => "next match",
        FindPrev => "previous match",
        FitColumn => "fit column width",
        Narrow => "narrow column",
        Widen => "widen column",
        Inspect => "inspect row",
        LoadAll => "load all rows",
        Filter => "add filter chip (table tabs)",
        ClearFilters => "clear filters",
        EditCell => "edit cell (table tabs with a primary key)",
        UndoEdit => "undo last staged edit",
        NextResult => "next result",
        PrevResult => "previous result",
        Escape => "clear selection / close inspector",
        _ => "",
    }
}

/// (key, description, section) rows for the ? help screen.
pub fn help_entries(app: &App, focus: Option<Focus>) -> Vec<(String, String, String)> {
    let km = &app.keymap;
    let mut out = Vec::new();
    let push_ctx = |ctx: Ctx, section: &str, out: &mut Vec<(String, String, String)>| {
        for c in COMMANDS {
            if c.keys.iter().any(|(x, _)| *x == ctx) {
                let d = if c.title.is_empty() { describe(c.action) } else { c.title };
                let d = if d.is_empty() { describe(c.action) } else { d };
                let keys: Vec<String> = c
                    .keys
                    .iter()
                    .filter(|(x, _)| *x == ctx)
                    .filter_map(|(_, k)| crate::keys::parse_key(k))
                    .map(|k| crate::keys::display_key(&k, km.ascii))
                    .collect();
                // reflect keymap overrides for the primary key
                let primary = km.hint(c.action);
                let shown = if !primary.is_empty() && !keys.contains(&primary) { primary } else { keys.join(" ") };
                out.push((shown, d.to_string(), section.to_string()));
            }
        }
    };
    let editor_keys: &[(&str, &str)] = &[
        ("arrows home end", "move"),
        ("^A", "line start"),
        ("M-b M-f", "word left / right"),
        ("shift+arrows", "select"),
        ("^Z ^Y", "undo / redo"),
        ("M-c M-x M-v", "copy / cut / paste selection"),
        ("tab", "accept completion"),
        ("esc", "dismiss completion · go to results"),
        ("^X ^E", "edit in $EDITOR"),
        ("⏎", "newline"),
    ];
    let all = focus.is_none();
    push_ctx(Ctx::Global, "global", &mut out);
    for n in 1..=9 {
        let _ = n;
    }
    out.push(("M-1…M-9".into(), "jump to tab".into(), "global".into()));
    if all || focus == Some(Focus::Editor) {
        for (k, d) in editor_keys {
            out.push((k.to_string(), d.to_string(), "editor".into()));
        }
    }
    if all || matches!(focus, Some(Focus::Results) | Some(Focus::Inspector)) {
        push_ctx(Ctx::Pane, "results", &mut out);
        out.push(("[ ]".into(), "previous / next result".into(), "results".into()));
        out.push(("yy yc yC yj ym yi yn yw".into(), "copy values · csv · csv no headers · json · markdown · insert · names · in list".into(), "results".into()));
        out.push(("gd".into(), "open referenced row (foreign key)".into(), "results".into()));
    }
    if all || focus == Some(Focus::Sidebar) {
        for (k, d) in [("⏎", "open"), ("/", "filter"), ("s", "structure"), ("j k", "move"), ("esc", "back")] {
            out.push((k.into(), d.into(), "sidebar".into()));
        }
    }
    if all || focus == Some(Focus::Inspector) {
        for (k, d) in [("j k", "scroll"), ("n p", "next / previous row"), ("esc", "back to results"), ("⏎", "close")] {
            out.push((k.into(), d.into(), "inspector".into()));
        }
    }
    let mut menu_rows = Vec::new();
    fn walk(path: &str, keys: String, rows: &mut Vec<(String, String, String)>) {
        let (title, items) = menu(path);
        for it in items {
            let k = format!("{keys} {}", it.key);
            match it.submenu {
                Some(s) => walk(s, k, rows),
                None => rows.push((k.trim().to_string(), format!("{title}: {}", it.label), "space menu".into())),
            }
        }
    }
    walk("", "␣".into(), &mut menu_rows);
    out.extend(menu_rows);
    let _ = Level::Local;
    out
}

impl App {
    /// Bracketed paste: goes to whatever input has focus.
    pub fn on_paste(&mut self, s: String) {
        self.touch();
        match &mut self.overlay {
            Some(Overlay::Palette(p)) => {
                p.input.insert(&s);
                p.filter();
            }
            Some(Overlay::Prompt(p)) => p.input.insert(&s),
            Some(Overlay::Confirm(c)) => c.typed.insert(&s),
            Some(Overlay::Params(f)) => {
                let i = f.idx;
                f.values[i].insert(&s);
            }
            Some(Overlay::Form(f)) => {
                let i = f.focus;
                if i < crate::app::overlay::FIELD_COUNT {
                    f.fields[i].insert(s.trim());
                    if i == crate::app::overlay::URL {
                        f.split_url();
                    }
                }
            }
            Some(Overlay::Help(h)) => h.query.insert(&s),
            Some(_) => {}
            None => {
                if self.focus == Focus::Sidebar && self.sidebar.filtering {
                    self.sidebar.filter.insert(&s);
                } else if self.tab().editor_visible() {
                    self.focus = Focus::Editor;
                    self.hide_editor = false;
                    let s = s.replace("\r\n", "\n").replace('\r', "\n");
                    self.tab_mut().editor.insert_str(&s);
                    self.tab_mut().error = None;
                    self.complete = None;
                }
            }
        }
    }
}
