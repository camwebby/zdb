//! Result grid, EXPLAIN plan tree and plain text views.

use super::{pad, pad_left, trunc};
use crate::app::{ResultBody, ResultView};
use crate::theme::{Glyphs, Theme};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

pub struct Ctx<'a> {
    pub theme: &'a Theme,
    pub glyphs: &'a Glyphs,
    pub group: bool,
    pub focused: bool,
    pub stale: bool,
    pub comfortable: bool,
}

pub fn draw(f: &mut Frame, r: Rect, v: &mut ResultView, cx: &Ctx) {
    let t = cx.theme;
    let g = cx.glyphs;
    let rs = &v.rs;
    let gs = &mut v.grid;
    if rs.cols.is_empty() {
        if !v.done {
            return;
        }
        f.buffer_mut().set_stringn(r.x + 1, r.y, "no columns", r.width as usize, Style::default().add_modifier(Modifier::DIM));
        return;
    }
    let gap: u16 = if cx.comfortable { 3 } else { 2 };
    let header_rows: u16 = if r.height >= 6 { 2 } else { 1 };
    let body_rows = r.height.saturating_sub(header_rows);
    gs.clamp(rs);
    gs.ensure_visible(body_rows as usize, r.width.saturating_sub(1));
    let dim = |s: Style| if cx.stale { s.add_modifier(Modifier::DIM) } else { s };

    // which columns are visible
    let mut cols: Vec<(usize, u16, u16)> = Vec::new();
    let mut x = r.x + 1;
    let order: Vec<usize> = (0..gs.pinned.min(rs.cols.len())).chain(gs.left.max(gs.pinned)..rs.cols.len()).collect();
    for c in order {
        if x >= r.right() {
            break;
        }
        let w = gs.width(c).min(r.right() - x);
        cols.push((c, x, w));
        x += w + gap;
    }
    let multi = gs.sort.len() > 1;
    let buf = f.buffer_mut();
    for &(c, x, w) in &cols {
        let meta = &rs.cols[c];
        let mut suffix = String::new();
        if let Some(i) = gs.sort.iter().position(|(sc, _)| *sc == c) {
            suffix.push_str(if gs.sort[i].1 { g.down } else { g.up });
            if multi {
                suffix.push_str(&(i + 1).to_string());
            }
        }
        let mut key = match rs.keys.get(c) {
            Some((true, _)) => " pk",
            Some((false, Some(_))) => if g.arrow == "->" { " ->" } else { " →" },
            _ => "",
        };
        // the name matters more than the badge
        if meta.name.width() + key.width() + suffix.width() > w as usize {
            key = "";
        }
        let name_w = (w as usize).saturating_sub(suffix.width() + key.width());
        let name = trunc(&meta.name, name_w.max(1), g.ellipsis);
        let mut hx = x;
        let hs = dim(Style::default().add_modifier(Modifier::BOLD));
        let hs = if c == gs.col && cx.focused { hs.fg(t.accent) } else { hs };
        let (nx, _) = buf.set_stringn(hx, r.y, &name, w as usize, hs);
        hx = nx;
        if !key.is_empty() && hx < x + w {
            let (nx, _) = buf.set_stringn(hx, r.y, key, (x + w - hx) as usize, dim(Style::default().add_modifier(Modifier::DIM)));
            hx = nx;
        }
        if !suffix.is_empty() && hx < x + w {
            buf.set_stringn(hx, r.y, &suffix, (x + w - hx) as usize, dim(Style::default().fg(t.accent)));
        }
        if header_rows == 2 {
            buf.set_stringn(x, r.y + 1, trunc(&meta.type_name, w as usize, g.ellipsis), w as usize, Style::default().add_modifier(Modifier::DIM));
        }
    }
    let body_y = r.y + header_rows;
    let sel = gs.selected(rs);
    let finds: std::collections::HashSet<(usize, usize)> = gs.find.as_ref().map(|f| f.matches.iter().copied().filter(|(r, _)| *r >= gs.top && *r < gs.top + body_rows as usize).collect()).unwrap_or_default();
    for i in 0..body_rows as usize {
        let dr = gs.top + i;
        if dr >= rs.rows {
            break;
        }
        let y = body_y + i as u16;
        let data_row = gs.data_row(dr);
        let cur_row = dr == gs.row;
        if cur_row && cx.focused {
            buf.set_style(Rect { x: r.x, y, width: r.width, height: 1 }, Style::default().bg(t.row));
        }
        for &(c, x, w) in &cols {
            let kind = rs.cols[c].kind;
            let edit = gs.edits.get(&(data_row, c));
            let val: Option<&str> = match edit {
                Some(e) => e.as_deref(),
                None => rs.get(data_row, c),
            };
            let mut st = Style::default();
            if cur_row && cx.focused {
                st = st.bg(t.row);
            }
            let text = match val {
                None => {
                    st = st.add_modifier(Modifier::DIM | Modifier::ITALIC);
                    pad("null", w as usize)
                }
                Some(s) => {
                    let d = crate::grid::display(kind, s, cx.group);
                    let d = trunc(&d, w as usize, g.ellipsis);
                    if kind.numeric() { pad_left(&d, w as usize) } else { pad(&d, w as usize) }
                }
            };
            if edit.is_some() {
                st = st.fg(t.pending).add_modifier(Modifier::BOLD);
            }
            if finds.contains(&(dr, c)) {
                st = st.fg(t.pending).add_modifier(Modifier::UNDERLINED);
            }
            if let Some((rr, cc)) = &sel
                && rr.contains(&dr) && cc.contains(&c) {
                    st = st.bg(t.selection);
                }
            if cur_row && c == gs.col {
                st = if cx.focused { st.add_modifier(Modifier::REVERSED) } else { st.add_modifier(Modifier::UNDERLINED) };
            }
            buf.set_stringn(x, y, &text, w as usize, dim(st));
        }
    }
    if rs.rows == 0 && v.done && body_rows > 0 {
        buf.set_stringn(r.x + 1, body_y, "(no rows)", r.width as usize, Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC));
    }
    // scroll position marker on the right edge
    if rs.rows > body_rows as usize && body_rows > 2 {
        let frac = gs.top as f64 / (rs.rows - body_rows as usize).max(1) as f64;
        let y = body_y + ((body_rows - 1) as f64 * frac).round() as u16;
        buf.set_string(r.right() - 1, y, g.bar, Style::default().add_modifier(Modifier::DIM));
    }
    gs.layout = crate::grid::Layout { header_y: r.y, body_y, body_rows, cols };
}

pub fn draw_plan(f: &mut Frame, r: Rect, v: &mut ResultView, cx: &Ctx) {
    let ResultBody::Plan(p) = &v.body else { return };
    let t = cx.theme;
    let g = cx.glyphs;
    let pre = crate::explain::prefixes(&p.nodes, g.arrow == "->");
    let max_self = p.nodes.iter().map(|n| n.self_cost).fold(0.0, f64::max).max(1e-9);
    let bar_w: usize = 12;
    let stat_w: usize = 24;
    let label_w = (r.width as usize).saturating_sub(bar_w + stat_w + 3);
    let mut lines: Vec<Vec<(String, Style)>> = Vec::new();
    for (n, pre) in p.nodes.iter().zip(pre) {
        let frac = n.self_cost / max_self;
        let filled = frac * bar_w as f64;
        let full = filled.floor() as usize;
        let mut bar: String = std::iter::repeat_n(g.block_full, full).collect();
        let rem = filled - full as f64;
        if full < bar_w && rem > 0.05 {
            let idx = ((rem * g.blocks.len() as f64) as usize).min(g.blocks.len() - 1);
            bar.push(g.blocks[idx]);
        }
        let hot = frac > 0.5;
        let bar_style = if hot { Style::default().fg(t.error) } else if frac > 0.2 { Style::default().fg(t.staging) } else { Style::default().fg(t.accent) };
        let stat = if p.analyze {
            format!("{:.2} ms · {} rows", n.self_cost, n.rows.map(fmt_num).unwrap_or_default())
        } else {
            format!("cost {} · {} rows", fmt_num(n.self_cost), n.rows.map(fmt_num).unwrap_or_default())
        };
        let label = trunc(&format!("{pre}{}", n.label), label_w, g.ellipsis);
        lines.push(vec![
            (pad(&label, label_w), if hot { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() }),
            (" ".into(), Style::default()),
            (pad(&bar, bar_w), bar_style),
            (" ".into(), Style::default()),
            (trunc(&stat, stat_w, g.ellipsis), Style::default().add_modifier(Modifier::DIM)),
        ]);
        if let Some(d) = &n.detail {
            // continue the tree lines under the node: its own branch, then room for children
            let stem: String = pre.chars().take(pre.chars().count().saturating_sub(3)).collect();
            let branch = if n.depth == 0 || n.last_child { "   " } else if g.arrow == "->" { "|  " } else { "│  " };
            let branch = if n.depth == 0 { "" } else { branch };
            lines.push(vec![(trunc(&format!("{stem}{branch}  {d}"), r.width as usize, g.ellipsis), Style::default().add_modifier(Modifier::DIM))]);
        }
    }
    if !p.footer.is_empty() {
        lines.push(vec![]);
        lines.push(vec![(p.footer.join(" · "), Style::default().add_modifier(Modifier::DIM))]);
    }
    draw_lines(f, r, &mut v.grid.top, &lines);
}

pub fn draw_text(f: &mut Frame, r: Rect, v: &mut ResultView, cx: &Ctx) {
    let ResultBody::Text(ls) = &v.body else { return };
    let _ = cx;
    let lines: Vec<Vec<(String, Style)>> = ls.iter().map(|l| vec![(l.clone(), Style::default())]).collect();
    draw_lines(f, r, &mut v.grid.top, &lines);
}

fn draw_lines(f: &mut Frame, r: Rect, top: &mut usize, lines: &[Vec<(String, Style)>]) {
    let h = r.height as usize;
    *top = (*top).min(lines.len().saturating_sub(h));
    let buf = f.buffer_mut();
    for (i, l) in lines.iter().skip(*top).take(h).enumerate() {
        let mut x = r.x + 1;
        for (s, st) in l {
            if x >= r.right() {
                break;
            }
            let (nx, _) = buf.set_stringn(x, r.y + i as u16, s, (r.right() - x) as usize, *st);
            x = nx;
        }
    }
}

fn fmt_num(x: f64) -> String {
    if x >= 1e6 {
        format!("{:.1}M", x / 1e6)
    } else if x >= 1e4 {
        format!("{:.0}k", x / 1e3)
    } else if x.fract() == 0.0 {
        crate::grid::group_digits(&format!("{x:.0}"))
    } else {
        format!("{x:.2}")
    }
}
