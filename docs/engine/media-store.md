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
  its `:poster`, `:preview…`, and `:rendition` keys, or a bare asset id from
  old projects) → `{ key, hash, size, type, revision, at, source }`.
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
  the rest) keeps its original as the asset's bytes; the VP9 playback proxy
  the legacy viewer needs is generated media under `<key>:rendition`, reused
  on the next open instead of converting again, and never stored in a
  project file. Such assets have `playbackProxy: false` and `rendition:
  true`. Assets imported earlier, whose stored bytes are the conversion,
  show "Using a converted copy · Relink to the original for full quality"
  with Locate Original… in the Media panel.
- **Content hashes** are recorded on every imported asset (`contentHash`)
  and used by the engine's handles and by relinking. The sampled
  fingerprint stays the storage key and the duplicate finder; only the full
  hash decides identity.
- The import summary says "Copied 12 clips · Linked 3" when anything was
  linked.

## Relinking

Automatic relink is by content hash only (`main/media-store/relink.ts`).
Opening a linked asset checks the recorded file: unchanged size and
modification time are enough; anything else is hashed. A missing or
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
- **Opening** restores embedded media through `PM.MediaStore.put`, which
  with the new engine streams it into the store, verified.

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
  removes its own stamp once none is. With the flag off, boot, tabs, the
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
| Hashing, store, classification, relink, service, handles, IPC | `vitest run src/main/media-store` |
| PMV4 stored and linked records | `vitest run src/main/project-save.test.ts` |
| Renderer store, decisions, bindings | `vitest run src/renderer/src/media/host-media-store.test.ts` |
| Save, export embedding | `vitest run src/renderer/src/legacy/core/project-file.test.ts` |
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

## Not done here

- Section 14.8 and the appendix D "Offline media and Blender at export"
  message are Phase 4a (offline handling, section 14.2). Linked media that
  is offline at export renders as missing media does today.
- The timeline label "Offline" (appendix D, Import and relink) needs the
  timeline extension to learn an asset's availability through the kernel
  API; the Media panel already shows "Media offline" with the missing path.

- A host-side copy of PMV4 media ranges into the store on open: today the
  renderer reads the ranges and streams them back to main.
- Image conversions (HEIC, AVIF, animated images) still store the
  converted bytes; only video playback proxies keep the original.
- Bookmark-based tracking of moved files (security-scoped bookmarks) and a
  Relink sheet for name, timecode, or duration proposals (4b).
- Garbage collection of unreferenced store objects (4b Storage).
