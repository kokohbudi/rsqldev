//! Edit ▸ Copy (Ctrl+C / Cmd+C) outside the SQL editor. The menu accelerator takes the key
//! before grids, the tree or text panes see it, so each of them registers itself here when the
//! user selects in it, and the menu copies from the last one. wxDragon has no FindFocus, and a
//! grid's focus sits on an inner window that sends the Grid no focus events.
//! Only the user's own selecting counts: code that moves a cursor or rebuilds a tree runs inside
//! [`quietly`], so a refresh never steals the target.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wxdragon::prelude::*;

/// Copies the target's selection; false when the target is gone.
type Target = Rc<dyn Fn() -> bool>;

thread_local! {
    static TARGET: RefCell<Option<Target>> = const { RefCell::new(None) };
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Make `f` what Copy copies from, until something else is selected. Ignored inside [`quietly`].
pub fn set(f: impl Fn() -> bool + 'static) {
    if QUIET.with(Cell::get) {
        return;
    }
    TARGET.with(|t| *t.borrow_mut() = Some(Rc::new(f)));
}

/// Run `f` (code moving a cursor or a selection) without it changing the Copy target.
pub fn quietly<R>(f: impl FnOnce() -> R) -> R {
    // Restored on drop: a panic inside `f` (wxDragon catches callback panics) must not leave
    // QUIET set, which would silently disable Copy for the rest of the session.
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            QUIET.with(|q| q.set(self.0));
        }
    }
    let _restore = Restore(QUIET.with(|q| q.replace(true)));
    f()
}

/// Copy from the current target. False when there is none (or it was closed).
pub fn copy() -> bool {
    let t = TARGET.with(|t| t.borrow().clone());
    t.is_some_and(|f| f())
}

/// A text pane (read-only output, filter boxes): Copy takes its selected text.
pub fn text(t: TextCtrl) {
    t.on_set_focus(move |_| {
        set(move || {
            if !t.is_valid() {
                return false;
            }
            let s = t.get_string_selection();
            if !s.is_empty() {
                Clipboard::get().set_text(&s);
            }
            true
        })
    });
}

/// A code editor other than the worksheet's (object viewer source, value viewer).
pub fn editor(e: StyledTextCtrl) {
    e.on_set_focus(move |_| {
        set(move || {
            if !e.is_valid() {
                return false;
            }
            e.copy();
            true
        })
    });
}
