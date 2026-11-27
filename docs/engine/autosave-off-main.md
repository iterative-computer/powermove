# Autosave serialization off main's thread

Plan section 26, amendment 5 (Phase 2 item). `docs/engine/spikes/S1.md` traced the
remaining frame drops and the input-latency p99 on every bridge path to main-thread
stalls during autosave. This note describes the change and the measurements.

## What ran on main

Every autosave sends the renderer's recovery state to main's store:

- `project.<id>` (the whole document) through `store:set` as a structured-clone
  object, on checkpoints (every 50th revision, a full journal, and every Save);
- `projectHistory.<id>` (the undo history) through `store:set-serialized` as a JSON
  string;
- small keys (registry, metadata, journal).

Main then ran `structuredClone`, history packing (which serializes every undo record
once to intern it), and `JSON.stringify` for an object, or `JSON.parse`, packing, and
`JSON.stringify` for a string, and kept the parsed values for the renderers'
synchronous reads. A native Save also parsed the whole received document (with its
undo history) in `ProjectSave.finish` before publishing it.

## What runs where now

- `store-engine.ts` (`StoreEngine`) owns parsing, history packing, serialization,
  validation (byte limits, JSON), the per-key debounce and write queues, the atomic
  write sequence (temporary sibling, fsync, close, rename), startup cleanup,
  quarantine, legacy history migration, and the registry reconciliation report.
  The write sequence and messages are unchanged.
- `persistence-worker.ts` hosts the engine and `validateProjectDocument` on a
  `worker_threads` thread inside the main process (`out/main/persistence-worker.js`).
  `persistence-thread.ts` is main's handle on it.
- `storage.ts` keeps, per key, the encoded JSON bytes the engine published (a
  `Buffer`, outside the V8 heap) and answers synchronous reads from them. It
  validates keys synchronously, as before.
- Heavy keys (`project`, `projectHistory`, `projectState`, `projectJournal`,
  `agentThreads`, `agentAttachment`) are listed at load without being read (main's
  lazy store). The first read asks the engine (`read`; synchronous reads wait for it
  with `Atomics.wait`, ceiling 60 s), which migrates a legacy history or quarantines
  a corrupt file then. Loaded heavy values share a 64 MB budget, least recently read
  dropped first; open, leased, most-recent-of-kind, and not-yet-written values stay.
  The engine keeps no session it has not read: a metadata-only `projectState` write
  for an unread or evicted session reads its stored history from disk first, and is
  refused if that read fails.
- `ProjectSave.finish` calls an injected validator; the desktop passes the worker's.
  Backups, the commit journal, `publishFile`, read-only (newer-reader) handling, and
  the `minReaderVersion` stamp are untouched.
- Hosts without the worker (the `powermove serve` host, tests that substitute the
  filesystem) run the same engine on their own thread through `InlineStoreChannel`.

## Ordering and reads

Requests carry increasing ids and the engine answers in arrival order. A set is
published to main (`committed`) once it is encoded and accepted; the write follows
after the debounce and is confirmed (`written`). A refused value is never published:
the invoke rejects (`store:set-serialized`) or the sending window gets `store:error`
(`store:set`, `store:delete`), as before. Main also reports it through `onError`.

Sibling windows hear about a key (`store:changed`) only after it is published, so
their synchronous refill finds it ready. If a synchronous read does reach a key whose
newest value is still being encoded, main waits for that one answer: the worker bumps
a shared counter after every reply, and main uses `Atomics.wait` and
`receiveMessageOnPort` (ceiling 5 s, then the last published value). Bootstrap only
waits for non-lazy keys; lazy keys are listed by name.

## Worker faults

If the worker exits unexpectedly, a replacement starts, requests in flight fail and
are reported, and the store re-sends every published value not yet confirmed
written, so no accepted edit is lost. After five replacements the store stops
restarting and reports every later write as failed (`Saving is unavailable until
Powermove restarts.`). Worker threads load from the asar archive in Electron 44
(checked with a packed test app).

## Measurements

`apps/desktop/scripts/profile-autosave-main-thread.mjs` runs the built app hidden on a
disposable profile: 884 layers with 48 keyframes each (a 9.9 MB document) and an undo
history of 42 MB (one batch) or 124 MB (three batches; the editor's history budget).
Main samples its event loop every 2 ms. Ten autosave cycles alternate checkpoint and
journal writes, then three native Saves. Same machine (Apple Silicon, shared with
other agents), 2026-10-12. "Before" is `feat/native-engine` before this branch; an
independent review re-measured `fe3a8302` at 277 ms for both autosave and Save with
the 124 MB history. "After" is this branch at its final commit. A hidden app's
timers are coalesced, so the idle floor is 14 to 16 ms.

| Scenario | Main's longest stall before | After | Gaps over 50 ms before | After |
|---|---|---|---|---|
| Autosave, 42 MB history, all cycles (max / median of cycle maxima) | 88.4 / 84.9 ms | 50.9 / 47.4 ms | 15 | 2 |
| Autosave, 42 MB history, journal cycles (max / median) | 84.9 ms | 28.6 / 15.2 ms | 5 | 0 |
| Native Save, 42 MB history (max / median) | 98.0 / 96.9 ms | 50.8 / 50.3 ms | 11 | 2 |
| Autosave, 124 MB history, all cycles | 270.7 / 245.9 ms | 66.2 / 47.4 ms | 15 | 2 |
| Autosave, 124 MB history, journal cycles | 270.7 ms | 30.8 / 29.7 ms | 5 | 0 |
| Native Save, 124 MB history | 270.9 / 264.1 ms | 49.9 / 49.8 ms | 12 | 0 |

The gaps over 50 ms that remain are checkpoint cycles and Saves, 50.3 to 66.2 ms,
explained below. Earlier runs of this branch landed within a few milliseconds of these
figures (42 MB autosave max 48.9 and 65.7 ms in two runs; the machine is shared).

Synchronous handler time on main for `store:set-serialized projectHistory`: 226 to
262 ms before, 6.1 to 7.9 ms after (124 MB; copying the string to the worker) and 2.2
to 2.4 ms after (42 MB). For `store:set project` (9.9 MB object): 54 to 65 ms before,
19.2 to 20.6 ms after.

Main's memory after the 124 MB run (`process.memoryUsage`, MiB): heap used 529 before,
261 after; external 516 before, 382 after; RSS 1,342 before, 1,576 after (the
worker's heap is in the same process). After the 42 MB run: heap used 102, RSS 646.

## Tests

Run serially (one vitest worker, one Electron instance) on the final commit:

- Unit: `persistence-thread`, `storage`, `project-save`, `project-files`,
  `native-ipc`, all of `src/shared` and `src/server` (including
  `reconnect.integration`, which reads a session synchronously right after writing
  it on the `powermove serve` host): 369 passed in 36 files. `history-memory`,
  `project-container`, `project-incremental`, `reader-version`, `store-keys`,
  `extensions-lifecycle`, `file-upload`, `media-path-cache`: 44 passed.
- e2e (`--workers=1`): `project-save`, `project-save-large-history`,
  `project-save-large-media`, `menu-save`, `persistence`, `memory-history-persistence`,
  `large-store-startup`, `project-panel-persistence`, `agent-project-persistence`,
  `agent-revision-recovery`: 16 passed, 3 failed. All three failures are listed in
  `apps/desktop/e2e/quarantine.json` as failing on the integration branch with the
  same errors (`persistence` and `large-store-startup` read the sealed
  `window.powermove` bridge; `memory-history-persistence` expects the compact history
  dictionary on disk, owned by the renderer write path).

## Sessions written without history

Current renderers write `projectState.<id>` without its `history` field; the engine
keeps the stored history (a legacy session may still hold one). It caches every
session it loads and, after load, treats a key it has not seen as having none, so
such a write encodes synchronously. Only an engine that has not loaded (a replacement
after a worker fault) or a session whose file could not be read at load reads the
stored file back first. On the worker, main's synchronous read waits for that as for
any encode. On the inline channel nothing can finish that read while the thread
waits, so the read is refused with an error instead of answered with the previous
value.

## What still runs on main

- Deserializing each IPC message from the renderer. For a checkpoint, the 9.9 MB
  document arrives as a structured-clone object (about 25 to 30 ms to deserialize)
  and is then structured-cloned to the worker (about 20 ms). That is the remaining
  ~50 ms on checkpoint cycles and Saves. Sending the slot as a string (the renderer
  already does this for history through `PM.store.setAsync`) would cut it to a copy;
  that is a renderer change (`legacy/core/projects.ts`, `legacy/host/electron-shim.ts`)
  and must keep the slot write ordered before the journal delete.
- Copying a history string to the worker (6 to 8 ms for 124 MB).
- Rare synchronous reads that hit a key still being encoded. The engine answers in
  order, so such a read (or a new window's bootstrap) also waits behind any earlier
  encode, up to the 5 s ceiling; a 124 MB history encode takes a few hundred
  milliseconds on the worker.
- Whole-store snapshots (`store:snapshot`, boot only) and the `store:get-sync`
  fallback parse the published bytes on main, and for undo histories also unpack
  and re-serialize them. Desktop renderers read through `store:get-encoded-sync`,
  which does neither; the `powermove serve` host uses the fallback, on its own
  thread.

A renderer-to-worker channel that bypasses main entirely would need a
`utilityProcess` host (Electron `MessagePortMain` cannot reach a worker thread) and a
different answer for main's synchronous reads.
