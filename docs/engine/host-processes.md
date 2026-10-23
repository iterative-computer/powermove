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

What remained after round 1 (PMV4 frame residuals at 0 s and 3.2 s, up
to 6 CV: a group's glow and blur from the user's effects, owned by effects
translation; a DM Sans text layer's edges with typography anchoring,
owned by `pm-text`) is unchanged. The round 2 changes and what remains
are below.

### Round 2 (2026-10-22)

What changed, by cause, measured on the private projects with the stage
breakdown (now with each frame's program build and probe time, the
longest frame's stages, and the slowest shape raster drawn ahead and its
size):

| Cause | Where | Fix |
|---|---|---|
| A fresh profile's first plays waited for each unknown user program's ANGLE probe and Metal compile | `effects::parity`, `effects::prewarm`, `present/render.rs` | Decision PROBE-1 (`effects.md`): viewer frames present natively and probe in the background, and programs are built on a worker at open and on definition changes. An idle sweep probes the document's other programs. Exports and snapshots still hold |
| The engine's own level 1 shape rasters ignored today's preview clipping: an 8000 px background was a 64 Mpx canvas emulated on the CPU every frame (it cannot fit the 192 MiB raster budget) | `plan.rs`, `vector/mod.rs`, `pm_vector::canvas2d::shape` | Today's decision for the engine's draw: a shape the viewport misses is not drawn, and a window is rasterized as today's crop canvas (`rasterShape(d, scale, crop)`) with today's texture coordinates. Inside a plain fill, the window over the viewport is drawn. Exports and snapshots clip the engine's own shapes and paths to their output (a producer still never sees a crop there) |
| Paths layers (outside today's clipping) had the same 8000 px canvases | `compat_crop::paths_plan`, `raster_paths_window` | The window of the bitmap the frame samples, with today's margin and bucket steps; the frame matches the whole bitmap's at max 0 |
| The exact canvas emulation visited each triangle's bounding box and blended every draw over the whole canvas | `pm_vector::canvas2d` (`msaa`, `analytic`, `context`) | Spans between the exact edge thresholds, analytic coverage stored for the mesh only, draws bounded to the pixels they reach, `unorm8` rounding in `f32`, a reciprocal table, paint once per pixel. Bit for bit against the kept references, and the canvas2d gate ran in 28 s instead of 83 s |
| Every layer's intermediate targets stayed out of the pool until the frame ended | `exec.rs`, `composite/exec.rs`, `pm_gpu::target` | Given back where a pass's draws have been recorded into its accumulator. Queue writes take only targets no command of the frame reads, and nothing is given back while the depth and id pass reads layer images. A 14-layer effect frame has less than half the peak, with the same pixels. `pooledPeak` reports the frame's peak |
| An export waited for a round trip through main to the window every fifth frame | `jobs/compat.rs` | A helper thread keeps the next 12 frames of the last frame's layers asked for (turns of 8, within main's 32), holding at most 384 MiB of answers |
| A playing frame waiting for raster-override answers was followed by frames waiting for their own | `present/render.rs` (`Renderer::compat_needs`) | While a frame waits, the next three frames the schedule will draw (at its stride) are planned and asked for in the same turn |
| The window skipped every need more than one frame behind its clock, and on a large project its turns ran longer than a frame | `render-transport.ts` | Needs up to 12 frames behind are answered |
| After a few heavy frames the schedule's cost estimate made every later frame look unreachable, so nothing was drawn | `present/schedule.rs` | After 250 ms without a rendered frame, a late frame still on display is drawn |
| With the engine drawing, the window's frame loop moved the playhead on every 60 Hz tick (timeline, panels and extensions followed), and autosave writes (4 s per 8 s of playback, against 1 s today) ran during playback | `legacy/core/engine.ts`, `legacy/app.ts` | The playhead moves once per project frame. An autosave write waits for the engine's playback to stop, for at most 15 s |

Measured after round 2, paired back to back on fresh profiles with the
user's extensions (release build, load average 12 to 28 from other lanes;
medians of two pairs, indicative until a quiet window):

| | today | engine | round 1 engine |
|---|---|---|---|
| PMV3 playback, frames in 8 s | 114 | 19 | 23 |
| PMV3 scrub p95 | 123 ms | 522 ms | 307 ms |
| PMV3 export, MP4 half size | 152 s | 287 s | 378 s |
| PMV4 playback, frames in 8 s | 175 | 88 | 86.5 |
| PMV4 scrub p95 | 93 ms | 131 ms | 107.5 ms |
| PMV3 first play: first moving frame, frames in 2 s | 63 ms, 32 | 159 ms, 19 | (6.4 s stalls) |

The engine's own frame cost on PMV3 fell from 261 ms to 34 to 46 ms per
playing frame and from 342 to 30 to 34 ms per scrub frame (prefetched
shape rasters 203 to 8 to 12 ms; program builds and probes 0 and 2 ms).
What remains:

- **The window's main thread.** Every engine frame of a project with a
  raster override needs window turns: its answers, then its
  presentation. On PMV3 the window is idle 0.2 s in 10 s of playback.
  The user's extension that draws a graph on every animation frame (with
  layout reads), the panels' rounded scroll and the mixer's edge
  measures take most of it, as they do today. Today draws each frame
  inside that animation frame, so it keeps 110 to 120 frames. With the
  engine, frames wait in its pool for the window to take them (pool
  waits), or wait for answers (after a slow frame the schedule now draws
  late rather than skip everything). The engine cannot fix this alone: it
  needs presentation and answers off the window's main thread, or the
  window's animation-frame work held to the presented frame rate (owners:
  bridge, kernel).
- **PMV4.** There is a render-thread turn of 0.4 to 2.6 s once per play,
  and about 415 waits for answers. Its only audio clip is offline, and
  the engine's audio underran on nearly every device callback (audio
  lanes).
- **Export.** PMV3 takes 287 s against 152 s. Answers come ahead in the
  background and the engine's own shapes are clipped. The rest is the
  job engine's per-frame render and readback (not profiled this round).
  The 74.7 MB file against 52.8 MB is the encoder settings (export
  encoding lane).
- **Memory (total RSS, all processes).** PMV3 is 4.7 to 5.2 GB against
  3.6 to 4.4 GB today. PMV4 is 8.6 to 10.4 GB against 4.4 to 5.5 GB. The
  window's JavaScript and wasm heap holds 2.6 to 4.1 GB against 1.9 GB
  today (the in-window document copy, bigopen; growth while scrubbing,
  open). The engine process holds 1.3 to 2.1 GB for PMV4, about 1 GB of
  it the document tree (bigopen). The frame pipeline is now small: PMV4's
  intermediate-target peak is 96 MB, vector rasters 16 MB, media 12 MB and
  frame surfaces 67 MB. With one shape-raster worker instead of four,
  engine RSS is the same (504 to 557 MiB against 467 to 548 MiB on PMV3),
  because clipped rasters are small. The four-worker cap stays.

### Round 3 (2026-10-22 to 23)

Round 2 left the window's main thread as the cause: every engine frame
needed turns of it (its raster-override answers, then its presentation),
and the window's own animation-frame work (an extension drawing a graph,
the panels' rounded scroll, the mixer) filled it. Round 3 keeps the
window's `requestAnimationFrame` callbacks at display rate, as today, and
takes the engine's per-frame work off that thread or down to one turn per
animation frame.

| Cause | Where | Fix |
|---|---|---|
| Each `needs` message was answered in its own task, and every answer was its own message (about 300 a frame on a large project); a 4 ms timer polled the lookahead | `render-transport.ts`, `pm-protocol` `CompatMessage::Answers` | While playing, needs wait for the window's next animation frame (where today's compositor ran the `raster` chain); that turn answers every need asked since the last one, each once, then the lookahead, and posts the pass-throughs and reuses as one `answers` message per session. Paused needs, seeks and Play still answer in their own task, batched |
| The lookahead followed at most 256 layers: on a large project a layer the engine asked for again came back with nothing sent and its whole horizon was answered again, every animation frame | `render-transport.ts` | 4096 layers, least recently asked first out. The scripted 300-layer playback posted 385 KB of answers a frame; now 24 KB |
| The engine asked again for needs the window's lookahead had already answered | `present/compat.rs` (`to_ask`) | Needs with a drawable answer (a pass-through, or a raster still held) are not asked. The public regression's engine asked 252 needs instead of 6,538 |
| A frame waited a turn of the window for its own answers | `present/render.rs` | The engine plans the frames it will draw next and asks for their needs ahead. It plans them only in the render thread's slack (the schedule waits for the next slot) and only as many as the slack holds at their measured cost: planning after every frame cost PMV3 15 to 80 ms a frame and made it skip frames |
| The presentation worker's frame releases and GPU completions crossed the window's main thread | `present-worker.ts`, `worker-presenter.ts`, `engine-process.ts`, `connection.ts` | The window offers the engine a release port over its own port; the worker hands frames back on it (its own envelope encoder, `release-envelope.ts`, byte-checked against the protocol's). The page keeps its last `VideoFrame` reference per slot (Electron's release callback must run on its thread) and closes it when the slot comes back. GPU completions cross only while a reveal waits on them |
| Frames the engine drew a frame or two ahead arrived after their interval when the window's main thread was late handing them over, and were dropped | `jitter-buffer.ts`, `presenter.ts` | The newest such frame, newer than the one on screen and at most 250 ms late, is drawn when nothing on time is there, as today's frame loop draws the playhead's frame as late as the page's work before it (`shownLate`) |
| New and animated shapes were rasterized (the exact CPU emulation of today's canvas) on the playing frame's own turn: up to 400 ms a frame on PMV3 | `pm-render` `vector/cache.rs` (`prefetch_ahead`), `Renderer::plan_ahead` | The plan of a frame the view will draw next has its level 1 shape and path rasters drawn on threads of their own; the frame takes them, or waits for one still being drawn. Same rasterizer, source and key: byte for byte the frame that drew its own (`rasters_drawn_ahead_of_a_frame_are_taken_by_it_and_draw_the_same_pixels`). At most 128 MiB held; what is not taken within 32 frames, or a seek leaves, is dropped |
| An export frame asked the window for each wrapped layer in its own IPC round trip (762,081 asks for one PMV3 export), and the background fetch ran in turns of 8 | `compat-frames.ts`, `preload/index.ts`, `jobs/compat.rs` | Main posts the asks it has at once to each window in one message, the window answers them in one task and returns the answers together; up to 128 in flight, the background fetch in turns of 96 |
| An autosave write whose debounce ended during playback wrote at once if the edit was more than 15 s old, and the idle consistency check (a hash of the whole document) ran during playback at its idle timeout | `legacy/app.ts`, `engine-doc/install.ts` | The hold counts from when the write first met playback; the idle check waits for playback to stop. Saves and exports still check at once |

Counted, not timed: `PM.EngineCounters` (`main-thread-counters.ts`) counts
the engine path's entries into the window's main thread by kind (port
messages, turns, frame events, export asks), messages and bytes each way,
and frames presented; the viewer's stats count the presentation worker's
messages and late frames; the engine counts `compatKeptAhead`,
`rastersAhead`, `rastersAheadTaken` and `rastersAheadWaited`.
`main-thread-budget.test.ts` scripts a 300-layer pass-through playback over
the real wire and holds its per-frame counts to
`main-thread-budget.baseline.json` (2.17 main-thread entries, 0.98 messages
and 24 KB to the engine, one turn a frame; before round 3, about 300
messages a frame and a polling timer), lowered with `RATCHET_UPDATE=1`. The
format moves to the perfbench lane's ratchet when it lands. In the engine,
`rasters_drawn_ahead_of_a_frame_are_taken_by_it_and_draw_the_same_pixels`
and `child_warm` hold the rasters drawn ahead to being taken (every one,
and 38 of 42 in the warm-up's play), and `child_compat_ahead` holds a
playing view's frames to at most one wait for answers in four frames.

Measured on the private projects, today and the engine back to back on
fresh profiles with the user's 39 extensions (release build; load average
10 to 30 from other lanes; medians of two pairs; each run plays twice, cold
then warm). "Frames" are `preview:presented` events, which both today and
the engine emit at most once per page animation frame; "drawn" is what the
presentation worker put on screen, which is not tied to the page's frames.

| | today | engine, round 2 | engine, round 3 (answers, presentation) | engine, round 3 (+ rasters ahead, slack planning) |
|---|---|---|---|---|
| PMV3 frames in 8 s, warm play (drawn) | 131 to 136 | 19 to 44 | 72 (75) | 145.5 (155.5) |
| PMV3 frames in 8 s, first play (drawn) | 83 to 144 | | 34.5 (36.5) | 102.5 (112.5) |
| PMV3 first moving frame | 82 to 98 ms | 159 ms | 499 ms | 138 ms |
| PMV3 scrub p95, warm / cold | 84 to 134 / 100 ms | 522 ms | 131 / 489 ms | 123 / 362 ms |
| PMV3 scrub p50, warm / cold | 43 / 38 ms | | 75 / 96 ms | 52 / 89 ms |
| PMV3 export, MP4 half size | 104 s | 287 s | 227 s (one run) | 168 s |
| PMV4 frames in 8 s, warm play (drawn) | 189 to 211 | 88 | 166.5 (200) | 175 (204) |
| PMV4 frames in 8 s, first play (drawn) | 191 to 196 | | 107 (127) | 105 (145) |
| PMV4 scrub p95, warm / cold | 44 to 51 / 63 ms | 131 ms | 70 / 165 ms | 63 / 168 ms |
| PMV4 scrub p50, warm / cold | 44 / 36 ms | | 45 / 65 ms | 34 / 46 ms |

The window's main thread, in the CPU profiles of the same runs (V8
sampling over the 10 to 13 s of a play): the engine path (the engine
connection, the presentation's page side, the transport and the `raster`
override's producer, which runs the user's wrapper chain as today's
compositor did) took 58 ms per presented frame on PMV3 before round 3 and
5.4 ms after, against 22 ms for today's frame draw. On PMV4 it is 1.5 s per
play against today's 1.6 s (19 ms per presented frame, 14.6 per frame drawn,
against 7.6): most of it is the user's wrapper chain, as in today's draw,
and the rest is base rasters fetched synchronously and wrapper results read
back for posting. The rest of the window's time is the page's own work,
the same as today's (an extension's graph, the panels' scroll edges).

Memory: the window's JavaScript heap no longer grows while scrubbing (PMV4,
garbage collected: 1,080 MiB before and 1,081 after 48 seeks, against
today's 1,142 and 1,144). The pm-wasm document mirror holds 1.24 GB of
wasm memory for PMV4 throughout (bigopen), and total RSS stays above
today's (PMV4 idle 10.2 GB against 6.0 GB: the window 6.1 GB against 3.4,
the engine process 1.3 GB).

What remains:

- **First play and first scrub.** The first play after an open meets every
  text layout and shape raster cold: the engine emulates Chromium's canvas
  on the CPU exactly, where today's Chromium draws on the GPU. A paused,
  idle view now plans the 48 frames after its playhead and draws their
  rasters ahead (no media decoded, no frame drawn; measured below once the
  last batch lands). A seek to a place not yet visited still pays them on
  its turn (PMV3 cold scrub p95 362 ms against 100).
- **PMV4's first play.** The engine waits for the window's answers on most
  frames (376 waits in 155 frames): the user's wrapper reads base pixels the
  window fetches from the engine, and the window answers once per page
  animation frame. Today runs that chain inside its draw.
- **Export.** PMV3 takes 168 s against 104 s: 62% of the export thread
  waits for the window's answers (`JobCompat::fetch`), now batched but still
  one window round trip per frame the background fetch did not reach.
- **The window's own work** (extensions drawing every animation frame,
  layout reads) is the same as today's and is not the engine's to change
  (decision: the window's `requestAnimationFrame` stays at display rate).
- **Memory** above today on PMV4 (bigopen: the in-window document mirror,
  the engine's document tree).

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
