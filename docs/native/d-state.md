# D-STATE: persistence and migration inventory

Source baseline: **`v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`)**. All shipped-source observations below were read using `git show v1.2.0:<path>` from this repository. Engine-only observations use the locally available `origin/feat/native-engine` at `1ac7075a1d980b1b35f09b04d3a8ffe1046cfc66`; they are not attributed to the shipped tag. Research only: no build, migration, profile write, or runtime launch was performed.

`P` means the app's profile, normally `~/Library/Application Support/Powermove`. `M`, `R`, `S` and `X` abbreviate `apps/desktop/src/main/`, `apps/desktop/src/renderer/src/`, `apps/desktop/src/shared/` and `apps/desktop/src/extensions/`. References are source file and line at the stated revision, not the working tree. Schema field names and path templates below are public code, never sampled user values.

## Decisions and non-loss contract

1. Import the library, current documents, trash, journals, undo/redo, conversations, attachment bytes, settings, workspaces, extension source/storage/provenance and all available media. A `.pmv` file is optional, not the library's source of truth.
2. Preserve unknown fields and unrecognised legacy keys as opaque migration input. Do not delete, normalize away, or silently reset state merely because current code no longer reads it. Preserve authored geometry, timing, text, picked colours, shader coordinates and extension semantics; browser rendering bugs are not migration requirements.
3. The kernel becomes the document/history/journal/settings/workspace authority. JSON extension storage retains its per-extension contract. Media enters the BLAKE3 content-addressed store; source metadata and legacy keys remain resolvable. Copy secrets through the transition release's Keychain handoff, or the direct Keychain fallback. Never write decrypted secrets to migration logs/manifests.
4. Use resumable, atomic `migration.json` checkpoints and availability manifests. Hash the migration base, include tombstones, and merge settings per key and extension storage per object key. Same-name workspace conflicts and changes to both extension trees retain both versions. Beta has its own profile/library and read-only shared extensions plus an overlay. Cutover adds both sides' project work rather than overwriting.
5. Copy/snapshot Chromium LevelDB **and its external blob directory** before direct reading, or wait for Electron to quit. Every native first launch performs a delta pass. TR dual-writes new imports to both stores. Nothing is removed from the Electron profile; media GC stays disabled until cutover + 1 release.
6. Reference closure includes current projects, trash, takes, undo/redo and agent artifacts that own media. Record each payload as verified original, proxy-only, or already offline. Only newly missing data fails migration. Missing posters can be regenerated. A proxy is never silently promoted to an original.
7. Restore compacts current journals into files/library recovery copies before reverse migration. Export current settings/storage/secrets and rollback-readable project/history formats, not just the pre-migration backup. Keep project ids, composition ids, source time/rate and timecode fields exact for later OTIO interchange.

These are migration requirements, not claims that a migration reader has already been implemented or tested.

## 1. File store: `P/store/<key>.json`

**52 registered key families: 42 literal keys + 10 dynamic prefixes.** Full authoritative registry: [store-keys.ts:8–83](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/src/shared/store-keys.ts#L8). Each is ordinary UTF-8 JSON, readable without Electron. Dynamic suffixes accept `[A-Za-z0-9_-]{1,120}`. The Electron shim accepts `pm.<key>` aliases but strips `pm.` before IPC: **there is no independent `pm.*` file-store namespace** (`R/legacy/host/electron-shim.ts:320–331`). The browser-only fallback does use localStorage `pm.*` (§2).

Common writer/reader: `M/storage.ts:95–138, 164–198, 295–383, 449–476`; `M/index.ts:934`; `R/legacy/host/electron-shim.ts:315–480`. Writes debounce 150 ms, then write/fsync/rename a sibling temporary file. Invalid JSON may become `<key>.json.corrupt-<timestamp>`; retain quarantined files and failed-save artifacts as recoverable input. Heavy keys load lazily; the 64 MiB cache is **not a data-size limit**. Limits: ordinary values 32 MiB; `project`, `projectState`, `projectHistory` 256 MiB each; `takes` 24 × 32 MiB (768 MiB). Source: `S/ipc.ts:247–262`, `M/storage.ts:18–24, 332–371`. Tables below give the logical writers/readers in addition to that common backend.

### Dynamic families (10)

| Key pattern | Schema and size characteristics | Logical writer → reader | Native target |
|---|---|---|---|
| `project.<id>` | `{v, proj}`; bare/stringified legacy projects accepted. Entire editable graph: compositions, layers, animation, assets, notes, extension data, export defaults; up to 256 MiB JSON | `R/legacy/core/projects.ts:58–112` → `:157–194`; `R/legacy/app.ts:692–701` | Library document/checkpoint; preserve ids/revisions and all opaque extension data |
| `projectJournal.<id>` | Array of revision/project/patch records; recovery uses path/existence/value patches. Checkpoint at 50 revisions or >4 MiB (ordinary write ceiling still 32 MiB) | `projects.ts:18–46, 312–324` → `:157–194` | Kernel journal keyed to migration base hash; never replay on another base; recover as copy on conflict |
| `projectState.<id>` | Legacy combined editor session, workspace, file hashes/path and optional history; 256 MiB ceiling | `projects.ts:214–229`; `R/legacy/app.ts:674–690` → `projects.ts:196–212` | Session + workspace + file association + imported history; preserve until split forms verified |
| `projectMeta.<id>` | Split lightweight session metadata excluding history; includes file state, workspace, selected/time/view state from session capture | `projects.ts:214–229`, `R/legacy/app.ts:674–690` → `projects.ts:196–212` and `M/index.ts:802` | Kernel uiState and native document association; split metadata overlays legacy session |
| `projectHistory.<id>` | Version-1 patch history or packed history dictionary; 256 MiB ceiling; see §5 | `projects.ts:223–224`, `S/history-memory.ts:107–125` → `projects.ts:196–212`, `R/legacy/core/history.ts:329` | Per-project kernel undo/redo with cursor; rollback export |
| `projectWorkspace.<id>` | Workspace object, separately saved from project/history | `R/legacy/core/workspace.ts:6–15, 550–559` → `R/legacy/app.ts:582,1145` | Named/project workspace + layout state; retain conflicts as both |
| `agentThreads.<id>` | `{version:1, activeId, threads:[...]}`; thread title/time, conversation/trace/visual-reply references, composer draft, attachment references, scope/provider/model/reasoning/access/run/orchestration state; ordinary 32 MiB ceiling | `R/panels/agent/threads.ts:4–25,139–239` → same archive class; `R/legacy/assistant/spatial.ts:420` | Agent conversation archive; retain run/thread relationships, resume handles and drafts |
| `agentAttachment.<id>` | Detached JSON attachment payloads, including encoded image/text/file content; archive keeps lightweight references and `payloadStored`; 32 MiB ceiling | `threads.ts:196–220` → same; `M/storage.ts:242–256` pins referenced payloads | Agent attachment objects; verify archive-to-payload closure, including message attachments |
| `agentModel.<provider>` | JSON model string per provider; bytes to short text | `R/legacy/assistant/spatial.ts:314,346,1007` → `:1013` | Provider preferences (distinct from old literal `agentModel`) |
| `ext.<extension-id>` | One arbitrary JSON object per extension; `api.storage.get/set/delete` changes its members; 32 MiB ceiling | `R/kernel/install.ts:539–567` → same; Store seed `R/kernel/sandbox-host.ts:205` | Same extension id and object/key contract; 3-way merge with tombstones; no flattening |

### Literal keys (42; every registered key is listed)

All use the JSON backend and are independently readable without Electron. Unless stated above, the per-value limit is 32 MiB. Grouped rows contain separately counted keys.

| Literal keys | Schema / purpose | Writer → reader at the tag | Native target |
|---|---|---|---|
| `projects` | Array `{id,name,at,w,h,thumb?}`; thumbnails may be encoded images | `R/legacy/core/projects.ts:49–54,115–129` → same; `M/index.ts:772–802` | Library index, reconciled with orphaned document slots |
| `openWindows`, `windowTabs`, `lastActiveProject` | Project ids, window `{tabs,active}` sessions, most recent id | `M/index.ts:958–961` → `:772–802` and `M/windows.ts` | Native window/tab restoration |
| `openTabs` | Old single-window id strip | No current primary writer; compatibility reader `projects.ts:304–307`, `M/index.ts:773` | Legacy restoration input; do not duplicate already restored windows |
| `restoreWindows` | Boolean | `R/settings/SettingsScreen.svelte:161,354` → same and main restoration | Settings |
| `autoDownloadCloudMedia` | Boolean | `R/legacy/core/cloud-media.ts:119–124`, settings `:162,533` → same | Cloud download preference |
| `previewMemoryMiB` | Numeric memory preference | Settings `:512` → `R/legacy/core/memory.ts:17` | Preview cache budget preference |
| `previewResolution` | Resolution mode | `R/legacy/core/preview-resolution.ts:10–22` → same | Viewer quality setting |
| `projectTrash` | Recoverable deletion metadata; document slots remain intact | `projects.ts:268–295` → trash/library functions | Library trash and tombstones; retain associated media/history |
| `takes` | Up to 24 editable snapshots; 768 MiB total write ceiling | `R/legacy/core/history.ts:482–497` → same | Named editable takes and all their media |
| `workspaces`, `workspace`, `defaultWorkspace` | Workspace array; current/default workspace ids | `R/legacy/core/workspace.ts:481–552`; `R/legacy/assistant/workspace-layout.ts:42–49` → same and app `:1150` | Workspace registry and selections |
| `afterEffectsExtensions` | List of imported tool/extension ids | `R/library/after-effects-imports.ts:12–23` → same | Imported-tool metadata; preserve source extensions |
| `workspaceTrash` | Recoverable workspace array | `R/legacy/core/workspace.ts:607–623` → same | Workspace trash |
| `panelVisibility` | Panel-id → visibility map | `R/layout/panel-visibility.ts:10–34` → same | Shell layout visibility |
| `theme`, `themeMode` | Legacy appearance value and system/light/dark preference | `R/legacy/app.ts:143–174` → same; `R/kernel/install.ts:856` | Appearance preference (T1) |
| `activeTheme` | Selected theme id | `R/kernel/install.ts:917` → `:927` | Theme selection, delayed until registration exists |
| `extensionWorkspaceAppearance`, `extensionThemeAppearance` | Versioned owner/base/override stacks; not only the currently visible result | `R/kernel/install.ts:749–762`; `R/kernel/workspace-appearance.ts`, `theme-state.ts` → same | Owned appearance stacks; preserve user edits and reversible enable/disable/reload |
| `exportOpts`, `renderPresets` | Export option object and preset array `{id,name,options}` | `R/legacy/core/exporter.ts:55,204–208,283` → same | Export defaults and presets |
| `renderQueue` | Render job array; former `rendering` entries reopen as `pending` | `R/legacy/core/render-queue.ts:4–5` → same | Background/export jobs; preserve specification and outputs, revalidate runtime handles |
| `autosave` | Legacy single-project envelope | Current readers/deleters `projects.ts:163–184`, `R/legacy/app.ts:75,210`; main recovery `:786` | Recover legacy project even if missing from index |
| `projectsSection`, `projectsSort`, `projectsView` | Library section/sort/grid-or-list strings | `R/legacy/ui/projects.ts:17–25,56,109,137` → same | Library UI preferences |
| `agentScope`, `agentProvider`, `agentReasoningEffort`, `agentAccessMode`, `agentApprovalMode`, `agentAutoApplyPanels` | Agent scope/provider/reasoning/access/approval strings and auto-apply Boolean | `R/legacy/assistant/spatial.ts:295–315,1007–1046,1708–1721` → same | Agent preferences and approvals; no increase of authority |
| `bootVersion`, `agentPresentation`, `agentFloatingPosition`, `agentBubblePosition`, literal `agentModel` | Registered compatibility slots; no current production get/set found at this tag (the code constant `PM.bootVersion` is not a persisted write) | Registry `S/store-keys.ts:35,41–46`; retained input, not proof of live writers | Preserve opaque legacy values; apply only after schema recognition |
| `extensions` | Extension-id → enabled-state map | `M/extensions/registry.ts:98–99,479` → registry | Extension enablement; preserve disabled installs |
| `cloudAccount` | Legacy non-secret account record; current code clears it | `R/cloud/account.ts:190`; registry `:54–56` | Legacy migration input; real session is in §4 |

**Not shipped:** `documentHost.<id>` is absent from `v1.2.0`'s registry. The engine branch adds it (`S/store-keys.ts:64`; `apps/desktop/src/engine/document-host/protocol.ts:43`, `worker.ts:36–46`): `{v:1,projectId,hash,revision,history,savedAt}`. Copy and validate the hash against the document, then import history into the kernel; do not replay onto a different document. It is plain JSON and needs no Electron reader.

## 2. Chromium localStorage (LevelDB)

Location: `P/Local Storage/leveldb/`, origin-scoped (packaged origin `app://powermove`, `M/index.ts:92,597`; development/remote origins can differ). **Three shipped key families** below, plus an open-ended trusted-extension namespace. No application sessionStorage persistence writer was found.

| Key family | Schema, writer and reader | Size / direct-read status | Native target |
|---|---|---|---|
| `pm.easing.library.v1` | `{version:1,groups:[{id,name,presets:[{id,name,color,curve}]}],active,preferences}`; preferences include view/tab/columns/size/labels/palette/preview. `R/panels/easing/library.ts:1–9,37–68`; `EasingPanel.svelte:26,56` | Up to 100 groups/5,000 presets by validator, browser quota applies. Requires logical export or LevelDB decoding | Easing library and exact curve points/handles/interpolation + panel preferences |
| `pm.fx.recent` | JSON array of effect ids, newest first, maximum 8. `R/panels/fx-browser.svelte.ts:10–26` | Small; same read requirement | Recent effects |
| `pm.<store-key>` | Browser fallback JSON values from `R/legacy/core/util.ts:246–249`; Electron replaces `PM.store` with file-store shim | May contain complete older library/session/extension objects. Do not assume all `pm.*` are disposable duplicates | Import with source/checkpoint precedence; retain conflicts, not blanket overwrite |
| Other trusted-extension keys | Trusted v1–3 code shares the document and can use browser storage directly; no finite application-owned key list exists | Preserve all origin key/value pairs, not only allowlist matches | Compat-origin import or per-extension opaque archive until ownership is known |

**Engine-only extra family:** `powermove:native-evaluator`, `R/engine-doc/flags.ts:8–16,38,80–84` on the engine branch: `off/on/audit/audit-proxy/strict` plus aliases. Absent at `v1.2.0`. Preserve in the legacy checkpoint, but do not enable obsolete evaluator modes in native production. This is a retired implementation switch, not authored project intent.

## 3. Chromium IndexedDB: imported media and posters

Location: `P/IndexedDB/<origin>.indexeddb.leveldb/` **and** the matching `<origin>.indexeddb.blob/`. Database **`powermove-media`, version 2**, object store **`assets`**, `keyPath: 'id'`. There are **two payload key families**: `media:<fingerprint>` (older entries can be bare asset ids), and `<storageKey>:poster`.

The record written at [media.ts:90–105](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/src/renderer/src/legacy/core/media.ts#L90) is:

```ts
{ id, blob: Blob, revision: string, fingerprint: string | null,
  size: number, type: string, at: number }
```

`revision` is a storage identity for each replacement, including same-size replacements; older records get one lazily on save (`media.ts:106–120`). `get` falls back from a content key to the old asset id (`:123–133`). This is actual media, not merely a cache of filenames. Bytes range from small posters to multi-gigabyte video; browser/disk quota is the practical limit. No global migration can safely materialize it all in RAM.

- [raster.ts:764–782](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/src/renderer/src/legacy/gl/raster.ts#L764) substitutes a playback Blob when video decode fails; `persistBlob` is the substituted source.
- [raster.ts:887–910](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/src/renderer/src/legacy/gl/raster.ts#L887) waits for that decision and writes the chosen Blob. Poster writes are at **943, 1089 and 1336**; restore reads are at **1251–1252**.
- [media-formats.ts:26–47](https://github.com/iterative-computer/powermove/blob/v1.2.0/apps/desktop/src/shared/media-formats.ts#L26) distinguishes always-converted containers (including MKV) from MOV/MP4 that may require fallback. `M/media-proxy.ts:46–64` makes full-dimension alpha-preserving VP9/Opus WebM playback proxies. Converted HEIC/TIFF stills and animated-image sequences can also store transformed payloads (`raster.ts:870–885`), so original fingerprints do not always describe stored bytes.
- **Important schema correction:** `playbackProxy`, `playbackProxyVersion`, `sourcePath`, source size/name and related import metadata live in **project asset metadata** (`raster.ts:828–841`), not in the IndexedDB record shape above. `MediaStore.put` does not copy these fields. Migration must join every project/history/take asset reference to the storage record. Metadata-only proxy detection is unavailable for an orphaned record: retain it and its provenance as unclassified, never invent an original.
- Original fingerprints are **`v2:<size>:<sha256>`**: SHA-256 of `powermove-media-v2\n<size>\n` followed by up to three 64 KiB samples at beginning/middle/end, deduplicating offsets (`R/legacy/core/media.ts:10–12,157–176`). This is **sampled identity, not full-file SHA-256 or BLAKE3**. Relink at `raster.ts:1003` uses it. Preserve the v2 value and verify the original candidate with that rule; then record full BLAKE3. Separately hash the stored Blob's actual bytes.

**Target:** stream/clone available Blobs to the content-addressed store; preserve every legacy key mapping and revision. Proxy objects get `proxy-of(original v2 fingerprint, sourcePath)`. Resolve matching originals when available, otherwise retain playable proxy-only state with relink and export warning. Never discard media merely because its source file disappeared. Missing originals already offline remain offline. Posters are reproducible, but copy available ones cheaply.

**Without Electron:** possible with a correct Chromium IndexedDB/structured-clone/external-Blob reader; no such verified reader was found at the shipped tag. S6 must prove it against snapshots, old bare keys, transformed/proxy-only records, deleted originals, large Blobs and interrupted imports. TR can export logically using Electron's IndexedDB API. Direct reading is required for a skipped TR and first-launch delta; it is not optional re-import-from-path recovery.

## 4. Credentials and non-store profile files

All paths in this section are templates under `P`, not observed filenames. **Four safeStorage payload families** are marked **K**. Their bytes can be copied without Electron, but decryption requires the legacy Keychain key/ACL or a TR handoff. Non-secret JSON is independently readable. Do not confuse expired authentication proof with authored work.

| Path / family | Schema and writer → reader | Size / offline access | Native target |
|---|---|---|---|
| `env/<envKey>.env` **K** | Dotenv lines preserving comments/unknown fields; ordinary quoted values and `KEY=enc:<base64(safeStorage)>` secrets; plaintext secret inputs are sealed on write. `M/env/store.ts:1–19,135–182`; extension env APIs via `M/index.ts:1023` | Per extension env identity, typically small, not necessarily empty. Parse plaintext without Electron; encrypted values need K | Extension environment store; preserve provenance `envKey`, separate secrets in Keychain |
| `cloud/session.bin` **K** | safeStorage-encrypted JSON `{origin,token,expiresAt}`. `M/cloud/session.ts:4–5,95–136` | Small sealed record; K | Native cloud session in Keychain; preserve sign-in where valid |
| `agent-provider.json` **K** | Compatible-provider config `{baseUrl,model,vision,hasKey,models?,secret?}`; `secret` is base64 safeStorage ciphertext. `M/compatible-provider.ts:15,41–75`; constructor `M/codex/index.ts:368` | Up to 8,192 API-key chars before encryption; config parses directly, secret needs K | Provider config + Keychain key; no re-prompt just because UI runtime changed |
| `cloud/human/<sha256(origin+scope)>.bin` **K** | `{origin,scope,clearance,expires}` sealed by safeStorage. `M/cloud/human-verification.ts:7,20–34,52–56` | Small, expiring; K | Preserve unexpired proof via secure handoff or renew; not a fifth user account |
| `cloud/session.json` | Non-secret `{origin,me,fetchedAt,provider?}` cache. `M/cloud/session.ts:5,108–115,132–136` | Small; direct JSON | Account display/cache tied to migrated session |
| `cloud/registry.json` | `{origin}` custom registry preference; packaged app deliberately ignores unsupported overrides. `M/cloud/registry-url.ts:5,60–61` | Small; direct JSON | Preserve valid preference without expanding packaged trust policy |
| `extensions-provenance.json` | Extension-id map: `envKey`, origin/release/tree lineage, published/upstream, pending update, trusted permission approval. Unknown fields retained. `M/cloud/provenance.ts:1–10,18–87,105–152` | Grows with installations; direct JSON | Same extension provenance, trust and env identity; do not reset trust or silently grant new permissions |
| `onboarding-v1.json` | `{version, ...choice}` including selected creative-app/workspace/agent/timeline choices. `M/onboarding.ts:9–12,23–40` | Small; direct JSON | Onboarding completion and choices; no forced setup repeat |
| `whats-new.json` | `{version}` last notes seen. `M/whats-new.ts:17,75–89` | Small; direct JSON | Release-notes marker |
| `mcp/connection.json` | Live descriptor with instance/port/token and protocol data. `M/agent-tools/external-server.ts:57–61` | Small, **secret and process-bound**; direct bytes only | Regenerate connection/session credentials; preserve client registration, never reuse stale capability tokens |
| `mcp/` registration receipt | Receipt backing external desktop-client configuration. `M/claude-desktop-mcp.ts:37–81` | `mcp/claude-desktop-registration.json`; small JSON, direct | Preserve registration/undo receipt; reconnect to native executable |

**Keychain and OS-owned state:** macOS holds Electron's application Safe Storage key and runtime-owned credentials; TCC grants/LaunchServices associations are OS records. Native cutover retains bundle identity, signing team and designated requirement. TR re-encrypts into an item usable by both designated requirements; a skipped TR uses the legacy Safe Storage Keychain key with the OS access prompt. Do not claim `safeStorage.decryptString` is a portable ciphertext format or that a second bundle id automatically inherits access. ACL behavior remains an S6 signed-build gate.

## 5. Project files, associations, backups and history

**Six location/format families** are separated here: external `.pmv` (three readable representations), association registry, association commit journal, backups, interrupted publication/recovery siblings, and exported deliverables/sidecars.

| Location / pattern | Format; writer → reader | Size / Electron needed? | Native target |
|---|---|---|---|
| User-selected `*.pmv` | JSON envelope `{v,proj,ws,history}` (old bare projects accepted); legacy embedded `media[id]={type,data:base64}`; PMV3 binary and PMV4 incremental below. `R/legacy/app.ts:781–794`; `R/legacy/core/project-file.ts:9–23,62–158`; `M/project-files.ts` | Media has no app-imposed total cap; bounded transfer. Ordinary byte/file parser, no Electron required | Read all variants; preserve envelope/unknown fields, missing-media references, nested comps, full undo/redo media closure |
| `project-files.json` | Project-id → `{path,hash,generation?}` associations; main owns external-change checks. `M/project-files.ts:28–41` | Small metadata; direct JSON | Document registry; keep relative relationship to session saved/baseline hashes |
| `project-files.json.pending-<uuid>.json` | Association publication recovery journal. `M/project-files.ts:44–78,134–166` | Small metadata; direct JSON | Recover uncertain publication first; do not claim unsaved work is already saved |
| `backups/<project-id>/*.pmv` | Previous complete versions, newest five retained after successful save; failed/uncertain copies can carry `.recovery.pmv`. `M/project-files.ts:88–132,157–164` | Up to many full media files; clone sharing on APFS is not byte duplication. Direct parser | Retain backups/recovery versions; do not prune during migration |
| External project sibling temporary / recovery files | Flushed staging file and conflict recovery copy from `M/durable-file.ts`, `M/project-files.ts:15–23` | Can equal complete project size; direct | Preserve/publish only after validating checksum and intended destination; incomplete data stays recoverable |
| User exports, caption sidecars, easing-library JSON exports | Movies/images/audio selected by save dialog; `.srt`/`.vtt` (optional language suffix) beside export; optional library JSON. `M/export-sidecar.ts:1–5,27–57` (32 MiB captions), `M/render-encoder.ts`, `R/panels/EasingPanel.svelte` | Potentially large ordinary files; direct | Leave user destinations in place, preserve queue/output references; these are not caches |

**PMV3:** `PMV3\n` magic (5 bytes), little-endian u32 JSON-header length, UTF-8 `{document,media:[{id,type,offset,length}]}` then raw media bytes (`S/project-container.ts:1–35`). Portable/browser saves still write this variant.

**PMV4 (desktop Save):** `PMV4\n` magic; immutable document/media byte ranges; tail JSON `{version:4,document:{offset,length,sha256},media:[{id,revision,type,offset,length,sha256}]}` and a **64-byte `PMV4END\n` footer** with index offset/length/SHA-256. Index ceiling 64 MiB and 100,000 media records. New snapshots/replaced media append inside a sibling clone, atomically published, not the live file. Check each checksum and range. `S/project-incremental.ts:1–59`; compaction policy and durability in `apps/desktop/docs/project-files.md:13–21`. A document has a u32 byte-length bound even though total embedded media has no total application cap. Rollback target must be a format the actual restore build can read; do not downgrade to plain JSON and lose embedded bytes.

**History has two storage representations.** Logical version 1: `{version:1,paths:'composition',index,entries:[{label,forward,backward}]}`; patch `{path:(string|number)[],exists,value?}`. Older root-based path history remains input. `index` is indispensable: it separates undo from redo. External callback/selection-only entries are excluded from persisted document history (`R/legacy/core/history.ts:314–350`). Retention: 1,000 entries plus a default 256 MiB shared live/parked budget (`:13–14,435–480`).

The profile may instead contain `{format:'powermove-history-dictionary',version:1,records,history}`. Forward/backward arrays then contain `{patch,refs?}`, with refs indexing immutable layer/edit records; paths can be root arrays or `comps/<id>/layers|edits`. Reject invalid refs; never silently open with incomplete undo. `S/history-memory.ts:1–17,75–125`. Unpack into kernel operations/inverses and re-export semantically equal history for rollback. Both embedded history and history-only media must survive migration, even when the current document no longer references those assets.

## 6. Extension and agent directories

All entries below are ordinary files readable without Electron; encrypted/runtime-owned authentication is the exception already described. Preserve private file modes. Native beta uses copies/overlays; cutover uses conflict-preserving file merges. **Eleven directory families**:

| Location | Payload and writer → reader | Size characteristics | Native target |
|---|---|---|---|
| `P/extensions/<extension-id>/` | Manifest, TS/JS/Svelte/CSS/assets, local forks, `.forked-from` pristine source and lineage. `M/extensions/registry.ts:98–145,394–420`; `fork.ts:61–109`, `rebase.ts:154–182` → registry/compiler/loader | Arbitrary source/assets; discovery/source export limits are not permission to drop files | Shared extension source unchanged; v1–3 compat, disabled installs and forks included |
| `P/extensions/{.staging,.trash,.updates}/` | Download stages, recoverable old installs, modified-install pending updates. `M/cloud/install.ts:169–171,263–327,465–553` | Can duplicate complete installs | Retain pending/trash work and provenance; complete/retain transactions atomically |
| Project-scope extension directories | `projectDirs` supplied to registry; sources outside profile; `M/extensions/registry.ts:708–713` | Unbounded external folders | Preserve project directory references; copy only for beta overlays/conflict resolution |
| `P/Agent Workspaces/<safe-project>/` | `inputs/{powermove-project.json,attachments,references}`, `artifacts/<run>`, `.powermove/extension-runs/<run>`, `.powermove/tmp` and command caches, `powermove-api` regenerated reference pack. `M/codex/workspace.ts:50–67,250–337`; `compatible-workspace.ts:280–286` | Can be many GiB; artifacts/agent-created files may be the only copy | Keep artifacts, staged extensions, edited files and reference relationships; rebuild only provably generated API/scratch data |
| `P/Agent State/<safe-project>/` | Main-owned session-v2 provider/authority/thread `.txt` resume handles; `.checkpoint.json` extension stages and baseline hashes; result schema/results and scratch. `M/codex/workspace.ts:64–67,144–155,232–265,377–383` | Many small files plus result JSON; independent of workspaces | Agent resume/checkpoint store; preserve thread/provider/authority distinctions and interrupted work; regenerate live handles only |
| `P/Agent Change History/<safe-project>/` | `change-set.json` version 1, before/after tree hashes, change records, `before`, `next`, `redo`, pending transaction snapshots. `M/codex/change-history.ts:9–12,40–48,170–191,235–300` | Each snapshot bounded by 4,000 files/32 MiB; retained trees can accumulate | Extension undo and crash recovery; keep pending publication, not only composition undo |
| `P/Agent Tasks/<sha256(project)>/<sha256(key)>.json` | `{version:1,rootThreadId?,records}`; task state, idempotency and orchestration relationships, maximum 512 records per file. `M/agent-orchestrator.ts:217–218,612–641`; `M/codex/index.ts:381–382` | Small per record but many scopes | Native agent orchestration archive; retain completed/cancelled task relationships |
| `P/Fork Updates/` | Private fork rebase workspace, linked to agent change history. `M/extensions/update-fork.ts:17–24` | Source-tree snapshots | Preserve unresolved conflicts and edits; no automatic replacement |
| `P/codex-runtime/` (or `codex-runtime-isolated/`) | Runtime-owned sessions/config/auth; file-only credentials, owner marker, user-resource symlinks. `M/codex/isolation.ts:6–21,35–98,109–125` | May be large; preserve without printing content | Keep same isolated runtime home; runtime owns auth renewal/logout; do not rebootstrap over intentional sign-out |
| `P/claude-runtime/` | Runtime config/session/auth owned by runtime; user resources shared, credentials not copied from terminal home; `.powermove-resource-backups/` preserves replaced resource entries (`M/agent-tools/user-resources.ts:28–31`). `M/claude/isolation.ts:6–22`; `M/agent-tools/user-resources.ts` | May be large; runtime-specific formats | Keep same isolated home and bundled runtime protocol; preserve config/skills/hooks pointers |
| External client and user-resource configuration | `~/.codex` or configured home, `~/.claude` or configured home, user MCP config, external desktop-client config and receipt. `M/agent-tools/user-mcp.ts:18`, `user-resources.ts:29–91`, `M/claude-desktop-mcp.ts:37–81` | User-owned files/symlinks, potentially secret | Keep references; preserve client registration through native executable. Do not indiscriminately copy terminal credentials or alter unrelated MCP entries |

Agent exports default to the public code template `~/Powermove` (`M/index.ts:1302`, `M/agent-export.ts:29`); preserve these user outputs. Temporary run/export ids are not stable native service credentials. Live process/IPC/pid state is reconstructed, but durable results and resume intent remain.

## 7. Caches, downloads and Chromium-managed files

**Nine cache/download families**; file inspection is possible without Electron. Rebuildability must be proven before deletion. In particular, an asset called a cache may still be the only available media copy.

| Location | Writer → reader, schema and size | Native disposition |
|---|---|---|
| `P/extensions-build/` | `M/extensions/compiler.ts:277–279`, registry `:145`; compiled JS/bundle/source-map files | Rebuild under the native toolchain; retain source and sourcemaps until equivalent activation succeeds |
| `P/Attachment Cache/<random>/...` | `M/shell.ts:30–35`; materialized attachment bytes for reveal/open | Preserve if archive closure cannot recover payload; otherwise rematerialize |
| `P/Media Tool Cache/<hash>.<ext>` | `M/media-path-cache.ts:84–128`; materialized imported bytes for agent tools, reset at service init | Relocate/rebuild from verified canonical objects; do not delete before verifying canonical bytes |
| OS temporary `powermove-*` proxy/encoder/tool directories | `M/media-proxy.ts`, `M/render-encoder.ts`, `M/agent-tools/media-tools.ts:401`; video/still/sequence conversions, upload chunks, encode outputs | Completed durable proxy payload is in IndexedDB; unfinished exports/conversions need recovery/cancellation, not resumable stale tokens |
| `P/models/transcription/{settings.json,<model>/model.json,<model>/*,.partial/<model>/*}` | `M/transcription/install.ts:68`, `models.ts:13–14,128–165,286–289,382–400`; selected model/language plus checksummed downloaded model files and partials | Preserve model/language and complete valid downloads; resume supported partials. Models may be hundreds of MiB/GiB; no forced re-download |
| `P/transcription-cache/<hash>.json` | `M/transcription/cache.ts:17–25,32–63`; transcript segments; cache key v4 includes file identity/model/language/span; default 400 disk entries/24 memory | Preserve useful transcripts; authored caption edits already reside in projects. Re-key carefully because file identity changes can invalidate hits |
| `P/transcription-warm.json` | `{key}` last successful app-version warmup; `M/transcription/install.ts:94,105–110` | Retain as checkpoint input; rebuild native warm marker |
| `P/runtimes/{<provider>.json,<provider> -> .versions/..., .versions/*}` | Version/integrity metadata and downloaded native CLI executables. `M/runtime-updates.ts:36–61,156–189`; `M/index.ts:933` | Validate/reuse pinned native runtimes; keep user/runtime state separate from replaceable binaries |
| Chromium automatic profile/cache state and updater downloads | `P/{Preferences,Local State,Network/,Cookies,Cache/,Code Cache/,GPUCache/,Dawn*Cache/,blob_storage/,Session Storage/}` when present; Chromium owns formats. Native updater also has OS cache/staged downloads. No application-specific writers beyond Electron dependencies identified | Preserve old profile intact; do not import GPU/code caches into native rendering. Recreate transient browser sessions and update staging; retain browser-origin extension state through logical/direct migration. TCC/Keychain are OS-owned, not recoverable by copying these cache files |

## 8. Host Media Store — engine branch only

**Three path families** beneath `P/Media Store`: immutable `objects/<first-two-hash-hex>/<full-BLAKE3>`; `keys/<sha256(renderer-key)>.json`; staging `tmp/*.part`. Source: engine branch `M/media-store/store.ts:1–23,34–42,83–125,186–218`; shared schema `S/media-store.ts:35–58`.

Key JSON: `{key,hash,size,type,revision,at,source?}` with source `import|stream|indexeddb|project|rendition`. Files are copied/cloned, fsynced and BLAKE3-verified before publication. `removeKey` removes only the mapping; objects remain. Stream chunks are 4 MiB and current implementation's stream limit is 256 GiB. Linked originals instead retain `{path,hash,size,mtimeMs?,dev?,ino?}` in project asset metadata. The current branch supports copied/linked storage and rendition bindings; it must not be described as shipped `v1.2.0` behavior.

Writer: `M/media-store/{store,service,ipc}.ts`; readers: the same service, native engine capability handles, renderer media-store adapter and restore build. Direct files/JSON can be read without Electron; BLAKE3/checksum verification is required. Target is the same host-owned native media layout, preserving key mappings, object hashes, revisions, copied-vs-linked intent and proxy provenance. Include beta/native-imported objects in handoff manifests. Do not collect them while another app/rollback version might still need them.

## 9. Blender legacy kinds (D9)

**Baseline discrepancy:** `git grep -i blender v1.2.0 -- apps/desktop` finds documentation/test labels and Blender-style UI descriptions, but **no Blender runtime, `.blend` importer, job schema or persisted surface store**. The implementation was removed before this tag by public commit `2b1fa379a`. Engine code still mentions Blender, but that is not evidence that the tag can recreate older Blender assets. D9 therefore requires older-version compatibility fixtures; do not fabricate a shipped on-disk cache.

To identify the actual older state, the following supplementary observations use **`2b1fa379a^`**, explicitly outside the tag baseline:

| Kind | Older schema / ownership | Reader/writer and size | Migration decision |
|---|---|---|---|
| Blender source/model assets | Asset `kind:'model'`, `format:'blend'`; source Blob, generated/imported `.blend`, ordinary model/image/texture assets; imported source links `{assetId,object}` | Older `R/legacy/gl/raster.ts:587,682` (256 MiB `.blend` cap); `S/blender.ts:280–282,528–540`; ordinary `PM.MediaStore` | Copy all source bytes and project references. No placeholder substitution |
| Jobs: **3 operations** | `BlenderJob {id,operation:'generate' or 'import' or 'render',recipe?,sourceAssetId?,snapshot?,assets:[{id,name,data:Uint8Array}]}`; output `{files:[{id,name,data}],manifest}` | Older `S/blender.ts:528–540`; `M/blender-service.ts:377–464`; temporary `powermove-blender-*/job-*/{request.json,manifest.json,assets,outputs}` | Preserve completed outputs/staged user work. Process ids/queues are not durable jobs; reconstruct from source/snapshot when needed |
| Generated models | Recipe, parts mapping, animated parameters, retired parts and `sourceAssetId?`; worker generation writes `model.blend`, import writes `imported.blend` with packed assets | Older `S/blender.ts:495–526`; `M/blender-worker.ts:350–361` | Preserve exact recipe/part/channel identities, source and packed dependencies |
| Render surfaces | Per-layer PNG outputs decoded/cropped to `{surface:ImageBitmap,rect}`. Frame owns key/time/composition/dimensions, scene revision, preview-vs-final, camera key, version and `Map<layerId,Surface>` | Older `R/core/scene3d/rendering.ts:29–51,141–192,564–611,669–763`; worker `:376–382`. In-memory cap: 96 frames or 512 MiB (retain at least one), width × height × 4 accounting | Capture available cached surface bytes with rect/key/revision/camera/preview metadata and feed native surface cache. Do not classify preview as final |

**Nonrecoverable-by-copy boundary:** these older `ImageBitmap` maps were in memory, not a persisted `P/Blender Surfaces` directory at the tag. If an older process holds the only cached rendered pixels, an export/capture from that process is necessary before it exits; stopping it loses that volatile cache. If it is already gone, copying a profile cannot recover pixels that were never saved. Sources and completed PNG artifacts must be searched before declaring data unavailable. D9 is not verified by this inventory; acceptance needs an older project + real cached-surface fixture and an importer/render path. Do not replace missing support with a placeholder or downgrade the requirement.

## 10. Read-only survey (aggregates only)

A profile survey confirmed scale and layout; it is **not a frozen `v1.2.0` fixture** (it also contains engine-branch storage). Logical bytes, not physical APFS allocation; no user content, ids, names or source paths were recorded.

| Location | Files | Logical bytes |
|---|---:|---:|
| `store` | 467 | 585,323,428 |
| `Local Storage` | 6 | 14,124 |
| `IndexedDB` | 605 | 15,744,680,705 |
| `extensions` | 154 | 5,249,017 |
| `Agent Workspaces` | 42,652 | 7,644,011,277 |
| `Agent Change History` | 686 | 88,674,117 |
| `Media Store` | 1,164 | 7,700,039,598 |
| `runtimes` | 44 | 673,463,847 |

Selected file-store aggregates: 47 project slots (196,411,402 bytes); 51 history slots (52,537,106); 10 journals (1,366,900); 95 metadata slots (83,933); 11 combined session slots (14,255); 32 project workspaces (39,147); 58 thread archives (7,459,511); 122 detached attachments (102,317,899); 8 extension-storage objects (5,708). Different slot counts require reconciliation; they do not justify dropping orphans. Zero observed env/cache files is not evidence that those code paths can be omitted.

## 11. Open reader/acceptance work and summary counts

| Inventory unit | Count |
|---|---:|
| Shipped registered file-store families | 52 (42 literal + 10 dynamic) |
| Shipped localStorage families | 3 + arbitrary trusted-extension keys |
| Engine-only flag / document-host families | 1 localStorage + 1 file store |
| IndexedDB database / store / payload families | 1 / 1 / 2 (legacy bare-id aliases included in media family) |
| safeStorage secret payload families | 4 |
| Other non-store profile file families in §4 | 7 |
| Project/association/recovery/output families in §5 | 6 |
| Extension/agent directory families in §6 | 11 |
| Cache/download families in §7 | 9 |
| Engine-only media-store path families | 3 |
| Older Blender job operations / surface representation | 3 / 1 per-layer surface + keyed frame map |

Current production code needs Electron for logical Chromium storage reads and `safeStorage` decryption. **No verified standalone migration of Chromium data/secrets was demonstrated here.** A correct direct reader plus Keychain access removes the running-Electron dependency for persisted state; implementing/testing that reader, secure handoff and rollback is S6/P1 work. Volatile older Blender surfaces are the special case that must be captured while their owning process still exists. Live closures, native handles, blob URLs, IPC tokens and GPU caches are reconstructed, not serialized as user-authored state.

Required gates: media only in IndexedDB with original deleted; proxy-only assets; transformed still/animation bytes; old bare asset ids; history/take-only media; already-offline sources; missing posters; inconsistent index/slot counts; packed/unpacked undo with redo cursor; agent resume and attachment closure; post-TR imports; skipped TR; profile snapshot with Electron running; interrupted/low-space migration; separate beta edits/deletions; native-imported media then rollback; and journal compaction after extended native editing.
