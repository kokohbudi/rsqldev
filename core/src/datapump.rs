//! Oracle Data Pump without SSH: every SQL / PL/SQL text is generated here (DBMS_DATAPUMP blocks)
//! so it can be shown before it runs and unit-tested. Names typed by the user are either
//! validated by [`valid_name`] or turned into escaped literals, never pasted raw.
//!
//! The encryption password is never in the text: blocks use the `:password` bind placeholder.

use crate::dialect::lit;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
