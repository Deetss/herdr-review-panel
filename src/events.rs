//! Shared reader and writer for the append-only event logs (`review-done.log`,
//! `review-cleared.log`). A line is `{unix_ts} {verb} {key}`. The last event for a key wins,
//! in file order, so a clock-skewed second writer cannot reorder history. A line without a
//! timestamp prefix is a legacy mark and reads as the positive verb. Nothing here ever
//! rewrites or truncates a file.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn append(path: &Path, verb: &str, key: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {verb} {key}", unix_now());
    }
}

/// Splits one line into its key and whether the event is the positive verb. Keys begin with
/// a date, so a legacy line never has a bare integer as its first word.
fn parse<'a>(line: &'a str, on: &str, off: &str) -> (&'a str, bool) {
    if let Some((ts, rest)) = line.split_once(' ')
        && ts.parse::<u64>().is_ok()
        && let Some((verb, key)) = rest.split_once(' ')
    {
        if verb == on {
            return (key, true);
        }
        if verb == off {
            return (key, false);
        }
    }
    (line, true)
}

/// Replays the file into `key -> last event was the positive verb`.
pub fn replay(path: &Path, on: &str, off: &str) -> HashMap<String, bool> {
    let Ok(file) = std::fs::File::open(path) else {
        return HashMap::new();
    };
    let mut state = HashMap::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if line.is_empty() {
            continue;
        }
        let (key, positive) = parse(&line, on, off);
        state.insert(key.to_string(), positive);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_event_for_a_key_wins() {
        let file = tempfile::NamedTempFile::new().unwrap();
        append(file.path(), "done", "k");
        append(file.path(), "open", "k");
        assert!(!replay(file.path(), "done", "open")["k"]);
        append(file.path(), "done", "k");
        assert!(replay(file.path(), "done", "open")["k"]);
    }

    #[test]
    fn a_line_without_a_timestamp_reads_as_the_positive_verb() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "2026-09-09 11:24:32|s|echo hi").unwrap();
        file.flush().unwrap();
        let state = replay(file.path(), "hide", "show");
        assert!(state["2026-09-09 11:24:32|s|echo hi"]);
    }
}
