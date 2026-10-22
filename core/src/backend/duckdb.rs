//! DuckDB (bundled). Plain queries are wrapped in `SELECT COLUMNS(*)::VARCHAR` so DuckDB renders
//! every type (lists, structs, intervals, ...) as text itself.

use super::{Batcher, numeric_type};
use crate::config::ConnConfig;
use crate::{Backend, Bind, Canceller, Column, Event, first_keyword};
use anyhow::Result;
use duckdb::Connection;
use duckdb::types::Value;

pub struct Duck {
    c: Connection,
    autocommit: bool,
    in_txn: bool,
}

impl Duck {
    pub fn connect(cfg: &ConnConfig) -> Result<Self> {
        let c = if cfg.database.is_empty() { Connection::open_in_memory()? } else { Connection::open(&cfg.database)? };
        Ok(Duck { c, autocommit: true, in_txn: false })
    }
}

fn wrappable(sql: &str) -> bool {
    matches!(first_keyword(sql).as_str(), "SELECT" | "WITH" | "VALUES" | "FROM" | "TABLE")
}

fn text(v: Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Text(s) => Some(s),
        Value::Blob(b) => Some(super::sqlite::hex(&b)),
        other => Some(format!("{other:?}")),
    }
}

impl Backend for Duck {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        let kw = first_keyword(sql);
        match kw.as_str() {
            "BEGIN" => self.in_txn = true,
            "COMMIT" | "ROLLBACK" | "END" | "ABORT" => self.in_txn = false,
            _ if !self.autocommit && !self.in_txn => {
                self.c.execute_batch("BEGIN")?;
                self.in_txn = true;
            }
            _ => {}
        }
        let body = sql.trim_end().trim_end_matches(';');
        if !crate::is_query(sql) {
            return Ok(Some(self.c.execute(body, [])? as u64));
        }
        let wrap = wrappable(sql);
        // Column types without running the query.
        let described: Vec<(String, String)> = if wrap {
            let mut d = self.c.prepare(&format!("DESCRIBE {body}"))?;
            d.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<_, _>>()?
        } else {
            vec![]
        };
        let types: Vec<String> = described.iter().map(|d| d.1.clone()).collect();
        let q = if !wrap {
            body.to_owned()
        } else if types.iter().any(|t| t == "BLOB") {
            // BLOB::VARCHAR gives escaped text ('PK\x03'), not hex: list columns to hex those.
            let cols = described
                .iter()
                .map(|(n, t)| {
                    let q = format!("\"{}\"", n.replace('"', "\"\""));
                    if t == "BLOB" { format!("'0x' || hex({q}) AS {q}") } else { format!("{q}::VARCHAR AS {q}") }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("SELECT {cols} FROM ({body})")
        } else {
            format!("SELECT COLUMNS(*)::VARCHAR FROM ({body})")
        };
        let mut st = self.c.prepare(&q)?;
        let mut rows = st.query([])?;
        let names = rows.as_ref().map(|s| s.column_names()).unwrap_or_default();
        let cols = names
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let t = types.get(i).cloned().unwrap_or_default();
                Column { name: n.clone(), numeric: numeric_type(&t), type_name: t }
            })
            .collect();
        let n = names.len();
        let mut b = Batcher::new(sink, fetch);
        if !b.columns(cols) {
            return Ok(None);
        }
        while let Some(r) = rows.next()? {
            if !b.push((0..n).map(|i| r.get::<_, Value>(i).ok().and_then(text)).collect()) {
                return Ok(None);
            }
        }
        b.finish();
        Ok(None)
    }

    fn execute_binds(&mut self, sql: &str, binds: &[Bind]) -> Result<Option<u64>> {
        if !self.autocommit && !self.in_txn {
            self.c.execute_batch("BEGIN")?;
            self.in_txn = true;
        }
        let params = binds.iter().map(|b| match b {
            Bind::Bytes(v) => Value::Blob(v.clone()),
            Bind::Text(s) => Value::Text(s.clone()),
        });
        Ok(Some(self.c.execute(sql, duckdb::params_from_iter(params))? as u64))
    }

    fn commit(&mut self) -> Result<()> {
        if std::mem::take(&mut self.in_txn) {
            self.c.execute_batch("COMMIT")?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        if std::mem::take(&mut self.in_txn) {
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
        let h = self.c.interrupt_handle();
        Some(Box::new(move || h.interrupt()))
    }
}
