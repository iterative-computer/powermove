# Writing Powermove extensions

Powermove is a kernel plus extensions. The built-in editor (timeline, effects,
theme, keymap, panels) is itself a set of extensions. You extend or replace any
of them by writing a new extension — never by editing the app bundle.

## Where extensions live

| Location | Scope |
|---|---|
| `~/Library/Application Support/Powermove/extensions/<id>/` | user (all projects) |
| `<project>/.powermove/extensions/<id>/` | project |
| app bundle `resources/builtin-extensions/<id>/` | built-in, **read-only** — copy to fork |

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
| `name`, `version` (`x.y.z`), `apiVersion` (`1`) | yes | |
| `description` | recommended | shown in the Mods list; one sentence |
| `entry` | no | default `index.ts`; `.ts` `.js` `.mjs`; may import relative `.ts`, `.js`, `.svelte`, `.css` |
| `contributes` | recommended | subset of `panels commands keybindings effects transitions layers themes palette menus status hooks` |
| `replaces` | no | ids of extensions to deactivate while this one is enabled (e.g. `["timeline"]`) |
| `dependsOn` | no | ids that must be enabled and load first |
| `forkedFrom` | no | `"<id>@<version>"` when copied from a built-in |
| `author` | no | `powermove` \| `user` \| `agent` |

Imports allowed: `powermove` (types only), `svelte`, `svelte/store`, relative files
inside the extension folder. No npm packages, no `..` escapes.

## The API (apiVersion 1)

Full types: `powermove.d.ts` (next to this file). Summary:

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
- **extensions** — introspection: `list`, `setEnabled`, `remove`, `reload`, `reveal`, `requestFix`.
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

**Replace a built-in** — copy `resources/builtin-extensions/timeline` to `~/…/extensions/my-timeline`, set `"replaces": ["timeline"]` and `"forkedFrom": "timeline@<version>"` in the manifest, then edit. Turning your mod off brings the built-in back.

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
Keep controls source-connected and undoable. An explicit user style request
overrides this default only for the requested surface.

`--accent --bg-window --bg-panel --bg-panel-2 --bg-sunken --bg-field --tx --tx-2 --line --r-base --f-ui --f-mono --row-h --ctl-h --fs-md --dur-2 --ease`

## Rules the kernel enforces

- Errors in `activate` → extension is disabled with the message shown in Mods; the app keeps running.
- Two runtime errors within 10 s → auto-disabled.
- `apiVersion` newer than the app → not loaded (“needs update”).
- Edits go through the typed boundary: locked layers and hand-edited channels are respected.

## For the agent

When asked to change Powermove itself: create or edit an extension under the user
extensions directory (your working directory). Prefer the smallest shape —
contribute → override by id → fork with `replaces`. Return the ids you created or
changed in `extensions` so the app reloads them. If the app reports a build or
activation error, fix the extension; do not work around by touching the app bundle.
