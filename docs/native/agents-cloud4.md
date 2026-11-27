# Agents/MCP cloud lane 4 — 2026-11-05

Reference: main `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`, retaining
v1.2.0 source recordings and extension/data compatibility. This is portable
service integration, **not completion of HANDOFF 9 or A01–A10**. No full row is
marked closed on the strength of a model, catalog or unavailable Mac surface.

## Concrete path implemented

`pm-panel-agentpanel::store::KernelStore::bind(existing_kernel_handle)` binds the
application's attached library writer. It opens no library and takes no second
lock. The existing actor state API admits `agentThreads.*` and
`agentAttachment.*`. A read-only profile call verifies that the actor actually
has a library: memory-only state acks are insufficient. Binding also requires
that profile's localStorage sidecar to be readable. Agent writes use the observed
slot value and the current actor state revision; foreign changes refuse the
write unchanged. Unrelated state-revision races are explicit retryable failures,
never permission to overwrite a stale archive. The adapter acknowledges only the
kernel's durable state/slot publication. All synchronous Store calls run on the
serial service worker; kernel `Pending::wait` rejects the Mac main thread.

`ProviderRuns` now connects real `pm-agents::transport::Handle`s to `Panel` and
that Store. The host stages its original request/workspace, calls `Panel::submit`,
then `ProviderRuns::start`. Start saves the complete brief/run/settings before
invoking the native launch factory; failed storage launches no provider. One
process lease per conversation and at most eight workers are admitted. App
registration must call these APIs on its existing worker, not create another
kernel actor or library writer.

`dispatch` saves steering/question/stop intent before enqueueing. The new
`Handle::send_checked` distinguishes bounded mailbox admission from the protocol
machine accepting an action. A withdrawn/already-answered question returns false.
The bridge durably settles the exact effect id before delivering later events;
a failed acknowledgement write retains the same result for retry. Checked
acceptance is **not provider RPC acknowledgement**. In particular a Codex stop
remains running/stopping until the actual interrupt response settles.

`poll` drains at most 256 observations, using stable admission-order polling
across available workers. This is not a claim of a global timestamp order across
independent OS processes. Every native event, including lifecycle inspection,
is stored with immutable run/sequence identity beside its conversation projection
in one archive publication. Failed publication leaves the old model and queued
observation intact. Late cleanup is inspection for the old run; it cannot end a
new turn. Archival retains the terminal receipt and later reaping observations.

Loading saved queued/running/waiting records durably marks them interrupted,
retaining the previous run/status in additive `nativeInterruptedRuns`. No provider
silently restarts and no Computer authority is restored. Unknown fields, partial
text, complete receipts, attachments and archived threads remain. The inherited
32 MiB archive bound is an explicit refusal; no history GC was added. This
implementation publishes full candidate archives and has no measured 60k-row
streaming/fsync latency result; profiling and a durable archive rotation path
remain necessary for very long professional sessions.

The Claude launch implementation and its 12 source executions are described in
[agents.md](agents.md#2026-11-05--current-main-native-claude-launch). The native
Claude Code registration worker and readback are described in the dated section
of [mcp.md](mcp.md).

## Evidence and rules

New tests drive actual subprocess pipes through questions, answers, background
threads, held Codex interrupt acknowledgement, storage/ack failure and retry,
terminal archival and late cleanup. Another test takes that same process path
through the real kernel actor and library writer, shuts the actor down and opens
the same profile again, comparing the complete retained archive. Actor tests
also reject stale writers, unobserved writes, foreign keys and memory-only acks.
These are synthetic peers, not real providers or a running signed application.

| Decision | Selecting rule |
| --- | --- |
| Bind the existing actor's durable settings path; compare slot and actor revision | N3/N4 and single-writer architecture; no lost foreign data or second writer |
| Publish intent before process launch, and event receipt with projection before acknowledgement | N1/N2/N4; no false started/completed or consumed result after failed durability |
| Separate mailbox admission, machine acceptance and interrupt completion | N1/N3; exact ownership/cancellation, not an Electron scheduling imitation |
| Keep native lifecycle/late events as inspectable records without applying them to another run | N3/N7; retain all evidence and fence old workers |
| Mark interrupted state durably, preserving previous state; do not restart or grant authority | N3/N4; preserve intent without silently performing new work |
| Use current-main Claude Auto and native CLI registration with config readback | N2/N3/N8; real native processes and observed success |
| Keep old extension/app consumers and all existing replay assertions | N4/N6, prune rule; no incomplete cutover or weakened strict gate |

## Remaining acceptance and exact outside-owned work

| Row | Portable change / remaining work |
| --- | --- |
| A01 | Unchanged: compatible provider presets, discovery, address-scoped Keychain keys, tool-model filtering and local context/error behavior need their real service/application integration. |
| A02 | Real Claude Code CLI registration + config readback added. Existing nine-client review/archives/Claude Desktop consent remain. Settings Accounts command registration, Copy setup, actual installed client restart and signed executable proof need `crates/builtins`, `crates/app`/shell and the Settings host owner. No silent Claude Desktop addition. |
| A03 | Native current-main Auto/supervised/editor/Computer launch implemented and source-compared. Application must supply normalized discovered model efforts, instructions, native MCP descriptors, isolated homes and real consent. Verify classifier and fallback cards on Mac. |
| A04 | Real streamed provider observations now reach durable panel conversations. Main work-group planning already exists; rich Markdown, safe local/remote grants and byte-range media playback are not connected to a native agent view. No parser-only or blank-pixel pass is claimed. |
| A05 | Real process/thread/restart/archival path added. `publish_visual_reply`, inert artifact viewer, resize/expand/replay, shell pin/drag/focus and native conversation renderer remain. |
| A06 | Existing MCP error content/source/catalog tests retained. Missing orchestration, panel, workspace/publication and other tool bodies remain explicit errors. No new capability flag claims them. |
| A07 | Closed by the kernel `Call::Memory` path: `project_memory` writes durable metadata without a revision or Undo step. See [agents-main.md](agents-main.md). |
| A08 | Extension rebase/publication/theme/workspace coordinator and DOM integration remain outside these paths. |
| A09 | Checked actions, per-conversation process lease, durable stream/ack ordering and retired-run fencing added. App document lifecycle admission, external-root lifetime, extension publication/recovery queue, panel capture/input generation fences remain with their owners. |
| A10 | Existing permissions retained; CLI teardown preserves its original error. Actual provider review/capture failures preserving edits, original settings, and supervised outside-sandbox consent still need signed-app evidence. |

The original raw-observation strict replay remains a separate failing check:
retained native lifecycle additions and the recorded main/Studio catalog delta
must not be removed or normalized to call it identical. Provider callbacks and
MCP error text have strict tests; that narrower fact does not close the raw gate.
No acceptance CSV, shared coverage table or denominator is rewritten.

`DESIGN-TODO(opus)`: reconcile the old Auto subtitle with Claude's current
classifier. The existing four agent-panel placeholders/TODOs remain owned by
Opus and the native kit/renderer owner. No visual output or product copy was added.

## macOS and opt-in provider checks

Run both providers through the signed app on both architectures: initialization,
question/withdrawal/answer, Project Auto classifier/fallback, supervised outside-
sandbox approval, fresh Computer consent, stop success/error/timeout, nested
cancel/drain, concurrent threads, reconnect and process-kill recovery. Verify
actor fsync/rollback behavior and isolated runtime auth/session retention. Check
real Claude Code registration, hand-edited config preservation, stale review,
readback and intentional removal; Claude Desktop must show consent first.

The explicit credential probe is editor-only and does not test Auto or signing:

```sh
# Supply existing native binary, isolated runtime home and disposable workspace.
# Run from apps/native, after the standard activation.
PM_AGENT_LIVE_CHECK=1 PM_AGENT_LIVE_PROVIDER=claude \
PM_AGENT_LIVE_BINARY=/absolute/native/claude \
PM_AGENT_LIVE_RUNTIME_HOME=/absolute/isolated/claude-home \
PM_AGENT_LIVE_WORKSPACE=/absolute/synthetic/workspace \
cargo run --locked -p pm-agents --example live-provider-check
# Repeat with provider=codex and its own binary/isolated home.
```

No credentials are configured here and neither probe was run. Native UI,
GPUI/WebKit, AX/IME, media playback, foreground/background timing and Opus visual
review remain unverified on this Linux container.

## Final portable verification — 2026-11-05

Implementation commits: `4b29920e5` (Claude launch), `0511f3daf` (process/panel/
actor path), `7f57fd07a` (native CLI registration). Merged base:
`5bcd425ddc158cbd891ee4b515cf9993fad29a66`; merge reported already up to date.
The launch oracle was then expanded from 12 to **15** current-main executions,
adding descriptor/model/resume-free standalone runs used by the live probe.
All previous cases remain. This supersedes the earlier 12-case count.

After standard activation and `cd apps/native`:

```sh
cargo test --locked -p pm-agents -p pm-mcp -p pm-panel-agentpanel
# 401 passed: 132 agents, 137 MCP, 132 panel; 0 failed, 0 ignored.
cargo clippy --locked -p pm-agents -p pm-mcp -p pm-panel-agentpanel --all-targets --no-deps -- -D warnings
cargo fmt -p pm-agents -p pm-mcp -p pm-panel-agentpanel --check
PM_REPLAY_REQUIRE_IDENTICAL=1 cargo test --locked -p pm-agents --test replay replay_all_surfaces_and_check_every_difference
# Separate strict result: 1 failed assertion, 9 filtered; not counted as a pass.
```

From the repository root:
`node apps/native/crates/agents/scripts/capture-main-claude-launch.cjs --check`
reproduces all 15 source cases. Rust 1.96.1, strict Clippy and formatting pass.
No dependency was added or resolved differently, so neither workspace lockfile
requires an update. Every Cargo test/build used the existing `--locked` graph.

The original strict report has **359 exact leaf differences**: 28 retained native
lifecycle additions and 331 pinned main/Studio catalog deltas relative to the
legacy recording. `mcp-tool-errors` has no difference. The regression suite
continues to require those exact deltas and the separate provider-callback
assertion remains strict. None is waived or erased. Full P4/A01–A10 completion
is still blocked on the concrete outside-owned bindings/metadata/view operations
above; the final branch signal must say BLOCKED, not imply application acceptance.
