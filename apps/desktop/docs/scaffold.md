# Electron scaffold

Phase 6 retired the independent classic-script page and native Swift build. The
`css/` and `assets/` trees in this package remain because the Electron renderer
imports their styles and fonts; the design tokens moved to the workspace package
`@powermove/tokens` (`packages/tokens/tokens.css` at the repository root) so the
website can share them.
Electron main-process code lives in `src/main/`.
The sandboxed preload bridge lives in `src/preload/`.
Shared IPC names and bridge types live in `src/shared/`.
The Svelte 5 renderer lives in `src/renderer/`.
Its shell, dock layout, overlays, and panels install unconditionally; there is
no legacy-chrome mode or persisted shell switch. The remaining
`src/renderer/src/legacy/` modules provide application engines, imperative
canvas hosts, and bootstrap adapters still consumed by the live renderer.
`electron.vite.config.ts` builds all three targets into `out/`.

Scripts run from `apps/desktop` (or from the repository root with
`bun run --cwd apps/desktop <script>`):

- `bun run dev` starts Electron with renderer hot reload.
- `bun run build` creates the production bundles.
- `bun run preview` launches the built application.
- `bun run typecheck` checks Svelte and both Node-side targets.
- `bun run test` and `bun run test:unit` run the Vitest suite.

Production renderer assets are served from the private `app://powermove/` origin.
