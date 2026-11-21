# Powermove Electron vs Powermove Native — benchmark on one macOS VM

## Summary

On this VM, **Native wins cold launch, memory and edit-scene exports; Electron wins everything interactive and the graphics-heavy exports.** After Effects was not run (owner decision), so there are no AE numbers.

- **Launch:** Native starts in 1.28 s vs Electron's 2.37 s (0.54×, about 1.9× faster).
- **Playback:** Electron renders ~29 of 30 fps in all three scenes; Native renders only 10.5–11.2 fps (0.36–0.39×), i.e. it skips about 190 of 300 frames in 10 s. This is the biggest gap.
- **Scrub:** Electron answers a seek in 33–67 ms (median); Native takes 155–189 ms (2.8–5.2× slower).
- **Open → first frame:** Electron 0.03–0.08 s, Native 1.05–1.39 s. Not like for like: Electron opens from its project library (its `.pmv` open is broken, see Bugs), Native parses the `.pmv` file.
- **Export, scene B (video edit):** Native is about 2.9× faster for H.264 (3.9 s vs 11.1 s, 0.35×) and 2.3× faster for ProRes 4444 (6.6 s vs 15.3 s).
- **Export, scenes A and C (graphics/effects):** Native is slower. A: H.264 1.9× slower (17.2 s vs 8.9 s). C: H.264 3.4× slower (63.7 s vs 18.7 s), ProRes 4444 3.8× slower.
- **Memory:** Native uses less in almost every case, most on the video edit. Scene B: 578 MB idle vs 1322 MB (0.44×), 342 MB vs 2405 MB peak during H.264 export (0.14×). The one exception is scene A playback (706 vs 689 MB, 1.02×).
- **Bugs that affect use:** Electron can't open `.pmv` files from disk in this production build. Native's ProRes exports drop the audio track.


**After Effects: not run (owner decision).** The owner's Adobe sign-in goes through a school SSO portal that does not work on this VM, and the owner then decided not to run AE anywhere. Every AE column below says so. `bench/ae/build_scenes.jsx` (an untested ExtendScript port of the three scenes) is kept in case AE is ever added later.

## Machine card

| | |
|---|---|
| Machine | Apple Virtual Machine 1 (`VirtualMac2,1`), a macOS VM |
| CPU | Apple M4 Pro (Virtual), 12 cores |
| RAM | 16 GB |
| GPU / Metal device | `Apple Paravirtual device`: **paravirtualized** GPU, unified memory, Metal recommended working set ≈ 11.45 GB |
| macOS | 26.6.2 (25G83) |
| Display | 1600×1200 points, backing scale 1.0 (no Retina) |

**This is a VM.** Its GPU is paravirtualized and its display is a virtual 1× screen, so absolute numbers say little about a physical M-series Mac (where both apps will be faster, and playback in particular can behave differently). The result is the **ratio between the two apps on the same VM**, run back to back.

## What was compared

| | Build | Source |
|---|---|---|
| Powermove Electron (v1) | production app, `electron-builder --mac --dir`, unsigned, Electron 44.0.0 arm64, app 1.1.3 | `main` @ `ea49609cb` |
| Powermove Native | `cargo build --locked --release -p pm-native-app --bin powermove-native`, `pm-glsl-compiler` (release) beside it | `backup/integration-19` @ `54d18fe07` |

No app code was changed. One harness-only change to the Electron app copy is described under Method.

## Test material

All media is synthetic (ffmpeg `testsrc2` / `mandelbrot` / `smptehdbars` / `rgbtestsrc` plus noise; sine-wave audio), made by `bench/make_media.sh`. The scenes are 1920×1080, 30 fps, 10 s, built by `bench/scenes.js` inside Electron with the app's own model factories (`PM.mkLayer`, `PM.mkEffect`, `PM.mkTransition`) and saved as `.pmv` with the app's own Save (`PM.Export.run({format:'json'})`).

- **A, motion graphics:** 40 keyframed layers (20 text, 20 shapes: ellipses, rounded rects, stars), 24 in the root and 8 in each of **2 precomps**. Every layer has linear position, scale and opacity keyframes. **Gaussian blur on 4 layers** (amount 6/10/14/18). The two precomp layers are also keyframed.
- **B, edit:** 6 H.264 1080p30 clips (4 s each, ≈20 Mbps, `libx264`), cuts at 2.0 / 5.4 / 8.5 s, **2 crossfades** of 0.5 s (into clip 3 at 3.2 s and clip 5 at 6.5 s), and one stereo audio track (10 s WAV).
- **C, effects stress:** 10 animated shapes (rects/ellipses 300×300, moving and rotating), each with **blur** (8), **glow** (threshold 40, radius 40, intensity 150) and **levels** (in black 10, in white 90, gamma 1.2), plus **one adjustment layer** with a colour adjustment (saturation 30, contrast 20).

**Both apps open the same `.pmv` files.** Native opens them through its v1 compatibility reader: all three scenes opened, played, scrubbed and exported in Native with no conversion, and the H.264 contact sheets of the two apps match frame for frame (checked by eye on every scene; see Output checks). So there are no structural differences between the Electron and Native versions of the scenes.

## Method

`bench/run_all.sh <Electron .app> <powermove-native> <electron apps/desktop dir> <media dir> <out dir> [runs]` runs everything; paths are parameters, so it runs on any Mac. Each run is a **fresh process with a fresh, isolated profile** (cold launch), opens one scene, then measures in order: idle memory, playback, scrub, exports. Electron and Native alternate scene by scene. **6 runs per app per scene; run 0 is the warm-up and is discarded; the tables show the median and min–max of runs 1–5.** Cold launch is measured on every run (15 samples per app).

- **Drivers.** Electron: `bench/electron_bench.mjs` through Playwright's Electron launcher (the same mechanism as the repo's `tools/perf-gate/electron.mjs`). Native: `bench/native_bench.py` through the app's opt-in perf gate (`PM_PERF_GATE=1`, the JSON stdin/stdout protocol used by `tools/perf-gate`) and its engine trace (`PM_ENGINE_TRACE`); Native exports run through the app's headless MCP host (`--powermove-mcp`, `export_video`), which uses the same export child as the Export panel. The Electron `--powermove-mcp` path was not used: renderer-level timing (`preview:presented`) is only reachable through the page.
- **Harness-only change (Electron).** The production app disables Node's inspector with an Electron fuse, which Playwright needs. `bench/electron_inspectable_copy.mjs` copies the app and flips only `EnableNodeCliInspectArguments` (ad-hoc re-signed). The asar and every other fuse are unchanged.
- **Opening projects in Electron.** Opening any `.pmv` from disk fails in this production build (see Bugs). Electron runs therefore open the same project from the app's own project library (`PM.openProjectHere`, the Recents path), in a fresh copy of the profile in which the scenes were built and saved. Before the timed open, a blank project is made current so the target is never the session-restored one. This favours Electron's open time slightly (no file parse or media copy from the `.pmv`); Native opens the `.pmv` file itself.
- **Cold launch:** Electron: process spawn → `PM.projectReady` + builtins ready + two painted animation frames with the window visible. Native: process spawn → the perf gate's `controller-ready` and `window_visible`.
- **Open → first frame:** Electron: open call → first `preview:presented` of the new project + one animation frame. Native: perf-gate `open` accepted → first GPU-complete frame (`completion_host_s`) of the new document.
- **Playback** (10 s from frame 0, Fit zoom, window as launched): number of **distinct content frames rendered** during the 10 s ÷ elapsed time; "not rendered" = expected frames − distinct frames. Electron counts `preview:presented` events (frame submitted to the GPU and completed, next animation frame); Native counts engine-trace frame rows with `gpu_ok` during `Playing`. **True presented-to-screen fps is n/a for both**: Native's `display` (CAMetalDrawable presented time) is null on this VM's virtual display for every frame, and Electron/Chromium has no per-frame present timestamp for the page. Both numbers are therefore "frames rendered", the same kind of endpoint in both apps.
- **Scrub:** 60 seeks to random frames (fixed seed), each waited until that exact frame is rendered; latency = seek request → GPU-complete frame (Native) / → `preview:presented` of that frame (Electron). Electron's value includes waiting for the next animation frame, so it cannot go below one display refresh (~16.7 ms) and is effectively quantized to frames.
- **Export:** H.264 1080p (`mp4`) and ProRes, whole 10 s, with audio, timed from export call to finished file. Electron's ProRes is hard-coded to **ProRes 4444** (`prores_ks -profile:v 4`, 10-bit 4:4:4), so its ProRes 422 is **n/a**; Native exports both ProRes 422 (as requested) and ProRes 4444 (the like-for-like comparison).
- **Memory:** RSS summed over the app's whole process family (Electron: every PID from `app.getAppMetrics()`, i.e. main, GPU, renderer and utility helpers; Native: the app and all child processes, including the `--engine-job` export child), sampled every 200 ms. Idle = 3 s after open. RSS double-counts shared pages between helpers and does not include GPU memory that is not mapped into the processes; Native also reports Metal allocation, listed in the raw data.

## Results

### Cold launch (all runs)

| Metric | Electron | Native | AE | Winner | Native ÷ Electron |
|---|---|---|---|---|---|
| Cold launch → usable editor window (s) | 2.37 (1.82–8.37) | 1.28 (0.789–1.67) | not run (owner decision) | Native | 0.54× |

### Scene A — motion graphics

| Metric | Electron | Native | AE | Winner | Native ÷ Electron |
|---|---|---|---|---|---|
| Open → first rendered frame (s) | 0.063 (0.047–0.102) | 1.05 (0.777–1.89) | not run (owner decision) | Electron | 16.60× |
| Playback, rendered fps (of 30) | 29.23 (21.33–29.45) | 10.49 (9.19–10.60) | not run (owner decision) | Electron | 0.36× |
| Playback, frames not rendered (of ~300) | 8.00 (6.00–88.00) | 195 (194–208) | not run (owner decision) | Electron | 24.38× |
| Scrub, median per seek (ms) | 33.40 (33.30–71.50) | 174 (142–186) | not run (owner decision) | Electron | 5.22× |
| Scrub, p95 per seek (ms) | 36.30 (35.70–275) | 201 (149–253) | not run (owner decision) | Electron | 5.55× |
| Export H.264 1080p (s) | 8.94 (7.84–41.68) | 17.16 (13.84–21.91) | not run (owner decision) | Electron | 1.92× |
| Export ProRes 422 (s) | n/a: v1 exports ProRes 4444 only | 14.50 (13.51–25.27) | not run (owner decision) | — | — |
| Export ProRes 4444 (s) | 8.57 (7.72–17.95) | 14.83 (13.72–26.97) | not run (owner decision) | Electron | 1.73× |
| Memory idle after open (MB) | 636 (631–654) | 578 (571–583) | not run (owner decision) | Native | 0.91× |
| Memory peak, playback (MB) | 689 (680–699) | 706 (701–714) | not run (owner decision) | Electron | 1.02× |
| Memory peak, H.264 export (MB) | 959 (932–976) | 415 (376–455) | not run (owner decision) | Native | 0.43× |
| Memory peak, ProRes 4444 export (MB) | 944 (932–1017) | 617 (492–664) | not run (owner decision) | Native | 0.65× |

### Scene B — edit

| Metric | Electron | Native | AE | Winner | Native ÷ Electron |
|---|---|---|---|---|---|
| Open → first rendered frame (s) | 0.080 (0.045–0.104) | 1.19 (1.07–1.55) | not run (owner decision) | Electron | 14.99× |
| Playback, rendered fps (of 30) | 28.42 (20.79–29.24) | 11.18 (10.90–11.19) | not run (owner decision) | Electron | 0.39× |
| Playback, frames not rendered (of ~300) | 16.00 (8.00–92.00) | 188 (188–191) | not run (owner decision) | Electron | 11.75× |
| Scrub, median per seek (ms) | 67.00 (66.60–135) | 189 (181–245) | not run (owner decision) | Electron | 2.82× |
| Scrub, p95 per seek (ms) | 87.70 (86.40–435) | 453 (331–460) | not run (owner decision) | Electron | 5.16× |
| Export H.264 1080p (s) | 11.05 (10.19–43.05) | 3.87 (3.37–7.69) | not run (owner decision) | Native | 0.35× |
| Export ProRes 422 (s) | n/a: v1 exports ProRes 4444 only | 4.28 (3.63–7.10) | not run (owner decision) | — | — |
| Export ProRes 4444 (s) | 15.25 (12.53–44.82) | 6.63 (4.19–11.66) | not run (owner decision) | Native | 0.43× |
| Memory idle after open (MB) | 1322 (1266–1607) | 578 (458–585) | not run (owner decision) | Native | 0.44× |
| Memory peak, playback (MB) | 1705 (1642–1839) | 625 (500–642) | not run (owner decision) | Native | 0.37× |
| Memory peak, H.264 export (MB) | 2405 (2320–2456) | 342 (298–384) | not run (owner decision) | Native | 0.14× |
| Memory peak, ProRes 4444 export (MB) | 2470 (2348–2546) | 721 (657–775) | not run (owner decision) | Native | 0.29× |

### Scene C — effects stress

| Metric | Electron | Native | AE | Winner | Native ÷ Electron |
|---|---|---|---|---|---|
| Open → first rendered frame (s) | 0.029 (0.023–0.033) | 1.39 (1.29–1.74) | not run (owner decision) | Electron | 47.75× |
| Playback, rendered fps (of 30) | 29.03 (19.77–29.37) | 10.99 (10.89–11.09) | not run (owner decision) | Electron | 0.38× |
| Playback, frames not rendered (of ~300) | 10.00 (6.00–104) | 190 (189–191) | not run (owner decision) | Electron | 19.00× |
| Scrub, median per seek (ms) | 33.90 (33.60–77.40) | 155 (154–165) | not run (owner decision) | Electron | 4.58× |
| Scrub, p95 per seek (ms) | 51.40 (50.80–248) | 173 (160–186) | not run (owner decision) | Electron | 3.37× |
| Export H.264 1080p (s) | 18.73 (15.64–34.14) | 63.73 (54.02–74.98) | not run (owner decision) | Electron | 3.40× |
| Export ProRes 422 (s) | n/a: v1 exports ProRes 4444 only | 77.09 (53.83–80.25) | not run (owner decision) | — | — |
| Export ProRes 4444 (s) | 18.29 (15.35–23.40) | 69.32 (53.55–77.10) | not run (owner decision) | Electron | 3.79× |
| Memory idle after open (MB) | 623 (614–630) | 596 (593–607) | not run (owner decision) | Native | 0.96× |
| Memory peak, playback (MB) | 676 (675–682) | 641 (630–646) | not run (owner decision) | Native | 0.95× |
| Memory peak, H.264 export (MB) | 935 (920–943) | 321 (318–382) | not run (owner decision) | Native | 0.34× |
| Memory peak, ProRes 4444 export (MB) | 919 (906–937) | 551 (507–571) | not run (owner decision) | Native | 0.60× |

Ratios are Native ÷ Electron medians: below 1 means Native is lower (faster or smaller), except for fps, where above 1 means Native renders more frames.

## Output checks

`bench/verify_outputs.sh` probes every render (codec, profile, size, decoded frame count, duration, audio) and writes one contact sheet per output (1 frame per second); `bench/results/verify.csv` holds the probe results. Contact sheets and renders are not committed.

All **99** renders checked are 1920×1080, exactly **300 frames**, **10.00 s**. That is 3 scenes × 6 runs × 5 outputs (Electron 2, Native 3), plus 9 from a run that was cut short and then redone (see Caveats). Formats:

| Output | Codec / profile | Pixel format | Audio (scene B) |
|---|---|---|---|
| Electron H.264 | H.264 High | yuv420p | AAC |
| Electron ProRes | ProRes 4444 | yuv444p12le | PCM 24-bit |
| Native H.264 | H.264 High | yuv420p | AAC |
| Native ProRes 422 | ProRes 422 (Standard) | yuv422p10le | **none** (bug) |
| Native ProRes 4444 | ProRes 4444 | yuv444p12le | **none** (bug) |

Scenes A and C have no audio tracks, so their renders have no audio in either app (as expected). I compared the contact sheets by eye: Electron and Native outputs of the same scene match frame for frame (layout, precomps, blur, crossfades at 3.2 s and 6.5 s, glow, levels, adjustment layer). I found no wrong renders.

## Bugs found

1. **Electron v1: opening any `.pmv` from disk fails** in the production build (`main` @ `ea49609cb`). Both macOS open-file and the renderer's file-open path (`PM.openFiles`) show the toast *"Could not open project: (intermediate value).cloudFileState is not a function"*, and the project is not opened. The cause: `src/main/file-cloud-state.ts` calls `(await import('@powermove/macos-haptics')).cloudFileState(file)`, but that package is CommonJS and exports an object literal with method shorthand (`module.exports = { cloudFileState(...) {...} }`). Node's CJS named-export detection does not see that, so the function exists only on `.default`. Repro: build with `electron-builder --mac --dir`, launch, File → Open… any `.pmv` (or `open -a Powermove.app scene.pmv`). Workaround used here: open from the project library (see Method).
2. **Native: ProRes exports drop the audio.** `export_video {format:"prores", profile:"422"|"4444", audio:true}` of scene B gives a `.mov` with no audio stream, in all 13 Native ProRes renders of scene B. The H.264 export of the same project has AAC audio. Repro: open `scene-B.pmv` in Native, export ProRes with audio on, then run `ffprobe -show_streams`.
3. **Native playback skips frames on this VM.** It renders about 11 fps of 30 in every scene, including the simple video edit. The engine trace shows content frames advancing 2–6 frames at a time. This is recorded as a result, not as a crash.
4. Not a bug, but a limitation: Electron's ProRes export is hard-coded to ProRes 4444 (`src/main/render-encoder.ts`, `-profile:v 4`), so it cannot produce ProRes 422.

No crashes or hangs in either app over the 36 app-runs (6 runs × 3 scenes × 2 apps, warm-ups included).

## Caveats

- VM with a paravirtualized GPU and a 1× virtual display; absolute numbers are not those of a physical Mac (see Machine card).
- Presented fps is n/a for both apps (see Method); the playback numbers are rendered frames.
- The endpoints for launch, open and scrub are each app's own "frame ready" signal and are not the identical code point in both apps. Electron's scrub includes one animation-frame wait.
- Electron opens scenes from its project library because opening `.pmv` files from disk is broken in this build; Native opens the `.pmv` file.
- Electron's ProRes is 4444 only; ProRes 422 is compared on Native alone.
- RSS is a summed per-process figure and double-counts shared memory; treat the memory rows as rough.
- 5 measured runs per cell; the ranges show the spread. Some Electron maxima are outliers (e.g. one 41.7 s H.264 export in scene A); medians are robust to them.
- Every benchmark process ran at `nice 10` (the automation shell's default), the same for both apps, on an otherwise idle VM.
- The campaign was interrupted once, at run 2, when ProRes 4444 renders filled the VM's disk (Native reported "destination disk is almost full"). All run-2 measurements were discarded and runs 2–5 were redone with the same scenes and binaries. The run script now deletes each render after checking it.
- While building Scene C, levels was first set on the wrong scale (0–100 instead of the effect's 0–1), which rendered black in both apps. It was fixed before the measured campaign; the numbers above use the corrected scene.
- AE was not run (owner decision); `bench/ae/build_scenes.jsx` is untested.

## Files

- `bench/run_all.sh`: full campaign. `bench/make_media.sh`: synthetic media. `bench/scenes.js`: scene definitions. `bench/electron_bench.mjs`, `bench/electron_inspectable_copy.mjs`, `bench/native_bench.py`: drivers. `bench/verify_outputs.sh`: output checks. `bench/parse_results.py`, `bench/make_tables.py`: CSV and tables.
- `bench/results/raw.csv` (every measurement), `bench/results/summary.csv` (median/min/max), `bench/results/verify.csv`.
- `bench/ae/build_scenes.jsx`: untested AE port of the scenes.
