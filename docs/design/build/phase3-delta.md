# Phase 3 fidelity delta

Captures come from `TestBackend` dumps (`REVIEW_PANEL_CAPTURE_DIR=<dir> cargo test`), rendered
with `rsvg-convert` at 10 x 20 px cells. Each pair is the mockup's panel (c) on the left and
the build on the right. Sample content is synthetic.

| Pair | File |
|---|---|
| (c) hidden view, 40 | `hidden-40-pair.png` |
| (c) hidden view, 32 | `hidden-32-pair.png` |

The six phase 2 captures were regenerated and their SVGs are byte-identical to the committed
ones, so those pairs still match.

## Phase 3

| Element | Spec value | Built value | Status |
|---|---|---|---|
| Header button, 40 | `restore` at cols 29 to 35 | same | fixed |
| Header button, 32 | `restore` at cols 21 to 27 | same | fixed |
| Header counts | exclude hidden rows | `4 open · 4 done` | fixed |
| HIDDEN label | `HIDDEN (3)` after a blank row | same | fixed |
| Session headers | `hub 17:28`, `cqc 09:07`, no `x`, dim bold repo | same | fixed |
| Tags | `superseded` / `cleared`, last char at `last` | same | fixed |
| 40 col truncation | 22 columns of room, `systemctl status orca` whole | same | fixed |
| 32 col truncation | `systemctl sta…`, `journalctl -u or…` | same | fixed |
| Cursor | first hidden row, box and text reversed | same | fixed |
| Footer, 40 | `r restore · R restore all · H back` on the last row | same | fixed |
| Footer, 32 | `r restore · R restore all` then `H back` | same | fixed |
| Blank row above the footer, 40 | blank row above the footer | row 22 is blank | fixed |
| Blank row above the footer, 32 | one blank row above the footer | row 21 is blank (the mockup had none) | fixed |
| Body scroll, 32 | rows overflow, list scrolls, footer fixed | 19 rows in an 18 row list, scrolled to the bottom, so the OPEN label is off the top | accepted: the blank row costs one body row, and the brief makes the footer the fixed part |
| Sample clocks and commands | mockup rows | same text and clocks | accepted: `cargo build` is hidden by a `hide` event and also superseded by the later done row, so `cleared` wins (spec 4.8) |
| Divider glyph rendering | continuous line | dotted in the PNG | accepted: same rasteriser limit as phase 2 |
