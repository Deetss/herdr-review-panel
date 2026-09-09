//! Tracks which flagged commands have been marked complete. One-way (no unmarking) and
//! append-only, same philosophy as review.log itself - `plugin.sh clear` truncates both.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

pub fn path() -> PathBuf {
    std::env::var("REVIEW_PANEL_DONE_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude/review-done.log")
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
