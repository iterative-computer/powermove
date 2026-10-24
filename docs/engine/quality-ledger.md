# Quality ledger

Places where the native engine deliberately differs from today's app
because today's output is further from a high-precision reference
(QUALITY-1: same or better, authored intent exact). Each row names the
reference and the gate that proves the engine is at least as close.
Errors are 8-bit display code values on the capture's own ICC profile.

| Surface | Today | Native | Reference | Evidence |
|---|---|---|---|---|
| Source monitor, video Chromium decodes with VideoToolbox (≥ 360 visible lines, e.g. HD H.264) | CSS video reads the CoreVideo 709 transfer as BT709_APPLE (1.961 gamma): mid-greys lifted, max 11.9, mean 3.4–5.8 from the reference; disagrees with composition and export | Same planes and compositor, tagged CICP BT.709 (sRGB curve): max 0.5 (1.04 for full-range, which is decoded to video range), mean 0.15–0.40; matches composition and export | Stream matrix/range/primaries (or Chromium's untagged defaults), section 9 transfer reading (sRGB curve, Chromium's own BT.709 rendering choice), linear-light primaries to the display ICC, f64: `e2e/helpers/source-colour-reference.ts` | `e2e/engine-source-compositor.spec.ts` (reference-gated cases; writes `source-compositor/comparison.json` under `PM_SOURCE_EVIDENCE`) |

Open decision, not changed: SD Rec.601 video (tagged, or untagged below 720
lines) carries SMPTE 170M primaries. Today's CSS monitor and the native
monitor convert them (0.6 from the reference); today's composition, the
native composition and export read them as Rec.709
(`pm_render::media::standard_video_primaries`), 11–12 codes from the
reference on saturated colours. Converting them in composition would shift
every SD video project, so it is left to the orchestrator.
