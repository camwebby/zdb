//! Overlays: palette, environment switcher, help, confirmations, prompts,
//! parameter form, connection form and the staged-edit review.

use super::{centered, framed, input_spans, pad, trunc};
use crate::app::App;
use crate::app::overlay::*;
use crate::config::Level;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(o) = app.overlay.take() else { return };
    match &o {
        Overlay::Palette(p) => palette(f, app, area, p),
        Overlay::Env(sel) => env_popup(f, app, area, *sel),
        Overlay::Help(h) => help(f, app, area, h),
        Overlay::Confirm(c) => confirm(f, app, area, c),
        Overlay::Prompt(p) if !p.inline() => prompt(f, app, area, p),
        Overlay::Prompt(_) => {}
        Overlay::Params(p) => params(f, app, area, p),
        Overlay::Form(c) => form(f, app, area, c),
        Overlay::Review(r) => review(f, app, area, r),
        Overlay::Insert(i) => insert(f, app, area, i),
    }
    app.overlay = Some(o);
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

fn footer(f: &mut Frame, r: Rect, text: &str) {
    if r.height == 0 {
        return;
    }
    let y = r.bottom() - 1;
    f.buffer_mut().set_stringn(r.x + 1, y, text, r.width.saturating_sub(1) as usize, dim());
}

fn palette(f: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let t = &app.theme;
    let w = 84.min(area.width.saturating_sub(4));
    let h = 18.min(area.height.saturating_sub(2));
    let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height / 6).min(area.height - h), width: w, height: h };
    let inner = framed(f, t, r, "go to", true);
    if inner.height < 3 {
        return;
    }
    let (spans, cx) = input_spans(&p.input, false, inner.width as usize - 3, Style::default());
    f.buffer_mut().set_string(inner.x, inner.y, "›", Style::default().fg(t.accent).add_modifier(Modifier::BOLD));
    f.buffer_mut().set_line(inner.x + 2, inner.y, &Line::from(spans), inner.width - 2);
    f.set_cursor_position((inner.x + 2 + cx, inner.y));
    if p.input.text.is_empty() {
        f.buffer_mut().set_string(inner.x + 2, inner.y, "type to search · @ connections # tables > commands / saved ! history", Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC));
    }
    let list_h = (inner.height - 2) as usize;
    let start = p.scroll.min(p.shown.len().saturating_sub(1));
    let start = if p.sel < start { p.sel } else if p.sel >= start + list_h { p.sel + 1 - list_h } else { start };
    for (row, &idx) in p.shown.iter().skip(start).take(list_h).enumerate() {
        let it = &p.items[idx];
        let y = inner.y + 2 + row as u16;
        let sel = start + row == p.sel;
        let base = if sel { Style::default().bg(t.selection).add_modifier(Modifier::BOLD) } else { Style::default() };
        let full = inner.width as usize;
        f.buffer_mut().set_stringn(inner.x, y, " ".repeat(full), full, base);
        f.buffer_mut().set_string(inner.x + 1, y, it.prefix.to_string(), base.fg(t.accent));
        let hint_w = it.hint.width().min(full / 2);
        let label_w = full.saturating_sub(hint_w + 5);
        f.buffer_mut().set_stringn(inner.x + 3, y, trunc(&it.label, label_w, app.glyphs.ellipsis), label_w, base);
        if hint_w > 0 {
            let hs = match it.hint_level {
                Some(Level::Prod) => base.fg(t.prod),
                Some(Level::Staging) => base.fg(t.staging),
                _ => base.add_modifier(Modifier::DIM),
            };
            let hint = trunc(&it.hint, hint_w, app.glyphs.ellipsis);
            f.buffer_mut().set_string(inner.right() - hint.width() as u16 - 1, y, &hint, hs);
        }
    }
    if p.shown.is_empty() {
        f.buffer_mut().set_string(inner.x + 3, inner.y + 2, "no matches", dim());
    }
    let n = p.shown.len();
    let count = format!(" {n} ");
    if r.width > count.len() as u16 + 4 {
        f.buffer_mut().set_string(r.right() - count.len() as u16 - 2, r.bottom() - 1, &count, dim());
    }
}

fn env_popup(f: &mut Frame, app: &App, area: Rect, sel: usize) {
    let t = &app.theme;
    let Some(p) = app.conns.project(&app.project) else { return };
    let w = p.envs.iter().map(|e| e.name.width()).max().unwrap_or(8).max(20) as u16 + 16;
    let h = p.envs.len() as u16 + 3;
    let r = Rect { x: (app.layout.env_span.0).min(area.right().saturating_sub(w)), y: area.y + 1, width: w.min(area.width), height: h.min(area.height - 1) };
    let inner = framed(f, t, r, &p.name, true);
    for (i, e) in p.envs.iter().enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.bottom() {
            break;
        }
        let cur = e.name == app.env;
        let st = if i == sel { Style::default().bg(t.selection).add_modifier(Modifier::BOLD) } else { Style::default() };
        let line = format!(" {} {}{}", app.glyphs.dot, pad(&e.name, inner.width as usize - 14), if cur { " current" } else { "" });
        f.buffer_mut().set_stringn(inner.x, y, pad(&line, inner.width as usize), inner.width as usize, st);
        f.buffer_mut().set_string(inner.x + 1, y, app.glyphs.dot, st.fg(t.env(e.level())));
        if i < 9 {
            f.buffer_mut().set_string(inner.right() - 2, y, (i + 1).to_string(), st.add_modifier(Modifier::DIM));
        }
    }
    footer(f, inner, &format!("{} switch · 1-9 · esc", app.glyphs.enter));
}

fn help(f: &mut Frame, app: &App, area: Rect, h: &Help) {
    let t = &app.theme;
    let r = centered(area, 90, area.height.saturating_sub(2));
    let inner = framed(f, t, r, &h.title, true);
    if inner.height < 3 {
        return;
    }
    let mut y = inner.y;
    if h.searching || !h.query.text.is_empty() {
        let (spans, cx) = input_spans(&h.query, false, inner.width as usize - 3, Style::default());
        f.buffer_mut().set_string(inner.x + 1, y, "/", Style::default().fg(t.accent));
        f.buffer_mut().set_line(inner.x + 3, y, &Line::from(spans), inner.width - 3);
        if h.searching {
            f.set_cursor_position((inner.x + 3 + cx, y));
        }
        y += 2;
    }
    let vis = h.visible();
    let mut lines: Vec<Line> = Vec::new();
    let mut section = "";
    let key_w = vis.iter().map(|(k, _, _)| k.width()).max().unwrap_or(6).min(28);
    for (k, d, s) in &vis {
        if s != section {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(Span::styled(s.to_uppercase(), Style::default().add_modifier(Modifier::BOLD | Modifier::DIM))));
            section = s;
        }
        lines.push(Line::from(vec![Span::styled(format!("  {}", pad(&trunc(k, key_w, "…"), key_w)), Style::default().fg(t.accent)), Span::raw(format!("  {d}"))]));
    }
    let body_h = inner.bottom().saturating_sub(y + 1) as usize;
    let scroll = h.scroll.min(lines.len().saturating_sub(body_h));
    for (i, l) in lines.into_iter().skip(scroll).take(body_h).enumerate() {
        f.buffer_mut().set_line(inner.x + 1, y + i as u16, &l, inner.width - 1);
    }
    footer(f, inner, "/ search · j k scroll · ? all keys · esc close");
}

fn confirm(f: &mut Frame, app: &App, area: Rect, c: &Confirm) {
    let t = &app.theme;
    let sql_lines: Vec<&str> = c.sql.lines().collect();
    let body_lines = super::wrap_width(&c.body, 66);
    let h = (body_lines.len() + sql_lines.len().min(12) + if c.require.is_some() { 3 } else { 0 } + 7) as u16;
    let r = centered(area, 72, h);
    let focus_style = if c.prod { Style::default().fg(t.prod) } else { Style::default().fg(t.accent) };
    let title = c.title.clone();
    let inner = {
        use ratatui::widgets::{Block, BorderType, Borders, Clear};
        let b = Block::default().borders(Borders::ALL).border_type(if c.prod { BorderType::Thick } else { BorderType::Rounded }).border_style(focus_style).title(Span::styled(format!(" {title} "), focus_style.add_modifier(Modifier::BOLD)));
        let inner = b.inner(r);
        f.render_widget(Clear, r);
        f.render_widget(b, r);
        inner
    };
    let mut y = inner.y;
    let target_style = if c.prod { Style::default().fg(t.prod).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
    f.buffer_mut().set_stringn(inner.x + 1, y, &c.target, inner.width as usize - 1, target_style);
    y += 2;
    for l in &body_lines {
        if y >= inner.bottom() {
            break;
        }
        f.buffer_mut().set_stringn(inner.x + 1, y, l, inner.width as usize - 1, Style::default());
        y += 1;
    }
    if !sql_lines.is_empty() {
        y += 1;
        let avail = inner.bottom().saturating_sub(y + if c.require.is_some() { 5 } else { 2 }) as usize;
        let scroll = c.scroll.min(sql_lines.len().saturating_sub(avail));
        for l in sql_lines.iter().skip(scroll).take(avail) {
            f.buffer_mut().set_stringn(inner.x + 2, y, l, inner.width as usize - 2, Style::default().fg(t.keyword));
            y += 1;
        }
        if sql_lines.len() > avail {
            f.buffer_mut().set_string(inner.right() - 12, y.saturating_sub(1), format!("↑↓ {} lines", sql_lines.len()), dim());
        }
    }
    if let Some(req) = &c.require {
        y += 1;
        if y < inner.bottom() {
            f.buffer_mut().set_stringn(inner.x + 1, y, format!("type {req} to confirm:"), inner.width as usize, Style::default().add_modifier(Modifier::BOLD));
            y += 1;
        }
        if y < inner.bottom() {
            let ok = c.typed.text.trim() == req;
            let st = if ok { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default() };
            let (spans, cx) = input_spans(&c.typed, false, inner.width as usize - 4, st);
            f.buffer_mut().set_string(inner.x + 1, y, "›", Style::default().fg(t.accent));
            f.buffer_mut().set_line(inner.x + 3, y, &Line::from(spans), inner.width - 3);
            f.set_cursor_position((inner.x + 3 + cx, y));
        }
    }
    footer(f, inner, &format!("{} confirm · esc cancel", app.glyphs.enter));
}

fn prompt(f: &mut Frame, app: &App, area: Rect, p: &Prompt) {
    let t = &app.theme;
    let extra = p.completions.len().min(6) as u16;
    let r = centered(area, 64, 5 + extra);
    let title = match &p.kind {
        PromptKind::Password { project, env, .. } => format!("password for {project} · {env}"),
        _ => p.label.clone(),
    };
    let inner = framed(f, t, r, &title, true);
    let y = inner.y;
    let (spans, cx) = input_spans(&p.input, p.masked(), inner.width as usize - 4, Style::default());
    f.buffer_mut().set_string(inner.x + 1, y, "›", Style::default().fg(t.accent));
    f.buffer_mut().set_line(inner.x + 3, y, &Line::from(spans), inner.width - 3);
    f.set_cursor_position((inner.x + 3 + cx, y));
    for (i, c) in p.completions.iter().take(6).enumerate() {
        let st = if i == p.comp_idx { Style::default().fg(t.accent) } else { dim() };
        f.buffer_mut().set_stringn(inner.x + 3, y + 1 + i as u16, c, inner.width as usize - 3, st);
    }
    let foot = match &p.kind {
        PromptKind::Password { store, .. } => format!("{} connect · tab {} in keychain · esc", app.glyphs.enter, if *store { "[x] save" } else { "[ ] save" }),
        PromptKind::ExportPath { .. } => format!("{} export · tab complete · esc", app.glyphs.enter),
        _ => format!("{} ok · esc cancel", app.glyphs.enter),
    };
    footer(f, inner, &foot);
}

fn params(f: &mut Frame, app: &App, area: Rect, p: &ParamForm) {
    let t = &app.theme;
    let r = centered(area, 64, p.names.len() as u16 + 5);
    let inner = framed(f, t, r, "parameters", true);
    let nw = p.names.iter().map(|n| n.width()).max().unwrap_or(4) as u16 + 2;
    for (i, (n, v)) in p.names.iter().zip(&p.values).enumerate() {
        let y = inner.y + 1 + i as u16;
        if y >= inner.bottom().saturating_sub(1) {
            break;
        }
        let focused = i == p.idx;
        f.buffer_mut().set_string(inner.x + 1, y, pad(n, nw as usize), if focused { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default() });
        let (spans, cx) = input_spans(v, false, (inner.width - nw - 2) as usize, Style::default());
        f.buffer_mut().set_line(inner.x + 1 + nw, y, &Line::from(spans), inner.width - nw - 1);
        if v.text.is_empty() {
            f.buffer_mut().set_string(inner.x + 1 + nw, y, "null", dim().add_modifier(Modifier::ITALIC));
        }
        if focused {
            f.set_cursor_position((inner.x + 1 + nw + cx, y));
        }
    }
    footer(f, inner, &format!("tab next · {} run · values are SQL literals unless numeric · esc", app.glyphs.enter));
}

fn insert(f: &mut Frame, app: &App, area: Rect, form: &InsertForm) {
    let t = &app.theme;
    let prod = app.level() == Level::Prod;
    let n = form.fields.len();
    let list_h = n.min((area.height as usize).saturating_sub(10).max(3));
    let r = centered(area, 86, list_h as u16 + 7 + form.error.is_some() as u16);
    let title = format!("insert · {}", form.table);
    let inner = if prod {
        use ratatui::widgets::{Block, BorderType, Borders, Clear};
        let st = Style::default().fg(t.prod);
        let b = Block::default().borders(Borders::ALL).border_type(BorderType::Thick).border_style(st).title(Span::styled(format!(" {title} "), st.add_modifier(Modifier::BOLD)));
        let inner = b.inner(r);
        f.render_widget(Clear, r);
        f.render_widget(b, r);
        inner
    } else {
        framed(f, t, r, &title, true)
    };
    let w = inner.width as usize;
    let target_style = if prod { Style::default().fg(t.prod).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
    f.buffer_mut().set_stringn(inner.x + 1, inner.y, format!("{} · {}", app.project, app.env), w.saturating_sub(12), target_style);
    if n > list_h {
        let pos = format!("{}/{n}", form.idx + 1);
        f.buffer_mut().set_string(inner.right().saturating_sub(pos.len() as u16 + 1), inner.y, pos, dim());
    }
    // keep the focused field in view without jumping around
    let mut top = form.scroll.get().min(n - list_h);
    if form.idx < top {
        top = form.idx;
    } else if form.idx >= top + list_h {
        top = form.idx + 1 - list_h;
    }
    form.scroll.set(top);
    let nw = form.fields.iter().map(|x| x.name.width()).max().unwrap_or(4).min(24) as u16;
    let tw = 20u16.min(inner.width / 4);
    let vx = inner.x + 4 + nw + 1;
    let vw = inner.width.saturating_sub(vx - inner.x + tw + 2) as usize;
    for (row, (i, fld)) in form.fields.iter().enumerate().skip(top).take(list_h).enumerate() {
        let y = inner.y + 1 + row as u16;
        let focused = i == form.idx;
        let name_style = if focused { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default() };
        if fld.required() {
            f.buffer_mut().set_string(inner.x + 1, y, "*", Style::default().fg(t.error));
        }
        f.buffer_mut().set_string(inner.x + 3, y, pad(&trunc(&fld.name, nw as usize, "…"), nw as usize), name_style);
        // empty fields show what leaving them empty will do
        let placeholder = match fld.blank {
            Blank::Null => Some(("NULL".to_string(), Style::default().fg(t.accent).add_modifier(Modifier::ITALIC))),
            Blank::EmptyString => Some(("'' (empty string)".to_string(), Style::default().fg(t.accent).add_modifier(Modifier::ITALIC))),
            Blank::Default => match (&fld.default, fld.nullable) {
                (Some(d), _) => Some((d.clone(), dim().add_modifier(Modifier::ITALIC))),
                (None, true) => Some(("NULL".to_string(), dim().add_modifier(Modifier::ITALIC))),
                (None, false) => Some(("required".to_string(), Style::default().fg(t.error).add_modifier(Modifier::ITALIC))),
            },
        };
        if fld.input.text.is_empty() {
            if let Some((text, st)) = placeholder {
                f.buffer_mut().set_stringn(vx, y, trunc(&text, vw, "…"), vw, st);
            }
        } else {
            let (spans, _) = input_spans(&fld.input, false, vw, Style::default());
            f.buffer_mut().set_line(vx, y, &Line::from(spans), vw as u16);
        }
        if focused {
            let (_, cx) = input_spans(&fld.input, false, vw, Style::default());
            f.set_cursor_position((vx + cx, y));
        }
        let mut tag = fld.type_name.clone();
        if fld.pk {
            tag.push_str(" · pk");
        }
        if let Some(fk) = &fld.fk {
            tag = format!("→ {fk}");
        }
        let tag = trunc(&tag, tw as usize, "…");
        f.buffer_mut().set_string(inner.right().saturating_sub(tag.width() as u16 + 1), y, &tag, dim());
    }
    // the statement that will run, so there are no surprises
    let sql = form.statement();
    let preview_y = inner.y + 2 + list_h as u16;
    for (k, line) in super::wrap_width(&sql, w.saturating_sub(3)).iter().take(2).enumerate() {
        f.buffer_mut().set_stringn(inner.x + 2, preview_y + k as u16, line, w.saturating_sub(2), Style::default().fg(t.keyword));
    }
    if let Some(e) = &form.error {
        f.buffer_mut().set_stringn(inner.x + 1, preview_y + 2, format!("{} {e}", app.glyphs.cross), w.saturating_sub(1), Style::default().fg(t.error));
    }
    footer(f, inner, &format!("tab next · ^N null · ^S insert · {} insert on last field · esc cancel", app.glyphs.enter));
}

fn form(f: &mut Frame, app: &App, area: Rect, c: &ConnForm) {
    let t = &app.theme;
    let vis = c.visible();
    let h = vis.len() as u16 + 6 + c.error.is_some() as u16;
    let r = centered(area, 78, h);
    let title = if c.replacing.is_some() { "edit connection" } else { "new connection" };
    let inner = framed(f, t, r, title, true);
    let lw = 15u16;
    let mut y = inner.y + 1;
    for &fid in &vis {
        if y >= inner.bottom().saturating_sub(1) {
            break;
        }
        let focused = c.focus == fid;
        let label_style = if focused { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default() };
        match fid {
            ADVANCED => {
                let mark = if c.advanced { "▾" } else { "▸" };
                let mark = if app.settings.ascii { if c.advanced { "v" } else { ">" } } else { mark };
                f.buffer_mut().set_string(inner.x + 1, y, format!("{mark} advanced"), if focused { label_style } else { dim() });
            }
            READONLY => {
                let ro = c.effective_read_only();
                let auto = if c.read_only.is_none() { " (from environment)" } else { "" };
                f.buffer_mut().set_string(inner.x + 1, y, pad("read-only", lw as usize), label_style);
                f.buffer_mut().set_string(inner.x + 1 + lw, y, format!("[{}]{auto}", if ro { "x" } else { " " }), Style::default());
            }
            TEST => {
                y += 1;
                let st = if focused { Style::default().bg(t.selection).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
                f.buffer_mut().set_string(inner.x + 1, y, " test ", st);
                match &app.test_result {
                    Some(Ok(v)) => {
                        f.buffer_mut().set_stringn(inner.x + 9, y, format!("{} {v}", app.glyphs.check), inner.width as usize - 9, Style::default().fg(t.accent));
                    }
                    Some(Err(e)) => {
                        f.buffer_mut().set_stringn(inner.x + 9, y, format!("{} {e}", app.glyphs.cross), inner.width as usize - 9, Style::default().fg(t.error));
                    }
                    None => {}
                }
            }
            SAVE => {
                let st = if focused { Style::default().bg(t.selection).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
                f.buffer_mut().set_string(inner.x + 1, y, " save ", st);
            }
            i if i < FIELD_COUNT => {
                f.buffer_mut().set_string(inner.x + 1, y, pad(FIELD_LABELS[i], lw as usize), label_style);
                let w = (inner.width - lw - 2) as usize;
                let li = &c.fields[i];
                let masked_url;
                let li = if i == URL {
                    masked_url = LineInput { text: mask_url_password(&li.text), cursor: li.cursor };
                    &masked_url
                } else {
                    li
                };
                let (spans, cx) = input_spans(li, i == PASSWORD, w, Style::default());
                f.buffer_mut().set_line(inner.x + 1 + lw, y, &Line::from(spans), w as u16);
                if i == ENV && !li.text.trim().is_empty() {
                    let l = c.level();
                    let x = inner.x + 1 + lw + li.text.width() as u16 + 1;
                    if x + 10 < inner.right() {
                        f.buffer_mut().set_string(x, y, format!("{} {}", app.glyphs.dot, l.label()), Style::default().fg(t.env(l)));
                    }
                }
                if li.text.is_empty() && !focused {
                    let ph = match i {
                        URL => "postgres://user@host:5432/db or mysql://…",
                        PORT => "default",
                        PASSWORD => "stored in the OS keychain",
                        SSH => "user@bastion (optional)",
                        SSL => "prefer",
                        _ => "",
                    };
                    f.buffer_mut().set_string(inner.x + 1 + lw, y, ph, dim().add_modifier(Modifier::ITALIC));
                }
                if focused {
                    if let Some(s) = c.suggestion() {
                        let x = inner.x + 1 + lw + cx;
                        let rest: String = s.chars().skip(li.text.chars().count()).collect();
                        if !rest.is_empty() {
                            f.buffer_mut().set_stringn(x, y, &rest, inner.right().saturating_sub(x) as usize, dim());
                        }
                    }
                    f.set_cursor_position((inner.x + 1 + lw + cx, y));
                }
            }
            _ => {}
        }
        y += 1;
    }
    if let Some(e) = &c.error
        && y < inner.bottom().saturating_sub(1) {
            f.buffer_mut().set_stringn(inner.x + 1, y, e, inner.width as usize - 1, Style::default().fg(t.error));
        }
    footer(f, inner, &format!("tab/↑↓ move · {} on a button · ^S save · esc", app.glyphs.enter));
}

fn review(f: &mut Frame, app: &App, area: Rect, rv: &Review) {
    let t = &app.theme;
    let lines: Vec<&str> = rv.sql.iter().flat_map(|s| s.lines()).collect();
    let r = centered(area, 100, (lines.len() as u16 + 6).min(area.height.saturating_sub(2)));
    let prod = app.level() == Level::Prod;
    let title = format!("review {} change{} · {} · {}", rv.sql.len(), if rv.sql.len() == 1 { "" } else { "s" }, app.project, app.env);
    let inner = framed(f, t, r, &title, true);
    let body_h = inner.height.saturating_sub(3) as usize;
    let scroll = rv.scroll.min(lines.len().saturating_sub(body_h));
    for (i, l) in lines.iter().skip(scroll).take(body_h).enumerate() {
        f.buffer_mut().set_stringn(inner.x + 1, inner.y + 1 + i as u16, l, inner.width as usize - 1, Style::default().fg(t.keyword));
    }
    let foot = if prod {
        format!("{} commit in one transaction (prod: you'll type the env name) · esc back", app.glyphs.enter)
    } else {
        format!("{} commit in one transaction · y copy SQL · esc back", app.glyphs.enter)
    };
    footer(f, inner, &foot);
}

/// Hide the password in `scheme://user:password@host`, keeping the character count.
pub fn mask_url_password(url: &str) -> String {
    let Some(start) = url.find("://").map(|i| i + 3) else { return url.to_string() };
    let rest = &url[start..];
    let Some(at) = rest.rfind('@') else { return url.to_string() };
    let Some(colon) = rest[..at].find(':') else { return url.to_string() };
    let pw = &rest[colon + 1..at];
    format!("{}{}{}", &url[..start + colon + 1], "•".repeat(pw.chars().count()), &rest[at..])
}

#[cfg(test)]
mod tests {
    #[test]
    fn masks_url_password() {
        assert_eq!(super::mask_url_password("mysql://app:secret@h:1/db"), "mysql://app:••••••@h:1/db");
        assert_eq!(super::mask_url_password("postgres://app@h/db"), "postgres://app@h/db");
        assert_eq!(super::mask_url_password("postgres://h/db"), "postgres://h/db");
    }
}
