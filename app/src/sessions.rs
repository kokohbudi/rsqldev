//! Tools ▸ Sessions: who is connected, what they run, who blocks whom; Kill Session.
//! Queries run on the connection's metadata session, so an open worksheet is not disturbed.

use crate::conn::{Conn, ConnRef};
use crate::grid::ResultGrid;
use rsqldev_core::dialect;
use rsqldev_core::registry::Kind;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};
use wxdragon::prelude::*;

pub struct Sessions {
    pub panel: Panel,
    pub conn_name: String,
    conn: ConnRef,
    db: Kind,
    grid: ResultGrid,
    status: StaticText,
    auto: CheckBox,
    me: Weak<Sessions>,
}

impl Sessions {
    pub fn new(parent: &dyn WxWidget, conn: ConnRef) -> Rc<Sessions> {
        let (db, conn_name) = (conn.borrow().kind, conn.borrow().cfg.name.clone());
        let panel = Panel::builder(parent).build();
        let bar = BoxSizer::builder(Orientation::Horizontal).build();
        let refresh = crate::icons::tool(&panel, "refresh", "Refresh");
        let kill = Button::builder(&panel).with_label("Kill Session…").build();
        let auto = CheckBox::builder(&panel).with_label("Auto-refresh (5 s)").build();
        let status = StaticText::builder(&panel).with_label(" ").build();
        status.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        bar.add(&refresh, 0, SizerFlag::All, 3);
        bar.add(&kill, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        bar.add(&auto, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 10);
        bar.add(&status, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 12);
        let grid = ResultGrid::new(&panel);
        grid.st.borrow_mut().db = Some(db);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
        s.add(&grid.grid, 1, SizerFlag::Expand, 0);
        panel.set_sizer(s, true);
        let v = Rc::new_cyclic(|me| Sessions { panel, conn_name, conn, db, grid, status, auto, me: me.clone() });
        let w = v.me.clone();
        refresh.on_click(move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh()
            }
        });
        let w = v.me.clone();
        kill.on_click(move |_| {
            if let Some(v) = w.upgrade() {
                v.kill()
            }
        });
        // Auto-refresh while the tab exists and the box is ticked.
        let w = v.me.clone();
        let mut last = Instant::now();
        crate::hub::every_tick(move || {
            let Some(v) = w.upgrade() else { return false };
            if v.auto.get_value() && last.elapsed() >= Duration::from_secs(5) {
                last = Instant::now();
                v.refresh();
            }
            true
        });
        v.refresh();
        v
    }

    pub fn refresh(&self) {
        let Some(qs) = dialect::sessions_sql(self.db) else {
            return self.status.set_label("Sessions are not available for file databases.");
        };
        self.status.set_label("Refreshing…");
        self.run(qs, 0);
    }

    /// Try the candidate queries in order until one works.
    fn run(&self, qs: Vec<&'static str>, i: usize) {
        let w = self.me.clone();
        Conn::meta_query(&self.conn, qs[i], move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    let bi = r.cols.iter().position(|c| c.name.eq_ignore_ascii_case("blocked_by"));
                    let blocked = bi.map_or(0, |b| r.rows.iter().filter(|row| row[b].as_deref().is_some_and(|s| !s.is_empty())).count());
                    let n = r.rows.len();
                    v.grid.set(r.cols, r.rows);
                    let when = chrono::Local::now().format("%H:%M:%S");
                    v.status.set_label(&format!("{n} session(s){} · refreshed {when}", if blocked > 0 { format!(", {blocked} blocked (listed first)") } else { String::new() }));
                }
                Err(_) if i + 1 < qs.len() => v.run(qs, i + 1),
                Err(e) => v.failed(&e),
            }
        });
    }

    /// Show why the list is unavailable in the grid (full text, with what to ask a DBA for),
    /// keep the status to one line, and stop auto-refresh from repeating a query that fails.
    fn failed(&self, e: &str) {
        self.auto.set_value(false);
        let user = self.conn.borrow().cfg.user.to_uppercase();
        let denied = e.contains("ORA-00942") || e.contains("ORA-01031") || e.to_lowercase().contains("permission denied") || e.contains("VIEW SERVER STATE");
        let hint: &[String] = &match (self.db, denied) {
            (Kind::Oracle, true) => vec![
                format!("{user} may not read V$SESSION. Ask a DBA for:"),
                format!("  GRANT SELECT ON SYS.V_$SESSION TO {user};   (or the SELECT_CATALOG_ROLE)"),
                format!("Kill Session also needs:  GRANT ALTER SYSTEM TO {user};"),
            ],
            (Kind::Mssql, true) => vec![format!("Ask a DBA for:  GRANT VIEW SERVER STATE TO [{user}];")],
            (Kind::Mysql, true) => vec![format!("Ask a DBA for the PROCESS privilege:  GRANT PROCESS ON *.* TO '{user}';")],
            _ => vec![],
        };
        let rows: Vec<Vec<rsqldev_core::Cell>> = e.lines().map(str::to_string).chain(hint.iter().cloned()).filter(|l| !l.trim().is_empty()).map(|l| vec![Some(l)]).collect();
        self.grid.set(vec![rsqldev_core::Column { name: "Sessions are not available".into(), type_name: String::new(), numeric: false }], rows);
        self.status.set_label(if denied { "No privilege to list sessions (see below)" } else { "Could not list sessions (see below)" });
    }

    /// Kill the session under the grid cursor, after confirming the exact statement.
    fn kill(&self) {
        let r = self.grid.grid.get_grid_cursor_row();
        let row = match self.grid.st.borrow().rows.get(r.max(0) as usize) {
            Some(row) if r >= 0 => row.cur.clone(),
            _ => return crate::ui::error(&self.panel, "Select a session row first."),
        };
        let Some(sql) = dialect::kill_sql(self.db, &row) else { return crate::ui::error(&self.panel, "This row has no session id to kill.") };
        if !crate::ui::confirm_sql(&self.panel, &format!("Kill this session on {}?", self.conn_name), &sql) {
            return;
        }
        let (w, q) = (self.me.clone(), sql.clone());
        Conn::meta_query(&self.conn, &q, move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(_) => v.refresh(),
                Err(e) => crate::ui::error(&v.panel, &format!("Kill failed:\n{sql}\n\n{e}")),
            }
        });
    }
}
