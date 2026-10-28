//! Saved connections and preferences (`<config_dir>/rsqldev/*.toml`); passwords live in the OS keychain.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct ConnConfig {
    pub name: String,
    /// Registry driver id.
    pub driver: String,
    /// Connections-tree folder ("" = root).
    pub folder: String,
    pub host: String,
    pub port: Option<u16>,
    /// Service name / database / file path, depending on driver.
    pub database: String,
    pub user: String,
    pub save_password: bool,
    /// Oracle: "basic" (default), "tns", "custom".
    pub conn_type: String,
    /// Oracle basic: treat `database` as SID instead of service name.
    pub sid: bool,
    /// Oracle: TNS alias or full connect descriptor for custom.
    pub tns: String,
    /// Oracle: SYSDBA, SYSOPER, ... ("" = default).
    pub role: String,
    /// Driver-specific options (sslmode, encrypt, trust_cert, ...).
    pub options: BTreeMap<String, String>,
    /// Connections-tree name filters, keyed by [`filter_key`] (owner + object kind).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub filters: BTreeMap<String, String>,
}

/// Key of a tree folder's filter in [`ConnConfig::filters`].
pub fn filter_key(owner: &str, kind: &str) -> String {
    format!("{owner}/{kind}")
}

/// Navigator filter match, case-insensitive. `%` / `*` = any run, `_` = one character;
/// a pattern without wildcards matches names containing it.
pub fn name_matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.trim().to_uppercase().chars().map(|c| if c == '*' { '%' } else { c }).collect();
    let p = if p.iter().any(|c| *c == '%' || *c == '_') { p } else { [&['%'][..], &p, &['%']].concat() };
    let n: Vec<char> = name.to_uppercase().chars().collect();
    // Greedy wildcard match, backtracking to the last `%`.
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == '_' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '%' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '%')
}

#[derive(Serialize, Deserialize, Default)]
struct ConnFile {
    #[serde(default)]
    connection: Vec<ConnConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Prefs {
    pub editor_font: String,
    pub editor_font_size: i32,
    pub autocommit: bool,
    /// Data editor: post a row's DML when the cursor leaves it (Commit then only commits).
    pub post_on_row_change: bool,
    /// Applied to Oracle sessions after connect.
    pub nls_date_format: String,
    /// Per-OS custom client library folder by driver id (overrides download).
    pub client_dirs: BTreeMap<String, String>,
    /// "light" (SQL Developer default), "dark" or "system".
    pub theme: String,
    /// Look up the latest release on start (one HTTPS request to GitHub).
    pub check_updates: bool,
    /// Result grids: text shown for NULL (typing it back sets NULL).
    pub null_display: String,
    /// Oracle: folder with tnsnames.ora when a connection sets no tns_admin.
    pub tns_dir: String,
    /// Connect from the connection dialog opens a worksheet (else only the tree expands).
    pub open_worksheet_on_connect: bool,
    pub history_limit: usize,
    pub line_numbers: bool,
    /// Oracle NLS_TIMESTAMP_FORMAT (TIMESTAMP WITH TIME ZONE adds TZH:TZM).
    pub nls_timestamp_format: String,
    /// Oracle DDL (DBMS_METADATA): include storage / tablespace / segment clauses.
    pub ddl_storage: bool,
    /// Keep bind values typed in worksheets across restarts (binds.toml).
    pub save_binds: bool,
    /// Reopen the worksheets left open at quit (connections with a saved password).
    pub reopen_worksheets: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            editor_font: if cfg!(windows) { "Consolas".into() } else { "Menlo".into() },
            editor_font_size: 12,
            autocommit: false,
            post_on_row_change: false,
            nls_date_format: "DD-MON-RR".into(),
            client_dirs: BTreeMap::new(),
            theme: "light".into(),
            check_updates: true,
            null_display: "(null)".into(),
            tns_dir: String::new(),
            open_worksheet_on_connect: true,
            history_limit: 500,
            line_numbers: true,
            nls_timestamp_format: "DD-MON-RR HH.MI.SSXFF AM".into(),
            ddl_storage: true,
            save_binds: true,
            reopen_worksheets: true,
        }
    }
}

/// `RSQLDEV_CONFIG_DIR` overrides the location (tests, portable installs).
pub fn config_root() -> PathBuf {
    std::env::var_os("RSQLDEV_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("rsqldev"))
}

fn read<T: for<'a> Deserialize<'a> + Default>(name: &str) -> T {
    std::fs::read_to_string(config_root().join(name)).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
}

fn write<T: Serialize>(name: &str, v: &T) -> Result<()> {
    std::fs::create_dir_all(config_root())?;
    std::fs::write(config_root().join(name), toml::to_string_pretty(v)?)?;
    Ok(())
}

pub fn load_connections() -> Vec<ConnConfig> {
    read::<ConnFile>("connections.toml").connection
}

pub fn save_connections(c: &[ConnConfig]) -> Result<()> {
    write("connections.toml", &ConnFile { connection: c.to_vec() })
}

pub fn load_prefs() -> Prefs {
    read("prefs.toml")
}

pub fn save_prefs(p: &Prefs) -> Result<()> {
    write("prefs.toml", p)
}

fn entry(conn_name: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new("rsqldev", conn_name)
}

pub fn get_password(conn_name: &str) -> Option<String> {
    entry(conn_name).ok()?.get_password().ok()
}

pub fn set_password(conn_name: &str, pw: &str) -> Result<()> {
    entry(conn_name)?.set_password(pw)?;
    Ok(())
}

pub fn delete_password(conn_name: &str) {
    if let Ok(e) = entry(conn_name) {
        let _ = e.delete_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::name_matches;

    #[test]
    fn filter_patterns() {
        assert!(name_matches("emp", "HR_EMPLOYEES"));
        assert!(!name_matches("emp", "DEPT"));
        assert!(name_matches("EMP%", "employees"));
        assert!(!name_matches("EMP%", "HR_EMPLOYEES"));
        assert!(name_matches("*_LOG", "AUDIT_LOG"));
        assert!(name_matches("T_B", "TAB"));
        assert!(!name_matches("T_B", "TB"));
        assert!(name_matches("%A%B%", "XAYYBZ"));
        assert!(!name_matches("%A%B", "XAYYBZ"));
        assert!(name_matches("", "ANY"));
    }
}
