//! ClickHouse over its HTTP interface. Results stream as TabSeparatedWithNamesAndTypes.

use super::{Batcher, numeric_type, opt, port};
use crate::config::ConnConfig;
use crate::registry::Driver;
use crate::{Backend, Canceller, Column, Event, is_query};
use anyhow::{Result, bail};
use reqwest::blocking::Client;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub struct Ch {
    http: Client,
    url: String,
    user: String,
    pw: String,
    current: Arc<Mutex<String>>,
}

static SEQ: AtomicU64 = AtomicU64::new(1);

impl Ch {
    pub fn connect(cfg: &ConnConfig, pw: &str, drv: &Driver) -> Result<Self> {
        let https = opt(cfg, drv, "https") == Some("true");
        if !https && !pw.is_empty() && !loopback(&cfg.host) && opt(cfg, drv, "allow_http") != Some("true") {
            bail!("refusing to send the password unencrypted over HTTP to {}: set https=true (port 8443), or allow_http=true to accept the risk", cfg.host);
        }
        let scheme = if https { "https" } else { "http" };
        let db = if cfg.database.is_empty() { "default" } else { &cfg.database };
        let ch = Ch {
            http: Client::builder().timeout(None).build()?,
            url: format!("{scheme}://{}:{}/?database={db}", cfg.host, port(cfg, drv)),
            user: cfg.user.clone(),
            pw: pw.into(),
            current: Arc::default(),
        };
        ch.post("SELECT 1", "")?;
        Ok(ch)
    }

    fn post(&self, sql: &str, query_id: &str) -> Result<reqwest::blocking::Response> {
        let r = self
            .http
            .post(format!("{}&query_id={query_id}", self.url))
            .header("X-ClickHouse-User", &self.user)
            .header("X-ClickHouse-Key", &self.pw)
            .body(sql.to_owned())
            .send()?;
        if !r.status().is_success() {
            bail!("{}", r.text().unwrap_or_default().trim());
        }
        Ok(r)
    }
}

fn loopback(host: &str) -> bool {
    host.is_empty() || host.eq_ignore_ascii_case("localhost") || host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// TSV field unescape; `\N` is NULL.
fn field(s: &str) -> Option<String> {
    if s == "\\N" {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    Some(out)
}

impl Backend for Ch {
    fn execute(&mut self, sql: &str, fetch: usize, sink: &mut dyn FnMut(Event) -> bool) -> Result<Option<u64>> {
        let qid = format!("rsqldev-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed));
        *self.current.lock().unwrap() = qid.clone();
        let body = sql.trim_end().trim_end_matches(';');
        if !is_query(sql) {
            let r = self.post(body, &qid)?;
            let written = r
                .headers()
                .get("X-ClickHouse-Summary")
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.split("\"written_rows\":\"").nth(1)?.split('"').next()?.parse().ok());
            return Ok(written);
        }
        let r = self.post(&format!("{body}\nFORMAT TabSeparatedWithNamesAndTypes"), &qid)?;
        let mut lines = BufReader::new(r).lines();
        let names: Vec<String> = lines.next().transpose()?.unwrap_or_default().split('\t').map(|s| field(s).unwrap_or_default()).collect();
        let types: Vec<String> = lines.next().transpose()?.unwrap_or_default().split('\t').map(|s| field(s).unwrap_or_default()).collect();
        let cols = names
            .into_iter()
            .zip(types)
            .map(|(name, t)| Column { name, numeric: numeric_type(&t) && !t.contains("Array"), type_name: t })
            .collect();
        let mut b = Batcher::new(sink, fetch);
        if !b.columns(cols) {
            return Ok(None);
        }
        for line in lines {
            if !b.push(line?.split('\t').map(field).collect()) {
                return Ok(None); // dropping the response closes the HTTP stream, server aborts the query
            }
        }
        b.finish();
        Ok(None)
    }

    fn commit(&mut self) -> Result<()> {
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        Ok(())
    }

    fn set_autocommit(&mut self, _: bool) -> Result<()> {
        Ok(())
    }

    fn canceller(&self) -> Option<Canceller> {
        let (http, url, user, pw, cur) = (self.http.clone(), self.url.clone(), self.user.clone(), self.pw.clone(), self.current.clone());
        Some(Box::new(move || {
            let q = cur.lock().unwrap().clone();
            let _ = http
                .post(&url)
                .header("X-ClickHouse-User", &user)
                .header("X-ClickHouse-Key", &pw)
                .body(format!("KILL QUERY WHERE query_id = '{q}' ASYNC"))
                .send();
        }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tsv_field() {
        assert_eq!(super::field("\\N"), None);
        assert_eq!(super::field("a\\tb\\\\c").as_deref(), Some("a\tb\\c"));
    }

    #[test]
    fn loopback_hosts() {
        assert!(super::loopback("localhost") && super::loopback("127.0.0.1") && super::loopback("[::1]"));
        assert!(!super::loopback("db.example.com") && !super::loopback("10.0.0.5"));
    }
}
