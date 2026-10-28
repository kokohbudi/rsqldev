//! Per-engine metadata: object browser folders, list queries, object viewer tabs, DDL,
//! explain plan and the data-tab query. Queries are templates:
//! `{o}`/`{n}` = owner/name escaped for a string literal, `{q}` = quoted qualified name,
//! `{lq}` = `{q}` escaped for a string literal.

use crate::registry::Kind as Db;

/// Object kinds shown as folders. Oracle-style names everywhere so the UI has one vocabulary.
pub const TABLE: &str = "TABLE";
pub const VIEW: &str = "VIEW";
pub const MVIEW: &str = "MATERIALIZED VIEW";
pub const INDEX: &str = "INDEX";
pub const PACKAGE: &str = "PACKAGE";
pub const PROCEDURE: &str = "PROCEDURE";
pub const FUNCTION: &str = "FUNCTION";
pub const TRIGGER: &str = "TRIGGER";
pub const TYPE: &str = "TYPE";
pub const SEQUENCE: &str = "SEQUENCE";
pub const SYNONYM: &str = "SYNONYM";
pub const DBLINK: &str = "DATABASE LINK";
/// Oracle folders that are not one owner's objects.
pub const DIRECTORY: &str = "DIRECTORY";
pub const PUBLIC_SYNONYM: &str = "PUBLIC SYNONYM";
pub const RECYCLE: &str = "RECYCLE BIN";
pub const JOB: &str = "JOB";

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    /// Multi-row grid.
    Grid(String),
    /// Single row shown transposed as Name / Value.
    Details(String),
    /// Editable data grid (see [`data_query`]).
    Data,
    /// Source text: concatenate column 0 of every row.
    Code(String),
    /// DDL text: column `.1` of the first row.
    Ddl(String, usize),
}

pub struct Tab {
    pub label: &'static str,
    pub view: View,
}

fn tab(label: &'static str, view: View) -> Tab {
    Tab { label, view }
}

pub fn lit(s: &str) -> String {
    s.replace('\'', "''")
}

pub fn quote(db: Db, ident: &str) -> String {
    match db {
        Db::Mysql | Db::Clickhouse => format!("`{}`", ident.replace('`', "``")),
        Db::Mssql => format!("[{}]", ident.replace(']', "]]")),
        _ => format!("\"{}\"", ident.replace('"', "\"\"")),
    }
}

/// Session / lock monitor queries, tried in order (later ones are fallbacks for servers without
/// the blocking views, e.g. MariaDB without the sys schema). Rows with a non-empty BLOCKED_BY
/// sort first. None = no sessions to show (file databases).
pub fn sessions_sql(db: Db) -> Option<Vec<&'static str>> {
    Some(match db {
        Db::Oracle => vec![
            "SELECT s.sid, s.serial# AS serial, s.username, s.status, s.osuser, s.machine, s.program, s.blocking_session AS blocked_by, s.event, s.seconds_in_wait AS wait_s, s.sql_id, s.logon_time FROM v$session s WHERE s.type = 'USER' ORDER BY s.blocking_session NULLS LAST, s.sid",
        ],
        Db::Postgres => vec![
            "SELECT pid, usename AS username, datname AS database, state, application_name, client_addr::text AS client, NULLIF(array_to_string(pg_blocking_pids(pid), ','), '') AS blocked_by, wait_event_type || ':' || wait_event AS wait, (now() - query_start)::text AS running_for, left(query, 300) AS query FROM pg_stat_activity WHERE backend_type = 'client backend' ORDER BY cardinality(pg_blocking_pids(pid)) DESC, pid",
            "SELECT pid, usename AS username, datname AS database, state, application_name, left(query, 300) AS query FROM pg_stat_activity ORDER BY pid",
        ],
        Db::Mysql => vec![
            "SELECT p.id, p.user, p.host, p.db, p.command, p.time, p.state, w.blocking_pid AS blocked_by, LEFT(p.info, 300) AS query FROM information_schema.processlist p LEFT JOIN sys.innodb_lock_waits w ON w.waiting_pid = p.id ORDER BY w.blocking_pid IS NULL, p.time DESC",
            "SELECT id, user, host, db, command, time, state, LEFT(info, 300) AS query FROM information_schema.processlist ORDER BY time DESC",
        ],
        Db::Mssql => vec![
            "SELECT s.session_id, s.login_name, s.host_name, s.program_name, DB_NAME(s.database_id) AS db, s.status, NULLIF(r.blocking_session_id, 0) AS blocked_by, r.wait_type, r.total_elapsed_time / 1000 AS elapsed_s, LEFT(t.text, 300) AS query FROM sys.dm_exec_sessions s LEFT JOIN sys.dm_exec_requests r ON r.session_id = s.session_id OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) t WHERE s.is_user_process = 1 ORDER BY CASE WHEN r.blocking_session_id > 0 THEN 0 ELSE 1 END, s.session_id",
        ],
        Db::Clickhouse => vec!["SELECT query_id, user, elapsed, read_rows, formatReadableSize(memory_usage) AS memory, left(query, 300) AS query FROM system.processes ORDER BY elapsed DESC"],
        Db::Sqlite | Db::Duckdb => return None,
    })
}

/// Top SQL sort orders: (key, label). Engines without a metric fall back to total time.
pub const TOP_ORDERS: &[(&str, &str)] = &[
    ("elapsed", "Elapsed time (total)"),
    ("avg", "Elapsed time per execution"),
    ("cpu", "CPU time"),
    ("gets", "Logical reads / buffer gets"),
    ("disk", "Disk reads"),
    ("execs", "Executions"),
];

/// Marker kept out of the Top SQL list (the monitor's own statements).
const TOP_TAG: &str = "rsqldev-topsql";

/// Top SQL monitor: the heaviest statements in the server's statement cache, ordered by `order`
/// (a [`TOP_ORDERS`] key), optionally only those whose text contains `filter`. Tried in order
/// (later ones for older versions). Column 0 = statement id; Oracle column 1 = child number.
/// Oracle reads V$SQL (no Diagnostics Pack needed, unlike AWR / ASH). None = no statement
/// statistics (file databases).
pub fn top_sql(db: Db, order: &str, filter: &str) -> Option<Vec<String>> {
    let mut f = lit(&filter.trim().to_uppercase());
    if matches!(db, Db::Mysql | Db::Clickhouse) {
        f = f.replace('\\', "\\\\");
    }
    let like = |col: &str| if f.is_empty() { String::new() } else { format!(" AND UPPER({col}) LIKE '%{f}%'") };
    let pick = |m: &[(&str, &str)]| m.iter().find(|(k, _)| *k == order).or(m.first()).map_or("1", |x| x.1).to_string();
    Some(match db {
        Db::Oracle => {
            let by = pick(&[("elapsed", "elapsed_time"), ("avg", "elapsed_time / NULLIF(executions, 0)"), ("cpu", "cpu_time"), ("gets", "buffer_gets"), ("disk", "disk_reads"), ("execs", "executions")]);
            vec![format!(
                "SELECT /* {TOP_TAG} */ * FROM (SELECT sql_id, child_number AS child, parsing_schema_name AS schema, executions AS execs, ROUND(elapsed_time / 1e6, 3) AS elapsed_s, ROUND(elapsed_time / NULLIF(executions, 0) / 1e3, 2) AS avg_ms, ROUND(cpu_time / 1e6, 3) AS cpu_s, ROUND(user_io_wait_time / 1e6, 3) AS io_wait_s, buffer_gets, disk_reads, rows_processed AS rows_out, optimizer_cost AS cost, plan_hash_value AS plan_hash, module, TO_CHAR(last_active_time, 'YYYY-MM-DD HH24:MI:SS') AS last_active, sql_text FROM v$sql WHERE executions > 0 AND parsing_schema_name <> 'SYS' AND sql_text NOT LIKE '%{TOP_TAG}%'{} ORDER BY {by} DESC NULLS LAST) WHERE ROWNUM <= 100",
                like("sql_text")
            )]
        }
        Db::Postgres => {
            // PostgreSQL 13 renamed total_time / mean_time.
            let q = |total: &str, mean: &str| {
                let by = pick(&[("elapsed", total), ("avg", mean), ("cpu", total), ("gets", "shared_blks_hit + shared_blks_read"), ("disk", "shared_blks_read"), ("execs", "calls")]);
                format!(
                    "SELECT /* {TOP_TAG} */ s.queryid::text AS query_id, r.rolname AS username, s.calls AS execs, round(({total} / 1000)::numeric, 3) AS elapsed_s, round(({mean})::numeric, 2) AS avg_ms, s.rows AS rows_out, s.shared_blks_hit + s.shared_blks_read AS buffer_gets, s.shared_blks_read AS disk_reads, s.temp_blks_written AS temp_blocks, s.query AS sql_text FROM pg_stat_statements s LEFT JOIN pg_roles r ON r.oid = s.userid WHERE s.query NOT LIKE '%{TOP_TAG}%'{} ORDER BY {by} DESC NULLS LAST LIMIT 100",
                    like("s.query")
                )
            };
            vec![q("s.total_exec_time", "s.mean_exec_time"), q("s.total_time", "s.mean_time")]
        }
        Db::Mysql => {
            // SUM_CPU_TIME arrived in MySQL 8.0.28; timers are picoseconds.
            let q = |cpu: bool| {
                let by = pick(&[("elapsed", "SUM_TIMER_WAIT"), ("avg", "AVG_TIMER_WAIT"), ("cpu", if cpu { "SUM_CPU_TIME" } else { "SUM_TIMER_WAIT" }), ("gets", "SUM_ROWS_EXAMINED"), ("disk", "SUM_ROWS_EXAMINED"), ("execs", "COUNT_STAR")]);
                format!(
                    "SELECT /* {TOP_TAG} */ DIGEST AS digest, SCHEMA_NAME AS db, COUNT_STAR AS execs, ROUND(SUM_TIMER_WAIT / 1e12, 3) AS elapsed_s, ROUND(AVG_TIMER_WAIT / 1e9, 2) AS avg_ms, {} SUM_ROWS_EXAMINED AS rows_examined, SUM_ROWS_SENT AS rows_out, SUM_NO_INDEX_USED AS no_index, LAST_SEEN AS last_active, DIGEST_TEXT AS sql_text FROM performance_schema.events_statements_summary_by_digest WHERE DIGEST_TEXT IS NOT NULL AND DIGEST_TEXT NOT LIKE '%{TOP_TAG}%'{} ORDER BY {by} DESC LIMIT 100",
                    if cpu { "ROUND(SUM_CPU_TIME / 1e12, 3) AS cpu_s," } else { "" },
                    like("DIGEST_TEXT")
                )
            };
            vec![q(true), q(false)]
        }
        Db::Mssql => {
            let by = pick(&[("elapsed", "qs.total_elapsed_time"), ("avg", "qs.total_elapsed_time / qs.execution_count"), ("cpu", "qs.total_worker_time"), ("gets", "qs.total_logical_reads"), ("disk", "qs.total_physical_reads"), ("execs", "qs.execution_count")]);
            let text = "SUBSTRING(t.text, qs.statement_start_offset / 2 + 1, (CASE qs.statement_end_offset WHEN -1 THEN DATALENGTH(t.text) ELSE qs.statement_end_offset END - qs.statement_start_offset) / 2 + 1)";
            vec![format!(
                "SELECT /* {TOP_TAG} */ TOP 100 CONVERT(varchar(64), qs.query_hash, 1) AS query_hash, DB_NAME(t.dbid) AS db, qs.execution_count AS execs, CAST(qs.total_elapsed_time / 1e6 AS decimal(18, 3)) AS elapsed_s, CAST(qs.total_elapsed_time / qs.execution_count / 1e3 AS decimal(18, 2)) AS avg_ms, CAST(qs.total_worker_time / 1e6 AS decimal(18, 3)) AS cpu_s, qs.total_logical_reads AS buffer_gets, qs.total_physical_reads AS disk_reads, qs.total_rows AS rows_out, qs.last_execution_time AS last_active, {text} AS sql_text FROM sys.dm_exec_query_stats qs CROSS APPLY sys.dm_exec_sql_text(qs.sql_handle) t WHERE t.text NOT LIKE '%{TOP_TAG}%'{} ORDER BY {by} DESC",
                like("t.text")
            )]
        }
        Db::Clickhouse => {
            let q = |cpu: bool| {
                let cpu_col = if cpu { "round(sum(ProfileEvents['OSCPUVirtualTimeMicroseconds']) / 1e6, 3) AS cpu_s, " } else { "" };
                let by = pick(&[("elapsed", "elapsed_s"), ("avg", "avg_ms"), ("cpu", if cpu { "cpu_s" } else { "elapsed_s" }), ("gets", "rows_read"), ("disk", "bytes_read"), ("execs", "execs")]);
                format!(
                    "SELECT /* {TOP_TAG} */ toString(normalized_query_hash) AS query_hash, any(user) AS username, count() AS execs, round(sum(query_duration_ms) / 1000, 3) AS elapsed_s, round(avg(query_duration_ms), 2) AS avg_ms, {cpu_col}sum(read_rows) AS rows_read, formatReadableSize(sum(read_bytes)) AS read_size, sum(read_bytes) AS bytes_read, sum(result_rows) AS rows_out, max(event_time) AS last_active, any(query) AS sql_text FROM system.query_log WHERE type = 'QueryFinish' AND event_time > now() - INTERVAL 1 DAY AND query NOT LIKE '%{TOP_TAG}%'{} GROUP BY normalized_query_hash ORDER BY {by} DESC LIMIT 100",
                    like("query")
                )
            };
            vec![q(true), q(false)]
        }
        Db::Sqlite | Db::Duckdb => return None,
    })
}

/// Oracle: full text and plan of a cursor from [`top_sql`] (id = SQL_ID, child number).
/// Ids must look like a SQL_ID / number, so a grid value cannot inject SQL.
pub fn top_sql_text(sql_id: &str) -> Option<String> {
    let id = sql_id.trim();
    (id.len() <= 13 && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| format!("SELECT sql_fulltext FROM v$sql WHERE sql_id = '{id}' AND ROWNUM = 1"))
}

pub fn top_sql_plan(sql_id: &str, child: &str) -> Option<String> {
    let id = sql_id.trim();
    let child = child.trim().parse::<u32>().ok()?;
    top_sql_text(id)?;
    Some(format!("SELECT plan_table_output FROM TABLE(DBMS_XPLAN.DISPLAY_CURSOR('{id}', {child}, 'TYPICAL +COST +PEEKED_BINDS'))"))
}

/// Statement that ends the session in `row` (as returned by [`sessions_sql`]). Ids must be
/// plain numbers (ClickHouse: a UUID-like query id), so a grid value cannot inject SQL.
pub fn kill_sql(db: Db, row: &[Option<String>]) -> Option<String> {
    let num = |i: usize| row.get(i)?.as_deref()?.trim().parse::<u64>().ok();
    Some(match db {
        Db::Oracle => format!("ALTER SYSTEM KILL SESSION '{},{}' IMMEDIATE", num(0)?, num(1)?),
        Db::Postgres => format!("SELECT pg_terminate_backend({})", num(0)?),
        Db::Mysql | Db::Mssql => format!("KILL {}", num(0)?),
        // ClickHouse strings take backslash escapes: accept only id characters, no escaping games.
        Db::Clickhouse => {
            let id = row.first()?.as_deref()?.trim();
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return None;
            }
            format!("KILL QUERY WHERE query_id = '{id}'")
        }
        Db::Sqlite | Db::Duckdb => return None,
    })
}

/// `SELECT COUNT(*)` over a query, for Count Rows without fetching. None when the statement
/// cannot be wrapped (not a plain query; SQL Server forbids a CTE inside a derived table).
pub fn count_sql(db: Db, sql: &str) -> Option<String> {
    let kw = crate::first_keyword(sql);
    let body = sql.trim().trim_end_matches(';').trim_end();
    match kw.as_str() {
        "WITH" if db == Db::Mssql => None,
        "SELECT" | "WITH" | "VALUES" => Some(format!("SELECT COUNT(*) FROM ({body}) rsq_count")),
        _ => None,
    }
}

/// Bind placeholder number `n` (1-based) for [`crate::Backend::execute_binds`].
pub fn placeholder(db: Db, n: usize) -> String {
    match db {
        Db::Oracle => format!(":{n}"),
        Db::Postgres => format!("${n}"),
        Db::Mssql => format!("@P{n}"),
        _ => "?".into(),
    }
}

pub fn qualified(db: Db, owner: &str, name: &str) -> String {
    if owner.is_empty() { quote(db, name) } else { format!("{}.{}", quote(db, owner), quote(db, name)) }
}

pub fn fill(db: Db, tpl: &str, owner: &str, name: &str) -> String {
    let q = qualified(db, owner, name);
    // MySQL / ClickHouse strings also take backslash escapes: a name ending in `\` would
    // otherwise escape the closing quote.
    let l = |s: &str| if matches!(db, Db::Mysql | Db::Clickhouse) { lit(&s.replace('\\', "\\\\")) } else { lit(s) };
    let subs = [("{lq}", l(&q)), ("{q}", q.clone()), ("{o}", l(owner)), ("{n}", l(name))];
    // One pass over the template: placeholder-like text inside a substituted name is never
    // scanned again (chained replace() would turn a name's "{q}" into an unescaped name).
    let mut out = String::with_capacity(tpl.len() + 64);
    let mut rest = tpl;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        match subs.iter().find(|(k, _)| rest.starts_with(k)) {
            Some((k, v)) => {
                out.push_str(v);
                rest = &rest[k.len()..];
            }
            None => {
                out.push('{');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Folders under a connection / schema node, in SQL Developer order.
pub fn folders(db: Db) -> &'static [(&'static str, &'static str)] {
    match db {
        Db::Oracle => &[
            ("Tables", TABLE),
            ("Views", VIEW),
            ("Indexes", INDEX),
            ("Packages", PACKAGE),
            ("Procedures", PROCEDURE),
            ("Functions", FUNCTION),
            ("Triggers", TRIGGER),
            ("Types", TYPE),
            ("Sequences", SEQUENCE),
            ("Materialized Views", MVIEW),
            ("Synonyms", SYNONYM),
            ("Public Synonyms", PUBLIC_SYNONYM),
            ("Database Links", DBLINK),
            ("Directories", DIRECTORY),
            ("Scheduler Jobs", JOB),
            ("Recycle Bin", RECYCLE),
        ],
        Db::Postgres => &[
            ("Tables", TABLE),
            ("Views", VIEW),
            ("Materialized Views", MVIEW),
            ("Indexes", INDEX),
            ("Functions", FUNCTION),
            ("Procedures", PROCEDURE),
            ("Triggers", TRIGGER),
            ("Sequences", SEQUENCE),
        ],
        // MySQL / SQL Server index names are unique per table: nodes are `table.index`.
        Db::Mysql => &[("Tables", TABLE), ("Views", VIEW), ("Indexes", INDEX), ("Procedures", PROCEDURE), ("Functions", FUNCTION), ("Triggers", TRIGGER)],
        Db::Mssql => &[
            ("Tables", TABLE),
            ("Views", VIEW),
            ("Indexes", INDEX),
            ("Procedures", PROCEDURE),
            ("Functions", FUNCTION),
            ("Triggers", TRIGGER),
            ("Synonyms", SYNONYM),
            ("Sequences", SEQUENCE),
        ],
        Db::Sqlite => &[("Tables", TABLE), ("Views", VIEW), ("Indexes", INDEX), ("Triggers", TRIGGER)],
        Db::Duckdb => &[("Tables", TABLE), ("Views", VIEW), ("Indexes", INDEX), ("Sequences", SEQUENCE)],
        Db::Clickhouse => &[("Tables", TABLE), ("Views", VIEW)],
    }
}

/// Label of the folder listing other schemas.
pub fn schemas_label(db: Db) -> &'static str {
    match db {
        Db::Oracle => "Other Users",
        Db::Mysql | Db::Clickhouse => "Databases",
        _ => "Schemas",
    }
}

pub fn schemas_sql(db: Db) -> &'static str {
    match db {
        Db::Oracle => "SELECT username FROM all_users ORDER BY username",
        Db::Postgres => "SELECT nspname FROM pg_namespace WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' ORDER BY 1",
        Db::Mysql => "SELECT schema_name FROM information_schema.schemata ORDER BY 1",
        Db::Mssql => "SELECT name FROM sys.schemas ORDER BY name",
        Db::Sqlite => "SELECT name FROM pragma_database_list ORDER BY seq",
        Db::Duckdb => "SELECT DISTINCT schema_name FROM duckdb_schemas() WHERE database_name = current_database() AND schema_name NOT IN ('information_schema', 'pg_catalog') ORDER BY 1",
        Db::Clickhouse => "SELECT name FROM system.databases ORDER BY name",
    }
}

pub fn current_schema_sql(db: Db) -> &'static str {
    match db {
        Db::Oracle => "SELECT SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') FROM dual",
        Db::Postgres | Db::Duckdb => "SELECT current_schema()",
        Db::Mysql => "SELECT DATABASE()",
        Db::Mssql => "SELECT SCHEMA_NAME()",
        Db::Sqlite => "SELECT 'main'",
        Db::Clickhouse => "SELECT currentDatabase()",
    }
}

/// Objects of one kind in a schema. Column 0 = name, optional column 1 = status.
pub fn list_sql(db: Db, kind: &str, owner: &str) -> Option<String> {
    let pg_rel = |k: &str| format!("SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = '{{o}}' AND c.relkind IN ({k}) ORDER BY 1");
    let pg_proc = |k: &str| format!("SELECT quote_ident(p.proname) || '(' || pg_get_function_identity_arguments(p.oid) || ')' FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = '{{o}}' AND p.prokind = '{k}' ORDER BY 1");
    let ms = |t: &str| format!("SELECT name FROM sys.objects WHERE schema_id = SCHEMA_ID('{{o}}') AND type IN ({t}) ORDER BY name");
    let tpl = match (db, kind) {
        (Db::Oracle, DBLINK) => "SELECT db_link FROM all_db_links WHERE owner = '{o}' ORDER BY 1".into(),
        (Db::Oracle, DIRECTORY) => "SELECT directory_name FROM all_directories ORDER BY 1".into(),
        // Second column INVALID paints the node red: broken / failed jobs.
        (Db::Oracle, JOB) => "SELECT job_name, CASE WHEN state IN ('BROKEN', 'FAILED') THEN 'INVALID' END FROM all_scheduler_jobs WHERE owner = '{o}' ORDER BY 1".into(),
        (Db::Oracle, PUBLIC_SYNONYM) => "SELECT synonym_name FROM all_synonyms WHERE owner = 'PUBLIC' ORDER BY 1".into(),
        // The session user's bin (third column = the label: original name and type).
        (Db::Oracle, RECYCLE) => "SELECT object_name, NULL, original_name || ' (' || type || ', ' || droptime || ')' FROM user_recyclebin WHERE can_undrop = 'YES' ORDER BY droptime DESC".into(),
        (Db::Oracle, TABLE) => "SELECT table_name, status FROM all_tables WHERE owner = '{o}' AND nested = 'NO' AND (iot_type IS NULL OR iot_type = 'IOT') ORDER BY 1".into(),
        (Db::Oracle, k) => format!("SELECT object_name, status FROM all_objects WHERE owner = '{{o}}' AND object_type = '{k}' ORDER BY 1"),
        (Db::Postgres, TABLE) => pg_rel("'r', 'p', 'f'"),
        (Db::Postgres, VIEW) => pg_rel("'v'"),
        (Db::Postgres, MVIEW) => pg_rel("'m'"),
        (Db::Postgres, INDEX) => pg_rel("'i', 'I'"),
        (Db::Postgres, SEQUENCE) => pg_rel("'S'"),
        (Db::Postgres, FUNCTION) => pg_proc("f"),
        (Db::Postgres, PROCEDURE) => pg_proc("p"),
        (Db::Postgres, TRIGGER) => "SELECT DISTINCT t.tgname FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = '{o}' AND NOT t.tgisinternal ORDER BY 1".into(),
        (Db::Mysql, TABLE) => "SELECT table_name FROM information_schema.tables WHERE table_schema = '{o}' AND table_type = 'BASE TABLE' ORDER BY 1".into(),
        (Db::Mysql, VIEW) => "SELECT table_name FROM information_schema.views WHERE table_schema = '{o}' ORDER BY 1".into(),
        (Db::Mysql, k @ (PROCEDURE | FUNCTION)) => format!("SELECT routine_name FROM information_schema.routines WHERE routine_schema = '{{o}}' AND routine_type = '{k}' ORDER BY 1"),
        (Db::Mysql, TRIGGER) => "SELECT trigger_name FROM information_schema.triggers WHERE trigger_schema = '{o}' ORDER BY 1".into(),
        (Db::Mysql, INDEX) => "SELECT DISTINCT CONCAT(table_name, '.', index_name) FROM information_schema.statistics WHERE table_schema = '{o}' ORDER BY 1".into(),
        (Db::Mssql, INDEX) => "SELECT t.name + '.' + i.name FROM sys.indexes i JOIN sys.tables t ON t.object_id = i.object_id WHERE t.schema_id = SCHEMA_ID('{o}') AND i.name IS NOT NULL ORDER BY 1".into(),
        (Db::Mssql, TABLE) => ms("'U'"),
        (Db::Mssql, VIEW) => ms("'V'"),
        (Db::Mssql, PROCEDURE) => ms("'P'"),
        (Db::Mssql, FUNCTION) => ms("'FN', 'IF', 'TF'"),
        (Db::Mssql, TRIGGER) => ms("'TR'"),
        (Db::Mssql, SYNONYM) => ms("'SN'"),
        (Db::Mssql, SEQUENCE) => ms("'SO'"),
        (Db::Sqlite, k) => format!(
            "SELECT name FROM {q}.sqlite_master WHERE type = '{t}' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY 1",
            q = quote(db, owner),
            t = k.to_lowercase()
        ),
        (Db::Duckdb, TABLE) => "SELECT table_name FROM duckdb_tables() WHERE schema_name = '{o}' AND database_name = current_database() ORDER BY 1".into(),
        (Db::Duckdb, VIEW) => "SELECT view_name FROM duckdb_views() WHERE schema_name = '{o}' AND NOT internal ORDER BY 1".into(),
        (Db::Duckdb, INDEX) => "SELECT index_name FROM duckdb_indexes() WHERE schema_name = '{o}' ORDER BY 1".into(),
        (Db::Duckdb, SEQUENCE) => "SELECT sequence_name FROM duckdb_sequences() WHERE schema_name = '{o}' ORDER BY 1".into(),
        (Db::Clickhouse, TABLE) => "SELECT name FROM system.tables WHERE database = '{o}' AND engine NOT LIKE '%View' ORDER BY name".into(),
        (Db::Clickhouse, VIEW) => "SELECT name FROM system.tables WHERE database = '{o}' AND engine LIKE '%View' ORDER BY name".into(),
        _ => return None,
    };
    Some(fill(db, &tpl, owner, ""))
}

/// Find DB Object: objects in every schema whose name contains `pattern` (case-insensitive).
/// Columns: owner, name, kind (one of the folder kinds). At most 200 rows.
pub fn find_sql(db: Db, pattern: &str) -> String {
    let mut p = lit(&pattern.trim().to_uppercase().replace('*', "%"));
    // MySQL / ClickHouse strings take backslash escapes: a typed backslash must stay literal.
    if matches!(db, Db::Mysql | Db::Clickhouse) {
        p = p.replace('\\', "\\\\");
    }
    match db {
        Db::Oracle => format!(
            "SELECT * FROM (SELECT owner, object_name, object_type FROM all_objects WHERE UPPER(object_name) LIKE '%{p}%' AND object_type IN ('TABLE', 'VIEW', 'MATERIALIZED VIEW', 'INDEX', 'PACKAGE', 'PROCEDURE', 'FUNCTION', 'TRIGGER', 'TYPE', 'SEQUENCE', 'SYNONYM') ORDER BY CASE WHEN owner = SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA') THEN 0 ELSE 1 END, object_name, owner) WHERE ROWNUM <= 200"
        ),
        Db::Postgres => format!(
            "SELECT n.nspname, c.relname, CASE c.relkind WHEN 'v' THEN 'VIEW' WHEN 'm' THEN 'MATERIALIZED VIEW' WHEN 'i' THEN 'INDEX' WHEN 'I' THEN 'INDEX' WHEN 'S' THEN 'SEQUENCE' ELSE 'TABLE' END FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE UPPER(c.relname) LIKE '%{p}%' AND c.relkind IN ('r', 'p', 'f', 'v', 'm', 'i', 'I', 'S') AND n.nspname NOT LIKE 'pg\\_%' AND n.nspname <> 'information_schema' \
             UNION ALL SELECT n.nspname, quote_ident(p.proname) || '(' || pg_get_function_identity_arguments(p.oid) || ')', CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE UPPER(p.proname) LIKE '%{p}%' AND p.prokind IN ('f', 'p') AND n.nspname NOT LIKE 'pg\\_%' AND n.nspname <> 'information_schema' ORDER BY 2, 1 LIMIT 200"
        ),
        Db::Mysql => format!(
            "SELECT table_schema, table_name, CASE table_type WHEN 'VIEW' THEN 'VIEW' ELSE 'TABLE' END FROM information_schema.tables WHERE UPPER(table_name) LIKE '%{p}%' \
             UNION ALL SELECT routine_schema, routine_name, routine_type FROM information_schema.routines WHERE UPPER(routine_name) LIKE '%{p}%' ORDER BY 2, 1 LIMIT 200"
        ),
        Db::Mssql => format!(
            "SELECT TOP 200 SCHEMA_NAME(schema_id), name, CASE WHEN type = 'U' THEN 'TABLE' WHEN type = 'V' THEN 'VIEW' WHEN type = 'P' THEN 'PROCEDURE' WHEN type IN ('FN', 'IF', 'TF') THEN 'FUNCTION' WHEN type = 'TR' THEN 'TRIGGER' WHEN type = 'SN' THEN 'SYNONYM' ELSE 'SEQUENCE' END FROM sys.objects WHERE UPPER(name) LIKE '%{p}%' AND type IN ('U', 'V', 'P', 'FN', 'IF', 'TF', 'TR', 'SN', 'SO') ORDER BY name"
        ),
        Db::Sqlite => format!("SELECT 'main', name, UPPER(type) FROM sqlite_master WHERE UPPER(name) LIKE '%{p}%' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name LIMIT 200"),
        Db::Duckdb => format!(
            "SELECT schema_name, table_name, 'TABLE' FROM duckdb_tables() WHERE UPPER(table_name) LIKE '%{p}%' UNION ALL SELECT schema_name, view_name, 'VIEW' FROM duckdb_views() WHERE NOT internal AND UPPER(view_name) LIKE '%{p}%' ORDER BY 2 LIMIT 200"
        ),
        Db::Clickhouse => format!("SELECT database, name, if(engine LIKE '%View', 'VIEW', 'TABLE') FROM system.tables WHERE upper(name) LIKE '%{p}%' AND database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema') ORDER BY name LIMIT 200"),
    }
}

/// Folder kind constant for a kind name from [`find_sql`].
pub fn kind_const(kind: &str) -> Option<&'static str> {
    [TABLE, VIEW, MVIEW, INDEX, PACKAGE, PROCEDURE, FUNCTION, TRIGGER, TYPE, SEQUENCE, SYNONYM, DBLINK].into_iter().find(|k| k.eq_ignore_ascii_case(kind.trim()))
}

// ---------- Oracle ----------
const ORA_COLUMNS: &str = "SELECT c.column_name, c.data_type || CASE WHEN c.data_type IN ('VARCHAR2', 'NVARCHAR2', 'CHAR', 'NCHAR') THEN '(' || c.char_length || CASE c.char_used WHEN 'C' THEN ' CHAR' ELSE ' BYTE' END || ')' WHEN c.data_type = 'RAW' THEN '(' || c.data_length || ')' WHEN c.data_type = 'NUMBER' AND c.data_precision IS NOT NULL THEN '(' || c.data_precision || ',' || c.data_scale || ')' END AS data_type, CASE c.nullable WHEN 'Y' THEN 'Yes' ELSE 'No' END AS nullable, c.data_default, c.column_id, CASE WHEN EXISTS (SELECT 1 FROM all_constraints k JOIN all_cons_columns kc ON kc.owner = k.owner AND kc.constraint_name = k.constraint_name WHERE k.owner = c.owner AND k.table_name = c.table_name AND k.constraint_type = 'P' AND kc.column_name = c.column_name) THEN 'Y' END AS primary_key, cc.comments FROM all_tab_columns c LEFT JOIN all_col_comments cc ON cc.owner = c.owner AND cc.table_name = c.table_name AND cc.column_name = c.column_name WHERE c.owner = '{o}' AND c.table_name = '{n}' ORDER BY c.column_id";
const ORA_DEPENDS: &str = "SELECT owner, name, type, 'Referenced by' AS relation FROM all_dependencies WHERE referenced_owner = '{o}' AND referenced_name = '{n}' UNION ALL SELECT referenced_owner, referenced_name, referenced_type, 'References' FROM all_dependencies WHERE owner = '{o}' AND name = '{n}' ORDER BY 4, 1, 2";
const ORA_GRANTS: &str = "SELECT privilege, grantee, grantable, grantor, hierarchy FROM all_tab_privs WHERE table_schema = '{o}' AND table_name = '{n}' ORDER BY grantee, privilege";
const ORA_ERRORS: &str = "SELECT type, line, position, text, attribute FROM all_errors WHERE owner = '{o}' AND name = '{n}' ORDER BY type, sequence";
const ORA_OBJECT: &str = "SELECT * FROM all_objects WHERE owner = '{o}' AND object_name = '{n}'";

fn ora_ddl(kind: &str) -> View {
    View::Ddl(format!("SELECT DBMS_METADATA.GET_DDL('{}', '{{n}}', '{{o}}') FROM dual", kind.replace(' ', "_").replace("DATABASE_LINK", "DB_LINK")), 0)
}

fn oracle_tabs(kind: &str) -> Vec<Tab> {
    let g = |s: &str| View::Grid(s.into());
    let d = |s: &str| View::Details(s.into());
    match kind {
        TABLE => vec![
            tab("Columns", g(ORA_COLUMNS)),
            tab("Data", View::Data),
            tab("Constraints", g("SELECT c.constraint_name, c.constraint_type, (SELECT LISTAGG(cc.column_name, ', ') WITHIN GROUP (ORDER BY cc.position) FROM all_cons_columns cc WHERE cc.owner = c.owner AND cc.constraint_name = c.constraint_name) AS columns, c.search_condition_vc AS search_condition, c.r_owner, c.r_constraint_name, c.delete_rule, c.status, c.deferrable, c.validated, c.generated, c.last_change FROM all_constraints c WHERE c.owner = '{o}' AND c.table_name = '{n}' ORDER BY c.constraint_type, c.constraint_name")),
            tab("Grants", g(ORA_GRANTS)),
            tab("Statistics", g("SELECT column_name, num_distinct, num_nulls, density, num_buckets, histogram, sample_size, last_analyzed FROM all_tab_col_statistics WHERE owner = '{o}' AND table_name = '{n}' ORDER BY column_name")),
            tab("Triggers", g("SELECT trigger_name, trigger_type, triggering_event, status FROM all_triggers WHERE table_owner = '{o}' AND table_name = '{n}' ORDER BY 1")),
            tab("Dependencies", g(ORA_DEPENDS)),
            tab("Details", d("SELECT o.created, o.last_ddl_time, t.* FROM all_tables t JOIN all_objects o ON o.owner = t.owner AND o.object_name = t.table_name AND o.object_type = 'TABLE' WHERE t.owner = '{o}' AND t.table_name = '{n}'")),
            tab("Indexes", g("SELECT i.index_name, i.uniqueness, i.status, i.index_type, (SELECT LISTAGG(ic.column_name, ', ') WITHIN GROUP (ORDER BY ic.column_position) FROM all_ind_columns ic WHERE ic.index_owner = i.owner AND ic.index_name = i.index_name) AS columns, i.tablespace_name FROM all_indexes i WHERE i.table_owner = '{o}' AND i.table_name = '{n}' ORDER BY 1")),
            tab("SQL", ora_ddl(TABLE)),
        ],
        VIEW | MVIEW => vec![
            tab("Columns", g(ORA_COLUMNS)),
            tab("Data", View::Data),
            tab("Grants", g(ORA_GRANTS)),
            tab("Dependencies", g(ORA_DEPENDS)),
            tab("Details", d(if kind == VIEW { "SELECT * FROM all_views WHERE owner = '{o}' AND view_name = '{n}'" } else { "SELECT * FROM all_mviews WHERE owner = '{o}' AND mview_name = '{n}'" })),
            tab("Errors", g(ORA_ERRORS)),
            tab("SQL", ora_ddl(kind)),
        ],
        INDEX => vec![
            tab("Details", d("SELECT * FROM all_indexes WHERE owner = '{o}' AND index_name = '{n}'")),
            tab("Columns", g("SELECT column_name, column_position, descend FROM all_ind_columns WHERE index_owner = '{o}' AND index_name = '{n}' ORDER BY column_position")),
            tab("SQL", ora_ddl(INDEX)),
        ],
        PACKAGE | PROCEDURE | FUNCTION | TRIGGER | TYPE => vec![
            tab("Code", View::Code(format!("SELECT text FROM all_source WHERE owner = '{{o}}' AND name = '{{n}}' AND type IN ('{kind}', '{kind} BODY') ORDER BY type, line"))),
            tab("Grants", g(ORA_GRANTS)),
            tab("Dependencies", g(ORA_DEPENDS)),
            tab("Errors", g(ORA_ERRORS)),
            tab("Details", d(ORA_OBJECT)),
        ],
        SEQUENCE => vec![
            tab("Details", d("SELECT * FROM all_sequences WHERE sequence_owner = '{o}' AND sequence_name = '{n}'")),
            tab("Dependencies", g(ORA_DEPENDS)),
            tab("SQL", ora_ddl(SEQUENCE)),
        ],
        SYNONYM => vec![tab("Details", d("SELECT * FROM all_synonyms WHERE owner = '{o}' AND synonym_name = '{n}'")), tab("SQL", ora_ddl(SYNONYM))],
        DBLINK => vec![tab("Details", d("SELECT * FROM all_db_links WHERE owner = '{o}' AND db_link = '{n}'")), tab("SQL", ora_ddl(DBLINK))],
        DIRECTORY => vec![tab("Details", d("SELECT * FROM all_directories WHERE directory_name = '{n}'")), tab("Grants", g("SELECT grantee, privilege, grantable FROM all_tab_privs WHERE table_name = '{n}' AND type = 'DIRECTORY' ORDER BY 1, 2"))],
        PUBLIC_SYNONYM => vec![tab("Details", d("SELECT * FROM all_synonyms WHERE owner = 'PUBLIC' AND synonym_name = '{n}'"))],
        RECYCLE => vec![tab("Details", d("SELECT * FROM user_recyclebin WHERE object_name = '{n}'"))],
        JOB => vec![
            tab("Details", d("SELECT * FROM all_scheduler_jobs WHERE owner = '{o}' AND job_name = '{n}'")),
            tab("Run Log", g("SELECT log_date, status, error#, run_duration, cpu_used, additional_info FROM all_scheduler_job_run_details WHERE owner = '{o}' AND job_name = '{n}' ORDER BY log_date DESC")),
            tab("Arguments", g("SELECT argument_position, argument_name, argument_type, value FROM all_scheduler_job_args WHERE owner = '{o}' AND job_name = '{n}' ORDER BY 1")),
        ],
        _ => vec![tab("Details", d(ORA_OBJECT))],
    }
}

// ---------- PostgreSQL ----------
const PG_COLUMNS: &str = "SELECT a.attname AS column_name, format_type(a.atttypid, a.atttypmod) AS data_type, CASE WHEN a.attnotnull THEN 'No' ELSE 'Yes' END AS nullable, pg_get_expr(d.adbin, d.adrelid) AS data_default, a.attnum AS column_id, CASE WHEN EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY (i.indkey)) THEN 'Y' END AS primary_key, col_description(a.attrelid, a.attnum) AS comments FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum WHERE a.attrelid = '{lq}'::regclass AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum";
const PG_TABLE_DDL: &str = "SELECT 'CREATE TABLE ' || '{lq}' || E' (\\n' || string_agg('    ' || quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod) || CASE WHEN a.attnotnull THEN ' NOT NULL' ELSE '' END || COALESCE(' DEFAULT ' || pg_get_expr(d.adbin, d.adrelid), ''), E',\\n' ORDER BY a.attnum) || COALESCE((SELECT E',\\n' || string_agg('    CONSTRAINT ' || quote_ident(conname) || ' ' || pg_get_constraintdef(oid), E',\\n') FROM pg_constraint WHERE conrelid = '{lq}'::regclass), '') || E'\\n);' || COALESCE((SELECT E'\\n' || string_agg(pg_get_indexdef(i.indexrelid) || ';', E'\\n') FROM pg_index i WHERE i.indrelid = '{lq}'::regclass AND NOT EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = i.indexrelid)), '') FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum WHERE a.attrelid = '{lq}'::regclass AND a.attnum > 0 AND NOT a.attisdropped";
/// Functions are listed as `name(args)`, so the name is used unquoted after the schema.
const PG_FUNC: &str = "SELECT pg_get_functiondef((quote_ident('{o}') || '.' || '{n}')::regprocedure)";

fn pg_tabs(kind: &str) -> Vec<Tab> {
    let g = |s: &str| View::Grid(s.into());
    let d = |s: &str| View::Details(s.into());
    match kind {
        TABLE => vec![
            tab("Columns", g(PG_COLUMNS)),
            tab("Data", View::Data),
            tab("Constraints", g("SELECT conname AS constraint_name, contype AS constraint_type, pg_get_constraintdef(oid) AS definition FROM pg_constraint WHERE conrelid = '{lq}'::regclass ORDER BY 2, 1")),
            tab("Grants", g("SELECT grantee, privilege_type, is_grantable, grantor FROM information_schema.role_table_grants WHERE table_schema = '{o}' AND table_name = '{n}' ORDER BY 1, 2")),
            tab("Statistics", d("SELECT * FROM pg_stat_all_tables WHERE relid = '{lq}'::regclass")),
            tab("Triggers", g("SELECT tgname AS trigger_name, pg_get_triggerdef(oid) AS definition, tgenabled AS enabled FROM pg_trigger WHERE tgrelid = '{lq}'::regclass AND NOT tgisinternal ORDER BY 1")),
            tab("Dependencies", g("SELECT DISTINCT v.relnamespace::regnamespace::text AS owner, v.relname AS name, 'VIEW' AS type FROM pg_depend d JOIN pg_rewrite r ON r.oid = d.objid JOIN pg_class v ON v.oid = r.ev_class WHERE d.refobjid = '{lq}'::regclass AND v.oid <> d.refobjid ORDER BY 1, 2")),
            tab("Details", d("SELECT c.*, pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size FROM pg_class c WHERE c.oid = '{lq}'::regclass")),
            tab("Indexes", g("SELECT indexrelid::regclass::text AS index_name, indisunique AS is_unique, indisprimary AS is_primary, pg_get_indexdef(indexrelid) AS definition FROM pg_index WHERE indrelid = '{lq}'::regclass ORDER BY 1")),
            tab("SQL", View::Ddl(PG_TABLE_DDL.into(), 0)),
        ],
        VIEW | MVIEW => vec![
            tab("Columns", g(PG_COLUMNS)),
            tab("Data", View::Data),
            tab("Details", d("SELECT * FROM pg_class WHERE oid = '{lq}'::regclass")),
            tab(
                "SQL",
                View::Ddl(
                    format!("SELECT 'CREATE {} ' || '{{lq}}' || E' AS\\n' || pg_get_viewdef('{{lq}}'::regclass, true)", if kind == VIEW { "OR REPLACE VIEW" } else { "MATERIALIZED VIEW" }),
                    0,
                ),
            ),
        ],
        INDEX => vec![
            tab("Details", d("SELECT * FROM pg_index WHERE indexrelid = '{lq}'::regclass")),
            tab("SQL", View::Ddl("SELECT pg_get_indexdef('{lq}'::regclass) || ';'".into(), 0)),
        ],
        FUNCTION | PROCEDURE => vec![
            tab("Code", View::Code(PG_FUNC.into())),
            tab("Details", d("SELECT * FROM pg_proc WHERE oid = (quote_ident('{o}') || '.' || '{n}')::regprocedure")),
        ],
        TRIGGER => vec![
            tab("Code", View::Code("SELECT pg_get_triggerdef(t.oid, true) || ';' FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = '{o}' AND t.tgname = '{n}'".into())),
        ],
        SEQUENCE => vec![
            tab("Details", d("SELECT * FROM pg_sequences WHERE schemaname = '{o}' AND sequencename = '{n}'")),
            tab("SQL", View::Ddl("SELECT format('CREATE SEQUENCE %s START %s INCREMENT %s MINVALUE %s MAXVALUE %s CACHE %s%s;', '{lq}', start_value, increment_by, min_value, max_value, cache_size, CASE WHEN cycle THEN ' CYCLE' ELSE '' END) FROM pg_sequences WHERE schemaname = '{o}' AND sequencename = '{n}'".into(), 0)),
        ],
        _ => vec![],
    }
}

// ---------- MySQL ----------
const MY_COLUMNS: &str = "SELECT column_name, column_type AS data_type, CASE is_nullable WHEN 'YES' THEN 'Yes' ELSE 'No' END AS nullable, column_default AS data_default, ordinal_position AS column_id, CASE column_key WHEN 'PRI' THEN 'Y' END AS primary_key, extra, column_comment AS comments FROM information_schema.columns WHERE table_schema = '{o}' AND table_name = '{n}' ORDER BY ordinal_position";

fn mysql_tabs(kind: &str) -> Vec<Tab> {
    let g = |s: &str| View::Grid(s.into());
    let d = |s: &str| View::Details(s.into());
    match kind {
        TABLE => vec![
            tab("Columns", g(MY_COLUMNS)),
            tab("Data", View::Data),
            tab("Constraints", g("SELECT tc.constraint_name, tc.constraint_type, GROUP_CONCAT(k.column_name ORDER BY k.ordinal_position) AS columns, MAX(k.referenced_table_name) AS referenced_table FROM information_schema.table_constraints tc LEFT JOIN information_schema.key_column_usage k ON k.constraint_schema = tc.constraint_schema AND k.table_name = tc.table_name AND k.constraint_name = tc.constraint_name WHERE tc.table_schema = '{o}' AND tc.table_name = '{n}' GROUP BY tc.constraint_name, tc.constraint_type ORDER BY 2, 1")),
            tab("Grants", g("SELECT grantee, privilege_type, is_grantable FROM information_schema.table_privileges WHERE table_schema = '{o}' AND table_name = '{n}' ORDER BY 1, 2")),
            tab("Triggers", g("SELECT trigger_name, action_timing, event_manipulation FROM information_schema.triggers WHERE event_object_schema = '{o}' AND event_object_table = '{n}' ORDER BY 1")),
            tab("Details", d("SELECT * FROM information_schema.tables WHERE table_schema = '{o}' AND table_name = '{n}'")),
            tab("Indexes", g("SELECT index_name, CASE non_unique WHEN 0 THEN 'UNIQUE' ELSE 'NONUNIQUE' END AS uniqueness, GROUP_CONCAT(column_name ORDER BY seq_in_index) AS columns, index_type FROM information_schema.statistics WHERE table_schema = '{o}' AND table_name = '{n}' GROUP BY index_name, non_unique, index_type ORDER BY 1")),
            tab("SQL", View::Ddl("SHOW CREATE TABLE {q}".into(), 1)),
        ],
        VIEW => vec![
            tab("Columns", g(MY_COLUMNS)),
            tab("Data", View::Data),
            tab("Details", d("SELECT * FROM information_schema.views WHERE table_schema = '{o}' AND table_name = '{n}'")),
            tab("SQL", View::Ddl("SHOW CREATE VIEW {q}".into(), 1)),
        ],
        PROCEDURE | FUNCTION => vec![
            tab("Code", View::Ddl(format!("SHOW CREATE {kind} {{q}}"), 2)),
            tab("Details", d("SELECT * FROM information_schema.routines WHERE routine_schema = '{o}' AND routine_name = '{n}'")),
        ],
        TRIGGER => vec![
            tab("Code", View::Ddl("SHOW CREATE TRIGGER {q}".into(), 2)),
            tab("Details", d("SELECT * FROM information_schema.triggers WHERE trigger_schema = '{o}' AND trigger_name = '{n}'")),
        ],
        INDEX => vec![tab(
            "Columns",
            g("SELECT seq_in_index, column_name, CASE non_unique WHEN 0 THEN 'UNIQUE' ELSE 'NONUNIQUE' END AS uniqueness, collation, index_type, sub_part, cardinality FROM information_schema.statistics WHERE table_schema = '{o}' AND CONCAT(table_name, '.', index_name) = '{n}' ORDER BY seq_in_index"),
        )],
        _ => vec![],
    }
}

// ---------- SQL Server ----------
const MS_TYPE: &str = "TYPE_NAME(c.user_type_id) + CASE WHEN TYPE_NAME(c.user_type_id) IN ('varchar', 'char', 'varbinary', 'binary') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')' WHEN TYPE_NAME(c.user_type_id) IN ('nvarchar', 'nchar') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')' WHEN TYPE_NAME(c.user_type_id) IN ('decimal', 'numeric') THEN '(' + CAST(c.precision AS varchar(10)) + ',' + CAST(c.scale AS varchar(10)) + ')' ELSE '' END";

fn mssql_tabs(kind: &str) -> Vec<Tab> {
    let g = |s: String| View::Grid(s);
    let d = |s: &str| View::Details(s.into());
    let cols = format!("SELECT c.name AS column_name, {MS_TYPE} AS data_type, CASE c.is_nullable WHEN 1 THEN 'Yes' ELSE 'No' END AS nullable, OBJECT_DEFINITION(c.default_object_id) AS data_default, c.column_id, CASE WHEN EXISTS (SELECT 1 FROM sys.index_columns ic JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) THEN 'Y' END AS primary_key, CAST(ep.value AS nvarchar(4000)) AS comments FROM sys.columns c LEFT JOIN sys.extended_properties ep ON ep.major_id = c.object_id AND ep.minor_id = c.column_id AND ep.name = 'MS_Description' WHERE c.object_id = OBJECT_ID('{{lq}}') ORDER BY c.column_id");
    let code = || View::Code("SELECT OBJECT_DEFINITION(OBJECT_ID('{lq}'))".into());
    match kind {
        TABLE => vec![
            tab("Columns", g(cols)),
            tab("Data", View::Data),
            tab("Constraints", g("SELECT name AS constraint_name, type_desc AS constraint_type FROM sys.objects WHERE parent_object_id = OBJECT_ID('{lq}') AND type IN ('PK', 'UQ', 'F', 'C', 'D') ORDER BY 2, 1".into())),
            tab("Triggers", g("SELECT name AS trigger_name, is_disabled FROM sys.triggers WHERE parent_id = OBJECT_ID('{lq}') ORDER BY 1".into())),
            tab("Details", d("SELECT * FROM sys.tables WHERE object_id = OBJECT_ID('{lq}')")),
            tab("Indexes", g("SELECT i.name AS index_name, i.type_desc, i.is_unique, i.is_primary_key, STRING_AGG(c.name, ', ') WITHIN GROUP (ORDER BY ic.key_ordinal) AS columns FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id WHERE i.object_id = OBJECT_ID('{lq}') GROUP BY i.name, i.type_desc, i.is_unique, i.is_primary_key ORDER BY 1".into())),
            tab("SQL", View::Ddl(format!("SELECT 'CREATE TABLE ' + '{{lq}}' + ' (' + CHAR(10) + STRING_AGG(CAST('    ' + QUOTENAME(c.name) + ' ' + {MS_TYPE} + CASE WHEN c.is_identity = 1 THEN ' IDENTITY' ELSE '' END + CASE WHEN c.is_nullable = 0 THEN ' NOT NULL' ELSE ' NULL' END AS nvarchar(max)), ',' + CHAR(10)) WITHIN GROUP (ORDER BY c.column_id) + CHAR(10) + ');' FROM sys.columns c WHERE c.object_id = OBJECT_ID('{{lq}}')"), 0)),
        ],
        VIEW => vec![
            tab("Columns", g(cols)),
            tab("Data", View::Data),
            tab("Details", d("SELECT * FROM sys.views WHERE object_id = OBJECT_ID('{lq}')")),
            tab("SQL", code()),
        ],
        PROCEDURE | FUNCTION | TRIGGER => vec![tab("Code", code()), tab("Details", d("SELECT * FROM sys.objects WHERE object_id = OBJECT_ID('{lq}')"))],
        SYNONYM => vec![tab("Details", d("SELECT * FROM sys.synonyms WHERE object_id = OBJECT_ID('{lq}')"))],
        SEQUENCE => vec![tab("Details", d("SELECT * FROM sys.sequences WHERE object_id = OBJECT_ID('{lq}')"))],
        INDEX => vec![tab(
            "Columns",
            g("SELECT ic.key_ordinal, c.name AS column_name, CASE ic.is_descending_key WHEN 1 THEN 'DESC' ELSE 'ASC' END AS sort, ic.is_included_column, i.type_desc, i.is_unique, i.is_primary_key FROM sys.indexes i JOIN sys.tables t ON t.object_id = i.object_id JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id WHERE t.schema_id = SCHEMA_ID('{o}') AND t.name + '.' + i.name = '{n}' ORDER BY ic.is_included_column, ic.key_ordinal".into()),
        )],
        _ => vec![],
    }
}

// ---------- SQLite ----------
fn sqlite_tabs(kind: &str) -> Vec<Tab> {
    let g = |s: &str| View::Grid(s.into());
    let ddl = || View::Ddl("SELECT sql FROM \"{o}\".sqlite_master WHERE name = '{n}'".into(), 0);
    match kind {
        TABLE => vec![
            tab("Columns", g("SELECT name AS column_name, type AS data_type, CASE \"notnull\" WHEN 1 THEN 'No' ELSE 'Yes' END AS nullable, dflt_value AS data_default, cid + 1 AS column_id, CASE WHEN pk > 0 THEN 'Y' END AS primary_key FROM pragma_table_info('{n}', '{o}')")),
            tab("Data", View::Data),
            tab("Constraints", g("SELECT id, seq, \"table\" AS referenced_table, \"from\" AS column_name, \"to\" AS referenced_column, on_update, on_delete FROM pragma_foreign_key_list('{n}', '{o}')")),
            tab("Triggers", g("SELECT name AS trigger_name, sql FROM \"{o}\".sqlite_master WHERE type = 'trigger' AND tbl_name = '{n}'")),
            tab("Indexes", g("SELECT name AS index_name, \"unique\", origin, partial FROM pragma_index_list('{n}', '{o}')")),
            tab("SQL", ddl()),
        ],
        VIEW => vec![
            tab("Columns", g("SELECT name AS column_name, type AS data_type FROM pragma_table_info('{n}', '{o}')")),
            tab("Data", View::Data),
            tab("SQL", ddl()),
        ],
        _ => vec![tab("SQL", ddl())],
    }
}

// ---------- DuckDB ----------
fn duckdb_tabs(kind: &str) -> Vec<Tab> {
    let g = |s: &str| View::Grid(s.into());
    let d = |s: &str| View::Details(s.into());
    let cols = "SELECT column_name, data_type, CASE WHEN is_nullable THEN 'Yes' ELSE 'No' END AS nullable, column_default AS data_default, column_index AS column_id, comment AS comments FROM duckdb_columns() WHERE schema_name = '{o}' AND table_name = '{n}' ORDER BY column_index";
    match kind {
        TABLE => vec![
            tab("Columns", g(cols)),
            tab("Data", View::Data),
            tab("Constraints", g("SELECT constraint_type, constraint_text FROM duckdb_constraints() WHERE schema_name = '{o}' AND table_name = '{n}'")),
            tab("Details", d("SELECT * FROM duckdb_tables() WHERE schema_name = '{o}' AND table_name = '{n}'")),
            tab("Indexes", g("SELECT index_name, is_unique, sql FROM duckdb_indexes() WHERE schema_name = '{o}' AND table_name = '{n}'")),
            tab("SQL", View::Ddl("SELECT sql FROM duckdb_tables() WHERE schema_name = '{o}' AND table_name = '{n}'".into(), 0)),
        ],
        VIEW => vec![
            tab("Columns", g(cols)),
            tab("Data", View::Data),
            tab("SQL", View::Ddl("SELECT sql FROM duckdb_views() WHERE schema_name = '{o}' AND view_name = '{n}'".into(), 0)),
        ],
        INDEX => vec![tab("SQL", View::Ddl("SELECT sql FROM duckdb_indexes() WHERE schema_name = '{o}' AND index_name = '{n}'".into(), 0))],
        SEQUENCE => vec![tab("Details", d("SELECT * FROM duckdb_sequences() WHERE schema_name = '{o}' AND sequence_name = '{n}'"))],
        _ => vec![],
    }
}

// ---------- ClickHouse ----------
fn clickhouse_tabs(_kind: &str) -> Vec<Tab> {
    vec![
        tab("Columns", View::Grid("SELECT name AS column_name, type AS data_type, default_kind, default_expression AS data_default, position AS column_id, if(is_in_primary_key, 'Y', '') AS primary_key, comment AS comments FROM system.columns WHERE database = '{o}' AND table = '{n}' ORDER BY position".into())),
        tab("Data", View::Data),
        tab("Details", View::Details("SELECT * FROM system.tables WHERE database = '{o}' AND name = '{n}'".into())),
        tab("SQL", View::Ddl("SHOW CREATE TABLE {q}".into(), 0)),
    ]
}

/// Object viewer tabs for an object kind, templates unfilled (use [`fill`]).
pub fn tabs(db: Db, kind: &str) -> Vec<Tab> {
    match db {
        Db::Oracle => oracle_tabs(kind),
        Db::Postgres => pg_tabs(kind),
        Db::Mysql => mysql_tabs(kind),
        Db::Mssql => mssql_tabs(kind),
        Db::Sqlite => sqlite_tabs(kind),
        Db::Duckdb => duckdb_tabs(kind),
        Db::Clickhouse => clickhouse_tabs(kind),
    }
}

/// Child nodes of an object in the tree (table/view → columns): column name + type.
pub fn children_sql(db: Db, kind: &str) -> Option<String> {
    if kind != TABLE && kind != VIEW && kind != MVIEW {
        return None;
    }
    match tabs(db, kind).into_iter().find(|t| t.label == "Columns")?.view {
        View::Grid(s) => Some(s),
        _ => None,
    }
}

/// More child nodes in the tree, after the columns: (name, detail) rows for package members,
/// routine arguments and table partitions. Templates unfilled (use [`fill`]).
pub fn extra_children_sql(db: Db, kind: &str) -> Option<&'static str> {
    Some(match (db, kind) {
        (Db::Oracle, PACKAGE) => "SELECT procedure_name, NULL FROM all_procedures WHERE owner = '{o}' AND object_name = '{n}' AND procedure_name IS NOT NULL GROUP BY procedure_name ORDER BY MIN(subprogram_id)",
        (Db::Oracle, PROCEDURE | FUNCTION) => "SELECT NVL(argument_name, 'RETURN'), LTRIM(NVL2(argument_name, in_out, '') || ' ' || data_type) FROM all_arguments WHERE owner = '{o}' AND object_name = '{n}' AND package_name IS NULL AND data_level = 0 AND (argument_name IS NOT NULL OR position = 0) ORDER BY overload, position",
        (Db::Oracle, TABLE) => "SELECT partition_name, 'PARTITION' FROM all_tab_partitions WHERE table_owner = '{o}' AND table_name = '{n}' ORDER BY partition_position",
        (Db::Postgres, TABLE) => "SELECT c.relname, 'PARTITION' FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid JOIN pg_class p ON p.oid = i.inhparent WHERE p.relkind = 'p' AND i.inhparent = '{lq}'::regclass ORDER BY 1",
        (Db::Mysql, TABLE) => "SELECT partition_name, CONCAT('PARTITION ', partition_method) FROM information_schema.partitions WHERE table_schema = '{o}' AND table_name = '{n}' AND partition_name IS NOT NULL ORDER BY partition_ordinal_position",
        (Db::Mysql, PROCEDURE | FUNCTION) => "SELECT COALESCE(parameter_name, 'RETURN'), CONCAT_WS(' ', parameter_mode, dtd_identifier) FROM information_schema.parameters WHERE specific_schema = '{o}' AND specific_name = '{n}' ORDER BY ordinal_position",
        (Db::Mssql, PROCEDURE | FUNCTION) => "SELECT CASE WHEN p.name = '' THEN 'RETURN' ELSE p.name END, CASE WHEN p.is_output = 1 AND p.parameter_id > 0 THEN 'OUT ' ELSE '' END + TYPE_NAME(p.user_type_id) FROM sys.parameters p WHERE p.object_id = OBJECT_ID('{lq}') ORDER BY p.parameter_id",
        _ => return None,
    })
}

/// Folder ▸ New …: a CREATE template for a worksheet (placeholders in lower case). `{s}` in
/// the patterns is the quoted owner.
pub fn create_template(db: Db, kind: &str, owner: &str) -> Option<String> {
    let t = match (db, kind) {
        (Db::Oracle, VIEW) => "CREATE OR REPLACE VIEW {s}.new_view AS\nSELECT *\n  FROM dual;\n",
        (Db::Oracle, MVIEW) => "CREATE MATERIALIZED VIEW {s}.new_mview\n  REFRESH COMPLETE ON DEMAND\nAS\nSELECT *\n  FROM dual;\n",
        (Db::Oracle, INDEX) => "CREATE INDEX {s}.new_index ON {s}.table_name (column_name);\n",
        (Db::Oracle, SEQUENCE) => "CREATE SEQUENCE {s}.new_sequence\n  START WITH 1\n  INCREMENT BY 1\n  NOCACHE\n  NOCYCLE;\n",
        (Db::Oracle, SYNONYM) => "CREATE OR REPLACE SYNONYM {s}.new_synonym FOR owner.object_name;\n",
        (Db::Oracle, PROCEDURE) => "CREATE OR REPLACE PROCEDURE {s}.new_procedure (\n  p_id IN NUMBER\n) AS\nBEGIN\n  NULL;\nEND;\n/\n",
        (Db::Oracle, FUNCTION) => "CREATE OR REPLACE FUNCTION {s}.new_function (\n  p_id IN NUMBER\n) RETURN NUMBER AS\nBEGIN\n  RETURN p_id;\nEND;\n/\n",
        (Db::Oracle, PACKAGE) => "CREATE OR REPLACE PACKAGE {s}.new_package AS\n  PROCEDURE run;\nEND;\n/\n\nCREATE OR REPLACE PACKAGE BODY {s}.new_package AS\n  PROCEDURE run IS\n  BEGIN\n    NULL;\n  END;\nEND;\n/\n",
        (Db::Oracle, TRIGGER) => "CREATE OR REPLACE TRIGGER {s}.new_trigger\n  BEFORE INSERT OR UPDATE ON {s}.table_name\n  FOR EACH ROW\nBEGIN\n  NULL;\nEND;\n/\n",
        (Db::Oracle, TYPE) => "CREATE OR REPLACE TYPE {s}.new_type AS OBJECT (\n  id   NUMBER,\n  name VARCHAR2(100)\n);\n/\n",
        (Db::Oracle, JOB) => "BEGIN\n  DBMS_SCHEDULER.CREATE_JOB(\n    job_name        => '{s}.NEW_JOB',\n    job_type        => 'PLSQL_BLOCK',\n    job_action      => 'BEGIN NULL; END;',\n    start_date      => SYSTIMESTAMP,\n    repeat_interval => 'FREQ=DAILY; BYHOUR=2',\n    enabled         => TRUE,\n    comments        => '');\nEND;\n/\n",
        (Db::Oracle, DBLINK) => "CREATE DATABASE LINK new_link\n  CONNECT TO remote_user IDENTIFIED BY \"password\"\n  USING 'tns_alias';\n",
        (Db::Postgres | Db::Duckdb, VIEW) => "CREATE OR REPLACE VIEW {s}.new_view AS\nSELECT 1 AS id;\n",
        (Db::Postgres, MVIEW) => "CREATE MATERIALIZED VIEW {s}.new_mview AS\nSELECT 1 AS id;\n",
        (Db::Postgres | Db::Duckdb | Db::Sqlite, INDEX) => "CREATE INDEX new_index ON {s}.table_name (column_name);\n",
        (Db::Postgres | Db::Duckdb, SEQUENCE) => "CREATE SEQUENCE {s}.new_sequence START 1 INCREMENT 1;\n",
        (Db::Postgres, FUNCTION) => "CREATE OR REPLACE FUNCTION {s}.new_function(p_id integer)\nRETURNS integer\nLANGUAGE plpgsql AS $$\nBEGIN\n  RETURN p_id;\nEND;\n$$;\n",
        (Db::Postgres, PROCEDURE) => "CREATE OR REPLACE PROCEDURE {s}.new_procedure(p_id integer)\nLANGUAGE plpgsql AS $$\nBEGIN\n  NULL;\nEND;\n$$;\n",
        (Db::Postgres, TRIGGER) => "CREATE OR REPLACE FUNCTION {s}.new_trigger_fn()\nRETURNS trigger\nLANGUAGE plpgsql AS $$\nBEGIN\n  RETURN NEW;\nEND;\n$$;\n\nCREATE TRIGGER new_trigger\n  BEFORE INSERT OR UPDATE ON {s}.table_name\n  FOR EACH ROW EXECUTE FUNCTION {s}.new_trigger_fn();\n",
        (Db::Mysql, VIEW) => "CREATE OR REPLACE VIEW {s}.new_view AS\nSELECT 1 AS id;\n",
        (Db::Mysql, INDEX) => "CREATE INDEX new_index ON {s}.table_name (column_name);\n",
        (Db::Mysql, PROCEDURE) => "DELIMITER //\nCREATE PROCEDURE {s}.new_procedure(IN p_id INT)\nBEGIN\n  SELECT p_id;\nEND //\nDELIMITER ;\n",
        (Db::Mysql, FUNCTION) => "DELIMITER //\nCREATE FUNCTION {s}.new_function(p_id INT) RETURNS INT\nDETERMINISTIC\nBEGIN\n  RETURN p_id;\nEND //\nDELIMITER ;\n",
        (Db::Mysql, TRIGGER) => "DELIMITER //\nCREATE TRIGGER {s}.new_trigger\n  BEFORE INSERT ON {s}.table_name\n  FOR EACH ROW\nBEGIN\n  SET NEW.column_name = NEW.column_name;\nEND //\nDELIMITER ;\n",
        (Db::Mssql, VIEW) => "CREATE OR ALTER VIEW {s}.new_view AS\nSELECT 1 AS id;\nGO\n",
        (Db::Mssql, INDEX) => "CREATE INDEX new_index ON {s}.table_name (column_name);\nGO\n",
        (Db::Mssql, PROCEDURE) => "CREATE OR ALTER PROCEDURE {s}.new_procedure\n  @p_id INT\nAS\nBEGIN\n  SET NOCOUNT ON;\n  SELECT @p_id;\nEND;\nGO\n",
        (Db::Mssql, FUNCTION) => "CREATE OR ALTER FUNCTION {s}.new_function (@p_id INT)\nRETURNS INT\nAS\nBEGIN\n  RETURN @p_id;\nEND;\nGO\n",
        (Db::Mssql, TRIGGER) => "CREATE OR ALTER TRIGGER {s}.new_trigger\nON {s}.table_name\nAFTER INSERT, UPDATE\nAS\nBEGIN\n  SET NOCOUNT ON;\nEND;\nGO\n",
        (Db::Mssql, SYNONYM) => "CREATE SYNONYM {s}.new_synonym FOR schema_name.object_name;\nGO\n",
        (Db::Mssql, SEQUENCE) => "CREATE SEQUENCE {s}.new_sequence START WITH 1 INCREMENT BY 1;\nGO\n",
        (Db::Sqlite, VIEW) => "CREATE VIEW {s}.new_view AS\nSELECT 1 AS id;\n",
        (Db::Sqlite, TRIGGER) => "CREATE TRIGGER {s}.new_trigger\n  AFTER INSERT ON table_name\nBEGIN\n  SELECT 1;\nEND;\n",
        (Db::Clickhouse, VIEW) => "CREATE VIEW {s}.new_view AS\nSELECT 1 AS id;\n",
        _ => return None,
    };
    Some(t.replace("{s}", &quote(db, owner)))
}

/// Sequence ▸ Edit…: current settings (Oracle / PostgreSQL), for [`alter_sequence`].
pub fn sequence_sql(db: Db, owner: &str, name: &str) -> Option<String> {
    let tpl = match db {
        Db::Oracle => "SELECT increment_by, min_value, max_value, cache_size, cycle_flag FROM all_sequences WHERE sequence_owner = '{o}' AND sequence_name = '{n}'",
        Db::Postgres => "SELECT increment_by, min_value, max_value, cache_size, CASE WHEN cycle THEN 'Y' ELSE 'N' END FROM pg_sequences WHERE schemaname = '{o}' AND sequencename = '{n}'",
        _ => return None,
    };
    Some(fill(db, tpl, owner, name))
}

/// ALTER SEQUENCE with the current settings from [`sequence_sql`] (increment, min, max,
/// cache, cycle Y/N), one clause per line to edit.
pub fn alter_sequence(db: Db, owner: &str, name: &str, row: &[Option<String>]) -> String {
    let v = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let cache = v(3).parse::<u64>().unwrap_or(0);
    let cycle = v(4) == "Y";
    let (cache, cycle) = match db {
        Db::Oracle => (if cache > 1 { format!("CACHE {cache}") } else { "NOCACHE".into() }, if cycle { "CYCLE" } else { "NOCYCLE" }),
        _ => (format!("CACHE {}", cache.max(1)), if cycle { "CYCLE" } else { "NO CYCLE" }),
    };
    format!("ALTER SEQUENCE {}\n  INCREMENT BY {}\n  MINVALUE {}\n  MAXVALUE {}\n  {cache}\n  {cycle};\n", qualified(db, owner, name), v(0), v(1), v(2))
}

/// Data tab query. `hidden` = leading row-locator column (ROWID, ctid, rowid) kept out of view
/// and used as the edit key.
pub struct DataQuery {
    pub sql: String,
    pub hidden: Option<&'static str>,
    pub editable: bool,
}

pub fn data_query(db: Db, kind: &str, owner: &str, name: &str) -> DataQuery {
    let q = qualified(db, owner, name);
    let plain = |editable| DataQuery { sql: format!("SELECT * FROM {q}"), hidden: None, editable };
    if kind != TABLE {
        return plain(false);
    }
    match db {
        Db::Oracle => DataQuery { sql: format!("SELECT ROWID, t.* FROM {q} t"), hidden: Some("ROWID"), editable: true },
        Db::Postgres => DataQuery { sql: format!("SELECT ctid, * FROM {q}"), hidden: Some("ctid"), editable: true },
        Db::Sqlite | Db::Duckdb => DataQuery { sql: format!("SELECT rowid, * FROM {q}"), hidden: Some("rowid"), editable: true },
        Db::Mysql | Db::Mssql => plain(true),
        Db::Clickhouse => plain(false),
    }
}

/// Primary-key columns (edit key; engines with a row locator fall back to it).
pub fn pk_sql(db: Db, owner: &str, name: &str) -> Option<String> {
    let tpl = match db {
        Db::Postgres => "SELECT a.attname FROM pg_index i JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey) WHERE i.indrelid = '{lq}'::regclass AND i.indisprimary ORDER BY array_position(i.indkey, a.attnum)",
        Db::Mysql => "SELECT column_name FROM information_schema.key_column_usage WHERE table_schema = '{o}' AND table_name = '{n}' AND constraint_name = 'PRIMARY' ORDER BY ordinal_position",
        Db::Mssql => "SELECT c.name FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id WHERE i.is_primary_key = 1 AND i.object_id = OBJECT_ID('{lq}') ORDER BY ic.key_ordinal",
        Db::Oracle => "SELECT cc.column_name FROM all_constraints c JOIN all_cons_columns cc ON cc.owner = c.owner AND cc.constraint_name = c.constraint_name WHERE c.owner = '{o}' AND c.table_name = '{n}' AND c.constraint_type = 'P' ORDER BY cc.position",
        Db::Sqlite => "SELECT name FROM pragma_table_info('{n}', '{o}') WHERE pk > 0 ORDER BY pk",
        Db::Duckdb => "SELECT unnest(constraint_column_names) FROM duckdb_constraints() WHERE schema_name = '{o}' AND table_name = '{n}' AND constraint_type = 'PRIMARY KEY'",
        Db::Clickhouse => return None,
    };
    Some(fill(db, tpl, owner, name))
}

/// Explain plan: run `stmts` in order and show the result of `stmts[show]`.
pub struct Explain {
    pub stmts: Vec<String>,
    pub show: usize,
}

pub fn explain(db: Db, sql: &str) -> Explain {
    let sql = sql.trim().trim_end_matches(';');
    let one = |p: &str| Explain { stmts: vec![format!("{p} {sql}")], show: 0 };
    match db {
        Db::Oracle => Explain {
            stmts: vec![
                "DELETE FROM plan_table WHERE statement_id = 'rsqldev'".into(),
                format!("EXPLAIN PLAN SET STATEMENT_ID = 'rsqldev' FOR {sql}"),
                "SELECT LPAD(' ', 2 * (LEVEL - 1)) || operation || NVL2(options, ' ' || options, '') AS operation, object_name, cardinality, cost, bytes, access_predicates, filter_predicates FROM plan_table START WITH id = 0 AND statement_id = 'rsqldev' CONNECT BY PRIOR id = parent_id AND statement_id = 'rsqldev' ORDER SIBLINGS BY id".into(),
            ],
            show: 2,
        },
        Db::Mssql => Explain { stmts: vec!["SET SHOWPLAN_TEXT ON".into(), sql.into(), "SET SHOWPLAN_TEXT OFF".into()], show: 1 },
        Db::Sqlite => one("EXPLAIN QUERY PLAN"),
        _ => one("EXPLAIN"),
    }
}

/// Autotrace: run the query and show its plan with actual rows and timings. Queries only (the
/// statement really runs). `tag` makes the Oracle cursor findable in v$sql even when DBMS
/// output or another call ran on the session in between. None = engine not supported.
pub fn autotrace(db: Db, sql: &str, tag: u64) -> Option<Explain> {
    let sql = sql.trim().trim_end_matches(';');
    // Read-only: starts with SELECT / WITH and no top-level DML (PostgreSQL `WITH ... DELETE`,
    // `INSERT ... SELECT`). `FOR UPDATE` is refused too, it would lock rows.
    let words = top_words(sql);
    if !words.first().is_some_and(|(_, w)| w == "SELECT" || w == "WITH") || words.iter().any(|(_, w)| matches!(w.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE")) {
        return None;
    }
    let one = |p: &str| Some(Explain { stmts: vec![format!("{p} {sql}")], show: 0 });
    match db {
        Db::Oracle => {
            let at = words.iter().find(|(_, w)| w == "SELECT")?.0;
            let marker = format!("rsqldev-autotrace-{tag}");
            let traced = format!("{} /*+ gather_plan_statistics */ /* {marker} */{}", &sql[..at + 6], &sql[at + 6..]);
            let show = format!(
                "SELECT t.plan_table_output FROM (SELECT sql_id, child_number FROM v$sql WHERE sql_text LIKE '%{marker}%' AND sql_text NOT LIKE '%v$sql%' ORDER BY last_active_time DESC FETCH FIRST 1 ROW ONLY) s, TABLE(DBMS_XPLAN.DISPLAY_CURSOR(s.sql_id, s.child_number, 'ALLSTATS LAST')) t"
            );
            Some(Explain { stmts: vec![traced, show], show: 1 })
        }
        Db::Postgres => one("EXPLAIN (ANALYZE, BUFFERS)"),
        Db::Mysql | Db::Duckdb => one("EXPLAIN ANALYZE"),
        _ => None,
    }
}

/// Words outside parentheses, strings and comments, upper case, with their byte offsets.
fn top_words(sql: &str) -> Vec<(usize, String)> {
    let b = sql.as_bytes();
    let (mut i, mut depth, mut out) = (0, 0i32, vec![]);
    while i < b.len() {
        let rest = &sql[i..];
        if rest.starts_with("--") {
            i = rest.find('\n').map_or(b.len(), |n| i + n);
        } else if rest.starts_with("/*") {
            i = rest[2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
        } else if b[i] == b'\'' || b[i] == b'"' {
            let q = b[i];
            i += 1;
            while i < b.len() && !(b[i] == q && b.get(i + 1) != Some(&q)) {
                i += if b[i] == q { 2 } else { 1 };
            }
            i += 1;
        } else if b[i] == b'(' {
            depth += 1;
            i += 1;
        } else if b[i] == b')' {
            depth -= 1;
            i += 1;
        } else if b[i].is_ascii_alphabetic() {
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '#')).map_or(b.len(), |n| i + n);
            if depth == 0 {
                out.push((i, sql[i..end].to_ascii_uppercase()));
            }
            i = end;
        } else {
            i += 1;
            while i < b.len() && !sql.is_char_boundary(i) {
                i += 1;
            }
        }
    }
    out
}

/// Query for an object's DDL: (sql, column), column `usize::MAX` = concatenate column 0 of
/// every row (source text). Oracle uses DBMS_METADATA for every kind.
pub fn ddl_sql(db: Db, kind: &str, owner: &str, name: &str) -> Option<(String, usize)> {
    let view = if db == Db::Oracle { ora_ddl(kind) } else { tabs(db, kind).into_iter().map(|t| t.view).find(|v| matches!(v, View::Ddl(..) | View::Code(_)))? };
    match view {
        View::Ddl(s, col) => Some((fill(db, &s, owner, name), col)),
        View::Code(s) => Some((fill(db, &s, owner, name), usize::MAX)),
        _ => None,
    }
}

/// Query for an object's source text to compare (cheaper than DDL where a Code view exists).
pub fn code_sql(db: Db, kind: &str, owner: &str, name: &str) -> Option<(String, usize)> {
    match tabs(db, kind).into_iter().map(|t| t.view).find(|v| matches!(v, View::Code(_))) {
        Some(View::Code(s)) => Some((fill(db, &s, owner, name), usize::MAX)),
        _ => ddl_sql(db, kind, owner, name),
    }
}

/// Run PL/SQL (Oracle): arguments of `[owner.][package.]name`. Columns: owner, package_name,
/// object_name, argument_name, position, data_type, in_out, overload; first overload first.
pub fn plsql_args_sql(ident: &str) -> String {
    let p: Vec<String> = ident.split('.').map(|s| lit(&fold(Db::Oracle, s.trim().trim_matches('"')))).collect();
    let cur = "SYS_CONTEXT('USERENV', 'CURRENT_SCHEMA')";
    let cond = match p.as_slice() {
        [n] => format!("object_name = '{n}' AND package_name IS NULL AND owner = {cur}"),
        [a, n] => format!("object_name = '{n}' AND ((owner = '{a}' AND package_name IS NULL) OR (package_name = '{a}' AND owner = {cur}))"),
        [o, k, n, ..] => format!("object_name = '{n}' AND owner = '{o}' AND package_name = '{k}'"),
        [] => "1 = 0".into(),
    };
    format!("SELECT owner, package_name, object_name, argument_name, position, data_type, in_out, overload FROM all_arguments WHERE data_level = 0 AND {cond} ORDER BY overload NULLS FIRST, position")
}

/// Anonymous block calling the procedure / function described by [`plsql_args_sql`] rows:
/// one variable per argument (IN values to fill in), OUT values and the return printed.
pub fn plsql_block(rows: &[Vec<Option<String>>]) -> Option<String> {
    let get = |r: &Vec<Option<String>>, i: usize| r.get(i).cloned().flatten().unwrap_or_default();
    let first = rows.first()?;
    let (owner, pkg, name, ov) = (get(first, 0), get(first, 1), get(first, 2), get(first, 7));
    let rows: Vec<_> = rows.iter().filter(|r| (get(r, 0), get(r, 1), get(r, 7)) == (owner.clone(), pkg.clone(), ov.clone())).collect();
    let ty = |dt: &str| match dt {
        "VARCHAR2" | "NVARCHAR2" | "CHAR" | "NCHAR" | "RAW" => format!("{dt}(32767)"),
        "REF CURSOR" => "SYS_REFCURSOR".into(),
        "PL/SQL BOOLEAN" => "BOOLEAN".into(),
        "NUMBER" | "DATE" | "CLOB" | "NCLOB" | "BLOB" | "BOOLEAN" | "BINARY_INTEGER" | "PLS_INTEGER" | "INTEGER" | "FLOAT" | "BINARY_FLOAT" | "BINARY_DOUBLE" | "ROWID" => dt.into(),
        t if t.starts_with("TIMESTAMP") || t.starts_with("INTERVAL") => t.into(),
        t => format!("{t} /* adjust type */"),
    };
    let printable = |dt: &str| !matches!(dt, "REF CURSOR" | "PL/SQL BOOLEAN" | "BOOLEAN" | "BLOB" | "PL/SQL RECORD" | "PL/SQL TABLE" | "TABLE" | "OBJECT" | "VARRAY");
    let target = [owner.as_str(), pkg.as_str(), name.as_str()].iter().filter(|s| !s.is_empty()).map(|s| quote(Db::Oracle, s)).collect::<Vec<_>>().join(".");
    let mut decl = String::new();
    let mut prints = String::new();
    let mut args = vec![];
    let mut ret = None;
    for r in &rows {
        let (arg, pos, dt, io) = (get(r, 3), get(r, 4), get(r, 5), get(r, 6));
        if arg.is_empty() {
            // Function return (position 0); a procedure without arguments has one empty row.
            if pos == "0" && !dt.is_empty() {
                decl += &format!("  v_return {};\n", ty(&dt));
                if printable(&dt) {
                    prints += "  DBMS_OUTPUT.PUT_LINE('v_return = ' || v_return);\n";
                }
                ret = Some(());
            }
            continue;
        }
        let v = quote(Db::Oracle, &arg);
        let init = if io == "OUT" { String::new() } else { " := NULL".into() };
        decl += &format!("  {v} {}{init};  -- {io}\n", ty(&dt));
        if io != "IN" && printable(&dt) {
            prints += &format!("  DBMS_OUTPUT.PUT_LINE('{} = ' || {v});\n", lit(&arg));
        }
        args.push(format!("{v} => {v}"));
    }
    let call = if args.is_empty() { target } else { format!("{target}(\n    {}\n  )", args.join(",\n    ")) };
    let call = if ret.is_some() { format!("v_return := {call}") } else { call };
    let decl = if decl.is_empty() { String::new() } else { format!("DECLARE\n{decl}") };
    Some(format!("SET SERVEROUTPUT ON\n{decl}BEGIN\n  {call};\n{prints}END;\n/\n"))
}

/// Columns query for DESCRIBE in the script runner.
pub fn describe_sql(db: Db, owner: &str, name: &str) -> Option<String> {
    children_sql(db, TABLE).map(|s| fill(db, &s, owner, name))
}

/// Folding identifiers the way the engine stores unquoted names.
pub fn fold(db: Db, ident: &str) -> String {
    match db {
        Db::Oracle => ident.to_uppercase(),
        Db::Postgres => ident.to_lowercase(),
        _ => ident.to_string(),
    }
}

/// One Sort… key: column name, descending, nulls first (None = engine default).
pub type SortKey = (String, bool, Option<bool>);

/// ` ORDER BY ...` for the data editor's Sort… dialog; empty for no keys. NULLS FIRST / LAST
/// only where the engine has it.
pub fn order_by(db: Db, keys: &[SortKey]) -> String {
    if keys.is_empty() {
        return String::new();
    }
    let nulls_ok = !matches!(db, Db::Mysql | Db::Mssql);
    let parts: Vec<String> = keys
        .iter()
        .map(|(c, desc, nulls)| {
            let mut p = quote(db, c);
            if *desc {
                p += " DESC";
            }
            match nulls {
                Some(true) if nulls_ok => p += " NULLS FIRST",
                Some(false) if nulls_ok => p += " NULLS LAST",
                _ => {}
            }
            p
        })
        .collect();
    format!(" ORDER BY {}", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_order_by() {
        let k = vec![("NAME".to_string(), false, Some(false)), ("id".to_string(), true, None)];
        assert_eq!(order_by(Db::Oracle, &k), r#" ORDER BY "NAME" NULLS LAST, "id" DESC"#);
        assert_eq!(order_by(Db::Mysql, &k), " ORDER BY `NAME`, `id` DESC");
        assert_eq!(order_by(Db::Postgres, &[]), "");
    }

    #[test]
    fn top_sql_queries() {
        let q = &top_sql(Db::Oracle, "cpu", "o'x").unwrap()[0];
        assert!(q.contains("ORDER BY cpu_time DESC") && q.contains("LIKE '%O''X%'") && q.contains("FROM v$sql"));
        assert_eq!(top_sql(Db::Postgres, "avg", "").unwrap().len(), 2);
        assert!(top_sql(Db::Mysql, "nope", "").unwrap()[0].contains("ORDER BY SUM_TIMER_WAIT DESC"));
        assert!(top_sql(Db::Sqlite, "elapsed", "").is_none());
        assert!(top_sql_plan("abc123', 1) --", "0").is_none());
        assert!(top_sql_plan("0w2qpuc6u2zsp", "1").unwrap().contains("DISPLAY_CURSOR('0w2qpuc6u2zsp', 1,"));
    }

    #[test]
    fn fill_single_pass() {
        assert_eq!(fill(Db::Oracle, "'{n}' {q} '{lq}'", "HR", "{q}'x"), "'{q}''x' \"HR\".\"{q}'x\" '\"HR\".\"{q}''x\"'");
        assert_eq!(fill(Db::Mysql, "'{n}'", "", "a\\"), "'a\\\\'");
    }

    #[test]
    fn plsql_run_block() {
        assert!(plsql_args_sql("hr.pkg.f").contains("owner = 'HR' AND package_name = 'PKG'"));
        let r = |v: &[&str]| v.iter().map(|x| (!x.is_empty()).then(|| x.to_string())).collect::<Vec<_>>();
        let rows = vec![
            r(&["HR", "PKG", "F", "", "0", "NUMBER", "OUT", "1"]),
            r(&["HR", "PKG", "F", "P_ID", "1", "NUMBER", "IN", "1"]),
            r(&["HR", "PKG", "F", "P_NAME", "2", "VARCHAR2", "OUT", "1"]),
            r(&["HR", "PKG", "F", "P_X", "1", "DATE", "IN", "2"]),
        ];
        let b = plsql_block(&rows).unwrap();
        assert_eq!(
            b,
            "SET SERVEROUTPUT ON\nDECLARE\n  v_return NUMBER;\n  \"P_ID\" NUMBER := NULL;  -- IN\n  \"P_NAME\" VARCHAR2(32767);  -- OUT\nBEGIN\n  v_return := \"HR\".\"PKG\".\"F\"(\n    \"P_ID\" => \"P_ID\",\n    \"P_NAME\" => \"P_NAME\"\n  );\n  DBMS_OUTPUT.PUT_LINE('v_return = ' || v_return);\n  DBMS_OUTPUT.PUT_LINE('P_NAME = ' || \"P_NAME\");\nEND;\n/\n"
        );
        let none = vec![r(&["HR", "", "P", "", "1", "", "IN", ""])];
        assert_eq!(plsql_block(&none).unwrap(), "SET SERVEROUTPUT ON\nBEGIN\n  \"HR\".\"P\";\nEND;\n/\n");
    }

    #[test]
    fn autotrace_hint() {
        let a = autotrace(Db::Oracle, "with q as (select 1 x from dual) select * from q;", 7).unwrap();
        assert_eq!(a.stmts[0], "with q as (select 1 x from dual) select /*+ gather_plan_statistics */ /* rsqldev-autotrace-7 */ * from q");
        assert!(a.stmts[1].contains("rsqldev-autotrace-7") && a.show == 1);
        assert!(autotrace(Db::Oracle, "update t set x = 1", 1).is_none());
        assert!(autotrace(Db::Oracle, "insert into t select * from s", 1).is_none());
        assert!(autotrace(Db::Postgres, "with d as (select 1) delete from t", 1).is_none());
        assert!(autotrace(Db::Postgres, "select * from t where s = 'delete'", 1).is_some());
        assert_eq!(autotrace(Db::Postgres, "select 1", 1).unwrap().stmts, vec!["EXPLAIN (ANALYZE, BUFFERS) select 1"]);
    }

    #[test]
    fn session_kill() {
        let r = |v: &[&str]| v.iter().map(|x| Some(x.to_string())).collect::<Vec<_>>();
        assert_eq!(kill_sql(Db::Oracle, &r(&["12", "345"])).unwrap(), "ALTER SYSTEM KILL SESSION '12,345' IMMEDIATE");
        assert_eq!(kill_sql(Db::Postgres, &r(&["99"])).unwrap(), "SELECT pg_terminate_backend(99)");
        assert!(kill_sql(Db::Mysql, &r(&["1; DROP TABLE x"])).is_none(), "non-numeric id refused");
        assert_eq!(kill_sql(Db::Clickhouse, &r(&["5a1c-9f_0"])).unwrap(), "KILL QUERY WHERE query_id = '5a1c-9f_0'");
        assert!(kill_sql(Db::Clickhouse, &r(&["a\\' OR 1=1 --"])).is_none());
    }

    #[test]
    fn counting() {
        assert_eq!(count_sql(Db::Oracle, "select * from emp;").unwrap(), "SELECT COUNT(*) FROM (select * from emp) rsq_count");
        assert!(count_sql(Db::Mssql, "with a as (select 1 x) select * from a").is_none());
        assert!(count_sql(Db::Oracle, "update t set a = 1").is_none());
    }

    #[test]
    fn create_templates_and_sequences() {
        let t = create_template(Db::Oracle, PROCEDURE, "HR").unwrap();
        assert!(t.starts_with("CREATE OR REPLACE PROCEDURE \"HR\".new_procedure") && t.ends_with("END;\n/\n"));
        assert!(create_template(Db::Mssql, VIEW, "dbo").unwrap().starts_with("CREATE OR ALTER VIEW [dbo].new_view"));
        assert!(create_template(Db::Clickhouse, SEQUENCE, "d").is_none());
        let row = |v: &[&str]| v.iter().map(|s| Some(s.to_string())).collect::<Vec<_>>();
        assert_eq!(alter_sequence(Db::Oracle, "HR", "S", &row(&["1", "1", "9999", "20", "N"])), "ALTER SEQUENCE \"HR\".\"S\"\n  INCREMENT BY 1\n  MINVALUE 1\n  MAXVALUE 9999\n  CACHE 20\n  NOCYCLE;\n");
        assert!(alter_sequence(Db::Postgres, "public", "s", &row(&["2", "1", "100", "1", "Y"])).ends_with("CACHE 1\n  CYCLE;\n"));
        assert!(sequence_sql(Db::Mysql, "a", "b").is_none());
    }

    #[test]
    fn templates_fill_and_every_folder_lists() {
        for db in [Db::Oracle, Db::Postgres, Db::Mysql, Db::Mssql, Db::Sqlite, Db::Duckdb, Db::Clickhouse] {
            for (_, kind) in folders(db) {
                let s = list_sql(db, kind, "O'X").unwrap_or_else(|| panic!("{db:?} {kind}"));
                assert!(!s.contains('{'), "{db:?} {kind}: {s}");
                if let Some(x) = extra_children_sql(db, kind) {
                    assert!(!fill(db, x, "O'X", "T").contains('{'), "{db:?} {kind} children");
                }
                for t in tabs(db, kind) {
                    if let View::Grid(s) | View::Details(s) | View::Code(s) | View::Ddl(s, _) = t.view {
                        let f = fill(db, &s, "O'X", "T");
                        assert!(!f.contains("{o}") && !f.contains("{n}") && !f.contains("{q}"), "{db:?} {kind} {}", t.label);
                    }
                }
            }
        }
        assert_eq!(fill(Db::Postgres, "'{lq}' '{o}'", "a'b", "t"), "'\"a''b\".\"t\"' 'a''b'");
        assert_eq!(explain(Db::Oracle, "select 1 from dual;").stmts[1], "EXPLAIN PLAN SET STATEMENT_ID = 'rsqldev' FOR select 1 from dual");
    }
}
