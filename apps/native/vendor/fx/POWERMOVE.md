# Powermove fork of fx (libfx core)

Upstream: https://github.com/vercel-labs/fx
Pinned: tag `v0.0.18`, commit `6196071b3` (2026-11-25; re-pinned from the
pre-release `8fbe9b8` with `tools/sync.sh`, every patch merged unchanged).
License: Apache-2.0, retained in `LICENSE`; upstream's third-party notices in
`THIRD_PARTY_NOTICES.md`. Every file we changed or added carries a
"Powermove notice"; nothing here has been submitted upstream.

This is Aero's agent core (design: Aero Phase 1, section 2). Only the closure
that the C ABI surface compiles is vendored, so the tree is libfx's headless
core and nothing else.

## What is here

| Path | What |
|---|---|
| `src/` | The closure of upstream's `src/` that `zig build capi` reads, plus our files (below). |
| `src/capi_core_main.zig` | Powermove code (P-CAPI, P-PANIC, P-TOOLS): the C entry, its generation-tagged slot table, the guarded host calls and panic handler, the tool bridge and the fetch bridge's host side. |
| `src/core/shared/panic_context.zig`, `src/core/tooling/host_tool_bridge.zig` | Powermove code: the runtime a core thread works for (P-PANIC), and the host tool executor hook (P-TOOLS). |
| `include/fx_core.h` | The C ABI, version 2 (patch P-CAPI): threads, liveness, leases, ownership, copies, sizes and closed/failed behaviour per function. |
| `build.zig`, `build.zig.zon` | Trimmed build: the `capi` static library and the `fake-server` test binary only. |
| `test-support/fake-inference-server.zig` | Upstream's `benchmarks/libfx/fake-inference-server.zig`, verbatim: the deterministic fake model for tests and benchmarks. |
| `toolchain.toml` | The pinned Zig (0.17.1) with the tarball's sha256 from ziglang.org's index. |
| `install-zig.sh` | Downloads that tarball, checks the sha256, unpacks it into `$PM_ZIG_HOME`. |
| `tools/sync.sh` | Re-pins to another upstream commit: our patch series merged three ways onto it, then the closure re-derived (below). |
| `tools/derive-closure.sh`, `tools/closure.txt` | Derives the closure from the build graph and refreshes `src/` to it. |
| `tools/extract-core-runtime.py` | Copies the N-API entry's runtime pieces verbatim into `src/libfx_core_runtime.zig`. |

Omitted from upstream: the fx CLI's and terminal UI's entry points
(`src/main.zig` and the files only they reach), the WebAssembly and Node-API
entries, the SDK JavaScript (`sdk/`), benchmarks, examples, e2e and TLA+
tests, fuzzing and PGO build steps, scripts and CI. The closure still holds
85 of its 722 upstream files under `src/ui/`: shared code `@import`s them, and Zig parses every
file an import names, so they are on disk though no C ABI path runs them. The TLA+ specs stay upstream
(`tests/tla/sdk/*.tla`); they are the steering, journal and image contract our
ABI keeps (section 2.7).

## Building

`apps/native/crates/aero-core/build.rs` builds this tree: it reads the pin from
`toolchain.toml`, uses exactly that Zig from
`$PM_ZIG_HOME/zig-aarch64-macos-<version>/zig` (default
`~/.cache/powermove/zig`), checks `zig version`, and runs
`zig build capi fake-server -Dtarget=aarch64-macos.13.0` with its local cache
and install prefix in cargo's `OUT_DIR` and `ZIG_GLOBAL_CACHE_DIR` at
`$PM_ZIG_HOME/zig-cache`. The library is always ReleaseSafe (forced in
`build.zig`, as upstream forces it for its N-API addon). **The build never
downloads.** A person or CI installs the pinned Zig once:

    apps/native/vendor/fx/install-zig.sh

Without that Zig (or on a target other than aarch64-apple-darwin), `build.rs`
builds a stub (`cfg(aero_core = "unavailable")`) whose every call returns
`Unavailable`, with one cargo warning, so the workspace and pm-gate build on a
machine without Zig. The stub is Rust only (`crates/aero-core/src/sys.rs`):
there is no C stub library, so a C host linking `fx_core.h` needs the real
core. `build.rs` tracks the resolved Zig path whether or not it exists, so
installing the pinned Zig into the same place after a stub build rebuilds the
real core. Platform code in the wrapper is gated (the main-thread check uses
`pthread_main_np` on Apple targets, `gettid` on Linux, nothing elsewhere; the
ready fd is `cfg(unix)`).

## Static link

`libfx_core.a` is one object (Zig compiles the whole module as one unit). Its
only global definitions are the 25 `fx_*` functions of `fx_core.h`; every
other symbol is local (the panic guard is naked Zig, not a C file, so it adds
no symbol). Zig's compiler-rt and UBSan runtime are not bundled
(`bundle_compiler_rt = false`): the archive's undefined builtins (`__divti3`,
`__udivti3`, the `f128` helpers such as `__multf3` and `__getf2`) resolve
against Rust's `compiler_builtins` and libSystem at the final link, so no
symbol can be defined twice. Checked with `nm -gU` (defined globals) and
`nm -gu` (undefined) on the archive; the link of the crate's tests proves the
undefined ones resolve.

## Closure

`tools/derive-closure.sh <upstream clone> <commit>` exports upstream's `src/`
at the commit, lays our files over it, builds `capi` there with a fresh cache,
and reads the compile's cache manifest, which lists every source file the
compiler read. Zig parses every file an `@import` names, whether or not its
code is used, so that list is exactly what the build needs on disk. The script
writes its upstream files (the 11 we patched included) to `tools/closure.txt` and makes `src/` exactly the closure plus our
files; a file with a notice is never overwritten. Every manifest of the fresh cache that names the C entry is read, in
sorted order, and the script fails if they list different sources. Building the crate from the pruned tree proves it complete.

## Powermove patches

| Patch | What and why | Status / upstream intent |
|---|---|---|
| P-CAPI | A C ABI entry beside the N-API and Wasm entries: `src/capi_core_main.zig` implements `include/fx_core.h` (ABI 2). It reuses the N-API entry's runtime, input queue, attachment table and fetch bridge, extracted verbatim by `tools/extract-core-runtime.py` into `src/libfx_core_runtime.zig` (new file; upstream's `napi_core_main.zig` is not vendored), and replaces the Node-API surface. Handles are generation-tagged `u64`s over 64 slots (40-bit generations; an exhausted slot is retired for good; occupancy, generation and pointer published with release, entered only when occupied). Host calls take the locks core threads share only with a bounded try-lock that gives up when the runtime panics. Sessions and turns still travel as ACP frames; what a turn streams comes back through the typed sink (P-EVENTS). TURN_END is queued and the turn ended in one critical section, so a host that drained it can prompt at once. Milestone 2b-1 (Sol's 2a re-check): a slot is reserved before create starts anything; `fx_turn_cancel` takes every lock before it changes anything and is linearised against fetch publication (the core publishes under the runtime lock, recording the fetch's turn under the bridge lock; a fetch for a cancelled turn is published retired); `fx_fetch_disposition_get` answers `FX_FETCH_BUSY` for contention, never RETIRED; a failed take or drain builds in spare storage and keeps the previous borrows; `fx_journal_ack`/`_fail` queue the barriers' answers in one write before they consume anything; `fx_turn_prompt` reserves its TURN_START before the turn starts. | Milestone 2b-1: implemented; tests green. Proposable upstream as a C ABI (needs the Chief's say). |
| P-PANIC | The C ABI root's own panic handler (`pub const panic` in `capi_core_main.zig`). A panic on a core thread records its message in the runtime's fixed buffer, marks it PANICKED with one atomic store, wakes the ready fd and parks the thread; the runtime's other threads park at their next boundary with the host, so no callback starts after. A panic inside a host call unwinds, through a naked-asm guard around every export, back to that export, which returns `FX_E_FAILED`: the host thread is never parked. The core's threads carry their runtime in a thread-local (`core/shared/panic_context.zig`), inherited at the four spawn sites a runtime uses: `acp/server.zig` (prompt worker), `core/agent/runtime/parallel_execution.zig` (parallel tool workers), `core/agent/runtime/request_recovery.zig` (status ticker) and `core/agent/runtime/cache_warmer.zig` (its `std.Io` task). A panic on a core thread with no runtime (none is known) fails every running runtime rather than leaving one stalled. A panicked runtime is never joined or freed. A transport failure the C entry publishes (FAILED) is published and wakes the host before the input is closed, and closing it never waits on a parked holder. A panic inside `fx_runtime_create` issues no handle and gives its slot back (the runtime is abandoned); one inside `fx_runtime_destroy` still retires the handle. | Milestone 2b-1: implemented, fault-injected at nine panic points (`fx_test_arm_fault`, the header's `FX_FAULT_*`), plus one injected transport failure and three injected allocation failures. Not upstream material. |
| P-TOOLS | Host tool calls reach the host directly: `acp/prompt.zig`'s `callHostTool` calls `Config.host_tool_bridge` (`core/cli/acp_runner.zig`, new field) when the embedder installed one, so the host gets the core's own copy of the model's input with no JSON-RPC frame and no further copy, pinned until the host answers (the lease). The answer's text is copied once, into the call's record, and becomes the tool result without another copy. Concurrency: up to `max_concurrent_tools` idempotent calls, a writes tool alone. Milestone 1 instead copied the input out of the `libfx/tool_call` frame and sent the result back through a JSON frame. | Milestone 2a: implemented. Upstream intent: an executor hook is reasonable to propose. |
| P-STEPS | The step limit's cause travels typed: the exhaustion branch in `core/agent/runtime/orchestrator.zig` sets `TurnFinalizationGuard.end_cause` (`core/agent/runtime/finalization.zig`), the guard hands it to the new optional `AgentRuntimeDeps.note_turn_end_cause` (`core/agent/runtime/deps.zig`) before `finalize_turn`, and `acp/prompt.zig` then ends that turn as ACP `max_model_turns` instead of `refused`. The C entry maps it to `FX_STOP_MAX_STEPS`. Milestone 2a matched the notice text in the reply, which a model quoting it could fake. | Milestone 2b-1: implemented. Upstream intent: reasonable to propose (ACP already has `max_model_turns`, nothing set it). |
| P-EVENTS | A typed sink beside the ACP writer (`core/hosts/host_event_sink.zig`, new; `Config.host_event_sink` in `core/cli/acp_runner.zig`). Where `acp/prompt.zig` and `acp/sessions.zig` would build a `session/update` for a reply or reasoning delta, a tool card's start or end, context usage or a joined steer, they hand the values to the sink; the C entry queues them as typed events (one copy into its event arena) and drops the session/update frames left (notices) without parsing them. Tool card ends carry the tool's name. | Milestone 2b-2: implemented; `tests/turns.rs`. Fix round 1 (Sol's m2 item 7): the C entry no longer drops reply text by a `[context]` prefix; `acp/prompt.zig`'s `pushContextNotice` keeps the core's context notices out of the sink at their source, so model text that starts with `[context]` streams intact (`tests/turns.rs`). Upstream intent: worth proposing with the C ABI. |
| P-IMAGES | Images as raw bytes, never base64 on the host's side: prompt images cross through the attachment table (image blocks name `_meta.fx.attachment`, and `stored` with an image store); a tool result's images are base64-encoded once by the C entry into the core's own `ToolImage` form; a steer with images never joins (`libfx/handoff`: the turn ends at its next safe point and the steer comes back as `FX_EVENT_STEER_RETURNED`); `fx_session_stage_images` stages a batch through `libfx/journal_images` after an open answered `imagesNeeded`. Bytes must be the type they name; the core's model bounds (3.75 MiB an image, 6 MiB a prompt, 8 tool images) answer `FX_E_TOO_LARGE`. | Milestone 2b-2: implemented in the C entry; `tests/images.rs`, `tests/resume.rs`. Fix round 1 (Sol's m2 items 3 and 6): `libfx/handoff` refuses a turn that takes no more steering ("No prompt is running", `acp/server.zig`, `acp/libfx_steering.zig`'s `requestHandoffChecked`), so a late handoff comes back as `FX_STALE`; a journaled session without an image store takes a prompt or tool result only if its text and base64 images fit 1.875 MiB (a turn's journal holds its input twice; a reopen loads 4 MiB), so no acknowledged head is one a reopen refuses (`tests/durability.rs`). |
| P-STOP | The host's completion gate as an fx Stop hook handler: `Config.host_hooks` (`core/hooks/host_hooks.zig`, new) registers the C entry's handlers before `acp/server.zig` freezes the hook runtime. A STOP question carries the final reply and the continuations so far; CONTINUE appends the host's message as fx's own synthetic continuation. The core asked a turn's Stop hook once; `core/agent/runtime/orchestrator.zig` now asks again after each continuation, and the handler bounds itself (asked at most `max_stop_continuations` + 1 times; the last CONTINUE ends the turn). Questions are leases (header rule 9): a cancelled turn's question still waits for its answer. | Milestone 2b-2: implemented; `tests/hooks.rs`. |
| P-PRETOOL | fx's PreToolUse hook for host tools, through the same registration: ALLOW runs the call, BLOCK gives the model the reason, REWRITE runs it with new input (a JSON object). The question names the call id the allowed call then carries. | Milestone 2b-2: implemented; `tests/hooks.rs`. |
| P-JOURNAL | `libfx/journal_append` and `libfx/journal_flush` bridged to `fx_host.journal_append` and `fx_journal_ack`/`fx_journal_fail`, with append seq ranges; flush answers are control frames that never meet the input bound. Open from a journal and a checkpoint (`libfx/journal_open` with both as attachments) reports the open turn, the last ended turn, the calls a crash left running and the held follow-ups as `FX_EVENT_RECOVERY`; `fx_turn_resume` continues the open turn (`continueRecovery`, never re-running its calls); a prompt repeating the last ended turn's message id runs nothing; `fx_checkpoint` is `libfx/snapshot`. The Rust side's session folder (`crates/aero-core/src/journal.rs`) is the fenced store. | Milestone 2b-2: implemented; `tests/resume.rs` (kill -9 of the host). Fix round 1 (Sol's m2 items 1 to 4 and 6): a store failure while a turn runs cancels it and ends it with `FX_STOP_ERROR` even between barriers; a journaled turn's TURN_END waits for the acks of every append it sent; a turn's end returns its queued steers, its error and TURN_END and clears the turn in one critical section, so a steer is admitted before (and comes back) or after (`FX_STALE`); a withdraw leaves the steer in place until libfx answers. The Rust store cuts a torn last record under the writer lock before any append, refuses a folder whose whole records are not valid and consecutive, stops after an uncertain write, refuses an append that would outgrow what a reopen loads, tracks the acknowledged head apart from the disk head for checkpoints, retries acks and failures the core could not take, and publishes images from distinct temporary files with a directory sync (`tests/journal_store.rs`, `tests/durability.rs`, `tests/steer_settle.rs`). |

## Where the header departs from the design's sketch (section 2.3)

- No credentials in `fx_config`: the host adds them at its fetch edge, bound to the provider's canonical origin (header rule 13). The core's Gateway path insists on a key, so it is given the placeholder `host-managed`; `fx_fetch_take` strips every credential-class header, the placeholder included. `fx_fetch_request` names its provider and purpose so the edge can bind credentials.
- `fx_runtime_info_get` added (lock-free): the host needs exited, failed and panicked state and a message without a call that might touch a failed runtime.
- Every long call returns a request id and answers with an event, so no call waits on the core; `fx_checkpoint` answers with `FX_EVENT_CHECKPOINT`.
- `fx_fetch_abort` dropped: the core retires a fetch itself (cancel, steer, deadline, stop), wakes the ready fd, and the host reads `fx_fetch_disposition_get`; the header wait is pulsed, so cancel reaches a request waiting on its headers.
- `fx_tool_progress` dropped: libfx has no host-tool progress the model sees; panel progress stays in Rust.
- `journal_append` carries no `expected` cursor or marks: the host owns the store's head and fencing (as `sdk/engine.js` does); appends carry their seq range and `fx_journal_ack` takes the append id.
- `stop` and `pre_tool` are one `hook` callback that answers at once (`ALLOW`) or later through `fx_hook_answer`, so a slow completion check never blocks a core thread inside a callback.
- Callbacks take a pointer to a sized struct (`fx_tool_call`, `fx_journal_append`, `fx_hook_question`), so a later field never changes a signature.
- `fx_turn_steer`, `fx_turn_withdraw` and `fx_turn_cancel` name the turn they mean (stale otherwise), and steer takes `fx_input` (images arrive with P-IMAGES).
- `fx_turn_resume`, `fx_session_stage_images` and `FX_EVENT_RECOVERY` added for durable recovery; `FX_EVENT_STEER_RETURNED` for a steer its turn did not take.
- `fx_events_drain` takes the host's element stride; in-structs carry `struct_size` and are read by prefix.
- `wake` dropped from `fx_host`: the ready fd is the one wake path (kqueue on the `pm-aero` worker).
- `fx_turn_prompt` while a turn runs is `FX_E_STATE` rather than an implicit steer.
- `fx_test_arm_fault` and `fx_test_set_slot_generation` added for the containment and slot tests; hosts never call them. `FX_FAULT_CREATE` is process-wide (armed with handle 0), since no runtime exists when it fires.
- `FX_FETCH_BUSY` added to the dispositions: contention past the bound is a retry, never a reason to abort a live HTTP request.
- `fx_tool_result` and `fx_hook_answer` may be called from the callback that handed out their own call or question (rule 1), so an early answer and a lease dropped on the callback's stack are legal; every other call from a callback is not.
- Credentials are a trusted allowlist of provider, origin and header (several headers per origin), checked again at every redirect hop (rule 13), rather than one header per provider.

## Upstream tracking

    tools/sync.sh ~/Developer/pm-durable/harness/refs/fx <tag or commit>

reads the pin from `tools/closure.txt`, checks that every upstream file
without a notice is upstream's at the pin, exports our series (the 11 patched
upstream files against the pin) to `series.diff` in a temporary folder,
merges each patched file with `git merge-file --diff3` (ours, the pin, the new
commit) in place and stops on a conflict with the markers left; after
resolving, `tools/sync.sh --continue …` re-derives the closure (which builds
it), re-extracts `src/libfx_core_runtime.zig` from the new
`napi_core_main.zig`, refreshes `LICENSE`, `THIRD_PARTY_NOTICES.md` and the
fake server, and names any change to upstream's `build.zig`/`build.zig.zon`
(ours are trimmed by hand). Then the Aero gates must pass before the pin
lands, and the pin and closure count above are updated.
