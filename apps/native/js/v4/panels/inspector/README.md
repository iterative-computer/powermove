# Properties built-in v4 panel

Wave 4 implementation of panel id `inspector`, title `Properties`.
Design source: `docs/native/design/panels/inspector.md` and its shared README.
**This is a tested panel and model adapter, not a production activation or full
spec-parity claim.** Visual output remains provisional until Opus review.

## Entry and authority

`mountInspector(session, transport, {retire})` mounts the JSX entry through the existing
six-op renderer. `InspectorSession` subscribes through the public `HostClient`
to `project`, `selection`, `time`, and `evaluated`; the injected `InspectorModel`
projects immutable publications and plans authorized host commands/transactions.
The root shares the host generation; its independent retirement callback releases
the session and invokes the required native retirement hook, including a fenced
root. No JS document writer, browser layout query, DOM, Node, or Chromium is required.

`contracts.ts` defines the presentation and native-service interfaces. The model
supplies ordered sections, applicability, first-layer identity, lazy mixed values,
exact authored values, binding capabilities and actual commands. The view never
walks a selected-layer collection. It shows composition controls and the specified
hint when no layer is selected, and only the first layer's fields for a large
selection. Stable row/axis keys preserve accepted node ids on value updates.

`pm_panel_inspector::v4::V4Adapter` is the thin native model adapter. It registers
opaque handles, refuses stale selection/document handles, reads `FieldState`,
plans absolute sets and property actions, and begins captured model gestures.
The native driver streams `Inspector::edit_step` and `finish_edit`; selection or
time changes cannot retarget them. Graph actions return model service intents.
Option-click removal explicitly declines on static fields and finds animated
secondary targets when the primary is static. Caller identity is preserved.

Only acknowledged bindings enable fields. Disposal releases subscriptions and
bindings, including admission races. Native binding disposal must cancel original
gestures before releasing their handles. Playback bursts coalesce through an
injected worker readout/display clock; pause publishes immediately. The existing
model's 15 Hz clock can feed this seam; the view introduces no competing timer.
Rejected edits are reported through host diagnostics and never optimistically
change authored values. Large edits are never split to evade the host's
256-command atomic-transaction limit.

## Spec section coverage (2026-11-01)

“Done” below is source/wire/model coverage. Native painting and production
integration remain unverified for every section.

| Inspector spec        | Status                        | Implemented / outstanding                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §1 Model contract     | Partial                       | Typed target/sections/row contract, four host subscriptions and Rust model adapter done. App publication-to-presentation and command dispatch adapter still required (G1).                                                                                                                                                                                                                                  |
| §2 Anatomy and order  | Partial                       | Properties header, identity, one-stop momentary alignment strip, model-ordered disclosures, fixed-column rows, bound number/text/choice/checkbox/colour/segmented controls, expression/error rows and composition hint done. Glyph/heading/axis/text/font/fill recipes incomplete (G2–G5).                                                                                                                  |
| §3 States             | Partial                       | 21 accepted-op-tree/AX snapshots cover empty, composition, selected, font loading request, hover, focus, editing, scrub, animated, playhead key, expression, mixed, partial applicability, locked, disabled, error, drop, playing, 60k selection, collapsed and audio. Hover/focus/font-loading snapshots cover native delegation, not rendered rings/menus. Error-driver and mixed AX gaps remain (G4/G6). |
| §4 Interactions       | Partial                       | Key/Option-key, context-menu descriptions, link, align, disclosure, section-action intents, native menu/popover requests and effect drop routing done. Real model tests prove edit/undo/locks/keyframe semantics. Native menus/pickers, parent whip and wheel arbitration remain (G1/G3/G5).                                                                                                                |
| §5 Keyboard and focus | Partial                       | Header actions, alignment roving focus, left-to-right axes, row diamonds/link, collapsed/disabled exclusion and segmented routes tested. Existing Rust numeric/toggle/select/text paths tested without JS callbacks. Native draft ownership, F2, second-Escape timeline focus, popover trap/restore and heading native activation remain (G3/G4).                                                           |
| §6 VoiceOver          | Partial                       | Real AccessKit tree cardinality, labels, numeric units/step/actions, checkbox mixed, colour role and depth-first focus order asserted. Mixed-number/colour/expression and glyph semantics require G2/G4/G6; real VoiceOver is a Mac gate.                                                                                                                                                                   |
| §7 Colour popover     | Not done in this lane         | ColourWell delegates to the host colour popover; Fill uses a neutral host trigger. SB/hue/alpha/gradient/recent colours/eyedropper/trap/live-undo require the host implementation (G5).                                                                                                                                                                                                                     |
| §8 Appearance         | Partial                       | Token-only authoring and 16 appearance combinations validate. Real contrast edges, collapse motion, transparency and native painting remain Mac gates.                                                                                                                                                                                                                                                      |
| §9 Copy               | Done for implemented controls | Exact spec labels, hints, errors, menu rows and New Layer items. No invented product error text; D2 remains. Native menu/picker copy remains the host's responsibility.                                                                                                                                                                                                                                     |
| §10 Don't             | Done in panel source          | Left-aligned kit values, shared value column, checkbox, composition instead of No Selection, no effects stack, no sheet colour picker, no web export. All legacy consumers retained.                                                                                                                                                                                                                        |

The shared owner decisions are honored: this panel makes no layer-list width,
agent placement or export choices, and introduces no `Effects & Presets` title.

## API gaps outside the owned implementation

- **G1 — app host integration:** register the entry under `inspector`; implement
  `InspectorModel` using the model's immutable `Published`/`QueryMirror` and the
  actual application `KernelData` adapter. Translate `Intent` to the Rust model
  adapter/typed actor calls, and route native bindings through captured model
  gestures. The current v4 service deliberately prescribes no application query
  schema. This lane does not invent a success-shaped backend or raw document
  commands. The Rust adapter is real; the application hookup is outstanding.
- **G2 — glyph/button recipes:** the kit has no SF Symbol prop, diamond shape
  states, empty-state symbol, identity lock symbol, or momentary segmented
  recipe. Header/diamond/link/alignment actions use neutral, labelled kit
  controls. Required header optical inset and Scale-overlap/token arithmetic
  need a supported recipe; no literal geometry substitutes are introduced.
- **G3 — disclosure and native interaction:** expose a trailing heading action
  slot (before the chevron), heading hover/focus glyph state, per-project section
  persistence and scroll-anchor publication/restore. The current section action
  is a neutral kit action in disclosure content, pending the real heading slot.
  Implement native context-menu placement, parent whip/cancel, rested-wheel
  arbitration, tooltip detail lines and timeline-focus restoration. The spec's
  exact multi-layer alignment target also needs D1 resolved.
- **G4 / I1 — multiline and input errors:** TextField is the existing single-line
  native recipe. The kit needs 2–6 line text and 1–6 line monospaced expression
  recipes, validity glyph/status, numeric error observations that keep the draft
  open, and native edit/readout ownership. The panel preserves authored strings
  and inline model-provided errors; it does not emulate AppKit text in JS.
- **G5 / I2 — font/fill/point/colour hosts:** Font and Fill have neutral kit
  triggers with authored family/gradient text and native requests. Point has
  real numeric axes and a neutral pad trigger. Implement field-menu search,
  fonts/loading/refresh, correct current-item NSMenu placement, structured Fill
  swatch/gradient editor and all §7 colour popover services. Do not flatten a
  gradient into a colour to satisfy ColourWell's string-only API.
- **G6 — accessibility projection:** the existing kit's number AX value remains
  numeric when `mixed` is true; it lacks the spec's mixed-number value, split
  colour description, colour hex/alpha wording, expression textarea status and
  separate link tooltip/AX semantics. Wire snapshots expose current behavior;
  they do not certify those missing semantics.

These gaps request changes from the surface, shell and app owners. This lane
changes neither `crates/surface` nor `js/v4/src`. Kit imports use the documented
source entry until the owning lane switches the public UI export.

## DESIGN-TODO(opus)

- **D1:** inspector §2 says multiple-layer alignment is `To selection`, while
  the frozen `AlignmentStrip.svelte` and current kernel align the union box to
  the composition, retaining relative offsets. The shared README gives the tag
  precedence on behavior. Resolve this conflict before wiring the native align
  service/tooltip. The panel emits only the semantic alignment intent.
- **D2:** no copy is specified for actor, subscription, binding or native-menu
  failures. Supply inline product copy/presentation for these failures. The lane
  uses the existing host diagnostic callback and adds no user-facing message.

The specified multiline/fill visuals are API gaps I1/I2, not new design questions.
No visual values or new interaction choices were made to fill them.

## Rule-selected decisions

| Decision                                                                                                   | Selecting rule                                   |
| ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| Kit components, pm-design references and neutral omissions where the kit cannot express the decided design | Design ownership, N1                             |
| Model owns applicability, mixed deltas, ratios, exact values, typed command planning and caller provenance | N3; kernel single-writer architecture            |
| Acknowledge binding registration before editing; capture gestures in Rust; retain actual host rejection    | N3/N8                                            |
| No old panel, extension consumer or user data is removed                                                   | N4/N6; prune rule                                |
| Stable ids, compact selection summary, async subscriptions, inspectable six-op tree                        | N7/N8                                            |
| Do not split atomic large-selection edits or manufacture success                                           | N3; W075                                         |
| Explicitly distinguish native-delegation snapshots from rendering/VoiceOver proof                          | N1/N2; Linux cannot establish native superiority |

## Reproduction and evidence

Run from the repository, with its normal toolchain activation. No test depends
on `/workspace`, `/proc`, temp-path identity, GUI, external spikes or media tools.
The conformance probe is owned by the inspector model crate. Its only extra
dependencies are development dependencies, already pinned in the workspace lock.
No registry/git package was upgraded. The workspace already includes `crates/*`.

```sh
cd apps/native
cargo test --locked -p pm-panel-inspector -p pm-surface -p pm-design
cargo clippy --locked -p pm-panel-inspector --all-targets --no-deps -- -D warnings
cargo fmt -p pm-panel-inspector --check
cd js/v4
bun install --frozen-lockfile
bun test
bun run typecheck
cd panels/inspector
bun run typecheck
bun run format
bun run build
```

Panel-only tests: `cd apps/native/js/v4/panels/inspector && bun test`.
The preload builds `inspector_wire` with `--locked`; the test helper imports that
setup too, so the parent v4 test run works from a clean checkout. Paths are
resolved relative to owned test files. The probe replays JSX batches through the
real Rust validator and AccessKit projection, and invokes real Controls/Bindings.
Model adapter tests submit requests to a real KernelActor. Failure-path tests use
an explicitly synthetic HostTransport; they do not claim an application backend.
The browser bundle also mounts with `process`, `Buffer`, `window` and `document`
absent. Snapshots normalize only Preact-generated label prefixes.

Linux results: **110 inspector model tests** (100 existing + 10 adapter),
**95 surface tests**, **10 design tests**, and **43 inspector panel tests with
21 snapshots**. **215 total parent v4 tests** (including the 43 panel tests) pass.

## macOS review gate

Verify the actual AppKit/GPUI renderer and model/host adapter: optical alignment,
header symbols, diamond/link/colour visuals, actual hit geometry at 1×/2×,
Infinite scrubbing/pointer restoration/haptics, focus and shortcut ownership,
IME/dictation/clipboard, native menus and font discovery, popover cancellation and
outside-commit as one undo, §7 gradient/eyedropper services, scroll memory,
contributed remote-DOM sections and per-project disclosure persistence. Exercise
VoiceOver through the real window subtree and all accessibility appearances.
Measure real 60k-selection responsiveness, playback cadence, exact pause and no
UI-thread JS waits. Capture the review checklist's screenshots and obtain Opus
review. None of these gates is self-certified by this Linux lane.

## Merge verification (2026-11-01)

Merged `origin/feat/native-engine` at
`4204a153dbf8af916ef2b92d7ca01858e363516a` and
`origin/feat/na-v4-integrate` at
`e5e209928817cb9434823ebf6ddf7d64f2213441`. The Cargo.lock conflict retained
both model registration and v4 pins. Both base commits are ancestors of this
branch. Re-ran the locked 215-test Rust gate and 215-test parent v4 suite after
the merges: no failures or skipped tests. Inspector and parent typechecks,
inspector formatting/browser build, strict inspector Clippy and rustfmt pass.
The test probe's target path uses file-URL decoding, including spaces and URL
delimiters; the runner retains no assumptions about macOS system temp links.
