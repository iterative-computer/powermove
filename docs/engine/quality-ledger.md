# Quality ledger

Places where the native engine deliberately differs from today's app
because today's output is further from a high-precision reference
(QUALITY-1: same or better, authored intent exact). Each row names the
reference and the gate that proves the engine is at least as close.

## Video in composition and export

Reference: each clip's decoded codes converted in f64 by code that shares
nothing with the engine (`pm-render/tests/support/standards.rs`): its
reading written by hand from the stream's tags and the shared table
(`fixtures/video/cases.json`; the probe must agree), H.273 range
normalization, Lanczos-3 chroma at the tagged siting, Kr/Kb, primaries
matrices derived from the published chromaticities, the extended sRGB
curve, and ST 2084 / BT.2100 / the BT.2390 EETF for HDR. Gate:
`pm-render/tests/video_reading.rs`. Distances are display codes, worst over
the row's clips; "today" is today's paused (= exported) composition, which
is also how far the correction moves existing footage. "Flat" counts the
pixels whose chroma is constant over the reconstruction footprint (no
filter choice involved). The engine is within 0.59 / 0.063 / 0.004 codes of
the reference on flat pixels at 8 / 16 / 32-bit, within 1 code on average
everywhere (the rest is Catmull-Rom against Lanczos-3 at edges), and
identical playing, paused and exported.

| Correction (standard) | Today vs reference: max, mean, flat max | Clips |
|---|---|---|
| Chroma siting: reconstructed at its tagged H.273 location (left for H.264, HEVC and untagged) instead of centered | 138, 2.3, 1.0 | fully tagged 8-bit H.264 |
| Chroma filter: Catmull-Rom across (4:2:0, 4:2:2) instead of bilinear; against 4:4:4 truth (`chroma_truth.rs`, two encoders' decimation) RMS 11.5 vs bilinear 11.8 vs today 13.2 (4:2:0) and 9.4 / 9.7 / 11.5 (4:2:2), lower in natural, graphic and edge regions alike. Rows stay linear by choice: Catmull-Rom down is 6–7% lower RMS again, but linear rows keep the 4:2:0 export round trip exact (a re-export loses no generation) | see measurement | `chroma-*.mp4` |
| Full-range H.264: today also samples the decoder's cleared coded padding at the bottom edge | 167, 2.6, 0.7 | 258×146 full range |
| 10-bit kept at half float or better, no RGBA8 store | 157, 2.9, 2.1 | fully tagged 10-bit HEVC |
| BT.2020 and Display P3 SDR converted once in float (BT.2087 linear light); today paused converts through two RGBA8 stores, playing not at all | 157, 2.7, 3.8 | H.264, HEVC 2020 and P3 |
| SD primaries converted (VIDEO-SD-1): untagged SD by line count, 576 lines BT.470BG, others SMPTE 170M (mpv's rule), tagged as tagged, to Rec.709 in linear light; today reads them as Rec.709 | 89, 5.5, 28 | SD clips with 170M or no primaries tag |
| Untagged and partly tagged video read by the shared table: untagged SD VP9 BT.601, a lone Rec.709 matrix tag kept | 87, 9.3, 45 | untagged VP9, matrix-only H.264 |
| Superwhites and sub-blacks kept through linearisation and primaries conversion for every SDR reading (today and the first rework clipped them whenever primaries converted); exposure recovers the same highlights on SD and HD (`superwhites_reach_effects_the_same_way_for_every_reading`) | — | `superwhite-*.mp4` |
| PQ and HLG decoded per BT.2100 and tone mapped with the BT.2390 EETF to 203 cd/m² SDR white (HDR-1), in float; today: RGBA8 before Skia's RWTMO curve paused, untone-mapped playing | 255, 16, 42 | HEVC/AV1 PQ and HLG, P3 PQ |
| VP9 alpha premultiplied in float (today: 8-bit straight store, nearest chroma) | 120, 4.3, 25 | VP9 alpha |
| HEVC alpha read as its `AlphaChannelMode` tag says (premultiplied, VIDEO-ALPHA-1); today ignores the tag | 172, 33, 130 | HEVC alpha |
| ProRes 422 and 4444 (today decodes only a VP9 rendition) | — | engine flat 0.45 / 0.03 / 0.0001 |

## Transparent PNG export

Reference: the premultiplied working value divided by alpha in f64.
Gate: `pm-export` `convert::tests::transparent_png_unpremultiplies_the_working_floats`.

| Cleanup | Today | Engine |
|---|---|---|
| Straight colour is the working float divided by alpha, rounded once (today: rounded to 8 bits, truncated on unpremultiply, premultiplied and unpremultiplied again by Canvas2D) | max 167, mean 1.7 over visible alpha | max 0.50, mean 0.25 |
