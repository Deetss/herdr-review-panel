# Sourced by review-notify.sh and plugin.sh: scope the review-queue log/done/cleared
# files to the Orca tab they're running in, so each tab gets its own queue instead of one
# machine-wide shared log. review-notify.sh inherits ORCA_TAB_ID from the Stop hook's own
# pane, and plugin.sh inherits it the same way (review-notify.sh execs it directly).
#
# The scope is the tab, not the terminal: the sidebar is a split in the same tab, so the tab
# is what a flag and the panel that shows it have in common, and a tab id survives Orca's
# session restore where a terminal handle may not. Falls back to the unscoped path outside
# Orca, so this is a no-op there.
#
# $1: "" for the main log, "-done" or "-cleared" for the companion files.
pane_scoped_path() {
  local pane_id="${ORCA_TAB_ID:-}"
  if [ -n "$pane_id" ]; then
    local dir="$HOME/.claude/review"
    mkdir -p "$dir" 2>/dev/null
    printf '%s/%s%s.log' "$dir" "${pane_id//:/_}" "$1"
  else
    printf '%s/.claude/review%s.log' "$HOME" "$1"
  fi
}
