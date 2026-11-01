# P2 extension checks — 2026-11-01

**OPEN / NOT PASSED.** No extension has paired Electron/native trace, visual
oracle and scripted-interaction evidence. The new executable removes the
previous missing V8 bundle-loader blocker; it does not complete the kernel,
WebKit replay or Store facade adapters.

Reference: frozen `v1.2.0`, `07ed5fa17ee640659cb075f9cad1af2095011d60`.
Base includes the dated macOS verification in `runtime-v8.md`. The local S1
sandbox archive and matching bindings passed all five checksum checks. Rust
1.96.1, shared target and build lock were used. No release, app update, tag or
deployment was created.

## Population and results

| Source | Total | Match | Mismatch | Can't run | Logic executed in real V8 | Activated |
|---|---:|---:|---:|---:|---:|---:|
| Owner installations | 44 | 0 | 0 | 44 | 44 | 30 |
| Local public corpus | 10 | 0 | 0 | 10 | 9 | 6 |
| Published Store, latest releases | 4 | 0 | 0 | 4 | 0 | 0 |
| **Total** | **58** | **0** | **0** | **58** | **53** | **36** |

These are separate populations, without cross-source deduplication. Owner
discovery counted immediate manifest-bearing installations, not the enabled
set; their versions were 24 v1 and 20 v3. Nested manifests and directories
without manifests are not separate installations. The public Store snapshot
contained four latest releases; complete archive/tree/file hashes and sizes
were checked, and the final catalog matched the initial catalog. Store reads
were anonymous GETs only. The connector required an editor session, so collection
used the public routes already documented in C-MATRIX.

Every one of the 58 reference attempts was blocked by another lane's GUI lock;
the screen check itself was unlocked. This lane did not take that session or
bypass its lock. Earlier disposable reference startup probes established a
correct instrumented frozen build, but did not produce accepted captures.
There are **zero accepted reference/native traces, screenshots or full scripts**.

On the native side, all 44 owner sources entered the actual compat bundle in
`pm-runtime-v8`; 30 activated. Nine local fixtures entered V8 and six activated.
The remaining local fixture is rejected by the frozen manifest parser because
it declares permissions with apiVersion 1. This is a baseline/input blocker,
not a native behavior mismatch. Store sources compiled, but were not run through
the trusted owner facade: their permissioned Store adapter is still missing.
No installed Store provenance, account state, variables or durable storage was
copied into a test profile.

Native blockers overlap reference blockers and must not be added to the totals:

| Native blocker | Cases | Family / required work |
|---|---:|---|
| Activation completed; no WebKit/scripted native capture | 36 | DOM panel, lifecycle: mount actual contributions, apply/ack remote DOM in WebKit, deliver real events and capture paint |
| Activation needs app capabilities/providers | 17 | PM facade and sync services, with raster/DOM consumers: attach the kernel, built-in provider closure and engine backends |
| Frozen manifest rejects a local fixture | 1 | Lifecycle/input: preserve the frozen rejection; test a valid version separately |
| Permissioned Store facade/helper unavailable | 4 | Lifecycle and PM facade: implement Store-specific admission and reference provenance; never substitute trusted execution |

No top **measured mismatch** family can be ranked because no pair completed.
The two independently reproduced defects below are synthetic component defects,
not claims about any owner's extension. Missing backends remain `can't run`;
there are no unsupported-extension exclusions.

## Executable and harness

[`tools/extcheck`](../../tools/extcheck/README.md) supplies read-only source
collection, ordinal temporary cases, frozen compiler/import-boundary reuse,
reference trace-recorder driving, script/PNG/trace comparison, fixed public
reason codes, per-launch screen locking and cleanup. Match admission requires
two equal captures per role, completed balanced traces, complete interaction
outcomes and decoded premultiplied-RGBA pixels. The default smoke script is
explicitly incomplete. No visual tolerance/design decision was made.

`apps/native/crates/compat-runner` is the new `pm-compat-runner` executable and
library. It loads the production compat bundle, installs its globals, compiles
extension sources with the frozen compiler as a build tool, and activates v1–3
modules inside the actual V8 owner. Shared Svelte runtime modules are bundled
only when imported, rather than eagerly requiring unused browser capabilities.
It awaits activation, preserves same-realm service/storage behavior across
replacement, disposes contributions, captures activation errors, and cleans
partial registrations after a throw. No FakeHost or Node runtime is linked.

The native recorder bridge executes a limited command subset through
`pm-recorder`, including actual CPU raster/readback and pinned immutable
drawImage sources. Unsupported operations explicitly fail. The V8-side real
remote-DOM shim emits bounded batches and accepts owner-thread feedback. An
eight-test native suite proves that event closures run in actual V8; it does
not prove WebKit rendering. The CLI reports its missing gates and does not
claim a completed native capture.

## Small fixes and synthetic reproductions

| Family | Before | Fix and proof |
|---|---|---|
| Lifecycle / bundle loading | The documented Bun 1.3.14 build silently ignored `--global-name`; its IIFE exposed no `PowermoveCompat`, so the real V8 loader failed with a missing global. | An explicit browser entry publishes the production namespace. A bare-V8 bundle regression test and the native runner tests now exercise it. |
| DOM panel | `Range.selectNodeContents` on a comment selected length 0 instead of the comment's UTF-16 data length. | The remote range handles comment data like text data. The original synthetic `Synthetic 🦀` comment reproduced 0 versus 12 against the independent DOM implementation, then passed after the fix. |

Larger missing paths have minimal synthetic probes in the native suite: a
default activation that calls `api.history.begin('Synthetic')` reaches the
explicit unavailable kernel adapter; a DOM input event updates an output in
the V8 mirror but has no WebKit presentation endpoint; drawing a rectangle and
copying its pinned source proves native pixels but does not establish the full
Canvas/text/paint/encoding/global contract. Those require owning-adapter work,
not changes to extension source or reference goldens.

## Validation and remaining gate

| Check | Result |
|---|---|
| Existing sandbox V8 runtime suite | 61 pass: 55 unit + 6 integration |
| New native compat runner | 8 pass, actual production bundle, V8 DOM event closures and native pixels |
| Compat JS suite | 174 pass, including the new bundle regression |
| Remote-DOM JS/Rust boundary suite | 37 pass, including the new range regression |
| Harness unit suite | 10 pass: trace validity/types/order, privacy-safe differences, symlink/traversal refusal, decoded visual comparison and missing evidence |
| Frozen compiler → bundled module → actual V8 CLI | Pass, original synthetic extension with omitted `entry`, shared compat/DOM bundle, command/storage result and disposal |
| Native runner Clippy | Pass with all targets, no dependencies, warnings denied |

The complete task remains open: wire actual kernel publications/leases and
the connected built-in providers; finish recorder/WebKit/Store adapters; author
reviewed per-extension projects and interaction scripts; obtain a free GUI slot;
capture repeatable frozen traces/PNGs and run the same scripts natively. The
current bundled native adapter performs activation/disposal only. It cannot
certify Gate P2. Private inputs and raw per-case artifacts were confined to
temporary copies and removed; only these aggregate counts and original
synthetic code enter Git.
