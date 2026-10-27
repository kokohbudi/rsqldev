//! SQL Worksheet: editor, Run Statement / Run Script / Explain Plan, result panes,
//! SQL*Plus command subset, DBMS output, history.

use crate::conn::{Conn, ConnRef, Done};
use std::rc::Weak;
use crate::{editor, icons};
use crate::grid::ResultGrid;
use rsqldev_core::dialect;
use rsqldev_core::registry::Kind;
use rsqldev_core::script::{self, Action, Defines, Item};
use rsqldev_core::worker::Cmd;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use wxdragon::prelude::*;

pub struct Worksheet {
    pub panel: Panel,
    pub editor: StyledTextCtrl,
    pub find: Rc<crate::find::FindBar>,
    conn: RefCell<ConnRef>,
    conn_choice: Choice,
    bottom: AuiNotebook,
    pub results: ResultGrid,
    script_out: TextCtrl,
    explain: ResultGrid,
    dbms: TextCtrl,
    /// Query Result pane message ("All Rows Fetched: 14 in 0.003 seconds").
    status: StaticText,
    /// Script Output pane message ("Task completed in 0.04 seconds").
    task: StaticText,
    elapsed: StaticText,
    defines: RefCell<Defines>,
    /// Last value typed per bind name (upper case), offered again next run.
    binds: RefCell<std::collections::HashMap<String, String>>,
    pub path: RefCell<Option<PathBuf>>,
    last: RefCell<Option<String>>,
    /// SPOOL target, open for appending (unbuffered: the file shows output as it comes).
    spool: RefCell<Option<std::fs::File>>,
    running: Cell<bool>,
    started: Cell<Option<Instant>>,
    me: Weak<Worksheet>,
    b_cancel: BitmapButton,
    busy: Cell<bool>,
    /// Autosave slot (`<config>/worksheets/<id>.toml`).
    pub autosave_id: u64,
    /// Text changed since the last autosave (set by the editor's change event).
    pub unsaved: Cell<bool>,
    /// Posts edits of a single-table query result (SQL Developer: editable query results).
    dml: RefCell<Option<Rc<crate::data_editor::Editor>>>,
    /// Bumped per Run Statement: a late editability check of an older run is dropped.
    run_gen: Cell<u64>,
    last_line: Cell<i32>,
}

const P_RESULT: usize = 0;
const P_SCRIPT: usize = 1;
const P_EXPLAIN: usize = 2;
const P_DBMS: usize = 3;

pub type SheetRef = Rc<Worksheet>;

/// A pane with a small icon toolbar and a message label on top, like SQL Developer's result panes.
fn pane(parent: &AuiNotebook, tools: &[(&str, &str)]) -> (Panel, BoxSizer, Vec<BitmapButton>, StaticText) {
    let p = Panel::builder(parent).build();
    let bar = BoxSizer::builder(Orientation::Horizontal).build();
    let btns: Vec<BitmapButton> = tools
        .iter()
        .map(|(icon, tip)| {
            let b = icons::tool(&p, icon, tip);
            bar.add(&b, 0, SizerFlag::All, 2);
            b
        })
        .collect();
    let label = StaticText::builder(&p).with_label("").build();
    bar.add(&label, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
    (p, s, btns, label)
}

impl Worksheet {
    pub fn new(parent: &dyn WxWidget, conn: ConnRef) -> SheetRef {
        let prefs = crate::app().prefs();
        let panel = Panel::builder(parent).build();
        let bar = BoxSizer::builder(Orientation::Horizontal).build();
        let mk = |icon: &str, tip: &str| {
            let b = icons::tool(&panel, icon, tip);
            bar.add(&b, 0, SizerFlag::All, 3);
            b
        };
        let sep = || {
            let l = StaticLine::builder(&panel).with_style(StaticLineStyle::Vertical).with_size(Size::new(1, 18)).build();
            bar.add(&l, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left | SizerFlag::Right, 4);
        };
        // SQL Developer worksheet toolbar order.
        let b_run = mk("run", "Run Statement (Ctrl+Enter)");
        let b_script = mk("script", "Run Script (F5)");
        sep();
        let b_explain = mk("explain", "Explain Plan (F10)");
        let b_autotrace = mk("autotrace", "Autotrace (F6)");
        sep();
        let b_commit = mk("commit", "Commit (F11)");
        let b_rollback = mk("rollback", "Rollback (F12)");
        sep();
        let b_unshared = mk("unshared", "Unshared SQL Worksheet (Ctrl+Shift+N)");
        let b_case = mk("case", "To Upper/Lower/Initcap (Ctrl+')");
        let b_clear = mk("clear", "Clear (Ctrl+D)");
        sep();
        let b_hist = mk("history", "SQL History (F8)");
        let b_cancel = mk("cancel", "Cancel");
        bar.add_stretch_spacer(1);
        let elapsed = StaticText::builder(&panel).with_label("").with_style(StaticTextStyle::AlignRight).build();
        elapsed.set_min_size(Size::new(110, -1));
        bar.add(&elapsed, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 10);
        let names: Vec<String> = crate::app().connection_names();
        let conn_choice = Choice::builder(&panel).with_choices(names.clone()).build();
        if let Some(i) = names.iter().position(|n| *n == conn.borrow().cfg.name) {
            conn_choice.set_selection(i as u32);
        }
        bar.add(&conn_choice, 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);

        let split = SplitterWindow::builder(&panel).build();
        let editor = editor::new_editor(&split, &prefs.editor_font, prefs.editor_font_size);
        let bottom = AuiNotebook::builder(&split).with_style(AuiNotebookStyle::from_bits_retain(0x1 | 0x20 | 0x100)).build();
        let mono = |p: &Panel| {
            let t = TextCtrl::builder(p).with_style(TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly | TextCtrlStyle::DontWrap).build();
            crate::copy::text(t);
            if let Some(f) = Font::new_with_details(prefs.editor_font_size, FontFamily::Teletype as i32, 0, 400, false, &prefs.editor_font) {
                t.set_font(&f);
            }
            t
        };
        let (qp, qs, qb, status) = pane(&bottom, &[("refresh", "Refresh (re-run)"), ("export", "Export…"), ("edit", "View / Edit Value… (Alt+Enter)")]);
        let results = ResultGrid::new(&qp);
        results.st.borrow_mut().db = Some(conn.borrow().kind);
        qs.add(&results.grid, 1, SizerFlag::Expand, 0);
        qp.set_sizer(qs, true);
        let (sp, ss, sb, task) = pane(&bottom, &[("clear", "Clear"), ("save", "Save Output…")]);
        let script_out = mono(&sp);
        ss.add(&script_out, 1, SizerFlag::Expand, 0);
        sp.set_sizer(ss, true);
        let (ep, es, _, _) = pane(&bottom, &[]);
        let explain = ResultGrid::new(&ep);
        es.add(&explain.grid, 1, SizerFlag::Expand, 0);
        ep.set_sizer(es, true);
        let (dp, ds, db, _) = pane(&bottom, &[("new", "Enable DBMS Output for this connection"), ("clear", "Clear"), ("save", "Save…")]);
        let dbms = mono(&dp);
        ds.add(&dbms, 1, SizerFlag::Expand, 0);
        dp.set_sizer(ds, true);
        bottom.add_page(&qp, "Query Result", true);
        bottom.add_page(&sp, "Script Output", false);
        bottom.add_page(&ep, "Explain Plan", false);
        bottom.add_page(&dp, "DBMS Output", false);
        split.split_horizontally(&editor, &bottom, 300);
        split.set_minimum_pane_size(60);

        let find = crate::find::FindBar::new(&panel, editor);
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&bar, 0, SizerFlag::Expand, 0);
        s.add(&find.panel, 0, SizerFlag::Expand, 0);
        s.add(&split, 1, SizerFlag::Expand, 0);
        panel.set_sizer(s, true);

        let autocommit = prefs.autocommit;
        b_commit.enable(!autocommit);
        b_rollback.enable(!autocommit);
        b_cancel.enable(false);
        let ws = Rc::new_cyclic(|me| Worksheet {
            panel,
            editor,
            find,
            conn: RefCell::new(conn),
            conn_choice,
            bottom,
            results,
            script_out,
            explain,
            dbms,
            status,
            task,
            elapsed,
            defines: RefCell::default(),
            binds: RefCell::default(),
            path: RefCell::default(),
            last: RefCell::default(),
            spool: RefCell::default(),
            running: Cell::new(false),
            started: Cell::new(None),
            me: me.clone(),
            dml: RefCell::default(),
            run_gen: Cell::new(0),
            last_line: Cell::new(1),
            b_cancel,
            busy: Cell::new(false),
            autosave_id: crate::autosave::next_id(),
            unsaved: Cell::new(false),
        });
        let acts: Vec<(BitmapButton, fn(&Worksheet))> = vec![
            (b_run, Worksheet::run_statement),
            (b_script, Worksheet::run_script),
            (b_explain, Worksheet::explain_plan),
            (b_autotrace, Worksheet::autotrace),
            (b_commit, Worksheet::commit),
            (b_rollback, Worksheet::rollback),
            (b_unshared, |_| {
                crate::app().menu(crate::ID_UNSHARED);
            }),
            (b_case, Worksheet::toggle_case),
            (b_clear, Worksheet::clear),
            (b_hist, Worksheet::history),
            (b_cancel, Worksheet::cancel),
            (qb[0], Worksheet::rerun),
            (qb[1], |w| w.results.export_dialog()),
            (qb[2], |w| w.results.edit_value()),
            (sb[0], |w| w.script_out.clear()),
            (sb[1], |w| save_text(&w.panel, &w.script_out.get_value())),
            (db[0], |w| w.set_output(true)),
            (db[1], |w| w.dbms.clear()),
            (db[2], |w| save_text(&w.panel, &w.dbms.get_value())),
        ];
        for (b, f) in acts {
            let w = Rc::downgrade(&ws);
            b.on_click(move |_| {
                if let Some(w) = w.upgrade() {
                    f(&w)
                }
            });
        }
        let w = Rc::downgrade(&ws);
        conn_choice.on_selection_changed(move |_| {
            let Some(ws) = w.upgrade() else { return };
            let Some(name) = ws.conn_choice.get_string_selection() else { return };
            let w2 = ws.me.clone();
            crate::app().connect(&name.clone(), move |c| {
                if let Some(ws) = w2.upgrade() {
                    ws.results.st.borrow_mut().db = Some(c.borrow().kind);
                    *ws.conn.borrow_mut() = c;
                    crate::app().retitle(&ws.panel, &name);
                }
            });
        });
        let w = Rc::downgrade(&ws);
        ws.editor.on_stc_update_ui(move |_| {
            if let Some(w) = w.upgrade() {
                let e = w.editor;
                let pos = e.get_current_pos();
                let line = e.line_from_position(pos);
                let col = pos - e.position_from_line(line) + 1;
                let app = crate::app();
                app.status(1, &format!("Line {} Column {col}", line + 1));
                app.status(2, "Insert");
                app.status(3, if e.is_modified() { "Modified" } else { "" });
            }
        });
        let w = Rc::downgrade(&ws);
        ws.editor.on_stc_char_added(move |e| {
            if let (Some(w), Some(k)) = (w.upgrade(), e.get_key()) {
                if k == '.' as i32 {
                    crate::complete::show(w.editor, &w.conn(), false);
                } else if let Some(c) = char::from_u32(k as u32) {
                    crate::complete::typed(w.editor, &w.conn(), c);
                }
            }
        });
        // Autosave copies the text only after a change (no 2-second full-text copy and compare).
        let w = Rc::downgrade(&ws);
        ws.editor.on_stc_change(move |_| {
            if let Some(w) = w.upgrade() {
                w.unsaved.set(true);
            }
        });
        // Ctrl/Cmd+F7 formats too (SQL Developer's key; the menu shows Ctrl/Cmd+Shift+F).
        let w = Rc::downgrade(&ws);
        ws.editor.on_key_down(move |e| {
            if let (WindowEventData::Keyboard(k), Some(w)) = (&e, w.upgrade()) {
                if k.cmd_down() && k.get_key_code() == Some(wxdragon::keycode::WXK_F7) {
                    return w.format_sql();
                }
            }
            e.skip(true);
        });
        // Ctrl+Click (Cmd+Click on macOS): open the object under the caret, or select an alias's source.
        let w = Rc::downgrade(&ws);
        ws.editor.on_mouse_left_up(move |_| {
            let Some(w) = w.upgrade() else { return };
            if !wxdragon::utils::get_key_state(wxdragon::keycode::WXK_COMMAND) {
                return;
            }
            let (text, pos) = (w.editor.get_text(), w.editor.get_current_pos() as usize);
            // An alias: select what it stands for (the table, or the subquery / CTE).
            if let Some((a, b)) = crate::editor::alias_target_at(&text, pos) {
                return w.editor.set_selection(a as i32, b as i32);
            }
            let ident = ident_at(&text, pos);
            if !ident.is_empty() {
                crate::app().goto_object(w.conn(), &ident);
            }
        });
        let w3 = Rc::downgrade(&ws);
        ws.results.st.borrow_mut().on_change = Some(Rc::new(move |st| {
            if let Some(w) = w3.upgrade() {
                let secs = w.started.get().map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
                let n = st.rows.len();
                let msg = if st.done { format!("All Rows Fetched: {n} in {secs:.3} seconds") } else { format!("Fetched {n} rows in {secs:.3} seconds") };
                w.status.set_label(&msg);
                w.elapsed.set_label(&format!("{secs:.3} seconds"));
                // Rows are on screen: the statement has executed (the cursor may stay open).
                if !w.running.get() {
                    w.set_busy(false);
                }
            }
        }));
        ws.editor.set_focus();
        ws
    }

    /// Statement in flight: Cancel enabled, tab marked.
    fn set_busy(&self, on: bool) {
        if self.busy.replace(on) == on {
            return;
        }
        self.b_cancel.enable(on);
        let name = self.conn().borrow().cfg.name.clone();
        crate::app().retitle(&self.panel, &if on { format!("⏳ {name}") } else { name });
    }

    fn weak(&self) -> Weak<Worksheet> {
        self.me.clone()
    }

    /// Format the selection (or everything) as one undo step.
    pub fn format_sql(&self) {
        let e = self.editor;
        let (mut a, mut b) = (e.get_selection_start(), e.get_selection_end());
        if a == b {
            (a, b) = (0, e.get_length());
        }
        let text = e.get_text();
        let Some(part) = text.get(a as usize..b as usize) else { return };
        let out = rsqldev_core::format::script(part, self.kind());
        if out != part {
            e.set_target_start(a);
            e.set_target_end(b);
            e.replace_target(&out);
        }
    }

    pub fn conn(&self) -> ConnRef {
        self.conn.borrow().clone()
    }

    fn kind(&self) -> Kind {
        self.conn.borrow().borrow().kind
    }

    fn rerun(&self) {
        if let Some(sql) = self.last.borrow().clone() {
            self.run_sql_grid(sql, 0);
        }
    }

    /// To Upper → To Lower → Initcap on the selection (or the word at the caret).
    fn toggle_case(&self) {
        let e = self.editor;
        let mut sel = e.get_selected_text();
        if sel.is_empty() {
            let p = e.get_current_pos();
            e.set_selection(e.word_start_position(p, true), e.word_end_position(p, true));
            sel = e.get_selected_text();
        }
        let start = e.get_selection_start();
        let next = if sel == sel.to_uppercase() && sel != sel.to_lowercase() {
            sel.to_lowercase()
        } else if sel == sel.to_lowercase() {
            let mut cap = true;
            sel.chars()
                .map(|c| {
                    let o = if cap { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
                    cap = !c.is_alphanumeric();
                    o
                })
                .collect()
        } else {
            sel.to_uppercase()
        };
        e.replace_selection(&next);
        e.set_selection(start, start + next.len() as i32);
    }

    fn out(&self, text: &str) {
        self.script_out.append_text(text);
        if !text.ends_with('\n') {
            self.script_out.append_text("\n");
        }
        if let Some(mut f) = self.spool.borrow().as_ref() {
            use std::io::Write;
            let _ = writeln!(f, "{}", text.trim_end_matches('\n'));
        }
    }

    fn dbms_lines(&self, lines: &[String]) {
        for l in lines {
            self.dbms.append_text(&format!("{l}\n"));
        }
        if !lines.is_empty() {
            self.bottom.set_selection(P_DBMS);
        }
    }

    fn set_output(&self, on: bool) {
        let w = self.task;
        Conn::tx(&self.conn(), Cmd::Output(on), move |r| w.set_label(&r.map(str::to_string).unwrap_or_else(|e| e)));
        self.bottom.set_selection(P_DBMS);
    }

    /// Text to run: the selection, or everything.
    fn source(&self) -> (String, usize) {
        let sel = self.editor.get_selected_text();
        if sel.trim().is_empty() { (self.editor.get_text(), 0) } else { (sel, self.editor.get_selection_start() as usize) }
    }

    /// `&name` substitution, then `:name` binds (asked with the last value typed for that name).
    fn substitute(&self, sql: &str) -> Option<String> {
        let p = self.panel;
        let sql = self.defines.borrow_mut().substitute(sql, &mut |name| crate::ui::ask(&p, &format!("Enter value for {name}:"), "Enter Substitution Variable", ""))?;
        let mut last = self.binds.borrow_mut();
        script::bind_literals(&sql, &mut |name| {
            let key = name.to_uppercase();
            let v = crate::ui::ask(&p, &format!("Value for :{name} (empty = NULL):"), "Enter Binds", last.get(&key).map_or("", String::as_str))?;
            last.insert(key, v.clone());
            Some(v)
        })
    }

    /// DELETE / UPDATE without WHERE: ask before it touches every row.
    fn allow(&self, sql: &str) -> bool {
        let Some(verb) = script::unbounded_dml(sql) else { return true };
        let head: String = sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
        crate::ui::confirm(&self.panel, &format!("{verb} without WHERE changes every row of the table:\n\n{head}\n\nRun it anyway?"))
    }

    /// Tint editor lines named in a compile-error report (`LINE/COL  ERROR` rows, numbered from
    /// the statement that starts at 1-based editor line `start`).
    fn mark_errors(&self, start: i32, err: &str) {
        for l in err.lines() {
            let Some((n, _)) = l.split_once('/') else { continue };
            if let Ok(n) = n.trim().parse::<i32>() {
                self.editor.marker_add(start + n - 2, editor::MARK_ERROR);
            }
        }
    }

    fn line_of(&self, pos: usize) -> i32 {
        self.editor.line_from_position(pos as i32) + 1
    }

    /// Uncommitted edits in the Query Result: Commit (then `again`), Discard or Cancel.
    /// True = do not go on now.
    fn unsaved_edits(&self, again: fn(&Worksheet)) -> bool {
        let Some(ed) = self.dml.borrow().clone().filter(|e| e.pending()) else { return false };
        match crate::ui::commit_discard(&self.panel, "The Query Result has changes that are not committed.") {
            Some(true) => {
                let me = self.weak();
                ed.commit_then(Box::new(move || {
                    if let Some(me) = me.upgrade() {
                        again(&me)
                    }
                }));
                true
            }
            Some(false) => {
                ed.discard();
                *self.dml.borrow_mut() = None;
                false
            }
            None => true,
        }
    }

    /// The Query Result is editable (self-test).
    pub fn result_editable(&self) -> bool {
        self.dml.borrow().is_some()
    }

    /// Query Result edits not committed yet (closing the worksheet asks).
    pub fn has_unsaved_edits(&self) -> bool {
        self.dml.borrow().as_ref().is_some_and(|e| e.pending())
    }

    pub fn run_statement(&self) {
        if self.unsaved_edits(Worksheet::run_statement) {
            return;
        }
        let sel = self.editor.get_selected_text();
        let item = if !sel.trim().is_empty() {
            script::split(&sel, self.kind()).into_iter().next()
        } else {
            let text = self.editor.get_text();
            let items = script::split(&text, self.kind());
            script::statement_at(&items, self.editor.get_current_pos() as usize).cloned()
        };
        let Some(item) = item else { return };
        if item.kind == script::Kind::Command {
            self.script_out.clear();
            self.run_items(vec![item], 0);
            return;
        }
        let Some(sql) = self.substitute(&item.text) else { return };
        if !self.allow(&sql) {
            return;
        }
        crate::app().add_history(&sql);
        let line = self.line_of(item.start);
        let sql = if item.kind == script::Kind::Block { format!("\u{1}{sql}") } else { sql };
        self.run_sql_grid(sql, line);
    }

    /// Execute one statement; queries go to the Query Result grid, the rest to Script Output.
    /// A leading U+0001 marks a PL/SQL block (never a query).
    fn run_sql_grid(&self, sql: String, line: i32) {
        let (block, sql) = match sql.strip_prefix('\u{1}') {
            Some(s) => (true, s.to_string()),
            None => (false, sql),
        };
        *self.last.borrow_mut() = Some(if block { format!("\u{1}{sql}") } else { sql.clone() });
        self.editor.marker_delete_all(editor::MARK_ERROR);
        self.started.set(Some(Instant::now()));
        self.status.set_label("Executing…");
        self.elapsed.set_label("");
        self.set_busy(true);
        let (w, el, task) = (self.status, self.elapsed, self.task);
        let me = self.weak();
        let query = rsqldev_core::is_query(&sql) && !block;
        let sql2 = sql.clone();
        let rows = Conn::exec(&self.conn(), &sql, move |d: Done| {
            let Some(me) = me.upgrade() else { return };
            me.set_busy(false);
            me.dbms_lines(&d.output);
            let secs = format!("{:.3} seconds", d.elapsed.as_secs_f32());
            match d.result {
                Err(e) => {
                    me.script_out.clear();
                    me.out(&format!("Error starting at line : {line} in command -\n{sql2}\nError report -\n{e}"));
                    me.mark_errors(line, &e);
                    me.bottom.set_selection(P_SCRIPT);
                    w.set_label("");
                    task.set_label(&format!("Task completed in {secs}"));
                    el.set_label(&secs);
                }
                Ok(aff) if !query => {
                    me.script_out.clear();
                    me.out(&script::feedback(&sql2, aff));
                    me.bottom.set_selection(P_SCRIPT);
                    task.set_label(&format!("Task completed in {secs}"));
                    el.set_label(&secs);
                }
                Ok(_) => {}
            }
        });
        if query {
            self.results.load(rows, 0, false);
            self.results.st.borrow_mut().count = Conn::counter(&self.conn(), &sql, self.status);
            self.results.st.borrow_mut().resume = Conn::resume(&self.conn(), &sql);
            self.bottom.set_selection(P_RESULT);
            self.last_line.set(line);
            self.editable_result(&sql);
        }
    }

    /// A query of one table whose every result column is a column of that table gets an
    /// editable grid: edits post on Commit (F11), keyed by the primary key (else every column).
    fn editable_result(&self, sql: &str) {
        *self.dml.borrow_mut() = None;
        let run = self.run_gen.get() + 1;
        self.run_gen.set(run);
        let Some((o, t)) = rsqldev_core::dml::single_table(sql) else { return };
        let conn = self.conn();
        let db = conn.borrow().kind;
        let name = rsqldev_core::ddl::ident(db, &t);
        let me = self.weak();
        let c2 = conn.clone();
        let check = move |owner: String| {
            let Some(cols_sql) = dialect::children_sql(db, dialect::TABLE) else { return };
            let c3 = c2.clone();
            Conn::meta_query(&c2, &dialect::fill(db, &cols_sql, &owner, &name), move |r| {
                let Ok(r) = r else { return };
                let table_cols: Vec<String> = r.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect();
                if table_cols.is_empty() {
                    return;
                }
                // Wait for the result's columns, then compare.
                crate::hub::every_tick(move || {
                    let Some(ws) = me.upgrade() else { return false };
                    if ws.run_gen.get() != run {
                        return false;
                    }
                    let st = ws.results.st.borrow();
                    if st.cols.is_empty() {
                        return !st.done;
                    }
                    let ok = st.cols.iter().all(|c| table_cols.iter().any(|t| t.eq_ignore_ascii_case(&c.name)));
                    drop(st);
                    if ok {
                        let (w1, w2) = (me.clone(), me.clone());
                        let ed = crate::data_editor::Editor::new(
                            c3.clone(),
                            &owner,
                            &name,
                            ws.results.clone(),
                            None,
                            move |line| {
                                if let Some(ws) = w1.upgrade() {
                                    ws.out(line)
                                }
                            },
                            move || {
                                if let Some(ws) = w2.upgrade() {
                                    let sql = ws.last.borrow().clone();
                                    if let Some(sql) = sql {
                                        ws.run_sql_grid(sql, ws.last_line.get());
                                    }
                                }
                            },
                        );
                        ws.results.set_editable(true);
                        *ws.dml.borrow_mut() = Some(ed);
                        crate::app().status(0, &format!("Query Result is editable ({name}): Commit (F11) posts the changes"));
                    }
                    false
                });
            });
        };
        match o {
            Some(o) => check(rsqldev_core::ddl::ident(db, &o)),
            None => Conn::meta_query(&conn, dialect::current_schema_sql(db), move |r| {
                if let Some(owner) = r.ok().and_then(|r| r.rows.into_iter().next()).and_then(|row| row.into_iter().next().flatten()) {
                    check(owner)
                }
            }),
        }
    }

    pub fn run_script(&self) {
        if self.unsaved_edits(Worksheet::run_script) {
            return;
        }
        self.editor.marker_delete_all(editor::MARK_ERROR);
        let (text, base) = self.source();
        let items: Vec<Item> = script::split(&text, self.kind())
            .into_iter()
            .map(|mut i| {
                i.start += base;
                i.end += base;
                i
            })
            .collect();
        if items.is_empty() {
            return;
        }
        self.script_out.clear();
        self.bottom.set_selection(P_SCRIPT);
        self.started.set(Some(Instant::now()));
        self.running.set(true);
        self.set_busy(true);
        crate::app().add_history(&text);
        self.run_items(items, 0);
    }

    /// Run script items one after another (each waits for the previous reply).
    fn run_items(&self, items: Vec<Item>, i: usize) {
        if i >= items.len() || !self.running.get() && i > 0 {
            self.running.set(false);
            self.set_busy(false);
            let secs = self.started.get().map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
            self.task.set_label(&format!("Task completed in {secs:.3} seconds"));
            self.elapsed.set_label(&format!("{secs:.3} seconds"));
            return;
        }
        let me = self.weak();
        let next = move |items: Vec<Item>| {
            if let Some(me) = me.upgrade() {
                me.run_items(items, i + 1)
            }
        };
        let item = items[i].clone();
        if item.kind == script::Kind::Command {
            let action = script::command(&item.text, &mut self.defines.borrow_mut());
            match action {
                Action::Run(sql) => return self.script_sql(sql, item.start, items, i, Box::new(next)),
                Action::Print(t) => self.out(&t),
                Action::Output(on) => self.set_output(on),
                Action::Clear => self.script_out.clear(),
                Action::Nothing => {}
                Action::Spool(p) => {
                    *self.spool.borrow_mut() = None;
                    if let Some(p) = p {
                        match std::fs::OpenOptions::new().append(true).create(true).open(&p) {
                            Ok(f) => *self.spool.borrow_mut() = Some(f),
                            Err(e) => self.out(&format!("SPOOL {p}: {e}")),
                        }
                    }
                }
                Action::Rerun => {
                    if let Some(sql) = self.last.borrow().clone() {
                        return self.script_sql(sql.trim_start_matches('\u{1}').to_string(), item.start, items, i, Box::new(next));
                    }
                }
                Action::Describe(obj) => {
                    let (owner, name) = match obj.split_once('.') {
                        Some((o, n)) => (dialect::fold(self.kind(), o), dialect::fold(self.kind(), n)),
                        None => (self.conn().borrow().schema.clone(), dialect::fold(self.kind(), &obj)),
                    };
                    if let Some(sql) = dialect::describe_sql(self.kind(), &owner, &name) {
                        return self.script_sql(sql, item.start, items, i, Box::new(next));
                    }
                }
                Action::Include(path) => {
                    let base = self.path.borrow().as_ref().and_then(|p| p.parent().map(|p| p.to_path_buf()));
                    let mut p = PathBuf::from(path.trim_matches('"'));
                    if p.extension().is_none() {
                        p.set_extension("sql");
                    }
                    if p.is_relative() {
                        if let Some(b) = base {
                            p = b.join(p);
                        }
                    }
                    match std::fs::read_to_string(&p) {
                        Ok(s) => {
                            let mut items = items;
                            let inc = script::split(&s, self.kind()).into_iter().map(|mut it| {
                                it.start = item.start;
                                it.end = item.end;
                                it
                            });
                            items.splice(i + 1..i + 1, inc);
                            return next(items);
                        }
                        Err(e) => self.out(&format!("SP2-0310: unable to open file \"{}\": {e}", p.display())),
                    }
                }
                Action::Unsupported(t) => self.out(&format!("Command skipped (not supported): {t}")),
            }
            return next(items);
        }
        let Some(sql) = self.substitute(&item.text) else {
            self.running.set(false);
            return;
        };
        if !self.allow(&sql) {
            self.out(&format!("\nScript stopped at line {}: not confirmed.\n", self.line_of(item.start)));
            self.running.set(false);
            return next(items);
        }
        self.script_sql(sql, item.start, items, i, Box::new(next));
    }

    fn script_sql(&self, sql: String, start: usize, items: Vec<Item>, _i: usize, next: Box<dyn FnOnce(Vec<Item>)>) {
        *self.last.borrow_mut() = Some(sql.clone());
        let me = self.weak();
        let line = self.line_of(start);
        Conn::work_query(&self.conn(), &sql.clone(), move |r| {
            let Some(me) = me.upgrade() else { return };
            match r {
                Ok(r) => {
                    if r.cols.is_empty() {
                        me.out(&format!("\n{}\n", script::feedback(&sql, r.affected)));
                    } else {
                        me.out(&script::format_table(&r.cols, &r.rows));
                    }
                    me.dbms_lines(&r.output);
                }
                Err(e) => {
                    me.out(&format!("\nError starting at line : {line} in command -\n{sql}\nError report -\n{e}\n"));
                    me.mark_errors(line, &e);
                }
            }
            next(items);
        });
    }

    /// The selection, or the statement under the caret, substituted.
    fn current_sql(&self) -> Option<String> {
        let sel = self.editor.get_selected_text();
        let sql = if sel.trim().is_empty() {
            let text = self.editor.get_text();
            let items = script::split(&text, self.kind());
            script::statement_at(&items, self.editor.get_current_pos() as usize)?.text.clone()
        } else {
            sel
        };
        self.substitute(&sql)
    }

    pub fn explain_plan(&self) {
        let Some(sql) = self.current_sql() else { return };
        let ex = dialect::explain(self.kind(), &sql);
        self.bottom.set_selection(P_EXPLAIN);
        self.explain.reset();
        self.set_busy(true);
        self.explain_step(ex.stmts, ex.show, 0);
    }

    /// Run ▸ Run PL/SQL (Oracle): a worksheet with an anonymous block calling the procedure or
    /// function under the caret, its arguments declared to fill in.
    pub fn run_plsql(&self) {
        if self.kind() != Kind::Oracle {
            return crate::ui::info(&self.panel, "Run PL/SQL is available on Oracle connections.");
        }
        let sel = self.editor.get_selected_text();
        let guess = if sel.trim().is_empty() { ident_at(&self.editor.get_text(), self.editor.get_current_pos() as usize) } else { sel.trim().to_string() };
        let Some(name) = crate::ui::ask(&self.panel, "Procedure or function ([owner.][package.]name):", "Run PL/SQL", &guess).filter(|n| !n.trim().is_empty()) else { return };
        let c = self.conn();
        let c2 = c.clone();
        Conn::meta_query(&c, &dialect::plsql_args_sql(name.trim()), move |r| {
            let app = crate::app();
            match r.map(|r| dialect::plsql_block(&r.rows)) {
                Ok(Some(block)) => {
                    app.open_worksheet(c2);
                    if let Some(ws) = app.current_sheet() {
                        ws.editor.set_text(&block);
                    }
                }
                Ok(None) => crate::ui::info(&app.frame, &format!("No procedure or function {} visible to this user.", name.trim())),
                Err(e) => crate::ui::error(&app.frame, &e),
            }
        });
    }

    /// Run the query and show the plan with actual rows / time (Oracle ALLSTATS LAST,
    /// EXPLAIN ANALYZE elsewhere) in the Explain Plan pane.
    pub fn autotrace(&self) {
        let Some(sql) = self.current_sql() else { return };
        let tag = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        let Some(ex) = dialect::autotrace(self.kind(), &sql, tag) else {
            return crate::ui::info(&self.panel, "Autotrace runs the statement, so it is offered for queries only (SELECT / WITH without DML or FOR UPDATE), on Oracle, PostgreSQL, MySQL and DuckDB.");
        };
        self.bottom.set_selection(P_EXPLAIN);
        self.explain.reset();
        self.set_busy(true);
        self.explain_step(ex.stmts, ex.show, 0);
    }

    fn explain_step(&self, stmts: Vec<String>, show: usize, i: usize) {
        if i >= stmts.len() {
            return self.set_busy(false);
        }
        let me = self.weak();
        let sql = stmts[i].clone();
        Conn::work_query(&self.conn(), &sql, move |r| {
            let Some(me) = me.upgrade() else { return };
            match r {
                Ok(r) if i == show => me.explain.set(r.cols, r.rows),
                Ok(_) => {}
                Err(e) => {
                    me.out(&format!("Explain failed:\n{e}"));
                    me.bottom.set_selection(P_SCRIPT);
                    if i < show {
                        return me.set_busy(false);
                    }
                }
            }
            me.explain_step(stmts, show, i + 1);
        });
    }

    pub fn commit(&self) {
        // Pending grid edits of an editable result: post them, then COMMIT.
        let ed = self.dml.borrow().clone();
        if ed.is_some_and(|ed| ed.pending() && ed.commit()) {
            return;
        }
        let me = self.weak();
        Conn::tx(&self.conn(), Cmd::Commit, move |r| {
            if let Some(me) = me.upgrade() {
                me.out(&r.map(str::to_string).unwrap_or_else(|e| e));
            }
        });
    }

    pub fn rollback(&self) {
        let ed = self.dml.borrow().clone();
        if let Some(ed) = ed.filter(|ed| ed.pending()) {
            return ed.rollback();
        }
        let me = self.weak();
        Conn::tx(&self.conn(), Cmd::Rollback, move |r| {
            if let Some(me) = me.upgrade() {
                me.out(&r.map(str::to_string).unwrap_or_else(|e| e));
            }
        });
    }

    pub fn cancel(&self) {
        self.running.set(false);
        self.set_busy(false);
        self.conn().borrow().cancel();
        self.status.set_label("Cancel requested");
    }

    pub fn clear(&self) {
        self.editor.clear_all();
    }

    pub fn history(&self) {
        let h = crate::app().history();
        if h.is_empty() {
            return crate::ui::info(&self.panel, "SQL History is empty: statements you run are listed here.");
        }
        let labels: Vec<String> = h.iter().rev().map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect()).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let d = SingleChoiceDialog::builder(&self.panel, "Select a statement to insert:", "SQL History", &refs).build();
        if d.show_modal() == ID_OK {
            let i = d.get_selection();
            if i >= 0 {
                let sql = &h[h.len() - 1 - i as usize];
                self.editor.insert_text(self.editor.get_current_pos(), &format!("{sql}\n"));
            }
        }
    }

    /// (fetched rows, all fetched, script output) for the self-test.
    pub fn probe(&self) -> (usize, bool, String) {
        let st = self.results.st.borrow();
        (st.rows.len(), st.done, self.script_out.get_value())
    }

    pub fn open_file(&self, p: PathBuf) -> std::io::Result<()> {
        let s = std::fs::read_to_string(&p)?;
        self.editor.set_text(&s);
        self.editor.empty_undo_buffer();
        *self.path.borrow_mut() = Some(p);
        Ok(())
    }

    pub fn save(&self, as_new: bool) {
        let path = self.path.borrow().clone();
        let path = match path {
            Some(p) if !as_new => p,
            _ => {
                let d = FileDialog::builder(&self.panel)
                    .with_message("Save SQL")
                    .with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
                    .with_wildcard("SQL files (*.sql)|*.sql|All files (*)|*")
                    .build();
                if d.show_modal() != ID_OK {
                    return;
                }
                match d.get_path() {
                    Some(p) => PathBuf::from(p),
                    None => return,
                }
            }
        };
        match std::fs::write(&path, self.editor.get_text()) {
            Ok(()) => {
                self.editor.set_save_point();
                self.status.set_label(&format!("Saved {}", path.display()));
                *self.path.borrow_mut() = Some(path);
            }
            Err(e) => crate::ui::error(&self.panel, &format!("Save failed: {e}")),
        }
    }
}

/// `[owner.]name` around byte offset `pos` (letters, digits, `_ $ #`, dots, double quotes).
fn ident_at(text: &str, pos: usize) -> String {
    let ok = |c: char| c.is_alphanumeric() || "_$#.\"".contains(c);
    let pos = pos.min(text.len());
    let Some((a, b)) = text.get(..pos).zip(text.get(pos..)) else { return String::new() };
    let start = a.char_indices().rev().take_while(|(_, c)| ok(*c)).last().map_or(pos, |(i, _)| i);
    let end = pos + b.char_indices().find(|(_, c)| !ok(*c)).map_or(b.len(), |(i, _)| i);
    text[start..end].trim_matches('.').to_string()
}

fn save_text(parent: &Panel, text: &str) {
    let d = FileDialog::builder(parent).with_message("Save").with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt).with_wildcard("Text (*.txt)|*.txt|All files (*)|*").build();
    if d.show_modal() == ID_OK {
        if let Some(p) = d.get_path() {
            if let Err(e) = std::fs::write(p, text) {
                crate::ui::error(parent, &e.to_string());
            }
        }
    }
}
