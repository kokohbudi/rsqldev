//! Single Record View (grid context menu): one row as Name / Value with First / Previous /
//! Next / Last, SQL Developer style. Edited values go into the grid as pending changes.

use crate::grid::{NULL_TEXT, ResultGrid};
use wxdragon::prelude::*;

pub fn show(g: &ResultGrid, start: usize) {
    let (names, hidden, editable) = {
        let st = g.st.borrow();
        (st.cols.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), st.hidden, st.editable)
    };
    if names.len() <= hidden || g.st.borrow().rows.is_empty() {
        return;
    }
    let dlg = Dialog::builder(&g.grid, "Single Record View").with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(560, 520).build();
    let grid = Grid::builder(&dlg).build();
    grid.create_grid((names.len() - hidden) as i32, 1, GridSelectionMode::Cells);
    grid.set_col_label_value(0, "Value");
    grid.set_row_label_size(180);
    grid.set_col_size(0, 340);
    for (i, n) in names.iter().enumerate().skip(hidden) {
        grid.set_row_label_value((i - hidden) as i32, n);
    }
    grid.enable_editing(editable);
    let pos = StaticText::builder(&dlg).with_label("").build();
    let cur = std::rc::Rc::new(std::cell::Cell::new(start));
    let shown: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();

    let (g1, cur1, shown1) = (g.clone(), cur.clone(), shown.clone());
    let fill = move || {
        let st = g1.st.borrow();
        let r = cur1.get().min(st.rows.len() - 1);
        cur1.set(r);
        let row = &st.rows[r];
        let mut vals = vec![];
        for c in hidden..names.len() {
            let v = row.cur[c].as_deref().map_or_else(|| NULL_TEXT.to_string(), |s| rsqldev_core::lob::grid_text(&st.cols[c].type_name, s).into_owned());
            let i = (c - hidden) as i32;
            grid.set_cell_value(i, 0, &v);
            let ro = !editable || row.posted || rsqldev_core::lob::is_blob_type(&st.cols[c].type_name);
            grid.set_read_only(i, 0, ro);
            vals.push(v);
        }
        *shown1.borrow_mut() = vals;
        pos.set_label(&format!("Row {} of {}{}", r + 1, st.rows.len(), if st.done { "" } else { "+" }));
    };
    // Changed values go back into the grid row before moving or closing.
    let (g2, cur2, shown2) = (g.clone(), cur.clone(), shown.clone());
    let save = move || {
        grid.save_edit_control_value();
        let r = cur2.get();
        let old = shown2.borrow().clone();
        for (i, before) in old.iter().enumerate() {
            let v = grid.get_cell_value(i as i32, 0);
            if &v != before {
                g2.set_cell(r, i + hidden, if v.is_empty() || v == NULL_TEXT { None } else { Some(v) });
            }
        }
    };

    let bar = BoxSizer::builder(Orientation::Horizontal).build();
    for (label, step) in [("|◀", i64::MIN), ("◀", -1), ("▶", 1), ("▶|", i64::MAX)] {
        let b = Button::builder(&dlg).with_label(label).build();
        bar.add(&b, 0, SizerFlag::All, 3);
        let (fill, save, cur, g) = (fill.clone(), save.clone(), cur.clone(), g.clone());
        b.on_click(move |_| {
            save();
            let n = g.st.borrow().rows.len() as i64;
            let r = match step {
                i64::MIN => 0,
                i64::MAX => n - 1,
                s => (cur.get() as i64 + s).clamp(0, n - 1),
            };
            cur.set(r as usize);
            fill();
        });
    }
    bar.add(&pos, 1, SizerFlag::AlignCenterVertical | SizerFlag::Left, 10);
    let close = Button::builder(&dlg).with_label("Close").with_id(ID_OK).build();
    bar.add(&close, 0, SizerFlag::All, 3);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add(&grid, 1, SizerFlag::Expand | SizerFlag::All, 6);
    s.add_sizer(&bar, 0, SizerFlag::Expand | SizerFlag::All, 4);
    dlg.set_sizer(s, true);
    fill();
    dlg.show_modal();
    save();
    let r = cur.get() as i32;
    let c = hidden as i32;
    g.move_cursor(r, c, true);
    dlg.destroy();
}
