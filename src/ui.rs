use crate::app::App;
use crate::log::Row;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

const CLEAR_ALL_LABEL: &str = "clear";
/// Content starts here on every item row, so paths and commands share one left edge and the
/// eye scans one column. Six is the checkbox's own width ("  [ ] "), see CHECKBOX_END_COL.
const FILE_INDENT: &str = "      ";

/// Screen regions the caller needs for mouse hit-testing - rendering owns layout, so it's the
/// one place that knows where things actually ended up.
pub struct Areas {
    pub close: Rect,
    pub clear_all: Rect,
    pub list: Rect,
}

pub fn draw(frame: &mut Frame, app: &App) -> Areas {
    let area = frame.area();
    // The hint shares the top row with the buttons: in a 45-column split every row of chrome
    // is a row of queue the user cannot see.
    let [close_area, divider_area, list_area] = Layout::vertical([
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

    // The hint doubles as the mode indicator: the detail view (or the report prompt) replaces
    // the list below it, and its own key line scrolls out of sight on a long command.
    let hint = if app.report_prompt_active() {
        "report? \u{b7} y: yes \u{b7} e: error \u{b7} Esc: not yet"
    } else if app.detail_text().is_some() {
        "full command \u{b7} Esc/Enter: back"
    } else {
        "click: open/copy \u{b7} box: done \u{b7} x: clear"
    };
    // Whatever is left of the row after the buttons and a two-column gap; a narrower pane
    // just loses the hint's tail rather than colliding with `clear`.
    let hint_width = clear_all_button.x.saturating_sub(2 + close_area.x);
    let hint_area = Rect {
        x: close_area.x,
        y: close_area.y,
        width: hint_width,
        height: 1,
    };
    // Drop whole segments rather than cutting mid-word: "box: done · x:" teaches nothing.
    let hint = fit_hint(hint, hint_width as usize);
    frame.render_widget(Paragraph::new(Span::styled(hint, theme::dim())), hint_area);

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
    if app.rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("Nothing to review.", theme::dim())),
            list_area,
        );
    } else {
        frame.render_widget(Paragraph::new(lines), list_area);
    }

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

    if app.report_prompt_active() {
        render_report_prompt(frame, list_area);
    } else if let Some(command) = app.detail_text() {
        render_detail(frame, list_area, command);
    }

    Areas {
        close: close_button,
        clear_all: clear_all_button,
        list: list_area,
    }
}

/// Full-width, wrapped view of one command. The panel's normal rows are single-line and
/// truncate, and a phone viewing this remotely gets no clipboard and no horizontal
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

/// Shown once every command in the queue is checked off, before anything gets reported back
/// to the agent - checking a box only means it was run, not that it worked, so this is where
/// the human says which.
fn render_report_prompt(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(Span::styled("Finished the review queue.", theme::header())),
        Line::default(),
        Line::from("Report to the agent?"),
        Line::default(),
        Line::from(Span::styled("y", theme::header())),
        Line::from("  yes, it all worked - continue"),
        Line::from(Span::styled("e", theme::header())),
        Line::from("  no, something went wrong - flag it instead"),
        Line::from(Span::styled("Esc", theme::header())),
        Line::from("  not yet - leave the queue as is"),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// Which end of a row's content to drop when it does not fit: a path keeps its tail (the
/// filename is what identifies it), a command keeps its head (the verb is).
#[derive(Clone, Copy)]
enum Trim {
    Head,
    Tail,
}

/// Appends right-padding plus a trailing "x" to reach `width` - the per-row/per-section clear
/// affordance app.rs's click_row treats any click on the rightmost column as. Only called for
/// row kinds that are actually clearable (not Blank).
///
/// The last span is the row's content and is cut to fit first: ratatui clips a too-long line
/// at the right edge, which used to take the "x" with it, leaving the rightmost click meaning
/// "clear" on a row that showed no clear glyph. Narrow splits (Orca's is half a tab) hit this
/// on almost every absolute path.
fn with_clear_glyph(mut spans: Vec<Span<'static>>, width: u16, trim: Trim) -> Vec<Span<'static>> {
    let last_col = (width as usize).saturating_sub(1);
    // One column of gap before the x, so a full row still reads as content-then-glyph.
    let budget = last_col.saturating_sub(1);
    let prefix_len: usize = spans
        .iter()
        .take(spans.len().saturating_sub(1))
        .map(|s| s.content.chars().count())
        .sum();
    if let Some(last) = spans.last_mut() {
        let room = budget.saturating_sub(prefix_len);
        if last.content.chars().count() > room {
            let fitted = fit(&last.content, room, trim);
            *last = Span::styled(fitted, last.style);
        }
    }
    let content_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if last_col > content_len {
        spans.push(Span::raw(" ".repeat(last_col - content_len)));
    }
    spans.push(Span::styled("x", theme::clear_icon()));
    spans
}

/// Keeps as many " \u{b7} "-separated segments of `hint` as fit in `room` columns.
fn fit_hint(hint: &str, room: usize) -> String {
    let mut out = String::new();
    for seg in hint.split(" \u{b7} ") {
        let candidate = if out.is_empty() {
            seg.to_string()
        } else {
            format!("{out} \u{b7} {seg}")
        };
        if candidate.chars().count() > room {
            break;
        }
        out = candidate;
    }
    out
}

fn fit(text: &str, room: usize, trim: Trim) -> String {
    let len = text.chars().count();
    if len <= room {
        return text.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let keep = room - 1;
    match trim {
        Trim::Tail => {
            let mut head: String = text.chars().take(keep).collect();
            head.push('\u{2026}');
            head
        }
        Trim::Head => {
            let tail: String = text.chars().skip(len - keep).collect();
            format!("\u{2026}{tail}")
        }
    }
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
            // Empty ts is the sentinel for the synthetic trailing "Completed" section (see
            // app.rs's sweep_completed) - it has no timestamp of its own, so skip the span
            // and its gap rather than rendering a blank-then-double-space.
            let mut spans = Vec::new();
            if !ts.is_empty() {
                // The list is a window of the last few minutes, so the date is always today
                // and only the clock part tells groups apart.
                let clock = ts.get(11..16).unwrap_or(ts.as_str()).to_string();
                spans.push(Span::styled(clock, theme::dim()));
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(repo.clone(), repo_style));
            Line::from(with_clear_glyph(spans, width, Trim::Tail))
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
            let mut spans = vec![Span::raw(FILE_INDENT)];
            if warn.is_some() {
                spans.push(Span::styled("\u{26a0} ", theme::warn_icon()));
            }
            spans.push(Span::styled(text, style));
            Line::from(with_clear_glyph(spans, width, Trim::Head))
        }
        Row::CommandItem {
            command,
            step,
            warn,
            terminal,
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
            if let Some(terminal) = terminal {
                spans.push(Span::styled(format!("[{terminal}] "), theme::dim()));
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
            Line::from(with_clear_glyph(spans, width, Trim::Tail))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn short_row_pads_out_to_the_glyph() {
        let spans = with_clear_glyph(vec![Span::raw("ab")], 6, Trim::Tail);
        assert_eq!(text(&spans), "ab   x");
    }

    #[test]
    fn long_path_keeps_its_tail_and_the_glyph() {
        let spans = with_clear_glyph(
            vec![Span::raw("  "), Span::raw("/a/b/c/file.md")],
            10,
            Trim::Head,
        );
        assert_eq!(text(&spans), "  \u{2026}le.md x");
    }

    #[test]
    fn long_command_keeps_its_head_and_the_glyph() {
        let spans = with_clear_glyph(
            vec![Span::raw("[ ] "), Span::raw("echo hello world")],
            12,
            Trim::Tail,
        );
        assert_eq!(text(&spans), "[ ] echo \u{2026} x");
    }

    #[test]
    fn hint_drops_whole_segments() {
        let hint = "click: open/copy \u{b7} box: done \u{b7} x: clear";
        assert_eq!(fit_hint(hint, 100), hint);
        assert_eq!(fit_hint(hint, 30), "click: open/copy \u{b7} box: done");
        assert_eq!(fit_hint(hint, 10), "");
    }

    #[test]
    fn exact_fit_is_left_alone() {
        let spans = with_clear_glyph(vec![Span::raw("abcd")], 6, Trim::Tail);
        assert_eq!(text(&spans), "abcd x");
    }
}
