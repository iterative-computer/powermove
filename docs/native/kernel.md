# pm-kernel: the kernel actor (P1 package 5, slices 1–2)

Crate: `apps/native/crates/kernel` (package `pm-kernel`). Pure Rust, no GPUI.
Baseline: plan v6 §3.2 (kernel actor, transactions, history), §6 (library,
journals keyed to base), §8 (behaviour oracle). "Today" means tag `v1.2.0`.
Status 2026-11-01: slices 1 and 2 implemented and tested: actor, transactions,
history, journal, recovery, library reads/writes, locks, state and shell session
operations. Outstanding integration and history work is listed under
[Remaining for P1](#remaining-for-p1).

## Architecture

```
KernelHandle (Clone, Send + Sync)          UI thread
  call() -> Pending<Ack> (Future)  ──┐       published(project) -> Arc<Published>
  call_with(cb) / call_blocking*     │       QueryMirror::sync(&published)  (sync queries)
  open / close / recover / sync      │       subscribe() -> KernelEvent
  register_realm(writer) -> mailbox  │
                                     ▼
pm-kernel thread ── Kernel ── slots: { project id → Slot }
                               Slot = Project (DocumentStore + open entry + acks + workspace)
                                    + Journal (session dir, segment file)
              Durability: pm-kernel-fsync (F_FULLFSYNC loop), pm-kernel-checkpoint (writer)
```

* **One writer.** `KernelActor::spawn` starts the thread that owns the
  `Kernel`; nothing else holds a `DocumentStore`. Calls run one at a time, so
  "taking the writer lease" for a short transaction is simply running on the
  actor; no writer ever waits for a gesture (§3.2.1).
* **No caller blocks.** `call` returns `Pending<T>`, a `Future` (std only, no
  runtime) that can also be polled with `try_take`. `call_with` runs a
  callback on the kernel thread. `call_blocking`/`Pending::wait` are for
  workers and tests and **panic on the main thread** (`pthread_main_np`).
* **Publication boundary.** After a call: the store publishes the revision →
  the journal `write(2)`s the call's durable changes → `Published` is swapped
  (copy-on-write `Arc`: snapshot, recent changes, history and selection
  summaries, open-entry indicator, workspace, dirty) → the caller gets its
  `Ack` → `KernelEvent`s go out. Any revision an event names is already
  readable.
* **Synchronous query mirror.** `QueryMirror` is a `QueryContext` (pm-wasm)
  owned by the UI thread; `sync(&published)` fast-forwards it through the
  published change ops (or adopts the snapshot when they no longer reach
  back). Evaluated values, bounds and picks never cross threads.
* **Realms** (compat page, V8 isolates, Store helpers) are `Writer { realm,
  generation }`. A realm registers a mailbox and receives `RealmMessage`s:
  run an external undo closure, run a cleanup.

### Modules

| Module | What |
|---|---|
| `document`, `commands`, `history`, `queries`, `selection`, `model`, `project_io` | The document layer grown from pm-mac-core: typed command catalog, atomic staging, PMV4 writer (golden tests unchanged), undo records, the query mirror. New: kernel primitives (apply now / record later), external records, a durable-change tap for the journal |
| `tx` | Transactions: the `Call`/`Reply` API, the open entry, leases, folding, selective cancel, sealing, txids |
| `kernel` | `Kernel`: open projects (untitled, `.pmv`, library, recovered), run calls, drive the journal |
| `actor` | The thread, `KernelHandle`, `Pending`, `Published`, `QueryMirror`, events, realm mailboxes |
| `journal` | Sessions, segments, checkpoints, durability threads, recovery |
| `library` | Read-only v1.2.0/v1.3 profile store reader |
| `persistence` | Locked store writer, 150 ms debounce, gzip codec, restartable multi-slot checkpoints, trash |
| `locks` | PID/heartbeat locks, cloud/file-provider/volume/Electron copy policy |
| `state` | UI/transport facts, settings/workspaces, contribution stacks and extension metadata |
| `session` | Shell open/save/save-as/close/duplicate/recents and rollback compaction |

## The API (C-MATRIX write-admission table → `tx::Call`)

| v1–3 API | Call | Reply (today's form) |
|---|---|---|
| `history.begin(label)` | `HistoryCall::Begin` | `Before(Some(snapshot))` outer or nested in a plain entry; `Before(None)` inside a scoped (edit/gesture) entry |
| `history.commit(label?)` | `HistoryCall::Commit` | `Bool`: `false` while nested or unchanged |
| `history.cancel()` | `HistoryCall::Cancel` | `Unit`; keeps edits, drops the entry |
| `history.do(label, fn)` | native body: `HistoryCall::Do`; realm-run callback: `Begin` + writes + `Commit`, or `Rollback` on throw | `Applied`/`Throw`; the realm returns the callback's own value |
| `history.external(label, undo, redo, {cleanup})` | `HistoryCall::External { handle, bytes, cleanup }` | `Entry(Some(id))`; the facade returns `null` for non-function closures |
| `history.undo/redo()` | `HistoryCall::Undo/Redo` | `Bool`; for an external entry `Continuation` (same realm: run inline, then `ExternalDone`) or a deferred reply (another realm's mailbox) |
| `PM.hist.clear()`, `undoIfTop` | `HistoryCall::Clear`, `UndoIfTop` | `Unit`, `Bool` |
| `edit.begin(label)` | `EditCall::Begin` | `Unit`, or `Throw("A source edit is already active")` |
| `edit.dispatch/apply` | `EditCall::Dispatch/Apply` | `EditResult` (`No source edits supplied`, `Project changed: expected revision …`, `Source updated`) |
| `edit.mutate(label, fn)` | `EditCall::Mutate { ops }` (the action's result) | `EditResult`; refuses **before the action** during a live edit (`Finish the active source edit before changing structure`), non-tree changes, no-op = `No source changes` |
| `edit.commit()` | `EditCall::Commit` | `EditResult` (`No source edit is active`) |
| `edit.cancel()` | `EditCall::Cancel` | `Bool` (+ notice) |
| `EditGestureAPI` command mode, native drags | `GestureCall::Begin/Step{replace}/Commit/Cancel` | `Gesture(id)`, `Applied`, `Bool`; set/local modes are history calls/direct writes in the adapter |
| `workspace.mutate(fn)` | `WorkspaceCall::Mutate { next, threw }` | `Workspace(value)`, or `Throw` with partial writes kept |
| MCP / UI / agent edits | `Call::Transact { body, expected_revision }` | `Applied`, or `Throw(Stale)` |
| `PM.proj` raw writes, touches | `Call::DirectWrite { ops }` (write-barrier flush) | `Applied`, or `Throw(DirectWrite { resync })` |
| lost ack | `Call::QueryTx { txid }` | `Ack(Some(stored))` |
| realm crash/hang/dispose | `Call::RealmFailed` | `Unit` (+ `Sealed` notice) |

Every `Ack` carries `revision`, `folded: Option<entry>`, `notice`, and the
`TxId (realm, generation, txid)` when the request had one.

## Transactions (§3.2.1)

* **Open entry and holders.** One per project. Each `begin`/lease pushes a
  holder (`Writer`); `commit` releases the caller's newest holder (or the
  newest, since today's counter had no owner). The entry closes when no
  holder remains. Depth = holder count, so the trace's `depth` matches.
* **Folding.** Any write while the entry is open applies at once (its own
  revision, visible to everyone) and is recorded in the entry with its
  writer, forward ops, inverses, capture scopes and **after-image probes**.
  Closing the entry records one step: forward = concatenation, inverse =
  reverse concatenation, plus the leaf-patch file form when any op is
  structural; an entry whose touched scopes are unchanged records nothing.
* **Leases.** `edit.begin` and gestures open a lease (owner, start index,
  starting selection). `edit.cancel`, a gesture cancel and `Rollback` revert
  the owner's writes after `start`, newest first, **only if every probe still
  holds** on the staged tree (`Path` value, `Element` by id, `Order` of ids;
  content equality ignoring node revisions), and only if no **index hazard**
  exists (one side restructured a list — insert/delete/reorder/splice — that
  the other side, writing later, addressed by index). Otherwise nothing is
  reverted: **commit with notice** keeps the whole entry as one undoable step.
  On success, folded foreign writes become **their own entry** (`ForeignSplit`)
  and the starting selection is restored.
* **Gesture steps.** `replace: true` supersedes the previous step (today's
  from-start drags) only while it is still the newest write and its probes
  hold; otherwise the step accumulates, so a foreign write in between is
  never clobbered.
* **Owner failure.** `RealmFailed` seals: the failed generation's leases and
  holders are released; if it owned the entry, the whole entry (owner and
  foreign writes; all were acknowledged when they returned) is recorded
  (`Sealed`). The generation is fenced: later calls get `StaleGeneration`
  (never acknowledged, never applied). Its external entries become barriers;
  continuations waiting on it resolve as failures.
* **Txids.** Acks with a txid are stored (last 4,096 per project); a retry
  returns the stored ack without running again; `QueryTx` answers lost acks.

## History (§3.2.2)

* One stack per project: document records (ops + inverses) and **external
  records** `(realm, generation, handle, cleanup)`, 256-byte minimum.
* Undo of an external entry moves the cursor first (as `idx--` did), then the
  owning realm runs the closure: inline continuation if that realm is the
  caller, otherwise a mailbox message and a deferred reply. A throwing
  closure leaves the cursor moved and throws, as today. A dead realm's entry
  is a barrier: the cursor stays, `Barrier` notice.
* Retention as today: 1,000 entries and a 256 MiB byte budget (distant redo
  first); entries with a cleanup that retire (trim, discarded redo, clear)
  send `Cleanup` to their realm.
* **Persisted history.** `History::import` reads `projectHistory.<id>` /
  `.pmv` history: `{version:1, paths, index, entries:[{label, forward,
  backward}]}` (root-relative histories canonicalized) and the packed
  `powermove-history-dictionary`; refusals carry a generic reason. Export:
  `to_saved()` (document entries only, index counting them) and
  `to_saved_packed()` (`packHistory`). Round trips are byte-identical on the
  fixtures; undo after migration is tested.

## Journal and durability

* Layout `<root>/<project>/<session>/journal-<n>.log` + `checkpoint-<n>.json`.
  Each line is `<16-hex sha256> <canonical json>`: `rev` (base, revision,
  ops), `push`, `squash`, `undo`/`redo` (external flagged), `clear`, `ws`,
  `ext`. Segment headers carry the session's **base hash** (canonical
  SHA-256 of the document as opened).
* **Write before ack, fsync in the background**: no acknowledged write is
  lost to a process crash; power loss is bounded by the flush interval
  (200 ms). A failed journal write is never silent (`NotDurable` notice).
* Checkpoints every 50 revisions or 4 MiB (today's constants), written by a
  background thread (temp, `F_FULLFSYNC`, rename, dir sync); older segments
  are pruned only after the newer checkpoint is durable.
* **Recovery** (`journal::recover` / `KernelHandle::recover`) loads the
  newest valid checkpoint, replays segments, stops at a torn tail
  (`truncated`), and returns `Recovered`, or **`Copy` when the current base
  differs** (never replayed onto a changed project), or `Unrecoverable`
  (files kept). Without a checkpoint, only the true base document (hash
  checked) is accepted. Adopting recovered work starts a new session that
  supersedes the old one once its first checkpoint is durable.
* Low-level document closes keep their journals. Shell session close first
  saves/compacts; failure keeps the session open. Old sessions retire only
  after the new base checkpoint is durable.

## Library store read path

`LibraryStore` reads `<profile>/store/<key>.json` read-only: 42 static keys,
10 dynamic families, quarantined `*.json.corrupt-*` and unregistered keys
listed (never dropped). `load(id)` ports `getAsync` + `getStateAsync`
(envelope/bare/stringified slots, `autosave` fallback, legacy journal replay
with `projects.ts` `applyPatches` semantics and the revision skip, history
from `projectHistory` or `projectState.history`, `projectMeta` over
`projectState`, `projectWorkspace`). `OpenSource::Library` opens it in the
kernel.

**Finding (D-STATE gap).** Since v1.3.0, `main/storage-codec.ts` stores values
≥ 32 KiB as `{__powermoveDiskValue:1, encoding:"gzip", bytes, data}` under
the same file name. The reader decodes it with the same validation and
per-key limits. Without it, 8 of 45 histories in a real profile would not
open.

**Smoke test** (an APFS clone of a real store, deleted afterwards; counts
only): 466 files (29 static keys; 47 `project`, 51 `projectHistory`, 10
`projectJournal`, 95 `projectMeta`, 11 `projectState`, 32 `projectWorkspace`,
58 `agentThreads`, 122 `agentAttachment`, 3 `agentModel`, 8 `ext`), index 45
rows + trash 2, 0 orphan or missing slots, 0 quarantined. All **47 project
slots load and open in the kernel; 45 histories import, 0 rejected; 10
legacy journal entries applied; 0 failures.** Run it with
`PM_KERNEL_STORE_COPY=<copy> cargo test -p pm-kernel --test library -- --ignored --nocapture`.

## Behaviour oracle: golden trace replay

`tests/trace_replay.rs` replays `tools/trace-recorder/golden` against the API
layer: root `api.*` calls, the `api.*` calls inside `gesture.*` adapters,
callback writes recovered from the recorded states, and raw writes between
calls as direct writes. It compares return forms and post-state (depth, undo
index, entry count, project name, layer count).

| Trace | Result |
|---|---|
| history-nesting | 9/9 |
| history-cancel | 2/2 |
| edit-cancel | 6/6 |
| edit-admission | 11/12 + 1 approved difference |
| external-history | 10/10 (cleanup count included) |
| gesture-modes | 15/15 (set, command, local modes) |
| workspace-direct | 3/3 (`PM.touch` n/a) |
| history-async | 2/2 |
| keymap-fields, keymap-order, menu-equivalents | skipped: the shell's resolver (§3.2.3) |

**58/59 match; 0 unapproved mismatches.** Approved difference: `edit.apply`'s
stale message names the kernel's revision counter (starts at 1) where the tag
named `PM.proj.revision` (0); same refusal and form. The goldens are
single-writer, so the deliberate §3.2.1 improvements (selective cancel,
foreign split, sealing) are covered by the S5 scenario tests instead.

## Decisions (and the rule that picked each)

| # | Decision | Rule |
|---|---|---|
| K1 | pm-mac-core's code moves into pm-kernel; `pm-mac-core` stays as a one-line re-export until nothing names it | N5 prune as you go, without breaking lanes that started on the old name (no code consumer today) |
| K2 | The actor is one OS thread with an mpsc mailbox and a std-only oneshot future; no async runtime | §3.2 "kernel waits are asynchronous in every host"; N8 no overhead |
| K3 | A short transaction is one actor turn; folding, never queueing; D10 not implemented | §3.2.1 "no writer ever waits"; D10 decided "no" |
| K4 | Depth is a holder list, not a counter | Same returns as today (traces match) while letting a failed realm release its nesting (§3.2.1 sealing) |
| K5 | Selective cancel by per-operation after-image probes plus the index-hazard rule; any conflict → commit with notice of the whole entry | S5's rule: the simplest implementation satisfying "a cancel never removes an acknowledged write", favouring keeping work |
| K6 | `history.do`'s throw path reverts only the thrower's writes; a failed `edit.apply` batch is atomic (applies nothing) | The same invariant beats the tag's whole-pending rollback (C-MATRIX "explicit improvement") |
| K7 | Undo/redo inside an open entry fold as navigation: included at commit (the tag's snapshot diff), neither reverted nor split by a cancel | Today's commit semantics; the cursor already accounts for them |
| K8 | Direct writes outside an entry record no undo step; inside, they fold | Trace `workspace-direct` (raw write creates no history); the tag's pending diff picks them up |
| K9 | External closures stay in their realm: inline continuation for the blocked caller, mailbox + deferred reply otherwise, barrier when the realm is dead | §3.2.2; no synchronous call into another isolate thread (§3.4) |
| K10 | External entries are not persisted (journal and checkpoints skip them) | The tag's `export()` excluded closure entries; after a crash the realms are gone |
| K11 | Journal: write before ack, `F_FULLFSYNC` every 200 ms on a background thread | N1/§0 never lose work + "≤ 1 s"; the actor never waits on the disk |
| K12 | Checkpoint every 50 revisions or 4 MiB | Today's `CHECKPOINT_REVISIONS`/`MAX_JOURNAL_BYTES` |
| K13 | Base hash = canonical SHA-256 (`pm_model::hash::canonical_hash`), mismatch → `Copy` | §6 "journals carry their base revision hash … offers a copy"; same identity pm-node's checkpoints use |
| K14 | Sessions are superseded only after the new session's checkpoint is durable; recovery never deletes; closing keeps the journal | N4 / D-STATE "never overwrite, never delete" |
| K15 | The reader decodes the v1.3.0 gzip envelope | N4: real profiles contain it |
| K16 | An unimportable history opens with empty undo and a reason | D-STATE: "never silently open with incomplete undo" |
| K17 | Packing interns records by canonical content | Deterministic; equals the tag's output (verified on fixtures) |

## Tests

Slice 1 contributes 140 passing + 1 ignored smoke test (`cargo test -p pm-kernel`): 98 carried
over from pm-mac-core (commands, document store, project I/O, the TS writer
golden parity) and 42 new: 23 transactions (the S5 stub's eight scenarios on
the real document, history nesting, edit return forms, selective cancel and
hazards, gestures, external entries, workspace, stale revisions), 4 actor, 6
journal (including the **kill test**: a child process writes as fast as it
can and is SIGKILLed mid-stream; recovery holds every acknowledged write,
sometimes one more, and undo works), 8 library, 1 trace replay.

## Deliberate differences from v1.2.0 (beyond the approved one above)

* Cancels never remove acknowledged foreign writes (K5, K6).
* `edit.commit` with no changes lowers the depth by one; the tag's
  `PM.hist.cancel()` there also dropped an enclosing pending entry.
* Revisions are the kernel's monotonic counter (undo is a new revision).
* `history.selection` entries are not ported yet (selection is not history).

## Engine needs (not changed here)

* Content equality for `pm_model::Node` ignoring revisions (the kernel has
  `same_node`; the engine is the natural home).
* The base hash is computed on the actor at open; for very large projects
  use `canonical_digest_parallel` or the file/slot hash computed while
  reading, off the actor thread.

## Remaining for P1

* History: parked histories sharing one global budget (`history.ts:460–480`),
  `history.selection`, `mark`/`squash` as calls (agent runs), export pinning
  of extension generations, v4 hot-reload staging (§3.2.1).
* Index hazards: rebase index paths by id so more cancels revert instead of
  committing with notice.
* Realm adapters: the compat page's write barrier (dirty-path diffing, resync
  on refusal) and the JS facades that map onto `Call`; real transport for
  txids and lost acks (S5 channel).
* Wire `KernelHandle::shell` and state events into the shell, including copy
  offers, copy/fallback notices, per-window document binding and the explicit
  `CompactAll` barrier before rollback. No shell files are changed in this slice.
* The P7 migration/restore coordinator still owns profile selection, TR version
  verification, media-store/IndexedDB/Keychain transfer and reverse migration.
  The kernel accepts explicit paths; it does not find or migrate real profiles.
* Runtime/compat adapters own callback handles, permissions and UI editions;
  contribution definitions here are JSON facts, not callable runtime objects.
  Workspace facade adapters supply the callback's resolved `next` object
  (including the tag's built-in clone/normalization before/after that callback).
* Async base hashing for very large documents; measured edit → publish
  latency under load.


## Slice 2: library writes, state and shell sessions

### Done

`KernelHandle::shell(ShellCall)` returns the existing asynchronous `Pending`
future. Attach an explicit **native** profile using `AttachLibrary`, then open
`SessionSource::New`, `Library { profile, project }`, or `File`. `Opened`
contains a generation-bound `SessionHandle`, `OpenInfo`, `OpenDecision`, and
`RecoveryResult`. Save, SaveAs and Close require that handle; an old handle
cannot affect a later reopen. An already-owned source cannot open twice.

* The store writer keeps the exact registered keys and `<key>.json` filenames.
  Writes debounce per key for 150 ms, exclusive-create a sibling `.tmp-*`,
  write, fsync, close, rename, and sync the directory. Failed writes remain
  queued for retry; explicit barriers report errors. Combined `projectState`
  partial writes preserve history. Unknown slots/fields and quarantine stay.
* A project save writes `{v, proj}`, `projectHistory`, `projectWorkspace`,
  `projectMeta`, `projects` rows (`id,name,at,w,h,thumb?`) and `projectTrash`.
  It clears legacy patches only after durable replacement. A validated,
  fsynced `library.pending` intent makes interrupted multi-slot saves/trash/
  restore roll forward on writer reopen. Slots remain ordinary Electron data.
* `DiskFormat::V1_2` (default) writes plain JSON at every size. Explicit
  `V1_3` applies the shipped 32 KiB threshold **and** 90% size test, writing
  `{__powermoveDiskValue:1,encoding:"gzip",bytes,data}`. Both read paths work.
* `store/library.lock` and `Movie.pmvlock` (for `Movie.pmv`) carry PID and
  millisecond heartbeat, refreshed every five seconds. Local `flock` gives
  exclusion during acquisition/reclaim. A dead PID **and** age ≥30 s are both
  required; malformed locks are refused. Stable inodes avoid unlink/recreate
  races; orderly release writes PID 0 before releasing the descriptor.
* Cloud path components (including symlink targets), ancestor
  `com.apple.file-provider*` xattrs, volume read-only flags/permission bits,
  and the source profile's Chromium `SingletonLock` produce copy decisions.
  Unknown/remote Singleton owners are conservatively treated as running.
  Supplying the Electron profile as the native destination is refused.
* `StateTransaction` stages an entire batch and checks its expected revision
  before publishing anything. Settings publish per-key events, persist under
  their original keys, and keep a native roll-forward state record outside
  those slots (including deletion and extension metadata). Named workspaces
  retain the tag's array/object shapes and current/default/trash keys;
  successful project workspace snapshots upsert the named registry.
* Per-project `UiState` owns tool/toolShape, timeline graph/pps/scroll/key
  selection facts, viewer pan/zoom/fit/shown/controls/path/text/source-preview
  facts, and transport time/playing. Opaque session members survive alongside
  these facts. Timeline/time/view session data imports and saves in legacy
  metadata. UI facts and contribution stacks are ephemeral until session save;
  they never enter document undo or cause a disk fsync per pointer update.
* Contribution stacks are ordered, owner/generation scoped, support top and
  buried disposal and reveal the previous provider. Extension metadata records
  installed/enabled/version/source, while legacy `extensions` stays a Boolean
  enablement map. Enablement alone does not prove installation; inventory
  supplies installation/version/source through the transaction API. No runtime.
* Workspace mutations keep partial writes on callback throw, publish a
  workspace event, and never dirty or add undo to the document. Their journal
  records persist even without a document edit. `Published` includes workspace
  and UI facts; the global immutable state is available through `handle.state()`.
* Explicit save/close rebases the journal only after saving. Copy journals use
  the copy's durable slot identity and saved base hash. A mismatched base
  materializes a named recovery slot and returns `OfferCopy`; current source
  data remains separate. File-save failure falls back to a recovery slot and
  reports its id. Embedded media in copies/duplicates is saved into independently
  verified PMV4 companions under the native profile's `recovery-files`, with
  the legacy `file.path/savedHash/baselineHash` association in metadata.
* File sessions also have library snapshots/recents. Duplicate uses live edits
  when open; trash retains slots/history; restore restores index membership.
  Reopening an associated external file from recents reacquires its lock and
  retains in-place saves; a changed saved hash forces a copy. State snapshot
  maps share `Arc` ownership and copy only the domain a transaction mutates.
  `CompactAll` saves active sessions and reopens unopened journal owners to
  compact into their slot or file, preserving trash membership and metadata.
  Missing/offline originals become copies;
  unrecoverable journals stay on disk and return an error. No journal is
  deleted before a replacement checkpoint is durable.

### Decisions and the rules that picked them

| # | Decision | Rule/evidence |
|---|---|---|
| K18 | Two explicit disk formats; default plain JSON for v1.2 rollback | N4/§6: v1.2 `acceptSerialized` does only JSON.parse; it cannot decode v1.3 gzip. Claiming one compressed representation works with both would lose work. |
| K19 | Restartable intent around multi-slot commits, retain old journals until replacement durability | N4: single-file atomic rename cannot make document/history/index changes atomic together. |
| K20 | Stable lock inode plus flock, PID/heartbeat and conservative copy decisions | §6's 30 s **and** dead PID rule; exclusion must survive two processes reclaiming at once. |
| K21 | Explicit native destination; foreign libraries always named copies | §6 beta never writes the Electron library. Profile migration/role selection belongs to P7. |
| K22 | Workspace stays outside document undo, including partial callback failure | N6; v1.2 `workspace.ts:560–589` and C-MATRIX admission row. Resolved callback state is passed to the kernel. |
| K23 | Registries hold JSON definitions/realm handles, not closures, canvases or DOM | §3.2 authority; C-MATRIX says those live-object/callback semantics remain in the compatibility edition until a full facade exists. |
| K24 | Keep extension enablement format; store inventory metadata in native state sidecar | N4 rollback keys must stay readable; the tag's `extensions` key is a Boolean map, not an inventory database. |
| K25 | Generation-bound sessions, independent media copies, recovery fallback, explicit compaction barrier | N4/§6 rollback must include current/untitled work and never overwrite changed/unwritable sources. |

### Verification

**180 passing pm-kernel tests + 1 ignored profile smoke test** (140 existing,
40 slice-2 tests), plus the `pm-mac-core` re-export crate check. All test profiles
are temporary. Formatting and `git diff --check` pass. Clippy is not installed
in the pinned toolchain; no toolchain changes were made.

`tests/tag_store_parity.mjs`, driven by the Rust suite with synthetic inputs,
reads **actual tagged source via git show**. It compares all 42 static and 10
dynamic keys; executes v1.2's `persist` body with instrumented filesystem calls
(write/open/fsync/close/rename); runs v1.2 `getAsync`, `getStateAsync`, history
unpacking and composition-path realization; and undoes a native edit back to
its original value. It also executes v1.3 `decodeStorageJSON` against native
compressed bytes. Rust tests independently reload those values and saved undo.
Existing byte-for-byte PMV writer golden tests remain green.

Additional tests cover two real processes contending for **both** lock types,
live heartbeat/inode stability, dead/live/recent lock rules, file-provider xattrs,
symlinked cloud paths, a sealed read-only volume, older Electron's SingletonLock,
failed-write retry, interrupted/invalid intents, copy crashes, independent embedded
media after deleting the original, settings deletion/restart, registry overrides,
publication/events, save-as, stale sessions, recents/trash/restore/duplicates,
base mismatch, rebase, and unopened library/file/offline rollback compaction.
