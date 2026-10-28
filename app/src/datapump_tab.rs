//! Tools ▸ Data Pump Jobs & Files: server jobs (stop, resume, kill, clean up, log) on top, the dump,
//! log and SQL files this app knows about below (download, view, delete). Runs on the metadata session;
//! downloads use their own session (`transfer`).

use crate::conn::{Conn, ConnRef};
use crate::grid::ResultGrid;
use crate::{transfer, ui};
use rsqldev_core::datapump::{self, FileKind, FileRecord};
use rsqldev_core::{Cell, Column};
use std::cell::{Cell as StdCell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};
use wxdragon::prelude::*;

/// One displayed file row (a `%U` record shows one row per existing piece).
#[derive(Clone)]
struct Disp {
    dir: String,
    file: String,
    /// The recorded name (with `%U` when it is a template).
    tmpl: String,
    job: String,
    kind: FileKind,
    created: String,
    exists: Option<bool>,
    size: Option<u64>,
}

struct Probe {
    todo: VecDeque<FileRecord>,
    cur: Option<(FileRecord, Vec<String>, usize)>,
    out: Vec<Disp>,
    err: Option<String>,
}

pub struct DataPumpTab {
    pub panel: Panel,
    pub conn_name: String,
    conn: ConnRef,
    jobs: ResultGrid,
    files: ResultGrid,
    jstatus: StaticText,
    fstatus: StaticText,
    auto: CheckBox,
    /// (DB_UNIQUE_NAME/CON_NAME or empty, USER) once read.
    env: RefCell<Option<(String, String)>>,
    disp: RefCell<Vec<Disp>>,
    busy: StdCell<bool>,
    jobs_busy: StdCell<bool>,
    last_done: StdCell<Instant>,
    /// Selected row keys: (owner, job) and (directory, file); they survive a refresh.
    jkey: RefCell<Option<Key>>,
    fkey: RefCell<Option<Key>>,
    restoring: StdCell<bool>,
    me: Weak<DataPumpTab>,
}

const JOB_KEY: (&str, &str) = ("owner", "job_name");
const FILE_KEY: (&str, &str) = ("directory", "file");

fn col(n: &str) -> Column {
    Column { name: n.into(), type_name: String::new(), numeric: false }
}

fn human(n: u64) -> String {
    let f = n as f64;
    if f >= 1_073_741_824.0 {
        format!("{:.2} GB", f / 1_073_741_824.0)
    } else if f >= 1_048_576.0 {
        format!("{:.1} MB", f / 1_048_576.0)
    } else {
        format!("{:.1} KB", f / 1024.0)
    }
}

fn kind_label(k: FileKind) -> &'static str {
    match k {
        FileKind::Dump => "dump",
        FileKind::Log => "log",
        FileKind::SqlFile => "sql",
        FileKind::Upload => "upload",
        FileKind::Other => "other",
    }
}

/// Cell `name` of grid row `row`.
fn row_cell(g: &ResultGrid, row: usize, name: &str) -> Option<String> {
    let st = g.st.borrow();
    let i = st.cols.iter().position(|c| c.name.eq_ignore_ascii_case(name))?;
    st.rows.get(row)?.cur.get(i).cloned().flatten()
}

type Key = (String, String);

/// Key (two columns) of grid row `row`.
fn row_key(g: &ResultGrid, row: usize, c: (&str, &str)) -> Option<Key> {
    Some((row_cell(g, row, c.0)?, row_cell(g, row, c.1)?))
}

/// Index of the grid row with this key.
fn find_row(g: &ResultGrid, key: &Key, c: (&str, &str)) -> Option<usize> {
    let n = g.st.borrow().rows.len();
    (0..n).find(|&i| row_key(g, i, c).as_ref() == Some(key))
}

fn lines_grid(g: &ResultGrid, title: &str, lines: Vec<String>) {
    let rows: Vec<Vec<Cell>> = lines.into_iter().filter(|l| !l.trim().is_empty()).map(|l| vec![Some(l)]).collect();
    g.set(vec![col(title)], rows);
}

/// Download text file `file` of server directory `dir` to a private temp folder, show it, then run
/// `then` (also when it could not be fetched). `user` names the grantee in error hints.
pub(crate) fn view_server_file(conn: &ConnRef, user: String, dir: String, file: String, then: impl FnOnce() + 'static) {
    let app = crate::app();
    if !datapump::valid_name(&dir) || !datapump::valid_name(&file) {
        ui::error(&app.frame, &format!("Invalid name: {dir} / {file}"));
        return then();
    }
    let tmp = match rsqldev_core::lob::private_dir("rsqldev-dp") {
        Ok(d) => d,
        Err(e) => {
            ui::error(&app.frame, &format!("Cannot create a temporary folder: {e}"));
            return then();
        }
    };
    let path = tmp.join(&file);
    let (title, cleanup, dir2) = (file.clone(), tmp.clone(), dir.clone());
    transfer::download(conn, dir, vec![file], tmp, move |r| {
        let app = crate::app();
        let text = r.and_then(|_| std::fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display())));
        let _ = std::fs::remove_dir_all(&cleanup);
        match text {
            Ok(b) => show_text(&title, &String::from_utf8_lossy(&b)),
            Err(e) => {
                let e = match datapump::hint(&e, &user, &dir2) {
                    Some(h) if !e.contains(&h) => format!("{e}\n\n{h}"),
                    _ => e,
                };
                ui::error(&app.frame, &e)
            }
        }
        then();
    });
}

/// Read-only monospace text in a modal dialog.
fn show_text(title: &str, text: &str) {
    let app = crate::app();
    let dlg = Dialog::builder(&app.frame, title).with_style(DialogStyle::DefaultDialogStyle).with_size(820, 600).build();
    // Logs and SQL files alike: highlighted like the worksheet.
    let t = crate::editor::sql_view(&dlg, text);
    let close = Button::builder(&dlg).with_label("Close").build();
    close.on_click(move |_| dlg.end_modal(ID_OK));
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add(&t, 1, SizerFlag::Expand | SizerFlag::All, 8);
    s.add(&close, 0, SizerFlag::AlignRight | SizerFlag::All, 8);
    dlg.set_sizer(s, true);
    dlg.centre();
    dlg.show_modal();
    dlg.destroy();
}

impl DataPumpTab {
    pub fn new(parent: &dyn WxWidget, conn: ConnRef) -> Rc<DataPumpTab> {
        let conn_name = conn.borrow().cfg.name.clone();
        let panel = Panel::builder(parent).build();
        let split = SplitterWindow::builder(&panel).build();
        let jp = Panel::builder(&split).build();
        let fp = Panel::builder(&split).build();

        let jbar = BoxSizer::builder(Orientation::Horizontal).build();
        let jrefresh = crate::icons::tool(&jp, "refresh", "Refresh");
        let stop = Button::builder(&jp).with_label("Stop…").build();
        let resume = Button::builder(&jp).with_label("Resume…").build();
        let kill = Button::builder(&jp).with_label("Kill…").build();
        let clean = Button::builder(&jp).with_label("Clean Up…").build();
        let vlog = Button::builder(&jp).with_label("View Log").build();
        let auto = CheckBox::builder(&jp).with_label("Auto-refresh (5 s)").build();
        auto.set_value(true);
        let jstatus = StaticText::builder(&jp).with_label(" ").build();
        jstatus.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        jbar.add(&jrefresh, 0, SizerFlag::All, 3);
        for b in [&stop, &resume, &kill, &clean, &vlog] {
            jbar.add(b, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        }
        jbar.add(&auto, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 10);
        jbar.add(&jstatus, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 12);
        let jobs = ResultGrid::new(&jp);
        jobs.st.borrow_mut().db = Some(rsqldev_core::registry::Kind::Oracle);
        let js = BoxSizer::builder(Orientation::Vertical).build();
        js.add_sizer(&jbar, 0, SizerFlag::Expand, 0);
        js.add(&jobs.grid, 1, SizerFlag::Expand, 0);
        jp.set_sizer(js, true);

        let fbar = BoxSizer::builder(Orientation::Horizontal).build();
        let frefresh = crate::icons::tool(&fp, "refresh", "Refresh");
        let down = Button::builder(&fp).with_label("Download…").build();
        let view = Button::builder(&fp).with_label("View").build();
        let del = Button::builder(&fp).with_label("Delete from Server…").build();
        let byname = Button::builder(&fp).with_label("Download by Name…").build();
        let fstatus = StaticText::builder(&fp).with_label(" ").build();
        fstatus.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        fbar.add(&frefresh, 0, SizerFlag::All, 3);
        for b in [&down, &view, &del, &byname] {
            fbar.add(b, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 3);
        }
        fbar.add(&fstatus, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 12);
        let files = ResultGrid::new(&fp);
        files.st.borrow_mut().db = Some(rsqldev_core::registry::Kind::Oracle);
        let fs = BoxSizer::builder(Orientation::Vertical).build();
        fs.add_sizer(&fbar, 0, SizerFlag::Expand, 0);
        fs.add(&files.grid, 1, SizerFlag::Expand, 0);
        fp.set_sizer(fs, true);

        split.split_horizontally(&jp, &fp, 260);
        split.set_minimum_pane_size(100);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add(&split, 1, SizerFlag::Expand, 0);
        panel.set_sizer(s, true);

        let v = Rc::new_cyclic(|me| DataPumpTab {
            panel,
            conn_name,
            conn,
            jobs,
            files,
            jstatus,
            fstatus,
            auto,
            env: RefCell::new(None),
            disp: RefCell::new(vec![]),
            busy: StdCell::new(false),
            jobs_busy: StdCell::new(false),
            last_done: StdCell::new(Instant::now()),
            jkey: RefCell::new(None),
            fkey: RefCell::new(None),
            restoring: StdCell::new(false),
            me: me.clone(),
        });
        let acts: [(Button, fn(&DataPumpTab)); 9] = [
            (stop, DataPumpTab::stop),
            (resume, DataPumpTab::resume),
            (kill, DataPumpTab::kill),
            (clean, DataPumpTab::cleanup),
            (vlog, DataPumpTab::view_job_log),
            (down, DataPumpTab::download),
            (view, DataPumpTab::view),
            (del, DataPumpTab::delete),
            (byname, DataPumpTab::download_by_name),
        ];
        for (b, f) in acts {
            let w = v.me.clone();
            b.on_click(move |_| {
                if let Some(v) = w.upgrade() {
                    f(&v)
                }
            });
        }
        let w = v.me.clone();
        jrefresh.on_click(move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh_jobs()
            }
        });
        let w = v.me.clone();
        frefresh.on_click(move |_| {
            if let Some(v) = w.upgrade() {
                v.refresh_files()
            }
        });
        let w = v.me.clone();
        v.jobs.grid.on_select_cell(move |e| {
            if let (Some(v), Ok(r)) = (w.upgrade(), usize::try_from(e.get_row())) {
                if !v.restoring.get() {
                    *v.jkey.borrow_mut() = row_key(&v.jobs, r, JOB_KEY);
                }
            }
        });
        let w = v.me.clone();
        v.files.grid.on_select_cell(move |e| {
            if let (Some(v), Ok(r)) = (w.upgrade(), usize::try_from(e.get_row())) {
                if !v.restoring.get() {
                    *v.fkey.borrow_mut() = row_key(&v.files, r, FILE_KEY);
                }
            }
        });
        let w = v.me.clone();
        crate::hub::every_tick(move || {
            let Some(v) = w.upgrade() else { return false };
            if v.auto.get_value() && !v.jobs_busy.get() && !v.busy.get() && v.last_done.get().elapsed() >= Duration::from_secs(5) {
                v.refresh();
            }
            true
        });
        v.refresh();
        v
    }

    pub fn refresh(&self) {
        self.refresh_jobs();
        self.refresh_files();
    }

    fn user(&self) -> String {
        self.env.borrow().as_ref().map(|e| e.1.clone()).unwrap_or_else(|| self.conn.borrow().cfg.user.to_uppercase())
    }

    fn with_env(&self, f: impl FnOnce(&DataPumpTab, String, String) + 'static) {
        let cur = self.env.borrow().clone();
        if let Some((d, u)) = cur {
            return f(self, d, u);
        }
        let w = self.me.clone();
        let cfg_user = self.conn.borrow().cfg.user.to_uppercase();
        Conn::meta_query(&self.conn, datapump::ENV_SQL, move |r| {
            let Some(v) = w.upgrade() else { return };
            let (db, user) = match r.ok().and_then(|r| r.rows.into_iter().next()) {
                Some(row) => {
                    let get = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                    let (uniq, con) = (get(1), get(2));
                    (if uniq.is_empty() { String::new() } else { format!("{uniq}/{con}") }, if get(3).is_empty() { cfg_user } else { get(3) })
                }
                None => (String::new(), cfg_user),
            };
            *v.env.borrow_mut() = Some((db.clone(), user.clone()));
            f(&v, db, user);
        });
    }

    // ---- jobs ----

    /// Show rows and put the cursor back on the row with the remembered key; forget the key when it is gone.
    /// A refresh, not the user: it runs quietly, so Ctrl+C keeps copying from where the user selected.
    fn set_keyed(&self, g: &ResultGrid, key: &RefCell<Option<Key>>, kc: (&str, &str), cols: Vec<Column>, rows: Vec<Vec<Cell>>) {
        self.restoring.set(true);
        crate::copy::quietly(|| {
            g.set(cols, rows);
            let k = key.borrow().clone();
            match k.and_then(|k| find_row(g, &k, kc)) {
                Some(i) => g.move_cursor(i as i32, 0, false),
                None => *key.borrow_mut() = None,
            }
        });
        self.restoring.set(false);
    }

    pub fn refresh_jobs(&self) {
        if self.jobs_busy.get() {
            return;
        }
        self.jobs_busy.set(true);
        self.jstatus.set_label("Refreshing…");
        let qs = vec![datapump::jobs_sql(true), datapump::jobs_sql(false), datapump::jobs_sql_basic(true), datapump::jobs_sql_basic(false)];
        self.run_jobs(qs, 0);
    }

    fn run_jobs(&self, qs: Vec<String>, i: usize) {
        let w = self.me.clone();
        let sql = qs[i].clone();
        Conn::meta_query(&self.conn, &sql, move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    let n = r.rows.len();
                    v.jobs_busy.set(false);
                    v.last_done.set(Instant::now());
                    v.set_keyed(&v.jobs, &v.jkey, JOB_KEY, r.cols, r.rows);
                    v.jstatus.set_label(&format!("{n} job(s) · refreshed {}", chrono::Local::now().format("%H:%M:%S")));
                }
                Err(_) if i + 1 < qs.len() => v.run_jobs(qs, i + 1),
                Err(e) => {
                    v.jobs_busy.set(false);
                    v.last_done.set(Instant::now());
                    v.jobs_failed(&e)
                }
            }
        });
    }

    fn jobs_failed(&self, e: &str) {
        self.auto.set_value(false);
        let user = self.user();
        let mut lines: Vec<String> = e.lines().map(str::to_string).collect();
        if e.contains("ORA-00942") || e.contains("ORA-01031") {
            lines.push(format!("{user} may not read the Data Pump job views. Ask a DBA for:"));
            lines.push(format!("  GRANT SELECT ON SYS.DBA_DATAPUMP_JOBS TO {user};   (or the SELECT_CATALOG_ROLE)"));
        } else if let Some(h) = datapump::hint(e, &user, "") {
            lines.push(h);
        }
        lines_grid(&self.jobs, "Data Pump jobs are not available", lines);
        self.jstatus.set_label("Could not list jobs (see below)");
    }

    /// (owner, job, state) of the selected job row, by its remembered key.
    fn job_sel(&self) -> Option<(String, String, String)> {
        let key = self.jkey.borrow().clone();
        let Some(row) = key.as_ref().and_then(|k| find_row(&self.jobs, k, JOB_KEY)) else {
            *self.jkey.borrow_mut() = None;
            ui::error(&self.panel, "Select a row first.");
            return None;
        };
        let (owner, job) = key?;
        Some((owner, job, row_cell(&self.jobs, row, "state").unwrap_or_default()))
    }

    /// Error text with the GRANT hint for `dir` (empty when no directory is involved).
    fn report(&self, what: &str, e: &str, dir: &str) {
        let msg = match datapump::hint(e, &self.user(), dir) {
            Some(h) => format!("{what}\n\n{e}\n\n{h}"),
            None => format!("{what}\n\n{e}"),
        };
        ui::error(&self.panel, &msg);
    }

    fn run_block(&self, what: String, sql: String) {
        let w = self.me.clone();
        Conn::meta_query(&self.conn, &sql, move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(_) => v.refresh_jobs(),
                Err(e) => v.report(&format!("{what} failed."), &e, ""),
            }
        });
    }

    fn stop(&self) {
        let Some((owner, job, _)) = self.job_sel() else { return };
        let sql = datapump::stop_block(&owner, &job, false);
        if ui::confirm_sql(&self.panel, &format!("Stop job {owner}.{job}?\n\nThe master table stays, so Resume can continue it."), &sql) {
            self.run_block(format!("Stop of {job}"), sql);
        }
    }

    fn kill(&self) {
        let Some((owner, job, _)) = self.job_sel() else { return };
        let sql = datapump::stop_block(&owner, &job, true);
        if ui::confirm_sql(&self.panel, &format!("Kill job {owner}.{job}?\n\nThe job is stopped and its master table dropped: it cannot be resumed."), &sql) {
            self.run_block(format!("Kill of {job}"), sql);
        }
    }

    fn resume(&self) {
        let Some((owner, job, _)) = self.job_sel() else { return };
        let Some(p) = ui::ask(&self.panel, "Parallel degree (empty = keep the job's):", "Resume Job", "") else { return };
        let parallel = match p.trim() {
            "" => None,
            s => match s.parse::<u32>() {
                Ok(n) if n >= 1 => Some(n),
                _ => return ui::error(&self.panel, "Parallel degree must be a whole number, 1 or more."),
            },
        };
        let sql = datapump::resume_block(&owner, &job, parallel);
        if ui::confirm_sql(&self.panel, &format!("Resume job {owner}.{job}?"), &sql) {
            self.run_block(format!("Resume of {job}"), sql);
        }
    }

    fn cleanup(&self) {
        let Some((owner, job, state)) = self.job_sel() else { return };
        if !state.trim().eq_ignore_ascii_case("NOT RUNNING") {
            return ui::error(&self.panel, "Clean Up is only for jobs in state NOT RUNNING. Use Kill for a running job.");
        }
        let sql = datapump::cleanup_sql(&owner, &job);
        if ui::confirm_sql(&self.panel, &format!("Drop the master table of {owner}.{job}?"), &sql) {
            self.run_block(format!("Clean up of {job}"), sql);
        }
    }

    fn view_job_log(&self) {
        let Some((_, job, _)) = self.job_sel() else { return };
        let known = datapump::load_files().unwrap_or_default().into_iter().find(|r| r.job == job && r.kind == FileKind::Log && r.conn == self.conn_name);
        let (dir, file) = match known {
            Some(r) => (r.directory, r.file),
            None => {
                let Some(dir) = ui::ask(&self.panel, &format!("Directory of the log of {job}:"), "View Log", "DATA_PUMP_DIR") else { return };
                (dir.trim().to_string(), format!("{job}.log"))
            }
        };
        self.view_file(dir, file);
    }

    // ---- files ----

    pub fn refresh_files(&self) {
        if self.busy.get() {
            return;
        }
        self.busy.set(true);
        self.fstatus.set_label("Refreshing…");
        self.with_env(|v, db, _| {
            let all = match datapump::load_files() {
                Ok(all) => all,
                Err(e) => {
                    v.busy.set(false);
                    v.last_done.set(Instant::now());
                    v.fstatus.set_label(&format!("The file list cannot be read: {e}"));
                    return;
                }
            };
            let todo: VecDeque<FileRecord> = datapump::files_for(&all, &v.conn_name, &db).into_iter().cloned().collect();
            v.probe(Probe { todo, cur: None, out: vec![], err: None });
        });
    }

    /// Check every recorded file on the server, one query at a time; a `%U` dump is followed piece by
    /// piece until the first missing number.
    fn probe(&self, mut p: Probe) {
        if p.cur.is_none() {
            let Some(rec) = p.todo.pop_front() else { return self.probed(p) };
            let c = if rec.kind == FileKind::Dump { datapump::percent_u_candidates(&rec.file, 99) } else { vec![rec.file.clone()] };
            p.cur = Some((rec, c, 0));
        }
        let (rec, cands, i) = p.cur.clone().unwrap();
        let file = cands[i].clone();
        let mk = {
            let (rec, file) = (rec.clone(), file.clone());
            move |exists: Option<bool>, size: Option<u64>| Disp {
                dir: rec.directory.clone(),
                file: file.clone(),
                tmpl: rec.file.clone(),
                job: rec.job.clone(),
                kind: rec.kind,
                created: rec.created.clone(),
                exists,
                size,
            }
        };
        let sql = match datapump::file_exists_sql(&rec.directory, &file) {
            Ok(s) => s,
            Err(e) => {
                p.out.push(mk(None, None));
                p.err.get_or_insert(e);
                p.cur = None;
                return self.probe(p);
            }
        };
        let w = self.me.clone();
        Conn::meta_query(&self.conn, &sql, move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    let row = r.rows.into_iter().next().unwrap_or_default();
                    let ex = row.first().cloned().flatten().is_some_and(|s| s.trim() == "1");
                    let size = row.get(1).cloned().flatten().and_then(|s| s.trim().parse::<f64>().ok()).map(|f| f as u64);
                    if ex {
                        p.out.push(mk(Some(true), size));
                        p.cur = if i + 1 < cands.len() { Some((rec, cands, i + 1)) } else { None };
                    } else {
                        if i == 0 {
                            p.out.push(mk(Some(false), None));
                        }
                        p.cur = None;
                    }
                }
                Err(e) => {
                    p.out.push(mk(None, None));
                    p.err.get_or_insert(e);
                    p.cur = None;
                }
            }
            v.probe(p);
        });
    }

    fn probed(&self, p: Probe) {
        self.busy.set(false);
        self.last_done.set(Instant::now());
        let rows: Vec<Vec<Cell>> = p
            .out
            .iter()
            .map(|d| {
                vec![
                    Some(d.dir.clone()),
                    Some(d.file.clone()),
                    Some(kind_label(d.kind).to_string()),
                    Some(d.job.clone()),
                    Some(d.created.clone()),
                    Some(match d.exists {
                        Some(true) => "yes".to_string(),
                        Some(false) => "missing".to_string(),
                        None => "?".to_string(),
                    }),
                    d.size.map(human),
                ]
            })
            .collect();
        let n = rows.len();
        self.set_keyed(&self.files, &self.fkey, FILE_KEY, ["Directory", "File", "Kind", "Job", "Created", "Exists", "Size"].iter().map(|c| col(c)).collect(), rows);
        *self.disp.borrow_mut() = p.out;
        let when = chrono::Local::now().format("%H:%M:%S");
        match p.err {
            None => self.fstatus.set_label(&format!("{n} file(s) · refreshed {when}")),
            Some(e) => {
                let first = e.lines().next().unwrap_or("").to_string();
                let h = datapump::hint(&e, &self.user(), "the directory").map(|h| format!(" · {h}")).unwrap_or_default();
                self.fstatus.set_label(&format!("{n} file(s) · could not check some: {first}{h}"));
            }
        }
    }

    fn sel_disp(&self) -> Option<Disp> {
        let key = self.fkey.borrow().clone();
        let found = key.as_ref().filter(|k| find_row(&self.files, k, FILE_KEY).is_some()).and_then(|(d, f)| self.disp.borrow().iter().find(|x| &x.dir == d && &x.file == f).cloned());
        if found.is_none() {
            *self.fkey.borrow_mut() = None;
            ui::error(&self.panel, "Select a row first.");
        }
        found
    }

    /// Existing pieces of the selected record (a dump) or just the selected file.
    fn pieces(&self, d: &Disp) -> Vec<String> {
        if d.kind == FileKind::Dump {
            self.disp.borrow().iter().filter(|x| x.dir == d.dir && x.tmpl == d.tmpl && x.exists == Some(true)).map(|x| x.file.clone()).collect()
        } else {
            vec![d.file.clone()]
        }
    }

    fn download(&self) {
        let Some(d) = self.sel_disp() else { return };
        if d.exists == Some(false) {
            return ui::error(&self.panel, &format!("{} does not exist on the server.", d.file));
        }
        let files = self.pieces(&d);
        if files.is_empty() {
            return ui::error(&self.panel, "Nothing to download.");
        }
        if d.job.is_empty() {
            return self.pick_and_download(d.dir, files);
        }
        self.check_not_running(d.job.clone(), false, move |v| v.pick_and_download(d.dir, files));
    }

    /// Run `go` (a download, or a delete when `deleting`) when the job is not running; refuse while
    /// it is, ask when it is stopped or its state cannot be read.
    fn check_not_running(&self, job: String, deleting: bool, go: impl FnOnce(&DataPumpTab) + 'static) {
        let w = self.me.clone();
        let (q1, q2) = (datapump::job_running_any_sql(&job), datapump::job_running_sql_user(&job));
        let decide = move |v: &DataPumpTab, rows: Vec<Vec<Cell>>, job: &str, go: Box<dyn FnOnce(&DataPumpTab)>| match rows.into_iter().next() {
            None => go(v),
            Some(row) => {
                let state = row.first().cloned().flatten().unwrap_or_default();
                if state.trim().eq_ignore_ascii_case("NOT RUNNING") {
                    let q = if deleting {
                        format!("Job {job} is stopped and could be resumed; without its files it cannot. Delete anyway?")
                    } else {
                        format!("Job {job} is stopped, so its files may be incomplete. Download anyway?")
                    };
                    if ui::confirm(&v.panel, &q) {
                        go(v);
                    }
                } else if deleting {
                    ui::error(&v.panel, &format!("The job {job} is still running ({state}) and uses its files. Delete them after it ends."));
                } else {
                    ui::error(&v.panel, &format!("The job {job} is still running ({state}); its files are incomplete. Download after it ends."));
                }
            }
        };
        let go: Box<dyn FnOnce(&DataPumpTab)> = Box::new(go);
        Conn::meta_query(&self.conn, &q1, move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(r) => decide(&v, r.rows, &job, go),
                Err(_) => {
                    let w2 = v.me.clone();
                    Conn::meta_query(&v.conn, &q2, move |r| {
                        let Some(v) = w2.upgrade() else { return };
                        match r {
                            Ok(r) => decide(&v, r.rows, &job, go),
                            Err(e) => {
                                let first = e.lines().next().unwrap_or("").to_string();
                                let tail = if deleting { "If it is, deleting its files breaks it. Delete anyway?" } else { "If it is, the file is incomplete. Download anyway?" };
                                if ui::confirm(&v.panel, &format!("Cannot tell whether job {job} is still running ({first}).\n\n{tail}")) {
                                    go(&v);
                                }
                            }
                        }
                    });
                }
            }
        });
    }

    fn pick_and_download(&self, dir: String, files: Vec<String>) {
        let dd = DirDialog::builder(&self.panel, "Download to folder", "").build();
        if dd.show_modal() != ID_OK {
            return;
        }
        let Some(path) = dd.get_path() else { return };
        transfer::download(&self.conn, dir, files, PathBuf::from(path), |r| {
            let app = crate::app();
            match r {
                Ok(p) => ui::info(&app.frame, &format!("Downloaded:\n{}", p.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"))),
                Err(e) => ui::error(&app.frame, &e),
            }
        });
    }

    fn download_by_name(&self) {
        let Some(dir) = ui::ask(&self.panel, "Server directory:", "Download by Name", "DATA_PUMP_DIR") else { return };
        let Some(names) = ui::ask(&self.panel, "File name(s), separated by comma or space:", "Download by Name", "") else { return };
        let dir = dir.trim().to_string();
        let files: Vec<String> = names.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(str::to_string).collect();
        if files.is_empty() {
            return;
        }
        if !datapump::valid_name(&dir) {
            return ui::error(&self.panel, &format!("Invalid directory name: {dir}"));
        }
        if let Some(bad) = files.iter().find(|f| !datapump::valid_name(f) || f.contains('%')) {
            return ui::error(&self.panel, &format!("Invalid file name: {bad} (give each real file name, no %U)"));
        }
        self.pick_and_download(dir, files);
    }

    fn view(&self) {
        let Some(d) = self.sel_disp() else { return };
        if !matches!(d.kind, FileKind::Log | FileKind::SqlFile) {
            return ui::info(&self.panel, "Only log and SQL files can be viewed. Use Download for a dump.");
        }
        if d.exists == Some(false) {
            return ui::error(&self.panel, &format!("{} does not exist on the server.", d.file));
        }
        self.view_file(d.dir, d.file);
    }

    /// Download a text file to the temp folder and show it.
    fn view_file(&self, dir: String, file: String) {
        view_server_file(&self.conn, self.user(), dir, file, || {});
    }

    fn delete(&self) {
        let Some(d) = self.sel_disp() else { return };
        let pieces = self.pieces(&d);
        if d.exists == Some(false) || pieces.is_empty() {
            // Nothing on the server: only forget the record.
            if ui::confirm(&self.panel, &format!("{} is not on the server. Remove it from the list?", d.file)) {
                self.forget(&d);
                self.refresh_files();
            }
            return;
        }
        if d.job.is_empty() {
            return self.delete_pieces(d, pieces);
        }
        self.check_not_running(d.job.clone(), true, move |v| v.delete_pieces(d, pieces));
    }

    /// Confirm with the statements shown, then remove the files one by one.
    fn delete_pieces(&self, d: Disp, pieces: Vec<String>) {
        let mut stmts = vec![];
        for f in &pieces {
            match datapump::delete_block(&d.dir, f) {
                Ok(b) => stmts.push(b),
                Err(e) => return ui::error(&self.panel, &e),
            }
        }
        if !ui::confirm_sql(&self.panel, &format!("Delete from directory {}? This cannot be undone.", d.dir), &stmts.join("\n")) {
            return;
        }
        let mut q = pieces;
        q.reverse();
        self.delete_next(d, q);
    }

    fn forget(&self, d: &Disp) {
        let db = self.env.borrow().as_ref().map(|e| e.0.clone()).unwrap_or_default();
        let conn = self.conn_name.clone();
        let r = datapump::update_files(|all| all.retain(|r| !(datapump::same_target(r, &conn, &db) && r.directory == d.dir && r.file == d.tmpl && r.job == d.job)));
        if let Err(e) = r {
            ui::error(&self.panel, &format!("The file list was not changed.\n\n{e}"));
        }
    }

    fn delete_next(&self, d: Disp, mut q: Vec<String>) {
        let Some(f) = q.pop() else {
            self.forget(&d);
            return self.refresh_files();
        };
        let blk = match datapump::delete_block(&d.dir, &f) {
            Ok(b) => b,
            Err(e) => return ui::error(&self.panel, &e),
        };
        let w = self.me.clone();
        Conn::meta_query(&self.conn, &blk, move |r| {
            let Some(v) = w.upgrade() else { return };
            match r {
                Ok(_) => v.delete_next(d, q),
                Err(e) => {
                    v.report(&format!("Delete of {f} failed."), &e, &d.dir);
                    v.refresh_files();
                }
            }
        });
    }
}
