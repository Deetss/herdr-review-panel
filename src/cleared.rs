//! Tracks which items (file or command) have been individually or section-dismissed from the
//! panel entirely - distinct from done.rs's "done" marks, which stay visible (struck through).
//! One-way and append-only, same philosophy as review.log itself - `plugin.sh clear` truncates
//! all three files together.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

pub fn path() -> PathBuf {
    std::env::var("REVIEW_PANEL_CLEARED_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".claude/review-cleared.log")
        })
}

pub fn load(path: &PathBuf) -> HashSet<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return HashSet::new();
    };
    BufReader::new(file).lines().map_while(Result::ok).collect()
}

pub fn mark(path: &PathBuf, key: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{key}");
    }
}
