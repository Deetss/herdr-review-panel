use ratatui::style::{Color, Modifier, Style};

pub fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub fn item() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::UNDERLINED)
}

pub fn item_highlighted() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::REVERSED)
}

pub fn close_button() -> Style {
    Style::default().fg(Color::Yellow)
}

pub fn close_button_highlighted() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::REVERSED)
}

pub fn command() -> Style {
    Style::default().fg(Color::Magenta)
}

pub fn command_highlighted() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::REVERSED)
}

pub fn command_done() -> Style {
    Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::CROSSED_OUT)
}

pub fn checkbox() -> Style {
    Style::default().fg(Color::Magenta)
}

pub fn checkbox_highlighted() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::REVERSED)
}

pub fn status() -> Style {
    Style::default()
        .fg(Color::Gray)
        .add_modifier(Modifier::DIM | Modifier::ITALIC)
}

pub fn header() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn header_highlighted() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
}

/// The trailing per-row/per-section clear glyph - dim red everywhere, regardless of whether
/// the row itself is hover-highlighted, so it always reads as a distinct "danger" affordance.
pub fn clear_icon() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::DIM)
}

pub fn clear_all_button() -> Style {
    Style::default().fg(Color::Red)
}

pub fn clear_all_button_highlighted() -> Style {
    Style::default()
        .fg(Color::Red)
        .add_modifier(Modifier::REVERSED)
}
