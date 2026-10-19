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
| Protocol | `pm_protocol::snapshot::CompatMessage` (`Producers`, `Native`, `Reuse`, `Needs`), `CompatPixels.size/uv`, `CompatFrame.native/size/uv`, `DeliveryOptions.compat` |
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
| the default framebuffer holds the presented frame (cleared to opaque black, the composite drawn at the canvas size), so `readPixels` reads the composite | before the first read (`readPixels`, `copyTex(Sub)Image2D/3D`, `blitFramebuffer` from it) or draw into the default framebuffer after a presented frame, the engine's canvas under the bound canvas is drawn over opaque black into a scratch canvas at the bound canvas's backing size (pixel for pixel at equal density, from the nearest whole-pixel origin) and blitted in, bottom row first. The extension's GL state (bindings, unpack flags, scissor, rasterizer discard) is restored |
| drawing into the default framebuffer shows in the viewer until the next frame clears it | after a draw call, clear or blit into the default framebuffer, the offscreen canvas is copied (after the task) to a layer canvas inserted right after the engine's canvas (under the viewer's overlay) at the bound canvas's box; the next presented frame hides it. A `frame:rendered` handler draws on the new frame: the host hears `preview:presented` first |
| exports and captures never include it (they render their own frames: `exporter.ts` `renderInto`, `GL.renderToPixels`) | the same: nothing reaches the engine |
| its own textures, framebuffers, programs | the same (an ordinary WebGL2 context) |

Limits: it is not the context that draws the frame, so GL state an extension
sets cannot affect the engine's drawing (it could disturb the compositor's
today); `drawImage(context.canvas)` reads the bound canvas, which the engine
presents over (it is blank), where today it read the composite; the frame
read back is the presented frame (an `rgbaf16` EDR canvas is read as 8-bit
sRGB, as today's 8-bit canvas held it); a lost offscreen context is made
again on the next access (a new object, as `GL.init` after a restore).
Drawing is shown in the viewer's DOM rather than sent through the engine:
today's result is the canvas's own contents, shown at once and replaced by
the next frame, which a round trip through the engine would show a frame
late.

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

### Measured (2026-10-19, e2e under the Electron lock, flag on)

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
  Side by side: `/tmp/ne-extcompat-evidence/shape-export-engine-left-today-right-frame45.png`
  and the 8× crop `shape-export-zoom-195-110-engine-left-today-right.png`.
- Render queue: a job queued with a raster override, the project then edited
  (the tinted ellipse narrowed and moved), draws the project as queued:
  equal to the export sheet's engine export of the queued state at frames 0,
  45 and 89 (max 0), 180 asks.

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
- The oldest fork shapes the loader accepts, from git history: the viewer at
  155481d0 (viewer@1.0.0, the commit the fork tool first shipped in) as that
  tool wrote it, and the same source as a `replaces`-only folder (the shape
  before the fork tool). Each is compared with today's app (flag off, a
  fresh profile) on whether the viewer shows the composition, through the
  old viewer's own `render.gl.context`. Measured: neither works on today's
  app (the old viewer's `#gl` canvas ends up out of the document; today's
  compositor draws into it unseen), and neither on the engine; the engine
  behaves as today. A frozen viewer that called `init` before the engine
  connection installed is now bound when it does (`bindLate`).
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

- **Text exports with a raster override** go to the current exporter: the
  job engine has no render fonts on this branch, so `pm_render::unsupported`
  routes every text export. The fonts lane has job fonts on `feat/ne-fonts`
  (`unsupported_with_fonts` in pm-export and the job lanes, not merged);
  with it, the typewriter export test is the gate (it asserts the engine
  drew it, asking the window each frame, and compares with today's export).
- **Native text against today's** on the unaffected 32 px regular layer of
  the typewriter project: max 57 code values, 2652 of 144,000 channel values
  over 1 (ink 1250 against 1294). Text-render lane; the typewriter test
  fails on it. Side by side (left engine, right today):
  `/tmp/ne-extcompat-evidence/typewriter-engine-left-today-right.png`.
- **The hidden host and `pm-cli`** have no window to draw a raster
  override's layers: such an export fails by name there (plan: the hidden
  host's on-demand Chromium, L6).
- The oldest fork shapes do not work on today's app either (above); the
  oldest shape that works on today's app lies between 155481d0 and
  470eeaf3 and is not bisected.
- The lookahead answers at the open composition's frame rate only.
- `drawImage(render.gl.context.canvas)` reads the bound canvas the engine
  presents over (blank), where today it read the composite; `readPixels` is
  the supported readback.
- `scene3d` controller methods and the `viewport.*` members are not probed
  one by one (the probe calls the reads); the 3D lane owns their backing.
