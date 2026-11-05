# Review queue: design constraints

Inputs for the round 2 mockup and the round 3 build. Everything here was read from the
code at commit 8758579.

## Pane widths

| Name | Pane columns | Drawable columns | Use |
|---|---|---|---|
| Normal | 40 | 39 | Default Orca split beside the agent pane |
| Narrow | 32 | 31 | Phone mirror (Orca Mobile) and cramped splits |

The pane width is whatever `orca terminal split` produces. It has no size option, so the
panel must lay out at any width. 40 and 32 are the two design points. Below 28 drawable
columns the panel only has to stay legible, not pretty.

`ui::draw` insets the frame by one row at the top and one column at the right, because
Orca overlays its own close and action buttons in the pane's top-right corner. The
drawable area is therefore `(width - 1) x (height - 1)`. The mockups draw that inset as a
faint gutter so the real column budget is visible.

## Cell grid

- One character per cell. Every glyph below renders in exactly one cell.
- Terminal cells are about 1:2 (width:height). The mockups use 10 x 20 px cells.
- Glyphs used: `[ ]`, `[x]`, `x` (clear), `⚠` (warn, U+26A0), `…` (U+2026), `·` (U+00B7),
  `─` (U+2500), `⏎` (U+23CE, multiline marker). Nothing else. No emoji, no box-drawing
  frame, no Nerd Font glyphs.
- Column and row numbers in the spec are 0-indexed within the drawable area.
- Width of a string is `chars().count()`, as in `ui::fit`. No wide (CJK) handling exists.

## Theme token source

`src/theme.rs` is the only source. It holds ratatui named ANSI colours (`Yellow`,
`Magenta`, `DarkGray`, `Gray`, `Cyan`, `Red`, `LightRed`) plus modifiers, and no RGB values.

The terminal resolves those names against its own palette. Omarchy reloads the terminal
palette when the theme changes, so the panel follows the Omarchy theme live with no code
involved. The mockup SVGs cannot reference "the theme", so they use fixed stand-in hex
values for each named colour. The hex values are illustrative only. The spec names tokens,
never hex.

| Token (colour) | Stand-in hex in SVG | Existing style fn |
|---|---|---|
| default fg | `#d8dadb` | `Span::raw` |
| bg | `#1d1f21` | terminal default |
| Cyan | `#8abeb7` | `header`, `status` |
| Magenta | `#b294bb` | `command` |
| Yellow | `#f0c674` | `item`, `close_button` |
| Gray | `#a8aaa9` | `checkbox` |
| DarkGray | `#707880` | `command_done` |
| Red | `#cc6666` | `clear_icon`, `clear_all_button` |
| LightRed | `#ff8c8c` | `warn_icon` |

Modifiers: `DIM` is drawn as 60 percent opacity, `BOLD` as bold weight, `CROSSED_OUT` as a
line through the text, `REVERSED` as swapped fg and bg, `UNDERLINED` as an underline.
Terminals without `CROSSED_OUT` drop it, so a done row must never rely on strikethrough
alone: the `[x]` glyph carries the state.

The design adds no colours. It adds style functions (see the spec's token table) built from
the colours above.

## Limits the tests enforce

All in `src/ui.rs` unit tests unless noted.

- `with_clear_glyph` pads a short row so the trailing `x` lands on the last column.
- A long row keeps its `x`: content is cut to `width - 3` minus the prefix, leaving one
  gap column. A path keeps its tail (`Trim::Head`), a command keeps its head (`Trim::Tail`).
- `fit_hint` drops whole segments separated by ` · `, never part of one.
- `render_report_prompt` options sit on their own key lines, two rows each.
- `src/log.rs` tests: mark keys stay one line (the escaped item is used, never the decoded
  one), cleared items do not reset group tracking, empty groups are pruned.
- `src/app.rs`: `CHECKBOX_END_COL = 6` is the boundary between "toggle done" and "copy"
  clicks. The build must keep the checkbox at columns 2 to 5.
- `tests/parse` and `tests/orca-pane` cover scripts, not layout.

No test asserts a column count for the whole panel. Round 3 adds buffer-level tests.

## How a build captures the panel

Use ratatui's `TestBackend` (already used in `ui.rs` tests). It needs no terminal and no
Orca.

1. Build an `App` with `App::with_paths` pointing at temp log, done and cleared files
   seeded with the sample content (the mockup's sessions and commands, no real data).
2. `Terminal::new(TestBackend::new(40, 24))`, then `terminal.draw(|f| { ui::draw(f, &app); })`.
3. Walk `terminal.backend().buffer()`. For every cell read `symbol()`, `fg`, `bg` and
   `modifier`, and write one line per row of text, plus a second dump of `fg/modifier` per
   span. Compare the text dump to the mockup text (a golden file per state and width).
4. Repeat at `TestBackend::new(32, 24)`.
5. For a PNG, render the cell dump to SVG with the same palette stand-ins as the mockup,
   then `rsvg-convert`. This gives a pixel-comparable image next to
   `docs/design/queue-mockup.png`.

`TestBackend` buffers carry the named ANSI colours, so the dump uses the same stand-in
table as above.
