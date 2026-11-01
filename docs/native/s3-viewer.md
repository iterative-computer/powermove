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
