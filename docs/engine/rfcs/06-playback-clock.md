# RFC 06: The playback clock

Status: draft, version 0. Module: `pm_protocol::clock`. Gates Phase 2;
freezes when Phase 2 starts. Owner: Audio (the Audio lane joins in Phase 0
for this contract, section 27).

## Purpose

The master clock is the audio device clock, including device latency (plan
section 15). The interactive engine publishes clock samples, each a line
"at host instant H the listener hears media time M, advancing at rate R".
Every consumer (the viewer's presentation, the A/V sync measurement, the
job engine's slicing governor) derives media time from the latest sample
with the same integer math, so they agree to the tick. Transport commands
from the renderer are notifications; the clock samples are the answer.

## Types

| Type | Meaning |
|---|---|
| `ClockSample` | `seq`, `epoch`, `TransportState`, `host_time` (µs), `media_time` (ticks), `PlaybackRate` (signed rational), output latency (informational), device sample rate, optional loop range |
| `TransportState` | Stopped, playing, shuttling (scrub or J/K/L at a non-real-time rate, audio scrub active) |
| `TransportCommand` | Play at a rate, pause, seek (exact or scrub), step by whole frames, set the loop range, play around the current frame, audio scrub on or off |
| `SeekMode` | Exact (original quality as soon as it decodes) or scrub (nearest fast frame, bypassing the jitter buffer) |
| `JKL_RATES` | 1× and 2×, forward and reverse |
| `AudioMessage` | One session-bound `audio` event between the renderer and the interactive engine: `monitor` (the mixer's monitor solo, renderer → engine), `levels` (`AudioLevels`: peak and RMS per bus channel of each strip and the master, with the media time heard; engine → renderer about 30 a second while playing, once silent when it stops), `peaksWanted` (renderer → engine) and `peaks` (`AudioPeaks`: an asset's 100-a-second envelope or why there is none) |
| `HostMicros` (contract 9) | A monotonic host instant in microseconds shared by every process on the machine: `mach_absolute_time` converted to µs on macOS, `QueryPerformanceCounter` on Windows, `CLOCK_MONOTONIC` on Linux |

Waveform requests carry an optional `request: Seq`. The worker echoes it on both successful and failed `AudioPeaks` replies, including cached envelopes. Consumers allocate a new generation after relink, replacement and full document resync, and reject a reply whose generation is no longer current. A request waiting for media resolution retains the latest generation for that session and asset. Omitted generations remain accepted by the engine for older wire clients.

## Invariants

1. **One formula.** `ClockSample::media_time_at(at)` is the only way to
   derive media time: `media_time + floor((at − host_time) × TICKS × num /
   (10⁶ × den))` in i128 with Euclidean division, then wrapped into the
   loop range with `rem_euclid`, then clamped to the safe tick range.
   Stopped clocks return `media_time`. Tested at 1×, −2×, before the
   anchor (floored toward −∞), across a loop wrap, and at 1000/1001 where
   1001 s of host time is exactly 1000 s of media.
2. **Publication.** A sample is published on every discontinuity (play,
   pause, seek, rate change, device change) with a new `epoch`, and at
   least 4 times a second while playing. Samples with the same epoch
   describe one continuous line; a consumer replaces its line only when the
   epoch or `seq` advances. A loop wrap is not a discontinuity: the line
   carries the loop range and invariant 1 wraps it, so one epoch spans
   every pass of a looped run (consumers can prepare the loop start ahead
   of the wrap). Changing the loop range publishes a new `seq`. A sample
   carries `loop_range` only while the media heard is inside the loop and
   the run wraps at its end: a run from a playhead before the loop start
   is a straight line (no range) until it enters the loop, where a new
   `seq` with the range follows; a run past the loop end, a play around,
   a scrub grain, and a stopped clock carry none. The range published is
   the sample-aligned one the audio wraps at.
3. **Latency is folded in.** `host_time` is when `media_time` reaches the
   listener, output latency included; `output_latency_us` is for logs only.
4. **State and rate agree.** A stopped clock has rate 0 and a running clock
   does not; the sample rate is 8 to 192 kHz; a loop range is non-empty.
   Checked by `ClockSample::validate`.
5. **Commands are well formed.** Play needs a non-zero rate with a positive
   denominator; a step moves at least one frame; pre-roll and post-roll are
   non-negative; a loop range is non-empty. Checked by
   `TransportCommand::validate`.
6. **Presentation** (section 15): the viewer presents the frame whose PTS
   interval contains the predicted display time, computed from the clock
   and the display link, not the rAF callback time.

## Phase 3b decisions (Audio lane, 2026-10-16)

- **J/K/L.** `Play { rate }` takes the four `JKL_RATES`; other rates are refused
  (faster shuttles are after cutover). A new rate while playing is a discontinuity: a
  new epoch from the media heard. Non-1× runs publish `Shuttling` samples at their rate
  (−2/1, −1/1, 2/1); invariant 1 already covers negative rates. 2× keeps the mix's pitch
  (one time stretch on the master); reverse plays the mix backwards (reverse audio).
  Reverse runs stop after media 0 and wrap inside the loop from its start to its end
  (invariant 1 wraps any line that carries a range, whatever the sign of its rate).
- **Play around.** `PlayAround { pre_roll, post_roll }` is a 1× run from `pre_roll`
  before the current media time to `post_roll` after it, carrying no loop range; when it
  ends the clock stops back on the media time it started around.
- **Open question 2 (scrub audio): decided.** Scrub audio rides on `Shuttling` samples:
  with `AudioScrub { enabled: true }`, a scrub seek or a step while stopped plays a grain
  (one frame or 60 ms, 4 ms fades) on a `Shuttling` line from the target, then a
  stopped sample on it. Consumers that follow a stopped line ignore grain samples (the
  viewer does not move).
- **Audio panels.** `AudioMessage` (above) joins this contract; it is session-bound and
  validated (`AudioMessage::validate`: finite non-negative levels per channel, at most
  4,096 strips, at most 3.6 million peaks).

## Versioning

Version 0 until Phase 2 starts. Reverse audio, faster shuttle, and frame
blending after cutover may add `TransportState` or `TransportCommand`
variants; each is a version bump.

## What Phase 0 must confirm

- **Audio lane, device clock:** that `cpal` on macOS exposes the device's
  presentation timestamps and latency precisely enough to produce
  `host_time` within the A/V sync bound (±10 ms, measured by photodiode
  loopback in the lab and frame-callback timing in the app).
- **S1 (frame bridge):** that the viewer's display link and the engine's
  `HostMicros` share a timebase across processes (`mach_absolute_time` is
  system-wide on macOS; S1 verifies against `requestVideoFrameCallback`
  presentation times), and that 4 samples a second keep presentation
  within the cadence bar (23.976 and 25 fps on 60 Hz and 120 Hz with zero
  irregular repeats over 60 s).
- **S8:** that the job engine can use the same samples for its slicing
  decisions (contract 13) without its own audio device.

## Open questions

1. Whether µs host time is fine-grained enough. One µs is 705.6 ticks, well
   under a frame at 240 fps (2,940,000 ticks); drift is impossible because
   each sample re-anchors. Nanoseconds would overflow the safe integer range
   after 104 days of uptime, so µs stays.
2. Whether scrub audio needs its own sample stream (short grains at the
   scrub position) or rides on `shuttling` samples. Audio lane decides in
   Phase 3b with J/K/L and audio scrub.
3. Whether a device change should pause playback or continue on the new
   device with a new epoch. The contract allows both; the Audio lane picks
   with Jude.
