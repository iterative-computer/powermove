# Extending Powermove

You can add a panel or replace a built-in panel by writing and publishing an
extension. Powermove loads the extension while it runs.

For composition work, see [Leave the project ready for hand editing](HANDOFF.md).

## Start a new panel in its workspace slot

Before writing a new panel, read `get_panel_layout`, choose a sensible location,
and call `panel_begin` with `title`, `placement` and optional `planned` controls
(`{kind, label}`). Placement is `{dock: "left" | "right" | "bottom"}`,
`{beside: "panel-id"}` or `{tab_in: "panel-id"}`. Unsupported locations, including
`{dock: "floating"}`, fall back to the right dock. Use the returned placement in
your chat explanation and keep the returned `build_id`.

Send `panel_build {build_id, event: "plan", row, kind, label}` when planning a
control and `event: "commit"` only after committing its code or definition.
These events drive the slot's Light sketch animation. Publish normally, then
send `panel_build {build_id, event: "ready", panelId: "registered-panel-id"}`.
The slot hands off after the panel's first committed render. Do not separately
open or dock the finished panel: the reserved slot becomes that panel. On failure
or cancellation send `event: "error"` or `event: "cancel"` to close the slot.
This applies to both native generated panels and extension/webview panels.

## The loop

1. Call `start_session` with `includeExtensionStaging: true` to get
   `stagingDirectory`, a private copy of the user's extensions, and
   `apiPackDirectory`, the authoring guides and types. Nothing in the staging
   folder runs until you publish.
2. Write or edit one folder per extension in `stagingDirectory`. The folder name
   is the manifest `id`.
3. `publish_extensions` with every folder you changed:
   `{"extensions":[{"id":"hello-panel","action":"created","summary":"..."}]}`.
   `action` is `created`, `updated` or `removed`. Powermove checks your report
   against the files, scans for credentials, compiles each extension and runs
   it once in a throwaway runtime. It also compiles the registered effect shaders.
   If a check fails, Powermove keeps your staged files and reports what to fix.
4. On success the extensions are live in every open window within a second.
   The result's `changeSetId` names this change; the user can undo it with
   **Undo Extension Change** in the command palette. The result also has a fresh
   `stagingDirectory` for the next change.

Verify on screen with `get_panel_layout`, `capture_panel` or `render_frames`
when available, and say honestly what you could not check.

## Built-in effects

The read-only [builtin-effects.json](builtin-effects.json) lists the 43 built-in
effects, with their ids, parameters, passes, and GLSL. See
[v3/EXTENSIONS.md](v3/EXTENSIONS.md#built-in-effect-reference) for examples and
the shader authoring contract. The Effects browser panel's fork id is `fxbrowser`.

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
  for legacy effects, or register the same EffectDefinition with
  `effects.register(definition)` in v4 `mount({ effects })`. Registration
  returns a disposer and parameters use the native keyframe controls.
  Effect definitions can declare `viewerHandles` for points, lines, angles,
  radii and rectangles. See **Viewer handles** in `v3/EXTENSIONS.md` for
  bindings and shader-layer examples.
- `replaces`: built-in ids this extension draws in place of (forks).
- Credentials never go in source: declare `vars` and read them at runtime.

## A v4 extension

The entry is TypeScript/TSX and exports one or more of:

- `view()`: a component. It becomes a panel named after the manifest, listed
  in the Window menu. See `samples/hello-panel`.
- `mount(env)`: the full panel entry that built-in panels use. `env` is
  `BuiltinEnv` in `v4/builtin.ts`: the host client (`env.host`: queries,
  subscriptions, undoable `edit`/`command`), `env.native` operations of the
  panel, `env.platform` requests (menus, focus), `env.root()` for a surface,
  and all extension APIs below. Return `{ root, dispose, settle?, show?, publish? }`.
- `activate(ctx)`: runs on load (and works on its own for headless/chrome/tool/effect
  extensions with no `view()` or `mount()`). `ctx` exposes:
  `{ host, id, hot, api, commands, themes, menus, overlays, timeline, effects, transitions, layers, toolbar, panelHeader, menuBar, tools, slots, events, workspace, dialogs, fs, storage, net, clipboard, media, shell }`.

### Update-safe API contract

Check `api.version`, `api.capabilities`, or `api.has("fs")` for feature detection.
The v4 API is strictly additive across app releases. Prefer composable hooks
(`slots.contribute`, `panelHeader.contribute`, `toolbar.contribute`, `menuBar.contribute`,
`tools.register`, `menus.contribute`, `commands.register`, `themes.register`) over
forking built-in panels so extensions never need updating when Powermove updates.
Built-in forks (`replaces: [...]`) are also automatically 3-way rebased on app updates
against `.forked-from/` when upstream changes do not overlap your edits; if a true
conflict exists, your working fork is preserved untouched.

### Chrome & UI integration (Layers 1–3)

- **Commands**: `commands.register({ id, title, key }, run)` adds `title` to the
  command palette and keymap. Returns a disposer.
- **Window toolbar**: `toolbar.contribute({ id, label, symbol?, tooltip?, active?, disabled?, order? }, run)`
  adds a native macOS `NSToolbarItem` button to the window titlebar. Returns a disposer.
- **Panel headers**: `panelHeader.contribute(panelId, { id, label?, symbol?, tooltip?, active?, disabled?, order? }, run)`
  adds a trailing native button inside any panel's header (`"timeline"`, `"viewer"`,
  `"inspector"`, `"assets"`, `"fxbrowser"`, `"agent"`, or `"*"`). Returns a disposer.
- **macOS menu bar**: `menuBar.contribute(menuName, { id, label, shortcut?, checked?, disabled?, separatorBefore?, order? }, run)`
  adds items to an existing macOS menu (`"File"`, `"Edit"`, `"View"`, `"Layer"`, `"Playback"`, `"Window"`, `"Help"`)
  or creates a new top-level menu. Returns a disposer.
- **Live UI slots inside built-in panels**: `slots.contribute(slot, Component)` mounts a
  live Preact/native-UI component directly inside a built-in panel without forking it.
  Supported slots: `"inspector.top"`, `"inspector.bottom"`, `"timeline.toolbar"`,
  `"timeline.footer"`, `"viewer.hud"`, `"agent.toolbar"`, `"assets.toolbar"`. Returns a disposer.
- **Interactive viewer tools & overlays**:
  - `tools.register({ id, label, symbol?, shortcut?, cursor? }, { onActivate?, onDeactivate?, onPointerDown?, onPointerMove?, onPointerUp?, onWheel?, onKeyDown? })`
    registers a first-class interactive viewer tool receiving composition-space coordinates
    `{ x, y, compX, compY, scale, width, height, button, clickCount, modifiers }`.
  - `overlays.viewer(({ frame }) => …, { interactive?: true })` draws over the viewer
    composition (`frame` = `{ x, y, width, height, scale }`). Pass `{ interactive: true }`
    to receive pointer clicks/drags on overlay elements.
- **Themes & menus & timeline**:
  - `themes.register({ id, title, tokens })`, `themes.activate(id | null)`
  - `menus.contribute(location, (ctx) => items)` for `"panel:context"`, `"layer:context"`, `"timeline:context"`, `"viewer:context"`
  - `timeline.chrome({ rowHeight, gutterWidth, rulerHeight, keyframeSize, clipRadius, surfaceOrder })`
  - `timeline.paint((clips) => shapes)`

### Custom GPU shaders (Layer 4)

- `effects.register(definition)`: register a custom GLSL `EffectDefinition` (returns disposer).
- `transitions.register(definition)`: register a custom GLSL `TransitionDefinition` (returns disposer).
- `layers.register(definition)`: register a custom GLSL/procedural `LayerTypeDefinition` (returns disposer).

### Events, workspace, dialogs & native OS capabilities (Layers 5–6)

- **Events**: `events.on(event, handler)` for `"selection"`, `"document"`, `"playback"`, `"layout"`, `"tool"`, `"command"`, `"time"`. Returns disposer.
- **Workspace**: `workspace.openPanel(id)`, `workspace.closePanel(id)`, `workspace.focusPanel(id)`, `workspace.listPanels()`, `workspace.runCommand(name, args?)`.
- **Native macOS dialogs**: `dialogs.openFolder(opts?)`, `dialogs.openFiles(opts?)`, `dialogs.saveFile(opts?)`, `dialogs.confirm(opts)`, `dialogs.alert(opts)`.
- **Filesystem**: `fs.homeDir()`, `fs.listDir(path, { includeHidden? })`, `fs.stat(path)`, `fs.exists(path)`, `fs.readText(path)`, `fs.readBytes(path)`, `fs.writeText(path, text)`, `fs.writeBytes(path, base64)`, `fs.mkdir(path)`, `fs.rename(from, to)`, `fs.trash(path)`.
- **Persistent per-extension KV storage**: `storage.get(key)`, `storage.set(key, value)`, `storage.delete(key)`, `storage.keys()`, `storage.clear()` (persists across reloads & app restarts).
- **Networking, clipboard, media, shell**:
  - `net.fetch(url, { method?, headers?, body?, timeoutMs? })`
  - `clipboard.readText()`, `clipboard.writeText(text)`
  - `media.importFiles(paths)` (imports local files into the open project's media pool)
  - `shell.revealInFinder(path)`, `shell.openPath(path)`, `shell.openUrl(url)`, `shell.exec(program, args?, { cwd?, timeoutMs? })`

Example — native File Browser panel using `fs`, `dialogs`, `storage`, `media`, and `shell`:

```tsx
export function view() {
  return null; // or use mount(env) / activate(ctx) for direct access to ctx.fs / ctx.dialogs / ctx.storage
}
export function mount(env: BuiltinEnv) {
  // env.fs.homeDir(), env.fs.listDir(dir, { includeHidden: true }),
  // env.dialogs.openFolder({ showHidden: true }),
  // env.storage.get("lastDir"), env.storage.set("lastDir", dir),
  // env.media.importFiles([path]), env.shell.revealInFinder(path)
}
```

State survives a hot-reload publish through `hot` (`hot.dispose` / `hot.data`), and
persists across restarts through `storage`.

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

Effects may declare `convexQuad: ['tlX','tlY','trX','trY','brX','brY','blX','blY']`
when those eight numeric parameters are percentage corner offsets. The compositor
checks their evaluated values each frame. Concave, crossing, singular or mirrored
manual quadrilaterals report an effect definition error. Viewer frames bypass
that instance; deliveries refuse invalid pins.
