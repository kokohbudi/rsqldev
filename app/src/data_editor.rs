//! Grid edits → DML on the work session, shared by a table's Data tab and editable worksheet
//! results: pending rows become INSERT / UPDATE / DELETE, posted in order with a log line each,
//! then COMMIT (Commit Changes) or row by row ("Post edits on row change").

use crate::conn::{Conn, ConnRef};
use crate::grid::{ResultGrid, Row};
use rsqldev_core::registry::Kind;
use rsqldev_core::worker::Cmd;
use rsqldev_core::{Bind, dialect, dml};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// One generated DML statement and the grid row it came from (for "Row n:" errors).
#[derive(Clone)]
struct Stmt {
    row: usize,
    sql: String,
    binds: Vec<Bind>,
}

pub struct Editor {
    conn: ConnRef,
    db: Kind,
    owner: String,
    name: String,
    grid: ResultGrid,
    /// Hidden row-locator column (ROWID, ctid, rowid), first in the grid.
    hidden: Option<&'static str>,
    pk: RefCell<Vec<String>>,
    busy: Cell<bool>,
    /// Rows already posted (row change) and not yet committed.
    posted: Cell<bool>,
    log: Box<dyn Fn(&str)>,
    /// Show the rows again after COMMIT / ROLLBACK.
    reload: Box<dyn Fn()>,
    me: Weak<Editor>,
}

impl Editor {
    /// Edits of `grid` go to table `owner.name` (stored names). Looks up the primary key, the
    /// edit key when there is no stable row locator.
    pub fn new(conn: ConnRef, owner: &str, name: &str, grid: ResultGrid, hidden: Option<&'static str>, log: impl Fn(&str) + 'static, reload: impl Fn() + 'static) -> Rc<Editor> {
        let db = conn.borrow().kind;
        let ed = Rc::new_cyclic(|me| Editor {
            conn,
            db,
            owner: owner.into(),
            name: name.into(),
            grid,
            hidden,
            pk: RefCell::default(),
            busy: Cell::new(false),
            posted: Cell::new(false),
            log: Box::new(log),
            reload: Box::new(reload),
            me: me.clone(),
        });
        if let Some(sql) = dialect::pk_sql(db, owner, name) {
            let me = ed.me.clone();
            Conn::meta_query(&ed.conn, &sql, move |r| {
                if let (Some(me), Ok(r)) = (me.upgrade(), r) {
                    *me.pk.borrow_mut() = r.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect();
                }
            });
        }
        let me = ed.me.clone();
        ed.grid.st.borrow_mut().on_row_leave = Some(Rc::new(move |r| {
            if let Some(me) = me.upgrade() {
                if crate::app().prefs().post_on_row_change {
                    me.post_row(r)
                }
            }
        }));
        ed
    }

    /// The rows were (re)loaded: nothing is posted any more.
    pub fn loaded(&self) {
        self.posted.set(false);
    }

    /// Edits not committed yet (in the grid or already posted).
    pub fn pending(&self) -> bool {
        self.posted.get() || self.grid.st.borrow().rows.iter().any(|r| r.dirty())
    }

    pub fn log(&self, line: &str) {
        (self.log)(line)
    }

    /// Generated DML for pending grid edits (rows picked by `want`), each with its grid row and
    /// the values it binds (staged BLOB files, and text too long for an Oracle literal).
    fn pending_dml(&self, want: impl Fn(usize) -> bool) -> Vec<Stmt> {
        let st = self.grid.st.borrow();
        let table = dialect::qualified(self.db, &self.owner, &self.name);
        let key = dml::Key::new(&st.cols, self.hidden, &self.pk.borrow());
        let skip = self.hidden.map(|_| 0);
        let mut out = vec![];
        for (ri, row) in st.rows.iter().enumerate().filter(|(ri, r)| want(*ri) && !r.posted) {
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
                (true, false) => out.push(Stmt { row: ri, sql: dml::insert(self.db, &table, &st.cols, skip, cur, &cols), binds: binds() }),
                (false, true) => out.push(Stmt { row: ri, sql: dml::delete(self.db, &table, &st.cols, &key, row.original()), binds: vec![] }),
                (false, false) if row.dirty() => out.extend(dml::update(self.db, &table, &st.cols, &key, skip, row.original(), cur, &cols).map(|sql| Stmt { row: ri, sql, binds: binds() })),
                (false, false) => {}
            }
        }
        out
    }

    /// Commit Changes (F11): post every pending row, then COMMIT. False = nothing to commit.
    pub fn commit(&self) -> bool {
        if self.busy.get() {
            return true;
        }
        let stmts = self.pending_dml(|_| true);
        if stmts.is_empty() && !self.posted.get() {
            return false;
        }
        self.busy.set(true);
        self.post(stmts, 0, true);
        true
    }

    /// "Post edits on row change": the row the cursor left goes to the database now.
    fn post_row(&self, r: usize) {
        if self.busy.get() {
            return;
        }
        let stmts = self.pending_dml(|i| i == r);
        if stmts.is_empty() {
            return;
        }
        self.busy.set(true);
        self.post(stmts, 0, false);
    }

    /// Run `stmts` in order on the worksheet session, logging each; then COMMIT (`commit`) or
    /// mark the rows posted. The first error rolls the whole batch back.
    fn post(&self, stmts: Vec<Stmt>, i: usize, commit: bool) {
        let me = self.me.clone();
        if i == stmts.len() {
            if !commit {
                self.busy.set(false);
                self.posted.set(true);
                let rows: Vec<usize> = stmts.iter().map(|s| s.row).collect();
                self.grid.mark_posted(&rows);
                return;
            }
            return Conn::tx(&self.conn, Cmd::Commit, move |r| {
                let Some(me) = me.upgrade() else { return };
                me.busy.set(false);
                match r {
                    Ok(_) => {
                        me.log(&format!("Commit Successful: {} statement(s)", stmts.len()));
                        (me.reload)();
                    }
                    Err(e) => {
                        me.log(&format!("Commit failed: {e}"));
                        crate::ui::error(&me.grid.grid, &e)
                    }
                }
            });
        }
        if i == 0 && crate::app().prefs().autocommit {
            self.log("Autocommit is on: every statement is committed as it runs, Rollback cannot undo it.");
        }
        let Stmt { row, sql, binds } = stmts[i].clone();
        self.log(&format!("{sql};"));
        let run = sql.clone();
        let done = move |r: Result<Option<u64>, String>| {
            let Some(me) = me.upgrade() else { return };
            let fail = |msg: String| {
                me.busy.set(false);
                me.posted.set(false);
                me.log(&format!("Row {}: {msg}", row + 1));
                me.log("Rollback Done");
                crate::ui::error(&me.grid.grid, &format!("One error saving changes to table {}:\nRow {}: {msg}\n\n{sql}", me.name, row + 1));
                Conn::tx(&me.conn, Cmd::Rollback, |_| {});
            };
            match r {
                Ok(Some(0)) => fail("Data got committed in another/same session, cannot update row.".into()),
                Ok(n) => {
                    if let Some(n) = n {
                        me.log(&format!("{n} row(s) affected"));
                    }
                    me.post(stmts, i + 1, commit)
                }
                Err(e) => fail(e),
            }
        };
        if binds.is_empty() {
            Conn::work_query(&self.conn, &run, move |r| done(r.map(|r| r.affected)));
        } else {
            Conn::work_binds(&self.conn, &run, binds, done);
        }
    }

    /// Rollback Changes (F12): asks first when there is something to lose, then reloads.
    pub fn rollback(&self) {
        if self.pending() && !crate::ui::confirm(&self.grid.grid, "Do you want to rollback changes ?") {
            return;
        }
        let me = self.me.clone();
        Conn::tx(&self.conn, Cmd::Rollback, move |_| {
            if let Some(me) = me.upgrade() {
                me.log("Rollback Done");
                (me.reload)();
            }
        });
    }
}
