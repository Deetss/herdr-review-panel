use crate::actions;
use crate::cleared;
use crate::done;
use crate::log::{self, ParsedLine, Row};
use crate::queue;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long a click-feedback message ("Opened x", "Copied to clipboard") stays in the hint
/// row before reverting to the normal hint - long enough to register as confirmation, short
/// enough that it's gone well before you'd click something else.
const STATUS_TTL: Duration = Duration::from_millis(1500);

/// How long the full-command detail overlay stays open before auto-hiding - a keypress
/// (Esc/Enter/'q') still dismisses it immediately regardless. This overlay exists so
/// mobile users (whose OSC-52 clipboard copy silently fails) can read and select the
/// full command by hand, which a wrapped multi-line command makes a several-second job; the
/// original 2.5s (mirroring STATUS_TTL) took it away mid-read, but 8s sat around long after
/// a short command was already read.
const DETAIL_TTL: Duration = Duration::from_millis(4000);

/// A repeat copy of the same command within this window is treated as a duplicate delivery
/// of the same click, not a deliberate re-copy - long enough to absorb a burst of duplicate
/// events, short enough that an intentional re-copy moments later (say, after a failed
/// paste) still goes through.
const COPY_DEBOUNCE: Duration = Duration::from_millis(500);

/// Column (within the list area, 0-indexed) where a command row's "[ ]"/"[x]" checkbox ends -
/// a 2-space indent then the 4-char box. Clicks before this column toggle done; clicks at or
/// past it copy the command instead. A step label (if any) renders after the checkbox, so it
/// never shifts this boundary. Kept in sync with ui.rs's render_row.
pub const CHECKBOX_END_COL: u16 = 6;

/// How many rows a single scroll-wheel tick moves, independent of cursor position.
pub const WHEEL_SCROLL_LINES: isize = 3;

pub struct App {
    /// The panel body: OPEN sessions, then the DONE drawer. Rebuilt from `entries` whenever
    /// the log, a mark file or the drawer's expansion changes.
    pub rows: Vec<Row>,
    /// Visible log rows grouped by session, before the OPEN/DONE split.
    log_rows: Vec<Row>,
    /// Sessions whose done rows are shown under their summary line. View state only.
    expanded: HashSet<String>,
    pub open_count: usize,
    pub done_count: usize,
    /// Set once the first frame has reported the real body height.
    sized: bool,
    /// Rows hidden by a `hide` event or by supersession, kept for the hidden view.
    hidden_rows: Vec<Row>,
    /// Whether the HIDDEN section is part of the body.
    hidden_view: bool,
    /// Number of hidden rows, whether or not the view is on.
    pub hidden_count: usize,
    /// Identity of the row the cursor was on when the hidden view opened.
    saved_cursor: Option<String>,
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
    /// Every parsed log line, in log order. Rows are derived from this.
    entries: Vec<ParsedLine>,
    done: HashSet<String>,
    done_path: PathBuf,
    /// Replayed cleared log: `true` for a key last hidden, `false` for one last restored.
    cleared: HashMap<String, bool>,
    cleared_path: PathBuf,
    status: Option<(String, Instant)>,
    /// The command currently shown full-screen, if any, alongside when it was shown. The
    /// OSC-52 clipboard copy that activation does cannot reach a phone viewing the terminal
    /// remotely - this is how the command becomes readable there. Desktop still
    /// gets the copy as well. Auto-hides after DETAIL_TTL, like the status toast (see detail_text).
    detail: Option<(String, Instant)>,
    /// "command" or "path": what the detail overlay is currently showing.
    detail_title: &'static str,
    /// Rows visible in the list area as of the last frame - drives ensure_cursor_visible, and
    /// lets scroll_by clamp independently of cursor position instead of fighting it every
    /// frame (a scroll wheel tick should move the view without yanking the cursor along).
    visible_height: usize,
    /// The Orca terminal handle that flagged items into this queue - set from
    /// REVIEW_PANEL_NOTIFY_TARGET, which plugin.sh's `open` threads through at panel-launch
    /// time. None if opened outside Orca.
    notify_target: Option<String>,
    /// Group keys ("{ts}|{session}") already notified this run, so a group isn't re-notified
    /// on every subsequent poll/mutation once it's fully done.
    notified: HashSet<String>,
    /// Set once the last command in the queue is confirmed done, blocking normal input until
    /// resolved via 'y'/'e'/Esc (see report_prompt_active). Checking a box only means it was
    /// run, not that it worked - this is where the human says which before anything gets
    /// reported back to the agent.
    pending_report: bool,
    /// The key and time of the last copy, so a burst of duplicate click events for the same
    /// row (observed under Orca - one physical click can deliver several Down events) copies
    /// to the clipboard once instead of once per duplicate. See COPY_DEBOUNCE.
    last_copy: Option<(String, Instant)>,
}

impl App {
    pub fn new(log_path: PathBuf, notify_target: Option<String>) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut app = Self::with_paths(log_path, home, done::path(), cleared::path());
        app.notify_target = notify_target;
        app
    }

    /// Everything `new` derives from globals (env `HOME`, `~/.claude/review-{done,cleared}.log`)
    /// is a plain parameter here instead, so tests can point them at tempfiles without racing
    /// real user state or each other over shared env vars.
    pub fn with_paths(
        log_path: PathBuf,
        home: String,
        done_path: PathBuf,
        cleared_path: PathBuf,
    ) -> Self {
        let done = done::load(&done_path);
        let cleared = cleared::load(&cleared_path);
        let mut entries = Vec::new();
        let offset = log::poll_tail(&log_path, 0, &mut entries);
        let mut app = App {
            rows: Vec::new(),
            log_rows: Vec::new(),
            expanded: HashSet::new(),
            open_count: 0,
            done_count: 0,
            sized: false,
            hidden_rows: Vec::new(),
            hidden_view: false,
            hidden_count: 0,
            saved_cursor: None,
            cursor: None,
            scroll: 0,
            close_hovered: false,
            clear_all_hovered: false,
            detail: None,
            detail_title: "command",
            should_quit: false,
            log_path,
            offset,
            home,
            entries,
            done,
            done_path,
            cleared,
            cleared_path,
            status: None,
            visible_height: 1,
            notify_target: None,
            notified: HashSet::new(),
            pending_report: false,
            last_copy: None,
        };
        app.rebuild();
        app.jump_to_latest();
        app
    }

    /// Re-derives the visible and hidden rows from the entries and the replayed mark files.
    fn rebuild(&mut self) {
        let old_cursor = self.cursor;
        let old_identity = old_cursor
            .and_then(|c| self.rows.get(c))
            .and_then(queue::identity);
        let rows = log::build_rows(&self.entries, &self.home, &self.cleared);
        self.log_rows = rows.visible;
        self.hidden_rows = rows.hidden;
        let body = queue::build_body(&self.log_rows, &self.done, &self.expanded);
        self.rows = body.rows;
        self.open_count = body.open;
        self.done_count = body.done;
        self.hidden_count = self
            .hidden_rows
            .iter()
            .filter(|r| row_key(r).is_some())
            .count();
        if self.hidden_view {
            queue::append_hidden(&mut self.rows, &self.hidden_rows);
        }
        self.cursor = match (old_cursor, old_identity) {
            (Some(_), Some(id)) => self
                .rows
                .iter()
                .position(|r| queue::identity(r).as_deref() == Some(id.as_str()))
                .or_else(|| self.nearest_target(old_cursor.unwrap_or(0))),
            (Some(c), None) => Some(c).filter(|&c| c < self.rows.len()),
            (None, _) => None,
        };
        self.ensure_cursor_visible();
    }

    /// The target at or before `idx`, else the first one after it. Where the cursor lands
    /// when its row moved out of view, e.g. a command just marked done.
    fn nearest_target(&self, idx: usize) -> Option<usize> {
        let targets: Vec<usize> = self.activatable_rows().collect();
        targets
            .iter()
            .rev()
            .find(|&&t| t <= idx)
            .or_else(|| targets.first())
            .copied()
    }

    /// Polls the log for newly-appended lines and reloads the done/cleared marks files, so
    /// changes made from another instance of this panel show up here too. Returns true if
    /// anything changed (so the caller knows to redraw immediately rather than waiting for
    /// the next tick).
    pub fn poll(&mut self) -> bool {
        let before = self.rows.len();
        let offset_before = self.offset;
        self.offset = log::poll_tail(&self.log_path, self.offset, &mut self.entries);
        let log_changed = self.offset != offset_before;
        let new_done = done::load(&self.done_path);
        let new_cleared = cleared::load(&self.cleared_path);
        let marks_changed = new_done != self.done || new_cleared != self.cleared;
        if !log_changed && !marks_changed {
            return false;
        }
        self.done = new_done;
        self.cleared = new_cleared;
        self.rebuild();
        // A supersession can leave the row count unchanged, so new log bytes count too.
        if log_changed || self.rows.len() != before {
            self.jump_to_latest();
        }
        true
    }

    pub fn is_done(&self, key: &str) -> bool {
        self.done.contains(key)
    }

    /// Called every frame with the terminal's current height, but only re-clamps scroll on an
    /// actual change (a resize) - otherwise this would re-run ensure_cursor_visible on every
    /// single frame and silently undo any independent scroll-wheel movement.
    pub fn set_visible_height(&mut self, height: usize) {
        let height = height.max(1);
        if !self.sized {
            self.sized = true;
            self.visible_height = height;
            self.default_scroll();
        } else if height != self.visible_height {
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

    pub fn detail_title(&self) -> &'static str {
        self.detail_title
    }

    /// Marks an item done for display purposes (struck through) - shared by copying and by
    /// the checkbox. Does *not* report anything back to the agent; see confirm_done for
    /// that. Returns whether this call is what actually marked it (false for an already-done
    /// key), so confirm_done only ever reports once.
    fn mark_done(&mut self, key: &str) -> bool {
        if self.done.insert(key.to_string()) {
            done::mark(&self.done_path, key);
            true
        } else {
            false
        }
    }

    /// The checkbox's undo - appends an `open` event for a done item (see toggle_done, the
    /// only caller). Also drops the item's group from `notified`, so if it's re-confirmed
    /// later and the group becomes fully done again, the agent still gets notified instead
    /// of that being silently swallowed by the earlier notification.
    fn unmark_done(&mut self, key: &str) {
        if !self.done.remove(key) {
            return;
        }
        done::open(&self.done_path, key);
        let group_key = key.splitn(3, '|').take(2).collect::<Vec<_>>().join("|");
        self.notified.remove(&group_key);
        self.set_status("Unchecked");
    }

    /// The checkbox's "I actually ran this and it worked" confirmation, as opposed to just
    /// copying it - copying only takes the command to run, so notifying the agent that a
    /// *group* is done at that point would be premature (you might still hit an error worth
    /// reporting some other way instead of nothing being wrong at all). Only this path
    /// triggers that per-group notify; see maybe_prompt_report for the queue-wide finish,
    /// which copying can trigger too since it still asks the human first either way.
    fn confirm_done(&mut self, key: &str) {
        if !self.mark_done(key) {
            return;
        }
        if !self.maybe_prompt_report() {
            self.check_group_complete(key);
        }
    }

    /// If marking something done just finished the whole queue, ask the human whether it all
    /// went well before anything gets reported (see report_prompt_active) - regardless of
    /// whether the last item was actually confirmed via checkbox or merely copied, since
    /// either way the prompt itself is what stands between this and reporting to the agent,
    /// not the checkbox. Shared by confirm_done and the copy path in run_activation, which
    /// used to only ever reach this via the checkbox - leaving a fully-copied queue with no
    /// way to close out was the "isn't reliably showing up" bug this fixes. Returns whether
    /// it fired, so callers can skip whatever they'd otherwise show in the same screen space
    /// (the detail overlay, e.g.) once the prompt has claimed it instead.
    fn maybe_prompt_report(&mut self) -> bool {
        if !self.all_commands_done() {
            return false;
        }
        self.pending_report = true;
        self.detail = None;
        true
    }

    /// The checkbox's full behavior: marks a not-yet-done command done (confirm_done), or
    /// reopens an already-done one (unmark_done).
    fn toggle_done(&mut self, key: &str) {
        if self.done.contains(key) {
            self.unmark_done(key);
        } else {
            self.confirm_done(key);
        }
        // A done row changes sections, so the body is re-derived.
        self.rebuild();
    }

    pub fn report_prompt_active(&self) -> bool {
        self.pending_report
    }

    /// 'y'/Enter on the finished-queue prompt: report success and clear/close as normal.
    pub fn confirm_report_success(&mut self) {
        self.pending_report = false;
        self.clear_and_close(Some("Finished the review queue - continue."));
    }

    /// 'e' on the finished-queue prompt: report a problem instead of success, and leave the
    /// queue open - clearing it too would throw away exactly what's needed to go back and
    /// re-check once the error's sorted out.
    pub fn confirm_report_error(&mut self) {
        self.pending_report = false;
        if let Some(target) = &self.notify_target {
            actions::notify_agent(
                target,
                "Ran into an error working through the review queue - check in.",
            );
        }
    }

    /// Esc on the finished-queue prompt: defer for now, no report, nothing else changes.
    pub fn dismiss_report_prompt(&mut self) {
        self.pending_report = false;
    }

    /// True once every CommandItem row anywhere in the queue is done, and there was at least
    /// one to begin with (a file-only queue never counts as "done" this way - files have no
    /// checkbox, so only an explicit clear closes those out).
    fn all_commands_done(&self) -> bool {
        let mut has_command = false;
        let all_done = self.log_rows.iter().all(|row| match row {
            Row::CommandItem { key, .. } => {
                has_command = true;
                self.done.contains(key)
            }
            _ => true,
        });
        has_command && all_done
    }

    /// Finds the open rows of the session that owns `idx`: from the nearest preceding
    /// GroupHeader through to the first row that is not an item.
    fn group_range(&self, idx: usize) -> Option<(usize, usize)> {
        let start = self.rows[..idx]
            .iter()
            .rposition(|r| matches!(r, Row::GroupHeader { .. }))?
            + 1;
        let end = self.rows[start..]
            .iter()
            .position(|r| !matches!(r, Row::FileItem { .. } | Row::CommandItem { .. }))
            .map(|i| start + i)
            .unwrap_or(self.rows.len());
        Some((start, end))
    }

    /// Notifies the agent that flagged this group once every command in it is checked off -
    /// called right after mark_done, so it only ever fires on the transition into "fully
    /// done", not on every poll (see notify_target's doc comment on App).
    fn check_group_complete(&mut self, key: &str) {
        let Some(target) = self.notify_target.clone() else {
            return;
        };
        // "{ts}|{session}" - the key's first two segments (see ParsedLine::key) - identifies
        // the hook invocation that flagged this row. A session group can span several, and
        // each is notified on its own once its commands are all done.
        let group_key = key.splitn(3, '|').take(2).collect::<Vec<_>>().join("|");
        if self.notified.contains(&group_key) {
            return;
        }
        let Some(idx) = self.rows.iter().position(|r| row_key(r) == Some(key)) else {
            return;
        };
        let Some((start, end)) = self.group_range(idx) else {
            return;
        };
        let prefix = format!("{group_key}|");
        let mut has_command = false;
        let all_done = self.rows[start..end].iter().all(|row| match row {
            Row::CommandItem { key: cmd_key, .. } if cmd_key.starts_with(&prefix) => {
                has_command = true;
                self.done.contains(cmd_key)
            }
            _ => true,
        });
        if !has_command || !all_done {
            return;
        }
        let repo = match self.rows.get(start - 1) {
            Some(Row::GroupHeader { repo, .. }) => repo.clone(),
            _ => return,
        };
        self.notified.insert(group_key);
        actions::notify_agent(
            &target,
            &format!("Finished the review items from {repo} - continue."),
        );
    }

    /// Appends a `hide` event for an item. The caller rebuilds `rows` once it has hidden
    /// everything it means to.
    fn mark_cleared(&mut self, key: &str) {
        if self.cleared.get(key) != Some(&true) {
            cleared::hide(&self.cleared_path, key);
            self.cleared.insert(key.to_string(), true);
        }
    }

    /// Rows the keyboard cursor can land on. Session headers are reachable by mouse only. The
    /// hidden view limits the cursor to the hidden rows, whose one action is restore.
    fn activatable_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().enumerate().filter_map(|(i, r)| {
            let reachable = if self.hidden_view {
                is_hidden_item(r)
            } else {
                matches!(
                    r,
                    Row::FileItem { .. } | Row::CommandItem { .. } | Row::Summary { .. }
                )
            };
            reachable.then_some(i)
        })
    }

    pub fn hidden_view(&self) -> bool {
        self.hidden_view
    }

    /// Index of the section label that starts with `prefix`.
    fn section_label(&self, prefix: &str) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| matches!(r, Row::Section { label } if label.starts_with(prefix)))
    }

    /// Index of the DONE label, if the drawer exists.
    fn done_label(&self) -> Option<usize> {
        self.section_label("DONE")
    }

    /// The newest open row, or the first DONE summary when nothing is open. In the hidden
    /// view, the first hidden row.
    fn jump_to_latest(&mut self) {
        if self.hidden_view {
            let first = self.activatable_rows().next();
            self.cursor = first;
            self.default_scroll();
            return;
        }
        let open_end = self.done_label().unwrap_or(self.rows.len());
        self.cursor = self
            .activatable_rows()
            .take_while(|&i| i < open_end)
            .last()
            .or_else(|| self.activatable_rows().next());
        self.default_scroll();
    }

    /// The smallest scroll offset that shows the cursor and, when both fit, the DONE label.
    fn default_scroll(&mut self) {
        let height = self.visible_height;
        if self.hidden_view {
            // Show the whole HIDDEN section when it fits, else start it at the top.
            let label = self.section_label("HIDDEN").unwrap_or(0);
            self.scroll = self.rows.len().saturating_sub(height).min(label);
            self.ensure_cursor_visible();
            return;
        }
        let Some(cursor) = self.cursor else {
            self.scroll = 0;
            return;
        };
        let mut scroll = (cursor + 1).saturating_sub(height);
        if let Some(label) = self.done_label() {
            let both = cursor.max(label) + 1;
            let candidate = both.saturating_sub(height);
            if candidate <= cursor.min(label) {
                scroll = candidate;
            }
        }
        self.scroll = scroll;
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
        if let Some(idx) = self.cursor {
            self.activate_row(idx);
        }
    }

    fn activate_row(&mut self, idx: usize) {
        match self.rows.get(idx) {
            Some(Row::Summary { group, .. }) => {
                let group = group.clone();
                if !self.expanded.remove(&group) {
                    self.expanded.insert(group);
                }
                self.rebuild();
            }
            Some(row) => {
                if let Some(activation) = plan_activation(row) {
                    self.run_activation(activation);
                }
            }
            None => {}
        }
    }

    /// Space/'d' (keyboard) or a click on the checkbox column (mouse): toggle the command at
    /// the cursor done/not-done. No-op for file rows, which have no done state.
    pub fn toggle_done_cursor(&mut self) {
        if let Some(Row::CommandItem {
            key, hidden: None, ..
        }) = self.cursor.and_then(|c| self.rows.get(c))
        {
            let key = key.clone();
            self.toggle_done(&key);
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
                // A hidden row's one action is restore.
                if is_hidden_item(row) {
                    return;
                }
                if let Some(key) = row_key(row) {
                    let key = key.to_string();
                    self.mark_cleared(&key);
                }
            }
            Some(Row::GroupHeader { keys, .. } | Row::Summary { keys, .. }) => {
                for key in keys.clone() {
                    self.mark_cleared(&key);
                }
            }
            _ => return,
        }
        self.rebuild();
        self.jump_to_latest();
    }

    /// `c`: hides every done row in every session.
    pub fn clear_done(&mut self) {
        let keys: Vec<String> = self
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Summary { keys, .. } => Some(keys.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        for key in keys {
            self.mark_cleared(&key);
        }
        self.rebuild();
        self.jump_to_latest();
    }

    /// `S`: hides every row, open and done, of the session the cursor is in.
    pub fn clear_session(&mut self) {
        let Some(idx) = self.cursor else { return };
        for key in self.session_keys(idx) {
            self.mark_cleared(&key);
        }
        self.rebuild();
        self.jump_to_latest();
    }

    /// Keys of the visible rows in the session that owns row `idx`. Empty for rows outside
    /// OPEN and DONE, including everything in the HIDDEN section.
    fn session_keys(&self, idx: usize) -> Vec<String> {
        let Some(row) = self.rows.get(idx) else {
            return Vec::new();
        };
        if matches!(
            row,
            Row::Blank | Row::Section { .. } | Row::Note(_) | Row::HiddenHeader { .. }
        ) || is_hidden_item(row)
        {
            return Vec::new();
        }
        let owner = self.rows[..=idx]
            .iter()
            .rposition(|r| matches!(r, Row::GroupHeader { .. } | Row::Summary { .. }));
        match owner.and_then(|i| self.rows.get(i)) {
            Some(Row::GroupHeader { keys, .. }) => keys.clone(),
            Some(Row::Summary { group, .. }) => self
                .log_rows
                .iter()
                .find_map(|r| match r {
                    Row::GroupHeader {
                        session,
                        repo,
                        keys,
                        ..
                    } if queue::group_id(session, repo) == *group => Some(keys.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// `H`: shows or hides the HIDDEN section. Opening it moves the cursor to the first hidden
    /// row; closing it puts the cursor back on the row it left.
    pub fn toggle_hidden_view(&mut self) {
        if self.hidden_view {
            self.hidden_view = false;
            let saved = self.saved_cursor.take();
            self.rebuild();
            self.cursor = saved
                .and_then(|id| {
                    self.rows
                        .iter()
                        .position(|r| queue::identity(r).as_deref() == Some(id.as_str()))
                })
                .or(self.cursor);
            self.default_scroll();
        } else {
            self.saved_cursor = self
                .cursor
                .and_then(|c| self.rows.get(c))
                .and_then(queue::identity);
            self.hidden_view = true;
            self.rebuild();
            self.jump_to_latest();
        }
    }

    /// `Esc`: leaves the hidden view, else closes the panel.
    pub fn escape(&mut self) {
        if self.hidden_view {
            self.toggle_hidden_view();
        } else {
            self.click_close();
        }
    }

    /// `r`: restores the hidden row under the cursor, or a whole session from its header.
    pub fn restore_cursor(&mut self) {
        if let Some(idx) = self.cursor {
            self.restore_at(idx);
        }
    }

    fn restore_at(&mut self, idx: usize) {
        let keys: Vec<String> = match self.rows.get(idx) {
            Some(row) if is_hidden_item(row) => {
                row_key(row).map(str::to_string).into_iter().collect()
            }
            Some(Row::HiddenHeader { keys, .. }) => keys.clone(),
            _ => return,
        };
        // Where the cursor sits among the hidden rows, so it can stay near that spot.
        let nth = self.rows[..idx]
            .iter()
            .filter(|r| is_hidden_item(r))
            .count();
        self.show_keys(&keys);
        let remaining: Vec<usize> = self.activatable_rows().collect();
        self.cursor = remaining
            .get(nth.min(remaining.len().saturating_sub(1)))
            .copied();
        self.ensure_cursor_visible();
    }

    /// `R` and the `restore` button: restores every hidden row.
    pub fn restore_all(&mut self) {
        let keys: Vec<String> = self
            .hidden_rows
            .iter()
            .filter_map(|r| row_key(r).map(str::to_string))
            .collect();
        self.show_keys(&keys);
        let first = self.activatable_rows().next();
        self.cursor = first;
        self.default_scroll();
    }

    fn show_keys(&mut self, keys: &[String]) {
        for key in keys {
            cleared::show(&self.cleared_path, key);
            self.cleared.insert(key.clone(), false);
        }
        self.rebuild();
    }

    /// `list_width` is the rendered row width - clicking its rightmost column always clears
    /// that row (or, for a GroupHeader, its whole section), matching the trailing "x" ui.rs
    /// draws there. Anywhere else on a command row before CHECKBOX_END_COL toggles done;
    /// past it (or anywhere on a file row) opens/copies as usual.
    pub fn click_row(&mut self, row_idx: usize, local_col: u16, list_width: u16) {
        if self.rows.get(row_idx).is_some_and(is_hidden_item) {
            if local_col < CHECKBOX_END_COL {
                self.restore_at(row_idx);
            }
            return;
        }
        if local_col + 1 >= list_width {
            self.clear_row_at(row_idx);
            return;
        }
        match self.rows.get(row_idx) {
            Some(Row::CommandItem { key, .. }) if local_col < CHECKBOX_END_COL => {
                let key = key.clone();
                self.toggle_done(&key);
            }
            Some(_) => self.activate_row(row_idx),
            None => {}
        }
    }

    /// The top "clear" button and `C`: appends a `hide` event for every visible row, open and
    /// done (the logs themselves are never cut). The panel stays open on the empty state.
    pub fn click_clear_all(&mut self) {
        if self.hidden_view {
            self.restore_all();
            return;
        }
        let keys: Vec<String> = self
            .log_rows
            .iter()
            .filter_map(|r| row_key(r).map(str::to_string))
            .collect();
        for key in keys {
            self.mark_cleared(&key);
        }
        self.rebuild();
        self.cursor = None;
        self.scroll = 0;
    }

    /// The finish prompt's "yes": hides the queue, notifies the flagging agent, then closes
    /// the panel the same way click_close does.
    fn clear_and_close(&mut self, notify_message: Option<&str>) {
        let _ = actions::clear_all(&self.log_path, &self.cleared_path);
        if let (Some(message), Some(target)) = (notify_message, self.notify_target.as_ref()) {
            actions::notify_agent(target, message);
        }
        actions::close();
        self.should_quit = true;
    }

    /// Runs the action and leaves a status message behind - opening an external editor can
    /// take a visible moment, so the click needs its own immediate confirmation that isn't
    /// waiting on that.
    fn run_activation(&mut self, activation: Activation) {
        match activation {
            Activation::OpenFile(path) => {
                let opener = actions::open_item(&path);
                // List rows keep only a path's tail on a narrow pane, so show and copy the
                // whole thing; the overlay wraps and the clipboard has it for pasting.
                actions::copy_to_clipboard(&path);
                self.detail = Some((path.clone(), Instant::now()));
                self.detail_title = "path";
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
            Activation::CopyCommand { command, key } => {
                let is_duplicate = self
                    .last_copy
                    .as_ref()
                    .is_some_and(|(_, at)| at.elapsed() < COPY_DEBOUNCE);
                if is_duplicate {
                    return;
                }
                self.last_copy = Some((key.clone(), Instant::now()));
                actions::copy_to_clipboard(&command);
                // Copying is the act of taking the command to run it; the checkbox is for
                // the ones run some other way. mark_done, not confirm_done or toggle_done -
                // a re-copy (say, after a failed paste) still never unmarks it; only the
                // checkbox itself can undo the mark this leaves behind (see toggle_done).
                self.mark_done(&key);
                // If that was the last command in the queue, the finish prompt now owns the
                // screen - don't also show the detail overlay underneath it.
                if !self.maybe_prompt_report() {
                    self.detail = Some((command, Instant::now()));
                    self.detail_title = "command";
                }
                self.set_status("Copied to clipboard");
                // Copying marks the row done, which moves it into the drawer.
                self.rebuild();
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
        Row::Blank
        | Row::GroupHeader { .. }
        | Row::HiddenHeader { .. }
        | Row::Section { .. }
        | Row::Note(_)
        | Row::Summary { .. } => None,
    }
}

fn is_hidden_item(row: &Row) -> bool {
    matches!(
        row,
        Row::FileItem {
            hidden: Some(_),
            ..
        } | Row::CommandItem {
            hidden: Some(_),
            ..
        }
    )
}

enum Activation {
    OpenFile(String),
    CopyCommand { command: String, key: String },
}

fn plan_activation(row: &Row) -> Option<Activation> {
    match row {
        Row::FileItem {
            abspath: Some(path),
            hidden: None,
            ..
        } => Some(Activation::OpenFile(path.clone())),
        Row::CommandItem {
            command,
            key,
            hidden: None,
            ..
        } => Some(Activation::CopyCommand {
            command: command.clone(),
            key: key.clone(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
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

    /// Key of the first command in the log, wherever the body currently shows it.
    fn first_command_key(app: &App) -> String {
        app.log_rows
            .iter()
            .find_map(|r| match r {
                Row::CommandItem { key, .. } => Some(key.clone()),
                _ => None,
            })
            .expect("a command row")
    }

    fn has_summary(app: &App) -> bool {
        app.rows.iter().any(|r| matches!(r, Row::Summary { .. }))
    }

    /// Two lines sharing one explicit timestamp, so grouping-dependent tests can't flake on a
    /// second boundary landing between two separate `Local::now()` calls.
    fn two_line_group(repo: &str, item_a: &str, item_b: &str) -> String {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        format!(
            "{ts} session=s repo={repo} cwd=/home/d item={item_a}\n{ts} session=s repo={repo} cwd=/home/d item={item_b}\n"
        )
    }

    /// Same as two_line_group, but both lines are commands - for tests that need to act on
    /// one command without that also being the last one in the queue.
    fn two_command_group(repo: &str, cmd_a: &str, cmd_b: &str) -> String {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        format!(
            "{ts} session=s repo={repo} cwd=/home/d kind=command item={cmd_a}\n{ts} session=s repo={repo} cwd=/home/d kind=command item={cmd_b}\n"
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
    fn copying_a_command_marks_it_done_and_persists_it() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let idx = f.app.cursor.unwrap();
        let key = first_command_key(&f.app);
        f.app.click_row(idx, CHECKBOX_END_COL + 2, 80); // on the command text: copy
        assert!(f.app.is_done(&key));
        assert!(done::load(&f.app.done_path).contains(&key));
        assert_eq!(f.app.status_text(), Some("Copied to clipboard"));
    }

    #[test]
    fn a_rapid_duplicate_copy_is_suppressed_but_a_later_one_is_not() {
        // Three commands, not one or two - copying the last remaining command would also
        // trigger the finish prompt (see maybe_prompt_report) and clear `detail`, which isn't
        // what this test is about.
        let mut f = fixture(&format!(
            "{}{}{}",
            now_command_line("r", "echo a"),
            now_command_line("r", "echo b"),
            now_command_line("r", "echo c")
        ));
        f.app.cursor = Some(2); // "echo a"
        let idx = f.app.cursor.unwrap();
        f.app.click_row(idx, CHECKBOX_END_COL + 2, 80); // first copy
        let first_detail_at = f.app.detail.as_ref().unwrap().1;

        // The copied row left OPEN, so the duplicate lands on the next one. Within
        // COPY_DEBOUNCE that must be a no-op and leave detail's timestamp as it was.
        f.app.click_row(idx, CHECKBOX_END_COL + 2, 80);
        assert_eq!(f.app.detail.as_ref().unwrap().1, first_detail_at);

        // No real sleep: back-date the in-memory timer past COPY_DEBOUNCE directly.
        f.app.last_copy = f
            .app
            .last_copy
            .take()
            .map(|(key, _)| (key, Instant::now() - COPY_DEBOUNCE * 2));
        f.app.click_row(idx, CHECKBOX_END_COL + 2, 80);
        assert_ne!(f.app.detail.as_ref().unwrap().1, first_detail_at);
    }

    #[test]
    fn toggle_done_marks_a_command_and_persists_it() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let key = first_command_key(&f.app);
        f.app.toggle_done_cursor();
        assert!(f.app.is_done(&key));
        assert!(done::load(&f.app.done_path).contains(&key));
    }

    #[test]
    fn toggle_done_cursor_marks_then_reopens_with_an_open_event() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let key = first_command_key(&f.app);
        f.app.toggle_done_cursor();
        assert!(f.app.is_done(&key));
        // The row left OPEN; the cursor falls to the drawer's summary, which Enter expands.
        f.app.activate_cursor();
        f.app.cursor = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, Row::CommandItem { .. }));
        f.app.toggle_done_cursor();
        assert!(!f.app.is_done(&key));
        assert!(!done::load(&f.app.done_path).contains(&key));
        let written = std::fs::read_to_string(&f.app.done_path).unwrap();
        assert_eq!(
            written.lines().count(),
            2,
            "done then open, nothing rewritten"
        );
    }

    #[test]
    fn unchecking_a_copied_item_untracks_it() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let idx = f.app.cursor.unwrap();
        let key = first_command_key(&f.app);
        f.app.click_row(idx, CHECKBOX_END_COL, 80);
        assert!(f.app.is_done(&key), "copy should have marked it done");
        f.app.activate_cursor(); // expand the drawer so the done row is clickable
        let done_idx = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, Row::CommandItem { .. }))
            .unwrap();
        f.app.click_row(done_idx, 0, 80); // inside the checkbox column
        assert!(!f.app.is_done(&key), "checkbox should undo the copy's mark");
    }

    #[test]
    fn checking_off_the_last_command_asks_before_reporting() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        assert!(f.app.report_prompt_active());
        assert!(!f.app.should_quit);
    }

    #[test]
    fn copying_the_last_command_still_asks_to_report() {
        // Copying alone used to never reach the finish prompt, leaving a fully-copied queue
        // with no way to close out - see maybe_prompt_report.
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let idx = f.app.cursor.unwrap();
        f.app.click_row(idx, CHECKBOX_END_COL + 2, 80); // on the command text: copy
        assert!(f.app.report_prompt_active());
        assert!(!f.app.should_quit);
        // The prompt now owns the screen instead of the copy's detail overlay.
        assert!(f.app.detail_text().is_none());
    }

    #[test]
    fn confirming_the_report_prompt_quits() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        f.app.confirm_report_success();
        assert!(!f.app.report_prompt_active());
        assert!(f.app.should_quit);
    }

    #[test]
    fn reporting_an_error_leaves_the_queue_open() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        f.app.confirm_report_error();
        assert!(!f.app.report_prompt_active());
        assert!(!f.app.should_quit);
        assert!(has_summary(&f.app), "the row stays in the DONE drawer");
    }

    #[test]
    fn dismissing_the_report_prompt_leaves_everything_as_is() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        f.app.dismiss_report_prompt();
        assert!(!f.app.report_prompt_active());
        assert!(!f.app.should_quit);
        assert!(has_summary(&f.app));
    }

    #[test]
    fn clicking_the_checkbox_column_toggles_done_and_moves_the_row_to_the_drawer() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let idx = f.app.cursor.unwrap();
        let key = first_command_key(&f.app);
        f.app.click_row(idx, 0, 80); // inside the checkbox (columns 0..CHECKBOX_END_COL)
        assert!(f.app.is_done(&key));
        assert!(has_summary(&f.app));
        assert!(
            !f.app
                .rows
                .iter()
                .any(|r| matches!(r, Row::CommandItem { .. }))
        );
    }

    #[test]
    fn a_done_row_stays_in_the_drawer_after_a_poll() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        f.app.poll();
        // OPEN, "Nothing open.", blank, DONE, one summary. No time-based move.
        assert_eq!(f.app.rows.len(), 5);
        assert_eq!((f.app.open_count, f.app.done_count), (0, 1));
    }

    #[test]
    fn enter_on_a_summary_expands_and_folds_its_done_rows() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        f.app.toggle_done_cursor();
        let summary = f.app.cursor.unwrap();
        assert!(matches!(
            f.app.rows[summary],
            Row::Summary {
                expanded: false,
                ..
            }
        ));
        f.app.activate_cursor();
        assert_eq!(f.app.cursor, Some(summary));
        assert!(matches!(
            f.app.rows[summary],
            Row::Summary { expanded: true, .. }
        ));
        assert!(matches!(f.app.rows[summary + 1], Row::CommandItem { .. }));
        f.app.activate_cursor();
        assert_eq!(f.app.rows.len(), summary + 1);
    }

    #[test]
    fn c_hides_done_rows_and_keeps_open_ones() {
        let mut f = fixture(&two_command_group("r", "echo a", "echo b"));
        f.app.toggle_done_cursor(); // "echo b", the newest
        f.app.clear_done();
        assert_eq!((f.app.open_count, f.app.done_count), (1, 0));
        let hides = std::fs::read_to_string(&f.app.cleared_path).unwrap();
        assert_eq!(hides.lines().filter(|l| l.contains(" hide ")).count(), 1);
    }

    #[test]
    fn the_clear_button_hides_everything_and_leaves_the_panel_open() {
        let mut f = fixture(&two_command_group("r", "echo a", "echo b"));
        f.app.toggle_done_cursor();
        f.app.click_clear_all();
        assert!(f.app.rows.is_empty());
        assert!(!f.app.should_quit);
        assert_eq!(f.app.cursor, None);
    }

    #[test]
    fn rows_done_before_launch_load_as_done_in_place() {
        let mut log = tempfile::NamedTempFile::new().unwrap();
        write!(log, "{}", now_command_line("r", "echo hi")).unwrap();
        log.flush().unwrap();
        let done = tempfile::NamedTempFile::new().unwrap();
        let cleared = tempfile::NamedTempFile::new().unwrap();
        let open = |done: &tempfile::NamedTempFile| {
            App::with_paths(
                log.path().to_path_buf(),
                "/home/d".to_string(),
                done.path().to_path_buf(),
                cleared.path().to_path_buf(),
            )
        };

        let key = first_command_key(&open(&done));
        done::mark(done.path(), &key);

        let app = open(&done);
        assert!(app.is_done(&key));
        assert!(has_summary(&app));
        // Nothing is open, so the cursor starts on the drawer's summary line.
        assert!(matches!(app.rows[app.cursor.unwrap()], Row::Summary { .. }));
    }

    #[test]
    fn clearing_a_session_header_hides_every_row_of_that_session() {
        let mut f = fixture(&two_command_group("r", "echo a", "echo b"));
        f.app.cursor = Some(1);
        f.app.clear_cursor();
        assert!(f.app.rows.is_empty());
        let hides = std::fs::read_to_string(&f.app.cleared_path).unwrap();
        assert_eq!(hides.lines().filter(|l| l.contains(" hide ")).count(), 2);
    }

    #[test]
    fn clicking_past_the_checkbox_copies_and_a_second_copy_never_unmarks() {
        let mut f = fixture(&now_command_line("r", "echo hi"));
        let idx = f.app.cursor.unwrap();
        let key = first_command_key(&f.app);
        f.app.click_row(idx, CHECKBOX_END_COL, 80);
        f.app.activate_cursor(); // expand the drawer
        let done_idx = f
            .app
            .rows
            .iter()
            .position(|r| matches!(r, Row::CommandItem { .. }))
            .unwrap();
        f.app.last_copy = None;
        f.app.click_row(done_idx, CHECKBOX_END_COL, 80);
        assert!(
            f.app.is_done(&key),
            "copying is a one-way done mark, unlike the checkbox toggle"
        );
        assert_eq!(f.app.status_text(), Some("Copied to clipboard"));
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
        // The header still has an item under it (b.txt) - only a.txt itself is gone.
        assert_eq!(f.app.rows.len(), 3);
        assert!(matches!(&f.app.rows[0], Row::Section { label } if label == "OPEN"));
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
    fn poll_recovers_when_the_log_is_replaced_externally() {
        let mut f = fixture(&now_line("r", "a.txt"));
        assert!(!f.app.rows.is_empty());
        std::fs::write(f._log.path(), "").unwrap();
        f.app.poll();
        assert!(f.app.rows.is_empty());
    }

    #[test]
    fn a_hide_then_show_event_round_trips_through_poll() {
        let mut f = fixture(&now_line("r", "a.txt"));
        let key = {
            let Row::FileItem { key, .. } = &f.app.rows[f.app.cursor.unwrap()] else {
                panic!("expected FileItem")
            };
            key.clone()
        };
        cleared::hide(&f.app.cleared_path, &key);
        f.app.poll();
        assert!(f.app.rows.is_empty());
        assert_eq!(f.app.hidden_rows.len(), 3);
        cleared::show(&f.app.cleared_path, &key);
        f.app.poll();
        assert_eq!(f.app.rows.len(), 3);
        assert!(f.app.hidden_rows.is_empty());
    }

    fn command_at(ts: &str, session: &str, repo: &str, item: &str) -> String {
        format!("{ts} session={session} repo={repo} cwd=/home/d kind=command item={item}\n")
    }

    /// Two sessions. `echo a` is posted twice in s1, so the first copy is superseded.
    const SUPERSEDED_LOG: [(&str, &str, &str, &str); 4] = [
        ("2026-11-05 09:00:00", "s1", "r1", "echo a"),
        ("2026-11-05 09:01:00", "s1", "r1", "echo a"),
        ("2026-11-05 09:02:00", "s2", "r2", "echo b"),
        ("2026-11-05 09:03:00", "s2", "r2", "echo c"),
    ];

    fn superseded_fixture() -> Fixture {
        let log: String = SUPERSEDED_LOG
            .iter()
            .map(|(ts, s, r, i)| command_at(ts, s, r, i))
            .collect();
        fixture(&log)
    }

    fn show_events(app: &App) -> usize {
        std::fs::read_to_string(&app.cleared_path)
            .unwrap()
            .lines()
            .filter(|l| l.contains(" show "))
            .count()
    }

    fn open_commands(app: &App) -> Vec<String> {
        app.rows
            .iter()
            .filter_map(|r| match r {
                Row::CommandItem {
                    command,
                    key,
                    hidden: None,
                    ..
                } if !app.is_done(key) => Some(format!("{command}@{}", &key[11..16])),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_default_view_never_shows_hidden_rows() {
        let f = superseded_fixture();
        assert_eq!((f.app.open_count, f.app.hidden_count), (3, 1));
        assert!(!f.app.hidden_view());
        assert!(f.app.rows.iter().all(|r| !matches!(
            r,
            Row::HiddenHeader { .. }
                | Row::CommandItem {
                    hidden: Some(_),
                    ..
                }
        )));
        assert!(f.app.section_label("HIDDEN").is_none());
    }

    #[test]
    fn h_shows_the_hidden_section_and_h_again_hides_it_and_returns_the_cursor() {
        let mut f = superseded_fixture();
        let before = f.app.cursor;
        let before_id = queue::identity(&f.app.rows[before.unwrap()]);

        f.app.toggle_hidden_view();
        assert!(f.app.hidden_view());
        let label = f.app.section_label("HIDDEN (1)").expect("HIDDEN label");
        assert!(matches!(f.app.rows[label - 1], Row::Blank));
        let cursor = f.app.cursor.unwrap();
        assert!(cursor > label);
        assert!(matches!(
            &f.app.rows[cursor],
            Row::CommandItem {
                hidden: Some(crate::log::Hidden::Superseded),
                ..
            }
        ));

        f.app.toggle_hidden_view();
        assert!(!f.app.hidden_view());
        assert!(f.app.section_label("HIDDEN").is_none());
        assert_eq!(f.app.cursor, before);
        assert_eq!(
            queue::identity(&f.app.rows[f.app.cursor.unwrap()]),
            before_id
        );
    }

    #[test]
    fn escape_leaves_the_hidden_view_without_closing_the_panel() {
        let mut f = superseded_fixture();
        f.app.toggle_hidden_view();
        f.app.escape();
        assert!(!f.app.hidden_view());
        assert!(!f.app.should_quit);
    }

    #[test]
    fn r_restores_a_superseded_row_beside_its_replacement() {
        let mut f = superseded_fixture();
        f.app.toggle_hidden_view();
        f.app.restore_cursor();

        assert_eq!(show_events(&f.app), 1);
        assert_eq!((f.app.open_count, f.app.hidden_count), (4, 0));
        f.app.toggle_hidden_view();
        let open = open_commands(&f.app);
        let first = open.iter().position(|c| c == "echo a@09:00").unwrap();
        let second = open.iter().position(|c| c == "echo a@09:01").unwrap();
        assert_eq!(second, first + 1, "{open:?}");
    }

    #[test]
    fn r_on_a_cleared_row_restores_it_and_the_cursor_stays_in_the_hidden_view() {
        let mut f = superseded_fixture();
        // Hide `echo c`, the newest open row.
        f.app.clear_cursor();
        assert_eq!(f.app.hidden_count, 2);
        f.app.toggle_hidden_view();
        f.app.restore_cursor();

        assert_eq!(f.app.hidden_count, 1);
        assert!(f.app.hidden_view());
        let cursor = f.app.cursor.expect("cursor stays on a hidden row");
        assert!(is_hidden_item(&f.app.rows[cursor]));
    }

    #[test]
    fn capital_r_restores_every_hidden_row() {
        let mut f = superseded_fixture();
        f.app.clear_cursor(); // hide `echo c`
        f.app.toggle_hidden_view();
        f.app.restore_all();

        assert_eq!(show_events(&f.app), 2);
        assert_eq!(f.app.hidden_count, 0);
        assert_eq!(f.app.open_count, 4);
        assert!(f.app.section_label("HIDDEN (0)").is_some());
        assert_eq!(f.app.cursor, None);
    }

    #[test]
    fn restoring_never_rewrites_the_logs() {
        let mut f = superseded_fixture();
        let log_before = std::fs::read(f._log.path()).unwrap();
        f.app.toggle_hidden_view();
        f.app.restore_all();
        assert_eq!(std::fs::read(f._log.path()).unwrap(), log_before);
        let events = std::fs::read_to_string(&f.app.cleared_path).unwrap();
        assert_eq!(events.lines().count(), 1);
    }

    #[test]
    fn clicking_a_hidden_rows_checkbox_restores_it_and_the_rest_of_the_row_does_nothing() {
        let mut f = superseded_fixture();
        f.app.toggle_hidden_view();
        let idx = f.app.cursor.unwrap();
        f.app.click_row(idx, CHECKBOX_END_COL + 2, 39);
        f.app.click_row(idx, 38, 39); // the reason column, not an x
        assert_eq!(f.app.hidden_count, 1);
        f.app.click_row(idx, 0, 39);
        assert_eq!(f.app.hidden_count, 0);
    }

    #[test]
    fn s_hides_only_the_cursors_session() {
        let mut f = superseded_fixture();
        // The cursor starts on `echo c` in s2. Marking it done leaves it in s2's DONE drawer.
        f.app.toggle_done_cursor();
        f.app.clear_session();

        assert_eq!(open_commands(&f.app), vec!["echo a@09:01".to_string()]);
        assert_eq!(
            f.app.done_count, 0,
            "the done row of s2 went with the session"
        );
        let hides = std::fs::read_to_string(&f.app.cleared_path).unwrap();
        assert_eq!(hides.lines().filter(|l| l.contains(" hide ")).count(), 2);
        assert!(hides.contains("|s2|echo b") && hides.contains("|s2|echo c"));
        assert!(!hides.contains("|s1|"));
    }

    #[test]
    fn s_does_nothing_on_a_hidden_row() {
        let mut f = superseded_fixture();
        f.app.toggle_hidden_view();
        f.app.clear_session();
        assert_eq!(f.app.hidden_count, 1);
        assert_eq!(f.app.open_count, 3);
    }

    #[test]
    fn hidden_rows_ignore_space_enter_and_backspace() {
        let mut f = superseded_fixture();
        f.app.toggle_hidden_view();
        f.app.toggle_done_cursor();
        f.app.activate_cursor();
        f.app.clear_cursor();
        assert_eq!(
            (f.app.open_count, f.app.done_count, f.app.hidden_count),
            (3, 0, 1)
        );
        assert!(f.app.status_text().is_none());
    }

    #[test]
    fn the_restore_button_restores_all_in_the_hidden_view() {
        let mut f = superseded_fixture();
        f.app.toggle_hidden_view();
        f.app.click_clear_all();
        assert_eq!(f.app.hidden_count, 0);
        assert!(f.app.hidden_view());
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
        cleared::hide(&f.app.cleared_path, &key);
        f.app.poll();
        assert!(f.app.rows.is_empty());
    }
}
