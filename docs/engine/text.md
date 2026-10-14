# pm-text and the font registry

Design notes for `engine/crates/pm-text`, the font registry export in
`engine/crates/pm-bridge`, the interactive engine's font service
(`engine/crates/pm-node/src/fonts`), and their wiring in Electron main
(`apps/desktop/src/main/engine-fonts.ts`). Plan: section 12, with section 26
amendments 10 and 19 and the Phase 2 item "Font registry and subsetting move
into the engine". Spike: `docs/engine/spikes/S4.md`.

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

## Blink's font sizes (amendment 19)

Level 1 reproduces three details of how Chromium sizes fonts on macOS. Each
was found on the corpus's real project and checked against Chromium itself
(`tests/blink_sizes.rs`: 46 widths of `H` and `i` in `system-ui` from 11 to
40 px, measured in Chrome for Testing 151, equal bit for bit).

1. **Effective and specified size.** Blink creates, scales, and measures a
   font at `FontDescription::EffectiveFontSize`, `floorf(size × 100) / 100`
   in `float`. Two things still see the specified size (the size as given):
   HarfBuzz's `trak` lookup (`HarfBuzzFace::GetScaledFont` sets `ptem` to it)
   and `opsz` (`FontPlatformDataFromCTFont` and `FontCustomPlatformData` set
   it to the specified size). `canvas_font` applies the floor once and keeps
   the specified size (`CanvasFont::specified`, `FontSetup::specified`). The
   real project's `system-ui` at 11.8675 px measures `H` at 8.8115234375
   px; at 11.86 px it is 8.812057495.
2. **HarfBuzz advance scaling.** For fonts with `trak` Blink takes advances
   from its unscaled parent `hb_ot` font, and HarfBuzz converts them to the
   sub-font's scale with `parent_scale_x_distance`, `v × x_scale /
   parent_x_scale` in `int64_t`: a truncating division, not `em_mult`'s
   rounding. At fractional sizes that is one 16.16 unit less about half the
   time (`BlinkFuncs::harfbuzz_advance`).
3. **The renderer's font cache.** `FontPlatformDataCache` keys platform
   data by family, effective size, and selection request (weight, width,
   slope), not by specified size. Whoever creates an entry first fixes its
   `opsz` for every later user in that renderer. DOM text is laid out at its
   CSS size times the zoom, and on macOS the device scale factor applies as
   zoom: UI text in `system-ui` at 12 px on a 2x display creates the 24 px
   entry with `opsz` 12 (clamped to SF's axis minimum, 17). A canvas that
   then measures 24 px `system-ui` gets SF's text-size advances, about 3.7%
   wider than at `opsz` 24 (`1`: 11.203 px instead of 10.805). That is why
   the real project's 24 and 26 px `system-ui` text measured wider in the
   capture: the app's UI had laid out `system-ui` text at `--fs-sm` (12 px)
   and `--fs-md` (13 px). Headless Chromium reproduces the capture bit for
   bit with 12 and 13 px `system-ui` DOM text at a scale factor of 2, and
   gives pm-text's fresh-renderer answer without that text or at a scale
   factor of 1.

   `Compat::set_font_cache_seeds` declares such entries (`FontCacheSeed`:
   family, weight, italic, effective size, and the specified size that
   created it), so level 1 can reproduce a renderer in that state. Without
   seeds pm-text measures as a fresh renderer. The corpus test seeds the
   capture's renderer from its device pixel ratio. Today's layout of such
   text therefore depends on the display and on what the UI drew before;
   the engine has to pick one answer for documents (open question for Jude:
   fresh renderer, or the app's own UI state at 2x).

   Not modelled: when a canvas itself first creates an entry at a specified
   size off the 1/100 px grid between 17 and 28 px, later canvas text in the
   same 1/100 px bucket inherits that `opsz` (a difference below 0.01 px of
   optical size).

## The engine's font service (Phase 2, "Use New Engine")

With "Use New Engine" on, the registry and subsetting run in the interactive
engine (`engine/crates/pm-node/src/fonts`), and the renderer's channels
below are unchanged. Main relays them:

- `installNativeEngine` registers the `engine-fonts:*` channels over an
  `EngineFontClient` and adds main as a client of every interactive launch
  with the `fonts` peer (`NativeEngine.addClient(FONT_CLIENT_ID, …, { peer:
  'fonts', processes: ['interactive'] })`). The job engine gets no font
  port. Main never loads a registry of its own on this path.
- The font port is not a protocol peer. Each message is one request or one
  reply (`fonts::wire`): `id u32 LE | op u8 | json length u32 LE | JSON |
  binary tail` and `id u32 LE | status u8 | payload`, with status JSON,
  bytes, or error. Registry answers stay pm-text's JSON strings end to end;
  fonts and subsets travel as raw bytes.
- The service is created on the first font port, on the utility process's
  JavaScript thread (the process main thread, which AppKit's font manager
  needs), and lives for the launch. Requests run synchronously on that
  thread, the one that also runs the engine protocol, so a slow request
  holds the engine's message handling: the first fallback manifest takes
  about 0.6 s. Only CoreText matching needs AppKit's thread; moving the
  manifest and subsetting off it is open work.
- Restarts: every new port opens with a generation base above every
  generation main has seen, then main replays the project fonts and
  variable sources in order. A renderer therefore always sees a new
  generation after a restart and starts its answers again. Read-only
  requests in flight when a port dies are sent again on the next one;
  requests made while the engine is down wait for it (30 s) and go out
  after the replay, in order.
- The replay is exactly the project font changes that went out on a port:
  a change joins it when it is sent and a clear empties it when it is sent,
  so a font added while the engine starts reaches it once. A change leaves
  the replay only when the engine refuses it, also after its caller timed
  out (the engine may still apply a request that went out; its late reply
  decides).
- A renderer that asks under an older generation (`known`) gets the queries
  it can still ask answered and the new generation, never an error for face
  ids from an earlier launch.
- For the engine's own text rendering, `FontService::line_runs` lays text
  out over the same registry and `FontService::face` gives the full font
  bytes (outlines). `tests/fonts.rs` checks that pm-wasm's replay over the
  served subsets lays text out glyph for glyph like the engine over the full
  fonts.

The page reaches the channels through `window.powermove.engineFonts`
(preload), which pm-wasm's font loop uses as its provider.

Measured through the real utility process
(`node apps/desktop/scripts/engine-fonts-integration.mjs`, M4 Max, macOS
27.0, Electron 44, 2026-10-13): `answer` for 3 families (Helvetica, Menlo,
`system-ui`) 3.6 ms from the channel; their 3 subsets, 951 KB, 1.6 ms; the
`system-ui` fallback manifest and Helvetica's faces 0.58 s (the first
manifest loads every CoreText substitute); after SIGKILL the restarted
engine had a new font port, the replayed project font, and a higher
generation 0.31 s later.

## Main's registry ("Use New Engine" off)

`POWERMOVE_ENGINE_FONTS=1` turns it on. Main loads pm-bridge's
`FontRegistry` from `engine/target/pm-bridge/pm_bridge.node`
(`engine/crates/pm-bridge/scripts/build.sh`) or `POWERMOVE_PM_BRIDGE`, and
registers these channels (`shared/engine-fonts-ipc.ts`; trusted editor
senders only; every reply is `{ ok: true, value } | { ok: false, error }`):

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
  within 0.25 px (regression sentinel 0.01 px). Measured (2026-10-13):
  max error 0.001047 px, p99 0.000172 px (p99 was 0.00026 px before the
  HarfBuzz advance scaling fix). Replays over full fonts and over subsets are
  bit-identical to the native run; subsets are 2.92% of their fonts
  (590.7 MB to 17.23 MB over 137 faces) and must stay under 5%.
- `tests/registry.rs`: enumeration, project fonts, variable sources, the
  manifest, family faces, subset tables, and the incremental protocol.
- `tests/blink_sizes.rs`: effective and specified sizes, HarfBuzz advance
  scaling, and font cache seeds against widths measured in Chromium.
  `scripts/blink-sizes.mjs` measures them again in Playwright's Chromium
  (a fresh renderer, and one that laid 12 and 13 px `system-ui` DOM text out
  under `--force-device-scale-factor=2`) and compares them with the test:
  54 of 54 equal in Chromium 151.0.7922.34 (2026-10-14).
- `tests/level2.rs`: level 2 breaking, baselines, alignment, fallback,
  carets, axes, and subsets laying out like full fonts.
- `scripts/wasm-identity.mjs`: the wasm32 build of level 1 and level 2 over
  subsets under JavaScriptCore (Bun) and V8 (Electron 44): 5,200 of 5,200
  cases byte-identical to native for both levels.

`engine/crates/pm-bridge/scripts/fonts-smoke.mjs` checks the addon from
JavaScript; `apps/desktop/src/main/engine-fonts.test.ts` checks the main
wiring (both providers, the font client's queueing, restarts, and replay,
and the real addon when it is built). The engine's font service:
`cargo test -p pm-node` (`tests/fonts.rs` and the wire format) and
`node apps/desktop/scripts/engine-fonts-integration.mjs` (the real utility
process; wrap it in `pm-electron-lock`).

Recapture the fixture (`capture/run.ts`, then `pm-text-corpus fixture`) after
a macOS update changes the fonts; `tests/corpus.rs` detects font drift and
says so.
