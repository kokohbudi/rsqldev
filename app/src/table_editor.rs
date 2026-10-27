//! Create Table / Edit Table dialog, after SQL Developer's: Columns (with Column Properties),
//! Primary Key, Unique Constraints, Foreign Keys, Check Constraints, Indexes, Storage Options
//! (Oracle) and a DDL tab showing exactly what will run. OK runs it (after confirming), Copy to
//! Worksheet opens it for running by hand.

use crate::conn::{Conn, ConnRef, Res};
use rsqldev_core::ddl::{self, ColumnDef, ConKind, ConstraintDef, Identity, IndexDef, Storage, TableDef};
use rsqldev_core::dialect;
use rsqldev_core::registry::Kind;
use std::cell::{Cell, RefCell};
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
// Unique / check grids: name, columns or condition, enabled, existing.
const K_NAME: i32 = 0;
const K_BODY: i32 = 1;
const K_ENABLED: i32 = 2;
const K_INFO: i32 = 3;
// Foreign keys grid.
const F_NAME: i32 = 0;
const F_COLS: i32 = 1;
const F_REF: i32 = 2;
const F_REFCOLS: i32 = 3;
const F_DELETE: i32 = 4;
const F_ENABLED: i32 = 5;
const F_INFO: i32 = 6;
// Indexes grid.
const I_NAME: i32 = 0;
const I_COLS: i32 = 1;
const I_UNIQUE: i32 = 2;
const I_INFO: i32 = 3;

/// Column facts kept beside the grid cells (Column Properties panel).
#[derive(Default, Clone)]
struct ColExtra {
    orig: Option<String>,
    identity: Option<Identity>,
    virtual_expr: String,
    default_on_null: bool,
}

/// Rows of the grids beyond what the cells hold: column extras and the loaded constraint /
/// index each row stands for.
#[derive(Default)]
struct Rows {
    cols: Vec<ColExtra>,
    uk: Vec<Option<ConstraintDef>>,
    fk: Vec<Option<ConstraintDef>>,
    ck: Vec<Option<ConstraintDef>>,
    idx: Vec<Option<IndexDef>>,
    pk_order: Vec<String>,
}

type QRows = Vec<Vec<Option<String>>>;

/// Run the queries one after another on the metadata lane (None = no query, empty result).
fn fetch_all(c: ConnRef, mut qs: Vec<Option<String>>, mut got: Vec<QRows>, done: Box<dyn FnOnce(Result<Vec<QRows>, String>)>) {
    if qs.is_empty() {
        return done(Ok(got));
    }
    match qs.remove(0) {
        None => {
            got.push(vec![]);
            fetch_all(c, qs, got, done)
        }
        Some(sql) => {
            let c2 = c.clone();
            Conn::meta_query(&c, &sql, move |r: Result<Res, String>| match r {
                Ok(r) => {
                    got.push(r.rows);
                    fetch_all(c2, qs, got, done)
                }
                Err(e) => done(Err(e)),
            });
        }
    }
}

/// Tables ▸ New Table…
pub fn create(conn: ConnRef, owner: String, done: impl Fn() + 'static) {
    dialog(conn, owner, None, Start::Columns, Rc::new(done));
}

/// Table ▸ Edit Table… / New Index…: load the table, then open the dialog.
pub fn edit(conn: ConnRef, owner: String, name: String, start: Start, done: impl Fn() + 'static) {
    let db = conn.borrow().kind;
    let Some(q) = ddl::load_sql(db, &owner, &name) else { return };
    let done: Rc<dyn Fn()> = Rc::new(done);
    let c2 = conn.clone();
    fetch_all(
        conn,
        vec![Some(q.columns), q.constraints, q.indexes, q.extra, q.comment],
        vec![],
        Box::new(move |r| match r {
            Ok(r) => {
                let mut t = ddl::from_rows(db, &owner, &name, &r[0], &r[1], &r[2]);
                ddl::apply_extra(&mut t, &r[3]);
                t.comment = r[4].first().and_then(|row| row.first().cloned().flatten()).unwrap_or_default();
                dialog(c2, owner, Some(t), start, done);
            }
            Err(e) => crate::ui::error(&crate::app().frame, &e),
        }),
    );
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
fn layout(nb: &Notebook, p: Panel, title: &str, g: Grid, bar: BoxSizer, note: Option<&str>) {
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
    if let Some(n) = note {
        let t = StaticText::builder(&p).with_label(n).build();
        t.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        s.add(&t, 0, SizerFlag::All, 4);
    }
    s.add(&g, 1, SizerFlag::Expand, 0);
    p.set_sizer(s, true);
    nb.add_page(&p, title, false, None);
}

fn yes(v: bool) -> &'static str {
    if v { "1" } else { "" }
}

/// A loaded row: grey and read-only, except the cells listed in `open` (rename, Enabled).
fn lock_row(g: &Grid, r: i32, open: &[i32]) {
    let grey = crate::theme::c(crate::theme::p().muted);
    for c in (0..g.get_number_cols()).filter(|c| !open.contains(c)) {
        g.set_read_only(r, c, true);
        g.set_cell_text_colour(r, c, &grey);
    }
}

fn label(p: &impl WxWidget, text: &str) -> StaticText {
    StaticText::builder(p).with_label(text).build()
}

/// Available ↔ Selected list pair with Add / Remove / Move Up / Move Down (Primary Key page and
/// the column pickers). `order` adds an ASC / DESC toggle for index columns.
#[derive(Clone, Copy)]
struct Shuttle {
    avail: ListBox,
    sel: ListBox,
}

impl Shuttle {
    fn build(parent: &impl WxWidget, order: bool, changed: Rc<dyn Fn()>) -> (Shuttle, BoxSizer) {
        let row = BoxSizer::builder(Orientation::Horizontal).build();
        let avail = ListBox::builder(parent).build();
        let sel = ListBox::builder(parent).build();
        avail.set_min_size(Size::new(200, 220));
        sel.set_min_size(Size::new(200, 220));
        let left = BoxSizer::builder(Orientation::Vertical).build();
        left.add(&label(parent, "Available"), 0, SizerFlag::Bottom, 3);
        left.add(&avail, 1, SizerFlag::Expand, 0);
        let right = BoxSizer::builder(Orientation::Vertical).build();
        right.add(&label(parent, "Selected"), 0, SizerFlag::Bottom, 3);
        right.add(&sel, 1, SizerFlag::Expand, 0);
        let mid = BoxSizer::builder(Orientation::Vertical).build();
        let sh = Shuttle { avail, sel };
        let mut labels = vec!["Add >", "< Remove", "Move Up", "Move Down"];
        if order {
            labels.push("ASC / DESC");
        }
        for (i, l) in labels.into_iter().enumerate() {
            let b = Button::builder(parent).with_label(l).build();
            mid.add(&b, 0, SizerFlag::All | SizerFlag::Expand, 3);
            let ch = changed.clone();
            b.on_click(move |_| {
                match i {
                    0 => sh.add(),
                    1 => sh.remove(),
                    2 | 3 => sh.step(i == 2),
                    _ => sh.toggle_order(),
                }
                ch();
            });
        }
        let ch = changed.clone();
        avail.on_item_double_clicked(move |_| {
            sh.add();
            ch();
        });
        sel.on_item_double_clicked(move |_| {
            sh.remove();
            changed();
        });
        row.add_sizer(&left, 1, SizerFlag::Expand | SizerFlag::All, 4);
        row.add_sizer(&mid, 0, SizerFlag::AlignCenterVertical, 0);
        row.add_sizer(&right, 1, SizerFlag::Expand | SizerFlag::All, 4);
        (sh, row)
    }

    fn items(l: ListBox) -> Vec<String> {
        (0..l.get_count()).filter_map(|i| l.get_string(i)).collect()
    }

    fn selected(&self) -> Vec<String> {
        Self::items(self.sel)
    }

    /// Show `chosen` on the right and the other `all` names on the left.
    fn set(&self, all: &[String], chosen: &[String]) {
        let bare = |s: &str| s.strip_suffix(" DESC").unwrap_or(s).to_string();
        let left: Vec<&str> = all.iter().filter(|a| !chosen.iter().any(|c| bare(c) == **a)).map(String::as_str).collect();
        self.avail.set_items(&left);
        self.sel.set_items(&chosen.iter().map(String::as_str).collect::<Vec<_>>());
    }

    fn add(&self) {
        if let Some(i) = self.avail.get_selection() {
            if let Some(s) = self.avail.get_string(i) {
                self.avail.delete(i);
                self.sel.append(&s);
            }
        }
    }

    fn remove(&self) {
        if let Some(i) = self.sel.get_selection() {
            if let Some(s) = self.sel.get_string(i) {
                self.sel.delete(i);
                self.avail.append(s.strip_suffix(" DESC").unwrap_or(&s));
            }
        }
    }

    fn step(&self, up: bool) {
        let Some(i) = self.sel.get_selection() else { return };
        let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < self.sel.get_count()) };
        let Some(j) = j else { return };
        let (a, b) = (self.sel.get_string(i).unwrap_or_default(), self.sel.get_string(j).unwrap_or_default());
        self.sel.set_string(i, &b);
        self.sel.set_string(j, &a);
        self.sel.set_selection(j, true);
    }

    fn toggle_order(&self) {
        let Some(i) = self.sel.get_selection() else { return };
        let s = self.sel.get_string(i).unwrap_or_default();
        self.sel.set_string(i, &match s.strip_suffix(" DESC") {
            Some(b) => b.to_string(),
            None => format!("{s} DESC"),
        });
    }
}

/// Column picker dialog: returns the chosen columns, comma-joined (None = cancelled).
fn pick_columns(parent: &impl WxWidget, title: &str, all: &[String], current: &str, order: bool) -> Option<String> {
    let dlg = Dialog::builder(parent, title).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).build();
    let chosen: Vec<String> = current.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    let (sh, row) = Shuttle::build(&dlg, order, Rc::new(|| {}));
    sh.set(all, &chosen);
    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    bs.add_stretch_spacer(1);
    bs.add(&Button::builder(&dlg).with_label("Cancel").with_id(ID_CANCEL).build(), 0, SizerFlag::All, 4);
    bs.add(&Button::builder(&dlg).with_label("OK").with_id(ID_OK).build(), 0, SizerFlag::All, 4);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&row, 1, SizerFlag::Expand | SizerFlag::All, 6);
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer_and_fit(s, true);
    let out = (dlg.show_modal() == ID_OK).then(|| sh.selected().join(", "));
    dlg.destroy();
    out
}

/// Names typed in the Columns grid, in order.
fn col_names(gc: Grid) -> Vec<String> {
    (0..gc.get_number_rows()).map(|r| gc.get_cell_value(r, C_NAME).trim().to_string()).filter(|n| !n.is_empty()).collect()
}

/// Size without a BYTE / CHAR unit, and the unit.
fn split_units(size: &str) -> (String, &'static str) {
    let s = size.trim();
    for u in ["BYTE", "CHAR"] {
        if let Some(n) = s.strip_suffix(u) {
            return (n.trim().to_string(), u);
        }
    }
    (s.to_string(), "")
}

fn dialog(conn: ConnRef, owner: String, old: Option<TableDef>, start: Start, done: Rc<dyn Fn()>) {
    let db = conn.borrow().kind;
    let oracle = db == Kind::Oracle;
    let frame = crate::app().frame;
    let editing = old.is_some();
    let title = match &old {
        Some(t) => format!("Edit Table {}", t.name),
        None => "Create Table".into(),
    };
    let dlg = Dialog::builder(&frame, &title).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(1040, 640).build();

    // Header: Schema, Name, Temporary Table, Comment.
    let top = FlexGridSizer::builder(0, 4).with_vgap(4).with_hgap(8).build();
    top.add_growable_col(3, 1);
    let schema = TextCtrl::builder(&dlg).with_value(&owner).build();
    schema.enable(!editing);
    schema.set_min_size(Size::new(180, -1));
    let name = TextCtrl::builder(&dlg).with_value(old.as_ref().map_or("", |t| t.name.as_str())).build();
    name.set_min_size(Size::new(260, -1));
    // Rename lives in the navigator (Rename…): the dialog edits one table.
    name.enable(!editing);
    top.add(&label(&dlg, "Schema:"), 0, SizerFlag::AlignCenterVertical, 0);
    top.add(&schema, 0, SizerFlag::empty(), 0);
    top.add(&label(&dlg, "Name:"), 0, SizerFlag::AlignCenterVertical, 0);
    top.add(&name, 0, SizerFlag::empty(), 0);
    let temp = CheckBox::builder(&dlg).with_label("Temporary Table").build();
    let temp_kind = Choice::builder(&dlg).with_choices(vec!["Session".into(), "Transaction".into()]).build();
    temp_kind.set_selection(0);
    let temp_ok = !editing && matches!(db, Kind::Oracle | Kind::Postgres | Kind::Mysql | Kind::Sqlite | Kind::Duckdb);
    temp.show(temp_ok);
    temp_kind.show(temp_ok && matches!(db, Kind::Oracle | Kind::Postgres));
    let temp_row = BoxSizer::builder(Orientation::Horizontal).build();
    temp_row.add(&temp, 0, SizerFlag::AlignCenterVertical, 0);
    temp_row.add(&temp_kind, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 6);
    let comment = TextCtrl::builder(&dlg).with_value(old.as_ref().map_or("", |t| t.comment.as_str())).build();
    comment.enable(ddl::supports_table_comment(db));
    top.add(&label(&dlg, ""), 0, SizerFlag::empty(), 0);
    top.add_sizer(&temp_row, 0, SizerFlag::empty(), 0);
    top.add(&label(&dlg, "Comment:"), 0, SizerFlag::AlignCenterVertical, 0);
    top.add(&comment, 1, SizerFlag::Expand, 0);
    let nb = Notebook::builder(&dlg).build();
    let mut pages: Vec<&str> = vec![];

    // Columns + Column Properties.
    let cp = Panel::builder(&nb).build();
    let (cbar, cb) = buttons(&cp, &["Add Column", "Remove", "Move Up", "Move Down"]);
    let gc = grid(&cp, &[("Name", 150), ("Data Type", 140), ("Size", 70), ("Scale", 50), ("Not Null", 60), ("Default", 120), ("PK", 34), ("Comment", 140)]);
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
    let props = Panel::builder(&cp).build();
    let pg = FlexGridSizer::builder(0, 2).with_vgap(4).with_hgap(6).build();
    pg.add_growable_col(1, 1);
    let text = |w: i32| {
        let t = TextCtrl::builder(&props).build();
        t.set_min_size(Size::new(w, -1));
        t
    };
    let units = Choice::builder(&props).with_choices(vec!["".into(), "BYTE".into(), "CHAR".into()]).build();
    let on_null = CheckBox::builder(&props).with_label("Default on NULL").build();
    let virt = text(150);
    virt.set_tooltip("Virtual / generated column: the expression (leave Default empty)");
    let kinds = ddl::identity_kinds(db);
    let ident_kind = Choice::builder(&props).with_choices(std::iter::once("(none)".to_string()).chain(kinds.iter().map(|k| k.to_string())).collect()).build();
    let (start_w, inc_w, min_w, max_w, cache_w) = (text(80), text(80), text(80), text(80), text(80));
    cache_w.set_tooltip("Cache size, or NOCACHE");
    let cycle = CheckBox::builder(&props).with_label("Cycle").build();
    let order = CheckBox::builder(&props).with_label("Order").build();
    let head = label(&props, "Column Properties");
    let ps = BoxSizer::builder(Orientation::Vertical).build();
    ps.add(&head, 0, SizerFlag::All, 4);
    let seq = matches!(db, Kind::Oracle | Kind::Postgres);
    // Label + widget rows; fields the engine has no use for are hidden.
    macro_rules! prop {
        ($l:expr, $w:expr, $show:expr) => {
            if $show {
                pg.add(&label(&props, $l), 0, SizerFlag::AlignCenterVertical, 0);
                pg.add(&$w, 0, SizerFlag::Expand, 0);
            } else {
                $w.show(false);
            }
        };
    }
    prop!("Units:", units, oracle);
    prop!("", on_null, oracle);
    prop!("Virtual:", virt, true);
    prop!("Identity:", ident_kind, !kinds.is_empty());
    prop!("Start With:", start_w, seq || db == Kind::Mssql);
    prop!("Increment By:", inc_w, seq || db == Kind::Mssql);
    prop!("Min Value:", min_w, seq);
    prop!("Max Value:", max_w, seq);
    prop!("Cache:", cache_w, seq);
    prop!("", cycle, seq);
    prop!("", order, oracle);
    ps.add_sizer(&pg, 0, SizerFlag::Expand | SizerFlag::All, 4);
    props.set_sizer(ps, true);
    let cs = BoxSizer::builder(Orientation::Vertical).build();
    cs.add_sizer(&cbar, 0, SizerFlag::Expand, 0);
    let body = BoxSizer::builder(Orientation::Horizontal).build();
    body.add(&gc, 1, SizerFlag::Expand, 0);
    body.add(&props, 0, SizerFlag::Expand | SizerFlag::Left, 6);
    cs.add_sizer(&body, 1, SizerFlag::Expand, 0);
    cp.set_sizer(cs, true);
    nb.add_page(&cp, "Columns", true, None);
    pages.push("Columns");

    // Primary Key.
    let pp = Panel::builder(&nb).build();
    let pk_name = TextCtrl::builder(&pp).with_value(&old.as_ref().map(|t| if t.pk_name.is_empty() { String::new() } else { ddl::shown(db, &t.pk_name) }).unwrap_or_default()).build();
    pk_name.set_min_size(Size::new(260, -1));
    let rows = Rc::new(RefCell::new(Rows::default()));
    let rs = rows.clone();
    let apply_pk: Rc<RefCell<Option<Shuttle>>> = Rc::default();
    let ap = apply_pk.clone();
    // Shuttle changes write the PK flags and order back to the Columns grid.
    let (pk_sh, pk_row) = Shuttle::build(
        &pp,
        false,
        Rc::new(move || {
            let Some(sh) = *ap.borrow() else { return };
            let sel = sh.selected();
            for r in 0..gc.get_number_rows() {
                gc.set_cell_value(r, C_PK, yes(sel.contains(&gc.get_cell_value(r, C_NAME).trim().to_string())));
            }
            rs.borrow_mut().pk_order = sel;
        }),
    );
    *apply_pk.borrow_mut() = Some(pk_sh);
    let pps = BoxSizer::builder(Orientation::Vertical).build();
    let pn = BoxSizer::builder(Orientation::Horizontal).build();
    pn.add(&label(&pp, "Name:"), 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 6);
    pn.add(&pk_name, 0, SizerFlag::AlignCenterVertical, 0);
    pps.add_sizer(&pn, 0, SizerFlag::empty(), 0);
    pps.add_sizer(&pk_row, 1, SizerFlag::Expand, 0);
    pp.set_sizer(pps, true);
    nb.add_page(&pp, "Primary Key", false, None);
    pages.push("Primary Key");
    let rs = rows.clone();
    // Selected = the PK flags of the Columns grid, in the order chosen here.
    let refresh_pk = move || {
        gc.save_edit_control_value();
        let names = col_names(gc);
        let flagged: Vec<String> = (0..gc.get_number_rows()).filter(|&r| gc.get_cell_value(r, C_PK) == "1").map(|r| gc.get_cell_value(r, C_NAME).trim().to_string()).filter(|n| !n.is_empty()).collect();
        let mut ord: Vec<String> = rs.borrow().pk_order.iter().filter(|n| flagged.contains(n)).cloned().collect();
        ord.extend(flagged.into_iter().filter(|n| !ord.contains(n)).collect::<Vec<_>>());
        pk_sh.set(&names, &ord);
        rs.borrow_mut().pk_order = ord;
    };

    // Unique / Foreign Keys / Check pages.
    let can_constraints = !editing || ddl::alters_constraints(db);
    let knote = (!can_constraints).then_some("This database cannot add or drop constraints on an existing table.");
    let up = Panel::builder(&nb).build();
    let (ubar, ub) = buttons(&up, &["Add", "Remove", "Columns…"]);
    let gu = grid(&up, &[("Name", 200), ("Columns", 260), ("Enabled", 60), ("Existing", 360)]);
    gu.set_col_format_bool(K_ENABLED);
    layout(&nb, up, "Unique Constraints", gu, ubar, knote);
    pages.push("Unique Constraints");
    let fp = Panel::builder(&nb).build();
    let (fbar, fb) = buttons(&fp, &["Add", "Remove", "Columns…", "Referenced Columns…"]);
    let gf = grid(&fp, &[("Name", 150), ("Columns", 140), ("Referenced Table", 170), ("Referenced Columns", 140), ("On Delete", 90), ("Enabled", 60), ("Existing", 230)]);
    gf.set_col_format_custom(F_DELETE, "choice:NO ACTION,CASCADE,SET NULL");
    gf.set_col_format_bool(F_ENABLED);
    layout(&nb, fp, "Foreign Keys", gf, fbar, knote);
    pages.push("Foreign Keys");
    let kp = Panel::builder(&nb).build();
    let (kbar, kb) = buttons(&kp, &["Add", "Remove"]);
    let gk = grid(&kp, &[("Name", 200), ("Condition", 320), ("Enabled", 60), ("Existing", 300)]);
    gk.set_col_format_bool(K_ENABLED);
    layout(&nb, kp, "Check Constraints", gk, kbar, knote);
    pages.push("Check Constraints");

    // Indexes.
    let ip = Panel::builder(&nb).build();
    let (ibar, ib) = buttons(&ip, &["Add Index", "Remove", "Columns…"]);
    let gi = grid(&ip, &[("Name", 200), ("Index Expressions (a, b DESC)", 280), ("Unique", 60), ("Existing", 320)]);
    gi.set_col_format_bool(I_UNIQUE);
    layout(&nb, ip, "Indexes", gi, ibar, None);
    pages.push("Indexes");

    // Storage Options (Oracle).
    let sp = Panel::builder(&nb).build();
    let sg = FlexGridSizer::builder(0, 2).with_vgap(6).with_hgap(8).build();
    let ts = Choice::builder(&sp).with_choices(vec![String::new()]).build();
    ts.set_min_size(Size::new(220, -1));
    let pct_free = TextCtrl::builder(&sp).build();
    let ini_trans = TextCtrl::builder(&sp).build();
    let logging = Choice::builder(&sp).with_choices(vec!["".into(), "LOGGING".into(), "NOLOGGING".into()]).build();
    let compress = Choice::builder(&sp).with_choices(vec!["".into(), "COMPRESS".into(), "NOCOMPRESS".into()]).build();
    macro_rules! srow {
        ($l:expr, $w:expr) => {
            sg.add(&label(&sp, $l), 0, SizerFlag::AlignCenterVertical, 0);
            sg.add(&$w, 0, SizerFlag::empty(), 0);
        };
    }
    srow!("Tablespace:", ts);
    srow!("Pct Free:", pct_free);
    srow!("Ini Trans:", ini_trans);
    srow!("Logging:", logging);
    srow!("Compression:", compress);
    let sps = BoxSizer::builder(Orientation::Vertical).build();
    sps.add_sizer(&sg, 0, SizerFlag::All, 10);
    let snote = label(&sp, if editing { "Only filled fields change. A new tablespace moves the table (MOVE) and rebuilds its indexes." } else { "Empty fields use the database defaults." });
    snote.set_foreground_color(crate::theme::c(crate::theme::p().muted));
    sps.add(&snote, 0, SizerFlag::All, 10);
    sp.set_sizer(sps, true);
    if oracle {
        nb.add_page(&sp, "Storage Options", false, None);
        pages.push("Storage Options");
        let ts2 = ts;
        Conn::meta_query(&conn, "SELECT tablespace_name FROM user_tablespaces ORDER BY 1", move |r| {
            if let (Ok(r), true) = (r, ts2.is_valid()) {
                for row in r.rows {
                    if let Some(Some(n)) = row.into_iter().next() {
                        ts2.append(&n);
                    }
                }
            }
        });
    } else {
        sp.show(false);
    }

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
    pages.push("DDL");
    let page = |n: &str| pages.iter().position(|p| *p == n).unwrap() as i32;
    let (pk_page, ddl_page, idx_page) = (page("Primary Key"), page("DDL"), page("Indexes"));

    // Loaded definition into the grids.
    let old = Rc::new(old);
    if let Some(t) = old.as_ref() {
        for (r, c) in t.columns.iter().enumerate() {
            let r = r as i32;
            gc.append_rows(1, false);
            for (col, v) in [(C_NAME, c.name.as_str()), (C_TYPE, c.ty.as_str()), (C_SIZE, c.size.as_str()), (C_SCALE, c.scale.as_str()), (C_NOTNULL, yes(c.not_null)), (C_DEFAULT, c.default.as_str()), (C_PK, yes(c.pk)), (C_COMMENT, c.comment.as_str())] {
                gc.set_cell_value(r, col, v);
            }
            rows.borrow_mut().cols.push(ColExtra { orig: c.orig.clone(), identity: c.identity.clone(), virtual_expr: c.virtual_expr.clone(), default_on_null: c.default_on_null });
        }
        for c in &t.constraints {
            let (g, open) = match c.kind {
                ConKind::Unique => (gu, [K_NAME, K_ENABLED]),
                ConKind::Foreign => (gf, [F_NAME, F_ENABLED]),
                ConKind::Check => (gk, [K_NAME, K_ENABLED]),
            };
            let r = g.get_number_rows();
            g.append_rows(1, false);
            g.set_cell_value(r, 0, &c.name);
            let (en, info) = if c.kind == ConKind::Foreign { (F_ENABLED, F_INFO) } else { (K_ENABLED, K_INFO) };
            g.set_cell_value(r, en, yes(c.enabled));
            g.set_cell_value(r, info, &c.info);
            lock_row(&g, r, &open);
            let mut rw = rows.borrow_mut();
            match c.kind {
                ConKind::Unique => rw.uk.push(Some(c.clone())),
                ConKind::Foreign => rw.fk.push(Some(c.clone())),
                ConKind::Check => rw.ck.push(Some(c.clone())),
            }
        }
        for (r, i) in t.indexes.iter().enumerate() {
            gi.append_rows(1, false);
            gi.set_cell_value(r as i32, I_NAME, &i.name);
            gi.set_cell_value(r as i32, I_UNIQUE, yes(i.unique));
            gi.set_cell_value(r as i32, I_INFO, &i.info);
            lock_row(&gi, r as i32, &[]);
            rows.borrow_mut().idx.push(Some(i.clone()));
        }
    }
    let tname = {
        let old = old.clone();
        move || old.as_ref().as_ref().map_or_else(|| name.get_value(), |t| t.name.clone())
    };
    let new_index = {
        let (rs, tname) = (rows.clone(), tname.clone());
        move || {
            let n = gi.get_number_rows();
            gi.append_rows(1, false);
            gi.set_cell_value(n, I_NAME, &format!("{}_IX{}", tname().trim_matches('"'), n + 1));
            rs.borrow_mut().idx.push(None);
            gi.set_grid_cursor(n, I_COLS);
        }
    };
    if start == Start::Indexes {
        new_index();
        nb.set_selection(idx_page as usize);
    }

    // Column Properties follow the cursor row.
    let sel = Rc::new(Cell::new(-1i32));
    let rs = rows.clone();
    let load_props = Rc::new(move |r: i32| {
        let rw = rs.borrow();
        let Some(x) = rw.cols.get(r.max(0) as usize).filter(|_| r >= 0) else {
            props.enable(false);
            return;
        };
        props.enable(true);
        head.set_label(&format!("Column Properties: {}", gc.get_cell_value(r, C_NAME)));
        let (_, u) = split_units(&gc.get_cell_value(r, C_SIZE));
        units.set_selection(["", "BYTE", "CHAR"].iter().position(|x| *x == u).unwrap_or(0) as u32);
        on_null.set_value(x.default_on_null);
        virt.set_value(&x.virtual_expr);
        let id = x.identity.clone().unwrap_or_default();
        ident_kind.set_selection(x.identity.as_ref().and_then(|i| kinds.iter().position(|k| *k == i.generate)).map_or(0, |p| p + 1) as u32);
        for (w, v) in [(start_w, &id.start), (inc_w, &id.increment), (min_w, &id.min), (max_w, &id.max), (cache_w, &id.cache)] {
            w.set_value(v);
        }
        cycle.set_value(id.cycle);
        order.set_value(id.order);
    });
    let rs = rows.clone();
    let save_props = Rc::new(move |r: i32| {
        let mut rw = rs.borrow_mut();
        let Some(x) = rw.cols.get_mut(r.max(0) as usize).filter(|_| r >= 0) else { return };
        let (n, _) = split_units(&gc.get_cell_value(r, C_SIZE));
        let u = units.get_string_selection().unwrap_or_default();
        if oracle {
            gc.set_cell_value(r, C_SIZE, &if n.is_empty() || u.is_empty() { n } else { format!("{n} {u}") });
        }
        x.default_on_null = on_null.get_value();
        x.virtual_expr = virt.get_value().trim().to_string();
        let g = ident_kind.get_selection().unwrap_or(0) as usize;
        let id = Identity {
            generate: kinds.get(g.wrapping_sub(1)).map(|k| k.to_string()).unwrap_or_default(),
            start: start_w.get_value().trim().into(),
            increment: inc_w.get_value().trim().into(),
            min: min_w.get_value().trim().into(),
            max: max_w.get_value().trim().into(),
            cycle: cycle.get_value(),
            cache: cache_w.get_value().trim().into(),
            order: order.get_value(),
        };
        // Unchanged loaded identity keeps its loaded value (no ALTER).
        x.identity = match (g, &x.identity) {
            (0, _) => None,
            (_, Some(o)) if *o == id => Some(o.clone()),
            _ => Some(id),
        };
    });
    props.enable(false);
    let (lp, sp2, sel2) = (load_props.clone(), save_props.clone(), sel.clone());
    gc.on_select_cell(move |e| {
        let r = e.get_row();
        if r != sel2.get() {
            sp2(sel2.get());
            sel2.set(r);
            lp(r);
        }
    });

    // Columns buttons.
    let (rs, lp, sp2, sel2) = (rows.clone(), load_props.clone(), save_props.clone(), sel.clone());
    cb[0].on_click(move |_| {
        sp2(sel2.get());
        let n = gc.get_number_rows();
        gc.append_rows(1, false);
        gc.set_cell_value(n, C_TYPE, ddl::types(db)[0]);
        rs.borrow_mut().cols.push(ColExtra::default());
        gc.set_grid_cursor(n, C_NAME);
        sel2.set(n);
        lp(n);
    });
    let (rs, lp, sel2) = (rows.clone(), load_props.clone(), sel.clone());
    cb[1].on_click(move |_| {
        let r = gc.get_grid_cursor_row();
        if r >= 0 && r < gc.get_number_rows() {
            gc.delete_rows(r, 1, false);
            rs.borrow_mut().cols.remove(r as usize);
            let r = gc.get_grid_cursor_row().min(gc.get_number_rows() - 1);
            sel2.set(r);
            lp(r);
        }
    });
    for (b, up) in [(cb[2], true), (cb[3], false)] {
        let (rs, lp, sp2, sel2) = (rows.clone(), load_props.clone(), save_props.clone(), sel.clone());
        b.on_click(move |_| {
            let r = gc.get_grid_cursor_row();
            let o = if up { r - 1 } else { r + 1 };
            if r < 0 || o < 0 || o >= gc.get_number_rows() {
                return;
            }
            gc.save_edit_control_value();
            sp2(r);
            for c in 0..gc.get_number_cols() {
                let (a, b) = (gc.get_cell_value(r, c), gc.get_cell_value(o, c));
                gc.set_cell_value(r, c, &b);
                gc.set_cell_value(o, c, &a);
            }
            rs.borrow_mut().cols.swap(r as usize, o as usize);
            gc.set_grid_cursor(o, gc.get_grid_cursor_col());
            sel2.set(o);
            lp(o);
        });
    }

    // Constraint page buttons: Add, Remove, column pickers.
    for b in ub.iter().chain(&fb).chain(&kb) {
        b.enable(can_constraints);
    }
    let add_row = |g: Grid, which: fn(&mut Rows) -> &mut Vec<Option<ConstraintDef>>, en: i32, rs: Rc<RefCell<Rows>>| {
        move |_| {
            let n = g.get_number_rows();
            g.append_rows(1, false);
            g.set_cell_value(n, en, "1");
            which(&mut rs.borrow_mut()).push(None);
            g.set_grid_cursor(n, 0);
        }
    };
    let remove_row = |g: Grid, which: fn(&mut Rows) -> &mut Vec<Option<ConstraintDef>>, rs: Rc<RefCell<Rows>>| {
        move |_| {
            let r = g.get_grid_cursor_row();
            if r >= 0 && r < g.get_number_rows() {
                g.delete_rows(r, 1, false);
                which(&mut rs.borrow_mut()).remove(r as usize);
            }
        }
    };
    ub[0].on_click(add_row(gu, |r| &mut r.uk, K_ENABLED, rows.clone()));
    ub[1].on_click(remove_row(gu, |r| &mut r.uk, rows.clone()));
    fb[0].on_click(add_row(gf, |r| &mut r.fk, F_ENABLED, rows.clone()));
    fb[1].on_click(remove_row(gf, |r| &mut r.fk, rows.clone()));
    kb[0].on_click(add_row(gk, |r| &mut r.ck, K_ENABLED, rows.clone()));
    kb[1].on_click(remove_row(gk, |r| &mut r.ck, rows.clone()));
    // Columns… on a new row: pick from the table's columns.
    let picker = |g: Grid, col: i32, order: bool, title: &'static str, rs: Rc<RefCell<Rows>>, which: fn(&Rows) -> Vec<bool>| {
        move |_| {
            let r = g.get_grid_cursor_row();
            if r < 0 || which(&rs.borrow()).get(r as usize).copied().unwrap_or(true) {
                return;
            }
            g.save_edit_control_value();
            if let Some(v) = pick_columns(&dlg, title, &col_names(gc), &g.get_cell_value(r, col), order) {
                g.set_cell_value(r, col, &v);
            }
        }
    };
    ub[2].on_click(picker(gu, K_BODY, false, "Unique Constraint Columns", rows.clone(), |r| r.uk.iter().map(Option::is_some).collect()));
    fb[2].on_click(picker(gf, F_COLS, false, "Foreign Key Columns", rows.clone(), |r| r.fk.iter().map(Option::is_some).collect()));
    ib[2].on_click(picker(gi, I_COLS, true, "Index Columns", rows.clone(), |r| r.idx.iter().map(Option::is_some).collect()));
    // Referenced Columns…: the referenced table's columns, fetched first.
    let (rs, c2, own) = (rows.clone(), conn.clone(), owner.clone());
    fb[3].on_click(move |_| {
        let r = gf.get_grid_cursor_row();
        if r < 0 || rs.borrow().fk.get(r as usize).is_none_or(Option::is_some) {
            return;
        }
        gf.save_edit_control_value();
        let rt = gf.get_cell_value(r, F_REF);
        let (o, t) = rt.trim().split_once('.').map_or((own.clone(), rt.trim().to_string()), |(o, t)| (o.to_string(), t.to_string()));
        if t.is_empty() {
            return crate::ui::info(&dlg, "Choose the Referenced Table first.");
        }
        let Some(sql) = dialect::children_sql(db, dialect::TABLE) else { return };
        let sql = dialect::fill(db, &sql, &ddl::ident(db, &o), &ddl::ident(db, &t));
        Conn::meta_query(&c2, &sql, move |res| {
            let cols: Vec<String> = match res {
                Ok(res) => res.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).map(|n| ddl::shown(db, &n)).collect(),
                Err(e) => return crate::ui::error(&dlg, &e),
            };
            if let Some(v) = pick_columns(&dlg, "Referenced Columns", &cols, &gf.get_cell_value(r, F_REFCOLS), false) {
                gf.set_cell_value(r, F_REFCOLS, &v);
            }
        });
    });
    // Referenced Table: the schema's tables as a dropdown.
    if let Some(sql) = dialect::list_sql(db, dialect::TABLE, &owner) {
        Conn::meta_query(&conn, &sql, move |r| {
            if let (Ok(r), true) = (r, gf.is_valid()) {
                let names: Vec<String> = r.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).map(|n| ddl::shown(db, &n).replace(',', "")).collect();
                if !names.is_empty() {
                    gf.set_col_format_custom(F_REF, &format!("choice:{}", names.join(",")));
                }
            }
        });
    }

    // Index buttons.
    ib[0].on_click(move |_| new_index());
    let rs = rows.clone();
    ib[1].on_click(move |_| {
        let r = gi.get_grid_cursor_row();
        if r >= 0 && r < gi.get_number_rows() {
            gi.delete_rows(r, 1, false);
            rs.borrow_mut().idx.remove(r as usize);
        }
    });

    // The edited definition and its DDL.
    let (rs, old3, sp3, sel3) = (rows.clone(), old.clone(), save_props.clone(), sel.clone());
    let build = Rc::new(move || -> Result<Vec<String>, String> {
        for g in [gc, gu, gf, gk, gi] {
            g.save_edit_control_value();
        }
        sp3(sel3.get());
        let rows = rs.borrow();
        let cell = |g: &Grid, r: usize, c: i32| g.get_cell_value(r as i32, c).trim().to_string();
        let o = old3.as_ref().as_ref();
        let mut t = TableDef {
            owner: o.map_or_else(|| schema.get_value().trim().to_string(), |t| t.owner.clone()),
            name: tname(),
            pk_name: pk_name.get_value().trim().to_string(),
            pk_order: rows.pk_order.clone(),
            comment: comment.get_value().trim().to_string(),
            temporary: (temp_ok && temp.get_value()).then(|| temp_kind.get_selection() != Some(1)),
            storage: Storage {
                tablespace: ts.get_string_selection().unwrap_or_default(),
                pct_free: pct_free.get_value().trim().into(),
                ini_trans: ini_trans.get_value().trim().into(),
                logging: logging.get_string_selection().unwrap_or_default(),
                compress: compress.get_string_selection().unwrap_or_default(),
            },
            ..Default::default()
        };
        // Edit Table: an unchanged loaded PK name stays the loaded one.
        if let Some(o) = o {
            if t.pk_name.is_empty() || t.pk_name == ddl::shown(db, &o.pk_name) {
                t.pk_name = o.pk_name.clone();
            }
        }
        for (r, x) in rows.cols.iter().enumerate() {
            let c = ColumnDef {
                orig: x.orig.clone(),
                name: cell(&gc, r, C_NAME),
                ty: cell(&gc, r, C_TYPE),
                size: cell(&gc, r, C_SIZE),
                scale: cell(&gc, r, C_SCALE),
                not_null: cell(&gc, r, C_NOTNULL) == "1",
                default: cell(&gc, r, C_DEFAULT),
                pk: cell(&gc, r, C_PK) == "1",
                comment: cell(&gc, r, C_COMMENT),
                default_on_null: x.default_on_null,
                virtual_expr: x.virtual_expr.clone(),
                identity: x.identity.clone(),
            };
            // A blank new row is ignored, not an error.
            if c.orig.is_none() && c.name.is_empty() {
                continue;
            }
            t.columns.push(c);
        }
        let mut cons = |g: &Grid, list: &[Option<ConstraintDef>], kind: ConKind| {
            for (r, ex) in list.iter().enumerate() {
                let en = if kind == ConKind::Foreign { F_ENABLED } else { K_ENABLED };
                let enabled = cell(g, r, en) == "1";
                t.constraints.push(match ex {
                    Some(c) => ConstraintDef { name: cell(g, r, 0), enabled, ..c.clone() },
                    None if kind == ConKind::Foreign => ConstraintDef {
                        name: cell(g, r, F_NAME),
                        kind,
                        enabled,
                        columns: cell(g, r, F_COLS),
                        ref_table: cell(g, r, F_REF),
                        ref_columns: cell(g, r, F_REFCOLS),
                        on_delete: cell(g, r, F_DELETE).replace("NO ACTION", ""),
                        ..Default::default()
                    },
                    None if kind == ConKind::Check => ConstraintDef { name: cell(g, r, K_NAME), kind, enabled, check: cell(g, r, K_BODY), ..Default::default() },
                    None => ConstraintDef { name: cell(g, r, K_NAME), kind, enabled, columns: cell(g, r, K_BODY), ..Default::default() },
                });
            }
        };
        cons(&gu, &rows.uk, ConKind::Unique);
        cons(&gf, &rows.fk, ConKind::Foreign);
        cons(&gk, &rows.ck, ConKind::Check);
        for (r, ex) in rows.idx.iter().enumerate() {
            match ex {
                Some(i) => t.indexes.push(i.clone()),
                None if cell(&gi, r, I_COLS).is_empty() => {}
                None => t.indexes.push(IndexDef { existing: false, name: cell(&gi, r, I_NAME), columns: cell(&gi, r, I_COLS), unique: cell(&gi, r, I_UNIQUE) == "1", info: String::new() }),
            }
        }
        match o {
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
    nb.on_page_changed(move |e| match e.get_selection() {
        Some(p) if p == ddl_page => show_ddl(),
        Some(p) if p == pk_page => refresh_pk(),
        _ => {}
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
    s.add_sizer(&top, 0, SizerFlag::Expand | SizerFlag::All, 8);
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
pub fn run(conn: ConnRef, stmts: Vec<String>, i: usize, done: Rc<dyn Fn()>) {
    if i >= stmts.len() {
        crate::app().status(0, &format!("{} statement(s) run", stmts.len()));
        return done();
    }
    let c = conn.clone();
    let (sql, q) = (stmts[i].clone(), stmts[i].clone());
    Conn::work_query(&conn, &q, move |r| match r {
        Ok(_) => run(c, stmts, i + 1, done),
        Err(e) => {
            let left = stmts.len() - i - 1;
            crate::ui::error(&crate::app().frame, &format!("{e}\n\nStatement:\n{sql}{}", if left > 0 { format!("\n\n{left} later statement(s) were not run.") } else { String::new() }));
            done();
        }
    });
}
