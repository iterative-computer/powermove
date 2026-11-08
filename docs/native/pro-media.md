# P5 pro media and colour (package 21)

Worker implementation: `apps/native/crates/promedia` (`pm-promedia`), portable
`engine/crates/pm-media/src/proxy.rs` and `cache_frame.rs`, and the existing
`engine/crates/pm-color/src/lut.rs` / `scopes.rs` sources. Real Linux software
encodes and exact source frames drive the job/cache/LUT/scopes paths. Native
composition-renderer wiring, VideoToolbox, UI, platform performance and cutover
acceptance remain unverified. The final slice-2 evidence below supersedes the
historical first-slice counts and lockfile limitation.

Read against `pro-editing.md`, W1's pro-media additions and W019/W020, D-STATE's
original/proxy-only/offline closure, the kernel single-writer contract,
`engine-service.md`, and P0's V8-logic/WebKit-DOM architecture decision.
The v1.2.0 `main/media-proxy.ts` preserves alpha, timing and optional audio;
its VP9/Opus choice addresses Chromium decoding and is not a native codec rule.
No Electron code is removed and no extension contract changes.

## In the app (wave 16)

`app/src/pro_media.rs` runs `pm_proxy::generate` on one worker thread into
this Mac's media storage (`pm_promedia::storage`, default
`<profile>/Media Storage`): proxies at Settings › Media's size and format,
optimized media as full-size ProRes 422 HQ (4444 for alpha or deeper than 10
bits). An index (`<profile>/Pro Media/index.json`) keys renditions by the
original's content (the media cache's object name, or a BLAKE3 of the file
hashed once per stamp on the pro media threads), so a touched or byte-identical
copy shares them and an offline original keeps its proxy through the project's
asset. Clear… never removes a file another original, an attached record, a
running task or the open project uses; replaced or late renditions are
retired, never left behind. Probes, hashes, deletes and settings writes run
on the pro media file thread, never the main thread. Reachable from:

- Project panel context menu on a video or a selection: Create, Rebuild,
  Delete Proxy Media; Attach Proxy Media… (frames, rate, duration, start and
  timecode, display aspect, alpha, audio and colour must match; a finishing
  task never replaces an attached proxy); Create, Delete Optimized Media; Always Play Original
  (checked). Rows show the state as quiet meta text with creation progress.
- Window › Background Tasks (⌘9): exports and media tasks with frames,
  Pause/Resume, Cancel, and a priority on right-click (an NSMenu, plan
  section 1; the list keeps queue order). An export row offers Cancel, which
  stops its encode; pause and priority are for media tasks. A paused task's encode stops and
  starts again on Resume. An encode waiting on pm-proxy's disk guard (keeps
  max(20 GiB, 10%) free) or on playback says so.
- Settings › Media: Create Proxies (When Needed: larger than 1080p, or
  deeper than 8 bits at 4K; For All Video; Only When I Ask) applied on
  import, Proxy Size (Automatic, Full, Half, Quarter, 1080p, 720p), Proxy
  Format; Storage: Location (Change…, refused unless empty or Powermove's;
  existing files stay; the next task writes there), Space Used, Clear… (keeps originals, attached proxies
  and the open project's media), Show in Finder. Kept in
  `<profile>/Pro Media/settings.json`: the v1.2.0 profile keys are unchanged.
- The viewer: Automatic plays optimized media in place of originals, Proxy
  Preferred and Proxy Only read proxies; Always Play Original wins in every
  mode. Movie exports read originals unless Export › Use Proxies (Draft);
  a still export always reads originals.

Not reachable yet: the disk render cache, its timeline render bar and Keep
Rendered (the `pm_promedia::cache` model has no engine writer or timeline
painter), and per-project storage locations.

## Decisions and selecting rules

| Decision | Rule and reason |
| --- | --- |
| Originals retain their identity and interpretation; proxy and optimized relationships are separate records. Proxy-only and already-offline records can be registered. | N3/N4: never promote a proxy into an original or lose existing playable bytes. |
| Match time base, frame rate, start, duration, alpha, audio and colour interpretation before attaching; optimized media keeps full dimensions. Relink requires matching content hashes. | N1/N3: preserve the authored source and exact timing rather than guessing by filename. |
| Worker outputs name their project, media, source, profile, task and catalog lifetime. Reject stale work and coordinated completion during pause/cancel. | N3/N7: late work cannot alter another project or a reopened catalog. |
| Priority/FIFO queue with cooperative control acknowledgements, monotone progress, window events and snapshot recovery after compaction. | N1/N2/N7: inspectable Background Tasks, accurate activity and no silent dropped state. |
| Disk frame identity includes semantic graph and media digests, exact ticks, dimensions, representation and renderer version. Checksummed immutable publication never replaces differing bytes. | N1/N3: no stale frame after an edit, relink, colour or implementation change. |
| Keep Rendered survives reopening and quota reductions; insufficient unkept capacity refuses. | N1/N4: explicit retention wins over automatic eviction. |
| Purge checks the full reference closure and archives all media classes, including originals. Verify and sync the archive before unlinking active bytes. | N4: never delete original or proxy-only user data; interruption may leave extra copies. |
| Refuse foreign/nonempty roots, traversal, symlink paths and overlapping locations; do not silently move or adopt existing storage. | N4: settings cannot cause unrelated files to be overwritten or collected. |
| LUT input-domain clamping is explicit; output values remain extended. Scopes require an explicit matrix and analysis stage. | N1/N3: standards and authored colour intent, no implicit display transform or double tone map. |
| Pure Rust models and injected native decoder/encoder/probe traits; all IO/codecs run on workers. | N8: no Node/Chromium or main-thread JS wait; no platform emulation claim. |
| Keep module exports, the native lockfile, host wiring and persistence adapters out of this lane. | Owned-path boundary and N4 prune rule: describe integration needs without modifying another lane or retiring live consumers. |

## Media and background task contracts

`MediaCatalog::register` accepts original identity and authored video metadata
without requiring an online original. Attach/detach returns prior rendition
records to the caller; it never deletes their files. `Artifact::inspect` probes a
regular file and streams BLAKE3 hashing. Hold the source's native media lease
while probing/hashing; metadata is not a substitute for byte identity.
`Profile` codec/container names are adapter requests, not codec capability claims.

`begin` creates a per-kind job only from an online original; two rendition kinds
can run independently. Job filenames are safe BLAKE3-derived names with container
extensions. The original path and original hash remain unchanged. Finishing,
manual attach, detach, source-offline and relink operations fence outstanding
work as appropriate. `status` exposes original/proxy-only/optimized-only/offline,
per-rendition availability, jobs, failures and the missing-original export warning.
`resolve(id, None)` requires the original. Preview/approved derivative export
must explicitly request a rendition; unavailable renditions fall back to an
online original. Missing sources remain diagnostics, never project-open failures.

The initial typed admission boundary is video with optional audio; audio-only,
stills and sequences continue through their existing import services. This crate
does not replace the project reader or reject documents containing other media.
Preserve unknown project/import fields in the existing kernel/media store.

`BackgroundTasks` is an execution model, not a thread pool. The host chooses worker
concurrency and calls `start_next`; higher priority wins, with FIFO task id ties.
Active workers are not forcibly preempted. Pause/cancel requests stay visible
until resources are released and `settle_control` acknowledges them. Completion
cannot race through a pending control request. A resumed task gets a fresh ticket;
a replacement queue/catalog has a fresh process-local lifetime. Old worker tickets
and jobs are fenced even if project/task ids are reused in the same process.

`events(window, cursor)` returns only that window's tasks. The global cursor
advances over other windows too. The last 1,024 events are retained; a stale cursor,
a future cursor or explicit terminal-task retirement requests a full window
snapshot. The host archives terminal task facts before `retire`; live tasks cannot
be retired. A cursor alone is not durable recovery state across process restarts.

Preferred success path, on the kernel actor: inspect the worker output, then call
`MediaCatalog::complete_task`. It validates job, task kind/project, ticket and
output before publishing success and attaching. `complete` is the lower-level
model operation for hosts that have already accepted the live task's completion.
Pause/failure/cancel settlement and orphan output archival remain host duties.
Never acknowledge cancellation while a codec still owns the output.

`Decoder`, `Encoder` and `MediaProbe` are platform traits. `transcode` holds one
sample at a time, preserves video/audio sample order and timestamps, validates
sample shapes/finite values and checks controls before and after decoding.
Pause/cancel/error never finalizes a partial output. A resume must use an adapter
checkpoint or restart into a new private output; it cannot append a replayed
stream silently. A macOS adapter performs requested scaling/encoding and verifies
output metadata before attachment. The test double copies synthetic samples;
it is not a production encoder.

## Disk cache and storage

`RenderCache` uses a dedicated marked directory. The caller supplies a complete
semantic graph digest including effects, LUT bytes, colour, fonts, sources and
versions, plus the selected media digest. This package does not invent a graph
hash from an incomplete document revision. Different bytes for one existing key
refuse rather than overwrite. File headers carry magic, payload length and BLAKE3;
reads reject truncation/corruption. Entries have a 1 GiB safety bound.

Publication writes a unique private partial file, syncs it, creates the final
name without replacement using a same-directory hard link and syncs the directory.
A failure may retain a partial file for inspection; partials are never cache hits
or automatic eviction candidates. Cache ownership is exclusive under the host's
profile lock, including read/keep/trim. This is not a concurrent multi-process
cache or a defense against hostile mutation of its private root during IO.

`keep_rendered` creates a durable pin marker. `trim_to` validates a full eviction
plan before removing only unkept derived `.frame` entries, oldest modified first
with path tie-breaking. It refuses when kept bytes exceed the target. Kept frames
remain readable after reducing quota. Capacity counts frame headers and payloads;
small pin/marker metadata and preserved partials are outside cache capacity.
`render_bar` produces coalesced exact half-open tick ranges and rejects overlap.
`render_bar_frames(segments, rate)` is the shared display conversion: pass the
composition's `pm_time::Rate`, never the source or nominal rounded frame rate.
Each frame belongs to the segment containing its exact engine frame-start tick;
end frames are exclusive and intervals containing no frame start disappear.
Views must use this helper rather than convert ticks ad hoc.

`state()` must receive the **current** complete graph/media key. Changing the
semantic graph digest immediately reports `Missing` (or `Rendering` for a job
for that new key), even before old frames are trimmed and while old pins remain.
The kernel/host must recompute the bar on graph publication before presenting it;
a previous bar snapshot is not valid for a new graph. The host supplies rendering
state only for outstanding jobs matching the current key. This implements review
item 2's immediate-`Missing` alternative without a redundant stale state.

`StorageSettings` has separate absolute, nonoverlapping originals/proxies/
optimized/renders locations and an archive location, with per-class active byte
quotas. Settings/record types serialize for the kernel's existing persistence
route; this package does not create a second project/settings authority.
`Storage::render_cache` opens a dedicated child of the renders location; cache
trim can never target originals merely because they have a `.frame` extension.
`usage` includes nested payloads; `put` refuses over quota instead of pruning.
Lowered quotas can report overage without deleting data. Space reservations and
physical-volume free-space checks belong to the host before starting a codec.

`purge` receives the host's full reference-closure verdict (projects, undo/redo,
trash, takes, agent attachments and rollback). Referenced bytes refuse, including
a sole playable proxy. Unreferenced bytes are copied into a unique archive name,
BLAKE3 verified, synced, then unlinked from active storage. Cross-volume archives
are supported by copying. Failed publication retains the original. An interrupted
copy may leave an additional partial archive copy; retries never overwrite it.
Archive bytes remain recoverable files with their original object name/class in
the name. There is deliberately no archive deletion API. Archiving decreases
active usage, not necessarily physical disk usage; full disks need another volume
or an explicit host recovery flow. Original storage roots and archived files are
never automatically collected by the render-cache policy.

## First-slice verification and remaining integration (historical)

Linux validation passes **83 crate tests** (72 original plus 11 design-review
regressions): 16 colour/scope, 21 task, 18 media, 21 disk and 7 codec tests.
They cover all six tetrahedra and nontrilinear fixtures, domains/shapers, malformed LUTs, exact scope density/conservation,
alpha/audio/timing metadata refusal, proxy-only relink, stale jobs/queue lifetimes,
window event compaction, cancellation races, checksums, Keep Rendered across
reopen/quota changes, quota refusal, archive preservation and symlink rejection.
The unchanged engine suite passes **61 pm-color tests**. Strict Clippy
(`--all-targets -- -D warnings`) and formatting pass on the new crate/sources.

The first-slice root `cd apps/native && cargo test --locked -p pm-promedia`
refused because the workspace package was absent from the native lockfile.
The slice-2 base includes it and the root locked command now passes. The lockfile
remains outside this lane's ownership and unchanged. Reproducible isolation:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
apps/native/crates/promedia/scripts/check-linux.sh
cd engine
cargo test --locked -p pm-color
```

The script copies exactly this crate and both engine sources to a temporary
workspace, seeds the native lock, updates only that copy's workspace entries,
then runs formatting, locked tests and strict Clippy. It leaves the snapshot for
inspection. This is portable crate validation, not a root-workspace locked pass.

Integration required outside owned paths:

1. Native workspace owner: the first slice required the `pm-promedia` lock entry;
   the current base already includes it. The native locked command now passes
   without changing the lockfile or root manifest.
2. Colour owner: add `pub mod lut; pub mod scopes;` to `pm-color/src/lib.rs`, then
   replace this crate's temporary source-path exports with a `pm-color` dependency
   when its integration lock entry is available. Tests currently compile the
   actual engine source files, not a copied colour implementation.
3. Kernel/host owners: own the models, persist media relationships and settings
   through the existing authority, publish task/window facts, bind menus/UI/MCP,
   supply full closure and complete render hashes, archive superseded outputs,
   and wire `pm-engine-service` media selection/invalidation. These are additions,
   not changes to v1–3 calls or extension execution realms.
4. macOS media owner: implement native probe/decoder/encoder and resumable worker
   adapters, codec capability negotiation and free-space reservation. Verify exact
   source-frame/PTS identity, variable-rate/time-base handling, audio sync, alpha,
   colour/HDR tags, proxy/optimized seek performance, cancellation/resource release,
   sandbox/security-scoped relinks and background/windowless operation.
5. macOS integration: verify APFS and cross-volume archive/publication durability,
   low-space/crash recovery, durable Keep Rendered and render bar invalidation;
   compare LUT/scopes against trusted reference vectors at the chosen colour stage
   and verify responsive native controls/accessibility. No Metal, AppKit, GPUI,
   WebKit, screen or VideoToolbox validation ran on Linux.

## Design review follow-up: render-bar model

Review: `design/reviews/na-cloud-promedia.md`, must-fix items 1 and 2.

| Decision | Selecting rule |
| --- | --- |
| Centralize half-open display ranges with `pm-time`'s exact composition rate and frame-start semantics; no view-specific float conversion. | N1/N3: one exact time-base interpretation preserves authored timing, including fractional rates. |
| Use the review's immediate-`Missing` alternative; old graph frames and Keep Rendered pins remain stored but cannot be hits for the edited graph. | N3 correctness and N4 retention: invalidate identity immediately without destroying retained bytes. |

Three regression tests cover integer/fractional/non-flick-dividing rates,
subframe ranges, invalid ranges, maximum tick bounds, and changed graphs with
both kept and unkept old bytes. The Linux harness now copies the unchanged
`pm-time` sources into its disposable dependency workspace. Native painter,
pattern, Reduce Motion and VoiceOver verification remain with the macOS host;
this change introduces no visual literals or UI copy.

## Design review follow-up: Background Tasks model

Review must-fix items 4–7 are implemented in `tasks.rs`:

- `enqueue(window, project, TaskSpec)` requires the media/composition `title`
  and explicit `ProgressUnit::{Frames, Bytes, Items}`. All in-repository callers
  are in this owned crate and are updated. `TaskKind::label` and `symbol` use
  the review's six table rows verbatim.
- Bind `Task::presentation()`, not raw task/debug/serialized fields. It excludes
  scheduling priority and log diagnostics. Units accompany counters; zero total
  produces `TaskProgress::Indeterminate`, even when completed work is nonzero.
  Unknown totals can become known; known totals cannot change or become unknown.
- The presentation exposes the exact `Pausing…` / `Cancelling…` transition copy,
  disables both controls during either transition, and offers `Resume` when
  paused. Cooperative worker acknowledgement and ticket fencing are unchanged.
- `finish` accepts `TaskFailure { code, diagnostic }`; the five review codes are
  represented by `TaskFailureCode`. String conversion classifies a legacy worker
  diagnostic as `Unknown`. `diagnostic_for_log` is the explicit logging accessor;
  serialization/debug retain model facts but must never be used for UI copy.
  DiskFull uses exactly: "Not enough disk space. Free some space and try again."
  Design re-review copy (2026-11-01): SourceOffline "The source media is
  offline. Reconnect it and try again."; CodecUnsupported "This media’s format
  isn’t supported."; Unknown (including legacy raw-diagnostic records) "The
  task couldn’t be completed. Try again." Cancelled has no message: it is not
  an error to the user, and the row leaves the list like a completed task.
  Hosts never fall back to log strings or enum names. A legacy task with an
  empty title shows its kind label alone.
- Older serialized task facts remain readable: missing title stays empty,
  missing unit stays `None` (host leaves the unit unspecified), and a legacy raw
  error is classified `Unknown`. These records are host persistence inputs, not
  a replacement authority or a v1.2.0 render-queue migration adapter.

| Decision | Selecting rule |
| --- | --- |
| Require authored titles and units for new work; preserve unspecified legacy fields without inventing their meaning. | N1/N3/N4: accurate progress and readable retained records. |
| Make `TaskPresentation` the UI boundary; keep diagnostics available separately for logs and omit scheduler priority. | N1/N7: consistent inspectable status without internal implementation text in the UI; review item 10 is pure plumbing. |
| Use only review-approved copy; keep explicit neutral placeholders for unprovided error messages. | Design ownership plus N3: no invented UX or accidental raw-error wording. |
| Keep queue execution, acknowledgement, event filtering and single-writer ownership unchanged; remove the old unitless admission signature only after updating all its consumers. | N6/N8 and the prune rule: no extension execution change or dropped live consumer. |

Eight additional task tests cover the required data, all kind labels/symbols,
window events, serialization, unknown totals, transitions, typed failures and
legacy diagnostics. Combined with the three render-bar tests, all 83 tests pass
in the Linux isolation harness, with formatting and strict Clippy clean.
The first-slice root native locked command refused because its shared lockfile
lacked the package. That limitation is resolved on the slice-2 base; this lane
keeps the shared lockfile unchanged.

Review disposition beyond must-fix items: item 10 is done at the presentation
boundary. Should-fix item 3 (shared VoiceOver range summary), item 8 (sample-gated
ETA), and item 9 (session-retained terminal rows) remain follow-up work. Nice-to-have
item 11 (legacy modal Retry/Remove routing) remains host/shell integration; the
old modal still has consumers and is not removed. No new visual surface is built.
The shell still owns toolbar visibility/aggregate progress, project grouping,
popover/window controls, Retry routing and completion-row lifecycle. Verify these
and live VoiceOver, Reduce Motion and render patterns on macOS using pm-design
and the review, with Opus review before treating any visual result as final.

## Pro media integration, slice 2: real worker encodes

`pm-media/src/proxy.rs` adds the portable `RenditionEncoder` worker port and a
Linux `SoftwareEncoder` using explicit FFmpeg/ffprobe executables, never a shell.
The existing engine's encode sinks live in `pm-export`, not `pm-media`; its
native build requires macOS dylibs/frameworks. This slice introduces the allowed
portable pm-media path without changing that build or claiming to have connected
VideoToolbox. A macOS adapter implements this same trait using the existing
`pm-export` sink; it must satisfy the same output-verification contract.

The software profile is deliberately `ffv1` / `mkv`: lossless, alpha-capable
video and copied optional audio. Other profiles return typed `CodecUnsupported`.
One video stream and at most one audio stream are admitted; other stream
configurations remain original media, not project-open errors. Stream timebase,
ordered presentation timestamps, individual sample durations, audio sample
counts, alpha presence and interpretation metadata must match after encoding.
Missing exact timing refuses rather than inferring a last frame from frame rate.
This restricted adapter is Linux correctness evidence, not the shipping codec
portfolio, performance certification or a replacement for the existing importers.

`worker::RenditionWork::prepare` freezes the catalog job, live task ticket and
original artifact on the kernel actor. Workers verify the source bytes, encode
into a new private staging directory, probe/verify output and recheck the source.
`WorkReport::accept` is the only attachment route in this integration; it calls
`MediaCatalog::complete_task`. Late cancellation/pause wins even over a completed
encode. Stale catalog jobs and queue lifetimes refuse. Retained staging paths
are returned on errors and controls for the host's archive policy; no worker
removes original, previous rendition or interrupted output bytes. Restarting a
paused task uses a fresh job/output and reports monotone frame progress.

| Decision | Selecting rule |
| --- | --- |
| Add a portable FFV1 encode path and native encoder trait rather than importing the Apple-only `pm-export` graph into Linux. | N1/N8 and ownership: real CPU codec work, no Node/Chromium or main-thread JS wait; preserve existing engine consumers. |
| Verify the ordered sample timeline and audio sample counts, beyond nominal frame-rate/duration metadata. Refuse missing exact timing. | N3: exact authored timing; do not manufacture unavailable evidence. |
| Admit only explicitly supported software profiles/stream layouts and retain originals for everything else. | N1/N3/N4: no silent flattening, discarded streams or user-data loss. |
| Validate output-path absence independently of process exit status; keep `-n` as defense in depth. | N3 quirk policy: FFmpeg 7.1.5 can report success for an existing-output refusal; correctness wins over reproducing the tool's status quirk. |
| Freeze work on the actor and accept only current job/ticket results; controls win after codec release. | N3/N7/N8: headless inspectable jobs without a second writer. |

The actual portable engine module is compiled by source path from `pm-promedia`,
following the existing colour-module boundary. Out-of-lane follow-up: the media
owner exports `pub mod proxy;` from `pm-media/src/lib.rs`, gates Apple-only build
requirements for Linux, and the native workspace owner then replaces the source
path with a normal `pm-media` dependency. No existing pm-media file is changed.
The current base already includes `pm-promedia` in the native lockfile; the earlier
lockfile limitation above is historical and no longer applies to this slice.

Synthetic tests exercise real alpha/audio transcodes, nonzero start timestamps,
optimized dimensions, no-clobber refusal, unsupported codecs, altered timing,
changed sources, late cancellation and pause/restart. The exact executable build
is a host-supplied deployment dependency; Linux validation does not establish
VideoToolbox, AVFoundation, Metal, GPUI or macOS UI acceptance. No visual design
or new UI copy is introduced.

## Pro media integration, slice 2: engine frame render path

`pm-media/src/cache_frame.rs` defines worker `FrameSource`, `GraphIdentity`,
`FrameRequest` and validated `EngineFrame` readback. The port carries exact
engine ticks, complete graph/media digests, renderer version and an explicit
colour encoding. Pixels are straight float RGBA at that declared stage. The
macOS renderer adapter must decode engine E and handle premultiplication through
`pm-color`; labelling raw E/premultiplied data as RGB violates this contract.

`render::RenderWork` routes those frames through the real engine
`pm-color/src/lut.rs` implementation before immutable cache publication. The key
combines the supplied complete graph digest with LUT content bytes, colour stage
and the versioned readback representation; media digest, dimensions, exact ticks
and renderer version remain separate key fields. A changed graph/LUT/media key
is immediately Missing while old Keep Rendered frames remain readable.
`FrameSource::identity` is checked before rendering and again before publication.
`RenderWork::accept` is the actor receipt fence: call it on the **current** work
and current identity, including the current LUT selection, to reject late results.
A concurrent graph publication can leave an old-key cache entry; it can never
make that old entry a hit under the new key.

Scopes consume validated engine float frames, before or after the LUT as
explicitly requested. After-LUT scopes use exactly the cached/rendered picture;
Before-LUT scopes request original engine readback even when the transformed
picture is already cached. Matrix, range and analysis stage are caller inputs;
there is no implicit display transform or tone map. Alpha and extended/negative
RGB survive the path; LUT domains clamp only as the existing LUT contract states.
Nonfinite values, f32 overflow, bad frame shapes, mismatched frame identities,
invalid scope configurations and overlarge readbacks refuse before cache fill.

| Decision | Selecting rule |
| --- | --- |
| Require the engine's complete graph/media identity; derive only the additional LUT/readback-stage identity in this lane. | N1/N3: document revision is not a semantic graph hash. |
| Verify frame identity around worker rendering and again at actor receipt. Keep old bytes under old keys. | N3/N4/N7: no stale presentation or destructive invalidation. |
| Apply the actual pm-color LUT source and compute scopes from the selected engine RGB stage on both cache misses and hits. | N1/N3: one numerical implementation; picture and analysis share authored colour intent. |
| Keep the renderer/readback port platform independent and inject native frames. | N7/N8: Linux test double and headless operation, no UI/JS wait. |

The native locked crate command now passes **102 tests** (83 existing, nine real
encode integrations and ten render-path regressions). Strict crate Clippy with
`--all-targets -- -D warnings` and package formatting pass; unchanged
`cd engine && cargo test --locked -p pm-color` passes **61 tests**. The isolation
harness copies both new engine files as well as the existing LUT/scopes sources.

Remaining out-of-lane changes: export `cache_frame` alongside `proxy` from
pm-media; export `lut`/`scopes` from pm-color; replace temporary source exports
with normal dependencies when those owners provide portable build boundaries.
The engine-service owner implements `FrameSource` around actual renderer graph
hashes and E-to-straight readback at the declared stage, wires task dispatch and
actor receipt, and supplies the source lease/archive/persistence route. macOS
must verify actual GPU readback, LUT/scopes reference vectors, graph publication
races, native encoder cancellation, HDR/alpha/audio tags, APFS publication and
real playback/seek performance. No visual surface or unprovided UX copy is added;
there are no new `DESIGN-TODO(opus)` placeholders in this slice.

### Worker control and sample verification follow-up

Probing now streams bounded records through a bounded channel and checks control
while waiting for output, including a blocked decoder. `probe_controlled` is the
native trait's override point; the software implementation kills/reaps and joins
its reader before returning Interrupted. Source/output hashing checks controls
at 64 KiB boundaries and reuses verified probe metadata instead of repeatedly
scanning entire movie frame tables. Interrupted probing/hashing settles as the
requested control, not a task failure. The software encoder uses four codec
threads and all-intra frames; it disables autorotation so stored interpretation
is not accidentally baked into pixels.

| Decision | Selecting rule |
| --- | --- |
| Poll controls while probes are blocked; acknowledge only after process release, including before/after encoding. | N1/N2/N8: useful native pause/cancel without main-thread waits. |
| Hash with bounded checkpoints and reuse one probe per source/output. | N1/N3: bounded memory and fewer redundant decodes without weakening byte identity. |
| Preserve resumed frame-progress floors and verify non-silent stereo samples byte-for-byte in integration tests. | N1/N3/N7: honest monotone progress and explicit audio-fidelity evidence. |
| Refuse a synthetic variable-PTS stream whose last duration is absent. | N3: frame-rate inference cannot establish exact authored timing; originals remain available under N4. |

The locked native suite passes **106 tests**, including blocked-probe/process
release, running encode cancellation with retained partials, cancellation during
probing, resume progress and missing-duration refusal. Clippy with warnings denied
and formatting pass. General VFR/container support still requires a native probe
that provides exact sample durations; this FFV1 test path does not infer them.

### Background render-cache fill jobs

`render_job::RenderFillWork` freezes a frame sequence against one complete engine
identity and a live `BackgroundTasks` Render ticket. Preparation validates project,
kind, units, total and graph ownership. Workers fill one frame at a time, check
controls between frames and preserve completed entries. Pause/restart reuses
exact cache hits with monotone progress. Receipt on the current work checks the
batch's graph/LUT/key signature and current publication before completing the
task. Late controls win after resource release, even across a graph publication.
A stale publication refuses without changing task progress; the host then cancels
and replaces that old-graph task. No old bytes are removed by invalidation.

| Decision | Selecting rule |
| --- | --- |
| Use the existing BackgroundTasks actor model for cache fills, with frame units and the same ticket/control lifecycle as media encodes. | N1/N7/N8: one inspectable execution model and no alternate writer. |
| Fence the complete ordered graph/LUT/key batch at receipt; retain frames completed before a stop. | N3/N4: accurate cache identity and recoverable work. |

Six additional regressions cover real queue completion, pause/cache-hit restart,
late cancellation across an edit, stale progress refusal, admission ownership,
worker failure and unrequested-stop refusal. Existing low-level `RenderWork`
remains available for direct frame/scopes requests. These worker primitives do
not schedule themselves or create another project/settings authority.

### Real software source-frame integration and final verification

`source_frames::SoftwareSourceFrames` implements the engine frame port for a
single source-media graph. It hashes the complete source interpretation and
explicit decoder/conversion version, uses the artifact fingerprint for media
identity, and maps engine ticks through `pm-time` to an exact native PTS. The
FFmpeg decode selector requires that PTS; between-frame and fractional-source
requests refuse. Nonzero starts stay nonzero. Source bytes are verified before
and after readback under the host's media lease. Cache hits require the host to
hold that lease and publish relinks/interpretation changes before using a new
source identity, as with composition-renderer sources.

This software RGB source path drives both direct `RenderWork` scopes and a real
BackgroundTasks Render cache fill. One integration test runs actual encoded
synthetic media through exact-PTS decode, float frames, the pm-color LUT, cache
miss/hit, integer scopes and task completion. The native composition renderer
must implement its own `FrameSource` using complete composition graph hashes;
this source-only graph does not impersonate a composition render.

| Decision | Selecting rule |
| --- | --- |
| Use exact PTS selection and engine rational conversion; reject absent timestamps rather than nearest-frame seeking. | N3: exact authored timing, including nonzero starts. |
| Restrict software scope readback to decoded RGB layouts; leave generic YUV/working-E conversion to the native pm-color adapter. | N1/N3: never let an implicit FFmpeg matrix/transfer choice become authored colour intent. |
| Disable automatic encoder pixel-format conversion (`-pix_fmt +`). | N3: an unsupported lossless input format must refuse instead of silently losing component precision. |
| Include decoder version and source interpretation in source-graph identity. | N3/N7: cache identity is inspectable and changes with the actual decode contract. |

Final Linux evidence, Rust **1.96.1**, synthetic fixtures only:

- `cd apps/native && cargo test --locked -p pm-promedia`: **115 passed** —
  83 existing plus 16 engine-work and 16 render-work tests (32 additions).
- `cargo clippy --locked -p pm-promedia --all-targets -- -D warnings`: pass.
- `cargo fmt -p pm-promedia -- --check` and formatting of both new engine files:
  pass. The lane's disposable-workspace Linux harness also passes 115 tests and
  strict Clippy against the actual engine source files.
- `cd engine && cargo test --locked -p pm-color`: **61 passed**, unchanged.
- `cd engine && cargo test --locked -p pm-media`: attempted; the existing
  `build.rs` refuses because this Linux container lacks the required Powermove
  macOS FFmpeg dylib build (`PM_FFMPEG_DIR`). No pm-media build/export file was
  modified. The new portable modules are tested via the native crate's source
  exports; this is not a full pm-media/platform pass.
- Staged gitleaks scans and the repository pre-commit hook pass before commits.

Remaining integration outside ownership: `pm-media` exports and portable build
boundary; `pm-color` exports; normal crate dependencies replacing source-path
exports; engine-service composition graph/readback and native VideoToolbox
adapters; kernel/host scheduling, current-result receipt, durable catalog/task
persistence, source leases, capability negotiation, free-space reservations and
archival of private/superseded outputs. No old implementation is removed while
it retains consumers (N4/N6 and the prune rule).

On macOS verify actual VideoToolbox codec/alpha/HDR/colour tags, exact VFR/PTS and
audio sync, cancellation/resource release, engine-E/unpremultiplication and
LUT/scope reference vectors, graph publication races, GPU/source-frame equivalence,
seek/playback performance, APFS/cross-volume durability, low-space recovery and
security-scoped source leases. No AppKit, Metal, GPUI, WebKit or screen validation
ran here. No visual/UX decisions or new `DESIGN-TODO(opus)` placeholders were added.
