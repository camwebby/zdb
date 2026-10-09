//! Actions, their default keys, the command registry shared by the palette, the Space
//! menu and ? help, and keymap.toml overrides.

use crate::config::config_dir;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // global
    Palette,
    GoToTable,
    SwitchConnection,
    SwitchEnv,
    RunStatement,
    RunAll,
    Cancel,
    NewTab,
    CloseTab,
    Tab(u8),
    NextTab,
    PrevTab,
    ToggleSidebar,
    Save,
    Quit,
    GrowEditor,
    ShrinkEditor,
    // panes
    SpaceMenu,
    Help,
    FocusEditor,
    NextPane,
    PrevPane,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Top,
    Bottom,
    FirstCol,
    LastCol,
    Sort,
    SortAdd,
    SelectCells,
    SelectRows,
    SelectCol,
    CopyMenu,
    Find,
    FindNext,
    FindPrev,
    FitColumn,
    Narrow,
    Widen,
    Inspect,
    LoadAll,
    Filter,
    ClearFilters,
    EditCell,
    UndoEdit,
    DeleteRow,
    NextResult,
    PrevResult,
    Escape,
    // menu / palette
    Explain,
    ExplainAnalyze,
    Begin,
    Commit,
    Rollback,
    RowLimit(usize),
    NewConnection,
    EditConnection,
    AllowWrites,
    RefreshSchema,
    Disconnect,
    OpenTable,
    Structure,
    Data,
    ShowSql,
    CountRows,
    Truncate,
    Copy(char),
    ViewSidebar,
    ViewInspector,
    ViewEditor,
    ViewResults,
    Zen,
    Density,
    KeyHints,
    Export(char),
    History,
    SavedQueries,
    FormatSql,
    AllKeys,
    OpenKeymap,
    OpenSettings,
    OpenTheme,
    ToggleVim,
    ExternalEditor,
    FollowFk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ctx {
    Global,
    Pane,
}

pub struct CmdDef {
    pub action: Action,
    pub id: &'static str,
    /// Palette title, `area: action`. Empty = not in the palette.
    pub title: &'static str,
    /// Space menu path, e.g. "r e".
    /// Space-menu path (checked against `menu()` in tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub menu: &'static str,
    pub keys: &'static [(Ctx, &'static str)],
}

use Action as A;
use Ctx::*;

pub const COMMANDS: &[CmdDef] = &[
    CmdDef { action: A::Palette, id: "palette", title: "", menu: "", keys: &[(Global, "ctrl+k")] },
    CmdDef { action: A::GoToTable, id: "go_to_table", title: "table: go to", menu: "", keys: &[(Global, "ctrl+p")] },
    CmdDef { action: A::SwitchConnection, id: "switch_connection", title: "connection: switch", menu: "c c", keys: &[(Global, "ctrl+o")] },
    CmdDef { action: A::SwitchEnv, id: "switch_env", title: "connection: environment", menu: "c e", keys: &[(Global, "ctrl+e")] },
    CmdDef { action: A::RunStatement, id: "run_statement", title: "run: statement", menu: "r r", keys: &[(Global, "ctrl+r"), (Global, "ctrl+enter")] },
    CmdDef { action: A::RunAll, id: "run_all", title: "run: all", menu: "r a", keys: &[(Global, "alt+r")] },
    CmdDef { action: A::Cancel, id: "cancel", title: "run: cancel", menu: "", keys: &[(Global, "ctrl+c")] },
    CmdDef { action: A::NewTab, id: "new_tab", title: "tab: new", menu: "", keys: &[(Global, "ctrl+t")] },
    CmdDef { action: A::NextTab, id: "next_tab", title: "tab: next", menu: "", keys: &[(Global, "ctrl+n"), (Global, "ctrl+pagedown"), (Pane, "t")] },
    CmdDef { action: A::PrevTab, id: "prev_tab", title: "tab: previous", menu: "", keys: &[(Global, "ctrl+pageup"), (Pane, "T")] },
    CmdDef { action: A::CloseTab, id: "close_tab", title: "tab: close", menu: "", keys: &[(Global, "ctrl+w")] },
    CmdDef { action: A::ToggleSidebar, id: "toggle_sidebar", title: "", menu: "", keys: &[(Global, "ctrl+b")] },
    CmdDef { action: A::Save, id: "save", title: "query: save to project", menu: "", keys: &[(Global, "ctrl+s")] },
    CmdDef { action: A::Quit, id: "quit", title: "app: quit", menu: "", keys: &[(Global, "ctrl+q")] },
    CmdDef { action: A::GrowEditor, id: "grow_editor", title: "", menu: "", keys: &[(Global, "alt+down")] },
    CmdDef { action: A::ShrinkEditor, id: "shrink_editor", title: "", menu: "", keys: &[(Global, "alt+up")] },
    CmdDef { action: A::ExternalEditor, id: "external_editor", title: "editor: open in $EDITOR", menu: "", keys: &[] },
    CmdDef { action: A::SpaceMenu, id: "space_menu", title: "", menu: "", keys: &[(Pane, "space")] },
    CmdDef { action: A::Help, id: "help", title: "", menu: "", keys: &[(Pane, "?")] },
    CmdDef { action: A::FocusEditor, id: "focus_editor", title: "", menu: "", keys: &[(Pane, "i"), (Global, "ctrl+j")] },
    CmdDef { action: A::NextPane, id: "next_pane", title: "", menu: "", keys: &[(Pane, "tab")] },
    CmdDef { action: A::PrevPane, id: "prev_pane", title: "", menu: "", keys: &[(Pane, "shift+tab")] },
    CmdDef { action: A::Up, id: "up", title: "", menu: "", keys: &[(Pane, "up"), (Pane, "k")] },
    CmdDef { action: A::Down, id: "down", title: "", menu: "", keys: &[(Pane, "down"), (Pane, "j")] },
    CmdDef { action: A::Left, id: "left", title: "", menu: "", keys: &[(Pane, "left"), (Pane, "h")] },
    CmdDef { action: A::Right, id: "right", title: "", menu: "", keys: &[(Pane, "right"), (Pane, "l")] },
    CmdDef { action: A::PageUp, id: "page_up", title: "", menu: "", keys: &[(Pane, "pageup"), (Pane, "ctrl+u")] },
    CmdDef { action: A::PageDown, id: "page_down", title: "", menu: "", keys: &[(Pane, "pagedown"), (Pane, "ctrl+d")] },
    CmdDef { action: A::Top, id: "top", title: "", menu: "", keys: &[(Pane, "g"), (Pane, "home")] },
    CmdDef { action: A::Bottom, id: "bottom", title: "", menu: "", keys: &[(Pane, "G"), (Pane, "end")] },
    CmdDef { action: A::FirstCol, id: "first_col", title: "", menu: "", keys: &[(Pane, "0")] },
    CmdDef { action: A::LastCol, id: "last_col", title: "", menu: "", keys: &[(Pane, "$")] },
    CmdDef { action: A::Sort, id: "sort", title: "results: sort column", menu: "", keys: &[(Pane, "s")] },
    CmdDef { action: A::SortAdd, id: "sort_add", title: "results: add secondary sort", menu: "", keys: &[(Pane, "S")] },
    CmdDef { action: A::SelectCells, id: "select_cells", title: "", menu: "", keys: &[(Pane, "v")] },
    CmdDef { action: A::SelectRows, id: "select_rows", title: "", menu: "", keys: &[(Pane, "V")] },
    CmdDef { action: A::SelectCol, id: "select_column", title: "", menu: "", keys: &[(Pane, "alt+v")] },
    CmdDef { action: A::CopyMenu, id: "copy_menu", title: "", menu: "", keys: &[(Pane, "y")] },
    CmdDef { action: A::Find, id: "find", title: "results: find", menu: "", keys: &[(Pane, "/")] },
    CmdDef { action: A::FindNext, id: "find_next", title: "", menu: "", keys: &[(Pane, "n")] },
    CmdDef { action: A::FindPrev, id: "find_prev", title: "", menu: "", keys: &[(Pane, "N")] },
    CmdDef { action: A::FitColumn, id: "fit_column", title: "", menu: "", keys: &[(Pane, "=")] },
    CmdDef { action: A::Narrow, id: "narrow_column", title: "", menu: "", keys: &[(Pane, "<")] },
    CmdDef { action: A::Widen, id: "widen_column", title: "", menu: "", keys: &[(Pane, ">")] },
    CmdDef { action: A::Inspect, id: "inspect", title: "", menu: "", keys: &[(Pane, "enter")] },
    CmdDef { action: A::LoadAll, id: "load_all", title: "results: load all rows", menu: "", keys: &[(Pane, "L")] },
    CmdDef { action: A::Filter, id: "filter", title: "table: add filter", menu: "", keys: &[(Pane, "f")] },
    CmdDef { action: A::ClearFilters, id: "clear_filters", title: "table: clear filters", menu: "", keys: &[(Pane, "F")] },
    CmdDef { action: A::EditCell, id: "edit_cell", title: "", menu: "", keys: &[(Pane, "e")] },
    CmdDef { action: A::UndoEdit, id: "undo_edit", title: "", menu: "", keys: &[(Pane, "u")] },
    CmdDef { action: A::DeleteRow, id: "delete_row", title: "table: delete row", menu: "", keys: &[(Pane, "D")] },
    CmdDef { action: A::NextResult, id: "next_result", title: "", menu: "", keys: &[(Pane, "]")] },
    CmdDef { action: A::PrevResult, id: "prev_result", title: "", menu: "", keys: &[(Pane, "[")] },
    CmdDef { action: A::Escape, id: "escape", title: "", menu: "", keys: &[(Pane, "esc")] },
    CmdDef { action: A::Explain, id: "explain", title: "run: explain", menu: "r e", keys: &[] },
    CmdDef { action: A::ExplainAnalyze, id: "explain_analyze", title: "run: explain analyze", menu: "r E", keys: &[] },
    CmdDef { action: A::Begin, id: "begin", title: "run: begin transaction", menu: "r b", keys: &[] },
    CmdDef { action: A::Commit, id: "commit", title: "run: commit", menu: "r c", keys: &[] },
    CmdDef { action: A::Rollback, id: "rollback", title: "run: rollback", menu: "r k", keys: &[] },
    CmdDef { action: A::RowLimit(100), id: "row_limit_100", title: "run: row limit 100", menu: "r l 1", keys: &[] },
    CmdDef { action: A::RowLimit(1000), id: "row_limit_1000", title: "run: row limit 1,000", menu: "r l 2", keys: &[] },
    CmdDef { action: A::RowLimit(10000), id: "row_limit_10000", title: "run: row limit 10,000", menu: "r l 3", keys: &[] },
    CmdDef { action: A::RowLimit(0), id: "row_limit_none", title: "run: no row limit", menu: "r l n", keys: &[] },
    CmdDef { action: A::NewConnection, id: "new_connection", title: "connection: new", menu: "c n", keys: &[] },
    CmdDef { action: A::EditConnection, id: "edit_connection", title: "connection: edit", menu: "c d", keys: &[] },
    CmdDef { action: A::AllowWrites, id: "allow_writes", title: "connection: allow writes", menu: "c w", keys: &[] },
    CmdDef { action: A::RefreshSchema, id: "refresh_schema", title: "connection: refresh schema", menu: "c s", keys: &[] },
    CmdDef { action: A::Disconnect, id: "disconnect", title: "connection: disconnect", menu: "c x", keys: &[] },
    CmdDef { action: A::OpenTable, id: "open_table", title: "table: open", menu: "t t", keys: &[] },
    CmdDef { action: A::Structure, id: "structure", title: "table: structure", menu: "t s", keys: &[] },
    CmdDef { action: A::Data, id: "data", title: "table: data", menu: "t d", keys: &[] },
    CmdDef { action: A::ShowSql, id: "show_sql", title: "table: show SQL", menu: "t q", keys: &[] },
    CmdDef { action: A::CountRows, id: "count_rows", title: "table: count rows", menu: "t n", keys: &[] },
    CmdDef { action: A::Truncate, id: "truncate", title: "table: truncate", menu: "t T", keys: &[] },
    CmdDef { action: A::Copy('y'), id: "copy_values", title: "copy: values, tab-separated", menu: "y y", keys: &[] },
    CmdDef { action: A::Copy('c'), id: "copy_csv", title: "copy: csv with headers", menu: "y c", keys: &[] },
    CmdDef { action: A::Copy('C'), id: "copy_csv_no_headers", title: "copy: csv without headers", menu: "y C", keys: &[] },
    CmdDef { action: A::Copy('j'), id: "copy_json", title: "copy: json", menu: "y j", keys: &[] },
    CmdDef { action: A::Copy('m'), id: "copy_markdown", title: "copy: markdown table", menu: "y m", keys: &[] },
    CmdDef { action: A::Copy('i'), id: "copy_insert", title: "copy: sql insert statements", menu: "y i", keys: &[] },
    CmdDef { action: A::Copy('n'), id: "copy_names", title: "copy: column names", menu: "y n", keys: &[] },
    CmdDef { action: A::Copy('w'), id: "copy_in_list", title: "copy: in (…) list", menu: "y w", keys: &[] },
    CmdDef { action: A::ViewSidebar, id: "view_sidebar", title: "view: sidebar", menu: "v s", keys: &[] },
    CmdDef { action: A::ViewInspector, id: "view_inspector", title: "view: inspector", menu: "v i", keys: &[] },
    CmdDef { action: A::ViewEditor, id: "view_editor", title: "view: editor", menu: "v e", keys: &[] },
    CmdDef { action: A::ViewResults, id: "view_results", title: "view: results", menu: "v r", keys: &[] },
    CmdDef { action: A::Zen, id: "zen", title: "view: zen", menu: "v z", keys: &[] },
    CmdDef { action: A::Density, id: "density", title: "view: density", menu: "v d", keys: &[] },
    CmdDef { action: A::KeyHints, id: "key_hints", title: "view: key hints", menu: "v h", keys: &[] },
    CmdDef { action: A::Export('c'), id: "export_csv", title: "export: csv", menu: "x c", keys: &[] },
    CmdDef { action: A::Export('j'), id: "export_json", title: "export: json", menu: "x j", keys: &[] },
    CmdDef { action: A::Export('s'), id: "export_sql", title: "export: sql", menu: "x s", keys: &[] },
    CmdDef { action: A::History, id: "history", title: "history: search", menu: "h", keys: &[] },
    CmdDef { action: A::SavedQueries, id: "saved_queries", title: "query: saved queries", menu: "q", keys: &[] },
    CmdDef { action: A::FormatSql, id: "format", title: "editor: format SQL", menu: "f", keys: &[] },
    CmdDef { action: A::AllKeys, id: "all_keys", title: "help: all keys", menu: "?", keys: &[] },
    CmdDef { action: A::OpenKeymap, id: "open_keymap", title: "settings: open keymap", menu: "", keys: &[] },
    CmdDef { action: A::OpenSettings, id: "open_settings", title: "settings: open config", menu: "", keys: &[] },
    CmdDef { action: A::OpenTheme, id: "open_theme", title: "settings: open theme", menu: "", keys: &[] },
    CmdDef { action: A::ToggleVim, id: "toggle_vim", title: "editor: toggle vim mode", menu: "", keys: &[] },
    CmdDef { action: A::FollowFk, id: "follow_fk", title: "table: follow foreign key", menu: "", keys: &[] },
];

/// Normalised key for map lookups: chars carry their case, so SHIFT is dropped for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Key {
    pub fn from_event(k: &KeyEvent) -> Key {
        let mut mods = k.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let code = match k.code {
            // macOS Option+1..9 types these unless the terminal treats Option as Alt;
            // some terminals (kitty protocol) also report the Alt modifier with them
            KeyCode::Char(c) if let Some(i) = OPTION_DIGITS.iter().position(|&o| o == c) => {
                mods = KeyModifiers::ALT;
                KeyCode::Char((b'1' + i as u8) as char)
            }
            KeyCode::Char(c) => {
                mods.remove(KeyModifiers::SHIFT);
                if mods.contains(KeyModifiers::CONTROL) { KeyCode::Char(c.to_ascii_lowercase()) } else { KeyCode::Char(c) }
            }
            KeyCode::BackTab => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            other => other,
        };
        Key { code, mods }
    }
}

const OPTION_DIGITS: [char; 9] = ['¡', '™', '£', '¢', '∞', '§', '¶', '•', 'ª'];

pub fn parse_key(s: &str) -> Option<Key> {
    let s = s.trim();
    let mut mods = KeyModifiers::NONE;
    let mut rest = s;
    loop {
        let lower = rest.to_ascii_lowercase();
        if let Some(r) = lower.strip_prefix("ctrl+").or_else(|| lower.strip_prefix("c-")) {
            mods |= KeyModifiers::CONTROL;
            rest = &rest[rest.len() - r.len()..];
        } else if let Some(r) = lower.strip_prefix("alt+").or_else(|| lower.strip_prefix("m-")).or_else(|| lower.strip_prefix("meta+")) {
            mods |= KeyModifiers::ALT;
            rest = &rest[rest.len() - r.len()..];
        } else if let Some(r) = lower.strip_prefix("shift+").or_else(|| lower.strip_prefix("s-")) {
            mods |= KeyModifiers::SHIFT;
            rest = &rest[rest.len() - r.len()..];
        } else {
            break;
        }
    }
    let code = match rest.to_ascii_lowercase().as_str() {
        "space" => KeyCode::Char(' '),
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" if mods.contains(KeyModifiers::SHIFT) => {
            mods.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        f if f.starts_with('f') && f.len() > 1 && f[1..].parse::<u8>().is_ok() => KeyCode::F(f[1..].parse().ok()?),
        _ => {
            let mut chars = rest.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if mods.contains(KeyModifiers::SHIFT) {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c.to_ascii_uppercase())
            } else if mods.contains(KeyModifiers::CONTROL) {
                KeyCode::Char(c.to_ascii_lowercase())
            } else {
                KeyCode::Char(c)
            }
        }
    };
    Some(Key { code, mods })
}

/// Short form for hints: `^R`, `M-r`, `␣`, `⏎`.
pub fn display_key(k: &Key, ascii: bool) -> String {
    let base = match k.code {
        KeyCode::Char(' ') => if ascii { "spc" } else { "␣" }.to_string(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => if ascii { "ret" } else { "⏎" }.to_string(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => "S-tab".into(),
        KeyCode::Up => "↑".into(),
        KeyCode::Down => "↓".into(),
        KeyCode::Left => "←".into(),
        KeyCode::Right => "→".into(),
        KeyCode::PageUp => "pgup".into(),
        KeyCode::PageDown => "pgdn".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::F(n) => format!("F{n}"),
        _ => "?".into(),
    };
    if k.mods.contains(KeyModifiers::CONTROL) {
        format!("^{}", base.to_uppercase())
    } else if k.mods.contains(KeyModifiers::ALT) {
        format!("M-{base}")
    } else {
        base
    }
}

pub struct Keymap {
    map: HashMap<(Ctx, Key), Action>,
    /// First key per action, for hints.
    first: HashMap<Action, Key>,
    pub ascii: bool,
}

impl Keymap {
    pub fn path() -> std::path::PathBuf {
        config_dir().join("keymap.toml")
    }

    pub fn defaults(ascii: bool) -> Keymap {
        let mut km = Keymap { map: HashMap::new(), first: HashMap::new(), ascii };
        for c in COMMANDS {
            for (ctx, k) in c.keys {
                if let Some(key) = parse_key(k) {
                    km.bind(*ctx, key, c.action);
                }
            }
        }
        for n in 1..=9u8 {
            km.bind(Global, parse_key(&format!("alt+{n}")).unwrap(), Action::Tab(n));
        }
        // macOS terminals without "Option as Meta" send these for Option+1…9 (US layout)
        km
    }

    fn bind(&mut self, ctx: Ctx, key: Key, a: Action) {
        self.map.insert((ctx, key), a);
        self.first.entry(a).or_insert(key);
    }

    /// Load keymap.toml: `[global]` and `[pane]` tables of `action_id = "key"` or `["k1", "k2"]`.
    pub fn load(ascii: bool) -> (Keymap, Option<String>) {
        let mut km = Keymap::defaults(ascii);
        let Ok(text) = std::fs::read_to_string(Self::path()) else { return (km, None) };
        let v: toml::Table = match toml::from_str(&text) {
            Ok(v) => v,
            Err(e) => return (km, Some(format!("keymap.toml: {e}"))),
        };
        let mut errors = Vec::new();
        for (section, ctx) in [("global", Global), ("pane", Pane)] {
            let Some(t) = v.get(section).and_then(|t| t.as_table()) else { continue };
            for (id, keys) in t {
                let Some(cmd) = COMMANDS.iter().find(|c| c.id == id) else {
                    errors.push(format!("unknown action {id}"));
                    continue;
                };
                let list: Vec<String> = match keys {
                    toml::Value::String(s) => vec![s.clone()],
                    toml::Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                    _ => continue,
                };
                // replace this action's bindings in this context
                km.map.retain(|(c, _), a| !(*c == ctx && *a == cmd.action));
                km.first.remove(&cmd.action);
                for k in list {
                    match parse_key(&k) {
                        Some(key) => km.bind(ctx, key, cmd.action),
                        None => errors.push(format!("bad key {k:?} for {id}")),
                    }
                }
            }
        }
        // re-derive hint keys for actions not overridden
        for c in COMMANDS {
            if !km.first.contains_key(&c.action)
                && let Some(k) = km.map.iter().find(|(_, a)| **a == c.action).map(|((_, k), _)| *k) {
                    km.first.insert(c.action, k);
                }
        }
        (km, if errors.is_empty() { None } else { Some(format!("keymap.toml: {}", errors.join(", "))) })
    }

    pub fn lookup(&self, ctx: Ctx, k: &KeyEvent) -> Option<Action> {
        self.map.get(&(ctx, Key::from_event(k))).copied()
    }

    pub fn hint(&self, a: Action) -> String {
        self.first.get(&a).map(|k| display_key(k, self.ascii)).unwrap_or_default()
    }

    pub fn template() -> String {
        let mut out = String::from(
            "# zdb keymap. Uncomment and change any line; values are a key or a list of keys.\n# Keys: ctrl+k, alt+r, shift+tab, space, enter, esc, f5, or a single character (case matters).\n\n[global]\n",
        );
        for c in COMMANDS.iter().filter(|c| c.keys.iter().any(|(x, _)| *x == Global)) {
            let ks: Vec<String> = c.keys.iter().filter(|(x, _)| *x == Global).map(|(_, k)| format!("\"{k}\"")).collect();
            out.push_str(&format!("# {} = [{}]\n", c.id, ks.join(", ")));
        }
        out.push_str("\n[pane]\n");
        for c in COMMANDS.iter().filter(|c| c.keys.iter().any(|(x, _)| *x == Pane)) {
            let ks: Vec<String> = c.keys.iter().filter(|(x, _)| *x == Pane).map(|(_, k)| format!("\"{k}\"")).collect();
            out.push_str(&format!("# {} = [{}]\n", c.id, ks.join(", ")));
        }
        out
    }
}

/// One level of the Space menu.
pub struct MenuItem {
    pub key: char,
    pub label: &'static str,
    pub submenu: Option<&'static str>,
    pub action: Option<Action>,
}

/// Items under a Space menu path ("" = root).
pub fn menu(path: &str) -> (&'static str, Vec<MenuItem>) {
    let item = |key, label, action| MenuItem { key, label, submenu: None, action: Some(action) };
    let sub = |key, label, s| MenuItem { key, label, submenu: Some(s), action: None };
    match path {
        "" => (
            "space",
            vec![
                sub('r', "run…", "r"),
                sub('y', "copy…", "y"),
                sub('c', "connection…", "c"),
                sub('t', "table…", "t"),
                sub('v', "view…", "v"),
                sub('x', "export", "x"),
                item('h', "history", A::History),
                item('q', "saved queries", A::SavedQueries),
                item('f', "format SQL", A::FormatSql),
                item('?', "all keys", A::AllKeys),
            ],
        ),
        "r" => (
            "run",
            vec![
                item('r', "statement", A::RunStatement),
                item('a', "all", A::RunAll),
                item('e', "explain", A::Explain),
                item('E', "explain analyze", A::ExplainAnalyze),
                item('b', "begin transaction", A::Begin),
                item('c', "commit", A::Commit),
                item('k', "rollback", A::Rollback),
                sub('l', "row limit…", "rl"),
            ],
        ),
        "rl" => (
            "row limit",
            vec![
                item('1', "100", A::RowLimit(100)),
                item('2', "1,000", A::RowLimit(1000)),
                item('3', "10,000", A::RowLimit(10000)),
                item('n', "none", A::RowLimit(0)),
            ],
        ),
        "c" => (
            "connection",
            vec![
                item('c', "switch", A::SwitchConnection),
                item('e', "environment", A::SwitchEnv),
                item('n', "new", A::NewConnection),
                item('d', "edit", A::EditConnection),
                item('w', "allow writes", A::AllowWrites),
                item('s', "refresh schema", A::RefreshSchema),
                item('x', "disconnect", A::Disconnect),
            ],
        ),
        "t" => (
            "table",
            vec![
                item('t', "open", A::OpenTable),
                item('s', "structure", A::Structure),
                item('d', "data", A::Data),
                item('q', "show SQL", A::ShowSql),
                item('n', "count rows", A::CountRows),
                item('T', "truncate", A::Truncate),
            ],
        ),
        "y" => (
            "copy",
            vec![
                item('y', "values, tab-separated", A::Copy('y')),
                item('c', "CSV with headers", A::Copy('c')),
                item('C', "CSV without headers", A::Copy('C')),
                item('j', "JSON", A::Copy('j')),
                item('m', "Markdown", A::Copy('m')),
                item('i', "INSERT", A::Copy('i')),
                item('n', "column names", A::Copy('n')),
                item('w', "IN (…) list", A::Copy('w')),
            ],
        ),
        "v" => (
            "view",
            vec![
                item('s', "sidebar", A::ViewSidebar),
                item('i', "inspector", A::ViewInspector),
                item('e', "editor", A::ViewEditor),
                item('r', "results", A::ViewResults),
                item('z', "zen", A::Zen),
                item('d', "density", A::Density),
                item('h', "key hints", A::KeyHints),
            ],
        ),
        "x" => ("export", vec![item('c', "CSV", A::Export('c')), item('j', "JSON", A::Export('j')), item('s', "SQL", A::Export('s'))]),
        _ => ("", vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_lookup() {
        let km = Keymap::defaults(false);
        let ev = |c, m| KeyEvent::new(c, m);
        assert_eq!(km.lookup(Global, &ev(KeyCode::Char('k'), KeyModifiers::CONTROL)), Some(Action::Palette));
        assert_eq!(km.lookup(Pane, &ev(KeyCode::Char('S'), KeyModifiers::SHIFT)), Some(Action::SortAdd));
        assert_eq!(km.lookup(Pane, &ev(KeyCode::BackTab, KeyModifiers::SHIFT)), Some(Action::PrevPane));
        assert_eq!(km.lookup(Global, &ev(KeyCode::Char('3'), KeyModifiers::ALT)), Some(Action::Tab(3)));
        assert_eq!(km.lookup(Global, &ev(KeyCode::Enter, KeyModifiers::CONTROL)), Some(Action::RunStatement));
        assert_eq!(km.hint(Action::RunStatement), "^R");
        assert_eq!(km.hint(Action::SpaceMenu), "␣");
        assert_eq!(parse_key("shift+s"), parse_key("S"));
        assert_eq!(parse_key("ctrl+K"), parse_key("ctrl+k"));
    }

    #[test]
    fn menu_paths_resolve() {
        for c in COMMANDS.iter().filter(|c| !c.menu.is_empty()) {
            let keys: Vec<char> = c.menu.split(' ').map(|k| k.chars().next().unwrap()).collect();
            let mut path = String::new();
            for (i, k) in keys.iter().enumerate() {
                let (_, items) = menu(&path);
                let it = items.iter().find(|it| it.key == *k).unwrap_or_else(|| panic!("{} missing at {path:?}", c.id));
                if i + 1 == keys.len() {
                    assert_eq!(it.action, Some(c.action), "{}", c.id);
                } else {
                    path = it.submenu.unwrap().to_string();
                }
            }
        }
    }
}
