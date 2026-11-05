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
| S005 P0 | Project Trash, restore and permanent-delete confirmation | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review specifies Move to Trash / Put Back / Recently Deleted only. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S005](design/surfaces/library.md). | `pm-services::library::LibraryAction`; `pm-kernel::{persistence,commands::library}`. N4 requires archives, not shipped irreversible deletion. **spec+model** **Library lane 2026-11-01:** confirmation/critical-alert contracts, first-save-in-Trash/Undo reducer and kernel Trash/Restore worker tested; archive-delete and open-session Trash APIs remain absent; AppKit alert adapter cross-checks, host routing/runtime review pending. |

| S001 P0 | Editor crash/OOM alert: Reload or Close Window; recovered project on relaunch | `M/renderer-recovery.ts`; `R/legacy/core/projects.ts`; `M/index.ts` | Services review specifies recovered-document notice and error vocabulary only. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S001](design/surfaces/recovery.md). | `pm-services::diagnostics::CrashRecord`; `pm-kernel::{journal,session}`. Native crash capture/host presentation pending. **spec+model** |
| S002 P0 | Unsaved tab/window/quit confirmation; wait for active save/import; save failure keeps work open | `R/legacy/app.ts`; `M/save.ts`; `M/windows.ts` | No complete spec. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S002](design/surfaces/documents.md). **Documents lane (2026-11-01):** Document controllers, FIFO system-host boundary and kernel-backed close/quit tests implemented; lifecycle caller integration and Finishing Up sheet metrics remain. See documents lane update below. | `pm-kernel::session` close/save/dirty policies; `pm-shell::session` windows. Dialog/quit wiring pending. **spec+model** |
| S003 P0 | External project changed, uncertain save, recovery copy/backup and save destination failures | `M/project-files.ts`; `M/save.ts`; `R/legacy/app.ts` | Services review has generic errors, not conflict decisions. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S003](design/surfaces/documents.md). **Documents lane (2026-11-01):** Exact notices, changed-file Save preflight and recovery-id resolution gate implemented; file-watcher/reload and notice delivery integration remain. See documents lane update below. | `pm-kernel::{session,project_io,locks}` preserves conflict copies and typed save outcomes. **spec+model** |
| S004 P0 | Invalid/locked/read-only/cloud-volume project open errors and copy policy | `M/save.ts`; `M/project-files.ts`; `R/legacy/app.ts` | Services review gives failed-open copy and Show in Finder; no complete lock/copy flow. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S004](design/surfaces/documents.md). **Documents lane (2026-11-01):** All eight CopyReason lines, service-code copy, retained failed-open path and native alert adapter implemented/tested; entrypoint callers remain to connect. See documents lane update below. | `pm-kernel::{locks,session}`; `pm-services::entrypoints::{EntryQueue,FileOpenFailure}`. **spec+model** |
| S005 P0 | Project Trash, restore and permanent-delete confirmation | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review specifies Move to Trash / Put Back / Recently Deleted only. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S005](design/surfaces/library.md). | `pm-services::library::LibraryAction`; `pm-kernel::{persistence,commands::library}`. N4 requires archives, not shipped irreversible deletion. **spec+model** |
| S006 P0 | Offline/changed source badges, locate/relink/replace and missing-original export warning | `R/panels/AssetsPanel.svelte`; `R/legacy/core/media.ts`; `R/legacy/gl/raster.ts`; `R/legacy/core/project-file.ts` | Media and Export cover entry points/warnings. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S006](design/surfaces/media.md). | `pm-panel-media::{model,edit}`; `pm-promedia::media::{Availability,MediaCatalog}`; `pm-engine-service` media mapping. **spec+model** (partial) |
| S007 P0 | Cloud-media download consent, per-file progress/cancel/retry and automatic-download preference | `M/cloud-media.ts`; `R/legacy/core/cloud-media.ts`; `R/settings/SettingsScreen.svelte` | Settings specifies the preference; Media covers availability. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S007](design/surfaces/media.md). | `pm-panel-media` host effects; `pm-services::media::ImportQueue` is partial queue backing, not cloud hydration. Production cloud adapter absent. **spec+model** |

| S005 P0 | Project Trash, restore and permanent-delete confirmation | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review specifies Move to Trash / Put Back / Recently Deleted only. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S005](design/surfaces/library.md). | `pm-services::library::LibraryAction`; `pm-kernel::{persistence,commands::library}`. N4 requires archives, not shipped irreversible deletion. **spec+model** |
| S006 P0 | Offline/changed source badges, locate/relink/replace and missing-original export warning | `R/panels/AssetsPanel.svelte`; `R/legacy/core/media.ts`; `R/legacy/gl/raster.ts`; `R/legacy/core/project-file.ts` | Media and Export cover entry points/warnings. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S006](design/surfaces/media.md). | `pm-panel-media::{model,edit}`; `pm-promedia::media::{Availability,MediaCatalog}`; `pm-engine-service` media mapping. **spec+model** (partial) **Media lane, 2026-11-01:** Rust relink metadata/confirmation/FIFO flows, matching-folder filtering and proxy-only/export-warning descriptions in `app/src/surfaces/media*.rs`; guarded kernel replacement preserves trim, timing and unknown fields. AppKit alerts/picker compile; live panel/export bindings remain. |
| S007 P0 | Cloud-media download consent, per-file progress/cancel/retry and automatic-download preference | `M/cloud-media.ts`; `R/legacy/core/cloud-media.ts`; `R/settings/SettingsScreen.svelte` | Settings specifies the preference; Media covers availability. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S007](design/surfaces/media.md). | `pm-panel-media` host effects; `pm-services::media::ImportQueue` is partial queue backing, not cloud hydration. Production cloud adapter absent. **spec+model** **Media lane, 2026-11-01:** Rust consent/preference intent, per-file measured progress, cancel/retry and once-per-open notice gate; AppKit suppression checkbox. Cloud provider hydration and Settings/session routing remain. |
| S008 P0 | Store Full Access trust alert and revoke/retrust entry points | `M/cloud/trust.ts`; `M/cloud/store-ipc.ts`; `R/store/StoreScreen.svelte` | Store-host review specifies alert, exact trust copy, Cancel default, identity fallback and stale handling. | `pm-store-host::permission::{PermissionRequest,Admission}` and stored decisions; host alert wiring pending. **spec+model** |
| S009 P0 | Isolated extension permission disclosure/admission and revocation | `R/store/StoreScreen.svelte`; `M/cloud/permission-scan.ts`; `M/cloud/store-ipc.ts`; `R/legacy/ui/extension-settings.ts` | Store-host review specifies batched permission sheet, display identity and Settings permission switches. This is native replacement guidance; tag discloses permissions on Store detail rather than presenting the same new sheet. | `pm-store-host::permission` request/respond-all/revoke contracts. **spec+model** |
| S010 P0 | Agent per-project computer-access and standing Full Access confirmations | `M/codex/consent.ts`; `R/panels/agent/AccessPicker.svelte`; `M/agent-approvals.ts` | Agent specifies access picker/inline approvals, not these full native consent dialogs. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S010](design/surfaces/authority.md). | `pm-agents::approvals`; `pm-panel-agentpanel` access commands/effects. `app::surfaces::authority` now supplies exact-copy alert state, safe defaults, request/generation guards, FIFO window queues, bounded worker publication, session-only standing-access state and an asynchronous AppKit alert adapter. **spec+model** (48 shared authority tests; macOS target checked; app dispatch and concrete agent-worker binding remain). |
| S011 P0 | Extension variable setup, secret input/question, missing required values | `R/vars/SetupSheet.svelte`; `R/vars/setup.ts`; `M/env/ipc.ts` | Settings links to setup; Agent covers secret question cards only. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S011](design/surfaces/authority.md). | `app::surfaces::authority::values` now supplies native setup/secret row descriptors, live required-value validation, write/re-entry/error/cancel states, immediate Remove and explicit Reveal protocols, and `EmptyState` copy. **spec+model** (headless surface only). Persistent env/Keychain service, native setup rendering, v4 empty-state mounting and app dispatch remain. Missing `sheet_inset` / `sheet_value_column` tokens are `DESIGN-TODO(opus)`; no geometry was invented. |
| S012 P0 | Uninstall extension / also remove its entered values; local panel/workspace deletion confirmations | `M/env/ipc.ts`; `R/legacy/ui/extension-settings.ts`; `R/library/LibraryScreen.svelte`; `R/panels/WorkspacesPanel.svelte` | Store-host/Settings cover revoke/management, not complete removal choices. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S012](design/surfaces/authority.md). | `pm-store-host::install::{ArchivedRelease,ArchiveReason}`; `pm-shell::layout::LayoutStore`. `app::surfaces::authority` now supplies exact-copy uninstall/workspace/local-project alerts, concealed-copy hosting, no-values bypass, and id/revision-scoped archive commands. **spec+model** (surface/adapter source). Concrete Store/value worker binding and durable LayoutStore archival + Undo Delete Workspace remain; `LayoutStore::delete_named` alone is insufficient. |
| S013 P0 | Extension panel crashed/hung/unavailable: recovery, reload, disable | `R/kernel/sandbox-host.ts`; `R/kernel/sandbox-view.ts`; `R/legacy/ui/extension-settings.ts` | Shared error states and Store-host lifecycle review are not a full recovery design. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S013](design/surfaces/recovery.md). | `pm-store-host::view::{ViewStatus,RecoveryPolicy,Action}`; runtime/compat lifecycle models. Full application watchdog wiring pending. **spec+model** |

| S010 P0 | Agent per-project computer-access and standing Full Access confirmations | `M/codex/consent.ts`; `R/panels/agent/AccessPicker.svelte`; `M/agent-approvals.ts` | Agent specifies access picker/inline approvals, not these full native consent dialogs. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S010](design/surfaces/authority.md). | `pm-agents::approvals`; `pm-panel-agentpanel` access commands/effects. OS consent host pending. **spec+model** |
| S011 P0 | Extension variable setup, secret input/question, missing required values | `R/vars/SetupSheet.svelte`; `R/vars/setup.ts`; `M/env/ipc.ts` | Settings links to setup; Agent covers secret question cards only. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S011](design/surfaces/authority.md). | Migration retains env payloads; no complete native variable setup/service model found. **nothing** |
| S012 P0 | Uninstall extension / also remove its entered values; local panel/workspace deletion confirmations | `M/env/ipc.ts`; `R/legacy/ui/extension-settings.ts`; `R/library/LibraryScreen.svelte`; `R/panels/WorkspacesPanel.svelte` | Store-host/Settings cover revoke/management, not complete removal choices. **Specified (Opus, 2026-11-01):** [surfaces/authority.md §S012](design/surfaces/authority.md). | `pm-store-host::install::{ArchivedRelease,ArchiveReason}`; `pm-shell::layout::LayoutStore` supports saved-workspace changes; variable archive flow incomplete. **spec+model** |
| S013 P0 | Extension panel crashed/hung/unavailable: recovery, reload, disable | `R/kernel/sandbox-host.ts`; `R/kernel/sandbox-view.ts`; `R/legacy/ui/extension-settings.ts` | Shared error states and Store-host lifecycle review are not a full recovery design. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S013](design/surfaces/recovery.md). **Recovery lane (2026-11-01):** all lifecycle copy/AX states, generation-fenced restart requests and v4 EmptyState content tested; serialized source contract checked against real store-host enums. Watchdog/application attachment and symbol slot pending. | `pm-store-host::view::{ViewStatus,RecoveryPolicy,Action}`; runtime/compat lifecycle models. Full application watchdog wiring pending. **spec+model** |
| S014 P1 | Main editor windows, title/dirty indicator, project tabs, tab menu, move tab to another/new window, restore | `M/windows.ts`; `M/index.ts`; `R/shell/Titlebar.svelte`; `R/legacy/app.ts` | Shell review covers native window/tab/title behavior. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S014](design/surfaces/documents.md). | `pm-shell::{session,native}`; `pm-kernel::session`; native window/kernel ownership integration remains. **spec+model** (partial) |
| S015 P1 | Docked panels, headers, collapse/hide, splitters, drag/drop previews, add/close/move panel menu | `R/layout/Dock.svelte`; `R/layout/DockLayout.svelte`; `R/layout/Splitter.svelte`; `R/legacy/ui/layout.ts` | Shared and Shell review specify native docking, context menus, previews and header state. | `pm-shell::layout::{Layout,DropTarget,DropPreview}`; app must connect arbitrary extension panel ids. **spec+model** |
| S016 P1 | Popped-out panel window, focus/close/return and shared project ownership | `M/panel-popout.ts`; `R/layout/PanelPool.svelte`; `R/legacy/ui/layout.ts` | Shared mentions Pop Out; no full popout lifecycle spec. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S016](design/surfaces/documents.md). | `pm-shell` window/layout models and engine independent viewer support are partial; arbitrary panel detach/reattach host remains. **spec+model** |
| S017 P1 | Home/project library: recent/all projects, search/sort, grid/list, project cards and empty state | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review defines row facts/vocabulary; no complete Project Library window design. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S017](design/surfaces/library.md). | `pm-services::library::{ProjectEntry,ProjectSummary,LibraryService}`; `pm-kernel` library. **spec+model** **Library lane 2026-11-01:** ID-driven controller, exact empty/loading/unavailable copy, search/sort, virtual viewport, keyboard/AX contract and headless host tested; native window/UI-kit renderer, autosaved frame and shell entry-point wiring pending. |
| S018 P1 | New Project dialog, defaults and creation errors | `R/legacy/app.ts`; `R/legacy/ui/project-settings.ts` | No dedicated spec. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S018](design/surfaces/library.md). | `pm-kernel::{session,commands::library}` creates projects; dialog state/host missing. **spec+model** **Library lane 2026-11-01:** live form validation, inherited exact format/timecode, uniqued name, stale-owner and retry state wired to kernel creation; AppKit sheet/colour-popover host and missing design tokens pending. |
| S019 P1 | Rename/duplicate project, project-card/context actions | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review has library vocabulary/data only. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S019](design/surfaces/library.md). | `pm-services::library::LibraryAction`; `pm-kernel::session`. **spec+model** **Library lane 2026-11-01:** in-place rename/duplicate reducer and kernel open-session rename/duplicate tested; unopened-slot rename API and native editable cell/title-menu host pending. |
| S020 P1 | Open/Save/Save As `.pmv` system picker; import/export file and directory pickers | `M/save.ts`; `M/render-encoder.ts`; `R/legacy/app.ts` | Export specifies export destination picker; no complete project open/save picker policy. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S020](design/surfaces/documents.md). | `pm-kernel::{session,project_io}`; `pm-services::entrypoints`; platform picker adapter required. **spec+model** |
| S021 P1 | New Composition / Composition Settings dialog; size/FPS/duration/background | `R/legacy/ui/compositions.ts`; `R/legacy/ui/project-settings.ts`; `E/inspector/CompositionSection.svelte` | Inspector and Settings cover inline fields; no complete standalone dialog spec. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S021](design/surfaces/dialogs.md). **Dialogs lane 2026-11-01:** exact-copy/validation/keyboard descriptors and guarded kernel settings/create adapters tested; AppKit form renderer awaits the missing sheet metric tokens and shell binding. | `pm-kernel::commands::comps`; `pm-panel-inspector` composition bindings. **spec+model** |
| S022 P1 | Pre-compose dialog and composition-delete confirmation | `R/legacy/ui/compositions.ts`; `R/panels/AssetsPanel.svelte` | Timeline/Media expose actions, not complete dialogs. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S022](design/surfaces/dialogs.md). **Dialogs lane 2026-11-01:** pre-compose states and delete NSAlert adapter added; kernel `CompCommand::Precompose` still returns `NotImplemented` (including duration/open-after integration). | `pm-kernel::commands::{comps,groups}`; `pm-panel-media::edit`. **spec+model** |

| S013 P0 | Extension panel crashed/hung/unavailable: recovery, reload, disable | `R/kernel/sandbox-host.ts`; `R/kernel/sandbox-view.ts`; `R/legacy/ui/extension-settings.ts` | Shared error states and Store-host lifecycle review are not a full recovery design. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S013](design/surfaces/recovery.md). | `pm-store-host::view::{ViewStatus,RecoveryPolicy,Action}`; runtime/compat lifecycle models. Full application watchdog wiring pending. **spec+model** |
| S014 P1 | Main editor windows, title/dirty indicator, project tabs, tab menu, move tab to another/new window, restore | `M/windows.ts`; `M/index.ts`; `R/shell/Titlebar.svelte`; `R/legacy/app.ts` | Shell review covers native window/tab/title behavior. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S014](design/surfaces/documents.md). **Documents lane (2026-11-01):** Deferred transfer/snap-back and restore-error contracts implemented/tested; shell ownership callbacks remain to connect. See documents lane update below. | `pm-shell::{session,native}`; `pm-kernel::session`; native window/kernel ownership integration remains. **spec+model** (partial) |
| S015 P1 | Docked panels, headers, collapse/hide, splitters, drag/drop previews, add/close/move panel menu | `R/layout/Dock.svelte`; `R/layout/DockLayout.svelte`; `R/layout/Splitter.svelte`; `R/legacy/ui/layout.ts` | Shared and Shell review specify native docking, context menus, previews and header state. | `pm-shell::layout::{Layout,DropTarget,DropPreview}`; app must connect arbitrary extension panel ids. **spec+model** |
| S016 P1 | Popped-out panel window, focus/close/return and shared project ownership | `M/panel-popout.ts`; `R/layout/PanelPool.svelte`; `R/legacy/ui/layout.ts` | Shared mentions Pop Out; no full popout lifecycle spec. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S016](design/surfaces/documents.md). **Documents lane (2026-11-01):** Popout lifecycle, system NSWindow constructor and v4 header implemented/tested; existing content/delegates/persistence remain to connect. See documents lane update below. | `pm-shell` window/layout models and engine independent viewer support are partial; arbitrary panel detach/reattach host remains. **spec+model** |
| S017 P1 | Home/project library: recent/all projects, search/sort, grid/list, project cards and empty state | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review defines row facts/vocabulary; no complete Project Library window design. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S017](design/surfaces/library.md). | `pm-services::library::{ProjectEntry,ProjectSummary,LibraryService}`; `pm-kernel` library. **spec+model** |
| S018 P1 | New Project dialog, defaults and creation errors | `R/legacy/app.ts`; `R/legacy/ui/project-settings.ts` | No dedicated spec. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S018](design/surfaces/library.md). | `pm-kernel::{session,commands::library}` creates projects; dialog state/host missing. **spec+model** |
| S019 P1 | Rename/duplicate project, project-card/context actions | `R/legacy/ui/projects.ts`; `R/legacy/core/projects.ts` | Services review has library vocabulary/data only. **Specified (Opus, 2026-11-01):** [surfaces/library.md §S019](design/surfaces/library.md). | `pm-services::library::LibraryAction`; `pm-kernel::session`. **spec+model** |
| S020 P1 | Open/Save/Save As `.pmv` system picker; import/export file and directory pickers | `M/save.ts`; `M/render-encoder.ts`; `R/legacy/app.ts` | Export specifies export destination picker; no complete project open/save picker policy. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S020](design/surfaces/documents.md). **Documents lane (2026-11-01):** Typed picker descriptions and AppKit NSOpenPanel/NSSavePanel adapter implemented; macOS filter/overwrite/extension behavior and caller integration remain. See documents lane update below. | `pm-kernel::{session,project_io}`; `pm-services::entrypoints`; platform picker adapter required. **spec+model** |
| S021 P1 | New Composition / Composition Settings dialog; size/FPS/duration/background | `R/legacy/ui/compositions.ts`; `R/legacy/ui/project-settings.ts`; `E/inspector/CompositionSection.svelte` | Inspector and Settings cover inline fields; no complete standalone dialog spec. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S021](design/surfaces/dialogs.md). | `pm-kernel::commands::comps`; `pm-panel-inspector` composition bindings. **spec+model** |
| S022 P1 | Pre-compose dialog and composition-delete confirmation | `R/legacy/ui/compositions.ts`; `R/panels/AssetsPanel.svelte` | Timeline/Media expose actions, not complete dialogs. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S022](design/surfaces/dialogs.md). | `pm-kernel::commands::{comps,groups}`; `pm-panel-media::edit`. **spec+model** |
| S023 P1 | Viewer canvas/chrome: zoom/fit/custom zoom, quality, safe areas, controls, focus/fullscreen, return-to-composition and offscreen recovery | `E/viewer/viewer.ts`; `E/viewer/index.ts`; `R/legacy/core/preview-resolution.ts` | Viewer and Viewer review specify chrome, quality and recovery banners. | `pm-panel-viewerchrome::{Navigation,ViewModel,Controller}`; `pm-engine-service` presentation. Physical EDR/input/performance gates open. **spec+model** |
| S024 P1 | Source-preview viewer: image/audio/video, scrub/play/time/range, close and drag source | `E/viewer/source-preview.ts`; `R/panels/AssetsPanel.svelte` | Viewer and Media specify source mode; decoder-error states were missing. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S024](design/surfaces/media.md). | `pm-panel-media::preview`; `pm-panel-viewerchrome::source`; engine independent source viewers. Host/media leases pending. **spec+model** (partial) **Media lane, 2026-11-01:** Rust source-failure states plus v4 `surfaces/media/SourceError`, exact copy/actions and wire projection; keep-last-frame/video flags. Engine/source lease and viewer-banner installation remain. |
| S025 P1 | On-canvas selection, transform/anchor/rotation, shape/pen/mask editing, text caret/selection/paragraph box and gizmos | `E/viewer/viewer.ts`; `E/viewer/path-editing.ts`; `E/viewer/text-editor.ts`; `E/viewer/text-geometry.ts` | Viewer covers gizmo appearance; Inspector covers properties. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S025](design/surfaces/editing.md). | Viewer model supplies selection/overlay plans; `pm-kernel::commands::{transform,masks,text}` and engine geometry. Full pointer/text/path editing host remains. **spec+model** (partial) |

| S024 P1 | Source-preview viewer: image/audio/video, scrub/play/time/range, close and drag source | `E/viewer/source-preview.ts`; `R/panels/AssetsPanel.svelte` | Viewer and Media specify source mode; decoder-error states were missing. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S024](design/surfaces/media.md). | `pm-panel-media::preview`; `pm-panel-viewerchrome::source`; engine independent source viewers. Host/media leases pending. **spec+model** (partial) |
| S025 P1 | On-canvas selection, transform/anchor/rotation, shape/pen/mask editing, text caret/selection/paragraph box and gizmos | `E/viewer/viewer.ts`; `E/viewer/path-editing.ts`; `E/viewer/text-editor.ts`; `E/viewer/text-geometry.ts` | Viewer covers gizmo appearance; Inspector covers properties. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S025](design/surfaces/editing.md). **Editing lane (2026-11-01):** token/AX path and text-overlay plans, modifier reducer and kernel path/text adapter in `app/src/surfaces/editing*.rs`; headless-tested; renderer/input attachment pending. | Viewer model supplies selection/overlay plans; `pm-kernel::commands::{transform,masks,text}` and engine geometry. Full pointer/text/path editing host remains. **spec+model** (partial) |
| S026 P1 | Main tool strip / registered headless `toolbar` (Tools) panel and creation menu; selection/hand/zoom/rotate/anchor, drawing/text/shape tools | `E/toolbar/index.ts`; `R/shell/ToolbarMount.svelte` | Shell review and Viewer specify toolbar/tool behavior; Shared owns tokens. | `pm-shell` toolbar/command tables; viewer `input` and tool state. **spec+model** |
| S027 P1 | Timeline panel: composition tabs, transport/time, layer switches/parenting, clips, work area, ruler, snapping, scroll/zoom, menus | `E/timeline/timeline.ts`; `E/timeline/comp-tabs.ts`; `E/timeline/track-view.ts`; `E/timeline/track-layout.ts` | Timeline supplies surface and states. | `pm-panel-timeline` snapshot/view/input/behavior models; `pm-kernel::commands::{track,timing,markers}`; engine transport. Native painter/input binding pending. **spec+model** |
| S028 P1 | Graph/dope sheet/combined views, channel rows, key diamonds, curve handles, interpolation and graph menus | `E/timeline/graph-controls.ts`; `E/timeline/graph-presentation.ts`; `E/timeline/timeline.ts` | Timeline specifies graph and local selection/focus/drag behavior. | `pm-panel-timeline::{TimelineModel,GraphDisplay}` and keyframe commands. **spec+model** |
| S029 P1 | Keyframe Value, Keyframe Velocity and Scale Selected Keyframes dialogs | `E/timeline/timeline.ts`; `E/timeline/graph-controls.ts` | Timeline names interactions but lacks complete dialog composition/copy. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S029](design/surfaces/dialogs.md). **Dialogs lane 2026-11-01:** exact-copy mixed-value/velocity/scale descriptors and kernel plans tested, including lock exclusion and refusal of silent scale clamping; native form binding pending. | `pm-panel-timeline::behaviour` planners; `pm-kernel::commands::keyframes`. **spec+model** |
| S030 P1 | Timeline marker/work-area menus and add/remove marker affordance | `E/timeline/markers.ts`; `E/timeline/timeline.ts` | Timeline ruler/work-area/menu guidance. | `pm-panel-timeline`; `pm-kernel::commands::{markers,comps}`. **spec+model** |
| S031 P1 | Properties/Inspector: composition/no selection, multi-selection/mixed values, content/transform/structured/mask/time/layer sections | `E/inspector/InspectorPanel.svelte`; `E/inspector/InspectorSurface.svelte`; `E/inspector/ContentSection.svelte`; `E/inspector/StructuredSection.svelte`; `E/inspector/MasksSection.svelte`; `E/inspector/RetimingSection.svelte`; `E/inspector/LayerOptions.svelte` | Inspector specifies section order, states, fields and local controls. | `pm-panel-inspector::{catalog,content,relations,edit,view}`; kernel property/mask/timing commands. **spec+model** |
| S032 P1 | Effect Controls panel: add/select/reorder/enable/remove, clipboard, effect parameters, missing definitions | `E/inspector/EffectsPanel.svelte`; `E/inspector/EffectsSection.svelte`; `E/inspector/ParameterList.svelte`; `E/inspector/effect-clipboard.ts` | Effects specifies stack, parameters, states and menus. | `pm-panel-inspector::effects` and kernel effect commands; runtime registry must supply definitions. **spec+model** |
| S033 P1 | Effects browser: search, categories, previews, drag/add effects and transitions | `R/panels/FxBrowserPanel.svelte`; `R/panels/fx-browser.svelte.ts`; `E/effects-basic/index.ts`; `E/transitions-basic/index.ts` | Browser specifies this surface, titled Effects per owner decision. | Kernel registered definitions and `pm-panel-inspector::effects` apply planner exist; dedicated search/category/preview browser model not found. **spec+model** (operation backing only; browser model remains) |
| S034 P1 | Font family/face popover, font preview, variable axes, text alignment and text animators | `R/controls/FontField.svelte`; `E/inspector/TypeSettings.svelte`; `E/inspector/TypeAxis.svelte`; `E/inspector/TextAnimators.svelte`; `E/inspector/TextAlignment.svelte` | Inspector plus Services review specify fonts/face menu and type rows. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S034](design/surfaces/editing.md). | `pm-services::fonts::{FontCatalog,Face}`; `pm-panel-inspector::content`; kernel text commands. **spec+model** (partial) |
| S035 P1 | Colour/fill popover: None/solid/linear/radial; sRGB/Display P3; OKLCH/HSL/RGB/OKLab/P3 notation and copy; hue mapping, previous/new swatches, gamut-clipping warning, alpha, gradient stops/midpoint and screen eyedropper | `R/controls/ColorField.svelte`; `R/controls/FillField.svelte`; `R/controls/RampField.svelte`; `R/controls/color-space.ts` | Inspector §7 and Kit review specify a simpler popover/ramp. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S035](design/surfaces/editing.md). | `pm-panel-inspector` typed binding/edit/input; `pm-surface::kit` colour-well recipe; engine colour math. No complete colour-picker/recent-colour or NSColorSampler adapter yet. **spec+model** (partial) |
| S036 P1 | Property context menu and expression editor modal: presets/edit/remove/reset/reveal/keyframe | `E/inspector/ChannelRow.svelte`; `E/inspector/PropertyStopwatch.svelte`; `R/legacy/core/expression.ts` | Inspector describes context actions, not full expression-editor modal. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S036](design/surfaces/dialogs.md). **Dialogs lane 2026-11-01:** debounced worker jobs, real engine preview, UTF-16 insert, exact-copy/keyboard/AX contracts and guarded expression edits tested; native NSTextView host and unlocated diagnostic design pending. | `pm-panel-inspector::edit` property actions; kernel channel/expression commands; engine expression evaluation. **spec+model** |

| S034 P1 | Font family/face popover, font preview, variable axes, text alignment and text animators | `R/controls/FontField.svelte`; `E/inspector/TypeSettings.svelte`; `E/inspector/TypeAxis.svelte`; `E/inspector/TextAnimators.svelte`; `E/inspector/TextAlignment.svelte` | Inspector plus Services review specify fonts/face menu and type rows. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S034](design/surfaces/editing.md). **Editing lane (2026-11-01):** v4 UI-kit animator section, native-menu/action queue and kernel snapshot projection; headless/wire-tested; unimplemented kernel animator commands and app registration remain. | `pm-services::fonts::{FontCatalog,Face}`; `pm-panel-inspector::content`; kernel text commands. **spec+model** (partial) |
| S035 P1 | Colour/fill popover: None/solid/linear/radial; sRGB/Display P3; OKLCH/HSL/RGB/OKLab/P3 notation and copy; hue mapping, previous/new swatches, gamut-clipping warning, alpha, gradient stops/midpoint and screen eyedropper | `R/controls/ColorField.svelte`; `R/controls/FillField.svelte`; `R/controls/RampField.svelte`; `R/controls/color-space.ts` | Inspector §7 and Kit review specify a simpler popover/ramp. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S035](design/surfaces/editing.md). **Editing lane (2026-11-01):** complete settings/copy/notation plan, ramp reducer, v4 colour worker and AppKit popover/sampler boundary; headless-tested; picker content/host binding and Mac review remain. | `pm-panel-inspector` typed binding/edit/input; `pm-surface::kit` colour-well recipe; engine colour math. No complete colour-picker/recent-colour or NSColorSampler adapter yet. **spec+model** (partial) |
| S036 P1 | Property context menu and expression editor modal: presets/edit/remove/reset/reveal/keyframe | `E/inspector/ChannelRow.svelte`; `E/inspector/PropertyStopwatch.svelte`; `R/legacy/core/expression.ts` | Inspector describes context actions, not full expression-editor modal. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S036](design/surfaces/dialogs.md). | `pm-panel-inspector::edit` property actions; kernel channel/expression commands; engine expression evaluation. **spec+model** |
| S037 P1 | Project/Media panel: composition/media/bin lists, selection, rename, drag, new/import/context menus, missing media | `R/panels/AssetsPanel.svelte`; `R/panels/register-simple.ts` | Media specifies rows/grid, bins, selection, empty/loading/offline states. | `pm-panel-media::{catalog,model,input,edit}`. Tag enumeration/collation and actual host operations still need integration. **spec+model** |
| S038 P1 | Import progress, conversions/proxies, drag/file/folder/paste imports, cancellation and failures | `R/legacy/core/import-progress.ts`; `R/legacy/core/folder-import.ts`; `R/legacy/core/media-work.ts`; `R/legacy/core/media-paste.ts`; `M/media-proxy.ts` | Media covers entry/row state; Pro-media review specifies Background Tasks. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S038](design/surfaces/media.md). | `pm-services::media::ImportQueue`; `pm-promedia::{media,tasks}`; engine probe/codec hooks. Full original-byte admission remains. **spec+model** (partial) **Media lane, 2026-11-01:** Rust per-action import rows, typed failures, failed-only retry and retained-original proxy failure; v4 task body and guarded one-step kernel batch admission. Production import worker/streamed admission grouping and Background Tasks installation remain. |
| S039 P1 | Import Image Sequence dialog: ordered files, FPS/range/name and replacement workflow | `R/legacy/core/image-sequence.ts`; `M/media-proxy.ts`; `R/panels/AssetsPanel.svelte` | Media lists the action, not complete form. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S039](design/surfaces/media.md). | Media import/edit effects and engine sequence handling are partial; no complete sequence dialog model. **spec+model** **Media lane, 2026-11-01:** Rust sequence form/validation/keyboard/stale-target/progress model. Native sequence sheet rendering remains blocked by unresolved sheet tokens; sequence-worker dispatch remains. |
| S040 P1 | Paste Figma selection: fallback warning/confirmation, image fallback and unsupported layers | `R/legacy/core/figma/paste.ts`; `R/legacy/core/figma/convert.ts`; `M/clipboard.ts` | No dedicated design. **Specified (Opus, 2026-11-01):** [surfaces/media.md §S040](design/surfaces/media.md). | Native clipboard/engine geometry alone do not implement tagged Figma conversion/fallback flow. **spec+model** (fallback state only; converter absent) **Media lane, 2026-11-01:** Rust Figma fallback/acknowledgement/banner flow plus queued AppKit alert support; native clipboard Figma conversion and installation remain. |
| S041 P1 | Export settings sheet: format/codec, dimensions/FPS/range/audio/alpha/colour/captions options and preview | `R/legacy/core/exporter.ts`; `R/legacy/core/export-preview.ts`; `R/legacy/ui/project-settings.ts` | Export specifies settings/validation. | `pm-engine-service::jobs`; `pm-export`; kernel delivery state. Full sheet reducer and all option bindings remain. **spec+model** (partial) |
| S042 P1 | Export destination picker and progress/cancel/success/error, reveal output, resume editing | `R/legacy/core/exporter.ts`; `M/render-encoder.ts`; `M/save.ts`; `M/export-sidecar.ts` | Export §3 and Pro-media review specify destination/progress/completion. | Engine export jobs and `pm-promedia::tasks`; native encoder/destination/sidecar binding required. **spec+model** |
| S043 P1 | Export preset save/select/manage and queued export action | `R/legacy/core/exporter.ts`; `R/legacy/core/render-queue.ts` | Export covers preset controls; detailed preset naming/management is incomplete. **Specified (Opus, 2026-11-01):** [surfaces/dialogs.md §S043](design/surfaces/dialogs.md). **Dialogs lane 2026-11-01:** nested NSAlert adapters and kernel `renderPresets` save/select/rename/delete planning added; complete entries must reach a durable archive adapter before removal/replacement. Production archive service and Export binding pending. | Kernel preserves `renderPresets`/`exportOpts`; export jobs have options, but no preset-manager model found. **spec+model** (export operation backing) |
| S044 P1 | Shipped Code (web animation) export, validation/warnings and standalone archive/player delivery | `R/legacy/core/exporter.ts`; `R/player/export-web.ts`; `R/player/player.ts` | Export/Shared explicitly record the owner decision to remove this option; no new spec requested. Preserve existing authored projects/assets and exported files. | No native web-animation delivery replacement claimed. **nothing** (explicit owner removal; W072) |
| S045 P1 | Render Queue modal: composition snapshots, pending/rendering/done/error, render pending, retry/remove | `R/legacy/core/render-queue.ts`; `R/legacy/core/exporter.ts` | Pro-media review explicitly replaces queue modal with Background Tasks and maps Retry/Remove. | `pm-promedia::tasks::BackgroundTasks`; migration preserves queue input. Adapter must retain queued composition/options/snapshot/output identity. **spec+model** (migration/queue adapter pending) |
| S046 P1 | Background activity pill and task popover: import/export/media work, progress/cancel and completed/failed work | `R/shell/TaskPill.svelte`; `R/shell/tasks.svelte.ts`; `R/legacy/core/media-work.ts` | Pro-media review supplies native Background Tasks popover and utility window. Native Pause/Resume and richer task kinds are additions. | `pm-promedia::tasks` title/unit/state/error/event models. ETA, terminal-row retention and shell wiring remain per review. **spec+model** |
| S047 P1 | Command palette: asynchronous providers, filtering, command availability, shortcuts, keyboard/focus | `R/overlays/Palette.svelte`; `R/overlays/palette.ts`; `R/overlays/palette-model.ts` | Shell review specifies native palette and shortcut column; full extension-provider state coverage still needs integration review. | `pm-shell::{transient,native,keyboard}`; compatibility contribution bridge required. **spec+model** |
| S048 P1 | Choice/dropdown/popover menus: searchable field choices, checks/disabled/header/separators, anchoring/dismissal | `R/controls/popover-menu.ts`; `R/overlays/Menu.svelte`; `R/overlays/native-menu.ts`; `R/controls/SelectField.svelte` | Shared, Kit review, Shell review and field specs cover shared behavior. Preserve tag follow-up menus; structured submenu API is later. | `pm-shell::{transient,native}`; `pm-surface::kit`; `pm-compat-host` menu protocol. **spec+model** |
| S049 P1 | Native application menus: Powermove, File, Edit, Composition, View, Window; OS Services, hide/minimize/fullscreen | `M/menu.ts`; `M/updates.ts` | Shell review covers native menus; Shared/Settings/Export supply owned actions. Tag has no top-level Layer or Help menu. | `pm-shell::{menu,keyboard,native}` frozen commands/equivalents; kernel command eligibility/acknowledgement integration remains. **spec+model** |
| S050 P1 | Context menus: viewer/layer, timeline/keyframe/property, asset/bin/composition and panel header | `R/legacy/ui/layer-menu.ts`; `E/timeline/timeline.ts`; `E/inspector/ChannelRow.svelte`; `R/panels/AssetsPanel.svelte`; `M/context-menu.ts` | Viewer/Timeline/Inspector/Media/Shared specify local menus, not a blanket license to copy later-main entries. | Native shell/menu and panel semantic action models; runtime extension contributions pending. **spec+model** |
| S051 P1 | Text context menu, spelling/edit commands, native text services and IME candidate UI | `M/text-context-menu.ts`; `M/native-edit.ts`; `R/controls/TextField.svelte`; `R/panels/agent/Composer.svelte` | Shared/Kit review and panel keyboard rules defer native text services to system. **Specified (Opus, 2026-11-01):** [surfaces/editing.md §S051](design/surfaces/editing.md). **Editing lane (2026-11-01):** all field policies, focus/IME reducer and AppKit NSTextView policy adapter; headless-tested; first-responder/NSTextInputClient attachment and Mac service checks remain. | `pm-shell::{text,keyboard,native}`; agent composer local text model. Real NSTextInput/WebKit/IME/AX integration requires Mac. **spec+model** (partial) |
| S052 P1 | Shared modal/sheet/confirm host: focus trap/return, Enter/Escape, scrim/outside dismissal | `R/overlays/Modal.svelte`; `R/overlays/modal.ts`; `R/overlays/confirm.ts`; `M/native-confirm.ts` | Shared and Shell/Kit reviews specify native sheets/system placement; surface-specific copy must come from its own spec. | `pm-shell::native::Sheet`; `pm-surface::kit` SheetAnchor contracts; compat modal handles. **spec+model** |
| S053 P1 | Toast/error notices, expandable diagnostic details/copy, retry/fix actions | `R/overlays/Toasts.svelte`; `R/errors/ToastError.svelte`; `R/errors/ErrorNotice.svelte`; `R/errors/ErrorLog.svelte`; `R/errors/ErrorFix.svelte` | Shared inline-error policy + Services review typed copy; not every notice/repair state is covered. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S053](design/surfaces/recovery.md). **Recovery lane (2026-11-01):** ten-code copy catalog, typed one-action notices, Error Log selection/copy/clear model and native table/split-view recipe tested. Native host rendering/service subscription pending. | `pm-services::{error,diagnostics}` and agent error facts; no full native notice/log UI model. **spec+model** |
| S054 P1 | Status bar: playback/performance/status contributions and “What is slowing the editor down” details modal | `R/shell/StatusBar.svelte`; `R/runtime/performance-monitor.ts` | Shell review covers status-bar vocabulary; no slowdown-details design. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S054](design/surfaces/recovery.md). **Recovery lane (2026-11-01):** ordered status/AX facts, Unicode-safe extension contributions and revision-fenced slowdown popover recipe/actions tested. Shell/telemetry attachment and NSPopover rendering pending. | Shell status state and engine telemetry are partial. **spec+model** |
| S055 P1 | Project opening ghost/loading overlay and delayed transitions; retained tab intent | `R/overlays/ProjectGhost.svelte`; `R/overlays/project-ghost.ts`; `R/legacy/app.ts` | Shared loading states do not specify project-level overlay. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S055](design/surfaces/documents.md). | Kernel async session/open and shell tab state exist; ghost/overlay model absent. **spec+model** |

| S053 P1 | Toast/error notices, expandable diagnostic details/copy, retry/fix actions | `R/overlays/Toasts.svelte`; `R/errors/ToastError.svelte`; `R/errors/ErrorNotice.svelte`; `R/errors/ErrorLog.svelte`; `R/errors/ErrorFix.svelte` | Shared inline-error policy + Services review typed copy; not every notice/repair state is covered. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S053](design/surfaces/recovery.md). | `pm-services::{error,diagnostics}` and agent error facts; no full native notice/log UI model. **spec+model** |
| S054 P1 | Status bar: playback/performance/status contributions and “What is slowing the editor down” details modal | `R/shell/StatusBar.svelte`; `R/runtime/performance-monitor.ts` | Shell review covers status-bar vocabulary; no slowdown-details design. **Specified (Opus, 2026-11-01):** [surfaces/recovery.md §S054](design/surfaces/recovery.md). | Shell status state and engine telemetry are partial. **spec+model** |
| S055 P1 | Project opening ghost/loading overlay and delayed transitions; retained tab intent | `R/overlays/ProjectGhost.svelte`; `R/overlays/project-ghost.ts`; `R/legacy/app.ts` | Shared loading states do not specify project-level overlay. **Specified (Opus, 2026-11-01):** [surfaces/documents.md §S055](design/surfaces/documents.md). **Documents lane (2026-11-01):** Delayed opening/cancel/recovery/error controller and v4 loading view implemented/tested; launch host/AX announcement callbacks remain to connect. See documents lane update below. | Kernel async session/open and shell tab state exist; ghost/overlay model absent. **spec+model** |
| S056 P2 | Agent launcher/popover, float/pin/dock/resize and busy/unread indication | `R/shell/AgentLauncher.svelte`; `R/panels/register-agent.ts`; `R/panels/agent-shell.ts`; `R/legacy/assistant/spatial.ts` | Agent explicitly chooses popover with pinning; owner decision supersedes competing presentation proposals. | `pm-panel-agentpanel::placement`; `pm-shell` Agent toolbar action. App placement adapter pending. **spec+model** |
| S057 P2 | Agent conversation: streaming text/Markdown, reasoning/tool activity groups, timeline/working timer, follow/scroll and plan preview | `R/panels/AgentPanel.svelte`; `R/panels/agent/Conversation.svelte`; `R/panels/agent/Timeline.svelte`; `R/panels/agent/Turn.svelte`; `R/panels/agent/Markdown.svelte`; `R/panels/agent/PlanPreview.svelte` | Agent covers main conversation and states. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S057](design/surfaces/agent-auth.md). | `pm-panel-agentpanel::{activity,model,view}`; `pm-agents::{conversation,persistence}`. Renderer/provider host integration pending. **spec+model** |
| S058 P2 | Agent composer, attachment chips, slash menu, draft/clipboard/selection and send/stop | `R/panels/agent/Composer.svelte`; `R/panels/agent/AttachmentChips.svelte`; `R/panels/agent/SlashMenu.svelte`; `R/panels/agent/attachments.ts` | Agent specifies composer and local focus/keyboard rules. | `pm-panel-agentpanel::{composer,input,interaction}`; native attachment/clipboard effects remain host-owned. **spec+model** |
| S059 P2 | Agent model/effort/provider/access/options menus and connection gate | `R/panels/agent/ModelPicker.svelte`; `R/panels/agent/AccessPicker.svelte`; `R/panels/agent/AgentOptions.svelte`; `R/panels/agent/ConnectionGate.svelte` | Agent covers picker/gate anatomy. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S059](design/surfaces/agent-auth.md). | Agent panel model/view connection generations; `pm-agents` provider protocol adapters. Auth/catalog host incomplete. **spec+model** (partial) |
| S060 P2 | Thread picker/history: new/ordinary/background threads, switch/search/rename/delete confirmation and unread/running state | `R/panels/agent/ThreadPicker.svelte`; `R/legacy/assistant/spatial.ts` | Agent covers thread picker; destructive/archive/history detail states incomplete. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S060](design/surfaces/agent-auth.md). | `pm-panel-agentpanel::{model,view}`; `pm-agents::persistence::Threads`; orchestration/thread tool bindings incomplete. **spec+model** (partial) |
| S061 P2 | Inline agent questions, approvals, text/secret answers, suggested options and pending response | `R/panels/agent/QuestionCard.svelte`; `R/panels/agent/inline-prompt.ts`; `M/agent-approvals.ts` | Agent specifies question-card behavior; no selection may submit implicitly. | `pm-panel-agentpanel::questions`; `pm-agents::approvals`. Native reply worker wiring pending. **spec+model** |
| S062 P2 | Agent visual reply, generated UI placement ghost/preview and reopen interactive artifact | `R/panels/agent/VisualReply.svelte`; `R/panels/agent/UIPlacementGhost.svelte`; `R/panels/agent/ui-placement.ts`; `R/panels/agent/visual-reply.ts` | Agent covers visual replies at a high level; full placement/artifact surface design missing. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S062](design/surfaces/agent-auth.md). | `pm-panel-agentpanel::{activity,view,model}` retains references/interactions; actual v4/compat renderer binding pending. **spec+model** |
| S063 P2 | Agent result cards and actions: apply/undo, extension/mod result, project handoff and follow-up | `R/panels/agent/ModResult.svelte`; `R/panels/agent/ResultActions.svelte`; `R/panels/agent/GhostCard.svelte`; `R/panels/agent/mod-result.ts` | Agent specifies result actions; **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S063](design/surfaces/agent-auth.md). | Agent panel immutable result/activity models, guarded kernel result undo; application artifact/adoption effects remain. **spec+model** (partial) |
| S064 P2 | Repair Agent/error recovery card and repair draft flow | `R/panels/RepairAgent.svelte`; `R/panels/repair-agent.svelte.ts`; `R/errors/ErrorFix.svelte` | No complete repair-flow design. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S064](design/surfaces/agent-auth.md). | General agent error/effect infrastructure is not a Repair Agent flow model. **nothing** |
| S065 P2 | Agent attachment image-preview modal and reveal/open attachment actions | `R/panels/agent/attachments.ts`; `M/shell.ts` | Agent mentions attachment activation; full preview modal absent. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S065](design/surfaces/agent-auth.md). | `pm-panel-agentpanel::composer` attachment records and host effects; byte hydration/reveal adapter pending. **spec+model** |
| S066 P2 | Spatial agent selection overlay: drag an interface region, screenshot attachment, target/focus picker, anchored prompt, send/cancel, reduced-motion/ripple fallback | `R/legacy/assistant/spatial.ts`; `R/legacy/assistant/RippleCanvas.svelte`; `R/legacy/assistant/RippleFrame.svelte` | Agent spec covers conversation placement, not spatial region selection. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S066](design/surfaces/agent-auth.md). | Agent composer/context and kernel selection provide partial backing; no native spatial overlay/capture controller found. **spec+model** |
| S067 P2 | Settings window/screen: search/sidebar sections, selection, no results and close | `R/settings/SettingsScreen.svelte`; `R/settings/install.ts`; `R/settings/search.ts` | Settings specifies native window, section/search states and keyboard. | `pm-shell` Settings shell; `pm-services::settings::SettingsService`; kernel state. Search/form reducers and controls still need wiring. **spec+model** |
| S068 P2 | Project Settings pane: composition, colour/depth, preview/delivery defaults and custom dimensions | `R/legacy/ui/project-settings.ts`; `R/settings/SettingsScreen.svelte`; `R/core/color-depth.ts` | Settings Project section and Export cover these fields. **DESIGN-TODO(opus):** uncovered colour/depth/validation states rather than inferring from engine options. | `pm-panel-inspector` composition binding; kernel state/commands; engine output/colour configuration. **spec+model** (partial) |
| S069 P2 | General Settings: appearance, workspace import, preview memory, cloud downloads and restore windows | `R/settings/SettingsScreen.svelte`; `R/onboarding/workspace-import.ts` | Settings specifies General rows. Import workflow is separately missing in S110. | `pm-services::settings`; kernel `StateTransaction`; `pm-shell` appearance/session. Actual preference consumers must be wired. **spec+model** |
| S070 P2 | Accounts subscriptions: ChatGPT/Codex and Claude sign-in/status/disconnect; external MCP client registration | `R/legacy/ui/chatgpt-settings.ts`; `M/codex/app-server-account.ts`; `M/claude/account.ts`; `M/claude-desktop-mcp.ts` | Settings names subscriptions; full connection, browser/device-code and external-client install/undo states missing. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S070](design/surfaces/agent-auth.md). | `pm-agents::{codex,claude,protocol}` partial provider backing; `pm-mcp` transport. OS registration/auth/secure credential host remains. **spec+model** |
| S071 P2 | API/local model provider configuration: endpoint/key/model/vision, validation/test/disconnect | `R/legacy/ui/compatible-settings.ts`; `M/compatible-provider.ts` | Settings Accounts gives placement, not full provider form/testing states. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S071](design/surfaces/agent-auth.md). | Migration preserves provider settings/secrets; no complete native provider-config/auth model found. **nothing** |
| S072 P2 | Agent runtime install/download/update progress and runtime errors | `M/runtime-updates.ts`; `M/codex/connection-status.ts`; `R/legacy/ui/chatgpt-settings.ts`; `R/panels/agent/ConnectionGate.svelte` | Agent/Settings specify gate/status only. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S072](design/surfaces/agent-auth.md). | Native agent process/protocol models exist; complete runtime installer/updater missing. **spec+model** |
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
| S083 P2 | Cloud sign-in/sign-up sheet: provider/email entry and waiting in browser | `R/cloud/SignInSheet.svelte`; `R/cloud/account.ts`; `M/cloud/auth.ts` | Settings accounts placement only. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S083](design/surfaces/agent-auth.md). | `pm-services::entrypoints::AuthLink` queue/parsing only; auth exchange/session model absent. **spec+model** (entry-point backing) |
| S084 P2 | Email verification code/resend/expiry/errors and publisher handle claim/resume | `R/cloud/SignInSheet.svelte`; `M/cloud/auth.ts`; `M/cloud/service.ts` | No dedicated design. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S084](design/surfaces/agent-auth.md). | No native code/handle/auth flow model found; migration retains session payloads only. **nothing** |
| S085 P2 | Account button/avatar/menu: identity, Choose Handle, Your Library, Settings, Sign Out and remembered-install preference | `R/cloud/AccountButton.svelte`; `R/cloud/Avatar.svelte`; `R/cloud/account.ts`; `R/settings/SettingsScreen.svelte` | Settings covers account rows, not complete account menu. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S085](design/surfaces/agent-auth.md). | No native account state/menu model found; kernel preference persistence is insufficient. **nothing** |
| S086 P2 | Delete Account and custom registry switch native confirmations | `M/cloud/service.ts`; `M/cloud/registry-url.ts`; `R/settings/SettingsScreen.svelte` | No confirmation specs. **Specified (Opus, 2026-11-04):** [surfaces/agent-auth.md §S086](design/surfaces/agent-auth.md). | Services validates Store origins; no native account deletion/registry-switch flow. **nothing** |
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

## Authority surface implementation — 2026-11-01

Lane `feat/na-cloud-s-authority` adds only app-owned authority source and module
registration. The S010–S012 coverage cells above distinguish implemented surface
contracts / native alert adapter source from app integration. They do **not**
claim that these surfaces are reachable in the running editor. S011 now has a
headless surface model; its persistent value service is still absent. No other
row or historical audit count is rewritten.

Implemented: exact Opus copy; safe keyboard defaults and native alert semantics;
per-window FIFO alerts; id/generation-scoped responses and stale-target closure;
run and standing-consent command protocols; session-only Full access restoration;
setup required/optional/secret/set/unreadable/saving/error/closed states; explicit
Reveal and concealed pasteboard Copy; immediate single-value Remove; archival
removal commands and the no-values uninstall bypass. The bounded worker bridge
uses `try_send`; no UI-thread JS, service IO or kernel-future wait was added.
`authority_check.rs` creates a disposable Cargo harness for these exact sources
under `work/`, without depending on spikes or modifying workspace manifests.

Decisions and selecting rules:

- N1/N3 + shared alert grammar: preserve S012's listed Delete/Cancel order but
  explicitly make Cancel the default and Esc action; destructive actions never
  acquire Return implicitly from AppKit's first-button behavior.
- N3: the newer S010 surface spec owns computer-access alerts even though the
  older Agent panel spec generally puts approvals inline. Generic agent secret
  questions continue to use the Agent inline card and cannot open this sheet.
- N3/N6: Reveal displays the full explicitly requested value as authority.md
  requires; tagged `env/ipc.ts`'s masked reveal detail is superseded. Keep the
  tagged immediate Remove and non-empty-value semantics (whitespace survives
  exactly); Cancel never resurrects an already removed value.
- N1/N2/N4: Save protocol requests an atomic durable batch, retaining typed drafts
  on errors instead of exposing tagged sequential partial-write success. Explicit
  Cancel may close an in-flight sheet; it does not undo already admitted writes.
- N4/N6: commands archive releases/workspaces/local projects; only an explicit
  value removal selects credential deletion. The worker must archive a release
  successfully before erasing its selected values, preserving tagged failure
  behavior. Nothing in the Electron app or existing consumers was removed.
- N7/N8: request ids and owner generations accompany all effects; secret-bearing
  payloads redact Debug and never enter general inspection, project state or a
  v4/remote-DOM tree. Worker completions must be checked against the live owner.
- Design ownership: no replacement sheet metrics, new error wording, palette,
  layout, spacing or visual default was chosen. All output awaits Opus review.

`DESIGN-TODO(opus)` remaining: supply `metrics.sheet_inset` and
`metrics.sheet_value_column`; specify failure copy for Reveal/Remove and a
post-uninstall value-deletion failure. Until supplied, setup geometry remains
unrendered and those operation failures return typed codes without invented copy.

Changes needed outside this lane's ownership:

1. App/session routing must instantiate the controller and AppKit host, register
   windows, drain native responses, invalidate targets before close/replacement,
   publish banner/notice AX announcements and restore trigger focus. Consume the
   process-only `StandingAccess` once across windows and persist its previous
   non-Full level only. Mount the missing-values `EmptyState` in the v4 panel host.
2. Add app dependencies / worker binding to `pm-agents`, `pm-panel-agentpanel`,
   `pm-store-host` and `pm-services` as needed (and update only their lock entries).
   Dispatch `ComputerConsent` to the matching Panel effect acknowledgement;
   dispatch `RunDecision` to the exact run's Approvals, mapping display
   `Allow This Run` to protocol `Allow for this run`. Never reuse a grant across
   run generations. Publish Awaiting answer / denied rows from real agent state.
3. Implement the env/Keychain worker behind `ValuesService`: resolve migrated
   provenance/env identity; validate owner generation and declared keys; commit
   writes atomically, return code-only failures and refresh/reload the extension
   in every window after value changes. Preserve C-MATRIX's frozen activation
   `api.vars` snapshots until replacement activation. Route successful explicit
   reveal completions to the matching native alert only.
4. Wire uninstall to `pm-store-host::install::uninstall` with expected revision;
   clear explicitly selected values only after successful release archival.
   Extend LayoutStore's durable archive and Undo Delete Workspace path; its
   current `delete_named` simply removes the saved entry. The surface must never
   call that alone while claiming archival/undo. Local project commands use the
   Library's Trash/archive service and preserve bytes.
5. Implement the native setup host with `NSTextField` / `NSSecureTextField`,
   borderless declared actions and native focus/Tab/Return/Esc once Opus supplies
   the missing metrics. The current `SetupHost` is a contract and Linux double,
   not a rendered sheet.

Validation (Linux): **48 tests passed, 0 ignored** using exact app-owned sources.
Locked Linux and macOS-target all-target clippy pass with `-D warnings`; the
AppKit adapter type-checks for `aarch64-apple-darwin`. No production dependency,
manifest or lockfile change is needed for this module. The transient harness
pins the app's objc2 versions and locks before testing. Reproduce from repo root:

```sh
source /workspace/.tools/activate.sh
mkdir -p work
rustc --edition=2024 apps/native/crates/app/src/surfaces/authority_check.rs -o work/authority-check-runner
work/authority-check-runner --macos
```

The runner executes `cargo test --locked --manifest-path
work/authority-check/Cargo.toml` (48), `cargo fmt --check`, and Linux/macOS-target
`cargo clippy --locked --all-targets -- -D warnings`, plus macOS-target
`cargo check --locked --all-targets`. Install the Rust `aarch64-apple-darwin`
standard-library target before `--macos`. The runner does not execute Mac UI.
The requested `cargo test --locked -p pm-native-app` stops at objc2's Apple-only
compile guard on Linux. Package-wide fmt currently also reports an existing,
unmodified `app/src/smoke.rs` formatting difference; authority sources and module
registration are formatted without editing that unowned source.

Still required on macOS: full locked app build/tests; native sheet ownership and
cross-surface queueing; Return/Esc and default-button-cell behavior (especially
Delete-first/Cancel-default); system VoiceOver and focus return; three *visual*
summary lines with ellipsis and the full activity row; secure fields/Keychain
ACL/re-entry; concealed pasteboard type; multiwindow/quit grant expiration;
actual durable archive/Undo and failure injection. Verify light/dark, Increase
Contrast, Reduce Transparency/Reduce Motion and obtain Opus visual approval.

## Library lane implementation coverage — 2026-11-01

S005/S017/S018/S019 now have a registered app module at
`apps/native/crates/app/src/surfaces/library.rs`, a nonblocking kernel-worker
port, a drivable reducer, an immutable/virtual presentation and a Linux
`LibraryHost` double. The macOS module presents the specified system NSAlert
as an asynchronous sheet, with explicit default/destructive behavior and both
Return and Escape when Cancel is default. This is **controller and host-contract
coverage**, not an installed Project Library window or visual approval.

The 44 tests cover exact copy, empty/search/Trash/loading/unavailable states,
keyboard and AX labels, modal focus isolation, alert ownership/ordering/stale
callbacks, validation and timecode, duplicate/rename/Undo, 60,000-project
virtualization and real actor persistence. Synthetic profiles are canonicalized
before use. A disposable harness compiles these exact module files and checks
that every dependency version/checksum matches `apps/native/Cargo.lock`.
No workspace manifest, dependency or lockfile change is needed.

Reproduce from the repository root after activating the shell:

```sh
mkdir -p work
rustc --edition=2024 apps/native/crates/app/src/surfaces/library_check.rs -o work/library-check
work/library-check prepare
work/library-check test --locked
work/library-check clippy --locked --all-targets -- -D warnings
work/library-check fmt --check
rustfmt --edition=2024 --check apps/native/crates/app/src/surfaces/library_check.rs
work/library-check check --locked --target x86_64-apple-darwin
```

The normal `cargo test --locked --manifest-path apps/native/Cargo.toml -p
pm-native-app` is blocked on Linux by unconditional Apple-framework dependencies
(`objc2`). The harness cross-check requires the matching Rust target; it checks
types and does not link or run AppKit. ARM cross-check additionally requires an
Apple-capable C compiler for the existing `sha2-asm` dependency. No native
runtime pass is claimed.

Decisions and selecting rules:

- **N3/design ownership:** use the exact Opus copy and existing token values;
  unresolved tokens stay symbolic. No layout, wording or interaction was invented.
- **N3/W006:** keep the tagged Recents default and 12-item limit until a search,
  alongside the native spec's removal of pagination. Native Name sort delegates
  to system locale collation; the Linux test comparator is deterministic only.
- **N4/N6/prune rule:** preserve every existing consumer and every project byte;
  there is no deletion fallback or hidden open/save/close rename workaround.
- **N7/N8:** all requests/completions/targets carry IDs; IO/search and file save
  packing run on workers, with nonblocking enqueue/poll/wake at the UI boundary.
- **N1/N3:** missing kernel operations report an inspectable integration gap
  and the approved generic error, never a successful placeholder result.

Changes required outside this lane: implement `LibraryHost` with the actual
NSToolbar/sidebar/grid/table and AppKit New Project sheet, wire File ▸ Project
Library / Welcome ▸ Show All Projects / New Project and the editor title menu,
route first-save-in-Trash/Save As through the document owner, and connect the
worker wake/publication to the app executor. The host must restore/persist frame
and library preferences, keep one window, use the system relative-date formatter,
honor sheet focus/Tab order and graft the AX list/table into the window tree.
Kernel owners must add unopened-slot rename, open-session Trash and an
ArchiveReason-backed library archive-delete operation, and typed storage errors.
Saved index rows still lack complete duration/rate/thumbnail capability facts;
the presentation retains unknown values rather than fabricating zeroes.

`DESIGN-TODO(opus)` items (also inspectable as `library_surface::GAPS`):

- Add the already specified `metrics.thumbnail_min`, `metrics.sheet_inset`,
  `metrics.sheet_value_column` and `colors.media_badge` to pm-design.
- Specify missing-summary card/VoiceOver wording, size-link toggle accessible
  name/initial state, loading reason and empty-state symbols.
- Define Duration/Size/Frame Rate column sort mapping/direction, drop-frame
  timecode input, how a duplicate outside the current search/filter is revealed,
  and cancellation once a New Project creation is already admitted.

On macOS, verify real sheet/alert ownership and focus return, Cancel's Return
and Escape behavior, in-place text/IME, title menu, Finder reveal, singleton
window/frame restoration, native date/collation, accessibility/keyboard scrolling
through 60,000 rows, colour-popover anchoring, light/dark and all accessibility
settings. All visual output remains provisional pending Opus review.

## Media surface implementation — 2026-11-01

Scope: S006, S007, S024, S038, S039, S040 only. Implementation lives in
`apps/native/crates/app/src/surfaces/media*.rs` (registered by `surfaces/mod.rs`)
and `apps/native/js/v4/surfaces/media/`. This is **partial surface integration**,
not a claim that these flows are installed in a running editor or visually approved.

The lane implements approved copy and semantic states, stable document/revision/
generation targets, per-window FIFO and one-shot alert responses, a Linux host
double, native asynchronous `NSAlert`/`NSOpenPanel` adapters, v4 source-error and
Background Tasks body components, and Rust-to-v4 projections. Guarded kernel
admission records completed batches as `Import Media` and replaces sources without
changing their IDs, layer trim/timing, folder or unknown extension fields. The
worker must archive original bytes before supplying its receipt. This module
performs no byte deletion or filesystem probing.

Decisions and selecting rules:

| Decision | Rule |
| --- | --- |
| Use system asynchronous alerts and file pickers, with a queued trait boundary and Linux double; no app-modal loop for document media. | N1/N2 system-first pro behavior; N8 no UI-thread waiting. |
| Compare rational probe values exactly before formatting; serialize u64 identity fields to v4 as decimal strings. | N3 exact authored intent; N7 exact id-based inspection/driving. |
| Cancel remaining jobs, retain completed originals and archived receipts, retry failed imports/proxies only, and fence late generations. | N4 no data loss; N3 correct retries and undo. |
| Guard kernel admission by document lifetime and revision; preserve unknown fields on replacement. | pm-kernel single writer; N3/N4/N6. |
| Leave v1–3 logic/remote DOM unchanged and remove no existing consumer or source. | N6/N8 adopted V8 architecture; prune rule. |
| Carry exact sequence frame numbers, hold gaps and fit mixed sizes to the first image; do not port the tagged skip/reject behavior. | N3 the approved specification wins over shipped behavior. |
| Expose partial coverage and missing native/provider installation explicitly. | N1/N3 correctness over unverified parity. |

`DESIGN-TODO(opus)` items, with neutral unresolved placeholders in code:

- S006 supplies only the negative audio mismatch sentence. Supply copy when the
  chosen file has audio and the recorded original does not. This remains
  `Check::NeedsDesign`, with no implicit use of the file.
- Choose the `{Object}` noun for a sequence target disappearing or its project
  closing. The model closes immediately and accepts an approved object label;
  `None` emits no invented banner wording.
- Shared sheet grammar names `m.sheet_inset` and `m.sheet_value_column`, but
  `pm-design` defines neither. Sequence sheet tokens remain unresolved; no
  metric alias or new layout is guessed. S039's native sheet is still pending.

Required changes outside this lane's ownership: the app session/controller must
retain `AlertQueue`, interactions and `CloudNotices` per document/window, install
`AppKitHost`, route its channels without blocking, and retire them on close.
Bind existing media-panel effects and worker probe/admission acknowledgements to
these states; map `pm-services::ImportQueue`/typed errors, Settings consent,
`pm-promedia::Availability` and Background Tasks events. Implement cloud hydration,
Figma clipboard conversion and sequence-worker dispatch where those services are
absent. Install the source view under the viewer's dark token scope with an opaque
SF Symbol resource and token-derived symbol size; keep source chrome, engine
leases, last-frame/video presentation and the existing shared banner host intact.
Install `TaskBody` in the shell-owned task row/disclosure, not a new window.
Streamed per-file kernel admission still needs service-owned grouping so every
completed card can appear immediately and cancellation keeps one undo step; the
provided completed-batch admission seam does not claim that streaming behavior.

No product crate or dependency was added, so neither production Cargo lockfile
needs an update. The portable validation harness below is temporary tooling,
not a new workspace member. From the repository root, reproduce it with Python
3.9+ (the library path references only existing production source):

```sh
mkdir -p work/media-check
python3 - <<'PY'
from pathlib import Path
import shutil
root = Path.cwd()
check = root / 'work/media-check'
(check / 'Cargo.toml').write_text('''[package]
name = "pm-media-surface-check"
version = "0.0.0"
edition = "2024"
[workspace]
[lib]
path = "lib.rs"
[dependencies]
serde_json = "1"
pm-kernel = { path = "../../apps/native/crates/kernel" }
pm-model = { path = "../../engine/crates/pm-model" }
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = "0.3"
objc2-foundation = "0.3"
block2 = "0.6"
async-channel = "2"
''')
(check / 'lib.rs').write_text(
    '#[path = "../../apps/native/crates/app/src/surfaces/mod.rs"]\n'
    'pub mod surfaces;\n')
shutil.copyfile(root / 'apps/native/Cargo.lock', check / 'Cargo.lock')
PY
cargo +1.96.1 metadata --manifest-path work/media-check/Cargo.toml --format-version 1 > work/media-check/metadata.json
cargo +1.96.1 test --locked --manifest-path work/media-check/Cargo.toml
cargo +1.96.1 clippy --locked --manifest-path work/media-check/Cargo.toml --all-targets -- -D warnings
```

The normal `cargo test --locked --manifest-path apps/native/Cargo.toml -p
pm-native-app` reaches the existing Apple-only `objc2` platform guard on Linux.
The portable harness exercises the same new Rust sources, including real kernel
actor/undo tests. macOS must run the app crate tests, verify actual sheets/pickers,
Return/Escape/Tab/focus restoration, suppression persistence, VoiceOver and
announcements, cancellation/provider/clipboard outcomes, every appearance and
accessibility setting, and Opus visual review. Apple-silicon cross-checking of the
isolated alert/picker source proves compilation only, with no AppKit runtime claim.

### Media lane reconciliation and checks — 2026-11-01

Initially reconciled at `7204fe91cde46019c2700e56c48f9301ad35d5f2`, then
fetched and merged the newly advanced `origin/feat/native-engine` at
`9897e48dd6986574b1beddccb47bab986dfa7bf4` without conflicts. The incoming
compatibility lane and its lockfile entries are retained. After that final merge, the portable media harness passes **48 tests** and the
existing kernel/media/services/pro-media suites pass **493 tests**, with **one
existing real-profile test ignored**. The v4 content suite passes **5 tests**.
Exact checks (repository-root working directory unless stated):

```sh
cargo +1.96.1 test --locked --manifest-path work/media-check/Cargo.toml
cargo +1.96.1 test --locked --manifest-path apps/native/Cargo.toml -p pm-panel-media -p pm-services -p pm-promedia -p pm-kernel -- --test-threads=1
cargo +1.96.1 clippy --locked --manifest-path work/media-check/Cargo.toml --all-targets -- -D warnings
work/js-media/node_modules/.bin/bun test apps/native/js/v4/surfaces/media
cargo +1.96.1 fmt --manifest-path apps/native/Cargo.toml --package pm-native-app -- --config skip_children=true --check
rustfmt +1.96.1 --edition 2024 --config skip_children=true --check apps/native/crates/app/src/surfaces/*.rs
work/js-media/node_modules/.bin/prettier --check apps/native/js/v4/surfaces/media
```

The Bun/Prettier paths above point to temporary development tools (Bun 1.3.14,
Prettier 3.6.2) installed under `work/`, not shipped dependencies. Strict
TypeScript 5.9.3 passes from `apps/native/js/v4`:

```sh
node_modules/.bin/tsc --noEmit --strict --skipLibCheck --moduleResolution bundler --module esnext --target es2022 --types bun surfaces/media/index.ts surfaces/media/media.test.ts
```

The same alert/picker source also passes isolated Apple-silicon cross-target
Clippy with `-D warnings`, using only its existing objc2/block2/async-channel
app dependencies. Its scratch manifest is `work/media-mac/Cargo.toml`; command:
`cargo +1.96.1 clippy --locked --manifest-path work/media-mac/Cargo.toml
--target aarch64-apple-darwin -- -D warnings`. This does not certify AppKit runtime,
GPUI/Metal integration, VoiceOver, the unfinished sequence sheet or visual review.

Initial backing-suite runs exposed missing local frozen reader tags (`v1.2.0`,
`v1.3.0`); fetching those read-only references fixed the oracle prerequisites.
One parallel kernel `slice2` run hit a library-lock assertion; its isolated
rerun and the complete serialized suite both pass. No backing crate, hook or
existing test was changed. Formatting is scoped to the touched sources; an
unrelated pre-existing formatting difference in `app/src/smoke.rs` is unchanged.
This lane adds no production lockfile entries beyond those inherited from the
merged base. Staged diff checks and both requested
Gitleaks forms pass before each commit; the repository pre-commit hook remains
active.

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

## Documents lane implementation — 2026-11-01

Coverage updates above apply **only** to S002/S003/S004/S014/S016/S020/S055.
These are implementation contracts and headless/native-target verification, not
running-app completion or Opus visual approval.

Owned source: `apps/native/crates/app/src/surfaces/documents*.rs`, module
registration, and `apps/native/js/v4/surfaces/documents/`. Public reducers expose
stable window IDs, generation-checked `SessionHandle`s and unique request
tickets. `Document::from_published` reads the real kernel publication;
`submit` returns the real actor `Pending`, never blocks the UI thread. The
service boundary uses `ServiceErrorCode`'s reviewed serde wire values; tests
exercise every real service code and `FileOpenFailure.path`, without showing
diagnostics or exposing lock owners. `save_preflight` is shared by Command-S
and close-save: active work joins, copies/never-saved documents use Save As,
and changed files require the specified choice.

Implemented behavior includes flush/join/recheck before close, per-tab review,
multi-document quit, archive-on-discard, cancelled Save panel cancellation,
failed-save abandonment, stale-result rejection, recovery-copy receipts,
delayed opening and announcements, cancelled-open retirement by generation,
restore aggregation, deferred tab transfer, and popout return/restore policy.
Compacted recovery IDs must be resolved to their **actual** document names and
their library notices retained before acknowledging `recoveries_recorded` and
terminating; they are never labeled with the last closing document's name.
`CloseDocument` retires a session/tab; it closes its native window only when
that window's final tab retires.

The macOS adapter uses real NSAlert/NSOpenPanel/NSSavePanel and standard titled,
resizable, non-floating NSWindow popouts with tabbing disallowed and per-panel
frame autosave. Alert key equivalents are Return/Esc/Command-D and destructive
actions are never default. Pickers use UniformTypeIdentifiers, preserve the
system extension-hidden preference and delegate overwrite confirmation to the
system. `announce` posts real NSAccessibility announcements. The v4 views use
existing kit components/tokens and six atomic ops for opening, inline/viewer
notices and the popout header; no browser APIs or JS timers own the lifecycle.
Viewer material/placement metadata must be consumed by the native host.

Required work outside this lane's ownership:

- App integration must connect `session.rs`, `lifecycle.rs` and existing picker
  callers to these effects, await kernel work off the UI thread, feed fresh
  publications/import state in tab order (including before barrier replies), gate
  new native/extension/agent edits during final release, and route AppKit completion tickets
  through `PresentationQueue`. Replace neither old caller until all its
  consumers migrate. `applicationShouldTerminate` must honor `TerminateLater`
  and eventually call `reply(toApplicationShouldTerminate:)` for the result.
- Shell integration must retire tabs by session, select the reviewed tab, defer
  ownership transfer until saves finish, route popout close/button/menu to the
  same reducer, preserve actual panel content, and persist popped-out state.
  Project close bypasses the normal popout-return delegate. Cancelled late opens
  must dispatch DeferClose for the returned generation, never save it.
- Services/kernel integration must supply external-change observations and
  successful reload receipts: there is no watcher/reload API to call today.
  `ShellReply::Changed(bool)` currently acknowledges unrelated library/session
  operations and is **not** proof that a file was reloaded. Preserve dirty state
  on uncertain durability; map untyped kernel Storage failures to `unknown`
  rather than parsing diagnostics. Resolve recovery IDs/path existence on a
  worker and retain inline notices when no viewer survives the close.
- Native/v4 hosting must attach the views to their owning viewer/library/header,
  consume the existing pm-design ViewerOverlay material in dark scheme, and
  deliver the announcement/lifetime/action/dismiss semantics. Action banners
  persist; passive viewer banners expire after four seconds. Standard native
  buttons supply keyboard activation and VoiceOver semantics.

`DESIGN-TODO(opus)` placeholders, with no lane-invented visual substitute:

- surfaces/README §2 names `m.sheet_inset` and `m.sheet_value_column`, neither
  exists in pm-design. The AppKit Finishing Up request reports an inspectable
  `Unavailable` result until the design tokens exist; it does not substitute
  an alert or guessed geometry. Its exact title/line/Cancel semantics and
  one-second delay are implemented and tested.
- The notice grammar requires a symbol but does not name the info/warning
  symbols. The v4 notice leaves that slot empty with a `DESIGN-TODO(opus)`
  metadata marker. Other specified symbols, copy and AX labels are exact.

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


## Dialogs lane implementation evidence — 2026-11-01

Owned source: `apps/native/crates/app/src/surfaces/dialogs*.rs`, registered
in the app's `main.rs`. Scope: S021, S022, S029, S036, S043 only. Coverage
cells above retain the original design evidence and append this lane's limits.
These are executable dialog/presentation contracts and real worker-side kernel
adapters, **not a claim that the editor now opens the form sheets**. Existing
surfaces/consumers and extension contracts are retained. All visual output is
provisional until Opus review.

Built and exercised:

- S021 six rows, resolution choices, field validation, unique names, timecode
  input, exact duration warning and Create/Apply. Kernel settings are one undo
  step and preserve coordinates, unknown project data and keyframe identities.
  Unchanged fields are omitted so a name-only edit keeps authored background
  fills and work areas intact; fractional frame rates remain exact.
- S022 Leave/Move descriptions, disabled explanation, duration/open checkboxes
  and delete confirmation. Delete is a real asynchronous NSAlert recipe with
  Cancel as default. Pre-compose operation is explicitly unavailable: existing
  `CompCommand::Precompose` is `NotImplemented`, not an implemented native
  operation despite the older backing-column summary.
- S029 mixed/two-component values, incoming/outgoing groups and units,
  influence validation, numeric-only value scale, collision copy, locked-layer
  exclusion and one labelled undo step. Scaling preflights the existing graph
  command and refuses clamping rather than changing authored intent silently.
- S036 native text requirements, exact editor/result labels, keyboard routes,
  shipped insertion names in tagged order, UTF-16 caret replacement, 250 ms
  worker-job debounce, stale-result rejection and medium announcements. A
  frozen scratch mirror runs the existing engine parser/evaluator without
  changing the document; runtime errors allow Apply. Source spans absent from
  engine diagnostics remain neutral placeholders, never a guessed line number.
- S043 exact replacement/rename/delete alerts, Export-sheet ownership, explicit
  accessory width, preset menu labels, preserved legacy options/unknown fields,
  output-name exclusion, state revision fencing and archive-before-removal.
  No in-memory archive is presented as a durable production implementation.

Presentation boundaries: `DialogHost`/`AlertHost` have a Linux recording double;
`AppKitAlertHost` builds standard NSAlert and NSTextField controls and calls
`beginSheetModalForWindow` asynchronously. Window lookup requires the actual
Export sheet for nested preset alerts and refuses a missing parent. The app
icon, placement and alert accessibility stay system-owned; none of these
reversible confirmations uses the critical style. No runModal, Node, Chromium,
main-thread JS wait or second live document writer is introduced.

Rule-selected decisions:

| Decision | Selecting rule |
| --- | --- |
| Use the specified strings, native alert controls, explicit Cancel destructive default and token references. | N1/N2 and Opus design ownership; no lane-authored visuals or copy. |
| Plan typed commands on a frozen worker scratch store, then submit the result through the actor's existing guarded Restore call. It checks document session and revision atomically and refuses an open edit. | N1/N3/N4: prevent a queued Apply from editing a closed/reopened project; the actor stays the only live writer. |
| Refuse a scale that the graph kernel would silently clamp. | N3: the authored scale wins; collision is inline with Apply disabled. |
| Preserve the v1–3 renderPresets array and unknown fields; archive complete old entries before removing/replacing them. | N4/N6: no data loss and no incompatible envelope/archived entries exposed to legacy consumers. |
| Expose missing backing, metric tokens and source spans as typed gaps/neutral placeholders. | N1/N3 and design ownership: no success-shaped operation or invented line/metric/copy. |
| Keep ID-addressed snapshots, input/completion queues and asynchronous kernel calls. | N7/N8: headless inspection/driving without UI-thread JS or kernel waits. |

Integration required outside this lane's owned paths:

1. `pm-design`: supply `metrics.sheet_inset`, `metrics.sheet_value_column`,
   `metrics.sheet_width_wide`; this lane does not alias them to unrelated tokens.
2. App session/shell: route M026/M028, Properties/Timeline/Project and Export
   entry points to these public contracts; resolve editor/Export-sheet IDs,
   construct AppKit form rows with the kit NumberField and P-TEXT NSTextView,
   consume queued native input, refresh/revalidate publications, close stale
   targets, show banners and restore trigger focus. Only module registration
   is permitted in this lane; `session.rs`/shell renderers remain unmodified.
3. Kernel/operation owner: implement actual pre-compose geometry, source mode,
   duration adjustment and open-after handoff. Also remove the composition
   settings one-frame duration floor where it conflicts with the specified
   0.1–3600 s range; this adapter returns a typed backing gap instead of
   silently changing a requested subframe duration. The dialog never fakes it.
4. Services/storage: implement `PresetArchive` with durable app-storage receipts
   before calling the guarded `renderPresets` state transaction. The test
   archive is intentionally test-only. Map the service's existing error enum
   into the boundary's matching codes, without showing diagnostic strings.
5. Engine: supply source spans for diagnostics that lack them; Opus chooses
   the neutral result placeholder until then.

DESIGN-TODO(opus), mirrored in source: missing sheet tokens; multi-selection's
unnamed-layer substitution for the disabled Leave option; export-preset
empty/duplicate rename validation and preview pending/unavailable states;
Time/Value percentage validation copy; source-line placeholders when the engine
has no span. No visual/form dimensions, copy or interaction are invented to
resolve these gaps.

Linux reproduction (all fixtures are owned Rust source; Python is unnecessary):

| Use real system controls, exact reviewed copy and existing kit/tokens; keep missing visuals as explicit placeholders. | N1/N3 and Opus ownership; no design choices by this lane. |
| Recheck fresh published dirtiness after a joined/completed save; cancel only close/quit intent. | N3 authored intent and N4 retained work. |
| Use DiscardClose and retain every recovery ID; never delete a version or infer a recovery name. | N4 archive-never-delete; N3 correctness. |
| Release clean sessions with DeferClose after the barrier; Close would save again and could rewrite an external version. Re-prompt on new edits during the barrier. | N3 authored intent; N4 no silent loss. |
| Session generation and process-unique tickets gate callbacks; expose reducer stages and effects by ID. | N7 agent-native; kernel single-writer contract. |
| Queue asynchronous Rust/AppKit work; v4 views own no IO or lifecycle timer. | N8 UI thread never waits on JS; N6 existing extension contracts untouched. |
| Keep all existing lifecycle consumers and edit only owned registration/coverage paths. | Prune rule and lane ownership. |
| Follow S016's Viewer/Tools dock-only policy instead of copying the tagged timeline prohibition. | N3 specified intent wins over implementation imitation. |
| Use OK as the explicit default for S004/S014 acknowledgements even where prose lists Finder/Library first. | N3 specific default instruction and shared NSAlert first-button grammar. |

Reproduction (from the repository root; all test/build/check commands use the
locked dependency graph):

## Editing implementation — 2026-11-01 (cloud editing lane)

S025/S034/S035/S051 now have source in
`apps/native/crates/app/src/surfaces/editing*.rs` (registered as the public
`editing` module) and `apps/native/js/v4/surfaces/editing/`. This is **tested
presentation/state/adapter coverage, not end-to-end app or visual acceptance**.
No existing panel/extension, document data or consumer was removed. The shared
inventory changes above append only to the four owned coverage cells.

- S025 plans dark-scheme path vertices, selected/neighbour tangents, hover,
  close ring, rubber band, locked/hidden controls, native text caret/marked text,
  paragraph overset and exact AX names. Combined engine transforms move geometry
  without scaling display-sized handles or changing the authored-pixel AX values.
  Tagged close/delete precedence and Option tangent mirroring are tested.
- S034 projects immutable kernel animator objects without discarding unknown
  fields; the v4 section uses the existing UI kit and six atomic operations.
  Disabled animators keep their content editable. Collapse, animated header,
  native menu requests, rename Return/Escape, style-selection guards, property
  menus/removal and stamped edit actions are tested. No optimistic document writes.
- S035 plans the specified `NSPopover`, four Fill types, previous/new swatch,
  spaces, six notations, checkmarked hue settings, gamut warning, gradient
  stop/midpoint keys and document recent colours. The worker reuses existing
  colour math and adds the missing OKLab notation parser. Copy is the current
  notation field verbatim. Live edits use real kernel gesture leases, so Escape
  uses selective cancellation rather than restoring a stale saved value.
- S051 supplies policy for every field kind and preserves captured AppKit
  defaults when a text slot is reused. It never replaces the system context
  menu. Focus remembers the originating panel, suppresses editor shortcuts while
  editing, and leaves marked-text Escape to the input manager. The AppKit host
  uses real `NSPopover`, `NSColorSampler` and pasteboard APIs behind `EditingHost`;
  `InspectHost` is the Linux double. Writing Tools selectors are availability
  guarded for macOS 14.

Decisions: N3 selects the spec's dark tokens/system insertion point over the
old hardcoded blue path/custom caret; N4/N6 retain unknown animator payloads and
all existing consumers; N7 selects object ids plus project/document/revision
stamps; N8 keeps native calls asynchronous and all v4 callbacks on their worker.
N1/N3 keep unsupported kernel commands as explicit failures, not apparent success.
`accepted(Ack)` checks `Reply::Throw`; a transport acknowledgment alone does not
mean a command was applied. No workspace dependency or lockfile change is needed.

**DESIGN-TODO(opus):** `m.gizmo_handle` named in S025 is absent from pm-design.
The provisional plan uses the spec-equivalent `m.keyframe - 2` (7 pt), with no
new token or geometry. All visual output remains provisional until Opus review.
The v4 SF Symbols and half-diamond have named neutral host placeholders; the
UI-kit/native renderer owner must render their specified glyphs, not text stand-ins.

Outside-owned-path integration still required:

1. App/viewer/inspector owners must instantiate these adapters, register actual
   native text editors and colour content/anchors, publish evaluated geometry,
   apply input/AX plans, route focus/IME/clipboard events and register the v4
   section. The existing temporary app panels are untouched beyond module
   registration. `EditingServices` is a queue contract, not an installed service.
2. Kernel text commands `AddAnimator`, `RemoveAnimator`, `SetStyleRange` remain
   `NotImplemented`; rename/duplicate/reorder/enable/property structural commands
   are also needed. The current property resolver lacks `ta.*`, `ts.*`, `g.*`
   and `mp.*` channels. Path create/add/close/delete and text/box edits are backed;
   the drag reducer alone is not an admitted vertex/tangent mutation.
3. The actor needs an atomic document-session guard for `Transact` and gestures.
   The adapter checks document id before enqueue and the actor checks revision,
   but close/reopen between those checks still requires a kernel-owned guard.
4. Bind native popover close/key callbacks to `Dismiss` (Escape cancel,
   outside-click commit, notation Return commit), reconcile acknowledgment notices,
   finish IME before canvas commit, and supply engine-derived text geometry and
   gradient interpolation. Render the picker body through the native UI-kit host.
   Full canvas editing (including empty/fresh text cleanup and tool restoration)
   is not supplied by the policy reducer alone. Font picker/type rows remain
   with the existing inspector/services owner, not replaced by the animator section.

Reproduce the Linux harness from the repository root (Rust 1.96.1, Bun 1.4.4):

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
mkdir -p work
rustc --edition 2024 apps/native/crates/app/src/surfaces/dialogs_check.rs -o work/check-dialogs
work/check-dialogs test
work/check-dialogs clippy
work/check-dialogs fmt
cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-kernel
```

The runner creates a disposable manifest in `work/dialogs-check`, pointing at
actual app-owned source and existing kernel/design/model crates. It seeds the
lock from `apps/native/Cargo.lock`; Cargo metadata prunes unused packages and
adds the harness root. Test and Clippy commands use `--locked`; checked-in
manifests/dependencies/locks do not change. The normal app command
`cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-native-app`
is blocked on Linux by the app's existing unconditional objc2/GPUI/Metal
platform dependencies, before these modules compile. The separate harness
avoids that platform dependency, not any test or document logic.

Current validation (Rust 1.96.1): **52 dialog tests passed**, **273 kernel tests passed**,
**1 existing kernel test ignored** (the opt-in real-profile test). Rustfmt and
all-target Clippy with `-D warnings` pass for the owned source. The AppKit module
also type-checks and passes all-target Clippy for `aarch64-apple-darwin` with
Zig's Apple-target assembler, without linking/running a Mac app. This is compile
evidence only. Both historical tags must be fetched for the existing kernel
parity test (`v1.2.0`, `v1.3.0`); Node is used by that development oracle only.

macOS still required: full app build, actual sheet/alert routing and parent
lifetime, Return/Esc and focus return, NumberField arithmetic/scrub, NSTextView
IME/native editing/selection and command routing, live validation, VoiceOver
reading/actions/announcements, multiwindow stale/reopen races, archive restart
and failure durability, light/dark/accessibility screenshots and Opus review.
The AppKit unit probe requires the main thread and explicitly reports a skip
when run on a normal Rust test worker; no native UI runtime pass is claimed.

Reconciled with `origin/feat/native-engine` at
`edc3c6e07164f6ac2bd71f1ef2405ff8d11520c3`, no conflicts. The commands
above were re-run after the final fetch/merge. No new crate or dependency was added, so
there is no checked-in lockfile delta to own. Full-app `cargo fmt -p
pm-native-app --check` also reports pre-existing formatting in the unowned
`app/src/smoke.rs` call to `media::for_session`; the touched registration and all
owned modules pass formatting checks. The app owner can format that call.

rustc --edition=2024 apps/native/crates/app/src/surfaces/documents_check.rs -o work/documents-check-tool
work/documents-check-tool
work/documents-check-tool --check-macos
cd apps/native
cargo test --locked -p pm-kernel
cd js/v4
bun install --frozen-lockfile
cd surfaces/documents
bun test
../../node_modules/.bin/tsc --noEmit -p tsconfig.json
../../node_modules/.bin/prettier --check index.tsx documents.test.tsx tsconfig.json
```

The portable driver compiles the **actual owned modules** against pm-kernel
and pm-services using a disposable `work/` manifest seeded from the workspace
lockfile. It runs `cargo test --locked --manifest-path
work/documents-check/Cargo.toml` (with an absolute manifest path), Clippy with
`--locked --all-targets -- -D warnings`, and formatting. `--check-macos` runs
`cargo check --locked` and Clippy for `x86_64-apple-darwin`; it is a type check,
not execution. The existing app manifest unconditionally links macOS UI, so
Linux cannot execute `cargo test -p pm-native-app`. No new workspace crate,
production dependency or production lockfile change is required.

macOS still must verify actual sheet ownership/queuing/placement, app-modal
fallback, terminate-later/cancel, open/save/import/folder filtering, extension
visibility and overwrite behavior, save/import races, tab transfer/snap-back,
popout restoration/focus, system AX announcements/labels/keyboard, and native
v4 material/placement rendering in every appearance/accessibility setting.
All visual output remains provisional until Opus review.

### Documents lane merge validation — 2026-11-01

Merged `origin/feat/native-engine` at
`edc3c6e07164f6ac2bd71f1ef2405ff8d11520c3` with no conflicts. The remote's
narrow fetch mapping initially left its tracking ref stale; an explicit refspec
fetch corrected it, and the final fetch/merge verified FETCH_HEAD and the
tracking ref both equal this commit. Only owned files differ from this base;
only the seven named design-coverage cells changed, each by appending text.
The production lockfiles match the merged base (including its upstream changes).

Post-merge results using the commands above:

- Documents: **53 Rust tests passed** (50 controller/kernel/lifetime tests and
  three real service-contract tests); zero failed/ignored. Linux Clippy
  `--locked --all-targets -- -D warnings` and owned-source rustfmt pass.
- v4 documents: **7 tests passed**, 60 assertions, including real Rust six-op,
  kit-diagnostic and accessibility/focus projection; TypeScript and Prettier pass.
- Kernel: `cd apps/native && cargo test --locked -p pm-kernel`:
  **273 passed, 1 existing ignored** (`smoke_real_store_copy_counts_only`,
  which requires a real-store copy and is intentionally not run on user data).
  Frozen v1.2.0/v1.3.0 reader-oracle tests pass after fetching their existing tags.
- macOS-target `cargo check --locked` and Clippy `--locked -- -D warnings`
  pass for the actual document modules/adapter on `x86_64-apple-darwin`.
  No AppKit/GPUI/Metal runtime or macOS visual test is claimed.
- `git diff --check`, append-safe coverage/ownership checks and staged gitleaks
  scans pass before commits. No new crate or production dependency was added.

One pre-existing whole-app formatting issue remains **outside owned paths**:
`cargo fmt --manifest-path apps/native/Cargo.toml -p pm-native-app -- --check`
reports the packed `for_session(&p.snapshot, &p.media, p.path.as_deref(), …)`
arguments at `apps/native/crates/app/src/smoke.rs:90`. That file is byte-identical
to the merged engine base; its owner should expand those arguments with rustfmt.
The portable lane driver checks every owned Rust source without changing it.

The clean-release regression verifies byte-identical external file contents
through the release itself and exact authored document/replayable history
through final kernel compaction. Existing `PrepareQuit` may append an identical
PMV4 record for a deferred journal; this lane does not claim physical-file
identity across that existing compaction operation. Watcher/reload integration
and the two Opus placeholders listed above remain open.

A repeated post-merge kernel run exposed one intermittent failure in the
existing `slice3::selection_is_atomic_ephemeral_deduplicated_and_published_before_event`
test (`slice3.rs:135`, first received event was not Published). Its isolated
locked rerun passed, and the subsequent complete locked kernel rerun passed
273/273 with the same one existing ignored test. Kernel sources/tests are
unchanged by this lane. The public actor code replies to Open before emitting
Opened; the test subscribes after awaiting that reply, so a late Opened event
can race its first-event assertion. The kernel owner should synchronize the
subscription/setup with Opened while retaining the Published-before-SelectionChanged
assertion. No test was ignored, weakened, or edited to obtain the rerun pass.

rustc +1.96.1 --edition 2024 apps/native/crates/app/src/surfaces/editing_check.rs -D warnings -o work/editing-check-setup
work/editing-check-setup
CARGO_TARGET_DIR="$PWD/apps/native/target" cargo +1.96.1 test --locked --manifest-path work/editing-check/Cargo.toml
CARGO_TARGET_DIR="$PWD/apps/native/target" cargo +1.96.1 clippy --locked --manifest-path work/editing-check/Cargo.toml --all-targets -- -D warnings
cargo +1.96.1 fmt --manifest-path work/editing-check/Cargo.toml --check
rustfmt +1.96.1 --edition 2024 --check apps/native/crates/app/src/surfaces/editing_check.rs
# Optional AppKit API/Clippy check without the kernel's SDK-dependent assembly:
work/editing-check-setup --native
rustup target add --toolchain 1.96.1 aarch64-apple-darwin
cargo +1.96.1 clippy --locked --manifest-path work/editing-native-check/Cargo.toml --target aarch64-apple-darwin -- -D warnings
cd apps/native/js/v4
bun install --frozen-lockfile
bun test surfaces/editing
bun x tsc --noEmit -p surfaces/editing/tsconfig.json
bun x prettier --check surfaces/editing
bun build surfaces/editing/index.tsx surfaces/editing/colour.ts --target browser --outdir ../../../../work/editing-bundle
```

The setup tool writes only disposable manifests/lockfiles under `work/`, seeds
resolution from `apps/native/Cargo.lock` and runs metadata for its own harness.
It compiles the actual editing sources and tests without copying/stubbing kernel
behavior. No new production crate or dependency is introduced. On macOS, run
`cd apps/native && cargo test --locked -p pm-native-app` and strict app Clippy;
Linux cannot compile the app's existing Apple-only dependencies. Cross-target
checking the isolated adapter is not an app build or a runtime/macOS test.

Mac verification remains: real canvas pointer transforms and modifiers, system
caret/IME candidate placement and marked text, context menu/Services/Writing
Tools per field, VoiceOver roles/labels/actions, focus return and shortcut
suppression, native menu/popup/sampler/pasteboard behavior, colour/gradient live
preview and cancellation (including foreign edits), and every light/dark,
contrast/transparency/motion combination. Opus reviews actual glyphs/layout and
all provisional output before visual acceptance.

**DESIGN-TODO(opus):** S034 names Shuffle but specifies no glyph. Its button
remains a named neutral placeholder; the lane does not select an SF Symbol.
N3 also selects a no-op hue edit that preserves an authored P3 colour exactly;
switching the picker space recomputes its plane without rewriting the colour.

Final reconciliation: merged `origin/feat/native-engine` at
`edc3c6e07164f6ac2bd71f1ef2405ff8d11520c3` without conflicts, then reran the
checks. **38 editing Rust tests passed; 277 v4 tests passed (19 editing);
283 kernel/design tests passed with one existing ignored kill-soak test**.
Kernel/design command:
`cargo +1.96.1 test --locked --manifest-path apps/native/Cargo.toml -p pm-kernel -p pm-design -- --test-threads=1`.
All other exact commands are above. Strict editing Clippy on Linux, isolated
AppKit Clippy for `aarch64-apple-darwin`, editing rustfmt/TypeScript/Prettier and
browser builds pass. Both staged gitleaks modes pass before each implementation
commit; the branch's implementation history is also scanned.

Pre-existing validation limits are retained: a parallel kernel run exposed the
`slice3::selection_is_atomic_ephemeral_deduplicated_and_published_before_event`
event-order race; the isolated test and complete serial rerun passed, including
after the merge. Whole-app `cargo fmt -p pm-native-app --check` reports existing
formatting in `app/src/smoke.rs:87`; it is outside this lane's ownership and is
unchanged. All new editing files and the module-registration edit are clean.
The requested Linux `cargo test --locked -p pm-native-app --no-run` stops in the
existing Apple-only `objc2` dependency; the Linux harness is the actual-source
substitute, not a claim that the app runs here. No Mac runtime/visual gate is
closed by these checks.
