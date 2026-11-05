# Phase 2 fidelity delta

Captures come from `TestBackend` dumps (`REVIEW_PANEL_CAPTURE_DIR=<dir> cargo test`),
rendered with the mockup's stand-in palette. Sample content is synthetic. Each pair is the
mockup panel on the left and the build on the right.

| Pair | File |
|---|---|
| (a) default, 40 | `default-40-pair.png` |
| (b) DONE expanded, 40 | `expanded-40-pair.png` |
| (d) empty, 40 | `empty-40-pair.png` |
| (a) default, 32 | `default-32-pair.png` |
| (b) DONE expanded, 32 | `expanded-32-pair.png` |
| (d) empty, 32 | `empty-32-pair.png` |

| Element | Spec value | Built value | Status |
|---|---|---|---|
| Header counts, 40 | `Queue: 4 open · 4 done`, cols 0 to 21 | same | fixed |
| Header counts, 32 | `4 open · 4 done`, cols 0 to 14 | same | fixed |
| Header counts, 32 empty | narrow form (brief overrides the mockup, which kept `Queue:`) | `0 open · 0 done` | fixed |
| `clear` button | cols 31 to 35 / 23 to 27, absent when empty | same | fixed |
| Divider | `─` x W, dim | same | fixed |
| OPEN list order | hub 17:28, hub 08:54, cqc 09:07, newest last | same | fixed |
| Step labels | `1.` and `2.` shown, orphan rule in `queue.rs` tests | same | fixed |
| Default cursor | newest open row (`cargo test`), box and command reversed | same | fixed |
| DONE summary lines | `hub  [x] 3 rows, 17:20`, `cqc  [x] 1 row, 09:05` | same | fixed |
| DONE expanded rows | struck through, `[x]` kept, summary reversed, `Enter: fold` hint | same | fixed |
| 32 col truncation | `systemctl restart o…`, `curl -s localhost:3000…`, `x` on every row at col 30 | same | fixed |
| Footer, open command row | 40: two full rows. 32: `Space done · Bksp clear` / `c clear done · H hidden` | same | fixed |
| Footer, summary row | `Enter fold · Bksp clear · c clear done` / `H hidden · S clear session` | same | fixed |
| Footer, empty | `H hidden · q close` on the last row | same | fixed |
| Scroll | offset 0, no page scroll at 24 rows | same | fixed |
| Divider glyph rendering | continuous line | dotted in the PNG | accepted: the capture script draws one `<text>` per cell, so `─` glyphs do not join in the rasteriser. The cells hold `─` in every column (asserted in `default_state_at_40_columns`). |
| DIM text brightness | opacity 0.6 | opacity 0.6 | accepted: same value, the pair only looks different because of image scaling |
| `H hidden` and `S clear session` hints | listed in the spec footer | shown, but the keys are phase 3 and do nothing yet | accepted: the brief asks for the spec's hints, and the keys land next phase |
