//! Closing the panel, opening a flagged file, and notifying the agent all reuse
//! scripts/plugin.sh rather than reimplementing that logic here, so there's one place that
//! knows how to do each.

use crate::cleared;
use crate::log::{self, Row};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn plugin_sh() -> Option<String> {
    std::env::var("REVIEW_PANEL_ROOT")
        .ok()
        .map(|root| format!("{root}/scripts/plugin.sh"))
}

pub fn close() {
    let Some(script) = plugin_sh() else { return };
    // spawn(), not status(): this call sits in the middle of the event loop's click handling,
    // so blocking here - even briefly - would make every click feel laggy regardless of how
    // fast the shelled-out script actually is.
    let _ = Command::new("bash")
        .arg(script)
        .arg("close")
        // Tells plugin.sh's Orca branch this is the panel's own pane (see its comment).
        .env("REVIEW_PANEL_SELF_CLOSE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Appends one `hide` event per row currently visible. review.log and the done log are only
/// read, so nothing is lost and the hidden rows can be restored later.
pub fn clear_all(log_path: &Path, cleared_path: &Path) {
    let entries = log::read_entries(log_path);
    let home = std::env::var("HOME").unwrap_or_default();
    let state = cleared::load(cleared_path);
    for row in log::build_rows(&entries, &home, &state).visible {
        if let Row::FileItem { key, .. } | Row::CommandItem { key, .. } = row {
            cleared::hide(cleared_path, &key);
        }
    }
}

/// Tells the agent that flagged these items to continue, via `orca terminal send` - see
/// plugin.sh's `notify` mode.
pub fn notify_agent(target: &str, text: &str) {
    let Some(script) = plugin_sh() else { return };
    let _ = Command::new("bash")
        .arg(script)
        .arg("notify")
        .env("REVIEW_PANEL_NOTIFY_TARGET", target)
        .env("REVIEW_PANEL_NOTIFY_TEXT", text)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// OSC 52 sets the terminal's own clipboard, not the process's - it's the only approach that
/// reliably crosses the WSL -> Windows Terminal boundary, since a system-clipboard crate
/// (X11/Wayland/Win32 API) would only ever reach a clipboard inside WSL, not the Windows one.
pub fn copy_to_clipboard(text: &str) {
    let encoded = STANDARD.encode(text);
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b]52;c;{encoded}\x07");
    let _ = stdout.flush();
}

/// Which opener plugin.sh actually used - it tries VS Code first, falling through to
/// xdg-open/explorer.exe only if that fails, so the caller can tell the user when the file
/// landed somewhere other than the intended editor view.
pub enum Opener {
    Editor,
    Fallback,
    Unknown,
}

pub fn open_item(abspath: &str) -> Opener {
    let Some(script) = plugin_sh() else {
        return Opener::Unknown;
    };
    // .output(), not .spawn(): plugin.sh's own opener attempts are synchronous RPC/handler
    // lookups (milliseconds), not a wait for the opened app itself, so capturing which one
    // succeeded doesn't reintroduce the perceptible delay spawn() was added to avoid.
    let Ok(output) = Command::new("bash")
        .arg(script)
        .arg("open-item")
        .env("REVIEW_PANEL_CLICKED_URL", format!("file://{abspath}"))
        .output()
    else {
        return Opener::Unknown;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.trim().rsplit("opener=").next() {
        Some("code") | Some("orca") => Opener::Editor,
        Some("xdg-open") | Some("explorer") => Opener::Fallback,
        _ => Opener::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_all_leaves_the_log_untouched_and_only_appends_hide_events() {
        let mut log_file = tempfile::NamedTempFile::new().unwrap();
        write!(
            log_file,
            "2026-11-05 09:00:00 session=s1 repo=r cwd=/home/d kind=command item=echo hi\n\
             2026-11-05 09:01:00 session=s2 repo=r cwd=/home/d item=a.txt\n"
        )
        .unwrap();
        log_file.flush().unwrap();
        let cleared_file = tempfile::NamedTempFile::new().unwrap();
        let before = std::fs::read(log_file.path()).unwrap();

        clear_all(log_file.path(), cleared_file.path());

        assert_eq!(std::fs::read(log_file.path()).unwrap(), before);
        let written = std::fs::read_to_string(cleared_file.path()).unwrap();
        let lines: Vec<&str> = written.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with(" hide 2026-11-05 09:00:00|s1|echo hi"));
        assert!(lines[1].ends_with(" hide 2026-11-05 09:01:00|s2|a.txt"));

        // Running it again hides nothing new and still never cuts the file.
        clear_all(log_file.path(), cleared_file.path());
        assert_eq!(
            std::fs::read_to_string(cleared_file.path())
                .unwrap()
                .lines()
                .count(),
            2
        );
    }
}
