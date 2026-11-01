# S3 — native viewer spike

**Gate: OPEN / NOT PASSED.** The spike source is present, but the complete crate
has not compiled or run. Five isolated Rust tests pass on the required toolchain.
There are **no measured viewer/export ΔE, playback, scrub or screenshot results**.
This is an implementation handoff, not evidence that the architecture passed P0.

## Results

| Requirement | Result | Evidence and limit |
|---|---|---|
| Below-slot EDR viewer | Source implemented; runtime unverified | RGBA16F `CAMetalLayer`, extended-linear Display P3, EDR enabled, three drawables, engine-owned device injected into GPUI. No CAMetalLayer pixels have been observed. BT.2100/PQ and HLG output modes are not implemented. |
| Completion and retirement | 3 policy/lease tests pass; native GPU path unverified | Three retained engine surfaces; one producer/consumer submission at a time; worker waits for producer and consumer completion. Requests carry document revision, sequence and output generation. Resize, display id and headroom changes reject prior stamps. Tests exercise the policy and Arc lease rule, not actual delayed Metal completion. |
| Locked overlays | Source implemented; **0-skew gate unverified** | GPUI publishes normalized safe-area geometry. The engine draws it in the final display pass with the picture. A static GPUI label demonstrates above-viewer chrome. Animated selected-layer transform evaluation and paused selection-gizmo switching are not implemented. |
| Playhead/timecode above-slots | Source implemented; runtime unverified | AppKit above sibling with Rust-driven CALayer/CATextLayer updates. Playback update handler does not call `cx.notify()`. GPUI canvas-paint counter is included in planned reports. No measured paint count or layer/display synchronization result. Timecode is a 120-fps spike formatter, not project timecode. |
| Viewer = export | Reference math tests pass; **SDR ΔE: unmeasured; PQ ΔE: unmeasured** | Diagnostic blit reads the actual RGBA16F drawable before presentation. Independent `pm_export::render::render_frame` renders the same time/size/document, converted by an f64 E-decode/P3 reference, then CIEDE2000. Published Lab reference pair yields 2.0425 within 0.0001; identity/extended-range test also passes. No accuracy run occurred. |
| 60 s corpus soak and scrub | **Unverified** | Harness defaults to 60 s after 1 s warmup, 120 requested Hz. Records CPU-to-consumer-completion frame cost, presentedTime callbacks, missed schedule, zero-time presentations, no drawables, stale rejection, scanout gaps and synthetic Rust seek enqueue→present latency. There are 0 completed soaks and 0 scrub samples. |
| Popout / display moves | Source implemented; native unverified | `--exercise` opens a second GPUI viewer on the exact same device, resizes the main window, requests 120 synthetic seeks and toggles simulated SDR headroom. One physical display exists; no physical mixed-headroom move was possible. No simulation or popout run occurred. |

Hardware inventory (`system_profiler SPDisplaysDataType`): Apple M5, 10 GPU cores,
Metal 4; **1** built-in Liquid Retina XDR display, **3024 × 1964**. This confirms
XDR hardware, not current usable EDR headroom or actual refresh cadence. Runtime
`NSScreen.maximumExtendedDynamicRangeColorComponentValue` and
`maximumFramesPerSecond` queries are in the host; their values are unmeasured.
The configured 120 Hz is a request, not a measured ProMotion result.

## Decisions and their picking rules

| Decision | Rule |
|---|---|
| Use S2's P-DEVICE/P-LAYER present hooks and keep the viewer separate from GPUI's SDR target. | Baseline §3.1.1, §3.1.2 and §3.3. A half-float texture imported into SDR GPUI is not an EDR viewer. |
| Engine selects wgpu Metal; retain its actual device object into metal 0.33 and inject that same object into GPUI. | P-DEVICE, N3. Matching only device names or independently selecting the same adapter is insufficient. Raw pointer shim takes its own retain and does not borrow a HAL guard past its lifetime. |
| Adapt engine-host's render worker, bounded ring, producer completion and resource leases; do not change apps/mac. | Reuse requirement, N5, engine-boundary §2.4–2.5 and §5. No transport, Node, readback/upload or new process in the presentation path. Font plumbing is deferred because the selected public corpus fixture has no text. |
| Decode premultiplied E-encoded engine output, transform linear BT.709 → P3 and write premultiplied linear pixels into a correctly labelled layer. | N3 and colour-row viewer=export. Existing engine-host DisplayTransform writes E-encoded P3, which must not be relabelled extended-linear. No Chromium colour quirk is reproduced. |
| Reuse the engine's current PQ interpretation/tone-map path for the PQ input fixture. | N3 and §1. The current engine path maps PQ into SDR working output. A match on that path would prove viewer/export consistency only, **not HDR mastering preservation**. BT.2100 output, HLG and mastering metadata are outstanding. |
| Clamp the provisional display pass to current headroom and invalidate prior output stamps on headroom/display change. | N1 mixed-headroom safety. This is a spike ceiling, **not a certified BT.2390 output transform**. It cannot satisfy the colour gate for highlights above the current output's range. N3 requires replacing it with the engine's explicit output policy before approval. |
| Keep safe-area overlays in the same engine pass; timecode/playhead update separate native layers without GPUI invalidation. | §3.3, N8. No main-thread per-frame gizmo update or JS presentation dependency. |
| Apply viewer/above-slot frames through S2's CATransaction present hook; use rectangular clipping and translation only. | §3.1.2. This constrained fixture does not certify arbitrary affine placement, nonrectangular clipping, capture, accessibility or hit-order behavior inherited from S2. |
| Schedule against monotonic host time, request timed drawable presentation and measure actual presentedTime callbacks. | §5 correctness. This is an initial harness, **not a display-link scheduler**. Adaptive ProMotion, display-rate transitions and audio-clock scheduling remain open. |
| Keep the work surface solid and neutral, with a small GPUI chrome surface. | KISS, N9 and §3.10. No glass over viewer pixels. |
| Stop heavy compilation below 4 GiB; keep results unverified. | Explicit S3 machine limit. The guard refused the first attempt and later terminated its own Cargo process group when available space crossed the limit again. No other lane's process, target directory or files were removed. |

## Validation performed

- Required **rustc 1.96.1** for the isolated tests and build.
- **5/5 isolated tests pass**: resize/display/headroom/revision retirement;
  latest-seek acceptance; completion-lease ownership; CIEDE2000 reference pair;
  identity plus extended-range conversion.
- `cargo fmt --check`, shell/Python syntax and parent-workspace metadata checks pass.
- The standard S3 manifest is a native workspace member. A separate `isolated/`
  build manifest/lockfile uses the same source to avoid writing the shared native
  lockfile. Its own workspace root is below S3, outside the parent member glob.
- Full Cargo build stopped in dependency compilation at the 4 GiB guard, before
  S3 type checking/linking. Compile errors in the complete crate remain possible.
  No Cargo test, on-screen run, GPU accuracy comparison or pacing run completed.
- The shared target was used throughout. No `cargo clean`, alternate target,
  GPUI vendor edit, apps/mac edit, screenshot commit or push.

The independent tests intentionally require no engine/GPUI dependency rebuild;
they are not a substitute for building and testing the app. They verify policy
and numeric reference code only.

## Reproduction after disk capacity is restored

From repository root:

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
python3 apps/native/spikes/s3/build.py build
zsh apps/native/spikes/s3/test-units.sh

# First validate the visible composition and exercise simulation/popout/resize.
zsh apps/native/spikes/s3/run.sh --verify --exercise --seconds=12 \
  --report=/tmp/s3-exercise.json

# Then one instance only, no popout or seek exercise in the playback timing run.
zsh apps/native/spikes/s3/run.sh --seconds=60 --hz=120 \
  --report=/tmp/s3-soak.json
```

`prepare.ts` obtains an unchanged public engine/corpus shape project and writes
it outside Git. For PQ accuracy, pass `--pq=<PQ MOV path>` from the existing engine
media corpus to the first command. No local HDR video fixture was available in
the expected cache, so even a successful SDR run would leave PQ outstanding.
A missing PQ argument currently omits that row; do not interpret it as a pass.

`run.sh` checks free space, acquires `/tmp/pm-shots/gui.lock` with noclobber, retries
every 30 s and releases only the lock it acquired. Its process has a 540 s timeout
and an exit trap. The binary also requires the S3 lock before opening windows.
The timeout limits each screen batch to under 10 minutes. No GUI lock was
acquired during this implementation because no on-screen run was possible.

For screenshots, while the wrapper holds the lock, use the required window-id
helper and window-only screencapture commands. Use only
`/tmp/pm-shots/s3-*.png`. **Screenshots produced: 0.** Do not commit them.

## Gate work remaining

Build and fix the complete crate on the pinned toolchain, then observe the real
window and verify the raw-device shim with Metal validation. Prove producer and
consumer failure/retirement with delayed GPU completion, not only Arc tests.
Finish animated gizmo binding, paused overlays, full composition hit/clip rules,
project-rate timecode, font service and error/cancellation behavior.

Replace the provisional host timer with display-link pacing and integrate the
engine transport/audio clock. Count missed requested frames, unpresented
drawables and display gaps independently; compare actual display-rate samples.
Report 60 s at the measured display rate: **0 dropped frames**, frame-time p95
**≤ 50% of the frame interval** (4.167 ms at 120 Hz; 8.333 ms at 60 Hz). Measure
cached input→present p95 **≤ vsync + 1** and the paired Electron improvement.
Current synthetic seek timing alone cannot certify physical input latency.

Run SDR and PQ drawable/export comparisons with overlays off, including alpha,
negative/out-of-gamut and >1 values. Add BT.2100/PQ/HLG output and correct
headroom-aware tone mapping; verify XDR presentation beyond SDR. Exercise popout,
resize and simulated moves; repeat physical mixed-display moves when hardware
is available. S2's unpassed native composition gate remains a dependency.

Finally run the required machine/corpus repetitions and paired budgets, and obtain
orchestrator verification and the required Opus review. No delegation, merge,
release, tag or publishing was performed by this implementation worker.

## Run 2026-11-01

**Gate: OPEN / NOT PASSED.** This dated run supersedes the earlier uncompiled
handoff status, which is preserved above. S3 now builds and runs. SDR and PQ-input
working-frame accuracy, safe-area registration, native playback layers, popout
and 120 Hz cadence have measured evidence. Playback continuity and scrub latency
fail; the complete viewer gate is not approved.

### Build and conditions

- Pinned Rust **1.96.1**; only `pm-native-s3-spike` built in the isolated manifest,
  using the shared target and `engine/env.sh`. **Unoptimized dev profile**;
  these are spike measurements, not release-build performance claims.
- Full-package `cargo test -p pm-native-s3-spike`: **5/5 pass**. The same five
  numeric/policy tests also pass in the isolated rustc harness. Formatting passes.
  The existing `block 0.1.6` future-compatibility warning remains.
- Final combined accuracy/popout/resize exercise completed with **Metal API
  Validation Enabled**, no API validation errors, and both accuracy rows passing.
  It rejected **47** stale submissions and recorded **2** unpresented drawables
  during synthetic seeks/resize; it is not a playback-soak pass.
- Apple M5, 10 GPU cores, one built-in XDR display, macOS **27.2 (26B5086k)**,
  AC power. Runtime screen maximum: **120 Hz**. Main drawable: **1280 × 720**.
  Load checks were approximately **5.9–6.4**, below 12. Only one S3 app instance
  ran in each timing batch. No popout, accuracy readback, input driver or overlay
  readback ran in the 120 Hz soaks.
- Public corpus shape-primitives project, unchanged, including its animated
  shape, looped over 2.5 s. This is one real public corpus project; it does not
  substitute for the private, long-form, media-heavy or 39-extension corpora.
- Disk checks started at **12 GiB**, stayed above **10 GiB**, and used the explicit
  4 GiB stop rule. No alternate target, clean, vendor change, release or push.
  Every GUI batch used the noclobber lock and released it on exit.

### Measured results

| Item | Measurement | Verdict and boundary |
|---|---|---|
| EDR / extended-linear P3 | RGBA16Float below-slot; current headroom **1.20× → 2.666667×**, potential **16×**. Actual drawable probe returned **0.5, 1, 2, 4, 8** for those linear inputs. | Surface supports and preserves extended values. Probe is read before WindowServer; physical luminance, gamut mapping and clipping were not measured with an instrument. |
| Safe-area locked overlay | **55/55** captured frames: **0 px** offset on all four edges and **0** overlay-mask pixel mismatches, against coordinates in the same content viewport. | Pass for the safe-area fixture. Readback deliberately slows this diagnostic; it is separate from timing runs. This does not certify a selected-layer gizmo tracking an animated transform. |
| Native playhead/timecode without GPUI redraw | Second 120 Hz soak: **7,187 native-layer updates**, **0 GPUI presents**, **0 GPUI canvas paints** during measurement. First soak: **7,186 / 0 / 0**. | Pass for independent layer updates. Presents are counted at the GPUI composition hook, not inferred solely from render callbacks. Timecode still uses the spike's 120-fps formatter. |
| Viewer versus export, SDR | **921,600 pixels**; ΔE2000 p50 **0.004284**, p99 **0.030891**, max **0.060174**. | Pass against p99 ≤ 1 and max ≤ 2. Actual drawable blit versus an independent `pm_export::render_frame` at the same time and size, overlays disabled. |
| Viewer versus export, PQ input | **921,600 pixels**; ΔE2000 p50 **0.030558**, p99 **0.057133**, max **0.121483**. Export peak linear P3 **1.342959**. | Pass at the export working-frame boundary. Checked-in HEVC 10-bit BT.2020/PQ fixture (256 × 144, 12 fps). This is not a PQ-encoded display layer or an encoded-file/metadata round trip. |
| Display-link pacing | View-bound `CADisplayLink`, requested 120 Hz. Consumed-tick interval p50/p95 **8.333333 / 8.333333 ms**; actual positive `presentedTime` gap p50/p95 **8.333250 / 8.333292 ms** in both 120 Hz soaks. | Measured 120 Hz normal cadence, with continuity failures recorded separately below. No claim that every refresh got a new image. |
| AppKit scrub input → present | **120** external timestamped HID clicks received; **104** seek presentations sampled. p50 **44.600 ms**, p95 **45.701 ms**. Remaining requests were superseded or did not yield a sampled presentation. | Fail. This exceeds both 9.333 ms (interval + 1 ms) and 16.667 ms (two intervals); the ambiguous shorthand “vsync + 1” cannot change the verdict. Start is `NSEvent.timestamp`, end is drawable `presentedTime`; hardware-device latency before event creation is excluded. |
| Popout and resize | Real second viewer **864 × 540** pixels; same engine-selected Metal device; visible playback, native layers and safe area. Main resized **1280 × 720 → 1344 × 676**. | Functional smoke pass. Two windows share the device; their transport clocks are independent spike clocks, not a finished multi-view engine service. |
| Display/headroom retirement | Simulated display id/headroom **1.0 → physical display / 2.666667** and resize exercised; initial exercise rejected **51** stale submissions and returned **1** size-mismatched/no drawable. | Executed, not a physical mixed-display move. Only one screen is available. Existing policy tests pass; delayed-GPU-completion fault injection remains unverified. |

The scrub report above is the final run after publishing the original event
timestamp atomically with its seek request. An earlier input run measured
36.698 ms p95; it is retained only as preliminary evidence, not the final row.
The standalone popout's initial 10 s exercise measured 1,197 frames and
8.286/9.722 ms frame-cost p50/p95; this two-window exercise is not a timing gate.

| 60 s, requested 120 Hz | First soak | Second soak | Rule |
|---|---:|---:|---|
| Measured submitted frames | 7,186 | 7,187 | After one second of warmup |
| Frame cost p50 / p95 (ms) | 3.682 / **8.597** | 3.163 / **3.944** | p95 ≤ **4.167 ms** (50% of 8.333 ms) |
| Unpresented drawables / missing callbacks | 0 / 0 | 0 / 0 | Correctness counters, insufficient alone to claim zero drops |
| Positive scanout gaps > 1.5 intervals | **11** | **6** | **Fail** the zero-drop continuity requirement |
| Display-link sequence ticks skipped by worker | 24 | 22 | Includes warmup; not added to scanout gaps as if independent drops |
| Measured frames costing > one interval | 504 | 11 | Separate from scanout/drop counters |
| No drawable / stale rejection | 0 / 0 | 0 / 0 | Whole-run counters |
| Presentation lateness versus link target p95 | 41.654 ms | 33.313 ms | Latency remains too high despite normal 120 Hz cadence |

Frame cost is worker tick receipt through producer/consumer completion, including
CPU encoding and worker waits; it is not isolated GPU execution time. Scanout gaps
count distinct observed intervals longer than 12.5 ms. Gap counts, skipped ticks
and unpresented drawables overlap in meaning and must not be summed into a
fabricated single drop count. Neither 120 Hz soak passes continuity. Headroom
passed only the second run; the first failure remains in the table.

A supplemental **60 s at 60 Hz** run on the same 120 Hz screen measured
**3,601 frames**, frame cost p50/p95 **5.123/5.793 ms** against the **8.333 ms**
headroom budget, consumed display-link interval **16.666667 ms**, and scanout gap
p50/p95 **16.666500/16.666542 ms**. It had **4** gaps over 25 ms, **2** skipped
link sequence ticks, **0** unpresented drawables or missing callbacks, and
**3,601** native updates with **0** GPUI presents/paints. Headroom passes;
continuity fails. This is a 60 Hz request on XDR, not the required base-Mac test.

### Fixes and decisions (including the picking rule)

| Change / decision | Rule and evidence |
|---|---|
| Fix private `CGSize`, disambiguate the raw window handle, retain Metal shader functions in locals, and add the direct geometry-type dependency. | Compile the existing architecture on the pinned toolchain, without touching shared vendor code. Full package now builds and tests. |
| Set the FFmpeg library path in the Python launcher's explicit child environment, including Cargo tests. | Reproduce and fix the dyld failure. System Python strips inherited `DYLD_*`; setting it only in the shell did not work. |
| Preserve signed and >1 linear P3 output; remove the provisional viewer clamp. | N3 / viewer=export: the original clamp produced PQ p99/max **8.279/13.399**. Removing it yields **0.057/0.121**. A transient screen-headroom query must not silently alter authored/export pixels. WindowServer handles display mapping; a certified engine BT.2390 output policy remains required. |
| Use `NSView`'s display link and requested `CAFrameRateRange`, with one bounded queued tick and explicit invalidation on teardown. | §3.3 / §5: pacing must be driven by display timing and checked against actual presented timestamps. No host sleep is represented as proof of 120 Hz. The bounded queue can still retain an old tick under load; latest-tick scheduling is follow-up work. |
| Make the native timecode/playhead above-slot view pass pointer hits through; route GPUI clicks/drags to seek. Post test events through HID. | Composition hit order / KISS. Process-directed event posting produced zero input samples and is not accepted as a measurement. The final path receives all 120 events. |
| Publish the source event timestamp and request together under one lock. | Accuracy of input→present: the worker must never consume a provisional enqueue timestamp while AppKit later fills in the origin. |
| Count GPUI presents, paints, native updates, skipped schedule ticks and actual presentation gaps separately. | N2 / §5: zero unpresented drawable callbacks cannot hide skipped frames or GPU headroom failures. |
| Wait for playback readiness before exercise resize/display simulation. | Correctness of the accuracy oracle: Metal validation made readbacks slow enough that the old exercise resized during the comparison and failed with a size-mismatched/no drawable. Exercise must not invalidate the accuracy fixture mid-comparison. |

### Reproduce and remaining gate work

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
python3 apps/native/spikes/s3/build.py build -p pm-native-s3-spike
python3 apps/native/spikes/s3/build.py test -p pm-native-s3-spike
zsh apps/native/spikes/s3/test-units.sh
zsh apps/native/spikes/s3/run.sh --verify --seconds=5 \
  --pq="$PWD/engine/crates/pm-render/tests/fixtures/video/hevc10-2020-pq.mp4" \
  --report=/tmp/s3-accuracy.json
zsh apps/native/spikes/s3/run.sh --measure-overlays --seconds=3 \
  --report=/tmp/s3-overlays.json
python3 apps/native/spikes/s3/input.py
zsh apps/native/spikes/s3/run.sh --seconds=60 --hz=120 \
  --report=/tmp/s3-soak.json
```

Raw reports/logs remain outside Git under `/tmp/s3-*`. Window-only screenshots:
`/tmp/pm-shots/s3-viewer.png` and `/tmp/pm-shots/s3-popout.png`. No screenshot,
private project, private plan or operational dump is committed.

Still unverified or incomplete: BT.2100/PQ and HLG **output** layers and metadata;
certified BT.2390 output policy; a broader alpha/out-of-gamut accuracy fixture
set; physical mixed-headroom display moves; animated
selected-layer gizmos and paused selection switching; native project-rate
timecode and presentation synchronization of the separate chrome layers;
delayed producer/consumer failure injection; audio-clock/A/V behavior; full
composition/accessibility/capture inherited from S2; release-build profiling;
paired Electron margins; required hardware/corpora and ≥30 repetitions; and
orchestrator/Opus review. Fix scrub/presentation latency and all continuity gaps,
then repeat the gates. This run does not authorize promotion or release.
