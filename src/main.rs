//! zdb — a calm, fast terminal UI for PostgreSQL and MySQL.

mod app;
mod config;
mod copy;
mod db;
mod editor;
mod explain;
mod grid;
mod keys;
mod secrets;
mod sql;
mod store;
mod theme;
mod ui;
#[cfg(test)]
mod it_tests;

use anyhow::{Context, Result};
use app::{App, External};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, EventStream, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use futures::StreamExt;
use std::io::stdout;
use std::time::Duration;

const USAGE: &str = "zdb — terminal UI for PostgreSQL and MySQL

usage:
  zdb                     reopen the last project
  zdb <project>           open a project (its last environment, or the first non-prod one)
  zdb <project>:<env>     open a project on a specific environment
  zdb <url>               scratch connection, e.g. postgres://user@localhost/app

config:   $XDG_CONFIG_HOME/zdb  (connections.toml, config.toml, keymap.toml, theme.toml)
data:     $XDG_DATA_HOME/zdb    (history, workspaces, saved queries)
";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("zdb {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() > 1 {
        eprint!("{USAGE}");
        std::process::exit(2);
    }

    let (settings, settings_err) = match config::Settings::load() {
        Ok(s) => (s, None),
        Err(e) => (config::Settings::default(), Some(format!("config.toml: {e}"))),
    };
    let conns = config::Connections::load().context("reading connections.toml")?;
    let data = config::data_dir();
    std::fs::create_dir_all(&data).with_context(|| format!("creating {}", data.display()))?;
    let store = store::Store::open(&data.join("zdb.db")).context("opening history database")?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(settings, conns, store, tx);

    match args.first() {
        Some(a) if a.contains("://") => {
            if config::UrlParts::parse(a).ok().and_then(|p| p.driver()).is_none() {
                anyhow::bail!("unsupported URL scheme in {a} (use postgres:// or mysql://)");
            }
            let (p, e) = app.conns.add_scratch(a);
            app.scratch_unsaved = true;
            app.open_project(&p, Some(&e));
        }
        Some(a) => {
            let (p, e) = match a.split_once(':') {
                Some((p, e)) => (p, Some(e)),
                None => (a.as_str(), None),
            };
            let Some(proj) = app.conns.project(p) else {
                let known: Vec<_> = app.conns.projects.iter().map(|p| p.name.clone()).collect();
                anyhow::bail!("no project named {p}{}", if known.is_empty() { String::new() } else { format!(" (known: {})", known.join(", ")) });
            };
            if let Some(e) = e
                && !proj.envs.iter().any(|x| x.name == e) {
                    let envs: Vec<_> = proj.envs.iter().map(|x| x.name.clone()).collect();
                    anyhow::bail!("project {p} has no environment {e} (has: {})", envs.join(", "));
                }
            app.open_project(p, e);
        }
        None => {
            let last = app.store.get("last_project").filter(|p| app.conns.project(p).is_some());
            match last.or_else(|| app.conns.projects.first().map(|p| p.name.clone())) {
                Some(p) => app.open_project(&p, None),
                None => {
                    app.add_query_tab(None, "");
                    app.open_conn_form(false);
                }
            }
        }
    }
    if let Some(e) = settings_err {
        app.toast_err(e);
    }

    let mut terminal = ratatui::init();
    let kitty = app.settings.kitty_keyboard && crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    enter_extras(&app, kitty);
    let res = run(&mut terminal, &mut app, &mut rx, kitty).await;
    leave_extras(&app, kitty);
    ratatui::restore();
    res
}

fn enter_extras(app: &App, kitty: bool) {
    let mut out = stdout();
    let _ = execute!(out, EnableBracketedPaste);
    if app.settings.mouse {
        let _ = execute!(out, EnableMouseCapture);
    }
    if kitty {
        let _ = execute!(out, PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    }
}

fn leave_extras(app: &App, kitty: bool) {
    let mut out = stdout();
    if kitty {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    if app.settings.mouse {
        let _ = execute!(out, DisableMouseCapture);
    }
    let _ = execute!(out, DisableBracketedPaste);
}

async fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App, rx: &mut tokio::sync::mpsc::UnboundedReceiver<app::Msg>, kitty: bool) -> Result<()> {
    let mut events = EventStream::new();
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if app.quit {
            return Ok(());
        }
        let wait = app.next_deadline().unwrap_or(Duration::from_secs(3600));
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(k))) => app.on_key(k),
                Some(Ok(Event::Mouse(m))) => app.on_mouse(m),
                Some(Ok(Event::Paste(s))) => app.on_paste(s),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            m = rx.recv() => {
                if let Some(m) = m {
                    app.on_msg(m);
                    while let Ok(m) = rx.try_recv() {
                        app.on_msg(m);
                    }
                }
            }
            _ = tokio::time::sleep(wait) => {}
        }
        app.tick();
        if let Some(x) = app.external.take() {
            // release the terminal (and stdin) while the editor runs
            drop(events);
            leave_extras(app, kitty);
            ratatui::restore();
            let result = external_editor(app, x);
            // re-enter without ratatui::init (its panic hook is already installed) and
            // without Terminal::clear (it queries the cursor position, which races stdin)
            crossterm::terminal::enable_raw_mode()?;
            execute!(stdout(), crossterm::terminal::EnterAlternateScreen, crossterm::terminal::Clear(crossterm::terminal::ClearType::All))?;
            *terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;
            enter_extras(app, kitty);
            events = EventStream::new();
            if let Err(e) = result {
                app.toast_err(e);
            }
        }
    }
}

fn external_editor(app: &mut App, x: External) -> Result<(), String> {
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "vi".into());
    let mut parts = shell_words::split(&editor).map_err(|e| format!("$EDITOR: {e}"))?;
    if parts.is_empty() {
        return Err("$EDITOR is empty".into());
    }
    let prog = parts.remove(0);
    let launch = |path: &std::path::Path| -> Result<(), String> {
        let st = std::process::Command::new(&prog).args(&parts).arg(path).status().map_err(|e| format!("{prog}: {e}"))?;
        if st.success() { Ok(()) } else { Err(format!("{prog} exited with {st}")) }
    };
    match x {
        External::EditBuffer => {
            let path = std::env::temp_dir().join(format!("zdb-{}-{}.sql", std::process::id(), app.tab().id));
            std::fs::write(&path, app.tab().editor.text()).map_err(|e| e.to_string())?;
            let r = launch(&path);
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string());
            let _ = std::fs::remove_file(&path);
            r?;
            let text = text?;
            let text = text.strip_suffix('\n').unwrap_or(&text);
            if text != app.tab().editor.text() {
                let ed = &mut app.tab_mut().editor;
                let (r, c) = ed.cursor();
                ed.set_text(text);
                ed.set_cursor(r, c);
                app.tab_mut().error = None;
            }
            app.focus = app::Focus::Editor;
            Ok(())
        }
        External::EditFile(path) => {
            launch(&path)?;
            app.reload_config();
            Ok(())
        }
    }
}
