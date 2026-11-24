# Aero eval fixtures

Synthetic and public only. `python3 generate.py` (standard library, Python 3.9+)
writes every file here and `../references/`, deterministically, and pins each
one in `manifest.json`. After any change: regenerate, then bump `version` in
`../suite.json` so old runs are never pooled with new ones. `aero-eval check`
fails on a fixture whose bytes no longer match its pin.

Stills are drawn by the script at 1280x720: a colour chart (grey ramps,
primaries, skin and nature tones), the same chart with lifted blacks, low
contrast and a cool cast, a dusk scene (sky gradient, sun, ground), and two
flat colour cards. The one video is the public engine fixture
`engine/fixtures/h264-aac.mp4` (160x90, red then green, 2 s, 440 Hz tone).
Projects are PMV4 containers with their media embedded, packed the way
`apps/native/crates/acceptance/data/fixtures/generate-portable.py` packs them.

| Project | Made of | Used by |
|---|---|---|
| `interview.pmv` | five 2 s clips of the video (Take 1, Take 2, Take 3, B-roll, Take 4) and a text lower third | cut tasks |
| `speed.pmv` | two clips (Clip, Clip 2) | speed-button panel |
| `chart-plate.pmv` | the chart as Plate | late-afternoon grade |
| `grade-match.pmv` | the cast chart as Plate, the clean chart as Reference (switched off) | grade match; `references/grade-match/chart.png` is the clean chart |
| `chart-mono.pmv` | the chart with the broken Mono effect applied | fix: wrong output |
| `promo.pmv` | the dusk scene as Background plate, a SUMMER DROP title | effects, restyles, Shot Notes fix, multi-step tools |
| `glow-title.pmv` | promo with the broken Soft Glow on the title | fix: compile error |
| `markers.pmv` | promo with markers Intro (0 s), Drop (2.5 s), Outro (5 s) | marker-list panel |
| `cards.pmv` | Shot A (orange card, 0–3 s), Shot B (teal card, 3–6 s) | transitions |

Broken extensions (written by the script, installed into the run's clean profile):

| Extension | Breakage |
|---|---|
| `softglow` | compile: the shader calls `luma()`, which is never defined |
| `shot-notes` | runtime: building the panel reads `project.comp.layers`; a project has no `comp`, so it throws |
| `mono` | wrong output: luma is subtracted from alpha, so the picture comes out as a negative |

Layer fields beyond id, type, name, timing and content are left for the
kernel to fill in, as the container fixtures in `engine/crates/pm-model`
do. Whether the real host opens each fixture as intended is checked by the
`none` harness on the first heavy slot (docs/native/aero-eval.md).
