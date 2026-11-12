# pm-kernel: the kernel actor (P1 package 5, slices 1–3)

Crate: `apps/native/crates/kernel` (package `pm-kernel`). Pure Rust, no GPUI.
Baseline: plan v6 §3.2 (kernel actor, transactions, history), §6 (library,
journals keyed to base), §8 (behaviour oracle). "Today" means tag `v1.2.0`.
Status 2026-11-01: slices 1–3 implemented: actor, transactions,
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
| `history.selection(before, after)` | `HistoryCall::Selection` | `Entry(Some(id))`, or `Entry(None)` for equality, pending history or redo navigation |
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
  content equality ignoring node revisions). Index paths in probes and
  inverses rebase through unique element IDs (including nested arrays and
  numeric string keys); inverse insert positions anchor to surviving neighbours.
  Changed unkeyed arrays, missing/ambiguous IDs and conflicting probes keep
  the conservative fallback. Otherwise nothing is reverted: **commit with notice** keeps the whole entry as one undoable step.
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
  (200 ms gate budget; the worker now schedules at most 100 ms, with disk work
  charged to the cycle). A failed journal write is never silent (`NotDurable` notice).
* Checkpoints every 50 revisions or 4 MiB (today's constants), written by a
  background thread (temp, `F_FULLFSYNC`, rename, dir sync); older segments
  are pruned only after the newer checkpoint is durable.
* A line holds the edit's operations, id-keyed where possible, so it costs what
  changed. A whole-value history form is kept only below 1 MiB; larger
  structural edits save as operations, which shipped readers refuse as history
  (the document still opens). A document of 4 MiB or more checkpoints by segment
  size alone (`VERIFY_MIN_CHARS`), and Undo skips an id-keyed operation a write
  outside history made stale.
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
| K1 | pm-mac-core's code moved into pm-kernel; the re-export crate is deleted | N5 prune as you go |
| K2 | The actor is one OS thread with an mpsc mailbox and a std-only oneshot future; no async runtime | §3.2 "kernel waits are asynchronous in every host"; N8 no overhead |
| K3 | A short transaction is one actor turn; folding, never queueing; D10 not implemented | §3.2.1 "no writer ever waits"; D10 decided "no" |
| K4 | Depth is a holder list, not a counter | Same returns as today (traces match) while letting a failed realm release its nesting (§3.2.1 sealing) |
| K5 | Selective cancel by per-operation after-image probes and ID-rebased paths; any conflict → commit with notice of the whole entry | S5's rule: the simplest implementation satisfying "a cancel never removes an acknowledged write", favouring keeping work |
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
* The 1,000-entry count cap is shared across open projects, as requested;
  v1.2 shared the byte budget but capped entry count per stack. Each project
  still retains its latest entry even if the aggregate exceeds a limit.
* Selection restoration also prunes missing key IDs using the existing native
  selection invariant; the tag's selection closure filtered only layer IDs.
* V4 activation stages edits until success and retries on a changed document
  or extension epoch, instead of v1–3's immediate live activation writes.
  V1–3 write admission is unchanged.

## Engine needs (not changed here)

* Content equality for `pm_model::Node` ignoring revisions (the kernel has
  `same_node`; the engine is the natural home).
* The base hash is computed on the actor at open; for very large projects
  use `canonical_digest_parallel` or the file/slot hash computed while
  reading, off the actor thread.

## Cloud history retention and selection

The kernel shares the 1,000-entry / 256 MiB budget across all open projects.
`set_history_budget` applies memory-pressure limits. Oldest recently active
background histories trim before the active history; distant redo goes before
old undo, and each project retains at least its latest entry, even over budget.
Retired external entries still queue cleanup to their owning realm, and the
actor republishes changed background histories. This extends the tag's shared
byte rule to the requested shared entry limit; the tag's count limit was per stack.

`HistoryCall::Selection { before, after }` is the explicit v1.2 navigation
record (ordinary selection calls remain unrecorded). Equal selections, pending
entries and redo navigation return `Entry(None)`. Undo/redo restores/prunes IDs,
without changing the document revision, persisted history or dirty state.

## Export generations and v4 activation

`Call::Extension` runs through normal actor fencing, deduplication and journal
publication. `PinExport` returns an `ExportJob`: one immutable document snapshot
and one immutable map of extension generations. Every frame/worker uses that
same job; reloads copy the map rather than updating a job's bindings. Generation
objects include realm/generation and immutable resource definitions. Runtime
adapters must keep the corresponding realm/resources alive while a job pins
that object, then dispose after the final pin; the kernel never executes JS.
A failed realm makes new exports refuse; existing jobs never silently rebind.

For v4 hot reload, `BeginActivation` allocates a writer-bound staging ticket.
The host runs/awaits activate outside the actor and sends document ops through
`Stage`; these calls append only, opening no entry or lease and publishing no
edits. `Finish(success: false)` discards only unpublished staged operations.
On success the kernel checks the document revision and extension publication
epoch, validates the complete batch, commits one history step, then swaps the
generation in the same actor turn. Invalid or stale batches leave the document
and old generation intact. A live entry refuses success (the ticket remains
available to abort/retry); failed realms drop their unpublished staging tickets.
No-op activations can swap generations without a document history entry.

This v4 policy deliberately differs from v1–3's live activation writes: staged
edits are invisible until success; concurrent document edits or another reload
require activate to retry, and activation never folds into a live gesture.
The runtime adapters still own JS execution, routing v4 writes to `Stage`,
resource disposal, and applying the pinned definitions to the render pipeline.

## ID-rebased selective cancellation

Each folded operation retains its immutable after-image root. Cancellation maps
its probes and inverse paths from that root onto the current staged tree by
unique IDs, recursively. Inverse batches update their source image as they run.
A deletion's inverse insertion anchors beside a surviving neighbour rather than
reusing its obsolete index. All probes and final validation must succeed before
any revert publishes; otherwise the complete acknowledged entry stays undoable
with notice. Unkeyed changed lists and missing/duplicate identities refuse.

After a successful cancel, retained history is regenerated with the engine's
ID-aware node diff against the entry's original document. This preserves foreign
writes whose original index paths depended on the cancelled reorder; undo/redo
and persisted patches therefore describe the retained tree, not stale indices.
This refines the already approved native selective-cancel difference from v1.2;
no additional tag cancellation behavior is copied.

### Cloud verification

`cargo test --locked -p pm-kernel -p pm-mcp -p pm-first-launch` passes:
**217 pm-kernel tests + 1 existing ignored profile smoke test, 25 pm-mcp,
16 pm-first-launch** (258 passing total). Ten new tests cover the shared
count/byte budgets, parked publication/cleanup, clean-state selection history,
activation success/failure/staleness/ownership/epoch races, immutable export
bindings, nested numeric-string indices, inverse insertion anchors, retained
foreign history and ambiguous-list fallback. The former index-hazard test now
asserts the intended successful selective revert plus undo/redo preservation.

The tagged writer parity test verifies/fetches v1.2.0 and v1.3.0, probes Node
or Bun (with `PM_TEST_JS_RUNTIME` override), and fails on missing runtimes.
An unavailable tag after a failed fetch emits an uncaptured, visible `SKIP`
message; a fetch that claims success but fails verification is an error.
The actual tagged scripts ran and passed here; no tag check was skipped.

Pinned-toolchain Clippy (`--locked -p pm-kernel --all-targets --no-deps`)
adds **zero warnings**, compared with a separate checkout of the starting
`origin/feat/native-engine` commit `5412ab207`: the same 15 library warnings
and one existing test warning remain. Formatting and `git diff --check` pass.

## Remaining for P1

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
* A project file on a volume only this Mac writes (a folder a sync client
  keeps is one) is locked in the profile: `locks/<key>.lock`, an `flock` and
  nothing else, keyed by the project's place (its folder's device and inode
  and its file name; a save replaces the file's own inode). Nothing is
  written beside the project, nothing is refreshed while it is held, and the
  file is removed on release (every taker checks that the name still leads
  to the file it locked). It excludes the instances that share the profile.
  Two Macs editing one synced file are guarded by the save's stale-write
  check only: a sync client carries files, not locks.
* `store/library.lock`, and `Movie.pmvlock` beside `Movie.pmv` on a volume
  other machines write, carry PID and millisecond heartbeat, refreshed every
  five seconds. Local `flock` gives exclusion during acquisition/reclaim. A
  dead PID **and** age ≥30 s are both required. An empty, torn or malformed
  record is stale on a volume only this Mac writes (the held `flock` proves
  no owner lives) and is rewritten; on a shared volume it is refused. Stable
  inodes avoid unlink/recreate races, so the file stays after close (hidden
  in Finder); orderly release writes PID 0 before releasing the descriptor.
  A lock file whose name no longer leads to the locked inode is refused at
  acquisition and no longer refreshed.
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

## Slice 3: integration, MCP and first-launch services

The kernel remains the only document, history, session and profile writer.
The new entry points are asynchronous on `KernelHandle`; synchronous `Kernel`
methods are for dedicated workers and tests.

### Recovery decisions and close

* `ShellCall::PrepareOpen` reads the source and probes recovery. If work is
  recoverable it returns `RecoveryRequired { ticket, recovery }` **before**
  opening/publishing a document or saving a slot. Cancelling the sheet leaves
  all data alone. A later preparation for the same ID invalidates the old ticket.
* A clean rebase checkpoint does not prompt: document, saved history and
  workspace must all match the base, and the journal must not be truncated.
  History-only and workspace-only changes still require a decision.
* `DecideRecovery { ticket, decision }` accepts `Recover`, `OpenAsCopy`, or
  `Discard`. Tickets are single-use and bound to the observed source base,
  journal session/revision/content and external file version. A changed base
  refuses the choice and requires a fresh preparation. Recover on a mismatched
  base opens the work as a named copy. A failed replay is never called recovery.
* Discard archives only the rejected journal session under `discarded/`, syncs
  both directories, and opens the unchanged base. It does not save recovered
  content or call the saving Close operation. Archives are retained for future
  user-managed cleanup. The existing `Open` entry point keeps its safe automatic
  recover/copy behavior for worker clients; the app uses explicit preparation.
* `DeferClose` flushes and releases a session's project lock while preserving its
  journal, without saving. `Close` flushes, saves/compacts, waits for durability,
  then releases the lock. Both refuse an active entry or an in-flight save.
  Failures keep the document/session available. Low-level close also refuses
  a live store save and a failed journal flush.
* `PrepareQuit` checks all sessions before compaction, compacts active and
  parked journals through existing shell operations, and waits on the library,
  journal and checkpoint durability barrier. It reports failure instead of
  certifying a completed shutdown. Application termination UI remains the shell
  lane's responsibility.

### DiscardClose

`ShellCall::DiscardClose(session)` is the standard **Don't Save** operation.
It validates the session generation, refuses an active edit or in-flight save,
waits for checkpoints and fsyncs, then atomically moves the rejected journal to
`<journal-root>/<project>/discarded/<session>`. Both directories are synced
before it releases the project lock and returns `Closed { recovery_copy: None }`.
It never saves the file or library slot; their bytes stay unchanged. A journal
must be configured so rejected work can be preserved. Archives are excluded
from recovery offers and quit compaction, and can be restored explicitly by
moving the archived session back into its project journal directory while closed.

`DeferClose` keeps work eligible for recovery and quit compaction; use
`DiscardClose` when the user rejects edits. Close/recovery transitions wait for
checkpoint creation, rename and pruning before using or moving a journal path.
Concurrent fsync callers also wait for the current flush, including rotated
segments. `ShellCall::DurabilityBarrier` returns `ShellReply::Durable` only when
all earlier acknowledged writes and queued checkpoints are fsynced; IO errors
are returned to the caller. It performs no compaction or project save.

### Selection, actor identity and history

* `SelectionCall::{Layers, ToggleLayer, Keys, Channel, Clear}` runs in an actor
  turn, uses ordinary request generation fencing/txid deduplication, and returns
  the resulting selection. Invalid layer IDs refuse the whole selection change.
  Selection stays ephemeral and outside dirty state; explicit history.selection records navigation as described above.
  `SelectionChanged` follows publication, including pruning by document edits.
* `KernelHandle::for_actor(Caller)` binds realm, generation and origin in an
  immutable `ActorHandle`. UI, MCP clients and agents never change a global
  "current actor". Their ordinary edits retain the existing fold/cancel rules.
* `HistoryCall::Mark` returns an opaque `ActorHistoryMark` bound to the document
  lifetime and writer. `Squash` checks that identity, the expected revision,
  absence of an open entry and absence of foreign mutations after the mark.
  Tracking is bounded to 4,096 mutations; older marks conservatively refuse.
  Mark queries and selection do not invalidate another actor's grouping.
* `Call::Restore` requires document identity and expected revision, validates
  the complete replacement and refuses an open entry. A successful restore is
  an ordinary undoable, journalled change with the caller's origin. Mark/squash
  and restore use the same journal/publication boundary as Transact; there is
  no caller-owned journal drain.

### Save handoff and shell metadata

`prepare_save(session, optional_save_as_path)` freezes a PMV save and acquires
its destination lock on the actor. `SessionSave::run` runs on a worker, then
`finish_save(completion)` adopts the receipt on the actor and coordinates the
library association and journal. The session/ticket pair prevents stale adoption.
Edits continue during the worker save and remain dirty if newer than its frozen
revision. A failed completion clears the flight without claiming a successful
save. Dropped handoffs require explicit `AbortSave` after the plan is dropped;
running plans cannot be aborted out from under a disk writer.

Library saves continue to use `ShellCall::Save`. Concurrent file-save edits keep
an older journal base until the next full save; recovery offers a copy when the
file and journal base differ. This preserves acknowledged work conservatively.

`Published` includes session handle/destination/metadata, path, saving state and
copy-on-write `MediaTable` read capabilities. The app streams those capabilities to
its private media cache instead of reopening the PMV and rediscovering metadata.
`MediaEntry::copy_to` uses exclusive destination creation, bounded streaming,
length/digest checks and external-file version checks, removing failed outputs.

### First-launch profile services

`KernelHandle::profile(ProfileCall)` uses the attached native library lock.
`ProfileStorage` borrows that same lock owner for the migration worker.

* Logical localStorage reads/writes address exact origin/key pairs and preserve
  exact DOM strings, empty strings and unrelated/unknown entries. Writes and
  deletes publish atomically to the imported sidecar. Initial import preserves
  existing native values and never activates legacy evaluator flags.
* Secret registration stores versioned `SecretLocator { id, version, service,
  account }` records, refuses differing locators for the same version, and
  returns all retained versions. First-launch registers the locator after an
  explicitly consented Keychain write and before its migration receipt. No
  secret value enters the kernel, document, journal, or locator sidecar.
  Consumer-specific Keychain resolution/updates remain host service work.
* Extension source inventory identifies read-only shared and writable overlay
  roots. Reads prefer overlay files, falling back to shared files. Writes
  create/replace only overlay files. Absolute/traversing paths and linked roots
  or ancestors are refused; the kernel never changes shared permissions.
* `ExternalMedia` reads a PMV's media capabilities without creating a source-side
  lock. Migration retains external associations as `embeddedMedia`, separate
  from in-place save destinations. Opening such a library copy checks its
  source hash when present and saves independent media into the native profile's
  existing PMV companions. A later save never targets the external source.

### Caller changes

* `app/src/session.rs`: kernel selection, pre-adoption recovery sheet, published
  destination/media, worker save handoff; removes local selection and recovery
  Discard workaround. `app/src/media.rs` and `smoke.rs`: stream kernel media capabilities.
* `mcp/src/automation.rs` and `session.rs`: actor-bound history marks, guarded
  squash/restore and selection messages; removes direct store mutation and empty
  transaction journal drains. `ActorAutomation` exposes the same operations and
  worker save handoff. Full transport/session hosting in the app remains an MCP
  integration task; standalone transport still owns its one dedicated kernel.
  MCP library opens use PrepareOpen and leave recovery decisions to the host.
* `first-launch/src/lib.rs`: kernel localStorage import, durable secret locators,
  kernel extension-source inventory and read-only external media associations on library copies.

### Decisions and selecting rules

| # | Decision | Rule |
|---|---|---|
| K26 | Pre-adoption, single-use recovery choices; archive explicit Discard | N4: no silent discard, no changed-base replay, no save of rejected recovery |
| K27 | Immutable per-caller actor handles and opaque history marks | §3.2 single writer; §3.2.1 foreign acknowledged edits retain their provenance/history |
| K28 | Refuse grouping/restores across foreign mutations, stale identities or open entries | N4: an agent run cannot remove or relabel another writer's work |
| K29 | Worker PMV save plans, actor completion and explicit close/durability barriers | §3.2 asynchronous hosts; N4 crash-safe work and lock ownership |
| K30 | Native-profile sidecars under the existing library lock; locators only | §6 beta isolation/secret handoff; N8 no second runtime or profile writer |
| K31 | Overlay-only extension writes and read capabilities for external PMV media | §6 shared roots read-only, named copies and rollback-safe media; N4 |

### Verification

The slice-3 regression suite uses synthetic documents and disposable temporary
profiles only. It covers publication/selection/deduplication, actor-bound guarded
history and journal replay, all recovery choices and stale tickets, saving with
concurrent edits, failure/abort/close/quit, exact localStorage, secret locator
versions, overlay traversal/symlink refusal, external media integrity and copy
independence. No real profile, project or Keychain item is used.

Verified with pinned Rust 1.96.1: **200 pm-kernel tests pass** (180 existing +
20 slice-3 tests; 1 optional profile smoke ignored), **25 MCP tests**, **16
first-launch tests**, and **1 app session workflow test** pass. The app and
pm-mac-core re-export check pass. Strict Clippy for MCP/first-launch (`--all-targets
--no-deps -- -D warnings`), formatting for kernel/MCP/first-launch, and diff
whitespace checks pass. Kernel Clippy reports 14 existing slice-2 warnings;
this slice introduces no new kernel lint warnings. The host dependency `block`
continues to emit Cargo's existing future-compatibility warning.

The three engine-service lane caller hunks in `app/src/session.rs` (scrub event
timestamp, export activity capture and scheduled export start) are included by
explicit lane agreement. Playback and performance-harness changes remain owned
by their lanes. **All on-screen rows remain UNVERIFIED**; no GUI smoke was run.

**On-screen save-in-flight (2026-11-01):** with a 1.2–1.6 s MCP save in flight,
Command-W and Command-Q were refused and the save completed intact. The refusal
note never appeared on screen, and the editor did not reflect the MCP edit on a
60k-layer fixture. Details and repro are in `integration.md`, “Save-in-flight
close/quit on screen”.

## Wave 2: Linux P1 kernel and library evidence

Status 2026-11-01. These are the package 5/7 rows exercised on Linux; this does
not certify the whole P1 gate or the macOS performance rows. All inputs are
synthetic, with no user profile or private corpus.

Verification: **273 kernel + 17 first-launch + 24 migration = 314 passing tests**,
with **1 optional real-profile kernel smoke ignored**. Two 1,000-round soak runs
(seeds `20261101` and `71`) completed **2,000 SIGKILL cycles with zero document
mismatches**; the final seed also asserts saved-base byte preservation during
DiscardClose. All three crates pass strict Clippy (`--all-targets --no-deps --
-D warnings`), formatting and diff whitespace checks. Gitleaks runs before every
commit. The unchanged `W071.fractional-rate` supporting probe reports pass.

| Gate P1 row | Evidence now covered | Remaining verification |
|---|---|---|
| Journal process-kill recovery | `tests/kill_soak.rs` runs the reusable `pm-journal-soak` child-process binary: 18 phase/timing combinations. Seeded kill targets cover multi-field transactions, owner/foreign folding, fold commit, library save/rebase (the headless autosave publication path), and DiscardClose while checkpoint rotation runs. Full recovered documents must match the last delivered ack at before/after boundaries; during a call, only that document or the complete next document is permitted. | Physical power loss, APFS/F_FULLFSYNC, macOS disk pressure, and shell autosave timer wiring. An in-flight write can reach the journal before its reply reaches the client; treating that complete write as a torn write would lose work. |
| Journal flush ≤200 ms | Background-worker test observes four actual fsync completions within 200 ms without a barrier, even with a requested 60-second interval. The effective cadence is clamped to 1–100 ms; fsync time is subtracted from the next sleep. Existing failure/retry and in-flight-barrier tests remain green. | A scheduler stall or unbounded device fsync cannot be given a hard wall-clock guarantee by a worker loop. Measure the ≤200 ms budget on supported Macs under load and physical failure conditions. |
| Library → native edit → old reader | `tests/library_roundtrip.rs` executes v1.2.0 and v1.3.0 project/session/history readers obtained with `git show`, plus the v1.3 gzip decoder. It checks full projects, fractional rate, exact timing/geometry/text/unknown fields, undo, redo, redo cursor, and project/history byte stability over reopen/save. v1.2 plain output also opens in v1.3. | Representative full-profile/library corpus and GUI open/save on macOS. v1.3 gzip output is opt-in and is not advertised as v1.2-readable. Node/Bun is used only by development oracles. |
| v1.3 journal import | Both plain/stringified envelopes and the gzip round trip preserve `recoverySequence`; stale sequence records are skipped, newer records survive a rewound document revision, and v1.2 retains its revision fallback. | Full legacy corpus; migration/host consumers already use this same kernel reader. |
| Authored fractional FPS / W071.fractional-rate | `CompCommand::Set` keeps 23.976, 29.97, 59.94 and 30000/1001 exact through undo/redo; invalid nonfinite batches stay atomic and existing 1–240 limits remain. The unchanged acceptance supporting probe reports **pass**. | Full W071 playback/export cadence and timecode remain platform/integration work. The acceptance crate's separate defect-baseline unit test still expects failure and needs its owner to update it; no acceptance files changed here. |
| Migration delta/resume/archive/proxy | See [first-launch](first-launch.md#wave-2-linux-migration-gates) and [S6](s6-migration.md#wave-2-linux-migration-gates): all durable step interruptions, SIGKILL/resume, interrupted deltas, old object/key/pass retention, immutable unknown/quarantined archives and proxy provenance. | Real profile closure, permissions, Keychain and signed transition/restore proofs. |

The soak samples six phases × three kill timings. Transactions also rotate
checkpoints every three revisions or 32 KiB, using multi-field edits with a
16 KiB name so writes are large enough to exercise partial-tail recovery.
Every child must exit by SIGKILL; a child assertion failure cannot count as a
successful kill. Progress has a timeout, children are reaped on failure, and a
failed round retains its generated fixture with seed/round/timing for inspection.
DiscardClose is also checked to leave saved project bytes identical and, after
its ack, to retain the rejected work only in the explicit archive.

Reproduce from the repository (source the lane environment first):

```sh
cd apps/native
cargo test --locked -p pm-kernel -p pm-native-migrate -p pm-first-launch
cargo run --locked -p pm-kernel --bin pm-journal-soak -- --rounds 1000 --seed 71
cargo clippy --locked -p pm-kernel -p pm-native-migrate -p pm-first-launch --all-targets --no-deps -- -D warnings
cargo fmt -p pm-kernel -p pm-native-migrate -p pm-first-launch --check
cargo run --locked -p pm-acceptance -- --case W071.fractional-rate
```

The tagged tests require local `v1.2.0` and `v1.3.0` tags, plus Node with
`stripTypeScriptTypes` or Bun (`PM_TEST_JS_RUNTIME` may select it). Missing
prerequisites fail the new round-trip test rather than passing silently.
No Node/Chromium is linked into the app (N8).

### Wave 2 decisions

| # | Decision | Selecting rule |
|---|---|---|
| K32 | Preserve finite fractional FPS instead of reproducing shipped command rounding; retain existing limits and atomic validation | N3 authored timing is exact; N1 professional rates; the quirk policy rejects rounding authored intent. |
| K33 | Import v1.3 recovery sequences independently from document revisions | N3/N4: undo can rewind an edit revision; stale replay must not clobber authored work or skip newer edits. The tagged reader supplies the format oracle. |
| K34 | Clamp requested background cadence to at most 100 ms and charge fsync work to the cycle | N1/N2: retain margin inside the 200 ms gate budget; N8 keeps the flush worker separate from the host UI. This is scheduling evidence, not a physical power-loss certification. |
| K35 | Require exact ack state at quiescent kill points; permit only the complete next state for a lost in-flight reply | N3/N4: durable work must not be discarded to make a test match an acknowledgment the client did not receive. N7: seeded, headless, inspectable process harness. |
| K36 | Extend delta interruption tests to immutable base archives, prior passes/objects/keys and beta tombstones; keep proxy flags/version separate from stored-byte identity | N4 archive-never-delete; N3 original/proxy provenance; N6 extension data survives unchanged. |

Strict Clippy also clears the previously documented 14 kernel warnings and its
Linux unused-argument warning with equivalent let chains/emptiness checks and a
fixture initializer; no contract or persistence behavior changes in that cleanup.
