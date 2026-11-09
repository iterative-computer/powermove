# Runtime2 verification (2026-11-04)

This lane has **not closed item 5**. Linux tests establish the specific fixes
below, not final macOS confinement, complete legacy Canvas/resources or the
production reconciler/39-extension budgets. No visual/UX changes or design
placeholders were introduced.

## Reproducible Linux checks

From the repository root, after the session's tool activation:

```sh
cargo test --locked --manifest-path apps/native/Cargo.toml \
  -p pm-runtime-v8 -p pm-recorder
cargo clippy --locked --manifest-path apps/native/Cargo.toml \
  -p pm-runtime-v8 -p pm-recorder \
  --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --check
cargo fmt --manifest-path apps/native/Cargo.toml -p pm-recorder --check
bash -n apps/native/crates/runtime-v8/scripts/verify-macos.sh
```

Expected suite count: **199 pass / 0 fail / 0 ignored**: 61 runtime unit,
eight helper integration, 94 recorder, 15 raster and 21 golden tests. The count
includes gated subprocess entry tests; their parent tests launch the actual
denial/JIT, fatal-OOM, heap/external-limit and snapshot-reuse probes. All tests
use public/synthetic inputs. No new dependencies or lockfile changes are needed.

The cross-kind snapshot regression deterministically exited with SIGSEGV before
the read-only snapshot flag fix. Keep it enabled on both platforms. The abort
regressions check actual host start/cancel counts and outstanding capacity;
they do not merely compare accounting arithmetic.

A full replay under concurrent release compilation also exposed an idle-test
scoping error: the fixture's 30-ms watchdog can legitimately notice its **initial
active task** under contention, while the test expected zero lifetime notices.
It now records execution IDs and requires every observed notice to belong to
that initial task, with no active execution and no additional kernel calls in
the idle window. No watchdog deadline/tolerance was changed and the timer-wake
assertions remain enabled. This is an idle assertion, not a startup timing gate.

## macOS ARM64 signed diagnostic probes

Prerequisites: macOS 14+, Xcode tools/codesign, Python 3.9+, Rust **1.96.1**,
the verified S1 ARM64 sandbox archive **and its matching generated bindings**,
and at least 8 GiB free space. Respect the Mac's build lease/PAUSE_LOCAL rules.
Use a normal terminal; no UI/screen/installed app/profile is required here.

```sh
# Set both to existing, verified S1 artifact files; the script checks SHA-256.
export RUSTY_V8_ARCHIVE=/ABSOLUTE/S1/librusty_v8.a
export RUSTY_V8_SRC_BINDING_PATH=/ABSOLUTE/S1/src_binding.rs
unset V8_FROM_SOURCE
# Optional: an already authorized local signing identity for private copies.
# Omission uses ad-hoc signing and is diagnostic evidence only.
# export PM_RUNTIME_SIGN_IDENTITY='Developer ID Application: ...'
flock /tmp/pm-build.lock \
  bash apps/native/crates/runtime-v8/scripts/verify-macos.sh

# Also replay every runtime/snapshot regression against the Mac sandbox build.
flock /tmp/pm-build.lock cargo +1.96.1 test --locked \
  --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml \
  --features macos-sandbox
```

The script refuses Linux and an incompatible architecture with exit **2**.
An Intel run needs its independently verified matching archive/binding pair;
the ARM64 artifact is never silently substituted. Artifact source builds,
checksum changes, or extra signing entitlements are not accepted by this probe.

It builds with `--locked`, locates the production `pm-store-helper` and helper
integration test executables through Cargo JSON, and copies/signs only those
fresh private copies. It verifies hardened runtime and exactly **allow-jit**
(no unsigned-executable-memory or library-validation bypass), then runs all
eight helper tests against the signed helper override. Its freshly created,
canonicalized scratch path is printed; evidence stays there on success/failure.
The script never signs, overwrites or launches an installed shipping app.

The confinement test uses the **same production helper initialization and
PlatformStoreSandbox**, after trusted V8 bootstrap and before extension input.
Before confinement it proves a synthetic file is readable/writable, an ephemeral
loopback listener is reachable, and a shell exits with the expected value.
After confinement every read/write/connect/spawn attempt must return **EPERM or
EACCES**. Missing paths, closed ports, unexpected errors and successful operations
fail the test. A real optimized function must return the correct value and both
pinned TurboFan optimization bits (8 and 32). Native test syntax is enabled only
in the test child, not in the production helper. The parent checks a platform
report (`seatbelt=true` on macOS, `seccomp=true` on Linux) and no forbidden file.

Other helper tests exercise inherited-pipe IPC, actual production snapshot
execution, host forwarding/abort, process shutdown/restart and real fatal V8
heap OOM while trusted parent and sibling state survive. This is broader than
checking a sandbox initialization return code. It still does not test every
Mach/TCC operation, every OS version, or external-allocation exhaustion.

Repeat with final app/helper identities, shipping entitlements, quarantine,
Gatekeeper/notarization and every supported OS/architecture before sign-off.
An ad-hoc signed copied diagnostic executable cannot certify the shipping bundle.
Record platform/architecture, commit, artifact hashes, signing mode and test
counts as public aggregates; keep raw diagnostic/signing logs local.

## Unclosed assertions and required owning integrations

| Gate | Concrete remaining assertion |
| --- | --- |
| Hard external memory | All sandbox ArrayBuffer/resizable/shared/Wasm allocations and native pixels/codecs are charged **before** allocation and released on actual free, including retained generations. The pinned Rust allocator API is disabled in sandbox mode; this needs a sandbox-aware allocator/page hook and a matching verified CI artifact. Sampling/host capacity reservations are insufficient. |
| Attribution | Fetch/engine capacity is now exposed per generation, but recorder tiles, paint/source leases, decoded/encoded media, shared snapshots, platform allocations and process physical footprint are not fully attributed. Do not add overlapping categories and report the sum as RSS. |
| Reconciler/cluster budgets | Actual production bundles, all six atomic operations/native validation, Refresh state, frame paint/presentation, retained-node memory and the real 39-extension workload must run in a quiet Mac window. Keep the provisional 4-ms reconciler ceiling and existing failed measurements visible. This lane supplies no new reconciler timing pass. |
| Synchronous Canvas/text/paint/encoding | Regional conversion is fixed. Recorder shadow capture/automatic pre-clip source-alpha replay, full font/style-source/variation/color-glyph behavior, context color/alpha formats, filters, Path2D/paint wrappers, PNG/JPEG/toBlob scheduling, sandbox pixels and promotion identity still need complete binding tests. Existing native primitives do not certify these contracts. |
| Media/resources | The production dispatch lives in `compat-runner/src/recorder.rs` and application/engine-service adapters, outside this lane's owned paths. Complete immutable decoded image/video/font/resource versions, origin/taint, bitmap close/transfer, permission/cancellation and engine upload/presentation there. Its current explicit unsupported cases cannot be replaced by success-shaped placeholders here. |
| Remote DOM | Fresh synchronous layout, same-element native canvas promotion, event/default semantics, native font/asset loading and WebKit lifetime/capture proof require the actual remote-DOM/application owners. JS forwarding tests alone are insufficient. |
| Signed confinement | Run the provided Mac probes, then verify the final signed shipping bundles on the supported OS/architecture matrix. Linux seccomp/JIT and fatal heap-OOM isolation do not certify macOS or bound arbitrary external allocation. |

Decisions: N1/N2 require real budgets and resource bounds; N3 preserves exact
pixels and fixes invalid snapshot sharing; N4/N6 retain every extension and
synchronous behavior; N7 exposes id/generation diagnostics; N8 keeps V8 and
native sync adapters off the main thread. The prune rule retains every existing
consumer. None of these open assertions is converted into a pass.

## Final Linux replay

Merged base: `ed5482622f41c533199a724dcb8c8738dd4dcc93`; the final fetch/merge
reported already up to date, without conflicts. The complete debug and release
workspace replays each pass **199 / 0 / 0** (pass/fail/ignored). Release command:

```sh
cargo test --locked --release --manifest-path apps/native/Cargo.toml \
  -p pm-runtime-v8 -p pm-recorder
```

The debug command, three-package strict all-target Clippy and package formatting
commands above also pass after integration. The Mac launcher passes Bash syntax,
all three embedded Python blocks parse as Python 3.9, and the entitlement plist
parses with exactly allow-jit. The actual Linux launcher returns **BLOCKED / 2**,
as required. macOS remains unexecuted. Item 5 remains **BLOCKED** for the hard
allocator, complete compatibility/resource integrations and real Mac budgets
listed above; passing these Linux suites is not a phase sign-off.

## Headless ARM64 Mac verification — 2026-11-04

macOS **27.3**, ARM64, Rust **1.96.1**, `macos-sandbox` enabled. Recovered the
binding from the documented existing S1 CI artifact (run 41286550390); the surviving
archive and recovered binding both match the pins above. The verified pair is
stored durably at `$HOME/Developer/pm-v8-s1/`, with `V8_FROM_SOURCE` unset. No V8
source rebuild, workflow dispatch, owner data, installed app, Keychain access or
UI/timing gate was used.

The first signed diagnostic attempt failed **before running its eight tests**:
the launcher searched for literal `(runtime)`, but this Mac's genuine hardened
ad-hoc signature reports `flags=0x10002(adhoc,runtime)`. The verifier now parses
the CodeDirectory hexadecimal flags and requires `CS_RUNTIME` (`0x10000`). The
exact allow-jit-only entitlement comparison is unchanged. Four executions of the
actual verifier block accepted combined/standalone runtime flags and refused
ad-hoc-only/missing flags. No signing permission or entitlement was broadened.

| Final check | Pass | Fail | Ignored |
| --- | ---: | ---: | ---: |
| Standalone sandbox runtime: unit + helper integration | 69 (61 + 8) | 0 | 0 |
| Workspace runtime + recorder + raster/golden | 199 | 0 | 0 |
| Hardened ad-hoc signed release helper integration | 8 | 0 | 0 |

Strict three-package all-target Clippy, all three isolated formatting checks and
launcher Bash syntax passed. The signed production initialization probe reported
`platform=macos`, `seatbelt=true`, TurboFan status **41** (required bits 8 and 32),
and successful optimized execution. Read/write/connect/spawn worked before
confinement and returned EPERM/EACCES afterward. Forbidden file creation was
absent. Snapshot reuse, inherited-pipe IPC, cancellation/capacity and fatal helper
OOM isolation passed; the expected fatal-OOM subprocess is not a parent failure.

Environment: source `engine/env.sh`, shared target, four build jobs,
`PM_MEDIA_NATIVE_PYTHON=/opt/homebrew/bin/python3.14`, canonical
`TMPDIR=/private/tmp/pm-native-tests/`, and
`CARGO_PROFILE_TEST_DEBUG=line-tables-only` matching development output. All
commands below used `nice -n 20 flock /tmp/pm-build.lock`; admission honored
`PAUSE_LOCAL` and the disk reserve. Tests ran serially to limit peak resource use;
no assertion, case or deadline changed.

```sh
export RUSTY_V8_ARCHIVE="$HOME/Developer/pm-v8-s1/librusty_v8.a"
export RUSTY_V8_SRC_BINDING_PATH="$HOME/Developer/pm-v8-s1/src_binding.rs"
unset V8_FROM_SOURCE
nice -n 20 flock /tmp/pm-build.lock bash apps/native/crates/runtime-v8/scripts/verify-macos.sh
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 test --locked --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --features macos-sandbox -- --test-threads=1 --nocapture
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 test --locked --no-fail-fast --manifest-path apps/native/Cargo.toml -p pm-runtime-v8 -p pm-recorder --features pm-runtime-v8/macos-sandbox -- --test-threads=1
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 clippy --locked --manifest-path apps/native/Cargo.toml -p pm-runtime-v8 -p pm-recorder --features pm-runtime-v8/macos-sandbox --all-targets --no-deps -- -D warnings
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 fmt --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --check
nice -n 20 flock /tmp/pm-build.lock cargo +1.96.1 fmt --manifest-path apps/native/Cargo.toml -p pm-recorder --check
nice -n 20 flock /tmp/pm-build.lock bash -n apps/native/crates/runtime-v8/scripts/verify-macos.sh
```

This closes only this lane's headless Mac verification. **Item 5 remains open**
for the hard external allocator, complete production resource/compatibility
bindings, actual reconciler/39-extension budgets and the other owning integrations
listed above. Hardened ad-hoc diagnostic copies do not certify shipping identities,
notarization/Gatekeeper, other supported OS versions or Intel. Heap statistics do
not establish physical footprint or the 20-MB budget.
