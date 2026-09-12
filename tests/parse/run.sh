#!/usr/bin/env bash
# Golden tests for scripts/review-parse.jq.
# The parser is a pure stdin->stdout function, so a case is just a hook payload
# plus the TSV it must produce. Run from anywhere: ./tests/parse/run.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
parser="$here/../../scripts/review-parse.jq"
fail=0
pass=0

for f in "$here"/cases/*.json; do
  name=$(basename "$f" .json)
  exp_file="${f%.json}.expected"
  if [ ! -f "$exp_file" ]; then
    printf 'MISSING GOLDEN %s\n' "$name"
    fail=1
    continue
  fi
  got=$(jq -r -f "$parser" <"$f" 2>&1)
  exp=$(cat "$exp_file")
  if [ "$got" = "$exp" ]; then
    pass=$((pass + 1))
  else
    printf 'FAIL %s\n' "$name"
    diff <(printf '%s\n' "$exp") <(printf '%s\n' "$got") | sed 's/^/    /'
    fail=1
  fi
done

printf '%d passed\n' "$pass"
[ "$fail" -eq 0 ] || printf 'FAILURES\n'
exit "$fail"
