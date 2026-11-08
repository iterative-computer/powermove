# Extending Powermove

Everything in Powermove is an extension, including the built-in panels. You can
add a panel, change a built-in one, or replace it, by writing an extension and
publishing it. The app loads it while it runs; there is no rebuild or restart.

## The loop

1. `start_session` gives you `stagingDirectory`: a private copy of the user's
   extensions. Nothing there runs until you publish.
2. Write or edit one folder per extension in `stagingDirectory`. The folder name
   is the manifest `id`.
3. `publish_extensions` with every folder you changed:
   `{"extensions":[{"id":"hello-panel","action":"created","summary":"..."}]}`.
   `action` is `created`, `updated` or `removed`. Powermove checks your report
   against the files, scans for credentials, compiles each extension and runs
   it once in a throwaway runtime. If anything fails, nothing is applied and the
   error says what to fix; the stage stays as you left it.
4. On success the extensions are live in every open window within a second.
   The result's `changeSetId` names this change; the user can undo it with
   **Undo Extension Change** in the command palette. The result also has a fresh
   `stagingDirectory` for the next change.

Verify on screen with `get_panel_layout`, `capture_panel` or `render_frames`
when available, and say honestly what you could not check.

## Manifest

```json
{ "id": "hello-panel", "name": "Hello", "version": "1.0.0", "apiVersion": 4,
  "entry": "index.tsx", "author": "agent" }
```

- `id`: 2–64 characters, `a-z`, `0-9`, `-`. `version`: `x.y.z`.
- `apiVersion` 4 is the native API below. Versions 1–3 are the shipped
  extension API, documented in `v3/EXTENSIONS.md` with its types in `v3/`
  (Svelte panels, effects, transitions, layer types, commands, keybindings,
  menus, themes, status items). They load and publish the same way. Use them
  for context menus and effects: v4 has no registration API for those yet.
- `replaces`: built-in ids this extension draws in place of (forks).
- Credentials never go in source: declare `vars` and read them at runtime.

## A v4 extension

The entry is TypeScript/TSX and exports one of:

- `view()`: a component. It becomes a panel named after the manifest, listed
  in the Window menu. See `samples/hello-panel`.
- `mount(env)`: the full panel entry that built-in panels use. `env` is
  `BuiltinEnv` in `v4/builtin.ts`: the host client (`env.host`: queries,
  subscriptions, undoable `edit`/`command`), `env.native` operations of the
  panel, `env.platform` requests (menus, focus), and `env.root()` for a surface.
  Return `{ root, dispose, settle?, show?, publish? }`.

An optional `activate({ host, id, hot, commands, themes })` runs before
`view()` renders.

Commands: `commands.register({ id, title, key }, run)` adds `title` to the
command palette and, with `key` (for example `"cmd+shift+n"`), to the keymap.
`run(args)` executes in your extension; it returns a function that removes the
command. Register in `activate` (in `mount(env)` use `env.commands`).

Themes: `themes.register({ id, title, tokens })` lists "Theme: <title>" in the
command palette; choosing it restyles the window chrome and every panel.
`tokens` maps design tokens to values: colours as `"#rrggbb"`
(`"colors.accent"`, `"colors.panel"`, …), lengths and alphas as numbers
(`"metrics.gap"`, `"translucency.panel"`, …). Unknown tokens are refused. It
returns a function that removes the theme; a chosen theme follows reloads.

State survives a publish only through `hot` (in `mount(env)` it is `env.hot`).
Each publish runs the new version in a fresh runtime. The old version first
calls its `hot.dispose` callbacks, and what they put in `hot.data` is the new
version's `hot.data` (keep it JSON). Register `dispose` once, in `activate`,
not on every render; `samples/hello-panel` shows the pattern.

Imports: relative files inside the extension folder, plus `preact`,
`preact/hooks`, `powermove/hooks`, `powermove/ui` (controls: `Row`, `Section`,
`NumField`, `SliderField`, `SelectField`, `ToggleField`, `Segmented`, `Menu`,
`Glass`, …; these draw inside your panel, so `ToolbarItem` does not reach the
window toolbar), `powermove/runtime`, `powermove/protocol`,
`powermove/host-api`, `powermove/components`, `powermove/memo`,
`powermove/renderer` and `powermove/builtin`. There is no DOM, `window`,
`document` or Node; JSX produces native elements.

Elements: `div`, `text`, `img`, `input`, `textarea`, `button`, `scroll`,
`virtual-list`, `anchored`, `path`, `canvas`, `frame`, `markdown`, `code`,
`Glass`, `material`. Text goes in `<text>` as one string. Styles are flex/grid
objects with token references, for example `{ padding: 8, gap: { token:
"metrics.gap" } }`. Unknown properties are reported, never fatal; a render
that throws keeps the last tree.

## Changing a built-in panel

`fork_builtin_extension {"id":"timeline"}` copies the shipped source of a
built-in into the stage as `timeline-fork`, with `replaces: ["timeline"]` and a
pristine copy in `.forked-from/` (the merge base; do not edit it). Edit the
fork, then publish it with action `created`. The fork draws the built-in's
panel in its place, with the same data and actions. Built-in ids: `timeline`,
`inspector` (Properties), `layer-effects` (Effect Controls), `viewer`,
`assets` (Project), `fxbrowser` (Effects), `agent`, `settings`, `export`.

To remove a fork, delete its folder and publish it as `removed`: the built-in
comes back. When the app ships a newer built-in, `stage_fork_rebase {"id":
"timeline-fork"}` merges the upstream changes into your fork in the stage and
lists `conflicts` to resolve by hand (`baseDir` is the old base, `oursDir` the
new built-in). Then publish it as `updated`.

Prefer, in order: a new extension; a change to an existing user extension; a
fork of a built-in.
