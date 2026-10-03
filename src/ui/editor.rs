//! The SQL editor: line numbers, a bar beside the statement that will run,
//! syntax highlighting, selection, the error underline and the completion popup.

use super::{trunc, wrap_width};
use crate::app::{App, Focus};
use crate::sql::{FUNCTIONS, Tok, is_keyword, tokenize};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn draw(f: &mut Frame, app: &mut App, r: Rect) {
    if r.height == 0 || r.width < 6 {
        return;
    }
    let theme = app.theme.clone();
    let glyphs = app.glyphs.clone();
    let blank_splits = app.settings.blank_line_splits;
    let focused = app.focus == Focus::Editor && app.overlay.is_none();
    let has_overlay = app.overlay.is_some();
    let error = app.tab().error.clone();
    // reserve the last line(s) for the error message
    let err_lines: Vec<String> = match &error {
        Some(e) if r.height >= 4 => wrap_width(&e.message, r.width.saturating_sub(4) as usize).into_iter().take(2).collect(),
        _ => vec![],
    };
    let text_h = r.height - err_lines.len() as u16;

    let tab = app.tab();
    let text = tab.editor.text();
    let stmts = tab.statements(blank_splits);
    let cur_off = tab.editor.cursor_offset();
    let sel_text = tab.editor.selection();
    let current = if sel_text.is_some() { None } else { crate::sql::stmt_at(&stmts.2, cur_off).map(|i| stmts.2[i].range.clone()) };
    drop(stmts);

    let nlines = tab.editor.lines.len();
    let gutter_digits = nlines.to_string().len().max(2) as u16;
    let gutter = gutter_digits + 2; // digits, bar, space
    app.layout.gutter = gutter;
    app.layout.editor = r;

    // keep the cursor visible
    let tab = &mut app.tabs[app.cur];
    let ed = &mut tab.editor;
    let (crow, ccol) = ed.cursor();
    let h = text_h as usize;
    if crow < ed.scroll {
        ed.scroll = crow;
    } else if h > 0 && crow >= ed.scroll + h {
        ed.scroll = crow + 1 - h;
    }
    let text_w = r.width.saturating_sub(gutter) as usize;
    let cursor_x: usize = ed.lines[crow].chars().take(ccol).map(|c| c.width().unwrap_or(0)).sum();
    if cursor_x < ed.hscroll {
        ed.hscroll = cursor_x.saturating_sub(4);
    } else if text_w > 1 && cursor_x >= ed.hscroll + text_w - 1 {
        ed.hscroll = cursor_x + 2 - text_w;
    }
    let ed = &app.tabs[app.cur].editor;
    let (scroll, hscroll) = (ed.scroll, ed.hscroll);

    // line start byte offsets
    let mut starts = Vec::with_capacity(nlines);
    let mut off = 0;
    for l in &ed.lines {
        starts.push(off);
        off += l.len() + 1;
    }
    let vis_end = (scroll + h).min(nlines);
    let vis_range = if scroll < nlines { starts[scroll]..(if vis_end < nlines { starts[vis_end] } else { text.len() }) } else { 0..0 };

    // style per byte in the visible range
    let mut styles: Vec<Style> = vec![Style::default(); vis_range.len()];
    let toks = tokenize(&text);
    for (ti, tk) in toks.iter().enumerate() {
        if tk.range.end <= vis_range.start || tk.range.start >= vis_range.end {
            continue;
        }
        let word = tk.text(&text);
        let st = match tk.kind {
            Tok::Word => {
                let next_paren = toks[ti + 1..].iter().find(|t| t.kind != Tok::Space).is_some_and(|t| t.text(&text) == "(");
                let lw = word.to_ascii_lowercase();
                if next_paren && FUNCTIONS.contains(&lw.as_str()) {
                    Style::default().fg(theme.function)
                } else if is_keyword(word) {
                    Style::default().fg(theme.keyword)
                } else {
                    Style::default()
                }
            }
            Tok::Str => Style::default().fg(theme.string),
            Tok::Number => Style::default().fg(theme.number),
            Tok::Comment => Style::default().fg(theme.comment).add_modifier(Modifier::ITALIC),
            Tok::Param => Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            _ => Style::default(),
        };
        let a = tk.range.start.max(vis_range.start) - vis_range.start;
        let b = tk.range.end.min(vis_range.end) - vis_range.start;
        for s in &mut styles[a..b] {
            *s = st;
        }
    }

    let buf = f.buffer_mut();
    let num_style = Style::default().add_modifier(Modifier::DIM);
    for i in 0..h {
        let row = scroll + i;
        let y = r.y + i as u16;
        if row >= nlines {
            if row == nlines && nlines == 1 && ed.lines[0].is_empty() && !has_overlay {
                // nothing typed yet: placeholder lives on line 1, drawn below
            }
            continue;
        }
        let line = &ed.lines[row];
        let lstart = starts[row];
        let in_stmt = current.as_ref().is_some_and(|c| c.start <= lstart + line.len() && c.end > lstart || (c.start >= lstart && c.start <= lstart + line.len() && !line.trim().is_empty()));
        let num = format!("{:>w$}", row + 1, w = gutter_digits as usize);
        let ns = if row == crow && focused { Style::default() } else { num_style };
        buf.set_string(r.x, y, &num, ns);
        if in_stmt {
            buf.set_string(r.x + gutter_digits, y, glyphs.bar, Style::default().fg(theme.accent));
        }
        // characters
        let mut x_w = 0usize; // display column within the line
        let mut boff = 0usize;
        for (ci, ch) in line.chars().enumerate() {
            let cw = ch.width().unwrap_or(0);
            let byte = lstart + boff;
            boff += ch.len_utf8();
            if x_w + cw <= hscroll {
                x_w += cw;
                continue;
            }
            let col_x = x_w - hscroll;
            if col_x + cw > text_w {
                break;
            }
            let mut st = if byte >= vis_range.start && byte < vis_range.end { styles[byte - vis_range.start] } else { Style::default() };
            if let Some(((sr, sc), (er, ec))) = sel_text {
                let after_start = (row, ci) >= (sr, sc);
                let before_end = (row, ci) < (er, ec);
                if after_start && before_end {
                    st = st.bg(theme.selection);
                }
            }
            if let Some(e) = &error
                && e.line == row && ci >= e.col && ci < e.col + e.len {
                    st = st.fg(theme.error).add_modifier(Modifier::UNDERLINED);
                }
            let s = if ch == '\t' { " ".to_string() } else { ch.to_string() };
            buf.set_string(r.x + gutter + col_x as u16, y, &s, st);
            x_w += cw;
        }
        // selection across line ends
        if let Some(((sr, _), (er, _))) = sel_text
            && row >= sr && row < er {
                let lw = line.width();
                if lw >= hscroll && lw - hscroll < text_w {
                    buf.set_string(r.x + gutter + (lw - hscroll) as u16, y, " ", Style::default().bg(theme.selection));
                }
            }
        if let Some(e) = &error
            && e.line == row && e.col >= line.chars().count() {
                let lw = line.width();
                if lw >= hscroll && lw - hscroll < text_w {
                    buf.set_string(r.x + gutter + (lw - hscroll) as u16, y, "_", Style::default().fg(theme.error));
                }
            }
    }
    // placeholder
    if nlines == 1 && ed.lines[0].is_empty() {
        let km = &app.keymap;
        let hint = format!(
            "write SQL · {} runs the statement · {} palette · {} tables",
            km.hint(crate::keys::Action::RunStatement),
            km.hint(crate::keys::Action::Palette),
            km.hint(crate::keys::Action::GoToTable)
        );
        let hint = trunc(&hint, text_w.saturating_sub(1), glyphs.ellipsis);
        f.buffer_mut().set_string(r.x + gutter + 1, r.y, hint, Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC));
    }
    // error message
    for (i, l) in err_lines.iter().enumerate() {
        let y = r.y + text_h + i as u16;
        let prefix = if i == 0 { format!("{} ", glyphs.cross) } else { "  ".into() };
        f.buffer_mut().set_string(r.x + gutter, y, format!("{prefix}{l}"), Style::default().fg(theme.error));
    }

    // cursor and completion popup
    let cy = crow as isize - scroll as isize;
    let cx = cursor_x as isize - hscroll as isize;
    if cy >= 0 && (cy as u16) < text_h && cx >= 0 && (cx as usize) < text_w {
        let pos = (r.x + gutter + cx as u16, r.y + cy as u16);
        if focused {
            let vim_block = ed.vim_mode().is_some_and(|m| m != crate::editor::VimMode::Insert);
            if vim_block {
                let cell = f.buffer_mut().cell_mut(pos);
                if let Some(c) = cell {
                    c.set_style(Style::default().add_modifier(Modifier::REVERSED));
                }
            } else {
                f.set_cursor_position(pos);
            }
            if let Some(c) = &app.complete {
                draw_completion(f, app, c, pos, r);
            }
        }
    }
}

fn draw_completion(f: &mut Frame, app: &App, c: &crate::app::Completion, pos: (u16, u16), area: Rect) {
    let t = &app.theme;
    let n = c.items.len().min(8);
    if n == 0 {
        return;
    }
    let (word, _) = app.tab().editor.word_before_cursor();
    let lw = c.items.iter().map(|(s, _)| s.width()).max().unwrap_or(4).min(40);
    let kw = c.items.iter().map(|(_, k)| k.len()).max().unwrap_or(0);
    let w = (lw + kw + 3) as u16;
    let full = f.area();
    let x = pos.0.saturating_sub(word.width() as u16).min(full.right().saturating_sub(w));
    let below = pos.1 + 1 + n as u16 <= full.bottom().saturating_sub(1);
    let y = if below { pos.1 + 1 } else { pos.1.saturating_sub(n as u16) };
    let _ = area;
    let start = c.sel.saturating_sub(n - 1);
    let buf = f.buffer_mut();
    for (i, (label, kind)) in c.items.iter().enumerate().skip(start).take(n) {
        let yy = y + (i - start) as u16;
        let sel = i == c.sel;
        let base = if sel { Style::default().bg(t.selection).add_modifier(Modifier::BOLD) } else { Style::default().bg(t.row) };
        let text = format!(" {} {} ", super::pad(&trunc(label, lw, "…"), lw), super::pad(kind, kw));
        buf.set_stringn(x, yy, &text, w as usize, base);
        buf.set_stringn(x + lw as u16 + 2, yy, super::pad(kind, kw), kw, base.add_modifier(Modifier::DIM));
    }
}
