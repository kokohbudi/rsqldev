//! Activity buddy: a little database that hops while the database works (in the status bar and
//! next to the running worksheet's timer), blinks now and then, sweats when the work drags on,
//! and beams for a moment when it is done.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wxdragon::prelude::*;

/// Frames per hop (one tick each, 25 ms).
const HOP: usize = 12;
/// Seconds of work after which the buddy sweats and the timer turns the accent colour.
pub const TIRED_SECS: f32 = 5.0;
/// Seconds of work after which finishing in the background notifies (notification, dock).
pub const NOTIFY_SECS: f32 = 30.0;

/// One 20×20 frame. `h` = hop height 0..1, `blink` closes the eyes, `happy` = done pose,
/// `tired` adds a drop of sweat, `dark` lightens the shadow for a dark background.
fn svg(h: f32, blink: bool, happy: bool, tired: bool, dark: bool) -> String {
    let lift = 3.2 * h;
    // Squashed when it lands, stretched in the air.
    let (sx, sy) = if h < 0.15 { (1.12, 0.86) } else { (0.96, 1.05) };
    let shadow = 5.0 - 2.5 * h;
    let eyes = if happy {
        "<path d='M6.7 10.6l1.1-1.3 1.1 1.3M11.1 10.6l1.1-1.3 1.1 1.3' fill='none' stroke='#1f2937' stroke-width='0.9' stroke-linecap='round' stroke-linejoin='round'/>".to_string()
    } else if blink {
        "<path d='M6.7 10h2.2M11.1 10h2.2' stroke='#1f2937' stroke-width='0.9' stroke-linecap='round'/>".to_string()
    } else {
        "<circle cx='7.8' cy='10' r='1.3' fill='#1f2937'/><circle cx='12.2' cy='10' r='1.3' fill='#1f2937'/><circle cx='8.2' cy='9.5' r='0.45' fill='#fff'/><circle cx='12.6' cy='9.5' r='0.45' fill='#fff'/>".to_string()
    };
    let mouth = if happy { "<path d='M8.8 12.1h2.4q-1.2 1.8-2.4 0z' fill='#c2410c'/>" } else { "<path d='M9.2 12.3q.8.7 1.6 0' fill='none' stroke='#1f2937' stroke-width='0.7' stroke-linecap='round'/>" };
    let sweat = if tired { "<path d='M17.6 2.2q2 2.9 0 4.3q-2-1.4 0-4.3z' fill='#74c0fc' stroke='#1c7ed6' stroke-width='0.5'/>" } else { "" };
    let shade = if dark { "fill='#fff' opacity='0.3'" } else { "fill='#000' opacity='0.18'" };
    let sparkle = if happy { "<path d='M17 1.2l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6z' fill='#fcc419'/><circle cx='2.5' cy='3' r='0.7' fill='#fcc419'/>" } else { "" };
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20' viewBox='0 0 20 20'>\
         <ellipse cx='10' cy='18.6' rx='{shadow:.2}' ry='1.1' {shade}/>\
         <g transform='translate(0 {:.2}) translate(10 17.4) scale({:.3} {:.3}) translate(-10 -17)'>\
         <path d='M4 5v9.5a6 2.3 0 0 0 12 0V5' fill='#5b9bd5' stroke='#2f6fa3' stroke-width='0.8'/>\
         <ellipse cx='10' cy='5' rx='6' ry='2.3' fill='#a9d1f5' stroke='#2f6fa3' stroke-width='0.8'/>\
         <path d='M4.4 8.2a5.6 2 0 0 0 11.2 0' fill='none' stroke='#9cc7f0' stroke-width='0.7'/>\
         {eyes}<ellipse cx='6.2' cy='12.2' rx='1' ry='0.6' fill='#ff8fab' opacity='0.85'/><ellipse cx='13.8' cy='12.2' rx='1' ry='0.6' fill='#ff8fab' opacity='0.85'/>{mouth}{sweat}\
         </g>{sparkle}</svg>",
        -lift,
        sx * 0.88,
        sy * 0.88
    )
}

fn bundle(s: &str) -> Option<BitmapBundle> {
    BitmapBundle::from_svg_text(s, Size::new(20, 20))
}

/// The hop frames (eyes open, blinking, sweating) and the done pose.
pub struct Frames {
    open: Vec<BitmapBundle>,
    blink: Vec<BitmapBundle>,
    tired: Vec<BitmapBundle>,
    pub happy: BitmapBundle,
}

impl Frames {
    fn new() -> Option<Frames> {
        let dark = crate::theme::is_dark();
        let h = |i: usize| (std::f32::consts::PI * i as f32 / HOP as f32).sin();
        let hop = |blink: bool, tired: bool| (0..HOP).map(|i| bundle(&svg(h(i), blink, false, tired, dark))).collect::<Option<Vec<_>>>();
        Some(Frames { open: hop(false, false)?, blink: hop(true, false)?, tired: hop(false, true)?, happy: bundle(&svg(0.0, false, true, false, dark))? })
    }

    /// Frame for tick `t`: a hop every 12 ticks, a blink for 4 ticks every ~3 s; sweating once
    /// the work has taken [`TIRED_SECS`].
    pub fn at(&self, t: u32, tired: bool) -> &BitmapBundle {
        let i = t as usize % HOP;
        if t % 120 < 4 {
            &self.blink[i]
        } else if tired {
            &self.tired[i]
        } else {
            &self.open[i]
        }
    }
}

thread_local! {
    static FRAMES: RefCell<Option<Rc<Frames>>> = const { RefCell::new(None) };
    static ACTIVE: Cell<bool> = const { Cell::new(true) };
    /// The last finish notification, kept alive until the next (dropping one closes it).
    static NOTE: RefCell<Option<NotificationMessage>> = const { RefCell::new(None) };
}

/// The frames, drawn on first use (the first busy moment, not at startup) and shared.
pub fn frames() -> Option<Rc<Frames>> {
    FRAMES.with(|f| {
        if f.borrow().is_none() {
            *f.borrow_mut() = Frames::new().map(Rc::new);
        }
        f.borrow().clone()
    })
}

/// The main window is (de)activated: finishing in the background notifies.
pub fn set_app_active(on: bool) {
    ACTIVE.with(|a| a.set(on));
}

/// Work that took [`NOTIFY_SECS`] finished while rsqldev was in the background: a system
/// notification and a dock bounce / taskbar flash. Does nothing while rsqldev is in front.
pub fn notify_done(frame: &Frame, secs: f32, title: &str, what: &str) {
    if secs < NOTIFY_SECS || ACTIVE.with(|a| a.get()) {
        return;
    }
    let what: String = what.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect();
    if let Ok(n) = NotificationMessage::builder().with_title(title).with_message(&format!("Finished in {secs:.1} s  ·  {what}")).with_parent(frame).build() {
        n.show(TIMEOUT_AUTO);
        NOTE.with(|x| *x.borrow_mut() = Some(n));
    }
    frame.request_user_attention(UserAttentionFlag::Info);
}

/// Startup splash: the buddy beaming hello. Shown only while the main window is being built
/// (no delay of its own); the caller destroys it once the main window is up.
pub fn splash() -> Frame {
    let pal = crate::theme::p();
    let c = crate::theme::c;
    let f = Frame::builder().with_title("rsqldev").with_style(FrameStyle::StayOnTop | FrameStyle::NoTaskbar).with_size(Size::new(300, 230)).build();
    let p = Panel::builder(&f).build();
    p.set_background_color(c(pal.bg));
    let s = BoxSizer::builder(Orientation::Vertical).build();
    s.add_stretch_spacer(1);
    if let Some(b) = bundle_at(&svg(0.0, false, true, false, crate::theme::is_dark()), 96) {
        let pic = StaticBitmap::new_with_bitmap_bundle(&p, ID_ANY as i32, &b);
        s.add(&pic, 0, SizerFlag::AlignCenterHorizontal, 0);
    }
    let name = StaticText::builder(&p).with_label("rsqldev").build();
    if let Some(font) = Font::new_with_details(22, FontFamily::Swiss as i32, 0, 700, false, "") {
        name.set_font(&font);
    }
    name.set_foreground_color(c(pal.text));
    s.add(&name, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top, 8);
    let note = StaticText::builder(&p).with_label(&format!("v{}  ·  starting…", env!("CARGO_PKG_VERSION"))).build();
    note.set_foreground_color(c(pal.muted));
    s.add(&note, 0, SizerFlag::AlignCenterHorizontal | SizerFlag::Top, 4);
    s.add_stretch_spacer(1);
    p.set_sizer(s, true);
    f.center_on_screen();
    f.show(true);
    f.update();
    f
}

fn bundle_at(s: &str, px: i32) -> Option<BitmapBundle> {
    BitmapBundle::from_svg_text(s, Size::new(px, px))
}

/// Horizontal offset at tick `t` within `width` pixels: back and forth, 1 px per tick.
pub fn walk(t: u32, width: i32) -> i32 {
    if width <= 0 {
        return 0;
    }
    let p = (t as i32) % (2 * width);
    if p < width { p } else { 2 * width - p }
}

#[cfg(test)]
mod tests {
    /// `BUDDY_DIR=/tmp/x cargo test -p rsqldev buddy -- --ignored`: frames as SVG files to look at.
    #[test]
    #[ignore]
    fn dump_frames() {
        let dir = std::env::var("BUDDY_DIR").unwrap();
        let h = |i: usize| (std::f32::consts::PI * i as f32 / super::HOP as f32).sin();
        for i in 0..super::HOP {
            std::fs::write(format!("{dir}/hop{i:02}.svg"), super::svg(h(i), false, false, false, false)).unwrap();
        }
        std::fs::write(format!("{dir}/blink.svg"), super::svg(0.0, true, false, false, false)).unwrap();
        std::fs::write(format!("{dir}/tired.svg"), super::svg(0.0, false, false, true, false)).unwrap();
        std::fs::write(format!("{dir}/happy.svg"), super::svg(0.0, false, true, false, false)).unwrap();
        std::fs::write(format!("{dir}/dark.svg"), super::svg(0.0, false, false, true, true)).unwrap();
    }

    #[test]
    fn walks_back_and_forth() {
        let xs: Vec<i32> = (0..8).map(|t| super::walk(t, 3)).collect();
        assert_eq!(xs, vec![0, 1, 2, 3, 2, 1, 0, 1]);
        assert_eq!(super::walk(5, 0), 0);
    }
}
