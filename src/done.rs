//! Done events for flagged commands: `{unix_ts} done {key}` and `{unix_ts} open {key}`, replayed
//! by events.rs. Reopening appends an `open` event, so the file only ever grows.

use crate::events;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn path() -> PathBuf {
    std::env::var("REVIEW_PANEL_DONE_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude/review-done.log")
        })
}

/// Keys whose last event is `done`.
pub fn load(path: &Path) -> HashSet<String> {
    events::replay(path, "done", "open")
        .into_iter()
        .filter_map(|(key, done)| done.then_some(key))
        .collect()
}

pub fn mark(path: &Path, key: &str) {
    events::append(path, "done", key);
}

pub fn open(path: &Path, key: &str) {
    events::append(path, "open", key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn done_then_open_leaves_the_key_open() {
        let file = tempfile::NamedTempFile::new().unwrap();
        mark(file.path(), "a");
        mark(file.path(), "b");
        open(file.path(), "a");
        assert_eq!(load(file.path()), HashSet::from(["b".to_string()]));
    }

    #[test]
    fn open_never_shrinks_the_file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        mark(file.path(), "a");
        let before = std::fs::read_to_string(file.path()).unwrap();
        open(file.path(), "a");
        let after = std::fs::read_to_string(file.path()).unwrap();
        assert!(after.starts_with(&before));
        assert_eq!(after.lines().count(), 2);
    }

    #[test]
    fn legacy_untimestamped_lines_read_as_done() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "2026-09-09 11:24:32|s|echo hi").unwrap();
        file.flush().unwrap();
        assert!(load(file.path()).contains("2026-09-09 11:24:32|s|echo hi"));
    }
}
