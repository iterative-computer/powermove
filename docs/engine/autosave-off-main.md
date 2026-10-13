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
other agents), 2026-10-12, built from `feat/native-engine` (before) and this branch
(after). A hidden app's timers are coalesced, so the idle floor is 15 to 32 ms.

| Scenario | Main's longest stall before | After | Gaps over 50 ms before | After |
|---|---|---|---|---|
| Autosave, 42 MB history, all cycles (max / median of cycle maxima) | 88.4 / 84.9 ms | 48.9 / 47.6 ms | 15 | 0 |
| Autosave, 42 MB history, journal cycles | 84.9 ms | 31.1 ms | 5 | 0 |
| Native Save, 42 MB history (max / median) | 98.0 / 96.9 ms | 62.5 / 48.7 ms | 11 | 1 |
| Autosave, 124 MB history, all cycles | 270.7 / 245.9 ms | 65.4 / 47.5 ms | 15 | 2 |
| Autosave, 124 MB history, journal cycles | 270.7 ms | 31.4 ms | 5 | 0 |
| Native Save, 124 MB history | 270.9 / 264.1 ms | 50.8 / 49.7 ms | 12 | 1 |

Synchronous handler time on main for `store:set-serialized projectHistory` (124 MB):
226 to 262 ms before, 6 to 8 ms after (copying the string to the worker). For
`store:set project` (9.9 MB object): 54 to 65 ms before, 19 to 22 ms after.

Main's memory after the 124 MB run (`process.memoryUsage`, MiB): heap used 529 before,
142 after; external 516 before, 251 after; RSS 1,342 before, 1,444 after (the
worker's heap is in the same process).

## What still runs on main

- Deserializing each IPC message from the renderer. For a checkpoint, the 9.9 MB
  document arrives as a structured-clone object (about 25 to 30 ms to deserialize)
  and is then structured-cloned to the worker (about 20 ms). That is the remaining
  ~50 ms on checkpoint cycles and Saves. Sending the slot as a string (the renderer
  already does this for history through `PM.store.setAsync`) would cut it to a copy;
  that is a renderer change (`legacy/core/projects.ts`, `legacy/host/electron-shim.ts`)
  and must keep the slot write ordered before the journal delete.
- Copying a history string to the worker (6 to 8 ms for 124 MB).
- Rare synchronous reads that hit a key still being encoded, and whole-store
  snapshots (boot only).

A renderer-to-worker channel that bypasses main entirely would need a
`utilityProcess` host (Electron `MessagePortMain` cannot reach a worker thread) and a
different answer for main's synchronous reads.
