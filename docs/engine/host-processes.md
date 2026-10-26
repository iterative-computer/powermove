# Engine processes and host services (Phase 2 infrastructure)

Design notes for the engine processes, their protocol connections, and host
services in Electron main and in the CLI host. Plan sections 4 and 19,
contracts 7, 8, 9, 12, 13, and 14, and Phase 0 amendments 4, 5, and 12.
Everything here is behind "Use New Engine"; with the flag off the app
starts no engine process and behaves as before.

## The flag

- Store key `useNewEngine` (boolean, off by default). The
  `POWERMOVE_NEW_ENGINE` environment variable (`1` or `0`) overrides it.
- Read once at launch (`main/engine/flag.ts`); flipping it takes a relaunch,
  so one running app never mixes engines.
- Off: main registers one read-only IPC handler (`engine:status`) that
  answers `enabled: false`. No utility process, no ports. The e2e spec
  `e2e/native-engine.spec.ts` checks both states.
- Settings › General › Engine has the "Use New Engine" switch (the
  tracer lane, [tracer.md](tracer.md)); it writes the same store key.

## Processes and ports

| Process | Code | Role |
|---|---|---|
| Main | `apps/desktop/src/main/engine/` | Host services, supervision, port wiring |
| Interactive engine | `utilityProcess` running `out/main/engine-process.js`, addon `pm-node` | Viewer sessions, playback (later lanes) |
| Job engine | same entry, `POWERMOVE_ENGINE_PROCESS=job`, started on first request | Exports, agent snapshots, job snapshot sessions; utility infrastructure QoS, dynamic user-export QoS |

An idle app starts only the interactive utility. `ensureReady('job')` shares
one startup among concurrent callers, including media probes and agent work.
It waits through negotiation, preserves the normal restart policy, cancels
waiters on host shutdown, and reports a fatal addon load without repeatedly
forking it. Late startup receives the current playback policy, memory lease,
font replay, client ports, and direct engine peer connection. Main starts it
as soon as an export or a queued job is asked for, while the offline message
and the save panel are open, and a render queue left running at quit resumes
once the engines are up. The export sheet never routes because the job
utility is stopped: main starts it, and answers `unavailable` only when it
cannot run.

A job utility nothing has used for `JOB_IDLE_MS` (60 s) exits. Main retires
it only when none of its own job requests is outstanding, no activity
happened during the check, and the utility's own report shows no session
(exports, agent frame sessions, validations and any renderer-opened session
hold one), nothing in flight, and no unfinished job, queued work, probe,
frame run or compile. Requests arriving during the check wait and then start
a new utility; an asked-for start after such a stop reports `starting`, not
`restarting`. Eager job fixtures never retire. Attribution, latency checks,
and remaining memory gates are in [memory.md](memory.md).

Main creates every `MessageChannelMain`:

- **Host port** per engine launch. Main opens it with `hello` (role
  `host`); the engine answers `welcome`. Main serves the engine's host
  requests on it. Its `PortId` is the launch's identity for handle binding.
- **Client ports** per window and engine. Posted to the window's main frame
  only (`webContents.mainFrame.postMessage`), never to extension sandbox
  frames. The preload hands each to the page as a transferred window
  message (`powermove:engine-port`); a `MessagePort` cannot cross the
  context bridge. The renderer opens with `hello` (role `renderer`).
- **Peer port** between the engines (amendment 12): the interactive engine
  opens it (role `interactiveEngine`); the job engine accepts. Per-frame
  `frameDone` signals take it; everything else goes through main. Replaced
  whenever either engine restarts.

The utility process tells the engine which kind of peer each port leads to
(`connect {port, peer}` over its control port). The engine refuses a `hello`
whose role does not match, so a renderer cannot pose as the host to grant
leases, relay deadline signals, report crashes, or answer the engine's
`resolveHandle`. Sessions belong to the port that opened them.

Per-frame descriptors (amendment 4) will travel engine → viewer on the
client port; main stays off the per-frame path.

### Control port

Main posts to a `UtilityProcess` exactly once, right after the fork: a
`{type: 'control'}` message carrying a `MessageChannelMain` port. Every
later control message (`connect`, `disconnect`, `shutdown`) goes over that
port. `UtilityProcess.postMessage` to a child that is exiting or has
exited crashes Electron's main process (SIGSEGV, SEGV_ACCERR near 0xf8),
and main cannot know a child is exiting before its `exit` event, so a
window closing or reloading while an engine dies, or one engine's exit
cleanup during `stop()`, could take the app down. A `MessagePortMain`
whose other end is gone drops the message instead.

On top of that, each launch has an `exiting` flag, set when main sends
`shutdown`, kills the engine, sees its control port close, or gets its
`exit`. Control messages to an exiting launch are dropped and the ports
they carried are closed (so their holders see the close). While
`NativeEngine.stop()` runs, the peer and client disconnects that an
engine's exit would otherwise send to the other engine are skipped. The
engine exits when its control port closes, as on `shutdown`.

An engine whose host connection ends gets 500 ms to exit on its own
before main kills it: a dying engine closes its ports just before its
exit, and the wait keeps the real exit status in the crash report.

## Version negotiation

Every connection: `hello` from the opener, `welcome` from the acceptor,
then `Negotiated::permits` on every message (RFC 09). All contracts are
drafts, so both sides must have the same protocol build id (a hash of the
`pm-protocol` sources). `bun run build:engine` refuses an addon whose build
id differs from main's generated `PROTOCOL_BUILD`. Main's codec
(`msgpack.ts`) is strict on input and writes the same bytes as
`rmp_serde` for every message main sends (checked against the golden
fixtures).

## Host services (contract 14)

`HostServicesCore` (`main/engine/services.ts`) in main and `CliHost`
(`engine/crates/pm-host`) for the CLI serve the same requests the same way;
`engine/crates/pm-host/tests/fixtures/host-contract.json` runs against both.

- **Capability handles**: 16 bytes from the OS CSPRNG as 32 lowercase hex
  digits, bound to (session, host port). `resolveHandle` is the only way an
  engine learns a path, only on its host port, only for a handle bound to
  that port. An engine launch ending revokes all its handles.
  `resolveHandle` was added to RFC 14 because a utility process cannot
  receive a file descriptor over a `MessagePort`.
- **Cache areas**: `<userData>/Engine Cache/<area>`, created 0700. Scratch
  is wiped at launch; staged resync documents (`engine:stage-document`,
  at most 512 MiB, 16 per launch) live there and go with their launch.
- **Outputs**: `outputDestination` creates `.powermove-<16 hex>.part` in
  the destination folder with `open(wx, 0600)` and answers with a write
  handle. `commitOutput` removes empty and unverified files (the verified
  byte count must equal the file size), then places the file by policy:
  agent outputs and `uniqueName` use a hard link that cannot replace
  anything (`Name.ext`, `Name (2).ext`, …; volumes without hard links
  reserve the name exclusively, then rename), `replace` renames over the
  chosen file, `fail` refuses an existing name. Unfinished outputs of a
  crashed launch are removed; leftover temporary files in `~/Powermove` are
  swept at launch. Agent outputs go to `~/Powermove`, as today.
- **Hardware codec sessions**: counted per kind across both engines and
  capped (decode 16, encode 2, ProRes decode 8, ProRes encode 2; S8
  recommendation 8; S6 sets the real limits). An engine's sessions are
  released when it exits.
- **Blender** requests from the engine answer `unsupported` until the
  Blender lane connects them; the current Blender service is unchanged.
- **Definitions**: main keeps each window's latest definition set
  (`engine:definitions`, same rules as `DefinitionSet::apply`) for sessions
  main opens itself (job snapshots), and pushes it as a full update.

## Memory leases (contract 8)

- Global budget: half of physical memory, never more than physical minus
  4 GiB, never less than 1 GiB (8 GB → 4 GiB, 16 GB → 8 GiB, 128 GB →
  64 GiB). This answers RFC 08 open question 1 until the memory gate
  measures headroom.
- Split in per-mille integers (identical in TypeScript and Rust): idle
  600/400 (interactive/job), playback 850/150.
- A shrink is in force only once acknowledged: until then main counts the
  larger of the old and new lease against the budget, and a grant that
  does not fit is deferred until the other engine acknowledges. At
  playback start the job engine shrinks first and the interactive engine
  grows after; at playback end the reverse.
- While one engine is down, `requestLease` lets the other use what is
  free, up to nearly the whole budget. When the down engine comes back,
  its initial lease first shrinks the other back to its share (reason
  `memoryPressure`), and the newcomer's grant grows to its share once that
  shrink is acknowledged.
- The engine (`pm_engine::lease::LeaseBudget`) charges every category,
  evicts in a fixed order (frame cache first, surface pool last), answers a
  grant with usage, and owes the acknowledgement until usage fits; it
  reports at least every 5 s while it has sessions.

## Priority and GPU time slicing (contracts 12 and 13, amendment 12)

- `pm_engine::priority::Scheduler`: one FIFO per class, highest class
  first, generation and background paused during playback, everything else
  in the job engine sliced; work never fails for being busy and reports
  its queue time.
- `pm_engine::slicing`: per-pass unit costs calibrated from GPU timestamps,
  corrected by the 90th percentile of measured over predicted time
  (clamped 0.25 to 16), units packed under the 1 ms target and the
  alignment window, one command buffer in flight, units predicted above
  2 ms refused so the caller bands them finer.
- Main sends the job engine `SlicingPolicy::DEFAULT` (2 ms bound, 1 ms
  target, one in flight, 1 ms guard, 100 ms yield, Blender paused) after
  every handshake, relays `playbackStarted`, `playbackStopped`,
  `deadlineMiss`, and `qualityStepDown`, moves leases with playback, and
  re-sends `playbackStarted` to a job engine that restarts mid-playback.
  When the interactive engine dies, main sends `playbackStopped` itself
  (RFC 13 invariant 9).
- Job infrastructure, agent exports and snapshots, and background and
  generation work run at utility QoS (`pthread_set_qos_class_self_np`). A
  user-started export from the sheet or render queue runs at default QoS
  while playback is idle, switches to utility during playback, and restores
  default when playback stops. Its render and encoder threads refresh at
  frame and GPU band boundaries; parallel CPU conversion workers refresh
  during rows. The render lane returns to utility after each user job.
  The job process's JavaScript transport thread also follows this policy
  while at least one user export is running, and keeps that class between
  native calls. It returns to utility when no user export is running.
  This shared thread can carry concurrent agent messages; their dedicated
  workers retain utility QoS. Queued user jobs alone do not raise it.
  macOS threads do not inherit their creator's QoS, so engine threads start
  through `pm_engine::qos::spawn_engine_thread` and scoped export workers set
  their own class. Chromium's IO and IPC threads in the job process keep
  their own classes. The host WebCodecs encoder window, bundled FFmpeg encoder
  process, and external Blender process are outside this thread-level QoS
  control.
- The shared host clock: engines read `CLOCK_UPTIME_RAW`, main reads
  libuv's `hrtime`; both are `mach_absolute_time`. Each engine stamps its
  start notice and main checks the delivery delay (77 to 387 µs measured).

## Crash handling (contract 7)

- Restart with backoff 250 ms doubling to 30 s; the attempt count resets
  after 60 s up. A launch that fails its handshake is killed and counts as
  a crash. An engine that reports `fatal` (addon missing or unloadable)
  stays stopped.
- The engine sends `inFlight` breadcrumbs whenever the set of attributable
  items changes; on a crash they are the suspects. An item suspected in two
  crashes is marked failed, reported once in the `CrashReport`, and sent to
  every restarted engine (`Event::Crash` with the full failed list), which
  refuses work on it with `crashAttributed`.
- On a crash main revokes the launch's handles, removes its unfinished
  outputs and staged documents, releases its codec sessions, frees its
  lease, tells every window (`engine:crash`, then `engine:status-changed`),
  and after the restart gives every window new ports with a new
  generation. The renderer re-opens its sessions and resyncs. The other
  engine keeps its sessions throughout.
- Electron reports a signal death and an exit status in the same number
  (`exit status 9 (SIGKILL if signalled)`).
- A window's connection can also close while its launch lives: the engine
  closes a connection whose message breaks a contract bound (every
  envelope is validated in `codec::decode`; RFC 09), and a port can be
  lost. Main hands out ports only with a launch, so such a window used to
  stay disconnected for good (C1: `connected: false`, `sessions: 0`,
  generation 1, the open never finished). The window now asks main for
  fresh ports itself (`engine:request-ports`, `PortReconnector` in
  `renderer/src/engine-connection/reconnect.ts`) with the supervisor's
  backoff (250 ms doubling to 30 s, reset after a minute connected),
  asking again every 2 s plus the backoff until a port connects; its
  sessions reopen and resync as after a restart. Main's
  `NativeEngine.refreshPorts` replaces the window's ports on every running
  launch and keeps its definitions (a reload still forgets them).
- Strings the renderer puts in protocol messages are bounded in UTF-8
  bytes, never in UTF-16 units (`shared/protocol-strings.ts`). A
  compatibility raster key over 512 bytes, or with a lone surrogate,
  becomes `powermove:key-sha256:<digest of its code units>`; a cut would
  merge two frames' keys and `reuse` the wrong pixels. Failure messages
  are cut at a code point.

## Large documents (C1, 2026-10-19)

A real user's production project from the private setup corpus (PMV4,
238 MB: a 180 MB document of 258 layers baked to a keyframe per frame on
seven properties, 1.2 million keyframes, plus 50 PNGs and an MP4) did not
open with the engine on. Three faults, each enough on its own:

1. A raster wrapper's own cache key carries the layer's text. Cut at 512 UTF-16 units, non-ASCII text made it up to 1,536 bytes;
   the engine refused the `compat` event and closed the window's connection
   (`the engine closed the connection: contract 2 (Frame): compatibility key
   length`), and nothing reconnected (see Crash handling).
2. The interactive engine held the document many times over. A pm-model
   tree was about 10 times its JSON (1.8 GB here), and the engine kept a
   session mirror, a cloned JSON value queued for the presenter, the
   presenter's and the audio thread's own trees, a full clone per snapshot
   job revision, and all of it again for main's snapshot session for the
   window: 13.3 GB RSS during the open.
3. The renderer passed V8's 4 GB heap (`V8 javascript OOM
   (CALL_AND_RETRY_LAST)`, "Target crashed"). Today's open of this project
   already peaks near 2.9 GB on its own whole-document strings (the file
   fingerprint, `stringifyAsync`, the autosave's clone); the engine path
   added a sorted-key canonical string of the shadow per resync and per
   document sent with a synchronous snapshot, two more in the save-time
   consistency check, and an encoded `load` batch nobody reads.

What changed:

- **pm-model** (`doc.rs`, `json/object_map.rs`). Arrays and objects hold
  their children behind an `Arc`: cloning a node or a `Document` is O(1),
  a patch copies only the containers on its path, and reconciling copies a
  shared container only when its content differs. Object members are a
  sorted vector (`ObjectMap`) instead of a `BTreeMap`, whose every object,
  even a two-member ease value, took a full eleven-slot leaf. The private
  document's tree went from 2,398 MB to 1,039 MB footprint, and pm-wasm's
  copy in wasm memory shrinks the same way.
- **pm-engine**. A resync parses once; the presenter and audio threads get
  the same tree (`DocumentUpdate::Replace` carries a shared `Node`), as do
  snapshot jobs. A resync reconciles against a mirror the engine already
  holds for the document (the session's own, or another session's), so
  consecutive resyncs and main's window session share unchanged subtrees,
  and a resync to a hash another session was checked at, with nothing
  applied since, takes that mirror without reading or parsing its body.
- **Renderer** (`engine-doc/barrier.ts`, `shared/canonical-json.ts`).
  `snapshot()` hands receivers pm-wasm's canonical JSON as bytes copied out
  of wasm memory (pm-wasm holds the same revision; its hash already went
  with it), so a resync or a document sent with a synchronous snapshot is
  never a JavaScript string. A `load` batch's body is encoded only if read.
  `canonicalHash` feeds SHA-256 in pieces of whole JSON tokens instead of
  building the string (same digest for every value shape).

Measured on the private project and on its public stand-in
(`e2e/engine-large-project.spec.ts`): see "Large documents, measured"
below.

Later rounds found more, each fixed on the same branch: the window kept a
clone of every autosaved project slot (now lazy with the engine on); each
asset a restore read scheduled its own whole-project autosave (now one,
after the restore); the dirty check held two whole-project strings (now
two fingerprints); the font prewarm laid each text layer out at every
baked transform key (about 50,000 layouts; now its text's own keys); the
shadow was a `JSON.parse` of the resync JSON (now `jsonClone`, which
shares strings: 249 MB instead of 601 MB); the tree used one `BTreeMap`
leaf per object (now `ObjectMap`, 1.0 GB instead of 2.4 GB); level 1 text
put a fallback font's run after the text that follows it ("⌘K" drew as
"K⌘"); and a busy window's async snapshots were refused by main instead
of queued (`snapshot-slots.ts`).

### Large documents, measured (2026-10-21)

Release LTO build, M4 Max, other lanes running, one Electron per run,
fresh isolated profiles with the corpus's 39 extensions. Today is the same
build with the setting off. The private harness is outside the repository;
it reads the setup directory from `PM_PRIVATE_SETUP_DIR`.

The failing project (PMV4, 180 MB live document, 258 layers):

| | Today | Engine |
|---|---|---|
| Open (choose file → open resolves) | 325 s | 447 s |
| JS heap after the open, after GC | 1,209 MB | 1,334 MB |
| Sampled JS heap peak during the open | 3.36 GB | 3.38 GB |
| Interactive engine RSS peak | none | 1.7 GB (13.3 GB before) |
| Connection | n/a | one connection throughout; a closed one is back in 3.7 s |
| 18 corpus frames at the corpus bound | reference | 16 pass; 2 at max 6 CV (99.994 % and 99.981 % within 2 CV) |
| Playback, presented frames in 8 s | 230 (p50 40 ms, p95 60 ms) | 33 (p50 50 ms, p95 439 ms, max 1.8 s) |
| Scrub, 24 seeks to a presented frame | p50 21 ms, p95 48 ms | p50 39 ms, p95 353 ms |
| Export, MP4 at half size | 21.7 s | refused: one offline clip (section 14.8 message; today exports without it) |

Both opens spend about 170 s in three synchronous whole-project store
writes (`PM.store.set` of the 180 MB slot: structured clone plus IPC, 50 to
60 s each), a cost today already has. The engine's extra 120 s is the
barrier (73 structural diffs of the whole document, 53 s; the longest
13 s), two resyncs, and the save-time consistency hashes.

The intermediate project (PMV3, 12 MB live document, 867 layers, 114 MB of
undo history in the file):

| | Today | Engine |
|---|---|---|
| Open | 125 s | 122 to 130 s |
| JS heap after the open, after GC | 171 MB | 177 to 179 MB |
| Sent to the engine | n/a | 10.7 MB (the canonical document only) |
| Undo history | 100.7 MB of JSON in the renderer | the same, never sent or re-serialised for the engine |
| Frames (`renderFrameToAsync`, 16 motion blur samples, 1080p) | 0.7 to 1.8 s each | 6 to 56 s each; one over the 40 s answer bound |
| Playback, presented frames in 8 s | 135 (p50 53 ms, p95 173 ms) | 1 (the clock ran to 9.4 s; the viewer kept its first frame) |
| Scrub, 24 seeks to a presented frame | p50 72 ms, p95 145 ms | p50 576 ms, p95 3.5 s |
| Export, MP4 at half size | 159 s | not measured (frames above) |

Both projects now open with the engine on and keep their connection, and
the renderer heap is within 10 % of today's. Rendering them is not yet as
fast as today: with 867 layers the engine draws a frame in seconds, so
playback stalls and scrubbing is 8 to 25 times slower; the failing
project plays at about 6 frames a second. Leads from the logs: the
compatibility rasters a frame waits for (about 1,000 needs answered per
minute of use, 251 async raster snapshots), effects routed to the shader
fallback runner until the first-use ANGLE parity probe pins them (a fresh
profile, as here), and shared-texture transfers that time out while a
6.7 s synchronous snapshot blocks the renderer.

Open with the public stand-in (17 MB of document): today 9.5 to 14.2 s,
engine 15.7 to 20.4 s; heap after GC today 354 to 367 MB, engine 317 to
325 MB; interactive engine RSS 919 to 925 MB; four frames within 2 CV.

## Resync

`resync` with `canonicalJsonV1` installs a `pm-model` mirror after checking
the canonical hash, inline or from a cache file the engine resolves with
main; `checkConsistency` compares hashes at one revision. Patch batches
follow the applied revision and apply through a pluggable RFC 05 decoder;
until the model lane's binary decoder is installed, a batch is reported as
`unsupported` once per session and the mirror keeps the resynced revision.

Large ingestion now runs on `pm-document-reader`, a single engine-QoS thread
with two queued inputs, shared by all sessions. All staged-file reads and inline
bodies of at least 64 KiB are read/parsed, reconciled, and canonical-hash-checked
there (`pm-engine/resync.rs`). Smaller inline control documents remain immediate.
The port loop installs only a completed, checked tree, provided the session,
project, and applied base revision still match. Cancellation and closing a port
discard the pending install; an overlapping resync is refused explicitly. The
reader bounds file reads to the announced length plus one byte and joins at
engine shutdown. Clock, lease, and port traffic continue during ingestion.

Short reconnects use a verified checkpoint plus the renderer's recent patch
suffix. A session retains its last checked shared tree while its suffix remains
within 128 batches and 8 MiB. A disconnected port leaves at most one such tree
in the process for 15 seconds. A new session can prove the checkpoint hash
without sending a JSON body, then replay its suffix. A process restart, expired
checkpoint, different project/hash, or overflow takes the full checked resync.
The checkpoint shares unchanged subtrees; it is not a second parsed document.

`Document::canonical_digest` caches its digest until a mutation (including a
partially applied failed batch). The wire hash remains RFC 05 SHA-256; local
renderer/pm-wasm consistency checks additionally use an incremental integrity
digest described in `write-path.md`.

Disk: a window may stage at most 16 documents of at most 512 MiB per
engine launch in `Engine Cache/Scratch`; they are removed when the launch
ends and the folder is wiped at the next engine start. Temporary
`.powermove-*.part` outputs are removed when their launch ends; after main
itself crashes, only the automatic `~/Powermove` folder is swept at the
next start, so a partial file can remain in a user-chosen export folder.

## Amendment 5 items outside this lane

Amendment 5 (S1 gate) adds two Phase 2 items that are not delivered here:
moving autosave serialization off main's thread (it touches the project
store and autosave code in main and the renderer, which this lane does not
own) and the C′ prototype (`CAContext`/`CALayerHost` presentation from the
utility process for playback, measured with the S1 harness), which belongs
to the frame bridge work. The engine processes here are where a C′ layer
would be created, so that lane builds on them.

## Packaging and signing

- `bun run build:engine` builds `pm-node-addon` (the cdylib over `pm-node`)
  with the pinned toolchain and the release profile's thin LTO into
  `apps/desktop/native/pm_node.node` (ignored by Git) and checks its
  protocol build ([release-build.md](release-build.md)).
- `electron-builder.yml` ships `native/pm_node.node` unpacked from the asar
  (`process.dlopen` cannot map out of an asar).
- `scripts/release-mac.mjs --with-engine` builds the addon and requires it
  in the bundle. Every release now checks every `.node` and `.dylib` under
  `app.asar.unpacked` for the app's Team ID and the hardened runtime, and
  refuses a stray local `native/pm_node.node` without `--with-engine`. The
  release workflow does not pass `--with-engine` and has no Rust toolchain.

## Tests

| What | Command |
|---|---|
| Engine core, leases, slicing, scheduler | `cargo test -p pm-engine` |
| Addon glue, utility QoS | `cargo test -p pm-node` |
| CLI host and the shared host contract | `cargo test -p pm-host` |
| Main's codec, host services, supervision with fake engines, shared contract | `vitest run src/main/engine` |
| Both engines in Electron: negotiate, resync, SIGKILL each, restart, resync, reconnects racing a dying engine, clean stop, restart, stop while an engine crashes | `bun run test:engine-host` |
| The flag in the real app (hidden window) | `playwright test e2e/native-engine.spec.ts --workers=1` |
| Shared document trees, reconciled resyncs, `ObjectMap` | `cargo test -p pm-model -p pm-engine` |
| Renderer protocol strings, the raster override's keys, reconnecting a window, `jsonClone`, the font prewarm's key times | `vitest run src/shared src/renderer/src/kernel src/renderer/src/engine-connection src/renderer/src/engine-doc` |
| Level 1 fallback runs in text order (macOS CoreText) | `cargo test -p pm-text --test blink_sizes` |
| A large synthetic project (16 embedded PNGs, text a `raster` wrapper redraws under keys over the protocol bound, a key per frame on seven properties of 64 layers, 17 MB of document): today's and the engine's open, heap, engine RSS, frames, playback, and a connection the engine closes coming back | `pm-electron-lock bunx playwright test e2e/engine-large-project.spec.ts --workers=1 --global-timeout=0` |

Measured on the reference machine (M4 Max, shared with other lanes), 20
consecutive runs of `bun run test:engine-host`, 20 passed: both engines
start and negotiate in 75 to 86 ms; a SIGKILLed engine is back,
renegotiated, and resynced by the client in 313 to 337 ms, of which 250 ms
is the first backoff step. Before the control port, Electron main died
with SIGSEGV during `stop()` in about a third of runs.

### Reader lifetime during a large file open (2026-10-21)

The main-process project reader starts its 120-second idle timeout after the
first completed media-range read, and pauses it during reads. Sending a large
document and history over IPC can itself take longer than that timeout; expiring
the token before the renderer can request its first range made an import resolve
with an error toast and an unchanged project. Explicit close and window destruction
still release readers. Each window has at most 32 outstanding readers, including
initial responses; a refused open closes its newly opened token.

The private opening harness must check that the document identity changed and
that the production document loaded, since `PM.openProject()` also resolves after
a handled import failure. An empty project is never an opening-time sample.

Beta DECISION bug fix: reader lifetime applies with either engine setting. A
reader cannot expire while its initial response is still crossing IPC. The
project-operation queue also retains only completion, releasing the full parsed
open response after its caller releases it. Intermittent blank opens must still
be counted separately; the lease fix does not establish the cause of another
handled open failure without its error toast.

Beta DECISION bug fix (2026-10-22, setting off): the large private project
opened blank in 3 of 6 attempts with today's app; one was the reader expiry
above. In the other two, the open reply's main-thread task (65 to 85 s)
ended, and the document was dropped before hydration. Those runs predate
toast capture, so the error is unknown. Engine opens, which restore embedded
media through main rather than the page, never blanked.

One candidate on that shared path is fixed. With the setting off, the open
restores embedded media into the page's IndexedDB store. That store's first
`indexedDB.open` had a five-second wall-clock timeout, which resolved the
shared connection to nothing and failed every waiting write ("Could not
restore … Check available disk space."). A main-thread stall counted against
it, and a connection that opened after the timeout leaked. The timeout now
counts only time in which the page could handle the answer: a late poll
counts at most 500 ms. A late connection is closed. `legacy/core/media.test.ts`
reproduces the timer-first ordering and keeps the bound.

Live reproduction failed. The page was blocked 8 s during the first open,
and also with main busy 3 to 20 s plus a 30 s page block. In every case
Chromium ran the queued open answer before the overdue timer. So this fix is
not established as the cause of the two blank opens.
`e2e/media-store-long-task.spec.ts` keeps the blocked-page case as a
regression test. The private harness now records every toast, the storage
writes, and the open window; later blank opens will name their error.

The ingestion reader admits one active document and two queued documents. A
full queue answers `budgetExceeded` with `Retry::After(250 ms)`. Renderer and main
resync callers honor that admission retry using the same snapshot and staged
file, without serializing another document. A connection close cancels renderer
retry timers; main retries remain fenced to the same engine launch. Permanent
parse/hash errors remain failures.

### One native document for large interactive projects

Large file and tab opens prepare the document before publishing `PM.proj`.
The renderer keeps the existing raw JavaScript objects and an independent JSON
shadow for patch generation and alias checks. It clones and traverses these in
cooperative slices, and streams canonical UTF-8 chunks through the trusted host.
There is no whole-document canonical string, UTF-8 input allocation, or SHA state
in the renderer's preparation path. Small documents, headless evaluation, and
standalone evaluator diagnostics continue to use WASM.

Main owns each temporary file, independently hashes its incoming chunks, and
removes the file in a `finally` block. The native reader parses the file and warms
both canonical and incremental integrity digests off the utility's JavaScript
thread. A prepared receipt does not replace the live evaluator. Publication
activates the checked tree at a revision after any edits to the previous project.
The evaluator and viewer then share the same native document containers; the
renderer does not grow a large, permanently resident WASM linear memory.

Synchronous extension APIs retain immediate native evaluation through an
owner-fenced host call. Existing synchronous project assignments also retain
their semantics. Ordinary file and tab opens use the asynchronous preparation
path. A failed preparation leaves the previous evaluator intact. Recovery checks
the project, barrier, and edit epoch before activation and retries if they changed.

Save, export, and idle checks compare live aliases against the shadow in bounded
slices before consulting cached subtree digests. Small digest operations are
batched to the host's independent SHA implementation; large leaves stream in
chunks. An edit invalidates ancestor caches and fences outstanding asynchronous
cache writes. Announced edits crossing a check cause a retry. As with any traversal
of mutable raw objects, unannounced writes to already visited members cannot be
made atomic across asynchronous tasks without changing the public alias contract.

RSS accounting must include renderer WASM high-water memory, transient canonical
staging, native model allocation, evaluator plans, and check caches, as well as
the JavaScript heap. Native heap diagnostics explicitly label tree sizes as
estimates; they are not RSS measurements. Short native JSON keys and values fit
inline without changing `JsString`'s three-word size. Integrity cache entries use
weak container references and a fixed entry bound; the wire digest is a cached
32-byte result, not a retained canonical document.

Prepared streams and digest tokens are owner-scoped and cleaned up on renderer
navigation, crash, destruction, cancellation, and shutdown. Pending owner cleanup
reserves that slot until earlier commits settle. Bounded reader backpressure
retries the same private file rather than creating duplicate staging. Binary
patches and fonts cross IPC as typed bytes, without JSON number-array expansion.

The native kernel retains the renderer's verified reconnect base across edits.
When the bounded patch suffix expires, a reader job hashes the current immutable
root; only the renderer's subsequent checkpoint acknowledgment replaces that
base. A stale root, revision, or owner cannot publish the receipt. Native viewer
reconnects use the verified root and suffix without a renderer JSON snapshot.

Diagnostic checks and recovery have bounded retries. Local saves persist the
authoritative JavaScript document even when a diagnostic or utility recovery
fails. Native export still waits for a checked, repaired mirror. Preparation and
recovery share one serialized queue; a revive request goes through connection
recovery instead of racing a second ingestion.

### Opening the private projects, paired medians (2026-10-22)

These runs used a release LTO build with the setting off ("today") and on
("engine"), on the same build. They were interleaved one pair at a time, three
runs per mode and project, with fresh profiles plus the corpus's 39 extensions.
Other lanes were running on the machine throughout. Memory was sampled 5 s
after the open resolved, after a forced GC. Physical footprint (`footprint`)
is what macOS charges a process. RSS also counts resident clean and
reusable pages, and it does not count GPU-owned memory. Totals cover the app's
process tree and exclude FFmpeg proxy conversions. Those are started by an
extension's 4K import, overlap the sample at random, and run in both modes.

| Median | PMV4 today | PMV4 engine | PMV3 today | PMV3 engine |
|---|---|---|---|---|
| Open (s) | 372.0 | 419.3 | 120.1 | 135.3 |
| Renderer RSS (MiB) | 2,342 | 3,062 | 1,560 | 1,716 |
| Total RSS (MiB) | 4,758 | 6,325 | 2,608 | 3,428 |
| Renderer footprint (MiB) | 1,549 | 1,815 | 965 | 1,129 |
| Total footprint (MiB) | 3,474 | 4,716 | 2,582 | 4,289 |
| Engine processes' footprint (MiB) | n/a | 1,014 | n/a | 999 |
| Longest main-thread task in the open (s) | 81.4 | 78.6 | 110.5 | 116.8 |
| Tasks of 100 ms to 20 s in the open | 31 | 6 | 6 | 8 |
| Longest of those (ms) | 1,197 | 1,623 | 3,314 | 3,273 |
| Blank opens | 0 of 3 | 0 of 3 | 0 of 3 | 0 of 3 |

Every open is dominated by work today already does. On PMV4 that is one
open-reply deserialization (62 to 75 s) and four synchronous whole-project
storage writes (56 to 79 s each). Those are `switchProject`'s put, the
autosave it schedules (a recovery copy with a thumbnail), the poster
backfill's `checkpointAssetMetadata`, and `captureProjectSession`. On PMV3 it
is one 90 to 119 s reply carrying its 114 MB of undo history.

The engine adds native preparation before publication: 23.5 to 34.7 s on
PMV4, cooperative and without long tasks, and 2.4 s on PMV3. Its PMV4
storage writes also ran 0 to 18 s slower each once the viewer was active.

An earlier engine pass (363 s against today's 397 s) came from a build whose
autosave dropped its write when an edit arrived during the async check (four
long tasks instead of today's five). Engine opens now perform exactly
today's writes.

Shrinking `JsString` to two words made a document node 24 bytes instead of 32.
The PMV4 tree fell from 438 to 355 MiB live (−19%), and from 462 to 362 MiB
after allocator rounding. The utility's dirty allocator memory fell from 642
to 552 MB, and its footprint from 1,099 to about 1,010 MiB. The two no-op
structural diffs are gone: `structuralDiffs` went from 2 to 0, and the flush
total from 958 ms to 0.2 ms.

What remains above today:

- **Utility.** About 370 MB is the native tree and 180 MB the rest of the
  allocator. GPU-owned memory is 490 MB on PMV4 and 0.7 to 1.3 GB on PMV3:
  frame pools and pipelines, which belong to the frame-pipeline and
  engine-memory lanes.
- **Renderer, PMV4.** The barrier shadow plus allocator account for +266 MiB
  of footprint. RSS is a further 450 MB higher, from V8 and allocator pages
  that are freed but still resident after the preparation's transient peak.
  The V8 heap reached 1.9 GB before GC against today's 1.4 GB, then fell
  below today's after GC (1,164 vs 1,124 MiB used, within noise).

Leads, by expected size:

1. A 16-byte node (a 32-bit revision beside a compact scalar payload) would
   save about 170 MB on PMV4.
2. Seeding the renderer's integrity cache from the native receipt, instead of
   a separate canonical pass, would remove one of preparation's three
   whole-document passes.
3. Folding each object's `Arc` header into its values allocation would save
   about 37 MB.
4. The storage writes during the open are a shared cost of about 4 minutes;
   each is a full main-thread freeze for both settings.

### Open-time storage writes and preparation (OPENWRITE-1, 2026-10-22)

Beta DECISION bug fix (OPENWRITE-1, both settings). Opening the large private
project froze the window for about six minutes with either setting. The
cause was the context bridge, not serialization. An object passed from the
page to preload is copied member by member on the page's JavaScript thread.
Two such crossings happened per open:

- **The open reply** carried the parsed document: 62 to 75 s for PMV4, and
  90 to 119 s for PMV3 with its undo history. Main now also returns the JSON
  text it parsed (`documentJSON`; for PMV3, the header with
  `documentMember: 'document'`), and the window parses it in about a second.
- **Five whole-project slot writes** (`PM.store.set` of `project.<id>`),
  at 56 to 79 s each. They were switchProject's put, the autosave it
  schedules, the poster backfill's checkpoint, the session capture as the
  open finishes, and the autosave after it. A project slot now crosses as its
  `JSON.stringify` text through `storage:set-serialized`. That is the same
  atomic snapshot of the call that the structured clone was, about a second
  for PMV4. They are also the bytes main's engine stores: it used to
  `JSON.stringify` the clone. Reads answer from the posted text until main
  has it.

The writes are also coalesced (`core/projects.ts`):

- An open checkpoints the document once, at the same point as before
  (switchProject).
- A later `Projects.put` of a large slot (4 Mi characters or more) that this
  window wrote from the same object, with an empty journal, is checked
  instead of rewritten. The document is serialized in slices (`json-slices.ts`:
  exactly `JSON.stringify`'s text, in 4 ms slices, about 0.5 s for PMV4) and
  digested with SHA-256. It is rewritten (synchronously, as a fresh snapshot)
  if the digest differs, or if a change was announced three times while it was
  read. Small slots are written directly, as before.
- Asset metadata changes outside typed history (the poster backfill) are one
  journal entry replacing `assets` (about 20 KB here), instead of a full
  checkpoint. The check understands a journal that holds only such entries.
- Main owns a project's durability (SAVE-1, below); the window only posts
  slot snapshots and journal entries, in order.
- The periodic checkpoint that keeps the journal short (every 50 revisions,
  or once this window's copy of the journal passes 4 MiB) is serialized in
  slices for a large slot, and posted (`PM.store.setText`) only if nothing
  changed while it was read: no announced change, document write, engine
  barrier epoch, open history transaction or other slot write. It is then
  the document as it was when posted, with exactly the entries posted before
  it, as main numbers it. Otherwise it stays due and the next autosave tries
  again; main keeps every entry meanwhile. Past 16 MiB of journal (edits
  kept landing while one was read) it is written at once, as before, so the
  journal stays well short of main's limit. Written in one task it was a 0.5
  to 1 s freeze every 50 edits of the private project.

**SAVE-1: one owner for a project's durability.** Three review rounds
patched a protocol in which the window decided when the journal was deleted
or rewritten, case by case; each round found an interleaving of slot writes,
failures and restarts that lost edits already on disk or recovered a
document that never existed. It is replaced by one owner and one invariant
(`main/project-durability.ts`, `shared/project-journal.ts`):

- Main's store engine numbers, per project and in arrival order, every slot
  write and every journal entry (`seq`). It writes the slot's `cp` and `seq`
  at the head of its envelope, and tags each entry with its `seq` and the
  `cp` of the slot on disk when it arrived. The window sends no ids.
- Main keeps `durable`, the newest slot fully on disk, and the journal
  against it. Only a slot write main itself completed changes `durable`, and
  only then is the journal rewritten: the entries numbered after that slot,
  retagged with it (deleted when none). A slot write that fails or is
  refused changes nothing. Journal entries are only appended; the journal
  file is rewritten whole and atomically from main's list, concurrently with
  slot writes. Nothing the window sends can replace or delete the journal:
  `set`, `setSerialized` and `delete` of `projectJournal.<id>` are refused.
  Deleting the slot (deleting the project) deletes its journal after it.
- Recovery (Projects.get, any build that has SAVE-1): the slot plus the
  journal entries numbered after it, in order. A slot holds every edit posted
  before it and an entry follows everything numbered before it, so this is
  right for whichever slot and journal a crash leaves, including a new slot
  with a journal not yet rewritten. Slots without a number (written before
  SAVE-1, or by an older build) replay every entry, as before.
- SAVE-1b: an entry that does not end in an acknowledged write leaves an edit
  missing from the journal, and a later, smaller entry accepted over it
  recovers a document that never existed (reviews recheck 3 and 4). One
  place decides this: main's FileStore (`storage.ts`) closes the project's
  journal at the request id of the latest such edit whenever an append is
  refused for its size there (it never reaches the worker), refused by the
  worker (its journal at the 32 MiB value limit, malformed, or the project
  unreadable in a replacement worker), lost because the worker stopped before
  answering it, or its journal write fails or cannot be replayed. While it is
  closed main refuses every later entry itself, moving the close point past
  each, and the worker refuses entries already on their way (it closes the
  journal itself the moment it refuses one or cannot read the project for
  one). Only a slot write posted after the close point, once it has landed,
  reopens it; main tells the worker. A slot posted earlier, even one landing
  later, does not hold the missing edits and does not reopen it.
- Before the journal shrinks after a slot lands, the slot file and its
  directory are synced (an ordering barrier, one per checkpoint). On macOS
  libuv's fsync is `fcntl(F_FULLFSYNC)`, falling back to `F_BARRIERFSYNC` and
  then `fsync`; the temporary file is synced the same way before its rename.
  On Windows a file is flushed through a handle opened for writing (`r+`; a
  read-only one fails with EPERM), and the directory is not synced (it cannot
  be opened to); NTFS journals the rename. No Windows build ships yet.
- A replacement storage worker reads a project's slot head and journal from
  disk before it touches the project. Main first tells it which journals are
  closed, then replays what had not reached disk in the order it was first
  made (request ids), not map order. A replayed slot reopens a closed journal
  only if it was first posted after the close point (main compares the
  original request id). A journal replay is merged with the journal it read
  from disk, by sequence number (the stopped worker may have written entries
  whose replies never reached main). A replay it cannot apply is reported as
  not saved; if it was the journal's, the worker closes the journal at once
  (entries already on their way are refused) and so does main.
- A bare project (a slot that is not a `{v, proj}` envelope; nothing writes
  one today) is not numbered; when it lands, the entries posted after it
  are kept, and recovery replays them over it.
- The window marks a project unsaved when one of its writes fails (main
  forwards persist failures to the last window whose write of the key it
  accepted; a refused write from another window does not take that over,
  review recheck 6 R6h), so its next
  checkpoint writes rather than checks and its next autosave checkpoints.
  While slot writes fail, every journal entry still reaches main; past the
  4 MiB bound the window does not checkpoint again on each edit. Once main
  refuses an entry the window keeps sending every later one (main must see
  each refused edit to know which slot holds them all) but stops keeping its
  own copy; that lasts until a flush finds the journal no longer failing (a
  slot holding the edits landed), not until a checkpoint is merely posted.
  The edits stay in the document, and the save error stays reported.
- `PM.store.flush()` learns from main which keys are still not on disk (a
  persist failure, or a refused entry, until a later write of that key
  lands), so a transient failure stops being reported once the disk
  recovers. "Could not save" appears once per failure episode, not once per
  edit: keys that fail together before the next flush share one toast, a key
  stays quiet while it keeps failing, and a key that starts failing after a
  flush is a new episode (so an unrelated error that never clears does not
  hide a later project's failure). A key main still reports failing stays
  quiet even after a flush with no new error, so a background checkpoint
  retried on a full disk is the same episode.
- SAVE-1c: a project is written only by the window that has it (main's window
  registry, `windows.ts`). Rename… on the Projects screen of a project open in
  another window used to write the stored copy (slot plus journal), which
  lacks that window's newest edits, among them any the journal refused; it
  landed after the close point, reopened the journal, and the owner's next
  edit was accepted over a document it never had (review recheck 5). Now:
  - a rename of a project another window has goes to that window
    (`window:rename-project`), which renames its live document and
    checkpoints it; a background tab of the same window is renamed through
    its live (parked) document; only a project no window has is renamed from
    its stored copy, as before;
  - main refuses a slot write from a window that does not have the project
    (each store write carries its window's id), so no stored copy ever lands
    beside the owner's edits; and a closed journal reopens only for a slot
    from the window that has the project when it lands;
  - main refuses a delete of the slot from a window that does not have the
    project, the same way (review recheck 6, R6e). Delete Forever from Trash
    (or the boot prune of empty projects) in one window, of a project open
    in another, used to delete the owner's slot and journal while it kept
    editing; a crash before its next slot lost the project. DECISION: the
    delete is not routed to the owner (that would close its tab, a visible
    change), and over IPC it is dropped quietly, so the deleting window sees
    exactly what it saw before (the project leaves its Trash, no error), and
    the owner's next checkpoint puts the project back in the library, as it
    did before. Only the slot and journal are kept: the deleting window
    still removes the project's metadata, history and workspace keys, which
    the owner rewrites on its next session capture;
  - a main-internal write or delete (no window id) is never refused, and a
    slot from it reopens a closed journal. Nothing in main writes or
    deletes a project slot today; such a writer must hold every edit the
    owner made.
- After a refused entry the window checkpoints 250 ms after the last refused
  edit, not at the next autosave (whose debounce grows to a large project's
  last write time, up to 60 s), so the journal reopens as soon as that slot
  lands while the user pauses. Each refused edit after a checkpoint was
  posted moves main's close point past it, and a large project's checkpoint
  costs about a second of serialization in the window and its whole size in
  IPC, so the checkpoint is throttled (review recheck 6): one is in flight
  at a time (posted, then a flush until it is on disk); refused edits
  meanwhile are covered by the next one, whose timer each refused edit
  restarts; while the journal stays closed after one landed, the wait
  doubles (and is at least four times the last checkpoint's own cost), up
  to 8 s while edits keep being refused and 60 s when none was (the slot
  write itself failed). It stops only when the journal reopens (the wait
  starts again at 250 ms) or the project leaves the screen (its switch
  checkpoints it), so a closed journal still converges to a landed slot.
  Twenty refused edits 300 ms apart post 2 checkpoints, not 20
  (`projects.behavior.test.ts`).
- Known gaps in the single-writer rule (review recheck 6), none a lock-out:
  - R6d, the no-owner gap: a window reads the stored copy before it claims
    the project (`switchProject` claims fire-and-forget), and a closing
    window's dirty check can post its slot after it released the project.
    If that late slot lands in the gap it is accepted, and recovery then
    mixes the closing window's raw (unjournaled) change into the new
    owner's journaled edits until the new owner's next slot. Raw changes
    already reach disk only with a slot, so this is narrow.
  - R6g: an unstamped (main-internal) slot is never refused and reopens a
    closed journal (above). There is no production caller.
  - A renderer that crashed but was not destroyed keeps ownership (main
    checks only `isDestroyed`), so a rename routed to it is dropped and the
    asking window does not fall back to renaming the stored copy. This
    matches the claim rule (no window can take the project either) and is
    not a new lock-out.
- Raw (unannounced) writes reach disk only with a slot, as before: a crash
  recovers the last slot and the journaled edits after it.

**On-disk format.** A project slot is `{"cp":"<id>","seq":n,"v":…,"proj":{…}}`
and a journal entry `{"revision":…,"patches":[…],"seq":n,"cp":"<id>"}`. The
members are optional and backward compatible: older builds read `proj` and
replay every entry, and data without them is read as before.

- Downgrade, then upgrade: an older build writes its journal on its own
  queue, so a crash (or a failing slot write) can leave a SAVE-1 numbered slot
  beside that build's unnumbered entries. The older build recovers that
  correctly; back on SAVE-1, a numbered slot replays only numbered entries,
  so those edits are dropped. It needs a downgrade, edits, a crash in that
  window and an upgrade.
- Power loss: correctness needs a slot's rename to be on stable storage before
  the journal shrinks, which the barrier above provides on macOS and Linux
  (file and directory synced). It also assumes the file system makes a
  rename atomic and keeps a synced file's contents (APFS, ext4, NTFS). The
  model check below covers process crashes, not power loss: its fsync does
  nothing. Measured cost of the barrier (slot write plus flush, load average
  35, other lanes running): a 0.12 MB slot, median 17.1 ms of which the barrier
  8.8 ms (the temporary file's sync 4.5 ms); a 63 MB slot, median 349 ms of
  which the barrier 13.8 ms (temporary file 9.8 ms).

**What is proven.** `main/project-durability.model.test.ts` is a model check
of the real FileStore, store engine and durability module on an in-memory
file system where every rename into place and every unlink waits for the
checker. Window A has the project (main's ownership says so) and is a script
that keeps every edit in its document; window B can write the project from
its stored copy. The model never changes the owner: ownership is fixed to A
for the whole schedule, and B's write is always refused before it is
posted. At every step the checker chooses among: posting A's next
operation, or its next two in one task; completing or failing any pending
disk operation; and the faults. The faults: a disk operation that fails; a
slot write or journal entry the worker refuses (alone, with the next
operation already on its way, or with its reply arriving late, after later
posts); an entry main refuses for its size before posting it; the worker's
replies held while later posts go out; B's write of its stored copy; the
worker replaced (pending operations abandoned before or after they landed);
an entry lost in flight as the worker stops; a replacement whose first read
of the project fails. States reached twice are explored once (the key holds
main's and the worker's state, held replies, and what the check has seen).

Three sweeps, 0 violations (about 21 min at load average 20 to 50): every
arrangement of 3 slot writes and 4 patches with any one fault of the kinds
before SAVE-1c (no held replies or second window), 304,150 schedules; every
arrangement of 2 slot writes and 3 patches with any one fault of any kind,
143,613; every arrangement of 2 slot writes and 2 patches, and of 1 slot
write and 3 patches, with any two faults of any kind, 282,922. Every disk
state of every schedule (each a crash point, for a crash of the whole app)
recovers a state A's document had, never older than an earlier crash point,
with every patch the worker had acknowledged as written. Once every write
has settled (quiescence), disk recovers the final document or main's
failing keys name the project; then (liveness) one more fault-free
checkpoint and edit from A must be accepted, recover the final document, and
leave nothing failing.

A fourth sweep, the two-fault sweep from before SAVE-1c, is kept but slow, so
it runs only with `PM_MODEL_SLOW=1` (beside the file's other `PM_MODEL_`
runs): every arrangement of 2 slot writes and 3 patches with any two faults
of the earlier kinds, 101,959 schedules, 0 violations (3.4 to 9 min at load
average about 30; the same count as the review's run at recheck 6).

Each of these deliberate bugs is caught: a failed slot write that shrinks the
journal; recovery that ignores sequence numbers; a journal shrunk before its
slot is on disk; a replacement worker that does not read the journal first;
a journal write silently skipped; an entry accepted after a refused one; a
journal reopened by a slot older than the close point; a journal left open
after an entry main refused for its size, lost as the worker stopped, or
refused or unreadable in the worker (with the next entry on its way); a
replayed slot judged by its replay's request id; and the four of review
recheck 5: a journal write failure or failed replay that does not close in
main (liveness), a close point that can move backward (a late refusal, then
main's own; two faults), main's own refusal closing at an older entry, and
main reopening without telling the worker (liveness). Also caught: a
non-owner's slot accepted, a worker that does not close on a failed journal
replay, and a replay that replaces the journal instead of merging; and the
two of review recheck 6, which the unit tests also catch directly now
(`main/storage.test.ts`): a non-owner's slot refused only while the journal
is closed (M1), and a replay merge that does not advance the sequence (M2). The
sweeps found four bugs in the design as it went, all fixed with named tests
in `main/storage.test.ts`: a replacement that could not replay a slot reused
its numbers; a slot replayed by a second replacement was judged by the first
replay's id; a replacement replaced the journal it read from disk with
main's older one, dropping an entry the stopped worker had written; and a
failed journal replay closed the journal only in main, so an entry already
on its way was accepted. Not covered: power loss (the model's fsync does
nothing), more than two faults, larger scripts with the newer fault kinds,
and the window's own rules (when it checkpoints, keeps its copy, routes a
rename, or reports), which the crash and behavior tests cover. Because the
owner is fixed, the model also does not exercise an ownership transfer
(the tab moving to another window while a slot is in flight), a gap with
no owner, an unstamped (main-internal) writer, the owner check made when a
slot lands, or deletes; named tests in `main/storage.test.ts` and the
review's hand probes (recheck 6, R6b to R6h) cover those.

The dirty check's fingerprints (twice per autosave and per open) are now
serialized in slices and hashed on a worker (`project-fingerprint.ts`, the
same SHA-256 as before). The unsaved baseline reuses the checkpoint's text
from the same task.

**Crash safety.** The bar was that a crash at any moment recovers at least
what the previous code did. Two tests prove it:

- `legacy/__tests__/project-checkpoint-crash.test.ts` runs the real window
  store, project registry and main store engine through an open's checkpoint
  sequence. That sequence includes a raw unannounced write, a history edit,
  and an announced edit during a check. The directory is rebuilt as it stood
  after every prefix of the logged file operations, which is every possible
  crash point. Each recovers nothing or a complete state the document had,
  never an older state than an earlier point. Every settled point recovers
  exactly what the previous code recovered there. Slot writes: 1 for the open
  (was 5), and 3 more only where the document really changed. It recovers
  through a fresh window's Projects.get. It also covers, end to end: a
  failing slot rename and an ENOSPC slot write (the edits survive every crash
  point, and the next autosave writes the slot and lets flush succeed); edits
  made while every slot write fails (each on disk once settled); a checkpoint
  refused while the one before it is still renaming; two slot writes in
  flight with the older failing, the newer landing later or failing too
  (review recheck 2); an entry refused at the 32 MiB limit with slot writes
  refused (review recheck 3); a raw write after a journaled edit to the same
  path; and one toast per failure episode under a disk that stays full.
  `main/storage.test.ts` covers the same cases at main's store, a
  replacement worker replaying a slot and its journal in either order and
  keeping a closed journal closed, a replay it cannot apply, the barrier's
  order, and a bare slot.
- `e2e/project-open-crash-recovery.spec.ts` SIGKILLs the whole app at 15
  seeded moments, from 0.2 s into a large open to after it settles, with
  either setting. Every relaunch recovered nothing (killed before the first
  checkpoint) or exactly the document a clean open stores, and no store file
  was quarantined. The first checkpoint is durable about 2 s into the open.
  The previous code made its first slot write durable only after its 75 s
  bridge copy.
  One review run failed (kill 3, 2067 ms, setting off). The two 57 MB
  documents differed in exactly one value, a keyframe id. The fixture built
  252,000 keyframes with PM.uid ids (`k` plus 7 random base-36 characters),
  which collide about one run in three, and every open gives the second of a
  duplicate pair a fresh random id (app.ts, sanitizeProp). The clean open
  and the killed open each stored, and recovered, their own renaming. A
  fixture with one forced duplicate reproduced it on every recovering kill,
  settled opens included. The fixture now has fixed, unique ids, and a
  mismatch prints the paths that differ. At 13ff12c23, 11 seeds
  (`POWERMOVE_CRASH_SEED` 1 to 11) gave 165 kills: 110 with the setting off
  and 55 with it on, from 40 ms to 5.9 s into the open plus 22 after it
  settled. 86 recovered the document (62 off, 24 on), 79 recovered nothing,
  and none recovered another document or quarantined a file. After the
  recheck fixes (failed-checkpoint semantics, deterministic keyframe ids),
  5 more seeds (12 to 16) at 037b44cf7 gave 75 kills (50 off,
  25 on, 10 settled), 36 recovering the document, all
  clean. With SAVE-1, 5 more seeds (17 to 21) at 912a22c58 gave 75 kills
  (50 off, 25 on, 10 settled), 30 recovering the document, all clean
  (315 in all since the fixture fix). With the recheck-3 changes (closed
  journal, the barrier), seeds 22 to 26 at 2fff8632d gave 75 more (50 off,
  25 on, 10 settled), 30 recovering the document, all clean: 390 in all. With SAVE-1b, seeds 27 to 31 at 378c574bc
  gave 75 more (50 off, 25 on, 10 settled), 34 recovering the document,
  all clean: 465 in all. With SAVE-1c, seeds 32 to 36 at 4a42a7bfb gave
  75 more (50 off, 25 on, 10 settled), 43 recovering the document, all
  clean: 540 in all.
  The e2e kills the whole app with no
  slot write failing, so the failure cases are the model check's and the
  crash tests'. Opening now
  replaces a repeated or missing keyframe id deterministically (`kA.2`,
  `k<index>`), so a document with real duplicates also opens to the same
  document every time.

Preparation (engine on) also lost two of its three whole-document passes:

- `JsonBudget.due()` read the clock on every value: half of the CPU.
- The renderer integrity digest is no longer computed before publication.
  Main hashes the ingested stream, and the utility derives the canonical hash
  of the tree it parsed, so the tree is exactly the shadow. The first
  consistency check after publication fills the digests and compares them.
- The shadow is cloned by `jsonCloneSliced` (jsonCloneAsync's copy, about 4
  times faster).
- The shadow is ingested as its `JSON.stringify` text, not canonical JSON.
  The utility loads it with `source: "text"`: it checks the bytes against
  main's hash of the stream and answers the canonical hash of the parsed
  tree. Recovery copies keep the byte hash (document-recovery.md).

The font prewarm yields every 8 ms. A simulated memory-pressure
notification to the window after a large ingestion was tried and removed:
it fired before the RSS peak, did not lower the measured RSS, and could
drop Blink caches and pause the live editor.

Paired medians, on the final build. There were three runs per mode and
project, interleaved, at a load average of 10 to 22 with other lanes running,
so this was not a quiet window. An earlier batch, before the two font-prewarm
changes, is in brackets (load 14 to 40). In that batch the PMV4 engine open
was 30.7 s and today's was 31.6 s.

| Median | PMV4 today | PMV4 engine | PMV3 today | PMV3 engine |
|---|---|---|---|---|
| Open (s) | 22.4 [31.6] (was 372) | 30.9 [30.7] (was 419) | 8.0 [9.5] (was 120) | 13.9 [14.7] (was 135) |
| Renderer RSS (MiB) | 1,899 | 2,383 | 1,369 | 1,358 |
| Total RSS (MiB) | 2,928 | 4,393 | 2,501 | 3,154 |
| Renderer footprint (MiB) | 1,428 | 1,668 | 1,004 | 1,013 |
| Total footprint (MiB) | 2,467 | 3,755 | 2,645 | 4,019 |
| Engine processes' footprint (MiB) | n/a | 1,050 | n/a | 1,079 |
| Longest main-thread task in the open (s) | 4.0 | 2.8 | 2.5 | 2.5 |
| Tasks over 100 ms in the open | 4 [48] | 3 [4] | 5 | 8 |
| Blank opens | 0 of 3 | 0 of 3 | 0 of 3 | 0 of 3 |

PMV4 preparation in the final batch took 5.3 to 7.9 s. Of that, 0.65 to
0.8 s was the clone, and the rest was the text stream plus the utility's
load: reading 180 MB, its SHA-256, the parse, the canonical hash, and the
integrity digest.

What remains:

- **Memory.** The utility holds about 1 GB of footprint: the native tree, its
  allocator, and 0.4 to 1 GB GPU-owned. The renderer holds the shadow (+142
  MiB of heap after GC on PMV4) plus freed pages. A 16-byte node is not
  possible inside pm-model's `forbid(unsafe_code)` with inline `Arc`
  children. It needs either an arena of containers addressed by index, or
  revisions held by containers rather than by every node.
- **PMV3 open.** The engine's tail asks the utility synchronous font and
  raster questions while the open finishes.
- **PMV4 tasks.** In the earlier batch, today's 48 tasks over 100 ms were
  mostly its legacy viewer drawing 258 layers once the window stayed
  responsive. The engine's tasks are the reply parse (0.6 s), switchProject
  (about 3 s, the same work as today's), and one or two of about 0.2 s.
