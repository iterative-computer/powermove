# v4 native UI kit — package 13

Status: **component/wire/native-description implementation tested on Linux;
AppKit/GPUI rendering, host integration and Opus visual approval pending**.
Governing design: [language.md](design/language.md), [tokens.md](design/tokens.md),
[review-checklist.md](design/review-checklist.md) and
[editor-mockup-v3.html](design/editor-mockup-v3.html). Protocol: [v4-api.md](v4-api.md).
The component entries are implementation contracts, not screenshots or a claim
that native controls already render in the editor.

## Component mapping

All entries below are done at the JSX, six-op wire, Rust recipe, validation and
AccessKit projection layers. Native widget names describe the macOS adapter's
required implementation. System-owned shapes/sizing remain unspecified in the
recipe (`None`), rather than overriding AppKit with invented dimensions.

| Component export | language.md | Native recipe / behavior |
|---|---|---|
| `InspectorRow` (`Row`) | §§5–6,9 Inspector rows | Flexible label slot, ellipsis + full-name tooltip; fixed `value_column` slot, optional leading keyframe/trailing linked-axis action slots outside it, panel `inset`, `row` height; context label association |
| `NumberField` (`NumField`) | §9 Scrubbable numbers | Scrubbable native text input, tabular figures, left alignment, trailing muted units; native transactional scrub and existing Controls typing/arithmetic |
| `TextField` | §9 Controls; §16 | Native text input, opaque binding; existing live editing, commit/cancel and native composition observations |
| `PopUpButton` (`SelectField`) | §9 Buttons and pop-ups | `NSPopUpButton`, `control` height standalone, `field` height inside an `InspectorRow`; typed option values preserved, including null/false |
| `SegmentedControl` (`Segmented`) | §9 Segmented controls | System segmented control; field track, `track_padding` = `control_radius − inner_radius`, floating selected segment, inner radius; arrow/Home/End navigation skips disabled choices, one Tab stop |
| `Checkbox` / `Toggle` (`ToggleField`) | §§1,9,16 | Native **checkbox** for both (switches are Settings-only); placed at the leading edge of the value column with no title (row label is its AX label); boolean/mixed state and native binding driver |
| `ColourWell` (`ColorField`) | §9 Popovers | `field`-square swatch, `control_radius`, authored colour string retained; opens the host colour popover ([panels/inspector.md §7](design/panels/inspector.md#7-colour-popover)) anchored by stable node ID |
| `DisclosureGroup` (`Disclosure`) | §9 Inspector rows | Labelled disclosure button, expanded state, token section typography/insets; collapsed descendants removed atomically |
| `List` / `ListItem` | §9 Lists and tabs | Row height, neutral `selected_fill` in a `control_radius` selection shape, selection state; Tab, Enter/Space and disabled guards |
| `Tree` / `TreeItem` | §9 Lists and tabs | Outline/tree roles; the same row/selection rules, explicit expanded state and Left/Right collapse/expand callbacks |
| `ToolbarItem` | §§2–3,8–9 | Native toolbar item; caller supplies symbol/content/copy; active glyph uses accent, rest secondary, padded hit target |
| `SheetAnchor` | §9 Sheets and alerts | Window-modal system sheet, **placed by the system** (centred in the window on macOS 26+); sheet-width token; panel-solid content and separate system frame material |
| `PopoverAnchor` | §9 Popovers | Control anchor, popover-width token, solid floating content; live edit declaration, Esc/outside-dismiss contract, focus scope |
| `ProgressRow` | §9 Progress, tasks and status | System determinate indicator with a finite fraction or indeterminate spinner; no synthetic percentage; AccessKit range only when determinate |
| `EmptyState` | §5; review checklist §3 | Centred heading/reading text with caller-supplied explanation and one action; no lane-authored product copy |

`Segment` and `DisclosureButton` are internal component recipes with their own
Rust tests. No old control, extension, profile or built-in panel is removed.

## Worker and macOS host boundary

The new JSX entry is `apps/native/js/v4/src/components/index.ts`. It intentionally
lives entirely inside this lane's owned directory. Existing `src/ui.ts`, package
exports and compiler aliases are **unchanged**: the owning API/compiler lane must
route `powermove/ui` to this entry, reconcile its compatible aliases and preserve
the other existing exports. Until then, source-level consumers can import this
entry directly. JSX still uses the existing `powermove-v4` adapter and six atomic
ops, without browser layout/style queries.

`pm_surface::kit::Panel::prepare` runs on a worker with an accepted `Arc<Surface>`,
current `pm_design::Tokens`, logical-point layout rectangles, focus and system
glass availability. It resolves recipes, produces a complete AccessKit tree and
returns diagnostics without changing authored data. `PanelHost::enqueue` is the
nonblocking publication seam; `InspectPanelHost` is the Linux double. The real
AppKit/GPUI adapter remains outside the owned paths. The host consumes semantic
`data-pm-component` and `data-pm-slot` markers; these do not implement a CSS engine.
`InspectorRow.value_column` applies to the value slot, never to the full row.

The adapter must use system controls first, give the label slot remaining width,
keep field visuals separate from their padded hit rectangles, tint native focus
with the app accent, and draw custom focus with the recipe's focus token. Glyph
sizes use symbol tokens; fonts use `pm_design::TEXT_FONT`. Recipes expose neutral
selection and hover colours and separate solid content from floating frame
materials. A toolbar host owns its material once; it must not stack a second
glass surface for each item. Accessibility notifications rebuild tokens and
republish recipes: contrast colours/borders, solid transparency fallback, and
reduced-motion cross-fades all come from pm-design.

Every descendant reaches `accessibility_tree`; labels resolve directly or through
row IDs. Numeric values include units, native text observations are preserved,
and checkbox/mixed, selected and expanded states have AccessKit representations.
`focus_order` follows the accepted tree's visual depth-first order with existing
Interaction rules. The shell must reconcile focus, activate transient scopes,
apply focus requests and restore trigger focus after closing an anchor. Hidden
content is absent from both focus and the AccessKit tree.

`accessibility_action` checks the expected surface revision, declared action,
target, enabled state and typed value, then returns a `NativeAction` for the
native input queue. The host must additionally check panel/realm generation and
graft/namespace IDs into the window tree. It must route native actions through
Controls/Bindings and native focus/selection/anchor handlers, never call JS
synchronously. `Click` on a native field uses its native driver; purely v4
selection callbacks are enqueued to the worker.

`NumericScrub` uses logical accumulated coordinates (including motion beyond the
screen), token drag threshold, property step/speed, Shift/Option, finite checks,
bounds and one Bindings begin/write/commit or cancel. It snapshots the original
binding; cancel before reconciliation on destroy/disable/rebind. Queue errors
remain retryable, including pointer capture after a failed first write. Finishing
is idempotent; a finished gesture cannot start again. The shell executes pointer
capture/lock, cursor restoration and edit-text effects. Existing Controls owns
separate typed drafts and commit/blur/Escape; large finite authored numbers
survive rounding without overflow, and typed finite commits do not acquire
scrub clamping. Wheel editing is deliberately left unbound pending the design
item below; ordinary inspector scrolling cannot accidentally edit values.

## Validation and tests

`validate_panel(surface, tokens, layout)` emits stable ID/property diagnostics:

- `missingLabel`: unnamed component/control or unresolved/blank label reference.
- `nonTokenValue`: literal visual values, unknown token names or wrong token
  families, after the stylesheet cascade. Only structural keywords are literal.
- `smallHitTarget`: actual logical layout hit width/height below `metrics.hit`
  (24 pt), or non-finite geometry. Actual geometry takes precedence over declared
  token minimums. Without layout, both token minimum dimensions are required;
  absent dimensions report `missingHitTarget`.
- `missingAnchor`, `invalidProgress`, `unknownComponent`: invalid kit descriptors.

Diagnostics do not mutate/reject a previously accepted surface or change v1–3
admission semantics. The app must surface diagnostics to extension authors and
agents and supply actual hit rectangles after native layout; token minimums alone
do not certify the rendered panel.

Rust tests cover every one of the 20 recipes, the 16 combinations of light/dark
and three accessibility flags, token patch propagation, labels, hit rectangles,
stylesheets, anchors, actions, focus, native text, progress and scrub failures.
Source guards reject numeric visual literals in recipes and non-token visual
assignments in JSX; mutation probes demonstrate failure on literal colours,
sizes, radii, type and durations. JSX fixtures for every public component are
replayed into the real Rust wire/panel validator. A browser-target kit bundle
also mounts without process, Buffer, window, document or Chromium.

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-surface
# Shared Cargo.lock reconciliation belongs to the integration lane.
cargo test --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface
cargo clippy --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface --all-targets -- -D warnings
cargo fmt --manifest-path crates/surface/standalone/Cargo.toml --check
cargo build --locked --manifest-path crates/surface/standalone/Cargo.toml --examples
cd js/v4
bun install --frozen-lockfile
PM_UI_KIT_WIRE=../../crates/surface/standalone/target/debug/examples/ui_kit_wire PM_SURFACE_WIRE=../../crates/surface/standalone/target/debug/examples/wire bun test
bun run typecheck
bun run format
bun run build
```

Linux result (2026-11-01): **73 pm-surface tests** (35 existing + 38 kit),
**81 Bun tests** (55 existing + 26 kit), no skipped tests; strict all-target
Clippy, rustfmt, TypeScript, Prettier and browser builds pass. The unchanged
**9 pm-design tests** pass in a disposable minimal workspace copy with the same
design sources, token table, snapshot and theme target file. The requested shared
workspace command stops at locked resolution; its unowned Cargo.lock remains
unchanged. These counts do not establish native rendering or visual acceptance.

## Rule-selected decisions

| Decision | Selecting rule |
|---|---|
| Consume pm-design; use system-owned/unspecified geometry where the spec has no value | Design ownership; N1: no lane-invented visual defaults |
| Keep diagnostics non-destructive and all previous UI/compatibility paths | N4/N6 and prune rule: no existing consumer has retired |
| Separate accepted surface, worker recipe publication and native action queue | N7/N8; pm-kernel single writer: headless ID inspection/drive without UI-thread JS waits |
| Follow tagged NumField's `dx - dy` and three-decimal drag rounding, correcting the scaffold's dominant-axis calculation | N3 and W073 B550/B580: exact authored two-axis behavior; diagonal cancellation is tested |
| Evaluate a typed relative arithmetic draft before stepping it | N3 and quirk policy: arithmetic intent wins over legacy parseFloat prefix truncation; typing and scrub clamping remain distinct |
| Validate actual hit geometry as well as token references; expose all descendants to AccessKit | N1/N2/N7: a token annotation alone does not prove usable targets or accessibility |
| Keep native renderer and export/alias cutover explicit integration requirements | Ownership and N3: Linux contracts cannot certify AppKit/GPUI behavior |

## DESIGN-TODO(opus) resolutions (design review 2026-11-01)

Review: [design/reviews/na-cloud-uikit.md](design/reviews/na-cloud-uikit.md).
Rules: [design/panels/README.md §5](design/panels/README.md#5-resolutions-of-the-ui-kit-design-todos).

1. **Segmented-track padding** = `control_radius − inner_radius` (concentric
   math, no new token). Implemented as `Recipe::track_padding`.
2. **Wheel editing** — designed: a wheel event edits a hovered number only
   after the pointer rests `motion.wheel_rest` (0.5 s, new token) with no
   scroll of the container in that time; discrete wheel, or trackpad with ⌥;
   one undo step. Still unbound in code until the token and gesture land
   (follow-up F1).
3. **Colour picker contents** — specified in
   [design/panels/inspector.md §7](design/panels/inspector.md#7-colour-popover);
   the host owns the popover.
4. **Tree indentation** = `metrics.tree_indent` (16, new token; AppKit's outline
   default). Disclosure `chevron.right` at `symbol_inline` in a `hit` square.
   Until the token lands, hosts use the system outline indentation (also 16)
   (follow-up F2).
5. **Checkbox/switch placement** — inspector and panel rows use a checkbox at
   the leading edge of the value column, untitled; `Toggle` now renders and
   announces as a checkbox. Switches appear only in Settings.

## macOS verification and integration still required

Wire `PanelHost` to AppKit/GPUI, kernel Bindings and the native input driver;
wire compiler/UI exports and reconcile the shared lock. Verify real control
layout/painting, units/precision and stable tabular values; two-axis infinite
pointer scrubbing, pointer restoration/cancel and haptics; native text services,
IME/dictation/clipboard/selection; pop-up menus, sheet placement, popover outside
and Esc dismissal, live undo and focus restoration. Exercise keyboard traversal
and VoiceOver actions through the real window accessibility subtree and confirm
actual padded hit rectangles at 1×/2×. Prove no UI-thread JS waits.

Review gate: **pending, cannot self-certify**. Native light/dark 2× screenshots,
Increase Contrast/Reduce Transparency/Reduce Motion captures, before/after pairs,
all changed interaction states, keyboard walkthrough, VoiceOver/AX evidence,
contrast spot checks and Opus sign-off are still required by the design review
checklist. All visual output is provisional until that review.
