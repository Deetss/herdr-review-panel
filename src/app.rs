use crate::actions;
use crate::log::{self, Row};
use chrono::Local;
use std::path::PathBuf;
use std::time::Duration;

pub struct App {
    pub rows: Vec<Row>,
    /// Index into `rows` of the currently highlighted item - set by keyboard navigation or
    /// mouse hover, whichever moved it most recently. Always points at a Row::Item when Some.
    pub cursor: Option<usize>,
    /// Index of the first row currently visible in the list area.
    pub scroll: usize,
    pub close_hovered: bool,
    pub should_quit: bool,

    log_path: PathBuf,
    offset: u64,
    home: String,
    last_key: Option<(String, String)>,
}

impl App {
    pub fn new(log_path: PathBuf, window_minutes: i64) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let cutoff = (Local::now() - Duration::from_secs((window_minutes.max(0) as u64) * 60))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let mut rows = Vec::new();
        let mut last_key = None;
        let offset = log::backfill(&log_path, &cutoff, &mut rows, &mut last_key, &home);
        let mut app = App {
            rows,
            cursor: None,
            scroll: 0,
            close_hovered: false,
            should_quit: false,
            log_path,
            offset,
            home,
            last_key,
        };
        app.jump_to_latest();
        app
    }

    /// Polls the log for newly-appended lines. Returns true if anything changed (so the caller
    /// knows to redraw immediately rather than waiting for the next tick).
    pub fn poll(&mut self) -> bool {
        let before = self.rows.len();
        self.offset = log::poll_tail(
            &self.log_path,
            self.offset,
            &mut self.rows,
            &mut self.last_key,
            &self.home,
        );
        if self.rows.len() != before {
            self.jump_to_latest();
            true
        } else {
            false
        }
    }

    fn item_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| matches!(r, Row::Item { .. }).then_some(i))
    }

    fn jump_to_latest(&mut self) {
        self.cursor = self.item_rows().last();
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let items: Vec<usize> = self.item_rows().collect();
        if items.is_empty() {
            return;
        }
        let current = self
            .cursor
            .and_then(|c| items.iter().position(|&i| i == c))
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, items.len() as isize - 1) as usize;
        self.cursor = Some(items[next]);
    }

    pub fn ensure_cursor_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
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

    pub fn activate_cursor(&mut self) {
        if let Some(Row::Item {
            abspath: Some(path),
            ..
        }) = self.cursor.and_then(|c| self.rows.get(c))
        {
            actions::open_item(path);
        }
    }

    pub fn click_row(&mut self, row_idx: usize) {
        if let Some(Row::Item {
            abspath: Some(path),
            ..
        }) = self.rows.get(row_idx)
        {
            actions::open_item(path);
        }
    }

    pub fn click_close(&mut self) {
        actions::close();
        self.should_quit = true;
    }
}
