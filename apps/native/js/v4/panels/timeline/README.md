# Timeline v4 panel — 2026-11-01

**Provisional implementation; not a complete timeline or an application cutover.**
The build spec is `docs/native/design/panels/timeline.md`, with the shared panel
rules and Owner decisions in `docs/native/design/panels/README.md`. Model behavior
comes from `pm-panel-timeline` and the tagged W1 contracts. No shipped provider,
compatibility edition, user data, extension contract or existing panel is removed.
All visual output still requires Opus review.

## Entry and model/data boundary

`mountTimeline` is the worker entry for panel id `timeline`, title `Timeline`.
It requires the existing v4 accepted-op transport, a generation-scoped
`HostClient`, a native model adapter, an actual viewport window, a unique instance
ID and native surface-retirement callback. Activation and teardown are awaited.
The application still needs to register this entry in its existing timeline slot;
no undocumented PM registration API or command namespace has been invented.

`TimelineSession` subscribes to the v4 host data service's `project`, `selection`
and `time` queries. These publications invalidate a native model projection;
JS never joins potentially inconsistent raw query snapshots or reconstructs a
project/selection/clock authority. Native `inspect` reads one immutable kernel
publication. The session serializes actions, awaits native acknowledgements,
coalesces publication invalidation, preserves the last accepted projection on
failure, exposes errors to the host, and fences late work immediately on dispose.
Partial subscription setup is cleaned up. Native cancellation and surface/realm
retirement run independently, and failures remain rejected.

`nativeTimeline` binds the view to `pm_panel_timeline::v4::{window,dispatch}`.
The native binding must supply the transport time, property facts, surface
stamp and shell key-admission context. It executes and acknowledges returned
`HostEffect`s before resolving `dispatch`; receiving an effect is not success.
The native presenter supplies real schema labels, evaluated values/units, media
readiness, field binding leases, tabs, focus/hover and transport facts. The
presenter cannot override model IDs, selection, locks, timing or revisions.
No presentation provider or host binding is fabricated by default.

The Rust adapter implements actual Layers/Tracks selection modifiers, explicit
idempotent disclosure, mode/view/snap state persistence, layer switches, linked
Scale/diamond requests, property-specific key navigation and admitted shortcuts.
Edits use the real kernel actor and retain native Undo/opaque key fields. Missing
geometry, menus, fields, project/tab services and track-lane presentation return
`MissingHost`, never a success-shaped stub. Gesture hit testing and the remaining
model commands are not wired through this adapter yet. Active native gestures
are cancelled by `TimelineClient::dispose`; Escape also routes cancellation when
such a gesture exists. Global extension mode/snap preference bridging and the
model's actor consumption-time document/selection race remain existing host gaps.

Windows use `model.window(first, visible, visible)`: one viewport of overscan
above and below. Row IDs encode JSON tuples, preserving arbitrary imported ID
punctuation without collisions. Native u64 stamps are decimal strings. Index
lookup is O(1), and rendering never measures every layer name. Equal bounded row
facts reuse their v4 subtree during playback; the test asserts zero row ops on
a transport-only change. This is not a native paint/latency/scroller certification.

## Spec section → status

| Timeline spec         | Done in this branch                                                                                                                                                                  | Not done / host gate                                                                                                                      |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| §1 Model contract     | Direct native model control/window adapter; mode/view/snap, selection, exact timing/IDs, UI persistence; scoped data subscriptions                                                   | Track-lane presentation; full view/geometry/media/graph facts and application binding                                                     |
| §2 Header             | Play/pause, time editing request/real supplied field lease, mode/view controls, conditional graph menu request, snap, tabs/close intents                                             | Glyphs, true tab recipe, fixed-width time field, native menus, time parsing, header math and trailing layout                              |
| §2 Layer column       | Bounded layer/property/track kit rows; names, indices, independent selection, disclosure, switch ordering, property navigation/diamond/Scale-link intents, supplied numeric bindings | Actual 266 pt column, indent/kind badges, parent pick whip/menu, compound Scale field, responsive readout hiding, native inline rename    |
| §2 Ruler/tracks/graph | Neutral labelled placeholders; native event forwarding and model render summary                                                                                                      | All drawing, hit geometry, native gestures, clips/keys/markers/work area/render bar/curves/ghosts/guides/graph split                      |
| §3 States             | Accepted wire-tree snapshots for every listed state; model and visible-window bounds; selected/disabled kit semantics                                                                | Snapshot coverage does not complete missing state visuals (table below)                                                                   |
| §4 Interactions       | Model-backed control actions; forwarding of modifiers, pointer cancellation, wheel, structured drag/drop, keyboard and range events; no optimistic edits                             | Physical pointer capture, hit testing, live drag/trim/key/graph/stagger gestures, menus/sheets, clipboard/import execution, tab services  |
| §5 Keyboard/focus     | Header order, one track-area Tab stop, non-tabbable row controls, conditional controls; raw key/repeat/modifier forwarding; Rust resolver after shell admission                      | Real shell arbitration and field ownership, row focus/reveal/rename navigation, Control-Tab exit, native tab roving and splitter behavior |
| §6 VoiceOver          | Real AccessKit assertions for group/outline/row labels, selected state, disabled values, units and spoken time; exact five announcement strings                                      | AXRowCount/virtual navigation, key Delete/Press, native slider/tab roles, `.medium` announcements and macOS window graft                  |
| §7 Appearance         | pm-design token references only; validator accepts all 16 scheme/accessibility combinations                                                                                          | Real native painting, contrast/opacity/Reduce Motion behavior, screenshots and Opus acceptance                                            |
| §8 Copy               | Spec copy and native schema/user labels; no new product error messages                                                                                                               | T6 close-tab action label needs design copy                                                                                               |
| §9 Don't              | No accent row/clip fills, per-axis graph hues, 3D column, letter badges, new shortcut registrations, or retirement of existing consumers                                             | All unimplemented visuals remain explicit placeholders                                                                                    |

The shared Owner decisions remain authoritative: `metrics.layer_column` is
266 pt; the Agent popover/pin, Effects naming and removal of web export are not
changed by this lane. No literal substitute for a missing token is introduced.

## State evidence

All 18 snapshots are accepted by the real Rust surface and kit validators.
They capture the current provisional op tree, including explicit placeholder
metadata, rather than certify the completed spec or physical appearance.

| State             | Implemented evidence                                                                   | Incomplete visual behavior                                                            |
| ----------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| No project        | Exact empty copy/action, disabled Play only in header                                  | Symbol and push-button recipe                                                         |
| Composition empty | Empty copy/import action and drop event routing                                        | Column/ruler/drop-target geometry                                                     |
| Loading           | Native loading fact retained in row metadata                                           | Clip fill and `Loading…` drawing                                                      |
| Selected          | Neutral selected row, original native IDs, actual AccessKit selected flag              | Clip ring and key marks                                                               |
| Hover             | Token row hover and conditional switches                                               | Edge bars/cursors and marker tooltips                                                 |
| Focus             | Focus order and native focus metadata                                                  | Focused row/key rings and real focus navigation                                       |
| Disabled          | Locked diamond/value guard and AX disabled values; layer selection retained            | Not-allowed cursors and every native gesture guard                                    |
| Error             | Offline/failure facts retained; exact failure announcement text                        | Offline clip treatment, exit motion, macOS announcement delivery                      |
| Drag/drop         | Structured data/modifiers forwarded, target/time/folder feedback retained              | Target/insertion/drop-time painting and actual import/effect application              |
| Multi-select      | Model-backed modifier selection, neutral rows, exact count copy                        | Rigid selected-clip/key gestures                                                      |
| Playing           | Play/pause/time facts change with zero row ops                                         | Native playhead and frame-clock rendering                                             |
| 60k rows          | Real 60k-layer Rust projection returns 30 rows for a 10-row viewport; bounded v4 nodes | Logical scroller, row anchoring after topology changes and full virtual AX navigation |
| Graph empty       | Exact empty-graph copy, conditional graph controls                                     | Geometry and graph split                                                              |
| Extra snapshots   | Tracks, graph-selected, time-edit, rename/tabs, compound Scale                         | Native lane projection, non-empty graph, actual editors/tabs/compound well            |

## DESIGN-TODO(opus) and API gaps

T1–T5 mark neutral placeholders for already-decided design awaiting kit/host
support. They do **not** ask Opus to redesign those controls. T6 is missing copy.

1. **T1 — glyph/control slots.** The kit has no SF/custom symbol slot or native
   push-button component. Required symbols stay inspectable metadata; no emoji,
   CSS drawing or invented substitute is used. Icon-only view segments currently
   carry their specified accessible labels as provisional kit content. Need
   symbol slots, per-segment symbols/tooltips, true push buttons, and snap's
   two-line tooltip. Play uses `colors.text`, per spec.
2. **T2 — time field.** Need the specified fixed width measured from
   `00:00:00:00`, text-field role in readout mode and transactional native
   timecode/frame/seconds parsing/commit/cancel. No width or parser is invented.
3. **T3 — composition tabs.** Need a kit tab group/tab with leading close,
   approved underline/token arithmetic, horizontal scrolling, fades, close
   eligibility/hover/focus and native tab roles. The existing toolbar kit
   controls expose native composition IDs and one roving Tab stop provisionally.
4. **T4 — timeline/layout/AX recipes.** Need the approved column/ruler/track/graph
   native recipe, token arithmetic, flexible spacers, splitter, hit map, pointer
   capture/cursors, direct native playback overlays and geometry-based virtual
   windows. Metadata is **not** a drawing API. Need AXRowCount, virtual-row
   navigation, sliders/tabs/keyframe actions and announcement publication.
   The surface validator does not accept even numeric flex weights or `auto`
   margins, so those spacers also remain neutral placeholders.
5. **T5 — compound/conditional values.** Need one dense Scale well with native
   axis hit mapping, linked editing and width-aware hiding. The specified compound
   readout stays a single semantic row; no two-field redesign or fake binding.
6. **T6 — close-tab VoiceOver copy.** §6 supplies the composition's tab name but
   not its separate close control's spoken label. Current neutral label is the
   composition name; Opus must supply the distinct action copy.

Outside owned paths: application panel registration, native binding/presentation
services, UI-kit additions, native layout/painting/menus/fields/geometry/AX and
HostEffect execution must be supplied by their owners. The old app and v1–3
compatibility closure remain consumers until the full replacement is proven.

## Rule-selected decisions

| Decision                                                                                          | Selecting rule                                                          |
| ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| Keep Rust as sole selection/edit/undo authority; require acknowledged native effects              | N3 correctness, N7 inspection, N8 worker boundary; kernel single writer |
| Preserve canonical key objects, subframes, negative starts, opaque UI state and u64 IDs           | N3 exact intent, N4 no lost data                                        |
| Use bounded native ordinal windows, stable encoded IDs and unchanged row subtrees during playback | N1 pro scale, N2 improve shipped work, N7 headless driving              |
| Return errors for unavailable host work, retain accepted state, fence retirement                  | N1 correctness, N3 honest outcomes, N4 preserve working state           |
| Use kit/tokens only; retain neutral placeholders rather than invent missing visuals or copy       | Design ownership, N1/N3; timeline spec and shared Owner decisions       |
| Retain every old provider, v1–3 contract and application path                                     | N4/N6, prune rule: consumers remain                                     |
| Browser-target runtime entry and async host/model ports; no runtime Node imports                  | N7/N8; Linux tests cannot prove application thread scheduling           |
| Merge both required bases and update only the panel's lock dependencies                           | Merge-ready instructions; no bulk dependency upgrades                   |

## Reproducible checks

Rust 1.96.1; Bun 1.4.4. Commands assume the repository's activation script has
configured those tools. Test helpers are owned here or in the model crate;
validators use the actual stable surface/design packages, with no spike paths,
private fixtures, system temp-path assumptions, ffmpeg tools or Python dependency.

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-panel-timeline -p pm-kernel
cargo fmt -p pm-panel-timeline -- --check
cargo clippy --locked -p pm-panel-timeline --all-targets --no-deps -- -D warnings
cd js/v4
bun install --frozen-lockfile
bun test
bun x tsc --noEmit
bun x tsc --noEmit -p panels/timeline/tsconfig.json
bun x prettier --check panels/timeline
bun run build
bun build panels/timeline/index.ts --target browser --outdir ../../../../work/timeline-bundle
```

The panel's five Bun test files cover mount/retirement, session failure/lifetime
races, model/presenter authority, actual Rust selection -> v4 -> AccessKit,
all specified state trees, focus/disabled/selection/units, live event modifiers,
large windows, transport-only row stability and a browser bundle mounted/driven
without Node, Chromium, window or global document. Rust adapter tests exercise
the real kernel; the existing timeline behavior suite remains intact.

macOS still must verify actual layout/painting at 1×/2×, Opus review captures,
keyboard/Full Keyboard Access/VoiceOver and window AX integration, native text
services/IME, menus/sheets/popovers, live gesture capture/cancel/Undo, clip/key/
graph timing, media/transport/render bar behavior, logical scrollers/60k latency,
accessibility system changes and absence of main-thread JS waits. These are
completion gates, not Linux claims.

## Final Linux validation — 2026-11-01

Merged `feat/native-engine` at `4204a153dbf8af916ef2b92d7ca01858e363516a`
and `feat/na-v4-integrate` at `e5e209928817cb9434823ebf6ddf7d64f2213441`.
The only merge conflict was the native workspace lock; all integrated panel
entries were retained. The final fetch/merge found both bases already current.
Only the timeline package's dependency entries changed subsequently.

- `cargo test --locked -p pm-panel-timeline -p pm-kernel`: **404 passed**:
  131 timeline (114 existing + 17 adapter) and 273 kernel. One existing optional
  private-profile kernel test is ignored; no failures.
- `cargo test --locked --manifest-path crates/panel-timeline/isolated/Cargo.toml -p pm-panel-timeline --target-dir target`:
  **131 passed**, no failures or ignored tests. Both owned lockfiles are current.
- `bun test` in `apps/native/js/v4`: **226 passed**, including **54 timeline
  tests**, **18 snapshots**; no failures/skips. All specified states have
  validated tree snapshots; incomplete visuals remain identified above.
- Package rustfmt, all-target strict Clippy, base/panel TypeScript, panel Prettier,
  frozen Bun installation, base browser build and timeline browser build pass.
- Gitleaks staged scans run before each commit; no leaks.

Native fixture compilation runs in a 120-second setup hook outside Bun's
five-second interaction-test timeout. The first final parallel run exposed
Cargo build-lock waiting inside that short timeout; after fixing the harness,
the complete sequential suite passed. This is setup tolerance, not a relaxed
interaction/performance budget or a macOS gate pass.

## Frozen v4 contract adoption — 2026-11-01

Merged `feat/native-engine` at `f7844ebc0` after the orchestrator recorded the
40/40 integrated eval and froze the v4 contract. The merge is conflict-free.
The API and kit sources, `docs/native/v4-api.md` and `docs/native/ui-kit.md`
are unchanged from the integration contract already consumed by this lane; the
incoming change records the freeze in `docs/native/p0-decisions.md`.

No API changes or compatibility workarounds are needed in the timeline adapter.
The existing typed primitives, worker lifecycle, generation/revision fencing,
acknowledged model actions and kit components remain the contract. Changes to
`crates/surface` or `js/v4/src` were not made. T1–T5 above remain additive **API
requests** to their owners, not permission for a breaking contract change. T6
remains missing design copy. The panel is still provisional; the incomplete
spec sections and macOS gates listed above have not been relabelled as done.

Against the frozen base, the locked combined Rust suite passes **404 tests**
(131 timeline + 273 kernel), with the one existing private-profile test ignored.
The complete Bun suite passes **226 tests**, including 54 timeline tests and
18 snapshots. Package rustfmt, strict all-target Clippy, both TypeScript checks,
panel Prettier, frozen dependency installation and both browser builds pass.
Decision: retain the frozen shape and submit missing capabilities as additive
requests (orchestrator freeze; N3/N6/N8), preserving existing consumers and the
single-writer worker boundary.

## Renderer follow-ups — 2026-11-01

`pm-panel-timeline::render` now owns the native paint plan, separate from the
six-op control tree. `Scene::from_model` consumes the real immutable model on a
worker; the native `ModelPresenter` supplies schema/AX names, label colours,
media readiness, interpolation and track labels. Layers use exact authored
seconds. Tracks use the existing indexed track packer, including property
sub-rows. No presentation callback runs on a display tick.

The retained `Frame` paints via `Painter`; Linux uses `Recorder`.
`Frame::transport` changes only playhead and visible property diamonds.
`Scene::prepare` visits the viewport plus one viewport of overscan, binary
searches visible keys and track clip times, and exposes full AX row count.
`Scene::navigate`, `scroll_for`, `anchor` and `reanchor` support virtual row
navigation and identity-based scrolling. The host retains its RowId→NodeId map.
Graph segments come from the evaluator and are painted by `graph_render`;
the renderer does not fit, sample, normalize or modify curves.

The header separator (review F3) and kind-badge slots are wired. A missing
native kind/label remains a neutral, noninteractive slot. Existing kit-owned
F1/F4/F5/F6 remain with UI-kit-2; this lane does not change the frozen v4 API.

Decisions and selecting rules:

- N3: preserve negative starts/subframes and exact native timing; no frame
  snapping on read and no Chromium paint behavior is reproduced.
- N2/N8: build geometry/key/track indexes on publication; repaint retained
  overlays independently of JS or row layout at playback cadence.
- N7: stable row-scoped key operation IDs, inspectable JSON, actual AccessKit
  row counts and explicit virtual navigation; missing native presentation fails.
- N1/design ownership: shapes/tokens follow the spec. Named custom paths are
  requests to the native painter, not fabricated vector art.
- N4/N6/prune: no Electron/compat consumer or user data is removed or changed.

DESIGN-TODO(opus): the native `marker` and `keyframe.hold` paths, and Rendering
stripe pitch/period, need the approved art/cadence. They remain named paint
requests; no substitute art or animation constants were chosen here. All
visual output remains provisional pending Opus review.

Run from `apps/native`:

```sh
cargo test --locked -p pm-panel-timeline
cargo clippy --locked -p pm-panel-timeline --all-targets --no-deps -- -D warnings
cargo run --locked --release -p pm-panel-timeline --example render_budget
cd js/v4
bun test panels/timeline
bun panels/timeline/bench.tsx
bunx tsc --noEmit -p panels/timeline/tsconfig.json
```

Release Linux evidence (synthetic 60,000 layers, 960×1080, 1,080 measured
frames after 120 warmup): max 114 mounted rows / 246 paint ops / 89,938 JSON
bytes; prepare p95 **292.821 µs**, p99 **412.181 µs**; JSON inspection p95
**1,607.909 µs**; retained playhead p95 **0.972 µs**. Half of a 120 Hz frame is
4,166.667 µs. A separate 60,000-clip single-track test verifies horizontal
culling. Worker index construction was 28.854 ms, outside the frame loop.
The v4 control tree benchmark (90 mounted rows, 842 nodes, 1,200 samples)
measured frame p95 **1,062.097 µs**, p99 **1,428.059 µs**, at most 42 commit ops
per frame (5,117 at initial mount). These are CPU/operation-tree measurements,
not macOS GPU, scanout or Electron comparison evidence.

macOS integration still required outside this lane's ownership: attach the
native painter to the shell's timeline bounds and display link, supply native
schema/media/evaluator/font services, route native hit/AX navigation into the
existing model, and enqueue native AX announcements. Test light/dark/Increase
Contrast, Reduce Motion, symbol paths, clipping, both timeline modes, resize,
VoiceOver navigation into unmounted rows, and physical 120 Hz cadence. Existing
menus, fields and gesture acknowledgements remain the host's responsibility.
