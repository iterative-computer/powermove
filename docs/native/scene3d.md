# Native Studio document core

## 2026-11-04 — source and decisions

This lane ports `origin/feat/na-3d-source` at `b81b94c66` (`core/scene3d`,
`core/studio/operations.ts`, and `extensions/layers-3d`). Shipped behaviour
reference: `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`; v1.2.0 remains a required
input. The integration design update was merged at `78294c4c9`.
[Opus's Studio spec](design/surfaces/3d-studio.md) controls all presentation.
No panels or visual output are built by this crate. Its section 12 proposals
are excluded. Exploded views, place-on and soften belong to agent operations.

| Decision | Selecting rule |
| --- | --- |
| Models, lights and cameras remain ordinary timeline layers; normal property paths own transforms and animation, `d.data` owns typed content | N3 exact authored state; N6 existing extension paths; N7 id-based commands |
| Plan edits as pure Rust data and execute against the actor's scratch tree, with normal inverse capture, journal and revision checks | Single-writer kernel architecture; N7; N8 no JS waits |
| Full channel assignment retains authored handles, key identities and unknown key metadata instead of reconstructing only ease/hold | N3 correctness over imitation |
| Use native OpenCascade/XDE in a separate import helper for STEP/IGES/BREP, preserving names, hierarchy and face colors in GLB | N1 native CAD quality; N8 excludes the Electron JS/WASM host; no Blender |
| Retain selected original import bytes and archive replaced legacy layers inside the document | N4 archive, never delete; N3 exact source provenance |
| Keep navigation transient and feed light previews to kernel gestures, with host-supplied surface picks | N3 shot intent; N7 headless driving; host independence |
| Use exact stable splat depth order; reject malformed/nonfinite input instead of reproducing WebGL errors or a quantization artifact | N3 spec over Chromium quirks |
| Use Softbox, Spotlight, Bulb and Sun in model labels, retaining area/spot/point/sun on disk; use the design's failure/progress copy | Opus design ownership and N6 stored types |

`pm-scene3d` joins the existing `crates/*` workspace member glob. It reuses
`pm-geom3d` for selected OBJ/MTL/texture packaging and self-contained glTF
loading, with Studio's 512 MiB, 10M triangle and 65,536-node import limits.
It does not fetch model dependencies. The old OBJ bundle packer's smaller
128 MiB ceiling is still a known engine dependency boundary.

Kernel wiring supplies `Scene3dCommand::Apply`, the existing `Edit`/`SetData`
entrypoints and 3D channel discovery for normal property/keyframe commands.
Final scratch validation checks coupled camera planes and authored ranges.
Structural changes are ordered before channel changes in a compound edit.

Upgrade runs in memory before query publication; source files change only on
Save. A legacy container becomes an enclosing group retaining its original ID,
outer transforms/effects/masks/locks and complete archived source; children
become ordinary 3D layers. Failed conversions retain all source data and a
machine-readable issue. This is not a claim of complete old-history or renderer
integration. Those gates remain visible until verified below.

## 2026-11-04 — initial checkpoint

Targeted Linux tests: 16 schema/document tests and 9 real-kernel tests pass,
covering atomic failure, camera cross-field edits, locks/hand intent, full
channel metadata, shot tracks, duplicate key identity, exact Undo/Redo and
PMV save/reopen including Undo.

Work remains in progress: MCP dispatch/schema/readback, imported geometry
bounds adapters, animated legacy upgrade, native CAD build/real fixtures,
navigation/light/import negative tests and final full test replay.
No macOS rendering, physical input, GPUI/WebKit, signed helper or Opus visual
acceptance is claimed by this checkpoint.
