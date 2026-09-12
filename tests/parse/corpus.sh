#!/usr/bin/env bash
# Replays every real Claude Code reply that ever contained a review tag through the
# parser, and compares the result against what the old grep-based hook would have caught.
#
# This is the regression net that actually proves the recovered misses: the golden cases
# in cases/ are hand-written and can only test shapes someone thought of.
#
# The harvested corpus is NOT committed. It is built from ~/.claude/projects transcripts,
# which contain real session content - hostnames, paths, possibly customer data. It lands
# in tests/parse/.corpus/ which .gitignore excludes. Regenerate it, never commit it.
set -u

here=$(cd "$(dirname "$0")" && pwd)
parser="$here/../../scripts/review-parse.jq"
out="$here/.corpus"
projects="${CLAUDE_PROJECTS_DIR:-$HOME/.claude/projects}"

mkdir -p "$out"

if [ ! -d "$projects" ]; then
  echo "SKIP: no transcript directory at $projects"
  exit 0
fi

echo "harvesting from $projects ..."
# Serial on purpose: parallel writers interleave their output mid-line and corrupt the JSONL.
find "$projects" -name '*.jsonl' -print0 2>/dev/null |
  xargs -0 -n 20 jq -c '
    select(.message.role == "assistant")
    | .message.content[]?
    | select(.type == "text")
    | .text
    | select(test("<user_(command|review)"))
    | {last_assistant_message: .}
  ' 2>/dev/null >"$out/corpus.jsonl"

payloads=$(wc -l <"$out/corpus.jsonl")
if [ "$payloads" -eq 0 ]; then
  echo "SKIP: no tagged replies found in $projects"
  exit 0
fi

if ! jq -r -f "$parser" <"$out/corpus.jsonl" >"$out/corpus.tsv" 2>"$out/corpus.err"; then
  echo "FAIL: parser errored on the corpus"
  head -5 "$out/corpus.err"
  exit 1
fi

python3 - "$out" <<'PY'
import json, re, sys, collections
out = sys.argv[1]

# The old hook's two patterns. grep -oP is line-oriented, so `.` never crossed a newline.
OLD_CMD = re.compile(r'<user_command\b[^>]*>(.*?)</user_command>')
OLD_REV = re.compile(r'(?<=<user_review>)(.*?)(?=</user_review>)')

def unesc(s):
    o, it = [], iter(s)
    for c in it:
        if c != '\\':
            o.append(c); continue
        n = next(it, '\\')
        o.append({'n': '\n', 't': '\t', 'r': '\r', '\\': '\\'}.get(n, '\\' + n))
    return ''.join(o)

old_cmd, old_rev = set(), set()
replies = 0
for line in open(f'{out}/corpus.jsonl'):
    replies += 1
    msg = json.loads(line)['last_assistant_message']
    old_cmd.update(m.strip() for m in OLD_CMD.findall(msg))
    old_rev.update(m.strip() for m in OLD_REV.findall(msg))

new_cmd, new_rev, dropped = set(), set(), {}
counts = collections.Counter()
warns = collections.Counter()
multiline = 0
for line in open(f'{out}/corpus.tsv'):
    f = line.rstrip('\n').split('\t')
    counts[f[0]] += 1
    if f[0] == 'item':
        body = unesc(f[4])
        (new_cmd if f[1] == 'command' else new_rev).add(body)
        if '\n' in body:
            multiline += 1
        if f[3] != '-':
            for w in f[3].split(','):
                warns[w] += 1
    elif f[0] == 'drop':
        dropped.setdefault(unesc(f[3]), f[2])

old_total, new_total = len(old_cmd) + len(old_rev), len(new_cmd) + len(new_rev)
print(f'\nreplies scanned      : {replies}')
print(f'old parser distinct  : {old_total}  (cmd {len(old_cmd)}, review {len(old_rev)})')
print(f'new parser distinct  : {new_total}  (cmd {len(new_cmd)}, review {len(new_rev)})')
print(f'multiline recovered  : {multiline}')
print(f'warn reasons         : {dict(warns)}')
print(f'suppressed           : {dict(collections.Counter(dropped.values()))}')

fail = 0

# Every body the old parser saw that the new one does not emit must be explained by a
# deliberate suppression rule. An unexplained one is a real regression.
unexplained = [b for b in (old_cmd | old_rev) - (new_cmd | new_rev) if b not in dropped]
if unexplained:
    fail = 1
    print(f'\nFAIL: {len(unexplained)} body(ies) lost with no suppression reason:')
    for b in unexplained[:10]:
        print(f'   {b[:100]!r}')
else:
    print('\nOK: every body the old parser saw is either emitted or deliberately suppressed')

if new_total < old_total:
    fail = 1
    print(f'FAIL: new parser emits fewer items than the old one ({new_total} < {old_total})')

if warns['misclosed'] == 0:
    print('WARN: no </parameter> miscloses in this corpus - the main recovery path is untested here')

print(f'\nnet recovered: {new_total - old_total} items')
sys.exit(fail)
PY
