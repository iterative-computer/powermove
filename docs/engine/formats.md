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
| Format matrix (195 files: 84 video, 5 HDR, 41 stills, 25 sequence frames, 4 animated, 36 audio) | `engine/crates/pm-media/tests/corpus/formats.sh [out]` | `~/Library/Caches/powermove-engine/media-corpus/formats` (`$PM_MEDIA_CORPUS/formats`) |
| Seek corpus (open GOP, B-pyramid, x264/x265 files) | `engine/crates/pm-media/tests/corpus/gen.sh` | `~/Library/Caches/powermove-engine/media-corpus` |
| Model corpus (parity GLBs, glTF edge cases, OBJ bundles) and the three.js references | `bun engine/crates/pm-geom3d/tools/three-models.ts --corpus [dir]` | `~/Library/Caches/powermove-engine/model-corpus` (`$PM_MODEL_CORPUS`) |

`formats.sh` uses our LGPL FFmpeg build (`PM_FFMPEG_DIR`, default
`~/Library/Caches/powermove-engine/ffmpeg/out`). Where our build has no
encoder a named tool writes the file and the script says why next to it:
Homebrew FFmpeg for the GPL x264/x265 and libmp3lame, `cwebp`/`img2webp`,
`sips` for HEIF, `cjpeg` for 12-bit and lossless JPEG, and
`formats_tools.py` for float TIFF, SVG, and the MOV boxes FFmpeg does not
write (`mdcv`, `clli`, `dvcC`, `clap`). Video frames come from `testsrc2`
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
| TIFF 8/16-bit, LZW, float 32 | `image` crate | 8, 16, 32 float |
| WebP lossy, lossless with alpha | `image` crate | 8 |
| HEIF/HEIC, AVIF still (8 and 10-bit) | ImageIO | as decoded |
| OpenEXR half and float, with alpha | OpenEXRCore | 16/32 float, linear |
| DPX 8, 10, 12, 16 | FFmpeg | as coded |
| BMP, ICO, GIF (still) | `image` crate | 8 |
| SVG | resvg/usvg, no external resources, rasterized at any size | float coverage |
| Image sequences (PNG, TIFF, DPX, EXR) | per frame, gaps reported | as the frames |
| Animated GIF, APNG, WebP, AVIF | composited frames with their delays | 8 (AVIF as decoded) |

Stills that carry an ICC profile other than sRGB (Display P3, Adobe RGB,
ProPhoto, gray gamma 2.2) convert through ColorSync into extended linear
sRGB, unclipped; sRGB and untagged stills bypass ColorSync; floating-point
stills are linear; integer stills without a profile read as sRGB
(section 9).

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
and read as absent. The engine accepts no raw paths: the sheet asks main,
main resolves the asset to a `mediaRead` capability handle on the job
engine (session 0), and the job engine answers `probeMedia` with
`mediaFacts` off its thread. The sheet follows the Settings sheet anatomy
(cards, rows, popover menus, no pointer cursor) and shows each value with
its origin; Apply writes the `interpret` member, and the display size and
duration it implies, as one Undo step.

## How sources are drawn

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
  field. Field-rate output equals FFmpeg's `bwdif=mode=send_field`
  converted with the same matrix and chroma filter.
- **Frame selection** follows section 5 rule 7 on presentation timestamps
  (VFR by timestamp, never by index) with amendment 2's tie-break; a
  conformed rate shows presented frame n from n / rate.
- **Geometry.** Quarter-turn display rotations and the clean aperture are
  exact permutations and crops of the decoded picture, rotated as FFmpeg's
  autorotate rotates; other angles are unsupported.

### Routing (`pm_render::unsupported`)

The capability check stays conservative: a feature leaves it only when the
renderer draws it and a parity test proves it.

| Source | Without `interpret` | With `interpret` |
|---|---|---|
| Quarter-turn rotation, clean aperture, interlaced, HDR (PQ, HLG, Dolby Vision with a base layer) | drawn | drawn |
| Wide-gamut SDR video (Display P3, BT.2020) | legacy (today's viewer clips at upload: the open decision of P3 stills) | drawn, converted in linear light |
| Stills with other ICC profiles | legacy | drawn through ColorSync |
| AVIF, BMP, ICO, GIF, EXR, DPX stills; image sequences | legacy (decoded and tested here, not yet proven against the legacy look) | legacy |
| Other rotation angles, Dolby Vision profile 5, unknown primaries | legacy | legacy |

A project stamped `requiresEngine` routes to the engine only when the stamp
names native features this renderer draws (linked media, `interpret`).

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
- ICC stills against ColorSync: Display P3 max 1.107 / mean 0.159 CV,
  Adobe RGB 0.529 / 0.091, ProPhoto 16-bit 0.525 / 0.256, gray 2.2 exact;
  out-of-sRGB pixels kept unclipped (18,956 of 38,644 for P3).
- Audio: every format decodes sample-identical to FFmpeg (max |Δ| 0) with
  FFmpeg's priming; seeks equal sequential decode within 3.1e-5 (codec
  overlap windows).
- Models: the 7 glTF/GLB corpus models match three.js r180 on every
  compared value (radius and center within 1e-9 relative); both OBJ bundles
  pack to imports identical to `prepareModelImport`'s; 19 `parseObj`
  inputs (3 files, 16 malformed or edge sources) match exactly.
