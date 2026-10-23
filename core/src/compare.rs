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

/// DDL fetched from the source, pointed at the target schema and terminated for Run Script.
/// Oracle code DDL can hold several units (package spec and body, trigger + ALTER TRIGGER).
fn statement(db: Db, kind: &str, ddl: &str, src_owner: &str, tgt_owner: &str) -> String {
    let mut ddl = ddl.trim().to_string();
    if !src_owner.is_empty() && !tgt_owner.is_empty() && src_owner != tgt_owner {
        ddl = ddl.replace(&format!("{}.", dialect::quote(db, src_owner)), &format!("{}.", dialect::quote(db, tgt_owner)));
    }
    if db != Db::Oracle || !is_code(kind) {
        return if ddl.ends_with(';') { ddl } else { format!("{ddl};") };
    }
    let mut out = String::new();
    let mut unit: Vec<&str> = vec![];
    let flush = |unit: &mut Vec<&str>, out: &mut String| {
        let u = unit.join("\n");
        let u = u.trim();
        if !u.is_empty() {
            let block = u.to_uppercase().starts_with("CREATE");
            out.push_str(u);
            out.push_str(if block { "\n/\n" } else if u.ends_with(';') { "\n" } else { ";\n" });
        }
        unit.clear();
    };
    for line in ddl.lines() {
        let t = line.trim_start().to_uppercase();
        if t.starts_with("CREATE OR REPLACE") || t.starts_with("ALTER ") {
            flush(&mut unit, &mut out);
        }
        unit.push(line);
    }
    flush(&mut unit, &mut out);
    out.trim_end().to_string()
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
        src.label,
        tgt.label,
        count(|d| *d == Diff::OnlySource),
        count(|d| matches!(d, Diff::Changed(_))),
        count(|d| *d == Diff::OnlyTarget),
    );
    let q = |name: &str| dialect::qualified(db, tgt.owner, name);
    let section = |out: &mut String, title: &str| out.push_str(&format!("\n-- ===== {title} =====\n"));
    section(&mut out, "Missing in target");
    for ((kind, name), d) in diffs {
        if *d == Diff::OnlySource {
            match ddl.get(&(*kind, name.clone())) {
                Some(t) => out.push_str(&format!("\n-- {kind} {name}\n{}\n", statement(db, kind, t, src.owner, tgt.owner))),
                None => out.push_str(&format!("\n-- {kind} {name}: no DDL available, create it by hand\n")),
            }
        }
    }
    section(&mut out, "Different");
    for ((kind, name), d) in diffs {
        let Diff::Changed(notes) = d else { continue };
        out.push_str(&format!("\n-- {kind} {name}\n"));
        for n in notes {
            out.push_str(&format!("--   {n}\n"));
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
            out.push_str(&statement(db, kind, t, src.owner, tgt.owner));
            out.push('\n');
        }
    }
    section(&mut out, "Only in target (commented out)");
    for ((kind, name), d) in diffs {
        if *d == Diff::OnlyTarget {
            out.push_str(&format!("-- DROP {kind} {};\n", q(name)));
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
    fn oracle_code_units() {
        let ddl = "\n  CREATE OR REPLACE PACKAGE \"HR\".\"P\" AS\n  x number;\nEND p;\n  CREATE OR REPLACE PACKAGE BODY \"HR\".\"P\" AS\nEND p;\nALTER TRIGGER \"HR\".\"T\" ENABLE";
        let s = statement(Db::Oracle, PACKAGE, ddl, "HR", "HR");
        assert_eq!(s, "CREATE OR REPLACE PACKAGE \"HR\".\"P\" AS\n  x number;\nEND p;\n/\nCREATE OR REPLACE PACKAGE BODY \"HR\".\"P\" AS\nEND p;\n/\nALTER TRIGGER \"HR\".\"T\" ENABLE;");
    }
}
