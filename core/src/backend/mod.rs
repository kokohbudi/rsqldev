//! Native backends. Each one is a thin adapter from a Rust driver crate to [`crate::Backend`].

mod clickhouse;
mod duckdb;
mod mssql;
mod mysql;
mod oracle;
mod postgres;
mod sqlite;
mod tunnel;

use crate::config::{ConnConfig, Prefs};
use crate::registry::{self, Kind};
use crate::{Backend, Cell, Event};
use anyhow::{Result, anyhow};
use std::path::PathBuf;

/// Native client libraries for a driver are not installed. The UI offers a download.
#[derive(Debug)]
pub struct DriverMissing(pub String);

impl std::fmt::Display for DriverMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "client libraries for driver '{}' are not installed", self.0)
    }
}

impl std::error::Error for DriverMissing {}

/// Client library folder: custom folder from prefs wins over the downloaded one.
pub fn client_dir(driver_id: &str, prefs: &Prefs) -> Option<PathBuf> {
    if let Some(p) = prefs.client_dirs.get(driver_id).filter(|p| !p.is_empty()) {
        return Some(p.into());
    }
    crate::download::installed_dir(registry::find(driver_id)?)
}

/// tnsnames.ora aliases (sorted, upper case) for the TNS connection type, from `tns_admin`
/// (the connection's option), $TNS_ADMIN, $ORACLE_HOME/network/admin and the Oracle client
/// folder's network/admin.
pub fn tns_aliases(tns_admin: Option<&str>, prefs: &Prefs) -> Vec<String> {
    let mut dirs: Vec<PathBuf> = tns_admin.filter(|s| !s.trim().is_empty()).map(PathBuf::from).into_iter().collect();
    dirs.extend(std::env::var_os("TNS_ADMIN").map(PathBuf::from));
    dirs.extend(std::env::var_os("ORACLE_HOME").map(|h| PathBuf::from(h).join("network").join("admin")));
    dirs.extend(client_dir("oracle", prefs).map(|c| c.join("network").join("admin")));
    let mut out: Vec<String> = dirs.iter().filter_map(|d| std::fs::read_to_string(d.join("tnsnames.ora")).ok()).flat_map(|s| parse_tnsnames(&s)).collect();
    out.sort();
    out.dedup();
    out
}

/// Alias names in tnsnames.ora text: `NAME[, NAME2] =` outside parentheses. The rest of an
/// `=` line at depth 0 (an IFILE path) is skipped.
pub fn parse_tnsnames(text: &str) -> Vec<String> {
    let (mut out, mut depth, mut head) = (vec![], 0i32, String::new());
    for line in text.lines() {
        let mut after_eq = false;
        for ch in line.split('#').next().unwrap_or("").chars() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                '=' if depth == 0 && !after_eq => {
                    out.extend(head.split(',').map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty() && s != "IFILE" && !s.contains(char::is_whitespace)));
                    head.clear();
                    after_eq = true;
                }
                c if depth == 0 && !after_eq => head.push(c),
                _ => {}
            }
        }
        if after_eq { head.clear() } else { head.push(' ') }
    }
    out
}

#[cfg(test)]
mod tns_tests {
    #[test]
    fn tnsnames_aliases() {
        let t = "# comment\nORCL =\n  (DESCRIPTION =\n    (ADDRESS = (PROTOCOL = TCP)(HOST = h)(PORT = 1521))\n    (CONNECT_DATA = (SERVICE_NAME = orcl)))\n\nIFILE = /etc/other.ora\nprod, prod.world=(DESCRIPTION=(ADDRESS=(HOST=p)))\nlast =\n (DESCRIPTION=)";
        assert_eq!(super::parse_tnsnames(t), ["ORCL", "PROD", "PROD.WORLD", "LAST"]);
    }
}

pub fn connect(cfg: &ConnConfig, password: &str, prefs: &Prefs) -> Result<Box<dyn Backend>> {
    let drv = registry::find(&cfg.driver).ok_or_else(|| anyhow!("unknown driver '{}'", cfg.driver))?;
    // SSH tunnel: connect to a forwarded local port instead of the database host.
    let ssh = opt(cfg, drv, "ssh").filter(|s| !s.trim().is_empty() && !drv.backend.is_file());
    if let Some(spec) = ssh {
        if drv.backend == Kind::Oracle && matches!(cfg.conn_type.as_str(), "tns" | "custom") {
            anyhow::bail!("SSH tunnel needs the Basic connection type (host and port)");
        }
        let target = if drv.backend == Kind::Oracle { cfg.port.unwrap_or(1521) } else { port(cfg, drv) };
        let t = tunnel::Tunnel::open(spec, opt(cfg, drv, "ssh_key"), &cfg.host, target)?;
        let local = ConnConfig { host: "127.0.0.1".into(), port: Some(t.port), options: cfg.options.iter().filter(|(k, _)| *k != "ssh").map(|(k, v)| (k.clone(), v.clone())).collect(), ..cfg.clone() };
        let inner = connect(&local, password, prefs)?;
        return Ok(Box::new(tunnel::Tunneled { inner, _tunnel: t }));
    }
    let mut b: Box<dyn Backend> = match drv.backend {
        Kind::Oracle => {
            let dir = client_dir(&drv.id, prefs).ok_or_else(|| DriverMissing(drv.id.clone()))?;
            Box::new(oracle::Oracle::connect(cfg, password, &dir, prefs)?)
        }
        Kind::Postgres => Box::new(postgres::Pg::connect(cfg, password, drv)?),
        Kind::Mysql => Box::new(mysql::My::connect(cfg, password, drv)?),
        Kind::Mssql => Box::new(mssql::Ms::connect(cfg, password, drv)?),
        Kind::Sqlite => Box::new(sqlite::Lite::connect(cfg)?),
        Kind::Duckdb => Box::new(duckdb::Duck::connect(cfg)?),
        Kind::Clickhouse => Box::new(clickhouse::Ch::connect(cfg, password, drv)?),
    };
    b.set_autocommit(prefs.autocommit)?;
    Ok(b)
}

/// Option lookup: connection value, then driver default.
pub(crate) fn opt<'a>(cfg: &'a ConnConfig, drv: &'a registry::Driver, k: &str) -> Option<&'a str> {
    cfg.options.get(k).or_else(|| drv.options.get(k)).map(String::as_str)
}

pub(crate) fn port(cfg: &ConnConfig, drv: &registry::Driver) -> u16 {
    cfg.port.or(drv.port).unwrap_or(0)
}

/// Buffers rows into `fetch`-sized batches for a sink.
pub(crate) struct Batcher<'a> {
    sink: &'a mut dyn FnMut(Event) -> bool,
    buf: Vec<Vec<Cell>>,
    fetch: usize,
    /// Binary columns: large values spill to disk decoded (see `lob`).
    binary: Vec<bool>,
}

impl<'a> Batcher<'a> {
    pub fn new(sink: &'a mut dyn FnMut(Event) -> bool, fetch: usize) -> Self {
        Batcher { sink, buf: Vec::new(), fetch: fetch.max(1), binary: vec![] }
    }

    pub fn columns(&mut self, c: Vec<crate::Column>) -> bool {
        self.binary = c.iter().map(|c| crate::is_binary(&c.type_name)).collect();
        (self.sink)(Event::Columns(c))
    }

    /// False = consumer wants no more rows.
    pub fn push(&mut self, mut row: Vec<Cell>) -> bool {
        // Large values go to disk now, so a batch of big BLOBs never sits in memory.
        for (i, v) in row.iter_mut().enumerate() {
            if v.as_ref().is_some_and(|s| s.len() > crate::lob::INLINE) {
                *v = v.take().map(|s| crate::lob::keep(s, self.binary.get(i).copied().unwrap_or(false)));
            }
        }
        self.buf.push(row);
        if self.buf.len() >= self.fetch {
            return (self.sink)(Event::Rows(std::mem::take(&mut self.buf)));
        }
        true
    }

    pub fn finish(mut self) {
        // Always flush, even empty, so the consumer learns the cursor is exhausted.
        (self.sink)(Event::Rows(std::mem::take(&mut self.buf)));
    }
}

pub(crate) fn numeric_type(t: &str) -> bool {
    let t = t.to_ascii_lowercase();
    ["int", "num", "dec", "float", "double", "real", "money", "serial", "binary_float", "binary_double", "bit"]
        .iter()
        .any(|k| t.contains(k))
        && !t.contains("interval")
        && !t.contains("point")
}
