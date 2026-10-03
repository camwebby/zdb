//! SQL text utilities: a forgiving tokenizer plus everything built on it
//! (statement splitting, classification, row caps, parameters, formatting).
//! None of this is a parser; it only needs to be right about depth-0 structure.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tok {
    Word,
    QuotedIdent,
    Str,
    Number,
    Comment,
    Param,
    Punct,
    Semi,
    Space,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: Tok,
    pub range: Range<usize>,
    /// Parenthesis depth at the start of the token.
    pub depth: i32,
}

impl Token {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.range.clone()]
    }
    pub fn is_word(&self, src: &str, w: &str) -> bool {
        self.kind == Tok::Word && self.text(src).eq_ignore_ascii_case(w)
    }
    pub fn significant(&self) -> bool {
        !matches!(self.kind, Tok::Space | Tok::Comment)
    }
}

pub fn tokenize(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut depth = 0i32;
    while i < b.len() {
        let start = i;
        let c = b[i];
        let kind = if c.is_ascii_whitespace() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            Tok::Space
        } else if c == b'-' && b.get(i + 1) == Some(&b'-') || c == b'#' && is_mysql_comment_hash(b, i) {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            Tok::Comment
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
            Tok::Comment
        } else if c == b'\'' || ((c == b'e' || c == b'E') && b.get(i + 1) == Some(&b'\'')) {
            let backslash = c != b'\'';
            if backslash {
                i += 1;
            }
            i += 1;
            while i < b.len() {
                if backslash && b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == b'\'' {
                    if b.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            i = i.min(b.len());
            Tok::Str
        } else if c == b'"' || c == b'`' {
            i += 1;
            while i < b.len() {
                if b[i] == c {
                    if b.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            i = i.min(b.len());
            Tok::QuotedIdent
        } else if c == b'$' {
            // $1 parameter, or $tag$ ... $tag$ dollar quote
            let mut j = i + 1;
            if j < b.len() && b[j].is_ascii_digit() {
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                i = j;
                Tok::Param
            } else {
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j < b.len() && b[j] == b'$' {
                    let tag = &src[i..=j];
                    match src[j + 1..].find(tag) {
                        Some(p) => i = j + 1 + p + tag.len(),
                        None => i = b.len(),
                    }
                    Tok::Str
                } else {
                    i += 1;
                    Tok::Punct
                }
            }
        } else if c == b':' {
            if b.get(i + 1) == Some(&b':') || b.get(i + 1) == Some(&b'=') {
                i += 2;
                Tok::Punct
            } else if b.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic() || *n == b'_')
                && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || b[i - 1] == b']'))
            {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                Tok::Param
            } else {
                i += 1;
                Tok::Punct
            }
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|n| n.is_ascii_digit())) {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                // exponent sign
                if (b[i] == b'e' || b[i] == b'E') && matches!(b.get(i + 1), Some(b'+') | Some(b'-')) {
                    i += 1;
                }
                i += 1;
            }
            Tok::Number
        } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$' || b[i] >= 0x80) {
                i += 1;
            }
            Tok::Word
        } else if c == b';' {
            i += 1;
            Tok::Semi
        } else {
            // multi-byte safe: advance one char
            i += src[i..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
            Tok::Punct
        };
        let tok_depth = depth;
        if kind == Tok::Punct {
            match &src[start..i] {
                "(" => depth += 1,
                ")" => depth = (depth - 1).max(0),
                _ => {}
            }
        }
        out.push(Token { kind, range: start..i, depth: tok_depth });
    }
    out
}

/// `#` starts a comment in MySQL; in Postgres it is an operator. We only treat it as a
/// comment at line start (after optional whitespace) which is the common scratchpad use.
fn is_mysql_comment_hash(b: &[u8], i: usize) -> bool {
    let mut j = i;
    while j > 0 {
        j -= 1;
        match b[j] {
            b'\n' => return true,
            b' ' | b'\t' => continue,
            _ => return false,
        }
    }
    true
}

/// A statement inside a buffer. `range` covers the statement text from its first
/// significant token to its last; `full` extends to and includes the terminating `;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stmt {
    pub range: Range<usize>,
    pub full: Range<usize>,
}

/// Split a buffer into statements on `;` and (optionally) on blank lines at depth 0.
pub fn split(src: &str, blank_lines_split: bool) -> Vec<Stmt> {
    let toks = tokenize(src);
    let mut out = Vec::new();
    let mut first: Option<usize> = None;
    let mut last_end = 0;
    let push = |out: &mut Vec<Stmt>, first: &mut Option<usize>, last_end: usize, full_end: usize| {
        if let Some(s) = first.take() {
            out.push(Stmt { range: s..last_end, full: s..full_end });
        }
    };
    for t in &toks {
        match t.kind {
            Tok::Semi => {
                push(&mut out, &mut first, last_end, t.range.end);
            }
            Tok::Space => {
                if blank_lines_split && t.depth == 0 && first.is_some() && t.text(src).matches('\n').count() >= 2 {
                    push(&mut out, &mut first, last_end, last_end);
                }
            }
            Tok::Comment => {
                // a comment does not start a statement on its own, but belongs to one already started
                if first.is_some() {
                    // keep last_end at significant tokens so trailing comments don't swallow appended text
                }
            }
            _ => {
                if first.is_none() {
                    first = Some(t.range.start);
                }
                last_end = t.range.end;
            }
        }
    }
    push(&mut out, &mut first, last_end, last_end);
    out
}

/// Index of the statement Ctrl+R would run for a cursor at byte offset `pos`.
pub fn stmt_at(stmts: &[Stmt], pos: usize) -> Option<usize> {
    if stmts.is_empty() {
        return None;
    }
    if let Some(i) = stmts.iter().position(|s| pos >= s.full.start && pos <= s.full.end) {
        return Some(i);
    }
    // between statements: previous one, else next
    match stmts.iter().rposition(|s| s.full.end < pos) {
        Some(i) => Some(i),
        None => Some(0),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Select,
    Explain { analyze: bool },
    Show,
    Insert,
    Update,
    Delete,
    Merge,
    Create,
    Alter,
    Drop,
    Truncate,
    Begin,
    Commit,
    Rollback,
    Set,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub kind: Kind,
    /// The verb word as written (lowercase), used for messages like `UPDATE 42`.
    pub verb: String,
    pub has_where: bool,
    pub has_limit: bool,
    pub locking: bool,
    pub select_into: bool,
}

impl Info {
    /// Whether this statement can write data or schema.
    pub fn is_write(&self) -> bool {
        match self.kind {
            Kind::Select => self.locking || self.select_into,
            Kind::Show | Kind::Set | Kind::Begin | Kind::Commit | Kind::Rollback => false,
            Kind::Explain { analyze } => analyze,
            _ => true,
        }
    }
    pub fn unfiltered_write(&self) -> bool {
        matches!(self.kind, Kind::Update | Kind::Delete) && !self.has_where
    }
}

pub fn classify(stmt: &str) -> Info {
    let toks = tokenize(stmt);
    let words: Vec<&Token> = toks.iter().filter(|t| t.significant()).collect();
    let w = |i: usize| words.get(i).filter(|t| t.kind == Tok::Word).map(|t| t.text(stmt).to_ascii_lowercase());
    let mut info = Info {
        kind: Kind::Other,
        verb: String::new(),
        has_where: false,
        has_limit: false,
        locking: false,
        select_into: false,
    };
    // skip leading parens: (select ...)
    let mut first = 0;
    while words.get(first).is_some_and(|t| t.text(stmt) == "(") {
        first += 1;
    }
    let Some(head) = w(first) else { return info };
    let base_depth = words[first].depth;
    let mut verb_idx = first;
    let mut verb = head.clone();
    if head == "with" {
        // main verb: first depth-0 DML/select keyword after the CTE list
        if let Some((i, t)) = words.iter().enumerate().skip(first + 1).find(|(_, t)| {
            t.depth == base_depth
                && t.kind == Tok::Word
                && matches!(t.text(stmt).to_ascii_lowercase().as_str(), "select" | "insert" | "update" | "delete" | "merge")
        }) {
            verb_idx = i;
            verb = t.text(stmt).to_ascii_lowercase();
        }
    }
    info.kind = match verb.as_str() {
        "select" | "values" | "table" => Kind::Select,
        "explain" | "describe" | "desc" => {
            let analyze = words.iter().skip(verb_idx + 1).take(6).any(|t| t.is_word(stmt, "analyze") || t.is_word(stmt, "analyse"));
            if verb == "explain" && words.len() > verb_idx + 1 && !analyze {
                // EXPLAIN on its own is a read regardless of target
            }
            Kind::Explain { analyze }
        }
        "show" | "pragma" => Kind::Show,
        "insert" | "replace" | "copy" | "load" => Kind::Insert,
        "update" => Kind::Update,
        "delete" => Kind::Delete,
        "merge" | "upsert" => Kind::Merge,
        "create" => Kind::Create,
        "alter" | "rename" | "comment" => Kind::Alter,
        "drop" => Kind::Drop,
        "truncate" => Kind::Truncate,
        "begin" | "start" => Kind::Begin,
        "commit" | "end" => Kind::Commit,
        "rollback" | "abort" => Kind::Rollback,
        "set" | "reset" | "use" => Kind::Set,
        _ => Kind::Other,
    };
    if verb == "begin" && w(verb_idx + 1).is_some_and(|n| n != "transaction" && n != "work" && n != "isolation") {
        info.kind = Kind::Other; // plpgsql-ish block
    }
    info.verb = verb;
    for (i, t) in words.iter().enumerate().skip(verb_idx + 1) {
        if t.depth != base_depth || t.kind != Tok::Word {
            continue;
        }
        let lw = t.text(stmt).to_ascii_lowercase();
        match lw.as_str() {
            "where" => info.has_where = true,
            "limit" | "fetch" => info.has_limit = true,
            "into" if info.kind == Kind::Select => info.select_into = true,
            "for" if info.kind == Kind::Select => {
                if w(i + 1).is_some_and(|n| matches!(n.as_str(), "update" | "share" | "no" | "key")) {
                    info.locking = true;
                }
            }
            "lock" if info.kind == Kind::Select => info.locking = true,
            _ => {}
        }
    }
    info
}

/// Byte offset just after the last significant token, so appended text lands
/// before trailing comments and semicolons.
fn significant_end(stmt: &str) -> usize {
    tokenize(stmt)
        .iter()
        .rev()
        .find(|t| t.significant() && t.kind != Tok::Semi)
        .map(|t| t.range.end)
        .unwrap_or(0)
}

/// Text of the statement without the trailing semicolon and trailing comments.
pub fn trimmed(stmt: &str) -> &str {
    &stmt[..significant_end(stmt)]
}

/// If this is a plain SELECT without its own LIMIT, return it with `limit cap+1`
/// appended (the extra row tells us whether there was more).
pub fn apply_cap(stmt: &str, cap: usize) -> Option<String> {
    let info = classify(stmt);
    if info.kind != Kind::Select || info.has_limit || info.locking || info.select_into {
        return None;
    }
    let head = tokenize(stmt).into_iter().find(|t| t.significant())?;
    let hw = head.text(stmt).to_ascii_lowercase();
    if hw != "select" && hw != "with" && hw != "(" {
        return None;
    }
    Some(format!("{} limit {}", trimmed(stmt), cap + 1))
}

/// Wrap a query so the server sorts it. `order` is (1-based column position, descending).
pub fn wrap_sorted(stmt: &str, order: &[(usize, bool)], cap: Option<usize>) -> String {
    let mut s = format!("select * from ({}) as _zdb_sorted", trimmed(stmt));
    if !order.is_empty() {
        let parts: Vec<String> = order.iter().map(|(c, d)| format!("{}{}", c, if *d { " desc" } else { "" })).collect();
        s.push_str(" order by ");
        s.push_str(&parts.join(", "));
    }
    if let Some(c) = cap {
        s.push_str(&format!(" limit {}", c + 1));
    }
    s
}

/// Distinct parameter names in order of appearance (`:name` or `$1`).
pub fn params(stmt: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tokenize(stmt) {
        if t.kind == Tok::Param {
            let name = t.text(stmt).to_string();
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Replace parameters with literal values. Numbers, `null`, `true`/`false` and
/// already-quoted values are inserted as typed; anything else becomes a string literal.
pub fn substitute(stmt: &str, values: &dyn Fn(&str) -> Option<String>) -> String {
    let toks = tokenize(stmt);
    let mut out = String::with_capacity(stmt.len());
    let mut last = 0;
    for t in toks.iter().filter(|t| t.kind == Tok::Param) {
        out.push_str(&stmt[last..t.range.start]);
        match values(t.text(stmt)) {
            Some(v) => out.push_str(&param_literal(&v)),
            None => out.push_str(t.text(stmt)),
        }
        last = t.range.end;
    }
    out.push_str(&stmt[last..]);
    out
}

pub fn param_literal(v: &str) -> String {
    let t = v.trim();
    let lower = t.to_ascii_lowercase();
    let numeric = t.parse::<f64>().is_ok() && !t.is_empty();
    let quoted = t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'');
    if numeric || quoted || matches!(lower.as_str(), "null" | "true" | "false" | "default") {
        t.to_string()
    } else {
        quote_literal(v)
    }
}

pub fn quote_literal(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}

/// Tables referenced at depth 0 of a statement after FROM / JOIN / UPDATE / INTO,
/// with their alias if one is given. Names are returned unquoted.
pub fn table_refs(stmt: &str) -> Vec<(String, Option<String>)> {
    let toks: Vec<Token> = tokenize(stmt).into_iter().filter(|t| t.significant()).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let lw = if t.kind == Tok::Word { t.text(stmt).to_ascii_lowercase() } else { String::new() };
        if matches!(lw.as_str(), "from" | "join" | "update" | "into" | "table") {
            let mut j = i + 1;
            loop {
                let (name, next) = read_qualified(stmt, &toks, j);
                let Some(name) = name else { break };
                j = next;
                let mut alias = None;
                if toks.get(j).is_some_and(|t| t.is_word(stmt, "as")) {
                    j += 1;
                }
                if let Some(a) = toks.get(j)
                    && ((a.kind == Tok::Word && !is_keyword(a.text(stmt))) || a.kind == Tok::QuotedIdent) {
                        alias = Some(unquote(a.text(stmt)));
                        j += 1;
                    }
                out.push((name, alias));
                if lw == "from" && toks.get(j).is_some_and(|t| t.text(stmt) == ",") {
                    j += 1;
                    continue;
                }
                break;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn read_qualified(src: &str, toks: &[Token], mut j: usize) -> (Option<String>, usize) {
    let mut parts = Vec::new();
    loop {
        match toks.get(j) {
            Some(t) if (t.kind == Tok::Word && !is_keyword(t.text(src))) || t.kind == Tok::QuotedIdent => {
                parts.push(unquote(t.text(src)));
                j += 1;
                if toks.get(j).is_some_and(|t| t.text(src) == ".") {
                    j += 1;
                    continue;
                }
                break;
            }
            _ => break,
        }
    }
    if parts.is_empty() { (None, j) } else { (Some(parts.join(".")), j) }
}

pub fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'`') && b[b.len() - 1] == b[0] {
        let q = b[0] as char;
        s[1..s.len() - 1].replace(&format!("{q}{q}"), &q.to_string())
    } else {
        s.to_string()
    }
}

/// A short label for a result: the first table read from, or the verb.
pub fn result_label(stmt: &str) -> String {
    let info = classify(stmt);
    if let Some((t, _)) = table_refs(stmt).into_iter().next() {
        return t.rsplit('.').next().unwrap_or(&t).to_string();
    }
    if info.verb.is_empty() { "result".into() } else { info.verb }
}

pub const KEYWORDS: &[&str] = &[
    "abort", "add", "all", "alter", "analyze", "and", "any", "array", "as", "asc", "begin", "between", "by", "call",
    "cascade", "case", "cast", "check", "column", "commit", "constraint", "copy", "create", "cross", "current_date",
    "current_timestamp", "database", "default", "delete", "desc", "describe", "distinct", "do", "drop", "else", "end",
    "except", "exists", "explain", "false", "fetch", "filter", "first", "for", "foreign", "from", "full", "grant",
    "group", "having", "if", "ilike", "in", "index", "inner", "insert", "intersect", "interval", "into", "is", "join",
    "key", "last", "lateral", "left", "like", "limit", "materialized", "merge", "natural", "not", "null", "nulls",
    "offset", "on", "only", "or", "order", "outer", "over", "partition", "primary", "procedure", "recursive",
    "references", "rename", "replace", "returning", "revoke", "right", "rollback", "row", "rows", "savepoint",
    "schema", "select", "set", "show", "table", "then", "to", "transaction", "true", "truncate", "union", "unique",
    "update", "use", "using", "values", "view", "when", "where", "window", "with", "within",
];

pub fn is_keyword(w: &str) -> bool {
    let lw = w.to_ascii_lowercase();
    KEYWORDS.binary_search(&lw.as_str()).is_ok()
}

pub const FUNCTIONS: &[&str] = &[
    "abs", "array_agg", "avg", "coalesce", "concat", "count", "current_setting", "date_part", "date_trunc", "extract",
    "format", "generate_series", "greatest", "json_agg", "json_build_object", "jsonb_agg", "jsonb_build_object",
    "least", "length", "lower", "max", "min", "now", "nullif", "rank", "round", "row_number", "string_agg",
    "substring", "sum", "to_char", "to_date", "trim", "upper",
];

/// Lightweight formatter: lowercase keywords, one clause per line, select-list
/// items and where conditions each on their own indented line.
pub fn format(src: &str) -> String {
    let stmts = split(src, false);
    if stmts.is_empty() {
        return src.to_string();
    }
    let mut out = Vec::new();
    for s in &stmts {
        let text = &src[s.range.clone()];
        out.push(format!("{};", format_one(text)));
    }
    out.join("\n\n") + "\n"
}

fn format_one(src: &str) -> String {
    let toks: Vec<Token> = tokenize(src).into_iter().filter(|t| t.kind != Tok::Space).collect();
    let mut out = String::new();
    let mut indent: Vec<usize> = vec![0];
    let mut paren_breaks: Vec<bool> = Vec::new();
    let mut clause = String::new();
    let mut line_start = true;
    let newline = |out: &mut String, n: usize| {
        while out.ends_with(' ') {
            out.pop();
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&" ".repeat(n));
    };
    let mut prev: Option<&Token> = None;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        let text = t.text(src);
        let lw = text.to_ascii_lowercase();
        let base = *indent.last().unwrap();
        let next_word = toks.get(i + 1).map(|n| n.text(src).to_ascii_lowercase()).unwrap_or_default();
        let clause_start = t.kind == Tok::Word
            && (matches!(lw.as_str(), "select" | "from" | "where" | "having" | "limit" | "offset" | "returning" | "values" | "set" | "union" | "intersect" | "except")
                || (matches!(lw.as_str(), "group" | "order") && next_word == "by")
                || lw == "join"
                    && !prev.is_some_and(|p| {
                        matches!(p.text(src).to_ascii_lowercase().as_str(), "left" | "right" | "inner" | "full" | "cross" | "outer" | "natural")
                    })
                || matches!(lw.as_str(), "left" | "right" | "inner" | "full" | "cross" | "natural") && matches!(next_word.as_str(), "join" | "outer"));
        let mut text_out = if t.kind == Tok::Word && is_keyword(text) { lw.clone() } else { text.to_string() };
        if t.kind == Tok::Word && FUNCTIONS.contains(&lw.as_str()) {
            text_out = lw.clone();
        }
        if clause_start {
            if !out.is_empty() {
                newline(&mut out, base);
            }
            line_start = true;
            clause = lw.clone();
        } else if t.kind == Tok::Word && (lw == "and" || lw == "or") && clause == "where" && paren_breaks.iter().all(|b| !b) {
            newline(&mut out, base + 2);
            line_start = true;
        }
        match text {
            "(" => {
                let sub = matches!(next_word.as_str(), "select" | "with");
                if !line_start && !prev.is_some_and(|p| p.kind == Tok::Word && !is_keyword(p.text(src)) || p.text(src) == "(") {
                    out.push(' ');
                }
                out.push('(');
                paren_breaks.push(sub);
                if sub {
                    indent.push(base + 4);
                    newline(&mut out, base + 4);
                    line_start = true;
                } else {
                    line_start = true;
                }
                prev = Some(t);
                i += 1;
                continue;
            }
            ")" => {
                if paren_breaks.pop() == Some(true) {
                    indent.pop();
                    newline(&mut out, *indent.last().unwrap());
                }
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(')');
                line_start = false;
                prev = Some(t);
                i += 1;
                continue;
            }
            "," => {
                out.push(',');
                if t.depth == 0 && matches!(clause.as_str(), "select" | "group" | "order" | "set" | "returning")
                    || paren_breaks.last() == Some(&true) && false
                {
                    newline(&mut out, base + 2);
                    line_start = true;
                } else {
                    out.push(' ');
                    line_start = true;
                }
                prev = Some(t);
                i += 1;
                continue;
            }
            "." | "::" => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push_str(text);
                line_start = true;
                prev = Some(t);
                i += 1;
                continue;
            }
            _ => {}
        }
        if !line_start && !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&text_out);
        line_start = false;
        if t.kind == Tok::Comment && text.starts_with("--") {
            newline(&mut out, base);
            line_start = true;
        }
        // select list starts on the next line, indented
        if clause_start && matches!(lw.as_str(), "select") {
            let distinct = next_word == "distinct";
            if !distinct {
                newline(&mut out, base + 2);
                line_start = true;
            }
        } else if t.kind == Tok::Word && lw == "distinct" && clause == "select" {
            newline(&mut out, base + 2);
            line_start = true;
        }
        prev = Some(t);
        i += 1;
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_sorted() {
        let mut k = KEYWORDS.to_vec();
        k.sort();
        assert_eq!(k, KEYWORDS);
    }

    #[test]
    fn splits_on_semicolons_and_blank_lines() {
        let src = "select 1;\nselect 2\n\nselect 'a;b'; -- tail\n";
        let s = split(src, true);
        let texts: Vec<&str> = s.iter().map(|s| &src[s.range.clone()]).collect();
        assert_eq!(texts, vec!["select 1", "select 2", "select 'a;b'"]);
        let s = split(src, false);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn blank_line_inside_parens_does_not_split() {
        let src = "select (\n\n1)";
        assert_eq!(split(src, true).len(), 1);
    }

    #[test]
    fn stmt_at_cursor() {
        let src = "select 1;\n\nselect 2;";
        let s = split(src, true);
        assert_eq!(stmt_at(&s, 3), Some(0));
        assert_eq!(stmt_at(&s, 9), Some(0)); // right after ;
        assert_eq!(stmt_at(&s, 10), Some(0)); // gap
        assert_eq!(stmt_at(&s, 13), Some(1));
    }

    #[test]
    fn classify_writes() {
        assert!(classify("update t set a=1").unfiltered_write());
        assert!(!classify("update t set a=1 where id=2").unfiltered_write());
        assert!(classify("delete from t").unfiltered_write());
        assert!(!classify("delete from t where (a)=1").unfiltered_write());
        assert!(classify("with x as (select 1 where true) delete from t").unfiltered_write());
        assert_eq!(classify("WITH x AS (select 1) SELECT * from x").kind, Kind::Select);
        assert!(!classify("select * from t where a in (select b from c limit 1)").has_limit);
        assert!(classify("select * from t for update").is_write());
        assert!(!classify("explain select 1").is_write());
        assert!(classify("explain analyze delete from t").is_write());
        assert_eq!(classify("DROP TABLE x").kind, Kind::Drop);
        assert_eq!(classify("  -- hi\n truncate x").kind, Kind::Truncate);
    }

    #[test]
    fn caps_selects_only() {
        assert_eq!(apply_cap("select * from t; -- x", 1000).unwrap(), "select * from t limit 1001");
        assert!(apply_cap("select * from t limit 5", 1000).is_none());
        assert!(apply_cap("update t set a = 1", 1000).is_none());
        assert!(apply_cap("values (1)", 1000).is_none());
    }

    #[test]
    fn params_and_casts() {
        let s = "select $1, :start_date, a::text, x := 1, ':no' from t where b = :start_date";
        assert_eq!(params(s), vec!["$1", ":start_date"]);
        let out = substitute(s, &|n| Some(if n == "$1" { "42".into() } else { "2026-09-01".into() }));
        assert_eq!(out, "select 42, '2026-09-01', a::text, x := 1, ':no' from t where b = '2026-09-01'");
    }

    #[test]
    fn dollar_quotes() {
        let s = "select $tag$ a; b $tag$; select 2";
        assert_eq!(split(s, false).len(), 2);
    }

    #[test]
    fn refs_with_aliases() {
        let r = table_refs("select * from public.orders o join users as u on u.id = o.user_id, items");
        assert_eq!(
            r,
            vec![
                ("public.orders".to_string(), Some("o".to_string())),
                ("users".to_string(), Some("u".to_string())),
            ]
        );
        let r = table_refs("select * from a, b x where 1=1");
        assert_eq!(r, vec![("a".into(), None), ("b".into(), Some("x".into()))]);
    }

    #[test]
    fn formats() {
        let f = format("SELECT a, b FROM t WHERE a = 1 AND b IN (SELECT x FROM y) ORDER BY 1");
        assert_eq!(
            f,
            "select\n  a,\n  b\nfrom t\nwhere a = 1\n  and b in (\n    select\n      x\n    from y\n)\norder by 1;\n"
        );
    }
}
