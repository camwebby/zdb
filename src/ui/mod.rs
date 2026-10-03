//! Rendering. Four calm bands — top line, editor, results, status line — plus the
//! sidebar, inspector, Space menu and overlays. Drawing also records where things
//! landed (`app.layout`) so mouse clicks can be hit-tested.

mod editor;
mod grid;
mod overlay;

use crate::app::overlay::{LineInput, Overlay};
use crate::app::{App, Focus, MsgKind, ResultBody, SchemaState, UiLayout};
use crate::config::Level;
use crate::keys::{Action, menu};
use crate::theme::{Glyphs, Theme};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.size = (area.width, area.height);
    app.layout = UiLayout::default();
    f.render_widget(Clear, area);
    if area.height < 6 || area.width < 24 {
        f.render_widget(Paragraph::new("zdb · terminal too small"), area);
        return;
    }
    let zen = app.zen;
    let mut y = area.y;
    if !zen {
        let top = Rect { x: area.x, y, width: area.width, height: 1 };
        draw_top(f, app, top);
        y += 1;
        let tabs = Rect { x: area.x, y, width: area.width, height: 1 };
        draw_tabs(f, app, tabs);
        y += 1;
    }
    let status = Rect { x: area.x, y: area.bottom() - 1, width: area.width, height: 1 };
    let body = Rect { x: area.x, y, width: area.width, height: status.y.saturating_sub(y) };
    app.layout.status = status;

    let mut main = body;
    let side_sidebar = app.sidebar_open && !app.narrow() && !zen;
    if side_sidebar {
        let w = 30.min(body.width / 3);
        let r = Rect { width: w, ..body };
        draw_sidebar(f, app, r);
        vline(f, &app.theme, Rect { x: r.right(), width: 1, ..body }, app.focus == Focus::Sidebar);
        main.x += w + 1;
        main.width = main.width.saturating_sub(w + 1);
    }
    let side_insp = app.inspector_open && app.inspector_docked();
    if side_insp {
        let w = 44.min(main.width / 3);
        let r = Rect { x: main.right() - w, width: w, ..main };
        main.width -= w + 1;
        vline(f, &app.theme, Rect { x: main.right(), width: 1, ..body }, app.focus == Focus::Inspector);
        draw_inspector(f, app, r);
    }
    draw_main(f, app, main);

    if app.sidebar_open && !side_sidebar {
        let w = 40.min(body.width);
        let r = Rect { width: w, ..body };
        f.render_widget(Clear, r);
        draw_sidebar(f, app, Rect { width: w.saturating_sub(1), ..r });
        vline(f, &app.theme, Rect { x: r.right() - 1, width: 1, ..r }, true);
    }
    if app.inspector_open && !side_insp && matches!(app.focus, Focus::Results | Focus::Inspector) {
        let r = centered(body, 70, body.height.saturating_sub(2));
        f.render_widget(Clear, r);
        let inner = framed(f, &app.theme, r, "row", true);
        draw_inspector(f, app, inner);
        app.layout.inspector = r;
    }
    draw_status(f, app, status);
    if app.space_visible() {
        draw_space_menu(f, app, body);
    }
    overlay::draw(f, app, area);
}

// ---------------- helpers ----------------

pub fn trunc(s: &str, w: usize, ell: &str) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let ew = ell.width();
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw + ew > w {
            break;
        }
        out.push(ch);
        used += cw;
    }
    if ew <= w {
        out.push_str(ell);
    }
    out
}

pub fn pad(s: &str, w: usize) -> String {
    let n = s.width();
    if n >= w { s.to_string() } else { format!("{s}{}", " ".repeat(w - n)) }
}

pub fn pad_left(s: &str, w: usize) -> String {
    let n = s.width();
    if n >= w { s.to_string() } else { format!("{}{s}", " ".repeat(w - n)) }
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2)).max(10.min(area.width));
    let h = h.min(area.height).max(3.min(area.height));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn vline(f: &mut Frame, theme: &Theme, r: Rect, focused: bool) {
    let style = if focused { Style::default().fg(theme.accent) } else { Style::default().add_modifier(Modifier::DIM) };
    for y in r.y..r.bottom() {
        f.buffer_mut().set_string(r.x, y, "│", style);
    }
}

/// Draw a box with a title; returns the inner rect.
pub fn framed(f: &mut Frame, theme: &Theme, r: Rect, title: &str, focused: bool) -> Rect {
    use ratatui::widgets::{Block, BorderType, Borders};
    let style = if focused { Style::default().fg(theme.accent) } else { Style::default().add_modifier(Modifier::DIM) };
    let b = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(style).title(Span::styled(format!(" {title} "), Style::default().add_modifier(Modifier::BOLD)));
    let inner = b.inner(r);
    f.render_widget(Clear, r);
    f.render_widget(b, r);
    inner
}

/// A horizontal rule with left content spans; returns x positions of each span.
fn rule(f: &mut Frame, r: Rect, glyphs: &Glyphs, style: Style, left: Vec<Span<'static>>, right: Vec<Span<'static>>) -> Vec<(u16, u16)> {
    let buf = f.buffer_mut();
    let line = glyphs.rule.repeat(r.width as usize);
    buf.set_stringn(r.x, r.y, &line, r.width as usize, style);
    let mut x = r.x + 1;
    let mut spans = Vec::new();
    let rw: u16 = right.iter().map(|s| s.content.width() as u16).sum();
    let limit = r.right().saturating_sub(rw + 2);
    for s in left {
        let w = s.content.width() as u16;
        if x >= limit {
            spans.push((x, x));
            continue;
        }
        let avail = (limit - x) as usize;
        let text = trunc(&s.content, avail, glyphs.ellipsis);
        let (nx, _) = buf.set_stringn(x, r.y, &text, avail, s.style);
        spans.push((x, nx));
        x = nx;
        let _ = w;
    }
    if rw > 0 && r.width > rw + 2 {
        let mut x = r.right() - rw - 1;
        for s in right {
            let (nx, _) = buf.set_stringn(x, r.y, &s.content, (r.right() - x) as usize, s.style);
            x = nx;
        }
    }
    spans
}

pub fn input_spans(li: &LineInput, masked: bool, width: usize, style: Style) -> (Vec<Span<'static>>, u16) {
    let text: String = if masked { "•".repeat(li.text.chars().count()) } else { li.text.clone() };
    let before: String = text.chars().take(li.cursor).collect();
    let cw = before.width();
    // scroll horizontally so the cursor stays visible
    let skip_w = cw.saturating_sub(width.saturating_sub(1));
    let mut skipped = 0;
    let mut shown = String::new();
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if skipped < skip_w {
            skipped += w;
            continue;
        }
        shown.push(ch);
    }
    let shown = trunc(&shown, width, "");
    (vec![Span::styled(shown, style)], (cw - skipped) as u16)
}

fn level_style(theme: &Theme, l: Level) -> Style {
    Style::default().fg(theme.env(l))
}

// ---------------- top line ----------------

fn draw_top(f: &mut Frame, app: &mut App, r: Rect) {
    let t = &app.theme;
    let g = &app.glyphs;
    let level = app.level();
    let prod = level == Level::Prod && app.has_connection();
    let base = if prod { Style::default().bg(t.prod).fg(t.prod_fg) } else { Style::default() };
    f.buffer_mut().set_style(r, base);
    let mut spans: Vec<Span> = vec![Span::styled(" zdb ", base.add_modifier(Modifier::BOLD))];
    let env_start;
    if app.has_connection() {
        spans.push(Span::styled(format!("{} ", app.project), base));
        env_start = spans.iter().map(|s| s.content.width()).sum::<usize>() as u16;
        let env_style = if prod { base.add_modifier(Modifier::BOLD) } else { level_style(t, level).add_modifier(Modifier::BOLD) };
        spans.push(Span::styled(format!("{} {}", g.dot, app.env), env_style));
        let lbl = match level {
            Level::Prod => {
                if app.read_only() {
                    " PROD · read-only".to_string()
                } else {
                    " PROD · WRITES ON".to_string()
                }
            }
            Level::Staging => " staging".into(),
            Level::Local => String::new(),
        };
        spans.push(Span::styled(lbl, if prod { base.add_modifier(Modifier::BOLD) } else { level_style(t, level) }));
        let w: usize = spans.iter().map(|s| s.content.width()).sum();
        app.layout.env_span = (r.x + env_start, r.x + w as u16);
        if let Some(e) = app.env_config()
            && let Ok(u) = crate::config::UrlParts::parse(&e.url) {
                spans.push(Span::styled(format!("  {}", u.display()), if prod { base } else { Style::default().add_modifier(Modifier::DIM) }));
            }
        match app.schemas.get(&app.key()) {
            Some(SchemaState::Loading) => spans.push(Span::styled("  loading schema…", base.add_modifier(Modifier::DIM))),
            Some(SchemaState::Failed(_)) => spans.push(Span::styled("  schema unavailable", Style::default().fg(t.error))),
            _ => {}
        }
    } else {
        spans.push(Span::styled("no connection · Space c n adds one", base.add_modifier(Modifier::DIM)));
    }
    // right: running state
    let mut right = String::new();
    if let Some(run) = &app.tab().run {
        let secs = run.started.elapsed().as_secs_f64();
        let sp = g.spinner[app.spinner % g.spinner.len()];
        right = if run.cancelling {
            format!("{sp} cancelling… ")
        } else if secs >= app.settings.slow_query_secs as f64 {
            format!("{sp} still running {:.0}s · ^C cancel ", secs)
        } else {
            format!("{sp} {:.1}s ", secs)
        };
    } else if let Some(e) = &app.export {
        let sp = g.spinner[app.spinner % g.spinner.len()];
        let file = std::path::Path::new(&e.path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        right = format!("{sp} exporting {} rows to {file} · ^C cancel ", crate::grid::group_digits(&e.rows.to_string()));
    }
    let rw = right.width() as u16;
    let avail = r.width.saturating_sub(rw + 1) as usize;
    let mut x = r.x;
    for s in spans {
        if (x - r.x) as usize >= avail {
            break;
        }
        let rem = avail - (x - r.x) as usize;
        let text = trunc(&s.content, rem, g.ellipsis);
        let (nx, _) = f.buffer_mut().set_stringn(x, r.y, &text, rem, s.style);
        x = nx;
    }
    if rw > 0 && r.width > rw {
        let style = if prod { base } else { Style::default().fg(t.accent) };
        f.buffer_mut().set_string(r.right() - rw, r.y, &right, style);
    }
    app.layout.top = r;
}

fn draw_tabs(f: &mut Frame, app: &mut App, r: Rect) {
    let t = &app.theme;
    let g = &app.glyphs;
    let mut x = r.x;
    let mut spans = Vec::new();
    // keep the current tab visible: start from a tab such that current fits
    let widths: Vec<u16> = app.tabs.iter().map(|tab| tab_label(tab, g).width() as u16 + 2).collect();
    let mut start = 0;
    while start < app.cur {
        let w: u16 = widths[start..=app.cur].iter().sum();
        if w <= r.width.saturating_sub(4) {
            break;
        }
        start += 1;
    }
    if start > 0 {
        f.buffer_mut().set_string(x, r.y, "‹ ", Style::default().add_modifier(Modifier::DIM));
        x += 2;
    }
    for (i, tab) in app.tabs.iter().enumerate().skip(start) {
        let label = tab_label(tab, g);
        let w = label.width() as u16 + 2;
        if x + w > r.right() {
            f.buffer_mut().set_string(r.right().saturating_sub(1), r.y, "›", Style::default().add_modifier(Modifier::DIM));
            break;
        }
        let cur = i == app.cur;
        let mut style = if cur { Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED).fg(t.accent) } else { Style::default().add_modifier(Modifier::DIM) };
        if tab.run.is_some() && !cur {
            style = style.remove_modifier(Modifier::DIM);
        }
        if tab.stale_from.is_some() {
            style = style.add_modifier(Modifier::ITALIC);
        }
        f.buffer_mut().set_string(x + 1, r.y, &label, style);
        spans.push((i, x, x + w));
        x += w;
    }
    app.layout.tabbar = r;
    app.layout.tab_spans = spans;
}

fn tab_label(tab: &crate::app::Tab, g: &Glyphs) -> String {
    let mut s = String::new();
    if tab.is_table() {
        s.push_str(if g.dot == "*" { "# " } else { "▦ " });
    }
    s.push_str(&tab.name);
    if tab.pending_edits() > 0 {
        s.push_str(&format!(" {}{}", g.dot, tab.pending_edits()));
    }
    if tab.in_tx {
        s.push_str(" tx");
    }
    if tab.run.is_some() {
        s.push_str(&format!(" {}", g.ellipsis));
    }
    s
}

// ---------------- main area ----------------

fn draw_main(f: &mut Frame, app: &mut App, area: Rect) {
    if area.height < 2 || area.width < 4 {
        return;
    }
    if app.tabs.is_empty() {
        return;
    }
    let tab = app.tab();
    let editor_shown = tab.editor_visible() && !app.hide_editor;
    let results_shown = !app.hide_results || !editor_shown;
    let chips = tab.table.as_ref().is_some_and(|t| !t.filters.is_empty() && !t.structure);
    let mut y = area.y;
    let total = area.height;
    let prod = app.level() == Level::Prod && app.has_connection();

    if editor_shown {
        let ed_h = if results_shown {
            let avail = total.saturating_sub(2 + chips as u16);
            ((avail as u32 * app.split as u32 / 100) as u16).clamp(1.min(avail), avail.saturating_sub(1).max(1))
        } else {
            total - 1
        };
        let rule_r = Rect { x: area.x, y, width: area.width, height: 1 };
        let focused = app.focus == Focus::Editor;
        let style = if prod {
            Style::default().fg(app.theme.prod)
        } else if focused {
            Style::default().fg(app.theme.accent)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        let tab = app.tab();
        let label = match &tab.table {
            Some(t) => format!(" SQL for {} (read-only) ", t.name),
            None => format!(" {} ", tab.saved.clone().unwrap_or_else(|| tab.name.clone())),
        };
        let mut right = Vec::new();
        if let Some(n) = tab.row_limit {
            right.push(Span::styled(if n == 0 { " no row limit ".to_string() } else { format!(" limit {} ", crate::grid::group_digits(&n.to_string())) }, Style::default().add_modifier(Modifier::DIM)));
        }
        if prod {
            right.push(Span::styled(if app.read_only() { " prod · read-only ".to_string() } else { " prod · writes allowed ".to_string() }, Style::default().fg(app.theme.prod).add_modifier(Modifier::BOLD)));
        }
        let lstyle = if focused { Style::default().add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::DIM) };
        rule(f, rule_r, &app.glyphs, style, vec![Span::styled(label, lstyle)], right);
        y += 1;
        let ed_r = Rect { x: area.x, y, width: area.width, height: ed_h };
        editor::draw(f, app, ed_r);
        y += ed_h;
    } else {
        app.layout.editor = Rect::default();
    }
    if !results_shown {
        return;
    }
    let rule_r = Rect { x: area.x, y, width: area.width, height: 1 };
    draw_results_rule(f, app, rule_r);
    y += 1;
    if chips {
        let r = Rect { x: area.x, y, width: area.width, height: 1 };
        draw_chips(f, app, r);
        y += 1;
    }
    let grid_r = Rect { x: area.x, y, width: area.width, height: area.bottom().saturating_sub(y) };
    app.layout.grid = grid_r;
    draw_results(f, app, grid_r);
}

fn draw_results_rule(f: &mut Frame, app: &mut App, r: Rect) {
    let t = &app.theme;
    let g = &app.glyphs;
    let focused = matches!(app.focus, Focus::Results);
    let tab = app.tab();
    let style = if focused { Style::default().fg(t.accent) } else { Style::default().add_modifier(Modifier::DIM) };
    let mut left: Vec<Span<'static>> = Vec::new();
    let mut idx = Vec::new();
    let views = tab.views();
    let structure = tab.table.as_ref().is_some_and(|t| t.structure);
    for (i, v) in views.iter().enumerate() {
        let cur = i == tab.cur_result;
        let mut label = if views.len() > 1 { format!(" {} {} ", i + 1, v.label) } else { format!(" {} ", v.label) };
        if structure && i == 0 && views.len() == 1 {
            label = " structure ".into();
        }
        let s = if cur && focused {
            Style::default().add_modifier(Modifier::BOLD).fg(t.accent)
        } else if cur {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        left.push(Span::styled(label, s));
        idx.push(i);
    }
    if !tab.messages.is_empty() || views.is_empty() {
        let errors = tab.messages.iter().filter(|m| m.kind == MsgKind::Error).count();
        let cur = tab.on_messages();
        let label = if views.is_empty() && tab.messages.is_empty() {
            " results ".to_string()
        } else if errors > 0 {
            format!(" messages {} ", g.cross)
        } else {
            format!(" messages ({}) ", tab.messages.len())
        };
        let mut s = if cur { Style::default().add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::DIM) };
        if errors > 0 {
            s = s.fg(t.error).remove_modifier(Modifier::DIM);
        } else if cur && focused {
            s = s.fg(t.accent);
        }
        left.push(Span::styled(label, s));
        idx.push(views.len());
    }
    // right summary
    let mut right = Vec::new();
    if let Some(v) = tab.view() {
        let mut s = String::new();
        if matches!(v.body, ResultBody::Grid) && !v.rs.cols.is_empty() {
            let n = crate::grid::group_digits(&v.rs.rows.to_string());
            s.push_str(&format!("{n} row{}", if v.rs.rows == 1 { "" } else { "s" }));
            if v.rs.limited {
                s.push_str(" · limited · L loads all");
            } else if !v.done {
                s.push_str(&format!(" {}", g.ellipsis));
            }
        }
        if v.done && v.ms > 0 {
            s.push_str(&format!(" · {}", crate::app::run::fmt_ms(v.ms)));
        }
        if !s.is_empty() {
            right.push(Span::styled(format!(" {} ", s.trim_start_matches(" · ")), Style::default().add_modifier(Modifier::DIM)));
        }
    }
    if let Some(old) = &tab.stale_from {
        right.push(Span::styled(format!(" from {old} · stale "), Style::default().fg(t.staging)));
    }
    let spans = rule(f, r, g, style, left, right);
    app.layout.result_spans = idx.into_iter().zip(spans).map(|(i, (a, b))| (i, a, b)).collect();
    app.layout.results_rule = r;
}

fn draw_chips(f: &mut Frame, app: &App, r: Rect) {
    let Some(t) = &app.tab().table else { return };
    let mut x = r.x + 1;
    f.buffer_mut().set_string(x, r.y, "where", Style::default().add_modifier(Modifier::DIM));
    x += 6;
    for (i, c) in t.filters.iter().enumerate() {
        if i > 0 {
            let (nx, _) = f.buffer_mut().set_stringn(x, r.y, "and ", (r.right().saturating_sub(x)) as usize, Style::default().add_modifier(Modifier::DIM));
            x = nx;
        }
        let text = format!("[{c}]");
        let (nx, _) = f.buffer_mut().set_stringn(x, r.y, &text, (r.right().saturating_sub(x)) as usize, Style::default().fg(app.theme.accent));
        x = nx + 1;
        if x >= r.right() {
            break;
        }
    }
    let hint = "  F clear";
    if x + hint.len() as u16 <= r.right() {
        f.buffer_mut().set_string(x, r.y, hint, Style::default().add_modifier(Modifier::DIM));
    }
}

fn draw_results(f: &mut Frame, app: &mut App, r: Rect) {
    if r.height == 0 {
        return;
    }
    let tab = app.tab();
    let running = tab.run.is_some();
    if tab.on_messages() {
        if tab.messages.is_empty() {
            let msg = if running {
                let sp = app.glyphs.spinner[app.spinner % app.glyphs.spinner.len()];
                format!("{sp} running…  ^C cancel")
            } else if !app.has_connection() {
                "Ctrl+K → @ to pick a connection, or Space c n to add one".to_string()
            } else if tab.is_table() {
                "loading…".to_string()
            } else {
                let run = app.keymap.hint(Action::RunStatement);
                format!("{run} runs the statement under the cursor · {} runs all", app.keymap.hint(Action::RunAll))
            };
            let y = r.y + r.height / 3;
            let x = r.x + (r.width.saturating_sub(msg.width() as u16)) / 2;
            f.buffer_mut().set_stringn(x, y, &msg, r.width as usize, Style::default().add_modifier(Modifier::DIM));
            return;
        }
        draw_messages(f, app, r);
        return;
    }
    let stale = tab.stale_from.is_some();
    let focused = app.focus == Focus::Results || app.focus == Focus::Inspector;
    let App { tabs, cur, theme, glyphs, settings, .. } = app;
    let tab = &mut tabs[*cur];
    let i = tab.cur_result;
    let Some(v) = tab.views_mut().get_mut(i) else { return };
    let cx = grid::Ctx { theme, glyphs, group: settings.group_digits, focused, stale, comfortable: settings.density == crate::config::Density::Comfortable };
    match &v.body {
        ResultBody::Grid => grid::draw(f, r, v, &cx),
        ResultBody::Plan(_) => grid::draw_plan(f, r, v, &cx),
        ResultBody::Text(_) => grid::draw_text(f, r, v, &cx),
    }
}

fn draw_messages(f: &mut Frame, app: &App, r: Rect) {
    let t = &app.theme;
    let tab = app.tab();
    let mut lines: Vec<Line> = Vec::new();
    for m in &tab.messages {
        let style = match m.kind {
            MsgKind::Error => Style::default().fg(t.error),
            MsgKind::Notice => Style::default().fg(t.staging),
            MsgKind::Info => Style::default(),
        };
        for (j, l) in m.text.lines().enumerate() {
            let prefix = if j > 0 {
                "  "
            } else {
                match m.kind {
                    MsgKind::Error => "✗ ",
                    MsgKind::Notice => "! ",
                    MsgKind::Info => "· ",
                }
            };
            let prefix = if app.settings.ascii && prefix == "✗ " { "x " } else { prefix };
            lines.push(Line::from(vec![Span::styled(prefix.to_string(), style.add_modifier(Modifier::DIM)), Span::styled(l.to_string(), style)]));
        }
    }
    let h = r.height as usize;
    let skip = lines.len().saturating_sub(h);
    let lines: Vec<Line> = lines.into_iter().skip(skip).collect();
    f.render_widget(Paragraph::new(lines), Rect { x: r.x + 1, width: r.width.saturating_sub(1), ..r });
}

// ---------------- sidebar ----------------

fn draw_sidebar(f: &mut Frame, app: &mut App, r: Rect) {
    app.layout.sidebar = r;
    if r.height == 0 || r.width < 4 {
        return;
    }
    let focused = app.focus == Focus::Sidebar;
    let t = app.theme.clone();
    let mut y = r.y;
    let title = if app.has_connection() { format!("{} · {}", app.project, app.env) } else { "no connection".into() };
    f.buffer_mut().set_stringn(r.x + 1, y, trunc(&title, r.width as usize - 1, app.glyphs.ellipsis), r.width as usize - 1, Style::default().add_modifier(Modifier::BOLD).fg(t.env(app.level())));
    y += 1;
    if app.sidebar.filtering || !app.sidebar.filter.text.is_empty() {
        let (spans, cx) = input_spans(&app.sidebar.filter, false, r.width as usize - 3, Style::default());
        f.buffer_mut().set_string(r.x + 1, y, "/", Style::default().fg(t.accent));
        f.buffer_mut().set_line(r.x + 2, y, &Line::from(spans), r.width - 2);
        if app.sidebar.filtering && app.overlay.is_none() {
            f.set_cursor_position((r.x + 2 + cx, y));
        }
        y += 1;
    }
    let items = app.sidebar_items();
    if items.is_empty() {
        let msg = match app.schemas.get(&app.key()) {
            Some(SchemaState::Loading) => "loading…".to_string(),
            Some(SchemaState::Failed(e)) => format!("schema failed: {e}"),
            _ if !app.sidebar.filter.text.is_empty() => "no matches".into(),
            _ => "nothing here yet".into(),
        };
        f.render_widget(Paragraph::new(msg).style(Style::default().add_modifier(Modifier::DIM)).wrap(ratatui::widgets::Wrap { trim: true }), Rect { x: r.x + 1, y, width: r.width - 1, height: r.bottom().saturating_sub(y) });
        app.layout.sidebar_items_y = y;
        return;
    }
    let h = r.bottom().saturating_sub(y) as usize;
    app.sidebar.sel = app.sidebar.sel.min(items.len() - 1);
    if app.sidebar.sel < app.sidebar.scroll {
        app.sidebar.scroll = app.sidebar.sel;
    } else if h > 0 && app.sidebar.sel >= app.sidebar.scroll + h {
        app.sidebar.scroll = app.sidebar.sel + 1 - h;
    }
    app.layout.sidebar_items_y = y;
    for (i, it) in items.iter().enumerate().skip(app.sidebar.scroll).take(h) {
        let sel = i == app.sidebar.sel;
        let (text, mut style) = match it {
            crate::app::input::SidebarItem::Header(h) => (h.to_uppercase(), Style::default().add_modifier(Modifier::DIM | Modifier::BOLD)),
            crate::app::input::SidebarItem::Table(n) => (format!(" {n}"), Style::default()),
            crate::app::input::SidebarItem::Saved(n, _) => (format!(" {n}"), Style::default()),
        };
        if sel {
            style = if focused { style.add_modifier(Modifier::REVERSED) } else { style.add_modifier(Modifier::BOLD) };
        }
        let w = r.width as usize - 1;
        f.buffer_mut().set_stringn(r.x + 1, y, pad(&trunc(&text, w, app.glyphs.ellipsis), w), w, style);
        y += 1;
    }
}

// ---------------- inspector ----------------

fn draw_inspector(f: &mut Frame, app: &mut App, r: Rect) {
    if app.layout.inspector == Rect::default() {
        app.layout.inspector = r;
    }
    if r.height == 0 || r.width < 6 {
        return;
    }
    let t = &app.theme;
    let tab = app.tab();
    let Some(v) = tab.view().filter(|v| matches!(v.body, ResultBody::Grid) && !v.rs.cols.is_empty() && v.rs.rows > 0) else {
        f.buffer_mut().set_stringn(r.x + 1, r.y, "no row selected", r.width as usize - 1, Style::default().add_modifier(Modifier::DIM));
        return;
    };
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("row {} of {}", crate::grid::group_digits(&(v.grid.row + 1).to_string()), crate::grid::group_digits(&v.rs.rows.to_string())),
        Style::default().add_modifier(Modifier::DIM),
    ))];
    let w = r.width.saturating_sub(2) as usize;
    let data_row = v.grid.data_row(v.grid.row);
    for (c, col) in v.rs.cols.iter().enumerate() {
        let edited = v.grid.edits.contains_key(&(data_row, c));
        let val = v.grid.cell_value(&v.rs, v.grid.row, c);
        let mut head = vec![Span::styled(col.name.clone(), Style::default().add_modifier(Modifier::BOLD).fg(if c == v.grid.col { t.accent } else { Color::Reset }))];
        head.push(Span::styled(format!("  {}", col.type_name), Style::default().add_modifier(Modifier::DIM)));
        if let Some((pk, fk)) = v.rs.keys.get(c) {
            if *pk {
                head.push(Span::styled(" pk", Style::default().add_modifier(Modifier::DIM)));
            }
            if let Some((tb, _)) = fk {
                head.push(Span::styled(format!(" {} {tb}", app.glyphs.arrow), Style::default().add_modifier(Modifier::DIM)));
            }
        }
        lines.push(Line::from(head));
        let vstyle = if edited { Style::default().fg(t.pending) } else { Style::default() };
        match val {
            None => lines.push(Line::from(Span::styled("  null", Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC)))),
            Some(s) => {
                let pretty = if col.kind == crate::db::ColKind::Json {
                    serde_json::from_str::<serde_json::Value>(s).ok().and_then(|j| serde_json::to_string_pretty(&j).ok()).unwrap_or_else(|| s.to_string())
                } else {
                    s.to_string()
                };
                for l in pretty.lines() {
                    for chunk in wrap_width(l, w.saturating_sub(2).max(1)) {
                        lines.push(Line::from(Span::styled(format!("  {chunk}"), vstyle)));
                    }
                }
                if pretty.is_empty() {
                    lines.push(Line::from(Span::styled("  (empty)", Style::default().add_modifier(Modifier::DIM))));
                }
            }
        }
    }
    let max_scroll = lines.len().saturating_sub(r.height as usize);
    app.inspector_scroll = app.inspector_scroll.min(max_scroll);
    let lines: Vec<Line> = lines.into_iter().skip(app.inspector_scroll).take(r.height as usize).collect();
    f.render_widget(Paragraph::new(lines), Rect { x: r.x + 1, width: r.width - 1, ..r });
}

pub fn wrap_width(s: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cw = 0;
    for ch in s.chars() {
        let chw = ch.width().unwrap_or(0);
        if cw + chw > w && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cw = 0;
        }
        cur.push(ch);
        cw += chw;
    }
    out.push(cur);
    out
}

// ---------------- status line ----------------

fn draw_status(f: &mut Frame, app: &mut App, r: Rect) {
    let t = app.theme.clone();
    let g = app.glyphs.clone();
    let buf_w = r.width as usize;
    // inline prompt replaces the status line
    if let Some(Overlay::Prompt(p)) = &app.overlay
        && p.inline() {
            let label = format!(" {}: ", p.label);
            f.buffer_mut().set_string(r.x, r.y, &label, Style::default().fg(t.accent).add_modifier(Modifier::BOLD));
            let x = r.x + label.width() as u16;
            let w = (r.right().saturating_sub(x)) as usize;
            let hint = if p.null { "  null · type to replace" } else { "" };
            let (spans, cx) = input_spans(&p.input, false, w.saturating_sub(hint.width() + 1), Style::default());
            let line = Line::from(spans);
            f.buffer_mut().set_line(x, r.y, &line, w as u16);
            if !hint.is_empty() && p.input.text.is_empty() {
                f.buffer_mut().set_string(x, r.y, hint, Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC));
            }
            if let Some(c) = p.completions.get(p.comp_idx) {
                let right = format!(" tab {c} ");
                if right.width() + 20 < buf_w {
                    f.buffer_mut().set_string(r.right() - right.width() as u16, r.y, &right, Style::default().add_modifier(Modifier::DIM));
                }
            }
            f.set_cursor_position((x + cx, r.y));
            return;
        }
    let tab = app.tab();
    let mut left: Vec<Span> = Vec::new();
    let focus = app.focus;
    let mode = format!(" {} ", focus.label(tab.is_table()));
    left.push(Span::styled(mode, Style::default().bg(t.accent).fg(Color::Black).add_modifier(Modifier::BOLD)));
    if focus == Focus::Editor
        && let Some(m) = tab.editor.vim_mode() {
            let s = match m {
                crate::editor::VimMode::Normal => " NORMAL ",
                crate::editor::VimMode::Insert => " INSERT ",
                crate::editor::VimMode::Visual => " VISUAL ",
                crate::editor::VimMode::VisualLine => " V-LINE ",
            };
            left.push(Span::styled(s, Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)));
        }
    if tab.in_tx {
        left.push(Span::styled(" TX ", Style::default().bg(t.staging).fg(Color::Black).add_modifier(Modifier::BOLD)));
    }
    if app.has_connection() {
        if app.level() == Level::Prod && !app.read_only() {
            left.push(Span::styled(" PROD rw ", Style::default().bg(t.prod).fg(t.prod_fg).add_modifier(Modifier::BOLD)));
        } else if app.read_only() {
            left.push(Span::styled(" ro", Style::default().add_modifier(Modifier::DIM)));
        }
    }
    let info = status_info(app);
    if !info.is_empty() {
        left.push(Span::styled(format!(" {info}"), Style::default().add_modifier(Modifier::DIM)));
    }
    // right side: toast or hints
    let right: Option<Span> = if let Some(toast) = &app.toast {
        let s = if toast.error { Style::default().fg(t.error).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.accent) };
        Some(Span::styled(format!("{} ", toast.text), s))
    } else if app.chord == Some('x') {
        Some(Span::styled("^X … ^E edit in $EDITOR ", Style::default().add_modifier(Modifier::DIM)))
    } else if let Some(sp) = &app.space {
        Some(Span::styled(format!("{}{} … ", g.space, sp.path.chars().map(|c| format!(" {c}")).collect::<String>()), Style::default().fg(t.accent)))
    } else if app.settings.key_hints {
        Some(Span::styled(format!("{} ", hints(app)), Style::default().add_modifier(Modifier::DIM)))
    } else {
        None
    };
    let lw: usize = left.iter().map(|s| s.content.width()).sum();
    let mut x = r.x;
    // toasts take priority over the stats text
    let rw = right.as_ref().map(|s| s.content.width()).unwrap_or(0);
    let toast_priority = app.toast.is_some();
    let left_limit = if toast_priority { buf_w.saturating_sub(rw + 1) } else { buf_w };
    for s in &left {
        let used = (x - r.x) as usize;
        if used >= left_limit {
            break;
        }
        let text = trunc(&s.content, left_limit - used, g.ellipsis);
        let (nx, _) = f.buffer_mut().set_stringn(x, r.y, &text, left_limit - used, s.style);
        x = nx;
    }
    if let Some(s) = right {
        let used = (x - r.x) as usize;
        let avail = buf_w.saturating_sub(used + 2);
        if avail > 3 {
            let text = trunc(&s.content, avail, g.ellipsis);
            let w = text.width() as u16;
            f.buffer_mut().set_string(r.right() - w, r.y, &text, s.style);
        }
    }
    let _ = lw;
}

fn status_info(app: &App) -> String {
    let tab = app.tab();
    match app.focus {
        Focus::Editor => {
            let (r, c) = tab.editor.cursor();
            let mut s = format!("ln {}, col {}", r + 1, c + 1);
            if let Some(sel) = tab.editor.selected_text() {
                s.push_str(&format!(" · {} chars selected", sel.chars().count()));
            }
            if let Some((ok, msg)) = &tab.last
                && *ok {
                    s.push_str(&format!(" · {}", msg.lines().next().unwrap_or("")));
                }
            s
        }
        Focus::Results | Focus::Inspector => {
            let Some(v) = tab.view() else { return String::new() };
            if !matches!(v.body, ResultBody::Grid) || v.rs.cols.is_empty() {
                return String::new();
            }
            if let Some((rows, cols)) = v.grid.selected(&v.rs) {
                let n = (rows.end() - rows.start() + 1) * (cols.end() - cols.start() + 1);
                if n <= 200_000 {
                    let numeric = cols.clone().all(|c| v.rs.cols[c].kind.numeric());
                    let mut vals = Vec::with_capacity(n);
                    for r in rows.clone() {
                        for c in cols.clone() {
                            vals.push(v.grid.cell_value(&v.rs, r, c));
                        }
                    }
                    let stats = crate::grid::selection_stats(&vals, numeric);
                    if v.grid.sel.is_some_and(|s| s.kind == crate::grid::SelKind::Rows) {
                        let nr = rows.end() - rows.start() + 1;
                        return format!("{} row{} · {stats}", crate::grid::group_digits(&nr.to_string()), if nr == 1 { "" } else { "s" });
                    }
                    return stats;
                }
                return format!("{} cells", crate::grid::group_digits(&n.to_string()));
            }
            let mut s = format!("{} · row {}/{}", v.rs.cols[v.grid.col].name, crate::grid::group_digits(&(v.grid.row + 1).to_string()), crate::grid::group_digits(&v.rs.rows.to_string()));
            if let Some(fd) = &v.grid.find {
                s.push_str(&format!(" · /{} {}/{}", fd.query, if fd.matches.is_empty() { 0 } else { fd.idx + 1 }, fd.matches.len()));
            }
            let pe = tab.pending_edits();
            if pe > 0 {
                s.push_str(&format!(" · {pe} pending · ^S review"));
            }
            s
        }
        Focus::Sidebar => String::new(),
    }
}

fn hints(app: &App) -> String {
    let km = &app.keymap;
    let h = |a: Action, label: &str| {
        let k = km.hint(a);
        if k.is_empty() { String::new() } else { format!("{k} {label}") }
    };
    let list: Vec<String> = match app.focus {
        Focus::Editor => {
            let mut v = vec![h(Action::RunStatement, "run"), h(Action::Palette, "palette")];
            let insert = app.tab().editor.vim_mode() == Some(crate::editor::VimMode::Insert);
            v.push(if insert { "esc normal".into() } else { "esc results".into() });
            v.push(h(Action::Help, "help"));
            v
        }
        Focus::Results => {
            if app.tab().pending_edits() > 0 {
                vec![h(Action::Save, "review"), "u undo edit".into(), h(Action::Help, "help")]
            } else {
                vec!["y copy".into(), "s sort".into(), "v select".into(), format!("{} menu", app.glyphs.space), h(Action::Help, "help")]
            }
        }
        Focus::Sidebar => vec![format!("{} open", app.glyphs.enter), "/ filter".into(), "s structure".into()],
        Focus::Inspector => vec!["j k scroll".into(), "n p next/prev row".into(), "esc back".into()],
    };
    list.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

// ---------------- Space menu ----------------

fn draw_space_menu(f: &mut Frame, app: &App, body: Rect) {
    let Some(sp) = &app.space else { return };
    let (title, items) = menu(&sp.path);
    if items.is_empty() {
        return;
    }
    let t = &app.theme;
    let col_w = items.iter().map(|it| it.label.width() + if it.submenu.is_some() { 4 } else { 3 }).max().unwrap_or(10) as u16 + 2;
    let cols = ((body.width.saturating_sub(4)) / col_w).max(1).min(items.len() as u16);
    let rows = (items.len() as u16).div_ceil(cols);
    let h = rows + 2;
    let w = (col_w * cols + 3).min(body.width);
    let r = Rect { x: body.right().saturating_sub(w + 1).max(body.x), y: body.bottom().saturating_sub(h), width: w, height: h.min(body.height) };
    let path_title = if sp.path.is_empty() { title.to_string() } else { format!("{} {}", app.glyphs.space, sp.path.chars().map(String::from).collect::<Vec<_>>().join(" ")) + &format!(" · {title}") };
    let inner = framed(f, t, r, &path_title, true);
    for (i, it) in items.iter().enumerate() {
        let c = i as u16 / rows;
        let row = i as u16 % rows;
        let x = inner.x + 1 + c * col_w;
        let y = inner.y + row;
        if y >= inner.bottom() {
            continue;
        }
        f.buffer_mut().set_string(x, y, it.key.to_string(), Style::default().fg(t.accent).add_modifier(Modifier::BOLD));
        let label = if it.submenu.is_some() { format!("{} {}", it.label, "") } else { it.label.to_string() };
        let style = if it.submenu.is_some() { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
        f.buffer_mut().set_stringn(x + 2, y, &label, col_w.saturating_sub(3) as usize, style);
    }
}

#[cfg(test)]
mod tests;
