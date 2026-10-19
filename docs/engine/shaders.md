# Shader layers, extension layers, and the shader gate

Plan section 10 (the `shader`, extension fragment, and extension mesh rows),
section 11 (all of it: the shader-layer and extension-fragment rows of the GLSL
contract, `@param` and `UIState.shaderMeta.udefs`, the five presets, the
fallback runner of decision 19, the gate), section 20 (the `raster` override
path the runner's upload follows), section 25, and amendment 9. Builds on
[effects.md](effects.md) (the production GLSL pipeline, definition snapshots)
and [scene3d.md](scene3d.md) (the legacy OBJ mesh shading). The plan stays the
authority.

Code:

- `engine/crates/pm-render/src/shader_layers/`: `uniforms` (an exact port of
  `PM.parseUniforms`), `plan` (what a shader or extension fragment layer draws
  at a frame), `exec` (the draw), `translate` (the process-wide translation
  cache for shader-layer code), `fallback` (the engine side of the runner),
  `builtins.json` (the app's shader-layer runtime, embedded).
- `engine/crates/pm-protocol/src/shader_fallback.rs`: the runner's messages
  (`Event::ShaderFallback`, contract 10).
- `engine/crates/pm-node/src/present/` (`render.rs`, `mod.rs`, `document.rs`)
  and `pm-engine/src/{engine,present}.rs`: the interactive presenter's side;
  `pm-node/src/present/layer_reports.rs` and `shader_layers/reports.rs`: the
  Shader panel's layer reports.
- `apps/desktop/src/renderer/src/engine-connection/shader-fallback.ts` (the
  runner), `session.ts`, `install.ts`.
- `engine/crates/pm-render/tools/shaders/`: the gate corpus and its tools;
  `tests/shader_gate.rs`, `tests/shader_layers.rs`.

## Shader layers

`compositor.ts:1015-1052`, reproduced in `shader_layers::plan::shader_layer`:

| Step | Today | Engine |
|---|---|---|
| Program | `PM.SHADER_HEADER + '\n' + d.code` (`d` from `resolveContent`), linked with `PM.VERT` | The same string, through `pm_glsl` (cached by source; disk with the shader cache folder). A missing `d.code` is the text `undefined` and fails to compile, as today |
| Target | `w = max(1, Number(d.w ‖ comp.w ‖ W))`, `targetW = min(4096, max(2, round(w · sx)))` (both axes), RGBA16F cleared to transparent, blending off | Same (the level 1 cap), working format (RGBA16F, or RGBA32F at 32-bit precision), GL row order |
| Base uniforms | `u_m`, `u_res`, `u_uv` (vertex); `iResolution` (target), `iTime = T − L.from`, `iGlobalTime = T`, `iProgress = clamp((T − L.from) / max(L.dur, 1e-4), 0, 1)`, `iFrame = uniform1i(round(T · PM.proj.fps))` (the open composition's rate), `iMouse = (0, 0)` | Same calls in the same order; `uniform1i` converts as WebIDL `long` |
| User uniforms | `for (un in d.uniforms)`: the `@param` definition of that name decides the call: color `uniform3f(hex2rgb(String(v)))`, toggle `uniform1i(v ? 1 : 0)`, anything else `uniform1f(Number(v) ‖ 0)` | Same, in `for-in` order after the base calls (a stored `iTime` overrides, as today) |
| GL type rules | A single float never sets a `vec2`, `vec4`, `int`, or a `vec3` without a `#` default: they stay 0 (`compositor.ts:1024-1030`); `bool` takes 1f or 1i | `effects::program::apply` (GL's rules, shared with effects) |
| Draw | `{tex, w, h, ax: 0, ay: 0, uv: [0,0,1,1], fromFbo: true}` through the layer's transform | `Source::Shader`, drawn as a `fromFbo` texture quad; E-encoded legacy output (`Resolved::encoded`) |
| Compile failure | `program()` returns null: nothing drawn, `GL.errors` | Nothing drawn, a `LayerFailure` with the diagnostics (author lines) |

`PM.parseUniforms` is ported exactly (`uniforms.rs`): the regular expression
with JavaScript's `\s` (U+FEFF in, U+0085 out) and ASCII `\w`, `\s*` after the
`;` crossing lines (an annotation on the next line belongs to the uniform),
no word boundary before `uniform`, the reserved `i*` names skipped, the label
transform, and `parseFloat` (longest decimal prefix, `Infinity`, `NaN` →
default 0 with `max` left `NaN` as `Math.max(1, NaN)` gives). The compositor
reads the definitions from the code (`uniformDefsFor`), not from
`UIState.shaderMeta.udefs`, which holds the same list for the Inspector; the
engine does the same. Held to the app's own function on 43 sources (the
presets, every generated shader layer, 14 edge cases):
`tests/shader_layers.rs` `parse_uniforms_matches_the_app`, fixture from
`tools/shaders/parse-uniforms-fixture.ts`.

The five presets (`PM.SHADER_PRESETS`, with Aurora Field equal to
`PM.SHADER_TEMPLATE`) are user GLSL copied into `d.code`; the engine never
reads them while rendering. `builtins.json` embeds them, the header, and the
missing-layer checkerboard for the gate and tests; the vitest
`kernel/shader-layer-builtins.test.ts` fails when it drifts
(`bun engine/crates/pm-render/tools/shaders/builtins.ts` rewrites it).

Texture units: nothing is bound for a shader layer's draw, so a sampler a
shader layer declares reads (0, 0, 0, 1). In today's compositor the unit
holds whatever the previous draw left bound (state the shader cannot rely on);
the gate's WebGL reference binds nothing, as the engine.

## Extension fragment layers

`compositor.ts:1053-1127` for `renderer.kind === 'fragment'`
(`shader_layers::plan::extension_layer`): the kernel's wrapped program from the
definition snapshot (`extensionLayerFragment`: header, `uniform vec3`/`bool`/
`float u_<k>` per parameter, the fragment), the same target and base uniforms,
then per parameter `d.params[k]` evaluated when truthy, else the snapshot's
default: colors `uniform3f`, toggles `uniform1i` on the `uniform bool`,
numbers `uniform1f`. The program is the definition set's (`Effects`, kind
`layer`, translated when the set changes).

| Case | Today | Engine |
|---|---|---|
| Fragment definition, program ready | drawn | drawn |
| Program WebGL refuses | nothing drawn | nothing drawn, a layer failure |
| Program only the runner draws | drawn | the fallback runner (below) |
| Definition marked missing (contract 10) | the checkerboard | the checkerboard (the header and `compositor.ts`'s `missing` body) |
| No snapshot, not marked missing | n/a | routed (`Feature::Definition("layer <id>")`) |
| Renderer has no definitions | n/a | routed (`Feature::ExtensionDefinition`), as before |
| Mesh, legacy scene, 3D roles | their paths | their paths (`crate::scene3d`; the ids it owns are never intercepted) |

Studio Cube (`powermove.3d.studio-cube`, a raymarcher with `uniform bool
u_autoRotate`) and the OBJ model layer are in `builtins.json` as the kernel
snapshots them; `pm_render::all_builtin_definitions()` is the 16 effects, 4
transitions, and these two, for hosts without a kernel (tests, `pm-cli`). In
the app every layer definition arrives with the kernel's push.

The interactive presenter used to route every composition with an extension
layer that was not a 3D role (`unsupported_with_media` lists
`ExtensionDefinition`); with the session's definitions it now skips that
feature, as it skips effects, and a layer the renderer cannot draw fails the
frame as unsupported and routes then (`pm-node/src/present/document.rs`).

## Extension mesh layers

The fixed WebGL shading of `meshExtensionQuad` is the 3D lane's
(`scene3d/legacy.rs`, 4 cases against the compositor's shader, the corpus
`layers/obj-model` exact). This lane feeds it third-party mesh definitions:
`Renderer::set_definitions` and `apply_definitions` now refresh the 3D pass's
mesh definitions from the set's `renderer.kind === 'mesh'` snapshots
(`assetField`, parameter defaults), on top of the built-in OBJ layer and
`set_mesh_definitions`. Picking: today's `GL.pick` picks mesh layers (whose
default background is truthy), shader layers, and fragment layers by their
rectangle; pm-eval's port does (`shader_extension_and_obj_layers_pick_by_their_rectangle`).

## The fallback runner (decision 19)

Trigger (S3.md, "The runtime fallback"): a program WebGL links (it passed
`pm_glsl`'s limits and ES validation) that a later stage refuses: glslang, a
SPIR-V rewrite, the bind check (amendment 9: samplers other than a float
`sampler2D`), naga, or Metal pipeline creation (recorded with
`translate::mark_failed`, so later plans route it).

| Host | What happens |
|---|---|
| Interactive engine with a viewer (`set_shader_fallback_runner(true)`, the presenter's composer) | The layer plans as a `FallbackRequest` (vertex, wrapped fragment, target, uniform calls, the failing stage). The frame fails with `RenderError::ShaderFallbackPending`; the presenter sends `Event::ShaderFallback(Requests)` to the view's renderer once per request and redraws when the pixels arrive. A program the device refuses at pipeline creation, on first contact (not known when the frame was planned), asks for the same request from that frame instead of routing. The layer is reported (below) and logged once per layer and session. A runner failure is reported and the layer draws nothing (`compile`: WebGL refuses it too, as today) or is turned off by the watchdog (`watchdog`); the composition is never routed (amendment 21, review fix 2026-10-18) |
| Job engine (desktop exports, agent stills) | The lane's `FallbackHost` (pm-node `jobs/fallback.rs`, contract 14) asks main to run the request in a window's runner (the job's owner, else any window with the new engine), with the textures and the result as files in the session's scratch area; the job waits for it and renders the frame again. `compile` skips the program as today's exporter does; `watchdogTimeout` stops the job; no window fails the job naming the layer |
| A host with no window open (the hidden host) | The relay has no runner to ask: the job fails naming the program (`shader-fallback-relay.test.ts`, "fails named when none is open"; `tests/effects_fallback.rs` `a_job_without_a_runner_fails_naming_the_program_instead_of_skipping_it`) |
| `pm-cli render` | No runner: exits 4, "not headless-renderable", naming each layer and definition, before anything is written; the same on every run (`pm-cli/tests/definitions.rs`) |

The runner (`ShaderFallbackRunner`): one WebGL2 context on an
`OffscreenCanvas` for the window, `EXT_color_buffer_float`, an RGBA16F target
cleared to transparent with blending off, the quad through `PM.VERT`, the
calls replayed with the GL entry point each names (GL's own type rules drop
the mismatched ones, as in the compositor), no texture bound, `readPixels`
as binary16 in GL row order. Compiles go through `KHR_parallel_shader_compile`
with a deadline (10 s); a lost context is recreated, and a program that lost it
is not run again. Programs are cached (32), requests run one at a time. Every
pass's draw is fenced and polled under a 10 s draw deadline (the runner's
watchdog, review fix 2026-10-18): a draw past it loses the context on purpose
so the renderer is never blocked in `readPixels`, fails with `watchdog`, and
the program is never run again until its source changes; the engine reports
`WatchdogTimeout` and turns it off (a job stops). Effects and transitions use
the same runner with their textures (effects.md, "The fallback runner").

Upload: the session sends the pixels in row bands of at most 8 MiB
(`ShaderFallbackMessage::Pixels`; messages are limited to 16 MiB), or
`Failed {reason}`; the presenter assembles bands (`shader_layers::Bands`) into
the renderer's `FallbackStore` (1 GiB, least recently used first) keyed by the
request's hash, and the layer draws them like any other `fromFbo` target, as
the `raster` override's pixels would be. Their GPU copy is kept by request
key (8 textures, least recently drawn first; `ShaderState::fallback_uploads`
counts the copies), so a fallback layer that does not change, a still frame
or a static layer during playback, is uploaded once, not on every render.
Pixels are per frame (the key covers every call, so an animated program asks
once per frame shown); the cost is a WebGL draw plus a readback and a copy
per frame, which is acceptable for a path that exists for programs nothing
else can draw. Bands that arrived for a session that closes mid-transfer are
dropped with it.

Reported and marked (plan section 11: such a program "is reported and
marked not headless-renderable"): the static check lists
`Feature::ShaderFallback` (marked), and `shader_layers::reports` sends the
Shader panel a layer-scope `NotHeadlessRenderable` report worded as a
warning ("Shader layer sky is drawn by the WebGL fallback, so headless
exports cannot render it (the native pipeline stops at bind: ...)", with the
diagnostics) at every revision presented, as the compile errors are. The
presenter also logs it once per layer and session. Extension fragment
definitions only the runner draws are reported per definition
(`effects::definition_error`, `DefinitionsApplied`).

Evidence:

- `pm-node/tests/shader_fallback.rs` (the interactive engine in a child
  process, driven through the protocol): a `samplerCube` shader layer is not
  routed; the engine sends one request (stage `bind`, the 80 × 45 layer at the
  viewer's 4× scale: 320 × 180), presents no frame until both bands arrive out
  of order, then reports the layer (`notHeadlessRenderable`, layer scope, at
  the frame's revision, before the frame), logs it once, and presents the
  frame; a runner failure for the next program is reported, the layer draws
  nothing, and the frame is presented, not routed.
- `shader_layers::exec` unit tests: a program the device refuses at pipeline
  creation (injected) asks the runner in the same frame with a runner, is
  reported, and its pixels are uploaded once for three frames; without a
  runner it routes (`Feature::ShaderFallback`).
- `engine-connection/shader-fallback.test.ts`: the session answers with bands
  under the limit (2,048 × 1,100: 512 + 512 + 76 rows), reports failures and a
  missing runner; the call replay and the binary16 conversion.
- The gate: three variants of `generated-agent/env-cubemap-sky` (a
  `samplerCube` uniform) rendered through the app's runner in headless
  Chromium (`tools/shaders/fallback-runner.ts`), drawn by the engine, and
  compared with the compositor's WebGL output: within tolerance.
- `tests/shader_gate.rs` `layers_needing_the_runner_are_not_headless_renderable`
  (also: the layer report, and one upload for two frames).

## The shader gate

`tools/shaders/collect.ts` gathers every GLSL source with the compositor's
calls; `tools/effects/webgl.ts --corpus <dir>` replays them in WebGL2 (ANGLE
on Metal, Chromium 151.0.7922.34 headless); `tests/shader_gate.rs` renders
them:

- shader and extension fragment layers through `Renderer::render` on a
  document holding the layer (the plan's calls, the program, the target, the
  composite), compared with the WebGL target flipped to top-down;
- effects and transitions through the effects lane's production passes
  (`effect_step`/`transition_step`, `run_effects`/`run_transition`).

Built-in effects and transitions are the effects lane's corpus (176 + 52
variants, all matching, effects.md) and are not repeated here.

Sources and provenance:

| Group | Source |
|---|---|
| shader-presets | `PM.SHADER_PRESETS` (the template is Aurora Field) |
| builtin-layers | Studio Cube; the missing-layer checkerboard |
| docs | `apps/desktop/docs/samples`, `EXTENSIONS.md` code blocks |
| cloud-samples | `apps/cloud/samples` |
| e2e | e2e fixtures, `register(...)` literals and shader-layer code in e2e specs |
| unit-tests | GLSL literals in kernel, legacy, and panel unit tests |
| scenes | onboarding and www hero `scene.json` |
| store | every public Store release, fetched without credentials through the desktop Store client's read-only routes (`tools/shaders/fetch-store.ts`): 4 releases of 4 extensions on 2026-10-17 (ascii-magic-effects 1.0.0, chrome 1.0.0, `zellzoi/liquid-glass-ios27` 2.0.1 and `mrsigma/liquid-glass-ios27` 2.0.8, two extensions of the same name), all effects. Kept outside the repository; activated under `sandbox-exec` with network and file writes denied (`tools/shaders/activate.ts`) |
| generated-s3 | spike S3's 56 agent-style shaders (written by Opus 5.5 from the docs) |
| generated-agent | 37 prompts covering every contract feature (`tools/shaders/prompts.ts`, `CONTRACT`; the script fails when a feature has no prompt), answered by `claude --print` (Claude Code 2.1.300, `claude-opus-5-5`) with the in-app agent's own system prompt (`agentInstructions()`, the adapter's schema suffix), the API pack readable with Read/Glob/Grep, and the shader-layer runtime the Shader panel shows. The in-app harness itself needs a running Powermove with its MCP tools, which cannot run headlessly here; differences: no render_frames or validate_effect loop, the deliverable returned as JSON. Outputs with provenance in `tools/shaders/generated/` |
| local | opt-in (`--local <project.pmv>` or an extensions folder); not run for this report |

Run again 2026-10-18 on the definitions lane's tip (corpus
`shaders-20261018-022611`; routing and errors count as failures): the same
table, cell for cell.

Measured 2026-10-17 (M4 Max, macOS 27, wgpu 30.0.1 Metal; corpus
`shaders-20261017-073721`, 144 cases, 522 variants), and reproduced on
2026-10-18 after merging `feat/native-engine` at `f3bf2478` from a fresh
run of every step (the Store fetched again: the same 4 releases; corpus
`shaders-20261018-022611`; WebGL and perturbed WebGL captured again, the
3 runner requests answered again): the same table, cell for cell. Run
again on the same corpus after the runner's layer reports, the first-contact
pipeline path, and the upload cache landed (2026-10-18): the same table.

| Group | Cases | Variants | Match WebGL | via the runner | Both refuse | WebGL-unstable | Failures |
|---|---|---|---|---|---|---|---|
| builtin-layers | 2 | 6 | 6 | 0 | 0 | 0 | 0 |
| cloud-samples | 5 | 20 | 20 | 0 | 0 | 0 | 0 |
| docs | 3 | 11 | 11 | 0 | 0 | 0 | 0 |
| e2e | 10 | 37 | 37 | 0 | 0 | 0 | 0 |
| generated-agent | 35 | 112 | 105 | 3 | 7 | 0 | 0 |
| generated-s3 | 56 | 212 | 204 | 0 | 8 | 0 | 0 |
| scenes | 2 | 8 | 8 | 0 | 0 | 0 | 0 |
| shader-presets | 5 | 15 | 15 | 0 | 0 | 0 | 0 |
| store | 5 | 22 | 19 | 0 | 0 | 3 | 0 |
| unit-tests | 21 | 79 | 79 | 0 | 0 | 0 | 0 |
| **all** | **144** | **522** | **504** | **3** | **15** | **3** | **0** |

Again on 2026-10-19 with ANGLE's built-in calls and the ANGLE-only listing
(below): store 22 of 22 match (the 4 liquid-glass 2.0.8 variants through
the runner); all groups 507 of 522 match (7 through the runner), 15 both
refuse, 0 WebGL-unstable, 0 failures.

**No WebGL-unstable allowance** (second review, 2026-10-19). The gate no
longer has the column: a variant that differs from WebGL fails, and when
WebGL also moves against its own perturbed runs the failure says so (with
the side-by-side under `PM_SHADER_GATE_DIFFS`) and points at
`pm_render::effects::ANGLE_ONLY`, where such a program goes once proved
(`a_difference_webgl_is_unstable_under_fails_the_gate`). Only the
undefined-behavior probes (`known_open`, not user content) are listed
apart. On `shaders-20261018-022611` with the rule (and `PIPELINE_REVISION`
2, `pm_round` exact): 507 of 522 match (7 through the runner), 15 both
refuse, 0 failures, the same as before it.

**A user's own programs: Chike's setup** (second review, 2026-10-19).
`collect.ts --local <user-setups/chike-20261018/extensions>` adds a user's
installed extensions (private group `local`; a local extension that does
not load in the collector is noted, not fatal). Corpus
`shaders-20261019-chike` (186 cases, 695 variants; the repository's cases
as of the merge, the Store's 4 releases, and Chike's extensions): 34 of his
38 extension folders load (`ae-workspace-theme`, `displacement-origin-guide`
and `pexels-browser` need DOM APIs at activation and register no GLSL
there; `rotoscope` imports a file its folder does not hold) and register
38 effects and 2 extension layers, 166 variants: 163 match WebGL and 3
both refuse (one extension layer compares a `uniform bool` with a float,
which WebGL refuses too). No program of his differs natively, so none is
listed in `ANGLE_ONLY`. All groups: 677 of 695 match (11 through the
runner), 18 both refuse, 0 failures, after two gate changes the corpus
called for:
- A program WebGL links but whose draw raises a GL error (the kernel test
  fixture `void main(){}`, GL error 1282) leaves the cleared target, which
  is what today's compositor shows; the gate compares the engine's output
  with it instead of failing unconditionally (both transparent here). A
  link refusal still fails when the engine draws.
- An effect the native pipeline cannot translate (the e2e's `sampler3D`
  effect) used to be listed as awaiting the runner and passed; it now
  draws through the gate's runner host and is compared with WebGL (4 of 4
  match), and any request the runner has not answered fails the gate.
This covers his programs at the gate's synthetic inputs and parameter
sets, not his projects' own frames.

By kind: 44 shader layers, 13 extension fragment layers, 73 effects, 14
transitions. Variants: layers at their defaults (1.25 s), at 70% of each range
with other colors and flipped toggles (3.7 s), and with coerced values (numbers
as strings, short colors, toggles as numbers) at 24 fps; effects also with a
viewport crop and E-encoded inputs above 1.0; transitions at four progress
values. Judged by section 25's Standard class on 8-bit values and on float
values scaled by max(1, |reference|).

- **Both refuse** (compile parity holds): the two deliberate S3 `linkError`
  cases, and two agent drafts WebGL refuses too (`agent-pixel-dissolve` uses
  `v_px`, which transitions do not declare; `glitch-blocks` names a variable
  `active`, a reserved word). The in-app agent would see the compile error and
  fix it; both engines render nothing. Two more agent definitions never reach
  a renderer: the kernel's validator rejects their toggle parameters, written
  without `type: 'toggle'` (`orb-extension`, `bars-equalizer`), in both engines.
- **WebGL-unstable** (amendment 9; judged by review): Store
  `mrsigma/liquid-glass-ios27` 2.0.8, the case S3 analysed: 43 to 49 pixels
  differ by up to 99 CV, and WebGL against itself with every input texel one
  binary16 step away from zero moves 26 to 33 pixels by up to 67 CV
  (`webgl.ts --perturb-input`; the class is decided per variant from the
  perturbed runs). `zellzoi/liquid-glass-ios27` 2.0.1, a separate Store
  extension of the same name, matches. How it arises: the gate gives both
  engines the same input texels, and the differing pixels sit in one small
  region (the refracted highlight of the small square near the top right),
  where the program's output changes discontinuously with its arithmetic (a
  one-step input change moves WebGL's own output there); naga's MSL and
  ANGLE's MSL round a few operations differently, which is enough. Metal fast
  math is not the cause: with it off in the engine the gate moved further
  from WebGL (48 to 53 px, and the alt variant failed too; effects.md,
  Parity, 2026-10-19). Under
  amendment 22 this is a visible difference for a user with this effect:
  escalated to Jude with side-by-sides (`PM_SHADER_GATE_DIFFS=<dir>` writes
  the engine's output, WebGL's, and the differing pixels for every
  WebGL-unstable variant). **Resolved 2026-10-19** (orchestrator decision
  DEFS-2): the program is listed in `pm_render::effects::ANGLE_ONLY` and
  draws through WebGL's runner wherever one is attached (the app's viewer
  and exports), bit-exact against WebGL in all four variants; the gate now
  draws it with a runner host (`<corpus>/fallback-effects`, answered by
  `fallback-runner.ts --fallback`) and counts it as via the runner. Legacy
  programs also compile with ANGLE's built-in calls now
  (`docs/engine/effects.md`, "ANGLE's Metal calls"); that alone did not fix
  this program (43, 1, 38, 46 pixels).
- **Through the runner**: `env-cubemap-sky` (a `samplerCube`), above.

App level: the v1 corpus's shader and extension projects
(`layers/shader-*` for the five presets and the binding-quirks project,
`layers/studio-cube`, `layers/obj-model`), captured through the app on
2026-10-12, now compare in `tests/corpus.rs`: 27 frames, max 1 CV (Studio Cube
99.99% exact; Scanline Bars's transparent float frames within 0 CV).
`tests/corpus.rs` marks every non-built-in extension definition of a capture
missing, as the capture app had only the built-ins. The whole corpus with this
lane (newest v1, render, effects, and composite captures): 249 frames, 0
failed, 6 listed apart (the effects lane's Posterize frames); the same on
2026-10-18 after the merge. On 2026-10-19 (with the motion, compclose, and
the new `effects-20261019-055047-8d8a4ce4` captures): 348 frames, 0 listed
apart (Posterize is bit-exact now), 5 failed, all video-class frames
outside this lane (`motion/blur-video`, `retime/speed`,
`space25d/precomps`) and identical with this lane's changes removed.

## Layer reports for the Shader panel

With the engine presenting, the WebGL compositor draws nothing, so nothing
fills `GL.errors` or `uiState.getShaderMeta` for shader and extension
layers. `shader_layers::reports::layer_reports` (through
`Renderer::layer_reports`) lists, for a document revision and the viewed
composition with the compositions it nests, every shader layer whose code
WebGL refuses too (`ShaderCompile`, layer scope, diagnostics in the author's
lines) and every extension layer drawn as the missing checkerboard
(`DefinitionMissing`, layer scope); drawn at the frame's time or not, so a
hidden layer's error shows too. The presenter
(`pm-node/src/present/layer_reports.rs`) sends the whole list as `Failure`
events at every revision it presents, before the frame, and only what
changed when the definitions or the composition change within a revision;
routing to the legacy viewer resets it. The app
(`kernel/engine-definitions.ts`) holds a layer report for its revision and
clears it when a frame of a later revision did not repeat it, or when the
definition a missing layer needs is pushed. This relies on the engine
repeating every layer report that still holds; a future sender of layer-scope
reports must repeat them the same way. A shader layer only the fallback
runner draws is reported the same way as `NotHeadlessRenderable` (a warning:
it draws in the viewer; headless hosts cannot render it); the app shows it
under the layer's shader key like a compile error.

Evidence: `pm-node/tests/layer_reports.rs` (the interactive engine in a child
process: a refused shader layer and a missing extension layer reach the view
at the frame's revision, before the frame, and are repeated per revision and
not within one), `pm-node/tests/shader_fallback.rs` (the runner's layer),
the unit test in `layer_reports.rs`, and the vitests in
`definition-snapshots.test.ts` ("holds layer reports for their revision",
"shows a shader layer the fallback runner draws as not headless-renderable").

Regression suite (`tests/shader_layers.rs`, no WebGL needed): the binding
quirks, the time and resolution uniforms at the root rate, the cap, refused
code, extension parameters and `uniform bool`, missing and unknown
definitions, third-party mesh snapshots, rectangle picking, and the
`parseUniforms` fixture. Amendment 9's probe for every failing user shader:
the one pipeline refusal (`env-cubemap-sky`, a non-`sampler2D` sampler) is the
class of the sweep's existing sampler probes and has its shader-layer probe in
`layers_needing_the_runner_are_not_headless_renderable` and
`pm-node/tests/shader_fallback.rs`; the unstable Store shader is not a
pipeline failure.

## Commands

```sh
bun engine/crates/pm-render/tools/shaders/builtins.ts            # rewrite builtins.json (--check)
bun engine/crates/pm-render/tools/shaders/fetch-store.ts         # public Store → corpus folder
bun engine/crates/pm-render/tools/shaders/generate.ts --jobs 3   # agent corpus (claude --print)
bun engine/crates/pm-render/tools/shaders/collect.ts             # → shaders-<stamp>/
C=~/Library/Application\ Support/Powermove\ Engine\ Corpus/shaders-<stamp>
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/effects/webgl.ts --corpus "$C"
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/effects/webgl.ts --corpus "$C" --perturb-input
cargo test -p pm-render --test shader_gate -- --nocapture       # writes fallback requests
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/shaders/fallback-runner.ts --corpus "$C"
cargo test -p pm-render --test shader_gate -- --nocapture       # with the runner's pixels
~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/shaders/fallback-runner.ts --fallback "$C/fallback-effects"  # ANGLE-only effects
cargo test -p pm-render --test shader_layers
PM_RENDER_ONLY=layers/ cargo test -p pm-render --test corpus -- --nocapture
cargo test -p pm-node --test shader_fallback --test layer_reports
bun engine/crates/pm-render/tools/shaders/parse-uniforms-fixture.ts
```

## Gaps

- **The in-app agent harness** did not generate the agent corpus: it needs a
  running Powermove with its MCP tools. The same model and CLI ran with the
  harness's instructions instead (provenance above and in every output).
- **Store:** 4 public releases (figuate chrome 1.0.0, zellzoi ascii-magic-effects
  1.0.0 and liquid-glass-ios27 2.0.1, mrsigma liquid-glass-ios27 2.0.8), 5 effects,
  all effects; no Store extension registers a
  shader layer or a fragment layer yet. Local projects (`--local`) were not
  collected for this run.
- **WebGL-unstable class** (one Store shader): resolved by DEFS-2 (the
  program draws through WebGL's runner, above), and the gate no longer
  passes such a difference (above).
- **One machine, one GPU family** (M4 Max); the runner was measured in
  headless Chromium, not inside the Electron renderer (the session path is
  covered by the vitest and the pm-node test, the WebGL path by the gate).
- **Effects and transitions in the runner** were built 2026-10-18 (effects.md,
  "The fallback runner"), and the runner has its own draw deadline (above).
  Shader and fragment layers the engine draws are timed and guarded against
  runaways like effects (effects.md, "The watchdog"; `tests/watchdog.rs`).
  Since 2026-10-19 the budget records only, as today's compositor does
  (`gpu-timing.ts` never turns a shader off), and the viewer's hang budget
  is Chromium's 10 s, so a slow but finite shader that draws today draws.
  A transient runner failure (a lost context) is retried, never kept
  (effects.md, "Transient failures").
- **Pixels per frame:** an animated fallback layer costs a WebGL draw,
  readback, and upload per frame shown; there is no render-ahead for it (a
  static one is uploaded once). The runner reads its target as half floats
  since 2026-10-19 (25 ms per full-HD request with two textures, from 2.7
  s); an animated fallback program still plays and exports below today's
  rate (effects.md, Gaps: a full-HD fallback effect export took 7.0 s
  against today's 1.1 s).
- **Not run:** the full pm-render suite (targeted tests only, per the machine
  limits).
- **Watchdog tests** (issue A2): fixed 2026-10-18 with the watchdog's
  deterministic test hook (effects.md, "The watchdog"); the pm-node test
  passed 20 of 20 runs.

## Perf3: bounded asynchronous runner readback (2026-10-19)

`ShaderFallbackRunner` retains its RGBA16F framebuffer at an unchanged size.
Its asynchronous path serializes GL submission and allows at most three
pixel-pack buffers to wait for GPU fences, yielding to the browser between
polls. A synchronous compatibility snapshot may draw between those submissions:
each PBO already owns its read command and is copied from its own buffer after
completion. Context replacement/disposal ends pending reads and frees the
buffers; a read has a 10-second bound. The blocking snapshot path remains
available.

RGBA16F is read as binary16 when the implementation exposes that read pair;
the float path converts the input view's bits without per-component allocation.
The conversion test covers every finite binary16 value, signed zero, subnormals
and a nonzero source byte offset. The real Electron/WebGL regression compares
more than 60 million components from concurrent requests against the blocking
path, with size changes and an interleaved synchronous snapshot: zero differing
components (`e2e/shader-fallback-readback.spec.ts`, 2026-10-19, under the Electron
lock). This is a correctness pass, not a paired export-throughput pass.

The effects/transition runner is integrated in this lane as of the defs merge.
Its input texture protocol, per-pass watchdog fences, MAX_TEXTURE_SIZE cap,
compile waits, synchronous snapshots, and context-loss retries are preserved.
GL submission is serialized; up to three PBO reads can finish independently.
Per-pass fence polling yields for 1 ms rather than a whole 16 ms interval.
The 60-frame HD export and animated-effect viewer rows remain unmeasured;
no faster-than-today claim follows from the PBO regression.
