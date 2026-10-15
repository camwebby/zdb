//! The SQL editor's text model: lines of text, a cursor, an optional selection,
//! undo/redo, and an optional Vim layer. Rendering and highlighting live in ui::editor.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    lines: Vec<String>,
    row: usize,
    col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

#[derive(Debug, Clone, Default)]
pub struct Vim {
    pub mode: Option<VimMode>,
    pending: String,
    register: String,
    register_lines: bool,
}

#[derive(Debug, Clone)]
pub struct Editor {
    pub lines: Vec<String>,
    pub row: usize,
    pub col: usize,
    pub anchor: Option<(usize, usize)>,
    pub scroll: usize,
    pub hscroll: usize,
    /// Rows of text the view shows; the UI keeps this current, and paging goes by it.
    pub view_rows: usize,
    /// What `follow_cursor` last scrolled for, so the view only follows when that changes.
    followed: Option<(usize, usize, u64, usize, usize)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    coalesce: bool,
    pub vim: Option<Vim>,
    /// Bumped on every text change, so caches (statement split, highlighting) can be keyed on it.
    pub generation: u64,
    /// Remembered column for vertical moves.
    goal: Option<usize>,
    clipboard: String,
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new("")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Handled,
    NotHandled,
    /// Vim: Esc in normal mode leaves the editor.
    Leave,
}

fn char_to_byte(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map(|(b, _)| b).unwrap_or(s.len())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    pub fn new(text: &str) -> Editor {
        let mut e = Editor {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            anchor: None,
            scroll: 0,
            hscroll: 0,
            view_rows: 10,
            followed: None,
            undo: Vec::new(),
            redo: Vec::new(),
            coalesce: false,
            vim: None,
            generation: 0,
            goal: None,
            clipboard: String::new(),
        };
        e.set_text(text);
        e.undo.clear();
        e
    }

    pub fn set_vim(&mut self, on: bool) {
        self.vim = if on { Some(Vim { mode: Some(VimMode::Insert), ..Default::default() }) } else { None };
    }

    pub fn vim_mode(&self) -> Option<VimMode> {
        self.vim.as_ref().and_then(|v| v.mode)
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn set_text(&mut self, text: &str) {
        self.snapshot(false);
        self.lines = text.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.row.min(self.lines.len() - 1);
        self.col = self.col.min(self.line_len(self.row));
        self.anchor = None;
        self.changed();
    }

    fn changed(&mut self) {
        self.generation += 1;
        self.redo.clear();
    }

    pub fn line_len(&self, r: usize) -> usize {
        self.lines.get(r).map(|l| l.chars().count()).unwrap_or(0)
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.line_len(self.row));
        self.goal = None;
    }

    /// Byte offset of a (row, col) position in `text()`.
    pub fn offset_of(&self, row: usize, col: usize) -> usize {
        let mut off = 0;
        for l in self.lines.iter().take(row) {
            off += l.len() + 1;
        }
        off + char_to_byte(&self.lines[row.min(self.lines.len() - 1)], col)
    }

    pub fn cursor_offset(&self) -> usize {
        self.offset_of(self.row, self.col)
    }

    pub fn pos_of(&self, mut off: usize) -> (usize, usize) {
        for (r, l) in self.lines.iter().enumerate() {
            if off <= l.len() {
                let off = (0..=off).rev().find(|o| l.is_char_boundary(*o)).unwrap_or(0);
                return (r, l[..off].chars().count());
            }
            off -= l.len() + 1;
        }
        let r = self.lines.len() - 1;
        (r, self.line_len(r))
    }

    fn snapshot(&mut self, coalesce: bool) {
        if coalesce && self.coalesce {
            return;
        }
        self.undo.push(Snapshot { lines: self.lines.clone(), row: self.row, col: self.col });
        if self.undo.len() > 500 {
            self.undo.remove(0);
        }
        self.coalesce = coalesce;
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(Snapshot { lines: std::mem::take(&mut self.lines), row: self.row, col: self.col });
            self.lines = s.lines;
            self.row = s.row;
            self.col = s.col;
            self.anchor = None;
            self.generation += 1;
            self.coalesce = false;
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(Snapshot { lines: std::mem::take(&mut self.lines), row: self.row, col: self.col });
            self.lines = s.lines;
            self.row = s.row;
            self.col = s.col;
            self.anchor = None;
            self.generation += 1;
            self.coalesce = false;
        }
    }

    pub fn selection(&self) -> Option<((usize, usize), (usize, usize))> {
        let a = self.anchor?;
        let b = (self.row, self.col);
        if a == b {
            return None;
        }
        Some(if a < b { (a, b) } else { (b, a) })
    }

    pub fn selected_text(&self) -> Option<String> {
        let (s, e) = self.selection()?;
        let text = self.text();
        Some(text[self.offset_of(s.0, s.1)..self.offset_of(e.0, e.1)].to_string())
    }

    /// Byte range of the selection in `text()`.
    pub fn selection_range(&self) -> Option<std::ops::Range<usize>> {
        let (s, e) = self.selection()?;
        Some(self.offset_of(s.0, s.1)..self.offset_of(e.0, e.1))
    }

    fn delete_range(&mut self, s: (usize, usize), e: (usize, usize)) {
        let text = self.text();
        let a = self.offset_of(s.0, s.1);
        let b = self.offset_of(e.0, e.1);
        let new = format!("{}{}", &text[..a], &text[b..]);
        self.lines = new.split('\n').map(String::from).collect();
        self.row = s.0;
        self.col = s.1;
        self.changed();
    }

    pub fn delete_selection(&mut self) -> bool {
        if let Some((s, e)) = self.selection() {
            self.snapshot(false);
            self.delete_range(s, e);
            self.anchor = None;
            true
        } else {
            self.anchor = None;
            false
        }
    }

    pub fn insert_str(&mut self, s: &str) {
        let coalesce = s.chars().count() == 1 && !s.contains(char::is_whitespace);
        if self.selection().is_some() {
            self.delete_selection();
        } else {
            self.snapshot(coalesce);
        }
        self.anchor = None;
        let s = s.replace("\r\n", "\n").replace('\r', "\n").replace('\t', "    ");
        let line = &self.lines[self.row];
        let b = char_to_byte(line, self.col);
        let tail = line[b..].to_string();
        let head = line[..b].to_string();
        let mut parts = s.split('\n');
        let first = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            self.lines[self.row] = format!("{head}{first}{tail}");
            self.col += first.chars().count();
        } else {
            self.lines[self.row] = format!("{head}{first}");
            let n = rest.len();
            for (i, p) in rest.iter().enumerate() {
                let l = if i == n - 1 { format!("{p}{tail}") } else { p.to_string() };
                self.lines.insert(self.row + 1 + i, l);
            }
            self.row += n;
            self.col = rest[n - 1].chars().count();
        }
        self.goal = None;
        self.changed();
    }

    pub fn newline(&mut self) {
        let indent: String = self.lines[self.row].chars().take_while(|c| *c == ' ').collect();
        let indent: String = indent.chars().take(self.col).collect();
        self.coalesce = false;
        self.insert_str(&format!("\n{indent}"));
        self.coalesce = false;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.col == 0 && self.row == 0 {
            return;
        }
        self.snapshot(true);
        if self.col == 0 {
            let l = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len(self.row);
            self.lines[self.row].push_str(&l);
        } else {
            let line = &mut self.lines[self.row];
            // remove a soft tab of spaces if we're in leading indentation
            let lead = line.chars().take_while(|c| *c == ' ').count();
            let n = if self.col <= lead && self.col >= 4 && self.col.is_multiple_of(4) { 4 } else { 1 };
            let a = char_to_byte(line, self.col - n);
            let b = char_to_byte(line, self.col);
            line.replace_range(a..b, "");
            self.col -= n;
        }
        self.goal = None;
        self.changed();
    }

    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        let len = self.line_len(self.row);
        if self.col >= len {
            if self.row + 1 < self.lines.len() {
                self.snapshot(false);
                let next = self.lines.remove(self.row + 1);
                self.lines[self.row].push_str(&next);
                self.changed();
            }
            return;
        }
        self.snapshot(false);
        let line = &mut self.lines[self.row];
        let a = char_to_byte(line, self.col);
        let b = char_to_byte(line, self.col + 1);
        line.replace_range(a..b, "");
        self.changed();
    }

    fn select_start(&mut self, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some((self.row, self.col));
            }
        } else {
            self.anchor = None;
        }
        self.coalesce = false;
    }

    pub fn left(&mut self, extend: bool) {
        if !extend && let Some((s, _)) = self.selection() {
            self.anchor = None;
            self.row = s.0;
            self.col = s.1;
            return;
        }
        self.select_start(extend);
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line_len(self.row);
        }
        self.goal = None;
    }

    pub fn right(&mut self, extend: bool) {
        if !extend && let Some((_, e)) = self.selection() {
            self.anchor = None;
            self.row = e.0;
            self.col = e.1;
            return;
        }
        self.select_start(extend);
        if self.col < self.line_len(self.row) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
        self.goal = None;
    }

    pub fn up(&mut self, extend: bool) -> bool {
        self.select_start(extend);
        if self.row == 0 {
            self.col = 0;
            return false;
        }
        let g = *self.goal.get_or_insert(self.col);
        self.row -= 1;
        self.col = g.min(self.line_len(self.row));
        true
    }

    pub fn down(&mut self, extend: bool) -> bool {
        self.select_start(extend);
        if self.row + 1 >= self.lines.len() {
            self.col = self.line_len(self.row);
            return false;
        }
        let g = *self.goal.get_or_insert(self.col);
        self.row += 1;
        self.col = g.min(self.line_len(self.row));
        true
    }

    pub fn home(&mut self, extend: bool) {
        self.select_start(extend);
        let lead = self.lines[self.row].chars().take_while(|c| c.is_whitespace()).count();
        self.col = if self.col == lead { 0 } else { lead };
        self.goal = None;
    }

    pub fn end(&mut self, extend: bool) {
        self.select_start(extend);
        self.col = self.line_len(self.row);
        self.goal = None;
    }

    pub fn doc_start(&mut self, extend: bool) {
        self.select_start(extend);
        self.row = 0;
        self.col = 0;
    }

    pub fn doc_end(&mut self, extend: bool) {
        self.select_start(extend);
        self.row = self.lines.len() - 1;
        self.col = self.line_len(self.row);
    }

    pub fn word_left(&mut self, extend: bool) {
        self.select_start(extend);
        if self.col == 0 {
            self.left(extend);
            return;
        }
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        let mut c = self.col;
        while c > 0 && !is_word(chars[c - 1]) {
            c -= 1;
        }
        while c > 0 && is_word(chars[c - 1]) {
            c -= 1;
        }
        self.col = c;
        self.goal = None;
    }

    pub fn word_right(&mut self, extend: bool) {
        self.select_start(extend);
        let chars: Vec<char> = self.lines[self.row].chars().collect();
        if self.col >= chars.len() {
            self.right(extend);
            return;
        }
        let mut c = self.col;
        while c < chars.len() && !is_word(chars[c]) {
            c += 1;
        }
        while c < chars.len() && is_word(chars[c]) {
            c += 1;
        }
        self.col = c;
        self.goal = None;
    }

    pub fn delete_word_left(&mut self) {
        let end = (self.row, self.col);
        self.word_left(false);
        let start = (self.row, self.col);
        if start != end {
            self.snapshot(false);
            self.delete_range(start, end);
        }
    }

    pub fn select_all(&mut self) {
        self.anchor = Some((0, 0));
        self.doc_end(true);
    }

    /// Move the cursor `rows` lines and the view with it, so the cursor keeps its place on
    /// screen. Paging stops scrolling once the last line is at the bottom.
    pub fn page(&mut self, rows: usize, down: bool, extend: bool) {
        let from = self.row;
        for _ in 0..rows {
            if down { self.down(extend) } else { self.up(extend) };
        }
        let moved = self.row.abs_diff(from);
        self.scroll = if down {
            let end = self.lines.len().saturating_sub(self.view_rows);
            (self.scroll + moved).min(end.max(self.scroll))
        } else {
            self.scroll.saturating_sub(moved)
        };
    }

    /// Rows a page key moves: the view's height less one, so a line of context carries over.
    fn page_step(&self) -> usize {
        self.view_rows.saturating_sub(1).max(1)
    }

    /// Scroll the view (the wheel) without moving the cursor or the selection.
    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.lines.len() - 1;
        self.scroll = (self.scroll as isize + delta).clamp(0, last as isize) as usize;
    }

    /// Display column of the cursor within its line.
    pub fn cursor_x(&self) -> usize {
        self.lines[self.row].chars().take(self.col).map(|c| c.width().unwrap_or(0)).sum()
    }

    /// Scroll the view just far enough to show the cursor in a `rows` by `width` window. It
    /// only does so when the cursor, the text or the window changed since the last call, which
    /// leaves the wheel free to scroll away from the cursor and stay there.
    pub fn follow_cursor(&mut self, rows: usize, width: usize) {
        let seen = (self.row, self.col, self.generation, rows, width);
        if self.followed == Some(seen) {
            return;
        }
        self.followed = Some(seen);
        if self.row < self.scroll {
            self.scroll = self.row;
        } else if rows > 0 && self.row >= self.scroll + rows {
            self.scroll = self.row + 1 - rows;
        }
        let x = self.cursor_x();
        if x < self.hscroll {
            self.hscroll = x.saturating_sub(4);
        } else if width > 1 && x >= self.hscroll + width - 1 {
            self.hscroll = x + 2 - width;
        }
    }

    /// Where a selection that takes in all of `row` ends: the start of the next line, or
    /// the end of the text on the last line.
    fn line_end_pos(&self, row: usize) -> (usize, usize) {
        if row + 1 < self.lines.len() { (row + 1, 0) } else { (row, self.line_len(row)) }
    }

    /// Select whole lines from `from` to `to`, in either direction.
    pub fn select_lines(&mut self, from: usize, to: usize) {
        let last = self.lines.len() - 1;
        let (from, to) = (from.min(last), to.min(last));
        if to >= from {
            self.anchor = Some((from, 0));
            (self.row, self.col) = self.line_end_pos(to);
        } else {
            self.anchor = Some(self.line_end_pos(from));
            (self.row, self.col) = (to, 0);
        }
        self.goal = None;
    }

    /// Select the cursor's line; when whole lines are already selected, take one more.
    pub fn select_line(&mut self) {
        let last = self.lines.len() - 1;
        let whole = self.selection().filter(|(s, e)| s.1 == 0 && (e.1 == 0 || (e.0 == last && e.1 == self.line_len(last))));
        match whole {
            Some((s, e)) if e.1 == 0 => {
                self.anchor = Some(s);
                (self.row, self.col) = self.line_end_pos(e.0);
                self.goal = None;
            }
            Some(_) => {}
            None => self.select_lines(self.row, self.row),
        }
    }

    /// Select the word at a position (a click just past a word's end still picks it).
    pub fn select_word_at(&mut self, row: usize, col: usize) {
        let row = row.min(self.lines.len() - 1);
        let chars: Vec<char> = self.lines[row].chars().collect();
        let col = col.min(chars.len());
        let on = |i: usize| chars.get(i).is_some_and(|c| is_word(*c));
        let at = if on(col) { col } else if col > 0 && on(col - 1) { col - 1 } else {
            self.anchor = None;
            self.set_cursor(row, col);
            return;
        };
        let mut s = at;
        while s > 0 && on(s - 1) {
            s -= 1;
        }
        let mut e = at + 1;
        while on(e) {
            e += 1;
        }
        self.anchor = Some((row, s));
        (self.row, self.col) = (row, e);
        self.goal = None;
    }

    /// Move the cursor, keeping (or starting) a selection from where it was; for shift-click and drag.
    pub fn drag_to(&mut self, row: usize, col: usize) {
        if self.anchor.is_none() {
            self.anchor = Some((self.row, self.col));
        }
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.line_len(self.row));
        self.goal = None;
    }

    /// First and last row a line-wise command applies to: the cursor's, or every row the
    /// selection touches (one ending at column 0 of a later row doesn't take that row).
    fn covered_rows(&self) -> (usize, usize) {
        match self.selection() {
            Some((s, e)) => (s.0, if e.1 == 0 && e.0 > s.0 { e.0 - 1 } else { e.0 }),
            None => (self.row, self.row),
        }
    }

    /// Swap the selected lines (or the cursor's line) with the one above or below.
    pub fn move_lines(&mut self, down: bool) {
        let (first, last) = self.covered_rows();
        if if down { last + 1 >= self.lines.len() } else { first == 0 } {
            return;
        }
        self.snapshot(false);
        if down {
            let l = self.lines.remove(last + 1);
            self.lines.insert(first, l);
        } else {
            let l = self.lines.remove(first - 1);
            self.lines.insert(last, l);
        }
        let moved = |p: (usize, usize)| if down { (p.0 + 1, p.1) } else { (p.0 - 1, p.1) };
        // a selection ending at the start of the next line can land past the last one
        let clamp = |e: &Editor, p: (usize, usize)| if p.0 >= e.lines.len() { (e.lines.len() - 1, e.line_len(e.lines.len() - 1)) } else { p };
        (self.row, self.col) = clamp(self, moved((self.row, self.col)));
        if let Some(a) = self.anchor {
            self.anchor = Some(clamp(self, moved(a)));
        }
        self.goal = None;
        self.changed();
    }

    /// The identifier fragment before the cursor (letters, digits, `_`), and whether a `.` precedes it.
    pub fn word_before_cursor(&self) -> (String, Option<String>) {
        let chars: Vec<char> = self.lines[self.row].chars().take(self.col).collect();
        let mut i = chars.len();
        while i > 0 && is_word(chars[i - 1]) {
            i -= 1;
        }
        let word: String = chars[i..].iter().collect();
        let qualifier = if i > 0 && chars[i - 1] == '.' {
            let mut j = i - 1;
            while j > 0 && (is_word(chars[j - 1]) || chars[j - 1] == '"' || chars[j - 1] == '`') {
                j -= 1;
            }
            Some(chars[j..i - 1].iter().collect::<String>().trim_matches(['"', '`']).to_string())
        } else {
            None
        };
        (word, qualifier)
    }

    /// Replace the word before the cursor with `with`.
    pub fn complete_word(&mut self, with: &str) {
        let (word, _) = self.word_before_cursor();
        let n = word.chars().count();
        self.snapshot(false);
        let line = &mut self.lines[self.row];
        let a = char_to_byte(line, self.col - n);
        let b = char_to_byte(line, self.col);
        line.replace_range(a..b, with);
        self.col = self.col - n + with.chars().count();
        self.coalesce = false;
        self.changed();
    }

    pub fn copy_selection(&mut self) -> Option<String> {
        let t = self.selected_text()?;
        self.clipboard = t.clone();
        Some(t)
    }

    /// Standard (non-modal) editing keys. Global keys are handled before this.
    pub fn handle_key(&mut self, k: KeyEvent) -> Outcome {
        if self.vim.is_some() && self.vim_mode() != Some(VimMode::Insert) {
            return self.vim_key(k);
        }
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let super_ = k.modifiers.intersects(KeyModifiers::SUPER | KeyModifiers::META);
        match k.code {
            KeyCode::Esc if self.vim.is_some() => {
                self.vim.as_mut().unwrap().mode = Some(VimMode::Normal);
                self.anchor = None;
                if self.col > 0 {
                    self.col -= 1;
                }
            }
            KeyCode::Char(c) if ctrl => match c {
                'a' => self.home(shift),
                'e' => self.end(shift),
                'l' => self.select_line(),
                'z' if shift => self.redo(),
                'z' => self.undo(),
                'y' => self.redo(),
                'h' => self.backspace(),
                'd' => self.delete(),
                'u' => {
                    let end = (self.row, self.col);
                    self.snapshot(false);
                    self.delete_range((self.row, 0), end);
                }
                _ => return Outcome::NotHandled,
            },
            KeyCode::Char(c) if alt => match c {
                'b' => self.word_left(shift),
                'f' => self.word_right(shift),
                'c' | 'w' => {
                    self.copy_selection();
                }
                'x' => {
                    if self.copy_selection().is_some() {
                        self.delete_selection();
                    }
                }
                'v' => {
                    let c = self.clipboard.clone();
                    self.insert_str(&c);
                }
                'a' => self.select_all(),
                _ => return Outcome::NotHandled,
            },
            KeyCode::Char(c) => self.insert_str(&c.to_string()),
            KeyCode::Enter => self.newline(),
            KeyCode::Backspace if alt || ctrl => self.delete_word_left(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            // Cmd+arrows when the terminal reports the Super modifier
            KeyCode::Left if super_ => self.home(shift),
            KeyCode::Right if super_ => self.end(shift),
            KeyCode::Up if super_ => self.doc_start(shift),
            KeyCode::Down if super_ => self.doc_end(shift),
            KeyCode::Up if alt && shift => self.move_lines(false),
            KeyCode::Down if alt && shift => self.move_lines(true),
            KeyCode::Left if alt || ctrl => self.word_left(shift),
            KeyCode::Right if alt || ctrl => self.word_right(shift),
            KeyCode::Left => self.left(shift),
            KeyCode::Right => self.right(shift),
            KeyCode::Up => {
                self.up(shift);
            }
            KeyCode::Down => {
                self.down(shift);
            }
            KeyCode::Home if ctrl => self.doc_start(shift),
            KeyCode::End if ctrl => self.doc_end(shift),
            KeyCode::Home => self.home(shift),
            KeyCode::End => self.end(shift),
            KeyCode::PageUp => self.page(self.page_step(), false, shift),
            KeyCode::PageDown => self.page(self.page_step(), true, shift),
            KeyCode::Tab => self.insert_str("    "),
            KeyCode::BackTab => {
                let lead = self.lines[self.row].chars().take_while(|c| *c == ' ').count().min(4);
                if lead > 0 {
                    self.snapshot(false);
                    self.lines[self.row].replace_range(0..lead, "");
                    self.col = self.col.saturating_sub(lead);
                    self.changed();
                }
            }
            _ => return Outcome::NotHandled,
        }
        Outcome::Handled
    }

    fn vim_key(&mut self, k: KeyEvent) -> Outcome {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let mode = self.vim_mode().unwrap_or(VimMode::Normal);
        let KeyCode::Char(c) = k.code else {
            return match k.code {
                KeyCode::Esc => {
                    let v = self.vim.as_mut().unwrap();
                    if !v.pending.is_empty() {
                        v.pending.clear();
                        return Outcome::Handled;
                    }
                    if matches!(mode, VimMode::Visual | VimMode::VisualLine) {
                        v.mode = Some(VimMode::Normal);
                        self.anchor = None;
                        return Outcome::Handled;
                    }
                    Outcome::Leave
                }
                KeyCode::Left => {
                    self.left(self.anchor.is_some());
                    Outcome::Handled
                }
                KeyCode::Right => {
                    self.right(self.anchor.is_some());
                    Outcome::Handled
                }
                KeyCode::Up => {
                    self.up(self.anchor.is_some());
                    Outcome::Handled
                }
                KeyCode::Down => {
                    self.down(self.anchor.is_some());
                    Outcome::Handled
                }
                KeyCode::Enter => {
                    self.down(false);
                    self.home(false);
                    Outcome::Handled
                }
                _ => Outcome::NotHandled,
            };
        };
        if ctrl {
            return match c {
                'd' => {
                    self.page((self.view_rows / 2).max(1), true, false);
                    Outcome::Handled
                }
                'u' => {
                    self.page((self.view_rows / 2).max(1), false, false);
                    Outcome::Handled
                }
                _ => Outcome::NotHandled,
            };
        }
        let visual = matches!(mode, VimMode::Visual | VimMode::VisualLine);
        let pending = self.vim.as_ref().unwrap().pending.clone();
        let set_mode = |e: &mut Editor, m: VimMode| e.vim.as_mut().unwrap().mode = Some(m);
        let clear = |e: &mut Editor| e.vim.as_mut().unwrap().pending.clear();

        // operator pending: d / c / y + motion
        if let Some(op) = pending.chars().next().filter(|p| "dcy".contains(*p) && pending.len() == 1) {
            clear(self);
            if c == op {
                // linewise: dd / cc / yy
                let line = self.lines[self.row].clone();
                let v = self.vim.as_mut().unwrap();
                v.register = format!("{line}\n");
                v.register_lines = true;
                match op {
                    'd' => {
                        self.snapshot(false);
                        if self.lines.len() > 1 {
                            self.lines.remove(self.row);
                            self.row = self.row.min(self.lines.len() - 1);
                        } else {
                            self.lines[0].clear();
                        }
                        self.col = 0;
                        self.changed();
                    }
                    'c' => {
                        self.snapshot(false);
                        self.lines[self.row].clear();
                        self.col = 0;
                        self.changed();
                        set_mode(self, VimMode::Insert);
                    }
                    _ => {}
                }
                return Outcome::Handled;
            }
            let start = (self.row, self.col);
            let inner_word = c == 'i';
            if inner_word {
                self.vim.as_mut().unwrap().pending = format!("{op}i");
                return Outcome::Handled;
            }
            self.vim.as_mut().unwrap().pending = "op".into(); // motions stay on this line
            let moved = self.vim_motion(c, false);
            clear(self);
            if !moved {
                return Outcome::Handled;
            }
            let mut end = (self.row, self.col);
            if c == 'e' || c == '$' {
                end.1 = (end.1 + 1).min(self.line_len(end.0));
            }
            let (s, e) = if start <= end { (start, end) } else { (end, start) };
            self.apply_operator(op, s, e);
            return Outcome::Handled;
        }
        if pending.len() == 2 && pending.ends_with('i') {
            clear(self);
            let op = pending.chars().next().unwrap();
            if c == 'w' {
                let chars: Vec<char> = self.lines[self.row].chars().collect();
                let mut a = self.col.min(chars.len());
                let mut b = a;
                while a > 0 && chars.get(a - 1).is_some_and(|c| is_word(*c)) {
                    a -= 1;
                }
                while b < chars.len() && is_word(chars[b]) {
                    b += 1;
                }
                self.apply_operator(op, (self.row, a), (self.row, b));
            }
            return Outcome::Handled;
        }
        if pending == "g" {
            clear(self);
            if c == 'g' {
                self.doc_start(visual);
            }
            return Outcome::Handled;
        }

        if visual {
            match c {
                'y' | 'd' | 'c' | 'x' => {
                    let (s, e) = self.visual_range(mode);
                    let op = if c == 'x' { 'd' } else { c };
                    self.anchor = None;
                    self.apply_operator(op, s, e);
                    if op != 'c' {
                        set_mode(self, VimMode::Normal);
                    }
                    if mode == VimMode::VisualLine {
                        self.vim.as_mut().unwrap().register_lines = true;
                        if !self.vim.as_ref().unwrap().register.ends_with('\n') {
                            self.vim.as_mut().unwrap().register.push('\n');
                        }
                    }
                    return Outcome::Handled;
                }
                'v' => {
                    set_mode(self, VimMode::Normal);
                    self.anchor = None;
                    return Outcome::Handled;
                }
                _ => {
                    if self.vim_motion(c, true) {
                        return Outcome::Handled;
                    }
                    return Outcome::Handled;
                }
            }
        }

        match c {
            'i' => set_mode(self, VimMode::Insert),
            'a' => {
                if self.col < self.line_len(self.row) {
                    self.col += 1;
                }
                set_mode(self, VimMode::Insert);
            }
            'A' => {
                self.end(false);
                set_mode(self, VimMode::Insert);
            }
            'I' => {
                self.col = 0;
                self.home(false);
                if self.col != 0 {
                } else {
                    let lead = self.lines[self.row].chars().take_while(|c| c.is_whitespace()).count();
                    self.col = lead;
                }
                set_mode(self, VimMode::Insert);
            }
            'o' => {
                self.end(false);
                self.newline();
                set_mode(self, VimMode::Insert);
            }
            'O' => {
                self.col = 0;
                self.insert_str("\n");
                self.row -= 1;
                set_mode(self, VimMode::Insert);
            }
            'v' => {
                self.anchor = Some((self.row, self.col));
                set_mode(self, VimMode::Visual);
            }
            'V' => {
                self.anchor = Some((self.row, 0));
                set_mode(self, VimMode::VisualLine);
            }
            'x' => {
                if self.line_len(self.row) > 0 {
                    let (r, c0) = (self.row, self.col);
                    self.apply_operator('d', (r, c0), (r, c0 + 1));
                }
            }
            'D' => {
                let len = self.line_len(self.row);
                self.apply_operator('d', (self.row, self.col), (self.row, len));
            }
            'C' => {
                let len = self.line_len(self.row);
                self.apply_operator('c', (self.row, self.col), (self.row, len));
            }
            'p' | 'P' => {
                let v = self.vim.as_ref().unwrap();
                let (reg, lines) = (v.register.clone(), v.register_lines);
                if lines {
                    let body = reg.trim_end_matches('\n');
                    self.snapshot(false);
                    let at = if c == 'p' { self.row + 1 } else { self.row };
                    for (i, l) in body.split('\n').enumerate() {
                        self.lines.insert(at + i, l.to_string());
                    }
                    self.row = at;
                    self.col = 0;
                    self.changed();
                } else {
                    if c == 'p' && self.line_len(self.row) > 0 {
                        self.col += 1;
                    }
                    self.insert_str(&reg);
                    self.col = self.col.saturating_sub(1);
                }
            }
            'u' => self.undo(),
            'U' => self.redo(),
            'd' | 'c' | 'y' | 'g' => self.vim.as_mut().unwrap().pending = c.to_string(),
            _ => {
                if !self.vim_motion(c, false) {
                    return Outcome::NotHandled;
                }
            }
        }
        Outcome::Handled
    }

    fn visual_range(&self, mode: VimMode) -> ((usize, usize), (usize, usize)) {
        let a = self.anchor.unwrap_or((self.row, self.col));
        let b = (self.row, self.col);
        let (s, e) = if a <= b { (a, b) } else { (b, a) };
        if mode == VimMode::VisualLine {
            ((s.0, 0), (e.0, self.line_len(e.0)))
        } else {
            (s, (e.0, (e.1 + 1).min(self.line_len(e.0))))
        }
    }

    fn apply_operator(&mut self, op: char, s: (usize, usize), e: (usize, usize)) {
        let text = self.text();
        let a = self.offset_of(s.0, s.1);
        let b = self.offset_of(e.0, e.1);
        let v = self.vim.as_mut().unwrap();
        v.register = text[a..b].to_string();
        v.register_lines = false;
        match op {
            'd' | 'c' => {
                self.snapshot(false);
                self.delete_range(s, e);
                if op == 'c' {
                    self.vim.as_mut().unwrap().mode = Some(VimMode::Insert);
                }
            }
            _ => {
                self.row = s.0;
                self.col = s.1;
            }
        }
    }

    /// Apply a motion; returns false if `c` is not a motion.
    fn vim_motion(&mut self, c: char, extend: bool) -> bool {
        match c {
            'h' => {
                if self.col > 0 {
                    self.left(extend)
                }
            }
            'l' => {
                if self.col < self.line_len(self.row) {
                    self.right(extend)
                }
            }
            'j' => {
                self.down(extend);
            }
            'k' => {
                self.up(extend);
            }
            'w' => {
                // start of next word (vim semantics), wrapping to the next line
                let chars: Vec<char> = self.lines[self.row].chars().collect();
                if !extend {
                    self.anchor = None;
                }
                let mut i = self.col;
                if i < chars.len() && is_word(chars[i]) {
                    while i < chars.len() && is_word(chars[i]) {
                        i += 1;
                    }
                } else if i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                if i >= chars.len() && self.row + 1 < self.lines.len() && self.vim.as_ref().is_some_and(|v| v.pending.is_empty()) {
                    self.row += 1;
                    self.col = self.lines[self.row].chars().take_while(|c| c.is_whitespace()).count();
                } else {
                    self.col = i;
                }
            }
            'b' => self.word_left(extend),
            'e' => {
                let chars: Vec<char> = self.lines[self.row].chars().collect();
                let mut i = self.col + 1;
                while i < chars.len() && !is_word(chars[i]) {
                    i += 1;
                }
                while i + 1 < chars.len() && is_word(chars[i + 1]) {
                    i += 1;
                }
                if !extend {
                    self.anchor = None;
                }
                self.col = i.min(chars.len().saturating_sub(1));
            }
            '0' => {
                if !extend {
                    self.anchor = None;
                }
                self.col = 0;
            }
            '^' => {
                if !extend {
                    self.anchor = None;
                }
                self.col = self.lines[self.row].chars().take_while(|c| c.is_whitespace()).count();
            }
            '$' => {
                if !extend {
                    self.anchor = None;
                }
                self.col = self.line_len(self.row).saturating_sub(if extend { 0 } else { 1 });
            }
            'G' => {
                if !extend {
                    self.anchor = None;
                }
                self.row = self.lines.len() - 1;
                self.col = 0;
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn typing_newlines_and_undo() {
        let mut e = Editor::new("");
        for c in "select 1".chars() {
            e.handle_key(key(c));
        }
        e.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        e.insert_str("from t");
        assert_eq!(e.text(), "select 1\nfrom t");
        e.undo();
        assert_eq!(e.text(), "select 1\n");
        e.undo();
        e.undo();
        assert_eq!(e.text(), "select ");
        e.redo();
        assert_eq!(e.text(), "select 1");
    }

    #[test]
    fn offsets_roundtrip() {
        let e = Editor::new("ab\ncdé\nf");
        assert_eq!(e.offset_of(1, 3), 7);
        assert_eq!(e.pos_of(7), (1, 3));
        assert_eq!(e.pos_of(8), (2, 0));
    }

    #[test]
    fn selection_replace_and_words() {
        let mut e = Editor::new("select foo_bar from t");
        e.set_cursor(0, 14);
        assert_eq!(e.word_before_cursor().0, "foo_bar");
        e.word_left(true);
        assert_eq!(e.selected_text().as_deref(), Some("foo_bar"));
        e.insert_str("x");
        assert_eq!(e.text(), "select x from t");
        let mut e = Editor::new("select o.");
        e.set_cursor(0, 9);
        assert_eq!(e.word_before_cursor(), (String::new(), Some("o".into())));
    }

    #[test]
    fn select_line_extends_by_line() {
        let mut e = Editor::new("one\ntwo\nthree");
        e.set_cursor(1, 2);
        e.select_line();
        assert_eq!(e.selected_text().as_deref(), Some("two\n"));
        e.select_line();
        assert_eq!(e.selected_text().as_deref(), Some("two\nthree"));
        e.select_line();
        assert_eq!(e.selected_text().as_deref(), Some("two\nthree"));
        e.delete_selection();
        assert_eq!(e.text(), "one\n");
        // a partial selection restarts from the cursor's line
        let mut e = Editor::new("one\ntwo");
        e.set_cursor(0, 1);
        e.right(true);
        e.select_line();
        assert_eq!(e.selected_text().as_deref(), Some("one\n"));
    }

    #[test]
    fn select_lines_both_directions_and_words() {
        let mut e = Editor::new("a\nbb\nccc\ndd");
        e.select_lines(1, 2);
        assert_eq!(e.selected_text().as_deref(), Some("bb\nccc\n"));
        e.select_lines(2, 0);
        assert_eq!(e.selected_text().as_deref(), Some("a\nbb\nccc\n"));
        e.select_lines(3, 3);
        assert_eq!(e.selected_text().as_deref(), Some("dd"));
        let mut e = Editor::new("select foo_bar, x");
        e.select_word_at(0, 10);
        assert_eq!(e.selected_text().as_deref(), Some("foo_bar"));
        e.select_word_at(0, 14);
        assert_eq!(e.selected_text().as_deref(), Some("foo_bar"), "just past the end still picks the word");
        e.select_word_at(0, 15);
        assert_eq!(e.selection(), None, "whitespace selects nothing");
        e.set_cursor(0, 2);
        e.drag_to(0, 6);
        assert_eq!(e.selected_text().as_deref(), Some("lect"));
        e.drag_to(0, 0);
        assert_eq!(e.selected_text().as_deref(), Some("se"));
    }

    #[test]
    fn move_lines_keeps_selection() {
        let alt_shift = KeyModifiers::ALT | KeyModifiers::SHIFT;
        let mut e = Editor::new("a\nb\nc\nd");
        e.set_cursor(1, 1);
        e.handle_key(KeyEvent::new(KeyCode::Down, alt_shift));
        assert_eq!(e.text(), "a\nc\nb\nd");
        assert_eq!(e.cursor(), (2, 1));
        e.handle_key(KeyEvent::new(KeyCode::Up, alt_shift));
        e.handle_key(KeyEvent::new(KeyCode::Up, alt_shift));
        assert_eq!(e.text(), "b\na\nc\nd");
        e.handle_key(KeyEvent::new(KeyCode::Up, alt_shift));
        assert_eq!(e.text(), "b\na\nc\nd", "stops at the top");
        e.undo();
        assert_eq!(e.text(), "a\nb\nc\nd");
        // a whole-line selection moves as a block and stays selected
        let mut e = Editor::new("a\nb\nc\nd");
        e.select_lines(1, 2);
        e.move_lines(true);
        assert_eq!(e.text(), "a\nd\nb\nc");
        assert_eq!(e.selected_text().as_deref(), Some("b\nc"));
        e.move_lines(false);
        assert_eq!(e.text(), "a\nb\nc\nd");
        assert_eq!(e.selected_text().as_deref(), Some("b\nc"), "still the same two lines");
        e.move_lines(true);
        e.move_lines(true);
        assert_eq!(e.text(), "a\nd\nb\nc", "stops at the bottom");
    }

    #[test]
    fn paging_extends_selection() {
        let mut e = Editor::new(&"x\n".repeat(40));
        e.view_rows = 16;
        e.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!((e.cursor(), e.selection()), ((15, 0), None));
        e.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::SHIFT));
        assert_eq!(e.cursor(), (30, 0));
        assert_eq!(e.selection(), Some(((15, 0), (30, 0))));
        e.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT));
        assert_eq!(e.selection(), None);
    }

    #[test]
    fn view_follows_the_cursor_only_when_it_moves() {
        let mut e = Editor::new(&"x\n".repeat(40));
        e.follow_cursor(10, 40);
        e.scroll_by(5);
        e.follow_cursor(10, 40);
        assert_eq!((e.scroll, e.cursor()), (5, (0, 0)), "scrolling away from the cursor sticks");
        e.down(false);
        e.follow_cursor(10, 40);
        assert_eq!(e.scroll, 1, "moving it brings the view back, by the least it takes");
        e.set_cursor(20, 0);
        e.follow_cursor(10, 40);
        assert_eq!(e.scroll, 11);
        e.scroll_by(-100);
        assert_eq!(e.scroll, 0);
        e.scroll_by(100);
        assert_eq!(e.scroll, 40, "the wheel stops at the last line");
        e.follow_cursor(10, 20);
        assert_eq!(e.scroll, 20, "resizing the view shows the cursor again, on the top row");
    }

    #[test]
    fn paging_keeps_the_cursor_on_its_screen_row() {
        let mut e = Editor::new(&"x\n".repeat(60));
        e.view_rows = 10;
        e.set_cursor(3, 0);
        let page = |e: &mut Editor, k| e.handle_key(KeyEvent::new(k, KeyModifiers::NONE));
        page(&mut e, KeyCode::PageDown);
        assert_eq!((e.scroll, e.row), (9, 12), "one line of overlap, cursor still on the 4th row");
        page(&mut e, KeyCode::PageUp);
        assert_eq!((e.scroll, e.row), (0, 3));
        e.set_cursor(58, 0);
        e.scroll = 51;
        page(&mut e, KeyCode::PageDown);
        assert_eq!((e.scroll, e.row), (51, 60), "stops with the last line at the bottom");
    }

    #[test]
    fn vim_basics() {
        let mut e = Editor::new("one two three\nsecond");
        e.set_vim(true);
        e.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        e.set_cursor(0, 0);
        e.handle_key(key('d'));
        e.handle_key(key('w'));
        assert_eq!(e.lines[0], "two three");
        e.handle_key(key('d'));
        e.handle_key(key('d'));
        assert_eq!(e.text(), "second");
        e.handle_key(key('p'));
        assert_eq!(e.text(), "second\ntwo three");
        e.handle_key(key('c'));
        e.handle_key(key('i'));
        e.handle_key(key('w'));
        assert_eq!(e.vim_mode(), Some(VimMode::Insert));
        e.insert_str("x");
        assert_eq!(e.text(), "second\nx three");
        assert_eq!(e.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), Outcome::Handled);
        assert_eq!(e.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)), Outcome::Leave);
    }
}
