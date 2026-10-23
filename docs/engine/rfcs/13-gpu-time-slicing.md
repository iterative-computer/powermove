# RFC 13: GPU time slicing and deadline signals

Status: draft, version 0. Module: `pm_protocol::slicing`. Gates Phase 2;
freezes when Phase 2 starts. Owner: Engine and platform (priority and
slicing, section 27). Producers: the interactive engine (signals), host
services in main (relay and policy). Consumer: the job engine.

## Purpose

Two engine processes share one GPU. Playback in the interactive engine must
not drop frames while the job engine exports 4K with blur, renders agent
snapshots, or waits on Blender (plan section 8, "Device"; section 4,
principle 7). Metal exposes no public way to give one process's GPU work priority
over another's, so the job engine yields cooperatively: during playback main signals it, it splits its
work into command buffers of at most 2 ms of GPU time (predicted to 1 ms),
keeps one in flight, submits only into the gap after the interactive
engine's frame (frame alignment), runs at utility QoS during playback (user
exports use default QoS while idle), and yields when the
interactive engine reports a deadline miss. Blender jobs pause during
playback. Spike S8 measured the policy (`docs/engine/spikes/S8.md`); plan
section 26 amendment 12 adopted it.

## Types

| Type | Meaning |
|---|---|
| `DeadlineSignal` | `Event::Deadline`, tagged by `signal`: `playbackStarted {at, rate, frameBudgetUs}`, `playbackStopped {at}`, `deadlineMiss {at, frameSeq, overrunUs}`, `qualityStepDown {at}` (interactive engine → main → job engine), and `frameDone {at, frameSeq, tick, nextSubmit, late}` (interactive engine → job engine on the direct port, once per frame) |
| `SlicingPolicy` | `Event::Slicing` (main → job engine): the hard bound on a job-engine command buffer and on any dispatch in it, the prediction target, command buffers in flight, the alignment guard, how long to submit nothing after a miss, and what Blender does during playback |
| `BlenderThrottle` | `pause`, or `throttle {maxWorkers}` (not used for playback, see invariant 8) |
| `SlicingGovernor`, `SliceBudget` | The job engine's state machine: feed it signals, ask it for a budget before each submission: `unlimited`, `sliced {maxUs, targetUs, maxInFlight, deadline?}`, `waitForFrame {until}`, or `yieldUntil(instant)` |
| `HostMicros` (contract 9) | The shared monotonic clock all signals are stamped with |

`SlicingPolicy::DEFAULT` (plan section 26, amendment 12, from S8): command
buffers bounded at 2 ms and predicted to a 1 ms target, one in flight, a
1 ms alignment guard, a 100 ms yield after a miss, Blender paused during
playback.

Implementation: `pm_protocol::slicing` (types and governor), and in
`pm-engine` the cost model and slice planner (`pm_engine::slicing`: per-pass
per-unit costs calibrated from GPU timestamps, corrected from the 90th
percentile of measured over predicted time, packing work units into command
buffers under the target and the alignment deadline, refusing any unit
predicted above the bound so the caller bands it finer) and the in-flight
limiter. The direct engine-to-engine port is made by main
(`apps/desktop/src/main/engine/`) and replaced whenever either engine
restarts.

## Invariants

1. **Command buffers during playback are at most 2 ms** of GPU time
   (section 8), and so is every dispatch in them: long passes are cut into
   bands. `SlicingPolicy::validate` rejects a bound above 2,000 µs or zero,
   a prediction target of zero or above the bound, 0 or more than 4
   buffers in flight, an alignment guard above 8 ms, a yield above 1 s, and
   a throttle of zero workers (pause means none).
2. **Signals are well formed.** Playback starts at a non-zero rate
   (reverse is negative) with a positive frame budget; a miss overruns by at
   least 1 µs; a finished frame's next submission comes after its tick
   (`DeadlineSignal::validate`, run by the decoder).
3. **The governor is the only submission gate** (tested): no playback →
   unlimited. During playback it waits until two frames have finished
   (`ALIGN_WARMUP_FRAMES`), then allows slices predicted to end by
   `nextSubmit − alignGuardUs` of the latest `frameDone`, and asks the job
   engine to wait for the next `frameDone` inside the guard. If no
   `frameDone` arrives for 8 frame budgets (`ALIGN_STALE_FRAMES`), it
   falls back to reactive slicing (bounded buffers, no deadline) so a silent
   interactive engine cannot starve jobs. After a miss or a quality
   step-down it submits nothing until `at + yieldAfterMissUs`, extending
   (never shortening) an active yield; playback stopped clears the yield and
   returns to unlimited. Out-of-order `frameDone` signals keep the latest
   plan. Blender's playback behavior is the policy's while playing and
   unrestricted otherwise.
4. **Time is the shared host clock.** Every signal carries `at` in
   `HostMicros` (contract 6), so the job engine compares instants with its
   own clock without translation; a signal delayed in transit shortens the
   yield rather than extending it.
5. **Classes decide who slices** (contract 12): everything outside the
   interactive engine is sliced during playback; generation and background
   classes pause entirely. Agent snapshots are sliced, not paused, so agents
   keep getting frames during playback (section 20).
6. **Leases move with playback** (contract 8): main shrinks the job
   engine's lease at `playbackStarted` and restores it at `playbackStopped`.
   Slicing bounds GPU time; leases bound memory. Both are needed for the S8
   row.
7. **Hardware codec sessions are arbitrated by main**, not by slicing
   (contract 14, `HardwareSessionRequest`).
8. **Blender pauses for playback.** Main always sends `blender: pause` in
   the playback policy: S8 measured that a worker count does not bound GPU
   bursts and a `SIGSTOP` duty cycle leaked them (18 and 96 drops).
9. **Main stops playback on behalf of a crashed interactive engine.** When
   the interactive engine exits, main sends the job engine
   `playbackStopped` and restores its lease, so the job engine never waits
   on a dead peer (the stale fallback in invariant 3 covers the gap).
10. **EXPORTQOS-1: User-started exports use Darwin default QoS while playback is idle**
    (export sheet and render queue), and utility QoS while it is active.
    Their render, encoder, and CPU conversion threads refresh their own class
    at frame, GPU band, and row boundaries, then the render lane returns to
    utility at job exit. Agent exports, snapshots, background and generation
    work, and job infrastructure remain at utility QoS. No engine-owned job
    thread enters Darwin's background state (S8: no GPU effect, and slower
    reaction to signals). The private `setGPUPriority:` is not used (Jude's decision,
    S8 recommendation 11). External encoder processes are outside this
    thread-level QoS control.

## Versioning

Version 0 until Phase 2 starts. S8 set the default numbers; changing a
default after the freeze is not a wire change, but changing the 2 ms ceiling
in `validate` or adding a signal is a version bump. `frameDone` and the
policy's target, in-flight, and guard fields were added in the draft
(amendment 12) before the freeze.

## Phase 2 infrastructure

Main sends the job engine the default policy after every handshake,
relays the interactive engine's non-per-frame signals, moves leases with
playback, re-sends `playbackStarted` to a job engine that restarts during
playback, and sends `playbackStopped` itself when the interactive engine
dies (`apps/desktop/src/main/engine/native-engine.ts`). The interactive
engine sends `frameDone` on the direct port (`Engine::signal_deadline`);
the job engine feeds every signal to `JobSlicer`. Job infrastructure starts
at utility QoS (`pm-node`); only user-started export workers change class
under invariant 10. Notes: `docs/engine/host-processes.md`.

## Phase 0 results

S8 passed with aligned slicing at a 1 ms prediction target (zero drops in
the gate scenario, amendment 12). The relay question is answered: per-frame
signals use a direct port; with a signal per frame, 4 ms of relay latency
cut export throughput by 24 to 55%. The yield stays fixed at 100 ms (open
question 1: with alignment it almost never fires). One 2 ms bound for the
whole job engine is the default (open question 2); a per-class slice needs
no wire change. Finer Blender throttling does not work (open question 3).

## What Phase 0 had to confirm

- **S8 (GPU contention), the gate:** 4K playback in the interactive engine
  while the job engine exports 4K with blur and Blender EEVEE renders, zero
  drops over 60 s (section 25). On failure: stronger slicing (smaller
  command buffers, longer yields) or pausing jobs during playback.
- **S8, mechanism:** that the job engine can bound a command buffer's GPU
  time before submitting it. wgpu has no preemption, so this needs per-pass
  cost estimates calibrated from GPU timestamp queries; S8 measures how
  close the estimates get to 2 ms on the reference machine, and whether
  Metal's lower-QoS process scheduling (`setpriority`, task QoS
  `utility`) has a measurable effect on GPU contention or only on CPU.
- **S8, latency:** that relaying deadline signals through main is fast
  enough. The signal path is interactive engine → main → job engine, two
  MessagePort hops; if S8 shows the relay delay matters, main hands the two
  engines a direct port for this event only (the message does not change).
- **S1:** that a `deadlineMiss` can be detected in the interactive engine at
  the presentation deadline (the frame bridge reports late frames), not only
  after the viewer drops one.

## Open questions

1. Whether the yield should scale with `overrun_us` (a 900 µs overrun and a
   12 ms overrun get the same 100 ms today). S8 decides with real traces.
2. Whether agent snapshots should get a larger slice than exports during
   playback, since they are higher priority (contract 12). One slice size
   for the whole job engine is simpler; S8 measures whether agent latency
   needs more.
3. Whether the Blender worker, a separate process the engine does not
   control at the GPU level, can be throttled finer than pause or a worker
   count (for example `SIGSTOP` between samples). Pause is the safe default.
4. Windows after cutover: DX12 has queue priorities but no cross-process
   preemption guarantee either; the same cooperative design is expected to
   apply.
