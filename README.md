# Review Queue

A review panel for [Orca](https://www.onorca.dev) that gives Claude Code (or any other coding
agent) a way to hand things back to you instead of doing them itself: files worth a look, and
commands only you should run. Flag either in a reply and they show up in a split panel you can
click, copy, check off, and clear.

```
Queue: 3 open · 1 done                                                        clear  x
─────────────────────────────────────────────────────────────────────────────────────
OPEN
my-project 13:23                                                                    x
      /home/me/my-project/etc/fail2ban/jail.local                                   x
  [ ] 1. sudo systemctl enable --now fail2ban                                       x
  [ ] 2. sudo systemctl status fail2ban                                             x

DONE (1)
my-project  [x] 1 row, 13:20                                                        x
```

## Why

Agents sometimes need to tell you "go look at this file" or "run this command yourself" -
things they shouldn't do unattended (destructive commands, credentials, anything you want
eyes on first). Saying it in prose gets lost in a long reply. This panel gives that request
a durable, actionable home: a panel that pops open the moment it happens, stays until you deal
with it, and survives long after the reply that created it has scrolled away.

## How it works

A Claude Code hook (`~/.claude/hooks/review-notify.sh`) watches for two tags in the agent's
replies:

- `` `<user_review>path/to/file</user_review>` `` - a file worth looking at. Click it to open
  in your editor.
- `` `<user_command>the command</user_command>` `` - a shell command *you* should run, not the
  agent. Click it to copy to your clipboard, or click its checkbox to mark it done. Add
  `step="2a"` for commands that must run in a specific order, and `terminal="ssh:host"` (or
  `terminal="pane:2"`, or any short label) when the command targets somewhere other than
  wherever you happen to click it from - a remote host, say. Left off, a command run inside
  tmux is tagged automatically with its pane (`tmux display-message -p '#S:#I'`); outside
  tmux with no explicit `terminal=`, it carries no location at all.

Either tag takes `id="name"` (letters, digits, `.`, `_`, `-`, up to 32). A later item in the
same session with the same id **supersedes** the earlier one, so an agent that regenerates a
file or revises a command swaps the stale row instead of stacking a second one. A later item
of the same kind and value from the same session supersedes too, with no id needed. A
superseded row is hidden, not deleted. The log stays append-only; supersession is derived
whenever rows are built, so a restart converges on the same view. Clicking a file row opens it, copies its full path, and shows the path in the
detail overlay, since a narrow pane only has room for the tail.

Matches get logged to a queue scoped to the Orca tab they came from (`ORCA_TAB_ID`, under
`~/.claude/review/`), and the panel pops open automatically as an `orca terminal split` to the
right of the pane that flagged the item, then hands focus back to it. The panel is found again
by its pane title (`Review Queue`, which the binary sets itself), so `open` is idempotent and
`close`/`toggle` work from the panel's own `x`. The panel itself is a small Rust +
[ratatui](https://ratatui.rs) app - real mouse and keyboard handling,
not a hand-rolled terminal escape-code parser (an earlier bash version tried that and it broke
in ways that were hard to reproduce and fix).

Tag scanning lives in `scripts/review-parse.jq`, a pure stdin-to-stdout filter with golden
tests in `tests/parse/`; `scripts/review-notify.sh` is side effects only. The parser is
deliberately forgiving in one direction and strict in the other. It accepts a multiline body
and a tag mistakenly closed with `</parameter>`, because those are real things agents emit and
silently dropping them is worse than logging them with a warning. It rejects tags carrying
anything other than a `step` attribute, tags inside a fenced code block, and the documentation
examples themselves, because a reply *describing* the convention should not file a queue item.

Rows flagged with a reason are marked with a warning glyph rather than hidden: `misclosed` for
the wrong closing tag, `unclosed` for an open tag with no terminator, `missing` for a review
target that does not resolve on disk, `prose` for a "command" that reads like a paraphrased
task, `truncated` for an over-long body.

Every subprocess call `review-notify.sh` makes after the log write - the `notify-send` toast
and the panel pop - runs under a `with_timeout` guard, so a hung `orca` call costs only that
one side effect (logged to the debug log) instead of Claude Code externally killing the whole
hook at its own 10s budget with no trace.

When you're done with a group (or the whole queue), the panel asks whether it all went well
and, on "yes," tells the agent that flagged it to continue via `orca terminal send`, so
nobody has to go back and prompt it by hand. The top `clear` button hides every open and done
row without telling the agent anything, and the panel stays open on the empty state.

To see what the hook decided and why, set `REVIEW_NOTIFY_DEBUG=1` (or `2` to include the raw
reply) and read `~/.claude/review-debug.log`.

### Panel controls

| Action | Mouse | Keyboard |
|---|---|---|
| Open a file / copy a command (copying also checks it off) | Click its text | `Enter` |
| Check a command off, or reopen a done one | Click its checkbox | `Space` or `d` |
| Expand or fold a session's done rows | Click its DONE summary line | `Enter` on the summary |
| Clear one row | Click the `x` on its row | `Backspace`/`Delete` |
| Clear a whole session | Click the `x` on its header | `Backspace`/`Delete` on the header |
| Clear every done row | Click the `x` on a DONE summary (one session) | `c` |
| Clear a whole session | - | `S` on any of its rows |
| Clear everything | Click `clear` (top) | `C` |
| Show or leave the hidden view | - | `H` (`Esc` also leaves it) |
| Restore one hidden row, or a session from its header | Click the row's checkbox | `r` |
| Restore every hidden row | Click `restore` (top, hidden view) | `R` |
| Close the panel | Click `x` (top) | `Esc` or `q` |
| Clear does not notify the agent - it only hides the queue. Confirming the finished queue does. | - | - |
| Scroll | Mouse wheel | `↑`/`↓`/`j`/`k` |

The header shows the open and done counts. The body is an OPEN list, one header per session
with its repo and the time of its newest open row (most recent session last), then a DONE
drawer with one summary line per session, collapsed by default. A step label (`1.`) only shows
when another open row in the same session has one. Checking a command off moves it to the
drawer, and expanding the summary shows it struck through. Unchecking reopens it. Clearing
hides a row from the panel. Two footer rows show the keys for whatever the cursor is on.
The panel starts with the cursor on the newest open row.

`H` adds a HIDDEN section below DONE, grouped by session. Each row is tagged `cleared` (hidden
by a clear) or `superseded` (replaced by a newer command with the same `id`, or the same kind
and value, from the same session). A row that is both reads `cleared`. The `clear` button
becomes `restore`, and the cursor only visits hidden rows. `r` and `R` append `show` events,
and a `show` on a superseded row brings it back beside its replacement.

## Where state lives

No file is ever truncated, rewritten or pruned. `~/.claude/review.log` holds the flagged items.
`~/.claude/review-done.log` and `~/.claude/review-cleared.log` hold one event per line,
`{unix_ts} done|open {key}` and `{unix_ts} hide|show {key}`, and the last event for a key wins.
A line with no timestamp is a legacy mark and reads as `done` or `hide`. Checking, unchecking
and clearing append events, so all three persist across restarts and sync live across open
panel instances. `plugin.sh clear` runs `review-panel --hide-all <review.log> <cleared.log>`,
and that and the `clear` button append `hide` events and leave `review.log` untouched. The files grow without bound by design. There is no time window and no
sweep of done rows, so `REVIEW_PANEL_WINDOW_MINUTES` no longer exists.

A clicked file opens in Orca's editor (`orca file open`) when it lives inside the current
worktree, and falls back to VS Code, then `xdg-open` on its directory, otherwise.

## On a phone

Orca Mobile mirrors terminals, so the panel is already on your phone. It is fully
keyboard-drivable for clients without a mouse: arrows, Enter, Space, Escape and Backspace
cover navigate / activate / mark done / close / clear one row, and `C` clears the whole
queue (shifted, so a stray keypress does not hide it). Activating a command row also opens it
full-screen, which is the only way to read a long command where OSC-52 clipboard copy
cannot reach.

## Install

```bash
git clone https://github.com/Deetss/herdr-review-panel.git ~/dev/personal/herdr-review-panel
cd ~/dev/personal/herdr-review-panel && cargo build --release
```

Then wire up the hook. Add a `Stop` and `SubagentStop` hook to your Claude Code settings
(`~/.claude/settings.json`) pointing at `~/.claude/hooks/review-notify.sh`, and make that file
a three-line shim into this checkout rather than a copy of it:

```bash
#!/bin/bash
target="$HOME/dev/personal/herdr-review-panel/scripts/review-notify.sh"
[ -r "$target" ] || exit 0
exec bash "$target"
```

A copy drifts. This one did: the panel gained a `warn=` field that the deployed writer never
emitted, and nobody noticed because both halves looked fine on their own.

Finally add the tag conventions to your `CLAUDE.md` so the agent knows to use them:

```markdown
- Wrap any file you want reviewed in <user_review>/absolute/path/to/file</user_review>, always
  the full absolute path. Add id="short-name" to an item you may re-flag, so the new one
  replaces the old one in the queue instead of stacking beside it.
- Wrap any command you should run yourself in <user_command>the command</user_command>
  (add step="N" for ordered steps, and terminal="ssh:host" when it targets somewhere other
  than wherever you're about to click it). Write both tags wrapped in a single backtick so
  they render as code instead of raw text. Close each tag with its own name - closing with
  </parameter> is the most common way an item gets dropped. When describing the convention
  rather than making a request, put the example in a fenced code block; the hook ignores tags
  inside fences.
```

Open, close or toggle the panel by hand from any Orca terminal with `scripts/plugin.sh open`
(or `close` / `toggle`). The scripts call `orca-ide`, or `$ORCA_CLI_COMMAND` when set, and never
a bare `orca`: outside an Orca terminal that name resolves to the GNOME screen reader.

## Watcher panes

`scripts/orca-pane` opens agent watchers (subagent transcript tails and similar) in one
right-hand column per Orca tab. `orca terminal split` has no size or position options and
halves whatever pane it targets, so naive callers shrink the main pane to a sliver.

```bash
orca-pane watch --label <name> -- <command...>
orca-pane list
orca-pane close <label|all>
```

The first watcher splits the agent pane side by side. Later watchers split the newest
watcher top/bottom. At `ORCA_PANE_MAX` watchers (default 3) the oldest is interrupted and
reused. The command runs as `<command>; exit`, so the pane closes when it ends, and dead
handles are pruned before every decision. State lives in
`${XDG_STATE_HOME:-~/.local/state}/orca-pane/<tab-id>.json`. Outside Orca it does nothing.

The Review Queue sidebar stays its own split in `plugin.sh`. It is a persistent panel with
its own per-tab dedupe and close-on-exit, not a watcher, so it does not count toward the cap.

## Development

Requires a Rust toolchain (stable) and [Orca](https://www.onorca.dev) for the live panel.

```bash
cargo build --release   # produces target/release/review-panel, which plugin.sh launches
cargo test              # unit tests covering log parsing/grouping, app state and panel layout
# REVIEW_PANEL_CAPTURE_DIR=<dir> cargo test also writes each layout state as an SVG there
cargo clippy --all-targets -- -D warnings
cargo fmt

./tests/parse/run.sh    # golden tests for the jq tag parser
./tests/orca-pane/run.sh # orca-pane layout tests against a fake orca-ide
./tests/parse/corpus.sh # replays real transcripts, compares against the old parser
```

`tests/parse/run.sh` is the fast one and the one to add a case to when a tag shape gets
missed: drop the payload in `tests/parse/cases/<name>.json` and freeze the output next to
it as `<name>.expected`.

`tests/parse/corpus.sh` replays every reply in `~/.claude/projects` that ever contained a
tag and fails if any body the old grep-based parser caught is now dropped without a
suppression reason. Its harvested corpus contains real session content and is gitignored;
never commit it.

`tests/fixtures/review.log` is written by the hook and read back by a Rust test. It is the
only thing pinning jq's `@tsv` escaping to the Rust `unescape` that decodes it, so
regenerate it with the hook rather than editing it by hand.

**After changing anything under `src/`, rebuild release and restart any open panel.** The
panel runs `target/release/review-panel`, not the debug build, and an already-running panel keeps
the code it started with - not just for log-format changes, for any behavior change. A stale
panel doesn't error, it just silently keeps running whatever it started with, which is
indistinguishable from the fix never having landed. `scripts/plugin.sh open`/`toggle` now
checks this itself (`rebuild_if_stale`, dev checkouts only) and rebuilds before opening, but
a panel that's already open still needs closing and reopening - a rebuild alone does not
reach a running process.

## License

[MIT](LICENSE)
