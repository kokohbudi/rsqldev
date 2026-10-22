//! UI-side connection: owns the worker sessions and routes their replies to callbacks.
//!
//! Two sessions per connection, like SQL Developer's navigator vs worksheet split:
//! `work` runs worksheets and data tabs (open cursors, transactions), `meta` serves the tree
//! and object viewer metadata, always fully drained so it never blocks.

use rsqldev_core::config::{ConnConfig, Prefs};
use rsqldev_core::registry::{self, Kind};
use rsqldev_core::worker::{Cmd, Reply, Session};
use rsqldev_core::{Cell, Column, Event};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

pub type ConnRef = Rc<RefCell<Conn>>;
/// Shared handle to an open cursor; set to None to close it.
pub type Rows = Rc<RefCell<Option<Receiver<Event>>>>;

pub struct Done {
    pub result: Result<Option<u64>, String>,
    pub output: Vec<String>,
    pub elapsed: Duration,
}

/// Fully collected statement result.
#[derive(Default)]
pub struct Res {
    pub cols: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    pub affected: Option<u64>,
    pub output: Vec<String>,
}


pub struct Fail {
    pub error: String,
    pub missing_driver: Option<String>,
}

type DoneCb = Box<dyn FnOnce(Done)>;
type TxCb = Box<dyn FnOnce(Result<&'static str, String>)>;
type ReadyCb = Box<dyn FnOnce(Result<(), Fail>)>;

struct Lane {
    s: Session,
    up: bool,
    pending: HashMap<u64, DoneCb>,
    tx: VecDeque<TxCb>,
}

#[derive(PartialEq)]
enum State {
    Connecting,
    Up,
    Failed,
}

pub struct Conn {
    pub cfg: ConnConfig,
    pub kind: Kind,
    pub prefs: Prefs,
    /// Current schema, filled after connect.
    pub schema: String,
    work: Lane,
    meta: Option<Lane>,
    state: State,
    /// Connected and current schema known.
    ready: bool,
    schema_asked: bool,
    waiters: Vec<ReadyCb>,
    open_fetch: Option<Rows>,
}

impl Conn {
    pub fn open(cfg: ConnConfig, password: String, prefs: Prefs) -> ConnRef {
        let kind = registry::find(&cfg.driver).map(|d| d.backend).unwrap_or(Kind::Sqlite);
        let lane = |s| Lane { s, up: false, pending: HashMap::new(), tx: VecDeque::new() };
        let work = lane(Session::open(cfg.clone(), password.clone(), prefs.clone()));
        // DuckDB locks the file per database instance: one session only. In-memory SQLite too.
        let shared = kind == Kind::Duckdb || (kind == Kind::Sqlite && cfg.database.is_empty());
        let meta = (!shared).then(|| lane(Session::open(cfg.clone(), password, prefs.clone())));
        let c = Rc::new(RefCell::new(Conn { cfg, kind, prefs, schema: String::new(), work, meta, state: State::Connecting, ready: false, schema_asked: false, waiters: vec![], open_fetch: None }));
        let weak = Rc::downgrade(&c);
        crate::hub::every_tick(move || match weak.upgrade() {
            Some(c) => {
                Conn::pump(&c);
                true
            }
            None => false,
        });
        c
    }

    pub fn is_up(&self) -> bool {
        self.state == State::Up
    }

    pub fn failed(&self) -> bool {
        self.state == State::Failed
    }

    /// Run `f` once connected (immediately if already up).
    pub fn when_ready(this: &ConnRef, f: impl FnOnce(Result<(), Fail>) + 'static) {
        let mut c = this.borrow_mut();
        if c.ready {
            drop(c);
            f(Ok(()))
        } else {
            c.waiters.push(Box::new(f));
        }
    }

    fn pump(this: &ConnRef) {
        let mut calls: Vec<Box<dyn FnOnce()>> = vec![];
        {
            let mut c = this.borrow_mut();
            let c = &mut *c;
            let mut fail = None;
            for lane in std::iter::once(&mut c.work).chain(c.meta.as_mut()) {
                loop {
                    match lane.s.replies.try_recv() {
                        Ok(Reply::Connected) => lane.up = true,
                        Ok(Reply::ConnectFailed { error, missing_driver }) => fail = Some(Fail { error, missing_driver }),
                        Ok(Reply::Done { id, result, output, elapsed }) => {
                            if let Some(cb) = lane.pending.remove(&id) {
                                calls.push(Box::new(move || cb(Done { result, output, elapsed })));
                            }
                        }
                        Ok(Reply::TxDone(r)) => {
                            if let Some(cb) = lane.tx.pop_front() {
                                calls.push(Box::new(move || cb(r)));
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            for (_, cb) in lane.pending.drain() {
                                calls.push(Box::new(move || cb(Done { result: Err("connection closed".into()), output: vec![], elapsed: Duration::ZERO })));
                            }
                            break;
                        }
                    }
                }
            }
            if c.state == State::Connecting {
                if let Some(f) = fail {
                    c.state = State::Failed;
                    let err = f.error.clone();
                    let missing = f.missing_driver.clone();
                    let mut first = Some(f);
                    for w in c.waiters.drain(..) {
                        let f = first.take().unwrap_or_else(|| Fail { error: err.clone(), missing_driver: missing.clone() });
                        calls.push(Box::new(move || w(Err(f))));
                    }
                } else if c.work.up && c.meta.as_ref().is_none_or(|m| m.up) {
                    c.state = State::Up;
                }
            }
        }
        // The first ready step: learn the current schema, then wake waiters.
        let ask = {
            let mut c = this.borrow_mut();
            let ask = c.state == State::Up && !c.schema_asked;
            c.schema_asked |= ask;
            ask
        };
        if ask {
            let c2 = this.clone();
            let sql = rsqldev_core::dialect::current_schema_sql(this.borrow().kind);
            Conn::meta_query(this, sql, move |r| {
                let schema = r.ok().and_then(|r| r.rows.into_iter().next()?.into_iter().next()?).unwrap_or_default();
                let waiters = {
                    let mut c = c2.borrow_mut();
                    c.schema = schema;
                    c.ready = true;
                    std::mem::take(&mut c.waiters)
                };
                for w in waiters {
                    w(Ok(()));
                }
            });
        }
        for f in calls {
            f();
        }
    }

    fn lane(&mut self, meta: bool) -> &mut Lane {
        if meta && self.meta.is_some() { self.meta.as_mut().unwrap() } else { &mut self.work }
    }

    fn exec_on(this: &ConnRef, meta: bool, sql: &str, fetch: usize, done: impl FnOnce(Done) + 'static) -> Rows {
        let mut c = this.borrow_mut();
        if !meta {
            // A new worksheet statement closes the previous open cursor on the work session.
            if let Some(prev) = c.open_fetch.take() {
                prev.borrow_mut().take();
            }
        }
        let lane = c.lane(meta);
        let (id, rx) = lane.s.exec(sql, fetch);
        lane.pending.insert(id, Box::new(done));
        let rows: Rows = Rc::new(RefCell::new(Some(rx)));
        if !meta {
            c.open_fetch = Some(rows.clone());
        }
        rows
    }

    /// Worksheet / data-tab statement with fetch-on-demand rows.
    pub fn exec(this: &ConnRef, sql: &str, done: impl FnOnce(Done) + 'static) -> Rows {
        let fetch = this.borrow().prefs.fetch_size;
        Conn::exec_on(this, false, sql, fetch, done)
    }

    fn collect(this: &ConnRef, meta: bool, sql: &str, cb: impl FnOnce(Result<Res, String>) + 'static) {
        let acc: Rc<RefCell<(Vec<Column>, Vec<Vec<Cell>>)>> = Rc::default();
        let a2 = acc.clone();
        let rows = Conn::exec_on(this, meta, sql, 500, move |d| {
            let (cols, rows) = std::mem::take(&mut *a2.borrow_mut());
            cb(d.result.map(|affected| Res { cols, rows, affected, output: d.output }));
        });
        crate::hub::every_tick(move || {
            let mut r = rows.borrow_mut();
            let Some(rx) = r.as_ref() else { return false };
            loop {
                match rx.try_recv() {
                    Ok(Event::Columns(c)) => acc.borrow_mut().0 = c,
                    Ok(Event::Rows(mut b)) => acc.borrow_mut().1.append(&mut b),
                    Err(TryRecvError::Empty) => return true,
                    Err(TryRecvError::Disconnected) => {
                        r.take();
                        return false;
                    }
                }
            }
        });
    }

    /// Metadata query, all rows.
    pub fn meta_query(this: &ConnRef, sql: &str, cb: impl FnOnce(Result<Res, String>) + 'static) {
        Conn::collect(this, true, sql, cb)
    }

    /// Statement on the work session, all rows (script runner, DML posting).
    pub fn work_query(this: &ConnRef, sql: &str, cb: impl FnOnce(Result<Res, String>) + 'static) {
        Conn::collect(this, false, sql, cb)
    }

    /// DML with bind values on the work session (BLOB upload, long text); `cb` gets rows affected.
    pub fn work_binds(this: &ConnRef, sql: &str, binds: Vec<rsqldev_core::Bind>, cb: impl FnOnce(Result<Option<u64>, String>) + 'static) {
        let mut c = this.borrow_mut();
        if let Some(prev) = c.open_fetch.take() {
            prev.borrow_mut().take();
        }
        let id = c.work.s.exec_binds(sql, binds);
        c.work.pending.insert(id, Box::new(move |d: Done| cb(d.result)));
    }

    pub fn tx(this: &ConnRef, cmd: Cmd, cb: impl FnOnce(Result<&'static str, String>) + 'static) {
        let mut c = this.borrow_mut();
        if let Some(prev) = c.open_fetch.take() {
            prev.borrow_mut().take();
        }
        c.work.tx.push_back(Box::new(cb));
        c.work.s.send(cmd);
    }

    pub fn cancel(&self) {
        self.work.s.cancel();
    }
}
