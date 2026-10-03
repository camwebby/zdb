//! Password lookup. Secrets never touch connections.toml: they come from the OS keychain,
//! an environment variable, ~/.pgpass, ~/.my.cnf, or a prompt once per session.

use crate::config::{EnvConfig, UrlParts};

const SERVICE: &str = "zdb";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Found(String),
    /// No password configured (trust/peer auth, or the URL has none and source is `none`).
    NotNeeded,
    /// Ask the user; `store_in_keychain` says whether to save what they type.
    Prompt { store_in_keychain: bool },
}

fn account(project: &str, env: &str) -> String {
    format!("{project}/{env}")
}

pub fn keychain_get(project: &str, env: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, &account(project, env)).ok()?.get_password().ok()
}

pub fn keychain_set(project: &str, env: &str, password: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, &account(project, env))
        .and_then(|e| e.set_password(password))
        .map_err(|e| format!("keychain unavailable: {e}"))
}

pub fn keychain_delete(project: &str, env: &str) {
    if let Ok(e) = keyring::Entry::new(SERVICE, &account(project, env)) {
        let _ = e.delete_credential();
    }
}

pub fn lookup(project: &str, env: &EnvConfig, parts: &UrlParts) -> Lookup {
    if parts.driver() == Some(crate::config::DriverKind::Sqlite) { return Lookup::NotNeeded; }
    if !parts.password.is_empty() {
        return Lookup::Found(parts.password.clone());
    }
    let src = env.password_source();
    if let Some(var) = src.strip_prefix("env:") {
        return match std::env::var(var) {
            Ok(v) => Lookup::Found(v),
            Err(_) => Lookup::Prompt { store_in_keychain: false },
        };
    }
    match src {
        "none" => Lookup::NotNeeded,
        "prompt" => Lookup::Prompt { store_in_keychain: false },
        "pgpass" => match pgpass(parts) {
            Some(p) => Lookup::Found(p),
            None => Lookup::Prompt { store_in_keychain: false },
        },
        "mycnf" => match mycnf() {
            Some(p) => Lookup::Found(p),
            None => Lookup::Prompt { store_in_keychain: false },
        },
        _ => {
            if let Some(p) = keychain_get(project, &env.name) {
                return Lookup::Found(p);
            }
            // fall back to the standard files before asking
            if let Some(p) = pgpass(parts).or_else(|| if parts.driver() == Some(crate::config::DriverKind::Mysql) { mycnf() } else { None }) {
                return Lookup::Found(p);
            }
            Lookup::Prompt { store_in_keychain: true }
        }
    }
}

fn pgpass(parts: &UrlParts) -> Option<String> {
    let path = std::env::var_os("PGPASSFILE").map(Into::into).or_else(|| dirs::home_dir().map(|h| h.join(".pgpass")))?;
    let text = std::fs::read_to_string(path).ok()?;
    pgpass_match(&text, &parts.host, &parts.port_or_default().to_string(), &parts.database, &parts.user)
}

pub fn pgpass_match(text: &str, host: &str, port: &str, db: &str, user: &str) -> Option<String> {
    for line in text.lines() {
        if line.trim_start().starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields = split_pgpass(line);
        if fields.len() != 5 {
            continue;
        }
        let m = |pat: &str, v: &str| pat == "*" || pat == v;
        if m(&fields[0], host) && m(&fields[1], port) && m(&fields[2], db) && m(&fields[3], user) {
            return Some(fields[4].clone());
        }
    }
    None
}

fn split_pgpass(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ':' if out.len() < 5 => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

fn mycnf() -> Option<String> {
    let text = std::fs::read_to_string(dirs::home_dir()?.join(".my.cnf")).ok()?;
    mycnf_password(&text)
}

pub fn mycnf_password(text: &str) -> Option<String> {
    let mut in_client = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_client = l == "[client]" || l == "[mysql]";
            continue;
        }
        if in_client
            && let Some((k, v)) = l.split_once('=')
                && k.trim() == "password" {
                    return Some(v.trim().trim_matches(['"', '\'']).to_string());
                }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pgpass_wildcards_and_escapes() {
        let t = "# comment\ndb.staging:5432:shop:app:s3cr\\:et\n*:*:*:admin:root\n";
        assert_eq!(pgpass_match(t, "db.staging", "5432", "shop", "app").as_deref(), Some("s3cr:et"));
        assert_eq!(pgpass_match(t, "x", "1", "y", "admin").as_deref(), Some("root"));
        assert_eq!(pgpass_match(t, "x", "1", "y", "app"), None);
    }

    #[test]
    fn mycnf_client_section() {
        let t = "[mysqld]\npassword=no\n[client]\nuser=me\npassword = \"yes\"\n";
        assert_eq!(mycnf_password(t).as_deref(), Some("yes"));
    }
}
