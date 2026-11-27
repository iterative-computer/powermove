# pm-runtime-v8 — P2 package 8, waves 1–2

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
isolate. Each v4 extension key has its own isolate. The compiler kind restores the pinned Svelte compiler snapshot. Direct
`RuntimePool::realm(Store(..))` is rejected: Store clusters use `StorePool` and
the separate `pm-store-helper` executable.

`Realm::{eval,reload,invoke_retained,pin}` return `Ticket<T>` immediately. Tickets
implement Future and `try_take`; there is no blocking wait or join API. Sources
are bundled script expressions, optionally returning Promises. The module loader must resolve imports, bind the shared Svelte runtime modules
and wrap activation before submission; `SvelteCompiler` supplies the front end. IDs fence queued
calls and Stop requests. `eval_generation` refuses a retired generation.
`active_execution()` returns generation/id/elapsed time from native state,
including during a hang, without entering JS.

Each worker exclusively owns its isolate, strong context handles, host jobs and
quota callback. `SharedIsolate`/Locker guards every V8 entry; no Local or borrowed
V8 reference crosses an unlock. Idle receive waits park the owner and release the
Locker. A realm with no timer/I/O work sleeps until a submission, close or pin
retirement; timer deadlines and active I/O determine wakeups. One persistent
watchdog per owner also sleeps while idle. Workers retain their OS threads;
migrating parked isolates onto a bounded shared executor is a possible later
optimization. All disposal, callback removal
and retained-generation retirement occur on that owner, including close and last
handle drop. `close()` is nonblocking; `is_disposed()` reports completed cleanup.

## Snapshots, caches and limits

Editor/rest/agent/extension/compat/compiler/Store have separate snapshot blobs.
The bootstrap contains the web definitions, no user state or native function
pointers. Restore attaches native functions and injects persisted JSON storage.
The process initializes V8 once. `CodeCache` has a byte budget and keys all of:
V8 version, exact snapshot bytes, exact bundle bytes, and runtime flag/feature
identity (including V8's effective cached-data version tag), with domain
separation and length-delimited SHA-256. Internal control scripts do not populate
the bundle cache. V8 independently
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
Untrusted Store code runs through the confined helper process described below. Structured-clone
serialization also checks its output budget after serialization. Neither case is
presented as a hard host allocator or OOM-isolation guarantee.

## Store helper and IPC

`StorePool::helper(cluster_id)` returns immediately and reuses one live process
for that cluster within its project scope. Process launch, pipe reads, host
forwarding and reaping run on native transport workers. `StoreHelper::eval`
returns a normal Future/ticket. Its queue has an explicit admission permit per
submission, including the active request; moving commands into the dispatch
queue cannot bypass the bound. Idle transport waits for events, while background
realm timers can still issue kernel/host RPC without another submitted eval.
`close()` and last-handle drop cancel the host budget and kill the child without
joining on the caller. The transport reaps it, completes pending tickets and
revokes/seals every acknowledged generation. Restart restores acknowledged kernel
storage; sources/storage are never deleted.

The helper starts with an empty environment, `/` as cwd and inherited stdin/stdout
pipes. It initializes trusted V8/Store snapshot code, applies confinement, then
accepts extension source. The `StoreSandbox` trait has a production macOS seatbelt
implementation using [store.sb](../../apps/native/crates/runtime-v8/store.sb),
and a Linux test double. Seatbelt is deny-default, with system-library, entropy,
self process-info, sysctl and JIT necessities. There is no project/home/media,
file-write, outbound-network, spawn or Mach-service grant. Anonymous inherited
pipe behavior, OS-version compatibility and final signed JIT **must be verified
on macOS**; Linux cannot establish seatbelt correctness.

The Linux double installs `no_new_privs` and an architecture-checked seccomp BPF
filter with TSYNC over existing V8 threads, denies filesystem open/mutation,
socket creation/connect, exec, ptrace and namespace/mount changes, and preserves
inherited-pipe IPC/JIT. It first attempts user/network/mount namespaces and reports
whether the host permits them. This container refused namespace creation (`namespaces=false`), while seccomp
was installed (`seccomp=true`). Namespace failure is explicit; seccomp failure
refuses startup. This is a real Linux denial test, not a claim of complete Linux
sandbox equivalence to the macOS policy.

IPC v1 uses four-byte little-endian lengths followed by UTF-8 JSON. Zero,
truncated, oversized (>64 MiB), malformed, unknown-version and wrong-id frames
fail closed. One reader acknowledgement bounds prefetched frames independently
of the submission queue. Source/response settings are bounded below the frame
limit to leave JSON-escaping/binary-array expansion headroom. Messages are
`init`/`ready`, `eval`/`result` and bidirectional `rpc`/`reply`. Kernel
load/commit/revoke/seal, console, cooperative engine operations and nonblocking
fetch start/poll/cancel are forwarded to the existing parent traits. No parent JS
runs. Each connection binds the project/cluster namespace; remote generations
are checked and translated into process-session-specific ids so stale helper
ids cannot revive after restart. Fetch remains permission-controlled by the
host adapter, including redirects, credentials and bytes; child-side requests
cannot open sockets directly.

`StoreStatus` exposes pid, confinement report, last owner-sampled V8 statistics
and Linux RSS separately. The helper's hard-failure probe is test-only: it enters
the **same helper initialization and sandbox trait**, then intentionally creates
a 16-MiB V8 heap without the production near-limit recovery callback and exhausts
it. A fatal V8 diagnostic plus SIGABRT/SIGTRAP is required. A live trusted parent
realm and another production Store helper must both retain their values. No
fatal-OOM command or quota-disabling switch exists in the production executable.

Store history/activation facade RPC remains an owning-adapter integration gate;
the helper currently refuses an unleased retained commit. The process boundary
does not replace permission checks or the kernel's writer/lease validation.

## Svelte compiler front end

`SvelteCompiler` restores the **5.56.10** standalone compiler inside the Compiler
snapshot. Its unmodified MIT artifact, license and SHA-256 provenance are owned
under `compiler/`; there is no runtime download, Node loader or package manager.
`compile(source, filename, Component|Module)` returns `SvelteTicket` immediately;
type parsing/transformation and all compiler JS execute on the compiler owner.
Components, including TypeScript script blocks, use Svelte's parser. `.svelte.ts`
modules first use native OXC **0.152.0**, pinned for Rust 1.96.1, targeting ES2022.
OXC 0.156.0 requires Rust 1.97 and was therefore not selected for this crate.

The source path follows `v1.2.0:apps/desktop/src/main/extensions/compiler.ts`:
client output, runes enabled, external component CSS and a `powermove:styles`
`add` acquisition import. Module output stays ESM. Source maps/warnings are kept;
Svelte errors preserve name, code, filename, positions and frame. Neither compiler
path executes the authored source. The bounded derived-output cache keys exact
source bytes/hash, compiler version/artifact, filename, unit/options and the OXC
version/target. Changed filenames matter to CSS hashes and source maps. Eviction
only discards derived output. The loader still owns dependency resolution,
shared `svelte/*` singleton binding, style acquisition lifetimes, activation and
remote-DOM mounting. This front end alone does not establish full Svelte panel
compatibility on macOS.

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
objects and unknown keys rather than creating a new disk format. Storage enters
V8 through JSON parsing rather than JavaScript object-literal source, preserving
own `__proto__` keys and keeping storage independent of the source-text budget.

## Activation and history

Only v4 can reload into a fresh isolate. The candidate receives deep JSON copies
of persisted storage and `hot.data`; `runtime.stage(op)` accumulates an op list.
Await activation, then call `KernelHooks::commit_activation(previous,next,ops,
storage)`. UTF-8 byte/count checks, scheduling and effect serialization all finish
before that kernel call; no further candidate JS runs after publication. The
adapter must validate and atomically publish ops, storage and new
registrations, replacing previous registrations only on success. Rejection or a
throw disposes the candidate and leaves the active generation unchanged. Network
I/O and logs already completed during activation are observable and cannot be
rolled back; pending requests are cancelled. Native model/UI edits must use the
staged op surface. Arbitrary module-global mutation is not transactional.

`GenerationPin` retains callable old closures for history/export. Successful
replacement cancels old I/O, clears old timer and EventTarget listener tables,
revokes old registrations and seals open gestures; pinned isolates stay until their last pin retires. Cleanup retains a private V8
function handle so replacing the global control object cannot disable retirement.
Retired generations cannot schedule new timers/fetch/listeners. The final pin
releases its strong reference **before** waking the idle owner, avoiding a race
that could otherwise strand the isolate until another submission.
`Realm::measure()` samples every active/retired isolate on its owner and reports
used/physical heap, external bytes and serialized snapshot bytes. These are
separate from configured admission reservations and process RSS. Retained calls
use `commit_retained`, which must apply only lease-authorized history operations
and never overwrite current storage. A throwing retained callback reports its error
and stays callable; termination/quota failure revokes/seals its generation.
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
`runtime.engine(operation,input)` calls the optional EngineProxy on the V8 owner.
HostBudget supplies a cancellation token, deadline and response-byte ceiling;
cooperative engine implementations must check these while waiting and before
allocating. Operation count, argument/output bytes and post-call deadline checks
are enforced here too. No engine callback may synchronously enter JS or main.
The Linux engine double verifies owner placement, arguments, timeout, over-budget
responses and cancellation during a native call.

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
| Deadline-driven idle sleep, bounded host queues, heap/external checks and shared admission, explicit Stop/Wait | N1/N2 responsive editor; N8 independent UI; S1's hard-external-limit gap remains explicit |
| UTF-8/WHATWG/V8 primitives, explicit adapter gaps, no browser-bug imitation | N3 specification wins; N4/N6 do not silently remove legacy capabilities |
| Preserve all replaced Electron/spike consumers | Prune rule: no deletion before the final consumer is retired |

## Wave 2 decisions and selecting rules

| Decision | Rule |
|---|---|
| Require the Store helper API, one process per project/cluster, fail closed on protocol or confinement failure | N1/N2 responsiveness and fatal-OOM containment; N8 no parent/main-thread JS wait |
| Keep permissioned native proxies, cluster/session fences, acknowledged storage and crash cleanup in the parent | N3/N4 exact acknowledged state; N6 unchanged host contract; N7 inspectable ids |
| Embed the pinned standalone compiler; compile in a restored isolate; perform TypeScript preprocessing with toolchain-compatible native OXC | N8 no Node/Chromium; N3 explicit compiler identities; N4/N6 retain Svelte module forms |
| Retire every ephemeral effect while history/export pins retain callable old closures and their exact bytes | N3/N4 authored history remains valid; N6 callback lifetime; N1 bounded resource accounting |
| Preserve same-realm legacy cluster identity and every existing consumer | N4/N6 and prune rule; fresh-isolate reload remains v4-only |
| Report Linux seccomp/namespaces, V8 heap, RSS and executable sizes separately; leave macOS seatbelt/JIT as explicit gates | N1/N3 evidence before performance/security claims; no Electron quirk copied |

## Linux verification and integration

Every shell sources the workspace tool setup. Use the owned isolated manifest,
which compiles the **same production sources**, binary and integration tests:

```sh
source /workspace/.tools/activate.sh
cd /workspace/powermove
cargo test --locked --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml
cargo test --locked --release --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml -- --nocapture
cargo clippy --locked --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --check
```

**63 tests pass: 55 unit + 8 integration**, including two gated subprocess probe
entry points. All original 45 tests remain green. Debug and optimized release
runs pass; strict Clippy and formatting pass. Coverage adds actual helper IPC,
cluster reuse/separation, confinement denials, engine/fetch/abort, idle timer RPC,
shutdown/spawn failure, crash lease cleanup, storage restart/session fencing,
real fatal OOM with parent/sibling survival, compiler snapshot/TS/CSS/cache/error
paths, successful/failed activation cleanup, every requested reload effect kind,
20 callable retained external-history generations, export pin independence,
idle pin retirement and control-global-independent cleanup. All inputs are
synthetic; no private profile, project or secret is used.

The required workspace command `cd apps/native && cargo test --locked -p
pm-runtime-v8` was attempted. Cargo refuses to update the **shared native
Cargo.lock** under `--locked`; that file is outside this lane's ownership.
The integration lane must regenerate it for the runtime/OXC dependencies. libc
is pinned to **0.2.186**, matching the existing S1 workspace consumer. The
`crates/*` workspace glob already discovers this crate. No shared lockfile,
Electron source, vendor or spike file was changed.

### Measurements

Linux x86_64, Rust 1.96.1, optimized release, upstream unsandboxed Linux V8
archive. These are synthetic observations, not 39-extension product budgets or
macOS physical-footprint claims. Snapshot/heap sizes can vary by V8 build.

| Observation | Bytes |
|---|---:|
| Store snapshot | 462,920 |
| Store V8 used heap after synthetic registration/storage | 541,704 |
| Store V8 physical heap | 1,048,576 |
| Store V8 external bytes | 34 |
| Store process RSS at that sample | 20,615,168 |
| Helper executable, release, unstripped | 69,965,176 |
| Compiler snapshot | 3,148,600 |
| Compiler used heap after TS module/error probes | 4,914,048 |
| Compiler physical heap | 6,160,384 |
| Compiler external bytes | 33,024 |

The helper RSS is **20.62 MB decimal**, not evidence of passing S1's macOS
≤20-MB physical-footprint gate. `STORE_MEASUREMENT` and `COMPILER_MEASUREMENT`
lines reproduce these sampling points. The repeated-reload test retains twenty
1-MiB external buffers and twenty callable undo closures: admission grows to
**1,344 MiB** for 21 configured 64-MiB isolates, then falls to **128 MiB** with
one old export pin plus the active isolate, **64 MiB** after export retirement,
and **zero** on close. V8 statistics independently verify the external payloads;
configured heap reservations are not presented as RSS or actual heap use.

Still needed outside this lane: regenerate the shared native lockfile and wire
package/signing of the helper executable; concrete kernel actor atomic
activation/registration/gesture/history leases and Store facade RPC; full module
resolution/shared Svelte runtime and same-realm legacy module reload; engine/
recorder and permission adapters; native prompt wiring; WebKit remote-DOM mounting
and full v1–3 facade/corpus/trace replay. No extension is declared incompatible
because its adapter is incomplete.

macOS must verify seatbelt denial/anonymous inherited pipes across supported
OS versions, sandbox-enabled S1 archive/bindings, final signed/notarized helper
JIT entitlements, helper crash blast radius, AppKit/main-thread independence,
real kernel/engine/recorder behavior, Svelte WebKit panels and 39-extension
memory/performance rows. No Metal/AppKit/GPUI/WebKit or on-screen claim is made
from Linux.

## macOS verification — 2026-11-01

Test lane: `feat/na-macverify`, based on native-engine commit
`ff42d751fbf6a9266cd2ba1a8e25345b60baa5d7`.
ARM64, macOS 27.2 (26B5086k), debug/test configuration.
No V8 source build, release signing, notarization or publication was performed.
All inputs were synthetic; logs remain under `/tmp/pm-macverify/`.

The local S1 artifact at `/private/tmp/pm-s1-v8-artifact-final` passed all five
`SHA256SUMS` checks. Its archive and binding match the pinned hashes in
[s1-v8.md](s1-v8.md); both overrides below were set and `V8_FROM_SOURCE` was unset.
Every build/test used the shared build lock and `~/Developer/pm-target`.
Disk checks stayed above the required 8 GiB.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export RUSTY_V8_ARCHIVE=/private/tmp/pm-s1-v8-artifact-final/librusty_v8.a
export RUSTY_V8_SRC_BINDING_PATH=/private/tmp/pm-s1-v8-artifact-final/src_binding.rs
unset V8_FROM_SOURCE
flock /tmp/pm-build.lock cargo +1.96.1 build --locked \
  --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml \
  --features macos-sandbox
flock /tmp/pm-build.lock cargo +1.96.1 test --locked \
  --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml \
  --features macos-sandbox -- --nocapture
```

| Item | Result |
| --- | --- |
| Runtime build | PASS: standalone build and complete test suite with pinned Rust 1.96.1. An earlier standalone build/test run with host-default Rust 1.99.1 also passed; the pinned run is the verification result. |
| Pinned runtime tests | PASS: **55 unit + 6 integration = 61**, zero failures/ignored. Helper binary and doc-test targets contain zero tests. The documented Linux total of 63 includes two Linux-only confinement tests. |
| Production Store helper IPC | PASS: cluster reuse/separation, inherited pipes, engine/fetch/abort forwarding, idle timer effects, bounded admission, stop/spawn failure, cleanup and storage restart tests. Helper reports `platform=macos`, `seatbelt=true`. |
| Fatal helper OOM | PASS: fatal V8 diagnostic and expected abort/trap required by the test; trusted parent and sibling helper retained their values. |
| Compiler/lifecycle coverage | PASS: compiler snapshot/TypeScript/CSS/cache/error paths, activation/reload, retained generations, quotas and watchdog tests in the 55-unit suite. |
| JS compat bundle inside pm-runtime-v8 | NOT RUN. [compat-realm.md](compat-realm.md) provides the JS embedding contract, but no executable pm-runtime-v8 runner or Rust kernel adapter for `apps/native/js/compat`; that adapter is explicitly listed as integration work. The runtime's synthetic `RealmKind::Compat` routing tests pass, which does not establish execution of the actual compat bundle/corpus. |

The pinned test log reports Store snapshot **402,464 bytes**, used V8 heap
**156,080 bytes**, physical V8 heap **786,432 bytes**, external **34 bytes**,
and helper executable **86,050,576 bytes**. macOS RSS is reported as null;
no physical-footprint or 20-MB budget claim follows from those heap figures.
The compiler sample reports snapshot **2,936,592 bytes**, used heap **3,516,516
bytes**, physical heap **4,587,520 bytes**, external **33,024 bytes**.

Remaining gates: the file/network-denial probe in this suite is Linux-only,
so seatbelt installation and successful confined IPC here do **not** certify
macOS denial coverage. Final hardened/signed helper JIT, other supported macOS
versions, production adapters, compat corpus, UI-thread independence and product
memory/performance gates remain open. This lane changes verification documents
only.

### Later P2 bundle integration — 2026-11-01

`pm-compat-runner` now executes the actual compat bundle and remote-DOM event
closures in this runtime, with eight native tests. The extension inventory and
remaining kernel/WebKit/Store capture gates are recorded in
[extcheck-p2.md](extcheck-p2.md). This supersedes only the missing executable
loader blocker above; it does not certify extension parity or complete the
production adapters.

## 2026-11-04 — runtime2 allocation boundaries and snapshot regression

Fetch admission now counts the owner-sampled V8 external bytes **at the host
callback**, including buffers created earlier in that same JS stack, together
with outstanding response reservations. Synchronous engine calls reserve their
response ceiling before entering the adapter and release it on success/error.
`GenerationMemory.memory` exposes `fetch_reserved_bytes` and
`engine_reserved_bytes` separately from V8 external bytes, heap and snapshots;
Store status adds `fetch_reserved` and `engine_reserved`. These are capacity
reservations, not RSS or actual allocator consumption. Native abort cleanup
cancels/releases queued and active fetches before acknowledging the JS task,
including a script that aborts and then throws; it no longer depends on a future
idle tick. Six new unit tests include four quota/abort regressions and a
subprocess snapshot regression plus its child entry.

That subprocess reproduced a **SIGSEGV** in pinned V8's
`StringTableInsertionKey` / `SharedHeapDeserializer`: build and retire a compiler
snapshot isolate, keep an extension-kind isolate alive, then restore the
compiler snapshot. Independently extended read-only snapshots were incompatible
with the isolate group's shared read-only heap. Initialization now sets
`--no-extensible-ro-snapshot` before V8 starts; per-kind contexts/compiler
snapshots remain separate while using stock read-only roots. The flag identity
is part of the existing code-cache key. The regression failed before the change;
the full native-workspace suite passes afterward. No test is serialized or
excluded to hide this failure. Recheck the same regression against S1 on macOS.

Decisions: N1/N2 bound host response admission and release cancelled work;
N3/N6 count live buffers rather than admitting against stale statistics;
N3 keep independently restored snapshots compatible; N7 expose per-generation
capacity categories; N8 all sampling/callbacks/disposal remain on V8 owners.
No user files, extension sources, registrations or existing consumers are pruned.

Linux: `cargo test --locked --manifest-path apps/native/Cargo.toml
-p pm-runtime-v8` passes **69 tests** (61 unit, eight helper integration; no
ignored tests). All-target package Clippy with `--no-deps -- -D warnings` and
rustfmt pass. The existing native/isolated lockfiles suffice; no dependency or
workspace membership changes are needed.

**Hard external allocation remains BLOCKED.** Sampling and response reservations
cannot cap every allocation by JS, WebAssembly, structured-clone serialization,
codec/media adapters or recorder tiles. The pinned rusty_v8 Rust allocator API
is disabled with `v8_enable_sandbox`; substituting malloc/Rust backing stores
would violate sandbox ownership. A sandbox-aware allocator/page-allocation hook
and verified matching CI archive/bindings are still needed. ArrayBuffer
resizable/shared/Wasm growth, transfer/clone, native pixel buffers, allocator
free/retirement, concurrent allocation and process-level fatal limits must all
be tested before claiming a hard cap. This patch does not replace that gate
with a post-allocation check or a Linux-only allocator implementation.

## 2026-11-04 — runtime2 confinement probes and remaining gates

The helper integration probe now runs on Linux **and macOS**. It proves access
before confinement, then requires EPERM/EACCES for synthetic file read/write,
loopback networking and spawn, followed by actual TurboFan optimization/value
checks under the production sandbox policy. Existing helper IPC/restart and
fatal heap-OOM parent/sibling-survival tests remain enabled. The tests canonicalize
their fresh temp root, including macOS system temp symlinks; there is no private
profile or hard-coded Linux fixture path. Linux passes all eight helper tests.

[`runtime-v8/VERIFICATION.md`](../../apps/native/crates/runtime-v8/VERIFICATION.md)
contains exact locked build/sign/run steps and the unclosed assertion ledger.
The owned `scripts/verify-macos.sh` validates the S1 artifact/binding hashes,
signs private copies of the production helper and integration test executable,
verifies hardened runtime/exact allow-jit entitlements, then runs the probes.
It reports BLOCKED (exit 2) off macOS/ARM64, never a skipped pass. Python uses
3.9-compatible syntax. This script has shell/Python syntax checks on Linux;
**no macOS codesign, seatbelt or shipping-bundle result is claimed**.

Remaining item-5 work is explicitly blocked, not complete: sandbox-aware hard
external allocation and complete native attribution; real reconciler/cluster
budgets; full synchronous Canvas/text/paint/encoding; real media/resource and
remote-DOM/application bindings. The production recorder dispatcher and engine/
application adapters that must consume these primitives are outside this lane's
owned paths. Preserve every fail/unsupported case and the extension denominator.
No visual/UX changes, DESIGN-TODO placeholders or consumer deletions were made.
Selecting rules: N1/N3 evidence before confinement/performance claims; N4/N6
preserve synchronous extension intent; N7 inspectable ids; N8 owner-only JS.
