//! Data Pump file transfer: dump files move between the server directory and this machine
//! over a session of their own (worksheets and metadata stay free). A modeless dialog shows
//! progress; the UI thread only polls channels from `hub::every_tick`, it never blocks.
//! Downloads land in `<file>.part` and are renamed only when complete, so an interrupted one
//! resumes from the part's size. A `<file>.part.json` sidecar records where the part came from:
//! it resumes only from the same file of the same server, and one folder never gets two
//! downloads of the same file at once.

use crate::conn::ConnRef;
use crate::ui;
use rsqldev_core::config::{self, ConnConfig};
use rsqldev_core::datapump::{self, PartInfo, UPLOAD_BLOCK, UPLOAD_CHUNK};
use rsqldev_core::worker::{Reply, Session};
use rsqldev_core::Bind;
use std::cell::{Cell, RefCell};
use std::collections::{HashSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;
use wxdragon::prelude::*;

/// `<to>/<file>.part`
pub fn part_path(to: &Path, file: &str) -> PathBuf {
    to.join(format!("{file}.part"))
}

/// `<to>/<file>.part.json`: the sidecar saying where the part's bytes came from.
pub fn sidecar_path(to: &Path, file: &str) -> PathBuf {
    to.join(format!("{file}.part.json"))
}

/// Bytes already downloaded into `part` (0 when absent).
pub fn resume_offset(part: &Path) -> u64 {
    std::fs::metadata(part).map(|m| m.len()).unwrap_or(0)
}

/// The server a connection points at (driver, host, port, service / TNS), for matching a part.
fn source_of(cfg: &ConnConfig) -> String {
    format!("{}|{}|{}|{}|{}", cfg.driver, cfg.host, cfg.port.map(|p| p.to_string()).unwrap_or_default(), cfg.database, cfg.tns)
}

thread_local! {
    /// Part files some download is writing (or has queued) right now.
    static ACTIVE: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new());
}

/// Reserve the part files of a download; Err(i) when `parts[i]` belongs to another download.
fn hold(parts: &[PathBuf]) -> Result<(), usize> {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        if let Some(i) = parts.iter().position(|p| a.contains(p)) {
            return Err(i);
        }
        a.extend(parts.iter().cloned());
        Ok(())
    })
}

fn release(parts: &[PathBuf]) {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        for p in parts {
            a.remove(p);
        }
    })
}

fn mb(n: u64) -> f64 {
    n as f64 / 1_048_576.0
}

/// The error plus, when known, the GRANT to ask for.
fn explain(err: &str, user: &str, dir: &str) -> String {
    match datapump::hint(err, user, dir) {
        Some(h) => format!("{err}\n{h}"),
        None => err.to_string(),
    }
}

/// Own session with the connection's config; asks for the password when not saved.
pub fn open_session(conn: &ConnRef) -> Option<Session> {
    let (cfg, prefs) = (conn.borrow().cfg.clone(), conn.borrow().prefs.clone());
    let pw = config::get_password(&cfg.name).or_else(|| ui::ask_password(&crate::app().frame, "Password:"))?;
    Some(Session::open(cfg, pw, prefs))
}

/// Modeless progress dialog. Closing it counts as Cancel.
struct Prog {
    dlg: Dialog,
    name: StaticText,
    gauge: Gauge,
    stat: StaticText,
    cancelled: Rc<Cell<bool>>,
}

impl Prog {
    fn new(title: &str) -> Prog {
        let app = crate::app();
        let dlg = Dialog::builder(&app.frame, title).with_style(DialogStyle::DefaultDialogStyle).with_size(460, 170).build();
        let name = StaticText::builder(&dlg).with_label("Connecting…").build();
        let gauge = Gauge::builder(&dlg).build();
        gauge.set_range(1000);
        let stat = StaticText::builder(&dlg).with_label(" ").build();
        let cancel = Button::builder(&dlg).with_label("Cancel").build();
        let cancelled = Rc::new(Cell::new(false));
        let c = cancelled.clone();
        cancel.on_click(move |_| c.set(true));
        let c = cancelled.clone();
        dlg.on_close(move |e| {
            c.set(true);
            e.skip(true);
        });
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add(&name, 0, SizerFlag::Expand | SizerFlag::All, 10);
        s.add(&gauge, 0, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 10);
        s.add(&stat, 0, SizerFlag::Expand | SizerFlag::All, 10);
        s.add(&cancel, 0, SizerFlag::AlignRight | SizerFlag::All, 10);
        dlg.set_sizer(s, true);
        dlg.centre();
        dlg.show(true);
        Prog { dlg, name, gauge, stat, cancelled }
    }

    fn show(&self, label: &str, done: u64, total: u64, since: Instant, base: u64) {
        self.name.set_label(label);
        self.gauge.set_value(if total == 0 { 0 } else { (done.min(total) as u128 * 1000 / total as u128) as i32 });
        let secs = since.elapsed().as_secs_f64().max(0.001);
        let rate = done.saturating_sub(base) as f64 / 1_048_576.0 / secs;
        let tail = if self.cancelled.get() { "  (cancelling…)" } else { "" };
        self.stat.set_label(&format!("{:.1} / {:.1} MB   {:.1} MB/s{tail}", mb(done), mb(total), rate));
    }

    fn close(&self) {
        self.dlg.destroy();
    }
}

type DownDone = Box<dyn FnOnce(Result<Vec<PathBuf>, String>)>;

struct Cur {
    name: String,
    part: PathBuf,
    sidecar: PathBuf,
    id: u64,
    rx: Receiver<(u64, u64)>,
    cancel: Arc<AtomicBool>,
    base: u64,
    /// Server length recorded when the resumed part was started (None = unknown).
    expect: Option<u64>,
    first_len: Option<u64>,
    /// The resumed part does not fit the server file (shorter, or another length than recorded).
    mismatch: bool,
    restarted: bool,
    last: (u64, u64),
    started: Instant,
}

struct Down {
    sess: Session,
    dir: String,
    user: String,
    /// The connection's server, written into the sidecars.
    source: String,
    to: PathBuf,
    queue: VecDeque<String>,
    /// Part files reserved for this download (see `hold`).
    held: Vec<PathBuf>,
    out: Vec<PathBuf>,
    cur: Option<Cur>,
    prog: Prog,
    done: Option<DownDone>,
}

impl Down {
    fn end(&mut self, r: Result<Vec<PathBuf>, String>) -> bool {
        release(&std::mem::take(&mut self.held));
        self.prog.close();
        if let Some(d) = self.done.take() {
            d(r);
        }
        false
    }

    /// Start the next queued file (a file the user would not overwrite is skipped); false when
    /// the whole transfer ended.
    fn next(&mut self) -> bool {
        loop {
            let Some(name) = self.queue.pop_front() else {
                let out = self.out.clone();
                return self.end(Ok(out));
            };
            let fin = self.to.join(&name);
            if fin.exists() && !ui::confirm(&self.prog.dlg, &format!("{} exists. Overwrite?\n\nNo skips this file.", fin.display())) {
                continue;
            }
            self.start(name, false);
            return true;
        }
    }

    /// Where a part resumes: its size when the sidecar says it came from this very file, else
    /// ask (default: start over). Also the server length recorded with it.
    fn resume_from(&self, name: &str, part: &Path, sidecar: &Path) -> (u64, Option<u64>) {
        let have = resume_offset(part);
        if have == 0 {
            return (0, None);
        }
        let saved = std::fs::read_to_string(sidecar).ok().and_then(|s| PartInfo::from_json(&s));
        match saved {
            Some(p) if p.matches(&self.source, &self.dir, name) => (have, p.len),
            _ => {
                let q = format!("Resume {:.1} MB from an earlier download of a different source?\n\n{} was not written by a download of {name} from {} on this server. No starts over.", mb(have), part.display(), self.dir);
                if ui::confirm_no(&self.prog.dlg, &q) { (have, None) } else { (0, None) }
            }
        }
    }

    fn sidecar(&self, name: &str, len: Option<u64>) -> PartInfo {
        PartInfo { source: self.source.clone(), directory: self.dir.clone(), file: name.to_string(), len }
    }

    fn start(&mut self, name: String, restarted: bool) {
        let part = part_path(&self.to, &name);
        let sidecar = sidecar_path(&self.to, &name);
        let (base, expect) = if restarted { (0, None) } else { self.resume_from(&name, &part, &sidecar) };
        if base == 0 {
            // The worker appends: a part that is not resumed must go first.
            let _ = std::fs::remove_file(&part);
        }
        let _ = std::fs::write(&sidecar, self.sidecar(&name, expect).to_json());
        let (id, rx, cancel) = self.sess.download(&self.dir, &name, part.clone(), base);
        self.cur = Some(Cur { name, part, sidecar, id, rx, cancel, base, expect, first_len: None, mismatch: false, restarted, last: (base, 0), started: Instant::now() });
    }

    fn drain(&mut self) {
        let mut learned = None;
        if let Some(c) = &mut self.cur {
            while let Ok((d, l)) = c.rx.try_recv() {
                c.last = (d, l);
                if c.first_len.is_none() {
                    c.first_len = Some(l);
                    if c.base > 0 && (l < c.base || c.expect.is_some_and(|e| e != l)) {
                        // Another file than the one the part started from: stop, then start over.
                        c.mismatch = true;
                        c.cancel.store(true, Ordering::Relaxed);
                    } else if c.expect.is_none() {
                        learned = Some((c.name.clone(), c.sidecar.clone(), l));
                    }
                }
            }
            self.prog.show(&c.name, c.last.0, c.last.1, c.started, c.base);
        }
        if let Some((name, path, l)) = learned {
            let _ = std::fs::write(&path, self.sidecar(&name, Some(l)).to_json());
            if let Some(c) = &mut self.cur {
                c.expect = Some(l);
            }
        }
    }

    /// One poll; false once finished.
    fn tick(&mut self) -> bool {
        if self.prog.cancelled.get() {
            match &self.cur {
                Some(c) => c.cancel.store(true, Ordering::Relaxed),
                None => return self.end(Err("Cancelled.".into())),
            }
        }
        self.drain();
        loop {
            let r = match self.sess.replies.try_recv() {
                Ok(r) => r,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return self.end(Err("The download session ended unexpectedly.".into())),
            };
            match r {
                Reply::Connected => {
                    if !self.next() {
                        return false;
                    }
                }
                Reply::ConnectFailed { error, .. } => return self.end(Err(explain(&error, &self.user, &self.dir))),
                Reply::Done { id, result, .. } if self.cur.as_ref().is_some_and(|c| c.id == id) => {
                    self.drain();
                    let c = self.cur.take().unwrap();
                    // The part came from another version of the file (it was replaced): start over
                    // once. Its bytes are never renamed into place, even when the worker reports Ok
                    // or the user cancelled at that moment.
                    if c.mismatch {
                        let _ = std::fs::remove_file(&c.part);
                        let _ = std::fs::remove_file(&c.sidecar);
                        if !c.restarted && !self.prog.cancelled.get() {
                            self.start(c.name, true);
                            continue;
                        }
                        return self.end(Err(if self.prog.cancelled.get() {
                            "Cancelled.".into()
                        } else {
                            format!("{} changed on the server during the download; start it again.", c.name)
                        }));
                    }
                    match result {
                        Ok(_) => {
                            let fin = self.to.join(&c.name);
                            let _ = std::fs::remove_file(&fin);
                            if let Err(e) = std::fs::rename(&c.part, &fin) {
                                return self.end(Err(format!("Cannot rename {}: {e}", c.part.display())));
                            }
                            let _ = std::fs::remove_file(&c.sidecar);
                            self.out.push(fin);
                            if !self.next() {
                                return false;
                            }
                        }
                        Err(_) if self.prog.cancelled.get() => {
                            return self.end(Err(format!("Cancelled. {} kept, the next download resumes from it.", c.part.display())));
                        }
                        Err(e) => return self.end(Err(explain(&e, &self.user, &self.dir))),
                    }
                }
                _ => {}
            }
        }
        true
    }
}

/// Download `files` of server directory `dir` into folder `to`, one after another. Resumes a
/// `.part` left by an earlier attempt. `done` gets the final paths or the error.
/// A second download of a file already downloading into the same folder is refused.
pub fn download(conn: &ConnRef, dir: String, files: Vec<String>, to: PathBuf, done: impl FnOnce(Result<Vec<PathBuf>, String>) + 'static) {
    let held: Vec<PathBuf> = files.iter().map(|f| part_path(&to, f)).collect();
    if let Err(i) = hold(&held) {
        return done(Err(format!("{} is already downloading into {}.", files[i], to.display())));
    }
    let Some(sess) = open_session(conn) else {
        release(&held);
        return done(Err("Cancelled.".into()));
    };
    let (user, source) = {
        let c = conn.borrow();
        (c.cfg.user.clone(), source_of(&c.cfg))
    };
    let mut st = Down { sess, dir, user, source, to, queue: files.into(), held, out: vec![], cur: None, prog: Prog::new("Download"), done: Some(Box::new(done)) };
    crate::hub::every_tick(move || st.tick());
}

type UpDone = Box<dyn FnOnce(Result<(), String>)>;

struct Up {
    sess: Session,
    dir: String,
    file: String,
    user: String,
    f: std::fs::File,
    len: u64,
    sent: u64,
    first: bool,
    pending: Option<u64>,
    started: Instant,
    prog: Prog,
    done: Option<UpDone>,
}

impl Up {
    fn end(&mut self, r: Result<(), String>) -> bool {
        self.prog.close();
        if let Some(d) = self.done.take() {
            d(r);
        }
        false
    }

    /// Read the next chunk and send it; false when the transfer ended.
    fn send(&mut self) -> bool {
        let mut buf = vec![0u8; UPLOAD_CHUNK];
        let mut n = 0;
        while n < buf.len() {
            match self.f.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) => return self.end(Err(format!("Cannot read the file: {e}"))),
            }
        }
        if n == 0 {
            if self.sent < self.len {
                return self.end(Err(format!(
                    "The local file got shorter while uploading ({} of {} bytes sent). {} on the server is incomplete.",
                    self.sent, self.len, self.file
                )));
            }
            return self.end(Ok(()));
        }
        buf.truncate(n);
        let mode = if self.first { "wb" } else { "ab" };
        self.first = false;
        self.sent += n as u64;
        self.pending = Some(self.sess.exec_binds(UPLOAD_BLOCK, vec![Bind::Text(self.dir.clone()), Bind::Text(self.file.clone()), Bind::Text(mode.into()), Bind::Bytes(buf)]));
        true
    }

    fn tick(&mut self) -> bool {
        loop {
            let r = match self.sess.replies.try_recv() {
                Ok(r) => r,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return self.end(Err(format!("The upload session ended unexpectedly. {} on the server may be incomplete.", self.file))),
            };
            match r {
                Reply::Connected => {
                    if !self.send() {
                        return false;
                    }
                }
                Reply::ConnectFailed { error, .. } => return self.end(Err(explain(&error, &self.user, &self.dir))),
                Reply::Done { id, result, .. } if self.pending == Some(id) => {
                    self.pending = None;
                    if let Err(e) = result {
                        return self.end(Err(explain(&e, &self.user, &self.dir)));
                    }
                    if self.sent >= self.len {
                        return self.end(Ok(()));
                    }
                    if self.prog.cancelled.get() {
                        return self.end(Err(format!("Cancelled. {} on the server is incomplete.", self.file)));
                    }
                    if !self.send() {
                        return false;
                    }
                }
                _ => {}
            }
        }
        if self.pending.is_some() || self.sent > 0 {
            self.prog.show(&self.file, self.sent, self.len, self.started, 0);
        }
        if self.pending.is_none() && self.prog.cancelled.get() {
            return self.end(Err("Cancelled.".into()));
        }
        true
    }
}

/// Upload local file `from` as `file` into server directory `dir`, 4 MiB per round trip.
pub fn upload(conn: &ConnRef, dir: String, from: PathBuf, file: String, done: impl FnOnce(Result<(), String>) + 'static) {
    let (f, len) = match std::fs::File::open(&from).and_then(|f| f.metadata().map(|m| (f, m.len()))) {
        Ok(x) => x,
        Err(e) => return done(Err(format!("Cannot open {}: {e}", from.display()))),
    };
    if len == 0 {
        return done(Err(format!("{} is empty.", from.display())));
    }
    let Some(sess) = open_session(conn) else { return done(Err("Cancelled.".into())) };
    let user = conn.borrow().cfg.user.clone();
    let mut st = Up { sess, dir, file, user, f, len, sent: 0, first: true, pending: None, started: Instant::now(), prog: Prog::new("Upload"), done: Some(Box::new(done)) };
    crate::hub::every_tick(move || st.tick());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_and_resume_offset() {
        let d = std::env::temp_dir().join(format!("rsq-tr-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = part_path(&d, "a.dmp");
        assert_eq!(p, d.join("a.dmp.part"));
        assert_eq!(resume_offset(&p), 0);
        std::fs::write(&p, [0u8; 10]).unwrap();
        assert_eq!(resume_offset(&p), 10);
        assert_eq!(sidecar_path(&d, "a.dmp"), d.join("a.dmp.part.json"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn one_download_per_part_file() {
        let (a, b) = (PathBuf::from("/x/a.dmp.part"), PathBuf::from("/x/b.dmp.part"));
        assert_eq!(hold(std::slice::from_ref(&a)), Ok(()));
        assert_eq!(hold(&[b.clone(), a.clone()]), Err(1), "a is taken");
        assert_eq!(hold(std::slice::from_ref(&b)), Ok(()), "a refused request reserves nothing");
        release(&[a.clone(), b]);
        assert_eq!(hold(std::slice::from_ref(&a)), Ok(()));
        release(&[a]);
    }
}
