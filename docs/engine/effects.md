# Effects, transitions, and the GLSL pipeline

Plan section 11 (all of it), section 8 (the watchdog; the effect pass and transition
nodes), section 25 (the shader gate, the WebGL-unstable class), amendment 9, and
contract 10. Code:

- `engine/crates/pm-glsl`: the production form of spike S3's naga path
  (`docs/engine/spikes/S3.md`): limits, ES validation, normalize, glslang, the SPIR-V
  rewrites and bind check, naga; the translation cache; isolation under a deadline; the
  fuzz target `pm-glsl-fuzz`.
- `engine/crates/pm-render/src/effects/`: definitions (contract 10) and their programs,
  the renderer's effect host (`EffectHost for Effects`: the effect pass and transition
  nodes the compositor runs), GL's uniform binding rules, the E encode and decode around
  legacy programs, the GPU watchdog; `builtins.json` embeds the built-in snapshots.
- `engine/crates/pm-render/src/composite/effects.rs` (the compositing lane's seam): where
  the chains run; see "Where effects run".
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
| 2 | `#version 450`, precision removed, GLSL 4.50 and Vulkan keywords renamed, polyfills (`inverse`, `outerProduct`, `modf`, scalar `normalize`/`faceforward`/`reflect`/`refract`, `round`, `radians`, `degrees`), line counts unchanged | `normalize` |
| 3 | glslang, relaxed Vulkan rules, automatic bindings and locations | `compile::compile` |
| 4 | Sampler arrays, distinct bindings, varyings by name, the fragment interface (narrow, array, and depth outputs; unused inputs), signed `%` as `OpSRem`, `uint` negation, `texelFetchOffset`, 2-row matrix strides, non-finite constants, the combined-sampler split; SPIR-V limits (call depth, recursion, private variables, uniform vectors); the bind check | `rewrite`, `limits::check_spirv`, `reflect::unbound_resources` |
| 5 | naga SPIR-V front end with `adjust_coordinate_space` (GL row order), derivative uniformity off, validation | `pipeline::Prepared::finish` |
| 6 | Cache by SHA-256 of (pipeline revision, glslang and naga versions, the device context, line offset, source): memory (512 entries) and disk (atomic JSON entries; a damaged or foreign entry is a miss); failures in memory only | `cache::Cache` |
| 7 | Diagnostics in the author's lines (`WrappedGlsl::body_line_offset`); preamble lines are reported as line 0, "generated code" | `errors::map` |

Pipeline creation (the Metal compiler) happens in pm-render on first use per target
format; Metal's own cache makes later builds cheap.

**ANGLE's Metal calls** (`pm_render::glsl`, 2026-10-19). Today's compositor
runs on ANGLE's Metal back end, which translates GLSL to its own MSL; Metal
compiles both translations with fast math, and where the two call different
built-ins they round differently. The engine no longer lets wgpu write the
MSL: `GpuProgram::new` writes it with naga's writer and wgpu-hal's options and
Metal slots (`glsl::slots`, from `create_pipeline_layout`), makes ANGLE's
calls, and compiles it as a passthrough module with Metal's default fast math
(a device without passthrough MSL keeps wgpu's translation). The calls come
from ANGLE's own output: `WEBGL_debug_shaders`' `getTranslatedShaderSource`
in headless Chromium with `--use-angle=metal --enable-privileged-webgl-extensions`
over all 604 programs of the `glsl-20261018-193347` and `shaders-20261018-022611`
corpora, compared call by call with naga's MSL (the dump script and the
comparison are kept outside the repository with the review evidence).

| GLSL | naga writes | ANGLE writes | Now |
|---|---|---|---|
| `pow` | `metal::pow` | `metal::powr` | `powr` |
| `normalize` (vectors) | `metal::normalize` | `metal::fast::normalize` | `fast::normalize` |
| `tanh` | `metal::tanh` | `metal::precise::tanh` | `precise::tanh` |
| `round` | `metal::rint` (glslang and naga fold it into `roundEven`) | `metal::round` (ties away from zero) | `pm_glsl`'s `pm_round` polyfill (exact ties away from zero on every back end), whose Metal body is `return metal::round(x);` (`ANGLE_POLYFILLS`) |
| `radians`, `degrees` | `(x) * c` | `ANGLE_radians`: `c * x` | `pm_glsl` polyfill, `c * x` |

`round` is a correctness fix as well: `round(0.5)` was 0 in the engine and is
1 in WebGL (`tests/effects.rs` `round_ties_away_from_zero_as_in_webgl`, which
fails without the mapping). Since the second review (2026-10-19) `pm_round` no longer
wraps `roundEven`: it computes ties away from zero itself (`t = trunc(x)`,
then `t + sign(x)` where `abs(x - t) >= 0.5`; `x - t` is exact, so the
largest float below 0.5 rounds to 0, which `floor(x + 0.5)` gets wrong), so a
back end without the passthrough MSL (wgpu's own translation, e.g. a future
DX12 path) rounds as WebGL too; on Metal the MSL writer replaces its body
with ANGLE's `metal::round` (the same value). `pm_glsl::normalize::tests`
`pm_round_rounds_ties_away_from_zero_exactly` (200,000 values and the edge
cases against `f32::round`) and `pm_round_does_not_reach_a_back_end_as_ties_to_even`;
`glsl::tests::round_polyfill_reaches_naga_as_a_named_function` (each Metal
overload body is the single call). Measured after the change (`effects_webgl`, `PM_SHADER_STATS=1`,
`glsl-20261018-193347`): the same 16 variants not bit-exact as before it
(none a `round` probe; every built-in variant bit-exact), and the shader
gate on `shaders-20261018-022611` unchanged (507 of 522 match, 0 failures). The other calls ANGLE writes as its own helpers
(`ANGLE_inverse`, `ANGLE_mod`, `ANGLE_outerProduct`, the scalar
`faceforward`/`reflect`/`refract`/`distance`, `ANGLE_componentWiseMultiply`)
have the same expressions as `pm_glsl`'s polyfills or naga's output; `dFdx`
and `dFdy` are ANGLE's `dfdx`/`dfdy` times its flip uniforms (±1 or 0),
numerically the same as the engine's. Measured on `glsl-20261018-193347`
(`effects_webgl`, `PM_SHADER_STATS=1`): variants not bit-exact against WebGL
went from 56 to 16 (every built-in effect and transition variant now
bit-exact; Levels and Gradient were not), the rest within 1 CV at a few
pixels of probes (`radians`/`degrees`, `halftone-fragcoord`,
`outer-product-twist`, `mix-bool`, `matrixCompMult`, `outerProduct`,
`modf-infinity`) and the known-open probes (`gl_FragCoord`, `output-vec3`,
undefined behaviour). The app corpus is unchanged (one 3D studio frame
closer).

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
engine-only: WebGL draws the program, so it runs in the decision 19 fallback runner with
its textures (`Status::NeedsFallback`, reported as `NotHeadlessRenderable`; "The fallback
runner" below). A renderer without a runner (`pm-cli`) names it as not
headless-renderable. Nothing is drawn wrong, and nothing is routed.

**ANGLE-only programs** (`effects::ANGLE_ONLY`, orchestrator decision DEFS-2,
2026-10-19). One Store program still differs natively with ANGLE's calls:
`mrsigma/liquid-glass-ios27` 2.0.8 (`liquid-glass-ios27.glass`). Its rim
search bisects the bilinear alpha of `u_orig` at coordinates built from
screen-space derivatives, a 2×2 inverse, and `cos`/`sin`, and branches on
`alpha < 0.5`; the hardware filter weighs texels with 8-bit fractions, so a
coordinate one ulp apart can move the rim a bisection step. Intermediate
probes of the program against WebGL (a private copy of the gate case whose
output is replaced by each stage's values) localise it: the Jacobian and
`lgP2S` match at 8 bits, the ray-seeded `best` distance already differs at
44 pixels, and everything after it inherits that. MSL `fp contract` pragmas
(`off`, `on`, `fast`) change nothing. So the program is listed, by the
SHA-256 of its user body with every number written as `0` (the extension
bakes the composition size into the body), and where a fallback runner is
attached (the app's viewer and its exports) it draws through it, the same
ANGLE that draws it today; a renderer without one (`pm-cli`) draws the
native translation. Its GPU program is still built, so `validate_effect`
compiles it. Built-ins (all bit-exact natively) and layers are never
listed; the Store's other `liquid-glass-ios27` (zellzoi 2.0.1) matches
natively and has another fingerprint. (Correction, second review: the
`liquid-glass-ios27` in Chike's setup snapshot, `user-setups/chike-20261018`,
is manifest 2.0.1, byte-identical to zellzoi 2.0.1's `index.ts`, not
mrsigma 2.0.8; earlier notes said he uses 2.0.8.) Through the runner the program is
bit-exact against WebGL in all four variants (`effects_webgl`
`fallback_effects_match_webgl` on a private mini corpus of the gate case,
`PM_SHADER_STATS=1`: 36,864 of 36,864 pixels each); the shader gate draws
it with a runner host (`<corpus>/fallback-effects`) and counts it as via the
runner. Tests: `effects_fallback.rs`
`an_angle_only_program_draws_through_the_runner_where_one_is_attached`
(viewer request with stage `angle-parity`, the runner's pixels drawn, a job
through its host, native without a runner, the built-in it copies stays
native); `glsl::tests::fingerprints_ignore_numbers_and_spacing_only`. The
cost is the runner's (see the throughput gap).

## Where effects run

The compositor (`crate::composite`, `docs/engine/composite.md`) owns the graph: a layer
that leaves today's fast path is a `composite::Node`, and its steps run in today's
order: content, masks, effects, track matte, composite. The effects lane plugs in at
`composite::EffectHost`, which `Effects` (the renderer's definition set) implements;
`Renderer::set_effect_host` replaces it in tests only. At plan time:

- `EffectHost::prepare(scope, layer, frame)` returns the layer's `PreparedEffects`
  (`effects/plan.rs`): the **effect pass nodes** (`EffectStep`), in `L.fx` order:
  enabled (`<fx>.$enabled`, animatable), not `missing`, definition registered.
  Parameters as `effectParamValue` reads them (no property or a nullish value: the
  default) and as `setParam` converts them: colors by `hex2rgb(String(v))` (first `#`
  removed, 3 or 4 digits doubled, `parseInt(…, 16) || 0` per channel, a sign accepted),
  toggles 0 or 1, numbers `Number(v) || 0`. With them the chain's total `padding` and
  whether a step reads `u_backdrop`. `Ok(None)` when no effect renders (all off or
  missing): the layer stays on the fast path; `Err(feature)` routes the composition
  (`Feature::Definition` for an unknown id).
- `EffectHost::prepare_transition` returns the **transition node** (`TransitionStep`):
  `activeTransition`'s edge and progress, with `prop ? evP : pd.def` parameters.
  `transition_known` answers `PM.transitionDef` for the group boundary rule.

At record time the compositor calls `EffectPasses::record(input, backdrop)` for a layer
(input = the masked layer target; backdrop = the accumulator beneath it, given only when
a step reads it) and for an adjustment layer (input and backdrop = the accumulator:
`runEffects(acc, acc)`; the result is then masked and composited over the accumulator
with the layer's opacity and blend through `FRAG_COMPOSITE`). A transition runs after
the layer's composite: `withLayer` is the accumulator with the layer composited (normal:
a copy plus the draw; other modes: `FRAG_COMPOSITE`), then
`TransitionPasses::record(before = acc, with_layer)` over the whole frame; `Ok(None)` (a
program that cannot run) leaves `withLayer`, as `runTransition` returning null.

Execution (`effects/exec.rs`) is in GL row order like every pm-render target:

- Each effect pass: a new buffer the size of the pass rectangle, cleared to
  transparent, blending off, the quad through `PM.VERT`, the compositor's calls in
  order: `u_tex` = 0, `u_orig` = 1 (with `keepOrig`), `u_backdrop` = 2 (with
  `backdrop`), `u_m`, `u_res`, `u_uv`, `u_coordRes`, `u_pxOrigin`, `u_texel`, `u_time`,
  `u_pass`, then the parameters. GL's rules (`effects/program.rs`): a call of the wrong
  type changes nothing (a single float never sets a `vec2`; `uniform1f` never sets an
  `int`), uniforms are program-wide (both stages' blocks), named parameters bind when
  the linked program has them active, else the positional alias `u_p<i>` or `u_c<i>`; a
  sampler reads unit 0 unless a `uniform1i` call moved it; an empty unit samples (0, 0,
  0, 1). Every texture is sampled linear with clamp-to-edge, today's state for every
  effect and transition buffer (`grab`).
- `clamp: true` (a definition flag the kernel passes through) clamps the effect's output
  to 0..1 in an extra pass; otherwise values above 1.0 pass (rgba16float or rgba32float).
- A step whose program the engine cannot build at record time (WebGL draws it) adds
  `Feature::ShaderFallback` and the frame routes.

**Color domain** (section 11: "legacy GLSL receives E-encoded values independent of the
display"). In Standard the working encoding is E and the chain runs on the targets as
they are. With linear blending on, `Gpu::chain` encodes the chain's input and its
backdrop with E before the first program and decodes the result after the last
(`SimpleOp::EncodeE`, `DecodeE`), and a transition's two inputs are encoded and its
output decoded the same way, so every legacy program reads and writes E values; the
compositing around it (the layer's composite, the adjustment's `FRAG_COMPOSITE`,
`withLayer`) blends in linear light. Both passes divide by alpha only within
(`PM_UNPREMULTIPLY_FLOOR`, 1]: legacy programs write alpha above 1 (Glow adds its light
over `u_orig`, so alpha reaches 1 + intensity) and color with zero alpha (chromatic
aberration's fringes); such color is light and is converted per channel. The frame's
output encode (`composite/ops.wgsl` `fs_encode_flip`) does the same, so a pixel no
linear-light blend touches displays the E value it displays in Standard. (In Standard,
an alpha above 1 also darkens the stack beneath by the excess, as WebGL does; with
linear blending on that composite is in linear light, like every other.) Today's
compositor has no linear blending, so there is no WebGL reference for this mode;
`tests/effects_linear.rs` holds it to Standard wherever no linear-light blend is
involved (see Parity).

**Regions of interest** (`composite::region`). A pass holding an effect renders over the
region grown by the node's padding, clamped to the frame (`Padding::Full` for an effect
that is not viewport-safe and for a transition, which reads the whole frame), with the
crop's effect coordinates (`effectViewportCoordinates`: `u_uv`, `u_pxOrigin`,
`u_coordRes`), and the region is cut from it. A step's padding is the sum of its
`viewportPadding` parameter values; a chain's paddings add up (each step reads the
previous step's output). Today's viewer (`extensions/viewer/viewer.ts`
`effectViewportPadding`) instead pads its crop by 128 px plus the largest single
`viewportPadding` value of any enabled effect in the open composition. The engine's
rule reads at least as far for every chain (the sum of one step's keys is at least
their maximum, and chained steps add), so a region equals the full frame where
today's crop may not; it renders more pixels for long chains. Tests: a blur, chroma,
and vignette chain's padded crop of a 1280 × 720 frame equals the full frame (measured
0 CV), a viewport-safe blur and an invert under a transition over two regions within
1 CV (`tests/effects.rs`), and the compositor's regions with masks, mattes, blends,
adjustments, and padded effects bit for bit (`tests/composite.rs`). Note: Mosaic computes
its grid from `v_px - u_pxOrigin` (crop-local), so its grid follows the crop in today's
zoomed viewer and here alike.

**Definitions in the plan.** `Renderer::set_definitions` (an embedded
`DefinitionSetSnapshot`) or `apply_definitions` (a kernel `DefinitionsUpdate`) configure
the renderer. Without definitions, effects, transitions, and adjustment layers with
effects are routed as before (`Feature::Effects`, `Feature::Transition`,
`Feature::Adjustment`). With them:

| Case | Today | Engine |
|---|---|---|
| Effect registered, program ready | drawn | drawn |
| Effect id the document marks `missing` | skipped (fast path) | skipped (fast path) |
| Effect id marked missing in the set (removal, omitted from a full update, `missing` of an embedded set) | `PM.FX[type]` undefined: skipped, layer buffer path | same |
| Effect id unknown (no snapshot, not missing) | n/a | routed (`Feature::Definition`); contract 10: `pm-cli` fails only here |
| Program WebGL refuses | skipped | skipped, `ShaderCompile` reported |
| Program WebGL draws, engine cannot translate | drawn | drawn by the fallback runner with its textures (viewer: the window's runner; jobs: a window's runner through the host); `NotHeadlessRenderable` reported; `pm-cli`: named failure, exit 4 |
| Program the fallback runner refuses (WebGL refuses it too) | skipped | skipped, `ShaderCompile` reported |
| Fallback draw past the runner's 10 s deadline | context lost | viewer: turned off, `WatchdogTimeout`; job: stops with a named error |
| Watchdog overrun | n/a | skipped from the next frame, `WatchdogTimeout` reported |
| Transition registered | drawn | drawn |
| Transition unknown / missing / program refused | — / no transition / `withLayer` | routed / no transition / no transition |
| Effects on adjustment layers | `compositeAdjustment` | drawn (the compositor's adjustment node with the real chain); the same per-effect rules |

`capability::unsupported_with_definitions` gives the same answer statically for routing
consumers; `unsupported` and `unsupported_with_media` are unchanged (they list effects,
transitions, and adjustment layers with effects). The interactive engine's presenter
(pm-node `present/document.rs`) routes with `unsupported_with_media` and, once the
session has definitions, ignores those three features; a definition it cannot draw
fails the frame as unsupported and routes then; a definition only the runner draws does
not: the presenter sends its request (and textures) to the window's runner and presents
the frame once the pixels arrive, and a runner failure marks the program (skipped or
turned off, reported) instead of routing (`present/render.rs`). Built-ins: `pm_render::builtin_definitions()` (the 16 effects and 4
transitions as the kernel snapshots them; a vitest fails when `builtins.json` drifts
from the kernel; `bun engine/crates/pm-render/tools/effects/builtins.ts` rewrites it).

## The watchdog

Issue A2 (2026-10-18) rebuilt it around two guards, judged when a frame's
commands have completed (nothing waits during rendering: `Watchdog::collect`
runs at the start of later frames), and a deterministic test hook.

- **The budget records only** (review fix, 2026-10-19, amendment 22):
  today's compositor (`legacy/gl/gpu-timing.ts`, `compositor.ts:1264`)
  times programs and never turns one off, so a slow but finite user
  program that draws today must draw here. `Watchdog::measured_ms` keeps
  the longest time per program; no production renderer enforces a budget.
  Enforcing it is an opt-in (`Watchdog::set_budget`, tests only) and never
  applies to built-ins (`Span::builtin`), so a built-in such as Gaussian
  Blur on a large composition and a slow GPU is never turned off. What
  follows describes the enforced budget. Each effect instance, transition,
  shader layer, and extension fragment layer of a timed frame is timed
  between two marker dispatches (one-workgroup compute passes whose
  end-of-pass sample is the timestamp, as `pm-export`'s `GpuSpan`: the first
  reads a texel of the program's input, the second of its output). The
  timestamps are resolved in a submission of their own once the frame's
  command buffer has completed (resolved in the frame's own command buffer,
  Metal returned the previous frame's end sample; found 2026-10-17). With
  an enforced budget, a user program over it is
  disabled from the next frame, skipped with its data kept, and reported
  (`WatchdogTimeout`; effect scope for an instance, definition scope for a
  transition, layer scope for a shader or fragment layer) until it is
  registered again: a kernel delta that upserts it (a register or hot
  reload of that id, even with the same source), a changed source in a full
  set, or its removal. A shader layer's code is disabled until the code
  changes (another source key) or `Watchdog::reset`. A full update or an
  embedded set that repeats an unchanged definition (session start, after a
  refused push, another view's set) keeps it off. `Effects::apply` forgets
  the records of a delta's upserts; the interactive engine loads full sets
  per view, so pm-engine passes a delta's upserted ids to
  `FramePresenter::definitions` and pm-node's `Composer::use_definitions`
  forgets them.
- **The hang guard** (every renderer, every frame, not only the one frame
  timed on the GPU), in two steps (review fix, 2026-10-18):
  1. *Suspicion.* Every frame drawing a user program is followed from when
     it could start (its submission, or the completion of the frame before
     it). It is suspected only when a poll made after the hang budget (10
     s, Chromium's own GPU watchdog, in the viewer as in jobs since
     2026-10-19; it was 2 s in the viewer, which turned off user programs
     today's WebGL still finishes) saw it still running. A completion seen late is not suspicion: wgpu fires
     `on_submitted_work_done` at the next poll or submit, so a frame that
     finished while nobody polled (the presenter waiting for the WebGL
     runner, a busy render thread) is not a runaway. Hosts poll between
     renders (`Renderer::watchdog_watch`), and job hosts wait through
     `Renderer::wait_idle`, which polls every 100 ms, so a runaway is seen
     running and its frame is never delivered. While a suspect runs, the
     viewer holds frames that would draw one of its user programs
     (`RenderError::WatchdogPending`) instead of submitting the runaway again.
  2. *Attribution*, once the suspect completed (or the system aborted it):
     a program is guilty when its own timestamps show it ran past the hang
     budget, or began and never ended in a frame whose wall time passed the
     budget. Programs whose timestamps show they finished are cleared, so GPU
     contention (another process, an export on the same GPU, a heavy frame
     of built-ins or 3D) turns nothing off. Without timestamps (no timestamp
     queries, or both query slots busy) a program is guilty only after two
     unexplained hangs.
  A guilty program is disabled and reported; a job stops. Built-ins cannot
  run away and are never suspected. A true hang ends when the operating
  system's GPU timeout aborts the command buffer; device-loss recovery stays
  the device owner's (Gaps). Before the fix the completion callback's time
  was read as the GPU's finish time and every non-built-in program of any
  frame slower than 2 s was blamed: a viewer waiting over 2 s for the WebGL
  runner, or an export on the same GPU, could turn off innocent user and
  Store effects.
- **The fallback runner's watchdog.** The renderer's runner fences every
  pass and polls it: a draw not finished within its 10 s deadline loses the
  context on purpose (so the renderer is never blocked in `readPixels`) and
  fails with `watchdog`; the program is never run again until its source
  changes. The engine turns it off like a runaway native program
  (`WatchdogTimeout`, viewer) or stops the job naming it. Compiles have their
  own 10 s deadline (`KHR_parallel_shader_compile`), which is transient
  (below), not a judgment of the program.
- **Trusted samples.** A timestamp sample is judged only when both samples
  are after the newest trusted sample, it ends after it begins, and it is no
  longer than the frame's own submission-to-completion wall time (+50 ms);
  only trusted samples raise that floor (`watchdog::plausible_spans`, unit
  tested). Before, one garbage end sample raised the floor past every later
  sample, after which the watchdog judged nothing for the rest of the
  process. Metal leaving a sample unwritten (seen with eleven test
  renderers busy in one process: 4 of 10 spans) still leaves that frame
  unjudged; the hang guard does not depend on samples.
- **Policies.** `Policy::Interactive` (the viewer): the hang guard (and an
  enforced budget, which no production renderer sets), a disabled program
  is skipped. `Policy::Job` (export jobs and agent
  stills; pm-node's job lanes set it and `reset` the watchdog per work item):
  no per-program budget (today's exporter renders a slow effect slowly, and
  a job never skips a program) and, after a runaway, every later frame fails
  with `RenderError::Watchdog` naming the programs ("The render stopped:
  acme.glow did not finish within 10 s of GPU time."), which the job reports
  as `exportFailed` with that message.
- **The test hook.** `Watchdog::set_timing_source(Some(Arc<dyn
  TimingSource>))` replaces the GPU timestamps and the wall clock: every
  frame is judged at its completion with the span and frame times the source
  gives (`FnTiming` for closures). The budget, the hang guard, both
  policies, and every re-enable rule are tested this way on any device and
  under any load: `tests/effects.rs`
  `an_enforced_watchdog_budget_disables_an_overrunning_user_effect_from_the_next_frame`,
  `tests/watchdog.rs` (`slow_programs_are_recorded_and_never_turned_off_in_the_viewer`:
  a user effect, a shader layer and a built-in at 3 s of GPU time a frame
  draw on every frame; `an_enforced_budget_never_turns_a_built_in_off`;
  shader layers, the hang guard with user and built-in
  programs, the job policy, a frame judged while still running),
  `pm-node/tests/definitions.rs`
  `a_disabled_effect_stays_off_across_unrelated_pushes_and_other_views`.
  `Watchdog::set_clock` replaces the wall clock alone, so the real
  completion path (polls, callbacks, timestamps) is tested with time passing
  at will: `tests/watchdog.rs`
  `a_frame_whose_completion_is_seen_late_is_not_a_runaway`,
  `a_frame_slowed_by_contention_turns_nothing_off`,
  `a_frame_seen_running_past_the_hang_budget_is_cleared_by_its_timestamps`,
  `a_suspect_frame_holds_frames_drawing_its_user_programs_until_it_is_judged`.
  Production renderers set neither. The GPU path keeps its own test,
  `the_watchdog_judges_every_timed_frame_not_only_the_first`.

Measured 2026-10-19 (M4 Max), after the budget became record-only and the
viewer's hang budget 10 s: `tests/watchdog.rs` 12 of 12 tests passing in
each of 10 runs; `pm-node/tests/definitions.rs` passing in 20 of 20 runs.

Measured 2026-10-18 (M4 Max): `tests/watchdog.rs` 5 of 5 tests passing in
each of 10 runs before the review fix; 10 of 10 tests passing in each of 10 runs after it; `pm-node/tests/definitions.rs` passing in 20 of 20 runs
(plus one before them). The two failures issue A2 recorded (the first test
failing and the pm-node test passing 1 in 6) did not reproduce on this tip
before the change either, idle or with two `effects` test binaries running
beside it (5 runs each): they depended on Metal writing every sample.

Built-in GPU times at UHD (3840 × 2160, defaults, a 3000 × 1700 image, release, M4 Max,
the longest of 5 frames, measured again 2026-10-17 with every frame judged): Gaussian Blur
14.4 ms, Glow 12.0, Drop Shadow 10.2, Turbulent Displace 5.3, Directional Blur 2.9,
Sharpen 0.95, every other built-in 0.66 to 0.77 ms. Translating the 20 built-ins in a cold
process: 149 ms; from the in-memory cache: 0.10 ms. First frame per effect, pipeline build
included: 63 to 112 ms.

## The fallback runner (decision 19, effects and transitions)

Review fix, 2026-10-18 (amendment 21: drawn, never routed). A step whose
program the native pipeline cannot translate (`Status::NeedsFallback`: a
sampler other than a float `sampler2D`, a naga or Metal failure on a user's
machine) runs in the renderer's own WebGL2 runner
(`engine-connection/shader-fallback.ts`), the same ANGLE that draws it today:

1. **Engine** (`effects/fallback.rs`, `effects/exec.rs`). The frame's commands
   so far are submitted, the step's textures are read back as binary16 RGBA in
   GL row order (an effect's input, its original with `keepOrig`, its backdrop;
   a transition's two frames), and a `FallbackRequest` is built with the
   wrapped program, the compositor's uniform calls for every pass
   (`morePasses`, 1 to 8 ping-pong passes), which unit samples which texture
   (`units`; `chain` samples the previous pass from the second pass on), and
   `setParam`'s positional alias per call (`u_p<i>`, `u_c<i>`, applied when
   the named uniform is not active). It is keyed by everything that decides
   its pixels, the textures included. The runner's pixels go into a working
   target that continues the chain (`clamp` after it, as a native step).
2. **Viewer** (pm-node `present/render.rs`). Without the pixels the frame is
   not presented (`RenderError::ShaderFallbackPending`): the presenter sends
   the request and then its textures in row bands
   (`ShaderFallbackMessage::Input`), the window's session collects them
   (`engine-connection/session.ts`), the runner draws, and the pixels come
   back as bands. A runner failure marks the program instead of routing:
   `compile` (WebGL refuses it too) is skipped and reported as today's
   `runEffects` skips it; `watchdog` turns it off (`WatchdogTimeout`).
   Every other failure is transient and retried (below).
3. **Jobs** (pm-node `jobs/fallback.rs`, contract 14). A job renderer waits
   on a `FallbackHost`: the lane writes the textures into the session's
   scratch cache area, sends `Request::ShaderFallback`, and main
   (`main/engine/shader-fallback-relay.ts`) reads them, runs the request in
   the window the job works for (an export's or agent batch's owner; render
   queue jobs and jobs whose window closed use any window with the new
   engine), and writes the pixels back. The runner is stateless, so any
   window draws the same pixels. `compile` skips the program as today's
   exporter does, `watchdogTimeout` stops the job; a transient failure is
   tried again in place, three attempts per frame, and if it persists the
   job stops with an error naming the program (never a frame without it).
   Each window acknowledges a request as soon as its preload listener has
   it; main passes over a window that does not within 10 s (reloading, its
   runner not installed yet) for the next one, so a job does not wait the
   110 s answer timeout on a window that never heard it. A renderer with a host does not list
   `Feature::ShaderFallback` (`Renderer::unsupported_with_media`).
4. **`pm-cli`** has no runner: such a program is a named, deterministic
   "not headless-renderable" failure (exit 4).
5. **Snapshots** (the render transport: `renderFrameTo`, `raster`, the
   thumbnails; 2026-10-19, with the merge of the bridge lane). A snapshot
   that needs the runner answers `needsRenderer` naming the requests; the
   renderer draws them and asks again. The caller may be the renderer
   thread blocked in `sendSync`, which reads no port, so an effect's or a
   transition's textures cannot follow on the session port as for the
   viewer: the interactive engine sends them to main on the host port,
   ahead of the answer on the same port (`present/render.rs`
   `fallback_inputs`); main keeps them by request key
   (`main/engine/fallback-inputs.ts`, bounded to 16 requests and 1 GiB) and
   hands them back with the answer (`EngineSnapshotReply.fallbackTextures`);
   the transport passes them to the runner (`render-transport.ts`
   `fallbackTextures`), on the calling thread through
   `ShaderFallbackRunner.renderSync(request, textures)`. `renderSync`
   shares the asynchronous path's draw (a generator over the passes): it
   links as WebGL's `LINK_STATUS` query does (blocking, no compile
   deadline; a compile an asynchronous request started is reused) and
   polls each pass's fence on the thread within the same draw deadline, so
   the watchdog holds there too. A request whose textures did not come
   fails as `internal` rather than drawing unsampled. Tests: pm-node
   `snapshot.rs` `snapshots_send_an_effects_textures_to_main_ahead_of_the_answer`
   (every band on the host port before the answer, none on the window's,
   and the frame drawn with the runner's pixels); `fallback-inputs.test.ts`;
   `snapshots.test.ts` "hands an effect request its textures";
   `native-engine.test.ts` (bands kept from the interactive engine only);
   `render-transport.test.ts` (textures on both paths; missing textures
   fail); `shader-fallback.test.ts` "the runner on the calling thread".

**Transient failures** (review fix, 2026-10-19). Only two failures say
something about a program and are kept until its source changes
(`FallbackFailure::is_final`): WebGL refusing it (`compile`) and a draw
past the runner's deadline (`watchdog`). A lost context (a GPU-process
restart, another context's loss), the compile deadline, no runner
answering, and a wrong size are transient, as today's compositor treats a
context loss: `compositor.ts:746-760` clears `GL.errors` and `GL.progs` on
`webglcontextrestored` and draws again.
- The runner makes a new context and draws the request once more; if that
  context is lost too it fails with `contextLost`. Before the fix a context
  lost under a draw or a readback was reported as `compile` and kept in a
  map that survived context recreation, so the window never drew that
  program again, and a job skipped it as if WebGL refused it. A link status
  read from a lost context (null) is no longer taken for a refusal either.
- The engine (`effects::fallback::Retries`, shared by effects,
  transitions, and shader layers) asks for the program again at once after
  the first transient failure, then after 250 ms, 1 s, 4 s, and 8 s from
  then on; meanwhile the viewer draws the frame without the program (a
  shader layer draws nothing), reports it once as `NotHeadlessRenderable`,
  and the presenter wakes to draw the view again when the retry is due
  (`Renderer::put_fallback_failure` returns the delay). The streak ends when
  the runner draws the program. Nothing is turned off or routed.
- Tests: `tests/effects_fallback.rs`
  `a_lost_context_is_not_blamed_on_the_program_and_it_draws_again`,
  `a_job_tries_a_lost_context_again_and_draws_the_program`,
  `a_job_whose_runner_keeps_losing_its_context_stops_naming_the_program`;
  `shader_layers::exec` `a_lost_runner_context_is_retried_and_the_layer_draws_again`;
  `effects::fallback` unit tests (backoff, which reasons are final);
  `shader-fallback.test.ts` "the runner after a lost context" (a new
  context draws again; contextLost when that is lost too, then drawn once
  the GPU is back; a lost link status; a refusal kept);
  `shader-fallback-relay.test.ts` "acknowledgements". The session keeps at
  most 32 requests waiting for their textures (the engine's own bound),
  failing the oldest as transient.

**Target size** (review fix, 2026-10-19). The runner used to refuse any
target edge over 4,096 px (`maxSize ?? 4096`) with `size`, retried as
transient: a 5K to 8K export of a project with a fallback-only effect failed
after three attempts and the viewer retried every 8 s, while today's
`runEffects` and `runTransition` draw at the composition's size (up to
16,384 px, `editing.ts`). The 4,096 px cap is the shader layers' alone
(`compositor.ts:1040`; the engine caps those requests itself,
`shader_layers::plan`). Now:
- The runner's limit is its context's `MAX_TEXTURE_SIZE` (16,384 on Apple
  GPUs), the limit today's `grab` buffers have. A target past it fails with
  `tooLarge` before compiling (`maxSize` is a test option only).
- `tooLarge` is final (`FallbackFailure::is_final`) and kept for the program
  at that size only (`sized_program_id`), so a lower preview resolution or a
  smaller export still draws it. The viewer skips the step and reports it
  once as `BudgetExceeded`; a job stops with an error naming the program and
  the limit, after one attempt. Main's relay sends it as `budgetExceeded`,
  which the job host reads back as `tooLarge`.
- A malformed request (texture count or length) stays `size`, transient.
- The protocol carries those sizes (second review, 2026-10-19):
  `pm_protocol::shader_fallback::MAX_EDGE` was still 4,096, and every
  envelope is validated when it is encoded, so a job's `ShaderFallback`
  host request, a viewer's request, and their texture bands past 4,096 px
  could not be sent at all (the earlier 4,500 px test drove the renderer
  with a test host, not the protocol). It is 16,384 now
  (`shader_fallback::tests`, an 8K effect request, host request, and band).
- Tests: `effects_fallback.rs`
  `a_job_draws_a_runner_step_wider_than_4096_px_at_the_compositions_size`
  (4,500 px), `a_target_past_the_gpus_webgl_limit_is_a_final_named_failure`;
  `shader-fallback.test.ts` "draws effect targets up to the context's
  MAX_TEXTURE_SIZE" (7,680 px) and "fails a target past MAX_TEXTURE_SIZE
  with tooLarge"; `shader_layers::fallback` unit tests.

**Rounds** (review note, 2026-10-19). A waiting step blocks every later
runner step of the frame (`EffectFallback::blocked`, frame-wide): its
pixels may feed them (a request is keyed by its textures' pixels), and the
engine does not track which targets derive from a left-out step. So a frame
takes one runner round trip per runner step it draws, even when the steps
are on independent layers, and each round renders the frame again; in the
viewer each such step adds a serial round trip. The job loop's old fixed
limit (16 rounds, then a failure today does not have) is replaced by a
progress rule: each round must ask for a request not asked before in the
frame, with a 1,024-round backstop. Job frames pin every result the host
answers until the frame is delivered (`FallbackStore::pin`): two 8K
results (265 MB each) are past the store's 512 MB budget and used to evict
each other, so a chain of them would have asked forever. Test:
`a_job_draws_more_runner_steps_than_a_fixed_round_limit` (20 steps; the
job's 20 requests are all new and its frame equals the viewer's);
`shader_layers::fallback`
`a_pinned_frame_keeps_every_result_past_the_budget_until_unpinned`.
Pinned results have no ceiling within a frame (20 8K steps would hold about
5 GB): a ceiling would fail an export today draws, and today's compositor
draws such a chain too; it is left to the memory arbiter.

**Compile deadline** (review note, 2026-10-19). A request waits up to 10 s
for its program to compile (`KHR_parallel_shader_compile`). A compile past
it is no longer deleted: it goes on in the driver, and the program's next
request (the engine retries `deadline` as transient) waits for the same
compile, so a program WebGL takes long to compile draws once it has, as
today's compositor waits for it (`shader-fallback.test.ts` "a program WebGL
takes long to compile"). In a job, an answer that the program is still
compiling does not count as an attempt: the job asks again for up to
`pm_render::HOST_COMPILE_WAITS` (60) such answers, about ten minutes of
10 s deadlines, as today's exporter waits for its compile; past that it
stops naming the program (`effects_fallback`
`a_job_waits_for_a_program_webgl_takes_long_to_compile`,
`a_job_whose_program_never_finishes_compiling_stops_naming_it`). The viewer
keeps retrying with backoff.

The runner (`ShaderFallbackRunner.render(request, textures)`) uploads each
texture as RGBA16F (linear, clamp to edge, as the compositor's `grab`
buffers), leaves units no request names empty (samplers read (0, 0, 0, 1),
as in the compositor), draws each pass into a fresh RGBA16F target cleared
to transparent with blending off, and fences every pass under the draw
deadline (the runner's watchdog, above).

Evidence (2026-10-18, M4 Max):
- Shader level against the compositor's WebGL replay: corpus
  `glsl-20261018-193347` adds three fallback cases (`collect.ts`): a 3-pass
  effect with `keepOrig` and a positional alias, a backdrop effect with a
  named number, color, and an aliased toggle, and a transition with a
  number and a color, each with default, alt, coerced or random values and
  above-1.0 inputs. They and the 8 sampler probes, 31 variants, run through
  the engine, the request goes to the real runner in headless Chromium
  (`tools/shaders/fallback-runner.ts --fallback`), and the composed result
  is compared with WebGL: 31 of 31 match within 1 CV
  (`tests/effects_webgl.rs` `fallback_effects_match_webgl`). A non-float
  sampler is drawable in WebGL only on a unit no float sampler uses, so
  these cases name it after a unit the compositor sets (`u_orig` with
  keepOrig, `u_to` for transitions).
- `tests/effects_fallback.rs` (6 tests: without a runner it is still
  `Feature::ShaderFallback`; the viewer asks with the effect's textures and
  draws the pixels; a job renderer waits on its host and matches the
  viewer; a refused program is skipped and reported once; a draw past the
  runner's deadline is turned off and stops a job; a job without a runner
  fails naming the program), `pm-node/tests/shader_fallback.rs` (requests,
  texture bands, and pixels end to end through a child engine),
  `pm-node/tests/jobs.rs`
  `effects_only_the_webgl_runner_draws_run_in_a_window_through_the_host`,
  `shader-fallback.test.ts` (texture bands, units, the chain, `u_orig` and
  `u_backdrop`, aliases, the draw deadline), `shader-fallback-relay.test.ts`
  (main's relay: owner window, another window, failures, timeouts, the
  scratch files).
- `e2e/engine-definitions-export.spec.ts` "an effect only the WebGL runner
  draws is exported through the window's fallback runner": with Use New
  Engine on, a raw effect with a `sampler3D` is exported by the job engine
  (AVAssetWriter, not the current exporter's FFmpeg) and drawn by the
  window's runner over the layer (passed).

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
  deltas numbered by one; a refused update makes the next one full. A viewer session
  (`engine-connection/session.ts`) resends the full set at once after the first
  refusal (a generation gap after an engine restart), then backs off from 250 ms,
  doubling; after 6 refusals in a row it stops and the next registry change sends the
  full set (`SessionDefinitions.stale`). A closed session's stream leaves the list
  `createEngineDefinitions` keeps.
- `engine-definitions.ts` installs it with "Use New Engine" on: main's relay gets the
  stream (`nativeEngine.pushDefinitions`), `PM.EngineDefinitions.session(send)` gives an
  engine session its own stream, and `GL.compileError` answers for engine reports too.

Engine (`pm_render::effects::Effects`): applies updates in generation order
(`pm_protocol::definitions::DefinitionSet`), translates what changed (a same-source
re-push reuses the program; a changed one gets a fresh watchdog record, as does every
upsert of a delta), and returns the per-definition failures
(`DefinitionsApplied.failures`).

**Program cache on disk.** `Renderer::set_program_cache_dir(dir)` keeps translated GLSL
in `dir/glsl` (`pm_glsl::Cache`; 3D programs in `dir`). Main passes
`<userData>/Engine Cache/Shader Cache` to the engine processes
(`POWERMOVE_ENGINE_SHADER_CACHE`) and the interactive engine's composer uses it, so only
the first launch translates the built-ins on the render thread. Measured
(`tests/effects.rs`, release, M4 Max): the 20 built-ins in 120 ms cold, 1.4 ms from the
folder. The folder outlives the engine build that wrote it, so every change to what
`pm_glsl` emits bumps `pm_glsl::PIPELINE_REVISION`, which is part of every key and every
entry: revision 2 (2026-10-19) is the `round`, `radians`, and `degrees` polyfills, so a
machine that ran an earlier build translates programs calling them again rather than
loading revision 1's SPIR-V (where `round` was ties to even). `cache::tests`
`an_entry_written_under_an_earlier_revision_is_not_reused` plants revision 1 entries
under both keys; `emitted_programs_match_the_pipeline_revision` hashes what a set of
programs over every polyfill prepares to and fails when it changes without a bump.

**Shader meta.** `EngineShaderMeta` fills `uiState.getShaderMeta` and the compile errors
from engine reports at their revision: definition failures go to every extension layer
of that definition (`shaderKey: extension:<id>`) and to `GL.compileError('fx:<id>')`,
`tr:`, `extension:`; layer reports set `shaderKey: engine:<layer>` (compile errors),
`missing` (missing definitions), `missingAsset` (offline media), or `sceneError`; a
report older than one applied for the layer is ignored; a re-pushed definition clears
its errors.

**Jobs, stills, and `pm-cli`** (2026-10-18). Exports and agent stills draw
effects, transitions, and extension layers instead of routing them:

- Main's relay (`main/engine/definitions.ts`) versions each window's set.
  Every job session main opens gets the owning window's set: export sheet,
  agent export, and render queue snapshots (`AgentJobs.openSnapshot`), and
  a window's agent frame session before any batch after the window pushed
  (`AgentJobs.currentDefinitions`, in the window's frame-session chain so
  pushes never race). The updates are a full update and, when some
  definitions have no registration left, a delta removing them, so the job
  skips those as missing as the window does. Each update is acknowledged
  before the job or batch is submitted; a refused set fails the export
  instead of rendering without it. Render queue jobs persist the set
  captured when they were queued (`<id>.definitions.json`, owner-only,
  removed with the job) and render with it after a relaunch.
- Nothing is stale: the export sheet, the render queue, `export_video`'s
  context, and `render_frames`' plan call `PM.EngineDefinitions.settle()`
  first, which waits for queued extension reloads (`loader.whenIdle`) and
  flushes the publisher, so a publication reaches main (IPC from one
  renderer is ordered) before the render is asked for.
- pm-engine: `JobDocument.definitions` is the session's
  `DefinitionSetSnapshot`, shared per session and generation, frozen with
  the document when the work is submitted.
- pm-node's job lanes load that set into the lane renderer before planning
  (`jobs/definitions.rs`; programs kept while their source is unchanged) and
  plan with the renderer's own capability check
  (`pm_export::render::plan_export_for`, `StillRouting::check` with the
  renderer; `Renderer::unsupported_with_media` now checks against the
  renderer's definitions when it holds them, and fonts when it has them:
  `capability::unsupported_for`). A session without definitions reports its
  effects unsupported, never drawn from a stale or built-in set.
- **Compiled before acknowledged.** A `pushDefinitions` to a job session is
  answered by the job service's compile worker (`jobs/compile.rs`), in
  arrival order: every upsert goes through `pm_glsl`, and every user
  definition also has its GPU pipeline built (`Effects::build`), so
  `definitionsApplied.failures` is real (`shaderCompile` with diagnostics
  in the author's lines for programs WebGL refuses too;
  `notHeadlessRenderable` for programs only the fallback runner draws).
  Translations are cached by source.
- **`validate_effect`** (with the new engine on): the renderer runs the
  kernel's registration validator and returns the wrapped snapshot
  (`__validate_effect_snapshot`), main compiles it on a scratch job session
  (`AgentJobs.compileDefinitions`). `shaderCompile` makes the answer
  `valid: false` with the errors by line; `notHeadlessRenderable` stays
  `valid: true` with a warning that headless renders cannot draw it, so the
  tool never accepts less than WebGL does. With the engine off the tool is
  unchanged.
- **`pm-cli render`** draws with the built-in definitions, or with a
  `--definitions` snapshot (the app's winning registrations; an override by
  id draws the fork's GLSL). An effect with no snapshot that is not marked
  missing fails naming it (exit 3, contract 10); one marked missing is
  skipped; a program only the WebGL runner draws fails as not
  headless-renderable, named per layer and definition, the same on every
  run (exit 4). The summary names the definitions' source.

Evidence: `pm-node/tests/jobs.rs`
`agent_frames_and_exports_draw_the_definitions_their_session_holds` (no
definitions: unsupported; the window's set: Invert drawn; an override by id
draws the fork; disposing it restores the built-in; a removal skips it; an
export with the set is done, one without it fails `unsupported` before
anything is written) and
`pushed_definitions_are_compiled_before_they_are_acknowledged`;
`pm-cli/tests/definitions.rs` (4 tests); `main/engine/agent-jobs.test.ts`
("definitions in the job engine", 5 tests: order and acknowledgment before
submit, refusal fails the export, frame sessions after every publication
and under concurrent batches, queue jobs, `compileDefinitions`);
`render-queue.test.ts` (persisted across relaunch, removed with the job);
`bridge.test.ts` (`validate_effect` through the engine);
`e2e/engine-definitions-export.spec.ts` (passed 2026-10-18 with a release
engine build: an effect registered in the same task as the export is drawn
by the job engine, an override by id draws the override, disposing it
restores the earlier registration, and the export is never the current
exporter's). `pm-cli` warns when a project that draws definitions renders
with the built-in set (`rendering_with_the_built_in_set_warns_when_the_project_draws_definitions`),
and the render queue logs a job whose definitions file cannot be read
(`render-queue.test.ts`).

**Compile errors follow whoever draws** (2026-10-18, review fix). With the
new engine on and the engine presenting the composition (the viewer
session's route is `engine`, `EngineShaderMeta.setRoute` from
`engine-connection/install.ts`), `GL.compileError` answers every engine key
from engine reports only: `fx:`, `tr:`, `extension:`, and a shader layer's
legacy `sh:<layer>:<hash>` key (read as that layer's `engine:` report). A
program the engine compiles shows no error, and an error the legacy
compositor recorded before for the same key is never shown again. While the
legacy compositor draws (no viewer session yet, a session closed, or the
composition routed for a feature the engine does not draw yet), those keys
are the legacy compositor's again, so a shader layer it draws with broken
GLSL shows its error in the Inspector as today. `engine:` keys exist only in
engine reports and are always the engine's.
Definition compile failures carry the definitions generation they were
translated at in `FailureReport.revision` (pm-node `present/render.rs`); one
older than the session stream's latest push of that definition
(`SessionDefinitions.changedAt`) is the translation of a set already
replaced and is ignored. A shader layer the watchdog turned off shows as its
error until its code changes. At each new revision the engine sets the shader
meta the legacy compositor set for every shader and fragment extension layer
it draws without a report (`shaderKey`, the registered `definition`,
`missing`); 3D, scene, and mesh layers keep theirs. Uniform metadata
(`udefs`) stays the Inspector's, as today. Tests:
`definition-snapshots.test.ts` (5 new).

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

Rerun 2026-10-18 on `glsl-20261018-193347` (the same cases plus the three fallback
cases), with programs only the fallback runner draws in a column of their own (compared
with WebGL by `fallback_effects_match_webgl`, above) instead of known open:

| Group | Variants | Match WebGL | Fallback runner | Both refuse | Known open | Failures |
|---|---|---|---|---|---|---|
| Built-in effects | 176 | 176 | 0 | 0 | 0 | 0 |
| Built-in transitions | 52 | 52 | 0 | 0 | 0 | 0 |
| Fallback | 23 | 0 | 23 | 0 | 0 | 0 |
| Generated (S3) | 219 | 211 | 0 | 8 | 0 | 0 |
| Coverage probes | 394 | 260 | 8 | 122 | 4 | 0 |

Known open (each also in S3.md): until the fallback runner drew them, 8 probes whose only
sampler is not a float `sampler2D` (WebGL samples (0, 0, 0, 1); now drawn by the runner and
matched), ternary on a
struct or an array (WebGL refuses; the engine accepts: needs expression types), `out
vec3` (GL leaves alpha undefined; 320 values at 2 CV), integer division by zero
(undefined). `output-ivec4` links in WebGL and its draw fails; the engine refuses it at
pipeline creation.

**App level** (`engine/corpus/src/projects/render-effects.ts`, captured through the app;
`tests/corpus.rs` with the built-in definitions): every built-in effect at two parameter
sets on a photo, an alpha image at 85% opacity, and a rotated solid; every transition in
and out on a photo and a solid; a stack with Glow, an animated enable, Drop Shadow, a
disabled effect, and a missing one; effects on a group, inside a precomp, on a precomp
layer, and a transition on a group; effects with the compositor's other steps
(`render/fx-compositing`: a masked blur, Glow with screen, Drop Shadow before a track
matte, a transition after a multiply composite, an adjustment with animated opacity, a
masked adjustment with multiply); adjustment layers (`render/fx-adjustment`: a blur over
a time range, Glow and Duotone feathered with screen at 80%, Drop Shadow at 60%, alpha
content over transparency, an adjustment inside a precomp). Capture
`effects-20261017-060101-ca4d364f` (87 samples, each opaque and as a transparent float
frame), measured 2026-10-17: 84 samples within 1 CV everywhere (max 1 CV; their float
frames within 0.63 CV), the 3 Posterize samples in the WebGL-unstable class (below), 0
failures. With the newest v1, render, and composite captures, `corpus.rs` compares 202
frames: 0 failed, 6 (the 3 Posterize samples, opaque and float) listed apart. Again
after the wave 4 merges (text, motion, 3D), 2026-10-17: 228 frames compared, 0 failed,
the same 6 listed apart.

**Linear blending** (`tests/effects_linear.rs`, no WebGL reference exists): each of the
16 built-in effects (the corpus's non-trivial parameters for Color, Levels, and
Gradient), on an opaque picture larger than the frame over black and on a solid over
transparency; the 4 transitions at progress 0.2, 0.5, and 0.8 over a full-frame
picture; a raw effect mixing the layer with `u_backdrop`; adjustment layers with
Invert, Blur, and Duotone at 100%. Each, with linear blending on, against the same
project in Standard: every case within 1 CV for 100% of channel values (max 1 CV).
Also: a blur across a hard black and white edge averages E values (midpoint 0.5, not
0.735) and a 50% white-over-black composite in the same renderer is 188 with linear
blending on and 128 off (the control that linear light is in force).

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

How the instability arises (2026-10-18). Posterize computes `floor(x * n +
.5) / n` per channel: a value within rounding of a level boundary lands on
either level, a full step apart (255 / (n − 1) CV: 16, 27, and 85 CV at the
corpus's 16, 9.5, and 3 levels). Levels clamps, then raises to a steep
`pow`, so a rounding-level input change near the clamp is amplified past a
code value; Color's hue rotation is unstable at saturated extremes. Today's
WebGL output itself moves by that much when its input moves one binary16
step. In the app the engine's input to these effects (the layer buffer:
pm-render's sampling and compositing) equals today's within 1 CV everywhere
but not bit for bit, so isolated pixels whose input sits on a boundary land
on the other level. They are not drawn "as today" bit for bit, and under
amendment 22 they are a visible difference on existing projects: escalated
with side-by-sides (`PM_RENDER_DIFFS` now also writes review frames:
engine, legacy, and a map of the differing pixels). Measured 2026-10-18 on
`effects-20261017-060101-ca4d364f`: `render/fx-posterize` t = 0, 1, 2 differ
at 0.10%, 0.05%, 0.02% of pixels, max 16, 27, 85 CV (float 15.9, 26.9,
85.0 CV). Matching them requires a layer buffer bit-identical to WebGL's
for every input that reaches these effects; not attempted here (Gaps).

Rerun 2026-10-18 after the review (same capture, `PM_RENDER_ONLY`, debug
build: the release corpus build fails to link under kache with "failed to
get bitcode from object file for LTO"): `render/fx-color` t = 0, 1, 2 at
98.12%, 97.80%, 97.50% exact, 100% within 1 CV, max 1 CV (float 0.50,
0.75, 0.75 CV); `render/fx-levels` at 98.35%, 97.24%, 98.49% exact, 100%
within 1 CV, max 1 CV (float 0.25, 0.25, 0.37 CV); `render/fx-posterize`
unchanged (99.90%, 99.95%, 99.98% exact, max 16, 27, 85 CV). Color and
Levels render as today within the drawing class's 1 CV at the corpus's
parameters; at parameters where they are step functions they have
Posterize's exposure. Side-by-sides of Posterize: `PM_RENDER_DIFFS`
(review copies outside the repository).

**Resolved 2026-10-19: Posterize, Color, and Levels render as today, bit
for bit.** Two causes, both now removed:
- *Input.* The layer buffer feeding them was within 1 CV of WebGL's but not
  bit-identical: today's image layers sample Blink's 8-bit premultiplied
  bitmap as an `RGBA8` texture, and the GPU's bilinear filter weighs 8-bit
  texels differently from half-float ones. The compositing lane now uploads
  and samples Level 1 stills as `Rgba8Unorm` (`docs/engine/composite.md`,
  merged into this branch). The differing Posterize pixels were all inside
  the rotated, resampled photo.
- *Arithmetic.* naga writes GLSL's `pow` as `metal::pow`, ANGLE as
  `metal::powr`; under Metal's fast math they round differently, so Levels'
  continuous path (`pow(x, 1/gamma)`, at gamma 1 too) differed by one
  binary16 step in about 10% of pixels. Legacy programs now compile from
  MSL the engine writes with ANGLE's calls (`pm_render::glsl`, below).

Measured 2026-10-19 (M4 Max, debug build):
- App level, capture `effects-20261017-060101-ca4d364f` (`tests/corpus.rs`,
  `PM_RENDER_ONLY=render/fx-`): `render/fx-posterize`, `render/fx-color`,
  and `render/fx-levels` at t = 0, 1, 2: the transparent float frames
  equal today's exactly (max 0.0000 CV; before: Posterize 15.9, 26.9,
  85.0 CV), the opaque 8-bit frames 100.00% exact (rounded) with max 1 CV,
  as every effect project's opaque frame. 63 frames compared, 0 failed, 0
  listed apart; `CORPUS_UNSTABLE` is now empty, so a departing frame fails.
- Shader level with bit-identical inputs (`effects_webgl`,
  `PM_SHADER_STATS=1`, `glsl-20261018-193347`), every variant including the
  step-function parameters that move WebGL itself under a one-step input
  perturbation (Color `max`; Levels `alt`, `max`, `random2`, `coerced`,
  `viewport-alt`, `hdr-alt`; Posterize `alt`, `max`, `random2`, `coerced`,
  `hdr`, `hdr-alt`): Color 11 of 11, Levels 11 of 11 (8 of 11 before
  `powr`), Posterize 11 of 11 bit-exact. Every built-in effect and
  transition variant is now bit-exact (Gradient's `defaults` and `hdr` were
  2 px off by one binary16 step before).
- App level at the step parameters themselves: `render/fx-steps-color`,
  `-levels`, `-posterize` (`render-effects.ts`: the variants above on
  rotated, stretched photos and the alpha image, plus Levels at gamma 1 and
  a steep 45/55/0.2, Posterize at 2 and 3 levels), captured through the app
  with every other effect and transition project as
  `effects-20261019-055047-8d8a4ce4` (93 samples): `tests/corpus.rs`
  compares 348 frames with the newest captures; all 93 effect and
  transition float frames equal today's (max 0.0000 CV), the opaque frames
  100.00% exact (rounded) with max 1 CV. The 5 failures of that run are
  video-class frames outside this lane (`motion/blur-video`,
  `retime/speed`, `space25d/precomps`), identical with this lane's changes
  removed.

The Store `mrsigma/liquid-glass-ios27` 2.0.8 glass effect is the shader
gate's WebGL-unstable item (`docs/engine/shaders.md`): there both renderers
get bit-identical inputs, so the difference is the arithmetic. ANGLE and
naga each translate the GLSL to their own MSL, and Metal compiles both
(wgpu with Metal's default fast math, which permits reassociation and
contraction that depend on the exact MSL text). Two translations of the same
program then round differently, and where the program is discontinuous (a
threshold or branch on a computed value in its highlight) a rounding
difference flips the side; WebGL itself moves by up to 67 CV there when its
input moves one binary16 step.

Fast math is not the cause (checked 2026-10-19): with wgpu-hal patched to
compile every Metal library with fast math off (a temporary
`[patch.crates-io]` copy outside the repository, separate target folder,
reverted after), the shader gate on `shaders-20261018-022611` got further
from WebGL, not closer: defaults 48 px over 1 CV (max 99 CV), coerced 49
(max 72), hdr 53 (max 45), and the alt variant, which matches with fast
math, failed at 2 px (max 10 CV); with fast math on (production) the
counts are 43, 45, and 49 px and alt matches. So ANGLE's MSL behaves like
fast-math MSL for this program, and the residual is the two translations'
different MSL text. Matching it bit for bit would need the
engine to emit ANGLE's MSL for that program; running it in the fallback
runner would match the arithmetic but not in-app inputs (the layer buffer is
within 1 CV, not bit-identical) and would read the frame back every frame.

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

## Shared files this lane changed

For the owners of these files (derouter, device, bridge, dochost, jobs):

- **pm-render outside `effects/`, `glsl/`, `shader_layers/`**: `renderer.rs`
  (the fallback host loop in `render` and `render_into`, `set_fallback_host`,
  `put_fallback_pixels`, `put_fallback_failure` returning the retry delay,
  `fallback_retry_at`, `fallback_textures`, `watchdog_watch`,
  `watchdog_holding`, `wait_idle`, `watchdog_reports`), `capability.rs`
  (`unsupported_for`), `error.rs` (three `RenderError` variants:
  `WatchdogPending`, `Fallback(String)`, `Watchdog(String)`; outside
  pm-render they reach generic error arms, and pm-node's presenter handles
  `WatchdogPending` itself), `exec.rs`, `lib.rs` (`pub mod glsl`), and
  `tests/corpus.rs` (the review side-by-side dump under `PM_RENDER_DIFFS`,
  now for every failing frame; the `CORPUS_UNSTABLE` machinery is removed).
  2026-10-19: `renderer.rs` also pins a job frame's runner results and
  replaces the 16-round limit with the progress rule (`host_progress`); in
  the second review it waits for a program still compiling
  (`HOST_COMPILE_WAITS`, exported from `lib.rs`). Merging the bridge and
  perf2 lanes: `renderer.rs` keeps the job watchdog stop ahead of
  `plan::build_blender`, `lib.rs` lists `frame_prefix` and `glsl`.
- **pm-engine**: `engine.rs`, `jobs.rs` (`job_definitions`;
  `JobService::definitions_pushed` defers the push's acknowledgement until
  the job engine holds the set).
- **pm-protocol**: `envelope.rs` (`Request::ShaderFallback`,
  `Event::ShaderFallback` with `Input`), `samples.rs`, `shader_fallback.rs`
  (second review: `MAX_EDGE` 16,384), wire fixtures; the generated
  `shared/engine-protocol/` types. Merging the bridge lane, `ShaderFallback`
  became the last request variant (index 28, sample id 29), after the
  bridge's `Snapshot` and `CompatFrame`, which keep their indices.
- **pm-glsl**: `pipeline.rs` (`PIPELINE_REVISION` 2), `cache.rs`
  (`Key::at_revision` and the revision tests), `normalize.rs` (`pm_round`).
- **RFCs**: `rfcs/07-errors-and-isolation.md` and
  `rfcs/10-definition-snapshots.md`: a definitions failure report's
  `FailureReport.revision` carries the definitions generation it answers
  (the field is overloaded: for document failures it is the document
  revision).
- **Kernel**: `kernel/definition-snapshots.ts` (`changedAt` and the latest
  generation per entry, so a push is acknowledged against the generation it
  carried).
- **pm-node**: `present/render.rs` (texture bands, failures without
  routing, transient retry wake-ups; second review: a snapshot's texture
  bands to main ahead of `needsRenderer`, `fallback_inputs`),
  `jobs/{fallback,definitions,compile,mod}.rs`, `tests/document.rs`,
  `tests/snapshot.rs` (bridge's: one child test).
- **pm-export** `render.rs`, **pm-cli** `main.rs`, `render.rs`,
  `render_parity.rs` (second review: the WebGL-unstable review class and
  its fx stripping are removed; a departing frame fails).
- **Corpus**: `engine/corpus/src/projects/render-effects.ts` (2026-10-19:
  `render/fx-steps-{color,levels,posterize}`, the effects at their step
  parameters).
- **Desktop**: `preload/index.ts` (bridge's: the `onShaderFallback`
  listener and its acknowledgement), `shared/engine-ipc.ts`,
  `main/engine/{index,native-engine,protocol,definitions,agent-jobs,render-queue}.ts`,
  `main/agent-tools/{bridge,spec}.ts`, `engine-connection/{install,session}.ts`,
  `legacy/assistant/harness.ts` and `legacy/core/exporter.ts` (a
  `PM.EngineDefinitions.settle()` call before a render or export),
  `main/engine/shader-fallback-relay.ts` (2026-10-19: `tooLarge` as
  `budgetExceeded`). Second review, with the bridge lane's render
  transport merged: `main/engine/fallback-inputs.ts` (new),
  `native-engine.ts` (the interactive engine's texture bands kept, ahead of
  the session-event filter; `fallbackTextures`), `snapshots.ts` (bridge's:
  `fallbackTextures` on `needsRenderer`), `shared/engine-ipc.ts` (the
  reply's `fallbackTextures`), `engine-connection/render-transport.ts`
  (bridge's: textures to `render`/`renderSync`), `install.ts` (the job
  runner hook and `setRoute` re-applied to the bridge's `openSession`),
  `session.ts` (the bridge's `uploadFallback` for runner results), and
  `pm-render/tools/shaders/collect.ts` (a local extension that does not
  load is noted, not fatal).
  `kernel/api.ts`, `main/index.ts`, `legacy/core/editing.ts`, and
  `legacy/gl/raster.ts` are untouched.

## Commands

```sh
bun engine/crates/pm-render/tools/effects/builtins.ts            # rewrite builtins.json
bun engine/crates/pm-render/tools/effects/collect.ts             # shader corpus → engine corpus folder
~/Library/Caches/powermove-engine/bin/pm-electron-lock \
  bun engine/crates/pm-render/tools/effects/webgl.ts [--perturb-input]
bun engine/crates/pm-render/tools/effects/expectations.ts        # probe verdict fixture
cargo test -p pm-render --test effects_webgl -- --nocapture      # shader parity
PM_SHADER_STATS=1 cargo test -p pm-render --test effects_webgl -- --nocapture  # bit-exact counts per variant
cargo test -p pm-render --release --test effects_linear -- --nocapture  # linear blending
bun engine/corpus/src/capture.ts --only render/fx-,render/tr- --label effects --skip-real --skip-scenarios
cargo test -p pm-render --release --test corpus -- --nocapture   # app-level parity
cargo run --release -p pm-glsl --bin pm-glsl-fuzz -- --seeds <seeds.json> --iterations 3000
cargo test -p pm-render --release --test effects builtin_costs -- --ignored --nocapture
cargo test -p pm-render --test watchdog --test effects                    # watchdog (test hook)
cargo test -p pm-node --test definitions --test jobs                      # views, jobs, compile
cargo test -p pm-cli --test definitions                                   # pm-cli and snapshots
PM_RENDER_ONLY=render/fx-posterize PM_RENDER_DIFFS=<dir outside the repo> \
  cargo test -p pm-render --test corpus -- --nocapture                    # review side-by-sides
```

## Gaps

- **The per-program budget records only** (2026-10-19): plan section 8 and
  issue A2 ask for a budget that turns an overrunning program off, but
  today's compositor (`legacy/gl/gpu-timing.ts`) only records GPU time, and
  amendment 22 forbids the visible difference. The engine now records
  (`Watchdog::measured_ms`) and enforces nothing; the hang guard's 10 s
  (Chromium's) is the only thing that turns a program off. Enforcing a
  budget (`Watchdog::set_budget`, user programs only) needs Jude's call
  before any renderer sets it.
- **Uniform metadata (`udefs`) is not written by the engine**: descoped
  from this lane by the orchestrator (decision DEFS-1, 2026-10-19; the
  Inspector stays the only writer, exactly as today), by design:
  today's compositor never writes `shaderMeta.udefs`; the built-in
  Inspector extension does (`extensions/inspector/shader.ts:43`,
  `syncShaderUniforms`, from its own `parseUniforms`), and section 11 lists
  the meta the engine fills (`shaderKey`, `missing`, `missingAsset`,
  `sceneWarnings`, `sceneError`) without it. An engine write would clobber a
  forked Inspector's own `udefs` and would change the Shader panel's "N
  uniforms" for a user who disabled the Inspector (today 0), both visible
  differences. The engine binds uniforms from its own exact port of
  `parseUniforms` (`shader_layers::uniforms`, `parse_uniforms_matches_the_app`).
  `sceneWarnings` and `sceneError` for 3D scene layers are the 3D lane's.
- **Fallback runner throughput is below today's** (measured 2026-10-19, M4
  Max, needs Jude's call; the ANGLE-only listing adds the Store
  liquid-glass-ios27 2.0.8 effect to the programs that pay it). Each fallback step reads its textures back and
  sends them to the renderer and the pixels back (binary16, about 16 MB per
  full-HD texture each way); results are cached by content key (512 MB), so
  a still frame costs one round trip, but an animated fallback effect does
  one per frame. Two costs were fixed: the engine keyed the request with
  SHA-256 one value at a time (139 ms of engine time per full-HD frame with
  two textures, now 20 ms: BLAKE3 in parts on up to four threads;
  `fallback_step_cost_at_full_hd`, release, the native Invert 1.0 ms), and
  the runner converted its float readback to binary16 one value at a time
  with an allocation per value (2.7 s per full-HD request, now 25 ms: it
  reads RGBA16F as half floats; headless Chromium, ANGLE Metal). Exporting
  a 2 s, 30 fps, 1920 × 1080 project whose only effect is a sampler3D
  effect only the runner draws took 7.0 and 7.2 s through the job engine
  (the same project with the built-in Invert: 1.1 and 1.6 s); today's
  exporter takes 1.1 s for both. About 100 ms per frame remain in the job
  path (the scratch files, main's relay, the IPC copies to and from the
  window); the viewer's path (texture bands over the session's port) was
  not timed in the app, and an animated fallback effect at full HD there
  is expected to play well below today's frame rate. Pixels are identical;
  the time is the difference.
- **Translation in a helper process** not built: a crash inside glslang or naga takes
  the engine process (the fuzz runs found none; limits stop every bomb found); a hang
  costs one leaked thread past the deadline. The job engine's compile worker
  translates on its own thread, so a translation past the 20 s deadline
  delays later acknowledgments, not rendering.
- **Device-loss recovery** for a true GPU hang is the device owner's (pm-node, pm-export)
  and not built here; the hang guard makes sure the program that hung is not
  submitted again once the system's GPU timeout ends it.
- **Collect Project and `.pmv` files do not embed definitions yet** (the save
  lane's packer): `pm-cli` takes them with `--definitions` or uses the
  built-ins; a project made with a fork of a built-in renders the built-in's
  GLSL without the flag.
- **The engine process** (`pm-engine`) applies `pushDefinitions` to an
  interactive session's `DefinitionSet` without translating, so that
  `definitionsApplied` carries no failures; the presenter translates and
  sends each failure as a `failure` event (definition scope, at its
  definitions generation). Job sessions are acknowledged by the compile
  worker with real failures.
- **Linear blending with an alpha above 1:** in Standard such a pixel darkens the stack
  beneath by the excess (WebGL's arithmetic, kept); with linear blending on the
  composite is in linear light and there is no reference for it.
- **WebGL-unstable class: no longer an allowance anywhere** (2026-10-19,
  second review). The shader gate fails a variant that differs from WebGL
  whether or not WebGL moves against its own perturbed runs (the
  perturbation result stays in the failure as the explanation, with the
  side-by-side under `PM_SHADER_GATE_DIFFS`; `shader_gate`
  `a_difference_webgl_is_unstable_under_fails_the_gate`); before, such a
  variant was counted apart and passed, the mechanism that hid
  liquid-glass 2.0.8. `tests/corpus.rs` and `pm-cli parity` lost their
  (empty) listing machinery: a departing frame fails. The three built-ins
  exposed to rounding drift moved from `pm_render::effects::WEBGL_UNSTABLE`
  to a fixture in `effects_webgl` (`EXPOSED_BUILTINS`): the perturbation
  test fails when a built-in outside it turns out ill-conditioned.
- **WebGL-unstable class: resolved for level 1 content** (2026-10-19).
  Posterize, Color, and Levels are bit-exact against today (Parity); the
  Store liquid-glass-ios27 2.0.8 glass effect draws through WebGL's runner
  in the app (ANGLE-only programs, above). What remains: `pm-cli` draws
  that program natively (no WebGL runner there, and no "today" to match);
  a user's own program with the same ill-conditioning (a decision on a
  texture sample at computed coordinates) that is not in the Store and not
  listed would draw natively and could differ at isolated pixels. The
  listing is by fingerprint, so it is extended as the gate finds programs;
  nothing detects such a program on its own.
- **ANGLE parity residue** (shader level, within 1 CV): `radians`/`degrees`
  probes (6 of 36,864 pixels at 1 CV even with ANGLE's operand order),
  `halftone-fragcoord`, `outer-product-twist`, `mix-bool`,
  `matrixCompMult`, `outerProduct`, `modf-infinity`; none of them in a
  built-in or a Store or corpus program that differs at app level.
- **Not run**: the full pm-render suite (targeted tests only; clippy is
  clean on pm-render, pm-glsl, pm-cli, and pm-node).
- The generated corpus was written by an agent from the docs, not produced through the
  in-app agent harness (S3 provenance gap); the Store's 4 public releases (figuate
  chrome 1.0.0, zellzoi ascii-magic-effects 1.0.0 and liquid-glass-ios27 2.0.1, mrsigma
  liquid-glass-ios27 2.0.8; 5 effects) are not in this corpus but in the shader gate's
  (`docs/engine/shaders.md`; their sources stay outside the repository); one machine
  and one GPU family.
- WebGL's "Expression too complex" rule and ternaries on structs and arrays are not
  reproduced (S3).
