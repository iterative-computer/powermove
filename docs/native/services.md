# Native application services (package 17)

`apps/native/crates/services` is package **pm-services**. The CPU implementation
runs on Linux without Node, Chromium, GPU, AppKit, WebKit, or a real user profile.
This is a services foundation with explicit host hooks, not an app-wide parity
pass. Captions/transcription are owned by another lane and are excluded here.

The reference is **v1.2.0** (`07ed5fa17ee640659cb075f9cad1af2095011d60`).
Inventory used `git grep -n 'ipcMain\|handle(' v1.2.0 -- apps/desktop`, then
followed registrations in `src/main/index.ts`, including event-based services
and services without IPC handlers. Tests and duplicated `* 2.ts` files are not
counted as additional active services: index imports `fonts.ts` and
`cloud-media.ts`. A `handle(` match alone is not proof of an IPC route.

In the table, **M/** means `v1.2.0:apps/desktop/src/main/`, **R/** means
`v1.2.0:apps/desktop/src/renderer/src/`, and **S/** means
`v1.2.0:apps/desktop/src/shared/`. “Hook” means the trait and Linux test double
exist; production application wiring is still required. Owners outside this
lane identify integration destinations, not new work claimed in those crates.

| Service | v1.2.0 source / provided operations | Native owner crate | Status |
|---|---|---|---|
| Project library and recents | M/storage.ts; M/index.ts startup restoration; M/windows.ts; R/legacy/core/projects.ts: list/index, open, duplicate, trash/restore, recent sessions | pm-kernel; pm-services::library; pm-shell | CPU filtering, ordered/deduplicated recents, typed open/duplicate/archive/restore/checkpoint forwarding implemented. Kernel remains sole writer; app actor adapter pending. Existing kernel owns create/rename/save/session policy. |
| Autosave, journal and quit flush | M/storage.ts (`storeSet`, serialized set, delete, flush, bootstrap and synchronous reads); R/legacy/core/projects.ts; R/legacy/core/history.ts | pm-kernel; pm-services::library | Checkpoint request preserves expected revision through hook. Existing kernel owns debounce, journal, history and save/quit durability; service never writes a project document. Shell shutdown wiring pending. |
| Portable project save/read, associations and recovery | M/save.ts; M/project-files.ts; M/project-save.ts; M/durable-file.ts; S/project-container.ts; S/project-incremental.ts: streamed open/read/close, save begin/chunk/finish/abort, overwrite/hash checks, recovery journals | pm-kernel::session/project_io/persistence; pm-shell | Already kernel-owned; no duplicate PMV writer or association registry added. Native file dialogs and app binding need macOS verification. |
| Backups | M/project-files.ts:88–164: previous complete versions, five-successful-version pruning, uncertain recovery copies | pm-services::storage; pm-kernel | CPU streamed immutable backup generations and SHA-256 inventory implemented. Keeps every successful and incomplete generation; never prunes. Kernel must supply a coherent snapshot reader; external-save backup binding pending. |
| Settings and extension storage | M/storage.ts; S/storage.ts; R/kernel/install.ts:539–567: app keys and `ext.<id>` objects, synchronous trusted read-after-write, async sandbox RPC, serialized bootstrap | pm-kernel::state/persistence; pm-services::settings; pm-runtime-v8 | CPU get/set/archive hook validates ordinary 32 MiB values, preserves unknown JSON and distinguishes null/deletion. Kernel owns application state. Special large project/history/takes keys stay in kernel, outside this ordinary-settings facade. V8 owns live JS identity and immediate trusted updates. |
| Service-local settings | M/storage.ts durability patterns; D-STATE service metadata families | pm-services::storage | CPU private directory, writer lock, immutable JSON generations, tombstones, capped reads and corrupt-state refusal implemented. Auxiliary service state only: not the legacy profile reader or an alternate kernel settings database. |
| Fonts catalog | M/fonts.ts:5–21: registry version, family enumeration, stable-version nonempty cache | pm-services::fonts; pm-engine-service / pm-node CoreText broker | CPU cache contract implemented, including copy ownership and unstable/empty/error recovery. FontHook must use the native main-thread CoreText broker; activation, notifications and font registration require macOS. |
| Media probe and import queue | M/agent-tools/media-tools.ts (`probe_media`, source sampling); R/legacy/core/media.ts (import/fingerprint); M/media-path-cache.ts (lookup/stage/chunks/finish/abort/release) | pm-services::media; pm-engine-service; pm-kernel | CPU FIFO id-keyed queue, idempotent enqueue, attempt-fenced completion, cancellation/retry, exact rational probe metadata, persisted queue restore implemented. MediaHook probes on a worker and admits through kernel. FFmpeg/canonical media ingestion, retained asset versions/Undo and host route binding pending. |
| Local/cloud/remote media grants | M/cloud-media.ts: status/prompt/download/open/read/release; M/remote-media.ts: fetch/read/release; M/media-path-cache.ts: owner-scoped staged sources | pm-engine-service; pm-services host adapters; pm-shell | Inventory complete; production owner-scoped token/byte-stream adapters and cloud-provider consent/hydration remain. Queue source paths are explicitly supplied; it never discovers a profile or reads an arbitrary source itself. |
| Playback proxies, preview conversions, image sequences/stills/animations | M/media-proxy.ts: preview begin/chunk/finish, proxy/sequence/image creation, animation frames, read/release | pm-engine-service; pm-services::media | Provenance and decoder identity represented in CPU jobs/cache. Actual conversion/decode uses engine hooks; no substitute pixel path or fake successful conversion. |
| Thumbnails/posters and waveform artifacts | R/legacy/core/media.ts, R/legacy/core/projects.ts, R/extensions/timeline/; M/agent-tools/media-tools.ts:447–506 for agent waveform generation | pm-services::media; pm-engine-service | CPU bounded artifact cache implemented. Thumbnail/waveform kind, original/proxy content identity, exact ticks/timebase, dimensions, decoder, colour pipeline, channels/sample rate enter the key; digest checked on read. Renderer-managed posters/timeline waveforms are not a separate main IPC cache at the tag. Agent waveform generation is main-owned FFmpeg work; engine adapter pending. |
| Logs | M/log.ts: trusted validated level/text, single-line renderer stderr messages | pm-services::diagnostics | CPU local structured logs with explicit host text redactor, control normalization and bounds. Safe default omits text. No network sink. App stderr routing and broader error-code catalog pending. |
| Diagnostics bundle | M/log.ts; M/agent-tools/media-tools.ts and R/engine-connection diagnostics (separate diagnostic sources) | pm-services::diagnostics | Additive CPU deterministic JSON export: last 256 structured log codes/levels and explicitly selected crash records, at most 128. Never exports log text, file paths, environment, attachments, project data or raw OS dumps. No general diagnostics-bundle main IPC handler found at the tag. |
| Crash reports and recovery | M/renderer-recovery.ts; M/index.ts renderer-failure hooks | pm-services::diagnostics; pm-kernel; pm-shell | CPU stable-id local crash/recovered records with restricted process/reason vocabulary and archived revisions. No dedicated crash-upload service found in tagged main. Native panic/process termination capture, recovery UI and journal replay binding pending; platform proof on macOS required. |
| Store search/catalog metadata | M/cloud/store-client.ts; M/cloud/store-ipc.ts: browse/extensions/detail/release/tree/file/compare/library/versions | pm-services::store; pm-mcp; extension runtime host | CPU HTTPS-origin-bound search and batched version metadata client implemented, query encoding, tagged category vocabulary, 50-item pages/200-repo batches, same-origin content-addressed icon policy, capped responses and HTTP errors without body leakage. Other catalog/detail/source/compare endpoints and MCP result adapters remain with host integration. No credentials are stored or discovered by this crate. |
| Store install/update metadata | M/cloud/install.ts; M/cloud/store-ipc.ts: install/update/checkUpdates/uninstall; provenance/trust/pull modules | pm-services::store; pm-runtime-v8 / extension host; pm-kernel | CPU install/update plans refresh latest metadata, refuse unavailable/yanked/mismatched releases, check API 1–4, manifest identity/version, SHA formats, safe unique paths, tagged file/count/byte limits and unchanged update id. Plans retain manifest permissions and unknown release metadata. They are not installed/trusted receipts. Archive download/content verification, scans, fork merges, activation and uninstall archival remain host work. |
| Store trust and publishing | M/cloud/store-ipc.ts; M/cloud/trust.ts; M/cloud/publish.ts: trust/untrust, prepare/publish/yank | Extension runtime host; pm-mcp | Inventoried, outside this metadata-only implementation. No publish, yank or remote install-list mutation API in pm-services. Existing extensions/data remain untouched. |
| Cloud account and registry URL | M/cloud/service.ts; M/cloud/ipc.ts; M/cloud/auth.ts; M/cloud/session.ts; M/cloud/registry-url.ts: account/status, social/email auth, handle, remember installs, sign-out/delete, registry switch | pm-first-launch; native account host | Inventoried; auth/session/Keychain migration and network client are not duplicated here. Registry switching still needs host sign-out/confirmation and credential isolation. |
| Updater check | M/updates.ts: status/check/install, 30 s startup / 4 h polling; M/runtime-updates.ts; M/whats-new.ts | pm-services::updater; pm-first-launch; pm-shell | CPU TR/migration/OS/package routing and version/minimum-OS/beta/no-downgrade checks implemented. Trait returns feed metadata only. macOS 13 routes to final Electron-compatible metadata; native-cutover metadata requires macOS 14+, migration complete and verified TR receipt. Nonpackaged/ineligible hosts make no feed call. Timer driving, signed updater runtime, What's New state and app result adapter pending. No download/install/quit/publish path. |
| File associations and single-instance file opens | M/index.ts:815–860; M/windows.ts; M/save.ts; apps/desktop/electron-builder.yml | pm-services::entrypoints; pm-shell; pm-kernel | CPU validated absolute `.pmv` queue, deduplication, failure-preserving retry, `.pmv`/`powermove` registration hook implemented. Dev does not claim associations without explicit override. LaunchServices, bundle identity, single-instance routing and open-file delivery require macOS. |
| Deep links | M/cloud/service.ts:29–53; M/cloud/auth.ts:85–97; M/index.ts:815–846 | pm-services::entrypoints; native account host; pm-shell | CPU auth link parsing and eight-newest cold-start queue implemented; overflow is inspectable. Invalid/ambiguous targets/duplicate callback fields refused without echoing callback data. No Debug/Serialize on AuthLink. Auth owner must check pending state/expiry/PKCE before exchange. Drain auth and files independently after migration; failure retains unprocessed entries. |
| Windows/tabs, initial project, popouts and sandbox focus/termination | M/index.ts; M/windows.ts; M/panel-popout.ts; M/permissions.ts | pm-shell; pm-compat-host; pm-runtime-v8; pm-kernel | Inventoried, outside CPU services implementation. Native shell/focus/surface and extension-generation ownership remain with those lanes. |
| Native menus, edit/text services, confirm, clipboard, paste, haptics, theme and capture | M/menu.ts; M/context-menu.ts; M/text-context-menu.ts; M/native-edit.ts; M/native-confirm.ts; M/clipboard.ts; M/media-paste.ts; M/haptics.ts; M/theme.ts; M/capture.ts | pm-shell; pm-design; pm-compat-host | Inventoried, macOS integration and relevant shell/compat contracts; not implemented in this crate. |
| Shell external/reveal and attachments | M/shell.ts: openExternal, extension permission-gated open, media-source/attachment reveal | pm-shell; extension runtime host | Inventoried, requires native consent/permissions and scoped file grants; not treated as an unrestricted URL/path service. |
| Extension discovery/source/compiler/watch/protocol and environment variables | M/extensions/index.ts and registry/compiler/discovery/assets modules; M/env/ipc.ts; M/env/store.ts | pm-runtime-v8; pm-compat-host; pm-first-launch; pm-kernel | Inventoried, existing extension host and encrypted-variable contracts; not reimplemented or pruned here. |
| Render/encode/export destination and sidecars | M/render-encoder.ts; M/save.ts; M/export-sidecar.ts; M/agent-export.ts | pm-engine-service; pm-kernel; pm-shell | Inventoried; existing native engine job/export ownership. Captions/sidecar contents excluded from this lane. |
| Agents/MCP/provider runtimes/notifications and creative-app access | M/codex/index.ts; M/agent-tools/bridge.ts; M/agent-tools/external-server.ts; M/agent-tools/mcp-stdio.mjs; M/claude-desktop-mcp.ts; M/agent-notifications.ts; M/creative-workspace.ts; M/compatible-provider.ts; M/runtime-updates.ts | pm-mcp; native agent host; pm-shell | Inventoried; agent execution/approvals/provider credentials, orchestration and external client registration remain with their lanes. No messaging or auth operation performed by this lane. |
| Onboarding and release-note persistence | M/onboarding.ts; M/whats-new.ts; M/index.ts | pm-first-launch; pm-shell; pm-kernel | Inventoried; preserve completion/seen markers through migration, separate from update installation. |

## CPU API and durability

`ServiceWorker::start` initializes on its dedicated Rust worker, accepts at most
64 queued tasks and returns a nonblocking `Pending::try_take` receiver. Submission
never waits for disk, a hook, main or JS. Full/disconnected queues fail explicitly.
Task errors leave the worker usable; initialization failure disconnects pending
calls. Dropping the sender does not join on main; already admitted tasks drain.
The app must submit/poll its final checkpoint/flush before dropping the worker.
No kernel actor callback or GPUI action should call these I/O methods inline.

Library and settings persistence go through the one existing KernelHandle; no
service attaches a new library or creates a second document/settings authority.
`FontHook` accesses the existing CoreText broker. `MediaHook` must probe/import
an immutable source and deduplicate admission by job id; the kernel owns asset
mutation and Undo. Host network/OS traits are absent until explicitly supplied.

`ServiceStorage::open` requires an absolute private path. A new root is created
with mode 0700; an existing root must have the service marker and pass symlink
checks. It holds an exclusive writer lock. Files use 0600. Generation publication
streams into an exclusive `.pending` file, fsyncs it, publishes a durable SHA-256/size receipt, atomically hard-links a
no-clobber `.data` name, then syncs directories. Staging names remain as archives,
sharing the inode after successful publication. Failed partial writes stay
unpublished; retries skip their numbers. Every read verifies its publication receipt, including syntactically valid JSON.
Settings corruption is an explicit
recovery error and cannot be hidden by writing a fresh empty value. Tombstones
retain previous JSON values. There is no garbage collector or retention deletion.
This is a new auxiliary format, not a replacement for the kernel's V1_2/PMV4
rollback formats. Backups consume supplied snapshot streams and never open the
original project or legacy profile. Filesystem durability and hard-link support
must be checked on the actual native service volume.

The import queue separates start/probe/completion. A completion must match the
running job's request and attempt; cancel/retry/restart fences old results.
`persist` is an explicit commit boundary: the host must persist transitions before
acknowledging them to a durable caller. Restore requeues in-flight attempts and
retains completed/failed/cancelled jobs. A crash between kernel admission and
queue persistence requires idempotent kernel admission by job id. An admission
failure or durability warning must not be converted into an empty success.
Original and proxy-only provenance are distinct throughout. There is no automatic
relabeling of a proxy or approximate thumbnail/waveform hit.

The Store transport must cap streaming reads, retain HTTPS verification and
same-origin redirect/auth binding. The client also checks returned body lengths;
no credential is part of its API. Search retains listing fields while validating
identity/icon metadata; this is not yet the complete frozen Store wire validator.
Install plans validate metadata consistency, not archive contents or signatures.
They neither grant trust nor execute code. Host extraction must verify all archive
and Git blob/tree hashes, normalize destination paths, preserve local edits and
archive prior versions before activation. No extension, cache or legacy consumer
was removed by this change.

V1–3 `api.storage`, `api.log` and live `api.services` keep their realm-local
identity/read-after-write contracts. These Rust data/worker hooks are backing
operations, not a replacement async public extension API. V8 retains the full
synchronous logic/service cluster; WebKit remains a remote DOM renderer.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Keep project persistence, app settings and recents owned by kernel; expose hooks | N3 exact state; kernel single writer; N7 id/revision addressing. |
| Immutable generations/tombstones, preserve partials and all backups instead of five-version pruning | N4 archive rather than delete; N2 stronger recovery than the shipped retention policy. Disk reclamation requires a later proven archive policy. |
| Fail on corrupt latest data rather than silently returning defaults | N3 correctness and N4 no silent user-data loss. |
| Dedicated bounded Rust worker with nonblocking submit/poll | N1 responsive editing, N7 inspectability, N8 main never waits on JS or service I/O. |
| Preserve original/proxy provenance and exact cache identities | N3 authored intent, N4 retain offline/proxy-only work, N1 reliable previews. |
| Keep stable-version font cache behavior and owned snapshots | N6 retain successful source contract; N2 recover from empty/unstable enumeration. |
| Export structured diagnostics without free text; local crash records only | N1 useful inspectable failures; public-repository/no-user-data constraint. |
| Store metadata plans refresh latest and retain permissions/unknown fields; no fake install receipts | N3 correct state, N4 preserve extensions/forks, N6 keep the real installation contract for the extension host. |
| Updater returns only TR-gated metadata and keeps macOS 13 on compatible route | N4/OS-base D8; explicit lane restriction. Signed install/release gates remain open. |
| Preserve eight-newest auth queue, report overflow, keep file opens unbounded/deduplicated | N6 source callback behavior; N4 project opens are never evicted by auth traffic. |
| Reject credential-bearing/ambiguous auth targets and duplicate state/token fields | N3 unambiguous authored protocol; no Electron URL-parser quirk is promoted to a requirement. |
| Keep all existing consumers and use an owned isolated Linux manifest | Prune rule; strict lane ownership leaves shared lockfile/macOS wiring to integration. |

## Verification and remaining integration

Tests use fresh temporary directories and synthetic IDs/media/metadata only.
There is no real-profile read/write, live Store request, auth exchange, updater
feed access, OS association change or crash upload. No Electron/Node test runtime
is needed. Source inspection is evidence of the tagged contract; these doubles
are not frozen-editor or signed-macOS end-to-end parity evidence.

**58 tests pass** (27 foundation/worker, 7 media, 3 diagnostics, 12 Store,
4 updater, 5 entrypoint tests; see the test modules for exact grouping). Strict
Clippy with `-D warnings`, formatting and whitespace checks pass.

Linux verification commands (Rust 1.96.1):

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-services
cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-services --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/Cargo.toml -p pm-services -- --check
```

The isolated manifest points to the **same `src/lib.rs` and tests** and keeps its
lockfile inside the owned crate. The public manifest participates in the existing
`crates/*` workspace glob. The required `cd apps/native && cargo test --locked
-p pm-services` was attempted and refuses to update `apps/native/Cargo.lock`,
which has no pm-services entry. That shared lockfile is outside this lane's owned
paths and is unchanged. The integration owner must resolve/commit its entry,
then rerun the locked workspace command on Linux/macOS as supported. No existing
crate tests were modified; the shared workspace suite is not claimed green here.

Required integration outside owned paths: bind KernelHandle and the real state
transaction/checkpoint acknowledgements; supply FFmpeg/media/import/version/Undo
and CoreText brokers; connect the V8 facade and MCP schemas; add scoped cloud/
remote-media grants; implement the credential-owning Store transport, full wire
validation, verified archive/install/merge lifecycle; wire native crash capture,
entry events and normal save/quit barriers. Caption/transcription work remains
with its owning lane.

Required macOS checks: real CoreText version changes and font activation races;
LaunchServices `.pmv`/`powermove` registration, cold/second-instance open delivery
and bundle coexistence; pending-state/expiry/PKCE auth completion; native media
imports/cancellation/proxy recovery; APFS backup/failure/power-loss and full-sync
behavior; crash recovery and shutdown flush; signed TR/migration receipt, minimum
OS routing and final Electron-compatible metadata. AppKit, WebKit, GPUI and signed
updater behavior were not run or simulated as passed on Linux. No feed, tag,
release, publication, or production updater was changed.

## Design review follow-up

The governing review is [na-cloud-services](design/reviews/na-cloud-services.md).
This lane supplies data contracts only; shell rendering remains with its owner.

| Review item | Implementation / selecting rule |
|---|---|
| 1 — typed UI errors | Every fallible service and host hook uses `Result<T, ServiceError>`. `ServiceErrorCode` is stable, serializable data; Display/Debug/source remain developer diagnostics and must never be used as UI copy. I/O and HTTP failures have typed classifications, bounds use `TooLarge`, invalid PMV admission uses `InvalidProject`, and worker initialization/task failures retain codes. The UI owns localized sentence-case copy and project-name substitution. N1 useful failures, N3 explicit meaning, N7 inspectable codes, N8 nonblocking worker. |

At this step **63 tests pass** (the original 58 plus five error regression tests),
with strict crate Clippy and formatting checks. No user-facing error sentences
were invented or renderer/theme files changed. Host adapters must now return the
typed service result and select copy by code; `anyhow` conversion remains available
for legacy host failures, preserving nested typed/I/O codes without text matching.

| Review item | Implementation / selecting rule |
|---|---|
| 2 — Mac library vocabulary | Verbatim menu labels `Move to Trash` / `Put Back` and filter label `Recently Deleted` are exported; internal Archive/Restore/trashed names remain documented and keep the same kernel forwarding. Review is the copy spec; N3 preserves action meaning, N4 retains all data. |
| 3 — library row facts | Optional `ProjectSummary` carries thumbnail asset ID, duration in milliseconds, dimensions and an exact rational fps, supplied by kernel on save. Missing old-row summaries remain unknown; list/recents preserve the facts. `modified_ms` remains an instant for the shell's system relative formatter. N1 pro library facts, N3 exact rates, N4 old indexes still open, kernel single writer. |

At this step **66 tests pass**, including menu/action forwarding, old index
round trips and saved display-fact retention through list/recents. Strict crate
Clippy passes. The integration owner must populate summaries on kernel save and
resolve thumbnail IDs; shell uses the system relative formatter, never a cached
English date. Native menu presentation, date localization and thumbnail drawing
require macOS verification. No visual values or new copy were introduced.

| Review item | Implementation / selecting rule |
|---|---|
| 4 — update-sheet facts | `FeedMetadata` requires the feed's display `name` and `size_bytes`; no values are synthesized. `notes` are explicitly untrusted Markdown bounded to 64 KiB of UTF-8. Shell must disable executable HTML/remote resources, use pm-design's `reading` / `reading_leading`, and open links through its safe external-browser capability. Verbatim title/buttons are exported, with Install Update documented as accent. N1 standard update sheet, N3 preserve source metadata, N8 no rendering/JS on the services worker. |

The six updater tests pass, including unchanged TR/macOS-13 routing, exact sheet
metadata/Markdown round trip and the UTF-8 byte boundary returning `TooLarge`.
Strict crate Clippy passes. Feed adapters must supply the new name/size fields;
there is still no production feed parser/installer in this crate. The shell owner
must render the specified sheet and verify browser links and pm-design styling on
macOS. Metadata is never interpreted as executable markup by this lane.

| Review item | Implementation / selecting rule |
|---|---|
| 5 — font faces | `Face { name, weight, italic }` and `FontCatalog::faces(family)` use the same required `FontHook` broker as family/version enumeration. Registered names and fractional CSS/OpenType weights are preserved; owned face snapshots are fetched fresh so activation cannot leave a stale face cache. Shell draws each family in its own face and shares face facts with the inspector Weight row. N1 pro font selection, N3 broker facts rather than inferred names, N6 existing family-cache contract, N8 worker-only broker calls. |

**71 tests pass**, including three face tests for family grouping/weight/italic
facts, owned results, activation changes, empty results and typed broker failure
recovery. Strict crate Clippy passes. The CoreText broker adapter must implement
`faces` and map its native weights into the engine's CSS/OpenType weight scale;
macOS must verify activation, face selection, family-font submenus and inspector
Weight choices. This lane adds no font renderer, face synthesis or visual metric.

| Review item | Implementation / selecting rule |
|---|---|
| 6 — recovery document name (nice-to-have) | Optional `CrashRecord.recovered_document` retains the kernel's user-facing name for the next-launch notice. Old records load with no name; prior crash generations remain archived. Diagnostic exports use an explicit field allowlist and omit the name. The review's sentence and one Show button remain shell-owned, using `reading`. N1 useful recovery, N3 exact names, N4 old records and revisions preserved. |

The five diagnostics tests pass, including old-record round trip, next-launch
name retention after reopen, prior-generation preservation and export omission.
Strict crate Clippy passes. The recovery owner must supply the document name;
the shell owner must verify the specified notice and Show action on macOS. No
user-authored name is added to log or bundle output.

| Review item | Implementation / selecting rule |
|---|---|
| 7 — failed-open path (nice-to-have) | `EntryQueue::failed_file()` exposes the exact retained path and typed file-error code for the verbatim `Show in Finder` action. File failure context stays independent of auth failure and advances/clears on retry; failing and later entries remain queued. N1 actionable failure, N3 exact path/code, N4 no dropped opens, N7 inspectable context. |

Final verification: **75 tests pass** (58 existing, 17 added), strict crate Clippy
with `--all-targets --no-deps -- -D warnings`, Rust formatting and diff whitespace
checks pass. The two new entry tests cover exact Unicode/case-preserved paths,
FIFO retention, retry, advancing failure context and independent auth errors.
Gitleaks scans run before every commit; all fixtures are synthetic.

The required `cd apps/native && cargo test --locked -p pm-services` remains blocked
because the shared workspace lock needs updating for this crate. It was attempted;
no shared lockfile or other lane's source was edited. The isolated locked manifest
continues to compile/test the same source files. The integration owner must add
pm-services to `apps/native/Cargo.lock` and rerun the locked workspace suite.

All seven review items are complete within the service-data scope. Remaining
host integration: typed error-to-copy mapping; kernel save summaries/thumbnail
resolution; native relative dates; feed name/size population and safe Markdown
sheet rendering; CoreText face enumeration/weight mapping; recovered-document
name supply and notice; Finder action wiring. Each is owned outside this lane.
macOS must verify those menus, localized dates, update-sheet typography/accent/
links, font family submenus/Weight rows, recovery notice and Finder selection.
No UI was drawn, no visual value was chosen, and no DESIGN-TODO(opus) placeholder
was needed. Opus review is still required for the eventual visual output.
