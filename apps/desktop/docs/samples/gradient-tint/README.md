# Gradient Tint effect

This complete extension adds an effect to Effects & Presets. It registers no
panel, command, or layer rig. The Inspector and animation controls are supplied
by Powermove from the four effect params.

1. Copy this folder into the run's extension staging directory. Rename the
   folder, manifest id, and effect id for the requested effect.
2. Implement the complete `definition` object. Read `../../api.ts` for its types.
   Effect ids match `/^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/`. Use at most **32 params**,
   each with a unique key matching `/^[a-z][a-zA-Z0-9]*$/`. Passes must be 1–8;
   the non-empty fragment body is limited to 65,536 characters.
3. Call `validate_effect` with `{ "definition": <complete definition> }` before
   returning the extension. This runs the real registration validator without
   changing the project. It does not compile GLSL or prove rendered behavior.
4. Return the extension id and action in `extensions`. Staged code is not live
   until Powermove promotes and loads it. Do not attempt to apply a new effect
   through live tools before it is registered.
5. In the automatic verification continuation, use `get_workspace_state` to
   inspect extension health and `registeredEffects`. If applying the effect was
   requested, use `apply_commands` with `add_effect` on the intended layer:
   `{ "type": "add_effect", "target": "LAYER_ID", "effect": "gradient-tint" }`.
   Inspect `get_project_state` for the effect instance and its editable property
   paths. Use those observed paths for `set_property` or `replace_keyframes`.
6. Check the effect in Effects & Presets and the Inspector. Inspect real
   `render_frames` images at representative times when applied, including
   animated parameter changes. Use isolated test data for extra test layers;
   never add test content to the user's project. If no applicable layer exists,
   report that rendered behavior remains unverified.

For text animation, a shader receives pixels, not glyph layout or character
boundaries. Do not present a uniform grid wipe as character-accurate typing.
Preserve the requested effect surface and report missing host capabilities
instead of silently replacing it with a panel or a layer rig.
