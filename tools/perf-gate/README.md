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

## Diagnostic foreground sessions

When an operator explicitly retains installed apps or accepts high load, pass
`--diagnostic`. Only those two condition blockers may be bypassed for collection.
Power, disk, unlocked-session and active-build checks still stop execution.
Every before/after observation retains app presence and load. Acceptance results
exclude invalid pairs; `diagnostic_results` separately summarizes the observed
numbers and cannot certify the gate.

For an operator-owned whole-session GUI lock, pass `--session-lock-inode` with
its inode. The lock must already contain `onscreen`; identity is checked for each
pair. The caller must retain the lock and release it in its own exit trap. The
runner never removes a caller-owned lock. Ordinary runs keep their per-pair lock
and nine-minute alarm.

Each app receives its own writable project copy for every pair. The prepared
corpus file remains immutable even when native edits save a PMV back to their
source path. Setup seeks skip a frame already displayed by native; the measured
cached seek still changes frame identity and requires a new presentation.
Playback errors retain earlier observations. Service traces accept both JSON
arrays and the service's NDJSON stream; malformed records still fail explicitly.

For real-app service foreground runs use `python3 tools/perf-gate/foreground.py`
with the original runbook arguments. It retains the service harness's guards and
uses Quartz host seconds for window markers. Python 3.9 process-relative
monotonic origins must not be joined directly to NSEvent/Metal host timestamps.

## Native fix diagnostics

`native.py` uses only the release app, disposable `/tmp` documents/profiles and
an owned `perffix` GUI lock. It pins 60/120 Hz through `display.swift`, restores
the exact original display/mode, bounds each batch to nine minutes plus teardown,
and preserves restore failures in the raw result. Existing app/load caveats are
retained; these native-only runs never certify the paired gate.

```sh
source engine/env.sh
export DYLD_LIBRARY_PATH="$PM_FFMPEG_DIR/lib"
python3 tools/perf-gate/native.py --project /tmp/public-shape.json --output /tmp/fix-120.json --hz 120 --runs 30 --seconds 60
```

Use `--audio --runs 1 --seconds 20` with an already linked public audio project
for a loop followed by seek/restart. `--interact --runs 1 --seconds 120` creates
a disposable registered app for manual inspector input; it does not start
playback. It leaves no installed bundle or user profile. Its trace records
missing presentation callbacks without substituting GPU completion.

Native automation origins are explicit command-acceptance host times; real
editor calls retain valid NSEvent origins, with a dispatch-time fallback when
there is no positive native event timestamp. Presented input/revision/viewer
identity must match before a seek, edit or open observation is accepted.
`trace.py` reports processing stages even for GPU-complete rows lacking physical
presentation; latency/A-V/display distributions still require positive display
stamps. The detailed service pipeline is diagnostic and does not establish total
GPUI/main-thread frame work.

## Thirty-pair P1 runner (2026-11-01)

`run.sh`, `driver.py` and `native.py` remain consumers of the exploratory
collectors. The new acceptance entry point is:

```sh
PM_PERF_GATE_CONFIG=/tmp/pm-p1-config.json tools/perf-gate/run-gate.sh p1
```

It writes a table (`report.md`), raw attempts, per-side logs, binary SHA-256s,
conditions and distributions (`report.json`) into a new private `/tmp/pm-gate-*`
directory. `--output` accepts only a new directory under the canonical temporary
root. Existing reports and source projects are never overwritten. Each side has
its own writable project copy and retained temporary profile. No application is
built, installed, published, or deleted by this entry point. Node/Playwright are
reference-test tools only; they do not enter the native application.

Prepare public fixtures and build both binaries **before** the timing session.
The frozen reference must be a clean v1.2.0 checkout, with Playwright installed.
Keep media links absolute and use the same original media on both sides.
The shape project must actually be 60 fps; the video project must use 4K
long-GOP media; the audio project must have device-playable audio. Do not use
personal projects or profiles. Manifest preparation runs no app:

```sh
python3 tools/perf-gate/configure.py \
  --native /tmp/pm-build/powermove-native \
  --electron /tmp/Powermove.app \
  --reference /tmp/pm-ref-v1.2.0 \
  --shape /tmp/public-shapes-60.json \
  --video /tmp/public-long-gop.json --video-media /tmp/public-4k.mp4 \
  --audio /tmp/public-audio.json --hz 120 --rig xdr-120 \
  --output /tmp/pm-p1-config.json
```

The Electron path can be an app bundle or an executable. For an Electron runtime
instead of a packaged Powermove app, add
`--electron-arg /tmp/pm-ref-v1.2.0/apps/desktop`. Native startup must have the
usual FFmpeg environment already configured. The runner hashes the actual
executables and verifies the reference commit; the operator must supply the
unmodified build from that reference, not infer its provenance from a filename.

The helper defaults to **uncertified** conditions. `--certify-rig` and
`--certify-quality --quality-note '…'` record independently verified hardware
and matched brightness, display/HDR, resolution and quality. Pin refresh with
the existing display tooling before timing; this runner only observes it.
The `base-60` rig is the cohort's common base M-series machine. A build Mac at
60 Hz is `stand-in`, explicitly ineligible for gate acceptance. A 120 Hz XDR
Apple-silicon MacBook Pro can fill `xdr-120` per the recorded owner adjustment.
Use `--allow-app <bundle-id>` only for deliberately reviewed system UI processes
such as Finder; the allowed list is retained in the report. No other application
is stopped. An existing instance of the configured Powermove executable remains
foreign; only the measured executable in the runner-owned process group is exempt.

All ten rows request **30 independent native/Electron pairs**, including 30 full
60-second intervals for each playback, playback-cost and A/V row. Order alternates
AB/BA. Both sides use the same scenario hash, edit values, cached seeks and seeded
random-seek target. Each pair has the existing inode-owned GUI lock and nine-minute
alarm; each adapter has a configurable bounded timeout (default 180 seconds).
No timeout removes earlier observations. A broken adapter stops repetitions for
that row, rather than launching 30 times into the same failure.

Quiet checks run before, during (every 0.5 seconds) and after each side. Every
1/5/15-minute load must be at most 0.5 per logical CPU; the threshold may be
tightened. Thermal must be nominal, AC and the unlocked session must be known,
disk must retain 8 GiB, no build may run, and the GUI-app inventory must contain
only the measured app and explicit exemptions. The single display's mode,
resolution, refresh, OS build and CPU count must stay fixed against the baseline.
Unknown thermal/refresh/inventory is a rejection. The Swift condition probe is
compiled once before the quiet window. It never changes a setting or requests
elevation.

Condition-invalid **whole pairs** are retained and excluded. Bounded replacements
default to at most 45 attempts to obtain 30 quiet pairs per row. Timing outliers
use a far Tukey fence (3 IQR, median halves), are flagged by zero-based observation
index and **never removed** from nearest-rank p50/p95 or the gate verdict. A slow
valid pair is never replaced because of its time. Paired comparisons intersect
by pair ID and metric; independently available side distributions remain visible
even when their paired reference is absent.

The owner endpoint decision replaces the old cached/edit physical absolute
bound: report `own_share_ms` (accepted input through GPU completion) and
`on_screen_ms` (the same input through matching `presentedTime`). Own share must
have p95 ≤9.333 ms; native on-screen p50 and p95 must each beat Electron at the
same endpoint. This runner conservatively retains the explicit 9.333 ms own-share
ceiling at both refresh rates. No compositor-floor subtraction is performed.
The edit row also requires ≤4 ms maximum main-thread frame work and shell repaint.
All other clauses remain in `budgets.py`; every required clause and endpoint
must be available on all 30 pairs. FAIL takes precedence over INCOMPLETE for a
known budget breach. Missing is displayed as `—`, never zero.

### Adapter contract and remaining instrumentation

`collect.py` is the bundled adapter. It launches the configured apps with private
profiles and drives the existing PM/kernel commands. Native reports both latency
endpoints when available, retaining GPU completion even if the physical callback
is missing. Electron reports fence completion separately from rAF and explicitly
leaves `on_screen_ms` missing. rAF cannot attest physical presentation. Frame
time/revision/viewer identity checks remain in the existing collectors.

The bundled collector cannot yet certify any row requiring unavailable streams:
first interactive launch, matched open boundaries/main stalls, Electron physical
presentation, verified Electron cache hits, random-seek preview, a complete
physical playback census, total frame work, app-attributed GPU/energy, paired
device A/V, shell repaint, extension wakeups and deduplicated CPU/GPU allocations.
Full native service traces are retained as diagnostics. This tooling **does not
implement or invent those app/OS instrument endpoints**. Mac instrumentation can
replace the adapter via the manifest's `adapter: ["/absolute/executable", "arg"]`.
The same adapter is invoked for both sides, one fresh process per side/row/pair.

It receives one JSON object on stdin: `version:1`, `row`, `side`, `index`, `hz`,
`project` (private writable copy), `scenario` (including prepared source SHA-256),
`apps`, `reference`, `electron_args`, and `scratch`. It returns exactly one JSON
object on stdout, with matching `version`, `row`, `side`, `index`,
`scenario_sha256`, `metrics:{…}`, `evidence:{…}` and optional `diagnostics`.
Diagnostics belong on stderr or under `scratch`. Spawn app children in the
adapter's inherited process group so the runner can exempt/reap only its own
children. A response with the wrong scenario/identity or nonfinite JSON is
rejected. `evidence.py` lists the exact metric and endpoint/census attestations
per row; an attestation must describe measured evidence, never an assumption.
Random media is independently checked with ffprobe (codec/dimensions/keyframe
intervals) before app startup. Unsupported/missing ffprobe blocks that workload;
no codec fallback substitutes a different benchmark.

### Linux verification

Python 3.9+ and Node 22.18+; no app, native library or ffprobe build is needed:

```sh
python3 -m unittest discover -s tools/perf-gate -p 'test_*.py' -v
node --check tools/perf-gate/electron.mjs
sh -n tools/perf-gate/run-gate.sh
tools/perf-gate/run-gate.sh p1 --replay tools/perf-gate/fixtures/recorded-p1.json
```

Replay intentionally exits **2** and labels the report `REPLAY ONLY`. The compact
recording contains **synthetic protocol responses**, not Mac measurements; its
row PASS values only exercise the policy. Three existing Darwin counter/clock
tests skip with explicit reasons on Linux. The Node test uses the vendored
Playwright/PM protocol double; the ffprobe tests inject recorded responses and
do not depend on an installed codec build. Live collection on Linux prints an
INCOMPLETE table and exits 2 before launching anything.
