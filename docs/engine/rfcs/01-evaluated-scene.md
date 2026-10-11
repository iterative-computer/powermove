# RFC 01: `EvaluatedScene`

Status: draft, version 0. Module: `pm_protocol::scene`. Gates Phase 1;
freezes when Phase 1 starts. Producer: `pm-eval` and `pm-geom3d` (Model and
evaluation lane, 3D lane for worlds). Consumers: `pm-render`, `pm-blender`,
`pm-cli parity`, agent `get_3d_scene`.

## Purpose

The output of evaluation for one (document revision, composition, time,
optional view override): everything the render graph needs and nothing it
must re-derive (plan sections 7, 8, 10). Evaluation is a pure function, so
the scene is a value: two evaluations of the same inputs are equal, natively
and in WASM (the determinism bar, section 2).

Inside an engine the scene is passed by reference. It crosses a process
boundary only for parity runs, `pm-cli`, and agent inspection, through
`Request::Evaluate` → `Response::Scene`.

## Types

| Type | Meaning |
|---|---|
| `EvaluatedScene` | Composition id, revision, time (ticks), exact frame rate, size, background, shutter, the view override used, layers bottom to top, 3D worlds, depth sets, ordering runs, and per-layer evaluation failures |
| `EvaluatedLayer` | Id, `LayerKind`, `Compat` level, visibility (active, not hidden, passing solo), opacity, blend mode, transform, enclosing group, `LayerSource`, effects, masks, track matte, active transition, motion-blur switch |
| `LayerKind` | The section 10 table: solid, shape, text, captions, image, SVG, video, shader, extension fragment, extension mesh, 3D object, light, camera, legacy `scene3d`, precomp, adjustment, group, model group, null, audio |
| `LayerTransform` | `flat` (3×3 layer-to-composition), `perspective` (4×4 through a 3D group's perspective, the 2.5D homography planes), `world` (4×4 layer-to-world, 3D layers only) |
| `LayerSource` | Where pixels come from: solid color and radius, vector content (by content key), a media frame at a source time after retiming, a fragment shader with parameters, a mesh definition, a precomp at a child time, a 3D world's output, adjustment, group (isolated or not), none |
| `EvaluatedEffect`, `EvaluatedTransition`, `NamedParam`, `ParamValue` | Definition id, animated enable, compat level, and parameter values (number, toggle, color, vector) |
| `EvaluatedMask`, `MatteRef` | Parametric rectangle and ellipse masks, bezier path masks (by content key), animated enables; track matte source and mode |
| `World3d` | Camera (layer or studio default), lights, objects, tone mapping (`ToneMapping3d`, contract 4), and whether Blender renders it |
| `Light3d`, `LightKind`, `Camera3d`, `Object3d`, `ModelSource` | The section 8 3D model: ambient, sun, point, spot, area; perspective or orthographic cameras; generated, imported (with clip time), or missing models |
| `DepthSet` | The object layers sharing a depth prepass and shadow maps, their shadow casters, and the bottom layer that draws the background |
| `OrderedRun` | One run of consecutive 3D layers with its back-to-front draw order |
| `Shutter` | Angle and phase; `sample_offsets(n)` gives the motion-blur sample times in frames |
| `Compat` | Level 1 or 2, encoded as the number 1 or 2 |

Geometry-heavy payloads (vector paths, text runs, glyph outlines, meshes,
BVHs) are not in the scene. Layers carry a `content_key` (vector content,
path masks) or a `geometry_key` (generated meshes) that hashes the evaluated
geometry; `pm-render` fetches the geometry from `pm-eval` and `pm-geom3d`
caches in the same process. This keeps the scene small enough to compare,
hash, and send, and makes content keys the render cache's geometry input
(section 8, content keys).

## Invariants

Checked by `EvaluatedScene::validate`:

1. The composition id is valid, the frame rate positive, the size non-empty,
   the shutter angle 0 to 720° (today's 0 to 2 frames).
2. Gradient backgrounds have 2 to 8 stops with positions in 0..1, ascending.
3. Layer ids are unique. Opacity is in 0..1. Transition progress is in 0..1.
4. A level 1 layer has at most 8 parametric masks (today's GPU path; more are
   a level 2 fix, appendix A item 14).
5. A matte source is another layer in the same scene.
6. 3D kinds (object, light, camera), and only they, use `world` transforms.
   A `world` source names an existing world.
7. Per world: at most 512 models, 16 lights, and 4 shadow-casting lights,
   not counting embedded glTF punctual lights (section 3, "may be raised,
   never lowered"). `casts_shadow` means the light casts in this render:
   `pm-eval` resolves "the first 4 shadow lights in layer order" before the
   scene is built. A composition over a limit does not produce an invalid
   scene; evaluation reports the failure and the 3D render fails, as today.
8. **Depth sets and ordering are separate computations** (section 27,
   contract 1). Depth sets contain only 3D object layers, each in exactly
   one set; shadow casters and the background layer are members; the
   background layer is the set's bottom layer in stack order. Each ordering
   run is a permutation of one contiguous block of 3D layers (objects,
   lights, cameras) and no layer is in two runs. Neither structure is derived
   from the other: a light in a run is never in a depth set, and a 2D layer or
   styled group ends a depth set without necessarily ending a run.

Not checkable from the scene alone, and owned by `pm-eval`'s tests and the
parity corpus: depth-set membership rules (inactive, hidden, lights,
cameras, and plain group headers do not end a set; opacity ≤ 0.001 and solo
exclusion), ordering by camera depth of the object origin with lights and
cameras at depth 0, the studio defaults, `writes_depth` (no transparent
material, opacity ≥ 0.999, no enabled mask), and glTF clip time
`max(0, t − layer.from)`.

Motion blur: the scene is evaluated at the center time. The renderer
evaluates one scene per sample at `t + shutter.sample_offsets(n)[i]` frames
with `n` from the composition's `motionBlurSamples` for the output kind
(contract 2, `MotionBlurKind`). `Shutter::from_frames(0.5)` reproduces
today's centered offsets exactly (tested against `compositor.ts`).

## Versioning

Version 0 until Phase 1 starts, then 1. The scene is the largest message on
the wire (the sample is 3.6 KB); new layer kinds, sources, or 3D features
are new enum variants, which old readers cannot read, so each is a version
bump after the freeze. `pm-cli parity` pins the scene version it compares.

## What Phase 0 must confirm

- **S5 (WASM evaluator):** the evaluator produces this structure within the
  S5 targets (`evP` < 1 µs cached is for the JS fast path, which does not
  build a scene; building a full scene for 884 layers is measured
  separately and reported).
- **S7 (3D):** that `World3d`, `Light3d`, `Camera3d`, and `Object3d` carry
  every input the three.js r180 port needs (shadow parameters are constants
  from section 8 and stay out of the scene; area light size, spot penumbra,
  and decay are in). Any missing input found by S7 is added before the
  freeze.
- **S4 (text and vectors):** that a content key per text or shape layer is
  enough for the text and vector renderers, and that captions word states
  fit in the vector content key rather than a separate field.
- **Phase 0 audit of third-party `layer3d`, `scene3d`, and `mesh`
  definitions:** that `LayerKind` and `LayerSource::Mesh` cover them.

## Open questions

1. Whether `LayerSource::Vector` should carry evaluated geometry inline for
   cross-process consumers (`pm-cli parity` could then diff geometry without
   a second request). Current answer: no; parity asks `pm-eval` directly.
2. The exact 2.5D representation: a 4×4 matrix per layer plus the group id,
   or the group's camera and each layer's plane. S5 and the 2.5D port decide.
3. Whether extension-layer and shader-layer uniforms should be resolved to
   GLSL binding values here (including today's quirk that `vec2`, `vec4`,
   and `int` uniforms stay 0, section 11) or in `pm-render`. Current answer:
   `pm-render`, from the definition snapshot (contract 10).
4. Scene diffs: whether the interactive engine wants a per-layer change set
   between consecutive scenes instead of whole scenes. Not needed for the
   contract; an implementation detail of `pm-render` caching.
