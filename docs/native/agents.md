# Rust agents (P4 package 16)

`apps/native/crates/agents` is `pm-agents`, a Linux-testable Rust service library.
Reference: tag `v1.2.0`, commit `07ed5fa17ee640659cb075f9cad1af2095011d60`.
This adds the provider protocol, worker, storage and workspace boundaries; it does
not claim that the native application is already wired to them or cutover-ready.
No Electron code is pruned: its consumers still exist.

## Architecture and entry points

- `transport::spawn(ProcessSpec, Machine, initial)` returns immediately with a
  handle. Spawn, pipe IO, timeouts and reaping run on workers. `Handle::send` uses
  `try_send`, returning a saturated action intact; `try_event` never waits. Queue
  limits are 128 actions, 256 events, 32 reader chunks and 32 outbound messages.
  A separate stdin writer keeps a stalled runtime from blocking cancellation.
  A saturated runtime/subscriber terminates explicitly rather than growing memory.
- `codex::Client` is a deterministic App Server state machine. It initializes with
  the shipped experimental question capability, sends `initialized`, starts an
  ephemeral read-only editor thread, starts/steers/interrupts turns, multiplexes
  request IDs and scopes notifications to thread/turn IDs. Explicit thread resume
  is separate from normal editor runs: the latter already carry bounded history.
  Client RPC timeouts are 30 seconds; turn timeouts are one hour.
- `claude::Client` handles stream-json, structured results, streamed-tool updates,
  stream/full-message deduplication, retry and compaction housekeeping, stdio
  `can_use_tool`, `AskUserQuestion`, control cancellation and held approvals.
  Replies use the original question text as the Claude answer key. Finishing or
  cancelling a run denies outstanding control requests and closes stdin.
- `codex_exec::{Options, Client}` retains the autonomous CLI route: native binary,
  permission profiles, supervised versus auto-review approvals, literal argument
  boundaries, image paths, provider session handles and display-field filtering
  of structured answers. `Machine::CodexExec` consumes the private result file
  after a successful child exit, using a bounded no-follow regular-file read.
  Failed resume is eligible for retry only before observed provider work; the
  application coordinator must decide and issue that retry.
- `approvals::Authority` requires fresh Computer consent. Claude Project runs ask
  under both approval modes because Claude has no automatic reviewer. Editor
  stdio permission requests are denied except user questions. An allow-for-run
  choice is in memory and expires with that run. Codex editor App Server threads
  remain `approvalPolicy: never`; unsupported server requests receive -32601,
  and MCP elicitation receives the shipped decline payload.
- `conversation` preserves the original brief and separate recent/decision
  context budgets, with durable message positions for explicit thread reads.
  Its excerpts and full-message text are compared against the unmodified frozen
  thread module; archive bytes are never replaced with a model summary.
- `persistence::Threads` reads/writes the exact logical store families
  `agentThreads.<project>` and `agentAttachment.<attachment>`. Its `Store` trait
  belongs on the kernel actor; no second library writer is created. Payloads
  publish before archive references, with immutable-ID conflict checks. Opaque
  fields, drafts, conversation, controls, provider/run/orchestration state,
  resume data and missing-payload markers survive. Unrecognized archives are
  refused unchanged. Thread removal archives the complete record. No GC runs.
- `workspace::Workspace<Host>` dispatches the seven compatible workspace tools:
  list/read/write, command start/status, compile and complete. It enforces the
  shipped limits (8 MiB reads, UTF-16 paging, 1,000,000-character writes, four
  active background jobs, ten-minute command ceiling). It stops jobs before
  validating/publishing completion. App-only context refuses timeline commands.
  Descriptor-relative file access rejects symlink escapes. File replacement
  atomically exchanges the displaced inode, then moves it into main-owned state;
  a durable path/staging receipt records crash recovery. An exclusive new-file
  rename never overwrites an arriving file. Archive and
  workspace must share a volume. A failed archive move leaves the original in
  workspace staging for recovery. Freshly consented Computer access permits
  explicit external paths. Symlink file paths are conservatively refused.
- `workspace::{session_path, read_session, write_session}` retains the shipped
  `Agent State/<safe-project>/threads/<thread>/session-v2[-provider]-<authority>.txt`
  spelling (ChatGPT omits the provider suffix). Replaced handles are archived.
- `runtime` installs reviewed exact npm packages, never a mutable `latest` tag.
  `bundled_pin` uses the public frozen `bun.lock`: Codex 0.159.1 and Claude
  2.1.280, both macOS architectures. The HTTPS adapter uses system curl, disables
  curl configuration, supplies no auth and streams downloads. SHA-512 SRI,
  metadata/version and tar header/path checks precede native executable probing.
  The native vendor tree retains code-mode/voice helpers, bundled shell, search
  executable and shared libraries; no npm, Node launcher or Chromium ships.
  A synced version tree is activated through an atomic symlink; discovery reads
  its embedded receipt, surviving a crash before legacy metadata publication.
  Previous versions and failed staging trees are retained. OAuth homes are not
  arguments to the updater. No runtime login/logout or credential copying occurs.

Each state machine can be driven without process IO, a screen, JS or a UI. The
worker handle exposes run IDs, questions, sessions, progress, notifications and
terminal results. Kernel/tool mutations must still go through the existing
kernel actor and Rust MCP host; provider traffic grants no direct document lease.

## Protocol generation and conformance evidence

`fixtures/protocol-ts` contains 32 dependent types produced by the pinned native
Codex 0.159.1 executable with `codex app-server generate-ts`. The root set covers
initialize, thread start/resume, turn start/steer/interrupt and user-input request/
response. `protocol-source.json` hashes each source. `generate-protocol.cjs`
translates those declarations, including literals/unions, into `generated.rs`.
`Field<Option<T>>` preserves missing versus explicit null; flattened future fields
survive round trips. These are the generated type closure used by this client,
not a claim to implement every App Server method. The wire client retains JSON
at its boundary for the shipped subset: for example, the tag omits text_elements
on text inputs although generated UserInput declares it. That omission is replayed
as shipped; no provider protocol is guessed from a newer SDK.

`capture-oracle.cjs` uses `git show v1.2.0` and TypeScript 5.9.3 to execute the
unmodified TypeScript client/parser methods with injected fake processes. It uses
synthetic requests from the shipped test patterns, the shipped Claude shell
fixture and the shipped Codex recorded NDJSON. No account, user profile, live
provider request or private project participates. Temporary App Server paths are
normalized to `/workspace/agent`; object property order is immaterial. The script
records inbound/outbound JSON, user control actions and expected trace/progress/
result events. A second fixture process replays the wire frames over real pipes.

| Transcript | Compared observations | Result |
|---|---|---|
| Codex happy | initialize/start, input, streamed answer deduplication, result | Identical |
| Codex question | held request/answers, ID collision, elicitation decline, unsupported request | Identical |
| Codex steering | exact `threadId`/`expectedTurnId`/input shape and final result | Identical |
| Codex cancel | exact interrupt and cancelled result | Identical |
| Codex compaction | compact start/end, reroute progress and final result | Identical |
| Codex async question | question card, hidden duplicate prose, final answer | Identical |
| Codex tool catalog | command/file/search/MCP/dynamic/media-subject/sleep/collaboration/image/error rows | Identical |
| Codex retry | warning/error deduplication, resumed progress and reasoning | Identical |
| Codex shipped exec NDJSON | progress, trace, structured-field filtering, duplicate session handling | Identical compared events |
| Claude stdio control | AskUserQuestion answer by original text, deny, unsupported control | Identical |
| Claude shipped stream | thinking/tool/text updates, full-event deduplication, structured output | Identical compared events |

Eleven transcript tests compare every recorded outbound value and every recorded
trace/progress/result in order, rejecting leftovers. Session/unknown-notification
inspection is additive and tested separately. Conformance is bounded to these
fixtures; it is not a live-runtime or whole-agent certification.

Intentional differences from shipped behavior are tested: retaining removed
threads; retaining unavailable payload markers; immutable attachment conflicts;
preserving unknown/malformed archives rather than filtering records away;
keeping old runtimes instead of pruning; reviewed pins instead of mutable npm
latest; refusing unsafe symlink file paths; atomic archive of replaced files;
discarding the entire oversized line; Unicode-safe excerpt boundaries; and exactly one terminal result when a
cancelled turn/start later times out or the process exits. None is described as
an identical observation outside the transcript set.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Preserve the two Codex routes and exact captured provider shapes | N3 authored intent; N4 retain provider workflows |
| Generated serde types retain omission/null and experimental fields | N3 correctness; N7 inspectable protocol boundaries |
| Independent state machines plus bounded process workers and stdin writer | N1 cancellation/backpressure; N7 headless; N8 main thread never waits on JS |
| No Node/Chromium dependency in the crate or installed runtime path | N8; npm metadata is only a distribution format for native executables |
| Keep context excerpts separate from lossless archives, retaining original briefs and user decisions | N3/N4 authored intent; no destructive model summary |
| Keep kernel persistence and mutation admission behind explicit host traits | Single-writer architecture; N3/N7; no bypass through agent code |
| Computer consent is fresh; approvals do not survive run end; Claude auto still asks | N3 shipped authority semantics; N4 no increased authority on reopen |
| Archive removed threads/replaced files/handles; immutable payloads; refuse malformed archive writes | N4 never lose user data; N3 retain exact authored and unknown data |
| Private result/session state remains outside an agent-writable workspace | N1/N3 main-owned authority and resume identity |
| Pinned npm SRI, native-only extraction, atomic activation, retain prior trees | N1/N2 robust update; N4 archive rather than delete; N8 |
| Do not prune Electron consumers or edit shared workspace files in this lane | Prune rule; explicit lane ownership |
| Do not reproduce oversized-line suffix parsing or duplicate cancellation results | Quirk policy and N3: no evidence of authored intent for either defect |

## Running checks

The owning lane cannot change `apps/native/Cargo.lock`. Its copy under `agents/`
adds only the `pm-agents` package entry; all existing resolutions are unchanged.
The pinned Cargo 1.96.1 supports alternate lockfiles behind a Cargo-only unstable
switch. `scripts/check.sh` sets that switch and runs the actual native workspace:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
crates/agents/scripts/check.sh
```

This runs `cargo test --locked -p pm-agents`, strict Clippy for all crate targets
and `cargo fmt -p pm-agents --check`, without changing the shared lockfile. The
switch does not enable experimental language features in shipped code. After the
workspace owner adds the new package entry to the shared lock, ordinary locked
commands need no switch. Python 3 is required only for fake-runtime pipe tests.
TypeScript/npm are required only to regenerate reference evidence, not to build
or run pm-agents. To regenerate, install TypeScript 5.9.3 and Codex 0.159.1 under
`work/oracle`, capture the pinned CLI's generate-ts output under `work/`, then run
`capture-oracle.cjs` and `generate-protocol.cjs`; all retained inputs are public
synthetic fixtures and frozen type declarations.

Verification on Linux: **87 Cargo tests pass** (11 transcript, 4 conversation, 10 persistence,
32 protocol, 12 runtime, 6 process transport, and 12 workspace). Strict Clippy (`--all-targets --no-deps -D warnings`),
crate formatting and fixture regeneration checks pass. No existing crate source
or shared lockfile is modified. All four pinned macOS Codex/Claude tarballs were also
SRI-verified and installed on Linux using a Mach-O-header-only injected probe;
44 Codex and 4 Claude public package headers per architecture are recorded in `runtime-layout.json`.
Native helpers, search, shell and voice paths survived extraction; no executable
was launched. `capture-runtime-layout.py` and `verify-runtime-archives` reproduce
this check.

## Integration and macOS gates still open

Changes requested outside this lane:

1. Workspace owner: add the 12-line `pm-agents` package entry from the lane
   lockfile to `apps/native/Cargo.lock`. The existing `crates/*` glob already
   discovers the crate. No shared dependency version changes are needed.
2. Kernel/application owner: implement `Store` on the actor's logical library
   read/write path; serialize archive ownership and drive durable session events.
   Wire loaded threads, orphan attachments, interrupted runs and archived thread
   views into the native application. No direct filesystem library writer is
   supplied here.
3. Rust MCP/workspace host: implement `Host` with macOS OS-enforced Project command
   confinement, process-family cancellation/draining (including escaped process
   groups), compiler/API-pack staging, extension publication/change-history,
   artifact import and their existing kernel transactions. Linux tests inject a
   deterministic Host; this crate never substitutes an unrestricted Project shell.
   The provider process transport kills its own process group, not arbitrary
   escaped descendants; whole-family draining remains this host's responsibility.
4. Shell/application: launch workers with discovered bundled/updated native
   binaries and the existing isolated runtime homes/user-resource configuration;
   drive approvals/questions and explicitly consented Computer runs; route session
   updates, notifications, runtime update progress and resume failures. Normal
   cancellation should complete before dropping the worker handle.
5. Compatible HTTP provider owner: attach the workspace tool dispatcher to the
   native compatible-provider streaming client and native Keychain secret service.
   No OAuth/provider secret store is read or migrated by this crate.

On macOS verify both architectures' Mach-O package layout, signatures and
hardened runtime launch; signed isolated homes and preserved runtime-owned auth;
real pinned App Server and Claude stream/control protocol and automatic
compaction; `/usr/bin/curl` under app distribution/network conditions; sandboxed
commands and full descendant draining; `renameatx_np` swap/exclusive behavior,
same-volume archive recovery and durability under process kill; kernel lock,
attachment/media closure and rollback; and shell progress/question/approval UX.
No GPUI, AppKit, WebKit, Metal, Keychain or real account was exercised on Linux.
