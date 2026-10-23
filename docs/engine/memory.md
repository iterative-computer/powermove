# Engine startup memory

This note covers process lifetime, fixed GPU pipelines, idle GPU resources,
font-registry storage, and allocation attribution. Frame/raster/geometry/plan
caches and document/shadow/check memory have separate owners. Memory cuts must
preserve existing projects, extensions, shaders, and render tolerances.

## Process and request lifetime

`NativeEngine.start()` starts the interactive utility. It installs leases,
slicing policy, playback state, font replay, renderer ports, and the direct
peer channel. `ensureReady` coalesces starts and waits for negotiation;
fatal-load and crash recovery remain supervised. Normal app startup does not
start the job utility. Exports, validations, and agent frames start it on
demand. Its render, export, user job, probe, and compile lanes stay isolated
from the interactive presenter.

Media facts run on the already-running interactive utility. Its probe-only
service has no job render lanes or GPU device, and starts one CPU worker on
first use. Main resolves the asset and mints a short-lived `mediaRead`
capability for the interactive host port and session 0; the same host resolves
that handle. Concurrent requests for one asset keep the newest pending
interpretation. Agent frames still use the separate job utility and GPU gate.

An unused job utility has a 60 s quiet timer. Before retirement, main checks
its status for zero sessions, in-flight requests, active jobs, probes, frame
runs, compiles, and queued work. Requests arriving during the status check
cancel retirement or wait for a completed stop before starting a new
utility. A busy or failed check retries after another quiet interval.
Active project agent runs hold the started utility through model-thinking
pauses: the bridge begins startup when a run opens, tool calls renew a
bounded 30 min hold, and finish, cancellation, owner loss, or shutdown
releases it. The ordinary 60 s countdown then resumes. Accepted async
exports retain their own sessions until completion. Agent frame sessions
close after their last in-flight batch when the run ends; window closure
also closes sessions. A direct cold job call still pays utility startup;
early run preparation overlaps it with agent preparation.

Five real utility-host integration runs answered a first media probe in
30.98–45.91 ms (median 43.31 ms), with no job utility started. These timings
start after the eagerly launched interactive utility is ready; they do not
include app startup or claim that direct cold utility launch became faster.

## Viewer lifecycle and compatibility

The native viewer keeps its original initial drawing prewarm and eager home
presentation. Closing a viewer no longer starts an unclaimed replacement
drawing. The unused legacy WebGL context is allocated synchronously on first
compatibility access; repeated access returns the same real context and
keeps the first requested canvas attributes. Native frame/status/boot paths
avoid touching that compatibility getter merely to test readiness.

Final import cleanup belongs to the built-in native channel. An optional
`retireClosedPool` capability authorizes retirement only after a current
channel actually posts headless state; it also checks the connection, lease,
and preload launch when cleanup runs. Presentation waits for submitted GPU
work and chrome readers before invoking it. A replacement drawing keeps its
pool; a restart keeps the last frame; a delayed cleanup cannot retire a new
launch's reused generation. Custom providers retain their existing `close`
and frame-source contracts and ownership.

Full home-viewer deferral increased first presented-frame latency by a
paired median 240.7 ms over five runs. Loading only worker code still added
221.5 ms. Keeping the drawing and headless document warm reduced that
penalty to 44.0 ms, but one of five pairs was 51.7 ms slower. Native frame
arrival alone was within 19.6 ms; that does not substitute for presentation.
The option was rejected. Its five reopen penalties after a 2.5 s home pause
were −4.5, −13.3, +3.1, +11.5, and +18.9 ms. Those narrow teardown results
do not clear the failing first-open gate or establish a home-memory saving
for the selected eager policy.

## Fixed pipelines and GPU at rest

The constructors defer 47 render pipelines: 18 2D format/blend variants,
8 compositing operations, 8 resampling variants, 10 mask/filter pipelines,
and 3 text paint variants. `LazyRenderPipeline` uses `OnceLock` and a
protected one-shot factory. Compilation releases captured shader, layout,
and device handles; repeated calls reuse the result. The renderer releases
unused factories and compiled variants when dropped. Pipeline descriptors,
shader sources, render targets, and binding layouts retain their values.
ID/source-compute pipelines, shader modules, and the decode importer remain
eager. Presenter and job-service devices are created lazily.

Session definition updates translate and validate GLSL on the presenter
thread even before a device exists. The first composer adopts those
prepared effect sets, including per-owner watchdog state and translation
failures, without translating them again. Shader-cache keys, parity
configuration, custom mesh definitions, and GPU program creation keep their
existing behavior. Staging ownership ends on transfer or the last pre-GPU
session close; it does not retain another cache after the composer drops.

After 2 s without a frame, snapshot, or playback, and when no readback or
view work is pending, the presenter drops idle intermediate targets, viewer
scratch textures, its cached snapshot attachment, and staging buffers.
Exported surface pools remain live. With no sessions, or with every view
headless and paused, it also drops the device and composer after all preview,
snapshot, readback, completion, and shader-watchdog work has drained. CPU
definition state and policy survive; device-bound programs and query objects
do not. A native reattachment diagnostic took 19.025 ms, and pixel tests
verified a fresh pool generation with identical output. A native test with a 1496 × 904
viewer measured Metal allocated bytes at 113,213,440 before rest,
101,744,640 after rest, and 113,213,440 after resuming: exactly 11,468,800
bytes reclaimed and recreated for the scratch texture. The exported
IOSurfaces, pool generation, and resumed pixels were identical. Resuming
took 8.575 ms in that diagnostic. Resource counts and Metal allocation
counters remain distinct from macOS process footprint.

The measurement harness's `closed` state replaces a corpus project with an
empty project through `replaceProject`; its viewer session remains open.
It measures what corpus content leaves behind, not actual session closure.
The optional `homeClosed` state actually closes the generated tabs after
replacement and waits at home. The transport can retain a headless document
session for captures and clock state, so closing the last tab does not prove
the native session count is zero. The allocation inventory separately shows
whether the viewer pool, device, and composer were released. Keep these
transitions distinct in tables.

With the selected eager policy, Projects home can keep the viewer mounted.
Last-tab close therefore does not automatically trigger final import
retirement or headless device retirement. These changes reclaim resources
when that presentation really detaches; they do not claim that every visit
to Projects clears the native device.

The separate cold pipeline probe compiled all 47 variants once; its slowest
variant took 2.858 ms and all variants together took 25.453 ms on the shared
machine. Those are CPU-side compilation durations, not app first-frame
times. The cold/warm Float16 and Float32 composite comparison and existing
headless, path-mask, sampling, and GPU-text checks cover render parity.

## Attribution and remaining boundaries

A five-run standalone Electron utility ablation measured these medians with
default V8 flags. `plain` is a no-op utility, `addon` loads the native addon,
and the final rows construct their idle hosts.

| Utility | Physical footprint MiB | Ready ms |
|---|---:|---:|
| Plain Electron utility | 14.814 | 108.15 |
| Addon loaded | 16.564 | 115.82 |
| Interactive host | 16.658 | 101.65 |
| Job host | 16.783 | 109.12 |

The plain utility's V8 physical heap was 3.750 MiB, with 2.542 MiB used.
The addon and idle-host increments are much smaller than the entire utility.
The no-op launch floor exceeds the 50 ms first-use target on this loaded
machine, so moving native initialization cannot clear a direct cold job
call. Testing `--initial-old-space-size=4` alone and with
`--optimize-for-size --max-semi-space-size=1` reduced used heap to about
2.47 MiB but increased physical footprint by about 0.2–0.3 MiB across
utility variants; some ready times also rose. Production keeps default V8
flags. No arbitrary heap limit was applied to user projects. Optional
`POWERMOVE_ENGINE_RUNTIME_MEMORY=1` adds Node `process.memoryUsage()` and V8
heap statistics to status reports for attribution; ordinary reports do not
sample them. These counters do not measure native Metal allocations.

An earlier five-pair app ablation attributed about 17.0 MiB of macOS
physical footprint and 73.6 MiB of summed working set at idle to the
previously resident job utility. That component cut does not establish
app-level parity. The final comparisons below use the selected eager viewer
and the same on-demand job policy on both native-addon variants.

One 1496 × 904 inventory contained four BGRA IOSurfaces totaling 21,753,856
bytes and 10,819,072 nominal scratch bytes. Of 49,774,592 Metal allocated
bytes, 17,201,664 were not assigned to individual objects by that inventory.
Texture alignment makes actual scratch allocation differ from its nominal
pixel length. None of these counters alone identifies the process's system
graphics charge or its complete physical footprint.

macOS `footprint` charges owned/unmapped graphics allocations to processes,
including the GPU process. This charge can vary sharply after GPU activity
and dominate combined physical footprint while working set changes little.
It is not identified as Rust heap, V8 heap, or a font cache. Keep OS
combined-process physical footprint separate from summed per-process
working sets. `footprint --swapped` combines compressed and swapped bytes;
zero means both are zero, while a nonzero value cannot be split without
further evidence. A fixed settling delay alone does not prove quiescence.

The standalone DHAT example (which does not change the shipping allocator)
found 1,899 live Rust bytes for an interactive host without a viewer, 5,444
for a job host with idle lanes, 89,682 for a Metal device, and 1,397,028 for
an empty renderer. Peak live Rust allocation was 1,450,040 bytes. DHAT does
not include Node/V8, Metal/CoreText allocations, mapped fonts, or GPU
surfaces, and cannot explain app footprint alone. An Instruments
Allocations/VM Tracker attempt failed to attach to the standalone native
probe; that remains an attribution gap.

System fonts under `/System/` use immutable file mappings. Other installed
fonts remain owned bytes so truncating a mutable font file cannot fault a
running app. Registry `files`, `blobs`, and face records share `FontBytes`
through `Arc`; the larger retained split-face TTC pool belongs to the text
lane. Further deduplication must preserve original collection-index identity
and avoid holding both split and source representations. Renderer WebAssembly
heap, bridge backing storage, document/shadow state, and frame caches have
separate owners and remain visible in app-level status; this lane's host and
pipeline cuts do not claim to eliminate them. No experimental renderer
variant is counted as shipped here.

`CoreTextRegistry` also retains installed-font files, face records, blobs,
subsets, and fallback manifests for its generation. `clear_project_fonts`
clears project faces and sources; it does not invalidate the face IDs and
blob keys already handed to layout and render clients. A reset therefore
needs coordinated font-generation invalidation and replay across those
clients, rather than removing a map while its stable IDs remain in use.

## Validation

The native library's 141 tests and seven watchdog tests passed; clippy passed
with warnings denied for the native and render libraries. Renderer lifecycle
and compatibility checks passed 75 focused tests. Svelte check reported no
errors and one existing warning. Broader TypeScript checks still encounter
unrelated existing project-configuration and application errors.

The export E2E passed solid and text H.264 MP4s and alpha ProRes, each with
24 frames and one save dialog. MP4 encoder metadata confirmed native export
for text as well as solids; temporary outputs were removed. The same test
verified that first compatibility access creates a real, stable WebGL
context in 3.6 ms, below the 50 ms limit. This is one first-use observation,
not a five-pair launch gate.

## Reproduction

`scripts/measure-engine-memory.mjs` runs prebuilt app trees under the
Electron lock with isolated profiles, alternating order, matching renderer
GC, and idle/empty/generated-corpus states. Set `PM_MEMORY_PROJECT` at
runtime for an asset-free captured project, `PM_MEMORY_OUTPUT` to a directory
outside the checkout, and `PM_MEMORY_BASELINE_ADDON` when comparing the
original addon. `PM_MEMORY_CLOSED=1` enables the empty replacement capture;
`PM_MEMORY_HOME_CLOSED=1` additionally closes generated tabs for the home
capture. The runner removes profiles in `finally`; raw vmmap, footprint,
process counts, thread counts, and latency artifacts stay outside Git.

## Final five-pair gate: P7 remains open

The frozen app used the eager home viewer, initial drawing prewarm, default
V8 flags, and on-demand jobs. Twenty launches covered five alternating rounds
of engine-off, baseline addon/current renderer, forced compatibility GL,
and the selected addon/current renderer. Each captured all five states.
Profiles were isolated and removed; matching renderer GC preceded a fixed
3 s capture series. Artifact hashes stayed unchanged. Electron was reserved
exclusively; machine CPU load was not exclusive (one-minute load 8.7–19.1,
median 11.8). The generated corpus has six text layers and no media assets.
It does not establish large-project cache or document-memory behavior.

All memory figures below are MiB. Each variant column is its observed
median; each delta is the median of five per-round differences, so it need
not equal the difference of the two displayed medians. Combined physical
footprint accounts for shared regions across the app's PIDs. Working set is
a sum and is not an interchangeable measure. All three required footprint
gates fail.

| State | Footprint off | Footprint on | Paired Δ | Working set off | Working set on | Paired Δ |
|---|---|---|---|---|---|---|
| idle | 509.4 | 1041.4 | +513.5 | 688.6 | 867.7 | +177.4 |
| empty | 714.1 | 825.0 | +116.1 | 713.2 | 886.9 | +171.8 |
| corpus | 723.4 | 869.4 | +151.3 | 720.4 | 1014.4 | +294.0 |
| closed | 711.3 | 874.9 | +167.8 | 722.0 | 1019.2 | +297.2 |
| homeClosed | 526.1 | 751.6 | +222.2 | 725.7 | 1094.7 | +369.0 |

The idle combined delta is dominated by variable GPU graphics charges.
Even the diagnostic sum of per-process minima over the settling series stays
above engine-off: +128.8 MiB idle, +110.3 empty, +146.5 corpus, +163.1 after
replacement, and +204.2 after last-tab close. Those minima can come from
different instants and are not a simultaneous or shared-accounted app total;
they cannot replace the failing combined-footprint gate.

Per-process dirty, graphics dirty, compressed-plus-swapped, and thread
medians follow. Graphics is already included in dirty. Zero contribution
for a missing process is distinct from a resident process of that size.
The job utility was absent from every memory capture.

| State | Process | Dirty off / on | Graphics off / on | Compressed + swapped off / on | Threads off / on |
|---|---|---|---|---|---|
| idle | Browser | 137.5 / 93.7 | 0.0 / 0.0 | 0.0 / 0.0 | 45 / 47 |
| idle | GPU | 243.4 / 689.9 | 75.7 / 490.2 | 0.0 / 0.0 | 22 / 22 |
| idle | Network | 7.7 / 7.7 | 0.0 / 0.0 | 0.0 / 0.0 | 13 / 13 |
| idle | Page | 118.8 / 146.7 | 0.0 / 0.0 | 0.0 / 0.0 | 25 / 25 |
| idle | Interactive utility | 0.0 / 96.0 | 0.0 / 17.9 | 0.0 / 0.0 | 0 / 27 |
| idle | Job utility | 0.0 / 0.0 | 0.0 / 0.0 | 0.0 / 0.0 | 0 / 0 |
| empty | Browser | 137.8 / 95.7 | 0.0 / 0.0 | 0.0 / 0.0 | 43 / 45 |
| empty | GPU | 439.7 / 458.0 | 131.3 / 133.3 | 0.0 / 0.0 | 22 / 21 |
| empty | Network | 7.7 / 7.7 | 0.0 / 0.0 | 0.0 / 0.0 | 13 / 12 |
| empty | Page | 122.2 / 149.6 | 0.0 / 0.0 | 0.0 / 0.0 | 25 / 25 |
| empty | Interactive utility | 0.0 / 96.9 | 0.0 / 17.9 | 0.0 / 0.0 | 0 / 25 |
| empty | Job utility | 0.0 / 0.0 | 0.0 / 0.0 | 0.0 / 0.0 | 0 / 0 |
| corpus | Browser | 138.4 / 101.3 | 0.0 / 0.0 | 0.0 / 0.0 | 43 / 44 |
| corpus | GPU | 444.3 / 453.3 | 140.4 / 135.1 | 0.0 / 0.0 | 21 / 21 |
| corpus | Network | 7.6 / 7.7 | 0.0 / 0.0 | 0.0 / 0.0 | 12 / 13 |
| corpus | Page | 126.2 / 165.1 | 0.0 / 0.0 | 0.0 / 0.0 | 29 / 26 |
| corpus | Interactive utility | 0.0 / 127.0 | 0.0 / 25.8 | 0.0 / 0.0 | 0 / 28 |
| corpus | Job utility | 0.0 / 0.0 | 0.0 / 0.0 | 0.0 / 0.0 | 0 / 0 |

### Measured component cuts

The addon control uses the baseline addon with the same current renderer
and on-demand jobs. The GL control uses the selected addon and forces the
real legacy compatibility context without changing native rendering.
Every captured state asserted the expected GL allocation state. Negative
values below indicate savings; these diagnostic cuts do not clear the
engine-off target and should not be added to each other.

| State | Addon footprint Δ | Addon working-set Δ | GL footprint Δ | GL working-set Δ |
|---|---|---|---|---|
| idle | -9.0 | -1.8 | -11.1 | -2.3 |
| empty | -15.0 | -2.1 | -12.1 | -5.1 |
| corpus | -16.7 | -3.0 | -12.8 | -4.9 |
| closed | -17.1 | -2.6 | -12.8 | -4.7 |
| homeClosed | -17.0 | -5.3 | -5.2 | -6.0 |

The native cut consistently removes about 10.94 MiB of interactive owned
unmapped graphics, matching the scratch-allocation release; its interactive
total dirty cut is about 12.5–13.2 MiB. Deferred compatibility GL removes
about 11–13 MiB of total footprint in the first four states, mostly in the
GPU process. It does not remove the remaining page or GPU overhead.
Paired page dirty overhead is +27.8 MiB idle, +27.3 empty, and +38.7 corpus;
paired GPU dirty overhead is +23.6 MiB empty and +9.7 corpus. Idle GPU dirty
adds +427.9 MiB in the combined captures, including the variable OS graphics
charge. Page JS heap adds only +0.41, +0.68, and +0.48 MiB respectively;
that counter does not explain the rest of page dirty memory.

### Retention and allocation counters

| Counter (on) | idle | empty | corpus | closed | homeClosed |
|---|---|---|---|---|---|
| Interactive Node heap used | 2.58 | 2.63 | 2.67 | 2.72 | 2.75 |
| Interactive V8 physical | 3.75 | 3.75 | 3.75 | 4.00 | 4.00 |
| Page JS heap used | 16.25 | 16.82 | 17.32 | 17.48 | 17.72 |
| Page backing storage | 9.53 | 9.54 | 9.54 | 9.54 | 9.54 |
| Page Wasm live | 0.01 | 0.02 | 5.18 | 5.09 | 5.09 |
| Page Wasm memory | 1.88 | 1.88 | 7.19 | 7.19 | 7.19 |
| Unique font payload | 0.00 | 0.00 | 10.41 | 10.41 | 10.41 |
| Metal allocated | 36.53 | 36.53 | 42.56 | 42.56 | 42.56 |
| Color IOSurfaces | 20.75 | 20.75 | 20.75 | 20.75 | 20.75 |
| Working targets | 0.00 | 0.00 | 0.00 | 0.00 | 0.00 |

All selected captures reported a live device and the same 20.75 MiB color
surface pool. Rest-release counts advanced from one at idle to five after
last-tab close; working targets, snapshot targets and readback buffers were
zero at each capture. This proves those idle targets retired, not that the
device or exported imports were freed at home.

Corpus-to-empty replacement retains a paired +31.17 MiB of interactive
dirty memory and +12.61 MiB of page dirty memory. Interactive category deltas
include +18.45 MiB in App-Specific Tag 14, +7.52 MiB in owned unmapped
graphics, +2.88 MiB in owned unmapped non-graphics, and +1.53 MiB in Tag 16.
The OS tags do not identify the underlying allocator. The font inventory
retains 10.41 MiB of unique payload and page Wasm retains about 5.09 MiB live;
these counters can overlap dirty categories and must not be added to them.
A coordinated font-generation reset and the document/renderer owners are
still needed to close this retention gap. First-round vmmap stack estimates
were 0.75–0.78 MiB resident despite 133.5 MiB reserved virtual
ranges; virtual stack reservations are not equivalent to resident memory.

### First use and agent readiness

| Selected timing | Median ms | Maximum ms | Paired median / maximum change from baseline addon ms |
|---|---:|---:|---:|
| First empty snapshot | 4.00 | 4.40 | 0.00 / +0.20 |
| First corpus snapshot | 7.30 | 14.50 | +0.60 / +1.20 |
| Snapshot after fixed capture delay | 24.40 | 25.50 | −2.10 / +3.90 |
| Job utility preparation | 79.89 | 86.75 | +1.27 / +20.56 |
| Job readiness after completed preparation | 0.0044 | 0.0055 | +0.0005 / +0.0007 |
| First agent frame after preparation | 300.12 | 353.28 | −17.28 / +66.78 |
| Warm agent frame | 6.01 | 10.12 | −0.22 / +3.32 |

Five forced compatibility first accesses took 5.2–6.6 ms (median 6.0 ms).
Snapshots and first compatibility access remain within the measured 50 ms
regression limit. Direct cold job startup still takes about 80 ms;
preparation moves that wait before use rather than reducing launch cost.
First agent-frame rendering also includes fresh renderer/font/pipeline work.
Although its paired median improved, one pair regressed by 66.78 ms, so this
batch does not clear an unrestricted first-use latency gate. CPU load alone
is not evidence that this pair may be discarded. Neither the memory target
nor the complete latency target is claimed as passed.
