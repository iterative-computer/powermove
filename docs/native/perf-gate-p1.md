# Phase 1 performance gate

Status, 2026-11-01: **INCOMPLETE — no valid paired timing runs.** The owner
confirmed the Mac stays locked tonight; no on-screen runs or claims are made. This report
does not establish a P1 pass or a measured performance regression. The initial
preflight refused to launch either benchmark app: other Powermove instances
were active, one-minute load was 17.01, and builds were active. Existing apps
were left untouched. Zero observations means p50, p95 and ratios are unavailable,
not zero. The behavior-reference build succeeded; native release verification
and cleanup are recorded below.

## Machine and conditions

| Item | Observed |
| --- | --- |
| Hardware | Apple M5, model Mac17,2, 16 GiB RAM |
| OS | macOS 27.2, build 26B5086k |
| Power | AC, battery charged |
| Initial available disk | About 46 GiB; checked before builds; stop threshold 8 GiB |
| Initial preflight load (1/5/15 minutes) | 17.01 / 17.56 / 11.83 |
| Quiet window | Unavailable: locked Mac, existing app instances and builds |
| Required machines | M4 Max with 120 Hz XDR and cohort base M-series at 60 Hz; neither certified here |
| Display/quality/thermal certification | Not performed; no on-screen timing session entered |
| Reference | Frozen `v1.2.0`, commit `07ed5fa17ee640659cb075f9cad1af2095011d60` |
| Native | `apps/native/crates/app`, release, pinned Rust 1.96.1 |
| Projects | Only generated copies from public `engine/corpus`; no personal projects or installed profile read |

The task's explicit 8 GiB stop threshold controls this run's build decision.
The stricter long-term free-space target is not satisfied by this Mac. This
machine must not be presented as one of the two specified performance rigs.
`powermetrics` was invoked directly without sudo and returned
`powermetrics must be invoked as the superuser`. No privileged retry or prompt
was attempted.

## Method

[`tools/perf-gate`](../../tools/perf-gate/README.md) contains a Python driver,
gate evaluator, Darwin process accounting, reference adapter, corpus preparer
and lifecycle wrapper. Run `tools/perf-gate/run.sh --runs 30 --playback-runs 3
--refresh-hz 60` from a quiet Mac. Read the tool README before interpreting
the observations: several required endpoints are still unavailable.

The reference is built in a detached temporary worktree using `bun install
--frozen-lockfile` and its desktop `build` package script. Following the trace
recorder, the tag's own e2e helper is transpiled for Node with finite startup
waits and failure cleanup. No behavior trace probes are enabled for timing.
The generated helper is untracked; app sources remain unmodified. Each launch
gets an isolated temporary user-data directory. Native uses a separate temporary
profile and no implicit migration. The wrapper removes the reference, its
node_modules and its own native release artifacts on exit.

Each pair acquires the GUI lock with noclobber, retries an occupied lock every
30 seconds, and releases only its own lock after closing both apps. A nine-minute
alarm bounds a batch below ten minutes. Native/Electron order alternates, with
one benchmark instance at a time. Existing app instances or builds block timing.
The runner also refuses a locked or unverifiable Mac session. Load, power,
OS build and disk are recorded before and after each app; invalid
pairs are retained with reasons and excluded from acceptance. Raw logs, profiles,
fixture identities, hashes and observations stay outside Git.

The default is 30 independent launch pairs for readiness, open, cached seeks,
edits, idle CPU and small-project footprint. Playback CPU collection defaults to
three complete 60-second intervals per side, looping the short composition;
these reduced repeats are explicitly diagnostic because physical-frame, GPU
and energy endpoints are missing. No shortened run counts as 60 seconds. Set
`--playback-runs 30` when full measurement endpoints exist. **This execution
collected zero pairs for every row**, because repeating a known invalid quiet
window cannot provide valid evidence.

Percentiles use nearest rank (p95 of 30 values is rank 29). Ratios compare native
and Electron at the same quantile; missing or zero-denominator comparisons stay
missing. Absolute and relative clauses must both pass. Required hardware,
conditions and endpoint checks cannot be overridden by fast diagnostic numbers.
The evaluator reports PASS, FAIL for an observed budget failure, or INCOMPLETE
for missing evidence. Reported failures never turn incomplete evidence into a
full gate verdict.

## Results

All values below are native/Electron paired results. A dash means **not
measured, n=0 per side**. `R` is the configured refresh interval in milliseconds.
The `R+1` / `R+2` latency wording is conservatively implemented as milliseconds;
this interpretation has not been used to accept any result.

### Cold launch to first interactive frame

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | p95 <400 ms and ≤50% of Electron | INCOMPLETE |

Controller/harness readiness is recorded only as a diagnostic. Neither endpoint
proves the first interactive, physically displayed frame, and filesystem caches
are not purged. Fresh process/profile launch is not claimed to mean disk-cold.

### Open public corpus composition

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | ≤50% of Electron; no main-thread stall >1 frame | INCOMPLETE |

The public fixture is unaltered and extension-free. Native file/kernel opening
and the reference corpus-event path have different start boundaries. This does
not substitute for a paired PMV file-open measurement or the private-project
budget. Main-thread stall instrumentation is still needed.

### Cached scrub, input to presentation

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | p95 ≤R+1 ms and ≤60% of Electron | INCOMPLETE |

The native diagnostic requires exact frame ticks, a cache hit and drawable
`presentedTime` after command acceptance. Electron matches the real render
event, waits for a WebGL fence, then a subsequent rAF. As documented by the
existing 120 Hz rig, this is a compositor opportunity, not physical scanout.
The endpoints differ and input is an automation command, so no gate ratio is
accepted from them.

### Random seek, 4K long-GOP

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | Preview ≤R+2 ms; full ≤150 ms; ≤60% of Electron | INCOMPLETE |

The prepared public shape fixture has no video. A validated public 4K long-GOP
media fixture, equal media hydration, seeded seek sequence, preview/full-quality
identities and physical presentation observations remain required. No small
video or shape seek is substituted for this row.

### Playback, 60 seconds

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | At 60 fps, zero drops; frame-work p95 ≤half frame interval | INCOMPLETE |

Latest-only native frame status can miss intervening frames. The service lane's
opt-in `PM_ENGINE_TRACE` is retained and analyzed without claiming a complete
frame census. Skipped-target
counters, bus events or GPU completions cannot independently prove a physical
zero-drop census. A complete frame/presentation stream and matching per-frame
work distribution are needed. The prepared corpus composition is 30 fps and is
never used to claim a 60 fps pass.

### Playback cost

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | CPU ≤60%; GPU time ≤80%; energy ≤70% of Electron | INCOMPLETE |

Unprivileged process-family CPU time is available through `proc_pid_rusage` v4.
Mach ticks are converted with `mach_timebase_info`; one logical CPU is 100%.
GPU-time intervals are not exposed by the native public status seam, and
app-attributed energy is unavailable. System-wide power or CPU time is not a
replacement for either missing clause. No CPU-only row passes the combined
budget.

### A/V

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | Matched-device mean absolute offset ≤1 ms; p95 jitter ≤2 ms | INCOMPLETE |

Native service traces retain matched-frame audio host and physical display
timestamps when available. Their signed/absolute offset and jitter around the
mean are diagnostic until the measurement window and full pairing are proved.
Matched device timestamps are not available through the reference adapter. Clock polling or subtracting unrelated
host/media times is not accepted as an A/V measurement.

### Edit to repaint

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | p95 ≤R+1 ms; ≤60% of Electron; main-thread work ≤4 ms/frame | INCOMPLETE |

The diagnostic makes a real kernel opacity edit and checks a changed frame
stamp. Viewer presentation does not establish completed shell repaint. The
paired physical endpoint and native main-thread work measurement are missing.

### Idle CPU and wakeups

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | CPU <1% and ≤50% of Electron; ≤2 wakeups/s per idle extension | INCOMPLETE |

The runner settles before a configurable idle interval, then reads the whole
process family without polling either app. Interrupt and package-idle wakeups
are separate kernel counters; neither attributes wakeups to each idle extension.
Process-family churn or PID reuse invalidates the interval. The quiet window
blocked even these otherwise accessible CPU measurements.

### Small-project memory

| Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Result |
| --- | --- | --- | --- | --- |
| — / — | — / — | — / — | Native family ≤Electron family −150 MB | INCOMPLETE |

The driver records per-process `phys_footprint`, family sums, a base checkpoint
and document delta. Native Metal device allocation is reported separately and
never added to the footprint. Shared CPU/GPU allocations counted once and an
extension allocation split require additional accounting; neither is inferred
from RSS or an unverified sum. Therefore family sums remain diagnostic.

## Minimal app edits

Only two app files belong to this lane:

- `src/main.rs`: declare `perf_gate` and install it after controller creation.
- `src/perf_gate.rs`: opt-in `PM_PERF_GATE=1` JSON-lines command/status channel,
  requiring an explicit temporary profile; reuse existing open, seek, edit and
  transport APIs, configure a full-composition playback loop, and expose existing
  frame/status and Metal allocation observations. No polling timer or listener
  is installed when the flag is absent.

Kernel, engine-service, shell, vendor, engine and desktop source files were not
edited by this lane. Other lanes' work in the shared checkout is excluded from
this lane's commit.

## Decisions and rules

| Decision | Picking rule |
| --- | --- |
| Frozen tag, its package scripts and e2e launcher; no behavior trace probes for timing | N6 frozen behavior reference; trace-recorder notes warn that probes add overhead |
| Temporary profiles and public corpus only | Explicit task safety constraint; no private projects/profile discovery |
| Refuse timings while other apps/builds run; retain missing results | One-app quiet-window rule and conditions; N2 comparative evidence |
| Do not terminate existing app instances to obtain numbers | Shared unlocked Mac and explicit prohibition on real-profile interference |
| Separate native drawable presentation from Electron fence+rAF | N3 correctness; the 120 Hz rig explicitly distinguishes compositor opportunity from physical presentation |
| Missing A/V, GPU, energy and shared-memory evidence blocks acceptance | Every budget clause is required; no inferred stand-ins for device observations |
| Nearest-rank p50/p95, alternating independent launches, ≥30 for gate acceptance | Run-count rule and paired comparison requirement |
| Reduced CPU-only playback repeats, with stated reason | Task permits fewer where impractical; missing combined-row endpoints prevent acceptance regardless of CPU repeat count |
| Keep native hooks in app; no service/kernel redesign | Assigned ownership and minimal instrumentation rule; N5 reuse |
| Report M5 observations without certifying required rigs | Required-machine rule; do not equate different hardware |

## Verification and remaining work

The harness acceptance/safety suite passes 14 tests, including locked-preflight
refusal before app construction, real Darwin
child-process enumeration and CPU time conversion checked against process
resource usage. Shell and Node syntax checks pass. Public-corpus preparation
and the Node e2e-helper transpilation succeed. The frozen reference's Bun install
and desktop production build succeed without tracked reference changes.

The final `cargo +1.96.1 build --manifest-path apps/native/Cargo.toml --release
-p pm-native-app` succeeds. Final preflight exits 2 as intended: screen locked,
load 13.97 / 15.51 / 13.37, other app instances active, and a build still
active in the shared checkout. It accepts zero pairs and reports every row
INCOMPLETE. The owned reference worktree (including node_modules), generated
corpus and `~/Developer/pm-target/release` were removed after verification; no
GUI lock was acquired or left behind. No on-screen session, screenshots or user-profile action
was performed. No Opus review or in-person gate verification is claimed.

To close P1 performance, provide the two specified rigs and a real quiet window;
add equal file-open/input/repaint endpoints, native main-thread work and complete
frame telemetry, physical Electron presentation and matched A/V device stamps,
GPU-time/app-energy observations, the validated 4K long-GOP public fixture,
extension wakeup attribution and deduplicated memory accounting. Then collect
at least 30 valid paired runs per row across the required corpus scope and
repeat the percentile/budget evaluation. The current harness is a conservative
measurement foundation, not a completed P1 certification.
