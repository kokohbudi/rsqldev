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
    /// Rows fetched per round trip (SQL Developer default 50).
    pub fetch_size: usize,
    pub autocommit: bool,
    /// Applied to Oracle sessions after connect.
    pub nls_date_format: String,
    /// Per-OS custom client library folder by driver id (overrides download).
    pub client_dirs: BTreeMap<String, String>,
    /// "light" (SQL Developer default), "dark" or "system".
    pub theme: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            editor_font: if cfg!(windows) { "Consolas".into() } else { "Menlo".into() },
            editor_font_size: 12,
            fetch_size: 50,
            autocommit: false,
            nls_date_format: "DD-MON-RR".into(),
            client_dirs: BTreeMap::new(),
            theme: "light".into(),
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
