# Engine memory

P7 remains open. In the current five-pair campaign, combined engine-on physical
footprint exceeds today by 128.3 MiB at initial home, 110.9 MiB with an empty
project, and 139.0 MiB with the generated text corpus. Last-tab cleanup saves
104.6 MiB against its same-build control, but does not close the project-open
gap. Frame/raster/geometry/plan caches and document/shadow/check memory have
separate owners; no project, shader, extension, or render tolerance was removed.

## Process and request lifetime

`NativeEngine.start()` eagerly starts the interactive utility and negotiates
leases, slicing policy, playback state, fonts, renderer ports and the direct
peer channel. Normal startup does not start the job utility. Exports,
validations and agent frames start it on demand. Exports retain process crash
isolation. Moving exports into interactive worker threads would lose that
boundary, and there is no resting job process to remove from the memory gates.
Export QoS policy is unchanged here.

Media facts run on the already-ready interactive utility. Its probe-only
service starts one CPU worker on first use and creates no job GPU device.
Main resolves the asset and mints a short-lived `mediaRead` capability for
that host and session 0. Five earlier real-host runs answered their first
probe in 30.98–45.91 ms, median 43.31 ms, without starting a job process.
These timings exclude interactive-utility startup.

The interactive engine holds that probe service in the same slot as the job
engine's render service, so `jobs.is_some()` no longer means "this is the job
engine". `Engine::has_job_service()` (job process and a service installed)
now guards every job-only path: job submission, cancellation, agent frames,
releasing agent frames waiting for a revision, and the service's Blender
events. Review found the Blender guard missing: the paused viewer's Blender
surfaces went to the probe service instead of the presenter, and Material
Preview drew nothing in the native viewer (`engine-blender.spec.ts`).
`tests/present_host.rs` now sends Blender progress and surfaces to an
interactive engine that has both a presenter and a probe service. It also
checks that ticks and pumps reach no job-only path, that probes still reach
the service, and that agent frames and cancels are refused.

The headless native host (`src/server/native/host.ts`) renders on the job
engine as its main work, so it keeps the job engine resident (`eagerJobs`).
Harnesses that measure agent snapshots during playback start their run the
way an agent does (`AgentJobs.prepareRun`) rather than waiting for a job
utility that no longer starts with the app.

Agent frames retain their separate job host, frozen document revisions,
JPEG behavior and job-slicer playback progress. They do not borrow the
interactive preview as a snapshot shortcut. Opening an agent run begins job
startup early; tools renew a bounded 30 min hold through model-thinking
pauses. Finish, cancellation, owner loss and shutdown release the hold.
An unused job host exits after 60 s quiet, only after checking zero sessions,
in-flight requests, active jobs, probes, frames, compiles and queued work.
Requests racing retirement cancel it or await stop before restarting.
Accepted exports retain their sessions through completion. Cold launch is
still measurable; prewarming overlaps it with agent preparation.

## Viewer lifecycle

Initial home presentation and drawing prewarm remain eager. Full initial
home deferral was rejected after a five-pair +240.7 ms first-presented-frame
penalty; code-only warming added 221.5 ms. Warming a headless document reduced
the median penalty to 44.0 ms, but its maximum 51.7 ms failed the limit.

The selected policy suspends only after a real project has been visited and
the final tab actually closes. A Projects overlay with an open tab does not
qualify. The presentation object, worker, WebGL context and program remain
warm. Suspension hides stale pixels and closes the native channel. GPU drain
precedes shrinking the hidden painter's upload texture and drawing buffer to
1 × 1. Preload import release waits for chrome readers; native headless
retirement proceeds independently, and backing can remain until main and
Chromium release their references. A token prevents a late trim from racing
a resumed presentation.
Reopen renews the surface epoch and channel on the warm worker and painter.
Crash handling remains separate and preserves the last frame.

Closing a viewer channel prepares a detached drawing for the next viewer, as
startup does. Only the final-tab suspension skips this, because it keeps its
own drawing and passes `close({ keepsDrawing: true })`. A suspended viewer
that is later disposed prepares one at that point. An interim revision had
removed the preparation on every close and did not disclose it. With that
change, a viewer created after a built-in viewer reload or update, a service
replacement, or a fork binding its own canvas built its drawing cold (code
alone about 220 ms, by the numbers above). That revision is reverted except
for the suspension case, which the home-closed memory gate measures.

`retireClosedPool` is an optional built-in-channel capability: it checks the
connection, lease and preload launch after real channel headlessness. A
replacement drawing keeps its pool; delayed cleanup cannot retire a new
launch's reused generation. Custom providers retain their close and frame
ownership contracts. The unused legacy compatibility WebGL context remains
lazy, synchronous and stable on first API access. Native readiness checks do
not allocate it. No compatibility API is stubbed.

After 2 s without frames, snapshots or playback, and with pending GPU work
drained, the native presenter drops scratch targets, cached snapshot targets
and staging buffers. When every view is headless and paused, or no sessions
remain, it also releases the composer and device. CPU definition and policy
state survive. The selected last-tab suspension reaches this existing native
release path. A headless document session can remain for captures and clock
state; session count alone is not evidence of GPU retention.

The active renderer presenter is WebGL2 (`painter.ts`), not the dormant
WebGPU backend. At 1496 × 904, its F16 upload texture and F16 drawing buffer
each contain 10,819,072 nominal bytes (10.318 MiB). Unused depth and stencil
attachments are disabled; alpha, premultiplication, F16 format, resolution,
pool count and antialias policy are preserved. Chromium compositor buffers
and driver alignment are additional, unknown allocations. Native four-surface
BGRA storage is 21,753,856 bytes (20.746 MiB), including padded rows; announced
logical pixel storage is 20.636 MiB. These overlapping ownership counters
must not be added to OS physical footprint.

All five candidate homeClosed captures observed no native device/composer,
zero color/depth/working/snapshot/readback storage, zero held imports and
pending drains/fences, and 1 × 1 F16 drawing/upload storage (8 bytes each).
The worker and context survived. Open-state captures held four imported
surface leases and zero local VideoFrames after settling; their allocated
surface pool and painter storage remained live. This rules out a retained queue of live VideoFrames
at those capture instants, not Chromium's internal buffer retention.

## Pipelines and counters

The merged renderer defers 48 render pipelines: 18 2D format/blend variants,
8 compositing, 8 resampling, 10 mask/filter and 4 text paint variants,
including the newly merged BGRA paint variant. `LazyRenderPipeline` uses a
`OnceLock` and one-shot factory; compilation releases captured handles.
Descriptors, shaders, formats and binding layouts are unchanged. ID/source
compute pipelines, shader modules and decode importing remain eager.
Definitions still translate and validate GLSL on the presenter thread before
device creation; the first composer adopts prepared effects without another
translation. Cache keys and shader watchdog policy remain intact.

Opt-in `POWERMOVE_ENGINE_ALLOCATION_INVENTORY=1` reports process-wide
`malloc_zone_statistics(NULL)` blocks, in-use bytes, high-water bytes and
reserved bytes. It covers libmalloc zones, not just Rust, and does not cover
all direct VM, GPU or Chromium allocator-shim allocations. Native Metal
reports include public `MTLDevice.currentAllocatedSize`, recommended working
set and unified-memory information where available. Known textures expose
`MTLResource.allocatedSize` and whether they belong to a Metal heap. The API
does not enumerate all heaps or expose actual resource residency; neither is
inferred. Renderer diagnostics inventory painter sizes, announced pools,
leases, VideoFrames and fences without creating a legacy context.

## Five-pair memory gate

The frozen release addon and prebuilt app were measured in five alternating
rounds of engine-off, same-build cleanup-disabled control (`homeControl`),
and the selected candidate (`after`): 15 launches and 75 states. A one-sample
candidate dry run preceded the batch. Profiles were isolated and removed on
success and failure; hashes stayed fixed. Renderer GC and a 3 s settling
series matched across variants. Electron access was exclusive; CPU load was
not (one-minute load 11.2–16.5, median 12.6). The asset-free generated corpus
has six text layers. This is not a large-project cache benchmark.

`closed` replaces the corpus with an empty project while keeping its viewer.
`homeClosed` actually closes all generated tabs. The latter waits for channel
suspension, import drain and painter trim before capture and asserts native
GPU/pool release. All memory figures below are MiB; variant columns are
medians and deltas are medians of per-round differences, which need not equal
the difference of displayed medians. Combined physical footprint accounts
for shared regions across app PIDs; summed working set is a different measure.
All three required combined-footprint gates fail. The job host is absent in
every resting capture.

| State | Footprint off | Footprint on | Paired Δ | Working set off | Working set on | Paired Δ |
|---|---:|---:|---:|---:|---:|---:|
| idle | 505.1 | 632.6 | +128.3 | 688.9 | 870.2 | +181.0 |
| empty | 714.4 | 820.4 | +110.9 | 716.4 | 889.8 | +173.7 |
| corpus | 725.2 | 864.9 | +139.0 | 724.0 | 1018.2 | +294.4 |
| closed | 713.7 | 865.6 | +151.8 | 726.1 | 1023.3 | +298.1 |
| homeClosed | 532.2 | 633.1 | +102.1 | 723.0 | 1092.0 | +370.7 |

Compared with cleanup disabled on the identical addon/app, homeClosed saves
104.6 MiB combined physical footprint (paired changes −100.1, −104.6, −105.9,
−101.3 and −106.3 MiB) and 6.9 MiB summed working set. Per-process dirty
savings are 31.9 MiB interactive, 56.3 MiB GPU and 1.4 MiB page. GPU-category
medians include 39.4 MiB IOSurface, 10.9 MiB IOAccelerator and 6.3 MiB owned
unmapped graphics; interactive owned unmapped graphics falls 22.4 MiB and
Tag 14 falls 5.0 MiB. Category medians overlap totals and are not additive.
This cut applies after final-tab close, not to the empty/corpus open states.
The control also uses depth/stencil disabled, so that change has no isolated
physical-saving claim in this campaign.

Per-process dirty, graphics dirty, compressed-plus-swapped and thread medians
follow. Graphics is already included in dirty; absent-process zero is an
observed absence. `footprint --swapped` combines compression and swap; every
capture below reported zero for that combined field.

| State | Process | Dirty off / on | Graphics off / on | Compressed + swapped off / on | Threads off / on |
|---|---|---|---|---|---|
| idle | Browser | 138.0 / 93.3 | 0.0 / 0.0 | 0.0 / 0.0 | 45 / 46 |
| idle | GPU | 239.8 / 279.3 | 75.7 / 76.6 | 0.0 / 0.0 | 21 / 22 |
| idle | Network | 7.6 / 7.7 | 0.0 / 0.0 | 0.0 / 0.0 | 13 / 13 |
| idle | Page | 119.3 / 148.1 | 0.0 / 0.0 | 0.0 / 0.0 | 24 / 25 |
| idle | Interactive utility | 0.0 / 96.1 | 0.0 / 17.9 | 0.0 / 0.0 | 0 / 27 |
| idle | Job utility | 0.0 / 0.0 | 0.0 / 0.0 | 0.0 / 0.0 | 0 / 0 |
| empty | Browser | 138.3 / 95.2 | 0.0 / 0.0 | 0.0 / 0.0 | 42 / 43 |
| empty | GPU | 437.9 / 452.7 | 131.0 / 125.6 | 0.0 / 0.0 | 22 / 22 |
| empty | Network | 7.6 / 7.7 | 0.0 / 0.0 | 0.0 / 0.0 | 13 / 13 |
| empty | Page | 124.3 / 151.0 | 0.0 / 0.0 | 0.0 / 0.0 | 24 / 25 |
| empty | Interactive utility | 0.0 / 97.0 | 0.0 / 17.9 | 0.0 / 0.0 | 0 / 25 |
| empty | Job utility | 0.0 / 0.0 | 0.0 / 0.0 | 0.0 / 0.0 | 0 / 0 |
| corpus | Browser | 138.9 / 101.6 | 0.0 / 0.0 | 0.0 / 0.0 | 42 / 43 |
| corpus | GPU | 444.5 / 442.7 | 140.5 / 127.4 | 0.0 / 0.0 | 22 / 21 |
| corpus | Network | 7.6 / 7.7 | 0.0 / 0.0 | 0.0 / 0.0 | 12 / 13 |
| corpus | Page | 127.8 / 165.8 | 0.0 / 0.0 | 0.0 / 0.0 | 28 / 25 |
| corpus | Interactive utility | 0.0 / 126.9 | 0.0 / 24.8 | 0.0 / 0.0 | 0 / 28 |
| corpus | Job utility | 0.0 / 0.0 | 0.0 / 0.0 | 0.0 / 0.0 | 0 / 0 |

macOS GPU charges varied sharply after recent activity: candidate idle GPU
dirty differed from control by roughly −409 to +411 MiB in individual pairs.
No outlier was discarded. The diagnostic sum of per-process minima over the
settling series is not a shared-accounted or necessarily simultaneous app
total and does not replace the failing combined-footprint gate. First-round
vmmap showed only 0.75–0.80 MiB resident utility thread stacks despite
133.5 MiB virtual reservations; virtual reservation is not physical memory.

## Retention and attribution

| Candidate counter, MiB | idle | empty | corpus | closed | homeClosed |
|---|---:|---:|---:|---:|---:|
| Interactive Node heap used | 2.58 | 2.63 | 2.67 | 2.73 | 2.77 |
| Interactive V8 physical | 3.75 | 3.75 | 3.75 | 4.00 | 4.00 |
| Page JS heap used | 16.27 | 16.85 | 17.34 | 17.50 | 17.74 |
| Page backing storage | 9.55 | 9.55 | 9.55 | 9.55 | 9.55 |
| Page Wasm live | 0.01 | 0.02 | 5.18 | 5.09 | 5.09 |
| Page Wasm memory | 1.88 | 1.88 | 7.19 | 7.19 | 7.19 |
| Unique font payload | 0.00 | 0.00 | 10.41 | 10.41 | 10.41 |
| libmalloc in use | 0.24 | 0.24 | 0.24 | 0.24 | 0.24 |
| libmalloc reserved | 24.00 | 24.00 | 24.00 | 24.00 | 24.00 |
| Metal allocated | 36.53 | 36.53 | 45.48 | 45.48 | no device |
| Native color surfaces | 20.75 | 20.75 | 20.75 | 20.75 | 0.00 |
| Painter drawing buffer | 10.32 | 10.32 | 10.32 | 10.32 | 8 bytes |
| Painter upload texture | 10.32 | 10.32 | 10.32 | 10.32 | 8 bytes |

Corpus-to-empty replacement retains +30.88 MiB interactive dirty: +19.05 MiB
Tag 14, +6.08 MiB owned unmapped graphics, +2.88 MiB owned unmapped
non-graphics and +1.55 MiB Tag 16 among its category medians. After true
last-tab close the device is gone, but Tag 14 remains +14.23 MiB above initial
home, alongside retained 10.41 MiB unique font payload. Those measurements
overlap and do not identify an allocation site's exact physical cost.
`CoreTextRegistry` preserves installed-font files, faces, blobs, subsets and
fallback manifests for stable IDs. `clear_project_fonts` cannot reset IDs
already handed to layout/render clients. Any further release needs coordinated
font-generation invalidation and replay, not an uncoordinated map clear.

Chromium's [page allocator tag definitions](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/base/allocator/partition_allocator/src/partition_alloc/page_allocator.h)
assign 253 to PartitionAlloc and 255 to V8, corresponding to macOS
App-Specific Tags 14 and 16. Tag 14 identifies the allocator family here,
not a Rust, font or GPU call site. The libmalloc counter stays about 0.243 MiB
while this region grows, so trimming libmalloc alone does not account for it.
A standalone DHAT probe previously found only 1.40 MiB live Rust allocation
for an empty renderer; DHAT excludes Node/V8, mapped fonts, Metal/CoreText and
GPU surfaces. Instruments failed to attach to that probe.

Page dirty overhead versus today is +29.13 MiB idle, +26.70 MiB empty and
+38.72 MiB corpus, while page JS heap adds just +0.73, +0.69 and +0.49 MiB.
GPU dirty overhead is +42.11 MiB idle, +16.59 MiB empty and −1.72 MiB corpus
in this batch; the earlier +28 MiB estimate is not a stable universal offset.
After last-tab cleanup, page overhead remains +37.08 MiB even though imports
are drained and hidden painter storage is tiny. Page Wasm retains 5.09 MiB
live in 7.19 MiB linear memory, outside this lane's document/cache ownership.
The remaining page/native PartitionAlloc and Chromium compositor/driver
storage are not fully attributed. No claim is made that F16 nominal storage
alone explains OS page/GPU deltas.

A separate tracing launch requested four successful detailed memory dumps
and sampled native heaps at empty, corpus, replacement and homeClosed. The
immediate GUID events contain only four sampled allocator paths per process;
they are not full resident/category measurements. Later rich dumps for the
first two GUIDs arrived 1.6–4.9 s late and crossed subsequent transitions,
so they cannot provide state-specific retention subtraction. The trace is
useful allocation evidence, but does not close the call-site attribution gap.

## Cost of the isolated job renderer

Five matched treatment/control pairs used a ready interactive viewer. Phase
A had no job; B prepared a held, device-less job; C rendered a first 320 px
agent frame; D waited 3 s; E finished and observed job retirement. Controls
waited matching intervals without starting jobs. Differences below subtract
the matching control's phase change. A development-only 3 s idle timeout
bounded retirement; production remains 60 s.

| Increment | Combined physical MiB | Job physical MiB | Job graphics dirty MiB | Summed working set MiB |
|---|---:|---:|---:|---:|
| Ready host, B − A | +17.38 | +17.03 | 0.00 | +74.02 |
| First frame/device/renderer, C − B | +503.58 | +498.02 | +419.23 | +146.47 |
| Rested device/renderer, D − B | +90.19 | +85.19 | +6.25 | +146.80 |
| After retirement, E − A | +6.72 | 0.00 | 0.00 | +4.20 |

The first/rested contrasts include renderer, font, frame and driver work,
not a pure second-device ablation. The job's absolute Metal allocation is
6.64 MiB after rendering; a missing device at B is null rather than zero in
native reports, so a raw numeric Metal subtraction is unavailable. The
job's Tag 14 rises about 71 MiB and remains after settling, whereas Node heap
used grows only 0.04–0.06 MiB and libmalloc in-use grows about 0.01 MiB. The
runtime baseline is much smaller than the transient GPU charge. Retirement
eliminates the job PID and its device. Sharing a device across threads would
require redesigning cross-process export isolation, and removing the absent
job at rest cannot clear P7. Exports therefore remain separate. Further
agent-device sharing needs a measured design preserving frozen revisions,
JPEG output, slicing progress and fault boundaries.

An earlier five-run utility ablation measured 14.814 MiB physical for a plain
Electron utility, 16.564 MiB with addon, 16.658 MiB for an idle interactive
host and 16.783 MiB for an idle job host. V8 physical heap was 3.750 MiB.
Small-heap flags reduced used heap slightly but increased physical footprint
by 0.2–0.3 MiB and sometimes latency; production retains default flags.
System font files use immutable mappings; mutable installed files retain
owned bytes to avoid faults if files truncate. Registry tables share `Arc`
backing. No unsafe allocator purge or arbitrary project heap limit was added.

## Latency and validation

Five alternating control/candidate pairs measured reopening after actual
last-tab close and a 3.5 s pause. Completion means the new view sequence was
drawn, its GPU completion callback fired and a subsequent page animation
frame ran; it is not native arrival or physical display scanout. Candidate
absolute reopen times were 128.9, 120.7, 113.0, 116.1 and 105.1 ms (median
116.1 ms); paired regressions were 41.4, 42.0, 45.4, 29.5 and 25.1 ms:
median +41.4, maximum +45.4. This passes the 50 ms regression bound, not an
absolute 50 ms response bound. Initial-open paired regression was +3.6 ms
median, maximum +12.2 ms; eager initial home remains in place.

In the memory matrix, first empty/corpus snapshots took median 4.0/7.7 ms,
with maximum paired regressions against homeControl of 0.7/4.0 ms. A snapshot
after rest took median 22.1 ms, maximum paired regression 4.5 ms. Job startup
preparation took median 72.64 ms, maximum 77.55 ms; it still exceeds 50 ms if
requested synchronously. After completed preparation, readiness took median
0.0042 ms. First agent frame took median 296.65 ms (maximum 301.19 ms), paired
median −5.41 ms and maximum +30.46 ms against homeControl. Warm frames took
median 5.81 ms, maximum paired regression +0.49 ms. These observations do not
make unprepared cold launch fast or establish all agent-first-use paths.

The release addon/app build passed. Native library tests passed 142/142,
Metal inventory 1/1, pipeline probe 48/48, and clippy denied warnings for
pm-node/pm-gpu. Renderer focused checks passed 164 tests across 17 files,
plus a directly rerun last-tab eligibility test. Svelte check reported zero
errors and one existing warning. Export E2E passed H.264 solid and text plus
alpha ProRes, each 24 frames/one save dialog. Text exports use the native job
engine; the earlier legacy-routing expectation was corrected. Temporary
outputs and profiles were removed. No QoS implementation was changed.

## Reproduction and next work

`scripts/measure-engine-memory.mjs` measures prebuilt app trees. Set
`PM_MEMORY_PROJECT` to an asset-free runtime corpus document and
`PM_MEMORY_OUTPUT` outside the checkout. `PM_MEMORY_SAMPLES=5`,
`PM_MEMORY_VARIANTS=off,homeControl,after`, `PM_MEMORY_CLOSED=1`,
`PM_MEMORY_HOME_CLOSED=1` and `PM_MEMORY_HOME_SUSPEND_GATE=1` reproduce the
current matrix. The homeControl override is diagnostic; production suspension
is enabled by default. Wrap only the actual Electron command with the shared
`pm-electron-lock`; build outside the lock, source `engine/env.sh`, use
`nice -n 15`, and use one Playwright worker. Raw profiles, traces, footprint,
vmmap and operational reports stay outside Git.

The remaining project-open physical gaps are release blockers for P7. Next
work needs symbolized PartitionAlloc call-site attribution, coordinated font
registry generation release, and page/compositor allocation attribution with
matching engine-off trace controls. Actual Metal heap residency remains
unavailable through these public counters. Document and frame-cache cuts
belong with their owning lanes. Existing shaders, formats, compatibility
contracts, crash isolation and initial viewer latency remain requirements.
