# Motion blur, 2.5D, retiming, level 2 sampling, scopes

The time and space features of Phase 3a in the native renderer (plan sections 5, 8,
10, and 25; amendment 16). Code: `engine/crates/pm-render/src/motion/`,
`src/space25d/`, `src/sampling/` (with `shaders/resample.wgsl` and
`shaders/mips.wgsl`), `src/retime.rs`, and `engine/crates/pm-cli/src/scope.rs`. The
plan stays the authority; this file records what was built, how it matches today's
look, what was measured, and what is left.

## Motion blur

Today's compositor (`legacy/gl/compositor.ts`, `renderProject`) blurs a layer whose
`mblur` switch is on (animatable) by drawing it `n` times into its own cleared
`RGBA16F` target at `T + ((s + ½)/n − ½) × shutter / fps`, each draw at
`opacity / n` with additive blending (`ONE, ONE`), and then drawing that target
over the stack at opacity 1. A group renders its members' compositing pass at
every sample time with motion blur off inside (`{groupParent, mblur: false}`),
accumulates the passes at `1/n`, and draws the sum at the group's opacity.

The engine builds the same graph (`motion::plan`): one
`plan::Source::Accumulate` per blurred layer, whose draws are the samples. The
executor accumulates them with additive blending into a target of the frame's
working format, so the sum is kept at float precision in the active encoding
(`rgba16float` as today's target, or `rgba32float` at 32-bit precision), with no
adaptive reduction. Each sample is its own pass and its sources (nested
compositions, group passes) go back to the target pool as soon as the pass is
recorded, so a blurred precomp holds one sample's targets at a time, as today's
`free(sample)` does (`blurred_precomps_hold_one_sample_at_a_time`).

Rules carried over, each checked against the captures below:

| Rule | Today | Engine |
|---|---|---|
| Sample count | `opt.mbSamples \|\| 10` | `RenderRequest::motion_blur`: `MotionBlur::Kind(kind)` reads the root composition's `motionBlurSamples[kind]`, else today's count; `MotionBlur::Samples(n)`; `Off` |
| Counts by output | opaque export 16 (`renderOpaqueFrame`), alpha and WebM export 20, viewer 6 playing and 12 paused | `export` 16, `alphaExport` 20, `previewPlaying` 6, `previewPaused` 12; WebM exports pass `alphaExport` (contract 2 has no WebM kind) |
| Shutter | `PM.proj.shutter \|\| .5` frames of the open composition; 0 means half a frame | `composition.shutter \|\| 0.5` of the rendered root, kept for nested compositions |
| Rate | `proj.fps` of the composition at each nesting level | the same (`motion::level_fps`) |
| Offsets | `((s + .5) / n − .5) × shutter / fps`, in that order | bit-identical expression (`motion::sample_offset`), equal to contract 1's `Shutter::from_frames(0.5)` within 1e-15 |
| Opacity, active range | evaluated at the center time; samples do not re-check the layer's range | the same |
| Groups | members sampled with blur off inside; a blurred group is a compositing boundary only while blur applies | the same (`group_creates_compositing_boundary(…, blur_on, …)`) |
| Precomps | a blurred precomp renders its child at the child time of each sample; layers inside blur again with the child's rate | the same |
| Video in samples | see below | the frame selected at the center time of its level, the transform moving |

Every layer type is blurred (on the layer, inside a blurred group, inside a
blurred precomp): shapes, paths, and text re-rasterize at each sample's density
and content time; a blurred 3D object layer draws the composition's scene rebuilt
at each sample time (`Pass3d::at_time`: today's `compositionRuntime(PM, T + dt)`
syncs objects, lights, and camera at the sample time, and `compositionDepthIds`
reads activity and opacity then, over the pass ordered at the center time); a
blurred layer that also needs its own target (masks, effects, matte, blend mode,
transition) takes the accumulated samples as its layer target and the masks,
effects, and matte apply at the center time, as today's slow path does.
Measured (compclose capture): `motion/blur-vector`, `motion/blur-slow` bit-exact
in float frames at 20, 3, and 6 samples, opaque within 1 CV; `motion/blur-3d`,
`motion/blur-3d-group`, and v1 `3d/motion-blur` within the 3D class;
`motion/blur-text` within the text class. Generated layers (compclose3 capture
`compclose3-20261018-174842-bcfa91ad`): `motion/blur-generated` (a translucent
shader layer, an extension fragment layer moving and spinning, a shader in a
blurred group, a shader and an extension in a blurred precomp) within 1 CV
opaque, 0.25 CV in float frames; `motion/blur-captions` (captions blurred, in a
blurred group, in a blurred precomp) within the text class, every other pixel
exact; `composite/path-masks-more` (a blurred path-masked layer) bit-exact.

`motionBlurSamples` (section 6, additive) is an object keyed by contract 2's
`MotionBlurKind` names: `{"export": 16, "alphaExport": 20, "previewPlaying": 6,
"previewPaused": 12}`. A finite number ≥ 1 is floored and capped at 256; a missing or
invalid key keeps today's count.

Wiring: `RenderRequest::from_frame_request` maps contract 2's `motion_blur`;
`pm-export` passes `Export` or `AlphaExport` when the export's motion-blur setting is
on; the viewer's `Composer` (pm-node) passes `PreviewPlaying` for timed frames and
`PreviewPaused` otherwise (`Composer::set_playing`).

### Video in blurred samples (a legacy defect)

Today's capture path (`renderToPixels`, which `renderOpaqueFrame` and the alpha
exporter also use) draws a motion-blurred video layer from a stale, unseeked frame:
frame 0 in the corpus capture. `renderToPixels` sets `useVideoPreviews = false`
(`compositor.ts:1862`), so `contentQuad` reads the shared asset element `a.el`
(`compositor.ts:958`). Frame preparation stores its frames under `L.id + '@' + T`
for the center time only, so no prepared frame exists for a sample time `T + dt`,
and each sample draws whatever frame is uploaded for `a.el`. In the capture
`motion-20261016-085932-b52905bb`, `motion/blur-video` at t = 0.5 and t = 1.2 draws
both blurred clips (the moving one on `corpus-frames-30.mp4` and the static one on
`corpus-frames-24.mp4`) showing frame 0 (timecode 00:00:00.000, counter 0), while
the unblurred clip shows the correct frames (00:00:00.500 / 12 and 00:00:01.167 /
28). `tests/motion.rs` checks this: the capture's blurred static clip matches the
engine's unblurred frame 0 within 2 CV over 8969 interior pixels at both times,
and differs from the center frame by 255. `PM.prepareFrame(t)` does seek `a.el` to the center time before the render;
the texture for that element is versioned by its presented-frame count
(`videoTextureVersion`), not by its time, which is the likely reason the seek is not
picked up (inferred, not traced further). Each sample also calls
`seekPreviewVideo(a.el, vt, .0005)` with its own sample time when not playing
(`compositor.ts:979`); that seek is asynchronous, and the version does not pick up
its result while the render runs. Outside the capture the frame shown is
whatever was last uploaded for that element, so it depends on history.
The same path runs for a video layer inside a blurred precomp (or a precomp inside
a blurred group): `prepareFrame` visits nested precomps and stores their video
frames at the child center time only (`frame-preparation.ts`), while the child
renders at each sample's child time with the same `L.id + '@' + T` lookup, so this
deviation applies there too. Video and image are
compared types, so such layers are not routed by `pm_render::unsupported`; no
corpus case covers them yet (`motion/blur-precomps` holds only solids).
The engine draws the frame selected at the center time (what the viewer shows at
the playhead) with the transform sampled across the shutter.

**Today's export and preview, measured** (`e2e/legacy-blurred-video-export.spec.ts`,
legacy engine, a 64² VP9 clip whose frames are flat levels 20 + 10i, 20 frames
at 30 fps, MP4 export with motion blur on):

- the sharp export holds each frame (levels 20, 30, …, 210, within 1 code
  value of H.264);
- **the blurred export holds frame 0 in every frame** (level 20 twenty times):
  users' exports today contain the stale frame;
- the paused viewer shows the right frame with blur on (frames 3, 8, 15: levels
  50, 100, 170, the same as without blur).

So today's preview and export disagree, and the engine follows the preview.
This is an appendix A item for the plan owner, not a parity target the engine
should reproduce: side-by-sides of the corpus capture against the engine at
`/tmp/ne-compclose/evidence/blur-video/side-by-side-t0.5.png` and `-t1.2.png`,
the export levels at `/tmp/ne-compclose/evidence/blur-video/legacy-export-levels.jsonl`.

## 2.5D

`space25d::projected` folds `pm_eval`'s `planeMatrix` (3D switch, Z position,
anchor and scale with their Z components, X and Y rotation, orientation, skew, the
lens of the outermost 3D group or the layer's own `perspective`) with today's
output scale and the content quad's size and anchor, in `drawContent`'s expression
order. The result is a `plan::Content::Placed` with the projective `mat3` the vertex
shader already takes (`w = p.z`), so the rasterizer clips what is behind the camera
plane and interpolates texture coordinates perspective-correctly, as WebGL does.
`space25d::order` is `compositionOrderedLayers` without the 3D scene pass's camera
sort: runs of 2.5D layers are depth ordered (coplanar layers form one surface in
timeline order; surfaces sort by their first layer's anchor depth), 2D layers and 3D
object blocks are barriers. The 3D scene lane's `order_pass` does the same for
non-model blocks and supersedes this call when the lanes merge.

Drawn for every layer type: solids, images, video, precomps, groups, level 2
shapes and editable paths, text and captions, shader and extension layers, and
masked adjustment layers inside 3D groups. Level 1 text keeps today's raster
density (the continuous raster scale of the 2D world matrix, as `contentQuad`
computes it for a 3D layer outside the preview's clipping) and draws through the
plane. **Level 1 shapes on planes stay routed** (`Feature::PlaneShapeRaster`):
the plane magnifies the Canvas2D emulation's pinned MSAA residuals, which v1
`space/groups-25d` shows at its rounded corners (up to 47 CV; composite.md,
Gaps). The engine draws them correctly otherwise (`space25d/vector` was
bit-exact), so the route goes when the vector residuals close. 3D object, light, and camera layers draw in screen space, as today, also
inside 3D groups (`space25d/others`).

**Level 2 content on planes** (`space25d/local.rs`; no legacy reference): level 2
coverage scenes (solids, shapes, paths, plain SVG images) used to draw through the
2D world matrix and ignore the plane. They are now flattened in layer space at the
plane's densest-corner density and every vertex is projected through the
homography (clipped at the camera plane in homogeneous coordinates), so the
projected outline is exact and analytically covered at output resolution
(`tests/space25d_level2.rs`: a flat plane equals the 2D render within 0.005 in
alpha, coverage within 1e-4; a turned plane covers its homography). Level 2 text
and resvg rasters are rasterized in layer space at that density and drawn by the
level 2 resampler.

Measured on `compclose-20261018-121218-230f9731`: `space25d/vector` (shapes, a
path, a masked shape, shapes in a 3D group, a blurred 3D shape) bit-exact in
opaque and float frames when drawn (now routed, above); `space25d/text` within
the text class (SSIM ≥ 0.9975, other pixels exact); `space25d/others` within the
3D class. On `compclose3-20261018-174842-bcfa91ad`: `space25d/generated` (a
shader layer turned in Y, an extension layer turned in X and moving in Z, a
shader and a blurred extension in a 3D group) within 1 CV opaque and 0.25 CV
float; `space25d/captions` (captions oriented in Y, captions in a 3D group)
within the text class, every other pixel exact.

**Bounds and picks against the display** (`tests/space25d_picks.rs`): pm-eval's
`bounds` and `pick` are ports of `GL.bounds` and `GL.pick` checked against the
TypeScript (pm-eval's layer parity vectors, 3D layers of every type among them).
The test closes the loop with what the engine draws: for every 2.5D layer of
`space25d/vector`, `text`, `captions`, and `generated` at every captured time,
each pixel the layer draws (its frame against the frame with the layer at
opacity 0, away from the antialiased edge) is picked as the layer or a layer
drawn over it; what it draws lies inside the region picked as it; and shader and
extension layers, which fill their bounds, draw exactly the picked region's
box. A smoke test of level 2 text on planes (no legacy reference) checks that an
unturned plane draws what the flat layer draws and that a turned one draws
inside its pick region, at two scales.

## Retiming

- **Amendment 16** (`pm-eval/src/retime.rs`, outside this lane, the Phase 1 gate's
  T3 patch): variable speed integrates with the midpoint rule at
  `1 / (max(30, fps) × 2)` seconds, `fps` the owning composition's (`comp.fps ||
  PM.proj.fps`). At the root this is today's step, and the engine matches
  `retiming.ts` bit for bit at 23.976, 24, 29.97, 30, 59.94, 60, and 120 fps over
  400 times each (`engine_retiming_matches_today_at_the_root`). A nested composition
  keeps its own rate whichever composition is open (the allowed difference).
- **Video frames.** The source time is clamped to `[0, dur − 0.04]` as
  `videoPlaybackTime`; `pm-media` selects the last frame with PTS ≤ t + q/2
  (section 5 rule 7).
- **Frame-rate override** (Interpret Footage, `asset.interpret.frameRate`, `[num,
  den]` or a number snapped like composition rates): frame n is shown at n / rate;
  the frame is ⌊t · rate + 1/8⌋ (rule 7 on the overridden grid), clamped to the
  file's last frame (`retime::override_frame`). The interpretation lane owns the
  sheet that writes the field; audio follows in pm-audio.
- **Keep frame rate when nested** (`d.keepFrameRate`): the child composition's time
  is quantized down to the start of its own frame (`n / fps`, amendment 1) before it
  renders (`retime::keep_frame_rate`).

## Level 2 sampling

Level 1 content keeps today's sampling exactly (bilinear, no mipmaps, hard UV edges,
pixel-center coverage); `level1_keeps_todays_sampling` checks that the resampler
setting does not reach it. Level 1 stills are sampled from today's 8-bit texture
(composite.md, "Today's composite drift"). Level 2 raster content (`compat` ≥ 2
image, video, precomp, shader, and extension layers; SVG images, shapes, text, and
solids belong to their own renderers) is drawn by `sampling::gpu`. Level 2 shader
and extension fragment layers render at the density they are drawn at, without
today's 4,096 px cap (level 1 keeps it), up to 16,384 px. Every layer type is
drawn at level 2; level 2 text that `pm-text` does not lay out yet (animators,
range styles, typography anchoring) is a named text reason, not a level 2 route.

- **Geometry.** The draw covers the pixel bounds of the content polygon: the
  layer's quad, or the part of it the UV window fills (`contain` letterboxing),
  clipped at w ≥ 10⁻⁶ in homogeneous space. Each fragment maps its pixel center back
  to the unit quad through the inverse homography; the Jacobian is exact (from the
  inverse, not screen derivatives, which differed between a full frame and a region
  of interest on Metal).
- **Edges.** Coverage is the exact area of the pixel square inside the polygon
  (Sutherland–Hodgman in coordinates relative to the pixel, unit edge normals
  relative to the bounds' origin): within 1.2e-5 of an f64 reference
  (`level2_edges_are_the_exact_pixel_coverage`).
- **Final** (paused frames, exports, captures, thumbnails): Lanczos-3 in the
  principal axes of the mapping (the texel axes for isotropic maps), its support
  widened by the minification (`max(1, σ)` per axis), read from the pyramid level
  that keeps at most 4 texels per kernel unit, normalized, and clamped to the
  minimum and maximum of the texels inside the central lobe (anti-ringing, in
  premultiplied space).
- **Playback** (frames while playing, scrub frames): trilinear over the pyramid with
  the exact gradients.
- **Pyramids** are box-filtered (2×2 mean, edge texels repeated for odd sizes),
  built per frame for video and precomps and kept for the last 16 stills (counted
  in `Renderer::pooled_bytes`, dropped by `trim`).
- **Continuous rasterization for precomps.** A level 2 precomp renders its child at
  the density it is drawn at (the world matrix's column lengths, or the densest
  corner of a 2.5D plane), never below its own size (inner level 1 content keeps
  today's sampling, which aliases when minified), capped at 16384 pixels a side.
  Level 1 precomps keep their own size.

### The antialiasing bar

Section 2: zone plate and Siemens star under animated scale 1.0 → 0.25 and a quarter
turn (16 frames, 256² sources in a 384² frame), against each path's declared kernel
evaluated exactly in f64 on the same 8-bit texels, with the exact pixel-area coverage
(the limit of the bar's 16× supersampling): PSNR ≥ 40 dB (Lanczos), ≥ 34 dB
(trilinear), flicker energy (second temporal difference) at most 1.5× the
reference's. Measured (Apple M4 Max, `tests/sampling.rs`):

| Pattern, path, precision | PSNR min / mean (dB) | Flicker | Against an output-space Lanczos low-pass of the continuous pattern (16× supersampled; not a bar), min / mean (dB) |
|---|---|---|---|
| Zone plate, final, 32-bit | 71.80 / 114.89 | 1.000× | 29.67 / 35.72 |
| Zone plate, final, 16-bit | 70.78 / 78.64 | 1.000× | 29.67 / 35.72 |
| Zone plate, playback, 32-bit | 54.75 / 71.77 | 0.997× | 22.96 / 26.19 |
| Zone plate, playback, 16-bit | 54.75 / 62.51 | 0.996× | 22.96 / 26.19 |
| Siemens star, final, 32-bit | 100.75 / 118.41 | 1.000× | 30.88 / 33.64 |
| Siemens star, final, 16-bit | 80.57 / 84.20 | 1.000× | 30.88 / 33.64 |
| Siemens star, playback, 32-bit | 56.51 / 76.32 | 1.000× | 23.16 / 27.80 |
| Siemens star, playback, 16-bit | 56.55 / 66.83 | 1.000× | 23.15 / 27.80 |
| Zone plate, level 1 (today's sampling), against the Lanczos reference | 17.28 / 31.29 | 0.899× | |
| Siemens star, level 1, against the Lanczos reference | 22.48 / 36.84 | 1.002× | |

The bar is read as "each path implements its declared kernel": the reference is the
declared kernel itself, not an ideal low-pass. Against an output-space Lanczos
low-pass of the continuous pattern (last column) the paths are 23 to 36 dB, and that
reference is itself not ideal at 1:1 (it attenuates content near Nyquist that an
interpolator keeps). The plan owner may want a stricter quality reading.

## Scopes

`pm-cli scope <project.pmv|.json> -o <dir> [--time S] [--composition ID] [--size
WxH | --scale S] [--media ASSET=FILE] [--motion-blur off|export|N] [--transparent]
[--columns N]` renders the frame at 32-bit precision and writes `waveform.png`
(luma, levels −0.1 to 1.1), `parade.png` (R′G′B′), `vectorscope.png` (Cb right, Cr
up, ±0.6, with the 75% and 100% bar targets and the skin-tone line), and
`scope.json` (per-channel and luma minimum, maximum, mean, 0.1/50/99.9 percentiles,
counts below 0 and above 1, NaN counts, the largest chroma and its hue, a 256-bin
luma histogram). `pm-cli scope --frame <capture.rgba32f|frame.png> -o <dir>` measures
a captured frame instead, so the QA lane can compare engine and legacy frames.
Values are working values (premultiplied, E, Rec.709 primaries, unclamped) with
BT.709 coefficients; a transparent frame is measured over black. Exit status as
`pm-cli render` (0, 1, 2 usage, 3 routed).

## Parity corpus and measurements

Cases: `engine/corpus/src/projects/motion.ts` (`motion/*`, `space25d/*`,
`retime/*`), drawn only with layer types the native renderer draws. Video cases use
generated frame-counter clips (`engine/corpus/src/videos.ts`: FFmpeg `testsrc2`,
libx264, untagged, 30 and 24 fps) so the selected frame is visible in its pixels.
`blurSamples` adds transparent float captures at other sample counts
(`frames/t<time>.mb<n>.rgba32f`, listed in the manifest).

```sh
bun run --cwd apps/desktop build
~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  bun engine/corpus/src/capture.ts --only motion/,space25d/,retime/ --label motion --skip-scenarios --skip-real
cargo test -p pm-render --test motion          # PM_MOTION_CORPUS=<capture> to pick one
```

Measured 2026-10-16 against capture `motion-20261016-085932-b52905bb` (Chromium 152,
Apple M4 Max, 16-bit): 89 frames compared (33 opaque at 16 samples, 56 transparent
float frames at 20 samples and at 1, 3, 6, and 12), 0 failures, 11 recorded
deviations:

| Class | Result |
|---|---|
| Standard (23 opaque 8-bit frames without video) | Every pixel of every frame within 1 CV; at least 99.10% of the pixels of each frame exact |
| Alpha (53 transparent float frames without video) | 49 within 1 CV (26 of them exact: every blurred solid, group, precomp, and 2.5D plane at every sample count); the 4 blurred-still frames at 20 samples 1.49 to 1.62 CV (deviation 1) |
| Video (10 opaque and 3 float frames) | 7 opaque frames and 1 float frame within the video class (≤ 2 CV on patch interiors; max 1 and 1.12 CV). 3 opaque and 2 float frames above it (3, 7, 9 CV; 7.19 and 9.09 CV) at a few pixels of saturated chroma edges, in `retime/speed` and the tilted video plane of `space25d/precomps`; in each the same render one frame later differs by 200 to 249 CV, so the selected frames are the same (deviation 2) |
| Retiming | `retime/speed` (eased, expression, and constant speed, trim, a 24 fps source, an eased remap, a freeze, nested compositions at 30 and 24 fps, precomp eased speed): the same frames at every sample |
| Motion blur | Solids, groups (with nested blurred groups), precomps (24 fps inside 30, nested blur, retimed), 2.5D planes, a wide shutter at 24 fps with samples outside the layer range and hold keys, shutter 0: exact floats at 1, 3, 6, 12, and 20 samples |
| 2.5D | `space25d/planes`, `groups`, `blur`: within 1 CV at 8-bit, ≤ 0.12 CV in floats |

Recorded deviations:

1. **Video decode** (above): differences at saturated chroma edges, frame identity
   shown.
2. **Blurred video** draws a stale, unseeked frame in today's captures and exports
   (frame 0; above), while today's viewer shows the right frame. The engine draws
   the right frame: an appendix A item for the plan owner.

The f16 accumulation deviation of blurred stills (1.49 to 1.62 CV at 20
samples) is resolved: its cause was the still's sampling, not the
accumulation (both paths add into `rgba16float` with the same rounding).
Sampling today's 8-bit texture makes `motion/blur-solids` bit-exact at every
captured sample count (1, 3, 6, 12, 20), and `tests/motion.rs` holds it to the
alpha class.

The existing corpus test (`tests/corpus.rs`, v1 and render captures) and `pm-cli
parity render` now request the captures' 16 and 20 samples; 70 frames, 0 failures.
The v1 projects with motion blur or 2.5D stay routed only by their shape and text
layers.

Synthetic tests (`tests/motion.rs`, `tests/sampling.rs`, `tests/retime.rs`): the
accumulation against an exact count of covered samples at 1 to 20 samples (f16
within half an ulp per sample, f32 within 6e-8 per sample), one sample equals the
sharp frame, counts per output kind, determinism and regions of interest (blur,
2.5D, level 2), plane coverage against the inverse homography, planes crossing the
camera plane, the painter sort and coplanar order, per-sample pool reuse,
`keepFrameRate`, frame-rate overrides, the antialiasing bar, exact edges, level 1
unchanged, continuous precomp rasterization, level 2 routing.

## Changes outside the lane

- `engine/crates/pm-eval/src/retime.rs` and `tests/layers_unit.rs`: amendment 16
  (the Phase 1 gate's T3 patch) and its root-parity test.
- `engine/crates/pm-export/src/render.rs`: the export request's motion blur
  (`Export` or `AlphaExport` when the setting is on) and resampler (final).
- `engine/crates/pm-node/src/present/compose.rs`, `render.rs`: the viewer's
  playing state chooses today's preview counts and the playback resampler.
- `engine/crates/pm-cli/src/render_parity.rs`, `render.rs` (shared loader),
  `main.rs`: capture sample counts in parity; `pm-cli scope`.
- `engine/crates/pm-render/src/{plan,exec,pipelines,renderer,request,media,capability}.rs`,
  `tests/{corpus,headless}.rs`, `tests/support/mod.rs`: small hooks (new `Content`
  and `Source` variants, the request's `motion_blur` and `sampling`, accumulation and
  level 2 draws, the frame-rate override in `MediaCache::video`, routing).
- `engine/corpus/src/{capture,page,verify,media}.ts`, `projects/{common,index}.ts`,
  `test/corpus.test.ts`: `blurSamples` captures, the generated videos, the cases.
- Phase 3c closure (lane compclose): `scene3d/layers.rs` (about 130 lines:
  `Pass3d::at_time`, the depth sets factored into `depth_sets`),
  `shader_layers/plan.rs` (level 2 target size), `exec.rs` (level 2 pyramids for
  vector, text, and shader sources), `plan.rs` (about 300 lines: blurred slow
  path, 3D samples, level 2 on planes, path masks), `capability.rs` (every
  routing change of the lane), `media.rs` (level 1 stills sampled as today's
  8-bit texture: a behaviour change for every level 1 still in the Standard
  encoding), `engine/corpus/src/projects/compclose.ts` and `compclose3.ts` (new
  cases), and `apps/desktop/e2e/legacy-blurred-video-export.spec.ts` (today's
  export and preview of blurred video, VP9, H.264, and ProRes sources).

## Gaps

- The cached preview (section 8, "used by every path including … cached preview")
  does not exist in the engine yet; content keys do not include the sample count.
- Level 2 has no legacy reference (new content); the antialiasing bar is read as
  fidelity to each path's declared kernel (see above). Anisotropic minification
  beyond 4 texels per kernel unit along the major axis reads a box-filtered level
  that also blurs the minor axis. Level 2 text and resvg rasters on planes are
  resampled from a layer-space raster rather than projected analytically (a smoke
  test covers level 2 text's placement; resvg rasters on planes have none).
- Level 1 shapes on 2.5D planes stay routed (`Feature::PlaneShapeRaster`, above).
- Video decode differences at saturated chroma edges (up to 9 CV at a few pixels)
  belong to the media lane's decode path; an H.264 file tagged only with a BT.709
  matrix (no primaries or transfer) is read with BT.601 coefficients by Chromium and
  BT.709 by the engine (up to 23 CV on saturated red), an interpretation question
  for the color lane (the corpus videos are untagged).
- WebM exports have no `MotionBlurKind`; the export lane passes `AlphaExport` (20)
  for them.
- Blurred video: appendix A decision (above).
