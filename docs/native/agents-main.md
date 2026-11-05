# Main-reference agent services

## 2026-11-05 — A01–A10 portable implementation and explicit integration gates

The behavior reference is main commit
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`, per the owner decision.
This document supplements `agents.md` and `agents-replay.md`; historical dated
sections describe their original evidence, not the current acceptance target.
All A01–A10 remain in scope. Portable service implementation is **not** proof
that an application tool, account screen, provider or signed macOS viewer works.
No existing consumer was removed.

| Delta | Agent-owned implementation | Remaining acceptance/integration |
|---|---|---|
| A01 | `compatible` canonicalizes addresses, preserves aliases, discovers models/local servers, probes tool support and Ollama context, binds keys per address and plans durable configuration. `completion` incrementally parses bounded UTF-8/SSE, tool fragments and terminal errors. Thirteen public presets come from executed main. | Real worker HTTP (no redirects), native Keychain/config archive host, network cancellation, provider runner/tool loop and Opus connection views. Recorded HTTP and encrypted keys are synthetic. |
| A02 | `registration` supports nine public client formats/metadata, ownership inspection, conservative commented/custom config preservation, native Claude CLI arguments and compare-before-publish plans. Claude Desktop **Connect requires explicit single-use consent bound to the exact plan**. Identical plans do not write. | Real worker config/CLI host with archive, atomic compare/write/readback and intentional-removal receipt; installed signed native executable; S070 consent presenter. No live settings were changed. |
| A03 | Claude Project Auto arguments now use main's `auto` permission classifier; Supervised uses `acceptEdits`, Editor uses `default`. All six access/approval flag combinations compare with actual main adapter execution. The fallback stdio authority stays conservative. | Live pinned Claude classifier, sandbox and held-question round trips. Selecting `auto` does not certify classifier/runtime capability. |
| A04 | Inert markdown AST, nested/task/ordered lists, tables, quotes, fences, safe URLs and media descriptors; reply/work groups and steering clock. Rooted regular-file media grants, ranges/HEAD and session-bound reads. Main's 21 actual markdown tests are executed by the reference recorder. | Native token-based renderer/player, remote-image opt-in, actual sandbox/file-grant and media playback tests on macOS. Parser emits semantics and performs no fetch or raw-HTML rendering. |
| A05 | `visual` validates/preserves authored HTML and publication limits, durable-thread candidates, opaque-frame message authority and size constraints. Existing conversations preserve provider session/recovery observations. | Real `publish_visual_reply` MCP binding, safe WebKit viewer/CSP enforcement, conversation drag/pin/resize/replay and Opus review. Validation alone cannot enforce browser isolation. |
| A06 | Existing pinned transport/replay and session gates; `jobs` tracks 20-minute call wait separately from 12-hour export deadline, measured engine progress, retained terminal outcomes and archive acknowledgement. | MCP catalog/description/job-field updates, tool implementations still explicit errors, real engine progress/deadline/cancel wiring and external application host. No fabricated export progress or successful tool result. |
| A07 | `memory` ports list/add/update/remove, singleton goal, kinds/reasons/resolution/limits, disabled-memory hiding and project index. Host publication requires actor compare-and-swap, non-Undo autosave and no revision bump. Displaced raw records archive instead of deleting. | Actual kernel actor metadata operation, autosave integration, `project_memory` MCP schema/dispatch and `get_project_state` memory index. The current MCP binary still lacks this tool. |
| A08 | Publication receipts/session fences prevent late Stop from undoing an already committed actor publication. | Complete reversible theme/workspace and fork compile/activate/rebase/Undo coordinators still require their existing kernel/extension authorities. The four source files `theme-state.ts`, `workspace-appearance.ts`, `rebase.ts`, `update-fork.ts` have no tag-to-main content delta; that does not establish native parity. |
| A09 | `session` rejects delayed authorization after disconnect/document replacement, permits fresh session recovery, serializes finish and checks panel registration generation between capture/input. Already-open media leases are revoked on session loss. | Real socket/application/session recovery and durable publication hosts, actual panel generations and app restart races. Document owner is the kernel actor, not a WebKit panel: panel loss need not kill the native document. |
| A10 | `review` freezes original provider/settings, keeps executed edits and original errors, bounds repairs to two one-use passes, retains frames/structured review without inventing a visual-success flag. | Real actor source-edit/history integration, frame capture, review provider and pinned-source structured-response orchestration. Applied edits survive review/capture failure. |

### Host contracts and safety decisions

`memory::Host` is an adapter on the existing kernel single writer. Its snapshot
identity includes document, generation and revision; publication must compare
both identity and original metadata bytes. A memory-only mutation must autosave
without entering Undo or advancing project revision. Disabled memory remains
stored and is omitted from agent-facing indexes. Unknown active-record fields
survive; removed, evicted, replaced and malformed raw records enter
`memoryArchive`. Unsupported archive shape fails unchanged. **N3** selects
main's authored memory semantics; **N4** selects retention over source trimming.

`registration::Host::commit` must atomically compare reviewed config bytes,
archive the complete predecessor/CLI receipt, publish/fsync/read back, and record
Claude Desktop intentional disconnect in the same recoverable transaction.
Plans contain no launcher JavaScript. Raw config bytes are omitted from Debug.
A `ConsentGate` creates a bounded request and accepts an explicit response;
its non-cloneable permit is consumed once and only for that reviewed plan.
No startup auto-connect or persisted default approval exists. The owner's
explicit Claude Desktop decision selects this gate; **N1/N4** select exact-plan
approval and recoverable configuration preservation. OpenCode unknown fields
are conservatively treated as custom; unreadable client config is reported as
unreadable rather than falsely missing (**N3/N4**). Already-identical settings
are a no-op.

`compatible::Host` owns real HTTP/Keychain/config IO on workers. Request contracts
include redirect refusal, bounded responses and per-call timeout. Config publish
must archive original bytes and keys before replacement, reject a stale
snapshot, and retain address-scoped keys rather than garbage-collecting them.
The service only reports capability/context from supplied measured responses;
unknown tool support remains unknown (**N1/N3/N4**). Synthetic source recordings
are not a live-provider capability claim.

The typed process `Handle::try_delivery` consumes the same bounded mailbox as
`try_event`; the application chooses one drain API. Callback and inspection
variants preserve every event and arrival order. The actor queue must admit
whole effects atomically, then durably receipt events before acknowledgement.
The legacy raw API remains for existing consumers (**N4/N7**, prune rule).
Complete-observation strict replay still includes inspection additions.

Session ownership and panel registration are separate authorities. A delayed
prompt/call must carry the original owner/lease; finish is single-flight, new
calls reject while finishing, and owner loss rolls back a pending transaction
without claiming to close a surviving external connection. A committed receipt
survives later Stop. The original failure remains separate from cleanup failure
(**N3/N4/N7**). This ports authored authority fences without reproducing a
Chromium renderer-reload quirk. Export terminal results remain available until
archive acknowledgement succeeds (**N2/N4**).

All public input bounds fail visibly. Text truncation preserves Unicode scalar
boundaries rather than manufacturing broken UTF-16 surrogate data (**N3**).
No parser output or model review claims that native pixels were verified.
No Node/Chromium dependency was added to the shipped Rust crate; Node is used
only to regenerate source-reference evidence (**N8**).

### Source-derived evidence

`fixtures/main/contracts.json` and `provider.json` pin main source hashes and
recorder hashes. Recorders execute actual source with synthetic input/IO and
refuse an unexpected import or TypeScript version. The contracts recorder
captures 18 connect/disconnect operations across nine clients, six Claude
permission configurations, memory, URLs/presets, grouping, media, visuals and
markdown. It runs all 21 source markdown tests with active assertions and records
38 block inputs, 34 inline inputs and three reveal-word inputs. Production client
and preset metadata compare with executed source output.

The provider recorder captures ten setup/model/error cases and exact method,
URL, body, key-binding and timeout observations, plus seven completion-stream
cases including split multibyte input, split tool arguments, arrival order and
missing termination. No private account, real key or provider is used. Unobserved
imports are explicitly injected; the six Claude comparisons cover permission
flags, not the injected instruction/tool-catalog dependencies. Reference bytes
are never generated from Rust output.

Regeneration needs Node and TypeScript 5.9.3 in `work/oracle/node_modules` (install
with a local scratch npm prefix); primary replay also needs Zod 4.6.5. Ordinary
Rust tests use committed fixtures, helpers and fetched source objects; they do
not need those scratch dependencies or any file in spikes/vendor.

```sh
node apps/native/crates/agents/scripts/capture-main-contracts.cjs --check
node apps/native/crates/agents/scripts/capture-main-provider.cjs --check
node apps/native/crates/agents/scripts/capture-main-replay.cjs --check
cd apps/native
crates/agents/scripts/check.sh
PM_REPLAY_REQUIRE_IDENTICAL=1 cargo test --locked -p pm-agents --test replay replay_all_surfaces_and_check_every_difference -- --exact
```

Full main strict identity remains an independent **failing gate**: 336 exact leaf
differences, comprising 28 retained native lifecycle observations, 304 MCP catalog
leaves (including array shifts from the missing `project_memory` insertion) and
four MCP error leaves. All 21 provider public callbacks match; this does not turn
the complete-observation gate green. The exact ledger detects drift and is not
a waiver. Original tagged fixtures remain mandatory legacy evidence (**N3/N4**).

### Remaining bindings, design and live verification

MCP/kernel/application bindings above live outside the original owned paths.
An ownership clarification was requested; this lane has not edited those paths
or claimed those tools as implemented. Actual ordinary-thread tools, task_send,
child question forwarding, automatic durable continuations and agent/panel tool
hosts listed in `agents.md` remain open. `project_memory` schema/body and actor
metadata publication must be integrated together, without a second writer.

**DESIGN-TODO(opus):** bind the typed registration consent request to the revised
S070 presenter, including explicit Claude Desktop consent. Use existing design
specs/tokens for markdown, media, visual-reply and conversation views and obtain
Opus acceptance. This lane supplies semantic data/authority only and makes no
new layout, copy, spacing, material or interaction choice. All visuals remain
provisional.

On macOS verify signed executable/CLI discovery, every real client format,
config archives and consent denial/approval/stale-plan races; Keychain and HTTP
cancellation; native media grant/range/player behavior; opaque WebKit CSP and
message isolation; actual kernel autosave without Undo/revision change; export
progress/deadline enforcement; actor publication and session/panel generation
races through process loss/restart. With credentials run pinned Codex and Claude
questions, approvals/classifier, cancellation/timeout, concurrent runs,
reconnect, history/compaction, orchestration/watch delivery and source-edit
review. Linux fixture success certifies none of these live/platform behaviors.
