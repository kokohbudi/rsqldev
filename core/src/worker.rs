//! One thread per open connection. The UI sends [`Cmd`]s and polls [`Reply`]s.
//!
//! Fetch-on-scroll: rows go through a zero-capacity channel, so the worker only fetches the
//! next batch when the UI receives one. Dropping the receiver ends the cursor.

use crate::config::{ConnConfig, Prefs};
use crate::{Backend, Bind, Canceller, Event};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static BUSY: AtomicUsize = AtomicUsize::new(0);

/// Connection threads currently working on a database: connecting, executing, fetching.
/// An open cursor waiting for the UI to ask for more rows does not count. Drives the UI's
/// activity bar.
pub fn busy() -> usize {
    BUSY.load(Ordering::Relaxed)
}

/// Counts the current thread as busy while alive.
struct Busy;

impl Busy {
    fn new() -> Busy {
        BUSY.fetch_add(1, Ordering::Relaxed);
        Busy
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::Relaxed);
    }
}

pub enum Cmd {
    Exec { id: u64, sql: String, fetch: usize, rows: SyncSender<Event> },
    /// DML with bind values (BLOB upload, long text). Replies `Done`.
    ExecBinds { id: u64, sql: String, binds: Vec<Bind> },
    Commit,
    Rollback,
    Autocommit(bool),
    Output(bool),
}

#[derive(Debug)]
pub enum Reply {
    Connected,
    /// `missing_driver` is set when the native client must be downloaded first.
    ConnectFailed { error: String, missing_driver: Option<String> },
    Done { id: u64, result: Result<Option<u64>, String>, output: Vec<String>, elapsed: Duration },
    TxDone(Result<&'static str, String>),
}

pub struct Session {
    tx: Sender<Cmd>,
    pub replies: Receiver<Reply>,
    cancel: Arc<Mutex<Option<Canceller>>>,
    next_id: u64,
}

impl Session {
    pub fn open(cfg: ConnConfig, password: String, prefs: Prefs) -> Session {
        let (tx, rx) = channel::<Cmd>();
        let (rtx, replies) = channel();
        let cancel: Arc<Mutex<Option<Canceller>>> = Arc::default();
        let c2 = cancel.clone();
        std::thread::Builder::new()
            .name(format!("conn {}", cfg.name))
            .spawn(move || {
                let connecting = Busy::new();
                let mut b = match crate::backend::connect(&cfg, &password, &prefs) {
                    Ok(b) => b,
                    Err(e) => {
                        let missing = e.downcast_ref::<crate::backend::DriverMissing>().map(|m| m.0.clone());
                        let _ = rtx.send(Reply::ConnectFailed { error: format!("{e:#}"), missing_driver: missing });
                        return;
                    }
                };
                drop(connecting);
                *c2.lock().unwrap() = b.canceller();
                let _ = rtx.send(Reply::Connected);
                for cmd in rx {
                    let r = {
                        let _busy = Busy::new();
                        run(b.as_mut(), cmd)
                    };
                    if rtx.send(r).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn connection thread");
        Session { tx, replies, cancel, next_id: 1 }
    }

    /// Start a statement. Returns its id and the row receiver (Columns first, then Rows batches).
    pub fn exec(&mut self, sql: &str, fetch: usize) -> (u64, Receiver<Event>) {
        let (rows, rx) = sync_channel(0);
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.tx.send(Cmd::Exec { id, sql: sql.into(), fetch, rows });
        (id, rx)
    }

    /// Start DML with bind values. Returns its id.
    pub fn exec_binds(&mut self, sql: &str, binds: Vec<Bind>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.tx.send(Cmd::ExecBinds { id, sql: sql.into(), binds });
        id
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }

    pub fn cancel(&self) {
        if let Some(c) = self.cancel.lock().unwrap().as_ref() {
            c();
        }
    }
}

fn run(b: &mut dyn Backend, cmd: Cmd) -> Reply {
    let tx = |r: crate::Result<()>, ok: &'static str| Reply::TxDone(r.map(|_| ok).map_err(|e| format!("{e:#}")));
    match cmd {
        Cmd::Exec { id, sql, fetch, rows } => {
            let t = Instant::now();
            // Idle while the UI holds the next batch back (fetch-on-scroll).
            let result = b
                .execute(&sql, fetch, &mut |e| {
                    BUSY.fetch_sub(1, Ordering::Relaxed);
                    let ok = rows.send(e).is_ok();
                    BUSY.fetch_add(1, Ordering::Relaxed);
                    ok
                })
                .map_err(|e| format!("{e:#}"));
            let output = b.take_output().unwrap_or_default();
            Reply::Done { id, result, output, elapsed: t.elapsed() }
        }
        Cmd::ExecBinds { id, sql, binds } => {
            let t = Instant::now();
            let result = b.execute_binds(&sql, &binds).map_err(|e| format!("{e:#}"));
            Reply::Done { id, result, output: vec![], elapsed: t.elapsed() }
        }
        Cmd::Commit => tx(b.commit(), "Commit complete."),
        Cmd::Rollback => tx(b.rollback(), "Rollback complete."),
        Cmd::Autocommit(on) => tx(b.set_autocommit(on), "Autocommit changed."),
        Cmd::Output(on) => tx(b.enable_output(on), if on { "Server output enabled." } else { "Server output disabled." }),
    }
}
