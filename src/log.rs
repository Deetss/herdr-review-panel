//! Parsing and tailing of `review.log`, written by `~/.claude/hooks/review-notify.sh`.
//! Each line looks like:
//!   2026-09-09 11:24:32 session=... repo=NAME [cwd=PATH] item=VALUE

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub struct ParsedLine {
    pub ts: String,
    pub repo: String,
    pub cwd: Option<String>,
    pub item: String,
}

/// A single row of the rendered display. Blank and GroupHeader rows are not clickable.
pub enum Row {
    Blank,
    GroupHeader {
        ts: String,
        repo: String,
    },
    Item {
        label: String,
        abspath: Option<String>,
    },
}

pub fn parse_line(line: &str) -> Option<ParsedLine> {
    let mut parts = line.splitn(3, ' ');
    let date = parts.next()?;
    let time = parts.next()?;
    let rest = parts.next().unwrap_or("");
    if date.is_empty() || time.is_empty() {
        return None;
    }
    let repo = extract_field(rest, "repo=")?;
    let cwd = extract_field(rest, "cwd=");
    let item = extract_rest(rest, "item=")?;
    Some(ParsedLine {
        ts: format!("{date} {time}"),
        repo,
        cwd,
        item,
    })
}

fn extract_field(s: &str, key: &str) -> Option<String> {
    let idx = s.find(key)?;
    let after = &s[idx + key.len()..];
    let end = after.find(' ').unwrap_or(after.len());
    Some(after[..end].to_string())
}

fn extract_rest(s: &str, key: &str) -> Option<String> {
    let idx = s.find(key)?;
    Some(s[idx + key.len()..].to_string())
}

/// Older log lines predate the `cwd=` field, and items can be absolute, `~`-relative, or
/// relative to the hook's cwd at the time it fired - mirrors review-notify.sh's own expansion.
pub fn resolve_abspath(item: &str, cwd: Option<&str>, home: &str) -> Option<String> {
    if item.starts_with('/') {
        Some(item.to_string())
    } else if item == "~" || item.starts_with("~/") {
        Some(format!("{home}{}", &item[1..]))
    } else {
        cwd.map(|cwd| format!("{cwd}/{item}"))
    }
}

/// Turns one parsed log line into its display row(s), appending to `rows` and tracking
/// `last_key` across calls so consecutive items sharing a timestamp+repo group under one
/// header instead of repeating it per item - same rule review-notify.sh's writer follows
/// (one hook invocation, one timestamp, possibly several `<user_review>` items).
pub fn append_row(
    rows: &mut Vec<Row>,
    last_key: &mut Option<(String, String)>,
    line: &ParsedLine,
    home: &str,
) {
    let key = (line.ts.clone(), line.repo.clone());
    if last_key.as_ref() != Some(&key) {
        rows.push(Row::Blank);
        rows.push(Row::GroupHeader {
            ts: line.ts.clone(),
            repo: line.repo.clone(),
        });
        *last_key = Some(key);
    }
    let abspath = resolve_abspath(&line.item, line.cwd.as_deref(), home);
    rows.push(Row::Item {
        label: line.item.clone(),
        abspath,
    });
}

/// Reads every line currently in the log whose timestamp is >= cutoff ("YYYY-MM-DD HH:MM:SS",
/// which sorts correctly as a plain string). Returns the byte offset to resume tailing from.
pub fn backfill(
    path: &Path,
    cutoff: &str,
    rows: &mut Vec<Row>,
    last_key: &mut Option<(String, String)>,
    home: &str,
) -> u64 {
    let Ok(file) = File::open(path) else { return 0 };
    let reader = BufReader::new(file);
    let mut offset = 0u64;
    for line in reader.lines().map_while(Result::ok) {
        offset += line.len() as u64 + 1;
        let Some(parsed) = parse_line(&line) else {
            continue;
        };
        if parsed.ts.as_str() >= cutoff {
            append_row(rows, last_key, &parsed, home);
        }
    }
    offset
}

/// Reads whatever's been appended to the log since `offset`, returning the new offset. Only
/// complete (newline-terminated) lines are consumed; a partial trailing line is left for the
/// next poll rather than risking a half-written row from a concurrent hook append.
pub fn poll_tail(
    path: &Path,
    offset: u64,
    rows: &mut Vec<Row>,
    last_key: &mut Option<(String, String)>,
    home: &str,
) -> u64 {
    let Ok(mut file) = File::open(path) else {
        return offset;
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return offset;
    };
    if len <= offset {
        return offset;
    }
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return offset;
    }
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return offset;
    }
    let mut consumed = 0u64;
    for line in buf.split_inclusive('\n') {
        if !line.ends_with('\n') {
            break; // partial line - wait for the rest next poll
        }
        consumed += line.len() as u64;
        if let Some(parsed) = parse_line(line.trim_end_matches('\n')) {
            append_row(rows, last_key, &parsed, home);
        }
    }
    offset + consumed
}
