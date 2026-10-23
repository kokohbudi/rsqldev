//! rsqldev core: database backends, driver registry/downloader, dialect metadata, script runner.

pub mod backend;
pub mod compare;
pub mod complete;
pub mod config;
pub mod datapump;
pub mod ddl;
pub mod dialect;
pub mod dml;
pub mod download;
pub mod export;
pub mod format;
pub mod import;
pub mod lob;
pub mod migrate;
pub mod registry;
pub mod script;
pub mod worker;

pub use anyhow::{Error, Result};

/// One grid cell. Every backend renders values as text, the way SQL Developer's grid shows them.
pub type Cell = Option<String>;

#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    /// Right-align in grid, unquoted in JSON/INSERT export.
    pub numeric: bool,
}

#[derive(Debug)]
pub enum Event {
    Columns(Vec<Column>),
    Rows(Vec<Vec<Cell>>),
}

/// Bind value for [`Backend::execute_binds`].
#[derive(Clone, Debug, PartialEq)]
pub enum Bind {
    /// BLOB / binary upload.
    Bytes(Vec<u8>),
    /// Text too long for a literal (Oracle caps string literals at 4000 bytes).
    Text(String),
}

/// Cancels the statement currently running on a connection. Callable from any thread.
pub type Canceller = Box<dyn Fn() + Send + Sync>;

/// A live session. All calls happen on the connection's worker thread.
pub trait Backend: Send {
    /// Run one statement. Row-producing statements push `Columns` then `Rows` batches of
    /// `fetch` rows into `sink`; stop fetching when `sink` returns false.
    /// Returns rows affected for DML when the driver reports it.
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>>;

    fn commit(&mut self) -> Result<()> {
        self.execute("COMMIT", 1, &mut |_| true).map(|_| ())
    }

    fn rollback(&mut self) -> Result<()> {
        self.execute("ROLLBACK", 1, &mut |_| true).map(|_| ())
    }

    fn set_autocommit(&mut self, on: bool) -> Result<()>;

    /// Turn on server output capture (DBMS_OUTPUT, RAISE NOTICE, ...).
    fn enable_output(&mut self, _on: bool) -> Result<()> {
        Ok(())
    }

    /// Drain server output produced since the last call.
    fn take_output(&mut self) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn canceller(&self) -> Option<Canceller> {
        None
    }

    /// Run DML whose placeholders ([`dialect::placeholder`]) take `binds` in order. BLOB uploads
    /// and long text go through here: literals cap out at a few KB and differ per engine.
    fn execute_binds(&mut self, _sql: &str, _binds: &[Bind]) -> Result<Option<u64>> {
        anyhow::bail!("bind parameters are not supported for this database")
    }

    /// Stream a file of a server directory object (Oracle BFILE) into `out`, starting at byte
    /// `offset`. `progress(done_total, file_len)` runs after each read; `false` cancels.
    /// Returns the bytes written.
    fn read_server_file(
        &mut self,
        _dir: &str,
        _file: &str,
        _offset: u64,
        _out: &mut dyn std::io::Write,
        _progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<u64> {
        anyhow::bail!("server files are only available on Oracle")
    }
}

/// BLOB / BYTEA / (VAR)BINARY / RAW / IMAGE column, as each backend names it.
pub fn is_binary(type_name: &str) -> bool {
    let t = type_name.to_ascii_uppercase();
    ["BLOB", "BYTEA", "RAW", "IMAGE", "VARBIN", "BIGBINARY"].iter().any(|k| t.contains(k))
        || (t.contains("BINARY") && !t.contains("BINARY_"))
}

/// Raw bytes of a grid cell: binary columns are shown as hex (`0x…`, `\x…` or bare), so decode
/// them; anything else (or text that is not hex) is written as its UTF-8 text.
pub fn cell_bytes(col: &Column, s: &str) -> Vec<u8> {
    if let Some(b) = lob::bytes(s) {
        return b;
    }
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("\\x")).unwrap_or(s);
    is_binary(&col.type_name).then(|| hex_decode(h)).flatten().unwrap_or_else(|| s.as_bytes().to_vec())
}

/// Uppercase hex, no prefix.
pub fn hex(b: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        let _ = write!(s, "{x:02X}");
    }
    s
}

/// Decode hex digits (no prefix); None when not hex.
pub fn hex_decode(h: &str) -> Option<Vec<u8>> {
    if !h.len().is_multiple_of(2) {
        return None;
    }
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok()).collect()
}

/// Convenience: run a query and collect every row (metadata queries, tests).
pub fn query_all(b: &mut dyn Backend, sql: &str) -> Result<(Vec<Column>, Vec<Vec<Cell>>)> {
    let (mut cols, mut rows) = (vec![], vec![]);
    b.execute(sql, 500, &mut |e| {
        match e {
            Event::Columns(c) => cols = c,
            Event::Rows(mut r) => rows.append(&mut r),
        }
        true
    })?;
    Ok((cols, rows))
}

/// First keyword of a statement, uppercased, skipping leading comments and parens.
pub fn first_keyword(sql: &str) -> String {
    let mut s = sql.trim_start();
    loop {
        if let Some(r) = s.strip_prefix("--") {
            s = r.split_once('\n').map_or("", |x| x.1).trim_start();
        } else if let Some(r) = s.strip_prefix("/*") {
            s = r.split_once("*/").map_or("", |x| x.1).trim_start();
        } else if let Some(r) = s.strip_prefix('(') {
            s = r.trim_start();
        } else {
            break;
        }
    }
    s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect::<String>().to_uppercase()
}

/// Statements that return a result set on every supported engine.
pub fn is_query(sql: &str) -> bool {
    matches!(
        first_keyword(sql).as_str(),
        "SELECT" | "WITH" | "VALUES" | "TABLE" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "PRAGMA" | "EXISTS" | "FROM"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword() {
        assert_eq!(first_keyword("  -- hi\n /* x */ (select 1)"), "SELECT");
        assert!(is_query("with a as (select 1) select * from a"));
        assert!(!is_query("insert into t values (1)"));
    }

    #[test]
    fn binary_cells() {
        let col = |t: &str| Column { name: "c".into(), type_name: t.into(), numeric: false };
        assert!(is_binary("BLOB") && is_binary("bytea") && is_binary("RAW(16)") && is_binary("BigVarBin") && is_binary("VARBINARY"));
        assert!(!is_binary("BINARY_FLOAT") && !is_binary("VARCHAR2") && !is_binary("TEXT"));
        assert_eq!(cell_bytes(&col("BLOB"), "0xFF00"), vec![0xFF, 0]);
        assert_eq!(cell_bytes(&col("bytea"), "\\xff41"), vec![0xFF, 0x41]);
        assert_eq!(cell_bytes(&col("BLOB"), "504B"), b"PK".to_vec());
        assert_eq!(cell_bytes(&col("BLOB"), "not hex"), b"not hex".to_vec());
        assert_eq!(cell_bytes(&col("VARCHAR"), "504B"), b"504B".to_vec());
    }
}
