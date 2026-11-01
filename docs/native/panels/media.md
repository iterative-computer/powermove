# Media browser model — Wave 4 head start

Package: `pm-panel-media`, `apps/native/crates/panel-media`. Model only; no
GPUI, AppKit, WebKit, V8, Node, pixels, styling or product copy. Reference:
`v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`).

Read with [kernel](../kernel.md), [W1 workflows](../w1-workflows.md) W018–021,
[keymap](../w1-keymap.md), [C-MATRIX](../c-matrix.md),
[D-STATE](../d-state.md), [MCP](../mcp.md), [traces](../traces.md),
[engine service](../engine-service.md), [first launch](../first-launch.md),
[V8](../s1-v8.md), [compat](../s5-compat.md), [pro media](../pro-media.md),
and the existing [design language](../design/language.md).

## Authority and integration

`Catalog::published` builds an immutable index from a kernel `Published`
snapshot. **Build catalogs, filter indexes and edit plans on a worker.** Keep
the previous publication visible until its replacement is ready. The index
retains canonical `pm_model::Value`, including unknown extension fields and lone
UTF-16 surrogates; no serde JSON round-trip rewrites project data. The public
view data and inputs use serde and stable `ItemId::{Asset,Folder,Composition}`.

The canonical kernel tree sorts object keys; it does not retain JavaScript
insertion order from A:67. `with_asset_order` accepts a complete validated host
enumeration, and that same order drives pages, ranges, keyboard focus and drag
batches. Without a hint the model retains canonical key order; it never infers
import chronology from ids. Exact tagged asset enumeration requires migration/
import host ordering evidence, outside this lane. Composition names and bin names
use the tagged numeric/base sort; `with_collation` lets macOS supply its locale
comparator without adding any sort control.

`MediaPanel` owns only panel-local selection, anchor, focus, bin, search/filter,
rename, drag, marquee and source-monitor state. It never owns a `DocumentStore`.
Call `refresh` after a changed publication/filter/runtime registry. A document
lifetime change releases selection, preview, drags, pending confirmation and
rename. A missing open bin falls back to the root. Composition changes within
the same document keep project-wide bins. Searches are case-insensitive literal
matches on asset name/id/source path, scoped to the current bin; kind, missing
and proxy filters are semantic additions, not a claimed tagged search UI.

`page(offset, limit)` returns a bounded window, total, next offset and revision
stamp. No layer scan, whole-document serialization or all-row runtime lookup
occurs in paging. Runtime is queried only for the returned assets. The maximum
page size (4096) is a resource bound, not a visual row count. The 60,001-asset
fixture exercises navigation outside mounted rows. Selection/ranges and marquee
use the complete filtered asset index rather than mounted view nodes. Folder
subtree counts use iterative bottom-up accumulation; malformed cycles and chains
feeding them surface at the root without rewriting the saved data.

`dispatch(catalog, runtime, Input)` returns `Outcome { owner, handled, effects, edit }`.
`menu` returns semantic commands, enabled state **and selection/preview effects**.
The host must apply those effects too. An `Outcome` can enqueue its effects as
`HostRequest { owner: Stamp, effect }` through `MediaHost`. The stamp contains
project id, document lifetime and revision. The host checks ownership before
starting work and before admitting completions; it never retargets an old picker,
drop, download or preview callback to the currently focused project. A host
adapter must enqueue promptly and never wait on JS, IO or a kernel reply on the
main thread. Linux tests include a recording host double and a runtime double.

`Catalog::prepare(Edit)` produces a reviewable `PreparedEdit`. `submit` returns a
kernel `Pending`; poll or await it. The kernel owns validation, publication,
journals and one-step undo/redo. `accept_edit` runs only after a successful ack:
new folders become selected with a rename draft; duplicated compositions become
selected. No phantom folder is selected before the kernel accepts it.

**Current kernel constraint:** `Call::Transact` has an expected revision but no
atomic expected-document guard. These plans therefore use the existing
`Call::Restore { expected_revision, expected_document }`. This is a guarded
undoable kernel transaction; it refuses while any entry is open and does not
fold into another gesture. A busy/stale failure preserves all other work; keep
the plan for inspection and explicitly replan after refreshing. It must not be
silently retried against a new document. A kernel-owned document-guarded Transact
variant would allow ordinary nesting; that change is outside this lane's paths.

## Import, relink and runtime boundaries

`MediaRuntime::state` is a nonblocking cache lookup. Only observed readiness can
produce `Ready`; persisted metadata and a ready proxy do not prove the original
is usable. Unobserved state is `Unknown`, not offline. Loading, offline, failure,
cloud provider/download state, proxy job state/version and an opaque cached
poster key are separate semantic facts. Posters may survive offline originals.

Pick/drop effects request media-only import (no placement or layer). An Option
drop carries `as_folder: true`; folder scanning, image-sequence detection, rate
choice, gap warning, cancel/progress and decoding remain the import service's
job. `StoredAsset` is a **trusted host result**, not a disk verifier: the host
must durably archive original bytes and retain every replaced byte version before
constructing it. An empty original version is rejected. `prepare_import` checks
the captured owner, complete batch, ids and target bin before publishing anything.
Replacements keep asset id, bin, authored layer timing/trim/transform and opaque
metadata; supplied metadata cannot move a relinked asset into another bin. Undo
restores old metadata and the original-version pointer. The byte host must keep
both versions resolvable after undo/redo and history retention changes. This
crate never unlinks originals, proxies, posters or archive objects.

`ShowPreview`, `TogglePreview`, `SeekSource` and `ClearPreview` go to the viewer
service. Seeking is source seconds, independent of composition fps. Playback
state is acknowledged by stamped `PreviewFeedback`; a requested toggle is not
reported as confirmed playback. Stills have no transport; models do not preview.
`poster_time` preserves the tagged sampling time without Chromium readiness
workarounds. `waveform_bins` returns measured normalized means, or `None` if
analysis is absent; display dimensions and styling are not model data.

Agents can serialize `Input` and drive the same dispatch by ids, inspect bounded
`Page` results and prepare the same undoable edits. No new public MCP tool name,
schema or v1–3 extension contract is changed here. Binding this model into the
app's agent/MCP routing is host integration, outside the owned paths.

## Tagged behaviour inventory

All line references below are to **v1.2.0**. `A` means
`apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`; `F` means
`apps/desktop/src/renderer/src/legacy/core/asset-folders.ts`; `M` means
`apps/desktop/src/renderer/src/legacy/core/media.ts`; `S` means
`apps/desktop/src/extensions/viewer/source-preview.ts`.
Composition dependency/count/order details also use
`apps/desktop/src/renderer/src/legacy/core/compositions.ts:187–231`.
**Done** means the model/command behavior has headless tests. **Host pending**
means the semantic request is implemented and tested; native work/visuals are
not claimed complete.

| Behaviour | Tagged source | Native disposition |
|---|---|---|
| Root compositions, bins, metadata; project-wide bins | A:42–80, 116–139, 802–809, 916–1019 | Done: indexed semantic rows; dimensions/duration/format/bytes/geometry counts are raw facts |
| Asset enumeration drives ranges, focus and batch drag order | A:67–71, 267–286, 595–609 | Done when host supplies with_asset_order; original insertion-order evidence is not present in canonical snapshots |
| Bin path, recursive counts, numeric case-insensitive folder order | F:23–81; A:891–909 | Done for deterministic numeric/base ordering; exact locale collation needs macOS verification |
| Missing bin → root; corrupted cycle visibility | F:23–49; A:48–63 | Done, iterative cycle-safe navigation; damaged source preserved |
| Plain select, Cmd/Ctrl toggle, Shift range, Cmd/Ctrl+Shift union | A:267–286 | Done, id-indexed anchor and selection; no layer selection side effect |
| Preserve multi-selection on press for drag; collapse on click release | A:421–435 | Done, separate Down and Click inputs |
| Secondary press / nested buttons do not select | A:423–435, 613–619, 932–934, 966–968 | Done, button/target routing flags |
| Cmd/Ctrl+A only visible-bin assets, not compositions/bins | A:548–558 | Done over all filtered assets, independent of mounted window |
| Arrow keys wrap; Home/End; modifiers extend/toggle | A:595–609 | Done with focus-by-id effect |
| Enter adds; Space selects and requests source toggle | A:289–301, 567–572, 626–633 | Done request; host pending for actual placement/playback |
| Delete/Backspace; list Escape clears; nested controls keep Enter/Space | A:548–564, 612–638 | Done; text/nested controls keep their own commands |
| Double-click selects and previews; models do not preview | A:567–587 | Done request; host pending for decoded picture/audio |
| Offline primary/double-click locates or downloads cloud media | A:289–298, 575–584 | Done request; duplicate in-flight download suppressed (N1/N3) |
| Loading is not offline; cached poster survives missing bytes | A:150–173, 991–1059 | Done semantic runtime state; host pending for actual poster resolution |
| Selection release outside panel; source monitor retains ownership | A:438–474 | Done; folder/comp exclusive selection clears on a different item press |
| Project document swap / teardown releases source | A:86–114 | Done on lifetime change; host calls Clear on teardown |
| Marquee empty-space start; modified additive base; hit union; pointer capture ownership | A:477–545 | Done semantic state/all-row hits; host pending for geometry/capture |
| Marquee edge autoscroll; scrolling updates hit set; up/cancel/blur ends | A:499–546, 875 | Done owned-edge scroll request + refreshed hits; host pending for geometry/ticks |
| Media batch drag: primary payload plus whole selected batch; copy to timeline/move to bin | A:367–387, 732–737 | Done typed Drag; host pending for platform drag/drop |
| External file drag depth, copy drop, no timeline insertion | A:389–418, 658–663 | Done import-only effect, depth reset; host pending for NSDragging/picker/import worker |
| Option keeps dropped directory as individual-files folder | A:655–663 | Done as_folder intent; host pending for image-sequence/import service |
| New bin, unique default name, select + inline rename after acceptance | A:666–680; F:84–95 | Done, existing shipped default name and Undo label |
| Open bin / breadcrumb; folder Enter opens; Delete/Backspace ungroups; Escape clears | A:683–696, 782–789 | Done; no media deletion when removing a bin |
| Rename Enter/blur commits trimmed nonempty text; Escape cancels | A:698–701, 791–799, 942–944 | Done CommitRename/CancelRename; host pending for native field focus/select |
| Move media/bins; reject cycles; deduplicate sibling names | A:711–714; F:106–129 | Done atomic batch; invalid target/cycle refuses the whole plan |
| Delete bin moves direct media and subfolders to parent | A:704–708; F:132–144 | Done with real-kernel Undo/Redo |
| Folder/breadcrumb copy or move target; no self/descendant drop | A:739–779 | Done target eligibility, hover state, drop/cleanup |
| Project menu: New Comp, New Folder, Import Media, Import Folder | A:642–652 | Done all semantic commands; host pending for picker/comp sheet |
| Folder menu: Open, Rename, New Inside, Import Into, Move Out, Delete | A:716–729 | Done; import effect host pending |
| Asset menu selects clicked asset; batch only offers Delete | A:180–190 | Done, selection effects returned with menu |
| Asset menu Add/New Comp/Locate or Replace/Move Out/Reveal/Delete; cloud Download | A:191–206 | Done every command/eligibility; New Comp excludes audio/models; host pending for platform/engine actions |
| Reveal button selects asset; fallback sourcePath → path | A:176–177, 304–318, 1044–1048 | Done semantic path effect; host pending for Finder result/error presentation |
| Unreferenced delete immediate; referenced delete single confirmation; duplicate request suppression | A:322–364, 1098–1103 | Done one batch plan/Undo; stamped token; cancel is safe |
| Switching project while confirmation is open cannot delete new project | A:322–326, 350–364 | Done document lifetime + revision guards; stale same-project edits also refuse |
| Media deletion removes direct and extension sources, scene descendants/maps; unparents survivors | M:189–244 | Done complete known reference paths; exact undo restores graph/opaque metadata; no byte deletion |
| Comp select/open/nest; Enter opens; Delete/Backspace deletes; Escape clears | A:812–829, 864–870 | Done; open/nest are viewer/shell requests; duplicate/delete use kernel command planner |
| Comp context menu New/Open/Add/Settings/Duplicate/Delete eligibility | A:831–846 | Done semantic commands; self/cycle nesting and last-comp delete guarded |
| Comp drag uses asset-like comp payload, copy only; cannot file comp in bin | A:849–862 | Done typed Drag and drop eligibility |
| Source picture/play click toggles, close/Escape clears | S:249–255, 279–280 | Done via Source Space/Escape semantic bindings; host pending for actual monitor |
| Source pointer scrub; Left/Right ±5 seconds; Home/End | S:238–277 | Done finite/clamped SeekSource and keys; host computes pointer ratio/capture |
| Still source hides transport; source clock independent of project fps | S:45–49, 181–230 | Done semantic transport flag/source seconds |
| Thumbnail real-frame sample; audio peak means | A:214–264 | Done sample/analysis request data; host pending for native decode/thumbnail generation |
| Search/kind/missing/proxy filters; proxy status by version | No tagged search/proxy control in A:1–1115 | Done semantic addition for this lane; no shortcut or visual treatment invented |

The W1 A412 summary mentions search/filter, but the frozen panel contains no
search input or filter handler. This implementation records it as a requested
native addition, not shipped evidence. Likewise no thumbnail hover scrubbing is
invented: the shipped scrubbing interaction belongs to the source monitor.

## Decisions and rules

| Decision | Rule that selected it |
|---|---|
| Kernel publications are the only document authority; model state is local | Kernel architecture; N3 exact intent; N7 inspectable/drivable |
| Revision + document guard through Restore; refuse another open edit | N4 never apply stale async work to a reopened project; N3 correctness. Current nesting limitation explicitly retained above |
| Keep canonical pm-model data and unknown fields untouched | N4 no lost user/extension data; N6 v1–3 contract |
| Bin deletion ungroups; media deletion retains archived bytes/versions and Undo graph | Shipped authored behavior; N4 archive, never delete |
| Full known extension/3D media reference paths are supported here | M:195–244; N4/N6; do not use the narrower kernel RemoveAsset planner for this panel |
| Accept explicit host asset order; never guess import chronology from ids | A:67–71; N3 authored order, N7 deterministic id driving; source-order producer remains host integration |
| Missing analysis is unknown, never fabricated audio; omit DOM/video ready-state quirks | N3 correctness and quirk policy; N2 strictly better source truth |
| Ready proxy and persisted metadata never imply a ready original | N3/N4; D-STATE provenance; no proxy silently promoted to original |
| Bound pages, indexed id navigation and all-row marquee | N1 pro-scale first, N2 improvement over mounting every card, N7 headless operation |
| Runtime/pickers/import/viewer are nonblocking traits/effects; no JS dependency | N8 main thread never waits; adopted V8/WebKit architecture stays intact |
| Search/proxy additions expose only semantic facts and intents | Lane requirement; design authority remains Opus |
| No existing consumers/files removed | Prune rule; this lane replaces no live consumer yet |

## Verification and remaining work

Linux harness compiles **the same production source and integration test files**
without resolving the macOS-only UI/V8 workspace. Its lockfile is owned by this
crate.

Final Linux result: **62 media behavior tests passed**, including a 60,001-asset
fixture, real-actor Undo/Redo, all context actions, tagged precomp dependencies,
structured extension references, opaque UTF-16 metadata, stale confirmations,
host-owner stamps, explicit asset order and serialized agent inputs/pages.
`cargo fmt --check` and Clippy with `-D warnings` pass for this crate.

An unchanged copy of pm-kernel sources/tests was also run in an external Linux
regression harness (standalone manifest, same engine sources, fixture/oracle
paths preserved, existing v1.2.0/v1.3.0 references available): **273 passed,
0 failed, 1 pre-existing ignored**. This is a Linux regression check, not a
successful run of the macOS workspace or its shared lock.

Run the crate checks with the repository toolchain:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
cargo +1.96.1 test --locked --manifest-path apps/native/crates/panel-media/isolated/Cargo.toml
cargo +1.96.1 clippy --locked --manifest-path apps/native/crates/panel-media/isolated/Cargo.toml --all-targets -- -D warnings
cargo +1.96.1 fmt --manifest-path apps/native/crates/panel-media/isolated/Cargo.toml --check
```

The requested `cd apps/native && cargo test --locked -p pm-panel-media` was
attempted. It refuses because the **shared Cargo.lock needs the new workspace
member**; that lock is outside this lane's write ownership. The integrating lane
must update it and rerun the native workspace checks. No shared lock/manifests,
other crates, app, vendor, spikes or private docs were edited.

macOS verification must cover native pickers and Finder failure/success,
actual import/archive/relink/undo byte provenance, image sequences and Option
folders, cloud retry/progress/cancel, proxy-version registry binding, decoder
poster readiness, source audio/video clock and composition pause, full-keyboard
focus/VoiceOver, pointer capture/autoscroll, cross-panel drags, system menu
modifiers, localized folder collation, and high-volume UI latency/memory.
Headless paging is not evidence of a frame budget or macOS accessibility pass.

Remaining host integration: connect `MediaRuntime` to promedia/engine-service,
route effects through the app's actor and native services, bind id dispatch to
agent/MCP surfaces without changing frozen schemas, and supply the v4 panel view
after the API freezes. The migration/import host must provide original asset
enumeration evidence to `with_asset_order`; exact locale collation needs the
macOS comparator. A document-guarded folding Transact API needs a kernel-lane
change. No claim of shipped app or v4 integration is made here.

**DESIGN-TODO(opus):** specify/review the media view's search/filter and proxy
presentation, status/error/confirmation text mapping, empty/loading/offline/
unknown-analysis treatment, row/thumbnail/waveform presentation, drag/marquee and
focus presentation. This model supplies semantic placeholders (Unknown/None,
ids, enums and raw values) only. Existing tokens/specs own all visuals; all future
visual output remains provisional until Opus review.
