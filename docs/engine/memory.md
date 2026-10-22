# Engine startup memory

This area owns process startup, fixed GPU pipeline creation, font-registry
storage, thread startup, and allocator attribution. Frame/raster/geometry/plan
caches and document/shadow/check memory have separate owners. No rendering
capability, cache budget, precision, or parity tolerance changes here.

## Process lifetime

`NativeEngine.start()` starts the interactive utility. The job utility starts
on the first job request or media probe. `ensureReady` coalesces concurrent
starts, waits for negotiation, and retains fatal-load and crash reporting.
Host shutdown rejects startup waiters. Existing startup work still installs
leases, slicing policy, current playback state, font replay, renderer ports,
and the direct peer channel.

Agent frame sessions wait for the startup generation announcement before
entering their cache. Otherwise the announcement could discard the first
cache entry and leave a live session that the next batch cannot reuse.
Export fixtures explicitly request their job utility before asserting its
readiness. Eager startup exists for the process-recovery fixtures and an
unsigned-development measurement control; packaged apps use demand startup.

Automatic retirement after use is still open. Renderer job ports can open
sessions independently of main's requests. A status poll followed by a kill
would race a new session. Retirement needs an atomic quiescence protocol or
an explicit ownership change before it can safely release a used utility.

## Fixed GPU pipelines

The constructors defer 47 render pipelines: 18 2D format/blend variants,
8 compositing operations, 8 resampling variants, 10 mask/filter pipelines
across the compositor and text renderer, and 3 text paint variants.
`LazyRenderPipeline` uses `OnceLock` for the compiled pipeline and a protected
one-shot factory. The first caller consumes the factory; its captured shader,
layout, and device handles are released once compilation finishes. Repeated
calls return the same pipeline. Dropping the renderer releases unused
factories and compiled variants. There is no global device retention.

A separate five-pair ablation kept both utilities eager and changed only the
addon. Interactive working sets fell by 1.44 MiB idle, 1.47 MiB empty, and
1.22 MiB on the text project. Total app working-set changes were a 1.20 MiB
saving idle, a 0.28 MiB increase empty, and a 1.33 MiB increase on the project.
This small per-utility reduction does not establish a total-app improvement;
the larger measured saving comes from leaving the unused job process absent.

All pipeline descriptors, shader sources, render targets, and binding layouts
retain their previous values. ID pipelines, source compute pipelines, shader
modules, and the decode importer are still eager. Device creation in the
presenter and job service was already lazy.

## Allocation attribution

`pm-node/examples/startup_heap.rs` uses DHAT only in its standalone example;
the shipping addon keeps its existing allocator. It creates/drops an
interactive host, a job host with idle lanes, a Metal device, and an empty
renderer on the main thread. It requires an output path outside the checkout.
The measured live Rust allocations were:

| Checkpoint | Live bytes |
|---|---:|
| Interactive host without a viewer | 1,899 |
| After interactive host drop | 1,088 |
| Job host with idle lanes | 5,444 |
| After job host drop | 1,088 |
| Metal device | 89,682 |
| Empty renderer | 1,397,028 |
| After renderer/device drop | 10,976 |

Peak live Rust allocation was 1,450,040 bytes; cumulative allocation was
6,674,351 bytes. These counts exclude Node/V8, native Metal/CoreText
allocations, mapped font files, and GPU surfaces. They do not explain or
replace process working sets. No allocator, stack-size, or decoder-pool
setting was changed based on these counts.

System font files under `/System/` already use immutable file mappings.
Other installed font files are read into owned bytes to avoid faults if a
mutable file is truncated. Registry `files`, `blobs`, and face records share
`FontBytes` payloads through `Arc`. The large TTC copies discussed in the
text notes belong to Level 2's retained split-face pool. Centralizing selected
faces would require preserving original blob/collection-index identity while
using read index zero internally; moving extraction into the registry alone
would retain both representations. This remains a coordinated font follow-up.

## Verification and reproduction

Release probes found zero deferred variants compiled by construction, then
compiled all 47 exactly once. The slowest cold variant took 2.858 ms; all 47
together took 25.453 ms. These are CPU-side pipeline creation durations on
the shared M4 Max, rather than end-to-end app first-frame latency.

The 12 composite tests pass, including a new cold/warm reference comparison
at Float16 and Float32. Existing headless, path-mask, sampling, and GPU-text
checks pass. Host/font tests pass (178); changed Rust crates pass Clippy with
warnings denied. Node TypeScript has 75 unrelated diagnostics and none in the
changed engine files.

`scripts/measure-engine-memory.mjs` runs prebuilt app trees under the Electron
lock. It uses isolated profiles, alternating order, matching renderer GC,
and idle/empty/corpus states. `PM_MEMORY_PROJECT` supplies an asset-free
captured project at runtime. `PM_MEMORY_OUTPUT` must be outside the checkout.
The optional `before` variant requires `PM_MEMORY_BASELINE_ADDON`, an original
addon saved before the change. The after addon is frozen and hashed for the
entire batch. Raw vmmap, footprint, process categories, thread counts, and
latency samples stay outside Git. Profiles are removed in `finally` on
success and failure; interrupt handlers close the active isolated app.

`footprint --swapped` combines compressed and swapped bytes. When its count
is zero, both columns are zero; nonzero counts stay combined rather than
being assigned to one without evidence. Total footprint uses the tool's
combined-process total; summed process working sets are a separate metric.
Instruments Allocations with VM Tracker failed to attach to the standalone
native probe; its error was retained. This is an attribution gap, not a pass.

## Paired app memory gate

Five alternating pairs per state, a generated six-layer text corpus project,
with no media bindings required. The shared machine's 1-minute load ranged
from 25.5 to 47.2. These are observed loaded-machine results; they do not
substitute for the final quiet measurement window.

| State | Today working set MiB | Native before MiB | Native after MiB | Paired saving MiB | Paired after minus today MiB |
|---|---:|---:|---:|---:|---:|
| idle | 708.45 | 961.56 | 885.59 | 75.70 | +176.45 |
| empty | 738.56 | 974.22 | 890.58 | 77.25 | +163.62 |
| corpus | 724.73 | 1086.44 | 1007.91 | 76.77 | +283.95 |

Independent total physical-footprint medians (MiB) were today/native-after:
983.07/1525.37 idle, 1213.05/1661.82 empty, and 766.90/1649.66 corpus.
The native path fails the total-memory target by both reported metrics.
Graphics allocations charged as owned/unmapped memory account for much of
its physical footprint and vary considerably across runs. They are not
identified as Rust heap allocations or font caches.

Per-process dirty-footprint medians below compare today with the changed
native path. Every captured process had **zero compressed and zero swapped
bytes**; the combined OS counter was zero in every sample. Threads are
observed counts, not reserved stack bytes. The absent job utility contributes
zero; the original native utility used about 73.6 MiB of working set and
17.3 MiB of physical footprint, with 22–23 threads.

| State | Process | Today dirty MiB | Native dirty MiB | Today threads | Native threads | Compressed / swapped bytes |
|---|---|---:|---:|---:|---:|---:|
| idle | Browser | 137.55 | 93.05 | 45 | 48 | 0 / 0 |
| idle | GPU | 716.86 | 761.72 | 20 | 21 | 0 / 0 |
| idle | Network | 7.66 | 7.70 | 13 | 13 | 0 / 0 |
| idle | Tab | 118.03 | 141.35 | 25 | 26 | 0 / 0 |
| idle | Interactive utility | 0.00 | 513.85 | 0 | 25 | 0 / 0 |
| idle | Job utility | 0.00 | 0.00 | 0 | 0 | 0 / 0 |
| empty | Browser | 138.00 | 95.05 | 45 | 48 | 0 / 0 |
| empty | GPU | 936.91 | 920.61 | 20 | 21 | 0 / 0 |
| empty | Network | 7.66 | 7.70 | 13 | 13 | 0 / 0 |
| empty | Tab | 122.66 | 143.03 | 25 | 26 | 0 / 0 |
| empty | Interactive utility | 0.00 | 491.45 | 0 | 25 | 0 / 0 |
| empty | Job utility | 0.00 | 0.00 | 0 | 0 | 0 / 0 |
| corpus | Browser | 138.53 | 101.28 | 43 | 44 | 0 / 0 |
| corpus | GPU | 485.11 | 883.77 | 21 | 21 | 0 / 0 |
| corpus | Network | 7.56 | 7.69 | 12 | 13 | 0 / 0 |
| corpus | Tab | 126.33 | 156.39 | 29 | 26 | 0 / 0 |
| corpus | Interactive utility | 0.00 | 534.53 | 0 | 27 | 0 / 0 |
| corpus | Job utility | 0.00 | 0.00 | 0 | 0 | 0 / 0 |

Cold job readiness regressed by a paired median **118.70 ms**, maximum cold
readiness **134.65 ms**, against the 50 ms limit. No-op Electron utility
startup alone took 84.57 ms median in a separate five-pair attribution run.
With the addon, entry occurred at 84.57 ms median, addon loading took 9.45 ms,
and the native idle job host constructor took 0.71 ms (the first constructor
was 8.80 ms). This is a measured runtime/launch cost; moving font or worker
initialization cannot remove the observed no-op startup floor.

The first empty snapshot medians were 4.9 ms before and 5.0 ms after;
first corpus snapshots were 16.7 ms before and 13.4 ms after. Each deferred
pipeline stayed below 50 ms in the separate cold-creation probe. These do
not clear the failing cold utility startup line.

The nine real app lifecycle/transport tests and utility crash/recovery
integration pass. The empty and corpus native viewer states remained on
the engine path; no corpus case or tolerance was removed to produce these
numbers. **P7 remains open.** Safe post-use job retirement, the remaining
interactive process/graphics overhead, and cold-launch latency are required
follow-ups before this policy meets the brief's production bar.
