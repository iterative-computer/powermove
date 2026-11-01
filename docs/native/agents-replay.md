# Agent conformance replay — Package 16 / Gate P4

**Harness implemented; “conformance replay identical” remains OPEN.**
Baseline: `v1.2.0`, commit `07ed5fa17ee640659cb075f9cad1af2095011d60`.
Native base: agents lane `1b88d384e`. No production client or MCP behavior is
changed by this replay lane. No Electron consumer is pruned.

## Evidence and comparison contract

`apps/native/crates/agents/tests/replay/capture.cjs` executes frozen tagged code
with synthetic requests, fake provider processes and synthetic services. It
reuses the inherited provider recorder functions with their output destination
redirected, and adds Codex failures, Claude control/housekeeping and MCP sessions.
The shipped Claude shell fixture and Codex recorded NDJSON are public test data.
No real client transcript, account, credential, user profile or project is read.
This is source-derived reference execution, **not a running Electron capture**.

`fixtures/replay/manifest.json` pins the tag commit, 18 tagged source hashes,
both capture harness hashes, TypeScript 5.9.3, Zod 4.6.5 and each of the 23 case
files. Captures come from tagged methods and getters, never native outputs.
Dependencies not exercised by these probes are injected test doubles; this does
not certify their behavior. The inherited recorder normalizes only generated
`/tmp/powermove-codex-app-*` working directories to `/workspace/agent`.
Authored IDs, numeric versus string IDs, array order, input/schema fields,
error text, null versus omission and serialized MCP text remain exact. Only
JSON object property order is insignificant.

The new test-only module `src/replay/mod.rs` is included by `tests/replay.rs`
using `#[path]`. It needs no edit to the production `lib.rs`, shared manifests
or lockfile. Provider inputs drive the real native state machines. Every wire
output and every public event is compared in order. Each carries the last
stimulus position, so an output moving across an incoming message or user action
also differs. Trace and progress share one ordered stream. Native Ready,
Session, Notification, Stopped and stdin-close observations are retained in a
separate ordered lifecycle stream; none is silently ignored. Message run IDs
must equal the synthetic run ID. Unknown fixture frames fail closed.

`Effects` exposes wire output and events in separate collections; it supplies
no total order between those collections. Replay checks each collection's order
and stimulus boundary, not an invented cross-collection timestamp. Lifecycle
and public-event relative order is likewise not a certified application dispatch
order. Production dispatch/queue ordering remains an integration gate.

MCP replay builds the actual `powermove-mcp` binary and drives its JSON-RPC stdio
with a fresh temporary synthetic profile. It compares **all stdout messages**,
including notification silence, parse errors, request IDs and complete schemas.
Both pipes are drained concurrently; a stalled child is killed and reaped.
`tools/list` compares all 67 frozen names, descriptions and schemas. The one
successful tool-body probe executes tagged `mcpProjectTool` for an empty home
library and compares the native `list_projects` result. Error probes execute
`ExternalAgentSession.call` without an active session. These probes do not
claim every tool body works or exercise a live project mutation.

Claude parser captures enable session and result callbacks as well as trace,
progress and warning callbacks. The tagged `onResult` signal records stdin
closure. Terminal result values adapt the tagged parser's actual output/text/
error getters into the runner result envelope; these are not a full Claude
runner/process-exit capture.

## Measured coverage on Linux

**23 cases; 164 reference messages; 192 native observations.**
Six cases match all compared observations. Wire messages match in **22/23**
cases; public events, including causal stimulus positions, match in **22/23**.
The other observations and all current differences appear explicitly in
`fixtures/replay/differences.json`. An empty wire/event stream is not evidence
of outbound protocol coverage; the rows below identify each boundary.

| Cases | Observations | Replay result |
|---|---|---|
| `codex-happy` | initialize/initialized/thread/turn, model/effort/schema, streamed answer and duplicate suppression | Wire + public events identical; native Ready/Session additions |
| `codex-question` | blocking question, exact answer shape, string RPC ID, numeric ID collision, MCP elicitation decline, unsupported request error | Wire + public events identical; Ready/Session additions |
| `codex-steer` | thread/expected-turn IDs, steering input and final result | Wire + public events identical; Ready/Session additions |
| `codex-cancel` | interrupt RPC, cancelled result, causal timing | Wire identical; result timing differs; Ready/Session additions |
| `codex-compact` | compaction start/end, model reroute, final answer | Wire + public events identical; Ready/Session additions |
| `codex-async-question` | async question card, hidden duplicate prose, final answer | Wire + public events identical; Ready/Session additions |
| `codex-tools` | command/file/search/MCP/dynamic/media/sleep/collaboration/image/tool-error rows | Wire + public events identical; Ready/Session additions; no MCP tool-body claim |
| `codex-retry` | reconnect warning/error deduplication, resumed progress/reasoning | Wire + public events identical; Ready/Session additions |
| `codex-exec` | shipped NDJSON parser traces/progress, structured display filtering | Public events identical; native session addition; not a process/result-file replay |
| `codex-initialize-error`, `codex-thread-error`, `codex-turn-error` | exact request/result error envelopes and cleanup | Wire + public events identical; native lifecycle additions |
| `codex-missing-thread`, `codex-missing-turn` | absent provider identity produces exact failure text | Wire + public events identical; native lifecycle additions |
| `codex-turn-failed`, `codex-context-exhausted` | failed turn and post-compaction exhaustion diagnostic | Wire + public events identical; Ready/Session additions |
| `claude-control` | question held/replied by original text, tool permission denial, unsupported control | Identical at control/answer boundary |
| `claude-stream` | shipped stream, thinking/tool/text, deduplication, session, structured output, stdin-close signal | Identical at parser/runner-result adapter boundary |
| `claude-skipped-withdrawn` | skipped question denial, control cancellation, late answer refusal | Identical at control/answer boundary |
| `claude-compaction-retry` | duplicate compacting status, boundary pair, retry progress, result/close | Identical at parser/runner-result adapter boundary |
| `claude-error` | provider failure, terminal error and close | Identical at parser/runner-result adapter boundary |
| `mcp-stdio` | pre-init refusal, initialization/version, notification silence, duplicate init, ping, 67-tool catalog, malformed arguments/ID/version/JSON, unknown method, empty-library tool call | All 12 response messages identical |
| `mcp-tool-errors` | missing session, malformed finish args, unknown tool and finish without session | Initialization identical; all four tool-error text values differ |

The exact discrepancies are:

- Native Codex emits Ready/Session or Stopped inspection events unavailable at
  the tagged callback boundary. Native Codex-exec also exposes its session ID.
  These explain the **28 extra native observations**. They remain in the report.
- In `codex-cancel`, native completion occurs at stimulus **5** (cancel), while
  the frozen Promise resolves after stimulus **6** (interrupt acknowledgement).
  The payload and interrupt match. The earlier completion was invisible to the
  inherited queue-only test. No cancellation implementation is edited here.
- Native MCP wraps tool errors in JSON text containing `code`/`message`, whereas
  the tag returns plain text. It substitutes `Missing arguments.commit` for the
  shipped Zod diagnostic. With an unknown tool and no session, it reports
  `Unknown tool: synthetic_unknown_tool` before the tag's no-session check.
  These are compatibility gaps, not approved golden substitutions.

## Decisions and selecting rules

| Decision | Selecting rule |
|---|---|
| Execute tagged logic with public synthetic inputs; hash sources and fixtures; never overwrite reference output with native output | N3 authored intent, N4 no user data, N7 inspectable evidence |
| Preserve exact message fields, IDs, errors, ordering and stimulus positions; distinguish absent from null in persisted diffs | N3; N6 extension/tool callers cannot rely on a filtered comparison |
| Keep additive native inspection events visible and separately identified | N7; N3 no false claim of full identity |
| Record cancellation scheduling as a gap pending integration resolution, without imitating asynchronous Electron dispatch in Rust | N1/N3/N7 and quirk policy; no authored intent establishing microtask timing was found |
| Keep frozen MCP error text beside native structured errors; request an owner fix/intent decision | N3/N4; a richer native error must not silently replace the client contract |
| Exact difference ledger for regression testing, plus strict identity mode that fails on any difference | N1/N3 evidence integrity; known does not mean conformant |
| Test-only Rust replay, process workers, injected services, no app dependency on Node/Chromium | N7 headless; N8; Node/TypeScript are oracle tooling only |
| Preserve inherited consumers, source fixtures and production modules | Prune rule and explicit lane ownership |

## Running and interpreting checks

The agents base has its package entry in the lane-owned alternate lockfile;
`apps/native/Cargo.lock` is intentionally unchanged. Its existing check script
sets Cargo's alternate-lockfile switch and runs the real native workspace:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
PM_REPLAY_REPORT=/workspace/powermove/work/replay-report.json crates/agents/scripts/check.sh
```

This runs locked tests for `pm-agents`, strict Clippy across all agent targets
and formatting. The replay test builds `pm-mcp` once with the same lock settings.
CI may supply `PM_REPLAY_MCP_BINARY` pointing to its freshly built native binary.
Python 3 is used by process tests. Rust tests do not require Node or a provider.

Regression mode compares every current difference against the exact ledger,
failing on a new, changed, removed, orphaned or stale difference. The ledger
records rules/reasons; it is **not** a waiver of the P4 gate. For strict identity:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove/apps/native
RUSTC_BOOTSTRAP=1 CARGO_UNSTABLE_LOCKFILE_PATH=true \
  CARGO_RESOLVER_LOCKFILE_PATH="$PWD/crates/agents/Cargo.lock" \
  PM_REPLAY_REQUIRE_IDENTICAL=1 cargo test --locked -p pm-agents --test replay \
  replay_all_surfaces_and_check_every_difference
```

**Strict identity currently fails, as expected.** There is no golden-accept or
native-to-reference update mode. To independently regenerate the reference:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
npm install --prefix work/oracle typescript@5.9.3 zod@4.6.5
node apps/native/crates/agents/tests/replay/capture.cjs --check
```

`--check` writes only to `work/` and compares generated bytes to committed
fixtures. Omitting it deliberately regenerates reference artifacts from tagged
code; review their diff separately from `differences.json`.

Linux verification: **95 tests pass** (87 inherited + 8 new). The new tests
cover all 23 replay cases, source/recorder integrity, required-flow inventory,
strict unknown-frame rejection, altered IDs/schemas/errors/order/extra messages,
causal shifts, absence/null round trips, manifest tampering/path/version checks,
full pipe draining and stalled-peer termination. Oracle regeneration is identical;
Clippy with `-D warnings` and package formatting pass. Strict identity is also
executed and fails on the retained discrepancies; it is not counted as a passing
conformance check. No real provider binary or account was launched.

## Remaining work and macOS verification

Changes needed outside this lane:

1. Agents/application owner: resolve the observable cancel-completion boundary
   in `codex.rs` and final application dispatch. Decide how additive inspection
   events enter the compatibility surface; do not remove native inspectability.
2. MCP owner: preserve tagged tool-error content and resolve validation/error
   precedence (`session.rs`, catalog validation and the stdio adapter). Additional
   structured data may be additive, but needs a contract decision and replay.
3. Shared workspace owner: integrate the existing agents package lock entry.
   Export the test-only replay module from `lib.rs` only if another Rust consumer
   needs it; the current gate test already uses it without production changes.

On macOS verify both pinned native provider architectures in the signed app,
real app-server and Claude stdio control, automatic compaction/exhaustion,
question and cancellation UX, process shutdown and runtime-owned session/auth
preservation. Replay integrated Rust MCP through the real kernel actor and
native application queues, including renderer-independent hidden-host use.
No Metal/AppKit/GPUI/WebKit/Keychain behavior is proven by these Linux tests.

Coverage still missing: live/provider process scheduling and startup failures;
multiple concurrent runs and resume/reconnect across process restarts; timer and
transport fault schedules at every causal boundary; Project/Computer approval
flows; end-to-end orchestration/delegation/thread watches and durable compaction
history; mutation/rollback/export/media/extension tool bodies; full extension
activation and tool availability. The existing MCP inventory has only 13 of
67 implemented standalone bodies (54 pending); catalog identity does not close
that gap. These limitations prevent declaring all of Gate P4 complete.
