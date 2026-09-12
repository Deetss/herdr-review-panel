# Review Queue

A [Herdr](https://herdr.dev) plugin that gives Claude Code (or any other coding agent) a way to
hand things back to you instead of doing them itself: files worth a look, and commands only
you should run. Flag either in a reply and they show up in a split panel you can click, copy,
check off, and clear.

```
                                                                                clear  x
Click a file/command, its box to check off, or the x on any row/section to clear it.
─────────────────────────────────────────────────────────────────────────────────────

2026-09-09 13:23:33  my-project
  sudo apt update && sudo apt install -y fail2ban                                    x
  [ ] 2a. sudo systemctl enable --now fail2ban                                       x
  [x] echo "ok" >> ~/.ssh/authorized_keys                                            x
```

## Why

Agents sometimes need to tell you "go look at this file" or "run this command yourself" -
things they shouldn't do unattended (destructive commands, credentials, anything you want
eyes on first). Saying it in prose gets lost in a long reply. This plugin gives that request
a durable, actionable home: a panel that pops open the moment it happens, stays until you deal
with it, and survives long after the reply that created it has scrolled away.

## How it works

A Claude Code hook (`~/.claude/hooks/review-notify.sh`) watches for two tags in the agent's
replies:

- `` `<user_review>path/to/file</user_review>` `` - a file worth looking at. Click it to open
  in your editor.
- `` `<user_command>the command</user_command>` `` - a shell command *you* should run, not the
  agent. Click it to copy to your clipboard, or click its checkbox to mark it done. Add
  `step="2a"` for commands that must run in a specific order.

Matches get logged to `~/.claude/review.log` and the panel pops open automatically. The panel
itself is a small Rust + [ratatui](https://ratatui.rs) app - real mouse and keyboard handling,
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

To see what the hook decided and why, set `REVIEW_NOTIFY_DEBUG=1` (or `2` to include the raw
reply) and read `~/.claude/review-debug.log`.

### Panel controls

| Action | Mouse | Keyboard |
|---|---|---|
| Open a file / copy a command | Click its text | `Enter` |
| Check a command off | Click its checkbox | `Space` or `d` |
| Clear one row | Click the `x` on its row | `Backspace`/`Delete` |
| Clear a whole section | Click the `x` on its header | `Backspace`/`Delete` on the header |
| Clear everything | Click `clear` (top) | - |
| Close the panel | Click `x` (top) | `Esc` or `q` |
| Scroll | Mouse wheel | `↑`/`↓`/`j`/`k` |

Checking a command off keeps it visible, struck through. Clearing removes it entirely. Both
persist across restarts (`~/.claude/review-done.log`, `~/.claude/review-cleared.log`) and sync
live across multiple open panel instances.

## On a phone (Collie)

[Collie](https://github.com/herdr-dev/collie) serves a mobile web UI for your herd over
Tailscale. It has no terminal emulator: it strips ANSI server-side, renders the pane grid as
text, and **sends no mouse events at all**. So the panel's click targets cannot work there,
and no amount of changing this plugin will make them.

Two things bridge the gap.

**The panel is keyboard-drivable.** Collie's Keys tray already ships a fixed keyboard with the
arrows, Enter, Space, Escape and Backspace, which cover navigate / activate / mark done /
close / clear one row. Only "clear the whole queue" needed a key, and it is `C` — shifted,
because there is no undo. `collie-keys.toml.example` puts it on a labelled button; read its
header first, because an unscoped row replaces the shipped presets on every pane.

Activating a command row also opens it full-screen. On the desktop that is alongside the
clipboard copy; on a phone it is the only way to read a long command, since OSC-52 cannot
reach a phone through a UI that strips ANSI.

**Queued commands become launcher rows.** `scripts/collie-sync.sh` runs at the end of every
hook invocation and mirrors the queue into Collie's `launchers.toml`, so each queued command
is a button on the phone's Launch section. It rewrites only the block between its own markers;
rows you wrote by hand are preserved. Items already checked off or cleared on the desktop drop
out, and running one from the phone marks it done, so the two surfaces agree.

Every row points at `scripts/review-run.sh <id>`, never at the command itself. That is
deliberate. `launchers.toml` is the allowlist `POST /api/launch` matches against and it
accepts no confirm option, so putting agent-proposed commands in it directly would make
anything the agent suggested one tap from running over Tailscale with no reading step — the
opposite of what `<user_command>` is for. Instead the allowlist holds only a fixed wrapper
invocation; the id resolves against `review.log` locally, the real command is printed, and it
runs only after an explicit `y`.

Set `REVIEW_PANEL_COLLIE_ROWS` to change how many rows are mirrored (default 12). Collie picks
up edits live, but you need to reload the page to see them.

## Install

```bash
herdr plugin install Deetss/herdr-review-panel
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
- Wrap any file you want reviewed in <user_review>path/to/file</user_review>.
- Wrap any command you should run yourself in <user_command>the command</user_command>
  (add step="N" for ordered steps). Write both tags wrapped in a single backtick so they
  render as code instead of raw text. Close each tag with its own name - closing with
  </parameter> is the most common way an item gets dropped. When describing the convention
  rather than making a request, put the example in a fenced code block; the hook ignores
  tags inside fences.
```

Open the panel manually with `herdr plugin action invoke deetss.review-panel.open`, or bind a
key to it in your Herdr `config.toml`:

```toml
[[keys.command]]
key = "prefix+r"
type = "plugin_action"
command = "deetss.review-panel.open"
```

## Development

Requires a Rust toolchain (stable) and [Herdr](https://herdr.dev) `>= 0.8.0`.

```bash
cargo build --release   # produces target/release/review-panel, which herdr-plugin.toml runs
cargo test              # unit tests covering log parsing/grouping and app state
cargo clippy --all-targets -- -D warnings
cargo fmt

./tests/parse/run.sh    # golden tests for the jq tag parser
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

`herdr plugin link .` registers a local checkout for testing without publishing.

**After changing the log format, rebuild release and restart any open panel.** herdr runs
`target/release/review-panel`, not the debug build, and an already-running panel keeps the
code it started with. A panel from before the change parses every new line to `None` and
skips it silently, so the queue looks empty while the log fills up normally - which is
indistinguishable from the hook not firing. `cargo build --release`, then close and reopen
the panel.

## License

[MIT](LICENSE)
