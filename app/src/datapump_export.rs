//! Tools > Data Pump > Export: a dialog (What / Options / Files / PL/SQL) that builds one
//! DBMS_DATAPUMP block. The PL/SQL tab shows exactly what runs; OK confirms and runs it, Copy to
//! Worksheet opens it for running by hand. The encryption password never reaches the text: the
//! block keeps the `:password` placeholder and the password goes in as a bind.

use crate::conn::{Conn, ConnRef};
use crate::ui;
use rsqldev_core::datapump::{self, Compression, Content, ExportSpec, FileKind, FileRecord, Mode};
use rsqldev_core::worker::Reply;
use rsqldev_core::Bind;
use std::cell::RefCell;
use std::rc::Rc;
use wxdragon::prelude::*;

/// What the server told us about itself (all optional).
#[derive(Clone)]
struct Env {
    /// `DB_UNIQUE_NAME/CON_NAME`, empty when unknown.
    db: String,
    user: String,
    enterprise: bool,
}

const COMPRESSION: [(&str, Compression); 4] = [
    ("Metadata only (default)", Compression::MetadataOnly),
    ("All (needs Advanced Compression)", Compression::All),
    ("Data only (needs Advanced Compression)", Compression::DataOnly),
    ("None", Compression::None),
];

/// Tools > Data Pump > Export…: read the server environment and the directories, then open the dialog.
pub fn show(conn: ConnRef) {
    let c2 = conn.clone();
    let cfg_user = conn.borrow().cfg.user.to_uppercase();
    Conn::meta_query(&conn, datapump::ENV_SQL, move |r| {
        let env = match r.ok().and_then(|r| r.rows.into_iter().next()) {
            Some(row) => {
                let get = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                let (uniq, con) = (get(1), get(2));
                Env {
                    db: if uniq.is_empty() { String::new() } else { format!("{uniq}/{con}") },
                    user: if get(3).is_empty() { cfg_user } else { get(3) },
                    enterprise: datapump::is_enterprise(&get(0)),
                }
            }
            // ENV_SQL failed: identity unknown, parallel stays available.
            None => Env { db: String::new(), user: cfg_user, enterprise: true },
        };
        let c3 = c2.clone();
        Conn::meta_query(&c2, datapump::DIRECTORIES_SQL, move |r| {
            let dirs: Vec<String> = r.map(|r| r.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect()).unwrap_or_default();
            dialog(c3, env, dirs);
        });
    });
}

/// Unquoted names are upper-cased, `"Quoted"` keeps its case.
fn ident(s: &str) -> String {
    let s = s.trim();
    match s.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        Some(inner) => inner.to_string(),
        None => s.to_uppercase(),
    }
}

/// Non-empty trimmed items split on newlines and commas.
fn items(text: &str) -> Vec<String> {
    text.split(['\n', ',']).map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect()
}

/// `SCHEMA.TABLE` or `TABLE` (default schema).
fn split_table(s: &str, default_schema: &str) -> (String, String) {
    match s.split_once('.') {
        Some((a, b)) => (ident(a), ident(b)),
        None => (default_schema.to_string(), ident(s)),
    }
}

fn where_clause(w: &str) -> String {
    let w = w.trim();
    if w.len() >= 5 && w[..5].eq_ignore_ascii_case("where") { w.to_string() } else { format!("WHERE {w}") }
}

fn grid(parent: &Panel) -> Grid {
    let g = Grid::builder(parent).build();
    g.create_grid(1, 2, GridSelectionMode::Cells);
    g.set_row_label_size(32);
    g.set_col_label_value(0, "Table (SCHEMA.TABLE)");
    g.set_col_label_value(1, "WHERE");
    g.set_col_size(0, 260);
    g.set_col_size(1, 420);
    g.enable_editing(true);
    g
}

fn muted(p: &Panel, text: &str) -> StaticText {
    let t = StaticText::builder(p).with_label(text).build();
    t.set_foreground_color(crate::theme::c(crate::theme::p().muted));
    t
}

/// What OK does after the dialog closed.
enum Picked {
    Run(ExportSpec, String, Option<String>),
    Copy(String),
}

fn dialog(conn: ConnRef, env: Env, dirs: Vec<String>) {
    let frame = crate::app().frame;
    let conn_name = conn.borrow().cfg.name.clone();
    let dlg = Dialog::builder(&frame, &format!("Data Pump Export: {conn_name}")).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(780, 560).build();
    let nb = Notebook::builder(&dlg).build();

    // What.
    let wp = Panel::builder(&nb).build();
    let mode = RadioBox::builder(&wp, &["Full", "Schemas", "Tables", "Tablespaces"]).with_label("Export").build();
    mode.set_selection(1);
    let hint = muted(&wp, "Schemas: one per line. Tables: SCHEMA.TABLE per line (one schema). Tablespaces: one per line. Full: list ignored.");
    let list = TextCtrl::builder(&wp).with_style(TextCtrlStyle::MultiLine).with_value(&env.user).build();
    list.set_min_size(Size::new(-1, 90));
    let content = Choice::builder(&wp).with_choices(vec!["All (metadata and data)".into(), "Metadata only".into(), "Data only".into()]).build();
    content.set_selection(0);
    let include = TextCtrl::builder(&wp).build();
    let exclude = TextCtrl::builder(&wp).build();
    let form = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    form.add_growable_col(1, 1);
    ui::row(&wp, &form, "Content:", &content);
    ui::row(&wp, &form, "Include object types:", &include);
    ui::row(&wp, &form, "Exclude object types:", &exclude);
    let inc_note = muted(&wp, "Comma list, e.g. TABLE,INDEX or STATISTICS,GRANT. Leave empty for everything.");
    let fbar = BoxSizer::builder(Orientation::Horizontal).build();
    let add = Button::builder(&wp).with_label("Add Filter").build();
    let rem = Button::builder(&wp).with_label("Remove").build();
    let ftitle = StaticText::builder(&wp).with_label("Table WHERE filters:").build();
    fbar.add(&ftitle, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 8);
    fbar.add(&add, 0, SizerFlag::All, 2);
    fbar.add(&rem, 0, SizerFlag::All, 2);
    let filters = grid(&wp);
    let ws = BoxSizer::builder(Orientation::Vertical).build();
    ws.add(&mode, 0, SizerFlag::Expand | SizerFlag::All, 6);
    ws.add(&hint, 0, SizerFlag::Left | SizerFlag::Right, 8);
    ws.add(&list, 0, SizerFlag::Expand | SizerFlag::All, 6);
    ws.add_sizer(&form, 0, SizerFlag::Expand | SizerFlag::All, 6);
    ws.add(&inc_note, 0, SizerFlag::Left | SizerFlag::Right, 8);
    ws.add_sizer(&fbar, 0, SizerFlag::Expand | SizerFlag::Top, 6);
    ws.add(&filters, 1, SizerFlag::Expand | SizerFlag::All, 6);
    wp.set_sizer(ws, true);
    nb.add_page(&wp, "What", true, None);
    add.on_click(move |_| {
        filters.append_rows(1, false);
        filters.set_grid_cursor(filters.get_number_rows() - 1, 0);
    });
    rem.on_click(move |_| {
        let r = filters.get_grid_cursor_row();
        if r >= 0 && r < filters.get_number_rows() {
            filters.delete_rows(r, 1, false);
        }
    });

    // Options.
    let op = Panel::builder(&nb).build();
    let consistent = CheckBox::builder(&op).with_label("Consistent (FLASHBACK_TIME = SYSTIMESTAMP)").build();
    consistent.set_value(true);
    let estimate = CheckBox::builder(&op).with_label("Estimate only (no dump written)").build();
    let parallel = SpinCtrl::builder(&op).with_range(1, 32).with_initial_value(1).build();
    let par_note = muted(&op, if env.enterprise { "Parallel above 1 writes several files (%U)." } else { "Parallel needs Enterprise Edition: this database is not Enterprise." });
    if !env.enterprise {
        parallel.enable(false);
    }
    let fsize = TextCtrl::builder(&op).build();
    let fsize_note = muted(&op, "Maximum size of one dump file, e.g. 2G or 500M. Empty = no limit.");
    let comp = Choice::builder(&op).with_choices(COMPRESSION.iter().map(|c| c.0.to_string()).collect()).build();
    comp.set_selection(0);
    let encrypt = CheckBox::builder(&op).with_label("Encrypt the dump (needs Advanced Security)").build();
    let pw = TextCtrl::builder(&op).with_style(TextCtrlStyle::Password).build();
    let pw2 = TextCtrl::builder(&op).with_style(TextCtrlStyle::Password).build();
    let og = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    og.add_growable_col(1, 1);
    ui::row(&op, &og, "Parallel:", &parallel);
    ui::row(&op, &og, "File size:", &fsize);
    ui::row(&op, &og, "Compression:", &comp);
    let eg = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    eg.add_growable_col(1, 1);
    ui::row(&op, &eg, "Password:", &pw);
    ui::row(&op, &eg, "Confirm:", &pw2);
    let os = BoxSizer::builder(Orientation::Vertical).build();
    os.add(&consistent, 0, SizerFlag::All, 6);
    os.add(&estimate, 0, SizerFlag::All, 6);
    os.add_sizer(&og, 0, SizerFlag::Expand | SizerFlag::All, 6);
    os.add(&par_note, 0, SizerFlag::Left | SizerFlag::Right, 8);
    os.add(&fsize_note, 0, SizerFlag::Left | SizerFlag::Right, 8);
    os.add(&encrypt, 0, SizerFlag::All, 6);
    os.add_sizer(&eg, 0, SizerFlag::Expand | SizerFlag::All, 6);
    op.set_sizer(os, true);
    nb.add_page(&op, "Options", false, None);

    // Files.
    let fp = Panel::builder(&nb).build();
    let dir = Choice::builder(&fp).build();
    let mut names = dirs.clone();
    if names.is_empty() {
        names.push("DATA_PUMP_DIR".into());
    }
    for d in &names {
        dir.append(d);
    }
    dir.set_selection(names.iter().position(|d| d == "DATA_PUMP_DIR").unwrap_or(0) as u32);
    let job_name = datapump::job_name("RSQ_EXP", chrono::Local::now().naive_local());
    let (d0, l0) = datapump::default_files(&job_name, 1, "");
    let job = TextCtrl::builder(&fp).with_value(&job_name).build();
    let dump = TextCtrl::builder(&fp).with_value(&d0).build();
    let log = TextCtrl::builder(&fp).with_value(&l0).build();
    let fg = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    fg.add_growable_col(1, 1);
    ui::row(&fp, &fg, "Directory:", &dir);
    ui::row(&fp, &fg, "Job name:", &job);
    ui::row(&fp, &fg, "Dump file:", &dump);
    ui::row(&fp, &fg, "Log file:", &log);
    let fnote = muted(&fp, "The dump and log names follow the job name, parallel and file size until you edit them.");
    let fs = BoxSizer::builder(Orientation::Vertical).build();
    fs.add_sizer(&fg, 0, SizerFlag::Expand | SizerFlag::All, 10);
    fs.add(&fnote, 0, SizerFlag::Left | SizerFlag::Right, 12);
    fp.set_sizer(fs, true);
    nb.add_page(&fp, "Files", false, None);

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

    // Dump / log names follow the defaults until the user edits them.
    let last = Rc::new(RefCell::new((d0, l0)));
    let sync = {
        let last = last.clone();
        Rc::new(move || {
            let (d, l) = datapump::default_files(job.get_value().trim(), parallel.value().max(1) as u32, fsize.get_value().trim());
            let mut last = last.borrow_mut();
            if dump.get_value().trim() == last.0 {
                dump.set_value(&d);
            }
            if log.get_value().trim() == last.1 {
                log.set_value(&l);
            }
            *last = (d, l);
        })
    };

    let user = env.user.clone();
    let names2 = names.clone();
    // The spec the controls describe. The password is read separately, never into the spec.
    let build = Rc::new(move || -> Result<ExportSpec, String> {
        filters.save_edit_control_value();
        let mode_sel = mode.get_selection();
        let lines = items(&list.get_value());
        let mode_v = match mode_sel {
            0 => Mode::Full,
            1 => Mode::Schemas(lines.iter().map(|x| ident(x)).collect()),
            2 => {
                let parsed: Vec<(String, String)> = lines.iter().map(|x| split_table(x, &user)).collect();
                let schema = parsed.first().map(|p| p.0.clone()).unwrap_or_default();
                if parsed.iter().any(|p| p.0 != schema) {
                    return Err("All tables must be in one schema.".into());
                }
                Mode::Tables { schema, tables: parsed.into_iter().map(|p| p.1).collect() }
            }
            _ => Mode::Tablespaces(lines.iter().map(|x| ident(x)).collect()),
        };
        let mut table_filters = vec![];
        for r in 0..filters.get_number_rows() {
            let (t, w) = (filters.get_cell_value(r, 0), filters.get_cell_value(r, 1));
            if t.trim().is_empty() && w.trim().is_empty() {
                continue;
            }
            if t.trim().is_empty() || w.trim().is_empty() {
                return Err(format!("Filter row {} needs both a table and a WHERE clause.", r + 1));
            }
            let (schema, table) = split_table(&t, &user);
            table_filters.push((schema, table, where_clause(&w)));
        }
        let encrypt_on = encrypt.get_value();
        if encrypt_on {
            if pw.get_value().is_empty() {
                return Err("Enter the encryption password.".into());
            }
            if pw.get_value() != pw2.get_value() {
                return Err("The encryption passwords differ.".into());
            }
        }
        Ok(ExportSpec {
            job: job.get_value().trim().to_string(),
            mode: mode_v,
            directory: names2.get(dir.get_selection().unwrap_or(0) as usize).cloned().unwrap_or_default(),
            dump_file: dump.get_value().trim().to_string(),
            log_file: log.get_value().trim().to_string(),
            file_size: fsize.get_value().trim().to_uppercase(),
            parallel: parallel.value().max(1) as u32,
            content: [Content::All, Content::MetadataOnly, Content::DataOnly][content.get_selection().unwrap_or(0).min(2) as usize],
            include: items(include.get_value().as_str()).iter().map(|x| x.to_uppercase()).collect(),
            exclude: items(exclude.get_value().as_str()).iter().map(|x| x.to_uppercase()).collect(),
            table_filters,
            consistent: consistent.get_value(),
            estimate_only: estimate.get_value(),
            compression: COMPRESSION[comp.get_selection().unwrap_or(0).min(3) as usize].1,
            encrypt: encrypt_on,
        })
    });
    let block = {
        let build = build.clone();
        Rc::new(move || build().and_then(|s| datapump::export_block(&s)))
    };

    let (sy, bl) = (sync.clone(), block.clone());
    nb.on_page_changed(move |e| match e.get_selection() {
        Some(2) => sy(),
        Some(3) => {
            sy();
            text.set_value(&match bl() {
                Ok(b) => b,
                Err(e) => format!("-- Cannot generate the block yet:\n-- {e}"),
            });
        }
        _ => {}
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
        let (spec, b) = match bd().and_then(|s| datapump::export_block(&s).map(|b| (s, b))) {
            Ok(x) => x,
            Err(e) => return ui::error(&dlg, &e),
        };
        if ui::confirm(&dlg, &format!("Start this export on {cn}?\n\n{b}")) {
            let password = spec.encrypt.then(|| pw.get_value());
            *p2.borrow_mut() = Some(Picked::Run(spec, b, password));
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
        Some(Picked::Run(spec, b, password)) => run(conn, env, spec, b, password),
        None => {}
    }
}

/// Start the job: on the metadata session, or on a short-lived session with the password bound.
fn run(conn: ConnRef, env: Env, spec: ExportSpec, block: String, password: Option<String>) {
    let conn_name = conn.borrow().cfg.name.clone();
    let c2 = conn.clone();
    let spec2 = spec.clone();
    let env2 = env.clone();
    let done = move |r: Result<(), String>| finish(&c2, &env2, &spec2, r);
    match password {
        None => Conn::meta_query(&conn, &block, move |r| done(r.map(|_| ()))),
        Some(pw) => {
            let Some(mut sess) = crate::transfer::open_session(&conn) else { return };
            let mut pending: Option<u64> = None;
            let mut done = Some(done);
            crate::app().status(0, &format!("Starting export job {} on {conn_name}…", spec.job));
            crate::hub::every_tick(move || {
                while let Ok(r) = sess.replies.try_recv() {
                    match r {
                        Reply::Connected => pending = Some(sess.exec_binds(&block, vec![Bind::Text(pw.clone())])),
                        Reply::ConnectFailed { error, .. } => {
                            if let Some(d) = done.take() {
                                d(Err(error));
                            }
                            return false;
                        }
                        Reply::Done { id, result, .. } if pending == Some(id) => {
                            if let Some(d) = done.take() {
                                d(result.map(|_| ()));
                            }
                            return false;
                        }
                        _ => {}
                    }
                }
                true
            });
        }
    }
}

/// Report the outcome; on success remember the files for the Jobs & Files tab.
fn finish(conn: &ConnRef, env: &Env, spec: &ExportSpec, r: Result<(), String>) {
    let app = crate::app();
    let conn_name = conn.borrow().cfg.name.clone();
    match r {
        Err(e) => {
            let e = match datapump::hint(&e, &env.user, &spec.directory) {
                Some(h) => format!("{e}\n\n{h}"),
                None => e,
            };
            ui::error(&app.frame, &format!("Export job {} did not start.\n\n{e}", spec.job));
        }
        Ok(()) => {
            let created = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
            let rec = |file: &str, kind| FileRecord { conn: conn_name.clone(), db: env.db.clone(), directory: spec.directory.clone(), file: file.to_string(), kind, job: spec.job.clone(), created: created.clone() };
            let mut all = datapump::load_files();
            if !spec.estimate_only {
                all.push(rec(&spec.dump_file, FileKind::Dump));
            }
            all.push(rec(&spec.log_file, FileKind::Log));
            let saved = datapump::save_files(&all);
            app.status(0, &format!("Export job {} started on {conn_name}", spec.job));
            let mut msg = format!("Export job {} started on {conn_name}.\n\nDirectory: {}\n", spec.job, spec.directory);
            if !spec.estimate_only {
                msg.push_str(&format!("Dump file: {}\n", spec.dump_file));
            }
            msg.push_str(&format!("Log file: {}", spec.log_file));
            if let Err(e) = saved {
                msg.push_str(&format!("\n\nThe file list could not be saved: {e}"));
            }
            ui::info(&app.frame, &msg);
        }
    }
}
