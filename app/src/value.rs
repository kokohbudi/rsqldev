//! View Value dialog for one grid cell (SQL Developer's "…" cell editor): long text / CLOB in a
//! multi-line editor, BLOBs viewed as image, text or hex, with Save / Load to file.

use rsqldev_core::Column;
use wxdragon::prelude::*;

pub enum Outcome {
    Keep,
    /// Edited text (None = NULL).
    Text(Option<String>),
    Save,
    Load,
}

const ID_SAVE: i32 = 7101;
const ID_LOAD: i32 = 7102;
/// Hex view cap: past this the dump is cut (the file save always has every byte).
const HEX_LIMIT: usize = 1 << 20;

/// Classic 16-bytes-per-line dump: offset, hex, printable ASCII.
pub fn hex_dump(b: &[u8], limit: usize) -> String {
    let mut out = String::new();
    for (i, chunk) in b[..b.len().min(limit)].chunks(16).enumerate() {
        let hex: Vec<String> = chunk.iter().map(|x| format!("{x:02X}")).collect();
        let txt: String = chunk.iter().map(|&x| if (0x20..0x7f).contains(&x) { x as char } else { '.' }).collect();
        out += &format!("{:08X}  {:<47}  {txt}\n", i * 16, hex.join(" "));
    }
    if b.len() > limit {
        out += &format!("… {} more bytes (use Save to File)\n", b.len() - limit);
    }
    out
}

/// `bytes` is set for binary columns (staged upload or decoded cell); `text` for the rest.
pub fn show(parent: &dyn WxWidget, col: &Column, text: Option<&str>, bytes: Option<&[u8]>, editable: bool) -> Outcome {
    let dlg = Dialog::builder(parent, &format!("View Value: {}", col.name))
        .with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder)
        .with_size(760, 540)
        .build();
    let s = BoxSizer::builder(Orientation::Vertical).build();
    let size = match (bytes, text) {
        (Some(b), _) => format!("{} bytes", b.len()),
        (None, Some(t)) => format!("{} characters", t.chars().count()),
        (None, None) => "NULL".into(),
    };
    let info = StaticText::builder(&dlg).with_label(&format!("{}  ·  {}", col.type_name, size)).build();
    info.set_foreground_color(crate::theme::c(crate::theme::p().muted));
    let prefs = crate::app().prefs();
    let mono = |t: &TextCtrl| {
        if let Some(f) = Font::new_with_details(prefs.editor_font_size, FontFamily::Teletype as i32, 0, 400, false, &prefs.editor_font) {
            t.set_font(&f);
        }
    };
    let head = BoxSizer::builder(Orientation::Horizontal).build();
    head.add(&info, 1, SizerFlag::AlignCenterVertical, 0);
    s.add_sizer(&head, 0, SizerFlag::Expand | SizerFlag::All, 8);

    let text_ro = bytes.is_some() || !editable;
    let style = if text_ro { TextCtrlStyle::MultiLine | TextCtrlStyle::ReadOnly } else { TextCtrlStyle::MultiLine };
    let body = TextCtrl::builder(&dlg).with_style(style).build();
    mono(&body);
    s.add(&body, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
    let null = CheckBox::builder(&dlg).with_label("NULL").build();
    null.set_value(text.is_none() && bytes.is_none());

    match bytes {
        Some(b) => {
            let img = image::load_from_memory(b).ok().map(|i| i.thumbnail(1600, 1200).to_rgba8());
            let views: Vec<String> = img.iter().map(|_| "Image".to_string()).chain(["Text".into(), "Hex".into()]).collect();
            let choice = Choice::builder(&dlg).with_choices(views.clone()).build();
            choice.set_selection(0);
            head.add(&StaticText::builder(&dlg).with_label("View as:").build(), 0, SizerFlag::AlignCenterVertical | SizerFlag::Right, 6);
            head.add(&choice, 0, SizerFlag::AlignCenterVertical, 0);
            let scroll = ScrolledWindow::builder(&dlg).build();
            scroll.set_scroll_rate(10, 10);
            if let Some(px) = &img {
                if let Some(bmp) = Bitmap::from_rgba(px.as_raw(), px.width(), px.height()) {
                    let sb = StaticBitmap::new_with_bitmap(&scroll, ID_ANY as i32, &bmp);
                    let ss = BoxSizer::builder(Orientation::Vertical).build();
                    ss.add(&sb, 0, SizerFlag::All, 4);
                    scroll.set_sizer(ss, true);
                }
            }
            s.add(&scroll, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 8);
            let data = b.to_vec();
            let render = move |view: &str| match view {
                "Image" => {
                    body.show(false);
                    scroll.show(true);
                }
                v => {
                    scroll.show(false);
                    body.show(true);
                    body.set_value(&if v == "Hex" { hex_dump(&data, HEX_LIMIT) } else { String::from_utf8_lossy(&data[..data.len().min(HEX_LIMIT * 4)]).into_owned() });
                }
            };
            render(&views[0]);
            choice.on_selection_changed(move |_| {
                if let Some(v) = choice.get_string_selection() {
                    render(&v);
                    dlg.layout();
                }
            });
        }
        None => body.set_value(text.unwrap_or("")),
    }

    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    let btn = |label: &str, id: i32| {
        let b = Button::builder(&dlg).with_label(label).with_id(id).build();
        b.on_click(move |_| dlg.end_modal(id));
        b
    };
    bs.add(&btn("Save to File…", ID_SAVE), 0, SizerFlag::All, 4);
    if editable {
        bs.add(&btn("Load from File…", ID_LOAD), 0, SizerFlag::All, 4);
        if bytes.is_none() {
            bs.add(&null, 0, SizerFlag::AlignCenterVertical | SizerFlag::Left, 8);
        }
    } else {
        null.show(false);
    }
    bs.add_stretch_spacer(1);
    let text_edit = editable && bytes.is_none();
    bs.add(&btn(if text_edit { "Cancel" } else { "Close" }, ID_CANCEL), 0, SizerFlag::All, 4);
    if text_edit {
        bs.add(&btn("OK", ID_OK), 0, SizerFlag::All, 4);
    }
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer(s, true);
    match dlg.show_modal() {
        ID_OK if null.get_value() => Outcome::Text(None),
        ID_OK => Outcome::Text(Some(body.get_value())),
        ID_SAVE => Outcome::Save,
        ID_LOAD => Outcome::Load,
        _ => Outcome::Keep,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dump() {
        let d = super::hex_dump(b"PK\x03\x04hello world!!!!!X", 1024);
        assert!(d.starts_with("00000000  50 4B 03 04 68"), "{d}");
        assert!(d.contains("PK..hello world!"), "{d}");
        assert!(d.lines().nth(1).unwrap().starts_with("00000010  21 21 21 21 58"));
        assert!(super::hex_dump(&[0; 40], 16).contains("24 more bytes"));
    }
}
