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
same decision from the viewer's complete canvas, including its pan buffer
(`RenderRequest.preview_canvas`), or the request's region when no canvas
was supplied (viewer frames only: not exports, not snapshots), for top-level
layers of the root composition without paths,
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
`PM.raster` (an external setup shifts text anchors with animated font axes
that way). Today the compositor calls whatever
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

A frame whose compatibility answers are missing waits for its window producer.
Playback preparation follows the audio-clock playhead, quantized to composition
frames, with eighteen frames of answers ahead. A 4 ms poll replenishes that
horizon even when nothing has reached the viewer. Delayed needs cannot imply
reverse playback: the transport rate supplies direction, and obsolete needs
yield to the current playhead. Each producer turn admits at most sixteen
answers within a 6 ms budget. Layer/source entries and their time windows
are bounded.

Two answer preparations and one speculative preparation may be in flight.
They use asynchronous IPC with native dependency priority: ordinary async
snapshots yield to due viewer frames, but a viewer waiting for these pixels
cannot make progress first. Once every layer's eighteen-frame answer horizon
is answered or in flight, complete results for frames 19 to 24 are prepared
speculatively. A paused need warms only within a 500 ms deadline, without a
continuing paused timer. Prepared results stay within 64 MiB and 4096 entries.
An actual compositor need remains asynchronous even after the stopped clock
arrives, and must finish even if that warm deadline expires. Required paused
layers share the same concurrency bound; only speculative warm-up expires.
The native thread schedules views and processes commands between dependency
snapshots during playback, rather than draining a burst across a frame deadline.

The base surface is lazy. A wrapper that passes it through untouched costs
no native raster; the engine draws the layer itself. Reading an unavailable
base during playback preparation suspends that attempt. The native engine
prepares the real base asynchronously, then the wrapper retries with those
pixels. This also discovers detached, retimed and cropped bases computed
inside a wrapper, including the typewriter's changing prefix. Up to 64
dependency phases may complete; a nonconvergent chain fails by name. A wrapper
that catches a missing-base read cannot publish its incomplete result. Once
the wrapper completes, its pixels and geometry are retained for the exact
composition, layer, time, density, crop and revision. The presentation answer
consumes that result without running another synchronous native snapshot.

Playback and speculative wrapper continuations yield through a background
scheduler task so frame presentation and input can run between preparations.
A required paused edit starts immediately and prepares the current frame only;
warming future frames at an obsolete revision adds work to every next edit.
The initial paused warm-up and playback horizon remain bounded as above.
Their canvases are captured as premultiplied ImageBitmaps without color
conversion and sent to a dedicated worker. That worker uses the same WebGL
upload/readback function as synchronous callers and transfers the pixel buffer
back, so playback's GPU readback never waits on the renderer thread. A paused
edit's own answer is the exception: its frame waits for those pixels, so it is
read back on the renderer thread with that same function, without the worker's
two message hops. Geometry is captured before the async readback, so a wrapper
reusing its surface object cannot change an earlier frame's quad. The worker
closes each bitmap, reports errors to the request, and terminates on disposal.

The base cache follows today's raster cache (`legacy/gl/raster.ts`): plain
text and plain shapes key by native-evaluated content, density and crop,
with composition/layer identity for document layers and content identity for
detached copies. It retains at most sixteen surfaces / 32 MiB. Paths, text
animators and styles, font anchoring and captions are never cached because
they can change with time beyond their resolved content. A genuinely changed
base is prepared afresh; the changing-base cadence gates assert asynchronous
loads throughout playback and zero synchronous loads during the measured span.
A plain shape's raster is a function of its resolved content and density
only, so its base survives document revisions (moving it needs no new
raster, as in today's cache); text bases expire at each revision (fonts). The
large-document edit gate resizes the shape on every move, so each move still
needs fresh off-thread native raster work from the worker's mirror.

A required paused viewer answer remains available until presentation, so a
native need emitted before the proactive upload can repost those same pixels
without a second wrapper evaluation and readback. Presentation ends this sharing
window. Explicit captures and redraws evaluate the wrapper afresh, including
when its own state changed without a document edit.

Reset, revision, layer identity, chain changes, fonts and disposal invalidate
prepared answers. Public synchronous APIs, synchronous snapshots and exports
retain their existing synchronous behavior. An asynchronous or idle snapshot
(a capture or thumbnail taken during playback) prepares its round's needs the
same way playback does and posts the prepared answers. Only a need whose
prepared answer did not survive a reset is answered synchronously. Before
this, such a snapshot ran the wrapper over a synchronous base raster in the
middle of playback; one of thirty loaded runs caught it.
`RasterCompat.stats.syncLoadSites` names the callers of the last four
synchronous loads, and the gates print them on failure. Nested or retimed
layers retain the open composition's preparation step and ask for missing
times.

### Paused-edit performance gate and attribution

A paused edit with an override goes: the window sends the layers the write
most likely changed as a provisional batch (`write-path.md`), prepares the
frame on screen at that revision at once (`prepareShown`: the wrapper over the
cached base, the readback, the answer), then compares the rest of the
document, which confirms the revision or supersedes it. Meanwhile the engine
applies the batch, plans the frame, waits for the answer (`CompatPending`),
draws it and parks it until the confirmation. The page forwards it to the
viewer's worker, which draws it on arrival; `frame:rendered` follows in the
window's next animation frame. The gate "a paused edit is presented only
whole" makes the comparison find more twice (the write moved another layer;
the override moves one and calls `touch()` while it draws) and requires that
no frame of either superseded revision is ever drawn. A third phase adds
2,500 layers, so the comparison outlasts the engine's frame: the engine parks
it, and the extra change found then must drop it.

Build the native addons, wasm and app before acquiring the Electron lock.
Run the opt-in gate from the repository root:

```sh
CARGO_BUILD_JOBS=4 bun run --cwd apps/desktop build:engine
CARGO_BUILD_JOBS=4 bun run --cwd apps/desktop build:wasm
bun run --cwd apps/desktop build
python3 apps/desktop/scripts/extension-perf.py --out /tmp/powermove-extension-perf
```

The launcher checks the native protocol, acquires `pm-electron-lock` for only
the prebuilt Playwright command with `--workers=1`, and starts two SHA256 load
workers after acquisition. It runs engine/today/today/engine in fresh profiles,
with 60 paused edits per run on a public synthetic document with 2500 inactive
keyframed layers. Each engine run also measures without the override. The
endpoint is `frame:rendered`, requiring the current revision on engine frames.
It saves every sample, traces, load averages, quantiles and comparison outside
the repository, then asserts engine-with-override p50, p95 and maximum are each
no greater than today's. This is a separate opt-in latency gate; the existing
38 compatibility gates retain their assertions. Profiles, temporary IPC files
and owned load workers are cleaned up on completion and interruption.

`EngineConnection.setTrace(true)` enables the bounded page-clock trace at
`EngineRender.trace.take()`: document diff/encoding/apply/dispatch, wrapper
scheduling/CPU, base snapshot native wait/render, main service and round-trip,
bitmap capture, readback, answer upload, viewer submission and the frame
event. Pair it with the viewer's presentation trace and the native
`turnTrace` / `presentTrace` (`POWERMOVE_BRIDGE_TRACE=1`): per frame, when
its revision arrived and was applied, render start, CPU, GPU completion and
send. Native layer raster queue time, worker execution and completion
delivery are separate; `render wake late` is time beyond the render thread's
requested wake, and `command drain` a burst's duration and count. For
snapshot wait/render/main events, `count` is the measured microseconds;
subtract main service time from round-trip time to locate renderer/main IPC
delay. The traces hold timing and revision tags, not document content.

The playback cadence gates run with all of these traces on. For the run's
longest gap between presented frames, `stallAttribution` lists what each
trace recorded inside it: native renders (start, CPU, GPU done, sent, timed
or not), render-thread turns, the viewer's arrivals, drops with their reason
and draws, the longest wait between animation frames, renderer stages and
long tasks. A failing cadence assertion prints it; the full traces are
attached to the test result.

A session announces its compatibility producers as soon as its id is bound,
before the initial resync. Main-owned snapshot sessions receive the producer
declarations and compatibility pixel bands through preload too, with byte
payloads preserved as typed arrays. Dropping those fields while copying a
request let an early capture render without its override; the first capture
must ask the producer just as later captures do.

An explicit compatibility-context read reserves the deferred compositor's
WebGL context before its canvas can become a 2D mirror. Replacing a viewer's
bound canvas while paused retains the selected completed engine surface,
instead of trying to recover pixels from an already cleared WebGPU swapchain.
Surface preservation also continues while the compatibility WebGL context is
lost. Recovering that context while paused then reads the current engine frame
even when no further frame is due; `active` still describes GL availability,
while `capturesFrames` describes this preservation requirement.

The `pm-perfbench` row `paused_edit_with_override_2500` ratchets native work
for 60 edits with a fresh 640×360 premultiplied canvas answer on each revision.
It includes the patch, upload, composite and GPU completion; window wrapper
preparation, IPC and display remain the Electron gate's responsibility.

Timed worker commits retain the earliest queued refresh deadline. When a later
BeginFrame overtakes an overdue timer, it fulfils that turn immediately. The
previous cancellation on every BeginFrame could starve timed drawing under
load, which also stopped the earlier presentation-driven lookahead. The
presenter regression test delays every timer past the next BeginFrame and
requires the timed frame to draw. Viewer version is 1.3.8.

### Shipping playback measurement (2026-10-21)

Ten serial three-second, 30 fps runs used the release addon with thin LTO and
one Playwright worker, alongside two independent SHA256 CPU workers. Each run
compared the engine and today's compositor in the same process campaign:

| Metric | Engine | Today |
| --- | --- | --- |
| Presented frames | 90–92 | 89–90 |
| Longest inter-frame gap | 42–46 ms | 37–75 ms |
| First frame after Play | 1–14 ms | 87–178 ms |

All ten runs meet the original cadence bound (today's longest gap plus one
30 fps frame, and at least today's frame count minus one). The gate also checks
distinct frame times and first-frame latency against today with that same
one-frame alignment allowance. Preparation never exceeded two in-flight jobs;
no native compatibility miss arrived during those playback windows. The
focused transport, raster, kernel, presenter and anonymized-fixture unit set
passes 182 tests.

### Round six: bases that change every frame (2026-10-22)

The round-five cadence fixture rotates a static ellipse and reuses its base.
Three additional gates exercise changing pixels on all 90 composition frames:
the unchanged compat-typewriter override with progress keyed 0 to 100% and a
new typed prefix each frame; shape-tint over an ellipse whose size animates
from 60×40 to 200×120; and shape-tint over a paths layer with animated vertex
and tangent channels. Each fixture verifies 90 distinct evaluated base states.

Both engine and today play the full three-second composition span, ending at
the last frame (six-second deadline, no frames after a wrap). This avoids
comparing today's startup delay with the engine's already advancing clock.
The today-relative bounds stay: longest gap, first frame and completion at
most today plus 34 ms; presented and distinct frame counts at least today
minus one. The changing-base gates additionally require new asynchronous
base work throughout the span and no synchronous base loads during playback.

The final campaign runs each changing-base gate ten times with two SHA256
workers, one Playwright worker, the shipping thin-LTO native addon and the
Electron slot lock. The complete public suite includes the original static
rotation gate and all three changing-base gates. Worker pixel fidelity has a
separate gate covering alpha ramps and antialiasing in sRGB and Display P3,
requiring exact pixel and geometry equality against synchronous readback.

Measured 2026-10-22 on the final tree: two campaigns of ten loaded runs per
gate, each with two SHA256 workers. Other lanes' Electron and render runs
shared the machine, with a load average of 10 to 27. Engine against today in
the same run, all twenty runs:

| Gate | Pass | Distinct frames | Longest gap | First frame | Last frame |
| --- | --- | --- | --- | --- | --- |
| Typewriter keyed 0→100% | 19/20 | 65–90 vs 90 | 44–467 (passing 44–53) vs 43–54 ms | 6–46 vs 127–230 ms | 3003–3018 vs 3110–3211 ms |
| Animated ellipse size | 19/20 | 89–90 vs 90 | 42–84 (passing 42–75) vs 35–42 ms | 2–20 vs 128–211 ms | 3003–3020 vs 3101–3184 ms |
| Animated path vertices | 20/20 | 90 vs 90 | 42–52 vs 35–43 ms | 1–46 vs 132–231 ms | 3002–3018 vs 3100–3198 ms |

No run had a synchronous base load during playback. The paused warm-up
prepared 14 to 21 bases before Play, and 71 to 95 more were prepared
asynchronously during playback. An earlier campaign of the same renderer
code passed 30/30, with two leftover load workers making four in all. That
campaign predates the native schedule fix below.

The two failures are not compatibility misses:

- Typewriter (one run): a stall of the whole pipeline from frame 44 to 58,
  with a 467 ms gap. The engine asked for 14 answers, its schedule skipped 5
  late frames and the presenter dropped 8 late ones. No renderer task ran
  past 50 ms. Today's run, measured minutes apart, did not meet a stall.
- Animated size (one run): one frame near frame 73 was dropped late, and the
  engine asked for no answer. In 5 of the 20 runs, the failing one included,
  one frame between 67 and 73 was lost; the passing four stayed within the
  one-frame allowance. It is not explained yet. Bounding the lookahead to the
  composition's span did not remove it (reverted, see the history).

A loaded campaign before the schedule fix showed a worse collapse: one
render took 530 ms of GPU time. The playback schedule's cost estimate then
exceeded the whole lead window, so every later frame was skipped at its due
time. Nothing rendered again, and the estimate never recovered: one frame in
three seconds, 191 skipped. This is the "1 or 2 frames" collapse from the
round-five review. `pm-node` `present/schedule.rs` now skips on the estimate
only while a frame rendered at its due time could still be shown.
Otherwise it renders, and the measured costs bring the estimate back.
Frames whose interval passed are still skipped.

The campaign on the build before the 500 ms warm-up and the asynchronous
snapshot round passed 7/10, 9/10 and 10/10. With a 150 ms warm window only
five bases were ready at Play, and the typewriter skipped frames 4 and 5 at
the start (89 to 167 ms gaps). The public set on the final tree passed 34
gates: the 30 original gates, the readback gate and the three changing-base
gates. One skip is expected: the C1 private-setup gate, which is named and
skips with the private corpus variable unset.

Round six left three items open: Play right after a seek, the lookahead past
the end, and base rasters on the native render thread. Round seven closes
all three (below).

### Round seven: rasters off the presentation thread (2026-10-22)

**Why size frames 67 to 73 were lost.** A traced campaign of the animated
size gate (with its traces) showed the
native render thread starting timed frames 58 ms apart, instead of 33 ms,
for k 59 to 70, and nowhere else. During those frames the lookahead, 18 to
24 frames ahead, asked for the bases of frames 80 to 113: the largest
ellipse sizes, held at 200×120 past the composition's end. Each base was
rasterized on the CPU, on the render thread, at about 4× density: over
1 MiB, published through shared memory, about 22 ms unloaded
(`snapshotUsMax`). Under a load average of 10 to 27 such a raster took long
enough to push the next timed frame past its delivery deadline. That frame
then either reached the viewer late (dropped `late`) or found its answer
missing and was skipped. The other gates' bases are smaller and never
crossed that point, so only the size gate lost frames, always in the same
window. Bounding the lookahead alone (round six) removed the bases past the
end but not those of frames 80 to 89.

**Layer rasters on a worker.** `present/rasters.rs` draws every layer
snapshot (the `render.raster()` base) on its own thread with a second
`Renderer` on the same device: its own evaluator, text context and fonts
(`RenderFonts::sibling`: the session's font seeds, its own health), its own
command buffer, and a readback that waits for that submission only
(`pm_gpu::readback` now waits for its own submission index, not the
queue's latest). It answers main itself; the render thread hears what it
published (`Command::RasterDone`) for the shared-memory accounting. The
document was shared as an `Arc` of a deep copy made per revision on the
render thread; round eight replaced that with a mirror (below). A rebuilt device gets a new renderer. Sync and async layer jobs leave
the snapshot queue for the worker without waiting for a playing view's next
frame (`SnapshotQueue::next_layer`); idle ones still wait; at most eight are
in flight. The pixels come from the same `Renderer::layer_raster` over the
same revision, so the parity gates are unchanged. In the traced loaded runs
a base took 0.7 to 38 ms on the worker (the path and size gates' 15 to 25 ms
on the CPU). Timed frames started at most 34 to 41 ms apart in the size
gate, against 58 ms before.

**A frame waiting for its answers keeps its slot.** A timed frame whose
compatibility answers were missing used to be passed over at once, even
with 95 ms to spare (traced: frame 32 tried 104 ms after Play, shown at
199 ms). It now waits; an arriving answer wakes the thread, and the
schedule skips it only once it can no longer be shown.

**The native preroll.** With compatibility producers, a Play whose first
six frames are not all answered holds the stopped line for at most 80 ms.
Each viewer render records the needs it consulted (`CompatView::consulted`),
and the preroll checks them at the run's first frames' times. A Play that
is already answered (the paused warm-up filled it) starts at once, and the
preroll ends before the pass's clock sample. Otherwise the run's first
timed frame reached the viewer before the sample of its run and was dropped
as `stopped`: one 83 ms gap in ten runs before that fix. After a cold seek
the window's answers arrive two at a time about every 50 ms. Its
round-trip, not the 1 to 10 ms native text base, is the bound, so the preroll usually
starts the line at its bound with two to three frames answered. The waiting
frames above cover the rest. (Round seven reported the engine's first
frame at 37 to 70 ms after Play, against today's 106 to 216 ms. Those times
were re-presentations of the paused frame, not the run's first frame:
round eight below.)

**The lookahead stops at the end.** Both horizons are bounded by the open
composition's span (its loop range, else the composition), as in round
six's reverted change. The gates no longer count raw answer calls since the
playhead reset. Those counts included answers for frames past the end and
missed answers the paused warm-up had posted earlier. The transport
remembers the latest answer it posted per layer and frame
(`EngineRenderTransport.answeredAt`). Each gate requires the window's
answer for the overridden layer at every presented time, at the play's
revision, and pixels or reuses for all but one of them. No bound changed.

**New gate.** "Play right after a seek with the typewriter override" rests
at 0, then seeks to 1 s and plays in the same task, so nothing from 1 s on
was prepared. It holds the same today-relative bounds, and also requires no
skipped frame among the run's first six and no synchronous base load.

**Measured 2026-10-22 on the final tree.** Two campaigns of ten loaded
runs per gate, each with two SHA256 workers, one Playwright worker, the
release thin-LTO addon and the Electron slot lock. Other lanes shared the
machine, with a 1-minute load average of 10 to 40 per run. Engine against
today in the same run:

| Gate | Pass | Distinct frames | Longest gap | First frame | Last frame |
| --- | --- | --- | --- | --- | --- |
| Typewriter keyed 0→100% | 20/20 | 90 vs 90 | 43–57 vs 41–52 ms | 6–53 vs 98–258 ms | 3002–3018 vs 3072–3236 ms |
| Animated ellipse size | 20/20 | 90 vs 90 | 42–50 vs 35–43 ms | 2–37 vs 102–239 ms | 3002–3018 vs 3073–3209 ms |
| Animated path vertices | 20/20 | 90 vs 90 | 42–51 vs 35–42 ms | 1–48 vs 104–281 ms | 3001–3018 vs 3072–3249 ms |
| Typewriter, Play right after a seek | 20/20 | 60 vs 60 | 42–64 vs 39–48 ms | 37–93 vs 106–360 ms | 2083–2097 vs 2075–2329 ms |

The "First frame" column times the first `frame:rendered` after Play. With
the engine that was a repeated presentation of the paused frame (the Play's
own seek, and answers arriving during the preroll; round eight), so it
overstates the engine's lead. Round eight gates the run's first advancing
frame instead.

No run skipped a frame, dropped one in the presenter, or loaded a base
synchronously during playback. Every presented time had the window's
answer. An earlier campaign, on the build before the preroll's
clock-sample fix, passed the three required gates 30/30 and the seek gate
9/10 (the `stopped` drop above). The public set passes 35: the 34 of round
six and the seek gate. The C1 private-setup gate skips by name with the
private corpus variable unset.

**Campaign hygiene.** The campaign harness waits for any other SHA256 load
generator to end before it starts and records the load average before and
after. Each run prints `os.loadavg()`. Only the harness's own two workers
are stopped afterwards.

### Round eight: copies, repeated frames, loops, redraws (2026-10-22)

Merged `feat/native-engine` (164 commits) first. Integration's readback
rewrite (an asynchronous ring with a completion thread) kept this lane's
per-submission wait for serial readbacks (`Readback::with_mapped_rows`
waits for its own submission index). `CompatFrameRequest` carries both
`frame` (this lane) and `blender` (Blender PNG maps). The job engine refuses
a PNG answer for a layer. `CompatPending` is a non-transient render error.
The viewer is 1.3.9. PROTOCOL_BUILD `protocol-ed91b6f951dbf4be`.

**No document copy per revision.** Round seven gave the raster worker an
`Arc` of a deep copy of the session document, made on the render thread at
every new revision a raster was drawn at. Review measured the copy at
13.8 ms for 1.7 MB of JSON and 154.6 ms for 17.2 MB. The session document
(`ViewDocument::doc`) is now an `Arc` that updates apply in place
(`Arc::make_mut`; a resync replaces it without copying). The worker keeps
its own mirror of each session it draws for, fed by the same updates the
render thread applies (`Work::Update`), as the audio thread's mirror is.
The first raster of a session seeds the mirror with the render thread's
`Arc`, and the worker makes the copy (`Arc::unwrap_or_clone`). A job names
its revision. A mirror at another revision, or none, sends the job back
(`Command::RasterReseed`) to be seeded again. An update the mirror cannot
take drops it. The render thread copies only when an update arrives while
a seed is still held (`documentCopies`). The render-thread fallback borrows
the document again and loads the session's definitions, as before round
seven. The worker's in-flight results count against the snapshot queue's
1 GiB bound (`SnapshotQueue::set_in_worker`).

**No repeated `frame:rendered`.** Three causes, all fixed:

- An answer for another frame (the lookahead, a preroll) set the view
  dirty and re-presented the complete paused frame. Answers now redraw
  only a frame waiting for them: a timed frame kept in its slot, or a
  paused frame that lacked answers (`View::compat_paused_wait`, which also
  ends the paused frame's per-turn retry; it asks again after
  `ASK_AGAIN`). A changed producer list, an invalidation (`Forget`), and an
  answer that replaces the shown one at the same revision
  (`Applied::Replaced`) still redraw. The same producer list sent again is
  no change.
- Play sent an exact seek to the time already shown, and the engine
  presented that paused frame again just before the run did. Play from a
  stopped line no longer seeks it to where the latest clock sample (newer
  than any play or pause sent) says it rests (`EngineTransport.shuttle`).
- A seek and a Play in one task drew the paused frame while the Play
  waited in its preroll. Paused redraws now wait while a Play is in its
  preroll or waits for the audio's first sample, without polling.

The preroll checks the frames a reverse or faster run shows. The cadence
gates assert that no time is presented more often than today
(`repeatCount`), and compare the run's first advancing frame
(`firstAdvanceMs`) with today's.

**Loops.** Looping playback is today's default (`PM.loop`). Both
lookahead horizons stopped at the loop's end. A need for the first frame
after the wrap, asked while the clock still showed the last ones, counted
as stale. Lookahead distances now run along the loop (`runAhead`,
`intoLoop`): the frames after the wrap follow the last ones, the wrap does
not restart the windows, and a nested composition's entries wrap with the
clock's shift. `span()` says when the clock loops (`loop: true` from the
clock's `loopRange`). The engine reports store evictions
(`compatEvicted`). A new gate plays a 10 s loop of 300 viewer-sized
rasters, more than the 256 MiB store keeps, and 45 frames past the wrap.

That gate first found a worse failure than the wrap. Each 1100×600
ellipse base took 26 ms on the CPU (91 ms at twice the density), so
preparations fell behind the clock. The lookahead kept preparing the frame
at the clock, each result came back after the clock had passed it, and
playback presented nothing for 7.7 s (232 frames skipped) until the wrap.
Two fixes:

- Canvas2D rasters (`pm-vector`, outside this lane): triangle fills write
  only each sample row's covered span, using the same exact rational
  thresholds as `fill_polygons`. A solid unclipped paint blends through a
  256-entry table. The box resolve is specialized for one and four samples.
  Pixels are unchanged: tests compare spans with the per-pixel point test
  on 400 random and fan cases, the table with per-sample blending, and the
  resolve with the mean, and every pm-vector parity test passes. The
  ellipse now takes 4.5 ms (16 ms at twice the density).
- The lookahead keeps a running average of a preparation's time. While the
  line moves, it starts the horizon at the first frame a preparation can
  still reach in time. A Play held in the preroll still prepares the frames
  at the clock.

**Paused redraws.** Today's frame loop renders on every draw request while
paused (a selection change, `transport.invalidate()`). It calls every
raster wrapper again and announces the frame with `frame:rendered`. With
the engine, nothing was asked or announced without an edit. Now, on a
paused draw with no new revision, view or time, the window answers the
frame on screen again (`EngineRenderTransport.refreshShown`, compared per
exact need, layer, time, source, density and window). A wrapper whose
output changed has its layer forgotten first, so stale answers at other
densities, for example a capture's, are asked again, and the engine draws
and presents the frame once. Otherwise the frame on screen is announced
again (`PresentedEvents.redraw`: `preview:presented`, then `overlay`), once
per draw. The legacy frame loop calls it (`PM.EngineRedraw`). A paused edit
also starts preparing the shown frame's answers at once
(`prepareShown`), beside the engine's render that will ask for them.

**Measured on the final tree** (`1ec57f9e3` and the fixes after it; the
release thin-LTO addon; the Electron lock; one Playwright worker; load
average 12 to 36 from other lanes). One run of the five spec files had
25 passed and 2 failed. Both failures are listed under "What remains".
Engine against today in the same run:

| Gate | Distinct frames | Longest gap | First advancing frame | Times presented twice |
| --- | --- | --- | --- | --- |
| Rotation | 90 vs 90 | 43 vs 36 ms | 81 vs 166 ms | 0 vs 0 |
| Typewriter keyed 0→100% | 90 vs 90 | 42 vs 43 ms | 86 vs 179 ms | 0 vs 0 |
| Ellipse size | 90 vs 90 | 44 vs 35 ms | 72 vs 189 ms | 0 vs 0 |
| Path vertices | 90 vs 90 | 44 vs 42 ms | 77 vs 221 ms | 0 vs 0 |
| Play right after a seek (frame 30 first shown at 99 vs 147 ms) | 60 vs 60 | 47 vs 43 ms | 124 vs 181 ms | 0 vs 0 |
| 10 s loop, then 45 frames past the wrap | 300 + 45 vs 300 + 45 | 48 vs 38 ms | 69 vs 197 ms | 0 vs 0 |

At the loop's wrap the gap was 34 ms (today 35 ms), and the longest gap
after it 46 ms (today 35 ms). The engine evicted 275 rasters before the
wrap, and the window had answered every frame after it. Earlier runs of
the same gate measured wrap gaps of 20 and 27 ms. No gate skipped a frame
or loaded a base synchronously during playback. In the paused redraw gate,
the engine and today each sent one `frame:rendered` for a no-op draw and
one for a changed wrapper, and showed the new tint at the ellipse's centre.

**Edit drag on a large document** (5.46 MB of JSON: the overridden ellipse
and 2,500 switched-off shapes with keyframes; 60 moves, each waiting for
its frame). The worker drew about 119 bases off the render thread. The
render thread copied the document 0 times. There were no reseeds and no
render-thread turn of 50 ms or more during the drag. Every move was
presented. Latency from the edit to its `frame:rendered`, p50/p95/max:

| Run | Engine with the override | Engine without it | Today |
| --- | --- | --- | --- |
| Before `prepareShown` | 42 / 51 / 60 ms | n/a | 18 / 21 / 29 ms |
| With it | 51 / 67 / 75 ms | 42 / 44 / 50 ms | 15 / 18 / 25 ms |
| Final run | 50 / 57 / 59 ms | 25 / 34 / 35 ms | 14 / 16 / 22 ms |

The engine is slower than today with and without the override. These are
gaps (below).

### Round nine: a sync capture owns its needs, a preview owns its canvas

**Synchronous captures.** The timeout came from outstanding-ask bookkeeping,
not the raster worker, its mirror, fonts, or the asynchronous GPU readback.
The viewer could already have asked for the capture's exact raster and
started preparing it asynchronously. `CompatStore::to_ask` suppressed that
need in the snapshot's reply and put the job in `waiting_for`. The window
was blocked in `sendSync`, so it could not finish the preparation that
would release the job. The first capture expired at the unchanged 5 s bound.

`SnapshotRequest.compat_uploaded` now names the needs the capture itself
answered and posted after its last dependency round. The engine returns
every missing need not in that list, including a viewer's outstanding one.
Only an explicitly posted answer whose same-class snapshot ask is still
outstanding at this revision waits for the two ports' delivery race. A
terminal answer clears that ask, including a reuse rejected after eviction.
A later viewer or asynchronous thumbnail ask cannot make a synchronous
retry wait for work that needs its blocked window. The queued job's wait
tag carries the same class and revision fence, so a replacement ask also
wakes a retry that was already waiting when the old answer was rejected.
Synchronous, main-owned-document, and asynchronous capture rounds all carry
the list. It has the same 256-need bound and scale/crop validation as a
compatibility request. Viewer ask suppression, snapshot deadlines, and
queue/memory bounds are unchanged. A native regression leaves a real viewer
need unanswered, requires the synchronous capture to return it, marks the
retry, and verifies that the independently arriving pixels complete it.
Store regressions also cover eviction, a rejected reuse, a newer viewer or
thumbnail ask, and a marker from another revision.

**Deep zoom.** Three separate causes had been hidden by the initial missing
high-density callback:

- The original typewriter fixture's identity fragment has no `viewportSafe`
  declaration. Today's viewer correctly retains a composition-sized canvas
  for it, so screen zoom alone cannot require density 24 or 32. The original
  entry and its metadata remain covered by the other compatibility gates;
  the zoom gate uses a separate entry that declares this identity fragment
  safe. It pins full preview quality and supplies a matching canvas for the
  custom 16x request instead of inheriting the built-in viewer's 8x canvas.
- At 16x on a 2x display, that canvas derives density 32. Internal
  `RenderRequest::validate` incorrectly applied the ordinary frame scale
  cap of 16 before compatibility planning. A preview canvas now accepts its
  finite positive derived density. Wire frame requests, exports, and final
  captures keep their scale limits, and texture/allocation bounds still
  apply. The validation regression checks each boundary and invalid axis.
- Source crop planning used the screen-visible render region. Today's
  compositor passes wrappers the complete canvas, including its pan buffer.
  `RenderRequest.preview_canvas` carries that distinction: source clipping
  uses the complete canvas while presentation still renders just its region.
  A native regression distinguishes the 512-square canvas from its central
  256-square presentation region and keeps exports uncropped.

**Synchronous viewport picking.** The full public set also exposed a
regression in all four viewport API contexts: `glyphsIn` returned `[]`
before the first matching native chrome frame. Integration had put the
native-readiness return ahead of hit-geometry updates. Pending glyph hits
now use strict synchronous wasm `active/world3D` queries, as the other
viewport hit tests do, and `glyphsIn` flushes scheduled geometry updates.
Unavailable viewports clear their cached hits. Drawing still waits for
matched native chrome metadata. The flag-on path never constructs the
reference scene. Picking also refreshes geometry at the caller's current
time when an older presented frame has the same revision and view sequence.
The compiled probes check an immediate seek out of a light's active range
and back in the same task, in addition to the original 104 member checks.
Unit coverage checks current transforms and time, exclusions, and an
unavailable native API.

On the rebuilt thin-LTO addon (`protocol-8e74d7bb9f0c4b33`), the two blocker
gates pass together under two SHA256 load workers. At 8x the plain text's
crop is `(2048,768,2304,1536)` at density 24; at 16x it is
`(2816,1280,1280,1024)` at density 32. Both equal the legacy function applied
to the actual canvas, including its pan buffer. The typed layer gets no
crop, frames match the requested time and current view sequence, and text
pixels change after seeking. The synchronous empty/partial/full captures
match today's geometry and pixels (max 1, no channel values over 1).

The final loaded cadence campaign runs each changing-base gate 20 times,
with two SHA256 workers, one Playwright worker, and native/viewer traces.
All 60 runs pass the original assertions. Each presents 90 distinct frames,
all answered by the window, with no skipped frames, presenter drops,
repeated engine times, or synchronous base loads. Measured ranges:

| Gate | Pass | Longest gap, engine / today | First advancing frame, engine / today |
| --- | --- | --- | --- |
| Typewriter keyed 0→100% | 20/20 | 42–51 / 44–52 ms | 76–90 / 196–320 ms |
| Animated ellipse size | 20/20 | 42–50 / 35–43 ms | 61–85 / 159–323 ms |
| Animated path vertices | 20/20 | 42–49 / 35–43 ms | 67–87 / 209–303 ms |

The per-run one-minute load average ranges from 17.9 to 39.7. An initial
campaign passed 59/60: one size run showed 72 distinct frames and a 329 ms
gap (today 90 and 43 ms), nine late presenter drops, and no main-thread long
task. That run had no frame trace, so its cause is unproven. It is retained
in the report rather than attributed to readback or canvas refinement.
The final traced size campaign has no recurrence; its longest timed native
render is 18.2 ms. Diagnostics now retain complete preparation, bitmap,
worker-reply and worker-conversion timings, and every cadence run collects
native status after the measured span instead of relying on the last
periodic sample.

The final capture campaign passes 10/10 synchronous typewriter runs alone
and 10/10 zoom-then-synchronous-capture pairs, under the same two SHA256
load workers. The raw report confirms their actual start-time order, one
Playwright worker, no retries, and no skips. All empty/partial/full pixel
comparisons stay within 1, with no channel value over 1. These run after
the complete public set in one serial, prebuilt Playwright command.

All 38 public gates pass across the seven compatibility spec files,
including all four compiled viewport contexts, the 104 member probes and
the immediate-seek checks. The long loop presents 300 distinct frames and
45 after wrapping, with no skips, repeated engine times, or presenter
drops. Its longest engine gap is 48 ms (today 35 ms); the wrap gap is 28 ms
(today 32 ms). The large-document gate presents all 60 edits with no extra
document copies, reseeds, or render-thread turns over 100 ms. Its engine
p50/p95/max is 52/66/86 ms, without the override 33/35/36 ms, and today
18/22/28 ms; the latency-parity gap remains.

The final native build passes all 154 `pm-node` library tests and all 14
snapshot parent regressions, including the withheld viewer need and the
queued upload rejected after eviction. The existing parent tests invoke
their isolated child processes; no public gate is skipped.

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
  now bounded at today's 32 independently of output-frame scaling for the
  compositor's compatibility messages. Bad densities there produce
  `invalidMessage` in `EngineConnection.status().failures` and the viewer's
  report line. (Round five: a public `render.raster` takes any scale, and an
  extension's failed snapshot no longer reaches the viewer; see below.) The old hidden legacy-window comparison was not a zoomed
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
  The member inventory is exhaustive against the public interfaces. Native
  3D geometry and chrome-plane rendering have separate gates.
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
- **Paused edit latency on a large document** (round eight's drag): with
  the override, the engine shows an edit 42 to 51 ms after it (p50); today
  shows it in 14 to 18 ms. Without the override the engine took 25 to
  42 ms in two runs, so part of the difference is the engine's own edit
  path (patch, render, presentation: the presenter and document lanes).
  The override adds 9 to 25 ms at p50: the window's base preparation and
  the answer's round trip, now started at the edit (`prepareShown`).
- A paused redraw re-asks only the needs the engine asked for most
  recently (the frame on screen). A wrapper whose output follows its own
  state is asked again for other frames and densities only after a
  change it shows at the frame on screen (its layer is then forgotten).
- `main/codex/instructions.test.ts` fails on integration too: its golden
  and its 10 KB budget (11,576 bytes; 343 of them this lane's deprecation
  guidance). The update needs the agents lane.

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
| Synchronous `render.raster` at 16.25, 24, 32 | Geometry equal to today; max 0 (round four refused 33; superseded in round five) |
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

C1 is `e2e/engine-private-setup-compat.spec.ts`. Set `PM_PRIVATE_SETUP_DIR`
to an external corpus containing `compat-manifest.json`, `extensions/`, and the
project file named by the manifest. The manifest holds the expected extension
IDs, project metadata, reference times, recovery state, and private probes.
The test installs those extension folders into disposable profiles, then opens
the project off and on. Its result ledger is
`$TMPDIR/powermove-private-setup-compat/results.json`, outside Git. It records
per-extension activation, health, contribution ids, winners, panels, safe
open/read commands, the workspace theme and tools, wrapper presence and
project frames. It uses the manifest's exact reference times at full project
size with the corpus's motion-blur settings and unchanged section-25 bounds.
Any declared build failure is checked by name on both paths; no other failure
or runtime auto-disable is excused.

The isolated profile applies the manifest's recovery state before activation,
so the private source remains untouched. The activation barrier waits for actual loader activations,
not main's successful compilation records. No live profile is read or written.
A missing `PM_PRIVATE_SETUP_DIR` yields a named skip. With the corpus present, run its
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

The private schedule lives in the external manifest and uses full composition
size. Reused baseline
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

## Round five: public rasters at any scale, the viewer keeps its frame

**Any scale, as today.** Today's `PM.raster` (legacy `gl/raster.ts`) has no
upper bound on `scale`: every rasterizer takes
`density = min(scale, 8192 / w, 8192 / h)` (`textRasterGeometry`,
`shapeRasterGeometry`, `rasterPaths`, the captions rasterizer, which also
floors at 0.05). The 32 of `continuousRasterScale` bounds only what the
compositor asks for. Round four had put `0.01..32` on the public path too,
so `render.raster(layer, 40)` returned null. Now:

- `pm_protocol::snapshot::valid_layer_raster_scale`: a `Layer` snapshot
  takes any positive finite scale (main's `validSnapshotRequest` agrees).
  `MAX_RASTER_SCALE` (32) stays on the compositor's compatibility messages
  (`CompatNeed`, `Native`, `Reuse`, `CompatPixels`, `CompatFrameRequest`,
  `CompatFrame`), which only carry `continuousRasterScale`.
- The engine's rasterizers already applied today's cap
  (`layer_raster.rs` `js_density`, `TextRasterizer` text/animated
  text/captions, `pm_vector::shape::ShapeRaster`, `raster_paths`).
- `layerRasterScale` (render-transport.ts) sends a positive scale as is up
  to `Number.MAX_SAFE_INTEGER`, and anything above it (Infinity, 1e300) as
  that: every number above 2^53 is an integer main's MessagePack encoder
  refuses (the first e2e run of this gate sent `Number.MAX_VALUE` and the
  request went unanswered for 15 s), and any raster box wider than 1e-12
  pixels meets the 8192 cap far below it. Zero, negatives and NaN, whose raster today is a canvas of at most one
  pixel with nothing on it, as `VANISHING_RASTER_SCALE` (1e-9). At that
  density the text rasterizer can leave one faint pixel (alpha 82
  measured), so the raster surface clears non-positive requests to today's
  transparent pixel (`withoutInk`; captions keep their 0.05 floor).
  `engineRasterBase` no longer turns scale 0 into 1.
- Known difference kept: today's NaN canvas is 0×0 and a non-positive
  paths raster throws (an invalid canvas size); the engine returns the
  one-pixel transparent surface with today's geometry for both.

**The viewer keeps its frame.** `DocumentSession.reportFailure(code,
message, at, frame)`: an extension's failed snapshot (`render.raster`,
`render.snapshot`, `renderFrameTo`, through the transport's `onFailure`) is
recorded in `EngineConnection.status().failures` and logged, but never sent
to the viewer as `{type:'failure'}`, which hides the canvas until the next
frame. The viewer's own frame failures (the engine's session report, a
raster producer answering the compositor) still name themselves on the
stage.

**Tests.** pm-protocol `a_public_layer_raster_takes_any_positive_scale_as_today`;
pm-node `snapshot` (a 40×20 box at 40, 50, 1000, `f64::MAX`, 0.005 and 1e-9:
today's canvas `ceil(48·d) × ceil(28·d)` capped at 8192, anchor, box and
selection unchanged, ink inside) and `render_fonts` (a text layer at 40,
1000, `f64::MAX`: 8192×3594 from a 310×136 base; 1e-9: 1×1); vitest
`layerRasterScale`, `withoutInk`, and the session's frame flag. The e2e
gate `render.raster at any scale returns today's surface…` compares the
engine with today's rasterizer at 16.25, 24, 32, 33, 40, 50 (a 16×12 box),
1000 and Infinity (a 200×2 bar that meets the 8192 edge), 0.005, 0 and -1,
and checks that no failure was recorded and the presentation canvas stays
on the stage.

The oldest-viewer gate now also requires the presentation canvas (engine)
or the viewer's GL canvas (today) to be on the stage, not only readable
through the compatibility context's preserved copies.

### Measured (2026-10-19, round five, e2e under the Electron lock, flag on)

Release addon `protocol-8a10dfb17b4ff9bc`. Targeted run: the any-scale
raster gate, the oldest-viewer gate, the zoom 8/16 gate, label
substitution and the WebM context gate. The first run passed four of five;
`render.raster(bar, Infinity)` timed out after 15 s because the scale went
on the wire as `Number.MAX_VALUE` (fixed: `MAX_SAFE_INTEGER`). The rerun of
the two changed gates passed:

| `render.raster` | Canvas (engine = today) | Pixels | Sync call |
|---|---|---|---|
| box 16×12 at 16.25, 24, 32 | 390×325, 576×480, 768×640 | max 0 | 7, 8, 13 ms |
| box at 33, 40, 50 | 792×660, 960×800, 1200×1000 | max 0 | 14, 20, 30 ms |
| bar 200×2 at 1000, Infinity | 8192×394 (the 8192 cap) | max 0 | 56, 55 ms |
| box at 0.005, 0, -1 | 1×1, no ink on either side | max 0 | ≤ 1 ms |

Box, anchor and selection equal today's in every row. No failure was
recorded, the status line stayed empty, and the presentation canvas stayed
on the stage. The oldest viewer fork and replaces-only folder pass with the
new stage check (`shown: true` on both sides; whole-canvas max 2, 37 of
3,619,008 RGB values over 1, as in round four). Zoom 8/16 (frames 9 → 12 →
13 at 16×, density 32), label substitution and the WebM context gate passed
in the first run.
