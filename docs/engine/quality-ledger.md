# Quality ledger

Places where the native engine deliberately differs from today's app
because today's output is further from a high-precision reference
(QUALITY-1: same or better, authored intent exact). Each row names the
reference and the gate that proves the engine is at least as close.
Errors are 8-bit display code values on the capture's own ICC profile.

Reference for every video row: the shared colour table
`pm_media::probe::color::video_reading` (tags, else the untagged rules),
decoded in f64, signal clipped at the EOTF, sRGB curve, linear-light
primaries to the display ICC (`e2e/helpers/source-colour-reference.ts`).
Gate: `e2e/engine-source-compositor.spec.ts`, which writes
`source-compositor/comparison.json` under `PM_SOURCE_EVIDENCE`.

| Surface | Today (CSS video monitor) | Native monitor |
|---|---|---|
| HD video Chromium decodes with VideoToolbox (≥ 360 lines): its CoreVideo 709 transfer is read as BT709_APPLE (1.961 gamma), lifting mid-greys; composition and export use the sRGB curve | max 11.9, mean 3.4–5.8 | max 0.54, mean 0.15–0.21 |
| Untagged video below 720 lines: Chromium reads SMPTE 170M primaries; the shared table reads Rec.709 primaries, as composition and export do (VIDEO-SD-1 switch `UNTAGGED_SD_PRIMARIES`) | max 11.8, mean 2.6–6.0 | max 0.51, mean 0.19–0.33 |
| Partially tagged video (matrix only): Chromium discards the set for Rec.601; the table keeps the tagged matrix | max 23.6, mean 6.4 | max 0.45, mean 0.14 |
| 10-bit HEVC (Main10, P010) | max 11.6, mean 2.8–3.6 | max 2.44, mean 0.79–0.87; Chromium's compositor keeps this residual for 10-bit frames, also when they arrive as WebCodecs I420P10 buffers |

Fully tagged 8-bit video that Chromium software-decodes already reads the
table's way and stays byte-exact with today (max 0.45–0.50 from the
reference for both).
