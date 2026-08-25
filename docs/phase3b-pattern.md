# Phase 3b legacy-module conversion recipe

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
