//! Oracle via ODPI-C (rust-oracle), loading the Instant Client downloaded by the driver manager.

use super::{Batcher, numeric_type};
use crate::config::{ConnConfig, Prefs};
use crate::{Backend, Bind, Canceller, Column, Event};
use anyhow::{Result, anyhow};
use oracle::sql_type::OracleType;
use oracle::{Connection, Connector, InitParams, Privilege};
use std::path::Path;
use std::sync::{Arc, OnceLock};

pub struct Oracle {
    c: Arc<Connection>,
    autocommit: bool,
    output: bool,
    date_fmt: String,
    ts_fmt: String,
}

const MONTHS: [&str; 12] = ["JANUARY", "FEBRUARY", "MARCH", "APRIL", "MAY", "JUNE", "JULY", "AUGUST", "SEPTEMBER", "OCTOBER", "NOVEMBER", "DECEMBER"];
const DAYS: [&str; 7] = ["SUNDAY", "MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY"];

/// Render a date/time with an Oracle format model (the NLS_DATE_FORMAT subset people use),
/// so the grid shows what SQL Developer shows. Unknown characters pass through.
pub fn format_model(fmt: &str, y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ns: u32) -> String {
    let f = fmt.to_uppercase();
    let mut out = String::new();
    let mut i = 0;
    let b = f.as_bytes();
    // Zeller-free weekday via days since 1970-01-01 (a Thursday).
    let wd = {
        let (yy, mm) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
        let days = 365 * yy as i64 + (yy / 4 - yy / 100 + yy / 400) as i64 + (153 * (mm as i64 - 3) + 2) / 5 + d as i64 - 719469;
        ((days % 7 + 11) % 7) as usize // 0 = Sunday
    };
    let tok = |t: &str, i: usize| f[i..].starts_with(t);
    while i < b.len() {
        let (txt, n): (String, usize) = if tok("YYYY", i) || tok("RRRR", i) {
            (format!("{y:04}"), 4)
        } else if tok("YY", i) || tok("RR", i) {
            (format!("{:02}", y.rem_euclid(100)), 2)
        } else if tok("MONTH", i) {
            (MONTHS[(mo - 1) as usize].to_string(), 5)
        } else if tok("MON", i) {
            (MONTHS[(mo - 1) as usize][..3].to_string(), 3)
        } else if tok("MM", i) {
            (format!("{mo:02}"), 2)
        } else if tok("MI", i) {
            (format!("{mi:02}"), 2)
        } else if tok("DAY", i) {
            (DAYS[wd].to_string(), 3)
        } else if tok("DY", i) {
            (DAYS[wd][..3].to_string(), 2)
        } else if tok("DD", i) {
            (format!("{d:02}"), 2)
        } else if tok("HH24", i) {
            (format!("{h:02}"), 4)
        } else if tok("HH12", i) || tok("HH", i) {
            (format!("{:02}", if h % 12 == 0 { 12 } else { h % 12 }), if tok("HH12", i) { 4 } else { 2 })
        } else if tok("SS", i) {
            (format!("{s:02}"), 2)
        } else if tok("FF", i) {
            let digits = f[i + 2..].chars().next().and_then(|c| c.to_digit(10)).filter(|n| (1..=9).contains(n));
            let full = format!("{ns:09}");
            match digits {
                Some(n) => (full[..n as usize].to_string(), 3),
                None => (full, 2),
            }
        } else if tok("AM", i) || tok("PM", i) {
            ((if h < 12 { "AM" } else { "PM" }).to_string(), 2)
        } else if tok("X", i) {
            (".".into(), 1)
        } else if b[i] == b'"' {
            let end = fmt[i + 1..].find('"').map_or(fmt.len(), |e| i + 1 + e);
            (fmt[i + 1..end].to_string(), end + 1 - i)
        } else {
            let c = fmt[i..].chars().next().unwrap();
            (c.to_string(), c.len_utf8())
        };
        out.push_str(&txt);
        i += n;
    }
    out
}

/// `CREATE [OR REPLACE] [EDITIONABLE] PACKAGE BODY [owner.]name …` → (ALL_ERRORS type, owner, name),
/// identifiers as the dictionary stores them.
pub fn plsql_target(sql: &str) -> Option<(String, Option<String>, String)> {
    let words: Vec<&str> = sql.split_whitespace().take(8).collect();
    let up: Vec<String> = words.iter().map(|w| w.to_uppercase()).collect();
    if up.first()? != "CREATE" {
        return None;
    }
    let mut i = 1;
    while matches!(up.get(i).map(String::as_str), Some("OR" | "REPLACE" | "EDITIONABLE" | "NONEDITIONABLE")) {
        i += 1;
    }
    let mut kind = up.get(i)?.clone();
    if !matches!(kind.as_str(), "PROCEDURE" | "FUNCTION" | "PACKAGE" | "TRIGGER" | "TYPE") {
        return None;
    }
    if matches!(kind.as_str(), "PACKAGE" | "TYPE") && up.get(i + 1).map(String::as_str) == Some("BODY") {
        kind += " BODY";
        i += 1;
    }
    let raw = words.get(i + 1)?.split(['(', ';']).next()?;
    let ident = |s: &str| if s.starts_with('"') { s.trim_matches('"').to_string() } else { s.to_uppercase() };
    let (owner, name) = match raw.split_once('.') {
        Some((o, n)) => (Some(ident(o)), ident(n)),
        None => (None, ident(raw)),
    };
    Some((kind, owner, name))
}

/// `+07:00` / `-03:30` (ODPI gives the minute offset with the hour's sign).
fn tz_offset(h: i32, m: i32) -> String {
    format!("{}{:02}:{:02}", if h < 0 || m < 0 { '-' } else { '+' }, h.abs(), m.abs())
}

/// ODPI-C can be initialised once per process, so the first client folder wins.
fn init(dir: &Path, tns_admin: Option<&str>) -> Result<()> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        let mut p = InitParams::new();
        p.oracle_client_lib_dir(dir).map_err(|e| e.to_string())?;
        if let Some(t) = tns_admin {
            p.oracle_client_config_dir(t).map_err(|e| e.to_string())?;
        }
        p.init().map(|_| ()).map_err(|e| e.to_string())
    })
    .clone()
    .map_err(|e| anyhow!("Oracle client init failed ({}): {e}", dir.display()))
}

pub fn connect_string(cfg: &ConnConfig) -> String {
    let port = cfg.port.unwrap_or(1521);
    match cfg.conn_type.as_str() {
        "tns" | "custom" => cfg.tns.clone(),
        _ if cfg.sid => format!(
            "(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP)(HOST={})(PORT={port}))(CONNECT_DATA=(SID={})))",
            cfg.host, cfg.database
        ),
        _ => format!("{}:{port}/{}", cfg.host, cfg.database),
    }
}

impl Oracle {
    pub fn connect(cfg: &ConnConfig, pw: &str, dir: &Path, prefs: &Prefs) -> Result<Self> {
        init(dir, cfg.options.get("tns_admin").or(Some(&prefs.tns_dir)).map(String::as_str).filter(|s| !s.trim().is_empty()))?;
        let mut k = Connector::new(&cfg.user, pw, connect_string(cfg));
        // Expired password (ORA-28001): the app asks for a new one and passes it here once.
        if let Some(np) = cfg.options.get("new_password").filter(|s| !s.is_empty()) {
            k.new_password(np.as_str());
        }
        match cfg.role.to_uppercase().as_str() {
            "SYSDBA" => k.privilege(Privilege::Sysdba),
            "SYSOPER" => k.privilege(Privilege::Sysoper),
            "SYSASM" => k.privilege(Privilege::Sysasm),
            "SYSBACKUP" => k.privilege(Privilege::Sysbackup),
            "SYSDG" => k.privilege(Privilege::Sysdg),
            "SYSKM" => k.privilege(Privilege::Syskm),
            _ => &mut k,
        };
        let c = k.connect()?;
        c.set_module("rsqldev")?;
        let date_fmt = if prefs.nls_date_format.is_empty() { "DD-MON-RR".to_string() } else { prefs.nls_date_format.clone() };
        let ts_fmt = if prefs.nls_timestamp_format.trim().is_empty() { "DD-MON-RR HH.MI.SSXFF AM".to_string() } else { prefs.nls_timestamp_format.replace('\'', "''") };
        // Same formats server-side, so grid text converts back implicitly in generated DML.
        c.execute(&format!("ALTER SESSION SET NLS_DATE_FORMAT = '{}'", date_fmt.replace('\'', "''")), &[])?;
        c.execute(&format!("ALTER SESSION SET NLS_TIMESTAMP_FORMAT = '{ts_fmt}'"), &[])?;
        c.execute(&format!("ALTER SESSION SET NLS_TIMESTAMP_TZ_FORMAT = '{ts_fmt} TZH:TZM'"), &[])?;
        if !prefs.ddl_storage {
            // Best effort: DDL without STORAGE / TABLESPACE / segment clauses for this session.
            let _ = c.execute("BEGIN DBMS_METADATA.SET_TRANSFORM_PARAM(DBMS_METADATA.SESSION_TRANSFORM, 'SEGMENT_ATTRIBUTES', FALSE); END;", &[]);
        }
        Ok(Oracle { c: Arc::new(c), autocommit: false, output: false, date_fmt, ts_fmt })
    }
}

impl Backend for Oracle {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        let mut st = self.c.statement(sql).fetch_array_size(fetch as u32).build()?;
        if !st.is_query() {
            st.execute(&[])?;
            // ORA-24344 "success with compilation error": report ALL_ERRORS like SHOW ERRORS.
            if self.c.last_warning().is_some_and(|w| w.db_error().is_some_and(|e| e.code() == 24344)) {
                if let Some((kind, owner, name)) = plsql_target(sql) {
                    let lit = |s: &str| format!("'{}'", s.replace('\'', "''"));
                    let owner = owner.map_or("SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')".to_string(), |o| lit(&o));
                    let q = format!("SELECT line, position, text FROM all_errors WHERE owner = {owner} AND name = {} AND type = {} ORDER BY sequence", lit(&name), lit(&kind));
                    let mut msg = format!("{} {name} compiled with errors\nLINE/COL  ERROR\n", crate::script::title_case(&kind));
                    for r in self.c.query(&q, &[])? {
                        let r = r?;
                        msg += &format!("{}/{}  {}\n", r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?.trim_end());
                    }
                    anyhow::bail!("{}", msg.trim_end());
                }
            }
            let n = st.row_count().ok();
            if self.autocommit && (st.is_dml() || st.is_plsql()) {
                self.c.commit()?;
            }
            return Ok(if st.is_dml() { n } else { None });
        }
        let rs = st.query(&[])?;
        let cols = rs
            .column_info()
            .iter()
            .map(|c| {
                let t = c.oracle_type().to_string();
                Column { name: c.name().into(), numeric: numeric_type(&t), type_name: t }
            })
            .collect::<Vec<_>>();
        // DATE / TIMESTAMP [WITH [LOCAL] TIME ZONE] render through the session format models
        // (TZ adds TZH:TZM), so edited cells convert back implicitly.
        let fmts: Vec<Option<(&str, bool)>> = rs
            .column_info()
            .iter()
            .map(|c| match c.oracle_type() {
                OracleType::Date => Some((self.date_fmt.as_str(), false)),
                OracleType::Timestamp(_) | OracleType::TimestampLTZ(_) => Some((self.ts_fmt.as_str(), false)),
                OracleType::TimestampTZ(_) => Some((self.ts_fmt.as_str(), true)),
                _ => None,
            })
            .collect();
        let n = cols.len();
        let mut b = Batcher::new(sink, fetch);
        if !b.columns(cols) {
            return Ok(None);
        }
        for row in rs {
            let row = row?;
            let cells = (0..n)
                .map(|i| match fmts[i] {
                    Some((f, tz)) => row
                        .get::<usize, Option<oracle::sql_type::Timestamp>>(i)
                        .map(|t| {
                            t.map(|t| {
                                let s = format_model(f, t.year(), t.month(), t.day(), t.hour(), t.minute(), t.second(), t.nanosecond());
                                if tz { format!("{s} {}", tz_offset(t.tz_hour_offset(), t.tz_minute_offset())) } else { s }
                            })
                        })
                        .unwrap_or_else(|e| Some(format!("<{e}>"))),
                    None => row.get::<usize, Option<String>>(i).unwrap_or_else(|e| Some(format!("<{e}>"))),
                })
                .collect();
            if !b.push(cells) {
                return Ok(None);
            }
        }
        b.finish();
        Ok(None)
    }

    fn commit(&mut self) -> Result<()> {
        Ok(self.c.commit()?)
    }

    fn rollback(&mut self) -> Result<()> {
        Ok(self.c.rollback()?)
    }

    fn set_autocommit(&mut self, on: bool) -> Result<()> {
        self.autocommit = on;
        Ok(())
    }

    fn enable_output(&mut self, on: bool) -> Result<()> {
        self.output = on;
        let sql = if on { "BEGIN DBMS_OUTPUT.ENABLE(NULL); END;" } else { "BEGIN DBMS_OUTPUT.DISABLE; END;" };
        self.c.execute(sql, &[])?;
        Ok(())
    }

    fn take_output(&mut self) -> Result<Vec<String>> {
        if !self.output {
            return Ok(vec![]);
        }
        let mut st = self.c.statement("BEGIN DBMS_OUTPUT.GET_LINE(:1, :2); END;").build()?;
        st.bind(1, &OracleType::Varchar2(32767))?;
        st.bind(2, &OracleType::Number(0, 0))?;
        let mut out = vec![];
        loop {
            st.execute(&[])?;
            if st.bind_value::<_, i32>(2)? != 0 {
                return Ok(out);
            }
            out.push(st.bind_value::<_, Option<String>>(1)?.unwrap_or_default());
        }
    }

    fn read_server_file(
        &mut self,
        dir: &str,
        file: &str,
        offset: u64,
        out: &mut dyn std::io::Write,
        progress: &mut dyn FnMut(u64, u64) -> bool,
    ) -> Result<u64> {
        use std::io::Read;
        let (dir, file) = (dir.to_string(), file.to_string());
        let mut buf = vec![0u8; crate::datapump::DOWNLOAD_READ];
        let mut written = 0u64;
        loop {
            let mut st = self.c.statement(crate::datapump::DOWNLOAD_BLOCK).build()?;
            st.bind(1, &dir)?;
            st.bind(2, &file)?;
            st.bind(3, &((offset + written + 1) as i64))?;
            st.bind(4, &(crate::datapump::DOWNLOAD_READ as i64))?;
            st.bind(5, &OracleType::Number(0, 0))?;
            st.bind(6, &OracleType::BLOB)?;
            st.execute(&[])?;
            let len = st.bind_value::<_, u64>(5)?;
            let before = written;
            if offset + written < len {
                if let Some(mut blob) = st.bind_value::<_, Option<oracle::sql_type::Blob>>(6)? {
                    loop {
                        let n = blob.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        out.write_all(&buf[..n])?;
                        written += n as u64;
                    }
                }
            }
            if offset + written < len && written == before {
                anyhow::bail!("server file shrank or could not be read at offset {}", offset + written);
            }
            if !progress(offset + written, len) {
                anyhow::bail!("cancelled");
            }
            if offset + written >= len {
                return Ok(written);
            }
        }
    }

    fn execute_binds(&mut self, sql: &str, binds: &[Bind]) -> Result<Option<u64>> {
        use oracle::sql_type::{Blob, Clob, ToSql};
        use std::io::Write;
        // RAW binds cap at 2000 bytes and VARCHAR2 at 4000: bigger values go through temporary LOBs.
        let mut params: Vec<Box<dyn ToSql>> = vec![];
        for b in binds {
            params.push(match b {
                Bind::Bytes(v) if v.len() <= 2000 => Box::new(v.clone()),
                Bind::Bytes(v) => {
                    let mut l = Blob::new(&self.c)?;
                    l.write_all(v)?;
                    Box::new(l)
                }
                Bind::Text(s) => {
                    let mut l = Clob::new(&self.c)?;
                    l.write_all(s.as_bytes())?;
                    Box::new(l)
                }
            });
        }
        let refs: Vec<&dyn ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let st = self.c.execute(sql, &refs)?;
        let n = st.row_count()?;
        if self.autocommit {
            self.c.commit()?;
        }
        Ok(Some(n))
    }

    fn canceller(&self) -> Option<Canceller> {
        let c = self.c.clone();
        Some(Box::new(move || {
            let _ = c.break_execution();
        }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn format_models() {
        assert_eq!(super::format_model("DD-MON-RR", 2003, 6, 17, 0, 0, 0, 0), "17-JUN-03");
        assert_eq!(super::format_model("YYYY-MM-DD HH24:MI:SS", 2026, 10, 22, 14, 5, 9, 0), "2026-10-22 14:05:09");
        assert_eq!(super::format_model("DD-MON-RR HH.MI.SSXFF AM", 2003, 6, 17, 13, 0, 0, 120000000), "17-JUN-03 01.00.00.120000000 PM");
        assert_eq!(super::format_model("DY DD \"de\" MONTH", 2026, 10, 22, 0, 0, 0, 0), "THU 22 de OCTOBER");
        assert_eq!(super::tz_offset(7, 0), "+07:00");
        assert_eq!(super::plsql_target("create or replace package body hr.pkg as"), Some(("PACKAGE BODY".into(), Some("HR".into()), "PKG".into())));
        assert_eq!(super::plsql_target("CREATE EDITIONABLE PROCEDURE \"MyProc\"(a number) is"), Some(("PROCEDURE".into(), None, "MyProc".into())));
        assert_eq!(super::plsql_target("create table t (a int)"), None);
        assert_eq!(super::tz_offset(-3, -30), "-03:30");
    }
}
