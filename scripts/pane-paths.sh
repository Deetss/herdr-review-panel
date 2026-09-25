# Sourced by review-notify.sh and plugin.sh: scope the review-queue log/done/cleared
# files to the herdr pane they're running in, so each pane gets its own queue instead of
# one machine-wide shared log. HERDR_PANE_ID is the same env var in both scripts' cases -
# review-notify.sh inherits it from the Stop hook's own pane, and plugin.sh either inherits
# it the same way (review-notify.sh execs it directly) or gets it injected by herdr as the
# focused pane when an action/keybind triggers `open` some other way (see herdr-plugin.toml's
# `contexts = ["pane", "workspace"]`). Falls back to the original unscoped path when there's
# no pane context at all (e.g. run outside herdr), so this is a no-op for anyone not using it.
#
# $1: "" for the main log, "-done" or "-cleared" for the companion files.
pane_scoped_path() {
  local pane_id="${HERDR_PANE_ID:-}"
  if [ -n "$pane_id" ]; then
    local dir="$HOME/.claude/review"
    mkdir -p "$dir" 2>/dev/null
    printf '%s/%s%s.log' "$dir" "${pane_id//:/_}" "$1"
  else
    printf '%s/.claude/review%s.log' "$HOME" "$1"
  fi
}
