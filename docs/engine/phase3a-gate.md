# Phase 3a gate

The Phase 3a gate (plan section 26, "Phase 3a: Rendering parity"; section 25's
render parity classes, the Phase 3a performance rows, and the memory row; the
section 2 bars Phase 3a names; section 11's shader gate; amendments 9 to 11),
measured on `feat/ne-p3agate`: the integration branch `feat/native-engine` at
`c4c1c5f5` (waves 1 to 4 and wave 3d, the long-GOP scrub lane) plus the two lanes
with commits not yet on it, `feat/ne-effects` (the watchdog's deferred resolve)
and `feat/ne-text3a` (the stricter CSS number grammar). `feat/ne-composite`,
`feat/ne-vector3a`, `feat/ne-motion`, `feat/ne-scene3d`, `feat/ne-gltf`,
`feat/ne-blender`, and `feat/ne-shaders` had nothing left to merge. No engine
code was changed here; this lane added measurement code only (below).

Measured 2026-10-17 between 20:55 and 23:57 (the video class, the review
rule's corpus rerun, the pm-wasm budget, png16 determinism, and the shader
corpus recollection again on 2026-10-18 between 00:05 and 00:35, after review;
the video class with tagged sources and the pm-wasm reply kinds again between
00:40 and 00:55)
on Jude's MacBook Pro (Apple M4
Max, 128 GB, macOS 27.0), wgpu 30.0.1 on Metal, Electron 44.0.0, release builds
in the worktree. The Mac was shared with other lanes: the 1-minute load average
was 5.5 to 12.8 during the engine runs (each step's load is in its log). Every
Electron run was hidden and unfocused, one at a time under the machine lock.
This is the Pro-class reference machine of decision 11; base-machine rows are
deferred by Jude's decision and are marked so. Numbers are as measured; nothing
is passed on a looser tolerance than the plan's. Where a lane's test pins a
known difference as a "recorded deviation" or "known residual", this report
counts the frame against the plan's tolerance and lists the pin.

> Gate: render parity including the 3D and Blender classes; the section 11
> shader gate (repository, Store, docs samples, generated agent corpus);
> linear-blending invariants and linear blending, antialiasing, 3D, and
> determinism bars; text review; Phase 3a performance rows.

## Summary

| Gate line | Measured | Result |
|---|---|---|
| Render parity, Standard color, whole corpus (`pm-cli parity render`) | 218 projects in 5 captures: 123 compared (454 frames), 92 routed to the legacy engine, 3 not comparable (the legacy frame is a playback proxy). 434 frames within their class, 14 fail, 6 WebGL-unstable for review | **fail** |
| Every corpus project renders (section 2 parity bar) | 92 of 218 projects still route: SVG images 34, text without engine fonts 68 (32 of them for nothing else), stills outside the measured classes 15, 2.5D text 3, masks on 3D layers 3, motion blur on text and 3D 3, animated images 3, path masks 1 | **fail** |
| Real-world project (the 884-layer Powermove ad) | Routed (2.5D text, text without fonts); with fonts it would still route for 2.5D text. Not compared; the 20% class rule not measurable | **fail** |
| Video class | Against Chromium (natively decoded sources, patch interiors): 29 of 34 frames within 2 CV, 5 fail (max 3 to 255 CV). Against FFmpeg plus zimg (new): the references take matrix, range, and siting from ffprobe's tags under section 9's untagged rules, and the engine's reading equals them on all 29 sources. The 10 e2e fixture videos are one flat colour per frame and all read BT.601 limited, so they check only that conversion (0.000 CV); the chroma path is measured on 17 generated sources with chroma edges (BT.601 limited, BT.709 limited, BT.709 full range, BT.2020 NCL 10-bit, and a fully tagged BT.2020 source, which routes to the legacy engine for its primaries) and on every frame of the corpus's 2 H.264 sources. Sources FFmpeg decodes as the engine does: 0.000 CV with the engine's edge rule (zimg's own edges: up to 3.31 CV on the 2-pixel border; zimg's 16-bit resize clips chroma overshoot: up to 25.5 CV on 368 corpus pixels, 0.000 against an unclipped f64 reference; full-range chroma reaches the code floor and ceiling, 0.982 CV). ProRes 422 HQ: 2.017 CV (VideoToolbox's decode differs from FFmpeg's; 0.000 against FFmpeg's VideoToolbox decode) | **fail**: the Chromium line; ProRes 422 HQ against FFmpeg's decoder |
| Text class (level 1, whole frames) | 31 projects, 123 frames: text pixels SSIM min 0.989, every other pixel exact | pass |
| Text class (level 1, rasters) | 495 cases: 80 pinned below SSIM 0.98 (text at 18 px and below, synthesized bold, emoji under 10 px) | **fail** |
| Text class (level 1, line breaks and advances) | Native: 0 line-break differences, 0 advances over 0.25 px. Through pm-wasm: 1,260 `system-ui` layouts of the real project off by up to 18.2 px (Phase 1 T1, open) | **fail** (T1) |
| Text review | Sheets regenerated at this tip: `~/Library/Application Support/Powermove Engine Corpus/review/text-1792297672/index.html` (420 cases) | needs Jude |
| Alpha class | 157 of 165 transparent float frames within 1 CV; 5 fail (1.49 to 4.61 CV: `motion/blur-solids` 4, `composite/blend-modes-alpha` 1); 3 `render/fx-posterize` frames WebGL-unstable, for review (15.9 to 85.0 CV) | **fail** |
| 3D class, corpus | 21 projects, 59 frames: 100% of 3D pixels within 2 CV, max 2 CV, outside MSAA edges max 1 CV, object SSIM min 0.99993 | pass |
| 3D bar (three.js r180 sweeps) | 74 primitive-scene outputs and 165 glTF outputs (105 Khronos models): every output within the bar | pass |
| Blender surfaces | Exact at 32-bit (max 6.8e-8, f32 rounding); 1.06e-3 at 16-bit | pass |
| Blender snapshots | 158 corpus snapshots: 135 equal (geometry max 3.5e-16, 9,622 other values exact), 23 not built (`3d/gltf`, `3d/gltf-materials`: imported models); 74 of 74 frame lists equal | **fail** (23) |
| Shader gate (section 11) | 522 variants of 144 sources (repository, Store, docs, cloud samples, generated agent corpus): 504 match, 3 through the fallback runner, 15 refused by both, 3 WebGL-unstable (review), 0 failures. Built-ins 228 of 228; probe sweep 394: 260 match, 122 both refuse, 12 known open | pass, with the review items |
| Color invariants (1) and (2) with linear blending on | 0 samples differ (4:2:0, ProRes 422 HQ → `v216`, 4444 XQ → `y416`); solids ≤ 1 CV at 10 bits and above, ≤ 2 CV 8-bit limited, greys neutral | pass |
| Linear blending: 50% white over black, 2× checkerboard | 188 and 188 (0.735357) | pass |
| Linear blending: blurred white line conserves energy | Layer effect over the background: 0.000% error. Adjustment layer over opaque black: −76.5% to −92.3% (legacy GLSL runs in E, section 11) | **fail** |
| Antialiasing (level 2) | Zone plate and Siemens star: final PSNR min 70.78 dB (≥ 40), playback min 54.75 dB (≥ 34), flicker ≤ 1.000× (≤ 1.5×); level 1 equals today (corpus) | pass |
| Determinism | 289 corpus samples rendered twice with caches dropped: 0 differ; 270 16-bit PNG export frames of 9 projects exported twice: byte-identical. Native against WASM evaluation: 1,815,420 replies, 0 differ | pass |
| Layer drag, 884-layer project | Not measured: the harness runs found the main display asleep and the session locked (22:31 and 23:56) and skip, as they must. By the static check the presenter runs, the project routes to the legacy engine (2.5D text, text), so the engine path would not draw it either | **not measured**; the engine does not draw the project |
| 3D pick, 1M-vertex skinned model | `pm-geom3d`: unchanged frame max 0.016 ms (≤ 2), first pick after a time change max 15.48 ms (≤ 33). The app still picks through three.js (wired in 3c) | pass (engine); app path not on the engine |
| Canvas text typing | Engine share p95 1.70 to 3.50 ms (≤ 33). End to end not measurable: text routes to the legacy engine (the hosts give no fonts) | **fail** (not drawn by the engine) |
| UHD ProRes 422 HQ, 3 layers with blur | Drawn by the engine, 60 s looping: 23.976 fps 0 drops (1,438 frames shown), 29.97 fps 0 drops (1,799); 59.94 fps 4 frames skipped of 3,596, all at loop wraps of a composition that was not a whole number of frames (3.6 expected from the loop length alone), 1 late-skipped by the engine; its rerun did not complete (disk). With linear blending on, the composition routes to the legacy engine at every rate (`requiresEngine` stamp) | **fail**: linear blending not drawn by the engine; 59.94 not cleanly measured |
| The 884-layer project, the five ceilings | Random scrub max render 14.7 ms (ceiling 100); high-zoom wheel 1 render (ceiling 15); region pan 3, 5, 5 renders (ceiling 25). The same with Use New Engine off (14.9 ms; 1; 2, 5, 5). The project routes to the legacy engine, so both runs measure today's compositor | pass as measured; not an engine measurement |
| Memory: within leases, no growth over a 1-hour loop, pm-wasm budget with the 3D corpus | 1-hour loop of the 1,000-layer project drawn by the engine (107,965 frames, 0 late, 0 errors): lease usage 623 MB of a 58.4 GB interactive lease at every sample; the interactive engine's resident memory rose 357 → 384 MiB between minutes 5 and 30, then stayed at 384 MiB for the last 30 minutes; the editor page 566 → 577 MiB. pm-wasm: every 3D corpus project ≤ 0.21 MB live heap and 1.97 MB linear memory; the real project 11.3 MB live (S5's proposal 8 MB), 25.5 MB linear (≤ 32). Every layer and `ev` query answered; 1,020 of 1,280 picks unanswered (960 `host`, 60 `fonts`), so pick memory is not in these figures | pass for leases and the last 30 minutes; the 3D corpus within the pm-wasm proposal; the real project's mirror over it (decision) |
| Base-machine rows (UHD ProRes 29.97 on base) | Not measured | deferred (base machine) |

The gate does not pass as measured. The Phase 3a gate also stands on the
Phase 1 and Phase 2 gates, which did not pass as `phase1-gate.md` and
`phase2-gate.md` record them (T1 remains open here).

What stands between this tip and a pass, by owner:

1. **Text (`pm-text`, `pm-node`, `pm-wasm`)**: the hosts give their renderers
   no fonts, so every text and captions layer routes (68 corpus projects, the
   real project, canvas typing); the level 1 Canvas2D profile misses SSIM 0.98
   on 80 raster cases (18 px and below, synthesized bold, small emoji); the
   pm-wasm text measurement needs Jude's T1 decision (font-cache seeds).
   Text review: Jude, on the sheets above.
2. **Vector (`pm-vector`)**: SVG images route at level 1 (34 projects): resvg
   differs from Chromium's Skia by up to 116 CV at edges. The one
   `composition/metadata` residual (one flipped MSAA sample) is a level 1
   emulation limit, now a decision.
3. **Render graph (`pm-render`)**: 2.5D text and other non-raster 2.5D types,
   motion blur on text and 3D layers, masks on 3D object layers, and path
   masks route; the static check routes every project stamped
   `requiresEngine` (linear blending, linked media), so the native-only
   features it was added for are drawn by the legacy engine
   (`capability.rs`, `Feature::RequiresEngine`; measured in the app with
   "Blend in linear light"); the f16 motion-blur
   accumulator is off by up to 1.62 CV in alpha; the blurred-white-line energy
   line fails for stacks because level 1 effects run in E and no level 2 blur
   exists; imported glTF models are not built into Blender snapshots (23); the
   job engine carries no definitions, so exports route every effect and
   transition.
4. **Media (`pm-media`, `pm-color`)**: still classes outside the measured
   ones route (15 projects: AVIF, 16-bit PNG, Display P3, Y′CbCr JPEG, WebP
   with alpha, oriented); animated images route (3); ProRes 422 HQ decoded by
   VideoToolbox is 2.017 CV from the class's FFmpeg reference at saturated
   chroma edges (the decoders differ; a plan decision or an FFmpeg ProRes
   path). The probe reads H.273 matrix 10 (BT.2020 constant luminance) as
   BT.2020 NCL (`pm-media` `probe/color.rs`; no source in the corpus or the
   video class uses it, so unmeasured). Decoded video differs from Chromium by 3 to 9 CV at saturated chroma
   edges (3 frames), where the engine matches FFmpeg exactly. With
   Proxies and storage: linked UHD ProRes clips were offline to the engine
   once main's rendition finished (not investigated; below).
5. **Plan owner (Jude)**: appendix A decisions for today's composite drift
   (`composite/blend-modes-alpha`) and today's stale blurred-video frame
   (`motion/blur-video`) and Chromium's chroma edges (`retime/speed`,
   `space25d/precomps`), which ProRes decoder the video class's reference
   uses (VideoToolbox, which the engine uses, or FFmpeg's), whether the video
   class's reference may clip Catmull-Rom chroma overshoot (zimg's 16-bit
   resize does, 23.2 and 25.5 CV on the corpus H.264 media; an unclipped f64
   reference matches the engine to 0.000 CV), the WebGL-unstable review (Store
   `liquid-glass-ios27` 2.0.8; built-in Color, Levels, Posterize), how the
   energy line applies to `compat` level 1 effects, and the S5 per-window
   budget for `pm-wasm` (the real project's mirror holds 11.3 MB against the
   proposed 8 MB).
6. **Performance rows**: linear blending is drawn by the legacy engine in
   the app (the `requiresEngine` routing above), so the UHD row's linear half
   fails; 59.94 needs a clean rerun; canvas typing and the 884-layer drag
   need text and 2.5D text on the engine first, then a drag run with the
   display awake (the harness skips runs while the display sleeps).

## Render parity (section 25)

`pm-cli parity render` (the job engine's frame path, `pm_export::render::render_frame`)
over every project-layout capture: corpus v1 (`v1-20261012-230504-ae47a3b3`, 131
synthetic projects and the real project), the render cases
(`render-20261014-131536-ea28ddc1`, 40), the effects cases
(`effects-20261017-060101-ca4d364f`, 24), the composite cases
(`composite-20261016-042859-be1e4467`, 11), and the motion cases
(`motion-20261016-085932-b52905bb`, 11): 218 projects. The renderer holds the
built-in definitions the kernel pushes, as the interactive engine does; the real
project is added with its media unpacked from the file the capture opened
(`--real`, checked by SHA-256). Classes as section 25 states them: Standard
(≤ 1 CV for 99.9% of pixels, ≤ 4 for any), video (natively decoded sources only,
patch interiors within 2 CV), alpha (transparent float frames within 1 CV), 3D
(section 2's bar on 3D pixels and MSAA edges, Standard elsewhere). Summary:
`phase3a-gate-measurements/parity-render.json`.

| Class | Frames | Within | Fail | Review (WebGL-unstable) |
|---|---:|---:|---:|---:|
| Standard | 213 | 206 | 4 | 3 |
| Standard with 3D pixels | 42 | 42 | 0 | 0 |
| Alpha (float) | 165 | 157 | 5 | 3 |
| Video | 34 | 29 | 5 | 0 |
| **All** | **454** | **434** | **14** | **6** |

Every frame is counted once, in its class. Standard fails: `composition/metadata`
3, `composite/blend-modes-alpha` 1; alpha fails: `motion/blur-solids` 4,
`composite/blend-modes-alpha` 1 (1.49 to 4.61 CV). The six review frames are
`render/fx-posterize` t = 0, 1, 2 in the Standard view (max 16, 27, 85 CV at
isolated pixels, ≥ 99.90% exact) and the alpha view (15.9, 26.9, 85.0 CV):
Posterize is WebGL-unstable at the shader level (`effects_webgl.rs`: its WebGL
output moves by as much under a one-step input perturbation), judged by review
under amendment 9. The runner moves a failing frame to review only when the
difference is confined to the pixels the unstable effect changes: it renders
the frame again without the WebGL-unstable effects and requires the frame to
pass its class on every pixel they leave unchanged (rerun at this tip: the same
6 frames, all confined; every other frame's result unchanged). Only Standard
and alpha frames can move to review; a Standard+3D or video frame always stays
a failure (the rule is not defined against the 3D bar's masks; unit tested).

Failures, triaged:

| Frames | Measured | Cause | Owner |
|---|---|---|---|
| `composition/metadata` t = 0.5, 2, 3.5 (Standard) | 99.997 to 99.999% within 1 CV; max 43, 30, 42 CV at 2 to 6 pixels | Canvas2D's raster of a rotated radius-24 rectangle has one MSAA sample flipped at a tessellated arc edge (0.145/256 px inside a sliver triangle); the level 1 emulation reproduces every other sample. Pinned as `KNOWN_RESIDUALS` in `tests/corpus.rs` | `pm-vector` (level 1 Canvas2D emulation, `docs/engine/vector.md` "Residuals") |
| `composite/blend-modes-alpha` t = 0 (alpha and Standard) | float max 4.61 CV; 8-bit max 5 CV, 99.944% within 1 | Today's composite drift: `FRAG_COMPOSITE` round-trips every pixel of the frame through unpremultiply and re-premultiply in RGBA16F for each of 48 blend layers; the engine passes uncovered pixels through exactly. Pinned as `COMPOSITE_DRIFT` | `pm-render` composite (`docs/engine/composite.md`); proposed appendix A item, a decision for Jude |
| `motion/blur-solids` t = 0.2, 0.5, 1, 1.6 (alpha) | float max 1.49 to 1.62 CV | Each still sample differs from Chromium's by up to 2 f16 ulps and the RGBA16F accumulator rounds every partial sum over 20 samples. Pinned as a recorded deviation in `tests/motion.rs` | `pm-render` motion (`docs/engine/motion.md`): an f32 accumulator, or an appendix A decision |
| `motion/blur-video` t = 0.5, 1.2 (video) | max 255 CV | Today's capture draws a stale, unseeked frame for blurred video (`renderToPixels` sets `useVideoPreviews = false`; no prepared frame exists per sample time); the engine draws the right frames. A legacy defect | Parity and QA: appendix A item, and the capture should not be the reference for this frame |
| `retime/speed` t = 2.9; `space25d/precomps` t = 1.5, 2.6 (video) | max 3, 7, 9 CV on patch interiors (the frame one later differs by 200 to 249 CV, so the frame is right) | Chromium's decode and the engine's differ at saturated chroma edges (chroma upsampling and 8-bit clipping). The engine's side is right: every frame of the two sources (`corpus-frames-24.mp4`, 96 frames; `corpus-frames-30.mp4`, 120) is within 0.000 CV of FFmpeg's decode upsampled with the plan's filter (below) | Parity and QA: the Chromium capture is not the reference for these frames (appendix A item); no engine change |

Not comparable with the capture (3 projects): `media/converted-mkv`,
`media/prores422-mov`, `media/prores4444-alpha-mov`: the legacy frame is the
playback proxy (a VP9 transcode), not a decode of the original. Section 25
compares such sources with an FFmpeg plus zimg reference "using the same
chroma filter" within 1 CV, which was not built before this gate;
`pm-cli` `tests/phase3a_video_class.rs` (new) measures it. Each source is drawn
alone at its native size at 32-bit (one source pixel per output pixel) and
compared with frame n decoded by FFmpeg and converted by zimg to 32-bit float
RGB (no clipping of the result) with the engine's chroma filter: 4:2:2 linear
across (zimg `bilinear`); 4:2:0 Catmull-Rom across and linear between rows
(`pm-color` `chroma.rs`), which zimg cannot do in one pass, so the reference
runs two zscale passes (4:2:0 to 4:2:2 at 16 bits with `bilinear`, then to
RGB with `bicubic` b = 0, c = 0.5).

The matrix, range, and siting of the references come from the file's tags,
not from the engine: ffprobe's `color_space`, `color_range`, and
`chroma_location`, with section 9's rules for untagged values written into the
test from the plan (matrix BT.601 at heights up to 576, else BT.709; MJPEG,
`yuvj`, and RGB full range, other Y′CbCr limited; siting left). The test
asserts that the engine's reading (`Renderer::video_color`, or the probe's
for a source the renderer refuses) equals that reading, so a misread tag fails
the test instead of being compared with a reference built from the same
misreading. Result: the engine's reading equals the tags' on all 29 sources
(matrix, range, subsampling, and siting where the source is subsampled).

Three readings of each frame:

- **zimg's edges**: zimg as it is.
- **The engine's edge rule** (judged): the decoded planes are first extended
  by 8 repeated edge samples (`fillborders` `smear`) and cropped after, as
  `chroma.rs` clamps taps to the plane.
- **f64** (a diagnostic): FFmpeg's decoded planes upsampled and converted by
  the test's own f64 implementation of the plan's filter (H.273 range
  scaling: limited 219 and 224 × 2^(n−8), full 2^n − 1), edges clamped,
  nothing clipped. zimg's word-type resize clamps chroma overshoot at the
  16-bit code floor and ceiling; this reading does not.

The e2e fixture videos are **flat**: every frame is one Y′CbCr triple (ffmpeg
`signalstats` over 24 frames: `prores422.mov` Y = 324, U = 360, V = 960
throughout; the 4:2:0 files Y = 81, U = 90, V = 240; `prores4444-alpha.mov`
flat colour with an alpha edge). All ten are untagged or tagged only `tv`, so
they read as BT.601 limited (height 90). They check that one conversion only
and never exercise chroma upsampling, siting, or the filter; that is all they
settle for the three proxy-only corpus sources (which use these files). All
ten: 0.000 CV in every reading (alpha 0.000 CV). The 0.970 CV this report gave
before came from the old reference's 16-bit integer RGB, which clipped the
fixtures' slightly out-of-gamut red (G and B decode to −0.19% and −0.38%); the
engine's 32-bit output keeps them.

The chroma path, and the matrices and ranges beyond BT.601 limited, are
measured on sources with chroma edges: 17 generated from `testsrc2` (saturated
bars, a moving pattern, and text; chroma codes 16 to 240, 0 to 255 at full
range, with edges both ways) with the pinned FFmpeg, frames 0, 7, 15, 23, and
every frame of the corpus's own H.264 media (the motion capture's
`corpus-frames-24.mp4` and `corpus-frames-30.mp4`, the sources of the
`retime/speed` and `space25d/precomps` rows above). The first 12 generated
sources are tagged BT.601 (`smpte170m`) limited; the tagged five are converted
from RGB with their own matrix and range (`scale`) and then tagged
(`setparams`, written by the muxers as Matroska `Colour`, MOV `colr`, and HEVC
VUI):

| Source | Chroma, siting | Tags, as read | Decoded by the engine with | zimg's edges | Engine's edge rule | f64 |
|---|---|---|---|---:|---:|---:|
| ProRes 422 HQ (VideoToolbox encoder) | 4:2:2 10-bit, left | BT.601 limited | VideoToolbox | 2.017 (224 px over 1) | **2.017** (224 px) | 2.017 |
| ProRes 4444 | 4:4:4 12-bit | BT.601 limited | VideoToolbox | 0.541 | 0.541 | 0.541 |
| ProRes 4444 with alpha edges | 4:4:4 12-bit + alpha | BT.601 limited | VideoToolbox | 0.204 (alpha 0.012) | 0.204 | — |
| FFV1 | 4:2:2 10-bit, left | BT.601 limited | FFmpeg | 0.000 | 0.000 | 0.000 |
| FFV1 | 4:4:4 8-bit | BT.601 limited | FFmpeg | 0.000 | 0.000 | 0.000 |
| FFV1 | 4:2:0 8-bit, left | BT.601 limited | FFmpeg | 1.009 (14 px, border) | 0.000 | 0.000 |
| FFV1 | 4:2:0 8-bit, center | BT.601 limited | FFmpeg | 3.310 (146 px, border) | 0.000 | 0.000 |
| FFV1 | 4:2:0 8-bit, top-left | BT.601 limited | FFmpeg | 1.009 (15 px, border) | 0.000 | 0.000 |
| VP9 lossless | 4:2:0 8-bit, left | BT.601 limited | FFmpeg | 1.009 (14 px, border) | 0.000 | 0.000 |
| AV1 (SVT, CRF 10) | 4:2:0 8-bit, left | BT.601 limited | VideoToolbox where the hardware decodes AV1, else dav1d | 1.135 (19 px, border) | 0.000 | 0.000 |
| H.264 (VideoToolbox encoder) | 4:2:0 8-bit, left | BT.601 limited | VideoToolbox | 1.009 (14 px, border) | 0.000 | 0.000 |
| HEVC Main 10 (VideoToolbox encoder) | 4:2:0 10-bit, left | BT.601 limited | VideoToolbox | 1.040 (8 px, border) | 0.000 | 0.000 |
| FFV1 | 4:2:0 8-bit, left (tagged) | BT.709 limited, BT.709 primaries | FFmpeg | 1.188 (14 px, border) | 0.000 | 0.000 |
| FFV1 | 4:2:0 8-bit, left | BT.709 **full range** | FFmpeg | 1.133 (14 px, border) | 0.982 (0 px over 1) | 0.000 |
| FFV1 | 4:2:0 10-bit, left | BT.2020 NCL matrix, BT.709 primaries, limited | FFmpeg | 1.096 (14 px, border) | 0.000 | 0.000 |
| ProRes 422 HQ (VideoToolbox encoder) | 4:2:2 10-bit, left | BT.709 limited | VideoToolbox | 0.739 | 0.739 | 0.739 |
| HEVC Main 10 (VideoToolbox encoder) | 4:2:0 10-bit, left (tagged) | BT.2020 NCL, BT.2020 primaries, `bt2020-10` | — (routes to the legacy engine: "Bt2020 primaries") | — | — | — |
| `corpus-frames-24.mp4`, 96 frames | 4:2:0 8-bit, left | untagged: BT.601 limited | VideoToolbox | 23.167 (174 px) | 23.167 (174 px) | 0.000 |
| `corpus-frames-30.mp4`, 120 frames | 4:2:0 8-bit, left | untagged: BT.601 limited | VideoToolbox | 25.499 (194 px) | 25.499 (194 px) | 0.000 |

(Max CV over color; "px over 1" counts pixels above 1 CV over all frames
measured. Interior figures, a 2-pixel border left out, equal the
engine's-edge-rule column for every source. Against FFmpeg's VideoToolbox
decode with the same chain, the ProRes rows are 0.000, 0.006, 0.012, and
0.000.)

What the rows show:

- **The engine reads the tags.** BT.601, BT.709, and BT.2020 NCL matrices,
  limited and full range, and left, center, and top-left siting are each read
  as tagged (and untagged values by section 9's rules), and every source the
  engine draws converts exactly with them where the decoders agree (next
  bullets). The fully tagged BT.2020 source is read correctly (BT.2020 NCL,
  limited, left, BT.2020 primaries) and then refused by the renderer
  (`pm-render` `media.rs` `video_supported`: wide gamut sources wait for
  Interpret and wide-gamut projects), so it routes to the legacy engine; no
  corpus project uses one.
- **The chroma filter and siting are exact.** Every source FFmpeg decodes the
  way the engine's decoder does, 4:2:0, 4:2:2, and 4:4:4 with left, center,
  and top-left siting, through FFmpeg and VideoToolbox (and dav1d or
  VideoToolbox for AV1; H.264, HEVC, and AV1 decoding is bit-exact), is within
  0.000 CV of the f64 reference, and of the zimg reference with the engine's
  edge rule wherever zimg does not clip (all but 368 pixels of the corpus
  H.264 frames, and the full-range source; next bullets).
- **zimg's edge extension differs from the engine's** (up to 3.31 CV, only in
  the 2-pixel border; 0.000 inside): a reference construction choice, not an
  engine difference. Judged with the engine's edge rule.
- **zimg clips chroma overshoot.** The corpus H.264 sources put chroma code 14
  beside 240; Catmull-Rom overshoots below code 0 there and zimg's 16-bit
  resize clamps it. The test prints the worst pixel of each source with all
  three readings: `corpus-frames-24.mp4` frame 66 (157, 75), B: engine −57.8,
  zimg −34.6, f64 −57.8 CV; `corpus-frames-30.mp4` frame 85 (157, 69), B:
  engine −57.8, zimg −32.3, f64 −57.8 CV. (An ad-hoc hand check, not in the
  test: at `corpus-frames-24.mp4` frame 15 (157, 94) the upsampled Cb is
  −6.27 in 8-bit codes; the engine's B is −43.8 CV and zimg's −31.2, 12.6 CV
  = 6.27 × 2.018.) The f64 reference, which clips nothing, matches the engine
  to 0.000 CV on all 216 frames. The full-range source's 0.982 CV (under the
  bar, interior pixels only) is consistent with the same clamp: full-range
  chroma reaches codes 0 and 255, and zimg's full-range scaling itself
  matches H.273 (an ad-hoc check, not in the test: a 4:4:4 full-range Cb of
  255 converts to B = 1.426122 in zimg and by H.273). Whether the class's
  reference may clip is a reference choice; the engine follows the plan's
  filter.
- **ProRes 422 HQ fails the class as written: the decoder.** The test
  compares VideoToolbox's ProRes decode with FFmpeg's plane by plane over all
  30 frames (both downloaded as planar 16-bit at the source's subsampling;
  differences in codes of the source's depth). BT.601 4:2:2 10-bit: Y max
  1.00 (0 samples over 1 code), Cb 4.00 (5 over 1), Cr 4.00 (925 over 1).
  4444 12-bit: Y 1.44 (318), Cb 4.00 (2,328), Cr 4.00 (5,133); with alpha Y
  1.44 (415), Cb 1.44 (580), Cr 1.50 (636), A 0.19 (0). The BT.709 422 HQ
  source: every plane within 1 code, 0.739 CV. FFV1 at the same 4:2:2 10-bit
  is exact. Against FFmpeg's VideoToolbox hwaccel decode with the same zimg
  chain the engine is within 0.000 CV (4444: 0.006, with alpha 0.012). The
  engine decodes ProRes with VideoToolbox only (`pm-media` `decode/mod.rs`
  `backends`), so its ProRes 422 frames can be 2 CV from an FFmpeg reference
  at saturated chroma edges; how far depends on the content.

Result: the matrix (BT.601, BT.709, BT.2020 NCL), range (limited and full),
siting, filter, and edge rule pass on sources that exercise them, and the
engine reads every source's tags as section 9 says. The class as written
fails for ProRes 422 HQ by 2.017 CV because the reference decoder is FFmpeg's
and the engine's is VideoToolbox, and against zimg on the corpus H.264 media
(25.5 CV) because zimg clips the Catmull-Rom overshoot the plan's filter
keeps. Owners: `pm-media` (the ProRes decoder) or a plan decision on which
ProRes decoder the class's reference uses; a plan decision on whether the
reference may clip. Not exercised: BT.2020 constant luminance (the probe maps
it to NCL, `pm-media` `probe/color.rs`), full range at 4:2:2 and 4:4:4, and
HDR transfers (refused before drawing). The flat fixtures alone would have
shown none of this.

One observation while measuring, not part of the class: the source time is
clamped to the asset's duration less the end padding (today's
`videoPlaybackTime`, 0.04 s in the viewer; `pm_render::retime::playback_time`),
so at 30 fps the last frame of a file is never drawn (it starts 0.033 s
before the end); the measurement declares assets slightly longer for that
reason. It matches today's viewer by design.

The lanes' own corpus runs at this tip agree with the runner where they
overlap and pin the same differences: `tests/corpus.rs` 256 frames, 0 failed
(3 `composition/metadata` frames listed as known residuals, `composite/blend-modes-alpha`
as composite drift, 6 Posterize frames as WebGL-unstable); `tests/motion.rs` 89
frames, 0 failed, 11 recorded deviations (the motion and video rows above).

### What still routes to the legacy engine

`pm_render::unsupported` on the corpus, per project. "Interactive" is what the
viewer's engine routes today (built-in definitions, no fonts); "with fonts" is
what would still route once the hosts give their renderers fonts
(`docs/engine/text-render.md`, "Engine hosts do not give their renderers fonts
yet"). The job engine (exports, agent stills, `pm-cli render`) carries no
definitions yet (`docs/engine/effects.md`, "Gaps"), so it also routes 26
projects the viewer draws: every `render/fx-*` and `render/tr-*` case,
`groups/transforms-and-styles`, and `layers/studio-cube` ("no definition for
effect …", `phase3a-gate-measurements/parity-render-jobs.json`).

Routed only for text or captions (drawn once the hosts give fonts), 32:
`animation/interpolation`, `captions/box-highlight-pop`, `captions/classic`,
`captions/fade-top-upper`, `captions/middle-lines`, `captions/preset-boxed`,
`captions/preset-classic`, `captions/preset-paper`, `captions/preset-pop`,
`captions/preset-spotlight`, `captions/preset-whisper`, `composition/fps-23_976`,
`composition/fps-25`, `composition/fps-29_970`, `composition/fps-60`,
`layers/null-parenting`, `precomps/nesting`, `retiming/precomp`,
`text/animator-blur`, `text/animator-pop`, `text/animator-pulse`,
`text/animator-range`, `text/animator-rise`, `text/animator-rise-words`,
`text/animator-spring-drop`, `text/animator-tracking`, `text/animator-tumble`,
`text/animator-typewriter`, `text/animator-wave`, `text/properties`,
`text/range-styles`, `text/variable-axes`.

The other 60:

| Project | Interactive | With fonts | Owner |
|---|---|---|---|
| `real/powermove-ad` | 2.5D layers, text | 2.5D layers (2.5D text) | `pm-render` motion and text (2.5D text rasters) |
| `3d/motion-blur` | motion blur | same | `pm-render` motion and scene3d (a pass context per shutter sample) |
| `compositing/motion-blur`, `compositing/motion-blur-shutter-360` | motion blur, text | motion blur (on text) | `pm-render` motion and text |
| `3d/transparency`, `composite/3d-mask`, `composite/3d-blend-mask` | masks (on 3D object layers) | same | `pm-render` composite and scene3d |
| `masks/path` | SVG images, path masks, text | SVG images, path masks | `pm-vector` (SVG level 1), `pm-render` composite (Skia-equivalent path mask coverage and blur) |
| `space/layers-25d` | 2.5D layers, SVG images, text | 2.5D layers, SVG images | `pm-render` motion, `pm-vector` |
| `space/groups-25d` | 2.5D layers, text | 2.5D layers | `pm-render` motion and text |
| `effects/extension-typewriter` | SVG images, no definition for effect typewriter, text | SVG images, the typewriter definition | `pm-vector`; the corpus app had no third-party extension (expected) |
| `layers/svg-scales`, `mattes/nested-and-animated` | SVG images | same | `pm-vector` (level 1 SVG differs from Chromium's Skia by up to 116 CV at edges) |
| 26 more with SVG images and text: `compositing/adjustment`, `compositing/blend-modes`, `compositing/blend-modes-alpha`, `composition/multiple`, `effects/{blur, chroma, color, displace, duotone, glow, gradient, grain, invert, levels, missing-definition, motionblurDir, pixelate, posterize, shadow, sharpen, vignette}`, `groups/solo-and-hidden`, `layers/extension-missing`, `masks/parametric`, `mattes/modes`, `transitions/{crossfade, slide, wipe, zoom}` | SVG images, text | SVG images | `pm-vector` |
| `media/animated-apng-png`, `media/animated-gif`, `media/animated-webp` | animated images and image sequences | same | `pm-media` and `pm-render` (image sequences) |
| `media/still-avif`, `render/still-photo-avif` | AVIF stills | same | `pm-media` (stills classes) |
| `render/still-16bit-png` | 16-bit PNG stills | same | `pm-media` |
| `render/still-p3-heic`, `render/still-p3-jpg`, `render/still-p3-png` | Display P3 stills (out-of-gamut handling differs) | same | `pm-media`, `pm-color` |
| `render/still-photo-{420, 422, 422-tall-mcu, 444, 444-tall-mcu, progressive}-jpg` | Y′CbCr JPEG stills (decoder differs from libjpeg-turbo) | same | `pm-media` |
| `render/still-photo-alpha-webp`, `render/still-photo-lossless-webp` | WebP stills with alpha | same | `pm-media` |
| `render/still-photo-orientation-6-jpg` | oriented stills | same | `pm-media` |

The static check stays conservative: no feature was found drawn without a
parity case, and the determinism rerun found no frame that renders differently
the second time.

### Text class

`tests/text_corpus.rs` (whole frames, fonts from the CoreText registry with the
capture's font-cache seeds): every corpus project with text or captions that is
not routed for another feature, 31 projects, 123 frames: text pixels SSIM min
0.9890 (≥ 0.98), every other pixel exact (max 0 CV). Pass.

`tests/text_raster.rs` (level 1 rasters against Canvas2D, ink SSIM): 495 cases,
80 pinned below 0.98 by the text lane as gaps: text at 18 px device size and
below (sizes groups min 0.925), synthesized bold (synthetic min 0.929),
emoji below 10 px (min 0.962), and some weights and families at small sizes
(weights min 0.871). Captions: every preset and state ≥ 0.985. These are
failures of the text class as the plan writes it. Owner: `pm-text` (the
Canvas2D stem-darkening profile for small sizes, amendment 10).

`tests/text_level2.rs` (level 2 against CoreText with smoothing): 400 cases and
20 layers, 0 below their gate (min 0.9799 pinned, layers min 0.9907).

Line breaks and advances (level 1): native `pm-eval` 0 line-break differences
and 0 advances over 0.25 px on the corpus; through `pm-wasm` 1,260 `system-ui`
layouts of the real project are off by up to 18.2 px (`pm-cli parity eval
--wasm`; Phase 1 gate T1, the font-cache seeds, still open). Owner: `pm-wasm`
and `pm-text` with Jude's T1 decision.

Text review (needs Jude): sheets regenerated at this tip with `cargo run
--release -p pm-render --example text_review`: 420 cases (rows: today's
Canvas2D, engine level 1, CoreText with smoothing, engine level 2),
`~/Library/Application Support/Powermove Engine Corpus/review/text-1792297672/index.html`
(supersedes `text-1792218705`). One thing to know while reviewing: CoreText
logs that a request for the `.SFNS-Regular` name falls back to Times New Roman
(28 times while writing the sheets, 14 in `text_level2.rs`), so a CoreText row
of a `system-ui` case may show Times; the level 1 rows are not affected. Owner:
`pm-text` (its CoreText reference should create the system font through
`CTFontCreateUIFontForLanguage`).

### 3D and Blender classes

| Test | Measured | Result |
|---|---|---|
| Corpus, 3D pixels (`pm-cli parity render`) | 21 projects, 59 frames with 3D objects: 100% of 3D pixels within 2 CV, max 2 CV, max 1 CV outside MSAA edges, object SSIM min 0.99993 | pass |
| `tests/scene3d.rs` | 31 scenes, 74 outputs against three.js r180 through the production `SceneRuntime`: every output 100% within 2 CV, SSIM ≥ 0.99984; 265 draws in three.js's order, 23,831 uniform values equal as f32 | pass |
| `tests/scene3d_gltf.rs` | 165 outputs (S7 models and 105 Khronos samples, several times each): worst ≤ 2 CV fraction 99.934%, max outside edges 6 CV (bar 12), min object SSIM 0.99973; 3 Khronos files refused by both importers | pass |
| `tests/scene3d_mesh.rs`, `scene3d_layering.rs`, `scene3d_document.rs`, `scene3d_resources.rs`, `three_programs.rs` | Legacy OBJ meshes max 1 CV; 80 seeded compositions' depth sets and order equal today's; programs byte-identical to three.js's | pass |
| `tests/blender_composite.rs` (Blender surfaces) | Two layers of 16-bit surfaces, crops, 50% opacity, against an f64 reference: 32-bit max 6.8e-8, 16-bit max 1.06e-3 | pass (exact at 32-bit within f32 rounding) |
| `tests/blender_snapshots.rs` (Blender snapshots) | 158 snapshots: 135 equal (11,274,896 geometry values, max 3.5e-16; 9,622 other values exact; asset bytes equal), 23 not built (`3d/gltf` and `3d/gltf-materials`: imported glTF models are not built into snapshots); 74 of 74 frame lists equal | **fail** for the 23. Owner: `pm-render` Blender (`docs/engine/blender.md`, "Gaps": imported models and recipe parts) |

## Shader gate (section 11, amendment 9)

`tests/shader_gate.rs` on corpus `shaders-20261018-022611` (collected by the
shaders lane on 2026-10-18 from a fresh run: the repository, the 4 public Store
releases fetched without credentials, docs samples, cloud samples, scenes,
unit-test and e2e literals, S3's 56 agent shaders, and 35 agent-generated
answers to 37 contract prompts; WebGL2 references from Chromium on ANGLE Metal,
and the fallback runner's pixels), rendered by this tip's production path.
Collected again from this tip (`collect.ts --out` with the same Store fetch,
compared with `engine/crates/pm-cli/scripts/shader-corpus-same.py`; repeated
2026-10-18 00:28): the same 144 cases and 522 variants, no case changed, the
same 4 Store releases, so the WebGL references stand.

| Group | Variants | Match WebGL | Via the runner | Both refuse | WebGL-unstable | Failures |
|---|---:|---:|---:|---:|---:|---:|
| builtin-layers | 6 | 6 | 0 | 0 | 0 | 0 |
| cloud-samples | 20 | 20 | 0 | 0 | 0 | 0 |
| docs | 11 | 11 | 0 | 0 | 0 | 0 |
| e2e | 37 | 37 | 0 | 0 | 0 | 0 |
| generated-agent | 112 | 105 | 3 | 7 | 0 | 0 |
| generated-s3 | 212 | 204 | 0 | 8 | 0 | 0 |
| scenes | 8 | 8 | 0 | 0 | 0 | 0 |
| shader-presets | 15 | 15 | 0 | 0 | 0 | 0 |
| store | 22 | 19 | 0 | 0 | 3 | 0 |
| unit-tests | 79 | 79 | 0 | 0 | 0 | 0 |
| **all** | **522** | **504** | **3** | **15** | **3** | **0** |

`tests/effects_webgl.rs` (corpus `glsl-20261016-041029`): the 16 built-in
effects in 176 variants and the 4 transitions in 52 match WebGL; 219 generated
effect variants, 211 match and 8 refused by both; the 394-probe coverage sweep:
260 match, 122 refused by both, 12 known open (8 non-`sampler2D` samplers, which
route to the fallback runner per amendment 9; `out vec3` with GL's undefined
alpha; two ternaries on structs and arrays that WebGL refuses and the engine
accepts). The app captures of every shader and extension project compare in
`tests/corpus.rs` (above).

Pass as measured, with three items for review: the Store's
`mrsigma/liquid-glass-ios27` 2.0.8 (43 to 49 pixels up to 99 CV; WebGL against
itself under a one-step input perturbation moves 26 to 33 pixels up to 67 CV),
and the built-in Color, Levels, and Posterize effects, WebGL-unstable at the
shader level (Posterize's corpus frames are the six review frames above). Open
items outside the pass line: the two ternary probes (the engine accepts
programs WebGL refuses; owner `pm-glsl`), and the fallback runner for effects
and transitions (a definition only the runner could draw still routes; owner
`pm-render` shaders and effects). The in-app agent harness did not generate the
agent corpus (it needs a running app with its MCP tools); the same model and
CLI ran with the harness's instructions (`docs/engine/shaders.md`).

## Section 2 bars

| Bar | Measured | Result |
|---|---|---|
| Color invariants with linear blending on: (1) | `tests/color.rs`: H.264 4:2:0 → `420v` 259,200 samples, luma and chroma 0 CV; ProRes 4444 XQ → `y416` 688,500 samples and ProRes 422 HQ → `v216` 345,600 samples, 0 differ, with linear blending on and off | pass |
| Color invariants with linear blending on: (2) | 512 solids: sRGB PNG and 10/16-bit RGB 0 CV, 8-bit limited Y′CbCr ≤ 2 CV, 10- and 12-bit ≤ 1 CV, greys neutral, with linear blending on and off | pass |
| Linear blending: 50% white over black | 188 (0.735357) with linear blending, 128 without (`tests/linear.rs`) | pass |
| Linear blending: 1-pixel checkerboard downscaled 2× | 188 | pass |
| Linear blending: a Gaussian blur of a white line conserves energy within 0.5% | New measurement (`pm-cli` `tests/phase3a_bars.rs`, 32-bit, the built-in blur at amounts 2, 8, 24, 64). The blur on the line's own layer, composited over black: energy 1.000000 of the sharp line at every amount (the blur spreads coverage, and coverage composites in linear light). The same picture blurred by an adjustment layer over an opaque black solid: 0.235 to 0.077 of the sharp line (−76.5% to −92.3%), identical to linear blending off, because legacy GLSL receives E-encoded values (section 11) | **fail** (the stack form). The plan's two rules conflict for `compat` level 1 effects; a level 2 blur (section 11, "visual fixes … apply only to compat level 2") would decide it, and no level 2 effect path exists. Owner: `pm-render` effects, with the plan owner |
| Antialiasing (`compat` level 2) | `tests/sampling.rs`: zone plate and Siemens star under scale 1.0 → 0.25 and rotation, against each path's declared kernel at 16×: final (Lanczos) PSNR min 70.78 dB (zone plate, 16-bit) and 80.57 dB (star); playback (trilinear) min 54.75 dB; flicker 0.996× to 1.000× (≤ 1.5×). Level 1 keeps today's sampling (corpus parity above) | pass |
| 3D | See "3D and Blender classes" | pass, except the 23 Blender snapshots |
| Determinism: two renders identical | `pm-cli parity render --determinism`: 289 corpus samples rendered a second time after dropping the renderer's caches (`trim(0)`), 0 differ bit for bit (including 21 3D projects, effects, motion blur, video); `tests/headless.rs` "every feature is supported and frames are deterministic" pass | pass |
| Determinism: lossless outputs | `pm-cli render --format png16` of the first second of 9 corpus projects (`3d/primitives`, `3d/lights`, `3d/gltf`, `shapes/primitives`, `layers/solid`, `backgrounds/radial`, `animation/transforms`, `media/h264-aac-mp4`, `media/prores422-mov`), twice each: 270 16-bit PNG frames, every file byte-identical (`engine/crates/pm-cli/scripts/png16-determinism.sh`; repeated 2026-10-18 00:29 with the same result). The two media projects use the flat e2e fixtures, so their 30 frames are each one colour (`media/prores422-mov`'s frames are identical to each other); they show the decode path deterministic, not a varied picture | pass |
| Determinism: native and WASM evaluation | `pm-cli parity eval --wasm`: 1,815,420 replies of pm-wasm compiled natively and the wasm32 module in Node, 0 differ | pass |

## Performance rows (section 25, Phase 3a)

App rows: `e2e/native-engine-p3a-measure.spec.ts` with Use New Engine on, hidden
and unfocused, one Electron at a time (summary
`phase3a-gate-measurements/app-rows.json`); engine rows: the gate script.
Load average 3.2 to 12.6 during the app runs. The main display was found
asleep with the session locked at 22:31 and at 23:56 (the drag runs' check);
the UHD runs (22:17 to 22:31, 23:34 to 23:56) did not record the display
state, so their display intervals may be Chromium's estimates; the frames
counted are the frames the presentation handed over, which the display state
does not change.

| Row | Gate | Measured | Result | Owner |
|---|---|---|---|---|
| Layer drag, 884-layer project | p95 ≤ 33 ms, at most 1 frame worse than today | Not measured (display asleep, session locked; the harness skips such runs). The project routes to the legacy engine (2.5D text, text). Two harness runs are added for it (`p3a-drag-today-884-idle`, `p3a-drag-engine-884-idle`); the harness no longer built at this tip (`src/main/engine/blender.ts` imports `worker.py?raw`, which its esbuild had no loader for), fixed here | **not measured** | text and 2.5D routing first (`pm-render`, `pm-text`); then a run with the display awake |
| 3D pick, 1M-vertex skinned model | ≤ 2 ms at an unchanged frame; ≤ 33 ms first pick after a time change | `pm-geom3d` `gltf_picks.rs` (release): load 51 ms, BVH 126 ms; first pick after a time change median 14.15, p95 15.08, max 15.48 ms; unchanged frame median 0.002, max 0.016 ms | pass for the engine's picker; the app still picks through three.js (`pm-eval`'s `LayerHost` hooks are wired in Phase 3c) | `pm-eval` and the hosts (3c) |
| Canvas text typing | p95 ≤ 33 ms | Engine share (`examples/text_typing.rs`: layout, scene, coverage, paint, composite at 1920 × 1080, GPU finished, 71 keystrokes): 96 px title p95 1.70 ms, 48 px paragraph p95 3.50 ms, 36 px caption size p95 1.93 ms. In the app a text layer routes to the legacy engine ("text layers without the engine's fonts"), so typing is not drawn by the engine | **fail**: not drawn by the engine | `pm-node` and `pm-text` (fonts for the hosts' renderers) |
| UHD ProRes 422 HQ, 3 layers with blur, Pro | Real time at 23.976, 29.97, 59.94; zero drops over 60 s; same with linear blending on | Three 3840 × 2160 ProRes 422 HQ layers (12 s clips from VideoToolbox's encoder with noise, 0.78, 0.97, and 1.94 Gbit/s; three different sections of the clip, so three decodes per frame), each with Blur at 24 and the upper two at 60% opacity, a composition of 96, 120, or 240 frames looping for 60 s, drawn by the engine, clips copied into the media store as an import does. 23.976: 1,438 frames shown, 0 skipped, 0 repeated, display interval p50 41.5 ms, max 51.6; 29.97: 1,799 shown, 0 skipped, 0 repeated, p50 33.2, max 41.5; presenter drops 0, engine late-skips 0 (the counter covers the interactive engine's whole session, not only the 60 s window), compose mean 1.7 to 1.8 ms, compose max 197 to 212 ms in every run (the engine keeps only the maximum, so which frame it was is not recorded; probably the first frame with cold decoders and pipelines, not verified). 59.94: the whole-frame run did not complete (the 2.9 GB clip's import timed out at 15 minutes with the disk at 4 GB free); the earlier run with a 4 s composition (239.8 frames) showed 3,597 frames with 4 skipped, every one frame 0 after a loop wrap (the loop length alone predicts 3.6 over 15 loops; the 23.976 and 29.97 runs of that form skipped 1 and 2 the same way, as predicted, and 0 with whole-frame loops), display interval p50 16.6, max 24.9 ms, compose max 209 ms, 1 late-skipped by the engine (a session counter, so possibly outside the 60 s window). With "Blend in linear light" on, the project is stamped `requiresEngine: "native"` and routes to the legacy engine ("native-only features") at every rate | 23.976 and 29.97: pass with linear blending off. **Fail**: the linear-blending half is drawn by the legacy engine; 59.94 not cleanly measured (1 engine late-skip in the run that completed) | Linear: `pm-render` `capability.rs`. 59.94: a rerun with disk to spare (Parity and QA) |
| UHD ProRes 422 HQ, 29.97 on base | Real time | Base machine | **deferred** | |
| The 884-layer project: the five ceilings | The five ceilings that fail today pass | `editor-responsiveness.spec.ts:6`, `high-zoom-performance.spec.ts`, `region-pan-performance.spec.ts` with the perf project copy: Use New Engine on: random scrub render p95 5.1, max 14.7 ms (ceiling 100); high-zoom wheel 1 render (ceiling 15); region pan 3, 5, 5 renders (ceiling 25). Off: p95 5.6, max 14.9 ms; 1; 2, 5, 5. All five pass in both builds today (they failed when the plan was written; `apps/desktop/docs/memory-investigation.md`). The project routes to the legacy engine, so both runs measure today's compositor (`PM.GL.render`) | pass as measured, not an engine measurement | the specs read the legacy compositor; engine equivalents follow the routing |
| Memory | Within leases; no growth over a 1-hour playback loop; `pm-wasm` budget with the 3D corpus | The spec's memory test (`PM_P3A_MEMORY_SECONDS=3600`): the synthetic 1,000-layer project (2 images) the engine draws, looping for 3,600 s; 107,965 document frames, 0 late-skipped, 0 render errors, 0 layer failures. Every 30 s: `app.getAppMetrics()` and each engine's last lease usage. Lease usage 623,423,488 bytes (frame cache 512 MiB, surface pool 82.5 MiB) against a 58.4 GB interactive lease at all 121 samples (the job engine 0 of 10.3 GB). Resident memory, interactive engine (the larger Node utility): 282 MiB at start, 357 at 5 min, 378 at 20, 384 from 30 to 60 min; job engine 72 → 81; editor page 377 → 566 at 5 min, 576 to 578 after; GPU process 138 flat; page JS heap 42 to 65 MB, no trend. `pm-wasm` (`wasm-budget-3d.mjs`): the 18 3D corpus projects (and the OBJ and Studio Cube layers) hold at most 0.21 MB live heap, 0.25 MB peak, 1.97 MB linear memory with every layer query at every sample time; the real 884-layer project 11.3 MB live, 14.1 MB peak, 25.5 MB linear. Every reply's kind is counted (rerun 2026-10-18): of 1,078,013 queries, 1,076,993 answer `ok`, 0 `error` or `throw`; every `active`, matrix, opacity, `resolveContent`, 3D, and `ev` query answers, but 1,020 of the 1,280 `pick` queries are unanswered: 960 `host` (the 3D corpus's picks, which only today's 3D runtime answers until 3c) and 60 `fonts` (the real project's picks that meet text). An unanswered query does no work in the module, so pick memory is not measured | Leases: pass. Growth: the engine plateaus after 30 minutes (+27 MiB from minute 5 to 30, 0 after); pass by the last half hour, a rise to watch. `pm-wasm`: the 3D corpus is within S5's proposed budget (live ≤ 8 MB, linear ≤ 32 MB); the real project's live heap is over the proposal, which Jude has not confirmed. pm-wasm holds no 3D meshes or BVHs yet (3D queries stay on three.js until 3c), so the plan's 3D share is not measurable | `pm-wasm` (the real project's mirror heap); the 3D share with 3c |

**Linked media.** A first run linked the clips instead of copying them
(`PM.HostMedia.setProjectStorage('link')` before the import): once main's
rendition for the legacy viewer had finished, the engine routed the
composition with "media offline: prores422hq-2160p-23976.mov" (summary
`linkedRun`). The copied clips (an import's default) were drawn. Not
investigated further here; owner: Proxies and storage (`MediaStore.resolveAsset`
for linked originals) with the media lane.

**`requiresEngine` routing.** `pm_render::capability::check` lists
`Feature::RequiresEngine` for any document stamped `requiresEngine`, and the
presenter routes such a composition to the legacy viewer. The stamp
(`shared/engine-stamp.ts`) marks exactly the projects that need the new
engine: linear blending or 32-bit project precision, linked media,
interpretation overrides, clip pitch. So linear blending, which the renderer
draws (`tests/linear.rs`, `tests/color.rs`, `tests/effects_linear.rs`), never
reaches it in the app, and neither does any project once the stamp is
applied for linked media or an interpretation override. Owner: `pm-render`
(`capability.rs`; the check predates the native-only features), then a
corpus case per stamped feature.

## Measurement code added

- `engine/crates/pm-cli/src/render_parity.rs`, `corpus.rs`, `main.rs`:
  `pm-cli parity render` now holds the built-in definitions the kernel pushes
  (third-party extension layers of a capture marked missing, as the capture
  app had them), so effects and transitions are compared instead of routed
  (`--no-definitions` restores the job engine's routing); `--real <file.pmv>`
  adds each capture's real-world project with its media unpacked from the file
  the capture opened (checked by SHA-256); `--determinism` renders every sample
  again after `Renderer::trim(0)` and requires bit-identical float buffers;
  a failing frame of a project with a WebGL-unstable effect (Posterize) is
  listed for review only when its difference is confined to the pixels the
  effect changes (rendered again without it);
  a project routed for text also reports what would still route with fonts.
  Unit tests: option parsing, real-project listing and naming, the unstable
  effect walk, stripping, and the confinement rule. Not unit tested:
  `capture_definitions` and `routing_with_fonts` (they need a capture; the
  corpus run exercises them).
- `engine/crates/pm-cli/tests/phase3a_bars.rs`: the blurred-white-line energy
  line; `tests/phase3a_video_class.rs`: the video class against FFmpeg plus
  zimg, with matrix, range, and siting from ffprobe's tags (the engine's
  reading asserted equal), on the flat fixtures, 17 generated chroma-edge
  sources (BT.601, BT.709 limited and full, BT.2020 NCL, BT.2020 primaries),
  the VideoToolbox against FFmpeg ProRes plane comparison, and the corpus
  H.264 media (both ignored by default; run with `--ignored`).
- `engine/crates/pm-cli/scripts/wasm-budget-3d.mjs`: the pm-wasm mirror's
  heap and linear memory with each 3D corpus project and the real project;
  every reply's kind is counted per op (`ok`, `error`, `throw`, and the
  unanswered `host`, `fonts`, `ink`, `legacy`). Rerun: 0 errors or throws in
  1,078,013 queries; 1,020 of 1,280 picks unanswered (960 `host`, 60
  `fonts`); the same memory figures.
- `engine/crates/pm-cli/scripts/png16-determinism.sh`: the lossless-output
  determinism row (nine projects exported twice, compared byte for byte).
- `engine/crates/pm-cli/scripts/shader-corpus-same.py`: compares a shader
  corpus collected again at the tip with the one the WebGL references came
  from (rerun at this tip: 144 cases, 522 variants, none changed, same 4 Store
  releases).
- `engine/crates/pm-cli/scripts/phase3a-gate.sh`: runs every engine-side step
  of this report at low priority and writes logs outside the repository.
- `apps/desktop/e2e/native-engine-p3a-measure.spec.ts` (outside the owned
  paths: Playwright specs live in `e2e/`; a new file, opt-in with
  `PM_P3A_MEASURE=1`): the UHD ProRes row, the canvas text route, and the
  memory loop.
- `apps/desktop/scripts/frame-bridge-measure.mjs` (the bridge lane's harness):
  two run entries (`p3a-drag-today-884-idle`, `p3a-drag-engine-884-idle`),
  a per-run summary folder, and an esbuild plugin for Vite's `?raw` imports,
  without which the harness no longer built (`src/main/engine/blender.ts`).

## Reproducing

```sh
source engine/env.sh
engine/crates/pm-cli/scripts/phase3a-gate.sh /tmp/phase3a-gate          # engine side, about 25 minutes
#   it includes the png16 determinism row (png16-determinism.sh) and the shader corpus
#   recollection (tools/shaders/collect.ts --out, then shader-corpus-same.py)
engine/target/release/pm-cli parity render --help                         # the runner's options
bun run --cwd apps/desktop build:engine && bun run --cwd apps/desktop build:wasm && bun run --cwd apps/desktop build
# UHD clips (outside the repository; about 5.5 GB, and each import copies its clip
# into the test profile), our LGPL FFmpeg with VideoToolbox's ProRes encoder:
ffmpeg -f lavfi -i "testsrc2=size=3840x2160:rate=30000/1001,noise=alls=12:allf=t" -t 12 \
  -c:v prores_videotoolbox -profile:v hq -pix_fmt p210le \
  -color_primaries bt709 -color_trc bt709 -colorspace bt709 prores422hq-2160p-2997.mov
cd apps/desktop
PM_P3A_MEASURE=1 pm-electron-lock bunx playwright test e2e/native-engine-p3a-measure.spec.ts --workers=1 --global-timeout=0
PM_P3A_MEASURE=1 PM_P3A_MEMORY_SECONDS=3600 pm-electron-lock bunx playwright test e2e/native-engine-p3a-measure.spec.ts -g memory --workers=1 --global-timeout=0
pm-electron-lock node scripts/frame-bridge-measure.mjs --only p3a-drag-today-884-idle,p3a-drag-engine-884-idle --tag p3a
PM_PERF_PROJECT=<the perf project JSON> POWERMOVE_NEW_ENGINE=1 pm-electron-lock bunx playwright test \
  e2e/editor-responsiveness.spec.ts:6 e2e/high-zoom-performance.spec.ts e2e/region-pan-performance.spec.ts --workers=1
cargo run --release -p pm-render --example text_review                    # the text review sheets
```
