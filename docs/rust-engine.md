# Powermove native engine: plan and design

Status: proposal, not started. This document covers the whole rewrite of the
Powermove engine (project evaluation, decode, compositing, color, text, audio,
playback, caching, proxies, and export) from TypeScript and WebGL2 into Rust,
while the Svelte UI, the kernel, extensions, and the agent harness stay on the
web stack.

It is written to be executed by several parallel implementation lanes. Every
phase ends at a gate with measurable exit criteria. No phase is complete until
its gate passes.

Contents

1. Why
2. Goals, non-goals, and the quality bar
3. What exists today
4. Architecture overview
5. Time
6. Document model, schema v2, and ownership
7. Evaluation (`pm-eval`)
8. GPU and the render graph (`pm-render`)
9. Color pipeline
10. Layer types, one by one
11. Effects, transitions, and the shader contract
12. Text
13. Media I/O (`pm-media`)
14. Proxies, optimized media, and caches
15. Playback
16. Audio
17. Export and delivery
18. Viewer integration and the frame bridge
19. Process model, IPC, and packaging
20. Kernel API, extensions, and the agent harness
21. Project files and media management
22. Web player, `powermove serve`, and the headless engine
23. Cross-platform
24. Licensing and compliance
25. Testing, parity, and performance gates
26. Phases
27. Lanes and ownership
28. Risks
29. Decisions needed from Jude
30. Appendix A: defects found during the survey
31. Appendix B: feature parity inventory
32. Appendix C: glossary

---

## 1. Why

Powermove is used by more professional editors and motion designers every
release. Their footage, codecs, and color expectations exceed what the
browser engine can deliver. The survey behind this document found the limits
are structural, not tuning problems:

- **Codecs.** Chromium cannot decode ProRes, DNxHR, MXF, camera RAW, or most
  intermediate codecs. Powermove transcodes them on import to an 8-bit 4:2:0
  VP9 file, and that transcode replaces the original in the project
  (`legacy/gl/raster.ts:874-889`). A ProRes 4444 XQ source is edited and
  exported from a lossy 8-bit copy.
- **Precision.** Every source texture is RGBA8 with premultiply-on-upload
  (`compositor.ts:541-545`). Intermediates are RGBA16F, but readback for export
  is 8-bit (`compositor.ts:1825`). ProRes 4444 is written as 10-bit with
  `-alpha_bits 16` from 8-bit data.
- **Color.** There is no color management. Blending, blur, opacity, and motion
  blur all run on gamma-encoded sRGB values. Import ignores primaries,
  transfer, matrix, range, and HDR metadata. Export tags the transfer as sRGB
  (`iec61966-2-1`) for both H.264 and ProRes, which most NLEs read differently
  from BT.709.
- **Time.** Everything is float seconds. 29.97 fps is a float, comp frame rates
  are rounded to integers on two of three code paths, timecode is wrong at
  fractional rates, and nested compositions are never re-quantized.
- **Audio.** WebAudio with whole-file decode and a 128 MiB PCM cap (about 5.8
  minutes of stereo). Speed changes and time remapping silence or ignore audio.
- **Export.** Frames are rendered in the renderer, read back, and copied over
  IPC in 4 MiB chunks to an ffmpeg child process. A 4K frame is 33 MB; the
  measured end-to-end rate is about 11 fps at 4K for a trivial project.
- **Headless.** There is no way to render without a browser tab. Agents, the
  CLI, and any future render service all need one.

A native engine fixes all of these at the root and gives Powermove a single
renderer that can run in the app, in a CLI, and later on Windows.

## 2. Goals, non-goals, and the quality bar

### Goals

1. **Image fidelity.** Float precision from decode to encode. Nothing drops to
   8-bit unless the delivery format requires it.
2. **Color management.** A scene-referred linear working space, per-source
   input transforms, display transforms at the edge, OCIO and ACES, LUTs, HDR
   (PQ and HLG) delivery.
3. **Native codecs.** Read and write the formats professionals use, using the
   original media at all times, with hardware acceleration where the platform
   offers it.
4. **Exact time.** Rational frame rates, integer tick time, frame-accurate
   decode, drop-frame timecode, sample-accurate audio.
5. **One renderer.** Preview, export, thumbnails, agent snapshots, and headless
   renders produce the same pixels for the same inputs.
6. **Proxies done right.** Full control like After Effects and Final Cut Pro,
   with an automatic default that needs no setup (section 14).
7. **Speed.** Real-time playback of multi-layer 4K timelines on Apple Silicon,
   multi-frame parallel export, no main-thread rendering in the UI process.
8. **Keep what works.** The Svelte UI, the kernel, the extension system and
   Store, the agent harness, Undo, and the file format's durability guarantees
   all survive.
9. **Portability.** macOS first; the engine has no macOS-only assumptions
   outside clearly separated platform backends, so a Windows build is a port,
   not a rewrite.

### Non-goals

- Rewriting the UI, timeline, inspector, or kernel in Rust.
- Changing the product's interaction model.
- Rewriting the agent harness. It gains capabilities; it is not redesigned.
- OpenFX hosting, camera RAW SDKs, real 3D cameras and lights. These are
  planned after cutover (section 26, "After cutover") and the architecture
  leaves room for them, but they are not part of reaching parity.

### The quality bar

"Professional grade" is made concrete by these acceptance tests. Each has an
automated check (section 25).

| Area | Bar |
|---|---|
| Bit depth | A 10-bit ProRes 4444 source exported to ProRes 4444 without effects decodes back within 1 code value of the source in every channel. A 16-bit synthetic gradient exported to 10-bit has at least 900 distinct levels across 1024 columns. |
| Alpha | ProRes 4444 and HEVC-with-alpha sources round-trip with alpha preserved and no premultiplication error larger than 1 code value at 10-bit. Straight and premultiplied output are both available. |
| Color | Every export carries correct primaries, transfer, matrix, and range tags, verified by probing the output. sRGB, BT.709, Display P3, BT.2020 PQ, and BT.2020 HLG outputs pass reference-patch checks within ΔE2000 0.5. |
| Linear light | In linear mode, a 50% opacity white-over-black layer produces 0.5 linear (about 0.735 sRGB-encoded), and a Gaussian blur of a white line on black conserves energy within 0.5%. |
| Time | At 23.976, 29.97 (drop-frame and non-drop), 59.94, and 25 fps, frame N of a decoded export equals frame N of the composition for N up to 1,000,000; timecode round-trips exactly. |
| Seeking | Random access to any frame of every supported codec returns the same pixels as sequential decode of that frame. |
| Audio | Exported audio is sample-accurate to the video: a click at frame N lands within one sample of N × sampleRate / fps. No length limit. |
| Parity | Every project in the parity corpus renders through the new engine in compatibility mode within the tolerances in section 25. |
| Antialiasing | Rotated footage, shapes, and text edges are antialiased; no shimmering in downscaled footage or text during animated scale (mipmapped or high-quality filtered sampling). |
| Determinism | Two renders of the same frame on the same machine are byte-identical. |

## 3. What exists today

This summary is the starting point. Full inventories are in appendix B.

### Code that the engine replaces

| Area | Main files | Approximate non-test LOC |
|---|---|---|
| Model, animation, expressions, retiming, 3D math, text animators | `legacy/core/{model,anim,temporal-bridge,easing,expression,retiming,parenting,layer-groups,space-3d,text-animation,text-layout,split-text,vector-paths,content-properties,size-anchor,font-anchor,matte}.ts`, `core/anim/temporal-ease.ts` | 3,500 |
| Compositor, rasterizer, shaders, caches | `legacy/gl/*` | 4,200 |
| Media import, decode, proxies, caches | `legacy/core/{media,media-conversion,video-*,image-sequence,poster,preview-*,memory}.ts`, import half of `raster.ts`, `main/media-proxy.ts` | 3,800 |
| Playback loop | `legacy/core/engine.ts` | 500 |
| Audio and mixer engine | `legacy/core/audio.ts`, non-UI mixer | 1,600 |
| Export | `legacy/core/exporter.ts`, `frame-capture.ts`, `render-queue.ts`, `main/render-encoder.ts` | 1,000 |
| Built-in effects, transitions, 3D layers (GLSL) | `extensions/{effects-basic,transitions-basic,layers-3d}` | 570 |

About 15,000 lines of dense TypeScript. The Rust replacement will be larger,
because it takes on decode, color, text shaping, and audio work that the
browser does today.

### What stays

- `apps/desktop/src/renderer` UI, panels, Settings, all built-in UI extensions
  (viewer, timeline, inspector, toolbar, mods, keymap, theme).
- The kernel (`renderer/src/kernel`) and its public API, extended as described
  in section 20.
- The edit pipeline (`legacy/core/editing.ts`), history and Undo, the command
  vocabulary, and validation, at least until after cutover (section 6).
- The agent harness, MCP bridge, and media tools in `main/agent-tools`.
- The `.pmv` container and its durability (`main/project-save.ts`,
  `project-files.ts`, `durable-file.ts`).

### Constraints inherited from earlier decisions

These come from `apps/desktop/docs/*` and must hold in the new engine:

- **One compositor for preview, capture, export, and nested comps**
  (`rendering-performance.md`). The new engine keeps that single path.
- **Pixel-exact acceptance for optimizations.** Performance work never changes
  output. The new engine adopts the same rule internally, with byte-identical
  renders before and after any optimization.
- **No `'unsafe-eval'`; bounded expression interpreter.** The Rust interpreter
  keeps the same limits (2,000 chars, 160 nodes, depth 24, 12 args).
- **One user intent equals one Undo step.** Unchanged; Undo stays in the edit
  pipeline.
- **Effect contract limits.** 32 params, 1 to 8 passes, 64 KiB fragments;
  extension layers 48 params, 96 KB. GLSL ES 3.00 sources stored in projects
  and Store extensions must keep working (section 11).
- **Per-layer failure isolation.** A layer that fails to render is reported and
  skipped; the frame still renders.
- **GPU context loss recovery** without replacing the project, and exports that
  abort on loss without writing bad bytes.
- **Predictability over raw speed** (`rendering-performance.md`). The engine
  prefers bounded, measured behavior to peak throughput.

## 4. Architecture overview

```
┌──────────────────────────── Electron app ─────────────────────────────┐
│                                                                        │
│  Renderer (per window, sandboxed)                                      │
│  ├─ Svelte UI, kernel, extensions, edit pipeline, history (TS)         │
│  ├─ pm-wasm: document mirror + evaluator + text layout (sync queries)  │
│  └─ Viewer: WebGPU canvas showing engine frames + DOM/2D overlays      │
│          ▲ MessagePort (control, patches, events)   ▲ VideoFrame       │
│          │                                          │                  │
│  Main process                                       │                  │
│  ├─ window, menus, files, save, agent bridge (TS)   │                  │
│  └─ pm-bridge (napi): frame-handle import ──────────┘                  │
│          ▲ shared GPU surface handles                                  │
│          │                                                             │
│  Engine process (utilityProcess hosting pm-engine via napi-rs)         │
│  ├─ sessions (one per open project tab)                                │
│  ├─ pm-eval, pm-render (wgpu), pm-color, pm-text, pm-vector            │
│  ├─ pm-media (VideoToolbox / FFmpeg / image readers), pm-cache         │
│  ├─ pm-audio (device clock, mixer, stretch)                            │
│  └─ pm-export, pm-proxy job queues                                     │
└────────────────────────────────────────────────────────────────────────┘

Same crates, other hosts:
  pm-cli      powermove-render: headless render and export, no Electron
  pm-wasm     renderer-side evaluator (no GPU), and later the web player
```

Key properties:

1. **The engine never runs on a UI thread.** It lives in its own process with
   its own threads. A slow effect cannot stall typing in the inspector.
2. **The renderer keeps synchronous answers.** Hit-testing, bounds, property
   evaluation, and text layout are synchronous in the UI today (about 300 call
   sites in built-ins). They are served by `pm-wasm`, the same Rust evaluator
   compiled to WebAssembly, holding a mirror of the document. UI geometry and
   rendered pixels therefore come from identical code.
3. **Frames stay on the GPU.** The engine renders into shared GPU surfaces
   (IOSurface on macOS, NT-handle D3D textures on Windows). Main imports them
   through Electron's `sharedTexture` API and hands the renderer a `VideoFrame`
   that the viewer draws into a WebGPU canvas. No pixel copies on the preview
   path (section 18).
4. **One renderer for everything.** Preview, export, thumbnails, agent
   snapshots, and the CLI all call `pm-render` with the same inputs.
5. **Transport-agnostic engine API.** The engine is a library with a typed
   message protocol. The same protocol works over MessagePorts in the app,
   in-process in the CLI, and over a socket for `powermove serve` later.

### Crate layout

A Cargo workspace at the repository root, `engine/`.

| Crate | Responsibility | Targets |
|---|---|---|
| `pm-time` | Ticks, rational rates, frame math, timecode (drop-frame) | native, wasm |
| `pm-model` | Schema v2 types, serde, migrations from v1, patch application, validation | native, wasm |
| `pm-eval` | Keyframes, temporal ease, expressions, parenting, groups, 2.5D math, retiming, text animator state, masks/paths geometry, `pick`/`bounds` | native, wasm |
| `pm-text` | Font discovery, shaping, layout, glyph outlines, caret geometry | native, wasm |
| `pm-vector` | Paths, trim, repeat, strokes, booleans, squircle corners | native, wasm |
| `pm-color` | Transfer functions, working spaces, OCIO bridge, LUT parsing and baking, tone mapping | native |
| `pm-gpu` | wgpu device, resource pools, shader compilation, GLSL compatibility translation | native (wasm later) |
| `pm-render` | Render graph, layer renderers, effects, transitions, compositing, motion blur, scopes | native (wasm later) |
| `pm-media` | Probe, demux, decode backends, frame index, image readers, color metadata | native |
| `pm-cache` | Decoded-frame cache, render cache, disk cache, budgets | native |
| `pm-proxy` | Proxy and optimized-media policy, jobs, store | native |
| `pm-audio` | Device output and clock, mixer, automation, time-stretch, waveforms, meters | native |
| `pm-export` | Encoders, muxers, render queue, output verification | native |
| `pm-engine` | Sessions, scheduler, playback, protocol, budgets, crash recovery | native |
| `pm-node` | napi-rs host for the engine process | native |
| `pm-bridge` | napi-rs addon in main: shared-surface import into `sharedTexture` | native |
| `pm-wasm` | wasm-bindgen exports for the renderer | wasm |
| `pm-cli` | `powermove-render` binary | native |
| `pm-protocol` | Message types, versioning, TypeScript type generation | native, wasm |

TypeScript types for every protocol message are generated from `pm-protocol`
(for example with `specta` or `ts-rs`) and checked in CI, so the two sides
cannot drift.

## 5. Time

### Representation

- **Ticks.** All times are `i64` ticks of 1/705,600,000 second (the "flick").
  Every common frame rate (23.976, 24, 25, 29.97, 30, 48, 50, 59.94, 60, 90,
  100, 119.88, 120, 240) and every common audio sample rate (8 kHz to 192 kHz,
  including 44.1 kHz) divides it exactly, so frame and sample boundaries are
  integers. The range covers about 400 years.
- **Rates** are `Rational { num: u32, den: u32 }`: 24000/1001, 30000/1001,
  60000/1001, 25/1, and so on.
- **Frame math** is integer: `frame_start(n) = n × den × TICKS / num` with exact
  division guaranteed for supported rates and rounded-down division otherwise.
- **Timecode** lives only at the UI and IO edges (`pm-time::Timecode`), with
  SMPTE drop-frame for 29.97 and 59.94.
- **Expressions** keep `time`, `t`, and `fps` as floats in seconds, derived from
  ticks, so existing expressions keep working.

### Rules

1. Composition frame rate is a `Rational`. All three code paths that round or
   clamp fps today are replaced by one validator.
2. Keyframes created by the UI land on frame boundaries of the composition that
   owns the layer, in composition time, not layer-local time. Layer-local key
   storage is kept (keys move with the layer) but expressed in ticks.
3. Nested compositions sample their children at the parent time by default
   (continuous). A precomp option, "Keep frame rate when nested", quantizes the
   child to its own rate, as in After Effects.
4. Variable speed integrates in ticks using the owning composition's rate, not
   the root project's rate.
5. Export frame rate can differ from the composition rate. The exporter steps
   in export frames and evaluates at the exact tick of each export frame.
6. Motion blur sub-samples are tick offsets inside the shutter interval.
7. Media sample times map to ticks through each stream's time base without
   float conversion.

### Migration from float seconds

- Times are converted to the nearest tick; the maximum error is 0.7 ns.
- Known NTSC floats (29.97, 29.970029…, 23.976, 59.94 and their float
  representations) map to the exact rational. Other non-integer rates map to
  the smallest rational that reproduces the float within 1e-9.
- Keyframes that sat off the frame grid stay where they were. They are not
  snapped, because snapping changes animation. A "Snap keys to frames" command
  is offered.

## 6. Document model, schema v2, and ownership

### Ownership

The renderer remains the **authority** for the document and Undo. This keeps
the edit pipeline, history, validation, agent rollback, and the frozen kernel
contracts intact, and limits the rewrite to the engine.

Two mirrors follow the authority:

- **`pm-wasm` in each renderer** for synchronous queries.
- **The engine session** for rendering, playback, and export.

Both are fed the same ordered stream of document patches, tagged with
`revision`. The patch format is the existing leaf patch
(`src/shared/patch.ts`: `{path, exists, value}`), which history already
produces.

### Making every write visible

Today the document is a mutable object graph written by about 47 raw
`PM.hist.do` closures, the edit engine, live gestures, `anim.setKey`, asset
metadata updates, and extensions through `api.host.pm`. A mirror is only
correct if every write produces a patch. The plan:

1. **Commit-time diffing.** History already diffs a snapshot against the live
   graph at commit. Live gestures (`Edit.begin/dispatch`) also emit
   intermediate patches per dispatch so the viewer can follow a drag.
2. **Scoped writers get explicit patch emission.** Asset metadata
   (`raster.ts:836-841`), `anim.setKey`, extension-layer data refresh
   (`install.ts:789-798`), and anything else that writes outside history call
   `doc.patch(paths)`.
3. **A debug-build mutation auditor** hashes the document at frame boundaries
   and compares it with the mirror's hash. Any mismatch is logged with the
   stack of the last writer and fails tests. CI runs the full e2e suite with
   the auditor on.
4. **Fallback resync.** If a revision gap is detected in production, the mirror
   requests a full snapshot. This is a correctness net, not a design path.

### Schema v2

`pm-model` defines schema v2. The JSON shape stays close to v1 so the UI,
validators, and extensions change as little as possible. Changes:

| v1 | v2 |
|---|---|
| Float seconds everywhere | Ticks (`i64`, serialized as integers) for all times; `Rational` fps |
| Implicit migration by `hydrate` normalizers on every load | Explicit `schemaVersion` with ordered, tested migrations in `pm-model`, run in WASM in the renderer and natively in the CLI |
| The open composition rotated into the project root | All compositions stored uniformly in `comps`; the "open comp" is UI state. The root-rotation and canonical-path translation code is removed |
| Keyframe objects carrying non-enumerable accessors (`eo`, `ei`, `hold`, `bezierMode`) | Plain data only. The legacy easing accessors become functions in the UI layer |
| Assets store a fingerprint and `storageKey`; converted video replaces the original | Assets store the original reference (section 21), source color metadata, a media index id, and a list of representations (original, optimized, proxies) |
| No color settings | Project color settings and per-asset interpretation (section 9) |
| `PM.version = '1.0.0'` constant | `schemaVersion` integer plus `engineFeatures` set for forward compatibility |

Projects opened from v1 are migrated in memory and saved as v2 only when the
user saves. A v2 project opened by an old build shows "This project needs a
newer version of Powermove" instead of misreading it.

### Later: moving the authority

After cutover, the edit pipeline and history can move to Rust (`pm-doc`) so
agents and the CLI can edit projects without a renderer. This is optional and
deliberately not on the critical path.

## 7. Evaluation (`pm-eval`)

`pm-eval` is a pure function of `(document, composition, time) → evaluated
state`. It replaces `anim.ts`, `temporal-bridge.ts`, `temporal-ease.ts`,
`easing.ts`, `expression.ts`, `retiming.ts`, `parenting.ts`, the group and 3D
math in `layer-groups.ts` and `space-3d.ts`, text animator state, path and
mask geometry, and the evaluated parts of `content-properties.ts`.

Design:

- **Pure and explicit.** No global current composition, no global time. Scope
  (composition stack, project params) is an argument. This removes the
  `PM.scope` stack and the version-keyed memo maps.
- **Incremental.** Evaluation caches are keyed by `(node id, revision of that
  node's inputs, tick)`. A patch invalidates only the affected nodes and their
  dependents (parents, children, matte users, expression readers).
- **Expression dependencies.** Expressions can read raw project fields today.
  The Rust interpreter records which fields each expression reads during
  evaluation, so dependency tracking is exact.
- **Temporal model.** The After Effects-style speed and influence model,
  auto-bezier, hold, and the stored spring parameters are ported exactly.
  Spatial bezier motion paths are added as a new capability after parity.
- **Parity rules.** The interpreter reproduces JavaScript semantics where
  projects depend on them: loose `==`, string concatenation coercion,
  `Math.round` rounding half toward +∞, `%` on negatives, the `sin`-based
  `random` and `wiggle`, the float32 spring table, and the hex color byte
  interpolation. Each rule has a test that compares against V8.
- **Geometry queries.** `pick(point, t)`, `bounds(layer, t)`,
  `worldMatrix(layer, t)`, and group bounds are computed here from the same
  geometry the renderer uses, including text bounds from `pm-text`.

## 8. GPU and the render graph (`pm-render`)

### Device

- wgpu with the Metal backend on macOS, DX12 on Windows, Vulkan available for
  Linux CLI renders. Versions pinned; upgrades are deliberate lanes.
- Platform escape hatches through wgpu-hal for shared-surface export and
  hardware decoder output import.
- One device per engine process, shared across sessions; budgets per session.

### Working format

- Intermediates are `rgba16float` by default and `rgba32float` when the project
  is set to 32-bit float.
- Pixels are premultiplied linear light in the project working space
  (section 9).
- Source frames enter as their native layout: 8, 10, 12, or 16-bit YUV planes
  (NV12, P010, P016, 4:4:4) or RGB(A) of any depth. Conversion to working
  space happens in a shader with the correct matrix, range, transfer, and chroma
  siting, then into a float texture.

### Render graph

Each frame is a directed graph compiled from the evaluated state:

- **Nodes:** source (decoded frame, rasterized vector or text, solid, shader
  output), transform/resample, mask, effect pass, matte, blend, transition,
  motion-blur accumulate, precomp, adjustment, color transform, output.
- **Content hashing:** every node has a hash of its inputs and parameters.
  Identical subtrees are rendered once per frame. Results are cached across
  frames in the render cache (section 14.6). A static layer over 500 frames is
  rendered once.
- **Region of interest:** each node knows its domain of definition and the
  region downstream needs (including effect padding, the existing
  `viewportPadding` contract). When the viewer is zoomed in, only visible
  regions are rendered. When an effect expands bounds (glow, drop shadow), the
  layer buffer grows to fit, up to a limit, instead of clipping.
- **Resource pooling:** transient textures are allocated by an aliasing
  allocator per frame.
- **Isolation:** a node that fails marks its layer failed and is replaced by a
  transparent output; the rest of the frame renders.

### Sampling and antialiasing

- Footage and raster sources use mipmapped trilinear sampling for
  downscaling, and a selectable high-quality resampler (bicubic, Lanczos) for
  export and paused frames.
- Quad edges use analytic coverage antialiasing, not the current hard UV cut.
- Vector shapes and text are rendered as vectors at output resolution, never
  as scaled bitmaps, so continuous rasterization scale buckets go away.

### Motion blur

- Temporal supersampling, as today, with the sample count set by a single
  project setting used everywhere (today it is 6, 12, 16, or 20 depending on
  the code path).
- Adaptive samples: layers whose motion within the shutter is below a pixel are
  rendered once.
- Accumulation happens in linear light at float precision.

### 2.5D

Parity first: per-layer 3D switch, position, anchor and scale Z, rotation X
and Y, orientation, perspective lens per 3D group, homography planes,
coplanar ordering, painter sort. Then, after parity, a depth buffer with
correct plane intersection, followed by camera and light layers (section 26).

### Scopes

Waveform, RGB parade, vectorscope, and histogram are computed by compute
shaders from the frame on screen, before or after the display transform as
the user chooses. They run in the engine and arrive in the UI as small
textures through the same frame bridge.

## 9. Color pipeline

### Model

```
source pixels
  → decode (YUV/RGB, native bit depth)
  → input transform  (matrix, range, transfer, primaries → working space)
  → working space    (premultiplied, linear, float)
      compositing, effects, motion blur
  → output transform (working space → delivery encoding)    for export
  → display transform (working space → viewer display)      for preview
```

### Project color settings

A Settings › Project section, using the existing Settings card and row
components.

| Setting | Options | Default for new projects |
|---|---|---|
| Color mode | Standard, Linear, ACES | Linear |
| Working space | Linear Rec.709/sRGB primaries, Linear Display P3, Linear Rec.2020, ACEScg | Linear Rec.709 |
| Precision | 16-bit float, 32-bit float | 16-bit float |
| Display | Automatic (follows the screen profile), sRGB, Display P3, Rec.709 (BT.1886), HDR (EDR) | Automatic |
| HDR output | Off, PQ, HLG | Off |

- **Standard** reproduces today's look: blending happens on display-encoded
  values, so existing projects look the same. Projects migrated from v1 open in
  Standard. The setting can be changed at any time; it changes the look, so the
  change is one Undo step.
- **Linear** is the professional default: physically correct blending, blur,
  opacity, and motion blur.
- **ACES** uses an OCIO ACES 2.0 configuration with ACEScg working space and
  ACES output transforms.

### Per-source interpretation

Every asset has an "Interpret" sheet (After Effects' Interpret Footage), filled
automatically from container and stream metadata:

- Color: primaries, transfer (including camera log curves such as S-Log3,
  LogC3, LogC4, V-Log, C-Log2/3, Apple Log), matrix, range.
- Alpha: none, straight, premultiplied; invert.
- Frame rate override and field order.
- Pixel aspect ratio.
- Input LUT (optional).

Automatic detection is shown as "Automatic (Rec.709)" so users see what was
inferred. Overrides are per asset and undoable.

### OCIO, LUTs, and color effects

- OpenColorIO 2.5 C++ through a thin, owned FFI bridge in `pm-color`. GPU
  shaders from OCIO are translated with the same shader pipeline as user GLSL
  (section 11) and cached.
- LUT formats: `.cube` (1D and 3D), `.3dl`, `.csp`, CLF/CTF.
- New color effects, all operating in a selectable space (working, log, or
  display): Apply LUT, Exposure, Curves, Lift/Gamma/Gain wheels, Hue vs Hue/Sat
  curves, White balance, Color space transform, Tone map.
- Color pickers keep working in sRGB or Display P3 display values; the engine
  converts picked colors to the working space. Gradient interpolation offers
  Linear, Perceptual (OKLab), and Display options.

### HDR

- Preview: the viewer's WebGPU canvas is configured with `rgba16float` and
  extended tone mapping on HDR-capable displays; values above 1.0 reach the
  screen (to be confirmed in spike S1, section 26).
- An optional native reference viewer (`CAMetalLayer` with extended dynamic
  range) for color-critical work on a second display, after cutover.
- Export: PQ and HLG with mastering display and content light level metadata
  for HEVC 10-bit and ProRes.

## 10. Layer types, one by one

| Type | Today | Native engine |
|---|---|---|
| `solid` | Uniform-color quad | Same, in working space; corner radius via `pm-vector` |
| `shape` primitives and paths | Canvas2D raster at quantized scale | `pm-vector` geometry, GPU vector rendering (Vello or own tessellation, chosen in spike S4), analytic AA, exact trim/repeat, strokes with joins and caps, squircle corners |
| `text` | Canvas2D `fillText`, browser shaping | `pm-text` (section 12) rendered as vector glyphs |
| `captions` | Canvas2D boxes and stroked text | `pm-text` plus `pm-vector` boxes; word states (pop, fade, highlight) evaluated in `pm-eval` |
| `image` | `ImageBitmap` RGBA8 | `pm-media` image readers at native depth (PNG 16-bit, TIFF, EXR, HEIF/AVIF HDR, JPEG, WebP, SVG via vector path) |
| `image` (SVG) | `<img>` rasterized per size bucket | Parsed to vector (usvg) and rendered as vectors at any scale |
| `video` | `<video>` element or VP9 proxy | `pm-media` decode of the original or a chosen representation (section 14) |
| `shader` | User GLSL into an FBO, capped at 4,096 px | GLSL compatibility pipeline (section 11); no size cap beyond device limits |
| `extension` fragment | Kernel-assembled GLSL | Same compatibility pipeline |
| `extension` mesh | Fixed WebGL shading for OBJ | Same look in compatibility mode; physically based shading after parity |
| `precomp` | Recursive render into FBO, depth 6 | Render graph subgraph, cached by content hash; depth limit raised and cycle-checked in `pm-eval` |
| `adjustment` | Effects on the accumulator | Same semantics |
| `group` | Transform space or compositing boundary | Same semantics and boundary rules |
| `null`, `audio` | Not visual | Not visual |

Animated GIF, APNG, animated WebP and AVIF, and image sequences are decoded
directly by `pm-media`. They are no longer transcoded to VP9, so their
full color depth and exact frame timing are kept.

## 11. Effects, transitions, and the shader contract

### Built-in effects

The 16 built-in effects, 4 transitions, 5 shader presets, and 2 extension
layers are re-implemented natively in WGSL with these fixes:

- Gaussian Blur uses a correct kernel at every radius, with a downsample
  pyramid for large radii (today, taps are spaced `r/N` apart and skip texels).
- Glow and Drop Shadow use the same blur and work in linear light.
- Effects declare whether they expect linear or display-referred input
  (`colorDomain: 'linear' | 'display' | 'log'`). The engine converts around each
  effect accordingly.
- Effects declare their bounds expansion so buffers grow instead of clipping.

In Standard color mode, built-in effects reproduce today's output within the
parity tolerances.

### The GLSL contract stays

Projects, Store extensions, agent-written extensions, and web exports carry
GLSL ES 3.00 source with Powermove's preambles (`PM.GLSL_PRE`,
`TRANSITION_PRE`, `SHADER_HEADER`, the `u_<param>` uniforms, the `u_p<i>` and
`u_c<i>` aliases). That contract keeps working:

1. **Wrap.** The existing generators (`kernel/glsl.ts`, `extension-layers.ts`)
   produce the full GLSL ES 3.00 program, as today.
2. **Up-convert.** A source rewriter converts GLSL ES 3.00 to GLSL 450 with
   Vulkan semantics: explicit bindings for samplers and a uniform block for
   loose uniforms, `precision` qualifiers removed, `texture()` calls mapped to
   separate texture and sampler objects.
3. **Compile.** glslang compiles GLSL 450 to SPIR-V; naga translates SPIR-V to
   the backend language (MSL, HLSL). Results are cached by source hash on
   disk.
4. **Color domain.** Legacy GLSL effects are treated as display-referred: the
   engine encodes to the display transfer before the effect and decodes after,
   so they look the same in Linear mode as they did before.
5. **Errors.** Compile errors are mapped back to the user's line numbers, as
   `gl.compileError` does today.

Spike S3 (section 26) proves this on every GLSL source in the repository,
the Store, and the sample docs before the decision is final.

### Contract v2

A WGSL contract is added for new extensions (`apiVersion` 4), with explicit
`colorDomain`, bounds expansion, multiple inputs, and compute passes. GLSL
remains supported indefinitely.

## 12. Text

Text is the hardest layer to match and the one designers notice first.

- **Stack:** Parley for layout (bidi, line breaking, variable fonts, color
  emoji, fallback), HarfRust for shaping, Skrifa for outlines and metrics,
  rendered as vector paths by the vector renderer. Chosen in spike S4 against
  cosmic-text.
- **Fonts:** system fonts enumerated natively (CoreText on macOS, DirectWrite
  on Windows) and loaded from the same files on both the engine and `pm-wasm`.
  Project-embedded fonts are passed to both. Font matching is by family, style,
  and axis values; the CSS shorthand used today is parsed into these.
- **Layout parity:** today's layout is greedy word-then-character wrapping,
  baseline at `size × 0.82`, line height `size × leading`, kerning via measured
  widths. Compatibility mode reproduces these metrics so existing layouts keep
  their line breaks and positions. New text layers use font-metric baselines.
- **Animators:** stagger, range, and wave selectors on characters, words, and
  lines; per-glyph opacity, position, scale, rotation, skew, tracking, blur,
  and color. All are evaluated in `pm-eval` and rendered per glyph.
- **Editing:** the canvas text session in the viewer keeps its UX. Caret and
  selection geometry come from `pm-wasm`, so the caret lines up exactly with
  rendered glyphs.
- **Quality:** text is rendered at output resolution with analytic coverage,
  gamma-correct blending of edges, and no bitmap caps (today 8,192 px).
- **Bar:** a side-by-side corpus against CoreText rendering of the same fonts
  at 12 to 400 px must pass review by Jude before text cutover.

## 13. Media I/O (`pm-media`)

### Principles

1. **The original is the truth.** Import never replaces the original with a
   transcode. Everything else is a representation derived from it.
2. **Exact frames.** Every source is indexed so frame N can be decoded exactly
   and quickly.
3. **Native depth and color.** Frames reach the GPU at their native bit depth
   with their color metadata attached.

### Formats

| Category | Formats | Backend |
|---|---|---|
| Professional intermediates | ProRes 422 Proxy/LT/422/HQ, 4444, 4444 XQ | VideoToolbox (hardware on Apple Silicon) |
| Camera and delivery | H.264, HEVC 8/10-bit, HEVC with alpha, AV1 | VideoToolbox; FFmpeg software fallback |
| Avid and others | DNxHD, DNxHR, CineForm, MJPEG, DV | FFmpeg |
| Web | VP8, VP9 (with alpha via libvpx), AV1 | FFmpeg (libvpx, dav1d) |
| Containers | MOV, MP4, M4V, MXF (OP1a, OP-Atom), MKV, WebM, AVI, MTS/M2TS, TS, MPEG-PS, and the rest of today's list | FFmpeg demuxers |
| Images | PNG (8/16), JPEG, TIFF (8/16/32f), OpenEXR (half/float, multi-part later), DPX (10/12/16), HEIF/HEIC, AVIF (HDR), WebP, BMP, GIF, SVG | ImageIO on macOS, `image`/`exr` crates, usvg |
| Sequences | Any image format above, with gaps reported | `pm-media` sequence reader |
| Animated images | GIF, APNG, animated WebP, animated AVIF | Native decoders, original timing |
| Audio | WAV, AIFF, CAF, MP3, AAC/M4A, FLAC, Opus, Ogg Vorbis, and audio in all video containers | FFmpeg (symphonia where it is complete) |
| Later | Camera RAW (BRAW, R3D, ARRIRAW, Canon RAW, ProRes RAW, cDNG), PSD layers | Section 26 |

### Probe and index

On import, `pm-media` runs a probe in the background:

1. Container and streams: codec, profile, dimensions, pixel format and depth,
   frame rate (constant or variable), duration, timecode, reel name, rotation,
   pixel aspect, field order, audio layout.
2. Color: primaries, transfer, matrix, range, mastering metadata, ICC profile,
   camera log hints.
3. Frame index: presentation timestamps and keyframe positions, stored in the
   media index (section 14.6) so seeks never scan the file.
4. Decode cost: a short decode benchmark on this machine, used by the proxy
   policy (section 14.3).
5. Poster and thumbnails.

Variable frame rate sources are conformed to a constant rate for editing with a
quiet notice in the asset's details.

### Decode service

- Decode sessions per source and per playback instance, as today's
  per-instance decoders, but pooled and budgeted.
- Hardware decoders output GPU surfaces (CVPixelBuffer/IOSurface on macOS)
  imported into wgpu without CPU copies.
- Scheduling is driven by the playback scheduler (section 15): direction,
  speed, read-ahead window, and the GOP structure of each source.
- Long-GOP sources are seeked through the nearest keyframe using the index;
  for scrubbing they can use an intra-frame representation (section 14).
- Decoded frames live in a frame cache with GPU and CPU tiers and byte budgets.

## 14. Proxies, optimized media, and caches

### 14.1 What the user should feel

Footage always plays. Nobody has to learn what a proxy is to get smooth
playback, and a professional who wants exact control has every setting After
Effects and Final Cut Pro offer, in one place, with clear language.

### 14.2 Representations

Every video asset can have several representations, all derived from the
original:

| Representation | Purpose | Default recipe on macOS |
|---|---|---|
| **Original** | The truth. Always used for export and paused full-quality frames when available. | — |
| **Optimized** | Full resolution, intra-frame, fast to seek. For long-GOP H.264/HEVC sources that scrub poorly. | ProRes 422 (ProRes 4444 if the source has alpha) |
| **Proxy** | Reduced resolution for playback of heavy footage. | ProRes 422 Proxy at half or quarter size (ProRes 4444 at proxy size if the source has alpha) |

Windows recipes use DNxHR (LB for proxies, HQ/HQX for optimized; HQX or 444
for alpha) or hardware all-intra HEVC, decided in the Windows phase.

Representations are keyed by the original's fingerprint and the recipe, so
they survive renames, moves, and relinking, and are shared across projects.

### 14.3 Automatic mode (the default)

The engine decides per clip, based on measurements, not guesses:

1. On import, the probe measures decode cost for the source on this machine.
2. The policy estimates whether the source plays in real time at the sizes it
   is used at, with the number of simultaneous streams in the composition.
3. If it cannot, the engine creates the cheapest representation that makes it
   play: an optimized copy for scrubbing problems, a proxy for throughput
   problems.
4. Jobs run in the background at low priority, pause during playback and
   export, resume afterwards, and work on the used range of each clip (plus
   handles) before the rest.
5. Clips near the playhead and in the open composition go first.
6. The viewer chooses a representation per clip, per frame:
   - Playing: the best representation that sustains real time at the viewer's
     current size. A proxy is never used when the viewer is large enough to
     show the difference and the original keeps up.
   - Paused or scrubbing stopped: the original at full quality, as soon as it
     decodes. The frame upgrades in place.
   - Export: always the original.
7. If a representation is not ready, the engine uses the next best one. It
   never blocks playback waiting for a proxy.

### 14.4 Full control

All of this is in **Settings › Project › Media**, built from the existing
Settings rows, selects, and switches:

| Setting | Options |
|---|---|
| Proxies | Automatic, Always, Never |
| Proxy size | Automatic, Half, Quarter, Eighth, 1280 px long edge, 1920 px long edge |
| Proxy format | ProRes 422 Proxy, ProRes 422 LT, H.264 (smallest files), HEVC |
| Optimized media | Automatic, Always for long-GOP footage, Never |
| Store generated media | With the project, In the app's media cache, Custom folder… |
| Playback uses | Automatic, Originals only, Proxies when available |

And per clip, from the asset's context menu (an NSMenu, per native
conventions):

- Create Proxy, Create Optimized Media
- Rebuild, Delete Generated Media
- Use Original for This Clip (pins the original, even when playing)
- Attach Proxy File… (for camera-generated proxies, matched by name,
  timecode, and duration)
- Reveal in Finder

The viewer's existing resolution menu gains one section, "Playback":
Automatic, Originals, Proxies. Its button label shows, as quiet text, what is on
screen when it is not the original (for example "Proxy ¼").

Keyboard: a command to toggle between Originals and Proxies, bindable in the
keymap.

### 14.5 Status and storage

- Generation progress appears as one global status item ("Preparing media ·
  3 of 12") and in the asset's details. Lists stay quiet text, per the design
  rules: no badges, glyphs, or counts in rows.
- Offline originals with available proxies keep playing and editing. Export
  states the facts plainly: "2 clips are offline. Export uses their proxies."
  with Export Anyway and Locate… actions.
- **Settings › General › Storage** shows space used by proxies, optimized
  media, the render cache, the media index, and thumbnails and waveforms, each
  with Clear and a size limit. Generated media is disposable and rebuildable,
  so clearing it does not need a confirmation dialog. Clearing shows a toast
  with Undo where the files can be kept until the toast expires.

### 14.6 Caches

| Cache | Contents | Key | Location | Eviction |
|---|---|---|---|---|
| Decoded frame cache | Recently decoded source frames (GPU and CPU tiers) | source, representation, frame | Memory | Byte budget, LRU, protected around the playhead |
| Render cache (memory) | Rendered node outputs, including whole frames | Render graph node content hash, tick, resolution | GPU memory | Byte budget |
| Render cache (disk) | Rendered frames of heavy subtrees, for instant replay | Same | App cache or custom folder | Size limit, LRU |
| Media index | Frame indexes, probe results, decode cost | Original fingerprint | App cache | Rebuilt on demand |
| Thumbnails, waveforms | Filmstrips and peak data | Fingerprint, recipe | App cache | Size limit |
| Generated media | Optimized media and proxies | Fingerprint, recipe | Per the "Store generated media" setting | User-controlled |
| Shader cache | Compiled pipelines | Source hash, backend, driver version | App cache | Version change |

Because cache keys are content hashes of the render graph, an edit invalidates
exactly the nodes it changes. Background rendering (Final Cut Pro's background
render) fills the disk cache for heavy sections while the app is idle, governed
by a switch in Settings › General.

## 15. Playback

- **Master clock:** the audio output device clock (`pm-audio`). When no audio
  plays, a monotonic clock stands in. Video presentation follows the clock.
- **Render-ahead:** the scheduler renders frames ahead of the playhead into a
  small ring (default 4 frames, adaptive), decoding further ahead for each
  source according to its GOP and speed.
- **Frame pacing:** frames are presented at their target time through the
  frame bridge. The engine measures presentation latency of the bridge and
  Chromium's compositor and offsets audio to match. Measured A/V offset is part
  of the performance gate.
- **Drops:** if a frame cannot be ready in time, the previous frame stays on
  screen and the drop is counted and reported (today `E.drops` is declared and
  never updated).
- **Quality ladder:** under sustained load, Automatic quality lowers preview
  resolution in steps (as today: down to 0.25) and switches clips to proxies.
  Paused frames always render at full quality.
- **Scrubbing:** paused seeks coalesce to the latest target; the engine shows a
  complete frame before chasing the next target, as today. Intra-frame
  representations make scrubbing of long-GOP sources fast.
- **Loop and range:** work-area looping with pre-rolled loop entrances; play
  around current frame; J/K/L shuttle with speeds up to 8× and reverse; frame
  stepping.
- **Cached preview (RAM preview):** the engine can render a range into the
  render cache and play it back at full rate, with audio. The UI exposes this
  as a command (today `PM.Preview.cache()` exists without a trigger).

## 16. Audio

- **Engine:** `pm-audio` mixes in f32 at the project sample rate (48 kHz
  default, 44.1, 96 kHz available), resampling sources with `rubato`. Output via
  `cpal` (Core Audio, WASAPI).
- **Streaming decode:** no whole-file decode and no length cap.
- **Graph:** clip voices → per-layer strip (gain, mute, solo, meters) →
  precomp nesting (gains multiply as today) → master. The mixer UI keeps
  working on the same data; meters come from the engine.
- **Automation:** keyframed gain and fades are applied per sample, not as
  frame-step ramps.
- **Speed and time remapping:** audio follows speed changes and time remaps.
  Pitch is preserved by default (Signalsmith Stretch), with a per-clip option
  to let pitch follow speed. Reverse plays reversed audio. Today these clips
  are silent or wrong.
- **Waveforms:** peak data computed by the engine at multiple resolutions,
  cached on disk, and drawn by the timeline from shared data.
- **Export:** sample-accurate mix, rendered offline in ticks; 24-bit or 32-bit
  float PCM for ProRes and WAV, AAC through the platform encoder for MP4.
- **After parity:** pan, EQ, compressor, limiter, and loudness normalization
  (EBU R128 measurement exists in the agent tools today).

## 17. Export and delivery

### Pipeline

```
evaluate (pm-eval) → render (pm-render, float) → output transform (pm-color)
  → pixel format conversion (shader, with dithering when reducing depth)
  → encoder (VideoToolbox / FFmpeg / image writers) → muxer → verify → move
```

- Runs in the engine, never in the renderer, on its own queue. Users can keep
  editing while an export runs; the export renders a snapshot of the document
  at the revision when it started.
- **Multi-frame rendering:** several frames render concurrently when GPU memory
  allows, as in After Effects. The encoder receives them in order.
- **No pixel copies across processes.** Encoders read GPU surfaces directly
  (VideoToolbox) or a single readback in the engine (FFmpeg software encoders).
- **Atomic output:** writes go to a temporary file in the destination folder
  and are moved into place on success; failure or cancel leaves nothing
  behind.
- **Verification:** after encoding, a quick probe checks frame count, duration,
  and color tags. Optional full verification decodes every frame.

### Formats

| Format | Encoder | Options |
|---|---|---|
| ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ | VideoToolbox (macOS) | Alpha (straight or premultiplied) for 4444; audio PCM 24-bit or 32-bit float |
| H.264 | VideoToolbox (hardware), x264 when quality mode requests it and licensing allows | Bitrate or quality target, profile, keyframe interval, 8-bit 4:2:0 |
| HEVC | VideoToolbox | 8 or 10-bit, alpha, HDR (PQ/HLG metadata) |
| DNxHR LB/SQ/HQ/HQX/444, DNxHD | FFmpeg | MXF OP1a or MOV |
| AV1 | SVT-AV1 | 8 or 10-bit |
| VP9 WebM | libvpx | Alpha; frame-exact |
| Image sequences | PNG 8/16, TIFF 8/16/32f, OpenEXR half/float (PIZ, ZIP, DWAA), DPX 10/16, JPEG | Alpha, color tags or embedded profiles |
| Still | Same as sequences | Current frame |
| Audio only | WAV, AIFF, AAC | — |
| GIF and animated WebP | Native encoders with palette optimization | Loop count |
| Web/code export | Section 22 | — |
| Project package | `.pmv` with or without media (section 21) | — |
| Captions | SRT, WebVTT sidecars as today; burned-in option | — |

### Presets and the render queue

- Presets combine format, size, frame rate, color output, and audio. The
  existing presets and per-project export defaults migrate.
- A Render Queue panel runs jobs unattended: each job has a destination chosen
  when it is queued, so no dialog interrupts the run. Jobs survive relaunch.
  (Today the queue exists in code with no UI and prompts per job.)
- Export dialogs and the queue reuse the Settings sheet components, per the
  design rules.

### Color tags

Every output is tagged consistently with its encoding: BT.709 transfer (code 1)
for Rec.709 video, sRGB for sRGB image outputs, PQ and HLG with metadata for
HDR, and full or limited range explicitly. A test probes every format.

## 18. Viewer integration and the frame bridge

### Frame bridge

The design, to be proven by spike S1 before any viewer work:

1. The engine renders the displayed frame into one of a pool of 3 to 4 shared
   surfaces (`IOSurface` on macOS, D3D11 NT-handle textures on Windows) in an
   `rgba16float` or `bgra8` format chosen per display.
2. The surface handle is transferred to main (on macOS through a Mach port
   from `IOSurfaceCreateMachPort`, on Windows through `DuplicateHandle`). The
   `pm-bridge` addon in main turns it into a local handle and calls Electron's
   `sharedTexture.importSharedTexture`, then `sendSharedTexture` to the
   renderer.
3. The viewer receives a `VideoFrame`, imports it into its WebGPU canvas with
   `importExternalTexture`, and draws it, applying CSS-pixel scale and the
   current pan and zoom offset. When the frame is released, the surface returns
   to the pool.
4. Overlays (selection, handles, guides, text caret, pen paths) stay on the
   existing DOM and Canvas2D overlay above the canvas. They are drawn with
   geometry from `pm-wasm`, so they line up exactly.
5. Because frames are inside Chromium's compositor, `capturePage` (agent
   `capture_panel`, `computer_use_panel`) keeps working.

Fallbacks if S1 fails its gate:

- **A:** run the engine inside the main process (Rust threads, same message
  boundary), which removes the cross-process handle transfer.
- **B:** shared-memory CPU frames at reduced preview resolution.
- **C:** a native child view (`CAMetalLayer`) placed under a transparent region
  of the page, with input forwarded. Overlays stay in the DOM above it. Agent
  capture would need engine-side frame capture.

### Viewer behavior

- The viewer tells the engine its size in device pixels, DPR, zoom, pan, and
  preview resolution. The engine renders only the visible region at the
  required density (today's preview-viewport cropping, done by the render
  graph).
- During pan and zoom, the viewer reuses the last frame scaled while the engine
  catches up, as today's retained-image presentation does.
- Hit-testing and bounds are synchronous through `pm-wasm`.
- The `frame:rendered` and `overlay` events fire when a frame is presented,
  with the revision and time it represents.

## 19. Process model, IPC, and packaging

### Processes

| Process | Runs | Notes |
|---|---|---|
| Main | Windows, menus, files, save, agent bridge, `pm-bridge` | Unchanged except for the bridge |
| Renderer, per window | UI, kernel, extensions, edit pipeline, `pm-wasm` | The legacy compositor is removed at cutover |
| Engine, one per app | `pm-engine` in an Electron `utilityProcess`, loaded as a napi-rs addon | Owns the GPU device, decoders, audio device, caches; one session per open project tab |
| Extension sandboxes, transcription, ffmpeg tools, agent CLIs | Unchanged | — |

One engine process serves all windows so decode caches, proxies, and GPU memory
are shared. If the engine crashes, main restarts it. Each renderer re-sends a
full snapshot at its current revision, the viewer shows "Reconnecting…" for
the duration, and work continues. The document never lives only in the
engine, so an engine crash cannot lose edits.

Using a `utilityProcess` follows the transcription precedent: it gets
`MessagePort`s to each renderer (control and patches go renderer↔engine
directly, not through main's event loop), it is managed by Electron, and the
native addon is signed like the existing ones.

### Protocol

- Messages are defined in `pm-protocol`, serialized with a compact binary
  encoding over `MessagePort` (transferable `ArrayBuffer`s for bulk data).
- Requests carry ids; responses and events carry the document revision they
  correspond to.
- Groups: session (open, close, snapshot, patch), transport (play, pause, seek,
  rate, loop, step), viewer (size, viewport, resolution, display), render
  (frame, thumbnails, filmstrip, snapshot for agents), media (import, probe,
  relink, interpret), proxy (policy, jobs, status), export (start, cancel,
  progress, queue), audio (meters, waveforms, device), engine (budgets,
  stats, errors, version).
- Version negotiation at connect. The renderer refuses an engine whose major
  protocol version differs.

### Security

- The engine reads media paths the user imported or relinked; the renderer
  never passes arbitrary paths without main's validation, matching today's
  trusted-sender and path-validation rules.
- Decoders run in the engine process, isolated from the UI. Third-party SDKs
  (camera RAW later) run in further child processes.
- Fuzzing covers project parsing, patch application, and every demuxer and
  image reader we own.

### Packaging and CI

- Add a Rust toolchain to CI (stable, pinned) for `aarch64-apple-darwin`;
  `x86_64-apple-darwin` and Windows targets in their phases.
- `pm-node` and `pm-bridge` are napi-rs addons, listed in `asarUnpack` and in
  the `NATIVE_RUNTIME` verification list in `scripts/release-mac.mjs`, signed
  with the Team ID and hardened runtime.
- FFmpeg is built by us from source with a pinned, audited configuration
  (section 24) and linked or shipped as dylibs inside the app bundle. The
  `ffmpeg-static` dependency is removed.
- OCIO and glslang are built from source and linked statically.
- `pm-wasm` is built with `wasm-pack` and loaded by the renderer. The editor
  CSP gains `'wasm-unsafe-eval'` for the app origin only (Store sandboxes
  already allow it).
- Universal or per-architecture builds use electron-builder's
  `singleArchFiles` for the native addons.

## 20. Kernel API, extensions, and the agent harness

### API surface mapping

| API (today) | After cutover |
|---|---|
| `project.get/apply/time/play/undo` | Unchanged |
| `project.snapshot(t, maxWidth)` | Engine render, same return type |
| `anim.ev/evP/active/worldMatrix/localMatrix/resolveContent` | `pm-wasm`, synchronous, same signatures; times accept seconds |
| `anim.setKey` and in-place key helpers | Unchanged signatures; emit patches |
| `model.*` factories | Unchanged; produce schema v2 data |
| `groups.bounds`, `space3d.*` | `pm-wasm` |
| `transport.*` | Engine transport; `quality` and `previewResolution` map to engine settings |
| `media.importFiles`, `assets.get/add` | Engine import; `assets.get` returns a Blob read through main for compatibility |
| `media.audio.drawWaveform(ctx, …)` | Same signature, drawn from engine peak data |
| `render.gl.bounds/pick/previewViewport` | `pm-wasm` |
| `render.gl.init(canvas)/resize` | Viewer-internal; still accepted and mapped to the frame bridge |
| `render.gl.compileError` | Engine shader compiler, mirrored synchronously |
| `render.gl.context` (raw `WebGL2RenderingContext`) | Removed; returns `null` with a deprecation warning. Two built-in call sites are rewritten |
| `render.raster()`, `render.renderFrameTo()` | Engine-rendered, returned as `ImageBitmap`/canvas asynchronously; sync versions deprecated |
| `render.textLayout` | `pm-wasm` (`pm-text`), synchronous |
| `services` `raster` override | Supported for compatibility by uploading the override's pixels to the engine; marked deprecated in favor of extension layers |
| `effects/transitions/layers.register` | Unchanged GLSL contract (section 11) plus WGSL v2 |
| `events` `frame:rendered`, `overlay`, `time` | Unchanged names; fired on presentation |

`EXTENSION_API_VERSION` moves to 4 for the additions. Every apiVersion 1 to 3
extension keeps loading. The api-pack shipped to agents and authors is
regenerated and `docs/EXTENSIONS.md` is updated, including the existing
mismatch about which varyings effects receive.

### Agent harness

- `render_frames`, `media_contact_sheet` with a composition target, and
  `project.snapshot` call the engine directly. Renders no longer depend on a
  visible editor and become faster.
- New tools: `get_media_info` (engine probe including color metadata),
  `manage_proxies`, `export` with presets, `get_render_status`. Agents can
  render and verify exports headlessly.
- `capture_panel` and `computer_use_panel` keep working because the viewer is
  still inside Chromium.

## 21. Project files and media management

### Linked and embedded media

Professional footage is often hundreds of gigabytes. Today every asset is
copied into IndexedDB and embedded in the `.pmv`. The new model:

- **Embedded:** the bytes live in the project file, as today. Best for small
  assets and portable projects.
- **Linked:** the project stores the path, volume identity, fingerprint, size,
  modification time, and camera metadata (reel, timecode, duration). The bytes
  stay where they are.

Settings › Project › Media, "Imported media": **Copy into project**, **Link to
original files**, **Automatic** (default: link video and audio larger than a
size threshold, embed everything else). Each asset's details show which it
is, as text, and offer Embed or Link.

### Relinking

When linked media is offline, Powermove searches near the last location and
on mounted volumes by fingerprint, then name, size, duration, and timecode. A
Relink sheet lists offline assets with Locate… and batch relink by folder.
Offline clips with proxies keep playing.

### Collect and package

"Collect Project…" writes a folder with the project and all media (originals,
optionally generated media) for handoff or archive. "Package for Travel…"
includes proxies only, so a project can be edited on a laptop without the
originals and reconnected later.

### File format

- PMV4 keeps its container and durability guarantees. Linked assets add
  records without media bytes.
- Generated media and caches are never stored in the project unless "Store
  generated media: With the project" is chosen.
- Schema v2 documents carry `schemaVersion`; v1 documents migrate on open.

## 22. Web player, `powermove serve`, and the headless engine

### Web export and `packages/player`

Code export ships a JS runtime that reuses the editor's TypeScript evaluator
and WebGL compositor. Exports in the wild must keep working, and www and
onboarding use the evaluator.

1. **At cutover:** the current TypeScript evaluator and compositor move into
   `packages/player` as a frozen "web player v1" that only serves code export,
   www, and onboarding. The editor stops depending on them. Code export
   converts schema v2 to the v1 scene format (ticks to seconds). A parity test
   compares the web player's frames with the engine's in Standard mode.
2. **After cutover:** a WASM build of `pm-eval`, `pm-text`, `pm-vector`, and
   `pm-render` on WebGPU replaces web player v1 for new exports (scene format
   v2), with browser decoding through WebCodecs. Old exports keep their bundled
   v1 runtime.

### `powermove serve`

Paused today. When it resumes, the host runs the engine (requiring a GPU or a
software rasterizer on the host) and streams viewer frames to the browser
as low-latency encoded video, while the UI runs in the tab as today. Document
tools continue to run headless. A WASM engine in the tab is the alternative
for hosts without a GPU. Decision deferred.

### Headless rendering

`powermove-render` (from `pm-cli`) renders a `.pmv` to any export format
without Electron. It serves agents, automation, CI parity tests, and any future
render service. It accepts presets, ranges, and overrides of project
parameters (the `setParameter` and `setText` equivalents), which enables
template rendering.

## 23. Cross-platform

- **macOS (first):** Apple Silicon. Intel builds only if demand justifies the
  CI and test cost.
- **Windows (after cutover):** wgpu DX12, D3D11/D3D12 video decode through
  FFmpeg hwaccel, Media Foundation where it is needed, WASAPI audio,
  DirectWrite fonts, NT-handle shared textures for the frame bridge. The UI
  shell needs a Windows Electron build, installer, signing, and native
  conventions review; that is a parallel product lane.
- **Linux:** `powermove-render` and `powermove serve` only, with Vulkan or a
  software fallback.
- Platform code lives behind traits in `pm-media` (decoder backends),
  `pm-gpu` (surface sharing), `pm-text` (font discovery), and `pm-audio`
  (device). Every platform backend has a software fallback for tests.

## 24. Licensing and compliance

These must be settled with counsel before the relevant phase ships. They are
listed here so they are not discovered late.

1. **FFmpeg build.** Replace `ffmpeg-static` with an FFmpeg we build from source
   with an audited configuration: LGPL or GPLv3-compatible components only,
   never `--enable-nonfree`. This is a prerequisite in Phase 0 regardless of
   the rewrite.
2. **ProRes.** Apple licenses ProRes and lists FFmpeg's implementation as
   unauthorized. On macOS, use VideoToolbox (Apple's implementation) for ProRes
   decode and encode. Windows ProRes requires joining Apple's licensing program
   or omitting ProRes encode on Windows.
3. **H.264 and HEVC encode.** Prefer the OS encoders (VideoToolbox, Media
   Foundation). Confirm royalty exposure for any bundled software encoder
   (x264, x265) given Powermove's pricing.
4. **AGPL and proprietary SDKs.** Camera RAW SDKs (BRAW, R3D, ARRI, Canon) are
   closed. Linking them into an AGPL program needs a GPLv3 section 7 additional
   permission from all copyright holders, which needs a contributor policy that
   allows it. Settle before any camera RAW work.
5. **Dependencies.** Prefer MIT, Apache-2.0, BSD, and Zlib crates. A
   `cargo-deny` policy in CI enforces the allowed license list and checks
   advisories (for example, `rustybuzz` and `ttf-parser` now carry
   unmaintained advisories, which is one reason HarfRust and Skrifa are chosen).

## 25. Testing, parity, and performance gates

### Parity corpus

- **Capture.** Before any engine code lands, record the legacy engine's output
  for a corpus of projects: the 12 existing pixel-hash cases, every e2e
  fixture, synthetic projects per feature (one per layer type, effect,
  transition, blend mode, matte mode, mask type, text animator, 3D feature,
  retiming mode, motion blur), and a set of real-world projects Jude supplies
  (kept outside the repository). Output: float EXR and 8-bit PNG per frame at
  chosen times, plus evaluated property dumps as JSON.
- **Evaluation parity.** `pm-eval` must match the legacy evaluator's property
  dumps within 1e-9 relative for numeric values and exactly for strings and
  booleans, across the whole corpus.
- **Render parity (Standard mode).** Per-channel absolute difference at 8-bit:
  at most 1 for 99.9% of pixels and at most 4 for any pixel, excluding text
  and known improvements. Text uses SSIM ≥ 0.98 plus a reviewed side-by-side
  sheet. Known improvements (edge antialiasing, blur kernel, alpha rounding)
  are listed per case with a reason; each one is reviewed.
- **Correctness tests (Linear mode).** Analytic expectations: energy-conserving
  blur, linear opacity, color patch accuracy, gradient smoothness, alpha
  round-trips, and the bars in section 2.
- **Codec conformance.** For every supported format: decoded frames match
  FFmpeg reference decodes (framemd5 or within 1 code value for hardware
  decoders), random access equals sequential decode, frame counts and
  timestamps are exact.
- **Export conformance.** Every format probed for codec, profile, pixel format,
  bit depth, color tags, frame count, duration, audio sample count, and alpha.
- **Determinism.** Every corpus case renders twice with byte-identical output.

### Infrastructure

- Rust unit and property tests per crate; `cargo-fuzz` targets for parsers.
- A parity runner (`pm-cli parity`) that renders the corpus and produces a
  report with difference images, used locally and in CI.
- Existing Playwright e2e specs run against the new engine through the same
  UI. Specs that depend on WebGL internals are rewritten to use engine stats.
- Existing harnesses keep their role: `native-export-performance`,
  `opaque-capture-parity`, `export-video-frames`, `video-frame-integrity`,
  `rendering-performance` hashes, `export-code` player parity.
- A debug "Compare engines" command in the app renders the current frame with
  both engines and shows the difference, used during migration and removed at
  cutover.

### Performance gates

Targets to be calibrated in Phase 0 on reference machines (a base M-series
MacBook Air and an M-series Pro), then fixed:

| Scenario | Gate |
|---|---|
| 4K ProRes 422 HQ, 3 stacked layers with a blur, Full resolution | Real time at 23.976 and 29.97 on the Pro machine, zero drops over 60 s |
| 4K long-GOP HEVC 10-bit, single layer, Automatic | Real time on both machines, using proxies only if the policy says so |
| Scrub latency, intra codecs | p95 under 50 ms from pointer to presented frame |
| Scrub latency, long-GOP with Automatic | p95 under 100 ms |
| The 884-layer project from the large-project suite | All five failing ceilings in `memory-investigation.md` pass |
| Export, 4K ProRes 4444, trivial project | At least 4× today's measured rate |
| UI responsiveness during playback | Inspector edit latency p95 under 16 ms with 1,000 layers |
| A/V sync | Measured offset under 1 frame at presentation |
| Memory | Engine footprint within the user's budget setting; no unbounded growth over a 1-hour playback loop |
| Cold start | Engine ready before the window finishes loading the last project |

## 26. Phases

Phases are ordered by dependency. Several run in parallel lanes (section 27).
Each ends at a gate; a phase is done when its gate passes, not when its code
lands. The legacy engine stays the default until Phase 7.

### Phase 0: Foundations and spikes

Work:

- Cargo workspace, CI toolchain, `cargo-deny`, napi-rs and wasm-pack builds,
  signing and notarization of a placeholder addon through the real release
  script.
- Replace `ffmpeg-static` with our own FFmpeg build (section 24.1).
- Parity corpus capture from the legacy engine (section 25).
- Fix the cheap defects in appendix A that change current output only where it
  is wrong (color transfer tag, motion blur sample count consistency, the
  ProRes original being replaced), so the corpus captures intended behavior.
- Spikes, each a throwaway branch with a written result:
  - **S1 frame bridge:** engine process → IOSurface → main `sharedTexture` →
    renderer WebGPU at 4K 60 fps. Measure frame pacing, latency, CPU use, and
    whether `rgba16float` values above 1.0 survive to the screen. Test
    `capturePage` sees the frame.
  - **S2 precision path:** ProRes 4444 XQ in (VideoToolbox) → wgpu f16 linear →
    ProRes 4444 out (VideoToolbox). Verify the bit-depth bar in section 2.
  - **S3 shader compatibility:** translate every GLSL source in the repository,
    the Store, and the docs samples through the section 11 pipeline; compare
    output images with WebGL.
  - **S4 text and vectors:** Parley + Skrifa + Vello versus cosmic-text, and
    Vello versus own tessellation, against Canvas2D and CoreText on a font and
    shape corpus.
  - **S5 WASM evaluator:** load a WASM module in the renderer under the CSP
    change; measure `evP` call cost from JS (target under 1 µs for cached
    values) and patch application cost (target under 0.2 ms per typical edit).
  - **S6 decode:** VideoToolbox and FFmpeg decode of the fixture matrix into
    wgpu without CPU copies; frame-exact random access via the index.

Gate: all spikes have a written result and a chosen path; corpus captured;
FFmpeg replaced and shipped; performance targets calibrated; Jude approves
continuing.

### Phase 1: Time, model, and evaluation

Work: `pm-time`, `pm-model` (schema v2, migrations), `pm-eval`, `pm-vector`
geometry, `pm-text` layout, `pm-protocol`, `pm-wasm`. Patch emission for all
writers and the mutation auditor. A flag routes the renderer's synchronous
queries (`anim.*`, `pick`, `bounds`, `textLayout`) to `pm-wasm` while the
legacy compositor still renders.

Gate: evaluation parity on the whole corpus; e2e suite green with the flag on
and the auditor on; schema v1 → v2 migration round-trips every corpus project;
query latency targets met.

### Phase 2: Media I/O

Work: `pm-media` probe, index, decode backends, image readers, color metadata;
`pm-audio` streaming decode and waveforms; the engine process skeleton
(`pm-engine`, `pm-node`) with sessions and the protocol; import routed through
the engine with originals preserved.

As an early user-visible win, decoded frames can feed the legacy compositor
through the frame bridge (as `VideoFrame` textures), so ProRes and other
formats play from originals before the new compositor exists.

Gate: codec conformance suite passes; imports keep originals; ProRes plays in
the legacy compositor without transcoding.

### Phase 3: Renderer and export

Work: `pm-gpu`, `pm-render` (all layer types, effects, transitions, masks,
mattes, blend modes, motion blur, 2.5D, precomps, adjustment layers, groups),
`pm-color` (Standard and Linear modes, input and output transforms), the GLSL
compatibility pipeline, `pm-export` with all formats, the render queue, and
`pm-cli`. Export routed to the engine behind a "Use new engine for export"
switch.

Gate: render parity in Standard mode across the corpus; Linear-mode
correctness tests; export conformance for every format; export performance
gate; determinism.

### Phase 4: Live viewer and playback

Work: frame bridge in production, viewer integration, overlays from
`pm-wasm`, engine playback with the audio clock, scrubbing, J/K/L, loop,
cached preview, Automatic quality, the full audio engine including stretch,
meters, and the mixer. Behind a "Use new engine for preview" switch.

Gate: all playback performance gates; A/V sync gate; e2e suite green on the new
engine; agent capture tools work; crash and GPU-loss recovery tests pass.

### Phase 5: Proxies, optimized media, and caches

Work: `pm-proxy` policy and jobs, representation selection in the scheduler,
render cache (memory and disk), background rendering, the Media settings,
per-clip commands, viewer playback menu, status item, Storage settings,
linked media, relinking, Collect and Package for Travel.

Gate: the automatic policy keeps every performance-gate project in real time
on the base machine without user action; proxies never appear in exports
unless the user accepts the offline-media prompt; generated media survives
moves and relinking; storage limits are enforced; UI review by Jude.

### Phase 6: Professional color and finishing

Work: ACES mode with OCIO, working spaces, 32-bit float, Interpret sheet,
camera log curves, LUTs, color effects, HDR preview and PQ/HLG export, scopes
panel, gradient interpolation options.

Gate: color conformance bars (section 2) on all outputs; HDR round trip; scopes
match reference implementations on test patterns; UI review by Jude.

### Phase 7: Cutover

Work: new engine on by default; one release with the legacy engine available
behind a switch; then removal of `legacy/gl`, the legacy media, playback,
audio, and export paths; web player v1 moved into `packages/player`;
`EXTENSION_API_VERSION` 4; docs updated (`EXTENSIONS.md`, README, the design
docs that describe the old engine).

Gate: no regressions reported during the switch release that are not fixed;
legacy code deleted; all docs updated.

### After cutover

In rough priority order, each its own plan:

1. Windows build of the full app.
2. Depth-buffered 3D, then camera and light layers.
3. Spatial bezier motion paths.
4. Camera RAW, starting with BRAW and R3D in separate processes (after the
   licensing work in section 24.4).
5. WASM + WebGPU web player (scene v2).
6. Audio effects and loudness tools.
7. OpenFX hosting out of process.
8. Edit pipeline and history in Rust (`pm-doc`) for fully headless editing.
9. PSD and multi-part EXR layer import.

## 27. Lanes and ownership

Work splits into lanes with clear crate ownership so they can proceed in
parallel. Interfaces between lanes are the crate APIs and `pm-protocol`,
agreed at the start of each phase.

| Lane | Owns | Phases |
|---|---|---|
| Model and evaluation | `pm-time`, `pm-model`, `pm-eval`, `pm-wasm`, renderer patch emission, auditor | 1, then support |
| Text and vectors | `pm-text`, `pm-vector` | 0 (S4), 1, 3 |
| Media | `pm-media`, decode backends, image readers, probe, index | 0 (S6), 2, 5 |
| GPU and render | `pm-gpu`, `pm-render`, GLSL compatibility | 0 (S3), 3, 4 |
| Color | `pm-color`, OCIO bridge, color effects, scopes | 0 (S2), 3, 6 |
| Engine and platform | `pm-engine`, `pm-node`, `pm-bridge`, `pm-protocol`, packaging, CI, FFmpeg build | 0 (S1), 2, 4 |
| Audio | `pm-audio`, mixer integration | 2, 4 |
| Export | `pm-export`, `pm-cli`, render queue | 3 |
| Proxies and storage | `pm-proxy`, `pm-cache`, media management UI | 5 |
| UI integration | Viewer, settings, panels, kernel API mapping, extension docs | 1, 4, 5, 6, 7 |
| Parity and QA | Corpus, parity runner, conformance suites, performance gates | 0 onward |

Rules:

- Every lane works in its own worktree and branch, commits granularly, and
  submits stacked PRs.
- Reviews use independent reviewers. The parity and QA lane owns the gates and
  reports failures as they are, with no loosened thresholds.
- The legacy engine is frozen for features from the start of Phase 3. Bug fixes
  continue. New engine features land only in the new engine.

## 28. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `sharedTexture` is experimental, main-process only, and tracks Chromium internals | Viewer path breaks on an Electron upgrade | Spike S1 first; fallbacks A to C; pin Electron upgrades behind a frame-bridge test |
| Text does not match designers' expectations | Visible regressions on every project | Spike S4; compatibility metrics; reviewed text corpus before cutover |
| GLSL translation fails on some user shaders | Broken Store extensions and saved projects | Spike S3 across all known sources; keep a GLSL-to-WebGL fallback path in the web player; clear error with line mapping |
| Two document copies (renderer graph and `pm-wasm` mirror) diverge | Wrong pixels or picks | Patch emission everywhere; mutation auditor in CI; revision-gap resync |
| Scope: the engine replaces browser decode, text, and audio, not just WebGL | Long road to parity | Phases deliver value early (ProRes playback in Phase 2, export in Phase 3); legacy stays default until gates pass |
| Feature drift while the rewrite runs | Double work | Legacy feature freeze from Phase 3 |
| wgpu breaking releases | Upgrade churn | Pin versions; upgrades as deliberate lanes |
| Licensing (ProRes on Windows, HEVC encode, RAW SDKs under AGPL) | Blocked features | Section 24 settled before the dependent phase |
| Memory pressure from decoders, caches, and two document copies | Crashes on 8 GB machines | One budget authority in the engine; tests on the base machine |
| Engine crashes | Lost frames, broken export | Crash isolation; automatic restart with resync; atomic export output |
| Hardware decoder differences across Apple Silicon generations | Inconsistent behavior | Software fallback; conformance on more than one generation |

## 29. Decisions needed from Jude

1. **Default color mode for new projects:** Linear (recommended) or Standard.
2. **Default media storage:** Automatic link-or-embed (recommended), Copy into
   project, or Link.
3. **Where generated media lives by default:** app media cache (recommended) or
   next to the project.
4. **Windows timing:** right after cutover (as written) or later.
5. **Intel Mac support:** no (recommended) or yes.
6. **Web export future:** keep v1 JS player indefinitely, or replace it with the
   WASM engine after cutover (recommended).
7. **Licensing counsel** for section 24 items 2 to 4.
8. **Real-world corpus projects** to include in parity (stored outside the
   repository).

## 30. Appendix A: defects found during the survey

Found while mapping the current engine. Several are worth fixing in the legacy
engine during Phase 0, so the parity corpus captures intended behavior.

1. **Converted video replaces the original.** For ProRes, MKV, MXF, and other
   FFmpeg-only formats, the VP9 transcode is stored, saved, and exported
   instead of the original (`raster.ts:874-889`, `media/media-path.ts:29`). The
   README says the original stays embedded; it does not.
2. **Export transfer tag.** H.264 and ProRes are tagged with sRGB transfer
   (`iec61966-2-1`) rather than BT.709 (`render-encoder.ts:12-15`).
3. **ProRes precision.** ProRes 4444 is written at 10-bit with `-alpha_bits 16`
   from 8-bit data; 24-bit PCM is padded from 16-bit WAV.
4. **Motion blur sample counts differ by path:** 16 (native opaque, PNG), 20
   (alpha, WebM, Recorder), 6 (web player), 6/12 (preview).
5. **WebM and Recorder exports may drop shaders still compiling.** They render
   through `GL.render` without `exporting: true`, so parallel compilation can
   skip a layer on early frames (`exporter.ts:609-613`, `compositor.ts:1752`).
   Inferred from code; not reproduced.
6. **Render queue has no UI** and prompts per job, so it cannot run unattended.
7. **Comp frame rate handling differs by path:** `set_composition` and Project
   Settings round to integers; `Comps.writeSettings` does not.
8. **Timecode at fractional rates is wrong** (`util.ts:197-205`).
9. **Variable speed integrates at the root project's fps** in nested comps
   (`retiming.ts:13-17`).
10. **Audio ignores clip speed**, and time-remapped clips are silent.
11. **WebM exports carry no color metadata.**
12. **Quad edges are aliased** (hard UV cut, `antialias: false`), and there are
    no mipmaps.
13. **Large blur radii skip texels** (`effects-basic/effects.ts:43-49`).
14. **Docs disagree** on effect varyings (`EXTENSIONS.md:439` versus
    `api.ts:180-191`), and on when editing copies are made
    (`video-preview-performance.md` versus `video-preview.ts:23-24`).
15. **Tracked Finder duplicates** (`* 2.ts` files) in `src/shared`,
    `legacy/core`, and `main`.

## 31. Appendix B: feature parity inventory

Everything below must work in the new engine before cutover.

**Composition.** Size, rational frame rate, duration, work area, background
(none, solid, linear and radial gradient up to 8 stops), shutter angle, master
audio gain, markers, scene parameters, multiple compositions, nesting, open
composition switching.

**Layers.** solid, shape (rect, ellipse, polygon, star, line, editable paths),
text, image, SVG, video, audio, adjustment, shader, extension (fragment and
mesh), null, group, precomp, captions. Switches: visibility (keyframable), lock,
shy, solo, collapsed, color label, motion blur, 3D, linked scale. Parenting
that preserves pose. Groups as transform spaces and compositing boundaries.

**Transforms.** Anchor, position, scale, rotation, opacity, skew; 3D position,
anchor and scale Z, rotation X and Y, orientation, perspective per 3D group.

**Animation.** Keyframes with linear, bezier, hold; speed and influence;
auto-bezier and continuous; spring parameters; legacy easing presets; color
interpolation; string and boolean steps; expressions with all helpers
(`clamp`, `lerp`, `linear`, `ease`, `random`, `wiggle`, `bounce`, `loop`,
`pingpong`, `param`) and identifiers.

**Shapes and paths.** Fill and stroke, per-corner radius, smoothing
(squircles), polygon and star points, cubic paths with per-vertex tangents,
trim start/end/offset, repeater (up to 256 copies), fill and stroke opacity.

**Text.** Font, weight, size, tracking, leading, color, alignment, italic, box
width and height, variable font axes, range styles, animators (stagger, range,
wave; characters, words, lines; all orders and shapes; opacity, position,
scale, rotation, skew, tracking, blur, color), typewriter, font anchoring,
local and embedded fonts.

**Masks and mattes.** Up to 8 parametric rect and ellipse masks with feather,
rotation, add and subtract; bezier path masks; track mattes alpha, alpha
inverted, luma, luma inverted; matte nesting and cycle rejection.

**Compositing.** Nine blend modes (normal, add, screen, multiply, overlay, soft
light, difference, lighten, darken), opacity, group effects, adjustment layers,
motion blur, in and out transitions per layer.

**Effects.** Gaussian Blur, Directional Blur, Sharpen, Glow, Vignette, Drop
Shadow, Color, Levels, Duotone, Posterize, Invert, Film Grain, Mosaic,
Chromatic Aberration, Turbulent Displace, Gradient Ramp; multi-pass, backdrop,
`keepOrig`, viewport-safe and padding semantics; extension effects.

**Transitions.** Crossfade, Wipe, Slide, Zoom; extension transitions.

**Shader layers and extension layers.** Shadertoy-style header, `@param`
uniforms, 5 presets, Studio Cube, OBJ model.

**Retiming.** Speed (animated), time remap, freeze, reverse, source time.

**Media.** Every import format in section 13 that exists today, animated
images, image sequences with gap reporting, posters, iCloud/File Provider
materialization, URL import for extensions, remote media in serve mode.

**Audio.** Per-layer gain, mute, solo, fades, keyframed gain, precomp nesting,
embedded video audio, meters, waveforms, master gain.

**Preview.** Resolution (Auto, Full, Half, Quarter), preview frame rate,
Automatic quality, frame cache, cached preview, retained presentation during
pan and zoom, viewport-cropped rendering, context-loss recovery, FPS readout.

**Export.** Every format in today's dialog, presets, per-project defaults,
work-area range, scale, fractional rates, captions sidecars and burn-in,
progress and cancel, render queue, code export, project export.

**Agent and kernel.** Every API in section 20; `render_frames`, contact sheets,
snapshots, panel capture.

## 32. Appendix C: glossary

- **Tick:** 1/705,600,000 second; the engine's unit of time.
- **Representation:** a version of a media asset: original, optimized, or
  proxy.
- **Optimized media:** a full-resolution, intra-frame copy for fast seeking.
- **Proxy:** a reduced-resolution copy for playback.
- **Working space:** the color space the compositor computes in.
- **Standard mode:** display-referred blending that matches pre-engine
  projects.
- **Linear mode:** scene-referred, linear-light blending.
- **Frame bridge:** the path that carries rendered frames from the engine to
  the viewer as shared GPU surfaces.
- **Mirror:** a copy of the document kept in sync by patches (`pm-wasm` and
  engine sessions).
- **Parity corpus:** the projects and expected outputs captured from the legacy
  engine.
