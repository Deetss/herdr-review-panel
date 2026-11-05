//! Parsing and tailing of `review.log`, written by `scripts/review-notify.sh`.
//!
//! Two line formats coexist. v2 (current) is tab-delimited:
//!   2026-09-09 11:24:32<TAB>session=...<TAB>repo=NAME<TAB>cwd=PATH<TAB>kind=review|command<TAB>[step=LABEL]<TAB>[warn=REASON]<TAB>[terminal=LABEL]<TAB>item=VALUE
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
//! `id=` is an optional agent-chosen name for the item (<user_review id="audio-ab">). A later
//! item in the same session with the same id supersedes the earlier one, and so does a later
//! item of the same kind and value. The log itself stays append-only; supersession is derived
//! every time rows are built, so a restart converges on the same view.
//! `terminal=` names which terminal/pane/SSH session a command targets - either an explicit
//! <user_command terminal="ssh:host"> attribute, or the auto-detected tmux pane when that
//! attribute is absent. Absent entirely outside tmux with no explicit override.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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
    /// Which terminal/pane/SSH session this targets - explicit override or auto-detected
    /// tmux pane (see `terminal=` in the module doc).
    pub terminal: Option<String>,
    /// Agent-chosen name that lets a later item replace this one (see `id=` in the module doc).
    pub id: Option<String>,
    /// The decoded value, for display and for opening files.
    pub item: String,
    /// The value exactly as it appears in the log, still escaped. Mark files key off this
    /// rather than `item`: a decoded multiline command would write a multi-line key and
    /// permanently corrupt cleared.rs/done.rs. For single-line items the two are identical,
    /// so keys written before v2 keep matching.
    pub raw_item: String,
}

impl ParsedLine {
    /// Identifies this exact log entry in the done and cleared logs.
    fn key(&self) -> String {
        format!("{}|{}|{}", self.ts, self.session, self.raw_item)
    }

    fn kind(&self) -> &'static str {
        if self.is_command { "command" } else { "review" }
    }
}

/// Why a row is out of the main queue. Exposed on the row model for the hidden view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hidden {
    Cleared,
    Superseded,
}

/// A single row of the rendered display. Blank and GroupHeader rows are not clickable and
/// carry no key. One GroupHeader covers one session.
#[derive(Clone)]
pub enum Row {
    Blank,
    GroupHeader {
        /// Timestamp of the newest row in the group, "YYYY-MM-DD HH:MM:SS".
        ts: String,
        repo: String,
        session: String,
        /// Keys of every row in the session, open and done, so hiding the header can reach
        /// rows that are folded out of view.
        keys: Vec<String>,
    },
    /// An OPEN or DONE section label. `label` carries the count, e.g. "DONE (4)".
    Section {
        label: String,
    },
    /// A dim one-line placeholder such as "Nothing open.".
    Note(String),
    /// One line per session that has done rows. `group` identifies the session for the
    /// expand/fold state and `keys` are its done rows.
    Summary {
        group: String,
        repo: String,
        /// Timestamp of the session's newest done row.
        ts: String,
        keys: Vec<String>,
        expanded: bool,
    },
    /// `key` identifies this exact log entry for the done and cleared logs.
    FileItem {
        label: String,
        abspath: Option<String>,
        /// Some(reason) when the hook flagged this target - e.g. "missing" for a path that
        /// did not resolve on disk, which used to be dropped silently instead of shown.
        warn: Option<String>,
        key: String,
        /// Why the row is hidden. Only rows in the HIDDEN section carry one.
        hidden: Option<Hidden>,
    },
    /// `key` identifies this exact log entry for the done and cleared logs.
    CommandItem {
        command: String,
        step: Option<String>,
        /// Some(reason) when the prose heuristic flagged this command - currently always
        /// "prose" but kept as a string in case other heuristics are added later.
        warn: Option<String>,
        /// Which terminal/pane/SSH session this targets - see `terminal=` in the module doc.
        terminal: Option<String>,
        key: String,
        hidden: Option<Hidden>,
    },
    /// A session header inside the HIDDEN section: no clear glyph, and `keys` are the
    /// session's hidden rows so `r` on it restores them all.
    HiddenHeader {
        ts: String,
        repo: String,
        keys: Vec<String>,
    },
}

/// The log split by visibility. Both lists are grouped by session, oldest group first.
pub struct Rows {
    pub visible: Vec<Row>,
    pub hidden: Vec<Row>,
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
    let (mut repo, mut cwd, mut step, mut warn, mut terminal) = (None, None, None, None, None);
    let mut id = None;
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
            "terminal" => terminal = Some(unescape(v)),
            "id" => id = Some(v.to_string()),
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
        terminal,
        id,
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
        // v1 predates terminal tagging entirely - every legacy line gets None.
        terminal: None,
        id: None,
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

/// Splits `entries` into visible and hidden rows, each grouped by session. A row's cleared-log
/// event decides first (`hide` hides, `show` shows). With no event, a row is hidden when a
/// later row of the same session supersedes it, and visible otherwise.
pub fn build_rows(entries: &[ParsedLine], home: &str, cleared: &HashMap<String, bool>) -> Rows {
    let superseded = superseded_flags(entries);
    let (mut visible, mut hidden) = (Vec::new(), Vec::new());
    for (line, superseded) in entries.iter().zip(superseded) {
        let key = line.key();
        let state = match cleared.get(&key) {
            Some(true) => Some(Hidden::Cleared),
            Some(false) => None,
            None => superseded.then_some(Hidden::Superseded),
        };
        let row = item_row(line, key, state, home);
        if state.is_some() {
            hidden.push((line, row));
        } else {
            visible.push((line, row));
        }
    }
    Rows {
        visible: group_by_session(visible),
        hidden: group_by_session(hidden),
    }
}

fn item_row(line: &ParsedLine, key: String, hidden: Option<Hidden>, home: &str) -> Row {
    if line.is_command {
        Row::CommandItem {
            command: line.item.clone(),
            step: line.step.clone(),
            warn: line.warn.clone(),
            terminal: line.terminal.clone(),
            key,
            hidden,
        }
    } else {
        Row::FileItem {
            label: line.item.clone(),
            abspath: resolve_abspath(&line.item, line.cwd.as_deref(), home),
            warn: line.warn.clone(),
            key,
            hidden,
        }
    }
}

/// An entry is superseded when a later entry from the same session has the same `id`, or the
/// same kind and value. Walking backwards means "later" is just "already seen".
fn superseded_flags(entries: &[ParsedLine]) -> Vec<bool> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut flags = vec![false; entries.len()];
    for (i, line) in entries.iter().enumerate().rev() {
        let by_value = format!("{}|{}|{}", line.session, line.kind(), line.raw_item);
        let by_id = line
            .id
            .as_ref()
            .map(|id| format!("{}|id|{id}", line.session));
        flags[i] = seen.contains(&by_value) || by_id.as_ref().is_some_and(|s| seen.contains(s));
        seen.insert(by_value);
        seen.extend(by_id);
    }
    flags
}

/// One group per session, items in log order, groups ordered by their newest item so the
/// most recent session sits last. A line with no session (v1 logs) groups by repo instead.
fn group_by_session(items: Vec<(&ParsedLine, Row)>) -> Vec<Row> {
    struct Group {
        session: String,
        repo: String,
        newest: String,
        rows: Vec<Row>,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for (line, row) in items {
        let id = (
            line.session.clone(),
            if line.session.is_empty() {
                line.repo.clone()
            } else {
                String::new()
            },
        );
        let slot = *index.entry(id).or_insert_with(|| {
            groups.push(Group {
                session: line.session.clone(),
                repo: line.repo.clone(),
                newest: line.ts.clone(),
                rows: Vec::new(),
            });
            groups.len() - 1
        });
        let group = &mut groups[slot];
        if line.ts >= group.newest {
            group.newest = line.ts.clone();
            group.repo = line.repo.clone();
        }
        group.rows.push(row);
    }
    groups.sort_by(|a, b| a.newest.cmp(&b.newest));
    let mut out = Vec::new();
    for group in groups {
        let keys = group
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::FileItem { key, .. } | Row::CommandItem { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect();
        out.push(Row::Blank);
        out.push(Row::GroupHeader {
            ts: group.newest,
            repo: group.repo,
            session: group.session,
            keys,
        });
        out.extend(group.rows);
    }
    out
}

/// Reads every line in the log, in order. Used for the first load and every poll alike.
pub fn read_entries(path: &Path) -> Vec<ParsedLine> {
    let mut entries = Vec::new();
    poll_tail(path, 0, &mut entries);
    entries
}

/// Reads whatever's been appended to the log since `offset`, returning the new offset. Only
/// complete (newline-terminated) lines are consumed; a partial trailing line is left for the
/// next poll rather than risking a half-written row from a concurrent hook append.
pub fn poll_tail(path: &Path, offset: u64, entries: &mut Vec<ParsedLine>) -> u64 {
    let Ok(mut file) = File::open(path) else {
        return offset;
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return offset;
    };
    // A shorter file than our last-known offset means something outside this program
    // replaced it. Restart from the top instead of a stale offset that would never satisfy
    // `len <= offset` again and silently stop noticing new content.
    let offset = if len < offset {
        entries.clear();
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
            entries.push(parsed);
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
    fn parses_a_command_line_with_a_terminal_label() {
        let line = "2026-09-09 12:00:00\tsession=abc\trepo=deetss\tcwd=/home/d\tkind=command\tterminal=ssh:jump-host\titem=sudo systemctl restart nginx";
        let parsed = parse_line(line).unwrap();
        assert!(parsed.is_command);
        assert_eq!(parsed.terminal.as_deref(), Some("ssh:jump-host"));
        assert_eq!(parsed.item, "sudo systemctl restart nginx");
    }

    #[test]
    fn a_line_with_no_terminal_field_parses_to_none() {
        let line = "2026-09-09 12:00:00\tsession=abc\trepo=deetss\tcwd=/home/d\tkind=command\titem=git status";
        let parsed = parse_line(line).unwrap();
        assert_eq!(parsed.terminal, None);
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
        let rows = build(&[parsed]);
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
        assert_eq!(parsed.len(), 7, "every fixture line must parse");

        // v2: a path with a space survives intact, which v1 could not do.
        for p in &parsed[..6] {
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

        // An explicit terminal="..." override survives the round trip.
        assert_eq!(parsed[5].terminal.as_deref(), Some("ssh:jump-host"));
        assert_eq!(parsed[5].item, "sudo systemctl restart nginx");

        // v1 lines keep parsing alongside v2 ones in the same file.
        let legacy = &parsed[6];
        assert_eq!(legacy.repo, "deetss");
        assert_eq!(legacy.item, "git status");
        assert!(legacy.is_command);
        assert_eq!(legacy.terminal, None);
    }

    #[test]
    fn file_rows_carry_a_warn_reason_when_flagged() {
        let line = "2026-09-11 18:26:26\tsession=a\trepo=r\tcwd=/c\tkind=review\twarn=missing\titem=docs/gone.md";
        let parsed = parse_line(line).unwrap();
        let rows = build(&[parsed]);
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
            terminal: None,
            id: None,
            item: item.to_string(),
            raw_item: item.to_string(),
        }
    }

    fn build(entries: &[ParsedLine]) -> Vec<Row> {
        build_rows(entries, "/home/d", &HashMap::new()).visible
    }

    fn item_keys(rows: &[Row]) -> Vec<&str> {
        rows.iter()
            .filter_map(|r| match r {
                Row::FileItem { key, .. } | Row::CommandItem { key, .. } => Some(key.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn two_replies_from_one_session_share_a_group() {
        let rows = build(&[
            line("2026-11-05 09:00:00", "s1", "repo", false, None, "a.txt"),
            line("2026-11-05 09:00:00", "s1", "repo", false, None, "b.txt"),
            line("2026-11-05 09:05:00", "s1", "repo", false, None, "c.txt"),
        ]);

        // One Blank+Header, then all three items.
        assert_eq!(rows.len(), 5);
        assert!(matches!(rows[0], Row::Blank));
        assert!(
            matches!(&rows[1], Row::GroupHeader { ts, session, .. } if ts == "2026-11-05 09:05:00" && session == "s1")
        );
        assert!(rows[2..].iter().all(|r| matches!(r, Row::FileItem { .. })));
    }

    #[test]
    fn two_sessions_in_one_repo_get_two_groups_newest_last() {
        let rows = build(&[
            line("2026-11-05 09:10:00", "s2", "repo", false, None, "late.txt"),
            line(
                "2026-11-05 09:00:00",
                "s1",
                "repo",
                false,
                None,
                "early.txt",
            ),
            line(
                "2026-11-05 09:20:00",
                "s1",
                "repo",
                false,
                None,
                "newest.txt",
            ),
        ]);

        let headers: Vec<(&str, &str)> = rows
            .iter()
            .filter_map(|r| match r {
                Row::GroupHeader { session, ts, .. } => Some((session.as_str(), ts.as_str())),
                _ => None,
            })
            .collect();
        // s2's newest row (09:10) is older than s1's (09:20), so s1 sits last.
        assert_eq!(
            headers,
            [("s2", "2026-11-05 09:10:00"), ("s1", "2026-11-05 09:20:00")]
        );
    }

    #[test]
    fn a_later_item_with_the_same_id_supersedes_the_earlier_one() {
        let mut first = line("t1", "s1", "repo", false, None, "/tmp/listen2");
        first.id = Some("audio".to_string());
        let mut second = line("t2", "s1", "repo", false, None, "/tmp/listen3");
        second.id = Some("audio".to_string());
        let rows = build_rows(&[first, second], "/home/d", &HashMap::new());

        assert_eq!(item_keys(&rows.visible), ["t2|s1|/tmp/listen3"]);
        assert_eq!(item_keys(&rows.hidden), ["t1|s1|/tmp/listen2"]);
        assert!(matches!(
            rows.hidden[2],
            Row::FileItem {
                hidden: Some(Hidden::Superseded),
                ..
            }
        ));
    }

    #[test]
    fn the_same_text_in_another_session_is_not_superseded() {
        let rows = build_rows(
            &[
                line("t1", "s1", "repo", true, None, "echo hi"),
                line("t1", "s2", "repo", true, None, "echo hi"),
                line("t1", "s1", "repo", true, None, "echo other"),
                line("t2", "s1", "repo", true, None, "echo hi"),
            ],
            "/home/d",
            &HashMap::new(),
        );

        assert_eq!(
            item_keys(&rows.visible),
            ["t1|s2|echo hi", "t1|s1|echo other", "t2|s1|echo hi"]
        );
        assert_eq!(item_keys(&rows.hidden), ["t1|s1|echo hi"]);
    }

    #[test]
    fn a_show_event_brings_a_superseded_row_back() {
        let entries = [
            line("t1", "s1", "repo", true, None, "echo hi"),
            line("t2", "s1", "repo", true, None, "echo hi"),
        ];
        let cleared = HashMap::from([("t1|s1|echo hi".to_string(), false)]);
        let rows = build_rows(&entries, "/home/d", &cleared);

        assert_eq!(item_keys(&rows.visible), ["t1|s1|echo hi", "t2|s1|echo hi"]);
        assert!(rows.hidden.is_empty());
    }

    #[test]
    fn a_hide_event_wins_over_supersession_and_reads_as_cleared() {
        let entries = [
            line("t1", "s1", "repo", true, None, "echo hi"),
            line("t2", "s1", "repo", true, None, "echo hi"),
        ];
        let cleared = HashMap::from([("t1|s1|echo hi".to_string(), true)]);
        let rows = build_rows(&entries, "/home/d", &cleared);

        assert!(matches!(
            rows.hidden[2],
            Row::CommandItem {
                hidden: Some(Hidden::Cleared),
                ..
            }
        ));
    }

    #[test]
    fn different_ids_coexist() {
        let mut a = line("t1", "s1", "repo", false, None, "/a");
        a.id = Some("one".to_string());
        let mut b = line("t1", "s1", "repo", false, None, "/b");
        b.id = Some("two".to_string());
        assert_eq!(item_keys(&build(&[a, b])).len(), 2);
    }

    #[test]
    fn id_field_parses_from_a_v2_line() {
        let l = "2026-10-14 16:01:30\tsession=s1\trepo=r\tcwd=/tmp\tkind=review\tid=ab\titem=/tmp";
        assert_eq!(parse_line(l).unwrap().id.as_deref(), Some("ab"));
    }

    #[test]
    fn command_rows_carry_their_step_label_and_a_stable_key() {
        let rows = build(&[line("t1", "s1", "repo", true, Some("2a"), "echo hi")]);

        let Row::CommandItem {
            command,
            step,
            warn,
            terminal,
            key,
            ..
        } = &rows[2]
        else {
            panic!("expected CommandItem")
        };
        assert_eq!(command, "echo hi");
        assert_eq!(step.as_deref(), Some("2a"));
        assert_eq!(warn, &None);
        assert_eq!(terminal, &None);
        assert_eq!(key, "t1|s1|echo hi");
    }

    #[test]
    fn command_rows_carry_an_explicit_terminal_override() {
        let mut targeted = line(
            "t1",
            "s1",
            "repo",
            true,
            None,
            "sudo systemctl restart nginx",
        );
        targeted.terminal = Some("ssh:jump-host".to_string());
        let rows = build(&[targeted]);

        let Row::CommandItem { terminal, .. } = &rows[2] else {
            panic!("expected CommandItem")
        };
        assert_eq!(terminal.as_deref(), Some("ssh:jump-host"));
    }

    #[test]
    fn command_rows_carry_a_warn_reason_when_flagged() {
        let mut flagged = line("t1", "s1", "repo", true, None, "redeploy the app");
        flagged.warn = Some("prose".to_string());
        let rows = build(&[flagged]);

        let Row::CommandItem { warn, .. } = &rows[2] else {
            panic!("expected CommandItem")
        };
        assert_eq!(warn.as_deref(), Some("prose"));
    }

    #[test]
    fn a_cleared_item_leaves_no_orphaned_header() {
        let cleared = HashMap::from([("t1|s1|a.txt".to_string(), true)]);
        let rows = build_rows(
            &[line("t1", "s1", "repo", false, None, "a.txt")],
            "/home/d",
            &cleared,
        );
        assert!(rows.visible.is_empty());
        assert_eq!(item_keys(&rows.hidden), ["t1|s1|a.txt"]);
    }

    #[test]
    fn poll_tail_only_consumes_complete_lines() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "2026-09-09 12:00:00 session=s repo=r cwd=/home/d item=a.txt\n2026-09-09 12:00:01 session=s repo=r cwd=/home/d item=partial").unwrap();
        file.flush().unwrap();

        let mut entries = Vec::new();
        let offset = poll_tail(file.path(), 0, &mut entries);

        // Only the complete first line was consumed - the partial trailing line is untouched.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].item, "a.txt");
        assert!(offset < file.as_file().metadata().unwrap().len());
    }

    #[test]
    fn poll_tail_restarts_when_the_file_is_replaced_by_a_shorter_one() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            file,
            "2026-09-09 12:00:00 session=s repo=r cwd=/home/d item=a.txt"
        )
        .unwrap();
        file.flush().unwrap();
        let mut entries = Vec::new();
        let offset = poll_tail(file.path(), 0, &mut entries);
        assert_eq!(entries.len(), 1);

        // Something outside this program swaps in a shorter file under a stale offset.
        std::fs::write(file.path(), "2026-09-09 12:05:00 session=s repo=r item=b\n").unwrap();
        assert!(std::fs::metadata(file.path()).unwrap().len() < offset);
        let offset_after = poll_tail(file.path(), offset, &mut entries);
        assert!(offset_after > 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].item, "b");
    }
}
