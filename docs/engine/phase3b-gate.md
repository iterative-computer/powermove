# Phase 3b gate

The Phase 3b gate (plan section 26, "Phase 3b: Media and delivery"; section 25's
correctness and conformance line and its Phase 3b performance rows; the section 2
bit depth, alpha, SDR color, seeking, and audio bars), measured on `feat/ne-p3bgate`:
the integration branch `feat/native-engine` at `c5833c5d` (waves 1 to 4, with
`feat/ne-exports`, `feat/ne-audio3b`, and `feat/ne-blender` already in it) plus
`feat/ne-formats` at `a9add0e4`, merged here (`56cf279d`), plus this lane's
measurement scripts. No engine code was changed here beyond the merge resolution
below.

Measured 2026-10-17 between 20:30 and 22:20 on Jude's MacBook Pro (Apple M4 Max,
128 GB, macOS 27.0, built-in 120 Hz XDR panel, MacBook Pro Speakers as the output
device), Electron 44.0.0, Blender 4.5.14 LTS, release `pm_node.node` built in the
worktree. The Mac was shared with other lanes (1-minute load average 5 to 10; another
lane's e2e ledger held the Electron lock for part of the evening). Every Electron run
was hidden and unfocused, one at a time under the machine lock. This is the
Pro-class reference machine of decision 11; base-machine rows are deferred by Jude's
decision and are marked so. Numbers are as measured; nothing is passed on a looser
tolerance than the plan's.

> Gate: codec and export conformance; bit depth, alpha, SDR color, seeking, and audio
> bars; model import correctness on the corpus; J/K/L and audio scrub functional tests;
> Phase 3b performance rows.

## The merge

`feat/ne-formats` conflicted with the integration branch in six files.

- `pm-geom3d`: the formats lane and the glTF lane each wrote a `gltf` module. The glTF
  lane's (`load`, `anim`, `pose`, `pick`; what the renderer draws with) stays
  `pm_geom3d::gltf`. The formats lane's import check (`loadGltf`'s checks and
  normalization, the object graph three.js builds, the GLB writer, OBJ packing) moves
  unchanged to `pm_geom3d::model_import`; `obj/pack.rs`, `tests/gltf_three.rs`, and
  `tests/obj_three.rs` name the new path. `Cargo.toml` takes the formats lane's
  feature list (a superset). The two loaders now coexist; one of them should go
  (owner: 3D, with Media).
- `pm-node` `jobs/mod.rs`: both the Blender stage (blender lane) and the probe lane
  (formats lane) kept.
- `engine/Cargo.lock`: regenerated (`git checkout --theirs` and `cargo update -w`).

After the merge `cargo check --workspace --tests` is clean, and pm-geom3d, pm-node,
pm-cli, pm-media, pm-render, pm-export, and pm-audio tests pass (below).

## Summary

| Gate line | Measured | Result |
|---|---|---|
| Codec conformance: FFmpeg references | 222 framemd5 references, 13,175 of 13,175 frames equal; every audio format sample-identical to FFmpeg (max abs difference 0); stills max 0.000 CV against `djpeg`, `dwebp`, FFmpeg | pass |
| Codec conformance: random access | 227 backend runs over 166 files: 13,689 of 13,689 random-access probes equal sequential decode, in a new session and in the same session | pass |
| Codec conformance: the section 2 corpus in the renderer | HDR10 and HLG through Method A within 6.78e-4 and 1.34e-5 of the f64 oracle; bwdif within 2.5e-7 of FFmpeg; rotation and clean aperture exact; Interpret Footage overrides within 0.372 CV | pass |
| Export conformance including sampled pixel values | Every format at cutover exports and verifies (22 outputs); worst block error ProRes 0.063 to 0.500 CV, H.264 0.078, HEVC 0.094 to 0.141, VP9/VP8 0.859 to 1.906, PNG 0 | pass |
| Bit depth | ProRes 422 HQ ramp: 877 luma codes 64 to 940, monotonic, step 1, Cb = Cr; 897 Cb codes 64 to 960; 16-bit PNG every column ideal, largest step 18 (bound 18); 8-bit dither 16-column mean within 0.025 CV | pass |
| Alpha | ProRes 4444 alpha exact at 16 bits, straight and premultiplied (color worst 1.34 and 2.09 CV at 12 bits for α ≥ 1/64); HEVC with alpha mean alpha PSNR 54.85 dB against AVFoundation's preset 54.79 dB (frame 23: 53.57 against 53.65) | pass on the mean |
| SDR color | Patch centers within 0.496 CV (H.264), 0.495 (HEVC 10), 0.070 (ProRes HQ); ΔE2000 FFmpeg+zimg: H.264 max 0.628 mean 0.162, HEVC 10 max 0.127, ProRes max 0.127; AVFoundation at half float: H.264 max 0.722 mean 0.234, HEVC 10 max 0.161, ProRes max 0.063 | pass |
| Seeking | Open-GOP HEVC and H.264, B-pyramid H.264, edit lists, interlaced MXF, 1 ms WebM, 90 kHz TS, and 1/600 MOV at 23.976 and 29.97: random access equals sequential on every frame | pass |
| Audio | Clicks on their samples at 23.976, 25, 29.97, and 59.94 (within ±1); totals equal round(duration × sr); a 190-minute export (4.44 GB, 547,199,453 samples) exact in total and click placement | pass |
| Model import correctness | 7 corpus models against three.js r180 (6 equal on every compared value; `sparse-quantized.gltf` equals the engine's reading where three.js has a known defect); 2 OBJ bundles pack identically; `parseObj` exact; glTF poses, picks, programs, and renders within the 3D bar (100% within 2 CV, SSIM ≥ 0.99982) | pass |
| J/K/L and audio scrub functional tests | pm-audio J/K/L, reverse, rate change, scrub: 17 of 17; renderer J/K/L, keymap, audio: 156 of 156; e2e play around and audio scrub pass; e2e J/K/L shuttle passes 7 of 12 runs (2× reverse A/V sync) | **fail** (intermittent) |
| 4K ProRes 422 HQ export, real time on base | This Mac: UHD ProRes 422 HQ with 3 layers 18.1 fps end to end (23.976 needed) | **fail** on this Mac; base deferred |
| Export, every format at cutover, at least today's baseline | In the app, 1080p: H.264 0.82×, ProRes 4444 1.01×, WebM VP9 0.04×, PNG 1.29× today's exporter | **fail** (H.264, VP9) |
| 3D export, 3 models with shadows, Blender EEVEE, 16 samples | 1080p, 24 frames, H.264: job engine 0.737 fps against today's 0.654 fps (1.13×); app process tree peak 2,251 MiB with Blender, memory pressure normal throughout | pass on this Mac; base memory gate deferred |
| E2E ledger, flag off | 574 run: 540 pass, 3 fail, 31 skipped | **fail**: 1 new deterministic failure, 1 intermittent, 1 environment |
| Engine specs | 38 of 38 in one run; the J/K/L test 6 of 10 on repeat | **fail** (intermittent) |
| Engine-related specs with the flag on for every launch | 138 tests in 43 files: 105 pass, 30 fail, 3 skipped | **fail**: 7 tests from 1 defect (E1); 23 by design, observing the resting legacy engine, or quarantined |

The gate does not pass as measured. Everything the bars and conformance lines ask of
decode, export, audio, and model import passes; what fails is export throughput, one
intermittent A/V sync line, and app-level defects found by the e2e runs.

What stands between this tip and a pass, by owner:

1. **Export and agents** (pm-export, main): 4K ProRes 422 HQ export below real time
   (18.1 fps; the GPU encoder-input pass is designed but not built, `export.md`);
   H.264 at 0.82× and WebM VP9 at 0.04× today's exporter; an export started right after
   an import asks about offline media (E1, with Proxies and storage); stored preset
   ids changed with the flag off (E2); the image sequence destination (E3, a decision).
2. **Audio** (pm-audio transport, with the tracer lane): J/K/L at 2× loses A/V sync in
   5 of 12 runs (A1).
3. **3D** and **Media**: two glTF loaders in `pm-geom3d` after the merge.
4. **Parity and QA**: 12 legacy tests that read the WebGL compositor, video elements,
   Web Audio, or the legacy RAM preview, 1 that calls the legacy capture
   (`PM.renderFrameTo`, Phase 3c), and the by-design differences listed below.

## Codec conformance

`cargo test --release -p pm-media --test formats --test random_access --test
index_crosscheck --test probe --test audio_decode --test images -- --include-ignored`
(56 tests, 0 failed), against the format matrix (`media-corpus/formats`: 207 files)
and the seek corpus (`media-corpus`: 62 files), our LGPL FFmpeg build:

- Every video file decodes on every backend that takes it (VideoToolbox, FFmpeg,
  dav1d); random access equals sequential decode and FFmpeg's framemd5. The one
  tolerated difference is the documented edit-list case (`editlist-trim.mov`: FFmpeg
  drops the frame straddling the edit's start; the engine has 94 frames, FFmpeg 93, the
  other 92 equal offset by one).
- Backends agree: where two backends decode a file, their normalized frames are equal
  bit for bit.
- Native depth: 10-bit ramps keep 877 of 877 codes (ProRes HQ, DNxHR HQX, HEVC, AV1,
  VP9).
- Audio: 36 formats sample-identical to FFmpeg with its priming; seeks within 3.1e-5
  of sequential decode (codec overlap windows), and exact for PCM.
- Stills, ICC profiles, CMYK, palette, HEIC, EXR, DPX, SVG, sequences with gaps,
  animated GIF, APNG, and WebP: as `formats.md` records, unchanged.

`cargo test --release -p pm-render --test footage --test stills --test video_end
--test retime` (23 tests, 0 failed): HDR10 and HLG tone mapping, P3 footage in linear
light (1.18e-6), BT.601 readings, bwdif against FFmpeg's at field rate (4 files, 2.47e-7
to 2.50e-7), rotation and clean aperture exact, Interpret Footage (alpha, primaries,
transfer, ColorSync profiles, CMYK) within 0.372 CV, conformed rates by section 5
rule 7, video end padding per output kind.

## Export conformance

`cargo test --release -p pm-export --test bars_delivery --test delivery --test formats
--test videotoolbox --test job_export --test blender_export --test bars --test
time_bar` (52 tests, 0 failed, 4 measurement tests not run):

- `every_format_at_cutover_exports_and_verifies`: ProRes Proxy, LT, 422, HQ, 4444
  (straight and premultiplied), 4444 XQ (with and without alpha), H.264, HEVC 8 and 10
  in MP4 and MOV, HEVC with alpha, VP9 and VP8 with and without alpha, PNG sequences 8
  and 16-bit, PNG stills; each verified by its box, demux, or decode checks with sampled
  pixel values against the encoder input.
- Delivery options reach the file (PCM 16/24/float, AAC with `edts`, timecode and
  reel, color tags); captions left out for a sidecar; offline media listed by clip;
  exports after preview renders equal empty-cache exports.
- Pipeline transparency and codec generation (Phase 2 bars, re-run): ProRes 422 HQ
  0.000 CV and Δ 0.000 dB; ProRes 4444 XQ 1.603 CV within 8 px of α = 0 (unchanged
  from the Phase 2 gate; that bar is Phase 2's).
- Blender in exports: every exported sample equals the composite of the stored
  surfaces (max 0 of 65,535); batched and per-layer surfaces equal; cancel stops
  Blender; Cycles devices recorded per job.

## Bars

From the same run (`bars_delivery.rs`) and this lane's scripts:

| Bar | Measured | Result |
|---|---|---|
| Bit depth: 16-bit grey ramp across 3,840 columns to ProRes 422 HQ | 877 luma codes 64 to 940, monotonic, largest step 1 CV, Cb = Cr exactly neutral | pass |
| Bit depth: chroma ramp | 897 Cb codes 64 to 960 | pass |
| Bit depth: 16-bit PNG | monotonic, every column on its ideal code, largest step 18 (bound ⌈65535 / 3839⌉ = 18) | pass |
| Bit depth: default dither on 8-bit | worst 16-column local mean 0.025 CV from ideal (bar 0.5) | pass |
| Alpha: ProRes 4444 | alpha exact at 16 bits, straight and premultiplied; color worst 1.34 CV (straight) and 2.09 CV (premultiplied) at 12 bits where α ≥ 1/64 | pass |
| Alpha: HEVC with alpha against AVFoundation's HEVC-with-alpha preset | mean alpha PSNR 54.85 dB against 54.79 dB over frames 0, 11, 23 (56.75/56.60, 54.23/54.11, 53.57/53.65); the engine's file is 4.3× larger (337,137 against 78,374 bytes) | pass on the mean; frame 23 is 0.08 dB lower |
| SDR color, 8-bit (H.264) | patch centers within 0.496 CV; ΔE2000 FFmpeg+zimg max 0.628 mean 0.162; AVFoundation half float max 0.722 mean 0.234, 8-bit BGRA max 0.900 mean 0.176 | pass |
| SDR color, 10-bit (HEVC 10) | patch centers 0.495 CV; ΔE2000 FFmpeg+zimg max 0.127; AVFoundation half float max 0.161 | pass |
| SDR color, 10-bit (ProRes 422 HQ) | patch centers 0.070 CV; ΔE2000 FFmpeg+zimg max 0.127; AVFoundation half float max 0.063 | pass |
| Seeking | every supported codec in the matrix and the seek corpus; the plan's named cases: `hevc-opengop.mp4` 150/150, `h264-opengop.mov` 150/150, `h264-bpyramid.mp4` 150/150, `editlist-trim.mov` 94/94, `mxf-xdcam422-1080i-tff.mxf` 45/45, `interlaced-bff.mxf` 25/25, `webm-1ms-23976.webm` and `webm-1ms-2997.webm` 150/150, `ts-90k-23976.ts` and `ts-90k-2997.ts` 150/150, `mov-600-23976.mov`, `mov-600-2997.mov` 150/150, `mov-600-prores-23976.mov` and `-2997.mov` 100/100 | pass |
| Audio: click at frame N | frame 37 at 24000/1001 sample 74,074 (expected 74,074), 25 fps 71,040 (71,040), 30000/1001 59,259 (59,259), 60000/1001 29,630 (29,629, within ±1); frames 0, 1, 7 too | pass |
| Audio: totals | 96,096 / 96,000 samples = round(duration × sr) at every rate | pass |
| Audio: no length limit | `long-audio.mjs --minutes 190`: 341,658 frames at 29.97, a FLAC bed as long as the composition, clicks at frames 37, 170,840, and 341,597: 547,199,453 samples reported, decoded, and expected; clicks at 59,259, 273,617,344, and 547,101,755, each exactly floor(N × sr × den / num); 4.44 GB MOV with float PCM past 4 GiB; 2,040 fps | pass |

The SDR color bar names two references. `bars_delivery.rs` reads AVFoundation through
8-bit BGRA, which cannot judge the 10-bit line (its 8-bit numbers: HEVC 10 max 0.549,
ProRes 0.417). `scripts/gate-3b/sdr-avfoundation.mjs` renders the same chart through
`pm-cli render` and reads it with AVAssetReader at half float; its 8-bit BGRA numbers
equal the bar test's (0.900/0.176, 0.549/0.210, 0.417/0.185), so the chart is the same,
and its half-float numbers are the ones above.

## Model import

- `cargo test --release -p pm-geom3d --features import` (with the model corpus,
  `PM_MODEL_CORPUS`): `imports_like_three` (7 models: `camera-light.gltf`,
  `corpus-materials.glb`, `corpus-model.glb`, `shared-mesh.gltf`, `skinned-u8.gltf`,
  `strip-fan.gltf` equal three.js r180 on object types, names, matrices, attribute and
  index counts and sums, morph targets, materials, and the importer's facts;
  `sparse-quantized.gltf` checked against the engine's reading, where GLTFLoader copies
  the whole interleaved array, `formats.md`); `packs_like_prepare_model_import`
  (`obj-gem`, `obj-shapes` identical to `prepareModelImport`'s GLBs);
  `parses_like_parse_obj` (19 inputs exact); `skinned_picks_follow_the_pose`.
- `cargo test --release -p pm-render --test scene3d_gltf`: poses, picks, programs and
  uniforms equal three.js's; 21 rendered views of skinned, PBR-extension, shadowed,
  transmission, WebP/AVIF-texture, and object-material models: 100% of pixels within 2
  CV, max 2 CV, SSIM ≥ 0.99982.
- `blender_snapshots.rs`: 158 snapshots, 135 equal to today's `renderSnapshot`, 23 not
  built (`3d/gltf`, `3d/gltf-materials`): Blender compositions with imported glTF
  models still route to today's renderer (`blender.md`). Not a model-import failure;
  listed under gaps.
- Not measured: the 1M-vertex pick target (`PM_GLTF_BIG` not set; a Phase 3a row).

## J/K/L and audio scrub

| Test | Result |
|---|---|
| `pm-audio` `tests/playback.rs` (17): J/K/L at 2×, −2×, −1× on 48 and 44.1 kHz devices (bed 999.9 to 1000.0 Hz, bursts within 0.02 ms of the line), reverse on its clock (1.2 µs), rate change from what is heard, scrub, pause, seek, loop, device changes, drift | 17 of 17 |
| `pm-audio` `mixer`, `layouts`, `sources`, `resampler`, `legacy_parity` (25) | 25 of 25 |
| Renderer: `engine-connection.test.ts`, `audio.test.ts`, `keymap.test.ts`, `keymap-parity.test.ts`, `InterpretSheet.test.ts` | 156 of 156 |
| e2e `native-engine-transport.spec.ts` play around | pass |
| e2e audio scrub, engine meters, solo, and waveforms | pass |
| e2e J/K/L shuttle at 1× and 2× both ways | 7 of 12 runs: passed in the engine-spec run, failed in the ledger run, 6 of 10 with `--repeat-each=10` |

**A1** (Audio, with the tracer lane): every failure is the 2× reverse run. The
frame-to-line offsets ramp by about 0.13 ms per frame from 7 to 9 ms up to 11 to 13 ms,
then drop back to about 8.5 ms (one run: `0.0 … 8.6 8.7 8.9 … 12.2 12.3 8.3 8.4 8.5
8.7`): a slope between the frame line and the audio clock that is re-anchored, not
random lateness. Passing runs are close to the line too: worst frames 0.1 to 9.1 ms
(2× forward) and 0 to 9.8 ms (2× reverse) against the ±10 ms bar; rates 1.9747 to
2.0004 at 2×. The audio lane recorded 2 of 2 passes on 2026-10-16 on its branch; the
integration base without the formats merge was not re-measured here (the formats merge
touches `pm-audio` only in `conform.rs`, for interpreted assets, which this project
does not have).

## Performance rows

Base-machine rows are deferred (no base machine); this Mac is the Pro-class machine.

| Row | Measured | Result |
|---|---|---|
| 4K ProRes 422 HQ export, at least real time on base | `measure_delivery.rs export_throughput`, UHD ProRes 422 HQ source, 3 layers, 48 frames end to end with verification, in process: 18.1 fps (frames through the encoder 18.5 fps); real time is 23.976 | **fail** on this Mac; base deferred |
| Export, every format at cutover: absolute fps per codec and machine | same test, 1080p, 48 frames: ProRes Proxy 41.8, HQ 42.1, 4444 41.4, 4444 XQ 40.4, H.264 39.7, HEVC 40.0, HEVC 10-bit 40.0, HEVC with alpha 38.7, VP9 13.8, VP8 25.6, PNG 8-bit 41.6, PNG 16-bit 34.3 fps | recorded |
| …at least today's re-measured baseline | `export-baseline.spec.ts` (`PM_EXPORT_BASELINE=1`), one 1080p project of 6 moving solids, 144 frames, Export to done in the app: today's exporter / job engine: H.264 47.9 / 39.4 (0.82×), ProRes 4444 44.6 / 45.0 (1.01×), WebM VP9 162.6 / 6.2 (0.04×), PNG 33.3 / 42.9 (1.29×). Formats today does not write have no baseline | **fail** (H.264, VP9) |
| 3D export, 3 models with shadows, Blender EEVEE, 16 samples: at least today's baseline; memory gate with it running | 0.737 fps against 0.654 fps (1.13×); peak 2,251 MiB, pressure level normal (below) | pass on this Mac; base deferred |

WebM: today's exporter is a realtime-deadline WebCodecs encode at a fraction of the
bit rate; the job engine's is libvpx good quality, cpu-used 2 (`export.md`). H.264: the
job engine writes about 8.6 times the bits today's encoder does at the same tier
(`export.md`); speed was not traded for it here.

### 3D export with Blender

`scripts/gate-3b/blender-export.spec.ts` (`PM_GATE_3B=1`): a 1920 × 1080 composition at
24 fps, 1 s, with a camera, a shadow-casting sun, a ground slab, and three primitive
models (box, sphere, torus) that turn or move so every frame is its own Blender frame;
rendering Blender EEVEE at 16 samples; exported to H.264 MP4 from the export sheet's
`PM.Export.run`, Export to done, each engine in a fresh hidden app. The app's process
tree (main, renderer, engines, Blender workers) was sampled every 500 ms for resident
memory, and `kern.memorystatus_vm_pressure_level` was read.

| | Today's exporter (flag off) | Job engine (flag on) |
|---|---|---|
| Throughput | 0.654 fps (36.71 s) | 0.737 fps (32.58 s), 1.13× |
| Peak resident memory, app tree | 1,828 MiB (Blender 508 MiB) | 2,251 MiB (Blender 645 MiB) |
| Memory pressure | normal (level 1) throughout | normal (level 1) throughout |
| File | 135,648 bytes | 1,977,480 bytes |

The job engine's export was not routed (no routed status), and Blender ran through host
services. Models are primitives: Blender compositions with imported glTF models still
route to today's renderer (`blender.md`), so a 3-model glTF scene would measure
today's exporter twice. On a 128 GB machine the pressure level says nothing about the
8 GB base Air; the memory gate's base form is deferred. One run each; an earlier attempt
failed in today's exporter with `ENOSPC` while the disk was full (below).

## E2E ledger, Use New Engine off

`POWERMOVE_NEW_ENGINE=0 playwright test --config playwright.ledger.config.ts
--workers=1`, no retries: 574 tests run (quarantine.json's 27 entries excluded), 540
passed, 3 failed, 31 skipped (the ledger's 21 environment skips and 10 measurement
tests that run on request: 9 tracer measurements and `export-baseline.spec.ts`),
19.3 min. The Phase 2 gate ran 560 with 3 failures; the suite gained 14 tests since.

| Test | Failure | Triage | Owner |
|---|---|---|---|
| `ae-audit-implementation.spec.ts:102` | A saved render preset reads back with `format: 'prores-4444'` where the spec expects `'prores'` | **E2, new, deterministic.** The delivery lane's format catalog (`shared/delivery.ts` `migrateFormat`, `migratePreset`) maps today's `prores` to `prores-4444`, and `legacy/core/exporter.ts` now writes the catalog id into `renderPresets` with the flag off too. Users without the flag get new ids in their store (a build before the catalog cannot read them). Either the spec moves to the catalog ids or the legacy sheet keeps writing today's ids; a decision | Export and agents |
| `native-engine-transport.spec.ts:91` | 2× reverse: 12 of 43 frames outside ±10 ms | **A1, intermittent** (above) | Audio |
| `chatgpt-connection.spec.ts:5` | Two "Disconnect" buttons | **Environment**, as in the Phase 2 gate: the hidden profile copies the developer's `~/.claude/settings.json`, which carries an auth token on this Mac | Export and agents |

The Phase 2 gate's R1 (`import-image-sequence.spec.ts:7`) passes.

## Engine specs

`native-engine`, `native-engine-tracer`, `native-engine-transport`, `media-store`,
`engine-export`, `engine-delivery`, `engine-blender`, `engine-present-recovery`
(they set the flag themselves): 38 of 38 pass in 2.6 min. Among them: every sheet
format and preset, the Render Queue with persistence, the offline-media message; the
job engine's Blender still 100% within 2 CV of today's surfaces composited (today's own
still 97.7%, max 207 CV, as `blender.md` found); the viewer's Blender preview within 1
CV of its surfaces; J/K/L, play around, and audio scrub. The J/K/L test fails 4 of 10
on repeat (A1).

## Engine-related specs with Use New Engine on

43 files with `POWERMOVE_NEW_ENGINE=1` for every launch (imports, exports, playback
and scrubbing, project files, media recovery and replacement, agent media and models,
Blender, 3D, boot, saves, precomps, compositions, GPU and viewer recovery, captions,
the export presets in `ae-audit-implementation`), the plain config (quarantined tests
included): 105 passed, 30 failed, 3 skipped, 8.0 min.

**Defect**

| Tests | Failure | Triage | Owner |
|---|---|---|---|
| `export-mp4.spec.ts:7`, `export-webm.spec.ts:8`, `import-animated-image.spec.ts:15` (GIF, APNG, WebP), `import-converted-formats.spec.ts:21` (TIFF), `import-image-sequence.spec.ts:7` (120 fps) | `PM.Export.run` answers `{cancelled: true}` | **E1.** An export started right after an import says "1 clip is offline" and stops. Reproduced with `scripts/gate-3b/offline-probe.spec.ts` (export-mp4's steps, every native message recorded): the first export shows "1 clip is offline … tone.wav" (Locate Media…, Cancel), a hidden test window answers Cancel; the same export 1.5 s later starts without a message. Main's offline check (`src/main/engine/index.ts` `checkOffline`, `offline-media.ts`) asks the media store (`media-store/service.ts` `resolveAsset`), which has no binding for the new asset until the renderer's binding arrives. A user who exports right after importing gets a false offline warning | Export and agents, with Proxies and storage |

**Behavior that differs by design with the flag on** (each needs a decision on the spec):

| Tests | Difference | Owner |
|---|---|---|
| `export-destination.spec.ts:38` | **E3.** The job engine's PNG sequence takes a folder its save panel names; the spec answers the still's file path (`chosen.png`, which exists), and the commit refuses to replace a file with a folder ("A file with that name already exists."). Today's sequence writes into an existing folder | Export and agents |
| `ae-audit-implementation.spec.ts:102` | E2 above (fails with the flag off too) | Export and agents |
| `import-converted-formats.spec.ts:21` (HEIC), `:50` (Matroska), `import-prores.spec.ts:7`, `import-prores-alpha.spec.ts:10`, `local-media-recovery.spec.ts:38` | Kept originals instead of converted copies and proxies; a changed source no longer makes the asset unavailable (as in the Phase 2 gate) | Proxies and storage |

**Specs that observe the resting legacy engine** (with the engine presenting, the
WebGL compositor, video elements, Web Audio, and the legacy RAM preview rest by
design):

| Tests | What they read | Owner |
|---|---|---|
| `video-frame-integrity.spec.ts:33`, `:63`, `:99`, `video-scrubbing.spec.ts:7`, `video-playback-regression.spec.ts:79`, `:128` (×2) | `PM.GL.render` hooks and `gl.readPixels`. `:33` and `:63` passed in the Phase 2 gate; the likely reason is that the engine now draws what was routed to the legacy compositor then (motion blur since the motion lane), not verified | Parity and QA |
| `video-playback-regression.spec.ts:6` | The asset's `<video>` element | Parity and QA |
| `import-mp4.spec.ts:166`, `playback-audio.spec.ts:15` | Web Audio voices (`dataset.audioVoices`) | Audio, Parity and QA |
| `import-svg.spec.ts:7` | `PM.GL.previewViewport` (the legacy high-zoom vector viewport) | Parity and QA |
| `ae-audit-implementation.spec.ts:50` | `PM.Preview.cache()` and `PM.Preview.active` (the legacy RAM preview) | Parity and QA |
| `media-replacement.spec.ts:91` | `PM.renderFrameTo` (the legacy capture: "The GPU could not capture the export frame"); the kernel API mapping is Phase 3c | Parity and QA (Phase 3c) |

**Quarantined** (fail with the flag off too, `quarantine.json`):
`ae-audit-implementation.spec.ts:9`, `agent-media-tools.spec.ts:42`,
`precomp.spec.ts:46`.

Phase 2's flag-on failures that pass now: `export-video-frames.spec.ts` (D1 fixed),
`export-mp4.spec.ts:61` (D2 fixed), `composited-video-sync.spec.ts:9`,
`viewer-recovery.spec.ts:18` and `:128`, `project-file-media.spec.ts:9` and `:65`,
`viewer-tools.spec.ts:140` (D3 did not recur in this run).

## Other checks run

- `cargo check --workspace --tests` clean after the merge.
- `cargo test --release -p pm-node -p pm-cli`: 95 passed, 3 ignored (measurements).
- Vitest, main and shared (`src/main/engine`, `src/shared`): 375 of 375.

## Gaps

- **Base-machine rows**: deferred (4K ProRes export on base; the memory gate's 8 GB
  form).
- **Blender compositions with imported glTF models** route to today's renderer (23 of
  158 corpus snapshots not built), so the 3D export row is measured with primitives.
- **The integration base was not re-measured** for A1 (J/K/L 2× reverse); the
  attribution to the audio lane rests on the failure's shape and on the formats merge
  not touching the playback path.
- **HEVC with alpha** is judged at different file sizes (the engine's 4.3× larger);
  the bar does not fix the data rate.
- **The disk filled up** at about 22:15 (129 MB free on the data volume) while other
  lanes ran; this lane removed its own `engine/target` (20 GB) and the 3D export was
  measured again. The temporary folder held 88 leftover `powermove-claude-*` profiles
  of e2e launches (about 369 MB each, 13 GB, dated 2026-10-12 to 10-17), which the
  hidden harness leaves behind when a run is interrupted; not removed here (owner:
  Parity and QA, the e2e harness).
- **`export_video` presets** were checked by main's unit tests (`bridge.test.ts`,
  in the 375), not through `scripts/test-mcp.mjs --desktop`, which has no preset case.

## Reproducing

```sh
source engine/env.sh
bun install
cd apps/desktop && node scripts/build-engine-addon.mjs \
  && node ../../engine/crates/pm-wasm/scripts/build.mjs && bun run build
L=~/Library/Caches/powermove-engine/bin/pm-electron-lock

# Conformance and bars (no Electron)
cd ../../engine
cargo test --release -p pm-media --test formats --test random_access --test index_crosscheck \
  --test probe --test audio_decode --test images -- --include-ignored --nocapture --test-threads=4
cargo test --release -p pm-render --test footage --test stills --test video_end --test scene3d_gltf \
  --test blender_snapshots --test blender_composite --test retime -- --nocapture --test-threads=4
cargo test --release -p pm-export --test bars_delivery --test delivery --test formats --test videotoolbox \
  --test job_export --test blender_export --test bars --test time_bar -- --nocapture --test-threads=2
PM_MODEL_CORPUS=~/Library/Caches/powermove-engine/model-corpus \
  cargo test --release -p pm-geom3d --features import -- --include-ignored --nocapture
cargo test --release -p pm-audio --test playback --test sources --test layouts --test mixer \
  --test legacy_parity --test resampler -- --nocapture --test-threads=4
cargo build --release -p pm-cli
node ../apps/desktop/scripts/gate-3b/long-audio.mjs --minutes 190
node ../apps/desktop/scripts/gate-3b/sdr-avfoundation.mjs

# Throughput (measurement, under the lock)
$L cargo test --release -p pm-export --test measure_delivery export_throughput -- --ignored --nocapture

# App runs
cd ../apps/desktop
POWERMOVE_NEW_ENGINE=0 $L bunx playwright test --config playwright.ledger.config.ts --workers=1
$L bunx playwright test e2e/native-engine.spec.ts e2e/native-engine-tracer.spec.ts \
  e2e/native-engine-transport.spec.ts e2e/media-store.spec.ts e2e/engine-export.spec.ts \
  e2e/engine-delivery.spec.ts e2e/engine-blender.spec.ts e2e/engine-present-recovery.spec.ts --workers=1
POWERMOVE_NEW_ENGINE=1 $L bunx playwright test <the 43 files above> --workers=1
$L bunx playwright test e2e/native-engine-transport.spec.ts -g 'J, K, and L shuttle' --repeat-each=10 --workers=1
PM_EXPORT_BASELINE=1 $L bunx playwright test e2e/export-baseline.spec.ts --workers=1
PM_GATE_3B=1 $L bunx playwright test --config scripts/gate-3b/playwright.config.ts --workers=1
```

The 43 flag-on files: `import-animated-image`, `import-converted-formats`,
`import-image-sequence`, `import-mp4`, `import-progress`, `import-prores-alpha`,
`import-prores`, `import-svg`, `export-capabilities`, `export-code`,
`export-destination`, `export-dialog`, `export-missing-effect`, `export-mp4`,
`export-video-frames`, `export-webm`, `playback-audio`, `video-scrubbing`,
`gif-preview-scrubbing`, `transparent-sequence-scrubbing`, `video-frame-integrity`,
`video-playback-regression`, `composited-video-sync`, `project-export-fixture`,
`project-file-media`, `local-media-recovery`, `media-replacement`, `workspace-import`,
`agent-models`, `agent-media-tools`, `blender-motion`, `scene3d`, `viewport3d`, `boot`,
`project-save`, `precomp`, `compositions`, `gpu-recovery`, `viewer-recovery`,
`viewer-tools`, `open-media`, `captions`, `ae-audit-implementation` (each
`e2e/<name>.spec.ts`).

`scripts/gate-3b/` holds this lane's measurements: `long-audio.mjs` (the audio bar's
length line through `pm-cli render`), `sdr-avfoundation.mjs` and `.swift` (the SDR
color bar's AVFoundation reference at half float), `blender-export.spec.ts` (the 3D
export row), and `offline-probe.spec.ts` (E1's probe), the last two run through
`scripts/gate-3b/playwright.config.ts` with `PM_GATE_3B=1`. None opens a visible
window.
