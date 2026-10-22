//! Tools ▸ Preferences.

use wxdragon::prelude::*;

pub fn show(parent: &Frame) {
    let app = crate::app();
    let mut p = app.prefs();
    let dlg = Dialog::builder(parent, "Preferences").with_style(DialogStyle::DefaultDialogStyle).with_size(480, 340).build();
    let g = FlexGridSizer::builder(0, 2).with_vgap(8).with_hgap(10).build();
    g.add_growable_col(1, 1);
    let font = TextCtrl::builder(&dlg).with_value(&p.editor_font).build();
    let size = SpinCtrl::builder(&dlg).with_range(6, 48).with_initial_value(p.editor_font_size).build();
    let fetch = SpinCtrl::builder(&dlg).with_range(1, 10000).with_initial_value(p.fetch_size as i32).build();
    let auto = CheckBox::builder(&dlg).with_label("Autocommit").build();
    auto.set_value(p.autocommit);
    let nls = TextCtrl::builder(&dlg).with_value(&p.nls_date_format).build();
    const THEMES: [(&str, &str); 3] = [("light", "Light (SQL Developer)"), ("dark", "Dark"), ("system", "Follow system")];
    let theme = Choice::builder(&dlg).with_choices(THEMES.iter().map(|t| t.1.to_string()).collect()).build();
    theme.set_selection(THEMES.iter().position(|t| t.0 == p.theme).unwrap_or(0) as u32);
    crate::ui::row(&dlg, &g, "Appearance", &theme);
    crate::ui::row(&dlg, &g, "Editor font", &font);
    crate::ui::row(&dlg, &g, "Font size", &size);
    crate::ui::row(&dlg, &g, "SQL array fetch size", &fetch);
    crate::ui::row(&dlg, &g, "", &auto);
    crate::ui::row(&dlg, &g, "Oracle NLS date format", &nls);
    let note = StaticText::builder(&dlg).with_label("Connection settings apply to new connections.").build();
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
        p.fetch_size = fetch.value().max(1) as usize;
        p.autocommit = auto.get_value();
        p.nls_date_format = nls.get_value();
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
