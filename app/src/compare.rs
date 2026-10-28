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
            let (s1, t1) = (s.clone(), t.clone());
            schema_of(&s1, src_schema, move |so| {
                schema_of(&t1, tgt_schema, move |to| run(s, so, t, to));
            });
        });
    });
}

/// The typed schema, folded the way the engine stores names, or the current one.
/// Tools ▸ Data Compare: one table's rows on a source and a target (connection + schema),
/// matched by the source's primary key; the INSERT / UPDATE / DELETE script that brings the
/// target in line opens in a worksheet on the target. Nothing is run.
// ponytail: both sides are read whole into memory; chunk by key ranges for very large tables.
pub fn show_data(frame: &Frame) {
    let names = crate::app().connection_names();
    if names.is_empty() {
        return crate::ui::info(frame, "Create a connection first.");
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let pick = |title: &str| {
        let d = SingleChoiceDialog::builder(frame, &format!("{title} connection:"), "Data Compare", &refs).build();
        (d.show_modal() == ID_OK && d.get_selection() >= 0).then(|| names[d.get_selection() as usize].clone())
    };
    let Some(src) = pick("Source (the reference)") else { return };
    let Some(src_schema) = crate::ui::ask(frame, "Source schema (empty = the connection's current schema):", "Data Compare", "") else { return };
    let Some(table) = crate::ui::ask(frame, "Table:", "Data Compare", "").filter(|t| !t.trim().is_empty()) else { return };
    let Some(tgt) = pick("Target (to bring in line)") else { return };
    let Some(tgt_schema) = crate::ui::ask(frame, "Target schema (empty = the connection's current schema):", "Data Compare", "") else { return };
    crate::app().connect(&src, move |s| {
        crate::app().connect(&tgt, move |t| {
            if s.borrow().kind != t.borrow().kind {
                return crate::ui::error(&crate::app().frame, "Data Compare needs two connections to the same kind of database.");
            }
            let db = s.borrow().kind;
            let name = rsqldev_core::ddl::ident(db, &table);
            let (s1, t1) = (s.clone(), t.clone());
            schema_of(&s1, src_schema, move |so| {
                schema_of(&t1, tgt_schema, move |to| {
                    let Some(pk_sql) = dialect::pk_sql(db, &so, &name) else {
                        return crate::ui::info(&crate::app().frame, "Data Compare needs primary keys, which this database does not have.");
                    };
                    let (src_t, tgt_t) = (dialect::qualified(db, &so, &name), dialect::qualified(db, &to, &name));
                    crate::app().status(0, &format!("Data Compare: reading {src_t} and {tgt_t}…"));
                    let s2 = s.clone();
                    Conn::meta_query(&s, &pk_sql, move |r| {
                        let pk: Vec<String> = match r {
                            Ok(r) => r.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect(),
                            Err(e) => return crate::ui::error(&crate::app().frame, &e),
                        };
                        let order = pk.iter().map(|c| dialect::quote(db, c)).collect::<Vec<_>>().join(", ");
                        let q = |t: &str| if order.is_empty() { format!("SELECT * FROM {t}") } else { format!("SELECT * FROM {t} ORDER BY {order}") };
                        let (qs, qt) = (q(&src_t), q(&tgt_t));
                        Conn::meta_query(&s2, &qs, move |rs| {
                            let rs = match rs {
                                Ok(r) => r,
                                Err(e) => return crate::ui::error(&crate::app().frame, &format!("{src_t}: {e}")),
                            };
                            let t2 = t.clone();
                            Conn::meta_query(&t, &qt, move |rt| {
                                let app = crate::app();
                                let rt = match rt {
                                    Ok(r) => r,
                                    Err(e) => return crate::ui::error(&app.frame, &format!("{tgt_t}: {e}")),
                                };
                                match compare::data_diff(db, &tgt_t, &rs.cols, &rt.cols, &pk, &rs.rows, &rt.rows) {
                                    Ok(d) if d.inserts.is_empty() && d.updates.is_empty() && d.deletes.is_empty() => {
                                        crate::ui::info(&app.frame, &format!("{src_t} and {tgt_t} hold the same {} rows.", d.same))
                                    }
                                    Ok(d) => {
                                        let mut text = format!(
                                            "-- Data Compare: {src_t} (source) -> {tgt_t} (target)\n-- {} to insert, {} to update, {} to delete, {} the same. Review, then Run Script (F5) and Commit.\n\n",
                                            d.inserts.len(),
                                            d.updates.len(),
                                            d.deletes.len(),
                                            d.same
                                        );
                                        for s in d.deletes.iter().chain(&d.updates).chain(&d.inserts) {
                                            text.push_str(s);
                                            text.push_str(";\n");
                                        }
                                        app.open_worksheet(t2);
                                        if let Some(ws) = app.current_sheet() {
                                            ws.editor.set_text(&text);
                                        }
                                        app.status(0, "Data Compare done");
                                    }
                                    Err(e) => crate::ui::error(&app.frame, &e),
                                }
                            });
                        });
                    });
                });
            });
        });
    });
}

/// Tools ▸ Database Export: the DDL of the chosen object kinds of one schema, and table data
/// as INSERT statements, into one .sql file (sequences, tables, data, then the rest). Read on
/// the metadata session; Oracle DDL comes with its ; and / terminators.
// ponytail: each table's rows are read whole before they are written; stream per batch if
// huge tables need it (the grid's Export streams).
pub fn show_export(frame: &Frame, copy: bool) {
    let title = if copy { "Database Copy" } else { "Database Export" };
    let names = crate::app().connection_names();
    if names.is_empty() {
        return crate::ui::info(frame, "Create a connection first.");
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let pick = |msg: &str| {
        let d = SingleChoiceDialog::builder(frame, msg, title, &refs).build();
        (d.show_modal() == ID_OK && d.get_selection() >= 0).then(|| names[d.get_selection() as usize].clone())
    };
    let Some(conn) = pick(if copy { "Source connection:" } else { "Connection:" }) else { return };
    let Some(schema) = crate::ui::ask(frame, "Schema (empty = the connection's current schema):", title, "") else { return };
    // Copy: the script goes to a worksheet on the target, the schema renamed.
    let target = if copy {
        let Some(t) = pick("Target connection (a worksheet with the script opens there):") else { return };
        let Some(ts) = crate::ui::ask(frame, "Target schema (empty = the target's current schema):", title, "") else { return };
        Some((t, ts))
    } else {
        None
    };
    let frame = *frame;
    crate::app().connect(&conn, move |c| {
        let db = c.borrow().kind;
        let kinds: Vec<&'static str> = [dialect::SEQUENCE, dialect::TABLE, dialect::INDEX, dialect::VIEW, dialect::MVIEW, dialect::SYNONYM, dialect::TYPE, dialect::PACKAGE, dialect::PROCEDURE, dialect::FUNCTION, dialect::TRIGGER]
            .into_iter()
            .filter(|k| dialect::folders(db).iter().any(|f| f.1 == *k) && dialect::list_sql(db, k, "x").is_some())
            .collect();
        let labels: Vec<String> = kinds.iter().map(|k| k.to_lowercase()).collect();
        let lrefs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let md = MultiChoiceDialog::builder(&frame, "Object types:", title, &lrefs).build();
        md.set_selections(&(0..kinds.len() as i32).collect::<Vec<_>>());
        if md.show_modal() != ID_OK {
            return;
        }
        let picked: Vec<&'static str> = md.get_selections().into_iter().filter_map(|i| kinds.get(i as usize).copied()).collect();
        if picked.is_empty() {
            return;
        }
        let data = picked.contains(&dialect::TABLE) && crate::ui::confirm(&frame, "Include table data (INSERT statements)?");
        let (sink, path) = match &target {
            Some(_) => (Sink::Text(vec![]), String::new()),
            None => {
                let fd = FileDialog::builder(&frame).with_message("Export to").with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt).with_default_file("export.sql").with_wildcard("SQL files (*.sql)|*.sql").build();
                let Some(path) = (fd.show_modal() == ID_OK).then(|| fd.get_path()).flatten() else { return };
                match std::fs::File::create(&path) {
                    Ok(f) => (Sink::File(std::io::BufWriter::new(f)), path),
                    Err(e) => return crate::ui::error(&frame, &format!("{path}: {e}")),
                }
            }
        };
        let file = std::rc::Rc::new(std::cell::RefCell::new(sink));
        schema_of(&c.clone(), schema, move |owner| {
            let lists: Vec<String> = picked.iter().filter_map(|k| dialect::list_sql(db, k, &owner)).collect();
            let objs: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, String)>>> = Default::default();
            let o2 = objs.clone();
            let c2 = c.clone();
            crate::app().status(0, &format!("Database Export: listing {owner}…"));
            chain(
                c.clone(),
                lists,
                move |i, r| {
                    for row in r.map(|r| r.rows).unwrap_or_default() {
                        if let Some(Some(n)) = row.into_iter().next() {
                            o2.borrow_mut().push((picked[i], n));
                        }
                    }
                },
                move || match target {
                    None => export_objects(c2, owner, objs.take(), data, file, path, None),
                    Some((t, ts)) => crate::app().connect(&t, move |tc| {
                        let tc2 = tc.clone();
                        schema_of(&tc, ts, move |towner| export_objects(c2, owner, objs.take(), data, file, path, Some((tc2, towner))));
                    }),
                },
                0,
            );
        });
    });
}

/// Tools ▸ ER Diagram: a schema's tables (optionally a name pattern) and foreign keys as an
/// SVG file, opened in the default viewer (a browser).
pub fn show_er(frame: &Frame) {
    let names = crate::app().connection_names();
    if names.is_empty() {
        return crate::ui::info(frame, "Create a connection first.");
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let d = SingleChoiceDialog::builder(frame, "Connection:", "ER Diagram", &refs).build();
    if d.show_modal() != ID_OK || d.get_selection() < 0 {
        return;
    }
    let conn = names[d.get_selection() as usize].clone();
    let Some(schema) = crate::ui::ask(frame, "Schema (empty = the connection's current schema):", "ER Diagram", "") else { return };
    let Some(pattern) = crate::ui::ask(frame, "Tables (name filter: % or * any, _ one, plain text = contains; empty = all):", "ER Diagram", "") else { return };
    crate::app().connect(&conn, move |c| {
        let c2 = c.clone();
        schema_of(&c, schema, move |owner| {
            let db = c2.borrow().kind;
            let Some((cq, fq)) = rsqldev_core::er::queries(db, &owner) else {
                return crate::ui::info(&crate::app().frame, "No foreign keys to draw on this database.");
            };
            let c3 = c2.clone();
            crate::app().status(0, &format!("ER Diagram: reading {owner}…"));
            Conn::meta_query(&c2, &cq, move |r| {
                let mut tables = match r {
                    Ok(r) => rsqldev_core::er::tables(&r.rows),
                    Err(e) => return crate::ui::error(&crate::app().frame, &e),
                };
                if !pattern.trim().is_empty() {
                    tables.retain(|n, _| rsqldev_core::config::name_matches(&pattern, n));
                }
                if tables.is_empty() {
                    return crate::ui::info(&crate::app().frame, "No tables to draw.");
                }
                Conn::meta_query(&c3, &fq, move |r| {
                    let app = crate::app();
                    let fks: Vec<(String, String)> = r.map(|r| r.rows).unwrap_or_default().into_iter().filter_map(|row| Some((row.first().cloned().flatten()?, row.get(1).cloned().flatten()?))).collect();
                    let svg = rsqldev_core::er::svg(&format!("{owner} — {} tables", tables.len()), &tables, &fks);
                    let fd = FileDialog::builder(&app.frame).with_message("Save ER diagram").with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt).with_default_file(&format!("{owner}.svg")).with_wildcard("SVG (*.svg)|*.svg").build();
                    let Some(path) = (fd.show_modal() == ID_OK).then(|| fd.get_path()).flatten() else { return };
                    match std::fs::write(&path, svg) {
                        Ok(()) => {
                            let _ = crate::driver_manager::open::that(std::path::Path::new(&path));
                            app.status(0, &format!("ER diagram saved to {path}"));
                        }
                        Err(e) => crate::ui::error(&app.frame, &format!("{path}: {e}")),
                    }
                });
            });
        });
    });
}

/// What each export job's result is.
enum Job {
    Setting,
    Ddl(String, usize),
    Data(String),
}

/// Where Database Export / Copy writes.
enum Sink {
    File(std::io::BufWriter<std::fs::File>),
    Text(Vec<u8>),
}

impl std::io::Write for Sink {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self {
            Sink::File(f) => f.write(b),
            Sink::Text(v) => v.write(b),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Sink::File(f) => f.flush(),
            Sink::Text(_) => Ok(()),
        }
    }
}

/// `to`: Database Copy's target connection and schema (the source schema prefix renamed).
fn export_objects(c: ConnRef, owner: String, objs: Vec<(&'static str, String)>, data: bool, file: std::rc::Rc<std::cell::RefCell<Sink>>, path: String, to: Option<(ConnRef, String)>) {
    use std::io::Write;
    let db = c.borrow().kind;
    if to.as_ref().is_some_and(|t| t.0.borrow().kind != db) {
        return crate::ui::error(&crate::app().frame, "Database Copy needs two connections to the same kind of database (the script is in the source's SQL dialect).");
    }
    let towner = to.as_ref().map(|t| t.1.clone()).filter(|t| *t != owner);
    let rename = {
        let (from, into) = (format!("{}.", dialect::quote(db, &owner)), towner.as_ref().map(|t| format!("{}.", dialect::quote(db, t))));
        move |s: &str| match &into {
            Some(i) => s.replace(&from, i),
            None => s.to_string(),
        }
    };
    let mut jobs: Vec<String> = vec![];
    let mut what: Vec<Job> = vec![];
    if db == rsqldev_core::registry::Kind::Oracle {
        jobs.push(compare::ORA_TERMINATORS_ON.into());
        what.push(Job::Setting);
    }
    for (kind, name) in &objs {
        if let Some((sql, col)) = dialect::ddl_sql(db, kind, &owner, name) {
            jobs.push(sql);
            what.push(Job::Ddl(format!("{} {name}", kind.to_lowercase()), col));
        }
        // Table data right after the tables (before views, code and triggers).
        if data && *kind == dialect::TABLE {
            jobs.push(format!("SELECT * FROM {}", dialect::qualified(db, &owner, name)));
            what.push(Job::Data(dialect::qualified(db, towner.as_deref().unwrap_or(&owner), name)));
        }
    }
    if db == rsqldev_core::registry::Kind::Oracle {
        jobs.push(compare::ORA_TERMINATORS_OFF.into());
        what.push(Job::Setting);
    }
    let _ = writeln!(file.borrow_mut(), "-- rsqldev Database Export: {} objects of {owner}{}\n", objs.len(), if data { ", with table data" } else { "" });
    let (f2, n) = (file.clone(), objs.len());
    let oracle = db == rsqldev_core::registry::Kind::Oracle;
    chain(
        c,
        jobs,
        move |i, r| {
            let mut f = f2.borrow_mut();
            match (&what[i], r) {
                (Job::Setting, _) => {}
                (Job::Ddl(label, _), Err(e)) | (Job::Data(label), Err(e)) => {
                    let _ = writeln!(f, "-- {label}: {}\n", e.replace('\n', " "));
                }
                (Job::Ddl(label, col), Ok(r)) => {
                    let text: String = if *col == usize::MAX { r.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect() } else { r.rows.into_iter().next().and_then(|row| row.into_iter().nth(*col).flatten()).unwrap_or_default() };
                    let text = text.trim();
                    // Oracle brings its own terminators; others get a ; when missing.
                    let end = if oracle || text.ends_with(';') { "" } else { ";" };
                    let _ = writeln!(f, "-- {label}\n{}{end}\n", rename(text));
                }
                (Job::Data(table), Ok(r)) => {
                    let opts = rsqldev_core::export::Options { table: table.clone(), ..Default::default() };
                    let _ = write!(f, "{}\n", rsqldev_core::export::text_with(rsqldev_core::export::Format::Inserts, db, &opts, &r.cols, &r.rows));
                }
            }
        },
        move || {
            let res = file.borrow_mut().flush();
            let app = crate::app();
            match (to, res) {
                (Some((tc, _)), _) => {
                    let Sink::Text(v) = &*file.borrow() else { return };
                    app.open_worksheet(tc);
                    if let Some(ws) = app.current_sheet() {
                        ws.editor.set_text(&String::from_utf8_lossy(v));
                    }
                    app.status(0, &format!("Database Copy: {n} objects scripted; review, then Run Script (F5)"));
                }
                (None, Ok(())) => app.status(0, &format!("Exported {n} objects to {path}")),
                (None, Err(e)) => crate::ui::error(&app.frame, &format!("{path}: {e}")),
            }
        },
        0,
    );
}

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
