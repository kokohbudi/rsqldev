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
    me: Weak<DataPumpTab>,
}

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
    }
}

fn cell(g: &ResultGrid, name: &str) -> Option<String> {
    let st = g.st.borrow();
    let r = g.grid.get_grid_cursor_row();
    let i = st.cols.iter().position(|c| c.name.eq_ignore_ascii_case(name))?;
    st.rows.get(usize::try_from(r).ok()?)?.cur.get(i).cloned().flatten()
}

fn lines_grid(g: &ResultGrid, title: &str, lines: Vec<String>) {
    let rows: Vec<Vec<Cell>> = lines.into_iter().filter(|l| !l.trim().is_empty()).map(|l| vec![Some(l)]).collect();
    g.set(vec![col(title)], rows);
}

/// Read-only monospace text in a modal dialog.
fn show_text(title: &str, text: &str) {
    let app = crate::app();
    let dlg = Dialog::builder(&app.frame, title).with_style(DialogStyle::DefaultDialogStyle).with_size(820, 600).build();
    let t = TextCtrl::builder(&dlg).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap).build();
    let prefs = app.prefs();
    if let Some(f) = Font::new_with_details(prefs.editor_font_size, FontFamily::Teletype as i32, 0, 400, false, &prefs.editor_font) {
        t.set_font(&f);
    }
    t.set_value(text);
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

    pub fn refresh_jobs(&self) {
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
                    v.jobs.set(r.cols, r.rows);
                    v.jstatus.set_label(&format!("{n} job(s) · refreshed {}", chrono::Local::now().format("%H:%M:%S")));
                }
                Err(_) if i + 1 < qs.len() => v.run_jobs(qs, i + 1),
                Err(e) => v.jobs_failed(&e),
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
        } else if let Some(h) = datapump::hint(e, &user, "DATA_PUMP_DIR") {
            lines.push(h);
        }
        lines_grid(&self.jobs, "Data Pump jobs are not available", lines);
        self.jstatus.set_label("Could not list jobs (see below)");
    }

    /// (owner, job, state) under the jobs cursor.
    fn job_sel(&self) -> Option<(String, String, String)> {
        let job = cell(&self.jobs, "job_name");
        let Some(job) = job else {
            ui::error(&self.panel, "Select a job row first.");
            return None;
        };
        let owner = cell(&self.jobs, "owner").unwrap_or_else(|| self.user());
        Some((owner, job, cell(&self.jobs, "state").unwrap_or_default()))
    }

    fn report(&self, what: &str, e: &str) {
        let msg = match datapump::hint(e, &self.user(), "DATA_PUMP_DIR") {
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
                Err(e) => v.report(&format!("{what} failed."), &e),
            }
        });
    }

    fn stop(&self) {
        let Some((owner, job, _)) = self.job_sel() else { return };
        if ui::confirm(&self.panel, &format!("Stop job {owner}.{job}?\n\nThe master table stays, so Resume can continue it.")) {
            self.run_block(format!("Stop of {job}"), datapump::stop_block(&owner, &job, false));
        }
    }

    fn kill(&self) {
        let Some((owner, job, _)) = self.job_sel() else { return };
        if ui::confirm(&self.panel, &format!("Kill job {owner}.{job}?\n\nThe job is stopped and its master table dropped: it cannot be resumed.")) {
            self.run_block(format!("Kill of {job}"), datapump::stop_block(&owner, &job, true));
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
        self.run_block(format!("Resume of {job}"), datapump::resume_block(&owner, &job, parallel));
    }

    fn cleanup(&self) {
        let Some((owner, job, state)) = self.job_sel() else { return };
        if !state.trim().eq_ignore_ascii_case("NOT RUNNING") {
            return ui::error(&self.panel, "Clean Up is only for jobs in state NOT RUNNING. Use Kill for a running job.");
        }
        let sql = datapump::cleanup_sql(&owner, &job);
        if ui::confirm(&self.panel, &format!("Drop the master table of {owner}.{job}?\n\n{sql}")) {
            self.run_block(format!("Clean up of {job}"), sql);
        }
    }

    fn view_job_log(&self) {
        let Some((_, job, _)) = self.job_sel() else { return };
        let known = datapump::load_files().into_iter().find(|r| r.job == job && r.kind == FileKind::Log && r.conn == self.conn_name);
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
            let all = datapump::load_files();
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
        self.files.set(["Directory", "File", "Kind", "Job", "Created", "Exists", "Size"].iter().map(|c| col(c)).collect(), rows);
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
        let (dir, file) = (cell(&self.files, "directory"), cell(&self.files, "file"));
        let d = match (dir, file) {
            (Some(d), Some(f)) => self.disp.borrow().iter().find(|x| x.dir == d && x.file == f).cloned(),
            _ => None,
        };
        if d.is_none() {
            ui::error(&self.panel, "Select a file row first.");
        }
        d
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
        self.check_not_running(d.job.clone(), move |v| v.pick_and_download(d.dir, files));
    }

    /// Run `go` when the job is not running; refuse while it is, ask when the state cannot be read.
    fn check_not_running(&self, job: String, go: impl FnOnce(&DataPumpTab) + 'static) {
        let w = self.me.clone();
        let user = self.user();
        let (q1, q2) = (datapump::job_running_sql(&user, &job), datapump::job_running_sql_user(&job));
        let decide = move |v: &DataPumpTab, rows: Vec<Vec<Cell>>, job: &str, go: Box<dyn FnOnce(&DataPumpTab)>| match rows.into_iter().next() {
            None => go(v),
            Some(row) => {
                let state = row.first().cloned().flatten().unwrap_or_default();
                if state.trim().eq_ignore_ascii_case("NOT RUNNING") {
                    if ui::confirm(&v.panel, &format!("Job {job} is stopped, so its files may be incomplete. Download anyway?")) {
                        go(v);
                    }
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
                                if ui::confirm(&v.panel, &format!("Cannot tell whether job {job} is still running ({first}).\n\nIf it is, the file is incomplete. Download anyway?")) {
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
        if !datapump::valid_name(&dir) || !datapump::valid_name(&file) {
            return ui::error(&self.panel, &format!("Invalid name: {dir} / {file}"));
        }
        let tmp = std::env::temp_dir().join(format!("rsqldev-dp-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let path = tmp.join(&file);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(transfer::part_path(&tmp, &file));
        let title = file.clone();
        transfer::download(&self.conn, dir, vec![file], tmp, move |r| {
            let app = crate::app();
            match r {
                Ok(_) => match std::fs::read(&path) {
                    Ok(b) => {
                        let _ = std::fs::remove_file(&path);
                        show_text(&title, &String::from_utf8_lossy(&b));
                    }
                    Err(e) => ui::error(&app.frame, &format!("Cannot read {}: {e}", path.display())),
                },
                Err(e) => ui::error(&app.frame, &e),
            }
        });
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
        if !ui::confirm(&self.panel, &format!("Delete from directory {}:\n\n{}\n\nThis cannot be undone.", d.dir, pieces.join("\n"))) {
            return;
        }
        let mut q = pieces;
        q.reverse();
        self.delete_next(d, q);
    }

    fn forget(&self, d: &Disp) {
        let mut all = datapump::load_files();
        all.retain(|r| !(r.directory == d.dir && r.file == d.tmpl && r.job == d.job && (r.conn == self.conn_name || r.kind == d.kind)));
        let _ = datapump::save_files(&all);
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
                    v.report(&format!("Delete of {f} failed."), &e);
                    v.refresh_files();
                }
            }
        });
    }
}
