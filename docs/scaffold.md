# Electron scaffold

Phase 6 retired the independent classic-script page and native Swift build. The
shared `css/` and `assets/` trees remain because the Electron renderer imports
their styles and fonts.
Electron main-process code lives in `src/main/`.
The sandboxed preload bridge lives in `src/preload/`.
Shared IPC names and bridge types live in `src/shared/`.
The Svelte 5 renderer lives in `src/renderer/`.
Its shell, dock layout, overlays, and panels install unconditionally; there is
no legacy-chrome mode or persisted shell switch. The remaining
`src/renderer/src/legacy/` modules provide application engines, imperative
canvas hosts, and bootstrap adapters still consumed by the live renderer.
`electron.vite.config.ts` builds all three targets into `out/`.

- `npm run dev` starts Electron with renderer hot reload.
- `npm run build` creates the production bundles.
- `npm run preview` launches the built application.
- `npm run typecheck` checks Svelte and both Node-side targets.
- `npm run test` and `npm run test:unit` run the Vitest suite.

Production renderer assets are served from the private `app://powermove/` origin.
