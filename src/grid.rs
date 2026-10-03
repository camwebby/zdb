//! Result data (column-wise) and the grid's view state: cursor, scroll, sort,
//! selection, find, widths and pending edits. Rendering lives in ui::grid.

use crate::db::{Cell, ColKind, ColumnMeta};
use std::collections::BTreeMap;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default)]
pub struct ResultSet {
    pub cols: Vec<ColumnMeta>,
    /// data[col][row]
    pub data: Vec<Vec<Cell>>,
    pub rows: usize,
    /// More rows existed than the cap; only `rows` were kept.
    pub limited: bool,
    pub complete: bool,
    /// Header suffixes from the schema: (is primary key, foreign key target)
    pub keys: Vec<(bool, Option<(String, String)>)>,
}

impl ResultSet {
    pub fn new(cols: Vec<ColumnMeta>) -> ResultSet {
        let n = cols.len();
        ResultSet { cols, data: vec![Vec::new(); n], rows: 0, limited: false, complete: false, keys: vec![(false, None); n] }
    }
    pub fn push(&mut self, row: Vec<Cell>) {
        for (c, v) in self.data.iter_mut().zip(row.into_iter().chain(std::iter::repeat(None))) {
            c.push(v);
        }
        self.rows += 1;
    }
    pub fn get(&self, row: usize, col: usize) -> Option<&str> {
        self.data.get(col)?.get(row)?.as_deref()
    }
    /// Remove rows past `cap`, marking the result as limited.
    pub fn truncate(&mut self, cap: usize) {
        if self.rows > cap {
            for c in &mut self.data {
                c.truncate(cap);
            }
            self.rows = cap;
            self.limited = true;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelKind {
    Cells,
    Rows,
    Cols,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub kind: SelKind,
}

#[derive(Debug, Clone, Default)]
pub struct Find {
    pub query: String,
    pub matches: Vec<(usize, usize)>,
    pub idx: usize,
}

/// Where the last render put things, for mouse hit-testing.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub header_y: u16,
    pub body_y: u16,
    pub body_rows: u16,
    /// (col index, x, width)
    pub cols: Vec<(usize, u16, u16)>,
}

#[derive(Debug, Clone, Default)]
pub struct GridState {
    /// Display row (index into `order` if sorted).
    pub row: usize,
    pub col: usize,
    pub top: usize,
    /// First scrollable column (columns before `pinned` are always drawn).
    pub left: usize,
    pub pinned: usize,
    pub sort: Vec<(usize, bool)>,
    pub order: Option<Vec<u32>>,
    pub sel: Option<Selection>,
    pub widths: Vec<u16>,
    pub find: Option<Find>,
    /// Pending edits keyed by (data row, col).
    pub edits: BTreeMap<(usize, usize), Option<String>>,
    pub layout: Layout,
    /// Cursor position saved by `g` so `gd` follows the key under the original cell.
    pub before_g: Option<(usize, usize)>,
}

pub const MAX_WIDTH: u16 = 40;

impl GridState {
    pub fn data_row(&self, display: usize) -> usize {
        match &self.order {
            Some(o) => o.get(display).map(|v| *v as usize).unwrap_or(display),
            None => display,
        }
    }

    pub fn fit_widths(&mut self, rs: &ResultSet, group: bool, sample: usize) {
        self.widths = (0..rs.cols.len()).map(|c| fit_width(rs, c, group, sample, MAX_WIDTH)).collect();
        if self.pinned == 0 && rs.cols.len() > 1 {
            self.pinned = 1;
        }
        if let Some(pk) = rs.keys.iter().position(|k| k.0)
            && pk == 0 {
                self.pinned = 1;
            }
    }

    pub fn width(&self, c: usize) -> u16 {
        self.widths.get(c).copied().unwrap_or(10)
    }

    pub fn clamp(&mut self, rs: &ResultSet) {
        if rs.rows == 0 {
            self.row = 0;
        } else {
            self.row = self.row.min(rs.rows - 1);
        }
        if rs.cols.is_empty() {
            self.col = 0;
        } else {
            self.col = self.col.min(rs.cols.len() - 1);
        }
        self.pinned = self.pinned.min(rs.cols.len());
        self.left = self.left.max(self.pinned).min(rs.cols.len().saturating_sub(1).max(self.pinned));
    }

    pub fn move_by(&mut self, rs: &ResultSet, dr: isize, dc: isize) {
        self.row = (self.row as isize + dr).max(0) as usize;
        self.col = (self.col as isize + dc).max(0) as usize;
        self.clamp(rs);
    }

    /// Scroll so the cursor is visible given the body height and available width.
    pub fn ensure_visible(&mut self, rows_visible: usize, width: u16) {
        if rows_visible > 0 {
            if self.row < self.top {
                self.top = self.row;
            } else if self.row >= self.top + rows_visible {
                self.top = self.row + 1 - rows_visible;
            }
        }
        if self.col >= self.pinned {
            if self.col < self.left {
                self.left = self.col;
            }
            // advance left until the cursor column fits
            loop {
                let pinned_w: u16 = (0..self.pinned).map(|c| self.width(c) + 2).sum();
                let mut x = pinned_w;
                let mut fits = false;
                for c in self.left..=self.col {
                    x += self.width(c) + 2;
                }
                if x <= width + 2 || self.left >= self.col {
                    fits = true;
                }
                if fits {
                    break;
                }
                self.left += 1;
            }
        }
    }

    pub fn sort_cycle(&mut self, rs: &ResultSet, col: usize, add: bool) {
        let existing = self.sort.iter().position(|(c, _)| *c == col);
        match (existing, add) {
            (Some(i), _) => {
                if !self.sort[i].1 {
                    self.sort[i].1 = true;
                } else {
                    self.sort.remove(i);
                }
            }
            (None, true) => self.sort.push((col, false)),
            (None, false) => self.sort = vec![(col, false)],
        }
        if !add {
            self.sort.retain(|(c, _)| *c == col);
        }
        self.apply_local_sort(rs);
    }

    pub fn apply_local_sort(&mut self, rs: &ResultSet) {
        if self.sort.is_empty() {
            self.order = None;
            return;
        }
        let mut idx: Vec<u32> = (0..rs.rows as u32).collect();
        let keys = self.sort.clone();
        idx.sort_by(|a, b| {
            for (c, desc) in &keys {
                let o = compare(rs.cols[*c].kind, rs.get(*a as usize, *c), rs.get(*b as usize, *c), *desc);
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            std::cmp::Ordering::Equal
        });
        self.order = Some(idx);
    }

    /// Display rows and columns covered by the selection (or the cursor cell).
    pub fn selected(&self, rs: &ResultSet) -> Option<(std::ops::RangeInclusive<usize>, std::ops::RangeInclusive<usize>)> {
        let sel = self.sel?;
        let (ar, ac) = sel.anchor;
        let rows = ar.min(self.row)..=ar.max(self.row);
        let cols = ac.min(self.col)..=ac.max(self.col);
        let all_rows = 0..=rs.rows.saturating_sub(1);
        let all_cols = 0..=rs.cols.len().saturating_sub(1);
        Some(match sel.kind {
            SelKind::Cells => (rows, cols),
            SelKind::Rows => (rows, all_cols),
            SelKind::Cols => (all_rows, cols),
        })
    }

    pub fn run_find(&mut self, rs: &ResultSet, query: &str) {
        let q = query.to_lowercase();
        let mut matches = Vec::new();
        if !q.is_empty() {
            for r in 0..rs.rows {
                let dr = self.data_row(r);
                for c in 0..rs.cols.len() {
                    if rs.get(dr, c).is_some_and(|v| v.to_lowercase().contains(&q)) {
                        matches.push((r, c));
                    }
                }
            }
        }
        let idx = matches.iter().position(|&(r, c)| (r, c) >= (self.row, self.col)).unwrap_or(0);
        if let Some(&(r, c)) = matches.get(idx) {
            self.row = r;
            self.col = c;
        }
        self.find = Some(Find { query: query.to_string(), matches, idx });
    }

    pub fn find_step(&mut self, forward: bool) -> bool {
        let Some(f) = &mut self.find else { return false };
        if f.matches.is_empty() {
            return false;
        }
        let n = f.matches.len();
        f.idx = if forward { (f.idx + 1) % n } else { (f.idx + n - 1) % n };
        let (r, c) = f.matches[f.idx];
        self.row = r;
        self.col = c;
        true
    }

    pub fn cell_value<'a>(&'a self, rs: &'a ResultSet, display_row: usize, col: usize) -> Option<&'a str> {
        let dr = self.data_row(display_row);
        if let Some(v) = self.edits.get(&(dr, col)) {
            return v.as_deref();
        }
        rs.get(dr, col)
    }
}

pub fn compare(kind: ColKind, a: Option<&str>, b: Option<&str>, desc: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let o = match (a, b) {
        (None, None) => Equal,
        // nulls last ascending, first descending (Postgres default)
        (None, _) => Greater,
        (_, None) => Less,
        (Some(x), Some(y)) => {
            if kind.numeric() {
                match (x.parse::<f64>(), y.parse::<f64>()) {
                    (Ok(p), Ok(q)) => p.partial_cmp(&q).unwrap_or(Equal),
                    _ => x.cmp(y),
                }
            } else {
                x.cmp(y)
            }
        }
    };
    if desc { o.reverse() } else { o }
}

/// Display text for a cell (no NULL handling: callers draw `null` themselves).
pub fn display(kind: ColKind, v: &str, group: bool) -> String {
    let one_line = |s: &str| -> String {
        if s.contains(['\n', '\r', '\t']) { s.replace(['\n', '\r'], "↵").replace('\t', " ") } else { s.to_string() }
    };
    match kind {
        ColKind::Decimal | ColKind::Float if group => group_digits(v),
        _ => one_line(v),
    }
}

pub fn group_digits(v: &str) -> String {
    let (sign, rest) = match v.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", v),
    };
    let (int, frac) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    if !int.bytes().all(|b| b.is_ascii_digit()) || int.len() <= 3 {
        return v.to_string();
    }
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    match frac {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

pub fn fit_width(rs: &ResultSet, c: usize, group: bool, sample: usize, cap: u16) -> u16 {
    let meta = &rs.cols[c];
    let mut w = meta.name.width() + 2; // room for sort arrow / pk suffix
    if rs.keys.get(c).is_some_and(|k| k.0 || k.1.is_some()) {
        w += 3;
    }
    w = w.max(meta.type_name.width()).max(4);
    for r in 0..rs.rows.min(sample) {
        let vw = match rs.get(r, c) {
            Some(v) => display(meta.kind, v, group).width(),
            None => 4,
        };
        w = w.max(vw);
    }
    (w as u16).min(cap).max(3)
}

/// Summary for the status line: count, and sum/avg/min/max when every value is numeric.
pub fn selection_stats(values: &[Option<&str>], numeric: bool) -> String {
    let n = values.len();
    let label = if n == 1 { "cell" } else { "cells" };
    if !numeric {
        return format!("{n} {label}");
    }
    let nums: Vec<f64> = values.iter().filter_map(|v| v.and_then(|s| s.parse::<f64>().ok())).collect();
    if nums.is_empty() {
        return format!("{n} {label}");
    }
    let scale = values
        .iter()
        .filter_map(|v| v.and_then(|s| s.split_once('.').map(|(_, f)| f.len())))
        .max()
        .unwrap_or(0)
        .min(6);
    let sum: f64 = nums.iter().sum();
    let min = nums.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let f = |x: f64, s: usize| group_digits(&format!("{x:.s$}"));
    if n == 1 {
        return format!("1 cell · {}", f(sum, scale));
    }
    format!(
        "{n} {label} · sum {} · avg {} · min {} · max {}",
        f(sum, scale),
        f(sum / nums.len() as f64, scale.max(2)),
        f(min, scale),
        f(max, scale)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rs() -> ResultSet {
        let cols = vec![
            ColumnMeta { name: "id".into(), type_name: "int8".into(), kind: ColKind::Int },
            ColumnMeta { name: "name".into(), type_name: "text".into(), kind: ColKind::Text },
        ];
        let mut rs = ResultSet::new(cols);
        rs.push(vec![Some("10".into()), Some("b".into())]);
        rs.push(vec![Some("9".into()), None]);
        rs.push(vec![Some("100".into()), Some("a".into())]);
        rs
    }

    #[test]
    fn numeric_sort_and_nulls() {
        let rs = rs();
        let mut g = GridState::default();
        g.sort_cycle(&rs, 0, false);
        assert_eq!(g.order.as_deref(), Some(&[1u32, 0, 2][..]));
        g.sort_cycle(&rs, 0, false);
        assert_eq!(g.order.as_deref(), Some(&[2u32, 0, 1][..]));
        g.sort_cycle(&rs, 0, false);
        assert!(g.order.is_none());
        g.sort_cycle(&rs, 1, false);
        assert_eq!(g.order.as_deref(), Some(&[2u32, 0, 1][..])); // null last
    }

    #[test]
    fn grouping_and_stats() {
        assert_eq!(group_digits("18402.50"), "18,402.50");
        assert_eq!(group_digits("-1234567"), "-1,234,567");
        assert_eq!(group_digits("412"), "412");
        assert_eq!(group_digits("abc"), "abc");
        let v = [Some("18402.50"), Some("17950.00"), Some("16210.75")];
        assert_eq!(
            selection_stats(&v, true),
            "3 cells · sum 52,563.25 · avg 17,521.08 · min 16,210.75 · max 18,402.50"
        );
        assert_eq!(selection_stats(&[Some("x"), None], false), "2 cells");
    }

    #[test]
    fn find_cycles() {
        let rs = rs();
        let mut g = GridState::default();
        g.run_find(&rs, "a");
        assert_eq!((g.row, g.col), (2, 1));
        g.run_find(&rs, "1");
        assert_eq!(g.find.as_ref().unwrap().matches.len(), 2);
        assert!(g.find_step(true));
    }

    #[test]
    fn truncate_marks_limited() {
        let mut r = rs();
        r.truncate(2);
        assert!(r.limited);
        assert_eq!(r.rows, 2);
        assert_eq!(r.data[0].len(), 2);
    }
}
