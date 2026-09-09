use ratatui::style::{Color, Modifier, Style};

pub fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub fn cyan_bold() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
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
