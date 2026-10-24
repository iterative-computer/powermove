# RFC 04: The color contract

Status: draft, version 0. Module: `pm_protocol::color`. Gates Phase 2;
freezes when Phase 2 starts. Owner: Color (`pm-color`).

## Purpose

How a source is read, how intermediates are encoded, how alpha and chroma
are handled, how 3D output is tone mapped, what an output is tagged as, and
which color inputs enter render cache keys (plan sections 8 and 9). The
module also holds the **normative reference functions**: f64
implementations through `libm` that `pm-color`, the GPU shaders, and the
encoder-input shader must agree with within each bar's tolerance.

## Types

| Type | Meaning |
|---|---|
| `Primaries`, `Transfer`, `MatrixCoefficients`, `Range`, `Subsampling`, `ChromaSiting`, `AlphaMode` | One axis each of a source or output description. `Transfer::DisplayE` is the display encoding E; `Bt1886` and `SceneReferredBt709` are the two Interpret alternatives; `Gamma22` is the QuickTime tag candidate; `Pq` and `Hlg` are HDR input. `ChromaSiting` is H.273 types 0 to 5 (untagged 4:2:0 is type 0, `left`). `AlphaMode` includes "premultiplied with color" |
| `ColorDescription` | All of the above plus bit depth. `input_is_identity` says whether reading the source into the working space is the identity |
| `WorkingEncoding` | `standard` (premultiplied, E-encoded, BT.709 primaries) or `linearLight` |
| `Precision` | `unorm8` (`rgba8unorm`), `float16` (`rgba16float`, default), or `float32` |
| `BlendDomain` | Where screen, overlay, and soft light run when linear blending is on: HDR-safe formulas (default) or in E |
| `ColorMode`, `ProjectColor` | The project `color` field as the engine sees it. `ProjectColor::DEFAULT` is decision 1 (Standard, linear blending off, 16-bit, HDR-safe) |
| `BlendMode` | The nine modes |
| `ToneMapping3d` | `acesFilmicR180 {exposure}` for native 3D, `agxBlender` for Blender surfaces (decision 17) |
| `OutputColor`, `QuickTimeTag`, `ViewerColorSpace`, `Cicp` | Output tags: sRGB images 1-13-0 full, Rec.709 broadcast video 1-1-1 limited, Rec.709 images 1-1-0 full, "Rec.709 (QuickTime and Safari)" with the S2 candidate, and the viewer (`srgb` or `display-p3` canvas, no tag) |
| `ColorKey` | The color components of a render cache key: working encoding, precision, blend domain, output, dither. `key_bytes()` is a positional MessagePack encoding identical on every target |
| `Normalization`, `SampleAlignment` | Code value to Y′ in 0..1 and Cb, Cr in −0.5..0.5, per bit depth, alignment, and range |

Reference functions: `display_encode` (E) and `display_decode` (E⁻¹),
`three_r180_srgb_oetf`, `bt1886_decode`, `bt709_inverse_oetf`,
`aces_filmic_r180`, `luma_709`, `may_unpremultiply`,
`ycbcr_codable_range`, `normalization`.

## Invariants

1. **E passes code values through unchanged everywhere.** E is the sRGB
   piecewise curve extended to all reals: odd-symmetric below 0, the power
   segment continued above 1. Tested: round trip over −4..4 within 1e-12
   relative, odd symmetry, continuity at the break, and values above 1 stay
   above 1.
2. **Linear blending bar value:** E(0.5) = 0.735357, which rounds half up to
   188 at 8 bits (section 2). Tested.
3. **Float clipping happens only at output.** Project depth 8 instead clips and
   quantizes each working target write to normalized 8-bit values, matching the
   shipped WebGL pipeline (DEPTH-1). Y′CbCr outputs clip to the codable
   range (8-bit 1 to 254, 10-bit 4 to 1019, 12-bit 16 to 4079); full-range
   RGB to 0..1. Superwhites and sub-blacks survive the pipeline.
4. **Normalization** matches section 13 exactly: `x420`/`x422`/P010
   c = v / 64, Y′ = (c − 64) / 876, C = (c − 512) / 896; `y416`
   Y′ = (v − 4096) / 56064, C = (v − 32768) / 57344; FFmpeg LSB planar
   16·2ⁿ⁻⁸, 219·2ⁿ⁻⁸, 128·2ⁿ⁻⁸, 224·2ⁿ⁻⁸; full range uses full scaling.
   Tested against those constants.
5. **Premultiplication:** identity-input premultiplied sources are used as
   they are; others are unpremultiplied (not below α = 1/4096 at half
   float), transformed, and re-premultiplied. Additive pixels (α = 0,
   RGB > 0) are never unpremultiplied. `may_unpremultiply` is the guard.
6. **3D tone mapping:** native 3D applies three.js r180's ACES Filmic with
   its `exposure / 0.6` pre-scale and saturate, then r180's own sRGB OETF
   (exponent 0.41666, not 1/2.4, and not extended), per fragment, before
   blending and MSAA resolve. `three_r180_srgb_oetf` differs from E by less
   than 2e-5 in (0.0031308, 1) and equals it at 1 and below the break
   (tested). Blender surfaces are AgX to piecewise-sRGB PNG codes, which
   are E. The two curves differ by design until after cutover.
7. **Luma mattes** use BT.709 coefficients on E-encoded values.
8. **Blend domains:** normal, add, multiply, difference, lighten, darken run
   in the active encoding; screen, overlay, soft light follow
   `BlendDomain` when linear blending is on (`BlendMode::uses_blend_domain`).
9. **Non-default color operations require the native engine; Color depth alone
   is shared with the shipped engine.**
   (`ProjectColor::requires_native`, section 6 table).
10. **Cache keys:** every color input that changes pixels is in `ColorKey`;
    `key_bytes` changes when any field changes (tested). The render graph
    hashes these bytes with its other inputs.

## Versioning

Version 0 until Phase 2 starts. Wide gamut, HDR, and ACES projects add
`ColorMode` variants after cutover; each is a version bump, and old peers
refuse such projects through negotiation rather than misrender them. The
reference functions are part of the contract: changing one is a bump even
if no type changes, because cache keys and parity references depend on it.

## What Phase 0 must confirm

- **S2 (precision and encode):**
  - which tag passes bar (3) for "Rec.709 (QuickTime and Safari)":
    CICP 1-13-1 or 1-2-1 with a gamma 2.2 tag. `QuickTimeTag` keeps both
    candidates until then; the losing variant is removed before the freeze;
  - f16 versus f32 on 12-bit and 16-bit sources, which sets the precision
    default rule (exports default to 32-bit when the output or any source
    exceeds 10 bits);
  - Metal precise math for color-critical shaders and `rgba32float` filter
    and blend support;
  - HEVC alpha depth and AVAssetWriter's `colr nclx` tags.
- **Phase 0 Chromium measurement** (section 25): today's conversion for
  1-1-1, 6-6-6, untagged SD and HD, full range, HLG and HDR10. Each
  deviation from E pass-through becomes an appendix A item; if one must be
  preserved for parity, it becomes an `Interpret` default here before the
  freeze.
- **S7:** that `aces_filmic_r180` and `three_r180_srgb_oetf` match three.js
  r180's shader output on the 3D corpus within the 3D bar.

## Open questions

1. The BT.2446 Method A tone mapping for HDR sources in Standard projects
   (section 9) has no reference function yet; the Color lane adds it with
   its parameters in Phase 3b, which is a version bump if the contract has
   frozen by then.
2. Whether `ColorDescription` should carry mastering metadata and an ICC
   profile hash for stills (probe reads them; only the hash would cross the
   wire). Needed by Interpret in Phase 3b.
3. Whether dither belongs in `ColorKey` for preview frames, where it is
   always off, or only in export keys. Kept in the key for simplicity.

## DEPTH-1: authoritative project depth (2026-10-22)

`project.colorDepth` is the single precision setting. The retired stored
`color.precision` is read only when depth is absent, then migrated to depth
16 or 32 on app hydration. Explicit depth wins. Native document readers
perform the same resolution, so CLI/export do not depend on UI migration.
Requests retain the precision hint for wire compatibility but never promote
an explicit project depth. This supersedes automatic export promotion for
project renders. Source conversion keys distinguish unorm8 from both floats.

Contract 4 adds `Precision::Unorm8`; draft version 0 remains draft and its
source build identity changes. Old peers fail draft-build negotiation rather
than interpreting an unknown precision. Existing enum spellings are unchanged.
The shader-fallback request carries depth 8/16/32: depth 8 transports exact
byte codes in the existing four u16 slots per pixel, depth 16 uses binary16,
and depth 32 uses binary32 bands without f16 rounding.
