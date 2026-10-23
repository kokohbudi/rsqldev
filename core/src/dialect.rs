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
    tpl.replace("{lq}", &lit(&q)).replace("{q}", &q).replace("{o}", &lit(owner)).replace("{n}", &lit(name))
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
            ("Database Links", DBLINK),
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
        Db::Mysql => &[("Tables", TABLE), ("Views", VIEW), ("Procedures", PROCEDURE), ("Functions", FUNCTION), ("Triggers", TRIGGER)],
        Db::Mssql => &[
            ("Tables", TABLE),
            ("Views", VIEW),
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
        _ => return None,
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn templates_fill_and_every_folder_lists() {
        for db in [Db::Oracle, Db::Postgres, Db::Mysql, Db::Mssql, Db::Sqlite, Db::Duckdb, Db::Clickhouse] {
            for (_, kind) in folders(db) {
                let s = list_sql(db, kind, "O'X").unwrap_or_else(|| panic!("{db:?} {kind}"));
                assert!(!s.contains('{'), "{db:?} {kind}: {s}");
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
