//! Result grid shared by worksheet results, object viewer tabs and the data editor.
//! Fetches the next batch only when the last loaded row scrolls into view (SQL Developer style).

use crate::conn::Rows;
use rsqldev_core::registry::Kind;
use rsqldev_core::{Cell, Column, Event, export};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::TryRecvError;
use wxdragon::prelude::*;

thread_local! {
    static NULL_TEXT: std::cell::RefCell<String> = std::cell::RefCell::new("(null)".into());
}

/// Text shown for NULL in result grids (Preferences ▸ Display NULL as).
pub fn null_text() -> String {
    NULL_TEXT.with(|t| t.borrow().clone())
}

pub fn set_null_text(s: &str) {
    NULL_TEXT.with(|t| *t.borrow_mut() = s.to_string());
}

#[derive(Clone, Default)]
pub struct Row {
    pub cur: Vec<Cell>,
    /// Values as fetched, copied only when the row is first edited (saves a copy of every
    /// untouched row). None = unedited, or a row inserted in the grid (`new`).
    pub orig: Option<Vec<Cell>>,
    /// Inserted in the grid (not fetched).
    pub new: bool,
    pub deleted: bool,
    /// Insert / delete already posted ("Post edits on row change"), waiting for Commit: the
    /// row has no locator yet, so it stays read-only until the data is refreshed.
    pub posted: bool,
    /// Files loaded into binary cells (column → bytes), posted as bind values on commit.
    pub blobs: HashMap<usize, Rc<Vec<u8>>>,
}

impl Row {
    fn fetched(cur: Vec<Cell>) -> Row {
        Row { cur, ..Row::default() }
    }

    /// Call before changing `cur`: keeps the fetched values for the WHERE clause.
    fn edit(&mut self) {
        if !self.new && self.orig.is_none() {
            self.orig = Some(self.cur.clone());
        }
    }

    /// Values as fetched (the key for UPDATE / DELETE).
    pub fn original(&self) -> &[Cell] {
        self.orig.as_deref().unwrap_or(&self.cur)
    }

    fn changed(&self, c: usize) -> bool {
        self.new || self.blobs.contains_key(&c) || self.orig.as_ref().is_some_and(|o| o[c] != self.cur[c])
    }

    pub fn dirty(&self) -> bool {
        self.deleted || self.new || !self.blobs.is_empty() || self.orig.as_ref().is_some_and(|o| *o != self.cur)
    }
}

/// A column filter: text the value contains (lower case), or exact values (None = NULL).
#[derive(Clone)]
enum ColFilter {
    Contains(String),
    In(Vec<Option<String>>),
}

impl ColFilter {
    fn ok(&self, v: Option<&str>) -> bool {
        match self {
            ColFilter::Contains(t) => v.is_some_and(|v| rsqldev_core::lob::display(v).to_lowercase().contains(t)),
            ColFilter::In(set) => set.iter().any(|s| s.as_deref() == v),
        }
    }
}

/// Filter Column…: a "contains" text, or ticked distinct values of the fetched rows (as SQL
/// Developer). Outer None = cancelled, inner None = no filter on this column.
fn filter_dialog(parent: &Grid, name: &str, values: &[Option<String>], more: bool, cur: Option<&ColFilter>) -> Option<Option<ColFilter>> {
    let dlg = Dialog::builder(parent, &format!("Filter Column {name}")).with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(420, 480).build();
    let text = TextCtrl::builder(&dlg).build();
    let label = |v: &Option<String>| v.as_deref().map_or_else(null_text, |s| rsqldev_core::lob::display(s).chars().take(200).collect());
    let list = CheckListBox::builder(&dlg).with_choices(values.iter().map(label).collect()).build();
    match cur {
        Some(ColFilter::Contains(t)) => text.set_value(t),
        Some(ColFilter::In(set)) => {
            for (i, v) in values.iter().enumerate() {
                list.check(i as u32, set.contains(v));
            }
        }
        None => {}
    }
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add(&StaticText::builder(&dlg).with_label("Rows whose value contains:").build(), 0, SizerFlag::Left | SizerFlag::Top | SizerFlag::Right, 10);
    s.add(&text, 0, SizerFlag::Expand | SizerFlag::All, 8);
    let hint = if more { "or exactly one of (first 1000 distinct values of the fetched rows):" } else { "or exactly one of (values of the fetched rows):" };
    s.add(&StaticText::builder(&dlg).with_label(hint).build(), 0, SizerFlag::Left | SizerFlag::Right, 10);
    s.add(&list, 1, SizerFlag::Expand | SizerFlag::All, 8);
    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    let clear = Button::builder(&dlg).with_label("Clear").build();
    clear.on_click(move |_| dlg.end_modal(ID_NO));
    bs.add(&clear, 0, SizerFlag::All, 4);
    bs.add_stretch_spacer(1);
    for (l, id) in [("Cancel", ID_CANCEL), ("OK", ID_OK)] {
        let b = Button::builder(&dlg).with_label(l).with_id(id).build();
        b.on_click(move |_| dlg.end_modal(id));
        bs.add(&b, 0, SizerFlag::All, 4);
    }
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer(s, true);
    text.set_focus();
    let code = dlg.show_modal();
    let ticked: Vec<Option<String>> = (0..values.len()).filter(|i| list.is_checked(*i as u32)).map(|i| values[i].clone()).collect();
    let t = text.get_value().trim().to_lowercase();
    dlg.destroy();
    match code {
        ID_OK if !ticked.is_empty() => Some(Some(ColFilter::In(ticked))),
        ID_OK if !t.is_empty() => Some(Some(ColFilter::Contains(t))),
        ID_OK | ID_NO => Some(None),
        _ => None,
    }
}

#[derive(Default)]
pub struct State {
    pub cols: Vec<Column>,
    pub rows: Vec<Row>,
    rx: Option<Rows>,
    pub done: bool,
    /// Leading row-locator columns kept hidden (ROWID, ctid, rowid).
    pub hidden: usize,
    pub editable: bool,
    fetch_all: bool,
    after_all: Vec<Box<dyn FnOnce()>>,
    sort: Option<(usize, bool)>,
    pub on_change: Option<Rc<dyn Fn(&State)>>,
    /// Export target table name and dialect for INSERT export.
    pub db: Option<Kind>,
    /// Column filters: rows a filter rejects are hidden.
    filters: Vec<(usize, ColFilter)>,
    /// Last "Find in Results" text, for Find Next.
    find: String,
    /// Count every row of the result on the server (set by the worksheet / data tab).
    pub count: Option<Rc<dyn Fn()>>,
    /// Sort… dialog re-querying with ORDER BY (set by the data tab).
    pub sort_dialog: Option<Rc<dyn Fn()>>,
    /// The cursor left this row ("Post edits on row change").
    pub on_row_leave: Option<Rc<dyn Fn(usize)>>,
    /// Continue the result from row n with a new statement ([`crate::conn::Conn::resume`]):
    /// set, an idle grid closes its cursor after [`IDLE_RELEASE`].
    pub resume: Option<Rc<dyn Fn(usize) -> Rows>>,
    /// The last batch arrived (idle measure).
    last_batch: Option<std::time::Instant>,
    /// Cursor closed while idle; the next fetch resumes.
    released: bool,
    /// Resumed statement: its column header is already shown.
    resumed: bool,
    /// Some row was edited since the result loaded (pending counts are worth computing).
    pub edited: bool,
    /// Rows the column filters let through (kept up to date as rows arrive).
    shown: usize,
}

/// Idle time before a resumable result closes its cursor (PostgreSQL autocommit transaction).
// ponytail: fixed 30 s; servers with a shorter idle_in_transaction_session_timeout still cut it.
const IDLE_RELEASE: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone)]
pub struct ResultGrid {
    pub grid: Grid,
    pub st: Rc<RefCell<State>>,
}

const ID_COPY: i32 = 7001;
const ID_COPY_HDR: i32 = 7002;
const ID_CSV: i32 = 7003;
const ID_JSON: i32 = 7004;
const ID_INSERT: i32 = 7005;
const ID_FETCH_ALL: i32 = 7006;
const ID_SAVE_VALUE: i32 = 7007;
const ID_LOAD_VALUE: i32 = 7008;
const ID_VIEW_VALUE: i32 = 7009;
const ID_XLSX: i32 = 7010;
const FETCH_ALL_STEP: usize = 100_000;
const ID_FIND: i32 = 7011;
const ID_FIND_NEXT: i32 = 7012;
const ID_COPY_CSV: i32 = 7013;
const ID_COPY_MD: i32 = 7014;
const ID_COPY_IN: i32 = 7015;
const ID_COPY_INSERT: i32 = 7016;
const ID_COUNT: i32 = 7017;
const ID_FILTER: i32 = 7018;
const ID_CLEAR_FILTERS: i32 = 7019;
const ID_INSERT_ROW: i32 = 7021;
const ID_DUP_ROW: i32 = 7022;
const ID_DELETE_ROWS: i32 = 7023;
const ID_SET_NULL: i32 = 7024;
const ID_REVERT: i32 = 7025;
const ID_PASTE: i32 = 7026;
const ID_RECORD: i32 = 7027;
const ID_SORT_DLG: i32 = 7028;
const ID_COPY_HEADS: i32 = 7029;
const ID_EXPORT_DLG: i32 = 7030;

impl ResultGrid {
    pub fn new(parent: &dyn WxWidget) -> ResultGrid {
        let grid = Grid::builder(parent).build();
        grid.create_grid(0, 0, GridSelectionMode::Cells);
        grid.use_native_col_header(true);
        grid.set_default_cell_overflow(false);
        grid.set_row_label_size(48);
        grid.enable_editing(false);
        let p = crate::theme::p();
        let c = crate::theme::c;
        grid.set_grid_line_colour(&c(p.grid_line));
        grid.set_label_background_colour(&c(p.label_bg));
        grid.set_label_text_colour(&c(p.text));
        grid.set_default_cell_background_colour(&c(p.bg));
        grid.set_default_cell_text_colour(&c(p.text));
        grid.set_selection_background(&c(p.select_bg));
        grid.set_selection_foreground(&c(p.select_fg));
        // From the font, not fixed pixels: Windows at 125-150% scaling has taller text.
        grid.set_default_row_size((grid.get_char_height() + 6).max(20), true);
        let g = ResultGrid { grid, st: Rc::default() };
        let weak = Rc::downgrade(&g.st);
        let me = g.clone_weak();
        crate::hub::every_tick(move || {
            if weak.upgrade().is_none() || !grid.is_valid() {
                return false;
            }
            me.pump();
            true
        });
        let me = g.clone();
        grid.on_label_left_click(move |e| {
            let col = e.get_col();
            if e.get_row() < 0 && col >= 0 {
                me.sort_by(col as usize);
            }
        });
        let me = g.clone();
        grid.on_cell_changed(move |e| me.cell_edited(e.get_row(), e.get_col()));
        // Double-click: cells that cannot be typed into (BLOB, LOB, long or multi-line text,
        // read-only results) open the value editor; the rest edit in place.
        let me = g.clone();
        grid.on_cell_left_dclick(move |e| {
            let (r, c) = (e.get_row(), e.get_col());
            if r < 0 || c < 0 {
                return;
            }
            if me.needs_popup(r as usize, c as usize) {
                me.view_value(r as usize, c as usize);
            } else {
                // The handler takes the event (no skip in wxDragon): start the in-place editor.
                me.grid.set_grid_cursor(r, c);
                me.grid.enable_cell_edit_control(true);
            }
        });
        let me = g.clone();
        grid.on_label_right_click(move |e| {
            if e.get_row() < 0 && e.get_col() >= 0 {
                me.column_menu(e.get_col() as usize);
            }
        });
        let me = g.clone();
        grid.on_cell_right_click(move |e| me.context_menu(Some(e.get_position()), (e.get_row(), e.get_col())));
        // Ctrl+C / Cmd+C copies the selected cells (the menu accelerator never reaches the grid).
        // Registered on the user's clicks, keys and drags; cursor moves from code run quietly.
        let st = Rc::downgrade(&g.st);
        let target = move || {
            let st = st.clone();
            crate::copy::set(move || match st.upgrade() {
                Some(st) if grid.is_valid() => {
                    ResultGrid { grid, st }.menu(ID_COPY);
                    true
                }
                _ => false,
            })
        };
        let t2 = target.clone();
        let last_row = Rc::new(std::cell::Cell::new(-1));
        let st2 = Rc::downgrade(&g.st);
        grid.on_select_cell(move |e| {
            target();
            let prev = last_row.replace(e.get_row());
            if prev >= 0 && prev != e.get_row() {
                let hook = st2.upgrade().and_then(|st| st.borrow().on_row_leave.clone());
                if let Some(h) = hook {
                    h(prev as usize)
                }
            }
            // The cursor cell only gets a border: select it too so it is filled like a
            // selection. Next tick, once wx has finished the click or key, and only when
            // nothing else is selected (shift / drag selections stay as they are).
            let (r, c) = (e.get_row(), e.get_col());
            crate::hub::every_tick(move || {
                if grid.is_valid() && !grid.is_selection() && (grid.get_grid_cursor_row(), grid.get_grid_cursor_col()) == (r, c) {
                    crate::copy::quietly(|| grid.select_block(r, c, r, c, false));
                }
                false
            });
        });
        grid.on_range_selected(move |_| t2());
        g
    }

    /// Same grid, weak state handle for the ticker (avoids keeping the state alive forever).
    fn clone_weak(&self) -> WeakGrid {
        WeakGrid { grid: self.grid, st: Rc::downgrade(&self.st) }
    }

    pub fn reset(&self) {
        let mut st = self.st.borrow_mut();
        if let Some(rx) = st.rx.take() {
            rx.borrow_mut().take();
        }
        // Spilled LOB files of the old result are no longer reachable.
        for row in &st.rows {
            for v in row.cur.iter().chain(row.orig.iter().flatten()).flatten() {
                rsqldev_core::lob::release(v);
            }
        }
        let keep = (st.on_change.take(), st.db, st.count.take(), std::mem::take(&mut st.find), st.sort_dialog.take(), st.on_row_leave.take());
        *st = State { on_change: keep.0, db: keep.1, count: keep.2, find: keep.3, sort_dialog: keep.4, on_row_leave: keep.5, ..State::default() };
        drop(st);
        self.redraw_all();
    }

    /// Start showing an open cursor.
    pub fn load(&self, rows: Rows, hidden: usize, editable: bool) {
        self.reset();
        let mut st = self.st.borrow_mut();
        st.rx = Some(rows);
        st.hidden = hidden;
        st.editable = editable;
        self.grid.enable_editing(editable);
    }

    /// Turn editing on (or off) for the rows already loaded and those still to come.
    pub fn set_editable(&self, on: bool) {
        self.st.borrow_mut().editable = on;
        let g = self.grid;
        g.enable_editing(on);
        let st = self.st.borrow();
        g.begin_batch();
        Self::col_formats(g, &st);
        // Only the cells that differ from a read-only result: numeric alignment, BLOB lock.
        for ri in 0..st.rows.len() {
            for ci in 0..st.cols.len() {
                Self::edit_attrs(g, &st, ri, ci);
            }
        }
        g.end_batch();
    }

    /// Read-only results right-align numeric columns with a column format (no per-cell
    /// attribute); editable ones keep the text editor and align per cell instead.
    fn col_formats(g: Grid, st: &State) {
        for (i, _) in st.cols.iter().enumerate().filter(|(_, c)| c.numeric) {
            if st.editable {
                g.set_col_format_custom(i as i32, "string");
            } else {
                g.set_col_format_number(i as i32);
            }
        }
    }

    /// Per-cell attributes an editable grid needs: right alignment of numbers, read-only
    /// BLOB cells and posted rows. Read-only grids need none.
    fn edit_attrs(g: Grid, st: &State, ri: usize, ci: usize) {
        if !st.editable {
            return;
        }
        let (r, c) = (ri as i32, ci as i32);
        if st.cols.get(ci).is_some_and(|c| c.numeric) {
            g.set_cell_alignment(r, c, 0x0200 /* wxALIGN_RIGHT */, 0x0800 /* wxALIGN_CENTER_VERTICAL */);
        }
        if !Self::cell_editable(st, ri, ci) {
            g.set_read_only(r, c, true);
        }
    }

    /// Show a complete result.
    pub fn set(&self, cols: Vec<Column>, rows: Vec<Vec<Cell>>) {
        self.reset();
        {
            let mut st = self.st.borrow_mut();
            st.cols = cols;
            st.rows = rows.into_iter().map(Row::fetched).collect();
            st.done = true;
        }
        self.redraw_all();
    }

    /// Fetch every remaining row, then run `f`.
    pub fn fetch_all(&self, f: impl FnOnce() + 'static) {
        let mut st = self.st.borrow_mut();
        if st.done || st.rx.is_none() {
            drop(st);
            return f();
        }
        st.fetch_all = true;
        st.after_all.push(Box::new(f));
    }

    fn pump(&self) {
        let (rx, hidden, fetch_all, n) = {
            let mut st = self.st.borrow_mut();
            let idle = st.last_batch.is_some_and(|t| t.elapsed() > IDLE_RELEASE);
            if st.resume.is_some() && st.rx.is_some() && !st.done && !st.fetch_all && idle {
                // Close the cursor (its transaction ends) until the grid wants more.
                if let Some(rx) = st.rx.take() {
                    rx.borrow_mut().take();
                }
                st.released = true;
                drop(st);
                crate::app().status(0, "Fetching paused while idle (the database transaction is released); scroll to continue");
                return;
            }
            match &st.rx {
                Some(rx) => (rx.clone(), st.hidden, st.fetch_all, st.rows.len()),
                None if st.released && !st.done => {
                    let n = st.rows.len();
                    let wants = st.fetch_all || self.grid.is_visible(n as i32 - 1, st.hidden as i32, false);
                    let Some(f) = st.resume.clone().filter(|_| wants) else { return };
                    st.released = false;
                    st.resumed = true;
                    st.last_batch = Some(std::time::Instant::now());
                    drop(st);
                    let rows = f(n);
                    self.st.borrow_mut().rx = Some(rows);
                    return;
                }
                None => return,
            }
        };
        let wants = fetch_all || n == 0 || self.grid.is_visible(n as i32 - 1, hidden as i32, false);
        if !wants {
            return;
        }
        let recv = || {
            let r = rx.borrow();
            match r.as_ref() {
                Some(rx) => rx.try_recv(),
                None => Err(TryRecvError::Disconnected),
            }
        };
        let mut ev = recv();
        // Fetch All: take every batch that is ready for up to ~15 ms, then draw them at once
        // (one batch per 25 ms tick capped it at 4,000 rows a second).
        if fetch_all {
            if let Ok(Event::Rows(first)) = ev {
                let mut batch = first;
                let t = std::time::Instant::now();
                let mut tail = Err(TryRecvError::Empty);
                while !batch.is_empty() && t.elapsed() < std::time::Duration::from_millis(15) && (n + batch.len()) / FETCH_ALL_STEP == n / FETCH_ALL_STEP {
                    match recv() {
                        Ok(Event::Rows(more)) if !more.is_empty() => batch.extend(more),
                        other => {
                            tail = other;
                            break;
                        }
                    }
                }
                // A last (empty) batch or a closed cursor seen while draining ends the fetch.
                let ended = matches!(tail, Ok(Event::Rows(_)) | Err(TryRecvError::Disconnected));
                ev = Ok(Event::Rows(batch));
                if ended {
                    if let Ok(Event::Rows(b)) = ev {
                        self.add_rows(n, b);
                    }
                    return self.finish();
                }
            }
        }
        match ev {
            Ok(Event::Columns(_)) if self.st.borrow().resumed => {}
            Ok(Event::Columns(c)) => {
                self.st.borrow_mut().cols = c;
                self.redraw_all();
            }
            Ok(Event::Rows(batch)) => {
                let start = n;
                let last = batch.is_empty();
                self.add_rows(n, batch);
                if last {
                    self.finish();
                } else if fetch_all && start / FETCH_ALL_STEP != self.st.borrow().rows.len() / FETCH_ALL_STEP {
                    // Every 100,000 rows: confirm before holding even more in memory.
                    self.st.borrow_mut().fetch_all = false;
                    let n = self.st.borrow().rows.len();
                    let more = crate::ui::confirm(
                        &self.grid,
                        &format!("{n} rows are loaded in the grid. Keep fetching?\n\nTo save a large result, use Export: it streams to the file without loading the grid."),
                    );
                    let mut st = self.st.borrow_mut();
                    if more {
                        st.fetch_all = true;
                    } else {
                        st.after_all.clear();
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.finish(),
        }
    }

    /// Fetched rows into the state and the grid.
    fn add_rows(&self, start: usize, batch: Vec<Vec<Cell>>) {
        {
            let mut st = self.st.borrow_mut();
            st.rows.extend(batch.into_iter().map(Row::fetched));
            st.last_batch = Some(std::time::Instant::now());
        }
        self.append_rows(start);
    }

    fn finish(&self) {
        let after = {
            let mut st = self.st.borrow_mut();
            st.done = true;
            st.rx = None;
            std::mem::take(&mut st.after_all)
        };
        self.changed();
        for f in after {
            f();
        }
    }

    fn changed(&self) {
        let cb = self.st.borrow().on_change.clone();
        if let Some(cb) = cb {
            cb(&self.st.borrow());
        }
    }

    pub fn redraw_all(&self) {
        crate::copy::quietly(|| self.redraw());
    }

    fn redraw(&self) {
        let g = self.grid;
        g.begin_batch();
        if g.get_number_rows() > 0 {
            g.delete_rows(0, g.get_number_rows(), true);
        }
        if g.get_number_cols() > 0 {
            g.delete_cols(0, g.get_number_cols(), true);
        }
        let (ncols, hidden) = {
            let st = self.st.borrow();
            g.append_cols(st.cols.len() as i32, true);
            for (i, c) in st.cols.iter().enumerate() {
                g.set_col_label_value(i as i32, &c.name);
            }
            (st.cols.len(), st.hidden)
        };
        for i in 0..hidden.min(ncols) {
            g.hide_col(i as i32);
        }
        Self::col_formats(g, &self.st.borrow());
        g.end_batch();
        self.append_rows(0);
        let st = self.st.borrow();
        if let Some((c, asc)) = st.sort {
            g.set_sorting_column(c as i32, asc);
        }
    }

    fn append_rows(&self, start: usize) {
        let g = self.grid;
        let st = self.st.borrow();
        let n = st.rows.len();
        if start >= n {
            return;
        }
        g.begin_batch();
        g.append_rows((n - start) as i32, true);
        for ri in start..n {
            Self::fill_row(g, &st, ri, false);
        }
        if start == 0 {
            g.auto_size_columns(false);
            for (i, col) in st.cols.iter().enumerate() {
                let c = i as i32;
                // Native headers are not measured by auto-size: fit the column name too.
                let head = g.get_text_extent(&col.name).width + 28;
                g.set_col_size(c, g.get_col_size(c).max(head).min(400));
            }
            // Auto-size re-shows hidden columns: hide the row locator (ROWID/ctid/rowid) again.
            for c in 0..st.hidden.min(st.cols.len()) as i32 {
                g.hide_col(c);
            }
        }
        g.end_batch();
        drop(st);
        self.changed();
        if !self.st.borrow().filters.is_empty() {
            self.apply_filters(start);
        }
    }

    /// Values and the few per-cell attributes of one grid row. A row new to the grid has no
    /// attributes yet: only NULL cells get a colour (a wx attribute per cell costs memory).
    /// `reset` (reverted / re-posted rows) puts edited colours back to normal first.
    fn fill_row(g: Grid, st: &State, ri: usize, reset: bool) {
        let p = crate::theme::p();
        // (null) placeholder, ≥ 4.5:1 in both palettes (see theme tests).
        let (grey, text, bg) = (crate::theme::c(p.null), crate::theme::c(p.text), crate::theme::c(p.bg));
        let row = &st.rows[ri];
        let null = null_text();
        for (ci, v) in row.cur.iter().enumerate() {
            let (r, c) = (ri as i32, ci as i32);
            match v {
                Some(s) => {
                    g.set_cell_value(r, c, &rsqldev_core::lob::grid_text(st.cols.get(ci).map_or("", |c| c.type_name.as_str()), s));
                    if reset {
                        g.set_cell_text_colour(r, c, &text);
                    }
                }
                None => {
                    g.set_cell_value(r, c, &null);
                    g.set_cell_text_colour(r, c, &grey);
                }
            }
            if reset {
                g.set_cell_background_colour(r, c, &bg);
            }
            // BLOB cells change through Load Value, not by typing over "(BLOB)".
            Self::edit_attrs(g, st, ri, ci);
        }
        Self::mark_row(g, st, ri);
    }

    /// Typing allowed: editable result, not a BLOB column, not a row already posted.
    fn cell_editable(st: &State, ri: usize, ci: usize) -> bool {
        st.editable && !st.rows[ri].posted && !st.cols.get(ci).is_some_and(|c| rsqldev_core::lob::is_blob_type(&c.type_name))
    }

    fn mark_row(g: Grid, st: &State, ri: usize) {
        Self::label_row(g, st, ri);
        let row = &st.rows[ri];
        let r = ri as i32;
        let edited = crate::theme::c(crate::theme::p().edited_bg);
        let removed = crate::theme::c(crate::theme::p().deleted_bg);
        for ci in 0..row.cur.len() {
            let changed = row.changed(ci);
            if row.deleted {
                g.set_cell_background_colour(r, ci as i32, &removed);
            } else if changed && st.editable {
                g.set_cell_background_colour(r, ci as i32, &edited);
            }
        }
    }

    /// Row number and marker (✓ posted, ✕ deleted, + new, * changed). wx row labels do not
    /// move when rows are inserted or deleted: rows after one need relabelling (labels only).
    fn label_row(g: Grid, st: &State, ri: usize) {
        let row = &st.rows[ri];
        let label = if row.posted {
            format!("{} ✓", ri + 1)
        } else if row.deleted {
            format!("{} ✕", ri + 1)
        } else if row.new {
            format!("{} +", ri + 1)
        } else if row.dirty() {
            format!("{} *", ri + 1)
        } else {
            (ri + 1).to_string()
        };
        g.set_row_label_value(ri as i32, &label);
    }

    fn cell_edited(&self, r: i32, c: i32) {
        let v = self.grid.get_cell_value(r, c);
        let mut st = self.st.borrow_mut();
        st.edited = true;
        let Some(row) = st.rows.get_mut(r as usize) else { return };
        row.edit();
        row.cur[c as usize] = if v.is_empty() || *v == null_text() { None } else { Some(v) };
        row.blobs.remove(&(c as usize));
        Self::mark_row(self.grid, &st, r as usize);
        drop(st);
        self.grid.force_refresh();
        self.changed();
    }

    /// Insert Row: an empty new row after the cursor row (at the end when there is none).
    pub fn insert_row(&self) {
        let w = self.st.borrow().cols.len();
        self.insert_new(self.after_cursor(), Row { cur: vec![None; w], new: true, ..Row::default() });
    }

    /// Duplicate Row: a new row after the cursor row with its values (row locator left empty;
    /// staged files copied too).
    pub fn duplicate_row(&self) {
        let r = self.grid.get_grid_cursor_row();
        let row = {
            let st = self.st.borrow();
            let Some(src) = st.rows.get(r.max(0) as usize).filter(|_| r >= 0) else { return };
            let mut cur = src.cur.clone();
            cur.iter_mut().take(st.hidden).for_each(|v| *v = None);
            Row { cur, new: true, blobs: src.blobs.clone(), ..Row::default() }
        };
        self.insert_new(r as usize + 1, row);
    }

    fn after_cursor(&self) -> usize {
        let (r, n) = (self.grid.get_grid_cursor_row(), self.st.borrow().rows.len());
        if r >= 0 && (r as usize) < n { r as usize + 1 } else { n }
    }

    fn insert_new(&self, at: usize, row: Row) {
        if !self.st.borrow().editable {
            return;
        }
        self.st.borrow_mut().edited = true;
        let n = self.st.borrow().rows.len();
        if at >= n {
            self.st.borrow_mut().rows.push(row);
            self.append_rows(n);
        } else {
            self.st.borrow_mut().rows.insert(at, row);
            let g = self.grid;
            g.begin_batch();
            g.insert_rows(at as i32, 1, true);
            let st = self.st.borrow();
            Self::fill_row(g, &st, at, false);
            for ri in at + 1..st.rows.len() {
                Self::label_row(g, &st, ri);
            }
            g.end_batch();
            drop(st);
            self.changed();
        }
        let hidden = self.st.borrow().hidden as i32;
        self.move_cursor(at as i32, hidden, true);
    }

    /// Rows whose DML went to the database ahead of Commit: updates become the new baseline;
    /// inserts and deletes stay read-only (✓) until the data is refreshed.
    pub fn mark_posted(&self, rows: &[usize]) {
        let g = self.grid;
        let mut st = self.st.borrow_mut();
        for &r in rows {
            let Some(row) = st.rows.get_mut(r) else { continue };
            if row.new || row.deleted {
                row.posted = true;
            } else {
                row.orig = None;
                row.blobs.clear();
            }
            Self::fill_row(g, &st, r, true);
        }
        drop(st);
        g.force_refresh();
        self.changed();
    }

    /// Rows touched by the selection, else the cursor row (Delete / Revert).
    fn picked_rows(&self) -> Vec<usize> {
        let cursor = (self.grid.get_grid_cursor_row(), self.grid.get_grid_cursor_col());
        let n = self.st.borrow().rows.len();
        rows_to_mark(self.grid.get_selected_rows(), &self.grid.get_selected_blocks(), cursor).into_iter().filter(|r| *r >= 0 && (*r as usize) < n).map(|r| r as usize).collect()
    }

    /// Selected cells (blocks), else the cursor cell; row locator columns left out.
    fn picked_cells(&self) -> Vec<(usize, usize)> {
        let (hidden, n, w) = {
            let st = self.st.borrow();
            (st.hidden as i32, st.rows.len() as i32, st.cols.len() as i32)
        };
        let mut out = vec![];
        let blocks = self.grid.get_selected_blocks();
        let cursor = (self.grid.get_grid_cursor_row(), self.grid.get_grid_cursor_col());
        if blocks.is_empty() || (blocks.len() == 1 && stale(&blocks[0], cursor)) {
            out.push(cursor);
        } else {
            for b in &blocks {
                for r in b.top_row..=b.bottom_row {
                    out.extend((b.left_col..=b.right_col).map(|c| (r, c)));
                }
            }
        }
        out.into_iter().filter(|&(r, c)| r >= 0 && r < n && c >= hidden && c < w).map(|(r, c)| (r as usize, c as usize)).collect()
    }

    /// Set to NULL: every selected cell of rows not yet posted.
    pub fn set_null(&self) {
        if !self.st.borrow().editable {
            return;
        }
        self.grid.begin_batch();
        for (r, c) in self.picked_cells() {
            if self.st.borrow().rows[r].posted {
                continue;
            }
            if let Some(row) = self.st.borrow_mut().rows.get_mut(r) {
                row.blobs.remove(&c);
            }
            self.put(r, c, None);
        }
        self.grid.end_batch();
        self.grid.force_refresh();
        self.changed();
    }

    /// Revert Row: forget a row's pending change (new rows go away).
    pub fn revert_rows(&self) {
        let rows = self.picked_rows();
        let g = self.grid;
        g.begin_batch();
        for r in rows.into_iter().rev() {
            let mut st = self.st.borrow_mut();
            let row = &mut st.rows[r];
            if row.posted {
                continue;
            }
            if row.new {
                st.rows.remove(r);
                g.delete_rows(r as i32, 1, true);
                for ri in r..st.rows.len() {
                    Self::label_row(g, &st, ri);
                }
                continue;
            }
            if let Some(o) = row.orig.take() {
                row.cur = o;
            }
            row.blobs.clear();
            row.deleted = false;
            Self::fill_row(g, &st, r, true);
        }
        g.end_batch();
        g.force_refresh();
        self.changed();
    }

    /// Paste tab-separated text (a copied block, Excel) from the cursor cell; rows past the
    /// end become new rows. Returns false when this grid cannot take it.
    pub fn paste(&self) -> bool {
        if !self.st.borrow().editable {
            return false;
        }
        let Some(text) = Clipboard::get().get_text() else { return true };
        let lines = parse_tsv(&text);
        let (r0, c0) = (self.grid.get_grid_cursor_row(), self.grid.get_grid_cursor_col());
        let (hidden, w) = {
            let st = self.st.borrow();
            (st.hidden, st.cols.len())
        };
        let (r0, c0) = (r0.max(0) as usize, (c0.max(0) as usize).max(hidden));
        let (mut done, mut skipped) = (0, vec![]);
        // Rows past the end, added at once.
        let n = self.st.borrow().rows.len();
        let need = (r0 + lines.len()).saturating_sub(n);
        if need > 0 {
            let mut st = self.st.borrow_mut();
            st.edited = true;
            st.rows.extend((0..need).map(|_| Row { cur: vec![None; w], new: true, ..Row::default() }));
            drop(st);
            self.append_rows(n);
        }
        self.grid.begin_batch();
        for (i, line) in lines.iter().enumerate() {
            let r = r0 + i;
            for (j, v) in line.iter().enumerate() {
                let c = c0 + j;
                let ok = c < w && Self::cell_editable(&self.st.borrow(), r, c);
                if !ok {
                    if let Some(col) = self.st.borrow().cols.get(c) {
                        skipped.push(format!("{} ({})", col.name, col.type_name));
                    }
                    continue;
                }
                self.st.borrow_mut().rows[r].blobs.remove(&c);
                self.put(r, c, if v.is_empty() || *v == null_text() { None } else { Some(v.clone()) });
                done += 1;
            }
        }
        self.grid.end_batch();
        self.grid.force_refresh();
        self.changed();
        skipped.dedup();
        let msg = if skipped.is_empty() { format!("Pasted {done} value(s)") } else { format!("Pasted {done} value(s). Cannot paste value for column: {}", skipped.join(", ")) };
        crate::app().status(0, &msg);
        true
    }

    /// Move the cursor from code: the old selection goes (Delete Row reads it) and the Copy
    /// target stays where the user put it. `show` scrolls the cell into view.
    pub fn move_cursor(&self, r: i32, c: i32, show: bool) {
        crate::copy::quietly(|| {
            self.grid.clear_selection();
            if show {
                self.grid.go_to_cell(r, c);
            } else {
                self.grid.set_grid_cursor(r, c);
            }
        });
    }

    /// Add rows as pending inserts (import); Commit posts them.
    pub fn append_new_rows(&self, rows: Vec<Vec<Cell>>) {
        let start = {
            let mut st = self.st.borrow_mut();
            st.edited = true;
            let start = st.rows.len();
            st.rows.extend(rows.into_iter().map(|cur| Row { cur, new: true, ..Row::default() }));
            start
        };
        self.append_rows(start);
        let hidden = self.st.borrow().hidden as i32;
        self.move_cursor(start as i32, hidden, true);
    }

    /// Marks every row touched by the selection (whole rows, and cell blocks dragged or
    /// shift / ctrl-clicked across several rows), or the cursor row when nothing is selected.
    pub fn delete_selected_rows(&self) {
        if !self.st.borrow().editable {
            return;
        }
        let rows = self.picked_rows();
        let mut st = self.st.borrow_mut();
        st.edited = true;
        for r in rows {
            if st.rows[r].posted {
                continue;
            }
            st.rows[r].deleted = true;
            Self::mark_row(self.grid, &st, r);
        }
        drop(st);
        self.grid.force_refresh();
        self.changed();
    }

    pub(crate) fn sort_by(&self, col: usize) {
        {
            let mut st = self.st.borrow_mut();
            let asc = !matches!(st.sort, Some((c, true)) if c == col);
            let numeric = st.cols.get(col).is_some_and(|c| c.numeric);
            sort_rows(&mut st.rows, col, asc, numeric);
            st.sort = Some((col, asc));
        }
        // ponytail: sorts fetched rows only; re-query with ORDER BY if users expect server-side sort.
        self.redraw_all();
    }

    /// Visible columns and rows of the current selection (or everything).
    fn selection(&self, all: bool) -> (Vec<Column>, Vec<Vec<Cell>>) {
        let st = self.st.borrow();
        let vis: Vec<usize> = (st.hidden..st.cols.len()).collect();
        let pick = |cols: &[usize], rows: &mut dyn Iterator<Item = usize>| {
            let c = cols.iter().map(|&i| st.cols[i].clone()).collect();
            let r = rows.filter_map(|r| st.rows.get(r)).map(|row| cols.iter().map(|&i| row.cur[i].clone()).collect()).collect();
            (c, r)
        };
        if !all {
            if let Some(b) = self.grid.get_selected_blocks().first() {
                let cols: Vec<usize> = (b.left_col.max(st.hidden as i32)..=b.right_col).map(|c| c as usize).collect();
                return pick(&cols, &mut (b.top_row..=b.bottom_row).map(|r| r as usize));
            }
            let (r, c) = (self.grid.get_grid_cursor_row(), self.grid.get_grid_cursor_col());
            if r >= 0 && c >= 0 {
                return pick(&[c as usize], &mut std::iter::once(r as usize));
            }
        }
        pick(&vis, &mut (0..st.rows.len()))
    }

    fn context_menu(&self, pos: Option<Point>, (r, c): (i32, i32)) {
        let (editable, hidden) = {
            let st = self.st.borrow();
            (st.editable, st.hidden as i32)
        };
        let has_count = self.st.borrow().count.is_some();
        let has_sort = self.st.borrow().sort_dialog.is_some();
        let mut b = Menu::builder();
        if editable {
            b = b
                .append_item(ID_INSERT_ROW, "Insert Row\tCtrl+I", "New row after this one; Commit to post")
                .append_item(ID_DUP_ROW, "Duplicate Row", "New row with this row's values")
                .append_item(ID_DELETE_ROWS, "Delete Selected Row(s)\tCtrl+D", "")
                .append_item(ID_SET_NULL, "Set to NULL", "Selected cells become NULL")
                .append_item(ID_REVERT, "Revert Row", "Forget this row's pending change")
                .append_item(ID_PASTE, "Paste", "Paste copied cells (tab-separated) from this cell")
                .append_separator();
        }
        if r >= 0 {
            b = b.append_item(ID_RECORD, "Single Record View…", "One row as Name / Value, with previous / next");
        }
        if has_sort {
            b = b.append_item(ID_SORT_DLG, "Sort…", "Sort the data by several columns on the server");
        }
        b = b
            .append_separator()
            .append_item(ID_COPY, "Copy", "Copy selected cells")
            .append_item(ID_COPY_HDR, "Copy with Headers", "Copy selected cells with column names")
            .append_item(ID_COPY_HEADS, "Copy Selected Column Header(s)", "The names of the selected columns, tab separated")
            .append_item(ID_COPY_CSV, "Copy as CSV", "")
            .append_item(ID_COPY_MD, "Copy as Markdown", "")
            .append_item(ID_COPY_IN, "Copy as IN List", "Distinct values of the first selected column: (a, b, c)")
            .append_item(ID_COPY_INSERT, "Copy as INSERT", "")
            .append_separator()
            .append_item(ID_FIND, "Find in Results…", "Search the fetched rows")
            .append_item(ID_FIND_NEXT, "Find Next", "")
            .append_separator();
        if has_count {
            b = b.append_item(ID_COUNT, "Count Rows", "Count every row of the result on the server, without fetching them");
        }
        if r >= 0 && c >= hidden {
            let label = if editable { "Edit Value…\tAlt+Enter" } else { "View Value…\tAlt+Enter" };
            b = b.append_item(ID_VIEW_VALUE, label, "The whole value in an editor: long text, dates (date picker), BLOB as image / text / hex");
            b = b.append_item(ID_SAVE_VALUE, "Save Value to File…", "Write this cell (BLOB bytes or text) to a file");
            if editable {
                b = b.append_item(ID_LOAD_VALUE, "Load Value from File…", "Replace this cell with a file's contents; Commit to post");
            }
            b = b.append_separator();
        }
        let mut m = b
            .append_item(ID_FETCH_ALL, "Fetch All Rows", "Fetch every remaining row")
            .append_item(ID_EXPORT_DLG, "Export…", "Format, header, delimiter, selected rows only, to a file or the clipboard")
            .append_item(ID_CSV, "Export CSV…", "")
            .append_item(ID_XLSX, "Export Excel…", "")
            .append_item(ID_JSON, "Export JSON…", "")
            .append_item(ID_INSERT, "Export INSERT…", "")
            .build();
        let me = self.clone();
        m.on_selected(move |e| match e.get_id() {
            ID_SAVE_VALUE => me.save_value(r as usize, c as usize),
            ID_VIEW_VALUE => me.view_value(r as usize, c as usize),
            ID_LOAD_VALUE => me.load_value(r as usize, c as usize),
            ID_RECORD => crate::record::show(&me, r as usize),
            id => me.menu(id),
        });
        let pos = pos.map(|p| self.grid.client_to_screen(p));
        self.grid.popup_menu(&mut m, pos);
        m.destroy_menu();
    }

    /// Save a cell to a file: staged upload bytes, decoded hex for binary columns, else text.
    fn save_value(&self, r: usize, c: usize) {
        let (bytes, name) = {
            let st = self.st.borrow();
            let Some(row) = st.rows.get(r) else { return };
            let bytes = match (row.blobs.get(&c), &row.cur[c]) {
                (Some(b), _) => b.to_vec(),
                (None, Some(s)) => rsqldev_core::cell_bytes(&st.cols[c], s),
                (None, None) => return crate::ui::error(&self.grid, "The value is NULL."),
            };
            // Default to a sibling cell that looks like a file name (FILE_NAME columns), like the
            // SQL Developer habit of storing it next to the BLOB.
            let name = row.cur.iter().enumerate().filter(|(i, _)| *i != c).filter_map(|(_, v)| v.as_deref()).find(|v| {
                v.len() < 200 && !v.contains(['/', '\\', '\n']) && v.rsplit_once('.').is_some_and(|(a, e)| !a.is_empty() && (1..=5).contains(&e.len()) && e.chars().all(char::is_alphanumeric))
            });
            (bytes, name.map_or_else(|| format!("{}.bin", st.cols[c].name), str::to_owned))
        };
        let d = FileDialog::builder(&self.grid)
            .with_message("Save Value to File")
            .with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
            .with_default_file(&name)
            .with_wildcard("All files (*)|*")
            .build();
        if d.show_modal() != ID_OK {
            return;
        }
        let Some(path) = d.get_path() else { return };
        if let Err(e) = std::fs::write(&path, bytes) {
            crate::ui::error(&self.grid, &format!("Save failed: {e}"));
        }
    }

    /// Load a file into a cell: binary columns stage the bytes for a bound upload on commit;
    /// other columns take the file as UTF-8 text.
    fn load_value(&self, r: usize, c: usize) {
        let d = FileDialog::builder(&self.grid).with_message("Load Value from File").with_style(FileDialogStyle::Open).with_wildcard("All files (*)|*").build();
        if d.show_modal() != ID_OK {
            return;
        }
        let Some(path) = d.get_path() else { return };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => return crate::ui::error(&self.grid, &format!("Load failed: {e}")),
        };
        let mut st = self.st.borrow_mut();
        let binary = st.cols.get(c).is_some_and(|col| rsqldev_core::is_binary(&col.type_name));
        let Some(row) = st.rows.get_mut(r) else { return };
        let text = if binary {
            row.edit();
            let t = format!("(BLOB {})", rsqldev_core::lob::size(bytes.len()));
            row.blobs.insert(c, Rc::new(bytes));
            t
        } else {
            match String::from_utf8(bytes) {
                Ok(t) => {
                    row.blobs.remove(&c);
                    t
                }
                Err(_) => return crate::ui::error(&self.grid, "This column holds text and the file is not UTF-8 text."),
            }
        };
        drop(st);
        self.set_cell(r, c, Some(text));
    }

    /// Write a value into a cell from code (dialogs, file loads) and mark the row dirty.
    pub(crate) fn set_cell(&self, r: usize, c: usize, v: Cell) {
        self.put(r, c, v);
        self.grid.force_refresh();
        self.changed();
    }

    /// [`Self::set_cell`] without the repaint and change notice (bulk callers do those once).
    fn put(&self, r: usize, c: usize, v: Cell) {
        let mut st = self.st.borrow_mut();
        st.edited = true;
        let Some(row) = st.rows.get_mut(r) else { return };
        row.edit();
        row.cur[c] = v.clone();
        let p = crate::theme::p();
        self.grid.set_cell_value(r as i32, c as i32, v.as_deref().unwrap_or(&null_text()));
        self.grid.set_cell_text_colour(r as i32, c as i32, &crate::theme::c(if v.is_some() { p.text } else { p.null }));
        Self::mark_row(self.grid, &st, r);
    }

    fn view_value(&self, r: usize, c: usize) {
        let (col, text, bytes, editable) = {
            let st = self.st.borrow();
            let (Some(row), Some(col)) = (st.rows.get(r), st.cols.get(c)) else { return };
            let binary = rsqldev_core::is_binary(&col.type_name);
            let bytes = row.blobs.get(&c).map(|b| b.to_vec()).or_else(|| row.cur[c].as_deref().filter(|_| binary).map(|s| rsqldev_core::cell_bytes(col, s)));
            let text = row.cur[c].as_deref().map(|s| rsqldev_core::lob::expand(s).into_owned());
            (col.clone(), text, bytes, st.editable && c >= st.hidden)
        };
        match crate::value::show(&self.grid, &col, text.as_deref(), bytes.as_deref(), editable) {
            crate::value::Outcome::Text(v) => {
                if v != text {
                    if let Some(row) = self.st.borrow_mut().rows.get_mut(r) {
                        row.blobs.remove(&c);
                    }
                    self.set_cell(r, c, v);
                }
            }
            crate::value::Outcome::Save => self.save_value(r, c),
            crate::value::Outcome::Load => self.load_value(r, c),
            crate::value::Outcome::Keep => {}
        }
    }

    /// Edit Value (pencil, Alt+Enter): the value editor for the cursor cell.
    pub fn edit_value(&self) {
        let (r, c) = (self.grid.get_grid_cursor_row(), self.grid.get_grid_cursor_col());
        let (n, hidden) = {
            let st = self.st.borrow();
            (st.rows.len() as i32, st.hidden as i32)
        };
        if r >= 0 && r < n && c >= hidden {
            self.grid.save_edit_control_value();
            self.view_value(r as usize, c as usize);
        }
    }

    /// The cell is better edited in the popup than in place.
    fn needs_popup(&self, r: usize, c: usize) -> bool {
        let st = self.st.borrow();
        let (Some(row), Some(col)) = (st.rows.get(r), st.cols.get(c)) else { return false };
        let long = row.cur[c].as_deref().is_some_and(|v| v.contains('\n') || v.len() > 200 || rsqldev_core::lob::display(v) != v);
        !Self::cell_editable(&st, r, c) || rsqldev_core::is_binary(&col.type_name) || long
    }

    /// Find in Results: case-insensitive, over the fetched (and not filtered-out) rows, from the
    /// cell after the cursor, wrapping. `ask` prompts for the text first.
    pub fn find(&self, ask: bool) {
        let mut text = self.st.borrow().find.clone();
        if ask || text.is_empty() {
            match crate::ui::ask(&self.grid, "Find in fetched rows:", "Find in Results", &text) {
                Some(t) if !t.is_empty() => text = t,
                _ => return,
            }
            self.st.borrow_mut().find = text.clone();
        }
        let needle = text.to_lowercase();
        let hit = {
            let st = self.st.borrow();
            let (nc, nr) = (st.cols.len(), st.rows.len());
            if nc <= st.hidden || nr == 0 {
                return;
            }
            let width = nc - st.hidden;
            let total = nr * width;
            let (r0, c0) = (self.grid.get_grid_cursor_row().max(0) as usize, (self.grid.get_grid_cursor_col().max(st.hidden as i32) as usize).saturating_sub(st.hidden));
            let start = (r0 * width + c0 + 1) % total;
            (0..total).map(|k| (start + k) % total).map(|k| (k / width, k % width + st.hidden)).find(|&(r, c)| {
                self.grid.is_row_shown(r as i32) && st.rows[r].cur[c].as_deref().is_some_and(|v| rsqldev_core::lob::display(v).to_lowercase().contains(&needle))
            })
        };
        match hit {
            Some((r, c)) => {
                let (r, c) = (r as i32, c as i32);
                self.move_cursor(r, c, true);
                crate::copy::quietly(|| self.grid.select_block(r, c, r, c, false));
            }
            None => crate::app().status(0, &format!("'{text}' not found in the fetched rows")),
        }
    }

    /// Header right-click: filter this column, clear filters, sort.
    fn column_menu(&self, col: usize) {
        const SORT: i32 = 7020;
        if col < self.st.borrow().hidden {
            return;
        }
        let has = !self.st.borrow().filters.is_empty();
        let mut b = Menu::builder().append_item(ID_FILTER, "Filter Column…", "Show only rows whose value contains a text or is one of the ticked values").append_item(SORT, "Sort", "");
        if has {
            b = b.append_item(ID_CLEAR_FILTERS, "Remove All Filters", "");
        }
        let mut m = b.build();
        let me = self.clone();
        m.on_selected(move |e| match e.get_id() {
            ID_FILTER => {
                let (name, cur, values, more) = {
                    let st = me.st.borrow();
                    let mut seen = std::collections::BTreeSet::new();
                    for r in st.rows.iter().filter(|r| !r.new) {
                        seen.insert(r.cur[col].clone());
                        if seen.len() > 1000 {
                            break;
                        }
                    }
                    let more = seen.len() > 1000;
                    (st.cols[col].name.clone(), st.filters.iter().find(|f| f.0 == col).map(|f| f.1.clone()), seen.into_iter().take(1000).collect::<Vec<_>>(), more)
                };
                if let Some(f) = filter_dialog(&me.grid, &name, &values, more, cur.as_ref()) {
                    let mut st = me.st.borrow_mut();
                    st.filters.retain(|f| f.0 != col);
                    st.filters.extend(f.map(|f| (col, f)));
                    drop(st);
                    me.apply_filters(0);
                }
            }
            ID_CLEAR_FILTERS => {
                me.st.borrow_mut().filters.clear();
                me.apply_filters(0);
            }
            SORT => me.sort_by(col),
            _ => {}
        });
        self.grid.popup_menu(&mut m, None);
        m.destroy_menu();
    }

    /// Hide fetched rows from `from` on that fail a column filter (client side, fetched rows
    /// only); 0 = a filter changed, all rows again.
    fn apply_filters(&self, from: usize) {
        let mut st = self.st.borrow_mut();
        let g = self.grid;
        g.begin_batch();
        let mut shown = if from == 0 { 0 } else { st.shown };
        for (r, row) in st.rows.iter().enumerate().skip(from) {
            let ok = row.new || st.filters.iter().all(|(c, f)| f.ok(row.cur[*c].as_deref()));
            if ok {
                g.show_row(r as i32);
                shown += 1;
            } else {
                g.hide_row(r as i32);
            }
        }
        g.end_batch();
        st.shown = shown;
        let msg = if st.filters.is_empty() { String::new() } else { format!("Filter: {shown} of {} fetched rows shown", st.rows.len()) };
        drop(st);
        crate::app().status(0, &msg);
    }

    /// Stream the result to a file: rows already in the grid first, then the rest of the open
    /// cursor straight into the file (never into the grid), so memory stays flat.
    /// `only`: these rows (selected rows) instead of the whole result.
    fn export_to(&self, fmt: export::Format, path: std::path::PathBuf, opts: &export::Options, only: Option<Vec<Vec<Cell>>>) {
        if let Some(rows) = only {
            let (cols, _) = self.selection(true);
            let db = self.st.borrow().db.unwrap_or(Kind::Oracle);
            let res = export::Export::create_with(&path, fmt, &cols, db, opts).and_then(|mut x| {
                x.rows(&rows)?;
                x.finish()
            });
            return match res {
                Ok(n) => crate::app().status(0, &format!("Exported {n} rows to {}", path.display())),
                Err(e) => {
                    let _ = std::fs::remove_file(&path);
                    crate::ui::error(&self.grid, &format!("Export failed: {e:#}"))
                }
            };
        }
        let (vis, cols, hidden, db, rx) = {
            let mut st = self.st.borrow_mut();
            let vis: Vec<usize> = (st.hidden..st.cols.len()).collect();
            let cols: Vec<Column> = vis.iter().map(|&i| st.cols[i].clone()).collect();
            // The rest of the cursor goes to the file: the grid keeps what it has.
            let rx = st.rx.take();
            st.done = true;
            (vis, cols, st.hidden, st.db.unwrap_or(Kind::Oracle), rx)
        };
        let fail = move |grid: &Grid, path: &std::path::Path, e: rsqldev_core::Error| {
            let _ = std::fs::remove_file(path);
            crate::ui::error(grid, &format!("Export failed: {e:#}"));
        };
        // Rows already in the grid, written 1,000 at a time (no copy of the whole result).
        let written = export::Export::create_with(&path, fmt, &cols, db, opts).and_then(|mut x| {
            let st = self.st.borrow();
            let mut buf: Vec<Vec<Cell>> = Vec::with_capacity(1000);
            for r in st.rows.iter().filter(|r| !r.deleted) {
                buf.push(vis.iter().map(|&i| r.cur[i].clone()).collect());
                if buf.len() == 1000 {
                    x.rows(&buf)?;
                    buf.clear();
                }
            }
            x.rows(&buf)?;
            Ok(x)
        });
        let ex = match written {
            Ok(x) => x,
            Err(e) => return fail(&self.grid, &path, e),
        };
        let grid = self.grid;
        let done = move |ex: export::Export, path: &std::path::Path| match ex.finish() {
            Ok(n) => crate::app().status(0, &format!("Exported {n} rows to {}", path.display())),
            Err(e) => fail(&grid, path, e),
        };
        let Some(rx) = rx else { return done(ex, &path) };
        let mut ex = Some(ex);
        crate::hub::every_tick(move || {
            let r = rx.borrow();
            // Bounded per tick so the UI stays responsive while a big result drains.
            for _ in 0..20 {
                let ev = match r.as_ref() {
                    Some(rx) => rx.try_recv(),
                    None => Err(TryRecvError::Disconnected),
                };
                match ev {
                    Ok(Event::Rows(b)) if !b.is_empty() => {
                        let b: Vec<Vec<Cell>> = b.into_iter().map(|row| row.into_iter().skip(hidden).collect()).collect();
                        let x = ex.as_mut().unwrap();
                        let res = x.rows(&b);
                        // Streamed rows never reach the grid: free their spilled LOB files now.
                        b.iter().flatten().flatten().for_each(|v| rsqldev_core::lob::release(v));
                        if let Err(e) = res {
                            fail(&grid, &path, e);
                            return false;
                        }
                        crate::app().status(0, &format!("Exporting… {} rows", x.count()));
                    }
                    Ok(Event::Columns(_)) => {}
                    Err(TryRecvError::Empty) => return true,
                    Ok(Event::Rows(_)) | Err(TryRecvError::Disconnected) => {
                        done(ex.take().unwrap(), &path);
                        return false;
                    }
                }
            }
            true
        });
    }

    /// Export… button: pick a format, then save.
    /// Rows touched by the selection (every visible column), else the cursor row.
    fn selected_rows(&self) -> Vec<Vec<Cell>> {
        let st = self.st.borrow();
        let mut idx: Vec<usize> = self.grid.get_selected_blocks().iter().flat_map(|b| b.top_row.max(0) as usize..=b.bottom_row.max(0) as usize).collect();
        if idx.is_empty() && self.grid.get_grid_cursor_row() >= 0 {
            idx.push(self.grid.get_grid_cursor_row() as usize);
        }
        idx.sort_unstable();
        idx.dedup();
        idx.iter().filter_map(|r| st.rows.get(*r)).filter(|r| !r.deleted).map(|r| r.cur[st.hidden..].to_vec()).collect()
    }

    /// Export…: format, header, delimiter, INSERT table name, all or selected rows, to a file
    /// (the whole result streams) or the clipboard (fetched rows).
    pub fn export_dialog(&self) {
        use export::Format::*;
        const FORMATS: [(&str, export::Format, &str); 6] = [("CSV / delimited", Csv, "csv"), ("Excel (xlsx)", Xlsx, "xlsx"), ("JSON", Json, "json"), ("INSERT statements", Inserts, "sql"), ("HTML", Html, "html"), ("XML", Xml, "xml")];
        const DELIMS: [(&str, char); 4] = [(", (comma)", ','), ("; (semicolon)", ';'), ("Tab", '\t'), ("| (pipe)", '|')];
        let dlg = Dialog::builder(&self.grid, "Export").with_style(DialogStyle::DefaultDialogStyle).with_size(420, 380).build();
        let fmt = Choice::builder(&dlg).with_choices(FORMATS.iter().map(|f| f.0.to_string()).collect()).build();
        fmt.set_selection(0);
        let header = CheckBox::builder(&dlg).with_label("Header row (column names)").build();
        header.set_value(true);
        let delim = Choice::builder(&dlg).with_choices(DELIMS.iter().map(|d| d.0.to_string()).collect()).build();
        delim.set_selection(0);
        let table = TextCtrl::builder(&dlg).with_value("EXPORT_TABLE").build();
        let r_all = RadioButton::builder(&dlg).with_label("All rows").first_in_group().build();
        let r_sel = RadioButton::builder(&dlg).with_label("Selected rows").build();
        let o_file = RadioButton::builder(&dlg).with_label("File").first_in_group().build();
        let o_clip = RadioButton::builder(&dlg).with_label("Clipboard (fetched rows)").build();
        let g = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
        g.add_growable_col(1, 1);
        crate::ui::row(&dlg, &g, "Format", &fmt);
        crate::ui::row(&dlg, &g, "", &header);
        crate::ui::row(&dlg, &g, "Delimiter", &delim);
        crate::ui::row(&dlg, &g, "INSERT table", &table);
        crate::ui::row(&dlg, &g, "Rows", &r_all);
        crate::ui::row(&dlg, &g, "", &r_sel);
        crate::ui::row(&dlg, &g, "Save to", &o_file);
        crate::ui::row(&dlg, &g, "", &o_clip);
        let bs = BoxSizer::builder(Orientation::Horizontal).build();
        bs.add_stretch_spacer(1);
        for (l, id) in [("Cancel", ID_CANCEL), ("OK", ID_OK)] {
            let b = Button::builder(&dlg).with_label(l).with_id(id).build();
            b.on_click(move |_| dlg.end_modal(id));
            bs.add(&b, 0, SizerFlag::All, 4);
        }
        let s = BoxSizer::builder(Orientation::Vertical).build();
        s.add_sizer(&g, 1, SizerFlag::Expand | SizerFlag::All, 12);
        s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
        dlg.set_sizer(s, true);
        let ok = dlg.show_modal() == ID_OK;
        let (f, ext) = FORMATS.get(fmt.get_selection().unwrap_or(0) as usize).map(|f| (f.1, f.2)).unwrap_or((Csv, "csv"));
        let opts = export::Options { header: header.get_value(), delimiter: DELIMS.get(delim.get_selection().unwrap_or(0) as usize).map_or(',', |d| d.1), table: table.get_value().trim().to_string() };
        let (selected, clip) = (r_sel.get_value(), o_clip.get_value());
        dlg.destroy();
        if !ok {
            return;
        }
        let only = selected.then(|| self.selected_rows());
        if clip {
            if f == Xlsx {
                return crate::ui::info(&self.grid, "Excel exports go to a file.");
            }
            let (cols, all) = self.selection(true);
            let rows = only.unwrap_or(all);
            let db = self.st.borrow().db.unwrap_or(Kind::Oracle);
            Clipboard::get().set_text(&export::text_with(f, db, &opts, &cols, &rows));
            return crate::app().status(0, &format!("Copied {} rows to the clipboard", rows.len()));
        }
        let d = FileDialog::builder(&self.grid)
            .with_message("Export")
            .with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
            .with_default_file(&format!("export.{ext}"))
            .with_wildcard(&format!("{} (*.{ext})|*.{ext}|All files (*)|*", FORMATS.iter().find(|x| x.2 == ext).map_or("", |x| x.0)))
            .build();
        if d.show_modal() == ID_OK {
            if let Some(p) = d.get_path() {
                self.export_to(f, std::path::PathBuf::from(p), &opts, only);
            }
        }
    }

    fn menu(&self, id: i32) {
        match id {
            ID_COPY_CSV | ID_COPY_MD | ID_COPY_IN | ID_COPY_INSERT => {
                let (c, r) = self.selection(false);
                let db = self.st.borrow().db.unwrap_or(Kind::Oracle);
                let text = match id {
                    ID_COPY_CSV => export::csv(&c, &r),
                    ID_COPY_MD => export::markdown(&c, &r),
                    ID_COPY_IN => match c.first() {
                        Some(col) => export::in_list(db, col, &r),
                        None => return,
                    },
                    _ => export::inserts(db, "TABLE_NAME", &c, &r),
                };
                Clipboard::get().set_text(&text);
            }
            ID_FIND => self.find(true),
            ID_FIND_NEXT => self.find(false),
            ID_INSERT_ROW => self.insert_row(),
            ID_DUP_ROW => self.duplicate_row(),
            ID_DELETE_ROWS => self.delete_selected_rows(),
            ID_SET_NULL => self.set_null(),
            ID_REVERT => self.revert_rows(),
            ID_PASTE => {
                self.paste();
            }
            ID_SORT_DLG => {
                let f = self.st.borrow().sort_dialog.clone();
                if let Some(f) = f {
                    f()
                }
            }
            ID_COUNT => {
                let f = self.st.borrow().count.clone();
                if let Some(f) = f {
                    f()
                }
            }
            ID_COPY_HEADS => {
                let (c, _) = self.selection(false);
                Clipboard::get().set_text(&c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join("\t"));
            }
            ID_EXPORT_DLG => self.export_dialog(),
            ID_COPY | ID_COPY_HDR => {
                let (c, r) = self.selection(false);
                let t = export::tsv((id == ID_COPY_HDR).then_some(&c[..]), &r);
                crate::app().remember_clip(&t);
                Clipboard::get().set_text(&t);
            }
            ID_FETCH_ALL => self.fetch_all(|| {}),
            ID_CSV | ID_XLSX | ID_JSON | ID_INSERT => {
                let (ext, wild) = match id {
                    ID_CSV => ("csv", "CSV (*.csv)|*.csv"),
                    ID_XLSX => ("xlsx", "Excel (*.xlsx)|*.xlsx"),
                    ID_JSON => ("json", "JSON (*.json)|*.json"),
                    _ => ("sql", "SQL (*.sql)|*.sql"),
                };
                let d = FileDialog::builder(&self.grid)
                    .with_message("Export")
                    .with_style(FileDialogStyle::Save | FileDialogStyle::OverwritePrompt)
                    .with_default_file(&format!("export.{ext}"))
                    .with_wildcard(wild)
                    .build();
                if d.show_modal() != ID_OK {
                    return;
                }
                let Some(path) = d.get_path() else { return };
                let fmt = match id {
                    ID_CSV => export::Format::Csv,
                    ID_XLSX => export::Format::Xlsx,
                    ID_JSON => export::Format::Json,
                    _ => export::Format::Inserts,
                };
                self.export_to(fmt, std::path::PathBuf::from(path), &export::Options::default(), None);
            }
            _ => {}
        }
    }
}

struct WeakGrid {
    grid: Grid,
    st: std::rc::Weak<RefCell<State>>,
}

impl WeakGrid {
    fn pump(&self) {
        if let Some(st) = self.st.upgrade() {
            ResultGrid { grid: self.grid, st }.pump();
        }
    }
}

/// Rows Delete Row marks: whole selected rows plus every row of the selected blocks, else the
/// cursor row. A lone 1x1 block away from the cursor is a stale auto-selection (the cursor was
/// moved by code since): it is ignored, so the cursor row is marked instead.
/// A lone 1x1 block away from the cursor: left over after code moved the cursor.
fn stale(b: &GridBlockCoords, cursor: (i32, i32)) -> bool {
    b.top_row == b.bottom_row && b.left_col == b.right_col && (b.top_row, b.left_col) != cursor
}

fn rows_to_mark(selected: Vec<i32>, blocks: &[GridBlockCoords], cursor: (i32, i32)) -> Vec<i32> {
    let mut rows = selected;
    if !(rows.is_empty() && blocks.len() == 1 && stale(&blocks[0], cursor)) {
        for b in blocks {
            rows.extend(b.top_row..=b.bottom_row);
        }
    }
    rows.sort_unstable();
    rows.dedup();
    if rows.is_empty() {
        rows.push(cursor.0);
    }
    rows
}

/// Sort rows by one column, keys computed once per row (not parsed again per comparison):
/// numbers as numbers (text in a numeric column after them), NULLs last when ascending.
fn sort_rows(rows: &mut [Row], col: usize, asc: bool, numeric: bool) {
    if numeric {
        rows.sort_by_cached_key(|r| match &r.cur[col] {
            None => (true, 0),
            Some(s) => (false, ordered(s.parse::<f64>().unwrap_or(f64::NAN))),
        });
    } else {
        rows.sort_by_cached_key(|r| (r.cur[col].is_none(), r.cur[col].clone()));
    }
    if !asc {
        rows.reverse();
    }
}

/// An f64 as an integer with the same order as `f64::total_cmp` (NaN after every number).
fn ordered(x: f64) -> i64 {
    let b = x.to_bits() as i64;
    b ^ ((((b >> 63) as u64) >> 1) as i64)
}

/// Clipboard text as rows of tab-separated cells (trailing line break ignored).
// ponytail: no quoted cells (Excel quotes values holding tabs / line breaks); add when users paste those.
fn parse_tsv(s: &str) -> Vec<Vec<String>> {
    let s = s.strip_suffix('\n').unwrap_or(s);
    let s = s.strip_suffix('\r').unwrap_or(s);
    if s.is_empty() {
        return vec![];
    }
    s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).split('\t').map(str::to_owned).collect()).collect()
}

/// Name / Value grid for single-row "Details" tabs.
pub fn transpose(cols: &[Column], rows: &[Vec<Cell>]) -> (Vec<Column>, Vec<Vec<Cell>>) {
    let tc = |n: &str| Column { name: n.into(), type_name: String::new(), numeric: false };
    let first = rows.first();
    let out = cols.iter().enumerate().map(|(i, c)| vec![Some(c.name.clone()), first.and_then(|r| r[i].clone())]).collect();
    (vec![tc("Name"), tc("Value")], out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(top: i32, left: i32, bottom: i32, right: i32) -> GridBlockCoords {
        GridBlockCoords { top_row: top, left_col: left, bottom_row: bottom, right_col: right }
    }

    fn rows_of(vals: &[Option<&str>]) -> Vec<Row> {
        vals.iter().map(|v| Row::fetched(vec![v.map(str::to_owned)])).collect()
    }

    fn col0(rows: &[Row]) -> Vec<Option<String>> {
        rows.iter().map(|r| r.cur[0].clone()).collect()
    }

    #[test]
    fn sort_numbers_as_numbers_nulls_last() {
        let mut rows = rows_of(&[Some("10"), None, Some("9"), Some("-1.5")]);
        sort_rows(&mut rows, 0, true, true);
        assert_eq!(col0(&rows), vec![Some("-1.5".into()), Some("9".into()), Some("10".into()), None]);
        sort_rows(&mut rows, 0, false, true);
        assert_eq!(col0(&rows), vec![None, Some("10".into()), Some("9".into()), Some("-1.5".into())]);
    }

    #[test]
    fn sort_text_and_unparsable_numbers() {
        let mut rows = rows_of(&[Some("b"), Some("a"), None, Some("c")]);
        sort_rows(&mut rows, 0, true, false);
        assert_eq!(col0(&rows), vec![Some("a".into()), Some("b".into()), Some("c".into()), None]);
        // Text in a numeric column sorts after the numbers (NaN), before NULL.
        let mut rows = rows_of(&[Some("x"), Some("2"), None, Some("1")]);
        sort_rows(&mut rows, 0, true, true);
        assert_eq!(col0(&rows), vec![Some("1".into()), Some("2".into()), Some("x".into()), None]);
    }

    #[test]
    fn tsv() {
        assert_eq!(parse_tsv("a\tb\r\nc\t\r\n"), vec![vec!["a", "b"], vec!["c", ""]]);
        assert_eq!(parse_tsv("x"), vec![vec!["x"]]);
        assert!(parse_tsv("").is_empty());
    }

    #[test]
    fn delete_row_ignores_a_stale_auto_selection() {
        // The cursor cell's own 1x1 block, a drag over rows 1-3, whole rows.
        assert_eq!(rows_to_mark(vec![], &[block(4, 2, 4, 2)], (4, 2)), vec![4]);
        assert_eq!(rows_to_mark(vec![], &[block(1, 0, 3, 1)], (9, 0)), vec![1, 2, 3]);
        assert_eq!(rows_to_mark(vec![5, 2], &[], (0, 0)), vec![2, 5]);
        // A 1x1 block left on row 3 after code moved the cursor to row 7: row 7 is marked.
        assert_eq!(rows_to_mark(vec![], &[block(3, 1, 3, 1)], (7, 1)), vec![7]);
        // Ctrl-clicked cells on two rows are a real selection.
        assert_eq!(rows_to_mark(vec![], &[block(3, 1, 3, 1), block(5, 1, 5, 1)], (5, 1)), vec![3, 5]);
        assert_eq!(rows_to_mark(vec![], &[], (6, 0)), vec![6]);
    }
}
