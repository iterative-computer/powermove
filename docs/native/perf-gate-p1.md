# Phase 1 performance gate

Status, 2026-11-01 unlocked session: **INCOMPLETE / NOT PASSED**. Five
release-native / frozen-Electron pairs completed before the bounded retry reached
its 900-second deadline (builds included). A sixth partial pair is excluded.
These are real exploratory observations, not the required ≥30 accepted runs.
No performance or release gate is certified.

## Conditions

Apple M5 (Mac17,2), 16 GiB, macOS 27.2 / 26B5086k, AC power, unlocked.
Rust 1.96.1; native release build passed twice (7m48s and 8m28s).
The reference is unmodified v1.2.0 / `07ed5fa17ee640659cb075f9cad1af2095011d60`.
Only an extension-free public shape composition was timed: 640×360, 30 fps,
3 seconds, looped for CPU playback. No personal project/profile was opened.

Every completed retry pair recorded app presence and 1/5/15-minute load before
and after each side. Retry one-minute load ranged **3.89–9.62**;
the first attempt ranged 8.14–15.98, and compilation briefly exceeded 30.
Present and untouched throughout: Finder, Aside, Grok Bot, Notion, Mail, Claude,
T3 Code (Nightly), installed Powermove and its helpers; RobloxPlayer,
RobloxMenuBar and figma_agent were also present. Seven existing Powermove main
processes were initially detected (five at the end), including helper-mode hosts. No claim that all were
idle is made. These retained apps prevent a settled-machine acceptance claim.

The initial display check found Q27G41ZE, 2560×1440 at 240 Hz, EDR current /
potential 1.0 / 1.0. It was switched to 120 Hz. A later check found only the
built-in 3024×1964 Retina display at 120 Hz, potential EDR 16. The transition time
was not recorded; a continuously pinned display configuration is not proven.
Neither required hardware rig is certified. Later native checks physically pin
120/60 Hz on the built-in panel. See the dated S3/service sections.

`powermetrics` without sudo returned “powermetrics must be invoked as the
superuser”; no elevated retry. Disk started at about 22 GiB free, stayed above
8 GiB, and reference worktrees/node_modules and release artifacts were removed
at the end of each attempt. The external display was later reconnected and
its exact original mode 150 / 240 Hz restored and verified. Raw results, traces, profiles and screenshots stay
under `/tmp`, outside Git. The caller holds one inode-checked GUI lock for the
whole session; harnesses borrow it and cannot release it.

## Results

Values are **p50 / p95**, nearest rank; n=5 per side except as stated. Ratios
compare matching quantiles. At n=5, p95 is the maximum observation, not a stable
tail estimate. `—` means unmeasured, never zero. Readiness, open and repaint
endpoints differ or are incomplete; their ratios are diagnostic only. All rows
remain INCOMPLETE even where an individual observed clause is below its bound.

| Row | Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Gate verdict | Conditions / limits |
|---|---:|---:|---:|---|---|---|
| Cold launch (readiness diagnostic) (ms) | 967.402 / 1620.746 | 2010.985 / 4093.630 | 0.481 / 0.396 | p95 <400 ms; ratio ≤0.50 | INCOMPLETE | Controller/harness readiness, not first interactive frame |
| Open (ms) | 574.760 / 636.730 | 75.800 / 585.900 | 7.583 / 1.087 | ratio ≤0.50; main stall ≤1 frame | INCOMPLETE | Different native/Electron boundaries; main stalls absent |
| Cached scrub (ms) | — / — | 15.100 / 15.500 | — / — | p95 ≤9.333 ms; ratio ≤0.60 | INCOMPLETE | Native presentation unavailable in all five attempts; Electron GPU + rAF |
| 4K long-GOP random seek (ms) | — / — | — / — | — / — | preview ≤10.333 ms; full ≤150 ms; ratio ≤0.60 | INCOMPLETE | No public 4K long-GOP corpus fixture prepared |
| 60 s playback (physical drops/headroom) (ms) | — / — | — / — | — / — | zero drops at 60 fps; p95 work ≤8.333 ms | INCOMPLETE | Both CPU windows exceeded 60 s; 30 fps shape fixture and incomplete physical census |
| Playback CPU cost (%) | 14.719 / 14.719 | 54.088 / 54.088 | 0.272 / 0.272 | CPU ratio ≤0.60; GPU ≤0.80; energy ≤0.70 | INCOMPLETE | n=1 full 60 s interval/side; CPU clause only; no GPU/energy |
| Matched-device A/V (ms) | — / — | — / — | — / — | mean ≤1 ms; p95 jitter ≤2 ms | INCOMPLETE | No paired Electron device timestamps; separate native clip check below |
| Edit → viewer presentation (ms) | 47.231 / 47.231 | 16.400 / 18.800 | 2.880 / 2.512 | p95 ≤9.333 ms; ratio ≤0.60; main work ≤4 ms | INCOMPLETE | Native n=1, Electron n=5; four native timeouts; shell repaint/main work absent |
| Idle CPU (%) | 1.264 / 1.893 | 9.749 / 15.393 | 0.130 / 0.123 | CPU <1%; ratio ≤0.50; ≤2 wakes/s/extension | INCOMPLETE | Observed native absolute CPU target missed; 3 s settle + 2 s sample; no extension attribution |
| Small-project family footprint (MB) | 186.746 / 207.636 | 1063.132 / 1071.471 | 0.176 / 0.194 | native ≤Electron −150 MB; allocations counted once | INCOMPLETE | Summed phys_footprint; deduplication unverified; no extension load |

The single CPU playback intervals were **60.009 s native** and **60.009 s
Electron**; CPU uses one logical core as 100%. Native CPU ratio was **0.272**,
below the CPU-only 0.60 bound. Native idle p95 **1.893%** exceeds the absolute
1% target in this diagnostic. Family footprint p95 saved **863.835 MB**, but
shared CPU/GPU allocation identity is not established. Native base footprint
p50/p95 was **33.588 / 33.867 MB**, Electron **571.988 / 602.921 MB**;
document-delta checkpoints are retained privately. Metal allocation is separate
and is never added to family footprint. No extension split is claimed.

## Harness fixes and validation

The first pair reproduced cross-app fixture contamination: native edits saved
PMV data back to the shared temporary JSON, and Electron's JSON parser refused
it. Each app/pair now receives its own writable copy, preserving the prepared
fixture hash. Completed observations survive a later playback failure. Setup
seeks skip an already displayed native frame; the measured cached seek still
requires a changed frame and positive drawable presentation. NDJSON service
traces now parse without losing absent timestamps; malformed rows still fail.

Explicit diagnostic mode permits only retained-app/high-load collection while
preserving invalid conditions and separate acceptance/diagnostic summaries.
Unlocked-session, disk, power and active-build checks remain mandatory. Borrowed
whole-session locks require the expected owner and inode. **21/21 harness tests
pass**, including real Darwin CPU/family accounting, fixture mutation isolation,
partial-result preservation, NDJSON, lock ownership and fail-closed conditions.
No product code changed.

The first attempt stopped after launch/open measurement errors. The retry
retained five completed pairs, then timed out and reaped its owned applications;
no partial sixth pair is silently counted. Screenshots during timed intervals
were not used as performance evidence. No push, release, tag or workflow dispatch.

## Still needed

Repeat ≥30 per row on both required rigs with stable display/quality/thermal
conditions and the required corpora. Supply first-interactive-frame readiness,
matching file-open boundaries/main-thread stalls, physical cached/random seek,
4K long-GOP hydration, full physical frame census, matched Electron device A/V,
per-frame work, GPU/energy cost, shell repaint/main work and deduplicated memory.
The separate foreground S3/service observations do not fill these paired gaps.

## Fix pass — 2026-11-01

**Partial fix; Gate P1 remains NOT PASSED.** Measurements below use optimized
Rust 1.96.1 builds on the same M5/16 GiB machine and OS as the initial run.
The built-in panel was explicitly set to 120/60 Hz with CoreGraphics and its
original mode restored after each completed batch. AC power, existing apps
retained, disposable public-project copies and profiles only. Each batch owned
an inode-checked `perffix` GUI lock, had a nine-minute alarm and bounded teardown.
Raw conditions, profiles, traces and screenshots are outside Git. No required
M4 Max/base-M-series rig or settled-machine acceptance is claimed. No new paired
Electron runs were made; the original Electron numbers remain context only.

### Instrumentation and findings

The trace now connects input origin, kernel actor work, COW publication work,
service request, worker start, render submission, drawable acquisition, command
commit, GPU execution/completion and actual drawable presentation. Missing display
callbacks remain missing. Already-submitted stale frames are recorded before
status rejection, rather than silently disappearing from diagnostics.

An instrumented release baseline reproduced the real-app bottleneck: render
p95 **0.763 ms**, drawable acquisition p95 **0.514 ms**, worker → completion
p95 **2.450 ms**, but commit → display p95 **24.530 ms** at 120 Hz. Its 60-second
window contained **4,929 positive presentations and 2,266 gaps** above 1.5 refresh
intervals. Kernel actor work for tagged edits was sub-millisecond and publication
was tens of microseconds. Snapshot serialization or full graph preparation was
not the dominant delay for this small fixture. This real-app baseline is
separate from S3's earlier **4/1 gaps**: those spike observations are not an
interchangeable before measurement.

The two-frame limit remained occupied until presentation, even after GPU work
completed. At approximately three refresh intervals of presentation delay that
limit cannot sustain 120 Hz. The worker also rewound its next target on repeated
link ticks and polled completion at 1 ms. The fix uses three bounded drawables,
forward-only targets, a bounded wake token from completion/presentation callbacks,
deadline waits and interactive worker QoS. Callbacks never take the service state
lock. Existing revision, resize and display rejection remains in force.
Dynamic headroom alone no longer retires identical extended-linear pixels;
explicit mastering transforms, size and display changes still retire them.
The existing engine draw-reuse switch is enabled; no new render-graph cache or
engine/vendor changes were introduced. An unsynchronized-presentation experiment
was rejected and removed; display synchronization remains enabled.

A/V used wrapped media time as input to `TransportClock::host_at`, which expects
an unwrapped coordinate. Each loop therefore selected an earlier pass. The
service now lifts the frame onto the current audio epoch's line near its intended
presentation; seek/play/device discontinuities retain the presenter's epoch
fence. A decoded, audible PCM WAV test crosses repeated wraps and a seek with
160 ms simulated device latency. Before the fix it measured **350 ms mean /
450 ms p95 jitter**; after the fix both are **0.000 ms**. This is an arithmetic
regression test, not a physical A/V pass. The on-screen loop no longer accumulates
seconds of offset, but presentation delay and repeated 30-fps picture sampling
remain in the physical result below.

The native command seam now supplies an explicit monotonic seek/edit origin,
presented revision and viewer identity. A cached seek must match its own origin
and exact ticks; an edit must match the **presented** revision; open must match
the first positive presentation of the new viewer. A pending edit token cannot
be consumed by playback of an older revision. This removes stale AppKit-event
origins from automated seeks. Electron's endpoint remains GPU completion + rAF,
so matching logical boundaries do not establish matching physical endpoints.

### Before / after

| Row | First gate / release baseline | Final fix-pass observation | Result |
|---|---|---|---|
| Edit → presentation, 120 Hz (ms) | First gate 47.231 (n=1); release baseline 22.921 / 23.659 (n=3) | **23.429 / 30.545**, n=30 | FAIL ≤9.333 ms; no new paired ratio; main work unverified |
| Edit → presentation, 60 Hz (ms) | No matching first-gate row | **23.588 / 32.110**, n=30 | FAIL ≤17.667 ms |
| Cached scrub, 120 / 60 Hz (ms) | Missing native samples in paired gate | **22.958 / 24.249** and **23.250 / 24.506**, n=30 each | Measurement repaired; both absolute latency bounds FAIL |
| Open → first new-viewer presentation (ms) | First gate 574.760 / 636.730, different boundary | **415.883** at 120 Hz; **330.845** at 60 Hz, n=1 each | INCOMPLETE; corrected endpoint, no paired ratio or main-stall data |
| 60 s real-app scanout gaps, 120 Hz | Release baseline **2,266**, 4,929 positive presentations | **34**, 7,141 positive presentations | FAIL zero gaps |
| 60 s real-app scanout gaps, 60 Hz | Earlier service dev run **5**; S3 independently **1** | **3**, 3,598 positive presentations | FAIL zero gaps; no matched release-before row |
| Worker → completion p95, 120 / 60 Hz (ms) | Release baseline **2.450** at 120 Hz; earlier dev 7.019 / 7.559 | **1.511 / 1.725** | PASS local 4.167 / 8.333 ms bounds only |
| Device A/V mean absolute / p95 jitter (ms) | **8,996.517 / 9,047.986**, earlier 20 s loop | **34.634 / 12.558**, 2,397 matched samples in final 20 s loop | Epoch accumulation fixed; physical bounds still FAIL |
| Post-loop paused seek/restart | No comparable row | **27.329 ms** seek; positive subsequent playback timestamps | Functional observation only |

Final one-minute load before → after: **8.508 → 8.829** (120 Hz),
**8.829 → 6.317** (60 Hz), **6.317 → 5.978** (audio). Existing applications
were left running. Earlier retained fix iterations varied: 120-Hz edit p95
31.360–44.916 ms, gaps 45–107 and work p95 1.737–4.224 ms; 60-Hz work p95
4.562 ms; audio mean 30.032 ms. The table uses the last completed run of the
final source, not the fastest attempt. A display disconnect invalidated the
first exploratory batch's restoration; it is excluded. Subsequent listed
batches all restored the original display mode successfully.

In the final 120-Hz trace, tagged actor work p95 was **0.483 ms**, publication
**0.011 ms**, request → worker **0.035 ms**, while commit → display remained
**24.706 ms**. These spans do not include all GPUI/main-thread frame work.

Thirty command-driven cached seeks and edits were attempted at each refresh
rate. Open is one observation per side/rate; playback is one full 60-second
window per rate; audio is one 20-second loop followed by a paused seek and a
three-second restart. Quantiles use nearest rank. Positive timestamp gaps are
not a complete content-frame census; the public shape fixture is 30 fps, not
P1's 60-fps workload. Main-thread frame work, shell repaint, paired ratios and
random 4K long-GOP seek remain **UNVERIFIED**.

A separate real inspector interaction selected a layer and applied four numeric
opacity values in the native editor. The values and viewer changed on screen;
the trace reached GPU completion, but the edit drawables had no positive
presentation timestamps. No input → physical-presentation number is assigned to
those missing callbacks. Coordinate/AX input in this check is not a calibrated
HID latency distribution. This still needs a reliable foreground presentation
endpoint before the edit row can pass.

### Validation and decisions

- **248 Rust tests pass:** kernel 209, service 23, app 16. This includes the new
  decoded-audio loop/seek, old-revision input-token and headroom telemetry tests.
  The audio test drives a callback after seek to retire old-epoch ring slots
  before waiting for current-epoch PCM; otherwise the test itself can deadlock.
- **25 harness tests pass**, including rejection of another seek's origin, an
  unpresented edit revision and an old viewer after open. Clippy for app/service
  (`--no-deps --all-targets -- -D warnings`) passes. Release builds pass.
- **Every kernel change:** only `crates/kernel/src/actor.rs`; two diagnostic
  `Duration` fields on `Published`, timing around actor handling and publication,
  and a private timed-publication wrapper. No transaction, document, journal,
  persistence or history semantics changed. No Cargo.lock entries changed.
- **Picking rules:** COW authority and asynchronous hosts (§3.2) keep document
  work on the kernel; bounded presentation and stale rejection (§3.3) select the
  three-drawable/token design; authored timing/extended light (N3) selects epoch
  lifting and telemetry-only headroom; reuse/prune (N5) selects the existing
  planner switch; measured physical endpoints and all budget clauses (N2/§5)
  prohibit claiming a gate pass from GPU completion or corrected clock algebra.

Remaining implementation: reduce the measured compositor/presentation queue,
schedule picture onsets against the device audio line, explain every physical
scanout gap and missing foreground callback, and measure total main-thread work.
No engine change is justified by this small fixture's render timings; a broader
native presentation/layer investigation is needed before changing engine or
vendor code. S3's independent spike failure is still open. Paired Electron,
required machines/corpora and repetitions still gate acceptance.

Reproduce with a prepared disposable public shape document and linked public
audio project (build before the GUI batch):

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export DYLD_LIBRARY_PATH="$PM_FFMPEG_DIR/lib"
cargo +1.96.1 build --manifest-path apps/native/Cargo.toml --release -p pm-native-app
python3 tools/perf-gate/native.py --project /tmp/public-shape.json --output /tmp/fix-120.json --hz 120 --runs 30 --seconds 60
python3 tools/perf-gate/native.py --project /tmp/public-shape.json --output /tmp/fix-60.json --hz 60 --runs 30 --seconds 60
python3 tools/perf-gate/native.py --project /tmp/public-audio.pmv --output /tmp/fix-audio.json --hz 120 --runs 1 --seconds 20 --audio
python3 tools/perf-gate/native.py --project /tmp/public-shape.json --output /tmp/fix-inspector.json --hz 120 --runs 1 --seconds 120 --interact
```

The runner refuses locked sessions, active builds, insufficient disk and battery
power. It retains existing-app/high-load caveats as diagnostics, restores display
mode and reaps only its own process group. Release artifacts are removed after
measurement. No push, tag, release or workflow dispatch.

## Latency pass — 2026-11-01

**Gate P1 remains NOT PASSED.** This pass root-caused the presentation rows,
changed the engine service's playback schedule and the harness's A/V/drop
definitions, and measured on screen. The session was locked 09:00–09:45; after
unlock, two probe batches (120/60 Hz) and the 120 Hz app A/B ran under the
`latency` GUI lock (each < 10 min, display mode restored). The 60 Hz A/B ran
next (same binary) under the on-screen lane's lock, then the audio A/B. The
follow-up fixes (loop wrap, render-ahead, timed present) were not measured on
screen by this lane. An earlier probe run made behind
the lock screen is superseded by the on-screen repeat.

Conditions: Apple M5 / 16 GiB, macOS 27.2 (26B5086k), AC power, built-in
Retina panel only (3024×1964, ProMotion: minimum/maximum refresh interval
8.333/41.667 ms, EDR potential 16). Rust 1.96.1 release build. The release
build also contained other lanes' **uncommitted** GPUI-CE/shell edits
(P-120/P-TEXT); load 12–15 during the probe (another build), 2.8–5.5 later.

### Root causes

| Row | Root cause | Evidence |
|---|---|---|
| Edit → present, cached scrub (p95 30.5 / 24.2 ms vs ≤ 9.333) | **WindowServer composition latency, not Powermove.** An asynchronous `CAMetalLayer` present reaches glass ~20 ms (p50) / ~24 ms (p95) after commit at **both** 120 and 60 Hz, so the delay is time-based, not a frame count. Kernel, service and render work total ≈ 2–3 ms. | **On screen**, standalone Swift probe (`tools/perf-gate/present_probe.swift`: layer-hosting root, RGBA16F EDR `CAMetalLayer`, clear-only frame, n=39 each). 120 Hz commit → display p50/p95 (ms): default **20.5/24.4**, live 120 Hz `CADisplayLink` 19.8/24.4, `maximumDrawableCount` 2 19.8/23.8, `displaySyncEnabled = NO` 19.8/23.7, `presentsWithTransaction` + main present 20.2/24.1. 60 Hz: 22.7/45.4 (one tail), live link 20.8/24.5, 2 drawables 21.6/24.9, sync off 19.8/24.1, transaction 20.7/24.7. **No layer setting moves the floor.** In continuous mode, a present for the link's target displays exactly 3 refreshes later at 120 Hz (24.99 ms p50 and p95); 3 drawables add one refresh of commit-side queueing over 2 (32.4 vs 24.1 ms commit → display). |
| 60 s playback gaps (34 at 120 Hz) | **Drawable-pool starvation by re-presenting held frames.** The service presented every display tick, and a 30 fps frame was presented 4× at 120 Hz. A drawable stays busy acquire → commit → ~2.4 intervals → on glass → released after the *next* scanout, ≈ 3.5–4 intervals. Three drawables (the CAMetalLayer maximum) cannot sustain one present per vsync, so `nextDrawable` blocks and ticks are missed. This is also why 2 drawables showed 2,266 gaps. | Pool arithmetic from the measured lag. Headless: the new schedule submits exactly **1,800** frames per 60 s (30 fps content) and holds **5,290** ticks at 120 Hz / **1,796** at 60 Hz. |
| A/V mean 34.6 / jitter p95 12.6 ms | **(1) Metric:** every scanout of a held frame was compared with its frame-start audio time, giving offsets 0/8.3/16.7/25 ms + lag. That is exactly ±12.5 ms of "jitter" (reported 12.558). **(2) Mapping:** the picture was chosen for the *target* vsync, but it reaches glass one lag (~22 ms) later. **(3) Phase:** audio started at `now + 50 ms`, not on the display's vsync line, so onsets cannot be closer than the phase difference (≤ ±4.2 ms at 120 Hz). | Same arithmetic reproduced in the new harness test (`av_all_presentations` jitter 12.5 ms, onset jitter 0). |
| Main-thread GPU waits (edit row's ≤ 4 ms clause) | **GPUI-CE vendor:** `composition::present` always returns `Some(Transaction)`, so `MacWindow::draw` sets `presents_with_transaction(true)` on every frame and never clears it. Every GPUI frame then runs `commit(); wait_until_scheduled(); present()` on the main thread inside the composition `CATransaction`. | Code inspection (`gpui_macos/src/window.rs` `draw`, `composition.rs` `present`); not timed this pass. |

`displaySyncEnabled`, `present(afterMinimumDuration:)` and EDR settings are not
factors for these rows. A transaction-coupled viewer present is unnecessary
except while slot geometry is moving.

### Fixes (engine service, harness, two app hunks)

- **Hold, don't re-present.** While playing, a tick whose frame (stamp + media
  time) equals the last submitted one is skipped. The layer keeps showing it.
  `ViewerStatus::held_frames` counts these. A submission that never reached
  glass clears the hold, so the next tick re-presents it.
- **Present-lag compensation.** The worker keeps the median of the last 15
  `presentedTime − target` values per viewer. It chooses the picture for
  `target + lag`, and for forward play rounds to the frame whose onset is
  nearest that vsync (half-interval lead, not past the end). `audio_host` is
  matched at the same instant. Traces add `predicted`.
- **Vsync-aligned audio start.** `Transport::grid` holds the latest display tick
  or `presentedTime`. Play places its sample-accurate `Start::At` (and the
  host-clock run start) on the first vsync of that grid ≥ `now + 50 ms`. The
  first sample and every refresh-multiple onset then share the display line.
- **App** (`crates/app/src/playback.rs`, one line): the view display link runs
  while `ViewerStatus::wants_display` (playing *or* awaiting the audio epoch).
  Previously it ran only once the clock had a rate, so no phase was available
  for the start. `crates/app/src/perf_gate.rs` reports `held_frames`.
- **Harness:** A/V is measured at **content-frame onsets** (first presentation
  of each media frame). All-presentation numbers stay as
  `av_all_presentations`. Playback continuity reports **skipped and late
  content frames** (`continuity.dropped`; late = onset interval > content
  interval + ¾ refresh). Raw `presentation_gaps` are retained but are no longer
  a drop count with held frames. `native.py --lock-owner` and
  `PM_ENGINE_PRESENT=legacy` (A/B of the previous schedule in the same build)
  were added.
- **Follow-ups after the on-screen run (tested headlessly only):** the loop-wrap
  lead fix, and adaptive render-ahead (2 refreshes while content changes at
  most every other refresh, else 1).
- **Not changed:** drawable count stays 3, display sync stays on and
  `presentsWithTransaction` stays off on the viewer. Measurements showed no
  latency benefit, and transaction coupling would add main-thread waits.

### Before / after (on screen, 120 Hz, same release build, `PM_ENGINE_PRESENT=legacy` vs default)

Public shape fixture (640×360, 30 fps, 3 s loop), 30 cached seeks + 30 edits,
then 60 s looped playback. Load (1-min) 8.2 → 5.9 legacy, 5.7 → 3.9 default.
Existing apps retained, as in earlier passes. Quantiles are nearest rank.

| Row (120 Hz) | Fix pass (earlier build) | Legacy schedule, this build | New schedule | Verdict |
|---|---|---|---|---|
| Edit → presentation p50 / p95 (ms) | 23.429 / 30.545 | 23.528 / 24.407 | **22.958 / 24.533** | FAIL ≤ 9.333; at the compositor floor |
| Cached scrub p50 / p95 (ms) | 22.958 / 24.249 | 22.918 / 24.333 | **22.712 / 24.489** | FAIL ≤ 9.333; at the floor |
| Presents in 60 s (30 fps content) | 7,141 | 7,175 | **1,800** (+5,486 held ticks) | Drawable/compositor load ÷ 4 |
| Commit → display p50 / p95 (ms) | 24.706 p95 | 24.503 / 24.687 | **19.783 / 21.172** | One refresh of drawable queueing removed |
| Target → display p50 / p95 (ms) | — | 16.5 / 24.5 (2–3 refreshes, bimodal) | **13.739 / 14.024** (1.65 refreshes, stable) | Predictable lag for A/V |
| Raw scanout gaps > 1.5 refresh | 34 | 19 | 1,799 (expected: held frames) | Not a drop measure any more |
| Dropped content frames (skipped + late onsets) | — | 12 (0 + 12) | **36 (0 + 36)** | FAIL zero; see below |
| Worker → completion p95, new frames only (ms) | 1.511 (all presents) | 2.624 | 4.061 | PASS 4.167 local bound, barely |
| A/V | 34.634 / 12.558 (all presentations) | see audio table | see audio table | mean PASS, jitter FAIL |

The 36 dropped onsets break down as follows.
- **19 are a bug in this change, at the loop wrap (every 3.00 s).** The
  half-refresh lead wrapped to the loop start, and a `lead ≥ plain` guard fell
  back to the unrounded time, so the first frame of each pass landed one
  refresh late. **Fixed after the run (the wrapped lead is used); not
  re-measured.**
- **17 are WindowServer missing a vsync** (display − target 1.65 → 2.65
  refreshes). In each one, GPU completion landed ≈ 2 ms before the target
  (median −4.35 ms for on-time frames).
- **Follow-up, not re-measured:** while content changes at most every other
  refresh, frames are now encoded **two** refreshes ahead. This applies to
  30 fps at 120/60 Hz but not at content rate = refresh rate, where it would
  hold a third drawable. It gives completion a full extra refresh of slack.
- Worker p95 rose because every submission is now a new frame with a full
  render, and fewer submissions let the GPU clock down. Legacy's per-present
  average was diluted by 3 cache hits per frame.

**60 Hz, same binary and protocol** (run by the on-screen lane under its GUI lock,
load 10.7 → 7.0 legacy, 7.0 → 6.8 default):

| Row (60 Hz) | Fix pass | Legacy schedule | New schedule | Verdict |
|---|---|---|---|---|
| Edit → presentation p50 / p95 (ms) | 23.588 / 32.110 | 23.364 / 24.579 | **23.497 / 24.624** | FAIL ≤ 17.667; at the floor |
| Cached scrub p50 / p95 (ms) | 23.250 / 24.506 | 23.106 / 24.396 | **22.856 / 24.529** | FAIL ≤ 17.667; at the floor |
| Presents in 60 s | 3,598 | 3,600 | **1,800** (+1,830 held) | — |
| Dropped content frames | 3 raw gaps | **0** | 5 (all loop wraps after 47 s; bug fixed afterwards) | Re-run needed |
| Worker → completion p95 (ms) | 1.725 | 2.964 | **3.209** | PASS 8.333 local bound |

The audio A/B was first refused (active builds) and then run by the on-screen lane;
see below.

**Audio A/B, 120 Hz, 20 s, n≈600 onsets** (on-screen lane, binary `c1dbf3bc9`, i.e.
before the loop-wrap fix; load 6.2 → 5.3 legacy, 5.3 → 4.8 default):

| Row | Fix pass | Legacy schedule | New schedule | Verdict |
|---|---|---|---|---|
| Onset A/V mean signed / abs (ms) | 34.634 abs (all presentations) | 20.680 / 20.680 | **−0.063 / 0.588** | PASS ≤ 1 ms mean (diagnostic, n=1 run) |
| Onset jitter p50 / p95 about mean (ms) | 12.558 (all presentations) | 0.279 / 0.315 | 0.021 / **8.291** | FAIL ≤ 2 ms |
| All-presentation mean / jitter p95 (ms) | 34.634 / 12.558 | 33.058 / 12.681 | — | Old metric artefact |
| Late / skipped onsets | — | 11 / 0 of 601 | 30 / 0 of 600 | FAIL zero |

Legacy's onsets were consistently 20.7 ms late (one target-to-glass lag) with low
jitter. The new schedule removed the bias. Its 36 outliers above 2 ms are one
refresh each:
- **10 at media time 0** are the loop-wrap bug, fixed in `751f09522`.
- **~20 are early (−8.35 ms):** the compositor latched the commit one refresh
  sooner than usual (display − target 0.99 instead of 1.98 refreshes).
- **~6 are compositor misses (+8 to +17 ms).**

The early latch is fixed afterwards by presenting `atTime:` the predicted glass
time minus half a refresh, never earlier than the target. A frame can then no
longer appear before the instant its picture was chosen for. Lag is still
sampled against the link target. Limitation: a lag estimate that ratchets up
(≥ 8 of 15 late samples) does not correct downward. **Not re-measured on
screen.** In the on-screen lane's sampler, 60 Hz playback CPU was ~12.5%
(legacy) vs ~9.7% (new), one core = 100%.

**Re-run after the loop-wrap fix + two-ahead encoding** (release `ca2950b3a`, on-screen
lane, load 7.6–7.8 from other lanes' builds, so not a quiet machine):

| Row (120 Hz) | New schedule at `c1dbf3bc9` | At `ca2950b3a` | Verdict |
|---|---|---|---|
| Onset A/V mean signed / abs (ms) | −0.063 / 0.588 | **0.646 / 0.728** | PASS ≤ 1 ms (n=1 run) |
| Onset jitter p50 / p95 (ms) | 0.021 / 8.291 | 0.688 / **3.453** | FAIL ≤ 2 ms |
| Late onsets, audio clip (of 600) | 30 | 20 | FAIL zero |
| Late / skipped onsets, 60 s shape (of 1,800) | 36 / 0 | **18 / 0** | FAIL zero |
| Cached scrub p50 / p95 (ms) | 22.712 / 24.489 | 23.263 / 24.357 | FAIL; floor |
| Edit p50 / p95 (ms) | 22.958 / 24.533 | 23.175 / 30.385 | FAIL; tail under load 7.8 |
| Worker → completion p95 (ms) | 4.061 | 4.392 (audio 4.729) | Above the 4.167 local bound under this load |
| Process CPU, sampled window incl. open/edit, 1 core = 100% | 14.3% (audio) | 16.7% audio / 12.1% shape | Legacy audio 27.1% (window incl. open) |

The loop-wrap outliers are gone. Encoding two refreshes ahead made presents
normally land **1** refresh after target, with ≈10% at 2. That gives 1,454 / 333 /
13 presents at 1 / 2 / 3 refreshes in the shape run, and 575 / 22 / 2 in the
audio run. The median lag estimate therefore sat on the fast slot, and every
slow present was a refresh late: 63 of 600 audio onsets were > 4 ms late, 1 was
early. **Follow-up (headless-tested only):** the lag estimate is now the
**upper decile** of the last 15 samples. Combined with the timed present
(`73b91610c`), every frame is held to the slower slot, the samples then all
read that slot, and onsets stop alternating. This adds one refresh of
picture latency during playback only; A/V is unaffected. A residual of
presents slower than that slot (~0.7% at 3 refreshes here) remains.
**Neither `73b91610c` nor this change was measured on screen:** the session
locked again at ~10:17. A release binary of `73b91610c` is in
`~/Developer/pm-target/release`; rebuild for the decile change.

Headless service (dev, offscreen) after the change: exactly **1,800** submissions
per 60 s at 120 and 60 Hz, **5,290 / 1,796** held ticks, worker → completion p95
**1.806 / 2.657 ms** (earlier 3.445 / 4.147 ms), 0 GPU failures.

### What still fails or is open

- **Edit/scrub ≤ vsync + 1 against `presentedTime` is unattainable for a
  windowed `CAMetalLayer` on this OS/machine.** The on-screen probe floor
  (p95 ≈ 24 ms at 120 and 60 Hz) is ~2.6× the 120 Hz budget before any app
  work, and the app now sits on that floor (p95 24.4–24.5 ms). Electron's
  15–19 ms reference ends at GPU completion + rAF, not glass, and is subject to
  the same compositor. The §5 row needs a defined endpoint (e.g. commit, or
  the floor subtracted) and a matched-endpoint Electron run. This needs an
  orchestrator/council decision, not more service work.
- **Zero dropped frames** is not shown. The loop-wrap fix and two-ahead encoding
  should remove the 19 wrap drops and most of the 17 compositor misses. Both
  need an on-screen re-run (≥ 8 GiB free, rebuild).
- **A/V ≤ 1 / ≤ 2 ms** depends on (a) a stable lag estimate (lag variance of one
  refresh appears directly as onset jitter), (b) the grid being current at Play
  (the start uses the last known vsync; ProMotion may re-phase after idle),
  and (c) content rates that divide the refresh rate. 24/25 fps at 60/120 Hz
  keeps a cadence error of up to half a refresh.
- **GPUI-CE patch for the P-120 lane (described, not applied):** in
  `MacWindow::draw`, enable `presents_with_transaction` only when the
  composition `present` callback reports that slot geometry changed this
  frame (live resize/animation). Restore `false` after `draw`. Otherwise use
  `present_drawable` + `commit` with no `wait_until_scheduled` on main. The
  composition contract (§3.1.2) only needs the transaction when slots move.
- Main-thread work per frame, paired Electron runs, required rigs/corpora and
  ≥30-run gates remain outstanding.

### Reproduce (unlocked, AC, quiet machine)

```sh
source engine/env.sh; export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cargo +1.96.1 build --manifest-path apps/native/Cargo.toml --release -p pm-native-app
swiftc -O tools/perf-gate/present_probe.swift -o /tmp/pm-present-probe
# Fixtures: public shapes/primitives document + media/audio with absolute tone/AAC paths under /tmp.
bun apps/native/crates/app/tools/prepare-corpus.ts /tmp/pm-lat-corpus
# 1) Compositor floor, per refresh (pin with tools/perf-gate/display.swift, restore after):
set -o noclobber; echo latency > /tmp/pm-shots/gui.lock
for c in "count=3 sync=1" "count=3 sync=1 link=1" "count=2 sync=1" "count=3 sync=0" "count=3 sync=1 txn=1"; do
  /tmp/pm-present-probe mode=ondemand samples=40 $=c; done
for c in "at=0" "at=1"; do /tmp/pm-present-probe mode=continuous seconds=8 $=c; done
rm /tmp/pm-shots/gui.lock
# 2) App A/B, same build (each command owns the lock as `latency`, < 10 min):
for mode in legacy default; do for hz in 120 60; do
  PM_ENGINE_PRESENT=$mode python3 tools/perf-gate/native.py --lock-owner latency \
    --project /tmp/pm-lat-fixtures/public-shape.json --output /tmp/pm-lat/$mode-$hz.json --hz $hz --runs 30 --seconds 60
done
  PM_ENGINE_PRESENT=$mode python3 tools/perf-gate/native.py --lock-owner latency --audio \
    --project /tmp/pm-lat-fixtures/public-audio.json --output /tmp/pm-lat/$mode-audio.json --hz 120 --runs 1 --seconds 20
done
```

Read `playback.continuity`, `playback.av_*` (onsets) and the runs' scrub/edit
`ms`. Headless service evidence: `pm-engine-service --bench --hz=120|60
--seconds=60` (dev build), reporting `held_frames`.

Validation: service **25/25** (kernel adapter) and **25/25**
(`--no-default-features`), including new tests for held frames and grid
alignment. App **16/16**. Harness **27/27**, including onset A/V and content
continuity. App/service Clippy `--all-targets -D warnings` and package `fmt`
pass. No kernel, engine, vendor, shell or Cargo.lock change.

## On-screen rerun 2 — 2026-11-01 (10:33–11:00)

**Gate P1 remains NOT PASSED. The default (new) playback schedule at
`0863a9732` regresses at 120 Hz: the present-lag estimate runs away and
playback stops reaching glass after ≈10 s.** This was the first on-screen run
of `73b91610c` (timed present) + `53f8f7027` (upper-decile lag). Both were
previously tested only headlessly.

Conditions: release build of `origin/feat/native-engine` @ `0863a9732` in a
clean detached worktree. There were no uncommitted edits, unlike the latency
pass. Rust 1.96.1, built in 5 min 22 s. Apple M5 / 16 GiB, macOS 27.2
(26B5086k), AC, built-in panel only, unlocked, caffeinate held. The shared
build lock (`/tmp/pm-build.lock`) was held for the whole session, so no other
lane compiled. Each batch held the GUI lock as `onscreen2` and restored the
display mode (`display_restore_ok` true for every run). The machine was **not
quiet**. One-minute load was 18–19 during the audio runs and 13 → 4 later,
from `fileproviderd` (~100% CPU) and an already-running Electron Powermove.
Existing apps were retained, so every run is a diagnostic, not gate-eligible.
Disk: 6.7 GiB free at the start. The oldest 7 GiB of stale
`pm-target/debug/deps` artifacts (hash groups untouched for > 3 h) were
evicted to keep ≥ 8 GiB free for the harness.

Commands: the "Latency pass → Reproduce" block, with `--lock-owner onscreen2`
and outputs under `/tmp/pm-onscreen2/lat/` (not tracked).

**Audio A/B, 120 Hz, 20 s** (`public-audio.json`, `--audio --runs 1`):

| Row | Legacy schedule | Default schedule | Verdict |
|---|---|---|---|
| Content onsets reached glass (of ≈600) | 601 | **131** | Default FAIL: 275 skipped |
| Onset A/V mean signed / abs (ms) | 21.608 / 21.608 | 19.371 / 19.371 | FAIL ≤ 1 ms (both) |
| Onset jitter p50 / p95 about mean (ms) | 2.276 / 6.045 | 11.053 / 13.944 | FAIL ≤ 2 ms (both) |
| Late / skipped onsets | 27 / 0 | 18 / 275 (293 dropped) | FAIL zero |
| All-presentation mean / jitter p95 (ms) | 33.878 / 14.651 | 19.371 / 13.944 | — |
| Worker → completion p50 / p95 (ms) | 0.687 / 2.091 | **60.851 / 113.343** | Default FAIL |
| Commit → display p50 / p95 (ms) | 24.601 / 24.710 | **174.164 / 249.146** | Default FAIL |
| Load (1-min) before → after | 18.7 → 18.8 | 17.7 → 13.1 | Not quiet |

**60 s shape run** (`public-shape.json`, 30 cached seeks + 30 edits, then 60 s
looped playback; nearest-rank quantiles):

| Row | Legacy 120 Hz (control) | Default 120 Hz | Default 60 Hz | Verdict |
|---|---|---|---|---|
| Cached scrub p50 / p95 (ms) | 23.034 / 24.367 | 22.659 / 24.383 | 22.711 / 24.471 | FAIL ≤ vsync+1; compositor floor (unchanged) |
| Edit → presentation p50 / p95 (ms) | 22.957 / 24.457 | 22.932 / 24.530 | 23.063 / 28.408 | FAIL; floor (60 Hz tail) |
| Content onsets reached glass (of 1,800) | 1,801 | **132** | 1,800 | Default 120 Hz FAIL |
| Late / skipped onsets (dropped) | 39 / 0 (39) | 18 / 272 (**290**) | **1 / 0 (1)** | FAIL zero; 60 Hz nearly clean |
| Held ticks | 0 (7,139 presents) | 63 | 1,829 | — |
| Worker → completion p50 / p95 (ms) | 0.748 / 2.329 | **60.012 / 113.291** | 2.023 / 4.340 | 120 Hz FAIL; 60 Hz PASS 8.333 local bound |
| Commit → display p50 / p95 (ms) | 24.604 / 24.730 | **174.047 / 249.143** | **73.926 / 82.463** | Default lag inflated at both rates |
| Load before → after | 5.0 → 4.1 | 13.1 → 6.9 | 7.4 → 5.4 | — |

Scrub/edit are measured before playback, so they are unaffected and remain on
the ≈24 ms compositor floor.

### Regression: present-lag runaway (from the trace)

`predicted − target` (the lag the worker compensates for) grows without bound
while playing at 120 Hz:

- Audio run: 0 → 54 → 83 → 108 → 133 → 167 → 192 → 217 → 242 → 275 → 300 →
  **333 ms** at t = 0, 0.4, 0.8, 1.3, 1.9, 2.6, 3.4, 4.3, 5.4, 6.6, 7.9, 9.3 s.
  It climbs one refresh (8.3 ms) at a time, and `display − target` tracks it
  at +8 ms. From **t = 9.97 s**, submissions never get a `presentedTime` (77 of
  260 rows have none). Worker → GPU-done grows with it (3 → 101 ms).
- Shape run, 120 Hz: the lag reaches **375 ms** by ≈17 s and stays there. From
  9.6 s, **397 of 560** submissions are never displayed.
- 60 Hz: the lag creeps 28 → 36 → 42 → 46 → **54 ms** over 60 s (still
  rising at the end). Frames keep reaching glass, but picture latency is
  ~3–5× the 60 Hz legacy value.

Likely mechanism (not verified in code by this lane): `73b91610c` presents
`atTime:` predicted glass − ½ refresh. The measured `presentedTime − target`
therefore includes the requested delay. `53f8f7027` takes the **upper decile**
of those samples, so each late slot raises the next prediction by a refresh
and nothing pulls it back down. The latency pass already noted that a
ratcheting estimate "does not correct downward". Lag must be sampled against
the requested present time (or `presentedTime − scheduled-at`), not the link
target. Alternatively the estimate needs a ceiling (e.g. 3 refreshes) and a
downward path. The legacy schedule in the same binary has no runaway
(commit → display p95 24.73 ms, 0 skipped).

### Status after this rerun

- Do not ship the default schedule at `0863a9732`. The legacy schedule is
  stable but keeps the ~21 ms A/V bias and 27–39 late onsets per run.
- Re-run the audio A/B and both shape runs after a lag-sampling fix, on a quiet
  machine (`fileproviderd` idle, no Electron instance).

## P1 fix pass — 2026-11-01

**Gate P1 remains NOT PASSED** (edit/scrub stay on the compositor floor;
late onsets are not zero). This pass fixes the default schedule's present-lag
runaway found by on-screen rerun 2 (`feat/na-onscreen2`, at `0863a9732`): at
120 Hz the predicted lag climbed one refresh at a time to 333–375 ms and after
~10 s submissions stopped reaching glass; at 60 Hz it crept 28 → 54 ms.

### Root cause

`73b91610c` requested `presentDrawable:atTime:` at `target + lag − ½ refresh`.
`53f8f7027` then took the upper decile of `presentedTime − target`. The
`atTime:` argument is when WindowServer may composite, not a glass time, and
glass follows about one refresh later. So every sample contained the previous
estimate plus that refresh, and the estimate fed back on itself: one refresh
per window at 120 Hz. At 60 Hz the extra delay is under half a refresh, so only
jitter pushed samples into the upper decile, hence the slower creep. Once the
lag passed the drawable pool and the worker's 250 ms flight window, every
drawable sat queued for a far-future present (worker → completion p95
113 ms) and `presentedTime` was never observed. The median would also have run
away; the decile only made it faster. A headless simulation of the old rule
(below) reproduces both rates.

### Fix (`engine-service/src/present.rs`)

- **The request never depends on the estimate.** Playing frames request
  `target − ½ refresh`, moved to a later refresh only if that is already due.
  A frame still cannot be latched for a vsync before its target.
- **Lag = `presentedTime − requested`** (traces add `requested`). It is a
  sliding median of 31 samples. Each sample and the estimate are clamped to 4
  refreshes, with a prior of 1.5 refreshes until 3 samples exist. Old samples
  age out, so a burst of slow presents cannot ratchet it. The picture and
  `audio_host` are chosen for `requested + lag`.
- **Rejected variant (measured, round 2):** anchoring the request to the last
  `presentedTime` phase (`LEAD` 12.5 ms before a real vsync). At 120 Hz it was
  within noise (15 vs 17 late). At 60 Hz this ProMotion panel shows a frame a
  near-constant ~12.5–13 ms after its request instead of on a fixed grid. The
  glass phase then fed back into the next request, random-walked, and caused
  10 late onsets (vs 0). The phase is not an independent input on this panel.
- Legacy (`PM_ENGINE_PRESENT=legacy`) is unchanged, for same-build A/B.

Headless tests (service lib, no GPU): a simulated WindowServer with a
compositing delay, jitter, a grid phase offset and 3% missed vsyncs runs 60 s
of 30 fps content at 120 and 60 Hz, with `presentedTime` arriving a refresh
after glass. The estimate must settle within one refresh, stay ≤ 4 refreshes,
not drift, lose no frame past 250 ms, and land ≥ 95% of frames exactly as
predicted (≥ 50% and within 2 refreshes for a compositor that itself splits
between slots). The same simulator running the old rule must exceed 250 ms at
120 Hz and 50 ms at 60 Hz; it does. Also: a burst of slow samples does not
ratchet and ages out, and the request is identical with any estimate.

### On screen (120/60 Hz, release, default vs legacy in the same binary)

Conditions: Apple M5 / 16 GiB, macOS 27.2, AC, built-in panel only, unlocked
(caffeinate). One-minute load 3.8–7.6 from other lanes; existing apps were
retained, so these are diagnostics, not gate-eligible. Each run held the GUI
lock as `p1fix`, and the batch held `/tmp/pm-build.lock`. `display_restore_ok`
was true for every run. Commands are those of "Latency pass → Reproduce"
(`--lock-owner p1fix`, outputs under `/tmp/pm-p1fix/lat/`, not tracked). Final
binary: round 3. Round 1 used the same request rule without the "already due"
guard, which never triggers for 30 fps content.

| Row | Rerun 2 default (`0863a9732`) | **Default, round 3** | Default, round 1 | Legacy, round 3 | Legacy, round 1 |
|---|---|---|---|---|---|
| Audio 120 Hz 20 s: onsets shown | 131 / ≈600 | **600 / 600** | 600 / 600 | 602 | 601 |
| Audio: skipped / late onsets | 275 / 18 | **0 / 3** | 0 / 3 | 0 / 5 | 0 / 2 |
| Audio: A/V mean signed / abs (ms) | 19.371 / 19.371 | **0.007 / 0.076** | 0.015 / 0.095 | 23.217 / 23.217 | 20.577 / 20.577 |
| Audio: jitter p50 / p95 about mean (ms) | 11.053 / 13.944 | **0.042 / 0.078** | 0.055 / 0.092 | 0.221 / 0.258 | 0.028 / 0.064 |
| Audio: worker → completion p95 (ms) | 113.343 | **2.763** | 2.666 | 2.529 | 2.483 |
| Shape 120 Hz 60 s: onsets shown (of 1,800) | 132 | **1,800** | 1,800 | 1,801 | 1,801 |
| Shape 120: skipped / late | 272 / 18 | **0 / 9** | 0 / 17 | 0 / 24 | 0 / 21 |
| Shape 120: worker → completion p50 / p95 (ms) | 60.0 / 113.3 | 3.653 / 5.058 | 2.331 / 5.472 | 0.757 / 2.196 | 0.768 / 3.426 |
| Shape 120: commit → display p50 / p95 (ms) | 174.0 / 249.1 | 21.063 / 21.657 | 21.982 / 22.437 | 24.576 / 24.694 | 24.597 / 24.695 |
| Shape 60 Hz: onsets shown / skipped / late | 1,800 / 0 / 1 (lag creeping) | **1,800 / 0 / 4** | 1,800 / 0 / 0 | 1,801 / 0 / 4 | 1,801 / 0 / 2 |
| Shape 60: worker → completion p95 (ms) | 4.340 | 3.476 | 3.812 | 4.875 | 4.857 |
| Cached scrub p50 / p95, 120 Hz (ms) | 22.659 / 24.383 | 22.771 / 24.271 | 22.852 / 24.563 | 22.967 / 24.518 | 23.047 / 24.388 |
| Edit → presentation p50 / p95, 120 Hz (ms) | 22.932 / 24.530 | 23.090 / 24.408 | 22.886 / 24.182 | 23.112 / 24.603 | 22.913 / 24.463 |

Round 2 (rejected phase-anchored variant): audio 600 / 0 / 1 late, A/V
−0.022 / 0.078, jitter p95 0.063 ms. Shape 120: 1 skipped, 15 late (legacy 1
skipped, 49 late). Shape 60: 10 late (legacy 2).

From the traces:
- The estimate stays put. Audio: 1.47–1.50 refreshes for the whole run.
  120 Hz shape: 1.15–1.23, drifting slowly with the link/glass phase. 60 Hz:
  0.47–0.50, then 0.80–0.81 after the panel re-phased mid-run; only 4 frames
  landed off-prediction.
- 597/600 (audio) and 1,790/1,800 (shape) presents land exactly where predicted.
- **Every remaining late onset is a WindowServer miss.** GPU completion and
  commit were 13–16 ms before target at 120 Hz (28–30 ms at 60 Hz), and the
  frame still appeared one refresh late. No present schedule removes these.
  Legacy, which re-presents every vsync, has as many or more.
- Worker → completion p95 at 120 Hz (5.1 ms) is above the 4.167 ms local
  bound. As noted in the latency pass, every submission is now a full new
  frame and there are 4× fewer submissions, so the GPU clocks down. Legacy's
  per-present average is diluted by cache hits.

Against the targets: **all onsets shown ✔, zero skipped ✔ (round 3), A/V mean
≤ 1 ms ✔, jitter p95 ≤ 2 ms ✔, zero late ✘** (3 / 9 / 4 compositor misses).
The default schedule beats legacy on every playback row except raw worker
time, so it stays the default.

Validation: service **30/30** with and without default features (12 lib
incl. 5 new schedule tests, 18 integration), app **16/16**, compat-host
**22/22** plus the offscreen probe (7/7). Clippy `--all-targets --no-deps -D
warnings` passes for service, app and compat-host (all features); `fmt`
passes. No kernel, engine, vendor, shell or Cargo.lock change. To keep ≥ 8 GiB
free, 4 GiB of `pm-target/debug/deps` hash groups untouched for > 6 h were
evicted.

## Budget endpoint decision (2026-11-01)

The edit → display and cached-scrub rows are now judged as two checks (owner decision, see
p0-decisions.md): Powermove's own share (input → frame ready, ending at GPU completion) at
p95 ≤ 9.333 ms, and input → on screen faster than Electron when both are measured to
`presentedTime` on the same machine. The ~20–24 ms macOS composition floor measured in the
"Latency pass" counts against neither app. The harness must report both numbers per row.

## Cloud gate tooling — 2026-11-01

**Code and Linux fixtures only; no Mac runs. Gate P1 remains NOT PASSED.**
`tools/perf-gate/run-gate.sh p1` now schedules thirty independent native/Electron
pairs for each of the ten existing rows, alternates AB/BA order, retains all
attempts, and writes a Markdown report table plus raw JSON outside Git. The
configured Electron bundle/executable receives its own temporary profile; each
side receives an independent writable copy of the same hashed public workload.
The former diagnostic entry points remain for their existing consumers.

Quiet checks cover normalized 1/5/15-minute load, NSProcessInfo thermal state,
GUI-app inventory, active builds, AC, session lock, free disk and unchanged
display/OS conditions before, during and after both sides. An already-running
copy of the configured app is foreign; only the measured executable in the
runner's owned process group is exempt. Invalid-condition pairs are retained
and replaced within a bounded attempt limit. Far Tukey outliers are flagged,
retained in every quantile and never used as a reason for replacement. Missing
reference observations cannot form a pair with another run's native result.

The approved endpoint split is enforced: edit/cached own-share p95 ≤9.333 ms,
and native on-screen p50/p95 strictly faster than paired Electron
`presentedTime`. Native GPU completion remains reportable when physical
presentation is missing; Electron GPU fence/rAF remains explicitly nonphysical.
No composition-floor estimate is subtracted. Reported side distributions remain
visible independently of the smaller paired intersection used for comparisons.

Decisions and selecting rules:

- **N1/N2, owner endpoint decision:** all clauses, ≥30 pairs per row, full
  60-second playback windows, nearest-rank tails, and both latency numbers;
  no timing-based trimming or promotion of incomplete diagnostics to a pass.
- **N2/N3:** use a conservative load ceiling of 0.5 per logical CPU at all
  three horizons, nominal thermal only, fixed conditions and whole-pair rejection.
  No additional public outlier prescription exists; the far 3-IQR rule only
  annotates observations. The explicit 9.333 ms own-share limit is retained
  conservatively at 60 Hz as well as 120 Hz.
- **N3:** validate actual random-seek media with ffprobe before timing, requiring
  ≥3840×2160 inter-frame video and a measured keyframe interval ≥1 second;
  use identical seeded frame targets. No still/shape or intra-only substitute.
- **N4/N6:** retain raw evidence and private profiles/copies, never change the
  source document or an installed profile, and keep the unmodified frozen
  reference. No app or user data is deleted by the new runner.
- **N7/N8:** one inspectable CLI and bounded JSON adapters; Node is used only
  by external Electron-reference tooling, with no new native-app dependency.
- **Prune rule:** keep `run.sh`, `driver.py` and `native.py`; their diagnostic
  consumers remain. No product visual/interaction changes or design decisions.

See `tools/perf-gate/README.md` for manifest preparation, command usage, the
versioned adapter contract, retained evidence, and Linux replay. The recording
is synthetic protocol data, not a new measurement of either app. Replay always
exits 2 and explicitly cannot certify a gate. Python 3.9+ paths canonicalize the
system temporary root for macOS's root-owned `/tmp` link.

Linux verification: **61 tests collected, 58 pass, 3 existing Darwin-only tests
skip**. Node syntax and shell syntax checks pass. No Rust crate or dependency
changed, so no Cargo lockfile update or Rust compilation is required by this
tooling change.

The bundled adapter automates the existing app seams. It still cannot certify
first interactive launch, matching physical Electron/open endpoints, verified
Electron cache hits, random-seek preview, complete playback/frame-work census,
GPU/energy attribution, paired device A/V, shell/main-thread repaint, extension
wakeups, or deduplicated memory. These require the Mac instrumentation adapters
described in the owned README; unavailable counters remain missing. Before any
Mac timing, compile/check the Swift condition probe, verify foreground app/process
group accounting and reference launches, pin/match display and quality, then
exercise the real endpoint adapters on both required rigs. A 60 Hz build-Mac
stand-in stays labelled and cannot satisfy the base-machine gate. No crate,
engine, vendor, spike, or native-app source changed in this lane.

