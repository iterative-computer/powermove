# Timeline panel model

Wave 4 head start. Package `pm-panel-timeline`, at
`apps/native/crates/panel-timeline`. **Model and headless behavior only; no
rendered panel or v4 surface registration.** No shipped code is replaced or
removed. The reference is tag `v1.2.0`, commit
`07ed5fa17ee640659cb075f9cad1af2095011d60`.

## Boundary and integration

`TimelineIndex` reads an immutable canonical `pm-kernel::Snapshot`, resolves the
open composition, and indexes layers, channels and stored key identities. Exact
layer-local key times, negative strip starts, UTF-16 names and opaque key fields
survive reads. It indexes transform, content, effect, mask, shader, extension,
transition, vector/vertex, text animator/range and embedded scene channels.
`TimelineModel` adds selection, disclosure/reveal, graph focus, modes and virtual
row queries. Scale is a semantic row backed by two real channels and their
original key IDs; no animation data is merged or resampled.

`TimelineClient` submits `Request`s asynchronously to the actual kernel actor.
It never owns a `DocumentStore`, blocks on a future, calls JS, or creates a
platform object. Tests use the same kernel on Linux. Typed commands retain the
caller's Interface/Agent/MCP origin and kernel lock/provenance rules. Relative
drags use `GestureCall::{Begin,Step,Commit,Cancel}` with replacement steps.
Acknowledgements, including cancellation conflict/durability notices, are
returned intact. Hosts must await `dispose()` when retiring an active panel;
`Drop` never blocks or silently rolls back a kernel lease.

Exact block reorders and track moves use the existing native `Call::Transact`
operation body. The model checks source/destination/group locks, runs membership
or timing commands against a scratch snapshot using the supplied origin, then
adds ID permutations and minimal preferred-track leaf patches. The kernel owns
atomic application, inverses, history and journaling. These plans carry expected
revision, document identity and required origin; the client refuses a mismatched
origin or document before dispatch. Retire clients on project close/reopen;
the kernel's ordinary Transact API itself only has a revision guard, so its
owner should add same-turn document-session admission before cross-session
application hosting. This crate does not claim to close that actor API gap.

`command(id, ...)`, ID-based selection, key/channel lookup, row-position lookup,
ordinal windows, context action IDs, explicit drag kinds and
`transact_idempotent(..., txid)` are the headless/agent seams. They do not register
new MCP tools or change the frozen MCP catalog. The MCP/application owner can
adapt them without DOM selectors or pointer coordinates.

Playback, clipboard, imports, shared layer menus, composition opening, numeric
sheets and graph fit/merge requests are explicit `HostEffect` values. An effect
is a request to a host, **not a success acknowledgement**. `track.paste` without a
clipboard payload returns `MissingHost`; use `paste_keys` with an explicit
payload. Unknown IDs/actions and unimplemented kernel bodies return errors.

UI session patches clone `UiState` and update owned semantic members, retaining
legacy zoom/scroll values and unknown extension/session/disclosure fields.
They publish through `StateTransaction`, without document undo or per-pointer
fsync. The host still must bridge the tag's global extension `mode`/`trackSnap`
preferences; `patch_ui` is not that global-storage adapter. Imported legacy
geometry remains opaque until the visual adapter can interpret it with the
approved design tokens.

## Behavior inventory

**Done** means implemented/tested at this model boundary, not physically wired
into the macOS app. **Host** means an explicit semantic request exists but its
application adapter remains. **Not done** identifies missing behavior rather
than a success-shaped placeholder. Paths below are relative to
`v1.2.0:apps/desktop/src/`; each reference is to that immutable tag, not main.

| v1.2.0 behavior and source lines | Native behavior | Status |
|---|---|---|
| Visible hierarchy, property rows, ancestors, search: `extensions/timeline/timeline.ts:1025–1084` | Iterative hierarchy traversal, ID row lookup, schema-label search, selected-ancestor reveal; malformed cycles bounded | Done |
| Default and independent group/property disclosure: `renderer/src/legacy/core/ui-state.ts:234–279`; `extensions/timeline/timeline.ts:142–152` | Default collapsed, separate hierarchy/property facts, no document edits | Done |
| Layer click and Shift anchor extend/shrink; Cmd/Ctrl toggle and Cmd+Shift union: `extensions/timeline/timeline.ts:3110–3135` | `select_layer`, original visible-row anchor, selected click retains multi-selection, clears keys | Done |
| Tracks use modifier toggles rather than layer Shift ranges: `extensions/timeline/track-view.ts:665–676` | `select_track_layer` returns selection calls and whether a drag may start | Done |
| Track name selects lane; lane lock/visibility/disclosure: `extensions/timeline/track-view.ts:553–564,767–805` | `track_header` derives lane membership from current projection; edits one transaction | Done |
| Property/graph focus retained across strip selection: `extensions/timeline/timeline.ts:600–613,3704–3730`; `graph-selection.ts:28–42` | Stable curve focus, `graph_target`, channel selection | Done |
| Key selection, selected-hit preference, deferred additive deselection: `extensions/timeline/timeline.ts:521–535,3704–3771`; `graph-selection.ts:56–68` | Explicit hit IDs; linked scale hits expand original keys; pointer-up toggles, drags retain selection | Done (semantic hit); host hit testing remains |
| Key marquee toggles from pointer-down base; graph provisional union, empty release, Escape: `extensions/timeline/timeline.ts:4110–4190`; `graph-selection.ts:46–53` | `MarqueeSession` preserves original IDs and consumes fresh hit sets after scroll | Done (semantic hits) |
| Track clip marquee: `extensions/timeline/track-view.ts:919–959` | Explicit layer ID sets and additive selection calls | Done (semantic hits); auto-scroll/hit projection Host |
| Play/pause, stepping, Home/End, work bounds, J/K and real adjacent keys: `extensions/keymap-default/keymap.ts:108–144`; `extensions/timeline/keyframe-navigation.ts:5–49`; `timeline.ts:4440–4449`; `renderer/src/legacy/ui/shortcuts.ts:930–949` | Exact command IDs, field/repeat admission, frame/10-frame stepping, channel/selection key scope, no adjacent-key wrap; authored layer Out is exclusive boundary | Done; transport Host |
| Ruler scrub, lazy Shift targets, Shift-down/up during drag: `extensions/timeline/timeline.ts:689–709,3175–3223,326–373` | `Scrub`, exact subframe seek requests; clamp composition; first/last visible-frame targets; lazy latch | Done; transport Host |
| Readout drag: `extensions/timeline/timeline.ts:913–918` | No readout-coordinate controller in this crate | Not done; host must convert specified drag units to seek intent |
| Strip crossing snap, release/reverse, incompatible edges, live playhead priority: `extensions/timeline/directional-snap.ts:36–133`; `timeline.ts:3529–3560` | `DirectionalSnap`, weighted ties, sorted target-neighborhood index | Done |
| Track magnet defaults on; Shift inverts: `extensions/timeline/timeline.ts:703–709`; `track-view.ts:808–894` | Track-mode drag snapping preference and modifier inversion | Done; preference persistence Host |
| Work handles, whole-range drag, frame minimum, duration extension and cancel: `extensions/timeline/timeline.ts:3138–3173` | Work drag kinds; from-start baseline; one kernel undo entry | Done |
| Marker frame alignment, duplicate protection, remove by current identity: `extensions/timeline/markers.ts:6–31` | Marker query/add/remove/menu, frame snapping and revision-fenced transactions | Done for ID markers; ID-less legacy marker removal Not done |
| Strip move, trim, grouped members, live preview, Escape: `extensions/timeline/timeline.ts:3496–3603` | Capture expanded IDs once; exact snapped/negative timing; kernel timed-source trim semantics; cancel selective rollback | Done |
| Bracket timing, I/O and split/right-segment selection: `extensions/keymap-default/keymap.ts:64–65,137–144`; `extensions/timeline/index.ts:83–115` | Kernel timing/split commands; preserve key composition times, select generated tails | Done |
| Quick layer offset: `extensions/timeline/timeline.ts:222–246,3064–3108` | Ordered roots, grouped descendants, first anchored, fractional subframe distribution | Done |
| Track projection and movement: `extensions/timeline/track-layout.ts:1–209,230–304` | Separate audio/caption/video areas, overlapping stack priority, preferred lanes, group blocks, slot-preserving moves; atomic timing/track/order commit | Done |
| Name Enter/blur, Escape, blank-name rejection: `extensions/timeline/timeline.ts:4236–4257` | `Draft::Name::finish` commit/cancel; kernel validation/history | Done |
| Numeric property draft, value drag, fine/fast modifiers and limits: `extensions/timeline/property-values.ts:37–42`; `timeline.ts:3282–3395` | `Draft::Property`, `dragged_value`, evaluator-captured baseline, multi-target atomic edit | Done; field focus/IME Host |
| Selected numeric keys between playhead keys: `extensions/timeline/timeline.ts:3265–3280` | Captured selected-key values change by one delta; no inserted playhead key | Done for explicit scalar targets; automatic linked-axis field routing Host |
| Scale grouping, linked ratio, paired animated axes, zero baseline: `extensions/timeline/property-tracks.ts:36–84`; `renderer/src/controls/binding.ts:39–67` | Compound semantic row, exact-time members, `expand_scale_keys`, `scale_value` | Done |
| Multi-layer property ownership and mask/effect retargeting; diamond and stopwatch: `extensions/timeline/timeline.ts:161–218,3225–3229` | `property_targets`, compatible ordinals/types, no locked ancestors, all-target diamond policy, kernel stopwatch | Done |
| Property single/double/Shift reveal and global M/all: `extensions/timeline/property-reveal.ts:7–107` | Injected monotonic milliseconds and schema facts; repeat/double-tap identities; UI-only updates | Done; schema/default/material/missing-effect fact provider Host |
| Key move, composition limits, occupied-frame replacement and restoration on return: `extensions/timeline/timeline.ts:432–501,3605–3702` | Kernel Move with from-start gesture steps and original IDs | Done for uninterrupted gesture; foreign-write refusal described below |
| Graph value/time move, no neighbor replacement, box transforms and Option endpoint stretch: `extensions/timeline/graph-selection.ts:193–290`; `timeline.ts:3820–3966` | Kernel GraphMove/Transform, explicit anchors/scales, native ease-speed handling | Done for semantic transforms; graph hit geometry Host |
| Key proximity snapping and sticky group lock: `extensions/timeline/timeline.ts:298–324,3645–3667` | Separate `keyframe_group` proximity latch, injected acquire/release tolerances | Done |
| Graph handle temporal edit and Option split: `extensions/timeline/timeline.ts:3968–4060`; `extensions/timeline/bezier-drag.ts:20–105` | Kernel handle requests (time/value, side, split), owning key by ID | Done for native numeric handles; complete legacy nonnumeric/unit-handle behavior Not done |
| Value/velocity/scale sheets; easing presets, hold, copy/delete: `extensions/timeline/timeline.ts:4267–4370` | Key-context scope, native easing/hold/delete, typed numeric drafts; dialog/clipboard requests | Done at request boundary; sheets Host |
| Lossless copied keys and paste retargeting: `extensions/timeline/timeline.ts:4350–4356,4411` | Clipboard tracks retain complete key objects and composition offsets; explicit-destination paste selects new IDs | Done for explicit destination; automatic cross-effect clipboard retargeting Not done |
| Track add/paste/clear; ruler/background actions and contributions: `extensions/timeline/timeline.ts:4375–4437` | Semantic checked/enabled menu IDs; stable marker identity; work actions use **playhead**, as tag code does; clipboard payload required | Built-ins Done; async extension menu gather/ordering Host |
| Parent pick/menu; reorder before/inside/after: `extensions/timeline/timeline.ts:3427–3431,3452–3492`; `extensions/timeline/layer-drop.ts:2–17` | Parent IDs, exact block-drop semantics, command-planned world-pose preservation, cycle/lock checks | Done; pickwhip hit testing/menu Host |
| Visibility, lock, 3D and embedded audio: `extensions/timeline/timeline.ts:3397–3445`; shared layer menu | Kernel switch/content transactions; kernel lock policy retained | Done for unlocked targets; locked-video mute override Not done (kernel lacks typed override) |
| Text animator timing bands: `extensions/timeline/text-stagger.ts:103–113`; `timeline.ts:3031–3047` | `TextStagger` pure move/stretch arithmetic and existing channel command requests | Done arithmetic; kernel `ta.*` writes currently unsupported, so application Not done |
| Cue click/modifier selection, move, neighbor-constrained trim, double-click add/text: `extensions/timeline/caption-track.ts:157–242,248–303` | `CueSelection`; typed caption edit requests reject actual kernel NotImplemented | Selection Done; cue drag/trim/add/text behavior Not done pending kernel captions |
| Composition tabs, focused Enter/Space, close eligibility and tab context: `extensions/timeline/comp-tabs.ts:28–86` | `composition_tabs` active/closable facts and OpenComposition host effect vocabulary | Predicates Done; tab actions/service bridge Not done |
| Wheel/pinch, middle-button pan, negative scroll, playback viewport: `extensions/timeline/timeline.ts:2833–2867,3049–3061,1282–1286` | `TimeViewport` time-unit pan/zoom and row ordinal window; transport-only publications do not rebuild rows | Semantic viewport Done; physical wheel delta conversion, graph vertical pan, auto-scroll Host |
| Modes, graph Value/Speed, combined/dope/graph buttons, fit and merge: `extensions/timeline/timeline.ts:790–860,929–960` | Enum state and command IDs; checked/enabled graph menu rows; explicit fit/merge requests | Modes Done; speed sampling/fit/merge computations and graph-toggle restoration of combined preference Not done |

For relative key/graph move and scale gestures, a foreign document write between
samples returns `ConcurrentRelativeGesture` before accepting another relative
step. This prevents the kernel's accumulate fallback from applying a from-start
delta/scale twice. The active entry remains finishable/cancellable; already
acknowledged edits remain. Automatic gesture rebasing across foreign revisions
is not implemented. Absolute strip/work/property steps still use the kernel's
normal selective-cancel behavior.

## Virtualization and evidence

Row windows are ordinal, with caller-supplied count and overscan: no row heights,
scroll pixels or layout decisions. Repeated same-document/revision publications
update selection without scanning the property graph. Snapshot and row builds
are linear in layers/channels/keys for ordinary acyclic data. Group traversal is
iterative and malformed cycles cannot recurse indefinitely. Track overlap
packing uses a coordinate range index with suffix-max skylines instead of the
tag's sibling-pair scan. This is functional/performance-structure evidence, not
a macOS latency claim.

`tests/tag-oracle.mjs` reads and executes the exact tagged TypeScript helpers
using test-only Node type stripping; expected output never calls Rust. The
committed synthetic fixture includes:

- 40 track layouts **and 40 move previews**, including overlapping clips,
  preferred tracks, nested/collapsed groups, audio and captions.
- 960 directional-latch samples, including reversals and modifier changes.
- 600 proximity-latch samples and 20 playhead visible-frame target cases.

The Rust suite also exercises the actual kernel actor, transactions, selective
cancel, foreign writes, history, stale revisions, agent/MCP lock protection,
retry deduplication, clipboard unknown fields, UI opaque state, and a **60,001
layer** window/track-packing case. Fixture data is generated and synthetic.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Native headless model consumes kernel publications; async requests only, no JS/platform dependencies | N7 inspectable/drivable IDs; N8 main never waits on JS; existing single-writer architecture |
| Preserve authored subframes, negative starts, key objects, UTF-16 names and opaque UI fields; never normalize on read | N3 authored intent; N4 no lost data |
| Use the tag's executed behavior and shortcuts, including distinct Layers/Tracks selection, first/last visible scrub frames and playhead-based background work actions | N3 correctness; frozen W1 source evidence beats later-main additions or misleading comments |
| Keep crossing and proximity snap algorithms distinct; preserve original tie order when indexing candidates | N3 exact behavior; N2 efficient large selections |
| Use ordinal virtualization, iterative grouping, and indexed overlap packing | N1 60k+ usability; N2 improve shipped quadratic packing; N7 arbitrary row queries |
| Plan validated group/timing commands and minimal native ops for exact track/block commits | N3 exact placement; N4 retain all unrelated fields; kernel remains sole writer |
| Fence preplanned ops by revision/document/origin and refuse relative gesture continuation after a foreign write | N3 no double-applied delta; N4 never discard acknowledged edits; N7 caller lock policy |
| Return caption/unsupported-command errors and explicit host requests instead of fabricated success | N1 correctness; N4/N6 no silently missing behavior |
| Keep the shipped timeline and compatibility closure untouched | N4/N6 retained extensions; prune rule: consumers still remain |
| Isolated Linux manifest and lock remain inside this crate; shared workspace lock untouched | Lane ownership; same production Rust sources/dependencies, no macOS runtime claim |
| No visual constants, styling, component hierarchy, new copy or hit geometry in the model | Opus design ownership; approved `docs/native/design/language.md` and `pm-design` remain authoritative |

## Verification and handoff

Verified on Linux with Rust **1.96.1**:

| Check | Result |
|---|---|
| Timeline tests | 114 passed, 0 failed, 0 ignored |
| Existing pm-kernel tests | 273 passed, 0 failed, 1 ignored private-profile test; 24 targets |
| Tag oracle | 40 layouts + 40 moves, 960 crossing + 600 proximity samples, 20 playhead cases |
| Clippy, all timeline targets, `--no-deps -- -D warnings` | Clean |
| Rustfmt check | Clean |

For the existing kernel regression run, the new owned crate was temporarily
moved to scratch outside the workspace glob and restored with an EXIT trap.
This let the unchanged shared lock run `cd apps/native && cargo test --locked
-p pm-kernel`; no workspace/kernel source or lock was edited. The first run
needed the existing `v1.3.0` tag fetched for the kernel's library oracle; the
complete rerun above passed. No private-profile test or real user fixture ran.

Use Rust **1.96.1**, matching `apps/native/rust-toolchain.toml`:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
cargo +1.96.1 test --locked --manifest-path apps/native/crates/panel-timeline/isolated/Cargo.toml -p pm-panel-timeline
cargo +1.96.1 clippy --locked --manifest-path apps/native/crates/panel-timeline/isolated/Cargo.toml -p pm-panel-timeline --all-targets --no-deps -- -D warnings
cargo +1.96.1 fmt --manifest-path apps/native/crates/panel-timeline/isolated/Cargo.toml -- --check
node apps/native/crates/panel-timeline/tests/tag-oracle.mjs
```

The shared workspace's existing `crates/*` glob automatically discovers the
production package. Its owner must add the package to `apps/native/Cargo.lock`
before `cd apps/native && cargo test --locked -p pm-panel-timeline` can pass.
The standard command currently refuses the missing lock entry; this lane does
not edit that unowned file. The isolated manifest tests the exact production
sources and keeps dependency versions from the existing shared lock.

Remaining cross-lane work: shared-lock integration; kernel captions and
text-animator channel bodies; same-turn document guard; legacy ID-less marker
removal; typed locked-video audio override; automatic clipboard retargeting;
composition-tab service actions; global extension preference storage;
application MCP/command/surface registration; async extension menu contributions;
transport/clipboard/import/dialog/graph-view effect consumers. Remaining model
behavior is explicitly marked Not done above; this is not a complete timeline
cutover or full behavioral-parity claim.

On macOS verify real input/field-focus/IME routing, physical modifiers and
Escape/disposal, pointer capture and auto-scroll, native undo/menu integration,
engine scrub/presentation/audio continuity, project/session switching, global
preference migration, extension contributions and the retained v1–3 compatibility
closure. Run matched-device 60/120 Hz latency/large-project tests; Linux model
and oracle tests do not prove presentation, AppKit, GPUI, Metal or WebKit behavior.

**DESIGN-TODO(opus):** the v4/GPUI adapter must use the approved timeline tokens
and undergo design review. Geometry, hit targets, wheel-to-time conversion,
graph pan/zoom mapping, drag activation thresholds, focus/field placement and
error/notice presentation remain neutral semantic inputs/outputs here. No
visual output was produced; all later visual output remains provisional until
Opus review.
