# Viewer built-in v4 panel — 2026-11-01

Status: **partial implementation; not application cutover or visual acceptance**.
The supported chrome uses the existing UI-kit entry directly, as ui-kit.md permits
until the compiler/export owner reconciles `powermove/ui`. No new component,
visual token, display string, shortcut registry, document store or compatibility
provider is introduced. All visual output remains provisional until Opus review.
The shared owner decisions (266 pt layer column, agent popover/pin, Effects,
no web-animation export) are unchanged; none adds a viewer control.

## Spec section → implementation

Source: `docs/native/design/panels/viewer-chrome.md`, shared `panels/README.md`,
`docs/native/ui-kit.md`, and `docs/native/panels/viewerchrome.md`.

| Spec section                                    | Done in this lane                                                                                                                                                                                       | Not done / gate                                                                                                                                                                                      |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §1 Model                                        | Rust model projection, actual displayed fit zoom, quality/effective scale, empty/selection/controls/lost facts, document/selection/viewport ticket guards; validated host observations                  | Application `KernelData` route registration and renderer/source observation producer                                                                                                                 |
| §2 Viewer bar                                   | No title; docked bar; source leading copy and Back action; zoom/resolution ordering, preset/action choices, changing titles, spec tooltips and AX labels; UI-kit components and token references        | Native borderless variants, fixed measured title widths, menu separators/checks/subtitle/key equivalents/type-ahead, chevron symbol slots and trailing inset math                                    |
| §2 Surround/frame/gizmos/toolbar/overlay/banner | Neutral canvas group retains observed frame resource, loading/failure, presentation and source facts for inspection; hides docked bar in Focus/FullScreen                                               | Actual native image slot, clipping/frame/checkerboard/pasteboard, all gizmo drawing, contextual FloatingBar, glass capsules, timing, recovery button and banners/Retry                               |
| §3 States                                       | 23 real wire-validator snapshots, including every listed state and 60k selection count; first-frame/reload/error observations retain the last-good resource in the contract                             | These snapshots deliberately cover placeholders for canvas states. They do not prove painting, retained GPU pixels, loading spinner, drop placement, source waveform/transport or selection geometry |
| §4 Interactions                                 | Every zoom/resolution menu choice dispatches once to the existing Rust model; source Back and Open callbacks; real kernel ack path tested; native pop-up adapter normalizes Fit/Actual/action sentinels | Native NSMenu and gesture/drag/drop/tool/text routing; no new JS keymap or guessed gesture classifier                                                                                                |
| §5 Keyboard/focus                               | Accepted-tree Tab order: Back (source), zoom, resolution, canvas; no hidden pop-up stops with no project or in Focus/FullScreen                                                                         | Application arbitration of all w1-keymap bindings; Escape drag/text/source/deselect order; system focus rings, Focus overlay focus restoration                                                       |
| §6 VoiceOver                                    | Real AccessKit group/button/ComboBox roles, canvas value, labels and focus order asserted in light/dark                                                                                                 | Layer custom rotor, AXLayoutItem/custom nudge actions, source slider/time semantics, complete real-window AX graft                                                                                   |
| §7 Appearance                                   | Existing pm-design references only, no literal visual measurements/colours/type/radii/motion; validator checks both schemes                                                                             | Token and recipe gaps below; native accessibility setting changes and Opus review                                                                                                                    |
| §8 Copy                                         | Implemented empty, source-leading, zoom and resolution copy is exact; no new visible strings; continuous adaptive scale is never mislabelled Half/Quarter                                               | Auto subtitle, banners, recovery, locked-layer tooltip and source transport await their APIs                                                                                                         |
| §9 Don't                                        | No viewer title, new guide/grid/safe-area toggles, custom AppKit control copies or animated menu zoom; no old consumers removed                                                                         | Visual compliance of native renderer requires macOS proof                                                                                                                                            |

The viewer has no row list. The 60k JS case proves bounded chrome tree size for
an observed selection count; the existing Rust model suite separately submits a
real 60,000-ID selection. Neither test establishes interactive 60k performance;
the model documentation still records the kernel's quadratic selection guard.

## Worker and model connection

`mountViewer` creates a v4 surface and `ViewerService`, subscribes to project
channel `viewer.chrome`, and renders validated `{model, host}` observations.
`model` is produced by `pm_panel_viewerchrome::v4::Session::publish` on a native
worker using one coherent model and approved viewport. `host` contains observed
presentation, renderer resource/status, source epoch/time/duration and interaction
facts, as declared in `contract.ts`. Resource strings are generation-scoped host
leases, never bytes, file paths or a DOM canvas. `metadata` is inspection data:
the current native renderer explicitly **does not interpret it as recipes**.

Commands use the existing v4 host data service's `command` operation:

```json
{
  "command": "viewer.chrome.drive",
  "args": { "ticket": "1", "element": "viewer.quality", "argument": "0.5" }
}
```

The owning application's `KernelData` must register that channel and command.
Create one Rust `Session` per panel/realm generation. Publish `{model:null,host}`
when there is no project; opening a document creates a fresh host generation.
Commands must authenticate that same generation/project, decode the args to
`pm_model::Value`, read the current model, call `Session::drive`, then
`Controller::submit`. Preserve the pending actor ack and all Throw/folding/
durability notices. An enqueued media/transport request is not execution success.
The adapter introduces no JavaScript dependency in the production model crate.

Native UI-kit pop-up bindings are `viewer.zoom` and `viewer.quality`. The owning
`BindingSink` routes their selected values through `Session::popup` with the
published ticket. It handles Fit, Actual Size and menu action sentinels before
calling the existing model. Native writes must not also echo a JS `change` event
that submits the same edit. Worker/headless delivery uses the tested `onChange`
callbacks instead. Registration/driver wiring is an application API gap, not
implemented by assigning a binding string.

The ticket retains full-width document/state revisions, selection identity and
viewport in Rust. It prevents stale commands after a publication, view edit,
selection-only change, resize or reopened document. The existing model's
consumption-time lifecycle/selection race remains documented in
`docs/native/panels/viewerchrome.md`; the app must serialize those operations until
the kernel owner adds atomic actor admission. This adapter does not claim to fix
that unowned actor operation.

The service never optimistically changes its observed state after an intent ack,
automatically retries a rejected write, or replaces good state with malformed or
foreign observations. It copies observations at both boundaries. Stop during
subscription waits for native unsubscription and suppresses late delivery;
repeated stop/dispose is idempotent. Async command/surface failures reach the
provided diagnostic callback with no lane-authored user copy.

## API gaps outside ownership

1. Register the nonclosable `viewer` built-in, its `KernelData` query/command routes,
   source/renderer publications and native binding handles in the app. Supply the
   Open action. Reconcile the UI-kit package export/compiler alias. No application
   registry API exists in the current v4 package.
2. UI kit needs the specified viewer-bar borderless PopUpButton/text-button variants,
   native measured fixed widths, symbol slots and token arithmetic. A flat options
   array cannot express NSMenu separators, action key equivalents, checkmarks and
   the Auto subtitle. Custom zoom is retained as its real typed value, with no
   invented menu row. Default kit recipes may still supply well visuals despite
   the neutral background override; native rendering is not certified.
3. Add viewer/frame/gizmo recipes, approved geometry/token references and a real
   generation-scoped engine image lease slot. Drawing cannot be implemented by
   pretending `metadata` is a native canvas API. Preserve the last good GPU frame
   across reload/zoom/failure in pm-engine-service/shell.
4. Add dark-scheme-over-image ViewerOverlay/FloatingBar/banner/transport recipes,
   source slider/Play semantics, symbol support, Reduce Transparency fallback and
   token-based reveal/dismiss timers. The docked bar hides in Focus/FullScreen;
   replacement overlay controls are still placeholders.
5. Add native focus ring and renderer/system setting publications, custom layer
   rotor/LayoutItem/action semantics, source time slider AX values and native
   window tree grafting. Existing AccessKit only supports the basic roles here.
6. Bind native classified wheel/pinch/pan, pointer capture, tool gestures, hit tests,
   effects/media drop coordinates and text/IME services to the owning models.
   Reuse model input/command candidates after shell extension-priority arbitration;
   don't reproduce Chromium wheel heuristics or duplicate a keymap in this panel.
7. Retire no v1–3 provider or old viewer consumer until the synchronous closure,
   source/render/tool, accessibility and Opus gates pass (C-MATRIX, N4/N6/prune).

## DESIGN-TODO(opus)

- **D1 — Token names:** provide pm-design names for the spec's viewer-only
  geometry/keylines/opacity/checkerboard, fixed-width/inset expressions and overlay
  reveal/dismiss durations. Correct light `colors.viewer` to the spec's neutral
  value in the design-owned crate. Existing tokens currently differ; this lane
  deliberately supplies no literal override. Recipe APIs are the implementation
  owner's work, not a request for a new visual design.
- **D2 — Adaptive quality:** the model permits continuous effective scale
  (e.g. 0.75), while the spec names only Full/Half/Quarter. Specify the title/AX
  wording for intermediate observations. Until then the title stays `Auto`, and
  no incorrect discrete effective-quality claim is spoken.
- **D3 — Unnamed composition:** specify the canvas spoken-value fallback when an
  older valid composition has no name. Preserve its empty name; do not invent
  a title or reject the project. Explicit authored UTF-16 names survive unchanged.
- **D4 — Playing source accessibility:** the source table gives `Play` but no
  playing-state button label. Confirm that state before transport copy is added.
- **D5 — Presentation focus transitions:** specify where focus goes when a focused
  docked pop-up disappears on entering Focus/FullScreen and on return. The spec
  defines ordering and fade timing, but no focus destination; none is invented.

## Rule-selected decisions

| Decision                                                                                                                             | Selecting rule                                                         |
| ------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------- |
| Use the existing model and real actor acknowledgements; no optimistic document or view mirror                                        | N3 authored intent; kernel single writer; N7 inspectable outcomes      |
| Keep revision/selection/viewport tickets in Rust, send bounded chrome facts                                                          | N1/N7 scalability and honest stale rejection; preserve 64-bit identity |
| Copy and validate publications; preserve good state and authored UTF-16 names                                                        | N3 exact text; N4 no data loss                                         |
| Worker transport only; test-only Rust wire binaries/Node VM; browser-target panel has no Node/DOM globals                            | N8 no UI-thread JS waits and no Node/Chromium in app                   |
| Use kit components and existing tokens; neutral placeholders for missing recipes and explicit questions for missing design decisions | Opus ownership; N1 correctness; N3 no guessed look or copy             |
| Native classified gesture intent and shell arbitration stay with their owners                                                        | N3 quirk policy; N6 overrides and claim/decline contract               |
| Remove no implementation, extension, profile data or compatibility route                                                             | N4/N6 and prune-only-after-last-consumer                               |

## Validation

From `apps/native` using the repository's Rust toolchain and installed Bun:

```sh
cargo test --locked -p pm-panel-viewerchrome
cargo clippy --locked -p pm-panel-viewerchrome --all-targets --no-deps -- -D warnings
cargo fmt -p pm-panel-viewerchrome -- --check
cd js/v4
bun install --frozen-lockfile
bun test
bun x tsc --noEmit -p panels/viewer/tsconfig.json
bun x prettier --check panels/viewer
bun run typecheck
bun run build
```

Panel tests automatically build the **owned** `pm-panel-viewerchrome` wire
examples with `cargo build --locked`, replay actual six-op batches through
pm-surface, validate tokens and inspect the real AccessKit projection. No
spike/test binary environment variable, ffmpeg build, user profile, screen,
AppKit or Metal is required. Helpers resolve paths from their own module URLs.
The data roundtrip invokes a synthetic real kernel actor, waits on its actual
state acknowledgement, publishes its projection and asserts stale replay fails.
`v4_wire` explicitly refuses host-only commands rather than treating InspectHost
acceptance as execution proof.

Current counts before final integration: 32 viewer Bun tests, 23 tree snapshots,
8 new Rust adapter tests plus the existing model suite; the whole v4 suite has
203 tests. Final merged-base counts are recorded in the final task report.

macOS must verify native placement, fixed widths and menu behavior, real source
seek/playback/pointer capture, classified gestures/shortcuts/IME, retained GPU
frames/EDR and geometry at 1×/2×, real VoiceOver traversal/rotor/actions, all
accessibility appearances, lifecycle/selection fencing and no main-thread JS
waits. Native screenshots, keyboard/AX evidence and Opus approval remain required.
