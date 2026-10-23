//! Tools ▸ Schema Compare: pick a source and a target (connection + schema), read both
//! schemas' objects, columns and code over the metadata sessions, and open the sync script in a
//! worksheet on the target. Nothing is run.

use crate::conn::{Conn, ConnRef, Res};
use rsqldev_core::compare::{self, Diff, Obj, Schema, Side};
use rsqldev_core::dialect;
use std::collections::BTreeMap;
use wxdragon::prelude::*;

/// Object kinds compared (presence for all; columns or code where it applies).
const KINDS: &[&str] = &[dialect::TABLE, dialect::VIEW, dialect::MVIEW, dialect::PACKAGE, dialect::PROCEDURE, dialect::FUNCTION, dialect::TRIGGER, dialect::TYPE, dialect::SEQUENCE];

pub fn show(frame: &Frame) {
    let names = crate::app().connection_names();
    if names.is_empty() {
        return crate::ui::info(frame, "Create a connection first.");
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let pick = |title: &str| {
        let d = SingleChoiceDialog::builder(frame, &format!("{title} connection:"), "Schema Compare", &refs).build();
        (d.show_modal() == ID_OK && d.get_selection() >= 0).then(|| names[d.get_selection() as usize].clone())
    };
    let Some(src) = pick("Source (the reference)") else { return };
    let Some(src_schema) = crate::ui::ask(frame, "Source schema (empty = the connection's current schema):", "Schema Compare", "") else { return };
    let Some(tgt) = pick("Target (to bring in line)") else { return };
    let Some(tgt_schema) = crate::ui::ask(frame, "Target schema (empty = the connection's current schema):", "Schema Compare", "") else { return };
    let app = crate::app();
    app.connect(&src, move |s| {
        crate::app().connect(&tgt, move |t| {
            if s.borrow().kind != t.borrow().kind {
                return crate::ui::error(&crate::app().frame, "Schema Compare needs two connections to the same kind of database.");
            }
            schema_of(&s, src_schema, move |so| {
                schema_of(&t, tgt_schema, move |to| run(s, so, t, to));
            });
        });
    });
}

/// The typed schema, folded the way the engine stores names, or the current one.
fn schema_of(c: &ConnRef, typed: String, cb: impl FnOnce(String) + 'static) {
    let db = c.borrow().kind;
    if !typed.trim().is_empty() {
        return cb(dialect::fold(db, typed.trim()));
    }
    Conn::meta_query(c, dialect::current_schema_sql(db), move |r| cb(r.ok().and_then(|r| r.rows.into_iter().next()?.into_iter().next()?).unwrap_or_default()));
}

/// Concatenated column 0 of every row (`col == usize::MAX`), else column `col` of the first row.
fn text(r: Res, col: usize) -> String {
    if col == usize::MAX {
        r.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect()
    } else {
        r.rows.into_iter().next().and_then(|row| row.into_iter().nth(col).flatten()).unwrap_or_default()
    }
}

/// Metadata queries one after another: `each` gets every result in order, then `done` runs.
fn chain(c: ConnRef, mut jobs: Vec<String>, mut each: impl FnMut(usize, Result<Res, String>) + 'static, done: impl FnOnce() + 'static, i: usize) {
    if jobs.is_empty() {
        return done();
    }
    let sql = jobs.remove(0);
    let c2 = c.clone();
    Conn::meta_query(&c, &sql, move |r| {
        each(i, r);
        chain(c2, jobs, each, done, i + 1)
    });
}

/// Read a schema: object names per kind, then columns or code per object.
// ponytail: one query per object; per-schema bulk queries if large schemas are too slow.
fn read(c: ConnRef, owner: String, label: String, cb: impl FnOnce(Schema) + 'static) {
    let db = c.borrow().kind;
    let kinds: Vec<&'static str> = KINDS.iter().copied().filter(|k| dialect::folders(db).iter().any(|f| f.1 == *k)).collect();
    let lists: Vec<String> = kinds.iter().filter_map(|k| dialect::list_sql(db, k, &owner)).collect();
    let kinds2: Vec<&'static str> = kinds.iter().copied().filter(|k| dialect::list_sql(db, k, &owner).is_some()).collect();
    let schema = std::rc::Rc::new(std::cell::RefCell::new(Schema::new()));
    let s2 = schema.clone();
    let c2 = c.clone();
    chain(
        c.clone(),
        lists,
        move |i, r| {
            for row in r.map(|r| r.rows).unwrap_or_default() {
                if let Some(Some(name)) = row.into_iter().next() {
                    s2.borrow_mut().insert((kinds2[i], name), Obj::default());
                }
            }
        },
        move || {
            // Second pass: details for each object that has any.
            let keys: Vec<(&'static str, String)> = schema.borrow().keys().cloned().collect();
            let mut jobs = vec![];
            let mut what = vec![];
            for (k, n) in keys {
                let q = if compare::has_columns(k) {
                    dialect::children_sql(db, k).map(|s| (dialect::fill(db, &s, &owner, &n), None))
                } else if compare::is_code(k) {
                    dialect::code_sql(db, k, &owner, &n).map(|(s, col)| (s, Some(col)))
                } else {
                    None
                };
                if let Some((sql, col)) = q {
                    jobs.push(sql);
                    what.push(((k, n), col));
                }
            }
            let total = jobs.len();
            let s3 = schema.clone();
            chain(
                c2,
                jobs,
                move |i, r| {
                    crate::app().status(0, &format!("Schema Compare: reading {label} {}/{total}", i + 1));
                    let Ok(r) = r else { return };
                    let (key, col) = &what[i];
                    let mut s = s3.borrow_mut();
                    let Some(o) = s.get_mut(key) else { return };
                    match col {
                        None => o.columns = r.rows.into_iter().map(|row| {
                            let mut it = row.into_iter().map(Option::unwrap_or_default);
                            (it.next().unwrap_or_default(), it.next().unwrap_or_default(), it.next().unwrap_or_default())
                        }).collect(),
                        Some(col) => o.code = text(r, *col),
                    }
                },
                move || cb(schema.take()),
                0,
            );
        },
        0,
    );
}

fn run(s: ConnRef, so: String, t: ConnRef, to: String) {
    let (sl, tl) = (format!("{} / {so}", s.borrow().cfg.name), format!("{} / {to}", t.borrow().cfg.name));
    let db = s.borrow().kind;
    let s2 = s.clone();
    read(s.clone(), so.clone(), sl.clone(), move |src| {
        read(t.clone(), to.clone(), tl.clone(), move |tgt| {
            let diffs = compare::diff(&src, &tgt);
            // Source DDL for what the target lacks, and for changed code (CREATE OR REPLACE).
            let need: Vec<(&'static str, String)> = diffs
                .iter()
                .filter(|(k, d)| *d == Diff::OnlySource || (matches!(d, Diff::Changed(_)) && compare::is_code(k.0)))
                .map(|(k, _)| k.clone())
                .collect();
            let jobs: Vec<(String, usize, (&'static str, String))> = need.into_iter().filter_map(|(k, n)| dialect::ddl_sql(db, k, &so, &n).map(|(q, c)| (q, c, (k, n)))).collect();
            let ddl = std::rc::Rc::new(std::cell::RefCell::new(BTreeMap::new()));
            let d2 = ddl.clone();
            // Oracle: DBMS_METADATA adds the ; and / terminators itself while fetching.
            let ora = db == rsqldev_core::registry::Kind::Oracle;
            let mut sqls: Vec<String> = jobs.iter().map(|j| j.0.clone()).collect();
            if ora {
                sqls.insert(0, compare::ORA_TERMINATORS_ON.into());
                sqls.push(compare::ORA_TERMINATORS_OFF.into());
            }
            chain(
                s2,
                sqls,
                move |i, r| {
                    let Some(j) = (if ora { i.checked_sub(1) } else { Some(i) }).and_then(|i| jobs.get(i)) else { return };
                    if let Ok(r) = r {
                        d2.borrow_mut().insert(j.2.clone(), text(r, j.1));
                    }
                },
                move || {
                    let out = compare::script(db, &Side { label: &sl, owner: &so, schema: &src }, &Side { label: &tl, owner: &to, schema: &tgt }, &diffs, &ddl.borrow());
                    let app = crate::app();
                    app.status(0, &format!("Schema Compare: {} difference(s)", diffs.len()));
                    app.open_worksheet(t);
                    if let Some(ws) = app.current_sheet() {
                        ws.editor.set_text(&out);
                    }
                },
                0,
            );
        });
    });
}
