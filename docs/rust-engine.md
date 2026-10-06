# Powermove native engine: plan and design

Status: proposal, not started. Revision 3.

This document covers the whole rewrite of the Powermove engine (evaluation,
decode, compositing, color, text, audio, playback, caching, proxies, and export)
from TypeScript and WebGL2 into Rust. The Svelte UI, the kernel, extensions, and
the agent harness stay on the web stack. It is written to be executed by
parallel implementation lanes. Every phase ends at a gate with objective exit
criteria; a phase is done when its gate passes, not when its code lands.

"The engine" means the new native engine. The existing Node host in
`apps/desktop/src/engine/` is called "the headless document host".

## Summary

**What users get at cutover**

- ProRes, DNxHR, MXF, HEVC 10-bit, and every format imported today play and
  export from the original files, never from a lossy transcode.
- Float precision end to end, correct color tags, exact frame rates and
  timecode, sample-accurate audio that follows speed changes.
- Antialiased edges, sharp scaled text and vectors, correct blur at every radius.
- Optional linear-light blending.
- Automatic proxies that need no setup, a viewer that always says what it is
  showing, and exports that never contain proxy pixels by accident.
- Linked media for large footage, relinking, a working render queue, editing
  while exporting.
- Agents and a CLI that render without a visible window.

**After cutover** (each its own plan): ACES, wide-gamut and HDR projects, scopes,
LUTs and color effects, Windows, camera and light layers, camera RAW, the WASM
web player, OpenFX.

**Phases**

| Phase | Outcome |
|---|---|
| 0 Foundations | Contracts frozen, spikes proven, CI hardware, FFmpeg replaced, parity corpus captured |
| 1 Evaluation | The Rust evaluator answers every synchronous UI query, with a write path that keeps it exact |
| 2 Tracer bullet | Engine processes, frame bridge, ProRes and H.264 playback and export from originals, linked media |
| 3a Rendering parity | Every layer, effect, text, mask, blend, motion blur, 2.5D feature |
| 3b Media and delivery | Every import and export format today, render queue, audio |
| 3c Integration | Kernel API, agent tools, every engine consumer, CLI |
| 4 Proxies and caches | Automatic proxies, render cache, media management |
| 5 Cutover | New engine default, legacy engine removed |

**Cutover is done when** the Phase 5 gate in section 26 passes: parity on the
corpus, every performance gate, a soak on a switch release, and legacy code
deleted.

**Decisions needed** are in section 29.

Contents

1. Why
2. Goals, non-goals, and the quality bar
3. What exists today
4. Architecture overview
5. Time
6. The document: ownership, writes, mirrors, engine facts
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
20. Kernel API, extensions, the agent harness
21. Media management and project files
22. Web player, `powermove serve`, the headless document host, the CLI
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
  path and fingerprint are (`legacy/gl/raster.ts:874-889`).
- **Precision.** Source textures are RGBA8 with premultiply-on-upload
  (`legacy/gl/compositor.ts:541-545`). Intermediates are RGBA16F, but export
  readback is 8-bit (`compositor.ts:1825`). ProRes 4444 is written at 10-bit
  with `-alpha_bits 16` from 8-bit data.
- **Color.** No color management. Blending, blur, opacity, and motion blur run on
  gamma-encoded values. Import ignores color metadata. Export tags H.264 and
  ProRes with the sRGB transfer (`iec61966-2-1`).
- **Time.** Float seconds everywhere. The UI and `set_composition` round frame
  rates to integers; load and validation keep fractions. Timecode is wrong at
  fractional rates. Nested compositions are never re-quantized.
- **Audio.** WebAudio with whole-file decode and a 128 MiB PCM budget (about 5.8
  minutes of stereo float). Speed changes are ignored; time-remapped clips are
  silent.
- **Export.** Frames are rendered in the renderer, read back, and copied over IPC
  in 4 MiB chunks to FFmpeg. A 4K frame is 33 MB. H.264 export of a trivial 4K
  project measures about 11 fps (`apps/desktop/docs/native-export-performance.md`).
- **Headless.** No pixels without a browser tab.

## 2. Goals, non-goals, and the quality bar

### Goals

1. **Image fidelity.** Float precision from decode to encode.
2. **Color correctness.** Explicit, tested color invariants; per-source input
   transforms; optional linear-light blending; later OCIO, ACES, HDR.
3. **Native codecs** from the original media, hardware-accelerated where
   available.
4. **Exact time.** Rational frame rates, integer ticks inside the engine,
   frame-accurate decode, drop-frame timecode, sample-accurate audio.
5. **One renderer** for preview, export, thumbnails, snapshots, and the CLI.
6. **Proxies done right** (section 14).
7. **Speed.** Real-time multi-layer 4K playback on Apple Silicon, pipelined
   export, no rendering on the UI thread.
8. **Keep what works.** The UI, the kernel and its frozen contracts, extensions
   and the Store, the agent harness, Undo, the file format's durability.
9. **Portability.** Platform code behind traits; Windows is a port.

### Non-goals before cutover

- Rewriting the UI, timeline, inspector, or kernel in Rust.
- Changing the document's JSON shape or the public API's units (section 6).
- New delivery formats beyond today's list, ACES, HDR and wide-gamut projects,
  scopes as a product feature, camera RAW, OpenFX, camera and light layers.
  These follow cutover (section 26).

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
| Time | Frame math verified headlessly to 10⁸ frames at every rate. Exports carry a burned-in binary frame code: 10,000 consecutive frames per rate (23.976, 24, 25, 29.97 DF and NDF, 59.94, 60), plus 100-frame windows starting at N = 10⁶ and 10⁷. Drop-frame vectors: 29.97 frame 1800 = 00:01:00;02, frame 17982 = 00:10:00;00; 59.94 frame 3600 = 00:01:00;04. Invalid drop-frame strings are rejected. | 2 |
| Seeking | For every supported codec, including open-GOP HEVC, B-pyramid H.264, and files with edit lists, random access to frame N returns the pixels of sequential decode of frame N. | 3b |
| Audio | A click at frame N lands at sample floor(N × sr × den / num) ± 1. Total exported samples equal round(duration × sr). No length limit. | 3b |
| Antialiasing | Zone-plate and Siemens-star renders under animated scale 1.0 → 0.25 and rotation, compared with the path's declared kernel evaluated at 16× supersampling: PSNR ≥ 40 dB on export and paused frames (Lanczos), ≥ 34 dB during playback (trilinear); flicker energy at most 1.5× the reference. | 3a |
| Proxies | No exported frame contains proxy or reduced-quality cache pixels unless the user chose a draft render. A paused frame shows the original within 150 ms (4K ProRes) and 300 ms (4K long-GOP HEVC) on the base reference machine. Proxy and original at matched scale differ by at most ΔE2000 1.0. | 4 |
| Parity | Every corpus project renders in Standard color within the tolerances of section 25. | 3a |
| Determinism | Two renders of the same frame on the same machine produce byte-identical buffers and byte-identical lossless outputs; native and WASM evaluation are bit-identical. | 1 and 3a |
| HDR color | PQ and HLG outputs within ΔE_ITP (BT.2124) 1.0. | After cutover |

## 3. What exists today

Full inventories: appendices B and C.

### Code the engine replaces

Non-test lines, excluding tracked `* 2.ts` duplicates.

| Area | Main files | LOC |
|---|---|---|
| Evaluation, animation, expressions, retiming, 3D math, text animators | `legacy/core/{anim,temporal-bridge,easing,expression,retiming,parenting,layer-groups,space-3d,text-animation,text-layout,split-text,vector-paths,content-properties,size-anchor,font-anchor,matte}.ts`, `core/anim/temporal-ease.ts` | 3,190 |
| Compositor, rasterizer, shaders, GL caches, asset import half of `raster.ts` | `legacy/gl/*` | 4,196 |
| Media import, decode, posters, editing copies, preview caches | `legacy/core/{media,media-conversion,video-*,image-sequence,poster,preview-cache,preview-quality,memory,cloud-media}.ts`, `main/media-proxy.ts` | 1,890 |
| Playback loop | `legacy/core/engine.ts` | 503 |
| Audio and mixer engine | `legacy/core/audio.ts`, non-UI mixer | 1,825 |
| Export | `legacy/core/{exporter,frame-capture,frame-preparation,render-queue}.ts`, `main/render-encoder.ts` | 1,003 |
| Built-in GLSL effects, transitions, 3D layers | `extensions/{effects-basic,transitions-basic,layers-3d}` | 574 |
| Other engine code | `legacy/core/{compositions,svg-import,fonts}.ts`, `kernel/{glsl,obj,extension-layers}.ts`, `captions/render.ts` | 1,540 |

About 14,700 lines. The web player (`renderer/src/player/*`, 873 lines) runs the
same code (section 22). The Rust replacement will be larger, because it takes
over decode, color, text shaping, and audio work the browser does today.

### What stays

- The renderer UI, panels, Settings, and the built-in UI extensions.
- The kernel and its public API, extended as in section 20.
- The edit pipeline (`legacy/core/editing.ts`), history and Undo, the command
  vocabulary, validation, and composition management
  (`legacy/core/compositions.ts`), with geometry calls redirected.
- The agent harness, MCP bridge, and main-process media tools.
- The `.pmv` container and its durability.

### Constraints inherited from earlier decisions

From `apps/desktop/docs/*`:

- One compositor for preview, capture, export, and nested comps.
- Pixel-exact acceptance for optimizations.
- Bounded expression interpreter (2,000 characters, 160 nodes, depth 24, 12
  arguments), no `'unsafe-eval'` in the editor.
- One user intent equals one Undo step.
- Effect contract limits: effects 32 params, 1 to 8 passes, fragment at most
  65,536 characters; extension layers 48 params, fragment at most 96,000 UTF-8
  bytes. GLSL ES 3.00 in projects and Store extensions keeps working.
- Per-layer failure isolation.
- GPU loss recovery without replacing the project; exports abort on loss without
  bad bytes.
- Predictability over raw speed.

## 4. Architecture overview

```
┌──────────────────────────── Electron app ──────────────────────────────┐
│  Renderer, per window (sandboxed, context-isolated)                     │
│  ├─ UI, kernel, extensions, edit pipeline, history (TS, seconds)        │
│  ├─ pm-wasm: canonical mirror, evaluator, text layout, edit kernels     │
│  └─ Viewer: WebGPU canvas drawing engine frames + DOM/2D overlays       │
│        ▲ MessagePort: canonical patches, control, events                │
│        │                                     ▲ VideoFrame               │
│  Main process                                │                          │
│  ├─ windows, files, save, agent bridge, media store, budget arbiter     │
│  └─ pm-bridge (napi): Mach rendezvous, surface import ─┘                │
│        ▲ surface rights once; per-frame descriptors                     │
│  Interactive engine (utilityProcess, pm-engine via napi-rs)             │
│  ├─ sessions, pm-eval, pm-render (wgpu), pm-color, pm-text, pm-vector   │
│  ├─ pm-media, pm-cache, pm-audio (device clock, mixer)                  │
│  Job engine (second utilityProcess, same library)                       │
│  └─ exports, generated media, background renders                        │
└─────────────────────────────────────────────────────────────────────────┘
Other hosts of the same crates: pm-cli (headless), pm-wasm in the headless
document host.
```

1. **No engine work on a UI thread.**
2. **Synchronous answers stay synchronous.** About 130 built-in call sites ask
   for evaluated values, geometry, picks, or text layout synchronously.
   `pm-wasm` answers from a mirror after flushing pending writes (section 6).
3. **Frames stay on the GPU.** A pool of shared surfaces imported once into main;
   one small message per frame (section 18).
4. **One renderer** for every output.
5. **Two engine processes.** A crash in one does not take down the other. Main
   arbitrates memory between them.
6. **Transport-agnostic protocol** over MessagePorts, in-process, or a socket.

### Crate layout

A Cargo workspace at `engine/`.

| Crate | Responsibility | Targets |
|---|---|---|
| `pm-time` | Ticks, rationals, frame math (i128, Euclidean division), timecode | native, wasm |
| `pm-model` | Canonical model, `v1_ingest`, id-keyed reconciliation, validation | native, wasm |
| `pm-eval` | Keyframes, ease, expressions, parenting, groups, 2.5D, retiming, text animator state, path and mask geometry, pick and bounds | native, wasm |
| `pm-edit` | Geometry-dependent edit kernels returning patches | native, wasm |
| `pm-text` | Font registry, subsetting, shaping, layout, outlines, caret geometry | native; wasm (layout) |
| `pm-vector` | Paths, trim, repeat, strokes, booleans, squircles, coverage | native; wasm (geometry) |
| `pm-color` | Transfer functions, working spaces, tone and gamut mapping, OCIO bridge (after cutover) | native |
| `pm-gpu` | wgpu device, pools, shader compilation, GLSL compatibility, interop trait | native |
| `pm-render` | Render graph, layers, effects, compositing, motion blur | native |
| `pm-media` | Probe, demux, decode, index, image readers, color metadata | native |
| `pm-cache` | Frame cache, render cache, disk caches, budget leases | native |
| `pm-proxy` | Representation policy, segment jobs, generated-media store | native |
| `pm-audio` | Device and clock, mixer, automation, stretch, waveforms, meters | native |
| `pm-export` | Encoders, muxers, render queue, verification | native |
| `pm-engine` | Sessions, scheduler, playback, crash attribution | native |
| `pm-protocol` | Messages, MessagePack codec, TypeScript types via `ts-rs` | native, wasm |
| `pm-node` | napi-rs host for engine processes | native |
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
- **Frame math** in i128 with Euclidean division (correct for negative times,
  which groups can have):
  - `frame_start(n) = ceil(n × den × TICKS / num)`
  - `frame_of(t) = (t × num).div_euclid(den × TICKS)`
  - Property test: `frame_of(frame_start(n)) == n` for every supported rate and
    for arbitrary rationals, up to 10⁸ frames.
- **Media timebases** that do not divide the flick (MP4 timescales 12288 or
  15360, microseconds, nanoseconds) convert in i128 with round-half-even. The
  media index keeps native PTS.
- **Timecode** only at UI and IO edges, with SMPTE drop-frame for 29.97 and
  59.94.

### At the document boundary

The document and public API keep float seconds (section 6). `pm-model::v1_ingest`
converts in one place:

1. If `|t × fps − round(t × fps)| < 1e-4` against the owning composition's grid,
   convert by frame index: `frame_start(round(t × fps))` under the exact rational.
   This keeps keys and in-points on frame n even when the stored rate is the float
   29.97.
2. Otherwise convert to the nearest tick.
3. Rates come from `fpsRational` (new optional `[num, den]`) when present.
   Otherwise known NTSC floats map to their exact rationals and other values to
   the smallest rational that reproduces the float within 1e-9.
4. Writing back: `pm-edit` echoes the original f64 for every time it did not
   change, so edit kernels never rewrite stored values they did not edit. Times it
   creates are `ticks / TICKS`.

### Rules

1. **One frame-rate validator** for the UI, `set_composition`, load, and
   validation (Phase 1). It accepts rationals and stores `fps` plus
   `fpsRational`. New rates must have a frame duration that divides the flick;
   unusual rates already in projects keep working through the ceil and Euclidean
   math above.
2. UI-created keyframes land on frame boundaries in composition time.
3. Nested compositions sample their children at the parent time by default.
   "Keep frame rate when nested" quantizes the child to its own rate, phased from
   the precomp's start.
4. Variable speed integrates in ticks at the owning composition's rate (today:
   twice the open composition's rate, at least 60 Hz).
5. Export frame rate can differ from composition rate; each export frame
   evaluates at its exact tick.
6. Source frame selection: the last frame with PTS at or before the source time,
   as today. Frame blending and optical flow come after cutover.
7. Motion blur uses shutter angle and phase. Today's default (0.5 frame,
   centered) maps to 180° at −90°.
8. Expressions see `time`, `t`, and `fps` as floats, plus a new integer `frame`.
   `time` is nudged by at most one tick so `floor(time × fps) == frame`.

## 6. The document: ownership, writes, mirrors, engine facts

### Authority

The renderer stays the authority for the document and Undo. The JSON,
`core/types/project.ts` (shipped to extensions and agents in the api-pack), the
kernel API, and `EditCommand`s keep float seconds and their shape.

Additive document fields before cutover:

| Field | Purpose |
|---|---|
| `fpsRational` | Exact composition rates |
| `minReaderVersion` (root) | Older builds refuse instead of overwriting (section 21) |
| `requiresEngine: "native"` (root) | Set when a project uses a native-only feature; the legacy engine refuses it (section 21) |
| Layer `compat` | Compatibility level. Layers created before cutover have level 1: legacy text metrics, solid corner radius ignored. New layers have level 2 |
| Asset `link` | Linked-media reference: path, volume identity, fingerprint, content hash, size, modification time, camera metadata |
| Asset `interpret` | User overrides of interpretation only (color, alpha, rate, field order, pixel aspect, LUT) |
| Project `color`, `media` | Project settings (sections 9 and 14) |

**Not in the document:** representations (proxies, optimized media) and their
availability, probe results, measured decode cost. They live in the engine's
media index and generated-media store and are keyed by content, so they never
travel inside a project and never cause document churn.

Removing root rotation and storing ticks on disk wait for `pm-doc` after
cutover.

### The patch stream

1. The renderer produces leaf patches (`src/shared/patch.ts`) in canonical form,
   using `PM.Comps.canonicalize` for every patch, not only history entries.
2. `pm-wasm` ingests them through `v1_ingest` and applies them with **id-keyed
   reconciliation**: when a patch replaces an array (today `diffPatches` replaces
   the whole array on any length or order change), items are matched by `id`, so
   unchanged layers keep their revisions and caches.
3. `pm-wasm` emits canonical binary patches to the engine session. One
   translation point; both mirrors hash identically per revision.

### Making every write visible

Today's writers: about 19 direct `PM.hist.do` sites and 18 `api.history.do`
calls, the edit engine, live gestures, `anim.setKey`, asset metadata
(`raster.ts:970,990,1026,1098,1277,1314`), in-place path and mask edits followed
by `anim.touch()` (`viewer/path-editing.ts:75,80`,
`inspector/MasksSection.svelte:26`, `inspector/retiming.ts:47`,
`timeline/graph-controls.ts:32`), `shaderHooks.syncShaderUniforms`
(`inspector/shader.ts:39-51`), extension-layer refresh
(`kernel/install.ts:789-798`), the render queue's project swap
(`render-queue.ts:10-12`), and `api.host.pm` in user extensions.

1. **Write barrier.** Every write records its dirty path. In development and test
   builds, `PM.proj` is wrapped in a Proxy that records paths and throws on writes
   outside a known writer. In production, inventoried sites call
   `doc.write(path, value)` or `doc.touched(path)`.
2. **Flush before query.** Every `pm-wasm` query drains dirty paths into patches
   first, so reads that follow writes inside a transaction (pose preservation in
   `parenting.ts`, group and ungroup in `layer-groups.ts`) see new state.
3. **Edit kernels.** Pose preservation, group and ungroup, align and distribute,
   anchor moves with font anchors, and composition bounds move into `pm-edit`,
   returning patches the TS edit pipeline applies. Undo is unchanged.
4. **Shadow-replay auditor.** In test builds, the patch stream sent to `pm-wasm`
   is also applied to a JS shadow copy with `applyPatch`. At frame boundaries, a
   sorted-key serialization of the shadow must equal a sorted-key serialization
   of `PM.Comps.canonicalize(PM.proj)`. A mismatch fails the test with the last
   writer's stack. The e2e suite runs with the auditor on.
5. **Dual-answer mode.** In Phase 1 tests, every synchronous query is answered by
   both the TS evaluator and `pm-wasm` and compared with the section 25
   tolerances; text queries require identical line breaks and positions within
   0.25 px.
6. **Resync.** A revision gap in production triggers a canonical snapshot.

Prior art: `renderer/src/host/remote-sync.ts` catches non-history writes by
diffing; `server/sessions.ts` keeps a patch-fed mirror through `shared/patch.ts`.

### Engine-originated facts

The engine and main learn facts the document may need. They arrive as
`engine:proposeEdit` events and the renderer commits them:

| Fact | How it is committed |
|---|---|
| Probe results, measured duration, detected color and alpha | Not committed; read from the media index |
| Automatic interpretation | Not committed; overrides only are stored |
| Relink to a found file (fingerprint match) | Metadata write, not undoable, recorded in the edit log |
| Relink chosen by the user, Replace File…, Attach Camera Proxy… | Undoable user edit |
| Font substitution for a missing font | Not committed; shown as a warning until the user chooses |
| Asset marked failed after crash attribution | Session state only |

## 7. Evaluation (`pm-eval`)

A pure function of (document, composition, time) to evaluated state. It replaces
`anim.ts`, `temporal-bridge.ts`, `temporal-ease.ts`, `easing.ts`,
`expression.ts`, `retiming.ts`, and the evaluation parts of `parenting.ts`,
`layer-groups.ts`, `space-3d.ts`, `content-properties.ts`, text animator state,
and path and mask geometry.

- **Pure.** No global composition, time, or `PM.scope` stack.
- **Incremental.** Caches keyed by node id, input revision, and tick; patches
  invalidate a node and its dependents (parents, children, matte users,
  expression readers).
- **Expression dependencies** recorded per expression, so tracking is exact even
  though expressions read raw project fields.
- **Temporal model** ported exactly: speed and influence, auto-bezier,
  continuous, hold, spring parameters, legacy easing presets.
- **JavaScript semantics** where projects depend on them: loose `==`, string
  coercion in `+`, `Math.round` rounding half toward +∞, `%` on negatives, the
  `sin`-based `random` and `wiggle`, the float32 spring table, hex color byte
  interpolation, today's epsilons. Each has a test against V8.
- **Determinism.** Every transcendental function uses the `libm` crate on every
  target.
- **Geometry queries:** `pick`, `bounds`, `worldMatrix`, `localMatrix`, group and
  composition bounds, from the geometry the renderer uses, including text bounds.
- **JS fast path.** Hot queries use numeric ids interned per property and write
  results into a shared `Float64Array`. No strings or objects cross per call.
  Views are re-acquired after WASM memory grows.

## 8. GPU and the render graph (`pm-render`)

### Device

- wgpu (30.x today), Metal on macOS, DX12 on Windows, Vulkan for Linux CLI
  renders. Pinned; upgrades are lanes, budgeted for roughly quarterly majors.
- One `pm-gpu` interop trait owns decoder output import, shared-surface export,
  and encoder input, through wgpu-hal.
- The engine's adapter matches Chromium's GPU process adapter (Metal
  `registryID`, or LUID on Windows).
- **Metal math mode.** Color-critical shaders (input, output, display transforms,
  LUTs) are compiled with precise math through passthrough shaders, or wgpu is
  patched. Decided in Phase 0.
- **Watchdog and device loss.** A frame that exceeds a time limit is cancelled;
  the offending effect is disabled for the session and reported. Device loss
  recreates the device and resumes without replacing the document.

### Working encoding and precision

| Blending | Intermediate encoding |
|---|---|
| Standard (linear blending off) | Premultiplied, display encoding E (section 9), Rec.709 primaries |
| Linear blending on | Premultiplied, linear light, Rec.709 primaries |
| After cutover: wide gamut, HDR, ACES | Premultiplied, linear light in the chosen working space; linear blending forced on |

Blending, mipmap generation, resampling, and motion-blur accumulation use the
active encoding.

Storage is `rgba16float` by default and `rgba32float` at 32-bit precision
(requires `FLOAT32_FILTERABLE` and float32 blending or shader-side blending;
verified on the reference machines in Phase 0). Source conversion shaders
compute in f32. Exports default to 32-bit when the output or any source exceeds
10 bits; preview uses the project precision. A difference between preview and
export precision is stated in the export sheet.

### Render graph

- **Nodes:** source (decoded frame, vector, text, solid, shader output),
  transform and resample, mask, effect pass, matte, blend, transition,
  motion-blur accumulate, precomp, adjustment, color transform, output.
- **Content keys** hash every quality-affecting input: evaluated parameters,
  source frame identity (asset, representation, source frame), color transforms,
  resolution bucket, resampler, motion-blur sample count, preview-quality state,
  precision, and the engine build hash. Time enters only through evaluated values
  and `u_time`, so a static layer renders once for a whole range.
- **Cache granularity.** Layer and precomp outputs are cached on their full
  extents. Region-of-interest cropping applies to the final output only.
- **Continuous rasterization for precomps.** A precomp renders at the maximum
  effective scale its parent chain requires at that frame, rounded up to a
  bucket, and is never upsampled. (Today precomps render at their own size, so a
  vector precomp scaled up is soft; this is a known improvement.) A "Collapse
  transformations" switch follows cutover.
- **Region of interest.** Each node knows its domain of definition and the region
  downstream needs, including effect padding (`viewportPadding`). Bounds-expanding
  effects grow their buffers up to a limit instead of clipping.
- **Transient resources** from a per-frame aliasing allocator.
- **Isolation.** A failing node marks its layer failed and outputs transparency.

### Sampling and antialiasing

- Downscaling: mipmapped trilinear during playback; a high-quality resampler
  (bicubic, or Lanczos with clamped negative lobes) for export and paused frames.
  Mipmaps are generated in the active encoding.
- Analytic coverage antialiasing on quad edges (today: hard UV cut,
  `antialias: false`).
- Vectors and text rendered at output resolution.

### Motion blur

- Temporal supersampling with one project setting for sample count (Settings ›
  Project › Composition), used by every path. Today it is 1, 6, 10, 12, 16, or 20
  depending on the path (appendix A).
- Accumulation in the active encoding at float precision.
- No adaptive sample reduction unless proven output-identical.

### 2.5D

Parity: the 3D switch, Z position, anchor and scale, X and Y rotation,
orientation, perspective per 3D group, homography planes, coplanar ordering,
painter sort. After cutover: depth-buffered intersections, cameras, lights.

### Scopes

An internal waveform and vectorscope tool exists from Phase 3a for the QA lane.
Scopes as a product feature follow cutover. Scopes always read a full-precision
working render of the current frame, never bridge pixels, and say when the
viewer is showing a proxy or reduced resolution.

## 9. Color pipeline

### The rule

sRGB and Rec.709 share primaries. By default they also share one **display
encoding E**, the sRGB piecewise curve. Code values in E pass through unchanged
in every configuration:

- **Standard** (linear blending off): intermediates are in E. Hex colors,
  pass-through footage, and today's look hold exactly.
- **Linear blending on:** inputs in E are decoded with E⁻¹, blended in linear
  light, and encoded with E on output. Pass-through and hex colors still hold
  (bars 1 and 2 in section 2), because decode and encode use the same curve.
- **Interpret** offers other readings of Rec.709 sources: "BT.1886 (gamma 2.4)"
  and "Scene-referred (inverse BT.709 OETF)".
- **Viewer:** working values are encoded with E and handed to the WebGPU canvas
  in `srgb` or `display-p3`; ColorSync maps them to the display. One display
  transform, specified per canvas color space.
- **QuickTime.** QuickTime and AVFoundation show 1-1-1 tagged files lighter than
  most other players. Export offers "Rec.709 (broadcast)" with tags 1-1-1 and
  "Rec.709 (QuickTime and Safari)" whose tags make QuickTime match the viewer.
  Spike S2 determines the exact tags; bar (3) verifies them.

### Alpha

- Straight sources: input transform, then premultiply.
- Sources premultiplied in encoded space: unpremultiply (guarded for α < 1/4096
  at half float), input transform, re-premultiply.
- "Premultiplied with color" interpretation (matted against white or black).
- Color-domain effects (Levels, Curves, Hue/Saturation, legacy GLSL in display
  domain) run on unpremultiplied values. Additive pixels (α = 0, RGB > 0) are never
  unpremultiplied.
- Output: unpremultiply, output transform, then write straight, or re-premultiply
  the encoded values for premultiplied outputs (the convention of ProRes, HEVC
  with alpha, and PNG consumers).

### Blend modes and mattes

Normal, add, multiply, lighten, and darken run in the active encoding. With linear
blending on, screen, overlay, and soft light use HDR-safe formulas by default; a
per-project option runs them in E instead, which matches the look designers know
from display-referred tools. Luma mattes use Rec.709 coefficients on E-encoded
values by default.

### Project color settings

Settings › Project › Color, built from the existing Settings rows and switches.

| Setting | Options | Default | Phase |
|---|---|---|---|
| Blend in linear light | Switch | Off (decision 1) | 3a |
| Precision | 16-bit float, 32-bit float | 16-bit float | 3a |
| Color | Standard (sRGB and Rec.709), Wide gamut (Display P3), HDR (Rec.2020 PQ), HDR (Rec.2020 HLG), ACES | Standard | Options other than Standard after cutover |
| Advanced › Working space | Linear Rec.709, Linear Display P3, Linear Rec.2020, ACEScg | Follows Color | After cutover |
| Advanced › Graphics white in HDR | 203 nits (BT.2408), custom | 203 nits | After cutover |

Changing a color setting changes the look and is one Undo step. Projects migrated
from today open with Standard color and linear blending off. The display choice
(Automatic, sRGB, Display P3) is a machine setting in Settings › General ›
Display.

### Per-source interpretation

"Interpret Footage…" on an asset or a multi-selection, as a Settings-style sheet.
Values are filled from metadata and show their origin, for example "Automatic
(Rec.709) · From file" or "Assumed · No color tags in file".

Before cutover (Phase 3b):

- Color: primaries, transfer (E, BT.1886, scene-referred), matrix, range.
- HDR sources (iPhone HLG, HDR10, Dolby Vision base layer) in Standard projects
  are tone mapped to SDR by default with BT.2446 Method A: "HDR (HLG) · Tone mapped
  to SDR". Today Chromium does this implicitly; the engine does it explicitly.
- Alpha: none, straight, premultiplied, premultiplied with color; invert.
- Frame rate override, field order, pixel aspect ratio.
- Untagged sources: SD (480 or 576 lines) assumes BT.601 with SMPTE-C or BT.470BG
  primaries; HD and above assume BT.709. Stills use their ICC profile through
  ColorSync.
- Superwhites and sub-blacks are preserved; clamping happens only at output.

After cutover: camera log curves with gamuts (S-Log3/S-Gamut3 and .Cine,
LogC3/AWG3, LogC4/AWG4, V-Log, C-Log2/3, Log3G10/RWG, BMD Film Gen5, DaVinci
Intermediate, F-Log/F-Log2, N-Log, D-Log, Apple Log) with "Convert to Rec.709 /
Show as log", input LUTs, camera rules, and "Show as HDR values".

### After cutover: professional color

- OpenColorIO (2.6 at time of writing) through a thin `cxx` bridge with its
  built-in ACES 2.0 configurations; graphics default to the "Graphics" input (the
  inverse output transform) so picked colors display as picked; OCIO GPU shaders
  emitted as MSL where supported, checked against OCIO's CPU path; tetrahedral 3D
  LUT interpolation.
- Wide-gamut and HDR projects; output gamut mapping (clip, or ACES reference gamut
  compression); negative components allowed in intermediates.
- LUTs (`.cube`, `.3dl`, `.csp`, CLF/CTF) and color effects with a selectable
  domain (working, ACEScct log, display): Apply LUT, Exposure, Curves,
  Lift/Gamma/Gain, Hue curves, White Balance, Color Space Transform, Tone Map.
- Gradients interpolating in Display (today), Linear, or OKLab.
- Legalizer, gamut warning, false color, zebras, scopes in code values, IRE, nits.
- HDR preview: the WebGPU canvas in `rgba16float` with extended tone mapping;
  EDR headroom from native screen APIs through main. Prior art:
  `renderer/public/onboarding/hdr-output.js:93-132`.
- HDR export: PQ with mastering metadata and measured MaxCLL and MaxFALL; HLG with
  optional metadata; PQ 1.0 = 203 nits by default.
- HDR stills with gain maps (HEIC, ISO 21496-1 JPEG).
- A native reference viewer window (`CAMetalLayer`, extended dynamic range).

### Output tags

| Output | CICP (primaries-transfer-matrix) | Range |
|---|---|---|
| sRGB images | 1-13-0, or ICC | Full |
| Rec.709 (broadcast) | 1-1-1 | Limited for video, full for images |
| Rec.709 (QuickTime and Safari) | Determined in S2 | Limited |
| After cutover: Display P3 video | 12-13-1 | Limited |
| After cutover: Rec.2020 SDR, PQ, HLG | 9-1-9, 9-16-9, 9-18-9 | Limited |

ProRes Y′CbCr is limited range with full-range alpha. MOV and MP4 write `colr`
`nclx` when full range must be signaled; WebM writes Colour elements; PNG writes
`cICP` or ICC.

## 10. Layer types

| Type | Today | Engine |
|---|---|---|
| `solid` | Uniform quad; `d.radius` stored but ignored | Same; `radius` honored only on `compat` level 2 layers |
| `shape` primitives and paths | Canvas2D raster at quantized scale | `pm-vector` geometry at output resolution, analytic coverage (renderer chosen in S4) |
| `text` | Canvas2D `fillText` | `pm-text` (section 12) |
| `captions` | Canvas2D (`captions/render.ts`) | `pm-text` and `pm-vector`; word states in `pm-eval` |
| `image` | `ImageBitmap`, RGBA8 | Native-depth readers |
| `image` (SVG) | `<img>` per size bucket | usvg (with its text feature off) to vectors; SVG text through `pm-text`; filters through resvg on CPU |
| `video` | `<video>` or VP9 copy | `pm-media` decode of the chosen representation |
| `shader` | User GLSL, FBO capped at 4,096 px | GLSL compatibility pipeline; device limits only |
| `extension` fragment | Kernel-assembled GLSL | Same pipeline |
| `extension` mesh | Fixed WebGL shading | Same look; physically based shading after cutover |
| `precomp` | Recursive render at its own size, depth 6 (frame preparation allows 8) | Subgraph at effective scale (section 8), cached; one depth limit and cycle check |
| `adjustment` | Effects on the accumulator | Same |
| `group` | Transform space or compositing boundary | Same |
| `null`, `audio` | Not visual | Not visual |

Animated images and image sequences are decoded directly at original timing and
depth.

## 11. Effects, transitions, and the shader contract

### Built-in effects

The 16 effects, 4 transitions, 5 shader presets, and 2 extension layers are
re-implemented in WGSL. Gaussian Blur has a correct kernel at every radius with a
downsample pyramid for large radii (today taps are spaced `r/N` apart); Glow and
Drop Shadow use it. Each declares `colorDomain` and bounds expansion. In Standard
color they reproduce today's output within parity tolerances, except fixes listed
in appendix A.

### The GLSL contract stays

Projects, Store extensions, agent-written extensions, and web exports carry
GLSL ES 3.00 with Powermove's preambles, `u_<param>` uniforms, and the `u_p<i>`
and `u_c<i>` aliases.

1. **Wrap** with the existing generators (`kernel/glsl.ts`,
   `extension-layers.ts`).
2. **Normalize:** `#version 300 es` → `#version 450`; remove precision
   qualifiers; inject `pm_FragCoord` built from a resolution uniform in place of
   `gl_FragCoord`.
3. **Compile** with glslang using relaxed Vulkan rules (loose uniforms into one
   uniform block) and automatic binding and location mapping.
4. **Split combined samplers** (including `sampler2D` function parameters) at the
   SPIR-V level; naga's SPIR-V frontend maps `OpTypeSampledImage` to the image
   only.
5. **Translate:** parse SPIR-V with naga ourselves and validate with
   derivative-uniformity checks off, passed as `ShaderSource::Naga`, because
   Shadertoy-style shaders call `texture()` in data-dependent control flow that
   WebGL accepts. S3 compares this path with SPIRV-Cross to MSL through
   passthrough shaders and picks the more robust one.
6. **Cache** by source hash, backend, and driver version.
7. **Errors** map back to the user's line numbers.

**Origin and sampling.** Legacy shaders see GL conventions (`v_uv`, `v_st`,
`pm_FragCoord`, `dFdy` as in WebGL, bottom-left origin). Sampler state used by
the WebGL host per effect type is recorded and reproduced.

**Color domain.** Legacy GLSL is display-referred: it receives E-encoded values,
independent of the user's display, so output is identical on every screen and in
export. Values above 1.0 pass through by default (today's intermediates are
RGBA16F); an effect may declare `clamp: true`.

### Definition snapshots

Effects, transitions, and extension layers are registered at runtime by
JavaScript; projects store only definition ids. The engine needs definitions
without JavaScript (job engine, `pm-cli`):

- The kernel pushes a **definition snapshot** `{id, version, glsl, params, flags}`
  for every registered effect, transition, and extension layer to each session.
- Export jobs and Collect Project embed the snapshots of the definitions they use.
- `pm-cli` renders from embedded snapshots, or fails with "Extension X isn't
  available" naming the extension.
- The `raster` service override produces pixels in JavaScript. Projects that use
  it export through the interactive engine with a renderer round trip and are
  marked as not headless-renderable. At apiVersion 4 the override is deprecated in
  favor of extension layers.

### Contract v2

A WGSL contract for new extensions (apiVersion 4): explicit `colorDomain`, bounds
expansion, multiple inputs, compute passes. GLSL stays supported.

## 12. Text

- **Stack.** Parley (layout, bidi, line breaking, variable fonts, emoji,
  fallback), HarfRust (shaping), Skrifa (outlines, metrics). RustSec advisories on
  `rustybuzz` and `ttf-parser` rule those out. Glyph rendering is chosen in S4.
- **Font registry.** One native authority enumerates system fonts (CoreText,
  DirectWrite), loads project fonts, resolves fallback, and ships a fallback
  manifest. Before Phase 2 it runs in main through `pm-bridge`; from Phase 2 in
  the engine. The UI font picker reads it.
- **Fonts in `pm-wasm`.** The sandboxed renderer cannot read font files, and full
  fonts are large (Apple Color Emoji is about 200 MB of bitmaps). The registry
  produces metrics-and-shaping subsets per font: `cmap`, `hmtx`, `hhea`, `OS/2`,
  `head`, `name`, `GSUB`, `GPOS`, `GDEF`, `kern`, `morx`, `kerx`, `trak`, `fvar`,
  `avar`, `HVAR`, `MVAR` (and a synthesized `HVAR` for variable fonts that lack
  one); no outlines or bitmaps. Main serves them lazily per family within a
  per-window memory budget.
- **Compatibility (level 1 layers).** Today: greedy word-then-character wrapping
  when `paragraph` is set and `boxWidth > 0` (`text-layout.ts:22`), character
  splits by UTF-16 unit, baseline at `size × 0.82` (`raster.ts:210,260,339`),
  line height `size × leading` (default 1.15), kerning through measured widths,
  CoreText's `trak` tables and automatic optical sizing for SF fonts, and hidden
  system fonts. Level 1 reproduces these. Gate: identical line breaks and glyph
  advances within 0.25 px against today. Level 2 (new layers) uses
  grapheme-aware breaking and font metrics.
- **Animators.** Stagger, range, wave on characters, words, lines; per-glyph
  opacity, position, scale, rotation, skew, tracking, blur, color. The 13
  inspector presets stay UI-side.
- **Editing.** Caret, selection, and the font offset the editor reads today
  (`viewer/text-editor.ts:102`) come from `pm-wasm`. Native and WASM layout are
  identical (tested).
- **Rendering quality.** Canvas2D draws glyphs through CoreText with dilation;
  plain path coverage looks thinner. A coverage gamma or stem darkening is tuned
  to match CoreText at text sizes.
- **Gates.** A side-by-side corpus against CoreText at 9 to 400 px, light and
  dark, reviewed by Jude; SSIM ≥ 0.98 against today for parity cases; canvas
  typing latency (section 25).

## 13. Media I/O (`pm-media`)

### Principles

1. The original is the truth. Import never replaces it.
2. Every source is indexed for exact, fast random access.
3. Frames reach the GPU at native depth with color metadata.

### Backends

- **VideoToolbox** (objc2 crates) for ProRes, H.264, HEVC, and AV1 where hardware
  exists. Requested outputs:
  - ProRes 422 → `x422`; ProRes 4444 and XQ → `y416` (12-bit color, 16-bit alpha)
  - HEVC and H.264 10-bit → `x420` (4:2:0) or `x422` (4:2:2)
  - 8-bit → biplanar 8-bit
- **Metal import.** wgpu's NV12 and P010 formats are unavailable on Metal.
  Biplanar buffers import as per-plane `R8`/`RG8` or `R16Unorm`/`RG16Unorm`
  textures through `CVMetalTextureCache`; `y416` imports as one `RGBA16Unorm`
  texture with an AYCbCr swizzle. For `x420` and `x422` only, MSB-aligned samples
  are normalized (`v >> 6`) before limited-range scaling: Y′ = (c − 64)/876,
  C = (c − 512)/896. Untagged 4:2:0 assumes left chroma siting.
- **FFmpeg** (our build, section 24) through `ffmpeg-the-third` or our own bindgen
  crate, linked as dylibs: demuxing, DNxHD and DNxHR, CineForm, MJPEG, DV, VP8 and
  VP9 (libvpx for alpha), AV1 (dav1d), DPX, software fallbacks, and all audio
  decoding.
- **Images.** ImageIO on macOS (HEIF, AVIF, TIFF, ICC); the `image` crate; OpenEXR's
  C library; usvg for SVG.

### Formats at cutover

| Category | Formats |
|---|---|
| Intermediates | ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ; DNxHD; DNxHR; CineForm |
| Camera and delivery | H.264 (including 4:2:2 10-bit, software-only), HEVC 8 and 10-bit (4:2:0, 4:2:2), HEVC with alpha, AV1, MJPEG, DV |
| Web | VP8, VP9 with alpha |
| Containers | MOV, MP4, M4V, MXF (OP1a, OP-Atom), MKV, WebM, AVI, WMV/ASF, FLV, MPEG-PS, TS, MTS/M2TS, 3GP, OGV, VOB (today's list) |
| Images | PNG 8/16, JPEG, TIFF 8/16/32f, OpenEXR, DPX, HEIF/HEIC, AVIF, WebP, BMP, ICO, GIF, SVG |
| Sequences | Any image format, gaps reported |
| Animated images | GIF, APNG, animated WebP and AVIF |
| Audio | WAV, AIFF, CAF, MP3, AAC/M4A, FLAC, Opus, Vorbis, WebA, audio in every container; any channel layout |

After cutover: camera RAW, PSD layers, multi-part EXR.

Hardware notes for the proxy policy: base M1 has no ProRes engine (M1 Pro, Max,
Ultra and M2 and later do); AV1 hardware decode starts at M3; H.264 4:2:2 10-bit
is software-only.

### Probe and index

In the background on import:

1. Streams: codec, profile, size, pixel format and depth, rate and whether
   variable, duration, timecode, reel, UMID or content identifier, rotation, pixel
   aspect, field order, audio layout.
2. Color: primaries, transfer, matrix, range, mastering metadata, ICC, HDR format.
3. **Frame index** from container tables (MOV/MP4 `stss` and `stts`, MXF index
   tables, MKV cues); scanning only where none exist (MPEG-TS). Open-GOP CRA and
   RASL frames are marked not cleanly seekable.
4. **Timing:** edit lists, composition offsets and negative CTS, stream start
   offsets, AAC and MP3 priming.
5. **Content hash.** A full BLAKE3 hash computed in the background. The sampled
   fingerprint used today (size plus three 64 KiB samples, `media.ts:153-171`,
   which its own comments say can collide) remains a lookup hint only. Files still
   being written are detected and wait.
6. **Decode cost** on this machine.
7. Poster, filmstrip seed, waveform peaks.

### Variable frame rate and interlacing

- VFR sources are sampled by timestamp: the frame at source time t is the last
  frame with PTS ≤ t. Duration and sync come from timestamps. A nominal rate
  (the most common frame duration snapped to a standard rate) is used for
  snapping and stepping only. Generated media preserves the original PTS exactly,
  so switching representations never changes the selected frame.
- Interlaced sources are deinterlaced (bwdif) by default, field order from
  metadata or Interpret. Pulldown removal follows cutover.

### Decode service

Sessions per source and playback instance, pooled and budgeted; hardware output
imported without CPU copies; scheduling by direction, speed, read-ahead, and GOP
structure; decoded frames in the frame cache.

## 14. Proxies, optimized media, and caches

### 14.1 What users should feel

Footage always plays without setup. Nobody needs to know what a proxy is.
Professionals find every control After Effects and Final Cut Pro offer, in plain
language. The viewer always says what is on screen. Exports never contain proxy
pixels by accident. Controls and copy are in appendix D.

### 14.2 The representation ladder

From no generation to most generation:

1. **Original** at full resolution.
2. **Reduced-resolution decode** with no files: ProRes partial-resolution decode,
   ImageIO downsampling, EXR mips. Tried before generating anything.
3. **Proxy segments**, reduced resolution.
4. **Optimized segments**, full resolution, intra-frame, for long-GOP sources.
5. **Kept renders**, pinned precomp renders (section 14.6).

Also covered: image sequences (EXR and DPX sequences get proxy segments) and
large stills (100-megapixel TIFF or EXR get a tiled mip cache, no proxy file).

### 14.3 Segments, addressed by content

- Generated media is stored as segments of about 2 seconds aligned to source
  keyframes, each a QuickTime file, with one segment index per source.
- Keyed by (content hash, recipe, segment index). The content hash is the full
  BLAKE3 hash; segments record the source's size, modification time, and inode
  and are re-verified on use.
- Only used ranges plus 2-second handles are generated; trimming outward generates
  only new segments. Jobs resume after relaunch. Availability is per frame.
  Identical footage in two projects is generated once. Atomic writes make a shared
  volume safe for several Macs.
- Playback across segment boundaries is seamless (gate: zero drops across
  boundaries with 4 streams).

### 14.4 Recipes and fidelity

| Purpose | macOS recipe |
|---|---|
| Proxy | ProRes 422 Proxy (10-bit) by default; ProRes 422 LT or HEVC 10-bit by choice. Sources with alpha always get ProRes 4444 at proxy size |
| Optimized | ProRes 422 HQ; ProRes 4444 for sources that are 4:4:4, above 10-bit, or have alpha. Created only when asked by default (decision 4) |

Every generated representation keeps the source encoding (log stays log, HDR
stays HDR), is 10-bit (8-bit H.264 only for SDR non-log sources by explicit
choice), and keeps alpha, PCM audio, timecode, reel name, pixel aspect, frame
count, and exact PTS. It is sampled in the original's coordinate space, so layer
geometry is unchanged. Analysis (tracking, transcription, agent media tools)
always uses originals.

### 14.5 The automatic policy

A **machine profile** measures decode cost per codec class and size, GPU headroom,
disk throughput, and power state (battery, Low Power Mode, thermal state). The
project stores only intent; the same project uses originals on a Mac Studio and
proxies on a MacBook Air.

1. Choose the best representation that sustains real time at the viewer's size in
   device pixels for the number of simultaneous streams.
2. During playback, switch only at clip boundaries or when playback starts, never
   mid-shot.
3. When paused with Resolution set to Automatic, show the original at full quality
   as soon as it decodes, in place, without a fade. A Resolution the user set by
   hand (Half, Quarter) is respected when paused.
4. Never wait for generated media; use the best ready representation.
5. Re-evaluate when the power state changes.

Generation runs in the job engine at low priority, pauses during playback and
export, and goes clip-near-playhead first. "Create proxies: When needed" (the
default) generates proxies only when the policy predicts a source cannot play in
real time. A **disk guard** pauses generation when free space falls below the
larger of 20 GB or 10% of the volume.

### 14.6 Render cache and kept renders

- Node outputs (section 8 keys) are cached in GPU memory and on disk. The disk
  format is half float with lossless compression per tile.
- A 2-pixel track under the timeline ruler shows cached frames in a quiet ink
  tone: lighter for memory, stronger for disk. View › Show Rendered Frames turns it
  off.
- Render Work Area, Render Selection, Delete Rendered Frames.
- Keep Rendered on a precomp pins a render that is invalidated on edit.
- Background rendering fills the disk cache while idle (default: when plugged in).
- **Export safety.** Export reads only cache entries whose keys match its own
  quality inputs (original representation, export resampler, export motion-blur
  samples, export precision, full quality, same engine build). Gate: fill the
  cache by playing at Automatic, Half, and Quarter, with proxies, during a
  sustained-load drop; then export; the output is byte-identical to an export with
  an empty cache.

### 14.7 Caches

| Cache | Contents | Key | Location | Eviction |
|---|---|---|---|---|
| Frame cache | Decoded frames, GPU and CPU tiers | Source, representation, frame | Memory | Lease, LRU, protected near the playhead |
| Render cache | Node outputs | Section 8 key | GPU memory, disk | Lease and size limit, LRU |
| Media index | Frame indexes, probe, content hash, decode cost | Content hash and file identity | App cache | Rebuilt on demand |
| Thumbnails, filmstrips, posters, waveforms | Derived images and peaks | Content hash, recipe | App cache | Size limit |
| Generated media | Proxy and optimized segments | Content hash, recipe, segment | Location setting | Storage limit; pinned items never |
| Large-still mips | Tiled mip levels | Content hash | App cache | Size limit |
| Shader cache | Compiled pipelines | Source hash, backend, driver | App cache | Version change |
| Font subsets | Subsets for `pm-wasm` | Font file hash | App cache | Size limit |

These replace today's IndexedDB media store (media, posters, 1280-pixel editing
copies), the editing-copy queue, the decoded audio cache, raster and texture
caches, and the GL program cache. Today's `memory.ts` budgets and the
`previewMemoryMiB` setting map to engine leases (section 19).

### 14.8 Offline media and exports

- Interactive export with offline originals uses a native three-button message:
  Cancel, Export with Proxies, Locate Media… (copy in appendix D).
- Queued jobs never fall back to proxies; they wait with "Waiting for media · 2
  clips offline".
- Draft renders from proxies are an explicit queue option, with an optional
  "Draft" mark, reported in verification.

## 15. Playback

- **Master clock:** the audio device clock, including device latency (Core Audio
  latency plus safety offset; Bluetooth adds 150 to 250 ms). A monotonic clock
  when nothing plays.
- **Render-ahead** into an adaptive ring (default 4); decoders read ahead per GOP
  and speed.
- **Presentation.** Frames carry PTS and revision. The viewer keeps a jitter
  buffer and presents on `requestAnimationFrame` by PTS during playback. Scrub and
  gesture frames bypass it.
- **A/V sync.** Presentation latency is measured (photodiode loopback in the lab,
  frame-callback timing in the app; Chromium exposes no present timestamp for
  WebGPU canvases) and audio is offset. Gate: ±10 ms.
- **Drops** are counted, attributed (decode, render, bridge), and reported.
- **Quality ladder.** Under sustained load, Automatic lowers preview resolution
  (down to quarter) and moves clips down the representation ladder.
- **Scrubbing.** Paused seeks coalesce; a complete frame is shown before chasing
  the next.
- **Transport.** Work-area loop with pre-rolled entrances, frame stepping. New
  before cutover: J/K/L at 1× and 2× forward and reverse, play around current
  frame, audio scrub. After cutover: faster shuttle and reverse audio playback.
- **Cached preview.** Render Work Area renders into the render cache and plays at
  full rate with audio.

## 16. Audio

- **Engine.** f32 mixing at the project rate (Settings › Project › Composition:
  48 kHz default, 44.1, 96), resampling with `rubato`, output through `cpal`.
- **Decode** through FFmpeg only (symphonia lacks Opus and HE-AAC), streaming, no
  length cap, priming honored.
- **Graph.** Clip voices → per-layer strip (gain, mute, solo, meters) → precomp
  nesting (gains multiply) → master. Channel layouts preserved and down-mixed to
  the output layout.
- **Automation** per sample.
- **Speed and remapping.** Audio follows speed and remaps; pitch preserved by
  default with Signalsmith Stretch (our own thin binding to upstream), or follows
  speed per clip. Defined pre-roll makes a seek produce the same samples as
  continuous playback.
- **Frame-rate override** keeps audio in sync by resampling or stretching, chosen
  per asset.
- **Waveforms** computed by the engine at several resolutions, cached, drawn by the
  timeline, Assets panel, and `media.audio.drawWaveform`.
- **Mixer** meters, solo, and retune from the engine at display rate.
- **Export.** Offline, sample-accurate. PCM 24-bit or 32-bit float for ProRes and
  WAV; AAC through AudioToolbox for MP4 with `edts` and priming (2112 samples) so
  audio is not late; TPDF dither when reducing to 16-bit. Stereo output as today.
- **After cutover:** reverse playback, channel mapping on export (discrete mono,
  stems, 5.1 labels), pan, EQ, compressor, limiter, loudness normalization.

## 17. Export and delivery

### Pipeline

```
evaluate → render (float) → output transform → encoder input conversion
  (engine shader: matrix, chroma downsampling and siting, dither) → encode
  → mux → verify → move into place
```

- Runs in the job engine from a document snapshot with definition snapshots
  (section 11). Users keep editing.
- **Pipelined stages** with bounded queues: decode, evaluate, render, convert,
  encode on different frames concurrently.
- **Encoder input.** The engine converts RGB to Y′CbCr itself and hands
  VideoToolbox native buffers (`y416`, `x422`, `v210`, `x420`) with CVPixelBuffer
  color attachments set; those become the `colr` and VUI tags.
- **Dither** (TPDF or blue noise seeded by frame, x, y) in the encoded domain,
  only when the working precision exceeds the output precision and the output is
  10 bits or fewer; never on alpha. Color-invariant and transparency tests run
  with dither off.
- **Muxers.** AVAssetWriter for MOV and MP4 (`colr`, `tmcd`, alpha); FFmpeg for
  WebM. NTSC timescales are 24000, 30000, or 60000.
- **Atomic output** via a temporary file in the destination folder.
- **Verification** probes codec, profile, pixel format, depth, tags, range, frame
  count, duration, audio samples, alpha, and pixel values on sample frames.
- **Comfort.** Prevent sleep; macOS notification on completion; estimated size
  with a warning if the destination lacks room; overwrite policy (Replace, Keep
  both); file name tokens; start timecode (default 01:00:00:00) and reel name.

### Formats at cutover

Today's list, from originals, at correct depth and tags:

| Format | Encoder | Notes |
|---|---|---|
| ProRes 422 Proxy, LT, 422, HQ; 4444; 4444 XQ | VideoToolbox | Today only 4444 is offered; the others come free with VideoToolbox. Alpha straight or premultiplied |
| H.264 (MP4) | VideoToolbox | Bitrate or quality target, profile, keyframe interval. A VMAF comparison against x264 at matched bitrate decides whether a software encoder is also offered, subject to licensing |
| HEVC (MP4, MOV) | VideoToolbox | 8 or 10-bit SDR; alpha at 8-bit (confirmed in S2) |
| WebM VP9 and VP8 | libvpx | Frame-exact; Colour elements written |
| PNG sequence and still | Engine | 8 or 16-bit, alpha |
| Code export, project export, captions sidecars and burn-in | Section 22, section 21, as today | — |

After cutover: DNxHR and DNxHD in MXF and MOV, AV1, EXR (including DWAA through
OpenEXR) and DPX sequences, TIFF sequences, GIF and animated WebP, audio-only,
HDR outputs, ACES deliverables, interlaced output, non-square-pixel compositions,
FCPXML and OTIO.

### Presets and the render queue

- Presets combine format, size, frame rate, color output, audio, range. Defaults:
  "Master · ProRes 422 HQ", "Master with alpha · ProRes 4444", "Web · H.264",
  "Web · HEVC", "Image sequence · PNG". Existing presets and per-project defaults
  migrate.
- **Render Queue panel.** Today a queue dialog exists, but nothing adds jobs, each
  job prompts for a destination, a failure stops the run, and the run swaps the
  live project. New: jobs are added from the export sheet or by agents with their
  destination; they run unattended in the job engine from snapshots without
  touching the live project; failures are per job and the queue continues; jobs
  persist across relaunch (main persists them).
- Export sheet and queue reuse the Settings sheet cards and rows. The export
  progress sheet keeps its live preview with small frames streamed from the job
  engine.

## 18. Viewer integration and the frame bridge

### The bridge

Proven by spike S1 before any viewer work.

1. **Rendezvous.** `pm-bridge` in main allocates a Mach receive right and
   registers it under a per-launch random bootstrap name (`bootstrap_check_in`,
   Chromium's pattern), passed to the engine through the `utilityProcess.fork`
   environment. Main checks each message's audit-token PID against the engine's
   `child.pid`. On Windows, main uses `DuplicateHandle` (Electron's NT handles are
   process-local).
2. **Pool, imported once.** The engine allocates IOSurface-backed textures at
   viewer device pixels. Pool size: maximum render-ahead ring + jitter-buffer
   depth + 2; growing the ring imports new surfaces once. Formats are limited to
   Electron's import formats: `bgra` (8-bit, with blue-noise dither in the display
   transform) or `rgbaf16`, chosen in S1 by banding and memory measurements (a 5K
   viewer surface is about 59 MB at `rgbaf16`). Each surface is sent once as an
   `IOSurfaceCreateMachPort` right and imported in main once with
   `IOSurfaceLookupFromMachPort` and `sharedTexture.importSharedTexture`. Pool
   memory is charged to the engine's lease.
3. **Per frame** the engine sends `{poolIndex, seq, pts, revision, colorSpace,
   overlayGeometry}` after the frame's command buffer completes
   (`on_submitted_work_done` or `MTLSharedEvent`). Main calls `sendSharedTexture`
   to the window that hosts the viewer (following panel popouts).
4. **Release per frame.** The viewer calls `VideoFrame.close()`, and the imported
   texture's per-use release callback (or release sync token, per Electron 44's
   typings) forwards `{poolIndex, seq}` to the engine, which then reuses the
   surface. `allReferencesReleased` is used only for pool teardown. S1 confirms
   the exact API.
5. **Renderer.** `setSharedTextureReceiver` runs in the preload's isolated world;
   S1 proves the `VideoFrame` reaches the viewer in the main world. The viewer
   draws it into its WebGPU canvas with `importExternalTexture`, or
   `copyExternalImageToTexture` into an `rgba16float` texture if needed.
6. **Overlays.** The DOM and Canvas2D overlay stays. `overlayGeometry` carries the
   world matrices, bounds, and paths of selected and hovered layers and the text
   caret at the frame's revision. During gestures the overlay draws from it, so
   handles never lead the pixels. Hit-testing uses `pm-wasm` at the current
   revision.
7. **Capture.** Frames are inside Chromium's compositor, so `capturePage`
   (`capture_panel`, `computer_use_panel`) keeps working.

S1 also measures main-process cost per frame and whether one imported texture can
be sent repeatedly.

### S1 gate

At 4K 60 fps on the reference machines, with main under load (autosave of the
largest corpus project, agent tool traffic, a 50 ms long task every second,
window resize):

- p99 presentation jitter at most 4 ms, zero drops over 60 s;
- layer drag pointer-to-pixel at most 33 ms at 60 Hz and no more than 1 frame worse
  than today;
- a ramp read back through `capturePage` matches the engine's output; a 10-bit ramp
  shows no visible banding (maximum run length test); values above 1.0 survive on
  an HDR display (informs the after-cutover HDR plan).

### Fallbacks, ranked by S1 data

- **A′.** Engine in main presenting through `CAMetalLayer` from a Rust render
  thread. Loses crash isolation; media parsing in the browser process.
- **C′.** Engine in a utility process with a remote layer (`CAContext` and
  `CALayerHost`, the private mechanism Chromium uses); overlays in the DOM above a
  transparent region; engine-side capture for agents.
- **B.** Shared-memory CPU frames at reduced preview resolution.

### Viewer behavior

- The viewer sends size, DPR, zoom, pan, and resolution; the engine renders only
  the visible region.
- During pan and zoom the last frame is reused scaled until the engine catches up.
- `frame:rendered` and `overlay` fire on presentation with the frame's revision.
  `anim.version()` comparisons with `frame:rendered.version` (`viewer.ts:747,916`)
  keep working because both report presented revisions.

## 19. Process model, protocol, security, packaging

### Processes

| Process | Runs |
|---|---|
| Main | Windows, files, save, agent bridge, media store, capability handles, budget arbiter, queue persistence, `pm-bridge` |
| Renderer, per window | UI, kernel, extensions, edit pipeline, `pm-wasm` |
| Interactive engine (utilityProcess) | GPU device, decoders, audio device, caches, sessions |
| Job engine (utilityProcess) | Exports, generated media, background renders |
| Extension sandboxes, transcription, agent CLIs | Unchanged |

Today no code uses `MessageChannelMain`; the transcription utility process talks
only to main. Main creates a `MessageChannelMain` per window and engine. Engine
ports are never given to extension sandbox frames.

### Memory

Main is the **budget arbiter**: it grants each engine leases from one global
budget (GPU and unified memory), shrinking the job engine's lease during playback.
Each engine enforces its lease across frame cache, render cache, surface pool, and
decoders. `pm-wasm` has a per-window budget. Gate: export plus playback on the
base 8 GB machine for 10 minutes without memory-pressure warnings.

### Crash handling

- Main restarts a crashed engine with exponential backoff; renderers re-send
  snapshots; the viewer shows "Reconnecting…". The document never lives only in
  the engine.
- **Attribution.** The engine records the active decode, shader, or job before
  risky work. Two crashes attributed to one asset or effect mark it failed for the
  session ("A001_C003.mov couldn't be decoded", with Replace File… and Reveal in
  Finder).
- Queue and generation jobs persist in main and resume.

### Protocol

- `pm-protocol` messages, MessagePack (`rmp-serde`, `msgpackr`), TypeScript types
  from `ts-rs`, checked in CI. Version negotiation at connect.
- `MessagePortMain` transfers ports only; data is copied. Bulk data (filmstrips,
  waveforms, scopes) goes through shared surfaces or cache files.
- Groups: session, transport, viewer, render, media, proxy, export and queue,
  audio, definitions, engine stats (including per-node and per-extension GPU
  timing).

### Security

- The engine accepts only capability handles minted by main (HMAC over
  `{assetId, path, sessionId}`), never raw paths.
- Decoders run in engine processes isolated from UI and main.
- Fuzzing: protocol decoder, patch ingest, project parsing, demuxers and readers.

### Packaging and CI

- Pinned Rust toolchain; `aarch64-apple-darwin` first.
- **Self-hosted Apple Silicon runners**: reference machines registered for parity
  and performance jobs only, isolated from secrets, corpus outside the repository.
- `pm-node` and `pm-bridge` are napi-rs addons in `asarUnpack`, signed with the
  Team ID and hardened runtime. `scripts/release-mac.mjs` verification is
  generalized (today `NATIVE_RUNTIME` lists the transcription runtime and
  `verifyRuntimeContract` assumes its first entry).
- FFmpeg, glslang, and OpenEXR built from source at pinned versions; FFmpeg as
  dylibs; `ffmpeg-static` removed.
- `pm-wasm` built with the wasm-bindgen CLI. The editor CSP gains
  `'wasm-unsafe-eval'` for the app origin (`main/security-policy.ts:9`), which
  `powermove serve` also serves (`server/index.ts:157`).
- Minimum macOS follows Electron 44's; newer-OS features are gated at runtime.
- **Crash and session reporting.** Opt-in, with a privacy note, needed for the
  cutover gate (decision 10). The repository has none today.

## 20. Kernel API, extensions, the agent harness

### API mapping

| API | After cutover |
|---|---|
| `project.get`, `latest`, `apply`, `time`, `play`, `undo` | Unchanged (seconds) |
| `project.snapshot(t, maxWidth)` | Engine render, same return type |
| `render.snapshot(time, maxWidth): string` (sync) | Served from an engine-filled snapshot cache; returns `''` while rendering (the type is frozen). Built-ins move to `project.snapshot` |
| `anim.ev`, `evP`, `active`, `worldMatrix`, `localMatrix`, `resolveContent`, `findProp`, `allProps`, `hasKeyAt`, `wouldCycle`, `transformParentMatrix` | `pm-wasm`, synchronous, same signatures |
| `anim.version()`, `anim.touch()` | Kept; `touch` feeds the write barrier; `version` reports the mirror revision |
| `anim.expressionErrors` | Kept; `pm-eval` errors mapped back to channel objects |
| `anim.setKey` and in-place helpers | Same signatures, through the write barrier |
| `model.*` | Unchanged |
| `groups.bounds`, `space3d.*` | `pm-wasm` |
| `transport.*` including `perf`, `invalidate`, `quality`, `previewResolution` | Engine transport and stats |
| `media.importFiles`, `assets.pick`, `import`, `importUrl`, `get`, `readText` | Engine import through main. `assets.get` still returns an object; live fields (`url`, `el`, `audioBlob`, `peaks`, `audioDecodeError`) become deprecated compatibility shims backed by engine data (a `media://` URL for formats Chromium can play, posters, peaks) |
| `media.timing.*`, `media.assets.kind`, `media.fonts` | Kept; fonts from the font registry |
| `media.audio.drawWaveform(ctx, …)` | Same, from engine peaks |
| `render.gl.bounds`, `pick`, `previewViewport` | `pm-wasm` |
| `render.gl.init(canvas)`, `resize` | Viewer-internal; mapped to the bridge |
| `render.gl.compileError` | Engine compiler, mirrored |
| `render.gl.context` | `null` with a deprecation warning; two built-in call sites rewritten |
| `render.raster()` | Metrics (`fontOffset`, bounds, `selection`) from `pm-wasm`; pixels deprecated |
| `render.renderFrameTo()`, `render.textLayout` | Async engine render; `pm-wasm` |
| `services`: `raster` override | Section 11, definition snapshots |
| `services`: `viewer`, `shaderHooks`, `compositions`, `captions` | Kept; GL calls redirected to the engine |
| `effects`, `transitions`, `layers.register` | GLSL contract unchanged; WGSL at apiVersion 4; definition snapshots pushed to sessions |
| `events` (all 15, including `frame:rendered`, `overlay`, `time`, `invalidate`, `project:changed`, `fonts`) | Same names; frame events on presentation |
| `PanelDefinition.library.render` | Kept; engine snapshot for the viewer card |

`EXTENSION_API_VERSION` moves to 4 (WGSL and async render APIs). It lives in
`packages/registry/src/manifest.ts`, shared with Powermove Cloud, so the bump is a
coordinated registry release. Extensions at apiVersion 1 to 3 keep loading. The
api-pack and `apps/desktop/docs/EXTENSIONS.md` are updated, including its existing
mismatch about effect varyings.

### Agent harness

- `render_frames`, composition contact sheets, and snapshots call the engine
  directly; no visible editor needed.
- `apply_commands` text-anchor edits and `get_project_state` text measurement use
  `pm-wasm`.
- `check_project` reads media status and peaks from the engine.
- `probe_media` gains color metadata, HDR format, and VFR status.
- `validate_effect` compiles through the engine's shader pipeline (today it checks
  structure only).
- New in Phase 3c: `export` with presets and the queue, `get_render_status`. New in
  Phase 4: `manage_proxies`.
- `sample_media_frames`, `media_waveform`, `transcribe_media` keep using originals;
  their retiming math (`legacy/assistant/media-source.ts`) moves to `pm-wasm`.
- `capture_panel` and `computer_use_panel` keep working.

Every other consumer of the engine is in appendix C.

## 21. Media management and project files

### Where bytes live

Today every asset's bytes are copied into the renderer's IndexedDB and embedded in
the `.pmv`. The engine cannot read IndexedDB. From Phase 2:

- **Media store in main.** A persistent store owned by main is the source of media
  bytes; the engine reads only through capability handles.
- **Two storage modes,** one rule each:
  - **Linked:** the file stays where it is; the project stores the `link`
    reference.
  - **Copied into project:** bytes are embedded in the `.pmv` (as today).
  - Items without a file (clipboard, web downloads, generated stills) are always
    copied into the project.
- **When importing** (Settings › Project › Media, from Phase 2): Link to files
  (default) or Copy into project.
- **Memory cards.** A file on removable camera media cannot be linked safely.
  Phase 2: such files are copied into the project and the import toast says so.
  After cutover: a "Copy from memory cards" option offloads them to a chosen
  folder with checksum verification and links them there.

### Relinking

- Automatic relink only by content hash, UMID, or content identifier.
- Matches by name, timecode, or duration are proposals in a Relink sheet that the
  user confirms (camera names like C0001.MP4 repeat on every card).
- Replace File… keeps the interpretation and offers to match scale.
- Offline clips show "Media offline · A001_C003.mov" in the viewer and a quiet
  "Offline" in the timeline clip label. Offline clips with proxies keep playing.

### Collect

**Collect Project…** (Phase 4) writes a folder with the project, originals, fonts,
and the definition snapshots of the extensions used. After cutover: LUTs and OCIO
configurations, "Trim media to used ranges", and **Package for Travel…** with proxy
segments.

### File format

- Desktop Save writes PMV4; Export › Project and the browser fallback write PMV3.
  Linked assets add records without bytes.
- Generated media is never stored in the project file.
- **`minReaderVersion` guard.** No build checks a version on load today, so an
  older build would "repair" new fields and autosave over them. Phase 0 ships a
  guard: a project above the build's version shows "This project needs a newer
  version of Powermove" and opens read-only.

### Switching engines safely

While the legacy engine can still be selected (the Phase 2 to 4 flag and the
Phase 5 switch release):

- The IndexedDB media is copied to the media store, never deleted, until legacy
  code is removed.
- A project that uses a native-only feature (linked media, kept non-Chromium
  originals, linear blending, interpretation overrides, `compat` level 2 layers) is
  stamped `requiresEngine: "native"`. The legacy engine refuses it with "This
  project uses the new engine. Turn on Use New Engine to open it."
- Gate (Phase 5): every corpus project without native-only features round-trips
  flag on → off → on with no loss; projects with them are refused cleanly.

### Migrating existing projects

- Assets whose original was replaced by a VP9 transcode show "Using a converted
  copy · Relink to the original for full quality" with Locate Original….
  Relinking by the stored fingerprint and `sourcePath` restores full quality.
- Embedded media moves from IndexedDB to the media store on first open with the new
  engine. 1280-pixel editing copies are deleted when legacy code is removed.

## 22. Web player, `powermove serve`, the headless document host, the CLI

### Web export and `packages/player`

`packages/player` is a build-only package over `renderer/src/player/*`, which
imports `legacy/core` and `legacy/gl`. It powers code export, the www hero (SVG
backend and `createEngine`), and a prebuilt copy vendored into onboarding
(`renderer/public/onboarding/player.js`).

1. **At cutover**, the TypeScript evaluator, rasterizer, and compositor move into
   `packages/player` as a frozen web player v1 for code export, www, onboarding,
   and serve (below). Code export keeps the v1 scene format. A parity test
   compares web player v1 with the engine in Standard color; onboarding's copy is
   rebuilt from it.
2. **After cutover**, a WASM build of `pm-eval`, `pm-text`, `pm-vector`, and
   `pm-render` on WebGPU replaces it for new exports (scene v2), decoding with
   WebCodecs. Old exports keep their v1 runtime.

### `powermove serve`

Paused today. Each tab runs the full TypeScript engine. Before Phase 5 removes the
legacy compositor, serve tabs switch to web player v1 for their preview
(decision 9). Later: host-side engine with streamed frames, or the WASM engine.

### The headless document host

`apps/desktop/src/engine/main.ts` runs `legacy/core` under happy-dom. In Phase 1 it
switches to `pm-wasm` for evaluation, text measurement, and edit kernels. That also
fixes two problems: `export_captions` and `generate_captions` are routed there but
`captions/install` is never loaded, and `editing.ts:859` calls `PM.raster`
unguarded on text anchor edits.

### The CLI

`pm-cli` renders a `.pmv` to any export format without Electron, from definition
snapshots, with presets, ranges, and parameter overrides (`setParameter` and
`setText` equivalents) for templates. It also hosts the parity runner (evaluation
in Phase 1, rendering from Phase 2). Users reach it through the existing
`powermove` command from `packages/cli` (`powermove render`).

## 23. Cross-platform

- **macOS:** Apple Silicon. Intel only if demand justifies it.
- **Windows (after cutover):** wgpu DX12, D3D11/D3D12 decode through FFmpeg,
  Media Foundation where needed, WASAPI, DirectWrite, NT-handle surfaces with LUID
  adapter matching. The Electron shell needs a Windows build, installer, signing,
  and a native-conventions review: a parallel product lane.
- **Linux:** `pm-cli` and serve, with Vulkan or a software fallback.
- Platform code behind traits in `pm-media`, `pm-gpu`, `pm-text`, `pm-audio`, each
  with a software fallback for tests.

## 24. Licensing and compliance

Settled with counsel before the dependent phase ships.

1. **FFmpeg.** Replace `ffmpeg-static`, a third-party build whose configuration we
   have not audited, with our own build: pinned version, LGPL or
   GPLv3-compatible components only, never `--enable-nonfree`, shipped as dylibs.
   It covers everything that uses FFmpeg today: export, the media-proxy converters
   (libvpx, libopus, PNG), and the agent media tools (`ebur128`, `silencedetect`,
   `astats`, `showwaves`, `tile`, `thumbnail`, MJPEG, `scale`, `fps`). Runs in
   parallel with Phase 0 and gates Phase 2.
2. **ProRes.** Apple licenses ProRes and names FFmpeg's implementation
   unauthorized; today's export uses FFmpeg's `prores_ks`. On macOS, ProRes decode
   and encode use VideoToolbox. On Windows, both need Apple's licensing program or
   are omitted.
3. **H.264 and HEVC encode.** Today's H.264 export uses libx264. Prefer OS
   encoders; confirm royalty exposure for any software encoder.
4. **AGPL and proprietary SDKs.** Camera RAW SDKs need a GPLv3 section 7 additional
   permission from all copyright holders, which needs a contributor policy that
   allows it.
5. **Dependencies.** `cargo-deny` enforces an allowed license list and RustSec
   advisories.

## 25. Testing, parity, and gates

### Baseline

- **E2E ledger.** Phase 0 makes the e2e suite green or quarantined, with each
  quarantined spec's owner and reason. Gates refer to the ledger.
- **Parity corpus**, captured before engine code lands: the 12 pixel-hash
  scenarios (today compared only when `PM_RENDER_BASELINE` is set; no hashes are
  checked in), every e2e fixture, synthetic projects per feature, and real-world
  projects Jude supplies (outside the repository). Outputs: float EXR and 8-bit PNG
  per sampled frame, and evaluated property dumps.

### Parity

- **Evaluation:** |a − b| ≤ 1e-9 + 1e-9·|b| for numbers; exact for strings and
  booleans; text queries as in section 6.
- **Render, Standard color:** per-channel 8-bit difference at most 1 for 99.9% of
  pixels and at most 4 for any pixel. Classes that differ by design have their own
  objective test:

  | Class | Test |
  |---|---|
  | Video frames | Today's Chromium frames against the engine within 2 CV; the engine within 1 CV of an FFmpeg plus zimg reference decode |
  | Text | Identical line breaks and advances within 0.25 px (level 1); SSIM ≥ 0.98; reviewed sheet |
  | Edges | Against the section 2 antialiasing reference |
  | Blur | Against an analytic Gaussian |
  | Alpha | Within 1 CV of an f64 reference |
  | Appendix A fixes | Listed per case with the defect number |

  Each real-world corpus project has at most 20% of its pixels covered by these
  classes at any sampled frame, or it gets a dedicated reviewed comparison.
- **Web player v1** against the engine in Standard color.

### Correctness and conformance

- Section 2 bars as automated tests.
- **Codecs.** Decodes match FFmpeg references (framemd5, or within 1 CV for
  hardware decoders); random access equals sequential; counts and timestamps exact.
  Corpus includes open-GOP HEVC, B-pyramid H.264, edit lists, VFR phone footage,
  interlaced MXF, HLG and HDR10, Dolby Vision, 4:2:2 10-bit HEVC and H.264, ProRes
  4444 XQ, DNxHR.
- **Exports.** Every format probed for codec, profile, pixel format, depth, tags,
  range, frame count, duration, audio samples, alpha, and sampled pixel values.
- **Determinism.** Identical buffers on re-render; native and WASM evaluation
  identical.
- **Fuzzing** with `cargo-fuzz`.

### Infrastructure

- Rust unit and property tests per crate.
- `pm-cli parity`, owned by the Parity and QA lane: evaluation dumps from Phase 1,
  renders from Phase 2, with difference images.
- Playwright e2e through the same UI; specs tied to WebGL internals rewritten
  against engine stats.
- Existing harnesses stay: `native-export-performance`, `opaque-capture-parity`,
  `export-video-frames`, `video-frame-integrity`, `rendering-performance`,
  `export-code`.
- A debug "Compare engines" command shows the difference for the current frame
  until cutover.

### Performance gates

Calibrated in Phase 0 on three reference machines: a base M1 MacBook Air (no
ProRes engine, the floor for the Automatic policy), a base M-series MacBook Air with
a ProRes engine ("base"), and an M-series Pro ("Pro"). Fixed after calibration.

| Scenario | Gate | Phase |
|---|---|---|
| Frame bridge under main load | Section 18 S1 gate | 0 |
| Layer drag, synthetic project of slice features (500 layers) | Pointer-to-pixel p95 ≤ 33 ms at 60 Hz | 2 |
| Inspector edit during playback, 1,000 layers | p95 ≤ 16 ms, no dropped playback frames | 2 |
| Scrub, intra codecs | p95 ≤ 50 ms | 2 |
| Scrub, long-GOP, no generated media | p95 ≤ 300 ms | 2 |
| A/V sync | ±10 ms | 2 |
| Export plus playback memory, base 8 GB | No memory-pressure warnings over 10 minutes | 2 |
| Cold start | Engine ready before the window finishes loading the last project | 2 |
| Layer drag, 884-layer project | p95 ≤ 33 ms, at most 1 frame worse than today | 3a |
| Canvas text typing | Keystroke-to-pixel p95 ≤ 33 ms | 3a |
| UHD ProRes 422 HQ, 3 layers with blur | Real time at 23.976, 29.97, 59.94 on Pro; 29.97 on base; zero drops over 60 s | 3a |
| Same, linear blending on | Same | 3a |
| The 884-layer project | The five ceilings that fail today pass | 3a |
| 4K ProRes 422 HQ export | At least real time on base | 3b |
| Export, every format at cutover | Absolute fps per codec and machine, at least today's re-measured baseline for that codec | 3b |
| 4K long-GOP HEVC 10-bit and H.264 4:2:2 10-bit, Automatic | Real time on all three machines, with generated media when the policy calls for it | 4 |
| Scrub, long-GOP with optimized media | p95 ≤ 100 ms | 4 |
| Paused upgrade to original | ≤ 150 ms (4K ProRes, base); ≤ 300 ms (4K long-GOP HEVC, base) | 4 |
| Segment boundaries, 4 streams | Zero drops | 4 |
| Memory | Within leases; no growth over a 1-hour playback loop; `pm-wasm` budget met | 3a and 4 |

## 26. Phases

Ordered by dependency; lanes run in parallel within and across phases (section
27). The legacy engine stays the default until Phase 5. Each gate has objective
criteria; Jude signs off at each gate in addition.

### Phase 0: Foundations, contracts, spikes

Work:

- Cargo workspace, CI toolchain, `cargo-deny`, napi-rs and wasm-bindgen builds; a
  placeholder addon signed and notarized through the real release script.
- Self-hosted reference runners.
- E2E ledger and parity corpus capture.
- `minReaderVersion` guard shipped in a normal release.
- Appendix A items marked (Phase 0).
- Opt-in crash and session reporting (decision 10).
- The FFmpeg replacement track (section 24).
- Frozen lane contracts (section 27).
- Spikes, each a throwaway branch with a written result:
  - **S1 frame bridge** (section 18).
  - **S2 precision and encode:** ProRes 4444 XQ in → wgpu → ProRes out;
    VideoToolbox formats; f16 versus f32 on 12-bit and 16-bit sources; HEVC alpha
    depth (`VTCopyVideoEncoderList`); AVAssetWriter tags; the QuickTime-matching
    Rec.709 tags.
  - **S3 shaders:** every GLSL source in the repository, the Store, and the docs
    samples through section 11; images compared with WebGL; naga versus
    SPIRV-Cross; flip and derivative cases.
  - **S4 text and vectors:** Parley, HarfRust, Skrifa layout against Canvas2D and
    CoreText (line breaks, advances, `trak`, optical sizes); glyph and vector
    rendering through (a) Vello for coverage with our own float paint, (b)
    `vello_gpu` (8-bit layers today), (c) Skia Graphite through `skia-safe` on
    Metal with half-float surfaces; stress with 4K caption-heavy scenes and large
    paths; a CPU fallback kept.
  - **S5 WASM evaluator:** `evP` under 1 µs for cached values through the
    shared-buffer path; patch apply under 0.2 ms for property edits and under 2 ms
    for structural edits on the 884-layer project with only changed ids
    invalidated; per-window memory budget.
  - **S6 decode:** VideoToolbox and FFmpeg decode of the corpus into wgpu without
    CPU copies; exact random access from container indexes.

Gates, per consumer:

- S5 and contracts 1, 5, 9 gate Phase 1.
- S1, S2, S6, the FFmpeg replacement, the reference runners, and the remaining
  contracts gate Phase 2.
- S3 and S4 gate the effects and text work in Phase 3a.

### Phase 1: Evaluation and the write path

Work: `pm-time`, `pm-model` (`v1_ingest`, reconciliation), `pm-eval`, `pm-edit`,
`pm-vector` geometry, `pm-text` layout, the font registry and subsetter in main
through `pm-bridge`, `pm-protocol`, `pm-wasm`; the write barrier,
flush-before-query, shadow-replay auditor, dual-answer mode; the single
frame-rate validator; `pm-cli parity` for evaluation. Synchronous queries and
geometry-dependent edits switch to `pm-wasm` behind a flag while the legacy
compositor renders. The headless document host switches to `pm-wasm`.

Gate: evaluation parity on the corpus; zero dual-answer mismatches beyond
tolerance and zero auditor failures across the e2e suite per the ledger; native
and WASM bit-identical; S5 targets met in the app.

### Phase 2: Tracer bullet

A thin slice through every layer of the architecture, behind "Use New Engine":

- Engine processes, MessagePorts, capability handles, budget leases, crash
  handling.
- The production frame bridge with jitter buffer and overlay geometry.
- `pm-media`: probe, container indexes, content hashes, VideoToolbox decode of
  ProRes, H.264, HEVC; image readers.
- Minimal `pm-render`: solid, image, video, transform, opacity, normal blend,
  precomp, groups; Standard color with the color invariants.
- `pm-audio`: device clock, streaming decode, mixing; playback; scrubbing.
- `pm-export`: ProRes and H.264 through VideoToolbox in the job engine; atomic
  output; verification.
- Media store in main, linked media, relinking by content hash, originals kept on
  import, `requiresEngine` stamping, the "When importing" setting.
- Font registry moves into the engine.
- `pm-cli parity` for renders.

Gate: Phase 2 performance rows; the color invariant, pipeline transparency, codec
generation, and time bars; crash and GPU-loss recovery; the legacy engine refuses
stamped projects and opens unstamped ones unchanged.

### Phase 3a: Rendering parity

All layer types, shapes and paths, text and captions, masks and mattes, all blend
modes, built-in effects and transitions in WGSL, the GLSL compatibility pipeline
and definition snapshots, motion blur, 2.5D, adjustment layers, retiming,
continuous precomp rasterization, linear blending, the internal scope tool.

Gate: render parity on the corpus; linear-blending, antialiasing, and determinism
bars; text review; Phase 3a performance rows.

### Phase 3b: Media and delivery

Every import format at cutover (section 13), interpretation and HDR input tone
mapping, VFR and interlacing, every export format at cutover (section 17),
presets, the render queue, audio stretch and remapping, channel layouts, meters,
waveforms, J/K/L at 1× and 2×, audio scrub.

Gate: codec and export conformance; bit depth, alpha, SDR color, seeking, and audio
bars; Phase 3b performance rows.

### Phase 3c: Integration

The kernel API mapping (section 20), agent tool changes, every consumer in
appendix C, posters, thumbnails, filmstrips, `pm-cli` exports, the headless
document host on engine data, serve's switch to web player v1.

Gate: every appendix C row done and covered by a test; e2e suite green on the new
engine per the ledger; agent tools pass their suites headlessly.

### Phase 4: Proxies, optimized media, caches

`pm-proxy` (ladder, segments, policy, machine profile, disk guard), render cache
with visibility and Keep Rendered, background rendering, appendix D controls,
diagnosis, Storage, Collect Project, the Relink sheet, Replace File, offline-media
export behavior, `manage_proxies`.

Gate: Phase 4 performance rows on the three machines without user action; the
cache-to-export byte-identity test; no proxy pixels in exports without a draft
choice; generated media survives moves, relinks, relaunch; storage limits and
pinning enforced.

### Phase 5: Cutover

New engine on by default for one release with the legacy engine behind a switch;
then removal of `legacy/gl` and the legacy media, playback, audio, and export
paths from the editor; web player v1 in `packages/player`; onboarding copy
rebuilt; `EXTENSION_API_VERSION` 4 with the registry release; docs updated
(`apps/desktop/docs/EXTENSIONS.md`, README, the engine docs in
`apps/desktop/docs`).

Gate: on the switch release, at least 2,000 sessions from at least 100 users who
opted in to reporting, with crash-free sessions at least 99.5% (or the cohort
alternative in decision 10); zero open P0 or P1 regressions; the engine-switch
round-trip test; legacy engine code deleted from the editor; docs updated.

### After cutover

Each with its own plan, roughly in this order:

1. Professional color (section 9 "After cutover"), scopes.
2. Delivery formats listed in section 17 "After cutover"; audio channel mapping.
3. Camera proxies and camera rules, Package for Travel, trimmed Collect, verified
   card offload.
4. Windows.
5. Depth-buffered 3D, cameras, lights; Collapse transformations.
6. Spatial bezier motion paths; frame blending and optical flow; faster shuttle
   and reverse audio.
7. Camera RAW (after section 24.4).
8. WASM web player (scene v2).
9. Audio effects and loudness tools.
10. OpenFX hosting out of process.
11. `pm-doc`: edit pipeline and history in Rust, ticks on disk.
12. PSD and multi-part EXR layers; FCPXML and OTIO.

## 27. Lanes, contracts, and ownership

### Contracts frozen in Phase 0

1. `EvaluatedScene`, between `pm-eval` and `pm-render`.
2. `FrameRequest` and `FrameDescriptor` (region, scale, color, precision,
   revision, PTS, overlay geometry).
3. The `pm-gpu` interop trait (decode in, bridge out, encode in).
4. The color contract: encoding E, encodings per blending setting,
   premultiplication, where transforms run, cache-key components.
5. Patches, paths, canonicalization, `v1_ingest`, and the engine-originated facts
   table.
6. The playback clock between `pm-audio` and the scheduler.
7. Errors and isolation (node, layer, asset, crash attribution).
8. Budget leases.
9. Protocol codec and code generation.
10. Definition snapshots.

### Lanes

| Lane | Owns | Phases |
|---|---|---|
| Model and evaluation | `pm-time`, `pm-model`, `pm-eval`, `pm-edit`, `pm-wasm` | 0, 1 |
| Renderer write path | Write barrier, flush, auditor, dual-answer mode, frame-rate validator, `minReaderVersion` and `requiresEngine`, headless document host | 0, 1, 2 |
| Text and vectors | `pm-text`, font registry, `pm-vector` | 0 (S4), 1, 3a |
| Media | `pm-media`, decoders, readers, index, content hashing | 0 (S6), 2, 3b |
| GPU and render | `pm-gpu`, `pm-render`, GLSL compatibility, definition snapshots | 0 (S3), 2, 3a |
| Color | `pm-color`, invariants, interpretation, tone mapping | 0 (S2), 2, 3a, 3b |
| Engine and platform | `pm-engine`, `pm-node`, `pm-bridge`, `pm-protocol`, leases, packaging, CI, runners, FFmpeg build, crash reporting | 0 (S1), 2 |
| Audio | `pm-audio`, mixer | 2, 3b |
| Export | `pm-export`, `pm-cli`, render queue | 2, 3b |
| Proxies and storage | `pm-proxy`, `pm-cache`, media store, media management | 2, 4 |
| Viewer and settings UI | Viewer, overlays, settings, appendix C and D, API mapping, extension docs | 2, 3c, 4, 5 |
| Parity and QA | Ledger, corpus, parity runner, conformance, performance gates, appendix A Phase 0 fixes | 0 onward |

Rules: each lane works in its own worktree and branch, commits granularly, and
submits stacked PRs (no squash merges); independent reviewers; the Parity and QA
lane owns gates and reports failures as they are; the legacy engine is frozen for
features from the start of Phase 3a.

## 28. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `sharedTexture` is experimental and tracks Chromium | Viewer breaks on an Electron upgrade | S1 first; ranked fallbacks; upgrades gated by a bridge test |
| Gesture latency through the bridge | Handles feel detached | Overlay geometry in frame descriptors; latency gates in S1, Phases 2 and 3a |
| Main-process jitter | Uneven playback | Import-once pools, small messages, jitter buffer, S1 under load, fallback A′ |
| Reads after writes inside edits | Wrong geometry | Write barrier, flush, edit kernels, dual-answer mode |
| Text looks different | Regressions in every project | Level 1 compatibility, S4, coverage tuning, reviewed corpus |
| GLSL translation failures | Broken extensions and projects | S3 on every known source; two paths evaluated; clear errors |
| Vector renderer maturity | Quality or stability | S4 includes Skia Graphite; float paint; CPU fallback |
| Extensions unavailable to headless renders | Wrong exports | Definition snapshots; explicit failure naming the extension |
| Switching engines loses data | Broken projects | `requiresEngine` stamping; media copied, not moved; round-trip gate |
| Hidden consumers of renderer media | Blank thumbnails, silent waveforms | Appendix C owned by a lane and gated in Phase 3c |
| Weak media identity | Wrong footage after relink or reuse | Full content hash; file identity re-verification |
| No CI hardware | Gates cannot run | Self-hosted runners in Phase 0 |
| No crash telemetry | Cutover gate unmeasurable | Opt-in reporting in Phase 0, or the cohort alternative |
| Scope creep before cutover | Parity quality suffers | Non-goals in section 2; "after cutover" lists |
| Feature drift | Double work | Legacy feature freeze from Phase 3a |
| Toolchain churn (wgpu, Electron, FFmpeg) | Upgrade cost | Pinned versions; upgrades as lanes |
| Licensing | Blocked features or exposure | Section 24 before dependent phases |
| Memory on 8 GB machines | Crashes | Main as arbiter, leases, pool sizing, base-machine gates |
| Engine crashes and loops | Lost playback or exports | Two engines, attribution, backoff, persisted jobs, atomic output |
| Chip-generation differences | Inconsistent behavior | Three reference machines; software fallbacks |
| Old builds overwriting new projects | Data loss | `minReaderVersion` guard in Phase 0 |

## 29. Decisions needed from Jude

1. **Default for new projects:** Standard color with "Blend in linear light" off
   (recommended; matches Figma and web hex expectations, and After Effects'
   default), or on.
2. **When importing:** Link to files (recommended) or Copy into project.
3. **Generated media location:** the Powermove cache (recommended), next to the
   project as a sidecar folder, or a chosen folder.
4. **Optimized media:** Only when I ask (recommended).
5. **Disk guard and storage limit:** pause below the larger of 20 GB or 10% free;
   storage limit 25% of the cache volume (recommended).
6. **Order after cutover:** professional color first, then delivery formats, then
   Windows (as written), or Windows earlier.
7. **Intel Macs:** no (recommended).
8. **Web export future:** replace web player v1 with the WASM engine after cutover
   (recommended).
9. **Serve interim:** serve tabs use web player v1 (recommended).
10. **Crash and session reporting** for the cutover gate: opt-in reporting with a
    privacy note (recommended), or a beta cohort submitting local crash logs, with
    the same thresholds.
11. **Reference machines:** a base M1 MacBook Air, a base M-series Air with a
    ProRes engine, an M-series Pro.
12. **Licensing counsel** for section 24 items 2 to 4.
13. **Real-world corpus projects** (kept outside the repository).

## 30. Appendix A: defects found during the survey

Items marked (Phase 0) are fixed in the legacy engine in Phase 0 by the Parity and
QA lane, so the corpus captures intended behavior. Others are fixed by the engine.

1. **Converted video replaces the original** (ProRes, MKV, MXF, and other
   FFmpeg-only formats become VP9 `yuva420p` with Opus audio; `raster.ts:874-889`,
   `media/media-path.ts:29`, `main/media-proxy.ts:39-51`).
   `apps/desktop/README.md:110-111` says the original stays embedded; it does not
   (`README.md:152-156` is correct). (Phase 0: correct the README.)
2. **Export transfer tag** is sRGB for H.264 and ProRes (`render-encoder.ts:12-15`).
   Fixed with the color rule in section 9.
3. **ProRes precision:** 10-bit with `-alpha_bits 16` from 8-bit data; 24-bit PCM
   padded from 16-bit WAV.
4. **Motion blur sample counts differ by path:** 16 (opaque native export, PNG,
   still), 20 (alpha, WebM, Recorder), 6 (web player, RAM preview), 6 playing and 12
   paused (preview), 10 (compositor default), 1 (agent snapshots, composition
   thumbnails). (Phase 0: one setting for all export paths.)
5. **WebM and Recorder exports may drop shaders still compiling:** `renderInto`
   (`exporter.ts:609-614`) renders without `exporting: true`, which no caller
   passes. Inferred from code. (Phase 0)
6. **Render queue:** nothing adds jobs; each prompts for a destination; a failure
   stops the run; the run swaps the live project (`render-queue.ts:4-12`).
7. **Frame-rate handling differs by path.**
8. **Timecode at fractional rates** is wrong (`util.ts:197-205`). (Phase 0)
9. **Variable speed** integrates at twice the open composition's rate, at least 60
   Hz (`retiming.ts:13`).
10. **Audio** ignores clip speed; time-remapped clips are silent.
11. **WebM exports** carry no color metadata.
12. **Aliasing:** hard UV edges, `antialias: false`, no mipmaps.
13. **Large blur radii** skip texels (`effects-basic/effects.ts:43-49`).
14. **Masks:** more than 8 parametric masks are silently dropped on the GPU path;
    any path mask sends all masks through Canvas2D with a CSS-blur feather.
15. **Solid corner radius** is stored but not rendered.
16. **Headless document host:** captions tools routed there fail; text anchor edits
    can throw (`editing.ts:859`).
17. **Docs disagree** on effect varyings (`EXTENSIONS.md:439` versus
    `api.ts:180-191`) and on when editing copies are made. (Phase 0)
18. **51 tracked Finder duplicates** (`* 2.*`). (Phase 0: delete.)
19. **`PM.previewFps`** is read but never assigned (`engine.ts:47`).
20. **No version check on load.** (Phase 0: the guard.)
21. **Unbounded caches:** the IndexedDB media store, editing copies, the live asset
    map, Bezier and spring solver caches, font sources.
22. **Sampled media fingerprint** can collide (`media.ts:96-97,153-171`).

## 31. Appendix B: feature parity inventory

Everything below works in the engine before cutover.

**Composition.** Size; frame rate (rational); duration; work area; background
(none, solid, linear and radial gradient up to 8 stops, radial with angle only);
shutter as a fraction of a frame (0 to 2, default 0.5); master audio gain; markers;
scene parameters (`param()`, `setParameter`); multiple compositions; nesting with
cycle checks; pre-compose (move and leave-attributes).

**Layers.** solid, shape (rect, ellipse, polygon, star with fixed 0.46 inner ratio,
line, editable paths), text, image, SVG, video, audio, adjustment, shader, extension
(fragment, mesh), null, group, precomp, captions. Fields: visibility (keyframable),
lock, shy, solo, collapsed, color label, blend (keyframable), motion blur
(keyframable), 3D, linked scale, parent, group, track, matte source and mode
(keyframable). Parenting that preserves pose. Groups as transform spaces and
compositing boundaries.

**Transforms.** Anchor, position, scale, rotation, opacity, skew (±80°); Z position,
anchor, scale; X and Y rotation; orientation; perspective per 3D group.

**Animation.** Linear, bezier, hold; speed and influence; auto-bezier; continuous;
spring parameters; legacy easing presets; color interpolation; string and boolean
steps. Expressions: `time`, `T`, `t`, `fps`, `value`, `layer`, `comp`, `ch`, `idx`,
`PI`, `Math.*`; `clamp`, `lerp`, `linear`, `ease`, `random`, `wiggle`, `bounce`,
`loop`, `pingpong`, `param`.

**Shapes and paths.** Fill and stroke, per-corner radius, smoothing (squircles),
polygon and star points, cubic paths with per-vertex tangents, trim and repeater
(up to 256) on editable paths, fill and stroke opacity, SVG import to editable paths.

**Text.** Font, weight, size, tracking, leading, color, alignment, italic, box width
and height, variable axes, range styles, animators (stagger, range, wave;
characters, words, lines; all orders and shapes; opacity, position, scale, rotation,
skew, tracking, blur, color), font anchoring, local and embedded fonts, canvas text
editing.

**Masks and mattes.** Parametric rect and ellipse masks (feather, rotation, add,
subtract), bezier path masks, track mattes (alpha, alpha inverted, luma, luma
inverted), matte nesting, cycle rejection.

**Compositing.** Nine blend modes, opacity, group effects, adjustment layers, motion
blur, in and out transitions.

**Effects.** Gaussian Blur, Directional Blur, Sharpen, Glow, Vignette, Drop Shadow,
Color, Levels, Duotone, Posterize, Invert, Film Grain, Mosaic, Chromatic Aberration,
Turbulent Displace, Gradient Ramp; multi-pass, backdrop, `keepOrig`, viewport-safe
and padding semantics; extension effects.

**Transitions.** Crossfade, Wipe, Slide, Zoom; extension transitions.

**Shader and extension layers.** Shadertoy-style header, `@param` uniforms, five
presets, Studio Cube, OBJ model.

**Retiming.** Animated speed, time remap, freeze, reverse, source time, independent
decoders per instance.

**Media.** Every import format today, animated images, image sequences with gap
reporting, posters, iCloud and File Provider materialization, URL import for
extensions, remote media in serve mode, OBJ, SRT and VTT import to captions.

**Audio.** Per-layer gain, mute, solo, fades, keyframed gain, precomp nesting,
embedded video audio, meters, waveforms, master gain.

**Preview.** Resolution (Automatic, Full, Half, Quarter), Automatic quality, frame
cache, cached preview, retained presentation during pan and zoom, viewport-cropped
rendering, context-loss recovery, FPS readout and status states.

**Export.** Native MP4 and ProRes, WebM (frame-exact and realtime), PNG sequence,
still, code export, project export, presets, per-project defaults, work-area range,
scale, fractional rates, captions sidecars and burn-in, progress with live preview,
cancel, render queue.

**Kernel and agents.** Every API and tool in section 20.

## 32. Appendix C: integration checklist

Every consumer of today's engine outside the compositor. Owned by the Viewer and
settings UI lane and gated in Phase 3c.

| Consumer | Today | Replacement |
|---|---|---|
| Composition thumbnails (`panels/comp-thumbnails.svelte.ts:58-70`) | `PM.GL.renderToPixels` | Async engine thumbnails |
| Project thumbnail on save (`legacy/app.ts:567-576`) | Reads the viewer canvas | Engine snapshot |
| Library look thumbnails (`legacy/core/library.ts:87`) | `PM.Export.snapshot` | Engine snapshot |
| Library viewer card (`library/LibraryScreen.svelte:352-361`) | Copies live canvases | `library.render` with an engine snapshot |
| Assets panel posters and inline video (`panels/AssetsPanel.svelte:152-153,209-217,1019-1024`) | IndexedDB posters, blob-URL `<video>` | Engine posters and filmstrips; `media://` playback only for Chromium-playable formats |
| Assets panel audio thumbnails (`AssetsPanel.svelte:254-261`) | WebAudio peaks | Engine peaks |
| Source preview monitor (`viewer/source-preview.ts:177-205`) | `<video>`, `<audio>` | Engine source frames through the bridge, engine audio |
| Timeline waveforms (`timeline.ts:1702-1716`) | WebAudio peaks | Engine peaks |
| Mixer meters, solo, retune (`panels/MixerPanel.svelte`, `mixer/meter-loop.ts`) | WebAudio analysers | Engine meters, solo, retune |
| Caption rendering (`captions/render.ts`) | Canvas2D | `pm-text`, `pm-vector` |
| Text editor caret and font offset (`viewer/text-editor.ts:98-102`) | `textLayout`, `raster().fontOffset` | `pm-wasm` |
| Text animators, text stagger (`TextAnimators.svelte:142`, `text-stagger.ts:30`) | `textLayout` | `pm-wasm` |
| Align, group and comp bounds (`shortcuts.ts:150,222`, `layer-groups.ts:48`, `compositions.ts:504`, `AlignmentStrip.svelte:27`) | `PM.GL.bounds` | `pm-wasm`, `pm-edit` |
| Shader compile status (`CompileStatus.svelte:21`, `ShaderPanel.svelte:51`, `ContentSection.svelte:35`) | `compileError`, `dropProgram`, program keys | Engine compiler status |
| Status bar (`shell/StatusBar.svelte:6-65`) | GPU loss, compiling, per-extension GPU timing | Engine stats with per-node and per-extension timing |
| Performance panel and store (`panels/PerfPanel.svelte`, `runtime/install-legacy.ts:101-114`) | `GL.stats`, `rasterStats` | Engine stats |
| Frame bus (`runtime/frame-bus.ts`) | `overlay` after `GL.render` | On presentation |
| Export progress preview (`legacy/core/export-preview.ts`) | Live frames | Streamed small frames |
| RAM preview (`legacy/core/preview-cache.ts`) | `ImageBitmap`s | Render cache |
| Posters (`legacy/core/poster.ts`) | Seeks `asset.el` | Engine posters |
| Cloud and remote media (`legacy/core/cloud-media.ts`, `kernel/remote-media.ts`) | Bytes through the renderer, Chromium probe | Main materializes, engine probes |
| Image sequence import (`legacy/core/image-sequence.ts`) | `createImageBitmap` check, VP9 encode | Engine sequence reader |
| Fonts (`legacy/core/fonts.ts`, `typography/*`, `main/fonts.ts`) | `document.fonts`, `queryLocalFonts` | Font registry and subsets |
| Panel popout (`main/panel-popout.ts`) | Same-realm child windows | Frames sent to the window hosting the viewer |
| Web player, www, onboarding | TypeScript engine | Web player v1 (section 22) |
| Headless document host (`src/engine/main.ts`) | `legacy/core` under happy-dom | `pm-wasm` |
| Agent tools | Compositor, Canvas2D | Section 20 |

## 33. Appendix D: media and proxy interface

All surfaces follow the design language: quiet text in lists (no badges, glyphs,
counts, or thumbnails in rows), Settings-sheet cards and rows for dialogs and
sheets (Interpret Footage, Collect Project, export, queue), NSMenu only for
right-click, the app's popover menu for button menus, the app's confirm dialog for
confirmations, sentence case in Settings, title case in menus.

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
| Storage | Space used by proxies, optimized media, rendered frames, media index, thumbnails and waveforms; each with Clear… and a limit |

### Asset right-click menu

Create Proxy · Create Optimized Media · Rebuild Proxy · Delete Proxy and Optimized
Media · Always Play Original (check item) · Interpret Footage… · Replace File… ·
Reveal in Finder. (After cutover: Attach Camera Proxy….)

### Viewer menu

The viewer's resolution control (today a native `<select>`, `viewer/viewer.ts:858`)
becomes a popover menu. `controls/popover-menu.ts` is extended with per-section
headers, one radio group per section, and a check item:

| Section | Items |
|---|---|
| Resolution | Automatic, Full, Half, Quarter |
| Media | Automatic, Originals, Proxies |
| (none) | Play Every Frame (check item) |

The button label states what is on screen when degraded, as quiet text: "Half ·
Proxy". It shows "Full" once a paused frame upgrades. "Proxies" works like Final Cut
Pro's Proxy Only: clips without a proxy show as offline. A keymap command toggles
Originals and Proxies.

### Status and diagnosis

- Generating: "Creating proxies · 3 of 12". Paused: "Proxies paused · Low disk
  space". The status item does not change during playback.
- After playback with drops: "Dropped 14 frames · 2 clips decode slowly", action
  Create Proxies; or "Dropped 14 frames · GPU busy".
- Errors: "Couldn't create a proxy for A001_C003.mov. The file ends early at
  01:12:04. Copy the complete file, then choose Rebuild Proxy."

### Storage

Clear… confirms: "Clear proxies?" with "This frees 48.2 GB. Powermove creates them
again when needed. Proxies for 3 clips whose originals are offline are kept." Clear
and eviction never remove pinned media (proxies of offline originals, kept renders,
and after cutover attached camera proxies and travel packages).

### Offline media at export

Native three-button message:

- Title: "2 clips are offline"
- Body: "Their original files can't be found. Locate them to export at full quality,
  or export these clips from their proxies at lower resolution."
- Buttons: Cancel, Export with Proxies, Locate Media…

Queue: "Waiting for media · 2 clips offline". Draft option: "Render using: Proxies
(draft)", with an optional "Draft" mark.

### Import and relink

- Import toast: "Linked 12 clips · Copied 3 images".
- Viewer: "Media offline · A001_C003.mov"; timeline clip label: "Offline".
- Converted legacy assets: "Using a converted copy · Relink to the original for full
  quality", Locate Original….
- Variable frame rate: "Variable frame rate · Plays at 60 fps by timestamp".

### Render cache

A 2-pixel track under the ruler (memory lighter, disk stronger); View › Show
Rendered Frames; Render Work Area; Render Selection; Delete Rendered Frames; Keep
Rendered on precomps.

## 34. Appendix E: glossary

- **Tick:** 1/705,600,000 second.
- **Display encoding E:** the sRGB piecewise curve shared by sRGB and Rec.709 by
  default.
- **Standard color:** intermediates in E; today's look.
- **Linear blending:** "Blend in linear light" on.
- **Canonical patch:** a document patch addressed without the open-composition root
  rotation.
- **Mirror:** a patch-fed copy of the document (`pm-wasm`, engine sessions).
- **Write barrier:** records every document write so mirrors can flush before
  queries.
- **Edit kernel:** a geometry-dependent edit computed in `pm-edit`, returned as
  patches.
- **Definition snapshot:** the GLSL and parameters of a registered effect,
  transition, or extension layer, pushed to sessions and embedded in exports.
- **Representation:** original, reduced-resolution decode, proxy segments,
  optimized segments, kept render.
- **Segment:** a ~2-second keyframe-aligned piece of generated media, addressed by
  content.
- **Machine profile:** measured decode, GPU, disk, and power characteristics of this
  Mac.
- **Lease:** a memory allowance granted by main to an engine process.
- **Frame bridge:** the shared-surface path from the engine to the viewer.
- **Interactive engine, job engine:** the two engine processes.
- **`compat` level:** per-layer compatibility marker (1: legacy behavior, 2: new).
- **Parity corpus:** projects and expected outputs captured from the legacy engine.
- **E2E ledger:** quarantined end-to-end specs with owners and reasons.
