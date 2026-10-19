# RFC 07: Errors and isolation

Status: draft, version 0. Module: `pm_protocol::error`. Gates Phase 2;
freezes when Phase 2 starts. Owner: Engine and platform (section 27).

## Purpose

Every failure names what failed (scope), why (code), and whether trying
again can help (retry), and the scope decides the isolation rule (plan
sections 8 and 19). Per-layer failure isolation, GPU loss recovery without
replacing the project, and crash attribution are inherited constraints
(section 3). Errors answer requests (`Message::Error`); isolated failures
during work that continues are events (`Event::Failure`); engine crashes are
reported by the host (`Event::Crash`).

## Types

| Type | Meaning |
|---|---|
| `EngineError` | `code`, `scope`, one-sentence `message`, `retry`, optional `detail` |
| `ErrorCode` | Invalid message, unsupported version, unknown session, stale revision, capability rejected, definition missing, shader compile, watchdog timeout, decode failed, media offline, budget exceeded, device lost, Blender missing, Blender failed, not headless-renderable, export failed, cancelled, timeout, unsupported (capability routing), crash attributed, internal |
| `ErrorScope` | Request, layer, effect (with layer and definition), definition, asset, job, session, connection |
| `Isolation` | What the engine does for each scope (below) |
| `Retry` | Never, after N ms, after a resync, after a user action |
| `ErrorDetail` | Shader diagnostics with user line numbers, stale revision (applied and wanted), unsupported version (local and remote versions), missing surface count, offline assets |
| `FailureReport` | An isolated failure at a revision; fills `uiState.getShaderMeta` and the status bar at the frame's revision. For a definition-scoped compile failure (`shaderCompile`, `notHeadlessRenderable` with `ErrorScope::Definition`) `revision` carries the definitions generation the program was translated at instead of a document revision (pm-node `present/render.rs`); the kernel's `EngineShaderMeta` ignores one older than its stream's latest push of that definition. Draft overload, to become its own field at the freeze |
| `EngineProcess` | Interactive or job |
| `CrashReport` | Which process, consecutive attempt, exit, suspects in flight, items now marked failed, restart delay |
| `CrashLedger` | Host-side attribution |
| `InFlight` | `Event::InFlight` (engine → host): the attributable items the engine is working on, sent whenever the set changes; the host keeps the last one per process as the crash's suspects |
| `restart_backoff_ms` | 250 ms doubling per attempt, capped at 30 s |

## Invariants

1. **Scope decides isolation** (`ErrorScope::isolation`, tested):

   | Scope | Isolation |
   |---|---|
   | Request | That request fails; nothing else is affected |
   | Layer | The layer outputs transparency (section 8, isolation) |
   | Effect | The effect is skipped with its data kept |
   | Definition | The definition fails everywhere it is used; compile failures stay per definition (Store GLSL is untrusted) |
   | Asset | The asset is marked failed (decode failure or offline) |
   | Job | That job fails; other jobs continue (the render queue no longer stops at the first failure, appendix A item 6) |
   | Session | The session restarts; renderers re-send snapshots (device loss, crash) |
   | Connection | The connection closes (bad bytes, version mismatch) |

2. **Missing definitions behave as today** (section 11): a missing effect is
   skipped with its data kept; a missing extension layer shows the
   checkerboard placeholder. `DefinitionMissing` is reported, not fatal,
   except in `pm-cli` for a definition with no snapshot that is not already
   marked missing.
3. **Watchdog:** an effect or shader over its GPU timestamp budget is
   disabled from the next frame and reported (`WatchdogTimeout`, effect or
   definition scope). A true hang is device loss (`DeviceLost`, session
   scope): the device is rebuilt and rendering resumes without replacing the
   document.
4. **Crash attribution:** an asset, effect, definition, or layer suspected
   in two crashes is marked failed for the session (`CrashLedger`, tested:
   reported once, on the second crash; sessions and connections are never
   attributable, and `InFlight::validate` rejects them as suspects).
   Restarts back off exponentially (`restart_backoff_ms`, tested); the host
   resets the attempt count after the engine has stayed up for a healthy
   interval (60 s in main and `pm-host`). After a restarted engine's
   handshake the host sends it `Event::Crash` with `marked_failed` listing
   every item failed so far in the session, and the engine refuses work on
   those items with `CrashAttributed` (item scope) instead of crashing on
   them again.
5. **Messages are for people.** One sentence, 1 to 4096 bytes, saying what
   failed and what to do; no paths the engine was not given, no secrets.
   Length is checked by `EngineError::validate`.
6. **Codec and negotiation failures** convert to `InvalidMessage` and
   `UnsupportedVersion` with connection scope (`From<CodecError>`,
   `From<NegotiationError>`).

## Versioning

Version 0 until Phase 2 starts. New codes and details are new variants;
after the freeze each is a version bump. Consumers must treat an error as
failed even when they do not recognize its detail, but an unrecognized
variant does not decode, which is why codes are part of negotiation.

## What Phase 0 must confirm

- **S1 and S8:** that GPU device loss on Metal is observable and that the
  rebuild path keeps sessions (the session scope's "re-send snapshots" is
  the fallback, not the norm).
- **S3 (shaders):** that every compile and translation error maps to user
  line numbers (`ShaderDiagnostic.line`, after subtracting the wrapper's
  `body_line_offset` from contract 10).
- **Phase 0 macOS sandbox evaluation for engine processes:** whether a
  sandbox violation needs its own code. Currently it would surface as
  `Internal` or `DecodeFailed`.

## Open questions

1. Whether crash attribution should persist across app relaunches (today
   the ledger is per session, as section 19 says "for the session").
2. Whether `Retry::After` needs a jitter rule for agents that retry in a
   loop. Agents read `get_render_status` instead of retrying exports, so
   this matters only for frames.
3. Localization of `message`: the app is English-only today; codes allow the
   UI to substitute its own copy later.
