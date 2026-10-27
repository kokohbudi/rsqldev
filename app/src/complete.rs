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
    /// Editor showing the tip.
    static EDITOR: Cell<Option<StyledTextCtrl>> = const { Cell::new(None) };
    /// Table whose columns are being fetched for the tip (one query, not one per keystroke).
    static ASKED: RefCell<Option<((String, String), Vec<Box<dyn FnOnce(Vec<String>)>>)>> = const { RefCell::new(None) };
}

/// Caret inside an INSERT's VALUES tuple or column list: a call tip listing the columns with
/// the one being typed highlighted, like an IDE's parameter info. Columns the statement lists are shown as
/// written; without a list they come from the table (metadata cache).
/// ponytail: copies the editor text per caret move (no ranged GetText in wxDragon); fine for MB scripts.
pub fn insert_tip(editor: StyledTextCtrl, conn: &ConnRef) {
    let Some((h, pos)) = hint_at(editor) else { return hide_tip(editor) };
    let arg = h.arg;
    match h.columns {
        Some(cols) => show_tip(editor, &cols, arg),
        None => table_columns(conn, &h, move |cols| {
            if editor.get_current_pos() as usize == pos && !cols.is_empty() {
                show_tip(editor, &cols, arg);
            }
        }),
    }
}

/// The INSERT under the caret, and the caret's byte offset.
fn hint_at(editor: StyledTextCtrl) -> Option<(rsqldev_core::complete::InsertHint, usize)> {
    let (text, pos) = (editor.get_text(), editor.get_current_pos() as usize);
    let s = crate::editor::statement_start(&editor, &text, pos).filter(|&s| text.as_bytes()[s..pos].windows(4).any(|w| w.eq_ignore_ascii_case(b"into")))?;
    insert_hint(&text[s..], pos - s).map(|h| (h, pos))
}

/// Columns of the INSERT's table (metadata cache); one query per table however many callers wait.
fn table_columns(conn: &ConnRef, h: &rsqldev_core::complete::InsertHint, cb: impl FnOnce(Vec<String>) + 'static) {
    let (db, schema) = {
        let c = conn.borrow();
        (c.kind, c.schema.clone())
    };
    let (o, n) = (h.owner.as_deref().map_or(schema, |o| stored(db, o)), stored(db, &h.table));
    // Names go into a metadata query as string literals.
    if o.contains(['\'', '\\']) || n.contains(['\'', '\\']) {
        return;
    }
    let key = (o.clone(), n.clone());
    let waiting = ASKED.with(|a| {
        let mut a = a.borrow_mut();
        match a.as_mut() {
            Some((k, w)) if *k == key => {
                w.push(Box::new(cb));
                true
            }
            _ => {
                *a = Some((key, vec![Box::new(cb)]));
                false
            }
        }
    });
    if waiting {
        return;
    }
    Conn::columns(conn, &o, &n, move |cols| {
        let waiters = ASKED.with(|a| a.borrow_mut().take()).map(|(_, w)| w).unwrap_or_default();
        for w in waiters {
            w(cols.clone());
        }
    });
}

/// The INSERT column tip on screen ("" = none); the self-test reads it.
pub fn tip_text() -> String {
    TIP.with(|t| t.borrow().clone())
}

fn show_tip(e: StyledTextCtrl, cols: &[String], arg: usize) {
    if e.auto_comp_active() {
        return e.call_tip_cancel();
    }
    let (tip, hl) = tip_layout(cols, arg, 100);
    TIP.with(|t| {
        let mut t = t.borrow_mut();
        if !e.call_tip_active() || *t != tip {
            e.call_tip_show(e.get_current_pos(), &tip);
            *t = tip;
        }
    });
    EDITOR.with(|x| x.set(Some(e)));
    let (a, b) = hl.unwrap_or((0, 0));
    e.call_tip_set_highlight(a as i32, b as i32);
}

/// Tip text: the active column first (`3/12  NAME`), then every column with the active one
/// in brackets, wrapped at `width` bytes; and the byte range to highlight. wxDragon cannot
/// colour or bold the highlight, so the brackets and the first line make it stand out.
fn tip_layout(cols: &[String], arg: usize, width: usize) -> (String, Option<(usize, usize)>) {
    let mut tip = match cols.get(arg) {
        Some(c) => format!("{}/{}  {c}", arg + 1, cols.len()),
        None => format!("more values than columns ({})", cols.len()),
    };
    let mut hl = None;
    let mut line = usize::MAX; // start of the current list line
    for (i, c) in cols.iter().enumerate() {
        let item = if i == arg { format!("[{c}]") } else { c.clone() };
        let sep = if i + 1 < cols.len() { "," } else { "" };
        if line == usize::MAX || tip.len() - line + 1 + item.len() + sep.len() > width {
            tip.push('\n');
            line = tip.len();
        } else {
            tip.push(' ');
        }
        if i == arg {
            hl = Some((tip.len(), tip.len() + item.len()));
        }
        tip.push_str(&item);
        tip.push_str(sep);
    }
    (tip, hl)
}

/// Close the tip (the editor lost focus or the app went to the background); the next caret
/// move shows it again.
pub fn cancel_tip() {
    if let Some(e) = EDITOR.with(Cell::get).filter(|e| e.is_valid()) {
        e.call_tip_cancel();
    }
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
    if !(ch.is_alphanumeric() || matches!(ch, '_' | ' ' | '(' | ',')) {
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
        if editor.get_current_pos() != pos || editor.auto_comp_active() {
            return false;
        }
        let (db, schema) = {
            let c = conn.borrow();
            (c.kind, c.schema.clone())
        };
        if expects_table(&editor.get_text(), pos as usize) {
            Conn::names(&conn, &schema, move |names| popup(editor, db, names, false));
        } else if let Some((h, _)) = hint_at(editor).filter(|(h, _)| h.written.is_some()) {
            // INSERT column list: the table's columns not written yet.
            let written = h.written.clone().unwrap_or_default();
            table_columns(&conn, &h, move |cols| {
                let left = cols.into_iter().filter(|c| !written.iter().any(|w| w.trim_matches(['"', '`', '[', ']']).eq_ignore_ascii_case(c))).collect();
                if editor.get_current_pos() == pos {
                    popup(editor, db, left, false);
                }
            });
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
    e.call_tip_cancel();
    e.auto_comp_set_separator('\n' as i32);
    e.auto_comp_show(ctx.prefix.len() as i32, &list.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::tip_layout;

    #[test]
    fn tip_names_the_active_column_and_marks_it_in_the_list() {
        let cols: Vec<String> = ["ID", "NAME", "CODE"].iter().map(|s| s.to_string()).collect();
        let (tip, hl) = tip_layout(&cols, 1, 100);
        assert_eq!(tip, "2/3  NAME\nID, [NAME], CODE");
        let (a, b) = hl.unwrap();
        assert_eq!(&tip[a..b], "[NAME]");
        let (tip, hl) = tip_layout(&cols, 3, 100);
        assert_eq!((tip.as_str(), hl), ("more values than columns (3)\nID, NAME, CODE", None));
    }

    #[test]
    fn tip_wraps_long_column_lists() {
        let cols: Vec<String> = (0..6).map(|i| format!("COLUMN_{i}")).collect();
        let (tip, hl) = tip_layout(&cols, 4, 30);
        assert_eq!(tip, "5/6  COLUMN_4\nCOLUMN_0, COLUMN_1, COLUMN_2,\nCOLUMN_3, [COLUMN_4], COLUMN_5");
        let (a, b) = hl.unwrap();
        assert_eq!(&tip[a..b], "[COLUMN_4]");
    }
}
