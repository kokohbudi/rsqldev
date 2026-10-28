//! Table node submenus in the navigator, after SQL Developer's: Table ▸, Column ▸,
//! Constraint ▸, Index ▸, Trigger ▸. Each action asks for its options in a small form, then
//! confirms with the exact SQL and runs it on the worksheet session.

use crate::conn::{Conn, ConnRef};
use rsqldev_core::ddl::{self, ColumnDef, ConKind};
use rsqldev_core::registry::Kind;
use rsqldev_core::table_actions as ta;
use std::rc::Rc;
use wxdragon::prelude::*;

const T_RENAME: i32 = 7201;
const T_STATS: i32 = 7211;
const T_COPY: i32 = 7202;
const T_DROP: i32 = 7203;
const T_TRUNCATE: i32 = 7204;
const T_LOCK: i32 = 7205;
const T_COMMENT: i32 = 7206;
const T_PARALLEL: i32 = 7207;
const T_NOPARALLEL: i32 = 7208;
const T_LOGGING: i32 = 7209;
const T_COUNT: i32 = 7210;
const C_ADD: i32 = 7220;
const C_RENAME: i32 = 7221;
const C_DROP: i32 = 7222;
const C_COMMENT: i32 = 7223;
const K_ADD_PK: i32 = 7230;
const K_ADD_FK: i32 = 7231;
const K_ADD_UNIQUE: i32 = 7232;
const K_ADD_CHECK: i32 = 7233;
const K_ENABLE_ALL: i32 = 7234;
const K_DISABLE_ALL: i32 = 7235;
const K_ENABLE: i32 = 7236;
const K_DISABLE: i32 = 7237;
const K_ENABLE_FKS: i32 = 7238;
const K_DISABLE_FKS: i32 = 7239;
const K_RENAME: i32 = 7240;
const K_DROP: i32 = 7241;
const I_CREATE: i32 = 7250;
const I_DROP: i32 = 7251;
const I_REBUILD: i32 = 7252;
const G_CREATE: i32 = 7260;
const G_PK_SEQ: i32 = 7261;
const G_ENABLE_ALL: i32 = 7262;
const G_DISABLE_ALL: i32 = 7263;
const G_ENABLE: i32 = 7264;
const G_DISABLE: i32 = 7265;
const G_DROP: i32 = 7266;

/// Every id handled here (the navigator routes these to [`run`]).
pub fn handles(id: i32) -> bool {
    (7201..=7266).contains(&id)
}

/// Add the submenus for a table of engine `db` to the node's context menu.
pub fn append(m: &Menu, db: Kind) {
    let oracle = db == Kind::Oracle;
    let toggles = matches!(db, Kind::Oracle | Kind::Mssql);
    let mut t = Menu::builder().append_item(T_RENAME, "Rename…", "").append_item(T_COPY, "Copy…", "").append_item(T_DROP, "Drop…", "").append_item(T_TRUNCATE, "Truncate…", "");
    if !ta::lock_modes(db).is_empty() {
        t = t.append_item(T_LOCK, "Lock…", "");
    }
    if ddl::supports_table_comment(db) {
        t = t.append_item(T_COMMENT, "Comment…", "");
    }
    if oracle {
        t = t.append_item(T_PARALLEL, "Parallel…", "").append_item(T_NOPARALLEL, "No Parallel…", "").append_item(T_LOGGING, "Logging…", "");
    }
    t = t.append_separator().append_item(T_COUNT, "Count Rows…", "");
    if ta::gather_stats(db, "o", "t").is_ok() {
        t = t.append_item(T_STATS, "Gather Statistics…", "Refresh the optimizer statistics of the table (and its indexes)");
    }
    m.append_separator();
    m.append_submenu(t.build(), "Table", "");

    let mut c = Menu::builder().append_item(C_ADD, "Add…", "").append_item(C_RENAME, "Rename…", "").append_item(C_DROP, "Drop…", "");
    if matches!(db, Kind::Oracle | Kind::Postgres | Kind::Duckdb | Kind::Clickhouse) {
        c = c.append_item(C_COMMENT, "Comment…", "");
    }
    m.append_submenu(c.build(), "Column", "");

    if ddl::alters_constraints(db) {
        let mut k = Menu::builder().append_item(K_ADD_PK, "Add Primary Key…", "").append_item(K_ADD_FK, "Add Foreign Key…", "").append_item(K_ADD_UNIQUE, "Add Unique…", "").append_item(K_ADD_CHECK, "Add Check…", "");
        if toggles {
            k = k
                .append_separator()
                .append_item(K_ENABLE_ALL, "Enable all…", "")
                .append_item(K_DISABLE_ALL, "Disable all…", "")
                .append_item(K_ENABLE, "Enable Single…", "")
                .append_item(K_DISABLE, "Disable Single…", "")
                .append_item(K_ENABLE_FKS, "Enable Related Foreign Keys…", "Foreign keys of other tables that reference this one")
                .append_item(K_DISABLE_FKS, "Disable Related Foreign Keys…", "");
        }
        k = k.append_separator();
        if matches!(db, Kind::Oracle | Kind::Postgres | Kind::Mssql) {
            k = k.append_item(K_RENAME, "Rename Single…", "");
        }
        m.append_submenu(k.append_item(K_DROP, "Drop…", "").build(), "Constraint", "");
    }

    let mut i = Menu::builder().append_item(I_CREATE, "Create Index…", "");
    if db != Kind::Clickhouse {
        i = i.append_item(I_DROP, "Drop…", "");
    }
    if ddl::rebuild_index(db, "", "", "x").is_some() {
        i = i.append_item(I_REBUILD, "Rebuild…", "");
    }
    m.append_submenu(i.build(), "Index", "");

    if ta::triggers_sql(db, "", "x").is_some() {
        let mut g = Menu::builder().append_item(G_CREATE, "Create…", "Trigger template in a worksheet");
        if oracle {
            g = g.append_item(G_PK_SEQ, "Create (PK from Sequence)…", "");
        }
        if matches!(db, Kind::Oracle | Kind::Postgres | Kind::Mssql) {
            g = g.append_item(G_ENABLE_ALL, "Enable All…", "").append_item(G_DISABLE_ALL, "Disable All…", "").append_item(G_ENABLE, "Enable Single…", "").append_item(G_DISABLE, "Disable Single…", "");
        }
        m.append_submenu(g.append_item(G_DROP, "Drop…", "").build(), "Trigger", "");
    }
}

pub(crate) enum F {
    Text(&'static str, String),
    /// Masked, not trimmed.
    Password(&'static str),
    Choice(&'static str, Vec<String>),
    Check(&'static str, bool),
}

/// A small options form: one value per field (`"1"` / `""` for check boxes). None = Cancel.
pub(crate) fn form(parent: &dyn WxWidget, title: &str, fields: Vec<F>) -> Option<Vec<String>> {
    let dlg = Dialog::builder(parent, title).with_style(DialogStyle::DefaultDialogStyle).build();
    let g = FlexGridSizer::builder(0, 2).with_vgap(6).with_hgap(10).build();
    g.add_growable_col(1, 1);
    let mut read: Vec<Box<dyn Fn() -> String>> = vec![];
    for f in fields {
        match f {
            F::Text(l, v) => {
                let t = TextCtrl::builder(&dlg).with_value(&v).build();
                t.set_min_size(Size::new(300, -1));
                crate::ui::row(&dlg, &g, l, &t);
                read.push(Box::new(move || t.get_value().trim().to_string()));
            }
            F::Password(l) => {
                let t = TextCtrl::builder(&dlg).with_style(TextCtrlStyle::Password).build();
                t.set_min_size(Size::new(300, -1));
                crate::ui::row(&dlg, &g, l, &t);
                read.push(Box::new(move || t.get_value()));
            }
            F::Choice(l, v) => {
                let c = Choice::builder(&dlg).with_choices(v).build();
                c.set_selection(0);
                crate::ui::row(&dlg, &g, l, &c);
                read.push(Box::new(move || c.get_string_selection().unwrap_or_default()));
            }
            F::Check(l, v) => {
                let c = CheckBox::builder(&dlg).with_label(l).build();
                c.set_value(v);
                crate::ui::row(&dlg, &g, "", &c);
                read.push(Box::new(move || if c.get_value() { "1".into() } else { String::new() }));
            }
        }
    }
    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    bs.add_stretch_spacer(1);
    bs.add(&Button::builder(&dlg).with_label("Cancel").with_id(ID_CANCEL).build(), 0, SizerFlag::All, 4);
    bs.add(&Button::builder(&dlg).with_label("Apply").with_id(ID_OK).build(), 0, SizerFlag::All, 4);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&g, 1, SizerFlag::Expand | SizerFlag::All, 12);
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer_and_fit(s, true);
    let out = (dlg.show_modal() == ID_OK).then(|| read.iter().map(|f| f()).collect());
    dlg.destroy();
    out
}

/// Confirm with the statements, then run them in order (stop at the first error).
fn confirm_run(parent: &dyn WxWidget, conn: &ConnRef, stmts: Result<Vec<String>, String>, done: Rc<dyn Fn()>) {
    let stmts = match stmts {
        Ok(s) => s,
        Err(e) => return crate::ui::error(parent, &e),
    };
    let text = stmts.iter().map(|s| format!("{s};")).collect::<Vec<_>>().join("\n");
    if crate::ui::confirm_sql(parent, &format!("Run on {}?", conn.borrow().cfg.name), &text) {
        crate::table_editor::run(conn.clone(), stmts, 0, done);
    }
}

/// Fetch a name list (first column, plus the rest of each row), then continue.
fn names(conn: &ConnRef, sql: Option<String>, f: impl FnOnce(Vec<Vec<String>>) + 'static) {
    let Some(sql) = sql else { return f(vec![]) };
    Conn::meta_query(conn, &sql, move |r| match r {
        Ok(r) => f(r.rows.into_iter().map(|row| row.into_iter().map(Option::unwrap_or_default).collect()).collect()),
        Err(e) => crate::ui::error(&crate::app().frame, &e),
    });
}

/// Run a submenu action on table `owner.name`; `done` refreshes the navigator.
pub fn run(id: i32, conn: ConnRef, owner: String, name: String, done: Rc<dyn Fn()>) {
    let db = conn.borrow().kind;
    let frame = crate::app().frame;
    let p = &frame;
    let one = |s: Result<String, String>| s.map(|s| vec![s]);
    let shown = ddl::shown(db, &name);
    let tq = ta::table(db, &owner, &name);
    let load = ddl::load_sql(db, &owner, &name);
    let cols_sql = load.as_ref().map(|l| l.columns.clone());
    let cons_sql = load.as_ref().and_then(|l| l.constraints.clone());
    let idx_sql = load.as_ref().and_then(|l| l.indexes.clone());
    match id {
        T_RENAME => {
            if let Some(v) = form(p, "Rename Table", vec![F::Text("New Table Name:", shown)]) {
                confirm_run(p, &conn, one(ta::rename(db, &owner, &name, &v[0])), done);
            }
        }
        T_COPY => {
            if let Some(v) = form(p, "Copy Table", vec![F::Text("New Table Owner:", ddl::shown(db, &owner)), F::Text("New Table Name:", format!("{shown}_COPY")), F::Check("Include Data", false)]) {
                confirm_run(p, &conn, ta::copy(db, &owner, &name, &v[0], &v[1], v[2] == "1"), done);
            }
        }
        T_DROP => {
            let mut f = vec![];
            if matches!(db, Kind::Oracle | Kind::Postgres | Kind::Duckdb) {
                f.push(F::Check("Cascade Constraints", false));
            }
            if db == Kind::Oracle {
                f.push(F::Check("Purge", false));
            }
            if let Some(v) = form(p, &format!("Drop Table {shown}"), f) {
                let on = |i: usize| v.get(i).is_some_and(|x| x == "1");
                confirm_run(p, &conn, one(ta::drop(db, &owner, &name, on(0), on(1))), done);
            }
        }
        T_TRUNCATE => {
            let f = if db == Kind::Oracle { vec![F::Choice("Storage:", vec!["Drop Storage".into(), "Reuse Storage".into()])] } else { vec![] };
            if let Some(v) = form(p, &format!("Truncate Table {shown}"), f) {
                confirm_run(p, &conn, one(ta::truncate(db, &owner, &name, v.first().is_some_and(|s| s.starts_with("Reuse")))), done);
            }
        }
        T_LOCK => {
            let mut f = vec![F::Choice("Lock Mode:", ta::lock_modes(db).iter().map(|s| s.to_string()).collect())];
            if db != Kind::Mysql {
                f.push(F::Check("No Wait", true));
            }
            if let Some(v) = form(p, &format!("Lock Table {shown}"), f) {
                confirm_run(p, &conn, one(ta::lock(db, &owner, &name, &v[0], v.get(1).is_some_and(|x| x == "1"))), done);
            }
        }
        T_COMMENT => {
            let c2 = conn.clone();
            names(&conn, load.as_ref().and_then(|l| l.comment.clone()), move |rows| {
                let cur = rows.first().and_then(|r| r.first().cloned()).unwrap_or_default();
                if let Some(v) = form(&frame, &format!("Comment on {shown}"), vec![F::Text("Comment:", cur)]) {
                    confirm_run(&frame, &c2, one(ta::comment(db, &owner, &name, &v[0])), done);
                }
            });
        }
        T_PARALLEL => {
            if let Some(v) = form(p, "Parallel", vec![F::Text("Degree (empty = default):", String::new())]) {
                confirm_run(p, &conn, one(ta::parallel(db, &owner, &name, Some(&v[0]))), done);
            }
        }
        T_NOPARALLEL => confirm_run(p, &conn, one(ta::parallel(db, &owner, &name, None)), done),
        T_LOGGING => {
            if let Some(v) = form(p, "Logging", vec![F::Choice("Logging:", vec!["LOGGING".into(), "NOLOGGING".into()])]) {
                confirm_run(p, &conn, one(ta::logging(db, &owner, &name, v[0] == "LOGGING")), done);
            }
        }
        T_STATS => confirm_run(p, &conn, one(ta::gather_stats(db, &owner, &name)), done),
        T_COUNT => {
            Conn::meta_query(&conn, &format!("SELECT COUNT(*) FROM {tq}"), move |r| match r {
                Ok(r) => crate::ui::info(&frame, &format!("{shown}: {} rows", r.rows.first().and_then(|r| r[0].clone()).unwrap_or_default())),
                Err(e) => crate::ui::error(&frame, &e),
            });
        }
        C_ADD => {
            let types: Vec<String> = ddl::types(db).iter().map(|s| s.to_string()).collect();
            if let Some(v) = form(p, &format!("Add Column to {shown}"), vec![F::Text("Column Name:", String::new()), F::Choice("Data Type:", types), F::Text("Precision / Size:", String::new()), F::Text("Scale:", String::new()), F::Check("Not Null", false), F::Text("Default:", String::new())]) {
                let c = ColumnDef { name: v[0].clone(), ty: v[1].clone(), size: v[2].clone(), scale: v[3].clone(), not_null: v[4] == "1", default: v[5].clone(), ..Default::default() };
                confirm_run(p, &conn, one(ta::add_column(db, &owner, &name, &c)), done);
            }
        }
        C_RENAME | C_DROP | C_COMMENT => {
            let c2 = conn.clone();
            names(&conn, cols_sql, move |rows| {
                let cols: Vec<String> = rows.iter().filter_map(|r| r.first().cloned()).collect();
                if cols.is_empty() {
                    return;
                }
                let (title, extra) = match id {
                    C_RENAME => ("Rename Column", Some(F::Text("New Name:", String::new()))),
                    C_COMMENT => ("Comment on Column", Some(F::Text("Comment:", String::new()))),
                    _ => ("Drop Column", None),
                };
                let mut f = vec![F::Choice("Column:", cols)];
                f.extend(extra);
                let Some(v) = form(&frame, title, f) else { return };
                let s = match id {
                    C_RENAME => ta::rename_column(db, &owner, &name, &v[0], &v[1]),
                    C_COMMENT => ta::comment_column(db, &owner, &name, &v[0], &v[1]),
                    _ => ta::drop_column(db, &owner, &name, &v[0]),
                };
                confirm_run(&frame, &c2, one(s), done);
            });
        }
        K_ADD_PK | K_ADD_UNIQUE => {
            let title = if id == K_ADD_PK { "Add Primary Key" } else { "Add Unique" };
            let default = if id == K_ADD_PK { format!("{}_PK", name.trim_matches('"')) } else { format!("{}_UK1", name.trim_matches('"')) };
            if let Some(v) = form(p, title, vec![F::Text("Constraint Name:", default), F::Text("Columns (a, b):", String::new())]) {
                let kind = (id == K_ADD_UNIQUE).then_some(ConKind::Unique);
                confirm_run(p, &conn, one(ta::add_constraint(db, &owner, &name, kind, &v[0], &v[1], "", "", "")), done);
            }
        }
        K_ADD_FK => {
            if let Some(v) = form(p, "Add Foreign Key", vec![F::Text("Constraint Name:", format!("{}_FK1", name.trim_matches('"'))), F::Text("Column Name(s):", String::new()), F::Text("References Table Name:", String::new()), F::Text("Referencing Column(s):", String::new()), F::Choice("On Delete:", vec!["NO ACTION".into(), "CASCADE".into(), "SET NULL".into()])]) {
                let s = ta::add_constraint(db, &owner, &name, Some(ConKind::Foreign), &v[0], &v[1], &v[2], &v[3], "").map(|s| if v[4] == "NO ACTION" { s } else { format!("{s} ON DELETE {}", v[4]) });
                confirm_run(p, &conn, one(s), done);
            }
        }
        K_ADD_CHECK => {
            if let Some(v) = form(p, "Add Check", vec![F::Text("Constraint Name:", format!("{}_CK1", name.trim_matches('"'))), F::Text("Check Condition:", String::new())]) {
                confirm_run(p, &conn, one(ta::add_constraint(db, &owner, &name, Some(ConKind::Check), &v[0], "", "", "", &v[1])), done);
            }
        }
        K_ENABLE_ALL | K_DISABLE_ALL | K_ENABLE | K_DISABLE | K_RENAME | K_DROP => {
            let c2 = conn.clone();
            names(&conn, cons_sql, move |rows| {
                // rows: name, kind, description, status
                let enable = matches!(id, K_ENABLE_ALL | K_ENABLE);
                let single = !matches!(id, K_ENABLE_ALL | K_DISABLE_ALL);
                let pick: Vec<String> = rows.iter().filter(|r| !single || id != K_ENABLE || r.get(3).is_some_and(|s| s == "DISABLED")).filter_map(|r| r.first().cloned()).collect();
                if pick.is_empty() {
                    return crate::ui::info(&frame, "No matching constraints.");
                }
                let mut f = vec![];
                if single {
                    f.push(F::Choice("Constraint:", pick.clone()));
                }
                if id == K_RENAME {
                    f.push(F::Text("New Name:", String::new()));
                }
                if matches!(id, K_ENABLE_ALL | K_DISABLE_ALL | K_ENABLE | K_DISABLE) && db == Kind::Oracle {
                    f.push(F::Check(if enable { "VALIDATE (check existing rows)" } else { "VALIDATE (keep rows checked)" }, enable));
                }
                let title = match id {
                    K_ENABLE_ALL => "Enable All Constraints",
                    K_DISABLE_ALL => "Disable All Constraints",
                    K_ENABLE => "Enable Constraint",
                    K_DISABLE => "Disable Constraint",
                    K_RENAME => "Rename Constraint",
                    _ => "Drop Constraint",
                };
                let Some(v) = form(&frame, title, f) else { return };
                let validate = v.last().is_some_and(|x| x == "1");
                let kind_of = |n: &str| rows.iter().find(|r| r.first().is_some_and(|x| x == n)).and_then(|r| r.get(1).cloned()).unwrap_or_default();
                let s: Result<Vec<String>, String> = match id {
                    K_ENABLE_ALL | K_DISABLE_ALL => pick.iter().map(|n| ta::enable_constraint(db, &tq, n, enable, validate)).collect(),
                    K_ENABLE | K_DISABLE => one(ta::enable_constraint(db, &tq, &v[0], enable, validate)),
                    K_RENAME => one(ta::rename_constraint(db, &owner, &name, &v[0], &v[1])),
                    _ => one(ta::drop_constraint(db, &owner, &name, &v[0], &kind_of(&v[0]))),
                };
                confirm_run(&frame, &c2, s, done);
            });
        }
        K_ENABLE_FKS | K_DISABLE_FKS => {
            let c2 = conn.clone();
            let enable = id == K_ENABLE_FKS;
            names(&conn, ta::related_fks_sql(db, &owner, &name), move |rows| {
                if rows.is_empty() {
                    return crate::ui::info(&frame, "No other table references this one.");
                }
                let s = rows.iter().map(|r| ta::enable_constraint(db, &ta::table(db, &r[0], &r[1]), &r[2], enable, enable)).collect();
                confirm_run(&frame, &c2, s, done);
            });
        }
        I_CREATE => crate::table_editor::edit(conn, owner, name, crate::table_editor::Start::Indexes, move || done()),
        I_DROP | I_REBUILD => {
            let c2 = conn.clone();
            names(&conn, idx_sql, move |rows| {
                let idx: Vec<String> = rows.iter().filter_map(|r| r.first().cloned()).collect();
                if idx.is_empty() {
                    return crate::ui::info(&frame, "This table has no indexes of its own (constraint indexes go with their constraint).");
                }
                let mut f = vec![F::Choice("Index:", idx)];
                if id == I_REBUILD && db == Kind::Oracle {
                    f.push(F::Check("Online", false));
                }
                let Some(v) = form(&frame, if id == I_DROP { "Drop Index" } else { "Rebuild Index" }, f) else { return };
                let s = if id == I_DROP { Ok(ddl::drop_index(db, &owner, &name, &v[0])) } else { ta::rebuild_index(db, &owner, &name, &v[0], v.get(1).is_some_and(|x| x == "1")) };
                confirm_run(&frame, &c2, one(s), done);
            });
        }
        G_CREATE => match ta::trigger_template(db, &owner, &name) {
            Ok(sql) => {
                let app = crate::app();
                app.open_worksheet(conn);
                if let Some(ws) = app.current_sheet() {
                    ws.editor.set_text(&sql);
                }
            }
            Err(e) => crate::ui::error(p, &e),
        },
        G_PK_SEQ => {
            let c2 = conn.clone();
            let seq_sql = rsqldev_core::dialect::fill(db, "SELECT sequence_name FROM all_sequences WHERE sequence_owner = '{o}' ORDER BY 1", &owner, &name);
            names(&conn, Some(seq_sql), move |seqs| {
                let seqs: Vec<String> = seqs.iter().filter_map(|r| r.first().cloned()).collect();
                if seqs.is_empty() {
                    return crate::ui::info(&frame, "The schema has no sequences: create one first.");
                }
                let c3 = c2.clone();
                names(&c2, cols_sql, move |cols| {
                    // Primary key columns first (Columns tab: column 5 = Y).
                    let mut cols: Vec<(bool, String)> = cols.iter().filter_map(|r| r.first().map(|n| (r.get(5).is_some_and(|y| y == "Y"), n.clone()))).collect();
                    cols.sort_by_key(|c| !c.0);
                    let f = vec![F::Text("Trigger Name:", format!("{}_BI", name.trim_matches('"'))), F::Choice("Sequence Name:", seqs), F::Choice("Column Name:", cols.into_iter().map(|c| c.1).collect())];
                    let Some(v) = form(&frame, "Create Trigger (PK from Sequence)", f) else { return };
                    confirm_run(&frame, &c3, one(ta::pk_from_sequence(db, &owner, &name, &v[0], &v[1], &v[2])), done);
                });
            });
        }
        G_ENABLE_ALL | G_DISABLE_ALL => confirm_run(p, &conn, one(ta::enable_triggers(db, &owner, &name, id == G_ENABLE_ALL)), done),
        G_ENABLE | G_DISABLE | G_DROP => {
            let c2 = conn.clone();
            names(&conn, ta::triggers_sql(db, &owner, &name), move |rows| {
                let want = match id {
                    G_ENABLE => Some("DISABLED"),
                    G_DISABLE => Some("ENABLED"),
                    _ => None,
                };
                let trg: Vec<String> = rows.iter().filter(|r| want.is_none_or(|w| r.get(1).is_some_and(|s| s == w))).filter_map(|r| r.first().cloned()).collect();
                if trg.is_empty() {
                    return crate::ui::info(&frame, "No matching triggers.");
                }
                let title = match id {
                    G_ENABLE => "Enable Trigger",
                    G_DISABLE => "Disable Trigger",
                    _ => "Drop Trigger",
                };
                let Some(v) = form(&frame, title, vec![F::Choice("Trigger:", trg)]) else { return };
                let s = if id == G_DROP { ta::drop_trigger(db, &owner, &name, &v[0]) } else { ta::enable_trigger(db, &owner, &name, &v[0], id == G_ENABLE) };
                confirm_run(&frame, &c2, one(s), done);
            });
        }
        _ => {}
    }
}
