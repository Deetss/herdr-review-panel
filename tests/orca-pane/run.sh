#!/usr/bin/env bash
# Tests for scripts/orca-pane against a fake orca-ide that records calls.
# Run from anywhere: ./tests/orca-pane/run.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
script="$here/../../scripts/orca-pane"
fail=0
pass=0

check() { # name expected actual
  if [ "$2" = "$3" ]; then
    pass=$((pass + 1))
  else
    printf 'FAIL %s\n    want: %s\n    got:  %s\n' "$1" "$2" "$3"
    fail=1
  fi
}

setup() {
  tmp=$(mktemp -d)
  mkdir "$tmp/bin"
  : >"$tmp/calls"
  : >"$tmp/live"
  echo 0 >"$tmp/n"
  cat >"$tmp/bin/orca-ide" <<'FAKE'
#!/usr/bin/env bash
echo "$*" >>"$FAKE_DIR/calls"
case "$1 $2" in
"terminal split")
  n=$(($(cat "$FAKE_DIR/n") + 1)); echo $n >"$FAKE_DIR/n"
  echo "w$n" >>"$FAKE_DIR/live"
  printf '{"ok":true,"result":{"split":{"handle":"w%s"}}}\n' "$n" ;;
"terminal list")
  jq -Rn '{ok:true,result:{terminals:[inputs|{handle:.}]}}' <"$FAKE_DIR/live" ;;
esac
FAKE
  chmod +x "$tmp/bin/orca-ide"
  export FAKE_DIR=$tmp PATH="$tmp/bin:$PATH" XDG_STATE_HOME="$tmp/state"
  export ORCA_TERMINAL_HANDLE=main ORCA_TAB_ID=tab1 ORCA_CLI_COMMAND=orca-ide ORCA_PANE_SETTLE=0
  unset ORCA_PANE_MAX
}

splits() { grep '^terminal split' "$tmp/calls" | sed 's/ --command.*//'; }
count() { grep -c -- "$1" "$tmp/calls"; }
watch() { "$script" watch --label "$1" -- tail -f "$1.log" >/dev/null; }

# first watcher splits main vertically
setup
watch a
check "first watcher" "terminal split --terminal main --direction vertical" "$(splits)"

# second splits the newest watcher horizontally
watch b
check "second watcher" "$(printf 'terminal split --terminal main --direction vertical\nterminal split --terminal w1 --direction horizontal')" "$(splits)"

# the command self-closes the pane
check "command ends with exit" 1 "$(count '--command tail -f a.log; exit --json')"

# fourth reuses the oldest instead of splitting
watch c
watch d
check "no fourth split" 3 "$(count '^terminal split')"
check "reuse interrupts oldest" 1 "$(count '^terminal send --terminal w1 --interrupt')"
check "reuse sends command" 1 "$(count '^terminal send --terminal w1 --text tail -f d.log; exit --enter')"
check "list rotates labels" "b c d" "$("$script" list | cut -f1 | tr '\n' ' ' | sed 's/ $//')"

# dead handles get pruned
setup
watch a
watch b
grep -vx w1 "$tmp/live" >"$tmp/live.new"
mv "$tmp/live.new" "$tmp/live"
check "pruned list" "b" "$("$script" list | cut -f1)"
watch c
check "split targets surviving watcher" "terminal split --terminal w2 --direction horizontal" "$(splits | tail -1)"

# close by label
setup
watch a
watch b
"$script" close a
check "close by label" 1 "$(count '^terminal close --terminal w1')"
check "close leaves others" "b" "$("$script" list | cut -f1)"

# outside Orca does nothing
setup
unset ORCA_TERMINAL_HANDLE
"$script" watch --label a -- tail -f a.log
check "unset handle exits 0" 0 $?
check "unset handle makes no calls" 0 "$(wc -l <"$tmp/calls" | awk '{print $1 + 0}')"

printf '%d passed\n' "$pass"
exit "$fail"
