#!/bin/bash
# Claude Code Stop/SubagentStop hook for the Review Queue.
#
# Scans the finished reply for <user_review> and <user_command> tags, appends each to
# the queue log, toasts, and pops the panel. All parsing lives in review-parse.jq; this
# script is side effects only. That split is deliberate: the parser is a pure
# stdin->stdout function with golden tests (tests/parse/), and the quoting hazards that
# used to live here (grep -oP per tag, then sed to strip the tag) are gone with it.

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
parser="$script_dir/review-parse.jq"
# shellcheck source=pane-paths.sh
source "$script_dir/pane-paths.sh"
LOG="${REVIEW_PANEL_LOG:-$(pane_scoped_path "")}"
DEBUG="${REVIEW_NOTIFY_DEBUG:-0}"
DEBUG_LOG="${REVIEW_NOTIFY_DEBUG_LOG:-$HOME/.claude/review-debug.log}"
# Never bare `orca`: outside an Orca terminal it resolves to the GNOME screen reader.
ORCA="${ORCA_CLI_COMMAND:-${ORCA_BIN:-orca-ide}}"

# Off costs one string compare - no forks, no date, no stat.
dbg() {
  [ "$DEBUG" = "0" ] && return 0
  printf '%s\n' "$*" >>"$DEBUG_LOG"
}

# Portable timeout (macOS lacks GNU timeout): poll the child and SIGKILL it after SECONDS.
# Ported from structupath.browser's lib.sh - same shape, same reasoning. Every call below
# runs after the log write is already durable (see the comment above that printf), so a
# kill here only costs a toast or panel-pop side effect, never a queued item.
with_timeout() {
  local secs="$1"
  shift
  "$@" &
  local pid=$!
  local i=0
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$i" -ge "$((secs * 10))" ]; then
      kill -9 "$pid" 2>/dev/null
      break
    fi
    sleep 0.1
    i=$((i + 1))
  done
  wait "$pid" 2>/dev/null
  return $?
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

emit() { # kind step warn item(already escaped) [terminal]
  local line
  printf -v line '%s\tsession=%s\trepo=%s\tcwd=%s\tkind=%s' \
    "$ts" "$session_id" "$repo_esc" "$cwd_esc" "$1"
  [ -n "$2" ] && printf -v line '%s\tstep=%s' "$line" "$2"
  [ -n "$3" ] && printf -v line '%s\twarn=%s' "$line" "$3"
  [ -n "${5:-}" ] && printf -v line '%s\tterminal=%s' "$line" "$5"
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
while IFS=$'\t' read -r rtype f1 f2 f3 f4 f5; do
  case "$rtype" in
    ctx)
      session_id=$(undash "$f1")
      [ -n "$(undash "$f2")" ] && cwd=$(undash "$f2")
      repo_name=$(basename "$cwd")
      ts=$(date '+%Y-%m-%d %H:%M:%S')
      repo_esc=$(esc "$repo_name")
      cwd_esc=$(esc "$cwd")
      # Auto-detected once per invocation (every item in one Stop hook shares the same
      # terminal), and reused below for the toast label too - one tmux call, not two.
      # Guarded like the calls below it even though this one runs before the log write,
      # since it's the one call that could otherwise stall commands from ever reaching disk.
      auto_terminal=$(with_timeout 1 tmux display-message -p '#S:#I' 2>/dev/null || echo "")
      ;;
    item)
      step=$(undash "$f2")
      warn=$(undash "$f3")
      # An explicit terminal="..." attribute overrides the auto-detected tmux pane - the
      # only way to express a target tmux can't see, like a remote SSH host.
      attr_terminal=$(undash "$f5")
      term_val="${attr_terminal:-$auto_terminal}"
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
      emit "$f1" "$step" "$warn" "$f4" "$term_val"
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

loc_label="$repo_name"
[ -n "$auto_terminal" ] && loc_label="$loc_label (tmux $auto_terminal)"

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
if command -v notify-send >/dev/null 2>&1; then
  with_timeout 2 notify-send "Review needed" "$msg" >/dev/null 2>&1
fi

# plugin.sh lives next to this script and opens the sidebar as an Orca terminal split in
# this hook's own tab. It runs after the log write above is already durable, so the worst
# case of a hang is a lost panel-pop with a recorded reason, never an externally-killed hook
# with no trace (see with_timeout's own comment). Three orca CLI calls fit comfortably inside
# Claude Code's external 10s hook cap.
if [ -n "${ORCA_TERMINAL_HANDLE:-}" ] && command -v "$ORCA" >/dev/null 2>&1; then
  open_out=$(with_timeout 6 bash "$script_dir/plugin.sh" open 2>&1 </dev/null) ||
    dbg "  orca panel open failed or timed out: ${open_out//$'\n'/ | }"
fi

finish ok
