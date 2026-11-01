# pm-runtime-v8 — P2 package 8

Headless runtime infrastructure in `apps/native/crates/runtime-v8`, pinned to
rusty_v8 **152.2.0** (V8 **15.2.124.1-rusty**). No Node, Chromium, UI, kernel or
engine implementation is linked. Linux uses upstream's prebuilt archive. macOS
requires `--features macos-sandbox` and the S1 sandbox archive **and matching
bindings**; the build refuses a macOS consumer without that feature. No V8 source
build was performed in this lane.

## Ownership and use

`RuntimePool` is scoped to one project window. `realm(kind)` starts asynchronously
and returns the same live worker for that key. `builtin(id, compat_editor)` routes
C-MATRIX's connected editor closure together: timeline, inspector, viewer chrome,
toolbar, shader/source-preview hooks, caption/3D callbacks and unproven callback
consumers. Only theme/effects/transitions default to the rest cluster. Agent is an
explicit native-port option, never an inferred independent legacy cluster. A
compat provider replacement routes the whole editor closure to one trusted Compat
isolate. Each v4 extension key has its own isolate. Store/compiler kinds are
available infrastructure; Store process confinement is a separate host duty.

`Realm::{eval,reload,invoke_retained,pin}` return `Ticket<T>` immediately. Tickets
implement Future and `try_take`; there is no blocking wait or join API. Sources
are bundled script expressions, optionally returning Promises. The compiler lane
must resolve imports and wrap activation before submission. IDs fence queued
calls and Stop requests. `eval_generation` refuses a retired generation.

Each worker exclusively owns its isolate, strong context handles, host jobs and
quota callback. `SharedIsolate`/Locker guards every V8 entry; no Local or borrowed
V8 reference crosses an unlock. Idle receive waits park the owner and release the
Locker. Workers retain their OS threads; migrating parked isolates onto a bounded
shared executor is a possible later optimization. All disposal, callback removal
and retained-generation retirement occur on that owner, including close and last
handle drop. `close()` is nonblocking; `is_disposed()` reports completed cleanup.

## Snapshots, caches and limits

Editor/rest/agent/extension/compat/compiler/Store have separate snapshot blobs.
The bootstrap contains the web definitions, no user state or native function
pointers. Restore attaches native functions and injects persisted JSON storage.
The process initializes V8 once. `CodeCache` has a byte budget and keys all of:
V8 version, exact snapshot bytes, exact bundle bytes, and runtime flag/feature
identity, with domain separation and length-delimited SHA-256. V8 independently
checks cached-data acceptance. Snapshots are in-memory process caches.

`Limits` bounds source bytes, queued submissions, concurrent host requests,
response bytes, host argument bytes, staged op count/bytes, timers, console queue,
heap and external bytes. `Admission` reserves configured heap maxima for every
active/candidate/retained isolate; share one account across pools, including
compiler/Store hosts. It releases only after disposal. This is admission
accounting, not a process-RSS measurement.

A near-heap-limit callback terminates execution and grants one bounded 8 MiB
emergency increment to unwind. External memory is checked after microtasks and
through V8 interrupts during long tasks; host response capacity is reserved
before starting I/O. This **does not provide a hard allocation-time cap for all
JS ArrayBuffers**: S1's sandbox allocator cannot be replaced with rusty_v8's
non-sandbox Rust allocator. Oversized allocations can precede a sampled check.
Untrusted Store code still requires a confined helper process. Structured-clone
serialization also checks its output budget after serialization. Neither case is
presented as a hard host allocator or OOM-isolation guarantee.

## Watchdog and recovery

Defaults: notice after **2 s**, native **Stop/Wait** prompt hook after **8 s**,
repeat prompt after another 8 s on Wait. Hooks enqueue native notices/prompts;
they must return promptly. Wait never silently kills trusted code. The shell can
later call `stop(execution)`; stale IDs cannot terminate later tasks. A watchdog
also covers Promise microtasks, event pumping, result conversion and activation
effect serialization. Kernel and host trait implementations must themselves be
bounded; V8 termination cannot interrupt arbitrary blocking Rust code.

On termination/quota failure, the worker cancels host work, calls
`revoke_generation` and `seal_open_gestures`, unwinds handles and disposes on the
owner, then restores a new kind snapshot plus `load_storage` from the kernel.
Process-unique generation IDs prevent revival after close/reopen. Completed
acknowledged edits/storage remain kernel-owned. Ephemeral JS state is lost on a
failure; persisted storage is never deleted. The adapter must map scoped realm
identities to the existing extension ID/storage namespace, preserving `ext.<id>`
objects and unknown keys rather than creating a new disk format.

## Activation and history

Only v4 can reload into a fresh isolate. The candidate receives deep JSON copies
of persisted storage and `hot.data`; `runtime.stage(op)` accumulates an op list.
Await activation, then call `KernelHooks::commit_activation(previous,next,ops,
storage)`. The adapter must validate and atomically publish ops, storage and new
registrations, replacing previous registrations only on success. Rejection or a
throw disposes the candidate and leaves the active generation unchanged. Network
I/O and logs already completed during activation are observable and cannot be
rolled back; pending requests are cancelled. Native model/UI edits must use the
staged op surface. Arbitrary module-global mutation is not transactional.

`GenerationPin` retains callable old closures for history/export. Successful
replacement cancels old I/O, clears old timer registrations and revokes old
registrations; pinned isolates stay until their last pin retires. Retained calls
use `commit_retained`, which must apply only lease-authorized history operations
and never overwrite current storage. Failure revokes/seals failed generations.
The compat/editor cluster deliberately rejects whole-isolate hot reload: that
would break held provider identity. Staged same-realm module replacement belongs
to the compiler/compat host, keeping the C-MATRIX live-object contract.

## Web subset

The snapshot supplies timers/clear/interval, Promise microtask checkpoints,
queueMicrotask, monotonic performance.now, UTF-8 TextEncoder/encodeInto and
streaming/fatal/BOM-aware UTF-8 TextDecoder, URL and live URLSearchParams,
structuredClone using V8's serializer (cycles, aliases, Map/Set/Date/BigInt,
typed arrays and ArrayBuffer transfer), console, EventTarget, AbortController /
AbortSignal, fetch and buffered Headers/Response. URL parsing and form encoding
use Rust's WHATWG URL implementation. Fetch is only HTTP(S), through HostProxy;
there is no socket/filesystem capability in the JS runtime.

HostProxy starts nonblocking requests; HostRequest::poll is nonblocking and
cancel propagates on abort, timeout, shutdown, reload and generation failure.
The host enforces permissions, credentials, redirects and streaming byte limits
before buffering. The runtime independently checks completed response limits.
These are bounded buffered APIs, not a complete browser platform: non-UTF-8
TextDecoder labels, streams, Blob/FormData, full Request/cache/service-worker
semantics, DOM/layout/canvas and remote panels need their owning compatibility
adapters. No extension is classified as incompatible merely because it needs
those adapters. No Electron quirk was adopted as a runtime specification.

## Decisions and selecting rules

| Decision | Rule |
|---|---|
| Keep the connected editor closure and the trusted v1–3 realm together; never synchronous isolate-thread RPC | N4/N6 exact live identity and synchronous contract; C-MATRIX and adopted S5c fallback |
| Owner workers, Locker parking, Future/ticket API and owner disposal | N8 main never waits on JS; N7 headless/id-driven runtime |
| Fresh v4 activation candidates, atomic staged op publication, persisted-storage recovery | N3 authored edits are exact; N4 never discard acknowledged state |
| Pin old callable generations; separate retained history commits from active storage | N4/N6 history remains callable without overwriting newer storage |
| Version/snapshot/bundle/flags cache key; reject stale execution/generation IDs | N3 correctness before reuse; N1 reliable reload and recovery |
| Bounded host queues, heap/external checks and shared admission, explicit Stop/Wait | N1/N2 responsive editor; N8 independent UI; S1's hard-external-limit gap remains explicit |
| UTF-8/WHATWG/V8 primitives, explicit adapter gaps, no browser-bug imitation | N3 specification wins; N4/N6 do not silently remove legacy capabilities |
| Preserve all replaced Electron/spike consumers | Prune rule: no deletion before the final consumer is retired |

## Linux verification and integration

From the repository root (every shell sources the local tool setup):

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
cargo test --locked --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml
cargo clippy --locked --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --check
```

The isolated manifest compiles the **same production sources** with an owned
lockfile, following the existing S1 pattern. `cd apps/native && cargo test
--locked -p pm-runtime-v8` was attempted: Cargo refuses because the shared native
lockfile needs this new package/dependencies. This lane does not own that file.
The integration lane must regenerate it; the existing `crates/*` workspace glob
already discovers this crate. No other crate/source or shared lock was edited.

Current evidence: **30 tests pass**, including real isolate/snapshot/cache
execution, cross-thread submissions with parked Lockers, routing/identity,
web APIs, host abort/time/byte bounds, staged activation failure/success,
retained closures/admission retirement, Wait/Stop and microtask/infinite-loop
recovery, persisted storage, external-memory termination and a real near-heap
callback in a subprocess. Strict Clippy and Rust formatting pass. All inputs
are synthetic; no private profile, user project or secret is used.

Still needed from other lanes: concrete kernel actor hooks (atomic activation,
writer fencing, gesture sealing and retained history leases), engine/recorder
bindings, compiler/module loader, host permission/network adapter, native prompt
wiring, remote-DOM/WebKit panels and full v1–3 facade/corpus/trace replay. This
crate alone does not establish all-extension compatibility or product budgets.

macOS verification must link the S1 sandbox library/bindings and confirm sandbox
activation, hardened-runtime JIT/signing, AppKit prompt/main-thread independence,
real kernel/engine/recorder behavior, WebKit remote panels, supported-OS and
39-extension memory/performance rows. No Metal/AppKit/GPUI/WebKit or on-screen
claim is made from Linux.
