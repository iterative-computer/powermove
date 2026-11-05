# Runtime2 verification (2026-11-04)

This lane has **not closed item 5**. Linux tests establish the specific fixes
below, not final macOS confinement, complete legacy Canvas/resources or the
production reconciler/39-extension budgets. No visual/UX changes or design
placeholders were introduced.

## Reproducible Linux checks

From the repository root, after the session's tool activation:

```sh
cargo test --locked --manifest-path apps/native/Cargo.toml \
  -p pm-runtime-v8 -p pm-recorder -p pm-recorder-raster
cargo clippy --locked --manifest-path apps/native/Cargo.toml \
  -p pm-runtime-v8 -p pm-recorder -p pm-recorder-raster \
  --all-targets --no-deps -- -D warnings
cargo fmt --manifest-path apps/native/crates/runtime-v8/isolated/Cargo.toml --check
cargo fmt --manifest-path apps/native/crates/recorder/isolated/Cargo.toml --check
cargo fmt --manifest-path apps/native/crates/recorder-raster/isolated/Cargo.toml --check
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
