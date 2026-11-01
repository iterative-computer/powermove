# P1 paired performance harness

Run from the implementation checkout on macOS with Node 22.18+, Bun 1.3+,
Python 3.9+, the pinned Rust toolchain and the existing FFmpeg environment:

```sh
python3 -m unittest discover -s tools/perf-gate -p 'test_*.py' -v
tools/perf-gate/run.sh --runs 30 --playback-runs 3 --refresh-hz 60
```

The wrapper checks `df -h /` before each build and refuses below 8 GiB. It
creates a detached `v1.2.0` reference outside the repo, installs with Bun,
builds with the tag's desktop `build` script, then builds `powermove-native`
in release using `engine/env.sh` and `~/Developer/pm-target`. Existing reference
or release directories are refused rather than reused or deleted. An exit trap
removes its reference (including node_modules) and release build. Raw JSON,
generated documents and runtime data remain in the printed `/tmp` directory;
never stage them. The script does not publish or change any real profile.

`prepare.ts` reads an unaltered extension-free shape composition from the
public corpus. It writes its document and ordinal/hash metadata only to `/tmp`.
It transpiles the frozen tag's e2e helper for Node, following trace-recorder's
approach, with finite timeouts and failed-launch cleanup. App renderer/main
sources and the reference's behavior remain unchanged; no behavior trace
probes are enabled for timing. Electron always gets a fresh temporary profile.

The Python driver alternates native/Electron order across independent launch
pairs. Every pair owns `/tmp/pm-shots/gui.lock` using noclobber, retries an
occupied lock every 30 seconds (up to the configured wait), and releases it
after both apps close. A nine-minute alarm bounds each batch. A foreign lock
is never removed. Existing Powermove/Electron instances and active builds
block timings; the runner does not close them. A locked or unverifiable Mac
session also blocks every on-screen run; there is no unlock action. It records load, AC power,
disk and OS build before and after each app. Invalid-condition pairs are
retained and excluded explicitly, never silently discarded. Inspect the
desktop/display configuration and settle the machine before a run. Thermal
state, pinned refresh, brightness, quality and required-machine certification
still need an operator; this harness never auto-certifies these conditions.

Default repetitions: 30 independently launched pairs for readiness, open,
cached seek, edit, idle CPU and family footprint; three complete 60-second
CPU-accounting playback intervals per side. The latter count is deliberately
lower because missing physical-frame/GPU/energy endpoints already block those
rows; extra CPU-only repeats do not prove the combined gate. Increase
`--playback-runs` to 30 when those streams are supplied. Idle follows a
three-second settle and samples a two-second interval (both configurable).
These are exploratory defaults; accepted idle/energy results need a justified
stable window and the required rigs. No shortened playback is called 60 s.

Each raw pair contains individual observations, errors and conditions. Results
include sample counts, nearest-rank p50/p95, ratios of matching quantiles,
budgets, failures and missing-evidence reasons. Ratios with a zero reference
denominator are missing. All clauses of a budget are required. The ambiguous
`vsync + 1/2` latency wording is interpreted conservatively as the refresh
interval plus 1/2 milliseconds; physical input/presentation must be established
before that interpretation can accept a result.

## Measurements and deliberate limits

- Native opt-in `PM_PERF_GATE=1` requires a temporary profile. JSON-lines stdin
  commands drive the existing controller's open, seek, edit and transport
  methods. The controller-ready marker is a diagnostic, not first interactive
  frame. `status` reads the existing engine-service FrameReport, exact frame
  ticks/stamp/cache hit, physical drawable presentation, GPU completion and
  Metal device allocation. It adds no frame timer in normal runs.
- Electron uses the shipped `PM`/kernel paths, matching render-event identity,
  then a WebGL fence and next rAF, as the existing 120 Hz rig does. This is a
  compositor opportunity, **not physical display presentation**. Native and
  Electron latency endpoints differ and are never used for a passing gate.
- `proc_pid_rusage` v4 reads the whole process family, including Electron
  helpers. Mach CPU ticks are converted with `mach_timebase_info`; CPU percent
  uses one logical core as 100%. PID churn/reuse invalidates an interval.
  Interrupt and package-idle wakeups are kept separate and never called
  per-extension wakeups. No privileged process access or RSS fallback occurs.
- Family `phys_footprint` sums are diagnostic. They do not prove shared CPU/GPU
  allocations were counted once. Native Metal allocated bytes are separate,
  never added to the footprint. Base and document-delta checkpoints and per-process footprints are retained;
  extension splits and allocation identity accounting remain unavailable.
- `powermetrics` is probed directly without sudo or any prompt. If denied,
  CPU accounting remains available. Native public status has no Metal GPU-time
  interval counters; energy and GPU are missing rather than inferred from CPU
  or machine-wide power. The gate requires all three cost clauses.
- The runner retains the service lane's opt-in `PM_ENGINE_TRACE` in `/tmp` and
  analyzes positive display timestamps, input origins, worker-to-completion
  spans and matched-frame audio host times. Missing timestamps stay missing;
  worker span is never called GPU-only time. A native-only trace has no paired
  Electron device stamps or complete-frame census guarantee. Trace recording
  has observation overhead; these observations remain diagnostic.
- A/V needs matched actual device presentation timestamps. Neither latest-only
  native frame status nor an incomplete trace establishes physical drop census or frame-work
  headroom. The small shape workload does not stand in for a 4K long-GOP seek
  or every public-corpus project. These missing capabilities remain explicit.

Consequently this first harness can collect reproducible diagnostics and reject
bad/incomplete evidence, but **cannot certify Gate P1** yet. See the gate report
for the current run and the exact remaining instrument/API requirements.
