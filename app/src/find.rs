//! Find / Replace bar above the worksheet editor: Ctrl+F (find), Ctrl+R (replace),
//! F3 / Shift+F3 (next / previous), Esc closes.

use std::rc::Rc;
use wxdragon::prelude::*;

pub struct FindBar {
    pub panel: Panel,
    editor: StyledTextCtrl,
    find: TextCtrl,
    repl: TextCtrl,
    case: CheckBox,
    word: CheckBox,
    msg: StaticText,
}

const WXK_ESCAPE: i32 = 27;

impl FindBar {
    pub fn new(parent: &Panel, editor: StyledTextCtrl) -> Rc<FindBar> {
        let panel = Panel::builder(parent).build();
        let s = BoxSizer::builder(Orientation::Horizontal).build();
        let label = |t: &str| {
            let l = StaticText::builder(&panel).with_label(t).build();
            s.add(&l, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left | SizerFlag::Right, 4);
        };
        let field = || {
            let t = TextCtrl::builder(&panel).with_style(TextCtrlStyle::ProcessEnter).with_size(Size::new(200, -1)).build();
            crate::copy::text(t);
            s.add(&t, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 2);
            t
        };
        let button = |t: &str, tip: &str| {
            let b = Button::builder(&panel).with_label(t).build();
            if !tip.is_empty() {
                b.set_tooltip(tip);
            }
            s.add(&b, 0, SizerFlag::AlignCenterVertical | SizerFlag::All, 2);
            b
        };
        label("Find:");
        let find = field();
        let prev = button("◀", "Find Previous (Shift+F3)");
        let next = button("▶", "Find Next (F3)");
        let case = CheckBox::builder(&panel).with_label("Match case").build();
        s.add(&case, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 6);
        let word = CheckBox::builder(&panel).with_label("Whole word").build();
        s.add(&word, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left | SizerFlag::Right, 6);
        label("Replace:");
        let repl = field();
        let one = button("Replace", "");
        let all = button("Replace All", "");
        let msg = StaticText::builder(&panel).with_label("").build();
        msg.set_foreground_color(crate::theme::c(crate::theme::p().muted));
        s.add(&msg, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
        let close = button("✕", "Close (Esc)");
        panel.set_sizer(s, true);
        panel.show(false);
        let bar = Rc::new(FindBar { panel, editor, find, repl, case, word, msg });
        let acts: [(Button, fn(&FindBar)); 5] =
            [(prev, |b| b.next(true)), (next, |b| b.next(false)), (one, FindBar::replace_one), (all, FindBar::replace_all), (close, FindBar::close)];
        for (b, f) in acts {
            let w = Rc::downgrade(&bar);
            b.on_click(move |_| {
                if let Some(w) = w.upgrade() {
                    f(&w)
                }
            });
        }
        let enter: [(TextCtrl, fn(&FindBar)); 2] = [(bar.find, |b| b.next(false)), (bar.repl, FindBar::replace_one)];
        for (t, f) in enter {
            let w = Rc::downgrade(&bar);
            t.on_text_enter(move |_| {
                if let Some(w) = w.upgrade() {
                    f(&w)
                }
            });
            let w = Rc::downgrade(&bar);
            t.on_key_down(move |e| {
                if let WindowEventData::Keyboard(k) = &e {
                    if k.get_key_code() == Some(WXK_ESCAPE) {
                        if let Some(w) = w.upgrade() {
                            return w.close();
                        }
                    }
                }
                e.skip(true);
            });
        }
        bar
    }

    /// Show the bar; the editor selection (one line) becomes the search text.
    pub fn open(&self, replace: bool) {
        let sel = self.editor.get_selected_text();
        if !sel.is_empty() && !sel.contains('\n') {
            self.find.set_value(&sel);
        }
        if !self.panel.is_shown() {
            self.panel.show(true);
            if let Some(p) = self.panel.get_parent() {
                p.layout();
            }
        }
        let t = if replace { self.repl } else { self.find };
        t.set_focus();
        t.select_all();
    }

    pub fn close(&self) {
        self.panel.show(false);
        if let Some(p) = self.panel.get_parent() {
            p.layout();
        }
        self.editor.set_focus();
    }

    fn flags(&self) -> FindFlags {
        let mut f = FindFlags::None;
        if self.case.is_checked() {
            f |= FindFlags::MatchCase;
        }
        if self.word.is_checked() {
            f |= FindFlags::WholeWord;
        }
        f
    }

    /// Select the next (or previous) match, wrapping around the document.
    pub fn next(&self, back: bool) {
        let text = self.find.get_value();
        if text.is_empty() {
            return self.open(false);
        }
        let (e, f, len) = (self.editor, self.flags(), self.editor.get_length());
        let hit = if back {
            let from = e.get_selection_start();
            e.find_text(from, 0, &text, f).or_else(|| e.find_text(len, from, &text, f))
        } else {
            let from = e.get_selection_end();
            e.find_text(from, len, &text, f).or_else(|| e.find_text(0, from, &text, f))
        };
        match hit {
            Some(p) => {
                // Byte length: STC positions are UTF-8 byte offsets.
                e.set_selection(p, p + text.len() as i32);
                e.ensure_caret_visible();
                self.msg.set_label("");
            }
            None => self.msg.set_label("Not found"),
        }
    }

    /// Replace the selected match, then move to the next one.
    pub fn replace_one(&self) {
        let (e, text) = (self.editor, self.find.get_value());
        let start = e.get_selection_start();
        if !text.is_empty() && e.find_text(start, e.get_selection_end(), &text, self.flags()) == Some(start) && e.get_selection_end() - start == text.len() as i32 {
            e.replace_selection(&self.repl.get_value());
        }
        self.next(false);
    }

    /// Replace every match as a single undo step (one target replace of the whole document).
    pub fn replace_all(&self) {
        let (e, text, with) = (self.editor, self.find.get_value(), self.repl.get_value());
        if text.is_empty() {
            return;
        }
        let (f, len) = (self.flags(), e.get_length());
        let mut hits = vec![];
        let mut from = 0;
        while let Some(p) = e.find_text(from, len, &text, f) {
            hits.push(p as usize);
            from = p + text.len().max(1) as i32;
        }
        if hits.is_empty() {
            return self.msg.set_label("Not found");
        }
        let doc = e.get_text();
        let mut out = String::with_capacity(doc.len());
        let mut last = 0;
        for &p in &hits {
            out.push_str(&doc[last..p]);
            out.push_str(&with);
            last = p + text.len();
        }
        out.push_str(&doc[last..]);
        e.set_target_start(0);
        e.set_target_end(len);
        e.replace_target(&out);
        self.msg.set_label(&format!("{} replaced", hits.len()));
    }
}
