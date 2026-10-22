//! Large values stay out of RAM. A cell longer than [`INLINE`] characters is written to a temp
//! file as it is fetched; the grid keeps a short marker shown as `(BLOB 5.2 MB)` / `(CLOB 1.1 MB)`
//! and the value is read back only when it is viewed, saved, exported or used in DML.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Cells up to this many characters stay in memory (32 KB of binary as hex).
pub const INLINE: usize = 64 * 1024;
const TAG: char = '\u{1}';
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Random per-process token inside every marker: a fetched value that happens to start with
/// TAG cannot pass for a spill marker (and so cannot read another cell's file).
fn secret() -> &'static str {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos());
        format!("{:016x}", h.finish())
    })
}

/// This run's spill folder: random name, owner-only (0700), created fresh. An existing folder
/// is never reused, so another user on a shared /tmp cannot pre-create or watch it.
fn dir() -> Option<&'static PathBuf> {
    static D: OnceLock<Option<PathBuf>> = OnceLock::new();
    D.get_or_init(|| {
        let p = std::env::temp_dir().join(format!("rsqldev-lobs-{}", secret()));
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(&p).ok().map(|_| p)
    })
    .as_ref()
}

fn path(id: u64) -> Option<PathBuf> {
    dir().map(|d| d.join(format!("{id}.bin")))
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
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(path(id)?).ok()?, bytes).ok()?;
    // Size comes free here: the bytes are already in hand.
    Some(format!("{TAG}{}:{id}{TAG}({kind} {})", secret(), size(bytes.len())))
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
    let (sec, id) = s.strip_prefix(TAG)?.split_once(TAG)?.0.split_once(':')?;
    if sec != secret() {
        return None;
    }
    id.parse().ok()
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

/// Grid text for a cell of a column of type `type_name`. An in-memory BLOB's size is its hex
/// length / 2: no decoding, no extra query.
pub fn grid_text<'a>(type_name: &str, s: &'a str) -> Cow<'a, str> {
    // Already a label (a file staged by Load Value): keep it.
    if is_blob_type(type_name) && !is_lob(s) && !s.starts_with("(BLOB") {
        let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("\\x")).unwrap_or(s);
        Cow::Owned(format!("(BLOB {})", size(hex.len() / 2)))
    } else {
        Cow::Borrowed(display(s))
    }
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
    std::fs::read(path(id(s)?)?).ok()
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
    if let Some(p) = id(s).and_then(path) {
        let _ = std::fs::remove_file(p);
    }
}

/// Remove spill folders left by earlier runs (crashes) after a day.
pub fn cleanup_stale() {
    let Ok(rd) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| SystemTime::now().duration_since(t).unwrap_or_default() > Duration::from_secs(86_400));
        // A day old: never this run's folder; other users' folders fail to delete (not ours).
        if old && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rsqldev-lobs-")) {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

/// Remove this run's spill folder (on exit).
pub fn cleanup_mine() {
    if let Some(d) = dir() {
        let _ = std::fs::remove_dir_all(d);
    }
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
        assert_eq!(display(&cell), "(BLOB 97.7 KB)");
        assert_eq!(grid_text("BLOB", "0x0102"), "(BLOB 2 bytes)");
        assert_eq!(grid_text("RAW(16)", "0102"), "0102");
        assert_eq!(bytes(&cell).unwrap(), data);
        assert!(expand(&cell).starts_with("0x00010203"));
        let text = "é".repeat(40_000);
        let t = keep(text.clone(), false);
        assert_eq!(display(&t), "(CLOB 78.1 KB)");
        assert_eq!(expand(&t), text);
        release(&cell);
        assert!(bytes(&cell).is_none());
        assert_eq!(display("plain"), "plain");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        // Data that imitates a marker is just data.
        let fake = format!("{TAG}0000000000000000:1{TAG}(BLOB 1 KB)");
        assert!(!is_lob(&fake) && bytes(&fake).is_none() && display(&fake) == fake);
    }
}
