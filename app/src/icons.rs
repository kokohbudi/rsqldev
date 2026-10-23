//! Toolbar and tree icons, drawn as small SVGs in the visual language of SQL Developer
//! (green run triangle, database cylinders, yellow folders). Original artwork, not Oracle's.

use std::cell::RefCell;
use std::collections::HashMap;
use wxdragon::prelude::*;

const DB: &str = "<path d='M3 3.5C3 2.1 5.7 1 8 1s5 1.1 5 2.5v9c0 1.4-2.7 2.5-5 2.5s-5-1.1-5-2.5z' fill='#9db8d9' stroke='#3a6aa0'/><ellipse cx='8' cy='3.5' rx='5' ry='2' fill='#c9dbef' stroke='#3a6aa0'/>";
const SHEET: &str = "<path d='M3 1h7l3 3v11H3z' fill='#fff' stroke='#6b7d93'/><path d='M10 1v3h3' fill='none' stroke='#6b7d93'/>";
const RUN: &str = "<path d='M5 3l8 5-8 5z' fill='#2f9e44' stroke='#1d6b2d' stroke-linejoin='round'/>";
const GRID: &str = "<rect x='1.5' y='2.5' width='13' height='11' fill='#fff' stroke='#5a7fa8'/><rect x='1.5' y='2.5' width='13' height='3' fill='#8fb0d6' stroke='#5a7fa8'/><path d='M1.5 8.5h13M1.5 11h13M6 5.5v8M10.5 5.5v8' stroke='#9fb4cc'/>";

/// Database cylinder in an engine's colour (original shapes, not the vendors' logos).
fn cyl(fill: &str, stroke: &str, top: &str) -> String {
    format!("<path d='M2.5 3.5C2.5 2.1 5.2 1 8 1s5.5 1.1 5.5 2.5v9c0 1.4-2.7 2.5-5.5 2.5s-5.5-1.1-5.5-2.5z' fill='{fill}' stroke='{stroke}'/><ellipse cx='8' cy='3.5' rx='5.5' ry='2' fill='{top}' stroke='{stroke}'/>")
}

/// Per-engine connection icon body; `None` for an unknown name.
fn engine(name: &str) -> Option<String> {
    Some(match name {
        "oracle" => format!("{}<ellipse cx='8' cy='9.5' rx='3' ry='2' fill='none' stroke='#fff' stroke-width='1.5'/>", cyl("#d9534f", "#962d22", "#f1a097")),
        "postgres" => format!("{}<path d='M5.5 7.5c0 3 1 5 2.5 5s2.5-2 2.5-5' fill='none' stroke='#fff' stroke-width='1.4' stroke-linecap='round'/>", cyl("#3f73ad", "#1f416b", "#93b6de")),
        "mysql" => format!("{}<path d='M4.5 11c2-3 5-3 7-5' fill='none' stroke='#f29111' stroke-width='1.8' stroke-linecap='round'/>", cyl("#1d7f99", "#0d4f61", "#7fc1d3")),
        "mssql" => format!("{}<path d='M4 8.5h8M4 11h8' stroke='#e03a3e' stroke-width='1.6'/>", cyl("#6b7787", "#3e4652", "#b4bdc8")),
        "sqlite" => "<path d='M3 1h7l3 3v11H3z' fill='#d6ebfa' stroke='#2f6fa3'/><path d='M10 1v3h3' fill='none' stroke='#2f6fa3'/><path d='M5 7.5h6M5 10h6M5 12.5h4' stroke='#2f6fa3' stroke-width='1.3'/>".into(),
        "duckdb" => "<circle cx='7.5' cy='8' r='6.5' fill='#ffd43b' stroke='#a07800'/><path d='M12 7l3.5 1.5L12 10z' fill='#f76707' stroke='#b04a00' stroke-linejoin='round'/><circle cx='8.5' cy='6' r='1.1' fill='#222'/>".into(),
        "clickhouse" => "<rect x='1' y='1.5' width='14' height='13' rx='2' fill='#ffe14d' stroke='#a08a00'/><path d='M4 4v8M6.7 4v8M9.4 4v8M12 6.5v3' stroke='#222' stroke-width='1.6'/>".into(),
        _ => return None,
    })
}

/// Engine names for [`engine`] icons, by backend.
pub fn engine_name(kind: rsqldev_core::registry::Kind) -> &'static str {
    use rsqldev_core::registry::Kind::*;
    match kind {
        Oracle => "oracle",
        Postgres => "postgres",
        Mysql => "mysql",
        Mssql => "mssql",
        Sqlite => "sqlite",
        Duckdb => "duckdb",
        Clickhouse => "clickhouse",
    }
}

/// Tree icon for a connection of driver `driver_id`, open (`up`) or not.
pub fn conn_icon(driver_id: &str, up: bool) -> String {
    match rsqldev_core::registry::find(driver_id) {
        Some(d) => format!("db_{}{}", engine_name(d.backend), if up { "_up" } else { "" }),
        None => (if up { "conn_up" } else { "conn" }).into(),
    }
}

fn svg(name: &str) -> String {
    let engine_icon = name.strip_prefix("db_").and_then(|n| {
        let (n, up) = n.strip_suffix("_up").map_or((n, false), |n| (n, true));
        engine(n).map(|b| if up { format!("{b}<circle cx='12.5' cy='12.5' r='3.2' fill='#2f9e44' stroke='#fff'/>") } else { b })
    });
    let body = match name {
        _ if engine_icon.is_some() => engine_icon.unwrap(),
        "new" => "<path d='M8 2v12M2 8h12' stroke='#2f9e44' stroke-width='3' stroke-linecap='round'/>".to_string(),
        "open" => "<path d='M1.5 4h5l1 1.5h7v8h-13z' fill='#f2c94c' stroke='#b08a1e'/><path d='M1.5 13.5l2-6h12l-2 6z' fill='#f7dc80' stroke='#b08a1e'/>".into(),
        "save" => "<rect x='2' y='2' width='12' height='12' rx='1' fill='#5b7fbd' stroke='#2f4d80'/><rect x='4.5' y='2.5' width='7' height='4' fill='#fff'/><rect x='4' y='9' width='8' height='4.5' fill='#dfe7f3'/>".into(),
        "saveall" => "<rect x='1' y='1' width='10' height='10' rx='1' fill='#8aa6d4' stroke='#2f4d80'/><rect x='5' y='5' width='10' height='10' rx='1' fill='#5b7fbd' stroke='#2f4d80'/><rect x='7' y='5.5' width='6' height='3' fill='#fff'/>".into(),
        "back" => "<path d='M10 3L5 8l5 5' fill='none' stroke='#3a6aa0' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'/>".into(),
        "forward" => "<path d='M6 3l5 5-5 5' fill='none' stroke='#3a6aa0' stroke-width='2.5' stroke-linecap='round' stroke-linejoin='round'/>".into(),
        "worksheet" => format!("{SHEET}<path d='M5 6h6M5 8.5h6M5 11h4' stroke='#3a6aa0' stroke-width='1.2'/><path d='M4.5 13.5h7' stroke='#c0392b' stroke-width='1.5'/>"),
        "run" => RUN.into(),
        "script" => format!("{SHEET}<path d='M5 6h5M5 8.5h5M5 11h3' stroke='#8a9bb0'/><path d='M8.5 8l6 3.5-6 3.5z' fill='#2f9e44' stroke='#1d6b2d' stroke-linejoin='round'/>"),
        "explain" => "<rect x='1' y='1.5' width='5' height='3.5' fill='#8fb0d6' stroke='#3a6aa0'/><rect x='6' y='6.5' width='5' height='3.5' fill='#c9dbef' stroke='#3a6aa0'/><rect x='10' y='11.5' width='5' height='3.5' fill='#c9dbef' stroke='#3a6aa0'/><path d='M3.5 5v3.2H6M8.5 10v3.2H10' fill='none' stroke='#3a6aa0'/>".into(),
        "autotrace" => format!("{RUN}<path d='M1.5 14.5h13' stroke='#c0392b' stroke-width='1.5'/>"),
        "commit" => format!("{DB}<circle cx='11.5' cy='11.5' r='4' fill='#2f9e44'/><path d='M9.5 11.5l1.5 1.5 2.7-3' fill='none' stroke='#fff' stroke-width='1.5'/>"),
        "rollback" => format!("{DB}<circle cx='11.5' cy='11.5' r='4' fill='#d9480f'/><path d='M13.5 12.5a2.2 2.2 0 1 1-1-3H10m0 0l1.4-1.4M10 9.5l1.4 1.4' fill='none' stroke='#fff' stroke-width='1.2'/>"),
        "cancel" => "<rect x='3' y='3' width='10' height='10' rx='1.5' fill='#e03131' stroke='#a61e1e'/>".into(),
        "unshared" => format!("{SHEET}<circle cx='11.5' cy='11' r='3.5' fill='#f08c00' stroke='#a86200'/><path d='M11.5 9v2.3l1.5 1' stroke='#fff' fill='none'/>"),
        "case" => "<path d='M1 13.5L4.5 3h1L9 13.5M2.4 10h5.2' fill='none' stroke='#3a6aa0' stroke-width='1.6' stroke-linejoin='round'/><circle cx='12' cy='11' r='2.3' fill='none' stroke='#3a6aa0' stroke-width='1.4'/><path d='M14.3 8.5v5' stroke='#3a6aa0' stroke-width='1.4'/>".into(),
        "clear" => "<path d='M2 11l6-7 6 5-4 5H5z' fill='#f4a6b8' stroke='#b04060'/><path d='M5 14h9' stroke='#6b7d93'/><path d='M5 7.5l5 4' stroke='#b04060'/>".into(),
        "history" => "<circle cx='8' cy='8' r='6.5' fill='#fff' stroke='#3a6aa0'/><path d='M8 4v4l3 2' fill='none' stroke='#3a6aa0' stroke-width='1.5' stroke-linecap='round'/>".into(),
        "refresh" => "<path d='M13 8a5 5 0 1 1-1.5-3.6' fill='none' stroke='#2f9e44' stroke-width='2'/><path d='M12.5 1.5v4h-4' fill='none' stroke='#2f9e44' stroke-width='2' stroke-linejoin='round'/>".into(),
        "filter" => "<path d='M1.5 2h13l-5 6v5l-3 1.5V8z' fill='#c9dbef' stroke='#3a6aa0' stroke-linejoin='round'/>".into(),
        "collapse" => "<path d='M4 8l4-4 4 4M4 13l4-4 4 4' fill='none' stroke='#3a6aa0' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'/>".into(),
        "pin" => "<path d='M6 1.5h4l-.5 5 2.5 2H4l2.5-2z' fill='#9db8d9' stroke='#3a6aa0'/><path d='M8 8.5v6' stroke='#3a6aa0' stroke-width='1.5'/>".into(),
        "insertrow" => "<path d='M1.5 3h8M1.5 6.5h8M1.5 10h5' stroke='#5a7fa8' stroke-width='1.6'/><circle cx='11.5' cy='11.5' r='4.5' fill='#2f9e44'/><path d='M11.5 8.8v5.4M8.8 11.5h5.4' stroke='#fff' stroke-width='1.8'/>".into(),
        "deleterow" => "<path d='M1.5 3h8M1.5 6.5h8M1.5 10h5' stroke='#5a7fa8' stroke-width='1.6'/><circle cx='11.5' cy='11.5' r='4.5' fill='#e03131'/><path d='M9.6 9.6l3.8 3.8M13.4 9.6l-3.8 3.8' stroke='#fff' stroke-width='1.8'/>".into(),
        "sort" => "<path d='M4 2v11M1.5 10.5L4 13l2.5-2.5' fill='none' stroke='#3a6aa0' stroke-width='1.5'/><path d='M8 4h7M8 8h5M8 12h3' stroke='#3a6aa0' stroke-width='1.5'/>".into(),
        "actions" => "<rect x='1.5' y='3' width='13' height='10' rx='1.5' fill='#eef2f7' stroke='#6b7d93'/><path d='M5 7l3 3 3-3' fill='none' stroke='#3a6aa0' stroke-width='1.5'/>".into(),
        "export" => format!("{SHEET}<path d='M8 13V7M5.5 9.5L8 7l2.5 2.5' fill='none' stroke='#2f9e44' stroke-width='1.6'/>"),
        "drivers" => "<rect x='2' y='4' width='12' height='8' rx='1' fill='#c9dbef' stroke='#3a6aa0'/><path d='M4 12v2.5M7 12v2.5M9 12v2.5M12 12v2.5M5 7h6' stroke='#3a6aa0'/>".into(),
        // ---- tree nodes ----
        "conn" => DB.into(),
        "conn_up" => format!("{DB}<circle cx='12' cy='12' r='3.5' fill='#2f9e44' stroke='#fff'/>"),
        "folder" => "<path d='M1.5 3.5h5l1.2 1.5h6.8v8.5h-13z' fill='#f2c94c' stroke='#b08a1e'/>".into(),
        "table" => GRID.into(),
        "view" => "<rect x='1.5' y='2.5' width='13' height='11' fill='#fff' stroke='#7a63a8'/><rect x='1.5' y='2.5' width='13' height='3' fill='#c3b3e3' stroke='#7a63a8'/><circle cx='5.5' cy='10' r='2' fill='none' stroke='#7a63a8'/><circle cx='10.5' cy='10' r='2' fill='none' stroke='#7a63a8'/><path d='M7.5 10h1' stroke='#7a63a8'/>".into(),
        "mview" => format!("{GRID}<circle cx='12' cy='12' r='3.5' fill='#fff' stroke='#7a63a8'/><path d='M12 10v2l1.4 1' stroke='#7a63a8' fill='none'/>"),
        "index" => "<path d='M9 1L3 9h4l-1 6 6-8H8z' fill='#f2c94c' stroke='#b08a1e' stroke-linejoin='round'/>".into(),
        "package" => "<path d='M8 1.5l6 3v7l-6 3-6-3v-7z' fill='#e6c79c' stroke='#8a6334' stroke-linejoin='round'/><path d='M2 4.5l6 3 6-3M8 7.5v7' fill='none' stroke='#8a6334'/>".into(),
        "procedure" => "<circle cx='8' cy='8' r='4.2' fill='#9db8d9' stroke='#3a6aa0' stroke-width='2.5' stroke-dasharray='2.2 1.1'/><circle cx='8' cy='8' r='1.6' fill='#fff' stroke='#3a6aa0'/>".into(),
        "function" => "<path d='M9.5 2c-2 0-2.6 1-3 3L5 12.5C4.6 14.3 4 15 2.5 15M3.5 6.5h5' fill='none' stroke='#3a6aa0' stroke-width='1.6' stroke-linecap='round'/><path d='M10 8l4 5M14 8l-4 5' stroke='#c0392b' stroke-width='1.4'/>".into(),
        "trigger" => "<path d='M9 1L3 9h4l-1 6 6-8H8z' fill='#ff922b' stroke='#b35c00' stroke-linejoin='round'/>".into(),
        "type" => "<rect x='2' y='2' width='12' height='12' rx='2' fill='#d0ebff' stroke='#3a6aa0'/><path d='M5 5.5h6M8 5.5v6' stroke='#3a6aa0' stroke-width='1.8'/>".into(),
        "sequence" => "<rect x='2' y='9' width='3' height='5' fill='#9db8d9' stroke='#3a6aa0'/><rect x='6.5' y='6' width='3' height='8' fill='#9db8d9' stroke='#3a6aa0'/><rect x='11' y='2.5' width='3' height='11.5' fill='#9db8d9' stroke='#3a6aa0'/>".into(),
        "synonym" => "<path d='M2 10a4 4 0 0 1 4-4h6' fill='none' stroke='#3a6aa0' stroke-width='1.8'/><path d='M10 3l3 3-3 3' fill='none' stroke='#3a6aa0' stroke-width='1.8'/>".into(),
        "dblink" => "<rect x='1' y='5.5' width='7' height='5' rx='2.5' fill='none' stroke='#3a6aa0' stroke-width='1.8'/><rect x='8' y='5.5' width='7' height='5' rx='2.5' fill='none' stroke='#3a6aa0' stroke-width='1.8'/>".into(),
        "user" => "<circle cx='8' cy='5' r='3' fill='#f2c94c' stroke='#8a6334'/><path d='M2.5 15c0-3.5 2.5-5.5 5.5-5.5s5.5 2 5.5 5.5z' fill='#5b7fbd' stroke='#2f4d80'/>".into(),
        "column" => "<rect x='5' y='1.5' width='6' height='13' fill='#fff' stroke='#5a7fa8'/><rect x='5' y='1.5' width='6' height='3' fill='#8fb0d6' stroke='#5a7fa8'/>".into(),
        _ => "<rect x='3' y='3' width='10' height='10' fill='#ccc'/>".into(),
    };
    // Dark appearance: lift the navy strokes so line icons keep their contrast.
    let body = if crate::theme::is_dark() {
        [("#3a6aa0", "#8ab8ec"), ("#5a7fa8", "#94b6dc"), ("#6b7d93", "#a9b7c8"), ("#2f4d80", "#9fbde6"), ("#7a63a8", "#b9a6e6"), ("#8a6334", "#d1a66b"), ("#b08a1e", "#d9b443")]
            .iter()
            .fold(body, |b, (from, to)| b.replace(from, to))
    } else {
        body
    };
    format!("<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'>{body}</svg>")
}

thread_local! {
    static CACHE: RefCell<HashMap<(String, i32), Bitmap>> = RefCell::new(HashMap::new());
}

/// Bitmap at `px` pixels (pass 32 for crisp toolbar icons on Retina).
pub fn bitmap(name: &str, px: i32) -> Option<Bitmap> {
    CACHE.with(|c| {
        if let Some(b) = c.borrow().get(&(name.to_string(), px)) {
            return Some(b.clone());
        }
        let b = BitmapBundle::from_svg_text(&svg(name), Size::new(16, 16))?.get_bitmap(Size::new(px, px))?;
        c.borrow_mut().insert((name.to_string(), px), b.clone());
        Some(b)
    })
}

/// Flat, borderless icon button with a tooltip — SQL Developer style toolbar item.
pub fn tool(parent: &dyn WxWidget, name: &str, tip: &str) -> BitmapButton {
    let b = BitmapButton::builder(parent).with_bitmap(bitmap(name, 16)).with_style(BitmapButtonStyle::BorderNone | BitmapButtonStyle::ExactFit).build();
    b.set_tooltip(tip);
    b
}

/// Tree image list; index order = [`TREE`].
pub const TREE: &[&str] = &[
    "conn", "conn_up", "folder", "table", "view", "mview", "index", "package", "procedure", "function", "trigger", "type", "sequence", "synonym", "dblink", "user",
    "column", "db_oracle", "db_oracle_up", "db_postgres", "db_postgres_up", "db_mysql", "db_mysql_up", "db_mssql", "db_mssql_up", "db_sqlite", "db_sqlite_up",
    "db_duckdb", "db_duckdb_up", "db_clickhouse", "db_clickhouse_up",
];

pub fn tree_index(name: &str) -> i32 {
    TREE.iter().position(|n| *n == name).map_or(-1, |i| i as i32)
}

pub fn tree_images() -> ImageList {
    let il = ImageList::new(16, 16, true, TREE.len() as i32);
    for n in TREE {
        if let Some(b) = bitmap(n, 16) {
            il.add_bitmap(&b);
        }
    }
    il
}

/// Icon for an object kind folder / node.
pub fn for_kind(kind: &str) -> &'static str {
    use rsqldev_core::dialect::*;
    match kind {
        TABLE => "table",
        VIEW => "view",
        MVIEW => "mview",
        INDEX => "index",
        PACKAGE => "package",
        PROCEDURE => "procedure",
        FUNCTION => "function",
        TRIGGER => "trigger",
        TYPE => "type",
        SEQUENCE => "sequence",
        SYNONYM => "synonym",
        DBLINK => "dblink",
        _ => "folder",
    }
}
