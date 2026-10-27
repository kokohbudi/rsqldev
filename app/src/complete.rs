//! Completion insight in the worksheet editor: Ctrl+Space offers keywords, tables and the
//! columns of the statement's tables; typing `x.` offers the columns of alias/table `x`
//! (or the tables of schema `x`). Names come from the connection's metadata cache.

use crate::conn::{Conn, ConnRef};
use rsqldev_core::complete::{context, expects_table, filter, insert_hint, match_case, resolve, stored, tables};
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};
use rsqldev_core::registry::Kind;
use rsqldev_core::script;
use wxdragon::prelude::*;

/// `explicit` = Ctrl+Space; otherwise only after a `.` qualifier.
pub fn show(editor: StyledTextCtrl, conn: &ConnRef, explicit: bool) {
    let (text, pos) = (editor.get_text(), editor.get_current_pos() as usize);
    let ctx = context(&text, pos);
    if !explicit && ctx.qualifier.is_none() {
        return;
    }
    let (db, schema) = {
        let c = conn.borrow();
        (c.kind, c.schema.clone())
    };
    let items = script::split(&text, db);
    let refs = tables(&script::statement_at(&items, pos).map(|i| i.text.clone()).unwrap_or_default());
    let target = |owner: Option<&str>, name: &str| (owner.map_or(schema.clone(), |o| stored(db, o)), stored(db, name));
    // Names go into metadata queries as string literals: skip ones that could break the literal.
    let safe = |s: &str| !s.contains(['\'', '\\']);
    match ctx.qualifier {
        Some(q) if !safe(&q) => {}
        Some(q) => match resolve(&refs, &q) {
            Some(r) => {
                let (o, n) = target(r.owner.as_deref(), &r.name);
                if !safe(&o) || !safe(&n) {
                    return;
                }
                Conn::columns(conn, &o, &n, move |cols| popup(editor, db, cols, false));
            }
            None => Conn::names(conn, &stored(db, &q), move |names| popup(editor, db, names, false)),
        },
        None => {
            let todo: Vec<(String, String)> = refs.iter().map(|r| target(r.owner.as_deref(), &r.name)).filter(|(o, n)| safe(o) && safe(n)).collect();
            let c = conn.clone();
            gather(conn, todo, vec![], move |cols| {
                Conn::names(&c, &schema, move |mut names| {
                    names.extend(cols);
                    popup(editor, db, names, true)
                })
            });
        }
    }
}

thread_local! {
    /// The INSERT column tip on screen (shown again only when its text changes).
    static TIP: RefCell<String> = const { RefCell::new(String::new()) };
    /// Table whose columns are being fetched for the tip (one query, not one per keystroke).
    static ASKED: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// Caret inside an INSERT's VALUES tuple: a call tip listing the columns with the one being
/// typed highlighted, like an IDE's parameter info. Columns the statement lists are shown as
/// written; without a list they come from the table (metadata cache).
/// ponytail: copies the editor text per caret move (no ranged GetText in wxDragon); fine for MB scripts.
pub fn insert_tip(editor: StyledTextCtrl, conn: &ConnRef) {
    let (text, pos) = (editor.get_text(), editor.get_current_pos() as usize);
    let hint = crate::editor::statement_start(&editor, &text, pos)
        .filter(|&s| text.as_bytes()[s..pos].windows(6).any(|w| w.eq_ignore_ascii_case(b"values")))
        .and_then(|s| insert_hint(&text[s..], pos - s));
    let Some(h) = hint else { return hide_tip(editor) };
    let arg = h.arg;
    match h.columns {
        Some(cols) => show_tip(editor, &cols, arg),
        None => {
            let (db, schema) = {
                let c = conn.borrow();
                (c.kind, c.schema.clone())
            };
            let (o, n) = (h.owner.as_deref().map_or(schema, |o| stored(db, o)), stored(db, &h.table));
            // Names go into a metadata query as string literals.
            if o.contains(['\'', '\\']) || n.contains(['\'', '\\']) {
                return hide_tip(editor);
            }
            let key = Some((o.clone(), n.clone()));
            if ASKED.with(|a| *a.borrow() == key) {
                return;
            }
            ASKED.with(|a| *a.borrow_mut() = key);
            Conn::columns(conn, &o, &n, move |cols| {
                ASKED.with(|a| *a.borrow_mut() = None);
                if editor.get_current_pos() as usize == pos && !cols.is_empty() {
                    show_tip(editor, &cols, arg);
                }
            });
        }
    }
}

/// The INSERT column tip on screen ("" = none); the self-test reads it.
pub fn tip_text() -> String {
    TIP.with(|t| t.borrow().clone())
}

fn show_tip(e: StyledTextCtrl, cols: &[String], arg: usize) {
    let mut tip = cols.join(", ");
    let at = cols.iter().take(arg).map(|c| c.len() + 2).sum::<usize>();
    let hl = cols.get(arg).map(|c| (at, at + c.len()));
    if hl.is_none() {
        tip.push_str("   (more values than columns)");
    }
    TIP.with(|t| {
        let mut t = t.borrow_mut();
        if !e.call_tip_active() || *t != tip {
            e.call_tip_show(e.get_current_pos(), &tip);
            *t = tip;
        }
    });
    let (a, b) = hl.unwrap_or((0, 0));
    e.call_tip_set_highlight(a as i32, b as i32);
}

fn hide_tip(e: StyledTextCtrl) {
    TIP.with(|t| {
        if !t.borrow().is_empty() {
            t.borrow_mut().clear();
            e.call_tip_cancel();
        }
    });
}

thread_local! {
    /// Bumped on every keystroke: a pending automatic popup only fires if it is still the latest.
    static TYPED: Cell<u64> = const { Cell::new(0) };
}

/// After a typing pause, pop the table list up by itself where a table name goes
/// (`FROM em|`, `JOIN |`, `INTO |`), like SQL Developer.
pub fn typed(editor: StyledTextCtrl, conn: &ConnRef, ch: char) {
    if !(ch.is_alphanumeric() || ch == '_' || ch == ' ') {
        return;
    }
    let me = TYPED.with(|t| {
        t.set(t.get() + 1);
        t.get()
    });
    let pos = editor.get_current_pos();
    let (conn, at) = (conn.clone(), Instant::now());
    crate::hub::every_tick(move || {
        if TYPED.with(Cell::get) != me || !editor.is_valid() {
            return false;
        }
        if at.elapsed() < Duration::from_millis(300) {
            return true;
        }
        if editor.get_current_pos() == pos && !editor.auto_comp_active() && expects_table(&editor.get_text(), pos as usize) {
            let (db, schema) = {
                let c = conn.borrow();
                (c.kind, c.schema.clone())
            };
            Conn::names(&conn, &schema, move |names| popup(editor, db, names, false));
        }
        false
    });
}

/// Columns of every (owner, table), one metadata query after another.
fn gather(conn: &ConnRef, mut todo: Vec<(String, String)>, mut acc: Vec<String>, done: impl FnOnce(Vec<String>) + 'static) {
    let Some((o, n)) = todo.pop() else { return done(acc) };
    let c = conn.clone();
    Conn::columns(conn, &o, &n, move |cols| {
        acc.extend(cols);
        gather(&c, todo, acc, done)
    });
}

/// Show the list for the word now at the cursor (the user may have typed on meanwhile).
fn popup(e: StyledTextCtrl, db: Kind, names: Vec<String>, keywords: bool) {
    if !e.is_valid() {
        return;
    }
    let ctx = context(&e.get_text(), e.get_current_pos() as usize);
    let mut list: Vec<String> = filter(&ctx.prefix, names).iter().map(|w| match_case(db, &ctx.prefix, w, false)).collect();
    if keywords {
        list.extend(filter(&ctx.prefix, crate::editor::KEYWORDS.iter().map(|k| k.to_string())).iter().map(|k| match_case(db, &ctx.prefix, k, true)));
        list.sort_by_key(|w| w.to_lowercase());
    }
    if list.is_empty() {
        return;
    }
    e.auto_comp_set_separator('\n' as i32);
    e.auto_comp_show(ctx.prefix.len() as i32, &list.join("\n"));
}
