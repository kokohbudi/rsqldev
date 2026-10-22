//! SQLite (bundled).

use super::{Batcher, numeric_type};
use crate::config::ConnConfig;
use crate::{Backend, Bind, Canceller, Column, Event, first_keyword};
use anyhow::Result;
use rusqlite::Connection;
use rusqlite::types::ValueRef;

pub struct Lite {
    c: Connection,
    autocommit: bool,
}

impl Lite {
    pub fn connect(cfg: &ConnConfig) -> Result<Self> {
        let path = if cfg.database.is_empty() { ":memory:" } else { &cfg.database };
        Ok(Lite { c: Connection::open(path)?, autocommit: true })
    }
}

pub(crate) fn hex(b: &[u8]) -> String {
    format!("0x{}", b.iter().map(|x| format!("{x:02X}")).collect::<String>())
}

impl Backend for Lite {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        let kw = first_keyword(sql);
        if !self.autocommit && self.c.is_autocommit() && !matches!(kw.as_str(), "BEGIN" | "COMMIT" | "ROLLBACK" | "END") {
            self.c.execute_batch("BEGIN")?;
        }
        let mut st = self.c.prepare(sql)?;
        if st.column_count() == 0 {
            return Ok(Some(st.execute([])? as u64));
        }
        let cols = st
            .columns()
            .iter()
            .map(|c| {
                let t = c.decl_type().unwrap_or("").to_owned();
                Column { name: c.name().into(), numeric: numeric_type(&t), type_name: t }
            })
            .collect();
        let n = st.column_count();
        let mut b = Batcher::new(sink, fetch);
        if !b.columns(cols) {
            return Ok(None);
        }
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let row = (0..n)
                .map(|i| match r.get_ref(i) {
                    Ok(ValueRef::Null) | Err(_) => None,
                    Ok(ValueRef::Integer(v)) => Some(v.to_string()),
                    Ok(ValueRef::Real(v)) => Some(v.to_string()),
                    Ok(ValueRef::Text(t)) => Some(String::from_utf8_lossy(t).into_owned()),
                    Ok(ValueRef::Blob(b)) => Some(hex(b)),
                })
                .collect();
            if !b.push(row) {
                return Ok(None);
            }
        }
        b.finish();
        Ok(None)
    }

    fn execute_binds(&mut self, sql: &str, binds: &[Bind]) -> Result<Option<u64>> {
        if !self.autocommit && self.c.is_autocommit() {
            self.c.execute_batch("BEGIN")?;
        }
        let params = binds.iter().map(|b| match b {
            Bind::Bytes(v) => rusqlite::types::Value::Blob(v.clone()),
            Bind::Text(s) => rusqlite::types::Value::Text(s.clone()),
        });
        Ok(Some(self.c.execute(sql, rusqlite::params_from_iter(params))? as u64))
    }

    fn commit(&mut self) -> Result<()> {
        if !self.c.is_autocommit() {
            self.c.execute_batch("COMMIT")?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        if !self.c.is_autocommit() {
            self.c.execute_batch("ROLLBACK")?;
        }
        Ok(())
    }

    fn set_autocommit(&mut self, on: bool) -> Result<()> {
        if on {
            self.commit()?;
        }
        self.autocommit = on;
        Ok(())
    }

    fn canceller(&self) -> Option<Canceller> {
        let h = self.c.get_interrupt_handle();
        Some(Box::new(move || h.interrupt()))
    }
}
