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
| `Request::Snapshot(HostSnapshot)` | main → interactive engine, on main's host port only | One frame (or one layer's raster) of the document of `session`, which the window port `renderer` owns. The envelope's revision is the minimum to render (read-your-writes). Answer: `Response::Snapshot` |
| `Response::Snapshot(SnapshotOutcome)` | engine → main | `rendered` (pixels and tags) or `needsRenderer` (what only the renderer can draw: fallback programs, compatibility rasters) |
| `Event::Compat(CompatMessage)` | renderer → interactive engine (session-bound) | `pixels`: one row band of a compatibility raster; `forget`: drop a layer's rasters |
| `Request::CompatFrame(CompatFrameRequest)` | job engine → host | A layer's compatibility pixels for an export frame (the host asks the editor windows). Answer: `Response::CompatFrame` |
| `Event::RenderStats(RenderStats)` | interactive engine → renderer (session-bound) | What presenting the session costs, at most 2 Hz while presenting and once after idle |

### Snapshot request

`SnapshotRequest { composition, time (ticks), outputSize, target, pixels,
class, viewSeq, waitMs, uploaded }`:

- `target`: `frame { motionBlurSamples, background, draft3d }` or
  `layer { layer, scale, crop }`.
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
- `uploaded`: shader fallback keys the caller drew and posted on the
  session's port after a `needsRenderer` answer (see below).

### Snapshot result

`SnapshotResult { revision, viewSeq, width, height, pixels, payload, origin,
quality, waitedUs, renderUs }`. `revision` is the exact revision rendered;
`viewSeq` echoes the request. `payload` is `inline { bytes }` up to 1 MiB,
otherwise `sharedMemory { name, length }`: a POSIX segment the engine created
exclusively (mode 0600, `/pm.` and 24 hex digits, 96 random bits). Main maps
it once (pm-bridge `takePixels`), which unlinks it; the engine unlinks
segments nobody took after a minute and at shutdown. Electron's IPC carries
the buffer to the renderer in shared memory again (Mojo big buffers).

### Invariants

1. **Only main asks, only for the window's own session.** The engine refuses
   `snapshot` from any port but the host's (`capabilityRejected`), and
   refuses a session the named renderer port does not own
   (`unknownSession`). Main names the port from the IPC sender and refuses a
   session of an earlier engine launch.
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
   snapshots in flight per window, 1 GiB of results in flight in all.
6. **Failures are named, never another engine's pixels.** `unsupported`
   (with the engine's reasons), `timeout`, `unknownSession`,
   `budgetExceeded`, `deviceLost`, `invalidMessage`, `internal`; main adds
   `unavailable` (engine off or not running) and `refused` (main's caps).
   `render.snapshot` returns `''` only then.

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

### Stats

`RenderStats { frames, timedFrames, lateSkipped, cpuUs, gpuUs, layers,
frameUsAvg, programs, rasters, layerFailures, snapshots, snapshotUsMax,
revision }` per session. `frameUsAvg` is today's `E.ms` average (0.85/0.15)
of CPU plus GPU time.

## The TypeScript interfaces

### Preload (`window.powermove.nativeEngine`, `shared/engine-ipc.ts`)

```ts
snapshotSync?(request: EngineSnapshotRequest): EngineSnapshotReply; // never throws
snapshot?(request: EngineSnapshotRequest): Promise<EngineSnapshotReply>; // never rejects
onCompatAsk?(listener: (ask: EngineCompatAsk) => void): () => void;
answerCompat?(answer: EngineCompatAnswer): void;

type EngineSnapshotRequest = { generation: number; session: number; revision: number | null; snapshot: SnapshotRequest };
type EngineSnapshotReply =
  | { status: 'rendered'; revision; viewSeq; width; height; pixels; data: Uint8Array; origin: [number, number]; quality: FrameQuality; waitedUs; renderUs; mainMs }
  | { status: 'needsRenderer'; revision; fallback: ShaderFallbackRequest[]; compat: CompatNeed[] }
  | { status: 'failed'; code: string; message: string };
```

### Renderer (`PM.EngineRender`, `engine-connection/render-transport.ts`)

`EngineRenderTransport`, present only with the new engine on:

```ts
readonly ready: boolean;
snapshotSync(options: EngineSnapshotOptions): EngineSnapshot;          // throws EngineSnapshotError
snapshot(options: EngineSnapshotOptions, cls?: 'async' | 'idle'): Promise<EngineSnapshot>;
renderFrameToSync(time, width, height, options?): HTMLCanvasElement | null; // today's renderOpaqueFrame
renderFrameTo(time, width, height, options?): Promise<HTMLCanvasElement>;
snapshotDataUrlSync(time, maxWidth, size: { w; h }): string;          // JPEG 0.74, '' only on failure
snapshotDataUrl(time, maxWidth, size): Promise<string>;
readonly thumbnails: EngineThumbnails;                                  // idle class, stale-rejecting
uploadCompat(upload): boolean;
forgetCompat(layer, composition?): boolean;
provideCompat(source: CompatSource, producer: CompatProducer): () => void;
readonly stats: TransportStats;

type EngineSnapshotOptions = { time; width; height; composition?; pixels?; motionBlurSamples?; background?; draft3d?; waitMs?;
  layer?: { id: string; scale: number; crop?: { x; y; width; height } | null } };
type EngineSnapshot = { width; height; data: Uint8ClampedArray; pixels; revision; viewSeq; origin; quality; waitedUs; renderUs; mainMs };
class EngineSnapshotError extends Error { code: string }

EngineThumbnails.request({ key, ...EngineSnapshotOptions, minRevision? }, revision?): Promise<EngineSnapshot | null>; // null: stale or superseded
EngineThumbnails.cancel(key): void;
```

### Kernel render facade (`kernel/install.ts`)

With the engine on: `render.snapshot` and `render.renderFrameTo` come from
the engine through `snapshotSync`; `project.snapshot` through `snapshot`.
apiVersion 4's async siblings are on the render object:
`snapshotAsync(time, maxWidth = 480): Promise<string>`,
`renderFrameToAsync(time, w, h, options?): Promise<HTMLCanvasElement>`,
`rasterAsync(layer, scale = 1, time?, crop?): Promise<EngineSnapshot>`.
Their typings in `kernel/api.ts` belong to the extcompat lane.
`render.raster` itself stays on today's rasterizer until pm-render draws
layer rasters (see Open).

`render.gl.init(canvas)` and `render.gl.resize(…)` bind engine presentation
to the caller's canvas (`PM.EngineCanvas`,
`engine-connection/canvas-binding.ts`).

### Frame events and stats (`engine-connection/presentation-events.ts`)

Per new engine frame on screen: `preview:presented` (forwarded as
`frame:rendered { time, viewport, version, quality }` with `viewport` the
exact `render.gl.previewViewport` object and `version` the `animVersion` of
the frame's revision), then `overlay`. `PM.EngineStats.perf()` gives today's
`perf` fields (`ms, fps, draws, passes, progs, raster`).

### Viewer service (`EngineViewerService`, engine-present)

Added members: `exclusive?: boolean` (the kernel's connection sets it: the
viewer never hands the stage, transport, or audio back to the current
engine) and `presented?(frame, quality)`. `ViewerPresentation` gains
`exclusive`, `status`, `role` (`viewer` | `binding`), marks its stage
`data-engine-stage`, and dispatches `powermove:engine-stage` when a viewer
presents by itself.

## Versioning

Draft. The Rust types are additive to contracts 2 and 14, but
`PROTOCOL_BUILD` changes with any source change, so lanes regenerate the
TypeScript and rebuild their addons when this lands.

## Open

- **Depth and ID surface delivery** aligned to a presented frame (contract
  2's `FrameDescriptor.depthId`): not delivered yet; see bridge.md gaps.
- **Layer rasters**: the engine answers `layer` targets `unsupported` until
  pm-render draws single-layer rasters; the compatibility lane builds
  `render.raster` on it.
- **Compatibility rasters in frames**: the store holds them; pm-render does
  not draw a layer from them yet, so `needsRenderer.compat` is always empty.
- **Snapshots before the window's first engine session** (during boot) fail
  with `noSession`; afterwards the standby session keeps them working while
  no viewer is open.
