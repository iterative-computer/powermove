# Native theme pipeline (P2/P3 package 12)

**In production (2026-11-08):** the compat realm bundles these modules and
publishes the active v1–3 theme's `nativePatch` (`js/compat/src/native-theme.ts`);
the app applies it to the shell's tokens (`app/src/extensions.rs`,
`apply_v1_theme`). Verified on a co-founder's After Effects theme. Surface
rules beyond tokens are not yet applied natively.

Implementation: `apps/native/js/themes`, dependency-free ECMAScript modules for
the trusted V8 realm. Node is used only for tests and fixture generation. There
is no browser, Node API, network access, disk writer or UI-thread call in the
runtime modules. No Rust crate, design, kernel, shell, engine, vendor or spike
source was changed.

This implements translation, semantic-tree evaluation, ownership reducers and a
publication port. It does **not** implement the GPUI renderer, production V8
module loader, remote-DOM transport or kernel persistence adapter. Passing these
Linux tests does not certify visual equivalence or cutover. Unsupported shell
features remain explicit native requirements; there is no browser shell fallback.

Baseline: [T1](t1-themes.md), shipped `v1.2.0` at
`07ed5fa17ee640659cb075f9cad1af2095011d60`. Architecture follows
[P0 decisions](p0-decisions.md), [kernel](kernel.md), [S1](s1-v8.md),
[S5](s5-compat.md), [C-MATRIX](c-matrix.md), [D-STATE](d-state.md),
[W1](w1-workflows.md), [keymap](w1-keymap.md), [traces](traces.md),
[MCP](mcp.md), [engine service](engine-service.md) and
[first launch](first-launch.md). The adopted architecture in P0 supersedes older
S5 alternatives: extension logic stays in V8; WebKit renders compat content only.

## Input and output contract

`translateTheme(definition, options)` accepts the shipped
`{id,name,scheme,tokens,darkTokens,css,rootAttributes}` shape. Options specify
the user preference, actual system scheme, current workspace, trusted/Store
registration, and target document. It returns a serializable version-4 program:

- Original definition for a **private** archive; arbitrary authored root
  attributes and custom properties are retained.
- Scheme and root predicates, inline token contributions, workspace appearance,
  stable-id dock/panel geometry, ordered surface rules and source provenance.
- Selector ASTs, specificity, media/container/supports conditions, declaration
  domains, native motion blocks, compat rules and unresolved native requirements.

The 82-token frozen foundation is included by default. User/dark inline token
values beat normal stylesheet declarations; CSS `!important` beats normal inline
values. Workspace tokens apply last at inline priority. `rootAttributes.style`
has its authored `setAttribute` replacement semantics: it replaces the theme
inline dictionary, after which workspace tokens apply. Arbitrary root classes
and audio/environment predicates participate in matching. Live diagnostic
attributes are inputs, not imported appearance preferences.

`workspaceAppearance` translates accent/bg/panel/text/line/font/mono, the exact
12% sRGB accent wash and 82% Oklab accent-text expression, and
`--r-base = max(2, radius) / 3 px`. It retains density, preview corners,
timeline-surface order, arbitrary chrome fields and dock/panel geometry.
Compact/comfy overrides use the frozen foundation rather than a second palette.

`evaluateTheme(program, tree, environment)` produces:

- An id-keyed surface snapshot, with separate base/decorations/highlight slots,
  inherited token values, custom-property dependencies/cycles and provenance.
- All resolved v4 token values, typed representations where supported, a patch
  for existing `pm-design::Tokens` fields, and required design additions.
- Timeline track/gutter/text/line/accent inputs, gizmo accent and preview-corner
  state. Reversed timeline surfaces swap painter inputs, not project pixels.
- Retained motion/compat rules and blockers for unsupported native features.

The semantic tree uses `{id,cssId?,tag,classes?,attributes?,states?,text?,children?}`.
Ids are unique, stable automation ids; `cssId` can preserve a legacy CSS id
without changing the automation id. Named/unnamed query ancestors supply
`container: {name?,width,height}`. Nodes inherit a `document` boundary; the app
document is `shell` (including native panels), a separate translated document
may be `native-content`, and legacy documents do not receive shell selectors.
The compat bridge receives its own rules and an explicit palette snapshot.
Environment supplies viewport dimensions, root font size, system appearance,
accessibility preferences, hover/pointer state and optional supports/pseudo
callbacks. Missing required dimensions and unknown conditions are diagnostics,
never silently false branches.

Selectors cover every family in T1: ids/classes/types/universal selectors,
all six attribute operators and flags, descendant/child/adjacent/general sibling
combinators, root/ARIA/data/native states, structural/nth selectors,
`:is/:where/:not/:has`, Svelte `:global` source wrappers, pseudo-elements and
named highlights. Relative `:has` selectors are anchored to the candidate
surface; `:where` contributes zero specificity. Generated Svelte hashes are
not a native selector contract.

Custom properties remain case-sensitive. Aliases compute at the element where
they are declared, then inherit their computed value. Cycles include references
inside fallbacks; missing/cyclic values follow fallback and invalid-at-computed-
value rules. Rich expressions such as `color-mix`, shadows, masks and easing
retain their exact expression text for the native style evaluator. The small
numeric evaluator projects exact supported `calc` arithmetic to native metric
fields; it does not approximate unsupported expressions or negative dimensions.

Surface operations have layout, paint, text, interaction, material, motion and
decoration domains. **The native host must implement property grammar and
shorthand expansion.** Operations are ordered by importance, specificity,
rule order and declaration order, retaining authored values and reset operations
for invalid computed values. The `style` dictionary is an inspection of winning
property declarations/inheritance, not a browser-computed longhand style dump.
Use the ordered operations to resolve shorthand/longhand overlap. Geometry
operations include display/grid/flex, sizing, padding/gaps, positions, collapse,
overflow and stacking; they must drive actual layout, clipping and hit bounds.
Docking, tabs, splitters, popouts and layout always take this native path.

Electron drag CSS lowers to `window-drag-region`; prefixed selection/material
properties lower to semantic properties. Font-smoothing/text-rendering directives
are recorded platform decisions. Native scrollbar/control decorations and native
focus/accessibility behavior remain responsibilities of the host. A theme's
`outline:none` does not authorize suppressing the native accessible focus ring.

Unknown properties, pseudos, conditions and at-rules (including currently
unlowered layers/imports/font-face/scope) retain their source and report a
native requirement. Malformed input raises a structured `ThemeError` before a
replacement can be registered. The imported original remains an archive input;
the existing working edition must stay available until the missing translation
has an implementation. No network imports are fetched by this pipeline.

## Reversibility and ownership

`AppearanceLedger` is a pure reducer. It records per-scope/per-field base values
and owner stacks, rebases on external user writes, and returns conditional
restoration patches. Missing values have an explicit `{present:false}` tombstone;
null, empty string and absence are distinct after JSON restart. Removing a buried
owner leaves the visible owner unchanged; either removal order restores the base.
A later user value/deletion survives release. Unavailable saved workspaces stay
pending rather than losing their ownership record.

`workspaceFields` and `applyWorkspacePatches` address dock and panel geometry by
stable ids, preserving reorder/additions and never recreating deleted docks or
panels. Container markers restore absent/scalar/object shape; cleanup removes
only empty containers after leaf restoration, preserving user-added leaves.
`captureWorkspaceMutation` wraps the existing synchronous V8 facade, records in
`finally`, returns the original result and rethrows the original error. Partial
mutations on throw remain visible and reversible, as required by C-MATRIX.

`importLegacyAppearance` imports the version-1
`extensionWorkspaceAppearance` and `extensionThemeAppearance` owner/base/stack
formats. It retains their complete original objects, unknown fields and rejected
entries in the private archive. Unknown versions/unsafe entries produce
diagnostics; this is not permission to reset them to empty state.

`ThemeController` supplies the synchronous trusted theme register/activate/active/
list/scheme/setScheme surface, LIFO override handles, stable registered objects,
immediate reads, theme-change notifications, fixed/auto/system distinction,
reload restoration and deferred restoration of an unregistered prior theme.
Store registration strips root attributes and preserves the restrictive
root/html/body custom-property CSS grammar, qualifies theme ids into the owner's
namespace, rejects cross-owner collisions and denies `setScheme`. Store reads
return effective appearance; trusted reads retain the user's preference. Store
catalog copies cannot mutate host registrations. No DOM permission is added. The
generic kernel registry still owns full production activation/batching; this
standalone controller is an adapter seam, not a second application authority.

Disposed/replaced contributions and ownership records are archived, not erased
from user storage. Exported ledger/controller state is private user data and
must go through the kernel actor's durable state transaction. A kernel adapter
must apply returned patches to current values and persist the reducer state
atomically; it must retry unavailable workspaces and reconcile missing owners
before publishing an appearance. This module does not perform those disk writes.
The original Electron code still has consumers and was not pruned.

## Publication and required integration

`ThemePublication` exposes the native-host port
`replace({generation,snapshot})`. Call `request()` **before** scheduling worker
translation. Only the current generation can commit. The snapshot is copied at
publication so retained JS references cannot mutate native state. The native
implementation must enqueue an atomic replacement of tokens, predicates,
surface rules, decorations, layout and painter inputs. Removing a theme publishes
a newly evaluated baseline, which removes every old theme-only contribution.
Never merge an old theme's rule dictionary into the replacement.

The Linux test host implements the same port with an in-memory snapshot. The
macOS port must live with the shell owner; no AppKit/GPUI/WebKit implementation
was added here. V8 compilation/evaluation runs on the realm/worker, native UI
uses published data, and the main thread never waits for JS.

```js
import {
  translateTheme,
  evaluateTheme,
  ThemePublication,
  cutoverRequirements,
} from "./apps/native/js/themes/index.mjs";

const publisher = new ThemePublication(nativeHost);
const generation = publisher.request(); // capture before scheduling worker work
const program = translateTheme(definition, {
  workspace,
  preference: "system",
  systemScheme: "dark",
});
const snapshot = evaluateTheme(program, semanticTree, environment);
const missing = cutoverRequirements(snapshot, nativeCapabilities);
// Previewing an incomplete program is not a cutover certificate.
publisher.commit(generation, snapshot);
```

`cutoverRequirements` checks unresolved features, missing token representations,
host property support, motion and compat support. A property capability means
the host implements its full grammar/cascade/reset semantics, including related
shorthands; declaring a name alone is not conformance. No successful native visual
or compatibility verdict is generated by this module.

Integration requested outside this lane:

1. Kernel-owned appearance snapshot/state publication, conditional restoration,
   migrated ownership import and durable private archives. Existing state APIs
   remain the only writers.
2. V8 module loading, production theme/workspace facade hooks and trusted DOM
   style/class/attribute mutation capture; compile shell effects from those
   mutations through the same path. Raw DOM compatibility is not implemented here.
3. Shell semantic-node/state/container registry, complete native CSS-value and
   shorthand evaluator, native motion/material/scroll/control implementations,
   atomic layout/hit-test replacement and asynchronous publication port.
4. Reactive palette delivery to timeline/gizmo/curve/mixer painters, native
   popouts and compat-document remote-DOM style/palette sinks. The publication
   carries a full snapshot suitable for popout creation; physical popout behavior
   remains a host test.

## Design token mapping and additions

`tokens.mjs::FOUNDATION_TOKENS` is the exhaustive machine-readable map.
**27 of 82** foundation names map to existing native fields. `crates/design/tests/spec.rs`
checks every destination against `Tokens::PATCHABLE`.
Opaque sRGB colours project to the current `u32` fields; nonnegative px metrics
and supported arithmetic project to `f32`. No alpha is flattened or composited
onto a guessed background. The other **55** tokens receive typed v4 destinations
under `legacy.<kind>.<name>`; custom extension variables use `custom.<name>`.

| Existing native fields                                                                   | Legacy inputs                                                                       |
| ---------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `colors.window/panel/panel_secondary/viewer/field/floating/hover`                        | `--bg-window/--bg-panel/--bg-panel-2/--bg-sunken/--bg-field/--bg-float/--bg-row-hi` |
| `colors.separator/text/secondary/muted/accent/selected_text`                             | `--line/--tx/--tx-2/--tx-3/--accent/--on-accent`                                    |
| `metrics.control/gap/border/header/inset/panel_radius/control_radius/capsule_radius/row` | `--ctl-h/--gap/--hairline/--hdr-h/--pad/--r-lg/--r-sm/--r-pill/--row-h`             |
| `typography.minimum/label/body/title/display`                                            | `--fs-xs/--fs-sm/--fs-md/--fs-lg/--fs-xl`                                           |

Needed additions, for the design crate owner (not edited here):

| Kind                    | Foundation tokens without native fields                                                                                                                                                                                                                                                                      |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Colour (24)             | `--accent-dim`, `--accent-hover`, `--accent-tx`, `--bg-rail`, `--bg-row`, `--blue`, `--blue-deep`, `--cream`, `--danger`, `--gray`, `--green`, `--ink-1`, `--ink-2`, `--ink-3`, `--line-2`, `--line-strong`, `--red`, `--sand`, `--section-line`, `--success`, `--tx-4`, `--violet`, `--warning`, `--yellow` |
| Channel tuples (2)      | `--ink-rgb`, `--paper-rgb`                                                                                                                                                                                                                                                                                   |
| Geometry (8)            | `--gut`, `--hit`, `--panel-edge-inset`, `--r-base`, `--r-md`, `--r-xl`, `--r-xs`, `--val-w`                                                                                                                                                                                                                  |
| Font families (2)       | `--f-mono`, `--f-ui`                                                                                                                                                                                                                                                                                         |
| Number (6)              | `--fw-medium`, `--fw-regular`, `--fw-semibold`, `--lh`, `--lh-tight`, `--disabled`                                                                                                                                                                                                                           |
| Shadow/focus stacks (7) | `--ctl-edge`, `--ctl-edge-inset`, `--focus-ring`, `--shadow-float`, `--shadow-lift`, `--shadow-panel`, `--shadow-raise`                                                                                                                                                                                      |
| Paint (1)               | `--ctl-raised`                                                                                                                                                                                                                                                                                               |
| Durations (3)           | `--dur-1`, `--dur-2`, `--dur-3`                                                                                                                                                                                                                                                                              |
| Easing (2)              | `--ease`, `--ease-io`                                                                                                                                                                                                                                                                                        |

Existing colour fields also need a floating RGBA/colour-space representation and
expression support: the default `--line`, `--tx-2`, `--tx-3` contain alpha; themes
may author alpha/wide-gamut/mixed colours for any colour field. Font-family
fallbacks must be data rather than only the current font constants. Multi-layer
focus rings are not one `colors.focus` colour; CSS easing/duration intent is not
the same as the existing spring enter/bounce/exit parameters. Surface-rule
geometry/materials/motion and open extension token namespaces need first-class
native representations in addition to the flat baseline token struct.

## T1 compat selector list

`compatSelectors(catalog)` returns all **19 occurrences**, preserving source,
line, condition and surface. These dispositions apply to their owning document;
`body` in a legacy document does not make the app body a compat shell.

| Source at v1.2.0                  | Line | Exact selector(s)                                                       | Occurrences |
| --------------------------------- | ---: | ----------------------------------------------------------------------- | ----------: |
| `panels/agent/VisualReply.svelte` |  146 | `iframe`                                                                |           1 |
| `renderer/sandbox/boot.ts`        |  149 | `html`, `body`                                                          |           2 |
| Same                              |  150 | `html`, `html::-webkit-scrollbar`                                       |           2 |
| Same                              |  151 | `html[data-noscroll]`                                                   |           1 |
| Same                              |  152 | `body`                                                                  |           1 |
| Same                              |  153 | `input`, `textarea`, `[contenteditable]:not([contenteditable="false"])` |           3 |
| Same                              |  154 | `.pm-view-error`                                                        |           1 |
| `panels/agent/visual-reply.ts`    |   62 | `:root`, `html`, `body`, `*`, `button`, `input`, `select`, `a`          |           8 |

Full paths and all **3,329 occurrences in 74 files** are in
`fixtures/t1-catalog.json`. There are 3,310 native and 19 compat dispositions.
Arbitrary installed selectors are unbounded: a theme that requires an unchanged
legacy content DOM keeps that content's compat edition until equivalence is
verified. Unknown shell effects always require native implementation.

## Decisions and selecting rules

| Decision                                                                                       | Selecting rule                                                                                                                                                                                |
| ---------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure V8 modules and asynchronous copied publication; Node only in tests                        | Adopted V8 architecture and N8; Linux cannot validate GPUI/WebKit.                                                                                                                            |
| Exact selector ASTs, conditions, dependencies, operation ordering and original expressions     | N3 authored intent; N6 CSS/appearance behavior. Token copying alone loses geometry and cascade.                                                                                               |
| All shell rules lower to semantic native operations; unsupported parts block certification     | N1/N2 and T1's mandatory shell translation; no shell compat edition.                                                                                                                          |
| Typed additional tokens and explicit unrepresentable values instead of guessed colours/metrics | N3; current `pm-design` cannot encode every authored value.                                                                                                                                   |
| Owner/base/stack reducers, absent-value markers, conditional cleanup, private archive          | N4/N6 and D-STATE. A user edit or unknown state must survive disable/restart.                                                                                                                 |
| Stable-id surface/geometry addressing, serializable diagnostics and generations                | N7 inspectability/driving by id; N3 rejects stale worker results.                                                                                                                             |
| Keep Store token-only CSS/root-attribute restrictions; reject uppercase URL syntax too         | N6 preserves the permission boundary; N3/spec intent fixes a case-sensitive URL check rather than reproducing it. Token dictionaries retain the shipped path.                                 |
| Theme notifications describe effective fixed/auto appearance                                   | T1's resolved-scheme contract and N3; a fixed theme stays fixed when the user preference changes. This intentionally corrects the kernel-only `setScheme` event's contradictory scheme value. |
| Native rasterization/focus/scroll behavior; preserve authored decoration/layout                | N1/N2 and N3 quirk policy; browser smoothing and blanket focus suppression are not authored design requirements.                                                                              |
| Retain all old consumers and their source                                                      | Prune rule and N4; no production host replacement/visual gate has passed here.                                                                                                                |

## Linux verification and remaining gates

**3,401 tests pass:** 3,332 catalog/foundation/compat tests (including all 3,329
selector occurrences), 30 ownership/state tests, 27 pipeline/publication tests,
10 selector-matching tests and 2 native-field/frozen-source oracle tests. The
test runtime is Node v24.19.0; the isolated VM test supplies only V8 intrinsics.

Run from the repository root:

```sh
source /workspace/.tools/activate.sh; cd /workspace/powermove
node apps/native/js/themes/scripts/catalog.mjs --check
node --test --test-reporter=tap apps/native/js/themes/test/*.test.mjs
```

The generator reads public Git objects and T1, checks the inventory hash/counts
and verifies the frozen foundation/built-ins. It never reads installed themes or
user data. Every catalog occurrence round-trips selector AST → JSON → selector
AST and translates through its condition/disposition with no lost rule. The
synthetic studio fixture covers token/dark/workspace layers, docking, panels,
grid layout, tabs, `:has`, decorations, named containers and native motion IR.
Additional tests execute actual frozen scheme/token function bodies, check all
native field names, run the runtime in a VM with only V8 intrinsics, and cover
both owner disposal orders, reload, removal, restart, two workspaces, delayed
registration, later user writes/deletions, source errors and stale publication.

This container has neither `/workspace/.tools/activate.sh` nor Cargo/rustfmt/
Clippy. The allowed JS-only implementation introduces no Rust crate or lockfile
change; Rust build/test/fmt/Clippy checks do not apply to these files. JS syntax,
formatting, all tests, whitespace and pre-commit secret scans are checked here.

macOS verification still required: production sandboxed-V8 loading and realm
generation fences; atomic kernel state/archive persistence; GPUI layout, text,
native focus/IME/AX, hit/drop/resize bounds, named container updates and popout
snapshots; dynamic Reduce Motion/Transparency/contrast and material fallbacks;
remote-DOM CSS/palette delivery to the 19 compat occurrences and arbitrary real
legacy content; painter invalidation without changing project/export pixels.
Run the T1 visual oracle and real-theme enable → disable → enable, reload,
removal, restart, two saved workspaces, overlap in both disposal orders and later
user-edit preservation. No private theme/profile or screen was accessed here.
