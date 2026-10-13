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
- There is no Settings row yet; the switch is for development and the
  Phase 2 gate.

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
  at most 256 MiB, 16 per launch) live there and go with their launch.
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
  never in the Darwin background state.
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

## Resync

`resync` with `canonicalJsonV1` installs a `pm-model` mirror after checking
the canonical hash, inline or from a cache file the engine resolves with
main; `checkConsistency` compares hashes at one revision. Patch batches
follow the applied revision and apply through a pluggable RFC 05 decoder;
until the model lane's binary decoder is installed, a batch is reported as
`unsupported` once per session and the mirror keeps the resynced revision.

## Packaging and signing

- `bun run build:engine` builds `pm-node` with the pinned toolchain into
  `apps/desktop/native/pm_node.node` (ignored by Git) and checks its
  protocol build.
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

Measured on the reference machine (M4 Max, shared with other lanes), 20
consecutive runs of `bun run test:engine-host`, 20 passed: both engines
start and negotiate in 75 to 86 ms; a SIGKILLed engine is back,
renegotiated, and resynced by the client in 313 to 337 ms, of which 250 ms
is the first backoff step. Before the control port, Electron main died
with SIGSEGV during `stop()` in about a third of runs.
