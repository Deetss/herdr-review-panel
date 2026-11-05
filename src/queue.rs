//! Turns the session-grouped visible rows from `log.rs` into the panel body: an OPEN list
//! of sessions with their open rows, then a DONE drawer with one summary line per session.

use crate::log::Row;
use std::collections::HashSet;

pub struct Body {
    pub rows: Vec<Row>,
    pub open: usize,
    pub done: usize,
}

struct Group {
    id: String,
    repo: String,
    session: String,
    keys: Vec<String>,
    open: Vec<Row>,
    done: Vec<Row>,
}

/// The row's log timestamp, which leads its key.
fn key_ts(key: &str) -> &str {
    key.split('|').next().unwrap_or("")
}

fn newest(rows: &[Row]) -> Option<&str> {
    rows.iter().filter_map(item_key).map(key_ts).max()
}

fn item_key(row: &Row) -> Option<&str> {
    match row {
        Row::FileItem { key, .. } | Row::CommandItem { key, .. } => Some(key),
        _ => None,
    }
}

/// Stable name for a session across rebuilds. A legacy line with no session groups by repo.
fn group_id(session: &str, repo: &str) -> String {
    if session.is_empty() {
        repo.to_string()
    } else {
        session.to_string()
    }
}

/// What a row is, across rebuilds, so the cursor can follow it when rows move.
pub fn identity(row: &Row) -> Option<String> {
    match row {
        Row::Summary { group, .. } => Some(format!("sum:{group}")),
        other => item_key(other).map(str::to_string),
    }
}

fn split_groups(visible: &[Row], done: &HashSet<String>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for row in visible {
        match row {
            Row::GroupHeader {
                repo,
                session,
                keys,
                ..
            } => groups.push(Group {
                id: group_id(session, repo),
                repo: repo.clone(),
                session: session.clone(),
                keys: keys.clone(),
                open: Vec::new(),
                done: Vec::new(),
            }),
            Row::CommandItem { key, .. } if done.contains(key) => {
                if let Some(group) = groups.last_mut() {
                    group.done.push(row.clone());
                }
            }
            Row::FileItem { .. } | Row::CommandItem { .. } => {
                if let Some(group) = groups.last_mut() {
                    group.open.push(row.clone());
                }
            }
            _ => {}
        }
    }
    groups
}

/// A step label only means something next to another one: a lone `2.` says nothing.
fn drop_orphan_step(rows: &mut [Row]) {
    let with_step = rows
        .iter()
        .filter(|r| matches!(r, Row::CommandItem { step: Some(_), .. }))
        .count();
    if with_step == 1 {
        for row in rows.iter_mut() {
            if let Row::CommandItem { step, .. } = row {
                *step = None;
            }
        }
    }
}

pub fn build_body(visible: &[Row], done: &HashSet<String>, expanded: &HashSet<String>) -> Body {
    let mut groups = split_groups(visible, done);
    let open_total: usize = groups.iter().map(|g| g.open.len()).sum();
    let done_total: usize = groups.iter().map(|g| g.done.len()).sum();
    if open_total + done_total == 0 {
        return Body {
            rows: Vec::new(),
            open: 0,
            done: 0,
        };
    }

    for group in &mut groups {
        drop_orphan_step(&mut group.open);
    }

    let mut rows = vec![Row::Section {
        label: "OPEN".to_string(),
    }];
    let mut open_groups: Vec<&Group> = groups.iter().filter(|g| !g.open.is_empty()).collect();
    open_groups.sort_by_key(|g| newest(&g.open).unwrap_or("").to_string());
    if open_groups.is_empty() {
        rows.push(Row::Note("Nothing open.".to_string()));
    }
    for group in open_groups {
        rows.push(Row::GroupHeader {
            ts: newest(&group.open).unwrap_or("").to_string(),
            repo: group.repo.clone(),
            session: group.session.clone(),
            keys: group.keys.clone(),
        });
        rows.extend(group.open.iter().cloned());
    }

    if done_total > 0 {
        rows.push(Row::Blank);
        rows.push(Row::Section {
            label: format!("DONE ({done_total})"),
        });
        let mut done_groups: Vec<&Group> = groups.iter().filter(|g| !g.done.is_empty()).collect();
        done_groups.sort_by_key(|g| newest(&g.done).unwrap_or("").to_string());
        for group in done_groups {
            let is_open = expanded.contains(&group.id);
            rows.push(Row::Summary {
                group: group.id.clone(),
                repo: group.repo.clone(),
                ts: newest(&group.done).unwrap_or("").to_string(),
                keys: group
                    .done
                    .iter()
                    .filter_map(item_key)
                    .map(str::to_string)
                    .collect(),
                expanded: is_open,
            });
            if is_open {
                rows.extend(group.done.iter().cloned());
            }
        }
    }

    Body {
        rows,
        open: open_total,
        done: done_total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(ts: &str, session: &str, cmd: &str, step: Option<&str>) -> Row {
        Row::CommandItem {
            command: cmd.to_string(),
            step: step.map(str::to_string),
            warn: None,
            terminal: None,
            key: format!("{ts}|{session}|{cmd}"),
            hidden: None,
        }
    }

    fn header(ts: &str, repo: &str, session: &str) -> Row {
        Row::GroupHeader {
            ts: ts.to_string(),
            repo: repo.to_string(),
            session: session.to_string(),
            keys: Vec::new(),
        }
    }

    fn steps(rows: &[Row]) -> Vec<Option<String>> {
        rows.iter()
            .filter_map(|r| match r {
                Row::CommandItem { step, .. } => Some(step.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_lone_step_label_is_dropped_and_a_pair_is_kept() {
        let visible = vec![
            header("2026-11-05 09:00:00", "r", "s"),
            command("2026-11-05 09:00:00", "s", "a", Some("1")),
            command("2026-11-05 09:00:00", "s", "b", None),
        ];
        let body = build_body(&visible, &HashSet::new(), &HashSet::new());
        assert_eq!(steps(&body.rows), vec![None, None]);

        let visible = vec![
            header("2026-11-05 09:00:00", "r", "s"),
            command("2026-11-05 09:00:00", "s", "a", Some("1")),
            command("2026-11-05 09:00:00", "s", "b", Some("2")),
        ];
        let body = build_body(&visible, &HashSet::new(), &HashSet::new());
        assert_eq!(
            steps(&body.rows),
            vec![Some("1".to_string()), Some("2".to_string())]
        );
    }

    #[test]
    fn marking_step_one_done_orphans_step_two() {
        let visible = vec![
            header("2026-11-05 09:00:00", "r", "s"),
            command("2026-11-05 09:00:00", "s", "a", Some("1")),
            command("2026-11-05 09:00:00", "s", "b", Some("2")),
        ];
        let done = HashSet::from(["2026-11-05 09:00:00|s|a".to_string()]);
        let body = build_body(&visible, &done, &HashSet::new());
        assert_eq!(steps(&body.rows), vec![None]);
        assert_eq!((body.open, body.done), (1, 1));
    }

    #[test]
    fn open_with_nothing_open_shows_a_note_then_the_drawer() {
        let visible = vec![
            header("2026-11-05 09:00:00", "r", "s"),
            command("2026-11-05 09:00:00", "s", "a", None),
        ];
        let done = HashSet::from(["2026-11-05 09:00:00|s|a".to_string()]);
        let body = build_body(&visible, &done, &HashSet::new());
        assert!(matches!(&body.rows[1], Row::Note(n) if n == "Nothing open."));
        assert!(matches!(&body.rows[2], Row::Blank));
        assert!(matches!(&body.rows[3], Row::Section { label } if label == "DONE (1)"));
        assert!(matches!(
            &body.rows[4],
            Row::Summary {
                expanded: false,
                ..
            }
        ));
        assert_eq!(body.rows.len(), 5);
    }
}
