# Powermove native engine: plan and design

Status: proposal, not started. Revision 2, after independent review by five
reviewers (video and color science, codebase fidelity, architecture, product
and proxy UX, Rust technology feasibility) and four supporting audits (agent
tools, caches, engine consumers, current code).

This document covers the whole rewrite of the Powermove engine (evaluation,
decode, compositing, color, text, audio, playback, caching, proxies, and
export) from TypeScript and WebGL2 into Rust. The Svelte UI, the kernel,
extensions, and the agent harness stay on the web stack.

It is written to be executed by parallel implementation lanes. Every phase
ends at a gate with objective exit criteria. A phase is done when its gate
passes, not when its code lands.

Naming: "the engine" in this document means the new native engine. The
existing Node document host in `apps/desktop/src/engine/` is called "the
headless document host" to avoid confusion.

Contents

1. Why
2. Goals, non-goals, and the quality bar
3. What exists today
4. Architecture overview
5. Time
6. The document: ownership, writes, and mirrors
7. Evaluation (`pm-eval`)
8. GPU and the render graph (`pm-render`)
9. Color pipeline
10. Layer types
11. Effects, transitions, and the shader contract
12. Text
13. Media I/O (`pm-media`)
14. Proxies, optimized media, and caches
15. Playback
16. Audio
17. Export and delivery
18. Viewer integration and the frame bridge
19. Process model, protocol, security, packaging
20. Kernel API, extensions, the agent harness, and other consumers
21. Media management and project files
22. Web player, `powermove serve`, and the headless document host
23. Cross-platform
24. Licensing and compliance
25. Testing, parity, and gates
26. Phases
27. Lanes, contracts, and ownership
28. Risks
29. Decisions needed from Jude
30. Appendix A: defects found during the survey
31. Appendix B: feature parity inventory
32. Appendix C: integration checklist
33. Appendix D: glossary

---

## 1. Why

Powermove is used by more professional editors and motion designers every
release. Their footage, codecs, and color expectations exceed what the
browser engine can deliver. The survey behind this document found the limits
are structural:

- **Codecs.** Chromium cannot decode ProRes, DNxHR, MXF, camera RAW, or most
  intermediate codecs. Powermove transcodes them on import to VP9 `yuva420p`
  (CRF 18) with Opus audio (`main/media-proxy.ts:39-51`), and that transcode
  is what gets stored, saved, and exported. The original is never kept; only
  its path and fingerprint are (`legacy/gl/raster.ts:874-889`). A ProRes 4444
  XQ source with PCM audio is edited and exported from a lossy 8-bit 4:2:0
  copy with lossy audio.
- **Precision.** Every source texture is RGBA8 with premultiply-on-upload
  (`legacy/gl/compositor.ts:541-545`). Intermediates are RGBA16F, but export
  readback is 8-bit (`compositor.ts:1825`). ProRes 4444 is written as 10-bit
  with `-alpha_bits 16` from 8-bit data.
- **Color.** There is no color management. Blending, blur, opacity, and motion
  blur run on gamma-encoded values. Import ignores primaries, transfer,
  matrix, range, and HDR metadata. Export tags H.264 and ProRes with the sRGB
  transfer (`iec61966-2-1`), which most NLEs and players read differently from
  BT.709.
- **Time.** Everything is float seconds. Frame rates are rounded to integers
  by the UI and `set_composition` but kept fractional by load and validation.
  Timecode is wrong at fractional rates. Nested compositions are never
  re-quantized.
- **Audio.** WebAudio with whole-file decode and a 128 MiB PCM budget (about
  5.8 minutes of stereo float). Speed changes are ignored and time-remapped
  clips are silent.
- **Export.** Frames are rendered in the renderer, read back, and copied over
  IPC in 4 MiB chunks to an FFmpeg child process. A 4K frame is 33 MB. The
  measured H.264 rate is about 11 fps at 4K for a trivial project
  (`apps/desktop/docs/native-export-performance.md`).
- **Headless.** No pixels can be rendered without a browser tab. Agents, the
  CLI, and any future render service need that.

A native engine fixes these at the root and gives Powermove one renderer that
runs in the app, in a CLI, and later on Windows.

## 2. Goals, non-goals, and the quality bar

### Goals

1. **Image fidelity.** Float precision from decode to encode. Nothing drops to
   8-bit unless the delivery format requires it.
2. **Color management.** Explicit, tested color invariants; per-source input
   transforms; display transforms at the edge; linear-light compositing as a
   choice; OCIO and ACES; LUTs; HDR delivery.
3. **Native codecs.** Read and write the formats professionals use, from the
   original media, with hardware acceleration where available.
4. **Exact time.** Rational frame rates, integer ticks inside the engine,
   frame-accurate decode, drop-frame timecode, sample-accurate audio.
5. **One renderer.** Preview, export, thumbnails, agent snapshots, and headless
   renders produce the same pixels for the same inputs and precision.
6. **Proxies done right.** Full control, with an automatic default that needs no
   setup and never puts proxy pixels into an export by accident (section 14).
7. **Speed.** Real-time multi-layer 4K playback on Apple Silicon, pipelined
   export, no rendering on the UI thread.
8. **Keep what works.** The Svelte UI, the kernel and its frozen contracts, the
   extension system and Store, the agent harness, Undo, and the file format's
   durability guarantees.
9. **Portability.** macOS first. Platform code sits behind traits, so Windows is
   a port.

### Non-goals

- Rewriting the UI, timeline, inspector, or kernel in Rust.
- Changing the document's JSON shape or the public API's units before cutover
  (section 6).
- Rewriting the agent harness. It gains capabilities only.
- OpenFX hosting, camera RAW SDKs, camera and light layers. Planned after
  cutover (section 26).

### The quality bar

Each line is an automated test (section 25). "Code value" is abbreviated CV.

| Area | Bar |
|---|---|
| Color invariants | (1) With no effects, a Rec.709 source exported to Rec.709 at the same bit depth and range matches a reference decode exactly at 32-bit float and within 1 CV at 16-bit float, in Standard and Linear modes. (2) A solid set to `#RRGGBB` exports to sRGB and Rec.709 outputs and decodes, with the tagged matrix and range, to the same RGB codes within 1. (3) The viewer shows an exported Rec.709 file the way AVFoundation shows it, verified on a test chart through ColorSync. |
| Pipeline transparency | ProRes 4444 XQ decode → engine (no effects) → uncompressed output (v210, 16-bit TIFF, or EXR) matches a VideoToolbox reference decode exactly at 10-bit and within 1 CV at 12-bit. |
| Codec generation | ProRes re-encode: PSNR against the source decode is at least that of Apple's own re-encode of the same frames, and maximum error is at most 4 CV. |
| Bit depth | A 16-bit grey ramp across at least 1024 columns, exported to a 10-bit limited-range format with dither off, contains every luma code 64 to 940 (877 levels), is monotonic, and has no step larger than 1 CV. Full-range RGB outputs (16-bit TIFF to 10-bit DPX) contain all 1024 codes. |
| Alpha | ProRes 4444 sources round-trip alpha within 1 CV at 12-bit. Straight and premultiplied output are both available and correct. HEVC with alpha is measured by PSNR, because Apple codes its alpha lossily. |
| SDR color | Decoded patch centers (64×64 flat areas) within 1 CV of analytically computed expected values; ΔE2000 maximum 1.0 and mean 0.5 at 8-bit, maximum 0.5 at 10-bit; decoded with two independent references (FFmpeg with zimg, and AVFoundation). |
| HDR color | PQ and HLG outputs within ΔE_ITP (ITU-R BT.2124) 1.0 maximum on reference patches. |
| Linear light | In Linear mode with sRGB output, 50% white over black encodes to 0.7354 (188 of 255). A 1-pixel black and white checkerboard downscaled 2× yields 188, not 128. A Gaussian blur of a white line conserves energy within 0.5%. |
| Time | At 23.976, 24, 25, 29.97 (DF and NDF), 59.94, and 60, every exported frame carries a burned-in binary frame code equal to its frame number up to N = 1,000,000. Drop-frame vectors: 29.97 frame 1800 = 00:01:00;02, frame 17982 = 00:10:00;00; 59.94 frame 3600 = 00:01:00;04. Invalid drop-frame strings are rejected. |
| Seeking | For every supported codec, including open-GOP HEVC, B-pyramid H.264, and files with edit lists, random access to frame N returns the same pixels as sequential decode of frame N. |
| Audio | A click at frame N lands at sample floor(N × sr × den / num) ± 1. Total exported samples equal round(duration × sr). No length limit. |
| Antialiasing | Zone-plate and Siemens-star renders under animated scale 1.0 → 0.25 and rotation match a 16× supersampled reference with PSNR ≥ 40 dB per frame, and frame-to-frame flicker energy at most 1.5× the reference. |
| Proxies | No exported frame contains proxy pixels unless the user chose a draft render. A paused frame shows the original within 150 ms on the base reference machine (4K ProRes). Proxy and original at matched scale differ by at most ΔE2000 1.0 on test patches. |
| Parity | Every project in the parity corpus renders through the engine in Standard mode within the tolerances in section 25. |
| Determinism | Two renders of the same frame on the same machine produce byte-identical buffers and byte-identical lossless outputs. |

## 3. What exists today

Full inventories are in appendices B and C.

### Code that the engine replaces

Non-test lines, excluding the tracked `* 2.ts` duplicates.

| Area | Main files | LOC |
|---|---|---|
| Model evaluation, animation, expressions, retiming, 3D math, text animators | `legacy/core/{anim,temporal-bridge,easing,expression,retiming,parenting,layer-groups,space-3d,text-animation,text-layout,split-text,vector-paths,content-properties,size-anchor,font-anchor,matte}.ts`, `core/anim/temporal-ease.ts` | 3,190 |
| Compositor, rasterizer, shaders, GL caches, asset import half of `raster.ts` | `legacy/gl/*` | 4,196 |
| Media import, decode, posters, editing copies, preview caches | `legacy/core/{media,media-conversion,video-*,image-sequence,poster,preview-cache,preview-quality,memory,cloud-media}.ts`, `main/media-proxy.ts` | 1,890 |
| Playback loop | `legacy/core/engine.ts` | 503 |
| Audio and mixer engine | `legacy/core/audio.ts`, non-UI mixer | 1,825 |
| Export | `legacy/core/{exporter,frame-capture,frame-preparation,render-queue}.ts`, `main/render-encoder.ts` | 1,003 |
| Built-in GLSL effects, transitions, 3D layers | `extensions/{effects-basic,transitions-basic,layers-3d}` | 574 |
| Other engine code | `legacy/core/{compositions,svg-import,fonts}.ts`, `kernel/{glsl,obj,extension-layers}.ts`, `captions/render.ts` | 1,540 |

About 14,700 lines. The web player (`renderer/src/player/*`, 873 lines) also
runs this code and is handled in section 22. The Rust replacement will be
larger, because it takes over decode, color, text shaping, and audio work the
browser does today.

### What stays

- The renderer UI, panels, Settings, and all built-in UI extensions (viewer,
  timeline, inspector, toolbar, mods, keymap, theme).
- The kernel and its public API, extended as described in section 20.
- The edit pipeline (`legacy/core/editing.ts`), history and Undo, the command
  vocabulary, validation, and `legacy/core/compositions.ts` composition
  management (with its geometry calls redirected).
- The agent harness, MCP bridge, and main-process media tools.
- The `.pmv` container and its durability (`main/project-save.ts`,
  `project-files.ts`, `durable-file.ts`).

### Constraints inherited from earlier decisions

From `apps/desktop/docs/*`:

- **One compositor for preview, capture, export, and nested comps**
  (`rendering-performance.md`).
- **Pixel-exact acceptance for optimizations.** Performance work never changes
  output.
- **Bounded expression interpreter, no `'unsafe-eval'` in the editor.** The
  Rust interpreter keeps the limits: 2,000 characters, 160 nodes, depth 24, 12
  arguments. (The script sandboxes do allow `unsafe-eval`; they are not part of
  the engine.)
- **One user intent equals one Undo step.**
- **Effect contract limits.** Effects: 32 params, 1 to 8 passes, fragment at most
  65,536 characters. Extension layers: 48 params, fragment at most 96,000 UTF-8
  bytes. GLSL ES 3.00 sources stored in projects and Store extensions keep
  working (section 11).
- **Per-layer failure isolation.** A failing layer is reported and skipped.
- **GPU loss recovery** without replacing the project; exports abort on loss
  without writing bad bytes.
- **Predictability over raw speed.**

## 4. Architecture overview

```
┌──────────────────────────── Electron app ──────────────────────────────┐
│                                                                         │
│  Renderer, per window (sandboxed, context-isolated)                     │
│  ├─ Svelte UI, kernel, extensions, edit pipeline, history (TS, seconds) │
│  ├─ pm-wasm: canonical mirror, evaluator, text layout, edit kernels     │
│  └─ Viewer: WebGPU canvas drawing engine frames + DOM/2D overlays       │
│        ▲ MessagePort: patches (from pm-wasm), control, events           │
│        │                                       ▲ VideoFrame             │
│  Main process                                  │                        │
│  ├─ windows, menus, files, save, agent bridge, media store (TS)         │
│  └─ pm-bridge (napi): Mach rendezvous, surface pool import ─┘           │
│        ▲ Mach port rights (pool surfaces, once)                         │
│        │ per-frame descriptors {pool index, seq, pts, revision}         │
│  Interactive engine (utilityProcess hosting pm-engine via napi-rs)      │
│  ├─ sessions (one per open project tab)                                 │
│  ├─ pm-eval, pm-render (wgpu), pm-color, pm-text, pm-vector             │
│  ├─ pm-media (VideoToolbox, FFmpeg, image readers), pm-cache            │
│  └─ pm-audio (device clock, mixer, stretch)                             │
│  Job engine (second utilityProcess, same library)                       │
│  └─ exports, proxy and optimized-media jobs, background renders         │
└─────────────────────────────────────────────────────────────────────────┘

Same crates, other hosts:
  pm-cli    headless render, export, parity runner (no Electron)
  pm-wasm   renderer-side evaluator; also the headless document host
```

Key properties:

1. **No engine work on a UI thread.** A slow effect cannot stall typing.
2. **Synchronous answers stay synchronous.** About 130 built-in call sites ask
   for evaluated values, geometry, bounds, picks, or text layout synchronously.
   `pm-wasm` (the same Rust evaluator compiled to WebAssembly) answers them from
   a mirror of the document, after flushing pending writes (section 6).
3. **Frames stay on the GPU.** The engine renders into a pool of shared
   surfaces. Main imports them once through Electron's `sharedTexture` API and
   forwards a `VideoFrame` per frame. No pixel copies on the preview path
   (section 18).
4. **One renderer.** Preview, export, thumbnails, snapshots, and the CLI call
   `pm-render` with the same inputs.
5. **Two engine processes.** The interactive engine serves windows; the job
   engine runs exports and generation. A crash in one does not take down the
   other. They share disk caches and generated media.
6. **Transport-agnostic protocol.** The engine is a library with a typed message
   protocol that works over MessagePorts, in-process (CLI), or a socket (serve,
   later).

### Crate layout

A Cargo workspace at `engine/` in the repository root.

| Crate | Responsibility | Targets |
|---|---|---|
| `pm-time` | Ticks, rationals, frame math (i128 intermediates), timecode | native, wasm |
| `pm-model` | Canonical document model, `v1_ingest` (seconds to ticks), id-keyed patch reconciliation, validation | native, wasm |
| `pm-eval` | Keyframes, temporal ease, expressions, parenting, groups, 2.5D math, retiming, text animator state, path and mask geometry, pick and bounds | native, wasm |
| `pm-edit` | Geometry-dependent edit kernels that return patches (pose preservation, group and ungroup, align, anchor moves) | native, wasm |
| `pm-text` | Font registry, shaping, layout, outlines, caret geometry, metrics subsets | native, wasm (layout only) |
| `pm-vector` | Paths, trim, repeat, strokes, booleans, squircle corners, coverage | native, wasm (geometry only) |
| `pm-color` | Transfer functions, working spaces, OCIO bridge, LUTs, tone mapping, gamut mapping | native |
| `pm-gpu` | wgpu device, pools, shader compilation, GLSL compatibility, interop trait (decode in, bridge out, encode) | native |
| `pm-render` | Render graph, layer renderers, effects, compositing, motion blur, scopes | native |
| `pm-media` | Probe, demux, decode backends, index, image readers, color metadata | native |
| `pm-cache` | Frame cache, render cache, disk caches, budgets | native |
| `pm-proxy` | Representation policy, segment jobs, generated-media store | native |
| `pm-audio` | Device output and clock, mixer, automation, stretch, waveforms, meters | native |
| `pm-export` | Encoders, muxers, render queue, verification | native |
| `pm-engine` | Sessions, scheduler, playback, budgets, crash attribution | native |
| `pm-protocol` | Messages, versioning, MessagePack codec, TypeScript types via `ts-rs` | native, wasm |
| `pm-node` | napi-rs host for engine processes | native |
| `pm-bridge` | napi-rs addon in main: Mach rendezvous, surface import | native |
| `pm-wasm` | wasm-bindgen exports for the renderer and the headless document host | wasm |
| `pm-cli` | Headless binary | native |

## 5. Time

### Inside the engine

- **Ticks.** `i64` ticks of 1/705,600,000 second (the flick).
  705,600,000 = 2⁹·3²·5⁵·7². Every supported frame rate (23.976, 24, 25,
  29.97, 30, 47.952, 48, 50, 59.94, 60, 90, 100, 119.88, 120, 239.76, 240) and
  sample rate (8 kHz to 192 kHz including 44.1, 88.2, 176.4) divides it
  exactly. The range is about 414 years.
- **Rates** are `Rational { num, den }`.
- **Frame math** uses i128 intermediates:
  `frame_start(n) = n × den × TICKS / num`,
  `frame_of(t) = floor(t × num / (den × TICKS))`. A property test checks
  `frame_of(frame_start(n)) == n` for every supported rate up to 10⁸ frames.
- **Media timebases** that do not divide the flick (MP4 timescale 12288 or
  15360, microseconds, nanoseconds) are converted with i128 and
  round-half-even. The media index keeps native PTS; conversion happens at the
  edge.
- **Timecode** exists only at the UI and IO edges (`pm-time::Timecode`), with
  SMPTE drop-frame for 29.97 and 59.94.

### At the document boundary

The document JSON and the public API keep float seconds (section 6).
`pm-model::v1_ingest` converts at one place:

1. A time is first tested against its composition's frame grid. If
   `|t × fps − round(t × fps)| < 1e-4`, it converts by frame index:
   `frame_start(round(t × fps))` under the exact rational. This keeps keys and
   in-points that sat on frame n at frame n even when the stored rate is the
   float 29.97 rather than 30000/1001.
2. Other times convert to the nearest tick.
3. Frame rates read `fpsRational` (a new optional `[num, den]` field) when
   present; otherwise known NTSC floats map to their exact rationals, and other
   values map to the smallest rational that reproduces the float within 1e-9.
4. Writing back to seconds is `ticks / TICKS` as f64.

### Rules

1. One validator for composition frame rates. Today the UI and
   `set_composition` round to integers and load and validation keep fractions;
   after Phase 1 every path accepts and stores rationals (`fps` plus
   `fpsRational`).
2. Keyframes created by the UI land on frame boundaries in composition time.
3. Nested compositions sample their children at the parent time by default.
   "Keep frame rate when nested" quantizes the child to its own rate, phased
   from the precomp's start.
4. Variable speed integrates in ticks at the owning composition's rate (today
   it uses twice the open composition's rate, at least 60 Hz).
5. Export frame rate can differ from composition rate; the exporter evaluates at
   the exact tick of each export frame.
6. Source frame selection between frames: the last frame with PTS at or before
   the source time ("floor"), as today. Frame blending and optical flow for
   retiming and rate conversion are added after cutover.
7. Motion blur uses a shutter angle and phase. The current default (0.5 frame,
   centered) maps to 180° at −90° phase.
8. Expressions see `time`, `t`, and `fps` as floats as today, plus a new integer
   `frame`. `time` is nudged by at most one tick so that
   `floor(time × fps) == frame` at every rate.

## 6. The document: ownership, writes, and mirrors

### Authority

The renderer stays the authority for the document and Undo. The document JSON,
`core/types/project.ts` (shipped to extensions and agents in the api-pack), the
kernel API, and `EditCommand`s keep **float seconds** and their current shape.
This keeps the frozen contracts intact and limits the rewrite to the engine.

Additive fields only, before cutover:

- `fpsRational` on compositions.
- Asset representations, linked-media references, source color metadata,
  interpretation overrides (sections 9, 14, 21).
- Project color and media settings.
- `minReaderVersion` at the document root (section 21).

Removing the "open composition at the root" rotation and storing ticks on disk
are deferred to the `pm-doc` work after cutover.

### The patch stream

1. The renderer produces leaf patches (`src/shared/patch.ts`) in **canonical**
   form (no root rotation), using `PM.Comps.canonicalize` for every patch, not
   only history entries.
2. `pm-wasm` ingests them through `pm-model::v1_ingest` and applies them with
   **id-keyed reconciliation**: when a patch replaces an array (today
   `diffPatches` replaces the whole array on any length or order change), items
   are matched by `id`, so unchanged layers keep their revisions and caches.
3. `pm-wasm` emits the resulting **canonical binary patches** to the engine
   session. There is one translation point, and both mirrors hash identically
   per revision.

### Making every write visible

Today's writers include about 19 direct `PM.hist.do` sites and 18
`api.history.do` calls, the edit engine, live gestures, `anim.setKey`, asset
metadata updates (`raster.ts:970,990,1026,1098,1277,1314`), in-place path and
mask edits followed by `anim.touch()` (`viewer/path-editing.ts:75,80`,
`inspector/MasksSection.svelte:26`, `inspector/retiming.ts:47`,
`timeline/graph-controls.ts:32`), `shaderHooks.syncShaderUniforms`
(`inspector/shader.ts:39-51`), extension-layer data refresh
(`kernel/install.ts:789-798`), the render queue's project swap
(`render-queue.ts:10-12`), and `api.host.pm` in user extensions.

The plan:

1. **Write barrier.** Every write records its dirty path. In development and
   test builds, `PM.proj` is wrapped in a Proxy that records paths and throws on
   writes outside a known writer. In production, the inventoried sites call
   `doc.write(path, value)` or `doc.touched(path)`.
2. **Flush before query.** Every `pm-wasm` query first drains dirty paths into
   patches. Reads that follow writes inside one transaction (for example
   `parenting.ts` pose preservation, `layer-groups.ts` group and ungroup) see
   the new state.
3. **Edit kernels.** Geometry-dependent edit operations move into `pm-edit` in
   WASM: pose preservation on reparent, group and ungroup, align and distribute,
   anchor moves with font anchors, composition bounds. They return patches that
   the TS edit pipeline applies, so Undo is unchanged.
4. **Shadow-replay auditor.** In test builds, the exact patch stream sent to
   `pm-wasm` is also applied to a JS shadow copy with the existing
   `applyPatch`. At frame boundaries, `JSON.stringify(shadow)` must equal
   `JSON.stringify(PM.proj)`. A mismatch fails the test with the last writer's
   stack from the write barrier. The full e2e suite runs with the auditor on.
5. **Dual-answer mode.** During Phase 1, a test mode answers every synchronous
   query with both the TS evaluator and `pm-wasm` and compares results. Zero
   mismatches is a gate.
6. **Resync.** A revision gap in production triggers a full canonical snapshot.
   This is a safety net, not a path.

Prior art to reuse: `renderer/src/host/remote-sync.ts` already catches
non-history writes by diffing, and `server/sessions.ts` keeps a patch-fed
authoritative mirror through `shared/patch.ts`.

### Later: moving the authority

After cutover, the edit pipeline and history can move to Rust (`pm-doc`), with
ticks on disk and uniform composition storage. That is when schema v2 and an
`apiVersion` bump with tick-based types happen. Not on the critical path.

## 7. Evaluation (`pm-eval`)

`pm-eval` is a pure function of (document, composition, time) to evaluated
state. It replaces `anim.ts`, `temporal-bridge.ts`, `temporal-ease.ts`,
`easing.ts`, `expression.ts`, `retiming.ts`, the evaluation parts of
`parenting.ts`, `layer-groups.ts`, `space-3d.ts`, `content-properties.ts`, text
animator state, and path and mask geometry.

- **Pure and explicit.** No global current composition, time, or `PM.scope`
  stack. Scope is an argument.
- **Incremental.** Caches keyed by node id, the revision of that node's inputs,
  and tick. Patches invalidate the node and its dependents (parents, children,
  matte users, expression readers).
- **Expression dependencies.** The interpreter records which fields each
  expression reads, so dependency tracking is exact even though expressions can
  read raw project fields.
- **Temporal model.** The speed and influence model, auto-bezier, continuous,
  hold, stored spring parameters, and legacy easing presets, ported exactly.
  Spatial bezier motion paths are added after cutover.
- **JavaScript semantics where projects depend on them:** loose `==`, string
  coercion in `+`, `Math.round` rounding half toward +∞, `%` on negatives, the
  `sin`-based `random` and `wiggle`, the float32 spring table, hex color byte
  interpolation, and the existing epsilons. Each has a test against V8.
- **Determinism across targets.** All transcendental functions use the `libm`
  crate on every target, so native and WASM evaluation are bit-identical. A
  cross-target test compares them on the whole corpus.
- **Geometry queries.** `pick`, `bounds`, `worldMatrix`, `localMatrix`, group
  and composition bounds, computed from the same geometry the renderer uses,
  including text bounds from `pm-text`.
- **Fast path from JavaScript.** Hot queries (`evP`, `ev`, `worldMatrix`) use
  numeric ids interned per property and write results into a shared
  `Float64Array` scratch buffer. No strings or objects cross the boundary per
  call. Views are re-acquired after WASM memory grows.

## 8. GPU and the render graph (`pm-render`)

### Device

- wgpu (currently 30.x), Metal on macOS, DX12 on Windows, Vulkan for Linux CLI
  renders. Pinned; upgrades are deliberate lanes, budgeted for wgpu's roughly
  quarterly major releases.
- wgpu-hal escape hatches for decoder output import, shared-surface export, and
  encoder input, all behind one `pm-gpu` interop trait.
- The engine's adapter must match Chromium's GPU process adapter (by Metal
  `registryID`, or LUID on Windows) so shared surfaces stay on one GPU.
- **Metal math mode.** wgpu leaves Metal fast math on. Color-critical shaders
  (input and output transforms, OCIO, LUTs) are compiled as passthrough shaders
  with precise math, or wgpu is patched. Decided in Phase 0.
- **Device loss and runaway shaders.** User and Store shaders run on a shared
  device. A GPU watchdog cancels frames that exceed a time limit, disables the
  offending effect for the session, and reports it. Device loss recreates the
  device and resumes without replacing the document.

### Working encoding

Intermediates are `rgba16float` by default and `rgba32float` in 32-bit mode
(requires `FLOAT32_FILTERABLE` and blendable float32 targets or shader-side
blending; verified on the reference machines in Phase 0). The intermediate
**encoding** depends on the color mode (section 9):

| Mode | Intermediate encoding |
|---|---|
| Standard | Premultiplied, display-encoded (sRGB curve), working primaries |
| Linear | Premultiplied, linear light, working primaries |
| ACES | Premultiplied, linear ACEScg |

Blending, mipmap generation, resampling, and motion-blur accumulation all use
the mode's encoding.

Source conversion shaders compute in f32 regardless of storage precision. Exports
default to 32-bit float when the output or any source is above 10 bits; preview
uses the project precision. A precision mismatch between preview and export is
reported in the export dialog as text, not hidden.

### Render graph

Each frame is a graph compiled from the evaluated state:

- **Nodes:** source (decoded frame, vector, text, solid, shader output),
  transform and resample, mask, effect pass, matte, blend, transition,
  motion-blur accumulate, precomp, adjustment, color transform, output.
- **Content keys.** A node's key is the hash of its evaluated inputs, the source
  frame identity (asset, representation, source frame), the color transforms in
  effect, the resolution bucket, and the precision. Time enters only through
  evaluated values and `u_time`, so a static layer is rendered once for a whole
  range.
- **Cache granularity.** Layer and precomp outputs are cached on their full
  extents at quantized scales. Region-of-interest cropping applies to the final
  output only.
- **Region of interest.** Each node knows its domain of definition and the
  region downstream needs, including effect padding (the existing
  `viewportPadding` contract). Effects that expand bounds (glow, drop shadow)
  grow their buffers up to a limit instead of clipping.
- **Transient resources** come from an aliasing allocator per frame.
- **Isolation.** A failing node marks its layer failed and outputs transparent
  pixels; the frame still renders.

### Sampling and antialiasing

- Footage and raster sources: mipmapped trilinear sampling when downscaling, and
  a selectable high-quality resampler (bicubic, Lanczos with clamped negative
  lobes) for export and paused frames. Mipmaps are generated in the mode's
  encoding.
- Quad edges: analytic coverage antialiasing (today: a hard UV cut with
  `antialias: false`).
- Vector shapes and text: rendered at output resolution, never as scaled
  bitmaps.

### Motion blur

- Temporal supersampling with one project setting for sample count, used by
  every path. Today it is 1, 6, 10, 12, 16, or 20 depending on the code path
  (appendix A).
- Accumulation in the mode's encoding at float precision.
- No adaptive sample reduction unless it is proven output-identical, because it
  would break determinism and pixel-exact optimization rules.

### 2.5D

Parity first: the 3D switch, Z position, anchor and scale, X and Y rotation,
orientation, perspective lens per 3D group, homography planes, coplanar
ordering, painter sort. After cutover: depth-buffered intersections, then
camera and light layers.

### Scopes

Waveform, RGB parade, vectorscope, and histogram computed by compute shaders on
the presented frame, before or after the display transform, in code values,
IRE, or nits for HDR. Delivered to the UI through the frame bridge as small
textures. Shipped after cutover with the color work.

## 9. Color pipeline

### Model

```
source pixels (native depth)
  → input transform: range, matrix, chroma, transfer, primaries → working space
  → premultiply (in the working encoding)
  → working space: compositing, effects, motion blur
  → output: unpremultiply → output transform → re-premultiply if requested
  → display: working → reference display encoding → viewer canvas
```

### Color invariants

These hold in Standard and Linear modes and are tested (section 2):

1. Rec.709 video is treated as display-referred by default. Input linearizes
   with the inverse of the BT.1886 display EOTF (gamma 2.4); Rec.709 output uses
   the same curve. A pass-through is therefore exact.
2. Colors picked in sRGB linearize with the sRGB curve, and sRGB output uses the
   same curve.
3. The scene-referred inverse BT.709 OETF is available only as an explicit
   Interpret choice.
4. The viewer shows Rec.709 the way AVFoundation does. QuickTime and Chrome
   disagree on Rec.709 display gamma; export offers two named outputs,
   "Rec.709 (broadcast)" with tags 1-1-1 and "Rec.709 (QuickTime and Safari)"
   using Apple's convention. Final copy is verified against Apple and Adobe
   behavior before shipping.

### Alpha

- Straight sources: input transform, then premultiply.
- Sources premultiplied in encoded space: unpremultiply (guarded for
  α < 1/4096 at half float), input transform, re-premultiply.
- "Premultiplied with color" interpretation (matted against white or black), as
  in After Effects.
- Color-domain effects (Levels, Curves, LUT, Hue/Saturation, legacy GLSL in
  display or log domains) run on unpremultiplied values. Additive pixels
  (α = 0, RGB > 0, such as glows) are never unpremultiplied.
- Output: unpremultiply, output transform, then write straight, or
  re-premultiply the encoded values for premultiplied outputs (the convention of
  ProRes, HEVC with alpha, and PNG consumers).

### Project color settings

Settings › Project › Color, built from the existing Settings rows, selects, and
switches. Settings are named by outcome:

| Setting | Options | Default |
|---|---|---|
| Color | Standard (sRGB and Rec.709), Wide gamut (Display P3), HDR (Rec.2020 PQ), HDR (Rec.2020 HLG), ACES | Standard |
| Blend in linear light | Switch | Off (decision 1, section 29) |
| Advanced › Working space | Linear Rec.709, Linear Display P3, Linear Rec.2020, ACEScg | Follows Color |
| Advanced › Precision | 16-bit float, 32-bit float | 16-bit float |
| Advanced › Graphics white in HDR | 203 nits (BT.2408), custom | 203 nits |

- Working space is hidden while blending is not linear.
- Projects migrated from today open as Standard with linear blending off and
  reproduce today's look.
- Changing color settings changes the look; each change is one Undo step.
- Display choice (Automatic, sRGB, Display P3, Rec.709, HDR) is a machine
  setting in Settings › General › Display, not a project setting.

### Per-source interpretation

"Interpret Footage…" on any asset, and on a multi-selection. Filled
automatically from container and stream metadata, and each value shows its
origin, for example "Automatic (Rec.709) · From file" or "Assumed · No color
tags in file".

- Color: primaries, transfer, matrix, range; camera log curves with their gamuts
  (S-Log3/S-Gamut3 and .Cine, LogC3/AWG3, LogC4/AWG4, V-Log/V-Gamut,
  C-Log2/C-Log3/Cinema Gamut, Log3G10/RWG, BMD Film Gen5, DaVinci
  Intermediate/DWG, F-Log/F-Log2, N-Log, D-Log/D-Log M, Apple Log); "Log
  footage: Convert to Rec.709 / Show as log".
- HDR sources: in SDR projects, tone mapped by default ("HDR (HLG) · Tone mapped
  to SDR") using BT.2446 or the OCIO SDR view; option "Show as HDR values".
  Dolby Vision files use the base layer; the RPU is ignored.
- Alpha: none, straight, premultiplied, premultiplied with color; invert.
- Frame rate override, field order, pixel aspect ratio.
- Input LUT.
- Camera rules: remember an interpretation for a camera model.
- Untagged sources: SD (480/576 lines) assumes BT.601 with SMPTE-C or BT.470BG
  primaries; HD and above assume BT.709. Stills use their ICC profile through
  ColorSync (or lcms2 off macOS).

Superwhites and sub-blacks are preserved: decode is unclamped, clamping happens
only at output.

### Working space and gamut

- Negative components (wide-gamut sources in a Rec.709 working space) are
  allowed in intermediates. Blend modes and color effects that assume [0, 1]
  have HDR-safe formulas or clamp explicitly (below).
- Output gamut mapping: clip, or compress (ACES reference gamut compression),
  selectable in export presets.

### Blend modes in Linear and HDR

Each blend mode declares its domain. Normal, add, multiply, lighten, and darken
run in the working encoding. Screen, overlay, and soft light use HDR-safe
formulas in linear modes, or run in the reference display encoding when the
After Effects-style "Blend colors using 1.0 gamma" is off. Luma mattes use
Rec.709 coefficients on the reference display encoding by default.

### ACES mode

- OpenColorIO 2.6 (or the version current at Phase 0) through a thin `cxx` FFI
  bridge, using its built-in ACES 2.0 configurations.
- Graphics (solids, shapes, text, sRGB stills) default to the "Graphics" input,
  the inverse of the output transform, so picked colors display as picked.
- OCIO GPU shaders are generated as Metal Shading Language directly where OCIO
  supports it, otherwise through the shader pipeline in section 11, and checked
  against OCIO's CPU path in the conformance suite. 3D LUTs use tetrahedral
  interpolation.

### LUTs and color effects (after cutover)

- Formats: `.cube` (1D, 3D), `.3dl`, `.csp`, CLF/CTF.
- Effects with a selectable domain (working, log as ACEScct, display): Apply
  LUT, Exposure, Curves, Lift/Gamma/Gain, Hue vs Hue and Saturation, White
  Balance, Color Space Transform, Tone Map.
- Gradients interpolate in Display (today), Linear, or Perceptual (OKLab).
- Broadcast-safe legalizer, gamut warning, false color, zebras.

### HDR

- **Preview.** The viewer's WebGPU canvas is configured `rgba16float` with
  `toneMapping: { mode: 'extended' }` in `srgb` or `display-p3`. The engine
  outputs extended sRGB- or P3-encoded values (not linear). EDR headroom comes
  from native screen APIs through main and changes with brightness. Prior art:
  the onboarding HDR path (`renderer/public/onboarding/hdr-output.js:93-132`).
- **Reference viewer** (after cutover): a native `CAMetalLayer` window with
  extended dynamic range for a second display.
- **Export.** PQ (HDR10) requires mastering display metadata and MaxCLL/MaxFALL,
  measured during export. HLG metadata is optional. PQ 1.0 maps to 203 nits by
  default.
- **Stills.** HEIC and JPEG gain-map HDR images (ISO 21496-1) are read with their
  gain maps.

### Output tags

| Output | CICP (primaries-transfer-matrix) | Range |
|---|---|---|
| sRGB images | 1-13-0 or ICC | Full |
| Rec.709 (broadcast) | 1-1-1 | Limited (video), full (images) |
| Rec.709 (QuickTime and Safari) | 1-2-1 (transfer unspecified, Apple's convention) | Limited |
| Display P3 video | 12-13-1 (documented choice) | Limited |
| Rec.2020 SDR | 9-1-9 (or 9-14-9 for 10-bit) | Limited |
| PQ | 9-16-9 | Limited |
| HLG | 9-18-9 | Limited |

ProRes Y′CbCr is limited range with full-range alpha. MOV and MP4 write `colr`
`nclx` when full range must be signaled; WebM writes Colour elements; PNG writes
`cICP` or ICC.

## 10. Layer types

| Type | Today | Engine |
|---|---|---|
| `solid` | Uniform quad; `d.radius` stored but ignored (`compositor.ts:851-853`) | Same. Honoring `radius` is a known change, applied only to layers created after cutover |
| `shape` primitives and paths | Canvas2D raster at quantized scale | `pm-vector` geometry rendered at output resolution with analytic coverage (renderer chosen in spike S4) |
| `text` | Canvas2D `fillText`, browser shaping | `pm-text` (section 12) |
| `captions` | Canvas2D boxes and text (`captions/render.ts`) | `pm-text` plus `pm-vector` boxes; word states evaluated in `pm-eval` |
| `image` | `ImageBitmap`, RGBA8 | Native depth readers (section 13) |
| `image` (SVG) | `<img>` rasterized per size bucket | usvg to vectors, rendered at any scale; SVG filters through resvg on CPU |
| `video` | `<video>` or VP9 copy | `pm-media` decode of the chosen representation (section 14) |
| `shader` | User GLSL, FBO capped at 4,096 px | GLSL compatibility pipeline (section 11); device limits only |
| `extension` fragment | Kernel-assembled GLSL | Same pipeline |
| `extension` mesh | Fixed WebGL shading for OBJ | Same look in parity; physically based shading after cutover |
| `precomp` | Recursive render, depth 6 (frame preparation allows 8) | Subgraph cached by content key; one depth limit and cycle check in `pm-eval` |
| `adjustment` | Effects on the accumulator | Same |
| `group` | Transform space or compositing boundary | Same rules |
| `null`, `audio` | Not visual | Not visual |

Animated GIF, APNG, animated WebP and AVIF, and image sequences are decoded
directly with their original timing and depth; no VP9 transcode.

## 11. Effects, transitions, and the shader contract

### Built-in effects

The 16 effects, 4 transitions, 5 shader presets, and 2 extension layers are
re-implemented in WGSL:

- Gaussian Blur with a correct kernel at every radius and a downsample pyramid
  for large radii (today taps are spaced `r/N` apart and skip texels).
- Glow and Drop Shadow use the same blur.
- Each declares `colorDomain` (`working`, `display`, `log`) and bounds expansion.
- In Standard mode they reproduce today's output within parity tolerances,
  except where appendix A lists a defect being fixed.

### The GLSL contract stays

Projects, Store extensions, agent-written extensions, and web exports carry
GLSL ES 3.00 with Powermove's preambles (`PM.GLSL_PRE`, `TRANSITION_PRE`,
`SHADER_HEADER`), `u_<param>` uniforms, and the `u_p<i>` and `u_c<i>` aliases.

Pipeline:

1. **Wrap.** The existing generators (`kernel/glsl.ts`, `extension-layers.ts`)
   produce the full GLSL ES 3.00 program.
2. **Normalize.** Rewrite `#version 300 es` to `#version 450` (glslang refuses ES
   below 310 for SPIR-V); remove precision qualifiers; inject a
   `pm_FragCoord` built from a resolution uniform and replace `gl_FragCoord`
   (origin contract below).
3. **Compile with glslang** using relaxed Vulkan rules (loose uniforms collected
   into one uniform block) and automatic binding and location mapping.
4. **Split combined samplers** (including `sampler2D` function parameters) at the
   SPIR-V level, because naga's SPIR-V frontend maps `OpTypeSampledImage` to the
   image only.
5. **Translate.** Parse SPIR-V with naga ourselves and validate with
   derivative-uniformity checks off (`ShaderSource::Naga`), because typical
   Shadertoy-style raymarchers call `texture()` inside data-dependent control
   flow, which WebGL accepts and wgpu's default validation rejects. Spike S3
   compares this path against SPIRV-Cross to MSL through wgpu passthrough
   shaders and picks the more robust one.
6. **Cache** by source hash, backend, and driver version on disk.
7. **Errors** are mapped back to the user's line numbers, as
   `gl.compileError` does today.

**Origin and sampling contract.** GL's origin is bottom-left; Metal and wgpu
render targets are top-left. Legacy shaders see GL conventions: `v_uv`,
`v_st`, `pm_FragCoord`, and `dFdy` behave as in WebGL. The sampler state
(filtering, wrap) used by the WebGL host per effect type is recorded and
reproduced. S3 includes flip and derivative test cases.

**Color domain.** Legacy GLSL is display-referred: the engine converts to a
fixed reference display encoding (sRGB curve, working primaries, values above
1.0 optionally passed through), runs the shader, and converts back.
"Reference" means independent of the user's actual display, so output is the
same on every screen and in export.

### Contract v2

A WGSL contract for new extensions (apiVersion 4): explicit `colorDomain`,
bounds expansion, multiple inputs, compute passes. GLSL remains supported
indefinitely.

## 12. Text

- **Stack.** Parley for layout (bidi, line breaking, variable fonts, color
  emoji, fallback), HarfRust for shaping, Skrifa for outlines and metrics.
  RustSec advisories on `rustybuzz` and `ttf-parser` rule those out. Glyph
  rendering is decided in spike S4 (section 8 vector renderer).
- **Font registry.** The engine is the single font authority: it enumerates
  system fonts natively (CoreText, DirectWrite), loads project-embedded fonts,
  and resolves fallback. It ships a fallback manifest to `pm-wasm`.
- **Fonts in `pm-wasm`.** The sandboxed renderer cannot read font files, and full
  fonts are large (Apple Color Emoji alone is hundreds of megabytes of bitmaps).
  The engine produces metrics-and-shaping subsets per font (`cmap`, `hmtx`,
  `hhea`, `OS/2`, `head`, `GSUB`, `GPOS`, `GDEF`, `fvar`, `avar`, `HVAR`,
  `name`; no outlines or bitmaps). Main serves them to `pm-wasm` lazily per
  family under a per-window memory budget.
- **Layout compatibility.** Today: greedy word-then-character wrapping when
  `paragraph` is set and `boxWidth > 0` (`text-layout.ts:22`), character splits
  by UTF-16 unit, baseline at `size × 0.82` (`raster.ts:210,260,339`), line
  height `size × leading` (default 1.15), kerning through measured widths.
  Compatibility mode reproduces these for existing layers so line breaks and
  positions stay. New layers use grapheme-aware breaking and font metrics.
- **Animators.** Stagger, range, and wave on characters, words, lines; per-glyph
  opacity, position, scale, rotation, skew, tracking, blur, color. The 13
  inspector presets (including Typewriter) stay UI-side.
- **Editing.** The canvas text session keeps its UX. Caret, selection, and the
  font offset it reads today (`viewer/text-editor.ts:102`) come from `pm-wasm`.
- **Quality.** Canvas2D on macOS draws glyphs through CoreText with dilation;
  plain path coverage looks thinner. The renderer applies a coverage gamma or
  stem darkening tuned to match CoreText at text sizes.
- **Gate.** A side-by-side corpus against CoreText at 9 to 400 px, light and dark,
  must pass Jude's review before text cutover, plus SSIM ≥ 0.98 against today's
  output for parity cases.
- **Typing latency** in the canvas text editor gets its own gate (section 25).

## 13. Media I/O (`pm-media`)

### Principles

1. **The original is the truth.** Import never replaces the original. Everything
   else is a representation derived from it.
2. **Exact frames.** Every source is indexed for exact, fast random access.
3. **Native depth and color.** Frames reach the GPU at native depth with color
   metadata attached.

### Backends

- **VideoToolbox** through objc2 crates for ProRes, H.264, HEVC, and AV1 where
  hardware exists. Requested output formats keep native depth:
  - ProRes 422 → `x422` or `v210`
  - ProRes 4444 and XQ → `y416` (12-bit color, 16-bit alpha)
  - HEVC and H.264 10-bit 4:2:0 → `x420`; 4:2:2 → `x422`
  - 8-bit → biplanar 8-bit
- **Metal import.** wgpu's NV12 and P010 formats are not available on Metal, so
  planes are imported as separate `R8`/`RG8` or `R16Unorm`/`RG16Unorm`
  textures from `CVMetalTextureCache`, with Metal-compatible pixel buffers
  requested from the decoder. MSB-aligned 10-bit samples are normalized
  (`v >> 6`), then limited-range scaled: Y′ = (c − 64)/876, C = (c − 512)/896.
  Untagged 4:2:0 assumes left (type 0) chroma siting.
- **FFmpeg** (our own build, section 24) through `ffmpeg-the-third` or our own
  bindgen sys crate, linked as dylibs: demuxing, DNxHD/DNxHR, CineForm, MJPEG,
  DV, VP8 and VP9 (libvpx for alpha), AV1 (dav1d), DPX, software fallbacks, and
  all audio decoding.
- **Images.** ImageIO on macOS (HEIF/HEIC, AVIF, TIFF, gain maps, ICC); the
  `image` crate for common formats; OpenEXR through its C library (reads all
  compressions, writes DWAA); usvg for SVG.

### Formats

| Category | Formats |
|---|---|
| Professional intermediates | ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ |
| Camera and delivery | H.264 (including 4:2:2 10-bit, which has no hardware decode), HEVC 8 and 10-bit (4:2:0 and 4:2:2), HEVC with alpha, AV1 |
| Avid and others | DNxHD, DNxHR, CineForm, MJPEG, DV |
| Web | VP8, VP9 (with alpha), AV1 |
| Containers | MOV, MP4, M4V, MXF (OP1a, OP-Atom), MKV, WebM, AVI, MTS/M2TS, TS, MPEG-PS, and the rest of today's list |
| Images | PNG 8/16, JPEG, TIFF 8/16/32f, OpenEXR, DPX 10/12/16, HEIF/HEIC, AVIF, WebP, BMP, GIF, SVG |
| Sequences | Any image format, with gap reporting |
| Animated images | GIF, APNG, animated WebP and AVIF at original timing |
| Audio | WAV, AIFF, CAF, MP3, AAC/M4A, FLAC, Opus, Vorbis, and audio in every container; mono, stereo, 5.1, multichannel, MXF OP-Atom mono files |
| After cutover | Camera RAW (BRAW, R3D, ARRIRAW, Canon RAW, ProRes RAW, cDNG), PSD layers, multi-part EXR |

Hardware notes that drive the proxy policy: the base M1 has no ProRes engine
(M1 Pro, Max, Ultra and M2 and later do); AV1 hardware decode starts at M3;
H.264 4:2:2 10-bit is software-only everywhere.

### Probe and index

On import, in the background:

1. Container and streams: codec, profile, size, pixel format and depth, frame
   rate and whether it is variable, duration, timecode, reel, rotation, pixel
   aspect, field order, audio layout.
2. Color: primaries, transfer, matrix, range, mastering metadata, ICC, log hints,
   HDR format.
3. **Frame index** from the container's own tables where they exist (MOV/MP4
   `stss` and `stts`, MXF index tables, MKV cues), scanning only containers
   without them (MPEG-TS). Open-GOP CRA and RASL frames are marked as not cleanly
   seekable.
4. **Timing:** MOV and MP4 edit lists, composition offsets and negative CTS,
   stream start offsets, and AAC and MP3 encoder priming are honored.
5. **Decode cost** on this machine (short benchmark), used by the proxy policy.
6. Poster, filmstrip seed, and waveform peaks.

### Variable frame rate and interlacing

- VFR sources are sampled by timestamp: the frame shown at source time t is the
  last frame with PTS ≤ t. Duration and audio sync come from timestamps. A
  nominal rate (the most common frame duration snapped to a standard rate) is
  used only for snapping and stepping. The asset's details say, for example,
  "Variable frame rate · Plays at 60 fps by timestamp". Optimized media for VFR
  sources is written at the nominal rate by PTS.
- Interlaced sources (DV, AVCHD, 1080i MXF) are deinterlaced (bwdif) by default,
  with field order from metadata or Interpret, and pulldown removal for
  telecined material.

### Decode service

- Decode sessions per source and per playback instance, pooled and budgeted.
- Hardware decoder output is imported into wgpu without CPU copies.
- Scheduling follows the playback scheduler: direction, speed, read-ahead
  window, GOP structure.
- Decoded frames live in the frame cache (section 14.7).

## 14. Proxies, optimized media, and caches

### 14.1 What users should feel

Footage always plays, with no setup. Nobody needs to know what a proxy is. A
professional who wants control finds every setting After Effects and Final Cut
Pro offer, with plain language. The viewer always tells the truth about what is
on screen, and exports never contain proxy pixels by accident.

### 14.2 The representation ladder

For each source, the engine chooses from a ladder, cheapest first:

1. **Original** at full resolution.
2. **Reduced-resolution decode** of the original, with no files: ProRes
   partial-resolution decode, RAW decode quality, ImageIO downsampling, EXR
   mips. Tried before anything is generated.
3. **Proxy segments**, reduced resolution.
4. **Optimized segments**, full resolution, intra-frame, for long-GOP sources
   that scrub poorly.
5. **Kept renders**, pinned renders of precomps (section 14.6).

Representations cover video, image sequences (EXR and DPX sequences are the
heaviest media motion designers use), and large stills (100-megapixel TIFF,
EXR, PSD), which get a tiled mip cache instead of a proxy file.

### 14.3 Segments, addressed by content

Generated media is stored as segments of about 2 seconds, aligned to source
keyframes, keyed by (original fingerprint, recipe, segment index):

- Only used ranges plus 2-second handles are generated. Trimming a clip outward
  generates only the new segments.
- Jobs resume after a crash or relaunch.
- Availability is per frame.
- Identical footage used in two projects is generated once.
- Segments are written atomically, so a shared volume used by several Macs is
  safe.

### 14.4 Recipes and fidelity

| Purpose | macOS recipe | Notes |
|---|---|---|
| Proxy | ProRes 422 Proxy (10-bit), ProRes 4444 at proxy size for alpha | HEVC 10-bit for smaller files |
| Optimized | ProRes 422 HQ; ProRes 4444 for sources that are 4:4:4, above 10-bit, or have alpha | Off by default (decision 4) |

Every generated representation:

- keeps the source encoding (log stays log, HDR stays HDR), so changing an
  interpretation never forces a rebuild;
- is 10-bit (8-bit H.264 only for SDR, non-log sources, by explicit choice);
- keeps alpha, PCM audio, timecode, reel name, pixel aspect, frame count, and
  timestamps;
- is sampled in the original's coordinate space, so layer geometry is unchanged;
- is never used for analysis (tracking, stabilization, transcription, agent
  media tools use originals).

Windows recipes use DNxHR or hardware HEVC 10-bit, decided in the Windows phase.

### 14.5 The automatic policy

A **machine profile** measures decode cost per codec class and size, GPU
headroom, disk throughput, and power state (battery, Low Power Mode, thermal
state). The project stores only intent. The same project uses originals on a
Mac Studio and proxies on a MacBook Air.

Selection rules:

1. Choose the best representation that sustains real time at the viewer's size
   in device pixels, given the number of simultaneous streams.
2. During playback, switch representations only at clip boundaries or when
   playback starts, never mid-shot.
3. When paused or when scrubbing stops, show the original at full quality as soon
   as it decodes (gate: within 150 ms on the base machine for 4K ProRes). The
   frame upgrades in place with no fade.
4. Never wait for a proxy. If one is not ready, use the next rung.
5. Re-evaluate when the power state changes.

Generation:

- **Create proxies: When needed** (default) generates proxies only when the
  policy predicts a source cannot play in real time.
- Optimized media is generated only on request by default, because 4K ProRes 422
  is about 210 GB per hour of footage.
- Jobs run in the job engine at low priority, pause during playback and export,
  and go clip-near-playhead first.
- **Disk guard.** Generation pauses when free space falls below the larger of
  20 GB or 10% of the volume. The status item says "Proxies paused · Low disk
  space".

### 14.6 Render cache and kept renders

- The render cache stores node outputs (section 8 keys) in GPU memory and on
  disk.
- **Visible.** A 2-pixel track under the timeline ruler, in a quiet ink tone:
  lighter for memory, stronger for disk, nothing when not cached. Off from View ›
  Show Rendered Frames.
- **Commands.** Render Work Area, Render Selection, Delete Rendered Frames.
- **Keep Rendered** on a precomp pins a render that is invalidated on edit
  (the modern After Effects comp proxy and pre-render).
- **Background rendering** fills the disk cache for heavy sections while idle.
  Setting: When plugged in (default), Always, Never.
- **Export safety.** Cache keys include representation and precision. Export
  reads only entries rendered from originals at export precision; otherwise it
  renders. Phase 4 gate: fill the cache from proxies, then export; the result is
  byte-identical to an export with an empty cache.

### 14.7 Caches

| Cache | Contents | Key | Location | Eviction |
|---|---|---|---|---|
| Frame cache | Decoded source frames, GPU and CPU tiers | Source, representation, frame | Memory | Byte budget, LRU, protected near the playhead |
| Render cache | Node outputs | Section 8 content key | GPU memory, disk | Budgets, LRU |
| Media index | Frame indexes, probe, decode cost | Fingerprint | App cache | Rebuilt on demand |
| Thumbnails, filmstrips, posters, waveforms | Derived images and peaks | Fingerprint, recipe | App cache | Size limit |
| Generated media | Proxy and optimized segments | Fingerprint, recipe, segment | Per the location setting | Storage limit, never pinned items |
| Large-still mips | Tiled mip levels | Fingerprint | App cache | Size limit |
| Shader cache | Compiled pipelines | Source hash, backend, driver | App cache | Version change |
| Font subsets | Metrics subsets for `pm-wasm` | Font file hash | App cache | Size limit |

These replace today's IndexedDB `MediaStore` (media, posters, 1280-pixel
editing copies), the editing-copy queue, the decoded audio cache, raster and
texture caches, and the GL program cache. One budget authority in the engine
owns them; today's `memory.ts` budgets and the `previewMemoryMiB` setting map
to engine budgets. Existing editing copies in IndexedDB are deleted after
migration (section 21).

### 14.8 Controls

**Settings › Project › Media** (intent, travels with the project):

| Setting | Options |
|---|---|
| Create proxies | When needed, For all video, Only when I ask |
| Proxy size | Automatic, Half, Quarter, 1080p, 720p (Automatic never goes below 540p) |
| Proxy format | ProRes 422 Proxy, ProRes 422 LT, HEVC (smaller files) |
| Create optimized media | Only when I ask, When needed, For H.264 and HEVC video |
| When importing | Link to files, Copy into project; switch: Copy from memory cards |

**Settings › General › Media** (this Mac):

| Setting | Options |
|---|---|
| Generated media location | Powermove cache, Next to the project, Choose Folder… |
| Background rendering | When plugged in, Always, Never |
| Storage | Space used by proxies, optimized media, rendered frames, media index, thumbnails and waveforms; each with Clear… and a limit |

**Per asset**, from the right-click menu (NSMenu, per native conventions):

- Create Proxy, Create Optimized Media
- Rebuild Proxy, Delete Proxy and Optimized Media
- Always Play Original (check item)
- Attach Camera Proxy…
- Interpret Footage…, Replace Footage…
- Show in Finder

**Viewer.** One menu, built on the app's popover menu (today it is a native
`<select>`, `viewer/viewer.ts:858`), with headers:

| Header | Items |
|---|---|
| Resolution | Automatic, Full, Half, Quarter |
| Media | Automatic, Originals, Proxies |
| (none) | Play Every Frame (check item) |

The button label states what is on screen when it is degraded, as quiet text:
"Half · Proxy". It shows "Full" once a paused frame upgrades. "Proxies" works
like Final Cut Pro's Proxy Only: clips without a proxy show as offline, so a
travel package can be checked before leaving.

A keymap command toggles Originals and Proxies.

### 14.9 Status, diagnosis, storage

- One global status item while generating: "Creating proxies · 3 of 12". It does
  not change during playback. Lists stay quiet text: no badges, glyphs, or
  counts in rows.
- **Diagnosis.** After playback that dropped frames, the status item says, for
  example, "Dropped 14 frames · 2 clips decode slowly", with the action Create
  Proxies. When the cause is GPU load, it says that instead. No modal.
- **Clear…** in Storage asks for confirmation through the app's confirm dialog:
  "Clear proxies?" with "This frees 48.2 GB. Powermove creates them again when
  needed. Proxies for 3 clips whose originals are offline are kept."
- Clear and LRU eviction never remove pinned media: proxies whose originals are
  offline, proxies in a travel package, attached camera proxies, kept renders.
- Errors say what failed and what to do: "Couldn't create a proxy for
  A001_C003.mov. The file ends early at 01:12:04. Copy the complete file, then
  choose Rebuild Proxy."

### 14.10 Camera proxies

Known card layouts (Sony Sub, Canon, ARRI, RED, Blackmagic proxy folders) are
detected on import. Proxies are matched by UMID or QuickTime content identifier,
or by timecode plus duration, never by name alone. They get their own
interpretation (camera proxies often have a Rec.709 LUT baked in while the
original is log, and may differ in aspect ratio). They are pinned.

### 14.11 Export and offline media

- Interactive export with offline originals:
  - Title: "2 clips are offline"
  - Body: "Their original files can't be found. Locate them to export at full
    quality, or export these clips from their proxies at lower resolution."
  - Actions: Cancel, Export with Proxies, Locate Media…
- Queued jobs never fall back to proxies. A job with offline media waits with
  "Waiting for media · 2 clips offline".
- A draft render is an explicit queue option, "Render using: Proxies (draft)",
  with an optional "Draft" mark in the corner. The verification report says
  "Rendered 2 clips from proxies".

## 15. Playback

- **Master clock.** The audio device clock (`pm-audio`), including device
  latency (Core Audio latency plus safety offset; Bluetooth adds 150 to 250 ms).
  A monotonic clock stands in when nothing plays.
- **Render-ahead.** Frames render ahead of the playhead into a ring (default 4,
  adaptive); decoders run further ahead per GOP and speed.
- **Presentation.** Frames carry PTS and revision. The viewer keeps a small
  jitter buffer and presents on `requestAnimationFrame` by PTS during playback.
  Scrub and gesture frames bypass the buffer.
- **A/V sync.** The engine measures bridge and compositor presentation latency
  (Chromium exposes no present timestamps for a WebGPU canvas, so latency is
  measured by loopback with a photodiode rig in the lab and by frame-callback
  timing in the app) and offsets audio. Gate: ±10 ms.
- **Drops** are counted, attributed (decode, render, bridge), and reported
  (section 14.9). Today `E.drops` is declared and never updated.
- **Quality ladder.** Under sustained load, Automatic lowers preview resolution
  (down to quarter, as today) and moves clips down the representation ladder.
  Paused frames always render at full quality.
- **Scrubbing.** Paused seeks coalesce to the latest target; a complete frame is
  shown before chasing the next (as today).
- **Transport.** Work-area loop with pre-rolled entrances, frame stepping. New:
  J/K/L shuttle up to 8× and reverse, play around current frame, audio scrub.
- **Cached preview.** Render a range into the render cache and play it at full
  rate with audio. Exposed as Render Work Area (today `PM.Preview.cache()` exists
  without a trigger).

## 16. Audio

- **Engine.** f32 mixing at the project rate (48 kHz default; 44.1 and 96 kHz
  available), resampling with `rubato`, output through `cpal` (Core Audio,
  WASAPI).
- **Decode.** FFmpeg is the single decode path (symphonia lacks Opus and HE-AAC),
  streaming with no length cap. AAC and MP3 priming is honored.
- **Graph.** Clip voices → per-layer strip (gain, mute, solo, meters) → precomp
  nesting (gains multiply as today) → master. Channel layouts are preserved and
  down-mixed to the output layout.
- **Automation** is per sample, not per frame step.
- **Speed and time remapping.** Audio follows speed and remaps. Pitch is preserved
  by default with Signalsmith Stretch (through our own thin binding to upstream);
  a per-clip option lets pitch follow speed. Reverse plays reversed audio.
  Stretch and reverse use defined pre-roll so a seek produces the same samples as
  continuous playback.
- **Frame-rate override** (Interpret) keeps audio in sync by resampling or
  stretching, per a per-asset choice.
- **Waveforms** are engine-computed peaks at several resolutions, cached on disk,
  drawn by the timeline, the Assets panel, and `media.audio.drawWaveform`.
- **Meters, solo, retune** for the mixer come from the engine at display rate.
- **Export.** Offline, sample-accurate. 24-bit or 32-bit float PCM for ProRes and
  WAV; AAC through AudioToolbox for MP4, with `edts` and priming (2112 samples)
  written so audio is not late; TPDF dither when reducing to 16-bit.
- **After cutover:** pan, EQ, compressor, limiter, loudness normalization (EBU R128
  measurement already exists in the agent tools).

## 17. Export and delivery

### Pipeline

```
evaluate → render (float) → output transform → convert to encoder input
  (engine shader: matrix, chroma downsampling and siting, dither) → encode
  → mux → verify → move into place
```

- Runs in the **job engine**. Users keep editing; an export renders the
  document revision at which it started. A crash of the interactive engine does
  not stop an export, and an export crash does not affect editing.
- **Pipelined stages** with bounded queues: decode, evaluate, render, convert,
  encode run concurrently on different frames. Parallelism comes from stages and
  decoders, not concurrent GPU frames.
- **Encoder input.** The engine converts RGB to Y′CbCr itself with its own matrix,
  chroma filter, and siting, and hands VideoToolbox native buffers (`y416`,
  `x422`, `v210`, `x420`) with the CVPixelBuffer color attachments set; those
  attachments become the `colr` and VUI tags.
- **Dither** (TPDF or blue noise, seeded by frame, x, y) is applied in the
  encoded domain only when reducing to 10 bits or fewer, never to alpha, and is
  off for transparency tests.
- **Muxers.** AVAssetWriter for MOV and MP4 (handles `colr`, `mdcv`, `clli`,
  `tmcd`, alpha); FFmpeg for MXF, MKV, and WebM. Timescales: 24000, 30000, or
  60000 for NTSC rates (a 90 kHz timescale cannot represent 23.976 exactly).
- **Atomic output.** Write to a temporary file in the destination folder; move
  into place on success; failure or cancel leaves nothing behind.
- **Verification.** A probe checks codec, profile, pixel format, depth, color
  tags, range, frame count, duration, audio sample count, and alpha. Optional
  full decode.
- **Comfort.** Prevent sleep during export; a macOS notification on completion;
  estimated output size with a warning before starting if the destination lacks
  room; overwrite policy (Replace, Keep both); file name tokens; start timecode
  (default 01:00:00:00) and reel name passed through.

### Formats

| Format | Encoder | Notes |
|---|---|---|
| ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ | VideoToolbox | Alpha straight or premultiplied on 4444; PCM 24-bit or 32-bit float |
| H.264 | VideoToolbox | Bitrate or quality target, profile, keyframe interval. An encode-quality gate (VMAF against x264 at matched bitrate) decides whether a software encoder is also offered, subject to licensing |
| HEVC | VideoToolbox | 8 or 10-bit; HDR (PQ, HLG) on 10-bit; alpha on 8-bit (to be confirmed with `VTCopyVideoEncoderList` in S2) |
| DNxHR LB/SQ/HQ/HQX/444, DNxHD | FFmpeg | MXF OP1a or MOV |
| AV1 | SVT-AV1 | 8 or 10-bit |
| VP9 WebM | libvpx | Alpha; Colour elements written |
| Image sequences | PNG 8/16, TIFF 8/16/32f, OpenEXR half/float (PIZ, ZIP, DWAA), DPX 10/16, JPEG | EXR is scene-linear in the working space with `chromaticities`; an ACES deliverable writes ACES2065-1 (AP0, ST 2065-4); DPX writes transfer and colorimetric fields |
| Still | Same as sequences | Current frame |
| Audio only | WAV, AIFF, AAC | — |
| GIF, animated WebP | Native encoders with palette optimization | Loop count |
| Code export | Section 22 | — |
| Project package | Section 21 | — |
| Captions | SRT, WebVTT sidecars (as today); burn-in | — |

Today's ProRes export uses FFmpeg's `prores_ks` and H.264 uses libx264
(`main/render-encoder.ts:14`). Both are replaced by VideoToolbox (section 24).

### Presets and the render queue

- Presets combine format, size, frame rate, color output, audio, and range.
  Defaults: "Master · ProRes 422 HQ", "Master with alpha · ProRes 4444",
  "Web · H.264", "Web · HEVC", "Image sequence · PNG". Existing presets and
  per-project export defaults migrate.
- **Render Queue panel.** Today a queue dialog exists, but nothing in the UI or
  agent tools adds jobs, each job prompts for a destination, a failure stops the
  run, and the run swaps the live project. The new queue: jobs are added from the
  export sheet or by agents; each job has its destination when queued; jobs run
  unattended in the job engine from a document snapshot without touching the live
  project; failures are reported per job and the queue continues; jobs survive
  relaunch (persisted by main).
- Export sheets and the queue reuse the Settings sheet cards, rows, and switches.
- The export progress sheet keeps its live preview by streaming small preview
  frames from the job engine.
- After cutover: FCPXML and OTIO export.

## 18. Viewer integration and the frame bridge

### The bridge

To be proven by spike S1 before any viewer work.

1. **Rendezvous.** `pm-bridge` in main allocates a Mach receive right and
   registers it under a per-launch random bootstrap name (`bootstrap_check_in`,
   the pattern Chromium uses). The name is passed to the engine through the
   `utilityProcess.fork` environment. Main checks each message's audit-token PID
   against the engine's `child.pid`. On Windows, main calls `DuplicateHandle`
   into its own process (Electron's NT handles are also process-local).
2. **Import once.** The engine allocates a pool of IOSurface-backed textures
   (size: render-ahead ring + 2) in the display's format: `bgra8` or
   `rgb10a2` for SDR, `rgba16float` for HDR, at viewer device pixels only. Each
   surface is sent once as an `IOSurfaceCreateMachPort` right and imported in
   main once with `IOSurfaceLookupFromMachPort` and
   `sharedTexture.importSharedTexture`. The pool's memory is charged to the
   engine budget.
3. **Per frame,** the engine sends only `{poolIndex, seq, pts, revision,
   colorSpace}`, after the frame's command buffer has completed
   (`on_submitted_work_done` or `MTLSharedEvent`). Main calls
   `sendSharedTexture` to the window's renderer.
4. **Release.** A surface returns to the pool only on Electron's release callback
   (`allReferencesReleased`), forwarded to the engine. Never on
   `IOSurfaceIsInUse` alone.
5. **Renderer.** `setSharedTextureReceiver` runs in the preload's isolated world.
   S1 proves the `VideoFrame` reaches the viewer in the main world under context
   isolation. The viewer draws it into its WebGPU canvas with
   `importExternalTexture` (or `copyExternalImageToTexture` into an
   `rgba16float` texture if HDR values do not survive the external texture path),
   applying CSS scale and pan.
6. **Overlays** stay on the DOM and Canvas2D overlay. During gestures, overlays
   draw at the **revision of the presented frame**, so handles never lead the
   pixels. Geometry comes from `pm-wasm` evaluated at that revision.
7. **Capture.** Frames are inside Chromium's compositor, so `capturePage` (agent
   `capture_panel` and `computer_use_panel`) keeps working.
8. **Popout.** If a viewer panel is popped out into another window, its frames
   follow it: the engine sends to that window's renderer.

S1 also determines whether one imported texture can be sent many times or a new
shared image is needed per frame, and measures main-process cost per frame.

### S1 gate

At 4K 60 fps on the base and Pro reference machines, while main is under load
(autosave of the largest corpus project, agent tool traffic, a 50 ms long task
every second, window resize):

- p99 presentation jitter at most 4 ms and zero drops over 60 s;
- pointer-to-pixel latency for a layer drag at most 2 frames at 60 Hz (33 ms) and
  no more than 1 frame worse than today;
- a code-value ramp read back through `capturePage` matches what the engine
  rendered; values above 1.0 survive on an HDR display.

### Fallbacks, ranked by S1 data

- **A′.** Engine in main presenting through a `CAMetalLayer` from a Rust render
  thread, independent of main's JS event loop. Loses crash isolation and puts
  media parsing in the browser process.
- **C′.** Engine in a utility process with a remote layer (`CAContext` and
  `CALayerHost`, the private mechanism Chromium uses). Overlays stay in the DOM
  above a transparent region. Agent capture would need engine-side capture.
- **B.** Shared-memory CPU frames at reduced preview resolution.

### Viewer behavior

- The viewer sends size in device pixels, DPR, zoom, pan, and preview resolution.
  The engine renders only the visible region at the required density.
- During pan and zoom, the last frame is reused scaled while the engine catches
  up, as today.
- `pick`, `bounds`, and text layout are synchronous through `pm-wasm`.
- `frame:rendered` and `overlay` fire on presentation with the frame's revision
  and time. `anim.version()` comparisons with `frame:rendered.version`
  (`viewer.ts:747,916`) keep working because both report the presented
  revision.

## 19. Process model, protocol, security, packaging

### Processes

| Process | Runs |
|---|---|
| Main | Windows, menus, files, save, agent bridge, media store and capability handles, render queue persistence, `pm-bridge` |
| Renderer, per window | UI, kernel, extensions, edit pipeline, `pm-wasm` |
| Interactive engine (utilityProcess, napi-rs) | GPU device, decoders, audio device, caches, sessions |
| Job engine (utilityProcess, napi-rs) | Exports, generated media, background renders |
| Extension sandboxes, transcription, agent CLIs | Unchanged |

Today no code uses `MessageChannelMain`; the transcription utility process talks
only to main. Direct renderer-to-engine ports are new plumbing: main creates a
`MessageChannelMain` per window and engine and hands one end to each. Engine ports
are never given to extension sandbox frames.

### Crash handling

- Main restarts a crashed engine with exponential backoff. Each renderer
  re-sends a canonical snapshot; the viewer shows "Reconnecting…" meanwhile.
  The document never lives only in the engine.
- **Crash attribution.** The engine records the active decode, shader, or job
  before risky work. After two crashes attributed to the same asset or effect,
  it is marked failed for the session ("A001_C003.mov couldn't be decoded" with
  Replace Footage… and Show in Finder) instead of crash-looping.
- Render queue and generation jobs are persisted by main and resume.

### Protocol

- Messages defined in `pm-protocol`, encoded with MessagePack (`rmp-serde` and
  `msgpackr`), TypeScript types generated with `ts-rs` and checked in CI.
- `MessagePortMain` transfers ports only; buffers from the engine side are
  copied. Bulk data (filmstrips, waveforms, scopes) uses shared GPU surfaces or
  files in the cache, not messages.
- Requests carry ids; events carry the revision they correspond to.
- Groups: session (open, close, snapshot, patch), transport, viewer, render
  (frame, thumbnails, filmstrips, posters, snapshots), media (import, probe,
  relink, interpret), proxy, export and queue, audio (meters, waveforms,
  device), engine (budgets, stats, per-node and per-extension GPU timing, errors,
  version).
- Version negotiation at connect.

### Security

- The engine accepts only **capability handles** minted by main (HMAC over
  `{assetId, path, sessionId}`), never raw paths. Import, relink, and Replace
  Footage go through main's file dialogs and validation, as today.
- Decoders run in engine processes isolated from UI and main. Third-party SDKs
  (camera RAW, later) run in further child processes.
- Fuzzing covers the protocol decoder, patch ingest, project parsing, and every
  demuxer and image reader we own or configure.

### Packaging and CI

- Rust toolchain in CI, pinned, `aarch64-apple-darwin` first.
- **Self-hosted Apple Silicon runners**: one base and one Pro reference machine,
  registered for parity and performance jobs only, isolated from secrets, with
  the corpus stored outside the repository. Hosted macOS runners are virtual
  machines without reliable hardware decode or representative GPUs.
- `pm-node` and `pm-bridge` are napi-rs addons, in `asarUnpack`, signed with the
  Team ID and hardened runtime. `scripts/release-mac.mjs` verification is
  generalized: today `NATIVE_RUNTIME` is the transcription runtime list and
  `verifyRuntimeContract` assumes its first entry.
- Our FFmpeg, OCIO, glslang, and OpenEXR are built from source with pinned
  versions; FFmpeg ships as dylibs (LGPL linking model); the others link
  statically. `ffmpeg-static` is removed.
- `pm-wasm` is built with wasm-bindgen and `wasm-pack`. The editor CSP gains
  `'wasm-unsafe-eval'` for the app origin. The same CSP is served by
  `powermove serve` (`server/index.ts:157`), which is accounted for.
- Per-architecture native addons use electron-builder's `singleArchFiles` if
  universal builds are ever needed.
- **Minimum macOS** follows Electron 44's minimum; features that need newer
  versions (HDR APIs, AVIF HDR, VideoToolbox features) are gated at runtime.

## 20. Kernel API, extensions, the agent harness, and other consumers

### API mapping

| API | After cutover |
|---|---|
| `project.get`, `latest`, `apply`, `time`, `play`, `undo` | Unchanged (seconds) |
| `project.snapshot(t, maxWidth)` | Engine render, same return type (async today) |
| `render.snapshot(time, maxWidth)` (sync string) | Served from an engine-filled snapshot cache; returns the last snapshot for that time or `null` while it renders. Built-in callers migrate to `project.snapshot` |
| `anim.ev`, `evP`, `active`, `worldMatrix`, `localMatrix`, `resolveContent`, `findProp`, `allProps`, `hasKeyAt`, `wouldCycle`, `transformParentMatrix` | `pm-wasm`, synchronous, same signatures |
| `anim.version()`, `anim.touch()` | Kept; `touch` records a dirty path set for the write barrier; `version` reports the mirror revision |
| `anim.expressionErrors` (WeakMap by channel object) | Kept by mapping `pm-eval` errors back to channel objects in the renderer |
| `anim.setKey` and in-place helpers | Unchanged signatures; go through the write barrier |
| `model.*` | Unchanged |
| `groups.bounds`, `space3d.*` | `pm-wasm` |
| `transport.*` including `perf {fps, ms, drops, budget, auto}`, `invalidate`, `quality`, `previewResolution` | Engine transport and stats |
| `media.importFiles`, `assets.pick`, `import`, `importUrl`, `get`, `readText` | Engine import through main. `assets.get` keeps returning an object; live fields (`url`, `el`, `audioBlob`, `peaks`, `audioDecodeError`) are provided as compatibility shims backed by engine data (a playable `media://` URL for formats Chromium can play, posters, peaks) and documented as deprecated |
| `media.timing.*`, `media.assets.kind`, `media.fonts` | Kept; fonts come from the engine's font registry instead of `queryLocalFonts` |
| `media.audio.drawWaveform(ctx, …)` | Same signature, from engine peaks |
| `render.gl.bounds`, `pick`, `previewViewport` | `pm-wasm` |
| `render.gl.init(canvas)`, `resize` | Viewer-internal; still accepted and mapped to the bridge |
| `render.gl.compileError` | Engine shader compiler, mirrored to the renderer |
| `render.gl.context` (raw `WebGL2RenderingContext`) | Returns `null` with a deprecation warning; the two built-in call sites are rewritten |
| `render.raster()` | Metrics (`fontOffset`, bounds, `selection`) from `pm-wasm`, synchronous. Pixels (`cv`, `bitmap`) deprecated |
| `render.renderFrameTo()` | Async engine render; sync version deprecated |
| `render.textLayout` | `pm-wasm`, synchronous |
| `services`: `raster` override | Supported for compatibility by uploading the override's pixels to the engine; deprecated in favor of extension layers |
| `services`: `viewer` (`deferNavigationRender`, `canvasTextEditing`, `snapshotSnapCandidates`), `shaderHooks`, `compositions`, `captions` | Kept; implementations call the engine where they touched GL |
| `effects`, `transitions`, `layers.register` | GLSL contract unchanged (section 11), plus WGSL in apiVersion 4 |
| `events` (all 15 `KernelEvents`, including `frame:rendered`, `overlay`, `time`, `invalidate`, `project:changed`, `fonts`) | Unchanged names; frame events fire on presentation |
| `PanelDefinition.library.render` | Kept; the viewer's Library preview uses an engine snapshot |

`EXTENSION_API_VERSION` moves to 4 for the WGSL contract and new async render
APIs. It lives in `packages/registry/src/manifest.ts`, shared with Powermove
Cloud, so the bump needs a coordinated registry release. Every apiVersion 1 to 3
extension keeps loading. The api-pack and `docs/EXTENSIONS.md` are updated,
including the existing mismatch about which varyings effects receive.

### Agent harness

- `render_frames`, composition contact sheets, and snapshots call the engine
  directly. Renders no longer need a visible editor and are faster.
- `apply_commands` text-anchor edits and `get_project_state` text measurement use
  `pm-wasm` instead of the Canvas2D rasterizer.
- `check_project` gets media status and audio peaks from the engine.
- `probe_media` is extended with color metadata, HDR format, and VFR status.
- `validate_effect` gains a real compile through the engine's shader pipeline
  (today it validates structure only).
- New: `manage_proxies`, `export` with presets and queue, `get_render_status`.
- `sample_media_frames`, `media_waveform`, `transcribe_media` keep using originals;
  their retiming math (`legacy/assistant/media-source.ts`) moves to `pm-wasm`.
- `capture_panel` and `computer_use_panel` keep working (section 18).

### Other consumers

Every place outside the viewer that touches the engine is listed in appendix C,
with its replacement. Highlights: composition thumbnails, the project thumbnail
on save, Library look thumbnails and the Library viewer card, Assets panel
posters and inline video, the source preview monitor, timeline and Assets
waveforms, mixer meters, the performance panel and status bar (shader compile
status, GPU loss, per-extension GPU timing with "disable extension"), the export
progress preview, and RAM preview.

## 21. Media management and project files

### Where bytes live

Today every asset's bytes are copied into the renderer's IndexedDB and embedded
in the `.pmv`. The engine cannot read IndexedDB. New model, delivered in
Phase 2 together with keeping originals:

- **Media store in main.** A persistent, content-addressed store owned by main
  replaces IndexedDB as the source of media bytes. The engine reads media only
  through capability handles from main.
- **Linked media.** The project stores path, volume identity, fingerprint, size,
  modification time, and camera metadata (reel, timecode, UMID, duration). Bytes
  stay where they are.
- **Copied media** goes into a project media folder next to the project (or
  embedded in the `.pmv` for small items, as today), never silently mixed by
  size.

**When importing** (Settings › Project › Media): Link to files (default) or Copy
into project; switch "Copy from memory cards" (default on). Files from memory
cards, the clipboard, the web, and temporary folders are copied; files on drives
are linked. Copying from a card is a verified offload: checksums are compared
after copying. After import, a toast says, for example, "Linked 12 clips ·
Copied 3 images".

### Relinking

- Automatic relink only by fingerprint (and UMID or content identifier).
- Matches by name, timecode, or duration are proposals in the Relink sheet that
  the user confirms, because camera names like C0001.MP4 repeat on every card.
- Replace Footage… keeps the interpretation and offers to match scale when
  resolution differs.
- Offline clips show "Media offline · A001_C003.mov" in the viewer and a quiet
  "Offline" in the timeline clip label. Offline clips with proxies keep playing.

### Collect and package

- **Collect Project…** writes a folder with the project, originals, fonts, LUTs,
  the OCIO configuration, and the Store extensions used. Option: "Trim media to
  used ranges, with 2-second handles".
- **Package for Travel…** includes proxy segments, small originals, fonts, LUTs,
  the OCIO configuration, and extensions. Originals reconnect by fingerprint on
  return.

### File format

- Desktop Save writes PMV4; Export › Project and the browser fallback write PMV3.
  Both keep their containers. Linked assets add records without bytes.
- Generated media is never stored in the project file.
- **`minReaderVersion` guard.** No current build checks a version on load, so an
  older build would "repair" new fields and autosave over them. Phase 0 ships a
  guard in a normal release: a project with a `minReaderVersion` above the
  build's shows "This project needs a newer version of Powermove" and opens
  read-only.

### Migration of existing projects

- VP9 transcodes that replaced originals cannot be undone. On open, such assets
  show "Using a converted copy · Relink to the original for full quality", with
  Locate Original…. Relinking by fingerprint of the original (stored today as
  `fingerprint` and `sourcePath`) restores full quality.
- Embedded media moves from IndexedDB to the media store on first open with the
  new engine. 1280-pixel editing copies in IndexedDB are deleted.

## 22. Web player, `powermove serve`, and the headless document host

### Web export and `packages/player`

`packages/player` is a build-only package over `renderer/src/player/*`, which
imports `legacy/core` and `legacy/gl` directly. It powers code export, the www
hero animation (through the SVG backend and `createEngine`), and a prebuilt copy
vendored into onboarding (`renderer/public/onboarding/player.js`).

1. **At cutover**, the TypeScript evaluator, rasterizer, and compositor move into
   `packages/player` as a frozen "web player v1" serving code export, www, and
   onboarding. Code export keeps producing the v1 scene format (seconds). A
   parity test compares the web player with the engine in Standard mode, and
   the vendored onboarding copy is rebuilt from it.
2. **After cutover**, a WASM build of `pm-eval`, `pm-text`, `pm-vector`, and
   `pm-render` on WebGPU replaces it for new exports (scene format v2), decoding
   with WebCodecs. Old exports keep their bundled v1 runtime.

### `powermove serve`

Paused today. Each tab runs the full TypeScript engine. Deleting the legacy
compositor would leave serve tabs without a preview, so the decision is made
before Phase 5, not after:

- **Interim (recommended):** serve tabs use web player v1 from `packages/player`
  as their preview renderer.
- **Later:** the host runs the engine and streams viewer frames as low-latency
  video, or the tab runs the WASM engine.

### The headless document host

`apps/desktop/src/engine/main.ts` runs `legacy/core` under happy-dom for
document and agent tools when serving. It switches to `pm-wasm` for evaluation,
text measurement, and edit kernels. This also fixes two problems found in the
survey: `export_captions` and `generate_captions` are routed there but
`captions/install` is never loaded, and `editing.ts:859` calls `PM.raster`
unguarded on anchor edits of text layers with font anchor bounds.

### Headless rendering

`pm-cli` renders a `.pmv` to any export format without Electron. It accepts
presets, ranges, and parameter overrides (the `setParameter` and `setText`
equivalents) for template rendering, and hosts the parity runner. It is exposed
through the existing `powermove` command from `packages/cli` (for example
`powermove render`), not a second command.

## 23. Cross-platform

- **macOS:** Apple Silicon. Intel only if demand justifies the test cost.
- **Windows (after cutover):** wgpu DX12, D3D11/D3D12 video decode through FFmpeg
  hardware acceleration, Media Foundation where needed, WASAPI, DirectWrite,
  NT-handle shared textures with adapter matching by LUID. The Electron shell
  needs a Windows build, installer, signing, and a native-conventions review: a
  parallel product lane.
- **Linux:** `pm-cli` and serve only, with Vulkan or a software fallback.
- Platform code lives behind traits in `pm-media`, `pm-gpu`, `pm-text`, and
  `pm-audio`, each with a software fallback for tests.

## 24. Licensing and compliance

Settled with counsel before the dependent phase ships.

1. **FFmpeg.** Replace `ffmpeg-static`, a third-party build whose configuration
   we have not audited, with an FFmpeg we build from source: pinned version,
   LGPL or GPLv3-compatible components only, never `--enable-nonfree`, shipped as
   dylibs. The build covers everything that uses FFmpeg today: export,
   the media-proxy converters (libvpx, libopus, PNG), and the agent media tools
   (`ebur128`, `silencedetect`, `astats`, `showwaves`, `tile`, `thumbnail`,
   MJPEG, `scale`, `fps`). This is a prerequisite that runs in parallel with
   Phase 0 and gates Phase 2.
2. **ProRes.** Apple licenses ProRes and names FFmpeg's implementation
   unauthorized. Today's export uses FFmpeg's `prores_ks`. On macOS, use
   VideoToolbox for ProRes decode and encode. On Windows, ProRes decode and encode
   need Apple's licensing program or are omitted.
3. **H.264 and HEVC encode.** Today's H.264 export uses libx264. Prefer the OS
   encoders; confirm royalty exposure for any bundled software encoder given
   Powermove's pricing.
4. **AGPL and proprietary SDKs.** Camera RAW SDKs are closed. Linking them into an
   AGPL program needs a GPLv3 section 7 additional permission from all copyright
   holders, which needs a contributor policy that allows it.
5. **Dependencies.** `cargo-deny` enforces an allowed license list (MIT,
   Apache-2.0, BSD, Zlib, ISC, Unicode, and reviewed exceptions) and RustSec
   advisories.

## 25. Testing, parity, and gates

### Baseline

- **E2E ledger.** E2E specs were already failing on main. Phase 0 makes the suite
  green or explicitly quarantined, with a ledger of each quarantined spec, its
  owner, and its reason. Gates refer to that ledger.
- **Parity corpus capture** happens before engine code lands: the 12 pixel-hash
  scenarios (today compared only when `PM_RENDER_BASELINE` is set; no hashes are
  checked in), every e2e fixture, synthetic projects per feature (each layer
  type, effect, transition, blend mode, matte mode, mask type, text animator, 3D
  feature, retiming mode, motion blur path), and real-world projects Jude
  supplies (outside the repository). Output: float EXR and 8-bit PNG per frame at
  chosen times, and evaluated property dumps as JSON.

### Parity

- **Evaluation.** `pm-eval` matches the legacy evaluator's dumps within
  |a − b| ≤ 1e-9 + 1e-9·|b| for numbers, exactly for strings and booleans.
- **Render (Standard mode).** Per-channel 8-bit difference at most 1 for 99.9% of
  pixels and at most 4 for any pixel. Exclusions, each listed per case with a
  reason and reviewed: text (SSIM ≥ 0.98 plus a reviewed side-by-side sheet),
  video (today's frames pass through Chromium's own YUV conversion; the engine's
  correct conversion is a known improvement), edge antialiasing, blur kernels,
  alpha rounding, and the defects in appendix A.
- **Web player.** The frozen web player v1 against the engine in Standard mode.

### Correctness and conformance

- Section 2 bars as automated tests.
- **Codecs.** Decoded frames match FFmpeg reference decodes (framemd5, or within 1
  CV for hardware decoders); random access equals sequential decode; frame counts
  and timestamps exact; corpus includes open-GOP HEVC, B-pyramid H.264, edit
  lists, VFR phone footage, interlaced MXF, HDR HLG and PQ, Dolby Vision, HEVC
  and H.264 4:2:2 10-bit, ProRes 4444 XQ.
- **Exports.** Every format probed for codec, profile, pixel format, depth, tags,
  range, frame count, duration, audio samples, alpha; pixel values checked, not
  just tags.
- **Determinism.** Every corpus case renders twice with identical buffers;
  native and WASM evaluation identical.
- **Fuzzing.** `cargo-fuzz` targets for protocol, ingest, parsers.

### Infrastructure

- Rust unit and property tests per crate.
- `pm-cli parity` renders the corpus and writes a report with difference images.
- Playwright e2e runs through the same UI. Specs tied to WebGL internals are
  rewritten against engine stats.
- Existing harnesses stay: `native-export-performance`, `opaque-capture-parity`,
  `export-video-frames`, `video-frame-integrity`, `rendering-performance`,
  `export-code`.
- A debug "Compare engines" command renders the current frame with both engines
  and shows the difference. Removed at cutover.

### Performance gates

Calibrated in Phase 0 on the reference machines (a base M-series MacBook Air with
a ProRes engine, and an M-series Pro), then fixed. Each gate has an owning phase.

| Scenario | Gate | Phase |
|---|---|---|
| Frame bridge under main load | Section 18 S1 gate | 0 |
| Layer drag, 884-layer project | Pointer-to-pixel p95 ≤ 33 ms at 60 Hz, at most 1 frame worse than today | 2 |
| Canvas text typing | Keystroke-to-pixel p95 ≤ 33 ms | 3 |
| Inspector edit during playback, 1,000 layers | Edit latency p95 ≤ 16 ms, no dropped playback frames | 2 |
| UHD ProRes 422 HQ, 3 layers with blur | Real time at 23.976, 29.97, 59.94 on Pro; 29.97 on base; zero drops over 60 s | 3 |
| 4 to 9 streams of ProRes (multicam-style) | Real time at 29.97 on Pro | 3 |
| 4K long-GOP HEVC 10-bit and H.264 4:2:2 10-bit, Automatic | Real time on both machines, with generated media when the policy calls for it | 4 |
| Scrub, intra codecs | p95 ≤ 50 ms | 2 |
| Scrub, long-GOP before optimized media | p95 ≤ 300 ms | 2 |
| Scrub, long-GOP with optimized media | p95 ≤ 100 ms | 4 |
| Paused frame upgrade to original | ≤ 150 ms (4K ProRes, base) | 4 |
| The 884-layer project | All five ceilings that fail today pass | 3 |
| Export | Absolute fps per codec and machine, measured against today's re-measured baseline for that codec | 3 |
| A/V sync | ±10 ms at presentation | 2 |
| Memory | Within the budget setting; no growth over a 1-hour playback loop; `pm-wasm` per-window budget met | 2 and 3 |
| Linear and HDR preview on | The UHD gates above still pass | 3 |
| Cold start | Engine ready before the window finishes loading the last project | 2 |

## 26. Phases

Phases are ordered by dependency; lanes run in parallel within and across phases
(section 27). The legacy engine stays the default until Phase 5. Jude's sign-off
is required at each gate in addition to its objective criteria.

### Phase 0: Foundations, contracts, spikes

Work:

- Cargo workspace, CI toolchain, `cargo-deny`, napi-rs and wasm-pack builds; a
  placeholder addon signed and notarized through the real release script.
- Self-hosted reference runners.
- E2E ledger; parity corpus capture.
- `minReaderVersion` guard shipped in a normal release.
- Frozen lane contracts (section 27).
- Spikes, each a throwaway branch with a written result:
  - **S1 frame bridge** (section 18), including gesture latency and HDR.
  - **S2 precision and encode:** ProRes 4444 XQ in → wgpu → ProRes out, VideoToolbox
    pixel formats, f16 versus f32 on 12-bit and 16-bit sources, HEVC alpha depth
    (`VTCopyVideoEncoderList`), AVAssetWriter tags.
  - **S3 shaders:** every GLSL source in the repository, the Store, and the docs
    samples through section 11; compare images with WebGL; naga versus
    SPIRV-Cross; flip and derivative cases.
  - **S4 text and vectors:** Parley, HarfRust, Skrifa layout; glyph and vector
    rendering through (a) Vello for coverage with our own float paint, (b)
    `vello_gpu`, (c) Skia Graphite through `skia-safe` on Metal with half-float
    surfaces; compared with Canvas2D and CoreText; stress test with 4K
    caption-heavy scenes and large paths; a CPU fallback kept.
  - **S5 WASM evaluator:** `evP` from JS under 1 µs on cached values through the
    shared-buffer fast path; patch apply under 0.2 ms for property edits and under
    2 ms for structural edits on the 884-layer project with only changed ids
    invalidated; per-window memory budget.
  - **S6 decode:** VideoToolbox and FFmpeg decode of the corpus into wgpu without
    CPU copies; exact random access from container indexes.

Gates, per consumer rather than one Phase 0 gate:

- S5 and the contracts gate Phase 1.
- S1, S2, S6, the FFmpeg replacement, and the reference runners gate Phase 2.
- S3 and S4 gate the effects and text lanes in Phase 3.

### Phase 1: Evaluation and the write path

Work: `pm-time`, `pm-model` (`v1_ingest`, id-keyed reconciliation), `pm-eval`,
`pm-edit`, `pm-vector` geometry, `pm-text` layout with font subsets,
`pm-protocol`, `pm-wasm`. The write barrier, flush-before-query, shadow-replay
auditor, dual-answer mode. The renderer's synchronous queries and the
geometry-dependent edit operations switch to `pm-wasm` behind a flag while the
legacy compositor still renders. The headless document host switches to
`pm-wasm`.

Gate: evaluation parity on the whole corpus; zero dual-answer mismatches and zero
auditor failures over the full e2e suite (per the ledger); native and WASM
bit-identical; S5 targets met in the app.

### Phase 2: Tracer bullet

A thin vertical slice through every layer of the architecture, behind a "Use new
engine" flag, judged on real projects:

- Engine processes (`pm-engine`, `pm-node`), protocol over MessagePorts,
  capability handles, crash handling.
- The production frame bridge with jitter buffer and revision-tagged overlays.
- `pm-media`: probe, container indexes, VideoToolbox decode of ProRes, H.264, HEVC;
  image readers.
- Minimal `pm-render`: solid, image, video, transform, opacity, normal blend,
  precomp, groups; Standard mode with color invariants.
- `pm-audio`: device clock, streaming decode, basic mixing; playback on the audio
  clock; scrubbing.
- `pm-export`: ProRes and H.264 through VideoToolbox in the job engine, atomic
  output, verification.
- Media store in main, linked media, copy-from-cards offload, relinking by
  fingerprint; originals kept on import (for projects opened with the flag on).

Gate: the Phase 2 rows of the performance table; ProRes imports play and export
from originals with the pipeline-transparency and color-invariant bars passing;
crash and GPU-loss recovery tests pass; memory gates for the slice.

### Phase 3: Breadth and parity

Work: everything else in appendix B: all layer types, shapes and paths, text and
captions, masks and mattes, all blend modes, effects and transitions (WGSL
built-ins and the GLSL compatibility pipeline), motion blur, 2.5D, adjustment
layers, retiming, Linear mode, HDR input tone mapping, all import formats, all
export formats, the render queue, audio stretch and reverse, meters and mixer,
waveforms, thumbnails and posters, the kernel API mapping (section 20), agent
tool changes, every consumer in appendix C, and `pm-cli`.

Gate: render parity in Standard mode on the whole corpus; Linear-mode correctness;
codec and export conformance; text review; the Phase 3 performance rows;
determinism.

### Phase 4: Proxies, optimized media, and caches

Work: `pm-proxy` (ladder, segments, policy, machine profile, disk guard), camera
proxy attach, render cache with visibility and Keep Rendered, background
rendering, all controls in section 14.8, diagnosis, Storage, Collect and Package
for Travel, the Relink sheet, Replace Footage, offline-media export behavior.

Gate: the Phase 4 performance rows on the base machine without user action; the
cache-to-export byte-identity test; no proxy pixels in exports without a draft
choice; generated media survives moves, relinks, and relaunch; storage limits and
pinning enforced.

### Phase 5: Cutover

Work: new engine on by default for one release with the legacy engine behind a
switch; serve interim decided and implemented; then removal of `legacy/gl` and
the legacy media, playback, audio, and export paths from the editor; web player
v1 moved into `packages/player`; onboarding copy rebuilt; `EXTENSION_API_VERSION`
4 with the registry release; documentation updated (`EXTENSIONS.md`, README,
the engine design docs in `apps/desktop/docs`).

Gate: over at least 14 days and a defined number of sessions on the switch
release, crash-free sessions at least 99.5%, zero open P0 or P1 regressions;
legacy engine code deleted from the editor; docs updated.

### After cutover

Each with its own plan, roughly in this order:

1. Professional color: ACES mode, OCIO, wide gamut and HDR projects, PQ and HLG
   export, scopes, LUTs and color effects, legalizer and warnings, reference
   viewer.
2. Windows.
3. Depth-buffered 3D, then camera and light layers.
4. Spatial bezier motion paths; frame blending and optical flow.
5. Camera RAW (after section 24.4), starting with BRAW and R3D out of process.
6. WASM and WebGPU web player (scene v2).
7. Audio effects and loudness tools.
8. OpenFX hosting out of process.
9. `pm-doc`: edit pipeline and history in Rust, ticks on disk, schema v2.
10. PSD and multi-part EXR layers; FCPXML and OTIO interchange.

## 27. Lanes, contracts, and ownership

### Contracts frozen in Phase 0

1. `EvaluatedScene`, the intermediate representation between `pm-eval` and
   `pm-render`.
2. `FrameRequest` and `FrameDescriptor` (region, scale, color, precision,
   revision, PTS).
3. The `pm-gpu` interop trait owning every IOSurface and CVPixelBuffer import and
   export (decode in, bridge out, encode in).
4. The color contract: encodings per mode, premultiplication rules, reference
   display encoding, where display and output transforms run, cache-key
   components.
5. Patch, path, canonicalization, and `v1_ingest` rules.
6. The playback clock interface between `pm-audio` and the scheduler.
7. The error and isolation model (per node, per layer, per asset, crash
   attribution).
8. The budget API.
9. Protocol codec and code generation (MessagePack, `ts-rs`).

### Lanes

| Lane | Owns | Phases |
|---|---|---|
| Model and evaluation | `pm-time`, `pm-model`, `pm-eval`, `pm-edit`, `pm-wasm` | 0, 1, support |
| Renderer write path | Write barrier, flush, auditor, dual-answer mode, edit-kernel integration, headless document host | 1 |
| Text and vectors | `pm-text`, `pm-vector` | 0 (S4), 1, 3 |
| Media | `pm-media`, decode backends, image readers, index | 0 (S6), 2, 3 |
| GPU and render | `pm-gpu`, `pm-render`, GLSL compatibility | 0 (S3), 2, 3 |
| Color | `pm-color`, invariants, input tone mapping | 0 (S2), 2, 3, after |
| Engine and platform | `pm-engine`, `pm-node`, `pm-bridge`, `pm-protocol`, packaging, CI, runners, FFmpeg build | 0 (S1), 2 |
| Audio | `pm-audio`, mixer integration | 2, 3 |
| Export | `pm-export`, `pm-cli`, render queue | 2, 3 |
| Proxies and storage | `pm-proxy`, `pm-cache`, media store, media management UI | 2, 4 |
| Viewer and settings UI | Viewer integration, overlays, settings, panels, appendix C consumers, API mapping, extension docs | 2, 3, 4, 5 |
| Parity and QA | Corpus, ledger, parity runner, conformance, performance gates | 0 onward |

Rules:

- Each lane works in its own worktree and branch, commits granularly, and submits
  stacked PRs. No squash merges.
- Reviews use independent reviewers. The parity and QA lane owns the gates and
  reports failures as they are, with no loosened thresholds.
- The legacy engine is frozen for features from the start of Phase 3. Bug fixes
  continue.

## 28. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `sharedTexture` is experimental and tracks Chromium internals | Viewer path breaks on an Electron upgrade | S1 first; ranked fallbacks; Electron upgrades gated by a frame-bridge test |
| Gesture latency through the bridge | Handles feel detached; motion designers notice first | Revision-tagged overlays; latency gates in S1 and Phase 2 |
| Main-process jitter delays frames | Dropped or uneven playback | Import-once pools, tiny per-frame messages, jitter buffer, S1 under load, fallback A′ |
| Reads after writes inside edits | Wrong geometry in reparent, group, align | Write barrier, flush-before-query, edit kernels, dual-answer mode |
| Text looks different | Visible regressions on every project | S4, compatibility metrics, coverage tuning, reviewed corpus |
| GLSL translation fails on some shaders | Broken Store extensions and saved projects | S3 on every known source; two translation paths evaluated; uniformity checks off; clear errors |
| Vector renderer maturity (Vello pre-1.0, 8-bit layers in `vello_gpu`) | Quality or stability problems | S4 includes Skia Graphite; float paint in our own shaders; CPU fallback |
| Hidden consumers of renderer-side media | Blank thumbnails, silent waveforms | Appendix C checklist owned by the viewer and settings UI lane |
| No CI hardware | Gates cannot run | Self-hosted runners in Phase 0 |
| Feature drift during the rewrite | Double work | Legacy feature freeze from Phase 3 |
| wgpu, Electron, OCIO, FFmpeg upgrades | Churn | Pinned versions; upgrades as lanes |
| Licensing | Blocked features or exposure | Section 24 before dependent phases |
| Memory on 8 GB machines | Crashes | One budget authority, pool sizing, `pm-wasm` budgets, base-machine gates |
| Engine crashes and crash loops | Lost playback or exports | Two engine processes, attribution, backoff, persisted jobs, atomic output |
| Hardware differences across chip generations | Inconsistent behavior | Software fallbacks; conformance on more than one generation |
| Old builds overwriting new projects | Data loss | `minReaderVersion` guard shipped in Phase 0 |

## 29. Decisions needed from Jude

1. **Default color for new projects.** Recommended: Standard with "Blend in
   linear light" off. Powermove's audience builds brand and UI work that must
   match Figma and web hex values, and After Effects ships the same default.
   Footage-oriented templates turn linear blending on.
2. **When importing.** Recommended: Link to files, with "Copy from memory cards"
   on (by provenance, not size).
3. **Generated media location.** Recommended: the Powermove cache, with "Next to
   the project" as a sidecar folder option, never inside the `.pmv`.
4. **Optimized media.** Recommended: Only when I ask.
5. **Disk guard and storage limit defaults.** Recommended: pause below the larger
   of 20 GB or 10% free; storage limit 25% of the cache volume.
6. **Windows timing.** Right after cutover's professional color work (as written)
   or later.
7. **Intel Macs.** Recommended: no.
8. **Web export future.** Recommended: replace the v1 JS player with the WASM
   engine after cutover.
9. **Serve interim.** Recommended: serve tabs use web player v1 until a streaming
   or WASM path exists.
10. **Reference machines** for the self-hosted runners.
11. **Licensing counsel** for section 24 items 2 to 4.
12. **Real-world corpus projects** (kept outside the repository).

## 30. Appendix A: defects found during the survey

Fixes worth making in the legacy engine during Phase 0 are marked (P0); the
corpus then captures intended behavior. Others are fixed by the engine.

1. **Converted video replaces the original.** ProRes, MKV, MXF, and other
   FFmpeg-only formats are stored as VP9 `yuva420p` with Opus audio; the original
   is never kept (`raster.ts:874-889`, `media/media-path.ts:29`,
   `main/media-proxy.ts:39-51`). `apps/desktop/README.md:110-111` says the
   original stays embedded; it does not (`README.md:152-156` is correct). (P0:
   correct the README.)
2. **Export transfer tag** is sRGB (`iec61966-2-1`) for H.264 and ProRes
   (`render-encoder.ts:12-15`). Not a one-line fix: the right tag depends on the
   color invariants in section 9. Fixed by the engine.
3. **ProRes precision.** 10-bit with `-alpha_bits 16` from 8-bit data; 24-bit PCM
   padded from 16-bit WAV.
4. **Motion blur sample counts differ by path:** 16 (opaque native export, PNG,
   still; `frame-capture.ts:24`), 20 (alpha, WebM, Recorder;
   `exporter.ts:612,620`), 6 (web player, RAM preview), 6 playing and 12 paused
   (preview; `engine.ts:426`), 10 (compositor default), 1 (agent snapshots and
   composition thumbnails). (P0: one setting for all export paths.)
5. **WebM and Recorder exports may drop shaders still compiling.** `renderInto`
   (`exporter.ts:609-614`) renders without `exporting: true`, and no caller passes
   it anywhere, so parallel compilation can skip a layer on early frames. Inferred
   from code. (P0)
6. **Render queue.** A dialog exists, but nothing adds jobs, each job prompts for a
   destination, a failure stops the run, and the run swaps the live project
   (`render-queue.ts:4-12`).
7. **Frame rate handling differs by path:** the UI and `set_composition` round to
   integers; load, hydrate, and validation keep fractions.
8. **Timecode at fractional rates** is wrong (`util.ts:197-205`). (P0)
9. **Variable speed** integrates at twice the open composition's rate, at least
   60 Hz, not the nested composition's (`retiming.ts:13`).
10. **Audio** ignores clip speed; time-remapped clips are silent.
11. **WebM exports** carry no color metadata.
12. **Aliasing.** Hard UV edges with `antialias: false`; no mipmaps.
13. **Large blur radii** skip texels (`effects-basic/effects.ts:43-49`).
14. **Masks.** More than 8 parametric masks are silently dropped on the GPU path;
    any path mask sends all masks through Canvas2D with a CSS-blur feather.
15. **Solid corner radius** is stored but not rendered.
16. **Headless document host.** Captions tools routed there fail (captions never
    installed); text anchor edits can throw (`editing.ts:859`).
17. **Docs disagree** on effect varyings (`EXTENSIONS.md:439` versus
    `api.ts:180-191`) and on when editing copies are made
    (`video-preview-performance.md` versus `video-preview.ts:23-24`). (P0)
18. **51 tracked Finder duplicates** (`* 2.*`) across the repository. (P0: delete.)
19. **`PM.previewFps`** is read but never assigned (`engine.ts:47`).
20. **No version check on load** (section 21). (P0: the guard.)
21. **Unbounded caches.** The IndexedDB media store, editing copies, the live asset
    map, Bezier and spring solver caches, and font sources have no eviction.

## 31. Appendix B: feature parity inventory

Everything below works in the engine before cutover.

**Composition.** Size; frame rate (rational); duration; work area; background
(none, solid, linear and radial gradient with up to 8 stops; radial has angle
only); shutter as a fraction of a frame (0 to 2, default 0.5); master audio gain;
markers; scene parameters (`param()`, `setParameter`); multiple compositions;
nesting with cycle checks; pre-compose (move and leave-attributes modes).

**Layers.** solid, shape (rect, ellipse, polygon, star with fixed 0.46 inner
ratio, line, editable paths), text, image, SVG, video, audio, adjustment, shader,
extension (fragment, mesh), null, group, precomp, captions. Fields: visibility
(keyframable), lock, shy, solo, collapsed, color label, blend (keyframable),
motion blur (keyframable), 3D, linked scale, parent, group, track, matte source
and mode (keyframable). Parenting that preserves pose. Groups as transform spaces
and compositing boundaries.

**Transforms.** Anchor, position, scale, rotation, opacity, skew (±80°); Z
position, anchor, scale; X and Y rotation; orientation; perspective per 3D group.

**Animation.** Keyframes: linear, bezier, hold; speed and influence;
auto-bezier; continuous; spring parameters; legacy easing presets; color
interpolation; string and boolean steps. Expressions: identifiers `time`, `T`,
`t`, `fps`, `value`, `layer`, `comp`, `ch`, `idx`, `PI`, `Math.*`; helpers
`clamp`, `lerp`, `linear`, `ease`, `random`, `wiggle`, `bounce`, `loop`,
`pingpong`, `param`.

**Shapes and paths.** Fill and stroke, per-corner radius, smoothing (squircles),
polygon and star points, cubic paths with per-vertex tangents; trim
(start, end, offset) and repeater (up to 256 copies) on editable paths; fill and
stroke opacity; SVG import to editable paths.

**Text.** Font, weight, size, tracking, leading, color, alignment, italic, box
width and height, variable axes, range styles, animators (stagger, range, wave;
characters, words, lines; all orders and shapes; opacity, position, scale,
rotation, skew, tracking, blur, color), font anchoring, local and embedded fonts,
canvas text editing.

**Masks and mattes.** Parametric rect and ellipse masks with feather, rotation,
add and subtract; bezier path masks; track mattes (alpha, alpha inverted, luma,
luma inverted); matte nesting and cycle rejection.

**Compositing.** Nine blend modes (normal, add, screen, multiply, overlay, soft
light, difference, lighten, darken); opacity; group effects; adjustment layers;
motion blur; in and out transitions per layer.

**Effects.** Gaussian Blur, Directional Blur, Sharpen, Glow, Vignette, Drop
Shadow, Color, Levels, Duotone, Posterize, Invert, Film Grain, Mosaic, Chromatic
Aberration, Turbulent Displace, Gradient Ramp; multi-pass, backdrop, `keepOrig`,
viewport-safe and padding semantics; extension effects.

**Transitions.** Crossfade, Wipe, Slide, Zoom; extension transitions.

**Shader and extension layers.** Shadertoy-style header, `@param` uniforms, five
presets, Studio Cube, OBJ model.

**Retiming.** Animated speed, time remap, freeze, reverse, source time,
independent decoders per instance.

**Media.** Every import format today, animated images, image sequences with gap
reporting, posters, iCloud and File Provider materialization, URL import for
extensions, cloud media prompts, remote media in serve mode, OBJ, SRT and VTT
import to captions.

**Audio.** Per-layer gain, mute, solo, fades, keyframed gain, precomp nesting,
embedded video audio, meters, waveforms, master gain.

**Preview.** Resolution (Automatic, Full, Half, Quarter), Automatic quality,
frame cache, cached preview, retained presentation during pan and zoom,
viewport-cropped rendering, context-loss recovery, FPS readout and status states.

**Export.** Every format in today's dialog (native MP4 and ProRes, WebM, realtime
WebM, PNG sequence, still, code export, project), presets, per-project defaults,
work-area range, scale, fractional rates, captions sidecars and burn-in, progress
with live preview, cancel, render queue.

**Kernel and agents.** Every API in section 20; every agent tool listed there.

## 32. Appendix C: integration checklist

Every consumer of today's engine outside the compositor, with its replacement.
Owned by the viewer and settings UI lane unless noted.

| Consumer | Today | Replacement |
|---|---|---|
| Composition thumbnails (`panels/comp-thumbnails.svelte.ts:58-70`) | `PM.GL.renderToPixels` synchronously | Async engine thumbnails |
| Project thumbnail on save (`legacy/app.ts:567-576`) | Reads the viewer's WebGL canvas | Engine snapshot |
| Library look thumbnails (`legacy/core/library.ts:87`) | `PM.Export.snapshot` | Async engine snapshot |
| Library viewer card (`library/LibraryScreen.svelte:352-361`) | Copies live panel canvases | `library.render` with an engine snapshot |
| Assets panel posters and inline video (`panels/AssetsPanel.svelte:152-153,209-217,1019-1024`) | IndexedDB posters, `<video src>` blob URLs | Engine posters and filmstrips; `media://` playback only for formats Chromium supports, otherwise engine frames |
| Assets panel audio thumbnails (`AssetsPanel.svelte:254-261`) | `asset.peaks` from WebAudio decode | Engine peaks |
| Source preview monitor (`viewer/source-preview.ts:177-205`) | `<video>` and `<audio>` from asset URLs | Engine-rendered source frames through the bridge, engine audio |
| Timeline waveforms (`timeline.ts:1702-1716`) | `drawWaveform` over WebAudio peaks | Engine peaks |
| Mixer meters, solo, retune (`panels/MixerPanel.svelte`, `mixer/meter-loop.ts`) | WebAudio analysers | Engine meters at display rate, engine solo and retune |
| Caption rendering (`captions/render.ts`) | Canvas2D | `pm-text` and `pm-vector` |
| Text editor caret and font offset (`viewer/text-editor.ts:98-102`) | `textLayout`, `raster().fontOffset` | `pm-wasm` |
| Inspector text animators, timeline text stagger (`TextAnimators.svelte:142`, `text-stagger.ts:30`) | `textLayout` | `pm-wasm` |
| Align and distribute, group and comp bounds (`shortcuts.ts:150,222`, `layer-groups.ts:48`, `compositions.ts:504`, `AlignmentStrip.svelte:27`) | `PM.GL.bounds` | `pm-wasm` and `pm-edit` |
| Shader compile status (`panels/shader/CompileStatus.svelte:21`, `ShaderPanel.svelte:51`, `inspector/ContentSection.svelte:35`) | `PM.GL.compileError`, `dropProgram`, program keys | Engine compiler status mirrored to the renderer |
| Status bar (`shell/StatusBar.svelte:6-65`) | `GL.contextLost`, `GL.compiling`, per-extension GPU timing with "disable extension" | Engine stats with per-node and per-extension GPU timing |
| Performance panel and store (`panels/PerfPanel.svelte`, `runtime/install-legacy.ts:101-114`) | `GL.stats`, `rasterStats` | Engine stats |
| Frame bus (`runtime/frame-bus.ts`) | `overlay` right after `GL.render` | Fired on presentation |
| Export progress preview (`legacy/core/export-preview.ts`) | Scans the frames as they render | Small frames streamed from the job engine; decorative shader unchanged |
| RAM preview (`legacy/core/preview-cache.ts`) | `ImageBitmap`s per frame | Render cache, Render Work Area |
| Poster generation (`legacy/core/poster.ts`) | Seeks `asset.el`, `drawImage` | Engine posters |
| Cloud and remote media in the renderer (`legacy/core/cloud-media.ts`, `kernel/remote-media.ts`) | Bytes through the renderer, decode probe in Chromium | Main materializes; engine probes |
| Image sequence import (`legacy/core/image-sequence.ts`) | Size check with `createImageBitmap`, VP9 encode | Engine sequence reader |
| Fonts (`legacy/core/fonts.ts`, `typography/*`, `main/fonts.ts`) | `document.fonts`, `queryLocalFonts`, family names | Engine font registry, subsets to `pm-wasm`; the UI font picker reads the registry |
| Panel popout (`main/panel-popout.ts`) | Same-realm child windows | Bridge sends frames to the window hosting the viewer |
| Web player, www, onboarding (`renderer/src/player/*`, `apps/www/src/lib/svg-player.ts`, `renderer/public/onboarding/*`) | TypeScript engine | Web player v1 (section 22) |
| Headless document host (`src/engine/main.ts`) | `legacy/core` under happy-dom | `pm-wasm` |
| Agent tools (section 20) | Renderer compositor and Canvas2D | Engine and `pm-wasm` |

## 33. Appendix D: glossary

- **Tick:** 1/705,600,000 second; the engine's unit of time.
- **Canonical patch:** a document patch addressed without the open-composition
  root rotation.
- **Mirror:** a copy of the document kept in sync by patches (`pm-wasm`, engine
  sessions).
- **Write barrier:** the mechanism that records every document write so mirrors
  can be flushed before a query.
- **Edit kernel:** a geometry-dependent edit computed in `pm-edit` and returned as
  patches.
- **Representation:** a version of a media asset: original, reduced-resolution
  decode, proxy segments, optimized segments, kept render.
- **Segment:** a ~2-second, keyframe-aligned piece of generated media, addressed by
  content.
- **Machine profile:** measured decode, GPU, disk, and power characteristics of
  this Mac.
- **Standard mode:** display-referred blending that matches today's projects.
- **Linear blending:** linear-light compositing ("Blend in linear light").
- **Reference display encoding:** the fixed sRGB-curve encoding that legacy and
  display-domain effects see, independent of the user's display.
- **Frame bridge:** the path that carries rendered frames from the engine to the
  viewer as shared GPU surfaces.
- **Interactive engine, job engine:** the two engine processes.
- **Parity corpus:** projects and expected outputs captured from the legacy
  engine.
- **E2E ledger:** the list of quarantined end-to-end specs, owners, and reasons.
