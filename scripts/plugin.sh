#!/usr/bin/env bash
# Review Queue plugin actions.
#
#   plugin.sh open       open the sidebar, no-op if one is already open
#   plugin.sh toggle     open the sidebar, or close it if one is already open
#   plugin.sh close      close every Review Queue pane in the workspace, no-op if none
#   plugin.sh clear      truncate the underlying review.log, done-marks, and cleared-marks files
#   plugin.sh open-item  open the file:// path Ctrl+clicked in the sidebar (link_handlers)
#   plugin.sh tools-menu open the fzf tool picker popup (see scripts/tools-menu.sh)
#
# Mirrors the memex plugin's open/close/toggle pattern: there's no separate state file, the
# workspace's live pane list (matched by label) is the source of truth for "is it open".
set -uo pipefail

mode="${1:-toggle}"
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

H="${HERDR_BIN_PATH:-herdr}"
PLUGIN_ID="${HERDR_PLUGIN_ID:-deetss.review-panel}"
ws="${HERDR_WORKSPACE_ID:-}"
pane="${HERDR_PANE_ID:-}"
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

# herdr runs $BIN directly (see herdr-plugin.toml) and an already-open pane keeps whatever
# code it started with - a rebuild alone does not reach it. Left unchecked this produces the
# exact bug that motivated this guard: a fix lands in source, nobody rebuilds+restarts, and
# the panel silently runs stale code with no error (see README's "After changing the log
# format" note - this generalizes that to every source change, not just log-format ones).
# Only applies to a dev checkout with source next to it; an install shipping only the
# compiled binary has nothing here to compare against and is left alone.
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
  if ( cd "$plugin_root" && cargo build --release >/dev/null 2>&1 ); then
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

if [ "$mode" = "tools-menu" ]; then
  [ -n "$ws" ] || refuse "no workspace context (invoke from inside herdr)"
  out=$("$H" plugin pane open --plugin "$PLUGIN_ID" --entrypoint tools-menu --placement popup 2>&1) ||
    refuse "herdr plugin pane open failed: $out"
  printf 'opened tools menu\n'
  exit 0
fi

if [ "$mode" = "open-item" ]; then
  url="${HERDR_PLUGIN_CLICKED_URL:-}"
  [ -n "$url" ] || refuse "no clicked URL (HERDR_PLUGIN_CLICKED_URL unset)"
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

  # Preference order: VS Code (primary editor) -> xdg-open on the containing dir (native Linux
  # file manager, works regardless of desktop environment) -> explorer.exe (real WSL2 only,
  # opens *something* on the Windows side but not necessarily the intended app/view). Run each
  # candidate synchronously - a quick RPC/handler lookup, not a wait for the opened app itself -
  # and check its exit status too, so an unexpected failure still falls through instead of
  # silently opening nothing. The caller (the Rust sidebar) reports `opener=` back to the user,
  # since explorer.exe is a degraded fallback worth calling out.
  opener=""
  if vscode_ready && code -g "$item_path" >/dev/null 2>&1; then
    opener="code"
  elif command -v xdg-open >/dev/null 2>&1 && xdg-open "$(dirname "$item_path")" >/dev/null 2>&1; then
    opener="xdg-open"
  elif command -v explorer.exe >/dev/null 2>&1; then
    winpath=$(wslpath -w "$item_path" 2>/dev/null) || winpath="$item_path"
    explorer.exe "$winpath" >/dev/null 2>&1 &
    opener="explorer"
  else
    refuse "no opener found (code/xdg-open/explorer.exe)"
  fi
  printf 'opened %s opener=%s\n' "$item_path" "$opener"
  exit 0
fi

[ -n "$ws" ] || refuse "no workspace context (invoke from inside herdr)"

PANES_JSON=$("$H" pane list --workspace "$ws" 2>/dev/null) && [ -n "$PANES_JSON" ] ||
  refuse "herdr pane list failed for $ws"
EXISTING=$(printf '%s' "$PANES_JSON" | jq -r '.result.panes[] | select(.label == "Review Queue") | .pane_id' 2>/dev/null)

close_existing() {
  local failed="" p
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    "$H" pane close "$p" >/dev/null 2>&1 || failed="$failed $p"
  done <<EOF
$EXISTING
EOF
  [ -z "$failed" ] || refuse "failed to close$failed in $ws"
}

case "$mode" in
close)
  [ -n "$EXISTING" ] || {
    printf 'close: no review panel open in %s\n' "$ws"
    exit 0
  }
  close_existing
  printf 'closed review panel in %s\n' "$ws"
  ;;

toggle)
  if [ -n "$EXISTING" ]; then
    close_existing
    printf 'closed review panel in %s\n' "$ws"
    exit 0
  fi
  ;&
open)
  if [ -n "$EXISTING" ]; then
    printf 'open: already open (%s) in %s\n' "$(printf '%s' "$EXISTING" | tr '\n' ' ' | sed 's/ $//')" "$ws"
    exit 0
  fi
  if [ -z "$pane" ]; then
    pane=$(printf '%s' "$PANES_JSON" | jq -r '.result.panes[0].pane_id // empty' 2>/dev/null)
  fi
  [ -n "$pane" ] || refuse "no pane to attach to in $ws"
  rebuild_if_stale
  # Explicit --env, not inherited environment: QUEUE_LOG/DONE_LOG/CLEARED_LOG above were
  # computed from *this* invocation's pane context, which is what should be scoped, not
  # whatever the spawned process would otherwise pick up on its own.
  out=$("$H" plugin pane open --plugin "$PLUGIN_ID" --entrypoint sidebar \
    --placement split --target-pane "$pane" --direction right --no-focus \
    --env "REVIEW_PANEL_LOG=$QUEUE_LOG" \
    --env "REVIEW_PANEL_DONE_LOG=$DONE_LOG" \
    --env "REVIEW_PANEL_CLEARED_LOG=$CLEARED_LOG" 2>/dev/null) ||
    refuse "herdr plugin pane open failed"
  opened=$(printf '%s' "$out" | jq -r '.result.plugin_pane.pane.pane_id // empty' 2>/dev/null)
  printf 'opened review panel %s in %s\n' "${opened:-pane}" "$ws"
  ;;

*)
  refuse "unknown mode '$mode'"
  ;;
esac
