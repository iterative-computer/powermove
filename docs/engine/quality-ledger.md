# Quality ledger

Places where the native engine deliberately differs from today's app
because today's output is further from a high-precision reference
(QUALITY-1: same or better, authored intent exact). Each row names the
reference and the gate that proves the engine is at least as close.

## Video in composition and export

Reference: each clip's decoded Y′CbCr converted in f64 per the standards
its reading names: the shared colour table
(`pm_media::probe::color::video_reading`), H.273 chroma siting with
section 9's filters, the matrix's Kr/Kb, primaries to Rec.709 in linear
light (signal clipped before linearising), alpha as tagged, and PQ/HLG per
BT.2100 with the BT.2390 EETF. Gate:
`engine/crates/pm-render/tests/video_reading.rs` (engine within its bound,
and at least as close as today, at 8/16/32-bit). Distances are display
codes, worst over the row's clips; "today" is its paused (= exported)
composition. Today's distance from the reference is also how far the
correction moves existing footage; "flat" counts only pixels whose chroma
neighbourhood is constant (no filter choice involved). The engine is
within 0.62 / 0.28 / 0.15 codes everywhere, identical playing, paused and
exported.

| Correction (standard) | Today vs reference: max, mean, flat max | Clips |
|---|---|---|
| Chroma reconstructed at its tagged siting (H.273, section 9 filters) instead of centered bilinear; nearest on the CPU for full range | 77.5, 1.5, 0.5 | fully tagged 8-bit H.264 |
| Full-range H.264: today also samples the decoder's cleared coded padding at the bottom edge | 168, 2.2, 0.7 | 258×146 full range |
| 10-bit kept at half float or better, no RGBA8 store | 84, 2.3, 1.8 | fully tagged 10-bit HEVC |
| BT.2020 and Display P3 SDR converted once in float (BT.2087 linear light); today paused converts through two RGBA8 stores, playing not at all (playing max 158, mean 17) | 143, 2.1, 4.0 | H.264, HEVC 2020 and P3 |
| SD primaries converted (VIDEO-SD-1): SMPTE 170M, tagged or untagged at SD sizes, to Rec.709 in linear light; today reads them as Rec.709 | 84, 5.1, 28 | SD clips with 170M or no primaries tag |
| Untagged and partly tagged video read by the shared table (with SD primaries): untagged SD VP9 BT.601, a lone Rec.709 matrix tag kept | 85, 8.9, 45 | untagged VP9, matrix-only H.264 |
| PQ and HLG decoded per BT.2100 and tone mapped with the BT.2390 EETF to 203 cd/m² SDR white (HDR-1), in float; today: RGBA8 before Skia's RWTMO curve paused, untone-mapped playing (playing mean 33) | 255, 16, 50 | HEVC/AV1 PQ and HLG, P3 PQ |
| VP9 alpha premultiplied in float (today: 8-bit straight store, nearest chroma) | 115, 4.2, 25 | VP9 alpha (SD primaries too) |
| HEVC alpha read as its `AlphaChannelMode` tag says (premultiplied); today ignores the tag | 100, 19, 48 | HEVC alpha (table and SD primaries too) |
| ProRes 422 and 4444 (today decodes only a VP9 rendition) | — | engine 0.45 / 0.05 / 0.0001 |

## Transparent PNG export

Reference: the premultiplied working value divided by alpha in f64.
Gate: `pm-export` `convert::tests::transparent_png_unpremultiplies_the_working_floats`.

| Cleanup | Today | Engine |
|---|---|---|
| Straight colour is the working float divided by alpha, rounded once (today: rounded to 8 bits, truncated on unpremultiply, premultiplied and unpremultiplied again by Canvas2D) | max 167, mean 1.7 over visible alpha | max 0.50, mean 0.25 |
