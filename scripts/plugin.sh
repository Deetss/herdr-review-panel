#!/usr/bin/env bash
# Review Queue plugin actions.
#
#   plugin.sh open       open the sidebar, no-op if one is already open
#   plugin.sh toggle     open the sidebar, or close it if one is already open
#   plugin.sh close      close every Review Queue pane in the workspace, no-op if none
#   plugin.sh clear      truncate the underlying review.log
#   plugin.sh open-item  open the file:// path Ctrl+clicked in the sidebar (link_handlers)
#   plugin.sh tools-menu open the fzf tool picker popup (see scripts/tools-menu.sh)
#
# Mirrors the memex plugin's open/close/toggle pattern: there's no separate state file, the
# workspace's live pane list (matched by label) is the source of truth for "is it open".
set -uo pipefail

mode="${1:-toggle}"
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

H="${HERDR_BIN_PATH:-herdr}"
PLUGIN_ID="${HERDR_PLUGIN_ID:-bh.review-panel}"
ws="${HERDR_WORKSPACE_ID:-}"
pane="${HERDR_PANE_ID:-}"
QUEUE_LOG="${REVIEW_PANEL_LOG:-$HOME/.claude/review.log}"

refuse() {
  printf 'review-panel: %s\n' "$1" >&2
  exit 1
}

if [ "$mode" = "clear" ]; then
  : >"$QUEUE_LOG" 2>/dev/null || refuse "cannot clear $QUEUE_LOG"
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
  # Preference order: VS Code (primary editor) -> xdg-open on the containing dir (native Linux
  # file manager, works regardless of desktop environment) -> explorer.exe (real WSL2 only).
  if command -v code >/dev/null 2>&1; then
    code -g "$item_path" >/dev/null 2>&1 &
  elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$(dirname "$item_path")" >/dev/null 2>&1 &
  elif command -v explorer.exe >/dev/null 2>&1; then
    winpath=$(wslpath -w "$item_path" 2>/dev/null) || winpath="$item_path"
    explorer.exe "$winpath" >/dev/null 2>&1 &
  else
    refuse "no opener found (code/xdg-open/explorer.exe)"
  fi
  printf 'opened %s\n' "$item_path"
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
  out=$("$H" plugin pane open --plugin "$PLUGIN_ID" --entrypoint sidebar \
    --placement split --target-pane "$pane" --direction right --no-focus 2>/dev/null) ||
    refuse "herdr plugin pane open failed"
  opened=$(printf '%s' "$out" | jq -r '.result.plugin_pane.pane.pane_id // empty' 2>/dev/null)
  printf 'opened review panel %s in %s\n' "${opened:-pane}" "$ws"
  ;;

*)
  refuse "unknown mode '$mode'"
  ;;
esac
