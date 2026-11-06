# Main delta against the frozen native reference

## Scope and decisions — 2026-11-04

> **Superseded on scope, 2026-11-04:** the owner moved the parity target to `main` at `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. Every row here is now required work, not an optional delta. See p0-decisions.md.
>
> **Design:** the UI rows are specified in [design/surfaces/main-delta-ui.md](design/surfaces/main-delta-ui.md).

This is a source audit, not a change to the parity oracle or an implementation
plan sign-off. Frozen behavior remains **v1.2.0**
(`07ed5fa17ee640659cb075f9cad1af2095011d60`). The audited main tip is
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d` (2026-11-03).
`git log v1.2.0..origin/main` contains **138 commits**, including merge commits
and test-only changes. Native coverage is evaluated against integration commit
`ed5482622f41c533199a724dcb8c8738dd4dcc93`, with working-lane evidence identified
separately. These hashes pin the audit even if either branch advances.

Read together with [HANDOFF.md](HANDOFF.md), [p0-decisions.md](p0-decisions.md),
[C-MATRIX](c-matrix.md), [D-STATE](d-state.md), [W1](w1-workflows.md),
[keymap](w1-keymap.md), [traces](traces.md), [surfaces](surfaces.md),
[MCP](mcp.md), [engine service](engine-service.md) and
[first launch](first-launch.md). Source paths below are repository-relative;
inspect immutable evidence with `git show COMMIT:PATH` or `git show COMMIT -- PATH`.
The commit ledger accounts for every commit; expanded rows split broad commits
into independently owned user-visible changes. Merge ancestry and tests are not
counted as additional shipped features. Main source availability does not mean
a release containing it shipped.

| Decision | Selecting rule |
| --- | --- |
| Keep the v1.2.0 oracle; record subsequent behavior as additive deltas, including removed/superseded behavior | Recorded P0 reference decision; N3 exact authored intent. |
| Flag document fields, media bytes/references, saved workspaces, undo and profile formats as **HIGH** | N4 every project opens, preserve user data; N3 exact authored state. HIGH is an audit priority, not a new phase approval. |
| Count native primitives as partial coverage; require real application and physical evidence for completion | N1 pro-grade first, N2 better than shipped, N3 correctness. Linux models and documentation cannot prove Mac UI/rendering. |
| Route extension logic and synchronous services to V8/native recorder, panel presentation to remote DOM; headless/MCP to Rust | Recorded architecture; N6 v1–3 contract, N7 id-driven automation, N8 no Node/Chromium or main-thread JS waits. |
| Preserve data and old consumers even when main now cleans up or removes a feature | N4 archive, never delete; prune only after no consumer remains. Web-animation delivery has a separate recorded owner removal. |
| Record visual changes as evidence for Opus, without adopting colors, dimensions, copy or interaction decisions | User design ownership instruction; existing pm-design/specs govern native presentation. |

Coverage vocabulary: **Partial** means a named native primitive/spec exists but
complete behavior is unverified; **Gap** means the audited delta has no complete
native implementation/evidence identified; **Disposition** means a recorded scope
or design decision applies; **N/A** means no additional native runtime behavior.
No row asserts full native/macOS parity. Owner names below designate native areas,
not permission to edit their files or other branches.

## History reconciliation — 2026-11-04

**A commit in the range does not necessarily introduce a feature after the tag.**
The range includes side-branch recovery/integration history. Direct tag inspection
already finds captions, color depth, media tools, external MCP sessions, visual
replies, easing/simplification, PMV4 and dictionary-packed history. In particular,
`aff932bc2`, `16087057b` and parts of `bf26d9b3e` carry that earlier behavior.
The ledger records the action in each commit; the expanded inventory identifies
surviving tag-to-main differences and baseline carries. Use both comparisons:

```sh
git log --reverse --format='%h %s' v1.2.0..origin/main
git show COMMIT -- PATH
git diff v1.2.0 origin/main -- PATH
git show v1.2.0:PATH
```

**MCP name delta:** `project_memory` is the sole added literal tool name in
`main/agent-tools/spec.ts`; `external-spec.ts`'s seven names are unchanged.
Orchestration tools are imported separately, so literal counts are not the full
catalog count. Native's 67-tool fixture/catalog remains frozen and has no
`project_memory` body. [W1 A511/A512](w1-workflows.md) currently calls that member
shipped, although the actual tagged spec has no such member. Keep the discrepancy
visible: adding a native catalog member requires explicit reconciliation, not
rewriting the oracle. Schemas/descriptions and bodies still need comparison even
when tool names are unchanged.

**Keyboard delta:** the direct default-keymap diff adds only bare backtick
(`maximizePanel`). Cmd/Ctrl+Alt+Y for Adjustment Layer appears in `6e0fefabe`
but already exists at the tag. Clipboard paste changes the existing Cmd/Ctrl+V
route. Other changes are focus-local/context interactions; they are recorded
below rather than invented global bindings. J/K/L pro shuttle remains the
separate native cutover addition in W1, not a feature added on this main range.

## Native ownership and coverage evidence — 2026-11-04

| Area key | Native owner and evidence | Coverage boundary used below |
| --- | --- | --- |
| K | Kernel/project/history: [kernel](kernel.md), `apps/native/crates/kernel/src/{commands,project_io,session.rs,persistence.rs}` | PMV readers/writer, raw unknown-field preservation, locks, transactions/history and typed edits exist. A field surviving round-trip does not prove evaluation or UI. |
| E | Engine/media/export: [engine service](engine-service.md), [pro media](pro-media.md), [pro editing](pro-editing.md) | In-process workers, source/media/export/cache/color primitives exist. Full-project parity, physical A/V, codecs/HDR and production resources remain open. |
| X | Extensions: [C-MATRIX](c-matrix.md), [V8](s1-v8.md), [compat](compat-realm.md), [S5](s5-compat.md), [raster](recorder-raster.md) | Frozen helpers and additive async render wrappers exist; complete PM/backend/recorder/remote-DOM/Store comparisons remain open. No capability flag is accepted from its name alone. |
| B | Built-ins: [Inspector](panels/inspector.md), [Timeline](panels/timeline.md), [Viewer](panels/viewerchrome.md), [surfaces](surfaces.md) | Models and v4 trees exist. Nine live mounts on the built-ins lane have incomplete presentation/actions and no final Mac/Opus acceptance. |
| S | Shell/layout/OS: [shell](shell.md), [OS base](os-base.md), [surfaces](surfaces.md), [design](design/language.md) | Workspace/session/overlay primitives and approved specs exist; app input, IME, AX, menus, focus, docking and window lifecycle need Mac proof. |
| A | Agents/providers: [agents](agents.md), [replay](agents-replay.md), [agent panel](design/panels/agent-panel.md) | Rust provider state machines, persistence and workspace guards exist. Real providers, coordinator/tool lifecycle and strict replay identity remain open. |
| M | Rust MCP/headless: [MCP](mcp.md), [CLI](cli.md) | Frozen 67 schemas; concrete bodies/adapters are distinguished from 36 pending bodies. Engine/export adapters require actual hosts. |
| P | Profile/migration/storage/services: [D-STATE](d-state.md), [services](services.md), [first launch](first-launch.md), [migration](s6-migration.md) | Gzip/legacy readers, immutable checkpoints/media objects and archive-safe services exist. Full closure, reverse migration, real profile, signed Keychain/updater and UI remain open. |
| D | Opus/pm-design: [tokens](design/tokens.md), [surface inventory](surfaces.md), [reviews](design/reviews/) | Existing approved specs/tokens control presentation; descriptions of Electron visuals below are source observations only. |
| W | Public website/branding: `apps/www`, `assets/brand`, `packages/player` | Outside the native app implementation; preserve website consumers when pruning desktop export plumbing. |

Read-only working-lane evidence: engine `7ca9048a586c824fcbfa138f4ae93285b07a7087`
(`handoff/engine-macos.md`), corpus `4aa346755f7dd6b127eec7521e7c1ac8044978b5`
(`handoff/corpus-p1.md`), built-ins `e835967d12f944b4a6b2fba011783c0cdc903d1b`
(`handoff/builtins-live.md`), extensions `9253bc087b6ce8066a3426ff8693d4970a4365cf`
(`extcheck-p2.md`), pacing `55d396b62082671079f6006f43f782466c9037d6`
(`handoff/p120.md`), all beneath `docs/native` on their respective branches.
These are HANDOFF's recorded checkpoints, not claims that those branches were
merged by this lane. The notes retain AUDIO-2, incomplete engine replay,
26/58 historical project all-stage passes, 0/58 accepted extension matches,
incomplete built-in presentation/snapshots and unverified pacing/crash fixes.

## Expanded change inventory — 2026-11-04

Paths in this section are beneath `apps/desktop/src/` unless explicitly prefixed.
“Carry” identifies a behavior already present in the tag, even when a range commit
reintroduces or improves it. HIGH includes persisted profile data and changes to
authored values/references, not just a new file magic. Entries can share a commit.

### Projects, durable data and migration

| ID / priority | Commit(s) | What changed and source evidence | Native already covers it? / owner |
| --- | --- | --- | --- |
| P01 HIGH | `b7b302d1e`, `6a457a304`, `88b701c97`, `bf26d9b3e` | Bottom timeline dock; migrate project-saved workspaces on restore; rerun migration as version 4, later prototype/default-workspace migration 5 and timeline chrome schema 6. Saved groups/active tabs, Default/Animation names, Effects workspace and stock-only migrations preserve customized values. `legacy/core/workspace.ts`, `layout/model.ts`. Bottom docking is a tag carry; later schema/default changes survive. | Partial — K preserves workspace JSON; S/B models/specs cover layout. Complete historical migrations and user customizations need fixtures and Mac review. **2026-11-04 data lane:** Data/model `65c497123`: global and project workspace migrations 4–5, idempotence, saved groups and unknown/custom chrome retained; native schema-6 presentation remains with S/B/D. |
| P02 HIGH | `32ffd5432` | Adds `Comp.memory[]` and `memoryEnabled`; goal/constraint/decision/rejected/feedback/open entries with identity, author, reason, timestamps/resolution. One goal, bounded history, disabled state retains notes, autosave without edit-revision/Undo bump. Settings Memory page. `core/{memory.ts,types/project.ts}`, `settings/SettingsScreen.svelte`. True tag delta. | Partial — the data model now implements Memory indexing/mutation; Settings and actor/tool integration are still outstanding. A/M/S own behavior; frozen MCP lacks the tool. **2026-11-04 data lane:** Data/model `64528efad` + `6f53efa80`: lossless Memory fields, pure CRUD/index/list, disabled retention and bounded active history with archives; kernel actor/MCP/Settings binding remains. |
| P03 HIGH | `bf26d9b3e` | Ordinary disk imports become verified source links; normal Save/Save As retain links, while portable Project export embeds available media. Pasted/generated/converted bytes stay owned; missing links remain editable and cannot block unrelated saves. `main/library-media.ts`, `host/library-media.ts`, `legacy/core/{media,project-file}.ts`, `shared/library-media.ts`; compare `apps/desktop/docs/project-files.md` directly with tag. | Partial — K PMV4 and E/P source/proxy provenance exist. Native source resolution, durable import/replacement/Undo, save/export closure and rollback must prove both linked and embedded cases. HIGH even though PMV magic is unchanged. **2026-11-04 data lane:** Data/model `424c30c42` + `65c497123`: verified main disk media import, explicit source-link provenance, linked Save/Save As, portable receipts with offline IDs, owned fallback bytes and raw metadata retention; production admission/resolver binding remains. |
| P04 HIGH | `bf26d9b3e` | Disk gzip envelope `__powermoveDiskValue:1`, `encoding:gzip`, declared bytes and base64 data for project/history/session/journal/takes; decode bounds, lazy recompression and atomic queue. `main/{storage-codec,storage}.ts`. | Partial — K persistence/library and P first-launch decode/preserve gzip; real profiles, interrupted recovery and reverse handoff still need acceptance. **2026-11-04 data lane:** Data fixtures `424c30c42`: main gzip envelope exact decode, bounds/corruption failures and all five heavy value families exercised through existing codec; physical interrupted writes remain Mac work. |
| P05 HIGH | `bf26d9b3e` | One-time `storage-upgrade-v1.json` stages recovery/browser/source/cleanup, origin-specific browser progress, checksum-before-retirement, lease across windows, interruption retry, task progress and persisted completion acknowledgment. `main/storage-upgrade.ts`, `host/storage-upgrade.ts`. | Partial — P snapshot/checkpoint/verified-object primitives exist; Electron's exact upgrade receipt/state machine and task UI are not the native migration. Preserve old browser/conflicting bytes; never adopt deletion merely to imitate main (N4). **2026-11-04 data lane:** Data/model `65c497123`: origin-scoped receipt reader, verified-step/lease/retry/ack state machine and immutable capture of the actual receipt; native worker/receipt-store and task UI binding remains. |
| P06 HIGH | `bf26d9b3e` | Storage Settings reports categories/reclaimable bytes, automatic cleanup and backup budgets; retention/dedup rules change. Successful saves stop retaining five full backup generations; publication rollback copy is temporary, uncertain recovery survives. `main/{storage-management,backup-retention,project-files}.ts`, `settings/StorageSettings.svelte`. | Disposition + Partial — P services deliberately archive all generations (N4/N2); do not port deletion. Storage/task surface and recovery-version selection remain incomplete. **2026-11-04 data lane:** Retention `65c497123` + `424c30c42`: backups, uncertain recovery, pending association receipts and unused payloads are archived, never deleted; budgets/cleanup UI remains with P/S. **2026-11-05 data lane:** one exception, for owner review: a save that fails on a full disk deletes archived backup generations, oldest first, and retries. Active versions and retained originals stay. |
| P07 HIGH | `bf26d9b3e`, `70ffd6494` | Background autosave queue tracks per-project revisions, pending/failure/retry, unnamed recovery and named files; version listing/open-as-copy includes recovery/file/macOS sources. Export later waits for background save. `legacy/core/autosave-queue.ts`, `main/{project-files,native-versions}.ts`. | Partial — K journals/session/dirty/hash guards and P immutable backups exist. Queue/version UI, native NSFileVersion and export-save arbitration require app/Mac checks; preserve recovery on conflicts. **2026-11-04 data lane:** Data/model `424c30c42`: independent per-project autosave revisions, bounded debounce, failure/retry/cancel/stale completion and close/export flush barrier; worker/actor and NSFileVersion binding remains. |
| P08 HIGH (carry) | `16087057b`, `921687251`, `6cfdc59ae`, `87cdf3ea4`, `136f546d3` | Bounded large-document/media save/open, range-based PMV4 document, reliable whole-file hashing, transient macOS read recovery, async parsing/validation and lazy heavy store values. These are mostly baseline recovery/performance work; PMV4/PMV3/JSON support itself is already in the tag. `main/{project-files,project-document-validator,project-file-hash,storage}.ts`. | Partial — K has readers/checksums/streaming writer/history and P old-profile import. Validate large/low-space/concurrent saves and actual Mac filesystem behavior; never claim a new PMV5 format. **2026-11-04 data lane:** Formats `424c30c42` + `64528efad`: synthetic JSON, PMV3 and noncontiguous range PMV4 round-trip, whole-file/media hashing, future header/index metadata and checksum-validated archive ranges; Mac filesystem/large-profile qualification remains. |
| P09 HIGH (carry) | `f868f7901` | Composition-relative history dictionaries intern repeated layer/edit records while retaining forward/backward patches, Undo/Redo cursor and pending redo. Tag and main versions of `shared/history-memory.ts` are identical. | Partial — K history imports/unpacks dictionaries and exports logical history; P retains history-only media. Full real-history/rollback closure remains open. **2026-11-04 data lane:** History `424c30c42`: original dictionaries survive repeated saves; redo/Undo cursor replay and history-only media are tested; changed unknown/refused inputs remain archived. |
| P10 HIGH | `d8d887e0e` | Reopens join unchanged in-flight video conversion; each caller gets its own output, final consumer/crash cancels. If original moved/changed, provided owned bytes are stored rather than rejecting project open. Superseded recovery checkpoint is not reported as a failed save. `main/{media-proxy,library-media}.ts`, `legacy/core/projects.ts`. | Partial — E/P queues, source identity and proxy provenance exist. Native consumer-sharing and open/save binding need proof with original offline; proxy must never become an original. **2026-11-04 data lane:** Data/model `424c30c42`: source/recipe conversion coalescing, distinct consumer outputs, final-consumer cancellation and supplied owned-byte fallback; real conversion-worker binding remains. |
| P11 HIGH | `bf26d9b3e` | Close/Quit waits for imports, recovery and staged publication; failed flush keeps edits open, multiwindow cancellation preserves earlier approved work. Last-window close quits; failed/late preparation is retryable. `main/editor-close.ts`, `legacy/core/close-preparation.ts`. | Partial — K close/session and S/P lifecycle models exist; cross-window real app and signed termination checks remain. **2026-11-04 data lane:** Data/model `424c30c42`: flush barrier remains false on pending/failure/cancellation and never acknowledges newer edits with an older completion; actual close/quit/multiwindow host remains S/P work. |
| P12 HIGH (carry + UI) | `bf26d9b3e` | Home drops open media/projects without modifying previous document; nested local/cloud folder import, download/import task state; unsupported Figma paste offers informed cancel/fallback. Underlying Figma/import/easing algorithms already exist at tag; later presentation changes are mixed. `legacy/core/figma/`, `legacy/core/{folder-import,import-progress}.ts`, `library/LibraryScreen.svelte`. | Partial — K import metadata, E/P admission/queue and S library models exist. Real clipboard/Finder/cloud/Figma byte paths and task UI remain; never hydrate unavailable sources automatically. **2026-11-04 data lane:** Data fixtures `424c30c42`: nested folder/cloud/import metadata and prior-document references survive opaque round-trip; actual Finder/cloud/Figma admission and task UI remain unimplemented by this lane. |
| P13 HIGH | `929102722` | Removes saved export format `web` from `core/export-defaults.ts` and desktop web/React/player/agent-handoff output; website preview/copy now advertises Video/Images/Project and website player remains. `legacy/core/exporter.ts`, `player/`, `apps/www/src/lib/{AppPreview,Features}.svelte`. Existing project export preferences must remain recoverable even though the action is retired. | Disposition + Partial — recorded owner removal in P0/W1. K/P preserve original records/unknown fields, S/E remove only retired delivery, W retains player consumers; no source/project deletion (N4/prune). **2026-11-04 data lane:** Retention `6f53efa80` + `424c30c42`: retired web export preferences/unknown delivery fields round-trip without enabling the retired action; website/player consumers untouched. |

### Effects, rendering, media and exports

| ID / priority | Commit(s) | What changed and source evidence | Native already covers it? / owner |
| --- | --- | --- | --- |
| E01 HIGH | `bf26d9b3e` | Saved `Effect.version` (absent means 1), historical full `legacy` definitions, `effects.get(id, version)`, missing-version diagnostics and capability detection. Prevents updated built-ins from reinterpreting old artwork. `kernel/{api,effect-versions}.ts`, `core/types/project.ts`. | Partial — K `commands/effects.rs` carries definition revision and inputs; X frozen registry/backend and E renderer must prove exact revision resolution, legacy pixels and missing-provider behavior.  **2026-11-04 lane:** Version catalog and historical Gradient collision recorded; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 Mac hooks (partial):** Saving-file provenance now travels outside authored JSON through kernel snapshots and export children; version-specific native program keys and exact tagged pixel regressions added. Winning extension-history publication and all saved-revision/project coverage remain open. |
| E02 HIGH | `bf26d9b3e` | Static `Effect.inputs`: layer/stage (`source`, `masks`, `effects`), source/composition space; typed path/mask and image/LUT asset refs, four-input scheduling, stable remap on copy/delete/Undo, cycle/error diagnostics. `core/effect-inputs.ts`, `kernel/{api,install}.ts`, `legacy/gl/effect-path.ts`. | Partial — K typed input edits preserve refs; E graph/resources and X feature flags need complete producer/consumer gates. Unknown-data retention alone cannot render references.  **2026-11-04 lane:** Typed input validation and bounded request scheduler added; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 hooks2 (partial):** Composition-space producers and typed asset leases now execute; `metal_hidden_reference_stages_produce_before_the_consumer_and_cycles_fail` passes source/masks/effects and cycle pixels. General source-space/group/matte/backdrop and lifecycle closure remains open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** Source/composition leases now distinguish output size, scale and crop origin; `metal_reference_cache_keeps_each_precomp_density_independent` checks both child densities. Source-stage, mask-stage and effect-stage local rasters execute for supported 2D providers. General group/matte/backdrop policy and full lifecycle/remap conformance remain open. |
| E03 HIGH | `bf26d9b3e` | Layer `effectSpace`, owner/source rasters before transforms/opacity, off-composition bounds, source geometry/padding, exact subframe video/precomp input sampling and owner validity uniforms. `legacy/gl/{compositor,raster,effect-path}.ts`, `core/render-samples.ts`. | Partial — K `commands/layers.rs` accepts effect space; E source/evaluator/compositing foundations exist. Historical source-space fixtures, hidden sources, singular transforms, shutter phases and lease failures remain unqualified.  **2026-11-04 lane:** Exact source request identities and extent operations added; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 Mac hooks (partial):** Sized 2D owner matrices/resolutions now bind for versioned native programs. Source-space/3D/singular owners still refuse unavailable bindings; source-before-transform rasters and leases remain open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** Local source effects now run before inherited transform and opacity, with complete shape/path raster bounds, prefix padding/clipping, exact video/precomp times and additive shutter samples. `metal_source_reference_retains_hidden_offscreen_singular_content_and_stages`, `metal_source_precomp_subframes_seek_and_preview_export_are_exact`, `metal_source_video_subframes_keep_native_frame_selection_and_seek_identity`, and the source allocation/refusal tests supply focused evidence. Source temporal/brush domains, text/captions, arbitrary extension/3D rasters and complete lifetime/cancellation coverage remain open.  Allocation accounting `8a11a8f45` adds passing atlas/pass/asset/replay admission checks without changing the budget; full lifetime/cancellation qualification remains open. |
| E04 HIGH | `bf26d9b3e` | Spatial sampling/mip contracts, bounded cell grids, layer-space radii and viewport padding; source-dimension defaults, parameter disclosures and undoable color-swap actions. Adds `supportsVersions`, `supportsLayerInputs`, `supportsInputSubframes`, `supportsSpatialSampling`, `supportsSourceRasters`, `supportsSourceOwners`, `supportsTypedReferences`, `supportsBrushGeometry`, `supportsLutAssets`, `supportsOwnerValidity`, `supportsTemporalInputs`. `kernel/api.ts`; `extensions/effects-basic/index.ts` refuses activation unless every required flag and `parameterDefault` are present. | Partial — E GLSL/native resources, K effect commands and B/X controls are pieces. No complete flag-by-flag native conformance identified; report only a demonstrated capability (N3/N6).  **2026-11-04 lane:** Source-tested mip/grid/default calculations added; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 hooks2 (partial):** Native mip/grid binding is exercised by `metal_mosaic_cell_atlas_and_wide_gaussian_mips_preserve_reference_values`; source owners, full defaults/actions and complete capability conformance remain open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** Local source owner/resolution/validity uniforms and per-prefix spatial domains now bind. No capability flag is promoted; complete defaults/actions, source brush/temporal and flag-by-flag conformance remain open. |
| E05 HIGH | `bf26d9b3e` | New/revised color families: Tint + Swap Colors, Channel Mixer, Invert/Levels revisions, Levels (Individual Controls), Threshold, Shift Channels, Exposure, Tritone, Color Balance (HLS), Brightness & Contrast, Black & White. Straight/premultiplied alpha, signed HDR and channel persistence matter. `extensions/effects-basic/{color-correction,core-image,tonal}.ts`. | Partial — E color/GLSL/effects and B controls exist; X must load correct definitions and E verify each historical/current revision at working depth. Old Invert/Levels already existed at tag.  **2026-11-04 lane:** All color-family definitions and assembled shaders retained; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 Mac hooks (partial):** Current scalar color programs now use the versioned renderer path and sized 2D owner bindings, with exact Invert/identity pixel regressions. Complete family/depth/alpha/HDR qualification remains open. |
| E06 HIGH | `bf26d9b3e` | Gaussian Blur and Mosaic revisions, Unsharp Mask, Find Edges; wide kernels and layer-local cells preserve edges/HDR at cropped zoom. `extensions/effects-basic/spatial.ts`. | Partial — E spatial/GLSL support; complete per-revision source/preview/export pixel qualification remains.  **2026-11-04 lane:** Spatial-family shaders and sampling calculations retained; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 hooks2 (partial):** `metal_mosaic_cell_atlas_and_wide_gaussian_mips_preserve_reference_values` passes real cell integrals and wide HDR Gaussian constants. Complete revision/crop/source/export qualification remains open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** `metal_source_padding_clips_each_prefix_before_later_spatial_passes` exercises a padded local blur after an alpha-writing prefix and exact cropped output. Full spatial revisions, working depths and decoded export qualification remain open. |
| E07 HIGH | `bf26d9b3e` | Revised Drop Shadow and Gradient Ramp: layer-coordinate softness/offset, Shadow Only, source-sized ramp/reset points and historical versions. `extensions/effects-basic/shadow-ramp.ts`. | Partial — E native shader path and K channels preserve authored values. Corpus still has historical generated shader failures; no completed full-project proof.  **2026-11-04 lane:** Both revisions retained; historical Gradient provenance remains ambiguous; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 Mac hooks (partial):** Tagged Gradient/Shadow pixels retain their original programs; current Gradient receives layer-coordinate bindings. Full shadow spatial/mip binding, contextual defaults and reset/swap integration remain open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** Local solid/shape ramp coordinates, raster margins and inherited parent rotation have named Metal tests in `advanced_effects`. All shadow revisions, contextual defaults and reset/swap application acceptance remain open. |
| E08 HIGH | `bf26d9b3e` | Set Matte, Displacement Map, Compound Blur, Texturize, Set Channels, Blend, Calculations and Channel Combiner, including later displacement/combiner revisions. Reference stages/spaces, hidden sources, channel/alpha math and four-source scheduling. `extensions/effects-basic/{layer-effects,compound-blur,texturize,set-channels,layer-compositing}.ts`. | Partial — K inputs and E graph/render primitives; no complete consumer-family native acceptance identified. Missing inputs/cycles must remain failures/diagnostics.  **2026-11-04 lane:** Consumer shaders and typed references retained; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 hooks2 (partial):** Hidden source-stage, mask-stage and effect-stage Set Matte pixels pass `metal_hidden_reference_stages_produce_before_the_consumer_and_cycles_fail`; the complete consumer families and four-source qualification remain open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** Set Matte now consumes supported unclipped source-space 2D rasters across source/masks/effects stages, including hidden/off-composition/singular providers. Complete consumer families, four-source and group/matte/backdrop qualification remain open. |
| E09 HIGH | `bf26d9b3e` | Stroke typed animated path/mask geometry, brush hardness/spacing/trims/sequential masks; Apply Color LUT loads durable `.cube` assets with 1D/3D domain interpolation. `extensions/effects-basic/{stroke,apply-color-lut}.ts`, `core/cube-lut.ts`. | Partial — E/P LUT/pro-media jobs and K references exist. UI, exact Stroke behavior and durable LUT media/Undo/export closure still require integration.  **2026-11-04 lane:** Brush/LUT definitions and typed references retained; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 hooks2 (partial):** Typed LUT and image pixels pass `metal_typed_lut_1d_3d_domain_alpha_replacement_and_missing_lease` and `metal_typed_image_samples_the_reference_texturize_slope`; typed soft mask and saved hard brush cases have named tests in `advanced_effects`. Full animated path/mask, durable import/Undo/export closure remains open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** `metal_typed_stroke_reads_provider_geometry_in_the_consuming_owner_space` proves geometry evaluation on the provider with placement through the consuming owner. Source brush-domain expansion, complete animated geometry and durable LUT/Undo/export acceptance remain open. |
| E10 HIGH | `bf26d9b3e` | Color Profile Converter (explicit D65/transfer/clipping) and signed HDR Compander; Channel Combiner's multichannel conversion revision. `extensions/effects-basic/{color-transforms,layer-compositing}.ts`. | Partial — E color management is stronger than SDR-only output but needs each saved definition/version and decoded output comparison. Do not blindly copy browser clipping (N3).  **2026-11-04 lane:** Rust profile/compander helpers and main value oracle added; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 hooks2 (partial):** `metal_profile_converter_pixels_match_the_executed_main_value_oracle` passes all 216 source values with clip/amount variants; `metal_hdr_compander_keeps_signed_color_and_alpha_at_saved_depths` passes signed HDR. Complete saved-depth/combiner/export qualification remains open. |
| E11 HIGH | `bf26d9b3e` | Echo bounded owner-prefix temporal samples, seven operators, historical masks/references, multiple echo convolution and reverse-seek determinism. `extensions/effects-basic/echo.ts`, `kernel/api.ts`, `legacy/gl/compositor.ts`. | Partial — E time/graph/worker machinery exists; full exact prefix/seek/hidden-source/cancellation/memory native acceptance remains.  **2026-11-04 lane:** Echo shader, exact times/prefix keys and bounded scheduler added; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 hooks2 (partial):** `metal_echo_exact_prefix_subframes_seven_operators_and_reverse_seek` passes exact seven-operator prefix/subframe/seek pixels. General source owners, convolution, cancellation and full memory/lifecycle closure remain open.  **Source-raster implementation `23ea75d89` (Partial/OPEN):** Source-owner shutter samples use exact local times and additive accumulation. Source Echo prefix/domain expansion, general convolution/cancellation/memory and complete temporal lifecycle evidence remain open. |
| E12 HIGH | `bf26d9b3e` | `continuousRaster` is formalized in saved layer/type/command fields; precomp raster at displayed size, sharper scaling; Release Pre-comp/layer switches exposed in AE-style UI. `core/types/{project,commands}.ts`, `legacy/core/compositions.ts`. Continuous-raster rendering tests already exist at tag; this is a mixed API/UI delta. | Partial — K layer patch and E precomp rendering exist. B UI plus picture/rig-preserving release/Undo and real output need acceptance.  **2026-11-04 lane:** No continuous-raster output closure claimed; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 Mac hooks (partial):** The saved continuousRaster switch now selects the existing display-density precomp raster; a real output/child-size regression was added. Release/precomp rig preservation, Undo and UI acceptance remain open. |
| E13 HIGH | `cc6adc885` | HDR/log import interpretation: MP4 nclc/video tags, PQ/HLG tone-map to tagged SDR preview, BT.2020 log code values retained for grading; GPU helper with FFmpeg fallback, opaque-vs-alpha routing. `shared/video-color.ts`, `legacy/core/video-preview.ts`, `main/media-proxy.ts`, `packages/macos-haptics/src/hdr-proxy.m`. | Partial — E native color/source metadata/proxy path exists. Native spec and authored color intent govern (N3); verify tagged originals, grading, EDR/SDR/export and fallback on Mac. Main's SDR proxy is not a universal native HDR requirement.  **2026-11-04 lane:** Source-tested legacy preview interpretation helper added; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 Mac hooks (partial):** Real Metal video-reading pixel suite passes for native HDR/alpha fixtures and viewer/export parity at existing depth bounds. Physical EDR/SDR and complete helper/fallback import qualification remain open. |
| E14 HIGH | `cdab66cb4` | Sequential MP4 sample decode with B-frame/edit-list timing, fallback when WebCodecs cannot reproduce media; bounded encoder/IPC queue, VideoToolbox capability probe/fallback and matching color tags. `shared/mp4-{boxes,samples}.ts`, `legacy/core/export-video-decoder.ts`, `main/render-encoder.ts`. | Partial — E native decode/export workers supply an alternative implementation; exact frames/timestamps/color and cancellation/backpressure need same-source output comparison. No WebCodecs/Chromium port (N8).  **2026-11-04 lane:** No sequential decoder closure claimed; [integration remains open](../engine/main-delta-fx.md). **2026-11-05 Mac hooks (partial):** Added same-source B-frame and edit-list sequential/random-access qualification using native backends. Bounded encoder/cancellation/VideoToolbox delivery coverage remains open; no whole-row closure claimed. |
| E15 | `cdab66cb4` | Long `export_video` adds optional `jobId` to its input schema and returns `{status:"running", jobId, progress}` after about 20 minutes; a call with only that ID resumes waiting, a second export is refused, completed/error results are consumed. Success adds stage timings and errors identify decode/render/encode/audio/finalization stage. `main/agent-tools/{bridge,spec}.ts`, `legacy/assistant/mcp-project-tools.ts`, `legacy/core/export-stats.ts`. Tool name is unchanged; schema/body/result behavior changes. | Partial — M export host adapter exists; resumable job state/wait/cancel/client-disconnect behavior is not proven by schema identity. A/E/M own coordinator. |
| E16 (carry + extensions) | `16087057b`, `921687251`, `bf26d9b3e` | Source warmup, static preview/export reuse; opaque/alpha PNG/ProRes/WebM routes, async readback guarded against changed quality/provider/revision, GPU loss and stale ACKs. Asynchronous GPU readback is distinct from the public render API. `legacy/core/exporter.ts`, `legacy/gl/compositor.ts`. Much of the optimization already exists at tag. Neither tagged nor audited-main `kernel/api.ts` declares `renderFrameToAsync`, `snapshotAsync` or `rasterAsync`; P0 separately adopts them as a native superset, not evidence that this main range delivered them. | Partial — X `js/compat/src/extensions.ts` provides async wrappers, E export workers/alpha conversion exist. Complete native recorder/resource/pixel behavior and output parity remain open. Sync contract stays intact (N6). |
| E17 | `70ffd6494` | Disk-backed original/preview URLs replace whole-file blob copies; ranged media server and per-file-state link validation; idle decoder/GPU eviction with fast-scrub cap, two preview installs at a time, macOS pressure into caches. `host/streamed-media.ts`, `legacy/core/video-residency.ts`, `main/{library-media,memory-pressure}.ts`. | Partial — E source/cache/decoder jobs and P media objects support this architecture. App pressure/eviction/seek/hidden-source recovery and measured memory/playback still need Mac/full-project proof. Main's reported memory reduction is not native evidence. |
| E18 | `70ffd6494`, `c7f46a056` | Stop hidden panels/mixer/status/layout/style churn in playback; return invalidated preview framebuffers to pool so warmed numeric scrubs allocate no target each step. `layout/{PanelPool,Dock}.svelte`, `mixer/meter-loop.ts`, `legacy/gl/compositor.ts`. | Partial — B/E bounded paints/pools and engine service exist. Actual pacing, CPU, allocations and edit-to-glass gates remain open; do not translate CSS workarounds into native requirements. |

### Panels, editing controls and OS interaction

| ID / priority | Commit(s) | What changed and source evidence | Native already covers it? / owner |
| --- | --- | --- | --- |
| U01 HIGH | `989024400`, `b7b302d1e`, `bf26d9b3e`, `4f5cf623d` | Early Blender-style corner split/join/dock/replace (tag carry) evolves to AE tab stacks and above/below drop zones; tab drag moves one panel, header moves stack, backtick maximizes pointer panel. Persist groups/active tab; column sizing/scroll and titlebar fullscreen tools change. `layout/{area,drag,dock-zones,model,maximize}.ts`, `keymap-default/keymap.ts`. | Partial — S workspace/docking/keymap and B trees exist. No accepted full gesture/maximize/restore/IME/AX Mac proof; D specs win over main layout. |
| U02 | `1861cf88c` | Visual point controls use authored independent X/Y increments or range-derived defaults, anchored quantization; missing parameters don't create rows; generated panel grouping honors actual controls. `controls/PointField.svelte`, `inspector/parameter-layout.ts`, `panels/GeneratedPanel.svelte`, `legacy/assistant/spatial.ts`. Core point behavior is largely a tag carry. | Partial — B numeric/point binding models exist. Exact authored increment semantics and generated panel grouping need X/B comparisons. |
| U03 HIGH (carry) | `ce36c67d8`, `aff932bc2` | Existing paste chord accepts clipboard image and Finder media, dispatches durable import/placement, text focus retains native paste; local layer/key/effect copy supersedes old system media. `main/media-paste.ts`, `legacy/core/media-paste.ts`, `legacy/ui/shortcuts.ts`. | Partial — S clipboard/OS inventory and E/P import primitives; real pasteboard ownership/ordering/cancel/focus/media Undo remain Mac gaps. |
| U04 HIGH (carry + UI) | `9148c961b`, `ff5181601`, `bf26d9b3e`, `4f5cf623d` | Track parent pickwhip and live property values/drags; AE Layers column headings, switches (3D/raster/audio), Link Layers, section/property rows, resizable list, timecode/empty comp, composition path and grouped Project/Effect Controls. `extensions/timeline/{track-view,timeline,comp-tabs,property-sections}.ts`. Parent links are transform parenting, not linked A/V. | Partial — K transforms/parent edits, B timeline/Inspector models and E evaluator. Real pointer/keyboard/graph actions and rendered state remain incomplete. |
| U05 HIGH (carry + UI) | `6e0fefabe`, `b8fcb70ae`, `e8f95f1eb` | Negative-time layers/keys and graph/cache improvements; numeric key time extends beyond strip with neighbor collision guard; opened groups reveal descendants' keyframes in layers mode. `extensions/timeline/timeline.ts`, `legacy/core/anim.ts`. Negative-time support already exists at tag; group disclosure is a surviving UI delta. | Partial — K keyframe/time/lock commands and B timeline graph/reveal models; exact numeric validation, disclosure and native gestures require acceptance. |
| U06 | `c0e47ac20` | Selected keyframes become visually distinguishable in timeline and graph. `extensions/timeline/timeline.ts`. | Partial — B selection models and D timeline spec exist; Mac/Opus visual acceptance remains. No new palette chosen here. |
| U07 HIGH | `8eb092ee8` | Per-axis diamonds key only that axis; group diamond keys all. Timeline/Effect Controls diamonds replace stopwatches. New undoable `separateDimensions` defaults true, false joins Position keys by time while retaining per-axis values. Panel-library titlebar button returns. `inspector/ChannelRow.svelte`, `timeline/timeline.ts`, `core/types/project.ts`. | Partial — K layer patch supports `separateDimensions`; B channel/timeline actions and grouped-time semantics need full presenter and Undo/save/reopen proof. |
| U08 | `ff5181601`, `07a270030`, `bf26d9b3e`, `0e1af9e60` | Add Effect moves from section content to Layer Effects header, later explicit Add Effect in Effect Controls and + in Properties. Pointer and Enter creation remain; Inspector row well/group/disclosure and full-column fit update. `extensions/inspector/{index.ts,EffectsPanel.svelte,EffectsSection.svelte}`, `kernel/api.ts` (`fitContent`, menu contributions). | Partial — B Inspector/Effect Controls adapters exist; actual buttons/menu dispatch/keyboard/selection effects need Mac/Opus acceptance. |
| U09 | `81ed886fa` | Central tooltip delay/neighbor transition, parsed hint/shortcut, edge-aware placement, hide on input/scroll/leave, keyboard focus, accessible description and dynamic title tracking. Iframe titles retained. `controls/tooltips.ts`, `css/app.css`. | Partial — S/B UI-kit tooltip and AX primitives; final focus/placement/timing/remote-DOM behavior needs Mac review. Browser title suppression itself is not a native requirement. |
| U10 | `06d25cf64`, `8439c2ed1` | App-drawn cascading menus/selects/button popovers with type-select and press-drag-release; then text fields return to AppKit Writing Tools/Services/Look Up/spelling. Later commit restores shortcuts, dims unavailable actions and avoids repeated icons, superseding initial hide/shortcut policy. Context menus distinguish graph/ruler/lanes/keyframes/layers/canvas/media/easing/tabs/text animators; shared New order, effect category submenus. Menu bar Show All/Delete/Help; remove duplicate New Window. `overlays/{Menu.svelte,menu-rules.ts}`, `main/{menu,context-menu,text-context-menu}.ts`. | Partial — S menus/native text and B action models exist. AppKit services/shortcut precedence, nested menus and AX require Mac. D approved design overrides copied macOS measurements. |
| U11 | `ba5a8742c`, `bf26d9b3e`, `4f5cf623d` | Panel/sheet material, neutral dark chrome, title-case dialogs, aligned values; common header/row metrics, type/font/axis wells, titlebar tools/project title and in-place rename. `css/app.css`, `packages/tokens/tokens.css`, `shell/Titlebar.svelte`, Inspector/overlays. | Partial / Disposition — B/S models and D tokens/specs. Main visual choices are provisional source evidence, not instructions to alter pm-design. |
| U12 (carry + presentation) | `bf26d9b3e` | Easing curve editor/library/preset import-export, standard/multikey/vector and motion-path tooling; Simplify only on explicit user action with its own Undo after dense edits, preserve locked/expression/unknown tracks. Most algorithms and `simplify_keyframes` already exist at tag; newer menus/controls/exports are mixed. `panels/{EasingPanel.svelte,easing/}`, `core/anim/`, `legacy/core/keyframe-simplification.ts`. | Partial — K keyframe/temporal/handles/simplify commands and M planner exist; S/B Easing surface, presets and real evaluator proof remain incomplete. Preserve authored keys until explicit Simplify (N3/N4). |
| U13 HIGH (carry) | `16087057b`, `aff932bc2` | 8/16/32-bit working precision/settings; embedded/nested audio gains/mute and composition output gain, Mixer strips/undoable faders/solo/meters; source-time caption cues, captions inspector/timeline/edit/export/sidecars; transcription model selection/download/cancel/cache and onboarding handoff. `core/types/project.ts`, `captions/`, `transcription/`, `mixer/`. These already exist at tag, not newly discovered main features. | Partial — K caption command declarations (their planner still returns `NotImplemented`) and audio edit primitives, E `pm-captions`/audio, B cue/Mixer models; transcription/model host, complete UI and MCP caption generation remain open. N4 preserves all cues/words/gains/model preferences. **2026-11-04 data lane:** Data `6f53efa80` + `424c30c42`: precision, exact cue/word times, nested animated audio gain/mute, output gain and model preferences round-trip; renderer/transcription/caption UI and MCP bodies remain with their owners. |
| U14 HIGH | `5458b5449` | Anchor Point pointer press selects/narrows to hit layer and same drag moves its anchor; plain click only selects, including previous multiselection. `legacy/ui/shortcuts.ts`. | Partial — B viewer/K transform gestures exist. Exact hit/selection/drag/cancel behavior needs native pointer and authored-transform test. |
| U15 (carry) | `921687251`, `bf26d9b3e` | Loading ghost/progress, cooperative large-project open, tab-choice cancellation/join, active-project-only restoration, crash recovery; font-registry invalidation, idle 3D gizmo/status polling, video posters without seeking viewer, animated multiline text geometry. Most baseline source remains unchanged; later shell presentation/performance differences survive. | Partial — K/S lifecycle, E text/font/media/cache and B publication models; first-open/restore/crash, real fonts and full content/presentation acceptance remain. |
| U16 | `a69ff8fe7`, `bf26d9b3e` | Orange playhead brand/icon assets, font assets, renderer/cloud/website identity. Main range includes recovered assets already in tag plus surviving website/app presentation changes. `assets/brand`, desktop icons/CSS, `apps/www`, `apps/cloud/src/page.ts`. | Disposition — D owns native identity/icons/type; W/cloud owners own other products. No new assets or visual choices in this audit. |

### Agents, extension lifecycle and automation

| ID / priority | Commit(s) | What changed and source evidence | Native already covers it? / owner |
| --- | --- | --- | --- |
| A01 | `25d92afc5` | Provider presets grouped local/API; model discovery before connect filters non-tool models, discovers local servers; saves keys per address; actionable errors and Ollama low-context warning. Onboarding/connection gates and select optgroups updated. `main/compatible-provider.ts`, `shared/compatible-provider.ts`, `legacy/ui/compatible-settings.ts`. | Partial — A provider process/state and P Keychain primitives exist. These preset/discovery/address-secret/error/context behaviors and live provider/UI conformance remain gaps. Preserve saved aliases/keys. |
| A02 HIGH | `25d92afc5` | Settings Accounts installs/removes MCP entry for Claude Code, Codex, Claude, Gemini CLI, Cursor, VS Code, Windsurf, LM Studio and OpenCode; generic Copy setup; preserve hand-written/commented config and intentional removal. `main/agent-clients.ts`, `settings/AgentClientsSettings.svelte`. | Partial — M native transport/descriptor and P external config preservation exist. Exact per-client ownership/receipts/config round-trip and installed signed executable need Mac proof. No Node launcher in app (N8). |
| A03 | `4f5cf623d` | Claude Project Auto now uses Claude Code auto permission classifier. `main/claude/{adapter,runner}.ts`. | Gap — A documentation still says Claude Project asks under both modes. Must reconcile provider authority and pinned runtime support before advertising Auto. |
| A04 | `7cb061739` | Richer streaming markdown: nested/task/ordered lists, quotes/headings/fences/links and inert unsafe HTML/URLs; local image/video/audio attachments under prose with byte-range playback and safe file grants, remote images opt-in; finished work folds separately from reply, duration timer survives steering. `panels/agent/{markdown,work-groups}.ts`, `main/agent-media.ts`. | Partial — A conversation/events/persistence and B agent panel spec exist. New parser/local-media grants/grouping/timer semantics are not established by frozen replay; S/B/A presentation and safe playback integration remain. |
| A05 (carry + UI) | `dc2ff22fc`, `aff932bc2`, `bf26d9b3e` | Conversation visibility/drag/pin/resize, streaming recovery and Codex connection state; visual HTML replies publish/resize/expand/replay in durable threads with inert origin/CSP and no privileged bridge. Earlier core already ships at tag; later layouts/attachments vary. `panels/agent/`, `main/codex/`, `shared/visual-reply.ts`. | Partial — A persistence/protocols and B views; M `publish_visual_reply` is still pending. Stored HTML and artifacts must survive even before a native safe viewer exists. |
| A06 HIGH (carry) | `ff5181601`, `aff932bc2`, `2c64c9e0b` | External stdio MCP/headless sessions, desktop registration; project/library lifecycle, timeline strip gestures, direct media list/import/replace/manage, inspection/sample/contact-sheet/waveform/transcribe/check/caption tools; saves and exports without chooser. These tool names/basics already exist at tag. New descriptions/result/lifecycle behavior still needs auditing. | Partial — M has 67 frozen schemas and K bodies, E adapters; pending import/replace/activation/agent/transcription/panel bodies stay explicit errors. Native Rust headless replaces Chromium host (N7/N8), not its implementation quirks. |
| A07 HIGH | `32ffd5432` | Adds MCP `project_memory` list/add/update/remove with kind/reason/resolution/limit and memory index in `get_project_state`; disabled memory hidden from agents but retained. `main/agent-tools/spec.ts`, `legacy/assistant/harness.ts`. | Gap — M catalog has no name/body, K only opaque retention; A must preserve non-Undo autosave semantics. This is the confirmed name delta; resolve W1 inventory disagreement before extending freeze. |
| A08 HIGH (carry) | `ff5181601`, `aff932bc2` | Themes own reversible appearance, overlapping unload order/restart/user edits; workspace mutations distinguish temporary appearance from saved panel membership. Fork rebase uses pristine base/current/upstream, compile/activate/Undo and agent conflict fallback. `kernel/{theme-state,workspace-appearance}.ts`, `main/extensions/{rebase,update-fork}.ts`. Baseline core already exists. | Partial — X frozen helpers/registry lifecycle, K state/external history, P archived installs exist. Complete rebase coordinator and real extension lifecycle/DOM comparisons remain. |
| A09 HIGH (carry + later fencing) | `670b563dd`, `a889a5e78`, `e5bbfb677`, `bbf2083b5`, `908c89091`, `bf26d9b3e` | Recover sockets/sessions after renderer loss; reject delayed authorization/publication after disconnect or document replacement; serialize publication/recovery; fence finishing sessions and panel generations between capture and input. Earlier fixes often match tag; panel generation/finishing changes survive. `main/agent-tools/`, `legacy/assistant/{harness,panel-tools}.ts`. | Partial — M/K session document/revision guards and A cancellation exist. Complete application queue ordering, extension publication, late replies and panel input/capture remain open. |
| A10 HIGH (carry) | `aff932bc2`, `cf86afe39`, `864b9cc35`, `d9461e878` | Original provider/settings retained during frame review, capture failures preserve applied edits; bounded instructions, explicit group coordinates/composition-wide ordering; supervised outside-sandbox command uses person's approval; teardown keeps original error. `main/{codex,agent-tools}`, `legacy/assistant/harness.ts`. | Partial — A protocol/workspace/authority and M/K guards exist. Live provider review, user consent and strict error/event identity are unverified. |

The remaining small behavioral fixes are enumerated individually in the ledger
below; they are not hidden inside these families. Every ledger row supplies its
native coverage and owner, including baseline-equivalent repairs.

## Complete commit ledger — 2026-11-04

The **100 source/documentation commits** below include baseline carries,
new deltas, web-only work and a proposal; they are not a feature count.
Each native area key resolves to the evidence/remaining gates above. HIGH marks
persisted state, authored-data edits or data-protection behavior.

| Commit | Priority | User-visible action / disposition | Native coverage / owner |
| --- | --- | --- | --- |
| `989024400` | HIGH | Area-corner split/join/dock/replace and restorable hidden panels (baseline carry; later stack workflow supersedes presentation). | Partial S/B; see U01 |
| `1861cf88c` | HIGH | Respect authored per-axis increments, missing parameters and generated panel grouping. | Partial B/X; see U02 |
| `ce36c67d8` | HIGH | Clipboard images/Finder files paste through import; preserve text-field paste and local copy precedence. | Partial S/E/P; see U03 |
| `b7b302d1e` | HIGH | Bottom timeline dock, geometry/splitter/API dock support and saved-workspace migration (baseline carry). | Partial K/S/B; see P01/U01 |
| `6a457a304` | HIGH | Run migration when restoring a project's saved workspace as well as the global workspace. | Partial K/S; see P01 |
| `88b701c97` | HIGH | Bump bottom-dock migration to version 4 so earlier migrated workspaces are revisited. | Partial K/S; see P01 |
| `9148c961b` | HIGH | Track clips gain parent pickwhip and corrected target/live drag state. | Partial B/K; see U04 |
| `ff5181601` | HIGH | Header Add Effect, live track fields, speed-duration preservation, theme ownership and external MCP/headless session work; broad baseline recovery, expanded below. | Partial B/X/A/M; see U08/A06/A08 |
| `dc2ff22fc` | Normal | Conversation/header dragging, connection/stream recovery, stdin/stdout close and MCP disconnect handling. | Partial A/B/M; see A05 |
| `6e0fefabe` | HIGH | Graph cache/performance, negative-time layer/key editing, adjustment creation/key binding and color-depth edits (baseline carries). | Partial B/K/E; see U05/U13 |
| `16087057b` | HIGH | Working color depth/precision, warmup, retained static exports, source preparation and bounded large-project saves (baseline carries). | Partial E/K/P; see P08/E16/U13 |
| `aff932bc2` | HIGH | Gather captions/transcription, Mixer/audio gains, media tools/import/replace, workspace transfer/fork updates, agent review/approval/pinning and website work without new 3D system; largely already in tag. | Partial A/M/E/K/B/X; see A05/A06/A08/U03/U13 |
| `32ffd5432` | HIGH | Typed durable project memory, Settings Memory and project_memory tool; confirmed tag delta. | Gap K/A/M/S; see P02/A07 |
| `329c8d2f3` | Normal | Preserve Discord/community link in website Header; tag-to-tip file is unchanged. | N/A W; see U16 |
| `154dbf134` | Normal | Restore website description/community link and stable hero assets/preview; recovered tag content. | N/A W |
| `cf86afe39` | Normal | Integration reconciliation: provider display/model compatibility, bounded agent instructions and combined editor/tool wiring. | Partial A/M; see A10 |
| `67c56dae6` | HIGH | Keep saved Claude model aliases in picker rather than silently replacing a selected model. | Partial A/S |
| `07a270030` | Normal | Refresh combined built-ins; effect creation works by both pointer and keyboard. | Partial B/X; see U08 |
| `6cfdc59ae` | HIGH | Carry transient macOS project-read retry/recovery into current save path. | Partial K/P; see P08 |
| `87cdf3ea4` | HIGH | Keep dependable file hashing; exclude live renderer build artifacts from packaged app. | Partial K/P; see P08 |
| `2c64c9e0b` | HIGH | Recover paged timeline strip gestures, ordering/duplicate/state/trim/slip semantics and independent library lifecycle tools; mostly tag carries. | Partial K/M/A; see A06 |
| `2f3e9ae41` | Normal | Website preview uses existing public image assets; no screenshot-content change to native. | N/A W |
| `921687251` | HIGH | Broad loading/save/crash/preview/font/text/startup work, later performance and shell changes; baseline behavior is separately identified. | Partial K/E/P/S/B; see P08/U15 |
| `7320393a0` | HIGH | Apply appended/replaced key easing presets only after final neighbors exist; preserve curves/Undo and last edit at a snapped frame. | Partial K/E |
| `670b563dd` | HIGH | Reconnect external MCP sessions/socket host after renderer loss without admitting a stale renderer/document. | Partial M/A/K; see A09 |
| `864b9cc35` | HIGH | Clarify grouping preserves playhead world pose and reorder indices are composition-wide; sibling anchors retain membership. Production guidance equals tag at tip. | Partial K/M/A; see A10 |
| `100372d68` | HIGH | Precompose preserves animated outside-child rigs, shared group spaces/opacity and paint order through Undo/save/reopen. | Partial K/E |
| `b8fcb70ae` | HIGH | Numeric keyframe time can exceed visible strip, remains frame-snapped and refuses neighboring-key collisions. | Partial K/B; see U05 |
| `cf6d722e9` | HIGH | Finish sequential set_property key presets before later easing/evaluated reads; preserve explicit/omitted easing, last same-frame edit and atomic rollback. | Partial K/E |
| `d9461e878` | Normal | Retain original teardown failure while disposing unfinished workspace; not cleanup-error substitution (baseline-equivalent source). | Partial A/M; see A10 |
| `b164d6752` | HIGH | Retain original project-media staging/import error when cleanup also fails. | Partial E/P/K |
| `9c4e985b0` | HIGH | Unchanged static alignment in typography patch retains anchor; intentional alignment animation releases it. Tag and main font-anchor source match. | Partial K/E/B |
| `558eda947` | Normal | Renderer crash releases media proxy/conversion ownership and resources. | Partial E/P |
| `89f14bc50` | HIGH | Fence stale media recovery before staging/opening native project bytes, including document switches. | Partial E/K/P |
| `fc17dc877` | HIGH | Inherited group locks filter Inspector mixed selection and playhead-key actions while preserving editable siblings and single-step Undo. | Partial K/B |
| `f868f7901` | HIGH | Intern/pack composition-relative history without dropping redo or source records; identical tag/tip implementation. | Partial K/P; see P09 |
| `f930d89c6` | Normal | Notification actions remain available when dismiss button is hidden; tag/tip toast source matches. | Partial S/B |
| `60ebbc93f` | HIGH | Common selection resize scales parented shape subtrees around fixed corner without double-transforming selected children. | Partial K/B/E |
| `b41e1e47a` | Normal | Release export encoders and reject stale setup after renderer crash. | Partial E/M |
| `a9a405272` | Normal | PNG export cancellation stops delivery during preparation or encoding; no late file output. | Partial E/S |
| `17d126b99` | HIGH | Honor ancestor locks for mask, variable-font axes and alignment controls. | Partial K/B |
| `492977b97` | Normal | Refresh 3D gizmo hit geometry before deferred paint; interaction cannot hit old geometry. Tag/tip implementation matches. | Partial E/B/S |
| `66a64248f` | HIGH | Lock guards protect text animator structure and selector edits, including native Inspector paths. | Partial K/B |
| `10d1f4a9d` | Normal | Renderer loss closes media source readers and fences delayed opens; tagged source already matches. | Partial E/P |
| `8d02aaf48` | HIGH | Dragging selected media into folders moves the selection together, preserving media IDs/references and Undo. | Partial K/B |
| `2d56c3274` | Normal | After attachment preview closes, restore focus to its opener. | Partial S/B/A |
| `c00955f43` | HIGH | Ancestor group locks protect Inspector structural vector/path edits. | Partial K/B |
| `3429bc4c9` | HIGH | Deletion and easing obey inherited locks, including indirect group ownership. | Partial K/B |
| `742e16f3e` | Normal | Close external-project reader tokens when owning renderer exits. | Partial K/P |
| `27382cece` | Normal | Modal pointer actions keep keyboard focus routed inside active modal; tagged source already matches. | Partial S/B |
| `900c9cafc` | HIGH | Restore color-picker focus and ignore obsolete eyedropper completions; prevent stale color edits. Tag/tip implementation matches. | Partial S/B/K |
| `e951698c2` | HIGH | Pen/path canvas edits honor inherited group locks. | Partial K/B/S |
| `cbc6434cb` | HIGH | Partial precompose keeps shared group opacity/mask/blur/blend boundaries; copy transform-only spaces once and reject unsafe partial moves. | Partial K/E |
| `eac172bd4` | HIGH | Discard crashed-renderer save staging without releasing a fresh owner's project lock. | Partial K/P |
| `0c4e5e9bd` | HIGH | Rejected live gesture batches roll back only their partial values/keys/optional fields/cues; keep earlier coalesced commands, unrelated layers and retry baseline. | Partial K/X/B |
| `2766085af` | HIGH | Precompose dialog revalidates target identity/locks and project/composition after async choice; refuse partial stale selection. | Partial K/S/B |
| `2845a5e44` | HIGH | Fence abandoned save dialogs after renderer recovery so delayed chooser cannot publish stale bytes. | Partial K/P/S |
| `21f5c6501` | HIGH | Concurrent-save refusal leaves staged uploads retryable instead of losing their payload. | Partial K/P |
| `59e036642` | HIGH | Composition new/settings/delete dialogs stay tied to initiating project; recheck removed/last-comp targets while allowing same-project tab switch. | Partial K/S/B |
| `bad09d6b4` | HIGH | Spool cleanup errors cannot replace the primary save failure. | Partial K/P |
| `46dcd6730` | HIGH | Pen cancellation removes draft mask/restores path state; Escape reaches active canvas drag and idle Escape remains available. | Partial K/B/S |
| `2e936e3e1` | Normal | Closed project readers do not rearm idle expiry timers or prolong lifetime. | Partial K/P |
| `461fb9973` | HIGH | Canvas text edits and Pen double-click actions respect ancestor locks. | Partial K/B/S |
| `b73c2db98` | HIGH | Ungroup retains transform parent/null rigs needed by outside or locked descendants, including selected nested groups and save/reopen. Source is tag-equivalent at tip. | Partial K/E |
| `a889a5e78` | HIGH | Reject project actions whose async authorization completes after MCP disconnect; tagged implementation already matches. | Partial M/A/K; see A09 |
| `25bc8219c` | Normal | Bound Mod cleanup so reload/shutdown can complete instead of hanging indefinitely; tag-equivalent source. | Partial X/A/S |
| `37e9e0b71` | Normal | Retired Mods cannot register services/import defaults or rebuild UI; late cleanup executes once. | Partial X/B |
| `e5bbfb677` | HIGH | Fence extension publication after MCP session ends; tag-equivalent source. | Partial M/X/P; see A09 |
| `8bb51a966` | HIGH | Ungroup preserves compositing/alpha; reject unsafe nested/style/expression boundaries with visible error and no mutation, allow ordinary reversible ungroup. | Partial K/E |
| `3ec950575` | HIGH | Cancel/fence outstanding native file operations when editor document is replaced. | Partial K/P/S |
| `c5fd263b1` | Normal | Retired Mod cleanup cannot remove current-owner keybindings/shortcuts. | Partial X/S |
| `bbf2083b5` | HIGH | Document replacement terminates associated MCP run so same layer IDs in a new document cannot accept stale calls. | Partial M/A/K; see A09 |
| `8317d1f9d` | HIGH | Ungroup keeps hidden descendants hidden and refuses animated visibility boundaries it cannot preserve; tag-equivalent source. | Partial K/E |
| `77383a669` | HIGH | URL import completion stays bound to initiating project and is discarded/fenced after replacement. | Partial E/P/K |
| `a82d3bac7` | HIGH | Saving inactive live tab uses current document even when local recovery write is behind/failed. | Partial K/P |
| `fa6046879` | HIGH | Ungroup preserves Solo filtering or rejects unsafe removal without revealing unrelated layers; tag-equivalent source. | Partial K/E |
| `f07e6923b` | Normal | Release remote download handles when initiating editor document ends; tag-equivalent source. | Partial E/P |
| `908c89091` | HIGH | Serialize extension publication and recovery transactions to preserve current source/baseline; tag-equivalent implementation. | Partial X/P/M; see A09 |
| `136f546d3` | HIGH | Lazy main-process reads for heavy project/history/journal records instead of loading entire profile at bootstrap. | Partial K/P; see P08 |
| `a69ff8fe7` | Normal | Playhead-mark brand/icon update; many assets already equal tag, presentation/website ownership stays with Opus and web. | Disposition D/W; see U16 |
| `cc6adc885` | HIGH | HDR/log probe, tone mapping, native helper/fallback and tagged preview interpretation. | Partial E/P; see E13 |
| `bf26d9b3e` | HIGH | Large mixed commit: source-linked storage and gzip/upgrade/retention/autosave, effect revisions/references/new families, Inspector/timeline/shell, async/static export, visual replies/easing/imports and branding. Do not infer completeness from title or catalog. | Partial K/E/X/B/S/A/M/P/D; see expanded tables |
| `e8f95f1eb` | Normal | Expanded timeline groups show child property keyframes instead of only group keys. | Partial B; see U05 |
| `cdab66cb4` | HIGH | Sequential MP4 export, accurate sample timing, bounded encoding, capability-gated VideoToolbox and long-export job continuation. | Partial E/M/A; see E14/E15 |
| `4f5cf623d` | HIGH | AE docking/stack drags/maximize shortcut/fullscreen, timeline/Inspector wells and Claude auto approval. | Partial S/B/K/A/D; see U01/U04/U08/U11/A03 |
| `25d92afc5` | HIGH | Provider presets/model discovery/local setup and nine external agent-client registration targets. | Partial + gaps A/M/P/S; see A01/A02 |
| `929102722` | HIGH | Remove web-animation delivery from desktop menus/export/bridge/player handoff/types/docs; keep website player consumers. Owner removal already recorded in P0/W1; not a missing cutover feature. | Disposition S/E/M/W |
| `c0e47ac20` | Normal | Distinct timeline/graph selected-keyframe treatment. | Partial B/D; see U06 |
| `8eb092ee8` | HIGH | Per-axis key diamonds, unified diamond affordances, saved Separate Dimensions and panel-library button. | Partial K/B/S; see U07 |
| `81ed886fa` | Normal | Shared tooltips follow window edges and keyboard focus; input hides them and adjacent controls share timing. | Partial S/B/D; see U09 |
| `06d25cf64` | Normal | Custom macOS-style app menus/selects/popovers, contextual action grouping and menu-bar/dialog labels; later policy corrected by 8439c2ed1. | Partial S/B/D; see U10 |
| `8439c2ed1` | Normal | Restore AppKit text services; dim unavailable actions, restore context shortcuts and regroup menus by target. | Partial S/B/D; see U10 |
| `d8d887e0e` | HIGH | Join in-flight conversions, preserve owned bytes when original moved/changed and ignore superseded recovery failure. | Partial E/P/K; see P10 |
| `d9a98ea5a` | Normal | Adds docs/3d-studio-redesign.md proposal only: no running 3D implementation. Opus/product review and engine work would own any adoption. | N/A D/E/B |
| `7cb061739` | Normal | Richer streaming markdown/local media grants, folded work steps and persistent duration timer. | Partial A/B/S; see A04 |
| `0e1af9e60` | Normal | Add Effect button in Effect Controls and + on Properties Effects heading. | Partial B/D; see U08 |
| `ba5a8742c` | Normal | Sheet material, Title Case/action names/aligned value columns and neutral dark appearance; native tokens/spec review governs. | Partial B/S/D; see U11 |
| `70ffd6494` | HIGH | Disk-streamed media, decoder residency/pressure, limited preview installs, hidden-panel/layout/meter efficiency and export waiting for background save. | Partial E/B/P/K; see E17/E18/P07 |
| `5458b5449` | HIGH | Anchor Point selects/narrows and drags in one gesture; click alone changes only selection. | Partial B/K/S; see U14 |
| `c7f46a056` | Normal | Pool invalidated preview framebuffers for allocation-free warmed numeric-value scrubs. | Partial E/B; see E18 |

### 38 test-only and merge commits

These are explicitly accounted for, not omitted from the range. Test changes
are evidence candidates, not implemented product features or native passes.
For merges, inspect each parent if investigating integration resolution;
the component changes are already represented above.

| Commit | Subject | Native coverage / disposition |
| --- | --- | --- |
| `26917cc9f` | Integrate gathered editor and agent changes into main | N/A — Merge ancestry; component commits above carry behavior; no separate feature acceptance. |
| `831574234` | Align effect preflight regression with named uniform support | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `72ecc06d0` | Flush project recovery through the editor in save regression | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `9e9854d66` | Exercise extension reload and enable through the supported kernel API | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `ebf6dbd5a` | Save persistence fixtures before the harness discards unsaved projects | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `bba44c393` | Wait for the easing menu action before checking and undoing it | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `afa75edad` | Keep nested timeline disclosure test clicks inside the viewport | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `1831228ab` | Retain timeline caption import in the shared layer menu regression | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `b7c244715` | Disable test mods through the supported extension API | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `df9383806` | Configure the provider fixture through its real Settings controls | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `819dd30d3` | Exercise private bridge regressions through supported editor APIs | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `c5a6b4e16` | Save imported media fixtures before intentional unsaved discard | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `9aa0b161c` | Verify Settings sections and current color picker behavior | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `f473716a9` | Check shape dimensions and content keys in resize regressions | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `4685e2907` | Assert the raw Electron bridge stays private to the kernel | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `524bd7da0` | Verify current color picker layout and preview cancellation | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `d967801df` | Scroll number fields into view before testing scrub gestures | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `94be88710` | Drop the floating agent panel inside its docking edge zone | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `ba78b0c63` | test(agent): distinguish tool images from captured context | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `925d913cc` | test(agent): assert explicit attachment alongside context frames | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `33b58835e` | test(panels): target mounted notes after library closes | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `fc197cdf9` | test(animation): reveal advanced parameter stopwatches | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `abcaf8371` | test(panels): respect panels excluded from the library | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `782303cef` | test(media): reset marquee viewport between themes | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `17382dd77` | test(shell): verify current restore and document retention policy | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `7e8bd2acd` | test(editor): select text animator presets through current menu | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `fbf99c973` | test(controls): exercise dropdowns in the active workspace | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `dc5596466` | test(agent): exercise current isolated effect authoring workflow | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `1e89331bb` | test(effects): validate namespaced sample and supported parameter count | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `38901079e` | test(design): inspect mounted controls and current selection cues | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `03432a6f3` | test(agent): retain real context capture in provider checks | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `4fe41353c` | test: model external project readers with native lifecycle events | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `bbc6cf3b3` | test: click visible sandbox views before routing keyboard shortcuts | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `228a6a91d` | test: isolate sandbox clipboard permissions in hidden sessions | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |
| `e263accc8` | Merge pull request #186 from iterative-computer/fix/export-throughput-185 | N/A — Merge ancestry; component commits above carry behavior; no separate feature acceptance. |
| `3a4edb1f9` | Merge GitHub main into local main | N/A — Merge ancestry; component commits above carry behavior; no separate feature acceptance. |
| `f9bd84ee4` | Merge branch 'main' into hig-pass | N/A — Merge ancestry; component commits above carry behavior; no separate feature acceptance. |
| `a09dcaa16` | Match the inspector lock test to the title-cased menu labels | N/A — Tests/fixtures only; no new runtime behavior. Relevant area retains the underlying gate. |

## Follow-up boundaries — 2026-11-04

The data-first review order is P02/A07 (memory/catalog disagreement), P03–P07/P10
(linked/owned bytes, checkpoint/gzip, autosave/retention/rollback), E01–E12
(effect revisions/typed references/space/temporal/LUT closure), then U07
(Separate Dimensions). Keep open/save/reopen, Undo/Redo and reverse-handoff
fixtures for each. P13 is an approved product removal but does not waive data
retention. These are routed findings only; this lane implements nothing and
changes no phase gate, frozen catalog, shared inventory or lockfile.

**DESIGN-TODO(opus):** review the deltas against existing specs for Memory and
Storage (Settings explicitly leaves these for later), provider discovery/context
warnings and the nine-client setup surface. Neutral inventory placeholder for
unwritten states: **spec pending**. No layout, wording, color or gesture is
proposed here. Existing Agent/Timeline/Inspector/menu/tooltip specs apply where
present; Opus must review any delta-specific state they do not describe. Track
this alongside the already open surfaces in `surfaces.md`, not as an inferred
approval to copy main's chrome.

Mac verification remains necessary for AppKit text menus/Writing Tools/Services,
clipboard/Finder/focus/IME/AX, docking/fullscreen/maximize and all real panel
presentation; physical playback/pacing/A/V, Metal shader/decode/HDR/export; APFS
cloning/full-fsync/NSFileVersion, Keychain/client registration and real-profile
migration/restore. The corpus, extension and strict agent gates in HANDOFF remain
open. No Mac app or hardware behavior was exercised by this documentation lane.

## Audit verification — 2026-11-04

This is documentation-only: **0 Rust crates changed and 0 Cargo tests run**.
No dependency/lockfile change, formatting rewrite or implementation was needed.
The absent `/workspace/.tools/activate.sh` prevented that setup step; Git, Python
and a temporary standalone gitleaks binary supplied the source-audit checks.
Final portable audit: **9 checks passed, 0 failed**, covering 138 commits and
57 expanded entries; whitespace and staged secret scanning passed.
No tolerance, skipped test or optimistic capability was used to claim parity.

Run the self-contained audit below from the repository root; it depends only on
this owned document, tracked source/docs and fetched Git objects. It checks the
pinned snapshot rather than treating a later main tip as the same inventory.

```sh
python3 -c 'from pathlib import Path; s=Path("docs/native/main-delta.md").read_text(); exec(s.rsplit("<!-- audit-python:start -->",1)[1].split("```python",1)[1].split("```",1)[0])'
git diff --check
gitleaks git --pre-commit --staged --redact
```

<!-- audit-python:start -->
```python
from pathlib import Path
import re
import subprocess

DOC = Path("docs/native/main-delta.md")
text = DOC.read_text()
TAG = "07ed5fa17ee640659cb075f9cad1af2095011d60"
MAIN = "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()
def source(rev, path):
    return git("show", rev + ":" + path)
def check(name, ok):
    assert ok, name
    print("PASS " + name)

check("pinned tag and main", git("rev-parse", "v1.2.0^{}") == TAG
      and git("rev-parse", "origin/main") == MAIN)
ledger = text.split("## Complete commit ledger", 1)[1].split("## Follow-up boundaries", 1)[0]
listed = re.findall(r"^\| `([0-9a-f]{9})` \|", ledger, re.M)
expected = [sha[:9] for sha in git("rev-list", TAG + ".." + MAIN).splitlines()]
check("138 commits exactly once", len(listed) == len(set(listed)) == 138
      and set(listed) == set(expected))
parts = ledger.split("### 38 test-only and merge commits", 1)
check("100 source/doc plus 38 test/merge", len(parts) == 2
      and len(re.findall(r"^\| `[0-9a-f]{9}` \|", parts[0], re.M)) == 100
      and len(re.findall(r"^\| `[0-9a-f]{9}` \|", parts[1], re.M)) == 38)
expanded = text.split("## Expanded change inventory", 1)[1].split("## Complete commit ledger", 1)[0]
ids = re.findall(r"^\| ([PEUA][0-9]{2})[ \|]", expanded, re.M)
check("57 uniquely owned expanded changes", len(ids) == len(set(ids)) == 57)
links = re.findall(r"\[[^\]]+\]\(([^)]+)\)", text)
check("local documentation links", all((DOC.parent / link.split("#", 1)[0]).exists()
      for link in links if "://" not in link))
for section in [expanded, ledger]:
    width = None
    for line in section.splitlines():
        if not line.startswith("|"):
            width = None
            continue
        cols = len(line.split("|"))
        assert width is None or cols == width, line
        width = cols
check("table widths and per-row native coverage", all(
      any(word in line for word in ["Partial", "Gap", "Disposition", "N/A"])
      for line in (expanded + "\n" + ledger).splitlines()
      if re.match(r"^\| (?:[PEUA][0-9]{2}|`[0-9a-f]{9}`)", line)))
critical = ["P01", "P02", "P03", "P04", "P05", "P06", "P07", "P09", "P10", "P13",
            "E01", "E02", "E03", "E04", "E09", "E11", "E12", "U07", "A07"]
check("data changes marked HIGH", all("| " + item + " HIGH" in expanded for item in critical))
spec = "apps/desktop/src/main/agent-tools/spec.ts"
names = lambda rev: set(re.findall(r"name:\s*'([^']+)'", source(rev, spec)))
api = "apps/desktop/src/renderer/src/kernel/api.ts"
types = "apps/desktop/src/renderer/src/core/types/project.ts"
check("baseline carries and confirmed MCP/API deltas", names(MAIN) - names(TAG) == {"project_memory"}
      and not names(TAG) - names(MAIN)
      and "jobId:" in re.search(r"name: 'export_video'[^\n]+", source(MAIN, spec)).group()
      and "jobId:" not in re.search(r"name: 'export_video'[^\n]+", source(TAG, spec)).group()
      and "CaptionsLayer" in source(TAG, types) and "colorDepth?" in source(TAG, types)
      and "separateDimensions?" not in source(TAG, types)
      and "separateDimensions?" in source(MAIN, types)
      and all(name not in source(rev, api) for rev in [TAG, MAIN]
              for name in ["renderFrameToAsync", "snapshotAsync", "rasterAsync"])
      and '"project_memory"' not in Path("apps/native/crates/mcp/fixtures/catalog-v1.2.0.json").read_text())
changed = git("diff", "--name-only", "origin/feat/native-engine").splitlines()
check("lane ownership", changed == [str(DOC)])
print("9 checks passed; 138 commits; 57 expanded changes; 0 runtime claims")
```
<!-- audit-python:end -->

## 2026-11-05 — cloud Timeline integration, partial evidence

`feat/na-cloud-timeline`: `5fbeeb5ff`, `dfb9ee065`, `21c7536a5` connect the existing
v4 worker to kernel-backed gestures, scoped pro edits, parent/key/easing edits,
joined Position and per-axis diamonds; retain negative/coincident key identities;
and back the existing Easing library service with atomic archive-preserving
extension storage. Post-merge Linux evidence: 193 Rust tests and 54 Timeline Bun
tests / 18 snapshots passed; touched-crate Clippy, formatting and v4 typecheck
passed. The read-only timeline-render input is already contained in the base.

U04/U05/U06/U07/U12 and Timeline U13 remain **partial/blocked**, not closed:
application hit-testing/control/menu/field/presentation bindings, full Easing and
motion-path tools, caption command bodies, Mac physical checks and Opus review
remain. No coverage-column completion or whole-app acceptance promotion is
claimed. Exact ownership boundaries, commands and required Mac verification are
in [cloud-timeline.md](cloud-timeline.md#2026-11-05--final-integration-evidence-and-blocked-handoff).
## 2026-11-05 — Inspector lane application-path evidence (partial)

Reference remains `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. This append
preserves the inventory and records partial coverage, not closed UI acceptance.
See [cloud-inspector.md](cloud-inspector.md) for exact unowned dependencies and
Linux verification. No acceptance blocker or existing row is removed.

| Row | Coverage / commit | Still required |
| --- | --- | --- |
| U02 | Partial — existing point models retained; no new visual-point parity claim. | Actual point-pad/native recipe, independent axis gestures, generated/contributed parameter bindings and Mac input proof. |
| U03 | Partial — `85f03e014` connects archived media-completion metadata to guarded actor admission, with relink preservation/Undo tests. | Byte-store admission, shell pasteboard ownership/focus/precedence, image/Finder import and placement/cancellation/save/reopen. Completion descriptors do not prove bytes or readiness. |
| U08 | Partial — `2702a18cd` connects Properties Add Effect through real V8, asynchronous native-menu replies and kernel receipts. Supported Effect Controls scalar fields now write through real registered bindings at current time. `85f03e014` preserves row actions after separators and rejects disposed/opaque bindings. | Native effect menu groups/Recent/checkmarks, remaining property services, Mac pointer/keyboard/AX and Opus presentation acceptance. |
| U13 | Partial — `2702a18cd` adds the approved Settings Color Depth row and real 8/16/32 actor edits/Undo; `85f03e014` tests numeric/colour/toggle/choice/text effect fields and retained authored weight values. | Captions Add/Edit planner and extension-content kernel backing, full Inspector recipes/services, audio/captions/speech integration and Mac verification. |
| U14 | Partial — no anchor live-edit or on-screen claim. | Actual viewer hit/selection/narrowing/parent-transform drag/cancel, atomic document fencing for Selection/Gesture calls, then Mac screen verification. |
