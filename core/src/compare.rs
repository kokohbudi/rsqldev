//! Schema Compare: diff two schemas (object lists, table columns, code) and write the script
//! that brings the target in line. The script is only shown, never run by itself.

use crate::dialect::{self, FUNCTION, MVIEW, PACKAGE, PROCEDURE, TABLE, TRIGGER, TYPE, VIEW};
use crate::registry::Kind as Db;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Obj {
    /// (name, data type, nullable) per column: tables and views.
    pub columns: Vec<(String, String, String)>,
    /// Source text: code objects. Empty = not compared (presence only).
    pub code: String,
}

/// Objects keyed by (kind, name).
pub type Schema = BTreeMap<(&'static str, String), Obj>;

#[derive(Debug, PartialEq)]
pub enum Diff {
    OnlySource,
    OnlyTarget,
    /// Human-readable differences.
    Changed(Vec<String>),
}

pub fn is_code(kind: &str) -> bool {
    matches!(kind, PACKAGE | PROCEDURE | FUNCTION | TRIGGER | TYPE)
}

pub fn has_columns(kind: &str) -> bool {
    matches!(kind, TABLE | VIEW | MVIEW)
}

/// Line ends trimmed, blank edges dropped: whitespace-only edits do not count as changes.
fn norm(code: &str) -> String {
    code.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

pub fn diff(src: &Schema, tgt: &Schema) -> Vec<((&'static str, String), Diff)> {
    let mut out = vec![];
    for (k, s) in src {
        let Some(t) = tgt.get(k) else {
            out.push((k.clone(), Diff::OnlySource));
            continue;
        };
        let mut notes = vec![];
        let tcols: BTreeMap<&str, (&str, &str)> = t.columns.iter().map(|(n, ty, nl)| (n.as_str(), (ty.as_str(), nl.as_str()))).collect();
        for (n, ty, nl) in &s.columns {
            match tcols.get(n.as_str()) {
                None => notes.push(format!("column {n} missing in target ({ty})")),
                Some((tty, tnl)) if (*tty, *tnl) != (ty.as_str(), nl.as_str()) => notes.push(format!("column {n}: {ty} nullable {nl} → target {tty} nullable {tnl}")),
                _ => {}
            }
        }
        for (n, _, _) in &t.columns {
            if !s.columns.iter().any(|c| c.0 == *n) {
                notes.push(format!("column {n} only in target"));
            }
        }
        if norm(&s.code) != norm(&t.code) {
            notes.push("source code differs".into());
        }
        if !notes.is_empty() {
            out.push((k.clone(), Diff::Changed(notes)));
        }
    }
    out.extend(tgt.keys().filter(|k| !src.contains_key(*k)).map(|k| (k.clone(), Diff::OnlyTarget)));
    out
}

fn add_column(db: Db, table: &str, col: &str, ty: &str) -> String {
    match db {
        Db::Oracle => format!("ALTER TABLE {table} ADD ({col} {ty});"),
        Db::Mssql => format!("ALTER TABLE {table} ADD {col} {ty};"),
        _ => format!("ALTER TABLE {table} ADD COLUMN {col} {ty};"),
    }
}

/// Oracle: DBMS_METADATA writes the terminators itself (`;` after SQL, `/` after each PL/SQL
/// unit) while this is on, so package spec + body or trigger + ALTER TRIGGER come out runnable
/// without splitting the text. Run before fetching DDL, [`ORA_TERMINATORS_OFF`] after.
pub const ORA_TERMINATORS_ON: &str = "BEGIN DBMS_METADATA.SET_TRANSFORM_PARAM(DBMS_METADATA.SESSION_TRANSFORM, 'SQLTERMINATOR', TRUE); END;";
pub const ORA_TERMINATORS_OFF: &str = "BEGIN DBMS_METADATA.SET_TRANSFORM_PARAM(DBMS_METADATA.SESSION_TRANSFORM, 'SQLTERMINATOR', FALSE); END;";

/// DDL fetched from the source, pointed at the target schema and terminated for Run Script.
fn statement(db: Db, ddl: &str, src_owner: &str, tgt_owner: &str) -> String {
    let mut ddl = ddl.trim().to_string();
    if !src_owner.is_empty() && !tgt_owner.is_empty() && src_owner != tgt_owner {
        ddl = ddl.replace(&format!("{}.", dialect::quote(db, src_owner)), &format!("{}.", dialect::quote(db, tgt_owner)));
    }
    if ddl.ends_with(';') || (db == Db::Oracle && ddl.ends_with('/')) { ddl } else { format!("{ddl};") }
}

/// Identifiers and notes go into `--` comments: a newline in a (database-supplied) name must
/// not end the comment and leave the rest of the name as a statement in the script.
fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_control() || c == '\u{2028}' || c == '\u{2029}' { '?' } else { c }).collect()
}

pub struct Side<'a> {
    pub label: &'a str,
    pub owner: &'a str,
    pub schema: &'a Schema,
}

/// The compare report and sync script. `ddl` holds source DDL for objects to create or replace.
pub fn script(db: Db, src: &Side, tgt: &Side, diffs: &[((&'static str, String), Diff)], ddl: &BTreeMap<(&'static str, String), String>) -> String {
    let count = |f: fn(&Diff) -> bool| diffs.iter().filter(|d| f(&d.1)).count();
    let mut out = format!(
        "-- Schema Compare: {} → {}\n-- {} only in source, {} different, {} only in target. Review before running (F5).\n",
        safe(src.label),
        safe(tgt.label),
        count(|d| *d == Diff::OnlySource),
        count(|d| matches!(d, Diff::Changed(_))),
        count(|d| *d == Diff::OnlyTarget),
    );
    let q = |name: &str| dialect::qualified(db, tgt.owner, name);
    let section = |out: &mut String, title: &str| out.push_str(&format!("\n-- ===== {title} =====\n"));
    section(&mut out, "Missing in target");
    for ((kind, name), d) in diffs {
        if *d == Diff::OnlySource {
            let n = safe(name);
            match ddl.get(&(*kind, name.clone())) {
                Some(t) => out.push_str(&format!("\n-- {kind} {n}\n{}\n", statement(db, t, src.owner, tgt.owner))),
                None => out.push_str(&format!("\n-- {kind} {n}: no DDL available, create it by hand\n")),
            }
        }
    }
    section(&mut out, "Different");
    for ((kind, name), d) in diffs {
        let Diff::Changed(notes) = d else { continue };
        out.push_str(&format!("\n-- {kind} {}\n", safe(name)));
        for n in notes {
            out.push_str(&format!("--   {}\n", safe(n)));
        }
        if *kind == TABLE {
            let tcols = &tgt.schema.get(&(*kind, name.clone())).map(|o| o.columns.clone()).unwrap_or_default();
            for (c, ty, _) in src.schema.get(&(*kind, name.clone())).map(|o| o.columns.as_slice()).unwrap_or_default() {
                if !tcols.iter().any(|t| t.0 == *c) {
                    out.push_str(&add_column(db, &q(name), &dialect::quote(db, c), ty));
                    out.push('\n');
                }
            }
        } else if let Some(t) = ddl.get(&(*kind, name.clone())) {
            out.push_str(&statement(db, t, src.owner, tgt.owner));
            out.push('\n');
        }
    }
    section(&mut out, "Only in target (commented out)");
    for ((kind, name), d) in diffs {
        if *d == Diff::OnlyTarget {
            out.push_str(&format!("-- DROP {kind} {};\n", safe(&q(name))));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(cols: &[(&str, &str)]) -> Obj {
        Obj { columns: cols.iter().map(|(n, t)| (n.to_string(), t.to_string(), "Yes".into())).collect(), code: String::new() }
    }

    #[test]
    fn diff_and_script() {
        let mut s = Schema::new();
        let mut t = Schema::new();
        s.insert((TABLE, "EMP".into()), table(&[("ID", "NUMBER"), ("NAME", "VARCHAR2(50 BYTE)")]));
        t.insert((TABLE, "EMP".into()), table(&[("ID", "NUMBER(5,0)")]));
        s.insert((PACKAGE, "P".into()), Obj { code: "package p as\n  x number;  \nend;".into(), ..Default::default() });
        t.insert((PACKAGE, "P".into()), Obj { code: "package p as\n  x number;\nend;\n".into(), ..Default::default() });
        s.insert((VIEW, "V".into()), Obj::default());
        t.insert((TABLE, "OLD".into()), Obj::default());
        let d = diff(&s, &t);
        assert_eq!(d.len(), 3, "{d:?}");
        let Diff::Changed(n) = &d.iter().find(|x| x.0.1 == "EMP").unwrap().1 else { panic!() };
        assert_eq!(n.len(), 2);
        let mut ddl = BTreeMap::new();
        ddl.insert((VIEW, "V".to_string()), "CREATE OR REPLACE VIEW \"HR\".\"V\" AS SELECT 1 x FROM dual".to_string());
        let out = script(Db::Oracle, &Side { label: "a", owner: "HR", schema: &s }, &Side { label: "b", owner: "QA", schema: &t }, &d, &ddl);
        assert!(out.contains("CREATE OR REPLACE VIEW \"QA\".\"V\" AS SELECT 1 x FROM dual;"), "{out}");
        assert!(out.contains("ALTER TABLE \"QA\".\"EMP\" ADD (\"NAME\" VARCHAR2(50 BYTE));"));
        assert!(out.contains("-- DROP TABLE \"QA\".\"OLD\";"));
    }

    #[test]
    fn names_stay_in_comments() {
        let mut t = Schema::new();
        t.insert((TABLE, "X\nDROP TABLE EMP".into()), Obj::default());
        let d = diff(&Schema::new(), &t);
        let out = script(Db::Oracle, &Side { label: "a", owner: "HR", schema: &Schema::new() }, &Side { label: "b", owner: "QA", schema: &t }, &d, &BTreeMap::new());
        assert!(out.lines().all(|l| l.is_empty() || l.starts_with("--")), "{out}");
        // DBMS_METADATA terminators are kept as they come.
        assert_eq!(statement(Db::Oracle, "CREATE OR REPLACE PACKAGE \"HR\".\"P\" AS\nEND;\n/\n", "HR", "QA"), "CREATE OR REPLACE PACKAGE \"QA\".\"P\" AS\nEND;\n/");
    }
}
