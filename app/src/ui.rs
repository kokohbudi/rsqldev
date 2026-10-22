//! Small native dialog helpers.

use wxdragon::prelude::*;

pub fn error(parent: &dyn WxWidget, msg: &str) {
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
