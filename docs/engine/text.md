# pm-text and the font registry (Phase 1)

Design notes for `engine/crates/pm-text`, the font registry export in
`engine/crates/pm-bridge`, and its wiring in Electron main
(`apps/desktop/src/main/engine-fonts.ts`). Plan: section 12, with section 26
amendment 10. Spike: `docs/engine/spikes/S4.md`.

## Crate layout

| Module | What it is |
|---|---|
| `blink` | Rust port of Blink's `measureText` over HarfRust: segmentation, font fallback order, shaping with CoreText `trak` and optical sizing, Skia advances |
| `compat` | Level 1 layout: `text-layout.ts` wrapping with UTF-16 semantics, raster.ts baselines (`size × 0.82`), carets, and `fillText` glyph runs |
| `level2` (feature `level2`) | Level 2 layout on Parley: grapheme-aware breaking, font-metric baselines, fallback from the registry's manifest |
| `registry` (feature `system`, macOS) | `CoreTextRegistry`: Blink's macOS font matching, enumeration, project fonts (Blink's font selection algorithm), variable-font sources, the fallback manifest, subsets |
| `answers` | The registry boundary as data (`Query`, `Answers`, `ReplayResolver`, `Recorder`, `answer_batch`) |
| `subset` | Metrics-and-shaping subsets with the amended table list (adds `maxp`, `feat`, `STAT`, `ankr`; synthesizes `HVAR` for `gvar`-only fonts) |
| `corpus` | The S4 corpus, its Canvas2D fixture, and the comparison |

Layout never calls a platform API. Native and WASM builds run the same code
over the same answers and produce byte-identical output.

## The registry boundary

Main owns font resolution; the renderer (`pm-wasm`) owns layout.

1. The renderer lays text out over a `ReplayResolver`. A query the answers
   lack is recorded in `misses`; a face whose bytes are not loaded yet is
   recorded in `pending_blobs`. The layout continues with the next font in
   Blink's order (the "metrics fallback"), so a frame always has a layout.
2. The renderer sends the misses to main (`engine-fonts:answer`, a JSON
   `Query[]`). Main answers with `{ generation, faces, answers }` (pm-text's
   `Answers` plus the registry generation).
3. The renderer merges the reply (`add_answers`), requests the subset of every
   face named and every pending face (`engine-fonts:subset`, `(blob, index)`),
   adds them (`add_font(blob, index, subset, 0)`), and lays the text out again
   with a fresh fallback cache (`Compat::new` or `reset_session`). This is the
   re-layout event.
4. Repeat until a pass reports no misses and no pending faces.
   `tests/registry.rs` runs this loop from empty answers to convergence on
   the native layout (4 rounds for mixed Latin, CJK, Arabic, Hebrew,
   Devanagari, emoji, system UI bold italic, and Skia axes).

On project open the document's families are prefetched the same way (one
`answer` batch with a `family` query per family, weight, style, and size,
then their subsets) before the mirror reports ready.

Face ids belong to the registry that issued them; `answer` refuses queries
that name unknown faces. The generation increments whenever project fonts or
sources change. Answers given under an older generation may be stale (a
family that resolved to an installed font may now resolve to a project
font): the renderer drops its answers and starts again from step 1.

Queries key floats by their bits (`f32::to_bits`), so a replay answers
exactly what was asked. In JavaScript:
`new Uint32Array(new Float32Array([v]).buffer)[0]`.

## Main (before Phase 2)

`POWERMOVE_ENGINE_FONTS=1` turns it on. Main loads pm-bridge's
`FontRegistry` from `engine/target/pm-bridge/pm_bridge.node`
(`engine/crates/pm-bridge/scripts/build.sh`) or `POWERMOVE_PM_BRIDGE`, and
registers these channels (trusted editor senders only; every reply is
`{ ok: true, value } | { ok: false, error }`):

| Channel | Arguments | Value |
|---|---|---|
| `engine-fonts:answer` | `Query[]` JSON | `{ generation, faces, answers }` JSON |
| `engine-fonts:subset` | blob key (32 hex), face index | subset bytes |
| `engine-fonts:manifest` | base family (`system-ui`) | `FallbackManifest` JSON |
| `engine-fonts:family-faces` | family | `FaceSource[]` JSON |
| `engine-fonts:add-project-font` | family, bytes, `{ weight, style, stretch }` CSS strings, face index | new generation |
| `engine-fonts:set-font-source` | family, bytes | new generation |
| `engine-fonts:clear-project-fonts` | | new generation |
| `engine-fonts:generation` | | generation |

The registry runs on main's JavaScript thread (the process main thread;
CoreText matching uses AppKit's font manager). Measured costs on that thread
(Apple M4 Max, macOS 27.0, Bun 1.3.14 and Electron 44 as Node, `scripts/fonts-smoke.mjs`):

| Call | Cost |
|---|---|
| `new FontRegistry()` | 0.05 ms |
| `familyNames()` | 15 to 18 ms |
| `answer` (3 family queries) | 3.6 ms |
| `answer` (1 CJK fallback query) | 7.7 ms |
| `subset` first / cached | 0.04 to 0.64 ms (SF, 840 KB) / under 0.05 ms |
| `fallbackManifest('system-ui')` first / cached | about 0.6 s / 23 µs |

The first manifest is expensive because CoreText loads every substitute;
level 2 consumers should request it once, off the startup path.

## Level 2 inputs

Level 2 lays out over fonts the host adds (`Level2::add_font(key, bytes,
family)`) and the fallback manifest (`set_manifest`). The manifest lists, per
script and language, the families CoreText substitutes for a sample
character, common-script families (math, dingbats, combining marks,
symbols) appended to every script, the emoji family, and every face those
families need with the name to add it under (hidden system families have no
public name). In `pm-wasm` the bytes are subsets.

## Gates and how to run them

`engine/crates/pm-text/scripts/check.sh` runs everything:

- `tests/corpus.rs`: 5,200 cases against the checked-in Canvas2D capture
  (`tests/fixtures/canvas2d-corpus.json.gz`): identical breaks, advances
  within 0.25 px (regression sentinel 0.01 px). Measured: max error
  0.00105 px, p99 0.00026 px. Replays over full fonts and over subsets are
  bit-identical to the native run; subsets are 2.92% of their fonts
  (590.7 MB to 17.23 MB over 137 faces) and must stay under 5%.
- `tests/registry.rs`: enumeration, project fonts, variable sources, the
  manifest, family faces, subset tables, and the incremental protocol.
- `tests/level2.rs`: level 2 breaking, baselines, alignment, fallback,
  carets, axes, and subsets laying out like full fonts.
- `scripts/wasm-identity.mjs`: the wasm32 build of level 1 and level 2 over
  subsets under JavaScriptCore (Bun) and V8 (Electron 44): 5,200 of 5,200
  cases byte-identical to native for both levels.

`engine/crates/pm-bridge/scripts/fonts-smoke.mjs` checks the addon from
JavaScript; `apps/desktop/src/main/engine-fonts.test.ts` checks the main
wiring (and the real addon when it is built).

Recapture the fixture (`capture/run.ts`, then `pm-text-corpus fixture`) after
a macOS update changes the fonts; `tests/corpus.rs` detects font drift and
says so.
