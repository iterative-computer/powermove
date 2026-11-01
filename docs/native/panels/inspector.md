# Inspector and Effect Controls model

Package `pm-panel-inspector`, `apps/native/crates/panel-inspector`. Model only:
no GPUI, platform UI, Node, Chromium, JS execution, layout, colours or new copy.
The shipped baseline is `v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`).
This is a Wave 4 head start, not a panel activation or a parity/cutover claim.

## Authority and integration

`Inspector` reads an immutable kernel `Published` and owns a `QueryMirror`.
`sync` adopts selection even when the document revision is unchanged. A selected
group owns its selected children. The first remaining layer is primary; no
selection means composition properties. `Binding` and `PropertyId` are semantic
addresses. `PropertyId::value/from_value` round-trip ids without delimiter parsing.
`property_window` enumerates structural addresses and evaluates only a requested
ordinal range. It emits no row height, pixel offset, display text or styling.

`set`, `set_many`, `property_action` and effect actions return kernel `Request`s.
Short edits carry the read revision. Submit with `Inspector::submit`, await its
`Pending`, inspect the kernel reply/notices, then sync the publication. A stale
request is refused atomically. Agents pass their real `Caller` (MCP/Agent origin),
not `Caller::native()`. The planner preserves generated-origin intent protection.
The existing MCP adapter must expose these methods; this crate does not register
or change the frozen MCP tool catalog.

Live controls call `begin_edit`, await successful Begin, stream `edit_step`, then
`finish_edit`. The session captures targets, time, values, selected numeric keys
and scale pairing. A selection change cannot retarget the gesture. Call Cancel
for Escape, pointer cancellation and disposal. Kernel selective cancel / foreign
split / commit-with-notice remains authoritative. Never treat `history.cancel`
as rollback. Host-owned text fields may commit on blur/disposal as the tag does.

Effect clipboard ownership belongs to the app: pass clones of `EffectClipboard`
to both panel instances. Copy preserves source order, enabled animation,
parameter channels and disclosure, and survives destination changes/remounts.
Paste resolves currently registered definitions, generates fresh instance/key ids,
and skips unavailable definitions without discarding saved source effects.
Delete/paste return a proposed `EffectSelection`; apply it **only after success**.
No failed command should change selection. Effect row disclosure is transient
model state, independent of selection and undo. The host should retain the model
across view remounts; replacement document identities clear transient state.

Input reducers describe commands and focus intents, not widgets. A pointer click
ends its empty live edit with Cancel before opening the draft; a moved pointer
ends with Commit. `InputEffect::Write` during a draft/readout key is a one-shot
edit, while a scrub Write is a live step. `InputEffect::Begin` admits a live numeric gesture; `Consume` handles a local key
without writing or leaking it to global shortcuts. Numeric draft input and nested controls
consume their own key context before global layer shortcuts. Transport/readout
updates use `ReadoutClock`: 15 Hz in playback, exact when stopped or forced before
an input event. The host decides scheduling, focus, trackpad arbitration and haptics.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Immutable publications + kernel requests; no panel document writer | N1, N3; kernel single-writer architecture |
| Absolute typed values, from-start relative drags, fixed targets/time and selected-key deltas | N3 exact authored intent; shipped `multi-edit.ts` |
| Keep kernel selective cancellation instead of reintroducing whole-snapshot rollback | N2, N4: acknowledged foreign work survives |
| Preserve unknown/missing effects and opaque authored fields; clipboard stays app-owned | N4, N6; no migration, pruning or source deletion here |
| Direct engine address collector avoids quadratic JSON path discovery; no second channel schema | N1, N2, N3 |
| Structural ids and bounded evaluated windows; retain caller origin and revision checks | N7, N3 |
| No synchronous host services or JS on input paths | N8 |
| Keep Electron and every existing consumer untouched | Prune rule: consumers still exist |
| Isolated verification workspace; shared manifest/lockfile unchanged | Lane ownership restriction; shared lock registration is an integration task |

## Behaviour inventory

All paths in the source column are relative to
`v1.2.0:apps/desktop/src/`. Line numbers refer to that immutable tag. “Done” means
headless model behavior with tests, not native view or extension-host integration.
“Route” means the existing typed command can be submitted but this is not a
complete panel-specific behavior port. “Open” is explicit remaining work.

| Shipped behavior | Source and line | Native model status |
|---|---|---|
| First selected layer, composition when empty, group-owned children | `extensions/inspector/InspectorPanel.svelte:26`, `multi-edit.ts:24` | Done |
| Mixed applicable editable values, locked ancestors skipped | `extensions/inspector/multi-edit.ts:19,97,220` | Done |
| Content fanout across declared shared fields; same-type private fields | `extensions/inspector/multi-edit.ts:32,59` | Done |
| Typed absolute values; relative property/content drags | `extensions/inspector/multi-edit.ts:142` | Done |
| Multiple selected numeric keys offset from original values | `extensions/inspector/multi-edit.ts:117` | Done; fixed-key snapshots tested over repeated drag steps |
| Paired scale values and keys, prepared proportions | `renderer/src/controls/binding.ts:43` | Done |
| Gesture begin/dispatch/commit/cancel, one undo | `extensions/inspector/multi-edit.ts:180`, `renderer/src/controls/gesture.ts` | Done; real kernel actor |
| Property selection without revealing rows; graph/reveal services | `extensions/inspector/ChannelRow.svelte:158` | Selection and graph/reveal intents done; host adapter open |
| Stopwatch across translated targets | `extensions/inspector/selection-animation.ts:16`, `ChannelRow.svelte:111` | Done |
| Diamond adds missing playhead keys or removes all present playhead keys | `extensions/inspector/selection-animation.ts:39`, `PropertyStopwatch.svelte:12` | Done |
| Context add key, easing presets, expression apply/remove, reset | `extensions/inspector/ChannelRow.svelte:138,153,174,237,277` | Done; modal/focus host open |
| Numeric/color/toggle parameter animation, independent coordinates | `extensions/inspector/ParameterList.svelte:17,21,43` | Binding/paired batch/diamond done; widget composition open |
| Effect instance fields absent from old definitions | `extensions/inspector/ParameterList.svelte:13` | Property enumeration and present-definition parameter filtering done; runtime widget adapter open |
| Effect row click, Shift range, Cmd/Ctrl toggle, anchor/focus | `extensions/inspector/EffectsSection.svelte:96` | Done |
| Nested effect control retains selection; outside clears | `extensions/inspector/EffectsSection.svelte:57` | Done |
| Separate effect disclosure, enabled animation, remove by id | `extensions/inspector/EffectsSection.svelte:40,71,82` | Done |
| Up/Down range navigation, Option arrows reorder focused row | `extensions/inspector/EffectsSection.svelte:176,239` | Done; tag reorders focused effect, not whole selected block |
| Copy/Paste/Delete precede layer shortcuts; no invented cut shortcut | `extensions/inspector/EffectsSection.svelte:239` | Done |
| Clipboard order, immutable copies and destination-at-paste | `extensions/inspector/effect-clipboard.ts:30,41` | Done |
| Delete chooses nearest survivor; failed operation preserves selection | `extensions/inspector/EffectsSection.svelte:153` | Done; apply proposed selection after ack |
| Drag insertion slots before/after, index adjustment | `extensions/inspector/EffectsSection.svelte:204` | Done; host hit testing open |
| Global inspector-service paste to raw selection (distinct from row-local paste) | `extensions/inspector/index.ts:56` | Done; one atomic batch and Undo |
| Add registered effect to chosen layer | `extensions/inspector/actions.ts:16` | Done; menu grouping belongs to host registry adapter |
| Composition fields and scene parameters | `extensions/inspector/CompositionSection.svelte:18,71` | Typed binding done; malformed display normalization open |
| Layer solo, visibility, label, blend, motion blur, 3D, scale link | `extensions/inspector/LayerOptions.svelte:13,33`, `TransformSection.svelte:18` | Typed bindings done |
| Parent/matte source choices, parent pick and Escape | `extensions/inspector/LayerOptions.svelte:18,43` | Choices and captured pick/Cancel done; native pick targeting/focus adapter open |
| Alignment against union box and parent-space delta | `extensions/inspector/AlignmentStrip.svelte:26` | Kernel Transform route; text/font availability remains a kernel/host limit |
| Text focus/input/blur one edit; text alignment | `extensions/inspector/ContentSection.svelte:118,199`, `TextAlignment.svelte:10` | Generic live content binding done; text-selection host integration open |
| Independent corners seeding, smoothing detent/popover dismissal | `extensions/inspector/ContentSection.svelte:49,89,102` | Seed/detent commands done; popup/dismissal adapter open |
| Source asset choices, import/cancel/kind validation | `extensions/inspector/ContentSection.svelte:162` | Registered asset/kind validation and captured target write done; picker/import host open |
| Variable axes, saved axes, face slope/weight matching, refresh | `extensions/inspector/TypeSettings.svelte:18`, `TypeAxis.svelte:14`, `ContentSection.svelte:145` | Axes/saved axes, nonmutating inspection, nearest face and atomic materialization done; platform discovery via FontProvider remains open |
| Audio/mute/trim/gain/fades and nested-audio availability | `extensions/inspector/ContentSection.svelte:350`, `audio.ts:9` | Content bindings, numeric limits and recursive availability done; audio service wiring open |
| Mask add/remove and animated geometry/shape/mode/on | `extensions/inspector/MasksSection.svelte:37,53,81` | Kernel Mask and Property route; group-bounds initialization open |
| Shape paths, vertices, fill/stroke/trim/repeater, path grouping/removal | `extensions/inspector/StructuredSection.svelte:32,41,65` | Property route; structural controller/service adapter open |
| Text animator presets, CRUD/order/rename/selectors and style ranges | `extensions/inspector/TextAnimators.svelte:71,93,103,124` | Open; kernel text animator/style commands currently return NotImplemented |
| Caption style/placement/word-highlight fields | `extensions/inspector/CaptionsSection.svelte:19` | Open; kernel Caption commands currently return NotImplemented |
| Time remap/freeze/reverse, sampled animated speed | `extensions/inspector/retiming.ts:7,30` | Done; explicit captured-time commands, exact tag midpoint sampling; playback/audio integration open |
| Extension/shader parameter definitions and errors | `extensions/inspector/ShaderUniforms.svelte:12`, `ExtensionLayerParams.svelte:9,42` | Generic u./x. query/write route; runtime definition/compile-error adapter open |
| Contributed sections when/build/cleanup by insertion point | `extensions/inspector/ContributedSections.svelte:12,21` | Open; preserve V8/remote-DOM edition, no sync JS introduced |
| 15 Hz playback, exact pause/input refresh, structural independence | `extensions/inspector/context.ts:97,101,170`, `InspectorSurface.svelte:40` | Done |
| Per-layer/selection scroll memory | `extensions/inspector/InspectorSurface.svelte:17` | Semantic anchor memory done; pixel-to-anchor host adapter open |
| NumField pointer threshold/direction/modifiers/cancel/click | `renderer/src/controls/NumField.svelte:129,135` | Done |
| NumField horizontal wheel, accumulation/clamp, commit/cancel | `renderer/src/controls/NumField.svelte:107` | Value reducer done; native wheel-axis/timer arbitration open |
| NumField Enter/F2/arrows/Shift, draft stepping, blur/Enter/Escape | `renderer/src/controls/NumField.svelte:71,93,186` | Done |
| Arithmetic relative +/*/ -=; negative literal; finite full parser | `renderer/src/controls/NumField.svelte:78`, `arith.ts:5` | Done |
| Slider arrows/Home/End; Toggle directional/Space/Enter/disabled | `renderer/src/controls/SliderField.svelte:23`, `ToggleField.svelte:41` | Reducers done; pointer slider host open |
| Select/font/color/fill/point/ramp popup keyboard and preview lifecycles | `renderer/src/controls/FontField.svelte:251`, `ColorField.svelte:439,474`, `FillField.svelte:72,158`, `PointField.svelte:16`, `RampField.svelte:22` | Open; shared control models need their owning lane |

## Additional behavior and API details

`fields` exposes semantic content/composition/layer field availability and the
shipped numeric limits. It is an initial schema projection, not a full native
control arrangement: runtime 3D roles, caption schemas, font choices, shader
annotations and contributed sections still need their providers. Transform
fields whose input metadata is not supplied remain `Opaque`; this model invents
no replacement increments or defaults. Effect parameters and large structured
properties use `effect_parameters` and `property_window` separately.

`content::FontProvider` separates platform discovery from pure inspection.
`font_axes` retains saved custom axes and never creates values just by opening
the panel. `set_axis` creates and edits an absent axis in one kernel transaction.
`nearest_face` chooses matching slope before weight. The native font service
must provide actual axes/faces; no sliders are invented for a static font.
`imported_source` accepts only a registered asset of the requested layer kind;
file dialogs/import work happens asynchronously outside the model.

`remap` constructs one undoable content command from captured source timing.
It uses the shipped 1/(max(30,fps)*2) midpoint samples for animated speed and
preserves held freeze keys. Large animated spans should be planned on a worker
with a captured publication, then submitted with the revision fence. This is
not audio separation or an implementation of media decoding.

`commands` exposes the existing kernel typed vocabulary. It is not a success
stub for unimplemented operations. In particular Caption commands, text
animator/style-range commands, text alignment without font inputs, shader-code
uniform synchronization and some structure operations still fail explicitly in
the existing kernel. These require the kernel/runtime owner; this lane did not
replace validation with raw writes. Existing generic channels remain queryable.

## Verification and remaining gates

Final Linux gates: **100 inspector tests pass** (69 real-kernel model tests,
29 input/state reducers, 1 mixed-identity unit test, 1 large-property gate). **273 existing kernel tests pass,
1 optional real-profile smoke ignored**. Strict Clippy for all inspector targets
(`--no-deps -- -D warnings`), cargo fmt and diff whitespace checks pass. The
60,001-property gate evaluates an 11-row window and keeps a hidden failing
expression unevaluated. All fixtures are synthetic; no visual output is produced.

The verification script creates a fresh disposable source copy, preserves the
existing dependency pins, registers only the new local package there, then runs
locked inspector/kernel tests, strict inspector Clippy and formatting:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
PM_INSPECTOR_VERIFY_DIR=/workspace/scratch/work/inspector-check-new \
  apps/native/crates/panel-inspector/scripts/check.sh
```

The destination must be new and outside the repository. Tagged-source kernel
oracles require the local `v1.2.0` and `v1.3.0` Git objects and the test JS runtime;
this is development verification only, with no Node dependency in the app.

The parent workspace includes `crates/*`, so no member edit is required. Its
shared Cargo.lock needs a `pm-panel-inspector` package entry with the existing
`pm-kernel`, `pm-model` and `pm-eval` dependencies. The verification lock added
only that local package; every existing registry/git version stayed pinned.
That file is outside this lane's ownership. Verify the
exact sources in an isolated copy with the same engine/kernel source and existing
lock pins; tests run `--locked` after registering the new local package there.
The normal `cd apps/native && cargo test --locked -p pm-panel-inspector` was
attempted and correctly refused the absent lock entry; it becomes available
when the integration owner adds that entry.

Mac verification: native event/focus routing, VoiceOver semantic controls,
trackpad axis arbitration and pointer cancellation/lock, text services/IME,
font inspection, contributed remote DOM, both panel instances and service-intent
wiring, actual 15 Hz readout scheduling and exact pause, and 60k-row UI cost. Linux
model tests cannot certify any of these. Do not claim extension or visual parity.

**DESIGN-TODO(opus):** the future native surface, field/control composition,
section arrangement, focus/selection/disabled/error presentation, expression
editor, pickers/popovers, drag feedback, accessibility copy and scroll-anchor
mapping need existing design tokens/specs or Opus decisions. No placeholder
visuals, colours, sizing, layout, new user-facing strings or interactions are
invented here. All future visual output remains provisional until Opus review.
