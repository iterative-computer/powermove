# Phase 3b legacy-module conversion recipe

Historical note: this recipe records the migration that converted the classic
renderer into TypeScript modules. Phase 6 subsequently deleted its source
oracle; path names below describe migration inputs and are not live dependencies.
The commands are quoted as they were run at the time (npm at the repository
root); the desktop app now lives at `apps/desktop` and the equivalent live lanes
are `bun run build`, `bun run test`, `bun run typecheck`, and `bun run test:e2e`.

1. Find `js/<dir>/<name>.js` in `index.html`; record its exact script position.
2. Create `src/renderer/src/legacy/<dir>/<name>.ts` with this shell:
   ```ts
   /* Ported from js/<dir>/<name>.js — behavior-preserving. */
   import type { PMRegistry } from '../registry';
   export function install(PM: PMRegistry): void {
     // original IIFE body
   }
   ```
3. Copy the IIFE body verbatim. Remove only the wrapper and
   `const PM = window.PM`; add TS-strict annotations. Explicit `any` is allowed
   under `legacy/`. Do not redesign, rename, reorder, or eagerly evaluate code.
4. Keep browser globals explicit (`window.X`, not `X`). Add declarations only
   when TypeScript needs them; do not capture a global at install time if the
   legacy code read it later.
5. Import the new `install` in `legacy/bootstrap.ts` and call it in the same
   relative order as the original `index.html` tags.
6. Remove the original `<script>` tag from `index.html`. The one classic
   `host/legacy-bundle.js` tag stays at the former easing position; unlike a
   module tag it executes synchronously there, before later classic scripts.
   Move/split that tag only if a converted module truly needs an earlier
   classic module during installation rather than when one of its methods runs.
   If a wiring oracle searches for the old path, retain a non-script migration
   comment naming it; never retain the active tag and install the module twice.
7. Keep the original JS file as the Phase 6 oracle and add only this header:
   `No longer loaded — superseded by src/renderer/src/legacy/...; kept for the legacy test oracle until Phase 6.`
8. Add a colocated `<name>.test.ts` using Vitest. Call `install` with a stub
   `PMRegistry` and transplant representative assertions from the legacy tests.
   Never delete or rewrite the Node/vm oracle tests.
9. Build output is generated, ignored, and never committed. Do not hand-edit
   `host/legacy-bundle.js`; `npm run build:legacy` owns it.
10. Run: `npm run build`, `npm test`, `npx vitest run`, `npm run typecheck`,
    and `npx playwright test --reporter=line`.

## Phase 3b outcome (integrator notes)

- All 36 legacy modules (32 IIFEs + ui-state, selection, the Electron shim, app) are
  `install(PM)` TypeScript modules bundled into `host/legacy-bundle.js`; `index.html`
  carries one live script tag. Five parallel lanes, each Opus-reviewed for
  verbatim-ness via normalized diffs; 4 accepted outright. The originals under `js/`
  remain the `node --test` oracle until Phase 6.
- Deliberate deviation from verbatim: `legacy/gl/compositor.ts` precomp branch read an
  undeclared `gl` in the source (`js/gl/compositor.js:277`), so every nested
  composition render threw a ReferenceError. Fixed to `GL.gl`; `e2e/precomp.spec.ts`
  fails on the original line and passes on the fix (mutation-checked).
- Resolved: project expressions now use the bounded parser/interpreter instead of
  `Function`. The `idx` helper receives the layer's actual composition position,
  with unit, scale, CSP, and hidden-Electron coverage guarding the behavior.
- Process note: a reviewer flagged the integrator's precomp fix as lane drift because
  it landed mid-review; the auto-spawned fix lane was stopped before it could revert
  it. Integrator edits to lane-owned files now wait for the review stage to finish.
