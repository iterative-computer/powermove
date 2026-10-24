# Quality ledger

Places where the native engine deliberately differs from today's app
because today's output is further from a high-precision reference
(QUALITY-1, QUALITY-2: correctness wins, authored intent exact). Each row
names the reference and the gate that proves the engine is at least as
close. Errors are 8-bit display code values on the capture's own ICC profile.

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

Untagged 480-line-or-smaller SD H.264 (VIDEO-SD-1, resolved as a fix): the
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
