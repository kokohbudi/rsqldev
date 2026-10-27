//! SQL Server / Azure SQL / Babelfish over TDS (tiberius). tiberius is async-only, so each
//! connection owns a current-thread tokio runtime and blocks on it.

use super::{Batcher, numeric_type, opt, port};
use crate::config::ConnConfig;
use crate::registry::Driver;
use crate::{Backend, Bind, Column, Event, first_keyword, is_query};
use anyhow::Result;
use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use futures_util::TryStreamExt;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, FromSql, QueryItem};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub struct Ms {
    rt: Runtime,
    c: Client<Compat<TcpStream>>,
}

impl Ms {
    pub fn connect(cfg: &ConnConfig, pw: &str, drv: &Driver) -> Result<Self> {
        let mut config = Config::new();
        config.host(&cfg.host);
        config.port(port(cfg, drv));
        if !cfg.database.is_empty() {
            config.database(&cfg.database);
        } else if let Some(d) = &drv.database {
            config.database(d);
        }
        #[cfg(windows)]
        if cfg.user.is_empty() {
            config.authentication(AuthMethod::Integrated);
        } else {
            config.authentication(AuthMethod::sql_server(&cfg.user, pw));
        }
        #[cfg(not(windows))]
        config.authentication(AuthMethod::sql_server(&cfg.user, pw));
        // Encrypted and certificate-checked unless the connection opts out (self-signed dev servers:
        // trust_cert=true; encrypt=login encrypts the login only, encrypt=false not at all).
        config.encryption(match opt(cfg, drv, "encrypt") {
            Some("false") => EncryptionLevel::NotSupported,
            Some("login") => EncryptionLevel::Off,
            _ => EncryptionLevel::Required,
        });
        if opt(cfg, drv, "trust_cert") == Some("true") {
            config.trust_cert();
        }
        config.application_name("rsqldev");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        let c = rt.block_on(async {
            let tcp = TcpStream::connect(config.get_addr()).await?;
            tcp.set_nodelay(true)?;
            match Client::connect(config.clone(), tcp.compat_write()).await {
                // Azure gateway redirect.
                Err(tiberius::error::Error::Routing { host, port }) => {
                    config.host(&host);
                    config.port(port);
                    let tcp = TcpStream::connect(config.get_addr()).await?;
                    tcp.set_nodelay(true)?;
                    Ok::<_, anyhow::Error>(Client::connect(config, tcp.compat_write()).await?)
                }
                r => Ok(r?),
            }
        })
        .map_err(|e: anyhow::Error| {
            if format!("{e:#}").contains("certificate") {
                e.context("server certificate not trusted: add trust_cert=true to the connection Options for a self-signed certificate (dev servers)")
            } else {
                e
            }
        })?;
        Ok(Ms { rt, c })
    }
}

fn text(cd: ColumnData<'static>) -> Option<String> {
    use ColumnData::*;
    match &cd {
        U8(v) => v.map(|x| x.to_string()),
        I16(v) => v.map(|x| x.to_string()),
        I32(v) => v.map(|x| x.to_string()),
        I64(v) => v.map(|x| x.to_string()),
        F32(v) => v.map(|x| x.to_string()),
        F64(v) => v.map(|x| x.to_string()),
        Bit(v) => v.map(|b| if b { "1" } else { "0" }.into()),
        String(v) => v.as_ref().map(|s| s.to_string()),
        Guid(v) => v.map(|g| g.to_string().to_uppercase()),
        Binary(v) => v.as_ref().map(|b| format!("0x{}", b.iter().map(|x| format!("{x:02X}")).collect::<std::string::String>())),
        Numeric(v) => v.map(|n| n.to_string()),
        Xml(v) => v.as_ref().map(|x| x.to_string()),
        DateTime(_) | SmallDateTime(_) | DateTime2(_) => NaiveDateTime::from_sql(&cd).ok().flatten().map(|d| d.to_string()),
        Date(_) => NaiveDate::from_sql(&cd).ok().flatten().map(|d| d.to_string()),
        Time(_) => NaiveTime::from_sql(&cd).ok().flatten().map(|d| d.to_string()),
        DateTimeOffset(_) => chrono::DateTime::<FixedOffset>::from_sql(&cd).ok().flatten().map(|d: chrono::DateTime<FixedOffset>| d.to_string()),
    }
}

impl Backend for Ms {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        let kw = first_keyword(sql);
        if !(is_query(sql) || kw == "EXEC" || kw == "EXECUTE" || kw.starts_with("SP_")) {
            let r = self.rt.block_on(self.c.execute(sql, &[]))?;
            return Ok(Some(r.total()));
        }
        let Ms { rt, c } = self;
        rt.block_on(async {
            let mut s = c.simple_query(sql).await?;
            let mut b = Batcher::new(sink, fetch);
            let mut had_cols = false;
            let mut abandoned = false;
            while let Some(item) = s.try_next().await? {
                match item {
                    QueryItem::Metadata(m) if m.result_index() == 0 => {
                        had_cols = true;
                        let cols = m
                            .columns()
                            .iter()
                            .map(|c| {
                                let t = format!("{:?}", c.column_type());
                                Column { name: c.name().into(), numeric: numeric_type(&t), type_name: t }
                            })
                            .collect();
                        if !b.columns(cols) {
                            abandoned = true;
                            break;
                        }
                    }
                    QueryItem::Row(r) if r.result_index() == 0 => {
                        if !b.push(r.into_iter().map(text).collect()) {
                            abandoned = true;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            drop(s);
            if abandoned {
                // The grid stopped reading: a TDS Attention stops the server sending the rest
                // (otherwise the next request reads every remaining row first).
                tokio::time::timeout(std::time::Duration::from_secs(10), c.cancel_query()).await.map_err(|_| anyhow::anyhow!("connection lost: no reply to the query cancel"))??;
                return Ok(None);
            }
            if had_cols {
                b.finish();
            }
            Ok(None)
        })
    }

    fn execute_binds(&mut self, sql: &str, binds: &[Bind]) -> Result<Option<u64>> {
        let params: Vec<&dyn tiberius::ToSql> = binds
            .iter()
            .map(|b| match b {
                Bind::Bytes(v) => v as &dyn tiberius::ToSql,
                Bind::Text(s) => s as _,
            })
            .collect();
        let r = self.rt.block_on(self.c.execute(sql, &params))?;
        Ok(Some(r.total()))
    }

    fn commit(&mut self) -> Result<()> {
        self.execute("IF @@TRANCOUNT > 0 COMMIT", 1, &mut |_| true).map(|_| ())
    }

    fn rollback(&mut self) -> Result<()> {
        self.execute("IF @@TRANCOUNT > 0 ROLLBACK", 1, &mut |_| true).map(|_| ())
    }

    fn set_autocommit(&mut self, on: bool) -> Result<()> {
        self.execute(if on { "SET IMPLICIT_TRANSACTIONS OFF" } else { "SET IMPLICIT_TRANSACTIONS ON" }, 1, &mut |_| true).map(|_| ())
    }
}
