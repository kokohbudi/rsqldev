//! Integration tests against real servers from `docker-compose.test.yml`:
//!   docker compose -f docker-compose.test.yml up -d
//!   cargo test -p rsqldev-core --test backends -- --ignored --test-threads 1
//! Each test runs every dialect query (folders, viewer tabs, DDL, explain) and a data-editor round trip.

use rsqldev_core::config::{ConnConfig, Prefs};
use rsqldev_core::dialect::{self, View};
use rsqldev_core::registry::{self, Kind};
use rsqldev_core::worker::{Reply, Session};
use rsqldev_core::{Backend, Bind, Event, backend, cell_bytes, dml, download, is_binary, query_all};

fn cfg(driver: &str, port: u16, db: &str, user: &str) -> ConnConfig {
    ConnConfig { name: format!("it-{driver}"), driver: driver.into(), host: "127.0.0.1".into(), port: Some(port), database: db.into(), user: user.into(), ..Default::default() }
}

fn run(b: &mut dyn Backend, sql: &str) {
    b.execute(sql, 50, &mut |_| true).unwrap_or_else(|e| panic!("{sql}\n=> {e:#}"));
}

fn exercise(c: ConnConfig, pw: &str, owner: &str) {
    let db = registry::find(&c.driver).unwrap().backend;
    let mut bx = backend::connect(&c, pw, &Prefs { autocommit: false, ..Prefs::default() }).expect("connect");
    let b = bx.as_mut();
    let _ = b.execute("DROP TABLE rsq_t", 1, &mut |_| true);
    let _ = b.commit();
    run(
        b,
        match db {
            Kind::Clickhouse => "CREATE TABLE rsq_t (id Int32, name Nullable(String), d Nullable(Date), b Nullable(String), c Nullable(String)) ENGINE = MergeTree ORDER BY id",
            Kind::Oracle => "CREATE TABLE rsq_t (id NUMBER(9) PRIMARY KEY, name VARCHAR2(50), d DATE, b BLOB, tz TIMESTAMP WITH TIME ZONE, c CLOB)",
            Kind::Postgres => "CREATE TABLE rsq_t (id INT PRIMARY KEY, name VARCHAR(50), d DATE, b BYTEA, c TEXT)",
            Kind::Mysql => "CREATE TABLE rsq_t (id INT PRIMARY KEY, name VARCHAR(50), d DATE, b LONGBLOB, bt BIT(8) DEFAULT b'101', tx TEXT, c TEXT)",
            Kind::Mssql => "CREATE TABLE rsq_t (id INT PRIMARY KEY, name VARCHAR(50), d DATE, b VARBINARY(MAX), c NVARCHAR(MAX))",
            _ => "CREATE TABLE rsq_t (id INT PRIMARY KEY, name VARCHAR(50), d DATE, b BLOB, c TEXT)",
        },
    );
    for i in 0..120 {
        let d = if db == Kind::Oracle { "DATE '2026-01-02'".to_string() } else { "'2026-01-02'".to_string() };
        run(b, &format!("INSERT INTO rsq_t (id, name, d) VALUES ({i}, 'n''{i}', {d})"));
    }
    if db == Kind::Oracle {
        run(b, "UPDATE rsq_t SET tz = TIMESTAMP '2026-01-02 03:04:05 +07:00'");
    }
    b.commit().unwrap();
    if db == Kind::Oracle {
        // Compilation errors come back as an error listing ALL_ERRORS (SHOW ERRORS).
        let e = b.execute("CREATE OR REPLACE PROCEDURE rsq_bad AS\nBEGIN\n  no_such_thing;\nEND;", 1, &mut |_| true).unwrap_err().to_string();
        assert!(e.contains("Procedure RSQ_BAD compiled with errors") && e.contains("3/3") && e.contains("PLS-"), "{e}");
        run(b, "CREATE OR REPLACE PROCEDURE rsq_bad AS\nBEGIN\n  NULL;\nEND;");
        run(b, "DROP PROCEDURE rsq_bad");
    }
    if db == Kind::Mysql {
        let (c, r) = query_all(b, "SELECT bt, b, tx FROM rsq_t WHERE id = 1").unwrap();
        assert_eq!(r[0][0].as_deref(), Some("5"), "BIT renders as a number");
        assert!(is_binary(&c[1].type_name) && !is_binary(&c[2].type_name), "{c:?}");
    }

    // Full fetch + early stop after the first batch (fetch-on-scroll path).
    let (cols, rows) = query_all(b, "SELECT id, name, d FROM rsq_t ORDER BY id").unwrap();
    assert_eq!(cols.len(), 3);
    assert_eq!(rows.len(), 120);
    assert_eq!(rows[1][1].as_deref(), Some("n'1"));
    assert!(cols[0].numeric, "{:?}", cols[0]);
    let mut batches = 0;
    b.execute("SELECT id FROM rsq_t ORDER BY id", 50, &mut |e| {
        if let Event::Rows(r) = e {
            assert_eq!(r.len(), 50);
            batches += 1;
            return false;
        }
        true
    })
    .unwrap();
    assert_eq!(batches, 1);
    run(b, "SELECT 1 FROM rsq_t WHERE id = 1"); // connection still usable after abandoning a cursor
    let cnt = dialect::count_sql(db, "SELECT id, name FROM rsq_t ORDER BY id").unwrap();
    let cnt = if db == Kind::Mssql { cnt.replace(" ORDER BY id", "") } else { cnt }; // T-SQL: no ORDER BY in a derived table
    assert_eq!(query_all(b, &cnt).unwrap().1[0][0].as_deref(), Some("120"), "{cnt}");

    // Session monitor: the first candidate that runs lists at least this session.
    if let Some(qs) = dialect::sessions_sql(db) {
        let res: Vec<_> = qs.iter().map(|q| query_all(b, q)).collect();
        match res.iter().find_map(|r| r.as_ref().ok()) {
            Some((_, rows)) => assert!(!rows.is_empty() || db == Kind::Clickhouse, "no sessions listed"),
            // The app user may lack SELECT on v$session; the UI shows that message.
            None => assert!(db == Kind::Oracle && format!("{:#}", res[0].as_ref().unwrap_err()).contains("ORA-00942"), "{:?}", res.iter().map(|r| r.as_ref().err().map(|e| format!("{e:#}"))).collect::<Vec<_>>()),
        }
    }

    // Object browser: every folder query and every viewer tab for the test table.
    let schemas = query_all(b, dialect::schemas_sql(db)).unwrap().1;
    assert!(schemas.iter().any(|r| r[0].as_deref().map(|s| s.eq_ignore_ascii_case(owner)) == Some(true)), "{owner} not in {schemas:?}");
    let cur = query_all(b, dialect::current_schema_sql(db)).unwrap().1;
    assert!(cur[0][0].is_some());
    for (_, kind) in dialect::folders(db) {
        let sql = dialect::list_sql(db, kind, owner).unwrap();
        query_all(b, &sql).unwrap_or_else(|e| panic!("{kind}: {sql}\n=> {e:#}"));
    }
    let tables = query_all(b, &dialect::list_sql(db, dialect::TABLE, owner).unwrap()).unwrap().1;
    let name = tables.iter().filter_map(|r| r[0].clone()).find(|n| n.eq_ignore_ascii_case("rsq_t")).expect("rsq_t listed");
    // Completion: the columns query's first column is the column name.
    let dsql = dialect::describe_sql(db, owner, &name).unwrap();
    let names: Vec<String> = query_all(b, &dsql).unwrap().1.into_iter().filter_map(|r| r[0].clone()).collect();
    assert!(names.iter().any(|n| n.eq_ignore_ascii_case("name")) && names.iter().any(|n| n.eq_ignore_ascii_case("id")), "{dsql} => {names:?}");
    for t in dialect::tabs(db, dialect::TABLE) {
        let sql = match t.view {
            View::Data => dialect::data_query(db, dialect::TABLE, owner, &name).sql,
            View::Grid(s) | View::Details(s) | View::Code(s) | View::Ddl(s, _) => dialect::fill(db, &s, owner, &name),
        };
        let (_, r) = query_all(b, &sql).unwrap_or_else(|e| panic!("tab {}: {sql}\n=> {e:#}", t.label));
        if t.label == "SQL" || t.label == "Columns" {
            assert!(!r.is_empty() && r[0].iter().any(Option::is_some), "tab {} empty", t.label);
        }
    }
    let ex = dialect::explain(db, "SELECT * FROM rsq_t WHERE id = 5");
    let mut shown = vec![];
    for (i, s) in ex.stmts.iter().enumerate() {
        let r = query_all(b, s).unwrap_or_else(|e| panic!("explain {s}\n=> {e:#}")).1;
        if i == ex.show {
            shown = r;
        }
    }
    assert!(!shown.is_empty(), "explain returned no rows");

    // Data editor round trip.
    let dq = dialect::data_query(db, dialect::TABLE, owner, &name);
    if dq.editable {
        let (cols, rows) = query_all(b, &format!("{} WHERE id = 7", dq.sql)).unwrap();
        let pk: Vec<String> = dialect::pk_sql(db, owner, &name).map(|s| query_all(b, &s).unwrap().1.into_iter().filter_map(|r| r[0].clone()).collect()).unwrap_or_default();
        let key = dml::Key::new(&cols, dq.hidden, &pk);
        assert!(!key.all_columns, "no key for {db:?}");
        let skip = dq.hidden.map(|_| 0);
        let table = dialect::qualified(db, owner, &name);
        let mut new = rows[0].clone();
        let ni = cols.iter().position(|c| c.name.eq_ignore_ascii_case("name")).unwrap();
        new[ni] = Some("edited".into());
        let di = cols.iter().position(|c| c.name.eq_ignore_ascii_case("d")).unwrap();
        new[di] = rows[0][di].clone(); // unchanged date must round-trip through the literal
        let up = dml::update(db, &table, &cols, &key, skip, &rows[0], &new, &[]).unwrap();
        assert_eq!(b.execute(&up, 1, &mut |_| true).unwrap_or_else(|e| panic!("{up}\n=> {e:#}")), Some(1));
        let mut row = vec![None; cols.len()];
        row[cols.iter().position(|c| c.name.eq_ignore_ascii_case("id")).unwrap()] = Some("500".into());
        row[ni] = Some("x".into());
        row[di] = rows[0][di].clone();
        // Fetched TIMESTAMP WITH TIME ZONE text must convert back through its literal.
        if let Some(ti) = cols.iter().position(|c| c.name.eq_ignore_ascii_case("tz")) {
            row[ti] = rows[0][ti].clone();
        }
        // Imported ISO dates insert on every engine (Oracle via TO_TIMESTAMP, not NLS).
        let mut iso = vec![None; cols.len()];
        iso[cols.iter().position(|c| c.name.eq_ignore_ascii_case("id")).unwrap()] = Some("501".into());
        iso[di] = Some("2026-03-04".into());
        let ins = dml::insert(db, &table, &cols, skip, &iso, &[]);
        run(b, &ins);
        let got = query_all(b, "SELECT d FROM rsq_t WHERE id = 501").unwrap().1;
        assert!(got[0][0].as_deref().is_some_and(|d| d.contains("26") && (d.contains("03") || d.contains("MAR"))), "{ins} => {got:?}");
        run(b, "DELETE FROM rsq_t WHERE id = 501");
        // BLOB upload through binds: > 2000 bytes takes Oracle's temporary-LOB path.
        let bi = cols.iter().position(|c| c.name.eq_ignore_ascii_case("b")).unwrap();
        assert!(is_binary(&cols[bi].type_name), "{:?}", cols[bi]);
        // 100 KB: past lob::INLINE, so the fetched cell is spilled to disk and read back from there.
        let payload: Vec<u8> = (0..100_000u32).map(|i| (i * 7) as u8).collect();
        let ins = dml::insert(db, &table, &cols, skip, &row, &[bi]);
        assert_eq!(b.execute_binds(&ins, &[Bind::Bytes(payload.clone())]).unwrap_or_else(|e| panic!("{ins}\n=> {e:#}")), Some(1));
        let up = dml::update(db, &table, &cols, &key, skip, &new, &new, &[bi]).unwrap();
        assert_eq!(b.execute_binds(&up, &[Bind::Bytes(vec![0x50, 0x4B, 0, 0xFF])]).unwrap_or_else(|e| panic!("{up}\n=> {e:#}")), Some(1));
        let (bc, br) = query_all(b, "SELECT b FROM rsq_t WHERE id = 500").unwrap();
        assert!(rsqldev_core::lob::is_lob(br[0][0].as_deref().unwrap()), "large BLOB spilled, not held in memory");
        assert_eq!(cell_bytes(&bc[0], br[0][0].as_deref().unwrap()), payload, "BLOB download (bind insert)");
        let (bc, br) = query_all(b, "SELECT b FROM rsq_t WHERE id = 7").unwrap();
        assert_eq!(cell_bytes(&bc[0], br[0][0].as_deref().unwrap()), vec![0x50, 0x4B, 0, 0xFF], "BLOB download (bind update)");
        // Hex cell text written back through the engine's binary literal.
        let mut lit = new.clone();
        lit[bi] = br[0][0].clone();
        let mut orig = new.clone();
        orig[bi] = None;
        let up = dml::update(db, &table, &cols, &key, skip, &orig, &lit, &[]).unwrap();
        run(b, &up);
        let br = query_all(b, "SELECT b FROM rsq_t WHERE id = 7").unwrap().1;
        assert_eq!(cell_bytes(&bc[0], br[0][0].as_deref().unwrap()), vec![0x50, 0x4B, 0, 0xFF], "BLOB literal");
        // Long text (CLOB edits): over Oracle's 4000-byte literal cap, multi-byte on purpose.
        let ci = cols.iter().position(|c| c.name.eq_ignore_ascii_case("c")).unwrap();
        let long: String = "héllo wörld ".repeat(500);
        let mut txt = new.clone();
        txt[ci] = Some(long.clone());
        assert_eq!(dml::text_needs_bind(db, &cols[ci], &txt[ci]), db == Kind::Oracle);
        let binds = if db == Kind::Oracle { vec![ci] } else { vec![] };
        let up = dml::update(db, &table, &cols, &key, skip, &new, &txt, &binds).unwrap();
        let bound: Vec<Bind> = binds.iter().map(|_| Bind::Text(long.clone())).collect();
        let n = if bound.is_empty() { b.execute(&up, 1, &mut |_| true) } else { b.execute_binds(&up, &bound) };
        assert_eq!(n.unwrap_or_else(|e| panic!("{}\n=> {e:#}", &up[..up.len().min(200)])), Some(1));
        let cr = query_all(b, "SELECT c FROM rsq_t WHERE id = 7").unwrap().1;
        assert_eq!(cr[0][0].as_deref(), Some(long.as_str()), "long text round trip");
        let del = dml::delete(db, &table, &cols, &key, &new);
        run(b, &del);
        b.commit().unwrap();
        let after = query_all(b, "SELECT COUNT(*) FROM rsq_t WHERE name = 'edited' OR id = 500").unwrap().1;
        assert_eq!(after[0][0].as_deref(), Some("1"), "update/insert/delete round trip");
    }

    // Release locks held by the manual-commit session before another session drops the table.
    b.commit().unwrap();
    drop(bx);

    // Worker + rendezvous channel.
    let mut s = Session::open(c, pw.into(), Prefs::default());
    assert!(matches!(s.replies.recv().unwrap(), Reply::Connected));
    let (_, rx) = s.exec("SELECT id FROM rsq_t ORDER BY id", 50);
    assert!(matches!(rx.recv().unwrap(), Event::Columns(_)));
    assert!(matches!(rx.recv().unwrap(), Event::Rows(r) if r.len() == 50));
    drop(rx);
    assert!(matches!(s.replies.recv().unwrap(), Reply::Done { result: Ok(_), .. }));
    let (_, rx) = s.exec("DROP TABLE rsq_t", 50);
    drop(rx);
    let r = s.replies.recv().unwrap();
    assert!(matches!(r, Reply::Done { result: Ok(_), .. }), "{r:?}");
}

#[test]
#[ignore]
fn postgres() {
    exercise(cfg("postgres", 15432, "postgres", "postgres"), "rsqldev", "public");
}

#[test]
#[ignore]
fn mysql() {
    exercise(cfg("mysql", 13306, "rsq", "root"), "rsqldev", "rsq");
}

#[test]
#[ignore]
fn mssql() {
    // Secure default: the container's self-signed certificate is rejected without trust_cert.
    let e = backend::connect(&cfg("sqlserver", 11433, "master", "sa"), "Rsqldev#2026", &Prefs::default()).err().expect("self-signed cert must be rejected");
    eprintln!("expected TLS rejection: {e:#}");
    let mut c = cfg("sqlserver", 11433, "master", "sa");
    c.options.insert("trust_cert".into(), "true".into()); // container uses a self-signed certificate
    exercise(c, "Rsqldev#2026", "dbo");
}

#[test]
#[ignore]
fn clickhouse() {
    exercise(cfg("clickhouse", 18123, "default", "default"), "rsqldev", "default");
}

#[test]
fn sqlite() {
    let f = std::env::temp_dir().join(format!("rsq-{}.sqlite", std::process::id()));
    let c = ConnConfig { name: "it-sqlite".into(), driver: "sqlite".into(), database: f.display().to_string(), ..Default::default() };
    exercise(c.clone(), "", "main");
    // Expression / CTE columns have no declared type: numeric from the first row's values, so
    // the grid sorts them as numbers.
    let mut b = backend::connect(&c, "", &Prefs::default()).unwrap();
    let (cols, _) = query_all(b.as_mut(), "select 1 + 1 n, 2.5 r, 'x' t, null z union all select 3, 4, 'y', 5").unwrap();
    assert_eq!(cols.iter().map(|c| c.numeric).collect::<Vec<_>>(), vec![true, true, false, false]);
    let _ = std::fs::remove_file(f);
}

#[test]
fn duckdb() {
    let f = std::env::temp_dir().join(format!("rsq-{}.duckdb", std::process::id()));
    exercise(ConnConfig { name: "it-duckdb".into(), driver: "duckdb".into(), database: f.display().to_string(), ..Default::default() }, "", "main");
    let _ = std::fs::remove_file(f);
}

/// SSH tunnel through the compose `ssh` bastion to the postgres service (system ssh client).
#[test]
#[ignore]
fn ssh_tunnel() {
    let dir = std::env::temp_dir().join(format!("rsq-ssh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("id");
    let ok = |c: &mut std::process::Command| assert!(c.status().unwrap().success(), "{c:?}");
    ok(std::process::Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&key));
    let public = std::fs::read_to_string(dir.join("id.pub")).unwrap();
    let compose = concat!(env!("CARGO_MANIFEST_DIR"), "/../docker-compose.test.yml");
    ok(std::process::Command::new("docker").args(["compose", "-f", compose, "exec", "-T", "ssh", "sh", "-c"]).arg(format!("echo '{}' >> /config/.ssh/authorized_keys", public.trim())));
    // The container's host key changes when it is recreated: forget the old one (accept-new re-learns it).
    let _ = std::process::Command::new("ssh-keygen").args(["-R", "[127.0.0.1]:12222"]).output();
    let mut c = cfg("postgres", 5432, "postgres", "postgres");
    c.host = "postgres".into(); // resolved by the bastion, not by us
    c.options.insert("ssh".into(), "rsq@127.0.0.1:12222".into());
    c.options.insert("ssh_key".into(), key.display().to_string());
    let mut b = backend::connect(&c, "rsqldev", &Prefs::default()).unwrap_or_else(|e| panic!("{e:#}"));
    let r = query_all(b.as_mut(), "SELECT inet_server_port()").unwrap().1;
    assert_eq!(r[0][0].as_deref(), Some("5432"));
    drop(b);
    let _ = std::fs::remove_dir_all(dir);
}

/// Also exercises the driver downloader: installs Instant Client on first run.
#[test]
#[ignore]
fn oracle() {
    let d = registry::find("oracle").unwrap();
    if download::installed_dir(d).is_none() {
        let mut last = 0;
        download::install(d, &mut |done, total| {
            if done - last > 20 << 20 {
                last = done;
                eprintln!("instant client {} / {:?} MB", done >> 20, total.map(|t| t >> 20));
            }
            true
        })
        .unwrap();
    }
    exercise(cfg("oracle", 15210, "FREEPDB1", "rsq"), "rsqldev", "RSQ");
}
