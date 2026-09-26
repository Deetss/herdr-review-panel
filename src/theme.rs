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

/// The checkbox is a control, not content: gray keeps it from competing with the command
/// text beside it, and "[ ]" vs "[x]" carries the done state on its own.
pub fn checkbox() -> Style {
    Style::default().fg(Color::Gray)
}

pub fn checkbox_highlighted() -> Style {
    Style::default()
        .fg(Color::Gray)
        .add_modifier(Modifier::REVERSED)
}

/// The toast overlays the last list row, so it must read as something other than a row:
/// bold cyan is the header family, never used on an item. No ITALIC, which several
/// terminals drop, and no DIM, which made it vanish on dark backgrounds.
pub fn status() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
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

/// Leading marker on a command review-notify.sh's prose heuristic flagged - always bold and
/// regardless of hover state, same reasoning as `clear_icon`. Light red rather than yellow
/// so it is not the same color as the file rows it sits beside.
pub fn warn_icon() -> Style {
    Style::default()
        .fg(Color::LightRed)
        .add_modifier(Modifier::BOLD)
}

pub fn clear_all_button() -> Style {
    Style::default().fg(Color::Red)
}

pub fn clear_all_button_highlighted() -> Style {
    Style::default()
        .fg(Color::Red)
        .add_modifier(Modifier::REVERSED)
}
