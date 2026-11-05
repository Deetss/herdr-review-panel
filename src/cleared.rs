//! Hide events for flagged items: `{unix_ts} hide {key}` and `{unix_ts} show {key}`, replayed
//! by events.rs. Hiding is distinct from done.rs's marks, and restoring appends a `show`, so
//! the file only ever grows.

use crate::events;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn path() -> PathBuf {
    std::env::var("REVIEW_PANEL_CLEARED_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".claude/review-cleared.log")
        })
}

/// `key -> true` when the last event is `hide`, `false` when it is `show`. Keys with no
/// event are absent, which matters: a `show` overrides supersession, silence does not.
pub fn load(path: &Path) -> HashMap<String, bool> {
    events::replay(path, "hide", "show")
}

pub fn hide(path: &Path, key: &str) {
    events::append(path, "hide", key);
}

pub fn show(path: &Path, key: &str) {
    events::append(path, "show", key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn hide_then_show_leaves_the_key_visible() {
        let file = tempfile::NamedTempFile::new().unwrap();
        hide(file.path(), "a");
        hide(file.path(), "b");
        show(file.path(), "a");
        let state = load(file.path());
        assert!(!state["a"]);
        assert!(state["b"]);
    }

    #[test]
    fn legacy_untimestamped_lines_read_as_hidden() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "2026-09-09 11:24:32|s|a.txt").unwrap();
        file.flush().unwrap();
        assert!(load(file.path())["2026-09-09 11:24:32|s|a.txt"]);
    }
}
