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
| Job engine | same entry, `POWERMOVE_ENGINE_PROCESS=job` | Exports, agent snapshots, job snapshot sessions; utility QoS |

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
- The job engine runs at utility QoS (`pthread_set_qos_class_self_np`),
  never in the Darwin background state. macOS threads do not inherit
  their creator's QoS, so every engine thread starts through
  `pm_engine::qos::spawn_engine_thread`, which applies the class first.
  Threads Chromium starts in the utility process (IO, IPC) keep their own
  classes; they only move messages.
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

## Large projects, rendering (bigrender, 2026-10-22)

The two private production projects (PMV3: 867 layers, a raster wrapper
chain from the user's extensions, 75 effects; PMV4: 258 layers baked to a
key per frame, image-heavy) played one to thirty frames in eight seconds
with the engine on while today played 135 to 230. Profiled with `sample`
on the engine and the GPU process, V8 CPU profiles of the renderer, and a
per-frame stage breakdown the presenter now reports (`stages`: plan,
prefetch, record, submit, planning passes that waited and their time;
`memory`: the viewer renderer's caches; `definitionGpuMs`: the longest GPU
time per effect, transition and layer definition). The causes, in the
order they cost:

| Cause | Where | Fix |
|---|---|---|
| Every raster-override need waited for its base pixels, prepared two at a time over async IPC, while playing; the chain passed nearly all of them through | `kernel/raster-compat.ts`, `engine-connection/render-transport.ts` | The producer remembers which layers' chains read the base (`ready`); the rest are answered in the turn, the lookahead answers ready needs within its time budget (only preparations are capped) and only for layers in their frame (`active`) |
| A frame waiting on answers was planned again per answer and, while playing, dropped and the next frame tried at once (5,000 plans to show 46 frames) | `present/render.rs` | The frame keeps its playback slot and is planned again once every need has an answer at its revision (`CompatStore::all_answered`), or after 250 ms; a paused one waits the same way instead of spinning |
| A wrapper that drops the preview crop (typography anchoring over every layer) had each layer's whole raster drawn on the CPU, read back and posted every frame | `raster-compat.ts` | An untouched base of the whole layer is answered native (the engine's native draw is the whole layer) |
| Base rasters a wrapper reads were a synchronous raster round trip every frame | `raster-compat.ts`, `kernel/install.ts` | Today's raster cache: base surfaces kept by evaluated content and density (text and plain shapes, never animated characters, styles, typography anchoring or paths), within today's 192 MiB and 4,096 entries; surfaces carry a key as today's do |
| A wrapper result with an unchanged key and quad was read back and posted every frame | `raster-compat.ts`, `present/compat.rs` | `reuse` across frames when the key and the quad hold; the store resolves it to the pixels the layer posted under that key |
| Every channel read prepared its whole keyframe track again (adoption, auto-bezier) | `pm-eval` | Prepared tracks kept per `kf` array (immutable while shared), within 64 MiB (`track_cache.rs`), as today's `temporalEvaluate` caches per array |
| A user effect indexing a 1,104-entry glyph table took 1.47 s of GPU per frame: glslang copies a run-time indexed `const` array into each fragment and naga's MSL kept the copy | `pm-glsl` | `const_tables.rs`: such tables are read from a program-scope `constant` array (1.36 ms per frame after) |
| Level 1 shape and path rasters (the exact CPU emulation of Chromium's canvas) drew one after another | `vector/cache.rs`, `renderer.rs` | A frame's uncached rasters are drawn on worker threads before it records, under the same keys |
| Exports asked the window for each wrapped layer at each frame, one blocking round trip each | `jobs/compat.rs` | One batch per frame (`CompatRasters::fetch`), carrying the same layers at the next four frames |
| Window surfaces a busy page never took stayed undrawn: a timed-out transfer takes its import with it | `main/engine/bridge.ts` | Each send after a failure imports the IOSurface again, and an undelivered surface is offered again every 2 s while it lives |
| Idle intermediate targets of sizes no frame used again (animated scales) piled up | `pm-gpu/src/target.rs` | After each frame, idle targets the frame did not use are freed down to 64 MiB; the compatibility store is bounded at today's 192 MiB raster budget |

The custom definition reported as failing to compile on these projects
(a user effect with a `uniform bool` toggle compared with `> 0.5`) fails the
same way in WebGL in today's renderer (captured: the same message; 41
custom definitions of the setup, this one the only failure, and neither
project uses it). `pm-glsl/tests/uniform_types.rs` holds WebGL's verdicts
for 35 bool, int, uint and float uniform forms; the engine agrees on all.

The public stand-in is `e2e/engine-large-project-render.spec.ts` (288
layers baked to a key per frame, today's three wrapper patterns, today and
the engine on fresh profiles in one run): presented frames in four seconds
at least today's within one per second, and scrub p95 within a fifth and
one frame of today's.

What remains, measured on the private projects, is in the lane report
(`/tmp/ne-w5/reports/bigrender.json`, outside the repository) and below:

- Neither engine plays PMV4 in real time: in eight seconds today's
  playhead covers 2.8 s and presents 232 frames, the engine's covers 1.7 s
  and presents 110. Its only audio clip is offline, and the engine's
  output underran on nearly every device callback (72,089 of 73,350) while
  it played; the audio lanes (`pm-audio` playback, `present/audio.rs`) own
  why an all-offline mix underruns.
- A fresh profile's first play waits for first-use work: the ANGLE parity
  probe of each user effect (the frame waits for WebGL's reference) and
  each program's Metal compile, 1.5 to 3.5 s on the 867-layer project.
- A frame of PMV3 holds 300 to 360 MiB of intermediate targets: every
  layer's targets are returned to the pool at the end of the frame, not
  when the layer has been composited.
- PMV4 frames at 0 s and 3.2 s differ from today's by up to 6 CV: the
  glow of a group with two of the user's effects, a glow and a
  blur (effects translation), and the edges of a DM Sans text
  layer with typography anchoring (`pm-text` level 1 raster and the
  typography anchor offset).

## Resync

`resync` with `canonicalJsonV1` installs a `pm-model` mirror after checking
the canonical hash, inline or from a cache file the engine resolves with
main; `checkConsistency` compares hashes at one revision. Patch batches
follow the applied revision and apply through a pluggable RFC 05 decoder;
until the model lane's binary decoder is installed, a batch is reported as
`unsupported` once per session and the mirror keeps the resynced revision.

Known limit: a resync from a staged file is read (`std::fs::read`, up to
512 MiB) and parsed on the utility process's JavaScript thread, as an
inline resync is parsed there; while it runs that engine's ticks and port
traffic wait. Large documents do take this path now: for C1's 170 MB
document (release build, M4 Max) the parse is 0.76 s, building the tree
0.49 s (0.24 s reconciled against the session's previous tree), and the
hash check 0.53 s, so the engine's JavaScript thread (and the frames and
clock samples it posts) waits about 1.8 s per resync. A resync to a hash
another session was just checked at skips all three. Still open: move the
read, parse, and hash check to an engine thread (`spawn_engine_thread`),
with the mirror installed when the thread finishes.

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
