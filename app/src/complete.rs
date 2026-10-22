//! Completion insight in the worksheet editor: Ctrl+Space offers keywords, tables and the
//! columns of the statement's tables; typing `x.` offers the columns of alias/table `x`
//! (or the tables of schema `x`). Names come from the connection's metadata cache.

use crate::conn::{Conn, ConnRef};
use rsqldev_core::complete::{context, expects_table, filter, match_case, resolve, stored, tables};
use std::cell::Cell;
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
