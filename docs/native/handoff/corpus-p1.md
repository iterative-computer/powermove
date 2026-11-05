# Corpus P1 handoff — WIP

The incoming runner owns the next whole-project run. Work on this lane stopped at handoff. **P1 is open.** No new real-project results supersede the historical matrix below. Do not treat the WIP compiler-bundle changes as seven confirmed fixes yet.

## Run a folder of local projects

Use a Mac with Metal, the pinned toolchain, FFmpeg from `engine/env.sh`, and at least 8 GiB free before building. Keep at least 6 GiB free during a batch. Do not start builds/renders if `/tmp/pm-shots/PAUSE_LOCAL` exists. While `/tmp/pm-shots/USER_ACTIVE` exists, stay headless, use `nice -n 20` and four Cargo jobs. No screen checks are needed for this runner; any separate screen run must check `is_locked.sh` and own `gui.lock` exclusively.

From the repository root:

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export CARGO_BUILD_JOBS=4

(
  cd apps/native
  nice -n 20 flock /tmp/pm-build.lock cargo build --locked -p pm-native-app --example pm-corpus
)
(
  cd engine
  nice -n 20 flock /tmp/pm-build.lock cargo build --locked -p pm-glsl --bin pm-glsl-compiler
)

# An empty source profile prevents folder-only runs from also reading the
# current user's default Electron library. Keep its path for any resume.
PM_CORPUS_EMPTY_PROFILE="$(mktemp -d /private/tmp/pm-corpus-input.XXXXXX)"
mkdir -p "$PM_CORPUS_EMPTY_PROFILE/store"

nice -n 20 python3 tools/corpus/run.py \
  --binary "$CARGO_TARGET_DIR/debug/examples/pm-corpus" \
  --compiler "$CARGO_TARGET_DIR/debug/pm-glsl-compiler" \
  --profile "$PM_CORPUS_EMPTY_PROFILE" \
  --projects "/ABSOLUTE/PROJECT_FOLDER" \
  --batch-gib 1.5 --timeout 240 \
  --retain-for-debug \
  --report /tmp/pm-corpus-folder-summary.json
```

Replace only the project-folder placeholder. Embedded PMV media is supported. Linked media must already be local and resolvable; relative/stale links and original-versus-proxy binding still need investigation. `--projects` replaces the default file-search roots but does **not** suppress the library selected by `--profile`, hence the empty profile above. To include a real library intentionally, use its read-only source profile instead.

The driver exclusively owns `/tmp/pm-corpus`, mode 0700, and takes `/tmp/pm-corpus-run.lock`. It refuses an existing root unless explicitly resumed. Never delete another lane's root. It copies project inputs before kernel open, selects referenced payloads per project and never copies the whole `.blob` directory. Completed batch media is deleted immediately; `--retain-for-debug` keeps metadata and diagnostics only. Inspect private diagnostics locally; never stage them. Remove the owned snapshot and empty input profile when finished.

For a resource/pause resume, repeat the same command with `--resume-snapshot --resume-results`, the same profile path and report path. `--only-ids ID,ID` selects a focused replay. A focused or mixed-provenance run cannot close P1. Rebuilding either executable changes provenance: use a fresh result report for a new certification attempt.

File IDs are SHA-256 prefixes of absolute source paths; library IDs are BLAKE3 prefixes of `library:<slot-id>`. Moving projects changes file IDs. The private discovery/inventory files provide the source context; correlate privately and never commit that mapping.

**Do not download or hydrate the 18 iCloud/dataless inputs.** The current driver checks Darwin's `SF_DATALESS` flag before payload reads. Local availability is the owner's decision, not a test-runner repair.

## Recorded results

Historical audit: 92 discovered entries (47 library slots plus 45 external PMV entries, not deduplicated authored projects); 34 confirmed extension-dependent entries were P2. Of 58 candidates, **26 passed every sampled stage**, including one ancillary capture warning; 25 had no capture warning. Nineteen primary inputs failed capture: **18 iCloud/dataless placeholders and one unavailable source**. Their extension status remains unknown.

| Stage | Pass | Fail | Blocked |
|---|---:|---:|---:|
| Capture/open | 39 | 19 | 0 |
| Render | 28 | 7 | 23 |
| Play | 28 | 0 | 30 |
| Scrub | 28 | 0 | 30 |
| Audio | 34 | 1 | 23 |
| MCP edit | 39 | 0 | 19 |
| Save/reopen | 39 | 0 | 19 |
| MP4/decode | 26 | 9 | 23 |
| ProRes/decode | 26 | 9 | 23 |

The 32 candidates without all-stage passes were classified as 19 input-capture failures, **7 GLSL fallback cases, 5 unresolved-media cases and 1 rotated-video case**. These are entry counts, not unique authored projects or failed-stage counts. The consolidated audit used one frozen native worker but multiple driver-reporting versions and had a discovery timeout; it was never a P1 certification.

The 18 primary dataless-input IDs (class `source_not_local`; filesystem availability / driver `source_stat` guard) are:

`0a2681304224e9ac` `0aaf78b5ca8c8d3a` `12d2038ab45930f1` `329aa4334946090c`
`51f7a2405de3fc96` `5b3631592fb0e66e` `792b57e8cb4d6f87` `7dbb31affaf298d6`
`8af52e9d6db4a6a2` `8e954f016b3ee78f` `99ebfb70a042321f` `9e777ba588db52f6`
`a63eb19e9a18ef56` `c64d523347c50f49` `c65ac925907eba9b` `d3a26fa242d9270e`
`e194113606334042` `fab5a1847f8dcc1f`

The additional unavailable primary input is `ee20826fe4919b91` (class `source_unavailable`; source probe/capture, eligibility unverified). The 34 P2 IDs remain listed in the [historical anonymous evidence](../corpus-p1.md#anonymous-evidence).

## Fixed and validated

A fixed stage does not imply the entire entry passed.

| Anonymous ID / synthetic case | Failure class | Fix and code area |
|---|---|---|
| `0748a0f955d574ed` | font / export | Registered the already-pumped headless font broker's wake callback in `apps/native/crates/engine-service/src/jobs.rs`. A pre-fix text export produced zero bright glyph pixels; the pixel regression and real all-stage replay passed. |
| `0652bac842a7e562`, `898d29b5b621180b` | open / harness | Runner now accepts kernel safe-copy decisions on private copies. Both passed all sampled stages. `tools/corpus/main.rs`. |
| `899db5132a0d80a5`, `9501b730bc7ef8d2` | open / harness | Safe-copy open handling fixed; their later GLSL failures remain open below. `tools/corpus/main.rs`. |
| `67f4b158fb98f6d4` | dependency classification | Native built-in definitions are kept in P1 instead of treating every extension-shaped layer as P2. All sampled stages passed. `tools/corpus/main.rs::extensions`. |
| `a473e29fabf99677` | perf / worker deadline | Passed every stage with the 240-second worker limit. This is a harness deadline adjustment, not proof of latency SLOs. |
| Synthetic legacy-text fixture | open / legacy selection | Legacy composition-ID fallback and font servicing passed all nine stages. `tools/corpus/main.rs`, `synthetic.py --worker-only --legacy --text`. |
| Synthetic snapshot fixtures | migration / copy safety | Deferred, verified blob materialization avoids whole-library payload copies; changed sources, existing destinations and dangling symlinks are refused. Complete-snapshot identities and normal migration policy are preserved. `apps/native/crates/migrate/src/snapshot.rs`. |
| Synthetic migration fixtures | temp paths / ENOSPC | Canonical test roots, explicit reserve errors and interruption diagnostics are validated as described below. |

## WIP GLSL diagnosis and changes

**The earlier claim that all seven GLSL rows require new renderer fallback support is unverified.** A concrete runner packaging error was found: only the worker executable was frozen. `pm_glsl::compiler::default_pool` looks for `pm-glsl-compiler` beside the executable (or at `POWERMOVE_GLSL_COMPILER`). Relocating the worker alone therefore loses its compiler and can masquerade as a GLSL fallback capability failure.

The WIP driver now freezes both tools, passes an explicit compiler environment to workers and export children, records the helper checksum, and fails setup when the helper is missing. Fifteen driver tests passed, including a synthetic child process that still runs the frozen helper after both original executables are deleted. **The real seven-entry replay and native shader end-to-end verification have not been completed.**

Additional WIP changes retain selected, private kernel-opened asset descriptors in `asset-request.private.json` with failure diagnostics. They are intended to compare actual opened assets with pre-open inventory selections; they are not a completed media-resolution fix. The worker build completed, but full native suites have not been rerun after these P1 diagnostic changes.

`synthetic.py --effect glow`, `--effect shadow` and `--rotated` now expect successful all-stage runs. `--expect-refusal` explicitly requests the old refusal proof. These expectation changes are WIP; rotation is not implemented and native effect proofs were not rerun. `--compiler PATH` supports a relocated worker. `--offline` still expects missing-original refusal.

| Anonymous ID | Historical class | Suspected area / work remaining |
|---|---|---|
| `779c9324da71a72d` | GLSL fallback | Retest with the frozen helper; `tools/corpus/run.py::freeze_tools`, `pm_glsl::compiler::default_pool`. |
| `8018e392aa13c381` | GLSL fallback | Same; render samples passed but exports refused. Inspect compiler diagnostics before changing effects. |
| `899db5132a0d80a5` | GLSL fallback | Same; safe-copy open already works. |
| `9501b730bc7ef8d2` | GLSL fallback + unresolved payload capture | Same; also investigate its snapshot-payload copy warning. A later successful blob attempt can overwrite the single `blob.private.log`; missing detail is not proof that source bytes were absent. |
| `9d4d9482e8484ebb` | GLSL fallback | Same; includes built-in gradient/blur translation. |
| `e5a886f3a988c8ef` | GLSL fallback / shader | Same; preserve actual compiler failure stage rather than assuming a WebGL-host requirement. |
| `f9670f2ae7843523` | GLSL fallback / shader | Same. The earlier disk-reserve blocker is not the final diagnosis. |

If genuine translation failures remain with a working helper, inspect `engine/crates/pm-glsl/src/compiler.rs`, `pipeline.rs` and `engine/crates/pm-render/src/effects`. Do not remove capability guards or substitute blank rendering to pass the gate.

## Still-open rotation and media cases

| Anonymous ID | Failure class | Suspected code area / next check |
|---|---|---|
| `26449ed9778ccc7b` | rotated-video capability | `engine/crates/pm-render/src/media.rs::video_supported` rejects default rotated footage. The interpreted path and `MediaCache::video_inner` already apply quarter-turn geometry. Extend the existing exact-rotation/FFmpeg reference proof in `engine/crates/pm-render/tests/footage.rs` to the default path before lifting the guard. No rotation code was changed. |
| `2558fbe470ad20de` | offline media + dataless capture | Compare kernel-opened asset descriptors with selected media; a required associated/source input was not local. `tools/corpus/main.rs::catalog`, `project_assets`, driver capture. Do not hydrate it or promote a proxy. |
| `4005c00c421c75ef` | offline media + unavailable source | Same catalog/binding investigation; determine whether any verified original is already available in the copied stores. |
| `ab3e535075c3b8c7` | offline media + unavailable source | Same. |
| `cc165f3b87a3263c` | offline media + unavailable source | Same. |
| `d39a085b7fc00d8d` | offline media without a capture error | Pre-open inventory reported no used assets, yet the opened session reported offline originals. Compare the library reader's journal-applied project and actual published snapshot with inventory selection. Do not call this a decoder defect without a reduced repro. |

Also inspect `apps/native/crates/app/src/media.rs::for_session` (currently portable media and linked paths), `apps/native/crates/first-launch/src/closure.rs` and native Media Store key/object binding. The harness has its own catalog implementation, so distinguish a harness selection defect from missing app bindings or unavailable data. A cached rendition marked as a proxy must not become an export original merely because bytes exist.

`ce45a90483a44298` passed all samples but retained a dataless associated-input warning. It is not a clean capture pass. The gate validator now refuses capture warnings even when all sampled stages pass.

## Migration-test status

`origin/feat/native-engine` was merged in `22afc006f`; migration fixes were pushed in `9c17e0788`.

The unchanged interruption test passed with inherited TMPDIR, Darwin's default TMPDIR, and `/private/tmp/pm-test-tmp/`. The orchestrator also reproduced those passes and confirmed the earlier failure occurred with about 3 GB free. There is **no reproduced regression from deferred snapshot changes**; an early space/write failure prevented the intended crash-injection hook.

Changes retained:

- `tests/support/mod.rs` canonicalizes fixture and temp roots, including Darwin's `/var` alias. Production source protections were not relaxed.
- Reserve failures preserve an actual `io::Error` with `libc::ENOSPC`, plus available/reserve/payload byte counts. Both the normal 4 GiB floor and caller-specified higher floors remain enforced.
- Fixture capture prints the full error chain. The interruption assertion identifies an error that occurred before its selected checkpoint instead of just failing `assert!(interrupted)`.
- A synthetic capacity test proves default/custom reserve violations retain ENOSPC without filling the disk.

Validation at `9c17e0788`, with Darwin's default TMPDIR: **104 passed, 0 failed, 1 existing manual large-project test ignored**. Migrate: 28; engine-service: 33; app including corpus-example tests: 43.

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
export CARGO_BUILD_JOBS=4
(
  cd apps/native
  TMPDIR="$(getconf DARWIN_USER_TEMP_DIR)" nice -n 20 flock /tmp/pm-build.lock \
    cargo test --locked -p pm-native-migrate -p pm-engine-service -p pm-native-app --all-targets
)
```

This native-suite result precedes the unfinished P1 worker-diagnostic edits. Re-run after completing them. Shared-space failures must remain explicit, not be hidden by lowering reserves or skipping a failing case.

## Next steps

1. Rebuild both executables; run driver safety tests (`PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/corpus -p test_runner.py`). Keep all real data and diagnostics outside Git.
2. Prove the relocated worker can compile/render/export generated glow and shadow with its frozen helper, then replay the seven GLSL entries or their privately correlated folder copies. Replace historical diagnoses only with measured results.
3. Add a generated default-path rotated-video pixel comparison and preview/export proof; then fix the narrow rotation issue and replay its case. Preserve non-quarter-turn and unrelated media guards.
4. Use the private asset descriptors to reduce each of the five media failures. Fix selection/binding defects with synthetic fixtures; keep genuinely unavailable originals blocked. Do not download dataless files or relabel proxy bytes as originals.
5. Run the local project folder through every stage using one stable worker/helper pair. A fresh full run is required for certification; no subset, mixed tooling or incomplete discovery can close P1.
6. Re-run the native suites with default TMPDIR, update the anonymized aggregate report, remove owned private copies, stage by path, inspect the diff and run `gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow`. Push only `git push origin HEAD:refs/heads/feat/na-corpus`; commit messages end with the required co-author trailer.

## 2026-11-04 — cloud corpus2 synthetic reductions

This dated section supersedes only the WIP tooling/diagnosis notes above.
**P1 remains open. No private project was available or replayed. Historical
counts and all seven shader, five media and one rotation entries remain open.**

Implemented on `feat/na-cloud-corpus2`, starting from corpus
`4aa346755f7dd6b127eec7521e7c1ac8044978b5` and merging native-engine
`ed5482622f41c533199a724dcb8c8738dd4dcc93`:

- Tool freezing now uses a bounded independent stream when `clonefile` is absent,
  verifies existing frozen bytes/executable permissions and refuses symlink tools.
  A real relocated pair (portable inspection worker plus production compiler)
  translates **20/20 public builtin shaders to naga and MSL after both original
  executable copies are removed**. This establishes helper IPC/translation,
  **not Metal compilation, native-worker pixels or encoded output**.
- Seven shader-family reductions cover glow, shadow, gradient, blur, displacement,
  colour and crossfade. A malformed shader produces the real `es-validate`
  diagnostic at the authored line; an absent helper produces `compiler-process`.
  These seven synthetic inputs are not substitutes for the seven private entries.
- Five media reductions use **actual LibraryStore journal replay and Kernel
  document publication**, followed by the same selection/binding module imported
  by the native worker. Fixes include material-map references, ordered content-key
  versus bare-ID fallback, matching revisions, content objects without key
  manifests, record-level proxy provenance and complete duplicate checksums.
  Wrong content hashes and paths escaping the private snapshot are refused.
  Missing originals remain offline. Embedded payloads retain asset proxy flags.
  No app-wide Media Store integration or real-entry resolution is claimed.
- Each blob attempt now keeps a distinct private log; a later successful capture
  cannot overwrite the earlier failure's diagnostic.
- The generated 90/180/270-degree reference is an exact FFmpeg autorotate/luma
  permutation. It detects the supported display-matrix syntax. A build unable
  to generate/probe that fixture reports a clear skip in the Python test and a
  blocker in the synthetic command. All three angles ran here without skips.
- `rotation-proof` adds exact native default-path preview/final pixel comparison
  against the unrotated frame's permutation, with no interpretation override.
  **The production rotation guard was not lifted.** The qualification command
  must still fail at that guard until a Mac proves the pixels and the narrow
  guard change is qualified. Non-quarter-turn and other media guards are intact.
- Generated glow/shadow runs now compare nonblank captured preview and decoded
  MP4/ProRes pixels against an otherwise identical effect-free baseline, in
  addition to all nine stages. `--synthetic-preflight` executes these checks and
  current-production shader diagnostics with the frozen pair before source capture.
  These native paths were added but could not be built/run on Linux: the native
  example check fails at `objc2`'s explicit Apple-platform requirement.

### One-command Mac folder rerun after building the tools

Use the pinned Mac environment, build lock, free-space/pause rules and matching
worker/compiler builds described above. Then run this single driver command;
`--projects-only` prevents reading the default user's library/media stores, so
there is no temporary input-profile setup:

```sh
python3 tools/corpus/run.py \
  --binary "$CARGO_TARGET_DIR/debug/examples/pm-corpus" \
  --compiler "$CARGO_TARGET_DIR/debug/pm-glsl-compiler" \
  --projects-only --projects "/ABSOLUTE/PROJECT_FOLDER" \
  --synthetic-preflight --batch-gib 1.5 --timeout 240 \
  --retain-for-debug --report /tmp/pm-corpus-folder-summary.json
```

Both binaries are frozen before preflight or any source capture. A shader/effect
preflight failure stops the run and remains visible in the report and private
root diagnostics. Resume with the same options plus
`--resume-snapshot --resume-results`; changing library/folder-only mode is refused.
Use the same frozen tool pair for all entries; changing tool provenance cannot
certify P1. Folder-only mode cannot resolve a library-only payload; intentionally
select the read-only profile instead when that source is required.

Separately qualify rotation (expected to remain blocked at the production guard):

```sh
python3 tools/corpus/synthetic.py \
  --binary "$CARGO_TARGET_DIR/debug/examples/pm-corpus" \
  --compiler "$CARGO_TARGET_DIR/debug/pm-glsl-compiler" --rotated
```

`--rotated --expect-refusal` checks only the retained refusal. It is never a
rotation qualification pass. Native glow/shadow, rotation, Metal compilation,
export children, and the Mac app/example test and clippy suites remain required.

### Linux validation and decisions

Rust 1.96.1, Python 3.12, system FFmpeg/ffprobe. All executable test commands use
`--locked`; the standalone tooling workspace owns `tools/corpus/checks/Cargo.lock`.
No production workspace dependency/lockfile change was needed.

```sh
(cd engine && cargo build --locked -p pm-glsl --bin pm-glsl-compiler)
export POWERMOVE_GLSL_COMPILER="$CARGO_TARGET_DIR/debug/pm-glsl-compiler"
cargo test --locked --manifest-path tools/corpus/checks/Cargo.toml
cargo build --locked --manifest-path tools/corpus/checks/Cargo.toml
export PM_CORPUS_CHECKS="$CARGO_TARGET_DIR/debug/pm-corpus-checks"
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/corpus -p test_runner.py
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/corpus -p test_real_tools.py
(cd engine && cargo test --locked -p pm-glsl)
(cd apps/native && cargo test --locked -p pm-kernel --test library --test library_roundtrip)
cargo clippy --locked --manifest-path tools/corpus/checks/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path tools/corpus/checks/Cargo.toml -- --check
rustfmt --edition 2024 --check tools/corpus/main.rs tools/corpus/media.rs tools/corpus/rotation.rs
```

Results: corpus Rust **10 passed** (8 media + 2 compiler tests, including seven
shader families); Python driver **22 passed**; real frozen tools **1 passed**
(20 definitions); pm-glsl **65 passed**; kernel library/roundtrip **10 passed,
1 existing private-store manual test ignored**. The initial tagged-reader run
failed because the fresh checkout lacked v1.2.0/v1.3.0 refs; fetching those existing
reference tags fixed setup and the unchanged test passed. Native example check
is **blocked**, not passed: `objc2 only works on Apple platforms`.

Decisions:

| Decision | Selecting rule |
|---|---|
| Diagnose helper/ES/translation stages with the real compiler; keep the historical seven cases unresolved | N1/N3/N7: measured, inspectable evidence; no inferred success |
| Select journal-applied current references and ordered storage identities; verify originals and retain proxy roles | N3/N4: authored state/byte identity exact; no proxy promotion or source loss |
| Keep dataless/unavailable sources refused and every capture diagnostic visible | N4: no implicit hydration; evidence must not hide missing inputs |
| Keep the rotation guard until Mac default-path pixel proof | N3: correctness before capability claims; FFmpeg-only evidence is insufficient |
| Keep new reductions in an owned standalone tooling workspace with frozen public shader fixtures | Ownership boundary/N7/N8: headless tests, no app runtime or shared dependency upgrades |
| Replace only the corpus selection/binding consumers and retain production/UI/compat implementations | Prune rule/N4/N6: no consumer or extension contract removed |

No product design or UX changes; no `DESIGN-TODO(opus)` additions.
