# Electron scaffold

The legacy `index.html`, `js/`, `css/`, and native build remain independent.
Electron main-process code lives in `src/main/`.
The sandboxed preload bridge lives in `src/preload/`.
Shared IPC names and bridge types live in `src/shared/`.
The Svelte 5 renderer lives in `src/renderer/`.
`electron.vite.config.ts` builds all three targets into `out/`.

- `npm run dev` starts Electron with renderer hot reload.
- `npm run build` creates the production bundles.
- `npm run preview` launches the built application.
- `npm run typecheck` checks Svelte and both Node-side targets.
- `npm run test:unit` runs Vitest unit tests.
- `npm test` continues to run the untouched legacy suite.

Production renderer assets are served from the private `app://powermove/` origin.
