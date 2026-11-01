# P5 pro media and colour (package 21)

CPU implementation: `apps/native/crates/promedia` (`pm-promedia`), plus the new
`engine/crates/pm-color/src/lut.rs` and `scopes.rs` sources. These are worker-side
services and single-writer models. No native UI, codec, platform performance,
render-graph integration or cutover acceptance is claimed.

Read against `pro-editing.md`, W1's pro-media additions and W019/W020, D-STATE's
original/proxy-only/offline closure, the kernel single-writer contract,
`engine-service.md`, and P0's V8-logic/WebKit-DOM architecture decision.
The v1.2.0 `main/media-proxy.ts` preserves alpha, timing and optional audio;
its VP9/Opus choice addresses Chromium decoding and is not a native codec rule.
No Electron code is removed and no extension contract changes.

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

## Verification and remaining integration

Linux validation passes **83 crate tests** (72 original plus 11 design-review
regressions): 16 colour/scope, 21 task, 18 media, 21 disk and 7 codec tests.
They cover all six tetrahedra and nontrilinear fixtures, domains/shapers, malformed LUTs, exact scope density/conservation,
alpha/audio/timing metadata refusal, proxy-only relink, stale jobs/queue lifetimes,
window event compaction, cancellation races, checksums, Keep Rendered across
reopen/quota changes, quota refusal, archive preservation and symlink rejection.
The unchanged engine suite passes **61 pm-color tests**. Strict Clippy
(`--all-targets -- -D warnings`) and formatting pass on the new crate/sources.

The requested root `cd apps/native && cargo test --locked -p pm-promedia` currently
refuses because the new workspace package is absent from the native lockfile.
That lockfile is outside ownership and remains untouched. Reproducible isolation:

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

1. Native workspace owner: add `pm-promedia` to `apps/native/Cargo.lock` with
   dependencies `blake3`, `serde`, `serde_json`, `tempfile` (all already locked)
   and the existing engine `pm-time` path package,
   then run the root locked test command. No root manifest edit is needed because
   its existing `crates/*` glob discovers this crate.
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
The root native locked command was attempted and refuses because the shared
lockfile lacks this package; ownership keeps that lockfile unchanged.

Review disposition beyond must-fix items: item 10 is done at the presentation
boundary. Should-fix item 3 (shared VoiceOver range summary), item 8 (sample-gated
ETA), and item 9 (session-retained terminal rows) remain follow-up work. Nice-to-have
item 11 (legacy modal Retry/Remove routing) remains host/shell integration; the
old modal still has consumers and is not removed. No new visual surface is built.
The shell still owns toolbar visibility/aggregate progress, project grouping,
popover/window controls, Retry routing and completion-row lifecycle. Verify these
and live VoiceOver, Reduce Motion and render patterns on macOS using pm-design
and the review, with Opus review before treating any visual result as final.
