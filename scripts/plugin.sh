#!/usr/bin/env bash
# Review Queue actions, run under Orca.
#
#   plugin.sh open       open the sidebar, no-op if one is already open
#   plugin.sh toggle     open the sidebar, or close it if one is already open
#   plugin.sh close      close every Review Queue pane in this tab, no-op if none
#   plugin.sh clear      truncate the underlying review.log, done-marks, and cleared-marks files
#   plugin.sh open-item  open the file:// path Ctrl+clicked in the sidebar
#   plugin.sh notify     tell the agent that flagged items to continue
#
# Orca has no plugin panes, so the sidebar binary runs in an `orca terminal split` beside the
# pane that flagged the item. There's no separate state file: the tab's live terminal list
# (matched by title) is the source of truth for "is it open".
set -uo pipefail

mode="${1:-toggle}"
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

# Never bare `orca`: outside an Orca terminal it resolves to the GNOME screen reader.
ORCA="${ORCA_CLI_COMMAND:-${ORCA_BIN:-orca-ide}}"
in_orca() { [ -n "${ORCA_TERMINAL_HANDLE:-}" ] && command -v "$ORCA" >/dev/null 2>&1; }
# shellcheck source=pane-paths.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/pane-paths.sh"
QUEUE_LOG="${REVIEW_PANEL_LOG:-$(pane_scoped_path "")}"
DONE_LOG="${REVIEW_PANEL_DONE_LOG:-$(pane_scoped_path "-done")}"
CLEARED_LOG="${REVIEW_PANEL_CLEARED_LOG:-$(pane_scoped_path "-cleared")}"
plugin_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$plugin_root/target/release/review-panel"

refuse() {
  printf 'review-panel: %s\n' "$1" >&2
  exit 1
}

# An already-open pane keeps whatever code it started with - a rebuild alone does not reach
# it. Left unchecked this produces the exact bug that motivated this guard: a fix lands in
# source, nobody rebuilds+restarts, and the panel silently runs stale code with no error (see
# README's "After changing the log format" note - this generalizes that to every source
# change, not just log-format ones). Only applies to a dev checkout with source next to it;
# an install shipping only the compiled binary has nothing here to compare against.
rebuild_if_stale() {
  [ -d "$plugin_root/src" ] || return 0
  command -v cargo >/dev/null 2>&1 || return 0
  local stale=""
  if [ ! -x "$BIN" ]; then
    stale=1
  else
    stale=$(find "$plugin_root/src" "$plugin_root/Cargo.toml" "$plugin_root/Cargo.lock" \
      -newer "$BIN" -print -quit 2>/dev/null)
  fi
  [ -n "$stale" ] || return 0
  printf 'review-panel: binary is stale, rebuilding...\n' >&2
  if (cd "$plugin_root" && cargo build --release >/dev/null 2>&1); then
    printf 'review-panel: rebuilt\n' >&2
  else
    printf 'review-panel: rebuild failed, launching existing binary anyway\n' >&2
  fi
}

if [ "$mode" = "clear" ]; then
  : >"$QUEUE_LOG" 2>/dev/null || refuse "cannot clear $QUEUE_LOG"
  : >"$DONE_LOG" 2>/dev/null
  : >"$CLEARED_LOG" 2>/dev/null
  printf 'cleared %s\n' "$QUEUE_LOG"
  exit 0
fi

# Invoked by the running panel itself (see actions::notify_agent) once it decides the
# human is done with a group (or the whole queue) - tells the agent that flagged those
# items to continue, so nobody has to go back and prompt it by hand.
if [ "$mode" = "notify" ]; then
  target="${REVIEW_PANEL_NOTIFY_TARGET:-}"
  text="${REVIEW_PANEL_NOTIFY_TEXT:-}"
  [ -n "$target" ] || refuse "no notify target (REVIEW_PANEL_NOTIFY_TARGET unset)"
  [ -n "$text" ] || refuse "no notify text (REVIEW_PANEL_NOTIFY_TEXT unset)"
  command -v "$ORCA" >/dev/null 2>&1 || refuse "$ORCA not on PATH"
  "$ORCA" terminal send --terminal "$target" --text "$text" --enter >/dev/null 2>&1 ||
    refuse "orca terminal send failed for $target"
  printf 'notified %s\n' "$target"
  exit 0
fi

if [ "$mode" = "open-item" ]; then
  url="${REVIEW_PANEL_CLICKED_URL:-}"
  [ -n "$url" ] || refuse "no clicked URL (REVIEW_PANEL_CLICKED_URL unset)"
  item_path="${url#file://}"
  [ -e "$item_path" ] || refuse "no such file: $item_path"
  # Under WSL, `code` on PATH is the Remote-WSL wrapper: it only works while its remote-cli
  # shim exists, which requires a VS Code window currently connected to this distro. Check for
  # that shim up front instead of just invoking `code` and handling the failure after the
  # fact - firing the wrapper when we already know it can't connect is pointless and, on some
  # setups, pops a distracting error window. Elsewhere (native Linux/macOS), `code` on PATH
  # works standalone with no such precondition.
  vscode_ready() {
    command -v code >/dev/null 2>&1 || return 1
    if [ -n "${WSL_DISTRO_NAME:-}" ] || grep -qi microsoft /proc/version 2>/dev/null; then
      compgen -G "$HOME/.vscode-server/bin/*/bin/remote-cli/code" >/dev/null 2>&1
    fi
  }

  # Preference order: Orca's editor -> VS Code -> xdg-open on the containing dir (native Linux
  # file manager, works regardless of desktop environment) -> explorer.exe (real WSL2 only,
  # opens *something* on the Windows side but not necessarily the intended app/view). Run each
  # candidate synchronously - a quick RPC/handler lookup, not a wait for the opened app itself -
  # and check its exit status too, so an unexpected failure still falls through instead of
  # silently opening nothing. The caller (the Rust sidebar) reports `opener=` back to the user,
  # since explorer.exe is a degraded fallback worth calling out.
  opener=""
  # Orca's editor only takes paths inside the current worktree (`invalid_relative_path`
  # otherwise), so anything it refuses falls through to the chain below unchanged.
  if in_orca && "$ORCA" file open "$item_path" --json 2>/dev/null | jq -e '.ok == true' >/dev/null 2>&1; then
    opener="orca"
  elif vscode_ready && code -g "$item_path" >/dev/null 2>&1; then
    opener="code"
  elif command -v xdg-open >/dev/null 2>&1 && xdg-open "$(dirname "$item_path")" >/dev/null 2>&1; then
    opener="xdg-open"
  elif command -v explorer.exe >/dev/null 2>&1; then
    winpath=$(wslpath -w "$item_path" 2>/dev/null) || winpath="$item_path"
    explorer.exe "$winpath" >/dev/null 2>&1 &
    opener="explorer"
  else
    refuse "no opener found (orca/code/xdg-open/explorer.exe)"
  fi
  printf 'opened %s opener=%s\n' "$item_path" "$opener"
  exit 0
fi

in_orca || refuse "not in an Orca terminal (ORCA_TERMINAL_HANDLE unset or $ORCA missing)"

# The sidebar closing itself names its own pane directly. It has already exited by the
# time this runs, so the split's shell prompt is back and has reset the pane title; a
# title lookup here would find nothing and strand the user in that shell.
if [ "$mode" = "close" ] && [ "${REVIEW_PANEL_SELF_CLOSE:-}" = "1" ]; then
  "$ORCA" terminal close --terminal "$ORCA_TERMINAL_HANDLE" >/dev/null 2>&1 ||
    refuse "orca terminal close failed for $ORCA_TERMINAL_HANDLE"
  printf 'closed review panel %s\n' "$ORCA_TERMINAL_HANDLE"
  exit 0
fi

# "Is it open" is the live terminal list filtered by title within this tab. The tab comes
# from the env of whoever invoked us: the Stop hook's Claude pane, or the sidebar itself on
# close. A pane `terminal close` just ended stays listed for a while with connected=false,
# hence the check.
tab="${ORCA_TAB_ID:-}"
EXISTING=$("$ORCA" terminal list --json 2>/dev/null |
  jq -r --arg tab "$tab" '.result.terminals[] | select(.title == "Review Queue" and .connected == true and ($tab == "" or .tabId == $tab)) | .handle' 2>/dev/null)

close_existing() {
  local failed="" p
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    "$ORCA" terminal close --terminal "$p" >/dev/null 2>&1 || failed="$failed $p"
  done <<EOF
$EXISTING
EOF
  [ -z "$failed" ] || refuse "failed to close$failed"
}

case "$mode" in
close)
  [ -n "$EXISTING" ] || {
    printf 'close: no review panel open in this tab\n'
    exit 0
  }
  close_existing
  printf 'closed review panel in this tab\n'
  ;;
toggle)
  if [ -n "$EXISTING" ]; then
    close_existing
    printf 'closed review panel in this tab\n'
    exit 0
  fi
  ;&
open)
  if [ -n "$EXISTING" ]; then
    printf 'open: already open (%s) in this tab\n' "$(printf '%s' "$EXISTING" | tr '\n' ' ' | sed 's/ $//')"
    exit 0
  fi
  rebuild_if_stale
  # --command is typed into the split's shell, which outlives the sidebar, so the pane is
  # closed by the shell once the binary exits: that covers q, Esc, the x, and a crash
  # alike. $ORCA_TERMINAL_HANDLE is left for that shell to expand, as it names the split
  # pane itself. REVIEW_PANEL_ROOT is how the binary finds this script.
  printf -v launch 'REVIEW_PANEL_ROOT=%q REVIEW_PANEL_LOG=%q REVIEW_PANEL_DONE_LOG=%q REVIEW_PANEL_CLEARED_LOG=%q REVIEW_PANEL_NOTIFY_TARGET=%q %q; %q terminal close --terminal "$ORCA_TERMINAL_HANDLE" >/dev/null 2>&1' \
    "$plugin_root" "$QUEUE_LOG" "$DONE_LOG" "$CLEARED_LOG" "$ORCA_TERMINAL_HANDLE" "$BIN" "$ORCA"
  # "vertical" is Orca's name for side-by-side (the divider is vertical); horizontal stacks.
  # A split issued within a second or two of a pane closing in the same tab fails with
  # "Timed out waiting for split pane handle" while the layout settles; the same call
  # succeeds a moment later, so retry rather than lose the pop-open.
  attempt=0
  until out=$("$ORCA" terminal split --terminal "$ORCA_TERMINAL_HANDLE" --direction vertical --command "$launch" --json 2>&1); do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 3 ] || refuse "orca terminal split failed: $(printf '%s' "$out" | jq -r '.error.message // .' 2>/dev/null | head -c 200)"
    sleep 1.5
  done
  opened=$(printf '%s' "$out" | jq -r '.result.split.handle // empty' 2>/dev/null)
  [ -n "$opened" ] || refuse "orca terminal split returned no handle"
  "$ORCA" terminal rename --terminal "$opened" --title "Review Queue" >/dev/null 2>&1
  # The split takes focus; give it back to the pane that flagged the item. `focus` isn't
  # a real orca terminal subcommand (only `switch` is) - this silently no-op'd before,
  # which is why the panel kept stealing focus on open despite this line existing.
  "$ORCA" terminal switch --terminal "$ORCA_TERMINAL_HANDLE" >/dev/null 2>&1
  printf 'opened review panel %s in this tab\n' "$opened"
  ;;
*)
  refuse "unknown mode: $mode"
  ;;
esac
