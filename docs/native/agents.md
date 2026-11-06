# Rust agents

## Current state (2026-11-06)

A panel run is real, from keystroke to durable reply:

1. `panels/agent/view.tsx` sends composer keys as `{type:"key"}`. The Rust panel
   edits the draft. The v4 host reports keys, not composed text, so IME and
   shifted punctuation need a real text input there. Actions name their own
   target, so a streaming run's publications don't refuse them. Only `draft`, which
   replaces the whole text, needs the latest publication.
2. `panel.agent.dispatch` in `builtins/src/models.rs` admits the action
   (`agent_host::admit`) before touching the panel. No runtime or no signed-in
   account means no consumed draft.
3. `agent_host::run_effect` builds the prompt with bounded thread context and
   calls `ProviderRuns::start`, which saves intent before launch.
4. `pm_agents::launch` finds the binary: `CODEX_BINARY`/`CLAUDE_BINARY`, then a
   managed update under `<profile>/runtimes`, then `Resources/<name>/bin/<name>`.
   Isolated homes are `codex-runtime` and `claude-runtime` in the profile.
   Editor ChatGPT runs use App Server; Project runs use `codex exec`; Claude uses
   stream-json. Providers reach tools through `--powermove-mcp`.
5. The `pm-agent-runs` worker polls events into the panel and publishes. A Backend
   lock is not held while it does. A failed save keeps the event for retry.
   Reply and reasoning fragments (`Event::streamed`) and composer typing are
   kept in memory and saved at most `CHECKPOINT_MS` (1 s) later, or with the
   next event that saves; they keep no receipt. Everything else saves at once.
   A crash can lose up to a second of streamed text or typing.
6. Anything that keeps an effect from its process settles in the conversation
   (`ProviderRuns::refuse`): a start fails its run with a readable reason, a
   refused Stop or answer is acknowledged as refused.
7. Window close sets `AgentLink::close`; the worker cancels runs, saves and
   exits. The application panel is the exception: when its window goes because
   a run opened a project, `Builtins::carry_agent` keeps the runs, the `Host`
   holds them, and the next application window shows them
   (`Builtins::application(.., carry)`). Dropping the carry cancels them.

Sign-in is `pm_agents::account`, on `pm-agent-account` workers. ChatGPT uses App
Server's `account/*` and `model/list`; Claude uses `auth status|login|logout` and
an `initialize` control request. Status drives the panel's connection and the live
rows in Settings › Accounts. Each provider's runtime owns its credentials.
Stop always reaches a live run and answers do while it lives, whatever the
account says now; the panel keeps a live run's conversation after sign-out.

The composer has Access, provider, model and effort pop-ups, filled from the same
workers. Access has no Full choice, because no consent alert exists. Question and
approval cards answer through their own buttons.

Composition-only runs differ from main on purpose. Main returns a structured plan
that the renderer applies. Here the run edits through its MCP session, limited to
`catalog::composition_names()`: `enabled_tools` on Codex, `--allowedTools` on Claude.

A click planned against an older surface revision lands if its target's subtree
is unchanged since (`Surface::unchanged_since`, mirrored in `renderer.ts`); keys
follow focus and land on their element as it is now. So Stop works while a run
streams, with no retry.

Proof is `builtins/src/agent_tests.rs`: real V8 panel, real kernel actor and a fake
provider over real pipes, for reply and reopen, Stop during a stream, question and
approval cards, pickers reaching the argv, sign-in, sign-out mid-run, and an
application run outliving its window. Its executable is `/bin/false`, so no test
connects a panel run to the app's MCP tools. Sign-in is proven from the
`panel.settings` command and, separately, from `SettingsModel.perform`; nobody has
clicked the rendered Settings button. No real provider has
run: `live-provider-check` is unexecuted, and the Claude model request is unproven
against the real binary.

Open:

- Provider `model` efforts aren't per model; the pop-up shows the provider's union.
- `Compatible` refuses at launch: no HTTP or Keychain host. Its parser and presets
  are in history (see agents-main.md). Computer access stays refused because no
  consent alert exists.
- Attachments refuse with an explanation.
- `INSTRUCTIONS` in `launch.rs` is a short stand-in for main's generated workspace
  instructions, and user MCP servers and skills aren't shared into the homes.
- Project access has no OS confinement host for Compatible-only workspace tools.

The sections below are older dated notes.

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

## 2026-11-04 — ordered delivery, history and ordinary-run watches

This section supersedes the earlier alternate-lockfile setup. The integration
base already contains the `pm-agents` package in `apps/native/Cargo.lock`.
`crates/agents/scripts/check.sh` uses ordinary locked workspace commands. There
are no new dependencies or lockfile resolutions in this change. The old
lane-owned lockfile remains historical evidence, with no active check consumer.

`delivery::{Surface, Queue, Delivery}` provides a bounded, actor-owned FIFO.
Each admitted batch receives monotonic sequence numbers. Saturation returns the
entire `Effects` unchanged, with no sequence consumed. Dispatch order is outgoing
messages, the original event order, then stdin closure. The process worker uses
that same ordering function. A wire enqueue is not a provider acknowledgement;
interruption completion is fenced by the RPC response or its bounded failure.

The typed boundary separates frozen callbacks from native inspection. Codex and
Codex-exec session identities, Ready, Stopped and unknown notifications remain
Inspection deliveries. Claude's session callback remains a Callback, matching
its tagged parser. Every event is retained. The existing native process handle
continues to expose all events; consumers opting into the compatibility boundary
must use the typed adapter and retain its inspection deliveries. This is an
application integration API, not a claim that the shell already consumes it.

`history::{Receipt, publish, read}` stores exact native event JSON in the existing
thread archive's additive `nativeRunEvents` field. A receipt's source/run and
sequence are immutable; retries are idempotent, reordered/conflicting receipts
fail atomically, and compaction does not replace earlier messages or decisions.
`Threads::update` validates and durably publishes a candidate before changing
its in-memory view. Failed publication acknowledges no event. Archive removal
retains these receipts with the entire thread. Unknown fields are preserved;
unrecognized history shapes are refused unchanged. Wire entries can create gaps
in receipt sequence numbers without making event order ambiguous.

`monitor::Monitor` ports the frozen ordinary-thread monitor behind a supplied
clock. It retains run/project/thread identity, progress, question/wait state,
terminal summaries and the 30-second announced-but-never-started timeout.
Run-specific watch leases enforce project ownership, reject cycles and support
idempotent release, including duplicate edges. Watching an older run never
switches to a later turn. It has a 32-watch-per-parent limit. At 512 retained run
records, admission refuses more work until the host explicitly archives a
terminal, unwatched snapshot. `archive_terminal` invokes the host's durable
archive operation before retiring that in-memory entry. The host must retain
its immutable run-ID index and serve archived reads; no silent pruning or GC is
provided.

Concurrent Codex initialization, broadcast progress and shutdown now use run
admission order, rather than lexicographic map order. Held questions retain
arrival order. For an identified active turn, cancellation sends interruption
first, refuses further answers/steering, and settles held questions and the
terminal result after interrupt settlement. The early-turn cleanup tombstone
still prevents a late `turn/start` reply from leaking work or a second result.

Decisions and selecting rules:

| Decision | Selecting rule |
|---|---|
| Make callback/inspection channels explicit; preserve every native observation and keep the original strict gate | N3 exact compatibility boundary; N7 inspectability; no filtered claim of whole-service identity |
| Atomic FIFO batch admission and one worker dispatch order | N1 bounded backpressure; N7 deterministic headless driving; N8 no JS/main-thread wait |
| Preserve run/question arrival order and fence cancellation on interruption | N3 authored request ownership and sequencing, demonstrated by tagged execution |
| Publish lossless event receipts before acknowledging them; retain opaque fields and archived history | N4 no lost data; N3 exact authored decisions and compaction history |
| Refuse capacity overflow and require durable archival instead of the monitor's automatic pruning | N1 bounded state; N4 archive, never silently delete |
| Canonicalize test temporary roots | macOS-safe root-owned `/tmp`/`/var` system symlinks; no Linux-only filesystem assumptions |

No visual surface, copy or interaction design was added. No new
`DESIGN-TODO(opus)` placeholder is required by these service-only changes.

## 2026-11-04 — durable task scope and combined result barrier

`orchestration::Scope` adds a provider-independent reservation/result state
machine. `Host` supplies actual provider/model selection and actor-owned durable
storage; this crate does not advertise provider availability. Its relative path
matches the shipped archive:
`Agent Tasks/<sha256(project)>/<sha256(project/thread)>.json`.
The storage adapter must compare the expected archive on the actor before
publishing, and reject stale scopes unchanged. The application must additionally
hold one execution lease per conversation, as the tag's `activeScopes` does.

The service validates requests and inherited identities, preserves the complete
parent request (project, planning, access, attachments and opaque context), and
returns typed Start/Cancel/Continue commands. It supports idempotent delegation,
immutable terminal results, parent ownership, four nesting levels, eight active
children and 32 tasks per root run. Reservation publication precedes any Start
command. Failed provider discovery or storage returns no Start and changes no
reservation. Saved nonterminal tasks reopen as interrupted without silently
restarting a provider. Unknown archive fields remain, and malformed/versioned or
scope-mismatched archives are refused unchanged. The 512-record/32-MiB bounds
refuse new work rather than pruning user history; the host needs an archive
rotation path for long-lived scopes.

`cancel` requests only the owned task and descendants; provider settlement is
required before the parent result becomes terminal. `stop` durably records the
root stop before returning cancellation commands. Independent ordinary threads
are absent from the task graph and are never stopped by these operations.
Terminal status reads acknowledge a result. Reading active status has no such
side effect, and waiting for children does not cancel them.

`after_turn_with_watches` joins child and run-specific watch results. It waits
for all pending work, then durably publishes a stable delivery ID and complete
untrusted result snapshots. `nativePendingDeliveries` is an additive archive
outbox; it survives a crash and preserves unknown result fields. The host must
idempotently accept that ID into the parent's durable history/continuation
outbox before calling `acknowledge_delivery` or `acknowledge_with_watches`.
Failed acknowledgement leaves the same delivery available for retry. Watch
leases are released only after durable acknowledgement. This intentionally
strengthens the tagged in-memory continuation handoff; it does not claim that
an unwired application already resumes work automatically.

| Decision | Selecting rule |
|---|---|
| Preserve tagged task fields, path, parent ownership, idempotency and inherited requests; prove the reservation boundary against the source | N3 authored intent; N4 existing task archives; N7 headless IDs |
| Require real provider selection and actor compare-before-publish; return commands only after reservation durability | N1/N4 prevent duplicate/lost work and stale overwrites; kernel single-writer architecture |
| Native durable continuation outbox with explicit, idempotent acknowledgement | N2/N4 improve crash behavior; N3 never silently mark undelivered results consumed |
| Root stop and descendant drain are separate from unrelated ordinary threads | N3 exact scope/authority semantics; N1 cancellation correctness |
| Retain interrupted/terminal/unknown records and refuse capacity overflow without GC | N4 archive rather than delete; N1 bounded state |

### Concrete integration still required outside this crate

The kernel/application owner must implement `Store` and orchestration `Host` on
the existing actor/archive authority, supply the real provider catalog, enforce
one active root per conversation and drive the supplied clock. The application
must consume Start/Cancel/Continue, retain workspace/session identity, route
child questions and approvals by original IDs, build the frozen continuation
prompt, persist/deduplicate deliveries and register the MCP adapters. Explicit
ordinary-thread launch/list/read/send/cancel, `task_send` live answer/steering and
provider reconfiguration, bounded wait scheduling, complete child-question
forwarding and external-MCP-root lifetime management are **not implemented by
this scope core**. No MCP tool is marked implemented by this lane.

The shell must opt into the typed callback boundary while continuing to consume
inspection events, and connect event receipts before acknowledging callbacks.
Final shell/kernel queue order, provider cleanup fences, real process restart
and automatic watch delivery remain integration gates. These changes stay within
`crates/agents/**` and the agents documentation; no kernel, MCP, shell, engine,
vendor, spike or Electron consumer is changed or pruned.

On macOS and with credentials, verify both pinned providers through the signed
app: questions and Project/Computer approvals, interruption success/error/timeout,
concurrent runs, reconnect/resume after a process restart, automatic compaction
with durable history, nested child cancellation/draining, watch/unwatch and
exactly-once accepted continuation delivery. Test actor/file durability and
runtime-owned auth/session preservation under process kill. Linux doubles and
source-derived recordings do not certify these behaviors, native UI or signing.

## 2026-11-05 — current-main native Claude launch

`claude_launch::Options` now builds and launches the native stream-json worker
against owner-pinned main `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`.
Project Auto selects Claude Code's `--permission-mode auto`; supervised Project
uses `acceptEdits`; Editor keeps its denied shell/write tools; Computer still
requires fresh run consent. Requests reaching stdio remain user questions or
fallback approval cards. This supersedes the old claim that Claude has no
classifier. No runtime-availability flag or live-provider pass is inferred.

Three portable tests compare all arguments and stdin against **12 executions of
main's unmodified adapter**, verify pinned source/recorder hashes, and reject
missing instructions/schema/MCP maps. CLI JSON values compare structurally;
all flag positions and other argument bytes remain exact. The recorder injects
synthetic native/external MCP descriptors and instruction strings, so it tests
launch policy, not tool catalog availability or generated workspace instructions.
Regenerate with `node crates/agents/scripts/capture-main-claude-launch.cjs --check`
from `apps/native` after installing the existing `work/oracle` TypeScript package.

`cargo run --locked -p pm-agents --example live-provider-check` is an explicitly
opted-in editor-only credential check. See the example's `PM_AGENT_LIVE_*`
inputs; no key or transcript is copied or printed. It has not been run here.
The Mac must verify the pinned native CLI, isolated auth homes, Project Auto
classifier/fallbacks, supervised approvals, signed launch and both architectures.

Decision: use main's real classifier/argv and preserve the remaining stdio
approval boundary (A03, N3/N4); native subprocess/worker launch with no Node or
main-thread JS wait (N7/N8). No old consumer or runtime archive is pruned.
`DESIGN-TODO(opus)`: reconcile the old Access picker's Auto subtitle in
`design/panels/agent-panel.md` with main's Claude classifier; no new copy or
visual choice is supplied by this service change.

## 2026-11-05 — real process, panel and actor persistence path

`pm-panel-agentpanel::{runtime::ProviderRuns,store::KernelStore}` connects real
provider process handles to the existing thread model and attached kernel library
writer. Submitted intent persists before launch; checked machine acceptance and
interrupt acknowledgement are distinct; event receipts and panel projections
publish together; failed IO retains the exact pending delivery. Reopen marks
unfinished runs interrupted without automatic restart, preserves their old
status, and cannot restore Computer authority. The real-actor/real-pipe test
reopens and compares the entire durable archive.

See [agents-cloud4.md](agents-cloud4.md) for the integration APIs, decisions and
rules, exact outside-owned changes and A01–A10 remaining acceptance. This closes
portable service boundaries, not the missing application registration/renderer,
provider credentials, project-memory metadata admission or all of HANDOFF 9.
