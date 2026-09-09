//! Parsing and tailing of `review.log`, written by `~/.claude/hooks/review-notify.sh`.
//! Each line looks like:
//!   2026-09-09 11:24:32 session=... repo=NAME [cwd=PATH] kind=review|command [step=LABEL] [warn=REASON] item=VALUE
//! `kind=` is absent on lines written before commands existed - those are always `review`.
//! `step=` is only present on commands the reply explicitly ordered (<user_command step="2a">).
//! `warn=` is only present on commands review-notify.sh's prose heuristic flagged as reading
//! like a paraphrased task rather than a real shell command.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub struct ParsedLine {
    pub ts: String,
    pub session: String,
    pub repo: String,
    pub cwd: Option<String>,
    pub is_command: bool,
    pub step: Option<String>,
    /// Set when review-notify.sh's prose heuristic flagged this command as reading like a
    /// paraphrased task rather than a real shell command (see `warn=` in the module doc).
    pub warn: Option<String>,
    pub item: String,
}

/// A single row of the rendered display. Blank and GroupHeader rows are not clickable and
/// carry no key - they're never individually cleared, only pruned once every item under them
/// already has been (see app.rs's clear_cursor/prune_empty_groups).
pub enum Row {
    Blank,
    GroupHeader {
        ts: String,
        repo: String,
    },
    /// `key` identifies this exact log entry for the cleared-marks file - see cleared.rs.
    FileItem {
        label: String,
        abspath: Option<String>,
        key: String,
    },
    /// `key` identifies this exact log entry for the done-marks and cleared-marks files.
    CommandItem {
        command: String,
        step: Option<String>,
        /// Some(reason) when the prose heuristic flagged this command - currently always
        /// "prose" but kept as a string in case other heuristics are added later.
        warn: Option<String>,
        key: String,
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
    let session = extract_field(rest, "session=").unwrap_or_default();
    let repo = extract_field(rest, "repo=")?;
    let cwd = extract_field(rest, "cwd=");
    let is_command = extract_field(rest, "kind=").as_deref() == Some("command");
    let step = extract_field(rest, "step=");
    let warn = extract_field(rest, "warn=");
    let item = extract_rest(rest, "item=")?;
    Some(ParsedLine {
        ts: format!("{date} {time}"),
        session,
        repo,
        cwd,
        is_command,
        step,
        warn,
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
/// (one hook invocation, one timestamp, possibly several flagged items). Skips items already
/// in `cleared` so a restart's backfill doesn't resurrect something dismissed last session.
pub fn append_row(
    rows: &mut Vec<Row>,
    last_key: &mut Option<(String, String)>,
    line: &ParsedLine,
    home: &str,
    cleared: &HashSet<String>,
) {
    let key = (line.ts.clone(), line.repo.clone());
    let item_key = format!("{}|{}|{}", line.ts, line.session, line.item);
    if cleared.contains(&item_key) {
        // Deliberately leave `last_key` untouched: recording this group as "already seen"
        // would suppress the header for the *next* (non-cleared) item in the same group,
        // since it would then look like a header was already emitted when it wasn't.
        return;
    }
    let is_new_group = last_key.as_ref() != Some(&key);
    if is_new_group {
        rows.push(Row::Blank);
        rows.push(Row::GroupHeader {
            ts: line.ts.clone(),
            repo: line.repo.clone(),
        });
        *last_key = Some(key);
    }
    if line.is_command {
        rows.push(Row::CommandItem {
            command: line.item.clone(),
            step: line.step.clone(),
            warn: line.warn.clone(),
            key: item_key,
        });
    } else {
        let abspath = resolve_abspath(&line.item, line.cwd.as_deref(), home);
        rows.push(Row::FileItem {
            label: line.item.clone(),
            abspath,
            key: item_key,
        });
    }
}

/// Reads every line currently in the log whose timestamp is >= cutoff ("YYYY-MM-DD HH:MM:SS",
/// which sorts correctly as a plain string). Returns the byte offset to resume tailing from.
pub fn backfill(
    path: &Path,
    cutoff: &str,
    rows: &mut Vec<Row>,
    last_key: &mut Option<(String, String)>,
    home: &str,
    cleared: &HashSet<String>,
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
            append_row(rows, last_key, &parsed, home, cleared);
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
    cleared: &HashSet<String>,
) -> u64 {
    let Ok(mut file) = File::open(path) else {
        return offset;
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return offset;
    };
    // A shorter file than our last-known offset means it was truncated (`plugin.sh clear`,
    // triggered externally or from this panel's own "clear everything" button) rather than
    // just having nothing new yet - restart from the top instead of a stale offset that would
    // otherwise never satisfy `len <= offset` again and silently stop noticing new content.
    let offset = if len < offset {
        rows.clear();
        *last_key = None;
        0
    } else {
        offset
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
            append_row(rows, last_key, &parsed, home, cleared);
        }
    }
    offset + consumed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn parses_a_review_line_with_no_kind_field() {
        let line = "2026-09-09 11:24:32 session=abc repo=herdr-review-panel cwd=/home/d item=herdr-plugin.toml";
        let parsed = parse_line(line).unwrap();
        assert_eq!(parsed.ts, "2026-09-09 11:24:32");
        assert_eq!(parsed.session, "abc");
        assert_eq!(parsed.repo, "herdr-review-panel");
        assert_eq!(parsed.cwd.as_deref(), Some("/home/d"));
        assert!(!parsed.is_command);
        assert_eq!(parsed.step, None);
        assert_eq!(parsed.warn, None);
        assert_eq!(parsed.item, "herdr-plugin.toml");
    }

    #[test]
    fn parses_a_command_line_with_a_step_label() {
        let line = "2026-09-09 12:00:00 session=abc repo=deetss cwd=/home/d kind=command step=2a item=sudo apt update";
        let parsed = parse_line(line).unwrap();
        assert!(parsed.is_command);
        assert_eq!(parsed.step.as_deref(), Some("2a"));
        assert_eq!(parsed.warn, None);
        assert_eq!(parsed.item, "sudo apt update");
    }

    #[test]
    fn parses_a_command_line_flagged_by_the_prose_heuristic() {
        let line = "2026-09-09 12:00:00 session=abc repo=deetss cwd=/home/d kind=command warn=prose item=redeploy the app in Dokploy";
        let parsed = parse_line(line).unwrap();
        assert!(parsed.is_command);
        assert_eq!(parsed.warn.as_deref(), Some("prose"));
        assert_eq!(parsed.item, "redeploy the app in Dokploy");
    }

    #[test]
    fn item_captures_the_rest_of_the_line_including_spaces_and_pipes() {
        let line = "2026-09-09 12:00:00 session=abc repo=deetss cwd=/home/d kind=command item=echo hi | sudo tee /x";
        let parsed = parse_line(line).unwrap();
        assert_eq!(parsed.item, "echo hi | sudo tee /x");
    }

    #[test]
    fn rejects_a_line_missing_item() {
        let line = "2026-09-09 12:00:00 session=abc repo=deetss cwd=/home/d kind=command";
        assert!(parse_line(line).is_none());
    }

    #[test]
    fn rejects_a_line_with_too_few_fields() {
        assert!(parse_line("2026-09-09").is_none());
        assert!(parse_line("").is_none());
    }

    #[test]
    fn resolves_absolute_paths_unchanged() {
        assert_eq!(
            resolve_abspath("/etc/hosts", Some("/home/d"), "/home/d"),
            Some("/etc/hosts".to_string())
        );
    }

    #[test]
    fn resolves_tilde_paths_against_home() {
        assert_eq!(
            resolve_abspath("~", None, "/home/d"),
            Some("/home/d".to_string())
        );
        assert_eq!(
            resolve_abspath("~/.ssh/config", None, "/home/d"),
            Some("/home/d/.ssh/config".to_string())
        );
    }

    #[test]
    fn resolves_relative_paths_against_cwd() {
        assert_eq!(
            resolve_abspath("foo.txt", Some("/repo"), "/home/d"),
            Some("/repo/foo.txt".to_string())
        );
    }

    #[test]
    fn relative_path_with_no_cwd_has_no_abspath() {
        assert_eq!(resolve_abspath("foo.txt", None, "/home/d"), None);
    }

    fn line(
        ts: &str,
        session: &str,
        repo: &str,
        is_command: bool,
        step: Option<&str>,
        item: &str,
    ) -> ParsedLine {
        ParsedLine {
            ts: ts.to_string(),
            session: session.to_string(),
            repo: repo.to_string(),
            cwd: Some("/home/d".to_string()),
            is_command,
            step: step.map(str::to_string),
            warn: None,
            item: item.to_string(),
        }
    }

    #[test]
    fn groups_consecutive_items_sharing_timestamp_and_repo() {
        let mut rows = Vec::new();
        let mut last_key = None;
        let cleared = HashSet::new();
        append_row(
            &mut rows,
            &mut last_key,
            &line("t1", "s1", "repo", false, None, "a.txt"),
            "/home/d",
            &cleared,
        );
        append_row(
            &mut rows,
            &mut last_key,
            &line("t1", "s1", "repo", false, None, "b.txt"),
            "/home/d",
            &cleared,
        );
        append_row(
            &mut rows,
            &mut last_key,
            &line("t2", "s1", "repo", false, None, "c.txt"),
            "/home/d",
            &cleared,
        );

        // Blank+Header for t1, two items, then Blank+Header for t2, one item.
        assert_eq!(rows.len(), 7);
        assert!(matches!(rows[0], Row::Blank));
        assert!(matches!(rows[1], Row::GroupHeader { .. }));
        assert!(matches!(rows[2], Row::FileItem { .. }));
        assert!(matches!(rows[3], Row::FileItem { .. }));
        assert!(matches!(rows[4], Row::Blank));
        assert!(matches!(rows[5], Row::GroupHeader { .. }));
        assert!(matches!(rows[6], Row::FileItem { .. }));
    }

    #[test]
    fn command_rows_carry_their_step_label_and_a_stable_key() {
        let mut rows = Vec::new();
        let mut last_key = None;
        let cleared = HashSet::new();
        append_row(
            &mut rows,
            &mut last_key,
            &line("t1", "s1", "repo", true, Some("2a"), "echo hi"),
            "/home/d",
            &cleared,
        );

        let Row::CommandItem {
            command,
            step,
            warn,
            key,
        } = &rows[2]
        else {
            panic!("expected CommandItem")
        };
        assert_eq!(command, "echo hi");
        assert_eq!(step.as_deref(), Some("2a"));
        assert_eq!(warn, &None);
        assert_eq!(key, "t1|s1|echo hi");
    }

    #[test]
    fn command_rows_carry_a_warn_reason_when_flagged() {
        let mut rows = Vec::new();
        let mut last_key = None;
        let cleared = HashSet::new();
        let mut flagged = line("t1", "s1", "repo", true, None, "redeploy the app");
        flagged.warn = Some("prose".to_string());
        append_row(&mut rows, &mut last_key, &flagged, "/home/d", &cleared);

        let Row::CommandItem { warn, .. } = &rows[2] else {
            panic!("expected CommandItem")
        };
        assert_eq!(warn.as_deref(), Some("prose"));
    }

    #[test]
    fn skips_a_cleared_item_but_still_headers_the_next_item_in_its_group() {
        let mut rows = Vec::new();
        let mut last_key = None;
        let mut cleared = HashSet::new();
        cleared.insert("t1|s1|a.txt".to_string());
        append_row(
            &mut rows,
            &mut last_key,
            &line("t1", "s1", "repo", false, None, "a.txt"),
            "/home/d",
            &cleared,
        );
        // Nothing rendered for the cleared item - no orphaned header either.
        assert!(rows.is_empty());
        // A second, non-cleared item in the *same* group still gets a header of its own, since
        // the cleared item deliberately left last_key untouched.
        append_row(
            &mut rows,
            &mut last_key,
            &line("t1", "s1", "repo", false, None, "b.txt"),
            "/home/d",
            &cleared,
        );
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0], Row::Blank));
        assert!(matches!(rows[1], Row::GroupHeader { .. }));
    }

    #[test]
    fn backfill_only_includes_lines_at_or_after_cutoff() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            "2026-09-09 10:00:00 session=s repo=r cwd=/home/d item=old.txt"
        )
        .unwrap();
        writeln!(
            file,
            "2026-09-09 12:00:00 session=s repo=r cwd=/home/d item=new.txt"
        )
        .unwrap();
        file.flush().unwrap();

        let mut rows = Vec::new();
        let mut last_key = None;
        let cleared = HashSet::new();
        let offset = backfill(
            file.path(),
            "2026-09-09 11:00:00",
            &mut rows,
            &mut last_key,
            "/home/d",
            &cleared,
        );

        assert!(offset > 0);
        let Row::FileItem { label, .. } = rows.last().unwrap() else {
            panic!("expected FileItem")
        };
        assert_eq!(label, "new.txt");
        // Only one group (the cutoff excluded the first line entirely).
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn poll_tail_only_consumes_complete_lines() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "2026-09-09 12:00:00 session=s repo=r cwd=/home/d item=a.txt\n2026-09-09 12:00:01 session=s repo=r cwd=/home/d item=partial").unwrap();
        file.flush().unwrap();

        let mut rows = Vec::new();
        let mut last_key = None;
        let cleared = HashSet::new();
        let offset = poll_tail(
            file.path(),
            0,
            &mut rows,
            &mut last_key,
            "/home/d",
            &cleared,
        );

        // Only the complete first line was consumed - the partial trailing line is untouched.
        assert_eq!(rows.len(), 3); // Blank, GroupHeader, FileItem
        let Row::FileItem { label, .. } = &rows[2] else {
            panic!("expected FileItem")
        };
        assert_eq!(label, "a.txt");
        assert!(offset < file.as_file().metadata().unwrap().len());
    }

    #[test]
    fn poll_tail_recovers_from_truncation() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            "2026-09-09 12:00:00 session=s repo=r cwd=/home/d item=a.txt"
        )
        .unwrap();
        file.flush().unwrap();
        let mut rows = Vec::new();
        let mut last_key = None;
        let cleared = HashSet::new();
        let offset = poll_tail(
            file.path(),
            0,
            &mut rows,
            &mut last_key,
            "/home/d",
            &cleared,
        );
        assert_eq!(rows.len(), 3); // Blank, GroupHeader, FileItem

        // Simulate `plugin.sh clear`: the file shrinks out from under a stale offset.
        file.as_file().set_len(0).unwrap();
        file.as_file().sync_all().unwrap();
        let offset_after_clear = poll_tail(
            file.path(),
            offset,
            &mut rows,
            &mut last_key,
            "/home/d",
            &cleared,
        );
        assert_eq!(offset_after_clear, 0);
        assert!(
            rows.is_empty(),
            "truncation must wipe previously-appended rows"
        );

        // And fresh content after the clear is picked up normally from offset 0.
        writeln!(
            file,
            "2026-09-09 12:05:00 session=s repo=r cwd=/home/d item=b.txt"
        )
        .unwrap();
        file.flush().unwrap();
        poll_tail(
            file.path(),
            offset_after_clear,
            &mut rows,
            &mut last_key,
            "/home/d",
            &cleared,
        );
        assert_eq!(rows.len(), 3);
        let Row::FileItem { label, .. } = &rows[2] else {
            panic!("expected FileItem")
        };
        assert_eq!(label, "b.txt");
    }
}
