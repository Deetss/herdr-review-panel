use crate::app::App;
use crate::log::Row;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Screen regions the caller needs for mouse hit-testing - rendering owns layout, so it's the
/// one place that knows where things actually ended up.
pub struct Areas {
    pub close: Rect,
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

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Click, or \u{2191}/\u{2193} + Enter, to open a file link.",
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
        .map(|(idx, row)| render_row(row, Some(idx) == app.cursor))
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);

    Areas {
        close: close_button,
        list: list_area,
    }
}

fn render_row(row: &Row, highlighted: bool) -> Line<'static> {
    match row {
        Row::Blank => Line::default(),
        Row::GroupHeader { ts, repo } => Line::from(vec![
            Span::styled(ts.clone(), theme::dim()),
            Span::raw("  "),
            Span::styled(repo.clone(), theme::cyan_bold()),
        ]),
        Row::Item { label, abspath } => {
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
            Line::from(vec![Span::raw("  "), Span::styled(text, style)])
        }
    }
}
