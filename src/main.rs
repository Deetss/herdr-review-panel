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
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::time::Duration;

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

    let mut app = App::new(log_path, window_minutes);

    let mut terminal = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, &mut app);
    execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    let mut areas: Option<ui::Areas> = None;

    loop {
        let size = terminal.size()?;
        app.set_visible_height(size.height.saturating_sub(3) as usize);
        terminal.draw(|frame| areas = Some(ui::draw(frame, app)))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Up | KeyCode::Char('k') => app.move_cursor(-1),
                    KeyCode::Down | KeyCode::Char('j') => app.move_cursor(1),
                    KeyCode::Enter => app.activate_cursor(),
                    KeyCode::Char(' ') | KeyCode::Char('d') => app.toggle_done_cursor(),
                    KeyCode::Backspace | KeyCode::Delete => app.clear_cursor(),
                    KeyCode::Esc | KeyCode::Char('q') => app.click_close(),
                    _ => {}
                },
                Event::Mouse(mouse) => {
                    if let Some(areas) = &areas {
                        let (col, row) = (mouse.column, mouse.row);
                        match mouse.kind {
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
                                    app.cursor = Some(app.scroll + (row - areas.list.y) as usize);
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
        }

        app.poll();

        if app.should_quit {
            return Ok(());
        }
    }
}

fn within(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}
