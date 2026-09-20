use crate::actions;
use crate::cleared;
use crate::done;
use crate::log::{self, Row};
use chrono::Local;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a click-feedback message ("Opened x", "Copied to clipboard") stays in the hint
/// row before reverting to the normal hint - long enough to register as confirmation, short
/// enough that it's gone well before you'd click something else.
const STATUS_TTL: Duration = Duration::from_millis(1500);

/// How long the full-command detail overlay stays open before auto-hiding, mirroring
/// STATUS_TTL - a keypress (Esc/Enter/'q') still dismisses it immediately regardless. Note:
/// this overlay exists specifically so Collie/mobile users (whose OSC-52 clipboard copy
/// silently fails) can read and select the full command by hand, so this trims the window
/// they have to do that before it vanishes on its own.
const DETAIL_TTL: Duration = Duration::from_millis(2500);

/// How long a checked-off command stays in place before relocating to the trailing
/// "Completed" section - long enough to register as confirmation of the check, short enough
/// that the queue doesn't stay cluttered with done items.
const COMPLETE_DELAY: Duration = Duration::from_secs(10);

/// Sentinel repo label for the synthetic trailing GroupHeader that collects completed
/// commands. Paired with an empty `ts` (real log timestamps are never empty), so this can
/// never collide with a genuine `(ts, repo)` group.
const COMPLETED_REPO: &str = "Completed";

/// Column (within the list area, 0-indexed) where a command row's "[ ]"/"[x]" checkbox ends -
/// a 2-space indent then the 4-char box. Clicks before this column toggle done; clicks at or
/// past it copy the command instead. A step label (if any) renders after the checkbox, so it
/// never shifts this boundary. Kept in sync with ui.rs's render_row.
pub const CHECKBOX_END_COL: u16 = 6;

/// How many rows a single scroll-wheel tick moves, independent of cursor position.
pub const WHEEL_SCROLL_LINES: isize = 3;

pub struct App {
    pub rows: Vec<Row>,
    /// Index into `rows` of the currently highlighted item - set by keyboard navigation or
    /// mouse hover, whichever moved it most recently. Points at any row type (Backspace on a
    /// GroupHeader clears its whole section), but keyboard Up/Down only ever lands on items.
    pub cursor: Option<usize>,
    /// Index of the first row currently visible in the list area.
    pub scroll: usize,
    pub close_hovered: bool,
    pub clear_all_hovered: bool,
    pub should_quit: bool,
    log_path: PathBuf,
    offset: u64,
    home: String,
    last_key: Option<(String, String)>,
    done: HashSet<String>,
    done_path: PathBuf,
    /// When each done key was checked, so sweep_completed knows when COMPLETE_DELAY has
    /// elapsed. In-memory only - a restart re-seeds it from done.rs (see with_paths), which
    /// is fine since a key already done before this run has no real "when" to honor anyway.
    done_since: HashMap<String, Instant>,
    /// Keys already relocated into the Completed section, so sweep_completed only ever
    /// considers newly-eligible ones instead of rescanning everything every tick.
    completed: HashSet<String>,
    cleared: HashSet<String>,
    cleared_path: PathBuf,
    status: Option<(String, Instant)>,
    /// The command currently shown full-screen, if any, alongside when it was shown. Collie
    /// (the mobile web UI) strips ANSI server-side, so the OSC-52 clipboard copy that activation
    /// does cannot reach a phone - this is how the command becomes readable there. Desktop still
    /// gets the copy as well. Auto-hides after DETAIL_TTL, like the status toast (see detail_text).
    detail: Option<(String, Instant)>,
    /// Rows visible in the list area as of the last frame - drives ensure_cursor_visible, and
    /// lets scroll_by clamp independently of cursor position instead of fighting it every
    /// frame (a scroll wheel tick should move the view without yanking the cursor along).
    visible_height: usize,
}

impl App {
    pub fn new(log_path: PathBuf, window_minutes: i64) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Self::with_paths(
            log_path,
            window_minutes,
            home,
            done::path(),
            cleared::path(),
        )
    }

    /// Everything `new` derives from globals (env `HOME`, `~/.claude/review-{done,cleared}.log`)
    /// is a plain parameter here instead, so tests can point them at tempfiles without racing
    /// real user state or each other over shared env vars.
    pub fn with_paths(
        log_path: PathBuf,
        window_minutes: i64,
        home: String,
        done_path: PathBuf,
        cleared_path: PathBuf,
    ) -> Self {
        let cutoff = (Local::now() - Duration::from_secs((window_minutes.max(0) as u64) * 60))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let done = done::load(&done_path);
        // Already done before this run started - there's no real "when" for these (done.rs
        // carries no timestamp), so treat them as already past COMPLETE_DELAY rather than
        // making the user wait again on every reopen.
        let done_since = done
            .iter()
            .map(|k| (k.clone(), Instant::now() - COMPLETE_DELAY))
            .collect();
        let cleared = cleared::load(&cleared_path);
        let mut rows = Vec::new();
        let mut last_key = None;
        let offset = log::backfill(
            &log_path,
            &cutoff,
            &mut rows,
            &mut last_key,
            &home,
            &cleared,
        );
        let mut app = App {
            rows,
            cursor: None,
            scroll: 0,
            close_hovered: false,
            clear_all_hovered: false,
            detail: None,
            should_quit: false,
            log_path,
            offset,
            home,
            last_key,
            done,
            done_path,
            done_since,
            completed: HashSet::new(),
            cleared,
            cleared_path,
            status: None,
            visible_height: 1,
        };
        app.sweep_completed();
        app.jump_to_latest();
        app
    }

    /// Polls the log for newly-appended lines and reloads the done/cleared marks files, so
    /// changes made from another instance of this panel show up here too. Returns true if the
    /// log changed (so the caller knows to redraw immediately rather than waiting for the next
    /// tick).
    pub fn poll(&mut self) -> bool {
        let before = self.rows.len();
        self.offset = log::poll_tail(
            &self.log_path,
            self.offset,
            &mut self.rows,
            &mut self.last_key,
            &self.home,
            &self.cleared,
        );
        self.done = done::load(&self.done_path);
        // A key can arrive here already done without ever going through this instance's
        // mark_done - another panel instance, or review-run.sh completing it from the phone
        // bridge. Same reasoning as with_paths' initial seed: there's no real "when" to honor
        // from this instance's perspective, so treat it as already elapsed rather than never
        // sweeping it at all (done_since would otherwise simply have no entry for it, ever).
        for key in &self.done {
            self.done_since
                .entry(key.clone())
                .or_insert_with(|| Instant::now() - COMPLETE_DELAY);
        }
        let new_cleared = cleared::load(&self.cleared_path);
        if new_cleared != self.cleared {
            // Someone (another instance, or this panel's own clear-all) dismissed something
            // already sitting in `rows` - drop it retroactively rather than waiting for a
            // restart's backfill to skip it.
            self.cleared = new_cleared;
            self.rows
                .retain(|r| row_key(r).is_none_or(|k| !self.cleared.contains(k)));
            self.prune_empty_groups();
        }
        let structural_change = self.rows.len() != before;
        if structural_change {
            self.jump_to_latest();
        }
        // Independent of the length check above: a sweep relocates rows without changing
        // how many there are, so it needs its own trigger for the caller to know to redraw
        // and its own jump_to_latest (idempotent if both fire in the same tick).
        if self.sweep_completed() {
            self.jump_to_latest();
            return true;
        }
        structural_change
    }

    pub fn is_done(&self, key: &str) -> bool {
        self.done.contains(key)
    }

    /// Called every frame with the terminal's current height, but only re-clamps scroll on an
    /// actual change (a resize) - otherwise this would re-run ensure_cursor_visible on every
    /// single frame and silently undo any independent scroll-wheel movement.
    pub fn set_visible_height(&mut self, height: usize) {
        let height = height.max(1);
        if height != self.visible_height {
            self.visible_height = height;
            self.ensure_cursor_visible();
        }
    }

    /// None once STATUS_TTL has elapsed - the caller doesn't need to clear it explicitly,
    /// since the event loop redraws at least every 200ms regardless of activity, so an
    /// expired message reliably disappears on its own within that window.
    pub fn status_text(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, at)| at.elapsed() < STATUS_TTL)
            .map(|(msg, _)| msg.as_str())
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    /// None once DETAIL_TTL has elapsed, for the same reason status_text() expires on its own -
    /// the 200ms poll loop redraws regardless of activity, so the overlay stops rendering on its
    /// own without an explicit clear.
    pub fn detail_text(&self) -> Option<&str> {
        self.detail
            .as_ref()
            .filter(|(_, at)| at.elapsed() < DETAIL_TTL)
            .map(|(cmd, _)| cmd.as_str())
    }

    fn mark_done(&mut self, key: &str) {
        if self.done.insert(key.to_string()) {
            done::mark(&self.done_path, key);
            self.done_since.insert(key.to_string(), Instant::now());
        }
    }

    /// Marks an item dismissed and drops it from `rows` immediately - unlike `mark_done`,
    /// which keeps the row visible (struck through), this removes it from view entirely.
    fn mark_cleared(&mut self, key: &str) {
        if self.cleared.insert(key.to_string()) {
            cleared::mark(&self.cleared_path, key);
        }
        self.rows.retain(|r| row_key(r) != Some(key));
        // Otherwise a cleared-then-forgotten key would sit in these maps forever - harmless
        // but pointless to keep once the row it describes is gone for good.
        self.done_since.remove(key);
        self.completed.remove(key);
    }

    /// Drops any GroupHeader (and its preceding Blank separator) left with no items under it
    /// after clearing - so dismissing everything in a section, one at a time or all at once,
    /// doesn't leave an orphaned timestamp/repo line with nothing beneath it.
    fn prune_empty_groups(&mut self) {
        let mut keep = vec![true; self.rows.len()];
        for i in 0..self.rows.len() {
            if !matches!(self.rows[i], Row::GroupHeader { .. }) {
                continue;
            }
            let has_items = self.rows[i + 1..]
                .iter()
                .take_while(|r| !matches!(r, Row::Blank | Row::GroupHeader { .. }))
                .any(|r| matches!(r, Row::FileItem { .. } | Row::CommandItem { .. }));
            if !has_items {
                keep[i] = false;
                if i > 0 && matches!(self.rows[i - 1], Row::Blank) {
                    keep[i - 1] = false;
                }
            }
        }
        let mut idx = 0;
        self.rows.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });
    }

    /// Relocates any command checked off at least COMPLETE_DELAY ago out of its normal
    /// position into a single trailing "Completed" section. Cheap no-op on an ordinary tick
    /// (the common case) since it bails before touching `rows` at all when nothing is newly
    /// eligible. Returns whether anything moved, so the caller knows to redraw/rejump.
    fn sweep_completed(&mut self) -> bool {
        let newly_eligible: HashSet<String> = self
            .done_since
            .iter()
            .filter(|(k, at)| !self.completed.contains(*k) && at.elapsed() >= COMPLETE_DELAY)
            .map(|(k, _)| k.clone())
            .collect();
        if newly_eligible.is_empty() {
            return false;
        }
        // Stable partition: both halves keep their original relative order for free, so
        // rows already sitting in a prior Completed section (kept, since their key is in
        // `completed` rather than `newly_eligible`) stay exactly where they are.
        let (kept, moved): (Vec<Row>, Vec<Row>) =
            std::mem::take(&mut self.rows).into_iter().partition(
                |r| !matches!(r, Row::CommandItem { key, .. } if newly_eligible.contains(key)),
            );
        self.rows = kept;
        self.prune_empty_groups();
        let has_completed_header = self.rows.iter().any(
            |r| matches!(r, Row::GroupHeader { ts, repo } if ts.is_empty() && repo == COMPLETED_REPO),
        );
        if !has_completed_header {
            self.rows.push(Row::Blank);
            self.rows.push(Row::GroupHeader {
                ts: String::new(),
                repo: COMPLETED_REPO.to_string(),
            });
        }
        self.rows.extend(moved);
        self.completed.extend(newly_eligible);
        true
    }

    fn activatable_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().enumerate().filter_map(|(i, r)| {
            matches!(r, Row::FileItem { .. } | Row::CommandItem { .. }).then_some(i)
        })
    }

    fn jump_to_latest(&mut self) {
        self.cursor = self.activatable_rows().last();
        self.ensure_cursor_visible();
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let items: Vec<usize> = self.activatable_rows().collect();
        if items.is_empty() {
            return;
        }
        let current = self
            .cursor
            .and_then(|c| items.iter().position(|&i| i == c))
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, items.len() as isize - 1) as usize;
        self.cursor = Some(items[next]);
        self.ensure_cursor_visible();
    }

    /// A scroll-wheel tick moves the view only - it doesn't touch the cursor, so looking
    /// through history doesn't fight with (or get overridden by) whatever's highlighted.
    pub fn scroll_by(&mut self, delta: isize) {
        let max_scroll = self.rows.len().saturating_sub(self.visible_height);
        let next = (self.scroll as isize + delta).clamp(0, max_scroll as isize);
        self.scroll = next as usize;
    }

    fn ensure_cursor_visible(&mut self) {
        let visible_height = self.visible_height;
        if let Some(cursor) = self.cursor {
            if cursor < self.scroll {
                self.scroll = cursor;
            } else if cursor >= self.scroll + visible_height {
                self.scroll = cursor + 1 - visible_height;
            }
        }
        let max_scroll = self.rows.len().saturating_sub(visible_height);
        self.scroll = self.scroll.min(max_scroll);
    }

    /// Enter (keyboard) or a click past the checkbox column (mouse): open a file, copy a
    /// command. Mirrors click_row's non-checkbox branch for whichever row the cursor is on.
    pub fn activate_cursor(&mut self) {
        if let Some(activation) = self
            .cursor
            .and_then(|c| self.rows.get(c))
            .and_then(plan_activation)
        {
            self.run_activation(activation);
        }
    }

    /// Space/'d' (keyboard) or a click on the checkbox column (mouse): mark the command at the
    /// cursor done. No-op for file rows, which have no done state.
    pub fn toggle_done_cursor(&mut self) {
        if let Some(Row::CommandItem { key, .. }) = self.cursor.and_then(|c| self.rows.get(c)) {
            let key = key.clone();
            self.mark_done(&key);
        }
    }

    /// Backspace/Delete (keyboard): dismiss the item at the cursor entirely, or - if the
    /// cursor is on a GroupHeader (only reachable by mouse hover, since keyboard nav skips
    /// straight to items) - every item in that section at once. Shares its logic with the
    /// trailing "x" clicked on any row (see click_row).
    pub fn clear_cursor(&mut self) {
        if let Some(idx) = self.cursor {
            self.clear_row_at(idx);
        }
    }

    fn clear_row_at(&mut self, idx: usize) {
        match self.rows.get(idx) {
            Some(row @ (Row::FileItem { .. } | Row::CommandItem { .. })) => {
                if let Some(key) = row_key(row) {
                    let key = key.to_string();
                    self.mark_cleared(&key);
                }
            }
            Some(Row::GroupHeader { .. }) => {
                let keys: Vec<String> = self.rows[idx + 1..]
                    .iter()
                    .take_while(|r| !matches!(r, Row::Blank | Row::GroupHeader { .. }))
                    .filter_map(row_key)
                    .map(str::to_string)
                    .collect();
                for key in keys {
                    self.mark_cleared(&key);
                }
            }
            _ => return,
        }
        self.prune_empty_groups();
        self.jump_to_latest();
    }

    /// `list_width` is the rendered row width - clicking its rightmost column always clears
    /// that row (or, for a GroupHeader, its whole section), matching the trailing "x" ui.rs
    /// draws there. Anywhere else on a command row before CHECKBOX_END_COL toggles done;
    /// past it (or anywhere on a file row) opens/copies as usual.
    pub fn click_row(&mut self, row_idx: usize, local_col: u16, list_width: u16) {
        if local_col + 1 >= list_width {
            self.clear_row_at(row_idx);
            return;
        }
        match self.rows.get(row_idx) {
            Some(Row::CommandItem { key, .. }) if local_col < CHECKBOX_END_COL => {
                let key = key.clone();
                self.mark_done(&key);
            }
            Some(row) => {
                if let Some(activation) = plan_activation(row) {
                    self.run_activation(activation);
                }
            }
            None => {}
        }
    }

    /// The top "clear" button: wipes review.log, done-marks, and cleared-marks entirely via
    /// the same plugin.sh path the tools-menu action uses. The panel stays open - poll()'s
    /// truncation handling (see log::poll_tail) picks up the now-empty file on the next tick.
    pub fn click_clear_all(&mut self) {
        actions::clear_all();
        self.set_status("Cleared everything");
    }

    /// Runs the action and leaves a status message behind - opening an external editor can
    /// take a visible moment, so the click needs its own immediate confirmation that isn't
    /// waiting on that.
    fn run_activation(&mut self, activation: Activation) {
        match activation {
            Activation::OpenFile(path) => {
                let opener = actions::open_item(&path);
                let label = Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(path);
                let msg = match opener {
                    actions::Opener::Editor | actions::Opener::Unknown => format!("Opened {label}"),
                    actions::Opener::Fallback => {
                        format!("Opened {label} (fallback opener, view may not match)")
                    }
                };
                self.set_status(msg);
            }
            Activation::CopyCommand(command) => {
                actions::copy_to_clipboard(&command);
                self.detail = Some((command, Instant::now()));
                self.set_status("Copied to clipboard");
            }
        }
    }

    pub fn click_close(&mut self) {
        actions::close();
        self.should_quit = true;
    }

    /// Dismisses the detail overlay. Returns whether there was one to dismiss, so the caller
    /// can swallow the keypress instead of also closing the panel with it.
    pub fn dismiss_detail(&mut self) -> bool {
        self.detail.take().is_some()
    }
}

fn row_key(row: &Row) -> Option<&str> {
    match row {
        Row::FileItem { key, .. } | Row::CommandItem { key, .. } => Some(key),
        Row::Blank | Row::GroupHeader { .. } => None,
    }
}

enum Activation {
    OpenFile(String),
    CopyCommand(String),
}

fn plan_activation(row: &Row) -> Option<Activation> {
    match row {
        Row::FileItem {
            abspath: Some(path),
            ..
        } => Some(Activation::OpenFile(path.clone())),
        Row::CommandItem { command, .. } => Some(Activation::CopyCommand(command.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Keeps the tempfiles alive for the fixture's lifetime - App only stores their paths,
    /// so dropping the NamedTempFiles early would delete the files out from under it.
    struct Fixture {
        _log: tempfile::NamedTempFile,
        _done: tempfile::NamedTempFile,
        _cleared: tempfile::NamedTempFile,
        app: App,
    }

    fn fixture(log_contents: &str) -> Fixture {
        let mut log = tempfile::NamedTempFile::new().unwrap();
        write!(log, "{log_contents}").unwrap();
        log.flush().unwrap();
        let done = tempfile::NamedTempFile::new().unwrap();
        let cleared = tempfile::NamedTempFile::new().unwrap();
        let app = App::with_paths(
            log.path().to_path_buf(),
            10,
            "/home/d".to_string(),
            done.path().to_path_buf(),
            cleared.path().to_path_buf(),
        );
        Fixture {
            _log: log,
            _done: done,
            _cleared: cleared,
            app,
        }
    }

    fn now_line(repo: &str, item: &str) -> String {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        format!("{ts} session=s repo={repo} cwd=/home/d item={item}\n")
    }

    fn now_command_line(repo: &str, item: &str) -> String {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        format!("{ts} session=s repo={repo} cwd=/home/d kind=command item={item}\n")
    }

    /// Two lines sharing one explicit timestamp, so grouping-dependent tests can't flake on a
    /// second boundary landing between two separate `Local::now()` calls.
    fn two_line_group(repo: &str, item_a: &str, item_b: &str) -> String {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        format!(
            "{ts} session=s repo={repo} cwd=/home/d item={item_a}\n{ts} session=s repo={repo} cwd=/home/d item={item_b}\n"
        )
    }

    #[test]
    fn backfill_jumps_cursor_to_the_last_item() {
        let f = fixture(&two_line_group("r", "a.txt", "b.txt"));
        assert_eq!(f.app.rows.len(), 4); // Blank, Header, a.txt, b.txt
        let cursor = f.app.cursor.unwrap();
        assert!(matches!(&f.app.rows[cursor], Row::FileItem { label, .. } if label == "b.txt"));
    }

    #[test]
    fn move_cursor_only_lands_on_items_and_clamps_at_the_ends() {
        let mut f = fixture(&two_line_group("r", "a.txt", "b.txt"));
        f.app.move_cursor(-100);
        assert!(
            matches!(&f.app.rows[f.app.cursor.unwrap()], Row::FileItem { label, .. } if label == "a.txt")
        );
        f.app.move_cursor(100);
        assert!(
            matches!(&f.app.rows[f.app.cursor.unwrap()], Row::FileItem { label, .. } if label == "b.txt")
        );
    }

    #[test]
    fn toggle_done_marks_a_command_and_persists_it() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        let Row::CommandItem { key, .. } = &f.app.rows[f.app.cursor.unwrap()] else {
            panic!("expected CommandItem")
        };
        assert!(f.app.is_done(key));
        assert!(done::load(&f.app.done_path).contains(key));
    }

    #[test]
    fn clicking_the_checkbox_column_toggles_done_but_leaves_the_row_in_place() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let idx = f.app.cursor.unwrap();
        f.app.click_row(idx, 0, 80); // inside the checkbox (columns 0..CHECKBOX_END_COL)
        let Row::CommandItem { key, .. } = &f.app.rows[idx] else {
            panic!("expected CommandItem")
        };
        assert!(f.app.is_done(key));
        assert_eq!(f.app.rows.len(), 3);
    }

    #[test]
    fn toggling_done_does_not_immediately_relocate_the_row() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        f.app.poll();
        // COMPLETE_DELAY hasn't elapsed yet - still Blank, GroupHeader, the one command, no
        // Completed section.
        assert_eq!(f.app.rows.len(), 3);
        assert!(
            !f.app
                .rows
                .iter()
                .any(|r| matches!(r, Row::GroupHeader { repo, .. } if repo == "Completed")),
            "no Completed section should exist before the delay elapses"
        );
    }

    #[test]
    fn a_command_moves_to_completed_once_the_delay_elapses() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        let key = {
            let Row::CommandItem { key, .. } = &f.app.rows[f.app.cursor.unwrap()] else {
                panic!("expected CommandItem")
            };
            key.clone()
        };
        // No real sleep: back-date the in-memory timer past COMPLETE_DELAY directly.
        f.app
            .done_since
            .insert(key.clone(), Instant::now() - Duration::from_secs(11));

        assert!(f.app.sweep_completed(), "sweep should report a relocation");

        let header_idx = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, Row::GroupHeader { repo, .. } if repo == "Completed"))
            .expect("expected a Completed header");
        let moved_idx = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, Row::CommandItem { key: k, .. } if k == &key))
            .expect("expected the command row to still exist");
        assert!(
            moved_idx > header_idx,
            "the completed command should sit after the Completed header"
        );
        // A second sweep with nothing newly eligible is a no-op, not a duplicate move.
        assert!(!f.app.sweep_completed());
    }

    #[test]
    fn items_already_done_at_load_time_land_in_completed_immediately() {
        let mut log = tempfile::NamedTempFile::new().unwrap();
        write!(log, "{}", now_command_line("r", "echo hi")).unwrap();
        log.flush().unwrap();
        let done = tempfile::NamedTempFile::new().unwrap();
        let cleared = tempfile::NamedTempFile::new().unwrap();

        // Mark it done on disk *before* the panel ever loads it - there's no session to have
        // watched it get checked, so it should skip the waiting period entirely.
        let key = {
            let probe = App::with_paths(
                log.path().to_path_buf(),
                10,
                "/home/d".to_string(),
                done.path().to_path_buf(),
                cleared.path().to_path_buf(),
            );
            let Row::CommandItem { key, .. } = &probe.rows[probe.cursor.unwrap()] else {
                panic!("expected CommandItem")
            };
            key.clone()
        };
        done::mark(&done.path().to_path_buf(), &key);

        let app = App::with_paths(
            log.path().to_path_buf(),
            10,
            "/home/d".to_string(),
            done.path().to_path_buf(),
            cleared.path().to_path_buf(),
        );
        let header_idx = app
            .rows
            .iter()
            .position(|r| matches!(r, Row::GroupHeader { repo, .. } if repo == "Completed"))
            .expect("expected a Completed header on load");
        let moved_idx = app
            .rows
            .iter()
            .position(|r| matches!(r, Row::CommandItem { key: k, .. } if k == &key))
            .expect("expected the command row to exist");
        assert!(moved_idx > header_idx);
    }

    #[test]
    fn clearing_the_completed_header_clears_every_item_under_it() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        let key = {
            let Row::CommandItem { key, .. } = &f.app.rows[f.app.cursor.unwrap()] else {
                panic!("expected CommandItem")
            };
            key.clone()
        };
        f.app
            .done_since
            .insert(key, Instant::now() - Duration::from_secs(11));
        f.app.sweep_completed();

        let header_idx = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, Row::GroupHeader { repo, .. } if repo == "Completed"))
            .unwrap();
        f.app.cursor = Some(header_idx);
        f.app.clear_cursor();
        assert!(f.app.rows.is_empty());
    }

    #[test]
    fn clicking_past_the_checkbox_copies_instead_of_toggling_done() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let idx = f.app.cursor.unwrap();
        f.app.click_row(idx, CHECKBOX_END_COL, 80);
        let Row::CommandItem { key, .. } = &f.app.rows[idx] else {
            panic!("expected CommandItem")
        };
        assert!(
            !f.app.is_done(key),
            "clicking past the checkbox should copy, not toggle done"
        );
    }

    #[test]
    fn clicking_the_rightmost_column_clears_the_row_and_prunes_the_empty_header() {
        let mut f = fixture(&now_line("r", "a.txt"));
        let idx = f.app.cursor.unwrap();
        f.app.click_row(idx, 79, 80); // width=80 -> last column is index 79
        assert!(
            f.app.rows.is_empty(),
            "clearing the only item should also drop its now-empty header"
        );
    }

    #[test]
    fn clearing_persists_so_a_fresh_backfill_does_not_resurrect_it() {
        let mut f = fixture(&now_line("r", "a.txt"));
        let idx = f.app.cursor.unwrap();
        f.app.click_row(idx, 79, 80);
        assert!(f.app.rows.is_empty());

        let reopened = App::with_paths(
            f._log.path().to_path_buf(),
            10,
            "/home/d".to_string(),
            f._done.path().to_path_buf(),
            f._cleared.path().to_path_buf(),
        );
        assert!(
            reopened.rows.is_empty(),
            "a fresh backfill should skip an already-cleared item"
        );
    }

    #[test]
    fn clearing_a_group_header_clears_every_item_under_it() {
        let mut f = fixture(&two_line_group("r", "a.txt", "b.txt"));
        f.app.cursor = Some(1); // the GroupHeader row
        f.app.clear_cursor();
        assert!(f.app.rows.is_empty());
    }

    #[test]
    fn clearing_one_item_leaves_the_header_and_the_other_item() {
        let mut f = fixture(&two_line_group("r", "a.txt", "b.txt"));
        f.app.cursor = Some(2); // a.txt
        f.app.clear_cursor();
        // The header still has an item under it (b.txt), so prune_empty_groups leaves the
        // Blank+GroupHeader pair in place - only a.txt itself is gone.
        assert_eq!(f.app.rows.len(), 3);
        assert!(matches!(f.app.rows[0], Row::Blank));
        assert!(matches!(f.app.rows[1], Row::GroupHeader { .. }));
        assert!(matches!(&f.app.rows[2], Row::FileItem { label, .. } if label == "b.txt"));
    }

    #[test]
    fn scroll_by_clamps_and_never_moves_the_cursor() {
        let contents: String = (0..20)
            .map(|i| now_line("r", &format!("f{i}.txt")))
            .collect();
        let mut f = fixture(&contents);
        f.app.set_visible_height(5);
        let cursor_before = f.app.cursor;

        f.app.scroll_by(-1000);
        assert_eq!(f.app.scroll, 0);
        assert_eq!(f.app.cursor, cursor_before);

        f.app.scroll_by(1000);
        assert_eq!(f.app.scroll, f.app.rows.len() - 5);
        assert_eq!(f.app.cursor, cursor_before);
    }

    #[test]
    fn poll_recovers_when_the_log_is_truncated() {
        let mut f = fixture(&now_line("r", "a.txt"));
        assert!(!f.app.rows.is_empty());
        f._log.as_file().set_len(0).unwrap();
        f._log.as_file().sync_all().unwrap();
        f.app.poll();
        assert!(f.app.rows.is_empty());
    }

    #[test]
    fn poll_retroactively_drops_items_cleared_by_another_instance() {
        let mut f = fixture(&now_line("r", "a.txt"));
        let key = {
            let Row::FileItem { key, .. } = &f.app.rows[f.app.cursor.unwrap()] else {
                panic!("expected FileItem")
            };
            key.clone()
        };
        cleared::mark(&f.app.cleared_path, &key);
        f.app.poll();
        assert!(f.app.rows.is_empty());
    }
}
