# P5 pro editing commands

This lane implements the kernel contracts for cutover additions W079–W083.
The shipped oracle is `v1.2.0`; these additions do not change the existing
`TimingCommand` or v1–3 extension command semantics. No Electron quirk is adopted.

## Atomic timing and persistent links (W080, W081, W083)

The catalog exposes `Command::ProEdit` and `Command::ProLink` (also `From` their
public command types). All identities are layer/composition/group ids. A
`TrackTarget { group_id, track }` identifies an authored lane; absent track is
lane 0, and absent/null group means the composition's root. Nothing uses cached
stack indices. Times and deltas are f64 composition seconds; ranges are half open.
The one-frame minimum uses the edited composition's FPS. No subframe input is
silently snapped. Pointer callers send total deltas through
`update_gesture_from_start`; commit/cancel use the existing gesture APIs.

| API | Behavior |
| --- | --- |
| `ProLinkCommand::Link` | Pair exactly one video and one audio clip in the same composition at their present sync relation, with a caller-supplied fresh link id. Different assets and manually synchronized clips are valid. Unlink before relinking. |
| `Unlink` | Remove both members' edit-link metadata; one undo step. |
| `linked_selection(root, ids, bypass)` | Expand by persistent pair identity, deduplicated in caller-first order. Selection itself remains ephemeral through the existing selection transaction. |
| `sync_badges(root, id)` | Signed timeline-seconds offset for both peers. Positive means that member is later relative to the recorded relation. UI formats seconds at the composition FPS. |
| `ProEditCommand::Move`, `Trim`, `Slip` | Explicit linked expansion or bypass. Move retains local animation/source; In trim advances source and rebases local keys; slip changes source offset by delta × each member's rate, without moving the strip or keys. |
| `Delete` | Delete the expanded set, preserving ordinary deletion/unparent behavior. Bypassing a pair also removes its survivor's link metadata, with lock checks and undo, so there is no dangling pair. |
| `Split` | Caller supplies each tail's layer id by original layer id and one new tail-pair link id. Rebase keys and source, clear transitions/fades at the cut, retain all other authored and opaque data. Both halves retain at least one frame. Splitting a linked member alone requires unlinking first. |
| `RippleDelete` | The selected clips cover one contiguous union of time. Close that interval on explicitly targeted tracks. Reject a surviving clip intersecting the removed interval, ambiguous gaps, crossing suffix clips, or an omitted linked suffix peer. |
| `RippleTrim` | Out delta changes duration and shifts the downstream suffix by that delta. In delta advances source In and shortens duration, keeping the timeline In fixed and shifting the suffix by the negative delta. Linked trim targets must share timeline bounds. |
| `Roll` | Explicit adjacent A/B on matching tracks: A's Out and B's In change by delta. Combined outer bounds remain fixed; B's source/keys retain absolute authored time. Linked pairs roll together. |
| `Slide` | Explicit adjacent A/B/C on matching tracks: B moves by delta with duration/source/local keys unchanged; A's Out and C's In compensate. Outer bounds remain fixed. Linked triples slide together. |

Document metadata is additive per layer:
`editLink: { id, phase }`, where `phase = from - trim / rate` at link creation.
The sync error is `(from - trim / rate - phase)` minus the peer's value. Thus
ordinary trims and identical moves/slips preserve sync, deliberate bypass shows
the offset, and manually aligned different sources start at zero. Link identity
is unrelated to transform `parent`, `group`, and the scale-link switch. Unknown
project/layer/content/channel fields survive edits. A malformed/orphaned link
refuses linked edits with a diagnostic; it is never silently discarded.

The new pro timing commands currently accept static-timing video/audio strips.
Source timing keys/expressions and enabled time remapping require explicit
property edits; the existing general layer commands remain available for other
layer kinds. This is an admission boundary, not a project-open restriction.
Moves need no media IO or known source duration; source-changing edits require
finite asset-duration metadata and enough handles. Audio stays at unit rate.
Every affected clip and ancestor is lock-checked; generated origins also respect
protected hand edits. A locked downstream clip refuses the entire operation,
including any earlier planned changes. No clamp conceals an exhausted handle.
Composition duration stays authored; extend it explicitly before an edit that
would push a clip past its end. Track scope never widens implicitly for links.

## Decisions and rules

| Decision | Rule and reason |
| --- | --- |
| Pro ripple/roll/slide semantics are new typed commands; legacy timing remains intact. | N1/N2 provide the missing professional workflows; N6 preserves the extension contract. W1 and the tag's `legacy/assistant/video-editing.ts` distinguish slip from absent pro slide/roll/ripple. |
| Explicit track targets, stable ids, caller-supplied created ids and pure sync queries. | N7 makes the same behavior headless and inspectable. N1 prevents a hidden track selection from changing unrelated content. |
| Refuse unsupported timing, unavailable handles, locks, ambiguous neighbors, or a partial linked edit. | N1/N3 choose atomic, explainable failure over an altered source frame, overlap guess, or silently skipped member. |
| Retain exact f64 authored timing and rebase keys on In edits; never copy browser snapping quirks. | N3 preserves authored intent. Frame minimum and composition bounds are validation rules. |
| Additive native link metadata uses the existing project, history and journal machinery. | N4 preserves data/undo and avoids a second persistence authority. No existing implementation or consumer is pruned. |
| Pure Rust planners and immutable query input. | N8 requires no Node/Chromium/runtime wait; the kernel remains the sole writer. |

## Verification boundary

Linux tests use synthetic media metadata and the real document store, operations,
lock policy and history. They verify exact geometry/timing/key/source data,
linked selection/badges, atomic failures, nested compositions and group tracks,
paired roll/slide, and multi-update gesture cancel/one-step undo/redo.
No renderer, decoder, audio device, AppKit, GPUI or WebKit runs here.
The macOS host must route tools/menus/agent adapters to these catalog variants,
apply linked selection deliberately, and display sync badges. Verify actual
source frame identity, audio sync, hit testing, accessibility, modifier bypass,
track-target UI and cancellation in the macOS application. This document does
not claim W079–W083 native UI acceptance has passed.

## Source-monitor editing (W082)

`Command::ProSource(ProSourceCommand)` resolves `EditPoints` before planning.
Each source/timeline mark is optional. Three marks determine the fourth; four
consistent marks are accepted. If both source marks exist and neither timeline
mark exists, playhead supplies timeline In. Two marks, negative/resolved-negative
points, reversed/zero ranges, nonfinite values or inconsistent durations refuse.
There is no implicit rate change or fit-to-fill. Source and timeline duration
must match exactly within the arithmetic comparison epsilon (1e-8 seconds).

`SourceTarget` explicitly names video/audio, a track/group id and a fresh layer
id. One source stream can target one lane; an A/V edit uses distinct lanes and
a fresh persistent pair id. Video-only inserts disable embedded audio;
audio-only uses an audio strip referencing the source asset. Audio from a video
requires imported `hasAudio: true` metadata. Asset duration is required, and
imported video dimensions are used when available. New clips reuse the kernel's
`make_layer` defaults rather than inventing renderer-specific transforms.
Target groups must exist in that composition and be unlocked. Group headers
are containers, not clips on their root track.

Insert splits a crossing strip and shifts its tail and the suffix by the exact
inserted duration on targeted tracks. Overwrite retains only the portions
outside the interval, without rippling. Keys and source positions are rebased
for each retained tail; effects, masks and unknown content survive. The caller
supplies `SourceSplit` ids only for originals retaining both sides. Those ids
must match exactly, and paired tails need one shared fresh pair id. Both tracks
of existing linked clips must be targeted and receive matching fragments;
otherwise the edit refuses. Entirely removed clips remain recoverable through
the standard transaction history, including after PMV save/reopen. Surviving
transform children are unparented as in ordinary deletion. Untargeted content
and composition duration remain authored.

Additional decisions: N1/N3 choose three-point range resolution and exact
source sampling over implicit stretching; N7 makes marks/targets/created ids
explicit command data; N4 retains fragments, opaque fields, native metadata and
undo through the existing PMV writer. N8 keeps all planning independent of
media decoders, JS and the renderer. Tests cover all four missing-point cases,
insert/overwrite boundaries and fragments, stream targeting, source/lock/id
failure, pair fragments and actual PMV reopen with persisted undo/redo.

## Shuttle transport facts (W079)

`commands::pro_transport` exposes `ShuttleState`, `ShuttleInput` and
`ShuttleOutcome`. This is UI/transport state, not an undoable document command.
`dispatch` is pure; the actor owns publication through the existing
`StateOp::Ui` / `StateTransaction`. `from_transport` hydrates old `{time,playing}`
facts and richer `{rate,playRange,returnTime}` facts, with clock availability
and composition FPS supplied by the host. `transport_value` validates and merges
the result while retaining unrelated/extension transport fields. Capability is
never asserted by persisted data. The actor integration test proves publication
without document revision, tree, dirty-state or history changes.

J/L during playback select reverse/forward 1×, advancing to 2× for another
press in the same direction (capped at 2×); switching direction starts at 1×.
K stops and snaps the time to a composition frame. When paused or the native
clock is unavailable, J/K/L return `Declined` so the legacy event/property
binding may run once. Explicit Reverse (Control-J) starts from paused.
PlayAround (Control-Shift-Space) takes explicit before/after/duration bounds,
starts at the clamped range start at +1×, and records the original return time.
`observe_clock` consumes the engine's actual observed time, stops at the
composition/range boundary and restores the return time after play-around.
It does not invent a wall-clock/audio scheduler. Nonfinite/malformed ranges
return errors rather than panicking or requesting fallback replay.

Decision: N1/N2 require usable pro shuttle rates and play-around; N6/W079 require
state-dependent decline, never double execution; N7 makes state transitions
inspectable data; N8 keeps JS/audio/UI work out of the planner. Kernel session
facts remain separate from document history, consistent with `kernel.md`.
Mac verification must connect the result to `pm-engine-service`'s actual clock,
check reverse audio/source sampling and ±1×/±2× rates, frame-aligned stopping,
work-area bounds, play-around return and single key consumption/fallback.

## Completion and remaining integration

Implemented entry points: ripple delete, ripple trim, slip, slide, roll,
link/unlink, linked selection, linked move/trim/split/delete, sync-offset data,
source insert/overwrite with three-point marks and track/stream targeting,
and J/K/L/explicit reverse/play-around transport facts. All are native Rust.
The command variants are reachable through the kernel's existing typed command
transaction route; no kernel actor, legacy command, renderer, vendor or app
host source was edited. Adapter/UI work outside the owned paths remains:
map tool/menu/MCP argument decoding to the new types; apply linked selection;
publish shuttle facts and drive the engine clock; display target tracks and
sync badges. Direct legacy timing calls retain their existing behavior.

The safe timing admission boundary remains static video/audio. Other layer
kinds, time remapping, animated source timing and mismatched linked-fragment
edits are explicitly refused by these new media commands. They continue to
open unchanged and can use existing general/property commands. Broader pro
source-time editing for those kinds needs its own intent-preserving design.
No existing implementation is removed and no user profile or media is touched.

Final Linux validation: `cd apps/native && cargo test --locked -p pm-kernel`
passed **257 tests**, with **1 pre-existing ignored profile test**. The additions
are **25 pro timing/link tests, 16 source-edit tests and 9 transport tests**
(**50 new tests**). `cargo fmt -p pm-kernel -- --check` passes.
`cargo clippy --locked -p pm-kernel --tests` passes with no warnings in the new
files; existing warnings remain in out-of-lane files. Repository hook and staged
`gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow` pass before
each logical commit. The environment initially lacked the activation/tooling
and required parity tags; local tools were installed outside Git and the public
v1.2.0/v1.3.0 tag objects were fetched for the existing parity tests.

## 2026-11-04 — cloud pro2 actor and automation adapter

`KernelHandle::pro_editing` now queues structured native pro actions on the
existing actor. `ProAction::decode` accepts strict argument objects for all
pro timing/link operations, linked selection, sync inspection, source marks,
source insert/overwrite and shuttle. This is an adapter for native UI/MCP hosts,
not an alteration of the frozen MCP tool schemas. The authenticated caller,
observed document-open identity and revisions come from the host, never tool
arguments. A native host must explicitly bind a real clock using
`bind_pro_clock`; persisted transport cannot assert availability, and bindings
from a prior document lifetime cannot drive a reopened project.

Document actions become ordinary guarded `Transact` calls, preserving locks,
provenance, journal notices, lost-ack txid deduplication and Undo. Linked selection
uses the ordinary selection route. Source marks are partial session facts under
`session.proSource`, bound to an explicit composition and preserving opaque
session/monitor members. Marks and transport require the observed state revision;
source edits resolve the current marks on the actor in the same turn as applying
the command. State-dependent actions deliberately reject txids and require fresh
inspection on retry. Inspection returns exact document/state identifiers as
strings, transport, source marks, clock availability and deduplicated sync badges.
Marks and transport do not create document history. Clock observation is a typed
native-host operation and cannot be supplied by decoded automation arguments.

Decisions: N1/N2 complete the missing native workflows through the existing
catalog; N3/N7 use stable ids, strict arguments, lifetime/revision fencing and
actor-time mark resolution; N4 keeps all existing history, opaque fields and
journal warnings; N6 leaves legacy command/key/MCP contracts intact; N8 routes
through asynchronous actor messages with no JS, engine or UI waits. No visual
output or new UX is introduced; there are no DESIGN-TODO(opus) additions.

The existing domain tests remain, with additional actor workflows covering
ripple/roll/slide, linked selection/bypass/split/delete, sync badges, source A/V
insert/overwrite, marks, transport, real transaction gestures, stale work,
locked downstream clips and retry deduplication. Production tool registration,
menus/gestures and engine clock delivery are outside this lane's owned paths;
Linux adapter tests do not claim those app/Mac bindings are complete.

## 2026-11-04 — cloud pro2 render lifecycle

### Render cache and tasks boundary

`pm_kernel::pro_jobs` adds a worker-owned `RenderJobs` lifecycle and
`RenderJobHost` IO interface. It does not replace the media lane's
`pm-promedia::{cache,tasks,storage}` or claim to supply a renderer/codec. The host
bridges those services, observes real kernel publications and supplies its
monotone media/font/effect/colour resource generation. Relink/proxy/resource
changes therefore fence work even without a document revision. A complete
`FrameKey` includes semantic graph/media digests, exact ticks, size,
representation, renderer version and explicit Originals/Proxy/Optimized
provenance. An original-only request refuses derivative keys; receipts must
match the requested key and reserved byte bound. Cache lookup verifies actual
bytes through the adapter instead of trusting a completed row. Keep Rendered
requires an existing verified object and delegates durable pins to the cache.

Every queued/running/progress/cancel/cleanup/terminal transition is saved before
publication. CancelRequested remains visible while a producer owns its output.
Settlement saves Cleaning, drains/stops/reaps the producer, archives partials,
then saves the terminal state. A drain/archive/save error remains an error;
cleanup can be retried. Startup validates the whole retained inventory before
cleanup IO, drains/archives interrupted work, and invalidates old tickets.
A crash between archival and terminal save is idempotent. A crash after cache
publication can retain valid derived bytes while the task remains interrupted;
that is not reported as successful task completion. Terminal ids are retained,
never reused. Unknown frame totals remain explicit None, without a percentage.

`RenderJobHost` must own exclusive private roots, validate physical disk space
plus its safety floor, verify source leases/provenance and actual pixels/output,
atomically sync job facts, publish immutable no-clobber cache objects, and verify
and sync archives before retiring active names. Raw durable envelopes/unknown
fields remain the storage host's responsibility; the serializable JobRecord is
the known lifecycle projection, not a replacement project/storage format.
Low-disk cleanup never evicts originals or deletes partials to manufacture
success. If archival lacks space, staging and a retryable diagnostic remain.
These are mandatory adapter contracts, not assertions that production backing
has already been wired.

Decisions and selecting rules:

| Decision | Rule |
| --- | --- |
| Reuse existing actor transaction/publication machinery and keep worker IO behind a trait. | N3 one document truth; N8 no decoder, filesystem or JS waits on the UI/kernel actor. |
| Fence jobs by project, document, revision and native resource generation; include provenance in cache identity. | N1/N3/N7 prevent stale frame attachment and proxy-as-original claims. |
| Save cancellation/cleanup intent, drain producers, archive partials, then acknowledge completion. | N4 preserves bytes on cancel/crash/low disk; N1/N7 expose failures and live ownership. |
| Verify cache objects on reads and refuse absent/corrupt pins and mismatched receipts. | N1/N3 choose actual bytes over a success-shaped remembered task. |
| Keep existing media/cache/task implementations and all live consumers. | N4 and the prune rule; this lane supplies coordination rather than a second media authority. |
