//! Light (SQL Developer default) and dark palettes. Chosen once at startup from
//! Preferences ▸ Appearance; native widgets follow via wxApp::SetAppearance.

use wxdragon::prelude::*;

pub struct Palette {
    pub bg: u32,
    pub text: u32,
    /// Secondary text (≥ 4.5:1 on `bg`).
    pub muted: u32,
    /// Headings / brand blue.
    pub accent: u32,
    pub link: u32,
    pub grid_line: u32,
    pub label_bg: u32,
    pub select_bg: u32,
    pub select_fg: u32,
    /// `(null)` placeholder text (≥ 4.5:1 on `bg`).
    pub null: u32,
    pub edited_bg: u32,
    pub deleted_bg: u32,
    pub invalid: u32,
    // Editor
    pub ed_bg: u32,
    pub ed_text: u32,
    pub ed_keyword: u32,
    pub ed_string: u32,
    pub ed_number: u32,
    pub ed_comment: u32,
    pub ed_caret_line: u32,
    pub ed_margin: u32,
    pub ed_margin_text: u32,
}

const LIGHT: Palette = Palette {
    bg: 0xffffff,
    text: 0x000000,
    muted: 0x555555,
    accent: 0x336699,
    link: 0x0645ad,
    grid_line: 0xdddddd,
    label_bg: 0xf2f2f2,
    select_bg: 0xc5dcf5,
    select_fg: 0x000000,
    null: 0x6e6e6e,
    edited_bg: 0xfff4c4,
    deleted_bg: 0xffd6d6,
    invalid: 0xc80000,
    // SQL Developer "Default" scheme.
    ed_bg: 0xffffff,
    ed_text: 0x000000,
    ed_keyword: 0x336699,
    ed_string: 0x0033cc,
    ed_number: 0x006600,
    ed_comment: 0x767676, // SQL Developer's #808080 nudged to 4.5:1
    ed_caret_line: 0xeef4fb,
    ed_margin: 0xf3f3f3,
    ed_margin_text: 0x8c8c8c,
};

const DARK: Palette = Palette {
    bg: 0x1e1f22,
    text: 0xdfe1e5,
    muted: 0xa0a4ab,
    accent: 0x7fb0e6,
    link: 0x8ab4f8,
    grid_line: 0x393b40,
    label_bg: 0x2b2d30,
    select_bg: 0x2e436e,
    select_fg: 0xffffff,
    null: 0x8c9099,
    edited_bg: 0x4a3f1c,
    deleted_bg: 0x5a2a2a,
    invalid: 0xff6b6b,
    ed_bg: 0x1e1f22,
    ed_text: 0xd4d4d4,
    ed_keyword: 0x6ea8dc,
    ed_string: 0x8fb8ff,
    ed_number: 0x8fcf8f,
    ed_comment: 0x8a8a8a,
    ed_caret_line: 0x2a2d33,
    ed_margin: 0x2b2b2b,
    ed_margin_text: 0x8c8c8c,
};

/// Resolved once: Preferences theme, "system" follows the OS.
pub fn is_dark() -> bool {
    use std::sync::OnceLock;
    static D: OnceLock<bool> = OnceLock::new();
    *D.get_or_init(|| {
        let t = rsqldev_core::config::load_prefs().theme;
        t == "dark" || (t == "system" && wxdragon::appearance::is_system_dark_mode())
    })
}

pub fn p() -> &'static Palette {
    if is_dark() { &DARK } else { &LIGHT }
}

pub fn c(hex: u32) -> Colour {
    Colour::rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(h: u32) -> f64 {
        let ch = |v: u32| {
            let s = v as f64 / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(h >> 16 & 0xff) + 0.7152 * ch(h >> 8 & 0xff) + 0.0722 * ch(h & 0xff)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn wcag_aa_text_contrast() {
        for p in [&LIGHT, &DARK] {
            for (name, fg) in [("text", p.text), ("muted", p.muted), ("null", p.null), ("accent", p.accent), ("link", p.link), ("invalid", p.invalid)] {
                assert!(contrast(fg, p.bg) >= 4.5, "{name} {fg:06x} on {:06x} = {:.2}", p.bg, contrast(fg, p.bg));
            }
            for (name, fg) in [("keyword", p.ed_keyword), ("string", p.ed_string), ("number", p.ed_number), ("comment", p.ed_comment), ("text", p.ed_text)] {
                assert!(contrast(fg, p.ed_bg) >= 4.5, "editor {name} {fg:06x} = {:.2}", contrast(fg, p.ed_bg));
            }
            assert!(contrast(p.select_fg, p.select_bg) >= 4.5);
            assert!(contrast(p.text, p.edited_bg) >= 4.5 && contrast(p.text, p.deleted_bg) >= 4.5);
        }
    }
}
