//! MySQL wire protocol (MySQL, MariaDB, TiDB, Doris, StarRocks, OceanBase, ...). Text protocol:
//! values arrive as strings already.

use super::{Batcher, numeric_type, opt, port};
use crate::config::ConnConfig;
use crate::registry::Driver;
use crate::{Backend, Bind, Canceller, Column, Event};
use anyhow::Result;
use mysql::prelude::Queryable;
use mysql::{Conn, Opts, OptsBuilder, SslOpts, Value};

pub struct My {
    c: Conn,
    opts: Opts,
}

impl My {
    pub fn connect(cfg: &ConnConfig, pw: &str, drv: &Driver) -> Result<Self> {
        let mut b = OptsBuilder::new()
            .ip_or_hostname(Some(cfg.host.clone()))
            .tcp_port(port(cfg, drv))
            .user(Some(cfg.user.clone()))
            .pass(Some(pw.to_owned()))
            .db_name((!cfg.database.is_empty()).then(|| cfg.database.clone()));
        if opt(cfg, drv, "ssl") == Some("true") {
            b = b.ssl_opts(Some(SslOpts::default().with_danger_accept_invalid_certs(opt(cfg, drv, "verify") != Some("true"))));
        }
        let opts = Opts::from(b);
        Ok(My { c: Conn::new(opts.clone())?, opts })
    }
}

/// How a column's text-protocol bytes are shown.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Text,
    /// binary charset (BINARY, VARBINARY, BLOB): always hex, even when valid UTF-8.
    Binary,
    /// BIT(n) arrives as big-endian bytes.
    Bit,
}

fn text(v: Value, shape: Shape) -> Option<String> {
    match v {
        Value::NULL => None,
        Value::Bytes(b) if shape == Shape::Bit => Some(b.iter().fold(0u64, |n, x| n << 8 | *x as u64).to_string()),
        Value::Bytes(b) if shape == Shape::Binary => Some(super::sqlite::hex(&b)),
        Value::Bytes(b) => Some(String::from_utf8(b).unwrap_or_else(|e| super::sqlite::hex(e.as_bytes()))),
        other => Some(other.as_sql(true).trim_matches('\'').to_owned()),
    }
}

/// Grid type name and shape. The wire type says BLOB for TEXT too; charset 63 means binary.
fn column(c: &mysql::Column) -> (Column, Shape) {
    let mut t = format!("{:?}", c.column_type()).trim_start_matches("MYSQL_TYPE_").to_owned();
    let binary = c.character_set() == 63;
    let shape = match t.as_str() {
        "BIT" => Shape::Bit,
        "TINY_BLOB" | "MEDIUM_BLOB" | "LONG_BLOB" | "BLOB" | "VAR_STRING" | "STRING" | "VARCHAR" if binary => Shape::Binary,
        _ => Shape::Text,
    };
    if shape == Shape::Text && t.ends_with("BLOB") {
        t = "TEXT".into();
    } else if shape == Shape::Binary && !t.ends_with("BLOB") {
        t = "VARBINARY".into();
    }
    let numeric = numeric_type(&t) || ["LONGLONG", "LONG", "SHORT", "TINY", "INT24", "YEAR"].contains(&t.as_str());
    (Column { name: c.name_str().into_owned(), numeric, type_name: t }, shape)
}

impl Backend for My {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        let mut r = self.c.query_iter(sql)?;
        let mut b = Batcher::new(sink, fetch);
        let mut had_rows = false;
        let mut affected = 0;
        // First result set with columns feeds the grid; later ones are drained.
        while let Some(set) = r.iter() {
            let (cols, shapes): (Vec<Column>, Vec<Shape>) = set.columns().as_ref().iter().map(column).unzip();
            if cols.is_empty() || had_rows {
                affected += set.affected_rows();
                for row in set {
                    row?;
                }
                continue;
            }
            had_rows = true;
            if !b.columns(cols) {
                // ponytail: dropping the result drains remaining rows; KILL QUERY if that gets slow.
                return Ok(None);
            }
            for row in set {
                if !b.push(row?.unwrap().into_iter().zip(&shapes).map(|(v, s)| text(v, *s)).collect()) {
                    return Ok(None);
                }
            }
        }
        drop(r);
        if had_rows {
            b.finish();
            return Ok(None);
        }
        Ok(Some(affected))
    }

    fn execute_binds(&mut self, sql: &str, binds: &[Bind]) -> Result<Option<u64>> {
        let params: Vec<Value> = binds
            .iter()
            .map(|b| match b {
                Bind::Bytes(v) => Value::Bytes(v.clone()),
                Bind::Text(s) => Value::Bytes(s.clone().into_bytes()),
            })
            .collect();
        self.c.exec_drop(sql, params)?;
        Ok(Some(self.c.affected_rows()))
    }

    fn set_autocommit(&mut self, on: bool) -> Result<()> {
        self.c.query_drop(if on { "SET autocommit=1" } else { "SET autocommit=0" })?;
        Ok(())
    }

    fn canceller(&self) -> Option<Canceller> {
        let (id, opts) = (self.c.connection_id(), self.opts.clone());
        Some(Box::new(move || {
            if let Ok(mut c) = Conn::new(opts.clone()) {
                let _ = c.query_drop(format!("KILL QUERY {id}"));
            }
        }))
    }
}
