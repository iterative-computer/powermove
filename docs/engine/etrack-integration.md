# E-track integration (2026-11-01)

Base: `feat/native-engine` at `ff42d751f`. Linux lane `feat/na-cloud-etrack`.
This is an integration record, not a claim that Gate E passed. Native plan v6
and docs/native/p0-decisions.md supersede older Electron-host assumptions.

## Merge 1: feat/ne-l4comp (`8bc927d3a`)

19 conflicted paths: four desktop bridge files; pm-export convert.rs,
mac/legacy_video.rs, webcodecs.rs, webm.rs; pm-node jobs encode.rs,
encode_gpu.rs, mod.rs, present/compose.rs, tests/compose.rs; pm-render
scene3d gpu.rs/mod.rs, corpus-pending.json/text-corpus-pending.json,
tests/support reference.rs/text_group_reference.rs.

- Ownership: preserve the base tree outside engine/** and docs/engine/**;
  author branches' desktop changes remain in their merge ancestry and need
  their owning lane's integration. No apps/native changes. Rule: lane ownership.
- Keep removed host encoder modules removed; keep native WebM's float input
  conversion instead of incoming Chromium RGBA8 conversion. N3/N8: no canvas
  precision loss or host JS encoder dependency.
- Keep the base's live-viewer resolution and compat dependency inspection,
  while merging the new placement/row handling. Do not restore the removed
  8-bit viewer canvas store or its artifact-specific tests. N2/N3.
- Retain both the existing Studio pass and the new scene resolve support.
  New Metal shader passes need macOS validation. N1, prune only without consumers.
- Keep base effect/transition reference semantics in effect_reference.rs;
  select that oracle for documents with effects/transitions. The graph oracle
  adds independently painted groups, precomps, mattes and sampled stills for
  other documents. Share the result type; neither reads native render pixels.
  Text reference keeps effects and adds precomp/perspective sampling. N3:
  never trade existing formula coverage for new graph coverage.
- Keep base pending fingerprints: no local capture can qualify replacement
  measurements on Linux. Remeasure on macOS; do not bless a stale digest. N3.

## Initial capability inventory

`pm-render/src/capability.rs::Feature` has 17 variants: LayerType,
ExtensionDefinition, Effects, Masks, Transition, MotionBlur, TwoAndAHalfD,
Adjustment, CompatLevel2, ProjectColor, RequiresEngine, ImageSequence, Media,
Request, Definition, ShaderFallback, Text.

They are diagnostic categories, not 17 proven missing rendering capabilities.
Producer inventory command (including execution-time diagnostics):

```sh
rg -n 'Feature::|Unsupported::request' engine/crates/pm-render/src --glob '*.rs'
rg -n 'Unsupported|problems' engine/crates/pm-export/src/render.rs engine/crates/pm-audio/src
```

The per-reason classification and gate evidence belong in e2e-ledger.md.

## Merge 2: feat/ne-l5stills

Two conflicts: pm-render/src/media.rs and tests/support/mod.rs.
Take the native-depth still reader and float interpretation, remove the
now-unconsumed Chromium RGBA8 helper; retain both sampling and still reference
modules. Adapt the L4 bitmap oracle to L5's independent standards conversion.
N2/N3: source precision, transfer, ICC and orientation follow the file, with no
Blink upload emulation. Prune rule: the replaced helper has no consumers.
No out-of-lane files were imported.

## Merge 3: feat/ne-phase4a (`ef5cbce3a`)

44 conflicts, listed below. The branch was built for an older Electron
renderer and contains obsolete Chromium SVG, gamma, video and RGBA8 viewer
paths. Preserve current engine consumer contracts and import the independently
consumable pm-proxy and pm-cache backends. The old whole-app wiring is retained
in merge ancestry, not installed over the native service (N3/N6/N8). This merge
does **not** complete Phase 4a viewer/job integration.

- Reintroduce pm-proxy/pm-cache as implemented crates: the base removed them
  only when they were empty. The prune rule no longer justifies that deletion.
- pm-proxy generation remains the default feature for Mac consumers. With
  `--no-default-features`, its policy, content-addressed store, disk guard and
  sequence identity compile without Apple media. Sequence generation adapts to
  L5 native PNG gamma metadata instead of removed ImageIO emulation (N3).
- Preserve base protocol, renderer, media, presenter and job wiring. A native
  service must bind the backend through its own worker/job interfaces. Until
  those owned native paths are changed, Gate E marks that integration blocked.
- Do not import the Electron UI/copy changes (ownership; Opus owns design).
- Author measurements in proxy.md, representations.md and phase4a-gate.md are
  historical branch evidence, not measurements of this merged native app.

Conflicted paths:

- `apps/desktop/e2e/ae-audit-implementation.spec.ts`
- `apps/desktop/e2e/automatic-preview-memory.spec.ts`
- `apps/desktop/scripts/engine-writers.json`
- `apps/desktop/src/extensions/.builtin-versions.json`
- `apps/desktop/src/extensions/viewer/engine-present/presenter.ts`
- `apps/desktop/src/extensions/viewer/engine-present/view-params.ts`
- `apps/desktop/src/extensions/viewer/manifest.json`
- `apps/desktop/src/extensions/viewer/viewer.ts`
- `apps/desktop/src/main/engine/index.ts`
- `apps/desktop/src/main/engine/native-engine.test.ts`
- `apps/desktop/src/main/engine/native-engine.ts`
- `apps/desktop/src/main/media-proxy.ts`
- `apps/desktop/src/main/render-encoder.ts`
- `apps/desktop/src/preload/index.ts`
- `apps/desktop/src/renderer/src/engine-connection/install.ts`
- `apps/desktop/src/renderer/src/engine-connection/session.ts`
- `apps/desktop/src/renderer/src/legacy/core/engine.ts`
- `apps/desktop/src/shared/engine-ipc.ts`
- `apps/desktop/src/shared/engine-protocol/Event.ts`
- `apps/desktop/src/shared/engine-protocol/index.ts`
- `engine/Cargo.lock`
- `engine/crates/pm-cache/src/lib.rs`
- `engine/crates/pm-media/Cargo.toml`
- `engine/crates/pm-media/src/decode/scrub.rs`
- `engine/crates/pm-media/src/image/mod.rs`
- `engine/crates/pm-node-addon/Cargo.toml`
- `engine/crates/pm-node/Cargo.toml`
- `engine/crates/pm-node/src/jobs/mod.rs`
- `engine/crates/pm-node/src/present/compose.rs`
- `engine/crates/pm-node/src/present/mod.rs`
- `engine/crates/pm-node/src/present/render.rs`
- `engine/crates/pm-node/src/present/schedule.rs`
- `engine/crates/pm-node/tests/compose.rs`
- `engine/crates/pm-protocol/src/envelope.rs`
- `engine/crates/pm-protocol/src/samples.rs`
- `engine/crates/pm-proxy/Cargo.toml`
- `engine/crates/pm-proxy/src/lib.rs`
- `engine/crates/pm-render/Cargo.toml`
- `engine/crates/pm-render/src/assets.rs`
- `engine/crates/pm-render/src/exec.rs`
- `engine/crates/pm-render/src/media.rs`
- `engine/crates/pm-render/src/plan.rs`
- `engine/crates/pm-render/src/renderer.rs`
- `engine/crates/pm-render/src/shaders/source.wgsl`

## Merge 4: feat/ne-bigrender (`94d982e28`)

Conflicts listed below. Preserve native coverage rasterization, native shader
ownership, recorder/compat and source-monitor protocols, plus current GPU
resource-release accounting (N3/N6/N8). Do not resurrect deleted browser SVG,
Canvas2D or runtime WebGL parity modules. These older optimizations target
paths already replaced on the base; ancestry archives their implementation.

Integrate immutable-array prepared keyframe caching alongside the channel memo,
without cloning the prepared track on each read (N2/N3); constant-table hoisting
into program constants (unchanged shader meaning); definition program builds
on the native worker, with source-specific claims published before sending and
retired on device release (N1/N8); and excluding unused audio from silent-export
offline dependencies (N3). GPU timing, prewarming and the full large-project
performance gate require the Mac run; no Linux performance claim is made.

- `apps/desktop/e2e/helpers/app.ts`
- `apps/desktop/src/extensions/viewer/engine-present/present-worker.ts`
- `apps/desktop/src/extensions/viewer/engine-present/presenter.ts`
- `apps/desktop/src/extensions/viewer/engine-present/worker-presenter.ts`
- `apps/desktop/src/extensions/viewer/manifest.json`
- `apps/desktop/src/main/engine/compat-frames.test.ts`
- `apps/desktop/src/main/engine/engine-process.ts`
- `apps/desktop/src/main/engine/index.ts`
- `apps/desktop/src/main/engine/native-engine.ts`
- `apps/desktop/src/main/engine/offline-media.test.ts`
- `apps/desktop/src/main/engine/offline-media.ts`
- `apps/desktop/src/main/engine/snapshots.ts`
- `apps/desktop/src/preload/index.ts`
- `apps/desktop/src/renderer/src/engine-connection/connection.ts`
- `apps/desktop/src/renderer/src/engine-connection/install.ts`
- `apps/desktop/src/renderer/src/engine-connection/render-transport.test.ts`
- `apps/desktop/src/renderer/src/engine-connection/render-transport.ts`
- `apps/desktop/src/renderer/src/engine-connection/transport.ts`
- `apps/desktop/src/renderer/src/engine-doc/install.ts`
- `apps/desktop/src/renderer/src/kernel/install.ts`
- `apps/desktop/src/renderer/src/kernel/raster-compat.ts`
- `apps/desktop/src/renderer/src/legacy/app.test.ts`
- `apps/desktop/src/renderer/src/legacy/app.ts`
- `apps/desktop/src/renderer/src/legacy/core/engine.ts`
- `apps/desktop/src/renderer/src/legacy/host/electron-shim.test.ts`
- `apps/desktop/src/shared/engine-protocol/CompatFrameRequest.ts`
- `apps/desktop/src/shared/engine-protocol/CompatMessage.ts`
- `apps/desktop/src/shared/engine-protocol/index.ts`
- `docs/engine/effects.md`
- `docs/engine/perfbench.md`
- `docs/engine/vector.md`
- `engine/Cargo.lock`
- `engine/crates/pm-eval/src/evaluator.rs`
- `engine/crates/pm-glsl/src/lib.rs`
- `engine/crates/pm-gpu/src/target.rs`
- `engine/crates/pm-node/src/jobs/blender_png.rs`
- `engine/crates/pm-node/src/jobs/compat.rs`
- `engine/crates/pm-node/src/jobs/mod.rs`
- `engine/crates/pm-node/src/jobs/svg.rs`
- `engine/crates/pm-node/src/present/compat.rs`
- `engine/crates/pm-node/src/present/compose.rs`
- `engine/crates/pm-node/src/present/document.rs`
- `engine/crates/pm-node/src/present/render.rs`
- `engine/crates/pm-node/src/present/schedule.rs`
- `engine/crates/pm-node/tests/render_fonts.rs`
- `engine/crates/pm-node/tests/snapshot.rs`
- `engine/crates/pm-perfbench/baselines.json`
- `engine/crates/pm-protocol/src/samples.rs`
- `engine/crates/pm-protocol/src/snapshot.rs`
- `engine/crates/pm-protocol/src/svg_raster.rs`
- `engine/crates/pm-render/src/compat.rs`
- `engine/crates/pm-render/src/composite/exec.rs`
- `engine/crates/pm-render/src/effects/exec.rs`
- `engine/crates/pm-render/src/effects/parity.rs`
- `engine/crates/pm-render/src/lib.rs`
- `engine/crates/pm-render/src/plan.rs`
- `engine/crates/pm-render/src/renderer.rs`
- `engine/crates/pm-render/src/renderer/layer_raster.rs`
- `engine/crates/pm-render/src/shader_layers/exec.rs`
- `engine/crates/pm-render/src/vector/cache.rs`
- `engine/crates/pm-render/src/vector/mod.rs`
- `engine/crates/pm-render/tests/effects_fallback.rs`
- `engine/crates/pm-vector/Cargo.toml`
- `engine/crates/pm-vector/src/canvas2d/analytic.rs`
- `engine/crates/pm-vector/src/canvas2d/context.rs`
- `engine/crates/pm-vector/src/canvas2d/msaa.rs`
- `engine/crates/pm-vector/src/canvas2d/shape.rs`

## Merge 5: feat/ne-svg (`b1d10d3ec`)

22 conflicts, listed below. The author branch implements Chromium bucket
replay and adds SvgImage routing for text, filters and unsupported replay.
The base already supersedes this with output-resolution native SVG at every
compat level, including text, gradients, filters and depth-safe parsing.
Preserve that implementation and its tests (N2/N3/N8). Do not reintroduce the
deleted Canvas2D subsystem, SVG unsupported reason or GPU-specific Skia
rounding/gradient/blur emulation. Inputs remain archived in merge ancestry;
no legacy consumer on the base refers to the deleted replay modules.

Retain the branch as an explicit no-ff merge with its disposition recorded.
Current native SVG regressions, rather than the replay admission test, define
the Mac and portable verification rows in the Gate E ledger.

- `docs/engine/vector.md`
- `engine/crates/pm-render/src/capability.rs`
- `engine/crates/pm-render/src/plan.rs`
- `engine/crates/pm-render/src/vector/svg.rs`
- `engine/crates/pm-render/tests/vector.rs`
- `engine/crates/pm-skia-path/src/blinksvg.rs`
- `engine/crates/pm-skia-path/src/blur.rs`
- `engine/crates/pm-skia-path/src/dash.rs`
- `engine/crates/pm-skia-path/src/gradient.rs`
- `engine/crates/pm-skia-path/src/m44.rs`
- `engine/crates/pm-skia-path/src/skrrect.rs`
- `engine/crates/pm-skia-path/src/svgrender.rs`
- `engine/crates/pm-vector/Cargo.toml`
- `engine/crates/pm-vector/examples/svg_compat_compare.rs`
- `engine/crates/pm-vector/src/canvas2d/analytic.rs`
- `engine/crates/pm-vector/src/canvas2d/mod.rs`
- `engine/crates/pm-vector/src/canvas2d/skpath.rs`
- `engine/crates/pm-vector/src/canvas2d/svg.rs`
- `engine/crates/pm-vector/src/render/svg.rs`
- `engine/crates/pm-vector/tests/svg.rs`
- `engine/crates/pm-vector/tools/canvas-ref/page.ts`
- `engine/crates/pm-vector/tools/canvas-ref/run.ts`

## Linux qualification of the final implementation

Rust 1.96.1; tools installed under /workspace/.tools because the attached
container did not include the activation script, Rust or gitleaks. The
activation script is outside Git. No Apple SDK, Metal, CoreAudio, CoreText,
VideoToolbox, WebKit, screen or frozen external captures are available here.

| Validation | Result |
|---|---|
| pm-time/model/expr/geom3d/reference/protocol/stretch/color, library + integration + doctests | **497 passed, 4 measurement tests ignored**, with only the known desktop TypeScript drift test explicitly filtered (see below) |
| pm-eval library and ten portable integration targets | **77 passed**; exact TS/V8 evaluation, captions, helper, layer, retiming, invalidation, identity and shadow vectors |
| pm-glsl default test concurrency, binaries, integration and doctests | **71 passed**, including constant tables, uniform acceptance and production source WGSL validation |
| pm-proxy + pm-cache, `--no-default-features` | **12 passed**: policy, power probe, store integrity/cancellation, sequence identity, disk guard and cache budgets |
| Actual production device_policy source compiled standalone, `rustc --test -D warnings` | **5 passed** with a Linux probe double |
| Gate E inventory audit unit tests | **5 passed**; inventory accounts for 17 variants and 74 production references |
| Total passing tests above, without double-counting the earlier 395-test library run | **667 passed; 4 ignored; one separately recorded cross-lane failing test** |
| Clippy `-D warnings` | CPU libraries pm-color/eval/expr/geom3d/reference/protocol/glsl; pm-glsl all targets; pm-proxy/cache portable all targets pass |
| Apple-dependent crate check | **Blocked:** objc2 explicitly rejects the Linux target. pm-media/render/export/audio/node and dependent Mac crates cannot be typechecked/tested/clippied here. No claim of Mac build success. |
| Formatting | rustfmt check passes for all 136 changed Rust files; unrelated base formatting is preserved |
| Secret scanning | Staged diff inspected and `gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow` before every commit; hook also enabled |

Representative commands (each shell first sources /workspace/.tools/activate.sh):

```sh
cd /workspace/powermove/engine
cargo test --locked -p pm-time -p pm-model -p pm-expr -p pm-geom3d -p pm-reference -p pm-protocol -p pm-stretch -p pm-color -- --skip typescript_regeneration_is_a_no_op
cargo test --locked -p pm-eval --lib --test index_identity --test expression_helpers --test layers_unit --test js_sem_v8 --test hooks_and_expressions --test shadow --test invalidation --test ts_parity --test layer_parity --test caption_parity
cargo test --locked -p pm-glsl
cargo test --locked --no-default-features -p pm-proxy -p pm-cache
cargo clippy --locked --lib -p pm-color -p pm-eval -p pm-expr -p pm-geom3d -p pm-reference -p pm-protocol -p pm-glsl -- -D warnings
cargo clippy --locked --all-targets -p pm-glsl -- -D warnings
cargo clippy --locked --all-targets --no-default-features -p pm-proxy -p pm-cache -- -D warnings
```

The unfiltered broad test run exposed exactly one remaining protocol failure:
`typescript_regeneration_is_a_no_op` needs the generated desktop index's build
id updated. Ownership forbids editing apps/desktop here. The exact patch is
`etrack-desktop-protocol.patch`; the owning lane should apply or regenerate it.
Do not remove the drift test or relax draft-protocol identity. pm-eval's full
`corpus` target requires CoreText and does not compile on Linux; portable targets
above were selected explicitly rather than pretending to run that capture gate.

The first compiler parallel run exposed a test-only scheduling issue: the
one-millisecond deadline measurement began before waiting for another unit
test's global compiler lock. Its timer now starts where the real request
budget starts; the 1 ms deadline and 2 s cleanup bound are unchanged. The
output-only constant-table tests serialize their own compiler requests; real
admission/overload tests remain intact. Final default-concurrency run passes.
Pipeline revision 4 gives the integrated optimization fresh cache identity;
the measured prepared-output digest is unchanged and pinned at that revision.

N2/N3 selected prepared-track reuse with reset/overflow regressions and the
extreme finite inverse-root fix. N1/N3 selected exact log metadata and native
float still handling, plus explicit device transitions. N6/N8 selected retaining
current native protocols and rejecting resurrected Chromium replay paths.
No visual/UX decisions or DESIGN-TODO placeholders were introduced.

Gate E remains open: see e2e-ledger.md and gate-e.json for implementation,
consumer-binding, platform and qualification blockers. A complete inventory
is not a completed rendering gate.

## macOS integration replay — base Apple M5 (2026-11-01)

The integration branch includes the engine-finish, native pro-media and
export-parity merges. The first build of the combined tree ran no engine tests:
Studio still used retired material slots/mesh groups and omitted the output
color-space label. Those compile failures are repaired against the current
whole-mesh material API; Studio's already tone-mapped output is explicitly
encoded. Regression tests retain the current material and output contracts.

Additional repairs found by the Mac replay:

- Move the proxy chart's independent f64 color reference into test support;
  it no longer imports the deleted S2 spike. Keep the Sharma CIEDE2000 vectors.
- Honor PNG cICP precedence over ICC when ImageIO/sips produces both. Preserve
  supported gamma/linear/log transfer readings and reject unknown explicit tags.
- Preserve low-bit-depth samples and original 16-bit unprofiled TIFF palette
  entries through ImageIO. Independent authored TIFF and packed-index tests
  verify the original codes rather than a converter's rescaled RGB8 palette.
- Allocate a real mip chain for stills and prepare native material maps for
  the standalone 3D entry point. Keep video normalization and zero-copy checks;
  still AVIF tests now compare their retained native YCbCr planes as well.
- Use normalized Lanczos-3 reconstruction for AVIF still chroma. The complete
  still standards sweep passes its original bounds, including tagged HDR
  samples; video keeps its separately qualified playback filter.
- Make the typography fixture explicitly Helvetica, the face whose metrics
  produced its frozen numbers. Keep those numbers. Make the still gate's
  project depth explicitly 32-bit so its requested float comparison is real.
- Isolate independent fake audio devices and native compiler fixtures from
  process-wide pools. Preserve deadline, PCM, shader and admission assertions.
- Restore the export conversion helper with an HDR-before-mutation regression;
  repair the three export-parity Mac failures described in [export.md](export.md).
- Update the generated desktop protocol hash and native lockfile dependency
  edges. The VFR probe is correct on this Mac: ffprobe reports an explicit
  final duration. Tests separately verify reported timing and refusal of absent,
  unknown or zero final duration without inventing timing.

The original frozen corpus and real project are unavailable. New synthetic v1
witnesses are captured on this build Mac (base M5), one category at a time,
using a temporary Electron profile. They are **not the original frozen
capture**. Strict fingerprints and numerical limits are unchanged. Recorded
category results and the remaining Gate E qualifications are in
[e2e-ledger.md](e2e-ledger.md) and [gate-e.json](gate-e.json).

### WIP owner handoff

Validation was stopped at the orchestrator's request before the full engine
integration replay finished. Jude takes over with the original frozen corpus
and projects. The exact failure register, verified subsets, environment and
next steps are in [the Mac engine handoff](../native/handoff/engine-macos.md).
The branch is not all green; Gate E and the cold distinct-voice AUDIO-2 gate
remain open. The last TIFF palette integration repair also needs a full replay.
