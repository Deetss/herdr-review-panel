mod actions;
mod app;
mod cleared;
mod done;
mod log;
mod theme;
mod ui;

use anyhow::Result;
use app::{App, WHEEL_SCROLL_LINES};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::SetTitle;
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How often the log, done and cleared files are re-read. Input events redraw immediately
/// but must not each trigger file reads, or a mouse sweep across the list does hundreds.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// How long to wait for a held mouse button to come back up before giving up on it.
const RELEASE_GRACE: Duration = Duration::from_secs(1);

fn main() -> Result<()> {
    let log_path = std::env::var("REVIEW_PANEL_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude/review.log")
        });
    let window_minutes: i64 = std::env::var("REVIEW_PANEL_WINDOW_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let notify_target = std::env::var("REVIEW_PANEL_NOTIFY_TARGET").ok();

    let mut app = App::new(log_path, window_minutes, notify_target);

    let mut terminal = ratatui::init();
    // The pane title is how plugin.sh finds an open panel; without this the pane keeps the
    // shell's own title.
    execute!(
        std::io::stdout(),
        EnableMouseCapture,
        SetTitle("Review Queue")
    )?;
    let result = run(&mut terminal, &mut app);
    execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    let mut areas: Option<ui::Areas> = None;

    let mut button_held = false;
    let mut last_poll = Instant::now();
    loop {
        let size = terminal.size()?;
        app.set_visible_height(size.height.saturating_sub(3) as usize);
        terminal.draw(|frame| areas = Some(ui::draw(frame, app)))?;

        let mut force_poll = false;
        if event::poll(Duration::from_millis(200))? {
            // Drain everything already queued before redrawing, so a burst of mouse moves
            // costs one frame rather than one per event.
            loop {
                let ev = event::read()?;
                if !matches!(&ev, Event::Mouse(m) if m.kind == MouseEventKind::Moved) {
                    force_poll = true;
                }
                match ev {
                    // The finished-queue prompt swallows every key but its own three, so a stray
                    // Space/Backspace can't act on a row underneath while it's up.
                    Event::Key(key) if app.report_prompt_active() => match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            app.confirm_report_success();
                        }
                        KeyCode::Char('e') | KeyCode::Char('E') => app.confirm_report_error(),
                        KeyCode::Esc => app.dismiss_report_prompt(),
                        _ => {}
                    },
                    // The detail overlay swallows the key that dismisses it, so Esc does not also
                    // close the whole panel on the way out.
                    Event::Key(key) if app.detail_text().is_some() => match key.code {
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                            app.dismiss_detail();
                        }
                        _ => {}
                    },
                    Event::Key(key) => match key.code {
                        KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
                        KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
                        KeyCode::Enter => app.activate_cursor(),
                        KeyCode::Char(' ') | KeyCode::Char('d') => app.toggle_done_cursor(),
                        KeyCode::Backspace | KeyCode::Delete => app.clear_cursor(),
                        // Shifted, because clearing everything from a stray keypress is not
                        // recoverable. The mouse route has a dedicated button; this is its only
                        // keyboard equivalent, which keyboard-only remote clients need.
                        KeyCode::Char('C') => app.click_clear_all(),
                        KeyCode::Esc | KeyCode::Char('q') => app.click_close(),
                        _ => {}
                    },
                    Event::Mouse(mouse) => {
                        match mouse.kind {
                            MouseEventKind::Down(_) => button_held = true,
                            MouseEventKind::Up(_) => button_held = false,
                            _ => {}
                        }
                        if let Some(areas) = &areas {
                            let (col, row) = (mouse.column, mouse.row);
                            match mouse.kind {
                                // The finished-queue prompt only answers its own three options, so
                                // a stray click can't reach a row underneath.
                                MouseEventKind::Down(MouseButton::Left)
                                    if app.report_prompt_active() =>
                                {
                                    if let Some([yes, error, later]) = areas.report {
                                        if within(yes, col, row) {
                                            app.confirm_report_success();
                                        } else if within(error, col, row) {
                                            app.confirm_report_error();
                                        } else if within(later, col, row) {
                                            app.dismiss_report_prompt();
                                        }
                                    }
                                }
                                MouseEventKind::Down(MouseButton::Left) => {
                                    if within(areas.close, col, row) {
                                        app.click_close();
                                    } else if within(areas.clear_all, col, row) {
                                        app.click_clear_all();
                                    } else if within(areas.list, col, row) {
                                        let idx = app.scroll + (row - areas.list.y) as usize;
                                        app.click_row(idx, col - areas.list.x, areas.list.width);
                                    }
                                }
                                MouseEventKind::Moved => {
                                    app.close_hovered = within(areas.close, col, row);
                                    app.clear_all_hovered = within(areas.clear_all, col, row);
                                    if within(areas.list, col, row) {
                                        app.cursor =
                                            Some(app.scroll + (row - areas.list.y) as usize);
                                    }
                                }
                                MouseEventKind::ScrollUp => app.scroll_by(-WHEEL_SCROLL_LINES),
                                MouseEventKind::ScrollDown => app.scroll_by(WHEEL_SCROLL_LINES),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                if app.should_quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }

        if force_poll || last_poll.elapsed() >= POLL_INTERVAL {
            app.poll();
            last_poll = Instant::now();
        }

        if app.should_quit {
            swallow_release(button_held);
            return Ok(());
        }
    }
}

/// A click that quits the panel is still held when mouse reporting is turned off, so its
/// release would reach the shell as typed text ("^[[<0;93;2m"). Reading it here keeps it out.
fn swallow_release(mut held: bool) {
    let deadline = Instant::now() + RELEASE_GRACE;
    while held && Instant::now() < deadline {
        if event::poll(Duration::from_millis(50)).unwrap_or(false)
            && let Ok(Event::Mouse(mouse)) = event::read()
            && matches!(mouse.kind, MouseEventKind::Up(_))
        {
            held = false;
        }
    }
}

fn within(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}
