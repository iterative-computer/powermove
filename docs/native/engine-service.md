# pm-engine-service

P1 package 6 implementation, macOS, pinned Rust 1.96.1. This is an in-process
Rust service and a windowless export-child entrypoint. **The P1 performance and
viewer gates remain open.** Headless correctness results below do not establish
physical playback continuity, scrub latency, or speaker/display synchronization.

## API and ownership

`apps/native/crates/engine-service` is a native-workspace member through the
existing `crates/*` glob. It owns the viewer's audio link (`audio`), transport
clock (`clock`), media catalog (`media`) and QoS (`qos`). Fonts come from
`pm_text::host` (the main-thread `FontService` broker) and
`pm_render::text::host::RenderFonts`.

- Create `Gpu` and `EngineService::start` on a worker. Inject
  `service.metal_device()` into GPUI (P-DEVICE). `start_with_fonts` accepts an
  existing `pm_render::text::host::RenderFonts`; the main-thread host must keep
  servicing that broker. `fonts_changed()` invalidates paused viewers.
- On main, `layers::configure_layer(MainThreadMarker, layer, device)` configures
  the caller's retained CAMetalLayer: RGBA16Float, EDR, extended-linear Display
  P3, two drawables, acquisition timeout, no transaction presentation. Device and
  pixel format are checked when attaching. The shell owns below-slot placement,
  clipping, drawable size and CATransaction coordination; the service never
  creates a window.
- `viewer(ViewerConfig)` creates an independent main/source/popout viewer.
  Composition identity stays fixed; retire/recreate to switch it. Up to eight
  viewers share one renderer/device and a fair worker. Each has its own
  transport/audio controller. Popouts that should mirror transport must receive
  the same commands; independent source-monitor playback is also possible.
- `Viewer::publish(&Published)` consumes the kernel's COW snapshot. Older
  revisions and different document sessions are refused. Whole immutable tree
  adoption is deliberate: missed change events need no patch recovery path.
  Forward kernel publication events to this method; the service does not own a
  second document authority or an event subscription thread.
- `Viewer::media` installs relinked original/proxy mappings and rotates content
  identity even without a document edit. Normal viewer rendering uses originals.
- `configure(Output)` retires size/display/headroom generations.
  `layers::screen_output` queries display id, current headroom and potential
  headroom on main. Call it from screen/EDR-change notifications and publish
  changed output in the shell's placement transaction.
- `command(TransportCommand)` queues ordered, validated controls (64 maximum;
  contiguous seeks coalesce). `scrub(Ticks)` pauses then wakes immediate exact
  rendering, independently of display ticks. `shuttle(key, from_pause)` follows
  shipped J/K/L decline and saturation: ±1x → ±2x, direction change → ±1x,
  repeated ±2x consumed, stopped keys decline unless explicitly allowed.
- `display_tick(target_host_seconds, interval_seconds)` accepts CADisplayLink
  phase; the worker keeps that phase between callbacks and counts expired
  targets. Without phase the provisional worker schedule is 60 Hz. This is not
  a certified refresh-rate/adaptive-quality scheduler.
- `status()` returns latest clock, levels, frame identity, failures and counters.
  `capture_next()` requests diagnostic half-float readback; normal presentation
  performs no CPU pixel readback. Physical presentedTime and GPU completion are
  separate, optional observations. Missing presentation stays missing.
- Explicitly `retire()` viewers. Drop `EngineService` on a worker: it stops and
  joins rendering/audio and drains GPU work. Drop/join is not a UI event action.

With the default `kernel` feature, `Snapshot` is the real pm-kernel snapshot.
`--no-default-features` offers a small immutable snapshot struct for standalone
CLI embedding, without a kernel dependency. `Headless::{new,with_fonts,frame}`
uses exactly the same final Metal shader offscreen; `export_reference` independently
calls `pm_export::render_frame`. Font-aware headless rendering belongs on a worker
while its main-thread broker is pumped.

## Rendering and overlays

Working frames use `pm_render::Renderer::render_into` into retained
`pm_gpu::bridge_out::ExportedSurface` textures. The final pass decodes signed,
premultiplied engine E encoding, converts linear BT.709 to P3, premultiplies and
writes RGBA16F. It preserves negative and extended values; transient display
headroom is not used as a destructive highlight clamp.

One LRU holds up to eight exact working frames, with a nominal 128 MiB cache
capacity (at least one frame for larger outputs). Keys include content identity,
revision, exact engine ticks and dimensions. Separate viewers get separate
content identities; asset changes rotate them. The source pool is capped at 11
textures. There are at most three submitted consumers globally and one per viewer.
A completed command retains its source lease until its callback is released.
Resize/display/edit changes serialize against the final submission check; already
submitted Metal work drains while its results are rejected if stale. Retirement
cannot recall a drawable already submitted to WindowServer.

`OverlayGeometry` contains up to 64 normalized line segments with linear P3
colours. `safe_areas` is time-invariant. `overlays(stamp,time,geometry)` accepts
paused geometry only at the matching revision/time. `locked_overlays(provider)`
installs a **native Rust** geometry evaluator: it runs on the render worker with
the same document, composition and ticks as the picture, immediately before the
same final pass. It must not call GPUI/JS or wait for main. A viewer-tools layer
still needs to bind actual selected-layer transforms to this interface.

`PlaybackLayers` creates retained CATextLayer/CALayers for timecode, playhead and
stereo meters. The shell attaches/styles them above the slot and pumps `update`
from its native display callback using project-rate timecode, clock and meter
values. It does not invalidate GPUI. Physical layer/picture alignment remains
unverified in this package.

## Audio and exports

The service's `audio::AudioLink` (which owns `pm_audio::Playback`) and
`clock::TransportClock` provide lazy device warm-up, 30 ms project-update
throttle, asynchronous controls, output-latency timestamps, loop entry, epochs
and host-clock fallback. An audible start holds picture time until a new audio
epoch arrives. Silent compositions use the host clock; an empty document or an
unresolved composition also leaves nothing to hear but is reported as the
viewer's status error. Pause and seek cannot adopt the previous run's device
epoch. The inherited audio-start timeout is used; fallback is visible in status.

`jobs::ExportRequest::new` freezes the kernel document/revision, `JobSpec::Export`,
media catalog and destination. A worker calls `ExportHandle::start(executable,
request)`: that executable starts itself with `--engine-job`. Integrators must
handle that argument before GPUI startup by calling `jobs::run_child()`.
The crate's own binary demonstrates this dispatch. Private payloads use inherited
pipes, not command-line JSON or files. Status/progress uses the existing
`pm_protocol::priority::JobStatus`. Explicit cancel signals the child, and
`wait()` reaps it. Dropping a live handle kills/reaps it; use explicit cancel/wait
for graceful removal of temporary output.

The child renders on its own worker while main services the reused CoreText font
broker. It uses `pm-export` planning, encoding, verification and atomic no-clobber
publication. Only `settings.draft_from_proxies=true` can select proxies; otherwise
only catalogued originals resolve. Missing originals refuse export rather than
silently substitute proxy pixels. The caller must correctly classify media
provenance. Compat producers are explicitly refused until the P2 runtime host
exists. Caller project-font registration/replay and extension definitions are not
yet serialized to the child. The initial API used `Unrestricted`; slice 2 below replaces this with
per-band child-pipe admission.

## Decisions and picking rules

| Decision | Rule |
|---|---|
| Reuse AudioLink/TransportClock in a plain Rust dependency; keep bridge consumers intact | §3.3 lifecycle reuse, N5: delete only after the last production consumer and replacement gate |
| Adopt COW kernel snapshots, reject revision/session regressions | §3.2 single writer, N3 correct authored state |
| Reuse S3b same-queue texture leases and exact cache; add media/session identities | N1 instant scrubbing, N3 no approximate/stale cache hits, N8 no Node or pixel bridge |
| Keep the EDR layer separate from GPUI SDR; retain engine device objects | §3.3 and P-DEVICE; S3 viewer/export colour evidence |
| Preserve extended values, track headroom as output identity | N3 viewer=export; S3's highlight-clamp failure must not return |
| Evaluate locked geometry on the worker, draw it in the picture pass | §3.3 zero skew and N8; no main-thread per-frame gizmo dependency |
| Use protocol jobs in a same-executable child, explicit original-only resolver | engine-boundary §2.6/§5 and §1 export-safety contract |
| Windowless native tests; report simulated clocks separately | N7 and §5 evidence integrity: GPU completion is not scanout |

## Validation

Run from the repository root with `source engine/env.sh`,
`CARGO_TARGET_DIR=$HOME/Developer/pm-target`, and `cargo +1.96.1`:

```sh
cargo +1.96.1 test --manifest-path apps/native/Cargo.toml -p pm-engine-service -- --nocapture
cargo +1.96.1 test --manifest-path apps/native/Cargo.toml -p pm-engine-service --no-default-features -- --nocapture
cargo +1.96.1 clippy --manifest-path apps/native/Cargo.toml -p pm-engine-service --no-deps -- -D warnings
cargo +1.96.1 fmt --manifest-path apps/native/crates/engine-service/Cargo.toml --package pm-engine-service --check
```

The FFmpeg build's library directory is supplied as an rpath to the service/test
binaries. Tests use the unchanged public shape-primitives corpus fixture, copied
from the S3 preparation path, never a private project.

Final verification: **16/16 tests pass** with the default kernel adapter, and
**16/16 pass** with `--no-default-features`. Clippy with `-D warnings` and package
formatting pass. The existing `block 0.1.6` future-compatibility warning remains.

Measured headless correctness: **57,600 pixels**, ΔE2000 **p99 0.030891**, **max
0.054050**, compared to an independent export working-frame render with the f64
P3 reference. The simulated device-callback harness has **120 matched-time
samples**, mean absolute offset **0.000 ms**, p95 jitter **0.000 ms**, including a
160 ms simulated output latency. This is clock arithmetic/epoch evidence, not a
physical §5 A/V pass. Empty samples stay unmeasured; constant bias is not hidden.

The suite covers forced publication/resize during encoding (actual stale
submission rejected), real delayed Metal completion retaining its source,
cache hits and revision/size/session invalidation, J/K/L/seek/step/loop,
worker-evaluated moving overlays, rapid J/K/L and 100-input scrub bursts, proxy safety, 30-frame child export completion
and cancel after progress (no partial output or temporary directory retained).
All validation is headless. No on-screen batch, screenshot or GUI lock is used.

## Remaining gates and integration

Wire shell/kernel publication events, below-slot transactions, native display
callbacks, layer styling, source-monitor media selection and viewer-tool transform
bindings. Finish resource snapshots (definitions, project fonts, extension
activation and generation pinning) for windowed and job/headless hosts. P2's
windowless compat activation/virtual-time host is a separate dependency.

Complete end-to-end source-preserving PQ/HLG, mastering metadata/export parity, 8/16/32-bit preview
selection, automatic resolution policy, cache memory accounting and physical validation of export GPU
slicing. This package currently presents Float16 extended-linear P3, including
engine-decoded PQ/HLG content; slice 2 adds explicit encoded mastering transforms, without claiming the full
source-to-display HDR/metadata/export gate.

Run real audible playback and matched device/scanout A/V capture, AppKit scrub
latency, 60-second zero-drop/headroom soaks, popouts and physical mixed-display
moves. Take the shared GUI lock for every on-screen batch. S3/S3b measurements
are prior evidence, not measurements of this service. The required machines,
corpora, paired Electron comparisons, repetitions, orchestrator verification and
Opus review remain outstanding. No P1/colour/performance gate is declared passed.

## Slice 2 (2026-11-01)

- `EngineService::notifications()` is a bounded, coalesced status edge feed.
  Consumers read `Viewer::status()` after a wake. Submission, completion,
  command/state changes and failures wake the host; paused viewers generate no
  periodic notifications. `layers::DisplayLink` supplies view-bound native phase
  and pauses with transport. On macOS 13, where the NSView selector is absent,
  the status feed and provisional worker scheduler remain available; that OS
  still needs a CoreVideo phase adapter before its pacing gate can pass.
- `layers::OutputBinding` observes the shell layer's `drawableSize` synchronously.
  It calls the supplied `Viewer::configure` closure inside the shell's placement
  CATransaction, without replacing its composition registration. Screen/backing
  changes and `NSApplicationDidChangeScreenParametersNotification` also refresh
  output, including headroom-only changes. `sync()` is available to other hosts. Physical resize/zoom transaction
  alignment and mixed-display movement remain **UNVERIFIED**.
- `Viewer::mastering` selects extended-linear P3 (default), BT.2020 PQ or HLG.
  Encoded modes require explicit source/target peaks (203–10,000 nits), use
  203-nit reference white, a BT.2390 PQ-domain Hermite shoulder when compressing
  the source peak, and label the retained RGBA16F layer with the matching
  BT.2100 colour space. HLG reverses the target-display OOTF before its OETF.
  The default path continues to preserve signed extended values. This is an
  **output mastering transform**, not restoration of HDR source light already
  tone-mapped by the engine's current input policy. Upstream source-preserving
  HDR evaluation, metadata/export agreement and mastering controls remain
  engine/colour-lane work; engine sources were not changed here.
- `ResourceSnapshot` pins a generation with future extension/font requirements.
  Viewer changes retire content/cache identity; export serializes the snapshot.
  Unsupported nonempty requirements fail visibly until P2 activation exists.
- `ExportHandle::start_scheduled` takes the service's `Activity` (busy now, and
  an epoch that rises with every activity, so no pulse is missed). Nothing is
  granted while the viewer is busy. Within 1 s of its last activity a grant
  admits a submission bounded to 8 ms of measured GPU time (one band probes an
  unmeasured job; when bands cost about a whole frame, as precomps and
  multi-pass effects do, the rest of the frame); after 1 s, whole frames.
  Banding every frame made Paper's agent export 0.37 fps against 11.6 fps in
  whole frames. A submission already on the GPU cannot be preempted.
- `scrub_at` preserves NSEvent input origin. Frame reports expose optional
  worker-to-completion cost, input origin and the audio presentation host time
  matching that exact video media time. `PM_ENGINE_TRACE=/tmp/report.trace.json`
  enables a bounded nonblocking diagnostic queue and a private worker that
  streams NDJSON as frames complete. It remains readable after AppKit termination;
  overflow/disconnection is an explicit status failure. Missing/zero
  display timestamps never become GPU-derived presentation timestamps.

Minimal app wiring: `src/playback.rs` replaces the 16 ms polling loop with the
notification/display-link feed, attaches `OutputBinding`, forwards peak levels,
and enables transport looping only for opt-in trace runs. `src/session.rs` uses
NSEvent timestamps for seeks and passes export activity into the child scheduler
(these three small hunks are coordinated with the concurrent kernel lane).
No shell structure, design, kernel, vendor or engine implementation is owned by
this change.

Picking rules: N8/§3.3 choose notification-driven native layers; the §3.1
composition contract chooses synchronous placement publication; N3/§1 choose
explicit mastering with matching colour labels rather than a headroom clamp;
N4 chooses visible unsupported-resource failures; §5 interactive budgets choose
single-band export admission and truthful reporting of its preemption limit.

### On-screen (2026-11-01)

The session was locked. The owner requested finishing without waiting for an
unlock. All physical rows below remain **UNVERIFIED**; there is no pass claim.

| Real app with corpus project | Result | Verdict |
|---|---|---|
| Physical cached scrub input → present p95, 120 / 60 Hz | Not measured; budgets 9.333 / 17.667 ms, plus ≤60% of paired Electron | **UNVERIFIED** |
| 60 s playback scanout gaps, 120 / 60 Hz | Not measured | **UNVERIFIED** |
| Playback frame-time p95 headroom | Not measured; budgets 4.167 / 8.333 ms | **UNVERIFIED** |
| Matched-time A/V offset with real audio | Not measured; mean ≤1 ms, p95 jitter ≤2 ms | **UNVERIFIED** |
| EDR headroom / PQ / HLG physical output | Not measured | **UNVERIFIED** |
| Resize/zoom placement and display moves | Not measured | **UNVERIFIED** |
| Playback while exporting | Not measured | **UNVERIFIED** |

Ready-to-run after unlocking, on power, with load below 12 and the other GUI
lanes stopped:

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cargo +1.96.1 build --manifest-path apps/native/Cargo.toml -p pm-native-app
bun apps/native/crates/app/tools/prepare-corpus.ts /tmp/es2-corpus
python3 tools/perf-gate/foreground.py \
  /tmp/es2-corpus/projects/shapes/primitives/document.json --hz 120 --seconds 60
# Repeat at --hz 60; use the audio corpus document for the A/V row.
# For physical scrub, add --scrub and click/drag the real timeline strip.
# Do not count stdin seek commands as physical input.
```

The script uses the opt-in `PM_PERF_GATE` stdin seam, refuses a non-temporary
project, creates a fresh disposable profile, logs readiness, requires positive
presentedTime, and bounds its acquired-lock batch to under ten minutes. It
acquires `/tmp/pm-shots/gui.lock` exclusively as `engsvc2`, retries every 30 s,
and releases only its own lock on every exit. It never unlocks the Mac. Repeat
with a real audio clip and inspect matching non-null `audio_host`/`display`
values; no audio samples means no A/V result. Human scrubs must retain the
NSEvent-origin samples and separate exact cache hits. Screenshots, if needed,
belong only at `/tmp/pm-shots/es2-*.png`. Full §5 certification still requires
paired Electron measurements, the required machines/corpora and ≥30 runs.

Analyze a real-app trace after graceful exit with:

```sh
python3 apps/native/crates/engine-service/tools/report.py /tmp/es2-app-120.trace.json --hz 120
```

The report preserves absent values and counts missing/zero presentations. Its
scanout span must cover the full requested window; a short run or late start
cannot pass the 60-second gate. Read EDR potential/current headroom separately
from `layers::screen_output` on the foreground screen.

The headroom notification follows [Apple's NSScreen contract](https://developer.apple.com/documentation/AppKit/NSScreen/maximumExtendedDynamicRangeColorComponentValue?language=objc), which posts screen-parameter changes when current headroom changes.

### Headless/offscreen validation (2026-11-01)

Pinned Rust **1.96.1**, dev builds, one public shape fixture, **1280×720**.
These single diagnostics ran while other lanes built/tested (observed system
load approximately **10–18**); they are not the settled, paired, ≥30-run gate.
Service playback reuses exact working frames at the 30 fps project cadence
while accepting synthetic 120/60 Hz output ticks. Report samples are observed
completion-status updates; counts alone do not prove uninterrupted delivery.

| Service, 60 measured seconds | Synthetic 120 Hz | Synthetic 60 Hz |
|---|---:|---:|
| Completion samples | 7,065 | 3,574 |
| Worker → completion p50 / p95 (ms) | 0.798 / **3.445** | 1.519 / **4.147** |
| Render-cost bound (ms) | 4.167 | 8.333 |
| Local render-cost verdict | **Pass, offscreen boundary only** | **Pass, offscreen boundary only** |
| GPU failures / expired service targets | 0 / 0 | 0 / 0 |
| Physical presentation, scrub, A/V and EDR | **UNVERIFIED** | **UNVERIFIED** |

Default-kernel and standalone-adapter suites each pass **20/20** tests. This
includes independently computed f64 PQ/HLG signal comparisons against the
actual final Metal pass (maximum permitted signal error **0.002**), paused
notification quiescence, bounded wake coalescing, resource generation/cache
retirement, and a child export held at zero progress during interactive activity,
then resumed or cancelled while deferred. The prior numerical simulated-device
A/V test remains arithmetic evidence only. App/service all-target Clippy with
`--no-deps -- -D warnings`, package formatting and the integrated app build pass.
The existing `block 0.1.6` future-compatibility warning remains.

Reproduce service diagnostics without a GUI lock or WindowServer:

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cargo +1.96.1 build --manifest-path apps/native/Cargo.toml -p pm-engine-service --no-default-features
"$CARGO_TARGET_DIR/debug/pm-engine-service" --bench --hz=120 --seconds=60 --report=/tmp/es2-service-120.json
"$CARGO_TARGET_DIR/debug/pm-engine-service" --bench --hz=60 --seconds=60 --report=/tmp/es2-service-60.json
```

Still required: physical frame continuity/headroom under export, calibrated HDR
output and mastering metadata/export parity, macOS 13 CoreVideo phase, resource
activation rather than rejection stubs, actual mixed-display transaction checks,
and all paired §5 gates. No release, tag or push was produced.


### Unlocked real-app verification (2026-11-01)

**Gate P1 remains NOT PASSED.** These foreground observations supersede the
earlier locked-session table only for the listed cases. Pinned native dev binary,
real `powermove-native`, public shape composition, explicit disposable profiles,
physical built-in Retina display modes 120/60 Hz, AC power. The source audio
project was a disposable copy of the public corpus audio composition, with linked
asset descriptors resolving the six shipped tone/AAC-video files; no personal
media or installed profile was read.

App set A, present and untouched: Finder, Aside, Grok Bot, Notion, Mail,
Claude, T3 Code (Nightly), installed Powermove/helpers, RobloxPlayer,
RobloxMenuBar and figma_agent. Every result below retains this app-presence
caveat; load is the observed one-minute average before → after the case.
Existing Powermove process count changed from seven to five without this lane
closing them. No owned benchmark instance remains. All projects and profiles
were disposable public/synthetic fixtures under `/tmp`.

| Check | Observation | Verdict | Conditions |
|---|---|---|---|
| Real app, 60 s, 120 Hz | 4,787 positive / 3 missing-or-zero; span 59.974 s; 2,394 observed gaps | FAIL zero-gap observation; full census unverified | A; 4.38 → 4.22 |
| Worker → completion, 120 Hz | p50/p95 1.218/7.019 ms; bound 4.167 ms | FAIL local bound | A; 4.38 → 4.22 |
| Real app, 60 s, 60 Hz | 3,579 positive / 337 missing-or-zero; span 59.983 s; 5 observed gaps | FAIL zero-gap observation; full census unverified | A; 4.05 → 3.75 |
| Worker → completion, 60 Hz | p50/p95 1.592/7.559 ms; bound 8.333 ms | PASS local bound only | A; 4.05 → 3.75 |
| Real clip, 20 s loop at 120 Hz | 1,596 matched-device API samples; mean absolute offset **8996.517 ms**, p95 jitter **9047.986 ms** | FAIL reported offset/jitter bounds; epoch mapping requires investigation | A; 3.75 → 3.74 |
| Physical cached scrub | P1's paused native seek attempts did not yield usable presentation results; no physical service scrub distribution collected | UNVERIFIED | A; paired-run load 3.89–9.62 |
| EDR API/configuration | Built-in current headroom ~1.2 → 2.667, potential 16; RGBA16Float extended-linear P3 | PASS API observation only; physical PQ/HLG/colour unverified | A; foreground-case loads above |

The audio offset grows across the repeated two-second clip. The first-second
mean signed offset was **−14.052 ms**, the last-second mean **18,035.612 ms**.
This is an observed failure of the exposed matched-device timeline in the loop
case, not proof that audible output is nine seconds late. Loop/device epoch
mapping needs investigation before treating the API as a calibrated physical
A/V oracle. No paired Electron audio stamps or acoustic/photometric capture
exists; no A/V pass is claimed.

The 60-s host acknowledgement windows were **60.013 / 60.012 s**. The original
foreground script's Python 3.9 monotonic origin was process-relative, excluding
all host-clock trace rows. Reanalysis uses the recorded native play/pause
acknowledgements, not a guessed offset or fabricated timestamp. Positive trace
spans are shorter than 60 seconds, and missing presentations remain missing;
counts/scanout gaps do not establish complete content-frame delivery. The public
shape project is 30 fps, so it cannot certify P1's 60-fps workload either.

Future runs should use `python3 tools/perf-gate/foreground.py` with the documented
arguments. Its small clock adapter uses Quartz host seconds while preserving the
original temporary-profile, lock and timeout guards. The host clock is checked
against independent Mach absolute ticks in the **21/21 passing** harness suite.
Only harness code changed. Native trace NDJSON now parses without promoting
missing display timestamps to zero. Traces and window markers remain in `/tmp`.

Window-specific screenshots (outside Git): `onscreen-engine-120.png`,
`onscreen-engine-60.png` and `onscreen-engine-audio.png` under `/tmp/pm-shots`.
They show the actual app and were taken before their playback windows. Remaining:
paired/repeated physical gates, real service input scrub, resize/export-load
protection, mixed-display alignment and calibrated HDR output.

### Performance fix pass (2026-11-01)

See [the P1 fix-pass report](perf-gate-p1.md#fix-pass--2026-11-01) for conditions,
before/after observations and remaining failures. **P1 is not passed.**

The native viewer now uses three bounded drawables, forward-only display targets,
interactive worker QoS and completion/presentation wake tokens. The bounded token
queue preserves a wake that arrives before a wait; Metal callbacks never lock
service state. Work waits for its next deadline instead of polling every 1 ms
(except while awaiting the initial audio epoch). Existing source leases still
cover GPU consumption; revision, size and display retirement still reject stale
submission. Headroom is telemetry: the shader already preserves extended linear
light and only explicit mastering changes transform it. Draw reuse uses the
engine's existing invalidation rules.

`Transport::audio_host_for` maps wrapped frame time to the current unwrapped
loop pass before inverting the followed audio clock. The decoded PCM loop/seek
regression now meets mean ≤1 ms / p95 jitter ≤2 ms numerically, including 160 ms
simulated output latency. Physical video still carries presentation delay and
30-fps sampling error; numerical clock agreement is not physical A/V acceptance.

Opt-in traces include event/request, actor/publication spans, render/drawable
stages, GPU timestamps, commit, intended target and physical presentation.
Submitted stale frames are retained as diagnostics. Missing presentation remains
null. Edit origins wait for a new revision, so an old playback frame cannot take
the token. The native harness matches input, presented revision and new viewer
identity for scrub/edit/open respectively. See `tools/perf-gate/native.py` for
release-only, lock-owned foreground diagnostics and display restoration.

No engine, vendor, shell structure or dependency-lock changes were made.
The only kernel edit adds actor/publication timing in `actor.rs`.

### Latency pass (2026-11-01)

See [the P1 latency pass](perf-gate-p1.md#latency-pass--2026-11-01). **P1 is not passed**; the
session was locked, so there are no on-screen rows.

- **Held frames:** during playback a tick whose stamp and media time match the last
  submission is not re-presented (`ViewerStatus::held_frames`). Re-presenting a
  30 fps frame at 120 Hz kept all three drawables queued behind ~2.4 intervals of
  WindowServer latency and starved `nextDrawable`.
- **Lag-compensated picture time:** each viewer keeps the median of the last 31
  `presentedTime − requested` values (`present::PresentLag`; each sample and the
  estimate are clamped to 4 refreshes; prior 1.5 refreshes until 3 samples exist).
  The picture and its matched `audio_host` are chosen for `requested + lag`,
  rounded to the nearest onset for forward play.
- **Vsync-aligned start:** `Transport::grid` (last display tick/presentation) places
  the audio `Start::At` and host-clock run start on a vsync. `wants_display` asks the
  host to run its display link while awaiting the audio epoch.
- **Render-ahead:** while content changes at most every other refresh, playing frames
  are encoded two refreshes before their target (one otherwise). WindowServer missed
  the vsync when GPU completion landed within ~2 ms of the target.
- **Timed present (P1 fix pass):** playing frames use `presentDrawable:atTime:` at
  **target − ½ refresh**, a time that never depends on the lag estimate
  (`present::plan`). The compositor cannot latch the frame for a vsync before the
  target, and a request exactly on the predicted grid line no longer slips a refresh
  when the real vsync is a hair early. On screen at 120 Hz, 597 of 600 presents then
  landed exactly 1.5 refreshes after the request. The `atTime:` argument is when
  WindowServer may composite, not a glass time: glass follows about one refresh later.
  The earlier schedule requested `target + lag − ½` and sampled lag against the
  target, so each sample contained the previous estimate plus that refresh. With the
  upper-decile estimator it climbed one refresh at a time to 333–375 ms at 120 Hz
  and frames stopped reaching glass (see
  [P1 fix pass](perf-gate-p1.md#p1-fix-pass--2026-11-01)). Traces record `requested`.
- `PM_ENGINE_PRESENT=legacy` restores the previous schedule for same-build A/B only.

Layer configuration is unchanged (3 drawables, display sync, no transaction present):
a standalone probe found no latency difference across those settings.

## Disk render cache

- View › Render Selection queues the selected layer's span (or the work area)
  of the open composition. The frames render in an `--engine-job` child with
  the viewer's own scene (document, media paths of its media mode,
  definitions), its paused request at the composition's size (RGBA16F), the
  export's admission gate and exact override answers, and are written to the
  Storage › Renders folder captured at its start. A Background Tasks row shows
  frame progress; Cancel stops the child; a full disk or Render Files Limit
  fails the task by name.
- Frames are stored losslessly (`frame_codec`): 64-row stripes of per-channel
  left differences split into byte planes, LZFSE each; stripes code and
  decode on four threads. A 1080p frame of the 867-layer project is 3.9 MB
  instead of 16.6 MB; the reader loads one (file, checksum, decode) in about
  14 ms.
- Frames are keyed by a fingerprint of the whole composition: canonical
  document, contributed definitions, media content (sampled) and stamps
  (rechecked every 2 s), installed fonts, the viewer's output tone, a digest
  of the running binary, and with raster overrides the stamp of the
  extension code the wrappers come from (ids, file names, sizes, mtimes) and
  their generation in the realm, so a relaunch with unchanged extensions
  finds them.
  Every asynchronous result (fingerprint, fill progress, read) is fenced by the
  observation generation; storage changes and deletions raise it.
- Playing and scrubbing show frames a reader thread has loaded ahead of the
  playhead (never waiting on disk); the present pass samples them as the
  whole canvas, never upscaled. Paused frames always render.
- The timeline's render bar reads a snapshot published on every change. Keep
  Rendered pins frames against the limit and Clear…; Delete Rendered Frames
  stops a running fill first, then removes the composition's frames, kept too.
  No disk work runs under the cache's lock.
