#!/usr/bin/env bash
# Run one queued <user_command> by id, after showing it and asking.
#
# This exists because Collie's launcher rows are a bare allowlist: `[[launchers]]` accepts only
# command/label/cwd, with no confirm option, and POST /api/launch matches the command string
# exactly. Putting agent-proposed commands straight in that file would make anything the agent
# suggested one tap from running over Tailscale, with no reading step - which is the opposite of
# what the <user_command> convention is for.
#
# So the allowlist holds `review-run <id>` and nothing else. The id resolves against review.log
# locally, the command is printed, and it runs only after an explicit y. The allowlist stays
# bounded and predictable no matter what the agent writes.
set -uo pipefail

LOG="${REVIEW_PANEL_LOG:-$HOME/.claude/review.log}"
DONE_LOG="${REVIEW_PANEL_DONE_LOG:-$HOME/.claude/review-done.log}"

id="${1:-}"
if [ -z "$id" ]; then
  printf 'usage: review-run <id>\n' >&2
  exit 2
fi

# Left-to-right scan. Replacing \\n before \\\\ (or the reverse) mangles a command containing a
# literal backslash-n, which is exactly what `printf 'a\nb'` is - so this cannot be a chain of
# substitutions.
unescape() {
  local s=$1 out="" i=0 c n
  while [ "$i" -lt "${#s}" ]; do
    c=${s:$i:1}
    if [ "$c" != "\\" ]; then
      out+=$c
      i=$((i + 1))
      continue
    fi
    n=${s:$((i + 1)):1}
    case "$n" in
      n) out+=$'\n' ;;
      t) out+=$'\t' ;;
      r) out+=$'\r' ;;
      \\) out+='\' ;;
      "") out+='\' ;;
      *) out+="\\$n" ;;
    esac
    i=$((i + 2))
  done
  printf '%s' "$out"
}

field() { # line key -> value (exact match on a whole tab-delimited field)
  local line=$1 key=$2 f
  while IFS= read -r f; do
    case "$f" in
      "$key="*) printf '%s' "${f#"$key"=}"; return 0 ;;
    esac
  done < <(printf '%s\n' "$line" | tr '\t' '\n')
  return 1
}

found=""
while IFS= read -r line; do
  case "$line" in
    *"	"*) : ;;      # v2 lines only; legacy space-delimited ones carry no stable id
    *) continue ;;
  esac
  ts=${line%%	*}
  raw_item=$(field "$line" item) || continue
  this_id=$(printf '%s\t%s' "$ts" "$raw_item" | sha256sum | cut -c1-8)
  [ "$this_id" = "$id" ] && found=$line
done <"$LOG"

if [ -z "$found" ]; then
  printf 'review-run: no queued item with id %s\n' "$id" >&2
  printf 'The queue may have been cleared since this row was generated.\n' >&2
  read -r -p "Press Enter to close. " _ </dev/tty
  exit 1
fi

kind=$(field "$found" kind) || kind=review
if [ "$kind" != "command" ]; then
  printf 'review-run: %s is not a command (kind=%s)\n' "$id" "$kind" >&2
  read -r -p "Press Enter to close. " _ </dev/tty
  exit 1
fi

raw_item=$(field "$found" item)
cmd=$(unescape "$raw_item")
step=$(field "$found" step) || step=""
warn=$(field "$found" warn) || warn=""
cwd=$(unescape "$(field "$found" cwd || printf '%s' "$HOME")")
session=$(field "$found" session) || session=""
ts=${found%%	*}

printf '\n'
[ -n "$step" ] && printf '  step %s\n' "$step"
[ -n "$warn" ] && printf '  flagged: %s\n' "$warn"
printf '  in %s\n\n' "$cwd"
printf '%s\n\n' "$cmd"

if [ "$warn" = "prose" ] || [ "${warn#*prose}" != "$warn" ]; then
  printf 'This was flagged as reading like a description rather than a command.\n\n'
fi

read -r -p "Run this? [y/N] " reply </dev/tty
case "$reply" in
  y | Y | yes | YES) ;;
  *)
    printf 'Not run.\n'
    exit 0
    ;;
esac

printf '\n'
cd "$cwd" 2>/dev/null || printf 'review-run: cannot cd to %s, running in %s\n' "$cwd" "$PWD"
bash -c "$cmd"
status=$?
printf '\n[exit %d]\n' "$status"

# Close the loop: the panel reads this file, so a command run from the phone shows as done
# on the desktop without having to check it off twice. Same key shape append_row builds.
if [ "$status" -eq 0 ] && [ -n "$session" ]; then
  printf '%s|%s|%s\n' "$ts" "$session" "$raw_item" >>"$DONE_LOG"
fi

read -r -p "Press Enter to close. " _ </dev/tty
