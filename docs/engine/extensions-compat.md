# Extension compatibility on the engine (amendments 23 and 24)

Design notes and status of the extension compatibility lane (Phase 3c,
survey lane L4): the section 20 API with "Use New Engine" on, the
deprecated compatibility members and their warnings, the `raster` override
and the compatibility context, today's document values for apiVersion 1 to
3, and the customization corpus. Branch `feat/ne-extcompat` (on
`feat/native-engine` with `feat/ne-bridge` merged).

## Where the code is

| Part | Code |
|---|---|
| Deprecated members, their replacements, the source analysis | `apps/desktop/src/shared/extension-analysis.ts` |
| Warnings returned to agents | `main/codex/validate-staged-extensions.ts` (every provider), `main/compatible-workspace.ts` (read, write, compile, finish), `main/agent-tools/external-session.ts` (`publish_extensions`), `main/cloud/publish.ts` (Store prepare: `deprecationFindings`), `shared/agent-guidance.ts` (the verification continuation) |
| Typings and named replacements | `renderer/src/kernel/api.ts`; table and guidance in `apps/desktop/docs/EXTENSIONS.md` (a test keeps the table equal to the module) |
| `media.assets.source` and `audio` | `renderer/src/kernel/live-media.ts` |
| The raster override as the engine's producer | `renderer/src/kernel/raster-compat.ts`; transport in `engine-connection/render-transport.ts` (`answerCompat`, `receiveCompat`, snapshot rounds, export asks) |
| Engine: drawing compatibility rasters | `engine/crates/pm-render/src/compat.rs`, the hook in `plan.rs` (`compat_quad`), upload in `exec.rs` |
| Engine: answers per session | `engine/crates/pm-node/src/present/compat.rs` (`CompatStore` answers, `CompatView`), `present/render.rs` (viewer frames and snapshots) |
| Engine: exports | `engine/crates/pm-node/src/jobs/compat.rs` (`JobCompat` over `Request::CompatFrame`), main's `engine/compat-frames.ts` |
| Protocol | `pm_protocol::snapshot::CompatMessage` (`Producers`, `Native`, `Reuse`, `Needs`), `CompatPixels.size/uv`, `CompatFrame.native/size/uv`, `DeliveryOptions.compat`; this round `CompatNeed/CompatNative/CompatPixels/Reuse.crop` and `CompatFrameRequest.frame` (`CompatFrameInput`), PROTOCOL_BUILD `protocol-d1e0c26ccb949da7` |
| Today's preview crop | `engine/crates/pm-render/src/compat_crop.rs`, `plan.rs` `preview_crop`, `renderer.rs` `preview_window` |
| Export frames through the context | `pm-export` `render::FramePost` (`ExportPlan::frame_post`), `pm-node` `jobs/compat.rs` `ContextFrames`, `kernel/compat-context.ts` `exportFrame`, `kernel/install.ts` (the `compatContext` producer) |
| `render.gl.canvas`, the compatibility context | `renderer/src/kernel/compat-context.ts` (`CompatContextHost`), wired in `kernel/install.ts` (hears `preview:presented` before extensions hear `frame:rendered`) |
| Export paths carrying producers | sheet and queue: `legacy/core/exporter.ts` `sheetOptions`; `export_video`: `legacy/assistant/mcp-project-tools.ts` (`__mcp_context.compat`) → `main/agent-tools/bridge.ts` → `main/engine/agent-jobs.ts` (`exportSettingsFor`); the job's snapshot layer: `AgentJobs.compatLayer` → `compat-frames.ts` (`EngineCompatAsk.layer`); main's own snapshot sessions: `EngineSnapshotRequest.compatEvents` → `window-sessions.ts` |
| Today's export format ids (issue P1) | `shared/delivery.ts` (`storedFormatId`), `core/export-defaults.ts` (`storedExportDefaults`) |
| Corpus | `apps/desktop/e2e/fixtures/compat-corpus/` (shipped built-ins, API probe, sandbox probe, override-builtins, shape tint), `e2e/engine-customization-corpus.spec.ts`, `e2e/engine-extension-compat.spec.ts`, `e2e/engine-compat-context.spec.ts`, `main/extensions/rebase-corpus.test.ts` |

## The raster override

Today the compositor calls `PM.raster(L, continuousRasterScale(…), T,
uploaded, crop)` for every text, captions, and shape layer, and draws
whatever the wrapper chain returns on the quad it describes. The engine
reproduces that contract:

1. While a wrapper is registered (`services.onChange('raster')`), the window
   tells each engine session it produces `rasterOverride`
   (`CompatMessage::Producers`). A chain change or a font change re-sends the
   list through empty, which drops every earlier answer.
2. With a producer, `pm_render::plan` asks for every text, captions, and
   shape layer that is not `compat` level 2, at the layer's own time and
   `continuousRasterScale(scaledWorld(L, T, W, H))`
   (`CompatRasters::answer`). An answer counts when it was made at the
   frame's revision or a newer one.
3. A frame without every answer fails with `RenderError::CompatPending`. The
   presenter sends the needs to the view's renderer (`Needs`; asked again
   after a second, or at once when a `Reuse` named a raster the store
   evicted meanwhile: `Applied::Evicted`), and draws the frame when they
   arrive; a snapshot answers
   `needsRenderer.compat`, and the renderer draws the needs on its blocked
   thread, posts them on the session's port, and asks again (the job waits
   for answers already on their way, as for fallback pixels).
4. The renderer runs the chain at that time and density over a **lazy**
   engine raster (`engineRasterSync`, made on first field read). The
   wrapper's own base object back means pass-through: `Native`, no pixels,
   no round trip. Any other result is read back as the compositor uploads a
   canvas (a WebGL2 texture with `UNPACK_PREMULTIPLY_ALPHA_WEBGL`, then
   `readPixels`) and posted with its `w`, `h`, anchor (shapes without paths
   anchored at their center, as `nativeContentQuad` does) and `uv`. An
   unchanged cache `key` is answered `Reuse`; a need asked again at the same
   revision after a `Reuse` is answered with fresh pixels. A captions
   layer's base is not lazy: between cues it is null, as today. A wrapper
   that throws fails the answer: today the exception left the compositor's
   draw and failed the frame (`legacy/core/engine.ts` reports it once per two
   seconds: `console.error('[engine] frame render failed', …)` and a toast);
   the engine's frame keeps waiting (the last frame stays) and the window
   reports it the same way; an export fails with the message. The plain
   layer is never drawn in its place.
5. pm-render draws a raster as a canvas backing store: premultiplied E
   values, top row first, on `Quad::Texture` through the layer's transform,
   effects, masks, mattes and blending (`Source::Compat`).
6. Exports name the window's producers (`DeliveryOptions.compat`): the
   export sheet and the render queue (`sheetOptions`), and `export_video`
   (the window's `__mcp_context` names them). The job asks the host for each
   such layer at each frame (`Request::CompatFrame`, blocking: an export has
   no per-frame deadline). Main finds the job by the asking session and sends
   the layer as the job's snapshot holds it (`EngineCompatAsk.layer`); every
   editor window with the producer may answer, and draws that layer (the
   live one when it is the same, so wrappers keep their caches; otherwise a
   copy of the snapshot's), so a render queue job draws the project as it
   was queued, as today's in-window queue rendered its stored snapshot. With
   no window answering, the export fails with the host's reason; it never
   draws the layer without its override.
7. A sync snapshot main serves from its own session for a window (at boot,
   or while a staged resync is on its way) carries the window's producers
   and, after a `needsRenderer` round, the window's answers
   (`EngineSnapshotRequest.compatEvents`); main posts them on its session
   before the snapshot, as the window's own session gets them on its port.

A wrapper that calls `render.snapshot` from inside its own answer gets `''`
(its frame would need the answer being made); today's compositor recursed.

## The compatibility context (`render.gl.context`)

Today it is the compositor's WebGL2 context (`GL.init`: no alpha,
premultiplied, `preserveDrawingBuffer`, no antialiasing) on the canvas a
viewer passed to `render.gl.init`. What extensions get from it:

| Today | With the engine (`CompatContextHost`) |
|---|---|
| null until `init` | the same |
| `context.canvas` is the bound canvas | the same (an own property of the context; the offscreen canvas is `context.compatCanvas`) |
| the default framebuffer holds the presented frame (cleared to opaque black, the composite drawn at the canvas size), so `readPixels` reads the composite | while a compatibility context exists, completed engine surfaces are copied before the presenter releases their slots; the public presentation event selects the exact `(revision, viewSeq, seq)` copy (at most eight are retained). Before the first read (`readPixels`, `copyTex(Sub)Image2D/3D`, `blitFramebuffer` from it) or draw after a presented frame, its region under the bound canvas is drawn over opaque black into a scratch canvas at the bound canvas's backing size (pixel for pixel at equal density, from the nearest whole-pixel origin) and blitted in, bottom row first. The extension's GL state (bindings, unpack flags, scissor, rasterizer discard) is restored |
| drawing into the default framebuffer shows in the viewer until the next frame clears it | after a draw call, clear or blit into the default framebuffer, the offscreen canvas is copied (after the task) to a layer canvas inserted right after the engine's canvas (under the viewer's overlay) at the bound canvas's box; the next presented frame hides it. A `frame:rendered` handler draws on the new frame: the host hears `preview:presented` first |
| WebM exports include it: the shipped exporter's `exportWebCodecs` and `exportRecorder` render each frame with `GL.render` into that canvas (`renderInto`), which emits `preview:presented` → `frame:rendered`, then encode the canvas. MP4 and ProRes (`exportNative`), PNG sequences, stills and captures read their own pass (`GL.renderToPixels`) and never include it | the same, per frame: see "Exports" below |
| `context.canvas` holds the composite and the drawing (`preserveDrawingBuffer`): `drawImage`, `toDataURL`, `toBlob`, texture uploads read them | the bound canvas gets the same pixels: each presented frame is copied into it (under the bound canvas, over opaque black), then any drawing over it. It rests hidden under the presentation. A bound canvas without a context gets a 2D one; one that has today's compositor context (kept while the legacy compositor runs beside the engine) is written through that context with its state restored |
| its own textures, framebuffers, programs | the same (an ordinary WebGL2 context) |

Limits: it is not the context that draws the frame, so GL state an extension
sets cannot affect the engine's drawing (it could disturb the compositor's
today); the frame read back is the presented frame (an `rgbaf16` EDR canvas is read as 8-bit
sRGB, as today's 8-bit canvas held it); a lost offscreen context is made
again on the next access (a new object, as `GL.init` after a restore).
Drawing is shown in the viewer's DOM rather than sent through the engine:
today's result is the canvas's own contents, shown at once and replaced by
the next frame, which a round trip through the engine would show a frame
late.

### Exports

A WebM export (the sheet's `webm`, `webm-vp9`, `webm-vp8`, or the render
queue) names the `compatContext` producer while an extension holds the
context (`EngineRenderTransport.exportSources`). The job engine renders each
frame, turns it into what today's canvas held (`output_rgba` premultiplied,
rounded to 8 bits, over black, opaque) and sends it to the window with
`Request::CompatFrame` (`CompatFrameRequest.frame`, a shared segment that
main inlines for the window). The window's `CompatContextHost.exportFrame`
sizes the context to the export, makes the frame its default framebuffer's
content, emits `frame:rendered` (`time` the frame's, `quality` 1, the last
presented `viewport`, as `renderInto` emitted them), and answers what the
default framebuffer holds after the handlers when one drew into it, or a
pass-through. The job encodes the answer (decoded back to working values) in
place of its frame (`pm_export::render::ExportPlan::frame_post`). Frames
nobody drew on keep the engine's precision. The drawing is not shown in the
viewer during the export (today's viewer showed the export frames, resized,
while it ran). `frame:rendered` is emitted for export frames only when an
extension holds the context; without one no pixels can differ, and today's
WebM exports emitted it for every frame.

### Crop

In a viewer frame of the open root composition, today's compositor passes
`PM.raster` a crop window for a large text raster (at least 4M pixels, the
visible part at most half, axes aligned: `previewTextRaster`) or a partly
visible shape without paths (`previewShapeRaster`). It does not call the
raster for a shape the viewport misses, and it fills the viewport with a
plain rectangle's color when the viewport lies inside its fill. A wrapper sees
those calls, and the typewriter fixture sizes its canvas from what it gets.
`pm_render::compat_crop` ports the three functions; `compat_quad` plans the
same decision from the request's region (viewer frames only: not exports,
not snapshots), for top-level layers of the root composition without paths,
renderable effects, masks, track matte or transitions (and, for text,
without animators, styles or typography anchoring). The window travels with
the need (`CompatNeed.crop`; `Native`, `Reuse` and `Pixels` carry it back) and
is part of the answer's identity in the store and in the snapshot job's tags.
The producer calls the chain with it, the base is the engine's raster of
that window (`sourceWindow` for text, `uv` for shapes, as today), and a
text result's `sourceWindow` is mapped onto the upload as the draw-window
program samples it (`resultUv`). Not ported: 3D layers' plane windows (they
get the whole raster) and the occlusion-culling scissor regions today draws
under an opaque rectangle (each region a separate crop). The pixels a
well-behaved wrapper returns are the same either way; a wrapper whose output
depends on the window could differ there.

### `PM.raster` assigned through `host.pm`

Before the services registry, extensions wrapped the raster by assigning
`PM.raster` (the C1 corpus's `private-extension-38` shifts the anchors of text
with animated font axes that way). Today the compositor calls whatever
`PM.raster` holds, so such a wrapper is outermost, above the registered ones.
`PM.raster` is now an accessor (reads and writes behave as the data
property did). An assigned function is the outermost link of the producer's
chain, and assigning or restoring it updates the producer like a services
change.

### Shader layers' program keys

Today's compositor set every drawn shader layer's `shaderKey` to
`sh:<id>:<hashStr(code)>`, and `render.gl.compileError(shaderKey)` is the
compiler's message or null. `EngineShaderMeta` sets the same key for each
shader layer on every presented frame and files a `shaderCompile` report
under it. A layer the WebGL fallback draws, which compiles today and cannot
be exported headless, keeps the engine's `engine:<id>` key and notice (shaders
lane).

### Playback

A frame whose compatibility answers are missing is not drawn: the presenter
draws the clock's frame when they arrive (`present/render.rs`, `CompatPending`),
and during playback the clock has moved on by then. Measured before the
lookahead: 2 frames presented in 3 s of 30 fps playback with a shape
override, against today's 90. So the window answers ahead
(`render-transport.ts` `lookahead`, `presentedWhilePlaying`): each layer the
engine asked for is answered for the next `COMPAT_LOOKAHEAD` (6) frames of
the open composition, starting at Play from the layers asked for while
paused, and one more per presented frame, within 6 ms of the main thread per
call. A nested or retimed layer whose time runs at another rate gets its
answers ahead at the open composition's frames, so its frames that miss are
asked for as before.

### Earlier measurements (2026-10-19, before round-three integration)

Playback (`engine-extension-compat.spec.ts`, a shape override, 3 s at 30
fps, against today's app with the flag off on the same project): 90 frames
presented, longest gap 42 ms, p95 34 ms; today 90, 41 ms, 34 ms. 192
answers were made ahead and no frame was asked for during playback.

`e2e/engine-compat-context.spec.ts` (the built-in viewer, 1464 × 824):
the context reads back the frame on screen exactly (`max 0` against the
engine's canvas region); against today's compositor on the same canvas, max
1 code value, 0 of 3,619,008 channel values over 1; a red square drawn into
it shows as the layer above the engine's canvas at the bound canvas's box,
equal to today's canvas with the same drawing (max 1), and is gone after the
next frame; `render.renderFrameTo` never includes it.

`e2e/engine-extension-compat.spec.ts`:

- Typewriter fixture (apiVersion 1), a 150% bold title keyed 0 → 100% and a
  plain 32 px layer: the typed band against today's compositor max 1 code
  value at 0%, 40%, 100% (ink pixels 0 / 2261 / 6869 on both); 8 needs
  answered for three snapshots; viewer frames asked 6 times in 1.5 s; after
  disposal the plain title draws again. The base raster geometry equals
  today's exactly (`w` 310, `h` 140, canvas 465 × 210, anchor 155, 48). The
  test then fails on the unaffected plain text (below).
- Shape override export (PNG sequence, 90 frames): 180 asks (2 layers × 90
  frames). Against today's own PNG export (a flag-off app, the same
  extension): every pixel equal except one, (195, 110), inside the plain
  blue rectangle the engine draws itself: today (46, 89, 169), the engine
  (51, 102, 204), 35 code values. Today's export of the same project with no
  extension has the same pixel, so it is today's drawing of a plain shape,
  not the override (vector/composite lanes; the gate fails on it).
  Side by side: `$TMPDIR/ne-extcompat-evidence/shape-export-engine-left-today-right-frame45.png`
  and the 8× crop `shape-export-zoom-195-110-engine-left-today-right.png`.
- Render queue: a job queued with a raster override, the project then edited
  (the tinted ellipse narrowed and moved), draws the project as queued:
  equal to the export sheet's engine export of the queued state at frames 0,
  45 and 89 (max 0), 180 asks.

### Measured (2026-10-19, round three, e2e under the Electron lock, flag on)

After merging `feat/native-engine` at 4bbb3e12 and `feat/ne-bridge` at
f0a927e3 (PROTOCOL_BUILD `protocol-243ac2c6ec3d6e99`):

- WebM export with a `render.gl.context` drawing (`engine-compat-moat`,
  fixture `context-watermark`, 320 × 180, 30 frames): the engine asked the
  window for all 30 frames, and the red mark is in the first and last frame
  (mean 232, 32, 42). The MP4 export asked 0 times and has no mark; neither
  does today's MP4. This branch's flag-off WebM has no mark (unreleased
  aad31a41 `exporting: true`), while the shipped app's does. Both are
  reported, and the gate asserts the shipped reference.
  `drawImage(context.canvas)` right after drawing reads the mark
  (232, 32, 42) and the frame's center (51, 204, 102) on the engine,
  exactly as on today's app.
- `PM.raster` assigned through `host.pm` (`legacy-raster-assign`): the
  assignment is the producer's chain (12 needs, 8 rasters answered), and the
  red ellipse and the plain rectangle equal today's exactly (max 0). The
  shifted text is at today's place. Its glyph edges differ by up to 86 code
  values (1665 of 144,000 values over 1): text rasterization, the
  text-render lane's gap. Side by side:
  `$TMPDIR/ne-extcompat-evidence/legacy-raster-assign-engine-left-today-right.png`.
  Before this round's stride fix the same text was streaked.
- The rebased shipped viewer fork reloads healthy, keeps the user's change,
  no longer reads `render.gl.context`, and presents the engine's frames.
  The update had been rolled back by the sandbox check until the fix in
  `shell/update-fork.ts`.
- API probe (v1, v2, v3 trusted, v3 full-access Store): every member
  passes except `renderFrameTo` over the whole frame. Its title rows differ
  by up to 30 code values (911 of 28,800 values over 1): text-render lane.
  The rows above and below are within section 25 (max 0 and 2). The
  stronger checks pass: the shader layers' `sh:` keys and compile errors, the
  waveform's peaks (max difference 7.6e-6) and pixels (max 0), the
  expression message (today's evaluator gives the same text), the fonts
  registry, the viewer and shaderHooks services, the scene3d round trips,
  and `render.raster`'s shape pixels with the text ink at today's place.
- Playback with a shape override, measured alone twice: 91 frames, longest
  gap 42 and 43 ms (today 90 frames, 35 and 41 ms); within the bound
  (today's gap plus one frame). In the full suite run, under load average
  about 9, it measured 88 frames and 79 ms against today's 91 and 42, and
  failed the bound.
- The round-three diagnosis of the zoom gate was wrong. The engine did
  zoom, but protocol validation rejected raster densities above 16, leaving
  the previous frame on screen without a failure report. Raster density is
  now bounded at today's 32 independently of output-frame scaling; the
  layer-snapshot and export-answer validators agree. Bad densities produce
  `invalidMessage` in `EngineConnection.status().failures` and the viewer's
  report line. The old hidden legacy-window comparison was not a zoomed
  reference. The replacement gate drives the engine view, checks advancing
  frames and seeks, and compares stable-view crop windows with the shipped
  TypeScript crop planner. It explicitly requests a 16x engine view in
  addition to the built-in menu's 8x maximum.
- The stronger crop gate found a second defect: `at_root()` excluded the
  canonical open composition that real app sessions send. Both the preview
  request and the raster planner now accept `is_open_canonical()`; nested
  precompositions remain excluded. A renderer regression checks stored and
  canonical documents against the same legacy-computed crop.

### The corpus

`e2e/engine-customization-corpus.spec.ts`:

- Forks of every shipped built-in (`fixtures/compat-corpus/shipped`, and
  timeline written from git history, `main` at 470eeaf3, at test time: its
  `timeline.ts` trips gitleaks' reviewed false positive), laid out as
  `api.extensions.fork` writes them: 8 of 10 load, replace their built-in
  and stay healthy; inspector and timeline fail to build exactly as on
  today's app (their shipped source imports `../../renderer/src/…`, which
  `main`'s compiler also refuses: a product bug of today's fork tool, not an
  engine regression).
- Historical viewer source: bisection of main's shipped source trees between
  155481d0 and 470eeaf3 found **c2139713 (viewer 1.0.6)** as the oldest that
  still renders today. Every earlier first-parent shipped tree was checked,
  including 155481d0 and the predecessor f5f993d3; none worked. The checked-in
  `fixtures/compat-corpus/oldest-viewer` is byte-for-byte that tree (the gate
  checks every file against git). Both a fork with a pristine `.forked-from`
  and a `replaces`-only folder activate unchanged on the engine, bind their
  canvas and read the composition through their original `render.gl.context`.
  Both whole-canvas comparisons pass: max 2; 37 of 3,619,008 RGB channel
  values over 1, within the unchanged section-25 bounds.

- An override by id of every built-in contribution
  (`fixtures/compat-corpus/override-builtins`): 16 effects, 4 transitions,
  48 commands, 6 panels, 264 keybindings, 2 themes, 6 layer types, an
  inspector section, a palette provider and 5 services. Each override is the
  registries' winner; a built-in command id runs the override; every effect
  and transition draws the override's program on the engine exactly as
  today's compositor does with the override (max 0 for all 20), visibly
  different from the built-in's program; disposing the overrides gives the
  built-ins back.
- The API probe (`fixtures/compat-corpus/api-probe`) at apiVersion 1, 2 and
  3 in the trusted in-realm context, and at 3 as a Store install the user
  trusted with full access (`permissions` exist from apiVersion 3 on). Each
  member is compared with today's answer in the same window (the current
  engine's functions through `api.host.pm`, the legacy helper
  implementations, known values): `project.*` (identity, select, frame-snapped
  `setTime`, `apply`/`undo`/`redo`, P1 ids, `snapshot`), all of `anim.*`
  (values within 1e-9, live channels and keys, key edits), `model.*`,
  `groups.*`, `space3d.*`, `util.*`, `transport.*`, `render.gl.*` (bounds
  within 1e-6, pick, viewport identity, the compatibility context reading
  back), `render.raster` geometry and `textLayout` against today's,
  `renderFrameTo` pixels against today's (the title's rows are text-render
  parity, gated by the typewriter test), `render.snapshot` equal to the JPEG
  of the engine's frame, the async siblings equal to the sync members, the
  `raster` service captured, wrapped and disposed, `media.timing.*` against
  today's, `commandForAsset`, the live asset fields of an image, a video, an
  audio file and a glTF model, `media.assets.source/audio`, `scene3d` reads,
  every `EDITOR_HELPER_EXPORTS` name from the compiled bundle and the
  evaluation shims against the legacy implementations, and all 16 events
  (each caused the way the editor causes it; payloads, the time event before
  the frame drawn for it, `frame:rendered.viewport` identity).
- `engine-viewport-compat.spec.ts` installs an independently compiled copy
  of `fixtures/compat-corpus/viewport-probe` for each of the six viewport
  members and twenty controller members, at apiVersion 1, 2 and 3 locally
  and apiVersion 3 as a declared full-access Store install. All 104 probes
  passed with the engine on. They assert selection, notifications and
  disposal, input handling, tools, cursor and wire picking, modal transforms,
  walk, extrusion/cancellation and non-applicable cases, not just names.
  The member inventory is exhaustive against the public interfaces. The
  3D lane is still unmerged here; this proves the current facade's behavior,
  not that every internal 3D computation has been replaced.
- The sandbox probe (`fixtures/compat-corpus/api-probe-sandbox`) as a Store
  install at apiVersion 1, 2 and 3: `project.get` resolves to a frozen
  snapshot, the reactive reads stay synchronous, `project.snapshot` resolves
  to the engine's frame (decoded, equal to the editor's own capture),
  `commands.run`, `assets.get` and `storage` are Promises, writes without
  `project:write` reject with today's permission error (apiVersion 1 and 2
  cannot declare it; at 3 the probe declares it and the writes resolve), the
  trusted-only namespaces are absent, and the editor events a sandbox gets
  (`HOST_EVENTS`) arrive. Passed at all three versions.
- The rebase gates (`main/extensions/rebase-corpus.test.ts`, real sources):
  a modified fork of the shipped viewer rebased onto this branch's viewer
  takes the migration off `render.gl.context` (no deprecated member left)
  and keeps the user's change; a conflict on the migrated line leaves the
  live fork untouched for the agent; undo restores the fork byte for byte;
  keymap-default merges with a user binding. A fork that is never rebased
  keeps working: the corpus loads it unchanged.

## Deprecations and warnings (amendment 23)

`DEPRECATED_MEMBERS` lists every compatibility member and its replacement;
EXTENSIONS.md mirrors it. The replacements exist for every apiVersion
(`render.snapshotAsync`, `renderFrameToAsync`, `rasterAsync`,
`media.assets.source`, `media.assets.audio`, `render.gl.canvas`); nothing
needs an apiVersion bump to migrate. The analysis follows the API object
through `activate`'s parameter, `PowermoveAPI` annotations, `api`, aliases,
destructuring, literal computed keys and call results; a computed key that
is not a literal is a possible use (`access: 'dynamic'`). It skips tests,
declarations, and files over 2 MiB; at most 200 warnings per extension.
Warnings never block.

## Today's document values (issue P1)

Documents and saved presets store today's export format ids where they exist
(`prores`, `mp4`, `webm`, a stored `rec`), so apiVersion 1 to 3 extensions
reading `project.exportDefaults` see what today's app wrote. Every app reader
migrates (`migrateFormat`). Formats the engine added keep their ids.
apiVersion 4 (not loadable yet: `EXTENSION_API_VERSION` is 3) is to read the
new ids through a typed accessor; none is added in this lane.

## What remains (blockers under amendments 22 to 24)

- External setup acceptance requires the original file to finish opening
  and every corpus-frame and extension-health gate to pass. Its private
  outcomes and crash evidence are reported outside Git.
- **The occlusion-culling crop regions and 3D plane windows** are not ported
  (see "Crop"). A crop-sensitive wrapper could differ there.

- The previous round's text/export failures were rerun after merging the
  fonts lane in c6966e68. The synchronous PM.raster-assignment gate now has
  max 1 and no channel values over 1. The whole API probe passes at v1–3
  local and full-access Store v3. The typewriter preview/disposal gate
  passes, and its export uses the job engine (`routed: null`, 180 window
  requests), with max 1 and no values over 1 in the sampled frames. These
  specific previously recorded differences are cleared by the final merge.
- **The hidden host and `pm-cli`** have no window to draw a raster
  override's layers: such an export fails by name there (plan: the hidden
  host's on-demand Chromium, L6).
- The lookahead answers at the open composition's frame rate only.

## Round-four verification and the private setup gate

After merging native integration `8ebaa950`, the shipping thin-LTO addons
and rebuilt wasm module passed all twenty public gates in one run (2.2 min).
The zoom test centres both viewports on the glyph bodies. It reads the
exact presented surface before its pool slot is
released, requires its current view sequence and requested time, checks
visible ink and changed pixels after seeking, and compares every crop with
the legacy TypeScript function. It keeps the same density and pixel checks.

| Gate | Observed round-four result |
|---|---|
| 8× zoom, density 24 | Frames 1 → 8 → 9; crop `(2304,1024,1792,1024)` |
| 16× custom-viewer request, density 32 | Frames 9 → 12 → 13; crop `(2816,1280,1024,768)` |
| Both zooms | Presented 0.2 s then 1.7 s at the current view sequence; no failures; text pixels change |
| Synchronous `render.raster` at 16.25, 24, 32 | Geometry equal to today; max 0; invalid 33 appears by name in status and the viewer report |
| Substituted text, sync and async raster APIs | Geometry equal; full-frame max 0 at 0.2 s and max 1 at 1.2 s; no RGB values over 1 |
| Trusted API v1–3, full-access Store v3, sandbox v1–3 | Behavior probes pass, including all sixteen public events in the trusted contexts |
| Viewport/controller | All 104 compiled per-member probes pass |
| Historical viewer fork and replaces-only | Both pass unchanged; max 2, 37 RGB values over 1 |
| Typewriter preview, disposal, export | Pass; export uses the job engine, 180 producer asks, no routing, sampled frames max 1 |
| Shape override and render-queue exports | Pass; 180 asks each, sampled frames max 1 and max 0 respectively |

Playback under load remains the orchestrator's measurement window. The
additional rebase, local fork update and built-in shader checks pass too
(seventeen tests).

The historical fork gate exposed a separate readback problem: WebGPU clears
its canvas swapchain after Chromium composites it, before the coalesced
`frame:rendered` event. Reading that canvas in a later task returned black.
`CompatContextHost.captureFrame/selectFrame` now preserves completed engine
surfaces before their pool slots are returned, and selects the copy by the
presented frame's exact tags. Copies are made only while a compatibility
context exists. Imports that arrive after descriptors are captured on the
surface notification too. Unit tests cover delayed reads and selecting an
older presented frame instead of the newest arrival. The historical fork
and replaces-only form both passed the functional readback gate after this
fix. The shared-texture import also needed its descriptor's Display P3
encoding interpreted before conversion into the legacy sRGB canvas; the
historical viewer gate exercises a colored background and full text pixels.
The context gate independently checks legacy compositor pixels, drawing and
canvas mirroring, with 59,037 green project pixels on each side and max 0.
A later read of the cleared WebGPU swapchain is no longer its reference.

C1 is `e2e/engine-private-setup-compat.spec.ts`. It reads the external corpus
in place, installs the 39 original extension folders into disposable profiles,
then opens the original project off and on. Its result ledger is
`$TMPDIR/powermove-private-setup-compat/results.json`, outside Git. It records
per-extension activation, health, contribution ids, winners, panels, safe
open/read commands, the workspace theme and tools, wrapper presence and
project frames. It uses the real-project corpus schedule (every two seconds
plus the last frame), adding each typewriter interval's midpoint, at full
project size with the corpus's motion-blur settings and unchanged section-25
bounds. The omitted TensorFlow runtime's private-extension-32 build failure is reported
on both paths; no other failure or runtime auto-disable is excused.

The isolated profile acknowledges the workspace theme's explicit one-time
appearance recovery before activation, so the requested theme can be exercised
without modifying the private source. The activation barrier waits for actual loader activations,
not main's successful compilation records. No live profile is read or written.
A missing corpus yields a named notice. With the corpus present, run its
external-artifact configuration so Playwright's traces and DOM snapshots
cannot copy private data into the worktree:

```sh
pm-electron-lock bun run --cwd apps/desktop test:e2e -- --config playwright.private-corpus.config.ts --workers=1
```

The viewport probes and private harness terminate their own isolated Electron
processes after assertions. Graceful quitting after these histories/viewports
was observed to wait indefinitely; shutdown is not claimed as passing here.


### Additional failures found by the real setup and the faster zoom gate

The public raster facade used to apply pm-wasm placement *after* an extension
wrapper returned. That overwrote a changed text surface's anchor with the
original text's metrics. `makeRender` now preserves wrapper-owned geometry;
it still applies native placement to an unwrapped base. The synthetic
`label-substitution` fixture covers this independently of private extension
source, including synchronous and asynchronous raster APIs and complete frames.

The snapshot queue also retained the byte charge of every shared-memory
result for sixty seconds after main had already taken and unlinked it.
High-density text rasters and the real setup exhausted that stale accounting:
the next base raster failed `budgetExceeded`, then the wrapper could not draw.
Published-result accounting now tracks the actual segment names and reclaims
only names main took or orphan cleanup unlinked. Other OS errors retain their
charge. The 1 GiB pending-result cap and queue bounds are unchanged. Layer
rasters additionally retain today's 8192×8192 bitmap limit instead of the
old 6000×6000 request ceiling: their admission limit is independent of the
unchanged 40-million-pixel full-frame limit. A queue test checks both classes. A real
POSIX shared-memory test proves that a full budget refuses a request before
main takes its result and admits it immediately afterwards, without another
publication or a timeout. The four shared-memory tests and thirteen snapshot
queue tests pass.

The merged shader fallback path exposed another snapshot limit: answering
raster needs can reveal successive dependent shader programs. A global
three-round limit can stop before the final render even while every phase
makes progress. Snapshot retries now track the actual unresolved
dependency set: three repeats fail by name, distinct dependency phases can
finish within a hard 68-phase bound, and the protocol's 64-upload limit is
unchanged. Tests cover synchronous, asynchronous and main-owned snapshot
sessions plus a repeatedly unanswered raster. The focused renderer/bridge/kernel tests
pass: 272 tests across eighteen files.

The private schedule uses full composition size, each typewriter midpoint,
the corpus interval and the original project's final frame. Reused baseline
PNGs must decode byte-for-byte to their original RGBA hashes. The external
reference ledger records the source commit/tree hash, project hash and each
frame's hash. The harness's thirty-minute overall budget accommodates large
saved histories; project-open checks and section-25 pixel bounds remain
explicit. Layer installation precedes file fingerprinting and workspace
restoration, so the harness awaits the original open promise, with a
ten-minute allowance for that prerequisite, and stops before captures if
opening is unfinished. Renderer exit reasons are collected from main even
when the renderer can no longer answer diagnostics. Private measurements,
crash evidence and automatic Playwright artifacts remain outside the worktree.
