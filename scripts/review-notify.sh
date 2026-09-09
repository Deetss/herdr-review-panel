#!/bin/bash
input=$(cat)
session_id=$(jq -r '.session_id' <<<"$input")
cwd=$(jq -r '.cwd' <<<"$input")

# Both Stop and SubagentStop payloads carry the final assistant text pre-extracted as
# last_assistant_message - use it directly instead of re-reading transcript_path's jsonl file
# ourselves. An earlier version tac/grep'd the transcript for Stop events specifically, which
# raced the file still being flushed to disk and intermittently picked up the *previous* turn
# instead of the one that just finished ("behind by one message"). last_assistant_message comes
# straight from the hook payload with no such race, for every profile/session uniformly.
last_text=$(jq -r '.last_assistant_message // empty' <<<"$input")
[ -z "$last_text" ] && exit 0

review_matches=$(grep -oP '(?<=<user_review>).*?(?=</user_review>)' <<<"$last_text")
# Full tag (with any attributes) so the step="..." label, if present, can be pulled out
# alongside the command text - grep -oP still emits one match per line even across a
# multi-line reply, since each individual tag pair is expected to stay on one logical line.
command_tags=$(grep -oP '<user_command\b[^>]*>.*?</user_command>' <<<"$last_text")
[ -z "$review_matches" ] && [ -z "$command_tags" ] && exit 0

repo_name=$(basename "$cwd")
ts=$(date '+%Y-%m-%d %H:%M:%S')

count=0
first=""
first_kind=""

log_item() {
  local kind="$1" value="$2" step="$3" warn="$4" step_field="" warn_field=""
  [ -n "$step" ] && step_field="step=$step "
  [ -n "$warn" ] && warn_field="warn=$warn "
  echo "$ts session=$session_id repo=$repo_name cwd=$cwd kind=$kind ${step_field}${warn_field}item=$value" >> ~/.claude/review.log
  count=$((count + 1))
  if [ -z "$first" ]; then
    first="$value"
    first_kind="$kind"
  fi
}

# Heuristic-only: flags text that reads like a paraphrased task/GUI-action description
# rather than an actual shell command (e.g. "redeploy the app in Dokploy's UI"), so it can
# be marked in the panel instead of trusted silently. Can false-positive on a real command
# that happens to contain two of these words - that's fine, it's a visual nudge, not a filter.
is_prose_command() {
  local cmd="$1"
  local hits
  # -c counts matching *lines*, not occurrences - useless on a single-line string, hence -o | wc -l.
  hits=$(grep -oiwE 'the|after|above|before|once|then|please|kindly' <<<"$cmd" | wc -l)
  [ "$hits" -ge 2 ]
}

# Guard against false positives when a reply just talks about the <user_review> convention
# itself (e.g. quoting the CLAUDE.md example literally) instead of naming a real file. Only
# paths that actually exist on disk count as a genuine review request. <user_command> has no
# equivalent hard check (a shell command isn't a thing you can stat) - it's logged as-is, the
# same trust boundary as any other command Claude proposes running, aside from the prose
# heuristic below that only adds a visual warning rather than filtering anything out.
while IFS= read -r item; do
  [ -z "$item" ] && continue
  expanded="$item"
  case "$expanded" in
    "~"|"~/"*) expanded="$HOME${expanded#\~}" ;;
  esac
  case "$expanded" in
    /*) : ;;
    *) expanded="$cwd/$expanded" ;;
  esac
  [ -e "$expanded" ] || continue
  log_item "review" "$item"
done <<<"$review_matches"

while IFS= read -r tag; do
  [ -z "$tag" ] && continue
  step=$(grep -oP '(?<=step=")[^"]*' <<<"$tag")
  cmd=$(sed -E 's/^<user_command[^>]*>//; s/<\/user_command>$//' <<<"$tag")
  [ -z "$cmd" ] && continue
  warn=""
  is_prose_command "$cmd" && warn="prose"
  log_item "command" "$cmd" "$step" "$warn"
done <<<"$command_tags"

[ "$count" -eq 0 ] && exit 0

tmux_loc=$(tmux display-message -p '#S:#I' 2>/dev/null || echo "")
loc_label="$repo_name"
[ -n "$tmux_loc" ] && loc_label="$loc_label (tmux $tmux_loc)"

if [ "$count" -eq 1 ]; then
  if [ "$first_kind" = "command" ]; then
    msg="Command to run: $first at $loc_label"
  else
    msg="Review needed: $first at $loc_label"
  fi
else
  msg="Review needed ($count items, first: $first) at $loc_label"
fi

tty_path=$(tty 2>/dev/null || echo "")
if [ -n "$tty_path" ] && [ -w "$tty_path" ]; then
  printf '\033]9;%s\007' "$msg" > "$tty_path"
fi
command -v herdr >/dev/null 2>&1 && herdr notification show "Review needed" --body "$msg" --sound request >/dev/null 2>&1
# `herdr plugin action invoke` always targets the globally-focused pane, not this hook's own
# pane, so it can pop the panel open in whichever tab happens to have UI focus at the moment.
# Run the plugin script directly instead: it reads HERDR_WORKSPACE_ID/HERDR_PANE_ID from this
# hook's own inherited env, which are this session's real pane, so it opens in the right tab.
# The plugin's install location varies per machine/install method, so resolve it from herdr's
# own registry rather than hardcoding a path.
if command -v herdr >/dev/null 2>&1; then
  plugin_root=$(herdr plugin list --plugin deetss.review-panel --json 2>/dev/null |
    jq -r '.result.plugins[0].plugin_root // empty')
  plugin_script="${plugin_root:+$plugin_root/scripts/plugin.sh}"
  [ -n "$plugin_script" ] && [ -x "$plugin_script" ] && bash "$plugin_script" open >/dev/null 2>&1
fi
exit 0
