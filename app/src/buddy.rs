//! Status-bar buddy: a little database that hops back and forth across the activity field while
//! the database works, blinks now and then, and beams for a moment when the work is done.

use wxdragon::prelude::*;

/// Frames per hop (one tick each, 25 ms).
const HOP: usize = 12;

/// One 20×20 frame. `h` = hop height 0..1, `blink` closes the eyes, `happy` = done pose.
fn svg(h: f32, blink: bool, happy: bool) -> String {
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
    let sparkle = if happy { "<path d='M17 1.2l.6 1.6 1.6.6-1.6.6-.6 1.6-.6-1.6-1.6-.6 1.6-.6z' fill='#fcc419'/><circle cx='2.5' cy='3' r='0.7' fill='#fcc419'/>" } else { "" };
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='20' height='20' viewBox='0 0 20 20'>\
         <ellipse cx='10' cy='18.6' rx='{shadow:.2}' ry='1.1' fill='#000' opacity='0.18'/>\
         <g transform='translate(0 {:.2}) translate(10 17.4) scale({:.3} {:.3}) translate(-10 -17)'>\
         <path d='M4 5v9.5a6 2.3 0 0 0 12 0V5' fill='#5b9bd5' stroke='#2f6fa3' stroke-width='0.8'/>\
         <ellipse cx='10' cy='5' rx='6' ry='2.3' fill='#a9d1f5' stroke='#2f6fa3' stroke-width='0.8'/>\
         <path d='M4.4 8.2a5.6 2 0 0 0 11.2 0' fill='none' stroke='#9cc7f0' stroke-width='0.7'/>\
         {eyes}<ellipse cx='6.2' cy='12.2' rx='1' ry='0.6' fill='#ff8fab' opacity='0.85'/><ellipse cx='13.8' cy='12.2' rx='1' ry='0.6' fill='#ff8fab' opacity='0.85'/>{mouth}\
         </g>{sparkle}</svg>",
        -lift,
        sx * 0.88,
        sy * 0.88
    )
}

fn bundle(s: &str) -> Option<BitmapBundle> {
    BitmapBundle::from_svg_text(s, Size::new(20, 20))
}

/// The hop frames (eyes open, then blinking) and the done pose.
pub struct Frames {
    open: Vec<BitmapBundle>,
    blink: Vec<BitmapBundle>,
    pub happy: BitmapBundle,
}

impl Frames {
    pub fn new() -> Option<Frames> {
        let h = |i: usize| (std::f32::consts::PI * i as f32 / HOP as f32).sin();
        Some(Frames {
            open: (0..HOP).map(|i| bundle(&svg(h(i), false, false))).collect::<Option<_>>()?,
            blink: (0..HOP).map(|i| bundle(&svg(h(i), true, false))).collect::<Option<_>>()?,
            happy: bundle(&svg(0.0, false, true))?,
        })
    }

    /// Frame for tick `t`: a hop every 12 ticks, a blink for 4 ticks every ~3 s.
    pub fn at(&self, t: u32) -> &BitmapBundle {
        let i = t as usize % HOP;
        if t % 120 < 4 { &self.blink[i] } else { &self.open[i] }
    }
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
            std::fs::write(format!("{dir}/hop{i:02}.svg"), super::svg(h(i), false, false)).unwrap();
        }
        std::fs::write(format!("{dir}/blink.svg"), super::svg(0.0, true, false)).unwrap();
        std::fs::write(format!("{dir}/happy.svg"), super::svg(0.0, false, true)).unwrap();
    }

    #[test]
    fn walks_back_and_forth() {
        let xs: Vec<i32> = (0..8).map(|t| super::walk(t, 3)).collect();
        assert_eq!(xs, vec![0, 1, 2, 3, 2, 1, 0, 1]);
        assert_eq!(super::walk(5, 0), 0);
    }
}
