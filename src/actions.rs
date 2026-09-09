//! Closing the panel and opening a flagged file both reuse scripts/plugin.sh rather than
//! reimplementing that logic here - it's the same code path the tools-menu keybinding and
//! `herdr plugin action invoke` already use, so there's one place that knows how to do each.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::io::Write;
use std::process::{Command, Stdio};

fn plugin_sh() -> Option<String> {
    std::env::var("HERDR_PLUGIN_ROOT")
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

pub fn clear_all() {
    let Some(script) = plugin_sh() else { return };
    let _ = Command::new("bash")
        .arg(script)
        .arg("clear")
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
        .env("HERDR_PLUGIN_CLICKED_URL", format!("file://{abspath}"))
        .output()
    else {
        return Opener::Unknown;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    match stdout.trim().rsplit("opener=").next() {
        Some("code") => Opener::Editor,
        Some("xdg-open") | Some("explorer") => Opener::Fallback,
        _ => Opener::Unknown,
    }
}
