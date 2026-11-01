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

## S3b (2026-11-01)

**Gate: OPEN / NOT PASSED.** This follow-up instruments and changes the scrub and
playback paths. The Mac stayed locked throughout. Offscreen GPU completion and
hidden-window drawable callbacks are explicitly separate from physical display
presentation; neither is accepted as a replacement for a foreground gate run.
Earlier results above remain the before measurements.

### Root causes and fixes

- A scrub request previously waited for a display-link tick, then performed a
  fresh full-resolution render even when revisiting an unchanged frame. Input
  now wakes the render worker immediately through a seek-priority mailbox.
  Eight exact working frames are cached by revision, time and physical size;
  display conversion and locked overlays are still applied in the final pass.
  Drawable loss or output retirement retries a paused seek without replacing
  newer input or resetting the original latency timestamps. Output changes also
  repaint a paused selection, without counting that repaint as a new input.
  Revision/size changes retire cached entries. This spike owns one immutable
  composition per engine; changing scenes clears the cache. There is no proxy
  approximation or change to export pixels.
- The one-item tick channel preserved an **old** tick while rejecting newer
  ones. It now replaces the pending tick with the newest one. Foreground
  playback also extrapolates deadlines from observed display-link phase on the
  worker, so a delayed AppKit callback does not itself stop render scheduling.
  Predicted frames, received callbacks, skipped callback sequences and expired
  render targets have separate counters. This is a silent spike clock, not the
  finished audio-clock service.
- The producer and consumer previously used separate Metal queues and two
  synchronous GPU waits per frame. The final pass now uses the engine's actual
  Metal command queue and tracked source-texture dependencies. Normal rendering
  polls completions without blocking; at most two submissions remain in flight.
  Completion handlers retain source leases. Waits remain for diagnostics,
  shutdown, obsolete producer retirement and bounded backpressure. The source
  pool is bounded to eleven surfaces (eight cached plus in-flight/free slots).
- The worker now requests interactive QoS. Encoding and presentation both have
  per-frame autorelease pools. Pipelines, offscreen destination and reusable
  source surfaces survive between frames. Trace records intentionally allocate
  per frame in this diagnostic harness; this is not a production allocation
  profile.
- The layer requests **two** drawables to bound FIFO latency. Hidden-window
  measurements show that acquisition can still consume substantial time; this
  is a latency experiment awaiting a foreground headroom/continuity check,
  not proof that drawable starvation has been eliminated.

The old **44.600 / 45.701 ms** AppKit-to-display result cannot be decomposed
exactly while locked. Waiting for a tick, unnecessary rendering, synchronous
completion and old queued ticks are confirmed code-path defects; their removal
is testable offscreen. How much of the remaining delay belonged to drawable
FIFO, WindowServer and display-link phase requires the new foreground trace.
No claim is made that all **6–11** earlier scanout gaps have been explained or
fixed by the headless evidence.

### Instrumentation and decisions

Each submitted frame writes monotonic host timestamps for input, request
publication, worker start, render start/submission, producer readiness bound,
drawable request/acquisition, encode end, presentation request, command commit,
scheduling callback, consumer GPU start/end, completion callback, actual drawable
`presentedTime` and display callback receipt. A one-texel source blit makes the
producer marker depend on the tracked source texture; an empty Metal command
buffer would not establish that dependency. Producer readiness is an upper
bound, not isolated producer GPU execution time. Raw traces are written beside
the report as `.trace.json`, outside Git.

Missing/zero display times never become GPU completion times. Empty latency
samples and unobservable scanout gaps are null. Scrub cache hits have their own
distribution, excluding the initial misses. Synthetic input comes from a
separate thread through the same control mailbox/wake; it does not include
AppKit/HID delivery. Real AppKit runs still start at `NSEvent.timestamp`.

| Decision | Picking rule |
|---|---|
| Exact cached frames and immediate seek priority; pause at the selected frame instead of replacing it on the next playback tick. | N1/N2 and §5 cached input-to-present budget; N3 forbids an approximate cache hit. No low-resolution stand-in was needed for the cached fixture. |
| Same-device, same-queue rendering, bounded submissions and retained leases, with revision/output stamps checked before submission. | §3.3 lifecycle and N8; adapt the existing engine presenter policy without changing engine crates or adding transport. |
| Worker deadlines use display-link phase, count expired targets, and keep seek wakeups independent. | §5 zero-drop constraint; a latest-tick queue alone cannot mask missing main-thread callbacks. |
| Keep `presentsWithTransaction=false`; present scrub frames ASAP and playback at explicit host targets. Do not add `afterMinimumDuration`. | N2/N8 low input latency; locked overlays already share the engine pass. Transaction presentation adds main-thread coordination, and minimum-duration presentation can defer an interactive replacement. See [Apple's transaction presentation contract](https://developer.apple.com/documentation/quartzcore/cametallayer/presentswithtransaction) and [timed drawable presentation](https://developer.apple.com/documentation/metal/mtldrawable/present(at:)). |
| Separate a GPUI-free headless binary, synthetic-clock offscreen runs, hidden-window drawable runs and foreground runs. | N7 and §5 evidence integrity: GPU completion is not scanout. No screen unlock, screenshot or HID posting occurred in this follow-up. |

### Measurements and verification

All final timing cases used the unchanged public shape fixture, **1280 × 720**,
one S3 process, one second of warmup and the pinned **Rust 1.96.1** toolchain.
Release offscreen cases sampled one-minute load every second: maxima were
**11.30 / 7.49 / 5.45** for sync-120 / async-120 / async-60 and below **4.90** for
both scrub cases. These are single runs, not the required ≥30 repetitions or a
paired Electron/hardware/corpus gate. Source rendering remains full resolution.

| Final offscreen, optimized, 60 s | Sync comparison, 120 Hz | Async, 120 Hz | Async, 60 Hz |
|---|---:|---:|---:|
| Measured frames | 7,113 | 7,188 | 3,589 |
| Worker → completion callback p50 / p95 (ms) | 3.326 / **5.563** | 2.367 / **4.085** | 1.856 / **2.936** |
| Worker → GPU done p50 / p95 (ms) | 3.234 / 5.459 | 2.187 / 4.005 | 1.662 / 2.754 |
| Offscreen headroom (4.167 / 8.333 ms budgets) | **Fail** | **Pass, render boundary only** | **Pass, render boundary only** |
| GPU failures | 0 | 0 | 0 |
| Synthetic timer sequences skipped, including warmup | 92 | 13 | 11 |
| Physical scanout / continuity | Unobserved | **Unverified** | **Unverified** |

The sync comparison is the **same instrumented code** with `--legacy-sync
--no-cache`, not a replay of the complete original S3 presenter. The rows have
separate load conditions and no paired-run confidence interval. Completion cost
includes CPU encoding, GPU work and callback delivery, without a drawable wait
in offscreen mode. Skipped synthetic timer slots prevent even these runs from
being advertised as uninterrupted 120/60 Hz delivery. Offscreen GPU completion
does not include the layer's FIFO or compositor delay.

| Scrub boundary | Before | S3b result and verdict |
|---|---|---|
| AppKit event → actual display | **44.600 / 45.701 ms** p50/p95; fail | **Unverified** while locked; no substitute endpoint is used. |
| Separate-thread synthetic input → GPU done, cache disabled | New diagnostic | **1.537 / 3.376 ms**, 120 requests. This is not AppKit input or display latency. |
| Separate-thread synthetic input → GPU done, exact cache hits | No previous cached-frame path | **0.600 / 3.031 ms**, **112 hits / 120 requests**; initial eight misses excluded from this row. |
| Cached request → worker; worker → commit | New diagnostic | p50/p95 **0.024 / 0.053 ms** and **0.123 / 0.207 ms**. No render submission on the 112 cache hits. |
| Cached physical gate (≤9.333 ms at 120 Hz and ≤60% Electron) | Fail in prior AppKit run | **Unverified**. GPU-only latency cannot pass this row. |

Initial instrumented **dev** diagnostics also failed 120 Hz headroom: sync p95
**6.743 ms**, async **7.988 ms**; async at 60 Hz was **6.733 ms**. The worst async
CPU render-encoding span was **183.172 ms**, versus consumer GPU execution p95
**1.123 ms**. Producer synchronous-wait p95 fell to **0.007 ms** on the async
path, but CPU/scheduling tails remained. These diagnostics preceded the final
producer marker and worker-phase updates; the failed 120 Hz rows are retained
without claiming a regression-free comparison. One launcher was edited while a shell was
executing it, causing that wrapper to fail after writing its 60 Hz report. The
final optimized matrix and hidden run all exited successfully with the launcher
unchanged during execution.

Final **hidden-window dev** diagnostic, three measured seconds at synthetic
120 Hz: **360** measured submissions/callbacks, **360 zero `presentedTime`**,
**0** positive display samples, **0** no-drawable returns. Drawable acquisition
p50/p95 **4.563 / 6.787 ms**; worker-to-completion **9.203 / 11.569 ms**. Thus the
hidden layer does recycle drawables, but **does not display** while locked and
fails the 4.167 ms diagnostic cost budget. This is evidence of drawable
backpressure, not a physical playback pass or a prediction of foreground cost.

Validation: **15/15 tests per binary (30 total)** pass in the full default-feature
Cargo test run; **11/11** isolated policy/color/mailbox tests pass. Both the dev
viewer and optimized GPUI-free binary build. Formatting, shell/Python syntax,
and diff whitespace checks pass. Offscreen SDR/PQ-input comparison under
**Metal API validation** has no validation errors: SDR ΔE2000 p99/max
**0.030891 / 0.060174**, PQ **0.057133 / 0.121483**, each over **921,600** pixels.
Linear probe patches remain **0.5, 1, 2, 4, 8**. This validates queue ordering and
render output, not a CAMetalLayer/display color oracle. The existing `block 0.1.6`
future-compatibility warning remains.

Final review added unit-tested paused-seek retries and output-change repainting
without changing the measured successful-render path. No drawable loss or stale
rejection occurred in the final offscreen timing cases.

No engine-crate or vendor files were changed by S3b. Disk stayed above the
explicit 4 GiB stop threshold (lowest observed approximately **5.8 GiB** before
space was reclaimed externally). Only the shared target was used; no clean,
release publication, tag, push, private plan or screenshot commit occurred.

### Reproduction

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
# No GPUI or WindowServer needed for this binary.
python3 apps/native/spikes/s3/build.py build -p pm-native-s3-spike \
  --no-default-features --bin pm-native-s3-headless --release
python3 apps/native/spikes/s3/bench.py --release
# Hidden capability check: never activates or shows its window.
python3 apps/native/spikes/s3/build.py build -p pm-native-s3-spike \
  --bin pm-native-s3-spike
python3 apps/native/spikes/s3/bench.py --hidden
python3 apps/native/spikes/s3/build.py test -p pm-native-s3-spike \
  --no-default-features --bin pm-native-s3-headless
zsh apps/native/spikes/s3/test-units.sh

# Ready for a future foreground quiet window; does not unlock the Mac.
zsh apps/native/spikes/s3/foreground.sh
```

`bench.py` runs one S3 process at a time, waits up to three minutes for load below
12, samples load during each run, and records the condition separately. It uses
the shared target only and stops its own batch below 4 GiB. `--case=<name>` selects
one experiment; `--quick` shortens playback diagnostics to three seconds. Raw
reports/logs stay in `/tmp/s3b-*`; no local dumps or screenshots are tracked.
`foreground.sh` builds and selects the optimized viewer by default;
`S3_PROFILE=debug` selects a dev comparison run. The script includes ordinary
and repeated-position **AppKit** scrub tests, synthetic cached seeks, two 60-second
soaks and SDR/PQ-input accuracy.

Still required: physical input-to-display p95 (including the ≤60% Electron
comparison), zero physical scanout gaps at both rates, foreground headroom,
release viewer build/run, matched audio-clock behavior, physical display moves,
required machines/corpora/repetitions and review. The earlier outstanding HDR
output, overlay and composition items remain open.
