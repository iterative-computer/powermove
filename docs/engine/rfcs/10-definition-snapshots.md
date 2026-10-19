# RFC 10: Definition snapshots and lifecycle

Status: draft, version 0. Module: `pm_protocol::definitions`. Gates Phase 2;
freezes when Phase 2 starts. Owner: GPU and render (definition snapshots,
section 27). Producer: the kernel (renderer, document host). Consumers:
`pm-render` and the GLSL pipeline in both engines, `pm-cli`, export jobs,
Collect Project.

## Purpose

The engine renders effects, transitions, and extension layers that the
kernel registers at runtime (plan section 11). The kernel stays the
validator and the GLSL generator (`kernel/glsl.ts`,
`kernel/extension-layers.ts`), so it pushes the engine a **snapshot** of
every winning registration: `{id, version, renderer (verbatim, including
role and assetField), glsl?, params, flags}`, with UI-only fields (`label`,
`group`, `ui`, `advanced`, `options`, `unit`, widget hints, `icon`,
`color`, `defaults`) stripped. Snapshots are re-pushed on every register,
dispose, and hot reload. Export jobs and Collect Project embed the
snapshots they use, so a job renders the definitions that were current
when it started, and `pm-cli` renders a `.pmv` without a kernel.

## Types

| Type | Meaning |
|---|---|
| `DefinitionSnapshot` | `id`, `DefinitionKind`, extension-layer `version` (0 for effects and transitions), `DefinitionOrigin`, `renderer` (extension layers only), `glsl` (effects, transitions, fragment layers), `params`, `flags` |
| `DefinitionKind` | Effect, transition, layer. Ids are unique per kind, not across kinds |
| `DefinitionOrigin` | Built-in, fork (a local fork registering under a built-in id), local (in-realm, including agent-published), Store (sandboxed, untrusted GLSL). Decides the limits |
| `LayerRenderer` | Verbatim from `ExtensionLayerDefinition.renderer`: `fragment {fragment}`, `mesh {assetField}`, `scene3d`, `layer3d {role}`. Encodes exactly as the kernel schema (`{kind: "mesh", assetField}`; tested) |
| `Layer3dRole` | Object, light, camera |
| `ParamSpec` | `number {key, default, min, max, step?}`, `color {key, default}` (`#rrggbb`), `toggle {key, default}`: `EffectParamDefinition` without its presentation fields |
| `WrappedGlsl` | The kernel-wrapped GLSL ES 3.00 source ready for the pipeline (section 11, step 1), the number of lines the wrapper added before the user's first line (for error line mapping), and the user's body length in UTF-16 code units and UTF-8 bytes (what the limits count) |
| `DefinitionFlags` | `passes` (1 to 8), `keepOrig`, `backdrop`, `rawShader`, `viewportSafe`, `viewportPadding`, `clamp` |
| `DefinitionsUpdate` | `generation`, `full`, `upserts`, `removals`. Sent as `Request::PushDefinitions` (session-bound), answered by `definitionsApplied` |
| `DefinitionRemoval` | No registration remains for `(kind, id)`; the engine marks it missing |
| `DefinitionsApplied` | The generation applied and the per-definition compile or translation failures (`EngineError` with definition scope and `ShaderDiagnostic`s, contract 7) |
| `DefinitionSetSnapshot` | What an export job or Collect Project embeds: the generation, every snapshot, and the ids already marked missing |
| `DefinitionSet`, `UpdateError` | Engine-side state: applies updates in generation order, tracks missing ids, and produces a `DefinitionSetSnapshot` |
| `limits`, `BUILTIN_3D_IDS` | The section 3 and `kernel/sandbox-schemas.ts` limits; the four built-in ids with id-based 3D rules |

## Invariants

Checked by `DefinitionSnapshot::validate` and `DefinitionsUpdate::validate`
(run by the decoder on every `pushDefinitions`):

1. **Limits match today's kernel** (section 3; may be raised, never
   lowered):

   | Kind and origin | Parameters | Fragment |
   |---|---|---|
   | Effects and transitions, any origin | 32 | 65,536 UTF-16 code units; 1 to 8 passes |
   | Extension layers in the editor realm (built-in, fork, local) | 48 | 96,000 UTF-8 bytes |
   | Extension layers from the Store (sandboxed) | 32 | 65,536 UTF-16 code units |

   The wrapped source (user body plus kernel preamble) is at most 512 KiB.
2. **Parameter keys** are unique per definition and match
   `^[a-z][a-zA-Z0-9]*$` (the kernel's rule, so `u_<key>` is a valid GLSL
   identifier). Numeric ranges are finite with `min ≤ max`; a step is
   positive; a color default is `#rrggbb`.
3. **Shape by kind.** Effects and transitions carry GLSL and no renderer.
   Transitions are single-pass with no `u_orig` and no `u_backdrop`.
   `viewportPadding` names numeric parameter keys. Extension layers carry a
   renderer and default flags; fragment layers carry wrapped GLSL whose
   UTF-8 body length equals the verbatim `fragment`; mesh, `scene3d`, and
   `layer3d` layers carry no GLSL; a mesh `assetField` is 1 to 128 bytes.
4. **The engine only sees winners.** The kernel resolves registration order
   (last registered wins; disposing restores the previous registration) and
   pushes the result. A fork registering a built-in id is pushed under that
   id with `origin: fork`, so the engine uses the fork's definition
   (section 11).
5. **Generations are ordered.** The first update on a session is `full`. A
   delta must follow the current generation by exactly one; a full update
   may jump to any larger generation and replaces the set. A gap fails with
   `UpdateError::Gap`, and the sender answers with a full update. One update
   names each `(kind, id)` at most once, and a full update has no removals
   (tested).
6. **Missing definitions behave as today** (section 11). Removing an id, or
   omitting it from a full update, marks it missing: a missing effect is
   skipped with its data kept; a missing extension layer shows the
   checkerboard placeholder. Re-registering clears the mark (tested).
   `pm-cli` fails only on an id that has no snapshot and is not in
   `missing`.
7. **3D roles keep today's id-based rules.** `powermove.3d.{object, light,
   camera}` and `powermove.3d.scene` place layers in the 3D scene by id; when
   a fork registers one of these ids with a different renderer kind, the
   snapshot's kind wins and the layer leaves the 3D scene (section 11).
8. **Compile failures stay per definition.** A definition that fails to
   compile or translate is reported in `DefinitionsApplied.failures` and
   fails only where it is used (contract 7, definition scope). Store GLSL is
   untrusted input to a fuzzed, resource-limited pipeline (section 11).
9. **Read-your-registrations.** `definitionsApplied` is sent after the
   update is in force for every later frame on the session, so a tool call
   that registers an extension and then renders sees it (contract 15,
   `ExtensionRegistered`).

Not checkable from the snapshot alone, and owned by the GLSL pipeline and
its parity tests: the effect contract's varyings and uniforms
(`v_uv`, `v_st`, `v_px`, `u_tex`, `u_res`, `u_texel`, `u_time`, `u_prog`,
`u_coordRes`, `u_pxOrigin`, `u_pass`, `u_orig`, `u_backdrop`, `u_<param>`,
positional `u_p<i>` and `u_c<i>`), toggles bound as `float` for effects and
`bool` for extension layers, sampler state per effect type, and the
`clamp` rule (values above 1.0 pass through unless an effect declares
`clamp: true`).

## Lifecycle

| Event | What is pushed |
|---|---|
| Session open | A `full` update with every winning definition |
| `register` | A delta upserting the new winner for that id |
| `dispose` | A delta upserting the previous registration, or a removal if none remains |
| Hot reload | A delta upserting the reloaded definition |
| Generation gap, engine restart | A `full` update at a new generation |
| Export job start | The job snapshot session receives the current set; the job embeds it as a `DefinitionSetSnapshot` |
| Collect Project, `.pmv` for `pm-cli` | The `DefinitionSetSnapshot` of the definitions the project uses |

Every engine session gets the same stream; the job engine's mirror session
and the document host's sessions (contract 15) receive it too.

## Versioning

Version 0 until Phase 2 starts. New renderer kinds, parameter types, or
flags are new variants or fields and a version bump after the freeze. The
WGSL contract at apiVersion 4 (section 11, "Contract v2") adds a
`WrappedWgsl` alongside `glsl` in one version bump; GLSL stays supported
indefinitely, so no snapshot field is ever removed for it.

## What Phase 0 must confirm

- **S3 (shaders):** that every GLSL source in the repository, the Store,
  the docs samples, and the generated agent corpus arrives through these
  snapshots with everything the pipeline needs, and that
  `WrappedGlsl.body_line_offset` maps every glslang and naga diagnostic to
  the user's line numbers. If the pipeline needs the unwrapped body or the
  preamble separately (for example to inject `pm_FragCoord` only in user
  code), the field is added before the freeze.
- **The Phase 0 Store and local-extension audit:** the third-party
  `layer3d`, `scene3d`, and `mesh` definitions and the local forks of
  built-ins, to confirm `LayerRenderer` and `DefinitionOrigin` cover them.
- **S1 (with the frame bridge):** that pushing a full update on session
  open, before the first frame, fits the cold-start gate ("Engine ready
  before the window finishes loading the last project", section 25).

## Open questions

1. **Shader layers.** User shader layers (`type: "shader"`, GLSL in
   `d.code`, the five presets) are document content, not registrations;
   their source reaches the engine through the patch stream. Either the
   kernel also pushes a `WrappedGlsl` for each distinct `d.code` keyed by
   its hash, or `pm-render` ports the shader-layer header and
   `parseUniforms` (section 11 says the engine ports `parseUniforms`
   exactly). Current answer: the port, because `pm-cli` must render shader
   layers from a `.pmv` without a kernel; S3 confirms the wrapped output is
   byte-identical to the kernel's.
2. Whether the kernel should strip `rawShader` positional aliases into
   explicit bindings here, or the pipeline keeps resolving `u_p<i>` and
   `u_c<i>` itself. Kept in the pipeline for now.
3. Whether `DefinitionSetSnapshot` should key `missing` by `(kind, id)`
   rather than id alone. Ids rarely collide across kinds; `pm-cli`
   resolves by the kind the document asks for. Decided before the freeze.
4. The fallback runner (decision 19): a definition that fails translation
   on a user's machine renders through the minimal GLSL runner in the
   renderer and is marked not headless-renderable. Whether that mark
   travels in `DefinitionsApplied` (per definition) or only in
   `FailureReport` events. Current answer: `DefinitionsApplied.failures`
   with `ErrorCode::NotHeadlessRenderable`. A viewer that has the runner
   draws such definitions through it (effects and transitions with their
   textures and passes, shader and fragment layers; `docs/engine/effects.md`,
   "The fallback runner"); a job asks the host to run them in a window's
   runner (`Request::ShaderFallback`, contract 14). Late compile failures
   arrive as definition-scoped `FailureReport` events whose `revision` is
   the definitions generation they were translated at (rfc 07).
