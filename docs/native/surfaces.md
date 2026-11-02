# Shipped surface inventory → Powermove Native

Audit date: **2026-11-01**. Shipped reference: **v1.2.0**, commit
`07ed5fa17ee640659cb075f9cad1af2095011d60`. Native assessment initially uses
`4204a153dbf8af916ef2b92d7ca01858e363516a`; the reconciliation section records
the final merged base. This is a source inventory, not a visual design,
implementation plan approval, or macOS parity result.

The inventory includes editor panels, application screens, windows, transient
controls, system dialogs and failure/recovery states. Rows group states of one
surface where they share an owner; separately launched dialogs get their own
rows. Extension-authored content is an open family, not a finite list of third
party panels. No private extensions, profiles or user data were inspected.

## Reading the inventory

Source shorthand is **only for v1.2.0**: `R/` =
`apps/desktop/src/renderer/src/`, `E/` = `apps/desktop/src/extensions/`,
`M/` = `apps/desktop/src/main/`, `S/` = `apps/desktop/src/shared/`.
Resolve every cited file with `git show v1.2.0:<expanded-path>`; these are
immutable source references, not links to possibly changed working-tree files.

Native shorthand: `N/` = `apps/native/crates/`, `G/` = `engine/crates/`.
The backing column names actual crates/modules, including partial backing.
An engine operation, storage reader or host trait does **not** establish a
complete surface model, adapter, running UI or accessibility implementation.

Status uses the requested three values:

- **spec+model**: surface-specific build guidance exists in design panels or
  reviews, and executable native backing exists for its stated scope. Partial
  behavior and missing integration are stated explicitly.
- **model only**: meaningful native backing exists, but a buildable design for
  this surface is absent or incomplete. A general design language, shared kit
  recipe or neighboring panel spec is insufficient.
- **nothing**: no surface-specific native backing was found. Generic JSON
  preservation, kernel transactions, a placeholder panel id or an extension
  runtime alone do not count. Any related spec is still recorded.

`DESIGN-TODO(opus)` marks a missing design specification or missing states in an
existing spec; it is a neutral placeholder, not permission to invent UI, copy or
interaction. **Every visual result remains provisional until Opus review.**

Impact order is a review priority, not measured usage: **P0** prevents lost work
or unintended authority; **P1** covers opening, editing and delivering work;
**P2** covers specialized creation, agents, customization and extensions;
**P3** covers first-run, maintenance and diagnostics. Rows are sorted by these
impacts, with stable ids for follow-up. A rare P0 failure outranks a frequent
convenience surface under N1/N4.

Design references below are relative to `docs/native/design/`:

| Key | Existing specification or review | Scope / limit |
| --- | --- | --- |
| Shared | [panels/README.md](design/panels/README.md), [language.md](design/language.md), [tokens.md](design/tokens.md) | Shared anatomy, states, accessibility and tokens; not a design for an unlisted surface. |
| Timeline | [panels/timeline.md](design/panels/timeline.md) | Timeline, graph, ruler, tracks and local menus. |
| Inspector | [panels/inspector.md](design/panels/inspector.md) | Properties, composition/no-selection state, typography, colour popover. |
| Effects | [panels/effect-controls.md](design/panels/effect-controls.md) | Applied effect stack and controls. |
| Browser | [panels/effects-presets.md](design/panels/effects-presets.md) | Effects browser. Owner decision in Shared calls it **Effects** until presets exist. |
| Viewer | [panels/viewer-chrome.md](design/panels/viewer-chrome.md) | Viewer chrome, gizmos, banners and source mode; not a full 3D editor specification. |
| Media | [panels/media-browser.md](design/panels/media-browser.md) | Project media browser, bins, import/relink entry points. |
| Agent | [panels/agent-panel.md](design/panels/agent-panel.md) | Popover/pinned presentation, composer, conversation, inline questions, pickers and results. |
| Settings | [panels/settings.md](design/panels/settings.md) | Project, General, Accounts, Transcription, Extensions; sections may delegate detailed workflows. |
| Export | [panels/export-sheet.md](design/panels/export-sheet.md) | Export settings, destination and progress. Web-animation export is explicitly removed by owner decision. |
| Shell review | [reviews/na-shell2.md](design/reviews/na-shell2.md), [reviews/na-shell3.md](design/reviews/na-shell3.md) | Shell, palette, workspace sheet, docking and tab/menu fixes; not every project dialog. |
| Viewer review | [reviews/na-viewer2.md](design/reviews/na-viewer2.md) | Viewer and shell review findings; open items remain evidence gaps. |
| Services review | [reviews/na-cloud-services.md](design/reviews/na-cloud-services.md) | Library row data/vocabulary, error copy, update sheet, font family/face menus and recovered-document notice. Library layout is not fully specified. |
| Store-host review | [reviews/na-cloud-storehost.md](design/reviews/na-cloud-storehost.md) | Batched isolated-permission sheet, Full Access alert, stored decisions, stale prompts and missing display-data fallbacks. Not a Store-screen specification. |
| Pro-media review | [reviews/na-cloud-promedia.md](design/reviews/na-cloud-promedia.md) | Render bar and Background Tasks popover/window, labels/errors and render-queue replacement; ETA/terminal rows and AX summary are deferred. |
| Kit review | [reviews/na-cloud-uikit.md](design/reviews/na-cloud-uikit.md) | Native control recipes, semantics and accessibility; renderer integration/review still required. |
| Theme reviews | [reviews/themes.md](design/reviews/themes.md), [reviews/na-cloud-editions.md](design/reviews/na-cloud-editions.md) | Theme translation and compat DOM appearances; not a theme-browser or arbitrary extension-content redesign. |

## Inventory, sorted by user impact

| Id / impact | User-facing surface and included states | v1.2.0 source files | Native design coverage / Opus gap | Native backing and status |
| --- | --- | --- | --- | --- |
| S001 P0 | Editor crash/OOM alert: Reload or Close Window; recovered project on relaunch | `M/renderer-recovery.ts`; `R/legacy/core/projects.ts`; `M/index.ts` | Services review specifies recovered-document notice and error vocabulary only. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S001](design/surfaces/recovery.md). **Recovery lane (2026-11-01):** recovery result/ticket adapter, exact alert/banner copy, FIFO/stale-safe actions and AppKit NSAlert presenter tested headlessly; application attachment and Recently Deleted/saved-only kernel APIs pending. | `pm-services::diagnostics::CrashRecord`; `pm-kernel::{journal,session}`. Native crash capture/host presentation pending. **spec+model** |
| S002 P0 | Unsaved tab/window/quit confirmation; wait for active save/import; save failure keeps work open | `R/legacy/app.ts`; `M/save.ts`; `M/windows.ts` | No complete spec. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S002](design/surfaces/documents.md). | `pm-kernel::session` close/save/dirty policies; `pm-shell::session` windows. Dialog/quit wiring pending. **spec+model** |
| S003 P0 | External project changed, uncertain save, recovery copy/backup and save destination failures | `M/project-files.ts`; `M/save.ts`; `R/legacy/app.ts` | Services review has generic errors, not conflict decisions. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S003](design/surfaces/documents.md). | `pm-kernel::{session,project_io,locks}` preserves conflict copies and typed save outcomes. **spec+model** |
| S004 P0 | Invalid/locked/read-only/cloud-volume project open errors and copy policy | `M/save.ts`; `M/project-files.ts`; `R/legacy/app.ts` | Services review gives failed-open copy and Show in Finder; no complete lock/copy flow. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S004](design/surfaces/documents.md). | `pm-kernel::{locks,session}`; `pm-services::entrypoints::{EntryQueue,FileOpenFailure}`. **spec+model** |
| S005 P0 | Project Trash, restore and permanent-delete confirmation | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review specifies Move to Trash / Put Back / Recently Deleted only. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S005](design/surfaces/library.md). | `pm-services::library::LibraryAction`; `pm-kernel::{persistence,commands::library}`. N4 requires archives, not shipped irreversible deletion. **spec+model** |
| S006 P0 | Offline/changed source badges, locate/relink/replace and missing-original export warning | `R/panels/AssetsPanel.svelte`; `R/legacy/core/media.ts`; `R/legacy/gl/raster.ts`; `R/legacy/core/project-file.ts` | Media and Export cover entry points/warnings. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S006](design/surfaces/media.md). | `pm-panel-media::{model,edit}`; `pm-promedia::media::{Availability,MediaCatalog}`; `pm-engine-service` media mapping. **spec+model** (partial) |
| S007 P0 | Cloud-media download consent, per-file progress/cancel/retry and automatic-download preference | `M/cloud-media.ts`; `R/legacy/core/cloud-media.ts`; `R/settings/SettingsScreen.svelte` | Settings specifies the preference; Media covers availability. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S007](design/surfaces/media.md). | `pm-panel-media` host effects; `pm-services::media::ImportQueue` is partial queue backing, not cloud hydration. Production cloud adapter absent. **spec+model** |
| S008 P0 | Store Full Access trust alert and revoke/retrust entry points | `M/cloud/trust.ts`; `M/cloud/store-ipc.ts`; `R/store/StoreScreen.svelte` | Store-host review specifies alert, exact trust copy, Cancel default, identity fallback and stale handling. | `pm-store-host::permission::{PermissionRequest,Admission}` and stored decisions; host alert wiring pending. **spec+model** |
| S009 P0 | Isolated extension permission disclosure/admission and revocation | `R/store/StoreScreen.svelte`; `M/cloud/permission-scan.ts`; `M/cloud/store-ipc.ts`; `R/legacy/ui/extension-settings.ts` | Store-host review specifies batched permission sheet, display identity and Settings permission switches. This is native replacement guidance; tag discloses permissions on Store detail rather than presenting the same new sheet. | `pm-store-host::permission` request/respond-all/revoke contracts. **spec+model** |
| S010 P0 | Agent per-project computer-access and standing Full Access confirmations | `M/codex/consent.ts`; `R/panels/agent/AccessPicker.svelte`; `M/agent-approvals.ts` | Agent specifies access picker/inline approvals, not these full native consent dialogs. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S010](design/surfaces/authority.md). | `pm-agents::approvals`; `pm-panel-agentpanel` access commands/effects. OS consent host pending. **spec+model** |
| S011 P0 | Extension variable setup, secret input/question, missing required values | `R/vars/SetupSheet.svelte`; `R/vars/setup.ts`; `M/env/ipc.ts` | Settings links to setup; Agent covers secret question cards only. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S011](design/surfaces/authority.md). | Migration retains env payloads; no complete native variable setup/service model found. **nothing** |
| S012 P0 | Uninstall extension / also remove its entered values; local panel/workspace deletion confirmations | `M/env/ipc.ts`; `R/legacy/ui/extension-settings.ts`; `R/library/LibraryScreen.svelte`; `R/panels/WorkspacesPanel.svelte` | Store-host/Settings cover revoke/management, not complete removal choices. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S012](design/surfaces/authority.md). | `pm-store-host::install::{ArchivedRelease,ArchiveReason}`; `pm-shell::layout::LayoutStore` supports saved-workspace changes; variable archive flow incomplete. **spec+model** |
| S013 P0 | Extension panel crashed/hung/unavailable: recovery, reload, disable | `R/kernel/sandbox-host.ts`; `R/kernel/sandbox-view.ts`; `R/legacy/ui/extension-settings.ts` | Shared error states and Store-host lifecycle review are not a full recovery design. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S013](design/surfaces/recovery.md). **Recovery lane (2026-11-01):** all lifecycle copy/AX states, generation-fenced restart requests and v4 EmptyState content tested; serialized source contract checked against real store-host enums. Watchdog/application attachment and symbol slot pending. | `pm-store-host::view::{ViewStatus,RecoveryPolicy,Action}`; runtime/compat lifecycle models. Full application watchdog wiring pending. **spec+model** |
| S014 P1 | Main editor windows, title/dirty indicator, project tabs, tab menu, move tab to another/new window, restore | `M/windows.ts`; `M/index.ts`; `R/shell/Titlebar.svelte`; `R/legacy/app.ts` | Shell review covers native window/tab/title behavior. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S014](design/surfaces/documents.md). | `pm-shell::{session,native}`; `pm-kernel::session`; native window/kernel ownership integration remains. **spec+model** (partial) |
| S015 P1 | Docked panels, headers, collapse/hide, splitters, drag/drop previews, add/close/move panel menu | `R/layout/Dock.svelte`; `R/layout/DockLayout.svelte`; `R/layout/Splitter.svelte`; `R/legacy/ui/layout.ts` | Shared and Shell review specify native docking, context menus, previews and header state. | `pm-shell::layout::{Layout,DropTarget,DropPreview}`; app must connect arbitrary extension panel ids. **spec+model** |
| S016 P1 | Popped-out panel window, focus/close/return and shared project ownership | `M/panel-popout.ts`; `R/layout/PanelPool.svelte`; `R/legacy/ui/layout.ts` | Shared mentions Pop Out; no full popout lifecycle spec. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S016](design/surfaces/documents.md). | `pm-shell` window/layout models and engine independent viewer support are partial; arbitrary panel detach/reattach host remains. **spec+model** |
| S017 P1 | Home/project library: recent/all projects, search/sort, grid/list, project cards and empty state | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review defines row facts/vocabulary; no complete Project Library window design. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S017](design/surfaces/library.md). | `pm-services::library::{ProjectEntry,ProjectSummary,LibraryService}`; `pm-kernel` library. **spec+model** |
| S018 P1 | New Project dialog, defaults and creation errors | `R/legacy/app.ts`; `R/legacy/ui/project-settings.ts` | No dedicated spec. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S018](design/surfaces/library.md). | `pm-kernel::{session,commands::library}` creates projects; dialog state/host missing. **spec+model** |
| S019 P1 | Rename/duplicate project, project-card/context actions | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review has library vocabulary/data only. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S019](design/surfaces/library.md). | `pm-services::library::LibraryAction`; `pm-kernel::session`. **spec+model** |
| S020 P1 | Open/Save/Save As `.pmv` system picker; import/export file and directory pickers | `M/save.ts`; `M/render-encoder.ts`; `R/legacy/app.ts` | Export specifies export destination picker; no complete project open/save picker policy. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S020](design/surfaces/documents.md). | `pm-kernel::{session,project_io}`; `pm-services::entrypoints`; platform picker adapter required. **spec+model** |
| S021 P1 | New Composition / Composition Settings dialog; size/FPS/duration/background | `R/legacy/ui/compositions.ts`; `R/legacy/ui/project-settings.ts`; `E/inspector/CompositionSection.svelte` | Inspector and Settings cover inline fields; no complete standalone dialog spec. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S021](design/surfaces/dialogs.md). | `pm-kernel::commands::comps`; `pm-panel-inspector` composition bindings. **spec+model** |
| S022 P1 | Pre-compose dialog and composition-delete confirmation | `R/legacy/ui/compositions.ts`; `R/panels/AssetsPanel.svelte` | Timeline/Media expose actions, not complete dialogs. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S022](design/surfaces/dialogs.md). | `pm-kernel::commands::{comps,groups}`; `pm-panel-media::edit`. **spec+model** |
| S023 P1 | Viewer canvas/chrome: zoom/fit/custom zoom, quality, safe areas, controls, focus/fullscreen, return-to-composition and offscreen recovery | `E/viewer/viewer.ts`; `E/viewer/index.ts`; `R/legacy/core/preview-resolution.ts` | Viewer and Viewer review specify chrome, quality and recovery banners. | `pm-panel-viewerchrome::{Navigation,ViewModel,Controller}`; `pm-engine-service` presentation. Physical EDR/input/performance gates open. **spec+model** |
| S024 P1 | Source-preview viewer: image/audio/video, scrub/play/time/range, close and drag source | `E/viewer/source-preview.ts`; `R/panels/AssetsPanel.svelte` | Viewer and Media specify source mode; decoder-error states were missing. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S024](design/surfaces/media.md). | `pm-panel-media::preview`; `pm-panel-viewerchrome::source`; engine independent source viewers. Host/media leases pending. **spec+model** (partial) |
| S025 P1 | On-canvas selection, transform/anchor/rotation, shape/pen/mask editing, text caret/selection/paragraph box and gizmos | `E/viewer/viewer.ts`; `E/viewer/path-editing.ts`; `E/viewer/text-editor.ts`; `E/viewer/text-geometry.ts` | Viewer covers gizmo appearance; Inspector covers properties. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S025](design/surfaces/editing.md). | Viewer model supplies selection/overlay plans; `pm-kernel::commands::{transform,masks,text}` and engine geometry. Full pointer/text/path editing host remains. **spec+model** (partial) |
| S026 P1 | Main tool strip / registered headless `toolbar` (Tools) panel and creation menu; selection/hand/zoom/rotate/anchor, drawing/text/shape tools | `E/toolbar/index.ts`; `R/shell/ToolbarMount.svelte` | Shell review and Viewer specify toolbar/tool behavior; Shared owns tokens. | `pm-shell` toolbar/command tables; viewer `input` and tool state. **spec+model** |
| S027 P1 | Timeline panel: composition tabs, transport/time, layer switches/parenting, clips, work area, ruler, snapping, scroll/zoom, menus | `E/timeline/timeline.ts`; `E/timeline/comp-tabs.ts`; `E/timeline/track-view.ts`; `E/timeline/track-layout.ts` | Timeline supplies surface and states. | `pm-panel-timeline` snapshot/view/input/behavior models; `pm-kernel::commands::{track,timing,markers}`; engine transport. Native painter/input binding pending. **spec+model** |
| S028 P1 | Graph/dope sheet/combined views, channel rows, key diamonds, curve handles, interpolation and graph menus | `E/timeline/graph-controls.ts`; `E/timeline/graph-presentation.ts`; `E/timeline/timeline.ts` | Timeline specifies graph and local selection/focus/drag behavior. | `pm-panel-timeline::{TimelineModel,GraphDisplay}` and keyframe commands. **spec+model** |
| S029 P1 | Keyframe Value, Keyframe Velocity and Scale Selected Keyframes dialogs | `E/timeline/timeline.ts`; `E/timeline/graph-controls.ts` | Timeline names interactions but lacks complete dialog composition/copy. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S029](design/surfaces/dialogs.md). | `pm-panel-timeline::behaviour` planners; `pm-kernel::commands::keyframes`. **spec+model** |
| S030 P1 | Timeline marker/work-area menus and add/remove marker affordance | `E/timeline/markers.ts`; `E/timeline/timeline.ts` | Timeline ruler/work-area/menu guidance. | `pm-panel-timeline`; `pm-kernel::commands::{markers,comps}`. **spec+model** |
| S031 P1 | Properties/Inspector: composition/no selection, multi-selection/mixed values, content/transform/structured/mask/time/layer sections | `E/inspector/InspectorPanel.svelte`; `E/inspector/InspectorSurface.svelte`; `E/inspector/ContentSection.svelte`; `E/inspector/StructuredSection.svelte`; `E/inspector/MasksSection.svelte`; `E/inspector/RetimingSection.svelte`; `E/inspector/LayerOptions.svelte` | Inspector specifies section order, states, fields and local controls. | `pm-panel-inspector::{catalog,content,relations,edit,view}`; kernel property/mask/timing commands. **spec+model** |
| S032 P1 | Effect Controls panel: add/select/reorder/enable/remove, clipboard, effect parameters, missing definitions | `E/inspector/EffectsPanel.svelte`; `E/inspector/EffectsSection.svelte`; `E/inspector/ParameterList.svelte`; `E/inspector/effect-clipboard.ts` | Effects specifies stack, parameters, states and menus. | `pm-panel-inspector::effects` and kernel effect commands; runtime registry must supply definitions. **spec+model** |
| S033 P1 | Effects browser: search, categories, previews, drag/add effects and transitions | `R/panels/FxBrowserPanel.svelte`; `R/panels/fx-browser.svelte.ts`; `E/effects-basic/index.ts`; `E/transitions-basic/index.ts` | Browser specifies this surface, titled Effects per owner decision. | Kernel registered definitions and `pm-panel-inspector::effects` apply planner exist; dedicated search/category/preview browser model not found. **spec+model** (operation backing only; browser model remains) |
| S034 P1 | Font family/face popover, font preview, variable axes, text alignment and text animators | `R/controls/FontField.svelte`; `E/inspector/TypeSettings.svelte`; `E/inspector/TypeAxis.svelte`; `E/inspector/TextAnimators.svelte`; `E/inspector/TextAlignment.svelte` | Inspector plus Services review specify fonts/face menu and type rows. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S034](design/surfaces/editing.md). | `pm-services::fonts::{FontCatalog,Face}`; `pm-panel-inspector::content`; kernel text commands. **spec+model** (partial) |
| S035 P1 | Colour/fill popover: None/solid/linear/radial; sRGB/Display P3; OKLCH/HSL/RGB/OKLab/P3 notation and copy; hue mapping, previous/new swatches, gamut-clipping warning, alpha, gradient stops/midpoint and screen eyedropper | `R/controls/ColorField.svelte`; `R/controls/FillField.svelte`; `R/controls/RampField.svelte`; `R/controls/color-space.ts` | Inspector §7 and Kit review specify a simpler popover/ramp. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S035](design/surfaces/editing.md). | `pm-panel-inspector` typed binding/edit/input; `pm-surface::kit` colour-well recipe; engine colour math. No complete colour-picker/recent-colour or NSColorSampler adapter yet. **spec+model** (partial) |
| S036 P1 | Property context menu and expression editor modal: presets/edit/remove/reset/reveal/keyframe | `E/inspector/ChannelRow.svelte`; `E/inspector/PropertyStopwatch.svelte`; `R/legacy/core/expression.ts` | Inspector describes context actions, not full expression-editor modal. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S036](design/surfaces/dialogs.md). | `pm-panel-inspector::edit` property actions; kernel channel/expression commands; engine expression evaluation. **spec+model** |
| S037 P1 | Project/Media panel: composition/media/bin lists, selection, rename, drag, new/import/context menus, missing media | `R/panels/AssetsPanel.svelte`; `R/panels/register-simple.ts` | Media specifies rows/grid, bins, selection, empty/loading/offline states. | `pm-panel-media::{catalog,model,input,edit}`. Tag enumeration/collation and actual host operations still need integration. **spec+model** |
| S038 P1 | Import progress, conversions/proxies, drag/file/folder/paste imports, cancellation and failures | `R/legacy/core/import-progress.ts`; `R/legacy/core/folder-import.ts`; `R/legacy/core/media-work.ts`; `R/legacy/core/media-paste.ts`; `M/media-proxy.ts` | Media covers entry/row state; Pro-media review specifies Background Tasks. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S038](design/surfaces/media.md). | `pm-services::media::ImportQueue`; `pm-promedia::{media,tasks}`; engine probe/codec hooks. Full original-byte admission remains. **spec+model** (partial) |
| S039 P1 | Import Image Sequence dialog: ordered files, FPS/range/name and replacement workflow | `R/legacy/core/image-sequence.ts`; `M/media-proxy.ts`; `R/panels/AssetsPanel.svelte` | Media lists the action, not complete form. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S039](design/surfaces/media.md). | Media import/edit effects and engine sequence handling are partial; no complete sequence dialog model. **spec+model** |
| S040 P1 | Paste Figma selection: fallback warning/confirmation, image fallback and unsupported layers | `R/legacy/core/figma/paste.ts`; `R/legacy/core/figma/convert.ts`; `M/clipboard.ts` | No dedicated design. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S040](design/surfaces/media.md). | Native clipboard/engine geometry alone do not implement tagged Figma conversion/fallback flow. **nothing** |
| S041 P1 | Export settings sheet: format/codec, dimensions/FPS/range/audio/alpha/colour/captions options and preview | `R/legacy/core/exporter.ts`; `R/legacy/core/export-preview.ts`; `R/legacy/ui/project-settings.ts` | Export specifies settings/validation. | `pm-engine-service::jobs`; `pm-export`; kernel delivery state. Full sheet reducer and all option bindings remain. **spec+model** (partial) |
| S042 P1 | Export destination picker and progress/cancel/success/error, reveal output, resume editing | `R/legacy/core/exporter.ts`; `M/render-encoder.ts`; `M/save.ts`; `M/export-sidecar.ts` | Export §3 and Pro-media review specify destination/progress/completion. | Engine export jobs and `pm-promedia::tasks`; native encoder/destination/sidecar binding required. **spec+model** |
| S043 P1 | Export preset save/select/manage and queued export action | `R/legacy/core/exporter.ts`; `R/legacy/core/render-queue.ts` | Export covers preset controls; detailed preset naming/management is incomplete. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S043](design/surfaces/dialogs.md). | Kernel preserves `renderPresets`/`exportOpts`; export jobs have options, but no preset-manager model found. **spec+model** (export operation backing) |
| S044 P1 | Shipped Code (web animation) export, validation/warnings and standalone archive/player delivery | `R/legacy/core/exporter.ts`; `R/player/export-web.ts`; `R/player/player.ts` | Export/Shared explicitly record the owner decision to remove this option; no new spec requested. Preserve existing authored projects/assets and exported files. | No native web-animation delivery replacement claimed. **nothing** (explicit owner removal; W072) |
| S045 P1 | Render Queue modal: composition snapshots, pending/rendering/done/error, render pending, retry/remove | `R/legacy/core/render-queue.ts`; `R/legacy/core/exporter.ts` | Pro-media review explicitly replaces queue modal with Background Tasks and maps Retry/Remove. | `pm-promedia::tasks::BackgroundTasks`; migration preserves queue input. Adapter must retain queued composition/options/snapshot/output identity. **spec+model** (migration/queue adapter pending) |
| S046 P1 | Background activity pill and task popover: import/export/media work, progress/cancel and completed/failed work | `R/shell/TaskPill.svelte`; `R/shell/tasks.svelte.ts`; `R/legacy/core/media-work.ts` | Pro-media review supplies native Background Tasks popover and utility window. Native Pause/Resume and richer task kinds are additions. | `pm-promedia::tasks` title/unit/state/error/event models. ETA, terminal-row retention and shell wiring remain per review. **spec+model** |
| S047 P1 | Command palette: asynchronous providers, filtering, command availability, shortcuts, keyboard/focus | `R/overlays/Palette.svelte`; `R/overlays/palette.ts`; `R/overlays/palette-model.ts` | Shell review specifies native palette and shortcut column; full extension-provider state coverage still needs integration review. | `pm-shell::{transient,native,keyboard}`; compatibility contribution bridge required. **spec+model** |
| S048 P1 | Choice/dropdown/popover menus: searchable field choices, checks/disabled/header/separators, anchoring/dismissal | `R/controls/popover-menu.ts`; `R/overlays/Menu.svelte`; `R/overlays/native-menu.ts`; `R/controls/SelectField.svelte` | Shared, Kit review, Shell review and field specs cover shared behavior. Preserve tag follow-up menus; structured submenu API is later. | `pm-shell::{transient,native}`; `pm-surface::kit`; `pm-compat-host` menu protocol. **spec+model** |
| S049 P1 | Native application menus: Powermove, File, Edit, Composition, View, Window; OS Services, hide/minimize/fullscreen | `M/menu.ts`; `M/updates.ts` | Shell review covers native menus; Shared/Settings/Export supply owned actions. Tag has no top-level Layer or Help menu. | `pm-shell::{menu,keyboard,native}` frozen commands/equivalents; kernel command eligibility/acknowledgement integration remains. **spec+model** |
| S050 P1 | Context menus: viewer/layer, timeline/keyframe/property, asset/bin/composition and panel header | `R/legacy/ui/layer-menu.ts`; `E/timeline/timeline.ts`; `E/inspector/ChannelRow.svelte`; `R/panels/AssetsPanel.svelte`; `M/context-menu.ts` | Viewer/Timeline/Inspector/Media/Shared specify local menus, not a blanket license to copy later-main entries. | Native shell/menu and panel semantic action models; runtime extension contributions pending. **spec+model** |
| S051 P1 | Text context menu, spelling/edit commands, native text services and IME candidate UI | `M/text-context-menu.ts`; `M/native-edit.ts`; `R/controls/TextField.svelte`; `R/panels/agent/Composer.svelte` | Shared/Kit review and panel keyboard rules defer native text services to system. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S051](design/surfaces/editing.md). | `pm-shell::{text,keyboard,native}`; agent composer local text model. Real NSTextInput/WebKit/IME/AX integration requires Mac. **spec+model** (partial) |
| S052 P1 | Shared modal/sheet/confirm host: focus trap/return, Enter/Escape, scrim/outside dismissal | `R/overlays/Modal.svelte`; `R/overlays/modal.ts`; `R/overlays/confirm.ts`; `M/native-confirm.ts` | Shared and Shell/Kit reviews specify native sheets/system placement; surface-specific copy must come from its own spec. | `pm-shell::native::Sheet`; `pm-surface::kit` SheetAnchor contracts; compat modal handles. **spec+model** |
| S053 P1 | Toast/error notices, expandable diagnostic details/copy, retry/fix actions | `R/overlays/Toasts.svelte`; `R/errors/ToastError.svelte`; `R/errors/ErrorNotice.svelte`; `R/errors/ErrorLog.svelte`; `R/errors/ErrorFix.svelte` | Shared inline-error policy + Services review typed copy; not every notice/repair state is covered. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S053](design/surfaces/recovery.md). **Recovery lane (2026-11-01):** ten-code copy catalog, typed one-action notices, Error Log selection/copy/clear model and native table/split-view recipe tested. Native host rendering/service subscription pending. | `pm-services::{error,diagnostics}` and agent error facts; no full native notice/log UI model. **spec+model** |
| S054 P1 | Status bar: playback/performance/status contributions and “What is slowing the editor down” details modal | `R/shell/StatusBar.svelte`; `R/runtime/performance-monitor.ts` | Shell review covers status-bar vocabulary; no slowdown-details design. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S054](design/surfaces/recovery.md). **Recovery lane (2026-11-01):** ordered status/AX facts, Unicode-safe extension contributions and revision-fenced slowdown popover recipe/actions tested. Shell/telemetry attachment and NSPopover rendering pending. | Shell status state and engine telemetry are partial. **spec+model** |
| S055 P1 | Project opening ghost/loading overlay and delayed transitions; retained tab intent | `R/overlays/ProjectGhost.svelte`; `R/overlays/project-ghost.ts`; `R/legacy/app.ts` | Shared loading states do not specify project-level overlay. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S055](design/surfaces/documents.md). | Kernel async session/open and shell tab state exist; ghost/overlay model absent. **spec+model** |
| S056 P2 | Agent launcher/popover, float/pin/dock/resize and busy/unread indication | `R/shell/AgentLauncher.svelte`; `R/panels/register-agent.ts`; `R/panels/agent-shell.ts`; `R/legacy/assistant/spatial.ts` | Agent explicitly chooses popover with pinning; owner decision supersedes competing presentation proposals. | `pm-panel-agentpanel::placement`; `pm-shell` Agent toolbar action. App placement adapter pending. **spec+model** |
| S057 P2 | Agent conversation: streaming text/Markdown, reasoning/tool activity groups, timeline/working timer, follow/scroll and plan preview | `R/panels/AgentPanel.svelte`; `R/panels/agent/Conversation.svelte`; `R/panels/agent/Timeline.svelte`; `R/panels/agent/Turn.svelte`; `R/panels/agent/Markdown.svelte`; `R/panels/agent/PlanPreview.svelte` | Agent covers main conversation and states. **DESIGN-TODO(opus):** any unlisted plan/archived-tool-detail layouts. | `pm-panel-agentpanel::{activity,model,view}`; `pm-agents::{conversation,persistence}`. Renderer/provider host integration pending. **spec+model** |
| S058 P2 | Agent composer, attachment chips, slash menu, draft/clipboard/selection and send/stop | `R/panels/agent/Composer.svelte`; `R/panels/agent/AttachmentChips.svelte`; `R/panels/agent/SlashMenu.svelte`; `R/panels/agent/attachments.ts` | Agent specifies composer and local focus/keyboard rules. | `pm-panel-agentpanel::{composer,input,interaction}`; native attachment/clipboard effects remain host-owned. **spec+model** |
| S059 P2 | Agent model/effort/provider/access/options menus and connection gate | `R/panels/agent/ModelPicker.svelte`; `R/panels/agent/AccessPicker.svelte`; `R/panels/agent/AgentOptions.svelte`; `R/panels/agent/ConnectionGate.svelte` | Agent covers picker/gate anatomy. **DESIGN-TODO(opus):** provider authentication/subscription details beyond the gate. | Agent panel model/view connection generations; `pm-agents` provider protocol adapters. Auth/catalog host incomplete. **spec+model** (partial) |
| S060 P2 | Thread picker/history: new/ordinary/background threads, switch/search/rename/delete confirmation and unread/running state | `R/panels/agent/ThreadPicker.svelte`; `R/legacy/assistant/spatial.ts` | Agent covers thread picker; destructive/archive/history detail states incomplete. **DESIGN-TODO(opus):** full history management and delete-as-archive confirmation. | `pm-panel-agentpanel::{model,view}`; `pm-agents::persistence::Threads`; orchestration/thread tool bindings incomplete. **spec+model** (partial) |
| S061 P2 | Inline agent questions, approvals, text/secret answers, suggested options and pending response | `R/panels/agent/QuestionCard.svelte`; `R/panels/agent/inline-prompt.ts`; `M/agent-approvals.ts` | Agent specifies question-card behavior; no selection may submit implicitly. | `pm-panel-agentpanel::questions`; `pm-agents::approvals`. Native reply worker wiring pending. **spec+model** |
| S062 P2 | Agent visual reply, generated UI placement ghost/preview and reopen interactive artifact | `R/panels/agent/VisualReply.svelte`; `R/panels/agent/UIPlacementGhost.svelte`; `R/panels/agent/ui-placement.ts`; `R/panels/agent/visual-reply.ts` | Agent covers visual replies at a high level; full placement/artifact surface design missing. **DESIGN-TODO(opus):** preview placement, adoption and reopen/error states. | `pm-panel-agentpanel::{activity,view,model}` retains references/interactions; actual v4/compat renderer binding pending. **model only** |
| S063 P2 | Agent result cards and actions: apply/undo, extension/mod result, project handoff and follow-up | `R/panels/agent/ModResult.svelte`; `R/panels/agent/ResultActions.svelte`; `R/panels/agent/GhostCard.svelte`; `R/panels/agent/mod-result.ts` | Agent specifies result actions; specialized extension result states need **DESIGN-TODO(opus)**. | Agent panel immutable result/activity models, guarded kernel result undo; application artifact/adoption effects remain. **spec+model** (partial) |
| S064 P2 | Repair Agent/error recovery card and repair draft flow | `R/panels/RepairAgent.svelte`; `R/panels/repair-agent.svelte.ts`; `R/errors/ErrorFix.svelte` | No complete repair-flow design. **DESIGN-TODO(opus):** diagnostic disclosure, repair request and recoverable failure. | General agent error/effect infrastructure is not a Repair Agent flow model. **nothing** |
| S065 P2 | Agent attachment image-preview modal and reveal/open attachment actions | `R/panels/agent/attachments.ts`; `M/shell.ts` | Agent mentions attachment activation; full preview modal absent. **DESIGN-TODO(opus):** image preview, unavailable attachment and focus return. | `pm-panel-agentpanel::composer` attachment records and host effects; byte hydration/reveal adapter pending. **model only** |
| S066 P2 | Spatial agent selection overlay: drag an interface region, screenshot attachment, target/focus picker, anchored prompt, send/cancel, reduced-motion/ripple fallback | `R/legacy/assistant/spatial.ts`; `R/legacy/assistant/RippleCanvas.svelte`; `R/legacy/assistant/RippleFrame.svelte` | Agent spec covers conversation placement, not spatial region selection. **DESIGN-TODO(opus):** region/target selection, screenshot disclosure and anchored prompt/failure states; no Electron ripple styling implied. | Agent composer/context and kernel selection provide partial backing; no native spatial overlay/capture controller found. **model only** |
| S067 P2 | Settings window/screen: search/sidebar sections, selection, no results and close | `R/settings/SettingsScreen.svelte`; `R/settings/install.ts`; `R/settings/search.ts` | Settings specifies native window, section/search states and keyboard. | `pm-shell` Settings shell; `pm-services::settings::SettingsService`; kernel state. Search/form reducers and controls still need wiring. **spec+model** |
| S068 P2 | Project Settings pane: composition, colour/depth, preview/delivery defaults and custom dimensions | `R/legacy/ui/project-settings.ts`; `R/settings/SettingsScreen.svelte`; `R/core/color-depth.ts` | Settings Project section and Export cover these fields. **DESIGN-TODO(opus):** uncovered colour/depth/validation states rather than inferring from engine options. | `pm-panel-inspector` composition binding; kernel state/commands; engine output/colour configuration. **spec+model** (partial) |
| S069 P2 | General Settings: appearance, workspace import, preview memory, cloud downloads and restore windows | `R/settings/SettingsScreen.svelte`; `R/onboarding/workspace-import.ts` | Settings specifies General rows. Import workflow is separately missing in S110. | `pm-services::settings`; kernel `StateTransaction`; `pm-shell` appearance/session. Actual preference consumers must be wired. **spec+model** |
| S070 P2 | Accounts subscriptions: ChatGPT/Codex and Claude sign-in/status/disconnect; external MCP client registration | `R/legacy/ui/chatgpt-settings.ts`; `M/codex/app-server-account.ts`; `M/claude/account.ts`; `M/claude-desktop-mcp.ts` | Settings names subscriptions; full connection, browser/device-code and external-client install/undo states missing. **DESIGN-TODO(opus):** these account workflows. | `pm-agents::{codex,claude,protocol}` partial provider backing; `pm-mcp` transport. OS registration/auth/secure credential host remains. **model only** |
| S071 P2 | API/local model provider configuration: endpoint/key/model/vision, validation/test/disconnect | `R/legacy/ui/compatible-settings.ts`; `M/compatible-provider.ts` | Settings Accounts gives placement, not full provider form/testing states. **DESIGN-TODO(opus):** compatible-provider configuration. | Migration preserves provider settings/secrets; no complete native provider-config/auth model found. **nothing** |
| S072 P2 | Agent runtime install/download/update progress and runtime errors | `M/runtime-updates.ts`; `M/codex/connection-status.ts`; `R/legacy/ui/chatgpt-settings.ts`; `R/panels/agent/ConnectionGate.svelte` | Agent/Settings specify gate/status only. **DESIGN-TODO(opus):** runtime install/update/retry/offline states. | Native agent process/protocol models exist; complete runtime installer/updater missing. **model only** |
| S073 P2 | Extensions Settings: category/filter/status, detail/contributions/replacement/source, enable/reload/fork/show in Finder/delete/setup | `R/legacy/ui/extension-settings.ts`; `R/settings/SettingsScreen.svelte`; `M/extensions/registry.ts` | Settings and Store-host review cover management rows/permissions, not every detail/contribution/fork state. **DESIGN-TODO(opus):** full extension-detail and failure/replacement states. | Kernel `extensions` metadata/state; `pm-store-host::{install,permission}`; runtime activation/reload. Full management view reducer remains. **spec+model** (partial) |
| S074 P2 | Theme/appearance choices and extension-owned reversible workspace/theme appearance | `R/kernel/theme-state.ts`; `R/kernel/workspace-appearance.ts`; `E/theme-default/index.ts`; `R/settings/SettingsScreen.svelte` | Settings, Shared and Theme reviews govern appearance. No new theme-browser layout is implied. | `pm-design` tokens; kernel contribution state; `pm-compat-host` theme pipeline/editions. **spec+model** |
| S075 P2 | Store screen: featured/browse/categories/search/sort/pagination and offline/empty/loading | `R/store/StoreScreen.svelte`; `R/store/data.ts`; `R/store/install.ts` | No Store-screen spec; Store-host review covers permissions only. **DESIGN-TODO(opus):** Store browsing/navigation/states. | `pm-services::store::{StoreClient,Search,SearchPage}` metadata subset. **model only** |
| S076 P2 | Store extension detail: icon/author/verification/description/screenshots/version/source/permissions, install and compatible/unavailable states | `R/store/StoreScreen.svelte`; `R/store/StoreIcon.svelte`; `M/cloud/store-client.ts` | Store-host review supplies consent/disclosure vocabulary only. **DESIGN-TODO(opus):** detail page and install lifecycle. | Services release/tree/install plans; `pm-store-host` verification/install/admission. Catalog/detail host coverage incomplete. **model only** |
| S077 P2 | Store Your Library/Yours: installed/remembered/published items, trust/untrust, update/uninstall and account state | `R/store/StoreScreen.svelte`; `M/cloud/store-ipc.ts`; `M/cloud/install.ts` | No complete design. **DESIGN-TODO(opus):** installed/remembered/publisher library and actions. | Services installed/version metadata; Store-host install archives/trust; account/remote remember-list host missing. **model only** |
| S078 P2 | Extension update notices, fork update/rebase, local-change comparison and apply/retry | `R/shell/extension-update-notices.ts`; `R/shell/fork-updates.ts`; `R/shell/update-fork.ts`; `M/extensions/update-fork.ts`; `M/extensions/rebase.ts` | No complete fork/update design. **DESIGN-TODO(opus):** extension update/compare/rebase/conflict states. | Services update plans and Store-host archived versions are partial; full fork/rebase coordinator missing. **model only** |
| S079 P2 | Publish extension sheet: project/source selection, name/slug/version/category/description/permissions and validation | `R/store/PublishSheet.svelte`; `R/store/publish-form.ts`; `R/store/publish-sheet.ts` | No publish-sheet spec. **DESIGN-TODO(opus):** all publish form/edit/validation states. | No complete native publishing model; services install metadata and migration storage do not implement publishing. **nothing** |
| S080 P2 | Publish preparing/progress/checklist and native final publish confirmation; failure/retry/yank | `R/store/PublishPreparing.svelte`; `M/cloud/publish.ts`; `R/store/StoreScreen.svelte` | No publishing-flow design. **DESIGN-TODO(opus):** preparation/progress, native confirmation, failure and yank states. | `pm-mcp` publish tools remain pending; no production native publishing flow model found. **nothing** |
| S081 P2 | Store icon image cropper, tag/search picker and publish project picker | `R/store/IconCropper.svelte`; `R/store/SearchPicker.svelte`; `R/store/project-picker.ts` | No dedicated specification. **DESIGN-TODO(opus):** crop/picker interactions and validation. | No surface-specific native crop/search/project-picker model found. **nothing** |
| S082 P2 | Test in Sandbox sheet/status: project chooser, progress, check results and retry | `R/store/SandboxCheckSheet.svelte`; `R/store/SandboxCheckStatus.svelte`; `R/store/sandbox-check.ts`; `R/store/sandbox-check-sheet.ts` | No sandbox-check sheet spec. **DESIGN-TODO(opus):** selection, running, cancelled and diagnostic result states. | Store-host can run isolated views; complete publish sandbox-check coordinator/model absent. **nothing** |
| S083 P2 | Cloud sign-in/sign-up sheet: provider/email entry and waiting in browser | `R/cloud/SignInSheet.svelte`; `R/cloud/account.ts`; `M/cloud/auth.ts` | Settings accounts placement only. **DESIGN-TODO(opus):** sign-in/sign-up/browser handoff/cancel/error. | `pm-services::entrypoints::AuthLink` queue/parsing only; auth exchange/session model absent. **model only** (entry-point backing) |
| S084 P2 | Email verification code/resend/expiry/errors and publisher handle claim/resume | `R/cloud/SignInSheet.svelte`; `M/cloud/auth.ts`; `M/cloud/service.ts` | No dedicated design. **DESIGN-TODO(opus):** code and handle steps, validation, interrupted signup. | No native code/handle/auth flow model found; migration retains session payloads only. **nothing** |
| S085 P2 | Account button/avatar/menu: identity, Choose Handle, Your Library, Settings, Sign Out and remembered-install preference | `R/cloud/AccountButton.svelte`; `R/cloud/Avatar.svelte`; `R/cloud/account.ts`; `R/settings/SettingsScreen.svelte` | Settings covers account rows, not complete account menu. **DESIGN-TODO(opus):** account menu/avatar/loading and sign-out errors. | No native account state/menu model found; kernel preference persistence is insufficient. **nothing** |
| S086 P2 | Delete Account and custom registry switch native confirmations | `M/cloud/service.ts`; `M/cloud/registry-url.ts`; `R/settings/SettingsScreen.svelte` | No confirmation specs. **DESIGN-TODO(opus):** delete-account and credential-isolating registry-switch confirmations. | Services validates Store origins; no native account deletion/registry-switch flow. **nothing** |
| S087 P2 | Captions cue-list panel, actions popover, cue edit/add/split/delete/search/select and playback-follow | `R/captions/CaptionsPanel.svelte`; `R/captions/install.ts` | No captions-panel spec. **DESIGN-TODO(opus):** full cue-list surface and editing/follow states. | `pm-kernel::commands::captions`; engine `pm-captions`; timeline cue selection partial. No dedicated cue-list view model. **model only** |
| S088 P2 | Captions Inspector: language/style/layout/words/highlight/animation and per-cue text controls | `E/inspector/CaptionsSection.svelte`; `E/inspector/InspectorPanel.svelte` | Inspector names Captions section but lacks its full form/state design. **DESIGN-TODO(opus):** complete captions inspector. | Inspector typed bindings and kernel caption/style commands; engine caption evaluation. **model only** |
| S089 P2 | Caption clips/words on timeline: selection, trim/move/split and cue edit handoff | `E/timeline/caption-track.ts`; `E/timeline/timeline.ts`; `R/captions/install.ts` | Timeline includes captions rows; cue-edit handoff needs **DESIGN-TODO(opus)** with S087. | `pm-panel-timeline::behaviour::CueSelection`; kernel caption commands. **spec+model** (partial) |
| S090 P2 | Generate Captions: choose audio/video, language/model availability, progress/cancel/failure | `R/captions/install.ts`; `R/transcription/client.ts`; `R/transcription/host.ts` | No generation-flow design. **DESIGN-TODO(opus):** source/model/language selection and progress/outcome states. | `pm-captions` and kernel caption commands are result/edit backing; native transcription execution/download host absent. **model only** |
| S091 P2 | Speech model chooser/download sheet: meters, sizes, selected/installed/downloading/cancel/retry | `R/transcription/ModelSheet.svelte`; `R/transcription/ModelMeters.svelte`; `R/transcription/sheet.ts` | Settings only names transcription rows. **DESIGN-TODO(opus):** model chooser/download states. | No native speech-model download/selection model found. **nothing** |
| S092 P2 | Transcription Settings: installed models, default/language/storage/delete confirmation | `R/transcription/TranscriptionSettings.svelte`; `R/transcription/format.ts`; `R/settings/SettingsScreen.svelte` | Settings has section guidance, not full model manager. **DESIGN-TODO(opus):** installed-model management and archive/removal policy. | General settings persistence exists; no surface-specific manager. **nothing** |
| S093 P2 | Import/export captions: SRT/VTT picker, layer/format/range export dialog and sidecars | `R/captions/install.ts`; `M/export-sidecar.ts` | Export covers delivery sidecars; no standalone Export Captions dialog spec. **DESIGN-TODO(opus):** caption import errors and standalone export form. | `pm-captions` parsing/export and kernel commands; standalone dialog/sidecar host incomplete. **model only** |
| S094 P2 | Easing panel: curve editor/coordinates, section/target tools, preset groups/grid, multikey mode and panel preferences | `R/panels/EasingPanel.svelte`; `R/panels/easing/CurveEditor.svelte`; `R/panels/easing/operations.ts`; `R/panels/easing/library.ts` | Timeline graph is a different surface; Browser explicitly does not promise a preset library. **DESIGN-TODO(opus):** complete Easing panel. | Kernel keyframe/channel operations and timeline curve planners are partial; no dedicated preset-library/Easing model found. **model only** |
| S095 P2 | Easing preset/group save/rename/colour/move/duplicate/delete, import file/folder and export library dialogs | `R/panels/EasingPanel.svelte`; `R/panels/easing/preset-files.ts` | No preset-manager/dialog specs; colour popover recipe alone is insufficient. **DESIGN-TODO(opus):** all preset/group dialogs and archive behavior. | No native easing-library management model found; retain serialized user presets. **nothing** |
| S096 P2 | Shader Lab panel: source editor, preset selection, compile shortcut/status/errors and uniform controls | `R/panels/ShaderPanel.svelte`; `R/panels/shader/ShaderEditor.svelte`; `R/panels/shader/CompileStatus.svelte`; `E/inspector/ShaderUniforms.svelte` | Inspector covers generic uniform rows, not source editor/compile UI. **DESIGN-TODO(opus):** Shader Lab panel. | `pm-glsl` compiler and kernel shader content; inspector parameter binding. Source-editor/compile-session model remains. **model only** |
| S097 P2 | Mixer panel: master/nested layer channel strips, faders, pan/mute/solo and live level readout | `R/panels/MixerPanel.svelte`; `R/mixer/MixerStrip.svelte`; `R/mixer/Fader.svelte`; `R/mixer/LevelReadout.svelte` | No mixer spec. **DESIGN-TODO(opus):** strips, gain/pan controls, meter/clip/readout and accessibility states. | Engine audio levels/mix; kernel audio properties and `pm-panel-inspector` bindings. No channel-strip view model found. **model only** |
| S098 P2 | 2.5D/3D scene controls, scene-field editor, 3D inspector/model creation/import and viewer navigation toolbar | `E/layers-3d/LayerInspector.svelte`; `E/layers-3d/SceneField.svelte`; `E/layers-3d/index.ts`; `E/viewer/GizmoToolbar.svelte` | Inspector/Viewer mention 3D, including provisional hidden-while-unshipped wording; source ships these surfaces. **DESIGN-TODO(opus):** reconcile scope and specify full 3D/scene editing. | `pm-kernel::commands::scene3d`; viewer `SceneState` model; engine `pm-geom3d`. Import/manipulation host incomplete. **model only** |
| S099 P2 | Workspaces panel: activate/list/current/save/delete/JSON editor entry and status | `R/panels/WorkspacesPanel.svelte`; `R/legacy/core/workspace.ts` | Shell review specifies Window ▸ Workspace replacement entry, not a complete panel. **DESIGN-TODO(opus):** retained panel or explicit mapped replacement for all actions. | `pm-shell::layout::LayoutStore`; kernel workspace state; full definitions/custom controls not covered by shell layout alone. **model only** |
| S100 P2 | Save Workspace / Save As New sheets and Workspace Definition JSON editor | `R/legacy/core/workspace.ts`; `R/legacy/ui/library.ts` | Shell review specifies Save Workspace sheet. **DESIGN-TODO(opus):** definition JSON editor, invalid JSON/conflicts and Save As variants. | Shell layout validation/store/save-name sheet; kernel workspace records. **spec+model** for save only; JSON editor remains |
| S101 P2 | Home Library workspaces/panels shelf, live panel preview, custom panel editor, imported After Effects tools and delete confirmations | `R/library/LibraryScreen.svelte`; `R/library/after-effects-imports.ts`; `R/legacy/ui/library.ts` | Media browser is not this library. **DESIGN-TODO(opus):** workspace/panel library, preview/editor and import-tool states. | Shell workspace records and compat view host are infrastructure; full shelf/preview/editor model absent. **model only** (workspace backing only) |
| S102 P2 | Generated/custom workspace panels, including built-in Gradient Editor and control previews/readouts/buttons | `R/panels/GeneratedPanel.svelte`; `R/panels/register-generated.ts`; `R/panels/generated/GeneratedControl.svelte`; `R/panels/generated/CurveControl.svelte`; `R/panels/generated/PreviewList.svelte`; `R/panels/generated/Readout.svelte`; `R/panels/generated/GeneratedButton.svelte`; `R/legacy/core/workspace.ts` | Kit review covers atomic widgets, not this composed panel schema/editor. **DESIGN-TODO(opus):** generated-panel host, missing bindings and preview/apply/undo states. | `pm-surface::kit`, v4 operations and compat realm can host controls; no complete generated workspace schema/controller model found. **model only** (control backing only) |
| S103 P2 | Takes panel: named editable snapshots, save/restore/delete/date/empty/status | `R/panels/TakesPanel.svelte`; `R/legacy/core/history.ts` | No Takes spec. **DESIGN-TODO(opus):** snapshot list/save/restore and archive-preserving removal. | Migration retains takes/history/media, but no native Takes controller found. **nothing** |
| S104 P2 | Notes panel: project creative brief, text editing, deferred/flush-on-switch persistence/status | `R/panels/NotesPanel.svelte` | No Notes spec. **DESIGN-TODO(opus):** notes editing and save/failure states. | Project payload preservation is not a note-editing command/controller; tag directly writes notes. **nothing** |
| S105 P2 | Mods panel: user mod/extension rows and enable/management actions | `E/mods/ModsPanel.svelte`; `E/mods/ModRow.svelte`; `E/mods/index.ts` | No Mods-panel spec; do not assume Extensions Settings covers every action. **DESIGN-TODO(opus):** Mods panel or explicit equivalent action mapping. | Kernel extension state and Store-host install models provide partial backing; no Mods panel model. **model only** |
| S106 P2 | Arbitrary v1–3 extension DOM panels, custom inspector sections/widgets, menus/palette/status/toolbar/settings contributions and extension modals | `R/kernel/api.ts`; `R/kernel/registries.ts`; `R/kernel/host.ts`; `R/kernel/sandbox-view.ts`; `E/inspector/ContributedSections.svelte`; `E/inspector/ExtensionLayerParams.svelte` | Shared, Kit/Theme reviews specify host appearance/semantics, not authors' content. **DESIGN-TODO(opus):** unowned host/failure/chrome states; preserve authored content instead of redesigning it. | `pm-runtime-v8` logic/sync services + recorder; `pm-remote-dom`, `pm-compat-host`, `pm-store-host` view/isolation; kernel contribution state. Full open DOM/contribution conformance remains. **model only** |
| S107 P3 | First-run animation/overlay and welcome window; skip/revisit/completion/restore focus | `M/onboarding.ts`; `R/onboarding/welcome.ts`; `R/onboarding/SuitcaseTowers.svelte`; `apps/desktop/src/renderer/onboarding/welcome.html`; `apps/desktop/src/renderer/public/onboarding/animation.html` | No native onboarding design. **DESIGN-TODO(opus):** animation/welcome, skip and accessibility/reduced-motion flow. | `pm-first-launch` preserves onboarding records; it is not a welcome/onboarding controller. **nothing** |
| S108 P3 | Onboarding timeline-style choice and illustrative preview | `R/onboarding/TimelineStep.svelte`; `R/onboarding/welcome.ts` | No spec. **DESIGN-TODO(opus):** timeline choice step. | Stored choice is retained; no native step model found. **nothing** |
| S109 P3 | Onboarding agent choice and transcription model step/progress | `R/onboarding/AgentStep.svelte`; `R/onboarding/TranscriptionStep.svelte`; `R/onboarding/welcome.ts` | Agent/Settings do not specify onboarding steps. **DESIGN-TODO(opus):** agent and speech-model onboarding. | Native agent protocols exist; no onboarding/model-download controller. **nothing** |
| S110 P3 | Bring My Workspace suitcase: creative-app selection/import/progress, launch agent/workspace handoff; also General Settings entry | `R/onboarding/WorkspaceStep.svelte`; `R/onboarding/SuitcaseTowers.svelte`; `R/onboarding/workspace-import.ts`; `M/creative-workspace.ts` | No native workspace-import design. **DESIGN-TODO(opus):** app chooser, access, scanning/import outcomes and handoff. | No native creative-workspace import controller found; migration preserves prior choices. **nothing** |
| S111 P3 | App Check for Updates/menu/status, downloading/ready/error/up-to-date and quit-to-install notice | `M/updates.ts`; `R/shell/app-updates.ts`; `R/shell/Titlebar.svelte` | Services review specifies native update sheet title/notes/three actions; existing native TR/Sparkle route is intentional, not Electron UI imitation. **DESIGN-TODO(opus):** checking/no-update/error/ready states beyond sheet. | `pm-services::updater::{UpdateStatus,FeedMetadata,TransitionGate}` metadata/routing only; signed installer/timers/quit barrier pending. **spec+model** (partial) |
| S112 P3 | What's New/release notes modal and seen marker | `R/shell/WhatsNew.svelte`; `R/shell/whats-new.ts`; `M/whats-new.ts` | Update notes guidance is not a What's New spec. **DESIGN-TODO(opus):** release-note presentation and seen/skip behavior. | Migration preserves marker; no native What's New controller found. **nothing** |
| S113 P3 | About system panel: app/version/copyright, bundled font/icon license notices | `M/index.ts`; `M/menu.ts` | No dedicated About/credits spec in panels/reviews. **DESIGN-TODO(opus):** About metadata/credits/license presentation. | `pm-shell::native` About selector exists; actual metadata/license population remains. **model only** (system action backing) |
| S114 P3 | Agent/task completion OS notifications and notification preference/permission outcomes | `M/agent-notifications.ts`; `R/panels/agent/notification-preferences.ts`; `S/agent-notifications.ts` | No notification/permission spec. **DESIGN-TODO(opus):** notification content/actions and denied authorization states. | Agent result/task models exist; native notification service/authorization integration absent. **model only** (completion backing only) |
| S115 P3 | Performance panel: FPS/frame time, draws/effect passes/program/raster/layer/key counts | `R/panels/PerfPanel.svelte`; `R/runtime/performance-monitor.ts` | No Performance-panel spec. **DESIGN-TODO(opus):** native metric names/units and live/pause/error/AX behavior; GL counters need intentional mapping. | Engine telemetry/status and kernel queries provide partial backing; dedicated panel model absent. **model only** |
| S116 P3 | Development-only Reload / Toggle Developer Tools | `M/menu.ts` | Explicitly outside packaged shipped UI; no native Chromium developer-tools design needed. | Shell intentionally omits Electron-only developer entries. No replacement claimed. **nothing** (development-only exclusion) |

## Native-only and absent surfaces: do not mislabel as shipped

These rows close requested-category ambiguities. They are not additions to the
v1.2.0 surface count or authorization to design new UI.

| Id | Surface / evidence | Existing design | Native backing / status / gap |
| --- | --- | --- | --- |
| N001 | Native beta first-launch gate: wait for Electron, migration progress/pause/resume/low disk | No spec in panels/reviews; [first-launch.md](first-launch.md) is an implementation contract. | `pm-first-launch::{run,Event,Outcome}` and migration worker. **model only**. **DESIGN-TODO(opus):** all migration/waiting/progress/retry states. |
| N002 | Legacy-secret/Keychain consent and fresh consent after interrupted access | No complete native migration consent design. | `pm-first-launch::secrets`, `NeedsSecretConsent`, durable consent/resume. **model only**. **DESIGN-TODO(opus):** consent, denial and interrupted-access presentation; OS Keychain alert remains system-owned. |
| N003 | Migration outcomes: histories/missing slots, original/proxy-only/offline closure, blocked owners, quarantine/conflict/archive review and rollback | No complete review/outcome design. | `pm-first-launch`, `pm-native-migrate`, `pm-kernel` receipts/recovery; three-way conflict/reverse migration remains incomplete. **model only**. **DESIGN-TODO(opus):** outcome/recovery/conflict/rollback surfaces. |
| N004 | Colour scopes: histogram, waveform monitor, vectorscope; LUT/advanced colour inspection | No shipped surface/registration found by tag searches. No panels/reviews scope specification. | `G/pm-color/src/scopes.rs` and `G/pm-color/src/lut.rs` are native worker sources; [pro-media.md](pro-media.md) explicitly leaves integration/export wiring open. **model only** (native addition). **DESIGN-TODO(opus):** scopes/LUT/analysis-stage and HDR presentation. Audio waveform displays in Timeline/Mixer are not colour scopes. |
| N005 | Native Background Tasks utility window, render-cache/Keep Rendered bar, proxy/optimized storage management | Tag has task pill and render queue (S045/S046), not all native pro-media tools. | Pro-media review designs window/popover/render bar; storage management UI lacks complete design. | `pm-promedia::{tasks,cache,media,storage}`. **spec+model** for tasks/render bar; **DESIGN-TODO(opus):** storage/rendition/purge/archive settings and incomplete task states. |
| N006 | J/K/L shuttle, ripple trims/delete, roll/slide, source-monitor three-point marks/insert/overwrite, persistent A/V links | W1 W079–083 and [pro-editing.md](pro-editing.md) identify cutover additions; no complete panels/reviews design for new tools. | `pm-kernel::commands::{pro_transport,pro_edit,pro_source,pro_links}`; engine transport. **model only**. **DESIGN-TODO(opus):** tool chrome, new editing states and source-monitor mark/insert/overwrite interactions. |
| N007 | Browser/OS permission prompts | `M/permissions.ts` automatically grants trusted local-fonts/pointerLock and selected-directory access, denies other requests/restricted paths. It does **not** present a microphone/camera/screen-capture permission dialog. | Native OS permission/result presentation beyond explicit S010/S114/N002 is unspecified. | No invented Chromium permission prompt. **DESIGN-TODO(opus):** native denied-file/screen-sampler/OS authorization outcomes if the native adapter requires them. System picker/Keychain/TCC UI must be verified on macOS. |

## Coverage and reproducible audit

Entry points were followed rather than counting filenames as independent UIs:

1. `git ls-tree -r --name-only v1.2.0 apps/desktop/src` enumerates production
   Svelte, HTML and TypeScript surfaces. Test/fixture/harness files and inactive
   numbered copies (`* 2.*`, `* 3.*`) are not additional shipped surfaces.
2. `git show v1.2.0:apps/desktop/src/renderer/src/panels/install.ts` and
   `register-simple.ts`, built-in extension entry points, captions install,
   agent registration and generated-panel registration supply the panel roots.
   Explicit panel ids covered here are `viewer`, `toolbar`, `timeline`, `inspector`,
   `layer-effects`, `assets`, `fxbrowser`, `mixer`, `workspaces`, `takes`,
   `notes`, `easing`, `shader`, `perf`, `agent`, `captions`, `mods`, plus
   generated/custom ids such as `gradient-controls` and extension-defined ids.
3. `git grep -n -E 'PM\.modal|api\.ui\.modal|PM\.confirm|showMessageBox|showOpenDialog|showSaveDialog|new BrowserWindow' v1.2.0 -- apps/desktop/src`
   finds non-panel dialogs/windows. Dynamic helpers (Easing `prompt`, account,
   model/setup/publish/sandbox sheets, workspace JSON) were followed to callers.
4. `git grep -n -E 'registerPanel|registerSveltePanel|registerSection|openPopoverMenu|PM\.menu' v1.2.0 -- apps/desktop/src`
   cross-checks panel, contributed-inspector and menu entry points.
5. `git grep -n -i -E 'vectorscope|histogram|colour.scope|color.scope|waveform.monitor' v1.2.0 -- apps/desktop/src`
   finds no colour-monitor UI. A broader `scopes` search returns programming
   scopes, not colour scopes. `M/permissions.ts` was read directly to avoid
   mistaking auto-grants/denials for user prompts.
6. Native design files in `design/panels/` and **all** reviews were checked
   against their actual scope, not just their titles. Native backing was
   cross-checked in crate source and [panel model docs](panels/inspector.md),
   [services](services.md), [store host](store-host.md), [shell](shell.md),
   [kernel](kernel.md), [pro media](pro-media.md), [agents](agents.md) and
   [MCP](mcp.md). A proposed model name in a design spec is not proof it exists.

Behavioral cross-checks: [W1](w1-workflows.md) W001–078 (and the post-tag
appendix), [keymap](w1-keymap.md), [C-MATRIX](c-matrix.md) / [CSV](c-matrix.csv),
[D-STATE](d-state.md), [traces](traces.md), [V8](s1-v8.md),
[compat](s5-compat.md), [engine service](engine-service.md) and
[first launch](first-launch.md). Historical WebKit-logic spike text is superseded
by the adopted V8-logic/remote-DOM decision in [P0 decisions](p0-decisions.md).

Small presentation components are included with their owning surface rather than
counted as separate windows. The remaining component files below complete the
cross-check of **146 production Svelte files** (excluding tests, fixtures,
harnesses and inactive numbered copies). A file being accounted for is not proof
that it renders in every configuration; registration/import reachability was
checked separately for the surface roots.

| Owning surface | Additional v1.2.0 component evidence | Native design/backing assessment |
| --- | --- | --- |
| Inspector S031/S034/S036 | `E/inspector/AlignmentStrip.svelte`; `E/inspector/AnimatedRow.svelte`; `E/inspector/Icon.svelte`; `E/inspector/InspectorHeader.svelte`; `E/inspector/SectionHeading.svelte`; `E/inspector/TransformSection.svelte` | Inspector/Effects; inspector model and kernel commands. No separate window. |
| Shared controls S031/S048/S051/S052 | `R/controls/BackTitle.svelte`; `R/controls/Disclosure.svelte`; `R/controls/NumField.svelte`; `R/controls/PointField.svelte`; `R/controls/Row.svelte`; `R/controls/Section.svelte`; `R/controls/Segmented.svelte`; `R/controls/SliderField.svelte`; `R/controls/ToggleField.svelte` | Shared/Kit review; kit recipes and inspector input reducers. Preserve keyboard, units, mixed/disabled/readout and gesture states. |
| Agent S056–S063 | `R/panels/agent/AgentAtmosphere.svelte`; `R/panels/agent/AgentAtmosphereCanvas.svelte`; `R/panels/agent/AgentAtmosphereFrame.svelte`; `R/panels/agent/GhostFieldCanvas.svelte`; `R/panels/agent/GhostFieldFrame.svelte`; `R/panels/agent/PromptGlowCanvas.svelte`; `R/panels/agent/PromptGlowFrame.svelte` | Agent spec/pm-design govern native materials and motion; no browser GPU ripple/material imitation implied. |
| Agent and Repair S057/S064 | `R/panels/agent/TextRow.svelte`; `R/panels/agent/ThoughtRow.svelte`; `R/panels/agent/ToolActivity.svelte`; `R/panels/agent/WorkingTimer.svelte` | Agent conversation model; Repair flow still needs its own specification/model. |
| Shell/panel chrome S014/S026 | `R/shell/Shell.svelte`; `R/panels/Icon.svelte` | Shell/Shared, pm-shell and pm-design. |

Store icons/status/pickers → S075–S082; mixer fader/readout → S097; layout
pool/dock/splitter → S015/S016. Tooltips and drag images are owned by their
launching control; they are not permission to introduce new copy, geometry or
animation. Hidden encoder/capture/helper windows are implementation machinery,
not additional user-facing windows; their progress/failure owners remain listed.

Finite source coverage does not enumerate content a user extension can generate.
S106 preserves that open contract, including v1–3 extension-owned windows of
content, synchronous callbacks/services, remote DOM and recorder canvas, under
N4/N6. Remote DOM is a rendering boundary; it does not move logic into WebKit.

## Decisions and selecting rules

| Decision | Rule that selected it |
| --- | --- |
| Treat data-safety and authority prompts as highest impact; retain even rarely used/zero-count surfaces. | N1 pro-grade first; N4 no user left behind; N6 complete extension contract. |
| Freeze shipped evidence at v1.2.0; distinguish later-main/native additions and explicit owner removals. | N3 authored intent, reference decision in P0; N2 native improvements must be explicit rather than mislabeled parity. |
| Separate buildable design coverage from partial reviews, operation backing and actual UI integration. | N1/N3 correctness; no success-shaped placeholder or unverified parity claim. |
| Record missing designs as neutral `DESIGN-TODO(opus)` entries; invent no layout, copy, visual or interaction. | Explicit lane/design ownership; use existing pm-design tokens/specs only. |
| Preserve Notes, Takes, workspace JSON, custom/generated panels and arbitrary extension content despite absent dedicated native models. | N4 archives every authored payload; N6 preserves v1–3 behavior; generic storage preservation is not a UI replacement. |
| Do not port permanent deletion, GL-only diagnostic names or Chromium permission/pointer-lock quirks as product requirements. | N3 spec/authored intent wins over browser quirks; N4 archive-never-delete; N2 strictly better replacement. |
| Preserve tagged synchronous extension behavior in V8, with WebKit rendering remote DOM only. | Adopted architecture; N4/N6 compatibility and N8 no Node/Chromium in app or main-thread JS wait. |
| Give surfaces stable audit ids and name id-based native model/host boundaries without creating a second writer. | N7 inspectable/drivable surfaces; pm-kernel single writer; N8 worker/async host boundaries. |
| Remove no shipped UI/source/consumer and change only this inventory. | Prune rule and lane ownership; this audit is not authorization to remove a surface. |

## macOS verification and remaining work

This Linux audit runs **no app, Metal, AppKit, GPUI or WebKit UI**. It certifies
source/reference integrity only. No Rust code/crate/manifest/lockfile changed,
so no crate tests, formatter or Clippy run is claimed.

On macOS, verify every row's real entry points and keyboard path, native
windows/sheets/popovers/menus and focus return, save/quit/recovery barriers,
file-picker and provider/Keychain/notification permissions, extension isolation/
restart and remote-DOM text/IME/accessibility. Check the design checklist's light,
dark, Increase Contrast, Reduce Transparency and Reduce Motion states. Test
viewer/export colour/EDR, source/media readiness, physical latency/audio meters,
multiwindow/popout/project ownership, background and windowless work, signed
updater routing and migration/rollback using authorized synthetic fixtures.
Source-derived models and historical review approvals do not close those gates.

Remaining work belongs to the surface owners: Opus specifies every marked gap;
implementation lanes finish missing models/adapters and bind existing models to
the kernel/app; integration owns complete W1/C-MATRIX replay and macOS visual/
behavior review. No cross-lane edits are requested by this documentation change.

## Reconciliation and validation — 2026-11-01

Merged `origin/feat/native-engine` at
`4204a153dbf8af916ef2b92d7ca01858e363516a` with no conflicts (already up to
date). This lane was not based on `feat/na-v4-integrate`.

The main table has **115 shipped surface families plus one explicitly excluded
development-only entry**. Shipped statuses: **44 spec+model, 49 model only,
22 nothing**; partial backing is qualified in each row. **87 shipped rows**
needed an Opus specification/completion at audit time. The seven native-only/absent-category
rows separately record migration, scopes, new pro tools and OS-policy gaps.
These counts describe audit families, not the number of widgets or user
extensions. Spec+model is not a readiness or visual approval score.

Validation uses `python3 -` with eight standard-library `unittest` checks:
frozen tag identity, all tagged source references, local document links, complete
design-panel/review catalog coverage, table ids/columns/status/impact order,
production-Svelte coverage, actual native crate/model definitions, and
cross-references/owned-file scope. **8 passed** after the merge. Coverage:
**303 distinct tagged source files, 146 production Svelte files, 43 local
document links and 25 existing native/engine crate names**. Named model checks
use their definition files (including Inspector's re-exported implementation).

`git diff --cached --check` and
`gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow` pass before
the documentation commit. The repository pre-commit hook stays enabled. Only
`docs/native/surfaces.md` is changed; no lockfile update is required. Rust tests,
Cargo formatting and Clippy are not applicable to this documentation-only lane;
there is no claim of a Linux UI test or a macOS test pass.

## Design coverage update — 2026-11-01 (Opus)

All **33 P0/P1** rows that carried `DESIGN-TODO(opus)` now have build specs in
[design/surfaces/](design/surfaces/README.md) (documents, recovery, library,
media, authority, dialogs, editing). Status moves only where native backing
already existed: 23 rows **model only → spec+model**; S011 and S040 stay
**nothing** (spec exists, no backing); eight rows were already spec+model
(partial) and keep their qualifiers. Shipped totals become **67 spec+model,
26 model only, 22 nothing**; **54** rows (all P2/P3) and the N001–N007
native-only rows still need an Opus specification. A spec is not a visual
approval: each lane's built surface still goes through
[review-checklist.md](design/review-checklist.md).

## Recovery surfaces lane — 2026-11-01

Scope: S001, S013, S053 and S054 only. Implementation lives in
`apps/native/crates/app/src/surfaces/recovery*.rs`; v4 card content lives in
`apps/native/js/v4/surfaces/recovery`. These are executable state/host contracts,
an AppKit alert presenter and a v4 content renderer, **not a claim that the
running application's old recovery/status controllers have switched over**.
All visual output remains provisional until Opus review.

The models cover exact spec copy, ordered per-window alerts, safe Return/Esc
choices, revision/generation fencing, recovered/truncated viewer banners,
service-code notices, all store view failures, Error Log selection/copy/clear,
status singular/plural forms and slowdown actions/inline failures. Native recipes
resolve pm-design tokens across all 16 system appearance/accessibility settings.
The AppKit presenter uses system sheets, destructive button flags and native AX
announcement priorities; app-modal presentation rejects Window ownership.
The serialized service/store seam is checked against the actual backing crate
values. Developer text is never part of panel, banner or table message copy.

Decisions and selecting rules:

| Decision | Selecting rule |
| --- | --- |
| Use the design spec's exact copy, including the editor alert's final period, rather than Electron punctuation | N3 and quirk policy; the authored build spec wins. |
| Omit Discard Edits unless the caller confirms a Recently Deleted archive capability; never map Open Saved Version to Discard | N4: journal archival alone does not prove a user-restorable Recently Deleted entry. |
| Keep kernel recovery tickets and stale prompt/view/issue checks at action boundaries | N3/N7; the kernel actor remains the only writer. |
| Separate accepted responses from the next alert's fallible publication; queue native work without waiting on JS | N1/N7/N8; a transport failure cannot swallow an accepted choice. |
| Keep current app consumers and add the new adapters without deleting them | Prune rule and N4/N6; application attachment is outside this lane's authorized edits. |
| Retain native system geometry and pm-design token recipes; leave a neutral missing-symbol slot | Design ownership; no lane-authored visual fallback. |

Portable verification (from the repository root, after the usual activation):

```sh
mkdir -p work
rustc --edition=2024 -D warnings apps/native/crates/app/src/surfaces/recovery_check.rs -o work/recovery-check
work/recovery-check test
work/recovery-check clippy
rustfmt --edition 2024 --check --config skip_children=true apps/native/crates/app/src/main.rs apps/native/crates/app/src/surfaces/*.rs
bun test --cwd apps/native/js/v4
bun x --cwd apps/native/js/v4 tsc --noEmit --strict --skipLibCheck --target ES2022 --module ESNext --moduleResolution Bundler --jsx react-jsx --jsxImportSource powermove-v4 --types bun surfaces/recovery/index.tsx surfaces/recovery/recovery.test.tsx
cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-kernel -p pm-services -p pm-store-host
```

The runner creates a disposable `work/recovery` manifest using these exact
sources and the production kernel/design/services/store-host crates. It starts
with a copy of `apps/native/Cargo.lock`; Cargo metadata registers the harness,
then every test/Clippy command uses `--locked`. No production dependency was
added and no production lockfile change is needed. No test references spikes,
real profiles or user data. The full app test command cannot run on Linux:
`objc2` requires an Apple target. Full app `cargo fmt --check` also finds an
existing formatting difference in unowned `app/src/smoke.rs`; touched files pass.

Linux results: **37 recovery Rust tests** (33 model/adapter/recipe tests + four
real service/store source-contract tests), **232 full v4 tests** (including six
recovery renderer tests), **273 kernel tests** (one opt-in real-profile smoke
ignored), **75 services tests**, and **88 store-host tests** pass. Recovery strict
Clippy, touched-file rustfmt, v4 typechecking/Prettier, and an isolated strict
Clippy cross-check of the AppKit presenter for `aarch64-apple-darwin` pass.

Integration outside owned paths, still required:

- `app/src/session.rs` and lifecycle/watchdog owners must feed recovery replies
  into `PreparedRecovery`/`opened_surface`, associate prompt IDs with tickets,
  enqueue `AppKitPlatform` presentations, handle completion before `pump`, and
  invalidate queues when windows/documents close. Submit returned `ShellCall`
  through the existing asynchronous kernel handle. Missing/stale windows must
  never be promoted to application-modal ownership.
- Kernel/library must expose a confirmed Recently Deleted recovery archive and
  an open-saved-without-replay policy that retains unreadable recovery. Until
  then Discard is omitted, and Open Saved Version has no fabricated kernel call.
- Store/runtime owners must feed serialized ViewStatus/Failure events into the
  projection, attach v4 content to the existing card, announce accepted changes,
  and route restart requests using fresh kernel generations. Store-host needs an
  explicit early-restart operation: its scheduled-restart API currently rejects
  Restart Now before the timer. Trusted-realm Wait/Stop and Settings enablement
  must remain extension-wide and use the host watchdog's existing threshold.
- Shell/native-host owners must consume `NativeUpdate` with actual NSTableView /
  NSSplitView Error Log and NSPopover slowdown controls, wire Copy/Clear and
  source fix actions to services, format times with the system formatter, and
  subscribe status/slowdown models to kernel/engine facts. Error Log remains a
  single titled frame-autosaved window. The current recipes are not renderers.
- Host must attach viewer-banner/inline-notice placement and announcement
  delivery, resolve the symbol placeholder, and retain existing controllers
  until all replacement consumers and parity gates pass.

`DESIGN-TODO(opus)` follow-ups, with neutral placeholders:

1. Esc behavior for alerts without Cancel or an explicit Esc assignment (editor
   failure, unrecoverable and trusted-realm alerts); no custom binding invented.
2. Content for the invalid/incomplete `WaitingToRestart` snapshot with no Failure;
   the model returns a nonvisual design placeholder.
3. Public UI-kit SF Symbol primitive/slot: the v4 EmptyState preserves the required
   `exclamationmark.triangle` name as metadata without inventing a fallback glyph.
   Host must use `c.warning` and the shared EmptyState's 2 × `m.symbol_panel` size.

macOS acceptance remains: actual sheet ownership/queuing/dismissal and default /
destructive / Esc behavior; app-modal fallback only without an owner window;
real AXGroup labels and announcement priorities; Error Log table/split-view,
clipboard and relative time formatting; anchored popover behavior and status
layout; accessibility setting changes; Opus visual review. A Linux cross-target
check typechecks the actual AppKit presenter, but does not establish runtime or
visual acceptance.
