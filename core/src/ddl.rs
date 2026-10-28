//! Table editor DDL (Create / Edit Table, indexes), SQL Developer style: the dialog builds a
//! [`TableDef`], and CREATE or the ALTERs between the loaded and the edited definition are
//! generated per engine. Statements are shown before anything runs.
//!
//! Names: `"quoted"` text is kept exactly; a plain identifier is folded the way the engine
//! stores unquoted names (Oracle upper case, PostgreSQL lower case), like typing it in SQL.

use crate::dialect::{fold, lit, quote};
use crate::registry::Kind as Db;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColumnDef {
    /// Name as loaded (edit mode): renames and changes are found through it. None = new.
    pub orig: Option<String>,
    pub name: String,
    /// Base type, e.g. VARCHAR2, NUMBER.
    pub ty: String,
    /// Length / precision, may hold a unit: `100`, `100 CHAR`.
    pub size: String,
    pub scale: String,
    pub not_null: bool,
    /// Default expression, as written in SQL (`'text'`, `0`, `SYSDATE`).
    pub default: String,
    pub pk: bool,
    pub comment: String,
    /// Oracle `DEFAULT ON NULL`.
    pub default_on_null: bool,
    /// Virtual / generated column expression (empty = a stored column).
    pub virtual_expr: String,
    pub identity: Option<Identity>,
}

/// Identity / auto-increment column (SQL Developer's Identity Column panel).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Identity {
    /// `ALWAYS`, `BY DEFAULT` or `BY DEFAULT ON NULL` (Oracle).
    pub generate: String,
    pub start: String,
    pub increment: String,
    pub min: String,
    pub max: String,
    pub cycle: bool,
    /// Cache size; `NOCACHE` turns caching off.
    pub cache: String,
    pub order: bool,
}

impl Identity {
    /// Parse Oracle's `ALL_TAB_IDENTITY_COLS.IDENTITY_OPTIONS` (`START WITH: 1, INCREMENT BY: 1, …`),
    /// keeping values that differ from the defaults.
    pub fn parse(generate: &str, options: &str) -> Identity {
        let mut id = Identity { generate: generate.trim().to_uppercase(), ..Default::default() };
        for part in options.split(',') {
            let Some((k, v)) = part.split_once(':') else { continue };
            let v = v.trim().to_string();
            match k.trim() {
                "START WITH" if v != "1" => id.start = v,
                "INCREMENT BY" if v != "1" => id.increment = v,
                "MIN_VALUE" if v != "1" => id.min = v,
                "MAX_VALUE" if v != "9999999999999999999999999999" => id.max = v,
                "CYCLE_FLAG" => id.cycle = v == "Y",
                "CACHE_SIZE" if v != "20" => id.cache = if v == "0" { "NOCACHE".into() } else { v },
                "ORDER_FLAG" => id.order = v == "Y",
                _ => {}
            }
        }
        id
    }
}

/// Oracle table storage (Storage Options panel). Empty fields are left out; in Edit Table a
/// filled field becomes an ALTER.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Storage {
    pub tablespace: String,
    pub pct_free: String,
    pub ini_trans: String,
    /// `LOGGING` / `NOLOGGING`.
    pub logging: String,
    /// `COMPRESS` / `NOCOMPRESS`.
    pub compress: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConKind {
    #[default]
    Unique,
    Foreign,
    Check,
}

impl ConKind {
    pub const ALL: [ConKind; 3] = [ConKind::Unique, ConKind::Foreign, ConKind::Check];

    pub fn label(self) -> &'static str {
        match self {
            ConKind::Unique => "UNIQUE",
            ConKind::Foreign => "FOREIGN KEY",
            ConKind::Check => "CHECK",
        }
    }

    pub fn parse(s: &str) -> Option<ConKind> {
        let s = s.trim().to_uppercase();
        ConKind::ALL.into_iter().find(|k| k.label() == s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstraintDef {
    /// Loaded from the database: definition read-only; name and Enabled can change, or drop it.
    pub existing: bool,
    /// Name as loaded (existing constraints): a different `name` is a rename.
    pub orig: String,
    pub name: String,
    pub enabled: bool,
    pub kind: ConKind,
    /// Comma-separated column names.
    pub columns: String,
    /// `TABLE` or `OWNER.TABLE`.
    pub ref_table: String,
    pub ref_columns: String,
    /// Empty, `CASCADE` or `SET NULL`.
    pub on_delete: String,
    pub check: String,
    /// Description of an existing constraint.
    pub info: String,
}

impl Default for ConstraintDef {
    fn default() -> Self {
        ConstraintDef { existing: false, orig: String::new(), name: String::new(), enabled: true, kind: ConKind::Unique, columns: String::new(), ref_table: String::new(), ref_columns: String::new(), on_delete: String::new(), check: String::new(), info: String::new() }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexDef {
    pub existing: bool,
    pub name: String,
    /// Comma-separated, each optionally followed by ASC / DESC.
    pub columns: String,
    pub unique: bool,
    pub info: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TableDef {
    pub owner: String,
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub constraints: Vec<ConstraintDef>,
    pub indexes: Vec<IndexDef>,
    /// Primary key constraint name (loaded, or typed on the Primary Key page).
    pub pk_name: String,
    /// Primary key column order from the Primary Key page (empty = PK flags in column order).
    pub pk_order: Vec<String>,
    /// Oracle: the primary key is DISABLEd (Primary Key page ▸ Enabled unticked).
    pub pk_disabled: bool,
    pub comment: String,
    /// Create only: `Some(true)` session (ON COMMIT PRESERVE ROWS), `Some(false)` transaction.
    pub temporary: Option<bool>,
    pub storage: Storage,
}

/// Common column types for the type dropdown (any other type can be typed in the DDL).
pub fn types(db: Db) -> &'static [&'static str] {
    match db {
        Db::Oracle => &["VARCHAR2", "NVARCHAR2", "CHAR", "NUMBER", "INTEGER", "FLOAT", "BINARY_DOUBLE", "DATE", "TIMESTAMP", "TIMESTAMP WITH TIME ZONE", "CLOB", "NCLOB", "BLOB", "RAW", "BOOLEAN", "JSON"],
        Db::Postgres => &["integer", "bigint", "smallint", "numeric", "real", "double precision", "varchar", "char", "text", "boolean", "date", "timestamp", "timestamptz", "time", "uuid", "jsonb", "json", "bytea", "serial", "bigserial"],
        Db::Mysql => &["INT", "BIGINT", "SMALLINT", "TINYINT", "DECIMAL", "DOUBLE", "FLOAT", "VARCHAR", "CHAR", "TEXT", "LONGTEXT", "DATE", "DATETIME", "TIMESTAMP", "TIME", "BOOLEAN", "JSON", "BLOB", "LONGBLOB"],
        Db::Mssql => &["int", "bigint", "smallint", "tinyint", "bit", "decimal", "numeric", "float", "real", "money", "varchar", "nvarchar", "char", "nchar", "date", "datetime2", "datetime", "datetimeoffset", "time", "uniqueidentifier", "varbinary"],
        Db::Sqlite => &["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC"],
        Db::Duckdb => &["INTEGER", "BIGINT", "SMALLINT", "DECIMAL", "DOUBLE", "REAL", "VARCHAR", "BOOLEAN", "DATE", "TIMESTAMP", "TIMESTAMPTZ", "TIME", "UUID", "BLOB", "JSON"],
        Db::Clickhouse => &["Int32", "Int64", "UInt32", "UInt64", "Float64", "Decimal", "String", "FixedString", "Date", "DateTime", "DateTime64", "UUID", "Bool"],
    }
}

/// Column comments can be written (MySQL inline, others COMMENT ON / COMMENT COLUMN).
pub fn supports_comments(db: Db) -> bool {
    matches!(db, Db::Oracle | Db::Postgres | Db::Mysql | Db::Duckdb | Db::Clickhouse)
}

/// Table comments can be written and loaded.
pub fn supports_table_comment(db: Db) -> bool {
    matches!(db, Db::Oracle | Db::Postgres | Db::Mysql | Db::Duckdb | Db::Clickhouse)
}

/// Identity / auto-increment columns, and which Generate choices the engine has.
pub fn identity_kinds(db: Db) -> &'static [&'static str] {
    match db {
        Db::Oracle => &["ALWAYS", "BY DEFAULT", "BY DEFAULT ON NULL"],
        Db::Postgres => &["ALWAYS", "BY DEFAULT"],
        Db::Mysql | Db::Mssql | Db::Sqlite => &["BY DEFAULT"],
        _ => &[],
    }
}

/// Constraints can be added / dropped after CREATE TABLE.
pub fn alters_constraints(db: Db) -> bool {
    matches!(db, Db::Oracle | Db::Postgres | Db::Mysql | Db::Mssql | Db::Clickhouse)
}

/// `"Exact"` kept, plain identifiers folded, anything else used as typed.
pub fn ident(db: Db, s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        return s[1..s.len() - 1].replace("\"\"", "\"");
    }
    let plain = s.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') && s.chars().all(|c| c.is_alphanumeric() || "_$#".contains(c));
    if plain { fold(db, s) } else { s.to_string() }
}

/// How a stored name is shown for editing: quoted when typing it plain would fold it into
/// another name.
pub fn shown(db: Db, stored: &str) -> String {
    if ident(db, stored) == stored { stored.to_string() } else { format!("\"{}\"", stored.replace('"', "\"\"")) }
}

pub(crate) fn q(db: Db, name: &str) -> String {
    quote(db, &ident(db, name))
}

fn table_q(db: Db, t: &TableDef) -> String {
    let n = ident(db, &t.name);
    if t.owner.is_empty() { quote(db, &n) } else { format!("{}.{}", quote(db, &t.owner), quote(db, &n)) }
}

/// `a, b DESC` → quoted, comma-joined.
pub(crate) fn col_list(db: Db, s: &str, order: bool) -> Result<String, String> {
    let mut out = vec![];
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, dir) = match part.rsplit_once(char::is_whitespace) {
            Some((n, d)) if order && (d.eq_ignore_ascii_case("ASC") || d.eq_ignore_ascii_case("DESC")) => (n.trim(), format!(" {}", d.to_uppercase())),
            _ => (part, String::new()),
        };
        out.push(format!("{}{dir}", q(db, name)));
    }
    if out.is_empty() { Err("a column list is empty".into()) } else { Ok(out.join(", ")) }
}

impl ColumnDef {
    pub fn full_type(&self) -> String {
        let ty = self.ty.trim();
        match (self.size.trim(), self.scale.trim()) {
            ("", _) => ty.to_string(),
            (s, "") => format!("{ty}({s})"),
            (s, sc) => format!("{ty}({s},{sc})"),
        }
    }

    /// Split a loaded type (`VARCHAR2(100 BYTE)`, `NUMBER(10,2)`) into type, size and scale.
    /// Types with text after the parentheses stay whole.
    pub fn set_type(&mut self, full: &str) {
        let full = full.trim();
        (self.ty, self.size, self.scale) = match (full.find('('), full.rfind(')')) {
            (Some(a), Some(b)) if b == full.len() - 1 && a < b => {
                let inner = &full[a + 1..b];
                let (s, sc) = inner.split_once(',').map_or((inner, ""), |(s, sc)| (s, sc));
                (full[..a].trim().to_string(), s.trim().to_string(), sc.trim().to_string())
            }
            _ => (full.to_string(), String::new(), String::new()),
        };
    }

    fn ch_type(&self) -> String {
        if self.not_null || self.pk { self.full_type() } else { format!("Nullable({})", self.full_type()) }
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<String>().to_uppercase()
}

/// Sequence numbers in identity options: an optional minus and digits.
fn number(what: &str, v: &str) -> Result<String, String> {
    let v = v.trim();
    let digits = v.strip_prefix('-').unwrap_or(v);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{what} must be a whole number, not '{v}'"));
    }
    Ok(v.to_string())
}

/// `GENERATED … AS IDENTITY (…)` / `IDENTITY(s, i)` / `AUTO_INCREMENT` for a column.
fn identity_sql(db: Db, c: &ColumnDef, id: &Identity) -> Result<String, String> {
    let name = ident(db, &c.name);
    let kind = id.generate.trim().to_uppercase();
    let kind = if kind.is_empty() { "BY DEFAULT".to_string() } else { kind };
    if !identity_kinds(db).contains(&kind.as_str()) {
        return Err(format!("column {name}: {} has no GENERATED {kind} identity", engine(db)));
    }
    let mut opts = vec![];
    for (kw, v) in [("START WITH", &id.start), ("INCREMENT BY", &id.increment), ("MINVALUE", &id.min), ("MAXVALUE", &id.max)] {
        if !v.trim().is_empty() {
            opts.push(format!("{kw} {}", number(&format!("column {name}: {kw}"), v)?));
        }
    }
    let seq_opts = |opts: &mut Vec<String>| -> Result<(), String> {
        if id.cycle {
            opts.push("CYCLE".into());
        }
        match id.cache.trim().to_uppercase().as_str() {
            "" => {}
            "NOCACHE" if db == Db::Oracle => opts.push("NOCACHE".into()),
            v => opts.push(format!("CACHE {}", number(&format!("column {name}: CACHE"), v)?)),
        }
        if id.order && db == Db::Oracle {
            opts.push("ORDER".into());
        }
        Ok(())
    };
    Ok(match db {
        Db::Oracle | Db::Postgres => {
            seq_opts(&mut opts)?;
            let o = if opts.is_empty() { String::new() } else { format!(" ({})", opts.join(" ")) };
            format!("GENERATED {kind} AS IDENTITY{o}")
        }
        Db::Mssql => format!("IDENTITY({}, {})", if id.start.trim().is_empty() { "1".into() } else { number("START WITH", &id.start)? }, if id.increment.trim().is_empty() { "1".into() } else { number("INCREMENT BY", &id.increment)? }),
        Db::Mysql => "AUTO_INCREMENT".into(),
        Db::Sqlite => "PRIMARY KEY AUTOINCREMENT".into(),
        _ => return Err(format!("{} has no identity columns", engine(db))),
    })
}

/// SQLite AUTOINCREMENT sits on the column as its (only) primary key.
fn sqlite_autoinc(db: Db, t: &TableDef) -> Result<bool, String> {
    let Some(c) = t.columns.iter().find(|c| c.identity.is_some()) else { return Ok(false) };
    if db != Db::Sqlite {
        return Ok(false);
    }
    if !c.ty.trim().eq_ignore_ascii_case("INTEGER") || pk_cols(db, t) != vec![ident(db, &c.name)] {
        return Err("SQLite AUTOINCREMENT needs an INTEGER column that is the only primary key column".into());
    }
    Ok(true)
}

/// `name TYPE [DEFAULT x] [NOT NULL] [COMMENT '...']`, with identity / virtual clauses.
pub(crate) fn col_def(db: Db, c: &ColumnDef) -> Result<String, String> {
    let name = ident(db, &c.name);
    let expr = c.virtual_expr.trim();
    let default = c.default.trim();
    if !expr.is_empty() && (c.identity.is_some() || !default.is_empty()) {
        return Err(format!("column {name}: a virtual column has no default or identity"));
    }
    if c.identity.is_some() && !default.is_empty() {
        return Err(format!("column {name}: an identity column has no default"));
    }
    if c.default_on_null && db != Db::Oracle {
        return Err(format!("column {name}: DEFAULT ON NULL is Oracle only"));
    }
    let ty = if db == Db::Clickhouse { c.ch_type() } else { c.full_type() };
    let mut s = if db == Db::Mssql && !expr.is_empty() { q(db, &c.name) } else { format!("{} {ty}", q(db, &c.name)) };
    if !expr.is_empty() {
        s += &match db {
            Db::Postgres => format!(" GENERATED ALWAYS AS ({expr}) STORED"),
            Db::Mssql => format!(" AS ({expr})"),
            Db::Clickhouse => format!(" ALIAS {expr}"),
            _ => format!(" GENERATED ALWAYS AS ({expr}) VIRTUAL"),
        };
    }
    if !default.is_empty() {
        s += &format!(" DEFAULT {}{default}", if c.default_on_null { "ON NULL " } else { "" });
    }
    if let Some(id) = &c.identity {
        s += &format!(" {}", identity_sql(db, c, id)?);
    }
    if c.not_null && db != Db::Clickhouse && !(db == Db::Mssql && !expr.is_empty()) {
        s += " NOT NULL";
    }
    if matches!(db, Db::Mysql | Db::Clickhouse) && !c.comment.trim().is_empty() {
        s += &format!(" COMMENT '{}'", lit(c.comment.trim()));
    }
    Ok(s)
}

pub(crate) fn con_def(db: Db, c: &ConstraintDef) -> Result<String, String> {
    let name = if c.name.trim().is_empty() { String::new() } else { format!("CONSTRAINT {} ", q(db, &c.name)) };
    if db == Db::Clickhouse && c.kind != ConKind::Check {
        return Err("ClickHouse has CHECK constraints only".into());
    }
    Ok(match c.kind {
        ConKind::Unique => format!("{name}UNIQUE ({})", col_list(db, &c.columns, false)?),
        ConKind::Foreign => {
            let rt = c.ref_table.trim();
            if rt.is_empty() {
                return Err(format!("foreign key {}: the referenced table is empty", c.name));
            }
            let rt = match rt.split_once('.') {
                Some((o, t)) => format!("{}.{}", q(db, o), q(db, t)),
                None => q(db, rt),
            };
            let on = match c.on_delete.trim().to_uppercase().as_str() {
                "" => String::new(),
                d @ ("CASCADE" | "SET NULL") => format!(" ON DELETE {d}"),
                d => return Err(format!("ON DELETE {d} is not CASCADE or SET NULL")),
            };
            format!("{name}FOREIGN KEY ({}) REFERENCES {rt} ({}){on}", col_list(db, &c.columns, false)?, col_list(db, &c.ref_columns, false)?)
        }
        ConKind::Check if c.check.trim().is_empty() => return Err(format!("check constraint {}: the condition is empty", c.name)),
        ConKind::Check if db == Db::Clickhouse => format!("{name}CHECK {}", c.check.trim()),
        ConKind::Check => format!("{name}CHECK ({})", c.check.trim()),
    } + match (c.enabled, db) {
        (true, _) | (false, Db::Mssql) => "",
        (false, Db::Oracle) => " DISABLE",
        (false, _) => return Err(format!("{} cannot create a disabled constraint", engine(db))),
    })
}

/// Turn a constraint on / off (Oracle ENABLE / DISABLE, SQL Server CHECK / NOCHECK).
fn enable_sql(db: Db, t: &str, c: &ConstraintDef) -> Result<String, String> {
    let n = quote(db, &ident(db, &c.name));
    match db {
        Db::Oracle => Ok(format!("ALTER TABLE {t} {} CONSTRAINT {n}", if c.enabled { "ENABLE" } else { "DISABLE" })),
        Db::Mssql if c.kind != ConKind::Unique => Ok(format!("ALTER TABLE {t} {} CONSTRAINT {n}", if c.enabled { "WITH CHECK CHECK" } else { "NOCHECK" })),
        _ => Err(format!("{} cannot enable or disable constraint {n}", engine(db))),
    }
}

/// Rename a constraint (existing constraint or primary key with a new name).
pub(crate) fn rename_constraint(db: Db, t: &str, owner: &str, old: &str, new: &str) -> Result<String, String> {
    let nn = ident(db, new);
    match db {
        Db::Oracle | Db::Postgres => Ok(format!("ALTER TABLE {t} RENAME CONSTRAINT {} TO {}", quote(db, old), quote(db, &nn))),
        Db::Mssql => Ok(format!("EXEC sp_rename N'{}', N'{}', 'OBJECT'", lit(&format!("{}.{}", quote(db, owner), quote(db, old))), lit(&nn))),
        _ => Err(format!("{} cannot rename constraint {old}: drop it and add it again", engine(db))),
    }
}

/// Primary key columns: the Primary Key page order, else the PK flags in column order.
fn pk_cols(db: Db, t: &TableDef) -> Vec<String> {
    let flagged: Vec<String> = t.columns.iter().filter(|c| c.pk).map(|c| ident(db, &c.name)).collect();
    let ordered: Vec<String> = t.pk_order.iter().map(|n| ident(db, n)).filter(|n| flagged.contains(n)).collect();
    if ordered.len() == flagged.len() { ordered } else { flagged }
}

/// Default primary key constraint name where the engine wants one (Oracle, SQL Server).
fn pk_clause(db: Db, t: &TableDef, cols: &[String]) -> String {
    let list = cols.iter().map(|c| quote(db, c)).collect::<Vec<_>>().join(", ");
    match db {
        Db::Oracle | Db::Mssql => {
            let n = if t.pk_name.trim().is_empty() { format!("PK_{}", ident(db, &t.name)).chars().take(128).collect() } else { ident(db, &t.pk_name) };
            let off = if db == Db::Oracle && t.pk_disabled { " DISABLE" } else { "" };
            format!("CONSTRAINT {} PRIMARY KEY ({list}){off}", quote(db, &n))
        }
        Db::Postgres | Db::Duckdb if !t.pk_name.trim().is_empty() => format!("CONSTRAINT {} PRIMARY KEY ({list})", q(db, &t.pk_name)),
        _ => format!("PRIMARY KEY ({list})"),
    }
}

fn check_columns(t: &TableDef, db: Db) -> Result<(), String> {
    if ident(db, &t.name).is_empty() {
        return Err("the table name is empty".into());
    }
    if t.columns.is_empty() {
        return Err(format!("Table {} must contain at least one column.", ident(db, &t.name)));
    }
    let mut seen = std::collections::HashSet::new();
    for c in &t.columns {
        let n = ident(db, &c.name);
        if n.is_empty() {
            return Err(format!("Table {}: A column name must be specified.", ident(db, &t.name)));
        }
        if c.ty.trim().is_empty() && !(db == Db::Mssql && !c.virtual_expr.trim().is_empty()) {
            return Err(format!("You must specify a valid datatype for the column {n}"));
        }
        if !seen.insert(n.clone()) {
            return Err(format!("Table {}: The name \"{n}\" is used by multiple columns. Column names must be unique.", ident(db, &t.name)));
        }
    }
    Ok(())
}

/// Table comment after CREATE / in Edit Table (MySQL and ClickHouse on create: table option).
pub(crate) fn table_comment_sql(db: Db, table: &str, text: &str) -> Option<String> {
    let t = lit(text);
    match db {
        Db::Oracle | Db::Postgres | Db::Duckdb => Some(format!("COMMENT ON TABLE {table} IS '{t}'")),
        Db::Mysql => Some(format!("ALTER TABLE {table} COMMENT = '{t}'")),
        Db::Clickhouse => Some(format!("ALTER TABLE {table} MODIFY COMMENT '{t}'")),
        _ => None,
    }
}

/// Oracle storage clause for CREATE TABLE.
fn storage_clause(db: Db, st: &Storage) -> Result<String, String> {
    if *st == Storage::default() {
        return Ok(String::new());
    }
    if db != Db::Oracle {
        return Err("Storage Options are Oracle only".into());
    }
    let mut s = String::new();
    if !st.tablespace.trim().is_empty() {
        s += &format!(" TABLESPACE {}", q(db, &st.tablespace));
    }
    for (kw, v) in [("PCTFREE", &st.pct_free), ("INITRANS", &st.ini_trans)] {
        if !v.trim().is_empty() {
            s += &format!(" {kw} {}", number(kw, v)?);
        }
    }
    for (v, ok) in [(&st.logging, ["LOGGING", "NOLOGGING"]), (&st.compress, ["COMPRESS", "NOCOMPRESS"])] {
        let v = v.trim().to_uppercase();
        if !v.is_empty() {
            if !ok.contains(&v.as_str()) {
                return Err(format!("storage option {v} is not {} or {}", ok[0], ok[1]));
            }
            s += &format!(" {v}");
        }
    }
    Ok(s)
}

pub(crate) fn comment_sql(db: Db, table: &str, col: &str, text: &str) -> Option<String> {
    match db {
        Db::Oracle | Db::Postgres | Db::Duckdb => Some(format!("COMMENT ON COLUMN {table}.{} IS '{}'", quote(db, col), lit(text))),
        Db::Clickhouse => Some(format!("ALTER TABLE {table} COMMENT COLUMN {} '{}'", quote(db, col), lit(text))),
        _ => None,
    }
}

/// CREATE TABLE plus column comments and the new indexes.
pub fn create_table(db: Db, t: &TableDef) -> Result<Vec<String>, String> {
    check_columns(t, db)?;
    // Temporary tables live in the session's own schema on these engines: no owner.
    let unqualified = t.temporary.is_some() && matches!(db, Db::Postgres | Db::Sqlite | Db::Duckdb);
    let tq = if unqualified { q(db, &t.name) } else { table_q(db, t) };
    let mut lines: Vec<String> = t.columns.iter().map(|c| col_def(db, c)).collect::<Result<_, _>>()?;
    let pk = pk_cols(db, t);
    if !pk.is_empty() && db != Db::Clickhouse && !sqlite_autoinc(db, t)? {
        lines.push(pk_clause(db, t, &pk));
    }
    for c in &t.constraints {
        lines.push(con_def(db, c)?);
    }
    let head = match (t.temporary, db) {
        (None, _) => "CREATE TABLE",
        (Some(_), Db::Oracle) => "CREATE GLOBAL TEMPORARY TABLE",
        (Some(_), Db::Postgres | Db::Mysql) => "CREATE TEMPORARY TABLE",
        (Some(_), Db::Sqlite | Db::Duckdb) => "CREATE TEMP TABLE",
        (Some(_), _) => return Err(format!("{} temporary tables are not supported here: write them in a worksheet", engine(db))),
    };
    let mut sql = format!("{head} {tq} (\n  {}\n)", lines.join(",\n  "));
    match (t.temporary, db) {
        (Some(session), Db::Oracle | Db::Postgres) => sql += if session { "\nON COMMIT PRESERVE ROWS" } else { "\nON COMMIT DELETE ROWS" },
        (Some(false), _) => return Err(format!("{} temporary tables are session tables only", engine(db))),
        _ => {}
    }
    sql += &storage_clause(db, &t.storage)?;
    let comment = t.comment.trim();
    if db == Db::Clickhouse {
        let order = if pk.is_empty() { "tuple()".to_string() } else { format!("({})", pk.iter().map(|c| quote(db, c)).collect::<Vec<_>>().join(", ")) };
        sql += &format!("\nENGINE = MergeTree\nORDER BY {order}");
    }
    if matches!(db, Db::Mysql | Db::Clickhouse) && !comment.is_empty() {
        sql += &format!("\nCOMMENT{}'{}'", if db == Db::Mysql { " = " } else { " " }, lit(comment));
    }
    let mut out = vec![sql];
    if matches!(db, Db::Oracle | Db::Postgres | Db::Duckdb) && !comment.is_empty() {
        out.extend(table_comment_sql(db, &tq, comment));
    }
    if matches!(db, Db::Oracle | Db::Postgres | Db::Duckdb) {
        out.extend(t.columns.iter().filter(|c| !c.comment.trim().is_empty()).filter_map(|c| comment_sql(db, &tq, &ident(db, &c.name), c.comment.trim())));
    }
    for i in &t.indexes {
        out.push(create_index(db, &t.owner, &ident(db, &t.name), i)?);
    }
    Ok(out)
}

/// The ALTERs that turn `old` (as loaded) into `new` (as edited): dropped constraints and
/// indexes, primary key, dropped / renamed / changed / added columns, new constraints and
/// indexes, comments.
pub fn alter_table(db: Db, old: &TableDef, new: &TableDef) -> Result<Vec<String>, String> {
    check_columns(new, db)?;
    let t = table_q(db, old);
    let tname = ident(db, &old.name);
    let mut drops = vec![];
    let mut body = vec![];
    let mut tail = vec![];
    let alter = |s: &str| format!("ALTER TABLE {t} {s}");
    let no_constraints = || Err(format!("{} cannot add or drop constraints after CREATE TABLE: recreate the table in a worksheet", engine(db)));

    // Constraints and indexes removed in the dialog.
    for c in old.constraints.iter().filter(|c| !new.constraints.iter().any(|n| n.existing && n.orig == c.orig)) {
        if !alters_constraints(db) {
            return no_constraints();
        }
        let n = quote(db, &c.orig);
        drops.push(alter(&match (db, c.kind) {
            (Db::Mysql, ConKind::Foreign) => format!("DROP FOREIGN KEY {n}"),
            (Db::Mysql, ConKind::Unique) => format!("DROP INDEX {n}"),
            (Db::Mysql, ConKind::Check) => format!("DROP CHECK {n}"),
            _ => format!("DROP CONSTRAINT {n}"),
        }));
    }
    for i in old.indexes.iter().filter(|i| !new.indexes.iter().any(|n| n.existing && n.name == i.name)) {
        drops.push(drop_index(db, &old.owner, &tname, &i.name));
    }

    // Column identity goes through `orig`; name after a rename is the new one.
    let renamed = |o: &str| new.columns.iter().find(|c| c.orig.as_deref() == Some(o)).map(|c| ident(db, &c.name));
    let old_pk: Vec<String> = old.columns.iter().filter(|c| c.pk).filter_map(|c| renamed(c.orig.as_deref().unwrap_or(&c.name))).collect();
    let new_pk = pk_cols(db, new);
    let pk_changed = old_pk != new_pk || old.columns.iter().any(|c| c.pk && renamed(c.orig.as_deref().unwrap_or(&c.name)).is_none());
    if pk_changed && old.columns.iter().any(|c| c.pk) {
        drops.push(match db {
            Db::Oracle | Db::Mysql => alter("DROP PRIMARY KEY"),
            Db::Postgres | Db::Mssql if !old.pk_name.is_empty() => alter(&format!("DROP CONSTRAINT {}", quote(db, &old.pk_name))),
            Db::Postgres | Db::Mssql => return Err("the primary key constraint name is unknown: drop it in a worksheet".into()),
            _ => return Err(format!("{} cannot change the primary key of an existing table", engine(db))),
        });
    }

    // Existing constraints kept: renamed, enabled / disabled.
    for n in new.constraints.iter().filter(|c| c.existing) {
        let Some(o) = old.constraints.iter().find(|o| o.orig == n.orig) else { continue };
        if ident(db, &n.name) != o.orig {
            body.push(rename_constraint(db, &t, &old.owner, &o.orig, &n.name)?);
        }
        if n.enabled != o.enabled {
            body.push(enable_sql(db, &t, n)?);
        }
    }
    let typed = new.pk_name.trim();
    if !pk_changed && !old.pk_name.is_empty() && !typed.is_empty() && typed != old.pk_name && typed != shown(db, &old.pk_name) && ident(db, typed) != old.pk_name {
        body.push(rename_constraint(db, &t, &old.owner, &old.pk_name, &new.pk_name)?);
    }
    // Enabled ticked / unticked on a kept Oracle primary key (a new one carries DISABLE itself).
    if db == Db::Oracle && !pk_changed && !old_pk.is_empty() && new.pk_disabled != old.pk_disabled {
        let n = if typed.is_empty() { old.pk_name.clone() } else { ident(db, typed) };
        body.push(alter(&format!("{} CONSTRAINT {}", if new.pk_disabled { "DISABLE" } else { "ENABLE" }, quote(db, &n))));
    }

    for o in &old.columns {
        let oname = o.orig.clone().unwrap_or_else(|| o.name.clone());
        let Some(n) = new.columns.iter().find(|c| c.orig.as_deref() == Some(oname.as_str())) else {
            body.push(alter(&format!("DROP COLUMN {}", quote(db, &oname))));
            continue;
        };
        let nn = ident(db, &n.name);
        if nn != oname {
            body.push(match db {
                Db::Mssql => {
                    let path = format!("{}.{}.{}", quote(db, &old.owner), quote(db, &tname), quote(db, &oname));
                    format!("EXEC sp_rename N'{}', N'{}', 'COLUMN'", lit(&path), lit(&nn))
                }
                _ => alter(&format!("RENAME COLUMN {} TO {}", quote(db, &oname), quote(db, &nn))),
            });
        }
        let c = quote(db, &nn);
        let ty_changed = norm(&n.full_type()) != norm(&o.full_type());
        let def_changed = n.default.trim() != o.default.trim();
        let null_changed = n.not_null != o.not_null;
        let comment_changed = n.comment.trim() != o.comment.trim();
        if n.virtual_expr.trim() != o.virtual_expr.trim() {
            return Err(format!("column {nn}: a virtual expression cannot be changed in place: drop the column and add it again"));
        }
        if n.identity != o.identity {
            body.push(match (db, &o.identity, &n.identity) {
                (Db::Oracle, Some(_), None) => alter(&format!("MODIFY ({c} DROP IDENTITY)")),
                (Db::Postgres, Some(_), None) => alter(&format!("ALTER COLUMN {c} DROP IDENTITY")),
                (Db::Oracle, Some(_), Some(id)) => alter(&format!("MODIFY ({c} {})", identity_sql(db, n, id)?)),
                (Db::Postgres, None, Some(id)) => alter(&format!("ALTER COLUMN {c} ADD {}", identity_sql(db, n, id)?)),
                _ => return Err(format!("column {nn}: {} cannot change the identity of an existing column here: write it in a worksheet", engine(db))),
            });
        }
        let def_changed = def_changed || n.default_on_null != o.default_on_null;
        if n.default_on_null && db != Db::Oracle {
            return Err(format!("column {nn}: DEFAULT ON NULL is Oracle only"));
        }
        if !(ty_changed || def_changed || null_changed || comment_changed) {
            continue;
        }
        match db {
            Db::Oracle if ty_changed || def_changed || null_changed => {
                let mut m = c.clone();
                if ty_changed {
                    m += &format!(" {}", n.full_type());
                }
                if def_changed {
                    m += &format!(" DEFAULT {}{}", if n.default_on_null { "ON NULL " } else { "" }, if n.default.trim().is_empty() { "NULL" } else { n.default.trim() });
                }
                if null_changed {
                    m += if n.not_null { " NOT NULL" } else { " NULL" };
                }
                body.push(alter(&format!("MODIFY ({m})")));
            }
            Db::Postgres | Db::Duckdb => {
                if ty_changed {
                    body.push(alter(&format!("ALTER COLUMN {c} TYPE {}", n.full_type())));
                }
                if def_changed {
                    body.push(alter(&if n.default.trim().is_empty() { format!("ALTER COLUMN {c} DROP DEFAULT") } else { format!("ALTER COLUMN {c} SET DEFAULT {}", n.default.trim()) }));
                }
                if null_changed {
                    body.push(alter(&format!("ALTER COLUMN {c} {} NOT NULL", if n.not_null { "SET" } else { "DROP" })));
                }
            }
            Db::Mysql => body.push(alter(&format!("MODIFY COLUMN {}", col_def(db, &ColumnDef { name: format!("\"{nn}\""), ..n.clone() })?))),
            Db::Mssql => {
                if ty_changed || null_changed {
                    body.push(alter(&format!("ALTER COLUMN {c} {}{}", n.full_type(), if n.not_null { " NOT NULL" } else { " NULL" })));
                }
                if def_changed {
                    // The old default is a constraint with a generated name: look it up.
                    let tl = lit(&t);
                    body.push(format!(
                        "DECLARE @d sysname; SELECT @d = name FROM sys.default_constraints WHERE parent_object_id = OBJECT_ID(N'{tl}') AND parent_column_id = COLUMNPROPERTY(OBJECT_ID(N'{tl}'), N'{}', 'ColumnId'); IF @d IS NOT NULL EXEC(N'ALTER TABLE {} DROP CONSTRAINT ' + QUOTENAME(@d))",
                        lit(&nn),
                        tl
                    ));
                    if !n.default.trim().is_empty() {
                        body.push(alter(&format!("ADD DEFAULT {} FOR {c}", n.default.trim())));
                    }
                }
            }
            Db::Clickhouse if ty_changed || def_changed || null_changed => {
                let d = if n.default.trim().is_empty() { String::new() } else { format!(" DEFAULT {}", n.default.trim()) };
                body.push(alter(&format!("MODIFY COLUMN {c} {}{d}", n.ch_type())));
            }
            Db::Sqlite if ty_changed || def_changed || null_changed => {
                return Err(format!("SQLite cannot change the type, NOT NULL or default of column {nn}: recreate the table (copy the data) in a worksheet"));
            }
            _ => {}
        }
        if comment_changed && db != Db::Mysql {
            tail.extend(comment_sql(db, &t, &nn, n.comment.trim()));
        }
    }

    for n in new.columns.iter().filter(|c| c.orig.is_none()) {
        let d = col_def(db, n)?;
        body.push(alter(&match db {
            Db::Oracle => format!("ADD ({d})"),
            Db::Mssql => format!("ADD {d}"),
            _ => format!("ADD COLUMN {d}"),
        }));
        if matches!(db, Db::Oracle | Db::Postgres | Db::Duckdb) && !n.comment.trim().is_empty() {
            tail.extend(comment_sql(db, &t, &ident(db, &n.name), n.comment.trim()));
        }
    }

    if pk_changed && !new_pk.is_empty() {
        if !matches!(db, Db::Oracle | Db::Mysql | Db::Postgres | Db::Mssql) {
            return Err(format!("{} cannot change the primary key of an existing table", engine(db)));
        }
        body.push(alter(&format!("ADD {}", pk_clause(db, &TableDef { pk_name: String::new(), ..new.clone() }, &new_pk))));
    }
    for c in new.constraints.iter().filter(|c| !c.existing) {
        if !alters_constraints(db) {
            return no_constraints();
        }
        body.push(alter(&format!("ADD {}", con_def(db, c)?)));
    }
    for i in new.indexes.iter().filter(|i| !i.existing) {
        tail.push(create_index(db, &old.owner, &tname, i)?);
    }
    if new.comment.trim() != old.comment.trim() {
        match table_comment_sql(db, &t, new.comment.trim()) {
            Some(s) => tail.push(s),
            None => return Err(format!("{} table comments are not supported here", engine(db))),
        }
    }
    // Storage: a filled field is a change (Edit Table starts them empty).
    let st = &new.storage;
    if *st != Storage::default() {
        storage_clause(db, st)?;
        for (kw, v) in [("PCTFREE", &st.pct_free), ("INITRANS", &st.ini_trans)] {
            if !v.trim().is_empty() {
                tail.push(alter(&format!("{kw} {}", v.trim())));
            }
        }
        for v in [&st.logging, &st.compress] {
            if !v.trim().is_empty() {
                tail.push(alter(&v.trim().to_uppercase()));
            }
        }
        if !st.tablespace.trim().is_empty() {
            // MOVE leaves the table's indexes UNUSABLE: rebuild the ones kept.
            tail.push(alter(&format!("MOVE TABLESPACE {}", q(db, &st.tablespace))));
            for i in new.indexes.iter().filter(|i| i.existing) {
                tail.extend(rebuild_index(db, &old.owner, &tname, &i.name));
            }
        }
    }
    Ok(drops.into_iter().chain(body).chain(tail).collect())
}

pub(crate) fn engine(db: Db) -> &'static str {
    match db {
        Db::Oracle => "Oracle",
        Db::Postgres => "PostgreSQL",
        Db::Mysql => "MySQL",
        Db::Mssql => "SQL Server",
        Db::Sqlite => "SQLite",
        Db::Duckdb => "DuckDB",
        Db::Clickhouse => "ClickHouse",
    }
}

/// Index name: Oracle and SQLite qualify it with the schema, the others take it bare.
fn index_name(db: Db, owner: &str, name: &str) -> String {
    match db {
        Db::Oracle | Db::Sqlite | Db::Postgres | Db::Duckdb if !owner.is_empty() => format!("{}.{}", quote(db, owner), q(db, name)),
        _ => q(db, name),
    }
}

/// Table in an index statement (SQLite: unqualified, the index name carries the schema).
fn index_table(db: Db, owner: &str, table: &str) -> String {
    if db == Db::Sqlite || owner.is_empty() { quote(db, table) } else { format!("{}.{}", quote(db, owner), quote(db, table)) }
}

pub fn create_index(db: Db, owner: &str, table: &str, i: &IndexDef) -> Result<String, String> {
    if ident(db, &i.name).is_empty() {
        return Err("an index needs a name".into());
    }
    if db == Db::Clickhouse {
        return Err("ClickHouse data skipping indexes need a TYPE: write ALTER TABLE ... ADD INDEX in a worksheet".into());
    }
    // PostgreSQL / DuckDB put the index in the table's schema: the name is bare.
    let name = if matches!(db, Db::Postgres | Db::Duckdb) { q(db, &i.name) } else { index_name(db, owner, &i.name) };
    Ok(format!("CREATE {}INDEX {name} ON {} ({})", if i.unique { "UNIQUE " } else { "" }, index_table(db, owner, table), col_list(db, &i.columns, true)?))
}

pub fn drop_index(db: Db, owner: &str, table: &str, name: &str) -> String {
    let n = quote(db, name);
    match db {
        Db::Mysql | Db::Mssql => format!("DROP INDEX {n} ON {}", index_table(db, owner, table)),
        Db::Clickhouse => format!("ALTER TABLE {} DROP INDEX {n}", index_table(db, owner, table)),
        _ if owner.is_empty() => format!("DROP INDEX {n}"),
        _ => format!("DROP INDEX {}.{n}", quote(db, owner)),
    }
}

/// Rebuild an index, where the engine has a statement for it.
pub fn rebuild_index(db: Db, owner: &str, table: &str, name: &str) -> Option<String> {
    let full = if owner.is_empty() { quote(db, name) } else { format!("{}.{}", quote(db, owner), quote(db, name)) };
    Some(match db {
        Db::Oracle => format!("ALTER INDEX {full} REBUILD"),
        Db::Postgres => format!("REINDEX INDEX {full}"),
        Db::Mssql => format!("ALTER INDEX {} ON {} REBUILD", quote(db, name), index_table(db, owner, table)),
        Db::Sqlite => format!("REINDEX {full}"),
        _ => return None,
    })
}

/// Queries that load a table for Edit Table. `constraints` rows: name, kind label
/// (`PRIMARY KEY`, `UNIQUE`, `FOREIGN KEY`, `CHECK`), description. `indexes` rows: name,
/// `Y` when unique, description; indexes behind constraints are left out.
pub struct LoadSql {
    pub columns: String,
    pub constraints: Option<String>,
    pub indexes: Option<String>,
    /// Rows for [`apply_extra`]: column, identity kind, virtual Y, default on null Y,
    /// virtual expression, Oracle identity options.
    pub extra: Option<String>,
    /// One cell: the table comment.
    pub comment: Option<String>,
}

pub fn load_sql(db: Db, owner: &str, name: &str) -> Option<LoadSql> {
    use crate::dialect::{children_sql, fill, TABLE};
    let columns = fill(db, &children_sql(db, TABLE)?, owner, name);
    let (cons, idx): (Option<&str>, &str) = match db {
        Db::Oracle => (
            Some("SELECT c.constraint_name, CASE c.constraint_type WHEN 'P' THEN 'PRIMARY KEY' WHEN 'U' THEN 'UNIQUE' WHEN 'R' THEN 'FOREIGN KEY' ELSE 'CHECK' END, (SELECT LISTAGG(cc.column_name, ', ') WITHIN GROUP (ORDER BY cc.position) FROM all_cons_columns cc WHERE cc.owner = c.owner AND cc.constraint_name = c.constraint_name) || CASE WHEN c.constraint_type = 'R' THEN ' → ' || r.owner || '.' || r.table_name WHEN c.constraint_type = 'C' THEN ': ' || c.search_condition_vc END, c.status FROM all_constraints c LEFT JOIN all_constraints r ON r.owner = c.r_owner AND r.constraint_name = c.r_constraint_name WHERE c.owner = '{o}' AND c.table_name = '{n}' AND c.constraint_type IN ('P', 'U', 'R', 'C') AND NOT (c.constraint_type = 'C' AND c.generated = 'GENERATED NAME' AND c.search_condition_vc LIKE '% IS NOT NULL') ORDER BY 2, 1"),
            "SELECT i.index_name, CASE i.uniqueness WHEN 'UNIQUE' THEN 'Y' END, (SELECT LISTAGG(ic.column_name || CASE ic.descend WHEN 'DESC' THEN ' DESC' END, ', ') WITHIN GROUP (ORDER BY ic.column_position) FROM all_ind_columns ic WHERE ic.index_owner = i.owner AND ic.index_name = i.index_name) FROM all_indexes i WHERE i.table_owner = '{o}' AND i.table_name = '{n}' AND i.index_type <> 'LOB' AND NOT EXISTS (SELECT 1 FROM all_constraints k WHERE k.owner = i.table_owner AND k.table_name = i.table_name AND k.index_name = i.index_name AND k.constraint_type IN ('P', 'U')) ORDER BY 1",
        ),
        Db::Postgres => (
            Some("SELECT conname, CASE contype WHEN 'p' THEN 'PRIMARY KEY' WHEN 'u' THEN 'UNIQUE' WHEN 'f' THEN 'FOREIGN KEY' ELSE 'CHECK' END, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = '{lq}'::regclass AND contype IN ('p', 'u', 'f', 'c') ORDER BY 2, 1"),
            "SELECT c.relname, CASE WHEN x.indisunique THEN 'Y' END, pg_get_indexdef(x.indexrelid) FROM pg_index x JOIN pg_class c ON c.oid = x.indexrelid WHERE x.indrelid = '{lq}'::regclass AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = x.indexrelid) ORDER BY 1",
        ),
        Db::Mysql => (
            Some("SELECT tc.constraint_name, tc.constraint_type, CONCAT(COALESCE(GROUP_CONCAT(k.column_name ORDER BY k.ordinal_position), ''), COALESCE(CONCAT(' → ', MAX(k.referenced_table_name)), '')) FROM information_schema.table_constraints tc LEFT JOIN information_schema.key_column_usage k ON k.constraint_schema = tc.constraint_schema AND k.table_name = tc.table_name AND k.constraint_name = tc.constraint_name WHERE tc.table_schema = '{o}' AND tc.table_name = '{n}' GROUP BY tc.constraint_name, tc.constraint_type ORDER BY 2, 1"),
            "SELECT index_name, CASE MIN(non_unique) WHEN 0 THEN 'Y' END, GROUP_CONCAT(column_name ORDER BY seq_in_index) FROM information_schema.statistics WHERE table_schema = '{o}' AND table_name = '{n}' AND index_name <> 'PRIMARY' AND index_name NOT IN (SELECT constraint_name FROM information_schema.table_constraints WHERE table_schema = '{o}' AND table_name = '{n}') GROUP BY index_name ORDER BY 1",
        ),
        Db::Mssql => (
            Some("SELECT o.name, CASE o.type WHEN 'PK' THEN 'PRIMARY KEY' WHEN 'UQ' THEN 'UNIQUE' WHEN 'F' THEN 'FOREIGN KEY' ELSE 'CHECK' END, o.type_desc, CASE WHEN OBJECTPROPERTY(o.object_id, 'CnstIsDisabled') = 1 THEN 'DISABLED' ELSE 'ENABLED' END FROM sys.objects o WHERE o.parent_object_id = OBJECT_ID('{lq}') AND o.type IN ('PK', 'UQ', 'F', 'C') ORDER BY 2, 1"),
            "SELECT i.name, CASE WHEN i.is_unique = 1 THEN 'Y' END, STRING_AGG(c.name, ', ') WITHIN GROUP (ORDER BY ic.key_ordinal) FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id WHERE i.object_id = OBJECT_ID('{lq}') AND i.is_primary_key = 0 AND i.is_unique_constraint = 0 AND i.type > 0 GROUP BY i.name, i.is_unique ORDER BY 1",
        ),
        Db::Sqlite => (None, "SELECT name, CASE WHEN \"unique\" THEN 'Y' END, origin FROM pragma_index_list('{n}', '{o}') WHERE origin = 'c' ORDER BY 1"),
        Db::Duckdb => (
            Some("SELECT constraint_text, constraint_type, constraint_text FROM duckdb_constraints() WHERE schema_name = '{o}' AND table_name = '{n}' AND constraint_type IN ('PRIMARY KEY', 'UNIQUE', 'FOREIGN KEY', 'CHECK')"),
            "SELECT index_name, CASE WHEN is_unique THEN 'Y' END, sql FROM duckdb_indexes() WHERE schema_name = '{o}' AND table_name = '{n}' ORDER BY 1",
        ),
        Db::Clickhouse => (None, "SELECT name, '', expr FROM system.data_skipping_indices WHERE database = '{o}' AND table = '{n}' ORDER BY name"),
    };
    let extra = match db {
        Db::Oracle => Some("SELECT c.column_name, i.generation_type, CASE c.virtual_column WHEN 'YES' THEN 'Y' END, CASE c.default_on_null WHEN 'YES' THEN 'Y' END, NULL, i.identity_options FROM all_tab_cols c LEFT JOIN all_tab_identity_cols i ON i.owner = c.owner AND i.table_name = c.table_name AND i.column_name = c.column_name WHERE c.owner = '{o}' AND c.table_name = '{n}' AND c.hidden_column = 'NO'"),
        Db::Postgres => Some("SELECT a.attname, CASE a.attidentity WHEN 'a' THEN 'ALWAYS' WHEN 'd' THEN 'BY DEFAULT' END, CASE WHEN a.attgenerated = 's' THEN 'Y' END, NULL, CASE WHEN a.attgenerated = 's' THEN pg_get_expr(d.adbin, d.adrelid) END, NULL FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum WHERE a.attrelid = '{lq}'::regclass AND a.attnum > 0 AND NOT a.attisdropped"),
        Db::Mysql => Some("SELECT column_name, CASE WHEN extra LIKE '%auto_increment%' THEN 'BY DEFAULT' END, CASE WHEN extra LIKE '%GENERATED%' THEN 'Y' END, NULL, NULLIF(generation_expression, ''), NULL FROM information_schema.columns WHERE table_schema = '{o}' AND table_name = '{n}'"),
        Db::Mssql => Some("SELECT c.name, CASE WHEN c.is_identity = 1 THEN 'BY DEFAULT' END, CASE WHEN c.is_computed = 1 THEN 'Y' END, NULL, cc.definition, CASE WHEN c.is_identity = 1 THEN CONCAT('START WITH: ', CAST(ic.seed_value AS varchar(40)), ', INCREMENT BY: ', CAST(ic.increment_value AS varchar(40))) END FROM sys.columns c LEFT JOIN sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id LEFT JOIN sys.identity_columns ic ON ic.object_id = c.object_id AND ic.column_id = c.column_id WHERE c.object_id = OBJECT_ID('{lq}')"),
        _ => None,
    };
    let comment = match db {
        Db::Oracle => Some("SELECT comments FROM all_tab_comments WHERE owner = '{o}' AND table_name = '{n}'"),
        Db::Postgres => Some("SELECT obj_description('{lq}'::regclass, 'pg_class')"),
        Db::Mysql => Some("SELECT table_comment FROM information_schema.tables WHERE table_schema = '{o}' AND table_name = '{n}'"),
        Db::Duckdb => Some("SELECT comment FROM duckdb_tables() WHERE schema_name = '{o}' AND table_name = '{n}'"),
        Db::Clickhouse => Some("SELECT comment FROM system.tables WHERE database = '{o}' AND name = '{n}'"),
        _ => None,
    };
    let f = |s: &str| fill(db, s, owner, name);
    Some(LoadSql { columns, constraints: cons.map(f), indexes: Some(f(idx)), extra: extra.map(f), comment: comment.map(f) })
}

type Row = Vec<Option<String>>;

/// Build the loaded definition from [`load_sql`] results (the Columns tab rows: name, type,
/// nullable Yes/No, default, id, primary key Y, comment; DuckDB and ClickHouse differ).
pub fn from_rows(db: Db, owner: &str, name: &str, cols: &[Row], cons: &[Row], idx: &[Row]) -> TableDef {
    let get = |r: &Row, i: usize| r.get(i).cloned().flatten().unwrap_or_default();
    // DuckDB: no primary key column (it comes from the constraints), comment in column 5.
    let (pk_at, comment_at) = match db {
        Db::Duckdb => (None, 5),
        _ => (Some(5), 6),
    };
    let mut t = TableDef { owner: owner.into(), name: shown(db, name), ..Default::default() };
    let mut pk_from_cons: Vec<String> = vec![];
    for r in cons {
        let (cname, kind, info) = (get(r, 0), get(r, 1), get(r, 2));
        if kind == "PRIMARY KEY" {
            t.pk_name = cname;
            t.pk_disabled = get(r, 3) == "DISABLED";
            if db == Db::Duckdb {
                // "PRIMARY KEY(id, b)"
                let inner = info.split_once('(').map_or("", |x| x.1).trim_end_matches(')');
                pk_from_cons = inner.split(',').map(|s| s.trim().trim_matches('"').to_string()).collect();
            }
            continue;
        }
        if let Some(k) = ConKind::parse(&kind) {
            let enabled = get(r, 3) != "DISABLED";
            t.constraints.push(ConstraintDef { existing: true, orig: cname.clone(), name: shown(db, &cname), kind: k, enabled, info, ..Default::default() });
        }
    }
    for r in cols {
        let stored = get(r, 0);
        let mut c = ColumnDef { orig: Some(stored.clone()), name: shown(db, &stored), ..Default::default() };
        let mut ty = get(r, 1);
        if db == Db::Clickhouse {
            match ty.strip_prefix("Nullable(").and_then(|s| s.strip_suffix(')')) {
                Some(inner) => ty = inner.to_string(),
                None => c.not_null = true,
            }
        } else {
            c.not_null = get(r, 2).eq_ignore_ascii_case("No");
        }
        c.set_type(&ty);
        c.default = get(r, 3).trim().to_string();
        c.pk = match pk_at {
            Some(i) => get(r, i) == "Y",
            None => pk_from_cons.iter().any(|p| *p == stored),
        };
        c.comment = get(r, comment_at);
        t.columns.push(c);
    }
    t.indexes = idx.iter().map(|r| IndexDef { existing: true, name: get(r, 0), columns: String::new(), unique: get(r, 1) == "Y", info: get(r, 2) }).collect();
    t
}

/// Identity / virtual / default-on-null facts from [`LoadSql::extra`] rows. The default the
/// engine reports for those columns (sequence call, expression) is not a plain default.
pub fn apply_extra(t: &mut TableDef, extra: &[Row]) {
    let get = |r: &Row, i: usize| r.get(i).cloned().flatten().unwrap_or_default();
    for r in extra {
        let Some(c) = t.columns.iter_mut().find(|c| c.orig.as_deref() == Some(get(r, 0).as_str())) else { continue };
        let kind = get(r, 1);
        if !kind.is_empty() {
            c.identity = Some(Identity::parse(&kind, &get(r, 5)));
            c.default.clear();
        }
        if get(r, 2) == "Y" {
            let e = get(r, 4);
            c.virtual_expr = if e.is_empty() { std::mem::take(&mut c.default) } else { e };
            c.default.clear();
        }
        c.default_on_null = get(r, 3) == "Y";
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, size: &str, pk: bool) -> ColumnDef {
        ColumnDef { name: name.into(), ty: ty.into(), size: size.into(), pk, not_null: pk, ..Default::default() }
    }

    #[test]
    fn create_oracle() {
        let mut t = TableDef { owner: "HR".into(), name: "emp".into(), ..Default::default() };
        t.columns = vec![col("id", "NUMBER", "", true), ColumnDef { comment: "full name".into(), default: "'x'".into(), ..col("name", "VARCHAR2", "100", false) }];
        t.constraints.push(ConstraintDef { name: "emp_dept_fk".into(), kind: ConKind::Foreign, columns: "dept_id".into(), ref_table: "hr.dept".into(), ref_columns: "id".into(), on_delete: "cascade".into(), ..Default::default() });
        t.indexes.push(IndexDef { name: "emp_name_ix".into(), columns: "name desc".into(), ..Default::default() });
        let s = create_table(Db::Oracle, &t).unwrap();
        assert_eq!(
            s[0],
            "CREATE TABLE \"HR\".\"EMP\" (\n  \"ID\" NUMBER NOT NULL,\n  \"NAME\" VARCHAR2(100) DEFAULT 'x',\n  CONSTRAINT \"PK_EMP\" PRIMARY KEY (\"ID\"),\n  CONSTRAINT \"EMP_DEPT_FK\" FOREIGN KEY (\"DEPT_ID\") REFERENCES \"HR\".\"DEPT\" (\"ID\") ON DELETE CASCADE\n)"
        );
        assert_eq!(s[1], "COMMENT ON COLUMN \"HR\".\"EMP\".\"NAME\" IS 'full name'");
        assert_eq!(s[2], "CREATE INDEX \"HR\".\"EMP_NAME_IX\" ON \"HR\".\"EMP\" (\"NAME\" DESC)");
        assert!(create_table(Db::Oracle, &TableDef { name: "x".into(), ..Default::default() }).is_err());
    }

    #[test]
    fn create_clickhouse_and_mysql() {
        let t = TableDef { owner: "db".into(), name: "t".into(), columns: vec![col("id", "UInt64", "", true), col("s", "String", "", false)], ..Default::default() };
        assert_eq!(create_table(Db::Clickhouse, &t).unwrap()[0], "CREATE TABLE `db`.`t` (\n  `id` UInt64,\n  `s` Nullable(String)\n)\nENGINE = MergeTree\nORDER BY (`id`)");
        assert_eq!(create_table(Db::Mysql, &t).unwrap()[0], "CREATE TABLE `db`.`t` (\n  `id` UInt64 NOT NULL,\n  `s` String,\n  PRIMARY KEY (`id`)\n)");
    }

    fn loaded(db: Db) -> TableDef {
        let r = |v: &[&str]| v.iter().map(|x| (!x.is_empty()).then(|| x.to_string())).collect::<Vec<_>>();
        from_rows(
            db,
            "HR",
            "EMP",
            &[r(&["ID", "NUMBER(10,0)", "No", "", "1", "Y", ""]), r(&["NAME", "VARCHAR2(50 BYTE)", "Yes", "", "2", "", ""]), r(&["OLD", "DATE", "Yes", "", "3", "", ""])],
            &[r(&["PK_EMP", "PRIMARY KEY", "ID"]), r(&["EMP_U", "UNIQUE", "NAME"])],
            &[r(&["EMP_IX", "", "NAME"])],
        )
    }

    #[test]
    fn load_round_trip_has_no_changes() {
        let t = loaded(Db::Oracle);
        assert_eq!(t.columns[0].ty, "NUMBER");
        assert_eq!((t.columns[0].size.as_str(), t.columns[0].scale.as_str()), ("10", "0"));
        assert_eq!(t.columns[1].size, "50 BYTE");
        assert!(t.columns[0].pk && t.columns[0].not_null);
        assert_eq!(t.pk_name, "PK_EMP");
        assert_eq!(alter_table(Db::Oracle, &t, &t.clone()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn alter_oracle() {
        let old = loaded(Db::Oracle);
        let mut new = old.clone();
        new.columns[1].name = "full_name".into();
        new.columns[1].size = "100 BYTE".into();
        new.columns[1].not_null = true;
        new.columns.remove(2);
        new.columns.push(col("hired", "DATE", "", false));
        new.constraints.clear();
        new.indexes.clear();
        new.indexes.push(IndexDef { name: "emp_hired_ix".into(), columns: "hired".into(), ..Default::default() });
        assert_eq!(
            alter_table(Db::Oracle, &old, &new).unwrap(),
            vec![
                "ALTER TABLE \"HR\".\"EMP\" DROP CONSTRAINT \"EMP_U\"",
                "DROP INDEX \"HR\".\"EMP_IX\"",
                "ALTER TABLE \"HR\".\"EMP\" RENAME COLUMN \"NAME\" TO \"FULL_NAME\"",
                "ALTER TABLE \"HR\".\"EMP\" MODIFY (\"FULL_NAME\" VARCHAR2(100 BYTE) NOT NULL)",
                "ALTER TABLE \"HR\".\"EMP\" DROP COLUMN \"OLD\"",
                "ALTER TABLE \"HR\".\"EMP\" ADD (\"HIRED\" DATE)",
                "CREATE INDEX \"HR\".\"EMP_HIRED_IX\" ON \"HR\".\"EMP\" (\"HIRED\")",
            ]
        );
    }

    #[test]
    fn alter_postgres_and_sqlite() {
        let mut old = loaded(Db::Postgres);
        old.owner = "public".into();
        let mut new = old.clone();
        new.columns[1].default = "'n/a'".into();
        new.columns[1].ty = "varchar".into();
        let s = alter_table(Db::Postgres, &old, &new).unwrap();
        assert_eq!(s, vec!["ALTER TABLE \"public\".\"EMP\" ALTER COLUMN \"NAME\" TYPE varchar(50 BYTE)", "ALTER TABLE \"public\".\"EMP\" ALTER COLUMN \"NAME\" SET DEFAULT 'n/a'"]);
        let old = TableDef { owner: "main".into(), name: "t".into(), columns: vec![ColumnDef { orig: Some("a".into()), ..col("a", "TEXT", "", false) }], ..Default::default() };
        let mut new = old.clone();
        new.columns[0].not_null = true;
        assert!(alter_table(Db::Sqlite, &old, &new).unwrap_err().contains("SQLite cannot change"));
    }

    #[test]
    fn identity_virtual_temp_storage() {
        let id = Identity::parse("ALWAYS", "START WITH: 100, INCREMENT BY: 1, MAX_VALUE: 9999999999999999999999999999, MIN_VALUE: 1, CYCLE_FLAG: N, CACHE_SIZE: 0, ORDER_FLAG: Y");
        assert_eq!(id, Identity { generate: "ALWAYS".into(), start: "100".into(), cache: "NOCACHE".into(), order: true, ..Default::default() });
        let mut t = TableDef { owner: "HR".into(), name: "t".into(), comment: "it's".into(), temporary: Some(false), ..Default::default() };
        t.columns = vec![
            ColumnDef { identity: Some(id), ..col("id", "NUMBER", "", true) },
            ColumnDef { virtual_expr: "id * 2".into(), ..col("dbl", "NUMBER", "", false) },
            ColumnDef { default: "0".into(), default_on_null: true, ..col("n", "NUMBER", "", false) },
        ];
        t.storage = Storage { tablespace: "users".into(), pct_free: "5".into(), logging: "nologging".into(), ..Default::default() };
        let s = create_table(Db::Oracle, &t).unwrap();
        assert_eq!(
            s[0],
            "CREATE GLOBAL TEMPORARY TABLE \"HR\".\"T\" (\n  \"ID\" NUMBER GENERATED ALWAYS AS IDENTITY (START WITH 100 NOCACHE ORDER) NOT NULL,\n  \"DBL\" NUMBER GENERATED ALWAYS AS (id * 2) VIRTUAL,\n  \"N\" NUMBER DEFAULT ON NULL 0,\n  CONSTRAINT \"PK_T\" PRIMARY KEY (\"ID\")\n)\nON COMMIT DELETE ROWS TABLESPACE \"USERS\" PCTFREE 5 NOLOGGING"
        );
        assert_eq!(s[1], "COMMENT ON TABLE \"HR\".\"T\" IS 'it''s'");
        // PostgreSQL: temp tables unqualified, STORED generated columns, no ON NULL default.
        t.storage = Storage::default();
        t.columns[0].identity = Some(Identity { generate: "BY DEFAULT".into(), increment: "5".into(), ..Default::default() });
        t.columns[2].default_on_null = false;
        let s = create_table(Db::Postgres, &t).unwrap();
        assert!(s[0].starts_with("CREATE TEMPORARY TABLE \"t\" (\n  \"id\" NUMBER GENERATED BY DEFAULT AS IDENTITY (INCREMENT BY 5) NOT NULL,\n  \"dbl\" NUMBER GENERATED ALWAYS AS (id * 2) STORED"), "{}", s[0]);
        assert!(s[0].ends_with("ON COMMIT DELETE ROWS"));
        // SQL Server identity, SQLite AUTOINCREMENT on the single INTEGER key.
        t.temporary = None;
        let mut ms = t.clone();
        ms.columns.truncate(1);
        assert!(create_table(Db::Mssql, &ms).unwrap()[0].contains("[id] NUMBER IDENTITY(1, 5) NOT NULL"));
        ms.columns[0].ty = "INTEGER".into();
        assert_eq!(create_table(Db::Sqlite, &ms).unwrap()[0], "CREATE TABLE \"HR\".\"t\" (\n  \"id\" INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL\n)");
        assert!(create_table(Db::Duckdb, &ms).unwrap_err().contains("DuckDB has no"));
        ms.columns[0].identity.as_mut().unwrap().start = "1; drop".into();
        assert!(create_table(Db::Oracle, &ms).unwrap_err().contains("whole number"));
    }

    #[test]
    fn primary_key_enabled_flag() {
        let r = |v: &[&str]| v.iter().map(|x| (!x.is_empty()).then(|| x.to_string())).collect::<Vec<_>>();
        let old = from_rows(Db::Oracle, "HR", "T", &[r(&["ID", "NUMBER", "No", "", "1", "Y", ""])], &[r(&["PK_T", "PRIMARY KEY", "ID", "ENABLED"])], &[]);
        assert!(!old.pk_disabled);
        let mut new = old.clone();
        new.pk_disabled = true;
        assert_eq!(alter_table(Db::Oracle, &old, &new).unwrap(), ["ALTER TABLE \"HR\".\"T\" DISABLE CONSTRAINT \"PK_T\""]);
        assert!(create_table(Db::Oracle, &new).unwrap()[0].contains("CONSTRAINT \"PK_T\" PRIMARY KEY (\"ID\") DISABLE"));
        assert!(!create_table(Db::Postgres, &new).unwrap()[0].contains("DISABLE"));
    }

    #[test]
    fn alter_constraints_identity_comment() {
        let r = |v: &[&str]| v.iter().map(|x| (!x.is_empty()).then(|| x.to_string())).collect::<Vec<_>>();
        let mut old = from_rows(Db::Oracle, "HR", "EMP", &[r(&["ID", "NUMBER", "No", "\"HR\".\"ISEQ$$_1\".nextval", "1", "Y", ""])], &[r(&["PK_EMP", "PRIMARY KEY", "ID", "ENABLED"]), r(&["EMP_C", "CHECK", "ID > 0", "DISABLED"])], &[]);
        apply_extra(&mut old, &[r(&["ID", "BY DEFAULT", "", "", "", "START WITH: 1, INCREMENT BY: 1"])]);
        assert_eq!(old.columns[0].default, "");
        assert!(old.columns[0].identity.is_some() && !old.constraints[0].enabled);
        let mut new = old.clone();
        new.constraints[0].enabled = true;
        new.constraints[0].name = "emp_pos".into();
        new.pk_name = "emp_pk".into();
        new.columns[0].identity = None;
        new.comment = "staff".into();
        assert_eq!(
            alter_table(Db::Oracle, &old, &new).unwrap(),
            vec![
                "ALTER TABLE \"HR\".\"EMP\" RENAME CONSTRAINT \"EMP_C\" TO \"EMP_POS\"",
                "ALTER TABLE \"HR\".\"EMP\" ENABLE CONSTRAINT \"EMP_POS\"",
                "ALTER TABLE \"HR\".\"EMP\" RENAME CONSTRAINT \"PK_EMP\" TO \"EMP_PK\"",
                "ALTER TABLE \"HR\".\"EMP\" MODIFY (\"ID\" DROP IDENTITY)",
                "COMMENT ON TABLE \"HR\".\"EMP\" IS 'staff'",
            ]
        );
        assert_eq!(alter_table(Db::Oracle, &old, &old.clone()).unwrap(), Vec::<String>::new());
        let mut st = old.clone();
        st.storage.tablespace = "data2".into();
        assert_eq!(alter_table(Db::Oracle, &old, &st).unwrap(), vec!["ALTER TABLE \"HR\".\"EMP\" MOVE TABLESPACE \"DATA2\""]);
    }

    #[test]
    fn names_and_indexes() {
        assert_eq!(ident(Db::Oracle, "emp"), "EMP");
        assert_eq!(ident(Db::Oracle, "\"Emp\""), "Emp");
        assert_eq!(ident(Db::Postgres, "Emp"), "emp");
        assert_eq!(shown(Db::Oracle, "Emp"), "\"Emp\"");
        assert_eq!(shown(Db::Oracle, "EMP"), "EMP");
        let i = IndexDef { name: "ix".into(), columns: "a, b".into(), unique: true, ..Default::default() };
        assert_eq!(create_index(Db::Postgres, "s", "t", &i).unwrap(), "CREATE UNIQUE INDEX \"ix\" ON \"s\".\"t\" (\"a\", \"b\")");
        assert_eq!(create_index(Db::Sqlite, "main", "t", &i).unwrap(), "CREATE UNIQUE INDEX \"main\".\"ix\" ON \"t\" (\"a\", \"b\")");
        assert_eq!(drop_index(Db::Mysql, "db", "t", "ix"), "DROP INDEX `ix` ON `db`.`t`");
        assert_eq!(rebuild_index(Db::Oracle, "HR", "EMP", "IX").unwrap(), "ALTER INDEX \"HR\".\"IX\" REBUILD");
    }
}
