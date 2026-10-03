//! Colours and glyphs. Defaults use the terminal's 16 ANSI colours so the app follows
//! the user's theme; theme.toml can override any role with a named or #rrggbb colour.

use crate::config::{Level, config_dir};
use ratatui::style::Color;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Theme {
    pub accent: Color,
    pub local: Color,
    pub staging: Color,
    pub prod: Color,
    pub prod_fg: Color,
    pub error: Color,
    pub pending: Color,
    pub row: Color,
    pub selection: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    pub function: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            accent: Color::Cyan,
            local: Color::Gray,
            staging: Color::Yellow,
            prod: Color::Red,
            prod_fg: Color::White,
            error: Color::Red,
            pending: Color::Yellow,
            row: Color::DarkGray,
            selection: Color::Blue,
            keyword: Color::Blue,
            string: Color::Green,
            number: Color::Magenta,
            comment: Color::DarkGray,
            function: Color::Cyan,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ThemeFile {
    accent: Option<String>,
    local: Option<String>,
    staging: Option<String>,
    prod: Option<String>,
    prod_fg: Option<String>,
    error: Option<String>,
    pending: Option<String>,
    row: Option<String>,
    selection: Option<String>,
    keyword: Option<String>,
    string: Option<String>,
    number: Option<String>,
    comment: Option<String>,
    function: Option<String>,
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let v = u32::from_str_radix(hex, 16).ok()?;
            return Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        return None;
    }
    if let Ok(n) = s.parse::<u8>() {
        return Some(Color::Indexed(n));
    }
    s.parse::<Color>().ok()
}

impl Theme {
    pub fn load() -> (Theme, Option<String>) {
        let mut t = Theme::default();
        let path = config_dir().join("theme.toml");
        let Ok(text) = std::fs::read_to_string(&path) else { return (t, None) };
        let f: ThemeFile = match toml::from_str(&text) {
            Ok(f) => f,
            Err(e) => return (t, Some(format!("theme.toml: {e}"))),
        };
        let set = |slot: &mut Color, v: &Option<String>| {
            if let Some(c) = v.as_deref().and_then(parse_color) {
                *slot = c;
            }
        };
        set(&mut t.accent, &f.accent);
        set(&mut t.local, &f.local);
        set(&mut t.staging, &f.staging);
        set(&mut t.prod, &f.prod);
        set(&mut t.prod_fg, &f.prod_fg);
        set(&mut t.error, &f.error);
        set(&mut t.pending, &f.pending);
        set(&mut t.row, &f.row);
        set(&mut t.selection, &f.selection);
        set(&mut t.keyword, &f.keyword);
        set(&mut t.string, &f.string);
        set(&mut t.number, &f.number);
        set(&mut t.comment, &f.comment);
        set(&mut t.function, &f.function);
        (t, None)
    }

    pub fn env(&self, l: Level) -> Color {
        match l {
            Level::Local => self.local,
            Level::Staging => self.staging,
            Level::Prod => self.prod,
        }
    }

}

#[derive(Debug, Clone)]
pub struct Glyphs {
    pub dot: &'static str,
    pub up: &'static str,
    pub down: &'static str,
    pub bar: &'static str,
    pub check: &'static str,
    pub cross: &'static str,
    pub arrow: &'static str,
    pub ellipsis: &'static str,
    pub rule: &'static str,
    pub space: &'static str,
    pub enter: &'static str,
    pub spinner: &'static [&'static str],
    pub block_full: char,
    pub blocks: &'static [char],
}

impl Glyphs {
    pub fn new(ascii: bool) -> Glyphs {
        if ascii {
            Glyphs {
                dot: "*",
                up: "^",
                down: "v",
                bar: "|",
                check: "ok",
                cross: "x",
                arrow: "->",
                ellipsis: "~",
                rule: "-",
                space: "spc",
                enter: "ret",
                spinner: &["|", "/", "-", "\\"],
                block_full: '#',
                blocks: &['.', ':', '#'],
            }
        } else {
            Glyphs {
                dot: "●",
                up: "↑",
                down: "↓",
                bar: "▌",
                check: "✓",
                cross: "✗",
                arrow: "→",
                ellipsis: "…",
                rule: "─",
                space: "␣",
                enter: "⏎",
                spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
                block_full: '█',
                blocks: &['▏', '▎', '▍', '▌', '▋', '▊', '▉'],
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors() {
        assert_eq!(parse_color("#ff0080"), Some(Color::Rgb(255, 0, 128)));
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("236"), Some(Color::Indexed(236)));
        assert_eq!(parse_color("#ff"), None);
    }
}
