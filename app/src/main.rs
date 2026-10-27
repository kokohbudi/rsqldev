#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! rsqldev: a native SQL Developer. wxWidgets gives Cocoa widgets on macOS and Win32 on Windows.

mod autosave;
mod compare;
mod complete;
mod copy;
mod data_editor;
mod conn;
mod connections;
mod datapump_export;
mod datapump_tab;
mod datapump_import;
mod driver_manager;
mod editor;
mod find;
mod grid;
mod hub;
mod icons;
mod theme;
mod object_view;
mod prefs;
mod record;
mod sessions;
mod snippets;
mod table_editor;
mod table_menu;
mod topsql;
mod transfer;
mod ui;
mod value;
mod worksheet;

use conn::{Conn, ConnRef};
use connections::Navigator;
use object_view::ObjectView;
use rsqldev_core::config::{self, ConnConfig, Prefs};
use rsqldev_core::dialect;
use rsqldev_core::registry::{self, Kind};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use worksheet::{SheetRef, Worksheet};
use wxdragon::prelude::*;

// wx stock ids (wx/defs.h) so the native Edit menu and macOS app menu behave.
const ID_NEW: i32 = 5002;
const ID_OPEN: i32 = 5000;
const ID_CLOSE_TAB: i32 = 5001;
const ID_SAVEAS: i32 = 5004;
const ID_UNDO: i32 = 5007;
const ID_REDO: i32 = 5008;
const ID_PREFERENCES: i32 = 5022;
const ID_CUT: i32 = 5031;
const ID_COPY: i32 = 5032;
const ID_PASTE: i32 = 5033;
const ID_SELECTALL: i32 = 5037;

const ID_RUN: i32 = ID_HIGHEST + 1;
const ID_SCRIPT: i32 = ID_HIGHEST + 2;
const ID_EXPLAIN: i32 = ID_HIGHEST + 3;
const ID_COMMIT: i32 = ID_HIGHEST + 4;
const ID_ROLLBACK: i32 = ID_HIGHEST + 5;
const ID_CANCEL_STMT: i32 = ID_HIGHEST + 6;
const ID_HISTORY: i32 = ID_HIGHEST + 7;
const ID_CLEAR: i32 = ID_HIGHEST + 8;
const ID_WORKSHEET: i32 = ID_HIGHEST + 9;
pub const ID_UNSHARED: i32 = ID_HIGHEST + 10;
const ID_DRIVERS: i32 = ID_HIGHEST + 11;
const ID_TOGGLE_NAV: i32 = ID_HIGHEST + 12;
const ID_NEW_CONN: i32 = ID_HIGHEST + 13;
const ID_SAVEALL: i32 = ID_HIGHEST + 14;
const ID_BACK: i32 = ID_HIGHEST + 15;
const ID_FORWARD: i32 = ID_HIGHEST + 16;
const ID_RECOVER: i32 = ID_HIGHEST + 17;
const ID_START: i32 = ID_HIGHEST + 18;
const ID_KEYS: i32 = ID_HIGHEST + 19;
const ID_THEME_LIGHT: i32 = ID_HIGHEST + 20;
const ID_THEME_DARK: i32 = ID_HIGHEST + 21;
const ID_THEME_SYSTEM: i32 = ID_HIGHEST + 22;
const ID_COMPLETE: i32 = ID_HIGHEST + 23;
const ID_FIND: i32 = ID_HIGHEST + 24;
const ID_REPLACE: i32 = ID_HIGHEST + 25;
const ID_FIND_NEXT: i32 = ID_HIGHEST + 26;
const ID_FIND_PREV: i32 = ID_HIGHEST + 27;
const ID_FORMAT: i32 = ID_HIGHEST + 28;
const ID_IMPORT_CONNS: i32 = ID_HIGHEST + 29;
const ID_UPDATES: i32 = ID_HIGHEST + 30;
const ID_SESSIONS: i32 = ID_HIGHEST + 31;
const ID_SNIPPET: i32 = ID_HIGHEST + 32;
const ID_SNIPPET_SAVE: i32 = ID_HIGHEST + 33;
const ID_NEW_SHEET: i32 = ID_HIGHEST + 34;
const ID_FIND_OBJECT: i32 = ID_HIGHEST + 35;
const ID_DUP_LINES: i32 = ID_HIGHEST + 36;
const ID_LINES_UP: i32 = ID_HIGHEST + 37;
const ID_LINES_DOWN: i32 = ID_HIGHEST + 38;
const ID_BRACE: i32 = ID_HIGHEST + 39;
const ID_BOOKMARK: i32 = ID_HIGHEST + 40;
const ID_BOOKMARK_NEXT: i32 = ID_HIGHEST + 41;
const ID_BOOKMARK_PREV: i32 = ID_HIGHEST + 42;
const ID_AUTOTRACE: i32 = ID_HIGHEST + 43;
const ID_COMPARE: i32 = ID_HIGHEST + 44;
const ID_RUN_PLSQL: i32 = ID_HIGHEST + 45;
const ID_TOPSQL: i32 = ID_HIGHEST + 46;
const ID_DP_EXPORT: i32 = ID_HIGHEST + 47;
const ID_DP_IMPORT: i32 = ID_HIGHEST + 48;
const ID_DP_JOBS: i32 = ID_HIGHEST + 49;
const ID_INSERT_ROW: i32 = ID_HIGHEST + 50;
const ID_EDIT_VALUE: i32 = ID_HIGHEST + 51;

enum Page {
    Sheet(SheetRef),
    Obj(Rc<ObjectView>),
    Sessions(Rc<sessions::Sessions>),
    TopSql(Rc<topsql::TopSql>),
    DataPump(Rc<datapump_tab::DataPumpTab>),
}

impl Page {
    fn panel(&self) -> Panel {
        match self {
            Page::Sheet(s) => s.panel,
            Page::Obj(o) => o.panel,
            Page::Sessions(s) => s.panel,
            Page::TopSql(s) => s.panel,
            Page::DataPump(s) => s.panel,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct History {
    #[serde(default)]
    entries: Vec<String>,
}

pub struct App {
    frame: Frame,
    nb: AuiNotebook,
    _left: AuiNotebook,
    start: std::cell::Cell<Panel>,
    nav: Navigator,
    split: SplitterWindow,
    nav_shown: std::cell::Cell<bool>,
    prefs: RefCell<Prefs>,
    conns: RefCell<Vec<ConnConfig>>,
    open: RefCell<HashMap<String, ConnRef>>,
    pages: RefCell<Vec<Page>>,
    history: RefCell<Vec<String>>,
    _timer: Timer<Frame>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

pub fn app() -> Rc<App> {
    APP.with(|a| a.borrow().clone().expect("app initialised"))
}

impl App {
    pub fn prefs(&self) -> Prefs {
        self.prefs.borrow().clone()
    }

    pub fn set_prefs(&self, p: Prefs) {
        if let Err(e) = config::save_prefs(&p) {
            ui::error(&self.frame, &format!("Saving preferences failed: {e}"));
        }
        *self.prefs.borrow_mut() = p;
    }

    pub fn add_history(&self, sql: &str) {
        let sql = sql.trim().to_string();
        let mut h = self.history.borrow_mut();
        h.retain(|s| *s != sql);
        h.push(sql);
        let n = h.len();
        if n > 500 {
            h.drain(..n - 500);
        }
        drop(h);
        // Saved at once: a crash or forced quit must not lose the history.
        self.save_history();
    }

    pub fn history(&self) -> Vec<String> {
        self.history.borrow().clone()
    }

    fn save_history(&self) {
        let h = History { entries: self.history() };
        if let Ok(s) = toml::to_string(&h) {
            let _ = std::fs::create_dir_all(config::config_root());
            let _ = std::fs::write(config::config_root().join("history.toml"), s);
        }
    }

    /// File ▸ Import Connections: SQL Developer / DBeaver files from their default locations
    /// (or a picked file), choose which to add. Names already in use get a numeric suffix.
    fn import_connections(&self) {
        use rsqldev_core::migrate;
        let mut files = migrate::default_files();
        if files.is_empty() {
            let d = FileDialog::builder(&self.frame)
                .with_message("SQL Developer connections.json / connections.xml, or DBeaver data-sources.json")
                .with_style(FileDialogStyle::Open)
                .with_wildcard("Connection files (*.json;*.xml)|*.json;*.xml|All files (*)|*")
                .build();
            if d.show_modal() != ID_OK {
                return;
            }
            files.extend(d.get_path().map(std::path::PathBuf::from));
        }
        let mut found = vec![];
        for f in &files {
            match migrate::read(f) {
                Ok(v) => found.extend(v),
                Err(e) => ui::error(&self.frame, &format!("Could not read {}: {e:#}", f.display())),
            }
        }
        if found.is_empty() {
            return ui::info(&self.frame, "No connections found in SQL Developer or DBeaver.");
        }
        let labels: Vec<String> = found
            .iter()
            .map(|f| {
                let c = &f.conn;
                let at = if c.host.is_empty() { c.database.clone() } else { format!("{}:{}/{}", c.host, c.port.map(|p| p.to_string()).unwrap_or_default(), c.database) };
                format!("{}   [{} · {}]   {at}", c.name, f.source, c.driver)
            })
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let d = MultiChoiceDialog::builder(&self.frame, "Connections to import (passwords are asked on first connect):", "Import Connections", &refs).build();
        d.set_selections(&(0..refs.len() as i32).collect::<Vec<_>>());
        if d.show_modal() != ID_OK {
            return;
        }
        let picked = d.get_selections();
        {
            let mut cs = self.conns.borrow_mut();
            for i in &picked {
                let mut c = found[*i as usize].conn.clone();
                let base = c.name.clone();
                let mut n = 2;
                while cs.iter().any(|x| x.name == c.name) {
                    c.name = format!("{base} ({n})");
                    n += 1;
                }
                cs.push(c);
            }
        }
        self.save_conns();
        self.status(0, &format!("Imported {} connection(s)", picked.len()));
    }

    fn save_conns(&self) {
        if let Err(e) = config::save_connections(&self.conns.borrow()) {
            ui::error(&self.frame, &format!("Saving connections failed: {e}"));
        }
        self.nav.reload(&self.conns.borrow());
    }

    pub fn connection_names(&self) -> Vec<String> {
        self.names()
    }

    /// Rename the document tab showing `panel`.
    pub fn retitle(&self, panel: &Panel, title: &str) {
        let i = self.nb.get_page_index(panel);
        if i >= 0 {
            self.nb.set_page_text(i as usize, title);
        }
    }

    fn names(&self) -> Vec<String> {
        self.conns.borrow().iter().map(|c| c.name.clone()).collect()
    }

    /// Store a dialog result; returns the connection name to connect to, if requested.
    fn store(&self, (old, cfg, pw, go): connections::DialogResult) {
        let old = old.as_deref();
        if let Some(o) = old {
            self.disconnect(o);
            if o != cfg.name {
                config::delete_password(o);
            }
        }
        if cfg.save_password && !pw.is_empty() {
            let _ = config::set_password(&cfg.name, &pw);
        } else if !cfg.save_password {
            config::delete_password(&cfg.name);
        }
        let name = cfg.name.clone();
        {
            let mut cs = self.conns.borrow_mut();
            cs.retain(|c| Some(c.name.as_str()) != old && c.name != name);
            cs.push(cfg);
        }
        self.save_conns();
        if go {
            let pw = (!pw.is_empty()).then_some(pw);
            self.connect_with(&name, pw, |c| app().open_worksheet(c));
        }
    }

    pub fn new_connection(&self) {
        let saved = self.conns.borrow().clone();
        if let Some(r) = connections::edit_dialog(&self.frame, None, &saved) {
            self.store(r);
        }
    }

    pub fn edit_connection(&self, name: &str) {
        let saved = self.conns.borrow().clone();
        let cfg = saved.iter().find(|c| c.name == name).cloned();
        if let Some(r) = cfg.and_then(|c| connections::edit_dialog(&self.frame, Some(&c), &saved)) {
            self.store(r);
        }
    }

    pub fn delete_connection(&self, name: &str) {
        self.disconnect(name);
        config::delete_password(name);
        self.conns.borrow_mut().retain(|c| c.name != name);
        self.save_conns();
    }

    pub fn open_conn(&self, name: &str) -> Option<ConnRef> {
        self.open.borrow().get(name).filter(|c| c.borrow().is_up()).cloned()
    }

    pub fn disconnect(&self, name: &str) {
        self.open.borrow_mut().remove(name);
        let mut pages = self.pages.borrow_mut();
        let mut i = pages.len();
        while i > 0 {
            i -= 1;
            let mine = match &pages[i] {
                Page::Sheet(s) => s.conn().borrow().cfg.name == name,
                Page::Obj(o) => o.key.starts_with(&format!("{name}|")),
                Page::Sessions(s) => s.conn_name == name,
                Page::TopSql(s) => s.conn_name == name,
                Page::DataPump(s) => s.conn_name == name,
            };
            if mine {
                let idx = self.nb.get_page_index(&pages[i].panel());
                pages.remove(i);
                if idx >= 0 {
                    self.nb.delete_page(idx as usize);
                }
            }
        }
    }

    pub fn connect(&self, name: &str, f: impl FnOnce(ConnRef) + 'static) {
        self.connect_with(name, None, f)
    }

    fn connect_with(&self, name: &str, pw: Option<String>, f: impl FnOnce(ConnRef) + 'static) {
        let existing = self.open.borrow().get(name).cloned();
        if let Some(c) = existing.filter(|c| !c.borrow().failed()) {
            let c2 = c.clone();
            return Conn::when_ready(&c, move |r| {
                if r.is_ok() {
                    f(c2)
                }
            });
        }
        let Some(cfg) = self.conns.borrow().iter().find(|c| c.name == name).cloned() else { return };
        let file = registry::find(&cfg.driver).is_some_and(|d| d.backend.is_file());
        let pw = match pw.or_else(|| cfg.save_password.then(|| config::get_password(name)).flatten()) {
            Some(p) => p,
            None if file => String::new(),
            None => match ui::ask_password(&self.frame, &format!("Password for {}@{}:", cfg.user, cfg.name)) {
                Some(p) => p,
                None => return,
            },
        };
        self.frame.set_status_text(&format!("Connecting to {name}…"), 0);
        let c = Conn::open(cfg, pw.clone(), self.prefs());
        self.open.borrow_mut().insert(name.to_string(), c.clone());
        let name = name.to_string();
        let c2 = c.clone();
        Conn::when_ready(&c, move |r| {
            let app = app();
            match r {
                Ok(()) => {
                    app.frame.set_status_text(&format!("Connected: {name}"), 0);
                    f(c2)
                }
                Err(fail) => {
                    app.open.borrow_mut().remove(&name);
                    app.frame.set_status_text("Connection failed", 0);
                    match fail.missing_driver.as_deref().and_then(registry::find) {
                        Some(d) => driver_manager::download_with_progress(&app.frame, d, move |ok| {
                            if ok {
                                crate::app().connect_with(&name, Some(pw), f);
                            }
                        }),
                        None => ui::error(&app.frame, &format!("Connection \"{name}\" failed:\n\n{}", fail.error)),
                    }
                }
            }
        });
    }

    fn add_page(&self, page: Page, title: &str) {
        let panel = page.panel();
        self.pages.borrow_mut().push(page);
        self.nb.add_page(&panel, title, true);
    }

    /// Data Pump Jobs & Files tab of a connection: focus it if open, else create it.
    pub fn open_datapump(&self, conn: ConnRef) {
        let name = conn.borrow().cfg.name.clone();
        let found = self.pages.borrow().iter().find_map(|p| match p {
            Page::DataPump(t) if t.conn_name == name => Some(t.clone()),
            _ => None,
        });
        if let Some(t) = found {
            let i = self.nb.get_page_index(&t.panel);
            if i >= 0 {
                self.nb.set_selection(i as usize);
            }
            return t.refresh();
        }
        let t = datapump_tab::DataPumpTab::new(&self.nb, conn);
        self.add_page(Page::DataPump(t), &format!("Data Pump: {name}"));
    }

    pub fn open_worksheet(&self, c: ConnRef) {
        let title = c.borrow().cfg.name.clone();
        let ws = Worksheet::new(&self.nb, c);
        self.add_page(Page::Sheet(ws.clone()), &title);
        ws.editor.set_focus();
    }

    pub fn open_object(&self, conn: &str, owner: &str, kind: &'static str, name: &str) {
        let Some(c) = self.open_conn(conn) else { return };
        let key = format!("{conn}|{owner}|{kind}|{name}");
        let idx = self.pages.borrow().iter().find(|p| matches!(p, Page::Obj(o) if o.key == key)).map(|p| self.nb.get_page_index(&p.panel()));
        if let Some(i) = idx.filter(|i| *i >= 0) {
            self.nb.set_selection(i as usize);
            return;
        }
        let v = ObjectView::new(&self.nb, c, owner, kind, name);
        self.add_page(Page::Obj(v), name);
    }

    /// View ▸ Find DB Object: name search across all schemas, open the pick in the object viewer.
    fn find_object(&self, c: ConnRef) {
        let Some(pat) = ui::ask(&self.frame, "Object name contains (* = any):", "Find DB Object", "").filter(|p| !p.trim().is_empty()) else { return };
        let conn = c.borrow().cfg.name.clone();
        found_objects(&c, &pat.clone(), move |rows| {
            let app = app();
            if rows.is_empty() {
                return ui::info(&app.frame, &format!("No objects match \"{pat}\"."));
            }
            let labels: Vec<String> = rows.iter().map(|(o, n, k)| format!("{n}    {k} · {o}")).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let d = SingleChoiceDialog::builder(&app.frame, &format!("{} object(s) found (max 200):", rows.len()), "Find DB Object", &refs).build();
            if d.show_modal() == ID_OK && d.get_selection() >= 0 {
                let (o, n, k) = &rows[d.get_selection() as usize];
                app.open_object(&conn, o, k, n);
            }
        });
    }

    /// Ctrl/Cmd+Click in the editor: open `[owner.]name` in the object viewer. An exact name
    /// match wins, in the given owner or else the current schema first.
    pub fn goto_object(&self, c: ConnRef, ident: &str) {
        let db = c.borrow().kind;
        let (owner, name) = match ident.rsplit_once('.') {
            Some((o, n)) => (Some(dialect::fold(db, o.trim_matches('"'))), dialect::fold(db, n.trim_matches('"'))),
            None => (None, dialect::fold(db, ident.trim_matches('"'))),
        };
        let (conn, schema) = (c.borrow().cfg.name.clone(), c.borrow().schema.clone());
        let n2 = name.clone();
        found_objects(&c, &name, move |rows| {
            // PostgreSQL functions are listed as name(args).
            let base = |n: &str| n.split('(').next().unwrap_or(n).trim_matches('"').to_string();
            let hits: Vec<_> = rows.into_iter().filter(|(o, n, _)| base(n).eq_ignore_ascii_case(&n2) && owner.as_ref().is_none_or(|w| w.eq_ignore_ascii_case(o))).collect();
            let pick = hits.iter().find(|(o, _, _)| o.eq_ignore_ascii_case(&schema)).or(hits.first());
            match pick {
                Some((o, n, k)) => app().open_object(&conn, o, k, n),
                None => app().status(0, &format!("No object named {n2}")),
            }
        });
    }

    pub fn ddl_to_worksheet(&self, conn: &str, owner: &str, kind: &'static str, name: &str) {
        let Some(c) = self.open_conn(conn) else { return };
        let db = c.borrow().kind;
        let Some((sql, col)) = dialect::tabs(db, kind).into_iter().find_map(|t| match t.view {
            dialect::View::Ddl(s, col) => Some((s, col)),
            dialect::View::Code(s) => Some((s, usize::MAX)),
            _ => None,
        }) else {
            return;
        };
        let sql = dialect::fill(db, &sql, owner, name);
        let c2 = c.clone();
        Conn::meta_query(&c, &sql, move |r| {
            let app = app();
            let text = match r {
                Ok(r) if col == usize::MAX => r.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect(),
                Ok(r) => r.rows.into_iter().next().and_then(|row| row.into_iter().nth(col).flatten()).unwrap_or_default(),
                Err(e) => return ui::error(&app.frame, &e),
            };
            app.open_worksheet(c2);
            if let Some(ws) = app.current_sheet() {
                ws.editor.set_text(text.trim_start());
                ws.editor.set_save_point();
            }
        });
    }

    fn sheets(&self) -> Vec<SheetRef> {
        self.pages.borrow().iter().filter_map(|p| if let Page::Sheet(s) = p { Some(s.clone()) } else { None }).collect()
    }

    /// Mirror worksheet text to autosave slots (called every ~2 s).
    fn autosave_tick(&self) {
        for s in self.sheets() {
            let text = s.editor.get_text();
            if *s.autosaved.borrow() != text {
                let path = s.path.borrow().as_ref().map(|p| p.display().to_string());
                autosave::write(s.autosave_id, &s.conn().borrow().cfg.name, path, &text);
                *s.autosaved.borrow_mut() = text;
            }
        }
    }

    /// Slots not belonging to an open worksheet.
    fn recoverable(&self) -> Vec<autosave::Slot> {
        let open: Vec<String> = self.sheets().iter().map(|s| format!("{}.toml", s.autosave_id)).collect();
        autosave::list().into_iter().filter(|sl| !sl.file.file_name().is_some_and(|f| open.iter().any(|o| *o == f.to_string_lossy()))).collect()
    }

    /// Reopen an autosaved worksheet on its connection.
    pub fn recover(&self, slot: autosave::Slot) {
        if !self.names().contains(&slot.conn) {
            return ui::error(&self.frame, &format!("Connection \"{}\" no longer exists. The text is kept in:\n{}", slot.conn, slot.file.display()));
        }
        let conn = slot.conn.clone();
        self.connect(&conn, move |c| {
            let app = app();
            app.open_worksheet(c);
            if let Some(ws) = app.current_sheet() {
                ws.editor.set_text(&slot.text);
                *ws.path.borrow_mut() = slot.path.clone().map(Into::into);
                autosave::remove_file(&slot.file);
            }
        });
    }

    fn show_start_page(&self) {
        let cur = self.start.get();
        if cur.is_valid() && self.nb.get_page_index(&cur) >= 0 {
            self.nb.set_selection(self.nb.get_page_index(&cur) as usize);
            return;
        }
        let p = start_page(&self.nb, &self.recoverable());
        self.nb.insert_page(0, &p, "Start Page", true);
        self.start.set(p);
    }

    /// Index into `pages` of the selected document tab.
    fn current(&self) -> Option<usize> {
        let sel = self.nb.get_selection();
        if sel < 0 {
            return None;
        }
        self.pages.borrow().iter().position(|p| self.nb.get_page_index(&p.panel()) == sel)
    }

    /// Drop pages whose tab was closed with the × button (AuiNotebook destroys them itself),
    /// and keep the title / status bar in sync with the selected tab.
    fn sync_tabs(&self) {
        self.pages.borrow_mut().retain(|p| p.panel().is_valid() && self.nb.get_page_index(&p.panel()) >= 0);
        let title = match self.current().map(|i| self.pages.borrow()[i].panel()) {
            Some(p) => self.nb.get_page_text(self.nb.get_page_index(&p) as usize),
            None => String::new(),
        };
        let want = if title.is_empty() { "rsqldev".to_string() } else { format!("rsqldev : {title}") };
        if self.frame.get_title() != want {
            self.frame.set_title(&want);
            if self.current_sheet().is_none() {
                for f in 1..4 {
                    self.frame.set_status_text("", f);
                }
            }
        }
    }

    /// Appearance can only be set before windows exist: restart to apply.
    /// Open worksheets are autosaved and offered on the Start Page afterwards.
    pub fn offer_restart(&self) {
        if !ui::confirm(&self.frame, "The new appearance applies after a restart.\n\nRestart rsqldev now? Open worksheets are kept and offered on the Start Page.") {
            return;
        }
        self.autosave_tick();
        self.save_history();
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(std::env::args().skip(1)).spawn()) {
            Ok(_) => self.frame.close(true),
            Err(e) => ui::error(&self.frame, &format!("Could not restart: {e}\nPlease restart rsqldev manually.")),
        }
    }

    pub fn status(&self, field: i32, text: &str) {
        self.frame.set_status_text(text, field);
    }

    /// The selected tab's object viewer, when its Data tab is showing.
    fn current_data(&self) -> Option<Rc<ObjectView>> {
        let i = self.current()?;
        match &self.pages.borrow()[i] {
            Page::Obj(o) if o.data_active() => Some(o.clone()),
            _ => None,
        }
    }

    fn current_sheet(&self) -> Option<SheetRef> {
        let i = self.current()?;
        match &self.pages.borrow()[i] {
            Page::Sheet(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn close_current(&self) {
        let Some(i) = self.current() else { return };
        if let Some(Page::Sheet(s)) = self.pages.borrow().get(i) {
            if s.editor.is_modified() && !s.editor.get_text().trim().is_empty() && !ui::confirm(&self.frame, "Close worksheet without saving?") {
                return;
            }
            if s.has_unsaved_edits() && !ui::confirm(&self.frame, "The Query Result has changes that are not committed. Close and discard them?") {
                return;
            }
        }
        if let Some(Page::Sheet(s)) = self.pages.borrow().get(i) {
            autosave::remove(s.autosave_id);
        }
        let idx = self.nb.get_page_index(&self.pages.borrow()[i].panel());
        self.pages.borrow_mut().remove(i);
        if idx >= 0 {
            self.nb.delete_page(idx as usize);
        }
    }

    /// Connection for a new worksheet: the current tab's, else the tree selection's.
    fn pick_conn(&self, f: impl FnOnce(ConnRef) + 'static) {
        if let Some(s) = self.current_sheet() {
            return f(s.conn());
        }
        let names = self.names();
        if names.is_empty() {
            return self.new_connection();
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let d = SingleChoiceDialog::builder(&self.frame, "Connection:", "Select Connection", &refs).build();
        if d.show_modal() == ID_OK && d.get_selection() >= 0 {
            self.connect(&names[d.get_selection() as usize], f);
        }
    }

    pub fn menu(&self, id: i32) -> bool {
        let sheet = self.current_sheet();
        let editor_focus = sheet.as_ref().is_some_and(|s| s.editor.has_focus());
        // Data editor keys (SQL Developer): F11 / F12 commit / rollback, Ctrl+I insert row,
        // Ctrl+D delete rows, Ctrl+R refresh, Ctrl+V paste cells.
        if let Some(o) = self.current_data() {
            match id {
                ID_COMMIT => o.commit(),
                ID_ROLLBACK => o.rollback(),
                ID_INSERT_ROW => o.insert_row(),
                ID_EDIT_VALUE => o.edit_value(),
                ID_CLEAR => o.delete_rows(),
                ID_REPLACE => o.refresh_data(),
                ID_PASTE if o.paste() => {}
                _ => return self.menu_rest(id, sheet, editor_focus),
            }
            return true;
        }
        self.menu_rest(id, sheet, editor_focus)
    }

    fn menu_rest(&self, id: i32, sheet: Option<SheetRef>, editor_focus: bool) -> bool {
        match id {
            ID_NEW_CONN | ID_NEW => self.new_connection(),
            ID_IMPORT_CONNS => self.import_connections(),
            ID_SESSIONS => self.pick_conn(|c| {
                let a = app();
                let title = format!("Sessions: {}", c.borrow().cfg.name);
                let s = sessions::Sessions::new(&a.nb, c);
                a.add_page(Page::Sessions(s), &title);
            }),
            ID_DP_EXPORT => self.pick_conn(|c| {
                if c.borrow().kind == rsqldev_core::registry::Kind::Oracle {
                    datapump_export::show(c);
                } else {
                    ui::info(&app().frame, "Data Pump is available on Oracle connections");
                }
            }),
            ID_DP_IMPORT => self.pick_conn(|c| {
                if c.borrow().kind == rsqldev_core::registry::Kind::Oracle {
                    datapump_import::show(c);
                } else {
                    ui::info(&app().frame, "Data Pump is available on Oracle connections");
                }
            }),
            ID_DP_JOBS => self.pick_conn(|c| {
                if c.borrow().kind == rsqldev_core::registry::Kind::Oracle {
                    app().open_datapump(c);
                } else {
                    ui::info(&app().frame, "Data Pump is available on Oracle connections");
                }
            }),
            ID_TOPSQL => self.pick_conn(|c| {
                let a = app();
                let title = format!("Top SQL: {}", c.borrow().cfg.name);
                let s = topsql::TopSql::new(&a.nb, c);
                a.add_page(Page::TopSql(s), &title);
            }),
            ID_UPDATES => check_updates(true),
            ID_OPEN => {
                let d = FileDialog::builder(&self.frame).with_message("Open SQL file").with_style(FileDialogStyle::Open).with_wildcard("SQL files (*.sql)|*.sql|All files (*)|*").build();
                if d.show_modal() == ID_OK {
                    if let Some(p) = d.get_path() {
                        self.pick_conn(move |c| {
                            let app = app();
                            app.open_worksheet(c);
                            if let Some(ws) = app.current_sheet() {
                                if let Err(e) = ws.open_file(p.into()) {
                                    ui::error(&app.frame, &e.to_string());
                                }
                            }
                        });
                    }
                }
            }
            ID_SAVE | ID_SAVEAS => {
                if let Some(s) = sheet {
                    s.save(id == ID_SAVEAS)
                }
            }
            ID_CLOSE_TAB => self.close_current(),
            ID_RECOVER => {
                let slots = self.recoverable();
                if slots.is_empty() {
                    return {
                        ui::info(&self.frame, "No unsaved worksheets to recover.");
                        true
                    };
                }
                let labels: Vec<String> = slots.iter().map(autosave::label).collect();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                let d = SingleChoiceDialog::builder(&self.frame, "Worksheets closed without saving:", "Recover Worksheet", &refs).build();
                if d.show_modal() == ID_OK && d.get_selection() >= 0 {
                    self.recover(slots[d.get_selection() as usize].clone());
                }
            }
            ID_START => self.show_start_page(),
            ID_THEME_LIGHT | ID_THEME_DARK | ID_THEME_SYSTEM => {
                let t = match id {
                    ID_THEME_DARK => "dark",
                    ID_THEME_SYSTEM => "system",
                    _ => "light",
                };
                let mut p = self.prefs();
                if p.theme != t {
                    p.theme = t.into();
                    self.set_prefs(p);
                    self.offer_restart();
                }
            }
            ID_KEYS => ui::info(&self.frame, SHORTCUTS),
            ID_SAVEALL => {
                let sheets: Vec<SheetRef> = self.pages.borrow().iter().filter_map(|p| if let Page::Sheet(s) = p { Some(s.clone()) } else { None }).collect();
                for s in sheets.into_iter().filter(|s| s.editor.is_modified()) {
                    s.save(false);
                }
            }
            ID_BACK | ID_FORWARD => {
                let (n, i) = (self.nb.page_count() as i32, self.nb.get_selection());
                if n > 1 && i >= 0 {
                    self.nb.set_selection(((i + if id == ID_BACK { n - 1 } else { 1 }) % n) as usize);
                }
            }
            ID_EXIT => self.frame.close(false),
            // Outside the worksheet editor, Copy takes the last selected grid, tree node or text.
            ID_COPY if !editor_focus => {
                copy::copy();
            }
            ID_UNDO | ID_REDO | ID_CUT | ID_COPY | ID_PASTE | ID_SELECTALL if editor_focus => {
                let e = sheet.unwrap().editor;
                match id {
                    ID_UNDO => e.undo(),
                    ID_REDO => e.redo(),
                    ID_CUT => e.cut(),
                    ID_COPY => e.copy(),
                    ID_PASTE => e.paste(),
                    _ => e.select_all(),
                }
            }
            ID_FIND | ID_REPLACE | ID_FIND_NEXT | ID_FIND_PREV => {
                if let Some(s) = sheet {
                    match id {
                        // Focus outside the editor: search the result grid instead.
                        ID_FIND if !editor_focus => s.results.find(true),
                        ID_FIND_NEXT if !editor_focus => s.results.find(false),
                        ID_FIND | ID_REPLACE => s.find.open(id == ID_REPLACE),
                        _ => s.find.next(id == ID_FIND_PREV),
                    }
                }
            }
            ID_EDIT_VALUE => {
                if let Some(s) = sheet {
                    s.results.edit_value();
                }
            }
            ID_FORMAT => {
                if let Some(s) = sheet {
                    s.format_sql();
                }
            }
            ID_COMPLETE => {
                if let Some(s) = sheet {
                    complete::show(s.editor, &s.conn(), true);
                }
            }
            ID_DUP_LINES | ID_LINES_UP | ID_LINES_DOWN | ID_BRACE | ID_BOOKMARK | ID_BOOKMARK_NEXT | ID_BOOKMARK_PREV => {
                let Some(e) = sheet.map(|s| s.editor) else { return true };
                match id {
                    ID_DUP_LINES => editor::duplicate_lines(e),
                    ID_LINES_UP | ID_LINES_DOWN => editor::move_lines(e, id == ID_LINES_UP),
                    ID_BRACE => editor::goto_brace(e),
                    ID_BOOKMARK => editor::toggle_bookmark(e),
                    _ => editor::goto_bookmark(e, id == ID_BOOKMARK_NEXT),
                }
            }
            ID_SNIPPET | ID_SNIPPET_SAVE => {
                if let Some(s) = sheet {
                    if id == ID_SNIPPET { snippets::insert(&self.frame, s.editor) } else { snippets::save_selection(&self.frame, s.editor) }
                }
            }
            ID_RUN | ID_SCRIPT | ID_EXPLAIN | ID_AUTOTRACE | ID_RUN_PLSQL | ID_COMMIT | ID_ROLLBACK | ID_CANCEL_STMT | ID_HISTORY | ID_CLEAR => {
                let Some(s) = sheet else { return true };
                match id {
                    ID_RUN => s.run_statement(),
                    ID_SCRIPT => s.run_script(),
                    ID_EXPLAIN => s.explain_plan(),
                    ID_AUTOTRACE => s.autotrace(),
                    ID_RUN_PLSQL => s.run_plsql(),
                    ID_COMMIT => s.commit(),
                    ID_ROLLBACK => s.rollback(),
                    ID_CANCEL_STMT => s.cancel(),
                    ID_HISTORY => s.history(),
                    _ => s.clear(),
                }
            }
            ID_WORKSHEET | ID_NEW_SHEET => self.pick_conn(|c| app().open_worksheet(c)),
            ID_FIND_OBJECT => self.pick_conn(|c| app().find_object(c)),
            ID_UNSHARED => {
                if let Some(s) = sheet {
                    let (cfg, prefs) = (s.conn().borrow().cfg.clone(), self.prefs());
                    let pw = config::get_password(&cfg.name).or_else(|| ui::ask_password(&self.frame, "Password:"));
                    if let Some(pw) = pw {
                        let c = Conn::open(cfg, pw, prefs);
                        let c2 = c.clone();
                        Conn::when_ready(&c, move |r| match r {
                            Ok(()) => app().open_worksheet(c2),
                            Err(f) => ui::error(&app().frame, &f.error),
                        });
                    }
                }
            }
            ID_TOGGLE_NAV => {
                if self.nav_shown.replace(!self.nav_shown.get()) {
                    self.split.unsplit(Some(&self.nav.panel));
                } else {
                    self.split.split_vertically(&self.nav.panel, &self.nb, 280);
                }
            }
            ID_DRIVERS => driver_manager::show(&self.frame),
            ID_COMPARE => compare::show(&self.frame),
            ID_PREFERENCES => prefs::show(&self.frame),
            ID_ABOUT => ui::info(
                &self.frame,
                &format!("rsqldev {}\nNative SQL Developer in Rust + wxWidgets.\n\nDrivers: {} built in or downloadable.", env!("CARGO_PKG_VERSION"), registry::drivers().len()),
            ),
            _ => return false,
        }
        true
    }
}

/// Run [`dialect::find_sql`] on the metadata session: (owner, name, kind) rows, or an error
/// dialog.
// ponytail: "contains" search capped at 200 rows; an exact-name lookup can fall outside it in
// very large catalogs. Add an exact query per engine if that bites.
fn found_objects(c: &ConnRef, pattern: &str, cb: impl FnOnce(Vec<(String, String, &'static str)>) + 'static) {
    let db = c.borrow().kind;
    Conn::meta_query(c, &dialect::find_sql(db, pattern), move |r| match r {
        Ok(r) => cb(r
            .rows
            .into_iter()
            .filter_map(|row| {
                let mut it = row.into_iter().map(Option::unwrap_or_default);
                let (owner, name, kind) = (it.next()?, it.next()?, it.next()?);
                Some((owner, name, dialect::kind_const(&kind)?))
            })
            .collect()),
        Err(e) => ui::error(&app().frame, &e),
    });
}

/// Status bar activity: an animated bar and elapsed time whenever a connection is connecting,
/// executing or fetching (see `worker::busy`), so long work never looks like a hang.
/// Look up the latest release in the background. On start (`manual` false) a newer version
/// only shows in the status bar; Help ▸ Check for Updates reports either way.
fn check_updates(manual: bool) {
    use rsqldev_core::download;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(download::latest_version().map_err(|e| format!("{e:#}")));
    });
    hub::every_tick(move || {
        let Ok(r) = rx.try_recv() else { return true };
        let cur = env!("CARGO_PKG_VERSION");
        match r {
            Ok(v) if download::is_newer(&v, cur) => {
                app().status(0, &format!("rsqldev {v} is available (you have {cur}): Help ▸ Check for Updates"));
                if manual && ui::confirm(&app().frame, &format!("rsqldev {v} is available (you have {cur}).\n\nOpen the download page?")) {
                    ui::open_url(download::DOWNLOAD_PAGE);
                }
            }
            Ok(_) if manual => ui::info(&app().frame, &format!("rsqldev {cur} is the latest version.")),
            Err(e) if manual => ui::error(&app().frame, &format!("Could not check for updates: {e}")),
            _ => {}
        }
        false
    });
}

fn activity_bar(sb: StatusBar) {
    const F_TIME: usize = 4;
    const F_BAR: usize = 5;
    let gauge = Gauge::builder(&sb).with_range(100).build();
    gauge.show(false);
    let mut since: Option<std::time::Instant> = None;
    let mut phase = 0;
    hub::every_tick(move || {
        if !gauge.is_valid() {
            return false;
        }
        let busy = rsqldev_core::worker::busy() > 0 || conn::meta_pending() > 0;
        match (busy, since) {
            (true, None) => since = Some(std::time::Instant::now()),
            (false, Some(_)) => {
                since = None;
                gauge.show(false);
                sb.set_status_text("", F_TIME);
            }
            _ => {}
        }
        // Shown at once: a click that waits on the database must never look like a hang.
        let Some(t) = since else { return true };
        let r = sb.get_field_rect(F_BAR);
        gauge.set_size_with_pos(r.x + 4, r.y + 2, r.width - 8, r.height - 4);
        gauge.show(true);
        // No indeterminate mode in wxDragon: sweep the bar instead (~1.5 s per pass at 25 ms ticks).
        phase = (phase + 2) % 102;
        gauge.set_value(phase.min(100));
        sb.set_status_text(&format!("Working {:.1} s", t.elapsed().as_secs_f32()), F_TIME);
        true
    });
}

fn menus(frame: &Frame) {
    let file = Menu::builder()
        .append_item(ID_NEW_CONN, "New Connection…\tCtrl+Shift+C", "Create a database connection")
        .append_item(ID_IMPORT_CONNS, "&Import Connections…", "Import connections from Oracle SQL Developer or DBeaver")
        .append_item(ID_NEW_SHEET, "New SQL &Worksheet\tCtrl+N", "Open a SQL worksheet on a connection")
        .append_item(ID_OPEN, "&Open…\tCtrl+O", "Open a SQL file in a worksheet")
        .append_item(ID_SAVE, "&Save\tCtrl+S", "")
        .append_item(ID_SAVEAS, "Save &As…", "")
        .append_item(ID_SAVEALL, "Save A&ll\tCtrl+Shift+S", "")
        .append_item(ID_CLOSE_TAB, "&Close\tCtrl+W", "Close the current tab")
        .append_item(ID_RECOVER, "&Recover Unsaved Worksheet…\tCtrl+Shift+T", "Reopen a worksheet closed without saving")
        .append_separator()
        .append_item(ID_EXIT, "E&xit\tCtrl+Q", "")
        .build();
    let edit = Menu::builder()
        .append_item(ID_UNDO, "&Undo\tCtrl+Z", "")
        .append_item(ID_REDO, "&Redo\tCtrl+Shift+Z", "")
        .append_separator()
        .append_item(ID_CUT, "Cu&t\tCtrl+X", "")
        .append_item(ID_COPY, "&Copy\tCtrl+C", "")
        .append_item(ID_PASTE, "&Paste\tCtrl+V", "")
        .append_item(ID_INSERT_ROW, "Insert &Row\tCtrl+I", "Data editor: new row after the cursor row")
        .append_item(ID_EDIT_VALUE, "Edit &Value…\tAlt+Enter", "Result grid: the cursor cell in an editor (long text, date picker, LOB)")
        .append_item(ID_SELECTALL, "Select &All\tCtrl+A", "")
        .append_separator()
        .append_item(ID_COMPLETE, "Completion &Insight\tRawCtrl+Space", "Complete keywords, tables and columns")
        .append_item(ID_FORMAT, "F&ormat SQL\tCtrl+F7", "Format the selection, or the whole worksheet")
        .append_item(ID_SNIPPET, "Insert S&nippet…\tCtrl+Shift+J", "Insert a SQL template at the caret")
        .append_item(ID_SNIPPET_SAVE, "Save Selection as Snippet…", "Keep the selected SQL as one of your snippets")
        .append_separator()
        .append_item(ID_DUP_LINES, "&Duplicate Lines\tCtrl+Shift+D", "")
        .append_item(ID_LINES_UP, "Move Lines &Up\tAlt+Up", "")
        .append_item(ID_LINES_DOWN, "Move Lines Do&wn\tAlt+Down", "")
        .append_item(ID_BRACE, "Go to &Matching Bracket\tCtrl+]", "")
        .append_separator()
        .append_item(ID_BOOKMARK, "Toggle &Bookmark\tCtrl+K", "")
        .append_item(ID_BOOKMARK_NEXT, "Ne&xt Bookmark\tF2", "")
        .append_item(ID_BOOKMARK_PREV, "Previous Bookmar&k\tShift+F2", "")
        .append_separator()
        .append_item(ID_FIND, "&Find…\tCtrl+F", "")
        .append_item(ID_REPLACE, "R&eplace…\tCtrl+R", "")
        .append_item(ID_FIND_NEXT, "Find &Next\tF3", "")
        .append_item(ID_FIND_PREV, "Find Pre&vious\tShift+F3", "")
        .build();
    let view = Menu::builder()
        .append_item(ID_TOGGLE_NAV, "&Connections\tCtrl+Shift+L", "Show or hide the Connections navigator")
        .append_item(ID_FIND_OBJECT, "Find DB &Object…\tCtrl+Shift+O", "Find tables, views, packages… by name in every schema")
        .append_separator()
        .append_radio_item(ID_THEME_LIGHT, "&Light Appearance", "SQL Developer's default look")
        .append_radio_item(ID_THEME_DARK, "&Dark Appearance", "")
        .append_radio_item(ID_THEME_SYSTEM, "&Follow System Appearance", "")
        .build();
    let run = Menu::builder()
        .append_item(ID_RUN, "Run &Statement\tCtrl+Enter", "")
        .append_item(ID_SCRIPT, "Run S&cript\tF5", "")
        .append_item(ID_EXPLAIN, "&Explain Plan\tF10", "")
        .append_item(ID_AUTOTRACE, "&Autotrace\tF6", "Run the query and show its plan with actual rows and time")
        .append_item(ID_RUN_PLSQL, "Run &PL/SQL…\tCtrl+F10", "Open an anonymous block calling the procedure or function under the caret")
        .append_separator()
        .append_item(ID_COMMIT, "C&ommit\tF11", "")
        .append_item(ID_ROLLBACK, "&Rollback\tF12", "")
        .append_item(ID_CANCEL_STMT, "Ca&ncel\tCtrl+.", "")
        .append_separator()
        .append_item(ID_HISTORY, "SQL &History\tF8", "")
        .append_item(ID_CLEAR, "C&lear\tCtrl+D", "")
        .build();
    let tools = Menu::builder()
        .append_item(ID_WORKSHEET, "SQL &Worksheet\tAlt+F10", "")
        .append_item(ID_UNSHARED, "&Unshared SQL Worksheet\tCtrl+Shift+N", "")
        .append_separator()
        .append_item(ID_SESSIONS, "S&essions…", "Sessions and locks of a connection: who blocks whom, kill session")
        .append_item(ID_TOPSQL, "&Top SQL…", "Heaviest statements on the server: elapsed, CPU, reads, executions, cost")
        .append_item(ID_DP_EXPORT, "Data Pump &Export…", "Export schemas, tables or the whole database with Oracle Data Pump (Oracle only)")
        .append_item(ID_DP_IMPORT, "Data Pump &Import…", "Import a dump file, uploading it first if it is on this computer (Oracle only)")
        .append_item(ID_DP_JOBS, "Data Pump &Jobs && Files…", "Running Data Pump jobs and the dump and log files on the server (Oracle only)")
        .append_item(ID_COMPARE, "Schema &Compare…", "Compare two schemas and open a script that brings the target in line")
        .append_item(ID_DRIVERS, "&Drivers…", "Download or locate native client drivers")
        .append_item(ID_PREFERENCES, "&Preferences…", "")
        .build();
    let help = Menu::builder()
        .append_item(ID_START, "&Start Page", "")
        .append_item(ID_KEYS, "&Keyboard Shortcuts", "")
        .append_item(ID_UPDATES, "Check for &Updates…", "")
        .append_separator()
        .append_item(ID_ABOUT, "&About rsqldev", "")
        .build();
    let bar = MenuBar::builder()
        .append(file, "&File")
        .append(edit, "&Edit")
        .append(view, "&View")
        .append(run, "&Run")
        .append(tools, "&Tools")
        .append(help, "&Help")
        .build();
    frame.set_menu_bar(bar);
    let current = match config::load_prefs().theme.as_str() {
        "dark" => ID_THEME_DARK,
        "system" => ID_THEME_SYSTEM,
        _ => ID_THEME_LIGHT,
    };
    if let Some(mb) = frame.get_menu_bar() {
        mb.check_item(current, true);
    }
}

fn toolbar(frame: &Frame) {
    let Some(tb) = frame.create_tool_bar(Some(ToolBarStyle::Default | ToolBarStyle::Flat), ID_ANY as i32) else { return };
    let add = |id: i32, icon: &str, label: &str, tip: &str| {
        if let Some(b) = icons::bitmap(icon, 16) {
            tb.add_tool(id, label, &b, tip);
        }
    };
    add(ID_NEW_CONN, "new", "New", "New Connection (Ctrl+Shift+C)");
    add(ID_OPEN, "open", "Open", "Open (Ctrl+O)");
    add(ID_SAVE, "save", "Save", "Save (Ctrl+S)");
    add(ID_SAVEALL, "saveall", "Save All", "Save All (Ctrl+Shift+S)");
    tb.add_separator();
    add(ID_BACK, "back", "Back", "Back (previous tab)");
    add(ID_FORWARD, "forward", "Forward", "Forward (next tab)");
    tb.add_separator();
    add(ID_WORKSHEET, "worksheet", "SQL Worksheet", "Open SQL Worksheet (Alt+F10)");
    add(ID_DRIVERS, "drivers", "Drivers", "Drivers…");
    tb.realize();
}

const SHORTCUTS: &str = "Worksheet
  Run Statement            Ctrl/Cmd+Enter
  Run Script               F5
  Explain Plan             F10
  Autotrace                F6
  Run PL/SQL               Ctrl/Cmd+F10
  Commit / Rollback        F11 / F12 (also in a table's Data tab)
  Cancel                   Ctrl/Cmd+.
  SQL History              F8
  Clear                    Ctrl/Cmd+D

Data tab (table editor)
  Insert Row               Ctrl/Cmd+I
  Delete Selected Row(s)   Ctrl/Cmd+D
  Refresh                  Ctrl/Cmd+R
  Paste cells              Ctrl/Cmd+V
  Edit Value (pencil)      Alt+Enter (date picker for DATE / TIMESTAMP)
  Unshared Worksheet       Ctrl/Cmd+Shift+N

Editor
  Completion Insight       Ctrl+Space (also after typing alias.)
  Format SQL               Ctrl/Cmd+F7
  Insert Snippet           Ctrl/Cmd+Shift+J
  Duplicate Lines          Ctrl/Cmd+Shift+D
  Move Lines Up / Down     Alt+Up / Alt+Down
  Matching Bracket         Ctrl/Cmd+]
  Bookmark: toggle         Ctrl/Cmd+K
  Bookmark: next / prev    F2 / Shift+F2
  Go to Definition         Ctrl+Click (Cmd+Click on macOS)
  Find / Replace           Ctrl/Cmd+F / Ctrl/Cmd+R
  Find Next / Previous     F3 / Shift+F3

Files and tabs
  New Connection           Ctrl/Cmd+Shift+C
  Open / Save / Save All   Ctrl/Cmd+O / S / Shift+S
  Close Tab                Ctrl/Cmd+W
  Recover Worksheet        Ctrl/Cmd+Shift+T
  New SQL Worksheet        Ctrl/Cmd+N (also Alt+F10)
  Find DB Object           Ctrl/Cmd+Shift+O

Grid
  Copy / Copy with Headers  right-click a cell
  Sort                      click a column header";

/// SQL Developer greets with a Start Page; this one offers first steps and recovery.
fn start_page(parent: &AuiNotebook, recover: &[autosave::Slot]) -> Panel {
    let p = Panel::builder(parent).build();
    let pal = theme::p();
    p.set_background_color(theme::c(pal.bg));
    let s = BoxSizer::builder(Orientation::Vertical).build();
    let font = |size: i32, weight: i32| Font::new_with_details(size, FontFamily::Swiss as i32, 0, weight, false, "");
    let title = StaticText::builder(&p).with_label("rsqldev").build();
    if let Some(f) = font(26, 700) {
        title.set_font(&f);
    }
    title.set_foreground_color(theme::c(pal.accent));
    let sub = StaticText::builder(&p)
        .with_label("Native SQL Developer for macOS and Windows. Oracle, PostgreSQL, MySQL, SQL Server, SQLite, DuckDB, ClickHouse and compatible databases.")
        .build();
    sub.set_foreground_color(theme::c(pal.muted));
    sub.wrap(620);
    s.add_spacer(36);
    s.add(&title, 0, SizerFlag::Left, 40);
    s.add_spacer(6);
    s.add(&sub, 0, SizerFlag::Left, 40);
    let section = |label: &str| {
        let h = StaticText::builder(&p).with_label(label).build();
        h.set_foreground_color(theme::c(pal.text));
        if let Some(f) = font(14, 700) {
            h.set_font(&f);
        }
        s.add_spacer(24);
        s.add(&h, 0, SizerFlag::Left, 40);
        s.add_spacer(6);
    };
    let link = |icon: &str, label: &str, on: Box<dyn Fn()>| {
        let row = BoxSizer::builder(Orientation::Horizontal).build();
        let ic = icons::tool(&p, icon, label);
        // Not a HyperlinkCtrl: wxDragon cannot mark its click handled, so wx also opened the
        // label as a URL in the browser.
        let l = StaticText::builder(&p).with_label(label).build();
        l.set_foreground_color(theme::c(pal.link));
        if let Some(mut f) = l.get_font() {
            f.set_underlined(true);
            l.set_font(&f);
        }
        l.set_cursor(wxdragon::cursor::Cursor::from_stock(wxdragon::cursor::StockCursor::Hand).as_ref());
        let on = std::rc::Rc::new(on);
        let o2 = on.clone();
        ic.on_click(move |_| o2());
        l.on_mouse_left_up(move |_| on());
        row.add(&ic, 0, SizerFlag::AlignCenterVertical, 0);
        row.add(&l, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 6);
        s.add_sizer(&row, 0, SizerFlag::Left, 40);
        s.add_spacer(4);
        (row, l, ic)
    };
    section("Get Started");
    for (id, icon, label) in [
        (ID_NEW_CONN, "new", "Create a Connection"),
        (ID_WORKSHEET, "worksheet", "Open SQL Worksheet"),
        (ID_OPEN, "open", "Open SQL File…"),
        (ID_DRIVERS, "drivers", "Manage Drivers…"),
        (ID_KEYS, "history", "Keyboard Shortcuts"),
    ] {
        link(icon, label, Box::new(move || {
            app().menu(id);
        }));
    }
    if !recover.is_empty() {
        section("Recover Unsaved Worksheets");
        for slot in recover.iter().take(8).cloned() {
            let label = autosave::label(&slot);
            let cell: std::rc::Rc<std::cell::Cell<Option<(StaticText, BitmapButton)>>> = Default::default();
            let c2 = cell.clone();
            let (_, l, ic) = link("worksheet", &label, Box::new(move || {
                if let Some((l, ic)) = c2.take() {
                    l.show(false);
                    ic.show(false);
                }
                app().recover(slot.clone());
            }));
            cell.set(Some((l, ic)));
        }
    }
    p.set_sizer(s, true);
    p
}

fn main() {
    SystemOptions::set_option_by_int("msw.no-manifest-check", 1);
    let prefs0 = config::load_prefs();
    rsqldev_core::lob::cleanup_stale();
    let _ = wxdragon::main(move |_| {
        if let Some(a) = wxdragon::app::get_app() {
            a.set_appearance(match prefs0.theme.as_str() {
                "dark" => wxdragon::appearance::Appearance::Dark,
                "system" => wxdragon::appearance::Appearance::System,
                _ => wxdragon::appearance::Appearance::Light,
            });
        }
        // Fit the usable screen area (menu bar / Dock / taskbar excluded), never off-screen.
        let (pos, size) = match wxdragon::display::Display::new(0).map(|d| d.client_area()) {
            Some(r) if r.width > 0 && r.height > 0 => {
                let (w, h) = ((r.width * 92 / 100).min(1600), (r.height * 92 / 100).min(1000));
                (Point::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2), Size::new(w, h))
            }
            _ => (Point::new(40, 40), Size::new(1200, 760)),
        };
        let frame = Frame::builder().with_title("rsqldev").with_position(pos).with_size(size).build();
        menus(&frame);
        toolbar(&frame);
        // Window / taskbar icon (the .app bundle and .exe carry their own icon files too).
        if let Some(b) = BitmapBundle::from_svg_text(include_str!("../assets/icon.svg"), Size::new(256, 256)).and_then(|b| b.get_bitmap(Size::new(256, 256))) {
            frame.set_icon(&b);
        }
        let sb = frame.create_status_bar(6, 0, ID_ANY as i32, "statusbar");
        sb.set_status_widths(&[-1, 150, 70, 90, 90, 130]);
        activity_bar(sb);
        let split = SplitterWindow::builder(&frame).build();
        // ponytail: AUI flags not exposed by wxDragon; values from wx/aui/auibook.h.
        let nb_flags = |close: bool| AuiNotebookStyle::from_bits_retain(0x1 | 0x20 | 0x100 | if close { 0x800 | 0x2000 } else { 0 });
        let left = AuiNotebook::builder(&split).with_style(nb_flags(false)).build();
        let nav = Navigator::new(&left);
        left.add_page(&nav.panel, "Connections", true);
        let nb = AuiNotebook::builder(&split).with_style(nb_flags(true)).build();
        let recover = autosave::list();
        let start = start_page(&nb, &recover);
        nb.add_page(&start, "Start Page", true);
        split.split_vertically(&left, &nb, 300);
        split.set_minimum_pane_size(140);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add(&split, 1, SizerFlag::Expand, 0);
        frame.set_sizer(s, true);

        let timer = Timer::new(&frame);
        timer.on_tick(|_| hub::tick());
        timer.start(25, false);
        let history: History = std::fs::read_to_string(config::config_root().join("history.toml")).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default();
        let a = Rc::new(App {
            frame,
            nb,
            _left: left,
            start: std::cell::Cell::new(start),
            nav,
            split,
            nav_shown: std::cell::Cell::new(true),
            prefs: RefCell::new(prefs0.clone()),
            conns: RefCell::new(config::load_connections()),
            open: RefCell::default(),
            pages: RefCell::default(),
            history: RefCell::new(history.entries),
            _timer: timer,
        });
        APP.with(|g| *g.borrow_mut() = Some(a.clone()));
        nav.reload(&a.conns.borrow());
        let mut ticks = 0u32;
        hub::every_tick(move || {
            let a = app();
            a.sync_tabs();
            ticks += 1;
            if ticks % 80 == 0 {
                a.autosave_tick();
            }
            true
        });
        frame.on_menu(move |e| {
            if !app().menu(e.get_id()) {
                e.skip(true);
            }
        });
        frame.on_close(move |e| {
            let app = app();
            app.save_history();
            app.autosave_tick();
            app.open.borrow_mut().clear();
            rsqldev_core::lob::cleanup_mine();
            e.skip(true);
        });
        frame.show(true);
        frame.set_status_text(if a.conns.borrow().is_empty() { "No connections yet: File ▸ New Connection…" } else { "Ready" }, 0);
        if let Ok(name) = std::env::var("RSQLDEV_SELFTEST") {
            selftest(name);
        } else if a.prefs().check_updates {
            check_updates(false);
        }
    });
}

/// RSQLDEV_SELFTEST=<connection> RSQLDEV_SELFTEST_PW=<pw>: drive the UI end to end, log, exit.
/// ponytail: one scripted scenario; grow into a UI test harness when the UI grows.
fn selftest(name: String) {
    let pw = std::env::var("RSQLDEV_SELFTEST_PW").ok();
    app().connect_with(&name, pw, |c| {
        let kind = c.borrow().kind;
        let schema = c.borrow().schema.clone();
        eprintln!("SELFTEST connected kind={kind:?} schema={schema}");
        let a = app();
        a.open_worksheet(c.clone());
        let ws = a.current_sheet().unwrap();
        let q = match kind {
            Kind::Oracle => "select level n from dual connect by level <= 500",
            Kind::Postgres => "select generate_series(1, 500) n",
            Kind::Mssql => "select top 500 row_number() over (order by (select null)) n from sys.all_objects a cross join sys.all_objects b",
            _ => "with recursive t(n) as (select 1 union all select n + 1 from t where n < 500) select n from t",
        };
        ws.editor.set_text(q);
        ws.run_statement();
        let mut step = 0;
        let start = std::time::Instant::now();
        hub::every_tick(move || {
            let el = start.elapsed().as_secs_f32();
            let (rows, done, out) = ws.probe();
            match step {
                0 if rows > 0 || el > 15.0 => {
                    eprintln!("SELFTEST run_statement rows={rows} done={done}");
                    ws.editor.set_text("create table rsq_ui (id int, name varchar(20));\ninsert into rsq_ui values (1, 'a');\nselect * from rsq_ui;\ndrop table rsq_ui;\n");
                    ws.run_script();
                    step = 1;
                }
                1 if out.contains("dropped") || out.contains("Error") || el > 30.0 => {
                    eprintln!("SELFTEST run_script output:\n{out}");
                    step = 2;
                }
                // Data editor: a table edited in its Data tab, then as an editable query result.
                2 => {
                    ws.editor.set_text("create table rsq_edit (id int primary key, name varchar(20));\ninsert into rsq_edit values (1, 'a');\ninsert into rsq_edit values (2, 'b');\ncommit;\n");
                    ws.run_script();
                    step = 10;
                }
                10 if out.contains("Commit") || el > 45.0 => {
                    let conn = ws.conn().borrow().cfg.name.clone();
                    let schema = ws.conn().borrow().schema.clone();
                    app().open_object(&conn, &schema, rsqldev_core::dialect::TABLE, &rsqldev_core::dialect::fold(kind, "rsq_edit"));
                    step = 11;
                }
                11 | 12 | 13 => {
                    let a = app();
                    let v = a.pages.borrow().iter().find_map(|p| if let Page::Obj(o) = p { Some(o.clone()) } else { None });
                    let Some(v) = v else { return true };
                    if step == 11 {
                        v.select_tab("Data");
                        step = 12;
                        return true;
                    }
                    let Some(g) = v.data_grid() else { return true };
                    let n = g.st.borrow().rows.len();
                    if step == 12 && n >= 2 && g.st.borrow().done {
                        let w = g.st.borrow().cols.len();
                        let name_col = w - 1;
                        g.set_cell(0, name_col, Some("a2".into()));
                        g.move_cursor(1, name_col as i32, false);
                        g.delete_selected_rows();
                        g.move_cursor(1, name_col as i32, false);
                        g.insert_row();
                        g.set_cell(2, name_col - 1, Some("3".into()));
                        g.set_cell(2, name_col, Some("c".into()));
                        eprintln!("SELFTEST data tab: {} rows, pending edits, commit", g.st.borrow().rows.len());
                        v.commit();
                        step = 13;
                    } else if step == 13 && !g.st.borrow().rows.iter().any(|r| r.dirty()) && g.st.borrow().done {
                        eprintln!("SELFTEST data tab after commit: {:?}", g.st.borrow().rows.iter().map(|r| r.cur.iter().skip(g.st.borrow().hidden).cloned().collect::<Vec<_>>()).collect::<Vec<_>>());
                        a.nb.set_selection(a.nb.get_page_index(&ws.panel) as usize);
                        ws.editor.set_text("select id, name from rsq_edit order by id");
                        ws.run_statement();
                        step = 14;
                    } else if el > 90.0 {
                        eprintln!("SELFTEST data tab timed out at step {step}");
                        step = 20;
                    }
                }
                14 if ws.result_editable() && done => {
                    ws.results.set_cell(0, 1, Some("w".into()));
                    ws.commit();
                    step = 15;
                }
                15 if !ws.has_unsaved_edits() && el > 1.0 => {
                    step = 16;
                }
                16 if done && rows > 0 => {
                    let st = ws.results.st.borrow();
                    eprintln!("SELFTEST worksheet after commit: {:?}", st.rows.iter().map(|r| r.cur.clone()).collect::<Vec<_>>());
                    drop(st);
                    ws.editor.set_text("drop table rsq_edit");
                    ws.run_statement();
                    step = 20;
                }
                14..=16 if el > 120.0 => {
                    eprintln!("SELFTEST worksheet edit timed out at step {step} editable={}", ws.result_editable());
                    step = 20;
                }
                20 => {
                    let a = app();
                    eprintln!("SELFTEST pages={} history={}", a.pages.borrow().len(), a.history().len());
                    if std::env::var("RSQLDEV_SELFTEST_KEEP").is_ok() {
                        // Leave a representative screen for screenshots: sample table, query, object viewer.
                        ws.editor.set_text("create table employees (employee_id int primary key, first_name varchar(30), last_name varchar(30), salary numeric(10,2), hire_date date);\ninsert into employees values (100, 'Steven', 'King', 24000, DATE '2003-06-17');\ninsert into employees values (101, 'Neena', 'Kochhar', 17000, DATE '2005-09-21');\ninsert into employees values (102, 'Lex', null, 17000, DATE '2001-01-13');\ncommit;\n");
                        ws.run_script();
                        step = 3;
                        return true;
                    }
                    a.frame.close(true);
                    return false;
                }
                3 if out.contains("Commit") || el > 40.0 => {
                    let a = app();
                    ws.editor.set_text("select e.employee_id, e.first_name, e.last_name, e.salary, e.hire_date\nfrom employees e\nwhere e.salary > 10000\norder by e.salary desc;");
                    ws.run_statement();
                    a.nav.expand_first();
                    let conn = ws.conn().borrow().cfg.name.clone();
                    let (schema, name) = (ws.conn().borrow().schema.clone(), rsqldev_core::dialect::fold(kind, "employees"));
                    a.open_object(&conn, &schema, rsqldev_core::dialect::TABLE, &name);
                    // Which screen to leave up for screenshots.
                    match std::env::var("RSQLDEV_SELFTEST_SHOW").as_deref() {
                        Ok("start") => {
                            a.nb.set_selection(0);
                        }
                        Ok("worksheet") => {
                            a.nb.set_selection(1);
                        }
                        Ok("data") => {
                            let v = a.pages.borrow().iter().find_map(|p| if let Page::Obj(o) = p { Some(o.clone()) } else { None });
                            if let Some(v) = v {
                                v.select_tab("Data");
                            }
                        }
                        Ok("dialog") => a.edit_connection(&conn),
                        Ok("drivers") => driver_manager::show(&a.frame),
                        _ => {}
                    }
                    return false;
                }
                _ => {}
            }
            true
        });
    });
}
