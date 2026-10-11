# Powermove native engine: plan and design

Status: proposal, not started. Revision 5, reconciled with main at `d5d5b731`
and audited by five independent reviewers (architecture and readiness, 3D and
Blender, agents and security, color and video, preservation).

This document covers the whole rewrite of the Powermove engine (evaluation,
decode, compositing, 3D, color, text, audio, playback, caching, proxies, and
export) from TypeScript, WebGL2, and three.js into Rust. The Svelte UI, the
kernel, extensions, the 3D viewport editor, and the agent harness stay on the web
stack. It is written to be executed by parallel implementation lanes. Every phase
ends at a gate with objective exit criteria; a phase is done when its gate passes,
not when its code lands.

"The engine" means the new native engine. The existing Node host in
`apps/desktop/src/engine/` is "the headless document host"; `powermove mcp`
(`apps/desktop/src/server/mcp.ts`) is "the hidden MCP host".

## Summary

**Three hard requirements**

1. **Every feature is preserved**, including every shader, effect, transition,
   extension layer, 3D feature, export option, and agent tool. A feature that
   cannot be reproduced exactly gets a compatibility path, never a removal.
2. **The kernel and extension system stays intact.** Extensions at apiVersion 1
   to 3 keep loading and behaving the same; agents keep writing extensions in
   TypeScript and Svelte against the `powermove` API, with GLSL for effects,
   transitions, and shader layers.
3. **The old TypeScript engine leaves the editor at cutover.** What remains
   anywhere else is listed in section 22 and decision 8.

**What users get at cutover**

- ProRes, DNxHR, MXF, HEVC 10-bit, and every format imported today play and export
  from the original files, never from a lossy transcode.
- Float precision end to end, correct color tags, exact frame rates and timecode,
  sample-accurate audio that follows speed changes.
- 3D model, light, and camera layers rendered natively, and Blender-rendered
  compositions composited at 16-bit.
- Antialiased edges, sharp scaled text and vectors, correct blur at every radius
  for new layers, with existing projects unchanged.
- Optional linear-light blending.
- Automatic proxies with full control and a viewer that always says what it is
  showing; exports never contain proxy pixels by accident.
- A working render queue and editing while exporting.
- Agents, MCP clients, and the CLI render and export on the GPU, concurrently,
  without interrupting the user.

**After cutover** (each its own plan): ACES, wide-gamut and HDR projects, scopes,
LUTs and color effects, more delivery formats, Windows, cameras and lights for
2.5D layers, camera RAW, OpenFX, the second half of proxy and media management
(Phase 4b), the WASM web player (unless decision 8 moves it earlier).

**Phases**

| Phase | Outcome | Size |
|---|---|---|
| 0 Foundations | Contracts drafted, spikes proven, secure CI hardware, parity corpus captured | L |
| 1 Evaluation | The Rust evaluator answers the UI's synchronous 2D queries, with a write path that keeps it exact | L |
| 2 Tracer bullet | Engine processes, frame bridge, ProRes and H.264 from originals, one 3D layer, media store, agent export on the job engine | XL |
| 3a Rendering parity | Every layer, effect, text, mask, blend, motion blur, 2.5D and 3D feature, Blender compositing | XL |
| 3b Media and delivery | Every import and export format today, model import, Blender jobs, render queue, audio | L |
| 3c Integration | Kernel API, the 3D viewport, agents and MCP, every engine consumer, CLI | L |
| 4a Proxies for cutover | Automatic proxies with full controls, export safety, offline handling, cached preview | M |
| 5 Cutover | 5a opt-in beta, 5b default on, 5c legacy removed | M |

3a, 3b, and 3c start together at the Phase 2 gate; the 3c gate requires 3a and 3b.

**Before any Phase 0 release:** the bundled FFmpeg is replaced as a standalone
compliance fix (section 24, item 1).

**Decisions** are in section 29; none remain open.

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
29. Decisions
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
  intermediate codecs. Powermove transcodes them on import to VP9 `yuva420p` (CRF
  18) with Opus audio (`main/media-proxy.ts:39-51`), and that transcode is what
  gets stored, saved, and exported. The original is never kept
  (`legacy/gl/raster.ts:887-902`).
- **Precision.** Source textures are RGBA8 with premultiply-on-upload
  (`legacy/gl/compositor.ts:546-550`). Intermediates are RGBA16F; export readback
  is 8-bit (`compositor.ts:1871`). ProRes 4444 is written at 10-bit from 8-bit
  data. 3D layers arrive as 8-bit three.js canvases and 8-bit Blender PNGs.
- **Color.** No color management. Blending, blur, opacity, and motion blur run on
  gamma-encoded values. Import ignores color metadata. Export tags H.264 and ProRes
  with the sRGB transfer.
- **Time.** Float seconds everywhere. Four paths set composition frame rates
  differently (the UI and `set_composition` round; load and validation keep
  fractions; Home's media matching copies a file's average rate,
  `legacy/app.ts:1338-1352`; MCP `create_project` accepts any float). Timecode is
  wrong at fractional rates.
- **Audio.** WebAudio with whole-file decode and a 128 MiB PCM budget (about 5.8
  minutes of stereo float). Speed changes are ignored; time-remapped clips are
  silent.
- **3D.** Model layers render through three.js in a second WebGL context and are
  copied into the compositor once per layer per frame. Blender renders are
  prepared by the renderer before each export frame, one render per 3D layer per
  motion-blur sample, serialized through one worker.
- **Export.** Frames are rendered in the renderer, read back, and copied over IPC
  in 4 MiB chunks to FFmpeg. H.264 export of a trivial 4K project measures about
  11 fps (`apps/desktop/docs/native-export-performance.md`).
- **Headless.** `powermove mcp` drives a Playwright Chromium on SwiftShader
  (software WebGL) with one project and one export per host. Agent frame captures
  pause the user's playback and fail while another capture or an export runs
  (`exporter.ts:856-870`), and agents now capture several frames on every turn.

## 2. Goals, non-goals, and the quality bar

### Goals

1. **Preservation.** Every feature and every extension contract keeps working
   (Summary).
2. **Image fidelity.** Float precision from decode to encode.
3. **Color correctness.** Explicit, tested color invariants; per-source
   interpretation; optional linear-light blending; later OCIO, ACES, HDR.
4. **Native codecs** from the original media.
5. **Exact time.** Rational rates, integer ticks inside the engine, frame-accurate
   decode, drop-frame timecode, sample-accurate audio.
6. **One renderer** for preview, export, thumbnails, agent snapshots, MCP, and the
   CLI, 2D and 3D.
7. **Proxies done right** (section 14).
8. **Speed.** Real-time multi-layer 4K playback on Apple Silicon, pipelined export,
   no rendering on the UI thread, concurrent agent renders.
9. **Portability.** Platform code behind traits; Windows is a port.

### Non-goals before cutover

- Rewriting the UI, timeline, inspector, 3D viewport editor, or kernel in Rust.
- Changing the document's JSON shape or the public API's units (section 6).
- New delivery formats beyond today's list; ACES, HDR, and wide-gamut projects;
  scopes as a product; camera RAW; OpenFX; cameras and lights for 2.5D layers;
  new 3D authoring features; bundling Blender; Linux engine prebuilds. These
  follow cutover.

### The quality bar

Each line is an automated test (section 25). CV means code value. Measurements
that need lossless output use test-only uncompressed sinks in `pm-cli` (raw
`y416`, `x422`, `x420` buffers, 16-bit TIFF, 16-bit PNG) and the VideoToolbox
encoder-input buffer. Dither is off for every bar unless stated.

| Area | Bar | Phase |
|---|---|---|
| Color invariants, Standard | (1) A Rec.709 source exported to Rec.709 at the same bit depth, range, and subsampling, with no effects, matches a reference decode exactly for 4:4:4 and co-sited 4:2:2 at 32-bit precision; for 4:2:0, luma exactly and chroma within 1 CV. (2) A solid set to `#RRGGBB` decodes to the same codes exactly for sRGB PNG, within 1 at 10-bit and above, and within 2 for 8-bit limited-range Y′CbCr, with greys exactly neutral. (3) An export using the "Rec.709 (QuickTime and Safari)" output, played through `AVPlayerItemVideoOutput` into the display profile, matches the viewer's ColorSync conversion within ΔE2000 1.0 on a test chart. | 2 |
| Color invariants, linear blending | Bars (1) and (2) also hold with linear blending on. | 3a |
| Pipeline transparency | At 32-bit precision, ProRes 4444 XQ decode → engine (no effects) → `y416` sink matches a VideoToolbox reference decode exactly at 12-bit; ProRes 422 → `x422` sink matches exactly. | 2 |
| Codec generation | Engine ProRes and H.264 exports versus Apple's re-encode (`AVAssetExportSession`) of the same decoded frames: maximum 1 CV at 10-bit and PSNR within 0.1 dB. | 2 |
| Bit depth | A 16-bit grey ramp across at least 1024 columns, exported to a 10-bit limited-range preset (ProRes 422 HQ), contains every luma code 64 to 940 (877 levels), is monotonic, steps at most 1 CV, and keeps Cb = Cr exactly neutral; a chroma ramp covers 64 to 960 (897 levels). 16-bit PNG output is monotonic with maximum step ⌈65535 / (columns − 1)⌉. With default dither on an 8-bit output, the 16-column local mean is within 0.5 CV of ideal. | 3b |
| Alpha | ProRes 4444 alpha is exact at the source alpha depth; straight and premultiplied outputs both correct. HEVC alpha PSNR at least that of AVFoundation's HEVC-with-alpha preset on the same frames. | 3b |
| SDR color | Patch centers within 1 CV of analytic expectations; ΔE2000 maximum 1.0 and mean 0.5 at 8-bit, maximum 0.5 at 10-bit; two references (FFmpeg with zimg, and AVFoundation). | 3b |
| Linear blending | Computed at 32-bit, round half up, sRGB output: 50% white over black is 188 (0.735357; boundary 187.5). A 1-pixel checkerboard downscaled 2× is 188. A Gaussian blur of a white line conserves energy within 0.5%. | 3a |
| 3D | Native 3D layers against three.js r180 renders of the 3D corpus: at most 2 CV for 99.5% of pixels, at most 12 CV for any pixel outside MSAA edges and shadow penumbrae, and per-object SSIM at least 0.99. Blender surfaces are exact at 32-bit precision. Blender snapshots equal today's (section 25). | 3a |
| Time | Frame math verified headlessly to 10⁸ frames at every supported rate, including negative frames. Exports burn in a binary frame code and timecode string: 10,000 consecutive frames per rate (23.976, 24, 25, 29.97 DF and NDF, 47.952, 59.94, 60, 119.88, 239.76) plus 100-frame windows at N = 10⁶ and 10⁷; the `tmcd` track matches. Vectors: 29.97 frame 1799 = 00:00:59;29, 1800 = 00:01:00;02, 17982 = 00:10:00;00, 24-hour wrap at 2,589,408; 59.94 frame 3600 = 00:01:00;04, 35964 = 00:10:00;00. 23.976 and other high-rate NTSC count non-drop. Invalid drop-frame strings are rejected. | 2 |
| Seeking | For every supported codec, including open-GOP HEVC, B-pyramid H.264, edit lists, interlaced MXF, and coarse timebases (1 ms WebM, 90 kHz TS, 1/600 MOV at 23.976 and 29.97), random access to frame N equals sequential decode of frame N. | 3b |
| Audio | A click at frame N lands at sample floor(N × sr × den / num) ± 1. Total exported samples, after edit-list trimming, equal round(duration × sr). No length limit. | 3b |
| Antialiasing | Zone-plate and Siemens-star renders under animated scale 1.0 → 0.25 and rotation, against the path's declared kernel at 16× supersampling: PSNR ≥ 40 dB on export and paused frames (Lanczos), ≥ 34 dB during playback (trilinear); flicker energy at most 1.5× the reference. Applies to new (`compat` level 2) content; level 1 content matches today. | 3a |
| Cadence | 23.976 and 25 fps content on 60 Hz and 120 Hz displays shows the ideal cadence with zero irregular repeats over 60 s. | 2 |
| Proxies | No exported frame contains proxy or reduced-quality cache pixels unless the user chose a draft render. A paused frame shows the original within 150 ms (4K ProRes) and 300 ms (4K long-GOP HEVC) on the base machine. On a chart sequence at matched scale and resampler, proxy versus original: patch-center ΔE2000 mean ≤ 0.5 and maximum ≤ 1.0; full-frame 99th percentile ≤ 3.0. | 4a |
| Parity | Every corpus project renders in Standard color within the tolerances of section 25. | 3a |
| Determinism | Two renders of the same frame produce byte-identical buffers and lossless outputs; native and WASM evaluation are bit-identical. Blender-rendered layers are excluded; their surfaces are cached by content key and never re-rendered within a job. | 1 and 3a |
| HDR color | PQ and HLG within ΔE_ITP (BT.2124) 1.0. | After cutover |

## 3. What exists today

Full inventories: appendices B and C.

### Code the engine replaces

Non-test lines at `d5d5b731`, excluding tracked `* 2.ts` duplicates.

| Area | Main files | LOC |
|---|---|---|
| Evaluation, animation, expressions, retiming, 2.5D math, text animators | `legacy/core/{anim,temporal-bridge,easing,expression,retiming,parenting,layer-groups,space-3d,text-animation,text-layout,split-text,vector-paths,content-properties,size-anchor,font-anchor,matte}.ts`, `core/anim/temporal-ease.ts` (evaluation parts) | ~3,200 |
| Compositor, rasterizer, shaders, GL caches, asset import half of `raster.ts` | `legacy/gl/*` | ~4,290 |
| 3D evaluation and native renderer, model import, Blender orchestration in the renderer | `core/scene3d/{runtime,rendering,service,recipe-preview,import-model,import-bundle}.ts`, the evaluation parts of `layers.ts` and `materials.ts` (`compositionScene`, `materialPreview`), 3D hooks in `frame-preparation.ts` | ~2,000 |
| Media import, decode, posters, editing copies, preview caches | `legacy/core/{media,media-conversion,video-*,image-sequence,poster,preview-cache,preview-quality,memory,cloud-media}.ts`, `main/media-proxy.ts` | ~1,900 |
| Playback loop | `legacy/core/engine.ts` | ~500 |
| Audio and mixer engine | `legacy/core/audio.ts`, non-UI mixer | ~1,830 |
| Export | `legacy/core/{exporter,frame-capture,frame-preparation,render-queue}.ts`, `main/render-encoder.ts` | ~1,040 |
| Built-in effect and transition rendering glue (the GLSL bodies stay, section 11), legacy mesh rendering | `extensions/{effects-basic,transitions-basic}` binding code, `kernel/obj.ts` rendering use, `captions/render.ts`, `legacy/core/fonts.ts` | ~950 |

About 15,700 lines, plus the parts of three.js r180 (MIT) whose output rendering the
engine reproduces. The Rust replacement will be larger, because it takes over
decode, color, text shaping, audio, and 3D work done today by the browser and
three.js.

### What stays

- The renderer UI, panels, Settings, built-in UI extensions, the bottom dock, the
  Track timeline.
- The kernel and its public API (section 20), including `kernel/glsl.ts` and
  `kernel/extension-layers.ts` as validators and GLSL generators,
  `kernel/sandbox-schemas.ts`, and the extension sandbox.
- The edit pipeline (`legacy/core/editing.ts`), history and Undo, the 25-command
  vocabulary, validation, composition management (`legacy/core/compositions.ts`),
  and SVG parsing into editable paths (`legacy/core/svg-import.ts`), with geometry
  calls redirected to `pm-wasm`. These move out of the `legacy/` folder at
  cutover.
- The 3D viewport editor (`core/scene3d/{controller,modal,overlay,gizmo,navigation,viewport,operators,chrome,targets,editmode,editmesh,walk,editor-state,commands,theme}.ts`,
  `extensions/viewer/viewport3d/*`, the `extensions/layers-3d` UI), the 3D document
  side (`core/scene3d/{operations,modeling,api,schema,migration}.ts` and the
  validation parts of `layers.ts` and `materials.ts`). three.js stays in the
  renderer for viewport chrome only (gizmos, grid, wires, Edit Mode points); it
  never produces output pixels.
- The Blender host (`main/blender-{service,worker,ipc}.ts`, `shared/blender.ts`),
  supervised through host services (section 19).
- The agent stack: harness, orchestrator, tool bridge, public MCP
  (`server/mcp.ts`, `main/agent-tools/external-*`, `mcp-stdio.mjs`,
  `mcp-client.mjs`, `--powermove-mcp`), Claude Desktop registration, agent export
  delivery, main-process media tools.
- The `.pmv` container and its durability, including read-only file saves.

### Constraints inherited from earlier decisions

From `apps/desktop/docs/*` (including `SCENE3D.md`, `MCP.md`, `EXTENSIONS.md`):

- One compositor for preview, capture, export, and nested comps.
- Pixel-exact acceptance for optimizations.
- Bounded expression interpreter (2,000 characters, 160 nodes, depth 24, 12
  arguments), no `'unsafe-eval'` in the editor.
- One user intent equals one Undo step.
- Effect contract limits: in-realm effects 32 params, 1 to 8 passes, fragment at
  most 65,536 characters; in-realm extension layers 48 params and 96,000 UTF-8
  bytes; Store (sandboxed) extension layers 32 params and 65,536 characters
  (`kernel/sandbox-schemas.ts:18`).
- 3D limits (may be raised, never lowered): 512 active models and 16 active lights
  per render (exceeding fails the 3D render); 4 shadow-casting lights; 500k vertices
  per scene; imports up to 128 MB GLB or OBJ bundle, 1M vertices, 500k triangles,
  4,096 nodes, 1.5M per accessor; OBJ 1M vertices and 500k triangles; mesh source
  300k numbers; 2,400 keys per channel; native 3D output at most 8192 px; legacy
  scene layers 4,096 px; nesting depth 8; 4,096 nested Blender frames; Blender 4.5
  or newer, inputs up to 256 MB, outputs up to 512 MB, 1.5M render vertices, 33,333
  triangles per object, 500k generated vertices, 64 MB snapshots, 512 assets, a
  queue of 16 jobs, 10-minute job timeout; materials up to 128 nodes, 256 links, 64
  inputs, 32 ramp stops, 12 modifiers, 64 slots. Exports fail rather than deliver
  draft shading when a Blender surface is missing.
- Runtime rendering for agents uses the production renderer, never a parallel
  implementation of the document model (`server/mcp.ts:26-27`). The engine is the
  production renderer and the document model stays TypeScript.
- MCP agents never choose output paths (`main/agent-export.ts`). `powermove render`
  on the CLI accepts any path.
- Agents get full freedom: no network or read restrictions on agents; only Store
  extensions are contained.
- Per-layer failure isolation; GPU loss recovery without replacing the project;
  predictability over raw speed.

## 4. Architecture overview

```
┌──────────────────────────── Electron app ──────────────────────────────┐
│  Renderer, per window (sandboxed, context-isolated)                     │
│  ├─ UI, kernel, extensions, edit pipeline, history (TS, seconds)        │
│  ├─ pm-wasm: canonical mirror, evaluator, 3D geometry, text layout,     │
│  │           edit kernels                                               │
│  ├─ Viewer: WebGPU canvas drawing engine frames + DOM/2D overlays       │
│  └─ 3D viewport chrome (three.js, no output pixels; uses engine depth)  │
│        ▲ MessagePort: canonical patches, control, events                │
│        │                                     ▲ VideoFrame               │
│  Main process (host services)                │                          │
│  ├─ windows, files, save, media store, budget and GPU arbiter,          │
│  │  capability handles, output destinations, agent bridge, external MCP │
│  └─ pm-bridge (napi): Mach rendezvous, surface import ─┘                │
│        ▲ surface rights once; per-frame descriptors                     │
│  Interactive engine (utilityProcess, pm-engine via napi-rs)             │
│  Job engine (second utilityProcess): exports, agent snapshots and       │
│  exports, generated media, background renders                           │
│  Blender worker (optional, user-installed, supervised by main)          │
└─────────────────────────────────────────────────────────────────────────┘
Other hosts: pm-cli (headless renders, parity); the hidden MCP host after
Phase 3c (per-project headless document hosts on pm-wasm + a pm-node engine
child process + host services in the CLI); pm-wasm in serve's document host.
```

1. **No engine work on a UI thread.**
2. **Synchronous answers stay synchronous.** About 150 built-in call sites and about
   23 in the 3D viewport editor ask for evaluated values, geometry, picks, bounds, or
   text layout synchronously. `pm-wasm` answers from a mirror after flushing pending
   writes (section 6). Frozen synchronous render APIs keep working through a
   compatibility path (section 20).
3. **Frames stay on the GPU.** A pool of shared surfaces imported once into main; one
   small message per frame (section 18).
4. **One renderer** for every output, 2D and 3D.
5. **Two engine processes.** A crash in one does not take down the other.
6. **Host services.** Arbitration, capability handles, output destinations, Blender
   supervision, and definition snapshots come from one interface implemented by
   Electron main and by the CLI host (contract 14).
7. **Priority across processes.** Interactive frames, then agent snapshots, then
   jobs; enforced by GPU time slicing (contract 13). Agent captures queue instead of
   failing and never pause playback.
8. **Transport-agnostic protocol.**

### Crate layout

A Cargo workspace at `engine/`.

| Crate | Responsibility | Targets |
|---|---|---|
| `pm-time` | Ticks, rationals, frame math (i128, Euclidean), timecode | native, wasm |
| `pm-model` | Canonical model, `v1_ingest`, id-keyed reconciliation, object-identity registry, schemas | native, wasm |
| `pm-eval` | Keyframes, ease, expressions, parenting, groups, 2.5D, retiming, text animator state, paths and masks, 3D content and namespaces, aims, recipes, pick and bounds | native, wasm |
| `pm-geom3d` | Primitive, lathe, extrude, recipe-preview, indexed meshes identical to today's; glTF 2.0 decode and animation; BVH; bounds; depth sets and ordering | native, wasm |
| `pm-edit` | Geometry-dependent edit kernels returning patches | native, wasm |
| `pm-text` | Font registry, subsetting, shaping, layout, outlines, caret geometry | native; wasm (layout) |
| `pm-vector` | Paths, trim, repeat, strokes, booleans, squircles, coverage | native; wasm (geometry) |
| `pm-color` | Transfer functions (extended), tone mapping, chroma, gamut, OCIO bridge (after cutover) | native |
| `pm-gpu` | wgpu device, pools, shader compilation, GLSL compatibility, interop trait, GPU time slicing | native |
| `pm-render` | Render graph, 2D layers, 3D scene pass, effects, compositing, motion blur | native |
| `pm-blender` | Blender job client: snapshots from the evaluated scene, protocol, surface import, cache keys | native |
| `pm-media` | Probe, demux, decode, index, image readers, color and chroma metadata | native |
| `pm-cache` | Frame cache, render cache, disk caches, leases | native |
| `pm-proxy` | Representation policy, jobs, generated-media store | native |
| `pm-audio` | Device and clock, mixer, automation, stretch, waveforms, meters | native |
| `pm-export` | Encoders, muxers, render queue, verification | native |
| `pm-engine` | Sessions, scheduler, priority classes, playback, crash attribution | native |
| `pm-protocol` | Messages, MessagePack codec, TypeScript types via `ts-rs`, contract types | native, wasm |
| `pm-host` | Host services interface and the CLI implementation | native |
| `pm-node` | napi-rs host for engine processes | native |
| `pm-bridge` | napi-rs addon in main: rendezvous, surface import, font registry before Phase 2 | native |
| `pm-wasm` | wasm-bindgen exports | wasm |
| `pm-cli` | Headless binary, parity runner, test sinks | native |

## 5. Time

### Inside the engine

- **Ticks.** `i64` ticks of 1/705,600,000 second (the flick). 705,600,000 =
  2⁹·3²·5⁵·7². Supported frame rates (23.976, 24, 25, 29.97, 30, 47.952, 48, 50,
  59.94, 60, 90, 100, 119.88, 120, 239.76, 240) and sample rates (8 to 192 kHz)
  divide it. Range about ±414 years.
- **Rates** are `Rational { num, den }`.
- **Frame math** in i128 with Euclidean division:
  `frame_start(n) = ceil(n × den × TICKS / num)`,
  `frame_of(t) = (t × num).div_euclid(den × TICKS)`; property test
  `frame_of(frame_start(n)) == n` for every supported rate and arbitrary rationals,
  including negative n, up to 10⁸.
- **Media timebases** that do not divide the flick convert in i128 with
  round-half-even. The media index keeps native PTS.
- **Timecode** only at UI and IO edges, SMPTE drop-frame for 29.97 and 59.94.
  `api.util.tc` keeps its format and signature; drop-frame display is a new
  function.

### At the document boundary

The document and public API keep float seconds (section 6). `pm-model::v1_ingest`
converts in one place:

1. Compute `n = round(t × fps_f64)` using the f64 `fps` the document stores. If
   `|t × fps_f64 − n| < 1e-6 × max(1, |n|)`, store `frame_start(n)` under the exact
   rational. This keeps every frame-aligned time on its frame in projects saved with
   the float 29.97, at any length.
2. Otherwise convert to the nearest tick.
3. Rates come from `fpsRational` (new optional `[num, den]`) when present.
   Otherwise: a float within 0.02% of k × 1000/1001 for k in {24, 30, 48, 60, 120,
   240} maps to k × 1000/1001; otherwise a float within 0.02% of an integer
   supported rate maps to it; anything else becomes the smallest rational within
   1e-9.
4. Writing back: `pm-edit` echoes the original f64 for every time it did not change.
   Times it creates are `ticks / TICKS`.

Dual-answer comparison (section 6) feeds the legacy evaluator the same snapped
times, so snapping never counts as a mismatch.

### Rules

1. **One frame-rate validator** for the UI, `set_composition`, load, validation,
   Home's media matching, and MCP `create_project` (Phase 1). It stores `fps` plus
   `fpsRational`. New rates must divide the flick; unusual rates already in projects
   keep working through the ceil and Euclidean math.
2. **Media-derived rates** use the source's nominal rate (section 13), never the raw
   average of a variable-rate file.
3. UI-created keyframes land on frame boundaries in composition time.
4. Nested compositions sample their children at the parent time by default. "Keep
   frame rate when nested" quantizes the child to its own rate.
5. Variable speed integrates in ticks at the owning composition's rate.
6. Export frame rate can differ from composition rate.
7. **Source frame selection:** the last frame with PTS ≤ t + q/2, where q is the
   stream's timebase unit, capped at a quarter of the nominal frame duration. This
   removes duplicates and skips on coarse timebases. Frame blending and optical flow
   come after cutover.
8. **Motion blur** sample times are t + phase + (i + ½)/N × angle. Today's default
   (0.5 frame, centered) is 180° at −90°.
9. Expressions see `time`, `t`, and `fps` as floats, plus a new integer `frame`.
10. **3D timing.** Shared-material channels keep composition timing; keys may lie
    before `layer.from`, down to −10⁸ s local. glTF clips evaluate at
    `max(0, t − layer.from)`, ignoring speed, trim, and time remap. Recipe
    parameters are evaluated per frame; natively, animated recipes rebuild without
    modifiers and 3D Text never rebuilds (as today).

## 6. The document: ownership, writes, mirrors, engine facts

### Authority

The renderer stays the authority for the document and Undo. The JSON, the api-pack
types, the kernel API, and `EditCommand`s keep float seconds and their shape.

Existing fields ingested as they are: composition `render3d`; layer
`d.data.{object,light,camera,environment}` (with `object.generation` and
`object.blender`); group `d.modeling`; asset `fps` (a hint) and `format`.

Additive fields before cutover. The legacy column says what the legacy engine does
while it is still selectable.

| Field | Purpose | Legacy behavior |
|---|---|---|
| `fpsRational` | Exact composition rates | Ignored safely |
| `minReaderVersion` (root) | Older builds refuse instead of overwriting | Read by the Phase 0 guard |
| `requiresEngine: "native"` (root) | Set when a native-only feature is used | Legacy refuses the project |
| Layer `compat` | 1 for layers created before cutover (today's text metrics, solid radius ignored, today's effect looks); 2 for new layers. Duplicate, paste, split, pre-compose, and clone keep the source's level | Ignored safely (level 1 behavior) |
| Asset `link` | Linked-media reference | Requires native |
| Asset `interpret` | Interpretation overrides | Requires native |
| Asset `storage` | `copied` or `linked` | Requires native when `linked` |
| Composition `motionBlurSamples` | Per output kind; migrated projects get today's counts (16 opaque, 20 alpha and WebM, preview 6 and 12) | Ignored safely |
| Composition `audioSampleRate` | Project audio rate | Ignored safely |
| Precomp `keepFrameRate` | Keep frame rate when nested | Requires native when true |
| Clip `pitch` | Preserve or follow speed | Requires native when not default |
| Clip `alwaysOriginal` | Always Play Original | Ignored safely |
| Precomp `keepRendered` | Kept render | Ignored safely |
| Project `color` | Linear blending, precision, blend-mode domain | Requires native when non-default |
| Project `media` | Proxy and import settings | Ignored safely |

The table is the input to contract 5. The engine-switch round-trip gate runs from
Phase 2 (section 21).

Not in the document: representations, probe results, decode cost, Blender
availability. They live in the engine's media index and generated-media store.

### The patch stream

1. The renderer produces leaf patches (`src/shared/patch.ts`) in canonical form
   (`PM.Comps.canonicalize` for every patch).
2. `pm-wasm` ingests them through `v1_ingest` and applies them with **id-keyed
   reconciliation**.
3. `pm-wasm` emits canonical binary patches to engine sessions. The job engine
   subscribes to the same stream, so agent snapshots read their own writes.

### Making every write visible

The writer inventory is generated by a checked-in script
(`scripts/engine-writers.mjs`) whose output CI diffs; new writers fail CI until
classified. At `d5d5b731` it covers direct `PM.hist.do` sites, `api.history.do`
calls, `Edit.mutate` sites, `PM.touch` and `api.anim.touch` sites, asset metadata
writes in `raster.ts`, `shaderHooks.syncShaderUniforms`, extension-layer refresh,
the render queue's project swap, Home's composition matching, Blender source assets
(`scene3d/modeling.ts:367,501`), agent media folders, shared-material fan-out
(`editing.ts:199-209`), aim-preserving rewrites (`parenting.ts:21-29`), revision
counters, and `api.host.pm`.

1. **Write barrier.** Every write records its dirty path. Development and test
   builds wrap `PM.proj` in a Proxy that records paths and throws on writes outside
   a known writer; production writers call `doc.write` or `doc.touched`. A `touch()`
   without a path triggers a structural diff of the open composition.
2. **Flush before query.** Every `pm-wasm` query drains dirty paths first.
3. **Object-identity registry.** Live objects passed to queries (channels, layers,
   keys) map to canonical paths. Detached objects (for example
   `api.model.mkLayer(type)` passed to `allProps`) are evaluated ad hoc from their
   JSON. Results return live document objects (`pick` returns the live `Layer`);
   `expressionErrors` is filled on the live channel objects.
4. **Edit kernels** in `pm-edit`: pose and aim preservation, model-group bounds,
   align and distribute, anchor moves with font anchors, split text, composition
   bounds, material peer synchronization.
5. **Shadow-replay auditor** (test builds): the patch stream applied to a JS shadow
   must equal `PM.Comps.canonicalize(PM.proj)` under sorted-key serialization at
   frame boundaries.
6. **Dual-answer mode** (Phase 1 tests): every synchronous query answered by the
   legacy code and by `pm-wasm`, compared with the section 25 tolerances.
7. **Production consistency check.** On idle, before save, and before every export
   or agent export, the renderer compares a canonical hash of `PM.proj` with
   `pm-wasm`'s hash for that revision. A mismatch forces a canonical resync before
   the save or export proceeds and records a telemetry event.

### Engine-originated facts

| Fact | Committed as |
|---|---|
| Probe results, detected color and alpha | Not committed; media index |
| Automatic interpretation | Not committed; overrides only |
| Relink to a found file (content hash) | Metadata write, not undoable, in the edit log |
| User relink, Replace File…, Attach Camera Proxy… | Undoable user edit |
| Blender generation or import result | Undoable edit; dropped if the revision changed meanwhile (as today) |
| Blender availability, font substitution, crash-attributed failures | Not committed |

## 7. Evaluation (`pm-eval`, `pm-geom3d`)

A pure function of (document, composition, time, optional view) to evaluated state.

- **Pure and incremental:** no global composition, time, or `PM.scope`; caches keyed
  by node id, input revision, and tick; expression dependencies recorded.
- **Temporal model** ported exactly, including springs, legacy easing presets, and
  `offset_property`.
- **JavaScript semantics** where projects depend on them: loose `==`, string
  coercion, `Math.round` half toward +∞, `%` on negatives, `sin`-based `random` and
  `wiggle`, float32 spring table, hex color interpolation; each tested against V8.
- **Editor helpers.** Every `EDITOR_HELPER_EXPORTS` member (`kernel/editor-helpers.ts`,
  re-exported to extension bundles) keeps its signature as a thin shim over
  `pm-wasm`: `evaluatedValue`, `pathValues`, `groupMatrix`, `tracePath`,
  `temporalKeys`, `enableTimeRemap`, `validMatteSource`, the space-3d math, the
  text-animation helpers (`countTextUnits` with `Intl.Segmenter`-identical
  graphemes), and `expressionDiagnostic` (using `pm-eval`'s parser, so the editor
  and engine accept the same expressions).
- **3D content:** channel namespaces `m.`, `light.`, `camera.`, `environment.`,
  `shader.`, `slots.<id>.`, `model.`, legacy `o.<id>.`; aims in parent space; shared
  materials; topmost active camera and environment; studio defaults (with no light
  layer, a sun at (−3, 5, 4) aimed at the origin, intensity 3, casting shadows; with
  no camera, a camera at (3, 2, 6) with a 45° field of view).
- **3D geometry (`pm-geom3d`):** meshes identical to today's generators; glTF decode
  with skinning, morphs, and animation; BVH refit when time changes.
  - **Picks** hit deformed geometry, respect single-sided materials, skip meshes
    with material opacity 0 and locked layers, and are restricted to the layer's
    depth set. Legacy OBJ mesh layers pick by rectangle (as today).
  - **Bounds** project the eight corners of rest-pose boxes and clamp. Model-group
    bounds today cache skinned boxes on first call; the pure version computes them
    per frame (an allowed difference, listed in appendix A).
  - Ordering and depth sets per section 8.
  - Every query accepts an optional view camera that is not in the document.
- **Determinism:** the `libm` crate for every transcendental function on every
  target.
- **JS fast path:** numeric ids per property; results in a shared `Float64Array`.

## 8. GPU and the render graph (`pm-render`)

### Device

- wgpu (30.x today), Metal on macOS, DX12 on Windows, Vulkan for Linux CLI. Pinned;
  upgrades are lanes.
- One `pm-gpu` interop trait owns decoder import, surface export, encoder input, and
  Blender surface import.
- Adapter matched with Chromium's GPU process (Metal `registryID`, LUID).
- **Metal math mode** for color-critical shaders (precise) and `rgba32float`
  filtering and blending support on every reference machine are verified in S2.
- **Watchdog.** Each effect and shader has a GPU timestamp budget; an overrun
  disables it from the next frame and reports it. A true hang means device loss: the
  device is rebuilt and rendering resumes without replacing the document.
- **GPU time slicing (contract 13).** During playback, main signals the job engine,
  which splits work into command buffers of at most 2 ms, runs at lower QoS, and
  yields when the interactive engine reports a deadline miss. Blender jobs pause or
  throttle during playback. Hardware decode and encode sessions are arbitrated by
  main. Spike S8 proves this.

### Working encoding and precision

| Blending | Intermediate encoding |
|---|---|
| Standard (linear blending off) | Premultiplied, display encoding E (section 9), Rec.709 primaries |
| Linear blending on | Premultiplied, linear light, Rec.709 primaries |
| After cutover: wide gamut, HDR, ACES | Linear in the chosen working space; linear blending forced on |

Storage is `rgba16float` by default, `rgba32float` at 32-bit precision. Source
conversion shaders compute in f32. Exports default to 32-bit when the output or any
source exceeds 10 bits. A preview-versus-export precision difference is stated in
the export sheet.

### Render graph

- **Nodes:** decoded frame, vector, text, solid, shader output, 3D scene pass
  output, Blender surface; transform and resample; mask; effect pass; matte; blend;
  transition; motion-blur accumulate; precomp; adjustment; color transform; output.
- **Content keys** hash every quality-affecting input: evaluated parameters, source
  frame identity (asset, representation, frame), color transforms, resolution
  bucket, resampler, motion-blur samples, preview-quality state, view override,
  Blender settings, precision, and the engine build hash.
- **Cache granularity:** layer and precomp outputs on full extents; region of
  interest on final output only.
- **Continuous rasterization for precomps** on `compat` level 2 precomps; level 1
  precomps render at their own size as today. "Collapse transformations" follows
  cutover.
- **Region of interest** with effect padding; bounds-expanding effects grow up to a
  limit.
- **Isolation:** a failing node marks its layer failed and outputs transparency.

### Sampling and antialiasing

- Level 1 content reproduces today's sampling (no mipmaps, hard UV edges) so existing
  projects look the same. Level 2 content gets mipmapped trilinear during playback,
  bicubic or Lanczos (anti-ringing clamp to the local minimum and maximum, in
  premultiplied space) for export and paused frames, and analytic coverage edges.
- Vectors and text at output resolution for level 2; level 1 matches today within
  the text class.
- MSAA in the 3D pass.

### Motion blur

- Sample counts come from the composition's `motionBlurSamples`, per output kind
  (export, alpha export, preview playing, preview paused), used by every path
  including Blender surface requests and cached preview. Migrated projects keep
  today's counts.
- Accumulation in the active encoding at float precision; no adaptive reduction
  unless output-identical.

### 2.5D

Parity: the 3D switch, Z position, anchor and scale, X and Y rotation, orientation,
perspective per 3D group, homography planes, coplanar ordering, painter sort.

### 3D layers

Today: three.js r180 in a second WebGL context. The engine's **3D scene pass**
reproduces it and writes each layer's premultiplied output directly into the 2D
graph, so masks, effects, opacity, and blend stay per layer.

- **Depth sets.** For each 3D object layer, its depth set walks outward through
  neighboring layers. Inactive or hidden layers of any type, lights, cameras, and
  plain group headers do not end it; a 2D layer or a styled group (a compositing
  boundary) does. Models are excluded when their layer opacity is ≤ 0.001 or when
  another layer is soloed and they are not. Only models in the same set cast shadows
  into it; lights illuminate every set. A model writes depth only with no
  transparent material, layer opacity ≥ 0.999, and no enabled mask.
- **Ordering** (separate from depth sets): within each run of 3D layers, all models
  sort back to front by the camera depth of their object origin; lights and cameras
  sort at depth 0; blocks are never reordered; the editor camera is used in the
  editor view (`compositionOrderedLayers`).
- **Background.** The bottom layer of a set draws the solid background color,
  opaque and not tone-mapped.
- **Shading order.** Exposure, ACES Filmic (with r180's `exposure / 0.6` pre-scale),
  and E are applied in every material's fragment shader. Blending, depth-sorted
  transparency (double-sided transparent materials draw back faces, then front), and
  MSAA resolve happen on E-encoded values, as three.js does when drawing to a canvas.
- **Materials.** `MeshStandardMaterial` with color, normal, roughness, metalness,
  emissive, and AO maps and UV placement. Imported glTF renders exactly as three.js
  r180's loader configures it: every extension it honors (punctual lights, unlit,
  texture transform, clearcoat, sheen, iridescence, transmission with its own pass
  in which only the layer's own objects are visible, volume, IOR, specular,
  anisotropy, dispersion, emissive strength, bump, mesh GPU instancing, and the rest
  of r180's list), vertex colors, alpha cutoff, flat shading when normals are
  missing, second UV sets for AO, and point and line primitives. Embedded punctual
  lights light the scene but count toward neither the 16-light nor the 4-shadow
  limit.
- **Animation.** Skinned and morph animation. A file's clips follow three.js's mixer:
  every clip enabled at weight 1, looping, blended equally.
- **Lights.** Hemisphere ambient (ground color 0.35× the sky color); sun, point, and
  spot; rectangular area lights through the LTC tables, without native shadows.
- **Shadows** reproduce r180 exactly: PCF-soft, 1024² maps, bias −0.0002, normal bias
  0.025; sun frustum orthographic ±10, near 0.01, far 200, from the light position;
  spot field of view from the cone angle, near 0.5, far = distance or 500; point
  lights in a 4:2 cube atlas with 9 taps and distance-based depth; depth packed into
  RGBA8 and compared manually with nearest fetches (no hardware comparison
  samplers); the first 4 shadow lights in layer order cast.
- **Cameras.** Perspective and orthographic (fixed half-height 3 divided by zoom);
  roll from the layer rotation; light rotation and scale ignored.
- **Missing models** draw an orange wireframe box in output, as today.
- **Editor looks** (view override, never in export or agent capture): X-ray multiplies
  opacity by 0.45 and writes no depth, in draft modes only; Wireframe draws 1-pixel
  edges; Local View; editor camera.
- **Legacy kinds.** Legacy `scene3d` layers render at most 4,096 px with their own
  background, then transform in 2D. Legacy OBJ `mesh` layers keep their fixed
  shading. Studio Cube is a GLSL fragment layer (section 11).
- **Blender surfaces.** With `render3d.enabled`, each 3D layer's pixels come from
  Blender as a source node with its crop rectangle (sections 9 and 17). A missing
  surface fails an export.

After cutover: cameras and lights for 2.5D layers; intersections between 2.5D planes
and models; physically based changes behind `compat` level 2.

### Depth and ID surface

With each viewer frame that contains 3D layers, the engine also renders a depth and
object-id surface (`rgbaf16`) at the frame's revision. The 3D viewport chrome uses it
for grid and wire occlusion and for selection outlines, so the chrome never re-renders
the scene or skins meshes on the UI thread.

### Scopes

An internal waveform and vectorscope for the QA lane from Phase 3a; scopes as a
product after cutover, reading a full-precision working render.

## 9. Color pipeline

### The rule

sRGB and Rec.709 share primaries and, by default, one **display encoding E**: the
sRGB piecewise curve, **extended to all reals** (odd-symmetric below 0, the power
segment continued above 1). Code values in E pass through unchanged everywhere:

- **Standard** (linear blending off): intermediates in E. Hex colors, pass-through
  footage, and today's look hold exactly.
- **Linear blending on:** E⁻¹ on input, linear blending, E on output.
- **Interpret** offers other readings of Rec.709: "BT.1886 (gamma 2.4)" and
  "Scene-referred (inverse BT.709 OETF)".
- **Viewer:** working values encoded with E. On a `display-p3` canvas, Rec.709
  primaries are converted to P3 in linear before encoding. ColorSync maps to the
  display.
- **Clipping happens only at output:** Y′CbCr outputs clip to the codable range
  (8-bit 1 to 254, 10-bit 4 to 1019, 12-bit 16 to 4079), full-range RGB outputs to 0
  to 1. Superwhites and sub-blacks survive the pipeline.
- **QuickTime.** Export offers "Rec.709 (broadcast)" (1-1-1) and "Rec.709 (QuickTime
  and Safari)". S2 tests 1-13-1 and 1-2-1 with a gamma 2.2 tag and picks the one that
  passes bar (3). "Rec.709 (broadcast)" looks darker than the viewer on a BT.1886
  monitor, by design.

### Chroma

- Probe reads chroma siting (`chroma_sample_location`, ProRes and DNx defaults, MXF
  descriptors); Interpret can override it. Untagged 4:2:0 is left (type 0).
- Upsampling is siting-aware and keeps co-sited samples exact: horizontal linear for
  4:2:2; for 4:2:0, Catmull-Rom horizontally and linear between rows vertically.
- The encoder-input shader downsamples by point decimation at co-sited positions for
  4:2:2, and with a declared filter for 4:2:0; output siting is written to the VUI or
  container.
- **Pass-through.** When an output frame is a single opaque, untransformed,
  effect-free source whose format, range, and subsampling match the output, the
  engine copies its Y′CbCr planes.

### 3D layers

- Native 3D tone-maps per fragment (section 8) to E, matching today's Native look.
- Blender renders through AgX to piecewise-sRGB-encoded PNG codes, which are E,
  matching today's Blender look. The two curves differ by design; parity keeps both
  (decision 17).
- Color and emissive maps are sRGB; data maps are linear.
- With linear blending on, the resolved, premultiplied, E-encoded 3D output is
  unpremultiplied, decoded with E⁻¹, and re-premultiplied.

### Alpha

- Straight sources: input transform, then premultiply.
- Premultiplied sources: when the input transform is the identity (Standard, matching
  primaries and transfer), used as they are; otherwise unpremultiply (guarded for
  α < 1/4096 at half float), transform, re-premultiply.
- "Premultiplied with color" interpretation.
- Color-domain effects run on unpremultiplied values; additive pixels (α = 0,
  RGB > 0) are never unpremultiplied.
- Output: with an identity output transform, premultiplied working values are written
  directly for premultiplied outputs; otherwise unpremultiply, transform, and write
  straight or re-premultiplied in the encoded domain.
- Blender surfaces are straight alpha and premultiplied on input.

### Blend modes and mattes

The nine modes are normal, add, screen, multiply, overlay, soft light, difference,
lighten, darken. Normal, add, multiply, difference, lighten, and darken run in the
active encoding. With linear blending on, screen, overlay, and soft light use HDR-safe
formulas (screen `a + b − ab` unclamped; overlay and soft light on values clamped to
0..1 for their switch condition only); a per-project option runs them in E instead.
Luma mattes use Rec.709 coefficients on E-encoded values.

### Project color settings

Settings › Project › Color.

| Setting | Options | Default | Phase |
|---|---|---|---|
| Blend in linear light | Switch | Off (decision 1) | 3a |
| Precision | 16-bit float, 32-bit float | 16-bit float | 3a |
| Color | Standard (sRGB and Rec.709), Wide gamut, HDR (PQ), HDR (HLG), ACES | Standard | Other options after cutover |
| Advanced › Working space, Graphics white in HDR | — | — | After cutover |

Changing a color setting is one Undo step. Migrated projects open in Standard with
linear blending off. The display choice is a machine setting.

### Per-source interpretation

"Interpret Footage…" on an asset or selection, as a Settings-style sheet, values
showing their origin. Before cutover (Phase 3b):

- Primaries, transfer (E, BT.1886, scene-referred), matrix, range, chroma siting.
- HDR sources (iPhone HLG, HDR10, Dolby Vision base layer) in Standard projects tone
  mapped to SDR with BT.2446 Method A. Phase 0 measures today's Chromium handling;
  any difference becomes an appendix A item with its own corpus case.
- Alpha modes; frame rate override; field order; pixel aspect and clean aperture
  (`pasp`, `clap`).
- Untagged video: height ≤ 576 uses BT.601 with 525- or 625-line primaries chosen by
  height; HD and above BT.709. MJPEG, `yuvj`, and RGB codecs are full range; other YUV
  is limited.
- Stills: ColorSync is bypassed when the profile is equivalent to sRGB (exact
  pass-through); P3 stills convert in float, relative colorimetric, no clip until
  output.

After cutover: camera log curves with gamuts, input LUTs, camera rules, "Show as HDR
values".

### After cutover: professional color

OCIO (2.6 at time of writing) through a `cxx` bridge with ACES 2.0 configurations;
graphics input as the inverse output transform; wide-gamut and HDR projects with gamut
mapping; LUTs and color effects; gradients in Display, Linear, or OKLab; legalizer,
gamut warning, false color, zebras, scopes; HDR preview with EDR headroom (prior art:
`renderer/public/onboarding/hdr-output.js:93-132`); HDR export with measured MaxCLL
and MaxFALL; HDR stills with gain maps; a reference viewer; unified 3D tone mapping.

### Output tags

| Output | CICP | Range |
|---|---|---|
| sRGB images (PNG `cICP` or ICC) | 1-13-0 | Full |
| Rec.709 (broadcast), video | 1-1-1 | Limited |
| Rec.709 images | 1-1-0 | Full |
| Rec.709 (QuickTime and Safari) | Determined in S2 | Limited |
| After cutover: P3 video and images, Rec.2020 SDR, PQ, HLG | 12-13-1 and 12-13-0, 9-1-9, 9-16-9, 9-18-9 | Limited for video |

ProRes Y′CbCr is limited range with full-range alpha. MOV and MP4 write `colr`
`nclx`; WebM writes Colour elements.

## 10. Layer types

| Type | Today | Engine |
|---|---|---|
| `solid` | Uniform quad; `d.radius` ignored | Same; `radius` honored on `compat` level 2 |
| `shape` (rect, ellipse, polygon, star with 0.46 inner ratio, line, editable paths) | Canvas2D raster at quantized scale | Level 1 matches today; level 2 at output resolution |
| `text` | Canvas2D `fillText` | `pm-text` (section 12) |
| `captions` | Canvas2D (`captions/render.ts`) | `pm-text`, `pm-vector`; word states in `pm-eval` |
| `image` | `ImageBitmap`, RGBA8 | Native-depth readers |
| `image` (SVG) | `<img>` per size bucket | usvg (text feature off) to vectors; SVG text through `pm-text`; filters through resvg |
| `video` | `<video>` or VP9 copy | `pm-media` decode of the chosen representation |
| `shader` | User GLSL, FBO capped at 4,096 px | GLSL pipeline (section 11); level 1 keeps the cap |
| `extension` fragment (including Studio Cube) | Kernel-assembled GLSL | Same pipeline |
| `extension` mesh (legacy OBJ, or a third-party mesh definition with `assetField`) | Fixed WebGL shading; legacy OBJ picks by rectangle | Same look and picking |
| `extension` `layer3d` object, light, camera | three.js; or a Blender PNG | 3D scene pass; Blender surface source |
| `extension` `scene3d` (legacy, read-only) | three.js, 4,096 px with own background | Same |
| `precomp` | Recursive render at its own size | Level 1 same; level 2 at effective scale |
| `adjustment` | Effects on the accumulator | Same |
| `group` | Transform space or compositing boundary | Same |
| `group` with `d.modeling` | 3D model group | `pm-eval` and `pm-geom3d` |
| `null`, `audio` | Not visual | Not visual |

## 11. Effects, transitions, and the shader contract

### No shader is dropped

- **Gate:** every GLSL source in the repository, the Store, the docs samples, and a
  generated corpus of agent-authored shaders (prompts covering every contract
  feature, written by the same agent harness users use) compiles and matches its WebGL
  output within the parity tolerances before cutover.
- **Runtime fallback:** a shader that fails translation on a user's machine renders
  through a minimal GLSL runner in the renderer (one WebGL2 context, the contract
  below, no compositor), uploaded to the engine like the `raster` override; it is
  reported and marked not headless-renderable (decision 19).
- A failure is fixed in the pipeline, never by changing the user's shader.

### Built-in effects keep today's look

The 16 effects, 4 transitions, and Studio Cube keep their GLSL source, which runs
through the compatibility pipeline and is the reference. An engine-side WGSL path may
accelerate a built-in only when it is output-identical to the GLSL on `compat` level 1
effect instances. Visual fixes (a correct Gaussian at every radius, Glow and Drop
Shadow on it, analytic edges) apply only to `compat` level 2 effect instances. The five
shader-layer presets are user GLSL copied into `d.code` and stay GLSL.

Forks of built-ins (`effects-basic`, `layers-3d`, updated by local merge in
`main/extensions/update-fork.ts`) register their own GLSL under the built-in ids; the
engine uses the fork's definition snapshot. The Phase 0 audit covers local forks.

### The GLSL contract (normative)

| Kind | Inputs and semantics |
|---|---|
| Effects | Varyings `v_uv`, `v_st`, `v_px`; uniforms `u_tex`, `u_res`, `u_texel`, `u_time`, `u_prog`, `u_coordRes`, `u_pxOrigin`, `u_pass`; `u_orig` when `keepOrig`; `u_backdrop` when `backdrop` (the accumulated lower stack, on every pass); `u_<param>` per parameter and positional aliases `u_p<i>`, `u_c<i>`; 1 to 8 ping-pong passes; `rawShader` passthrough; composition-sized buffers; output `o`; helpers `src`, `luma`, `hue2rgb`, `rgb2hsv`, `hsv2rgb`, `hash`, `noise`, `fbm`. Toggles are `float`. `viewportSafe: false` effects always see full-composition coordinates; `viewportPadding` as today |
| Transitions | `u_from`, `u_to`, `u_prog`, `u_res`, `u_texel`, `u_time` |
| Shader layers and extension fragment layers | `iResolution`, `iTime` (layer-local), `iGlobalTime`, `iProgress`, `iFrame` (root fps), `iMouse` = (0, 0), `uv`, `fragColor`, and the header helpers (SDF, noise, palette); extension-layer toggles are `uniform bool`; `@param` annotations |
| Enables | Effect and mask enables are animatable (`<fx>.$enabled`) |

Shader-layer uniform types come from `@param` annotations stored in
`UIState.shaderMeta.udefs`; numeric controls send one float, so `vec2`, `vec4`, and
`int` uniforms stay at 0 (`compositor.ts:1024-1030`). The engine ports
`parseUniforms` exactly and reproduces this binding.

### The pipeline

1. **Wrap** with the existing generators (`kernel/glsl.ts`, `extension-layers.ts`).
2. **Normalize:** `#version 300 es` → `#version 450`; remove precision qualifiers;
   inject `pm_FragCoord` for `gl_FragCoord`.
3. **Compile** with glslang (relaxed Vulkan rules, automatic bindings and locations).
4. **Split combined samplers** at the SPIR-V level.
5. **Translate** with naga, derivative-uniformity checks off (`ShaderSource::Naga`); S3
   compares against SPIRV-Cross to MSL through passthrough shaders.
6. **Cache** by source hash, backend, driver.
7. **Errors** map to the user's line numbers.

**Origin and sampling:** GL conventions (bottom-left origin, `dFdy` as in WebGL);
sampler state per effect type reproduced. **Color domain:** legacy GLSL receives
E-encoded values independent of the display; values above 1.0 pass through unless an
effect declares `clamp: true`. **Store GLSL is untrusted input:** the pipeline is
fuzzed and resource-limited, and compile failures stay per definition.

### Definition snapshots

- The kernel pushes a snapshot per registered definition to every session:
  `{id, version, renderer (verbatim, including role and assetField), glsl?, params,
  flags}`. UI-only fields (`ui`, `advanced`, `options`, widget hints) are stripped.
- Snapshots follow the winning registration per id (last registered wins; disabling
  restores the previous one) and are re-pushed on every register, dispose, and hot
  reload.
- 3D roles and `scene3d` behavior keep today's id-based rules (`powermove.3d.{object,
  light,camera}`, `powermove.3d.scene`). When a fork registers a built-in id with a
  different renderer kind, the snapshot's kind wins and the layer leaves the 3D scene.
  The Phase 0 audit lists third-party `layer3d`, `scene3d`, and `mesh` definitions.
- **Missing definitions** behave as today: a missing effect is skipped with its data
  kept; a missing extension layer shows the checkerboard placeholder.
- Export jobs and Collect Project embed the snapshots they use. `pm-cli` fails only
  when a definition has no snapshot and is not already marked missing.
- `uiState.getShaderMeta` (`shaderKey`, `missing`, `missingAsset`, `sceneWarnings`,
  `sceneError`) is filled from engine render events at the frame's revision.
- The `raster` override keeps working fully (section 20).

### Contract v2

A WGSL contract for new extensions at apiVersion 4. GLSL stays supported indefinitely.

## 12. Text

- **Stack:** Parley (layout), HarfRust (shaping), Skrifa (outlines and metrics); glyph
  rendering chosen in S4.
- **Font registry:** one native authority for system fonts, project fonts, and
  fallback. Before Phase 2 it runs in main through `pm-bridge`; from Phase 2 in the
  engine, which also produces subsets. Main serves subsets to `pm-wasm` in both cases.
- **Fonts in `pm-wasm`:** metrics-and-shaping subsets per font (`cmap`, `hmtx`,
  `hhea`, `OS/2`, `head`, `name`, `GSUB`, `GPOS`, `GDEF`, `kern`, `morx`, `kerx`,
  `trak`, `fvar`, `avar`, `HVAR`, `MVAR`, synthesized `HVAR` where missing). On project
  open, the document's fonts are prefetched before the mirror reports ready; a query
  for a font whose subset is still loading answers from the registry's metrics
  fallback and emits a re-layout event when the subset arrives.
- **Compatibility (level 1):** today's wrapping, UTF-16 splits, baseline at
  `size × 0.82`, line height `size × leading` (1.15), measured-width kerning, CoreText
  `trak` and optical sizing, hidden system fonts. Gate: identical line breaks and
  advances within 0.25 px. Level 2 uses grapheme-aware breaking and font metrics.
- **Animators, editing, split text** through `pm-wasm`; native and WASM layout
  identical.
- **Rendering:** coverage gamma or stem darkening tuned to match CoreText.
- **3D Text** from Blender's font, as today.
- **Gates:** CoreText side-by-side corpus reviewed by Jude; SSIM ≥ 0.98; typing latency.

## 13. Media I/O (`pm-media`)

### Principles

The original is the truth; every source is indexed; frames reach the GPU at native
depth with color and chroma metadata.

### Backends

- **VideoToolbox:** ProRes 422 → `x422`; ProRes 4444 and XQ → `y416`; 10-bit HEVC and
  H.264 → `x420` (4:2:0) or `x422` (4:2:2); 8-bit → biplanar 8-bit. The requested
  format's range matches the bitstream (`420v` or `420f`, `x420` or `xf20`), verified
  against buffer attachments.
- **Normalization:**
  - `x420`, `x422`, P010 (MSB-aligned): c = v / 64; Y′ = (c − 64) / 876,
    C = (c − 512) / 896.
  - `y416`: Y′ = (v − 4096) / 56064, C = (v − 32768) / 57344, A = v / 65535; imported
    as one `RGBA16Unorm` texture with an AYCbCr swizzle.
  - FFmpeg LSB-aligned planar n-bit (H.264 4:2:2 10-bit, DNxHR HQX and 444, VP9
    profile 2, DPX): c = v with constants 16·2ⁿ⁻⁸ and 219·2ⁿ⁻⁸ (luma), 128·2ⁿ⁻⁸ and
    224·2ⁿ⁻⁸ (chroma).
  - Full-range formats use full-range scaling.
- **Metal import:** biplanar buffers as per-plane `R8`/`RG8` or `R16Unorm`/`RG16Unorm`
  textures via `CVMetalTextureCache`.
- **FFmpeg** (our build, section 24) through `ffmpeg-the-third` or our own bindgen
  crate, as dylibs, for demuxing, DNxHD/DNxHR, CineForm, MJPEG, DV, VP8 and VP9
  (libvpx for alpha), AV1 (dav1d), DPX, software fallbacks, and all audio decoding.
- **Images:** ImageIO on macOS; the `image` crate; OpenEXR's C library; usvg.
- **3D models:** the `gltf` crate plus raw-JSON parsing for extensions it lacks; WebP
  and AVIF images inside GLB; OBJ assets for `mesh` layers stay OBJ; `.blend` only
  through Blender.

### Formats at cutover

| Category | Formats |
|---|---|
| Intermediates | ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ; DNxHD; DNxHR; CineForm |
| Camera and delivery | H.264 (including 4:2:2 10-bit), HEVC 8 and 10-bit, HEVC with alpha, AV1, MJPEG, DV |
| Web | VP8, VP9 with alpha |
| Containers | MOV, MP4, M4V, MXF, MKV, WebM, AVI, WMV/ASF, FLV, MPEG-PS, M2V, TS, MTS/M2TS, 3GP, 3G2, OGV, VOB, DV |
| Images | PNG 8/16, JPEG, TIFF 8/16/32f, OpenEXR, DPX, HEIF/HEIC, AVIF, WebP, BMP, ICO, GIF, SVG |
| Sequences, animated images | Any image format with gaps reported; GIF, APNG, animated WebP and AVIF |
| Audio | WAV, AIFF, CAF, MP3, AAC/M4A, FLAC, Opus, Vorbis, WebA, audio in every container |
| 3D models | GLB and self-contained glTF 2.0 (no Draco or meshopt), OBJ with MTL and textures (packed to GLB by `prepareImport`), `.blend` (Blender 4.5+) |

Hardware notes: base M1 has no ProRes engine; AV1 hardware decode starts at M3; H.264
4:2:2 10-bit is software-only.

### Probe and index

1. Streams, timecode, reel, UMID or content identifier, pixel aspect, clean aperture.
2. Color: primaries, transfer, matrix, range, chroma siting, mastering metadata, ICC,
   HDR format.
3. **Frame index** from container tables (MOV/MP4 `stss`/`stts`, MXF index, MKV
   cues); scanning only where none exist. Open-GOP CRA and RASL marked.
4. **Timing:** edit lists, composition offsets, negative CTS, stream start offsets;
   **priming** from `elst`, `iTunSMPB`, or the LAME/Xing header (+529 decoder delay),
   assuming 2112 only for Apple-encoded files without metadata. Replaces
   `shared/mp4-frame-rate.ts`.
5. **Nominal rate:** the supported rate whose frame duration is within ±1 timebase unit
   of the median duration over the first 300 frames, falling back to
   (last PTS − first PTS) / (frames − 1).
6. **Content hash:** full BLAKE3 in the background; the sampled fingerprint stays a
   lookup hint; files still being written wait.
7. Decode cost; poster, filmstrip seed, waveform peaks.

### Variable frame rate and interlacing

- VFR sampled by timestamp (section 5 rule 7); nominal rate for snapping and
  stepping; generated media preserves PTS exactly.
- Interlaced: field order from `fiel`, MXF, or the bitstream, with an override and a
  progressive (PsF) override. Deinterlace (bwdif) to field rate when the composition
  rate is at least the field rate, otherwise to frame rate, in Y′CbCr before chroma
  upsampling with interlaced 4:2:0 siting; random access includes neighboring fields.
  Pulldown removal after cutover.

### Decode service

Sessions per source and playback instance, pooled; hardware output imported without
copies; scheduling by direction, speed, read-ahead, and GOP; hardware session limits
arbitrated by main.

## 14. Proxies, optimized media, and caches

### 14.1 What users should feel

Footage always plays without setup. Professionals find full proxy control in plain
language. The viewer always says what is on screen. Exports never contain proxy pixels
by accident. Controls and copy are in appendix D.

### 14.2 Scope at cutover (Phase 4a) and after (Phase 4b)

| Phase 4a (before cutover) | Phase 4b (first release after cutover) |
|---|---|
| Automatic proxies for sources that cannot play in real time on this Mac; whole-clip generation; the representation ladder with reduced-resolution decode; the appendix D controls marked 4a; the viewer Media menu; Always Play Original; export safety; offline handling; disk guard; cached preview through an engine memory cache | Content-addressed segments shared across projects; used-range generation; optimized media; the disk render cache with its timeline track; Keep Rendered; background rendering; Storage settings; Collect Project; the Relink sheet; `manage_proxies` |

### 14.3 The representation ladder

From no generation to most generation: original; reduced-resolution decode with no
files (ProRes partial-resolution decode, ImageIO downsampling, EXR mips); proxies;
optimized media (4b); kept renders (4b). Image sequences get proxies; large stills get
a tiled mip cache.

### 14.4 Recipes and fidelity

| Purpose | macOS recipe |
|---|---|
| Proxy | ProRes 422 Proxy (10-bit) by default; ProRes 422 LT or HEVC 10-bit by choice. Sources with alpha always get ProRes 4444 at proxy size |
| Optimized (4b) | ProRes 422 HQ; ProRes 4444 for 4:4:4, above 10-bit, or alpha |

Generated media keeps the source encoding, 10-bit or the source depth up to 12, alpha,
PCM audio, timecode, reel, pixel aspect, frame count, and exact PTS, in the original's
coordinate space. Analysis always uses originals.

### 14.5 The automatic policy

A **machine profile** measures decode cost, GPU headroom, disk throughput, and power
state; the project stores intent.

1. Best representation that sustains real time at the viewer's device-pixel size.
2. Switch only at clip boundaries or when playback starts.
3. Paused with Resolution on Automatic: the original as soon as it decodes, in place;
   a hand-set Resolution is respected.
4. Never wait for generated media.
5. Re-evaluate on power-state changes.

Generation runs in the job engine at low priority, pauses during playback and export,
and goes clip-near-playhead first. The disk guard pauses generation below the larger
of 20 GB or 10% free.

### 14.6 Export safety

Export reads only cache entries whose keys match its own quality inputs (original
representation, export resampler and motion-blur samples, export precision, full
quality, Blender final settings, same engine build). Disk cache entries (4b) are stored
at their key's precision. Gate: fill the cache at Automatic, Half, Quarter, with
proxies and Blender preview settings, during a sustained-load drop; export;
byte-identical to an empty-cache export.

### 14.7 Caches

| Cache | Contents | Key | Location |
|---|---|---|---|
| Frame cache | Decoded frames | Source, representation, frame | Memory (lease) |
| Render cache | Node outputs | Section 8 key | GPU memory (4a), disk (4b) |
| Blender surfaces | Per-layer surfaces | Snapshot hash, worker-script hash, asset hashes, Blender version and settings, resolved device, preview flag, camera, size | GPU memory, disk |
| 3D geometry | glTF, meshes, BVHs | Asset hash or source signature | Memory (lease) |
| Media index | Indexes, probe, hashes, decode cost | Content hash and file identity | App cache |
| Thumbnails, filmstrips, posters, waveforms | Derived images, peaks | Content hash, recipe | App cache |
| Generated media | Proxies (4a whole clips; 4b segments) | Content hash, recipe | Location setting |
| Large-still mips, shader cache, font subsets | — | Content or source hash | App cache |

These replace today's IndexedDB media store, editing copies, decoded audio cache,
raster and texture caches, GL program cache, the renderer's Blender frame cache, three.js
scene runtimes, and the recipe-preview cache. `memory.ts` budgets and
`previewMemoryMiB` map to engine leases.

### 14.8 Offline media and exports

A native three-button message for interactive exports; queued jobs wait; draft renders
from proxies are explicit; compositions needing Blender name the requirement.

## 15. Playback

- **Master clock:** the audio device clock, including device latency.
- **Render-ahead** into an adaptive ring.
- **Presentation:** frames carry PTS and revision. The viewer presents the frame whose
  PTS interval contains the predicted display time (not the rAF callback time). On
  ProMotion displays it requests a refresh rate that is a multiple of the content
  rate. Scrub and gesture frames bypass the jitter buffer.
- **A/V sync** ±10 ms, measured by photodiode loopback in the lab and frame-callback
  timing in the app.
- **Drops** counted, attributed, reported.
- **Quality ladder** under sustained load.
- **Priority:** agent snapshots and MCP renders never interrupt playback (contract 13).
- **3D:** Blender shading is paused-only, as today; playback shows native 3D; editor
  view frames use the transient camera.
- **Transport:** work-area loop, frame stepping; J/K/L at 1× and 2× forward and
  reverse, play around current frame, audio scrub.
- **Cached preview** through Render Work Area into the engine memory cache (4a).

## 16. Audio

- f32 mixing at `audioSampleRate`; `rubato` sinc resampling (≥ 120 dB stopband,
  passband to 0.45·fs); `cpal` output.
- FFmpeg decode, streaming, no length cap, priming honored.
- Clip voices → per-layer strip → precomp nesting → master; channel layouts preserved
  and down-mixed.
- Per-sample automation.
- Speed and remaps followed; pitch preserved by default (Signalsmith Stretch, our own
  binding) or following speed per clip (`pitch`); deterministic pre-roll.
- Frame-rate override keeps audio in sync.
- Engine waveforms, meters, solo, retune.
- Export: PCM 24-bit or 32-bit float for ProRes and WAV; AAC through AudioToolbox with
  `edts` and priming; TPDF dither when reducing to 16-bit; stereo as today.
- After cutover: reverse playback, channel mapping, pan, EQ, dynamics, loudness.

## 17. Export and delivery

### Pipeline

```
evaluate → [Blender stage] → render (float) → output transform
  → encoder input (engine shader: matrix, siting-aware chroma, dither)
  → encode → mux → verify → move into place
```

- Runs in the job engine from a snapshot with definition snapshots; users keep
  editing; edits never invalidate an export's prepared surfaces.
- Pipelined stages with bounded queues.
- **Blender stage.** The job engine builds Blender snapshots from `pm-eval` and
  `pm-geom3d` (verified equal to today's snapshots, section 25) and requests surfaces
  per frame, shutter sample, and nested composition, ahead of compositing. A frame's
  layer renders may be batched in one worker invocation only when S7 shows equal
  surfaces; several workers only within the memory lease. Worker settings are pinned:
  16-bit RGBA PNG, `view_transform = 'AgX'`, `dither_intensity = 0`, `look = 'None'`,
  sRGB display device,
  gamma 1, scene-following image color management, fast compression; the engine
  ignores PNG `gAMA`, `sRGB`, and `iCCP` chunks. The 512 MB output cap is raised or
  surfaces stream per layer, because one 8192² 16-bit layer is 512 MiB. Cancel stops
  Blender. A missing surface fails the job. Cycles `device: auto` is resolved and
  recorded per job.
- **Encoder input:** the engine converts to Y′CbCr itself and hands VideoToolbox
  native buffers with color attachments, or copies planes on pass-through.
- **Dither:** deterministic blue-noise RPDF of ±0.5 LSB seeded by frame and pixel,
  after the matrix in the output domain (values already on the output grid are
  unchanged); on by default for 8-bit outputs, opt-in for 10-bit, never on alpha.
- **Muxers:** AVAssetWriter for MOV and MP4; FFmpeg for WebM. Timescales: NTSC rates
  use the rate numerator (24000, 30000, 48000, 60000, 120000, 240000); integer rate f
  uses f × 1000; audio tracks use the sample rate; `movieTimeScale` matches; a `tmcd`
  track with the drop-frame flag.
- **Atomic output** through a temporary file in the destination folder.
- **Verification** probes codec, profile, pixel format, depth, tags, range, frame
  count, duration, audio samples, alpha, and sampled pixel values.
- **Comfort:** prevent sleep, completion notification, size estimate, overwrite policy,
  file name tokens, start timecode and reel name.

### Formats at cutover

Today's list, from originals, at correct depth and tags, with today's settings
preserved (quality tiers Draft, High, Max at 4, 16, and 40 Mbps; scale 0.01 to 16 and
custom size; today's export frame-rate list; range; alpha where supported):

| Format | Encoder |
|---|---|
| ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ (alpha straight or premultiplied) | VideoToolbox |
| H.264 MP4 | VideoToolbox; a VMAF comparison against x264 decides whether a software encoder is also offered |
| HEVC MP4 and MOV (8 or 10-bit SDR; alpha at 8-bit, confirmed in S2) | VideoToolbox |
| WebM VP9 and VP8, frame-exact (today's "WebM · realtime capture" migrates to it) | libvpx |
| PNG sequence and still, 8 or 16-bit | Engine |
| Code export, project export, captions sidecars and burn-in | Sections 21 and 22 |

After cutover: DNxHR/DNxHD, AV1, EXR, DPX, TIFF sequences, GIF, animated WebP,
audio-only, HDR, ACES, interlaced output, non-square pixels, FCPXML and OTIO.

### Agent delivery

- `export_video` and `save_project` keep their schemas and contract: unique names chosen
  by the host, no dialog, owner-only files created with `open(wx, 0600)`, empty files
  removed, the real path returned after encoding and verification. Today only the
  desktop app meets this; serve and the hidden host name files through the save-dialog
  stub without exclusivity (appendix A). Through host services, every host meets it.
- **Queueing.** Agent exports run in their own lane ahead of user batch jobs. Timeouts
  count from job start. If an export waits longer than a configured queue bound, the
  call returns `{status: "queued", jobId}` and the agent reads `get_render_status`.
  `export_video` accepts `mode: "wait" | "async"` (default `wait`), like
  `delegate_task`. Cancelling or rolling back the agent run cancels its jobs; agent jobs
  do not resume after relaunch. The output scope is per job, not per window. Tool
  timeouts (`LONG_CALLS`, `toolTimeoutSec`), agent instructions, and `MCP.md` are
  updated to match.
- `save_project` is the project packer, run by host services against the media store,
  not a render job; it embeds linked originals (section 21).

### Presets and the render queue

- Presets: "Master · ProRes 422 HQ", "Master with alpha · ProRes 4444", "Web · H.264",
  "Web · HEVC", "Image sequence · PNG"; existing presets and per-project defaults
  migrate.
- **Render Queue panel:** jobs from the export sheet and agents, each with its
  destination; unattended runs in the job engine from snapshots without touching the
  live project; per-job failures; persistence across relaunch for user jobs.
- Export sheet and queue reuse the Settings sheet cards; the progress sheet keeps its
  live preview with streamed frames.

## 18. Viewer integration and the frame bridge

### The bridge

Proven by spike S1 before viewer work.

1. **Rendezvous.** `pm-bridge` registers a Mach receive right under a per-launch random
   bootstrap name passed to the engine in its environment. Main verifies the full audit
   token (PID and pidversion) and the sender's code signature against our Team ID,
   accepts exactly one check-in per engine launch, deregisters the name, and continues
   on an anonymous port; a restarted engine gets a new name. Windows uses
   `DuplicateHandle`.
2. **Pool, imported once:** IOSurface-backed textures at viewer device pixels; size =
   maximum ring + jitter-buffer depth + 2; formats `bgra` (8-bit with blue-noise dither
   in the display transform) or `rgbaf16`, chosen in S1 by banding and memory. Each
   surface is sent once and imported once with `importSharedTexture`.
3. **Per frame:** `{poolIndex, seq, pts, revision, colorSpace, overlayGeometry}` after
   the command buffer completes; `sendSharedTexture` to the window hosting the viewer
   (following popouts and reloads; `sendSharedTexture` has a 1000 ms timeout and needs
   a live receiver).
4. **Release per frame:** `VideoFrame.close()` and the imported texture's per-use
   release callback forward `{poolIndex, seq}`; `allReferencesReleased` only for
   teardown. S1 confirms the exact API.
5. **Renderer:** `setSharedTextureReceiver` in the preload's isolated world; S1 proves
   the `VideoFrame` reaches the main world.
6. **Overlays:** the DOM and Canvas2D overlay stays; `overlayGeometry` carries world
   matrices, bounds, paths, and the caret at the frame's revision, so handles never lead
   the pixels during gestures. The 3D viewport chrome draws with three.js against the
   engine's depth and ID surface (section 8). Hit-testing uses `pm-wasm`.
7. **`gl.init(canvas)`** from a forked viewer binds engine presentation to the passed
   canvas. `frame:rendered.viewport` stays identity-comparable with
   `render.gl.previewViewport`.
8. **Capture:** frames are inside Chromium's compositor, so `capturePage` works.

### View override

The viewer sends size, DPR, zoom, pan, resolution, and a view override (editor camera,
shading, X-ray, Local View). Ignored for export and agent capture.

### Measuring latency

**Pointer-to-pixel** is the time from an input event's timestamp to the presentation
time (from `requestVideoFrameCallback` or the display link) of the first frame whose
revision includes that input. Every latency gate uses this definition.

### S1 gate

At 4K 60 fps on the reference machines, with main under load (autosave of the largest
corpus project, agent tool traffic, a 50 ms long task every second, window resize):

- p99 presentation jitter ≤ 4 ms, zero drops over 60 s;
- layer drag, 3D orbit, and walk pointer-to-pixel ≤ 33 ms at 60 Hz, at most 1 frame
  worse than today;
- a ramp read back through `capturePage` matches; a 10-bit ramp shows no banding;
  values above 1.0 survive on an HDR display.

### Fallbacks and kill criteria

If S1 misses its gate after the pipeline is tuned:

- **C′.** Remote layer (`CAContext` and `CALayerHost`) from the utility process,
  evaluated for private-API risk.
- **B.** Shared-memory CPU frames at reduced preview resolution (also the viewer path
  in the hidden host's on-demand Chromium).
- **A′** (engine in main presenting through `CAMetalLayer`) is a last resort, because it
  puts decoders in the browser process; adopting it requires Jude's decision.

## 19. Process model, protocol, security, packaging

### Processes

| Process | Runs |
|---|---|
| Main (host services) | Windows, files, save, agent bridge, external MCP server, media store, capability handles, output destinations, memory and GPU arbiter, queue persistence, Blender supervision, definition snapshots, `pm-bridge` |
| Renderer, per window | UI, kernel, extensions, edit pipeline, `pm-wasm`, 3D viewport chrome |
| Interactive engine (utilityProcess) | GPU device, decoders, audio device, caches, sessions |
| Job engine (utilityProcess) | Exports, agent snapshots and exports, generated media, background renders |
| Blender worker (optional) | Generation, `.blend` import, renders; spawned by main |
| External MCP server | In main on the desktop; in the CLI process on the hidden host |
| MCP stdio relays | `Powermove --powermove-mcp`, `powermove mcp --connect`; unchanged |
| Hidden MCP host (`powermove mcp`) | Today: serve host plus Playwright Chromium. After Phase 3c on macOS: host services in the CLI process, one headless document host worker per open project, a `pm-node` engine child process, Chromium on demand (section 22). Linux keeps today's host until after cutover |
| Extension sandboxes, transcription, agent CLIs | Unchanged |

Main creates a `MessageChannelMain` per window and engine; engine ports are never given
to extension sandbox frames.

### Memory and GPU

Main grants each engine leases from one global budget, shrinking the job engine's
lease during playback, and relays deadline signals for GPU time slicing (contract 13).
Each engine enforces its lease across frame cache, render cache, Blender surfaces, 3D
geometry, surface pool, and decoders. The job engine's duplicated decoders, glTF data,
and pipelines count against the 8 GB gate. `pm-wasm` has a per-window budget including
3D meshes and BVHs. Blender's memory is outside leases; workers are bounded by measured
RSS.

### Crash handling

Exponential-backoff restarts; renderers re-send snapshots; "Reconnecting…"; crash
attribution marks an asset or effect failed after two crashes; user queue, generation,
and Blender jobs persist and resume. `pm-bridge` runs in main, so its code is kept small
and fuzzed (section 28).

### Protocol

`pm-protocol` messages, MessagePack, `ts-rs` types checked in CI, version negotiation;
bulk data through shared surfaces or cache files.

### Security

- **Capability handles** are unguessable, bound by the host to one session and port, and
  rejected elsewhere. The engine accepts no raw paths.
- **Engine processes provide crash isolation and least authority, not a security
  sandbox.** A macOS sandbox profile for engine processes is evaluated in Phase 0.
- **Agent paths** (`import_media`, `replace_media`, `import_media_to_timeline`) are
  validated by the host: a regular file whose type the importer's probe accepts (no
  separate extension list); relative paths keep resolving against the host's working
  directory where accepted today. Agents are not restricted in network or reads.
- **Output handles** come from the host's automatic destination for MCP tools.
- **Blender:** a scrubbed environment (it inherits main's full environment today),
  scratch `BLENDER_USER_*` directories, `--offline-mode`, `--factory-startup`,
  `--disable-autoexec` (the Powermove worker still runs through `--python`), external
  texture paths packed or reported at import, and a network-denying sandbox profile
  evaluated in Phase 0 (`sandbox-exec` is deprecated, a feasibility risk).
- **Untrusted input:** Store GLSL, `.blend`, glTF, demuxers, readers, the protocol
  decoder, patch ingest, and project parsing are fuzzed.

### Packaging and CI

- Pinned Rust toolchain; `aarch64-apple-darwin` first.
- **Reference machines (decision 20):** no self-hosted runners on the public
  repository. Hosted runners run unit, lint, and build jobs. Parity and performance
  gates run locally on the reference machines with a pinned Blender LTS; the private
  corpus never leaves them; each PR carries the gate report. If self-hosted runners are
  added later, they run only on trusted events, as ephemeral VMs, or from a private
  mirror.
- `pm-node` and `pm-bridge` are napi-rs addons in `asarUnpack`, signed with the Team
  ID and hardened runtime; `apps/desktop/scripts/release-mac.mjs` verification is
  generalized.
- **`packages/cli`:** `pm-node` prebuilds for darwin-arm64 and darwin-x64 from Phase 3c;
  Linux after cutover. Playwright stays as an optional dependency for panel tools;
  Chromium is installed with `powermove mcp-install-browser`.
- **FFmpeg** (section 24) ships as dylibs for the engine and as an `ffmpeg` executable
  from the same configuration for transcription, agent media tools, and the agent CLI
  environment.
- `pm-wasm` via the wasm-bindgen CLI; the editor CSP gains `'wasm-unsafe-eval'`
  (`main/security-policy.ts:9`, also served by `server/index.ts:159`).
- Minimum macOS follows Electron 44's.
- **Crash and session reporting:** opt-in (decision 10).

## 20. Kernel API, extensions, agents, and MCP

### Compatibility rule

Extensions at apiVersion 1 to 3 get today's behavior, including synchronous return
types. apiVersion 4 adds async siblings and the WGSL contract; it does not remove
anything. The api-pack and `apps/desktop/docs/EXTENSIONS.md` state behavior per
apiVersion.

**Synchronous render compatibility path.** `render.renderFrameTo()`,
`render.snapshot()`, and `render.raster()` pixels stay synchronous for apiVersion 1 to
3: the renderer calls `ipcRenderer.sendSync` to main, which waits on the engine and
returns pixels through shared memory. `render.snapshot` returns `''` only on real
failure. A deprecation notice points to the async siblings. Built-ins move to the async
APIs so the UI never blocks.

### API mapping

| API | After cutover |
|---|---|
| `project.*` | Unchanged (seconds); `snapshot` from the engine |
| `anim.ev`, `evP`, `active`, `worldMatrix`, `localMatrix`, `resolveContent`, `findProp`, `allProps`, `hasKeyAt`, `wouldCycle`, `transformParentMatrix`, `setKey`, `setKeyOn`, `removeKey`, `applyEaseTo`, `mul`, `version`, `touch`, `expressionErrors` | `pm-wasm`, synchronous, same signatures and live-object semantics (section 6) |
| `model.*`, `groups.bounds`, `space3d.*`, `util.*` | Unchanged signatures; geometry from `pm-wasm` |
| `transport.*` | Engine transport and stats |
| `media.importFiles`, `registerImportDefaults`, `commandForAsset`, `timing.*`, `fonts`, `audio.drawWaveform` | Kept; engine data |
| `assets.pick`, `import`, `importUrl` (decode check by the engine probe), `get`, `readText`, `kind` | Kept. `get` live fields: `url` and `el` point at a browser-playable rendition (the original when Chromium can play it, otherwise a generated web rendition); `audioBlob`, `peaks`, `audioDecodeError` from engine data; `object3d` built in the renderer with three.js from `pm-wasm` geometry |
| `render.gl.bounds`, `pick`, `previewViewport` | `pm-wasm`; 3D layers use mesh raycasts with the editor camera and Local View |
| `render.gl.init`, `resize` | Bind engine presentation to the passed canvas |
| `render.gl.compileError` | Engine compiler, mirrored |
| `render.gl.context` | The compositor's context no longer exists. Phase 0 audits the Store and known local extensions. Extensions that use it get a **compatibility context**: a WebGL2 context on an offscreen canvas whose result is composited as a layer. The two built-in call sites are rewritten |
| `render.raster()` | Metrics (`fontOffset`, bounds, `selection`) from `pm-wasm`; pixels through the sync path |
| `render.renderFrameTo()`, `render.snapshot()` | Sync path above; async siblings at apiVersion 4 |
| `render.textLayout` | `pm-wasm` |
| `services` `raster` override | Fully working: the base raster service returns engine-rendered layer pixels, wrappers modify them, the result is uploaded to the engine for preview and export. Headless: the hidden host runs such projects through its on-demand Chromium; `pm-cli` fails with a named reason |
| `services` `viewer`, `shaderHooks`, `compositions`, `captions` | Kept |
| `uiState.getShaderMeta` | From engine render events |
| `scene3d.*` (every member, including `edit`, `describe`, `model`, `modelRecipes`, `modelFields`, materials and presets, `renderFrame`, preview and rendering control, `blenderStatus`, `chooseBlender`, auto-key, `viewport.*` and controller methods, `prepareImport`) | Same signatures; evaluation and geometry through `pm-wasm`; Blender jobs through host services; `prepareImport` through the engine |
| `effects`, `transitions`, `layers.register` | GLSL contract unchanged (section 11); definition snapshots |
| `EDITOR_HELPER_EXPORTS` | Shims over `pm-wasm` (section 7) |
| `controls.*`, panels, docks, keybindings | UI only |
| `events` (all 16) | Same names; frame events on presentation; `frame:rendered.viewport` identity kept |
| `PanelDefinition.library.render` | Engine snapshot for the viewer card |

`EXTENSION_API_VERSION` moves to 4, a coordinated registry release
(`packages/registry/src/manifest.ts`). `api.scene3d` was added without a bump;
apiVersion 4 documents it. `publish_extensions` warns on deprecated APIs.

### How agents write extensions

Unchanged: TypeScript and Svelte against the `powermove` API, GLSL for effects,
transitions, and shader layers, the same tools, Store, and sandbox. `validate_effect`
gains a real compile through the engine's pipeline. WGSL is optional at apiVersion 4.

### Agents and MCP

All tools keep their schemas; changes are additive and optional (`export_video` `mode`,
the queued result).

- **Frames.** `render_frames`, composition contact sheets, and `observeVisual` become
  batched engine requests at agent priority, carrying the agent's revision
  (read-your-writes). They queue instead of failing and never pause playback. Defaults
  stay (`render_frames` 720 px; `observeVisual` 480 px; JPEG quality 0.74; motion blur
  off). Blender compositions use cached surfaces or native shading with
  `blender: "pending"` unless the agent requests Blender. Bound: `render_frames` with 5
  frames at 720 px, p95 ≤ 10 s during 4K playback.
- **Export and save:** section 17; `get_render_status` returns `{jobId, state, progress,
  path?, error?}` and is in the inspection subset.
- **Document and 3D:** `apply_commands`, `edit_video`, `edit_3d`, `get_project_state`,
  `get_3d_scene` through `pm-wasm`; model operations through host services.
- **Media:** imports through the host's media store, always copied unless
  `storage: "link"` (section 21); probes and peaks from the engine; deduplicated on the
  content hash.
- **Analysis:** `probe_media` gains color, HDR, and VFR facts; the other media tools keep
  using originals through the `ffmpeg` executable; retiming math moves to `pm-wasm`.
- **Panels** need a real DOM; on the hidden host they route only to a Chromium tab on the
  session's pinned project.
- **Orchestration:** 8 active subagents per root, depth 4, 32 tasks per run; engine
  sessions are per document, so renders run concurrently.
- **Pinning:** a desktop window with an active agent session asks before switching
  project; on the hidden host each pinned project has its own document host.
- **External MCP limits** stay: 10 s auth, 2 MiB lines, 32 queued requests, one call at a
  time per connection and per stdio relay, 64 MiB responses.
- **New:** `get_render_status` (Phase 3b); `manage_proxies` (Phase 4b).

## 21. Media management and project files

### Where bytes live

- **Media store** owned by the host (main on the desktop), replacing IndexedDB as the
  source of bytes; the engine reads through capability handles.
- **Two storage modes:** Copied into project (embedded in the `.pmv`, as today) and
  Linked (the file stays put).
- **When importing** (from Phase 2): Copy into project (default, decision 2) or Link to
  files.
- **Always copied:** items without a file (clipboard images, web downloads, generated
  stills), Blender sources, imported 3D models, files inside an agent workspace, and
  files imported by agents unless the call passes `storage: "link"`.
- **Files on removable camera media** are copied.
- **Export › Project and agent `save_project` embed every linked original**, so the file
  is self-contained as today.
- Deleting a project deletes its agent workspace (`legacy/core/projects.ts:241-243`) and
  never touches linked originals.

### Relinking

Automatic relink only by content hash, UMID, or content identifier. Name, timecode, or
duration matches are proposals the user confirms (Relink sheet in 4b). Replace File…
keeps the interpretation. Offline clips show "Media offline · A001_C003.mov" and keep
playing with proxies.

### File format

- Desktop Save writes PMV4; Export › Project and the browser fallback write PMV3.
  Linked assets add records without bytes.
- Saves keep the user's file mode, including read-only files.
- Generated media is never stored in the project file.
- **`minReaderVersion` guard**, shipped in Phase 0: a newer project opens read-only with
  "This project needs a newer version of Powermove".

### Switching engines safely

While the legacy engine can be selected (Phases 2 to 5b):

- IndexedDB media is copied to the media store, verified by hash, and never deleted until
  Phase 5c.
- **Capability routing:** with the flag on, a project or frame that uses a feature the
  engine does not support yet routes to the legacy engine (including agent exports).
- Projects using native-only features (section 6 table) are stamped
  `requiresEngine: "native"`; the legacy engine refuses them with "This project uses the
  new engine. Turn on Use New Engine to open it."
- **Gate (from Phase 2):** every corpus project without native-only features round-trips
  flag on → off → on with no loss; stamped projects are refused cleanly.

### Migrating existing projects

- Assets whose original was replaced by a VP9 transcode show "Using a converted copy ·
  Relink to the original for full quality" with Locate Original….
- Embedded media moves to the media store on first open with the new engine.
- Legacy 3D scene and OBJ layers keep rendering; conversion stays a user action.

## 22. Web player, serve, the headless hosts, the CLI

### Web export and `packages/player`

`packages/player` builds `renderer/src/player/*`, which imports `legacy/core`,
`legacy/gl`, `core/scene3d`, and three.js. It powers code export, the www hero (which
scrubs arbitrary times), serve, and a copy vendored into onboarding.

- **Option A (recommended, decision 8).** At Phase 5c the TypeScript renderer moves into
  `packages/player` as a frozen web player v1 for code export, www, and serve. The
  editor imports nothing from it except the code-export bundle string. Code export
  transcodes non-browser codecs to a web rendition through the job engine and warns when
  a project uses native-only features; parity is tested in Standard color with
  `compat` level 1. After cutover, a WASM build of the engine on WebGPU replaces v1.
- **Option B.** The WASM web player is built before Phase 5c, so no TypeScript renderer
  remains anywhere. This adds a large phase before cutover.
- **Onboarding** stops running the vendored player inside the app at Phase 5c: it plays
  a clip pre-rendered by the engine, or renders live through the engine.
- **Phase 5c gate:** a boundary lint proves the editor imports nothing from
  `packages/player` or legacy engine modules, except the code-export bundle string.

### `powermove serve`

Paused. Serve tabs use web player v1 (decision 9); serve previews may differ from the
desktop until a streaming or WASM path exists.

### The hidden MCP host

`powermove mcp` ships today (`apps/desktop/docs/MCP.md`): the production editor in
Playwright Chromium on SwiftShader, with no headless document host
(`server/mcp.ts:43-53`), behind a per-profile lock.

- **Phase 2:** agent `export_video` through the job engine on the desktop app, tested with
  `test-mcp.mjs --desktop` through `--powermove-mcp`.
- **Phase 3c (macOS):**
  - Host services in the CLI process (contract 14).
  - One headless document host worker per open project (`pm-wasm`, kernel, history,
    harness, extension registry), persisted to the profile store (contract 15). Local and
    agent-published extensions register in that kernel; Store extensions in their
    contained sandbox; each registration pushes definition snapshots before the tool
    call returns.
  - Media import into the host's media store.
  - A `pm-node` engine child process with the same restart, attribution, and backoff as
    the desktop.
  - Chromium starts on demand only for DOM tools (`capture_panel`, `computer_use_panel`,
    `interact_panel`, `get_panel_state`, `open_panel`, layout tools) and `raster`-override
    projects, joining the session's pinned project as a tab whose viewer uses CPU frames
    from `pm-node`.
  - The document model stays TypeScript (`server/mcp.ts:26-27`).
  - Gate: agent-tool contract tests produce byte-identical documents on the desktop and
    hidden hosts; `test-mcp.mjs` passes with a video import, a `publish_extensions` plus
    `render_frames` case, a panel case that starts Chromium on demand and fails with a
    named error when Chromium is missing, and verification through our FFmpeg; a run
    under launchd and over SSH with no GUI session.
- **Linux** keeps today's Chromium hidden host until after cutover (decision 16).

### The headless document host

`apps/desktop/src/engine/main.ts` runs `legacy/core` under happy-dom for serve. In Phase 1
it switches to `pm-wasm`. Its fixes: captions tools routed there without
`captions/install`; `editing.ts:927` calling `PM.raster` unguarded; history cleared on
every project switch (`engine/main.ts:106-115`); agent export and save routed to it with
no exporter (`server/runs.ts:20-23,194`); in Phase 3c export goes to the job engine and
save to the host-services packer.

### The CLI

`pm-cli` renders a `.pmv` from definition snapshots with presets, ranges, and parameter
overrides; Blender compositions need a local Blender or fail naming it; it hosts the
parity runner and test sinks. Users reach it through `powermove render`, which accepts any
output path.

## 23. Cross-platform

- **macOS:** Apple Silicon (the CLI also supports darwin-x64).
- **Windows (after cutover):** DX12, D3D11/D3D12 decode through FFmpeg, WASAPI,
  DirectWrite, NT-handle surfaces with LUID matching, Blender device selection for
  OptiX, HIP, oneAPI; the Electron shell needs a Windows build and conventions review.
- **Linux (after cutover):** `pm-cli` and the hidden host with Vulkan or a bundled software
  Vulkan driver (lavapipe or SwiftShader ICD), tested in a GPU-less container.
- Platform code behind traits with software fallbacks for tests.

## 24. Licensing and compliance

Settled with counsel before the dependent work ships.

1. **FFmpeg (standalone, urgent).** The shipped `ffmpeg-static` binary is a third-party
   build whose configuration includes components that must not be redistributed.
   Replacing it is a compliance fix that ships before any Phase 0 release and does not
   wait for the rewrite; its release needs Jude's authorization. Our build: pinned version, LGPL or GPLv3-compatible components
   only, never `--enable-nonfree`; dylibs for the engine plus an `ffmpeg` executable from
   the same configuration (including libfreetype `drawtext`, `silencedetect`, `ebur128`,
   `astats`, scene detection, MJPEG and PNG encoders, libvpx, libopus) for transcription,
   agent media tools, the agent CLI environment, the CLI, and test verification. Counsel
   reviews this item first.
2. **ProRes.** Apple licenses ProRes and names FFmpeg's implementation unauthorized;
   today's export uses `prores_ks`, including on Linux through the CLI. On macOS, ProRes
   decode and encode use VideoToolbox. On Windows and Linux, ProRes decode and encode and
   H.264 encode (libx264) are decision 16.
3. **H.264 and HEVC encode:** OS encoders preferred; royalty exposure confirmed for any
   software encoder.
4. **AGPL and proprietary SDKs:** camera RAW needs a GPLv3 section 7 additional
   permission.
5. **Blender:** user-installed, separate process, Powermove `bpy` worker script
   (GPL-covered when it runs); AGPL-3.0-or-later is GPLv3-compatible; counsel confirms.
6. **three.js** shader ports keep the MIT notice; **Playwright** is optional and downloads
   Chromium on request.
7. **Dependencies:** `cargo-deny` license list and RustSec advisories.

## 25. Testing, parity, and gates

### Baseline

- **E2E ledger.** Phase 0 makes the 228 e2e specs green or quarantined, with owners and
  reasons.
- **Order:** appendix A fixes marked (Phase 0) land first; then the parity corpus is
  captured on the reference machines.
- **Parity corpus:** the 12 pixel-hash scenarios; every e2e fixture; synthetic projects per
  feature; real-world projects Jude supplies (outside the repository); a 3D corpus
  (primitives, lathe and extrude, skinned and animated glTF with source materials and
  every r180 extension listed in section 8, multi-clip files, embedded punctual lights,
  point and line primitives, instancing, transmission, every light type with shadows,
  transparent and double-sided models, 2D barriers and styled groups between depth sets,
  soloed and hidden layers, nested 3D precomps, legacy OBJ, Studio Cube, legacy scene
  layers, studio defaults, missing models, orthographic cameras); Blender surfaces
  captured once with a pinned Blender as fixed inputs; variable-rate MP4 with a matched
  composition; coarse-timebase sources; read-only `.pmv` save.
- **Chromium measurement (Phase 0):** today's conversion for 1-1-1, 6-6-6, untagged SD and
  HD, full-range, and HLG and HDR10 sources; each deviation from E pass-through becomes an
  appendix A item with its own case.

### Parity

- **Evaluation:** |a − b| ≤ 1e-9 + 1e-9·|b|; exact strings and booleans; text per section
  6; 3D picks identical; bounds within 1e-6.
- **Render, Standard color:** per-channel 8-bit difference ≤ 1 for 99.9% of pixels and
  ≤ 4 for any pixel, with these classes tested separately:

  | Class | Test |
  |---|---|
  | Video frames | Chromium comparison only for natively decoded sources, on patch interiors, within 2 CV; the engine within 1 CV of an FFmpeg plus zimg reference using the same chroma filter |
  | Text | Identical line breaks and advances within 0.25 px (level 1); SSIM ≥ 0.98; reviewed sheet |
  | Edges (level 2) | Section 2 antialiasing reference |
  | Blur (level 2) | Analytic Gaussian |
  | Alpha | Within 1 CV of an f64 reference |
  | 3D shading | Section 2 3D bar |
  | Blender surfaces | Engine composite of the stored surfaces, exact |
  | Blender snapshots | For every 3D corpus frame (preview and final, every shutter sample, nested compositions), the `pm-blender` snapshot equals today's `renderSnapshot` output: geometry within 1e-6; materials, maps, blocks, background list, camera, and lights identical; texture bytes equal or listed in appendix A |
  | Appendix A fixes | Listed per case |

  Each real-world project has at most 20% of its pixels in these classes at any sampled
  frame, or a dedicated reviewed comparison.
- **Web player v1** against the engine in Standard color, `compat` level 1.

### Correctness and conformance

Section 2 bars; codec conformance (FFmpeg references, random access, the section 2
corpus); export conformance including sampled pixel values; determinism; fuzzing.

### Infrastructure

- Rust unit and property tests; `pm-cli parity` (evaluation from Phase 1, renders from
  Phase 2) owned by the Parity and QA lane; test-only sinks.
- Playwright e2e through the same UI; specs that read WebGL pixels (including
  `scene3d.spec.ts`, `viewport3d.spec.ts`) rewritten against engine frames.
- Existing harnesses stay: `native-export-performance`, `opaque-capture-parity`,
  `export-video-frames`, `video-frame-integrity`, `rendering-performance`,
  `export-code`, `export-missing-effect`, `scripts/test-mcp.mjs` (with `--desktop` and the
  new cases), `blender-motion.spec.ts`.
- A debug "Compare engines" command until cutover.

### Performance gates

Three reference machines (decision 11): a base M1 MacBook Air (no ProRes engine), a base
M-series Air with a ProRes engine ("base"), an M-series Pro ("Pro"). Calibrated in Phase
0, then fixed.

| Scenario | Gate | Phase |
|---|---|---|
| Frame bridge under main load | S1 gate (section 18) | 0 |
| Cross-process GPU contention | S8: 4K playback in the interactive engine while the job engine exports 4K blur and Blender EEVEE renders; zero drops over 60 s | 0 |
| Layer drag, synthetic 500-layer project of slice features | p95 ≤ 33 ms pointer-to-pixel | 2 |
| Inspector edit during playback, 1,000 layers | p95 ≤ 16 ms, no dropped frames | 2 |
| Scrub, intra codecs | p95 ≤ 50 ms | 2 |
| Scrub, long-GOP, no generated media | p95 ≤ 300 ms | 2 |
| A/V sync | ±10 ms | 2 |
| Cadence | Section 2 | 2 |
| Agent snapshots during playback | 4 frames per agent turn, no dropped playback frames | 2 |
| Export plus playback memory, base 8 GB | No memory-pressure warnings over 10 minutes | 2 |
| Cold start | Engine ready before the window finishes loading the last project | 2 |
| Layer drag, 884-layer project | p95 ≤ 33 ms, at most 1 frame worse than today | 3a |
| 3D pick, 1M-vertex skinned model | ≤ 2 ms at an unchanged frame; ≤ 33 ms for the first pick after a time change | 3a |
| Canvas text typing | p95 ≤ 33 ms | 3a |
| UHD ProRes 422 HQ, 3 layers with blur | Real time at 23.976, 29.97, 59.94 on Pro; 29.97 on base; zero drops over 60 s; same with linear blending on | 3a |
| The 884-layer project | The five ceilings that fail today pass | 3a |
| 4K ProRes 422 HQ export | At least real time on base | 3b |
| Export, every format at cutover | Absolute fps per codec and machine, at least today's re-measured baseline | 3b |
| 3D export, 3 models with shadows, Blender EEVEE, 16 samples | At least today's re-measured baseline; memory gate with this export running | 3b |
| 3D orbit, 3D corpus scene with shadows | p95 ≤ 33 ms pointer-to-pixel | 3c |
| Concurrency | 8 subagents, a second thread, and 2 external connections calling `render_frames` during an export: no failures, no playback pause; `render_frames` 5 × 720 px p95 ≤ 10 s during 4K playback | 3c |
| 4K long-GOP HEVC 10-bit and H.264 4:2:2 10-bit, Automatic | Real time on all three machines | 4a |
| Paused upgrade to original | ≤ 150 ms (4K ProRes, base); ≤ 300 ms (4K long-GOP HEVC, base) | 4a |
| Memory | Within leases; no growth over a 1-hour playback loop; `pm-wasm` budget with the 3D corpus | 3a and 4a |

## 26. Phases

Ordered by dependency; lanes run in parallel (section 27). The legacy engine stays the
default until Phase 5b.

**Implementation run.** Jude asked for the implementation in one run, up to a
beta-ready build (everything through Phase 4a plus the Phase 5a build and the WASM web
player under decision 8). The run stops there for Jude. Nothing is merged and nothing is
released: work stays on branches and stacked PRs are not merged. Each gate has objective criteria; Jude signs off at each gate in
addition.

**Engine-side features during the rewrite** follow decision 18: from Phase 0, a feature
touching `legacy/gl`, `legacy/core` evaluation, `core/scene3d` runtime and rendering, or
export lands with parity corpus cases (once the corpus tooling exists) and a port note; a
hard freeze starts at Phase 3a and ends at Phase 5b.

### Phase 0: Foundations, contracts, spikes

Base-machine gates are deferred (decision 11). Release-dependent items (the `minReaderVersion`
guard, notarizing the placeholder addon) need Jude's release authorization.

Work:

- Cargo workspace, toolchain, `cargo-deny`, napi-rs and wasm-bindgen builds; a placeholder
  addon signed and notarized through the real release script.
- Reference machines set up with a pinned Blender and the local gate tooling (section 19).
- Appendix A (Phase 0) fixes, then the e2e ledger and the parity corpus capture.
- `minReaderVersion` guard in a normal release.
- Opt-in crash and session reporting.
- The Store and local-extension audit for deprecated surfaces and third-party 3D
  renderers.
- The writer-inventory script.
- **Contracts** drafted as Rust types in `pm-protocol` with a short RFC each; after the
  freeze, a change needs an RFC and a version bump.
- Spikes:

| Spike | Lane | Pass criteria | Informs | On failure |
|---|---|---|---|---|
| S1 frame bridge | Engine and platform | Section 18 S1 gate, including 3D orbit and HDR values | Bridge design, contract 2 | Fallback C′, then B; A′ only by Jude's decision |
| S2 precision and encode | Color | Pipeline transparency and codec generation bars on a prototype; VideoToolbox formats; f16 vs f32 on 12-bit and 16-bit sources; HEVC alpha depth; AVAssetWriter tags; QuickTime-matching tags; Metal precise math; `rgba32float` filter and blend support on every reference machine | Contract 4, decision 15 | Adjust precision defaults; drop unsupported 32-bit features per machine |
| S3 shaders | GPU and render | Every repository, Store, docs, and generated agent shader compiles and matches WebGL; naga vs SPIRV-Cross chosen | Shader pipeline | Fallback runner (decision 19) for the residue; pipeline fixes |
| S4 text and vectors | Text and vectors | Layout: identical line breaks and advances within 0.25 px against today on the font corpus. Rendering: Vello coverage with float paint, `vello_gpu`, and Skia Graphite compared with CoreText; a choice made | Text and vector renderers | Level 1 text keeps legacy shims longer; CPU fallback renderer |
| S5 WASM evaluator | Model and evaluation | `evP` < 1 µs cached; patch apply < 0.2 ms property and < 2 ms structural (884 layers); per-window memory budget | Phase 1 | Narrow the mirror; more queries via cached results |
| S6 decode | Media | Corpus decode into wgpu without copies; exact random access including coarse timebases | `pm-media` | FFmpeg software path for failing formats |
| S7 3D | 3D | three.js r180 shading, shadows, area lights, MSAA, skinning, morphs, transmission, and imported-glTF extensions in wgpu within the 3D bar, using three.js's generated shader code through S3 as the default and comparing hand-written WGSL; `pm-geom3d` 1M-vertex skinned pick targets; 16-bit Blender surfaces with straight alpha after AgX into the job engine; Blender batching equality; orthographic framing and glTF texture orientation checked against appendix A | Decision 14, contract 11 | Narrow parity scope with Jude; keep a three.js pixel source for affected features (not headless) |
| S8 GPU contention | Engine and platform | Section 25 S8 row | Contract 13 | Stronger slicing; pause jobs during playback |

Gates, per consumer:

- S5, S4's layout half, and contracts 1, 5, 9 gate Phase 1.
- S1, S2, S6, S8, the reference machine setup, and the remaining contracts gate Phase 2 (the FFmpeg
  replacement has already shipped by then, section 24).
- S3, S4's rendering half, and S7 gate Phase 3a; S7 also gates Phase 3b.

### Phase 1: Evaluation and the write path

`pm-time`, `pm-model` (`v1_ingest`, reconciliation, identity registry), `pm-eval`,
`pm-edit`, `pm-vector` geometry, `pm-text` layout, the font registry in main,
`pm-protocol`, `pm-wasm`; write barrier, flush, identity registry, auditor, dual-answer
mode, production consistency check; the single frame-rate validator; editor-helper shims;
`pm-cli parity` for evaluation. 2D synchronous queries and geometry-dependent edits switch
to `pm-wasm` behind a flag. 3D geometry queries keep their three.js answers through shims
until 3c. The headless document host switches to `pm-wasm`.

Gate: 2D evaluation parity on the corpus; zero dual-answer mismatches beyond tolerance and
zero auditor failures across the e2e suite per the ledger; text sub-gate (line breaks and
advances); native and WASM bit-identical; S5 targets met in the app.

### Phase 2: Tracer bullet

Behind "Use New Engine", with capability routing:

- Engine processes, host services in main, MessagePorts, capability handles, leases, GPU
  time slicing, crash handling.
- The frame bridge with jitter buffer, overlay geometry, view override, depth and ID
  surface.
- `pm-media`: probe, indexes, content hashes, VideoToolbox decode of ProRes, H.264, HEVC;
  image readers; chroma handling and pass-through.
- Minimal `pm-render`: solid, image, video, transform, opacity, normal blend, precomp,
  groups, and one `layer3d` primitive lit by a sun light without shadows; Standard color
  with the invariants.
- `pm-audio`: device clock, streaming decode, mixing; playback; scrubbing.
- `pm-export`: ProRes and H.264 through VideoToolbox in the job engine; atomic output;
  verification; agent `export_video` and agent snapshots through the job engine.
- Media store, linked media, content-hash relink, originals kept on import, stamping, the
  "When importing" setting.
- Font registry and subsetting move into the engine.
- `pm-cli parity` for renders.

Gate: Phase 2 performance rows; Standard color invariants, pipeline transparency, codec
generation, time, and cadence bars; the 3D primitive within the 3D class; crash and
GPU-loss recovery; the IndexedDB copy verified by hash and never deleted; content-hash
relink; agent workspace and agent-imported files always copied; deleting a project never
touches a linked original; the engine-switch round trip; `test-mcp.mjs --desktop` export
through the job engine.

### Phase 3a: Rendering parity

All layer types, shapes and paths, text and captions, masks and mattes, blend modes,
built-in effects and transitions through the GLSL pipeline (WGSL acceleration only where
output-identical), definition snapshots and lifecycle, the shader fallback runner, motion
blur, 2.5D, the full 3D scene pass (section 8), Blender surface compositing and snapshot
parity, adjustment layers, retiming, continuous precomp rasterization on level 2, linear
blending, the internal scope tool.

Gate: render parity including the 3D and Blender classes; the section 11 shader gate
(repository, Store, docs samples, generated agent corpus); linear-blending invariants and
linear blending, antialiasing, 3D, and determinism bars; text review; Phase 3a
performance rows.

### Phase 3b: Media and delivery

Every import format at cutover including 3D models, interpretation and HDR input tone
mapping, VFR and interlacing, every export format and setting at cutover, Blender jobs in
the job engine, presets, the render queue, audio stretch, remapping, channel layouts,
meters, waveforms, J/K/L, audio scrub, `export_video` presets and formats.

Gate: codec and export conformance; bit depth, alpha, SDR color, seeking, and audio bars;
model import correctness on the corpus; J/K/L and audio scrub functional tests; Phase 3b
performance rows.

### Phase 3c: Integration

The kernel API mapping and the synchronous render compatibility path, the compatibility
WebGL context, the `raster` override path, `api.scene3d`, the 3D viewport editor and
chrome on `pm-wasm` geometry and the depth and ID surface, agent and MCP changes, batched
concurrent agent snapshots, the hidden MCP host on macOS, `pm-node` CLI prebuilds for
macOS, every consumer in appendix C, posters, thumbnails, filmstrips, `pm-cli` exports,
code export web renditions, serve's switch to web player v1.

Gate: every appendix C row done and tested; e2e suite green per the ledger; hidden-host
gates (section 22); the concurrency and 3D orbit rows; every apiVersion 1 to 3 extension
in the Store and the local corpus loads and behaves the same.

### Phase 4a: Proxies for cutover

Section 14.2's Phase 4a column.

Gate: Phase 4a performance rows on the three machines without user action; the export
safety byte-identity test; no proxy pixels in exports without a draft choice; proxies
survive moves, relinks, and relaunch; disk guard enforced; the section 2 Proxies bar.

### Phase 5: Cutover

- **5a, opt-in beta:** the new engine as an opt-in with reporting.
- **5b, default on:** for one release, the legacy engine behind a switch.
- **5c, removal:** `legacy/gl`, the legacy media, playback, audio, and export paths, and
  three.js as an output renderer leave the editor; the edit pipeline and document code
  move out of `legacy/`; web player v1 moves into `packages/player` (or the WASM player
  replaces it, decision 8); onboarding stops running the vendored player;
  `EXTENSION_API_VERSION` 4 with the registry release; docs updated (`EXTENSIONS.md`,
  `SCENE3D.md`, `MCP.md`, README, engine docs).

Gates: 5a → 5b requires the 5a cohort's crash-free sessions ≥ 99.5% over at least 2,000
sessions from at least 100 users (or the cohort alternative, decision 10) and zero open P0
or P1 regressions. 5b → 5c requires the same on the default-on release and the
engine-switch round-trip test. 5c is done when the boundary lint passes and docs are
updated.

### After cutover

1. Phase 4b (section 14.2).
2. Professional color and unified 3D tone mapping; scopes.
3. Delivery formats from section 17; audio channel mapping.
4. Camera proxies, Package for Travel, trimmed Collect, verified card offload.
5. WASM web player (if not done under decision 8).
6. Windows; Linux engine prebuilds and the Linux hidden host on the engine.
7. Cameras and lights for 2.5D layers; 2.5D and model intersections; Collapse
   transformations.
8. Spatial bezier motion paths; frame blending and optical flow; faster shuttle; reverse
   audio.
9. Camera RAW.
10. Audio effects and loudness tools.
11. OpenFX hosting out of process.
12. `pm-doc`: edit pipeline and history in Rust, ticks on disk.
13. PSD and multi-part EXR layers; FBX; FCPXML and OTIO; 3D Text from `pm-text`.
14. Shell review: Electron or Tauri (WKWebView differences, the per-process extension
    sandbox, the Node main process, agent panel capture, versus a smaller app and direct
    engine hosting).

## 27. Lanes, contracts, and ownership

### Contracts

Drafted in Phase 0 as Rust types and RFCs; frozen when their consumer phase starts.

1. `EvaluatedScene` (2D, 3D worlds, depth sets and ordering as separate computations).
2. `FrameRequest` and `FrameDescriptor` (region, scale, color, precision, revision, PTS,
   overlay geometry, view override, draft 3D, depth and ID surface).
3. The `pm-gpu` interop trait.
4. The color contract (E, extended curves, encodings, chroma, premultiplication, 3D tone
   mapping, cache-key components).
5. Patches, paths, canonicalization, `v1_ingest`, the field table, engine-originated facts.
6. The playback clock (owner: Audio; Audio joins in Phase 0 for this contract).
7. Errors and isolation (owner: Engine and platform).
8. Budget leases.
9. Protocol codec and code generation.
10. Definition snapshots and lifecycle.
11. The Blender job protocol, snapshot, and surface format.
12. Priority classes and agent frame requests.
13. GPU time slicing and deadline signals.
14. Host services: arbiter, handle minting, output destinations, Blender supervision,
    definition-snapshot source; implemented by Electron main and the CLI host.
15. The headless document host interface: per-project instances, persistence, extension
    runtime, agent tool routing.

### Lanes

| Lane | Owns | Phases |
|---|---|---|
| Model and evaluation | `pm-time`, `pm-model`, `pm-eval`, `pm-edit`, `pm-wasm`, editor-helper shims | 0, 1 |
| Renderer write path | Write barrier, flush, identity registry, auditor, dual-answer mode, consistency check, writer-inventory script, frame-rate validator, `minReaderVersion` and `requiresEngine`, headless document host and its per-project instances | 0, 1, 2, 3c |
| Text and vectors | `pm-text`, font registry, `pm-vector` | 0, 1, 2, 3a |
| 3D | `pm-geom3d`, 3D scene pass, depth and ID surface, `pm-blender`, 3D viewport integration | 0, 1, 2, 3a, 3b, 3c |
| Media | `pm-media`, decoders, readers, index, chroma, content hashing | 0, 2, 3b |
| GPU and render | `pm-gpu`, `pm-render` 2D, GLSL pipeline, definition snapshots, fallback runner | 0, 2, 3a |
| Color | `pm-color`, invariants, interpretation, tone mapping | 0, 2, 3a, 3b |
| Engine and platform | `pm-engine`, `pm-node`, `pm-bridge`, `pm-protocol`, leases, priority and slicing, host services in main | 0, 2 |
| Platform, CI, and compliance | Toolchain, hosted CI, local gate tooling, packaging, signing, FFmpeg build, crash reporting | 0 onward |
| Audio | `pm-audio`, mixer, clock contract | 0, 2, 3b |
| Export and agents | `pm-export`, `pm-cli`, render queue, agent export, save, and snapshots, `pm-host` CLI implementation, the hidden MCP host, CLI prebuilds | 2, 3b, 3c |
| Proxies and storage | `pm-proxy`, `pm-cache`, media store, media management | 2, 4a, 4b |
| Viewer and settings UI | Viewer, overlays, settings, appendices C and D, API mapping, sync compatibility path, extension docs | 2, 3c, 4a, 5 |
| Parity and QA | Ledger, corpus, parity runner, conformance, performance gates, appendix A Phase 0 fixes | 0 onward |

**Hot files** have one owning lane each: `legacy/core/editing.ts` (Renderer write path),
`legacy/gl/raster.ts` (Media), `kernel/api.ts` (Viewer and settings UI),
`preload/index.ts` and `main/index.ts` (Engine and platform), `engine/Cargo.toml`
(Platform, CI, and compliance). Others coordinate through the owner.

Rules: each lane works in its own worktree and branch, commits granularly, and submits
stacked PRs (no squash merges); independent reviewers; the Parity and QA lane owns gates
and reports failures as they are.

## 28. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `sharedTexture` is experimental | Viewer breaks on an Electron upgrade | S1; ranked fallbacks with kill criteria; upgrades gated by a bridge test |
| Gesture and orbit latency | Handles and 3D feel detached | Overlay geometry; depth and ID surface; latency gates |
| Cross-process GPU contention | Playback drops during exports and agent work | S8; GPU time slicing; Blender throttling |
| Main-process jitter | Uneven playback | Import-once pools, small messages, jitter buffer |
| Missed writes | Saved project differs from rendered output | Write barrier, inventory script in CI, production consistency check |
| three.js parity in wgpu | 3D projects look different | S7 using three.js's own shader code; reviewed corpus |
| Imported glTF breadth | Some models render differently | Full r180 loader feature list in spec and corpus |
| Blender snapshot drift | Blender renders change silently | Blender snapshot parity class |
| Blender as an external dependency | Failed or inconsistent exports | Pinned LTS, content-keyed cache, named failures, counsel, scrubbed environment |
| Text looks different | Regressions everywhere | Level 1 compatibility, S4, reviewed corpus |
| A shader cannot be translated | Broken extensions | S3; generated agent corpus; runtime fallback runner |
| Frozen sync APIs | Extensions break | Sync compatibility path through main |
| Hidden host loses capabilities without Chromium | Agent tools break | Per-project document hosts, extension registry, Chromium on demand, byte-identical gate |
| Agent render load | Stutter, slow agents | Priority classes, batching, slicing, latency bound |
| Agent media lost after linking | Broken agent projects | Agent imports always copied; `save_project` embeds |
| Switching engines loses data | Broken projects | Field table, stamping, copied media, round-trip gate from Phase 2 |
| Weak media identity | Wrong footage | Full content hash; re-verification |
| Private corpus exposure | Leak of user projects | No self-hosted runners; the corpus stays on the reference machines |
| `pm-bridge` crash in main | App crash | Small, fuzzed addon; S1 hardening |
| Long freeze before cutover | Product stalls | Phase 4 split; sizes; freeze ends at 5b |
| Engine-side features landing in legacy | Moving target | Decision 18 |
| Toolchain churn | Upgrade cost | Pinned versions; upgrades as lanes |
| Licensing | Blocked features | Section 24; FFmpeg fixed first |
| Memory on 8 GB machines | Crashes | Arbiter, leases, base-machine gates with Blender running |
| Engine crashes | Lost work | Two engines, attribution, backoff, persisted jobs, atomic output |
| Old builds overwriting new projects | Data loss | `minReaderVersion` guard in Phase 0 |

## 29. Decisions

Jude delegated these decisions on 2026-10-11, except the ones marked **Open**.

| # | Decision | Choice |
|---|---|---|
| 1 | Default for new projects | Standard color, "Blend in linear light" off |
| 2 | When importing | Copy into project; revisit after Collect Project ships in 4b |
| 3 | Generated media location | The Powermove cache |
| 4 | Optimized media | Only when I ask (4b) |
| 5 | Disk guard and storage limit | Pause below the larger of 20 GB or 10% free; limit 25% of the cache volume |
| 6 | Order after cutover | As written in section 26 |
| 7 | Intel Macs | No, apart from the CLI's existing darwin-x64 support |
| 8 | Web player and "no trace" | (B): the WASM web player is built before Phase 5c, so no TypeScript renderer remains anywhere. If it proves too complex, code export and the web player are removed instead (Jude accepts this); the www hero and onboarding then play engine-rendered video |
| 9 | Serve interim | The WASM player; if decision 8 falls back to removal, serve previews use CPU frames from a host-side engine or stay paused |
| 10 | Crash and session reporting | Beta cohort submitting local crash and session logs; no new backend. Cutover gates use the cohort: crash-free sessions ≥ 99.5% over at least 500 sessions from at least 20 users. Jude recruits the cohort at 5a |
| 11 | Reference machines | Jude's M4 Max MacBook Pro (128 GB, XDR) is the only reference machine for now. Base-machine gates (M1 Air, base Air) are deferred and marked unverified in gate reports |
| 12 | Licensing | No counsel needed for the pre-cutover macOS scope: an LGPL-only FFmpeg build (no x264, x265, or nonfree components), ProRes only through VideoToolbox, Blender as today. Counsel before Linux or Windows ProRes, software H.264 encode, or camera RAW |
| 13 | Real-world corpus projects | Synthetic projects, e2e fixtures, and Jude's "Powermove ad" project (kept outside the repository) |
| 14 | 3D rendering | Native port of the three.js r180 subset |
| 15 | Blender surfaces at cutover | 16-bit PNG through AgX |
| 16 | Linux | Today's Chromium hidden host stays on Linux until after cutover; Linux and Windows ProRes and H.264 encode decided with counsel then |
| 17 | 3D tone curves | ACES Filmic for Native and AgX for Blender at cutover; unified after |
| 18 | Engine-side feature policy | From Phase 0, features touching the engine land with corpus cases and port notes; hard freeze from Phase 3a to 5b. This binds work by Jude and other agents on main |
| 19 | Shader fallback runner | Keep a minimal in-renderer GLSL runner for shaders that fail translation |
| 20 | CI hardware | No self-hosted runners on the public repository. Hosted runners run unit, lint, and build jobs; parity and performance gates run locally on the reference machines and their reports are attached to each PR |
| 21 | Worker models | Opus 5.5 for implementation and reviews |

## 30. Appendix A: defects found during the survey

Items marked (Phase 0) are fixed in the legacy engine before the corpus is captured.
Line references are at `d5d5b731`.

1. **Converted video replaces the original** (`raster.ts:887-902`, `media/media-path.ts:29`,
   `main/media-proxy.ts:39-51`); `apps/desktop/README.md:149-150` says otherwise
   (`README.md:191-195` is correct). (Phase 0: correct the README.)
2. **Export transfer tag** is sRGB for H.264 and ProRes (`render-encoder.ts:13-16`).
3. **8-bit sources:** ProRes from 8-bit data; 24-bit PCM from 16-bit WAV; 8-bit three.js
   canvases and Blender PNGs; source-material textures re-encoded through Canvas2D
   (`scene3d/rendering.ts:308-337`).
4. **Motion blur sample counts differ by path** (16, 20, 6, 6/12, 10, 1); Blender shutter
   samples copy each path's count; RAM preview prepares 10 Blender samples but renders 6.
   The engine keeps today's counts per output kind on migrated projects.
5. **Fixed** in `aad31a41`: WebM and Recorder exports render with `exporting: true`.
6. **Render queue:** nothing adds jobs; per-job prompts; failure stops the run; the run
   swaps the live project; agent `export_video` bypasses it.
7. **Frame rates differ across four paths** (`editing.ts:526`; load and validation;
   `app.ts:1338-1352`; `mcp-project-tools.ts:5`).
8. **Timecode at fractional rates** is wrong (`util.ts:197-205`), now reached by Home-opened
   29.97 media and still-export names (`exporter.ts:532`). (Phase 0, keeping the function's
   format for integer rates.)
9. **Variable speed** integrates at twice the open composition's rate, at least 60 Hz
   (`retiming.ts:13`).
10. **Audio** ignores clip speed; time-remapped clips are silent.
11. **WebM exports** carry no color metadata.
12. **Aliasing** on 2D layers (hard UV edges, no mipmaps).
13. **Large blur radii** skip texels (`effects-basic/effects.ts:43-49`).
14. **More than 8 parametric masks** are silently dropped on the GPU path.
15. **Solid corner radius** stored but not rendered.
16. **Headless document host:** captions tools fail; text anchor edits can throw
    (`editing.ts:927`); history is cleared on every project switch
    (`engine/main.ts:106-115`), so concurrent runs on two projects break each other's
    rollback.
17. **Docs disagree** on effect varyings (`EXTENSIONS.md:441` vs `api.ts:201-212`).
    (Phase 0)
18. **51 tracked Finder duplicates** (`* 2.*`). (Phase 0: delete.)
19. **`PM.previewFps`** read but never assigned (`engine.ts:47`), also in the preview cache
    key (`compositor.ts:211`).
20. **No version check on load.** (Phase 0: the guard.)
21. **Unbounded caches:** IndexedDB media store, editing copies, live asset map, solver
    caches, font sources, resident GLB models.
22. **Sampled fingerprint** can collide (`media.ts:96-97,153-171`); agent import
    deduplicates on it.
23. **Native and Blender 3D tone curves differ** (ACES Filmic vs AgX).
24. **Variable-rate MP4s set the composition to their average rate**; `mp4FrameRate` ignores
    edit lists.
25. **Headless MCP exports render on SwiftShader.**
26. **Blender export cost** is 3D layers × (1 + shutter samples) renders per frame through
    one worker.
27. **Native 3D silently caps output at 8192 px** (`runtime.ts:262`) while Blender jobs
    reject sizes over 8192 (`blender-service.ts:79`).
28. **The hidden host's import filter accepts `.fbx`**, which import rejects
    (`server/mcp.ts:78`, `raster.ts:587`).
29. **Untrusted `.blend` files** are parsed by an unsandboxed child of main, which inherits
    main's full environment (`blender-service.ts:329`).
30. **Project deletion removes the agent workspace**, assuming its media is copied in
    (`projects.ts:241-243`); section 21 keeps such files copied.
31. **Any edit during an export invalidates prepared Blender frames** (`anim.ts:18`;
    `rendering.ts:183,682`).
32. **Web and code exports of compositions with `render3d.enabled`** likely fail in the
    player: `prepareFrame` requires a Blender bridge (`player.ts:114`,
    `frame-preparation.ts:22-25`, `rendering.ts:687-688`). Inferred from code.
33. **Cycles `device: auto`** makes exports machine-dependent (`blender-worker.ts:274-283`).
34. **The renderer's 512 MiB Blender surface budget** throws at 4K with motion blur
    (`rendering.ts:709-712,755-757`). Inferred from code.
35. **Agent capture is single-flight:** `snapshotAsync` pauses playback and throws during
    another capture or an export (`exporter.ts:856-870`).
36. **Serve routes agent export and save to the headless document host**, which has no
    exporter (`server/runs.ts:20-23,194`). Inferred from code.
37. **Hidden-host and serve agent exports are not exclusive or owner-only**
    (`electron-stub.ts:85-104`).
38. **Blender orthographic framing** fits the wider dimension (sensor fit AUTO,
    `blender-worker.ts:293-294`) while native uses a fixed half-height of 3
    (`runtime.ts:235`), so landscape orthographic renders differ. Inferred; S7 verifies and
    the decision on which to preserve is Jude's.
39. **glTF source textures in Blender** are probably upside down (raw UVs with
    non-flipped textures, `rendering.ts:391`). Inferred; S7 verifies.
40. **Model-group bounds cache skinned boxes on first call** (`service.ts:72`); the engine
    computes them per frame.

## 31. Appendix B: feature parity inventory

Everything below works in the engine before cutover.

**Composition.** Size; rational frame rate; duration; work area; background (none, solid,
linear and radial gradient up to 8 stops, radial with angle only); shutter; master audio
gain; markers; scene parameters (`param()`, `setParameter`); multiple compositions;
nesting with cycle checks; pre-compose (move, leave attributes); 3D render settings
(Native, EEVEE, Cycles; samples, preview samples and scale, denoise, device); matching to
media opened from Home.

**Layers.** solid, shape (rect, ellipse, polygon, star, line, editable paths), text, image,
SVG, video, audio, adjustment, shader, extension (fragment including Studio Cube, mesh
including third-party definitions with `assetField`, `layer3d` object, light, camera,
legacy `scene3d`), null, group (including model groups), precomp, captions. Fields:
visibility, lock, shy, solo, collapsed, color label, blend, motion blur, 3D, linked scale,
parent, group, track, matte source and mode (keyframable where they are today). Parenting
that preserves pose and aim.

**Transforms.** Anchor, position, scale, rotation, opacity, skew (±80°); Z position, anchor,
scale; X and Y rotation; orientation; perspective per 3D group.

**Animation.** Linear, bezier, hold; speed and influence; auto-bezier; continuous; springs
(including agent-authored); legacy easing presets; color interpolation; string and boolean
steps; `offset_property`; negative shared-material key times. Expressions: `time`, `T`,
`t`, `fps`, `value`, `layer`, `comp`, `ch`, `idx`, `PI`, `Math.*`; `clamp`, `lerp`,
`linear`, `ease`, `random`, `wiggle`, `bounce`, `loop`, `pingpong`, `param`.

**Shapes and paths.** Fill and stroke, per-corner radius, smoothing, polygon and star
points, cubic paths, trim and repeater (up to 256) on editable paths, fill and stroke
opacity, SVG import to paths.

**Text.** All properties, variable axes, range styles, animators, font anchoring, split
text, local and embedded fonts, canvas text editing.

**Captions.** Box (color, opacity, padding, radius), stroke, highlight color and scale, pop
and fade word animation, text case, max lines, safe margin, placement and offset; SRT and
VTT import and export; burn-in.

**Masks and mattes.** Up to 8 parametric GPU masks (rect, ellipse; feather, rotation, add,
subtract), bezier path masks, animatable mask enables, four track matte modes, matte
nesting, cycle rejection.

**Compositing.** Nine blend modes (including difference), opacity, group effects,
adjustment layers, motion blur, transitions.

**Effects and transitions.** The 16 effects and 4 transitions with today's look, the full
GLSL contract (section 11), extension effects and transitions, forks of built-ins, missing
definition behavior, LIFO registration and hot reload.

**Shader and extension layers.** Shadertoy-style header, `@param` uniforms with today's
binding quirks, five presets as user GLSL, Studio Cube, OBJ model.

**3D.** Everything in section 8's 3D layers subsection: depth sets, ordering, background,
per-fragment tone mapping, materials and maps, every imported-glTF feature r180 honors,
skinning, morphs, multi-clip animation, all light types, shadows as specified, cameras,
studio defaults, missing-model box, editor looks, legacy kinds. Plus: model groups with
geometry pivots; primitives, imported GLB, glTF, and OBJ, indexed meshes, lathe, extrude;
recipes (Rocket, Staircase, 3D Text, lathe, extrude, mesh) with bevel, subdivision,
solidify, array, mirror, and animated parameters; material slots, shared materials, and
whitelisted Blender shader graphs with animated inputs; Native, EEVEE, Cycles rendering;
Edit Mode; Lock Camera to View; Auto Keying; legacy conversion.

**Retiming.** Speed, time remap, freeze, reverse, source time, independent decoders per
instance.

**Media.** Every import format today including 3D models, animated images, image sequences,
posters, iCloud materialization, URL import for extensions, remote media in serve mode, OBJ,
SRT and VTT import, clipboard paste, Home's composition matching.

**Audio.** Per-layer gain, mute, solo, fades, keyframed gain, precomp nesting, embedded
audio, meters, waveforms, master gain.

**Preview.** Resolution, Automatic quality, frame cache, cached preview, retained
presentation, viewport-cropped rendering, context-loss recovery, FPS readout, paused Blender
Rendered preview.

**Export.** Native MP4 and ProRes, WebM, PNG sequence, still, code export (including native
3D), project export, presets, defaults, quality tiers, scale and custom size, frame-rate
list, ranges, captions, progress with live preview, cancel, render queue, Blender-rendered
compositions.

**Kernel, agents, and MCP.** Every API in section 20, including synchronous render APIs for
apiVersion 1 to 3, editor helpers, `getShaderMeta`, and event identity; every agent and MCP
tool; agent export delivery; agent media management; orchestration.

## 32. Appendix C: integration checklist

Every consumer of today's engine outside the compositor, gated in Phase 3c (agent rows by
the Export and agents lane, 3D rows by the 3D lane, the rest by the Viewer and settings UI
lane).

| Consumer | Today | Replacement |
|---|---|---|
| Composition thumbnails (`panels/comp-thumbnails.svelte.ts`, `draft3d`) | `PM.GL.renderToPixels` | Async engine thumbnails, draft 3D |
| Project thumbnail on save (`legacy/app.ts:578-587`) | Viewer canvas | Engine snapshot |
| Library thumbnails and viewer card (`library.ts:87`, `LibraryScreen.svelte:377-386`) | `PM.Export.snapshot`, live canvases | Engine snapshots |
| Assets panel posters, inline video, audio thumbnails | IndexedDB posters, blob-URL `<video>`, WebAudio peaks | Engine posters, filmstrips, browser-playable renditions, peaks |
| Source preview monitor | `<video>`, `<audio>` | Engine frames and audio |
| Timeline waveforms; mixer meters, solo, retune | WebAudio | Engine |
| Caption rendering | Canvas2D | `pm-text`, `pm-vector` |
| Text editor caret and font offset; text animators; stagger; split text; editor helpers | `textLayout`, `raster().fontOffset`, `Intl.Segmenter` | `pm-wasm` |
| Align, group, comp bounds; model-group bounds | `PM.GL.bounds`, three.js `Box3` | `pm-wasm`, `pm-edit` |
| 3D picks, bounds, ordering (`compositor.ts:1896-1946`, `service.ts`) | three.js runtime | `pm-wasm` |
| 3D viewport chrome (`overlay.ts`, `gizmo.ts`) | three.js redraws the scene for depth and masks | three.js chrome with the engine's depth and ID surface |
| Edit Mode, modal transforms, 3D cursor, navigation | three.js geometry and raycasts | `pm-wasm` |
| Blender preview, state, capture, cancel | ImageBitmap surfaces in the renderer | Engine Blender jobs through host services |
| Recipe preview, model import and disposal, `prepareImport` | TypeScript and three.js loaders | `pm-geom3d`, engine import |
| `get_3d_scene`, `edit_3d` model jobs | `evP` per channel, renderer Blender calls | `pm-wasm`, host services |
| Shader compile status, `getShaderMeta` | `compileError`, `dropProgram`, compositor writes | Engine events |
| Status bar, performance panel, frame bus | `GL.stats`, GPU timing, `overlay` after render | Engine stats; on presentation |
| Export progress preview; RAM preview | Live frames; `ImageBitmap`s | Streamed frames; engine memory cache |
| Posters; cloud and remote media; image sequence import | Renderer decode | Engine |
| Fonts | `document.fonts`, `queryLocalFonts` | Font registry and subsets |
| Panel popout | Same-realm windows | Frames to the hosting window |
| Agent frames (`harness.ts:177-202`, `spatial.ts`) | `snapshotAsync`, pauses playback | Batched engine requests |
| Agent export and save | `PM.Export.run`, one per window | Job engine; host services packer |
| Agent media import and replace (`media-management.ts`, `mcp.ts:73-115`), media source resolution, `list_media`, `check_project` statuses | Renderer, IndexedDB, Playwright `setInputFiles` | Host media store, engine probes |
| `generate_captions`, `publish_extensions` snapshot refresh | Renderer | Engine and definition snapshots |
| Composition contact sheet | `render_frames` loop | Engine batch |
| Hidden MCP host; serve routing (`runs.ts:20-23,194`) | Playwright Chromium; exports to the document host | Section 22; job engine |
| Code export | Embeds stored media bytes | Web renditions through the job engine |
| Web player, www, serve | TypeScript engine | Web player v1 (decision 8) |
| Onboarding | Vendored `player.js` in the app | Engine-rendered clip or live engine |
| Headless document host | `legacy/core` under happy-dom | `pm-wasm`, per-project instances |

## 33. Appendix D: media and proxy interface

All surfaces follow the design language: quiet text in lists, Settings-sheet cards and rows
for dialogs and sheets, NSMenu only for right-click, the app's popover menu for button
menus, the app's confirm dialog for confirmations, sentence case in Settings, title case in
menus.

### Settings › Project › Media

| Setting | Options | Phase |
|---|---|---|
| When importing | Copy into project, Link to files | 2 |
| Create proxies | When needed, For all video, Only when I ask | 4a |
| Proxy size | Automatic, Half, Quarter, 1080p, 720p (Automatic never below 540p) | 4a |
| Proxy format | ProRes 422 Proxy, ProRes 422 LT, HEVC (smaller files) | 4a |
| Create optimized media | Only when I ask, When needed, For H.264 and HEVC video | 4b |

### Settings › Project › Composition

Motion blur samples per output kind; audio sample rate.

### Settings › General › Media (this Mac)

Generated media location (Powermove cache, Next to the project, Choose Folder…), 4a;
Background rendering (When plugged in, Always, Never), 4b; Storage (space used per cache,
Clear… and a limit), 4b.

### Asset right-click menu (4a)

Create Proxy · Rebuild Proxy · Delete Proxy · Always Play Original (check item) · Interpret
Footage… · Replace File… · Reveal in Finder. In 4b: Create Optimized Media, Delete Proxy
and Optimized Media.

### Viewer menu (4a)

The viewer's resolution control (a native `<select>` today, `viewer/viewer.ts:883`) becomes
a popover menu; `controls/popover-menu.ts` gains per-section headers, one radio group per
section, and a check item:

| Section | Items |
|---|---|
| Resolution | Automatic, Full, Half, Quarter |
| Media | Automatic, Originals, Proxies |
| (none) | Play Every Frame (check item) |

The button label states what is on screen when degraded ("Half · Proxy") and shows "Full"
once a paused frame upgrades. "Proxies" works like Final Cut Pro's Proxy Only. A keymap
command toggles Originals and Proxies. The 3D viewport's shading modes stay in its own
header.

### Status and diagnosis

"Creating proxies · 3 of 12"; "Proxies paused · Low disk space"; unchanged during playback.
After drops: "Dropped 14 frames · 2 clips decode slowly" with Create Proxies, or "Dropped 14
frames · GPU busy". Errors say what failed and what to do.

### Storage (4b)

Clear… confirms with the size freed and what is kept; pinned media is never removed.

### Offline media and Blender at export

Native three-button message: "2 clips are offline" / "Their original files can't be found.
Locate them to export at full quality, or export these clips from their proxies at lower
resolution." / Cancel, Export with Proxies, Locate Media…. Queue: "Waiting for media · 2
clips offline". Draft option: "Render using: Proxies (draft)". Blender missing: "This
composition renders with Blender. Install Blender 4.5 or later, or set its renderer to
Native."

### Import and relink

"Copied 12 clips · Linked 3"; "Media offline · A001_C003.mov" and a timeline label
"Offline"; "Using a converted copy · Relink to the original for full quality" with Locate
Original…; "Variable frame rate · Plays at 60 fps by timestamp".

### Render cache (4b)

A 2-pixel track under the ruler; View › Show Rendered Frames; Render Selection; Delete
Rendered Frames; Keep Rendered on precomps. Render Work Area exists from 4a.

## 34. Appendix E: glossary

- **Tick:** 1/705,600,000 second.
- **Display encoding E:** the sRGB piecewise curve, extended to all reals, shared by sRGB and
  Rec.709 by default.
- **Standard color:** intermediates in E; today's look.
- **Linear blending:** "Blend in linear light" on.
- **`compat` level:** 1 keeps today's behavior for existing content; 2 gets fixes.
- **Depth set:** the 3D layers sharing a depth prepass and shadow maps with a given layer.
- **View override:** editor camera, shading, X-ray, Local View; preview only.
- **Depth and ID surface:** a per-frame engine output used by the 3D viewport chrome.
- **Blender surface:** a per-layer image rendered by Blender and composited by the engine.
- **Canonical patch:** a document patch without the open-composition root rotation.
- **Mirror:** a patch-fed copy of the document (`pm-wasm`, engine sessions).
- **Write barrier, identity registry, consistency check:** the mechanisms that keep mirrors
  equal to the document.
- **Edit kernel:** a geometry-dependent edit computed in `pm-edit`.
- **Definition snapshot:** a registered definition's renderer, GLSL, and parameters.
- **Sync compatibility path:** synchronous render APIs served through main for apiVersion 1
  to 3.
- **Host services:** arbitration, handles, destinations, Blender supervision, snapshots;
  provided by Electron main or the CLI host.
- **Priority classes, GPU time slicing:** how interactive work stays ahead of jobs.
- **Representation:** original, reduced-resolution decode, proxy, optimized media, kept
  render.
- **Machine profile:** measured decode, GPU, disk, and power characteristics.
- **Lease:** a memory allowance from the host to an engine process.
- **Frame bridge:** the shared-surface path from engine to viewer.
- **Interactive engine, job engine:** the two engine processes.
- **Hidden MCP host:** `powermove mcp`.
- **Pointer-to-pixel:** input timestamp to presentation of the first frame including it.
- **Parity corpus, E2E ledger:** captured expected outputs; quarantined specs with owners.
