# Powermove native engine: plan and design

Status: proposal, not started. Revision 4, reconciled with main at `d5d5b731`
(3D layers and Blender, the Powermove MCP and headless export, agent
orchestration, After Effects extension import).

This document covers the whole rewrite of the Powermove engine (evaluation,
decode, compositing, 3D, color, text, audio, playback, caching, proxies, and
export) from TypeScript, WebGL2, and three.js into Rust. The Svelte UI, the
kernel, extensions, the 3D viewport editor, and the agent harness stay on the
web stack. It is written to be executed by parallel implementation lanes.
Every phase ends at a gate with objective exit criteria; a phase is done when its
gate passes, not when its code lands.

"The engine" means the new native engine. The existing Node host in
`apps/desktop/src/engine/` is called "the headless document host"; the
`powermove mcp` host in `apps/desktop/src/server/mcp.ts` is called "the hidden
MCP host".

## Summary

**What users get at cutover**

- ProRes, DNxHR, MXF, HEVC 10-bit, and every format imported today play and
  export from the original files, never from a lossy transcode.
- Float precision end to end, correct color tags, exact frame rates and
  timecode, sample-accurate audio that follows speed changes.
- 3D model, light, and camera layers rendered natively by the engine, and
  Blender-rendered compositions composited at 16-bit or better.
- Antialiased edges, sharp scaled text and vectors, correct blur at every radius.
- Optional linear-light blending.
- Automatic proxies that need no setup, a viewer that always says what it is
  showing, and exports that never contain proxy pixels by accident.
- Linked media for large footage, relinking, a working render queue, editing
  while exporting.
- Agents, MCP clients, and the CLI render and export on the GPU without a
  browser, concurrently, while the user keeps editing.

**After cutover** (each its own plan): ACES, wide-gamut and HDR projects, scopes,
LUTs and color effects, more delivery formats, Windows, cameras and lights for
2.5D layers, camera RAW, the WASM web player, OpenFX.

**Phases**

| Phase | Outcome |
|---|---|
| 0 Foundations | Contracts frozen, spikes proven, CI hardware, FFmpeg replaced, parity corpus captured |
| 1 Evaluation | The Rust evaluator answers every synchronous UI query, 2D and 3D, with a write path that keeps it exact |
| 2 Tracer bullet | Engine processes, frame bridge, ProRes and H.264 playback and export from originals, one 3D layer, linked media, agent export on the job engine |
| 3a Rendering parity | Every layer, effect, text, mask, blend, motion blur, 2.5D and 3D feature, Blender compositing |
| 3b Media and delivery | Every import and export format today, model import, Blender jobs, render queue, audio |
| 3c Integration | Kernel API including `api.scene3d`, agent and MCP tools, the 3D viewport, every engine consumer, CLI |
| 4 Proxies and caches | Automatic proxies, render cache, media management |
| 5 Cutover | New engine default, legacy engine removed |

**Cutover is done when** the Phase 5 gate in section 26 passes.

**Decisions needed** are in section 29.

Contents

1. Why
2. Goals, non-goals, and the quality bar
3. What exists today
4. Architecture overview
5. Time
6. The document: ownership, writes, mirrors, engine facts
7. Evaluation (`pm-eval`, `pm-geom3d`)
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
20. Kernel API, extensions, agents, and MCP
21. Media management and project files
22. Web player, serve, the headless hosts, the CLI
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
33. Appendix D: media and proxy interface
34. Appendix E: glossary

---

## 1. Why

Powermove is used by more professional editors and motion designers every
release. Their footage, codecs, and color expectations exceed what the browser
engine can deliver. The limits are structural:

- **Codecs.** Chromium cannot decode ProRes, DNxHR, MXF, camera RAW, or most
  intermediate codecs. Powermove transcodes them on import to VP9 `yuva420p`
  (CRF 18) with Opus audio (`main/media-proxy.ts:39-51`), and that transcode is
  what gets stored, saved, and exported. The original is never kept; only its
  path and fingerprint are (`legacy/gl/raster.ts:887-902`).
- **Precision.** Source textures are RGBA8 with premultiply-on-upload
  (`legacy/gl/compositor.ts:546-550`). Intermediates are RGBA16F, but export
  readback is 8-bit (`compositor.ts:1871`). ProRes 4444 is written at 10-bit
  with `-alpha_bits 16` from 8-bit data. 3D layers arrive as 8-bit too: three.js
  canvases and Blender PNGs.
- **Color.** No color management. Blending, blur, opacity, and motion blur run on
  gamma-encoded values. Import ignores color metadata. Export tags H.264 and
  ProRes with the sRGB transfer. Native 3D tone-maps with ACES Filmic and
  Blender with AgX, so a 3D layer looks different in Solid preview, Rendered
  preview, and export.
- **Time.** Float seconds everywhere. Four paths set composition frame rates
  differently: the UI and `set_composition` round to integers, load and
  validation keep fractions, opening media from Home copies the file's average
  rate unrounded (`shared/mp4-frame-rate.ts`, `legacy/app.ts:1338-1352`), and the
  MCP `create_project` accepts any float from 1 to 120. Timecode is wrong at
  fractional rates.
- **Audio.** WebAudio with whole-file decode and a 128 MiB PCM budget (about 5.8
  minutes of stereo float). Speed changes are ignored; time-remapped clips are
  silent.
- **3D.** Model layers render through three.js in a second WebGL context and are
  copied into the compositor once per layer per frame. Blender renders are
  prepared by the renderer before each export frame, one render per 3D layer
  per motion-blur sample, serialized through one worker.
- **Export.** Frames are rendered in the renderer, read back, and copied over IPC
  in 4 MiB chunks to FFmpeg. A 4K frame is 33 MB. H.264 export of a trivial 4K
  project measures about 11 fps (`apps/desktop/docs/native-export-performance.md`).
- **Headless.** Headless rendering needs a whole browser. `powermove mcp` drives a
  Playwright-installed Chromium on SwiftShader (software WebGL), reads back 8-bit
  frames, and sends them over a WebSocket to FFmpeg. Each host runs one hidden
  editor with one open project and one export at a time. Agent frame captures
  pause the user's playback and fail while another capture or an export runs
  (`exporter.ts:856-870`), and agents now capture several frames on every turn.

## 2. Goals, non-goals, and the quality bar

### Goals

1. **Image fidelity.** Float precision from decode to encode.
2. **Color correctness.** Explicit, tested color invariants; per-source input
   transforms; optional linear-light blending; later OCIO, ACES, HDR.
3. **Native codecs** from the original media, hardware-accelerated where
   available.
4. **Exact time.** Rational frame rates, integer ticks inside the engine,
   frame-accurate decode, drop-frame timecode, sample-accurate audio.
5. **One renderer** for preview, export, thumbnails, agent snapshots, MCP, and
   the CLI, covering 2D and 3D.
6. **Proxies done right** (section 14).
7. **Speed.** Real-time multi-layer 4K playback on Apple Silicon, pipelined
   export, no rendering on the UI thread, concurrent agent renders.
8. **Keep what works.** The UI, the kernel and its frozen contracts, extensions
   and the Store, the 3D viewport editor, the agent harness and MCP contracts,
   Undo, the file format's durability.
9. **Portability.** Platform code behind traits; Windows is a port.

### Non-goals before cutover

- Rewriting the UI, timeline, inspector, 3D viewport editor, or kernel in Rust.
- Changing the document's JSON shape or the public API's units (section 6).
- New delivery formats beyond today's list; ACES, HDR and wide-gamut projects;
  scopes as a product feature; camera RAW; OpenFX; cameras and lights for 2.5D
  layers; new 3D authoring features (node editor, UV painting, physics);
  bundling Blender. These follow cutover (section 26).

### The quality bar

Each line is an automated test (section 25). CV means code value. The Phase
column says when the bar must pass.

| Area | Bar | Phase |
|---|---|---|
| Color invariants | (1) With no effects, a Rec.709 source exported to Rec.709 at the same bit depth and range matches a reference decode exactly at 32-bit precision and within 1 CV at 16-bit, with linear blending on or off. (2) A solid set to `#RRGGBB` exported to sRGB or Rec.709 decodes, with the tagged matrix and range, to the same codes within 1. (3) An export using the "Rec.709 (QuickTime and Safari)" output plays in QuickTime Player matching the viewer within ΔE2000 1.0 on a test chart, measured through ColorSync. | 2 |
| Pipeline transparency | At 32-bit precision, ProRes 4444 XQ decode → engine (no effects) → 16-bit TIFF RGB or `y416` matches a VideoToolbox reference decode exactly at 10-bit and within 1 CV at 12-bit. ProRes 422 sources through `v210` match exactly. | 2 |
| Codec generation | ProRes re-encode: PSNR against the source decode at least that of Apple's own re-encode of the same frames; maximum error at most 4 CV. | 2 |
| Bit depth | A 16-bit grey ramp across at least 1024 columns, exported to a 10-bit limited-range format with dither off, contains every luma code 64 to 940 (877 levels), is monotonic, and steps at most 1 CV. Full-range RGB outputs to 10-bit contain all 1024 codes. | 3b |
| Alpha | ProRes 4444 alpha round-trips within 1 CV at 12-bit; straight and premultiplied outputs are both correct. HEVC with alpha is measured by PSNR (Apple codes its alpha lossily). | 3b |
| SDR color | Decoded patch centers within 1 CV of analytic expectations; ΔE2000 maximum 1.0 and mean 0.5 at 8-bit, maximum 0.5 at 10-bit; checked with two references (FFmpeg with zimg, and AVFoundation). | 3b |
| Linear blending | With sRGB output, 50% white over black encodes to 0.7354 (188 of 255). A 1-pixel checkerboard downscaled 2× yields 188. A Gaussian blur of a white line conserves energy within 0.5%. | 3a |
| 3D | Native 3D layers match three.js r180 reference renders of the 3D corpus within the section 25 "3D shading" class. Blender surfaces enter the graph exactly as decoded, at 16 bits per channel or more. | 3a |
| Time | Frame math verified headlessly to 10⁸ frames at every rate. Exports carry a burned-in binary frame code: 10,000 consecutive frames per rate (23.976, 24, 25, 29.97 DF and NDF, 59.94, 60), plus 100-frame windows starting at N = 10⁶ and 10⁷. Drop-frame vectors: 29.97 frame 1800 = 00:01:00;02, frame 17982 = 00:10:00;00; 59.94 frame 3600 = 00:01:00;04. Invalid drop-frame strings are rejected. | 2 |
| Seeking | For every supported codec, including open-GOP HEVC, B-pyramid H.264, and files with edit lists, random access to frame N returns the pixels of sequential decode of frame N. | 3b |
| Audio | A click at frame N lands at sample floor(N × sr × den / num) ± 1. Total exported samples equal round(duration × sr). No length limit. | 3b |
| Antialiasing | Zone-plate and Siemens-star renders under animated scale 1.0 → 0.25 and rotation, compared with the path's declared kernel evaluated at 16× supersampling: PSNR ≥ 40 dB on export and paused frames (Lanczos), ≥ 34 dB during playback (trilinear); flicker energy at most 1.5× the reference. | 3a |
| Proxies | No exported frame contains proxy or reduced-quality cache pixels unless the user chose a draft render. A paused frame shows the original within 150 ms (4K ProRes) and 300 ms (4K long-GOP HEVC) on the base reference machine. Proxy and original at matched scale differ by at most ΔE2000 1.0. | 4 |
| Parity | Every corpus project renders in Standard color within the tolerances of section 25. | 3a |
| Determinism | Two renders of the same frame on the same machine produce byte-identical buffers and byte-identical lossless outputs; native and WASM evaluation are bit-identical. Blender-rendered layers are excluded; their surfaces are cached by content key and never re-rendered within a job. | 1 and 3a |
| HDR color | PQ and HLG outputs within ΔE_ITP (BT.2124) 1.0. | After cutover |

## 3. What exists today

Full inventories: appendices B and C.

### Code the engine replaces

Non-test lines at `d5d5b731`, excluding tracked `* 2.ts` duplicates.

| Area | Main files | LOC |
|---|---|---|
| Evaluation, animation, expressions, retiming, 3D math, text animators | `legacy/core/{anim,temporal-bridge,easing,expression,retiming,parenting,layer-groups,space-3d,text-animation,text-layout,split-text,vector-paths,content-properties,size-anchor,font-anchor,matte}.ts`, `core/anim/temporal-ease.ts` | 3,240 |
| Compositor, rasterizer, shaders, GL caches, asset import half of `raster.ts` | `legacy/gl/*` | 4,291 |
| 3D scene runtime, native 3D renderer, model import, Blender orchestration in the renderer | `core/scene3d/{runtime,rendering,service,layers,schema,recipe-preview,materials,import-model,import-bundle,migration}.ts`, 3D hooks in `frame-preparation.ts` | 2,520 |
| Media import, decode, posters, editing copies, preview caches | `legacy/core/{media,media-conversion,video-*,image-sequence,poster,preview-cache,preview-quality,memory,cloud-media}.ts`, `main/media-proxy.ts` | 1,907 |
| Playback loop | `legacy/core/engine.ts` | 503 |
| Audio and mixer engine | `legacy/core/audio.ts`, non-UI mixer | 1,825 |
| Export | `legacy/core/{exporter,frame-capture,frame-preparation,render-queue}.ts`, `main/render-encoder.ts` | 1,043 |
| Built-in GLSL effects and transitions, legacy mesh layers | `extensions/{effects-basic,transitions-basic}`, `layers-3d/{obj-model,studio-cube}.ts` | 510 |
| Other engine code | `legacy/core/{compositions,svg-import,fonts}.ts`, `kernel/{glsl,obj,extension-layers}.ts`, `captions/render.ts` | 1,545 |

About 17,400 lines, plus the parts of three.js r180 (MIT) whose rendering the
engine reproduces. The web player (`renderer/src/player/*`, 879 lines) runs the
same code (section 22). The Rust replacement will be larger, because it takes
over decode, color, text shaping, audio, and 3D work done today by the browser
and three.js.

### What stays

- The renderer UI, panels, Settings, and built-in UI extensions, including the
  bottom dock and the Track timeline.
- The kernel and its public API, extended as in section 20.
- The edit pipeline (`legacy/core/editing.ts`), history and Undo, the command
  vocabulary (25 commands), validation, and composition management
  (`legacy/core/compositions.ts`), with geometry calls redirected.
- **The 3D viewport editor** (`core/scene3d/{controller,modal,overlay,gizmo,navigation,viewport,operators,chrome,targets,editmode,editmesh,walk,editor-state,commands,theme}.ts`,
  `extensions/viewer/viewport3d/*`, the `extensions/layers-3d` UI) and the 3D
  document side (`core/scene3d/{operations,modeling,api}.ts`), re-pointed at
  `pm-wasm` geometry and engine frames.
- **The Blender host** (`main/blender-{service,worker,ipc}.ts`,
  `shared/blender.ts`, shipped in the api-pack), supervised for the engines
  (section 19).
- **The agent stack:** the harness, the orchestrator
  (`main/agent-orchestrator.ts`), the internal tool bridge, the public Powermove
  MCP (`server/mcp.ts`, `main/agent-tools/external-*`, `mcp-stdio.mjs`,
  `mcp-client.mjs`, `--powermove-mcp`), Claude Desktop registration
  (`main/claude-desktop-mcp.ts`), agent export delivery (`main/agent-export.ts`),
  and main-process media tools.
- The `.pmv` container and its durability, including read-only file saves
  (`main/project-save.ts:65-68,133`).

### Constraints inherited from earlier decisions

From `apps/desktop/docs/*` (including `SCENE3D.md` and `MCP.md`):

- One compositor for preview, capture, export, and nested comps.
- Pixel-exact acceptance for optimizations.
- Bounded expression interpreter (2,000 characters, 160 nodes, depth 24, 12
  arguments), no `'unsafe-eval'` in the editor.
- One user intent equals one Undo step.
- Effect contract limits: effects 32 params, 1 to 8 passes, fragment at most
  65,536 characters; extension layers 48 params, fragment at most 96,000 UTF-8
  bytes. GLSL ES 3.00 in projects and Store extensions keeps working.
- 3D limits: 512 objects, 16 lights, 4 shadow-casting lights, 500k vertices per
  scene, 2M imported vertices, 8 MB of scene JSON; Blender 4.5 or newer, inputs
  up to 256 MB, outputs up to 512 MB, 1.5M render vertices, 10-minute job
  timeout. Exports fail rather than deliver draft shading when a Blender surface
  is missing.
- Runtime rendering for agents uses the production renderer, never a parallel
  implementation of the document model (`server/mcp.ts:26-27`). The engine
  satisfies this: it is the production renderer, and the document model stays
  TypeScript.
- Agents never choose output paths; agent exports go to unique, owner-only files
  chosen by main (`main/agent-export.ts`).
- Per-layer failure isolation.
- GPU loss recovery without replacing the project; exports abort on loss without
  bad bytes.
- Predictability over raw speed.

## 4. Architecture overview

```
┌──────────────────────────── Electron app ──────────────────────────────┐
│  Renderer, per window (sandboxed, context-isolated)                     │
│  ├─ UI, kernel, extensions, edit pipeline, history (TS, seconds)        │
│  ├─ pm-wasm: canonical mirror, evaluator, 3D geometry, text layout,     │
│  │           edit kernels                                               │
│  ├─ Viewer: WebGPU canvas drawing engine frames + DOM/2D overlays       │
│  └─ 3D overlay context (editor chrome only, draws pm-wasm meshes)       │
│        ▲ MessagePort: canonical patches, control, events                │
│        │                                     ▲ VideoFrame               │
│  Main process                                │                          │
│  ├─ windows, files, save, media store, budget arbiter, agent bridge,    │
│  │  external MCP server, capability handles                             │
│  └─ pm-bridge (napi): Mach rendezvous, surface import ─┘                │
│        ▲ surface rights once; per-frame descriptors                     │
│  Interactive engine (utilityProcess, pm-engine via napi-rs)             │
│  ├─ sessions, pm-eval, pm-geom3d, pm-render (wgpu), pm-color,           │
│  │  pm-text, pm-vector, pm-media, pm-cache, pm-audio                    │
│  Job engine (second utilityProcess, same library)                       │
│  └─ exports, agent snapshots and exports, generated media, background   │
│     renders                                                             │
│  Blender worker (optional, user-installed; spawned and supervised by    │
│  main for the engines)                                                  │
└─────────────────────────────────────────────────────────────────────────┘
Other hosts of the same crates: pm-cli (headless renders, parity); the hidden
MCP host after Phase 3c (headless document host on pm-wasm + pm-node in one
Node process); pm-wasm in serve's document host.
```

1. **No engine work on a UI thread.**
2. **Synchronous answers stay synchronous.** About 150 built-in call sites, plus
   about 23 in the 3D viewport editor, ask for evaluated values, geometry,
   picks, bounds, or text layout synchronously, including 3D mesh raycasts,
   projected bounds, model-group bounds, camera-depth ordering, and Edit Mode
   vertex reads, some with a transient editor camera. `pm-wasm` answers from a
   mirror after flushing pending writes (section 6).
3. **Frames stay on the GPU.** A pool of shared surfaces imported once into main;
   one small message per frame (section 18).
4. **One renderer** for every output, 2D and 3D. three.js leaves the output path.
5. **Two engine processes.** A crash in one does not take down the other. Main
   arbitrates memory between them.
6. **Priority classes.** Interactive frames, then agent snapshots, then jobs.
   Agent captures queue instead of failing and never pause playback.
7. **Transport-agnostic protocol** over MessagePorts, in-process, or a socket.

### Crate layout

A Cargo workspace at `engine/`.

| Crate | Responsibility | Targets |
|---|---|---|
| `pm-time` | Ticks, rationals, frame math (i128, Euclidean division), timecode | native, wasm |
| `pm-model` | Canonical model, `v1_ingest`, id-keyed reconciliation, validation, scene3d and Blender schemas | native, wasm |
| `pm-eval` | Keyframes, ease, expressions, parenting, groups, 2.5D, retiming, text animator state, path and mask geometry, 3D layer content and channel namespaces, aim targets, recipe parameters, pick and bounds | native, wasm |
| `pm-geom3d` | Primitive, lathe, extrude, recipe-preview, and indexed meshes identical to today's; glTF 2.0 decode, skinning, morph targets, animation sampling; BVH raycast; bounds; depth blocks | native, wasm |
| `pm-edit` | Geometry-dependent edit kernels returning patches | native, wasm |
| `pm-text` | Font registry, subsetting, shaping, layout, outlines, caret geometry | native; wasm (layout) |
| `pm-vector` | Paths, trim, repeat, strokes, booleans, squircles, coverage | native; wasm (geometry) |
| `pm-color` | Transfer functions, tone mapping (ACES Filmic, AgX), working spaces, gamut mapping, OCIO bridge (after cutover) | native |
| `pm-gpu` | wgpu device, pools, shader compilation, GLSL compatibility, interop trait | native |
| `pm-render` | Render graph, 2D layers, 3D scene pass, effects, compositing, motion blur | native |
| `pm-blender` | Blender job client: snapshot from the evaluated scene, protocol, surface import, cache keys | native |
| `pm-media` | Probe, demux, decode, index, image readers, color metadata | native |
| `pm-cache` | Frame cache, render cache, disk caches, budget leases | native |
| `pm-proxy` | Representation policy, segment jobs, generated-media store | native |
| `pm-audio` | Device and clock, mixer, automation, stretch, waveforms, meters | native |
| `pm-export` | Encoders, muxers, render queue, verification | native |
| `pm-engine` | Sessions, scheduler, priority classes, playback, crash attribution | native |
| `pm-protocol` | Messages, MessagePack codec, TypeScript types via `ts-rs` | native, wasm |
| `pm-node` | napi-rs host for engine processes and the hidden MCP host | native |
| `pm-bridge` | napi-rs addon in main: rendezvous, surface import, font registry before Phase 2 | native |
| `pm-wasm` | wasm-bindgen exports | wasm |
| `pm-cli` | Headless binary and parity runner | native |

## 5. Time

### Inside the engine

- **Ticks.** `i64` ticks of 1/705,600,000 second (the flick).
  705,600,000 = 2⁹·3²·5⁵·7². Supported frame rates (23.976, 24, 25, 29.97, 30,
  47.952, 48, 50, 59.94, 60, 90, 100, 119.88, 120, 239.76, 240) and sample rates
  (8 to 192 kHz, including 44.1, 88.2, 176.4) divide it. Range about 414 years.
- **Rates** are `Rational { num, den }`.
- **Frame math** in i128 with Euclidean division (correct for negative times):
  - `frame_start(n) = ceil(n × den × TICKS / num)`
  - `frame_of(t) = (t × num).div_euclid(den × TICKS)`
  - Property test: `frame_of(frame_start(n)) == n` for every supported rate and
    arbitrary rationals, up to 10⁸ frames.
- **Media timebases** that do not divide the flick convert in i128 with
  round-half-even. The media index keeps native PTS.
- **Timecode** only at UI and IO edges, with SMPTE drop-frame for 29.97 and
  59.94.

### At the document boundary

The document and public API keep float seconds (section 6). `pm-model::v1_ingest`
converts in one place:

1. If `|t × fps − round(t × fps)| < 1e-4` against the owning composition's grid,
   convert by frame index: `frame_start(round(t × fps))` under the exact rational.
2. Otherwise convert to the nearest tick.
3. Rates come from `fpsRational` (new optional `[num, den]`) when present.
   Otherwise known NTSC floats map to their exact rationals; values within 0.1%
   of a supported rate snap to it; other values map to the smallest rational
   that reproduces the float within 1e-9.
4. Writing back: `pm-edit` echoes the original f64 for every time it did not
   change. Times it creates are `ticks / TICKS`.

### Rules

1. **One frame-rate validator** for the UI, `set_composition`, load, validation,
   Home's media matching (`matchSource`), and MCP `create_project` (Phase 1). It
   accepts rationals and stores `fps` plus `fpsRational`. New rates must have a
   frame duration that divides the flick; unusual rates already in projects keep
   working through the ceil and Euclidean math.
2. **Media-derived rates.** Composition matching uses the source's nominal rate
   (section 13: the most common frame duration snapped to a supported rate),
   never the raw average of a variable-rate file. Asset `fps` written at import
   today is ingested as a hint and superseded by the media index.
3. UI-created keyframes land on frame boundaries in composition time.
4. Nested compositions sample their children at the parent time by default.
   "Keep frame rate when nested" quantizes the child to its own rate.
5. Variable speed integrates in ticks at the owning composition's rate.
6. Export frame rate can differ from composition rate; each export frame
   evaluates at its exact tick.
7. Source frame selection: the last frame with PTS at or before the source time.
   Frame blending and optical flow come after cutover.
8. Motion blur uses shutter angle and phase. Today's default (0.5 frame,
   centered) maps to 180° at −90°.
9. Expressions see `time`, `t`, and `fps` as floats, plus a new integer `frame`.
10. **3D timing.** Shared-material channels (`m.*`, `shader.*`, `slots.*`) keep
    composition timing; their keys may lie before the layer's in point, down to
    −10⁸ s local. glTF animation clips evaluate at `max(0, t − in point)`. Recipe
    parameters (`model.*`) are evaluated per frame and regenerate geometry.

## 6. The document: ownership, writes, mirrors, engine facts

### Authority

The renderer stays the authority for the document and Undo. The JSON, the
api-pack types (`core/types/project.ts`, the scene3d and Blender schemas), the
kernel API, and `EditCommand`s keep float seconds and their shape.

Fields that already exist and are ingested as they are:

| Field | Purpose |
|---|---|
| Composition `render3d` | Native, EEVEE, or Cycles; samples, preview samples and scale, denoise, device |
| Layer `d.data.{object,light,camera,environment,generation,blender}` | 3D layer content |
| Group `d.modeling` | Generated model recipe, modifiers, animated parameters, part map |
| Asset `fps` | Average rate from MP4 `stts` (a hint; section 5) |
| Asset `format` | Model format: `obj`, `glb`, `gltf`, `blend` |

Additive fields before cutover:

| Field | Purpose |
|---|---|
| `fpsRational` | Exact composition rates |
| `minReaderVersion` (root) | Older builds refuse instead of overwriting (section 21) |
| `requiresEngine: "native"` (root) | Set when a project uses a native-only feature; the legacy engine refuses it |
| Layer `compat` | Compatibility level. Layers created before cutover have level 1 (legacy text metrics, solid corner radius ignored). New layers have level 2 |
| Asset `link` | Linked-media reference: path, volume identity, fingerprint, content hash, size, modification time, camera metadata |
| Asset `interpret` | User overrides of interpretation only |
| Project `color`, `media` | Project settings (sections 9 and 14) |

Not in the document: representations (proxies, optimized media), probe results,
measured decode cost, Blender availability. They live in the engine's media index
and generated-media store, keyed by content.

Removing root rotation and storing ticks on disk wait for `pm-doc` after cutover.

### The patch stream

1. The renderer produces leaf patches (`src/shared/patch.ts`) in canonical form
   (`PM.Comps.canonicalize` for every patch).
2. `pm-wasm` ingests them through `v1_ingest` and applies them with **id-keyed
   reconciliation**, so array replacements keep unchanged items' revisions.
3. `pm-wasm` emits canonical binary patches to the engine session. One
   translation point; both mirrors hash identically per revision.

### Making every write visible

Writers at `d5d5b731`: 20 direct `PM.hist.do` sites, 18 `api.history.do` calls,
8 `Edit.mutate` sites, 71 `PM.touch()` sites in core, and 23 `api.anim.touch()`
sites in built-ins (including `timeline.ts`, `track-view.ts`,
`timeline/index.ts`, `graph-controls.ts`, `viewer/path-editing.ts`,
`inspector/MasksSection.svelte`, `inspector/retiming.ts`). Also:

- asset metadata (`raster.ts:988,1008,1044,1117,1299,1336`)
- `shaderHooks.syncShaderUniforms` (`inspector/shader.ts:39-51`)
- extension-layer refresh (`kernel/install.ts:800-811`)
- the render queue's project swap (`render-queue.ts:10-12`)
- Home's composition matching (`legacy/app.ts:1338-1352`)
- Blender source assets added and removed outside history
  (`scene3d/modeling.ts:367,501`)
- agent media folder assignment (`assistant/media-management.ts:93`)
- shared-material fan-out across layers and compositions
  (`editing.ts:199-209`, `scene3d/materials.ts:270-282`)
- aim-preserving expression rewrites and key shifts (`parenting.ts:21-29`,
  `layer-groups.ts:124-226`)
- revision counters (`_scene3dRevision` advanced by `PM.touch`, `anim.ts:18`;
  `_blenderRevision`; `viewportRevision`)
- `api.host.pm` in user extensions

The mechanism:

1. **Write barrier.** Every write records its dirty path. In development and test
   builds, `PM.proj` is wrapped in a Proxy that records paths and throws on writes
   outside a known writer. In production, inventoried sites call
   `doc.write(path, value)` or `doc.touched(path)`. Multi-layer writes inside one
   command (material peers) record every peer path.
2. **Flush before query.** Every `pm-wasm` query drains dirty paths into patches.
3. **Edit kernels** in `pm-edit`: pose and aim preservation on reparent, group,
   and ungroup; model-group bounds from mesh geometry; align and distribute;
   anchor moves with font anchors; composition bounds; material peer
   synchronization. They return patches the TS edit pipeline applies.
4. **Shadow-replay auditor.** In test builds, the patch stream is also applied to
   a JS shadow copy. At frame boundaries, a sorted-key serialization of the shadow
   must equal a sorted-key serialization of `PM.Comps.canonicalize(PM.proj)`.
5. **Dual-answer mode.** In Phase 1 tests, every synchronous query is answered by
   both the legacy evaluator (and three.js for 3D geometry) and `pm-wasm`, and
   compared with the section 25 tolerances; text queries require identical line
   breaks and positions within 0.25 px.
6. **Resync.** A revision gap in production triggers a canonical snapshot.

Prior art: `renderer/src/host/remote-sync.ts`; `server/sessions.ts`.

### Engine-originated facts

| Fact | How it is committed |
|---|---|
| Probe results, duration, detected color and alpha | Not committed; read from the media index |
| Automatic interpretation | Not committed; overrides only are stored |
| Relink to a found file (content hash match) | Metadata write, not undoable, recorded in the edit log |
| Relink chosen by the user, Replace File…, Attach Camera Proxy… | Undoable user edit |
| Blender model generation or import result | Undoable user edit; dropped if the document revision changed while the job ran (as today) |
| Blender availability and version | Not committed; machine state |
| Font substitution | Not committed; shown as a warning until the user chooses |
| Asset or effect marked failed after crash attribution | Session state only |

## 7. Evaluation (`pm-eval`, `pm-geom3d`)

A pure function of (document, composition, time, optional view) to evaluated
state. It replaces `anim.ts`, `temporal-bridge.ts`, `temporal-ease.ts`,
`easing.ts`, `expression.ts`, `retiming.ts`, the evaluation parts of
`parenting.ts`, `layer-groups.ts`, `space-3d.ts`, `content-properties.ts`, text
animator state, path and mask geometry, and on the 3D side
`compositionScene` (`scene3d/layers.ts`), `evaluatedRecipe` and `animatedPart`
(`recipe-preview.ts`), `materialPreview`, and the three.js scene graph used for
picking and ordering.

- **Pure.** No global composition, time, or `PM.scope` stack.
- **Incremental.** Caches keyed by node id, input revision, and tick.
- **Expression dependencies** recorded per expression.
- **Temporal model** ported exactly: speed and influence, auto-bezier,
  continuous, hold, spring parameters (now agent-authorable through
  `CommandKeyframe.spring`), legacy easing presets, `offset_property`.
- **JavaScript semantics** where projects depend on them: loose `==`, string
  coercion, `Math.round` half toward +∞, `%` on negatives, `sin`-based `random`
  and `wiggle`, the float32 spring table, hex color byte interpolation. Each has
  a test against V8.
- **Text units.** Grapheme segmentation identical to `Intl.Segmenter`
  (`countTextUnits`), because the text-animation helpers are exported to
  extensions through `EDITOR_HELPER_EXPORTS` (`editor-helpers.ts:15-17`). Those
  helpers become shims over `pm-wasm`.
- **3D content.** The channel namespaces `m.`, `light.`, `camera.`,
  `environment.`, `shader.`, `slots.<id>.`, `model.`, and legacy `o.<id>.`;
  camera and light aim targets in parent space; shared materials with their
  timing rules; topmost active camera and environment.
- **3D geometry (`pm-geom3d`).** Primitives, lathe, extrude (bevel and earcut
  triangulation matching three.js), recipe previews, indexed and Edit Mode meshes,
  glTF decode with skinning, morphs, and animation sampling. BVH raycast picks
  within depth blocks honoring Local View and zero opacity; projected 3D bounds;
  model-group bounds in group space; camera-depth ordering of model blocks
  (`compositionOrderedLayers`); depth-block membership. Every query accepts an
  optional view camera that is not in the document.
- **Determinism.** Every transcendental function uses the `libm` crate on every
  target.
- **Geometry queries** for 2D: `pick`, `bounds`, `worldMatrix`, `localMatrix`,
  group and composition bounds, including text bounds.
- **JS fast path.** Numeric ids interned per property; results written into a
  shared `Float64Array`. 3D mesh buffers for the overlay are shared views into
  WASM memory, re-acquired after growth.

## 8. GPU and the render graph (`pm-render`)

### Device

- wgpu (30.x today), Metal on macOS, DX12 on Windows, Vulkan for Linux CLI.
  Pinned; upgrades are lanes, budgeted for roughly quarterly majors.
- One `pm-gpu` interop trait owns decoder output import, shared-surface export,
  encoder input, and Blender surface import.
- The engine's adapter matches Chromium's GPU process adapter (Metal
  `registryID`, or LUID on Windows).
- **Metal math mode.** Color-critical shaders are compiled with precise math
  through passthrough shaders, or wgpu is patched. Decided in Phase 0.
- **Watchdog and device loss.** A frame exceeding a time limit is cancelled; the
  offending effect is disabled for the session and reported. Device loss
  recreates the device and resumes.

### Working encoding and precision

| Blending | Intermediate encoding |
|---|---|
| Standard (linear blending off) | Premultiplied, display encoding E (section 9), Rec.709 primaries |
| Linear blending on | Premultiplied, linear light, Rec.709 primaries |
| After cutover: wide gamut, HDR, ACES | Premultiplied, linear light in the chosen working space; linear blending forced on |

Blending, mipmaps, resampling, and motion-blur accumulation use the active
encoding. Storage is `rgba16float` by default and `rgba32float` at 32-bit
precision (verified on reference machines in Phase 0). Source conversion shaders
compute in f32. Exports default to 32-bit when the output or any source exceeds
10 bits. A difference between preview and export precision is stated in the
export sheet.

### Render graph

- **Nodes:** source (decoded frame, vector, text, solid, shader output, 3D scene
  pass output, Blender surface), transform and resample, mask, effect pass,
  matte, blend, transition, motion-blur accumulate, precomp, adjustment, color
  transform, output.
- **Content keys** hash every quality-affecting input: evaluated parameters,
  source frame identity (asset, representation, source frame), color transforms,
  resolution bucket, resampler, motion-blur sample count, preview-quality state,
  view override, Blender version and settings for Blender surfaces, precision,
  and the engine build hash. A static layer renders once for a whole range.
- **Cache granularity.** Layer and precomp outputs cached on full extents;
  region-of-interest cropping applies to final output only.
- **Continuous rasterization for precomps.** A precomp renders at the maximum
  effective scale its parent chain requires, rounded up to a bucket, never
  upsampled. A known improvement; "Collapse transformations" follows cutover.
- **Region of interest** with effect padding (`viewportPadding`);
  bounds-expanding effects grow their buffers up to a limit.
- **Transient resources** from a per-frame aliasing allocator.
- **Isolation.** A failing node marks its layer failed and outputs transparency.

### Sampling and antialiasing

- Mipmapped trilinear during playback; bicubic or Lanczos (clamped negative
  lobes) for export and paused frames. Mipmaps in the active encoding.
- Analytic coverage antialiasing on quad edges; MSAA in the 3D pass.
- Vectors and text at output resolution.

### Motion blur

- Temporal supersampling with one project setting for sample count (Settings ›
  Project › Composition), used by every path, including Blender surface requests
  and RAM preview. Today the counts differ by path (appendix A).
- Accumulation in the active encoding at float precision.
- No adaptive sample reduction unless proven output-identical.
- Paused Blender "Rendered" preview shows no motion blur, as today.

### 2.5D and 3D

**2.5D parity:** the 3D switch, Z position, anchor and scale, X and Y rotation,
orientation, perspective per 3D group, homography planes, coplanar ordering,
painter sort.

**3D layers** (today three.js r180 in a second WebGL context):

- Each object layer gets its own color pass at output resolution, with MSAA,
  sharing a depth prepass and shadow maps with every model in its **depth
  block** (a contiguous run of 3D layers; a 2D layer ends a block). Only opaque,
  unmasked models write depth. The bottom layer of a block draws the camera's
  environment. Transparent models sort by camera depth. Model blocks are ordered
  by camera-space depth; 2D blocks keep painter order (`compositionOrderedLayers`).
- Shading parity target: `MeshStandardMaterial` with color, normal, roughness,
  metalness, emissive, and AO maps and UV placement; glTF source materials
  including the physical extensions three.js loads; hemisphere ambient; sun,
  point, and spot lights with PCF-soft shadows (1024², at most 4 shadow lights);
  rectangular area lights (LTC) without shadows; ACES Filmic with exposure;
  skinned and morph animation; perspective and orthographic cameras.
- Editor looks (Wireframe, X-ray, Local View, editor camera) are fields of the
  frame request and never honored for export or agent capture.
- In the engine, the **3D scene pass** writes each layer's premultiplied output
  directly into the 2D graph, so masks, effects, opacity, and blend stay per
  layer. No copies between contexts.
- **Blender surfaces.** When a composition has `render3d.enabled`, each 3D
  layer's pixels come from Blender as a source node with its crop rectangle
  (section 17 and decision 15). A missing surface fails an export.
- Legacy `mesh` layers (OBJ model, Studio Cube) keep their fixed shading.

After cutover: cameras and lights for 2.5D layers; intersections between 2.5D
planes and models; physically based changes behind `compat` level 2.

### Scopes

An internal waveform and vectorscope tool exists from Phase 3a for the QA lane.
Scopes as a product follow cutover. They read a full-precision working render of
the current frame.

## 9. Color pipeline

### The rule

sRGB and Rec.709 share primaries. By default they also share one **display
encoding E**, the sRGB piecewise curve. Code values in E pass through unchanged
in every configuration:

- **Standard** (linear blending off): intermediates are in E. Hex colors,
  pass-through footage, and today's look hold exactly.
- **Linear blending on:** inputs in E are decoded with E⁻¹, blended in linear
  light, and encoded with E on output. Pass-through and hex colors still hold.
- **Interpret** offers other readings of Rec.709 sources: "BT.1886 (gamma 2.4)"
  and "Scene-referred (inverse BT.709 OETF)".
- **Viewer:** working values are encoded with E and handed to the WebGPU canvas
  in `srgb` or `display-p3`; ColorSync maps them to the display.
- **QuickTime.** Export offers "Rec.709 (broadcast)" (tags 1-1-1) and "Rec.709
  (QuickTime and Safari)", whose tags make QuickTime match the viewer. S2
  determines the exact tags; bar (3) verifies them.

### 3D layers

- Native 3D shades in linear light and tone-maps with ACES Filmic (with exposure)
  to E before compositing, matching today's Native look.
- Blender renders through AgX to E, matching today's Blender look. The two curves
  differ by design today; parity keeps both (decision 17).
- Color and emissive maps are read as sRGB, data maps as linear.
- With linear blending on, tone-mapped 3D output is decoded with E⁻¹ like any
  E-encoded source.
- After cutover, scene-referred projects render 3D without per-layer tone mapping
  and take Blender output as linear EXR.

### Alpha

- Straight sources: input transform, then premultiply.
- Sources premultiplied in encoded space: unpremultiply (guarded for α < 1/4096
  at half float), input transform, re-premultiply.
- "Premultiplied with color" interpretation.
- Color-domain effects run on unpremultiplied values. Additive pixels (α = 0,
  RGB > 0) are never unpremultiplied.
- Output: unpremultiply, output transform, then straight, or re-premultiplied in
  the encoded domain for premultiplied outputs.
- Blender surfaces are straight alpha and premultiplied on input.

### Blend modes and mattes

Normal, add, multiply, lighten, darken run in the active encoding. With linear
blending on, screen, overlay, and soft light use HDR-safe formulas by default; a
per-project option runs them in E. Luma mattes use Rec.709 coefficients on
E-encoded values by default.

### Project color settings

Settings › Project › Color.

| Setting | Options | Default | Phase |
|---|---|---|---|
| Blend in linear light | Switch | Off (decision 1) | 3a |
| Precision | 16-bit float, 32-bit float | 16-bit float | 3a |
| Color | Standard (sRGB and Rec.709), Wide gamut (Display P3), HDR (Rec.2020 PQ), HDR (Rec.2020 HLG), ACES | Standard | Options other than Standard after cutover |
| Advanced › Working space | Linear Rec.709, Linear Display P3, Linear Rec.2020, ACEScg | Follows Color | After cutover |
| Advanced › Graphics white in HDR | 203 nits (BT.2408), custom | 203 nits | After cutover |

Changing a color setting changes the look and is one Undo step. Projects migrated
from today open with Standard color and linear blending off. The display choice is
a machine setting in Settings › General › Display.

### Per-source interpretation

"Interpret Footage…" on an asset or a multi-selection, as a Settings-style sheet.
Values are filled from metadata and show their origin.

Before cutover (Phase 3b):

- Color: primaries, transfer (E, BT.1886, scene-referred), matrix, range.
- HDR sources (iPhone HLG, HDR10, Dolby Vision base layer) in Standard projects
  are tone mapped to SDR by default with BT.2446 Method A.
- Alpha: none, straight, premultiplied, premultiplied with color; invert.
- Frame rate override, field order, pixel aspect ratio.
- Untagged sources: SD assumes BT.601; HD and above assume BT.709. Stills use
  their ICC profile through ColorSync.
- Superwhites and sub-blacks preserved; clamping only at output.

After cutover: camera log curves with gamuts, input LUTs, camera rules, "Show as
HDR values".

### After cutover: professional color

- OpenColorIO (2.6 at time of writing) through a thin `cxx` bridge with built-in
  ACES 2.0 configurations; graphics default to the "Graphics" input; OCIO GPU
  shaders as MSL where supported, checked against OCIO's CPU path.
- Wide-gamut and HDR projects; output gamut mapping.
- LUTs and color effects with a selectable domain.
- Gradients in Display, Linear, or OKLab.
- Legalizer, gamut warning, false color, zebras, scopes.
- HDR preview with EDR headroom from native screen APIs (prior art:
  `renderer/public/onboarding/hdr-output.js:93-132`); HDR export with measured
  MaxCLL and MaxFALL; HDR stills with gain maps; a native reference viewer.
- Unified 3D tone mapping under the project's color management.

### Output tags

| Output | CICP (primaries-transfer-matrix) | Range |
|---|---|---|
| sRGB images | 1-13-0, or ICC | Full |
| Rec.709 (broadcast) | 1-1-1 | Limited for video, full for images |
| Rec.709 (QuickTime and Safari) | Determined in S2 | Limited |
| After cutover: Display P3, Rec.2020 SDR, PQ, HLG | 12-13-1, 9-1-9, 9-16-9, 9-18-9 | Limited |

ProRes Y′CbCr is limited range with full-range alpha. MOV and MP4 write `colr`
`nclx` when full range must be signaled; WebM writes Colour elements; PNG writes
`cICP` or ICC.

## 10. Layer types

| Type | Today | Engine |
|---|---|---|
| `solid` | Uniform quad; `d.radius` ignored | Same; `radius` honored only on `compat` level 2 layers |
| `shape` primitives and paths | Canvas2D raster at quantized scale | `pm-vector` geometry at output resolution (renderer chosen in S4) |
| `text` | Canvas2D `fillText` | `pm-text` (section 12) |
| `captions` | Canvas2D | `pm-text` and `pm-vector`; word states in `pm-eval` |
| `image` | `ImageBitmap`, RGBA8 | Native-depth readers |
| `image` (SVG) | `<img>` per size bucket | usvg (text feature off) to vectors; SVG text through `pm-text`; filters through resvg on CPU |
| `video` | `<video>` or VP9 copy | `pm-media` decode of the chosen representation |
| `shader` | User GLSL, FBO capped at 4,096 px | GLSL compatibility pipeline; device limits only |
| `extension` fragment | Kernel-assembled GLSL | Same pipeline |
| `extension` mesh (legacy OBJ, Studio Cube) | Fixed WebGL shading, three.js picking | Same look; `pm-geom3d` picking |
| `extension` `layer3d` object | three.js pass in a second context, uploaded per layer per frame; or a Blender PNG surface | 3D scene pass; Blender surface source |
| `extension` `layer3d` light, camera | Not visual; topmost active camera supplies view and environment | Same, in `pm-eval` |
| `extension` `scene3d` (legacy, read-only) | three.js, one canvas per layer | 3D scene pass |
| `precomp` | Recursive render at its own size | Subgraph at effective scale, cached |
| `adjustment` | Effects on the accumulator | Same |
| `group` | Transform space or compositing boundary | Same |
| `group` with `d.modeling` | 3D model group; bounds from three.js geometry | `pm-eval` and `pm-geom3d` |
| `null`, `audio` | Not visual | Not visual |

## 11. Effects, transitions, and the shader contract

### Built-in effects

The 16 effects, 4 transitions, 5 shader presets, and 2 GLSL or mesh extension
layers are re-implemented in WGSL with fixes (correct Gaussian at every radius
with a downsample pyramid; Glow and Drop Shadow on it; declared `colorDomain`
and bounds expansion). In Standard color they reproduce today's output within
parity tolerances, except fixes listed in appendix A.

A user fork of a built-in extension (forks of `effects-basic` and `layers-3d`
update by local merge, `main/extensions/update-fork.ts`) registers its own
definitions under the built-in ids. The engine uses the fork's definition
snapshot, never the WGSL built-in.

### The GLSL contract stays

Projects, Store extensions, agent-written extensions (including those an agent
writes from imported After Effects scripts), and web exports carry GLSL ES 3.00
with Powermove's preambles, `u_<param>` uniforms, and `u_p<i>` and `u_c<i>`
aliases.

**No shader is dropped.** Every known GLSL source (the repository, the Store, the
docs samples, and agent-written local extensions) must compile and match its
WebGL output within the parity tolerances before cutover. A failure is fixed in
the pipeline, never by removing or rewriting the user's shader.

1. **Wrap** with the existing generators.
2. **Normalize:** `#version 300 es` → `#version 450`; remove precision
   qualifiers; inject `pm_FragCoord` in place of `gl_FragCoord`.
3. **Compile** with glslang using relaxed Vulkan rules and automatic binding and
   location mapping.
4. **Split combined samplers** at the SPIR-V level.
5. **Translate:** parse SPIR-V with naga ourselves and validate with
   derivative-uniformity checks off (`ShaderSource::Naga`). S3 compares this with
   SPIRV-Cross to MSL through passthrough shaders and picks the more robust one.
6. **Cache** by source hash, backend, and driver version.
7. **Errors** map back to the user's line numbers.

**Origin and sampling.** Legacy shaders see GL conventions (bottom-left origin,
`dFdy` as in WebGL). Sampler state per effect type is recorded and reproduced.

**Color domain.** Legacy GLSL receives E-encoded values, independent of the
user's display. Values above 1.0 pass through by default; an effect may declare
`clamp: true`.

### Definition snapshots

Effects, transitions, and extension layers are registered at runtime by
JavaScript; projects store only definition ids. The engine needs definitions
without JavaScript (job engine, `pm-cli`, the hidden MCP host):

- The kernel pushes a **definition snapshot** to each session:
  `{id, version, rendererKind, role, glsl?, params, flags}`.
- `layer3d`, `scene3d`, and `mesh` are engine-owned renderer kinds and carry no
  GLSL. Blender shader graphs are project data (`shared/blender.ts`).
- UI-only fields (`ui`, `advanced`, `options`, widget hints) are stripped.
- Export jobs and Collect Project embed the snapshots they use.
- `pm-cli` renders from embedded snapshots, or fails naming the missing
  extension.
- The `raster` service override produces pixels in JavaScript. Projects using it
  export through the interactive engine with a renderer round trip and are marked
  not headless-renderable. At apiVersion 4 the override is deprecated.

### Contract v2

A WGSL contract for new extensions (apiVersion 4): explicit `colorDomain`, bounds
expansion, multiple inputs, compute passes. GLSL stays supported.

## 12. Text

- **Stack.** Parley (layout), HarfRust (shaping), Skrifa (outlines, metrics).
  Glyph rendering chosen in S4.
- **Font registry.** One native authority enumerates system fonts, loads project
  fonts, resolves fallback, and ships a fallback manifest. Before Phase 2 it runs
  in main through `pm-bridge`; from Phase 2 in the engine. The UI font picker reads
  it.
- **Fonts in `pm-wasm`.** Metrics-and-shaping subsets per font (`cmap`, `hmtx`,
  `hhea`, `OS/2`, `head`, `name`, `GSUB`, `GPOS`, `GDEF`, `kern`, `morx`, `kerx`,
  `trak`, `fvar`, `avar`, `HVAR`, `MVAR`, synthesized `HVAR` where missing); no
  outlines or bitmaps (Apple Color Emoji alone is about 200 MB). Served lazily by
  main within a per-window budget.
- **Compatibility (level 1 layers).** Greedy word-then-character wrapping when
  `paragraph` is set and `boxWidth > 0`, UTF-16 character splits, baseline at
  `size × 0.82` (`raster.ts:214,264,343`), line height `size × leading` (default
  1.15), kerning through measured widths, CoreText's `trak` and optical sizing,
  hidden system fonts. Gate: identical line breaks and advances within 0.25 px.
  Level 2 uses grapheme-aware breaking and font metrics.
- **Animators.** Stagger, range, wave on characters, words, lines; per-glyph
  properties. The 13 inspector presets stay UI-side.
- **Editing.** Caret, selection, and font offset from `pm-wasm`; native and WASM
  layout identical.
- **Rendering quality.** Coverage gamma or stem darkening tuned to match
  CoreText.
- **3D Text** models are generated by Blender's built-in font and stored as
  meshes, as today. Generating them from `pm-text` outlines follows cutover.
- **Gates.** CoreText side-by-side corpus reviewed by Jude; SSIM ≥ 0.98 against
  today; canvas typing latency (section 25).

## 13. Media I/O (`pm-media`)

### Principles

1. The original is the truth. Import never replaces it.
2. Every source is indexed for exact, fast random access.
3. Frames reach the GPU at native depth with color metadata.

### Backends

- **VideoToolbox** for ProRes, H.264, HEVC, and AV1 where hardware exists.
  Outputs: ProRes 422 → `x422`; ProRes 4444 and XQ → `y416`; 10-bit HEVC and
  H.264 → `x420` or `x422`; 8-bit → biplanar 8-bit.
- **Metal import.** Biplanar buffers as per-plane `R8`/`RG8` or
  `R16Unorm`/`RG16Unorm` textures via `CVMetalTextureCache`; `y416` as one
  `RGBA16Unorm` texture with an AYCbCr swizzle. For `x420` and `x422` only,
  MSB-aligned samples are normalized (`v >> 6`) before limited-range scaling.
- **FFmpeg** (our build, section 24) through `ffmpeg-the-third` or our own bindgen
  crate, as dylibs: demuxing, DNxHD/DNxHR, CineForm, MJPEG, DV, VP8 and VP9
  (libvpx for alpha), AV1 (dav1d), DPX, software fallbacks, all audio decoding.
- **Images.** ImageIO on macOS; the `image` crate; OpenEXR's C library; usvg.
- **3D models.** The `gltf` crate; the existing OBJ parser ported; `.blend` only
  through the Blender worker.

### Formats at cutover

| Category | Formats |
|---|---|
| Intermediates | ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ; DNxHD; DNxHR; CineForm |
| Camera and delivery | H.264 (including 4:2:2 10-bit, software-only), HEVC 8 and 10-bit, HEVC with alpha, AV1, MJPEG, DV |
| Web | VP8, VP9 with alpha |
| Containers | Today's list (MOV, MP4, M4V, MXF, MKV, WebM, AVI, WMV/ASF, FLV, MPEG-PS, TS, MTS/M2TS, 3GP, OGV, VOB) |
| Images | PNG 8/16, JPEG, TIFF 8/16/32f, OpenEXR, DPX, HEIF/HEIC, AVIF, WebP, BMP, ICO, GIF, SVG |
| Sequences | Any image format, gaps reported |
| Animated images | GIF, APNG, animated WebP and AVIF |
| Audio | WAV, AIFF, CAF, MP3, AAC/M4A, FLAC, Opus, Vorbis, WebA, audio in every container |
| 3D models | GLB and self-contained glTF 2.0 (embedded resources; skins, morphs, animations; no Draco or meshopt; up to 128 MB), OBJ with MTL and textures (packed to GLB on import), `.blend` (Blender 4.5 or newer; packed textures; up to 256 MB) |

After cutover: camera RAW, PSD layers, multi-part EXR, FBX.

Hardware notes for the proxy policy: base M1 has no ProRes engine; AV1 hardware
decode starts at M3; H.264 4:2:2 10-bit is software-only.

### Probe and index

In the background on import:

1. Streams, including timecode, reel, UMID or content identifier.
2. Color: primaries, transfer, matrix, range, mastering metadata, ICC, HDR format.
3. **Frame index** from container tables; scanning only where none exist.
   Open-GOP CRA and RASL frames marked not cleanly seekable.
4. **Timing:** edit lists, composition offsets, negative CTS, stream start
   offsets, AAC and MP3 priming. This replaces `shared/mp4-frame-rate.ts`, which
   ignores edit lists and reports a variable-rate file's average rate.
5. **Nominal rate:** the most common frame duration snapped to a supported rate.
6. **Content hash:** full BLAKE3 in the background; today's sampled fingerprint
   (`media.ts:153-171`, which can collide) remains a lookup hint. Files still
   being written wait.
7. **Decode cost** on this machine.
8. Poster, filmstrip seed, waveform peaks.

### Variable frame rate and interlacing

- VFR sources are sampled by timestamp; nominal rate for snapping and stepping;
  generated media preserves the original PTS exactly.
- Interlaced sources deinterlaced (bwdif) by default. Pulldown removal follows
  cutover.

### Decode service

Sessions per source and playback instance, pooled and budgeted; hardware output
imported without CPU copies; scheduling by direction, speed, read-ahead, and GOP.

## 14. Proxies, optimized media, and caches

### 14.1 What users should feel

Footage always plays without setup. Professionals find every control After
Effects and Final Cut Pro offer, in plain language. The viewer always says what
is on screen. Exports never contain proxy pixels by accident. Controls and copy
are in appendix D.

### 14.2 The representation ladder

From no generation to most generation:

1. **Original** at full resolution.
2. **Reduced-resolution decode** with no files (ProRes partial-resolution decode,
   ImageIO downsampling, EXR mips).
3. **Proxy segments.**
4. **Optimized segments**, full resolution, intra-frame.
5. **Kept renders**, pinned precomp renders.

Image sequences get proxy segments; large stills get a tiled mip cache. Large 3D
models get decimated preview meshes only after cutover.

### 14.3 Segments, addressed by content

- ~2-second segments aligned to source keyframes, each a QuickTime file, one
  segment index per source.
- Keyed by (content hash, recipe, segment index); segments re-verify the source's
  size, modification time, and inode.
- Only used ranges plus 2-second handles are generated; jobs resume; availability
  per frame; shared across projects; atomic writes.
- Gate: zero drops across segment boundaries with 4 streams.

### 14.4 Recipes and fidelity

| Purpose | macOS recipe |
|---|---|
| Proxy | ProRes 422 Proxy (10-bit) by default; ProRes 422 LT or HEVC 10-bit by choice. Sources with alpha always get ProRes 4444 at proxy size |
| Optimized | ProRes 422 HQ; ProRes 4444 for 4:4:4, above 10-bit, or alpha. Created only when asked by default (decision 4) |

Generated media keeps the source encoding, is 10-bit, and keeps alpha, PCM audio,
timecode, reel name, pixel aspect, frame count, and exact PTS, in the original's
coordinate space. Analysis always uses originals.

### 14.5 The automatic policy

A **machine profile** measures decode cost, GPU headroom, disk throughput, and
power state. The project stores only intent.

1. Best representation that sustains real time at the viewer's device-pixel size
   for the number of streams.
2. During playback, switch only at clip boundaries or when playback starts.
3. Paused with Resolution set to Automatic: show the original as soon as it
   decodes, in place. A hand-set Resolution is respected when paused.
4. Never wait for generated media; use the best ready representation.
5. Re-evaluate on power-state changes.

Generation runs in the job engine at low priority, pauses during playback and
export, and goes clip-near-playhead first. A **disk guard** pauses generation below
the larger of 20 GB or 10% free.

### 14.6 Render cache and kept renders

- Node outputs cached in GPU memory and on disk (half float, lossless compression
  per tile).
- A 2-pixel track under the timeline ruler shows cached frames (memory lighter,
  disk stronger); View › Show Rendered Frames.
- Render Work Area, Render Selection, Delete Rendered Frames; Keep Rendered on
  precomps; background rendering when plugged in.
- **Export safety.** Export reads only cache entries whose keys match its own
  quality inputs (original representation, export resampler and motion-blur
  samples, export precision, full quality, Blender final settings rather than
  preview samples or scale, same engine build). Gate: fill the cache at
  Automatic, Half, Quarter, with proxies and Blender preview settings, during a
  sustained-load drop; then export; byte-identical to an empty-cache export.

### 14.7 Caches

| Cache | Contents | Key | Location | Eviction |
|---|---|---|---|---|
| Frame cache | Decoded frames | Source, representation, frame | Memory | Lease, LRU |
| Render cache | Node outputs | Section 8 key | GPU memory, disk | Lease, size limit |
| Blender surfaces | Per-layer cropped surfaces | Snapshot hash, Blender version and settings, size | GPU memory, disk | Lease; pinned for active jobs |
| 3D geometry | Decoded glTF, generated meshes, BVHs, GPU buffers | Asset hash or source signature | Memory | Lease |
| Media index | Frame indexes, probe, content hash, decode cost | Content hash and file identity | App cache | Rebuilt on demand |
| Thumbnails, filmstrips, posters, waveforms | Derived images and peaks | Content hash, recipe | App cache | Size limit |
| Generated media | Proxy and optimized segments | Content hash, recipe, segment | Location setting | Storage limit; pinned never |
| Large-still mips | Tiled mip levels | Content hash | App cache | Size limit |
| Shader cache | Compiled pipelines | Source hash, backend, driver | App cache | Version change |
| Font subsets | Subsets for `pm-wasm` | Font file hash | App cache | Size limit |

These replace today's IndexedDB media store, editing copies (now also made for
image sequences over 1920 px), the decoded audio cache, raster and texture caches,
the GL program cache, the renderer's Blender frame cache (96 frames, 512 MiB,
invalidated by any edit), three.js scene runtimes, and the recipe-preview cache.
Today's `memory.ts` budgets and `previewMemoryMiB` map to engine leases.

### 14.8 Offline media and exports

- Interactive export with offline originals: a native three-button message
  (appendix D).
- Queued jobs never fall back to proxies; they wait.
- Draft renders from proxies are an explicit option.
- A composition that renders with Blender cannot export on a machine without
  Blender; the export sheet and Collect Project say so.

## 15. Playback

- **Master clock:** the audio device clock, including device latency.
- **Render-ahead** into an adaptive ring; decoders read ahead per GOP and speed.
- **Presentation.** Frames carry PTS and revision; a jitter buffer presents on
  `requestAnimationFrame` by PTS. Scrub and gesture frames bypass it.
- **A/V sync** measured and offset. Gate: ±10 ms.
- **Drops** counted, attributed, reported.
- **Quality ladder.** Under sustained load, Automatic lowers resolution and moves
  clips down the representation ladder.
- **Priority classes.** Agent snapshots and MCP renders never interrupt
  interactive playback (today they pause it).
- **3D.** Blender shading is paused-only, as today; playback shows native 3D. In
  the editor view, frames render from the transient editor camera (section 18).
- **Transport.** Work-area loop, frame stepping; J/K/L at 1× and 2× forward and
  reverse, play around current frame, audio scrub (new before cutover).
- **Cached preview** through Render Work Area.

## 16. Audio

- f32 mixing at the project rate (Settings › Project › Composition), `rubato`,
  `cpal`.
- FFmpeg decode only, streaming, no length cap, priming honored.
- Clip voices → per-layer strip → precomp nesting → master; channel layouts
  preserved and down-mixed.
- Per-sample automation.
- Audio follows speed and remaps; pitch preserved by default (Signalsmith Stretch
  through our own binding), or follows speed per clip; deterministic pre-roll.
- Frame-rate override keeps audio in sync.
- Engine waveforms for the timeline, Assets panel, and `drawWaveform`.
- Mixer meters, solo, retune from the engine.
- Export: PCM 24-bit or 32-bit float for ProRes and WAV; AAC through AudioToolbox
  with `edts` and priming; TPDF dither when reducing to 16-bit; stereo as today.
- After cutover: reverse playback, channel mapping on export, pan, EQ,
  compressor, limiter, loudness normalization.

## 17. Export and delivery

### Pipeline

```
evaluate → [Blender stage] → render (float) → output transform
  → encoder input conversion (engine shader) → encode → mux → verify → move
```

- Runs in the job engine from a document snapshot with definition snapshots.
  Users keep editing; edits never invalidate an export's prepared surfaces.
- **Pipelined stages** with bounded queues.
- **Blender stage.** For compositions that render with EEVEE or Cycles, the job
  engine builds Blender snapshots from `pm-eval` and `pm-geom3d` (not from a
  three.js graph), requests surfaces per frame, per shutter sample, and per nested
  composition at its output size, ahead of compositing, with a bounded queue.
  Today each 3D layer is a separate Blender render with holdouts, so cost scales
  with layers × samples; the engine runs several workers when memory allows and
  asks Blender for per-layer passes from one render where possible. Surfaces are
  16-bit PNG through AgX at cutover (decision 15). Cancel stops Blender. A missing
  surface fails the job. Cycles `device: auto` is resolved and recorded per job so
  output is reproducible on the same machine.
- **Encoder input.** The engine converts RGB to Y′CbCr itself and hands
  VideoToolbox native buffers with color attachments set.
- **Dither** only when the working precision exceeds the output precision and the
  output is 10 bits or fewer; never on alpha; off for invariant and transparency
  tests.
- **Muxers.** AVAssetWriter for MOV and MP4; FFmpeg for WebM. NTSC timescales are
  24000, 30000, or 60000.
- **Atomic output** via a temporary file in the destination folder.
- **Verification** probes codec, profile, pixel format, depth, tags, range, frame
  count, duration, audio samples, alpha, and sampled pixel values.
- **Comfort.** Prevent sleep; completion notification; size estimate; overwrite
  policy; file name tokens; start timecode and reel name.

### Formats at cutover

Today's list, from originals, at correct depth and tags:

| Format | Encoder | Notes |
|---|---|---|
| ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ | VideoToolbox | Today only 4444 is offered; the others come with VideoToolbox. Alpha straight or premultiplied |
| H.264 (MP4) | VideoToolbox | A VMAF comparison against x264 decides whether a software encoder is also offered, subject to licensing |
| HEVC (MP4, MOV) | VideoToolbox | 8 or 10-bit SDR; alpha at 8-bit (confirmed in S2) |
| WebM VP9 and VP8 | libvpx | Frame-exact; Colour elements |
| PNG sequence and still | Engine | 8 or 16-bit, alpha |
| Code export, project export, captions | Sections 22 and 21, as today | — |

After cutover: DNxHR/DNxHD, AV1, EXR (DWAA through OpenEXR), DPX, TIFF
sequences, GIF, animated WebP, audio-only, HDR outputs, ACES deliverables,
interlaced output, non-square pixels, FCPXML and OTIO.

### Agent delivery

`export_video` and `save_project` keep their contract (`main/agent-export.ts`,
`assistant/mcp-project-tools.ts`): automatic unique names in `~/Powermove` (or the
hidden host's `--exports` folder), no dialog, owner-only files, empty files
removed, the real path returned only after encoding and verification. In the
engine they become render-queue jobs with a main-minted output handle. Several
agent exports per host may run at once, alongside user exports and editing.

### Presets and the render queue

- Presets: "Master · ProRes 422 HQ", "Master with alpha · ProRes 4444",
  "Web · H.264", "Web · HEVC", "Image sequence · PNG". Existing presets and
  per-project defaults migrate.
- **Render Queue panel.** Today nothing adds jobs, each job prompts for a
  destination, a failure stops the run, the run swaps the live project, and agent
  `export_video` bypasses it. New: jobs come from the export sheet and from agents
  with their destination; they run unattended in the job engine from snapshots
  without touching the live project; failures are per job; jobs persist across
  relaunch.
- Export sheet and queue reuse the Settings sheet cards and rows; the progress
  sheet keeps its live preview with small frames streamed from the job engine.

## 18. Viewer integration and the frame bridge

### The bridge

Proven by spike S1 before any viewer work.

1. **Rendezvous.** `pm-bridge` in main registers a Mach receive right under a
   per-launch random bootstrap name, passed to the engine through the
   `utilityProcess.fork` environment; main checks each message's audit-token PID.
   On Windows, `DuplicateHandle`.
2. **Pool, imported once.** IOSurface-backed textures at viewer device pixels.
   Pool size: maximum render-ahead ring + jitter-buffer depth + 2. Formats limited
   to Electron's import formats: `bgra` (8-bit, blue-noise dither in the display
   transform) or `rgbaf16`, chosen in S1 by banding and memory. Each surface is
   sent once as an `IOSurfaceCreateMachPort` right and imported once with
   `IOSurfaceLookupFromMachPort` and `sharedTexture.importSharedTexture`.
3. **Per frame:** `{poolIndex, seq, pts, revision, colorSpace, overlayGeometry}`
   after the command buffer completes. Main calls `sendSharedTexture` to the window
   hosting the viewer.
4. **Release per frame.** `VideoFrame.close()` and the imported texture's per-use
   release callback (or release sync token, per Electron 44's typings) forward
   `{poolIndex, seq}` to the engine. `allReferencesReleased` only for teardown. S1
   confirms the exact API.
5. **Renderer.** `setSharedTextureReceiver` in the preload's isolated world; S1
   proves the `VideoFrame` reaches the main world. Drawn with
   `importExternalTexture` or `copyExternalImageToTexture`.
6. **Overlays.** The DOM and Canvas2D overlay stays. `overlayGeometry` carries
   world matrices, bounds, and paths of selected and hovered layers and the text
   caret at the frame's revision; during gestures overlays draw from it. The **3D
   overlay** (grid, outlines, light and camera wires, gizmos, Edit Mode points)
   keeps its own WebGL context in the renderer and draws `pm-wasm` mesh buffers at
   the frame's revision. Hit-testing uses `pm-wasm` at the current revision.
7. **Capture.** Frames are inside Chromium's compositor, so `capturePage` keeps
   working.

### View override

The viewer sends size, DPR, zoom, pan, resolution, and a **view override**:
editor camera matrix and projection, shading (Wireframe, Solid, Material Preview,
Rendered), X-ray, and Local View ids. The override changes preview pixels without
touching the document and is ignored for export and agent capture.

### S1 gate

At 4K 60 fps on the reference machines, with main under load (autosave of the
largest corpus project, agent tool traffic, a 50 ms long task every second,
window resize):

- p99 presentation jitter at most 4 ms, zero drops over 60 s;
- layer drag, 3D orbit, and walk pointer-to-pixel at most 33 ms at 60 Hz, no more
  than 1 frame worse than today;
- a ramp read back through `capturePage` matches; a 10-bit ramp shows no banding;
  values above 1.0 survive on an HDR display (informs after-cutover HDR).

### Fallbacks, ranked by S1 data

- **A′.** Engine in main presenting through `CAMetalLayer` from a Rust render
  thread.
- **C′.** Remote layer (`CAContext` and `CALayerHost`) from a utility process.
- **B.** Shared-memory CPU frames at reduced preview resolution.

### Viewer behavior

- The engine renders only the visible region.
- During pan, zoom, and orbit the last frame is reused until the engine catches up.
- `frame:rendered` and `overlay` fire on presentation with the frame's revision;
  `anim.version()` comparisons with `frame:rendered.version`
  (`viewer.ts:752,941`) keep working.

## 19. Process model, protocol, security, packaging

### Processes

| Process | Runs |
|---|---|
| Main | Windows, files, save, agent bridge, external MCP server, media store, capability handles, budget arbiter, queue persistence, Blender supervision, `pm-bridge` |
| Renderer, per window | UI, kernel, extensions, edit pipeline, `pm-wasm`, 3D overlay context |
| Interactive engine (utilityProcess) | GPU device, decoders, audio device, caches, sessions |
| Job engine (utilityProcess) | Exports, agent snapshots and exports, generated media, background renders |
| Blender worker (optional, user-installed) | Model generation, `.blend` import, Blender renders; spawned by main, `--background --factory-startup --disable-autoexec`, requested by the engines through main |
| External MCP server (in main) | Loopback JSON-line server; credential in `<profile>/mcp/connection.json` (owner-only); one session per connection |
| MCP stdio relays | `Powermove --powermove-mcp` and `powermove mcp --connect`; unchanged |
| Hidden MCP host (`powermove mcp`) | Today: Node serve host plus Playwright Chromium on SwiftShader. After Phase 3c: headless document host on `pm-wasm` plus `pm-node` engine sessions; Chromium only when a panel tool is called |
| Extension sandboxes, transcription, agent CLIs | Unchanged |

Today no code uses `MessageChannelMain`. Main creates one per window and engine.
Engine ports are never given to extension sandbox frames.

### Memory

Main is the **budget arbiter**: it grants each engine leases from one global
budget, shrinking the job engine's lease during playback. Each engine enforces its
lease across frame cache, render cache, Blender surfaces, 3D geometry, surface
pool, and decoders. `pm-wasm` has a per-window budget including 3D meshes and
BVHs. Blender's own memory is outside leases; the gate accounts for it. Gate:
export plus playback on the base 8 GB machine for 10 minutes without
memory-pressure warnings, including a run with a Blender export in progress.

### Crash handling

- Main restarts a crashed engine with exponential backoff; renderers re-send
  snapshots; the viewer shows "Reconnecting…".
- **Attribution.** Two crashes attributed to one asset or effect mark it failed
  for the session.
- Queue, generation, and Blender jobs persist in main and resume.

### Protocol

- `pm-protocol` messages, MessagePack, TypeScript types from `ts-rs`, checked in
  CI. Version negotiation at connect.
- Bulk data through shared surfaces or cache files.
- Groups: session, transport, viewer (including view override), render, media,
  proxy, export and queue, audio, definitions, Blender, engine stats.

### Security

- The engine accepts only capability handles minted by main, never raw paths.
- **Agent paths.** Paths supplied by agents (`import_media`, `replace_media`,
  `import_media_to_timeline`) are validated in main (absolute, regular file,
  allowlisted type) and turned into read handles. Export and save outputs are
  write handles from main's automatic destination. Agents never choose output
  paths. Linked media lifts today's 512 MiB agent import cap.
- **Blender** parses untrusted `.blend` files: it runs with a scratch home
  directory, no network, scripts disabled, and capability-scoped files; the job
  validator is fuzzed.
- Decoders run in engine processes isolated from UI and main.
- Fuzzing: protocol decoder, patch ingest, project parsing, demuxers, readers,
  glTF decoding.

### Packaging and CI

- Pinned Rust toolchain; `aarch64-apple-darwin` first.
- **Self-hosted Apple Silicon runners**: reference machines for parity and
  performance jobs, isolated from secrets, corpus outside the repository, with a
  pinned Blender LTS for 3D gates.
- `pm-node` and `pm-bridge` are napi-rs addons in `asarUnpack`, signed with the
  Team ID and hardened runtime; `apps/desktop/scripts/release-mac.mjs`
  verification generalized.
- **`packages/cli`** (`powermove-cli`, npm, macOS and Linux) ships `pm-node`
  prebuilds for darwin-arm64 and linux-x64 and linux-arm64 from Phase 3c, and
  drops the Playwright dependency for rendering.
- FFmpeg, glslang, OpenEXR built from source at pinned versions; FFmpeg as dylibs;
  `ffmpeg-static` removed from the app and the CLI.
- `pm-wasm` built with the wasm-bindgen CLI; the editor CSP gains
  `'wasm-unsafe-eval'` for the app origin (`main/security-policy.ts:9`), also
  served by `powermove serve` (`server/index.ts:159`).
- Minimum macOS follows Electron 44's.
- **Crash and session reporting:** opt-in, with a privacy note (decision 10).

## 20. Kernel API, extensions, agents, and MCP

### API mapping

| API | After cutover |
|---|---|
| `project.get`, `latest`, `apply`, `time`, `play`, `undo` | Unchanged (seconds) |
| `project.snapshot(t, maxWidth)` | Engine render, same return type |
| `render.snapshot(time, maxWidth): string` (sync) | Engine-filled snapshot cache; `''` while rendering. Built-ins move to `project.snapshot` |
| `anim.ev`, `evP`, `active`, `worldMatrix`, `localMatrix`, `resolveContent`, `findProp`, `allProps`, `hasKeyAt`, `wouldCycle`, `transformParentMatrix` | `pm-wasm`, synchronous, same signatures, including 3D namespaces |
| `anim.version()`, `anim.touch()`, `anim.expressionErrors`, `anim.setKey` | Kept; through the write barrier |
| `model.*` | Unchanged |
| `groups.bounds`, `space3d.*` | `pm-wasm` |
| `transport.*` | Engine transport and stats |
| `media.importFiles`, `assets.*` | Engine import through main. `assets.get` live fields (`url`, `el`, `audioBlob`, `peaks`, `audioDecodeError`, `object3d`) become deprecated compatibility shims backed by engine data |
| `media.timing.*`, `media.assets.kind`, `media.fonts` | Kept; fonts from the registry |
| `media.audio.drawWaveform` | Same, engine peaks |
| `render.gl.bounds`, `pick`, `previewViewport` | `pm-wasm`; for 3D layers, mesh raycasts and projected bounds with the editor camera and Local View |
| `render.gl.init`, `resize` | Viewer-internal; mapped to the bridge |
| `render.gl.compileError` | Engine compiler, mirrored |
| `render.gl.context` | The compositor's WebGL context no longer exists. Phase 0 audits the Store and known local extensions for use; any user gets an equivalent path through extension layers before cutover. Then `null` with a deprecation warning; the two built-in call sites are rewritten |
| `render.raster()` | Metrics from `pm-wasm`; pixels deprecated |
| `render.renderFrameTo()`, `render.textLayout` | Async engine render; `pm-wasm` |
| `scene3d.edit`, `describe`, `selection`, `select`, `isGroup`, `create*`, `convert`, `model`, `setMaterial`, `setRendering` | Unchanged at the document level; `describe` evaluates through `pm-wasm` |
| `scene3d.renderFrame`, `cancelRender`, `previewState`, `setPreviewMode`, `onPreviewChange`, `blenderStatus`, `chooseBlender` | Engine-owned Blender jobs through main; state from engine events |
| `scene3d.prepareImport` | Engine import (`pm-geom3d`) |
| `scene3d.viewport.attach`, `run`, `getView`, `setView`, `frame`; controller `pick`, `glyphsIn`, `covers` | Renderer controller over `pm-wasm` geometry; view override in frame requests |
| `layers.register` with renderer `layer3d` or `scene3d` | Engine-owned 3D pass |
| `services` `raster` override | Section 11 |
| `services` `viewer`, `shaderHooks`, `compositions`, `captions` | Kept; GL calls redirected |
| `effects`, `transitions`, `layers.register` | GLSL contract unchanged; WGSL at apiVersion 4; definition snapshots; `ui`, `advanced`, `options` stripped from snapshots |
| `controls.*`, `PanelDock 'bottom'`, `contextual` keybindings | UI only |
| `events` (all 16, including `scene3d:selection`, `frame:rendered`, `overlay`, `time`) | Same names; frame events on presentation |
| `PanelDefinition.library.render` | Kept; engine snapshot for the viewer card |

`EXTENSION_API_VERSION` moves to 4 (WGSL, async render APIs). It lives in
`packages/registry/src/manifest.ts`, shared with Powermove Cloud, so the bump is a
coordinated registry release. `api.scene3d` was added without a bump; apiVersion 4
documents it. Extensions at apiVersion 1 to 3 keep loading. The api-pack and
`apps/desktop/docs/EXTENSIONS.md` are updated.

### Agents and MCP

All agent tools keep their schemas. What changes is where they run:

- **Frames.** `render_frames`, composition `media_contact_sheet`, and
  `observeVisual` (four representative frames on every send, review pass, and
  final check, `harness.ts:196-202`) become batched engine frame requests at the
  agent-snapshot priority. They queue instead of failing, never pause playback,
  run alongside exports, and include Blender surfaces headlessly.
- **Export and save.** `export_video` (MP4 or ProRes; range, audio, motion blur,
  alpha) and `save_project` move onto the job engine (section 17). From Phase 3b,
  `export_video` gains optional `preset` and `scale` and the formats at cutover;
  `get_render_status` covers async use.
- **Document and 3D.** `apply_commands`, `edit_video`, `edit_3d`,
  `get_project_state`, `get_3d_scene` evaluate and measure through `pm-wasm`;
  `edit_3d` model operations and rendering settings drive engine Blender jobs.
- **Media.** `list_media`, `import_media`, `replace_media`, `manage_media`,
  `import_media_to_timeline`, `check_project` get status, peaks, and probes from
  the engine; imports deduplicate on the content hash.
- **Analysis.** `probe_media` gains color metadata, HDR format, and VFR status.
  `sample_media_frames`, `media_waveform`, `transcribe_media` keep using
  originals; retiming math moves to `pm-wasm`. `validate_effect` compiles through
  the engine.
- **Panels.** `capture_panel`, `computer_use_panel`, `get_panel_state`,
  `interact_panel`, `open_panel`, layout tools need a real DOM. In the hidden host
  they use a Chromium started on demand.
- **Orchestration.** Up to 8 parallel subagents share the root's transaction.
  Engine sessions are per document, so their renders run concurrently.
- **New in Phase 4:** `manage_proxies`.
- **Concurrency.** Several external MCP connections on one host get per-session
  project pinning in main, and their renders and exports queue by priority.

Every other consumer of the engine is in appendix C.

## 21. Media management and project files

### Where bytes live

Today every asset's bytes are copied into the renderer's IndexedDB and embedded in
the `.pmv`. The engine cannot read IndexedDB. From Phase 2:

- **Media store in main**, read by the engine through capability handles.
- **Two storage modes:** Linked (file stays put; `link` reference) or Copied into
  project (embedded in the `.pmv`, as today).
- **Always copied:** items without a file (clipboard images, web downloads,
  generated stills), Blender sources (`.blend`), imported 3D models (GLB, glTF,
  OBJ bundles), and files inside an agent workspace. Deleting a project deletes its
  agent workspace (`legacy/core/projects.ts:241-243`), so workspace files must
  never be linked.
- Clipboard paste of a Finder file (`main/media-paste.ts`) is a real file and
  follows the import setting.
- **When importing** (Settings › Project › Media, from Phase 2): Link to files
  (default) or Copy into project.
- **Memory cards.** Phase 2 copies files on removable camera media into the
  project. After cutover: "Copy from memory cards" offloads with checksums to a
  chosen folder and links them.

### Relinking

- Automatic relink only by content hash, UMID, or content identifier.
- Matches by name, timecode, or duration are proposals in a Relink sheet.
- Replace File… keeps the interpretation and offers to match scale.
- Offline clips show "Media offline · A001_C003.mov" in the viewer and a quiet
  "Offline" in the timeline label; with proxies they keep playing.

### Collect

**Collect Project…** (Phase 4) writes a folder with the project, originals,
fonts, models, Blender sources, and the definition snapshots of the extensions
used, and states when a composition needs Blender. After cutover: LUTs, OCIO
configurations, trimmed media, and Package for Travel with proxy segments.

### File format

- Desktop Save writes PMV4; Export › Project and the browser fallback write PMV3.
  Linked assets add records without bytes.
- Saves keep the user's file mode, including read-only files: the incremental
  clone is made writable during the transaction and its mode is restored before
  publishing.
- Generated media is never stored in the project file.
- **`minReaderVersion` guard.** No build checks a version on load today. Phase 0
  ships a guard: a project above the build's version shows "This project needs a
  newer version of Powermove" and opens read-only.

### Switching engines safely

While the legacy engine can still be selected (the Phase 2 to 4 flag and the
Phase 5 switch release):

- IndexedDB media is copied to the media store, never deleted, until legacy code
  is removed.
- A project that uses a native-only feature (linked media, kept non-Chromium
  originals, linear blending, interpretation overrides, `compat` level 2 layers)
  is stamped `requiresEngine: "native"`; the legacy engine refuses it with "This
  project uses the new engine. Turn on Use New Engine to open it."
- Gate (Phase 5): every corpus project without native-only features round-trips
  flag on → off → on with no loss; projects with them are refused cleanly.

### Migrating existing projects

- Assets whose original was replaced by a VP9 transcode show "Using a converted
  copy · Relink to the original for full quality" with Locate Original….
- Embedded media moves from IndexedDB to the media store on first open with the new
  engine; editing copies are deleted when legacy code is removed.
- Legacy `powermove.3d.scene` and OBJ layers keep rendering; the existing explicit
  conversion (`scene3d/migration.ts`) stays a user action.

## 22. Web player, serve, the headless hosts, the CLI

### Web export and `packages/player`

`packages/player` is a build-only package over `renderer/src/player/*`, which
imports `legacy/core`, `legacy/gl`, and, through them, `core/scene3d` and
three.js. It powers code export, the www hero (which scrubs arbitrary times
through `createSvgPlayer` and `createEngine`), and a prebuilt copy vendored into
onboarding.

1. **At cutover**, the TypeScript evaluator, rasterizer, compositor, and native 3D
   renderer (three.js) move into `packages/player` as a frozen web player v1 for
   code export, www, onboarding, and serve. It renders native 3D and never Blender
   (Blender compositions export with native shading and a warning, as today; the
   player must not call the Blender bridge, appendix A). A parity test compares
   web player v1 with the engine in Standard color.
2. **After cutover**, a WASM build of `pm-eval`, `pm-geom3d`, `pm-text`,
   `pm-vector`, and `pm-render` on WebGPU replaces it for new exports (scene v2).

### `powermove serve`

Paused today. Before Phase 5, serve tabs switch to web player v1 for their preview
(decision 9).

### The hidden MCP host

`powermove mcp` and `--powermove-mcp` ship today (`apps/desktop/docs/MCP.md`).
The hidden host runs the production editor in Playwright Chromium on SwiftShader,
without the headless document host (`server/mcp.ts:43-53`), behind a per-profile
lock.

- **Phase 2:** agent `export_video` (ProRes, H.264) runs through the job engine on
  the desktop app; the first headless consumer.
- **Phase 3c:** the hidden host becomes the headless document host on `pm-wasm` for
  document tools, plus `pm-node` engine sessions for `render_frames`, contact
  sheets, `export_video`, and Blender jobs, all in the CLI's Node process.
  Chromium launches only when a panel tool is called. The document model stays
  TypeScript, keeping the rule in `server/mcp.ts:26-27`.
- On Linux, the CLI renders through Vulkan or a software fallback and encodes
  through our FFmpeg build. ProRes on Linux is decision 16.
- Gate: `scripts/test-mcp.mjs` passes on the engine host with no Chromium
  installed, apart from its panel-tool checks.

### The headless document host

`apps/desktop/src/engine/main.ts` runs `legacy/core` under happy-dom for serve. In
Phase 1 it switches to `pm-wasm`. That also fixes: captions tools routed there
without `captions/install`; `editing.ts:927` calling `PM.raster` unguarded; and
serve routing `__mcp_export_video` and `__mcp_save_project` to it although it has
no exporter (`server/runs.ts:20-23,194`). In Phase 3c those calls go to the job
engine.

### The CLI

`pm-cli` renders a `.pmv` to any export format without Electron, from definition
snapshots, with presets, ranges, and parameter overrides. Compositions that render
with Blender need a local Blender (`--blender`) or fail naming it. It hosts the
parity runner (evaluation in Phase 1, rendering from Phase 2). Users reach it
through the existing `powermove` command (`powermove render`).

## 23. Cross-platform

- **macOS:** Apple Silicon. Intel only if demand justifies it.
- **Windows (after cutover):** wgpu DX12, D3D11/D3D12 decode through FFmpeg,
  Media Foundation where needed, WASAPI, DirectWrite, NT-handle surfaces with LUID
  adapter matching. Blender discovery already covers Windows paths; Cycles device
  selection needs OptiX, HIP, or oneAPI. The Electron shell needs a Windows build,
  installer, signing, and a native-conventions review.
- **Linux:** `pm-cli`, the hidden MCP host, and serve, with Vulkan or a software
  fallback.
- Platform code behind traits in `pm-media`, `pm-gpu`, `pm-text`, `pm-audio`, each
  with a software fallback for tests.

## 24. Licensing and compliance

Settled with counsel before the dependent phase ships.

1. **FFmpeg.** Replace `ffmpeg-static` (bundled by the app, the CLI, and the hidden
   MCP host's export path), a third-party build whose configuration we have not
   audited, with our own build: pinned version, LGPL or GPLv3-compatible
   components only, never `--enable-nonfree`, shipped as dylibs. It covers export,
   the media-proxy converters, and the agent media tools. Runs in parallel with
   Phase 0 and gates Phase 2.
2. **ProRes.** Apple licenses ProRes and names FFmpeg's implementation
   unauthorized; today's export uses `prores_ks`, including on Linux through the
   CLI. On macOS, ProRes decode and encode use VideoToolbox. On Windows and Linux,
   both need Apple's licensing program or are omitted (decision 16).
3. **H.264 and HEVC encode.** Today's H.264 export uses libx264. Prefer OS
   encoders; confirm royalty exposure for any software encoder.
4. **AGPL and proprietary SDKs.** Camera RAW SDKs need a GPLv3 section 7 additional
   permission from all copyright holders.
5. **Blender.** Powermove does not ship or link Blender. It runs a user-installed
   Blender (GPL) with a Powermove-authored `bpy` worker script, which is
   GPL-covered when it runs; AGPL-3.0-or-later is compatible with GPLv3. Counsel
   confirms. Bundling Blender would require GPL compliance.
6. **three.js and Playwright.** Ports of three.js shader code keep its MIT notice.
   Playwright downloads Chromium at install time and is not bundled; it is removed
   from the rendering path in Phase 3c.
7. **Dependencies.** `cargo-deny` enforces an allowed license list and RustSec
   advisories.

## 25. Testing, parity, and gates

### Baseline

- **E2E ledger.** Phase 0 makes the e2e suite (228 specs at `d5d5b731`) green or
  quarantined, with owners and reasons.
- **Parity corpus**, captured before engine code lands: the 12 pixel-hash
  scenarios, every e2e fixture, synthetic projects per feature, real-world projects
  Jude supplies (outside the repository), and a **3D corpus**: primitives, lathe
  and extrude, skinned and animated glTF with source materials, every light type
  with shadows, transparent models, 2D barriers between depth blocks, nested 3D
  precomps, legacy OBJ, Studio Cube, legacy scene layers. Blender surfaces are
  captured once with a pinned Blender and stored as fixed inputs, because EEVEE and
  Cycles are not deterministic. Also: variable-rate MP4 with a matched composition;
  read-only `.pmv` save.

### Parity

- **Evaluation:** |a − b| ≤ 1e-9 + 1e-9·|b| for numbers; exact for strings and
  booleans; text as in section 6; 3D picks identical; bounds within 1e-6.
- **Render, Standard color:** per-channel 8-bit difference at most 1 for 99.9% of
  pixels and at most 4 for any pixel, with these classes tested separately:

  | Class | Test |
  |---|---|
  | Video frames | Today's Chromium frames vs the engine within 2 CV; the engine within 1 CV of an FFmpeg plus zimg reference |
  | Text | Identical line breaks and advances within 0.25 px (level 1); SSIM ≥ 0.98; reviewed sheet |
  | Edges | Section 2 antialiasing reference |
  | Blur | Analytic Gaussian |
  | Alpha | Within 1 CV of an f64 reference |
  | 3D shading | Against three.js r180 renders: at most 2 CV for 99.5% of pixels; MSAA edges use the Edges class |
  | Blender surfaces | Engine composite of the stored surfaces, exact |
  | Appendix A fixes | Listed per case |

  Each real-world corpus project has at most 20% of its pixels in these classes at
  any sampled frame, or a dedicated reviewed comparison.
- **Web player v1** against the engine in Standard color.

### Correctness and conformance

- Section 2 bars as automated tests.
- **Codecs:** FFmpeg reference decodes; random access equals sequential; corpus
  includes open-GOP HEVC, B-pyramid H.264, edit lists, VFR phone footage,
  interlaced MXF, HLG and HDR10, Dolby Vision, 4:2:2 10-bit, ProRes 4444 XQ, DNxHR.
- **Exports:** every format probed, including sampled pixel values.
- **Determinism**, **fuzzing** with `cargo-fuzz`.

### Infrastructure

- Rust unit and property tests per crate.
- `pm-cli parity`, owned by the Parity and QA lane: evaluation from Phase 1,
  renders from Phase 2.
- Playwright e2e through the same UI; specs that read WebGL pixels directly
  (including `scene3d.spec.ts` and `viewport3d.spec.ts`) are rewritten against
  engine frames.
- Existing harnesses stay: `native-export-performance`, `opaque-capture-parity`,
  `export-video-frames`, `video-frame-integrity`, `rendering-performance`,
  `export-code`, `scripts/test-mcp.mjs`, `blender-motion.spec.ts`. `test-mcp.mjs`
  gains a video import case.
- A debug "Compare engines" command until cutover.

### Performance gates

Three reference machines: a base M1 MacBook Air (no ProRes engine), a base
M-series Air with a ProRes engine ("base"), an M-series Pro ("Pro"). Calibrated in
Phase 0, then fixed.

| Scenario | Gate | Phase |
|---|---|---|
| Frame bridge under main load | Section 18 S1 gate | 0 |
| Layer drag, synthetic project of slice features (500 layers) | Pointer-to-pixel p95 ≤ 33 ms at 60 Hz | 2 |
| Inspector edit during playback, 1,000 layers | p95 ≤ 16 ms, no dropped playback frames | 2 |
| Scrub, intra codecs | p95 ≤ 50 ms | 2 |
| Scrub, long-GOP, no generated media | p95 ≤ 300 ms | 2 |
| A/V sync | ±10 ms | 2 |
| Agent snapshots during playback | 4 frames per agent turn with no dropped playback frames | 2 |
| Export plus playback memory, base 8 GB | No memory-pressure warnings over 10 minutes | 2 |
| Cold start | Engine ready before the window finishes loading the last project | 2 |
| Layer drag, 884-layer project | p95 ≤ 33 ms, at most 1 frame worse than today | 3a |
| 3D orbit, 3D corpus scene with shadows | Pointer-to-pixel p95 ≤ 33 ms | 3a |
| 3D pick, 1M-vertex skinned model | p95 ≤ 2 ms | 3a |
| Canvas text typing | Keystroke-to-pixel p95 ≤ 33 ms | 3a |
| UHD ProRes 422 HQ, 3 layers with blur | Real time at 23.976, 29.97, 59.94 on Pro; 29.97 on base; zero drops over 60 s | 3a |
| Same, linear blending on | Same | 3a |
| The 884-layer project | The five ceilings that fail today pass | 3a |
| 4K ProRes 422 HQ export | At least real time on base | 3b |
| Export, every format at cutover | Absolute fps per codec and machine, at least today's re-measured baseline | 3b |
| 3D export, 3 models with shadows, Blender EEVEE, 16 samples | At least today's re-measured baseline | 3b |
| Eight concurrent subagent `render_frames` during an export | No failures, no playback pause | 3c |
| 4K long-GOP HEVC 10-bit and H.264 4:2:2 10-bit, Automatic | Real time on all three machines | 4 |
| Scrub, long-GOP with optimized media | p95 ≤ 100 ms | 4 |
| Paused upgrade to original | ≤ 150 ms (4K ProRes, base); ≤ 300 ms (4K long-GOP HEVC, base) | 4 |
| Segment boundaries, 4 streams | Zero drops | 4 |
| Memory | Within leases; no growth over a 1-hour playback loop; `pm-wasm` budget met with the 3D corpus | 3a and 4 |

## 26. Phases

Ordered by dependency; lanes run in parallel (section 27). The legacy engine stays
the default until Phase 5. Each gate has objective criteria; Jude signs off at
each gate in addition.

**New engine-side features during the rewrite** (the 3D system landed between
revisions 3 and 4) follow decision 18: from Phase 0, any feature that touches
`legacy/gl`, `legacy/core` evaluation, `core/scene3d` runtime and rendering, or
export lands with parity corpus cases and a port note for the owning lane; a hard
freeze on such features starts at Phase 3a.

### Phase 0: Foundations, contracts, spikes

Work:

- Cargo workspace, CI toolchain, `cargo-deny`, napi-rs and wasm-bindgen builds; a
  placeholder addon signed and notarized through the real release script.
- Self-hosted reference runners with a pinned Blender.
- E2E ledger and parity corpus capture, including the 3D corpus.
- `minReaderVersion` guard shipped in a normal release.
- Appendix A items marked (Phase 0).
- Opt-in crash and session reporting (decision 10).
- The FFmpeg replacement track (section 24).
- Frozen lane contracts (section 27).
- Audit of the Store and known local extensions for `render.gl.context`, the
  `raster` override, and other deprecated surfaces (section 20).
- Spikes, each a throwaway branch with a written result:
  - **S1 frame bridge** (section 18), including 3D orbit latency.
  - **S2 precision and encode:** ProRes 4444 XQ round trip; VideoToolbox formats;
    f16 versus f32; HEVC alpha depth; AVAssetWriter tags; QuickTime-matching
    Rec.709 tags.
  - **S3 shaders:** every GLSL source in the repository, the Store, the docs
    samples, and a sample of agent-written local extensions; naga versus
    SPIRV-Cross; flip and derivative cases.
  - **S4 text and vectors:** Parley, HarfRust, Skrifa against Canvas2D and
    CoreText; vector rendering through Vello coverage with float paint,
    `vello_gpu`, and Skia Graphite (`skia-safe`, half-float surfaces); a CPU
    fallback.
  - **S5 WASM evaluator:** `evP` under 1 µs; patch apply under 0.2 ms (property)
    and 2 ms (structural, 884 layers); per-window memory budget.
  - **S6 decode:** VideoToolbox and FFmpeg into wgpu without copies; exact random
    access.
  - **S7 3D:** three.js r180 standard and physical-subset shading, PCF shadows,
    area lights, ACES Filmic, MSAA, skinning, and morphs in wgpu against the 3D
    corpus, comparing hand-written WGSL with three.js's generated GLSL compiled
    through the S3 pipeline; `pm-geom3d` in WASM with a 1M-vertex skinned model
    under the budget and picks under 2 ms; 16-bit Blender surfaces into the job
    engine through capability handles.

Gates, per consumer:

- S5, S7's `pm-geom3d` part, and contracts 1, 5, 9 gate Phase 1.
- S1, S2, S6, the FFmpeg replacement, the runners, and the remaining contracts gate
  Phase 2.
- S3, S4, and S7's rendering part gate Phase 3a.

### Phase 1: Evaluation and the write path

`pm-time`, `pm-model` (`v1_ingest`, reconciliation, scene3d schemas), `pm-eval`
(including 3D namespaces, aims, recipes), `pm-geom3d` queries, `pm-edit`
(including model-group bounds, aim preservation, material peers), `pm-vector`
geometry, `pm-text` layout, the font registry in main, `pm-protocol`, `pm-wasm`;
write barrier, flush-before-query, auditor, dual-answer mode (against the legacy
evaluator and three.js geometry); the single frame-rate validator covering all
four entry points; `pm-cli parity` for evaluation. Synchronous queries and
geometry-dependent edits, including the 3D viewport editor's, switch to `pm-wasm`
behind a flag. The headless document host switches to `pm-wasm`; the
text-animation editor helpers become shims.

Gate: evaluation parity on the corpus; zero dual-answer mismatches beyond
tolerance and zero auditor failures across the e2e suite per the ledger; native
and WASM bit-identical; S5 and `pm-geom3d` targets met in the app.

### Phase 2: Tracer bullet

A thin slice through every layer of the architecture, behind "Use New Engine":

- Engine processes, MessagePorts, capability handles, leases, priority classes,
  crash handling.
- Production frame bridge with jitter buffer, overlay geometry, view override.
- `pm-media`: probe, indexes, content hashes, VideoToolbox decode of ProRes, H.264,
  HEVC; image readers.
- Minimal `pm-render`: solid, image, video, transform, opacity, normal blend,
  precomp, groups, and one `layer3d` primitive lit by a sun light without shadows;
  Standard color with the invariants.
- `pm-audio`: device clock, streaming decode, mixing; playback; scrubbing.
- `pm-export`: ProRes and H.264 through VideoToolbox in the job engine; atomic
  output; verification; agent `export_video` and agent snapshots through the job
  engine on the desktop app.
- Media store in main, linked media, relinking by content hash, originals kept on
  import, `requiresEngine` stamping, the "When importing" setting.
- Font registry moves into the engine.
- `pm-cli parity` for renders.

Gate: Phase 2 performance rows; the color invariant, pipeline transparency, codec
generation, and time bars; crash and GPU-loss recovery; the legacy engine refuses
stamped projects and opens unstamped ones unchanged; `scripts/test-mcp.mjs` export
passes through the job engine.

### Phase 3a: Rendering parity

All layer types, shapes and paths, text and captions, masks and mattes, blend
modes, built-in effects and transitions in WGSL, the GLSL compatibility pipeline
and definition snapshots, motion blur, 2.5D, the full 3D scene pass (depth blocks,
shadows, all lights, materials and maps, glTF animation and skinning, legacy mesh
and scene layers, editor looks), Blender surface compositing, adjustment layers,
retiming, continuous precomp rasterization, linear blending, the internal scope
tool.

Gate: render parity on the corpus including the 3D classes; linear-blending,
antialiasing, 3D, and determinism bars; text review; Phase 3a performance rows.

### Phase 3b: Media and delivery

Every import format at cutover including 3D models, interpretation and HDR input
tone mapping, VFR and interlacing, every export format at cutover, Blender jobs in
the job engine (generation, import, renders with shutter samples and nested
compositions, multiple workers), presets, the render queue, audio stretch and
remapping, channel layouts, meters, waveforms, J/K/L at 1× and 2×, audio scrub,
`export_video` presets and formats.

Gate: codec and export conformance; bit depth, alpha, SDR color, seeking, and
audio bars; Phase 3b performance rows.

### Phase 3c: Integration

The kernel API mapping including `api.scene3d`, the 3D viewport editor and overlay
on `pm-wasm` geometry and engine frames, agent and MCP tool changes, batched
concurrent agent snapshots, the hidden MCP host on the headless document host plus
`pm-node` with Chromium on demand, `pm-node` CLI prebuilds, every consumer in
appendix C, posters, thumbnails, filmstrips, `pm-cli` exports, serve's switch to
web player v1.

Gate: every appendix C row done and tested; e2e suite green on the new engine per
the ledger; `test-mcp.mjs` on the desktop and hidden hosts without Chromium except
panel checks; the concurrency gate in section 25.

### Phase 4: Proxies, optimized media, caches

`pm-proxy`, render cache with visibility and Keep Rendered, background rendering,
appendix D controls, diagnosis, Storage, Collect Project, the Relink sheet,
Replace File, offline-media export behavior, `manage_proxies`.

Gate: Phase 4 performance rows on the three machines without user action; the
cache-to-export byte-identity test; no proxy pixels in exports without a draft
choice; generated media survives moves, relinks, relaunch; storage limits and
pinning enforced.

### Phase 5: Cutover

New engine on by default for one release with the legacy engine behind a switch;
then removal of `legacy/gl`, the legacy media, playback, audio, and export paths,
and the three.js output renderer from the editor; web player v1 in
`packages/player`; onboarding copy rebuilt; `EXTENSION_API_VERSION` 4 with the
registry release; docs updated (`EXTENSIONS.md`, `SCENE3D.md`, `MCP.md`, README,
the engine docs in `apps/desktop/docs`).

Gate: on the switch release, at least 2,000 sessions from at least 100 users who
opted in to reporting, with crash-free sessions at least 99.5% (or the cohort
alternative in decision 10); zero open P0 or P1 regressions; the engine-switch
round-trip test; legacy engine code deleted from the editor; docs updated.

### After cutover

Each with its own plan, roughly in this order:

1. Professional color (section 9) and unified 3D tone mapping; scopes.
2. Delivery formats listed in section 17; audio channel mapping.
3. Camera proxies, Package for Travel, trimmed Collect, verified card offload.
4. Windows.
5. Cameras and lights for 2.5D layers; intersections between 2.5D planes and
   models; Collapse transformations.
6. Spatial bezier motion paths; frame blending and optical flow; faster shuttle and
   reverse audio.
7. Camera RAW.
8. WASM web player (scene v2) including 3D.
9. Audio effects and loudness tools.
10. OpenFX hosting out of process.
11. `pm-doc`: edit pipeline and history in Rust, ticks on disk.
12. PSD and multi-part EXR layers; FBX; FCPXML and OTIO; 3D Text from `pm-text`.
13. Shell review: Electron or Tauri. The engine is host-agnostic, so a move reuses
    it. The review weighs WKWebView differences on macOS (Chromium-only CSS such
    as `corner-shape`, `queryLocalFonts`), the per-process extension sandbox,
    the Node main process (agent harness, MCP, save, Store), and agent panel
    capture, against a smaller app and direct engine hosting.

## 27. Lanes, contracts, and ownership

### Contracts frozen in Phase 0

1. `EvaluatedScene`, between `pm-eval` and `pm-render`, including 3D worlds and
   depth blocks.
2. `FrameRequest` and `FrameDescriptor` (region, scale, color, precision, revision,
   PTS, overlay geometry, view override, draft 3D).
3. The `pm-gpu` interop trait (decode in, bridge out, encode in, Blender surfaces
   in).
4. The color contract: encoding E, encodings per blending setting,
   premultiplication, tone mapping for 3D, where transforms run, cache-key
   components.
5. Patches, paths, canonicalization, `v1_ingest`, the engine-originated facts
   table.
6. The playback clock between `pm-audio` and the scheduler.
7. Errors and isolation.
8. Budget leases.
9. Protocol codec and code generation.
10. Definition snapshots, including engine-owned renderer kinds.
11. The Blender job protocol, snapshot, and surface format.
12. Priority classes and agent frame requests.

### Lanes

| Lane | Owns | Phases |
|---|---|---|
| Model and evaluation | `pm-time`, `pm-model`, `pm-eval`, `pm-edit`, `pm-wasm` | 0, 1 |
| Renderer write path | Write barrier, flush, auditor, dual-answer mode, frame-rate validator, `minReaderVersion` and `requiresEngine`, headless document host | 0, 1, 2 |
| Text and vectors | `pm-text`, font registry, `pm-vector` | 0 (S4), 1, 3a |
| 3D | `pm-geom3d`, the 3D scene pass, `pm-blender`, 3D viewport integration | 0 (S7), 1, 2, 3a, 3b, 3c |
| Media | `pm-media`, decoders, readers, index, content hashing | 0 (S6), 2, 3b |
| GPU and render | `pm-gpu`, `pm-render` 2D, GLSL compatibility, definition snapshots | 0 (S3), 2, 3a |
| Color | `pm-color`, invariants, interpretation, tone mapping | 0 (S2), 2, 3a, 3b |
| Engine and platform | `pm-engine`, `pm-node`, `pm-bridge`, `pm-protocol`, leases, priority classes, packaging, CI, runners, FFmpeg build, crash reporting | 0 (S1), 2 |
| Audio | `pm-audio`, mixer | 2, 3b |
| Export and agents | `pm-export`, `pm-cli`, render queue, agent `export_video`, `save_project`, and snapshots, the hidden MCP host on the engine, CLI prebuilds | 2, 3b, 3c |
| Proxies and storage | `pm-proxy`, `pm-cache`, media store, media management | 2, 4 |
| Viewer and settings UI | Viewer, overlays, settings, appendices C and D, API mapping, extension docs | 2, 3c, 4, 5 |
| Parity and QA | Ledger, corpus, parity runner, conformance, performance gates, appendix A Phase 0 fixes | 0 onward |

Rules: each lane works in its own worktree and branch, commits granularly, and
submits stacked PRs (no squash merges); independent reviewers; the Parity and QA
lane owns gates and reports failures as they are.

## 28. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `sharedTexture` is experimental and tracks Chromium | Viewer breaks on an Electron upgrade | S1 first; ranked fallbacks; upgrades gated by a bridge test |
| Gesture and orbit latency through the bridge | Handles and the 3D viewport feel detached | Overlay geometry; view override; latency gates in S1, Phases 2 and 3a |
| Main-process jitter | Uneven playback | Import-once pools, small messages, jitter buffer, S1 under load |
| Reads after writes inside edits | Wrong geometry | Write barrier, flush, edit kernels, dual-answer mode |
| three.js shading parity in wgpu | Every 3D project looks different | S7, port three.js shader code, reviewed 3D corpus |
| 3D geometry in `pm-wasm` | Memory, slow picks | S7 budgets, BVH, shared buffers |
| Blender as an external dependency | Failed or inconsistent exports | Pinned LTS on runners, content-keyed cache, named failures, counsel |
| Blender cost scales with layers × samples | Slow exports | Pipelined stage, several workers, per-layer passes, cache reuse |
| Text looks different | Regressions in every project | Level 1 compatibility, S4, coverage tuning, reviewed corpus |
| GLSL translation failures | Broken extensions and projects | S3 on every known source including local agent extensions |
| Vector renderer maturity | Quality or stability | S4 includes Skia Graphite; CPU fallback |
| Extensions unavailable to headless renders | Wrong exports | Definition snapshots; explicit failure |
| Agent render load (captures every turn, 8 subagents) | Playback stutter, slow agents | Priority classes, batching, leases, S1 under agent traffic |
| Two headless paths drift (hidden Chromium vs engine) | Agent exports differ from desktop | One engine for both; `test-mcp.mjs` against both until Phase 3c |
| Concurrent external agents on one host | Project switched under a session; collisions | Per-session pinning in main; queued jobs |
| Linux CLI and MCP lose ProRes or GPU | Regression for headless users | Decision 16; software fallback; explicit capability errors |
| Switching engines loses data | Broken projects | `requiresEngine` stamping; media copied, not moved; round-trip gate |
| Hidden consumers of renderer media and three.js | Blank thumbnails, broken viewport | Appendix C owned by a lane and gated in Phase 3c |
| Weak media identity | Wrong footage after relink, reuse, or agent dedupe | Full content hash; file identity re-verification |
| Engine-side features keep landing in legacy | Moving target, double work | Decision 18: corpus cases and port notes from Phase 0; hard freeze at 3a |
| No CI hardware | Gates cannot run | Self-hosted runners in Phase 0 |
| No crash telemetry | Cutover gate unmeasurable | Opt-in reporting in Phase 0, or the cohort alternative |
| Scope creep before cutover | Parity quality suffers | Non-goals; after-cutover lists |
| Toolchain churn (wgpu, Electron, FFmpeg, Blender) | Upgrade cost | Pinned versions; upgrades as lanes |
| Licensing | Blocked features or exposure | Section 24 before dependent phases |
| Memory on 8 GB machines | Crashes | Main as arbiter, leases, base-machine gates with Blender running |
| Engine crashes and loops | Lost playback or exports | Two engines, attribution, backoff, persisted jobs, atomic output |
| Old builds overwriting new projects | Data loss | `minReaderVersion` guard in Phase 0 |

## 29. Decisions needed from Jude

1. **Default for new projects:** Standard color with "Blend in linear light" off
   (recommended), or on.
2. **When importing:** Link to files (recommended) or Copy into project.
3. **Generated media location:** the Powermove cache (recommended), next to the
   project, or a chosen folder.
4. **Optimized media:** Only when I ask (recommended).
5. **Disk guard and storage limit:** below the larger of 20 GB or 10% free; limit
   25% of the cache volume (recommended).
6. **Order after cutover:** professional color first, then delivery formats, then
   Windows (as written), or Windows earlier.
7. **Intel Macs:** no (recommended).
8. **Web export future:** replace web player v1 with the WASM engine after cutover
   (recommended).
9. **Serve interim:** serve tabs use web player v1 (recommended).
10. **Crash and session reporting:** opt-in reporting (recommended), or a beta
    cohort submitting local crash logs, with the same thresholds.
11. **Reference machines:** a base M1 MacBook Air, a base M-series Air with a
    ProRes engine, an M-series Pro.
12. **Licensing counsel** for section 24 items 2 to 5.
13. **Real-world corpus projects** (kept outside the repository).
14. **3D rendering:** port the three.js r180 subset natively in the engine
    (recommended: one renderer, headless, no JavaScript in exports), or keep
    three.js in the renderer as a pixel source (not headless, an extra copy per
    layer per frame).
15. **Blender surfaces at cutover:** 16-bit PNG through AgX, keeping today's look
    exactly (recommended), or half-float EXR with the engine applying AgX (better
    precision, a small look risk). EXR becomes the path for scene-referred
    projects after cutover either way.
16. **Headless MCP host and Linux ProRes:** after Phase 3c the hidden host uses the
    headless document host plus `pm-node` with Chromium only for panel tools
    (recommended). ProRes from the Linux CLI: drop it (today it works through
    `prores_ks`) or license it.
17. **3D tone curves:** keep ACES Filmic for Native and AgX for Blender at cutover
    (recommended: no look change), and unify under project color management
    after cutover.
18. **Engine-side feature policy during the rewrite:** from Phase 0, new features
    touching the engine land with corpus cases and port notes, with a hard freeze
    from Phase 3a (recommended); or a hard freeze from Phase 0.

## 30. Appendix A: defects found during the survey

Items marked (Phase 0) are fixed in the legacy engine in Phase 0 by the Parity and
QA lane. Others are fixed by the engine. Line references are at `d5d5b731`.

1. **Converted video replaces the original** (`raster.ts:887-902`,
   `media/media-path.ts:29`, `main/media-proxy.ts:39-51`).
   `apps/desktop/README.md:149-150` says the original stays embedded; it does not
   (`README.md:191-195` is correct). (Phase 0: correct the README.)
2. **Export transfer tag** is sRGB for H.264 and ProRes (`render-encoder.ts:13-16`).
3. **8-bit sources:** ProRes written at 10-bit from 8-bit data; 24-bit PCM padded
   from 16-bit WAV; three.js canvases and Blender PNGs are 8-bit; source-material
   textures are re-encoded through Canvas2D (`scene3d/rendering.ts:308-337`).
4. **Motion blur sample counts differ by path:** 16 (opaque native export, PNG,
   still), 20 (alpha, WebM, Recorder), 6 (web player, RAM preview), 6 playing and
   12 paused (preview), 10 (compositor default), 1 (agent snapshots, composition
   thumbnails). Blender shutter samples copy each path's count; RAM preview
   prepares 10 Blender samples but renders 6. (Phase 0: one setting for all export
   paths.)
5. **Fixed** in `aad31a41`: WebM and Recorder exports now render with
   `exporting: true` (`exporter.ts:612`).
6. **Render queue:** nothing adds jobs; each prompts for a destination; a failure
   stops the run; the run swaps the live project; agent `export_video` bypasses it.
7. **Frame-rate handling differs across four paths** (UI and `set_composition`
   round, `editing.ts:526`; load and validation keep fractions; Home's media
   matching copies the unrounded average, `app.ts:1338-1352`; MCP `create_project`
   accepts any float, `mcp-project-tools.ts:5`).
8. **Timecode at fractional rates** is wrong (`util.ts:197-205`), now reached by
   ordinary Home-opened 29.97 media and still-export file names
   (`exporter.ts:532`). (Phase 0)
9. **Variable speed** integrates at twice the open composition's rate, at least 60
   Hz (`retiming.ts:13`).
10. **Audio** ignores clip speed; time-remapped clips are silent.
11. **WebM exports** carry no color metadata.
12. **Aliasing:** hard UV edges, `antialias: false`, no mipmaps for 2D layers (3D
    layers use MSAA).
13. **Large blur radii** skip texels (`effects-basic/effects.ts:43-49`).
14. **Masks:** more than 8 parametric masks silently dropped on the GPU path.
15. **Solid corner radius** stored but not rendered.
16. **Headless document host:** captions tools routed there fail; text anchor edits
    can throw (`editing.ts:927`).
17. **Docs disagree** on effect varyings (`EXTENSIONS.md:441` versus
    `api.ts:201-212`). (Phase 0)
18. **51 tracked Finder duplicates** (`* 2.*`). (Phase 0: delete.)
19. **`PM.previewFps`** is read but never assigned (`engine.ts:47`), now also in the
    preview cache key (`compositor.ts:211`).
20. **No version check on load.** (Phase 0: the guard.)
21. **Unbounded caches:** the IndexedDB media store, editing copies, the live asset
    map, solver caches, font sources, resident GLB models.
22. **Sampled media fingerprint** can collide (`media.ts:96-97,153-171`); agent
    import now deduplicates on it (`media-management.ts:83-88`).
23. **Native and Blender 3D tone curves differ** (ACES Filmic versus AgX), so Solid
    preview, Rendered preview, and export can look different.
24. **Variable-rate MP4s set the composition to their average rate** (for example
    29.87); `mp4FrameRate` ignores edit lists.
25. **Headless MCP exports render on SwiftShader:** slower, and not pixel-identical
    to GPU output.
26. **Blender export cost** is 3D layers × (1 + shutter samples) renders per frame,
    serialized through one worker.
27. **Blender frame requests skip the compositor's 4096-px nested-composition cap**
    (`compositor.ts:1040-1041` versus `frame-preparation.ts:42`), so large nested
    exports can miss the cache and throw. Inferred from code.
28. **The hidden MCP host's import filter accepts `.fbx`**, which import rejects
    (`server/mcp.ts:76`, `raster.ts:587`).
29. **Untrusted `.blend` files** are parsed by an unsandboxed child of main.
30. **Project deletion removes the agent workspace**, assuming all media is copied
    in (`projects.ts:241-243`); this would break under linked media (section 21
    keeps workspace files copied).
31. **Any edit during an export invalidates prepared Blender frames**
    (`_scene3dRevision`, `anim.ts:18`; `rendering.ts:183,682`).
32. **Web and code exports of compositions with `render3d.enabled`** likely fail in
    the player: `prepareFrame` requires a Blender bridge (`player.ts:114`,
    `frame-preparation.ts:22-25`, `rendering.ts:687-688`). Inferred from code.
33. **Cycles `device: auto`** makes exports machine-dependent
    (`blender-worker.ts:274-283`).
34. **The renderer's 512 MiB Blender surface budget** throws at 4K with motion blur
    (`rendering.ts:709-712,755-757`). Inferred from code.
35. **Agent capture is single-flight:** `snapshotAsync` pauses playback and throws
    during another capture or an export (`exporter.ts:856-870`), so parallel
    subagents' `render_frames` calls fail.
36. **Serve routes agent export and save to the headless document host**, which has
    no exporter (`server/runs.ts:20-23,194`). Inferred from code.

## 31. Appendix B: feature parity inventory

Everything below works in the engine before cutover.

**Composition.** Size; frame rate (rational); duration; work area; background
(none, solid, linear and radial gradient up to 8 stops); shutter; master audio
gain; markers; scene parameters; multiple compositions; nesting; pre-compose; 3D
rendering settings (Native, EEVEE, Cycles; samples, preview samples and scale,
denoise, device); matching to media opened from Home.

**Layers.** solid, shape, text, image, SVG, video, audio, adjustment, shader,
extension (fragment, mesh, `layer3d` object, light, camera, legacy `scene3d`),
null, group (including model groups), precomp, captions. Fields: visibility,
lock, shy, solo, collapsed, color label, blend, motion blur, 3D, linked scale,
parent, group, track, matte source and mode. Parenting that preserves pose and
aim.

**Transforms.** Anchor, position, scale, rotation, opacity, skew; Z position,
anchor, scale; X and Y rotation; orientation; perspective per 3D group.

**Animation.** Linear, bezier, hold; speed and influence; auto-bezier;
continuous; springs (including agent-authored); legacy easing presets; color
interpolation; string and boolean steps; `offset_property`; negative
shared-material key times. Expressions with all identifiers and helpers.

**Shapes and paths.** Fill and stroke, per-corner radius, smoothing, polygon and
star points, cubic paths, trim and repeater on editable paths, SVG import to
paths.

**Text.** All properties, variable axes, range styles, animators, font anchoring,
local and embedded fonts, canvas text editing.

**Masks and mattes.** Parametric and bezier masks; four track matte modes; matte
nesting; cycle rejection.

**Compositing.** Nine blend modes, opacity, group effects, adjustment layers,
motion blur, transitions.

**Effects and transitions.** The 16 effects and 4 transitions with all
semantics; extension effects and transitions; forks of built-ins.

**Shader and extension layers.** Shadertoy-style header, `@param` uniforms, five
presets, Studio Cube, OBJ model.

**3D.** Model, light, and camera layers; model groups with geometry pivots;
primitives, imported GLB, glTF, and OBJ, indexed meshes, lathe, extrude; recipes
(Rocket, Staircase, 3D Text, lathe, extrude, mesh) with bevel, subdivision,
solidify, array, and mirror, and animated parameters; materials with maps, UV
placement, slots, shared materials, and whitelisted Blender shader graphs with
animated inputs; sun, point, spot, and area lights with shadows; perspective and
orthographic cameras with aim targets and environment; depth blocks; glTF
animation and skinning; Native, EEVEE, and Cycles rendering; Edit Mode; viewport
shading modes, Local View, Lock Camera to View, Auto Keying; conversion of legacy
scene and OBJ layers.

**Retiming.** Speed, time remap, freeze, reverse, source time, independent
decoders per instance.

**Media.** Every import format today including 3D models, animated images, image
sequences, posters, iCloud materialization, URL import for extensions, remote
media in serve mode, OBJ, SRT and VTT import, clipboard paste, Home's composition
matching.

**Audio.** Per-layer gain, mute, solo, fades, keyframed gain, precomp nesting,
embedded audio, meters, waveforms, master gain.

**Preview.** Resolution, Automatic quality, frame cache, cached preview, retained
presentation, viewport-cropped rendering, context-loss recovery, FPS readout,
paused Blender Rendered preview.

**Export.** Native MP4 and ProRes, WebM, PNG sequence, still, code export
(including native 3D), project export, presets, defaults, ranges, scale,
fractional rates, captions, progress with live preview, cancel, render queue,
Blender-rendered compositions.

**Kernel, agents, and MCP.** Every API and tool in section 20; the public MCP and
hidden host; agent export delivery; agent media management; orchestration.

## 32. Appendix C: integration checklist

Every consumer of today's engine outside the compositor. Owned by the Viewer and
settings UI lane (3D rows by the 3D lane, agent rows by the Export and agents
lane) and gated in Phase 3c. Line references are at `d5d5b731`.

| Consumer | Today | Replacement |
|---|---|---|
| Composition thumbnails (`panels/comp-thumbnails.svelte.ts:58-70`, `draft3d`) | `PM.GL.renderToPixels` | Async engine thumbnails at draft 3D quality |
| Project thumbnail on save (`legacy/app.ts:578-587`) | Reads the viewer canvas | Engine snapshot |
| Library look thumbnails (`legacy/core/library.ts:87`) | `PM.Export.snapshot` | Engine snapshot |
| Library viewer card (`library/LibraryScreen.svelte:377-386`) | Copies live canvases | `library.render` with an engine snapshot |
| Assets panel posters and inline video | IndexedDB posters, blob-URL `<video>` | Engine posters and filmstrips |
| Assets panel audio thumbnails | WebAudio peaks | Engine peaks |
| Source preview monitor (`viewer/source-preview.ts`) | `<video>`, `<audio>` | Engine frames, engine audio |
| Timeline waveforms (`timeline.ts:1703-1717`) | WebAudio peaks | Engine peaks |
| Mixer meters, solo, retune | WebAudio analysers | Engine |
| Caption rendering (`captions/render.ts`) | Canvas2D | `pm-text`, `pm-vector` |
| Text editor caret and font offset (`viewer/text-editor.ts`) | `textLayout`, `raster().fontOffset` | `pm-wasm` |
| Text animators, text stagger, editor helpers (`TextAnimators.svelte:143`, `text-stagger.ts:30`, `editor-helpers.ts:15-17`) | `textLayout`, `Intl.Segmenter` | `pm-wasm` |
| Align, group and comp bounds (`shortcuts.ts:152,227`, `layer-groups.ts:51`, `compositions.ts:505`, `AlignmentStrip.svelte`) | `PM.GL.bounds` | `pm-wasm`, `pm-edit` |
| Model-group bounds (`layer-groups.ts:43`) | three.js `Box3` | `pm-edit` |
| 3D picks, bounds, ordering (`compositor.ts:1896-1946`, `scene3d/service.ts:80-108`) | three.js runtime | `pm-wasm` |
| 3D viewport overlay (`scene3d/overlay.ts`, `gizmo.ts`) | Own WebGL context drawing the three.js scene | `pm-wasm` mesh buffers |
| Edit Mode, modal transforms, 3D cursor, navigation (`editmode.ts`, `editmesh.ts`, `modal.ts`, `operators.ts:185`, `navigation.ts`, `targets.ts`, `controller.ts`, `viewport.ts`) | three.js geometry and raycasts | `pm-wasm` |
| Blender preview, state, and capture (`scene3d/rendering.ts`, `frame-preparation.ts:14-50`, `RenderingControls.svelte`, `Header3D.svelte`) | ImageBitmap surfaces in the renderer | Engine Blender jobs and events |
| Recipe preview (`scene3d/recipe-preview.ts`) | TypeScript approximation | `pm-geom3d` |
| Model asset import and disposal (`raster.ts:682-687`, `import-model.ts`, `import-bundle.ts`) | three.js loaders and exporter | Engine import |
| Export cancel to Blender (`exporter.ts:387`) | `blender.cancel` | Engine job cancel |
| `get_3d_scene` (`harness.ts:604-608`) | `evP` per channel | `pm-wasm` |
| Shader compile status | `compileError`, `dropProgram`, program keys | Engine compiler status |
| Status bar | GPU loss, compiling, per-extension GPU timing | Engine stats |
| Performance panel and store | `GL.stats`, `rasterStats` | Engine stats |
| Frame bus | `overlay` after `GL.render` | On presentation |
| Export progress preview | Live frames | Streamed frames |
| RAM preview | `ImageBitmap`s | Render cache |
| Posters | Seeks `asset.el` | Engine posters |
| Cloud and remote media | Bytes through the renderer | Main materializes, engine probes |
| Image sequence import | `createImageBitmap`, VP9 encode | Engine sequence reader |
| Fonts | `document.fonts`, `queryLocalFonts` | Font registry and subsets |
| Panel popout | Same-realm child windows | Frames to the hosting window |
| Agent frames (`harness.ts:177-202`, `spatial.ts:2169,2284,2399,2588`) | `snapshotAsync`, pauses playback | Batched engine frame requests |
| Agent export and save (`assistant/mcp-project-tools.ts`, `bridge.ts:479-484`, `agent-export.ts`) | `PM.Export.run` in the owner renderer | Job engine with main-minted output handle |
| Composition contact sheet (`agent-tools/media-tools.ts`) | `render_frames` loop | Engine batch |
| Hidden MCP host (`server/mcp.ts`) | Playwright Chromium, SwiftShader | Headless document host plus `pm-node` |
| Serve routing (`server/runs.ts:20-23,194`) | Agent exports to the document host | Job engine |
| Web player, www, onboarding | TypeScript engine with three.js | Web player v1 |
| Headless document host (`src/engine/main.ts`) | `legacy/core` under happy-dom | `pm-wasm` |

## 33. Appendix D: media and proxy interface

All surfaces follow the design language: quiet text in lists, Settings-sheet
cards and rows for dialogs and sheets, NSMenu only for right-click, the app's
popover menu for button menus, the app's confirm dialog for confirmations,
sentence case in Settings, title case in menus.

### Settings › Project › Media

| Setting | Options | Phase |
|---|---|---|
| When importing | Link to files, Copy into project | 2 |
| Create proxies | When needed, For all video, Only when I ask | 4 |
| Proxy size | Automatic, Half, Quarter, 1080p, 720p (Automatic never below 540p) | 4 |
| Proxy format | ProRes 422 Proxy, ProRes 422 LT, HEVC (smaller files) | 4 |
| Create optimized media | Only when I ask, When needed, For H.264 and HEVC video | 4 |

### Settings › Project › Composition

Motion blur samples; audio sample rate.

### Settings › General › Media (this Mac)

| Setting | Options |
|---|---|
| Generated media location | Powermove cache, Next to the project, Choose Folder… |
| Background rendering | When plugged in, Always, Never |
| Storage | Space used by proxies, optimized media, rendered frames, Blender renders, media index, thumbnails and waveforms; each with Clear… and a limit |

### Asset right-click menu

Create Proxy · Create Optimized Media · Rebuild Proxy · Delete Proxy and Optimized
Media · Always Play Original (check item) · Interpret Footage… · Replace File… ·
Reveal in Finder.

### Viewer menu

The viewer's resolution control (a native `<select>` today, `viewer/viewer.ts:883`)
becomes a popover menu. `controls/popover-menu.ts` gains per-section headers, one
radio group per section, and a check item:

| Section | Items |
|---|---|
| Resolution | Automatic, Full, Half, Quarter |
| Media | Automatic, Originals, Proxies |
| (none) | Play Every Frame (check item) |

The button label states what is on screen when degraded: "Half · Proxy". It shows
"Full" once a paused frame upgrades. "Proxies" works like Final Cut Pro's Proxy
Only. A keymap command toggles Originals and Proxies. The 3D viewport's shading
modes stay in its own header, as today.

### Status and diagnosis

- "Creating proxies · 3 of 12"; "Proxies paused · Low disk space". Unchanged
  during playback.
- "Dropped 14 frames · 2 clips decode slowly", action Create Proxies; or "Dropped
  14 frames · GPU busy".
- "Couldn't create a proxy for A001_C003.mov. The file ends early at 01:12:04.
  Copy the complete file, then choose Rebuild Proxy."

### Storage

Clear… confirms: "Clear proxies?" with "This frees 48.2 GB. Powermove creates them
again when needed. Proxies for 3 clips whose originals are offline are kept."
Pinned media is never removed.

### Offline media and Blender at export

- Native three-button message: title "2 clips are offline"; body "Their original
  files can't be found. Locate them to export at full quality, or export these
  clips from their proxies at lower resolution."; buttons Cancel, Export with
  Proxies, Locate Media….
- Queue: "Waiting for media · 2 clips offline". Draft option: "Render using:
  Proxies (draft)".
- Blender missing: "This composition renders with Blender. Install Blender 4.5 or
  later, or set its renderer to Native."

### Import and relink

- "Linked 12 clips · Copied 3 images".
- "Media offline · A001_C003.mov"; timeline label "Offline".
- "Using a converted copy · Relink to the original for full quality", Locate
  Original….
- "Variable frame rate · Plays at 60 fps by timestamp".

### Render cache

A 2-pixel track under the ruler; View › Show Rendered Frames; Render Work Area;
Render Selection; Delete Rendered Frames; Keep Rendered on precomps.

## 34. Appendix E: glossary

- **Tick:** 1/705,600,000 second.
- **Display encoding E:** the sRGB piecewise curve shared by sRGB and Rec.709 by
  default.
- **Standard color:** intermediates in E; today's look.
- **Linear blending:** "Blend in linear light" on.
- **Depth block:** a contiguous run of 3D layers sharing a depth prepass and shadow
  maps.
- **View override:** editor camera, shading, X-ray, Local View; affects preview
  only.
- **Blender surface:** a per-layer image rendered by Blender and composited by the
  engine.
- **Canonical patch:** a document patch without the open-composition root rotation.
- **Mirror:** a patch-fed copy of the document (`pm-wasm`, engine sessions).
- **Write barrier:** records every document write so mirrors flush before queries.
- **Edit kernel:** a geometry-dependent edit computed in `pm-edit`.
- **Definition snapshot:** a registered effect, transition, or extension layer's
  renderer kind, GLSL, and parameters, pushed to sessions and embedded in exports.
- **Priority classes:** interactive, agent snapshot, job.
- **Representation:** original, reduced-resolution decode, proxy segments,
  optimized segments, kept render.
- **Segment:** a ~2-second keyframe-aligned piece of generated media.
- **Machine profile:** measured decode, GPU, disk, and power characteristics.
- **Lease:** a memory allowance granted by main to an engine process.
- **Frame bridge:** the shared-surface path from the engine to the viewer.
- **Interactive engine, job engine:** the two engine processes.
- **Hidden MCP host:** `powermove mcp`, the headless agent host.
- **`compat` level:** per-layer compatibility marker.
- **Parity corpus:** projects and expected outputs captured from the legacy engine.
- **E2E ledger:** quarantined end-to-end specs with owners and reasons.
