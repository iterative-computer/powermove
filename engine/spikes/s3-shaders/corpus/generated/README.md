# Generated agent-style shaders

The S3 gate needs shaders written the way agents write them, covering every row
of the section 11 contract table (`CONTRACT_ROWS` in `index.ts`). Each entry
records the request (`prompt`) it answers and the contract features it
exercises (`covers`).

Provenance: written for the S3 spike by Claude Opus 5.5 working from the same
documentation the in-app agent reads (`apps/desktop/docs/EXTENSIONS.md`, the
`EffectDefinition`/`TransitionDefinition` contracts in `kernel/api.ts`, and the
shader-layer header in `legacy/gl/shaders.ts`). They were not produced by
running prompts through the in-app agent harness; the plan's final gate asks
for that, and this folder's prompts are the intended inputs.

Cases tagged `ub-probe` deliberately use operations GLSL leaves undefined
(`pow` of a negative base, `normalize` of zero, out-of-range `texelFetch`,
division by zero). Cases tagged `linkError` are invalid in WebGL on purpose:
the engine must reject them as WebGL does.
