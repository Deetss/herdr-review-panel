# review-parse.jq - pure parser for the review-queue Stop hook.
#
# In:  the raw hook payload (JSON) on stdin.
# Out: TSV records, one per line, every field @tsv-escaped so \t \n \r \\ are
#      two-character sequences and a record can never span lines.
#
#   ctx  \t session \t cwd
#   item \t kind    \t step   \t warn    \t body     -> log it
#   near \t kind    \t reason \t preview            -> log as a warn= row
#   drop \t kind    \t reason \t body               -> debug only, never logged
#   stat \t key=value ...                           -> debug only
#
# No side effects, so this is testable standalone:
#   jq -r -f review-parse.jq < fixture.json

def MAXBODY: 4000;
def MAXLOG: 2000;   # keep a written line under PIPE_BUF so concurrent appends stay atomic
def MAXNEAR: 2;     # a meta-heavy reply must not flood the queue with warn rows

def TAG_RE:
    "<user_(?<tag>command|review)(?<attrs>[^>]*)>"
  # The lookahead is load-bearing: it stops a lazy body from swallowing the next tag
  # when this one is genuinely unclosed. Without it an unclosed open steals the
  # following tag's closer and merges two items into one garbage row.
  + "(?<body>(?:(?!</user_command>|</user_review>|</parameter>|<user_command\\b|<user_review\\b).){0,"
  + (MAXBODY|tostring) + "}?)"
  # Tolerates the model closing with </parameter>, which it does often enough to be
  # the single largest source of dropped items.
  + "(?<close></user_command>|</user_review>|</parameter>)";

def FENCE_RE: "(?<f>```|~~~).*?\\k<f>";

# CLAUDE.md's own illustrative bodies. A reply quoting the convention is documentation,
# not a request. Exact match only, deliberately narrow, so a real command is never hit.
def EXAMPLES: ["the command", "path/to/file", "some shell command", "the file", "cmd", "command"];

def FUNCWORDS: "\\b(?:the|a|an|in|to|on|your|my|and|then|from|with|after|above|before|once|please|kindly)\\b";

def cap($m; $n): ($m.captures | map(select(.name == $n)) | .[0].string) // "";

# Optional fields are emitted as "-" rather than empty. bash treats a tab as IFS
# whitespace and collapses runs of it, so two adjacent empty fields would silently
# merge and shift every field after them. "-" is unambiguous here: step must start
# [A-Za-z0-9] and warn comes from a fixed vocabulary, so neither can be a literal "-".
def dash: if . == "" then "-" else . end;

def step_of($a): ([$a | capture("step\\s*=\\s*\"(?<s>[^\"]*)\"")] | .[0].s) // "";

# Only an empty attribute list or a single well-formed step="..." is a real tag. This one
# rule is what kills a reply that quotes the parser's own regex: the attrs it captures
# from `<user_command\b[^>]*>` are `\b[^`, which fails here.
def attrs_ok($a): $a | test("^\\s*(?:step\\s*=\\s*\"[A-Za-z0-9][A-Za-z0-9._-]{0,31}\"\\s*)?$");

def shelly($b): $b | test("[/|><$=\"'\\\\`*&;]|\\s-{1,2}[A-Za-z]");

# Reads like a paraphrased task rather than a runnable command. Warns, never drops.
# Anything with shell metacharacters or a flag is exempt outright: `grep -E "^(a|b)"`
# otherwise trips the function-word count on its own regex, and a real command must
# never be second-guessed. A single word is exempt too, so `a` is not "starts with an
# article". Both exemptions cost recall on prose that looks like a command, which is
# the right side to err on when the only consequence is a missing amber glyph.
def is_prose($b):
  (shelly($b) | not)
  and ( ($b | split(" ") | map(select(length > 0)) | length) as $w
        | ($w >= 2)
          and ( ([$b | scan("(?i)" + FUNCWORDS)] | length) >= 2
                or ($b | test("^\\s*(?:the|this|that|a|an|your|my|it|e\\.g\\.|i\\.e\\.)\\b"; "i"))
                or ($w >= 3 and ($b | test("(?i)" + FUNCWORDS))) ) );

. as $p
| (($p.last_assistant_message // "")) as $msg
| ([$msg | match(FENCE_RE; "gm") | {s: .offset, e: (.offset + .length)}]) as $fences
| ([$msg | match(TAG_RE; "gm")]) as $ms
| ([$msg | match("<user_(?:command|review)\\b"; "g")] | length) as $opens
| ([$msg | match("</user_(?:command|review)>"; "g")] | length) as $closes
| ["ctx", ($p.session_id // "unknown" | dash), ($p.cwd // "" | dash)],
  ( $ms[]
    | . as $m
    | cap($m; "tag")   as $kind
    | cap($m; "attrs") as $attrs
    | cap($m; "body")  as $raw
    | cap($m; "close") as $close
    | ($raw | sub("^\\s+"; "") | sub("\\s+$"; "")) as $body
    | (any($fences[]; $m.offset >= .s and $m.offset < .e)) as $fenced
    | ( [ (if $close == "</parameter>" then "misclosed" else empty end),
          (if ($raw | length) > MAXLOG then "truncated" else empty end),
          (if $kind == "command" and is_prose($body) then "prose" else empty end)
        ] | join(",") ) as $warn
    | if   (attrs_ok($attrs) | not)   then ["drop", $kind, "badattrs", $body]
      elif ($body | length) == 0      then ["near", $kind, "empty", ($attrs | dash)]
      elif (EXAMPLES | index($body))  then ["drop", $kind, "example", $body]
      elif $fenced                    then ["drop", $kind, "fenced", $body]
      else ["item", $kind, (step_of($attrs) | dash), ($warn | dash), ($body[0:MAXLOG])]
      end ),
  # Report only the opens that no match consumed, by offset. Counting opens against
  # matches would re-report tags that parsed fine alongside a genuinely unclosed one.
  ( [ $msg
      | match("<user_(?:command|review)\\b[^>]{0,40}>[^\\n]{0,60}"; "g")
      | . as $o
      | select( any($ms[]; $o.offset >= .offset and $o.offset < (.offset + .length)) | not )
      | $o.string ][0:MAXNEAR][]
    | ["near", "tag", "unclosed", .] ),
  ["stat", "opens=\($opens)", "closes=\($closes)", "matched=\($ms|length)",
           "fences=\($fences|length)", "bytes=\($msg|length)"]
| @tsv
