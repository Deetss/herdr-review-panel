//! Parsing and tailing of `review.log`, written by `scripts/review-notify.sh`.
//!
//! Two line formats coexist. v2 (current) is tab-delimited:
//!   2026-09-09 11:24:32<TAB>session=...<TAB>repo=NAME<TAB>cwd=PATH<TAB>kind=review|command<TAB>[step=LABEL]<TAB>[warn=REASON]<TAB>item=VALUE
//! v1 (legacy) used spaces as the delimiter. v1 could not represent a value containing a
//! space, which silently truncated every repo and cwd under a path like "Dylan Vault", and
//! could not represent a multiline command at all. v2 values are escaped exactly as jq's
//! `@tsv` does it (`\t` `\n` `\r` `\\`), so a value can never contain a raw delimiter and
//! splitting is total. Lines are told apart by the presence of a tab, which v1 could not emit.
//!
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
    /// The decoded value, for display and for opening files.
    pub item: String,
    /// The value exactly as it appears in the log, still escaped. Mark files key off this
    /// rather than `item`: a decoded multiline command would write a multi-line key and
    /// permanently corrupt cleared.rs/done.rs. For single-line items the two are identical,
    /// so keys written before v2 keep matching.
    pub raw_item: String,
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
        /// Some(reason) when the hook flagged this target - e.g. "missing" for a path that
        /// did not resolve on disk, which used to be dropped silently instead of shown.
        warn: Option<String>,
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
    if line.contains('\t') {
        parse_v2(line)
    } else {
        parse_v1(line)
    }
}

fn parse_v2(line: &str) -> Option<ParsedLine> {
    let mut fields = line.split('\t');
    let ts = fields.next()?.to_string();
    // "YYYY-MM-DD HH:MM:SS" - guards against treating a stray tabbed line as a record.
    if ts.len() != 19 || ts.as_bytes().get(10) != Some(&b' ') {
        return None;
    }
    let mut session = String::new();
    let (mut repo, mut cwd, mut step, mut warn) = (None, None, None, None);
    let (mut item, mut raw_item) = (None, String::new());
    let mut is_command = false;
    for f in fields {
        // Exact key match on a whole field. The v1 parser searched for "kind=" as a
        // substring of the remainder, so a command whose text contained `kind=command`
        // or `step="2a"` set a phantom field; here that text is inside the item value
        // and can never be mistaken for a key.
        let Some((k, v)) = f.split_once('=') else {
            continue;
        };
        match k {
            "session" => session = v.to_string(),
            "repo" => repo = Some(unescape(v)),
            "cwd" => cwd = Some(unescape(v)),
            "kind" => is_command = v == "command",
            "step" => step = Some(unescape(v)),
            "warn" => warn = Some(v.to_string()),
            "item" => {
                raw_item = v.to_string();
                item = Some(unescape(v));
            }
            _ => {}
        }
    }
    Some(ParsedLine {
        ts,
        session,
        repo: repo?,
        cwd,
        is_command,
        step,
        warn,
        item: item?,
        raw_item,
    })
}

fn parse_v1(line: &str) -> Option<ParsedLine> {
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
        raw_item: item.clone(),
        item,
    })
}

/// Inverse of jq's `@tsv`, which is what review-parse.jq emits. An unrecognised escape is
/// passed through verbatim so a Windows path like `C:\Users` survives even if the writer
/// ever failed to escape it.
fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
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
    let item_key = format!("{}|{}|{}", line.ts, line.session, line.raw_item);
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
            warn: line.warn.clone(),
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
    fn parses_v2_with_spaces_in_repo_and_cwd() {
        // The v1 parser split on the first space, so this vault's paths truncated to
        // "Dylan" and every relative review item under it then failed to resolve.
        let line = "2026-09-11 18:26:26\tsession=881a\trepo=Dylan Vault\tcwd=/home/deetss/Documents/Dylan Vault\tkind=command\tstep=6b\titem=qm set 9002 --name x";
        let parsed = parse_line(line).unwrap();
        assert_eq!(parsed.repo, "Dylan Vault");
        assert_eq!(
            parsed.cwd.as_deref(),
            Some("/home/deetss/Documents/Dylan Vault")
        );
        assert!(parsed.is_command);
        assert_eq!(parsed.step.as_deref(), Some("6b"));
        assert_eq!(parsed.item, "qm set 9002 --name x");
    }

    #[test]
    fn unescapes_a_multiline_item() {
        let line = "2026-09-11 18:26:26\tsession=a\trepo=r\tcwd=/c\tkind=command\titem=sudo tee /x <<'EOF'\\n[sshd]\\nenabled = true\\nEOF";
        let parsed = parse_line(line).unwrap();
        assert_eq!(
            parsed.item,
            "sudo tee /x <<'EOF'\n[sshd]\nenabled = true\nEOF"
        );
        assert!(parsed.raw_item.contains("\\n"));
    }

    #[test]
    fn distinguishes_a_literal_backslash_n_from_a_newline() {
        // `printf 'a\nb'` must come back as the four characters a \ n b. If unescape and
        // jq's @tsv ever disagree, this is the test that catches it.
        let line =
            "2026-09-11 18:26:26\tsession=a\trepo=r\tcwd=/c\tkind=command\titem=printf 'a\\\\nb'";
        let parsed = parse_line(line).unwrap();
        assert_eq!(parsed.item, "printf 'a\\nb'");
        assert!(!parsed.item.contains('\n'));
    }

    #[test]
    fn an_item_containing_kind_equals_command_does_not_set_a_phantom_kind() {
        let line = "2026-09-11 18:26:26\tsession=a\trepo=r\tcwd=/c\tkind=review\titem=grep kind=command /var/log/x";
        let parsed = parse_line(line).unwrap();
        assert!(!parsed.is_command);
        assert_eq!(parsed.item, "grep kind=command /var/log/x");
    }

    #[test]
    fn an_item_containing_step_equals_does_not_set_a_phantom_step() {
        let line =
            "2026-09-11 18:26:26\tsession=a\trepo=r\tcwd=/c\tkind=command\titem=echo step=\"2a\"";
        let parsed = parse_line(line).unwrap();
        assert_eq!(parsed.step, None);
    }

    #[test]
    fn legacy_space_delimited_lines_still_parse() {
        let line =
            "2026-09-09 11:24:32 session=abc repo=deetss cwd=/home/d kind=command item=git status";
        let parsed = parse_line(line).unwrap();
        assert_eq!(parsed.item, "git status");
        assert_eq!(parsed.raw_item, "git status");
    }

    #[test]
    fn the_mark_key_uses_the_escaped_item_so_it_stays_one_line() {
        // A decoded multiline item would write a multi-line key into cleared/done and
        // corrupt both files permanently.
        let line = "2026-09-11 18:26:26\tsession=a\trepo=r\tcwd=/c\tkind=command\titem=one\\ntwo";
        let parsed = parse_line(line).unwrap();
        let mut rows = Vec::new();
        let mut last_key = None;
        append_row(
            &mut rows,
            &mut last_key,
            &parsed,
            "/home/d",
            &HashSet::new(),
        );
        let key = rows
            .iter()
            .find_map(|r| match r {
                Row::CommandItem { key, .. } => Some(key.clone()),
                _ => None,
            })
            .unwrap();
        assert!(
            !key.contains('\n'),
            "mark key must stay on one line: {key:?}"
        );
        assert!(key.ends_with("one\\ntwo"));
    }

    #[test]
    fn the_committed_fixture_log_round_trips_through_both_parsers() {
        // tests/fixtures/review.log is written by scripts/review-notify.sh (so its escaping
        // comes from jq's @tsv) and read here by unescape(). This is the only test that
        // catches the two implementations drifting apart, which is the one seam in this
        // design where a silent corruption could hide. Regenerate it with the hook, never
        // by hand.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/review.log");
        let text = std::fs::read_to_string(path).expect("fixture log missing");
        let parsed: Vec<ParsedLine> = text.lines().filter_map(parse_line).collect();
        assert_eq!(parsed.len(), 6, "every fixture line must parse");

        // v2: a path with a space survives intact, which v1 could not do.
        for p in &parsed[..5] {
            assert_eq!(p.repo, "Dylan Vault");
            assert_eq!(p.cwd.as_deref(), Some("/home/deetss/Documents/Dylan Vault"));
        }

        // A heredoc comes back as real newlines.
        let heredoc = &parsed[0];
        assert_eq!(heredoc.item.matches('\n').count(), 3);
        assert!(
            heredoc
                .item
                .starts_with("sudo tee /etc/fail2ban/jail.local <<'EOF'")
        );
        assert!(heredoc.item.ends_with("EOF"));
        assert!(
            !heredoc.raw_item.contains('\n'),
            "log line must stay one line"
        );

        // The </parameter> misclose is recovered and flagged rather than dropped.
        assert_eq!(parsed[1].warn.as_deref(), Some("misclosed"));
        assert_eq!(parsed[1].item, "sc query glpi");

        // Backslashes in a command are data, not escapes: nothing becomes a newline.
        assert!(!parsed[2].item.contains('\n'));
        assert!(parsed[2].item.starts_with("printf "));

        // A review target that does not resolve is now logged with a reason.
        assert!(!parsed[4].is_command);
        assert_eq!(parsed[4].warn.as_deref(), Some("missing"));
        assert_eq!(parsed[4].item, "docs/gone.md");

        // v1 lines keep parsing alongside v2 ones in the same file.
        let legacy = &parsed[5];
        assert_eq!(legacy.repo, "deetss");
        assert_eq!(legacy.item, "git status");
        assert!(legacy.is_command);
    }

    #[test]
    fn file_rows_carry_a_warn_reason_when_flagged() {
        let line = "2026-09-11 18:26:26\tsession=a\trepo=r\tcwd=/c\tkind=review\twarn=missing\titem=docs/gone.md";
        let parsed = parse_line(line).unwrap();
        let mut rows = Vec::new();
        let mut last_key = None;
        append_row(
            &mut rows,
            &mut last_key,
            &parsed,
            "/home/d",
            &HashSet::new(),
        );
        let warn = rows
            .iter()
            .find_map(|r| match r {
                Row::FileItem { warn, .. } => Some(warn.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(warn.as_deref(), Some("missing"));
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
            raw_item: item.to_string(),
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
