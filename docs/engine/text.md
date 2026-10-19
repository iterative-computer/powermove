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
| `served` | `ServedResolver`: a resolver answered synchronously by a registry on another thread, over full fonts (`FontServer`, `Served`, `DirectServer`) |
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
   data by family, effective size, selection request (weight, width,
   slope), and the full `FontDescription::CacheKey` options, not by
   specified size. Those options include text rendering, optical sizing,
   synthesis permissions and synthetic flags, orientation, subpixel
   positioning, emoji variant, size adjustment, variation axes, palette,
   variant alternates and unique-face matching. macOS uses a constant
   device-scale component in this key. Kerning, feature settings and
   shaping variants were checked against Blink: they affect shaping but
   do not enter this platform-data cache key. Adding them here would
   incorrectly prevent Chromium's real sharing. Whoever creates an entry first fixes its
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
   family, weight, italic, effective size, full cache identity, and the
   specified size that created it), so level 1 can reproduce a renderer in that state. Without
   seeds pm-text measures as a fresh renderer. The corpus test seeds the
   capture's renderer from its device pixel ratio. Today's layout of such
   text therefore depends on the display and on what the UI drew before;
   the decision below picks the app's own state.

   Canvas-created entries matter on the 1/100 px grid too: a 30 px
   canvas created before 15 px UI text at 2x must keep `opsz` 30 for the
   shared 30 px key. Recording only the menu would instead use `opsz` 15
   and change the width visibly (roughly 3.7%).

   **Decided (Phase 1 T1, by the orchestrator under amendment 22): measure
   as the running app's renderer does.** The seeds are the font cache
   entries the window's own UI text has created (2026-10-18, third pass;
   the second pass modelled only `--fs-sm` and `--fs-md` at 12 and 13 px,
   which missed the density setting and every other UI size and weight):

   - **The UI font census** (`renderer/src/engine-doc/ui-font-census.ts`,
     started from `legacy/bootstrap.ts` with the new engine on) watches the
     document for text it lays out (elements with client rects holding
     non-blank text, form controls with a value or placeholder) and reports
     each new description (computed family stack, CSS size, weight, slope,
     with the device pixel ratio it was laid out at) on
     `engine-fonts:ui-fonts`. It rescans everything when the device pixel
     ratio changes (another display, page zoom: `devicePixelRatio`
     includes it) or the root's attributes change (`data-density`, theme),
     and otherwise only what mutations touched, when the renderer is idle.
     So a comfy window on a 2x display reports `--fs-md` at 14 px (the 28 px
     entry with `opsz` 14), and an open inspector or menu adds its 11, 15
     and 18 px text at weights 400, 500 and 600 as it appears.
   - **Main resolves each stack as Chromium does**
     (`shared/engine-fonts-seeds.ts` `uiFontSeed`): the first family that
     is a system font alias (`system-ui`, `-apple-system`,
     `BlinkMacSystemFont`, one cache key, as the real project's capture
     shows) creates the entry; an installed family before it (the
     registry's available family names, hidden `.`-prefixed ones excluded,
     fetched again when the registry's generation moves) or a named generic
     (`sans-serif`, `ui-monospace`, ...) means the text is not in the
     system font. `--f-ui` is `"SF Pro Text", -apple-system, ...`, so a user
     who installs SF Pro Text has no system font entries, as in Chromium.
   - **Each window keeps its entries in creation order**
     (`FontSeedTracker`): the first description to create a key (effective
     size, weight, slope and complete platform-data identity) fixes its `opsz`; a window keeps every display's
     entries; a closed window or a new renderer process
     (`render-process-gone`) forgets them.
   - pm-text matches a seed for any of the system font aliases
     (`blink::measure`, `same_cache_family`), so a canvas measuring
     `-apple-system` finds the entries too.
   - `app_ui_font_cache_seeds` (pm-text) and `appUiFontCacheSeeds` remain the
     startup model at the default density for tools with no window (the
     corpus test, `pm-cli parity`, `pm-cli render --ui-scale`).

   Main answers `engine-fonts:seeds` with the window's seeds and sends
   `engine-fonts:seeds-changed` when they change; the editor's pm-wasm
   takes them through `pm_fonts_seeds`. The client caches an accepted table
   only after the engine acknowledges it; an error reply rejects the update
   and session-owner assignment. Startup replies cannot overwrite a newer
   seeds-changed notification. Both engines' font services get
   every window's seeds (`Op::Seeds` as `{ seeds, owners: [{ owner, seeds
   }] }`, replayed last on every port). The round 4 typing measurement observed
   11 and 12 seeds in a fresh editor window, where the static model had 2. With complete identity,
   round 5 typing reports 18–19 distinct entries; the live comparison below
   checks their effective sizes, weights and creation orders.

   **Canvas creators (round 5).** Platform data is lazy: setting `font`
   alone need not realize it, and a context can change `textRendering` or
   stretch before measuring or drawing. The census wraps native
   `measureText`, `fillText` and `strokeText` on main-thread Canvas2D and
   OffscreenCanvas2D, keeps native parsing, and records the description
   actually used by that operation at DPR 1. Each context keeps its
   descriptions by state (round 8): font, text rendering and stretch, plus
   a style generation for a connected canvas, which every font-relevant
   mutation bumps, so its inherited style is read again only then. Pending
   DOM descriptions are drained first, but only before a description not
   yet recorded (one already recorded creates no cache entry); all
   wrappers are removed when the census stops. Pure transform
   and opacity updates do not trigger a tree walk; changes to cache-key
   properties do. Aggregate scan costs remain exposed through
   `EngineDoc.stats().fontCensus`.

   Round 5 closes the live-seed failure. Global UI text uses
   `optimizeLegibility`; Canvas2D defaults to `auto`. The previous reduced
   key conflated those entries, producing 6.45–40.26 px advance errors.
   Visible buttons and select labels instead use `auto` and legitimately
   seed the 24/26 px keys; retaining those entries preserves their sharing.
   Parked panels at -9999 px remain excluded, preserving the 24/560 fix.
   The native seed matcher accepts only a complete identity equal to the
   requested canvas description. Static captures without `cacheKey` keep
   their original default-canvas interpretation.

   The live e2e checks the census-derived seeds against the same page's
   Canvas2D and pm-wasm. Its 23 rows agree within 0.00001526 px, including
   20/400, 30/500/600, 24/560, valid 24/26 sharing, canvas-first 30/400 before
   an `auto` 15 px menu at 2x, and UI-first `auto` weights 450/650. The latter
   use distinct weights so the former startup failures remain independent
   probes. Primary sources: [Blink platform-data cache key](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/third_party/blink/renderer/platform/fonts/font_description.cc)
   and [Canvas font state](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/third_party/blink/renderer/modules/canvas/canvas2d/canvas_rendering_context_2d_state.cc).

   Remaining unmodelled entries also include pseudo-element text,
   font-cache purges under memory pressure and worker-owned canvases.
   Offscreen text scrolling into view without a DOM mutation is not yet
   observed. Arbitrary custom CSS still needs independent captures beyond
   the default and differing text-rendering identities. The in-app
   `native-engine-font-seeds.spec.ts` checks the census-derived seeds
   against this renderer's actual Canvas2D and pm-wasm advances, including
   the 30 px canvas before a visible 15 px menu at 2x (that first-creator
   check passes), UI-first weights 450/650, and the formerly poisoned
   descriptions even if the census leaves them out. It records fresh
   resolver diagnostics and can capture failures outside Git with
   `PM_FONT_SEEDS_CAPTURE`. Its failing rows must not be described as T1
   closure. Playback cost and the separate typing measurement are in
   `text-render.md`, round 4 verification.

   **Seeds per window (2026-10-18).** Each BrowserWindow is its own
   renderer with its own Chromium font cache, so the engines measure a
   layout with the seeds of the window it is drawn for, never the union:
   - main names the window of every renderer port when it connects it
     (`connect` carries `owner`, the webContents id; `EngineHost::attach`
     records it in the hub), so a view measures with its window's seeds;
   - the job engine's sessions are opened by main: `AgentJobs` tells the
     job engine's font client which window a session is for
     (`EngineFontClient::setSessionOwner`, the `sessions` list of
     `Op::Seeds`, job engine only) and waits for the engine to take it
     before submitting the job, so an export measures with the seeds of
     the window it was started from. A queued job keeps its starting
     window while it remains open (runtime-only ownership, never a
     persisted renderer id). If the engine does not take them
     within 5 s, once more; then the export does not start (an error to
     the window) rather than lay text out with every window's seeds;
   - a render thread's `ServedFonts` takes the scope of each layer
     (`Renderer::set_text_seed_scope`: the view's port in the presenter,
     the work's port and session in the job lanes) and resolves it port
     first, then session, then the fallback (every window's seeds, for
     work of no known window: queue jobs after their window closes or the app
     relaunches, `pm-cli`); it
     parks one measurer per seed set, so windows on different displays
     never rebuild each other's layouts.

   Measured (`pm-cli parity eval --wasm`, corpus v1, 2026-10-18): pm-wasm
   in Node with the seeds main sends gives 1,891,448 compared, 0
   mismatched with the static startup seeds, exactly as natively; this
   measurement did not validate the later census-derived seeds. Before,
   1,260 `system-ui` layouts of the
   real project differed (up to 18.2 px). Native and wasm32 replies: 0 of
   1,815,420 differ.

4. **Variable-font ink (Phase 1 T2, decided by the orchestrator).**
   CoreText instances variable outlines with its own variation arithmetic;
   skrifa's ink boxes differ by at most 1.19e-4 px (relative 4.3e-6) on the
   real project. That is a named bounds tolerance class, "variable-font ink
   (CoreText instances outlines)", bound 2e-4 px absolute: `pm-cli parity
   eval` counts values within it as matched in the class and reports them
   apart (`toleranceClasses` in the JSON; 405 text bounds and 405 raster
   selections, max 1.191e-4 px), never as mismatches or questions;
   pm-eval's corpus test uses the same bound (`VARIABLE_INK`). It covers
   bounds and raster boxes of text in a variable font only, never positions
   or advances.

## The engine's font service (Phase 2, "Use New Engine")

With "Use New Engine" on, the registry and subsetting run in the interactive
engine (`engine/crates/pm-node/src/fonts`), and the renderer's channels
below are unchanged. Main relays them:

- `installNativeEngine` registers the `engine-fonts:*` channels over an
  `EngineFontClient` and adds main as a client of every interactive launch
  with the `fonts` peer (`NativeEngine.addClient(FONT_CLIENT_ID, …, { peer:
  'fonts', processes: ['interactive'] })`), and a second client for the job
  engine (`JOB_FONT_CLIENT_ID`, "Fonts on render threads"). Main never loads
  a registry of its own on this path.
- The font port is not a protocol peer. Each message is one request or one
  reply (`fonts::wire`): `id u32 LE | op u8 | json length u32 LE | JSON |
  binary tail` and `id u32 LE | status u8 | payload`, with status JSON,
  bytes, or error. Registry answers stay pm-text's JSON strings end to end;
  fonts and subsets travel as raw bytes.
- The service is created with the engine host on the utility process's
  JavaScript thread (the process main thread, which AppKit's font manager
  needs), or on the first font port, and lives for the launch. Requests run synchronously on that
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

## Fonts on render threads (Phase 3c)

The engines draw text on render threads (the viewer presenter, the job
engine's export, agent frame, and snapshot lanes), but the registry runs on
the utility process's main thread (AppKit). Both meet in
`pm_node::fonts::hub::FontHub`:

- A render thread's renderer holds `pm_render::text::served::ServedFonts`:
  level 1 layout (`Compat`, `CompatMeasure`) over a `ServedResolver`. A
  query the resolver has not seen goes to the hub with its siblings (the
  other families of the canvas font list and the standard font at the same
  description, so an animated font size costs one round trip, not five);
  the hub queues it and wakes the JavaScript thread through the action sink
  (an empty action list makes `engine-process.ts` call `pump`).
- `EngineHost::pump` and `tick` drain the queue: `FontService::serve`
  answers each batch with the registry's `Answers` and the full bytes of
  every face named (shared `FontBytes`, no copy), so outlines, color glyphs,
  variable axes, hidden system fonts, and project fonts reach the renderer.
  The render thread waits (30 s at most once open); nothing is guessed: an
  unanswered query is a failure the layer reports (`Feature::Text`), never a
  layout in another font.
- Requests wait until main has finished its replay on a port: the open, the
  project fonts, then the seeds, which go last (20 s at most; a host that
  never opens a font port makes them fail). A frame is never laid out
  without the project fonts and seeds main is about to send.
- The hub's generation moves when the registry's does (project fonts,
  sources, a new generation base after a restart) or when the seeds change.
  `ServedFonts::sync`, called before each text layer, then starts over with
  a fresh resolver and measurer, and the generation enters every raster's
  content key. The text context drops every glyph cache then (outlines,
  color bitmaps, CoreGraphics masks, path masks and their CTFonts), and
  every glyph cache keys a face by `FontBytes::id`, a process-unique id
  that new bytes never reuse (never by the buffer's address, which a
  replaced font of the same size can land on).
- `EngineHost` drops the hub's waits before the presenter and job lanes are
  joined (shutdown never hangs on a render thread waiting for fonts).
- Only a host whose registry starts (macOS, on the main thread) gives its
  render threads fonts; elsewhere text stays unsupported, as before.

Both engines host a registry now. Main keeps one `EngineFontClient` per
engine: the job engine's (`JOB_FONT_CLIENT_ID`) mirrors every project font
change and the seeds of the interactive one's, joining its replay directly
while the job engine is down, so a restarted job engine replays them too.

`pm-cli` renders on its main thread: `pm_cli::fonts::install` serves its
renderer from a `DirectServer` over the CoreText registry, with the seeds of
an editor at `--ui-scale` (default 2) on `render` and `scope`.

Tests: `cargo test -p pm-node --test fonts` (the hub, on the main thread:
three layouts on a render thread equal the registry's glyph for glyph,
including the seeded `system-ui` case, a mixed-script line, and a hidden
system font; a whole frame rendered on a render thread equals one rendered
on the main thread; a project font changes the frame through the
generation; new seeds move the generation; a 1x editor window and a 2x
popout lay `system-ui` 24 px out differently, each view and the 1x
window's export with its own window's seeds, switching without rebuilds;
a restarted engine with main's replay lays text out the same; shutdown
ends a waiting render thread), the hub's and `served`'s unit tests (seeds
by port, session and fallback; a `DirectServer` never drops its registry
off its thread), and main's (`engine-fonts.test.ts`: seeds replayed last,
mirrors while down, session owners after the next replay;
`engine-fonts-seeds.test.ts`: censuses in, installed fonts and closed or
restarted renderers; `shared/engine-fonts-seeds.test.ts`: stack resolution
and creation order; `renderer/src/engine-doc/ui-font-census.test.ts`: what
the census reports, density changes;
`native-engine.test.ts`: connects name the window; `agent-jobs.test.ts`:
the session's window before the job).

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

### Level 2 text semantics (textl2)

`pm_eval::text_level2` evaluates range typography, Parley layout, selector
weights, shaped glyph poses and typography anchors as one frame. Rendering,
`textRaster`, bounds, picks, glyph units and the editor's caret layout all
consume it. Level 1 retains its original measurement and raster paths.

`TextMeasure::level2` resolves base and range typography through the existing
registry/replay boundary and resolves the needed script fallback faces before
building a Parley collection. Collections split into single-face files once
per font identity. A missing named family uses the registry's system UI
fallback, as the original level 2 manifest does. Explicit variable sources
are retained. Production served fonts use this path; it does not require the
test-only eager `Level2Fonts` collection. Existing host font-generation
rebuilds invalidate the split-face cache.

Ranges retain UTF-16 indices and authored overlap order, snap to grapheme
starts, and affect shaping, wrapping and metric baselines (including spaces).
Parley records each shaped glyph's source. Ligatures remain shaped; their
glyphs follow the first logical grapheme's pose. Selectors retain stagger,
range and wave modes, character/word/line units, ordering, easing and split
offsets. Equal poses share one paint operation. Typography anchoring uses
resting styled bounds, so it never cancels animator movement. Bounds and
picks follow posed glyphs and blur reach.

Cold WASM queries return `fonts` before reporting a failed level 2 shape,
allowing the normal font handshake to converge. No provisional geometry is
returned. Carets use Parley's UTF-16 boundaries and metric baselines. The
sanitizer preserves explicit layer `compat` 1/2 without adding it to old layers.

Eight `text-level2/*` corpus projects share a synthetic fixture with the
native semantic gate and native-on app e2e. The semantic gate invokes the
actual `support::routing_forbidden` predicate and refuses any escape. Level 2
uses its CoreText comparison and semantic invariants; the level 1 Canvas2D
references and FONTS-R fixture remain unchanged.
Generate their standalone corpus with
`bun engine/corpus/src/generate.ts --native-text --out <outside-repo-dir>`.
`textLevel2Projects` is the native-reference catalog; `allProjects` is the
legacy-reference catalog. All eight native projects run unconditionally in
`text_level2_semantics`, without private captures or a routing exemption.

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

## Round 6 integration

Canonical glyph paths and animator image filters now reuse pm-vector's
Canvas2D renderer and Graphite filter executor; the source trace and raster
results are in `text-render.md`, “Round 6”. Font selection and the full
platform-cache identity retain the round 5 behavior. With the document-host
integration, attaching or reviving a pm-wasm instance reapplies the latest
received window seeds after synchronizing the document and before declaring
it ready. An initial reply or notification arriving later still applies
the seeds and invalidates cached answers through `FontLoader.applySeeds`.
The paired typing harness remains committed and documented for the
performance lane; round 6 does not attempt an edit-to-present optimization.

The synchronous whole-face font provider carries the same initial seed
callback and seeds-changed subscription as the asynchronous provider.
The document-host integration introduced the synchronous branch; dropping
these callbacks left the first 26 px/400 layout at opsz 26 instead of the
window's seeded opsz 13. The provider regression test checks the initial
application and a later notification against the actual pm-wasm wrapper.

The fresh round 6 app matrix passes 22/22. Its live seed comparison includes
23 rows and the first 26 px/400 query, with maximum native/Canvas2D width
error 0.00001525879 px (unchanged bound 0.001 px). This validates the
census-derived table as applied through the synchronous provider, in
addition to the earlier static-startup parity run.

## Round 7 gate policy

Orchestrator decision **FONTS-R** (2026-10-19) places the 23 measured Round 6
pixel residuals in an explicit downward-only ratchet; see `text-render.md`,
“Round 7”. The ratchet leaves font selection, layout and the existing
1-CV raster / 2-CV composition bounds unchanged. Unlisted failures and stale budgets after
an improvement fail. The textexact follow-up owns the CPU AAA/SkStroke,
Graphite stroke tessellation and Fontations COLR ports. The ratchet changes
gate policy only; no glyph-rasterization port is started in this round.

The bridge integration retains the scoped served-font provider for views
and snapshots. Main-owned fallback snapshot sessions bind to their window's
font seeds before resync, fail on a rejected binding and clear stale bindings
before session ids are reused after a restart. Per-capture caption visibility
is carried through the typed protocol and restored for later captures and
viewer frames. See `text-render.md` and RFC 16. The fresh Round 7 app matrix
passes 22/22; all 23 measured residuals match their FONTS-R budgets in debug
and release, with every other case on its original strict bound.
