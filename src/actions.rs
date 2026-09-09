//! Closing the panel and opening a flagged file both reuse scripts/plugin.sh rather than
//! reimplementing that logic here - it's the same code path the tools-menu keybinding and
//! `herdr plugin action invoke` already use, so there's one place that knows how to do each.

use std::process::{Command, Stdio};

fn plugin_sh() -> Option<String> {
    std::env::var("HERDR_PLUGIN_ROOT")
        .ok()
        .map(|root| format!("{root}/scripts/plugin.sh"))
}

pub fn close() {
    let Some(script) = plugin_sh() else { return };
    let _ = Command::new("bash")
        .arg(script)
        .arg("close")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn open_item(abspath: &str) {
    let Some(script) = plugin_sh() else { return };
    let _ = Command::new("bash")
        .arg(script)
        .arg("open-item")
        .env("HERDR_PLUGIN_CLICKED_URL", format!("file://{abspath}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
