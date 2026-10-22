//! Large values stay out of RAM. A cell longer than [`INLINE`] characters is written to a temp
//! file as it is fetched; the grid keeps a short marker shown as `(BLOB)` / `(CLOB)` and the value
//! is read back only when it is viewed, saved, exported or used in DML.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Cells up to this many characters stay in memory (32 KB of binary as hex).
pub const INLINE: usize = 64 * 1024;
const TAG: char = '\u{1}';
static NEXT: AtomicU64 = AtomicU64::new(1);

fn dir() -> PathBuf {
    std::env::temp_dir().join(format!("rsqldev-lobs-{}", std::process::id()))
}

fn path(id: u64) -> PathBuf {
    dir().join(format!("{id}.bin"))
}

/// `5.2 MB` style size.
pub fn size(n: usize) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} bytes"),
    }
}

/// Spill raw bytes; `kind` labels the marker (BLOB, CLOB, TEXT).
pub fn spill(bytes: &[u8], kind: &str) -> Option<String> {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    std::fs::create_dir_all(dir()).ok()?;
    std::fs::write(path(id), bytes).ok()?;
    Some(format!("{TAG}{id}{TAG}({kind})"))
}

/// Keep a fetched cell in memory, or spill it when large. Binary cells arrive as hex text
/// (`0x…`, `\x…` or bare): they are stored decoded, so a 5 MB BLOB costs 5 MB of disk, no RAM.
pub fn keep(s: String, binary: bool) -> String {
    if s.len() <= INLINE {
        return s;
    }
    let spilled = if binary {
        let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("\\x")).unwrap_or(&s);
        match crate::hex_decode(h) {
            Some(b) => spill(&b, "BLOB"),
            None => spill(s.as_bytes(), "TEXT"),
        }
    } else {
        spill(s.as_bytes(), "CLOB")
    };
    spilled.unwrap_or(s)
}

fn id(s: &str) -> Option<u64> {
    s.strip_prefix(TAG)?.split_once(TAG)?.0.parse().ok()
}

pub fn is_lob(s: &str) -> bool {
    id(s).is_some()
}

/// LOB column types the grid always shows as `(BLOB)`, like SQL Developer; short binary types
/// (RAW, BINARY, VARBINARY(n)) keep showing their hex.
pub fn is_blob_type(type_name: &str) -> bool {
    let t = type_name.to_ascii_uppercase();
    ["BLOB", "BYTEA", "IMAGE", "LONG RAW", "LONG_RAW"].iter().any(|k| t.contains(k))
}

/// Grid text for a cell of a column of type `type_name`.
pub fn grid_text<'a>(type_name: &str, s: &'a str) -> &'a str {
    if is_blob_type(type_name) && !is_lob(s) { "(BLOB)" } else { display(s) }
}

/// Text to show in the grid: the marker label for spilled cells.
pub fn display(s: &str) -> &str {
    match id(s) {
        Some(_) => s[1..].split_once(TAG).map_or(s, |x| x.1),
        None => s,
    }
}

/// Bytes of a spilled cell.
pub fn bytes(s: &str) -> Option<Vec<u8>> {
    std::fs::read(path(id(s)?)).ok()
}

/// Full value as text for exports: spilled BLOBs as `0x` hex, spilled text as itself.
pub fn expand(s: &str) -> Cow<'_, str> {
    match bytes(s) {
        Some(b) if display(s).starts_with("(BLOB") => Cow::Owned(format!("0x{}", crate::hex(&b))),
        Some(b) => Cow::Owned(String::from_utf8_lossy(&b).into_owned()),
        None => Cow::Borrowed(s),
    }
}

/// Delete a spilled cell's file (grid cleared or closed).
pub fn release(s: &str) {
    if let Some(id) = id(s) {
        let _ = std::fs::remove_file(path(id));
    }
}

/// Remove spill folders left by earlier runs (crashes) after a day.
pub fn cleanup_stale() {
    let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else { return };
    let mine = dir();
    for e in rd.flatten() {
        let p = e.path();
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() > Duration::from_secs(86_400));
        if p != mine && old && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rsqldev-lobs-")) {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

/// Remove this run's spill folder (on exit).
pub fn cleanup_mine() {
    let _ = std::fs::remove_dir_all(dir());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spill_round_trip() {
        let small = "0xFF".to_string();
        assert_eq!(keep(small.clone(), true), small);
        let data: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        let cell = keep(format!("0x{}", crate::hex(&data)), true);
        assert!(cell.len() < 100 && is_lob(&cell), "{cell:?}");
        assert_eq!(display(&cell), "(BLOB)");
        assert_eq!(size(100_000), "97.7 KB");
        assert_eq!(bytes(&cell).unwrap(), data);
        assert!(expand(&cell).starts_with("0x00010203"));
        let text = "é".repeat(40_000);
        let t = keep(text.clone(), false);
        assert_eq!(display(&t), "(CLOB)");
        assert_eq!(expand(&t), text);
        release(&cell);
        assert!(bytes(&cell).is_none());
        assert_eq!(display("plain"), "plain");
    }
}
