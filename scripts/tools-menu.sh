#!/usr/bin/env bash
# Tools menu: fzf picker for the split-pane tool plugins, so there's one keybind instead of
# one per plugin. Add a line to TOOLS to add a plugin here.
set -uo pipefail

export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"
H="${HERDR_BIN_PATH:-herdr}"

# label|plugin_id|action_id
TOOLS=(
  "Review Queue|bh.review-panel|toggle"
  "Memex sidebar|nicosuave.memex|toggle"
  "Annotate: manage|annotate|manage"
  "Annotate: open here|annotate|open"
  "Hunk: review changes|jhochenbaum.hunkdiff|review"
)

choice=$(printf '%s\n' "${TOOLS[@]}" | cut -d'|' -f1 | fzf --prompt='tool > ' --height=100% --border --reverse) || exit 0
[ -n "$choice" ] || exit 0

for opt in "${TOOLS[@]}"; do
  label="${opt%%|*}"
  rest="${opt#*|}"
  plugin="${rest%%|*}"
  action="${rest#*|}"
  if [ "$label" = "$choice" ]; then
    "$H" plugin action invoke "$action" --plugin "$plugin" >/dev/null 2>&1
    exit 0
  fi
done
