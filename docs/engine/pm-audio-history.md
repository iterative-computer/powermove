# pm-audio: history of the A4–A8 campaigns

Chronological logs of the audio lane's review rounds (A4 validation, A6, the A7/A8
rounds 1–8 and the R7 scheduling work), moved out of `pm-audio.md` in review round 9
so that document describes the current state. Entries are kept as written: later
rounds supersede earlier ones where they disagree, and measurements here were taken
on the revisions named in each entry, not on the current one. The current status,
decisions and remaining work are in `pm-audio.md` ("Current status").

## Round 8: A7/A8 campaign, today's model build (2026-10-22)

Campaign run 20:00–20:05 PDT on the 19:33 build (before commits cb0b9065b, 113398716, cc2beab57, 82c3bc8be, 8056dca5b and 0c13380ae, so it does not qualify the round 8 head).

`tools/playback-gates.ts`, paired hidden apps (today, then the engine, one row
counterbalanced), 16 speed-1.5 voices, 30 edit-then-play samples per mode, MacBook
Pro speakers at 48 kHz, one exclusive Electron reservation; raw JSON outside the
repository. Latency from the edit and Play to the first scheduled presentation, ms:

| Row | Today p50 / p95 / max | Engine p50 / p95 / max |
|---|---|---|
| A7 60 s loop, then gain edit | 30.61 / 32.42 / 32.73 | 19.49 / 20.29 / 20.33 |
| speed edit | 31.13 / 32.61 / 32.63 | 26.85 / 27.75 / 29.58 |
| speed edit, 16 distinct trims (engine first) | 29.63 / 32.33 / 32.73 | 29.22 / 30.12 / 30.90 |
| cold same-task seek (2 s / 3 s) | 29.92 / 32.31 / 32.52 | 25.19 / 26.38 / 26.60 |
| 4 load threads: A7 60 s loop, then gain edit | 31.24 / 32.43 / 32.44 | 19.57 / 20.30 / 20.31 |
| 4 load threads: speed edit, distinct trims | 30.32 / 32.62 / 32.76 | **32.20 / 33.79 / 33.85** |
| 4 load threads: cold seek | 29.82 / 32.21 / 32.51 | 28.01 / 28.92 / 29.22 |

Where the loaded distinct-trim time goes (the same campaign's StartTiming, p50 /
max, ms): request to applied 0.25 / 2.06, applied to ready (sixteen independent
vocoder pre-rolls) 10.94 / 12.28 (8.49 / 9.59 unloaded), ready to callback 0.99 /
1.49, callback to device time 2.33, then the device's 15.6 ms output latency. Today
needs no preparation (it ignores speed) and reports 24 ms output latency. The
preparation workers run under the callback-matched Mach policy (a 64-frame, 1.33 ms
period, 95 % computation) and a checkpoint sleeps out the period once a worker has
used its computation less a reserve of up to 500 µs, so each worker does about
0.77 ms of DSP per 1.33 ms; under load that duty cycle, not the DSP, sets the
pre-roll's wall time. Not yet fixed: the candidates are a longer Mach period for
workers preparing a start that is not yet audible (a multiple of the callback, the
same reserve), or fewer pre-roll samples per distinct voice; both need a quiet
qualification window, because the policy also guards the playing producer.

The cold-start bound's measurement (AUDIO-2) was reviewed: the request is stamped
when the commands are sent, publication is stamped after the ring's head is
released (an upper bound on visibility), and the measured instant is the earlier
of that stamp and the observation. The observer used to spin on `yield_now`,
taking a core from the workers it timed; it now sleeps 100 µs between polls (the
interval cannot lengthen the measured value).

A7: both 60 s loops (plain and loaded) had 0 underruns, 0 starved renders and 0
overloads in the engine; today 0 underrun events. Every engine start had 0 program
underruns, 0 omitted prefix frames and 0 start spread. The loaded distinct-trim row
is 1.0–1.9 ms slower than today (16 independent vocoder pre-rolls on 4 cores under
load): an A8 miss, not qualified. The headless cold-start bound (AUDIO-2,
10.67 ms) still fails intermittently at machine load 19–41 (12.3 ms measured).


## A4 validation (2026-10-18)

Round 1 (historical; round 2 below supersedes it). The round 1 A4 implementation passed **30/30 plain and 30/30 stressed playback
suites**, 25 tests each: 1,500 executions, zero failures, none ignored. The
six mapped-clip comparisons in each suite total 360 comparisons of 80,000
frames in both channels (57,600,000 sample values): zero differences, source
starvation, drops, or ring underruns in those comparisons. The callback-paced
A4 probes assert zero ring underruns while a worker remains blocked, exact
ready-voice samples, bit-exact transient recovery, and one notification per
voice episode. The overlapping-episode probe checks all 24,000 audible B
samples while A remains stuck, then checks that A reports its next episode.

Validation uses the pinned Rust 1.96.1 toolchain, `kache`, `nice -n 15`, four
build jobs, and four test threads. The stress runs set
`PM_AUDIO_PLAYBACK_STRESS=1` (two shared CPU contention threads and 8 ms cold
decode delay). Reproduce the repetition commands above with the additional
local environment setting `CARGO_PROFILE_RELEASE_LTO=off`. Thin-LTO test
builds in this environment failed with `failed to get bitcode from object
file for LTO (Can't find section __bitcode)`, including a retry with explicit
embedded bitcode. (The cause was pm-media bundling CrabbyAvif's Rust
staticlib, whose objects carry no bitcode, into its rlib; not kache or this
machine. Fixed since: [release-build.md](release-build.md).) The tests passed
with optimized release code and LTO off;
the repository's release profile was not changed. These counts therefore do
not claim a successful thin-LTO test link. No parity tolerance or corpus case
was changed.

Final targeted checks also pass: 42 library tests (including forward/reverse
short-drop vocoder continuity), 9 decode-readiness tests, 14 mixer tests,
and 6 stretch tests. `cargo check --manifest-path engine/Cargo.toml --release
-p pm-audio --lib` passes with the normal release configuration and test hooks
disabled. Modified Rust files pass `rustfmt --check`.

### A4 review round 2 (2026-10-18)

The independent review found two blocking defects in round 1:

1. The per-voice stall message changed wording, and pm-node's
   `present::audio::tests::recurring_render_errors_are_listed_once`, which
   round 1 never ran, failed (it matches `exceeded 120 ms`). The 120 ms
   message is the A3 wording again (per voice); a reserve-ended wait has its
   own stable message. The pm-node test passes unchanged, and nothing in
   pm-node changed.
2. A cold play or seek dropped its missing voices about one device callback
   after starting, because real callbacks advance `played_frames` through
   underruns and an empty ring had no wait budget. The A3 start hold is back
   (see the policy above), and an all-missing mix mid-run keeps it too.

Non-blocking items addressed: whole-mix failures report per run episode
instead of once per playback; the stale doc comment on
`a_long_decode_stall_is_reported_and_recovers` is rewritten; the clippy
`type_complexity` warning is gone with `has_ready_source`; large device
callbacks keep a wait budget (`fill_target`); the long-gap catch-up cost is
remeasured (table above).

`a_cold_start_holds_by_device_time` and
`a_cold_start_past_the_bound_drops_only_the_missing_voice` are new and fail
on round 1's code (all-cold play from a stop: `dropped_renders` is 1, not 0,
one 64-frame callback before the bound). Their first version was flaky in
the seek cases (6 of 25 runs): with the ring full of the old epoch's chunks
the run renders nothing until a callback retires them, which the probe did
not do. The probe now calls back until the run waits and bounds the hold
start accordingly; 40 repetitions (20 plain, 20 stressed) then passed.

Measured at 7586687b (release, `CARGO_PROFILE_RELEASE_LTO=off`, `nice -n 15`,
four build jobs, four test threads, other lanes building on the same Mac):

- **30/30 plain and 30/30 stressed playback suites**
  (`PM_AUDIO_PLAYBACK_STRESS=1`), 27 tests each: 1,620 executions, zero
  failures, none ignored. 360 mapped-clip comparisons of 80,000 frames:
  zero differing samples, zero source starvation, zero ring underruns.
  Suite times 20.0 to 23.7 s (median 20.2 s).
- Full `cargo test --release -p pm-audio`: 44 library (1 ignored: the
  catch-up measurement), 9 decode-readiness, 2 layouts, 1 legacy-parity,
  14 mixer, 27 playback, 3 resampler, 5 sources, 6 stretch.
- `cargo test --release -p pm-node --lib present::audio`: 1 passed.
- `cargo clippy --release --all-targets` is clean for pm-audio and pm-node.
  `cargo fmt --check` reports only `pm-audio/src/graph.rs`, which this lane
  does not touch (it arrived unformatted from the integration branch).

Remaining: the mixed-readiness start needs the orchestrator's confirmation
(see the A3 review follow-ups); long-gap stretch continuity (over 120 ms of
dropped output) restarts by deterministic pre-roll; 4,096-frame and larger
callbacks leave mid-run misses beside a sounding voice no reserve-bounded
wait; thin-LTO test links still failed as in round 1 (fixed since:
release-build.md).

### A4 review round 3 (2026-10-18)

The independent review found one blocking defect: A4 point 1 failed during a
retune glide. Round 2 decided whether the reserve protects anyone from
`report.voices > report.starved`, but `voices` counts only the current mix
while `starved` adds every outgoing glide's copy of the missing voice. With
voice A ready, voice B missing and one glide in flight, that is 2 against 2,
so B took the full 120 ms hold, the 80 ms ring ran dry and A went silent.
Every live edit and monitor solo change while playing starts a glide, and a
waiting render never advances one, so a fader drag during a decode miss hit
this. The decision now uses ids: some voice in `RenderReport::heard_whole`
(sounded whole, on a heard strip, current or outgoing mix) is not in
`starved_voices`. `dropped_voice_frames` had the same inflation and now counts
each voice once.

New tests:

- `a_decode_miss_during_a_live_edit_keeps_ready_voices_heard` (playback,
  callback-paced): one live edit per 256-frame callback, a gain drag on the
  late voice and, in a second case, a monitor solo toggled between every strip
  and both strips (glides that do not change what is heard), from before the
  miss until after re-entry. It asserts the ring never runs below a callback,
  zero underruns, the drop before 120 ms with the output-buffer message, the
  ready voice bit exact, one contiguous gap, and `dropped_voice_frames` equal
  to the silenced samples. With round 2's `voices > starved` rule it fails:
  the ring runs out at turn 32 with `dropped_renders: 0` and 45 retries
  (`/tmp/ne-w5/audiopolicy-r3/live-edit-on-round2-protects.txt`, outside the
  repo). Before the per-voice count fix it also failed on 14,336 dropped
  frames for 5,120 silenced.
- `heard_whole_voices_count_once_across_glides_and_follow_the_solo`
  (decode_readiness): four glides in flight give `voices` 2 and `starved` 5,
  with `heard_whole` `["good"]` and `starved_voices` `["late"]`; a dropped
  render counts 256 frames once; a soloed-out voice is not heard.
- `stretch::tests::committed_drops_continue_within_the_catch_up_limit`:
  300 ms and 500 ms committed drops (forward and reverse maps) re-enter with
  one vocoder start and equal an uninterrupted run; 1 s and 1.28 s drops (the
  latter ending on a `GRID` boundary) equal a fresh run from the re-entry.
  With the old 120 ms window the 300 ms case fails; without the commit-drop
  reset the 1.28 s case fails.
- `a_stretched_voice_re_enters_a_300_ms_drop_bit_exact` (decode_readiness,
  through the mixer, speed 1.5): a playing stretched voice loses its next
  source block for 300 ms of output, then re-enters equal to a mixer that
  never dropped, with the other stretched voice equal throughout. It fails
  with the old 120 ms window (first differing render at output 53,248).

Non-blocking items addressed: the stretch continuity window is now the
work-based `catch_up_limit` (output samples, documented as render work, not
device time), replacing the hard-coded 120 ms; a soloed-out voice no longer
shortens a soloed voice's wait. The three orchestrator decisions are listed
under the A3 review follow-ups.

Measured at f466b8ed (release, `CARGO_PROFILE_RELEASE_LTO=off`, `nice -n 15`,
four build jobs, four test threads, other lanes building on the same Mac):

- **30/30 plain and 30/30 stressed playback suites**
  (`PM_AUDIO_PLAYBACK_STRESS=1`), 28 tests each: 1,680 executions, zero
  failures, none ignored. 360 mapped-clip comparisons of 80,000 frames:
  zero differing samples, zero source starvation, zero ring underruns.
  Suite times 20.3 to 21.6 s (median 20.5 s). The new live-edit test passed
  in all 60.
- Full `cargo test --release -p pm-audio` (before the mixer-level re-entry
  test): 45 library (1 ignored: the catch-up measurement), 10
  decode-readiness, 2 layouts, 1 legacy-parity, 14 mixer, 28 playback, 3
  resampler, 5 sources, 6 stretch. At f466b8ed the library, decode-readiness
  (11), mixer and stretch suites pass again; playback is the repetition
  above.
- `cargo test --release -p pm-node --lib present::audio`: 1 passed, pm-node
  unchanged.
- `cargo clippy --release -p pm-audio --all-targets` is clean. `cargo fmt
  --check` reports only `pm-audio/src/graph.rs`, inherited from the
  integration branch.
- Catch-up remeasured (same table shape as above): 0.12 s 1.1 / 4.0 ms,
  0.5 s 3.8 / 3.7 ms, 1 s 7.0 / 4.4 ms, 10 s 58.4 / 2.4 ms, 60 s 345.9 /
  3.6 ms (catch-up / fresh start; reads unchanged).

Remaining: the three orchestrator decisions above; stretch continuity past
`catch_up_limit` restarts by deterministic pre-roll; thin-LTO test links
still failed (fixed since: release-build.md).


## A6: uninterrupted clock, adaptive ring, paused preparation (2026-10-18)

A6 implements the engine policies for the three orchestrator decisions
previously listed as pending. The caller integration follow-up below also
delivers ordinary paused app seeks before Play.
The 120 ms cold-start no-loss hold stays. After a run has queued output,
every missing voice drops at the earlier of its own 120 ms bound and the
remaining ring budget. This applies when all voices are missing too: the
mixer commits intentional silence with the correct media tags, so the device
clock and the picture keep moving. Each voice retains its own episode,
recovery point and one diagnostic per episode; exports still wait for input.

The start hold now lasts until the first **device output chunk** is queued,
including time spent priming the 2× shuttle and device-rate converter. A6's
first repetition exposed a −2× cold/warm mismatch: ending the hold after the
first project input segment allowed a second source miss during shuttle
priming to drop with an empty ring. Moving the transition to actual output
fixes it; the existing bit-exact cold/warm comparison now also asserts zero
committed drops. No sample comparison or tolerance changed.

The callback only records its frame count. The render producer grows a
linked SPSC ring by splicing stable, empty nodes at its unfilled sentinel;
queued/partially consumed nodes never move and remain alive until both
sides close. The callback does not allocate, lock or wait. The ring retains
the largest observed callback size, so a first observation or buffer-size
increase adapts on the next fill. The target is at least the configured
80 ms, and otherwise three callbacks plus 40 ms: two callbacks of refill
reserve, the callback already spent waking the producer, and a per-voice
wait. At 48 kHz it is 3,840 frames for 512-frame callbacks, 14,080 for 4,096,
and 26,368 for 8,192 (whole 256-frame chunks). Growth preserves old-epoch
skips, exact partial-frame counts and sample order, including concurrent
producer/consumer traversal. It does not shrink while a viewer lives.

While paused, the render loop queues bounded look-ahead for every voice at
the playhead and work-area boundaries, including the stretch analysis
pre-roll. At most eight new time-map groups are bounded per window per turn.
Up to three starts keep separate, freshly pre-rolled vocoder histories;
preparing the work start cannot change the playhead's phase history. A new
project/evaluator invalidates these histories; a seek retains only relevant
starts. Prepared PCM never reaches the device or meters while paused. A
normal play installs the matching prepared history and replays its initial
chunk exactly as an offline render from that target.

The minimal presenter hooks are in `pm-node/src/present/audio.rs`: feed
project/media revisions to warmed mixers before the first Play, remember
paused seeks before device open, and handle a delivered paused scrub with
Scrub audio off as an exact, silent seek. They use the existing transport
and status interfaces; no protocol or app UI changed.

**Paused caller integration.**
The small follow-up in `pm-node/src/present/render.rs` forwards
paused exact seeks, forwards paused scrubs regardless of the audio-scrub
preference (the audio control handler decides whether to play a grain), and
seeds a newly created audio link with the current playhead. Host-clock test
views still avoid opening audio. A focused hidden-app regression verifies the
prepared sample positions after exact seeks, scrub-off moves, and an exact
seek before audible media/device open. Scrub-off emits no grain levels;
scrub-on emits a tone grain and returns to the requested stopped position.
The existing broader waveform/scrub test retains all its assertions.

A6 point 4 accepts deterministic pre-roll after long stretch gaps. Valid
phase history continues up to `catch_up_limit` (27,904 output samples,
0.5813 s at 48 kHz); longer committed gaps restart at the deterministic grid
and match a fresh render from re-entry. Recovering the old phase beyond that
requires analysing every missed sample: the earlier measurements above show
351 ms for a 60 s catch-up versus 3.5 ms for a fresh start. The long gap is
already audible, and the bounded restart protects the other voices.

New playback regressions cover an all-missing mid-run mix with zero ring
underruns and a continuously correct clock; 4,096-/8,192-frame callbacks
with transient recovery inside each voice's budget and prolonged drops
beside a ready voice or with every voice missing; and warm / initially cold
paused preparation at the playhead, a work start and a paused seek. Every
prepared start must queue a complete 256- or 512-frame buffer within its
first device callback (5.333 or 10.667 ms) and equal offline samples exactly
(128 frames before the buffer-size review follow-up below).
A ring test also grows while full and partially consumed, then skips an old
epoch; the concurrent order test grows twice while both threads run. `graph.rs` is
formatted as requested.

Prepared starts are best effort under cache pressure, pending media, or an
immediate play after an edit/seek; genuinely cold starts keep the no-loss
hold. Paused source windows now also look behind the playhead for reverse
runs. The project/device converter is constructed on the idle render thread;
the first audible pause also allocates and exercises the 2× master's FFT and
work buffers with discarded silence, then resets its stream bookkeeping.
A unit test proves that warming preserves exact forward/reverse PCM and tags.
This warms resources, not a signal-dependent speculative stream: the master
output-seek and converter histories still start on the real input at Play.
Caching complete streams at every direction/rate would also need separate
transport, voice histories, converted PCM and meter state. That larger change
is deferred; the one-callback prepared-start gate covers
normal forward play, the default app transport. Acoustic loopback and
listening are not claimed by the scheduled-presentation measurement.

### A6 initial play-start measurement (superseded by interactive-buffer follow-up)

The synthetic 10 s stereo WAV was captured through hidden, isolated real
app launches under `pm-electron-lock`, with 30 ready plays/seeks per engine
(alternating 0 and 3 s, 250 ms paused preparation, one initial unmeasured
play to resume the device). Scheduled output presentation, in milliseconds
from `PM.play`, was measured as follows:

- Today's WebAudio app: p50 **30.234**, p95 **32.031**, p99 **32.238**,
  range 27.432–32.238 ms.
- Native app: p50 **30.831**, p95 **38.781**, p99 **38.955**,
  range 28.621–38.955 ms. The native tail is **6.750 ms slower at p95**;
  this initial result failed the latency target; the follow-up below closes it.

The legacy capture maps the source's scheduled context time through
`AudioContext.getOutputTimestamp()`. The native capture maps the first
program sample on the device clock through the renderer's measured host
offset (at most 50 µs error in the captured native run). The native report
confirms one 48 kHz device, 15.625 ms reported output latency, zero
`hostClockRuns`, zero source-starved renders and no audio problems. Ring
underruns in that report include continuously running callbacks while
paused and are not a mid-run dropout measurement. Both apps report zero
page errors. This is scheduled-presentation evidence, not acoustic loopback.
The direct prepared-start gate separately measures command → first queued
buffer within one callback; it excludes host transport and output latency.

`engine/crates/pm-audio/tools/play-start.ts` reproduces the app capture. It
uses the corpus Node bridge because Bun's direct Electron inspector driver
does not resolve on this machine. Build `pm-wasm`, the native addon, and the
app first; keep generated media and output JSON outside the repository:

```sh
source engine/env.sh
nice -n 15 node engine/crates/pm-wasm/scripts/build.mjs
CARGO_PROFILE_RELEASE_LTO=off nice -n 15 node apps/desktop/scripts/build-engine-addon.mjs
nice -n 15 bun run --cwd apps/desktop build
"$PM_FFMPEG_DIR/bin/ffmpeg" -hide_banner -loglevel error -y -f lavfi \
  -i 'aevalsrc=0.000001*sin(2*PI*997*t):s=48000:d=10' -ac 2 \
  -c:a pcm_f32le /tmp/pm-audio-start.wav
~/Library/Caches/powermove-engine/bin/pm-electron-lock nice -n 15 \
  bun engine/crates/pm-audio/tools/play-start.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-start.json 30
```

### A6 initial validation

Pinned Rust 1.96.1, `kache`, four build jobs, niceness 15 and four test
threads: **30/30 plain and 30/30 stressed playback suites**, 31 tests each,
**1,860 executions**, zero failures and none ignored. Stress uses the
existing `PM_AUDIO_PLAYBACK_STRESS=1` profile (two shared CPU contention
threads and 8 ms cold-decode delay). Suite times were 21.065–21.260 s,
median 21.158 s. All 360 mapped-clip comparisons of 80,000 frames in both
channels were bit-exact, with zero source starvation or ring underruns in
those comparisons. Across 360 warm/paused-prepared starts, first-buffer
queue latency was p50 0.0845 ms, p95 0.361 ms, maximum **0.467 ms**, against
the 10.667 ms callback bound. The callback-paced all-missing and large-buffer
probes each assert zero mid-run ring underruns, exact ready samples and
sample-accurate re-entry.

Targeted checks pass: 46 library tests (one existing ignored catch-up
measurement), 11 decode-readiness, 14 mixer and 6 stretch tests; pm-node's
`present::audio::tests::recurring_render_errors_are_listed_once`; strict
`cargo clippy --release -p pm-audio -p pm-node --all-targets -- -D warnings`;
`cargo fmt -p pm-audio --check`; and production `cargo check --release
-p pm-node --lib` with test hooks disabled and the default release profile.
The app, pm-wasm and native addon builds pass; the paired hidden app capture
has zero page errors, zero source-starved renders and no audio problems.

As in A4, thin-LTO test linking failed with `Can't find section
__bitcode` (pm-media's bundled CrabbyAvif archive; fixed since, see
release-build.md). Tests, clippy and the measured addon use the local environment
setting `CARGO_PROFILE_RELEASE_LTO=off`; repository release settings are
unchanged. The repetition gate executes the built playback test binary
with the same four-thread arguments and FFmpeg library path supplied by
Cargo, avoiding Cargo's build lock during the app/addon builds. To repeat
with Cargo, use the existing 30-iteration commands above and the LTO-off
environment setting. No audio tolerance, corpus case or existing comparison
was relaxed. The follow-up below supersedes the initial caller and latency gaps.

### A6 interactive-buffer follow-up (buffer size superseded)

The 128-frame request below was wrong: it is WebAudio's render quantum,
not the device buffer today's app runs at. The review follow-up after this
section restores today's 256-frame output buffer and re-measures. The
latency attribution here still explains why an explicit size is needed.

The 512-frame inherited HAL default caused the slower start tail. Tracing
30 starts separated the app request, audio control arrival/application,
first output ready to queue, callback entry, and the HAL output timestamp.
App → audio control took 0.885–1.484 ms; control scheduling took 0.001–0.006
ms; preparing the first output took 0.014–0.021 ms. Waiting for the callback
then took 0.185–10.446 ms, followed by 11.602–11.650 ms of HAL scheduling
lead and the unchanged 15.625 ms device/stream latency. These were warm
starts, so opening the device was outside the measured path. Source/DSP
preparation and the bridge hop did not explain the additional tail.

`OutputStream` now requests 128 frames, clamped to the device's supported
buffer range; unknown-size devices retain their default. Devices whose
minimum is 4,096/8,192 frames retain that minimum and the producer's adaptive
reserve/growth behavior. Opening the stream can change HAL properties, so
latency, safety offset and buffer metadata are read again after opening.
The audio callback's observed frame count, rather than the request, proves
the size used in the measurement. No clock offset or reported output
latency was reduced to improve the numbers.

The final paired remeasurement of the verified binary, using 30 captures per
engine, 250 ms paused preparation, alternating 0/3 s, unmeasured initial
resume, hidden isolated app and scheduled-presentation method, measured:

| Engine | p50 (ms) | p95 (ms) | p99 (ms) |
| --- | ---: | ---: | ---: |
| Today's app | 29.929 | 32.128 | 32.433 |
| Native, 128-frame callback | 21.322 | 22.745 | 22.860 |

All three native percentiles are below today's. The native app used a
48 kHz device clock, reported the same 15.625 ms output latency, and had
zero host-clock runs, source-starved renders, audio problems or page errors.
All 30 traces match their app request on the host clock: app → audio control
0.824–1.211 ms, control scheduling 0.001–0.008 ms, first-output preparation
0.015–0.019 ms, callback wait 0.111–2.451 ms, and HAL lead 3.661–3.676 ms.
This remains scheduled device presentation evidence, not acoustic loopback.
The harness uses the packaged `native/pm_bridge.node` copy installed by the
addon build, so final captures can run after Rust target cleanup too.

`AudioStatus.startTiming` exposes the first-buffer trace and
`preparedStarts` exposes ready paused positions in project samples. Paused
audio status refreshes at the existing 33 ms reporting interval so it can
observe asynchronous preparation and device errors without a Play control.
The callback uses only atomics and reads the host clock once on the first
buffer of an epoch. Presenter and audio epoch counters are independent;
the capture tool associates a trace with its app request on the shared
host clock, rather than comparing the two counters.

The final follow-up playback gate repeats the updated binary **30/30 plain
and 30/30 stressed**, 31 tests per suite: **1,860 executions**, zero failures
or ignored tests. Suite times were 21.072–21.319 s (median 21.158 s). All
360 mapped comparisons of 80,000 stereo frames remained bit-exact with
zero source-starved renders. The 720 prepared starts split evenly between
callback sizes:

| Callback | Starts | Queue p50 / p95 / p99 (ms) | Maximum (ms) | One callback (ms) |
| --- | ---: | --- | ---: | ---: |
| 128 frames | 360 | 0.043 / 0.061 / 0.093 | 0.160 | 2.667 |
| 512 frames | 360 | 0.073 / 0.323 / 0.351 | 0.393 | 10.667 |

Both warm and initially cold-but-paused-prefetched voices pass; these tests
retain exact offline PCM, no source hold/drop, the work-area target and
paused-seek assertions. The 4,096-/8,192-frame and all-missing clock tests
continue unchanged. The local thin-LTO linking limitation described above
still applies; no repository profile or audio tolerance was changed.

The final targeted Rust checks pass: 49 library tests (one existing ignored
catch-up measurement), 11 decode-readiness, 14 mixer, 6 stretch tests, and the
presenter's recurring-audio-error diagnostic test. Strict release clippy for
`pm-audio`/`pm-node` all targets and the default production-profile `pm-node` library check with test hooks disabled pass.
The hidden transport app run passes J/K/L, play-around, and the new paused
seek/scrub regression (including ordinary `PM.setTime` calls). Its fourth,
unchanged waveform/meter test fails before reaching scrub assertions:
`peaksSource` never becomes `engine`, so the expected 150 engine buckets are
reported as zero. Diagnostics show the asset already has 150 legacy buckets
but the renderer's native peak-request/answer sets are empty. This broader
waveform synchronization gate remains open; its assertion was not removed,
skipped, or relaxed. No waveform routing/synchronization code was changed
for A6.

### A6 review follow-up: today's output buffer, overloads, idle cost

**Buffer size.** Today's app runs its `AudioContext` (latency hint
`interactive`) on Chromium's macOS output, whose
`AudioManagerMac::ChooseBufferSize` asks for 256 frames up to 96 kHz and
512 above, clamped to the device's I/O range. Every legacy capture reports
`AudioContext.baseLatency` 5.333 ms, 256 frames at 48 kHz. The previous
follow-up requested 128 frames, which halved the real-time deadline
(2.667 ms against 5.333 ms) and doubled callback wakeups (375 against 187.5
per second), a cost and underrun risk today's app does not have.
`OutputStream` now requests exactly Chromium's size, 256 frames, or 512
frames above 96 kHz. Both are clamped to the device range; unknown-size
devices keep their default. The real device (MacBook Pro speakers, 48 kHz)
runs 187.5 callbacks a second in every native measurement below, the same
rate as today's app.

**Overloads.** cpal 0.18 reports CoreAudio's `kAudioDeviceProcessorOverload`
as an `Xrun` stream error, which used to set the stream's diagnostic
"failed" flag. Overloads now leave the stream running (as WebAudio's does)
and are counted: `PlaybackStats::device_overloads` and
`device_callbacks`, and `AudioStatus.deviceOverloads` and
`deviceCallbacks` in the presenter report. Ring underruns still count
every callback while paused, because the ring is empty by design. Only
deltas over a playing window measure dropouts.

**Play-start, re-measured.** The measurement used the final binary at
256 frames, a hidden isolated app under the Electron lock, 30 paired
captures per engine, and the same scheduled-presentation method as
before. Values are in ms from `PM.play`:

| Seek → Play | Engine | p50 | p95 | p99 | max |
| --- | --- | ---: | ---: | ---: | ---: |
| 250 ms paused | Today's app | 29.412 | 32.027 | 32.315 | 32.315 |
| 250 ms paused | Native, 256 frames | 26.358 | 28.441 | 28.495 | 28.495 |
| same task (0 ms) | Today's app | 29.407 | 32.114 | 32.614 | 32.614 |
| same task (0 ms) | Native, 256 frames | 25.798 | 28.032 | 28.366 | 28.366 |

An earlier 256-frame capture, made before the preparation-order change,
agreed within 0.8 ms at every percentile (native p95 28.336 and 28.502).

`PM_AUDIO_START_WAIT_MS=0` seeks and plays in the same renderer task, so
paused preparation gets no time; the single-voice project's start still
leads today at every percentile. The native captures report a 256-frame
first callback, zero host-clock runs, zero device overloads and starved
renders, and no audio problems. Both engines report zero page errors.
Host-offset error was at most 50 µs (250 ms) and 150 µs (same task).
Legacy `baseLatency` was 5.333 ms in every sample.

**Device cost against today.** `engine/crates/pm-audio/tools/audio-load.ts`
measures both engines in the hidden app over 15 s windows: paused with the
device open, playing, and playing beside four busy loops (all at nice 15).
Each window records the Electron process tree's CPU time, top's energy
impact and coreaudiod's CPU. Two paired runs after the idle fix below:

| Window | Today's tree CPU / energy | Native tree CPU / energy | coreaudiod today / native |
| --- | --- | --- | --- |
| Paused | 6.6%, 5.3% / 6.5, 5.1 | 4.9%, 4.6% / 3.9, 4.1 | 3.1-3.5% / 3.3-3.5% |
| Playing | 50.0%, 51.3% / 47.4, 48.8 | 51.1%, 51.7% / 46.8, 47.7 | 3.8% / 4.1-4.2% |
| Playing under load | 33.2%, 34.0% / 31.4, 32.7 | 45.2%, 38.3% / 42.9, 36.2 | 3.0-3.1% / 3.1-3.6% |

Whole-tree CPU includes each engine's video path. Under load it reflects
the CPU share a niced app wins against the loops, so it is not an audio
cost. Today's Chromium exposes `AudioContext.playbackStats`. It counted zero
underrun events in every window, including under load. In every window,
native counted 187.5 callbacks a second, zero device overloads, and zero
ring underruns or starved renders while playing. top's idle-wakeup column
did not produce interval values usable as rates and is not reported.
Paused, today's AudioContext keeps rendering (it is never suspended), and
so does the native stream. The audio path alone is measured by
`examples/paused_cost.rs` on the real device. It uses a silent
-120 dBFS source, 10 s windows and process CPU from `getrusage`:

| Project | Paused CPU | Playing CPU | Overloads | Playing ring underruns |
| --- | ---: | ---: | ---: | ---: |
| 1 voice | 0.10-0.25% | 0.44-0.84% | 0 | 0 |
| 16 voices | 0.13-0.25% | 1.4-4.1% | 0 | 0 |
| 16 voices at 1.5x, looping | 0.13-0.30% | 25-32% | 0 | 0-7 (see below) |

Before this follow-up, the paused render loop re-bounded every voice's
spans and readiness for three starts on each 20 ms turn. That cost 0.8-2.2%
CPU idle with 16 stretched voices, against 0.19-0.22% on the base branch.
Fully prepared, unchanged starts are now re-checked once a second
(`PAUSED_REFRESH`), so evicted blocks are still queued again. The
presenter's paused 33 ms status wake is part of the 1-voice paused figure.

**Pre-existing loop-wrap underruns.** With 16 stretched voices looping
[4 s, 6 s), ring underruns occur at loop wraps: 0-7 counted 256-frame
callbacks over 10 s in six runs. The base branch (before A6, 512-frame default) also
counts 1-5 at 512 frames. A straight play-through of the same project
counts 0. A wrap re-enters the loop start with a fresh stretch pre-roll for
every voice in one render, and 16 voices nearly fill the 80 ms reserve. A6
did not introduce this, and its prepared paused histories are used only by
Play. Keeping a prepared loop-start history while playing is the likely
fix. It is listed as an open gap.

**Preemptible preparation.** Each paused start renders every voice, about
2.4 ms per stretched voice for a fresh pre-roll. The render loop now checks
for a control between starts, and between 8-group bounding steps, and
handles it first. The first start whose look-ahead is not yet bounded (the
playhead first) gets the turn's whole bounding budget: six steps, the same
total the pass used to spread over every start's windows. Reverse
look-behind windows follow once every look-ahead is bounded. A first
attempt bounded every start's look-ahead before any look-behind, and that
was slower for the playhead (0.91-0.99 s). The final order replaced it.
`play_preempts_paused_preparation_after_an_edit` edits 16 stretched voices
and sends Play at 48 phases, 2 ms apart, of the restarted pass:

| | Play acknowledged p95 / max (ms) | Playhead prepared | All three starts |
| --- | --- | ---: | ---: |
| No preemption, original order | 8.6-9.2 / 9.6-10.3 | not measured | 2.10-2.25 s |
| Preemption, original order | 2.3-4.0 / 3.3-4.3 | 0.65-0.73 s | 1.93-2.10 s |
| Final: preemption, playhead budget first | 1.5-1.8 / 1.9-2.0 | 0.49-0.52 s | 1.23-1.30 s |

The last row includes one run under the stress profile. A Play that lands
before the playhead is prepared is a fresh start. Its first 512-frame
buffer is queued 27.7-29.0 ms (p50), 43.3-48.4 ms (p95) and 49.8-51.6 ms
(max) after Play. For such a heavy project, that adds about the same to the
app-level start within half a second of an edit. That is the existing A3
start hold, not a dropout. Today's app has no equivalent wait, so it stays
on the orchestrator decision list above (item 1).

**Presenter.** A paused exact seek can create a view's audio link before
the view's first document arrives. `warm_audio` returns once any link
exists, so the device used to open cold at the first Play. `audio_for` now
sends `AudioCtl::Warm` whenever it creates a link. Paused exact seeks reach
audio only for views the engine draws, the same `Route::Engine` check that
scrubs and Play use. `present/audio.rs` is rustfmt-clean.

**Validation of the final binary.** These ran with pinned Rust, `kache`,
four build jobs, niceness 15, four test threads and the local
`CARGO_PROFILE_RELEASE_LTO=off` setting described above. The playback
suite passed **30/30 plain and 30/30 stressed**, 32 tests each (the new
preemption test included): 1,920 executions with zero failures or ignored
tests. Suite times were 24.10-25.23 s. All pm-audio targets pass: 49
library tests (one existing ignored measurement), 11 decode-readiness, 2
layouts, 1 legacy parity, 14 mixer, 3 resampler, 5 sources and 6 stretch.
pm-node passes its 37 `present::` unit tests and the present and scrub
integration tests. Strict release clippy for `pm-audio`/`pm-node` all
targets, `cargo fmt -p pm-audio --check`, a rustfmt-clean `present/`, and
the production `cargo check --release -p pm-node --lib` all pass. The hidden
transport app run passes J/K/L, play around and the paused seek/scrub
regression. The waveform/meter case still fails at the same assertion
(`peaksSource` never `engine`, 150 expected, 0 received), unchanged from
before this follow-up.

**What remains.** The loop-wrap underruns above, the A3 start hold after
an edit or cold seek, reverse and 2x master output-seek and converter
histories that still prime on Play, and the waveform/meter app case
(native peaks provenance, outside this lane). Bluetooth and USB output
devices were not attached to this machine. The buffer request matches
Chromium's for them too, but no measurement on them is claimed.
`RenderReport.heard_whole` is kept: it is a public diagnostic with its own
decode-readiness test.

## A7/A8: prepared loop histories and edit-to-play (2026-10-18)

> **Reading order.** The subsections from here through "A8 round 2" are the
> historical record of rounds 1 and 2. Their statements that A8 is still a
> blocker, that late voices omit a counted prefix, and that validation is
> incomplete are superseded: round 3 made every ordinary start complete and
> simultaneous, and round 4 (at the end of this section) qualifies it on
> distinct voices and closes the review's follow-ups. Current behavior and
> evidence are in "A8 round 3" and "A8 round 4".

A7 keeps the loop/work-area history when Play consumes the playhead's
prepared start. During playback the producer replenishes one prepared voice
between ring fills. A forward wrap installs that start's ready histories;
continuous entry from before the work area keeps its uninterrupted phase
history. Prepared histories and source windows remain bounded to three
positions. Preparation never advances meters, retune envelopes, or the
current vocoders. The first input chunk, including its deterministic
analysis pre-roll, is prepared before the optional long look-ahead.

A static level/fade edit retains a voice's prepared history only when its
source and complete map key agree. That key now includes each level's span,
on state, group on states, and audio-muted input as well as its existing
source, trim, speed, remap, conform, and rate inputs. Animated levels and
nonconstant maps invalidate prepared histories on a revision: a map
expression may depend on another edited property. Partially prepared starts
install only their ready voices; other voices take the existing no-loss
start hold. No missing voice is faded in late to improve these measurements.

The app-level trace exposed two additional causes of missed preparation:

- A callback publishes the first clock line before its device presentation.
  Pause in that interval now stays at the epoch's target instead of
  extrapolating backwards before it (or around the loop).
- Audio can stop a fraction of a frame past the displayed frame. Paused
  preparation warms that composition frame first, then retains the exact
  sample position for raw seeks and the work-area boundaries.

The minimal presenter change is `pm-node/src/present/audio.rs`: Play sends
SetLoop, Seek, and Play as one ordered batch through `Playback::commands`.
Previously each synchronous command could trigger another paused bounding
step before the next command arrived. The batch preserves command order,
error reporting, start timing, and the existing levels watch. A failed
command returns its error and leaves earlier commands applied.

Regressions compare both channels exactly: repeated prepared loop starts
against a fresh offline render; level edits against the edited offline mix;
map/span edits and animated maps invalidating histories; a pause before the
first device presentation; preparation of both displayed and raw seek
positions; and uninterrupted stretched PCM through the first entry into a
loop. The 16-voice level-edit regression requires the first 512-frame buffer
within one callback and asserts no source starve or committed drop. The
existing loaded remap comparisons retain their zero-underrun assertions and
bit equality.

The paired app harness is `engine/crates/pm-audio/tools/playback-gates.ts`.
It opens hidden isolated apps under `pm-electron-lock`, using the same
10 s stereo WAV, sixteen speed-1.5 voices, and work area [4 s, 6 s) for
both engines. It measures 60 s of continuous looping after a 1 s settling
period, then 30 edit-and-Play requests in the same renderer task. Before a
level/speed edit it seeks to 4 s and allows 250 ms for preparation;
`PM_AUDIO_START_WAIT_MS=0` alternates previously unprepared 2 s / 3 s
same-task seek/edit/Play requests.
`PM_AUDIO_LOAD_THREADS=4` runs four joined busy-loop processes at the
harness's inherited niceness throughout the stressed capture. Output is
scheduled device presentation through WebAudio's output timestamp or the
native device clock and measured host offset, without acoustic loopback.
Native traces also record the pre-edit prepared positions and first-buffer
timing. Generated media, app profiles and raw captures stay outside Git.

Reproduce with the A6 app/addon build commands above, then:

```sh
L=~/Library/Caches/powermove-engine/bin/pm-electron-lock
$L nice -n 15 bun engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a7-plain.json 30
PM_AUDIO_LOAD_THREADS=4 $L nice -n 15 bun \
  engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a7-stress.json 30
PM_AUDIO_START_EDIT=speed PM_AUDIO_LOOP_SECONDS=0 $L nice -n 15 bun \
  engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a8-speed.json 30
PM_AUDIO_START_WAIT_MS=0 PM_AUDIO_LOOP_SECONDS=0 $L nice -n 15 bun \
  engine/crates/pm-audio/tools/playback-gates.ts \
  /tmp/pm-audio-start.wav /tmp/pm-audio-a8-immediate.json 30
```

### Paired app latency: delivered and remaining (historical, round 1–2)

The final speaker captures used a 48 kHz device, 256-frame native callbacks,
15.625 ms native device/stream latency, and WebAudio `baseLatency` 5.333 ms.
Each row has 30 requests per engine. Times below are ms from `PM.play` to
scheduled presentation of the first program sample:

| Scenario | Engine | p50 | p95 | max |
| --- | --- | ---: | ---: | ---: |
| Level edit, playhead prepared before the edit | Today's app | 30.920 | 32.518 | 32.529 |
| Level edit, playhead prepared before the edit | Native | 28.547 | 31.604 | 31.784 |
| Speed edit invalidating every history | Today's app | 29.633 | 32.331 | 32.434 |
| Speed edit invalidating every history | Native | 143.900 | 146.738 | 146.750 |
| Same-task unprepared seek, level edit, Play | Today's app | 31.011 | 32.410 | 32.515 |
| Same-task unprepared seek, level edit, Play | Native | 171.279 | 186.590 | 203.590 |

The level-edit case meets all three comparison points. **A8 is still a
blocker for invalidated or unprepared starts.** The native first-buffer
trace measures up to 112.988 ms from audio-control application to queueing
after a speed edit, and 113.346–169.195 ms for the immediate unprepared
seeks. These captures have consistent 15.625 ms native clock/status latency
in all 30 samples and zero page errors; this failure is independent of the
Bluetooth metadata discrepancy below. Cold starts preserve every initial
sample through the no-loss hold. Meeting today's latency still requires
moving the serial per-voice map/pre-roll work off the producer or reusing
its exact evaluated maps; faster dispatch and retained static histories do
not close the cold case.

One earlier stressed app capture switched output configuration between the
paired launches: WebAudio reported 168 ms output latency, native audio
status reported 160 ms, but the renderer's clock carried 15.625 ms. The
native status retains the device descriptor from opening, while playback
watches the current default output, so this is evidence of inconsistent
metadata during output changes, not proof of which latency is acoustically
correct. Its raw renderer-clock comparison (172.912/175.511/176.004 ms today,
37.753/42.897/43.611 ms native) is **not a latency pass**. Reconstructing the
native first-buffer presentation using the reported status latency instead
gives 182.079/187.272/187.986 ms. That alternative is an inference, also not
a same-device pass. Both paths recorded zero ring underruns for their 60 s
loop windows; native also recorded zero decode starvation and device
overloads. A repeat with consistent output metadata is reported separately.

### A7 real-device loop gate and repeated stressed latency (historical, round 1–2)

The final plain capture and the repeated stressed capture each played
sixteen 1.5x voices over [4 s, 6 s) for 60 s: **zero loop-window ring
underruns in both today's app and the native engine**. Native also counted
zero source-starved renders and zero device overloads in both windows. The
stressed capture used four busy-loop processes. The repeat had consistent
15.625 ms native clock/status latency in every sample; WebAudio reported
24 ms output latency and a 5.333 ms base latency, as in the plain capture.

| Level edit under stress | p50 (ms) | p95 (ms) | max (ms) |
| --- | ---: | ---: | ---: |
| Today's app | 30.034 | 32.632 | 32.635 |
| Native | 28.530 | 32.250 | 32.938 |

P50 and p95 meet today's latency. Maximum latency is **0.303 ms slower**,
so this row also misses the brief's strict every-number requirement. It is
not rounded into a pass. Combined with the cold-start rows above, A8 remains
open despite the improvement for already prepared static level edits.

### Final Rust validation and qualification (historical, round 1–2)

The final binary passed **30/30 plain and 30/30 stressed playback suites**,
35 tests each: 2,100 executions, zero failures and none ignored. Stress is
`PM_AUDIO_PLAYBACK_STRESS=1` (two shared CPU contention threads and 8 ms cold
decode delay). Suite times were 23.04–23.45 s plain and 23.09–23.42 s
stressed. The 360 mapped comparisons of 80,000 stereo frames remained exact
(57,600,000 sample values), with zero source starvation and ring underruns
in those comparisons. The original simulated-device patience and all
comparison tolerances are unchanged.

| Sixteen-voice level edit → first 512-frame buffer | Starts | p50 (ms) | p95 (ms) | max (ms) |
| --- | ---: | ---: | ---: | ---: |
| Plain | 120 | 0.719 | 1.274 | 1.683 |
| Stressed | 120 | 0.700 | 0.839 | 2.701 |

Every start met the one-callback 10.667 ms bound and the edited offline PCM
comparison. Before those final repetitions, one 35-test suite during
concurrent builds failed the loaded-remap zero-underrun assertion: one ring
underrun, zero source-starved renders, and zero differences in its 80,000
compared frames. The final consecutive runs pass the same assertion; the
earlier scheduling failure remains evidence for review, not an allowed
difference.

Targeted checks pass: 52 library tests (one existing ignored catch-up
measurement), 11 decode-readiness, 14 mixer, 6 stretch and all 37 presenter
unit tests. Strict release clippy for all pm-audio/pm-node targets, pm-audio
formatting, and the normal production-profile pm-node library check with
test hooks disabled pass. App, pm-wasm and native addon builds pass.
Validation used pinned Rust 1.96.1, kache, four build jobs, niceness 15 and
four test threads, with the existing local `CARGO_PROFILE_RELEASE_LTO=off`
setting for test/addon links. The normal production check uses the default
release configuration; no successful thin-LTO test link is claimed. The
30-iteration reproduction commands in the A4/A6 sections still apply.

### A8 round 2: parallel preparation and measured late joins (historical, round 1–2)

The serial cold path was CPU-bound. In an opt-in timing capture of sixteen
stretched voices at 48 kHz, the eleven actual speed changes (the first of
twelve samples did not change speed) averaged approximately:

| Stage | Wall time (ms) |
| --- | ---: |
| Control building / delivery before application | 12.3 |
| Source bounds, including their map evaluations | 36.0 |
| Vocoder history map evaluations | 25.1 |
| Signalsmith construction/reset, output seek and processing | 45.1 |
| Cached source reads and copies | 0.3 |
| Explicit no-loss source wait | 0.0 |
| Other render work, allocation, scheduling and timer granularity | 2.8 |
| Queue to scheduled device output | 25.1 |

The applied-control-to-queue interval averaged 109.4 ms. These are timed
wall scopes, not sampled CPU counters; scheduling inside a scope is charged
to that scope. Reads were already cached in this edit scenario. The
instrumented end-to-end p50 was 147.0 ms, close to the earlier uninstrumented
143.9 ms capture. The source hold was not the cause of this regression.
`PM_AUDIO_PROFILE=1` enables the counters in `StartTiming.preparation`;
normal playback leaves timing disabled and the callback adds no profiling
work.

`preparation.rs` owns a process-wide worker pool capped at
`max(1, available_parallelism - 2)` (fourteen on the measured machine).
Workers use reduced thread priority: utility QoS with relative priority -5
on macOS, and niceness 15 on Linux. Project edits and seeks submit work at
control application, before Play's render turn. Batched transport commands
submit only their final target; intermediate Pause/SetLoop positions do not
launch obsolete histories. Playhead starts and the nearest sounding voices
come first. Loop/work-area replenishment uses at most one background job
at a time, preserving A7's render reserve. Stale document/seek generations
cancel at map/read boundaries, and completed jobs must still match their
original history allocation before installation.

Each job receives an evaluator from the configured factory. The native
presenter's one-line `set_standard_evaluator` opt-in identifies its standard
evaluator with the expression engine and no custom hooks. Constant maps
compile the original per-ancestor arithmetic, active/span tests, fades and
source-end calculation; deriving a single intercept would change rounding.
Those constant points also remove repeated content resolution from playback
and source bounding. Expressions retain full evaluation. Custom evaluator
factories retain their hooks and invalidate prepared histories on every
project revision. The ordinary audible render still reports failed
preparation's decode/evaluation errors.

The deterministic prefix is unchanged. Signalsmith reports 2,880 input and
2,880 output latency samples; the existing deterministic prefix is 11,520
samples at 48 kHz. Trying 0, 5,760, 8,640 and 11,504 samples at 0.75x and
1.5x, from 2, 3 and 4 seconds, changed every one of the 8,192 compared
stereo values in each of 24 probes. Even the nearest shorter control-aligned
candidate changes the existing offline PCM. The ordinary mapped playback
comparisons continue against the unchanged offline algorithm.

Parallel preparation with complete first buffers alone still measured
35.464/37.962/37.962 ms for speed edits and 38.162/46.062/46.062 ms for
unprepared seeks (p50/p95/max in twelve-start diagnostic captures). That
miss enabled the requested A4 alternative: an actually ready audible voice
starts the run, and late voices omit a counted prefix and join at the
current project sample. Workers catch up through real input from the
original start, retaining its offline vocoder phase. During this short
join period the ring keeps two callbacks ahead instead of the whole normal
look-ahead. A paused partial start discards its consumed prepared entry;
an inactive/out-of-span prepared voice cannot release other cold voices.
`StartTiming.lateVoiceFrames` records the omitted frames per voice, and the
existing starvation/drop counters record this loss. Prepared starts retain
complete PCM and have no such omission. Exports continue to wait.

The added sixteen-voice cold test verifies every stereo sample: each voice
is absent only for its recorded prefix and then equals the unchanged
offline render from the original start. It preserves the one-callback
first-buffer bound. Existing mapped comparisons and their patience,
zero-starvation/underrun assertions and tolerances are unchanged.

A subsequent prefetch fix queues all source blocks for the bounded canonical
prefix before a preparation worker reads its first block. Constant maps
use two exact endpoint evaluations and a conservative reader/latency pad;
dynamic maps reuse the bounded span traversal. It does not require decoding
an entire file. A gated AAC-source regression observed only one requested
block before the fix and two concurrent requests after it. Its extended
wall deadline isolates this queue observation; existing playback patience,
PCM comparisons and underrun assertions remain unchanged. Completed tasks
also pin their history allocation with a strong reference until accepted,
preventing a recycled allocation address from accepting obsolete work.

#### Latest completed paired captures (historical, round 1–2)

Thirty starts per engine, sixteen stretched voices, 48 kHz, 256-frame
callbacks. Native status and device-clock latency agreed at 15.625 ms;
WebAudio output latency was 24 ms. These are scheduled device presentation
times, not an acoustic loopback. Level-edit rows additionally looped
[4 s, 6 s) for 60 seconds per engine; stress used four busy-loop processes.
Each start continued for 120 ms after first onset to observe late joins.
Unprepared cases alternated seeks to 2 and 3 seconds with seek/edit/Play in
one renderer task; neither target was prepared before the edit.

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Every percentile passes |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 28.938/31.735/31.821 | 24.969/28.193/28.509 | Yes |
| Speed edit, plain | 30.324/32.425/32.621 | 30.163/32.651/35.038 | No |
| Unprepared seek/edit, plain | 30.010/32.415/32.515 | 29.453/31.390/31.859 | Yes |
| Prepared level edit, stressed | 30.137/32.130/32.719 | 25.825/28.560/28.682 | Yes |

The latest speed-edit p95 misses today by
0.225667 ms and its maximum by
2.416667 ms. These are raw failures, with no
rounding or noise exemption. Both completed 60-second loop pairs have zero
underruns in both engines, and native has zero loop-window source
starvation and overloads. A8 is still open.

| Cold case | Longest omitted voice prefix p50/p95/max (ms) | All voices at output p50/p95/max (ms) | Today node-start spread p50/p95/max (ms) |
| --- | ---: | ---: | ---: |
| Speed edit, plain | 32.000/48.000/48.000 | 62.270/77.988/79.424 | 0.000/0.000/0.000 |
| Unprepared seek/edit, plain | 26.667/32.000/32.000 | 55.742/61.482/61.492 | 0.000/0.000/0.000 |

All-voices output is first scheduled output plus the longest counted
omission. This exposes the partial-mix cost beyond first onset; it is not
a measurement of subjective audibility. Today scheduled all sixteen nodes.
The plain prepared baseline did show a 5.333 ms maximum node-start spread;
it is not assumed to be zero in every capture. Prepared native starts
omitted no frames.

#### Three paired stressed repeats and all-ready controls (historical, round 1–2)

The following completed repeats precede the source-prefetch follow-up.
Each row contains thirty starts per engine plus a 60-second loop per
engine. They are evidence for the pool implementation, not final-revision
qualification. Every loop had zero underruns; native had zero loop-window
source starvation and overloads.

| Earlier stressed repeat | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | ---: | ---: |
| 1 | 29.933/32.325/32.433 | 25.370/28.084/28.333 |
| 2 | 30.128/32.629/32.671 | 26.549/28.524/28.699 |
| 3 | 30.035/32.336/32.770 | 25.127/28.048/28.405 |

The maxima ranges do not overlap: 32.433–32.770 ms today and
28.333–28.699 ms native. The revised engine was faster in all
three repeats. This does not establish that the earlier 0.303 ms native
excess was noise; no noise exemption is claimed. Only the first repeat
completed after the prefetch follow-up. The remaining two final repeats
and final stressed cold cases were stopped before completion.

Earlier optimized all-ready controls temporarily removed only the
first-ready exit from the join wait. Their source and addons were restored
byte-for-byte afterward. They retained complete audio and still missed
today, justifying the requested A4 experiment:

| Earlier all-ready control | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | ---: | ---: |
| Speed edit | 30.236/32.234/32.427 | 32.639/35.330/35.540 |
| Unprepared seek/edit | 30.119/32.321/32.427 | 33.138/35.496/35.590 |

The all-ready control was not repeated after the source-prefetch fix. A4's
up-to-48 ms omitted prefix in the latest speed capture remains a measured
loss relative to today's complete start, even where first onset passes.

#### Validation status: incomplete repetition gate (historical, round 1–2)

The last full playback smoke run passed all 37 tests. Targeted checks
passed 56 library tests (one existing ignored measurement), 11 decode
readiness tests, 14 mixer tests, 6 stretch tests and 37 presenter tests.
The smoke and direct tests precede the final conservative reader-tap bound
adjustment; the final revision rebuilt its test executable and production
addon, and passed strict release clippy for pm-audio/pm-node all targets.
The earlier default production-profile pm-node check without test hooks
passed. Test/addon links used the existing local
`CARGO_PROFILE_RELEASE_LTO=off` workaround; a new default thin-LTO runtime
link is not claimed.

The repetition history is retained rather than combined into a pass:

- An initial interrupted campaign had passing runs, then cold/warm WAV
  fixture paths were isolated to prevent concurrent tests sharing a file.
- The next campaign passed eight plain and eight stressed suites, then
  plain run nine failed the existing loaded-remap zero-underrun assertion
  once. Its source-starvation count was zero and its 80,000-frame PCM
  comparison was exact. This led to the concurrent-prefix-queue fix and
  its deterministic gated-source regression; the failed log is preserved.
- The final-revision 30/30 plain and 30/30 stressed campaign had **zero
  completed runs** when the mandatory under-40-GiB disk guard stopped work.
  The final all-ready diagnostic had not started. The build target was
  deleted, but free disk remained below the guard. No final repetition
  pass is claimed.

Existing mapped comparisons remain against the unchanged offline PCM;
no tolerance, source-starvation assertion or ring-underrun assertion was
relaxed. Completing all final captures, the all-ready control and sixty
repetition suites remains required before sign-off. Only one additional
line changed pm-node's presenter in this round: its evaluator-factory
opt-in. No render.rs change was needed.

### A8 round 3: complete and simultaneous starts

The round-2 ordinary-start omission path is superseded. Play now waits
until every stretched voice has complete first-output PCM. The mix is
published in one interleaved buffer at one project sample. A CPU preparation
job cannot release a partial mix. Only source input still missing after
120 ms of device callbacks can use the A3 per-voice safety net; exports
continue to wait for input.

The start's independent workers still begin on Project/Seek, before Play.
Repeated voices can now share an immutable prefix of their stretched source
signal. Reuse requires the same source identity, exact voice bounds and
bit-identical compiled static map at every ancestor, including active state
and conform factor. The standard evaluator opts in; hooks, expressions and
animated maps keep their configured evaluation. Each voice still applies
its own gain, fades, layout matrix and strip in the original order.

The prefix contains 4,096 frames at 48 kHz (85.333 ms), scales with project
rate, and rounds up to a ring chunk. A representative job prepares it first.
Dependent jobs then prime independent vocoder histories while complete PCM
is already available for every voice. The producer uses that PCM until
the independent history can continue. Failed input reads retry from the
original start's phase. The canonical 11,520-sample pre-roll at 48 kHz is
unchanged; the existing short-prefix experiment and exact mapped
comparisons remain in place.

The process-wide pool remains capped at max(1, cores - 2), fourteen workers
on the measured machine. Foreground preparation requests user-interactive
QoS; boundary replenishment keeps one utility job at a time. The start's
control batch and producer request the same foreground class through scoped
guards, then restore the previous effective class. The guards stay on their
creating thread, and a nested-scope regression verifies restoration.

A live gain edit preserves the immutable prefix and original phase across
retune glides, including a canonical-grid boundary. Cancelled histories
return to the audible mix instead of receiving new detached jobs.
Replacing an evaluator clears the previous factory's histories and PCM.
Deterministic regressions cover both cases, different gain/fade/map
identities, independent-history handoff and two grouped decoder-stalled
voices recovering after the A3 bound.

The native I/O request is now 64 frames through 48 kHz, 256 above 48 through
96 kHz, and 512 above 96 kHz, clamped to the device's supported range.
Unknown ranges keep the device default. This reduces callback phase and
scheduling lead: the callback copies premixed PCM and publishes the clock,
while DSP and allocation remain on the producer/workers. Today's measured
WebAudio base latency is 5.333 ms, equivalent to 256 frames at 48 kHz.
The smaller native callback has a tighter real-time deadline; the paired
stressed loop captures below qualify this choice on the measured device.

StartTiming records each voice's first project sample, spread, summed
omitted prefix, A3 safety-net use and callbacks that ran out of program PCM
after first onset. Captures assert sixteen voices at the target sample and
zero for every loss/spread counter. The existing cumulative ring counter
still includes intentionally empty paused/start-wait callbacks; the A7
loop gate measures its change inside the steady playing window.
prefillReadyUs additionally records worker completion before the
producer's readyUs, separating compute from publication handoff.

#### All-ready scope profile

The baseline profile enabled the all-ready policy before adding shared
prefill. In eleven actual speed changes, the producer waited 7.260 ms.
Summed worker DSP took 63.668 ms; map evaluation took 0.127 ms and cached
reads 0.707 ms. A second split-scope capture measured DSP initialization
4.292 ms, output-seek priming 6.107 ms and processing 44.033 ms. Ready-to-queue
was only 0.091 ms in the baseline. These timed wall scopes identified
processing as the critical preparation cost.

The final opt-in captures separate these stages as follows (mean ms):

| Stage | Final speed edit | Final unprepared seek/edit |
| --- | ---: | ---: |
| Frontend/IPC/graph before audio command | 1.755 | 0.810 |
| Audio command channel delivery | 0.053 | 0.111 |
| Applied command to worker prefill ready | 3.697 | 3.646 |
| Worker ready to producer continuation | 0.010 | 0.077 |
| Producer join/poll scope | 3.695 | 3.674 |
| First mix | 0.036 | 0.036 |
| Ready to queue | 0.096 | 0.387 |
| Queue to callback | 0.622 | 0.506 |
| Callback to HAL presentation host time | 2.340 | 2.340 |
| Explicit source hold | 0.000 | 0.000 |
| Accepted worker DSP before first queue, summed | 6.709 | 31.146 |
| Accepted DSP initialization, summed | 0.323 | 1.587 |
| Accepted DSP seek, summed | 0.639 | 2.815 |
| Accepted DSP processing, summed | 5.744 | 26.735 |
| Accepted map evaluations, summed | 0.002 | 0.001 |
| Accepted source reads, summed | 0.050 | 0.213 |

Worker sums overlap the producer join. With prefill, only jobs accepted
before the first queue appear in that snapshot; the remaining independent
jobs finish afterward. The table does not claim their total CPU work was
eliminated. Cached prefill completion can precede Play, so handoff intervals
clamp their origin to command application. Speed scope averages exclude
the first unchanged-speed sample (eleven actual changes); the seek profile
has twelve samples. Normal qualification captures leave profiling off.

Smaller foreground pools did not improve the measured wait: 8/10/12-worker
sweeps averaged 7.023/6.986/6.159 ms, so the fourteen-worker cap remains.
The start-priority guards and shorter I/O phase complement the prefill;
the final captures measure their combined behavior.

#### Final paired qualification

Each row has thirty starts per engine on the same sixteen-voice project.
The plain and three stressed level-edit rows each include a 60-second
[4 s, 6 s) loop per engine. Stress uses four busy-loop processes. Cold
seek/edit cases alternate 2 s and 3 s, neither in the pre-edit prepared
start list; seek/edit/Play occur in one renderer task. Prepared cases
allow 250 ms before the edit. Starts continue for 120 ms after first clock
observation, covering prefill handoff. Each speed sequence retains its
first unchanged-speed sample and twenty-nine actual changes.

The clock reports scheduled device presentation, using the same method
for both engines. This is not an acoustic loopback. All native samples
use 48 kHz, 64-frame callbacks and 15.625 ms reported device latency;
clock and status metadata agree. Today's base latency is 5.333 ms
(256 frames) and output latency is 24 ms. The two all-ready control rows
use the same shipping policy and binary, with every completeness assertion
enabled.

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 29.703/32.426/32.432 | 19.680/20.575/20.619 | None |
| Speed edit, plain | 31.027/32.717/32.736 | 24.026/24.785/24.891 | None |
| Unprepared seek/edit, plain | 29.823/32.411/32.604 | 24.040/24.710/25.803 | None |
| Prepared level edit, stress 1 | 29.236/32.237/32.404 | 20.093/20.702/20.740 | None |
| Prepared level edit, stress 2 | 30.137/32.436/32.529 | 19.973/20.685/20.738 | None |
| Prepared level edit, stress 3 | 30.531/32.533/32.630 | 19.981/20.608/21.061 | None |
| Speed edit, stressed | 29.037/32.534/32.636 | 23.806/24.828/24.921 | None |
| Unprepared seek/edit, stressed | 29.322/32.527/32.660 | 23.765/24.862/25.965 | None |
| All-ready speed control | 30.333/31.821/32.216 | 23.998/24.769/24.810 | None |
| All-ready seek control | 30.125/32.509/32.617 | 23.950/24.706/26.830 | None |

All ten final paired cases meet today at every requested percentile.
Every final native start has sixteen voices at its exact target sample,
zero start spread, zero omitted frames, zero A3 safety-net use and zero
program underruns. All four paired loop windows have zero underruns in
both engines, with zero native source starvation and device overloads.

| Stressed repeat | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | ---: | ---: |
| 1 | 29.236/32.237/32.404 | 20.093/20.702/20.740 |
| 2 | 30.137/32.436/32.529 | 19.973/20.685/20.738 |
| 3 | 30.531/32.533/32.630 | 19.981/20.608/21.061 |

Observed maximum ranges: **32.404–32.630 ms today**, **20.738–21.061 ms native**. They do not overlap. The three distributions are reported without a noise exemption for earlier misses.

The prepared level-edit capture also records transport stages without DSP
profiling overhead (thirty plain starts, p50/p95/max in ms). Its immutable
prefill is already ready before Play; zero-valued disabled DSP scopes are
not interpreted as zero CPU work.

| Prepared level-edit stage | p50/p95/max (ms) |
| --- | ---: |
| Frontend/IPC/graph before audio command | 1.090/1.192/1.217 |
| Audio command delivery | 0.093/0.114/0.116 |
| Applied command to all-ready | 0.004/0.005/0.005 |
| Ready to queue | 0.199/0.228/0.517 |
| Queue to callback | 0.356/1.196/1.302 |
| Callback to HAL presentation host time | 2.337/2.343/2.343 |

#### Retained failed qualification attempts

An intermediate 256-frame/2,048-frame-prefill speed pair measured
28.950/32.006/33.410 ms native against 28.818/31.915/32.202 ms today.
Its raw p50/p95/max misses were +0.131667/+0.090667/+1.207667 ms.
After prefill review fixes, the first 128-frame qualification pass completed
four pairs before the speed maximum still failed. Those data remain
separate from the final revision:

| Earlier 128-frame case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit | 31.033/32.533/32.812 | 22.309/23.143/23.241 | None |
| Speed edit | 29.419/31.609/32.421 | 26.209/28.651/32.587 | max +0.165667 ms |
| Unprepared seek/edit | 29.417/31.826/32.327 | 25.785/26.976/28.437 | None |
| Stressed prepared edit | 30.826/32.033/32.535 | 21.652/22.892/23.160 | None |

The earlier maximum's trace contained 6.336 ms of delivery/build work,
6.557 ms from applied command to readiness, 0.103 ms ready-to-queue,
0.302 ms queue-to-callback, 3.664 ms callback host lead and 15.625 ms
device latency. It had zero omission/spread and no host-offset error.
The qualification stopped at the next source guard before another pair
or any repetition suite. The final source adds scoped start priorities,
an independent prefill-ready timestamp and the 64-frame request; all final
captures and repetitions were restarted. Earlier failed numbers remain
raw failures rather than being reclassified as noise.

#### Final repeated correctness and delivery checks

The frozen final playback executable passed **30/30 plain and 30/30 stressed
suites**, 38 tests each (2,280 executions). Stress includes the shared CPU
load and 8 ms cold-decode delay. The original mapped comparisons retain
their assertions and patience: 360 comparisons of 80,000 stereo frames,
57,600,000 sample values, exact PCM and zero mapped source starvation or
ring underruns. The cold sixteen-voice comparison also checks complete
output through its independent-history handoff.

The earlier round-2 plain-run-nine loaded-remap underrun was not reproduced
in these sixty final runs. Its failed log remains preserved. The bounded
prefix-input queue regression and source-prefetch fix remain in place;
no tolerance, mapped case or underrun assertion was relaxed.

| First 512 frames, sixteen voices | p50/p95/max (ms) |
| --- | ---: |
| plain, prepared level edit, 120 starts | 0.343/5.522/10.062 |
| plain, cold seek/speed edit, 120 starts | 4.640/8.287/11.810 |
| stress, prepared level edit, 120 starts | 0.330/3.944/5.703 |
| stress, cold seek/speed edit, 120 starts | 4.495/9.064/14.046 |

The elapsed-time observer also reports values above the 10.667 ms first-buffer
target, despite every existing queue/PCM assertion passing. The polling loop
checks queue readiness, tests the deadline separately and prints elapsed
time afterward. These are not an atomic publication timestamp: the raw data
cannot distinguish a late queue from a delayed observer. No test bound is
relaxed, and these exceedances are retained without a noise exemption. They
are separate from the paired app's scheduled-presentation gates above.

| Suite observation | Raw elapsed (ms) | Excess above 10.667 ms |
| --- | ---: | ---: |
| plain-22, cold=true, line 24 | 11.810 | +1.143333 |
| stress-15, cold=true, line 18 | 14.039 | +3.372333 |
| stress-15, cold=true, line 20 | 13.831 | +3.164333 |
| stress-15, cold=true, line 24 | 12.020 | +1.353333 |
| stress-23, cold=true, line 18 | 14.046 | +3.379333 |
| stress-5, cold=true, line 24 | 11.410 | +0.743333 |

Final targeted checks pass: 61 library tests (one existing ignored catch-up
measurement), 11 decode-readiness, 14 mixer, 6 stretch and 37 presenter
tests. Strict release clippy for pm-audio/pm-node all targets, formatting,
the default production-profile pm-node library check without test hooks
and the production native addon build pass. Test/addon links use the
existing local CARGO_PROFILE_RELEASE_LTO=off workaround; no fresh default
thin-LTO runtime link is claimed.

The only additional pm-node source change in round 3 is the four-line
control-batch priority guard in present/audio.rs. Over the whole branch,
`pm-node/src/present/audio.rs` changes in three commits, 31 insertions and
19 deletions: 8345dba5 (SetLoop/Seek/Play as one ordered command batch),
21fe9c6c (the `set_standard_evaluator` opt-in) and d2ccfadb (the start
priority guard). Round 4 changes nothing in pm-node. No render.rs change is
needed. The capture/repetition manifest verifies unchanged native addons
and playback executable throughout the final campaign. Raw evidence stays
outside the repository.

### A8 round 4: distinct voices and review follow-ups (2026-10-18)

The round-3 qualification used sixteen bit-identical voices, the best case
for the shared immutable head: one representative job served all sixteen.
Round 4 qualifies starts where no voice can share a head, and fixes the
review's correctness follow-ups. Commits: e92a267e (engine), 30b16308
(playback tests), ad3baecb (harness), 943b81fd, 987ef700 and fae25da8
(test synchronization and the first-buffer check). No pm-node, protocol or
render.rs change.

**Changes.**

- *Distinct-voice harness mode.* `PM_AUDIO_START_DISTINCT=trim` trims voice
  *i* by *i* × 61.3 ms, so every voice has its own stretched signal.
  Native samples must report `prefillReadyUs` 0 (no shared head) in
  addition to the existing completeness assertions. Reproduce with the
  commands above plus `PM_AUDIO_START_DISTINCT=trim`.
- *Buffered start lead.* A start's own job now renders 1,024 frames
  (rate-scaled, a whole number of ring chunks) of the voice's continuous
  vocoder output before reporting ready, kept in the history's output
  buffer. Without it, sixteen distinct voices whose second vocoder step fell
  inside the first 512 frames (for example 2 s at 1.6x) made the producer
  run sixteen vocoder steps serially between the first and second ring
  chunk: about 4 ms with only one 256-frame chunk queued. The lead is the
  same deterministic stream, so PCM is unchanged; the existing bit-exact
  comparisons run through it.
- *Per-voice A3 safety net.* When the A3 bound fires, only voices whose own
  first-chunk input is still missing are marked late. A healthy voice whose
  CPU job is still queued stays on its live history, which the ordinary
  render primes by the canonical pre-roll: complete from its first sample.
- *Bounded wait at an exhausted shared head.* Jobs carry a claim. When a
  shared head ends before a voice's independent history, the producer claims
  every such history no worker has started and primes it itself from the
  head's original start (`prepare_join`), bit exact. Only a history a worker
  is already running is awaited, so the wait is bounded by running work, not
  by a saturated pool's queue. Paused synchronous preparation uses the same
  claim, so no history is primed twice.
- *Producer publication times.* `StartTiming.publications` records the
  cumulative frames and host time of the run's first 16 ring pushes, taken
  by the producer after each push. The headless check now waits for the
  buffer and reads the producer's own publication time, so a descheduled
  observer no longer decides the result. Prepared starts (no vocoder work
  after Play) must publish 512 frames within one callback by that clock.
  Cold starts report it per start; see the repeated suites below for why
  it is not asserted, and the decision list.

**New regressions.** The first three fail with their fix removed (checked
by mutation); the map and distinct-voice tests guard existing behavior:
`the_a3_safety_net_omits_only_voices_whose_own_input_is_missing` (a
genuinely undecoded source beside a shared head and a CPU-pending healthy
voice; first 8,192 frames bit exact against the mix without the late voice),
`an_exhausted_head_never_waits_for_an_unstarted_history` (two unstarted
histories claimed, the running one awaited, 8,192 frames past the head bit
exact), `a_start_job_buffers_its_opening_output` (the lead is read with
panicking map/read callbacks at three targets),
`compiled_constant_maps_match_through_precomps_and_embedded_video` (one- and
two-level linear-parent precompositions and a video's embedded audio; every
`Point` field and map point bit-identical to the full evaluator), and the
playback tests `edit_then_play_reuses_sixteen_distinct_stretched_voices`
and `cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous`
(sixteen voices at the target, zero spread, omission, A3 use and program
underruns, no shared head, and all 8,192 stereo frames equal to the sum of
per-voice offline renders).

#### Paired qualification, round 4 revision

Same method as round 3: thirty starts per engine per row, scheduled device
presentation (no acoustic loopback), hidden isolated apps under
`pm-electron-lock`, four busy-loop processes for stress. Revision
ad3baecb; the playback executable and both addons were hashed
before and after the campaign; engine source and addons are unchanged
through fae25da8. Every native sample used 48 kHz, 64-frame
callbacks and 15.625 ms device latency; today's base latency was 5.333 ms and
output latency 24 ms.

Identical voices (round-3 project, re-measured on this revision):

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 30.429/32.720/32.825 | 19.875/20.563/20.698 | None |
| Speed edit, plain | 30.334/31.737/32.424 | 23.932/24.863/24.933 | None |
| Unprepared seek/edit, plain | 30.020/32.520/32.624 | 23.817/24.605/25.091 | None |
| Prepared level edit, stress 1 | 29.234/31.634/32.434 | 19.844/20.446/20.613 | None |
| Prepared level edit, stress 2 | 29.933/32.536/32.649 | 19.645/20.501/20.577 | None |
| Prepared level edit, stress 3 | 30.634/32.126/32.231 | 19.817/20.758/20.789 | None |
| Speed edit, stressed | 29.936/32.336/32.436 | 24.171/25.010/25.246 | None |
| Unprepared seek/edit, stressed | 29.432/32.509/32.616 | 24.007/25.055/25.449 | None |
| All-ready speed control | 29.318/32.329/32.616 | 24.072/24.883/24.899 | None |
| All-ready seek control | 30.229/32.033/32.428 | 23.737/25.076/25.280 | None |

Sixteen distinct voices (no shared head in any native sample):

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain | 29.418/32.750/32.814 | 19.726/20.568/20.583 | None |
| Speed edit, plain | 30.516/32.406/32.606 | 27.531/28.173/28.833 | None |
| Unprepared seek/edit, plain | 30.017/32.416/32.614 | 27.509/29.437/29.934 | None |
| Speed edit, stress 1 | 30.532/32.337/32.628 | 27.342/27.813/27.826 | None |
| Speed edit, stress 2 | 29.837/31.734/32.137 | 27.468/27.903/27.932 | None |
| Speed edit, stress 3 | 30.336/32.237/32.737 | 27.053/28.945/29.555 | None |
| Unprepared seek/edit, stress 1 | 29.524/32.396/32.610 | 27.465/29.433/30.578 | None |
| Unprepared seek/edit, stress 2 | 29.424/31.928/32.520 | 27.479/29.221/29.292 | None |
| Unprepared seek/edit, stress 3 | 30.511/32.418/32.430 | 27.553/28.095/28.252 | None |
| Prepared level edit, stressed, 60 s loop | 30.836/32.622/32.624 | 19.787/20.671/20.699 | None |

All twenty paired rows meet today at p50, p95 and max.
Identical rows: 300 native starts, 300 with a shared head.
Distinct rows: 300 native starts, 0 with a shared head. In
all 600 native starts: voices at the target sample
[16], total omitted prefix 0 frames,
maximum start spread 0 frames, A3 safety-net
uses 0, program underruns 0,
callback sizes [64] frames.

The distinct project is the costly case: every voice runs the canonical
pre-roll from its grid start (11,520 samples plus up to one 16,384-sample
grid step). The margin to today is smaller than with identical voices but
positive at every percentile; the worst distinct maximum is
30.578 ms against today's
32.137–32.814 ms
range of maxima.

| 60 s loop window [4 s, 6 s) | Today underruns | Native underruns | Native starved renders | Native overloads |
| --- | ---: | ---: | ---: | ---: |
| final-plain | 0 | 0 | 0 | 0 |
| final-stress-1 | 0 | 0 | 0 | 0 |
| final-stress-2 | 0 | 0 | 0 | 0 |
| final-stress-3 | 0 | 0 | 0 | 0 |
| distinct-plain | 0 | 0 | 0 | 0 |
| distinct-loop-stress | 2 | 0 | 0 | 0 |

Today's WebAudio path counted 2
underrun events in the stressed distinct loop window; native counted none in
any window. This is reported as measured, not as a claim about today's
audible behavior.

Opt-in profiles (native only, twelve starts each, `PM_AUDIO_PROFILE=1`;
speed means exclude the first, unchanged-speed sample; summed worker scopes
overlap the producer wait):

| Mean ms | Distinct speed edit (n=11) | Distinct unprepared seek/edit (n=12) |
| --- | ---: | ---: |
| Producer wait for complete first output | 6.688 | 3.998 |
| Summed worker DSP | 57.270 | 30.840 |
|   initialization | 4.609 | 2.502 |
|   output seek | 5.953 | 3.128 |
|   processing | 46.690 | 25.201 |
| Summed map evaluation | 0.004 | 0.046 |
| Summed source reads | 0.405 | 0.208 |
| First mix | 0.044 | 0.041 |

Sixteen distinct jobs take about two job lengths: a headless probe with 14,
16 and 18 workers measured the same ~7 ms from Play to all-ready. That is
consistent with jobs beyond the twelve performance cores queueing or running
on the slower efficiency cores, but per-core placement was not profiled. The
pool size is unchanged.

#### Repeated suites, round 4 revision

The first campaign on ad3baecb failed its first stressed suite:
`a_decode_wait_below_the_bound_is_bit_exact` asserted
`blocked_decoders() > 0` at the turn its first starved render was counted,
but the render counts the miss before the decode worker it queued reaches
the test gate. 943b81fd waits for the worker to reach the gate (wall time
only; no device callback runs, so no buffered device frames are spent) and
changes nothing else; that failed log is kept beside the campaign. The
engine source and addons of 943b81fd are those of the paired captures.

On 943b81fd the first plain suite then failed the producer-timed bound
for a cold identical start (published 10.823 ms; applied-to-ready 9.611 ms
against about 3.7 ms unloaded) while the distinct cold test ran beside it on
the same pool and other lanes' foreground work kept the load average near
11. A locked six-suite check with the four sixteen-voice tests serialized
still had one cold identical start at 11.146 ms (ready-to-published
4.192 ms, the producer descheduled; load average about 13, other lanes'
node, dsymutil and corpus processes at niceness 0). 987ef700 therefore
serializes the four sixteen-voice tests (one shared pool) and asserts the
producer-timed bound only for prepared starts. Those logs are kept with the
campaign.

On 987ef700, after twelve plain and eleven stressed suites passed, stressed
suite 12 counted one ring underrun in the loaded time-mapped comparison
(`a remapped soundtrack: device ring underruns`, a 40 ms real-time device;
load average about 15). That assertion failed under machine load in rounds
1 and 2 too, but round 4 doubled the sixteen-voice tests whose bursts of
user-interactive pool jobs compete with another test's producer in the same
process, which no app does during its own playback. fae25da8 makes the
start tests hold a write lock that the two time-mapped comparisons share;
no assertion, patience or comparison changed.

On fae25da8 the frozen executable (40 tests: the 38 of round 3 plus the two
distinct tests) passed **30/30 plain and 30/30 stressed
suites** (2,400 executions) under `pm-electron-lock`,
with `PM_AUDIO_PLAYBACK_STRESS=1` for stress, `nice -n 15` and four test
threads. The mapped comparisons, patience, zero-underrun assertions and
bit-exact checks are unchanged.

Outside the lock, while another lane's Electron app and builds ran (load
average about 15), 5 of 10 stressed suites of the ad3baecb executable failed:
seven sixteen-voice starts published after the bound (11.301–25.475 ms; the
prepared ones too), and the existing real-time mapped comparisons counted one
or two ring underruns, and one forced-miss test missed its miss. Six stressed
suites of 943b81fd under the lock right afterwards all passed. The
producer-timed bound is therefore a real-time check: it holds on this
machine without a concurrent app, and fails, like the existing real-time
tests, when the test process (niceness 15) competes with that much
foreground load.

First 512 frames published, producer timestamps (p50/p95/max ms over all
starts in the sixty suites; the observer column is the latest elapsed time
at which the test thread saw the buffer):

| Suite, start | Starts | Published p50/p95/max | Observed max | Published over 10.667 ms |
| --- | ---: | ---: | ---: | ---: |
| plain, prepared level edit, identical | 120 | 0.373/2.383/4.443 | 16.283 | 0 |
| plain, cold seek/speed edit, identical | 120 | 4.186/6.489/7.986 | 20.500 | 0 |
| plain, prepared level edit, distinct | 120 | 0.267/0.381/0.543 | 5.471 | 0 |
| plain, cold seek/speed edit, distinct | 120 | 8.350/16.384/23.131 | 23.143 | 35 |
| stress, prepared level edit, identical | 120 | 0.367/2.424/5.320 | 15.411 | 0 |
| stress, cold seek/speed edit, identical | 120 | 4.195/7.193/8.026 | 21.276 | 0 |
| stress, prepared level edit, distinct | 120 | 0.269/0.494/0.610 | 3.718 | 0 |
| stress, cold seek/speed edit, distinct | 120 | 8.829/22.698/36.952 | 39.085 | 40 |

Prepared starts are asserted against the one-callback bound on the
producer's timestamp. Cold starts (identical and distinct) are reported
only: their latency is the paired app gate above. The round-3 observer readings above 10.667 ms (up to
14.046 ms) are consistent with observer delay: in these suites the observer
saw buffers up to the observed maxima above while the producer had published
them earlier.

Targeted checks on this revision: 65 library tests (one existing ignored
measurement), 11 decode-readiness, 14 mixer, 6 stretch, 5 sources, 2
layouts, 1 legacy-parity, 3 resampler and 37 presenter tests pass; strict
release clippy for pm-audio/pm-node all targets, formatting, the
default-profile pm-node check and the production addon build pass (test and
addon links use the existing local `CARGO_PROFILE_RELEASE_LTO=off`
workaround).

#### Remaining and orchestrator decisions

- **64-frame native I/O request (decision).** Round 3 reversed the earlier
  256-frame parity choice. This revision re-measured it: every paired native
  start used 64-frame callbacks and all six 60 s loop windows (four
  identical, two distinct, four of them stressed) had zero native underruns
  and overloads. Energy use, Bluetooth/USB/aggregate devices and loads such
  as concurrent GPU render or video decode remain unmeasured. It needs
  explicit approval.
- **Headless first-buffer bound (decision).** The check now uses the
  producer's publication time. It is enforced for prepared starts; for cold
  starts it is reported, because on this shared machine a niceness-15 test
  process beside other lanes' foreground work measured cold identical
  starts at 10.823 and 11.146 ms (locked) and up to 19.592 ms (unlocked),
  where the round-3 observer-based check would usually have passed by being
  late itself. In the final sixty suites no cold identical start exceeded it
  (240 starts, maximum 8.026 ms); cold distinct starts exceeded it in 75 of
  240 (maximum 36.952 ms under stress). Enforcing it for cold starts needs a
  quiet machine or a different bound; the orchestrator should choose.
- **Preparation pool load.** Up to fourteen workers at user-interactive QoS
  during edits and seeks; a continuous speed or map drag resubmits sixteen
  jobs per edit, and Signalsmith processing between map/read callbacks runs
  to completion after cancellation. Effect on UI and render responsiveness
  (D6 layer drag, S1 pan) is unmeasured.
- **Trim drags on stretched clips.** The map key now includes span and on
  states, so a live trim drag or on-toggle replaces that voice's history
  and costs a fresh canonical pre-roll on the producer per edit (where the
  base kept the run). Unmeasured.
- **Cross-platform priority.** Foreground QoS elevation is macOS-only. Linux
  workers stay at niceness 15 (an unprivileged thread cannot lower it again
  per job) and Windows has no priority handling. Start latency there is
  unmeasured (plan section 23).
- **Slow-history wait.** The producer no longer waits for unstarted jobs at
  an exhausted head, but a history a worker is running is still awaited
  without its own deadline. Its length is that job's CPU work (reads are
  nonblocking), so it is bounded in practice, not by a timer. No test runs a
  descheduled worker.

Raw captures, profiles, suite logs and the artifact manifest are outside Git
in `/tmp/ne-w5/audioa7-r4`.

### A8 round 5: installed histories leave their start (2026-10-18)

The round-4 review found that the producer claim path could put jobs back
on histories that were already playing. The trace: `use_prepared` installed
a partly ready start (a voice behind a complete shared head, its job still
queued, got the entry's run and a `Prefill` sharing the entry's completion
flag) and marked the entry installed. At the head's end the producer
claimed the queued history; the worker then found its job claimed, and
`Preparation::accept` returned early, so the voice stayed not ready.
`prepare_boundaries` kept the installed, incomplete entry and scheduled a
new job on the same live run with a fresh claim and the shared completion
flag cleared. The producer then waited on that queued job again (exactly
the saturated-pool wait the claim exists to avoid), the job rewound the
live run, and at the next wrap of a loop start `use_prepared` could hand
the advanced run back as prepared, so the producer re-primed it on the wrap
(the A7 mechanism). It is reachable in the measured scenario: the harness
sets the work area to [4 s, 6 s] and the speed and prepared-edit rows play
from 4 s. PCM stayed bit exact (the reviewer's probe); the defect was the
real-time stall and the duplicate producer work.

The same ownership gap had two more paths: paused or boundary preparation
(`prepare_start`) could claim a queued job of an installed start and run it
on the live run without ever setting its completion flag (the producer
would then wait at the head's end forever), and evicting or invalidating an
installed entry dropped the only path that set the flag.

**Fix (961230f6).** Ownership now transfers with the history:

- `use_prepared` releases every history it installs (ready, or behind a
  complete head with its job queued or running) from the prepared entry
  (`Prepared::release`). The entry keeps only histories no voice plays, so
  boundary scheduling, paused preparation and the next loop wrap prepare
  fresh histories and never touch a live run. The `installed` state and its
  retention rule are gone.
- Each job owns its completion flag and sets it when its history is
  released, whether it ran to ready, stopped on a missing block, or failed.
  A head's consumer no longer depends on the job's result reaching an entry.
  Each submission gets a fresh flag, so a superseded job still exiting can
  never mark a newer job finished.
- The A3 late path (`begin_prepared`) releases a head-installed history the
  same way (the slot is marked served, so no join job is scheduled on it),
  and the next pre-roll discards a superseded late start's entry.

Nothing in the render path, the stretch, the pre-roll or the ring changed.
No pm-node, protocol or render.rs change in this round (the StartTiming
fields below are serialized through the existing diagnostics report).

**Regressions (all fail with the fix removed, checked by mutation):**
`claimed_installed_histories_are_never_rescheduled` drives the reviewer's
trace through `Preparation`: partial install, claim at the head's end, the
workers' claimed reports through `accept`, then `retain_prepared` and
`schedule` at the same start. It asserts that the four rescheduled jobs run
on histories no voice plays, the producer never waits again, 16,384 frames
past the head stay bit exact while the jobs run on the pool, and the next
wrap installs fresh histories with no producer pre-roll (`fresh_starts` 1)
and 8,192 bit-exact frames. `a_dropped_start_entry_never_strands_an_installed_history`
invalidates the entry while a worker runs a history and checks the job's
own completion releases the head's consumer.
`paused_preparation_never_runs_a_queued_job_on_a_live_history` runs paused
preparation after a partial install and checks the producer can still claim
all three queued histories. The A3 test now also checks the late path
schedules no job on a head-installed voice. The reviewer's probe, run
against the fix, reports 4 rescheduled jobs, 0 on live runs, producer not
awaiting, and 0 mismatched chunks. One existing test,
`immutable_heads_start_complete_then_handoff_to_independent_histories`,
simulated a job that stops on an evicted block by reporting its result
without running it; since the job now owns its completion flag, the
simulation sets that flag as `run` does. Its assertions are unchanged.

**Pool QoS (433bf464).** Investigating a stressed paired failure (below)
showed a second, pre-existing start hazard. Idle preparation workers waited
at utility QoS and raised themselves only after dequeuing a playhead job.
Under default-class load a woken utility thread was starved for longer than
a start's 85 ms shared head, so the producer reached the head's end with
most dependent histories unstarted, claimed them all, and primed them
serially in one render; the ring ran dry. Workers now wait at
user-interactive QoS; only boundary refill lowers itself to utility QoS for
the length of its own job. `idle_workers_wait_at_user_interactive_qos`
reads an idle worker's class from outside the pool and fails with the old
policy. Linux and Windows are unchanged.

**Start diagnostics (a3ed6997).** `StartTiming` now also reports
`historyWaitUs`/`historyWaits` (segment holds while an installed head waited
on a worker's history), `historyClaims` (histories the producer primed
itself), `maxRenderUs` (the run's longest segment render) and
`minQueuedFrames` (the ring's low-water mark when the producer returned).
They are diagnostics only; no gate reads them.

#### Round 5 measurements

**The output device changed during this round.** Jude's default output was
AirPods Pro (Bluetooth) for most of the session: both engines ran
256-frame callbacks; native reported 160 ms device latency, today 5.333 ms
base plus 168 ms output latency. The test signal peaks at -123 dBFS, so
the captures were inaudible. Later rows ran on the MacBook Pro speakers
(64-frame native callbacks), as in rounds 3 and 4. Each JSON sample
records its device.

*Interrupted campaign on 961230f6 (AirPods, paired, 30 starts per row):*

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) | Raw misses |
| --- | ---: | ---: | --- |
| Prepared level edit, plain, 60 s loop | 173.595/175.312/175.388 | 168.606/171.672/171.972 | None |
| Speed edit, plain | 173.606/175.607/175.797 | 172.966/176.066/176.099 | p95 +0.459, max +0.302 |
| Unprepared seek/edit, plain | 173.489/175.794/175.874 | 173.172/175.405/175.438 | None |
| Prepared level edit, stress 1, 60 s loop | 172.911/175.510/175.895 | 169.899/171.499/171.832 | None |
| Prepared level edit, stress 2, 60 s loop | 173.212/175.911/175.992 | 168.368/171.601/171.701 | None |
| Prepared level edit, stress 3, 60 s loop | 173.593/175.566/175.882 | 169.690/171.790/172.090 | None |
| Speed edit, stressed | 173.201/175.687/175.909 | 174.397/178.460/179.397 | p50 +1.196, p95 +2.773, max +3.488 |
| Unprepared seek/edit, stressed | failed (below) | failed (below) | — |

All four 60 s loop windows: 0 underruns today, 0 native underruns, starved
renders and overloads. Every completed native start was complete and
simultaneous (16 voices, zero spread, omission and safety-net use).

**A8 on Bluetooth: speed edits miss today.** Native compute is unchanged
from round 4 (Play to all-ready on the plain speed edit: 3.657 ms p50 in
round 4, 3.645 ms in round 5). The device decides the margin. Per start, native
= frontend (about 1.7 ms) + canonical pre-roll (3.6–3.7 ms, more under
stress) + producer queue (0.1 ms) + wait for the next callback (0 to one
period) + callback-to-presentation lead (one period) + device latency.
With 64-frame callbacks a period is 1.333 ms and native's device latency is
13.7 ms below today's (15.625 against 24 + 5.333), which hides the
pre-roll. On AirPods a period is 5.333 ms and the device-latency advantage
is only 8 ms (160 against 168 + 5.333), so the pre-roll plus callback phase
shows at p95 and max. Breakdown p50/p95/max (ms), speed edit, plain: wait
for callback 2.576/5.141/5.159 on AirPods against 0.598/1.314/1.319 on the
speakers; lead 5.335 against 2.341. The baseline binary (41b499d7, round 4)
misses the same way on AirPods: stressed unprepared seek/edit 175.747/
180.880/182.480 ms native against 172.879/175.487/175.576 ms today. This
confirms the review's note: **the A8 pass depends on a short native
callback**, and it does not hold on a 256-frame device for edits that need
a fresh pre-roll on the Play path. Prepared level edits still pass there.
This needs an orchestrator decision; it is not fixable inside the bit-exact
pre-roll.

**Stressed seek: program underruns, pre-existing, located and reduced.**
The stressed unprepared seek/edit row on 961230f6 failed twice on AirPods
(one start with 5 and one with 3 program underruns; a first attempt had
failed on today's side with "WebAudio output timestamp unavailable"). A
native-only copy of the harness that records instead of throwing then ran
alternating rows (40 starts each, unprepared seek/edit, AirPods unless
noted):

| Binary, stress threads | Starts | Starts with program underruns | Starts with claims (histories) |
| --- | ---: | ---: | --- |
| 41b499d7 (round 4), 4 | 150 | 1 | n/a |
| 961230f6, 4 | 120, plus two paired attempts stopped at their first violation | 0, plus those 2 | n/a |
| a3ed6997 (diagnostics), 4 | 120 | 0 | 0 (0) |
| a3ed6997, 12 | 120 | 14 | 30 (251) |
| a3ed6997, 12, speakers | 120 | 7 | 28 (149) |
| 433bf464 (QoS), 12, speakers, alternating with the row above | 120 | 1 | 0 (0) |
| 433bf464, 12, speakers | 120 | 0 | 0 (0) |

Every violating start before the QoS change had 8–15 claimed histories;
where the trace had recorded it before the harness read it, the start's
longest render was 68–111 ms: the claim burst described above. After it, no
start claimed a history; the one remaining violation had no claim and a
46.9 ms render with the whole harness at niceness 15 beside twelve busy
threads at the same niceness (a descheduled producer). The round-4 binary
shows the same failure, so it predates this round; the round-5 history fix
does not cause it.

**Not completed: the final qualification of the final revision.** After
merging feat/native-engine (d5a59079, 132 commits, none in pm-audio,
pm-stretch or present/audio.rs), the final campaign passed the targeted
tests (69 library with one existing ignored measurement, 11
decode-readiness, 14 mixer, 6 stretch, 5 sources, 2 layouts, 1
legacy-parity, 3 resampler) and the 59 presenter tests, then stopped at the
40 GB disk guard (32 GB free, falling to 25 GB as other lanes built). Strict
clippy, formatting and the library tests also pass on 433bf464. Not run on
the final revision: the twenty paired rows, the six loop windows, the
profiles and the sixty playback suites. Rerun with
`/tmp/ne-w5/audioa7-r6/campaign.sh` once at least 40 GB is free.

**Correction to the round-4 report.** Its deliverables line said no
assertion was changed. 987ef700 did change one: cold starts are no longer
held to the one-callback first-buffer bound (10.667 ms) and are only
reported, as the round-4 section above states.

Evidence outside Git: `/tmp/ne-w5/audioa7-r5` (interrupted campaign,
A/B and diagnostic rows, breakdown scripts) and `/tmp/ne-w5/audioa7-r6`
(final-revision verify logs).

#### Remaining and orchestrator decisions, round 5

- **Short native callback (decision; A8 depends on it).** On 64-frame
  devices A8 passed in rounds 3–4. On a 256-frame Bluetooth device
  (AirPods Pro), speed edits miss today at p95/max plain and at every
  percentile stressed, in this round's and round 4's binaries. Options are
  the orchestrator's: accept per-device behavior, or find a non-bit-exact
  or speculative way to take the pre-roll off the Play path (both outside
  the current rules).
- **Final qualification pending** (disk guard; see above).
- **Claim burst remains possible.** With the QoS change the pool starts
  dependents promptly; if it is genuinely saturated, the producer still
  primes every unstarted history serially at the head's end. Bounding that
  (priming at most what the ring can cover) is not done.
- **Producer priority.** The producer is not a real-time thread; today's
  WebAudio renders on Chromium's real-time audio thread. Under equal-
  niceness load the producer can be descheduled (one 46.9 ms render at
  twelve busy threads). Unchanged.
- Earlier items stand: trim drags on stretched clips (map key), pool load
  during drags (D6/S1), Linux and Windows priorities, the running-history
  wait without a timer, and the headless cold-start bound.


### A7/A8: real-time scheduling and enforced cold-start timing (2026-10-19)

AUDIO-1 accepts the recorded AirPods Pro / 256-frame latency difference
(0.3–3.5 ms on about 175 ms). The earlier raw figures remain in the round-5
table. That decision does not accept dropped audio. Ordinary starts still
require all voices, complete and simultaneous, with no omitted prefix.

The producer and every preparation job request a callback-matched Mach time
constraint. Device playback explicitly constructs a schedule that attempts
the retained CoreAudio output-device workgroup
(`kAudioDevicePropertyIOThreadOSWorkgroup`). A device without a usable group,
and `AudioSchedule::new(..., None)` used outside headless playback, keep the
Mach-only fallback. Headless playback explicitly calls
`AudioSchedule::headless(...)`, which creates a dedicated AudioWorkInterval
with the public `AudioWorkIntervalCreate` API. Its producer and workers join
after installing the callback-matched Mach policy. A missing device ID alone
does not request a custom interval.

The headless producer alone owns interval cycles. An edit starts detached
preparation immediately; the first Play/Seek cycle is anchored to the
original request timestamp, including after producer guard re-entry. A
partial first callback keeps that cycle across retries, and publication
completes successive 512-frame output cycles. Finishing ring refill closes a
cycle before idle or metadata work; guard teardown closes an active cycle
before leaving the group. An expired or failed first-cycle start is recorded
for its epoch and is not retried with a newer timestamp. If the producer
cannot install the unique master, it leaves the custom group, restores its
prior scheduling policy, and reports an unavailable schedule; a worker's
membership cannot masquerade as a producer master. Failed creation, joins,
expired requests and lifecycle operations remain observable and fail
ordinary-start gates. Deliberate A3 decoder holds can finish a cycle after its
declared deadline; late finishes
are reported separately, while the atomic first-buffer assertion keeps its
original deadline. Workgroup membership is attempted only after real-time policy
succeeds. The per-thread guard leaves the group and restores the prior real-time
policy, or standard scheduling with an explicit idle Mach base priority of
at least 47 on these owned audio threads. Legacy Mach scheduling permanently
opts a pthread out of QoS, so restoring a cached QoS label is insufficient.
Restoration checks return codes and kernel priority; a bounded feedback
adjustment handles the task's niceness offset. Dedicated-thread and leased
preparation-worker tests verify the actual kernel policy and idle priority. The producer reserves real-time CPU for preroll and ring refill, and releases
it before full-ring boundary scheduling and read-ahead metadata, as well as
document/control work, paused preparation and device-property polling.
The optional master shuttle is warmed before receiving project controls. Edits still launch
preparation before Play. The producer retains its real-time guard across
control waits for at most 20 ms after a paused edit while urgent preparation is pending,
and while playing unless
boundary or look-ahead metadata is actually due. A complete batch becomes
visible under one queue lock before workers wake. Boundary and secondary-start jobs use the same
real-time schedule, with one nonurgent background replenishment job at a time. Installing a
head promotes its queued independent histories to urgent work and wakes
the pool, bypassing that serial background limit.

Preparation workers reserve 95% of a callback period and have a full-period
constraint. The producer retains that callback period with an 80% computation
allowance and an earlier 85% constraint, so a new control can take precedence
over older preparation jobs. Both are preemptible; exact role parameters are
read back from the kernel. The producer's earlier deadline is a candidate
until the measurements below qualify it. Cooperative checkpoints precede vocoder initialization, seek and processing,
voice mixing and bounded map/read-ahead groups. The existing Signalsmith
internal-step hook also checkpoints inside seek, process and flush without
changing DSP arithmetic or process-call boundaries. Scheduled shuttle calls
use it too. A cheap wall-clock gate limits expensive thread-CPU samples to
at most one per 100 µs (or one-sixteenth of a shorter callback period);
forced outer checkpoints still sample immediately. They leave up to 500 µs
(minimum of that and half the computation reservation) for the next unit,
then sleep to the next period when the CPU budget is spent. Known indivisible
stretcher construction/reset units require a 1 ms admission reserve. A new Play
does not reset or forgive CPU already charged in the same period. CPU debt is
retained across job guards and viewers sharing a worker, while CPU spent
outside those guards is excluded. Unknown callback size uses the 512-frame
headless period; an actual 64-frame callback keeps its 1.333 ms period. Per-context
telemetry records maximum CPU between sampled checkpoints (including the
coalescing window, rather than one vendor step), maximum period CPU,
and computation/constraint CPU exceedances against each role's actual limits; blocked wall time is not
reported as CPU service. Per-start counters expose throttle sleeps and
total sleep time. Scheduling entry counters include preparation before
Play; CPU maxima aggregate producer and workers rather than identifying a producer
per-period distribution. CPU maxima and restoration failures cover the current device context
and restart when that context changes. Actual audio delivery is checked separately by
program-underrun and A7 ring counters.

Public Mach latency and throughput tier-0 hints supplement the real-time
policy on owned audio threads. Entry reads back both hints, the requested
time-constraint parameters, and the effective non-timeshare mode. The hints
do not guarantee P-core placement. Their prior values are restored on exit;
readback, hint and restoration failures remain separate zero gates. Kernel
fail-safe demotion after entry is not continuously polled.

The schedule also captures the SDK's `os_workgroup_max_parallel_threads`
recommendation once at creation and exposes it as
`scheduling_workgroup_recommended_threads` in start telemetry. Zero means
no group or no recommendation. This advisory value is not used as the pool
cap and does not establish affinity or current available CPU capacity.

The producer's serial history-claim budget is zero. Independent vocoder
histories stay on preparation workers; the old synchronous paused/boundary
preparation helper is removed too. The producer only marks decoded plain
voices ready. On a heterogeneous Mac reporting named performance and efficiency clusters,
the preparation cap is the larger of eight workers and the performance-core
count minus two, within the total-core-count-minus-two ceiling (minimum one).
This host has 12 performance and 4 efficiency cores, so the candidate uses
10 workers. The tested 14-worker control and bounded launch-gate candidate
both missed all 32 strict cold-start deadlines; the extra concurrency and
launch gate are not retained as latency fixes. Neither worker cap provides
core affinity. Other hardware remains unqualified. Complete voice batches enter the pool under one queue lock
before workers wake, preventing early real-time workers from preempting
the producer while it is still submitting the remaining voices. Rewritten
regressions exercise the current worker-only handoff, released live-history
ownership and fresh loop histories through the real preparation pool.
Each control carries an atomic pending ticket through the existing FIFO and
any deferred slot until producer dispatch. Detached preparation workers alone
install a thread-local view of that counter. Existing forced and wall-gated
DSP checkpoints block once for 50 µs if a control is pending, allowing the
producer to run even when older real-time jobs have earlier deadlines.
The producer never installs this worker gate. Failed sends, shutdown and
queued-control destruction release tickets; no history is cancelled from the
sender thread. Per-worker opt-in profiles distinguish these control waits
from CPU-budget throttle sleeps. Aggregate control-yield count and wait time
appear in capture telemetry as both a per-start delta from its scheduling
baseline and explicit current-context totals. The context totals can include
preparation before Play and must not be attributed solely to that start.
Opt-in producer profiles record whether a real-time guard was already held
at each control receipt as a zero-duration event. The worker profile's
`host_claim_us` is captured at actual queue dequeue, before closure entry
(`host_start_us`) and its worker control yield; cancelled jobs also emit a
record. Profile-only worker lines are formatted outside any real-time guard
and sent with `try_send` to a process-wide ordinary stderr writer.
Its 2,048-entry channel never waits for capacity on a preparation worker;
full or disconnected sends increment a visible dropped-event counter. An
ordinary shutdown caller can request a flush barrier that waits at most
100 ms and records failure separately. The barrier drains records already
queued; a worker finishing afterward can enqueue a later record. This changes
diagnostic delivery, not DSP or the timing bound. V17 runtime probing is
pending, so no latency gain or complete profile capture is claimed.
Forced checkpoints each perform their own bounded
50 µs wait while controls remain pending. A queued control now interrupts
both preroll waits and is dispatched before any audio is filled. Same-epoch
retries preserve the original device-frame origin and phase wall deadlines,
so a nonrestart control cannot renew the 120 ms A3 hold. A new transport epoch
gets a new origin. The existing Seek→Pause regression also forces a blocked
decoder, asserting prompt acknowledgement, no queued PCM and the seek target. Canonical pre-roll, source evaluation,
per-voice processing order and offline PCM comparisons are unchanged.

The headless first-buffer test again enforces 512 frames within 10.667 ms
after Play for both prepared and cold starts, identical and distinct. Each
ring slot publishes PCM with Release ordering and records an atomic host
timestamp afterward. The consumer acquires that same publication and
records its observation time. The earlier available time is an upper bound
on visibility: neither a pre-publication timestamp nor a delayed observer
can turn late output into a pass. The old post-push trace remains diagnostic;
it no longer determines the deadline.

Final qualification must be recorded below against the active output device;
no qualification is implied by the scheduling code or compiler checks alone.
The AirPods latency acceptance does not apply to new program underruns.

Detailed profile DSP scopes include any cooperative throttle sleeps. Cost
comparisons use per-job CPU and throttle times rather than treating those
wall-clock scopes as pure DSP computation.

Runtime captures use the pinned Rust 1.96.1 toolchain at niceness 15 with
four build workers, release optimization, default thin LTO, debug level zero
and symbol stripping disabled. Disabling stripping avoids the pinned toolchain's
[Mach-O string-pool alignment bug](https://github.com/rust-lang/rust/issues/157750);
the native addons must be loaded and checked before final measurements.
A minimal adjacent `pm-media/build.rs` change links the native CrabbyAVIF
archive without bundling it into the Rust rlib. Bundling put native Rust
objects without LLVM bitcode inside that rlib and broke default thin-LTO
links. The codec archive and decoder implementation are unchanged; the
final verification includes the affected image tests and addon load.

#### Cold-start scheduling experiments (qualification pending)

These are preflight experiments, not final campaign results. The strict
atomic first-512 bound is 10.666667 ms. Each row ran eight fresh stressed
processes with four cold distinct-voice starts each, at niceness 15, with
two headless busy/yield threads. The device campaign separately uses four
busy threads. Percentiles use nearest rank; every raw miss is retained.

| Candidate | Deadline misses / 32 | p50 / p95 / max (ms) |
| --- | ---: | ---: |
| P-core-aware cap, 10 workers | 20 | 11.595 / 13.089 / 13.126 |
| Earlier producer constraint, 10 workers | 18 | 11.455 / 12.329 / 12.888 |
| Pending-control yields and paused urgency, 10 workers | 13 | 10.142 / 15.763 / 28.716 |
| Same control handoff, 14 workers | 32 | 14.962 / 17.856 / 18.145 |
| Bounded launch gate, 14 workers | 32 | 15.753 / 19.001 / 25.649 |
| Custom headless work interval, 10 workers | 13 | 10.122 / 12.678 / 13.288 |
| Interleaved control, 10 workers | 14 | 10.504 / 13.098 / 45.321 |
| Interleaved candidate, 8 workers | 27 | 12.180 / 14.263 / 14.659 |

All completed cold-start trials in these rows compared the first 8,192
frames against independently rendered offline voices, summed in voice
order, with exact PCM and zero omitted prefix, start spread, A3 use and
program underruns. Deadline misses still fail the tests. The 14-worker,
launch-gate and 8-worker candidates are rejected. The two worker-count
variants ran interleaved under one shared lease, with the 10-worker variant
first in every pair; that order limits causal comparisons. The final device
campaign counterbalances the engine order across paired rows.

The control-handoff change reduced request-to-apply time to 0.049–0.250 ms
in its measured cold starts. Remaining costs are chiefly canonical DSP
preparation and worker wake/claim handoff. Typical profiled jobs use about
3.6–4.5 ms of CPU; some take 5.3–5.5 ms. In a representative 10-worker
capture, the first ten claims occurred 0.64–1.44 ms after submission and
the second wave at 4.9–5.8 ms. These timings include contention and scheduling;
they do not isolate OS wake latency. The 45.321 ms outlier was profiled,
with claims continuing to 40 ms while job CPU stayed about 4.3–4.7 ms and
no budget throttle occurred in those jobs. Worker stderr serialization was
a possible contributor; the bounded asynchronous writer is awaiting runtime
validation. No outlier is classified as noise.

One custom-interval library preflight failed because an existing worker
priority test expected the Mach-only diagnostic label after the test began
using a custom interval. The assertion now expects the actual custom group;
the later 8-worker library run passed 86 tests with one existing ignored
measurement. The earlier producer/preroll control regression runs passed
plain and stressed, including Pause during a blocked seek, a hold governed
by device time, and the A3 missing-decoder-only safety net. Those runs do
not qualify the current source revision.

An isolated compiler scheduling experiment retained the same baseline ISA,
strict arithmetic and process boundaries, and changed only the CPU tuning
from the default to Apple M4. Its 201,600 output floats per variant were
identical over 1,680 callbacks, but 48 interleaved canonical-job CPU samples
were not faster: default p50/p95/max 2.84046/2.86537/2.87550 ms, tuned
2.84383/2.86821/2.87879 ms. That compiler change is rejected.

The atomic deadline remains a blocker. No final paired rows, six loop
windows, or 30 plain plus 30 stressed repetition suites have yet run on the
current revision. Earlier round-5 statements about ordinary producer
priority, report-only cold timing and unbounded serial history claims are
historical; the implementation above supersedes them, while qualification
remains pending.

### R7 worker wake policy and cold-start preflight (not qualification)

The preparation pool now has an explicit idle scheduling lease. The caller
arms workers before returning a newly constructed playback; Play does not wait
for an arm command to reach the producer. A parked worker holds a verified
Mach-only reservation matching the output callback. It drops that guard before
queue locking, readiness evaluation, job dispatch, or diagnostics, and joins
the actual playback workgroup and reservation for DSP work. Idle contexts do
not retain the output workgroup. A revoked lease shares a tombstone with its
clones. Different simultaneous callback periods select ordinary idle waiting;
actual preparation still uses each job's own callback-matched policy. This
fallback has no multi-viewer cold-start latency qualification.

A wake generation and waiting flag use a sequentially consistent handshake:
a publisher either sees and wakes a parked worker, or the worker observes the
new generation and retries its queue scan. Wake diagnostics expose successful
idle entries, wake count, guard-drop time, queue-lock wait, and wake-to-claim.
The current lease and construction arm result are exported alongside process
pool counters. Counter deltas since Play are separate from lifetime peaks;
in particular, an idle context's period CPU peak includes work inherited from
the preceding job in the same physical thread's reservation window.

A physical thread keeps CPU debt when changing jobs, playback contexts, or
idle contexts with the same policy. Overrun flags belong to that reservation
window, so inherited debt is not charged as a new idle overrun. When the
numeric policy changes, the worker waits for the previous genuine period to
end before installing a fresh reservation. This wait holds no queue mutex,
TLS borrow, or real-time guard. Scheduling metrics retain those waits. The
first headless interval also retains the actual SDK anchor, start, and deadline
ticks; reentry is tied to the original Play request, not a renewed deadline.

V19 kept ten preparation workers, the canonical 11,520-frame pre-roll, and the
unchanged strict atomic first-512 bound of 10.666667 ms. Its eight stressed
cold-distinct preflight processes produced 32 complete bit-exact comparisons:
30 met the deadline and two missed at 10.946 and 12.228 ms. The distribution
was 8.644/10.946/12.228 ms (p50/p95/max). All 32 had zero omitted prefix,
zero start spread, zero A3 fires, and zero program underruns. The profiled
10.946 ms miss had all first ten claims within 64 microseconds; its last two
voices claimed about 5.7 ms later and each incurred roughly 0.5 ms of CPU-budget
throttle. Aggregate preparation CPU was 77.09 ms. These are raw misses.

The V19 library preflight passed 100 tests, failed one new policy-observation
test, and retained one existing ignored test. That new test incorrectly
assumed a real-time thread had no readable priority; the supported extended
query reported priority 97. The correction verifies exact Mach parameters and
the effective scheduling mode separately. Ten targeted seek, Pause, decoder,
and A3 regressions passed (five plain and five stressed). The complete playback
preflight passed 40/40 plain and 39/40 stressed; the stressed cold-distinct
case had two further deadline misses, 11.952 and 11.578 ms. Their complete
PCM, omission, spread, A3, and underrun checks still passed.

Timing suite processes now run one test at a time, within the brief's limit
of four, because different simulated device periods share the pool. All 40
public tests remain and all assertions are retained. Library checks still use
up to four test threads. Device pairs retain four busy stress threads. Earlier
four-test-thread failures remain evidence; a serial preflight is not the final
30 plain plus 30 stressed campaign.

Earlier logger-only V17 and idle-reservation V18 probes also failed the
same strict deadline. V17 passed 2/32 trials, with p50/p95/max
11.867/14.178/14.651 ms; V18 passed 12/32 at 11.023/12.519/13.069 ms.
V17's library passed 87 tests with one existing ignore. V18 passed 93,
failed three new scheduling/wake tests, and retained that ignore. Two failures
came from invalid 100 ms synthetic reservation expectations; the third
exposed a park-token handshake defect that was fixed before V19. Both V17
and V18 full preflights passed 37/40 plain and 37/40 stressed, with two
critical-vector indexing panics and the cold deadline failure in each mode.
The vector remap defect was fixed before V19. These failures remain recorded.

### R7 fair preparation and ownership preflight

A prepared voice now has one owned task through initialization, deterministic
join, optional continuous lead, and immutable head publication. The task keeps
its evaluator, retiming memo, constant map, scratch, and vocoder history when it
moves between workers. A CPU yield is distinct from missing decoded input and
from completion. Yielding happens only after a complete native process call
(or completed initialization), after its output and phase state have committed;
it never changes native call boundaries, canonical pre-roll, map arithmetic,
or output length. A request that has reached its end finishes copying and
pruning before it can yield.

A yielded task releases its history mutex and real-time guard, then returns to
the ordinary queue at the tail of its priority band. It remains pending, so no
duplicate job may claim that history. Only terminal completion sends Done and
sets finished; only a complete head becomes visible. Cancellation of a queued
continuation releases its dependent gate and completion ownership once. The
thread's CPU ledger survives both task migration and policy reentry. There is
no new CPU allowance per slice.

Per-voice profiling sums scopes, thread CPU, entry time, control waits, and
throttle across workers, and records first claim, intervening queue wait,
slices, requeues, and maximum slice CPU. Pool counters expose claims, requeues,
completions, wakes, and idle entries; these are process-wide and the exported
per-Play view subtracts counters, while maxima remain lifetime observations.
A completion only wakes sleepers when ready work exists; a requeue wakes one.

The exactness regressions compare every f32 bit and the native read-call
sequence at five speeds in both directions, plus a resumable join and actual
missing-input retry. Mixer checks compare through the head and 8,192 further
frames against both the unsliced path and a fresh offline render. Queue tests
use the production loop for FIFO rotation and the scan-to-park race. The
64-frame policy check still requires exactly seven logical jobs; its entry
count includes every slice. Construction arming keeps an acknowledgment for
the lease epoch; incompatible-period transitions are counted and gated for
each start. Actual output periods, not project sample rates, select the lease.

V20's eight stressed cold-start processes passed all 32 strict first-buffer
assertions: p50/p95/max 8.405/9.563/9.733 ms. V21 passed all 32 at
8.871/9.823/9.826 ms. Every trial compared complete PCM bit for bit, with
zero omitted prefix, start spread, A3 fires, or program underruns. V20's
library passed 106 tests; V21 passed 110, each with one existing ignored
measurement. Each candidate passed ten targeted regressions and one complete
40-test plain and stressed playback suite. These candidates predate the
latest integration merge and are preflight evidence, not qualification.

In V20's 16 fully logged cold preparation cohorts, all 16 voices completed
in 304 slices and 288 requeues per cohort, with zero budget throttles. First
claim to last completion was 6.842/8.379/9.285 ms (min/p50/max); summed
thread CPU was 60.059/71.292/77.015 ms and summed policy-entry time was
2.620/3.197/3.622 ms. Entry totals span parallel workers; they are not wall
time. Profile scopes overlap and include checkpoint waits. The raw scope,
queue, slice, entry and wake measurements are retained outside Git.

Independent review found two ownership races introduced or exposed by
slicing. A project edit can cancel a worker while it waits for a history
mutex, so cancellation is checked again after locking. Solo can transfer
history ownership without changing the project generation. A worker now
checks the finished ownership flag before initialization and after locking,
then retires silently without rewriting the head, completion flag, or live
history. Tests force both lock-wait interleavings and compare subsequent PCM
with a fresh offline render. Queued and mid-slice Solo retirement, dynamic
speed evaluation, missing-input recovery, and two mixer-level loop passes
also have exactness regressions.

The final merged revision and full device/repetition campaign remain
unqualified at this point. No historical miss is classified as noise.

### R7 callback preparation follow-up (qualification pending)

V22, after the integration merge, passed 32/32 stressed cold first-512
preflights at 7.351/7.957/8.899 ms (p50/p95/max). Each trial retained complete,
bit-exact PCM, 16 simultaneous voices, zero omitted prefix, zero A3 fires,
and zero program underruns. Its library passed 113 tests with one existing
ignore; ten targeted regressions and one 40-test plain and stressed playback
preflight passed. These results did not qualify the device path.

The subsequent partial device campaign used the built-in output at 48 kHz
with 64-frame callbacks and four busy stress threads. The following raw A8
rows remained blockers. Each entry is p50/p95/max in milliseconds, with 30
starts per engine. The third shared seek repeat passed; it does not erase
the other two misses.

| Scenario | Today | Native |
| --- | --- | --- |
| Shared unprepared seek, stressed repeat 1 | 29.512 / 32.507 / 32.511 | 32.249 / 35.243 / 36.027 |
| Shared unprepared seek, stressed repeat 2 | 29.218 / 32.408 / 32.513 | 33.311 / 35.451 / 35.653 |
| Shared unprepared seek, stressed repeat 3 | 29.627 / 32.212 / 32.295 | 28.631 / 30.344 / 30.614 |
| Distinct speed edit, plain | 29.931 / 32.321 / 32.530 | 30.817 / 31.349 / 31.372 |
| Distinct unprepared seek, plain | 30.222 / 32.414 / 32.611 | 30.879 / 32.112 / 32.200 |
| Distinct speed edit, stressed repeat 1 | 30.436 / 32.329 / 32.429 | 30.648 / 31.267 / 31.305 |

A separate stressed speed attempt recorded one computation-budget overrun:
maximum guarded period CPU was 1.309376 ms, and maximum sampled work-unit CPU
was 0.873375 ms. There was no constraint overrun, omitted audio, start spread,
or program underrun. The later retry passed; the first failure remains. The
old record did not identify the offending phase or its time within the
120 ms observation window. It cannot be attributed to first-buffer work.
Five completed 60-second loop pairs, including distinct voices, had zero
underruns in either engine and zero native starvation or device overloads.
The blocked revision's 60 repetition suites were not started. Three earlier
setup attempts also failed before playback measurement; fresh native, Wasm,
and desktop artifacts subsequently connected and played. Their precise API
failure was not isolated, and they do not count as passing captures.

Matched first-head profiles then captured two plain and two stressed native
rows, 12 starts each, on the same 64-frame output. The representative job used
19–21 slices. Across rows, median queue time was 4.026–4.373 ms, thread CPU
3.391–3.595 ms, policy entry 0.175–0.188 ms, and budget sleep 1.668–2.454 ms.
Cached input reads took about 0.027–0.029 ms; no inline decoder CPU was
needed for these already decoded sources. DSP scopes include checkpoint
sleeps and overlap the CPU and wait measures. Job logging followed head
publication by a median 0.024 ms and as much as 1.572 ms, so these full-job
totals are not all necessarily on the first-buffer critical path. They
identify queue handoff as a candidate cost, rather than proving an avoidable
four-millisecond delay.

The revised executor keeps a sole ready foreground priority-zero job on its
current worker for one more complete native DSP slice. It checks the ordinary
queue again before every continuation. An equally urgent ready competitor
puts the retained job at the FIFO tail. Each slice still releases its history
mutex and real-time guard, then reenters and verifies the actual scheduling
policy. The physical thread's CPU debt remains; retaining a worker can also
retain its throttle, so the latency benefit requires measurement. Profiling
separates CPU yields from actual queue reentries and records worker handoffs
and local continuations. HEAD remains 4096 frames, LEAD 1024, and canonical
pre-roll/grid remain 11,520/16,384. No voice is omitted to improve latency.

Pure control-point scans and resampling loops now reach cooperative checks
every 32 points and 64 output samples respectively, preserving arithmetic and
native call boundaries. A fixed atomic record preserves the first observed
computation-budget overrun: host time, role, phase, unit/prior/period CPU,
wall elapsed, and exact limits. Publication is Release/Acquire and performs
no allocation, lock, or logging on the audio thread. This is a schedule-lifetime
record, not a per-Play counter; a simultaneous observer can briefly see an
overrun count before its record is published. Phase changes occur after
sampling the preceding unit; internal DSP checks keep the current label.
Unlabeled work reports unknown. Small gain/mixing tails may remain within
the preceding voice phase until the next checkpoint; phase names are not
exclusive function-level CPU attribution.

V24 then completed four stressed paired preflights, with four busy threads
and 30 starts per engine on the built-in 48 kHz, 64-frame output. Shared
voices passed; distinct voices still missed the stated latency target.

| Scenario | Today p50/p95/max ms | V24 p50/p95/max ms |
| --- | --- | --- |
| Shared unprepared seek | 29.829 / 32.413 / 32.630 | 29.006 / 30.284 / 30.928 |
| Shared speed edit | 30.830 / 32.436 / 32.729 | 26.947 / 27.965 / 28.052 |
| Distinct unprepared seek | 30.432 / 31.923 / 32.528 | 30.590 / 32.116 / 32.296 |
| Distinct speed edit | 30.428 / 32.431 / 32.635 | 30.844 / 31.308 / 31.343 |

The distinct seek p50/p95 misses, and distinct speed p50 miss, remain
blockers. No miss is classified as noise. Across those 120 native starts,
12 shared-head profile starts and 24 distinct profile starts, the observed
program-underrun, omitted-prefix, start-spread, A3, producer history-claim,
and computation/constraint-overrun counters were zero. Application-lifetime
underrun counters were nonzero and remain in the raw reports; zero program
counters do not establish a zero application-lifetime count. These captures
do not replace the full repetition or 60-second loop campaign.

The shared profile showed that retaining a worker helps only a sole ready
head. One of 12 heads had 20 local continuations and 0.019 ms of queue time;
the other 11 competed with ready priority-zero peers and requeued 17–19
times. The median queue time remained 4.034 ms. The queue delay is largely
waiting for other urgent jobs, rather than a measured four milliseconds
inside the queue mutex.

Distinct profiling matched all 12 seek cohorts and 11 of 12 speed cohorts;
one speed start was already ready and had no contemporaneous cohort.
Seek applied-to-ready was 10.711/11.417 ms (p50/p95), with 56.485 ms summed
CPU across the 16 jobs. The last completing job's medians were 3.974 ms CPU,
3.495 ms queue, 3.027 ms budget sleep and 0.171 ms policy entry. Speed
applied-to-ready was 12.957/15.516 ms, with 78.968 ms summed CPU; the last
job's medians were 4.905 ms CPU, 4.962 ms queue, 3.851 ms budget sleep and
0.211 ms entry. Sleep overlaps entry/work wall time. Last job logs preceded
ready by 2–7 microseconds; they are correlated completion observations, not
the publication event itself. These profiled native rows were unpaired
and do not establish a paired latency pass.

A 512-frame output lead was investigated and rejected as the next fix.
The native interval is 1,440 frames at 48 kHz: the measured 2/3/4-second
anchors need the same native process calls for a 512- and 1,024-frame lead.
Other phases could lose useful reserve before a serial producer fill.
HEAD 4096, LEAD 1024 and canonical pre-roll/grid remain unchanged.

The instrumentation patch is measurement only: fixed atomic maxima and an observation
count record thread-CPU spans after admitted DSP checkpoints, including
the fraction of each span's actual computation budget. Such spans can
include intervening work; they are upper bounds rather than isolated vendor
call timers. No reservation, callback limit, CPU debt, or PCM change has been
made. Build, strict Clippy, addons and nine targeted pure regressions passed;
independent source review found no P1/P2 blockers. The values are
schedule-lifetime snapshots across producer
and worker roles; their separate maxima may come from different events.
Finite observed spans cannot establish a hard bound for a smaller reserve.

Its device capture then completed 55 of 84 planned native starts. The speed
row aborted on two worker computation-budget overruns. The first record
identified `stretch-init`: 1.276500 ms of unit CPU plus 0.004250 ms already
spent, against a 1.266666 ms computation allowance and 1.333333 ms constraint.
That record was already present before the first measured Play. Constraint
and program-underrun counters were zero; the strict computation gate still
failed. A larger reserve cannot make this indivisible unit fit its allowance.
Initialization needs finer checkpoints rather than a relaxed CPU limit.
Observed mixed-role DSP-span maxima were 0.132–0.227 ms across these rows;
they do not establish an initialization bound. All 55 observed starts kept
16 complete voices together, without an omitted prefix or A3 fallback.

The unprofiled seek row retained a 1,401.582 ms first-start outlier. Of that,
1,362.958 ms was between the UI request timestamp and the engine request;
apply-to-ready took 19.364 ms. The cause of the delivery delay was not
isolated. These native-only captures have no paired latency verdict, and
neither the outlier nor the overruns are classified as noise. Nonzero
application-lifetime underrun counters remain recorded separately.

The earlier V24 immutable headless preflight also completed: 119 library
tests passed, one was ignored, ten start regressions passed, and 32 cold
atomic first-512-frame observations had a 9.286 ms maximum, below the
10.666667 ms bound. Those results do not qualify the newer instrumentation
or the final revision.

The next candidate adds opt-in initialization checkpoints through FFT
twiddle construction, Kaiser and reconstruction loops, STFT reset, and
between allocation stages. The ordinary constructor/reset remains the
offline reference. The callback-enabled path also skips an initial ACG
window and reset that the immediately following Kaiser setup and final
reset overwrite before use. Final dimensions, buffers, seed and arithmetic
order remain unchanged; direct seeded PCM comparisons cover repeated
resets, 44.1/48 kHz and the random-phase slow-stretch path. Native reset
constructs its replacement before swapping, preserving live state on an
allocation failure. Allocations themselves remain indivisible and require
the strict actual-device CPU gate.

Preparation turns allow two resumable fresh/produce operations, with the
quota carried across stage and join-range transitions. A class-changing
operation can process an outgoing run and seek the incoming one; this is
not a two-FFI-call bound. Independent test counters check actual DSP calls
alongside existing exact PCM and history-handoff comparisons. The FIFO
handoff, continuous CPU checkpoints and physical debt remain intact, as do
the 500-microsecond ordinary reserve, initialization reserve and actual
callback limits.

V26 passed its build and strict Clippy checks, 119 audio library tests (one
ignored), two stretch library tests and five binding tests. Its runtime
preflight failed: 29 of 32 cold first-512 observations exceeded the enforced
10.666667 ms bound; p50/p95/max were 13.120/17.236/20.460 ms. On the built-in
48 kHz, 64-frame output, paired stressed distinct seek measured
42.129/44.837/44.967 ms against Today 29.194/32.203/32.495 ms; distinct speed
measured 39.905/42.853/44.058 ms against 29.973/32.012/32.307 ms. All 60
native starts in those two rows retained complete, simultaneous voices and
zero program-underrun and computation/constraint-overrun counters. The
latency and headless-bound failures still block qualification.

Independent review found setup checks in two shared per-block helpers had
prevented vectorization and bulk stores, affecting processing as well as
initialization. The remaining diagnostic runs were canceled and all
completed failures retained. V27 removes those checks and dispatches STFT
reset once between cooperative setup and an unhooked template path, keeping
ordinary processing clears and arithmetic available to the optimizer. A
stereo seeded repeated-reset comparison supplements the mono coverage.
V27 audio/stretch compilation, strict Clippy, addons and matching app/Wasm
builds passed. Disassembly restores the same helper mnemonic sequences as
the earlier vectorized object: 54 NEON instructions in window-product
arithmetic and six bulk-clear calls in output movement. The ordinary reset
specialization has bulk clears and no per-element indirect callbacks. This
verifies generated code, rather than a runtime or worst-case CPU bound.
V27 runtime qualification is pending. Its first paired attempt retained one
partial 2,063.992 ms native observation, then failed because first-buffer
timing was missing. The selected pre-edit audio report had no device or
project and zero callbacks, so this does not establish device-output timing.
The Today launch separately timed out waiting for its first window. Neither
part supplies a paired latency verdict. One subsequent unloaded startup
probe retained all 16 layers and opened the device within its 300 ms warmup,
without a host-clock or silent run; it did not reproduce or explain the
first failures. Capture setup needs explicit document, session and audio
readiness before the measured edits.
The sampled DSP-span telemetry now includes initialization and destructor
tails; its maxima are not isolated vendor-call times or directly comparable
to V25. Allocation and destruction can still be indivisible.

Full final device and repetition qualification remains pending. The
historical AirPods 256-frame allowance above remains limited to that
accepted device result and does not excuse the built-in-output misses.

V27's later reserved headless preflight passed 121 library tests (one
ignored), two stretch library tests, six binding tests and ten focused
start regressions. Four of its 32 cold atomic observations still exceeded
10.666667 ms; visible first-buffer p50/p95/max were
7.957/11.713/12.755 ms, with a 13.414 ms observer maximum. A separate seeded
probe compared the earlier vectorized object, V27's ordinary path and its
cooperative path: all 1,167,696 floats across eight mono/stereo, rate,
slow-stretch and repeated-reset cases were byte-identical. This supports
PCM preservation on this Mac; final mapped playback comparisons remain
required.

V28 increased the preparation pool from ten to twelve workers on the
16-core Mac (twelve performance and four efficiency cores), still below
total cores minus two. Its reserved preflight passed all 21 processes,
including the same library, binding and focused regressions and all 32
cold atomic observations. First-buffer p50/p95/max were
6.837/8.323/8.466 ms; the observer maximum was also 8.466 ms.

Four paired stressed rows then completed, with four busy threads and 30
starts per engine on MacBook Pro Speakers, 48 kHz and 64 frames. Both
halves of every pair held the exclusive app lease. Recorded one-minute
load means did not change sharply under the preregistered validity rule;
no pair was discarded or retried. Shared rows passed; distinct rows failed.

| Scenario | Today p50/p95/max ms | V28 p50/p95/max ms |
| --- | --- | --- |
| Distinct unprepared seek | 30.128 / 32.207 / 32.317 | 31.189 / 32.886 / 34.154 |
| Distinct speed edit | 30.133 / 32.135 / 32.536 | 32.388 / 34.926 / 35.194 |
| Shared unprepared seek | 30.222 / 32.603 / 32.711 | 29.474 / 31.369 / 31.506 |
| Shared speed edit | 30.034 / 32.528 / 32.635 | 28.749 / 29.790 / 29.834 |

The two distinct rows miss all three required latency statistics. No miss
is classified as noise. Those 120 native starts and 24 additional native
profile starts kept all 16 voices complete and simultaneous, with zero
program underruns, omitted prefix, start spread, A3 fallback, producer
history claims, and computation/constraint overruns. Application-lifetime
underruns are separate counters and are not claimed to be zero.

Profile cohorts matched all twelve seek starts and eleven cold speed
starts; the remaining speed start was already ready. Median apply-to-ready
was 12.170 ms for seek and 10.157 ms for speed. The final initial voice
claim arrived about 1.7 ms after enqueue. The last completing voice used
4.979/4.378 ms CPU, 2.991/2.608 ms queue time and 4.004/3.364 ms budget
sleep, respectively. These scopes overlap and must not be added as
independent wall time. Across the unprofiled rows, median UI-to-engine
request was 1.117 ms for seek and 2.178 ms for speed; request-to-apply was
only 0.022/0.018 ms. Dirty-project graph construction precedes that engine
request timestamp and needs its own control-thread measurement. Full
final qualification remains pending.

The resumed validation replaces test media paths with owned temporary-directory
handles throughout pm-audio and pm-media, including the shared playback
fixture, unit fixtures and examples. Fixture teardown joins decoder workers
before deleting media; panic unwinding releases the directory as well. The
corpus app harness removes profiles when setup or launch fails, and always
removes them after close. The catalog's temporary rewritten module is also
removed immediately after import.

The campaign now gives its children an isolated `TMPDIR` and compares file,
directory and byte counts against its empty starting state, even after a
child failure. Any growth fails the campaign; the outer directory guard then
removes its own tree. The guard's regression checks cover a successful child,
an exception exit and a deliberately leaking child. A failed-extension launch
also returns to an empty temporary tree. This prevents repeated playback
runs from accumulating generated media in the host temporary directory.

Presenter fixtures used by the audio campaign also retain their temporary
Blender area until both the host and worker threads have stopped. The audio
presenter test joins decoder workers during panic unwinding.

The guard uses a short owned directory and sets both `TMPDIR` and
`MAC_CHROMIUM_TMPDIR` for generated test media, harness profiles and
Chromium temporary files without exceeding macOS's socket-path limit.
Frameworks using `NSTemporaryDirectory()` can ignore these variables;
the guard only measures its owned tree. Actual Chromium cleanup remains
subject to the device smoke and final campaign checks. It checks for growth
after each paired app, before reserving, before the suites and at campaign exit.
The report is published as running before launch and must bracket the final
manifest's timestamps to qualify. Signal regressions verify that SIGTERM
and SIGINT join the child and remove the owned tree, including a cooperative
child that joins a grandchild in a separate session. The app and bridge use
one shared close operation for normal close, signals and disconnect; the
outer owners wait for teardown before removing profiles or releasing leases.
The first resumed Electron smoke exposed an empty driver artifacts directory
after a completed Today loop. Driver artifacts now live inside the owned app
profile, so their deletion is awaited with profile teardown. The failed smoke
is retained as a cleanup failure and supplies no native qualification.
An uncatchable process
kill still relies on the reservation's fixed expiry. The exclusive paired
measurement phase releases its reservation before the sixty playback-suite
runs, which each acquire an ordinary lease for their own duration.

The next diagnostic completed seven rows on AirPods Pro, 48 kHz with
256-frame native callbacks, under four stress threads. This candidate is
unqualified. The paired latency results were:

| Case | Today p50/p95/max (ms) | Native p50/p95/max (ms) |
| --- | --- | --- |
| Shared prepared level edit, 60 s loop | 173.286 / 175.887 / 176.096 | 169.881 / 171.948 / 172.448 |
| Distinct unprepared seek | 173.654 / 175.571 / 175.857 | 182.588 / 187.121 / 188.054 |
| Shared unprepared seek | 172.962 / 175.584 / 175.675 | 178.495 / 182.761 / 183.661 |
| Shared speed edit | 172.783 / 175.676 / 175.775 | 178.568 / 180.801 / 186.334 |

The distinct speed row failed final validation after its cold warm Play
waited 120.243 ms and invoked A3, omitting 1,280 frames per voice. Its thirty
later timed starts had zero start-program, real-time, omission, spread and
A3 counters, but that does not excuse the initial dropout. Its retained
partial native samples were 180.755 / 184.555 / 185.211 ms against Today's
173.384 / 175.879 / 175.982 ms; these are raw figures, not a validated row.
One shared-seek
pair was discarded under the fixed sharply unequal load criterion; its raw
results remain retained, along with the replacement pair's latency misses.
These new misses exceed the historical AirPods allowance and are blockers.
Both engines passed the per-app temporary-tree checks, and the failed
campaign's outer guard removed its empty owned tree.

Control profiles matched twelve seek and twelve speed Play receipts.
Median UI-to-control-receipt was 9.605 / 9.014 ms, respectively. Median graph
build-and-send was only 0.160 ms for the ten dirty seek starts; the speed
starts already had their edited projects installed. Separate synchronous
Pause and Exact Seek acknowledgements precede Play, each potentially
crossing a device period. Removing those waits must preserve producer FIFO
order and individual errors. Its effect on output latency needs measurement:
paused preparation can overlap the existing acknowledgement waits.

The unchanged runtime binary also passed one plain and one stressed
40-test headless smoke, with six exact mapped PCM comparisons in each and
zero temporary growth. Cold first-buffer publication maxima were 7.820 /
8.143 ms, below the enforced 10.666667 ms bound. Three observer polls were
later: 10.940 / 11.411 ms plain and 10.700 ms stressed. These remain recorded
separately. The deadline uses the earlier of the producer's post-release
timestamp and the observer's post-acquire timestamp, both valid upper
bounds on actual publication. The final thirty-plus-thirty campaign remains
required.

Simple transport controls now enqueue Pause, SetLoop and Exact Seek without
separate control-thread acknowledgement waits. The producer still applies
every command in FIFO order. Play keeps its synchronous batch acknowledgement
as a barrier; each earlier command retains its own result. Receipts expire
five seconds after enqueue and the pending queue is capped at 64. Scrub and
PlayAround remain synchronous. A pending Pause suppresses running meter
updates, and repeated identical transport errors do not displace other
problems. Device latency improvement remains unmeasured on this revision.
An opt-in send stamp separates frontend delivery from time waiting in the
audio control queue; it is logged after enqueue on the ordinary presenter
thread.

With `PM_AUDIO_PROFILE`, source-block events record source/block IDs, enqueue
and decode thread names, original request urgency, last duplicate request,
worker claim, decoder-lock acquisition, seek/read completion, cache publication
and thread CPU time. The publication stamp follows cache-lock release and is
an upper bound. Synchronous wait duplicates count once per caller; an inline
decode taking a queued block is identified separately. Events use the bounded
diagnostic writer. Profile-only map entries, thread-name copies and clock
reads add probe overhead under the cache lock; ordinary decoding keeps its
existing order and priority.

The V36 diagnostic on AirPods Pro at 48 kHz/256 frames remained unqualified.
Its 12 profiled speed starts measured 181.821/191.455/191.455 ms
(p50/p95/max); 30 unprofiled distinct seek starts measured
183.545/189.445/191.112 ms. All 42 timed starts reported zero program
underruns, omitted prefix, start spread, A3 fires and RT overruns. These are
native-only observations: the queued Today half was stopped before it ran,
so they establish no paired latency pass. The seek part also failed the
harness's outdated two-slot ownership check after the machine gained a third
slot. The external harness now reads the official configured slot list and
requires every slot for an exclusive reservation. Separately, the profiled
seek warm Play was paused 125 microseconds after producer Play; no timed
samples followed. Opt-in presenter traces now identify explicit, pending-route
and fallback Pause origins and deferred replay, without changing transport.

For the 12 profiled speed starts, median UI-to-control receipt fell to
1.880 ms, with UI-to-send 0.871 ms and send-to-receive 0.113 ms; tail delivery
still reached 11.284 ms. One truncated diagnostic line retains its timestamp
fields but has unknown dirty/opened flags. The cold source block took
78.723 ms from enqueue to publication: queue 2.292 ms, decoder-lock scope
11.878 ms and read scope 64.171 ms, versus 0.669 ms of decoder thread CPU.
During that wait, 1,913 fully enclosed missing-input preparation attempts
consumed 829.610 ms of aggregate CPU across concurrent workers. The seek
comparator published its first block in 4.850 ms. These data support retry
work starving ordinary decoding; off-CPU wall time can also include I/O and
page faults, so the trace does not attribute all of it to scheduling.
Both receipt regression tests and the node control/shutdown regression passed
under ordinary leases, with their owned temporary trees empty and removed.

Preparation now gates a failed missing-input attempt on the source cache's
publication epoch. Both decoded blocks and memoized decode errors advance
it; duplicate reads do not. A job samples the epoch before its step, so a
publication before its completion message cannot leave the voice asleep.
The producer retains the gate only while the same prepared history still
exists, and clears gates on graph invalidation. This bounds CPU retries by
cache progress rather than the preroll polling rate. Decoder threads remain
ordinary, and the 120 ms A3 bound, canonical preroll, lead and head sizes
are unchanged. New regressions hold a real decoder while repeatedly
scheduling preparation, then release it and compare 8192 output frames
exactly with the reference; they also cover early and error publications.
Runtime verification and final device qualification are still pending.

A separate native transport counterexample reproduced a stale stopped-clock
bug: after Play requested 4 s, `EngineTransport.now()` still returned the
previous stopped sample at 6 s. The frontend's work-area check could cancel
that pending start. The driver now holds its requested position while a run
has not played and its available stopped sample still has the pre-Play
epoch. An exact seek updates that held position; newer stopped seek replies
and completed runs report their actual position even if the UI never polled
a moving sample. A moving sample and an explicit Pause retain prior behavior. The pure-module counterexample fails before
and passes after the fix. A protocol-fixture regression also covers those
transitions and a rate change; its full Vitest run is pending. This proves
the standalone bug, not the origin of the earlier V36 warm-Play Pause.
The two frontend files changed are `engine-connection/transport.ts` and its
existing `engine-connection.test.ts`.

Independent review found that the initial hold also caught an unpolled
run's newer stopped end sample. The hold now requires the pre-Play epoch;
protocol regressions include stopped seek replies, unpolled forward and
reverse ends, PlayAround and moving-clock rate changes. The original pure
counterexample still passes. This correction does not attribute the V36 Pause.


The corrected transport and presentation tests passed 26/26; renderer checking
reported zero errors and one existing warning. A before/after pure-module
check also reproduces the old unpolled forward/reverse end freeze and verifies
6 s / 0 s end positions with the epoch guard. Disabled audible scrubbing now
enqueues its exact seek through the same FIFO receipt path as ordinary seeks;
the updated ordered-error/shutdown regression passed. The temporary-root
guard checks the process table if Darwin denies a reaped group probe; it
still rejects an unverified live group. All five cleanup regressions passed.

V37 plain and stressed playback smokes each finished 39/40. All four distinct
cold starts in each run exceeded the enforced 10.667 ms atomic first-512
bound: plain 12.561/13.272/13.910/13.275 ms, stressed
12.283/12.783/12.481/11.800 ms. The profiled inputs were preloaded, with no
source publications during those starts. The older frozen V29 binary also
missed four times on the current host (13.071/12.226/13.484/11.884 ms).
These remain raw failures, with no noise dismissal. Fresh V39 checks later
passed all four plain starts at 6.447–7.376 ms and stressed starts at
6.699–7.349 ms, including exact 8192-frame PCM and zero start/RT counters.

V39's seven-row device diagnostic remains unqualified. On MacBook Pro
Speakers at 48 kHz with 64-frame callbacks, distinct unprepared seek measured
35.421/39.629/41.414 ms against today's 29.999/32.400/32.600; distinct speed
edits measured 33.251/35.015/36.256 against 30.413/31.613/32.792. Shared seek
passed at 26.784/28.142/28.541 against 29.410/32.606/32.708. Shared speed's
load-valid second pair passed at 29.963/31.616/31.660 against
30.000/32.192/32.599; its first pair was repeated only for the predefined
sharp load change, retaining its positive p95/max differences. All 204
native timed starts, including discarded-load attempts, reported zero program
underruns, omitted prefix, spread, A3 fires and RT errors. The 60 s loop pair
failed device binding: today's default changed from speakers to AirPods and
reported one underrun lasting 0.598062 s. The native speaker loop reported
zero window underruns, starvation and overloads; its lifetime underrun counter
was 1142. Every owned temporary tree was empty and removed, and final source
and artifact hashes matched. This does not replace final qualification.

The matched distinct-start profiles show roughly 70 ms aggregate worker CPU
across 16 voices, while the last voice spends about 3.3–3.6 ms throttled.
At the 64-frame period, the generic 500 microsecond admission reserve leaves
only about 0.767 ms of the worker's 1.267 ms computation allowance usable.
The next candidate reserves 250 microseconds only at stretch-process entry
and its internal checkpointed steps. Profiled process spans peaked below
166 microseconds in these captures; that observation is not a worst-case
proof. Unknown work, setup, seek and shuttle retain their existing reserves,
including the explicit 1 ms initialization reserve. Callback period,
computation and constraint limits, overrun assertions, canonical preroll and
head sizes are unchanged. Runtime and paired verification remain required.


Review of that reserve change found a short-period regression: 250
microseconds can exceed the old half-computation reserve at 96/192 kHz with
32-frame buffers. Process admission now uses the smaller of 250 microseconds
and the old default, for both producer and workers. Explicit setup requests
remain unchanged. New arithmetic tests exercise short-period thresholds; the
actual checkpoint path test exercises both roles at 48 kHz / 64 frames, plus
role/phase attribution. Fixed atomic counters now expose admitted DSP
span maxima by worker/producer and the 19 stable phase IDs through
`schedulingMaxDspUnitByRolePhaseNs`; they add no allocation on audio threads.
These are context-lifetime maxima, not per-Play deltas. They attribute new
measurements only; neither the earlier 394 microsecond maximum nor V40's
252.875 microsecond stressed maximum can be attributed retrospectively.
No worst-case claim is made from the old worker profiles.

V40 was not device-qualified. Its plain and stressed 40-test smokes each
finished 39/40 at the enforced cold-distinct bound. Plain first buffers were
10.804/9.066/9.707/9.496 ms; stressed were
11.156/11.223/14.065/11.700 ms. Every start remained complete and simultaneous
with zero program underruns and RT errors. The owned temporary tree was
empty and removed. These failures remain in the evidence; the short-period
correction and new span attribution require fresh runtime verification.

V41 built the runtime, addons and desktop app, passed strict Clippy and all
22 realtime unit tests. Its plain and stressed playback smokes again finished
39/40. The cold-distinct atomic first-512 times were
11.675/11.034/10.944/11.204 ms plain and
12.975/11.766/3.203/11.713 ms stressed, against the unchanged 10.667 ms bound.
The complete 8192-frame PCM comparisons and start/RT counters passed; these
timing misses remain failures. Both owned temporary trees were empty and removed.

The new context-lifetime counters measured worker/producer stretch-process
maxima of 180.333/108.458 microseconds plain and 655.416/110.125 microseconds
stressed. The stressed worker maximum exceeds the 250 microsecond admission
reserve and the declared 25 percent margin (at most 200 microseconds of work).
V41 therefore has no device capture. Existing hooks surround logical process
steps, but some steps execute whole FFTs or full spectrum passes. Opt-in CPU
timing inside those operators will locate the required checkpoint boundaries;
the canonical preroll, PCM arithmetic and realtime limits remain unchanged.
V40's plain cold miss had no throttle sleeps, so its timing cannot be credited
to a reserve change. The final campaign and its reservation remain pending.

The diagnostic-only V42 build passed strict Clippy, its enabled/disabled PCM
regression and all 22 realtime unit tests. Its queued cold captures were
cancelled before execution: independent review found that the first native
snapshot mixed output seek, process and flush. The revised diagnostic splits
those entry points. `PM_STRETCH_DSP_DIAGNOSTICS=1` enables fixed native CPU
maxima before audio work; logs distinguish process-global maxima since enable
from the realtime schedule's context-lifetime maxima. Neither is a per-trial
delta. Capture one exact test with one test thread; resetting counters while
background preparation runs would not produce a clean trial boundary.

Nested scopes cover analysis/synthesis steps and their FFTs, spectrum passes,
input copying, setup, energy, logical steps and the work between hooks. Inner
scopes contribute to their enclosing scopes. Local FFT/channel indices differ
from the block-relative logical-step ordinal; the block's step layout can
change after seeks. Native scopes do not identify a particular historical
Rust maximum or its worker role. Enabled timing adds CPU and changes checkpoint
placement, so its latency and Rust span maxima cannot qualify A7/A8 or set a
reserve. Re-measure with diagnostics disabled after any checkpoint fix. Windows
CPU timing is unavailable in this diagnostic rather than reporting scheduler
quantum values as operator durations. No device qualification is implied.


## Review round 9 baseline (2026-10-22; superseded)

Code revision `c035ad187` (later commits change docs only). The chronological logs of
rounds 1–8 are in `pm-audio-history.md`.

### What changed this round

| Review item | Change | Evidence |
|---|---|---|
| 1. AAC/Opus/Vorbis had no random access; every source held its file's bytes | One shared whole decode per file, read through the I/O context (see Decoders); files today refuses decode with random access | `tests/chromium.rs`: random reads bit-equal to the sequential decode, cold block read 0.14 ms, per-file sample and seek checks only; the SourceCache retained decoders without a total budget in production (fixed in round 10); paired AAC soundtrack rows below |
| 2. Final revision not qualified | Campaign and suites rerun on `c035ad187` | Below |
| 3. A precomposition's animated trim re-read at every run start in playback | The producer keeps today's index playhead (Playback above) | `a_precomposition_animated_trim_reads_at_todays_index_build` (fails on the old code: 264,000 against 96,000) |
| Flaky `queued_transport_receipts_preserve_control_order_and_individual_errors` | The test drains the first run's older-epoch chunks as a device would (it waited for frames the full ring could not take) | 8 of 8 debug runs pass (5 of 6 failed before) |
| Lifetime underruns rose 5–9 per edit-then-Play | An empty ring counts only after the run's first chunk was heard | `ring::tests::underruns_count_after_the_runs_first_chunk`; the start-hold test now expects 0 |
| Keyframed speed-1 clips were not in the gates | `tools/playback-gates.ts` takes speed, clip length, keyed gains along the clip, a video soundtrack, seek targets, the work area and a settle wait | Rows `kf-*`, `aac-video-*` below |
| (found by the new rows) a keyed level channel re-prepared its whole track at every evaluation | Prepared once per voice and revision under the standard evaluator | `prepared_level_tracks_equal_ev_p` (bit-equal to `evP`) |
| (found by the new rows) replaced documents were freed on the producer, and the presenter deep-copied the document at every mix | Retire thread for documents, graphs, voices and finished glides; the presenter shares its document (copy on write, copied while controls rest) | `replaced_documents_are_freed_off_the_rendering_thread`; a long keyed project's copy and free were 2.7 and 2.4 ms |

Outside `pm-audio`: `pm-node/src/present/audio.rs` (the shared document, about 20
lines). No other crate changed this round.

### A7/A8 paired campaign (2026-10-22 23:20 – 2026-10-23 00:28)

`tools/playback-gates.ts` under exclusive Electron reservations, paired hidden apps
(today first, then the engine; the distinct-trim row counterbalanced), 30 samples per
mode and row, MacBook Pro speakers at 48 kHz (engine output latency 15.6 ms, today's
24 ms), machine load 3–28 from other lanes. Every stressed row ran three times with four
busy load threads; the first stressed repeat's first rows overlapped two of this lane's
suite runs (00:00–00:04). Latency from the edit and Play (one renderer task) to the first
scheduled presentation, ms. Raw JSON is outside the repository
(`/tmp/ne-w5/audioa7-r9/campaign`).

| Row | Run | Today p50 / p95 / max | Engine p50 / p95 / max | ≤ today | Loop (engine underruns / starved / overloads; today underruns) |
|---|---|---|---|---|---|
| A7 60 s loop (16 voices, 1.5×), then gain edit | plain | 30.22 / 32.32 / 32.62 | stopped at its scheduling gate (below) | no | |
| A7 60 s loop (16 voices, 1.5×), then gain edit | plain, rerun | 30.42 / 32.51 / 32.52 | 21.63 / 22.21 / 22.38 | yes | 0 / 0 / 0; 0 |
| A7 60 s loop (16 voices, 1.5×), then gain edit | 4 load threads, 1 | 29.83 / 32.43 / 32.43 | 21.13 / 21.65 / 21.85 | yes | 0 / 0 / 0; 0 |
| A7 60 s loop (16 voices, 1.5×), then gain edit | 4 load threads, 2 | 29.03 / 32.43 / 32.53 | 21.56 / 22.34 / 22.96 | yes | 0 / 0 / 0; 0 |
| A7 60 s loop (16 voices, 1.5×), then gain edit | 4 load threads, 3 | 30.21 / 32.50 / 32.62 | 21.65 / 23.45 / 24.23 | yes | 0 / 0 / 0; 0 |
| speed edit (16 voices, 1.5×) | plain | 30.71 / 32.32 / 32.61 | 31.49 / 34.02 / 34.19 | **no** |  |
| speed edit (16 voices, 1.5×) | 4 load threads, 1 | 30.03 / 32.33 / 32.83 | 26.67 / 28.16 / 28.19 | yes |  |
| speed edit (16 voices, 1.5×) | 4 load threads, 2 | 30.71 / 32.32 / 32.43 | stopped at its scheduling gate at sample 15 (below) | no | |
| speed edit (16 voices, 1.5×) | 4 load threads, 3 | 29.72 / 32.42 / 32.61 | 31.28 / 31.91 / 32.37 | **no** |  |
| speed edit, 16 distinct trims (engine first) | plain | 29.21 / 32.11 / 32.40 | 34.55 / 36.67 / 37.96 | **no** |  |
| speed edit, 16 distinct trims (engine first) | 4 load threads, 1 | 30.13 / 32.12 / 32.23 | 30.98 / 31.96 / 32.82 | **no** |  |
| speed edit, 16 distinct trims (engine first) | 4 load threads, 2 | 30.22 / 32.22 / 32.32 | 32.12 / 33.63 / 34.59 | **no** |  |
| speed edit, 16 distinct trims (engine first) | 4 load threads, 3 | 30.93 / 32.23 / 32.52 | 32.19 / 33.94 / 34.53 | **no** |  |
| cold same-task seek (2 s / 3 s) | plain | 29.51 / 32.50 / 32.51 | 28.39 / 31.23 / 37.40 | **no** |  |
| cold same-task seek (2 s / 3 s) | 4 load threads, 1 | 29.61 / 32.31 / 32.41 | 30.30 / 32.33 / 34.78 | **no** |  |
| cold same-task seek (2 s / 3 s) | 4 load threads, 2 | 29.91 / 32.30 / 32.31 | 29.99 / 31.46 / 34.93 | **no** |  |
| cold same-task seek (2 s / 3 s) | 4 load threads, 3 | 30.23 / 32.30 / 32.31 | 29.96 / 32.10 / 33.06 | **no** |  |
| keyed speed-1 290 s clips: 60 s loop, then key edit | plain | 30.91 / 32.81 / 33.51 | 34.38 / 38.28 / 40.72 | **no** | 0 / 0 / 0; 0 |
| keyed speed-1 290 s clips: 60 s loop, then key edit | 4 load threads, 1 | 31.02 / 33.41 / 33.51 | 32.50 / 36.51 / 37.43 | **no** | 0 / 0 / 0; 0 |
| keyed speed-1 290 s clips: 60 s loop, then key edit | 4 load threads, 2 | 30.61 / 32.82 / 32.92 | 32.34 / 36.42 / 36.70 | **no** | 0 / 0 / 0; 0 |
| keyed speed-1 290 s clips: 60 s loop, then key edit | 4 load threads, 3 | 30.40 / 32.70 / 33.21 | 36.61 / 44.90 / 45.15 | **no** | 0 / 0 / 0; 0 |
| keyed speed-1 290 s clips: cold seek (230 s / 200 s) | plain | 30.51 / 33.71 / 33.80 | 22.24 / 24.42 / 24.87 | yes |  |
| keyed speed-1 290 s clips: cold seek (230 s / 200 s) | 4 load threads, 1 | 30.43 / 33.32 / 33.41 | 21.71 / 22.75 / 43.40 | **no** |  |
| keyed speed-1 290 s clips: cold seek (230 s / 200 s) | 4 load threads, 2 | 30.91 / 33.40 / 33.42 | 21.88 / 23.10 / 26.17 | yes |  |
| keyed speed-1 290 s clips: cold seek (230 s / 200 s) | 4 load threads, 3 | 31.19 / 33.60 / 33.65 | 21.90 / 24.47 / 27.45 | yes |  |
| AAC video soundtrack (290 s): cold seek (240 s / 200 s) | plain | 29.60 / 32.00 / 32.70 | 21.03 / 22.30 / 22.61 | yes |  |
| AAC video soundtrack (290 s): cold seek (240 s / 200 s) | 4 load threads, 1 | 30.40 / 32.60 / 32.61 | 20.94 / 21.63 / 22.13 | yes |  |
| AAC video soundtrack (290 s): cold seek (240 s / 200 s) | 4 load threads, 2 | 29.71 / 32.60 / 32.61 | 21.09 / 21.72 / 21.74 | yes |  |
| AAC video soundtrack (290 s): cold seek (240 s / 200 s) | 4 load threads, 3 | 30.00 / 32.30 / 32.30 | 22.09 / 23.99 / 27.27 | yes |  |
| AAC video soundtrack: gain edit at 240 s | plain | 30.02 / 32.47 / 32.61 | 20.59 / 21.42 / 21.74 | yes |  |
| AAC video soundtrack: gain edit at 240 s | 4 load threads, 1 | 30.01 / 32.32 / 32.53 | 21.09 / 21.89 / 21.89 | yes |  |
| AAC video soundtrack: gain edit at 240 s | 4 load threads, 2 | 29.12 / 32.23 / 32.52 | 20.91 / 21.75 / 21.95 | yes |  |
| AAC video soundtrack: gain edit at 240 s | 4 load threads, 3 | 30.32 / 32.42 / 32.71 | 21.70 / 25.50 / 25.78 | yes |  |

- **A7 holds.** Every 60 s loop (the 16-voice 1.5× loop, plain and three stressed, and
  the keyed 290 s clips, plain and three stressed) had 0 ring underruns, 0 starved
  renders and 0 overloads; today had 0 underrun events. The first plain A7 attempt
  stopped on its scheduling gate (one producer unit used 1.44 ms of CPU against its
  1.07 ms computation budget, phase unknown, while one of this lane's suite runs
  overlapped it); its loop had 0 / 0 / 0, and the rerun passed. One stressed speed row
  stopped the same way at sample 15 (a worker's stretch step: 1.32 ms period CPU against
  1.27 ms).
- **Every engine start was complete and simultaneous:** 944 traced starts across the 33
  rows, 0 program underruns, 0 omitted prefix frames, 0 start spread, 0 A4 safety-net
  fires, 0 history claims.
- **AAC (review item 1):** the soundtrack rows beat today on every statistic in all
  eight runs (engine p50 20.6–22.1 ms against today's 29.1–30.4 ms), with no A4 fire.
- **A8 misses in five scenarios.** Speed-edit, distinct-trim speed-edit, cold-seek,
  keyed key-edit and keyed cold-seek have 15 measured percentile/max misses in 20 runs,
  plus one scheduling-gate abort. The first four scenarios account for 14 measured
  misses in 16 runs (one stressed speed run beat today, one stopped on its scheduling
  gate). The keyed key-edit reaches +12.2 ms p95
  (44.90 vs 32.70) and +11.9 ms max (45.15 vs 33.21). The cold seek's p50 is at or
  below today's in two of four runs. The keyed cold seek beats today in three of four
  runs (stressed run 1 has one 43.4 ms maximum). Two causes, measured:
  1. **Upstream of the audio path, in the merged integration branch.** The time from
     the renderer's `PM.play()` to the audio presenter sending its transport commands
     (`requestedUs` against the request on the host clock) grew from 0.43 ms (round 8's
     cold seek) to 2.36–2.80 ms, and from 1.60 to 2.82–4.54 ms in the speed rows. A build
     of the round 9 base (`12b2463bd`: this branch with the integration merge and none of
     this round's audio changes) measured the same: 3.09–3.20 ms (cold seek) and
     4.29–4.86 ms (speed), and its producer's longest render grew alike (round 8
     8.3–9.4 ms; base 14.8–21.5 ms; this revision 10.7–14.6 ms). One identified part: the
     presenter now replans the viewer's surface pool on Play
     (`BridgePresenter::replan_transport_pool`, `pm-node/src/present/mod.rs`) before the
     render thread forwards the transport command to audio; skipping it (an env-gated
     A/B build, not committed; these A/B runs shared the machine with another lane's
     e2e run, both arms alike) took the cold seek's request time from 2.77–3.39 to
     1.39–1.47 ms. That code belongs to the render lane.
  2. **The document update of a keyed edit.** In the keyed row the request time is
     9.2–13.2 ms: the renderer's document update for one changed key in each of 16
     layers of 290 keys reaches the engine before the Play does. The engine's own part
     after the request is 7.6–8.3 ms (it was 15–20 ms before this round's prepared tracks,
     retire thread and shared presenter document).
  The distinct-trim row also carries round 8's DECISION item (sixteen independent
  canonical pre-rolls under the workers' Mach duty cycle). These measurements concern
  start latency; sample-parity checks are separate.
- Device note: the default output was switched to AirPods Pro (160 ms latency) and back
  during smoke runs before the campaign; every campaign row ran on the speakers.

### Playback suite on `c035ad187` (43 tests, `--test-threads=1`)

| Series | Runs passing | Failures | Machine load (1 min) |
|---|---|---|---|
| plain, 2026-10-23 00:29–01:20 | 13 of 30 | 17 runs, all `cold_starts_of_sixteen_distinct_voices_are_complete_and_simultaneous` | 6–82 |
| `PM_AUDIO_PLAYBACK_STRESS=1`, 01:31–02:17 | 15 of 30 | 15 runs: 14 only the distinct cold start; run 27 also three sixteen-voice tests, all on one idle-pool arm timeout (`arm_timeouts` 1, a process-wide counter) at load 26 | 11–57 |

Every other test passed in every run of both series except stressed run 27, including
the new `a_precomposition_animated_trim_reads_at_todays_index_build`. Stressed, every
run below a 1-minute load of 20 passed (12 of 12) and 15 of the 18 at 20 or above
failed. The one failing test is
the AUDIO-2 headless gate: sixteen distinct-trim voices must have their first 512
frames visible within 10.67 ms of a cold Play. Its plain misses measured 10.69–17.68 ms
(median 11.59 ms, 43 trials over the bound; stressed 10.74–20.78 ms, median 12.29 ms,
47 trials); 3 of the 14 plain runs at a 1-minute load below
20 failed, and 14 of the 16 at 20 or above. The 30-of-30 requirement is not met.

It is not a regression of this round. Alternating the round 9 base's test binary
(`12b2463bd`'s pm-audio, built from the same toolchain) with this revision's, 15 runs
each at load 31–46: both failed 15 of 15, with 45 of 60 trials over the bound each
(base 10.69–73.6 ms, this revision 10.69–79.9 ms). Earlier rounds recorded the same
intermittent failure at load 19–41. Making it pass under load is the DECISION item
above (the workers' Mach duty cycle for starts not yet audible). Earlier partial
series this round (`/tmp/ne-w5/audioa7-r9/suite`, `suite2`, `suite3`) ran while
campaign rows or other lanes' builds overlapped and are not counted.

### Remaining and orchestrator decisions

- **A8 latency misses (blocker, needs escalation):** the speed-edit, distinct-trim
  speed-edit and cold-seek rows on today's 16-voice fixture and the keyed key-edit row
  miss today's p50, p95 or max in 14 of 16 runs. The keyed cold seek also misses
  once (43.40 vs 33.41 ms max): **five scenarios** miss, 15 of 20 runs. The largest
  keyed-edit miss is **+12.2 ms p95**, not 6 ms (table above). The audio
  path after the request is equal or faster than round 8's apart from the distinct
  pre-rolls; the added time is before the request, in the merged integration branch
  (the render lane's Play pool replan, and for the keyed row the document update
  pipeline). Needs the render and document-host lanes, or the audio presenter receiving
  Play ahead of the render thread's surface work.
- **AUDIO-2 distinct cold-start bound and the stressed distinct-trim row:** the worker
  period change was not attempted in round 9 (DECISION, as in round 8); implemented
  in round 10.
- **Real-time allocations:** 316,504 per 200 warm renders, unchanged (DECISION; the
  follow-up lane).
- **LONG-1 (approved for the beta list; current side-by-side in pm-audio.md):** files longer than today plays
  (decoded buffer past its 128 MiB `audio` budget) play in the engine through
  Powermove's random-access decoder (the plan's "no length limit"); today refuses
  them, so there is no today sample to match.
- **Gaps:** today's index playhead for exports and the first-open warm-up, the silent
  decoder fallback in the app, and sharing prepared tracks across revisions
  (see [the current gaps](pm-audio.md#gaps)).
