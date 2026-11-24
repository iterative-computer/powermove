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
| `include/fx_core.h` | The C ABI (patch P-CAPI): threads, ownership, copies and closed/failed behaviour per function. |
| `build.zig`, `build.zig.zon` | Trimmed build: the `capi` static library and the `fake-server` test binary only. |
| `test-support/fake-inference-server.zig` | Upstream's `benchmarks/libfx/fake-inference-server.zig`, verbatim: the deterministic fake model for tests and benchmarks. |
| `toolchain.toml` | The pinned Zig (0.17.1) with the tarball's sha256 from ziglang.org's index. |
| `install-zig.sh` | Downloads that tarball, checks the sha256, unpacks it into `$PM_ZIG_HOME`. |
| `tools/derive-closure.sh`, `tools/closure.txt` | Derives the closure from the build graph and refreshes `src/` to it. |
| `tools/extract-core-runtime.py` | Copies the N-API entry's runtime pieces verbatim into `src/libfx_core_runtime.zig`. |

Omitted from upstream: the fx CLI and terminal UI (`src/main.zig`, `src/ui/`
and what only they reach), the WebAssembly and Node-API entries, the SDK
JavaScript (`sdk/`), benchmarks, examples, e2e and TLA+ tests, fuzzing and PGO
build steps, scripts and CI. The TLA+ specs stay upstream
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
machine without Zig.

## Static link

`libfx_core.a` is one object (Zig compiles the whole module as one unit). Its
only global definitions are the 21 `fx_*` functions of `fx_core.h`; every
other symbol is local. Zig's compiler-rt and UBSan runtime are not bundled
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
files. Building the crate from the pruned tree proves it complete.

## Powermove patches

| Patch | What and why | Status / upstream intent |
|---|---|---|
| P-CAPI | A C ABI entry beside the N-API and Wasm entries: `src/capi_core_main.zig` implements `include/fx_core.h`. It reuses the N-API entry's runtime, input queue, attachment table, fetch bridge and runtime slots, extracted verbatim by `tools/extract-core-runtime.py` into `src/libfx_core_runtime.zig` (new file; upstream's `napi_core_main.zig` is not vendored), and replaces only the Node-API surface. Handles are generation-tagged `u64`s over 64 slots with a lock-free enter/leave count, so a stale handle answers `FX_E_CLOSED` and destroy waits for calls inside. Until P-EVENTS, the core still writes ACP frames: the C entry is its transport, turning host calls into frames on the input queue and frames into typed events and host callbacks inside the core's writer callback (on the core's thread). | Milestone 1: implemented; smoke test green. Proposable upstream as a C ABI (needs the Chief's say). |
| P-EVENTS | A typed event sink beside the ACP writer where `prompt.zig` builds `session/update`, so the hot path never formats or parses JSON. | Planned (milestone 2). |
| P-TOOLS | Host tool calls handed to the host with the model's raw input slice and no JSON-RPC hop; concurrent idempotent calls up to `max_concurrent_tools`. | Milestone 1 passes the raw input slice from the frame; the rest is milestone 2. |
| P-IMAGES | Prompt and tool-result images as raw bytes, never base64 on our side. | Planned (milestone 2); image arguments return `FX_E_UNSUPPORTED` until then. |
| P-STOP | fx's Stop hook as the host's completion gate, continued up to `max_stop_continuations` times (upstream allows one). | Planned (milestone 2). |
| P-PRETOOL | fx's PreToolUse hook bridged to `fx_host.pre_tool`. | Planned (milestone 2). |
| P-JOURNAL | `libfx/journal_append` and `libfx/journal_flush` bridged to `fx_host.journal_append` and `fx_journal_ack`/`fx_journal_fail`. | Milestone 1: append, ack, fail and flush release implemented; resume from a journal is milestone 2. |
| P-PANIC | The C ABI root's own panic handler: record, mark the runtime failed, wake, park the thread; the host abandons the runtime. | Planned (milestone 2). |

## Where the header departs from the design's sketch (section 2.3)

- No credentials in `fx_config`: the host adds them at its fetch edge. The core's Gateway path insists on a key, so it is given the placeholder `host-managed`, which the host's authorization header replaces.
- `fx_runtime_info_get` added: the host needs exited/failed state and a panic message without a call that might touch a failed runtime.
- Every long call returns a request id and answers with an event (`SESSION_OPENED`, `REQUEST_DONE`, `CHECKPOINT`); `fx_checkpoint` no longer returns bytes by pointer, so no call blocks on the core.
- `fx_fetch_abort` dropped: the core retires a fetch itself (cancel, steer, deadline); the host reads `fx_fetch_disposition_get` after each wake and aborts its HTTP request on `RETIRED`. `fx_fetch_disposition_get` added for that, as libfx's `coreFetchDisposition`.
- `fx_tool_progress` dropped: libfx has no host-tool progress the model sees; panel progress stays in Rust.
- `journal_append` carries no `expected` cursor or marks: in libfx the host owns the store's head and fences appends (as `sdk/engine.js` does); the core sends ordered events and waits on barriers. `fx_journal_ack` takes the append id, not a cursor.
- `stop` and `pre_tool` answer at once (`ALLOW`) or later through one `fx_hook_answer`, so a slow completion check never blocks a core thread inside a callback and a block reason or rewrite never needs a pointer returned across the boundary.
- `fx_turn_steer` takes text only: libfx's steer is text.
- `wake` dropped from `fx_host`: the ready fd is the one wake path (kqueue on the `pm-aero` worker).
- `fx_turn_prompt` while a turn runs is `FX_E_STATE` rather than an implicit steer.
- Events carry `turn` and `request` ids, and `fx_usage` has the fields libfx reports (cache read/write, reasoning, context used and size).

## Upstream tracking

Re-pin by running `tools/derive-closure.sh` at the new commit after
re-applying our patches (`sync.sh`, milestone 2, design 2.7), then
`tools/extract-core-runtime.py` against the new `napi_core_main.zig`.
