# Agent panel model (Wave 4 head start)

Package: `pm-panel-agentpanel`, at `apps/native/crates/panel-agentpanel`.
Scope: **MODEL ONLY**, pure Rust, Linux-testable. No GPUI, WebKit, V8,
Node, Chromium, renderer, colours, metrics, layout, typography or new display copy.
Node is used only by the optional frozen-source oracle generator.

Reference: **v1.2.0**, commit
`07ed5fa17ee640659cb075f9cad1af2095011d60`. This is a source-derived model,
not a claim of full frozen-Electron execution or macOS visual parity. The v4
atomic-op API is not frozen or redefined by this crate. Existing extensions,
registrations and Electron consumers are untouched.

Contracts read: `p0-decisions.md`, `w1-workflows.md` W024–W030,
`w1-keymap.md` K003/K004 and focus-local routes, `c-matrix.md`,
`kernel.md`, `mcp.md`, relevant agent rows of `d-state.md`, `traces.md`,
`first-launch.md`, `engine-service.md`, `s1-v8.md`, `s5-compat.md`, and
`design/language.md` §12. Extension logic remains V8; WebKit remains the
remote-DOM panel renderer. This model does not move extension logic into a view.

## Ownership and data flow

- `composer`: inline draft, immutable attachment records, UTF-16 selection,
  token moves, clipboard and local Undo/Redo. Text/newlines and all original
  encoded payload/unknown fields survive. The model never reads a user file.
- `input` / `interaction`: local key routes and slash choices call the same
  commands exposed to headless clients. `NativeText` and `NativePaste` remain
  explicit host actions for IME and the native pasteboard. The global Agent
  chord is a predicate; existing registry arbitration stays with its owner.
- `activity`: immutable `Arc<Value>` rows indexed by id, provider trace reducer,
  semantic work groups, placement-protocol filtering and media outcomes.
  `Rows::page` returns a borrowed slice; no full list serialization is required.
  `TranscriptIndex` pages individual **archived steps**, including 60k steps
  nested inside a single trace message. Its scoped ids distinguish legacy
  duplicate step ids. Build snapshot indexes on a worker, retain their matching
  row publication, then resolve only the requested window.
- `questions`: explicit option/typed-answer drafts and masked acknowledgements.
  No option is preselected and choosing an option never submits. Secret text is
  absent from Debug, inspection, transcripts and persisted acknowledgements.
- `model`: one project-scoped `Panel`, multiple thread-scoped `Session`s,
  semantic commands, fenced host effects and effect acknowledgements. Ordinary
  background launches and sends preserve the visible thread, draft, selection,
  attachments and draft Undo. Completed text stays above follow-ups. Result
  artifact references are archived under `nativeResult` before controls clear.
- `view`: thread picker, connection generations, reading/follow policy and
  visual-reply interaction state. It exposes facts, not view geometry.
- `kernel`: real `KernelHandle` publications, query mirror, selection,
  revision-guarded typed transactions, normal Undo/Redo, and guarded result undo.
  Calls are futures. Production code never calls `wait`/`call_blocking`.

A model command produces an `Effect { id, target, kind }`. Target carries the
project, thread and optional run id; effect ids are unique across model instances
in a process. Execute effects on the appropriate host worker, then acknowledge
that exact effect id. Provider updates require the exact thread/run id and a
contiguous sequence number: duplicates are idempotent; gaps and stale runs are
explicit errors. A switch of the visible thread does not change effect ownership.
`Panel::inspect` exposes a small machine-readable state without attachment bytes
or private answer drafts. Row ids/ranges, `send_message`, `launch_background`,
question ids and kernel layer ids provide headless/agent/MCP entry points.

The `pm-agents::persistence::Threads` archive remains the lossless storage
representation. `Panel::load`/`save` use its `Store` trait **on a worker**, supplied
by the kernel storage owner; this crate opens no profile or second library writer.
Attachments are hydrated for editing, detached before archive writes, and written
before the referencing archive. Missing bodies retain their metadata and
`payloadStored`; sending refuses a missing body without consuming the draft.
Unknown/malformed input is refused without a write or silent reset. The document
can still open independently; the host must report a panel storage error and keep
its input available. The inherited 32 MiB logical-store limit remains explicit:
a failed save preserves the in-memory model and reports `save_error` for retry.
The host must not discard an unsaved/failed Panel on close; retry or retain a
durable archive through the storage owner first.

## Behaviour inventory

Every location below is at **v1.2.0**. To avoid ambiguous abbreviations:

- `A/` = `apps/desktop/src/renderer/src/panels/agent/`.
- `P/` = `apps/desktop/src/renderer/src/panels/`.
- `S` = `apps/desktop/src/renderer/src/legacy/assistant/spatial.ts`.
- `M/` = `apps/desktop/src/shared/`.

For example, `A/Composer.svelte:106` means
`git show v1.2.0:apps/desktop/src/renderer/src/panels/agent/Composer.svelte`, line 106.
“Done” below means the headless data/behaviour/command contract. “Effect” means
model behaviour is implemented/tested, but the platform/runtime body remains
outside this owned lane; it does not report successful work by itself.

| v1.2.0 behaviour and source line | Native mapping / status | Headless evidence |
|---|---|---|
| Composer Enter sends; Shift+Enter newline; repeat/IME cannot send. `A/Composer.svelte:106–142` | Done: `composer_key`, `Submission`, `Draft` | `composer_keys_never_leak_into_editor_commands`, `enter_sends_once_repeat_and_ime_keep_draft` |
| Local Cmd/Ctrl+A, Cmd/Ctrl+V, Escape; outside primary/auxiliary click releases field. `A/Composer.svelte:108,125,130,185` | Done: local routes, native-paste route, shell-close effect | `local_primary_chords_and_native_paste_keep_exact_modifier_rules`, `outside_primary_and_auxiliary_clicks_release_composer_focus` |
| Local draft Undo/Redo, 600 ms typing groups, 100 entries. `A/inline-prompt.ts:196–225` | Done: attachment-preserving local history | Composer history/redo-branch tests; kernel history remains separate |
| Exact newlines/caret; platform word/line deletion range. `A/inline-prompt.ts:46,262–296` | Done: UTF-16 ranges; no DOM sentinels/placeholder breaks | Unicode/surrogate and native deletion tests |
| File-only draft; inline offsets and request markers. `A/inline-prompt.ts:46–73` | Done: original records plus inline tokens | UTF-16 offset/order and file-only tests |
| Two-step Backspace/Delete; repeats do not remove token; pointer disarms. `A/inline-prompt.ts:214–259,299` | Done | Two-step, forward-delete, repeat and disarm tests |
| Copy/cut/paste with rich local tokens and plain-text fallback; duplicate ids move once. `A/inline-prompt.ts:300–353` | Done; native pasteboard effect route | Clipboard move, cut, malformed clipboard and selected-token preservation tests |
| Attachment token drag/drop; insertion at captured caret. `A/inline-prompt.ts:355–368` | Done: `move_file`; native hit-test supplies offset | `attachment_drag_preserves_utf16_text_and_is_one_undo` |
| Picker/paste/drop image/binary/empty files, max six new attachments. `A/Composer.svelte:145–179`; `S:1725–1738`; `A/attachments.ts:18–42` | Done records/admission; Effect for OS picker/file reading | Delayed-picker thread fence, byte/empty-file/limit tests; imported larger lists remain readable |
| Image preview versus source/materialized file reveal, restore opener focus. `A/attachments.ts:45–105` | Effect: captured attachment record and opener id | `attachment_actions_return_preview_or_reveal_and_restore_focus_by_id`; format classification tests |
| Slash filtering, dynamic model/provider/effort choices, custom Claude ids. `A/slash-commands.ts:11–45` | Done; provider/catalog change is a host effect | 78 executions of tagged source compared with Rust |
| Slash Up/Down wraps, Enter/Tab/pointer chooses, Escape dismisses until edit. `A/Composer.svelte:43–59,114–123` | Done; settings/files/new-thread actions are explicit effects | Slash route, wrap, dismissal and full key-to-effect tests |
| Global Agent command Cmd/Ctrl+Shift+K; loose modifiers; no fields/repeat. `apps/desktop/src/extensions/keymap-default/keymap.ts:25` | Done predicate + Open effect; global registry wiring external | `agent_shortcut_keeps_loose_modifiers_but_declines_fields_and_repeat` |
| New/switch threads, reuse blank UI thread, apply blocks switching. `S:846–865`; `A/threads.ts:171–194` | Done | Blank reuse, background continuity and apply-block tests |
| Thread search, first result on search Enter, arrows, focused Enter/Space, Escape/Tab/outside dismissal. `A/ThreadPicker.svelte:41,105–119,162–215` | Done semantic picker and arbitrary row windows | Picker key/pointer/search tests; 60,001-thread window test |
| Confirm delete and stop running thread. `A/ThreadPicker.svelte:190–203`; `S:828–843` | Done with N4 archive policy; cancellation must settle first | Confirm/archive payload retention and final-output-before-archive tests |
| Drafts, attachments, titles, opaque thread/resume metadata survive switch/restart. `A/threads.ts:139–239`; `S:697–727` | Done via shared archive; generated-title callback is guarded by original request | Persistence, opaque preference, generated-title and failure/retry tests |
| Ordinary background `thread_launch` / `thread_send` do not steal visible chat. `S:556–619`; `apps/desktop/src/main/agent-tools/orchestration-spec.ts:52–64` | Done model commands; MCP RPC registration external | Background blank-thread, inherited-access and preserved-draft/selection/Undo tests |
| Streaming text updates in place; thoughts settle before prose/tools. `S:1785–1895`; `A/agent-state.svelte.ts:219–245` | Done ids, immutable row publications, sequenced events | Stable-row/duplicate/gap tests and full trace retention tests |
| Steering retains reasoning/work before the next user message; later tool/task ends find the archived row. `S:1954–1985,2708–2718` | Done | Steering checkpoint and archived-task Stop tests |
| Only the newest active group is emphasized; prose/reasoning/questions break consecutive tool groups. `A/activity-rows.ts:204–335` | Done semantic group counts/status/newest-active id; no animation chosen | Mixed-failure group test, row/index tests |
| Tool/thought disclosure defaults and manual override; subagent Stop/result/errors. `A/ToolActivity.svelte:22–62`; `A/task-trace.ts:5–23` | Done semantic state; task Stop effect | Subagent in-place result and stop/double-stop/error retry tests |
| Nonfinal transcription is pending/needs-model, not a completed transcript. `M/media-tools.ts:272–299` | Done semantic outcome/progress; view supplies copy | `media_nonfinal_outcomes_keep_truthful_status_and_full_original_output` |
| Multiple/single choices, optional typed answer, explicit Answer/Skip; no implicit option submission. `A/QuestionCard.svelte:17–45` | Done | Multi/single toggle, typed combination, empty-answer/skip tests |
| Reply questions use run request fallback; composer text answers a blocking question. `S:1836–1847,1917–1951,2699–2706` | Done: fenced Answer effect; masks secret acknowledgement | Blocking-composer, missing-request-id fallback, masked persistence and late-delivery tests |
| Nonblocking/message questions use follow-up/steering and preserve composer; secret answers never become follow-up prose. `S:1940–1951` | Done: ordinary Send effect, current/new run as needed | Message-question active/idle and private-answer tests |
| Working elapsed time survives steering/background switches; progress capped at 20. `A/agent-state.svelte.ts:317–394` | Done semantic timestamps/history | Stable timer, bounded progress and background tests |
| Stop retains prior and partial replies; rejected Stop can retry. `S:1762–1776` | Done with explicit pending cancellation; Effect invokes runtime | Stop identity/double-stop/retry, completion and archive-after-settlement tests |
| Selection suppresses automatic following; jump/latest and thread switch reset following. `P/AgentPanel.svelte:121–157` | Done semantic follow policy; reviewed host measures thresholds | Transcript selection/follow and independent-stream tests |
| Provider connection checking/retry gate, old responses ignored. `P/AgentPanel.svelte:85–118,161–185` | Done connection generations; connect/retry/settings effect request | Connection race, busy and retry gate tests |
| Provider/model/effort/access and panel focus choices. `S:1002–1046,1702–1722`; `A/panel-focus.ts:24–40` | Done validation, consent/revoke effects, focus normalization; provider capability catalog injected | Model/provider, consent, 32-id focus/dedup and persisted-intent tests |
| Plan apply/dismiss and auto-apply toggle; keep/guarded undo results. `A/PlanPreview.svelte:62–74`; `A/ResultActions.svelte:56–113` | Done admission + effects; project commands execute through `ProjectModel`; external workspace/extension undo adapter external | Apply failure/retry, preserved plan, kernel guarded-undo/interleaving tests |
| Result files reveal/import, duplicate import prevention, retain durable references. `A/ResultActions.svelte:25–41,90–105` | Effect; metadata/idempotency/archive done | Artifact import retry/duplicate and keep/restart artifact-reference tests |
| Metadata fragments do not enter prose or reasoning; only verified live ids can place work. `A/ui-placement.ts:18–92`; `S:1803–1835` | Done protocol splitter/parser; placement stays semantic and transient | 18 source executions, fragmented-stream and target validation tests |
| Open/close/pin/floating grip drag/cancel/dock keep the same conversation. `P/AgentPanel.svelte:191–197,231–235`; `P/agent-shell.ts:139–277` | Effect: `ShellAction`; layout/drag coordinator external | Shell effect ownership/continuity test; actual shell replay is not done |
| Visual reply expand/close/focus/zoom/replay. `A/VisualReply.svelte:50–73,89–112` | Done interaction state; HTML validation/isolation/host body not done here | Visual state bounds/reset test; sandbox/focus proofs require macOS host |
| Rich Markdown/native text selection, code/link interaction, native text context menus. `A/Conversation.svelte:52–65`; `A/Markdown.svelte:1`; `A/TextRow.svelte:1` | Not done: renderer/platform text layer. Exact prose/records are preserved | No visual parity or native text-context-menu claim |

There is no shipped agent-row scrubber or timeline/layer multi-selection gesture
to invent. Multi-select here means question choices and native draft/transcript
text selection. Standard text context-menu operations use the same Draft
clipboard/selection/Undo commands. The header/dock drag coordinator and native
text hit-testing are external host responsibilities.

## Decisions and selecting rules

| Decision | Rule and reason |
|---|---|
| Reuse `pm-agents` storage/provider types and the kernel actor, not a panel-owned document/profile writer. | N3/N4/N6: one authority, preserve exact archive and compatibility shapes. |
| Keep the v1.2.0 command/shortcut semantics, including loose Agent modifiers, blocking-question submit, rich token moves and native paste. | N2/N3: authored actions and local focus semantics survive the rewrite. |
| Model effects never fabricate success, files, cancellations, approvals or project edits. | N3/N7: inspectable pending work, exact target ids, explicit acknowledgements. |
| Main-thread commands return futures/effects; synchronous Store operations and snapshot-index construction are worker work. | N1/N8: no disk/provider/JS wait in input handling. |
| Require a host document-lifecycle lease for kernel commands. | N3/N4: revision equality alone cannot distinguish close/reopen; preserve work until the actor acknowledges. |
| Result-specific undo uses actor-side revision/document-guarded Restore, not an unguarded Undo of the current top entry. | N3/N4: interleaved foreign work is retained; ordinary user Undo still uses normal history. |
| Archive confirmed thread removals, including payloads and unknown fields. | N4 overrides the tag's permanent removal; no deletion or garbage collection is introduced. |
| Stop and archive expose pending cancellation until the run settles; accept its final sequenced output meanwhile. | N3/N4: do not discard acknowledged work or report stopped work before a host result. This is an explicit timing difference from the tag's immediate local abort. |
| Keep all trace/output bytes, with windowed access instead of 200-step/2,000-thought/6,000-text/600-output data deletion. | N1/N2/N4: large professional sessions remain inspectable; display excerpting belongs to the view. |
| Preserve original UTF-16 intent and intentional line breaks; do not synthesize zero-width sentinels or Chromium placeholder `<br>`s. | N3 quirk policy: authored text wins over implementation artifacts. Invalid split-surrogate ranges are refused. |
| A malformed/unknown archive or missing body is explicit; never silently reset, discard input or send a filename as bytes. | N3/N4: retain recoverable data and report the missing operation. |
| Saved computer access is retained as intent but not reactivated; fresh UI/child/background threads do not inherit a live computer grant. | N4/N6: preserve the recorded contract without increasing authority. |
| Secret question drafts/transport answers are transient, Debug is redacted, durable acknowledgement is masked. | N3/N6: reproduce `shownAnswers` and private follow-up behavior. |
| Preserve result artifact references beyond their transient result controls. | N2/N4: the next prompt or Keep cannot strand the only path to generated work. |
| Reject stale/gapped provider updates and unknown tool-end ids explicitly instead of silently continuing. | N3/N7: providers/automation can inspect and recover an incomplete stream; no successful no-op placeholder. |
| Keep legacy DOM selectors as bounded protocol data, with no native selector interpretation. | N3/N6: preserve intent until the v4 control-id mapping is specified; never infer a workspace edit. |
| No replacement consumers removed and no shared files edited. | Prune rule, ownership constraint, N4/N6. |

## Integration still required (outside this lane)

1. **Shared Cargo.lock entry.** The `crates/*` glob already discovers the package,
   but ownership excludes editing `apps/native/Cargo.lock`. The copied verification
   workspace generated only this additional package stanza (no dependency upgrades):

   ```toml
   [[package]]
   name = "pm-panel-agentpanel"
   version = "0.1.0"
   dependencies = [
    "pm-agents",
    "pm-kernel",
    "pm-model",
    "serde_json",
   ]
   ```

   The integration lane must add/regenerate this entry before the original
   checkout can run `cargo test --locked -p pm-panel-agentpanel`.
2. **Kernel storage adapter and lifecycle fencing.** Supply `Store` on the existing
   storage owner's worker; do not open another `LibraryWriter`. Supply a real
   `DocumentLease` that prevents close/reopen/rebind until its value is dropped
   after acknowledgement. Other document writers and gesture folding must remain
   allowed. Existing `Call::Transact` checks revision, not document identity;
   alternatively its owner may add an atomic document-id admission guard. The
   Linux lease double proves adapter behavior, not a production shell lease.
3. **Provider worker/controller.** Route Send/Steer/Answer/Stop/task Stop to existing
   `pm-agents`, feed contiguous run-scoped events back, and deliver explicit
   failures. Inject the live provider/model/effort-capability catalog and defaults,
   persist global provider preferences through the kernel settings API, perform
   generated-title requests, and supply connection statuses. No credentials or
   account flow live in this model. Keep one Panel per project/app context so a
   project-tab switch does not destroy live sessions.
4. **Host effects.** Map file/paste/drop reading to immutable attachment records;
   fulfill preview/reveal/import, native consent, settings and shell/drag effects.
   Apply JSON plans using the proper typed kernel command adapter; workspace and
   app-extension changes still require their own external-history/undo service.
   Feed kernel document/revision changes into preview invalidation before enabling
   Apply: stale plans must never be rebound to a new document. The kernel adapter
   rejects stale writes, but this model does not watch shell lifecycle itself.
5. **MCP/v4/UI integration.** Register the id-based methods with the owning MCP
   surface controller after v4 freezes. No public MCP tool schema is changed by
   this crate. Add accessibility/text input, row measurement/window requests,
   scroll observations, token hit-testing and actual focus restoration. Implement
   the renderer/Markdown and isolated visual-reply host; no HTML executes here.

`DESIGN-TODO(opus)`: the neutral placeholders are semantic state/effects only.
Specify/review the v4 control/disclosure/focus map, placement target ids and
loading representation, file/visual preview/error representation, and reconcile
shipped popover actions with `design/language.md` §12's native docked surface.
Use existing pm-design tokens when a visual layer is built. There is deliberately
no fallback layout, invented message copy or styling in this crate. All future
visual output remains provisional until Opus review.

## Verification

Linux, pinned Rust **1.96.1**. To respect shared-file ownership, verification uses
a disposable workspace copy with the exact owned sources, native/engine
dependencies, trace fixtures and read-only Git context. Its lock differs only by
the package stanza above; no dependency versions changed.

Commands in that copied `apps/native` workspace:

```sh
cargo test --locked -p pm-panel-agentpanel
cargo clippy --locked -p pm-panel-agentpanel --all-targets --no-deps -- -D warnings
cargo test --locked -p pm-kernel -p pm-agents
```

The final run passes **100 panel tests** (16 composer, 15 input/view, 9 interaction,
7 real-kernel, 50 model and 3 placement), strict clippy with **zero warnings**,
and package-scoped `cargo fmt --check`. Existing dependency regression results:
**95 pm-agents + 273 pm-kernel = 368 passed**, with **one pre-existing ignored
kernel test**. Both requested staged Gitleaks scans and the repository hook pass.
The original checkout was also tested with `--locked`: it stops before compiling
because the unowned shared lock lacks the stanza above, as expected.

The frozen-source generator is `node apps/native/crates/panel-agentpanel/scripts/tag-oracle.mjs`:
it pins the tag's commit, records source SHA-256s and executes the actual pure
TypeScript functions. It supplies **78 slash + 18 placement = 96 source cases**;
these are not Electron UI captures. The checked-in fixtures allow Rust-only runs.

macOS still must verify native IME/pasteboard/text menus, accessibility/focus,
image/visual-reply isolation, real provider/consent/file/shell effects, lifecycle
lease ordering during close/reopen, 60k-row scrolling/input latency and the
existing physical viewer/playhead guarantees. This model does not move the
playhead, render a frame, or certify a platform performance/visual gate.
