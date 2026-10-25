# Quality ledger

Every place the native engine deliberately renders differently from today's app because
today's output is measurably wrong (QUALITY-1, QUALITY-2, [gate-policy.md](gate-policy.md)).
Authored intent never changes here. A row records a correction, the standard it follows,
and the proof.

## Row format

One section per area (for example `## Video`, `## Text raster`). Each section states once:

- **Reference:** how the f64 or supersampled reference is computed and the standard it
  follows (for example "H.273 and BT.2100 in f64", "exact coverage, 16 × 16 cells",
  "W3C Compositing and Blending §10 in E").
- **Gate:** the test that proves the engine is at least as close as today.
- **Units:** 8-bit display code values (CV) unless stated.

Then one row per correction:

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|

## Required evidence

- Both distances come from the row's gate, on the cases it names. Describe cases; never
  name private corpus projects.
- Before/after side-by-sides stay outside Git, in the lane's evidence directory, linked
  from the lane report.
- The corrected content renders the same in the preview and the export.
- A correctness fix needs no decision, even when it shifts every project's look. Only
  a correction where "correct" is genuinely ambiguous (no standard applies, or tags
  conflict) cites the orchestrator's decision.

Rewrite a row when its numbers change. Keep no round history here.

## 3D Library cards (C3)

- **Reference:** the engine's render of the same pinned view at 4× per axis (each
  sample 4× MSAA), box-averaged to card pixels in the card's encoding. It is the
  engine's own 8-bit render averaged in gamma space; the 3D lane's linear-light
  resolve should replace it with an independent f32 reference.
- **Gate:** `e2e/library-card-parity.spec.ts`, 3D cases. Pixels away from edges are
  exact. A differing edge pixel (3×3 span above 8 CV in today's card) must be, per
  channel, no further from the reference than today's.
- **Units:** CV, max channel difference, over the pixels where today and the engine
  differ.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| MSAA edge resolve: Metal hardware resolve instead of WebGL's (box filter over samples) | 1, 1, 1.00 | 0, 0, 0.00 | Camera view, 4 viewers × 2 themes (35 edge pixels) |

Open, gate failing: in the Editor view, today is 20, 20, 5.02 and the engine is
19, 19, 4.65 (132 edge pixels, 16 cards), but 4–5 pixels per card are one CV
further from the reference than today's. Owner: 3D lane (linear-light resolve).
The raw camera frame's one interior pixel, `(799, 448)`, is a rounding-midpoint tie:
a surface fit over 2,805 face pixels gives 150.50 ± 0.02, and today (151) and the
engine (150) are both 0.50 away.

## Video

- **Reference:** each clip's decoded codes in f64 from `pm-reference` (H.273 ranges and
  Kr/Kb, primaries from the published chromaticities through RP 177, sRGB, ST 2084,
  BT.2100 HLG, the BT.2390 EETF, and Lanczos-3 chroma at the tagged H.273 location), with
  each clip's reading written by hand from its tags and the shared table
  (`pm-render/tests/fixtures/video/cases.json`). Chroma filtering is judged against a
  4:4:4 original instead.
- **Gate:** `pm-render/tests/video_reading.rs` (engine within 0.59 / 0.063 / 0.004 CV of
  the reference at 8 / 16 / 32-bit on pixels whose chroma is flat over the
  reconstruction footprint, and at least as close as today everywhere), and
  `pm-render/tests/chroma_truth.rs` for the filter.
- **Units:** CV. Engine columns are 16-bit projects; their max and p99.9 sit at chroma
  edges, where the engine's Catmull-Rom and the reference's Lanczos-3 legitimately
  differ.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Chroma at its tagged H.273 location (left for H.264, HEVC and untagged) instead of centered | 86, 77, 5.6 | 38, 29, 2.3 | fully tagged 8-bit H.264 |
| Chroma filter: Catmull-Rom across for 4:2:0 and 4:2:2 instead of bilinear, closest of the candidates to a 4:4:4 original subsampled by two encoders' filters, in natural, graphic and edge regions alike (RMS 11.5 vs bilinear 11.8 vs today 13.2 at 4:2:0; 9.4 / 9.7 / 11.5 at 4:2:2). Rows stay linear by choice: Catmull-Rom rows measure 5.6–6.5% lower RMS, but linear rows keep the 4:2:0 export round trip exact, so a re-export loses no generation | against the original: 166, 124, 10.9 | 157, 103, 9.0 (one case's max 0.3 above today's, within the 0.5 CV bound) | `chroma-*` (natural and graphic halves) |
| Full-range H.264 no longer samples the decoder's cleared coded padding at the bottom edge | 167, 154, 6.3 | 39, 26, 2.0 | 258×146 full range |
| 10-bit kept at half float or better, no RGBA8 store | 157, 100, 6.9 | 93, 47, 2.1 | 10-bit HEVC, full and limited |
| BT.2020 and Display P3 SDR converted once in float (BT.2087 linear light); today converts through two RGBA8 stores, or not at all while playing | 157, 106, 6.9 | 103, 57, 2.5 | H.264 and HEVC BT.2020, HEVC P3 |
| Untagged SD primaries by BT.601 system (VIDEO-SD-1): 576 active lines BT.470BG, other SD sizes SMPTE 170M, converted to Rec.709 in linear light; tagged 170M likewise; today reads them as Rec.709 | 89, 79, 15 | 39, 26, 2.2 | SD clips with 170M or no primaries tag |
| Untagged and partly tagged video read by the shared table: untagged SD VP9 BT.601, a lone Rec.709 matrix tag kept | 87, 81, 23 | 39, 28, 2.1 | untagged VP9, matrix-only H.264 |
| Superwhites and sub-blacks kept through linearisation and primaries conversion, allowing exposure to recover highlights before display clips them | 8.233, 8.233, 0.935 | 0.500, 0.500, 0.135 | `superwhite-*`; displayed output of 32-bit projects |
| PQ and HLG decoded per BT.2100 and tone mapped with the BT.2390 EETF to 203 cd/m² SDR white (HDR-1), in float; today: RGBA8 before Skia's RWTMO curve paused, untone-mapped playing | 255, 255, 42 | 170, 99, 2.1 | HEVC/AV1 PQ and HLG, P3 PQ |
| VP9 alpha premultiplied in float (today: an 8-bit straight store, nearest chroma) | 120, 93, 11 | 27, 18, 1.1 | VP9 alpha |
| HEVC alpha read as its `AlphaChannelMode` tag says (premultiplied, VIDEO-ALPHA-1); today ignores the tag | 172, 150, 71 | 37, 22, 1.7 | HEVC alpha |
| ProRes 422 and 4444 decoded natively (today shows a VP9 rendition) | Not applicable to the original stream: Chromium cannot decode it; comparing the VP9 rendition would also measure transcode loss | 0.03, 0.03, 0.02 | ProRes 422, 4444 with alpha |

The superwhite row measures 20 neutral band centers (Y′ 8, 16, 128, 235,
250), at zero and −1 stop exposure, in SD and HD clips. The independent
reference is `clamp(2^exposure × (Y′ − 16) / 219, 0, 1) × 255`.
`e2e/engine-video-superwhite.spec.ts` reads the legacy compositor and native
snapshot routes in Electron. Before display clipping, the Rust float-buffer
gate measures max / p99.9 / mean 0.000067 / 0.000067 / 0.000009 CV against
the unclipped reference over the same band centers.

## Video endpoint selection

- **Reference:** source frame timestamps and final-frame hold, independent of
  composition rate. Each synthetic clip has 20 numbered grey frames.
- **Gate:** `pm-render/tests/video_end.rs`, direct and nested compositions at
  matching and mixed rates. Native frame identities are read from GPU output;
  the legacy column evaluates the former 40 ms seek-padding rule, rather than
  measuring a running browser. Units are source frames, sampled at all 20
  source-frame starts. The hold after the source end is also tested.

| Correction | Legacy rule vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Remove browser seek padding at 30 fps | 1, 1, 0.05 | 0, 0, 0 | direct and nested |
| Remove browser seek padding at 60 fps | 2, 2, 0.15 | 0, 0, 0 | direct and nested |
| Remove browser seek padding at 120 fps | 4, 4, 0.50 | 0, 0, 0 | direct and nested |
| Preserve selection at 10, 24 and 25 fps | 0, 0, 0 | 0, 0, 0 | direct and nested |

## Transparent PNG export

- **Reference:** the premultiplied working value divided by alpha in f64, rounded once.
- **Gate:** `pm-export` `convert::tests::transparent_png_unpremultiplies_the_working_floats`.
- **Units:** CV, over visible alpha.

| Correction (standard) | Today vs reference: max, p99.9, mean | Engine vs reference: max, p99.9, mean | Cases |
|---|---|---|---|
| Straight colour is the working float divided by alpha (today: rounded to 8 bits, truncated on unpremultiply, premultiplied and unpremultiplied again by Canvas2D) | 167, 78, 1.7 | 0.50, 0.50, 0.25 | 400,000 random premultiplied pixels |

## Source monitor

Reference for every video row: the shared reading
`pm_media::probe::color::frame_reading` (tags, else the untagged rules),
decoded in f64, extended sRGB curve, linear-light primaries to the
display ICC, clipped only at the display encode (`e2e/helpers/source-colour-reference.ts`).
Gate: `e2e/engine-source-compositor.spec.ts`, which writes
`source-compositor/comparison.json` under `PM_SOURCE_EVIDENCE`.

| Surface | Today (CSS video monitor) | Native monitor |
|---|---|---|
| HD video Chromium decodes with VideoToolbox (≥ 360 lines): its CoreVideo 709 transfer is read as BT709_APPLE (1.961 gamma), lifting mid-greys; the shared reading uses the sRGB curve | max 11.9, mean 3.4–5.8 | max 0.54, mean 0.15–0.21 |
| Partially tagged video (matrix only): Chromium discards the set for Rec.601; the reading keeps the tagged matrix | max 26.2, mean 6.1 | max 0.51, mean 0.15 |
| 10-bit HEVC (Main10, P010) | max 11.6, mean 2.8–3.6 | max 2.44, mean 0.79–0.87; Chromium's compositor keeps this residual for 10-bit frames, also when they arrive as WebCodecs I420P10 buffers |
| Untagged SD ProRes (VideoToolbox): Chromium reads Rec.709 primaries; the reading gives SMPTE 170M (VIDEO-SD-1) | max 11.1, mean 5.2–6.3 | max 0.69, mean 0.31–0.59 |
| Untagged SD VP9 with alpha (software decode): Chromium reads Rec.709 primaries; opaque samples measured | max 11.9, mean 2.4 | max 0.49, mean 0.11 |
| Tagged BT.470BG PAL (720×576, BT.470BG primaries and gamma-2.8 tag read as the display encoding) | max 24.9, mean 8.9–13.6 | max 0.54, mean 0.19–0.23 |
| Untagged 576-line video: Chromium reads SMPTE 170M primaries; the reading gives the 625-line EBU/BT.470BG primaries (VIDEO-SD-1, by line count) | max 26.1, mean 6.4–6.9 | max 0.54, mean 0.19–0.23 |
| Untagged HD VP9: Chromium's software decode reads it with Rec.709 primaries but its own transfer handling | max 11.9, mean 4.2–5.4 | max 0.53, mean 0.18–0.21 |
| Image-sequence renditions written untagged (1080p VP9): Chromium reads BT.709, but the importer encoded them with FFmpeg's BT.601 matrix; `frame_reading` reads BT.601 at any size, and new renditions are tagged | max 26.8, mean 7.3–8.8 | max 0.53, mean 0.23–0.28 |
| BT.2020 SDR (10-bit HEVC, transfer 14): the engine converts the primaries itself, as composition does, and presents sRGB; out-of-gamut bars clip in linear light, where the reference clips the signal first | max 20.6, mean 6.5–6.9 | max 12.3, mean 0.9–1.5 |

HDR (PQ, HLG) is tone-mapped by the engine as composition does and
presented as sRGB; today's monitor puts HDR on screen. There is no f64
reference for tone mapping here, so the gate only requires that it plays
with an SDR capture.

Untagged non-576-line SD H.264 (VIDEO-SD-1, resolved as a fix): the
shared reading gives SMPTE 170M primaries, as today's VideoToolbox monitor
does, and the monitor is byte-exact with today (max 0.49–0.69 from the
reference for both). Composition and
export move from Rec.709 to the same primaries when the videoparity lane
adopts `frame_reading`; that change is its row. Tagged SMPTE 170M SD video is
byte-exact with today (max 0.49 from the reference for both).

Interpret Footage on the monitor: today's CSS monitor ignores it, so an
interpreted asset looked different in the monitor and in the viewer. The
native monitor draws interpreted footage as composition does (within one
code of a full-frame layer of it: `pm-render` test
`monitor_reads_interpret_footage_as_composition_does`).

Fully tagged 8-bit video that Chromium software-decodes already reads the
shared way and stays byte-exact with today (max 0.45–0.50 from the
reference for both).
