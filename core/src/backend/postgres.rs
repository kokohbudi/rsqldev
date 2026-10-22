//! PostgreSQL wire protocol (PostgreSQL, Redshift, Cockroach, Yugabyte, Greenplum, ...).
//!
//! Queries run through `DECLARE CURSOR` + `FETCH n` on the simple-query protocol: the server
//! renders every type as text and rows stream in fetch-size chunks, so no per-type decoding.

use super::{Batcher, numeric_type, opt, port};
use crate::config::ConnConfig;
use crate::registry::Driver;
use crate::{Backend, Bind, Canceller, Column, Event, first_keyword};
use anyhow::Result;
use postgres::config::SslMode;
use postgres::{Client, Config, SimpleQueryMessage};
use postgres_native_tls::MakeTlsConnector;
use std::sync::{Arc, Mutex};

pub struct Pg {
    c: Client,
    tls: MakeTlsConnector,
    autocommit: bool,
    in_txn: bool,
    output: bool,
    notices: Arc<Mutex<Vec<String>>>,
}

impl Pg {
    pub fn connect(cfg: &ConnConfig, pw: &str, drv: &Driver) -> Result<Self> {
        let mode = opt(cfg, drv, "sslmode").unwrap_or("prefer");
        let verify = mode == "verify-full";
        let tls = MakeTlsConnector::new(
            native_tls::TlsConnector::builder().danger_accept_invalid_certs(!verify).danger_accept_invalid_hostnames(!verify).build()?,
        );
        let notices = Arc::new(Mutex::new(Vec::new()));
        let n2 = notices.clone();
        let mut pc = Config::new();
        pc.host(&cfg.host)
            .port(port(cfg, drv))
            .user(&cfg.user)
            .password(pw)
            .dbname(if cfg.database.is_empty() { drv.database.as_deref().unwrap_or("postgres") } else { &cfg.database })
            .application_name("rsqldev")
            .ssl_mode(match mode {
                "disable" => SslMode::Disable,
                "require" | "verify-ca" | "verify-full" => SslMode::Require,
                _ => SslMode::Prefer,
            })
            .notice_callback(move |n| n2.lock().unwrap().push(format!("{}: {}", n.severity(), n.message())));
        let c = pc.connect(tls.clone())?;
        Ok(Pg { c, tls, autocommit: true, in_txn: false, output: false, notices })
    }

    fn cursor(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<()> {
        let stmt = self.c.prepare(sql)?;
        let cols = stmt
            .columns()
            .iter()
            .map(|c| Column { name: c.name().into(), type_name: c.type_().name().into(), numeric: numeric_type(c.type_().name()) })
            .collect();
        let own_txn = !self.in_txn;
        if own_txn {
            self.c.batch_execute("BEGIN")?;
        }
        let res = (|| -> Result<()> {
            self.c.batch_execute(&format!("DECLARE rsqldev_cur NO SCROLL CURSOR FOR {}", sql.trim_end().trim_end_matches(';')))?;
            let mut b = Batcher::new(sink, fetch);
            if !b.columns(cols) {
                return Ok(());
            }
            loop {
                let mut got = 0;
                for m in self.c.simple_query(&format!("FETCH {fetch} FROM rsqldev_cur"))? {
                    if let SimpleQueryMessage::Row(r) = m {
                        got += 1;
                        if !b.push((0..r.len()).map(|i| r.get(i).map(str::to_owned)).collect()) {
                            return Ok(());
                        }
                    }
                }
                if got < fetch {
                    b.finish();
                    return Ok(());
                }
            }
        })();
        if own_txn {
            self.c.batch_execute(if res.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
        } else if res.is_ok() {
            self.c.batch_execute("CLOSE rsqldev_cur")?;
        }
        res
    }
}

/// DECLARE CURSOR only accepts plain queries.
fn cursorable(sql: &str) -> bool {
    match first_keyword(sql).as_str() {
        "SELECT" | "VALUES" | "TABLE" => !sql.to_uppercase().contains(" INTO "),
        // ponytail: substring check, data-modifying CTEs fall back to the simple path.
        "WITH" => {
            let u = sql.to_uppercase();
            !["INSERT ", "UPDATE ", "DELETE ", "MERGE "].iter().any(|k| u.contains(k))
        }
        _ => false,
    }
}

impl Backend for Pg {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        let kw = first_keyword(sql);
        let ends_txn = matches!(kw.as_str(), "COMMIT" | "ROLLBACK" | "END" | "ABORT");
        if !self.autocommit && !self.in_txn && !ends_txn && kw != "BEGIN" {
            self.c.batch_execute("BEGIN")?;
            self.in_txn = true;
        }
        if kw == "BEGIN" || kw == "START" {
            self.in_txn = true;
        }
        if cursorable(sql) {
            self.cursor(sql, fetch, sink)?;
            return Ok(None);
        }
        let msgs = self.c.simple_query(sql);
        if ends_txn {
            self.in_txn = false;
        }
        let mut b = Batcher::new(sink, fetch);
        let mut affected = None;
        let mut has_rows = false;
        for m in msgs? {
            match m {
                SimpleQueryMessage::RowDescription(cols) => {
                    has_rows = true;
                    b.columns(cols.iter().map(|c| Column { name: c.name().into(), type_name: "text".into(), numeric: false }).collect());
                }
                SimpleQueryMessage::Row(r) => {
                    b.push((0..r.len()).map(|i| r.get(i).map(str::to_owned)).collect());
                }
                SimpleQueryMessage::CommandComplete(n) => affected = Some(n),
                _ => {}
            }
        }
        if has_rows {
            b.finish();
            return Ok(None);
        }
        Ok(affected)
    }

    fn execute_binds(&mut self, sql: &str, binds: &[Bind]) -> Result<Option<u64>> {
        if !self.autocommit && !self.in_txn {
            self.c.batch_execute("BEGIN")?;
            self.in_txn = true;
        }
        let params: Vec<&(dyn postgres::types::ToSql + Sync)> = binds
            .iter()
            .map(|b| match b {
                Bind::Bytes(v) => v as &(dyn postgres::types::ToSql + Sync),
                Bind::Text(s) => s as _,
            })
            .collect();
        Ok(Some(self.c.execute(sql, &params)?))
    }

    fn commit(&mut self) -> Result<()> {
        if self.in_txn {
            self.c.batch_execute("COMMIT")?;
        }
        self.in_txn = false;
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        if self.in_txn {
            self.c.batch_execute("ROLLBACK")?;
        }
        self.in_txn = false;
        Ok(())
    }

    fn set_autocommit(&mut self, on: bool) -> Result<()> {
        if on {
            self.commit()?;
        }
        self.autocommit = on;
        Ok(())
    }

    fn enable_output(&mut self, on: bool) -> Result<()> {
        self.output = on;
        Ok(())
    }

    fn take_output(&mut self) -> Result<Vec<String>> {
        let v = std::mem::take(&mut *self.notices.lock().unwrap());
        Ok(if self.output { v } else { vec![] })
    }

    fn canceller(&self) -> Option<Canceller> {
        let (t, tls) = (self.c.cancel_token(), self.tls.clone());
        Some(Box::new(move || {
            let _ = t.cancel_query(tls.clone());
        }))
    }
}
