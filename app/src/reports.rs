//! Tools ▸ Reports: shipped read-only reports per engine (version, storage, invalid objects,
//! tables without a primary key, users, locks, waits, ...). Runs on the metadata session;
//! the next candidate query is tried when one fails (fewer privileges needed).

use crate::conn::{Conn, ConnRef};
use crate::grid::ResultGrid;
use rsqldev_core::dialect;
use rsqldev_core::registry::Kind;
use std::rc::{Rc, Weak};
use wxdragon::prelude::*;

pub struct Reports {
    pub panel: Panel,
    pub conn_name: String,
    conn: ConnRef,
    db: Kind,
    list: Choice,
    grid: ResultGrid,
    status: StaticText,
    /// SQL of the shown result (Open SQL).
    shown: std::cell::RefCell<String>,
    me: Weak<Reports>,
}

impl Reports {
    pub fn new(parent: &dyn WxWidget, conn: ConnRef) -> Rc<Reports> {
        let db = conn.borrow().kind;
        let panel = Panel::builder(parent).build();
        let bar = BoxSizer::builder(Orientation::Horizontal).build();
        let refresh = crate::icons::tool(&panel, "refresh", "Run again");
        let list = Choice::builder(&panel).with_choices(dialect::reports(db).iter().map(|r| r.0.to_string()).collect()).build();
        list.set_selection(0);
        let open = Button::builder(&panel).with_label("Open SQL").build();
        let status = StaticText::builder(&panel).with_label(" ").build();
        status.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        bar.add(&refresh, 0, SizerFlag::All, 3);
        bar.add(&list, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        bar.add(&open, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        bar.add(&status, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 12);
        let grid = ResultGrid::new(&panel);
        grid.st.borrow_mut().db = Some(db);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
        s.add(&grid.grid, 1, SizerFlag::Expand, 0);
        panel.set_sizer(s, true);
        let conn_name = conn.borrow().cfg.name.clone();
        let v = Rc::new_cyclic(|me| Reports { panel, conn_name, conn, db, list, grid, status, shown: Default::default(), me: me.clone() });
        let w = v.me.clone();
        refresh.on_click(move |_| {
            if let Some(v) = w.upgrade() {
                v.run_selected()
            }
        });
        let w = v.me.clone();
        v.list.on_selection_changed(move |_| {
            if let Some(v) = w.upgrade() {
                v.run_selected()
            }
        });
        let w = v.me.clone();
        open.on_click(move |_| {
            let Some(v) = w.upgrade() else { return };
            let sql = v.shown.borrow().clone();
            if sql.is_empty() {
                return;
            }
            let app = crate::app();
            app.open_worksheet(v.conn.clone());
            if let Some(ws) = app.current_sheet() {
                ws.editor.set_text(&format!("{sql};\n"));
            }
        });
        v.run_selected();
        v
    }

    fn run_selected(&self) {
        let reports = dialect::reports(self.db);
        let Some((label, qs)) = reports.get(self.list.get_selection().unwrap_or(0) as usize) else { return };
        let schema = self.conn.borrow().schema.clone();
        let qs: Vec<String> = qs.iter().map(|q| dialect::fill(self.db, q, &schema, "")).collect();
        self.status.set_label(&format!("Running {label}…"));
        self.run(qs, 0, String::new());
    }

    /// Candidates in order; the first error is kept to show if all fail.
    fn run(&self, qs: Vec<String>, i: usize, first_err: String) {
        let w = self.me.clone();
        let sql = qs[i].clone();
        Conn::meta_query(&self.conn, &qs[i].clone(), move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    let n = r.rows.len();
                    v.grid.set(r.cols, r.rows);
                    *v.shown.borrow_mut() = sql;
                    let note = if i > 0 { " (reduced: fewer privileges)" } else { "" };
                    v.status.set_label(&format!("{n} row(s){note} · {}", chrono::Local::now().format("%H:%M:%S")));
                }
                Err(e) if i + 1 < qs.len() => v.run(qs, i + 1, if first_err.is_empty() { e } else { first_err }),
                Err(e) => {
                    v.grid.set(vec![], vec![]);
                    v.status.set_label(&format!("Not available: {}", if first_err.is_empty() { e } else { first_err }).replace('\n', " "));
                }
            }
        });
    }
}
