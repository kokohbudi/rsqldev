//! Edit ▸ Copy (Ctrl+C / Cmd+C) outside the SQL editor. The menu accelerator takes the key
//! before grids, the tree or text panes see it, so each of them registers itself here when the
//! user selects in it, and the menu copies from the last one. wxDragon has no FindFocus, and a
//! grid's focus sits on an inner window that sends the Grid no focus events.

use std::cell::RefCell;
use std::rc::Rc;
use wxdragon::prelude::*;

/// Copies the target's selection; false when the target is gone.
type Target = Rc<dyn Fn() -> bool>;

thread_local! {
    static TARGET: RefCell<Option<Target>> = const { RefCell::new(None) };
}

/// Make `f` what Copy copies from, until something else is selected.
pub fn set(f: impl Fn() -> bool + 'static) {
    TARGET.with(|t| *t.borrow_mut() = Some(Rc::new(f)));
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
