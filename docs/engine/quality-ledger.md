# Quality ledger

Places where the native engine deliberately differs from today's app
because today's output is further from a high-precision reference
(QUALITY-1: same or better, authored intent exact). Each row names the
reference and the gate that proves the engine is at least as close.

## Video in composition and export

Reference: each clip's decoded Y′CbCr converted in f64 with the shared
colour table (`pm_media::probe::color::video_reading`), section 9 chroma
siting and filters, signal clipped before linearising, the sRGB curve,
linear-light primaries, and today's RWTMO curve for HDR. Gate:
`engine/crates/pm-render/tests/video_reading.rs`. Distances are display
codes (0–255), worst case over the row's clips; today is its paused Canvas
upload / its playing direct upload; the engine is one reading at 8 / 16 /
32-bit project depth, identical when playing, paused and exported.

| Cleanup | Today (paused / playing) | Engine (8 / 16 / 32-bit) |
|---|---|---|
| Chroma at its siting with section 9 filters (today: centered bilinear, or nearest on the CPU for full range); 8-bit H.264, VP9, AV1 | max 79, mean 1.6 / same | max 0.50 / 0.062 / 0.0001 |
| Full-range H.264 with coded padding: today samples the decoder's cleared padding at the bottom edge | max 168, mean 2.2 / max 78, mean 1.5 | max 0.50 / 0.062 / 0.0001 |
| 10-bit HEVC and VP9 kept at half float or better (today: RGBA8 store; flat areas off by up to 2.4) | max 84, mean 2.3 / same | max 0.62 / 0.062 / 0.0001 |
| BT.2020 and Display P3 SDR converted once in float (today: two RGBA8 stores paused, no conversion playing) | max 143, mean 2.1 / max 158, mean 17 | max 0.62 / 0.062 / 0.0003 |
| PQ and HLG tone mapped in float with today's curve, also while playing (today: RGBA8 before the curve paused; untone-mapped playing) | max 255, mean 2.2 / max 183, mean 33 | max 0.62 / 0.071 / 0.042 |
| Untagged and partly tagged video read by the shared table (today: untagged VP9 as Rec.709 at SD; a lone Rec.709 matrix dropped for Rec.601) | max 85, mean 7.7 / same | max 0.50 / 0.062 / 0.0001 |
| VP9 alpha: straight colour premultiplied in float | max 120, mean 2.0 / max 87, mean 1.1 | max 0.50 / 0.062 / 0.0001 |
| HEVC alpha tagged premultiplied (`AlphaChannelMode`) read as tagged; today ignores the tag (DECISION VIDEO-ALPHA-1) | max 169, mean 30 / same | max 0.50 / 0.062 / 0.0001 |
| ProRes 422 and 4444 (today decodes only a VP9 rendition) | — | max 0.44 / 0.062 / 0.0001 |

## Transparent PNG export

Reference: the premultiplied working value divided by alpha in f64.
Gate: `pm-export` `convert::tests::transparent_png_unpremultiplies_the_working_floats`.

| Cleanup | Today | Engine |
|---|---|---|
| Straight colour is the working float divided by alpha, rounded once (today: rounded to 8 bits, truncated on unpremultiply, premultiplied and unpremultiplied again by Canvas2D) | max 167, mean 1.7 over visible alpha | max 0.50, mean 0.25 |
