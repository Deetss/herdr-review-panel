use crate::app::App;
use crate::log::{Hidden, Row};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

const CLEAR_ALL_LABEL: &str = "clear";
const RESTORE_LABEL: &str = "restore";
/// Rows reserved at the bottom for key hints.
const FOOTER_ROWS: u16 = 2;
/// Top inset, header and divider above the body.
const CHROME_ROWS: u16 = 3;

/// Rows the body gets in a terminal `total_width` by `total_height` cells.
pub fn body_height(app: &App, total_width: u16, total_height: u16) -> usize {
    let spacer = u16::from(footer_needs_spacer(app, total_width.saturating_sub(1)));
    total_height.saturating_sub(CHROME_ROWS + FOOTER_ROWS + spacer) as usize
}

/// The hidden view's footer can fill both footer rows, which would leave its last HIDDEN row
/// touching the first footer row. One body row is given up to keep a blank between them.
fn footer_needs_spacer(app: &App, drawable_width: u16) -> bool {
    app.hidden_view() && !footer_rows(&footer_segments(app), drawable_width as usize)[1].is_empty()
}

/// The header counts, shortened to fit `room` columns: the "Queue: " prefix goes first, then
/// the done count, then the text is cut with an ellipsis.
fn header_counts(open: usize, done: usize, room: usize) -> String {
    let candidates = [
        format!("Queue: {open} open \u{b7} {done} done"),
        format!("{open} open \u{b7} {done} done"),
        format!("{open} open"),
    ];
    match candidates.iter().find(|c| c.chars().count() <= room) {
        Some(fit) => fit.clone(),
        None => fit(&candidates[2], room, Trim::Tail),
    }
}

/// What Enter does on the DONE label's row, shown while the cursor is in the drawer.
fn done_hint(app: &App) -> Option<&'static str> {
    match app.cursor.and_then(|c| app.rows.get(c))? {
        Row::Summary {
            expanded: false, ..
        } => Some("Enter: open"),
        Row::Summary { .. } => Some("Enter: fold"),
        Row::CommandItem {
            key, hidden: None, ..
        } if app.is_done(key) => Some("Enter: fold"),
        _ => None,
    }
}

/// Key hints for whatever the cursor is on, in priority order.
fn footer_segments(app: &App) -> Vec<&'static str> {
    const TAIL: [&str; 4] = ["Bksp clear", "c clear done", "H hidden", "S clear session"];
    if app.hidden_view() {
        return vec!["r restore", "R restore all", "H back"];
    }
    if app.rows.is_empty() {
        return vec!["H hidden", "q close"];
    }
    let lead: Option<&'static str> = match app.cursor.and_then(|c| app.rows.get(c)) {
        Some(Row::CommandItem { key, .. }) if app.is_done(key) => Some("Space reopen"),
        Some(Row::CommandItem { .. }) => Some("Space done"),
        Some(Row::FileItem { .. }) => None,
        Some(Row::Summary {
            expanded: false, ..
        }) => Some("Enter open"),
        Some(Row::Summary { .. }) => Some("Enter fold"),
        _ => None,
    };
    let enter: Option<&'static str> = match app.cursor.and_then(|c| app.rows.get(c)) {
        Some(Row::CommandItem { key, .. }) if !app.is_done(key) => Some("Enter copy"),
        Some(Row::FileItem { .. }) => Some("Enter open"),
        _ => None,
    };
    lead.into_iter().chain(TAIL).chain(enter).collect()
}

/// Fills two rows greedily with ` · ` joined segments. A segment that fits on neither row is
/// dropped whole, along with every one after it.
fn footer_rows(segments: &[&str], width: usize) -> [String; 2] {
    let mut rows = [String::new(), String::new()];
    let mut r = 0;
    for seg in segments {
        loop {
            let candidate = if rows[r].is_empty() {
                seg.to_string()
            } else {
                format!("{} \u{b7} {seg}", rows[r])
            };
            if candidate.chars().count() <= width {
                rows[r] = candidate;
                break;
            }
            if r == 0 && !rows[0].is_empty() {
                r = 1;
                continue;
            }
            return rows;
        }
    }
    rows
}
/// Content starts here on every item row, so paths and commands share one left edge and the
/// eye scans one column. Six is the checkbox's own width ("  [ ] "), see CHECKBOX_END_COL.
const FILE_INDENT: &str = "      ";

/// Screen regions the caller needs for mouse hit-testing - rendering owns layout, so it's the
/// one place that knows where things actually ended up.
pub struct Areas {
    pub close: Rect,
    pub clear_all: Rect,
    pub list: Rect,
    /// The prompt's y / e / Esc options, in that order, while the finish prompt is up.
    pub report: Option<[Rect; 3]>,
}

pub fn draw(frame: &mut Frame, app: &App) -> Areas {
    let raw_area = frame.area();
    // Orca overlays its own close/action buttons in the pane's top-right corner;
    // inset so our close/clear controls don't sit under them.
    let area = Rect {
        x: raw_area.x,
        y: raw_area.y + 1,
        width: raw_area.width.saturating_sub(1),
        height: raw_area.height.saturating_sub(1),
    };
    let [close_area, divider_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(FOOTER_ROWS),
    ])
    .areas(area);
    let list_area = if footer_needs_spacer(app, area.width) {
        Rect {
            height: list_area.height.saturating_sub(1),
            ..list_area
        }
    } else {
        list_area
    };

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
    // In the hidden view the same slot holds `restore`, which restores every hidden row.
    let (button_label, has_rows) = if app.hidden_view() {
        (RESTORE_LABEL, app.hidden_count > 0)
    } else {
        (CLEAR_ALL_LABEL, app.open_count + app.done_count > 0)
    };
    let clear_all_width = button_label.len() as u16;
    let clear_slot = Rect {
        x: close_button.x.saturating_sub(2 + clear_all_width),
        y: close_area.y,
        width: clear_all_width,
        height: 1,
    };
    // The slot stays reserved when the button is absent, so the counts read the same in the
    // empty state as in every other one.
    let clear_all_button = Rect {
        width: if has_rows { clear_all_width } else { 0 },
        ..clear_slot
    };
    if has_rows {
        frame.render_widget(
            Paragraph::new(Span::styled(button_label, clear_all_style)),
            clear_all_button,
        );
    }

    // The detail view and the report prompt replace the list below, so the header row names
    // the mode; otherwise it carries the counts.
    let left_width = clear_slot.x.saturating_sub(2 + close_area.x);
    let left_area = Rect {
        x: close_area.x,
        y: close_area.y,
        width: left_width,
        height: 1,
    };
    let mode_hint = if app.report_prompt_active() {
        Some("report? \u{b7} y: yes \u{b7} e: error \u{b7} Esc: not yet")
    } else if app.detail_text().is_some() {
        Some(match app.detail_title() {
            "path" => "full path (copied) \u{b7} Esc/Enter: back",
            _ => "full command \u{b7} Esc/Enter: back",
        })
    } else {
        None
    };
    let left = match mode_hint {
        // Drop whole segments rather than cutting mid-word.
        Some(hint) => Span::styled(fit_hint(hint, left_width as usize), theme::dim()),
        None => Span::styled(
            header_counts(app.open_count, app.done_count, left_width as usize),
            theme::header(),
        ),
    };
    frame.render_widget(Paragraph::new(left), left_area);

    let divider = "\u{2500}".repeat(divider_area.width as usize);
    frame.render_widget(
        Paragraph::new(Span::styled(divider, theme::dim())),
        divider_area,
    );

    let visible = list_area.height as usize;
    let done_hint = done_hint(app);
    let lines: Vec<Line> = app
        .rows
        .iter()
        .enumerate()
        .skip(app.scroll)
        .take(visible)
        .map(|(idx, row)| {
            render_row(
                row,
                Some(idx) == app.cursor,
                app,
                list_area.width,
                done_hint,
            )
        })
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

    if mode_hint.is_none() {
        let [first, second] = footer_rows(&footer_segments(app), area.width as usize);
        // A one-row footer sits on the last row, leaving the row above blank.
        let (first, second) = if second.is_empty() {
            (String::new(), first)
        } else {
            (first, second)
        };
        for (offset, text) in [first, second].into_iter().enumerate() {
            let rect = Rect {
                x: footer_area.x,
                y: footer_area.y + offset as u16,
                width: footer_area.width,
                height: 1,
            }
            .intersection(footer_area);
            frame.render_widget(Paragraph::new(Span::styled(text, theme::dim())), rect);
        }
    }

    let mut report = None;
    if app.report_prompt_active() {
        report = Some(render_report_prompt(frame, list_area));
    } else if let Some(command) = app.detail_text() {
        render_detail(frame, list_area, app.detail_title(), command);
    }

    Areas {
        close: close_button,
        clear_all: clear_all_button,
        list: list_area,
        report,
    }
}

/// Full-width, wrapped view of one command. The panel's normal rows are single-line and
/// truncate, and a phone viewing this remotely gets no clipboard and no horizontal
/// scroll, so a long command is otherwise unreadable there. Real newlines are restored -
/// unlike the list rows, which flatten them to a marker to stay one line tall.
fn render_detail(frame: &mut Frame, area: Rect, title: &str, command: &str) {
    frame.render_widget(Clear, area);
    let mut lines = vec![
        Line::from(Span::styled(title.to_string(), theme::header())),
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
fn render_report_prompt(frame: &mut Frame, area: Rect) -> [Rect; 3] {
    frame.render_widget(Clear, area);
    const OPTIONS: [(&str, &str); 3] = [
        ("y", "  yes, it all worked - continue"),
        ("e", "  no, something went wrong - flag it instead"),
        ("Esc", "  not yet - leave the queue as is"),
    ];
    let mut lines = vec![
        Line::from(Span::styled("Finished the review queue.", theme::header())),
        Line::default(),
        Line::from("Report to the agent?"),
        Line::default(),
    ];
    let mut rects = [Rect::default(); 3];
    let mut y = area.y + lines.len() as u16;
    for (rect, (key, text)) in rects.iter_mut().zip(OPTIONS) {
        // A description that wraps on a narrow split pushes every later option down with it.
        let height = 1
            + (text.chars().count() as u16)
                .div_ceil(area.width.max(1))
                .max(1);
        *rect = Rect {
            x: area.x,
            y,
            width: area.width,
            height,
        }
        .intersection(area);
        y += height;
        lines.push(Line::from(Span::styled(key, theme::header())));
        lines.push(Line::from(text));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    rects
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

/// The row's HH:MM, from a "YYYY-MM-DD HH:MM:SS" timestamp.
fn clock(ts: &str) -> String {
    ts.get(11..16).unwrap_or(ts).to_string()
}

/// Columns before the trailing gap and `x`.
fn content_budget(width: u16) -> usize {
    (width as usize).saturating_sub(2)
}

/// A HIDDEN row: checkbox as it was, the text, and the reason right aligned to the last
/// column. The reason is never cut; the text gives way to it and keeps one gap column.
fn render_hidden_item(
    text: &str,
    trim: Trim,
    checked: bool,
    reason: Hidden,
    highlighted: bool,
    width: u16,
) -> Line<'static> {
    let tag = match reason {
        Hidden::Cleared => "cleared",
        Hidden::Superseded => "superseded",
    };
    let (box_style, text_style) = if highlighted {
        (
            theme::checkbox_highlighted(),
            theme::hidden_command_highlighted(),
        )
    } else {
        (theme::checkbox(), theme::hidden_command())
    };
    let last = (width as usize).saturating_sub(1);
    let room = last.saturating_sub(tag.len() + 6);
    let text = fit(text, room, trim);
    let used = 6 + text.chars().count() + tag.len();
    Line::from(vec![
        Span::raw("  "),
        Span::styled(if checked { "[x] " } else { "[ ] " }, box_style),
        Span::styled(text, text_style),
        Span::raw(" ".repeat((width as usize).saturating_sub(used))),
        Span::styled(tag, theme::dim()),
    ])
}

fn render_row(
    row: &Row,
    highlighted: bool,
    app: &App,
    width: u16,
    done_hint: Option<&'static str>,
) -> Line<'static> {
    match row {
        Row::Blank => Line::default(),
        Row::Section { label } => {
            let mut spans = vec![Span::styled(label.clone(), theme::section())];
            if let (true, Some(hint)) = (label.starts_with("DONE"), done_hint) {
                let used = label.chars().count() + hint.chars().count();
                let gap = (width as usize).saturating_sub(used);
                spans.push(Span::raw(" ".repeat(gap)));
                spans.push(Span::styled(hint, theme::dim()));
            }
            Line::from(spans)
        }
        Row::Note(text) => Line::from(Span::styled(text.clone(), theme::dim())),
        Row::GroupHeader { ts, repo, .. } => {
            let repo_style = if highlighted {
                theme::header_highlighted()
            } else {
                theme::header()
            };
            // Two sessions in one repo share the repo text, so the clock tells them apart.
            // The clock is never cut; the repo gives way first, down to three columns.
            let clock = clock(ts);
            let repo_room = content_budget(width)
                .saturating_sub(1 + clock.chars().count())
                .max(3);
            let spans = vec![
                Span::styled(fit(repo, repo_room, Trim::Tail), repo_style),
                Span::raw(" "),
                Span::styled(clock, theme::dim()),
            ];
            Line::from(with_clear_glyph(spans, width, Trim::Tail))
        }
        Row::Summary { repo, ts, keys, .. } => {
            let count = keys.len();
            let rows = format!("{count} row{}", if count == 1 { "" } else { "s" });
            let with_time = format!("{rows}, {}", clock(ts));
            // 2 for the gap after the repo, 4 for "[x] ".
            let budget = content_budget(width).saturating_sub(6);
            let (repo_room, tail) = if repo.chars().count() + with_time.chars().count() <= budget {
                (repo.chars().count(), with_time)
            } else if budget >= with_time.chars().count() + 3 {
                (budget - with_time.chars().count(), with_time)
            } else {
                (budget.saturating_sub(rows.chars().count()).max(3), rows)
            };
            let (text, box_style) = if highlighted {
                (theme::summary_highlighted(), theme::summary_highlighted())
            } else {
                (theme::dim(), theme::checkbox())
            };
            let spans = vec![
                Span::styled(format!("{}  ", fit(repo, repo_room, Trim::Tail)), text),
                Span::styled("[x] ", box_style),
                Span::styled(tail, text),
            ];
            Line::from(with_clear_glyph(spans, width, Trim::Tail))
        }
        Row::HiddenHeader { ts, repo, .. } => {
            let clock = clock(ts);
            let repo_room = (width as usize)
                .saturating_sub(1 + clock.chars().count())
                .max(3);
            Line::from(vec![
                Span::styled(fit(repo, repo_room, Trim::Tail), theme::hidden_header()),
                Span::raw(" "),
                Span::styled(clock, theme::dim()),
            ])
        }
        Row::FileItem {
            label,
            abspath,
            key,
            hidden: Some(reason),
            ..
        } => {
            let text = abspath.clone().unwrap_or_else(|| label.clone());
            render_hidden_item(
                &text,
                Trim::Head,
                app.is_done(key),
                *reason,
                highlighted,
                width,
            )
        }
        Row::CommandItem {
            command,
            key,
            hidden: Some(reason),
            ..
        } => render_hidden_item(
            &command.replace('\n', " \u{23ce} "),
            Trim::Tail,
            app.is_done(key),
            *reason,
            highlighted,
            width,
        ),
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
            ..
        } => {
            let done = app.is_done(key);
            let checkbox = if done { "[x] " } else { "[ ] " };
            let checkbox_style = if highlighted {
                theme::checkbox_highlighted()
            } else {
                theme::checkbox()
            };
            let text_style = match (done, highlighted) {
                // The cursor reverses the struck-through style rather than replacing it.
                (true, true) => theme::command_done().add_modifier(Modifier::REVERSED),
                (true, false) => theme::command_done(),
                (false, true) => theme::command_highlighted(),
                (false, false) => theme::command(),
            };
            let mut spans = vec![Span::raw("  "), Span::styled(checkbox, checkbox_style)];
            // A DONE row carries only its command: no warning, terminal or step label.
            if !done {
                if warn.is_some() {
                    spans.push(Span::styled("\u{26a0} ", theme::warn_icon()));
                }
                if let Some(terminal) = terminal {
                    spans.push(Span::styled(format!("[{terminal}] "), theme::dim()));
                }
                if let Some(step) = step {
                    spans.push(Span::styled(format!("{step}. "), theme::dim()));
                }
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
    fn report_options_sit_on_their_key_lines() {
        let backend = ratatui::backend::TestBackend::new(80, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut rects = [Rect::default(); 3];
        terminal
            .draw(|f| rects = render_report_prompt(f, f.area()))
            .unwrap();

        let buf = terminal.backend().buffer();
        for (rect, key) in rects.iter().zip(["y", "e", "Esc"]) {
            let line: String = (0..key.len() as u16)
                .map(|dx| buf[(rect.x + dx, rect.y)].symbol().to_string())
                .collect();
            assert_eq!(line, key);
            assert_eq!(rect.height, 2);
        }
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

    #[test]
    fn header_counts_drop_the_prefix_then_the_done_count() {
        assert_eq!(header_counts(4, 4, 29), "Queue: 4 open \u{b7} 4 done");
        assert_eq!(header_counts(4, 4, 21), "4 open \u{b7} 4 done");
        assert_eq!(header_counts(4, 4, 8), "4 open");
        assert_eq!(header_counts(4, 4, 4), "4 o\u{2026}");
    }

    #[test]
    fn footer_fills_row_one_then_row_two_and_drops_whole_segments() {
        let segs = ["Space done", "Bksp clear", "c clear done", "H hidden"];
        let [a, b] = footer_rows(&segs, 39);
        assert_eq!(a, "Space done \u{b7} Bksp clear \u{b7} c clear done");
        assert_eq!(b, "H hidden");
        let [a, b] = footer_rows(&segs, 23);
        assert_eq!(a, "Space done \u{b7} Bksp clear");
        assert_eq!(b, "c clear done \u{b7} H hidden");
        let [_, b] = footer_rows(&segs, 12);
        assert_eq!(b, "Bksp clear");
    }

    mod snapshots {
        use super::*;
        use ratatui::backend::TestBackend;
        use ratatui::buffer::Buffer;
        use ratatui::style::{Color, Modifier};
        use std::io::Write as _;

        const LOG: &str = "\
2026-11-04 17:10:00\tsession=sa\trepo=hub\tcwd=/home/d\tkind=command\titem=docker compose up -d
2026-11-04 17:15:00\tsession=sa\trepo=hub\tcwd=/home/d\tkind=command\titem=docker compose logs
2026-11-04 17:20:00\tsession=sa\trepo=hub\tcwd=/home/d\tkind=command\titem=curl -s localhost:3000/health
2026-11-04 17:27:00\tsession=sa\trepo=hub\tcwd=/home/d\tkind=command\tstep=1\titem=systemctl restart orca
2026-11-04 17:28:00\tsession=sa\trepo=hub\tcwd=/home/d\tkind=command\tstep=2\titem=hyprctl activewindow
2026-11-05 08:54:00\tsession=sb\trepo=hub\tcwd=/home/d\tkind=command\titem=docker compose ps
2026-11-05 09:05:00\tsession=sc\trepo=cqc\tcwd=/home/d\tkind=command\titem=cargo build
2026-11-05 09:07:00\tsession=sc\trepo=cqc\tcwd=/home/d\tkind=command\titem=cargo test
";
        const DONE: [&str; 4] = [
            "2026-11-04 17:10:00|sa|docker compose up -d",
            "2026-11-04 17:15:00|sa|docker compose logs",
            "2026-11-04 17:20:00|sa|curl -s localhost:3000/health",
            "2026-11-05 09:05:00|sc|cargo build",
        ];

        struct Files {
            _log: tempfile::NamedTempFile,
            _done: tempfile::NamedTempFile,
            _cleared: tempfile::NamedTempFile,
        }

        fn app_from(log: &str, done: &[&str]) -> (App, Files) {
            app_with_hides(log, done, &[])
        }

        /// Like `app_from`, with a `hide` event already logged for each of `hidden`.
        fn app_with_hides(log: &str, done: &[&str], hidden: &[&str]) -> (App, Files) {
            let mut log_file = tempfile::NamedTempFile::new().unwrap();
            write!(log_file, "{log}").unwrap();
            log_file.flush().unwrap();
            let done_file = tempfile::NamedTempFile::new().unwrap();
            for key in done {
                crate::done::mark(done_file.path(), key);
            }
            let cleared = tempfile::NamedTempFile::new().unwrap();
            for key in hidden {
                crate::cleared::hide(cleared.path(), key);
            }
            let app = App::with_paths(
                log_file.path().to_path_buf(),
                "/home/d".to_string(),
                done_file.path().to_path_buf(),
                cleared.path().to_path_buf(),
            );
            (
                app,
                Files {
                    _log: log_file,
                    _done: done_file,
                    _cleared: cleared,
                },
            )
        }

        fn sample() -> (App, Files) {
            app_from(LOG, &DONE)
        }

        /// Three hidden rows: `systemctl status orca` is superseded by the row that shares
        /// its `id`, `journalctl -u orca` is cleared, and `cargo build` is both superseded by
        /// the later done row and cleared, so it reads `cleared`.
        fn hidden_sample() -> (App, Files) {
            let log = format!(
                "2026-11-04 17:28:00\tsession=sa\trepo=hub\tcwd=/home/d\tkind=command\tid=svc\titem=systemctl status orca\n\
                 2026-11-04 17:28:00\tsession=sa\trepo=hub\tcwd=/home/d\tkind=command\titem=journalctl -u orca\n\
                 2026-11-05 09:07:00\tsession=sc\trepo=cqc\tcwd=/home/d\tkind=command\titem=cargo build\n\
                 {}",
                LOG.replace(
                    "step=1\titem=systemctl restart orca",
                    "step=1\tid=svc\titem=systemctl restart orca"
                )
            );
            app_with_hides(
                &log,
                &DONE,
                &[
                    "2026-11-04 17:28:00|sa|journalctl -u orca",
                    "2026-11-05 09:07:00|sc|cargo build",
                ],
            )
        }

        /// `content` with `tag` ending on the drawable width's last column.
        fn with_tag(content: &str, tag: &str, drawable: usize) -> String {
            let pad = drawable - content.chars().count() - tag.len();
            format!("{content}{}{tag}", " ".repeat(pad))
        }

        fn row_index(rows: &[String], text: &str) -> usize {
            rows.iter()
                .position(|r| r.trim_end() == text)
                .unwrap_or_else(|| panic!("no row reads {text:?}"))
        }

        fn draw_buffer(app: &mut App, width: u16, height: u16) -> Buffer {
            app.set_visible_height(body_height(app, width, height));
            let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| _ = draw(f, app)).unwrap();
            terminal.backend().buffer().clone()
        }

        fn rows_of(buf: &Buffer) -> Vec<String> {
            let area = buf.area;
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect()
        }

        /// Columns `from..to` of a row, counted in characters rather than bytes.
        fn cols(row: &str, from: usize, to: usize) -> String {
            row.chars().skip(from).take(to - from).collect()
        }

        /// A row's text padded so its trailing `x` lands on the drawable width's last column.
        fn with_x(content: &str, drawable: usize) -> String {
            let pad = drawable - 1 - content.chars().count();
            format!("{content}{}x", " ".repeat(pad))
        }

        fn expand_hub_summary(app: &mut App) {
            app.cursor = app
                .rows
                .iter()
                .position(|r| matches!(r, Row::Summary { repo, .. } if repo == "hub"));
            app.activate_cursor();
        }

        fn color_hex(color: Color) -> &'static str {
            match color {
                Color::Cyan => "#8abeb7",
                Color::Magenta => "#b294bb",
                Color::Yellow => "#f0c674",
                Color::Gray => "#a8aaa9",
                Color::DarkGray => "#707880",
                Color::Red => "#cc6666",
                Color::LightRed => "#ff8c8c",
                _ => "#d8dadb",
            }
        }

        /// Renders the buffer to an SVG with the mockups' stand-in palette and 10 x 20 cells.
        fn to_svg(buf: &Buffer) -> String {
            let (w, h) = (buf.area.width as usize, buf.area.height as usize);
            let mut svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" xml:space=\"preserve\">\n\
                 <style>text {{ font-family: \"Liberation Mono\", monospace; font-size: 16.6px; }}</style>\n\
                 <rect width=\"{}\" height=\"{}\" fill=\"#1d1f21\"/>\n\
                 <rect width=\"{}\" height=\"20\" fill=\"#2a2d30\"/><rect x=\"{}\" width=\"10\" height=\"{}\" fill=\"#2a2d30\"/>\n",
                w * 10,
                h * 20,
                w * 10,
                h * 20,
                w * 10,
                (w - 1) * 10,
                h * 20
            );
            for y in 0..h {
                for x in 0..w {
                    let cell = &buf[(x as u16, y as u16)];
                    if cell.modifier.contains(Modifier::REVERSED) {
                        svg.push_str(&format!(
                            "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"20\" fill=\"{}\"/>\n",
                            x * 10,
                            y * 20,
                            color_hex(cell.fg)
                        ));
                    }
                }
                for x in 0..w {
                    let cell = &buf[(x as u16, y as u16)];
                    if cell.symbol() == " " {
                        continue;
                    }
                    let reversed = cell.modifier.contains(Modifier::REVERSED);
                    let fill = if reversed {
                        "#1d1f21"
                    } else {
                        color_hex(cell.fg)
                    };
                    let mut style = format!("fill:{fill};");
                    if cell.modifier.contains(Modifier::BOLD) {
                        style.push_str("font-weight:bold;");
                    }
                    if cell.modifier.contains(Modifier::DIM) {
                        style.push_str("opacity:0.6;");
                    }
                    if cell.modifier.contains(Modifier::CROSSED_OUT) {
                        style.push_str("text-decoration:line-through;");
                    }
                    svg.push_str(&format!(
                        "<text x=\"{}\" y=\"{}\" style=\"{}\">{}</text>\n",
                        x * 10,
                        y * 20 + 15,
                        style,
                        cell.symbol().replace('&', "&amp;").replace('<', "&lt;")
                    ));
                }
            }
            svg.push_str("</svg>\n");
            svg
        }

        /// With `REVIEW_PANEL_CAPTURE_DIR` set, writes each state's SVG for the fidelity gate.
        fn capture(name: &str, buf: &Buffer) {
            if let Ok(dir) = std::env::var("REVIEW_PANEL_CAPTURE_DIR") {
                std::fs::write(format!("{dir}/{name}.svg"), to_svg(buf)).unwrap();
            }
        }

        #[test]
        fn default_state_at_40_columns() {
            let (mut app, _files) = sample();
            let buf = draw_buffer(&mut app, 40, 24);
            capture("default-40", &buf);
            let rows = rows_of(&buf);
            let w = 39;
            assert_eq!(rows[0].trim(), "");
            assert!(rows[1].starts_with("Queue: 4 open \u{b7} 4 done"));
            assert_eq!(cols(&rows[1], 31, 36), "clear");
            assert_eq!(rows[1].chars().nth(38), Some("x".chars().next().unwrap()));
            assert_eq!(rows[2].trim_end(), "\u{2500}".repeat(w));
            assert_eq!(rows[3].trim_end(), "OPEN");
            assert_eq!(rows[4].trim_end(), with_x("hub 17:28", w));
            assert_eq!(
                rows[5].trim_end(),
                with_x("  [ ] 1. systemctl restart orca", w)
            );
            assert_eq!(
                rows[6].trim_end(),
                with_x("  [ ] 2. hyprctl activewindow", w)
            );
            assert_eq!(rows[7].trim_end(), with_x("hub 08:54", w));
            assert_eq!(rows[8].trim_end(), with_x("  [ ] docker compose ps", w));
            assert_eq!(rows[9].trim_end(), with_x("cqc 09:07", w));
            assert_eq!(rows[10].trim_end(), with_x("  [ ] cargo test", w));
            assert_eq!(rows[11].trim_end(), "");
            assert_eq!(rows[12].trim_end(), "DONE (4)");
            assert_eq!(rows[13].trim_end(), with_x("hub  [x] 3 rows, 17:20", w));
            assert_eq!(rows[14].trim_end(), with_x("cqc  [x] 1 row, 09:05", w));
            assert_eq!(
                rows[22].trim_end(),
                "Space done \u{b7} Bksp clear \u{b7} c clear done"
            );
            assert_eq!(
                rows[23].trim_end(),
                "H hidden \u{b7} S clear session \u{b7} Enter copy"
            );
            // The cursor starts on the newest open row, with its box and command reversed.
            assert!(buf[(2, 10)].modifier.contains(Modifier::REVERSED));
            assert!(buf[(6, 10)].modifier.contains(Modifier::REVERSED));
            assert!(!buf[(38, 10)].modifier.contains(Modifier::REVERSED));
        }

        #[test]
        fn done_expanded_state_at_40_columns() {
            let (mut app, _files) = sample();
            expand_hub_summary(&mut app);
            let buf = draw_buffer(&mut app, 40, 24);
            capture("expanded-40", &buf);
            let rows = rows_of(&buf);
            let w = 39;
            assert_eq!(
                rows[12].trim_end(),
                format!("{:<28}Enter: fold", "DONE (4)")
            );
            assert_eq!(rows[13].trim_end(), with_x("hub  [x] 3 rows, 17:20", w));
            assert_eq!(rows[14].trim_end(), with_x("  [x] docker compose up -d", w));
            assert_eq!(rows[15].trim_end(), with_x("  [x] docker compose logs", w));
            assert_eq!(
                rows[16].trim_end(),
                with_x("  [x] curl -s localhost:3000/health", w)
            );
            assert_eq!(rows[17].trim_end(), with_x("cqc  [x] 1 row, 09:05", w));
            assert!(buf[(6, 14)].modifier.contains(Modifier::CROSSED_OUT));
            assert!(buf[(0, 13)].modifier.contains(Modifier::REVERSED));
            assert!(!buf[(38, 13)].modifier.contains(Modifier::REVERSED));
            assert_eq!(
                rows[22].trim_end(),
                "Enter fold \u{b7} Bksp clear \u{b7} c clear done"
            );
            assert_eq!(rows[23].trim_end(), "H hidden \u{b7} S clear session");
        }

        #[test]
        fn empty_state_at_40_columns() {
            let (mut app, _files) = app_from("", &[]);
            let buf = draw_buffer(&mut app, 40, 24);
            capture("empty-40", &buf);
            let rows = rows_of(&buf);
            assert_eq!(
                rows[1].trim_end(),
                format!("{:<38}x", "Queue: 0 open \u{b7} 0 done")
            );
            assert_eq!(rows[3].trim_end(), "Nothing to review.");
            assert_eq!(rows[23].trim_end(), "H hidden \u{b7} q close");
            assert!(rows.iter().all(|r| !r.contains("clear")));
        }

        #[test]
        fn open_empty_with_done_rows_shows_the_note_then_the_drawer() {
            let (mut app, _files) = app_from(
                LOG,
                &[
                    "2026-11-04 17:10:00|sa|docker compose up -d",
                    "2026-11-04 17:15:00|sa|docker compose logs",
                    "2026-11-04 17:20:00|sa|curl -s localhost:3000/health",
                    "2026-11-04 17:27:00|sa|systemctl restart orca",
                    "2026-11-04 17:28:00|sa|hyprctl activewindow",
                    "2026-11-05 08:54:00|sb|docker compose ps",
                    "2026-11-05 09:05:00|sc|cargo build",
                    "2026-11-05 09:07:00|sc|cargo test",
                ],
            );
            let buf = draw_buffer(&mut app, 40, 24);
            let rows = rows_of(&buf);
            assert_eq!(rows[3].trim_end(), "OPEN");
            assert_eq!(rows[4].trim_end(), "Nothing open.");
            // The cursor starts on the first summary, so the DONE label carries its hint.
            assert_eq!(rows[6].trim_end(), format!("{:<28}Enter: open", "DONE (8)"));
        }

        #[test]
        fn default_state_at_32_columns_keeps_every_x() {
            let (mut app, _files) = sample();
            let buf = draw_buffer(&mut app, 32, 24);
            capture("default-32", &buf);
            let rows = rows_of(&buf);
            let w = 31;
            assert!(rows[1].starts_with("4 open \u{b7} 4 done"));
            assert_eq!(cols(&rows[1], 23, 28), "clear");
            assert_eq!(
                rows[5].trim_end(),
                with_x("  [ ] 1. systemctl restart o\u{2026}", w)
            );
            assert_eq!(
                rows[6].trim_end(),
                with_x("  [ ] 2. hyprctl activewindow", w)
            );
            for y in [4, 5, 6, 7, 8, 9, 10, 13, 14] {
                assert_eq!(buf[(30, y)].symbol(), "x", "row {y} lost its x");
            }
            assert_eq!(rows[22].trim_end(), "Space done \u{b7} Bksp clear");
            assert_eq!(rows[23].trim_end(), "c clear done \u{b7} H hidden");
        }

        #[test]
        fn done_expanded_state_at_32_columns() {
            let (mut app, _files) = sample();
            expand_hub_summary(&mut app);
            let buf = draw_buffer(&mut app, 32, 24);
            capture("expanded-32", &buf);
            let rows = rows_of(&buf);
            let w = 31;
            assert_eq!(
                rows[12].trim_end(),
                format!("{:<20}Enter: fold", "DONE (4)")
            );
            assert_eq!(
                rows[16].trim_end(),
                with_x("  [x] curl -s localhost:3000\u{2026}", w)
            );
            for y in [13, 14, 15, 16, 17] {
                assert_eq!(buf[(30, y)].symbol(), "x", "row {y} lost its x");
            }
            assert_eq!(rows[22].trim_end(), "Enter fold \u{b7} Bksp clear");
            assert_eq!(rows[23].trim_end(), "c clear done \u{b7} H hidden");
        }

        #[test]
        fn empty_state_at_32_columns_uses_the_narrow_header() {
            let (mut app, _files) = app_from("", &[]);
            let buf = draw_buffer(&mut app, 32, 24);
            capture("empty-32", &buf);
            let rows = rows_of(&buf);
            assert_eq!(
                rows[1].trim_end(),
                format!("{:<30}x", "0 open \u{b7} 0 done")
            );
            assert_eq!(rows[3].trim_end(), "Nothing to review.");
        }

        #[test]
        fn default_view_never_draws_hidden_rows() {
            let (mut app, _files) = hidden_sample();
            let buf = draw_buffer(&mut app, 40, 24);
            let rows = rows_of(&buf);
            assert!(rows[1].starts_with("Queue: 4 open \u{b7} 4 done"));
            assert_eq!(cols(&rows[1], 31, 36), "clear");
            for text in [
                "HIDDEN",
                "superseded",
                "cleared",
                "journalctl",
                "status orca",
            ] {
                assert!(rows.iter().all(|r| !r.contains(text)), "{text} leaked");
            }
        }

        #[test]
        fn hidden_view_at_40_columns() {
            let (mut app, _files) = hidden_sample();
            app.toggle_hidden_view();
            let buf = draw_buffer(&mut app, 40, 24);
            capture("hidden-40", &buf);
            let rows = rows_of(&buf);
            let w = 39;
            assert!(rows[1].starts_with("Queue: 4 open \u{b7} 4 done"));
            assert_eq!(cols(&rows[1], 29, 36), "restore");
            assert!(!rows[1].contains("clear"));
            assert_eq!(rows[1].chars().nth(38), Some('x'));

            // OPEN and DONE keep their rows and their x.
            assert_eq!(rows[3].trim_end(), "OPEN");
            assert_eq!(rows[10].trim_end(), with_x("  [ ] cargo test", w));
            assert_eq!(rows[13].trim_end(), with_x("hub  [x] 3 rows, 17:20", w));

            let label = row_index(&rows, "HIDDEN (3)");
            assert_eq!(label, 16);
            assert_eq!(rows[label - 1].trim_end(), "", "blank row above the label");
            assert_eq!(rows[label + 1].trim_end(), "hub 17:28");
            assert_eq!(
                rows[label + 2].trim_end(),
                with_tag("  [ ] systemctl status orca", "superseded", w)
            );
            assert_eq!(
                rows[label + 3].trim_end(),
                with_tag("  [ ] journalctl -u orca", "cleared", w)
            );
            assert_eq!(rows[label + 4].trim_end(), "cqc 09:07");
            assert_eq!(
                rows[label + 5].trim_end(),
                with_tag("  [ ] cargo build", "cleared", w)
            );
            // The one-row footer leaves the row above it blank.
            assert_eq!(rows[22].trim_end(), "");
            assert_eq!(
                rows[23].trim_end(),
                "r restore \u{b7} R restore all \u{b7} H back"
            );

            // The cursor starts on the first hidden row: checkbox and text reverse.
            assert!(
                buf[(2, (label + 2) as u16)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
            assert!(
                buf[(6, (label + 2) as u16)]
                    .modifier
                    .contains(Modifier::REVERSED)
            );
            // Hidden text is dimmed grey, the tag is dim, and the header text is dim bold.
            assert_eq!(buf[(6, (label + 3) as u16)].fg, Color::DarkGray);
            assert!(
                buf[(38, (label + 3) as u16)]
                    .modifier
                    .contains(Modifier::DIM)
            );
            assert!(
                buf[(0, (label + 1) as u16)]
                    .modifier
                    .contains(Modifier::DIM)
            );
            assert!(
                buf[(0, (label + 1) as u16)]
                    .modifier
                    .contains(Modifier::BOLD)
            );
        }

        #[test]
        fn hidden_view_at_32_columns_keeps_the_x_and_the_tag_and_a_blank_above_the_footer() {
            let (mut app, _files) = hidden_sample();
            app.toggle_hidden_view();
            let buf = draw_buffer(&mut app, 32, 24);
            capture("hidden-32", &buf);
            let rows = rows_of(&buf);
            let w = 31;
            assert!(rows[1].starts_with("4 open \u{b7} 4 done"));
            assert_eq!(cols(&rows[1], 21, 28), "restore");

            // 19 body rows no longer fit beside a blank row, so the list is scrolled to the
            // bottom and the footer stays put.
            assert_eq!(
                rows[17].trim_end(),
                with_tag("  [ ] systemctl sta\u{2026}", "superseded", w)
            );
            assert_eq!(
                rows[18].trim_end(),
                with_tag("  [ ] journalctl -u or\u{2026}", "cleared", w)
            );
            assert_eq!(
                rows[20].trim_end(),
                with_tag("  [ ] cargo build", "cleared", w)
            );
            assert_eq!(rows[21].trim_end(), "", "blank row above the footer");
            assert_eq!(rows[22].trim_end(), "r restore \u{b7} R restore all");
            assert_eq!(rows[23].trim_end(), "H back");

            for text in ["  [ ] docker compose ps", "cqc 09:07", "hub 08:54"] {
                let y = rows
                    .iter()
                    .position(|r| r.starts_with(text) && r.trim_end().ends_with('x'))
                    .unwrap_or_else(|| panic!("{text} lost its x"));
                assert_eq!(buf[(30, y as u16)].symbol(), "x");
            }
        }

        #[test]
        fn hidden_view_with_nothing_hidden_says_so() {
            let (mut app, _files) = sample();
            app.toggle_hidden_view();
            let buf = draw_buffer(&mut app, 40, 24);
            let rows = rows_of(&buf);
            let label = row_index(&rows, "HIDDEN (0)");
            assert_eq!(rows[label + 1].trim_end(), "Nothing hidden.");
            assert!(!rows[1].contains("restore"));
        }
    }
}
