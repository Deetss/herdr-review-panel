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

## Install

```bash
herdr plugin install Deetss/herdr-review-panel
```

Then wire up the hook and tag conventions - add to your Claude Code settings
(`~/.claude/settings.json`) a `Stop` and `SubagentStop` hook pointing at
`~/.claude/hooks/review-notify.sh` (copy it from `scripts/` in this repo), and add the tag
conventions to your `CLAUDE.md` so the agent knows to use them:

```markdown
- Wrap any file you want reviewed in `<user_review>path/to/file</user_review>`.
- Wrap any command you should run yourself in `<user_command>the command</user_command>`
  (add `step="N"` for ordered steps). Write both tags wrapped in a single backtick so they
  render as code instead of raw text.
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
cargo test               # 27 unit tests covering log parsing/grouping and app state
cargo clippy --all-targets -- -D warnings
cargo fmt
```

`herdr plugin link .` registers a local checkout for testing without publishing.

## License

[MIT](LICENSE)
