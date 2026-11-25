# Powermove fork of fx (libfx core)

Upstream: https://github.com/vercel-labs/fx
Pinned commit: `8fbe9b8` (upstream version 0.0.18).
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
| `tools/derive-closure.sh`, `tools/closure.txt` | Derives the closure from the build graph and refreshes `src/` to it. |
| `tools/extract-core-runtime.py` | Copies the N-API entry's runtime pieces verbatim into `src/libfx_core_runtime.zig`. |

Omitted from upstream: the fx CLI's and terminal UI's entry points
(`src/main.zig` and the files only they reach), the WebAssembly and Node-API
entries, the SDK JavaScript (`sdk/`), benchmarks, examples, e2e and TLA+
tests, fuzzing and PGO build steps, scripts and CI. The closure still holds
85 files under `src/ui/`: shared code `@import`s them, and Zig parses every
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
writes it to `tools/closure.txt` and makes `src/` exactly the closure plus our
files. Every manifest of the fresh cache that names the C entry is read, in
sorted order, and the script fails if they list different sources. Building the crate from the pruned tree proves it complete.

## Powermove patches

| Patch | What and why | Status / upstream intent |
|---|---|---|
| P-CAPI | A C ABI entry beside the N-API and Wasm entries: `src/capi_core_main.zig` implements `include/fx_core.h` (ABI 2). It reuses the N-API entry's runtime, input queue, attachment table and fetch bridge, extracted verbatim by `tools/extract-core-runtime.py` into `src/libfx_core_runtime.zig` (new file; upstream's `napi_core_main.zig` is not vendored), and replaces the Node-API surface. Handles are generation-tagged `u64`s over 64 slots (40-bit generations; an exhausted slot is retired for good; occupancy, generation and pointer published with release, entered only when occupied). Host calls take the locks core threads share only with a bounded try-lock that gives up when the runtime panics. Sessions and turns still travel as ACP frames: the host's calls become frames, and the frames the core writes become typed events in its writer callback. | Milestone 2a: implemented; tests green. Proposable upstream as a C ABI (needs the Chief's say). |
| P-PANIC | The C ABI root's own panic handler (`pub const panic` in `capi_core_main.zig`). A panic on a core thread records its message in the runtime's fixed buffer, marks it PANICKED with one atomic store, wakes the ready fd and parks the thread; the runtime's other threads park at their next boundary with the host, so no callback starts after. A panic inside a host call unwinds, through a naked-asm guard around every export, back to that export, which returns `FX_E_FAILED`: the host thread is never parked. The core's threads carry their runtime in a thread-local (`core/shared/panic_context.zig`), inherited at the four spawn sites a runtime uses: `acp/server.zig` (prompt worker), `core/agent/runtime/parallel_execution.zig` (parallel tool workers), `core/agent/runtime/request_recovery.zig` (status ticker) and `core/agent/runtime/cache_warmer.zig` (its `std.Io` task). A panic on a core thread with no runtime (none is known) fails every running runtime rather than leaving one stalled. A panicked runtime is never joined or freed. | Milestone 2a: implemented, fault-injected at six points (`fx_test_arm_fault`). Not upstream material. |
| P-TOOLS | Host tool calls reach the host directly: `acp/prompt.zig`'s `callHostTool` calls `Config.host_tool_bridge` (`core/cli/acp_runner.zig`, new field) when the embedder installed one, so the host gets the core's own copy of the model's input with no JSON-RPC frame and no further copy, pinned until the host answers (the lease). The answer's text is copied once, into the call's record, and becomes the tool result without another copy. Concurrency: up to `max_concurrent_tools` idempotent calls, a writes tool alone. Milestone 1 instead copied the input out of the `libfx/tool_call` frame and sent the result back through a JSON frame. | Milestone 2a: implemented. Upstream intent: an executor hook is reasonable to propose. |
| P-EVENTS | A typed event sink beside the ACP writer where `prompt.zig` builds `session/update`, so the hot path never formats or parses JSON. | Planned (milestone 2b). |
| P-IMAGES | Prompt, steer and tool-result images as raw bytes, never base64 on our side; staged image batches for resume. | Planned (milestone 2b); image arguments return `FX_E_UNSUPPORTED` until then. |
| P-STOP | fx's Stop hook as the host's completion gate, continued up to `max_stop_continuations` times (upstream allows one). | Planned (milestone 2b); the limit is validated and stored now. |
| P-PRETOOL | fx's PreToolUse hook bridged to `fx_host.hook`. | Planned (milestone 2b). |
| P-JOURNAL | `libfx/journal_append` and `libfx/journal_flush` bridged to `fx_host.journal_append` and `fx_journal_ack`/`fx_journal_fail`, with append seq ranges; flush answers are control frames that never meet the input bound. Resume from a journal and checkpoints are specified in the header. | Appends, acks, failure and flush release: implemented. Resume and checkpoints: milestone 2b (`FX_E_UNSUPPORTED` until then). |

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
- `fx_test_arm_fault` and `fx_test_set_slot_generation` added for the containment and slot tests; hosts never call them.

## Upstream tracking

Re-pin by running `tools/derive-closure.sh` at the new commit after
re-applying our patches (`sync.sh`, milestone 2, design 2.7), then
`tools/extract-core-runtime.py` against the new `napi_core_main.zig`.
