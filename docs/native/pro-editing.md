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
