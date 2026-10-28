//! Worksheet autosave & restore, like SQL Developer reopening worksheets after a restart.
//! Every open worksheet mirrors its text into `<config>/worksheets/<id>.toml` every few
//! seconds; closing it with Cmd/Ctrl+W (after the save prompt) deletes the slot. Slots left
//! behind (tab closed with ×, app quit or crashed) are offered for recovery.

use rsqldev_core::config::config_root;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Slot {
    pub conn: String,
    pub path: Option<String>,
    pub text: String,
    /// Seconds since the Unix epoch.
    pub saved: u64,
    /// Still open when the app quit: reopened at the next start.
    #[serde(default)]
    pub open: bool,
    #[serde(skip)]
    pub file: PathBuf,
}

fn dir() -> PathBuf {
    config_root().join("worksheets")
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// Unique slot id for this process (time-based so ids never collide across runs).
pub fn next_id() -> u64 {
    let base = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    base * 100 + NEXT.fetch_add(1, Ordering::Relaxed) % 100
}

fn file(id: u64) -> PathBuf {
    dir().join(format!("{id}.toml"))
}

pub fn write(id: u64, conn: &str, path: Option<String>, text: &str) {
    write_slot(id, conn, path, text, false)
}

/// At quit: the worksheet was open, so the next start reopens it.
pub fn write_open(id: u64, conn: &str, path: Option<String>, text: &str) {
    write_slot(id, conn, path, text, true)
}

fn write_slot(id: u64, conn: &str, path: Option<String>, text: &str, open: bool) {
    if text.trim().is_empty() {
        return remove(id);
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let slot = Slot { conn: conn.into(), path, text: text.into(), saved: now, open, file: PathBuf::new() };
    if let Ok(s) = toml::to_string(&slot) {
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(file(id), s);
    }
}

pub fn remove(id: u64) {
    let _ = std::fs::remove_file(file(id));
}

pub fn remove_file(p: &std::path::Path) {
    let _ = std::fs::remove_file(p);
}

/// Recoverable worksheets, newest first.
pub fn list() -> Vec<Slot> {
    let mut v: Vec<Slot> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let mut s: Slot = toml::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            s.file = p;
            Some(s)
        })
        .collect();
    v.sort_by(|a, b| b.saved.cmp(&a.saved));
    // Same connection + same text left by several sessions: offer only the newest.
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert((s.conn.clone(), s.text.clone())));
    v
}

/// One-line label: "conn — first non-empty line".
pub fn label(s: &Slot) -> String {
    let first = s.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let first: String = first.chars().take(60).collect();
    format!("{} — {first}", s.conn)
}
