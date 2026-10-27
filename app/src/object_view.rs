//! Object viewer: one native notebook tab per dialect tab (Columns, Data, Constraints, ...),
//! loaded lazily on first view. The Data tab is an editable grid that posts generated DML.

use crate::conn::{Conn, ConnRef};
use crate::grid::{ResultGrid, transpose};
use rsqldev_core::dialect::{self, Tab, View};
use rsqldev_core::registry::Kind;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use wxdragon::prelude::*;

pub struct ObjectView {
    pub panel: Panel,
    pub key: String,
    conn: ConnRef,
    db: Kind,
    owner: String,
    kind: &'static str,
    name: String,
    nb: AuiNotebook,
    last_sel: Cell<i32>,
    tabs: Vec<Tab>,
    loaded: RefCell<Vec<bool>>,
    pages: Vec<Page>,
    data: Option<DataTab>,
    me: Weak<ObjectView>,
}

enum Page {
    Grid(ResultGrid),
    Text(StyledTextCtrl),
    Data,
}

struct DataTab {
    grid: ResultGrid,
    filter: TextCtrl,
    status: StaticText,
    /// Messages - Log: executed DML, "Row n: error", commit / rollback.
    log: TextCtrl,
    /// Sort… keys, appended as ORDER BY.
    order: RefCell<Vec<dialect::SortKey>>,
    /// Posts the grid edits (editable tables).
    ed: std::cell::OnceCell<Rc<crate::data_editor::Editor>>,
    hidden: Option<&'static str>,
    editable: bool,
    buttons: [BitmapButton; 6],
    import: BitmapButton,
}

impl ObjectView {
    pub fn new(parent: &dyn WxWidget, conn: ConnRef, owner: &str, kind: &'static str, name: &str) -> Rc<ObjectView> {
        let db = conn.borrow().kind;
        let key = format!("{}|{owner}|{kind}|{name}", conn.borrow().cfg.name);
        let panel = Panel::builder(parent).build();
        let head = BoxSizer::builder(Orientation::Horizontal).build();
        let refresh = crate::icons::tool(&panel, "refresh", "Refresh");
        let title = StaticText::builder(&panel).with_label(&format!("{owner}.{name}  ({})", kind.to_lowercase())).build();
        title.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        head.add(&refresh, 0, SizerFlag::All, 3);
        head.add(&title, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
        let nb = AuiNotebook::builder(&panel).with_style(AuiNotebookStyle::from_bits_retain(0x1 | 0x20 | 0x100)).build();
        let tabs = dialect::tabs(db, kind);
        let prefs = crate::app().prefs();
        let mut pages = vec![];
        let mut data = None;
        for t in &tabs {
            match &t.view {
                View::Grid(_) | View::Details(_) => {
                    let g = ResultGrid::new(&nb);
                    g.st.borrow_mut().db = Some(db);
                    nb.add_page(&g.grid, t.label, pages.is_empty());
                    pages.push(Page::Grid(g));
                }
                View::Code(_) | View::Ddl(..) => {
                    let e = crate::editor::new_editor(&nb, &prefs.editor_font, prefs.editor_font_size);
                    e.set_read_only(true);
                    nb.add_page(&e, t.label, pages.is_empty());
                    pages.push(Page::Text(e));
                }
                View::Data => {
                    let (p, d) = DataTab::build(&nb, db, kind, owner, name);
                    nb.add_page(&p, t.label, pages.is_empty());
                    data = Some(d);
                    pages.push(Page::Data);
                }
            }
        }
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&head, 0, SizerFlag::Expand, 0);
        s.add(&nb, 1, SizerFlag::Expand, 0);
        panel.set_sizer(s, true);
        let n = tabs.len();
        let v = Rc::new_cyclic(|me| ObjectView {
            panel,
            key,
            conn,
            db,
            owner: owner.into(),
            kind,
            name: name.into(),
            nb,
            last_sel: Cell::new(0),
            tabs,
            loaded: RefCell::new(vec![false; n]),
            pages,
            data,
            me: me.clone(),
        });
        // wxDragon's AuiNotebook has no page-changed event: poll the selection.
        let w = Rc::downgrade(&v);
        crate::hub::every_tick(move || {
            let Some(v) = w.upgrade() else { return false };
            if !v.nb.is_valid() {
                return false;
            }
            let sel = v.nb.get_selection();
            if sel >= 0 && sel != v.last_sel.get() {
                v.last_sel.set(sel);
                v.load(sel as usize, false);
            }
            true
        });
        let w = Rc::downgrade(&v);
        refresh.on_click(move |_| {
            if let Some(v) = w.upgrade() {
                v.load(v.nb.get_selection().max(0) as usize, true);
            }
        });
        v.bind_data();
        v.load(0, false);
        v
    }

    pub fn select_tab(&self, label: &str) {
        if let Some(i) = self.tabs.iter().position(|t| t.label == label) {
            self.nb.set_selection(i);
        }
    }

    fn load(&self, i: usize, force: bool) {
        if i >= self.tabs.len() || (self.loaded.borrow()[i] && !force) {
            return;
        }
        self.loaded.borrow_mut()[i] = true;
        let fill = |s: &str| dialect::fill(self.db, s, &self.owner, &self.name);
        match (&self.tabs[i].view, &self.pages[i]) {
            (View::Grid(s), Page::Grid(g)) => {
                let g = g.clone();
                Conn::meta_query(&self.conn, &fill(s), move |r| match r {
                    Ok(r) => g.set(r.cols, r.rows),
                    Err(e) => g.set(vec![rsqldev_core::Column { name: "Error".into(), type_name: String::new(), numeric: false }], vec![vec![Some(e)]]),
                });
            }
            (View::Details(s), Page::Grid(g)) => {
                let g = g.clone();
                Conn::meta_query(&self.conn, &fill(s), move |r| match r {
                    Ok(r) => {
                        let (c, rows) = transpose(&r.cols, &r.rows);
                        g.set(c, rows)
                    }
                    Err(e) => g.set(vec![rsqldev_core::Column { name: "Error".into(), type_name: String::new(), numeric: false }], vec![vec![Some(e)]]),
                });
            }
            (View::Code(s), Page::Text(e)) => {
                let e = *e;
                Conn::meta_query(&self.conn, &fill(s), move |r| {
                    let text = match r {
                        Ok(r) => r.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect::<String>(),
                        Err(err) => format!("-- {err}"),
                    };
                    set_text(e, &text);
                });
            }
            (View::Ddl(s, col), Page::Text(e)) => {
                let (e, col) = (*e, *col);
                Conn::meta_query(&self.conn, &fill(s), move |r| {
                    let text = match r {
                        Ok(r) => r.rows.into_iter().next().and_then(|row| row.into_iter().nth(col).flatten()).unwrap_or_default(),
                        Err(err) => format!("-- {err}"),
                    };
                    set_text(e, text.trim_start());
                });
            }
            (View::Data, Page::Data) => self.refresh_data(),
            _ => {}
        }
    }

    fn bind_data(&self) {
        let Some(d) = &self.data else { return };
        if d.editable {
            let (me, log) = (self.me.clone(), d.log);
            let ed = crate::data_editor::Editor::new(
                self.conn.clone(),
                &self.owner,
                &self.name,
                d.grid.clone(),
                d.hidden,
                move |line| log.append_text(&format!("{line}\n")),
                move || {
                    if let Some(me) = me.upgrade() {
                        me.load_data()
                    }
                },
            );
            let _ = d.ed.set(ed);
        }
        let st = d.status;
        d.grid.st.borrow_mut().on_change = Some(Rc::new(move |s| {
            let pending = s.rows.iter().filter(|r| r.dirty()).count();
            let mut msg = format!("{} rows{}", s.rows.len(), if s.done { "" } else { "+" });
            if pending > 0 {
                msg += &format!(" — {pending} pending change(s), Commit to post");
            }
            st.set_label(&msg);
        }));
        let me = self.me.clone();
        d.grid.st.borrow_mut().sort_dialog = Some(Rc::new(move || {
            if let Some(me) = me.upgrade() {
                me.sort_dialog()
            }
        }));
        let acts: [fn(&ObjectView); 6] = [ObjectView::insert_row, ObjectView::delete_rows, ObjectView::edit_value, ObjectView::commit, ObjectView::rollback, ObjectView::refresh_data];
        for (b, f) in d.buttons.iter().zip(acts) {
            let me = self.me.clone();
            b.on_click(move |_| {
                if let Some(me) = me.upgrade() {
                    f(&me)
                }
            });
        }
        let me = self.me.clone();
        d.import.on_click(move |_| {
            if let Some(me) = me.upgrade() {
                me.import()
            }
        });
        let me = self.me.clone();
        d.filter.on_text_enter(move |_| {
            if let Some(me) = me.upgrade() {
                me.refresh_data()
            }
        });
    }

    fn data_sql(&self) -> (String, &DataTab) {
        let d = self.data.as_ref().unwrap();
        let dq = dialect::data_query(self.db, self.kind, &self.owner, &self.name);
        let f = d.filter.get_value();
        let sql = if f.trim().is_empty() { dq.sql } else { format!("{} WHERE {}", dq.sql, f.trim()) };
        (sql + &dialect::order_by(self.db, &d.order.borrow()), d)
    }

    /// Refresh / filter / sort: uncommitted edits ask Commit, Discard or Cancel first.
    pub fn refresh_data(&self) {
        let Some(ed) = self.data.as_ref().and_then(|d| d.ed.get()).filter(|e| e.pending()) else { return self.load_data() };
        match crate::ui::commit_discard(&self.panel, &format!("Table {} has changes that are not committed.", self.name)) {
            // The commit reloads the data.
            Some(true) => {
                ed.commit();
            }
            Some(false) => {
                ed.discard();
                self.load_data()
            }
            None => {}
        }
    }

    pub fn load_data(&self) {
        let (sql, d) = self.data_sql();
        if let Some(ed) = d.ed.get() {
            ed.loaded();
        }
        let st = d.status;
        let rows = Conn::exec(&self.conn, &sql, move |done| {
            if let Err(e) = done.result {
                st.set_label(&e);
            }
        });
        d.grid.load(rows, d.hidden.map_or(0, |_| 1), d.editable);
        d.grid.st.borrow_mut().count = Conn::counter(&self.conn, &sql, d.status);
        d.grid.st.borrow_mut().resume = Conn::resume(&self.conn, &sql);
    }

    /// Commit Changes (F11): post every pending row, then COMMIT.
    pub fn commit(&self) {
        if let Some(ed) = self.data.as_ref().and_then(|d| d.ed.get()) {
            ed.commit();
        }
    }

    /// Rollback Changes (F12).
    pub fn rollback(&self) {
        if let Some(ed) = self.data.as_ref().and_then(|d| d.ed.get()) {
            ed.rollback();
        }
    }

    /// Sort… : several columns, ASC / DESC, NULLS FIRST / LAST, re-queried with ORDER BY.
    fn sort_dialog(&self) {
        let Some(d) = &self.data else { return };
        let names: Vec<String> = {
            let st = d.grid.st.borrow();
            st.cols.iter().skip(st.hidden).map(|c| c.name.clone()).collect()
        };
        if names.is_empty() {
            return;
        }
        let nulls_ok = !matches!(self.db, Kind::Mysql | Kind::Mssql);
        let dlg = Dialog::builder(&self.panel, "Sort").with_style(DialogStyle::DefaultDialogStyle).build();
        let head = StaticText::builder(&dlg).with_label("Select columns to sort by").build();
        let g = FlexGridSizer::builder(0, 3).with_vgap(4).with_hgap(6).build();
        let cols: Vec<String> = std::iter::once("(none)".to_string()).chain(names.iter().cloned()).collect();
        let cur = d.order.borrow().clone();
        let mut rows = vec![];
        for i in 0..4 {
            let c = Choice::builder(&dlg).with_choices(cols.clone()).build();
            let dir = Choice::builder(&dlg).with_choices(vec!["ASC".into(), "DESC".into()]).build();
            let nl = Choice::builder(&dlg).with_choices(vec!["Nulls default".into(), "Nulls First".into(), "Nulls Last".into()]).build();
            let k = cur.get(i);
            c.set_selection(k.and_then(|k| names.iter().position(|n| *n == k.0)).map_or(0, |p| p + 1) as u32);
            dir.set_selection(u32::from(k.is_some_and(|k| k.1)));
            nl.set_selection(match k.and_then(|k| k.2) {
                Some(true) => 1,
                Some(false) => 2,
                None => 0,
            });
            nl.show(nulls_ok);
            g.add(&c, 0, SizerFlag::Expand, 0);
            g.add(&dir, 0, SizerFlag::empty(), 0);
            g.add(&nl, 0, SizerFlag::empty(), 0);
            rows.push((c, dir, nl));
        }
        let bs = BoxSizer::builder(Orientation::Horizontal).build();
        let clear = Button::builder(&dlg).with_label("Clear").with_id(ID_NO).build();
        clear.on_click(move |_| dlg.end_modal(ID_NO));
        bs.add(&clear, 0, SizerFlag::All, 4);
        bs.add_stretch_spacer(1);
        bs.add(&Button::builder(&dlg).with_label("Cancel").with_id(ID_CANCEL).build(), 0, SizerFlag::All, 4);
        bs.add(&Button::builder(&dlg).with_label("OK").with_id(ID_OK).build(), 0, SizerFlag::All, 4);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add(&head, 0, SizerFlag::All, 8);
        s.add_sizer(&g, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
        s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
        dlg.set_sizer_and_fit(s, true);
        let keys = match dlg.show_modal() {
            ID_OK => rows
                .iter()
                .filter_map(|(c, dir, nl)| {
                    let i = c.get_selection().unwrap_or(0) as usize;
                    let nulls = match nl.get_selection().unwrap_or(0) {
                        1 => Some(true),
                        2 => Some(false),
                        _ => None,
                    };
                    (i > 0).then(|| (names[i - 1].clone(), dir.get_selection() == Some(1), nulls))
                })
                .collect(),
            ID_NO => vec![],
            _ => return dlg.destroy(),
        };
        dlg.destroy();
        *d.order.borrow_mut() = keys;
        self.refresh_data();
    }

    /// Import CSV / Excel rows as pending inserts: headers matched to columns by name, review
    /// in the grid, Commit inserts them all or rolls back on the first error.
    pub fn import(&self) {
        let Some(d) = &self.data else { return };
        let cols = d.grid.st.borrow().cols.clone();
        if cols.is_empty() {
            return crate::ui::error(&self.panel, "Load the table data first (Refresh).");
        }
        let f = FileDialog::builder(&self.panel)
            .with_message("Import Data")
            .with_style(FileDialogStyle::Open)
            .with_wildcard("Data files (*.csv;*.xlsx;*.xls;*.ods)|*.csv;*.xlsx;*.xls;*.ods|All files (*)|*")
            .build();
        if f.show_modal() != ID_OK {
            return;
        }
        let Some(path) = f.get_path() else { return };
        let (headers, rows) = match rsqldev_core::import::read(std::path::Path::new(&path)) {
            Ok(x) => x,
            Err(e) => return crate::ui::error(&self.panel, &format!("Import failed: {e:#}")),
        };
        let hidden = d.hidden.map(|_| 0);
        let map: Vec<Option<usize>> = rsqldev_core::import::map_columns(&headers, &cols).into_iter().map(|m| m.filter(|&i| Some(i) != hidden)).collect();
        let used: Vec<&str> = headers.iter().zip(&map).filter(|(_, m)| m.is_some()).map(|(h, _)| h.as_str()).collect();
        let ignored: Vec<&str> = headers.iter().zip(&map).filter(|(_, m)| m.is_none()).map(|(h, _)| h.as_str()).collect();
        if used.is_empty() {
            return crate::ui::error(&self.panel, &format!("No file column matches a column of {}.
File columns: {}", self.name, headers.join(", ")));
        }
        let msg = format!(
            "Add {} row(s) as new rows?

Columns: {}
Ignored: {}

Review them in the grid, then Commit (F11) to insert.",
            rows.len(),
            used.join(", "),
            if ignored.is_empty() { "none".into() } else { ignored.join(", ") }
        );
        if !crate::ui::confirm(&self.panel, &msg) {
            return;
        }
        let out: Vec<Vec<rsqldev_core::Cell>> = rows
            .into_iter()
            .map(|r| {
                let mut row = vec![None; cols.len()];
                for (fi, ti) in map.iter().enumerate() {
                    if let Some(ti) = ti {
                        row[*ti] = r.get(fi).cloned().flatten();
                    }
                }
                row
            })
            .collect();
        // Fetch the rest first so imported rows stay after every fetched row.
        let g = d.grid.clone();
        d.grid.fetch_all(move || g.append_new_rows(out));
    }

    pub fn insert_row(&self) {
        if let Some(d) = &self.data {
            d.grid.insert_row();
        }
    }

    pub fn delete_rows(&self) {
        if let Some(d) = &self.data {
            d.grid.delete_selected_rows();
        }
    }

    /// Edit Value (pencil, Alt+Enter) on the cursor cell.
    pub fn edit_value(&self) {
        if let Some(d) = &self.data {
            d.grid.edit_value();
        }
    }

    /// The Data tab's grid (self-test).
    pub fn data_grid(&self) -> Option<ResultGrid> {
        self.data.as_ref().map(|d| d.grid.clone())
    }

    /// The Data tab is the selected tab (keys F11, F12, Ctrl+I, Ctrl+D, Ctrl+R, paste go to it).
    pub fn data_active(&self) -> bool {
        let sel = self.nb.get_selection();
        sel >= 0 && matches!(self.pages.get(sel as usize), Some(Page::Data))
    }

    /// Paste into the data grid, unless the filter box has the keyboard.
    pub fn paste(&self) -> bool {
        match &self.data {
            Some(d) if !d.filter.has_focus() => d.grid.paste(),
            _ => false,
        }
    }
}

fn set_text(e: StyledTextCtrl, text: &str) {
    e.set_read_only(false);
    e.set_text(text);
    e.set_read_only(true);
}

impl DataTab {
    fn build(nb: &AuiNotebook, db: Kind, kind: &str, owner: &str, name: &str) -> (Panel, DataTab) {
        let p = Panel::builder(nb).build();
        let dq = dialect::data_query(db, kind, owner, name);
        let bar = BoxSizer::builder(Orientation::Horizontal).build();
        let mk = |icon: &str, tip: &str| {
            let b = crate::icons::tool(&p, icon, tip);
            b.enable(dq.editable);
            bar.add(&b, 0, SizerFlag::All, 3);
            b
        };
        let refresh = crate::icons::tool(&p, "refresh", "Refresh");
        bar.add(&refresh, 0, SizerFlag::All, 3);
        let buttons = [
            mk("insertrow", "Insert Row (Ctrl+I)"),
            mk("deleterow", "Delete Selected Row(s) (Ctrl+D)"),
            mk("edit", "Edit Value… (Alt+Enter): long text, dates, LOBs"),
            mk("commit", "Commit Changes (F11)"),
            mk("rollback", "Rollback Changes (F12)"),
            refresh,
        ];
        let import = mk("open", "Import Data from CSV / Excel…");
        let fl = StaticText::builder(&p).with_label("Filter:").build();
        let filter = TextCtrl::builder(&p).with_style(TextCtrlStyle::ProcessEnter).build();
        crate::copy::text(filter);
        filter.set_tooltip("WHERE clause, press Enter");
        bar.add(&fl, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 10);
        bar.add(&filter, 1, SizerFlag::All, 2);
        let grid = ResultGrid::new(&p);
        grid.st.borrow_mut().db = Some(db);
        let status = StaticText::builder(&p).with_label(" ").build();
        let log = TextCtrl::builder(&p).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap).build();
        log.set_min_size(Size::new(-1, 90));
        log.set_tooltip("Messages - Log: statements posted, errors per row, commit / rollback");
        crate::copy::text(log);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
        s.add(&grid.grid, 1, SizerFlag::Expand, 0);
        s.add(&status, 0, SizerFlag::Expand | SizerFlag::All, 3);
        s.add(&log, 0, SizerFlag::Expand, 0);
        log.show(dq.editable);
        p.set_sizer(s, true);
        let d = DataTab { grid, filter, status, log, order: RefCell::default(), ed: Default::default(), hidden: dq.hidden, editable: dq.editable, buttons, import };
        (p, d)
    }
}
