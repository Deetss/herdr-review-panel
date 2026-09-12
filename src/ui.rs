use crate::app::App;
use crate::log::Row;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

const CLEAR_ALL_LABEL: &str = "clear";

/// Screen regions the caller needs for mouse hit-testing - rendering owns layout, so it's the
/// one place that knows where things actually ended up.
pub struct Areas {
    pub close: Rect,
    pub clear_all: Rect,
    pub list: Rect,
}

pub fn draw(frame: &mut Frame, app: &App) -> Areas {
    let area = frame.area();
    let [close_area, hint_area, divider_area, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);

    let close_style = if app.close_hovered {
        theme::close_button_highlighted()
    } else {
        theme::close_button()
    };
    let close_button = Rect {
        x: close_area.right().saturating_sub(1),
        y: close_area.y,
        width: 1,
        height: 1,
    };
    frame.render_widget(Paragraph::new(Span::styled("x", close_style)), close_button);

    let clear_all_style = if app.clear_all_hovered {
        theme::clear_all_button_highlighted()
    } else {
        theme::clear_all_button()
    };
    let clear_all_width = CLEAR_ALL_LABEL.len() as u16;
    let clear_all_button = Rect {
        x: close_button.x.saturating_sub(2 + clear_all_width),
        y: close_area.y,
        width: clear_all_width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Span::styled(CLEAR_ALL_LABEL, clear_all_style)),
        clear_all_button,
    );

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Click a file/command, its box to check off, or the x on any row/section to clear it.",
            theme::dim(),
        )),
        hint_area,
    );

    let divider = "\u{2500}".repeat(divider_area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(divider, theme::dim())),
        divider_area,
    );

    let visible = list_area.height as usize;
    let lines: Vec<Line> = app
        .rows
        .iter()
        .enumerate()
        .skip(app.scroll)
        .take(visible)
        .map(|(idx, row)| render_row(row, Some(idx) == app.cursor, app, list_area.width))
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);

    // Transient click feedback ("Opened x", "Copied to clipboard") overlays the bottom-right
    // corner rather than living in the static hint row, so it reads as a toast confirming the
    // click rather than a change to the panel's standing instructions.
    if let Some(status) = app.status_text() {
        let width = (status.chars().count() as u16).min(area.width);
        let status_rect = Rect {
            x: area.width.saturating_sub(width),
            y: area.height.saturating_sub(1),
            width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Span::styled(status.to_string(), theme::status())),
            status_rect,
        );
    }

    if let Some(command) = &app.detail {
        render_detail(frame, list_area, command);
    }

    Areas {
        close: close_button,
        clear_all: clear_all_button,
        list: list_area,
    }
}

/// Full-width, wrapped view of one command. The panel's normal rows are single-line and
/// truncate, and a phone viewing this through Collie gets no clipboard and no horizontal
/// scroll, so a long command is otherwise unreadable there. Real newlines are restored -
/// unlike the list rows, which flatten them to a marker to stay one line tall.
fn render_detail(frame: &mut Frame, area: Rect, command: &str) {
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled("command", theme::header())),
        Line::default(),
    ];
    lines.extend(command.lines().map(|l| Line::from(l.to_string())));
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(
        "Esc / Enter to dismiss",
        theme::dim(),
    )));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// Appends right-padding plus a trailing "x" to reach `width` - the per-row/per-section clear
/// affordance app.rs's click_row treats any click on the rightmost column as. Only called for
/// row kinds that are actually clearable (not Blank).
fn with_clear_glyph(mut spans: Vec<Span<'static>>, width: u16) -> Vec<Span<'static>> {
    let content_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let last_col = (width as usize).saturating_sub(1);
    if last_col > content_len {
        spans.push(Span::raw(" ".repeat(last_col - content_len)));
    }
    spans.push(Span::styled("x", theme::clear_icon()));
    spans
}

fn render_row(row: &Row, highlighted: bool, app: &App, width: u16) -> Line<'static> {
    match row {
        Row::Blank => Line::default(),
        Row::GroupHeader { ts, repo } => {
            let repo_style = if highlighted {
                theme::header_highlighted()
            } else {
                theme::header()
            };
            let spans = vec![
                Span::styled(ts.clone(), theme::dim()),
                Span::raw("  "),
                Span::styled(repo.clone(), repo_style),
            ];
            Line::from(with_clear_glyph(spans, width))
        }
        Row::FileItem {
            label,
            abspath,
            warn,
            ..
        } => {
            let text = abspath.clone().unwrap_or_else(|| label.clone());
            let style = if abspath.is_some() {
                if highlighted {
                    theme::item_highlighted()
                } else {
                    theme::item()
                }
            } else {
                theme::close_button() // plain yellow, no underline - not a real link
            };
            let mut spans = vec![Span::raw("  ")];
            if warn.is_some() {
                spans.push(Span::styled("\u{26a0} ", theme::warn_icon()));
            }
            spans.push(Span::styled(text, style));
            Line::from(with_clear_glyph(spans, width))
        }
        Row::CommandItem {
            command,
            step,
            warn,
            key,
        } => {
            let done = app.is_done(key);
            let checkbox = if done { "[x] " } else { "[ ] " };
            let checkbox_style = if highlighted {
                theme::checkbox_highlighted()
            } else {
                theme::checkbox()
            };
            let text_style = if done {
                theme::command_done()
            } else if highlighted {
                theme::command_highlighted()
            } else {
                theme::command()
            };
            let mut spans = vec![Span::raw("  "), Span::styled(checkbox, checkbox_style)];
            if warn.is_some() {
                spans.push(Span::styled("\u{26a0} ", theme::warn_icon()));
            }
            if let Some(step) = step {
                spans.push(Span::styled(format!("{step}. "), theme::dim()));
            }
            // A ratatui Line cannot contain a newline, so a multiline command is shown on
            // one row with visible break markers. Copying still yields the real text -
            // Activation::CopyCommand carries the undisplayed command.
            spans.push(Span::styled(
                command.replace('\n', " \u{23ce} "),
                text_style,
            ));
            Line::from(with_clear_glyph(spans, width))
        }
    }
}
