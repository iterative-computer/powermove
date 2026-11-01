# P0 housekeeping evidence

Scope: tracked `engine/` and `docs/` at `6806778c9`. Delete only after the last
consumer is gone; retain parity and gate tooling until its retirement gate.
Sizes count tracked UTF-8 text lines (binary assets excluded), before this change.
Only candidates with established deadness are removed.

## Method

- Inventory every engine Cargo manifest and target using the pinned toolchain's
  `cargo metadata --no-deps --format-version 1`. `engine/Cargo.toml` includes all
  **31** `crates/*` packages; only vendored wgpu is excluded. There are no orphan
  engine package directories outside that workspace (FFmpeg is a retained build).
- For each crate, `git grep -l -e <hyphen-name> -e <underscore-name>
  6806778c9 -- .` searches the whole tracked repository, including `engine/`,
  `apps/`, `packages/`, `.github/` and `docs/`. Inspect Cargo path dependencies,
  target declarations, package scripts, `build.rs`, CI and documentation callers.
  Crate rows below list representative consumers, not just textual mentions.
- For the removed examples, whole-repository
  `git grep -n -i -E 'graphics[_ -]footprint|startup[_ -]heap|startup[_ -]footprint'
  6806778c9 -- .` finds **only their own source comments/labels** (two hits each).
  There are no references in manifests, path dependencies, build scripts,
  applications, tests, CI, scripts or doc-build inputs. Cargo auto-discovers them
  as standalone examples; nothing invokes them. Build scripts were also checked
  for dynamic directory/glob discovery; none consumes these example sources.
- Search all tracked docs and engine tools for `prune2`, `prune-2`, measurement,
  benchmark and perf-run material. No tracked prune2 document/directory exists at
  this revision. Existing measurement results are retained, including failures.
  A recorded result alone does not make its reproduction/gate tool dead.

## Removed candidates

| Candidate | Lines | References found | Verdict | Reason |
|---|---:|---|---|---|
| `engine/crates/pm-gpu/examples/graphics_footprint.rs` | 191 | Self only; automatic example target; no external invocation | Delete | One-off native Metal allocation attribution. The settled campaign and GPU-accounting limits are recorded in `docs/engine/memory.md`; current whole-app reproduction remains there. No parity comparison or gate assertion lives in this probe. |
| `engine/crates/pm-node/examples/startup_heap.rs` | 64 | Self only; automatic example target; `dhat` used only here within pm-node | Delete | One-off empty-host/renderer heap attribution. `memory.md` records the 1.40 MiB renderer observation and its limits; `scene3d.md` retains separate DHAT results. Current app attribution through pm-node-addon remains. |
| `engine/crates/pm-render/examples/startup_footprint.rs` | 125 | Self only; automatic example target; no external invocation | Delete | One-off settled renderer attribution. `memory.md` records the renderer lifecycle, allocation counters and current campaign/reproduction. This probe supplies no assertion or legacy oracle. |
| `engine/crates/pm-node/Cargo.toml`: `dhat` dev-dependency | 1 | Only the removed startup heap example in this crate | Delete | No remaining pm-node target imports dhat. |
| `engine/Cargo.lock`: pm-node → dhat edge | 1 | Matches removed dev-dependency | Delete | Cargo updates only this edge. The dhat package remains required by pm-node-addon's `dhat-heap` feature. |

**380 example lines + 2 dependency lines removed.** No crate is deleted, so the
workspace wildcard remains correct. `memory.md`'s failing MEM-1 rows stay open;
this removes retired diagnostics, not their results or current gate harness.

## Crates retained

All are workspace members. A standalone library with tests or tooling consumers
is not proven dead merely because no other Cargo package depends on it.

| Candidate (`engine/crates/`) | Lines | References found | Verdict | Reason |
|---|---:|---|---|---|
| `pm-audio` | 37,173 | `pm-cli`, `pm-export`, `pm-node`; 42 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-bridge` | 4,638 | `pm-bridge-addon`, `pm-node`; 84 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-bridge-addon` | 58 | `pm-bridge/Cargo.toml` packaging note; desktop `scripts/build-engine-addon.mjs`; 14 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-cli` | 10,561 | CLI binary; desktop `scripts/gate-quicktime-color.mjs`; phase-3a gate launcher; 70 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-color` | 3,785 | `pm-media`, `pm-reference`, `pm-render`, `pm-text`; 49 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-edit` | 5,380 | `pm-render`, `pm-wasm`; 41 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-engine` | 9,399 | `pm-node`, `pm-perfbench`; 55 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-eval` | 26,784 | `pm-audio`, `pm-cli`, `pm-edit`, `pm-export`, `pm-node`, `pm-perfbench`, `pm-render`, `pm-wasm`; 176 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-export` | 19,318 | `pm-cli`, `pm-node`, `pm-perfbench`, `pm-render`; 56 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-expr` | 28,955 | `pm-eval`, `pm-render`, `pm-wasm`; 42 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-geom3d` | 33,449 | `pm-eval`, `pm-render`, `pm-wasm`; 52 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-glsl` | 10,050 | `pm-render`; 26 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-gpu` | 8,709 | `pm-export`, `pm-glsl`, `pm-node`, `pm-perfbench`, `pm-render`; 117 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-host` | 1,805 | `tests/contract.rs`; shared fixture read by desktop `host-contract.test.ts`; 14 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-media` | 33,045 | `pm-audio`, `pm-cli`, `pm-export`, `pm-gpu`, `pm-node`, `pm-node-addon`, `pm-perfbench`, `pm-render`; 103 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-model` | 44,076 | `pm-audio`, `pm-cli`, `pm-edit`, `pm-engine`, `pm-eval`, `pm-export`, `pm-expr`, `pm-geom3d`, `pm-node`, `pm-perfbench`, `pm-render`, `pm-wasm`; 328 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-node` | 42,727 | `pm-node-addon`, `pm-perfbench`; 89 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-node-addon` | 114 | `pm-node/Cargo.toml` packaging note; desktop `scripts/build-engine-addon.mjs`; 13 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-perf` | 499 | `pm-gpu`, `pm-node`, `pm-perfbench`, `pm-render`; 55 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-perfbench` | 4,697 | `tests/ratchet.rs`; `docs/engine/perfbench.md`; desktop `perfbench-rig.spec.ts`; 6 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-protocol` | 17,839 | `pm-audio`, `pm-cli`, `pm-color`, `pm-engine`, `pm-export`, `pm-gpu`, `pm-host`, `pm-media`, `pm-node`, `pm-perfbench`, `pm-reference`, `pm-render`; 338 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-reference` | 4,766 | `pm-render`; 42 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-render` | 195,239 | `pm-cli`, `pm-export`, `pm-glsl`, `pm-node`, `pm-perfbench`; 163 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-skia-path` | 7,752 | `tests/skia_{conics,convexity,rect_contours}.rs`; `pm-vector/tools/graphite-oracle/generate.py` names its license; 2 external files match names | Keep | Tests and licensing/tooling references remain; no replacement gate established. |
| `pm-stretch` | 4,730 | `pm-audio`; 12 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-studio` | 4,263 | `pm-render`; 8 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-text` | 20,971 | `pm-bridge`, `pm-cli`, `pm-edit`, `pm-eval`, `pm-node`, `pm-perfbench`, `pm-render`, `pm-wasm`; 96 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-time` | 1,827 | `pm-audio`, `pm-cli`, `pm-edit`, `pm-eval`, `pm-export`, `pm-media`, `pm-model`, `pm-node`, `pm-render`, `pm-wasm`; 101 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-vector` | 21,938 | `pm-eval`, `pm-render`; 47 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-wasm` | 7,345 | `pm-cli`, `pm-edit`, `pm-node`, `pm-wasm-module`; 180 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |
| `pm-wasm-module` | 31 | `pm-wasm/scripts/build.mjs`; `engine-wasm-identity.yml`; 7 external files match names | Keep | Live dependency, executable, test or gate consumer remains. |

## Other candidates retained

| Candidate | Lines | References found | Verdict | Reason |
|---|---:|---|---|---|
| `engine/crates/pm-render/examples/path_mask_perf.rs` | 237 | No name reference outside self; Cargo auto-example; path-mask gates in `perf.md`, `composite.md` and `tests/path_masks.rs` | Keep | No proof its isolated GPU/encode measurement role has been replaced or its retirement gate passed. Unreferenced is insufficient. |
| `engine/crates/pm-gpu/src/glsl.rs` | 1,796 | `pm-render/src/three/{cache,mod,specialize}.rs`, `scene3d/gpu.rs`, `pm-gpu/tests/msl_math.rs` | Keep | Still used by the three.js scene path; not dead duplication of pm-glsl. |
| `engine/crates/pm-render/examples/text_level2_perf.rs` | 249 | `docs/engine/text.md` | Keep | Reproduction of retained text performance evidence. |
| `engine/crates/pm-audio/examples/paused_cost.rs` | 174 | `docs/engine/pm-audio-history.md` | Keep | Documented historical audio measurement; no proven retirement. |
| `engine/crates/pm-cli/scripts` | 405 | `phase3a-gate.md`; scripts call png16 determinism, shader comparison and WASM budget checks | Keep | Gate/parity tooling retained through P2. |
| `engine/corpus` | 9,290 | Corpus collectors, renderer/CLI tests and `docs/engine/corpus.md` | Keep | Legacy visual oracle and gate inputs retained through P2. |
| `engine/crates/pm-render/tools` | 7,205 | Shader/effect corpus collectors; `docs/native/s4-glsl.md`; three.js reference runners | Keep | Parity/oracle generation and S4 inventory consumers remain. |
| `engine/crates/pm-audio/tools` | 1,484 | `docs/engine/pm-audio.md`, `pm-audio-history.md`; playback-gates and temp-root tests | Keep | Current and historical playback/parity reproduction remains. |
| `engine/crates/pm-eval/examples` | 649 | `engine-wasm-identity.yml`; support/check-wasm-identity.ts; `pm-eval.md` | Keep | WASM/native identity gate and benchmark consumers remain. |
| `engine/crates/pm-text/scripts` | 219 | `engine-wasm-identity.yml`; blink_sizes test; `docs/engine/text.md` | Keep | Identity and Blink-reference gate consumers remain. |
| `engine/crates/pm-wasm/scripts` | 240 | `engine-wasm-identity.yml`; desktop build scripts; `pm-wasm/Cargo.toml` | Keep | Production module packaging and identity checks remain. |
| `engine/crates/pm-glsl/examples` | 478 | `docs/native/s4-glsl.md` inventory/supplement reproduction | Keep | Active S4 verification; gate remains open. |
| `docs/engine/perf.md` | 1,719 | `pm-eval.md`, `scene3d.md`, `effects.md` and implementation comments | Keep | Recorded performance gates and unresolved findings; not duplicate disposable output. |
| `docs/engine/perfbench.md` | 603 | `engine/Cargo.toml`, patched wgpu, pm-perf and pm-perfbench consumers | Keep | Defines counters, baseline ratchets and reproduction. |
| `docs/engine/pm-audio-history.md` | 2,831 | `docs/engine/pm-audio.md` history link | Keep | Unique historical evidence and rejected approaches; current summary is not a replacement for all evidence. |
| `docs/engine/spikes` | 4,308 | `rust-engine.md`, effects/GLSL/text/media docs and implementation comments | Keep | Unique decisions and oracle/gate references; not prune2 material. |

Other examples/benches were checked by their filename/stem across the whole
repository: the eval/expr/audio/seek/compose benchmarks, export throughput,
media/ICC comparisons, color-depth and text review probes, WASM identity/digest
examples, bridge font smoke and refresh probe retain documented, CI, test or
script callers. The source-monitor PCM helper is imported by the desktop source
media fixture. None is retired here. The retained engine docs (81 files) contain
contracts, unique evidence or gate reproduction; none was proven disposable.

## Shader documentation

`docs/rust-engine.md` §11 now describes ES validation → normalization/glslang →
SPIR-V rewriting/reflection → naga/MSL, bounded killable compiler processes,
cached finishing, named failures and the remaining in-process Metal compiler
gap. It preserves the shader contract and makes WebGL the last compatibility
route. SPIRV-Cross and the new native compat-page fallback are not claimed shipped.
See `s4-glsl.md` for the complete, still-open gate.

## Verification

- Rust 1.96.1; `source engine/env.sh`, shared external target directory.
- `cargo check --workspace --all-targets`: **PASS** (all 31 workspace crates).
  The two addon build scripts emit their expected no-Node-feature warnings.
- `cargo test -p pm-model -p pm-glsl`: **PASS**, 176 tests including doctests;
  one existing opt-in benchmark is ignored. GLSL includes the five process
  isolation tests and both shader-probe/mutant tests.
- `cargo test -p pm-model -p pm-eval -p pm-glsl`: **FAIL** at the existing
  `pm-eval/tests/color_ink.rs:39` fixed text-bounds assertion (case 519), after
  42 evaluator unit tests and caption parity passed. Actual bounds:
  `[12.98828125, -175.125, 660.671875, 22.625]`; expected:
  `[15.72265625, -175.125, 693.875, 22.625]`. The evaluator, text, model and GLSL
  source trees are unchanged from the audited revision; no fixture is weakened.
- `cargo test -p pm-eval --lib`: **PASS**, 42 tests.
- `cargo test -p pm-eval --test color_ink`: **FAIL**, reproducing the same
  assertion and bounds. The original test executable also reproduces it.
- `git diff --check`: **PASS**. Staged changes are inspected and scanned with
  the required gitleaks command before each commit; the pre-commit hook is enabled.
