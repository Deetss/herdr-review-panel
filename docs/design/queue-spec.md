# Review queue: build contract (round 2)

Source of truth for round 3. Visuals: `docs/design/queue-mockup.svg` (40 columns) and
`docs/design/queue-mockup-narrow.svg` (32 columns). Limits and capture path:
`docs/design/constraints.md`. Where this file and a mockup disagree, this file wins.

Coordinates are 0-indexed within the drawable area (pane minus the 1 row / 1 column Orca
inset). `W` is the drawable width (39 normal, 31 narrow), `last` is `W - 1`, `H` is the
drawable height. All widths count characters.

## 1. Decisions made in this round

Flagged for the user because the approved wireframe left them open.

1. **Header count.** The wireframe says `3 open` above four open rows. The build counts
   open rows, so the mockup shows `4 open`.
2. **`[D]` in the wireframe header.** No key or action was assigned to it, so it is not drawn.
   The drawer opens and folds with `Enter` on a DONE summary line.
3. **`C` and the `clear` button.** They hide everything (open and done) and the panel
   stays open on the empty state. Today they truncate the logs and close the panel.
4. **Footer is two rows, bottom anchored**, because one row cannot hold the key list at 40
   columns and still show `H hidden`.
5. **Per-row `x`** stays on every hideable row, because it is the mouse and phone route to
   hide. It is dim red so it does not compete with the content.
6. **Time-based behaviour is removed.** The 10 minute log window
   (`REVIEW_PANEL_WINDOW_MINUTES`) and the 10 second sweep into a "Completed" section are
   gone. A row marked done moves to the DONE drawer immediately.
7. **Wording.** The footer says "clear" (the approved wireframe's word). Internally and in
   the hidden view the state is "hidden", and `r` reverses it.

## 2. Frame

| Zone | Rows | Owner |
|---|---|---|
| Header | `0` | fixed |
| Divider | `1` | fixed |
| Body | `2` to `H - 3` | scrolls |
| Footer | `H - 2` to `H - 1` (reserved, 1 or 2 used) | fixed |

Nothing outside the body scrolls. The body is one flat row list (OPEN, DONE, HIDDEN
stacked) and `App.scroll` is the single scroll offset. The mouse wheel moves 3 rows per
tick (`WHEEL_SCROLL_LINES`). Keyboard movement scrolls just enough to keep the cursor row
visible. A one-row footer sits on row `H - 1`, leaving row `H - 2` blank.

The toast (`status`) is unchanged. It overlays the last drawable row, right aligned, Cyan
bold.

## 3. Style tokens

Existing functions in `src/theme.rs` are reused as is. Four are new and add no colour.

| Token | Colour | Modifiers | New |
|---|---|---|---|
| `header()` | Cyan | BOLD | no |
| `header_highlighted()` | Cyan | BOLD, REVERSED | no |
| `dim()` | default | DIM | no |
| `command()` / `command_highlighted()` | Magenta | none / REVERSED | no |
| `command_done()` | DarkGray | CROSSED_OUT | no |
| `checkbox()` / `checkbox_highlighted()` | Gray | none / REVERSED | no |
| `clear_icon()` | Red | DIM | no |
| `clear_all_button()` / `_highlighted()` | Red | none / REVERSED | no |
| `close_button()` / `_highlighted()` | Yellow | none / REVERSED | no |
| `warn_icon()` | LightRed | BOLD | no |
| `status()` | Cyan | BOLD | no |
| `section()` | Gray | BOLD | yes |
| `hidden_header()` | Cyan | BOLD, DIM | yes |
| `hidden_command()` / `_highlighted()` | DarkGray | none / REVERSED | yes |
| `summary_highlighted()` | Gray | REVERSED | yes |

`section()` is the OPEN, DONE and HIDDEN label. `hidden_header()` is a session header
inside HIDDEN. `summary_highlighted()` is the cursor on a DONE summary line (whole text
reversed, the trailing `x` is not).

## 4. Elements

### 4.1 Header, row 0

| Span | Columns (40 / 32) | Text | Token |
|---|---|---|---|
| Counts | `0` to 21 / `0` to 14 | `Queue: {open} open · {done} done` | `header()` |
| Clear button | `31` to `35` / `23` to `27` | `clear` | `clear_all_button()` |
| Restore button | `29` to `35` / `21` to `27` | `restore` (hidden view only) | `clear_all_button()` |
| Close | `last` | `x` | `close_button()` |

- The button sits `2` columns left of the close `x` (existing layout, `ui::draw`).
- Counts get whatever is left of the row, minus a 2 column gap. When `Queue: ` does not fit
  it is dropped (this is why the 32 column header reads `4 open · 4 done`). If it still does
  not fit, ` · N done` goes, then the count is cut with `…`.
- `clear` shows only when `open + done > 0`. `restore` shows only in the hidden view and
  only when `hidden > 0`.
- Counts exclude hidden rows in every view.
- Hover: the button and `x` use their `_highlighted()` style.

### 4.2 Divider, row 1

`─` repeated `W` times, `dim()`.

### 4.3 Section label

| Text | Token | Shown when |
|---|---|---|
| `OPEN` | `section()` | at least one open row, or DONE has rows (then a `Nothing open.` line in `dim()` follows) |
| `DONE ({n})` | `section()` | `n > 0` |
| `HIDDEN ({n})` | `section()` | hidden view and `n > 0` |

Column `0`, no trailing glyph, not a cursor target.

A blank row separates OPEN from DONE and DONE from HIDDEN. When the whole queue is empty
the body is the single line `Nothing to review.` (`dim()`) on row 2 and no label shows.

DONE hint: on the DONE label row, right aligned to `last`, `dim()`. Text is `Enter: open`
while the cursor is on a collapsed summary line and `Enter: fold` while it is on an
expanded one or one of its rows. Absent otherwise.

### 4.4 Session header (OPEN)

`{repo}` then one space then `{HH:MM}`; trailing `x` at `last`.

| Span | Token |
|---|---|
| repo | `header()` (`header_highlighted()` under the cursor) |
| clock | `dim()` |
| trailing `x` | `clear_icon()`, always, even when highlighted |

- One header per session (the log's `session` field). The label is the `repo` field, so two
  sessions in the same repo show the same name and differ by clock.
- The clock is the time of the newest open row in that session, so it moves when a row
  arrives.
- Groups sort by that newest time ascending. The newest session is last.
- Truncation: the repo is cut with a tail `…` so that `repo + space + clock` fits in
  `last - 1` columns. The clock is never cut. Minimum repo width is 3.
- `x` or `S` hides the session's rows. It does not hide rows by deleting them.

### 4.5 Open command row

Layout: `"  "` + `"[ ] "` + optional `"⚠ "` + optional `"[{terminal}] "` + optional
`"{step}. "` + command + padding + `x` at `last`.

| Span | Columns | Token |
|---|---|---|
| indent | `0` to `1` | none |
| checkbox | `2` to `5` | `checkbox()` (`checkbox_highlighted()` under cursor) |
| warn | after checkbox | `warn_icon()` |
| terminal label | after warn | `dim()` |
| step label | after terminal | `dim()` |
| command | rest | `command()` (`command_highlighted()` under cursor) |
| `x` | `last` | `clear_icon()` |

- The checkbox stays in columns 2 to 5 so `CHECKBOX_END_COL = 6` is unchanged.
- **Step label rule.** A row shows its step only when at least one other open row in the
  same session has a step. A lone `2.` shows nothing. The rule is re-evaluated whenever a
  row leaves OPEN, so ticking `1.` done removes the `2.` from the last remaining step.
  Labels keep their text (`1`, `2a`) and render as `{step}. `.
- Newlines in a command render as ` ⏎ `.
- Truncation: the command is cut with a tail `…`. Budget is `last - 1` minus all columns
  before the command, leaving one gap column before the `x`. At 40 columns a row with a
  step has 28 columns for the command, without one 31. At 32 columns it is 20 and 23.
  `systemctl restart orca` at 32 columns becomes `systemctl restart o…`.
- Hover and cursor: only the checkbox and command spans reverse.

### 4.6 DONE summary line (collapsed or expanded)

`{repo}  [x] {n} row(s), {HH:MM}` and a trailing `x`.

| Span | Token |
|---|---|
| repo and the text after the box | `dim()` |
| `[x] ` | `checkbox()` |
| whole line under cursor | `summary_highlighted()` |
| trailing `x` | `clear_icon()` |

- One line per session that has done rows, ordered by newest done time ascending.
- Two spaces after the repo. `1 row` singular, `N rows` plural. The time is that session's
  newest done time.
- Truncation: the repo is cut with `…` first (minimum 3), then `, HH:MM` is dropped.
- `x` hides that session's done rows (same as `S` on it, restricted to done rows).
- Collapsed is the default. `Enter` or a click expands or folds it. Several sessions may
  be expanded at once. Expansion is view state only and is not persisted.

### 4.7 DONE row (shown under an expanded summary)

`"  "` + `"[x] "` + command + padding + `x`.

| Span | Token |
|---|---|
| checkbox | `checkbox()` (`checkbox_highlighted()` under cursor) |
| command | `command_done()` (DarkGray, CROSSED_OUT) |
| `x` | `clear_icon()` |

- Command truncation: tail `…`, budget `last - 1 - 6`. At 40 columns that is 32, at 32
  columns it is 24.
- Strikethrough is a redundancy. `[x]` carries the state on terminals that drop it.
- A command that gets the cursor does not change its colour, only the checkbox reverses and
  the command gains REVERSED on `command_done()`.
- No step labels in DONE.
- `Space`, `d` or a checkbox click reopens the row. It returns to OPEN at the position
  its log order gives it.

### 4.8 HIDDEN section (hidden view only)

Session header: `{repo} {clock}`, both spans `hidden_header()` for the repo and `dim()` for
the clock, no `x`. Clock is the newest hidden row's time.

Row: `"  "` + checkbox + command + padding + reason at `last`.

| Span | Token |
|---|---|
| checkbox | `checkbox()` (`[ ]` or `[x]` as the row was when hidden) |
| command | `hidden_command()` (`hidden_command_highlighted()` under cursor), no strikethrough |
| reason | `dim()`, right aligned so its last character is at `last` |

- Reason is `cleared` for a hide event and `superseded` for a row replaced by a newer
  command with the same `id` or the same kind and value from the same session. If a row
  is both, `cleared` wins.
- Truncation: the reason is never cut. The command is cut with a tail `…` to
  `last - 6 - len(reason) - 1` columns. At 40 columns the 22 column room fits
  `systemctl status orca`; at 32 columns it becomes `systemctl sta…`.
- Hidden done rows keep `[x]`.
- No `x` in this view. The row's one action is restore.

### 4.9 Footer, rows `H - 2` and `H - 1`

All text `dim()`. Segments are joined with ` · `. They fill row `H - 2` greedily, then row
`H - 1`. A segment that fits on neither row is dropped whole, as are all after it. The
first row never exceeds `W` columns.

Segment order and context (the segment set depends on what the cursor is on):

| Cursor on | Segments in order |
|---|---|
| open command row | `Space done`, `Bksp clear`, `c clear done`, `H hidden`, `S clear session`, `Enter copy` |
| open file row | `Bksp clear`, `c clear done`, `H hidden`, `S clear session`, `Enter open` |
| DONE summary line | `Enter fold` (or `Enter open`), `Bksp clear`, `c clear done`, `H hidden`, `S clear session` |
| DONE row | `Space reopen`, `Bksp clear`, `c clear done`, `H hidden`, `S clear session` |
| session header | `Bksp clear`, `c clear done`, `H hidden`, `S clear session` |
| hidden view, any row | `r restore`, `R restore all`, `H back` |
| empty queue | `H hidden`, `q close` |

Result at 40 columns for an open command row: row `H - 2` is
`Space done · Bksp clear · c clear done`, row `H - 1` is
`H hidden · S clear session · Enter copy`. At 32 columns the rows are
`Space done · Bksp clear` and `c clear done · H hidden`; `S clear session` and `Enter copy`
are dropped.

## 5. States

### 5.1 Default (on launch)

- Hidden view off. DONE drawer collapsed (every summary folded).
- The cursor is on the newest open command row (the last OPEN row), as `jump_to_latest`
  does today. If OPEN is empty the cursor is on the first DONE summary line. If the queue is
  empty there is no cursor.
- Scroll offset is the smallest one that shows the cursor row and, when it fits, the DONE
  label too.
- Hidden rows are not drawn and are not counted anywhere.

### 5.2 DONE expanded

`Enter` on a summary adds that session's done rows beneath it. The cursor stays on the
summary. The hint on the DONE label reads `Enter: fold`. Folding returns the cursor to
the summary and drops the rows.

### 5.3 Hidden view

`H` shows the view. The only change to the other sections is that a HIDDEN section follows
DONE, after a blank row. The header button becomes `restore`. The cursor moves to the first
hidden row and the footer switches to its hidden-view set. `H` or `Esc` returns to the
default view and puts the cursor back where it was. `q` still closes the panel.

If `hidden == 0` the view shows `HIDDEN (0)` with `Nothing hidden.` in `dim()`.

### 5.4 Empty queue

`open == 0` and `done == 0`. Header counts read `0 open · 0 done`, the `clear` button is
absent, the body is `Nothing to review.`, the footer is `H hidden · q close`. Reached on a
fresh panel, and after `C` or the `clear` button (hidden rows can then be restored).

### 5.5 Open empty, done not

OPEN label, then `Nothing open.` (`dim()`), blank, then the DONE drawer as in 5.1. The
report prompt (existing) fires when the last open command is marked done.

### 5.6 Overlays (unchanged)

The finished-queue prompt, the detail overlay and the toast keep their current rendering
and key capture. The prompt's "yes" and "error" answers no longer clear the queue. They
notify the agent and leave the rows in DONE.

### 5.7 Narrow (32 columns)

Same structure and tokens. The deltas are all consequences of the truncation and footer
rules above: shorter counts, `…` on commands longer than 20 columns, a 2 row footer that
drops `S clear session` and `Enter copy`, hidden-view footer wrapping to
`r restore · R restore all` then `H back`. No element is removed and no key changes.

## 6. Keys

| Key | Action |
|---|---|
| `Up`, `k` / `Down`, `j` | Move cursor to the previous or next row that has an action (skips labels and blanks) |
| `Space`, `d` | Toggle done on an open command row (moves it to DONE); reopen a DONE row. No effect on a summary line |
| `Backspace`, `Delete` | Hide the row under the cursor. On a session header or DONE summary line, hide that header's rows. In the hidden view, no effect |
| `c` | Hide every done row in every session |
| `S` | Hide every row (open and done) of the cursor's session |
| `C` | Hide everything (open and done). The panel stays open |
| `Enter` | Open command row: copy command. Open file row: open it. DONE summary: expand or fold. DONE row: copy |
| `H` | Toggle the hidden view |
| `r` | Hidden view: restore the row under the cursor, or the whole session when on its header |
| `R` | Hidden view: restore every hidden row |
| `Esc` | Hidden view: back to default. Detail overlay and prompt: dismiss. Otherwise close the panel |
| `q` | Dismiss the overlay if one is up, otherwise close the panel |
| `y`, `e`, `Esc` | Finish prompt, unchanged |

Mouse (existing hit-testing, same column rules): the `x` column of a row hides it, the
checkbox toggles done, the rest of a command row copies, a summary line expands or folds,
a hidden row's `r` is the click on its checkbox column. The `clear` button hides all,
`restore` restores all, the top-right `x` closes. Wheel scrolls the body.

## 7. Data model

Principle: **no log file is ever truncated, rewritten or pruned.** Every state change is one
appended line. Visibility is recomputed on load by replaying the files.

Files (each scoped per Orca tab by `plugin.sh`, as today):

| File | Holds |
|---|---|
| `review.log` | Flagged items. Append-only, as today. `plugin.sh clear` and the log window no longer cut it |
| `review-done.log` | Done events: `{unix_ts} done {key}` and `{unix_ts} open {key}` |
| `review-cleared.log` | Hide events: `{unix_ts} hide {key}` and `{unix_ts} show {key}` |

`key` is the existing `ts|session|raw_item` string (still single line, still escaped).
Replay rule for both files: the last event for a key wins. A line without a `{unix_ts}`
prefix is a legacy line and reads as `done` or `hide` with time 0, so existing files keep
working.

| Action | What is written | Result |
|---|---|---|
| Mark done | `done {key}` to the done log | Row moves to DONE |
| Reopen | `open {key}` to the done log (the old `unmark` rewrite is removed) | Row returns to OPEN |
| Hide row | `hide {key}` to the cleared log | Row leaves its section, appears in HIDDEN as `cleared` |
| Hide session (`S`, header `x`) | one `hide {key}` per row of that session | Same, for each row |
| Hide done (`c`, summary `x`) | one `hide {key}` per done row in scope | Rows leave DONE, keep `[x]` in HIDDEN |
| Superseded | nothing. Derived at load: an earlier row is superseded when a later row from the same session has the same `id`, or the same kind and value | Row leaves OPEN, appears in HIDDEN as `superseded` |
| Clear all (`C`, `clear`) | one `hide {key}` per currently visible row, open and done | Empty state. New rows arriving later still show |
| Restore (`r`, `R`) | `show {key}` per row | Row is visible again, in the section its done state gives it |

Visibility of a row is decided in this order: the last cleared-log event if any (`hide`
hides, `show` shows), otherwise superseded means hidden, otherwise visible. A `show` on a
superseded row therefore brings the old row back next to its replacement.

Consequences for the code:

- `done::unmark` and its rewrite are replaced by an `open` event.
- `cleared::mark` becomes an event writer with a `show` counterpart.
- `actions::clear_all` and `plugin.sh clear` stop truncating. They append `hide` events
  (the shell mode can skip rows the cleared log already hides) and return.
- `App.window_minutes` and `COMPLETE_DELAY` are removed, along with the synthetic
  `Completed` group.
- The files grow without bound by design. Compaction, if ever wanted, is a separate
  explicit action and not part of this change.
