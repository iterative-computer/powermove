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
| Image layers in the tested still classes (below): 8-bit PNG, baseline gray JPEG, opaque WebP, 8-bit opaque sRGB HEIF, 8-bit opaque untagged stripped RGB TIFF | Supported | Corpus `render/still-*` (detailed content): ≤ 1 CV, 100% within 1, transparent frames ≤ 0.62 CV; `media/still-*` fixtures: exact |
| Other stills (Y′CbCr or progressive gray JPEG, Display P3, 16-bit PNG, WebP with alpha, AVIF, GIF, BMP, ICO, OpenEXR, DPX, ICC-tagged, tiled, float, or deep TIFF) | Routed | Measured differences below; `tests/stills.rs` |
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

`pm_render::stills` decides whether a still is drawn, from the decoded facts
and a header parse (JPEG frame type and components, PNG color chunks, WebP
chunks, TIFF image structure, EXIF orientation, HEIF transform properties).
The parse reads only headers and metadata (seeking past image data; EXIF
payloads over 1 MiB are unsupported rather than read). A class is drawn only if it matched the legacy renderer on the
`render/still-*` corpus cases: generated stills with smooth ramps, saturated
5 × 3 checkers (chroma edges at every subsampling phase), one-pixel lines,
and ring patterns, drawn 1:1 on texel centers and again minified and
rotated (`engine/corpus/src/stills.ts`; WebP, AVIF, and HEIC are encoded
with `cwebp` and `sips` from generated PNGs). Measured 2026-10-14 against
Chromium 152 (captures `render-20261014-084744-e5791e0a` and, for the TIFF
and opaque lossless WebP rows, `render-20261014-131536-ea28ddc1`):

| Class | Frame (≤ 1 CV / max) | Transparent | Routing |
|---|---|---|---|
| PNG 8-bit gray and alpha (Adam7) | 100% / 1 | 0.62 CV | Drawn |
| PNG 8-bit indexed with `tRNS` | 100% / 1 | 0.50 CV | Drawn |
| Gray JPEG, baseline | 100% / 1 | | Drawn |
| HEIC 8-bit sRGB | 100% / 1 | | Drawn |
| WebP lossy, opaque | 100% / 1 | | Drawn |
| WebP lossless, opaque | 100% / 1 | | Drawn |
| TIFF 8-bit chunky RGB in strips, no profile (uncompressed; LZW, both Deflate tags, PackBits decode identically) | 100% / 1 | | Drawn |
| Fixture PNG, WebP, HEIC, TIFF (flat colors) | exact | 0 CV | Drawn |
| Y′CbCr JPEG 4:4:4 / 4:2:0 / 4:2:2 (baseline, progressive, FFmpeg layouts, EXIF orientation 6) | 99.67% / 3; 90.5% / 4; 96.4% / 26; orientation 90.4% / 24 | | Routed |
| Display P3 PNG, JPEG, HEIC | 99.87% / 83 | | Routed |
| PNG 16-bit RGBA | 99.66% / 2 | 1.99 CV | Routed |
| WebP lossless or lossy with alpha | 99.998–100% / 1–2 | 1.12 CV | Routed |
| Any oriented still: EXIF orientation ≠ 1 (PNG `eXIf`, JPEG APP1, WebP `EXIF`, TIFF tag 274), HEIF `irot`/`imir`/`clap` | not compared | | Routed |
| AVIF (ImageIO) | 68.6% / 94 | | Routed |
| TIFF with an ICC profile (tag 34675, any profile, including sRGB), colorimetry tags, tiles, fill order 2, or other sample formats | not compared | | Routed |
| Progressive gray JPEG | not compared | | Routed |

Causes, measured:

- **Y′CbCr JPEG.** pm-media decodes JPEG with zune-jpeg (the `image`
  crate); Chromium uses libjpeg-turbo (islow IDCT, fancy upsampling).
  `djpeg` from libjpeg-turbo 3.2 reproduces the legacy 1:1 regions of the
  4:4:4, 4:2:0, and 4:2:2 cases exactly (0 CV, every pixel), and the
  orientation-6 case after a 90° clockwise rotation, so EXIF orientation is
  read the same way and the difference is the decoder. Gray JPEG is within
  1 CV. Drawing Y′CbCr JPEG needs pm-media to decode JPEG with
  libjpeg-turbo (pm-media lane).
- **Display P3.** On texel centers the converted values are within 1 CV of
  Chromium's (PNG, HEIC; JPEG adds the decoder difference). Section 9
  converts "in float, relative colorimetric, no clip until output", while
  Chromium clips to sRGB at decode, so bilinear filtering across
  out-of-gamut colors differs by up to 83 CV on the minified copy. Matching
  today's look needs a clip at the source pass in Standard, which section 9
  rules out, or an appendix A entry accepting the difference: a decision for
  the plan owner. Until then P3 stills are routed.
- **16-bit PNG.** Chromium reduces to 8 bits and premultiplies in 8 bits;
  the engine keeps 16 bits.
- **WebP with alpha.** libwebp's premultiplication rounding: 1.12 CV against
  the alpha class's 1 CV; opaque frames are within 1 CV.
- **AVIF.** pm-media reads AVIF through ImageIO; Chromium through libavif and
  dav1d; up to 94 CV on the chroma checker.

**Orientation.** No oriented still of a drawn class has been compared with
the legacy renderer, and the legacy paths differ by container (Chromium
applies EXIF orientation to PNG, JPEG, and WebP; TIFF reaches it through an
FFmpeg conversion to PNG, HEIF through `sips`). So an orientation other than
1, whether pm-media's decoder reports it (`StillImage::orientation`) or only
the container carries it (an `eXIf` chunk after the image data, a JPEG
APP1 the decoder skips), routes the still, as does any HEIF transform
property of any item. `tests/stills.rs` (`oriented_stills_are_unsupported`)
covers PNG, gray JPEG, WebP, TIFF, and HEIC written by ImageIO and by hand,
and checks both paths: the routing (where pm-media's decoder reports the
orientation first for ImageIO's TIFF and HEIC and PNG `eXIf` before the
image data, and only the header parse catches a late `eXIf` or a JPEG APP1
the decoder skips), and, separately, that the header parse
(`stills::container_transform`) finds the transform in every one of the
files on its own (ImageIO's HEIC `irot` of 90, 180, and 270 degrees
included; `imir` and `clap` only in the synthetic `stills::tests`).

**TIFF.** Drawn only as the compared structure, decided from the first
IFD: photometric RGB, three samples of 8-bit unsigned integers
(BitsPerSample and SampleFormat read per sample), chunky, in strips (no
tile tags 322 to 325), fill order 1, opaque, no ICC profile tag (34675)
and no colorimetry tags (TransferFunction, WhitePoint,
PrimaryChromaticities), compression none, LZW, Deflate (8 or 32946), or
PackBits, predictor none or horizontal. The ICC check is on the header tag
because pm-media's TIFF decoder (`image` 0.25 with `tiff` 0.11) reports no
profile: a Display P3 or Adobe RGB TIFF would otherwise be drawn as
untagged sRGB. Every ICC-tagged TIFF is routed, sRGB included, since none
was compared (ImageIO embeds a profile in every TIFF of a tagged image;
`tagged_tiff_is_unsupported` covers Adobe RGB, Display P3, and sRGB, LZW
and uncompressed). The corpus case (`render/still-photo-tiff`, detailed
content) is uncompressed, as FFmpeg writes it; `tiff_compressions_decode_alike`
shows FFmpeg's LZW, Deflate (32946), and PackBits files, the Deflate file
retagged as Adobe Deflate (8, same zlib stream), and ImageIO's untagged LZW
file with predictor 2 decode in pm-media to exactly the uncompressed file's
pixels, and that ImageIO's tiled file is routed. Gray, palette, CMYK,
Y′CbCr, planar, extra-sample, fill order 2, floating-point or signed
samples, colorimetry tags, and other compressions are routed
(`stills::tests::tiff_classes`).

**Gray JPEG.** Drawn only as baseline Huffman (SOF0), the structure of the
compared corpus file (`corpus-photo-gray.jpg`); progressive gray (SOF2) is
routed (`progressive_gray_jpeg_is_unsupported`, ImageIO's progressive
encoder).

Also routed, not compared: BMP, ICO, static GIF, OpenEXR, DPX, float and
deeper-than-8-bit or alpha or ICC-tagged TIFF, deeper-than-8-bit or alpha
HEIF, sub-8-bit PNG, PNG `gAMA` or `cHRM` without an `sRGB` chunk or ICC
profile (Chromium honors them, pm-media does not), PNG `cICP`, `mDCv`,
`cLLi`, WebP with an ICC profile, RGB, CMYK, arithmetic, lossless, and
12-bit JPEG, ICC profiles other than sRGB, and animated images.

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
| `tests/stills.rs`, `stills::tests` | Untested still classes (BMP, GIF, ICO, EXR, DPX, 16-bit, alpha, gray, tiled, and ICC-tagged TIFF, 1-bit PNG, `gAMA`, `cICP`, Y′CbCr JPEG from FFmpeg, progressive gray JPEG from ImageIO) are unsupported; oriented PNG, gray JPEG, WebP, TIFF, and HEIC are unsupported, and the header parse alone finds each transform; lossless TIFF compressions decode like the uncompressed file; header classes | Pass |
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
- The video class's FFmpeg plus zimg reference (section 25) is not compared
  here; video parity is against Chromium only.
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
- Stills routed by the image classes above: Y′CbCr JPEG (needs a
  libjpeg-turbo decode in pm-media), Display P3 (needs the section 9 versus
  legacy clipping decision), 16-bit PNG, WebP with alpha, AVIF, every
  oriented still (no oriented case compared yet), ICC-tagged, tiled, and
  other uncompared TIFF structures (an sRGB-tagged TIFF corpus case would
  lift the most common one, which ImageIO and Photoshop write), and
  progressive gray JPEG (needs a corpus case).
- No consumer of the capability check exists on the integration branch yet
  (no crate depends on `pm-render`): the tracer and export lanes must call
  `unsupported_with_media` or route on `RenderError::Unsupported` when they
  adopt it.

## Changes outside the lane

- `engine/crates/pm-export/src/convert.rs`: the CPU `420v` reference
  decimates with `VERTICAL_420` (above); its tests pass.
- `engine/corpus/src/projects/render.ts` (new) and `index.ts`: the render
  corpus cases.
- `engine/corpus/src/stills.ts` (new), `media.ts`, and
  `test/corpus.test.ts`: the generated stills (a dependency-free JPEG, PNG,
  and baseline TIFF encoder, a Display P3 ICC profile, and WebP, AVIF, and
  HEIC derived with `cwebp` and `sips`).
