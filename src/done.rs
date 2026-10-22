//! Tracks which flagged commands have been marked complete. Marking is append-only, same
//! philosophy as review.log itself - `plugin.sh clear` truncates both. Unmarking (the
//! checkbox's undo, before an item sweeps into Completed - see App::unmark_done) has no
//! append-only representation, so it rewrites the whole file from the set with that key
//! removed instead.

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

pub fn unmark(path: &PathBuf, key: &str) {
    let mut set = load(path);
    if !set.remove(key) {
        return;
    }
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
    else {
        return;
    };
    for remaining in &set {
        let _ = writeln!(file, "{remaining}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmark_removes_only_the_given_key() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();
        mark(&path, "a");
        mark(&path, "b");
        unmark(&path, "a");
        let remaining = load(&path);
        assert_eq!(remaining, HashSet::from(["b".to_string()]));
    }

    #[test]
    fn unmark_of_an_absent_key_is_a_noop() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();
        mark(&path, "a");
        unmark(&path, "does-not-exist");
        assert_eq!(load(&path), HashSet::from(["a".to_string()]));
    }
}
