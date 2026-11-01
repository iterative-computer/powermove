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
