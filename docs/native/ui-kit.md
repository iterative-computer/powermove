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

## v4 integration reconciliation (2026-11-01)

The strict primitive contract is retained: anchor fixtures create `anchored`,
matching the JSX components, instead of putting `anchor` on a `div`. No recipe,
token, visual property or Opus interaction decision changed. JSX declarations
explicitly enumerate the kit's accessibility and semantic markers. Component
options/value callbacks override primitive event signatures rather than
intersecting incompatible types; boolean list selection stays boolean.

Plain `bun test` now builds the standalone native examples and runs native
conformance instead of failing for a missing environment variable. Current
combined counts and freeze gates are in [v4-eval.md](v4-eval.md). The older kit-only
counts below remain historical evidence. New design TODOs: **none**; the existing
Opus F1–F7 follow-ups and rendering-review gate remain unchanged.

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
# The merged workspace lock resolves on Linux; standalone uses the same sources.
cargo test --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface
cargo clippy --locked --manifest-path crates/surface/standalone/Cargo.toml -p pm-surface --all-targets -- -D warnings
cargo fmt --manifest-path crates/surface/standalone/Cargo.toml --check
cargo build --locked --manifest-path crates/surface/standalone/Cargo.toml --examples
cd js/v4
bun install --frozen-lockfile
bun test # builds both kit/surface wire validators and host_wire automatically
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

## UI kit part 2 — additive panel kit (2026-11-01)

This section supersedes the earlier F1/F2 implementation status. The six-op
protocol and v1–3 extension paths are unchanged. Added exports: `SettingsRoot`,
`Switch`, `SecureField`, `FormGroup`, `FormRow`, `PushButton`, `ColourPopover`
(`ColorPopover`), `ViewerGizmo`, `ViewerOverlay`, `Banner`, and `ColorWell` as an
alias of `ColourWell`. `ColourWell` accepts `minimal`; `EmptyState` accepts an
optional decorative SF Symbol slot and an optional action. Pop-up options accept
separators, key equivalents and details; `widthSample` requests native measurement
of that string plus the system chevron. Separators are absent from primitive
selectable options, so neither keyboard nor binding edits can select one.

The new components publish through `Panel::components`, `Panel::decorations` and
AccessKit. `Component::ALL` retains the original catalog; `Component::catalog()`
adds `ADDITIONS`. Settings switches require `metadata.hostWindow: "settings"` on
the top-level content root (`SettingsRoot` provides it). The shell must verify
that declaration against its actual host window. Existing `Toggle` remains a
checkbox. Form titles sit outside the card; native form layout centres controls
on the label line and keeps wrapped detail below it. Native group separators
use the specified leading inset. Increase Contrast enables field/card outlines
using `metrics.border` and `colors.separator`, and strengthens gizmo keylines.

`SecureField` accepts an opaque native binding, with no value, child text or
input/change callbacks. The JSX entry rejects those inputs before publication;
AX projection never includes a secure value or text-setting action. Raw invalid
surface descriptors receive `secureValueOnSurface` diagnostics, not a claim that
the generic surface is a secret store. The AppKit adapter must keep secure editor
buffers and Keychain operations entirely outside the surface and JS, using
`SecureFieldHost`. The Linux `InspectSecureFieldHost` records only IDs, binding
references and native commands.

`ColourPopoverHost` owns the native picker and `NSColorSampler`; the Linux
`InspectColourPopoverHost` records its commands. The picker recipe describes the
SB area, hue/opacity controls, geometry, Fill modes, document recents and required
native AX axes. `ColourEdit` starts a binding transaction only on preview;
outside dismissal commits, Escape cancels, and opening/dismissing without editing
never normalizes or rewrites an authored colour string. The native adapter still
must create/graft the picker controls (including hex, gradient stops and recents)
and their live values, implement the specified rings/checkerboard using the
design tokens, and report native dismissal to the worker. The recipe and Linux
AX projection do not certify a rendered colour picker.

Viewer handles describe transform, rotation, anchor, path vertices and tangents
in display points, retain authored positions, pad targets to `metrics.hit`, and
use the dark palette over media. `Panel::over_image` includes plain text and
symbol descendants so their token styles also resolve with that palette.
`prepare_with_palettes` accepts the active theme's dark tokens while the window
is light; the simpler preparation methods use standard dark tokens in a light
window and retain supplied theme patches in a dark window. The production host
must supply the theme's dark palette. Viewer overlays use the permitted glass
material with solid dark fallback, enter/exit cross-fades, Reduce Motion timing,
and a token deadline that pauses for hover, an open menu or keyboard focus.
`BannerSlot` holds one banner, replaces its deadline and dismisses after the
specified four seconds unless it has an action. Banners publish medium AX/live
announcements (high for errors); inline notices use solid panel-secondary fill.

`NumericWheel` consumes native normalized notches only after both pointer rest
and absence of actual container scrolling meet `motion.wheel_rest`. Pointer
motion beyond `metrics.drag_threshold` restarts the gate. Discrete wheels are
eligible; trackpads require Option, which gates them without reducing the step.
Shift multiplies by ten; Option on a discrete wheel multiplies by one tenth.
One sequence uses one Bindings transaction, closing after the rest interval
following the last event, including events clamped at a bound. Failed queue
operations are retryable; an unwritten begin is cancelled on timeout. Retired,
disabled or rebound controls cancel the original binding. The host must notify
actual scrolling (including momentum), normalize native device units, schedule
`tick`, call `leave`, and cancel before Bindings reconciliation on retirement.
No event is captured while the rest/modifier gate is closed.

### Design-token dependency and neutral placeholders

**Update (design lead, 2026-11-01, `feat/na-design-fix-kit2`):** pm-design now
carries every panels/README §6 token (metrics, `motion.wheel_rest`,
`motion.overlay_idle`, `colors.media_badge` + `metrics.media_badge_opacity`,
light `colors.viewer` `#E4E4E4`), so `DesignTokenTable::from_design` resolves
them and the missing-token path is exercised with an older host table. The
paragraphs below describe the lane's state before that.

At this lane's base, the pm-design struct/table does **not** yet contain the
README §6 additions. That crate and its token snapshot are outside this lane's
owned paths. No production token values were duplicated into the kit.
`DesignTokenTable::from_design` resolves available names;
`prepare_with_token_table`/`prepare_with_palettes` accept a newer, host-owned
pm-design serialization. New recipes expose unresolved names and
`missingDesignToken` diagnostics instead of substituting geometry. NumericWheel
and overlay timing refuse construction without their token. Synthetic test
values exercise the exact specified 0.5-second rest, 16-point indentation and
other approved measurements; they are not runtime defaults.

**Required from the pm-design lane:** `motion.wheel_rest`, `metrics.tree_indent`,
`metrics.group_radius`, `metrics.form_row`, `metrics.gizmo_handle`,
`metrics.gizmo_keyline`, `metrics.rotation_stem`, `metrics.anchor_mark`,
`metrics.tangent_end`, `metrics.checker`, `motion.overlay_idle`; the containing
Settings panes still need the specified sheet/section tokens, and native
popover/gizmo painting needs `colors.media_badge`. This is an integration
dependency, not a missing design spec. New `DESIGN-TODO(opus)` items: **none**.

### Decisions and verification

- Add metadata/recipes instead of changing primitive option schemas or retiring
  old components: frozen v4 contract, N4/N6 and prune rule.
- Resolve current design tokens, report missing additions and keep neutral
  placeholders: design ownership and N1; no invented visuals or fallback values.
- Native secure text and colour-sampling seams expose only IDs/commands;
  gestures enqueue binding edits and remain inspectable without a JS wait:
  N7/N8 and the kernel single-writer boundary.
- Preserve authored colour strings and exact gizmo coordinates; use documented
  wheel gates/modifiers instead of browser wheel quirks: N3 and quirk policy.
- Strengthen contrast, preserve active dark-theme patches, suppress accidental
  wheel edits while scrolling, and test failed queues/cancellation: N1/N2/N4.

Linux validation: **129 pm-surface tests**, **242 Bun tests**, **28 snapshots**;
no skipped tests. New part-2 coverage includes 34 Rust tests and 16 Bun tests with
10 wire snapshots. All JSX snapshots replay through the real Rust surface/panel
validator; Rust tests assert AX roles/actions/redaction, token resolution,
contrast/transparency/motion variants, native command doubles, deadlines and
transaction/error paths. Existing Rust/JS token-literal guards pass. No dependency
was added, so neither Cargo lockfile needs a change. Commands:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
cargo test --locked -p pm-surface
cargo clippy --locked -p pm-surface --all-targets -- -D warnings
cargo fmt -p pm-surface --check
cd js/v4
bun install --frozen-lockfile
bun test
bun run typecheck
bun run format
bun run build
```

macOS remains required: connect the actual AppKit/GPUI host and panel ViewModel
adapters to the publications/traits; verify secure text services and Keychain,
colour picker/NSColorSampler/gradient keyboard and AX subtrees, native menu
measurement and key equivalents, Settings form alignment and regular switches,
actual hit rectangles, wheel/trackpad units and momentum, focus preservation,
Focus/full-screen idle fade and banner placement. Supply light/dark and all
accessibility-variant captures and VoiceOver evidence for Opus review. All visual
output remains provisional; this Linux implementation is not native visual
acceptance or a renderer integration claim.
