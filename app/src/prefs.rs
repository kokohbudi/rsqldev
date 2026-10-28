//! Tools ▸ Preferences.

use wxdragon::prelude::*;

pub fn show(parent: &Frame) {
    let app = crate::app();
    let mut p = app.prefs();
    let dlg = Dialog::builder(parent, "Preferences").with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(560, 640).build();
    let g = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    g.add_growable_col(1, 1);
    let font = TextCtrl::builder(&dlg).with_value(&p.editor_font).build();
    let size = SpinCtrl::builder(&dlg).with_range(6, 48).with_initial_value(p.editor_font_size).build();
    let auto = CheckBox::builder(&dlg).with_label("Autocommit").build();
    auto.set_value(p.autocommit);
    let post = CheckBox::builder(&dlg).with_label("Post edits on row change (data editor)").build();
    post.set_value(p.post_on_row_change);
    let upd = CheckBox::builder(&dlg).with_label("Check for updates on start").build();
    upd.set_value(p.check_updates);
    let nls = TextCtrl::builder(&dlg).with_value(&p.nls_date_format).build();
    let nls_ts = TextCtrl::builder(&dlg).with_value(&p.nls_timestamp_format).build();
    let null = TextCtrl::builder(&dlg).with_value(&p.null_display).build();
    let tns = TextCtrl::builder(&dlg).with_value(&p.tns_dir).build();
    tns.set_tooltip("Folder with tnsnames.ora, used when a connection sets no tns_admin option");
    let lines = CheckBox::builder(&dlg).with_label("Show line numbers").build();
    lines.set_value(p.line_numbers);
    let on_connect = CheckBox::builder(&dlg).with_label("Open a worksheet on connect").build();
    on_connect.set_value(p.open_worksheet_on_connect);
    let hist = SpinCtrl::builder(&dlg).with_range(10, 10000).with_initial_value(p.history_limit.min(10000) as i32).build();
    let reopen = CheckBox::builder(&dlg).with_label("Reopen worksheets at start (saved-password connections)").build();
    reopen.set_value(p.reopen_worksheets);
    let binds = CheckBox::builder(&dlg).with_label("Save bind values between sessions").build();
    binds.set_value(p.save_binds);
    let templates = Button::builder(&dlg).with_label("Code Templates…").build();
    let tpl = std::rc::Rc::new(std::cell::RefCell::new(p.code_templates.clone()));
    let t2 = tpl.clone();
    templates.on_click(move |_| templates_dialog(&dlg, &mut t2.borrow_mut()));
    let storage = CheckBox::builder(&dlg).with_label("Oracle DDL: storage and tablespace clauses").build();
    storage.set_value(p.ddl_storage);
    const THEMES: [(&str, &str); 3] = [("light", "Light (SQL Developer)"), ("dark", "Dark"), ("system", "Follow system")];
    let theme = Choice::builder(&dlg).with_choices(THEMES.iter().map(|t| t.1.to_string()).collect()).build();
    theme.set_selection(THEMES.iter().position(|t| t.0 == p.theme).unwrap_or(0) as u32);
    crate::ui::row(&dlg, &g, "Appearance", &theme);
    crate::ui::row(&dlg, &g, "Editor font", &font);
    crate::ui::row(&dlg, &g, "Font size", &size);
    crate::ui::row(&dlg, &g, "", &auto);
    crate::ui::row(&dlg, &g, "", &post);
    crate::ui::row(&dlg, &g, "", &upd);
    crate::ui::row(&dlg, &g, "", &lines);
    crate::ui::row(&dlg, &g, "", &templates);
    crate::ui::row(&dlg, &g, "", &on_connect);
    crate::ui::row(&dlg, &g, "", &reopen);
    crate::ui::row(&dlg, &g, "", &binds);
    crate::ui::row(&dlg, &g, "SQL History limit", &hist);
    crate::ui::row(&dlg, &g, "Display NULL as", &null);
    crate::ui::row(&dlg, &g, "Tnsnames directory", &tns);
    crate::ui::row(&dlg, &g, "", &storage);
    crate::ui::row(&dlg, &g, "Oracle NLS date format", &nls);
    crate::ui::row(&dlg, &g, "Oracle NLS timestamp format", &nls_ts);
    let note = StaticText::builder(&dlg).with_label("Connection settings (autocommit, NLS, DDL, tnsnames) apply to new connections;\nline numbers to new worksheets.").build();
    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    bs.add_stretch_spacer(1);
    let cancel = Button::builder(&dlg).with_label("Cancel").with_id(ID_CANCEL).build();
    let ok = Button::builder(&dlg).with_label("OK").with_id(ID_OK).build();
    bs.add(&cancel, 0, SizerFlag::All, 4);
    bs.add(&ok, 0, SizerFlag::All, 4);
    ok.on_click(move |_| dlg.end_modal(ID_OK));
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_sizer(&g, 1, SizerFlag::Expand | SizerFlag::All, 12);
    s.add(&note, 0, SizerFlag::All, 12);
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer(s, true);
    if dlg.show_modal() == ID_OK {
        p.editor_font = font.get_value();
        p.editor_font_size = size.value();
        p.autocommit = auto.get_value();
        p.check_updates = upd.get_value();
        p.post_on_row_change = post.get_value();
        p.nls_date_format = nls.get_value();
        p.nls_timestamp_format = nls_ts.get_value();
        p.null_display = null.get_value();
        p.tns_dir = tns.get_value().trim().to_string();
        p.line_numbers = lines.get_value();
        p.open_worksheet_on_connect = on_connect.get_value();
        p.history_limit = hist.value().max(10) as usize;
        p.ddl_storage = storage.get_value();
        p.save_binds = binds.get_value();
        p.code_templates = tpl.borrow().clone();
        p.reopen_worksheets = reopen.get_value();
        if !p.save_binds {
            let _ = std::fs::remove_file(rsqldev_core::config::config_root().join("binds.toml"));
        }
        let new_theme = THEMES[theme.get_selection().unwrap_or(0) as usize].0.to_string();
        let changed = new_theme != p.theme;
        p.theme = new_theme;
        app.set_prefs(p);
        if changed {
            app.offer_restart();
        }
    }
    dlg.destroy();
}

/// Preferences ▸ Code Templates…: abbreviation → text (`|` marks the caret).
fn templates_dialog(parent: &Dialog, map: &mut std::collections::BTreeMap<String, String>) {
    let dlg = Dialog::builder(parent, "Code Templates").with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(640, 460).build();
    let list = ListCtrl::builder(&dlg).with_style(ListCtrlStyle::Report | ListCtrlStyle::SingleSel).build();
    list.insert_column(0, "Abbreviation", ListColumnFormat::Left, 110);
    list.insert_column(1, "Template (Tab after the abbreviation; | = caret)", ListColumnFormat::Left, 480);
    let m = std::rc::Rc::new(std::cell::RefCell::new(map.clone()));
    let fill = {
        let m = m.clone();
        move || {
            list.delete_all_items();
            for (i, (k, v)) in m.borrow().iter().enumerate() {
                list.insert_item(i as i64, k, None);
                list.set_item_text_by_column(i as i64, 1, &v.replace('\n', " ⏎ "));
            }
        }
    };
    fill();
    let edit = {
        let (m, fill) = (m.clone(), fill.clone());
        move |key: Option<String>| {
            let cur = key.as_ref().and_then(|k| m.borrow().get(k).cloned()).unwrap_or_default();
            let d = Dialog::builder(&dlg, "Code Template").with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(480, 320).build();
            let k = TextCtrl::builder(&d).with_value(key.as_deref().unwrap_or("")).build();
            let v = TextCtrl::builder(&d).with_style(TextCtrlStyle::MultiLine).with_value(&cur).build();
            let s = BoxSizer::builder(Orientation::Vertical).build();
            s.add(&StaticText::builder(&d).with_label("Abbreviation:").build(), 0, SizerFlag::Left | SizerFlag::Top, 8);
            s.add(&k, 0, SizerFlag::Expand | SizerFlag::All, 8);
            s.add(&StaticText::builder(&d).with_label("Template (| marks the caret):").build(), 0, SizerFlag::Left, 8);
            s.add(&v, 1, SizerFlag::Expand | SizerFlag::All, 8);
            let bs = BoxSizer::builder(Orientation::Horizontal).build();
            bs.add_stretch_spacer(1);
            for (l, id) in [("Cancel", ID_CANCEL), ("OK", ID_OK)] {
                let b = Button::builder(&d).with_label(l).with_id(id).build();
                b.on_click(move |_| d.end_modal(id));
                bs.add(&b, 0, SizerFlag::All, 4);
            }
            s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 4);
            d.set_sizer(s, true);
            let ok = d.show_modal() == ID_OK;
            let (nk, nv) = (k.get_value().trim().to_string(), v.get_value());
            d.destroy();
            let valid = !nk.is_empty() && nk.chars().all(|c| c.is_alphanumeric() || c == '_');
            if ok && valid && !nv.is_empty() {
                let mut mm = m.borrow_mut();
                if let Some(old) = key.filter(|o| *o != nk) {
                    mm.remove(&old);
                }
                mm.insert(nk, nv);
                drop(mm);
                fill();
            } else if ok && !valid {
                crate::ui::info(&dlg, "An abbreviation is one word: letters, digits and _.");
            }
        }
    };
    let selected = {
        let m = m.clone();
        move || {
            let i = list.get_first_selected_item();
            (i >= 0).then(|| m.borrow().keys().nth(i as usize).cloned()).flatten()
        }
    };
    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    let b_add = Button::builder(&dlg).with_label("Add…").build();
    let b_edit = Button::builder(&dlg).with_label("Edit…").build();
    let b_del = Button::builder(&dlg).with_label("Remove").build();
    let b_reset = Button::builder(&dlg).with_label("Defaults").build();
    let (e1, e2, s1, s2) = (edit.clone(), edit.clone(), selected.clone(), selected.clone());
    b_add.on_click(move |_| e1(None));
    b_edit.on_click(move |_| {
        if let Some(k) = s1() {
            e2(Some(k))
        }
    });
    let (m1, f1) = (m.clone(), fill.clone());
    b_del.on_click(move |_| {
        if let Some(k) = s2() {
            m1.borrow_mut().remove(&k);
            f1();
        }
    });
    let (m2, f2) = (m.clone(), fill.clone());
    b_reset.on_click(move |_| {
        *m2.borrow_mut() = rsqldev_core::config::default_templates();
        f2();
    });
    let e3 = edit.clone();
    let s3 = selected.clone();
    list.on_item_activated(move |_| {
        if let Some(k) = s3() {
            e3(Some(k))
        }
    });
    for b in [&b_add, &b_edit, &b_del, &b_reset] {
        bs.add(b, 0, SizerFlag::All, 4);
    }
    bs.add_stretch_spacer(1);
    for (l, id) in [("Cancel", ID_CANCEL), ("OK", ID_OK)] {
        let b = Button::builder(&dlg).with_label(l).with_id(id).build();
        b.on_click(move |_| dlg.end_modal(id));
        bs.add(&b, 0, SizerFlag::All, 4);
    }
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add(&list, 1, SizerFlag::Expand | SizerFlag::All, 8);
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 4);
    dlg.set_sizer(s, true);
    if dlg.show_modal() == ID_OK {
        *map = m.borrow().clone();
    }
    dlg.destroy();
}
