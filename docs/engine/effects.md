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
| Program WebGL draws, engine cannot translate | drawn | routed (`Feature::ShaderFallback`), `NotHeadlessRenderable` reported |
| Watchdog overrun | n/a | skipped from the next frame, `WatchdogTimeout` reported |
| Transition registered | drawn | drawn |
| Transition unknown / missing / program refused | — / no transition / `withLayer` | routed / no transition / no transition |
| Effects on adjustment layers | `compositeAdjustment` | drawn (the compositor's adjustment node with the real chain); the same per-effect rules |

`capability::unsupported_with_definitions` gives the same answer statically for routing
consumers; `unsupported` and `unsupported_with_media` are unchanged (they list effects,
transitions, and adjustment layers with effects). The interactive engine's presenter
(pm-node `present/document.rs`) routes with `unsupported_with_media` and, once the
session has definitions, ignores those three features; a definition it cannot draw
fails the frame as unsupported and routes then. Built-ins: `pm_render::builtin_definitions()` (the 16 effects and 4
transitions as the kernel snapshots them; a vitest fails when `builtins.json` drifts
from the kernel; `bun engine/crates/pm-render/tools/effects/builtins.ts` rewrites it).

## The watchdog

Issue A2 (2026-10-18) rebuilt it around two guards, judged when a frame's
commands have completed (nothing waits during rendering: `Watchdog::collect`
runs at the start of later frames), and a deterministic test hook.

- **The budget** (interactive renderers): each effect instance, transition,
  shader layer, and extension fragment layer of a timed frame is timed
  between two marker dispatches (one-workgroup compute passes whose
  end-of-pass sample is the timestamp, as `pm-export`'s `GpuSpan`: the first
  reads a texel of the program's input, the second of its output). The
  timestamps are resolved in a submission of their own once the frame's
  command buffer has completed (resolved in the frame's own command buffer,
  Metal returned the previous frame's end sample; found 2026-10-17). A
  program over the budget (default 250 ms per program per frame) is
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
  timed on the GPU): every frame drawing a program is followed from its
  submission (`Queue::on_submitted_work_done` records the completion time).
  A frame whose commands have not completed within the hang budget of wall
  time (2 s interactive; 10 s for jobs, Chromium's own GPU watchdog, so an
  export that finishes today finishes here) is a runaway: every program it
  drew that is not a built-in (`DefinitionOrigin::BuiltIn`) is disabled at
  once and reported, so a runaway user loop is never submitted again; it is
  judged while still running, not when (if ever) it completes. Built-ins
  cannot run away and are kept. A true hang ends when the operating
  system's GPU timeout aborts the command buffer; device-loss recovery stays
  the device owner's (Gaps).
- **Trusted samples.** A timestamp sample is judged only when both samples
  are after the newest trusted sample, it ends after it begins, and it is no
  longer than the frame's own submission-to-completion wall time (+50 ms);
  only trusted samples raise that floor (`watchdog::plausible_spans`, unit
  tested). Before, one garbage end sample raised the floor past every later
  sample, after which the watchdog judged nothing for the rest of the
  process. Metal leaving a sample unwritten (seen with eleven test
  renderers busy in one process: 4 of 10 spans) still leaves that frame
  unjudged; the hang guard does not depend on samples.
- **Policies.** `Policy::Interactive` (the viewer): the budget and the hang
  guard, a disabled program is skipped. `Policy::Job` (export jobs and agent
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
  `the_watchdog_disables_an_overrunning_effect_from_the_next_frame`,
  `tests/watchdog.rs` (shader layers, the hang guard with user and built-in
  programs, the job policy, a frame judged while still running),
  `pm-node/tests/definitions.rs`
  `a_disabled_effect_stays_off_across_unrelated_pushes_and_other_views`.
  Production renderers never set a source. The GPU path keeps its own test,
  `the_watchdog_judges_every_timed_frame_not_only_the_first`.

Measured 2026-10-18 (M4 Max): `tests/watchdog.rs` 5 of 5 tests passing in
each of 10 runs; `pm-node/tests/definitions.rs` passing in 20 of 20 runs
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
folder.

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
`e2e/engine-definitions-export.spec.ts` (written, not run: Gaps).

**Compile errors are the engine's** (2026-10-18). With the new engine on,
`GL.compileError` answers every engine key from engine reports only: `fx:`,
`tr:`, `extension:`, `engine:`, and a shader layer's legacy `sh:<layer>:<hash>`
key (read as that layer's `engine:` report). A program the engine compiles
shows no error, and an error the legacy compositor recorded before for the
same key is never shown again (before, `?? legacy(key)` resurrected it).
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

- **Fallback for effects and transitions** (decision 19 with texture inputs)
  is not built: a definition only the runner could draw is still
  `Feature::ShaderFallback` (the interactive engine routes it; jobs and
  `pm-cli` name it as not headless-renderable). What it needs: an effect
  pass node that plans a `FallbackRequest` carrying its input and backdrop
  (read back from the frame's targets, keyed by the program, the calls, and
  the inputs' content), the runner contract extended with input textures
  (`u_tex`, `u_orig`, `u_backdrop`, `u_from`, `u_to`) and 1 to 8 ping-pong
  passes, the pixels composed back into the chain; for desktop job exports a
  host-services request that asks a window's runner (contract 14), so the
  job waits for it like the interactive presenter does. Shader layers have
  the runner today (`docs/engine/shaders.md`).
- **The fallback runner's own draw deadline**: the runner relies on WebGL
  context loss (Chromium's GPU watchdog) to stop a runaway program, and a
  program that lost the context is not run again; there is no shorter
  deadline of its own and no `WatchdogTimeout` report for it.
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
- **WebGL-unstable class**: Posterize (and at other parameters Levels and
  Color) differ from today at isolated pixels in the app (above); escalated
  to Jude with side-by-sides. Not matched.
- **Not run**: `e2e/engine-definitions-export.spec.ts` (needs the app and a
  release engine build; free disk was below the 40 GB limit when this lane
  ran, 24 to 38 GB, so no release build was started), the full pm-render
  suite.
- The generated corpus was written by an agent from the docs, not produced through the
  in-app agent harness (S3 provenance gap); Store has 3 public extensions, not in this
  corpus (their sources stay outside the repository); one machine and one GPU family.
- WebGL's "Expression too complex" rule and ternaries on structs and arrays are not
  reproduced (S3).
