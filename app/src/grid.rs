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

pub const NULL_TEXT: &str = "(null)";

#[derive(Clone, Default)]
pub struct Row {
    pub cur: Vec<Cell>,
    /// Values as fetched; None for rows inserted in the grid.
    pub orig: Option<Vec<Cell>>,
    pub deleted: bool,
    /// Files loaded into binary cells (column → bytes), posted as bind values on commit.
    pub blobs: HashMap<usize, Rc<Vec<u8>>>,
}

impl Row {
    fn fetched(r: Vec<Cell>) -> Row {
        Row { orig: Some(r.clone()), cur: r, ..Row::default() }
    }

    pub fn dirty(&self) -> bool {
        self.deleted || !self.blobs.is_empty() || self.orig.as_ref() != Some(&self.cur)
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
}

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
        grid.set_default_row_size(20, true);
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
        let me = g.clone();
        grid.on_cell_right_click(move |e| me.context_menu(Some(e.get_position()), (e.get_row(), e.get_col())));
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
        let keep = (st.on_change.take(), st.db);
        *st = State { on_change: keep.0, db: keep.1, ..State::default() };
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
            let st = self.st.borrow();
            match &st.rx {
                Some(rx) => (rx.clone(), st.hidden, st.fetch_all, st.rows.len()),
                None => return,
            }
        };
        let wants = fetch_all || n == 0 || self.grid.is_visible(n as i32 - 1, hidden as i32, false);
        if !wants {
            return;
        }
        let ev = {
            let r = rx.borrow();
            match r.as_ref() {
                Some(rx) => rx.try_recv(),
                None => Err(TryRecvError::Disconnected),
            }
        };
        match ev {
            Ok(Event::Columns(c)) => {
                self.st.borrow_mut().cols = c;
                self.redraw_all();
            }
            Ok(Event::Rows(batch)) => {
                let start = n;
                let last = batch.is_empty();
                {
                    let mut st = self.st.borrow_mut();
                    st.rows.extend(batch.into_iter().map(Row::fetched));
                }
                self.append_rows(start);
                if last {
                    self.finish();
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.finish(),
        }
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
        // (null) placeholder, ≥ 4.5:1 in both palettes (see theme tests).
        let grey = crate::theme::c(crate::theme::p().null);
        for (ri, row) in st.rows.iter().enumerate().skip(start) {
            for (ci, v) in row.cur.iter().enumerate() {
                let (r, c) = (ri as i32, ci as i32);
                match v {
                    Some(s) => g.set_cell_value(r, c, s),
                    None => {
                        g.set_cell_value(r, c, NULL_TEXT);
                        g.set_cell_text_colour(r, c, &grey);
                    }
                }
                if st.cols.get(ci).is_some_and(|c| c.numeric) {
                    g.set_cell_alignment(r, c, 0x0200 /* wxALIGN_RIGHT */, 0x0800 /* wxALIGN_CENTER_VERTICAL */);
                }
                if !st.editable {
                    g.set_read_only(r, c, true);
                }
            }
            Self::mark_row(g, &st, ri);
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
    }

    fn mark_row(g: Grid, st: &State, ri: usize) {
        let row = &st.rows[ri];
        let r = ri as i32;
        let label = if row.deleted {
            format!("{} ✕", ri + 1)
        } else if row.orig.is_none() {
            format!("{} +", ri + 1)
        } else if row.dirty() {
            format!("{} *", ri + 1)
        } else {
            (ri + 1).to_string()
        };
        g.set_row_label_value(r, &label);
        let edited = crate::theme::c(crate::theme::p().edited_bg);
        let removed = crate::theme::c(crate::theme::p().deleted_bg);
        for ci in 0..row.cur.len() {
            let changed = row.blobs.contains_key(&ci) || row.orig.as_ref().is_none_or(|o| o[ci] != row.cur[ci]);
            if row.deleted {
                g.set_cell_background_colour(r, ci as i32, &removed);
            } else if changed && st.editable {
                g.set_cell_background_colour(r, ci as i32, &edited);
            }
        }
    }

    fn cell_edited(&self, r: i32, c: i32) {
        let v = self.grid.get_cell_value(r, c);
        let mut st = self.st.borrow_mut();
        let Some(row) = st.rows.get_mut(r as usize) else { return };
        row.cur[c as usize] = if v.is_empty() || v == NULL_TEXT { None } else { Some(v) };
        row.blobs.remove(&(c as usize));
        Self::mark_row(self.grid, &st, r as usize);
        drop(st);
        self.grid.force_refresh();
        self.changed();
    }

    pub fn insert_row(&self) {
        let n = {
            let mut st = self.st.borrow_mut();
            let w = st.cols.len();
            st.rows.push(Row { cur: vec![None; w], ..Row::default() });
            st.rows.len() - 1
        };
        self.append_rows(n);
        self.grid.go_to_cell(n as i32, self.st.borrow().hidden as i32);
    }

    /// Add rows as pending inserts (import); Commit posts them.
    pub fn append_new_rows(&self, rows: Vec<Vec<Cell>>) {
        let start = {
            let mut st = self.st.borrow_mut();
            let start = st.rows.len();
            st.rows.extend(rows.into_iter().map(|cur| Row { cur, ..Row::default() }));
            start
        };
        self.append_rows(start);
        self.grid.go_to_cell(start as i32, self.st.borrow().hidden as i32);
    }

    pub fn delete_selected_rows(&self) {
        let mut rows: Vec<i32> = self.grid.get_selected_rows();
        if rows.is_empty() {
            rows.push(self.grid.get_grid_cursor_row());
        }
        let mut st = self.st.borrow_mut();
        for r in rows.into_iter().filter(|r| *r >= 0) {
            if let Some(row) = st.rows.get_mut(r as usize) {
                row.deleted = true;
            }
            Self::mark_row(self.grid, &st, r as usize);
        }
        drop(st);
        self.grid.force_refresh();
        self.changed();
    }

    fn sort_by(&self, col: usize) {
        {
            let mut st = self.st.borrow_mut();
            let asc = !matches!(st.sort, Some((c, true)) if c == col);
            let numeric = st.cols.get(col).is_some_and(|c| c.numeric);
            st.rows.sort_by(|a, b| {
                let (x, y) = (&a.cur[col], &b.cur[col]);
                let o = match (x, y) {
                    (None, None) => std::cmp::Ordering::Equal,
                    (None, _) => std::cmp::Ordering::Greater, // nulls last
                    (_, None) => std::cmp::Ordering::Less,
                    (Some(x), Some(y)) if numeric => x.parse::<f64>().unwrap_or(f64::NAN).total_cmp(&y.parse::<f64>().unwrap_or(f64::NAN)),
                    (Some(x), Some(y)) => x.cmp(y),
                };
                if asc { o } else { o.reverse() }
            });
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
        let mut b = Menu::builder()
            .append_item(ID_COPY, "Copy", "Copy selected cells")
            .append_item(ID_COPY_HDR, "Copy with Headers", "Copy selected cells with column names")
            .append_separator();
        if r >= 0 && c >= hidden {
            b = b.append_item(ID_VIEW_VALUE, "View Value…", "Show the whole value: long text, BLOB as image / text / hex");
            b = b.append_item(ID_SAVE_VALUE, "Save Value to File…", "Write this cell (BLOB bytes or text) to a file");
            if editable {
                b = b.append_item(ID_LOAD_VALUE, "Load Value from File…", "Replace this cell with a file's contents; Commit to post");
            }
            b = b.append_separator();
        }
        let mut m = b
            .append_item(ID_FETCH_ALL, "Fetch All Rows", "Fetch every remaining row")
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
            let t = format!("(BLOB {} bytes)", bytes.len());
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
    fn set_cell(&self, r: usize, c: usize, v: Cell) {
        let mut st = self.st.borrow_mut();
        let Some(row) = st.rows.get_mut(r) else { return };
        row.cur[c] = v.clone();
        let p = crate::theme::p();
        self.grid.set_cell_value(r as i32, c as i32, v.as_deref().unwrap_or(NULL_TEXT));
        self.grid.set_cell_text_colour(r as i32, c as i32, &crate::theme::c(if v.is_some() { p.text } else { p.null }));
        Self::mark_row(self.grid, &st, r);
        drop(st);
        self.grid.force_refresh();
        self.changed();
    }

    fn view_value(&self, r: usize, c: usize) {
        let (col, text, bytes, editable) = {
            let st = self.st.borrow();
            let (Some(row), Some(col)) = (st.rows.get(r), st.cols.get(c)) else { return };
            let binary = rsqldev_core::is_binary(&col.type_name);
            let bytes = row.blobs.get(&c).map(|b| b.to_vec()).or_else(|| row.cur[c].as_deref().filter(|_| binary).map(|s| rsqldev_core::cell_bytes(col, s)));
            (col.clone(), row.cur[c].clone(), bytes, st.editable && c >= st.hidden)
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

    /// Export… button: pick a format, then save.
    pub fn export_dialog(&self) {
        let d = SingleChoiceDialog::builder(&self.grid, "Format:", "Export", &["CSV", "Excel (xlsx)", "JSON", "INSERT statements"]).build();
        if d.show_modal() == ID_OK {
            match d.get_selection() {
                0 => self.menu(ID_CSV),
                1 => self.menu(ID_XLSX),
                2 => self.menu(ID_JSON),
                3 => self.menu(ID_INSERT),
                _ => {}
            }
        }
    }

    fn menu(&self, id: i32) {
        match id {
            ID_COPY | ID_COPY_HDR => {
                let (c, r) = self.selection(false);
                Clipboard::get().set_text(&export::tsv((id == ID_COPY_HDR).then_some(&c[..]), &r));
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
                let me = self.clone();
                self.fetch_all(move || {
                    let (c, r) = me.selection(true);
                    let db = me.st.borrow().db.unwrap_or(Kind::Oracle);
                    let data = match id {
                        ID_CSV => Ok(export::csv(&c, &r).into_bytes()),
                        ID_XLSX => export::xlsx(&c, &r),
                        ID_JSON => Ok(export::json(&c, &r).into_bytes()),
                        _ => Ok(export::inserts(db, "EXPORT_TABLE", &c, &r).into_bytes()),
                    };
                    if let Err(e) = data.and_then(|d| Ok(std::fs::write(&path, d)?)) {
                        crate::ui::error(&me.grid, &format!("Export failed: {e}"));
                    }
                });
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

/// Name / Value grid for single-row "Details" tabs.
pub fn transpose(cols: &[Column], rows: &[Vec<Cell>]) -> (Vec<Column>, Vec<Vec<Cell>>) {
    let tc = |n: &str| Column { name: n.into(), type_name: String::new(), numeric: false };
    let first = rows.first();
    let out = cols.iter().enumerate().map(|(i, c)| vec![Some(c.name.clone()), first.and_then(|r| r[i].clone())]).collect();
    (vec![tc("Name"), tc("Value")], out)
}
