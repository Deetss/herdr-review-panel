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

clear
printf '%sReview Queue%s\n' "$BOLD" "$RESET"
printf '%s%s%s\n\n' "$DIM" "$LOG" "$RESET"

render_line() {
  local line="$1" ts session repo item
  ts=$(cut -d' ' -f1-2 <<<"$line")
  repo=$(sed -n 's/.*repo=\([^ ]*\).*/\1/p' <<<"$line")
  item=$(sed -n 's/.*item=\(.*\)$/\1/p' <<<"$line")
  [ -n "$item" ] || return
  printf '%s%s%s  %s%s%s\n  %s%s%s\n\n' "$DIM" "$ts" "$RESET" "$CYAN" "$repo" "$RESET" "$YEL" "$item" "$RESET"
}

tail -n 20 -F "$LOG" 2>/dev/null | while IFS= read -r line; do
  render_line "$line"
done
