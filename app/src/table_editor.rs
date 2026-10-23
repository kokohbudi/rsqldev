//! Create Table / Edit Table dialog, after SQL Developer's: Columns, Constraints, Indexes and
//! a DDL tab showing exactly what will run. OK runs it (after confirming), Copy to Worksheet
//! opens it for running by hand.

use crate::conn::{Conn, ConnRef, Res};
use rsqldev_core::ddl::{self, ColumnDef, ConKind, ConstraintDef, IndexDef, TableDef};
use rsqldev_core::registry::Kind;
use std::cell::RefCell;
use std::rc::Rc;
use wxdragon::prelude::*;

/// Which tab opens first; `Indexes` also adds an empty index row (New Index…).
#[derive(Clone, Copy, PartialEq)]
pub enum Start {
    Columns,
    Indexes,
}

// Columns grid.
const C_NAME: i32 = 0;
const C_TYPE: i32 = 1;
const C_SIZE: i32 = 2;
const C_SCALE: i32 = 3;
const C_NOTNULL: i32 = 4;
const C_DEFAULT: i32 = 5;
const C_PK: i32 = 6;
const C_COMMENT: i32 = 7;
// Constraints grid.
const K_NAME: i32 = 0;
const K_TYPE: i32 = 1;
const K_COLS: i32 = 2;
const K_REF: i32 = 3;
const K_REFCOLS: i32 = 4;
const K_DELETE: i32 = 5;
const K_CHECK: i32 = 6;
const K_INFO: i32 = 7;
// Indexes grid.
const I_NAME: i32 = 0;
const I_COLS: i32 = 1;
const I_UNIQUE: i32 = 2;
const I_INFO: i32 = 3;

/// Rows of the three grids beyond what the cells hold: the loaded column name, and the
/// loaded (read-only) constraint / index a row stands for.
#[derive(Default)]
struct Rows {
    cols: Vec<Option<String>>,
    cons: Vec<Option<ConstraintDef>>,
    idx: Vec<Option<IndexDef>>,
}

/// Tables ▸ New Table…
pub fn create(conn: ConnRef, owner: String, done: impl Fn() + 'static) {
    dialog(conn, owner, None, Start::Columns, Rc::new(done));
}

/// Table ▸ Edit Table… / New Index…: load the table, then open the dialog.
pub fn edit(conn: ConnRef, owner: String, name: String, start: Start, done: impl Fn() + 'static) {
    let db = conn.borrow().kind;
    let Some(q) = ddl::load_sql(db, &owner, &name) else { return };
    let c2 = conn.clone();
    let done: Rc<dyn Fn()> = Rc::new(done);
    let fetch = |c: &ConnRef, sql: Option<String>, cb: Box<dyn FnOnce(Result<Vec<Vec<Option<String>>>, String>)>| match sql {
        Some(s) => Conn::meta_query(c, &s, move |r: Result<Res, String>| cb(r.map(|r| r.rows))),
        None => cb(Ok(vec![])),
    };
    let (cq, iq) = (q.constraints, q.indexes);
    Conn::meta_query(&conn, &q.columns, move |cols| {
        let cols = match cols {
            Ok(r) => r.rows,
            Err(e) => return crate::ui::error(&crate::app().frame, &e),
        };
        let c3 = c2.clone();
        fetch(
            &c2,
            cq,
            Box::new(move |cons| {
                let c4 = c3.clone();
                fetch(
                    &c3,
                    iq,
                    Box::new(move |idx| match (cons, idx) {
                        (Ok(cons), Ok(idx)) => {
                            let t = ddl::from_rows(db, &owner, &name, &cols, &cons, &idx);
                            dialog(c4, owner, Some(t), start, done);
                        }
                        (Err(e), _) | (_, Err(e)) => crate::ui::error(&crate::app().frame, &e),
                    }),
                );
            }),
        );
    });
}

fn grid(parent: &Panel, labels: &[(&str, i32)]) -> Grid {
    let g = Grid::builder(parent).build();
    g.create_grid(0, labels.len() as i32, GridSelectionMode::Cells);
    g.set_row_label_size(32);
    for (i, (l, w)) in labels.iter().enumerate() {
        g.set_col_label_value(i as i32, l);
        g.set_col_size(i as i32, *w);
    }
    g.enable_editing(true);
    g
}

fn buttons(parent: &Panel, labels: &[&str]) -> (BoxSizer, Vec<Button>) {
    let bar = BoxSizer::builder(Orientation::Horizontal).build();
    let bs: Vec<Button> = labels
        .iter()
        .map(|l| {
            let b = Button::builder(parent).with_label(l).build();
            bar.add(&b, 0, SizerFlag::All, 3);
            b
        })
        .collect();
    (bar, bs)
}

/// Lay out a grid page (buttons, optional note, grid) on `p` and add it to the notebook.
fn layout(nb: &Notebook, p: Panel, title: &str, g: Grid, bar: BoxSizer, note: Option<&str>, select: bool) {
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
    if let Some(n) = note {
        let t = StaticText::builder(&p).with_label(n).build();
        t.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        s.add(&t, 0, SizerFlag::All, 4);
    }
    s.add(&g, 1, SizerFlag::Expand, 0);
    p.set_sizer(s, true);
    nb.add_page(&p, title, select, None);
}

fn yes(v: bool) -> &'static str {
    if v { "1" } else { "" }
}

/// A loaded row: grey, every cell read-only.
fn lock_row(g: &Grid, r: i32) {
    let grey = crate::theme::c(crate::theme::p().muted);
    for c in 0..g.get_number_cols() {
        g.set_read_only(r, c, true);
        g.set_cell_text_colour(r, c, &grey);
    }
}

fn dialog(conn: ConnRef, owner: String, old: Option<TableDef>, start: Start, done: Rc<dyn Fn()>) {
    let db = conn.borrow().kind;
    let frame = crate::app().frame;
    let editing = old.is_some();
    let title = match &old {
        Some(t) => format!("Edit Table {}", t.name),
        None => "Create Table".into(),
    };
    let dlg = Dialog::builder(&frame, &title).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(900, 560).build();
    let top = BoxSizer::builder(Orientation::Horizontal).build();
    let schema = StaticText::builder(&dlg).with_label(&format!("Schema: {}", if owner.is_empty() { "(default)" } else { owner.as_str() })).build();
    let name_l = StaticText::builder(&dlg).with_label("Name:").build();
    let name = TextCtrl::builder(&dlg).with_value(old.as_ref().map_or("", |t| t.name.as_str())).build();
    name.set_min_size(Size::new(260, -1));
    // Rename lives in the navigator (Rename…): the dialog edits one table.
    name.enable(!editing);
    top.add(&schema, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 6);
    top.add(&name_l, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 18);
    top.add(&name, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 6);
    let nb = Notebook::builder(&dlg).build();

    // Columns.
    let cp = Panel::builder(&nb).build();
    let (cbar, cb) = buttons(&cp, &["Add Column", "Remove", "Move Up", "Move Down"]);
    let gc = grid(&cp, &[("Name", 170), ("Type", 150), ("Size", 70), ("Scale", 55), ("Not Null", 65), ("Default", 140), ("PK", 40), ("Comment", 160)]);
    let mut types: Vec<String> = ddl::types(db).iter().map(|s| s.to_string()).collect();
    if let Some(t) = &old {
        for c in &t.columns {
            if !types.iter().any(|x| x.eq_ignore_ascii_case(&c.ty)) {
                types.push(c.ty.clone());
            }
        }
    }
    gc.set_col_format_custom(C_TYPE, &format!("choice:{}", types.join(",")));
    gc.set_col_format_bool(C_NOTNULL);
    gc.set_col_format_bool(C_PK);
    if !ddl::supports_comments(db) {
        gc.hide_col(C_COMMENT);
    }
    layout(&nb, cp, "Columns", gc, cbar, None, true);

    // Constraints.
    let kp = Panel::builder(&nb).build();
    let (kbar, kb) = buttons(&kp, &["Add Constraint", "Remove"]);
    let gk = grid(&kp, &[("Name", 150), ("Type", 110), ("Columns", 150), ("Ref Table", 130), ("Ref Columns", 110), ("On Delete", 85), ("Check", 160), ("Existing", 220)]);
    gk.set_col_format_custom(K_TYPE, &format!("choice:{}", ConKind::ALL.map(|k| k.label()).join(",")));
    gk.set_col_format_custom(K_DELETE, "choice:,CASCADE,SET NULL");
    let can_constraints = !editing || ddl::alters_constraints(db);
    let knote = (!can_constraints).then_some("This database cannot add or drop constraints on an existing table.");
    layout(&nb, kp, "Constraints", gk, kbar, knote, false);

    // Indexes.
    let ip = Panel::builder(&nb).build();
    let (ibar, ib) = buttons(&ip, &["Add Index", "Remove"]);
    let gi = grid(&ip, &[("Name", 200), ("Columns (a, b DESC)", 260), ("Unique", 60), ("Existing", 320)]);
    gi.set_col_format_bool(I_UNIQUE);
    layout(&nb, ip, "Indexes", gi, ibar, None, false);

    // DDL.
    let dp = Panel::builder(&nb).build();
    let ddl_text = TextCtrl::builder(&dp).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap).build();
    let prefs = crate::app().prefs();
    if let Some(f) = Font::new_with_details(prefs.editor_font_size, FontFamily::Teletype as i32, 0, 400, false, &prefs.editor_font) {
        ddl_text.set_font(&f);
    }
    let ds = BoxSizer::builder(Orientation::Vertical).build();
    ds.add(&ddl_text, 1, SizerFlag::Expand, 0);
    dp.set_sizer(ds, true);
    nb.add_page(&dp, "DDL", false, None);

    let rows = Rc::new(RefCell::new(Rows::default()));
    let old = Rc::new(old);
    if let Some(t) = old.as_ref() {
        for (r, c) in t.columns.iter().enumerate() {
            let r = r as i32;
            gc.append_rows(1, false);
            for (col, v) in [(C_NAME, c.name.as_str()), (C_TYPE, c.ty.as_str()), (C_SIZE, c.size.as_str()), (C_SCALE, c.scale.as_str()), (C_NOTNULL, yes(c.not_null)), (C_DEFAULT, c.default.as_str()), (C_PK, yes(c.pk)), (C_COMMENT, c.comment.as_str())] {
                gc.set_cell_value(r, col, v);
            }
            rows.borrow_mut().cols.push(c.orig.clone());
        }
        for (r, c) in t.constraints.iter().enumerate() {
            gk.append_rows(1, false);
            gk.set_cell_value(r as i32, K_NAME, &c.name);
            gk.set_cell_value(r as i32, K_TYPE, c.kind.label());
            gk.set_cell_value(r as i32, K_INFO, &c.info);
            lock_row(&gk, r as i32);
            rows.borrow_mut().cons.push(Some(c.clone()));
        }
        for (r, i) in t.indexes.iter().enumerate() {
            gi.append_rows(1, false);
            gi.set_cell_value(r as i32, I_NAME, &i.name);
            gi.set_cell_value(r as i32, I_UNIQUE, yes(i.unique));
            gi.set_cell_value(r as i32, I_INFO, &i.info);
            lock_row(&gi, r as i32);
            rows.borrow_mut().idx.push(Some(i.clone()));
        }
    }
    let tname = || old.as_ref().as_ref().map_or_else(|| name.get_value(), |t| t.name.clone());
    if start == Start::Indexes {
        let n = gi.get_number_rows();
        gi.append_rows(1, false);
        gi.set_cell_value(n, I_NAME, &format!("{}_IX{}", tname().trim_matches('"'), n + 1));
        rows.borrow_mut().idx.push(None);
        nb.set_selection(2);
    }

    // Grid buttons.
    let rs = rows.clone();
    cb[0].on_click(move |_| {
        let n = gc.get_number_rows();
        gc.append_rows(1, false);
        gc.set_cell_value(n, C_TYPE, ddl::types(db)[0]);
        rs.borrow_mut().cols.push(None);
        gc.set_grid_cursor(n, C_NAME);
    });
    let rs = rows.clone();
    cb[1].on_click(move |_| {
        let r = gc.get_grid_cursor_row();
        if r >= 0 && r < gc.get_number_rows() {
            gc.delete_rows(r, 1, false);
            rs.borrow_mut().cols.remove(r as usize);
        }
    });
    for (b, up) in [(cb[2], true), (cb[3], false)] {
        let rs = rows.clone();
        b.on_click(move |_| {
            let r = gc.get_grid_cursor_row();
            let o = if up { r - 1 } else { r + 1 };
            if r < 0 || o < 0 || o >= gc.get_number_rows() {
                return;
            }
            gc.save_edit_control_value();
            for c in 0..gc.get_number_cols() {
                let (a, b) = (gc.get_cell_value(r, c), gc.get_cell_value(o, c));
                gc.set_cell_value(r, c, &b);
                gc.set_cell_value(o, c, &a);
            }
            rs.borrow_mut().cols.swap(r as usize, o as usize);
            gc.set_grid_cursor(o, gc.get_grid_cursor_col());
        });
    }
    let rs = rows.clone();
    kb[0].enable(can_constraints);
    kb[1].enable(can_constraints);
    kb[0].on_click(move |_| {
        let n = gk.get_number_rows();
        gk.append_rows(1, false);
        gk.set_cell_value(n, K_TYPE, ConKind::Unique.label());
        rs.borrow_mut().cons.push(None);
        gk.set_grid_cursor(n, K_NAME);
    });
    let rs = rows.clone();
    kb[1].on_click(move |_| {
        let r = gk.get_grid_cursor_row();
        if r >= 0 && r < gk.get_number_rows() {
            gk.delete_rows(r, 1, false);
            rs.borrow_mut().cons.remove(r as usize);
        }
    });
    let rs = rows.clone();
    let tn = name;
    let old2 = old.clone();
    ib[0].on_click(move |_| {
        let n = gi.get_number_rows();
        gi.append_rows(1, false);
        let t = old2.as_ref().as_ref().map_or_else(|| tn.get_value(), |t| t.name.clone());
        gi.set_cell_value(n, I_NAME, &format!("{}_IX{}", t.trim_matches('"'), n + 1));
        rs.borrow_mut().idx.push(None);
        gi.set_grid_cursor(n, I_COLS);
    });
    let rs = rows.clone();
    ib[1].on_click(move |_| {
        let r = gi.get_grid_cursor_row();
        if r >= 0 && r < gi.get_number_rows() {
            gi.delete_rows(r, 1, false);
            rs.borrow_mut().idx.remove(r as usize);
        }
    });

    // The edited definition and its DDL.
    let (rs, old3) = (rows.clone(), old.clone());
    let own = owner.clone();
    let build = Rc::new(move || -> Result<Vec<String>, String> {
        for g in [gc, gk, gi] {
            g.save_edit_control_value();
        }
        let rows = rs.borrow();
        let cell = |g: &Grid, r: usize, c: i32| g.get_cell_value(r as i32, c).trim().to_string();
        let mut t = TableDef { owner: own.clone(), name: old3.as_ref().as_ref().map_or_else(|| name.get_value(), |t| t.name.clone()), pk_name: old3.as_ref().as_ref().map(|t| t.pk_name.clone()).unwrap_or_default(), ..Default::default() };
        for (r, orig) in rows.cols.iter().enumerate() {
            let c = ColumnDef {
                orig: orig.clone(),
                name: cell(&gc, r, C_NAME),
                ty: cell(&gc, r, C_TYPE),
                size: cell(&gc, r, C_SIZE),
                scale: cell(&gc, r, C_SCALE),
                not_null: cell(&gc, r, C_NOTNULL) == "1",
                default: cell(&gc, r, C_DEFAULT),
                pk: cell(&gc, r, C_PK) == "1",
                comment: cell(&gc, r, C_COMMENT),
            };
            // A blank new row is ignored, not an error.
            if c.orig.is_none() && c.name.is_empty() {
                continue;
            }
            t.columns.push(c);
        }
        for (r, ex) in rows.cons.iter().enumerate() {
            t.constraints.push(match ex {
                Some(c) => c.clone(),
                None => ConstraintDef {
                    existing: false,
                    name: cell(&gk, r, K_NAME),
                    kind: ConKind::parse(&cell(&gk, r, K_TYPE)).unwrap_or(ConKind::Unique),
                    columns: cell(&gk, r, K_COLS),
                    ref_table: cell(&gk, r, K_REF),
                    ref_columns: cell(&gk, r, K_REFCOLS),
                    on_delete: cell(&gk, r, K_DELETE),
                    check: cell(&gk, r, K_CHECK),
                    info: String::new(),
                },
            });
        }
        for (r, ex) in rows.idx.iter().enumerate() {
            match ex {
                Some(i) => t.indexes.push(i.clone()),
                None if cell(&gi, r, I_COLS).is_empty() => {}
                None => t.indexes.push(IndexDef { existing: false, name: cell(&gi, r, I_NAME), columns: cell(&gi, r, I_COLS), unique: cell(&gi, r, I_UNIQUE) == "1", info: String::new() }),
            }
        }
        match old3.as_ref() {
            Some(o) => ddl::alter_table(db, o, &t),
            None => ddl::create_table(db, &t),
        }
    });
    let script = move |s: &[String]| -> String {
        // SQL Server batches (the default-constraint lookup) need GO, not ;.
        if db == Kind::Mssql { s.join("\nGO\n") + "\nGO\n" } else { s.iter().map(|x| format!("{x};\n")).collect::<Vec<_>>().join("\n") }
    };
    let b2 = build.clone();
    let show_ddl = move || {
        ddl_text.set_value(&match b2() {
            Ok(s) if s.is_empty() => "-- No changes.".into(),
            Ok(s) => script(&s),
            Err(e) => format!("-- Cannot generate the DDL yet:\n-- {e}"),
        });
    };
    let sd = show_ddl.clone();
    nb.on_page_changed(move |e| {
        if e.get_selection() == Some(3) {
            sd();
        }
    });

    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    let copy = Button::builder(&dlg).with_label("Copy to Worksheet").build();
    let cancel = Button::builder(&dlg).with_label("Cancel").with_id(ID_CANCEL).build();
    let ok = Button::builder(&dlg).with_label("OK").with_id(ID_OK).build();
    bs.add(&copy, 0, SizerFlag::All, 4);
    bs.add_stretch_spacer(1);
    bs.add(&cancel, 0, SizerFlag::All, 4);
    bs.add(&ok, 0, SizerFlag::All, 4);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&top, 0, SizerFlag::Expand | SizerFlag::All, 6);
    s.add(&nb, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer(s, true);

    // Statements to run once the dialog has closed (OK), or to open in a worksheet (Copy).
    let picked: Rc<RefCell<Option<(bool, Vec<String>)>>> = Rc::default();
    let (p1, b3) = (picked.clone(), build.clone());
    copy.on_click(move |_| match b3() {
        Ok(s) if !s.is_empty() => {
            *p1.borrow_mut() = Some((false, s));
            dlg.end_modal(ID_OK);
        }
        Ok(_) => crate::ui::info(&dlg, "No changes."),
        Err(e) => crate::ui::error(&dlg, &e),
    });
    let (p2, b4, conn_name) = (picked.clone(), build.clone(), conn.borrow().cfg.name.clone());
    ok.on_click(move |_| match b4() {
        Ok(s) if s.is_empty() => crate::ui::info(&dlg, "No changes."),
        Ok(s) => {
            if crate::ui::confirm(&dlg, &format!("Run on {conn_name}?\n\n{}", script(&s))) {
                *p2.borrow_mut() = Some((true, s));
                dlg.end_modal(ID_OK);
            }
        }
        Err(e) => crate::ui::error(&dlg, &e),
    });
    dlg.show_modal();
    dlg.destroy();
    match picked.take() {
        Some((true, stmts)) => run(conn, stmts, 0, done),
        Some((false, stmts)) => {
            let app = crate::app();
            app.open_worksheet(conn);
            if let Some(ws) = app.current_sheet() {
                ws.editor.set_text(&script(&stmts));
            }
        }
        None => {}
    }
}

/// Run the statements in order on the worksheet session; stop at the first error.
fn run(conn: ConnRef, stmts: Vec<String>, i: usize, done: Rc<dyn Fn()>) {
    if i >= stmts.len() {
        crate::app().status(0, &format!("{} statement(s) run", stmts.len()));
        return done();
    }
    let c = conn.clone();
    let sql = stmts[i].clone();
    Conn::work_query(&conn, &sql, move |r| match r {
        Ok(_) => run(c, stmts, i + 1, done),
        Err(e) => {
            let left = stmts.len() - i - 1;
            crate::ui::error(&crate::app().frame, &format!("{e}\n\nStatement:\n{sql}{}", if left > 0 { format!("\n\n{left} later statement(s) were not run.") } else { String::new() }));
            done();
        }
    });
}
