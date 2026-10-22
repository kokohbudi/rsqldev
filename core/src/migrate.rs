//! Import connections from Oracle SQL Developer and DBeaver. Passwords are left behind (both
//! tools store them encrypted with their own keys): rsqldev asks on first connect.

use crate::config::ConnConfig;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub conn: ConnConfig,
    /// "SQL Developer" or "DBeaver".
    pub source: &'static str,
}

/// Connection files of both tools in their default locations.
pub fn default_files() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else { return vec![] };
    let roaming = dirs::config_dir().unwrap_or_else(|| home.clone());
    let mut roots: Vec<(PathBuf, &str)> = vec![];
    // SQL Developer: <root>/system*/o.jdeveloper.db.connection*/connections.{json,xml}
    for r in [home.join(".sqldeveloper"), roaming.join("SQL Developer")] {
        roots.push((r, "sqldev"));
    }
    // DBeaver: <data>/workspace6/<project>/.dbeaver/data-sources*.json
    for r in [home.join("Library/DBeaverData/workspace6"), home.join(".local/share/DBeaverData/workspace6"), roaming.join("DBeaverData/workspace6")] {
        roots.push((r, "dbeaver"));
    }
    let list = |p: &Path| std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).collect::<Vec<_>>();
    let mut out = vec![];
    for (root, kind) in roots {
        for a in list(&root) {
            if kind == "sqldev" && a.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("system")) {
                for b in list(&a).into_iter().filter(|b| b.to_string_lossy().contains("o.jdeveloper.db.connection")) {
                    out.extend(["connections.json", "connections.xml"].iter().map(|f| b.join(f)).filter(|f| f.is_file()));
                }
            } else if kind == "dbeaver" {
                out.extend(list(&a.join(".dbeaver")).into_iter().filter(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("data-sources") && n.ends_with(".json"))));
            }
        }
    }
    out
}

/// Parse any supported file (detected by content).
pub fn read(path: &Path) -> crate::Result<Vec<Found>> {
    let text = std::fs::read_to_string(path)?;
    Ok(if text.trim_start().starts_with('<') {
        sqldeveloper_xml(&text)
    } else {
        let v: Value = serde_json::from_str(&text)?;
        if v.get("connections").is_some_and(Value::is_array) { sqldeveloper_json(&v) } else { dbeaver(&v) }
    })
}

/// Driver id from a tool's provider / driver / URL text.
fn driver_id(hint: &str) -> Option<&'static str> {
    let h = hint.to_ascii_lowercase();
    [
        ("redshift", "redshift"),
        ("cockroach", "cockroach"),
        ("yugabyte", "yugabyte"),
        ("timescale", "timescale"),
        ("greenplum", "greenplum"),
        ("materialize", "materialize"),
        ("mariadb", "mariadb"),
        ("tidb", "tidb"),
        ("mysql", "mysql"),
        ("postgres", "postgres"),
        ("azure", "azuresql"),
        ("sqlserver", "sqlserver"),
        ("mssql", "sqlserver"),
        ("jtds", "sqlserver"),
        ("sqlite", "sqlite"),
        ("duckdb", "duckdb"),
        ("clickhouse", "clickhouse"),
        ("jdbc:ch:", "clickhouse"),
        ("oracle", "oracle"),
    ]
    .iter()
    .find(|(k, _)| h.contains(k))
    .map(|(_, id)| *id)
}

/// Fill host / port / database (or Oracle descriptor) from a JDBC URL.
pub fn apply_jdbc_url(c: &mut ConnConfig, url: &str) {
    let rest = url.splitn(3, ':').nth(2).unwrap_or("");
    if c.driver == "oracle" {
        let Some(t) = url.split_once('@').map(|x| x.1) else { return };
        if t.starts_with('(') {
            c.conn_type = "custom".into();
            c.tns = t.into();
            return;
        }
        let t = t.trim_start_matches("//");
        let (hp, svc, sid) = match (t.split_once('/'), t.rsplitn(3, ':').collect::<Vec<_>>().as_slice()) {
            (Some((hp, svc)), _) => (hp.to_string(), svc.to_string(), false),
            (None, [sid, port, host]) => (format!("{host}:{port}"), sid.to_string(), true),
            _ => return,
        };
        let (h, p) = hp.split_once(':').unwrap_or((&hp, ""));
        c.conn_type = "basic".into();
        c.host = h.into();
        c.port = p.parse().ok();
        c.database = svc;
        c.sid = sid;
        return;
    }
    if matches!(c.driver.as_str(), "sqlite" | "duckdb") {
        c.database = rest.trim_start_matches("//").into();
        return;
    }
    // //host:port/db?x or //host:port;databaseName=db
    let r = rest.split_once("//").map_or(rest, |x| x.1);
    let (hp, tail) = r.split_once(['/', ';', '?']).unwrap_or((r, ""));
    let (h, p) = hp.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()).unwrap_or((hp, ""));
    c.host = h.into();
    c.port = p.parse().ok();
    let db = tail.split(';').find_map(|kv| kv.strip_prefix("databaseName=").or_else(|| kv.strip_prefix("database="))).unwrap_or_else(|| tail.split(['?', ';']).next().unwrap_or(""));
    if !db.contains('=') {
        c.database = db.into();
    }
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn sqldev_conn(name: String, get: &dyn Fn(&str) -> String) -> Option<ConnConfig> {
    let raptor = get("RaptorConnectionType");
    let url = get("customUrl");
    let id = driver_id(&raptor).or_else(|| driver_id(&get("driver"))).or_else(|| driver_id(&url)).unwrap_or("oracle");
    let mut c = ConnConfig { name, driver: id.into(), user: get("user"), role: get("role").to_uppercase(), ..Default::default() };
    if id == "oracle" {
        match get("OracleConnectionType").to_uppercase().as_str() {
            "TNS" => {
                c.conn_type = "tns".into();
                c.tns = ["TnsName", "tnsName", "TNS_ALIAS"].iter().map(|k| get(k)).find(|v| !v.is_empty()).unwrap_or_default();
            }
            "CUSTOM" | "JDBC_URL" if !url.is_empty() => apply_jdbc_url(&mut c, &url),
            _ => {
                c.conn_type = "basic".into();
                c.host = get("hostname");
                c.port = get("port").parse().ok();
                let svc = get("serviceName");
                c.sid = svc.is_empty();
                c.database = if svc.is_empty() { get("sid") } else { svc };
            }
        }
    } else if !url.is_empty() {
        apply_jdbc_url(&mut c, &url);
    } else {
        c.host = get("hostname");
        c.port = get("port").parse().ok();
        c.database = get("database");
    }
    Some(c)
}

/// SQL Developer 4.2+ `connections.json`.
pub fn sqldeveloper_json(v: &Value) -> Vec<Found> {
    v["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let info = &c["info"];
            let name = if s(c, "name").is_empty() { s(info, "ConnName") } else { s(c, "name") };
            sqldev_conn(name, &|k| s(info, k))
        })
        .map(|conn| Found { conn, source: "SQL Developer" })
        .collect()
}

/// Older SQL Developer `connections.xml`:
/// `<Reference name="X"><RefAddresses><StringRefAddr addrType="hostname"><Contents>h</Contents>…`.
pub fn sqldeveloper_xml(text: &str) -> Vec<Found> {
    let unescape = |s: &str| s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&");
    let attr = |tag: &str, a: &str| tag.split(&format!("{a}=\"")).nth(1).and_then(|x| x.split('"').next()).map(unescape);
    text.split("<Reference ")
        .skip(1)
        .filter_map(|r| {
            let r = r.split("</Reference>").next()?;
            let name = attr(r.split('>').next()?, "name")?;
            let pairs: Vec<(String, String)> = r
                .split("<StringRefAddr ")
                .skip(1)
                .filter_map(|p| Some((attr(p, "addrType")?, unescape(p.split("<Contents>").nth(1)?.split("</Contents>").next()?))))
                .collect();
            let get = |k: &str| pairs.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone()).unwrap_or_default();
            sqldev_conn(name, &get)
        })
        .map(|conn| Found { conn, source: "SQL Developer" })
        .collect()
}

/// DBeaver `data-sources.json`.
pub fn dbeaver(v: &Value) -> Vec<Found> {
    let Some(conns) = v["connections"].as_object() else { return vec![] };
    conns
        .values()
        .filter_map(|c| {
            let cfg = &c["configuration"];
            let url = s(cfg, "url");
            let id = driver_id(&s(c, "driver")).or_else(|| driver_id(&s(c, "provider"))).or_else(|| driver_id(&url))?;
            let mut conn = ConnConfig { name: s(c, "name"), driver: id.into(), folder: s(c, "folder"), user: s(cfg, "user"), ..Default::default() };
            if id == "oracle" && !url.is_empty() && s(cfg, "host").is_empty() {
                apply_jdbc_url(&mut conn, &url);
            } else if matches!(id, "sqlite" | "duckdb") {
                conn.database = if s(cfg, "database").is_empty() { url.splitn(3, ':').nth(2).unwrap_or("").into() } else { s(cfg, "database") };
            } else {
                conn.host = s(cfg, "host");
                conn.port = s(cfg, "port").parse().ok();
                conn.database = s(cfg, "database");
                if id == "oracle" {
                    conn.conn_type = "basic".into();
                    // DBeaver keeps service name / SID choice in provider properties.
                    conn.sid = cfg["provider-properties"]["@dbeaver-sid-service@"].as_str() == Some("SID");
                }
                if conn.host.is_empty() && !url.is_empty() {
                    apply_jdbc_url(&mut conn, &url);
                }
            }
            Some(Found { conn, source: "DBeaver" })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqldeveloper() {
        let j = r#"{"connections":[
          {"name":"HR","type":"jdbc","info":{"OracleConnectionType":"BASIC","RaptorConnectionType":"Oracle","hostname":"db1","port":"1521","serviceName":"ORCLPDB1","user":"hr","role":"sysdba"}},
          {"name":"OLD","type":"jdbc","info":{"OracleConnectionType":"BASIC","hostname":"db2","port":"1522","sid":"XE","user":"scott"}},
          {"name":"PG","type":"jdbc","info":{"RaptorConnectionType":"PostgreSQL","customUrl":"jdbc:postgresql://pg:5433/app","user":"u"}}]}"#;
        let f = sqldeveloper_json(&serde_json::from_str(j).unwrap());
        assert_eq!(f[0].conn, ConnConfig { name: "HR".into(), driver: "oracle".into(), host: "db1".into(), port: Some(1521), database: "ORCLPDB1".into(), user: "hr".into(), conn_type: "basic".into(), role: "SYSDBA".into(), ..Default::default() });
        assert!(f[1].conn.sid && f[1].conn.database == "XE" && f[1].conn.port == Some(1522));
        assert_eq!((f[2].conn.driver.as_str(), f[2].conn.host.as_str(), f[2].conn.port, f[2].conn.database.as_str()), ("postgres", "pg", Some(5433), "app"));
        let x = r#"<References><Reference name="Prod &amp; Co" className="oracle.jdeveloper.db.adapter.DatabaseProvider"><RefAddresses>
          <StringRefAddr addrType="hostname"><Contents>h</Contents></StringRefAddr><StringRefAddr addrType="port"><Contents>1521</Contents></StringRefAddr>
          <StringRefAddr addrType="serviceName"><Contents>S</Contents></StringRefAddr><StringRefAddr addrType="user"><Contents>u</Contents></StringRefAddr>
          </RefAddresses></Reference></References>"#;
        let f = sqldeveloper_xml(x);
        assert_eq!((f[0].conn.name.as_str(), f[0].conn.host.as_str(), f[0].conn.database.as_str()), ("Prod & Co", "h", "S"));
    }

    #[test]
    fn dbeaver_and_urls() {
        let j = r#"{"folders":{"Prod":{}},"connections":{
          "postgres-jdbc-1":{"provider":"postgresql","driver":"postgres-jdbc","name":"PG","folder":"Prod","configuration":{"host":"pg","port":"5432","database":"app","url":"jdbc:postgresql://pg:5432/app"}},
          "ms-1":{"provider":"sqlserver","driver":"microsoft","name":"MS","configuration":{"url":"jdbc:sqlserver://ms:1433;databaseName=sales;encrypt=true"}},
          "lite":{"provider":"generic","driver":"sqlite_jdbc","name":"L","configuration":{"database":"/tmp/a.db","url":"jdbc:sqlite:/tmp/a.db"}},
          "x":{"provider":"generic","driver":"unknown","name":"X","configuration":{}}}}"#;
        let mut f = dbeaver(&serde_json::from_str(j).unwrap());
        f.sort_by(|a, b| a.conn.name.cmp(&b.conn.name));
        assert_eq!(f.len(), 3, "unknown driver skipped");
        assert_eq!((f[0].conn.driver.as_str(), f[0].conn.database.as_str()), ("sqlite", "/tmp/a.db"));
        assert_eq!((f[1].conn.host.as_str(), f[1].conn.port, f[1].conn.database.as_str()), ("ms", Some(1433), "sales"));
        assert_eq!((f[2].conn.folder.as_str(), f[2].conn.driver.as_str()), ("Prod", "postgres"));
        let mut c = ConnConfig { driver: "oracle".into(), ..Default::default() };
        apply_jdbc_url(&mut c, "jdbc:oracle:thin:@db:1521:XE");
        assert!(c.sid && c.host == "db" && c.database == "XE");
        apply_jdbc_url(&mut c, "jdbc:oracle:thin:@(DESCRIPTION=(ADDRESS=(HOST=h)))");
        assert_eq!(c.conn_type, "custom");
    }
}
