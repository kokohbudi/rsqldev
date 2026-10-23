//! Tools ▸ Top SQL: the heaviest statements in the server's statement cache (Oracle V$SQL,
//! pg_stat_statements, performance_schema digests, dm_exec_query_stats, ClickHouse query_log),
//! sortable by elapsed / CPU / reads / executions. Runs on the metadata session.

use crate::conn::{Conn, ConnRef};
use crate::grid::ResultGrid;
use rsqldev_core::dialect;
use rsqldev_core::registry::Kind;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};
use wxdragon::prelude::*;

pub struct TopSql {
    pub panel: Panel,
    pub conn_name: String,
    conn: ConnRef,
    db: Kind,
    grid: ResultGrid,
    order: Choice,
    filter: TextCtrl,
    status: StaticText,
    auto: CheckBox,
    me: Weak<TopSql>,
}

impl TopSql {
    pub fn new(parent: &dyn WxWidget, conn: ConnRef) -> Rc<TopSql> {
        let db = conn.borrow().kind;
        let panel = Panel::builder(parent).build();
        let bar = BoxSizer::builder(Orientation::Horizontal).build();
        let refresh = crate::icons::tool(&panel, "refresh", "Refresh");
        let order = Choice::builder(&panel).with_choices(dialect::TOP_ORDERS.iter().map(|o| format!("Order by {}", o.1)).collect()).build();
        order.set_selection(0);
        let filter_label = StaticText::builder(&panel).with_label("SQL text contains:").build();
        let filter = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ProcessEnter).build();
        crate::copy::text(filter);
        filter.set_min_size(Size::new(200, -1));
        let open = Button::builder(&panel).with_label("Open SQL").build();
        let plan = Button::builder(&panel).with_label("Show Plan").build();
        plan.enable(db == Kind::Oracle);
        let auto = CheckBox::builder(&panel).with_label("Auto-refresh (10 s)").build();
        let status = StaticText::builder(&panel).with_label(" ").build();
        status.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        bar.add(&refresh, 0, SizerFlag::All, 3);
        bar.add(&order, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        bar.add(&filter_label, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
        bar.add(&filter, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        bar.add(&open, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        bar.add(&plan, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        bar.add(&auto, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 10);
        bar.add(&status, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 12);
        let grid = ResultGrid::new(&panel);
        grid.st.borrow_mut().db = Some(db);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
        s.add(&grid.grid, 1, SizerFlag::Expand, 0);
        panel.set_sizer(s, true);
        let conn_name = conn.borrow().cfg.name.clone();
        let v = Rc::new_cyclic(|me| TopSql { panel, conn_name, conn, db, grid, order, filter, status, auto, me: me.clone() });
        let acts: [(Button, fn(&TopSql)); 2] = [(open, TopSql::open_sql), (plan, TopSql::show_plan)];
        for (b, f) in acts {
            let w = v.me.clone();
            b.on_click(move |_| {
                if let Some(v) = w.upgrade() {
                    f(&v)
                }
            });
        }
        let w = v.me.clone();
        refresh.on_click(move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh()
            }
        });
        let w = v.me.clone();
        v.order.on_selection_changed(move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh()
            }
        });
        let w = v.me.clone();
        v.filter.on_text_enter(move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh()
            }
        });
        let w = v.me.clone();
        let mut last = Instant::now();
        crate::hub::every_tick(move || {
            let Some(v) = w.upgrade() else { return false };
            if v.auto.get_value() && last.elapsed() >= Duration::from_secs(10) {
                last = Instant::now();
                v.refresh();
            }
            true
        });
        v.refresh();
        v
    }

    pub fn refresh(&self) {
        let order = dialect::TOP_ORDERS.get(self.order.get_selection().unwrap_or(0) as usize).map_or("elapsed", |o| o.0);
        let Some(qs) = dialect::top_sql(self.db, order, &self.filter.get_value()) else {
            return self.status.set_label("Statement statistics are not available for file databases.");
        };
        self.status.set_label("Refreshing…");
        self.run(qs, 0);
    }

    /// Try the candidate queries in order (older server versions) until one works.
    fn run(&self, qs: Vec<String>, i: usize) {
        let w = self.me.clone();
        let sql = qs[i].clone();
        Conn::meta_query(&self.conn, &sql, move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    let n = r.rows.len();
                    v.grid.set(r.cols, r.rows);
                    v.status.set_label(&format!("Top {n} statement(s) · refreshed {}", chrono::Local::now().format("%H:%M:%S")));
                }
                Err(_) if i + 1 < qs.len() => v.run(qs, i + 1),
                Err(e) => v.failed(&e),
            }
        });
    }

    /// Why the list is unavailable, with the grant or setting to ask for; auto-refresh stops.
    fn failed(&self, e: &str) {
        self.auto.set_value(false);
        let user = self.conn.borrow().cfg.user.to_uppercase();
        let low = e.to_lowercase();
        let hint: Vec<String> = match self.db {
            Kind::Oracle if e.contains("ORA-00942") || e.contains("ORA-01031") => {
                vec![format!("{user} may not read V$SQL. Ask a DBA for:"), format!("  GRANT SELECT ON SYS.V_$SQL TO {user};   (or the SELECT_CATALOG_ROLE)")]
            }
            Kind::Postgres if low.contains("pg_stat_statements") => vec![
                "The pg_stat_statements extension is needed. A superuser can enable it:".into(),
                "  shared_preload_libraries = 'pg_stat_statements'   (postgresql.conf, then restart)".into(),
                "  CREATE EXTENSION pg_stat_statements;".into(),
            ],
            Kind::Mssql if e.contains("VIEW SERVER STATE") || low.contains("permission") => vec![format!("Ask a DBA for:  GRANT VIEW SERVER STATE TO [{user}];")],
            Kind::Mysql if low.contains("denied") => vec![format!("Ask a DBA for:  GRANT SELECT ON performance_schema.* TO '{user}';")],
            _ => vec![],
        };
        let rows: Vec<Vec<rsqldev_core::Cell>> = e.lines().map(str::to_string).chain(hint).filter(|l| !l.trim().is_empty()).map(|l| vec![Some(l)]).collect();
        self.grid.set(vec![rsqldev_core::Column { name: "Top SQL is not available".into(), type_name: String::new(), numeric: false }], rows);
        self.status.set_label("Could not read statement statistics (see below)");
    }

    /// Cell `col` (by name) of the grid cursor row.
    fn cell(&self, col: &str) -> Option<String> {
        let st = self.grid.st.borrow();
        let r = self.grid.grid.get_grid_cursor_row();
        let i = st.cols.iter().position(|c| c.name.eq_ignore_ascii_case(col))?;
        st.rows.get(usize::try_from(r).ok()?)?.cur.get(i).cloned().flatten()
    }

    /// The selected statement in a worksheet on this connection (Oracle: the full text).
    fn open_sql(&self) {
        let Some(text) = self.cell("sql_text") else { return crate::ui::error(&self.panel, "Select a statement row first.") };
        let c = self.conn.clone();
        let show = move |t: String| {
            let app = crate::app();
            app.open_worksheet(c);
            if let Some(ws) = app.current_sheet() {
                ws.editor.set_text(&t);
            }
        };
        match self.cell("sql_id").and_then(|id| dialect::top_sql_text(&id)) {
            Some(q) => Conn::meta_query(&self.conn, &q, move |r| {
                let full = r.ok().and_then(|r| r.rows.into_iter().next()?.into_iter().next()?);
                show(full.unwrap_or(text));
            }),
            None => show(text),
        }
    }

    /// Oracle: the cursor's execution plan (DBMS_XPLAN.DISPLAY_CURSOR) in a worksheet, run.
    fn show_plan(&self) {
        let (Some(id), Some(child)) = (self.cell("sql_id"), self.cell("child")) else { return crate::ui::error(&self.panel, "Select a statement row first.") };
        let Some(q) = dialect::top_sql_plan(&id, &child) else { return };
        let app = crate::app();
        app.open_worksheet(self.conn.clone());
        if let Some(ws) = app.current_sheet() {
            ws.editor.set_text(&format!("{q};"));
            ws.run_statement();
        }
    }
}
