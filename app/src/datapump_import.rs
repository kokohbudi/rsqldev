//! Tools > Data Pump Import: a dialog (Source / What / Options / PL/SQL) that builds one
//! DBMS_DATAPUMP import block. A local .dmp can be uploaded first. The PL/SQL tab shows exactly
//! what runs; OK confirms (naming REPLACE / TRUNCATE) and runs it. The password stays a bind.

use crate::conn::{Conn, ConnRef};
use crate::datapump_export::{exec, ident, items, load, muted, split_table, Env};
use crate::ui;
use rsqldev_core::datapump::{self, FileKind, FileRecord, ImportSpec, Mode, TableExists};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};
use wxdragon::prelude::*;

const EXISTS: [(&str, TableExists); 4] = [
    ("Skip (leave the existing table)", TableExists::Skip),
    ("Append rows", TableExists::Append),
    ("Truncate, then load", TableExists::Truncate),
    ("Replace (drop and recreate)", TableExists::Replace),
];

/// How often the job is looked up, and when we give up.
const POLL: Duration = Duration::from_secs(5);
const GIVE_UP: Duration = Duration::from_secs(24 * 3600);

/// Tools > Data Pump Import…
pub fn show(conn: ConnRef) {
    load(conn, dialog);
}

fn pairs_grid(parent: &Panel, a: &str, b: &str) -> Grid {
    let g = Grid::builder(parent).build();
    g.create_grid(1, 2, GridSelectionMode::Cells);
    g.set_row_label_size(32);
    g.set_col_label_value(0, a);
    g.set_col_label_value(1, b);
    g.set_col_size(0, 300);
    g.set_col_size(1, 300);
    g.enable_editing(true);
    g
}

/// Rows with both cells filled; a half-filled row is an error.
fn pairs(g: &Grid, what: &str, fix: fn(&str) -> String) -> Result<Vec<(String, String)>, String> {
    g.save_edit_control_value();
    let mut v = vec![];
    for r in 0..g.get_number_rows() {
        let (a, b) = (g.get_cell_value(r, 0), g.get_cell_value(r, 1));
        if a.trim().is_empty() && b.trim().is_empty() {
            continue;
        }
        if a.trim().is_empty() || b.trim().is_empty() {
            return Err(format!("{what} row {} needs both a source and a target.", r + 1));
        }
        v.push((fix(&a), fix(&b)));
    }
    Ok(v)
}

/// A grid add / remove button bar.
fn bar(p: &Panel, title: &str, g: Grid) -> BoxSizer {
    let s = BoxSizer::builder(Orientation::Horizontal).build();
    let add = Button::builder(p).with_label("Add").build();
    let rem = Button::builder(p).with_label("Remove").build();
    let t = StaticText::builder(p).with_label(title).build();
    s.add(&t, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
    s.add(&add, 0, SizerFlag::All, 2);
    s.add(&rem, 0, SizerFlag::All, 2);
    add.on_click(move |_| {
        g.append_rows(1, false);
        g.set_grid_cursor(g.get_number_rows() - 1, 0);
    });
    rem.on_click(move |_| {
        let r = g.get_grid_cursor_row();
        if r >= 0 && r < g.get_number_rows() {
            g.delete_rows(r, 1, false);
        }
    });
    s
}

/// A tablespace name: upper-cased unless quoted (same rule as schemas).
fn name(s: &str) -> String {
    ident(s)
}

/// What OK does after the dialog closed.
enum Picked {
    /// Spec, block, password, local file to upload first.
    Run(ImportSpec, String, Option<String>, Option<PathBuf>),
    Copy(String),
}

fn dialog(conn: ConnRef, env: Env, dirs: Vec<String>) {
    let frame = crate::app().frame;
    let conn_name = conn.borrow().cfg.name.clone();
    let dlg = Dialog::builder(&frame, &format!("Data Pump Import: {conn_name}")).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(780, 600).build();
    let nb = Notebook::builder(&dlg).build();

    // Source.
    let sp = Panel::builder(&nb).build();
    let src = RadioBox::builder(&sp, &["Upload a local .dmp file first", "Dump file(s) already in the directory"]).with_label("Source").build();
    src.set_selection(0);
    let local = TextCtrl::builder(&sp).build();
    let browse = Button::builder(&sp).with_label("Browse…").build();
    let remote = TextCtrl::builder(&sp).build();
    let dir = Choice::builder(&sp).build();
    let mut names = dirs.clone();
    if names.is_empty() {
        names.push("DATA_PUMP_DIR".into());
    }
    for d in &names {
        dir.append(d);
    }
    dir.set_selection(names.iter().position(|d| d == "DATA_PUMP_DIR").unwrap_or(0) as u32);
    let job_name = datapump::job_name("RSQ_IMP", chrono::Local::now().naive_local());
    let job = TextCtrl::builder(&sp).with_value(&job_name).build();
    let log = TextCtrl::builder(&sp).with_value(&format!("{job_name}.log")).build();
    let lrow = BoxSizer::builder(Orientation::Horizontal).build();
    lrow.add(&local, 1, SizerFlag::Expand | SizerFlag::Right, 6);
    lrow.add(&browse, 0, SizerFlag::empty(), 0);
    let fg = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    fg.add_growable_col(1, 1);
    let ll = StaticText::builder(&sp).with_label("Local file:").build();
    fg.add(&ll, 0, SizerFlag::AlignCenterVertical | SizerFlag::AlignRight, 0);
    fg.add_sizer(&lrow, 1, SizerFlag::Expand, 0);
    ui::row(&sp, &fg, "Dump file(s) on server:", &remote);
    ui::row(&sp, &fg, "Directory:", &dir);
    ui::row(&sp, &fg, "Job name:", &job);
    ui::row(&sp, &fg, "Log file:", &log);
    let snote = muted(&sp, "Upload: the file lands in the directory under its own name (an existing file of that name is overwritten). Server files: names separated by commas or lines; X_%U.dmp is looked up as written.");
    let ss = BoxSizer::builder(Orientation::Vertical).build();
    ss.add(&src, 0, SizerFlag::Expand | SizerFlag::All, 6);
    ss.add_sizer(&fg, 0, SizerFlag::Expand | SizerFlag::All, 10);
    ss.add(&snote, 0, SizerFlag::Left | SizerFlag::Right, 12);
    sp.set_sizer(ss, true);
    nb.add_page(&sp, "Source", true, None);
    browse.on_click(move |_| {
        let fd = FileDialog::builder(&dlg).with_message("Dump file").with_style(FileDialogStyle::Open).with_wildcard("Dump files (*.dmp)|*.dmp|All files (*)|*").build();
        if fd.show_modal() == ID_OK {
            if let Some(p) = fd.get_path() {
                local.set_value(&p);
                src.set_selection(0);
            }
        }
    });

    // What.
    let wp = Panel::builder(&nb).build();
    let mode = RadioBox::builder(&wp, &["Full", "Schemas", "Tables"]).with_label("Import from the dump").build();
    mode.set_selection(0);
    let hint = muted(&wp, "Schemas: one per line. Tables: SCHEMA.TABLE per line (one schema). Full: list ignored.");
    let list = TextCtrl::builder(&wp).with_style(TextCtrlStyle::MultiLine).build();
    list.set_min_size(Size::new(-1, 70));
    let exists = Choice::builder(&wp).with_choices(EXISTS.iter().map(|e| e.0.to_string()).collect()).build();
    exists.set_selection(0);
    let eform = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    eform.add_growable_col(1, 1);
    ui::row(&wp, &eform, "If a table exists:", &exists);
    let rs = pairs_grid(&wp, "Remap schema: from", "to");
    let rt = pairs_grid(&wp, "Remap tablespace: from", "to");
    let rs_bar = bar(&wp, "Remap schema:", rs);
    let rt_bar = bar(&wp, "Remap tablespace:", rt);
    let ws = BoxSizer::builder(Orientation::Vertical).build();
    ws.add(&mode, 0, SizerFlag::Expand | SizerFlag::All, 6);
    ws.add(&hint, 0, SizerFlag::Left | SizerFlag::Right, 8);
    ws.add(&list, 0, SizerFlag::Expand | SizerFlag::All, 6);
    ws.add_sizer(&eform, 0, SizerFlag::Expand | SizerFlag::All, 6);
    ws.add_sizer(&rs_bar, 0, SizerFlag::Expand | SizerFlag::Top, 6);
    ws.add(&rs, 1, SizerFlag::Expand | SizerFlag::All, 6);
    ws.add_sizer(&rt_bar, 0, SizerFlag::Expand | SizerFlag::Top, 6);
    ws.add(&rt, 1, SizerFlag::Expand | SizerFlag::All, 6);
    wp.set_sizer(ws, true);
    nb.add_page(&wp, "What", false, None);

    // Options.
    let op = Panel::builder(&nb).build();
    let parallel = SpinCtrl::builder(&op).with_range(1, 32).with_initial_value(1).build();
    let par_note = muted(&op, if env.enterprise { "Parallel above 1 reads several files (%U) at once." } else { "Parallel needs Enterprise Edition: this database is not Enterprise." });
    if !env.enterprise {
        parallel.enable(false);
    }
    let encrypted = CheckBox::builder(&op).with_label("The dump is encrypted (needs Advanced Security)").build();
    let pw = TextCtrl::builder(&op).with_style(TextCtrlStyle::Password).build();
    let sqlfile = CheckBox::builder(&op).with_label("Only write the DDL to a SQL file (nothing is imported)").build();
    let sqlname = TextCtrl::builder(&op).with_value(&format!("{job_name}.sql")).build();
    let og = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    og.add_growable_col(1, 1);
    ui::row(&op, &og, "Parallel:", &parallel);
    ui::row(&op, &og, "Password:", &pw);
    ui::row(&op, &og, "SQL file name:", &sqlname);
    let onote = muted(&op, "When the job ends, the SQL file is downloaded and opened in a worksheet.");
    let os = BoxSizer::builder(Orientation::Vertical).build();
    os.add(&encrypted, 0, SizerFlag::All, 6);
    os.add(&sqlfile, 0, SizerFlag::All, 6);
    os.add_sizer(&og, 0, SizerFlag::Expand | SizerFlag::All, 6);
    os.add(&par_note, 0, SizerFlag::Left | SizerFlag::Right, 8);
    os.add(&onote, 0, SizerFlag::Left | SizerFlag::Right, 8);
    op.set_sizer(os, true);
    nb.add_page(&op, "Options", false, None);

    // PL/SQL.
    let pp = Panel::builder(&nb).build();
    let text = TextCtrl::builder(&pp).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap).build();
    let prefs = crate::app().prefs();
    if let Some(f) = Font::new_with_details(prefs.editor_font_size, FontFamily::Teletype as i32, 0, 400, false, &prefs.editor_font) {
        text.set_font(&f);
    }
    let ps = BoxSizer::builder(Orientation::Vertical).build();
    ps.add(&text, 1, SizerFlag::Expand, 0);
    pp.set_sizer(ps, true);
    nb.add_page(&pp, "PL/SQL", false, None);

    // Log / SQL file names follow the job name until the user edits them.
    let last = Rc::new(RefCell::new((format!("{job_name}.log"), format!("{job_name}.sql"))));
    let sync = {
        let last = last.clone();
        Rc::new(move || {
            let j = job.get_value().trim().to_string();
            let (l, q) = (format!("{j}.log"), format!("{j}.sql"));
            let mut last = last.borrow_mut();
            if log.get_value().trim() == last.0 {
                log.set_value(&l);
            }
            if sqlname.get_value().trim() == last.1 {
                sqlname.set_value(&q);
            }
            *last = (l, q);
        })
    };

    let user = env.user.clone();
    let names2 = names.clone();
    // The spec the controls describe plus the local file to upload. The password is read separately.
    let build = Rc::new(move || -> Result<(ImportSpec, Option<PathBuf>), String> {
        let lines = items(&list.get_value());
        let mode_v = match mode.get_selection() {
            0 => Mode::Full,
            1 => Mode::Schemas(lines.iter().map(|x| ident(x)).collect()),
            _ => {
                let parsed: Vec<(String, String)> = lines.iter().map(|x| split_table(x, &user)).collect();
                let schema = parsed.first().map(|p| p.0.clone()).unwrap_or_default();
                if parsed.iter().any(|p| p.0 != schema) {
                    return Err("All tables must be in one schema.".into());
                }
                Mode::Tables { schema, tables: parsed.into_iter().map(|p| p.1).collect() }
            }
        };
        let (dump_files, upload) = if src.get_selection() == 0 {
            let path = PathBuf::from(local.get_value().trim());
            let Some(file) = path.file_name().map(|f| f.to_string_lossy().to_string()).filter(|f| !f.is_empty()) else {
                return Err("Choose the local .dmp file to upload.".into());
            };
            if !datapump::valid_name(&file) {
                return Err(format!("The file name {file:?} has characters Oracle directories do not take (use letters, digits and _ . $ # % -). Rename the file."));
            }
            (vec![file], Some(path))
        } else {
            let v = items(&remote.get_value());
            if v.is_empty() {
                return Err("Enter the dump file name(s) in the directory.".into());
            }
            (v, None)
        };
        let encrypted_on = encrypted.get_value();
        if encrypted_on && pw.get_value().is_empty() {
            return Err("Enter the encryption password.".into());
        }
        let sql_file = if sqlfile.get_value() {
            let n = sqlname.get_value().trim().to_string();
            if n.is_empty() {
                return Err("Enter the SQL file name.".into());
            }
            n
        } else {
            String::new()
        };
        Ok((
            ImportSpec {
                job: job.get_value().trim().to_string(),
                mode: mode_v,
                directory: names2.get(dir.get_selection().unwrap_or(0) as usize).cloned().unwrap_or_default(),
                dump_files,
                log_file: log.get_value().trim().to_string(),
                sql_file,
                parallel: parallel.value().max(1) as u32,
                remap_schema: pairs(&rs, "Remap schema", name)?,
                remap_tablespace: pairs(&rt, "Remap tablespace", name)?,
                table_exists: EXISTS[exists.get_selection().unwrap_or(0).min(3) as usize].1,
                encrypted: encrypted_on,
            },
            upload,
        ))
    });
    let block = {
        let build = build.clone();
        Rc::new(move || build().and_then(|(s, _)| datapump::import_block(&s)))
    };

    let (sy, bl) = (sync.clone(), block.clone());
    nb.on_page_changed(move |e| {
        if e.get_selection() == Some(3) {
            sy();
            text.set_value(&match bl() {
                Ok(b) => b,
                Err(e) => format!("-- Cannot generate the block yet:\n-- {e}"),
            });
        }
    });

    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    let copy = Button::builder(&dlg).with_label("Copy to Worksheet").build();
    let cancel = Button::builder(&dlg).with_label("Cancel").with_id(ID_CANCEL).build();
    let ok = Button::builder(&dlg).with_label("OK").with_id(ID_OK).build();
    bs.add(&copy, 0, SizerFlag::All, 4);
    bs.add_stretch_spacer(1);
    bs.add(&cancel, 0, SizerFlag::All, 4);
    bs.add(&ok, 0, SizerFlag::All, 4);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add(&nb, 1, SizerFlag::Expand | SizerFlag::All, 8);
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer(s, true);

    let picked: Rc<RefCell<Option<Picked>>> = Rc::default();
    let (p1, sy, bl) = (picked.clone(), sync.clone(), block.clone());
    copy.on_click(move |_| {
        sy();
        match bl() {
            Ok(b) => {
                *p1.borrow_mut() = Some(Picked::Copy(b));
                dlg.end_modal(ID_OK);
            }
            Err(e) => ui::error(&dlg, &e),
        }
    });
    let (p2, sy, bd) = (picked.clone(), sync, build);
    let cn = conn_name.clone();
    ok.on_click(move |_| {
        sy();
        let (spec, up, b) = match bd().and_then(|(s, u)| datapump::import_block(&s).map(|b| (s, u, b))) {
            Ok(x) => x,
            Err(e) => return ui::error(&dlg, &e),
        };
        if ui::confirm(&dlg, &confirm_text(&spec, up.as_ref(), &cn, &b)) {
            let password = spec.encrypted.then(|| pw.get_value());
            *p2.borrow_mut() = Some(Picked::Run(spec, b, password, up));
            dlg.end_modal(ID_OK);
        }
    });
    dlg.show_modal();
    dlg.destroy();
    match picked.take() {
        Some(Picked::Copy(b)) => {
            let app = crate::app();
            app.open_worksheet(conn);
            if let Some(ws) = app.current_sheet() {
                ws.editor.set_text(&format!("{b}\n"));
            }
        }
        Some(Picked::Run(spec, b, password, up)) => run(conn, env, spec, b, password, up),
        None => {}
    }
}

/// The confirmation: what will happen first (upload, REPLACE / TRUNCATE), then the block.
fn confirm_text(spec: &ImportSpec, upload: Option<&PathBuf>, conn: &str, block: &str) -> String {
    let mut t = String::new();
    if !spec.sql_file.is_empty() {
        t.push_str(&format!("Write the DDL to {} (nothing is imported) on {conn}?\n\n", spec.sql_file));
    } else {
        t.push_str(&format!("Start this import on {conn}?\n\n"));
        match spec.table_exists {
            TableExists::Replace => t.push_str("WARNING: tables that already exist are DROPPED and recreated (TABLE_EXISTS_ACTION = REPLACE).\n\n"),
            TableExists::Truncate => t.push_str("WARNING: tables that already exist are TRUNCATED before the rows load (TABLE_EXISTS_ACTION = TRUNCATE).\n\n"),
            _ => {}
        }
    }
    if let Some(p) = upload {
        t.push_str(&format!("First {} is uploaded to {} (an existing file of that name is overwritten).\n\n", p.display(), spec.directory));
    }
    t.push_str(block);
    t
}

/// Upload (when asked), then start the job.
fn run(conn: ConnRef, env: Env, spec: ImportSpec, block: String, password: Option<String>, upload: Option<PathBuf>) {
    let start = {
        let (conn, env, spec) = (conn.clone(), env.clone(), spec.clone());
        move |uploaded: bool| {
            let c2 = conn.clone();
            exec(&conn, &format!("import job {}", spec.job), block, password, move |r| finish(&c2, &env, &spec, uploaded, r));
        }
    };
    let Some(from) = upload else { return start(false) };
    let file = spec.dump_files[0].clone();
    crate::app().status(0, &format!("Uploading {file}…"));
    let (c2, e2, s2) = (conn.clone(), env.clone(), spec.clone());
    crate::transfer::upload(&conn, spec.directory.clone(), from, file.clone(), move |r| match r {
        Err(e) => {
            let e = match datapump::hint(&e, &e2.user, &s2.directory) {
                Some(h) => format!("{e}\n\n{h}"),
                None => e,
            };
            ui::error(&crate::app().frame, &format!("Upload of {file} failed; the import was not started.\n\n{e}"));
        }
        Ok(()) => {
            // The file is on the server now: remember it even if the job then fails to start.
            record(&c2, &e2, &s2, &[(file, FileKind::Upload)]);
            start(true);
        }
    });
}

fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// Add records to the local file list; the error text when saving failed.
fn record(conn: &ConnRef, env: &Env, spec: &ImportSpec, files: &[(String, FileKind)]) -> Option<String> {
    let conn_name = conn.borrow().cfg.name.clone();
    let mut all = datapump::load_files();
    for (file, kind) in files {
        all.push(FileRecord { conn: conn_name.clone(), db: env.db.clone(), directory: spec.directory.clone(), file: file.clone(), kind: *kind, job: spec.job.clone(), created: now() });
    }
    datapump::save_files(&all).err().map(|e| e.to_string())
}

/// Report the outcome; on success remember the files and, where needed, watch the job end.
fn finish(conn: &ConnRef, env: &Env, spec: &ImportSpec, uploaded: bool, r: Result<(), String>) {
    let app = crate::app();
    let conn_name = conn.borrow().cfg.name.clone();
    if let Err(e) = r {
        let e = match datapump::hint(&e, &env.user, &spec.directory) {
            Some(h) => format!("{e}\n\n{h}"),
            None => e,
        };
        let extra = if uploaded { format!("\n\nThe uploaded dump {} stays in {}.", spec.dump_files[0], spec.directory) } else { String::new() };
        return ui::error(&app.frame, &format!("Import job {} did not start.\n\n{e}{extra}", spec.job));
    }
    let sql = !spec.sql_file.is_empty();
    let mut files = vec![(spec.log_file.clone(), FileKind::Log)];
    if sql {
        files.push((spec.sql_file.clone(), FileKind::SqlFile));
    }
    let saved = record(conn, env, spec, &files);
    app.status(0, &format!("Import job {} started on {conn_name}", spec.job));
    let mut msg = format!("{} job {} started on {conn_name}.\n\nDirectory: {}\nLog file: {}", if sql { "SQL file" } else { "Import" }, spec.job, spec.directory, spec.log_file);
    if sql {
        msg.push_str(&format!("\nSQL file: {}", spec.sql_file));
    }
    if sql || uploaded {
        msg.push_str("\n\nThis window watches the job. When it ends, ");
        msg.push_str(match (sql, uploaded) {
            (true, true) => "the SQL file opens in a worksheet and you are asked whether to delete the uploaded dump.",
            (true, false) => "the SQL file opens in a worksheet.",
            _ => "you are asked whether to delete the uploaded dump.",
        });
        msg.push_str("\nIf the job cannot be watched (no access to DBA_DATAPUMP_JOBS) or this app is closed first, use the Jobs & Files tab later.");
    }
    if let Some(e) = saved {
        msg.push_str(&format!("\n\nThe file list could not be saved: {e}"));
    }
    ui::info(&app.frame, &msg);
    if sql || uploaded {
        watch(conn.clone(), env.clone(), spec.clone(), uploaded);
    }
}

enum Poll {
    Idle,
    Asking,
    Running,
    Gone,
    Failed,
}

/// Look the job up every 5 s (non-blocking) until it is gone, the lookup fails or 24 h pass.
fn watch(conn: ConnRef, env: Env, spec: ImportSpec, uploaded: bool) {
    let state = Rc::new(RefCell::new(Poll::Idle));
    let began = Instant::now();
    let mut last = Instant::now();
    crate::hub::every_tick(move || {
        let cur = std::mem::replace(&mut *state.borrow_mut(), Poll::Asking);
        match cur {
            Poll::Asking => return true,
            Poll::Failed => {
                ui::info(
                    &crate::app().frame,
                    &format!("Cannot watch job {} (the DBA_DATAPUMP_JOBS view is not readable).\n\nWhen it has ended, fetch the SQL file or delete the uploaded dump from the Jobs & Files tab.", spec.job),
                );
                return false;
            }
            Poll::Gone => {
                ended(&conn, &env, &spec, uploaded);
                return false;
            }
            Poll::Running | Poll::Idle => {}
        }
        // Not asking, not decided: wait out the interval, then ask once.
        if began.elapsed() > GIVE_UP {
            return false;
        }
        if last.elapsed() < POLL {
            *state.borrow_mut() = Poll::Running;
            return true;
        }
        last = Instant::now();
        let st = state.clone();
        Conn::meta_query(&conn, &datapump::job_running_sql(&env.user, &spec.job), move |r| {
            *st.borrow_mut() = match r {
                Err(_) => Poll::Failed,
                Ok(r) if r.rows.is_empty() => Poll::Gone,
                Ok(_) => Poll::Running,
            };
        });
        true
    });
}

/// The job is gone: open the SQL file, then offer to delete the uploaded dump.
fn ended(conn: &ConnRef, env: &Env, spec: &ImportSpec, uploaded: bool) {
    let (c2, e2, s2) = (conn.clone(), env.clone(), spec.clone());
    crate::app().status(0, &format!("Import job {} has ended", spec.job));
    if spec.sql_file.is_empty() {
        return offer_delete(&c2, &e2, &s2, uploaded);
    }
    let to = std::env::temp_dir().join("rsqldev-datapump");
    let _ = std::fs::create_dir_all(&to);
    crate::transfer::download(conn, spec.directory.clone(), vec![spec.sql_file.clone()], to, move |r| {
        match r.and_then(|v| {
            let p = v.into_iter().next().ok_or("No file came back.")?;
            std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).to_string()).map_err(|e| format!("Cannot read {}: {e}", p.display()))
        }) {
            Ok(sql) => {
                let app = crate::app();
                app.open_worksheet(c2.clone());
                if let Some(ws) = app.current_sheet() {
                    ws.editor.set_text(&sql);
                }
            }
            Err(e) => {
                let e = match datapump::hint(&e, &e2.user, &s2.directory) {
                    Some(h) => format!("{e}\n\n{h}"),
                    None => e,
                };
                ui::error(&crate::app().frame, &format!("Could not fetch the SQL file {}.\n\n{e}\n\nTry again from the Jobs & Files tab.", s2.sql_file));
            }
        }
        offer_delete(&c2, &e2, &s2, uploaded);
    });
}

fn offer_delete(conn: &ConnRef, env: &Env, spec: &ImportSpec, uploaded: bool) {
    if !uploaded {
        return;
    }
    let file = &spec.dump_files[0];
    let q = format!(
        "Import job {} has ended. Check the log {} if unsure that it went well.\n\nDelete the uploaded dump {file} from {} on the server?",
        spec.job, spec.log_file, spec.directory
    );
    if !ui::confirm(&crate::app().frame, &q) {
        return;
    }
    let block = match datapump::delete_block(&spec.directory, file) {
        Ok(b) => b,
        Err(e) => return ui::error(&crate::app().frame, &e),
    };
    let (conn_name, dir, f2, user) = (conn.borrow().cfg.name.clone(), spec.directory.clone(), file.clone(), env.user.clone());
    Conn::meta_query(conn, &block, move |r| match r {
        Ok(_) => {
            // Forget the Upload record of the deleted file.
            let mut all = datapump::load_files();
            all.retain(|x| !(x.conn == conn_name && x.directory == dir && x.file == f2 && x.kind == FileKind::Upload));
            let _ = datapump::save_files(&all);
            crate::app().status(0, &format!("Deleted {f2} from {dir}"));
        }
        Err(e) => {
            let e = match datapump::hint(&e, &user, &dir) {
                Some(h) => format!("{e}\n\n{h}"),
                None => e,
            };
            ui::error(&crate::app().frame, &format!("Could not delete {f2}.\n\n{e}"));
        }
    });
}
