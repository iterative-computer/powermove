# Phase 0 spike reports

One report per spike (plan section 26). Each spike's conclusion is in
`docs/rust-engine.md`, "Phase 0 results and plan amendments".

The spike code (`engine/spikes/s1-bridge` … `s8-contention`) was removed once its
results were ported into the workspace crates; the paths these reports cite live in
Git history at commit `68bb2dc61`. The pieces still in use moved:

- S1's main-process load generator: `apps/desktop/src/main/engine/integration/measure-load.cjs`
- S3's agent-style shader corpus: `engine/crates/pm-render/tools/agent-shaders/`
- S5's synthetic 884-layer project: `apps/desktop/scripts/lib/synth-project.mjs`
