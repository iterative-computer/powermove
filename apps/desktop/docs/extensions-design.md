# Extensions — design

Powermove is a small **kernel** plus **extensions**. The built-in editor ships as
extensions written in the same format a user's (or the agent's) extension uses.
Nothing in the UI is privileged: any built-in can be layered over or replaced.

Product surface (what a user sees): one verb ("ask"), one list ("Mods"), nothing
can brick the app, updates are one button, every change is reversible. Everything
below exists to make those five sentences true.

## 1. Vocabulary

| Term | Meaning |
|---|---|
| Kernel | `src/renderer/src/kernel/` + `src/main/extensions/`. Registries, loader, edit boundary, render engine. Ships in the read-only app bundle. |
| Extension ("mod" in UI) | A folder with `manifest.json` + entry module exporting `activate(api)`. |
| Built-in extension | Lives in `src/extensions/<id>/`, bundled with the app, loaded first. Read-only. |
| User extension | `<userData>/extensions/<id>/`. Written by the user or the agent. Compiled at runtime. |
| Project extension | `<projectDir>/.powermove/extensions/<id>/`. Same as user, scoped to a project. |
| Kernel API | `PowermoveAPI` (`src/renderer/src/kernel/api.ts`). Versioned (`apiVersion: 1`). The only thing an extension may import from the host (`import type … from 'powermove'`). |

## 2. Layout

```
src/renderer/src/kernel/
  api.ts            PowermoveAPI + all contribution types (FROZEN contract)
  registry.ts       generic Registry<T> with override/replace + change events
  registries.ts     panels, commands, keybindings, effects, transitions, themes,
                    palette providers, menu contributions, status items, hooks
  host.ts           creates the PowermoveAPI for a given extension id (scoped disposables)
  loader.ts         imports bundles, activate/deactivate, isolation, health
  extensions.svelte.ts   reactive list of loaded extensions (for the Mods panel)
src/extensions/              built-ins (each: manifest.json + index.ts [+ *.svelte])
  theme-default/ keymap-default/ effects-basic/ transitions-basic/
  toolbar/ viewer/ timeline/ inspector/ mods/
src/main/extensions/
  discovery.ts      scan dirs, read/validate manifests
  compiler.ts       esbuild (+ svelte plugin) → <userData>/extensions-build/<id>/bundle.js
  registry.ts       enabled-state persistence, IPC (list/enable/disable/remove/reload/create)
  watcher.ts        fs.watch → rebuild → renderer reload event
src/shared/extensions.ts     manifest schema + IPC types (shared main/renderer)
```

## 3. Manifest

```jsonc
{
  "id": "vhs-effect",            // [a-z0-9-]{2,64}
  "name": "VHS effect",
  "version": "1.0.0",
  "apiVersion": 1,
  "description": "Adds a VHS tape effect with tracking noise.",
  "entry": "index.ts",           // .ts/.js; may import .svelte and relative files
  "contributes": ["effects"],    // informational; used by Mods panel + agent
  "replaces": ["timeline"],      // optional: ids of extensions this one supersedes
  "dependsOn": ["effects-basic"],// optional
  "forkedFrom": "timeline@1.4.0",// optional, set when the agent forks a built-in
  "author": "agent" | "user" | "powermove"
}
```

## 4. Kernel API (summary; full types in `kernel/api.ts`)

```ts
export default function activate(api: PowermoveAPI): void | Disposable | Promise<...>
```

- `api.panels.register(def)` — Svelte component or `build(el)`; `size/min/flush/…`
- `api.commands.register({id,label,category,run,when?})`
- `api.keybindings.bind(key, commandId)`; `api.keybindings.unbind`
- `api.effects.register({id,label,group,params,frag,passes?,keepOrig?})` — name-based uniforms `u_<param>`; kernel generates declarations and validates/compiles eagerly
- `api.transitions.register({id,label,params,frag})` — two inputs `u_from`,`u_to`,`u_prog`
- `api.theme.register({id,name,tokens,css?})` / `api.theme.activate(id)`
- `api.palette.registerProvider(fn)`; `api.menus.contribute(location, items)`; `api.status.register(item)`
- `api.project` — `get()`, `apply(commands, meta)`, `selection`, `time`, `on(event)` — thin typed façade over `PM.Edit` etc.
- `api.ui` — `toast`, `confirm`, `menu`, `modal`, plus kernel-provided control
  components and binding helpers (`api.ui.controls`)
- `api.storage` — per-extension namespaced KV
- `api.on(event, fn)` — `project:changed | selection | time | layout | extension:loaded …`
- `api.log`, `api.id`, `api.version`
- Everything returns `Disposable`; all registrations are auto-disposed on deactivate/reload.

Escape hatch: `api.host.pm` exposes the legacy `PM` object, typed `unknown`. It is
documented as "unstable, used by built-ins mid-migration; agent should prefer the
typed surface". This is what makes "control every part" true on day one without
waiting for the whole legacy tree to be re-expressed through the API.
`api.host.state` similarly exposes the renderer's `doc`, `sel`, `transport`, and
`perf` rune stores for built-in UI that has not yet moved to typed store facades.

## 5. Override model

Registries key by id. Later registration with the same id **replaces** (LIFO stack;
disabling the top restores the previous). `manifest.replaces` deactivates the named
built-ins while this extension is enabled; toggling it off reactivates them.
Load order: built-ins (fixed order) → user (alphabetical, honoring `dependsOn`) →
project.

## 6. Loading & isolation

- Main compiles user/project extensions with esbuild (`format: esm`, `bundle: true`,
  `external: ['powermove', 'svelte', 'svelte/*']`), Svelte via `svelte/compiler`.
  Output is served at `app://powermove/ext/<id>/bundle.js?v=<hash>` (existing CSP
  `script-src 'self'` covers it). Import map `powermove`/`svelte` → kernel-provided
  modules via `api` argument + `window.__powermove_svelte` shim.
- Renderer `import()`s the bundle inside try/catch; `activate` runs in try/catch;
  runtime errors thrown from an extension's callbacks are attributed by extension id
  (wrappers in `host.ts`). Two runtime failures in 10 s → auto-disable + toast
  *"<name> stopped working — Fix it / Turn off"*.
- Disabled/errored extensions never affect boot; state in store key `extensions`.
- **Explicit trust decision:** extensions are trusted code running in the app's
  main frame with access to the preload bridge (store, codex, save, capture).
  This is personal software: the user (or their agent) authors what runs.
  Extensions may use HTTP(S) requests and WebSockets through the renderer's
  `connect-src` policy; normal browser CORS rules still apply. This network
  permission is shared by the editor frame, not granted per extension.
  Navigation guards and `setWindowOpenHandler` deny escapes, and the compile
  step rejects imports outside the extension folder. We deliberately do not
  sandbox extensions; error isolation, auto-disable, and one-click revert are
  the recovery model. The separate generated-script sandbox retains
  `connect-src 'none'`.
- Hot reload: watcher → rebuild → `extensions:changed` → renderer deactivates,
  re-imports (cache-busted), reactivates.

## 7. Agent contract

- Codex `project` workspace **is** `<userData>/extensions/` (plus `inputs/` for the
  project snapshot). Instructions include `docs/EXTENSIONS.md`, `kernel/api.d.ts`
  (generated by the `build:api-types` script at the time of this design; the shipped
  API pack is now copied straight from `src/renderer/src/kernel/api.ts` by
  `electron-builder.yml`), and the manifest of a built-in as example.
- Result schema gains `extensions: [{id, action: created|updated|removed}]`; the app
  reloads those ids and surfaces compile/activation errors back into the conversation.
- "Fix it" sends the error, stack, and file list of the failing extension as a prompt.
- Forking: agent copies `src/extensions/<id>` (shipped inside the app bundle at
  `resources/builtin-extensions/<id>/`, readable) into the user dir with
  `replaces` + `forkedFrom`.

## 8. Update policy

App bundle is read-only. Kernel `apiVersion` is checked at load; mismatched
extensions load if `apiVersion <= current` (additive changes) and are flagged
"needs update" on breaking bumps. Forks show "replaces built-in X (forked from vN)".

## 9. Migration plan (each phase leaves `main` green)

1. Kernel contracts + registries + host (additive; legacy still boots everything).
2. Route existing registration through registries: `PM.registerPanel`, `PM.commands`,
   keydown handler → `keybindings` registry, `PM.FX` → `effects` registry, theme → `theme`
   registry. Legacy callers unchanged; kernel is now the source of truth.
3. Main-process loader: discovery, compiler, IPC, watcher, `app://…/ext/` route.
4. Renderer loader + Mods panel + recovery UX.
5. Move built-ins into `src/extensions/*` (effects-basic, theme-default,
   keymap-default, toolbar, mods) and route the remaining panels through the
   kernel while their source is migrated.
6. Transitions: model + compositor pass + registry + commands + inspector + timeline mark.
7. Agent integration: workspace relocation, instructions, result schema, Fix-it.
8. Boundary lint (`scripts/check-boundaries.mjs`): `src/extensions/*` may import only
   `powermove`, svelte, own files. Docs: `docs/EXTENSIONS.md`.
9. Move the viewer, timeline, and inspector sources into forkable built-in
   extensions, exposing shared kernel controls and rune-store escape hatches
   through the versioned API.

## 10. Non-goals (this round)

npm dependencies inside extensions; extension marketplace; sandboxing extensions
(they are trusted code — personal software).
