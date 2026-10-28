//! Driver Manager (Tools ▸ Drivers…) and the DBeaver-style "download driver files" flow.

use rsqldev_core::download;
use rsqldev_core::registry::{self, Driver};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use wxdragon::prelude::*;

fn status(d: &Driver) -> String {
    if d.is_builtin() {
        return "Built-in".into();
    }
    if let Some(p) = crate::app().prefs().client_dirs.get(&d.id) {
        return format!("Custom folder: {p}");
    }
    match (download::installed_version(d), d.file_for_host()) {
        (Some(v), Some(f)) if v != f.version => format!("Installed {v} (update {} available)", f.version),
        (Some(v), _) => format!("Installed {v}"),
        (None, Some(f)) => format!("Not installed ({} available)", f.version),
        (None, None) => "Not installed (no download for this platform)".into(),
    }
}

/// Ask, download with a progress dialog, then call `done(true)` on success.
pub fn download_with_progress<W: WxWidget + Copy + 'static>(parent: &W, d: &'static Driver, done: impl FnOnce(bool) + 'static) {
    let Some(f) = d.file_for_host() else {
        crate::ui::error(parent, &format!("{} has no download for this platform.\nChoose the client library folder in Tools ▸ Drivers.", d.label));
        return done(false);
    };
    let host = f.url.split('/').nth(2).unwrap_or("");
    let lic = d.license.as_deref().map(|l| format!("\n\nBy downloading you accept the license:\n{l}")).unwrap_or_default();
    let msg = format!("Driver files for {} are missing.\n\nDownload {} client {} from {host}?{lic}", d.label, d.label, f.version);
    if !crate::ui::confirm(parent, &msg) {
        return done(false);
    }
    let prog = Arc::new((AtomicU64::new(0), AtomicU64::new(0), AtomicBool::new(false)));
    let (tx, rx) = mpsc::channel();
    let p2 = prog.clone();
    std::thread::spawn(move || {
        let r = download::install(d, &mut |n, total| {
            p2.0.store(n, Ordering::Relaxed);
            p2.1.store(total.unwrap_or(0), Ordering::Relaxed);
            !p2.2.load(Ordering::Relaxed)
        });
        let _ = tx.send(r.map_err(|e| format!("{e:#}")));
    });
    let dlg = ProgressDialog::builder(parent, "Download driver files", &format!("Downloading {}…", d.label), 1000)
        .with_style(ProgressDialogStyle::CanAbort | ProgressDialogStyle::AppModal | ProgressDialogStyle::AutoHide | ProgressDialogStyle::RemainingTime)
        .build();
    let parent = *parent;
    let mut done = Some(done);
    crate::hub::every_tick(move || {
        if let Ok(r) = rx.try_recv() {
            dlg.destroy();
            let ok = r.is_ok();
            if let Err(e) = r {
                if !e.contains("cancelled") {
                    crate::ui::error(&parent, &format!("Download failed:\n{e}"));
                }
            }
            if let Some(f) = done.take() {
                f(ok);
            }
            return false;
        }
        let (n, t) = (prog.0.load(Ordering::Relaxed), prog.1.load(Ordering::Relaxed));
        let v = if t > 0 { (n * 999 / t) as i32 } else { 0 };
        let msg = format!("Downloading {}… {} / {} MB", d.label, n >> 20, t >> 20);
        if !dlg.update(v, Some(&msg)) {
            prog.2.store(true, Ordering::Relaxed);
        }
        true
    });
}

fn via(k: rsqldev_core::registry::Kind) -> &'static str {
    use rsqldev_core::registry::Kind::*;
    match k {
        Oracle => "Oracle Instant Client",
        Postgres => "PostgreSQL protocol",
        Mysql => "MySQL protocol",
        Mssql => "TDS (SQL Server)",
        Sqlite => "SQLite, embedded",
        Duckdb => "DuckDB, embedded",
        Clickhouse => "ClickHouse HTTP",
    }
}

pub fn show(parent: &Frame) {
    let dlg = Dialog::builder(parent, "Drivers").with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(820, 520).build();
    let list = ListCtrl::builder(&dlg).with_style(ListCtrlStyle::Report | ListCtrlStyle::SingleSel).build();
    // Engine icons, as in the Connections tree.
    list.set_image_list(crate::icons::tree_images(), 1 /* image_list_type::SMALL */);
    for (i, (h, w)) in [("Driver", 190), ("Category", 110), ("Connects via", 170), ("Status", 280)].iter().enumerate() {
        list.insert_column(i as i64, h, ListColumnFormat::Left, *w);
    }
    let fill = move || {
        list.delete_all_items();
        for (i, d) in registry::drivers().iter().enumerate() {
            list.insert_item(i as i64, &d.label, Some(crate::icons::tree_index(&crate::icons::conn_icon(&d.id, false))));
            list.set_item_text_by_column(i as i64, 1, &d.category);
            list.set_item_text_by_column(i as i64, 2, via(d.backend));
            list.set_item_text_by_column(i as i64, 3, &status(d));
            // Built-in drivers need no action: dim them so the ones that do stand out.
            if d.is_builtin() {
                list.set_item_text_colour(i as i64, &crate::theme::c(crate::theme::p().muted));
            }
        }
    };
    fill();
    let bar = BoxSizer::builder(Orientation::Horizontal).build();
    let mk = |l: &str| {
        let b = Button::builder(&dlg).with_label(l).build();
        bar.add(&b, 0, SizerFlag::All, 4);
        b
    };
    let b_dl = mk("Download / Update");
    let b_rm = mk("Remove");
    let b_dir = mk("Choose Client Folder…");
    let b_open = mk("Open Folder");
    bar.add_stretch_spacer(1);
    let b_close = Button::builder(&dlg).with_label("Close").with_id(ID_OK).build();
    bar.add(&b_close, 0, SizerFlag::All, 4);
    let sel = move || {
        let i = list.get_first_selected_item();
        (i >= 0).then(|| &registry::drivers()[i as usize])
    };
    // Actions only make sense for drivers with native client files.
    let sync = move || {
        let needs = sel().is_some_and(|d| !d.is_builtin());
        b_dl.enable(needs && sel().is_some_and(|d| d.file_for_host().is_some()));
        b_rm.enable(needs);
        b_dir.enable(needs);
    };
    sync();
    list.on_item_selected(move |_| sync());
    list.on_item_deselected(move |_| sync());
    b_dl.on_click(move |_| {
        if let Some(d) = sel().filter(|d| !d.is_builtin()) {
            download_with_progress(&dlg, d, move |_| fill());
        }
    });
    b_rm.on_click(move |_| {
        if let Some(d) = sel().filter(|d| !d.is_builtin()) {
            if crate::ui::confirm(&dlg, &format!("Remove downloaded files for {}?", d.label)) {
                let _ = download::remove(d);
                let app = crate::app();
                let mut p = app.prefs();
                if p.client_dirs.remove(&d.id).is_some() {
                    app.set_prefs(p);
                }
                fill();
            }
        }
    });
    b_dir.on_click(move |_| {
        if let Some(d) = sel().filter(|d| !d.is_builtin()) {
            let dd = DirDialog::builder(&dlg, &format!("{} client library folder", d.label), "").build();
            if dd.show_modal() == ID_OK {
                if let Some(path) = dd.get_path() {
                    let app = crate::app();
                    let mut p = app.prefs();
                    p.client_dirs.insert(d.id.clone(), path);
                    app.set_prefs(p);
                    fill();
                }
            }
        }
    });
    b_open.on_click(move |_| {
        let p = sel().map(|d| download::drivers_root().join(&d.id)).unwrap_or_else(download::drivers_root);
        let _ = std::fs::create_dir_all(&p);
        let _ = open::that(&p);
    });
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add(&list, 1, SizerFlag::Expand | SizerFlag::All, 6);
    s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
    dlg.set_sizer(s, true);
    dlg.show_modal();
    dlg.destroy();
}

/// Minimal "open in file manager" / default app.
pub(crate) mod open {
    pub fn that(p: &std::path::Path) -> std::io::Result<()> {
        #[cfg(target_os = "macos")]
        let cmd = "open";
        #[cfg(windows)]
        let cmd = "explorer";
        #[cfg(all(unix, not(target_os = "macos")))]
        let cmd = "xdg-open";
        std::process::Command::new(cmd).arg(p).spawn().map(|_| ())
    }
}
