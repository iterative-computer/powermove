# Media input formats (Phase 3b)

What the native engine reads, how it reads it, and what proves it.
Plan references: section 13 (media input, "Formats at cutover"), section 9
(per-source interpretation, chroma), 14.7, section 25 (seeking, bit depth,
alpha, SDR color bars, model import correctness), amendments 2, 6, 13,
appendix D ("Interpret Footage…"), and spike S6.

Code: `engine/crates/pm-media` (probe, index, readers, decode service,
interpretation), `pm-color::tonemap` (BT.2446 Method A), `pm-render`
(`footage.rs`, `stills.rs`, `media.rs`, the source pass), `pm-geom3d`
(`gltf`, `obj`), `pm-protocol::media` (`probeMedia` / `mediaFacts`), and the
renderer's Interpret Footage sheet (`apps/desktop/src/renderer/src/media/`).

## Corpora and how to regenerate them

Every corpus is synthetic and lives outside the repository.

| Corpus | Command | Default location |
|---|---|---|
| Format matrix (207 files: 84 video, 5 HDR, 53 stills, 25 sequence frames, 4 animated, 36 audio) | `engine/crates/pm-media/tests/corpus/formats.sh [out]` | `~/Library/Caches/powermove-engine/media-corpus/formats` (`$PM_MEDIA_CORPUS/formats`) |
| Seek corpus (open GOP, B-pyramid, x264/x265 files) | `engine/crates/pm-media/tests/corpus/gen.sh` | `~/Library/Caches/powermove-engine/media-corpus` |
| Model corpus (parity GLBs, glTF edge cases, OBJ bundles) and the three.js references | `bun engine/crates/pm-geom3d/tools/three-models.ts --corpus [dir]` | `~/Library/Caches/powermove-engine/model-corpus` (`$PM_MODEL_CORPUS`) |

`formats.sh` uses our LGPL FFmpeg build (`PM_FFMPEG_DIR`, default
`~/Library/Caches/powermove-engine/ffmpeg/out`). Where our build has no
encoder a named tool writes the file and the script says why next to it:
Homebrew FFmpeg for the GPL x264/x265 and libmp3lame, `cwebp`/`img2webp`,
`sips` for HEIF, the ICC-tagged and CMYK stills and their sRGB references,
`cjpeg` for 12-bit and lossless JPEG, and `formats_tools.py` for float
TIFF, CMYK TIFF with straight alpha (and its opaque twin), SVG, and the MOV
boxes FFmpeg does not write (`mdcv`, `clli`, `dvcC`, `clap`). Video frames come from `testsrc2`
(every frame differs, so a seek one frame off never matches); ramps and
bars use exact codes so depth and color can be checked code by code.

The model tool loads each model with the app's own `loadGltf`
(`core/scene3d/import-model.ts`), `parseObj` (`kernel/obj.ts`), and
`prepareModelImport` (`core/scene3d/import-bundle.ts`) under Bun, and writes
`engine/crates/pm-geom3d/tests/fixtures/three-models.json` and
`three-obj.json` with each input's SHA-256, so a test against a changed
corpus fails instead of comparing different files.

Tests (corpus tests skip with a message when their corpus is absent; the
pm-media ones are `#[ignore]` and run with `--ignored`):

```sh
cd engine
pm-electron-lock cargo test -p pm-media --test formats --test random_access -- --ignored --test-threads=4
pm-electron-lock cargo test -p pm-render --test footage --test stills --test corpus -- --test-threads=4
cargo test -p pm-geom3d --features import
```

FFmpeg 7.1 and later apply container cropping (`clap`) and display
rotation by default; the framemd5 references use `-noautorotate
-apply_cropping codec` because the renderer applies both itself.

## Format matrix

"Native depth" is the depth the decoder hands to the GPU upload: nothing is
narrowed on the way (amendment 6). Planar and biplanar Y′CbCr upload as
per-plane `R8`/`RG8` or `R16Unorm`/`RG16Unorm` textures and convert in the
source pass at the project's working precision.

### Video

| Codec | Containers in the corpus | Decoder | Output (native depth) |
|---|---|---|---|
| H.264 8-bit 4:2:0 (and open GOP, B-pyramid, 4:2:2 10-bit from x264) | MP4, MOV, M4V, 3GP, 3G2, MKV, AVI, FLV, TS, M2TS, MTS, MXF | VideoToolbox, FFmpeg fallback | `420v`/`420f`; `yuv420p`, `yuv422p10le` |
| HEVC 8/10-bit 4:2:0, 10-bit 4:2:2, with alpha | MOV, MP4 | VideoToolbox, FFmpeg fallback | `420v`, `x420`, `x422` |
| ProRes Proxy, LT, 422, HQ, 4444, 4444 XQ (with and without alpha) | MOV | VideoToolbox | `x422` 10-bit; 4444 as `y416` (16-bit, 16-bit alpha); 4444 without alpha downloads as `p416` |
| DNxHD (120, 185x), DNxHR LB/SQ/HQ/HQX/444 | MOV, MXF | FFmpeg | `yuv422p`, `yuv422p10le`, `yuv444p10le` |
| CineForm 4:2:2, RGB, RGBA | MOV, AVI | FFmpeg | `yuv422p10le`, `gbrp12le`, `gbrap12le` |
| MJPEG 4:2:0, 4:2:2, 4:4:4 | MOV, AVI, MKV | FFmpeg | `yuvj4xxp` (full range) |
| DV, DVCPRO25/50, DVCPRO HD | DV, MOV | FFmpeg | `yuv411p`, `yuv420p`, `yuv422p` |
| MPEG-1, MPEG-2 | MPG, M2V, VOB, MXF | FFmpeg | `yuv420p` |
| MPEG-4 Part 2, H.263, Sorenson (FLV1), WMV2 | AVI, 3GP, FLV, WMV | FFmpeg | `yuv420p` |
| VP8 (libvpx for alpha), VP9 8/10-bit, 4:4:4, alpha | WebM, MKV | FFmpeg | `yuv420p`, `yuv420p10le`, `yuv444p`, with alpha |
| AV1 8/10-bit | MP4, MKV | VideoToolbox where the device decodes AV1, dav1d | `420v`/`x420`; `yuv420p`, `yuv420p10le` |
| HDR: HDR10 (PQ, `mdcv`, `clli`), HLG, PQ AV1, PQ VP9, Dolby Vision 8.4 | MOV, MKV, WebM | as the codec | 10-bit; tone mapped in Standard (below) |

### Stills, sequences, and animated images

| Format | Reader | Native depth |
|---|---|---|
| PNG 8/16-bit, gray, palette, RGBA | `image` crate | 8 or 16 bits |
| JPEG 4:2:0/4:2:2/4:4:4, gray, progressive, 12-bit, lossless 16-bit | libjpeg-turbo 3.2.0 (TurboJPEG, pinned in `native/sources.lock`) | 8, 12, 16 bits; Y′CbCr JPEG drawn from its planes |
| TIFF 8/16-bit, LZW, float 32 (RGB and gray) | `image` crate | 8, 16, 32 float |
| CMYK JPEG; CMYK TIFF, with straight alpha; palette, YCbCr, and Lab TIFF; any still ImageIO cannot hand over as RGB or gray samples | ImageIO, drawn through the file's profile into sRGB once (`IccClass::ConvertedToSrgb`) | 16 (sRGB) |
| WebP lossy, lossless with alpha | `image` crate | 8 |
| HEIF/HEIC, AVIF still (8 and 10-bit) | ImageIO | as decoded |
| OpenEXR half and float, with alpha | OpenEXRCore | 16/32 float, linear |
| DPX 8, 10, 12, 16 | FFmpeg | as coded |
| BMP, ICO, GIF (still) | `image` crate | 8 |
| SVG | resvg/usvg, no external resources, rasterized at any size | float coverage |
| Image sequences (PNG, TIFF, DPX, EXR) | per frame, gaps reported | as the frames |
| Animated GIF, APNG, WebP, AVIF | composited frames with their delays | 8 (AVIF as decoded) |

Stills with an ICC profile equivalent to sRGB (including Apple's "Generic
Gray Gamma 2.2", within 1/1024 of the sRGB curve) and untagged stills
bypass ColorSync; Display P3 converts in float by matrix; every other
profile (Adobe RGB, ProPhoto, "Generic Gray" gamma 1.8, camera and printer
RGB) converts through ColorSync, relative colorimetric, from float samples
into extended linear sRGB, unclipped. Floating-point stills are linear;
integer stills without a profile read as sRGB (section 9).

Stills in another color model (CMYK, Lab, indexed) are converted by ImageIO
when they are read: ColorSync draws them through the file's profile into
16-bit sRGB, and the reader marks them `IccClass::ConvertedToSrgb`, so no
later reading applies the profile again (the profile bytes stay for the
facts). A bitmap context only holds premultiplied alpha, so a file with
straight alpha is split first: its color converts as an opaque image and
its alpha is attached unchanged, keeping a straight reading whose color is
exactly the conversion of the same samples without alpha. TIFFs go to
ImageIO by their `PhotometricInterpretation`: the `image` crate reads RGB
and gray only (it failed on palette and on CMYK with alpha, and turned
plain CMYK into RGB without its profile).

### Audio

AAC (ADTS, M4A from AudioToolbox and FFmpeg, HE-AAC), AC-3, E-AC-3, ALAC
(M4A, CAF), AIFF 16/24, CAF PCM, FLAC, MP2, MP3 CBR/VBR, Opus, Vorbis, WAV
16/24/float/5.1, WebA, WMA, and audio inside DV, FLV, 3GP, MP4, TS, MKV,
VOB, MXF, AVI, MOV, OGV, WebM, and WMV, all through FFmpeg.

### 3D models

| Input | Engine path |
|---|---|
| GLB, glTF (self-contained, data URIs) | `pm_geom3d::gltf::import`: GLTFLoader r180's object graph with `loadGltf`'s checks and normalization |
| OBJ alone (a legacy `mesh` layer asset; stays OBJ) | `pm_geom3d::obj::parse`, the kernel's `parseObj` |
| OBJ with its MTL and images | `pm_geom3d::obj::prepare_import` (feature `import`): packed to one GLB as `prepareModelImport` packs it, then imported as a GLB |
| `.blend` | through Blender only (the host's Blender worker, `import_blend`); its outputs are imported as GLBs |

## Probe and color metadata

The probe reports the stream facts the plan lists (codec, profile, bit
depth, subsampling, field order, pixel aspect, clean aperture, display
rotation, timecode, rate and its evidence, VFR, color description, HDR
metadata, alpha mode, audio layout and priming) and, for every value of the
reading, its origin: `File { source }` (the tag that said it: `colr nclx`,
`pasp`, `fiel`, bitstream or container, ICC profile), `Default { rule }`
(the section 9 rule that applied), or `Override` (Interpret Footage).

Untagged defaults (section 9):

- untagged video up to 576 lines uses BT.601 coefficients and 525- or
  625-line primaries chosen by height; HD and above BT.709;
- MJPEG, `yuvj`, and RGB codecs are full range; other Y′CbCr is limited;
- untagged chroma is sited left (type 0); 4:4:4 has no siting;
- BT.709, BT.601, BT.2020 SDR, and sRGB transfers, and untagged video, read
  as the display encoding E (the sRGB curve, extended);
- no field order reads progressive; no pixel aspect reads square; no alpha
  channel reads opaque; the full encoded frame is shown (the clean aperture
  is a choice);
- a code point the engine does not model reads as the nearest supported
  value, with that rule as its origin.

## Interpret Footage

An asset's `interpret` member (additive, native-only, section 6) holds the
overrides; every absent member means "as the file says, or section 9's
default". Shape (all members optional; `"auto"` and `null` mean absent):

```json
{
  "primaries": "bt709" | "bt601_625" | "bt601_525" | "bt2020" | "displayP3",
  "transfer": "displayE" | "bt1886" | "sceneReferredBt709" | "linear" | "gamma22" | "pq" | "hlg",
  "matrix": "bt709" | "bt601" | "bt2020Ncl" | "identity",
  "range": "limited" | "full",
  "siting": "left" | "center" | "topLeft" | "top" | "bottomLeft" | "bottom",
  "alpha": "ignore" | "straight" | "premultiplied" | "premultipliedWithColor",
  "alphaMatte": "#RRGGBB",
  "frameRate": 23.976, "frameRateRational": [24000, 1001],
  "fieldOrder": "topFirst" | "bottomFirst" | "progressive",
  "pixelAspect": [10, 11],
  "cleanAperture": true
}
```

Members that cannot be used are reported (`interpret.<member>: <message>`)
and read as absent. `frameRate` is also read in the exact form `[num, den]`
(positive integers), which the motion lane's tests write; the sheet writes
the float with `frameRateRational` beside it. The renderer has one reading
of the member: `pm_render::retime::rate_override` and the video source's
`interpret` both go through `pm_media::interpret::Interpretation`.

Stills whose ICC profile is neither sRGB-equivalent nor Display P3
(`pm_media::interpret::interpret_still`, which both the sheet and the
renderer use):

- Alpha overrides change how ColorSync's input is read, never its output:
  the stored samples are taken out of their alpha reading (a matte removed,
  unpremultiplied above `UNPREMULTIPLY_ALPHA_FLOOR`) in the stored encoding,
  converted as opaque color, and premultiplied again in linear light
  (`convert_to_linear_srgb(still, pool, alpha)`).
- A primaries or transfer override sets the profile aside as a whole (it
  describes primaries and curve together and cannot be split): the stored
  samples are read with the override, and the value not overridden takes
  the default of a still without a profile. The sheet shows that origin
  ("The ICC profile is set aside by the override; ...").
- Without either, the sheet shows Rec.709 primaries and Linear transfer,
  both from the file ("ICC profile, converted by ColorSync to ...").

Stills ImageIO converted to sRGB (CMYK, Lab, indexed) read as sRGB, from
the file ("Converted to sRGB by ImageIO from the file's color model and
profile"); their overrides reinterpret those converted samples, and
ColorSync never runs again. On integer stills with an sRGB-equivalent or
Display P3 profile, the transfer's origin is the profile ("ICC profile
... (sRGB curve)"), as the primaries' is. The engine accepts no raw paths: the sheet asks main,
main resolves the asset to a `mediaRead` capability handle on the job
engine (session 0), and the job engine answers `probeMedia` with
`mediaFacts` off its thread. The sheet follows the Settings sheet anatomy
(cards, rows, popover menus, no pointer cursor) and shows each value with
its origin; Apply writes the `interpret` member, and the display size and
duration it implies, as one Undo step.

## How sources are drawn

What the renderer does with each kind of source. Several are drawn only for
assets with `interpret` until they are compared with today's viewer (see
Routing below).

- **HDR in Standard projects.** PQ and HLG sources tone map with BT.2446
  Method A (`pm_color::tonemap`, the WGSL `pm_hdr_to_sdr` in the source
  pass), then BT.2020 to Rec.709 in linear light. Through the renderer the
  result equals the f64 oracle applied to the same decode within 1e-3 (f32
  cancellation near black). Dolby Vision draws its base layer; profile 5
  (no base layer other decoders show) stays unsupported.
- **Interlaced sources** deinterlace with bwdif on normalized Y′CbCr before
  chroma upsampling, from the neighboring fields: at field rate when the
  composition rate is at least twice the source frame rate (each frame's
  fields in display order), else at frame rate showing each frame's first
  field. Field-rate output equals FFmpeg's `bwdif=mode=send_field` run on
  the same decode and converted with the same matrix and chroma filter, for
  4:2:2 8- and 10-bit, DV PAL 4:2:0, and DVCPRO 4:1:1, both field orders:
  bwdif runs on each plane, so interlaced 4:2:0 and 4:1:1 chroma is
  deinterlaced at its own resolution from its own fields, then upsampled
  with the source's siting as a progressive frame.
- **Frame selection** follows section 5 rule 7 on presentation timestamps
  (VFR by timestamp, never by index) with amendment 2's tie-break; a
  conformed rate shows presented frame n from n / rate, by rule 7 on the
  conformed grid (from an eighth of a frame before n / rate,
  `retime::override_frame_at`; field-rate output uses the doubled grid),
  clamped to the last frame. The source time is then not clamped to
  `dur − 0.04`, as the motion lane's override reads it. An interpreted
  video is never a pass-through candidate for export.
- **Geometry.** Quarter-turn display rotations and the clean aperture are
  exact permutations and crops of the decoded picture, rotated as FFmpeg's
  autorotate rotates; other angles are unsupported.

### Routing (`pm_render::unsupported`)

The capability check stays conservative: a feature leaves it only when the
renderer draws it and a parity test against today's viewer proves it. An
asset with `interpret` is different: the legacy renderer ignores the member
(the project is stamped `requiresEngine`), so there is no legacy look to
match and only what the renderer cannot draw at all is refused.

| Source | Without `interpret` | With `interpret` |
|---|---|---|
| Progressive, unrotated, square-pixel Rec.709 or BT.601 video (VFR included) | drawn | drawn |
| HDR video (PQ, HLG, Dolby Vision with a base layer) | legacy: today's Chromium handling is not measured (`corpus.md`, "Not covered yet"), so Method A's difference has no appendix A item or corpus case (DECISION) | drawn, tone mapped with Method A |
| Interlaced video | legacy (today's viewer shows the woven frame; bwdif is a look change) | drawn, bwdif at field or frame rate |
| Quarter-turn rotation, non-square pixels, a cropping clean aperture | legacy (exact against FFmpeg, not compared with today's viewer) | drawn |
| Wide-gamut SDR video (Display P3, BT.2020) | legacy (today's viewer clips at upload: the open decision of P3 stills) | drawn, converted in linear light |
| Stills with other ICC profiles | legacy | drawn through ColorSync |
| CMYK, Lab, and indexed stills (converted to sRGB by ImageIO) | legacy (Chromium converts CMYK itself; not compared) | drawn from the converted samples |
| AVIF, BMP, ICO, GIF, EXR, DPX, 16-bit PNG, PNG with `gAMA`/`cHRM`, and other still classes `pm_render::stills` has not proven against the legacy look | legacy | drawn (decoded and tested against reference decoders in `pm-media`) |
| Image sequences and animated images | legacy | legacy |
| HDR stills (a PQ or HLG reading, from the file or an override); OpenEXR chromaticities other than Rec.709 (no primaries override) | legacy | legacy |
| Other rotation angles, Dolby Vision profile 5, unknown primaries | legacy | legacy |

Routing asks at every document revision, so its media probes keep their
facts per path, size, and modification time: `probe_image` and
`probe_video` their answers, `probe_interpreted_image` the still's color
reading (one decode), and `probe_interpreted_video` the video probe (one
open and index); each interpretation is resolved from those facts. On a
6000 × 4000 16-bit PNG (debug build) the first interpreted probe takes
3.6 s and the following ones, under other interpretations, 4–23 µs.

A project stamped `requiresEngine` routes to the engine only when every
native-only field it uses (the section 6 table, `pm_protocol::patch::
FIELD_TABLE`) is one this engine draws or judges elsewhere and at least one
it draws is present: linked media and `interpret` are drawn; project
`color` (its own `ProjectColor` check) and clip `pitch` (audio) are judged
elsewhere; precomp `keepFrameRate` is not drawn, so a stamp that may carry
it routes the project away. A test keeps that classification equal to the
field table, so a new native-only field must be classified first.

### Open decisions

- **HDR in Standard projects.** Method A is implemented and equals the f64
  oracle through the renderer (HDR10 6.78e-4, HLG 1.34e-5), but section 9
  makes the change of look conditional on a measurement of today's Chromium
  handling, an appendix A item, and a corpus case, and none exists yet
  (`corpus.md`, "Not covered yet"). Until then iPhone HLG and HDR10 clips
  in existing projects keep today's look through the legacy renderer.
- **Interlaced, rotated, non-square-pixel, cropping-clean-aperture
  video.** Exact against FFmpeg (bwdif within 2.5e-7, rotations and crops
  bit-exact), not compared with today's viewer, which shows interlaced
  frames woven. They need legacy captures and parity cases before they are
  drawn without `interpret`.
- **Wide-gamut SDR video and ICC stills** shown unclipped versus today's
  clipping at upload: drawn only with `interpret`.

## 3D model import against three.js r180

`pm_geom3d::gltf::import` builds what GLTFLoader builds and what `loadGltf`
measures, compared with the app's own loader on the model corpus
(`tests/gltf_three.rs`): object types in traversal order; names
(`createUniqueName`, `sanitizeNodeName`, and `_getNodeRef`'s
`_instance_N` for meshes, cameras, and lights used by several nodes); local
matrices; attribute counts and sums; index counts and sums; morph targets;
geometry bounds (morph-grown); material classes, parameters, and texture
slots with UV channels; clips with durations and key counts; and the import
facts (vertices, triangles, normalization center and radius).

Findings while matching it:

- **Skinned bounds cancel the mesh's placement.** GLTFLoader binds with the
  mesh's world matrix before any world update (the identity), and
  `scene.updateMatrixWorld()` then sets `bindMatrixInverse` to the mesh's
  inverse world matrix (attached bind mode). The skinned box is in mesh
  space, so `Box3.setFromObject` applying the world matrix cancels the
  mesh node's own transform.
- **Names resolve in three.js's call order**: the scene, then node, camera,
  and light names synchronously in traversal order, then mesh primitive
  names as the meshes finish loading. The engine resolves mesh names in
  request order; three.js resolves them when each mesh's textures finish
  decoding, so two textured meshes whose sanitized names collide can number
  differently.
- **Primitives WebGL refuses to draw still import.** An index past the last
  vertex or an attribute shorter than POSITION (three.js's own exporter
  writes such a COLOR_0 for OBJ points without colors) loads in three.js and
  draws nothing; the engine marks the primitive `undrawable` and keeps it in
  the facts and bounds. Short skin attributes fail the import (three.js
  throws reading past them).
- **A three.js defect the engine does not reproduce:** with a sparse
  accessor over an interleaved buffer view GLTFLoader copies the whole
  interleaved array (`sparse-quantized.gltf` gets 16/3 positions). The
  engine reads the accessor.
- The reference rounds floats with `Math.fround`; the test rounds the
  engine's values the same way.
- Unknown required extensions fail the import (three.js warns and loads what
  it can).

OBJ (`tests/obj_three.rs`): `obj::parse` equals `parseObj` bit for bit (the
Float32 position, normal, and texcoord buffers by SHA-256, counts, bounds,
and every error message), including JavaScript number parsing (`0x10`,
`5.`, `-0`) and `Math.hypot`'s scaled Kahan sum. `obj::prepare_import` and
`prepareModelImport` produce GLBs whose imports are identical: objects,
names, matrices, topology, positions, normals, UVs, colors, indices,
bounds, undrawable marks, material facts, and material definitions. The
packer follows OBJLoader (objects and `g` groups, material groups, fan
triangulation, face normals `(C − B) × (A − B)`, sRGB vertex colors, `l`
as line segments, `p` as points), MTLLoader (first-occurrence key order
with last values, `Kd`/`Ke` sRGB to linear, `d`, `Tr`, `Ns`, `-bm`, `-s`,
`-o`, first texture per slot, every referenced texture must resolve and
decode), the app's conversion (standard, double-sided, unnamed, metalness
0, roughness `clamp(√(2/(Ns+2)), 0.02, 1)`, bump to a clamped normal map),
and GLTFExporter's layout (`AuxScene`, an unnamed container node, normals
normalized when any is off unit length, per-group index ranges, images
flipped and stored as PNG, `KHR_texture_transform`).

Known packer differences, by construction:

- Texture images decode with the `image` crate (EXIF orientation applied as
  an `<img>` applies it) without color management; Chromium converts
  embedded ICC profiles when it draws them into the export canvas.
- Semi-transparent texels: Chromium's canvas round-trips them through
  premultiplied 8-bit; the engine keeps the decoded values.
- Bump maps over 2048 px shrink with a triangle filter where the canvas
  uses the browser's resampler. PNG bytes differ from Chromium's encoder;
  pixels match for opaque images.
- Texture paths are covered by unit tests (flip, transform, wrap, bump
  math, missing and undecodable files); the three.js reference has no canvas
  under Bun, so the corpus bundles are texture-free.

## Measurements (this lane, Apple silicon Mac, debug builds)

From `cargo test -p pm-media --test formats -- --ignored --nocapture`:

- Random access equals sequential decode and FFmpeg's framemd5 for every
  video file in the matrix, through VideoToolbox, FFmpeg, and dav1d (for
  example `av1-8.mp4` 60/60 frames on VideoToolbox and dav1d,
  `av-h264-aac.ts` 50/50, `dnxhr-444.mxf` 25/25, `cineform-rgba.avi`
  25/25). Index builds take 0.04–4.3 ms (the TS demux scan the slowest).
- Native depth: 10-bit ramps keep 877 of 877 codes for ProRes HQ, DNxHR
  HQX, HEVC, AV1, and VP9 (CineForm 858 of 877; its wavelet coding is lossy
  on the ramp); the ProRes 4444 XQ 12-bit ramp has 764 of 1024 codes
  between 10-bit levels, so it is not narrowed to 10 bits.
- Stills against reference decoders (`djpeg`, `dwebp`, FFmpeg): max |Δ|
  0.000 CV for PNG 8/16, TIFF 8/16, BMP, GIF, ICO, JPEG (all subsamplings,
  progressive, 12-bit, lossless 16-bit), and WebP lossy and lossless.
- ICC stills against ColorSync's 8-bit conversion to sRGB (sips): Display
  P3 (matrix) max 1.107 / mean 0.159 CV; through ColorSync from float
  samples, Adobe RGB 1.422 / 0.165, ProPhoto 16-bit 1.511 / 0.279, Generic
  Gray (gamma 1.8) 0.559 / 0.155; Generic Gray Gamma 2.2 is
  sRGB-equivalent, passes through, and is exact. ColorSync's 8-bit path
  differs from its float path near black at the gamut edge; against the
  profiles' own matrix/TRC (or gray TRC) evaluated in f64 the
  ColorSync-converted stills are within Adobe RGB 0.318 / 0.064, ProPhoto
  0.258 / 0.023, Generic Gray 0.043 / 0.033 CV;
  out-of-sRGB pixels kept unclipped (18,956 of 38,644 for P3).
- Stills converted to sRGB by ImageIO, against sips's conversion
  (premultiplied): CMYK TIFF 0.498 / 0.279 CV; the CMYK TIFF with straight
  alpha's opaque twin 0.498 / 0.241, and the straight-alpha TIFF itself
  1.388 / 0.289 (sips's 8-bit path; its straight color equals the opaque
  twin's conversion exactly: max difference 0 in 16-bit codes); CMYK JPEG 2.160 / 0.275 (37 of
  172,800 channels over 1 CV, where sips's JPEG decode differs); palette
  TIFF against FFmpeg's palette lookup 1.000 / 0.326 (ColorSync takes the
  palette's device RGB to sRGB). Little CMS (`tificc`, relative
  colorimetric, 16-bit; measured once, not in the corpus) differs from our
  ColorSync conversion of the same CMYK TIFF by up to 15 CV (mean 0.98): CMYK conversions are CMM-specific, so ColorSync's
  own output is the reference.
- Audio: every format decodes sample-identical to FFmpeg (max |Δ| 0) with
  FFmpeg's priming; seeks equal sequential decode within 3.1e-5 (codec
  overlap windows).
- Models: the 7 glTF/GLB corpus models match three.js r180 on every
  compared value (radius and center within 1e-9 relative); both OBJ bundles
  pack to imports identical to `prepareModelImport`'s; 19 `parseObj`
  inputs (3 files, 16 malformed or edge sources) match exactly.

From `cargo test -p pm-render --test footage -- --nocapture` (renderer,
float32 targets):

- HDR through Method A against the f64 oracle: HDR10 6.78e-4, HLG 1.34e-5.
- bwdif at field rate against FFmpeg's bwdif on the same decode (8 fields
  each, first, middle, and last): `interlaced-bff.mxf` 4:2:2 2.48e-7,
  `interlaced-tff.mov` ProRes 4:2:2 10-bit 2.47e-7, `dv-pal.dv` 4:2:0
  2.50e-7, `dvcpro25-pal.mov` 4:1:1 2.48e-7; frame-rate output equals the
  first field exactly.
- Interpret Footage on an RGBA8 PNG with macOS's Adobe RGB (1998) profile,
  against Adobe RGB → linear Rec.709 evaluated in f64: as the file says
  (straight) 0.202 CV, alpha ignore 0.372, straight 0.202, premultiplied
  0.202, premultiplied with an #FF8000 matte 0.202; transfer gamma 2.2 and
  primaries Display P3 (profile set aside) 0.000.
- Interpret Footage on CMYK stills (`cmyk.jpg`, and `cmyka.tiff` with
  straight alpha) against their converted samples: as the file says, alpha
  ignore, and primaries Display P3 within 1e-4 CV; without `interpret`
  both route to the legacy renderer.

## Phase 3c media closure status

The native source-preview API and original animation timing reader are in
`pm-media::preview` and `image::animation`; see `media.md` for the caller
contract and the distinction between source frames and encoded thumbnails.
Fixture regressions cover original GIF/APNG/WebP timing, alpha, non-monotonic
reads, mixed-padding selected sequences with gaps, replacement/offline media,
filmstrip cancellation, retained frame accounting, and shared decode leases.
The opt-in format test also compares original AVIF animation frames.

H.273 matrix code 10 now remains `Bt2020Cl` throughout probing,
interpretation, color cache keys, and CICP conversion. It is distinct from
code 9 (`Bt2020Ncl`). The source shader performs the nonlinear
constant-luminance decode: sign-dependent chroma scales, green recovered in
linear light, and R′G′B′ returned to the selected source interpretation. A
12-bit FFV1 fixture verifies the probe and GPU against the f64 table-4
reference, including neutral and both chroma signs. The scalar NCL gate
oracle refuses CL rather than silently applying another matrix. The
normative equations and smooth transfer constants are in
[BT.2020-2, table 4](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.2020-2-201510-I!!PDF-E.pdf).
This fix does not establish legacy parity for un-interpreted wide-gamut
video or remove that gate.

The 15-project still gate classes are now drawn through a level-1
compatibility bitmap: early PNG16 reduction, pinned skcms ICC conversion and
clipping, libwebp's fixed-point premultiplication, and pinned Chromium
CrabbyAvif/dav1d/libyuv AVIF conversion. All 21 detailed still corpus projects
are within 1 CV at every opaque pixel and below 1 CV in transparent output.
P3 PNG/JPEG/HEIC, alpha WebP, AVIF and orientation-6 JPEG no longer route.
RGB TIFF ICC extraction handles the UNDEFINED tag type; Adobe RGB, P3 and
sRGB TIFF compatibility bitmaps match their shipped PNG conversion exactly.
Unsigned RGB/RGBA TIFF at 8/16 bits also matches its shipped PNG bitmap.
Native-depth readers and level 2 retain source precision and ICC interpretation.
An additional opaque P3/Adobe RGB PNG/TIFF app comparison covers ICC clipping.
Translucent wide-gamut ICC stills fail that extra capture's unchanged 2D bar
and retain their gate; their side-by-sides are in the external lane report.
The remaining structural gates (for example float/tiled or other-model
TIFF, progressive gray JPEG and HDR/gamma-only PNG) are listed in `stills.rs`;
decoder breadth beyond these proven classes is still an open deliverable.

Existing animated and numbered imports render their persisted WebM through
the video path. The three animated corpus sources pass Chromium's video
class in hidden app exports and all nine frames of their saved 2× corpus
compositions (interiors and alpha exact). Level 1 uses Chromium's libyuv
RGBA8 upload; level 2 passes the native same-filter FFmpeg + zimg reference.
The level-1 bitmap is up to 2 CV from nearest-filter zimg on saturated blue;
that independent ≤1 CV gate remains open, with side-by-sides in the report. Original-decoding timing APIs remain
available; this round does not change the import persistence policy.

The lane report retains the diagnostic comparisons and their side-by-side
artifacts outside the public repository. No corpus image, app trace, or
machine-local operational report is checked in.
