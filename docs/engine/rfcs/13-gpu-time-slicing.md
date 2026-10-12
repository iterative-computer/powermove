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
work into command buffers of at most 2 ms of GPU time, runs at lower QoS,
and yields when the interactive engine reports a deadline miss. Blender
jobs pause or throttle during playback. Spike S8 proves the policy; the
numbers in it are drafts until S8 passes.

## Types

| Type | Meaning |
|---|---|
| `DeadlineSignal` | `Event::Deadline` (interactive engine → main → job engine), tagged by `signal`: `playbackStarted {at, rate, frameBudgetUs}`, `playbackStopped {at}`, `deadlineMiss {at, frameSeq, overrunUs}`, `qualityStepDown {at}` |
| `SlicingPolicy` | `Event::Slicing` (main → job engine): the longest job-engine command buffer during playback, how long to submit nothing after a miss, and what Blender does during playback |
| `BlenderThrottle` | `pause`, or `throttle {maxWorkers}` |
| `SlicingGovernor`, `SliceBudget` | The job engine's state machine: feed it signals, ask it for a budget (`unlimited`, `sliced {maxUs}`, or `yieldUntil(instant)`) before each submission |
| `HostMicros` (contract 9) | The shared monotonic clock all signals are stamped with |

`SlicingPolicy::DEFAULT`: 2 ms command buffers, a 100 ms yield after a miss,
Blender paused during playback.

## Invariants

1. **Command buffers during playback are at most 2 ms** of GPU time
   (section 8). `SlicingPolicy::validate` rejects anything above 2,000 µs or
   zero; the yield after a miss is at most 1 s; a throttle allows at least
   one worker (pause means none).
2. **Signals are well formed.** Playback starts at a non-zero rate
   (reverse is negative) with a positive frame budget; a miss overruns by at
   least 1 µs (`DeadlineSignal::validate`, run by the decoder).
3. **The governor is the only submission gate** (tested): no playback →
   unlimited; playback → sliced at the policy's maximum; after a miss or a
   quality step-down → submit nothing until `at + yield_after_miss_us`,
   extending (never shortening) an active yield; playback stopped → clears
   the yield and returns to unlimited. Blender's playback behavior is the
   policy's while playing and unrestricted otherwise.
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

## Versioning

Version 0 until Phase 2 starts. S8 sets the default numbers; changing a
default after the freeze is not a wire change, but changing the 2 ms ceiling
in `validate` or adding a signal is a version bump.

## What Phase 0 must confirm

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
