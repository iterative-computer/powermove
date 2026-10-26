# Extension compatibility on the engine

Extensions written against today's app (apiVersion 1 to 3) run unchanged
with "Use New Engine" on: the same API answers, the same events, and the
same pixels from a `raster` override. This note describes each
compatibility path, why it exists, and the invariants it keeps.

## Where the code is

| Part | Code |
|---|---|
| Deprecated members, replacements, source analysis | `apps/desktop/src/shared/extension-analysis.ts` |
| Warnings returned to agents | `main/codex/validate-staged-extensions.ts`, `main/compatible-workspace.ts`, `main/agent-tools/external-session.ts`, `main/cloud/publish.ts`, `shared/agent-guidance.ts` |
| Typings and named replacements | `renderer/src/kernel/api.ts`; the table in `apps/desktop/docs/EXTENSIONS.md` (a test keeps it equal to the module) |
| `media.assets.source` and `audio` | `renderer/src/kernel/live-media.ts` |
| The raster override as the engine's producer | `renderer/src/kernel/raster-compat.ts`; transport in `engine-connection/render-transport.ts` |
| Engine: drawing compatibility rasters | `pm-render` `compat.rs`, `plan.rs` (`compat_quad`), `exec.rs` |
| Engine: answers per session | `pm-node` `present/compat.rs` (`CompatStore`, `CompatView`), `present/render.rs` |
| Engine: layer rasters off the render thread | `pm-node` `present/rasters.rs` |
| Engine: exports | `pm-node` `jobs/compat.rs`, main's `engine/compat-frames.ts` |
| Protocol | `pm_protocol::snapshot::CompatMessage` (`Producers`, `Native`, `Reuse`, `Needs`), `CompatPixels`, `CompatFrame`, `CompatFrameRequest`, `DeliveryOptions.compat` |
| Today's preview crop | `pm-render` `compat_crop.rs`, `plan.rs` `preview_crop` |
| The compatibility context | `renderer/src/kernel/compat-context.ts` (`CompatContextHost`), wired in `kernel/install.ts` |
| Provisional revisions | `engine-doc/barrier.ts`, `engine-connection/install.ts` (`flushSoon`), `engine-connection/session.ts`, `engine-connection/patch-coalesce.ts`, `pm-node` `present/render.rs` |
| Today's export format ids | `shared/delivery.ts` (`storedFormatId`), `core/export-defaults.ts` |
| Corpus and gates | `apps/desktop/e2e/fixtures/compat-corpus/`, `e2e/engine-*compat*.spec.ts`, `e2e/engine-customization-corpus.spec.ts`, `main/extensions/rebase-corpus.test.ts` |

## The raster override

Today's compositor calls `PM.raster(L, continuousRasterScale(…), T,
uploaded, crop)` for every text, captions and shape layer, and draws
whatever the wrapper chain returns on the quad it describes. The engine
keeps that contract:

1. While a wrapper is registered (`services.onChange('raster')`), the window
   tells each engine session it produces `rasterOverride`
   (`CompatMessage::Producers`). A session announces its producers as soon
   as its id is bound, before its first resync, so the first frame and the
   first capture ask the producer too. A chain or font change re-sends the
   list through empty, which drops every earlier answer.
2. With a producer, `pm_render::plan` asks for every text, captions and
   shape layer that is not `compat` level 2, at the layer's own time and
   density. An answer counts when it was made at the frame's revision or a
   newer one.
3. A frame without every answer fails with `RenderError::CompatPending`. The
   presenter sends the needs to the window (`Needs`) and draws the frame once
   the store answers all of them (not at each answer: every attempt plans the
   whole composition). A snapshot answers `needsRenderer.compat`; the
   renderer draws the needs, posts them on the session's port, and asks
   again.
4. The window runs the chain over a lazy engine raster (made on first field
   read). The wrapper's own base back means pass-through (`Native`: no
   pixels, the engine draws the layer). Any other result is read back as
   today's compositor uploads a canvas (premultiplied WebGL2 upload, then
   `readPixels`) and posted with its size, anchor and `uv`. An unchanged
   cache key is answered `Reuse`.
5. pm-render draws an answer as a canvas backing store (premultiplied, top
   row first) on the layer's quad, through its transform, effects, masks,
   mattes and blending (`Source::Compat`).
6. A wrapper that throws fails the answer, as it failed today's frame: the
   viewer keeps its last frame and the window reports it once per two
   seconds; an export fails with the message. The plain layer is never drawn
   in its place. A wrapper calling `render.snapshot` from inside its own
   answer gets `''` (today's compositor recursed).
7. Exports name the window's producers (`DeliveryOptions.compat`). The job
   asks for each such layer at each frame (`Request::CompatFrame`, blocking).
   Main sends the layer as the job's snapshot holds it, so a queued job draws
   the project as it was queued. With no window answering, the export fails
   by name; it never draws the layer without its override.
8. A sync snapshot main serves for a window carries the window's producers
   and answers (`EngineSnapshotRequest.compatEvents`).

### Crop

In viewer frames of the open root composition, today passes `PM.raster` a
crop window for a large text raster or a partly visible shape without paths,
skips shapes the viewport misses, and fills the viewport with a plain
rectangle's color when the viewport lies inside it. `pm_render::compat_crop`
ports those functions; `compat_quad` makes the same decision from the
viewer's complete canvas including its pan buffer
(`RenderRequest.preview_canvas`). The window travels with the need
(`CompatNeed.crop`) and is part of the answer's identity. Exports and
snapshots are never cropped.

### `PM.raster` assigned directly

Before the services registry, extensions wrapped the raster by assigning
`PM.raster`. `PM.raster` is an accessor that behaves as the data property
did; an assigned function is the outermost link of the chain, and assigning
or restoring it updates the producer like a services change.

### Any scale

Today's `PM.raster` has no upper bound on `scale`: every rasterizer takes
`density = min(scale, 8192 / w, 8192 / h)`. A public `Layer` snapshot takes
any positive finite scale (`valid_layer_raster_scale`); `MAX_RASTER_SCALE`
(32) bounds only the compositor's own compatibility messages. Values above
2^53 are sent as `Number.MAX_SAFE_INTEGER`; zero, negatives and NaN become
`VANISHING_RASTER_SCALE` and return today's transparent pixel.

### Layer rasters

A wrapper's base is a layer snapshot. `present/rasters.rs` draws them on
their own thread with a second `Renderer` on the same device (its own
evaluator, text context, fonts and command buffer; readback waits for its
own submission only), so a slow base never delays a timed frame. The worker
keeps a mirror of each session's document, fed by the same updates the
render thread applies; a job names its revision, and a mirror at another
revision sends it back to be reseeded. The session document is an `Arc`
updated in place, so no revision copies it. Pixels come from the same
`Renderer::layer_raster` over the same revision as before.

### The base cache

Bases follow today's raster cache (`legacy/gl/raster.ts`): plain text and
plain shapes key by native-evaluated content, density and crop, with layer
identity for document layers and content identity for detached copies (at
most sixteen surfaces, 32 MiB). Content without keyframes or expressions is
its own resolution and keys by what is stored, without asking the
evaluator. Font-anchored text keys by its placement too: the layer's anchor
at that time (pm-eval `text_raster_with`). Paths, text animators and
styles, compatibility level 2 text and captions are never cached. A plain
shape's raster depends only on its content and density, so its base
survives document revisions; text bases expire at each revision.
Preparations asking for the same content at once share one engine raster.

The content key is read when the base is requested, together with the
document revision it was read at. An asynchronous base is remembered under
that key only if the engine drew it at that same revision: an edit while
the raster was in flight would otherwise store old pixels under a key read
before the edit.

### Playback

A frame whose answers are missing waits for its producer, keeping its slot
until it can no longer be shown, and is drawn once all of them arrived.

Preparation schedules complete frames. It follows the audio-clock playhead
along the loop (`runAhead`, `intoLoop`), frame by frame in playback order:
every layer of a frame runs its chain in one turn, and the producer answers
a pass-through in that turn (`prepare` returns null; no base, no round
trip), so only wrappers that read bases continue later, as the engine draws
them (in the task their base arrived in). A turn takes at most 6 ms, but
never splits a frame; the rest is the next task. The horizon is eighteen
frames, and reaches as far as the slowest layer's answers take plus 250 ms,
up to sixty. A layer whose answers take longer than the clock leaves starts
where it can still be in time (with none on its way, at its furthest frame
when it can make none); at most 24 preparations wait for bases, and
layers answered at once never wait behind them. Preparation runs on the
engine's needs, completed preparations and presented frames, and once a
frame while playing; never as a poll. It stops at the composition's or
loop's end, and at a layer's own time span. Prepared results stay within
64 MiB and 4096 entries.

While playing, after each frame the presenter plans (without drawing) the
frame a second ahead and sends the window the needs it has not answered
(`ask_ahead`): a layer coming into view, a crop window that moved. A layer
whose crop window moves is prepared only at the frames the engine names,
each with its own window.

A paused edit prepares only the frame on screen, urgently.

The base is lazy: reading an unavailable base during preparation suspends
the attempt, the engine draws the base asynchronously, and the wrapper
retries (at most 64 dependency phases; a nonconvergent chain fails by name).
A wrapper that catches a missing-base read cannot publish its incomplete
result. Canvases are captured as premultiplied ImageBitmaps and read back on
a dedicated worker with the same upload and readback function as the
synchronous path; geometry is captured first. Bases arrive with straight
alpha: the engine divides the quantized premultiplied pixels by alpha
exactly as the window's `unpremultiply` did, so the base canvas is byte for
byte the same. The window keeps base requests within main's byte
reservation (each reserves its bound) instead of being refused; a playback
base reserves twice the largest one so far and asks again with the whole
8192² bound if the engine refuses it as larger. A paused edit's own answer is
read back on the renderer thread, since its frame waits for it.

With producers, a Play whose first six frames are not answered holds the
stopped line for at most 80 ms (the native preroll). An answer redraws only
a frame waiting for it, Play from a stopped line does not re-seek, and
paused redraws wait while a Play is in its preroll, so no time is presented
twice. A paused draw request with no new revision, view or time answers the
frame on screen again (`refreshShown`) and announces it again
(`PresentedEvents.redraw`), as today's frame loop did. Public synchronous
APIs, synchronous snapshots and exports keep their synchronous behavior.

### Synchronous captures

A capture blocks the window in `sendSync`, so it must never wait for work
that needs the window. `SnapshotRequest.compat_uploaded` names the needs the
capture itself answered; the engine returns every other missing need,
including one a viewer asked for and the window has not answered yet.

## Paused edits: provisional revisions

A `touch()` without paths compares every layer of the open composition,
which takes too long on a large document for an edit to wait for it. While
paused, the edit is sent in two steps:

1. `flushProvisional` diffs only the paths written by name and the layers
   recent diffs changed (`HOT_LAYERS`), and sends them as a provisional
   batch. The engine applies it and starts drawing.
2. In the same task, `flush` compares the whole document. If it finds
   nothing more, a `confirmed` stream item for that revision follows on the
   same port (`confirmRevision`), and the engine presents the frame. If it
   finds more, the next batch supersedes the provisional revision, and its
   frames are dropped.

Only then does the window prepare the frame on screen at the new revision
(`prepareShown`): the wrapper over the cached base, the readback, the answer.

Invariants:

- No frame of a provisional revision is ever presented before it is
  confirmed. With the completion thread, a finished paused frame is parked
  until `Confirm` (sent) or the next update (dropped). Without it, nothing
  can hold a frame, so a provisional revision is not drawn at all until it
  is confirmed.
- No timed playback frame renders at a provisional revision, and no
  provisional batch is sent while playing.
- Engine snapshots (captures, thumbnails, layer rasters) are not ready while
  their session's revision is provisional. They run once it is confirmed or
  superseded, so no snapshot reads a document state that never existed.
- No query, wrapper or override sees a partial document. The whole
  comparison runs before anything else in the task, every query flushes
  first, and `snapshot()` and `resume()` refuse a provisional state.
- At most one provisional revision is outstanding. A coalesced batch is
  provisional only if its last batch is.
- A session never resyncs from a provisional state. When it cannot forward
  a provisional batch (too large, or after a gap), it holds it. The
  confirmation, or the superseding batch, then resyncs from the settled
  document.
- A comparison that throws still ends the provisional revision: the barrier
  resyncs, which supersedes it.
- Reconnect and resync replay the journal in order, confirmations
  included, so the viewer is never left blank or holding a parked frame.

## The compatibility context (`render.gl.context`)

Today it is the compositor's WebGL2 context on the canvas a viewer passed
to `render.gl.init`; its default framebuffer holds the presented frame. With
the engine, `CompatContextHost` gives an ordinary WebGL2 context with the
same reads:

- It is null until `init`, and `context.canvas` is the bound canvas.
- While a context exists, completed engine surfaces are copied before their
  slots are released (at most eight). The public presentation event selects
  the exact `(revision, viewSeq, seq)` copy. Before the first read
  (`readPixels`, texture copies, blits from it) or draw after a presented
  frame, its region under the bound canvas is drawn over opaque black into
  the default framebuffer, and the extension's GL state is restored.
- Drawing into the default framebuffer is shown in a layer canvas right
  above the engine's canvas until the next presented frame. The host hears
  `preview:presented` before extensions hear `frame:rendered`, so a handler
  draws on the new frame.
- The bound canvas holds each presented frame and any drawing over it, as
  `preserveDrawingBuffer` did.

Limits: GL state an extension sets cannot affect the engine's drawing; the
frame is read as 8-bit sRGB, as today's canvas held it; a lost context is
made again on the next access. Drawing is shown in the DOM, not sent
through the engine, so it appears in the same frame as today.

### Exports

A WebM export names the `compatContext` producer while an extension holds
the context. The job renders each frame as today's canvas held it (8-bit,
premultiplied, over black) and sends it to the window
(`CompatFrameRequest.frame`). `CompatContextHost.exportFrame` makes it the
context's content, emits `frame:rendered` as today's `renderInto` did, and
answers what the framebuffer holds if a handler drew into it, or a
pass-through. Frames nobody drew on keep the engine's precision. MP4,
ProRes, PNG sequences and stills never included the context, as today.

## Shader layers' program keys

Every presented shader layer gets today's `shaderKey`
(`sh:<id>:<hashStr(code)>`), and `render.gl.compileError(shaderKey)`
answers its compile report. A layer the WebGL fallback draws keeps the
engine's `engine:<id>` key.

## Failures

An extension's failed snapshot (`render.raster`, `render.snapshot`,
`renderFrameTo`) is recorded in `EngineConnection.status().failures` and
logged, but never hides the viewer's canvas. Only the viewer's own frame
failures show on the stage.

## Deprecations and warnings

`DEPRECATED_MEMBERS` lists every compatibility member and its replacement;
EXTENSIONS.md mirrors it. Every replacement exists at every apiVersion
(`render.snapshotAsync`, `renderFrameToAsync`, `rasterAsync`,
`media.assets.source`, `media.assets.audio`, `render.gl.canvas`). The
analysis follows the API object through parameters, annotations, aliases,
destructuring and literal computed keys; a non-literal computed key is a
possible use. It skips tests, declarations and files over 2 MiB, reports at
most 200 warnings per extension, and never blocks.

## Today's document values

Documents and presets store today's export format ids where they exist
(`prores`, `mp4`, `webm`, a stored `rec`), so apiVersion 1 to 3 extensions
reading `project.exportDefaults` see what today's app wrote. Every app
reader migrates (`migrateFormat`).

## The corpus

- Forks of every shipped built-in, laid out as `api.extensions.fork`
  writes them, and the oldest viewer source that still renders today
  (`fixtures/compat-corpus/oldest-viewer`, checked byte for byte against
  git).
- An override by id of every built-in contribution
  (`fixtures/compat-corpus/override-builtins`).
- The API probe at apiVersion 1, 2 and 3, trusted and as a full-access
  Store install, compared member by member with today's answers in the same
  window; compiled probes for the released 3D view, navigation and gizmo
  members in each of those contexts; the sandbox probe as a Store install.
- The rebase gates: a modified fork rebased onto the current viewer.

The `v1.2.0` tag exposes `scene3d` view, navigation and gizmo methods, but
not `Scene3DViewportAPI` or `getRendering`. Those names came from an
unreleased October 2026 branch and are not apiVersion 1–3 release contracts.
The checked-in "shipped" viewer matches source at `470eeaf3`, the oldest viewer
matches `c2139713`, and neither source calls those names. The 3D probes therefore
exercise the tagged API while the unmodified viewer forks continue to guard
the behavior of extensions users could already have installed.

## Measuring

Tracing is off unless asked for. `EngineConnection.setTrace(true)` enables a
bounded page-clock trace (`EngineRender.trace.take()`) of the edit path:
diff, encode, apply, dispatch, wrapper preparation, base snapshots,
readback, answer upload and the frame event. `POWERMOVE_BRIDGE_TRACE=1`
enables the native turn and present traces. The traces hold timing and
revision tags, not document content.

The paused-edit latency gate is opt-in. Build first, then run from the
repository root:

```sh
CARGO_BUILD_JOBS=4 bun run --cwd apps/desktop build:engine
CARGO_BUILD_JOBS=4 bun run --cwd apps/desktop build:wasm
bun run --cwd apps/desktop build
python3 apps/desktop/scripts/extension-perf.py --out /tmp/powermove-extension-perf
```

It holds `pm-electron-lock` only for the Playwright command (one worker),
alternates engine and today's app in fresh profiles on a public synthetic
document, and keeps samples and traces outside the repository. The
playback cadence gates attribute their longest gap from the same traces
(`stallAttribution`). `pm-perfbench` row `paused_edit_with_override_2500`
covers the native side of the same edit.

## Known differences

- Today's occlusion-culling crop regions and 3D layers' plane windows are
  not ported (they get the whole raster). A wrapper whose output depends on
  the window could differ there.
- The hidden host and `pm-cli` have no window to answer a raster override:
  such an export fails by name.
- The lookahead answers at the open composition's frame rate only.
- Today's NaN-scale canvas is 0×0 and a non-positive paths raster throws;
  the engine returns a one-pixel transparent surface with today's geometry.
