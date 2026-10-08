use super::*;
use crate::app::{JobStmt, Purpose, ResultView};
use crate::config::{Connections, EnvConfig, ProjectConfig, Settings};
use crate::db::{ColKind, ColumnMeta};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn app() -> App {
    let env = |name: &str, url: &str| EnvConfig { name: name.into(), url: url.into(), level: None, read_only: None, password: Some("none".into()), ssh: None };
    let conns = Connections {
        projects: vec![ProjectConfig {
            name: "shop".into(),
            envs: vec![env("local", "postgres://app@localhost:5432/shop"), env("prod", "postgres://app@db.example.com/shop")],
        }],
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut a = App::new(Settings::default(), conns, crate::store::Store::in_memory().unwrap(), tx);
    a.open_project("shop", Some("local"));
    a
}

fn render(a: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    let buf = t.backend().buffer().clone();
    let mut s = String::new();
    for y in 0..h {
        for x in 0..w {
            s.push_str(buf[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

fn key(a: &mut App, code: KeyCode, mods: KeyModifiers) {
    a.on_key(KeyEvent::new(code, mods));
}

fn typ(a: &mut App, s: &str) {
    for c in s.chars() {
        key(a, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

fn grid_view() -> ResultView {
    let stmt = JobStmt { exec: "select".into(), orig: "select * from users".into(), offset: Some(0), label: "users".into(), cap: Some(1000), purpose: Purpose::Query, server_sort: false };
    let mut v = ResultView::new(stmt);
    let col = |n: &str, t: &str, k| ColumnMeta { name: n.into(), type_name: t.into(), kind: k };
    v.rs = crate::grid::ResultSet::new(vec![col("id", "int4", ColKind::Int), col("email", "text", ColKind::Text), col("balance", "numeric", ColKind::Decimal), col("note", "text", ColKind::Text)]);
    v.rs.keys[0].0 = true;
    for i in 0..40 {
        v.rs.push(vec![Some(i.to_string().into()), Some(format!("user{i}@example.com").into()), Some(format!("{}.50", i * 1234).into()), if i % 3 == 0 { None } else { Some("hello\nworld".into()) }]);
    }
    v.rs.complete = true;
    v.done = true;
    v.ms = 12;
    v.grid.fit_widths(&v.rs, true, 200);
    v
}

#[tokio::test]
async fn renders_at_many_sizes() {
    let mut a = app();
    for (w, h) in [(80, 24), (120, 40), (200, 50), (40, 12), (24, 6), (10, 3)] {
        let s = render(&mut a, w, h);
        assert!(!s.is_empty());
    }
    let s = render(&mut a, 80, 24);
    assert!(s.contains("shop"), "{s}");
    assert!(s.contains("local"), "{s}");
    assert!(s.contains("EDITOR"), "{s}");
}

#[tokio::test]
async fn grid_and_selection() {
    let mut a = app();
    a.tabs[0].results.push(grid_view());
    a.focus = Focus::Results;
    let s = render(&mut a, 80, 24);
    assert!(s.contains("email"), "{s}");
    assert!(s.contains("user1@example.com"), "{s}");
    assert!(s.contains("null"), "{s}");
    assert!(s.contains("40 rows"), "{s}");
    assert!(s.contains("1,234.50"), "{s}");
    // move and select
    typ(&mut a, "jjl");
    key(&mut a, KeyCode::Char('v'), KeyModifiers::NONE);
    typ(&mut a, "jl");
    let s = render(&mut a, 80, 24);
    assert!(s.contains("cells"), "{s}");
    // sort descending on balance
    typ(&mut a, "ss");
    let s = render(&mut a, 100, 30);
    assert!(s.contains("↓"), "{s}");
    // inspector
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let s = render(&mut a, 140, 30);
    assert!(s.contains("row "), "{s}");
    let s = render(&mut a, 80, 24);
    assert!(s.contains("row "), "{s}");
}

#[tokio::test]
async fn editor_highlight_and_completion_popup() {
    let mut a = app();
    typ(&mut a, "select id from users where id = 1;");
    let s = render(&mut a, 80, 24);
    assert!(s.contains("select id from users"), "{s}");
    assert!(s.contains("▌"), "{s}");
}

#[tokio::test]
async fn overlays_render() {
    let mut a = app();
    // palette
    key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("go to"), "{s}");
    typ(&mut a, ">help");
    let s = render(&mut a, 80, 24);
    assert!(s.contains("help"), "{s}");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // space menu (instant when the user waits)
    a.focus = Focus::Results;
    key(&mut a, KeyCode::Char(' '), KeyModifiers::NONE);
    a.space.as_mut().unwrap().opened -= std::time::Duration::from_secs(1);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("connection"), "{s}");
    key(&mut a, KeyCode::Char('c'), KeyModifiers::NONE);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("allow writes"), "{s}");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // help
    key(&mut a, KeyCode::Char('?'), KeyModifiers::NONE);
    let s = render(&mut a, 80, 24);
    assert!(s.to_lowercase().contains("sort"), "{s}");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // new connection form
    a.open_conn_form(false);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("new connection"), "{s}");
    let s = render(&mut a, 60, 16);
    assert!(!s.is_empty());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // env switcher
    key(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("prod"), "{s}");
}

#[tokio::test]
async fn prod_is_red_and_read_only() {
    let mut a = app();
    a.switch_env("prod");
    assert!(a.read_only());
    let s = render(&mut a, 80, 24);
    assert!(s.contains("PROD"), "{s}");
    let t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    drop(t);
    // writes are refused before any connection is attempted
    typ(&mut a, "delete from users");
    key(&mut a, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("read-only"), "{s}");
    // unlock: Space c w
    a.do_action(crate::keys::Action::AllowWrites);
    assert!(!a.read_only());
    key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
    typ(&mut a, ">writes");
    let s = render(&mut a, 80, 24);
    assert!(s.contains("lock writes") && !s.contains("allow writes"), "{s}");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('r'), KeyModifiers::CONTROL);
    let s = render(&mut a, 80, 24);
    assert!(s.contains("type prod to confirm"), "{s}");
}

#[tokio::test]
async fn plan_view() {
    let mut a = app();
    let json = r#"[{"Plan": {"Node Type": "Hash Join", "Join Type": "Left", "Total Cost": 100.0, "Plan Rows": 10, "Hash Cond": "(a.id = b.a_id)",
        "Plans": [{"Node Type": "Seq Scan", "Relation Name": "a", "Alias": "a", "Total Cost": 30.0, "Plan Rows": 100}]}}]"#;
    let mut v = grid_view();
    v.body = ResultBody::Plan(crate::explain::parse_pg_json(json, false).unwrap());
    a.tabs[0].results = vec![v];
    let s = render(&mut a, 100, 30);
    assert!(s.contains("Hash Left Join"), "{s}");
    assert!(s.contains("└─ Seq Scan on a"), "{s}");
}

#[tokio::test]
async fn mouse_clicks_hit_what_was_drawn() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut a = app();
    a.add_query_tab(None, "select 2");
    a.tabs[0].results.push(grid_view());
    a.cur = 0;
    render(&mut a, 100, 30);
    let click = |a: &mut App, x: u16, y: u16| {
        let ev = |kind| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
        a.on_mouse(ev(MouseEventKind::Down(MouseButton::Left)));
        a.on_mouse(ev(MouseEventKind::Up(MouseButton::Left)));
        a.last_click = None;
    };
    // tab bar: second tab
    let (_, x0, _) = a.layout.tab_spans[1];
    let (_, first, _) = a.layout.tab_spans[0];
    let ty = a.layout.tabbar.y;
    click(&mut a, x0 + 1, ty);
    assert_eq!(a.cur, 1);
    click(&mut a, first + 1, ty);
    assert_eq!(a.cur, 0);
    render(&mut a, 100, 30);
    // header click sorts by that column
    let (col, x, _) = a.tab().view().unwrap().grid.layout.cols[2];
    let hy = a.tab().view().unwrap().grid.layout.header_y;
    click(&mut a, x, hy);
    assert_eq!(a.tab().view().unwrap().grid.sort, vec![(col, false)]);
    assert_eq!(a.focus, Focus::Results);
    // body click moves the cursor
    render(&mut a, 100, 30);
    let l = a.tab().view().unwrap().grid.layout.clone();
    click(&mut a, l.cols[1].1, l.body_y + 3);
    let g = &a.tab().view().unwrap().grid;
    assert_eq!((g.row, g.col), (3, l.cols[1].0));
    // editor click focuses it
    let e = a.layout.editor;
    let gx = e.x + a.layout.gutter + 2;
    click(&mut a, gx, e.y);
    assert_eq!(a.focus, Focus::Editor);
    // wheel scrolls the grid
    let ev = MouseEvent { kind: MouseEventKind::ScrollDown, column: l.cols[0].1, row: l.body_y, modifiers: KeyModifiers::NONE };
    a.on_mouse(ev);
    assert_eq!(a.tab().view().unwrap().grid.top, 3);
}

#[tokio::test]
async fn scrolling_at_bottom_then_growing_terminal_keeps_marker_in_grid() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let mut a = app();
    a.tabs[0].results.push(grid_view());
    a.focus = Focus::Results;
    render(&mut a, 149, 24);
    for _ in 0..40 {
        let l = &a.tab().view().unwrap().grid.layout;
        let ev = MouseEvent { kind: MouseEventKind::ScrollDown, column: l.cols[0].1, row: l.body_y, modifiers: KeyModifiers::NONE };
        a.on_mouse(ev);
        render(&mut a, 149, 24);
    }
    let s = render(&mut a, 149, 36);
    let v = a.tab().view().unwrap();
    let l = &v.grid.layout;
    assert!(v.grid.top <= v.rs.rows.saturating_sub(l.body_rows as usize));
    assert!(s.lines().skip(l.body_y as usize).take(l.body_rows as usize).any(|line| line.ends_with(a.glyphs.bar)));
}
