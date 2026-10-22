# pm-render: design notes

The Phase 2 renderer and color pipeline (plan sections 8, 9, 10, 25, and 26
Phase 2; amendments 7, 8, 11; contracts 1 to 4). Crates:
`engine/crates/pm-render` (render graph, 3D scene pass, capability check,
headless API), `engine/crates/pm-color` (display encoding E, Standard
invariants, chroma, alpha, dither, CICP, cache keys), `engine/crates/pm-geom3d`
(three.js primitive geometry), and the device, target, GLSL, and MSL parts of
`engine/crates/pm-gpu`. This file records what the renderer draws, how it
matches today's look, and what it routes to the legacy engine. The plan stays
the authority.

## What it draws

| Feature | Status | Parity evidence |
|---|---|---|
| Solid layers (`d.radius` ignored, as level 1) | Supported | Corpus `layers/solid`, `render/*`: exact |
| Image layers in the tested still classes below, including PNG16, Display P3, alpha WebP, SDR AVIF, ICC-tagged RGB TIFF, and container transforms as Chromium shows them (EXIF in PNG/JPEG/TIFF, HEIF and AVIF `irot`/`imir`/`clap`, WebP EXIF ignored) | Supported | Corpus `render/still-*` (detailed content): ≤ 1 CV, 100% within 1, transparent frames below 1 CV; `media/still-*` fixtures: exact; 59 oriented, transformed and AVIF color-description app cases (`e2e/native-still-orientation-parity.spec.ts`): max ≤ 1 CV |
| Translucent P3/Adobe ICC; BMP, ICO, static GIF, progressive gray and RGB JPEG, sub-8-bit/gamma PNG (level 2 reads gAMA as ImageIO does), late PNG EXIF, strip palette/raw float32 TIFF, 10-bit SDR 4:2:0 AVIF | Supported in measured classes | 20 rotated/filtered app compositions max 1 CV; ICC upload exact, float max < 0.125 CV; `media.md` |
| HDR/chromaticity PNG, CMYK/12-bit/lossless JPEG, tiled/compressed float/other-model TIFF, other AVIF structures | Routed | Unclosed decoder classes and app reachability in `media.md`; `tests/stills.rs` |
| Video layers (pm-media decode service, zero-copy decode-in) | Supported for progressive, square-pixel, unrotated, SDR sources with Rec.709 or BT.601 primaries | Corpus `media/*`, `retiming/video`, `render/video`: within 1 CV on patch interiors (video class) |
| Transforms (position, anchor, scale, rotation, skew, parenting, size anchors), opacity | Supported, today's sampling (no mipmaps, bilinear, hard UV edges), shared quad edges | Corpus `render/transforms`: ≤ 1 CV, 100% within 1 |
| Normal blending (premultiplied source-over) | Supported | All of the above |
| Precomps (own size, retiming, `PC_MAX_DEPTH` 6, the open composition not nested) | Supported | Corpus `render/precomps`, `render/scaled-capture` |
| Groups: transform-only flattened, opacity boundaries composited | Supported | Corpus `render/groups` |
| Solo, hidden layers and groups, in and out points, matte sources skipped | Supported | Corpus `render/solo`, `render/groups` |
| Composition backgrounds (none, solid with alpha, linear, radial) | Supported | Corpus `render/background-*` |
| 3D layers: the full scene pass (depth sets, shadows, every light, maps, transparency, editor looks, legacy `scene3d` and OBJ mesh layers) | Supported | docs/engine/scene3d.md (Phase 3a 3D lane) |
| Region of interest, output scale, 16- and 32-bit precision | Supported | Regions equal the full frame bit for bit |
| Depth and object-id surface (contract 2) | Supported for top-level layers | `tests/headless.rs` |
| Effects and transitions (renderer given definitions, contract 10): the 16 built-in effects, 4 transitions, and any registered GLSL through `pm-glsl` | Supported, see `docs/engine/effects.md` | `tests/effects_webgl.rs` (shader level against WebGL2), `tests/corpus.rs` (`effects-*` capture), `tests/effects.rs` |
| Text and captions (level 1 rasters, animators, every caption style; level 2 at output resolution) | Drawn with fonts configured (`Renderer::set_text_fonts`); routed otherwise | `docs/engine/text-render.md` |
| Everything else | Routed by `unsupported` | `tests/headless.rs` capability test |

## API

`Renderer` is the headless API both engines use (deliverable 5):

- `Renderer::headless()` or `Renderer::new(device, queue)`: one per engine. It
  keeps compiled pipelines, the target pool, media caches, the 3D programs,
  and the evaluator's caches across frames.
- `render(document, composition, tick, &RenderRequest, &dyn AssetResolver) ->
  Frame`: a new texture with premultiplied working values (Standard: E-encoded,
  Rec.709 primaries) at the request's precision and the region's size, the
  depth and id surface when asked, the legend (top-level layer ids by id
  index), and per-layer failures (section 8 isolation: a failing layer draws
  nothing and is reported).
- `render_into(…, &Target, Option<&Target>)`: into a caller's working texture
  (`Target::wrap` over the bridge presenter's `WorkingTexture` or an export
  buffer; tested to give the same bytes).
- `read_rgba`, `read_depth_id`: readback for tests, captures, and the CPU
  encoder-input reference.
- `plan(…)`: the frame's render graph as data (tests, `pm-cli parity`).
- `pass_through(…)`: section 9's pass-through candidate (a single video drawn
  1:1 over the whole output) with the frame facts
  `pm_color::chroma::pass_through` needs; the exporter compares formats and
  copies Y′CbCr planes when they match.
- `interpret(path, Option<ColorDescription>)`, `video_color(path)`: Interpret
  Footage per file.
- `set_program_cache_dir(dir)`: the disk cache of translated 3D programs
  (amendment 11). The host passes its cache directory.
- `set_3d_msaa(false)`: parity-only, to classify MSAA edge pixels.

`RenderRequest::from_frame_request` maps contract 2's `FrameRequest`:
`output_size`, `region`, `scale`, `precision`, `depth_id`, and `purpose`
(export decodes ProRes 422 as `v216`, amendment 6). A view override is
refused (editor looks are not drawn here). `color` is not applied by the
renderer: frames are working values, and the presenter's display transform or
the exporter's encoder input applies the output (sections 9 and 17).
`resolution` and `media` are honored as full resolution and originals (no
proxies exist yet); `motion_blur` has no effect because layers with motion blur
are unsupported.

## Render graph

`plan::build` walks a composition as `renderProject` does: the compositing
pass (non-boundary groups flattened), bottom to top, today's skip rules, and
each layer's content quad (`contentQuad`) with its world matrix scaled to
output pixels, every number in the TypeScript expression order. The plan is
pure data; `exec` resolves sources (stills, video frames, nested targets, 3D
passes) and draws each target in one render pass with fixed-function
premultiplied source-over.

- Targets keep GL's row order, as today's framebuffers; the finished frame is
  flipped once, exactly, into the top-down output.
- A region of interest renders the full frame's geometry through a viewport
  placed so the region lands at the target's origin. Moving the matrices
  instead changed f32 rounding at hard edges (5 pixels of a rotated image);
  with the viewport every region equals the full frame bit for bit, for color
  and for the depth and id surface. A group boundary renders only the region.
- 3D layers render at the full output size and are drawn as screen-space
  quads; their pass does not shrink to a region yet.

## Image classes

Level 1 reads a compatibility bitmap before filtering; level 2 reads native
precision. `pm-media/image/compat.rs` reproduces Blink's PNG16 reduction,
skcms ICC conversion and clipping, rounded PNG/JPEG premultiplication and
libwebp's fixed-point premultiplication. `image/avif_compat.rs` uses Chromium
152's CrabbyAvif/dav1d/libyuv 8-bit conversion. See `media.md` for the source
revisions and call surfaces.

All 21 detailed `render/still-*` cases are drawn and compared in
`tests/corpus.rs` and `tests/still_compat.rs`: 100% of opaque pixels within
1 CV; all transparent-float comparisons below 1 CV. These include gray and
indexed PNG, PNG16 RGBA, P3 PNG/JPEG/HEIC, baseline and progressive Y′CbCr
JPEG with 4:2:0/4:2:2/4:4:4 MCU layouts, orientation-6 JPEG, gray JPEG,
opaque and alpha lossy/lossless WebP, SDR AVIF, sRGB HEIC and RGB TIFF.
No Interpret Footage override is needed to bypass a class gate.

RGB TIFF's UNDEFINED ICC tag is read explicitly when the TIFF library omits
it. Tests cover Adobe RGB, P3 and sRGB profiles with LZW and uncompressed
storage: their compatibility bitmaps are byte-identical to the shipped
FFmpeg PNG conversion, which forwards ICC. Native readers retain the ICC
for the correct high-precision path. A render at levels 1 → 2 → 1 verifies
that the texture cache never reuses an 8-bit bitmap for level 2. Unsigned
RGB/RGBA 8/16-bit TIFF bitmaps also equal their shipped PNG conversion.
An extra opaque P3/Adobe RGB PNG/TIFF app capture checks this color boundary.
Translucent P3/Adobe RGB PNG/TIFF now reproduce the source premultiply and
Skia color conversion byte-exactly; the transparent float comparison is
below 0.125 CV. See `media.md`, "SVG/still closure".
Opaque RGBA containers stay in the supported class. Matching SDR PNG cICP
and ICC declarations are accepted; conflicting/HDR declarations stay gated.

The header parser still restricts unmeasured structures: HDR/independent
chromaticity PNG; CMYK, 12-bit/arithmetic/lossless JPEG; tiled,
float compression/predictors outside uncompressed/Deflate and other-model
TIFF; and HDR/wide-CICP AVIF outside the measured SDR classes. Additional AVIF proof covers 39 native compositions:
8/10/12-bit 420/422/444/monochrome, enhanced 16-bit 444, embedded ICC and
SDR-base gain maps, max 1 CV with no routed exports. Seven further
compositions close RGB JPEG, gray/alpha TIFF and Deflate float RGB TIFF,
also at max 1 CV with no fallback. Container transforms match Chromium's
measured behavior; late PNG EXIF and WebP EXIF are ignored. BMP, ICO and static GIF now have
composition proof. EXR and DPX are rejected by today's importer even with
forced image MIME types; native source APIs remain available through
Interpret Footage. Full decoder breadth remains open (`media.md`).

Existing animated-image and numbered-sequence WebM renditions now use the
video path, with the shipped slot timing and converter matrix. The three
animated corpus sources pass hidden-app Chromium comparisons within the
video class, including the saved 2× corpus compositions (interiors and
alpha exact). Level 1 uses Chromium's libyuv nearest-chroma RGBA8 upload;
level 2 keeps native conversion and passes same-filter FFmpeg + zimg.
The level-1 bitmap's ≤1 CV independent zimg line remains open (up to 2 CV
on saturated blue); see `media.md` and the external report. Original containers
remain available through the source-preview APIs; import persistence is
unchanged. `ImageSequence` only remains for readable originals that are not
stored WebM renditions, or for the static check without source facts.

## Color

Standard color (decision 1): intermediates are premultiplied E-encoded values
with Rec.709 primaries; a hex code's value is the working value
(`pm_color::standard::parse_hex`, the legacy parser); sources tagged E pass
through; clipping happens only at output.

- `pm_color::transfer`: E (the sRGB piecewise curve extended odd-symmetric
  below 0 and as a power above 1), BT.1886, scene-referred BT.709, gamma 2.2,
  in f64 and the f32 forms the shaders use.
- `pm_color::standard`: the hex reading and section 2 invariant (2) as
  functions.
- `pm_color::linear`: linear-light helpers for linear blending (Phase 3a): E
  conversions of premultiplied values, the nine blend modes in both domains,
  HDR-safe screen, overlay, and soft light.
- `pm_color::chroma`: siting origins, the section 9 upsamplers (horizontal
  linear for 4:2:2; Catmull-Rom horizontally and linear vertically for 4:2:0;
  co-sited samples exact), decimation, and pass-through.
  **Declared 4:2:0 filter.** Section 9 leaves the 4:2:0 encoder-input filter
  to a declaration. A two-row average blurred chroma by `[1, 6, 1] / 8`
  against the linear upsampler (35 CV on an H.264 round trip, failing
  invariant (1)). `VERTICAL_420` is `[−1, 3, 3, −1] / 4` on rows 2y − 1 to
  2y + 2, the exact inverse of linear upsampling, also at clamped edges.
  `pm-export`'s `420v` conversion uses it (a minimal change in that lane's
  `convert.rs`).
- `pm_color::alpha`: section 9's premultiplication rules (identity inputs use
  premultiplied sources as they are; other inputs unpremultiply guarded below
  α = 1/4096, transform, re-premultiply; additive pixels are never
  unpremultiplied; premultiplied-with-color mattes).
- `pm_color::dither`: the deterministic blue-noise tile and RPDF offsets of
  sections 9 and 17.
- `pm_color::cicp`: output tags with amendment 7 ("Rec.709 (QuickTime and
  Safari)" is 1-13-1, limited) and code point readings.
- `pm_color::key`: the color components of cache keys (contract 4
  `ColorKey`, a positional source key) and amendment 8's export precision
  default.
- `pm_color::primaries`: primaries conversion in linear light (Display P3
  stills).

The source pass (`shaders/source.wgsl`) converts normalized planes (decode-in)
to working values in f32: chroma upsampling, matrix, transfer and primaries,
alpha. BT.601 primaries read as Rec.709 in Standard (today's Chromium does
not convert them).

### Level 1 video source storage

Ordinary opaque level 1 SDR video in Standard color converts directly into an
`Rgba8Unorm` source texture before the layer's bilinear filter, matching
`compositor.ts`'s `texImage2D(RGBA, UNSIGNED_BYTE)` upload. Previously the
source pass left negative and superwhite RGB in a float texture; filtering
those values before clipping changed saturated edges. This compatibility
upload also reconstructs subsampled chroma with centered bilinear filtering,
matching Chromium 152's [Skia YUV upload shader](https://skia.googlesource.com/skia/+/b6d106297ff9ef2ff8094033695d045e87775581/src/gpu/ganesh/effects/GrYUVtoRGBEffect.cpp).
Native sources retain their existing chroma siting and filter policy.
Chroma reconstruction and the Y′CbCr matrix stay in the existing GPU source
pass. The conversion stores f32 directly into RGBA8, without an intermediate
f16 rounding.

Level 2, colour interpretations, linear blending, HDR, alpha-bearing video,
and deinterlacing keep working-format float sources. A colour interpretation
is an Interpret Footage primaries, transfer, matrix, range or chroma siting
override, or the host's per-file colour override (`MediaCache::interpret`).
Members that only choose or place frames (frame rate, field order, pixel
aspect, clean aperture) and an alpha choice on a file without alpha keep the
RGBA8 upload: setting one must not change a level 1 video's colours (the
upload's first version left it for any member, so a frame-rate override alone
moved saturated chroma edges by up to 0.45). The same members no
longer replace the frame's own colour tags with the probe's reading, and the
numbered-sequence bitmap applies the same rule.

**Precision.** The RGBA8 upload ignores the composition's or export's
working precision: a level 1 video in a Float16 or Float32 project or export
is stored at 8 bits per channel, as today's `texImage2D(RGBA, UNSIGNED_BYTE)`
stores it and as level 1 stills already are. Float precision applies to the
layers and intermediates after the source; level 2 (or a colour
interpretation) reads the video at full precision.
Animated-image and numbered-sequence renditions keep their existing libyuv conversion.
Converted-frame cache readings distinguish these paths, and retired targets
are reused only at the selected storage format.

`media/source_storage_tests.rs` checks direct byte rounding on both sides
of every half-byte boundary, unclipped native float storage, and switching
a real H.264 frame between compatibility/native/linear/interpreted readings
through cache hits, frame advances, and per-file override removal. A normalized
4:2:0 fixture checks compatibility bytes against an independent centered
bilinear CPU reference and native floats against a left/Catmull-Rom reference.
The cache test also keeps a frame-placing interpretation (pixel aspect, clean
aperture, progressive, alpha ignored) on RGBA8. `tests/footage.rs`
`interpret_overrides_change_the_reading` checks an uninterpreted level 1
ProRes 4:2:2 frame against an independent RGBA8 reference (centered chroma,
BT.601, clipped and rounded: max 3e-8), each frame-placing member against that
render exactly, and colour overrides against the native float reading (max
3e-7). Its earlier uninterpreted float assertion predates the RGBA8 upload;
the native reading is now checked through a colour override restating the
file's primaries.

The general corpus request uses `VideoEnd::Final`, matching the capture's
`prepareFrame` before `renderToPixels`. The viewer default (`0.04` s padding)
selected frame 118 instead of 119 in the final `retime/speed` sample; final
output uses `1 / composition fps`. The renderer's two policies are unchanged.

**Recorded deviations in the corpus comparison** (`tests/corpus.rs`):
`hevc-alpha.mov` is encoded with straight alpha but its probe reports Apple's
premultiplied default, so the test interprets it as straight, as Chromium
reads it; Chromium reads untagged VP9 and AV1 with BT.709 coefficients where
section 9's rule reads untagged SD as BT.601 (H.264, HEVC, and VP8 read BT.601
in both), so the test interprets those two fixtures as BT.709. Both are
Interpret Footage settings, not renderer changes.

## The 3D scene pass

`scene3d` draws a 3D object layer as `compositor.ts` does (amendment 11's
native port of the three.js r180 path):

- **Scene.** `pm-eval`'s `compositionScene` gives the object, its material
  values, the lights, the topmost active camera and environment, and the
  studio defaults; the object sits at its `world3D` matrix; the camera at its
  world pose aimed at its target with the layer's world Y as up; light layers
  at their world positions aimed through their parent space.
- **Geometry.** `pm_geom3d::Primitive` builds three.js's `BoxGeometry`,
  `SphereGeometry`, `PlaneGeometry`, `CylinderGeometry`, `ConeGeometry`, and
  `TorusGeometry` as `primitiveGeometry` calls them, bit-identical to
  three.js's buffers (18 parameter sets, hashes from three.js itself,
  `pm-geom3d/tools/three-primitives.ts`).
- **Programs.** r180's shipped `ShaderChunk` is vendored
  (`src/three/r180/`, MIT, from the built module the app bundles; comments and
  blank lines are already stripped there) and `WebGLProgram`'s prefix,
  defines, includes, light counts, loop unrolling, and output encoding are
  ported (`three/program.rs`). Sources equal what WebGL receives byte for
  byte, including the `-0.0000` entries three.js prints for its Rec.709 round
  trip matrix (`tests/three_programs.rs`, 12 recorded programs).
- **Uniforms.** three.js's camera, light, and material uniforms through a port
  of its math (`Matrix4.lookAt`, quaternion and `compose`, `invert`,
  `makePerspective`, `makeOrthographic`, `getNormalMatrix`,
  `transformDirection`, `Color.setHex` with r180's sRGB constants): 858
  recorded values equal ours in f32.
- **Passes.** The shared depth prepass (no color writes; depth writes only for
  layer opacity ≥ 0.999), then the color pass (`LessEqual`, no depth writes),
  4× MSAA into RGBA8 and resolve, as the WebGL canvas. Opaque materials draw
  without blending; double-sided materials do not cull; single-sided ones cull
  back faces, with the front face flipped for mirrored objects (negative
  determinant). The bottom layer of the set clears to the environment
  background (`setClearColor` in sRGB, opaque); others clear to transparent.
  The resolved, premultiplied 8-bit output is drawn as a screen-space quad with
  the layer's opacity, as today's canvas upload. The canvas is clamped to
  1 to 8192 pixels a side and the camera's aspect comes from the clamped
  size, as `renderPass` and `resizeCamera` do.
- **Metal safe math.** wgpu 30 compiles every module with fast math and has no
  option for `MTLCompileOptions.mathMode`. `pm_gpu::msl` writes the naga
  modules as MSL in wgpu-hal's slot conventions, puts MSL 3.2's
  `#pragma METAL fp math_mode(safe)` first, and hands them to wgpu as
  passthrough modules (`Features::PASSTHROUGH_SHADERS`, requested by the
  engine device). No wgpu patch is needed. A GPU test shows the mode is real
  (`isnan(0 / 0)` is kept in safe mode and folded away in fast mode). With safe
  math every reference scene is within 1 CV everywhere; with fast math one box
  had MSAA coverage differences up to 33 CV (47 in a corpus frame), still
  inside the bar's edge exclusion. Devices without passthrough shaders fall
  back to wgpu's compile and report fast math (`Scene3dRenderer::math_mode`).
- **Program cache.** Translated MSL and the pass's reflection are cached on
  disk per program (SHA-256 of sources, math mode, translation revision),
  written atomically; a damaged or mismatched entry is a miss. A cached
  program's first frame took 2.5 ms against 137 ms to translate (debug
  build, on a loaded machine).
- **Depth surface.** With the depth and id surface requested, a single-sample
  pass writes linear view depth (`r32float`, +inf where nothing drew) in GL
  row order; the id pass reads it for the 3D layer.

The Phase 3a 3D lane extends this pass to the full section 8 scope; what it
draws, routes, and measures is in docs/engine/scene3d.md.

## Depth and id surface

`rgba16float`, top-down, the frame's size: r is the view depth of the visible
3D surface in world units (+inf where the top layer is 2D or nothing), g and b
the low and high 11 bits of the top layer's legend index, a is 1 where a layer
covers the pixel (its own alpha at least 0.5/255). Precomps report their
precomp layer, with the depth of 3D inside them (docs/engine/scene3d.md).

## Capability routing

`unsupported(document, composition)` lists every feature of a composition and
everything it nests that this renderer does not draw; `unsupported_with_media`
also opens media headers (the still classes above, animated images, color
profiles, interlaced, rotated, non-square, HDR, or wide-gamut video). The
static check knows only the recorded container (it lists JPEG, AVIF, and
image containers outside the classes above by format or extension), so it
cannot see a 16-bit PNG, a P3 profile, an animated GIF or WebP, or a rotated
video: routing consumers (the tracer and export lanes) must call
`unsupported_with_media`, or route on `RenderError::Unsupported`. The answer is static and
conservative, so one routing decision per document revision holds at every
frame: anything animatable that could need an unsupported path at some time is
listed (an animated blend mode, a material opacity channel that is not a
constant 1, shadows whenever defaults could apply). Listed features: layer
types other than solid, image, video, precomp, group, null, audio, and the 3D
roles; extension definitions; effects; masks; track mattes and matte sources;
transitions; blend modes other than normal; motion blur; 2.5D layers and 3D
groups; adjustment layers with effects; `compat` level 2; non-default project
color; `requiresEngine`; Blender rendering; SVG images; animated images and
sequences; unreadable media; the 3D limits above; request options (view
override, 32-bit precision on a device without `rgba32float` filtering and
blending). A render that still meets something unsupported fails with
`RenderError::Unsupported` rather than drawing it wrong; `render_into` then
leaves the caller's targets untouched (the presenter keeps its last frame).
Media caches key video sources and stills by file size and modification
time, so a file replaced at the same path is reopened.

## Reuse across frames (the interactive viewer)

Two caches serve the viewer's frames of one paused time (gestures, inspector
edits while paused); both are off unless the caller turns them on
(`Renderer::set_draw_reuse`), and `pm-node`'s composer turns them on only around
its own frames. Details and measurements: `perf.md`, "Draw reuse" and "Frame
prefix".

- `reuse.rs`: a layer's draws are given back from the last frame while its node,
  its parents and enclosing groups (transitively), the composition's, open
  composition's, and root's fields, the pass, the build's options, and its media
  path are unchanged (node revisions since the last frame, unique ids). Only
  solids and raster stills on today's fast path, without expressions; the plan
  is the plan a fresh build makes.
- `frame_prefix.rs`: the top-level pass draws from a kept copy of the target
  after its first draws when the frame starts with the same draws (stable
  content only: solids, stills, vector scenes, and groups and nested
  compositions of those; stills checked by size and modification time); exact,
  since splitting a pass and copying a float target change no pixel.
- `tests/draw_reuse.rs` compares plans and read-back pixels with and without
  them through seeded edit sequences and the parity corpus.

## Tests and measurements

Measured on an Apple M4 Max (macOS 27), wgpu 30.0.1 on Metal, Chromium 151
(ANGLE on Metal) for references, 2026-10-13; the corpus row and the still
classes re-measured 2026-10-14 against Chromium 152.

| Test | What | Result |
|---|---|---|
| `tests/corpus.rs` | Every supported project of the newest `v1-*` and `render-*` captures, Standard class (≤ 1 CV for 99.9%, ≤ 4 max), video class (≤ 2 CV on patch interiors), alpha class (transparent float frames within 1 CV), 3D class on 3D pixels | 70 frames, 0 failures (`v1-20261012-230504`, `render-20261014-131536`); every 3D frame within 1 CV everywhere; the routing of each `render/still-*` project is pinned, and the newest `render-*` capture missing one fails the test (with no `render-*` capture at all the pin, like every corpus comparison, reports and passes vacuously) |
| `tests/scene3d*.rs` | The 3D sweep, programs and uniforms, mesh layers, documents | docs/engine/scene3d.md |
| `tests/three_programs.rs` | Generated program sources against WebGL's | 12 of 12 byte-identical |
| `pm-geom3d` | Primitive buffers against three.js | 18 of 18 bit-identical |
| `tests/color.rs` | Invariant (2): 512 solids at 16- and 32-bit | sRGB PNG exact; 10/16-bit RGB 0 CV; 8-bit limited Y′CbCr ≤ 2 CV, greys neutral; 10- and 12-bit Y′CbCr ≤ 1 CV |
| `tests/color.rs` | Pipeline transparency (amendment 8): ProRes 4444 XQ → `y416`, ProRes 422 HQ → `v216`, 32-bit, against VideoToolbox clipped to the codable range | 688,500 and 345,600 samples, 0 differ (color under α = 0 not compared) |
| `tests/color.rs` | Invariant (1), 4:2:0: H.264 → `420v` | 259,200 samples: luma and chroma exact |
| `tests/headless.rs` | Determinism: every supported feature in one composition, 3 times, 16- and 32-bit, two renderers | Byte-identical |
| `tests/headless.rs` | Regions, depth and id surface, capability list, pass-through, `render_into`, contract 2, exec-time unsupported media leaves the target untouched, a replaced video is reopened | Pass |
| `tests/stills.rs`, `stills::tests` | Measured PNG, JPEG, TIFF, BMP/ICO/GIF and SDR AVIF classes pass capability checks; native-depth sources remain separate. Chromium upload hashes pin deep/ICC/chroma AVIF, RGB JPEG and gray TIFF. HDR, CMYK, tiled/other-model TIFF and independent PNG chromaticity remain explicit; malformed/header boundaries and ignored EXIF are checked | Pass |
| `pm-gpu/tests/msl_math.rs` | Safe math is in effect | Pass |
| `frame_time_at_1080p` (ignored) | The everything composition at 1920 × 1080, release | 3.06 ms (16-bit, p95 4.69), 2.48 ms (32-bit, p95 2.76) median per frame, plan and GPU |

The ProRes and H.264 sources come from the media corpus
(`~/Library/Caches/powermove-engine/media-corpus`, `PM_MEDIA_CORPUS`); the
corpus, render, and three.js captures from `~/Library/Application Support/Powermove
Engine Corpus/` (`PM_ENGINE_CORPUS`, `PM_THREE_REF`). Without them those tests
report and pass vacuously.

`render/3d-precomp` has 3D only inside a precomp, which the depth and id
surface does not cover (gaps), so its object mask is empty and its SSIM
prints NaN: every pixel there is judged by the stricter Standard class
(max 1 CV). The corpus ProRes files are tagged BT.601
(`smpte170m`); the transparency test reads them as BT.709 so source and Rec.709
output share the matrix.

## Tools

- `bun engine/crates/pm-render/tools/vendor-three.ts`: re-vendors r180's chunks.
- `pm-electron-lock bun engine/crates/pm-render/tools/three-ref/run.ts`: renders
  the scenes of `tests/fixtures/three-scenes.json` with the production
  `SceneRuntime` in headless Chromium (with and without MSAA), records program
  sources and per-draw uniforms outside the repository, and rewrites
  `tests/fixtures/three-programs.json`.
- `bun engine/crates/pm-geom3d/tools/three-primitives.ts`: primitive hashes.
- `engine/corpus/src/projects/render.ts`: the render corpus cases (`--only
  render/ --label render`; the still cases need `cwebp` from libwebp and
  macOS `sips` at generation), including 3D in a precomp, a parented rig with an
  orthographic rolled camera, the studio sun with a background and a faded
  layer, and a scaled 3D capture.

## Gaps

- 3D layers render at the full output size even for a region of interest.
- 3D output stays 8-bit (as today's canvas) at 32-bit precision.
- The persisted-animation tests compare the video class against both
  Chromium and FFmpeg plus zimg; the broader video gate is in `phase3a-gate.md`.
- No render cache: content keys for layer and precomp outputs (section 8) are
  not computed yet beyond `pm_color::key`.
- The GPU encoder-input pass of the export lane must adopt `VERTICAL_420`
  where it decimates 4:2:0 (the CPU reference already does).
- Decision pending on `VERTICAL_420` for rendered frames: it is the exact
  inverse of linear upsampling (why invariant (1) holds on the rendered
  path), but as a decimation filter for content not upsampled from 4:2:0 its
  gain is about 1.41 at the decimated Nyquist frequency and it undershoots a
  hard chroma step by 25% of the step, so rendered graphics in H.264 exports
  can ring. Alternative: a low-pass filter for rendered frames with
  invariant (1) carried by pass-through (section 9's condition).
- Standard invariant (3) (a "Rec.709 (QuickTime and Safari)" export played
  through `AVPlayerItemVideoOutput` matches the viewer within ΔE2000 1.0) is
  not tested here; only spike S2 measured it. `pm-color` writes the 1-13-1
  tags (amendment 7); the end-to-end test belongs with the export lane.
- Still structures outside the measured compatibility classes above remain
  gated. SVG's Chromium paint-record emulation is incomplete; see `vector.md`.
- Capability gates remain development scaffolding. This media lane removes
  only reasons with measured parity; the derouter lane owns retiring the
  remaining routing machinery.

## Changes outside the lane

- `engine/crates/pm-export/src/convert.rs`: the CPU `420v` reference
  decimates with `VERTICAL_420` (above); its tests pass.
- `engine/corpus/src/projects/render.ts` (new) and `index.ts`: the render
  corpus cases.
- `engine/corpus/src/stills.ts` (new), `media.ts`, and
  `test/corpus.test.ts`: the generated stills (a dependency-free JPEG, PNG,
  and baseline TIFF encoder, a Display P3 ICC profile, and WebP, AVIF, and
  HEIC derived with `cwebp` and `sips`).
