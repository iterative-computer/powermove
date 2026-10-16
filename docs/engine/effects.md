# Effects, transitions, and the GLSL pipeline

Plan section 11 (all of it), section 8 (the watchdog; the effect pass and transition
nodes), section 25 (the shader gate, the WebGL-unstable class), amendment 9, and
contract 10. Code:

- `engine/crates/pm-glsl`: the production form of spike S3's naga path
  (`docs/engine/spikes/S3.md`): limits, ES validation, normalize, glslang, the SPIR-V
  rewrites and bind check, naga; the translation cache; isolation under a deadline; the
  fuzz target `pm-glsl-fuzz`.
- `engine/crates/pm-render/src/effects/`: definitions (contract 10) and their programs,
  the effect pass and transition nodes, the staged-layer executor, GL's uniform
  binding rules, the GPU watchdog; `builtins.json` embeds the built-in snapshots.
- `apps/desktop/src/renderer/src/kernel/definition-snapshots.ts`,
  `engine-definitions.ts`: the kernel side (snapshots, the publisher, the session
  streams, shader meta from engine reports), installed only with "Use New Engine" on.
- `engine/crates/pm-render/tools/effects/`: the shader corpus and its WebGL2
  references; `engine/corpus/src/projects/render-effects.ts`: the app-level corpus.

The plan stays the authority.

## The pipeline

`pm_glsl::prepare(fragment, line_offset)` runs steps 0 to 4 and returns `Prepared`
(SPIR-V words and reflection, serializable); `Prepared::finish` runs step 5 (naga parse
and validation, about 0.03 ms). `translate` does both.

| Step | What | Where |
|---|---|---|
| 0 | Source limits: size, a budgeted GLSL ES preprocessor (macro expansion, statements), WebGL's identifier and struct-nesting rules, `#extension` placement | `limits::check_source` |
| 1 | WebGL's compile and link: glslang's ES 3.00 front end on the unmodified program with `PM.VERT` | `compile::validate_es` |
| 2 | `#version 450`, precision removed, GLSL 4.50 and Vulkan keywords renamed, polyfills (`inverse`, `outerProduct`, `modf`, scalar `normalize`/`faceforward`/`reflect`/`refract`), line counts unchanged | `normalize` |
| 3 | glslang, relaxed Vulkan rules, automatic bindings and locations | `compile::compile` |
| 4 | Sampler arrays, distinct bindings, varyings by name, the fragment interface (narrow, array, and depth outputs; unused inputs), signed `%` as `OpSRem`, `uint` negation, `texelFetchOffset`, 2-row matrix strides, non-finite constants, the combined-sampler split; SPIR-V limits (call depth, recursion, private variables, uniform vectors); the bind check | `rewrite`, `limits::check_spirv`, `reflect::unbound_resources` |
| 5 | naga SPIR-V front end with `adjust_coordinate_space` (GL row order), derivative uniformity off, validation | `pipeline::Prepared::finish` |
| 6 | Cache by SHA-256 of (pipeline revision, glslang and naga versions, the device context, line offset, source): memory (512 entries) and disk (atomic JSON entries; a damaged or foreign entry is a miss); failures in memory only | `cache::Cache` |
| 7 | Diagnostics in the author's lines (`WrappedGlsl::body_line_offset`); preamble lines are reported as line 0, "generated code" | `errors::map` |

Pipeline creation (the Metal compiler) happens in pm-render on first use per target
format; Metal's own cache makes later builds cheap.

**Isolation.** Translation runs on its own thread with a 1 GiB stack (a 100,000-deep
swizzle chain overflows 8 MiB) and a 20 s deadline (`cache::prepare_isolated`). A
program past the deadline fails with stage `deadline`; its thread is left to finish
(glslang and naga cannot be interrupted). Translation in a helper process (S3
recommendation 4) is not built; see Gaps.

**A bug the spike carried.** `glslang_shader_create` keeps a pointer to the input
struct's `code` field (`setStrings(&input->code, 1)`). The spike returned the input by
value, so the pointer dangled; release builds happened to elide the move. The input is
now boxed and outlives the shader (`compile.rs`). Every glslang call crashed in a debug
build before the fix.

**Verdicts.** A failure in `limits` or `es-validate` is a program WebGL refuses too (or,
for the macro and statement budgets, one that stalls WebGL for seconds): today it
renders nothing and `runEffects` skips it, so the engine skips it the same way
(`Status::Refused`, reported as `ShaderCompile`). A failure in any later stage is
engine-only: WebGL draws the program, so layers using it are routed
(`Feature::ShaderFallback`, reported as `NotHeadlessRenderable`) until the decision 19
fallback runner exists. Nothing is drawn wrong.

## Staged layers

`renderProject` takes a layer off the fast path when it has a renderable effect
(`hasRenderableEffects`) or an active transition: the layer is drawn into its own
buffer (`lf`, the size of the level), masks (not here), effects, the track matte (not
here), then composited, through `runTransition` when one is active. The engine plans the
same thing as a `Content::Staged` draw (`effects/plan.rs`):

- **Content**: the layer's own draws as a transparent `CompPlan` of the level's size
  (a group: its members' plan); the stage's `alpha` is the group's opacity
  (`compositeAlpha`), 1 for other layers.
- **Effect pass nodes** (`EffectStep`), in `L.fx` order: enabled (`<fx>.$enabled`,
  animatable), not `missing`, definition registered. Parameters as
  `effectParamValue` reads them (no property or a nullish value: the default) and as
  `setParam` converts them: colors by `hex2rgb(String(v))` (first `#` removed, 3 or 4
  digits doubled, `parseInt(…, 16) || 0` per channel, a sign accepted), toggles 0 or 1,
  numbers `Number(v) || 0`.
- **Transition node** (`TransitionStep`): `activeTransition`'s edge and progress, with
  `prop ? evP : pd.def` parameters.

Execution (`effects/exec.rs`) is in GL row order like every pm-render target:

- A level holding staged draws is drawn in runs: the draws before a staged layer in one
  render pass (the first clears and draws the background; later runs load), so the
  stack a staged layer reads as `u_backdrop` is exactly today's `acc`.
- Each effect pass: a new buffer cleared to transparent, blending off, the quad through
  `PM.VERT`, the compositor's calls in order: `u_tex` = 0, `u_orig` = 1 (with
  `keepOrig`), `u_backdrop` = 2 (with `backdrop`), `u_m`, `u_res`, `u_uv`,
  `u_coordRes`, `u_pxOrigin`, `u_texel`, `u_time`, `u_pass`, then the parameters. GL's
  rules (`effects/program.rs`): a call of the wrong type changes nothing (a single float
  never sets a `vec2`; `uniform1f` never sets an `int`), uniforms are program-wide
  (both stages' blocks), named parameters bind when the linked program has them active,
  else the positional alias `u_p<i>` or `u_c<i>`; a sampler reads unit 0 unless a
  `uniform1i` call moved it; an empty unit samples (0, 0, 0, 1). Every texture is
  sampled linear with clamp-to-edge, today's state for every effect and transition
  buffer (`grab`).
- `clamp: true` (a definition flag the kernel passes through) clamps the effect's output
  to 0..1 in an extra pass; otherwise values above 1.0 pass (rgba16float or rgba32float).
- The composite is `drawFbo(res, W, H, alpha)`: premultiplied source-over, 1:1. With a
  transition: `withLayer` = an exact copy of the stack with the layer composited,
  `from`/`to` by edge, the output copied back; a transition program that cannot run
  leaves `withLayer` (as `runTransition` returning null).

**Regions of interest.** A level whose staged layers are all viewport-safe and have no
transition draws a padded crop of the region (today's 128 px viewer overscan plus the
sum of the chain's `viewportPadding` values, clamped to the frame) with the crop's
effect coordinates (`effectViewportCoordinates`: `u_uv`, `u_pxOrigin`, `u_coordRes`),
then copies the region out. Any other staged level is drawn in full and cropped. Tests:
a blur, chroma, and vignette chain's padded crop equals the full frame (0 CV); levels with
transitions or non-viewport-safe effects equal it exactly. Note: Mosaic computes its grid
from `v_px - u_pxOrigin` (crop-local), so its grid follows the crop in today's zoomed
viewer and here alike.

**Definitions in the plan.** `Renderer::set_definitions` (an embedded
`DefinitionSetSnapshot`) or `apply_definitions` (a kernel `DefinitionsUpdate`) configure
the renderer. Without definitions, effects and transitions are routed as before
(`Feature::Effects`, `Feature::Transition`). With them:

| Case | Today | Engine |
|---|---|---|
| Effect registered, program ready | drawn | drawn |
| Effect id the document marks `missing` | skipped (fast path) | skipped (fast path) |
| Effect id marked missing in the set (removal, omitted from a full update, `missing` of an embedded set) | `PM.FX[type]` undefined: skipped, layer buffer path | same |
| Effect id unknown (no snapshot, not missing) | n/a | routed (`Feature::Definition`); contract 10: `pm-cli` fails only here |
| Program WebGL refuses | skipped | skipped, `ShaderCompile` reported |
| Program WebGL draws, engine cannot translate | drawn | routed (`Feature::ShaderFallback`), `NotHeadlessRenderable` reported |
| Watchdog overrun | n/a | skipped from the next frame, `WatchdogTimeout` reported |
| Transition registered | drawn | drawn |
| Transition unknown / missing / program refused | — / no transition / `withLayer` | routed / no transition / no transition |
| Effects on adjustment layers | `compositeAdjustment` | routed (`Feature::Adjustment`; the composite lane ports `FRAG_COMPOSITE`) |

`capability::unsupported_with_definitions` gives the same answer statically for routing
consumers; `unsupported` and `unsupported_with_media` are unchanged (they list effects
and transitions). Built-ins: `pm_render::builtin_definitions()` (the 16 effects and 4
transitions as the kernel snapshots them; a vitest fails when `builtins.json` drifts
from the kernel; `bun engine/crates/pm-render/tools/effects/builtins.ts` rewrites it).

## The watchdog

Each effect instance and transition is timed between two marker dispatches: one-workgroup
compute passes whose end-of-pass sample is the timestamp, the first reading a texel of
the effect's input, the second a texel of its output (as `pm-export`'s `GpuSpan`). One
query set and readback per renderer; a frame encoded while the previous timed frame is
unread is not timed. Readback is never waited for during rendering: the next frame
collects it (`Watchdog::collect`). A definition over the budget (default 250 ms per
instance per frame) is disabled from the next frame, skipped with its data kept, and
reported (`WatchdogTimeout`, effect scope) until it is registered again.

Measured: with eleven test renderers busy in one process, Metal left pass-boundary
samples of render passes unwritten (zeros) in 4 of 10 spans, and end-of-pass samples of
the marker dispatches stale; such spans are not judged (a sample at or before the newest
valid one, or an end not after its begin). The effect tests take turns on the GPU; the
engines have one device per process.

Built-in GPU times at UHD (3840 × 2160, defaults, a 3000 × 1700 image, release, M4 Max,
the longest of 5 frames): Gaussian Blur 13.4 ms, Glow 12.2, Drop Shadow 11.2, Turbulent
Displace 3.9, Directional Blur 2.0, every other built-in 0.5 to 0.8 ms. Translating the
20 built-ins in a cold process: 154 ms; from the cache: 0.27 ms. First frame per effect,
pipeline build included: 92 to 142 ms.

Not built: device-loss recovery (a true hang). The device's owner (pm-node, pm-export)
rebuilds the device; see Gaps.

## Definition snapshots (contract 10)

Kernel (`kernel/definition-snapshots.ts`):

- `effectSnapshot`, `transitionSnapshot`, `layerSnapshot`: the kernel's own wrapping
  (`toLegacyFx`, `toLegacyTransition`, `extensionLayerFragment`) with the installed
  preambles; `bodyLineOffset` is the line the body starts on; UTF-16 and UTF-8 body
  lengths. UI fields are dropped. Parameters: colors as `#rrggbb` with the values
  `hex2rgb` reads; any type other than color and toggle is a number (as `setParam`
  binds it); number ranges are made finite and ordered so one definition's slider bounds
  can never invalidate an update; `viewportPadding` keeps numeric keys only. `clamp` is
  read from the registration when present (`kernel/api.ts` does not declare it yet).
- Origin from the extension record: built-in; fork (`forkedFrom` or `replaces`, so a
  fork's GLSL is used under the built-in id); Store (sandboxed); local otherwise.
- `DefinitionPublisher` follows the effect, transition, and layer registries: the
  winner per id (last registered wins, disposing restores the previous one), one change
  per task (registry batches make a reload one change).
- `SessionDefinitions`: one receiver's stream, a full update at generation 1, then
  deltas numbered by one; a refused update makes the next one full.
- `engine-definitions.ts` installs it with "Use New Engine" on: main's relay gets the
  stream (`nativeEngine.pushDefinitions`), `PM.EngineDefinitions.session(send)` gives an
  engine session its own stream, and `GL.compileError` answers for engine reports too.

Engine (`pm_render::effects::Effects`): applies updates in generation order
(`pm_protocol::definitions::DefinitionSet`), translates what changed (a same-source
re-push reuses the program; a changed one gets a fresh watchdog record), and returns the
per-definition failures (`DefinitionsApplied.failures`).

**Shader meta.** `EngineShaderMeta` fills `uiState.getShaderMeta` and the compile errors
from engine reports at their revision: definition failures go to every extension layer
of that definition (`shaderKey: extension:<id>`) and to `GL.compileError('fx:<id>')`,
`tr:`, `extension:`; layer reports set `shaderKey: engine:<layer>` (compile errors),
`missing` (missing definitions), `missingAsset` (offline media), or `sceneError`; a
report older than one applied for the layer is ignored; a re-pushed definition clears
its errors.

## Parity

**Shader level** (`tools/effects/collect.ts`, `webgl.ts`; `tests/effects_webgl.rs`).
The corpus: the 16 built-in effects at defaults, 70% of range, minimum, maximum, two
seeded random sets, coerced values (numbers as strings, toggles as numbers, short and
odd colors), two crops of a 2× composition, and E-encoded inputs above 1.0 (values up to
3.0 in the layer and the backdrop); the 4 transitions at four progress values with three
parameter sets and above-1.0 inputs; spike S3's 43 generated agent-style effects and
transitions (every section 11 contract row; up to 8 passes); the 394 coverage probes.
WebGL2 on ANGLE Metal (Chromium 151.0.7922.34 headless via Playwright) replays the compositor's
exact calls; the engine runs the production path (`effect_step`/`transition_step`,
`Renderer::run_effects`/`run_transition`). Judged by section 25's Standard class on 8-bit
values plus the same rule on float values scaled by max(1, |reference|).

Measured 2026-10-16 (M4 Max, macOS 27, wgpu 30.0.1 Metal):

| Group | Variants | Match WebGL | Both refuse | Known open |
|---|---|---|---|---|
| Built-in effects | 176 | 176 | 0 | 0 |
| Built-in transitions | 52 | 52 | 0 | 0 |
| Generated (S3) | 219 | 211 | 8 (2 `linkError` cases) | 0 |
| Coverage probes | 394 | 260 of 270 drawn | 122 | 12 |

Known open (each also in S3.md): 8 probes whose only sampler is not a float `sampler2D`
(WebGL samples (0, 0, 0, 1); the bind check refuses: fallback runner), ternary on a
struct or an array (WebGL refuses; the engine accepts: needs expression types), `out
vec3` (GL leaves alpha undefined; 320 values at 2 CV), integer division by zero
(undefined). `output-ivec4` links in WebGL and its draw fails; the engine refuses it at
pipeline creation.

**App level** (`engine/corpus/src/projects/render-effects.ts`, captured through the app;
`tests/corpus.rs` with the built-in definitions): every built-in effect at two parameter
sets on a photo, an alpha image at 85% opacity, and a rotated solid; every transition in
and out on a photo and a solid; a stack with Glow, an animated enable, Drop Shadow, a
disabled effect, and a missing one; effects on a group, inside a precomp, on a precomp
layer, and a transition on a group. Capture `effects-20261016-041938-4837194b` (79
samples, each opaque and as a transparent float frame). With the newest v1 and render
captures, 149 samples are compared: 146 within the Standard class (their transparent
float frames within 1 CV), the 3 Posterize samples (opaque and float) in the
WebGL-unstable class (below), 0 failures.

**The WebGL-unstable class** (amendment 9). `webgl.ts --perturb-input` moves every
input texel one binary16 step away from zero; WebGL's own output then moves past the
tolerance for Posterize (7 of 11 variants, up to 43 CV), Levels (6 of 11, up to 138 CV),
and Color (1 of 11, 2 CV): step functions at some parameters (`floor(x * n + .5)`,
`clamp` then a steep `pow`). The engine's layer buffer equals today's within 1 CV but not
bit for bit (pm-render's drawing class), so `render/fx-posterize` differs at 0.02% to 0.1%
of pixels by exactly one posterize level (16, 27, and 85 CV at 16, 9.5, and 3 levels).
Those 3 samples pass the 99.9% rule and fail the maximum (and their float frames the
1 CV rule); `tests/corpus.rs` lists them
apart for review (`pm_render::effects::WEBGL_UNSTABLE`). Not decided here: the plan's
gate wording needs the class reviewed by Jude.

**Store GLSL.** Fuzz (`pm-glsl-fuzz`, worker process, per-input deadline), 2026-10-16:

| Run | Inputs | Outcomes | Crashes, hangs, panics | Slowest |
|---|---|---|---|---|
| seed 1, mutants of the 63 non-probe corpus programs | 3,000 | 189 ok, 2,810 `es-validate`, 1 `limits` | 0 | 66 ms |
| seed 3, mutants of the 394 probes | 3,000 | 103 ok, 11 `bind`, 2,850 `es-validate`, 36 `limits` | 0 | 68 ms |
| resource bombs | 17 | 5 ok, 6 `limits`, 5 `es-validate`, 1 `rewrite` | 0 | 538 ms |

The `rewrite` bomb is a 100,000-element constant array: glslang emits an instruction
longer than SPIR-V's 65,535-word limit and the SPIR-V loader refuses it; WebGL refuses
the program for its 64 KiB private-variable rule, the engine routes it. Compile parity
of the mutants with WebGL (S3's `webgl-compile.ts`) was not rerun here. `cargo test -p
pm-glsl` runs 300 mutants in process and the probes' verdicts on every build
(`tests/probes.rs`; the fixture is rewritten by `tools/effects/expectations.ts`).

## Commands

```sh
bun engine/crates/pm-render/tools/effects/builtins.ts            # rewrite builtins.json
bun engine/crates/pm-render/tools/effects/collect.ts             # shader corpus → engine corpus folder
~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  bun engine/crates/pm-render/tools/effects/webgl.ts [--perturb-input]
bun engine/crates/pm-render/tools/effects/expectations.ts        # probe verdict fixture
cargo test -p pm-render --test effects_webgl -- --nocapture      # shader parity
bun engine/corpus/src/capture.ts --only render/fx-,render/tr- --label effects --skip-real --skip-scenarios
cargo test -p pm-render --release --test corpus -- --nocapture   # app-level parity
cargo run --release -p pm-glsl --bin pm-glsl-fuzz -- --seeds <seeds.json> --iterations 3000
cargo test -p pm-render --release --test effects builtin_costs -- --ignored --nocapture
```

## Gaps

- **Fallback runner (decision 19)** not built: engine-only failures route the layer to
  the legacy engine (`Feature::ShaderFallback`) instead of rendering through a minimal
  WebGL runner; nothing is marked not headless-renderable beyond the error code.
- **Translation in a helper process** not built: a crash inside glslang or naga takes
  the engine process (the fuzz runs found none; limits stop every bomb found); a hang
  costs one leaked thread past the deadline.
- **Device-loss recovery** for a true GPU hang is the device owner's (pm-node, pm-export)
  and not built here; the watchdog covers slow-but-finishing effects.
- **Export jobs and pm-cli** do not pass definitions yet, so effects and transitions
  stay routed there as before. The pieces are ready: `Renderer::set_definitions`,
  `capability::unsupported_with_definitions`. What is left: `pm_engine::jobs::JobDocument`
  carries the session's `DefinitionSetSnapshot` (`job_document`), pm-node's job runner
  calls `set_definitions` on its renderer, and `pm_export::plan_export` and
  `StillRouting` route with `unsupported_with_definitions`; `pm-cli` reads the
  snapshot a collected project embeds.
- **The engine process** (`pm-engine`) applies `pushDefinitions` to the session's
  `DefinitionSet` without translating, so `definitionsApplied` carries no failures; the
  interactive engine's presenter translates the set when it arrives and sends each
  failure as a `failure` event instead (definition scope). Translating a cold set of
  the 20 built-ins takes about 150 ms on the presenter's render thread.
- **Adjustment layers** with effects stay routed (they need `FRAG_COMPOSITE`).
- **WebGL-unstable class** pending review (above).
- The generated corpus was written by an agent from the docs, not produced through the
  in-app agent harness (S3 provenance gap); Store has 3 public extensions, not in this
  corpus (their sources stay outside the repository); one machine and one GPU family.
- WebGL's "Expression too complex" rule and ternaries on structs and arrays are not
  reproduced (S3).
