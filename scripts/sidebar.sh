#!/usr/bin/env bash
# Review Queue panel body: tails ~/.claude/review.log (written by
# ~/.claude/hooks/review-notify.sh whenever a Claude Code reply contains a
# <user_review>path</user_review> tag) and renders each entry as timestamp / repo / path.
set -uo pipefail

LOG="${REVIEW_PANEL_LOG:-$HOME/.claude/review.log}"
mkdir -p "$(dirname "$LOG")" 2>/dev/null
touch "$LOG" 2>/dev/null

BOLD=$'\033[1m'
DIM=$'\033[2m'
CYAN=$'\033[1;36m'
YEL=$'\033[33m'
RESET=$'\033[0m'

# Herdr's link_handlers intercept Ctrl+click on actual terminal hyperlinks, not just
# URL-shaped plain text - so targets must be real OSC 8 hyperlinks. The visible label can
# differ from the link target; herdr matches link_handler patterns against the target.
# Ghostty has no config for hover-only underline on OSC 8 links, so underline the label
# unconditionally as the "this is clickable" affordance instead.
link() {
  printf '\033]8;;%s\033\\\033[4m%s\033[24m\033]8;;\033\\' "$1" "$2"
}

clear
printf '%sReview Queue%s\n' "$BOLD" "$RESET"
printf '%s%s%s\n' "$DIM" "$LOG" "$RESET"
printf 'Ctrl+click a file link to open it, or %s to close this panel.\n\n' \
  "$(link 'herdr-queue://close' "${YEL}[x] close${RESET}")"

# Items written by the same review-notify.sh invocation share one timestamp (computed once
# per hook run), so group consecutive lines with the same timestamp+repo under one header
# instead of repeating it per item.
last_key=""
render_line() {
  local line="$1" ts repo cwd item abspath key
  ts=$(cut -d' ' -f1-2 <<<"$line")
  repo=$(sed -n 's/.*repo=\([^ ]*\).*/\1/p' <<<"$line")
  cwd=$(sed -n 's/.*cwd=\([^ ]*\).*/\1/p' <<<"$line")
  item=$(sed -n 's/.*item=\(.*\)$/\1/p' <<<"$line")
  [ -n "$item" ] || return
  key="$ts|$repo"
  if [ "$key" != "$last_key" ]; then
    printf '\n%s%s%s  %s%s%s\n' "$DIM" "$ts" "$RESET" "$CYAN" "$repo" "$RESET"
    last_key="$key"
  fi
  # Older log lines predate the cwd= field; fall back to showing the raw item with no link.
  case "$item" in
    /*) abspath="$item" ;;
    *) [ -n "$cwd" ] && abspath="$cwd/$item" || abspath="" ;;
  esac
  if [ -n "$abspath" ]; then
    printf '  %s\n' "$(link "file://$abspath" "${YEL}${abspath}${RESET}")"
  else
    printf '  %s%s%s\n' "$YEL" "$item" "$RESET"
  fi
}

# Only backfill items from the last REVIEW_PANEL_WINDOW_MINUTES (default 10): the log is
# shared across every session on the machine, so an unbounded backfill shows old, already
#-handled items from unrelated work alongside whatever just triggered this panel to open.
# Timestamps are "YYYY-MM-DD HH:MM:SS", so a plain string compare sorts chronologically.
window_min="${REVIEW_PANEL_WINDOW_MINUTES:-10}"
cutoff=$(date -d "-${window_min} minutes" '+%Y-%m-%d %H:%M:%S' 2>/dev/null || date '+%Y-%m-%d %H:%M:%S')

# One combined stream (backfill, then follow) so last_key state carries across both phases
# instead of resetting between two separate pipelines.
{
  awk -v cutoff="$cutoff" '{ts=$1" "$2; if (ts >= cutoff) print}' "$LOG" 2>/dev/null
  tail -n 0 -F "$LOG" 2>/dev/null
} | while IFS= read -r line; do
  render_line "$line"
done
