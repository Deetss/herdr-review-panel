#!/bin/bash
# Claude Code Stop/SubagentStop hook for the Review Queue.
#
# Scans the finished reply for <user_review> and <user_command> tags, appends each to
# the queue log, toasts, and pops the panel. All parsing lives in review-parse.jq; this
# script is side effects only. That split is deliberate: the parser is a pure
# stdin->stdout function with golden tests (tests/parse/), and the quoting hazards that
# used to live here (grep -oP per tag, then sed to strip the tag) are gone with it.

parser="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/review-parse.jq"
LOG="${REVIEW_PANEL_LOG:-$HOME/.claude/review.log}"
DEBUG="${REVIEW_NOTIFY_DEBUG:-0}"
DEBUG_LOG="${REVIEW_NOTIFY_DEBUG_LOG:-$HOME/.claude/review-debug.log}"
PLUGIN_ID="${HERDR_PLUGIN_ID:-deetss.review-panel}"

# Off costs one string compare - no forks, no date, no stat.
dbg() {
  [ "$DEBUG" = "0" ] && return 0
  printf '%s\n' "$*" >>"$DEBUG_LOG"
}

# Single exit point, so every early return still leaves a trace in the debug log.
# Diagnosing "nothing appeared" used to be impossible because the old hook returned
# from three different places without recording that it had run at all.
finish() {
  dbg "result logged=${logged:-0} warned=${warned:-0} dropped=${dropped:-0} reason=${1:-ok}"
  exit 0
}

input=$(cat)

if [ "$DEBUG" != "0" ]; then
  if [ ! -e "$DEBUG_LOG" ]; then
    : >"$DEBUG_LOG" && chmod 600 "$DEBUG_LOG"
  elif [ "$(stat -c %s "$DEBUG_LOG" 2>/dev/null || echo 0)" -gt 5242880 ]; then
    mv -f "$DEBUG_LOG" "$DEBUG_LOG.1" && : >"$DEBUG_LOG" && chmod 600 "$DEBUG_LOG"
  fi
  dbg "=== $(date '+%Y-%m-%d %H:%M:%S') pid=$$ cwd=$PWD"
  if [ "$DEBUG" = "2" ]; then
    dbg "<<<BEGIN-MSG"
    jq -r '.last_assistant_message // ""' <<<"$input" >>"$DEBUG_LOG"
    dbg "<<<END-MSG"
  fi
fi

records=$(jq -r -f "$parser" <<<"$input" 2>&1)
if [ $? -ne 0 ]; then
  dbg "parser failed: $records"
  finish parser-error
fi

session_id="unknown"
cwd="$PWD"
logged=0
warned=0
dropped=0
first=""
first_kind=""
lines=()

# Values arrive already @tsv-escaped, so no field can contain a raw tab or newline and
# splitting on tab is total. The "-" sentinel stands in for an empty optional field.
undash() { [ "$1" = "-" ] && printf '' || printf '%s' "$1"; }

# Escape the two fields the hook contributes itself. Parameter expansion only, no forks.
esc() {
  local s=$1
  s=${s//\\/\\\\}
  s=${s//$'\t'/\\t}
  s=${s//$'\r'/\\r}
  s=${s//$'\n'/\\n}
  printf '%s' "$s"
}

emit() { # kind step warn item(already escaped)
  local line
  printf -v line '%s\tsession=%s\trepo=%s\tcwd=%s\tkind=%s' \
    "$ts" "$session_id" "$repo_esc" "$cwd_esc" "$1"
  [ -n "$2" ] && printf -v line '%s\tstep=%s' "$line" "$2"
  [ -n "$3" ] && printf -v line '%s\twarn=%s' "$line" "$3"
  printf -v line '%s\titem=%s' "$line" "$4"
  lines+=("$line")
  logged=$((logged + 1))
  [ -n "$3" ] && warned=$((warned + 1))
  if [ -z "$first" ]; then
    first="$4"
    first_kind="$1"
  fi
}

# The ctx record always comes first, so read it before anything needs $cwd.
while IFS=$'\t' read -r rtype f1 f2 f3 f4; do
  case "$rtype" in
    ctx)
      session_id=$(undash "$f1")
      [ -n "$(undash "$f2")" ] && cwd=$(undash "$f2")
      repo_name=$(basename "$cwd")
      ts=$(date '+%Y-%m-%d %H:%M:%S')
      repo_esc=$(esc "$repo_name")
      cwd_esc=$(esc "$cwd")
      ;;
    item)
      step=$(undash "$f2")
      warn=$(undash "$f3")
      if [ "$f1" = "review" ]; then
        # A review target that resolves on disk logs clean; a URL is a legitimate
        # target this convention never anticipated; anything else is logged with
        # warn=missing rather than vanishing the way it used to.
        target=$f4
        case "$target" in
          "~"|"~/"*) expanded="$HOME${target#\~}" ;;
          /*)        expanded="$target" ;;
          *)         expanded="$cwd/$target" ;;
        esac
        if [ -e "$expanded" ]; then
          :
        elif [[ "$target" =~ ^(https?|file|ssh):// ]]; then
          :
        else
          warn="${warn:+$warn,}missing"
        fi
      fi
      emit "$f1" "$step" "$warn" "$f4"
      ;;
    near)
      # Written as kind=command so it lands on the panel arm that already renders the
      # warning glyph, rather than needing a new Row variant for something this rare.
      preview=$(undash "$f3")
      [ -z "$preview" ] && preview="<$f2 $f1 tag>"
      emit "command" "" "$f2" "$preview"
      ;;
    drop)
      dropped=$((dropped + 1))
      dbg "  drop  kind=$f1 reason=$f2 body=$f3"
      ;;
    stat)
      dbg "  $f1 $f2 $f3 $f4"
      ;;
  esac
done <<<"$records"

if [ "$DEBUG" != "0" ]; then
  for l in "${lines[@]}"; do dbg "  item  $l"; done
fi

[ "${#lines[@]}" -eq 0 ] && finish no-items

# One append for the whole invocation. Each line is capped well under PIPE_BUF by the
# parser's body limit, so concurrent Stop hooks from parallel sessions cannot interleave
# mid-line.
printf '%s\n' "${lines[@]}" >>"$LOG"

tmux_loc=$(tmux display-message -p '#S:#I' 2>/dev/null || echo "")
loc_label="$repo_name"
[ -n "$tmux_loc" ] && loc_label="$loc_label (tmux $tmux_loc)"

# The toast shows the item as logged, so a multiline command appears on one line.
first_flat=${first//\\n/ }
if [ "$logged" -eq 1 ]; then
  if [ "$first_kind" = "command" ]; then
    msg="Command to run: $first_flat at $loc_label"
  else
    msg="Review needed: $first_flat at $loc_label"
  fi
else
  msg="Review needed ($logged items, first: $first_flat) at $loc_label"
fi

tty_path=$(tty 2>/dev/null || echo "")
if [ -n "$tty_path" ] && [ -w "$tty_path" ]; then
  printf '\033]9;%s\007' "$msg" >"$tty_path"
fi
command -v herdr >/dev/null 2>&1 && herdr notification show "Review needed" --body "$msg" --sound request >/dev/null 2>&1

# `herdr plugin action invoke` always targets the globally-focused pane, not this hook's own
# pane, so it can pop the panel open in whichever tab happens to have UI focus at the moment.
# Run the plugin script directly instead: it reads HERDR_WORKSPACE_ID/HERDR_PANE_ID from this
# hook's own inherited env, which are this session's real pane, so it opens in the right tab.
# The plugin's install location varies per machine/install method, so resolve it from herdr's
# own registry rather than hardcoding a path.
if command -v herdr >/dev/null 2>&1; then
  plugin_root=$(herdr plugin list --plugin "$PLUGIN_ID" --json 2>/dev/null |
    jq -r '.result.plugins[0].plugin_root // empty')
  if [ -z "$plugin_root" ]; then
    # herdr resolves plugin_id from the live manifest, so this only goes empty if the
    # plugin was uninstalled or the manifest id changed. Worth a debug line either way:
    # the old code swallowed it and the panel just silently never opened.
    dbg "  panel lookup failed for plugin id '$PLUGIN_ID' - panel not opened"
  else
    plugin_script="$plugin_root/scripts/plugin.sh"
    [ -x "$plugin_script" ] && bash "$plugin_script" open >/dev/null 2>&1
  fi
fi

finish ok
