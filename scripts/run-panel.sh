#!/usr/bin/env bash
# Runs inside the Orca split that plugin.sh opens: starts the panel, then closes the pane.
#
#   run-panel.sh <handle of the pane that flagged the items>
#
# The split's shell outlives the panel, so this closes the pane once the binary exits, which
# covers q, Esc, the x, and a crash alike. Everything the panel needs is derived here so the
# line typed into the shell stays short.
set -uo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=pane-paths.sh
source "$script_dir/pane-paths.sh"

ORCA="${ORCA_CLI_COMMAND:-${ORCA_BIN:-orca-ide}}"

export REVIEW_PANEL_ROOT="$(cd "$script_dir/.." && pwd)"
export REVIEW_PANEL_LOG="$(pane_scoped_path "")"
export REVIEW_PANEL_DONE_LOG="$(pane_scoped_path "-done")"
export REVIEW_PANEL_CLEARED_LOG="$(pane_scoped_path "-cleared")"
export REVIEW_PANEL_NOTIFY_TARGET="${1:-}"

"$REVIEW_PANEL_ROOT/target/release/review-panel"

"$ORCA" terminal close --terminal "$ORCA_TERMINAL_HANDLE" >/dev/null 2>&1
