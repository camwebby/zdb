//! Copy formats for grid selections, and the clipboard (the system clipboard
//! when running locally, or OSC 52 when it is unavailable or over SSH).

use crate::config::DriverKind;
use crate::db::{ColKind, literal, quote_ident};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Values,
    Csv,
    CsvNoHeader,
    Json,
    Markdown,
    Insert,
    Names,
    InList,
}

impl Format {
    pub fn from_key(c: char) -> Option<Format> {
        Some(match c {
            'y' => Format::Values,
            'c' => Format::Csv,
            'C' => Format::CsvNoHeader,
            'j' => Format::Json,
            'm' => Format::Markdown,
            'i' => Format::Insert,
            'n' => Format::Names,
            'w' => Format::InList,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Values => "values",
            Format::Csv => "CSV",
            Format::CsvNoHeader => "CSV without headers",
            Format::Json => "JSON",
            Format::Markdown => "Markdown",
            Format::Insert => "INSERT statements",
            Format::Names => "column names",
            Format::InList => "IN list",
        }
    }
}

/// A rectangular slice of a result, already resolved to values.
pub struct Table<'a> {
    pub names: Vec<&'a str>,
    pub kinds: Vec<ColKind>,
    pub rows: Vec<Vec<Option<&'a str>>>,
}

pub fn render(f: Format, t: &Table, driver: DriverKind, table_name: &str) -> String {
    match f {
        Format::Values => t.rows.iter().map(|r| r.iter().map(|v| tsv_field(v.unwrap_or(""))).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n"),
        Format::Csv | Format::CsvNoHeader => {
            let mut lines = Vec::new();
            if f == Format::Csv {
                lines.push(t.names.iter().map(|n| csv_field(n)).collect::<Vec<_>>().join(","));
            }
            for r in &t.rows {
                lines.push(r.iter().map(|v| csv_field(v.unwrap_or(""))).collect::<Vec<_>>().join(","));
            }
            let mut s = lines.join("\r\n");
            s.push_str("\r\n");
            s
        }
        Format::Json => {
            let arr: Vec<serde_json::Value> = t
                .rows
                .iter()
                .map(|r| {
                    let mut m = serde_json::Map::new();
                    for (i, v) in r.iter().enumerate() {
                        m.insert(t.names[i].to_string(), json_value(t.kinds[i], *v));
                    }
                    serde_json::Value::Object(m)
                })
                .collect();
            serde_json::to_string_pretty(&arr).unwrap_or_default()
        }
        Format::Markdown => {
            let esc = |s: &str| s.replace('|', "\\|").replace('\n', " ");
            let mut out = format!("| {} |\n", t.names.iter().map(|n| esc(n)).collect::<Vec<_>>().join(" | "));
            out.push_str(&format!(
                "|{}|\n",
                t.kinds.iter().map(|k| if k.numeric() { "---:" } else { "---" }).collect::<Vec<_>>().join("|")
            ));
            for r in &t.rows {
                out.push_str(&format!("| {} |\n", r.iter().map(|v| v.map(esc).unwrap_or_else(|| "NULL".into())).collect::<Vec<_>>().join(" | ")));
            }
            out
        }
        Format::Insert => {
            let cols = t.names.iter().map(|n| quote_ident(driver, n)).collect::<Vec<_>>().join(", ");
            let tn = table_name
                .split('.')
                .map(|p| quote_ident(driver, p))
                .collect::<Vec<_>>()
                .join(".");
            t.rows
                .iter()
                .map(|r| {
                    let vals = r.iter().enumerate().map(|(i, v)| literal(driver, t.kinds[i], *v)).collect::<Vec<_>>().join(", ");
                    format!("insert into {tn} ({cols}) values ({vals});")
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        Format::Names => t.names.join(", "),
        Format::InList => {
            let mut seen = Vec::new();
            for r in &t.rows {
                for (i, v) in r.iter().enumerate() {
                    let l = literal(driver, t.kinds[i], *v);
                    if !seen.contains(&l) {
                        seen.push(l);
                    }
                }
            }
            format!("in ({})", seen.join(", "))
        }
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn tsv_field(s: &str) -> String {
    if s.contains(['\t', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

pub fn json_value(kind: ColKind, v: Option<&str>) -> serde_json::Value {
    use serde_json::Value as J;
    let Some(v) = v else { return J::Null };
    match kind {
        ColKind::Int => v.parse::<i64>().map(J::from).unwrap_or_else(|_| J::String(v.into())),
        ColKind::Float | ColKind::Decimal => {
            serde_json::from_str::<serde_json::Number>(v).map(J::Number).unwrap_or_else(|_| J::String(v.into()))
        }
        ColKind::Bool => match v {
            "true" | "t" | "1" => J::Bool(true),
            "false" | "f" | "0" => J::Bool(false),
            _ => J::String(v.into()),
        },
        ColKind::Json => serde_json::from_str(v).unwrap_or_else(|_| J::String(v.into())),
        _ => J::String(v.into()),
    }
}

pub struct Clipboard {
    local: Option<arboard::Clipboard>,
}

pub fn over_ssh() -> bool {
    std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some()
}

impl Clipboard {
    pub fn new() -> Clipboard {
        let local = if over_ssh() { None } else { arboard::Clipboard::new().ok() };
        Clipboard { local }
    }

    pub fn set(&mut self, text: &str) -> Result<(), String> {
        let local = self.local.as_mut().map(|c| move |text: &str| c.set_text(text.to_string()).map_err(|e| e.to_string()));
        write_clipboard(text, local, &mut std::io::stdout())
    }
}

fn write_clipboard(text: &str, local: Option<impl FnOnce(&str) -> Result<(), String>>, out: &mut impl Write) -> Result<(), String> {
    if let Some(set_local) = local {
        // OSC 52 makes the terminal write the same clipboard asynchronously.
        // Using both paths can steal macOS pasteboard ownership between
        // arboard's clearContents and writeObjects calls.
        return set_local(text);
    }
    out.write_all(osc52(text).as_bytes()).and_then(|_| out.flush()).map_err(|e| e.to_string())
}

pub fn osc52(text: &str) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    if std::env::var_os("TMUX").is_some() && std::env::var("ZDB_TMUX_PASSTHROUGH").is_ok() {
        format!("\x1bPtmux;\x1b\x1b]52;c;{b64}\x07\x1b\\")
    } else {
        format!("\x1b]52;c;{b64}\x07")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Table<'static> {
        Table {
            names: vec!["week", "total"],
            kinds: vec![ColKind::Text, ColKind::Decimal],
            rows: vec![vec![Some("2026-09-28"), Some("18402.50")], vec![Some("a,\"b\""), None]],
        }
    }

    #[test]
    fn formats() {
        let d = DriverKind::Postgres;
        assert_eq!(render(Format::Values, &t(), d, "x"), "2026-09-28\t18402.50\n\"a,\"\"b\"\"\"\t");
        assert_eq!(render(Format::Csv, &t(), d, "x"), "week,total\r\n2026-09-28,18402.50\r\n\"a,\"\"b\"\"\",\r\n");
        assert_eq!(render(Format::CsvNoHeader, &t(), d, "x"), "2026-09-28,18402.50\r\n\"a,\"\"b\"\"\",\r\n");
        assert_eq!(
            render(Format::Insert, &t(), d, "public.orders"),
            "insert into public.orders (week, total) values ('2026-09-28', 18402.50);\ninsert into public.orders (week, total) values ('a,\"b\"', NULL);"
        );
        assert_eq!(render(Format::Names, &t(), d, "x"), "week, total");
        assert_eq!(render(Format::InList, &t(), d, "x"), "in ('2026-09-28', 18402.50, 'a,\"b\"', NULL)");
        assert!(render(Format::Markdown, &t(), d, "x").starts_with("| week | total |\n|---|---:|\n"));
        let j: serde_json::Value = serde_json::from_str(&render(Format::Json, &t(), d, "x")).unwrap();
        assert_eq!(j[0]["total"], serde_json::json!(18402.50));
        assert_eq!(j[1]["total"], serde_json::Value::Null);
    }

    #[test]
    fn osc52_encodes() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn local_copy_does_not_race_terminal_clipboard() {
        use std::cell::Cell;

        // Model an OSC 52-capable terminal claiming the pasteboard as soon as
        // output is written. A competing native write then fails like AppKit.
        struct Terminal<'a>(&'a Cell<bool>);
        impl Write for Terminal<'_> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.set(true);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let terminal_wrote = Cell::new(false);
        let text = render(Format::Csv, &t(), DriverKind::Postgres, "x");
        let local = |copied: &str| {
            assert_eq!(copied, text);
            if terminal_wrote.get() {
                Err("NSPasteboard#writeObjects: returned false".into())
            } else {
                Ok(())
            }
        };
        assert_eq!(write_clipboard(&text, Some(local), &mut Terminal(&terminal_wrote)), Ok(()));
        assert!(!terminal_wrote.get(), "native copies must not also emit OSC 52");
    }

    #[test]
    fn local_clipboard_errors_are_reported_without_terminal_write() {
        let mut out = Vec::new();
        let error = "NSPasteboard#writeObjects: returned false";
        let result = write_clipboard("hi", Some(|_: &str| Err(error.to_string())), &mut out);
        assert_eq!(result, Err(error.to_string()));
        assert!(out.is_empty());
    }

    #[test]
    fn clipboard_without_native_backend_uses_osc52() {
        let text = render(Format::Csv, &t(), DriverKind::Postgres, "x");
        let mut out = Vec::new();
        write_clipboard(&text, None::<fn(&str) -> Result<(), String>>, &mut out).unwrap();
        assert_eq!(out, osc52(&text).into_bytes());
    }

    #[test]
    fn terminal_clipboard_reports_write_and_flush_errors() {
        struct FailingTerminal {
            fail_write: bool,
        }
        impl Write for FailingTerminal {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if self.fail_write {
                    Err(std::io::Error::other("write failed"))
                } else {
                    Ok(buf.len())
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("flush failed"))
            }
        }

        for fail_write in [true, false] {
            let result = write_clipboard("hi", None::<fn(&str) -> Result<(), String>>, &mut FailingTerminal { fail_write });
            assert_eq!(result, Err(if fail_write { "write failed" } else { "flush failed" }.into()));
        }
    }
}
