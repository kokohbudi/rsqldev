//! Object viewer: one native notebook tab per dialect tab (Columns, Data, Constraints, ...),
//! loaded lazily on first view. The Data tab is an editable grid that posts generated DML.

use crate::conn::{Conn, ConnRef};
use crate::grid::{ResultGrid, Row, transpose};
use rsqldev_core::dialect::{self, Tab, View};
use rsqldev_core::{Bind, dml};
use rsqldev_core::registry::Kind;
use rsqldev_core::worker::Cmd;
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
    pk: RefCell<Vec<String>>,
    hidden: Option<&'static str>,
    editable: bool,
    busy: Cell<bool>,
    buttons: [BitmapButton; 5],
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
            (View::Data, Page::Data) => self.load_data(),
            _ => {}
        }
    }

    fn bind_data(&self) {
        let Some(d) = &self.data else { return };
        if let Some(sql) = dialect::pk_sql(self.db, &self.owner, &self.name) {
            let me = self.me.clone();
            Conn::meta_query(&self.conn, &sql, move |r| {
                if let (Some(me), Ok(r)) = (me.upgrade(), r) {
                    if let Some(d) = &me.data {
                        *d.pk.borrow_mut() = r.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect();
                    }
                }
            });
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
        let acts: [fn(&ObjectView); 5] = [ObjectView::insert_row, ObjectView::delete_rows, ObjectView::commit, ObjectView::rollback, ObjectView::load_data];
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
                me.load_data()
            }
        });
    }

    fn data_sql(&self) -> (String, &DataTab) {
        let d = self.data.as_ref().unwrap();
        let dq = dialect::data_query(self.db, self.kind, &self.owner, &self.name);
        let f = d.filter.get_value();
        let sql = if f.trim().is_empty() { dq.sql } else { format!("{} WHERE {}", dq.sql, f.trim()) };
        (sql, d)
    }

    pub fn load_data(&self) {
        let (sql, d) = self.data_sql();
        let st = d.status;
        let rows = Conn::exec(&self.conn, &sql, move |done| {
            if let Err(e) = done.result {
                st.set_label(&e);
            }
        });
        d.grid.load(rows, d.hidden.map_or(0, |_| 1), d.editable);
    }

    /// Generated DML for pending grid edits, with the values each statement binds (staged BLOB
    /// files, and text too long for an Oracle literal).
    fn pending_dml(&self) -> Vec<(String, Vec<Bind>)> {
        let d = self.data.as_ref().unwrap();
        let st = d.grid.st.borrow();
        let table = dialect::qualified(self.db, &self.owner, &self.name);
        let key = dml::Key::new(&st.cols, d.hidden, &d.pk.borrow());
        let skip = d.hidden.map(|_| 0);
        let mut out = vec![];
        for row in &st.rows {
            let Row { cur, deleted, blobs, new, .. } = row;
            let changed = |c: usize| *new || row.original()[c] != cur[c];
            let cols: Vec<usize> = (0..st.cols.len())
                .filter(|&c| Some(c) != skip && (blobs.contains_key(&c) || (changed(c) && dml::text_needs_bind(self.db, &st.cols[c], &cur[c]))))
                .collect();
            let binds = || {
                cols.iter()
                    .map(|c| match blobs.get(c) {
                        Some(b) => Bind::Bytes(b.to_vec()),
                        None => Bind::Text(cur[*c].as_deref().map(|s| rsqldev_core::lob::expand(s).into_owned()).unwrap_or_default()),
                    })
                    .collect::<Vec<_>>()
            };
            match (new, deleted) {
                (true, true) => {}
                (true, false) => out.push((dml::insert(self.db, &table, &st.cols, skip, cur, &cols), binds())),
                (false, true) => out.push((dml::delete(self.db, &table, &st.cols, &key, row.original()), vec![])),
                (false, false) if row.dirty() => out.extend(dml::update(self.db, &table, &st.cols, &key, skip, row.original(), cur, &cols).map(|s| (s, binds()))),
                (false, false) => {}
            }
        }
        out
    }

    pub fn commit(&self) {
        let Some(d) = &self.data else { return };
        if d.busy.get() {
            return;
        }
        let stmts = self.pending_dml();
        if stmts.is_empty() {
            return;
        }
        d.busy.set(true);
        self.post(stmts, 0);
    }

    fn post(&self, stmts: Vec<(String, Vec<Bind>)>, i: usize) {
        let d = self.data.as_ref().unwrap();
        let me = self.me.clone();
        if i == stmts.len() {
            return Conn::tx(&self.conn, Cmd::Commit, move |r| {
                let Some(me) = me.upgrade() else { return };
                let d = me.data.as_ref().unwrap();
                d.busy.set(false);
                match r {
                    Ok(_) => {
                        me.load_data();
                        d.status.set_label(&format!("Commit successful: {} statement(s)", stmts.len()));
                    }
                    Err(e) => crate::ui::error(&me.panel, &e),
                }
            });
        }
        let (sql, binds) = stmts[i].clone();
        let run = sql.clone();
        let done = move |r: Result<Option<u64>, String>| {
            let Some(me) = me.upgrade() else { return };
            match r {
                Ok(Some(0)) => {
                    me.data.as_ref().unwrap().busy.set(false);
                    crate::ui::error(&me.panel, &format!("Row was not found (changed by another session?):\n{sql}"));
                    Conn::tx(&me.conn, Cmd::Rollback, |_| {});
                }
                Ok(_) => me.post(stmts, i + 1),
                Err(e) => {
                    me.data.as_ref().unwrap().busy.set(false);
                    crate::ui::error(&me.panel, &format!("One error saving changes to table {}:\n{sql}\n\n{e}", me.name));
                    Conn::tx(&me.conn, Cmd::Rollback, |_| {});
                }
            }
        };
        if binds.is_empty() {
            Conn::work_query(&self.conn, &run, move |r| done(r.map(|r| r.affected)));
        } else {
            Conn::work_binds(&self.conn, &run, binds, done);
        }
        let _ = d;
    }

    pub fn rollback(&self) {
        let me = self.me.clone();
        Conn::tx(&self.conn, Cmd::Rollback, move |_| {
            if let Some(me) = me.upgrade() {
                me.load_data();
            }
        });
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
            mk("commit", "Commit Changes (F11)"),
            mk("rollback", "Rollback Changes (F12)"),
            refresh,
        ];
        let import = mk("open", "Import Data from CSV / Excel…");
        let fl = StaticText::builder(&p).with_label("Filter:").build();
        let filter = TextCtrl::builder(&p).with_style(TextCtrlStyle::ProcessEnter).build();
        filter.set_tooltip("WHERE clause, press Enter");
        bar.add(&fl, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 10);
        bar.add(&filter, 1, SizerFlag::All, 2);
        let grid = ResultGrid::new(&p);
        grid.st.borrow_mut().db = Some(db);
        let status = StaticText::builder(&p).with_label(" ").build();
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
        s.add(&grid.grid, 1, SizerFlag::Expand, 0);
        s.add(&status, 0, SizerFlag::Expand | SizerFlag::All, 3);
        p.set_sizer(s, true);
        (p, DataTab { grid, filter, status, pk: RefCell::default(), hidden: dq.hidden, editable: dq.editable, busy: Cell::new(false), buttons, import })
    }
}
