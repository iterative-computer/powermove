# Writing Powermove extensions

Powermove is a kernel plus extensions. The built-in editor (timeline, effects,
theme, keymap, panels) is itself a set of extensions. You extend or replace any
of them by writing a new extension — never by editing the app bundle.

## Choose the requested contribution

A request for an effect means a registered effect in **Effects & Presets**, with
keyframeable parameters in the Inspector. Use `api.effects.register`; adding a
panel, a script button, or a layer rig does not fulfill an effect request.
Applying an existing effect is a separate `add_effect` project command. Panels
are appropriate when requested or needed for a separate workflow; the panel
examples and style rules below do not imply that every extension needs one.

Start effect authoring with the complete [Gradient Tint sample](samples/gradient-tint/README.md).
Call the agent tool `validate_effect` with the complete definition before
returning staged changes. It uses the real registration validator, including
the **32-parameter maximum**, unique keys matching `/^[a-z][a-zA-Z0-9]*$/`,
1–8 passes, and a non-empty fragment limited to 65,536 characters. It does not
register or render the effect. After loading, check `get_workspace_state` for
extension health and `registeredEffects`, then verify the requested rendering.

## Where extensions live

| Location | Scope |
|---|---|
| `~/Library/Application Support/Powermove/extensions/<id>/` | user (all projects) |
| `<project>/.powermove/extensions/<id>/` | project |
| app bundle `resources/builtin-extensions/<id>/` | built-in, **read-only** — fork with `api.extensions.fork(id)` or `fork_builtin_extension` |

The folder name must equal the manifest `id`. The app watches the user directory:
saving a file rebuilds and hot-reloads the extension.

## Minimal extension

`~/…/extensions/hello/manifest.json`
```json
{
  "id": "hello",
  "name": "Hello",
  "version": "1.0.0",
  "apiVersion": 1,
  "description": "Adds a greeting command.",
  "contributes": ["commands", "keybindings"],
  "author": "agent"
}
```

`~/…/extensions/hello/index.ts`
```ts
import type { PowermoveAPI } from 'powermove';

export default function activate(api: PowermoveAPI) {
  api.commands.register({
    id: 'hello.greet',
    label: 'Say hello',
    category: 'Fun',
    run: () => api.ui.toast('Hello from an extension')
  });
  api.keybindings.bind({ key: 'cmd+shift+h', command: 'hello.greet' });
}
```

Everything you register is released automatically when the extension is disabled,
reloaded, or removed. Return a `Disposable` or use `api.onDispose` for anything else
(timers, DOM outside panels).

## Manifest fields

| Field | Required | Notes |
|---|---|---|
| `id` | yes | `a-z 0-9 -`, 2–64 chars, equals folder name |
| `name`, `version` (`x.y.z`), `apiVersion` (`1`, `2`, or `3`) | yes | |
| `description` | recommended | shown in the Mods list; one sentence |
| `entry` | no | default `index.ts`; `.ts` `.js` `.mjs`; may import relative `.ts`, `.js`, `.svelte`, `.css` |
| `contributes` | recommended | subset of `panels inspector media commands keybindings effects transitions layers themes palette menus status hooks` |
| `replaces` | no | ids of extensions to deactivate while this one is enabled (e.g. `["timeline"]`) |
| `dependsOn` | no | ids that must be enabled and load first |
| `forkedFrom` | no | `"<id>@<version>"` for a built-in or `"<handle>/<id>@<version>"` for a Store fork |
| `vars` | no | `apiVersion: 2`; up to 32 declarations `{ key, label, secret?, required?, hint? }`. Keys use uppercase letters, digits and underscores, starting with a letter. Read values through `api.vars`; never put credentials in source. |
| `author` | no | `powermove` \| `user` \| `agent` |

Imports allowed: `powermove` (types only), `svelte`, `svelte/store`, relative files
inside the extension folder. No npm packages, no `..` escapes.

## Permissions and the sandbox

Store extensions from other publishers run sandboxed. Declare `apiVersion: 3` for
new Store-bound extensions and list the access they need in `manifest.json`:

```json
"permissions": ["network", "assets", "project:write"]
```

- `network` allows HTTPS and WebSocket requests and remote images and media.
- `clipboard` allows writing to the clipboard.
- `assets` allows picking, importing, and reading asset files.
- `project:write` allows project mutation through `apply`, `undo`, `redo`, `select`, time and transport controls, and host commands. Without it, `commands.run` can call only commands registered by that extension. Permission errors name the missing permission.
- `full-access` allows trusted-only APIs. Store installs that request it stay off
  until the person installing them accepts Powermove's full-access dialog. They
  can later revoke trust from the Library.

Store extensions may subscribe to their own `<extension-id>:*` events and the read-only host events `project:changed`, `selection`, `time`, `transport`, `theme`, and `extensions:changed`; they may emit only their own events. Registration IDs must start with `<extension-id>.` or `<extension-id>-`, and keybindings may invoke only their own commands. Store code can list extensions and call `setUp` for itself; management of other extensions requires a trusted extension.

The project mirror is shared data visible to every sandboxed Store extension: it contains the full project except asset blob/source fields and keys matching `token`, `secret`, `password`, or ending in `key` within `library` and `notes`. Keep credentials in extension variables or storage. A mirror above 8 MiB makes `project.get()` throw until the project is smaller. Each extension is limited to 200 registrations, 2,000 live callback handles, 50 open panel views, 200 RPC messages/s, 1 MiB per RPC payload, 256 KiB of storage with keys at most 128 characters, and 50 logs/s.

`powermove serve` derives Store trust from the desktop provenance file and applies the same sandbox document and CSP. A Store install requesting `full-access` remains off as “needs trust”; trust it from the desktop app first, since serve has no trust dialog.

The trusted-only namespaces are `api.render`, `api.host`, `api.services`,
`api.inspector`, `api.anim`, `api.model`, `api.history`, `api.edit`,
`api.groups`, `api.uiState`, `api.dnd`, `api.workspace`,
`api.ui.controls`, `api.ui.modal`, `api.ui.menu`, `api.ui.drag`,
`api.ui.gesture`, `api.ui.mount`, `api.media.importFiles`,
`api.media.assets`, `api.media.audio`, and `api.media.fonts`.
Publishing scans direct uses of these names and network or clipboard APIs and
blocks undeclared permissions. This text scan does not detect destructured
aliases or dynamic property access. Local extensions made or forked on this Mac
are trusted and keep working without permission declarations.

## The API (apiVersion 1)

Full types: `api.ts` (next to this file in the agent API pack). Summary:

- **panels** — `register({ id, title, component?, build?, size, min, flush, noscroll, headless })`, `open(id, dock?)` or `open(id, { dock, index })`, `close`, `isOpen`, `refresh`, `list`.
  `component` is a Svelte 5 component receiving `{ panelId, spec }`. `build(body)` is the imperative alternative.
- **commands** — `register({ id, label, category, run, when? })`, `run(id, …args)`, `has`, `list`. Commands appear in the palette (⌘K).
- **keybindings** — `bind({ key, command, args?, inFields?, looseModifiers?, repeat?, priority? })`. Chords: `cmd+shift+k`, `space`, `shift+f9`, `alt+up`. Lower priority runs first; return `false` from the command to pass through. Repeated browser keydowns are ignored by default; set `repeat: true` only for continuous, repeat-safe actions such as frame stepping or nudging. Suppressed repeats do not prevent the browser's default behavior.
- **effects** — `register({ id, label, group, params, frag, passes?, keepOrig? })`.
  Write only the body of `main()`. Available: `v_st` (uv), `u_tex`, `u_res`, `u_texel`, `u_time`, `u_pass`, helpers `src() luma() noise() fbm() hash() rgb2hsv() hsv2rgb()`. Each param `k` is a uniform `u_<k>` (float, or vec3 for `type:'color'`). Output `o` (vec4).
  Params are keyframable automatically and appear in the Effects browser + inspector.
- **transitions** — `register({ id, label, params, frag })`. Inputs `u_from` (frame so far), `u_to` (incoming layer), `u_prog` 0→1. Output `o`. Applied on a layer via its `transition` property (inspector or `set_layer` command with `{ transition: { type, dur, p } }`).
- **layers** — `register({ id, label, version, params, defaults?, renderer })` adds a programmable renderer with structured project instances. Fragment renderers use `{ kind:'fragment', fragment }`; mesh renderers use `{ kind:'mesh', assetField:'assetId' }` and resolve a durable OBJ model id from layer data. Projects store only the definition id, version, JSON data, and keyframe channels—not renderer code or expanded vertex arrays. Missing definitions/assets keep their data and show a placeholder.
- **assets** — `pick({ accept, multiple? })`, `import(file, { layerDefinition? })`, `get(id)`, and `readText(id)`. Imported files live in Powermove's durable media store and are embedded when the `.pmv` is saved. Use an asset id in structured layer data instead of storing binary or large text in the project JSON.
- **theme** — `register({ id, name, scheme, tokens, darkTokens?, css?, rootAttributes? })`, `activate(id)`. Tokens are CSS custom properties (see "Theme tokens"). `css` may restyle anything.
- **palette** — `registerProvider(query => entries[])`.
- **menus** — `contribute(location, ctx => items[])`; locations: `panel:context`, `layer:context`, `timeline:context`, `viewer:context`. Titlebar extension shortcuts are retired; registered panels appear in the panel Library automatically.
- **status** — `register({ id, text: () => string|null, side?, onClick? })` for the status bar.
- **project** — `get()`, `revision()`, `apply(commands, meta?)`, `selection()`, `select()`, `time()`, `setTime()`, `play/pause/playing`, `undo/redo`, `snapshot(t?, maxWidth?)`.
  `apply` takes the typed edit commands (`set_property`, `replace_keyframes`, `set_easing`, `set_expression`, `set_content`, `set_layer`, `set_composition`, `add_layer`, `delete_layers`, `reorder_layer`, `add_effect`, `remove_effect`, `set_effect`, `set_scene_parameter`, `add_marker`, `create_section`, `update_section`, `transform_layers`). Every apply is one undo step, validated, lock-aware.
- **anim** — channel evaluation, property/keyframe edits, easing, expression errors, animation versioning, and 2D transform matrices.
- **model** — property/keyframe/layer/project factories, model schema tables, current composition, layer lookups, `cloneLayer(layer)`, and `normalizeFill(value, fallback?)`.
- **selection** — live selection reads, mutation with legacy events/invalidation, selected-key resolution, and key-selection mode.
- **groups** — hierarchy queries, selection expansion, stack normalization, and pose-preserving reparenting.
- **transport** — time, playback, stepping, quality/performance, preview resolution, and render/UI invalidation.
- **history** — raw transaction begin/commit/cancel, undo/redo, external entries, and transaction-aware selection history.
- **edit** — validated one-shot edits, gesture transactions, dispatch, cancel/rollback, and structural mutation.
- **media** — timing, file import, asset-to-layer commands, waveform drawing, runtime assets, and font loading.
- **media import defaults** — `registerImportDefaults({ anchor: { x: 0.5, y: 0.5 } })` sets normalized anchors for future image, video and SVG imports and asset-to-timeline additions. `getImportDefaults()` reads the active override. Registration is owned by the mod; unloading restores the prior default. Existing layers/keyframes and audio/3D origins are untouched.
- **inspector** — `registerSection({ id, title, after: 'content' | 'transform' | 'effects', when?, build })` adds controls to Properties. `build(target, { layerIds })` returns a disposer or cleanup function. Sections rebuild on selection changes, and disappear when the mod unloads. Use `sections()` to inspect active contributions.
- **render** — WebGL bounds/picking/setup and `gl.compileError(key)`, raster access, offscreen frame rendering, and snapshots.
- **uiState** — layer/FX disclosure, key handles, timeline reveal state, and shader metadata via `getShaderMeta(layer)` / `setShaderMeta(layer, patch)`.
- **ui** — API-backed controls, overlays, menus, pointer drag, parent picking, shader editor opening, and edit/history-backed `gesture` construction.
- **dnd** — canonical asset/FX MIME payloads, drag detection/parsing, live media drag state, and FX drop application.
- **workspace** — active workspace mutation plus indexed panel add/move/hide/restore/refresh operations.
- **util** — numeric interpolation/snapping, timecode, ids, and colour conversion.
- **ease** — easing preset lookup and handle-name matching.
- **space3d** — PM-bound 3D transforms, perspective planes, projection, inversion, and containment.
- **services** — LIFO typed runtime service registration; disposing an override restores the previous implementation.
- **storage** — per-extension `get/set/delete` (persisted).
- **events / on** — `project:changed`, `selection`, `time`, `transport`, `fonts` (complete family list), `layout`, `theme:changed`, `frame:rendered`, `extension:loaded/unloaded`.
- **extensions** — introspection: `list`, `fork`, `rebase`, `setEnabled`, `remove`, `reload`, `reveal`, `requestFix`, `setUp` (opens the values sheet).
- **vars** — `get(key)`, `has(key)`, `keys()`: the values the user entered for this extension's declared `vars` (see "Variables").
- **model.cloneLayer** — `cloneLayer(layer): Layer` deep-clones a layer and refreshes its layer/keyframe ids and numbered name.
- **model.normalizeFill** — `normalizeFill(value, fallback?): Fill` canonicalizes solid, gradient, radial, and empty fills.
- **uiState.getShaderMeta** — `getShaderMeta(layer): ShaderMeta | null` reads the compositor metadata cached for a layer.
- **render.gl.compileError** — `compileError(key): string | null` reads the latest shader compilation diagnostic for a program key.
- **events.fonts** — `on('fonts', families => …)` receives the complete ordered font-family list whenever the catalogue changes.

### Deprecated

`api.host.pm` and `api.host.state` exist for compatibility with older user
extensions only. They are unstable, untyped escape hatches; new code must use the
typed namespaces above. Both forms, plus the standalone `PM` identifier, fail the
built-in extension boundary lint.

## Patterns

### Global import defaults and Properties controls

These are supported extension capabilities; do not inject controls into another
panel's DOM or rewrite already imported layers to implement a future default.
Register defaults during activation so enabled user mods restore them in each
editor window and after restart. Store configurable choices in `api.storage`.

```ts
import type { PowermoveAPI } from 'powermove';
import AnchorPresets from './AnchorPresets.svelte';

export default function activate(api: PowermoveAPI) {
  api.media.registerImportDefaults({ anchor: { x: 0.5, y: 0.5 } });
  api.inspector.registerSection({
    id: 'anchor-presets', title: 'Anchor presets', after: 'transform',
    when: ({ layerIds }) => layerIds.length > 0,
    build(target, { layerIds }) {
      // Mount a Svelte component through the shared runtime; returning its
      // unmount function ties it to both selection changes and mod unloading.
      return api.host.mount(AnchorPresets, target, { api, layerIds });
    }
  });
}
```

This mod may declare `contributes: ["media", "inspector"]` in its manifest.

Import anchors use normalized visual bounds (`0`, `0.5`, `1` for the nine
presets). The editor compensates position so new content does not jump. Raster
image/video coordinates are centered, so their center anchor is **0 px**, not
half the source width. SVG content can have asymmetric bounds. Audio and model
imports keep their native nonvisual/3D origins. Disposing a default registration
changes only future imports; imported layers retain their own editable channels.

Custom Properties controls must use `api.project.apply` or `api.edit` for edits.
For an explicitly requested anchor change, set `preserveHandEdits: false` and
compensate position with `api.anim.localMatrix` (or `api.space3d` for 3D).
Preserve animation; do not flatten keyframe channels. Layer locks still apply.
Subscribe to project/time events within the mounted component when its values
need to update without rebuilding on every keystroke.

### Finding an existing capability before declaring it unsupported

- Read the current API types and use `get_workspace_state` for actual extension
  health/errors and registered effects; use `render.gl.compileError` for shader
  diagnostics. Do not infer a registration failure from a mock host.
- Use `select_layers` with IDs from `get_project_state` to operate selection-based
  panels. Extensions can use `api.selection.select` or `api.project.select`.
- Use `group_layers`, `ungroup_layers`, and `move_to_group` for editable groups.
  Inspect their documented semantics before substituting a group for a requested
  nested composition; they are different structures.
- Use `api.commands.list`, `api.panels.list`, and the typed namespaces for existing
  workflows. When a built-in needs deeper changes, `fork_builtin_extension`
  supplies its source in the isolated stage; contribute or override before forking.
- Validate effects with `validate_effect`. Mod completion checks manifests and
  compiles staged source before publication. A rejected change report includes
  the actual staged IDs/actions; correct the report or revert unintended edits.
  Native providers can correct these pre-publication failures twice in the same
  stage. Never replay an already completed project edit or external action.

### Panels that import media and drop onto the timeline

See the complete, commented [Media Browser sample](samples/media-browser/README.md).
It fetches each source URL as a `Blob`, wraps it in a named `File`, and calls
`api.assets.import(file)`. That is the durable asset API; `api.media.importFiles`
is the higher-level choice when files should be imported and placed immediately.

Asset drags carry `{ id, name, kind, dur? }` under `api.dnd.ASSET_MIME`. FX drags
carry `{ kind: 'effect' | 'transition', id, label }` under `api.dnd.FX_MIME`.
Call `api.dnd.startAssetDrag(event.dataTransfer, payload)` instead of spelling
the MIME string yourself. Set `api.dnd.mediaDrag = payload` for the duration of
an in-app asset drag so the timeline can render its preview during `dragover`,
then clear it on `dragend`.

Place a panel at a stable dock position with:

```ts
api.panels.open('media-browser', { dock: 'left', index: 0 });
```

Calling `open` from `activate()` is fine for a first reveal, but it runs on
every launch. The host therefore ignores an activation-time `open` for a panel
the user has closed: their layout wins, and the panel comes back only through
an explicit open later (a command, a menu item, a button). Don't try to work
around this; a panel that reappears after being closed reads as a bug.

Asset import and project history are deliberately separate. The imported asset
stays in the media library; dropping it on the timeline creates the undoable
layer edit. `api.project.undo()` is the simple project façade. `api.history.undo()`
targets the same history stack and returns whether an entry was undone; use the
rest of `api.history` only when bracketing lower-level mutations yourself.

**Add an effect**
```ts
api.effects.register({
  id: 'vhs', label: 'VHS', group: 'Stylize',
  params: [
    { k: 'tracking', label: 'Tracking', def: 0.3, min: 0, max: 1, step: 0.01 },
    { k: 'tint', label: 'Tint', def: '#ff88cc', type: 'color' }
  ],
  frag: `
    float band = step(0.98, fract(v_st.y * 40.0 + u_time * 2.0)) * u_tracking;
    vec2 uv = v_st + vec2(band * 0.02 * (noise(vec2(u_time, v_st.y * 10.0)) - 0.5), 0.0);
    vec4 c = texture(u_tex, uv);
    o = vec4(mix(c.rgb, c.rgb * u_tint, 0.15), c.a);`
});
```

**Add a panel (Svelte)** — `Counter.svelte` + `api.panels.register({ id:'counter', title:'Counter', component: Counter, size: 160 })`, then `api.panels.open('counter','right')`.

**Add a structured programmable layer**
```ts
api.layers.register({
  id: 'example.orb', label: '3D Orb', version: 1,
  params: [{ k: 'radius', label: 'Radius', def: 0.6, min: 0.1, max: 1 }],
  defaults: { objects: [{ id: 'orb', geometry: 'sphere' }] },
  renderer: {
    kind: 'fragment',
    fragment: `void main(){
      vec2 p=(uv-.5)*2.; float a=smoothstep(u_radius,u_radius-.01,length(p));
      fragColor=vec4(vec3(a),a);
    }`
  }
});

api.project.apply({
  type: 'add_layer', layerType: 'extension',
  content: { definition: 'example.orb', parameters: { radius: 0.72 } }
}, { label: 'Add 3D Orb' });
```
The definition is capability code. Every created layer remains ordinary structured `.pmv` data; its parameters appear in the Inspector and can be animated through `set_property` paths such as `x.radius`.

**Import an OBJ into a mesh layer**
```ts
const [file] = await api.assets.pick({ accept: '.obj,model/obj' });
if (file) {
  const asset = await api.assets.import(file, { layerDefinition: 'example.obj' });
  api.project.apply({
    type: 'add_layer', layerType: 'extension',
    content: { definition: 'example.obj', data: { assetId: asset.id } }
  }, { label: `Import ${file.name}` });
}
```
The bundled **3D Layers** extension provides this as **Import OBJ Model…**. Powermove triangulates polygon faces, supports positive/negative OBJ indices and supplied or generated normals, normalizes the mesh for a predictable camera, and keeps the original OBJ as the durable asset.

**Change the look** — a theme extension with `tokens` only (accent, backgrounds, radius) or with `css` for a full reskin. Windows 98 is `css` plus `rootAttributes`.

**Replace a built-in** — call `await api.extensions.fork('timeline')`, then edit the created user extension. Turning your fork off brings the built-in back.

**Updating a fork** — when Powermove ships a newer version of the built-in, call `api.extensions.rebase(id)` (or use the update notice). The agent stages the old pristine base, your fork, and the newly shipped source together, then performs a three-way merge that preserves your behavior and records the new `forkedFrom` version.

**Override just a piece** — don't fork; register the same panel/command/effect `id`. The latest registration wins; disabling yours restores the original.

## Theme tokens (subset; the full set is `@powermove/tokens/tokens.css`)

### Native panels by default

Unless the user's prompt explicitly requests a different style, new and modified
panels must match regular Powermove panels one to one. Let `api.panels.register`
supply the existing frame, header, docking, and scrolling. Reuse `api.ui.controls`
and the closest built-in panel's rows, fields, sections, buttons, and icons.
Match the same spacing, type sizes, label alignment, control heights, radii,
borders, and surfaces using the app's tokens. Inherit light/dark mode; do not
introduce a nested card, duplicate title bar, custom palette, or decorative UI.
Use flat tonal fills or transparent buttons without bevels, decorative gradients,
or raised shadows. Put recovery actions inside their borderless error surface.
Keep keyboard focus and selection indicators visible. See [the interface language](design-language.md).
Keep controls source-connected and undoable. An explicit user style request
overrides this default only for the requested surface.

`--accent --bg-window --bg-panel --bg-panel-2 --bg-sunken --bg-field --tx --tx-2 --line --r-base --f-ui --f-mono --row-h --ctl-h --fs-md --dur-2 --ease`

## Variables

An extension that needs an API key, account id or similar value declares it and
reads it at runtime. It never carries the value in its source.

- **Declare** each value in `manifest.json` with `apiVersion: 2`:
  `"vars": [{ "key": "OPENAI_API_KEY", "label": "OpenAI API key", "secret": true, "required": true }]`.
  `hint` is optional; `secret` masks the field; `required` keeps the extension off,
  shown as "Needs setup", until the value is set.
- **Read** with `api.vars.get('OPENAI_API_KEY')`, `has(key)` and `keys()`. Values are
  fetched once per activation; setting or removing one reloads the extension. Only
  declared keys are delivered.
- **Never in source.** When the agent saves an extension, its text files are scanned
  for credentials (provider key prefixes, JWTs, private keys, long high-entropy
  strings). A finding blocks the change and nothing is published. A long random
  string that is not a secret can be waived on its line with
  `// powermove-secret-ok: <reason>`; known key formats cannot be waived.
- **Values live outside the folder**, in the app profile at `env/<envKey>.env`
  (mode 600, secrets sealed with the macOS keychain through `safeStorage`). The user
  enters them in Settings › Extensions (Set up, or Variables… on the extension's
  page). Removing the extension asks whether to delete them too.
- **Shared realm.** Extensions run in one renderer, so a value delivered to one
  extension is readable by any running extension. Treat values as belonging to the
  Powermove profile.
- `powermove serve` hosts do not support variables yet.

## Rules the kernel enforces

- Errors in `activate` → extension is disabled with the message shown in Mods; the app keeps running.
- Two runtime errors within 10 s → auto-disabled.
- `apiVersion` newer than the app → not loaded (“needs update”).
- Edits go through the typed boundary: locked layers and hand-edited channels are respected.

## For the agent

When asked to change Powermove itself: create or edit an extension under the user
extensions directory (your working directory). Prefer the smallest shape —
contribute → override by id → `fork_builtin_extension`. Return the ids you created or
changed in `extensions` so the app reloads them. If the app reports a build or
activation error, fix the extension; do not work around by touching the app bundle.

### Raster overrides and verification

The host registers a `raster` service with the `RenderAPI['raster']` signature. Capture it with `api.services.get` before registering a wrapper under the same name. Delegate unaffected layers to the captured implementation; calling `api.render.raster` from inside the wrapper recurses. Native raster surfaces store their canvas in `cv`, with `w`, `h`, `anchorX`, `anchorY`, `selection` and a texture cache `key`. Preserve those fields when replacing pixels. The override affects preview and export and is removed automatically when the extension disposes.

Agent-authored extensions are staged until the agent returns them. After loading, the app continues the same agent task to inspect actual panels, rendered frames and runtime errors and repair failures. Each updated extension gets another verification pass, up to three follow-ups. Failed or incomplete verification is reported explicitly; no Fix it button is required. Undo restores the original change and its repair passes in reverse order.

### App update notices

Powermove records extension health by app version. After an app update, a previously
working custom extension that reports a build, manifest, activation, runtime, or
compatibility problem gets a persistent notice with **Review extensions**. Existing
fork update notices cover customizations based on older built-in versions. Notices
never automatically disable, delete, or rewrite an extension, and dismissals survive
restarts.

Release authors can declare that a built-in now includes custom functionality:

- `integrates: ["custom-extension-id"]` explicitly identifies an extension whose
  functionality has been incorporated. Only declarations on shipped built-ins count.
- `features: ["specific-stable-feature-id"]` declares precise capabilities on either
  kind of extension. Use the same identifiers only for equivalent functionality.
  All features declared by a custom extension must be covered by shipped built-ins
  before Powermove offers the inclusion notice; partial overlap is insufficient.

Do not use broad categories such as `panels`, extension names, or `forkedFrom` as
proof of equivalence. A fork may retain custom behavior that the app does not have.
Feature declarations are additive metadata and do not alter loading or replacement
rules. Maintainers must include accurate declarations in releases that adopt features.
