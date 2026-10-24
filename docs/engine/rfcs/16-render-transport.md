# RFC 16: The render transport (Phase 3c)

Status: draft, version 0 (extends contracts 2 and 14). Module:
`pm_protocol::snapshot`. Owner: Engine and platform (the bridge lane).
Consumers: the extension compatibility lane (extcompat), the 3D lane, the
consumer UI lane, and the agents lane. Plan sections 18 to 20; owner
amendments 21 to 24 (section 26).

The TypeScript interfaces in [The TypeScript interfaces](#the-typescript-interfaces)
are frozen for the other Phase 3c lanes: they may add members, never change
or remove the ones listed here, without a revision of this RFC.

## Purpose

With "Use New Engine" on, the engine draws every frame, including the ones
today's API returns synchronously (`render.snapshot`, `renderFrameTo`, and
`raster` at apiVersion 1 to 3, amendment 23) and the UI's snapshots
(thumbnails, library cards, posters, the project thumbnail). Pixels only the
renderer can draw (an extension's offscreen WebGL2 compatibility context, a
`raster` override's result, the shader fallback runner) go the other way.
Frame events (`frame:rendered`, `overlay`) fire when the engine's frame is
on screen, and engine stats replace `GL.stats`. Old custom viewers that hand
their own canvas to `render.gl.init` keep working. This RFC is the transport
for all of that.

## Messages

| Message | Direction | Meaning |
|---|---|---|
| `Request::Snapshot(HostSnapshot)` | main → interactive engine, on main's host port only | One frame (or one layer's raster) of the document of `session`, which the window port `renderer` owns, or which main opened itself (`renderer` names main's own host port; see "Sync snapshots the window's session cannot serve"). The envelope's revision is the minimum to render (read-your-writes). Answer: `Response::Snapshot` |
| `Response::Snapshot(SnapshotOutcome)` | engine → main | `rendered` (pixels and tags), `needsRenderer` (what only the renderer can draw: fallback programs, compatibility rasters), or `empty` (a layer target that draws nothing then: today's `raster()` returns null) |
| `Event::Compat(CompatMessage)` | renderer → interactive engine (session-bound) | `pixels`: one row band of a compatibility raster; `forget`: drop a layer's rasters |
| `Request::CompatFrame(CompatFrameRequest)` | job engine → host | A layer's compatibility pixels for an export frame (the host asks the editor windows). Answer: `Response::CompatFrame` |
| `Event::RenderStats(RenderStats)` | interactive engine → renderer (session-bound) | What presenting the session costs, at most 2 Hz while presenting and once after idle |

### Snapshot request

`SnapshotRequest { composition, time (ticks), outputSize, target, pixels,
class, viewSeq, viewer?, waitMs, blenderWaitMs?, captionsHidden?, uploaded }`:

- `target`: `frame { motionBlurSamples, background, draft3d }` or
  `layer { layer, scale, crop, content? }`. `content` is the layer object
  the caller passed, as canonical JSON (at most 4 MiB), when it is not the
  document's own (a raster wrapper's modified copy): the engine draws it in
  the composition in place of layer `layer`.
- `pixels`: the RGBA8 encoding of the result, rows top-down: `opaque`
  (premultiplied over black, alpha 255: today's `renderOpaqueFrame` and the
  input of `toDataURL('image/jpeg')`), `premultiplied`, `straight` (canvas
  `ImageData`).
- `class`: `sync` (a thread is blocked on it: rendered at the first free
  point of the render thread, between viewer frames), `async` (rendered
  once no playing view has a frame due before it would finish; the cost is
  learned from earlier snapshots), `idle` (UI snapshots: rendered only while
  no view plays, no gesture or scrub frame was drawn in the last 200 ms, and
  no `sync` or `async` job waits; one per turn of the render thread).
- `waitMs` (1 to 120,000): how long the engine waits for the revision and
  its turn before it answers `timeout`.
- `blenderWaitMs` (optional, 0 to 900,000; never above 0 for `sync`): a
  `frame` of a composition Blender renders, without `draft3d`. Above 0
  (today's `snapshotAsync`, which runs `prepareFrame`): the engine has
  Blender render the frame's missing surfaces first, through the paused
  viewer's Blender worker and host services' supervised Blender, for at
  most this long; that time does not count against `waitMs`, which counts
  afresh once Blender is done. 0 or absent (today's synchronous capture):
  draws a frame an earlier capture prepared, or fails `blenderFailed`
  with today's words. Main waits at most `2 × waitMs + blenderWaitMs`
  plus its render allowance.
- `captionsHidden` (optional, default false): omit caption layers for this
  frame capture, matching `PM.captionsHidden`. It does not alter the viewer
  or subsequent captures.
- `viewer` (optional): a pinned UI view `{ view: ViewOverride | null,
  canvas: ViewerCanvas, viewSeq }`. It selects the editor camera, shading,
  X-ray and Local View and renders the canvas's backing pixels and composition
  crop before stage scaling. `outputSize` must equal that backing size;
  `viewSeq`, when present at the request's top level, must match the pin.
  This uses paused viewer sampling and motion blur, without changing the live
  viewer or opening a presenter. It is a frame target only. Ordinary export,
  agent and thumbnail requests retain their defaults when it is absent.
- `uploaded`: shader fallback keys the caller drew and posted on the
  session's port after a `needsRenderer` answer (see below).

### Snapshot result

`SnapshotResult { revision, viewSeq, width, height, pixels, payload, origin,
layerBox, raster?, quality, waitedUs, renderUs }`. For a layer target `origin` is the
anchor and `layerBox` the raster's box in layer pixels (today's
`RasterSurface` `anchorX`, `anchorY`, `w`, `h`), and `raster` is
`RasterGeometry { canvas: [w, h], window: [x, y], cropped, selection? }`:
the whole canvas, where the pixels sit in it, whether the crop was applied
(today's raster crops plain text and shapes only), and today's
`selection` (`x0, y0, x1, y1`). pm-render paints every layer kind as
`PM.raster` dispatches (`Renderer::layer_raster`): captions, editable
paths, text (plain or animated), and any other layer as `rasterShape` of
its resolved content. A text layer with no ink is a transparent canvas of
its geometry (as today), a captions layer between cues is `empty`.
`revision` is the exact revision rendered;
`viewSeq` identifies the selected pinned viewer, or echoes a legacy request
without `viewer`. UI consumers reject a result for another selected sequence.
`payload` is `inline { bytes }` up to 1 MiB,
otherwise `sharedMemory { name, length }`: a POSIX segment the engine created
exclusively (mode 0600, `/pm.` and 24 hex digits, 96 random bits). Main maps
it once (pm-bridge `takePixels`), which unlinks it; the engine unlinks
segments nobody took after a minute and at shutdown. Electron's IPC carries
the buffer to the renderer in shared memory again (Mojo big buffers).

### Invariants

1. **Only main asks, only for the window's own session or main's session
   for that window.** The engine refuses `snapshot` from any port but the
   host's (`capabilityRejected`), and refuses a session the named port does
   not own (`unknownSession`); the named port is a renderer port or the
   asking host port itself. Main names the window's port from the IPC
   sender, refuses a session of an earlier engine launch, and keys its own
   sessions by the IPC sender.
2. **Pinned to a revision and a view.** The renderer flushes its pending
   edits to the engine and sends the revision it has; the engine renders
   that revision or a newer one and tags the result with it and with the
   view state the caller saw.
3. **Bounded.** The engine answers every request within `waitMs` plus the
   render; main gives up `waitMs + 10 s` after asking. A request that never
   reaches its revision fails with `timeout`.
4. **Never waits on the renderer.** A frame that needs pixels only the
   renderer draws is answered `needsRenderer`. The renderer draws them on
   the calling thread, posts them on the session's port, and asks again
   naming them in `uploaded`; the engine then waits for those bands (the two
   ports may race) instead of naming them again. At most three rounds.
   Nothing on this path waits for a renderer task, so a call from inside a
   `raster` wrapper or a WebGL callback cannot deadlock.
5. **Memory caps.** Engine: 64 jobs (16 per session), 40 million pixels per
   snapshot, 1 GiB of results queued or published and not taken. Main: 8
   snapshots in flight per window, 1 GiB of results in flight in all, a
   document of at most 512 MiB and 16 fallback programs (256 MiB) per
   request.
5a. **Media first.** A snapshot waits (within `waitMs`) until the host has
   answered for every asset the document names, as viewer frames do.
5b. **Fonts.** Text is laid out on the render thread over the engine's
   font service (the registry on the utility process's main thread,
   answered through a broker; `pm-node/src/fonts/render.rs`). A frame whose
   fonts did not answer in time fails with `timeout`; it never draws text
   with missing fonts. Main-owned snapshot sessions bind to the originating
   window's font seeds before resync; the render scope includes both port
   and session. This waits only on the engine font-service port using main's
   cached seeds, never on the blocked renderer.
6. **Failures are named, never another engine's pixels.** `unsupported`
   (with the engine's reasons), `timeout`, `unknownSession`,
   `budgetExceeded`, `deviceLost`, `invalidMessage`, `internal`,
   `blenderFailed` (a Blender composition captured without `draft3d` and
   without a prepared frame: today's words, "The Blender frame is not
   prepared. Render this frame before capturing it."; or Blender's own
   failure while preparing), `blenderMissing` (Blender not installed); main adds `unavailable` (engine off or not
   running), `refused` (main's caps), and `needsDocument` (see below).
   `render.snapshot` returns `''` only then, and throws today's error for
   `blenderFailed` as today's capture does.

### Sync snapshots the window's session cannot serve

A sync snapshot blocks the renderer, so the window's own session serves it
only when that session has the document and no staged resync is on its way
(a staged resync's next step needs the blocked renderer). Otherwise (no
session yet at boot, a closed viewer, a large document's resync in flight)
the request carries the window's document instead of a session, and main
serves it from a session it opens itself on the interactive engine
(`main/engine/window-sessions.ts`): one per window and engine launch, the
window's definitions from main's relay, the document resynced inline up to
12 MiB and staged above, fallback pixels of a `needsRenderer` round posted
before the next ask. A request whose revision main already has carries
only `{ id, revision }`; main answers `needsDocument` when it lacks it and
the window sends it. Main's session closes with its window, its page, its
engine launch, or after a minute unused. Every wait is bounded by the
request's own.

### Compatibility pixels

`CompatKey { composition, layer, time, source, key }` names one raster:
`source` is `rasterOverride` or `compatContext`, `key` the producer's cache
key (it changes whenever anything that decides the pixels does). Bands carry
`revision, width, height, format (rgba8Premultiplied | rgba8Straight |
rgba16fPremultiplied), origin, scale, row, rows, data` (8 MiB per band).
The interactive engine assembles bands in any order and keeps complete
rasters per session (256 MiB, least recently used first); `forget` and a
closed session drop them.

For exports the job engine sends `compatFrame { document, composition,
layer, time, source, scale, revision }` to the host. Main asks every editor
window; the window whose open document matches and that has a producer for
the source draws the layer and answers; the first pixels win, and when every
window declined the last reason is the answer (`notHeadlessRenderable`).
One answer per ask, 30 s per ask, 32 asks in flight, segments the engine
does not take unlinked after a minute.

### SVG images

SVG images, 3D SVG material maps included, are drawn natively by the
engine at output resolution; no SVG pixels cross the transport.

### Depth and object-id surfaces

`ViewerState.depthId: true` (the 3D viewport chrome) makes the interactive
engine create a depth pool for the view: `rgbaf16` surfaces at the
viewer's size, as many as the frame pool, handed to main and the window
like the frame pool and announced as `SurfacePool { role: depthId }` (a
depth pool never replaces the frame pool). Each frame renders its depth
and object-id surface (r depth, g and b the object id minus one, a
coverage) with its color, placed exactly like it and never filtered, into
slot `seq % count`, and names it in `FrameDescriptor.depthId { poolIndex,
seq, poolGeneration }`. A slot is reused only after `count` newer frames,
more than are ever in flight, so the surface of the frame on screen is not
overwritten while it shows; no release message is needed. The pool is
charged to the lease, retired when the viewer stops asking, and remade
after a GPU loss.

### Stats

`RenderStats { frames, timedFrames, lateSkipped, cpuUs, gpuUs, layers,
frameUsAvg, programs, rasters, layerFailures, snapshots, snapshotUsMax,
revision, audio? }` per session. `audio` carries the session’s optional
`{ error?, problems, starvedRenders, underruns, deviceOverloads }` diagnostics.
Changes are reported even while paused and before the surface pool arrives,
at most twice a second. `frameUsAvg` is today's `E.ms` average (0.85/0.15)
of CPU plus GPU time.

## The TypeScript interfaces

### Preload (`window.powermove.nativeEngine`, `shared/engine-ipc.ts`)

```ts
snapshotSync?(request: EngineSnapshotRequest): EngineSnapshotReply; // never throws
snapshot?(request: EngineSnapshotRequest): Promise<EngineSnapshotReply>; // never rejects
onCompatAsk?(listener: (ask: EngineCompatAsk) => void): () => void;
answerCompat?(answer: EngineCompatAnswer): void;

type EngineSnapshotRequest = {
  generation: number; session: number | null; revision: number | null; snapshot: SnapshotRequest;
  document?: { id: string; revision: number; hash?: string | null; json?: Uint8Array | null }; // with session: null
  fallbackPixels?: { key: string; width: number; height: number; rgba16f: Uint8Array; rgba32f?: Uint8Array }[]; // document requests only; rgba16f empty at depth 32
};
type EngineSnapshotReply =
  | { status: 'rendered'; revision; viewSeq; width; height; pixels; data: Uint8Array; origin: [number, number];
      layerBox: [number, number] | null; raster: RasterGeometry | null; quality: FrameQuality; waitedUs; renderUs; mainMs }
  | { status: 'needsRenderer'; revision; fallback: ShaderFallbackRequest[]; compat: CompatNeed[] }
  | { status: 'empty'; revision }
  | { status: 'failed'; code: string; message: string };
```

### Renderer (`PM.EngineRender`, `engine-connection/render-transport.ts`)

`EngineRenderTransport`, present only with the new engine on:

```ts
readonly ready: boolean;
snapshotSync(options: EngineSnapshotOptions): EngineSnapshot;          // throws EngineSnapshotError
snapshot(options: EngineSnapshotOptions, cls?: 'async' | 'idle'): Promise<EngineSnapshot>;
renderFrameToSync(time, width, height, options?): HTMLCanvasElement | null; // today's renderOpaqueFrame
renderFrameToCanvasSync(time, width, height, options?): HTMLCanvasElement; // throws EngineSnapshotError
renderFrameTo(time, width, height, options?): Promise<HTMLCanvasElement>;
snapshotDataUrlSync(time, maxWidth, size: { w; h }, options?: { captionsHidden?: boolean }): string;          // JPEG 0.74, '' only on failure (throws for blenderFailed)
snapshotDataUrl(time, maxWidth, size, options?: { prepareBlender?: boolean; captionsHidden?: boolean }): Promise<string>; // prepareBlender: today's snapshotAsync
currentRevision(): number | null;                                      // flushes pending edits
rasterSync(layer: string, scale, time, crop?, options?: RasterOptions): EngineSnapshot | null;  // null: draws nothing then
raster(layer: string, scale, time, crop?, options?: RasterOptions): Promise<EngineSnapshot | null>;
toCanvas(snapshot: EngineSnapshot): HTMLCanvasElement;
readonly thumbnails: EngineThumbnails;                                  // idle class, stale-rejecting
uploadCompat(upload): boolean;
forgetCompat(layer, composition?): boolean;
provideCompat(source: CompatSource, producer: CompatProducer): () => void;
readonly stats: TransportStats;

type EngineSnapshotOptions = { time; width; height; composition?; pixels?; motionBlurSamples?; captionsHidden?; background?; draft3d?; waitMs?;
  prepareBlender?;  // async and idle frames: Blender renders the frame first (only in documents with a Blender composition)
  layer?: { id: string; scale: number; crop?: { x; y; width; height } | null; content?: Uint8Array | null } };
type RasterOptions = { composition?: string; content?: Uint8Array | null; maxEdge?: number };
type EngineSnapshot = { width; height; data: Uint8ClampedArray; pixels; revision; viewSeq; origin; layerBox: [number, number] | null;
  raster: RasterGeometry | null; quality; waitedUs; renderUs; mainMs };
class EngineSnapshotError extends Error { code: string }

EngineThumbnails.request({ key, ...EngineSnapshotOptions, minRevision? }, revision?): Promise<EngineSnapshot | null>; // null: stale or superseded; default minimum: the window's revision when asked
EngineThumbnails.cancel(key): void;
```

### Kernel render facade (`kernel/install.ts`)

With the engine on: `render.snapshot` and `render.renderFrameTo` come from
the engine through `snapshotSync`; `project.snapshot` through `snapshot`
(today's synchronous capture: an unprepared Blender frame rejects with
today's words, other failures are `''`); `render.snapshotAsync` through
`PM.Export.snapshotAsync`, as with the flag off;
`PM.Export.snapshot` and `snapshotAsync` (the agent's look and observe, the
Library's thumbnails) through `PM.EngineCaptures`, which the exporter's own
methods ask first (`engine-connection/app-captures.ts`), so wrappers and
overrides of them reach the engine in any install order.
`PM.EngineCaptures.snapshotAsync` keeps today's contract: it refuses while
an export or another capture runs ("Wait for the current frame capture or
export to finish."), pauses playback around the capture and resumes it,
and prepares a Blender frame first (`prepareBlender`).
`render.raster(layer, scale, time, uploaded, crop)` returns today's
`RasterSurface` (`cv, w, h, anchorX, anchorY, selection`, plus `blank`
and `sourceWindow` for text, `uv` for shapes, and the engine's `revision`)
from the engine (`engine-connection/raster-surface.ts`): while a public
call runs, the raster service's base beneath any extension wrapper is the
engine's raster, so wrappers get engine pixels; the compositor's and the
editing helpers' own `PM.raster` calls keep today's rasterizer. A layer
object that is not the document's own goes with the request as `content`.
apiVersion 4's async siblings are on the render object:
`snapshotAsync(time, maxWidth = 480): Promise<string>`,
`renderFrameToAsync(time, w, h, options?): Promise<HTMLCanvasElement>`,
`rasterAsync(layer, scale = 1, time?, crop?): Promise<RasterSurface | null>`
(the same shape as `raster`, flag on or off; with a wrapper registered it
runs the wrapper chain). Their typings in `kernel/api.ts` belong to the
extcompat lane.

`render.gl.init(canvas)` and `render.gl.resize(…)` bind engine presentation
to the caller's canvas (`PM.EngineCanvas`,
`engine-connection/canvas-binding.ts`). The binding never presents over a
viewer that presents by itself: one whose stage is marked
`data-engine-stage="viewer"` (its presentation) or
`data-engine-self-presents` (`ENGINE_SELF_PRESENTS_ATTRIBUTE`, set by the
built-in viewer from attach until it is disposed, round 6). It ends when
its canvas leaves the document, at once (a ResizeObserver), not only at
the caller's next resize.

### Frame events and stats (`engine-connection/presentation-events.ts`)

Per new engine frame on screen: `preview:presented` (forwarded as
`frame:rendered { time, viewport, version, quality }` with `viewport` the
exact `render.gl.previewViewport` object and `version` the `animVersion` of
the frame's revision), then `overlay`. `PM.EngineStats.perf()` gives today's
`perf` fields (`ms, fps, draws, passes, progs, raster`).

### Viewer service (`EngineViewerService`, engine-present)

`ViewParams.depthId?: boolean` asks for the depth and object-id surfaces;
`ViewerPresentation.depthFrame(frame?)` returns the surface of a frame on
screen as a `VideoFrame` (close it after reading). Added service members:
`exclusive?: boolean` (the kernel's connection sets it: the
viewer never hands the stage, transport, or audio back to the current
engine), `presented?(frame, quality)`, and `revealFrames?: number`
(default 2; 0 for tests that run frames by hand). A canvas that was hidden
shows when its first frame is drawn, but counts that frame as on screen
(`latest`, `presenting`, frame events) only `revealFrames` animation
frames after the drawing reported the frame's GPU work finished
(`Presenter.onDone`; a presentation worker posts `done`), at most 30
frames after it was drawn; a new presenter during the wait starts its
own. `PresentedFrame` gains `width` and `height`. `ViewerPresentation` gains
`exclusive`, `status`, `role` (`viewer` | `binding`), marks its stage
`data-engine-stage`, and dispatches `powermove:engine-stage` when a viewer
presents by itself.

## Versioning

Draft. The Rust types are additive to contracts 2 and 14, but
`PROTOCOL_BUILD` changes with any source change, so lanes regenerate the
TypeScript and rebuild their addons when this lands.

## Open

- **Depth and ID consumers**: the surfaces are delivered
  (`ViewerPresentation.depthFrame`); the 3D viewport chrome reading them is
  the 3D lane's.
- **Compatibility rasters in frames**: the store holds them; pm-render does
  not draw a layer from them yet, so `needsRenderer.compat` is always empty.
- **The job engine's fonts**: the job engine has no font service yet, and
  pm-export gates exports with the fonts-less capability check, so text
  exports still route as before (export lane).
- **Compat level 2 text** needs level 2 fonts on the render thread
  (`TextFonts::level2`); the app creates no level 2 layers yet.

## Headless playback (bridge continuation, 2026-10-18)

`ViewerState.headless?: boolean | null` keeps the session's composition,
transport and pm-audio device alive without a presentation pool or draws.
Absent/false retains presentation. A headless clock is emitted only after
the document and media are ready to accept transport. Its envelope carries
the document revision only while the engine draws the composition (round 6):
for a composition the engine cannot draw it comes untagged, so a tagged
headless clock is how the renderer hears that an edit or a switch made the
composition drawable again (there is no frame to say so). Every transition to ready publishes a clock, including
paused sessions. Each paused headless document update also refreshes that
clock at the new revision, even if its route remains ready. Headless playback
still publishes levels and periodic clocks.

The renderer retains one document session per window. A presentation channel
attaches to it and sends normal viewer states; closing the channel sends a
headless state. Opening mid-play allocates presentation against the existing
clock without a new Play, seek, epoch or audio device. Play/seek/snapshot with
no viewer lazily creates this session and replays pending transport once ready.
A document load synchronizes the UI time before resyncing, replacing any
queued seek from the home screen. The protocol build changes; regenerate
TypeScript and rebuild the addons.

A headless session follows the open composition: switching compositions
inside a project is a document batch, not a load, so the kernel connection
sends a new headless state on `comps`/`project` and before every transport
command (`EngineTransport` option `prepare`). Play right after a switch
plays the composition just opened.

**Compositions the engine cannot draw (round 5).** The route the engine
decides is `engine`, `legacy` (the scaffolding name: something the renderer
cannot draw yet) or `pending`. With "Use New Engine" on, only `pending`
defers transport. For `legacy` the engine runs Play, play around, seeks,
scrub grains and audio exactly as for `engine` (a headless view publishes
its clock), never stops a run on the route change, and draws nothing. The
session-scope `unsupported` failure reads `The new engine cannot draw this
composition yet: <reasons>.`; the engine repeats it when a viewer attaches
to a viewerless session, and the kernel connection replays it to a viewer
that replaces another (`missedRoute`). An exclusive presentation shows that
sentence as the stage's status line, keeps the current engine's canvas
hidden, never hands the stage back, and clears the line at the next frame
the engine draws. `EngineTransport` takes commands for every decided route.

**Which frames a report makes stale (round 6, corrected in round 7).** The
revision cannot decide it: a composition switch reaches the engine as a
document batch (revision N) before the viewer state, so the engine reports
the old composition at N and then draws the new one at N. The frame `seq`
does, because it only increases within a session (a view lives as long as
its session) and the engine assigns it when it draws. Arrival order does
not: a session's events do not arrive in the engine's order. Paused frames
leave from the completion thread once the GPU finishes them
(`present/complete.rs`), while the render thread goes on to the next batch,
so round 6's rule (any frame after the report is a later decision) let a
paused frame of revision R, still on the GPU, arrive after a report caused
by the batch at R+1, with a higher seq than any frame received so far. That
frame brought the route back to `engine` and hid the reason. Two rules now
hold. The render thread drains the completion thread before it sends any
`unsupported` report (`report_legacy`, which every route to `legacy`
reaches: a static reason, a draw that fails with
`RenderError::Unsupported`, and a shader fallback runner failure), so every
frame drawn before the decision is sent before it. And the report carries
`ErrorDetail::Unsupported { lastFrame }`, the newest seq the engine had
given the view when it decided. The session drops frames with `seq` at or
below `lastFrame` (falling back to the newest frame it received when the
detail is absent), and passes it on in its `routed` message. The
presentation drops presented frames with `seq` at or below the larger of
`lastFrame` and the newest its channel had delivered (frames still in the
presenter's queue). A frame above it is a later decision at any revision. A
viewerless session takes `engine` from any route on a clock tagged with a
revision at or past the one it applied.

Headless playback carries playback pressure for idle snapshots but has no draw
schedule against which foreground async captures should wait. A Blender
capture's `blenderWaitMs` is one total wall-clock deadline across all replans;
finishing an attempt resets only the ordinary render wait. Absence of its
worker always yields a capture error. Native async Blender captures publish
scene3d busy/error state around preparation, as today's capture does. Font
bounds may be unavailable until font loading finishes; undefined font offsets
are retried rather than cached for the revision/time.

The native presenter retains one latest Play/play-around intent while its
composition or media is pending. It replays that command when ready; Pause
clears it. A pending-media transition during a run preserves its rate and
resumes on readiness. This closes the race between an imported audio layer,
a queued Play, and the renderer's still-current earlier readiness sample.
