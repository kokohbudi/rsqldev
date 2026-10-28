//! Small native dialog helpers.

use wxdragon::prelude::*;

/// Open a URL in the default browser.
pub fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    let _ = r;
}

pub fn error(parent: &dyn WxWidget, msg: &str) {
    if let Some(a) = crate::try_app() {
        a.log_line(&format!("Error: {msg}"));
    }
    MessageDialog::builder(parent, msg, "rsqldev").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconError).build().show_modal();
}

pub fn info(parent: &dyn WxWidget, msg: &str) {
    MessageDialog::builder(parent, msg, "rsqldev").with_style(MessageDialogStyle::OK | MessageDialogStyle::IconInformation).build().show_modal();
}

pub fn confirm(parent: &dyn WxWidget, msg: &str) -> bool {
    MessageDialog::builder(parent, msg, "rsqldev")
        .with_style(MessageDialogStyle::YesNo | MessageDialogStyle::IconQuestion)
        .build()
        .show_modal()
        == ID_YES
}

/// Like `confirm`, with the exact SQL shown highlighted under `msg`.
pub fn confirm_sql(parent: &dyn WxWidget, msg: &str, sql: &str) -> bool {
    let dlg = Dialog::builder(parent, "rsqldev").with_style(DialogStyle::DefaultDialogStyle | DialogStyle::ResizeBorder).with_size(760, 480).build();
    let text = crate::editor::sql_view(&dlg, sql);
    let bs = BoxSizer::builder(Orientation::Horizontal).build();
    bs.add_stretch_spacer(1);
    bs.add(&Button::builder(&dlg).with_label("No").with_id(ID_CANCEL).build(), 0, SizerFlag::All, 4);
    let yes = Button::builder(&dlg).with_label("Yes").with_id(ID_OK).build();
    yes.set_default();
    bs.add(&yes, 0, SizerFlag::All, 4);
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add(&StaticText::builder(&dlg).with_label(msg).build(), 0, SizerFlag::Expand | SizerFlag::All, 12);
    s.add(&text, 1, SizerFlag::Expand | SizerFlag::Left | SizerFlag::Right, 12);
    s.add_sizer(&bs, 0, SizerFlag::Expand | SizerFlag::All, 6);
    dlg.set_sizer(s, true);
    dlg.centre();
    yes.set_focus();
    let ok = dlg.show_modal() == ID_OK;
    dlg.destroy();
    ok
}

/// Like `confirm`, with No as the default button.
pub fn confirm_no(parent: &dyn WxWidget, msg: &str) -> bool {
    MessageDialog::builder(parent, msg, "rsqldev")
        .with_style(MessageDialogStyle::YesNo | MessageDialogStyle::NoDefault | MessageDialogStyle::IconQuestion)
        .build()
        .show_modal()
        == ID_YES
}

/// Uncommitted grid edits: Some(true) Commit, Some(false) Discard, None Cancel.
pub fn commit_discard(parent: &dyn WxWidget, msg: &str) -> Option<bool> {
    let d = MessageDialog::builder(parent, &format!("{msg}\n\nCommit them first?"), "rsqldev").with_style(MessageDialogStyle::YesNo | MessageDialogStyle::Cancel | MessageDialogStyle::IconQuestion).build();
    d.set_yes_no_labels("Commit", "Discard");
    match d.show_modal() {
        ID_YES => Some(true),
        ID_NO => Some(false),
        _ => None,
    }
}

pub fn ask(parent: &dyn WxWidget, msg: &str, title: &str, default: &str) -> Option<String> {
    let d = TextEntryDialog::builder(parent, msg, title).with_default_value(default).build();
    (d.show_modal() == ID_OK).then(|| d.get_value()).flatten()
}

pub fn ask_password(parent: &dyn WxWidget, msg: &str) -> Option<String> {
    let d = TextEntryDialog::builder(parent, msg, "Connection Password").password().build();
    (d.show_modal() == ID_OK).then(|| d.get_value()).flatten()
}

/// Label + control row for form dialogs.
pub fn row<P: WxWidget, W: WxWidget>(parent: &P, grid: &FlexGridSizer, label: &str, ctrl: &W) {
    let l = StaticText::builder(parent).with_label(label).build();
    grid.add(&l, 0, SizerFlag::AlignCenterVertical | SizerFlag::AlignRight, 0);
    grid.add(ctrl, 1, SizerFlag::Expand, 0);
}
