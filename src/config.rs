//! Configuration files: connections.toml (projects and environments) and config.toml (settings).
//! Both live in the XDG config directory; nothing secret is ever written here.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("ZDB_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("zdb"),
        _ => dirs::home_dir().unwrap_or_default().join(".config").join("zdb"),
    }
}

pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("ZDB_DATA_DIR") {
        return PathBuf::from(d);
    }
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("zdb"),
        _ => dirs::home_dir().unwrap_or_default().join(".local").join("share").join("zdb"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Local,
    Staging,
    Prod,
}

impl Level {
    pub fn infer(env_name: &str) -> Level {
        let n = env_name.to_ascii_lowercase();
        if n.starts_with("prod") || n == "live" || n == "prd" {
            Level::Prod
        } else if n.starts_with("stag") || n == "stg" || n == "qa" || n == "uat" || n == "test" {
            Level::Staging
        } else {
            Level::Local
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Level::Local => "local",
            Level::Staging => "staging",
            Level::Prod => "prod",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    Postgres,
    Mysql,
    Sqlite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvConfig {
    pub name: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<Level>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// keychain (default) · prompt · env:VAR · pgpass · mycnf · none
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// SSH target for a tunnel, e.g. `deploy@bastion.example.com` (uses your ssh config and agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
}

impl EnvConfig {
    pub fn level(&self) -> Level {
        self.level.unwrap_or_else(|| Level::infer(&self.name))
    }
    pub fn read_only(&self) -> bool {
        self.read_only.unwrap_or(self.level() == Level::Prod)
    }
    pub fn password_source(&self) -> &str {
        self.password.as_deref().unwrap_or("keychain")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    #[serde(default, rename = "env")]
    pub envs: Vec<EnvConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connections {
    #[serde(default, rename = "project")]
    pub projects: Vec<ProjectConfig>,
}

pub const SCRATCH: &str = "Scratch";

impl Connections {
    pub fn path() -> PathBuf {
        config_dir().join("connections.toml")
    }
    pub fn load() -> Result<Connections> {
        let p = Self::path();
        if !p.exists() {
            return Ok(Connections::default());
        }
        let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }
    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        let mut saved = self.clone();
        saved.projects.retain(|p| p.name != SCRATCH);
        let body = toml::to_string_pretty(&saved)?;
        let text = format!(
            "# zdb connections. Safe to commit: passwords live in the OS keychain or come from\n# env:VAR, ~/.pgpass or ~/.my.cnf (set `password = \"…\"` to choose).\n\n{body}"
        );
        let tmp = p.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, p)?;
        Ok(())
    }
    pub fn project(&self, name: &str) -> Option<&ProjectConfig> {
        self.projects.iter().find(|p| p.name == name)
    }
    pub fn find(&self, project: &str, env: &str) -> Option<&EnvConfig> {
        self.project(project)?.envs.iter().find(|e| e.name == env)
    }
    pub fn upsert(&mut self, project: &str, env: EnvConfig, replacing: Option<(&str, &str)>) {
        if let Some((op, oe)) = replacing {
            if let Some(p) = self.projects.iter_mut().find(|p| p.name == op) {
                p.envs.retain(|e| e.name != oe);
            }
            self.projects.retain(|p| !p.envs.is_empty() || p.name == project);
        }
        let idx = match self.projects.iter().position(|p| p.name == project) {
            Some(i) => i,
            None => {
                self.projects.push(ProjectConfig { name: project.to_string(), envs: vec![] });
                self.projects.len() - 1
            }
        };
        let envs = &mut self.projects[idx].envs;
        match envs.iter().position(|e| e.name == env.name) {
            Some(i) => envs[i] = env,
            None => {
                envs.push(env);
                // keep local · staging · prod order
                envs.sort_by_key(|e| e.level());
            }
        }
    }
    pub fn add_scratch(&mut self, url: &str) -> (String, String) {
        let env = EnvConfig {
            name: "scratch".into(),
            url: url.to_string(),
            level: Some(Level::Local),
            read_only: None,
            password: Some("prompt".into()),
            ssh: None,
        };
        self.upsert(SCRATCH, env, None);
        (SCRATCH.into(), "scratch".into())
    }
}

impl PartialOrd for Level {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Level {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

/// A parsed connection URL. The form edits these fields and rebuilds the URL.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UrlParts {
    pub scheme: String,
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: String,
    pub database: String,
    pub query: String,
}

impl UrlParts {
    pub fn parse(s: &str) -> Result<UrlParts> {
        if let Some(path) = s.trim().strip_prefix("sqlite://") {
            if path.is_empty() { bail!("SQLite URL needs a file path or :memory:"); }
            return Ok(UrlParts { scheme: "sqlite".into(), database: pct_decode(path), ..Default::default() });
        }
        let u = url::Url::parse(s.trim()).map_err(|e| anyhow!("invalid URL: {e}"))?;
        let scheme = u.scheme().to_string();
        if driver_for_scheme(&scheme).is_none() {
            bail!("unsupported scheme {scheme}:// (use postgres://, mysql:// or sqlite://)");
        }
        Ok(UrlParts {
            scheme,
            user: pct_decode(u.username()),
            password: u.password().map(pct_decode).unwrap_or_default(),
            host: u.host_str().unwrap_or("").trim_matches(['[', ']']).to_string(),
            port: u.port().map(|p| p.to_string()).unwrap_or_default(),
            database: pct_decode(u.path().trim_start_matches('/')),
            query: u.query().unwrap_or("").to_string(),
        })
    }
    /// Rebuild the URL, never including the password.
    pub fn build(&self) -> String {
        if self.driver() == Some(DriverKind::Sqlite) {
            return format!("sqlite://{}", self.database.split('/').map(pct_encode).collect::<Vec<_>>().join("/"));
        }
        let scheme = if self.scheme.is_empty() { "postgres" } else { &self.scheme };
        let mut s = format!("{scheme}://");
        if !self.user.is_empty() {
            s.push_str(&pct_encode(&self.user));
            s.push('@');
        }
        if self.host.contains(':') {
            s.push_str(&format!("[{}]", self.host));
        } else {
            s.push_str(&self.host);
        }
        if !self.port.is_empty() {
            s.push(':');
            s.push_str(&self.port);
        }
        if !self.database.is_empty() {
            s.push('/');
            s.push_str(&pct_encode(&self.database));
        }
        if !self.query.is_empty() {
            s.push('?');
            s.push_str(&self.query);
        }
        s
    }
    pub fn driver(&self) -> Option<DriverKind> {
        driver_for_scheme(&self.scheme)
    }
    pub fn port_or_default(&self) -> u16 {
        self.port.parse().unwrap_or(match self.driver() {
            Some(DriverKind::Mysql) => 3306,
            _ => 5432,
        })
    }
    pub fn param(&self, key: &str) -> Option<String> {
        self.query.split('&').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| pct_decode(v))
    }
    pub fn display(&self) -> String {
        let mut p = self.clone();
        p.password.clear();
        p.query.clear();
        p.build()
    }
}

pub fn driver_for_scheme(s: &str) -> Option<DriverKind> {
    match s {
        "postgres" | "postgresql" | "pg" => Some(DriverKind::Postgres),
        "mysql" | "mariadb" => Some(DriverKind::Mysql),
        "sqlite" => Some(DriverKind::Sqlite),
        _ => None,
    }
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || "-._~".contains(c) {
            out.push(c);
        } else {
            let mut buf = [0; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MenuMode {
    Delay,
    Instant,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    Comfortable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Cap for SELECTs without a LIMIT; 0 means no cap.
    pub row_limit: usize,
    /// Rows fetched when browsing a table.
    pub browse_rows: usize,
    pub space_menu: MenuMode,
    pub space_menu_delay_ms: u64,
    pub vim: bool,
    pub density: Density,
    pub ascii: bool,
    pub mouse: bool,
    pub key_hints: bool,
    pub prod_unlock_idle_minutes: u64,
    pub blank_line_splits: bool,
    pub group_digits: bool,
    pub toast_ms: u64,
    pub slow_query_secs: u64,
    pub kitty_keyboard: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            row_limit: 1000,
            browse_rows: 500,
            space_menu: MenuMode::Delay,
            space_menu_delay_ms: 300,
            vim: false,
            density: Density::Compact,
            ascii: false,
            mouse: true,
            key_hints: true,
            prod_unlock_idle_minutes: 30,
            blank_line_splits: true,
            group_digits: true,
            toast_ms: 2000,
            slow_query_secs: 10,
            kitty_keyboard: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }
    pub fn load() -> Result<Settings> {
        let p = Self::path();
        if !p.exists() {
            return Ok(Settings::default());
        }
        let text = std::fs::read_to_string(&p)?;
        toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))
    }
    pub fn save(&self) -> Result<()> {
        let p = Self::path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_roundtrip() {
        let p = UrlParts::parse("postgres://app@db.staging:5432/shop?sslmode=require").unwrap();
        assert_eq!(p.host, "db.staging");
        assert_eq!(p.user, "app");
        assert_eq!(p.port, "5432");
        assert_eq!(p.database, "shop");
        assert_eq!(p.param("sslmode").as_deref(), Some("require"));
        assert_eq!(p.build(), "postgres://app@db.staging:5432/shop?sslmode=require");
        let p = UrlParts::parse("mysql://r%40t:pw@localhost/app").unwrap();
        assert_eq!(p.user, "r@t");
        assert_eq!(p.password, "pw");
        assert_eq!(p.build(), "mysql://r%40t@localhost/app");
        assert!(UrlParts::parse("redis://x").is_err());
    }

    #[test]
    fn levels_and_toml() {
        let text = r#"
[[project]]
name = "shop-api"
  [[project.env]]
  name = "local"
  url = "postgres://localhost/shop"
  [[project.env]]
  name = "prod"
  url = "postgres://db.prod/shop"
"#;
        let c: Connections = toml::from_str(text).unwrap();
        let prod = c.find("shop-api", "prod").unwrap();
        assert_eq!(prod.level(), Level::Prod);
        assert!(prod.read_only());
        assert!(!c.find("shop-api", "local").unwrap().read_only());
        let back: Connections = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn upsert_keeps_level_order() {
        let mut c = Connections::default();
        let mk = |n: &str| EnvConfig { name: n.into(), url: "postgres://x/y".into(), level: None, read_only: None, password: None, ssh: None };
        c.upsert("p", mk("prod"), None);
        c.upsert("p", mk("local"), None);
        c.upsert("p", mk("staging"), None);
        let names: Vec<_> = c.project("p").unwrap().envs.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["local", "staging", "prod"]);
    }
}
