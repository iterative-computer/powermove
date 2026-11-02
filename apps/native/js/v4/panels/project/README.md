# Project v4 panel — 2026-11-01

Panel `assets`, title **Project**, left dock, tabbed with **Effect Controls**.
Built against `docs/native/design/panels/media-browser.md`, the shared panel
rules/owner decisions, `docs/native/ui-kit.md` and the actual `pm-panel-media`
serde contract. The layer-list width (266 pt), Agent popover/pin, **Effects** name
and removal of web export remain shell/other-panel responsibilities; this panel
introduces no override for them.

**Status: headless implementation, not production/visual acceptance.** The entry
exports `ProjectPanel`, `ProjectView`, `ProjectService` and `projectPanel` identity.
The app does not yet register/mount this entry. All visual output is provisional
until Opus review. The neutral primitive slots below do not implement missing
native painting merely by declaring props.

## Architecture and integration

- `contract.ts` follows the model's externally tagged serde IDs, enums, pages
  and inputs. `presentation.ts` maps spec copy and authored tagged duration/size
  formatting, menu groups, shortcuts and disabled eligibility.
- `ProjectService` uses the public v4 `HostClient` project subscription and an
  **injected, actually registered** native command ID. It copies inputs, queues
  commands, captures the latest acknowledged owner at dequeue, checks bounded
  replies, rejects unsafe integers/unknown statuses, and fences old document or
  disposed responses. It owns no writable document or alternate selection model.
  Its errors remain inspectable; rejected commands retain the last accepted view.
- `native/bridge.rs` converts `MediaPanel` pages and inputs into a bounded view.
  `Action::{Page,Input,Menu}` all require an exact project/document/revision stamp.
  No requested page means zero mounted cards/runtime poster lookups. Root drop
  targets serialize distinctly from no target. Navigation/filter inputs reset
  the window to the start.
- **A bridge reply is a plan, not a kernel edit acknowledgement.** The application
  worker must refresh `Catalog::published`, call `ProjectBridge::request`, prepare
  any returned edit, enqueue it through the kernel, await the actual ack on the
  worker, call `accept_edit`, and return the updated projection. It must enqueue
  menu/outcome effects with their captured owner, including effects produced by
  refresh, and guard native completions against project/document switches. Only
  then may its registered v4 command acknowledge. `native/test_host.rs` proves
  real actor edits and Undo/Redo for synthetic projects; it is a test process,
  uses blocking worker waits, and must never be used as the application adapter.
- The JS view composes kit controls and structural six-op primitives. Native
  menus receive exact labels, group boundaries, shortcuts and enabled state.
  Search and rename require real host-supplied local text-binding handles.
  Native grid layout supplies columns/row height and row windows; the view never
  measures all text or picks a production thumbnail size. Poster keys require a
  native resolver to generation-scoped resource leases, supplied only for the
  mounted window. The optional enqueue-only marquee seam supplies full-index
  hits and capture/autoscroll inputs, never just mounted-card hits.
- `native/accessibility.rs` enriches the kit's actual AccessKit tree with cell
  roles, full row count, multiselectability, active descendant, search/toolbar
  roles and applicable VoiceOver actions. Native custom actions validate the
  surface revision, document stamp, target and stable action ID, then return the
  same model input for the worker queue. The app must namespace/graft IDs and
  check the realm generation before routing them. Generic kit click/focus actions
  continue through the kit driver; this adapter handles custom media actions.

The catalog worker must supply tagged asset enumeration with `with_asset_order`
and the macOS locale comparator through `with_collation`. Canonical object-key
order is not evidence of authored import order. The migration/import ordering
producer and native collation remain app integration gates (N3); do not infer
chronology from IDs.

Mount on the existing JS worker after registering the native command and local
bindings; supply approved native layout and viewport before starting the service:

```tsx
const service = new ProjectService(host, {
  command: registeredMediaCommandId,
  subscription: uniqueProjectSubscriptionId,
  projectStamp: decodeActualProjectPublicationStamp,
});
await service.page(viewportItemOffset, viewportItemCount);
await service.start();
await root.render(
  <ProjectPanel
    service={service}
    native={nativeServices}
    layout={nativeGridLayout}
  />,
);
// Unmount root, then dispose service when its panel/document lifetime retires.
```

The application must implement that registration/publication decoder and the
native services. None is fabricated here, and no new public MCP command name or
v1–3 API is introduced.

## Spec section → implementation and gates

| Spec              | Done on Linux                                                                                                                                                                                                                                                                                        | Not done / integration gate                                                                                                                                                 |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §1 model          | Actual model IDs/pages, folder crumbs, selection count, rename, drag/drop and stamped inputs; project host subscription                                                                                                                                                                              | Changed status, cloud fraction, comp background/thumbnail absent from current model; search/all-except gaps below                                                           |
| §2 anatomy        | Kit header tabs/actions, conditional search/path, bounded virtual grid rows, content/meta/status text, hover/focus actions and selection bar                                                                                                                                                         | Exact grid/thumbnail/card painting, badge fill, glyphs, letterboxing/waveforms, middle truncation, rings and fade require native kit/token support                          |
| §3 states         | 24 accepted op-tree + actual AX snapshots: no project, empty project/folder, restoring before/after delay, loading/importing cards, selected, focus, multi, disabled comp, offline, unavailable, unknown, cloud stored/downloading/error, drag, external/folder drop, rename, 60k, hover, no results | Changed is unsupported by the model; stacked drag image/opacity, marquee drawing, invalid cursor, native 0.5-s restore observation and physical 60k scrolling not certified |
| §4 interactions   | Pointer/keyboard/model dispatch, exact model menus, preview/locate/download/import requests, atomic folder edits, guarded delete confirmation, move and kernel Undo/Redo tested                                                                                                                      | AppKit menus/alerts/pickers, Finder, viewer, import/archive/task workers and actual native completion handling                                                              |
| §5 keyboard/focus | Header → search → crumbs → one grid stop → Delete order asserted; hover actions not Tab stops; Escape, F2/IME guard, model wrap/Return/Space wired                                                                                                                                                   | Grid-row Up/Down differs from tagged model; native shortcut modifier admission, focus requests, rename select-all and capture/autoscroll require host                       |
| §6 VoiceOver      | Real AccessKit cells, selected states, full count, labels/units, applicable custom actions and stale-action guards                                                                                                                                                                                   | App window grafting, native announcements, traversal/scroll to unmounted rows and real VoiceOver walkthrough                                                                |
| §7 appearance     | Known pm-design token references; no literal visual values; no invented kit component                                                                                                                                                                                                                | Missing tokens/paint recipes, native light/dark/accessibility captures and Opus approval                                                                                    |
| §8 copy           | Spec strings/menu grouping and tagged formatting; missing states never acquire invented copy                                                                                                                                                                                                         | Transport/unknown-state design questions below; announcements need actual completion callbacks                                                                              |
| §9 constraints    | No import progress UI, list/sort controls, glass, filesystem deletion, browser/Node runtime dependency or replacement/removal of old consumers                                                                                                                                                       | Production integration remains necessary; these headless checks do not certify the application                                                                              |

## API requests (additive only; outside lane ownership)

1. **pm-design:** shared rules §6 request `metrics.thumbnail_min` and
   `colors.media_badge`; neither exists in the merged token struct. The spec
   explicitly blocks substitution. This view has neutral thumbnail/badge slots;
   it does not choose literal fallback values. Native grid geometry must use the
   approved thumbnail token once available. `metrics.kind_badge` is also pending
   for kind marks.
2. **Native kit/rendering:** typed SF Symbol, grid/card/image/waveform recipes,
   token math (including 16:9 sizing), middle truncation, comp background,
   selected/focus rings and their contrast variants, media badges, over-image
   buttons, drag image/opacity and reduced-motion hover fade are not exposed by
   the current kit. Generic primitives/labels do not implement these renderers.
   Search currently emits the kit's TextField; NSSearchField/system-small size,
   placeholder and trailing search progress need a native search recipe.
3. **Model search:** `Filter` currently searches asset name/id/path within the
   current folder. The spec requests name search through descendants and path
   metadata in results. This requires a model/index API change; JS does not
   maintain a competing search index.
4. **Model selection/navigation:** select-all is a vector/set operation, not
   O(1) all-except; Up/Down step one asset as the tag does, without a grid column
   argument. Exact native grid-row navigation and compact select-all require
   model work, not a second JS selection implementation.
5. **Model/runtime facts:** Changed/source fingerprint, determinate cloud
   progress, comp backgrounds/thumbs, measured waveform leases and descendant
   search paths need publication fields. Unknown is never relabelled offline or
   fabricated as a usable original. No Changed snapshot is claimed.
6. **App host:** register/mount this v4 entry, decode actual project publications,
   register the command/local bindings, apply edits/effects as described above,
   and wire scoped poster resources and native services. No app/surface/API-kit
   files are edited in this lane.
7. **Native interaction/accessibility:** pointer enter/leave for reliable hover
   exit, modifier-aware default key admission, rename select-all, geometry/
   capture/autoscroll and id-anchored scroll retention need native integration.
   Viewport range events are bounded and in grid rows; the host must request a
   row-aligned item window with one viewport of overscan above/below. Full
   VoiceOver row navigation must scroll/page outside the mounted window.
8. **Native completions:** NSAlert must use the spec's warning text and Cancel as
   default; only real Finder/preview/import/add/delete completions may announce
   success. Background Tasks owns import progress/retry. The model's tagged undo
   labels retain sentence case; the spec's Title Case undo names require the
   model/transaction owner to reconcile them.

## Every DESIGN-TODO(opus)

- **Unknown original readiness:** media-browser §3 does not specify
  `Availability::Unknown`. Keep a neutral poster well; do not infer offline,
  loading or ready. Decide its eventual presentation.
- **Worker/transport failure:** the spec covers media/import failures but gives
  no treatment/copy for failure of the panel's host query/command transport.
  Diagnostics stay inspectable while the last accepted view remains visible.

These are design questions, distinct from the API gaps above. Already specified
visuals are implementation gates, not requests for a new design decision.

## Rule-selected decisions

| Decision                                                                            | Rule                                                               |
| ----------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| Rust model owns selection, ranges, menus and edits; no parallel JS document/index   | N3 correctness; pm-kernel single writer; N7 inspectable id input   |
| Actual kernel ack before accepted edit projection; stale owner/epoch results refuse | N3/N4; no phantom edits or cross-project completion                |
| Keep archived source bytes, opaque fields, old consumers and v1–3 paths untouched   | N4/N6; prune rule                                                  |
| Bounded model pages, row virtualization and native geometry/resources               | N1/N2 pro-scale; N8 worker work only                               |
| Use tagged formatter intent, not Chromium media-readiness/DOM behavior              | N3 and quirk policy                                                |
| Neutral slots for missing token/renderer APIs, explicit design questions            | Design ownership rule; no substituted visual values/copy           |
| Stable AX action IDs survive runtime-only availability changes                      | N3/N7: late actions cannot be reinterpreted as a different command |
| HostClient async service + native guarded AX input                                  | N7/N8; no main-thread JS wait or Node/Chromium in the panel        |

## Verification

Run `./check.sh` from this directory with Rust 1.96.1, Bun and the repository
build tools on PATH. The panel-owned Rust adapter/verification package is an explicit native-workspace
member and uses the shared Cargo.lock. It compiles the same model/kernel and
surface sources on Linux/macOS.
No dependency checkout, private file, Linux-only path, ffmpeg build or external
spike is required. Test temporary directories canonicalize the system temp path.
`bun test` also auto-builds the native test binaries with `--locked`.

Native `test_host` and `inspect_wire`, Bun/Node APIs in `tests/`, are verification tools, not application runtime dependencies.

Linux result (2026-11-01): **13 adapter/AX Rust tests**, **62 media-model tests**,
**95 surface regression tests**, **225 Bun tests** (53 new Project tests),
**24 accepted op-tree/AX snapshots**, zero failures/skips. Both TypeScript
checks, touched-package Clippy with `-D warnings`, rustfmt and panel Prettier
checks pass. `check.sh` records the exact executable commands. These counts do
not certify app mounting, native painting or macOS behavior.
On macOS verify all rendering/appearance gates, actual padded hit targets and
1×/2× poster sizes, native field/IME/clipboard behavior and focus order, menus/
alerts/capture/drags, VoiceOver actions/announcements/virtual traversal, real
media/import/archive/relink/undo provenance, and no main-thread JS waits.

## Frozen-contract adoption — 2026-11-01

Merged `feat/native-engine` at `f7844ebc007c17900cb2d65f4ef63c6bd3f8b005`.
The merge adds the recorded freeze decision; `docs/native/v4-api.md`,
`docs/native/ui-kit.md` and the API/kit implementation are unchanged from the
integration revision already consumed by this panel. No panel API adaptation,
shared API edit or dependency/lockfile update is required by this merge.

The recorded integrated eval is 40/40. That result freezes the contract and does
not certify this panel's app mounting or macOS UI acceptance. Existing spec
coverage, both design questions and all remaining host/model/rendering gates
above remain explicit. The requests above are additive API requests, not a
reason to change the frozen surface contract (N3/N7/N8; design ownership).
