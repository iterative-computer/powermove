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
| `render.gl.canvas`, the compatibility context | `renderer/src/kernel/compat-context.ts`, `install.ts` |
| Today's export format ids (issue P1) | `shared/delivery.ts` (`storedFormatId`), `core/export-defaults.ts` (`storedExportDefaults`) |
| Corpus | `apps/desktop/e2e/fixtures/compat-corpus/` (shipped built-ins, API probe, shape tint), `e2e/engine-customization-corpus.spec.ts`, `e2e/engine-extension-compat.spec.ts` |

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
   after a second), and draws the frame when they arrive; a snapshot answers
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
   unchanged cache `key` is answered `Reuse`. A captions layer's base is not
   lazy: between cues it is null, as today.
5. pm-render draws a raster as a canvas backing store: premultiplied E
   values, top row first, on `Quad::Texture` through the layer's transform,
   effects, masks, mattes and blending (`Source::Compat`).
6. Exports name the window's producers (`DeliveryOptions.compat`). The job
   asks the host for each such layer at each frame (`Request::CompatFrame`,
   blocking: an export has no per-frame deadline); main asks the editor
   windows; the window with the document runs the chain with fresh pixels.
   With no window answering, the export fails with the host's reason; it
   never draws the layer without its override.

A wrapper that calls `render.snapshot` from inside its own answer gets `''`
(its frame would need the answer being made); today's compositor recursed.

### Measured (2026-10-18, e2e under the Electron lock, flag on)

`e2e/engine-extension-compat.spec.ts`:

- Typewriter fixture (apiVersion 1), a 150% bold title keyed 0 → 100% and a
  plain 32 px layer: the engine's frames against today's compositor in the
  same window, typed band max 1 code value at 0%, 40%, 100% (ink pixels
  0 / 2261 / 6869 on both); 10 needs answered for three snapshots (5
  rasters, 5 pass-throughs); viewer frames asked 6 times in 1.5 s; after
  disposal the plain title draws again.
- Base raster geometry: the engine's lazy base for the scaled title equals
  today's surface exactly (`w` 310, `h` 140, canvas 465 × 210, anchor 155,
  48). Before the layer raster fix it was the ink crop's (214 × 40.7),
  which squeezed the wrapper's canvas (`c733fa73`).
- Export (PNG sequence, 90 frames, shapes only so the job engine draws it):
  180 asks (2 layers × 90 frames); red pixels equal to today's compositor at
  frames 0, 45, 89 (7418/7418, 7360/7360, 7359/7358).

### The corpus (measured 2026-10-18, flag on, 4 of 4 passed in one run)

`e2e/engine-customization-corpus.spec.ts`:

- Forks of today's shipped built-ins (`fixtures/compat-corpus/shipped`, the
  source on `main`), laid out as `api.extensions.fork` writes them: 8 of 9
  load, replace their built-in, and stay healthy; the pre-engine viewer fork
  binds the engine's presentation through `render.gl.init` and
  `render.gl.context.canvas === render.gl.canvas`. The inspector fork fails
  to build exactly as on today's app (its shipped source imports
  `../../renderer/src/…`, which `main`'s compiler also refuses). Timeline is
  not in the fixtures (a gitleaks false positive on a new file; see the
  fixture README); its shipped fork fails the same way on today's app.
- The API probe at apiVersion 1, 2 and 3 (trusted): every check passes —
  live `project.get()` identity, `exportDefaults.format` `prores`, sync
  `render.snapshot`/`renderFrameTo`/`raster` shapes, `gl.context`, `textLayout`,
  `gl.bounds`, `anim.ev`/`worldMatrix`/`resolveContent`, the raster
  service, editor helpers from a compiled bundle (`resolveContent`,
  `evaluatedValue`, `is3DLayer`, `planeMatrix`, `countTextUnits`,
  constants), the async siblings, and `frame:rendered` with its viewport
  identical to `render.gl.previewViewport`.

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

- **Compatibility context drawing and readback.** `render.gl.context` keeps
  today's lifecycle and canvas (frozen viewer forks use exactly that), but
  with the engine what an extension draws into it is not shown and
  `readPixels` does not read the engine's frame. Today drawing into the
  compositor's default framebuffer shows in the viewer until the next frame.
  No known extension draws into it (Phase 0 audit); not built.
- **Text exports with a raster override** route to the current exporter
  because the job engine has no fonts yet (RFC 16 "Open"); the job's compat
  path is exercised by shapes only.
- **Agent exports and render queue jobs** carry no producers (main builds
  their settings without the window): a project with a raster override
  exports without it there. Sync snapshots main serves from its own session
  for a window (boot, staged resync) do not ask producers either.
- **Native text against today's** on an unaffected 32 px regular layer in
  the typewriter project: max 57 code values, 554 channel values over 8
  (`/tmp/ne-extcompat-evidence/typewriter-engine-left-today-right.png`, left
  engine, right today). Text-render lane.
- **Export pixels:** the shape export matches today's red coverage but
  differs from today's compositor by up to 35 code values in 3 channel
  values a frame (not localized in this lane).
- **Corpus:** not built: the oldest fork shape from git history, overrides by id of each
  built-in contribution, the probe in the declared full-access Store and
  sandbox contexts, the rebase gates (conflict, no-conflict, undo, deprecated
  calls migrated), and a probe per `scene3d.*` and controller member.
- Not re-verified on the engine in this lane: `assets.get` live fields from
  engine data (`audioDecodeError`, `object3d` from pm-wasm geometry),
  `uiState.getShaderMeta`, all 16 events through the sandbox transport.
