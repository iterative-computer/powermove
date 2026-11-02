# Built-in Effect Controls — v4

2026-11-01. Panel `layer-effects`, title `Effect Controls`. Build specification:
`docs/native/design/panels/effect-controls.md`, shared panel README (including
Owner decisions), inspector row grammar, and `docs/native/ui-kit.md`.
**Headless implementation; native application activation and Opus visual review
remain open. All visual output is provisional.** This is not a panel cutover or
complete VoiceOver/visual acceptance claim.

`effectControlsExtension` declares the built-in's v4 identity.
`mountEffectControls` starts an acknowledged host subscription, registers visible
parameter bindings, renders JSX into six native operations, and disposes the
surface, subscription and bindings. The app supplies the native menu/focus/
announcement services and `Tokens.metrics.row`; the view contains no visual
number literals. `powermove/ui` has not yet switched to the new kit entry, so this
panel imports that public kit's existing source entry explicitly. No changes were
made to the surface crate or v4 runtime/UI sources.

## Production worker integration

The thin `pm_panel_inspector::v4::EffectsPanel` adapter is compiled into the real
model crate. It owns only presentation metadata, a structural row index and the
visible range. The app retains the existing `Inspector` and shared app-owned
`EffectClipboard` across remounts. The adapter never owns a writable document,
submits a request, waits on an actor, or executes JS.

Register the command `panel.effect-controls` and evaluated query channel
`panel:layer-effects` with the application's `KernelData` implementation:

1. Query calls `EffectsPanel::project` on a synced immutable publication and
   current native playhead time. Supply actual registry parameter labels,
   controls, steps, units, options and localized layer-type labels in
   `Presentation`; no display defaults are guessed. Missing presentation data
   stays a read-only raw-value placeholder. Missing definitions keep raw authored
   channels, including unknown fields, intact.
2. Command arguments are `{layer, documentId, revision, intent}`. Resolve the
   authenticated caller and project/realm/generation before calling `plan`.
   Preserve request-ID → kernel txid dedupe and the returned request's caller.
   A plan may contain a typed revision-fenced kernel transaction or only a
   transient model change. Do not optimistically apply its proposed selection.
3. Submit a returned request asynchronously through `Inspector::submit`, retain
   the real pending result, and poll it on the worker. Sync the actual publication
   and call `acknowledge` only on its successful result. Preserve kernel notices
   and failures. Return the projection at the actual ack revision. Never convert
   an enqueue into a success ack. Transient actions publish at the same document
   revision; the host service's subscription sequences distinguish them.
4. Supply a new HostClient generation on document lifetime replacement/project
   change. The adapter also checks document identity, layer and revision, and a
   queued JS intent is copied and fenced against a changed target. Menu results
   cannot retarget a different layer/document. A failed action leaves the last
   accepted UI and selection intact and announces no success.
5. Route native controls through the registered bindings, including captured
   gesture cancellation on rebind/removal/disposal. The header checkbox declares
   click-default cancellation and queues exactly one model Toggle intent; its
   binding name is an inspection identity, not a second native write path.
6. Use `controller.outside(insideEffectSurface)` for window-wide outside-click
   routing. Nested controls consume their own context; effect shortcuts precede
   layer shortcuts. Implement the native routing gap G2 below. `Platform.focus`
   must reveal offscreen semantic IDs before resolving accepted numeric node IDs.

`Platform.effectMenu` is the existing native registry menu (groups/submenus,
Recent when available). `Platform.contextMenu` receives the exact spec items and
empty-clipboard disabled tooltip. It must use NSMenu. `Platform.announce` posts
medium-priority native accessibility announcements. `Platform.report` receives
technical diagnostics without replacing product copy. The shell owns dock menus
and the Project/Effect Controls tab placement.

## Spec acceptance map

“Done” below means Linux model/wire behavior, not a rendered macOS approval.

| Spec section | Done | Not done / native gate |
| --- | --- | --- |
| §1 Model | Actual inspector adapter; ordered header/parameter windows, independent selection/disclosure, shared clipboard, immutable values and actual kernel transactions | Application command/query registration, native binding implementation (G1) |
| §2 Anatomy | Kit tab group, identity, header controls, checkbox, parameter rows, fixed panel-wide value column, trailing action and missing definition text | Push/disclosure recipes, token math/indent, header gaps and native insertion geometry (G3–G5) |
| §3 States | Golden op-tree + real validator + AccessKit snapshots for no-project/no-selection/no-effects/unsupported, selected, hover, focus, disabled, locked, error, collapsed, multi-select, reorder, drop-new, 60k, contrast and reduced motion; synchronous loading is explicitly N/A | Hover exit, keyboard-focus ownership, drag visuals, scrolling/paint (G2–G5) |
| §4 Interactions | Actual-model selection, nested retention/outside clear, checkbox, reset/remove, add/menu intents, copy/paste, reorder/drop, diamond and animation removal; parameters preserve exact values and original handles | Native menus, controls, custom parameter widgets/pickers and complete expression UI (G1/G5/G6) |
| §5 Keyboard/focus | Up/Down/Shift, focused-effect Option arrows, copy/paste/delete, disclosure, Space, Return and Escape handlers; one list Tab stop, separate parameter stops; offscreen focus/reveal request | Native before-global routing, actual traversal/restore and first-entry focus design (G2, T1) |
| §6 VoiceOver | Real AccessKit assertions: panel/list/row roles, selected state, checkbox state, numeric value/units, names; full AX tree stored in every golden | AXRowCount/multiselect/custom actions, keyframe spoken-state mapping, native announcements and virtual navigation (G4) |
| §7 Appearance | pm-design token references; contrast header edges; reduced-motion declaration | Complete native token recipes/motion and Opus review (G3/G5) |
| §8 Copy | Exact specified copy; browser reference says `Effects` under the Owner decision; no web export added | Missing-error/announcement cases require T2/T3 |
| §9 Don't | No nested value column, switch, disclosure-on-selection, effects-in-Properties, source deletion or replacement of existing consumers | Native rendering still must prove this visually |

The 60k Rust gate builds 60,004 flattened header/parameter rows, then evaluates
only a three-row window. A deliberately throwing hidden expression is never
visited. Structural caches invalidate on document identity, revision, disclosure
or registry changes. A visible row's stable ID anchors scrolling through reorder;
range events request one viewport of overscan above/below. No text is measured in
JS. The native virtual-list must implement full logical scroller height and
visible-row navigation; the snapshots alone do not prove frame-time performance.

## API gaps (outside this lane's runtime ownership)

- **G1 — Application registration/activation.** Install the v4 built-in and
  KernelData command/query/binding adapters described above, including document
  lifetime resets, captured native edit sessions, current-time/15 Hz scheduling,
  rejected edits/notices, actual persistence and cancellation. Tests use the real
  HostRouter, HostDataService, model and kernel actor; the fixture's binding map
  and Platform are explicitly test doubles, not native controls.
- **G2 — Native key and focus routing.** Current `Interaction::plan` ORs an
  ancestor's declared key cancellation into nested fields. The panel therefore
  does not declare key cancellation on the ancestor list. The shell must resolve
  effect-list versus field context before global/default handling without waiting
  for JS, or the surface API must support target-scoped cancellation. Native focus
  modality/reveal/restoration must be supplied; pointer focus is not inferred to
  be keyboard focus. The kit's ListItem still has its own Enter/Space defaults.
- **G3 — Token/layout support.** `metrics.tree_indent` is still absent in this
  base. No literal substitute is used: parameter metadata carries the required
  token as a neutral placeholder. Token-expression support is also needed for
  the exact inset/hit math. A fixed `virtual-list.itemSize` cannot account for
  `metrics.gap` before later effect headers; per-kind offsets/row sizes are needed.
  Native range events must describe the visible window, not already-added
  overscan. The existing prototype header gap is provisional until that support.
- **G4 — Accessibility.** Full count/multiselect and Press/Expand/Collapse/Delete/
  Move Up/Move Down/Reset are inspectable declarations in metadata. The current
  kit does not project that metadata to AXRowCount/multiselect/custom actions.
  It also lacks a keyframe control with a spoken three-state value. Do not mistake
  accepted metadata for implemented VoiceOver actions or virtual row navigation.
- **G5 — Native kit recipes.** Add a system push-button recipe (trailing Add
  Effect currently uses a neutral ToolbarItem placeholder), leading disclosure/
  symbol slots, panel-wide disabled-label styling while values remain editable,
  selected-row focus outlines, token-derived drop-line/circle geometry, 50% drag
  rendering, pointer-leave notifications, and scoped row gap/layout support.
  Drag/focus/symbol declarations are inspectable; unsupported native consumption
  is not claimed. NSMenu grouping/recents and OS drag data are platform services.
- **G6 — Remaining inherited inspector controls.** The kit entry currently has
  number/text/checkbox/colour/choice controls. Point/fill/font/ramp/custom widgets,
  expression editor/status, property context menus, colour hex composition and
  picker internals need their owning kit/native binding implementations. Unknown
  control metadata displays raw values read-only; the adapter never fabricates
  widget defaults or writes unknown data.

## DESIGN-TODO(opus)

- **DESIGN-TODO(opus) T1:** Which header becomes active when Tab enters an effect list with no
  current effect focus/selection? The spec defines one Tab stop but no initial
  active-row rule. No automatic selection is invented.
- **DESIGN-TODO(opus) T2:** Specify announcements for multiple removals and downward reorder.
  Existing singular `Removed {effect}` and upward `Moved {effect} up` copy is
  retained; no new plural/downward string is invented.
- **DESIGN-TODO(opus) T3:** Specify inline copy/placement and recovery for host/admission/registry
  projection failures. The error-state table only covers missing definitions and
  an empty clipboard. Until decided, retain accepted content and report the
  diagnostic through the host.
- **DESIGN-TODO(opus) T4:** Specify presentation of opaque/custom parameter widgets when registry
  presentation metadata is unavailable. Neutral read-only raw values preserve
  inspectability and data; no replacement editor/layout/copy is invented.

- **DESIGN-TODO(opus) T5:** Specify empty-state symbols for No Selection and Unsupported. Only
  No Composition and a supported layer with No Effects name a symbol in the
  spec. These other states omit the symbol instead of guessing.

The spec already decides indentation, controls, drag/focus visuals and layout.
Their missing implementations are API gaps above, not new design decisions.

## Rule-selected decisions

| Decision | Selecting rule |
| --- | --- |
| Actual model/kernel transactions, ack-before-selection/bindings, document/layer/revision fences and copied queued intents | N3/N7/N8; actor single writer; no UI-thread JS waits |
| Preserve missing definitions/unknown channels and the app-owned cross-layer clipboard; never delete or normalize source data | N4/N6 and prune rule |
| Option arrows and reorder operate on the focused/source effect, not the selected block | Shared README precedence + tagged EffectsSection source/model; N3 authored behavior |
| Option-diamond on a static parameter is a no-op; animated parameters remove animation through the model | Inspector §4 explicit remove-animation intent; N3/spec over a generic model stopwatch toggle |
| Use registry-authored presentation and token references; leave unavailable recipes as inspectable neutral placeholders | Design ownership, N1/N3; no invented visuals |
| Apply `Effects` to the browser reference in empty-state copy; do not rename Effect Controls | Owner decision #3 specifically renames Effects & Presets; the panel spec still names layer-effects Effect Controls |
| Retain all Electron/v1–3 code and existing consumers | N4/N6; prune rule has not been satisfied |
| Register the test harness as an explicit workspace member, add only its local lock entry | Merge-ready rules #2/#6; no dependency bulk upgrade |

## Verification

From the repository (Bun 1.4.4, Rust 1.96.1, gitleaks available):

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
apps/native/js/v4/panels/effect-controls/check.sh
```

The script runs locked inspector/harness/kernel tests, strict Clippy for the
changed crates, cargo fmt, a locked harness build, frozen Bun install, the panel's
TypeScript/Prettier checks and the entire existing v4 Bun suite. The ordinary
`bun test` also builds the panel's workspace binaries automatically. No separate
checkout, private plan, spike, external fixture, Python version or Linux-specific
filesystem is required. Cargo metadata resolves the actual target directory;
URL paths are decoded. All fixture projects are synthetic.

Golden files hold complete emitted ops, accepted native trees, native focus
orders and full AccessKit projections. Regenerate only after reviewing the
intended state change:

```sh
cd apps/native/js/v4
UPDATE_EFFECT_SNAPSHOTS=1 bun test panels/effect-controls/test/panel.test.tsx
```

On macOS verify GPUI/AppKit recipes, system push buttons/checkboxes/NSMenu,
1×/2× geometry and one value column, all pointer/focus/drag states and
cross-panel drops, native controls/IME/clipboard and picker commit/cancel,
VoiceOver count/actions/announcements/virtual navigation, accessibility settings,
60k scroll/reveal cost, actor durability and no UI-thread JS waits. Obtain Opus
review screenshots/keyboard/VoiceOver evidence and sign-off before activation.

### Linux result and merge baseline — 2026-11-01

Normal locked workspace: **11 new adapter tests + 100 inspector tests + 273
kernel tests pass** (384 total); the existing optional real-profile smoke remains
ignored. The full v4 Bun suite reports **211 passed, zero failures, zero skipped**,
including **39 panel tests** and **18 complete operation/native-tree/AX state
goldens**. Strict all-target Clippy for both touched crates, cargo fmt, locked
binary build, frozen Bun install, the panel's TypeScript check and Prettier check
pass. The production panel browser-target bundle check finds no Node/DOM runtime
imports. Gitleaks passes on staged changes before each commit.

Merged `feat/native-engine` at `4204a153dbf8af916ef2b92d7ca01858e363516a`
and `feat/na-v4-integrate` at `e5e209928817cb9434823ebf6ddf7d64f2213441`.
The native-engine merge's only conflict was Cargo.lock; both the existing v4
UI-kit dependencies and incoming model package entries were retained. The
final v4 merge was conflict-free. The complete `check.sh` passed after both merges.
Only the test harness's local package was added to the lockfile; no registry
package was upgraded, and engine/Cargo.lock needed no change. Native rendering
and G1–G6 / T1–T5 above remain open; this branch is not a production activation.

### Frozen-contract revalidation — 2026-11-01

Merged the frozen `feat/native-engine` contract at
`f7844ebc007c17900cb2d65f4ef63c6bd3f8b005` without conflicts. Its surface,
v4 runtime, API and UI-kit sources are identical to the versions this panel
already used; the incoming change records the freeze in p0-decisions.md. No
implementation, golden or lockfile adaptation was necessary. The complete
`check.sh` passed again: 384 Rust tests (11 adapter, 100 inspector, 273 kernel;
one existing optional real-profile smoke ignored), 211 v4 tests, strict Clippy,
formatting, TypeScript and frozen install/build checks.

**API requests (additive only):** G2 needs target-scoped native key cancellation
and focus/reveal routing; G3 needs token expressions and per-kind virtual-row
offsets; G4 needs full row-count/multiselect/custom-action and spoken keyframe
state projection; G5/G6 need the missing system-control, drag/hover and inherited
widget/picker/expression recipes. G1 is application activation/binding integration,
not a request to change the frozen envelope. Preserve the existing contract and
request additive support rather than changing crates/surface or js/v4/src
(N3/N7/N8, ownership and the recorded API freeze). T1–T5 and the macOS/Opus gates
remain unchanged; the upstream 40/40 headless eval does not certify this panel's
native rendering or application activation.
