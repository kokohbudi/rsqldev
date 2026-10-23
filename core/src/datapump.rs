//! Oracle Data Pump without SSH: every SQL / PL/SQL text is generated here (DBMS_DATAPUMP blocks)
//! so it can be shown before it runs and unit-tested. Names typed by the user are either
//! validated by [`valid_name`] or turned into escaped literals, never pasted raw.
//!
//! The encryption password is never in the text: blocks use the `:password` bind placeholder.

use crate::config::config_root;
use crate::dialect::{lit, quote};
use crate::registry::Kind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Full,
    Schemas(Vec<String>),
    Tables { schema: String, tables: Vec<String> },
    Tablespaces(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    All,
    MetadataOnly,
    DataOnly,
}

/// `All` and `DataOnly` need Advanced Compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    #[default]
    MetadataOnly,
    All,
    DataOnly,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableExists {
    Skip,
    Append,
    Truncate,
    Replace,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportSpec {
    pub job: String,
    pub mode: Mode,
    pub directory: String,
    pub dump_file: String,
    pub log_file: String,
    /// Empty = no limit (`2G`, `500M`).
    pub file_size: String,
    pub parallel: u32,
    pub content: Content,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// (schema, table, WHERE clause)
    pub table_filters: Vec<(String, String, String)>,
    pub consistent: bool,
    pub estimate_only: bool,
    pub compression: Compression,
    /// Needs Advanced Security; the password comes in as the `:password` bind.
    pub encrypt: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportSpec {
    pub job: String,
    pub mode: Mode,
    pub directory: String,
    pub dump_files: Vec<String>,
    pub log_file: String,
    /// Empty = real import; a name writes the DDL to that file instead.
    pub sql_file: String,
    pub parallel: u32,
    pub remap_schema: Vec<(String, String)>,
    pub remap_tablespace: Vec<(String, String)>,
    pub table_exists: TableExists,
    pub encrypted: bool,
}

/// `RSQ_EXP_20261023_101500`.
pub fn job_name(prefix: &str, now: chrono::NaiveDateTime) -> String {
    format!("{prefix}_{}", now.format("%Y%m%d_%H%M%S"))
}

/// (dump file, log file) defaults for a job.
pub fn default_files(job: &str, parallel: u32, file_size: &str) -> (String, String) {
    let dump = if parallel > 1 || !file_size.trim().is_empty() { format!("{job}_%U.dmp") } else { format!("{job}.dmp") };
    (dump, format!("{job}.log"))
}

/// Directory, job and file names: `[A-Za-z0-9_.$#%-]+`, no `..`.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty() && !s.contains("..") && s.chars().all(|c| c.is_ascii_alphanumeric() || "_.$#%-".contains(c))
}

/// `X_%U.dmp` -> `X_01.dmp` .. `X_<max>.dmp`; a name without `%U` is the only candidate.
pub fn percent_u_candidates(dump_file: &str, max: usize) -> Vec<String> {
    if !dump_file.contains("%U") {
        return vec![dump_file.to_string()];
    }
    (1..=max).map(|i| dump_file.replace("%U", &format!("{i:02}"))).collect()
}

fn check(what: &str, s: &str) -> Result<(), String> {
    if valid_name(s) { Ok(()) } else { Err(format!("invalid {what} name: {s:?}")) }
}

/// `'text'` with quotes doubled.
fn q(s: &str) -> String {
    format!("'{}'", lit(s))
}

/// Argument for an `*_EXPR` filter: `'IN (''A'', ''B'')'` (names are literals inside a literal).
fn in_expr(items: &[String]) -> String {
    let inner = items.iter().map(|x| q(x)).collect::<Vec<_>>().join(", ");
    q(&format!("IN ({inner})"))
}

fn nonempty(what: &str, v: &[String]) -> Result<(), String> {
    if v.is_empty() || v.iter().any(|x| x.trim().is_empty()) { Err(format!("{what} list is empty")) } else { Ok(()) }
}

/// (job_mode, filter lines) for a mode.
fn mode_parts(mode: &Mode) -> Result<(&'static str, Vec<String>), String> {
    Ok(match mode {
        Mode::Full => ("FULL", vec![]),
        Mode::Schemas(s) => {
            nonempty("schema", s)?;
            ("SCHEMA", vec![format!("  DBMS_DATAPUMP.METADATA_FILTER(h, 'SCHEMA_EXPR', {});", in_expr(s))])
        }
        Mode::Tables { schema, tables } => {
            nonempty("table", tables)?;
            if schema.trim().is_empty() {
                return Err("schema is empty".into());
            }
            (
                "TABLE",
                vec![
                    format!("  DBMS_DATAPUMP.METADATA_FILTER(h, 'SCHEMA_EXPR', {});", in_expr(std::slice::from_ref(schema))),
                    format!("  DBMS_DATAPUMP.METADATA_FILTER(h, 'NAME_EXPR', {});", in_expr(tables)),
                ],
            )
        }
        Mode::Tablespaces(t) => {
            nonempty("tablespace", t)?;
            ("TABLESPACE", vec![format!("  DBMS_DATAPUMP.METADATA_FILTER(h, 'TABLESPACE_EXPR', {});", in_expr(t))])
        }
    })
}

fn wrap(body: Vec<String>) -> String {
    let mut s = String::from("DECLARE\n  h NUMBER;\nBEGIN\n");
    for l in body {
        s.push_str(&l);
        s.push('\n');
    }
    s.push_str(
        "EXCEPTION WHEN OTHERS THEN\n  IF h IS NOT NULL THEN\n    DBMS_DATAPUMP.STOP_JOB(h);\n  END IF;\n  RAISE;\nEND;",
    );
    s
}

fn start(body: &mut Vec<String>) {
    body.push("  DBMS_DATAPUMP.START_JOB(h, cluster_ok => 0);".into());
    body.push("  DBMS_DATAPUMP.DETACH(h);".into());
}

pub fn export_block(s: &ExportSpec) -> Result<String, String> {
    check("directory", &s.directory)?;
    check("job", &s.job)?;
    check("dump file", &s.dump_file)?;
    check("log file", &s.log_file)?;
    if !s.file_size.is_empty() && !s.file_size.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("invalid file size: {:?}", s.file_size));
    }
    let (mode, filters) = mode_parts(&s.mode)?;
    let mut b = vec![format!("  h := DBMS_DATAPUMP.OPEN(operation => 'EXPORT', job_mode => '{mode}', job_name => {});", q(&s.job))];
    let size = if s.file_size.is_empty() { String::new() } else { format!(", filesize => {}", q(&s.file_size)) };
    b.push(format!(
        "  DBMS_DATAPUMP.ADD_FILE(h, {}, {}{size}, filetype => DBMS_DATAPUMP.KU$_FILE_TYPE_DUMP_FILE);",
        q(&s.dump_file),
        q(&s.directory)
    ));
    b.push(format!(
        "  DBMS_DATAPUMP.ADD_FILE(h, {}, {}, filetype => DBMS_DATAPUMP.KU$_FILE_TYPE_LOG_FILE);",
        q(&s.log_file),
        q(&s.directory)
    ));
    b.extend(filters);
    if !s.include.is_empty() {
        b.push(format!("  DBMS_DATAPUMP.METADATA_FILTER(h, 'INCLUDE_PATH_EXPR', {});", in_expr(&s.include)));
    }
    if !s.exclude.is_empty() {
        b.push(format!("  DBMS_DATAPUMP.METADATA_FILTER(h, 'EXCLUDE_PATH_EXPR', {});", in_expr(&s.exclude)));
    }
    for (schema, table, wh) in &s.table_filters {
        b.push(format!("  DBMS_DATAPUMP.DATA_FILTER(h, 'SUBQUERY', {}, {}, {});", q(wh), q(table), q(schema)));
    }
    match s.content {
        Content::All => {}
        Content::MetadataOnly => b.push("  DBMS_DATAPUMP.DATA_FILTER(h, 'INCLUDE_ROWS', 0);".into()),
        Content::DataOnly => b.push("  DBMS_DATAPUMP.SET_PARAMETER(h, 'INCLUDE_METADATA', 0);".into()),
    }
    if s.consistent {
        b.push("  DBMS_DATAPUMP.SET_PARAMETER(h, 'FLASHBACK_TIME', 'SYSTIMESTAMP');".into());
    }
    let comp = match s.compression {
        Compression::MetadataOnly => "METADATA_ONLY",
        Compression::All => "ALL",
        Compression::DataOnly => "DATA_ONLY",
        Compression::None => "NONE",
    };
    b.push(format!("  DBMS_DATAPUMP.SET_PARAMETER(h, 'COMPRESSION', '{comp}');"));
    if s.encrypt {
        b.push("  DBMS_DATAPUMP.SET_PARAMETER(h, 'ENCRYPTION', 'ALL');".into());
        b.push("  DBMS_DATAPUMP.SET_PARAMETER(h, 'ENCRYPTION_PASSWORD', :password);".into());
    }
    if s.estimate_only {
        b.push("  DBMS_DATAPUMP.SET_PARAMETER(h, 'ESTIMATE_ONLY', 1);".into());
    }
    if s.parallel > 1 {
        b.push(format!("  DBMS_DATAPUMP.SET_PARALLEL(h, {});", s.parallel));
    }
    start(&mut b);
    Ok(wrap(b))
}

pub fn import_block(s: &ImportSpec) -> Result<String, String> {
    check("directory", &s.directory)?;
    check("job", &s.job)?;
    check("log file", &s.log_file)?;
    if s.dump_files.is_empty() {
        return Err("no dump files".into());
    }
    for f in &s.dump_files {
        check("dump file", f)?;
    }
    let sql_file = !s.sql_file.is_empty();
    if sql_file {
        check("SQL file", &s.sql_file)?;
    }
    let (mode, filters) = mode_parts(&s.mode)?;
    let op = if sql_file { "SQL_FILE" } else { "IMPORT" };
    let mut b = vec![format!("  h := DBMS_DATAPUMP.OPEN(operation => '{op}', job_mode => '{mode}', job_name => {});", q(&s.job))];
    for f in &s.dump_files {
        b.push(format!(
            "  DBMS_DATAPUMP.ADD_FILE(h, {}, {}, filetype => DBMS_DATAPUMP.KU$_FILE_TYPE_DUMP_FILE);",
            q(f),
            q(&s.directory)
        ));
    }
    b.push(format!(
        "  DBMS_DATAPUMP.ADD_FILE(h, {}, {}, filetype => DBMS_DATAPUMP.KU$_FILE_TYPE_LOG_FILE);",
        q(&s.log_file),
        q(&s.directory)
    ));
    if sql_file {
        b.push(format!(
            "  DBMS_DATAPUMP.ADD_FILE(h, {}, {}, filetype => DBMS_DATAPUMP.KU$_FILE_TYPE_SQL_FILE);",
            q(&s.sql_file),
            q(&s.directory)
        ));
    }
    b.extend(filters);
    for (from, to) in &s.remap_schema {
        b.push(format!("  DBMS_DATAPUMP.METADATA_REMAP(h, 'REMAP_SCHEMA', {}, {});", q(from), q(to)));
    }
    for (from, to) in &s.remap_tablespace {
        b.push(format!("  DBMS_DATAPUMP.METADATA_REMAP(h, 'REMAP_TABLESPACE', {}, {});", q(from), q(to)));
    }
    let act = match s.table_exists {
        TableExists::Skip => "SKIP",
        TableExists::Append => "APPEND",
        TableExists::Truncate => "TRUNCATE",
        TableExists::Replace => "REPLACE",
    };
    b.push(format!("  DBMS_DATAPUMP.SET_PARAMETER(h, 'TABLE_EXISTS_ACTION', '{act}');"));
    if s.encrypted {
        b.push("  DBMS_DATAPUMP.SET_PARAMETER(h, 'ENCRYPTION_PASSWORD', :password);".into());
    }
    if s.parallel > 1 {
        b.push(format!("  DBMS_DATAPUMP.SET_PARALLEL(h, {});", s.parallel));
    }
    start(&mut b);
    Ok(wrap(b))
}

fn jobs_select(dba: bool, percent: bool) -> String {
    let (owner, view) = if dba { ("j.owner_name", "dba_datapump_jobs") } else { ("USER", "user_datapump_jobs") };
    let (pct, join) = if percent {
        (
            "l.percent",
            format!(
                " LEFT JOIN (SELECT opname, ROUND(sofar / NULLIF(totalwork, 0) * 100) AS percent, \
                 ROW_NUMBER() OVER (PARTITION BY opname ORDER BY last_update_time DESC) AS rn \
                 FROM v$session_longops WHERE totalwork > 0) l ON l.opname = j.job_name AND l.rn = 1"
            ),
        )
    } else {
        ("CAST(NULL AS NUMBER)", String::new())
    };
    format!(
        "SELECT {owner} AS owner, j.job_name, j.operation, j.job_mode, j.state, j.degree, j.attached_sessions, {pct} AS percent \
         FROM {view} j{join} ORDER BY 1, 2"
    )
}

/// Jobs list with percent from `V$SESSION_LONGOPS` (latest row per job).
pub fn jobs_sql(dba: bool) -> String {
    jobs_select(dba, true)
}

/// Same list without the longops join (percent NULL), for users who cannot read `V$SESSION_LONGOPS`.
pub fn jobs_sql_basic(dba: bool) -> String {
    jobs_select(dba, false)
}

/// One row, column 0 = state; no row when the job is gone.
pub fn job_running_sql(owner: &str, job: &str) -> String {
    format!(
        "SELECT state FROM dba_datapump_jobs WHERE owner_name = '{}' AND job_name = '{}'",
        lit(owner),
        lit(job)
    )
}

fn attach_block(owner: &str, job: &str, body: &[String]) -> String {
    let mut v = vec![format!("  h := DBMS_DATAPUMP.ATTACH({}, {});", q(job), q(owner))];
    v.extend(body.iter().cloned());
    wrap(v)
}

/// Stop keeps the master table (resumable): `STOP_JOB(h, 0, 1)`; immediate (kill) drops it: `STOP_JOB(h, 1, 0)`.
pub fn stop_block(owner: &str, job: &str, immediate: bool) -> String {
    let call = if immediate { "  DBMS_DATAPUMP.STOP_JOB(h, 1, 0);" } else { "  DBMS_DATAPUMP.STOP_JOB(h, 0, 1);" };
    attach_block(owner, job, &[call.to_string()])
}

pub fn resume_block(owner: &str, job: &str, parallel: Option<u32>) -> String {
    let mut body = vec![];
    if let Some(p) = parallel {
        body.push(format!("  DBMS_DATAPUMP.SET_PARALLEL(h, {p});"));
    }
    start(&mut body);
    attach_block(owner, job, &body)
}

/// Drops the master table of a NOT RUNNING job.
pub fn cleanup_sql(owner: &str, job: &str) -> String {
    format!("DROP TABLE {}.{} PURGE", quote(Kind::Oracle, owner), quote(Kind::Oracle, job))
}

/// One row: banner, DB_UNIQUE_NAME, CON_NAME, USER.
pub const ENV_SQL: &str = "SELECT (SELECT banner FROM v$version WHERE ROWNUM = 1), \
    SYS_CONTEXT('USERENV','DB_UNIQUE_NAME'), SYS_CONTEXT('USERENV','CON_NAME'), USER FROM dual";

pub fn is_enterprise(banner: &str) -> bool {
    banner.contains("Enterprise")
}

pub const DIRECTORIES_SQL: &str = "SELECT directory_name, directory_path FROM all_directories ORDER BY 1";

/// One-line hint with the GRANT to ask for, for the usual Data Pump / UTL_FILE errors.
pub fn hint(err: &str, user: &str, dir: &str) -> Option<String> {
    let has = |c: &str| err.contains(c);
    if has("PLS-00201") && has("UTL_FILE") {
        Some(format!("UTL_FILE is not visible: ask a DBA for GRANT EXECUTE ON SYS.UTL_FILE TO {user}"))
    } else if has("ORA-29283") || has("ORA-39070") || has("ORA-39087") || (has("ORA-39002") && has("ORA-39070")) {
        Some(format!("No access to the directory: ask a DBA for GRANT READ, WRITE ON DIRECTORY {dir} TO {user}"))
    } else if has("ORA-31631") || has("ORA-39181") {
        Some("Privileges are required: ask a DBA for GRANT DATAPUMP_EXP_FULL_DATABASE (export) / DATAPUMP_IMP_FULL_DATABASE (import)".into())
    } else if has("ORA-22285") || has("ORA-29280") {
        Some(format!("File or directory does not exist: check that {dir} exists and the file name is right"))
    } else {
        None
    }
}

/// Bytes per round trip when downloading a dump.
pub const DOWNLOAD_READ: usize = 1024 * 1024;

/// One download chunk. Binds: `:1` directory, `:2` file, `:3` 1-based start, `:4` max bytes,
/// out `:5` file length (NUMBER), out `:6` chunk (BLOB).
/// Positional binds are numbered by the order each placeholder first appears in the text, not by
/// its digit: keep `:1`..`:6` appearing in this order.
pub const DOWNLOAD_BLOCK: &str = "DECLARE\n  src BFILE := BFILENAME(:1, :2);\n  dst BLOB;\n  soff INTEGER := :3;\n  amt INTEGER := :4;\n  doff INTEGER := 1;\n  len INTEGER;\nBEGIN\n  \
DBMS_LOB.FILEOPEN(src, DBMS_LOB.FILE_READONLY);\n  len := DBMS_LOB.GETLENGTH(src);\n  :5 := len;\n  DBMS_LOB.CREATETEMPORARY(dst, TRUE);\n  \
IF soff <= len THEN\n    amt := LEAST(amt, len - soff + 1);\n    DBMS_LOB.LOADBLOBFROMFILE(dst, src, amt, doff, soff);\n  END IF;\n  \
DBMS_LOB.FILECLOSE(src);\n  :6 := dst;\nEXCEPTION WHEN OTHERS THEN\n  IF DBMS_LOB.FILEISOPEN(src) = 1 THEN DBMS_LOB.FILECLOSE(src); END IF;\n  RAISE;\nEND;";

/// Upload chunk size: one round trip carries this many bytes.
pub const UPLOAD_CHUNK: usize = 4 * 1024 * 1024;

/// Binds (positional): `:1` directory, `:2` file, `:3` open mode (`wb` first chunk, `ab` after), `:4` BLOB chunk.
/// `:4` is assigned after `FOPEN` so the binds appear in order 1, 2, 3, 4.
pub const UPLOAD_BLOCK: &str = "DECLARE\n  f UTL_FILE.FILE_TYPE;\n  b BLOB;\n  pos PLS_INTEGER := 1;\n  len PLS_INTEGER;\nBEGIN\n  \
    f := UTL_FILE.FOPEN(:1, :2, :3, 32767);\n  b := :4;\n  len := DBMS_LOB.GETLENGTH(b);\n  \
    WHILE pos <= len LOOP\n    UTL_FILE.PUT_RAW(f, DBMS_LOB.SUBSTR(b, 32767, pos), TRUE);\n    pos := pos + 32767;\n  END LOOP;\n  \
    UTL_FILE.FCLOSE(f);\nEXCEPTION WHEN OTHERS THEN\n  IF UTL_FILE.IS_OPEN(f) THEN\n    UTL_FILE.FCLOSE(f);\n  END IF;\n  RAISE;\nEND;";

/// One row: FILEEXISTS (0/1), length in bytes (NULL when the file is missing).
pub fn file_exists_sql(dir: &str, file: &str) -> Result<String, String> {
    check("directory", dir)?;
    check("file", file)?;
    Ok(format!(
        "WITH f AS (SELECT BFILENAME({}, {}) AS b FROM dual) \
         SELECT DBMS_LOB.FILEEXISTS(b), CASE WHEN DBMS_LOB.FILEEXISTS(b) = 1 THEN DBMS_LOB.GETLENGTH(b) END FROM f",
        q(dir),
        q(file)
    ))
}

pub fn delete_block(dir: &str, file: &str) -> Result<String, String> {
    check("directory", dir)?;
    check("file", file)?;
    Ok(format!("BEGIN UTL_FILE.FREMOVE({}, {}); END;", q(dir), q(file)))
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Dump,
    Log,
    SqlFile,
    Upload,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileRecord {
    pub conn: String,
    /// `DB_UNIQUE_NAME/CON_NAME`; empty when unknown.
    pub db: String,
    pub directory: String,
    pub file: String,
    pub kind: FileKind,
    pub job: String,
    pub created: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Files {
    #[serde(default)]
    file: Vec<FileRecord>,
}

const FILES_TOML: &str = "datapump.toml";

/// Empty on a missing or unreadable file.
pub fn load_files() -> Vec<FileRecord> {
    std::fs::read_to_string(config_root().join(FILES_TOML))
        .ok()
        .and_then(|s| toml::from_str::<Files>(&s).ok())
        .map(|f| f.file)
        .unwrap_or_default()
}

pub fn save_files(v: &[FileRecord]) -> anyhow::Result<()> {
    std::fs::create_dir_all(config_root())?;
    std::fs::write(config_root().join(FILES_TOML), toml::to_string_pretty(&Files { file: v.to_vec() })?)?;
    Ok(())
}

/// Same database identity, or the same connection name when `db` is empty.
pub fn files_for<'a>(all: &'a [FileRecord], conn: &str, db: &str) -> Vec<&'a FileRecord> {
    all.iter().filter(|r| if db.is_empty() { r.conn == conn } else { r.db == db }).collect()
}

pub fn rename_conn(all: &mut [FileRecord], old: &str, new: &str) {
    for r in all.iter_mut().filter(|r| r.conn == old) {
        r.conn = new.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_block_binds() {
        assert!(DOWNLOAD_BLOCK.contains("LOADBLOBFROMFILE"));
        for i in 1..=6 {
            assert!(DOWNLOAD_BLOCK.contains(&format!(":{i}")), ":{i}");
        }
        assert!(!DOWNLOAD_BLOCK.contains(":7"));
        let pos = |b: &str, i: usize| b.find(&format!(":{i}")).unwrap();
        for i in 1..6 {
            assert!(pos(DOWNLOAD_BLOCK, i) < pos(DOWNLOAD_BLOCK, i + 1), "download :{i} before :{}", i + 1);
        }
        for i in 1..4 {
            assert!(pos(UPLOAD_BLOCK, i) < pos(UPLOAD_BLOCK, i + 1), "upload :{i} before :{}", i + 1);
        }
    }

    fn spec() -> ExportSpec {
        ExportSpec {
            job: "RSQ_EXP_1".into(),
            mode: Mode::Schemas(vec!["HR".into(), "O'X".into()]),
            directory: "DATA_PUMP_DIR".into(),
            dump_file: "RSQ_EXP_1_%U.dmp".into(),
            log_file: "RSQ_EXP_1.log".into(),
            file_size: "2G".into(),
            parallel: 4,
            content: Content::All,
            include: vec![],
            exclude: vec!["STATISTICS".into()],
            table_filters: vec![],
            consistent: true,
            estimate_only: false,
            compression: Compression::MetadataOnly,
            encrypt: true,
        }
    }

    #[test]
    fn export_schemas_block() {
        let b = export_block(&spec()).unwrap();
        assert!(b.contains("DBMS_DATAPUMP.OPEN(operation => 'EXPORT', job_mode => 'SCHEMA', job_name => 'RSQ_EXP_1')"));
        assert!(b.contains("'SCHEMA_EXPR', 'IN (''HR'', ''O''''X'')'"));
        assert!(b.contains("'EXCLUDE_PATH_EXPR', 'IN (''STATISTICS'')'"));
        assert!(b.contains("filesize => '2G'") && b.contains("DBMS_DATAPUMP.SET_PARALLEL(h, 4)"));
        assert!(b.contains("'FLASHBACK_TIME', 'SYSTIMESTAMP'") && b.contains("'ENCRYPTION_PASSWORD', :password"));
        assert!(b.contains("START_JOB(h, cluster_ok => 0)") && b.contains("DBMS_DATAPUMP.DETACH(h)"));
        assert!(b.contains("EXCEPTION WHEN OTHERS THEN") && b.trim_end().ends_with("END;"));
    }

    #[test]
    fn import_remap_and_sqlfile() {
        let mut s = ImportSpec {
            job: "RSQ_IMP_1".into(),
            mode: Mode::Full,
            directory: "D".into(),
            dump_files: vec!["a_01.dmp".into(), "a_02.dmp".into()],
            log_file: "RSQ_IMP_1.log".into(),
            sql_file: String::new(),
            parallel: 1,
            remap_schema: vec![("HR".into(), "HR2".into())],
            remap_tablespace: vec![],
            table_exists: TableExists::Truncate,
            encrypted: false,
        };
        let b = import_block(&s).unwrap();
        assert!(b.contains("'IMPORT', job_mode => 'FULL'") && b.contains("'REMAP_SCHEMA', 'HR', 'HR2'") && b.contains("'TABLE_EXISTS_ACTION', 'TRUNCATE'"));
        assert!(!b.contains(":password"));
        s.sql_file = "a.sql".into();
        assert!(import_block(&s).unwrap().contains("'SQL_FILE'"));
    }

    #[test]
    fn names_are_validated() {
        assert!(valid_name("RSQ_EXP_1_%U.dmp") && !valid_name("x'; DROP") && !valid_name("../etc") && !valid_name(""));
        let mut s = spec();
        s.directory = "DIR'X".into();
        assert!(export_block(&s).is_err());
    }

    #[test]
    fn percent_u_candidates() {
        assert_eq!(super::percent_u_candidates("X_%U.dmp", 3), vec!["X_01.dmp", "X_02.dmp", "X_03.dmp"]);
        assert_eq!(super::percent_u_candidates("X.dmp", 3), vec!["X.dmp"]);
        assert_eq!(
            job_name("RSQ_EXP", chrono::NaiveDate::from_ymd_opt(2026, 10, 23).unwrap().and_hms_opt(10, 15, 0).unwrap()),
            "RSQ_EXP_20261023_101500"
        );
    }

    #[test]
    fn job_control_sql() {
        assert!(stop_block("HR", "RSQ_EXP_1", false).contains("DBMS_DATAPUMP.ATTACH('RSQ_EXP_1', 'HR')"));
        assert!(stop_block("HR", "J", false).contains("STOP_JOB(h, 0, 1)") && stop_block("HR", "J", true).contains("STOP_JOB(h, 1, 0)"));
        assert!(resume_block("HR", "J", Some(2)).contains("SET_PARALLEL(h, 2)"));
        assert_eq!(cleanup_sql("HR", "J"), "DROP TABLE \"HR\".\"J\" PURGE");
        assert!(jobs_sql(true).contains("dba_datapump_jobs") && jobs_sql(false).contains("user_datapump_jobs"));
        assert!(is_enterprise("Oracle Database 19c Enterprise Edition Release 19.0.0.0.0") && !is_enterprise("Oracle Database 19c Standard Edition 2"));
    }

    #[test]
    fn job_running_check_sql() {
        assert!(job_running_sql("HR", "O'X").contains("job_name = 'O''X'"));
    }

    #[test]
    fn hints_name_the_grant() {
        assert!(hint("ORA-29283: invalid file operation", "SCOTT", "DATA_PUMP_DIR").unwrap().contains("GRANT READ, WRITE ON DIRECTORY DATA_PUMP_DIR TO SCOTT"));
        assert!(hint("PLS-00201: identifier 'UTL_FILE' must be declared", "SCOTT", "D").unwrap().contains("GRANT EXECUTE ON SYS.UTL_FILE TO SCOTT"));
        assert!(hint("ORA-31631: privileges are required", "SCOTT", "D").unwrap().contains("DATAPUMP_EXP_FULL_DATABASE"));
        assert!(hint("ORA-22285: non-existent directory or file for FILEOPEN operation", "SCOTT", "D").unwrap().contains("does not exist"));
        assert!(hint("ORA-00001", "SCOTT", "D").is_none());
    }

    #[test]
    fn upload_and_file_sql_bind_names() {
        assert!(UPLOAD_BLOCK.contains("UTL_FILE.FOPEN(:1, :2, :3, 32767)") && UPLOAD_BLOCK.contains("DBMS_LOB.SUBSTR(b, 32767, pos)"));
        assert!(file_exists_sql("D", "x.dmp").unwrap().contains("BFILENAME('D', 'x.dmp')"));
        assert!(file_exists_sql("D", "x'.dmp").is_err() && delete_block("D", "../x").is_err());
    }

    #[test]
    fn file_records() {
        let r = |c: &str, db: &str| FileRecord { conn: c.into(), db: db.into(), directory: "D".into(), file: "a.dmp".into(), kind: FileKind::Dump, job: "J".into(), created: "2026-10-23 10:00".into() };
        let mut all = vec![r("prod", "ORCL/PDB1"), r("prod2", "ORCL/PDB1"), r("dev", "DEV/DEV")];
        assert_eq!(files_for(&all, "x", "ORCL/PDB1").len(), 2);
        rename_conn(&mut all, "dev", "dev-new");
        assert_eq!(all[2].conn, "dev-new");
        let t = toml::to_string(&Files { file: all.clone() }).unwrap();
        assert_eq!(toml::from_str::<Files>(&t).unwrap().file, all);
    }
}
