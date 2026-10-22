//! Driver registry loaded from `drivers.toml`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Oracle,
    Postgres,
    Mysql,
    Mssql,
    Sqlite,
    Duckdb,
    Clickhouse,
}

impl Kind {
    /// File-based engines take a path instead of host/port.
    pub fn is_file(self) -> bool {
        matches!(self, Kind::Sqlite | Kind::Duckdb)
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Archive {
    Zip,
    Tgz,
    Dmg,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DriverFile {
    pub os: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub archive: Archive,
    #[serde(default)]
    pub strip: usize,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Driver {
    pub id: String,
    pub label: String,
    pub category: String,
    pub backend: Kind,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub user: Option<String>,
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    pub license: Option<String>,
    #[serde(default)]
    pub files: Vec<DriverFile>,
}

impl Driver {
    /// Native client files for the running platform, if this driver needs any.
    pub fn file_for_host(&self) -> Option<&DriverFile> {
        let host = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        self.files.iter().find(|f| f.os == host)
    }

    /// Built-in = pure-Rust backend, nothing to download.
    pub fn is_builtin(&self) -> bool {
        self.files.is_empty()
    }
}

#[derive(Deserialize)]
struct File {
    driver: Vec<Driver>,
}

pub fn drivers() -> &'static [Driver] {
    static D: OnceLock<Vec<Driver>> = OnceLock::new();
    D.get_or_init(|| toml::from_str::<File>(include_str!("../drivers.toml")).expect("drivers.toml").driver)
}

pub fn find(id: &str) -> Option<&'static Driver> {
    drivers().iter().find(|d| d.id == id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn registry_parses_and_ids_unique() {
        let d = super::drivers();
        assert!(d.len() > 30);
        let mut ids: Vec<_> = d.iter().map(|d| &d.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), d.len());
        assert!(!super::find("oracle").unwrap().is_builtin());
    }
}
