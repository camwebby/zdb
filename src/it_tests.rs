//! Live database tests. They drive the whole App (keys in, messages back) against real
//! servers and are skipped unless ZDB_TEST_PG / ZDB_TEST_MYSQL hold connection URLs:
//!
//!   ZDB_TEST_PG=postgres://app:secret@localhost:55432/shop \
//!   ZDB_TEST_MYSQL=mysql://app:secret@localhost:53306/shop cargo test it_ -- --test-threads=1

use crate::app::{App, Focus, Msg, MsgKind, ResultBody};
use crate::config::{Connections, EnvConfig, ProjectConfig, Settings};
use crate::keys::Action;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;

struct H {
    app: App,
    rx: UnboundedReceiver<Msg>,
}

fn harness(var: &str) -> Option<H> {
    let url = std::env::var(var).ok()?;
    let env = |name: &str| EnvConfig { name: name.into(), url: url.clone(), level: None, read_only: None, password: Some("none".into()), ssh: None };
    let conns = Connections { projects: vec![ProjectConfig { name: "it".into(), envs: vec![env("local"), env("prod")] }] };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(Settings::default(), conns, crate::store::Store::in_memory().unwrap(), tx);
    app.open_project("it", Some("local"));
    Some(H { app, rx })
}

impl H {
    /// Process messages until nothing is running (or the timeout passes).
    async fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let busy = self.app.tabs.iter().any(|t| t.run.is_some())
                || self.app.export.is_some()
                || self.app.schemas.values().any(|s| matches!(s, crate::app::SchemaState::Loading));
            if !busy && self.rx.is_empty() {
                // give stragglers (structure, count) a moment
                match tokio::time::timeout(Duration::from_millis(150), self.rx.recv()).await {
                    Ok(Some(m)) => {
                        self.app.on_msg(m);
                        continue;
                    }
                    _ => return,
                }
            }
            assert!(Instant::now() < deadline, "timed out waiting for jobs");
            if let Ok(Some(m)) = tokio::time::timeout(Duration::from_millis(100), self.rx.recv()).await {
                self.app.on_msg(m);
            }
        }
    }

    fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.app.on_key(KeyEvent::new(code, mods));
    }

    fn sql(&mut self, s: &str) {
        let t = self.app.tab_mut();
        t.editor.set_text(s);
        t.editor.doc_end(false);
        self.app.focus = Focus::Editor;
    }

    async fn run(&mut self, s: &str) {
        self.sql(s);
        self.key(KeyCode::Char('r'), KeyModifiers::CONTROL);
        self.accept_simple_confirm();
        self.settle().await;
    }

    async fn run_all(&mut self, s: &str) {
        self.sql(s);
        self.app.do_action(Action::RunAll);
        self.accept_simple_confirm();
        self.settle().await;
    }

    /// Confirmations that don't need typing (local drops) are accepted.
    fn accept_simple_confirm(&mut self) {
        if let Some(crate::app::overlay::Overlay::Confirm(c)) = &self.app.overlay
            && c.require.is_none() {
                self.key(KeyCode::Enter, KeyModifiers::NONE);
            }
    }

    fn rows(&self) -> usize {
        self.app.tab().view().map(|v| v.rs.rows).unwrap_or(0)
    }

    fn cell(&self, r: usize, c: usize) -> Option<String> {
        let v = self.app.tab().view()?;
        v.grid.cell_value(&v.rs, r, c).map(String::from)
    }

    fn errors(&self) -> Vec<String> {
        self.app.tab().messages.iter().filter(|m| m.kind == MsgKind::Error).map(|m| m.text.clone()).collect()
    }

    fn screen(&mut self) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut t = Terminal::new(TestBackend::new(100, 30)).unwrap();
        t.draw(|f| crate::ui::draw(f, &mut self.app)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..30 {
            for x in 0..100 {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }
}

async fn common(h: &mut H, mysql: bool) {
    let int_pk = if mysql { "int auto_increment primary key" } else { "serial primary key" };
    h.run_all(&format!(
        "drop table if exists it_orders;\ndrop table if exists it_users;\n\
         create table it_users (id {int_pk}, email varchar(200) not null, balance numeric(12,2), note text);\n\
         create table it_orders (id {int_pk}, user_id int, total numeric(10,2), foreign key (user_id) references it_users(id));"
    ))
    .await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    let mut values = Vec::new();
    for i in 1..=1500 {
        values.push(format!("('u{i}@x.io', {i}.25, {})", if i % 10 == 0 { "null".to_string() } else { format!("'n{i}'") }));
    }
    h.run(&format!("insert into it_users (email, balance, note) values {};", values.join(","))).await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    h.run("insert into it_orders (user_id, total) values (1, 10), (1, 20), (2, 5);").await;

    // select is capped at the row limit and marked limited
    h.run("select id, email, balance, note from it_users order by id").await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    let scr = h.screen();
    assert_eq!(h.rows(), 1000, "{:?} views={} cur={} {}", h.app.tab().messages, h.app.tab().results.len(), h.app.tab().cur_result, scr);
    assert!(h.app.tab().view().unwrap().rs.limited);
    assert_eq!(h.cell(0, 1).as_deref(), Some("u1@x.io"));
    assert_eq!(h.cell(9, 3), None);
    let s = h.screen();
    assert!(s.contains("1,000 rows"), "{s}");
    assert!(s.contains("limited"), "{s}");

    // server-side sort on a limited result
    h.app.focus = Focus::Results;
    h.app.tab_mut().view_mut().unwrap().grid.col = 0;
    h.key(KeyCode::Char('s'), KeyModifiers::NONE);
    h.settle().await;
    assert_eq!(h.app.tab().view().unwrap().grid.sort, vec![(0, false)]);
    h.key(KeyCode::Char('s'), KeyModifiers::NONE);
    h.settle().await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(h.cell(0, 0).as_deref(), Some("1500"), "server sort desc");

    // load all
    h.key(KeyCode::Char('L'), KeyModifiers::SHIFT);
    h.settle().await;
    assert_eq!(h.rows(), 1500);

    // error position lands in the editor
    h.run("select id, emial from it_users").await;
    let e = h.app.tab().error.clone().expect("editor error");
    assert_eq!(e.line, 0);
    assert_eq!(e.col, 11, "{e:?}");

    // multiple statements → multiple results
    h.run_all("select 1 as a;\nselect 2 as b;").await;
    assert_eq!(h.app.tab().results.len(), 2);

    // transactions
    h.run("begin").await;
    assert!(h.app.tab().in_tx);
    h.run("delete from it_orders where id = 3").await;
    h.run("rollback").await;
    assert!(!h.app.tab().in_tx);
    h.run("select count(*) from it_orders").await;
    assert_eq!(h.cell(0, 0).as_deref(), Some("3"));

    // explain → plan tree
    h.sql("select * from it_users u join it_orders o on o.user_id = u.id where u.id < 10");
    h.app.do_action(Action::Explain);
    h.settle().await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert!(matches!(h.app.tab().view().unwrap().body, ResultBody::Plan(_)), "explain body");
    let s = h.screen();
    assert!(s.to_lowercase().contains("it_users") || s.to_lowercase().contains(" u"), "{s}");

    // schema + completion
    h.app.ensure_schema();
    h.settle().await;
    let sc = h.app.schema().expect("schema");
    let users = sc.tables.iter().find(|t| t.name == "it_users").expect("it_users in schema").display();
    assert_eq!(sc.primary_key(&users), vec!["id".to_string()]);
    let orders = sc.tables.iter().find(|t| t.name == "it_orders").unwrap().display();
    assert!(sc.columns_of(&orders).iter().any(|c| c.name == "user_id" && c.fk.is_some()), "fk on user_id");
    h.sql("select ema");
    h.app.update_completion();
    assert!(h.app.complete.is_none() || h.app.complete.as_ref().unwrap().items.iter().all(|(n, _)| n != "email"));
    h.sql("select u.ema");
    h.app.tab_mut().editor.set_text("select u.ema from it_users u");
    h.app.tab_mut().editor.set_cursor(0, 12);
    h.app.update_completion();
    let c = h.app.complete.clone().expect("completion");
    assert!(c.items.iter().any(|(n, _)| n == "email"), "{c:?}");

    // table tab: browse, filter, sort, structure, count
    h.app.open_table(&users, None);
    h.settle().await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(h.rows(), 500);
    h.app.tabs[h.app.cur].table.as_mut().unwrap().filters.push("id <= 3".into());
    let i = h.app.cur;
    h.app.refresh_table(i);
    h.settle().await;
    assert_eq!(h.rows(), 3);
    h.app.do_action(Action::Structure);
    h.settle().await;
    let s = h.screen();
    assert!(s.contains("email"), "{s}");
    h.app.do_action(Action::Data);
    h.settle().await;
    h.app.do_action(Action::CountRows);
    h.settle().await;

    // staged edit → review → commit
    h.app.focus = Focus::Results;
    {
        let v = h.app.tab_mut().view_mut().unwrap();
        v.grid.row = 0;
        v.grid.col = 1;
    }
    let edited_id = h.cell(0, 0).unwrap();
    h.app.do_action(Action::EditCell);
    {
        let Some(crate::app::overlay::Overlay::Prompt(p)) = &mut h.app.overlay else { panic!("edit prompt") };
        p.input.set("changed@x.io");
    }
    h.key(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(h.app.tab().pending_edits(), 1);
    assert_eq!(h.cell(0, 1).as_deref(), Some("changed@x.io"));
    let s = h.screen();
    assert!(s.contains("1 pending"), "{s}");
    h.key(KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(matches!(h.app.overlay, Some(crate::app::overlay::Overlay::Review(_))));
    h.key(KeyCode::Enter, KeyModifiers::NONE);
    h.settle().await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(h.app.tab().pending_edits(), 0);
    h.app.add_query_tab(None, "");
    h.run(&format!("select email from it_users where id = {edited_id}")).await;
    assert_eq!(h.cell(0, 0).as_deref(), Some("changed@x.io"));

    // follow FK from it_orders.user_id
    h.app.open_table(&orders, None);
    h.settle().await;
    {
        let v = h.app.tab_mut().view_mut().unwrap();
        v.grid.row = 0;
        v.grid.col = 1;
    }
    h.app.focus = Focus::Results;
    h.app.do_action(Action::FollowFk);
    h.settle().await;
    assert_eq!(h.rows(), 1, "followed fk");
    assert_eq!(h.app.tab().table.as_ref().unwrap().name, users);

    // cancel a long query
    h.app.add_query_tab(None, "");
    let sleep = if mysql { "select sleep(10)" } else { "select pg_sleep(10)" };
    h.sql(sleep);
    h.key(KeyCode::Char('r'), KeyModifiers::CONTROL);
    let t0 = Instant::now();
    tokio::time::sleep(Duration::from_millis(700)).await;
    while let Ok(m) = h.rx.try_recv() {
        h.app.on_msg(m);
    }
    h.key(KeyCode::Char('c'), KeyModifiers::CONTROL);
    h.settle().await;
    assert!(t0.elapsed() < Duration::from_secs(5), "cancel took {:?}", t0.elapsed());
    assert!(!h.app.quit);
    // the session is usable after a cancel
    h.run("select 42").await;
    assert_eq!(h.cell(0, 0).as_deref(), Some("42"));

    // prod guard: read-only session refuses writes even if the check is bypassed
    h.app.switch_env("prod");
    h.run("delete from it_orders").await;
    assert!(h.app.toast.as_ref().is_some_and(|t| t.text.contains("read-only")), "toast");
    h.app.do_action(Action::AllowWrites);
    h.run("update it_orders set total = 1").await;
    let Some(crate::app::overlay::Overlay::Confirm(c)) = &h.app.overlay else { panic!("confirm on prod unfiltered update") };
    assert_eq!(c.require.as_deref(), Some("prod"));
    h.key(KeyCode::Esc, KeyModifiers::NONE);
    h.app.switch_env("local");

    // export
    let dir = std::env::temp_dir().join(format!("zdb-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    h.run("select id, email, note from it_users order by id").await;
    let path = dir.join(if mysql { "my.csv" } else { "pg.csv" });
    h.app.start_export('c', path.to_string_lossy().into());
    h.settle().await;
    let csv = std::fs::read_to_string(&path).expect("export file");
    assert_eq!(csv.lines().count(), 1501, "header + all rows (export ignores the cap)");
    let jpath = dir.join("out.json");
    h.app.start_export('j', jpath.to_string_lossy().into());
    h.settle().await;
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&jpath).unwrap()).expect("valid json");
    assert_eq!(j.as_array().unwrap().len(), 1500);

    // copy as INSERT round-trips
    h.run("select id, email, balance, note from it_users where id in (10, 11) order by id").await;
    let v = h.app.tab().view().unwrap();
    let table = crate::copy::Table {
        names: v.rs.cols.iter().map(|c| c.name.as_str()).collect(),
        kinds: v.rs.cols.iter().map(|c| c.kind).collect(),
        rows: (0..v.rs.rows).map(|r| (0..v.rs.cols.len()).map(|c| v.rs.get(r, c)).collect()).collect(),
    };
    let ins = crate::copy::render(crate::copy::Format::Insert, &table, h.app.driver(), "it_users");
    h.run("delete from it_users where id in (10, 11)").await;
    h.run_all(&ins).await;
    assert!(h.errors().is_empty(), "{:?} {ins}", h.errors());
    h.run("select note from it_users where id = 10").await;
    assert_eq!(h.cell(0, 0), None, "null round-trips");

    // history recorded
    assert!(h.app.store.history(100).unwrap().iter().any(|e| e.sql.contains("pg_sleep") || e.sql.contains("sleep(")));
}

#[tokio::test]
async fn it_postgres() {
    let Some(mut h) = harness("ZDB_TEST_PG") else { return };
    common(&mut h, false).await;
    // postgres extras: types, notices, params
    h.run("select 1::int8 as i, 1.5::float8 as f, true as b, '{\"a\":1}'::jsonb as j, now() as t, gen_random_uuid() as u, '\\x0102'::bytea as by").await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(h.cell(0, 2).as_deref(), Some("true"));
    h.run("do $$ begin raise notice 'hello there'; end $$").await;
    assert!(h.app.tab().messages.iter().any(|m| m.text.contains("hello there")), "{:?}", h.app.tab().messages);
    h.sql("select :n::int + 1 as x");
    h.key(KeyCode::Char('r'), KeyModifiers::CONTROL);
    {
        let Some(crate::app::overlay::Overlay::Params(f)) = &mut h.app.overlay else { panic!("param form") };
        f.values[0].set("41");
    }
    h.key(KeyCode::Enter, KeyModifiers::NONE);
    h.settle().await;
    assert_eq!(h.cell(0, 0).as_deref(), Some("42"));
}

#[tokio::test]
async fn it_mysql() {
    let Some(mut h) = harness("ZDB_TEST_MYSQL") else { return };
    common(&mut h, true).await;
    h.run("select cast(1 as signed) as i, 1.5e0 as f, json_object('a', 1) as j, now() as t, x'0102' as b").await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(h.cell(0, 4).as_deref(), Some("0x0102"));
}

#[tokio::test]
async fn it_sqlite_delete_rows() {
    let path = std::env::temp_dir().join(format!("zdb-it-delete-{}.db", std::process::id()));
    std::fs::File::create(&path).unwrap();
    // the harness reads its URL from an env var, so go through one
    unsafe { std::env::set_var("ZDB_TEST_SQLITE_DELETE", format!("sqlite://{}", path.display())) };
    let mut h = harness("ZDB_TEST_SQLITE_DELETE").unwrap();
    h.run_all("create table it_del (id integer primary key, name text); insert into it_del (name) values ('a'),('b'),('c'),('d'),('e');").await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    h.app.open_table("it_del", None);
    h.settle().await;
    h.app.focus = Focus::Results;
    assert_eq!(h.rows(), 5);

    // D on a row asks first, then deletes it and keeps the cursor on the same position
    h.app.tab_mut().view_mut().unwrap().grid.row = 1;
    let (id, next) = (h.cell(1, 0).unwrap(), h.cell(2, 1));
    h.key(KeyCode::Char('D'), KeyModifiers::SHIFT);
    let Some(crate::app::overlay::Overlay::Confirm(c)) = &h.app.overlay else { panic!("confirm") };
    assert_eq!(c.sql, format!("delete from it_del where id = {id};"));
    h.key(KeyCode::Enter, KeyModifiers::NONE);
    h.settle().await;
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    assert_eq!(h.rows(), 4);
    assert_eq!(h.app.tab().view().unwrap().grid.row, 1);
    assert_eq!(h.cell(1, 1), next);

    // Esc on the confirm leaves the data alone
    h.key(KeyCode::Char('D'), KeyModifiers::SHIFT);
    h.key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(h.app.overlay.is_none());
    assert_eq!(h.rows(), 4);

    // a row selection deletes every selected row in one go
    h.app.tab_mut().view_mut().unwrap().grid.row = 0;
    h.key(KeyCode::Down, KeyModifiers::SHIFT);
    h.key(KeyCode::Char('D'), KeyModifiers::SHIFT);
    let Some(crate::app::overlay::Overlay::Confirm(c)) = &h.app.overlay else { panic!("confirm") };
    assert_eq!(c.sql.lines().count(), 2, "{}", c.sql);
    h.key(KeyCode::Enter, KeyModifiers::NONE);
    h.settle().await;
    assert_eq!(h.rows(), 2);

    // staged edits must be committed or undone first
    h.app.do_action(Action::EditCell);
    {
        let Some(crate::app::overlay::Overlay::Prompt(p)) = &mut h.app.overlay else { panic!("edit prompt") };
        p.input.set("x");
    }
    h.key(KeyCode::Enter, KeyModifiers::NONE);
    h.key(KeyCode::Char('D'), KeyModifiers::SHIFT);
    assert!(h.app.overlay.is_none());
    assert_eq!(h.rows(), 2);
    let _ = std::fs::remove_file(&path);
}
