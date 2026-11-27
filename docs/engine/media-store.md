# Media store and linked media (Phase 2)

Design notes for plan section 21 ("Media management and project files"),
section 14.8, appendix D (Import and relink; Settings › Project › Media),
the section 20 agent rows that import files, and the Phase 2 gate items:
the IndexedDB copy verified by hash and never deleted, content-hash relink,
agent workspace and agent-imported files always copied, deleting a project
never touching a linked original, and the engine-switch round trip. The
plan stays the authority; this file records how the code meets it.

## Where bytes live

| Mode | Bytes | Document |
|---|---|---|
| Copied into project (default) | `<userData>/Media Store/objects/<2 hex>/<BLAKE3>` | asset as today, plus `contentHash` |
| Linked | the original file, where it is | asset `storage: "linked"`, `link`, `contentHash` |

With "Use New Engine" on, the store in main is the source of media bytes
(`apps/desktop/src/main/media-store/`). With it off, the renderer's
IndexedDB stays the source, exactly as before, and the store is a read-only
fallback for media that was imported while the flag was on.

Layout under `<userData>/Media Store` (owner-only, 0700):

- `objects/<2 hex>/<64 hex>`: immutable copies (0400) named by their full
  BLAKE3 hash. Content-addressed, so the same bytes in two projects, two
  keys, or two imports are stored once. On APFS a copy is a clone.
- `keys/<sha256(key)>.json`: a renderer storage key (`media:v2:<size>:<sha>`,
  its `:poster` and `:preview…` keys, `rendition:<BLAKE3 of the original>`
  for browser renditions, or a bare asset id from old projects) → `{ key, hash, size, type, revision, at, source }`.
  `revision` is the PMV4 storage revision (the hash unless the writer named
  one), so Save reuses ranges for unchanged media. `source` says where the
  bytes came from: `import`, `stream`, `indexeddb`, `project`, `rendition`.
- `tmp/`: partial writes, removed at launch.

Every object is committed only after it is fsynced and hashed from disk;
streamed bytes are also compared with the BLAKE3 the sender computed while
reading its source, so what the store hands back is what was put in.
Nothing in the store deletes an object: removing a key forgets the mapping.
Unreferenced objects are kept until the storage work in Phase 4b (Storage,
Clear…) or 5c.

## Hashing

`shared/blake3.ts` is BLAKE3 (plain hash, 32-byte digest) in TypeScript,
verified against the official implementation's digests on the standard
test-vector input (byte i = i mod 251) for 39 lengths from 0 to 4 MiB, under
any split of the input and at any alignment. It gives the same digest as
`pm-media::hash` (the `blake3` crate). Main does not load an engine addon
for it.

`main/media-store/hasher.ts` hashes files off main's thread on a pool of
worker threads (`media-hash-worker.js`, at most 3, stopped after 30 s
idle). A file larger than 16 MiB is split into 16 MiB segments; each
segment that does not hold the last byte is a complete BLAKE3 subtree whose
chaining value a worker computes, and the tail is hashed after them with
the chaining values pushed in order, so the digest equals a single-threaded
hash. As in `pm-media`, a file whose size or modification time is still
moving is waited on (1 s quiet for imports, 0 for the store's own files),
and a file that changes while it is hashed is hashed again (three
attempts). Measured single-threaded in Node 24 on the M4 Max: 520 to 531
MiB/s (three 256 MiB runs).

## Importing

`PM.MediaStore` keeps its shape; `media/host-media-store.ts` wraps it
(installed right after `core/media`).

- **Copy into project.** A picked or dropped file with a native path is
  copied by main (`putFile`: clone or copy, source stat checked before and
  after, then hashed from disk). Bytes without a path (conversions,
  clipboard, downloads, synthesized files) stream to main in 4 MiB chunks
  while the page hashes them in 1 MiB slices with a yield between.
- **Link to files.** Main hashes the original in place; the asset records
  `link: { path, hash, size, mtimeMs, dev, ino }` and no bytes are stored.
- **The setting.** Settings › Project › Media › When importing (Copy into
  project, Link to files) is project `media.whenImporting` (absent means
  copy). The row exists only with the new engine; the choice is an undoable
  edit.
- **Always copied:** items without a file; 3D models and Blender sources;
  imports whose bytes are a conversion (animated images, stills Chromium
  cannot decode); files under the app's data folder, which holds every
  agent workspace (`Agent Workspaces`, `Agent Tasks`, `Agent Change
  History`); files on removable camera media (macOS reports removable
  media, or the volume has a camera card layout: `DCIM`, `AVCHD`, `BPAV`,
  `XDROOT`, `MP_ROOT`, `PRIVATE/M4ROOT`, `PRIVATE/AVCHD`, `PRIVATE/XDROOT`,
  `PRIVATE/PANA_GRP`, `CONTENTS/CLIP`, `CONTENTS/CLIPS001`); and agent
  imports unless the call passes `storage: "link"`.
- **Originals kept.** The import never moves, renames, or writes the user's
  file. With the new engine, a video Chromium cannot play (ProRes, MKV and
  the rest) and a TIFF or HEIF still keep their original as the asset's
  bytes (stored, or linked when the project links files); the VP9 playback
  proxy or PNG the page's viewer needs is generated media under
  `rendition:<BLAKE3 of the original>` (the asset's `contentHash`, which
  for a linked original is its link's hash), reused on the next open
  instead of converting again, and never stored in a project file. Naming
  a rendition by its original's content, not by the storage key, keeps two
  originals that share a sampled fingerprint from showing each other's
  picture in the legacy viewer or the web export. While importing, the
  hash is the one main computed for the copy it just made: the renderer
  remembers the store record per stored Blob (`storedRecord`), so two
  concurrent imports under one key keep their own. With the store keeping
  originals, an import stores the video itself up front, so its hash is
  known before the proxy is made. Such assets have `rendition: true`
  (videos also `playbackProxy: false`). When a rendition is gone (another
  Mac, a cleared store), main converts the kept original again
  (`media-store:convert-still` and `media-store:convert-video`: the stored
  object or the verified link, read through a temporary link that names
  its format, since objects are named by hash). When main has neither, it
  says so (`missing: true`) and the page sends the bytes it holds
  (`beginPut` with no key, then `media-store:convert-upload`): main checks
  them against the page's BLAKE3, converts them, and removes them, so the
  store keeps nothing. That is the flag-off case of a project file saved
  with the new engine and opened on another Mac: the originals are only
  in IndexedDB, and without this the legacy viewer could not show them.
  The same path serves any restored video without a native path (bytes
  read back from IndexedDB or the store), for example a playable H.264 file
  whose first-frame probe timed out under load: before, its proxy failed
  for want of a path and the asset was re-imported from its source path
  with a new storage revision, so the next metadata-only Save uploaded it
  again. The same symptom had a second cause, the one seen in
  `project-file-media.spec.ts:9`: a boot or tab restore recovering the
  same media from its source path could store it under the shared key
  with a new revision just after a project file's media were restored
  there. Opening a project file now waits for asset restores in flight
  (`legacy/app.ts`, bounded at 60 s since a restore can wait on a cloud
  prompt) before restoring the file's media. The
  legacy viewer reads the same renditions with the flag off; renditions it
  makes itself go to IndexedDB under the same `rendition:<BLAKE3>` key,
  since the store is read-only with the flag off, and are reused there on
  the next open. A restored asset with `rendition: true` keeps its
  original as its bytes in either mode (the in-memory asset reports
  `rendition: true`, `playbackProxy: false`), so a later Save embeds the
  original, never the rendition. The web export (`player/export-web.ts`) first
  takes a kept rendition, then the browser bytes frozen before its first await
  when their source identity matches. With the engine on, a missing rendition
  is converted by reference through the existing host converter; bytes cross
  IPC only if main reports the original missing. Generated copies are cached
  by original hash. Engine-off exports keep their existing browser-copy path
  and error, and write no rendition. A job-engine rendition contract is still
  required; this is the existing host conversion path.
  Animated images are still stored as their conversion (an image
  sequence). Assets imported earlier, whose stored bytes are the
  conversion, show "Using a converted copy · Relink to the original for
  full quality" with Locate Original… in the Media panel.
- **Content hashes** are recorded on every imported asset (`contentHash`)
  and used by the engine's handles and by relinking. The sampled
  fingerprint (three 64 KiB samples and the size) stays the storage key
  and the duplicate finder; only the full hash decides identity:
  - reads, Desktop Save (with the flag on, and with it off for media the
    new engine imported, which lives only in the store), and IndexedDB
    migration go by the asset's own content hash, and a key that now names other content is never read in
    its place (two room-tone recordings can share a fingerprint, and so a
    key; the later import repoints the key, the earlier asset keeps its
    content);
  - an import whose content hash differs from a fingerprint match becomes a
    new asset (`MediaImport.match`); conversions, whose hash is of the
    converted bytes (`originalContentHash` is null for them), still match
    by fingerprint, so re-importing an original upgrades its converted copy;
  - agent `import_media` checks every asset that shares the fingerprint
    against the file's full hash (computed once) before answering
    `already_imported` (section 20, "deduplicated on the content hash");
    an asset without a known original hash (legacy media) matches only
    when its stored bytes hash the same as the file; one whose stored
    bytes are a conversion (a playback proxy, an image sequence) cannot be
    compared and still matches by fingerprint when no asset matches by
    hash.
  Posters stay keyed by the storage key, so two assets that share a
  fingerprint share a poster thumbnail until one is re-imported.
- The import summary says "Copied 12 clips · Linked 3" when anything was
  linked.

## Relinking

Automatic relink is by content hash only (`main/media-store/relink.ts`).
Opening a linked asset checks the recorded file: the same file (volume
and file number) with an unchanged size and modification time is enough;
anything else, including another file copied over the path with its
dates kept, is hashed. A missing or
changed file starts a search for candidates of exactly the recorded size:
a bounded walk (4,000 folders, 4 levels) of the recorded folder, its parent
and grandparent, folders other linked media of the window now live in, and
any extra roots; then Spotlight (`mdfind kMDItemFSSize == size`). Same-name
candidates are hashed first, at most 16 are hashed, and a candidate is used
only when its full BLAKE3 equals the recorded one. The app's data folder
and the store itself are never searched or chosen. A relink is an
engine-originated fact (section 6): a metadata write that is not undoable,
recorded in the edit log as "Relink media" with origin `engine`. When
nothing matches, the asset is offline ("Media offline", "Missing · path")
and is never silently replaced. A linked asset whose original is gone but
whose content is in the store (for example after opening an exported
project) reads the stored copy: content identity, not location, names it.

## Project files

- **Desktop Save (PMV4).** Linked assets are records without bytes in the
  index (`linked: [{ id, hash, size, path, type }]`; older readers ignore
  the field). Media in the store is named by hash (`SaveMedia.stored`) and
  main copies it from the store into the transaction file itself at finish,
  so the page uploads only media main cannot find. Ranges are reused by
  revision as before; the save keeps the user's file mode and all of
  `ProjectSave`'s durability (sibling transaction file, fsync, verified
  publish).
- **Export › Project and agent `save_project` (PMV3)** embed every linked
  original. In the exported document those assets become copied assets
  (`storage` and `link` removed, `contentHash` and `sourcePath` kept) and
  the engine stamp is recomputed, so the file is self-contained and opens
  in either engine. An original that cannot be read stays linked.
- **Opening** a project file with the new engine: main copies each
  embedded range from the file the window opened straight into the store
  (`media-store:put-project-range`, 64 MiB per call so the opening
  progress moves; reads go through that window's own project reader,
  `save.ts readProjectRangeFor`). The object is committed only when the
  bytes main read, and the copy read back from disk, match the SHA-256
  the PMV4 index records (PMV3 ranges, which record none, are compared
  with the bytes main read), and it is named by its BLAKE3. Records say
  `source: "project"`. An upload may finish without naming its BLAKE3 or
  a SHA-256 only when main read at least one chunk and the page sent none;
  an empty range has nothing for main to read and goes through the page.
  A range main cannot copy falls back to the page
  (`PM.MediaStore.put`, streamed and verified by BLAKE3); a checksum
  mismatch fails the open. The page path it replaces was not measured.
  The measurement is committed as an opt-in test
  (`src/main/media-store/project-range.measure.test.ts`): it drives the
  real IPC handlers over a generated project file and prints seconds per
  GiB and the longest event-loop stall in main:
  `PM_MEDIA_STORE_MEASURE=1 npx vitest run src/main/media-store/project-range.measure.test.ts`
  (`PM_MEDIA_STORE_MEASURE_MIB`, default 1024; `PM_MEDIA_STORE_MEASURE_RUNS`,
  default 3). Measured with it on 2026-10-14 (M4 Max, built
  hash worker pool, 1 GiB, three runs, `nice -n 15` while another
  worktree's Electron and a Rust build were running): 2.07 to 2.22 s per
  GiB including verification (461 to 496 MiB/s); the longest event-loop
  stall in main was 3.0 to 5.0 ms. (An earlier, uncommitted harness that
  drove `MediaStore` directly on an idle machine measured 1.43 to 1.50 s
  per GiB; it timed the store's chunk calls rather than main's event
  loop.)
  The page path (also the legacy engine's) now builds each range into a
  Blob from its bytes, folded every 64 MiB. It used to stage in OPFS, and
  Chromium keeps an IndexedDB blob from an OPFS file as a reference to
  that file, which was removed right after: the asset was re-imported
  from its source path with a new revision, so the next metadata-only
  save uploaded it again (the ledger's `project-file-media.spec.ts:9`).

## Switching engines safely

- IndexedDB is never written or deleted by the new-engine path. Media found
  only there is copied into the store and verified: the page reads the
  IndexedDB blob and hashes what it read; main commits the copy only when
  the bytes on disk hash to the same value. The record keeps the IndexedDB
  revision and says `source: "indexeddb"`. This runs for every read that
  falls back to IndexedDB and for each project's media (and posters) when
  the project first opens with the new engine.
- **The stamp.** `shared/engine-stamp.ts` sets `requiresEngine: "native"`
  at the project root while a native-only feature is in use (today: linked
  media and interpretation overrides; lanes that add `keepFrameRate`,
  `pitch`, or project `color` add a check to `NATIVE_ONLY_CHECKS`) and
  removes its own stamp once none is. Kept originals with renditions
  (ProRes and other videos Chromium cannot play, TIFF and HEIF stills;
  `rendition: true`) do not stamp the project: within this build the
  legacy viewer shows them through the kept rendition, or has main convert
  the original again when the rendition is gone, from the store, the
  link, or (flag off, fresh profile) the IndexedDB bytes the page sends.
  `e2e/media-store.spec.ts` covers that case end to end: saved with the
  flag on, opened with it off on a fresh profile, both shown, saved again
  with the originals, reopened without converting again. A project file saved with
  them embeds the originals, so a release that predates renditions (and
  has no media store to convert from) cannot display those assets; the
  engine switch the plan requires is within one build. With the flag off, boot, tabs, the
  Projects screen, opening a file, and `pm-open-project` refuse a stamped
  project with "This project uses the new engine. Turn on Use New Engine to
  open it."
- Copied media round-trips on → off → on: with the flag off, IndexedDB
  misses fall back to the store; media imported while off goes to IndexedDB
  and moves to the store on the next open with the flag on.
- Deleting a project deletes its agent workspace and never touches linked
  originals: the store only reads linked files, and files inside a
  workspace are never linked.

## The engine (contract 14)

Import completion is also a host-binding barrier: `ingestAsset` awaits
`HostMedia.flushBindings(project)` before returning. The debounced binding on
asset/project events remains useful for later document edits, but is too late
for an immediate export or a frame of a parallel image import. This is the
shared cause of rerun defects D4 and D5, fixed by `6a2f9587` before this lane.
Removing only the import's acknowledgement reproduces a cancelled immediate
MP4 export, both generated image assets resolving to null immediately, and
41 layer failures in the synthetic 500-layer project. With it restored those
three checks pass (zero render/layer failures). The new
`e2e/media-import-readiness.spec.ts` checks both engine settings, every image's
host resolution before any polling/extra flush, immediate export without an
offline prompt, and both images' exact exported pixels.

`import-converted-formats.spec.ts` now checks the original-preservation contract
in native mode: TIFF, HEIC and Matroska bytes hash exactly to the source fixture,
with separate PNG/WebM browser renditions. Legacy mode still checks stored
conversions. These storage assertions do not change the existing pixel checks.

Each window binds its open project's media in main (`mediaStore.bind`:
asset id → store key and hash, or link). `HostServicesCore` resolves
`mintHandle { mediaRead { asset } }` through `MediaStoreService.resolveAsset`:
the store object or the verified (or relinked) original, with its BLAKE3
as the grant's `contentHash`. The engine never sees a path except through
`resolveHandle` on its own host port. An asset id bound to different
content in two windows (a duplicated project whose copy was replaced) is
refused rather than guessed.

## IPC and security

`media-store:*` handlers accept only app windows' main frames (never an
extension sandbox frame). Uploads, reads, and bindings belong to the window
that made them and are dropped when it goes away. Keys, hashes, paths, and
chunk sizes are validated; reads and writes are at most 4 MiB per call.
Agents are not restricted in what they read; only Store extensions are
contained, and they have no access to this IPC.

## Tests

| What | Command |
|---|---|
| BLAKE3 vectors, splits, subtrees | `vitest run src/shared/blake3.test.ts` |
| Hashing, store, classification, relink, service, handles, IPC, project ranges, still and video conversion, converting uploads | `vitest run src/main/media-store` |
| PMV4 stored and linked records | `vitest run src/main/project-save.test.ts` |
| Renderer store, decisions, bindings, content identity | `vitest run src/renderer/src/media/host-media-store.test.ts` |
| Identity matching (fingerprint vs content hash) | `vitest run src/renderer/src/legacy/core/media.test.ts` |
| Save, export embedding, opening ranges | `vitest run src/renderer/src/legacy/core/project-file.test.ts` |
| Web export takes renditions | `vitest run src/renderer/src/player/export-web.test.ts` |
| Stamp and refusal | `vitest run src/shared/engine-stamp.test.ts` |
| Settings row | `vitest run src/renderer/src/legacy/ui/project-settings.test.ts` |
| The gate scenarios in the app | `playwright test e2e/media-store.spec.ts --workers=1` |

Measured on 2026-10-13 (M4 Max, release `pm_node.node` and
`pm_bridge.node` built in the worktree, hidden Electron under the machine
lock, `--workers=1`): `e2e/media-store.spec.ts`, 8 of 8, passes in 31 s.
Re-measured on 2026-10-14 after merging the wave 3 render lane: the eight
unit files above, 99 tests, pass, as do `src/main/agent-tools` (85) and
`src/main/engine` (63); `e2e/media-store.spec.ts`, 8 of 8, passes in
38.7 s. With the flag off
(default), the existing import, recovery, replacement, save, and agent
media specs (`import-prores`, `import-mp4`, `project-file-media`,
`menu-save`, `media-replacement`, `local-media-recovery`,
`agent-media-management`, `media-deletion-project`), 28 tests, pass.

Re-measured on 2026-10-14 after merging the wave 3 audio and wasm lanes
and the follow-up work above (content identity, main-side project range
copies, kept still originals, the OPFS staging fix; same machine, release
addons rebuilt in the worktree, hidden Electron under the machine lock,
`--workers=1`): `e2e/media-store.spec.ts`, 11 of 11, and with it
`agent-media-management`, `export-code`, `import-converted-formats`,
`import-mp4`, `import-prores`, `local-media-recovery`,
`media-deletion-project`, `media-replacement`, `menu-save`, and
`project-file-media`: 43 of 43 in 1.7 min. That includes the ledger's
`project-file-media.spec.ts:9`, which failed at the merge base
(`c9883147`) and passed 5 of 5 after the fix. Unit tests over the media
store, project files, save, agent tools, the engine host, the renderer's
media, legacy core, assistant, and player suites: 1,010 pass, 1 skipped.

Re-measured on 2026-10-14 after the review fixes for kept originals on a
flag-off profile (same machine, hidden Electron under the machine lock,
`--workers=1`): `e2e/media-store.spec.ts` (13), `project-file-media` (5),
and `agent-media-management` (3), 22 of 22 in 1.2 min; then
`project-file-media.spec.ts:9` with `--repeat-each=10`, 10 of 10 in 34 s.
With the `raster.ts` from before the fix, the new probe test fails as
expected: the reopened video was re-imported from its source path (no
proxy) instead of converted from its bytes.

## Not done here

- Section 14.8 and the appendix D "Offline media and Blender at export"
  message are Phase 4a (offline handling, section 14.2). Linked media that
  is offline at export renders as missing media does today.
- The timeline label "Offline" (appendix D, Import and relink) needs the
  timeline extension to learn an asset's availability through the kernel
  API; the Media panel already shows "Media offline" with the missing path.

- Animated images (GIF, APNG, animated WebP) still store their conversion
  (an image sequence); the engine has no reader for their originals' timing
  yet.
- A linked asset whose file was verified by hashing (its dates moved) is
  hashed again on every open until it is relinked; the recorded size, dates,
  and file number are not refreshed, because that would be a document write
  for every touch of the file.
- Bookmark-based tracking of moved files (security-scoped bookmarks) and a
  Relink sheet for name, timecode, or duration proposals (4b).
- Garbage collection of unreferenced store objects (4b Storage).


## K1, B1 and R3 follow-up (2026-10-18)

The export sheet and Add to Queue now await `HostMedia.flushBindings` before
submitting their captured project. The agent's `__mcp_context` does the same
before returning its project snapshot. The barrier first finishes migration of
IndexedDB-only media into the verified host store, then awaits main's binding
acknowledgement. A project switch during the await does not change which project
is exported. Flag off keeps the barrier a no-op.

Queue snapshots persist each copied asset's content hash when the entry is added,
including older projects that only have storage keys. Main binds every export
snapshot's copied hashes and linked originals to that job session before resync.
Host media capability requests carry their engine process and session to the
resolver, so those bindings survive a project switch, window close and queued-job
relaunch, and cannot be confused with an interactive session with the same number.
A later write under the same storage key does not change the queued content.
Completion, failure and cancellation release the session bindings. The live
window's bindings keep their existing lifetime and ambiguity checks.

A job's frozen entry is tried first: link, then hash object, then storage key
record (each copied asset keeps its key, also when it had no record at snapshot
time). If that is offline, the job falls back to live bindings from windows on
the same project id. A job waiting for media therefore resumes on its 2 s
re-mint when copied media is put later, when the user relinks it with Locate
Media in that project's window, or when a saved hash object is gone but the key
still has bytes. Windows on other projects are never consulted.

The unit regressions cover acknowledgement ordering, IndexedDB migration without
writing/deleting IndexedDB, copied and linked media after window release, reused
storage keys, session/process isolation, and release after success or submission
failure. The image-readiness e2e retains its immediate-import check, exact red and
green export pixels and zero import-time layer failures, then checks immediate
export after reopen and queued export after switching away and releasing the
window's bindings.
