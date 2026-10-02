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
| `vars` | no | `apiVersion: 2`; up to 32 declarations `{ key, label, secret?, hint? }`. Keys use uppercase letters, digits and underscores, starting with a letter. Read values through `api.vars`; never put credentials in source. |
| `permissions` | no | `apiVersion: 3`; see [Permissions and the sandbox](#permissions-and-the-sandbox) |
| `links` | no | `apiVersion: 3`; up to 5 https origins, written exactly as `"https://example.com"`, that `ui.openExternal` opens without asking, when the extension also declares `network` and the person just acted. The Store lists them. See [Opening links and importing from a URL](#opening-links-and-importing-from-a-url). |
| `author` | no | `powermove` \| `user` \| `agent` |

Imports allowed: `powermove` (types only), the client-side Svelte modules listed in
[Writing UI with Svelte 5](#writing-ui-with-svelte-5), relative files inside the
extension folder. No npm packages, no `..` escapes.

## Permissions and the sandbox

Store extensions from other publishers run sandboxed. Declare `apiVersion: 3` for
new Store-bound extensions and list the access they need in `manifest.json`
(`permissions` may be an empty array). `permissions` requires `apiVersion: 3`,
so code at `apiVersion` 1 or 2 can declare nothing: in the sandbox it cannot use
any permission, and publishing it with such a use is blocked until it sets
`apiVersion: 3` and declares the permission.

```json
"permissions": ["network", "assets"]
```

Reading the project needs no permission, at any `apiVersion`: `project.get`,
`project.latest`, `project.selection`, `project.snapshot` (a rendered frame),
`project.time`, `project.playing`, `project.revision`, the transport reads, and the
`project:changed`, `selection`, `time` and `transport` events are available to
every sandboxed extension. Without `network`, what it reads cannot leave the
sandbox.

- `network` allows HTTPS and WebSocket requests and remote images, media, fonts
  and stylesheets. It is the capability that lets project data leave, so it is
  the one to scrutinize. Without it, `fetch` still reads `data:` and `blob:`
  URLs and bundled `data:` fonts load, since neither leaves the machine.
- `clipboard` allows `ui.copy(text)`, which writes plain text (up to 500,000
  characters, at most once a second) from one of the extension's panels while
  that panel has focus and within 5 seconds of a click or key press in it (a
  modifier key alone does not count, and neither does focus coming back to the
  window, as after Command-Tab, a press on the app around the panel, or input
  from Powermove's agent). The runtime never copies. No permission lets an
  extension read the clipboard. `document.execCommand('copy')` in a panel is
  no way around it: Chromium lets it write only right after a click or key
  press in that panel, the same kind of gate.
- `assets` allows picking, importing, and reading asset files. `assets.importUrl`
  also needs `network`.
- `project:write` allows project mutation through `apply`, `undo`, `redo`, `select`, time and transport controls. `commands.run` can call an extension's own commands and, with this permission, the named legacy editing commands. It cannot call another extension's commands or File, app, export, settings, or mods commands.
- `full-access` allows trusted-only APIs. Store installs that request it stay off
  until the person installing them accepts Powermove's full-access dialog. They
  can later revoke trust from the Library.

Forms submit to their own `submit` handlers, and the sandbox then cancels the
navigation. A request the sandbox blocks is logged once and never turns the
extension off; the Sandbox compatibility check still reports it.

Store extensions supply simple event names; the kernel publishes them as `ext:<extension-id>:<name>`. They may subscribe to those events and the validated read-only host events `project:changed`, `selection`, `time`, `transport`, `theme`, and `extensions:changed`. Registration IDs must start with `<extension-id>.`, and keybindings may invoke only their own commands. Store code can list extensions and call `setUp` for itself; management of other extensions requires a trusted extension.

### Opening links and importing from a URL

`await api.ui.openExternal(url)` opens an https URL in the person's browser and
resolves `true`, or `false` when they decline. The URL must be https, at most 2
KB, and carry no user name or password. One call may be pending and at most one
runs every 2 s; others reject with `code: 'resource_limit'`. Nothing opens
while Powermove's window is in the background.

- An origin listed in the manifest's `links` opens without asking only when the
  extension also declares `network` and the call answers something the person
  just did: from one of the extension's panels while it has focus, within 5
  seconds of a click or key press in it, or from a command, status item,
  palette, menu or toast item the person started, within 5 seconds of starting
  it (not from the extension's own `commands.run`, a keybinding pressed in its
  own panel, a timer, an event, or input from Powermove's agent). At most 3
  links a minute open this way. Origins match exactly: listing
  `https://example.com` covers neither `https://www.example.com` nor another port.
- Every other call, and every call when the extension lacks `network`, opens
  only after the person confirms a Powermove sheet that names the extension by
  its id (not its display name) and shows the whole URL. Without `network`, a
  link is the one way data could leave, so it always asks.
- After the person declines, calls that would ask reject with
  `code: 'resource_limit'` for 30 seconds.

```json
"permissions": ["network"],
"links": ["https://replicate.com"]
```

`await api.assets.importUrl(url)` downloads an image, video or audio file and
imports it like `api.assets.import`, resolving to the new asset id. It needs
`assets` and `network`, and one download runs at a time. Powermove fetches it
itself, so CORS does not apply, but:

- the URL is https on the default port, at most 2 KB, without credentials; no
  cookies or `Authorization` are sent;
- the host must resolve to public internet addresses only (never loopback,
  private, link-local, CGNAT, unique-local or IPv4-mapped addresses), and the
  connection goes to the address that was checked;
- at most 5 redirects are followed, each one https and checked the same way;
- the file may be at most 512 MiB, must be PNG, JPEG, GIF, WebP, AVIF, BMP,
  MP4, MOV, WebM, MP3, AAC, M4A, WAV, Ogg or FLAC by its contents (not its
  name or `Content-Type`), and must decode. The asset is named after the last
  path segment with the extension of what the bytes are;
- a download counts toward the same 2 GiB a minute as `assets.import`, and one
  that would pass it rejects with `code: 'resource_limit'` before it is
  imported.

The desktop app provides `importUrl`; `powermove serve` does not.

The runtime sandbox is the security boundary. The **Sandbox compatibility check** is a compatibility lint: it runs a short, detectable sample of extension behavior to find problems before publishing. A pass is not a security review or trust signal.

### Isolation and the watchdog

Each sandboxed extension runs in its own process, separate from the editor and
from other extensions. Its runtime and its panel views load from an origin of
their own, which serves only the sandbox document and that extension's files. An
infinite loop or out-of-memory crash stops only that extension.

The kernel pings every sandboxed runtime every 2 s. A runtime that leaves a
ping unanswered for 8 s is unresponsive: Powermove ends its process, turns the
extension off with a runtime error, and shows a notice. A crashed process is
handled the same way. The extension stays off until it is turned on again, so
split long synchronous work into chunks that return to the event loop.

`powermove serve` loads every sandbox document from a single origin, so process
isolation there is whatever the browser provides, and there is no watchdog: one
spinning extension would stall its siblings too, and none could be ended. It derives Store trust from the
desktop provenance file and applies the same sandbox document and CSP. A Store
install requesting `full-access` remains off as “needs trust”; trust it from the
desktop app first, since serve has no trust dialog.

### Project data in the sandbox

The sandbox has no live project. Each sandboxed document keeps a small state
(time, playing, revision and the selection) that the host updates when it
changes, and reads the project on request:

- `await api.project.get()` returns a snapshot. It is cached until the project
  changes: repeated calls resolve at once without asking the host, and
  concurrent calls share one request. One snapshot build per change is shared by
  every sandboxed extension in the window, so reading on each `project:changed`
  is cheap. In a component, `api.project.latest()` reads the same cache
  reactively (see [Reactive API reads](#reactive-api-reads)).
- The snapshot is deep-frozen; assignments throw. Edit through `api.project.apply`.
- The snapshot omits the edit log (`edits` is absent although the type declares
  it), asset fields whose names contain `blob` or `source`, and keys matching
  `token`, `secret`, `password`, or ending in `key` within `library` and `notes`,
  with the same rules inside each composition. Every extension sees the rest;
  keep credentials in extension variables or storage.
- A snapshot above 8 Mi characters of JSON makes `project.get()` reject until the
  project is smaller.

Events reach listeners inside the extension's document. The host sends the
occurrences of subscribed events together with the state change, at most once per
flush. Within one flush, `time` and `selection` deliver only their latest value,
repeated `project:changed` of the same `kind` arrive once, and other events keep
their order. The synchronous reads return the state as of the latest delivery.

Each extension is limited to 200 registrations, 2,000 live callback handles, 50 open panel views, 200 RPC messages/s, 1 MiB per RPC payload (a file passed to `assets.import` is not counted; imports, including `assets.importUrl` downloads, are capped at 512 MiB per file and 2 GiB a minute), 256 KiB of storage with keys at most 128 characters, and 50 logs/s. These limits apply to messages from the extension; data the host sends, such as project snapshots, is not limited by them.

### Trusted-only APIs and publishing

The trusted-only namespaces are `api.render`, `api.host`, `api.services`,
`api.inspector`, `api.anim`, `api.model`, `api.history`, `api.edit`,
`api.groups`, `api.uiState`, `api.dnd`, `api.workspace`,
`api.ui.controls`, `api.ui.modal`, `api.ui.menu`, `api.ui.drag`,
`api.ui.gesture`, `api.ui.mount`, `api.media.importFiles`,
`api.media.assets`, `api.media.audio`, and `api.media.fonts`.
Publishing scans direct uses of these names and of network and clipboard APIs,
and blocks undeclared permissions. Below `apiVersion: 3` every such use is
undeclared, and the repair says to set `apiVersion: 3` as well as declare the
permission. Project reads need no permission, so the scan ignores them. This
text scan does not detect destructured aliases or dynamic property access. Local extensions made or
forked on this Mac are trusted and keep working without permission declarations.

Extensions you make here run with full access. Anything you publish runs
sandboxed for other people unless it declares `full-access`. Run **Test in
Sandbox…** from the Library before publishing; the publish sheet runs the same
check and blocks a release that fails it.

## Writing UI with Svelte 5

Panels (`component`), Properties sections and anything passed to `api.host.mount`
are Svelte 5 components. `.svelte` files always compile in runes mode, and
`.svelte.ts` / `.svelte.js` modules compile as rune modules, so `$state`,
`$derived` and `$effect` also work in shared classes outside components.
`export let` and `$:` do not compile: use `$props()`, `$derived` and `$effect`,
`onclick` rather than `on:click`, and snippets with `{@render}` rather than slots.

### What you can import

| Module | Exports |
|---|---|
| `svelte` | `onMount`, `onDestroy`, `tick`, `untrack`, `flushSync`, `setContext`/`getContext`/`createContext`, `createRawSnippet`, `getAbortSignal`, … |
| `svelte/reactivity` | `SvelteMap`, `SvelteSet`, `SvelteDate`, `SvelteURL`, `SvelteURLSearchParams`, `MediaQuery`, `createSubscriber` |
| `svelte/reactivity/window` | `innerWidth`, `innerHeight`, `devicePixelRatio`, `online`, `scrollX`, `scrollY`, … |
| `svelte/transition` | `fade`, `fly`, `slide`, `scale`, `blur`, `draw`, `crossfade` |
| `svelte/animate` | `flip` |
| `svelte/easing` | `cubicOut` and the other easing curves |
| `svelte/motion` | `Tween`, `Spring`, `prefersReducedMotion` |
| `svelte/events` | `on` |
| `svelte/attachments` | `createAttachmentKey`, `fromAction` |
| `svelte/store` | `writable`, `readable`, `derived`, `get`, `fromStore`, `toStore` |

Type-only imports such as `import type { HTMLButtonAttributes } from 'svelte/elements'`
or `'svelte/action'` are erased and always fine. `svelte/server`, `svelte/compiler`,
`svelte/legacy` and `svelte/internal/*` are not available. Every import resolves to
the Svelte the app runs, in the editor and in the sandbox alike: bundles carry no
copy of Svelte, and contexts, transitions and reactivity share one runtime.

`Spring.set()` rejects its promise with `Aborted` when a newer target replaces
it. Set `spring.target` instead, or give `.then()` a rejection handler: in a Store
sandbox an unhandled rejection counts as a runtime error, and repeated runtime
errors turn the extension off.

### Reactive API reads

These reads are reactive: `api.project.time()`, `playing()`, `revision()`,
`selection()` and `latest()`; `api.transport.time()` and `playing()`;
`api.theme.active()` and `scheme()`. Read one in markup, `$derived` or `$effect`
and that reader re-runs when the value changes. Anywhere else (`activate`, an
event handler, a timer) it returns the current value and subscribes to nothing.
The subscription starts with the first reactive reader and ends with the last, so
code that never reads reactively costs nothing. Closing a panel keeps its component
mounted, with its state, for when it reopens, so its readers keep running while it
is closed. Do not wire `events.on` into `$state` for these values.

```svelte
<script lang="ts">
  import type { PanelProps } from 'powermove';

  let { api }: PanelProps = $props();
  const selected = $derived(api!.project.selection().layers.length);
</script>

<p>{selected} selected · revision {api!.project.revision()}{api!.project.playing() ? ' · playing' : ''}</p>
```

`api.project.latest()` is the project a panel can show right now:

```svelte
<script lang="ts">
  import { fade } from 'svelte/transition';
  import type { PanelProps } from 'powermove';

  let { api }: PanelProps = $props();
  // Derive the fields you show, not the project object (see below).
  const layers = $derived(api!.project.latest()?.layers.map(({ id, name }) => ({ id, name })));
</script>

{#if layers}
  {#each layers as layer (layer.id)}<p transition:fade>{layer.name}</p>{/each}
{:else}
  <p>Loading…</p>
{/if}
```

- In the editor (local and full-access extensions) it is the live project, the
  object `get()` returns, and it re-runs readers on every `project:changed`. It is
  the same object after each change, so `$derived(api.project.latest())` on its own
  never changes value: derive the fields you display, or read them in markup.
  Never mutate it; edit with `api.project.apply`.
- In the sandbox it is the newest snapshot this document has pulled: `undefined`
  until the first pull, then the deep-frozen copy `await project.get()` returns. A
  reactive read starts a pull when there is none for the current project and
  re-runs when it lands, and again after changes: at most once a frame, so a
  drag that edits on every pointer move lands as one copy a frame, ending with
  its last change. Handle `undefined`.

`project.get()` is unchanged: synchronous in the editor, a Promise in the sandbox.
Use it in `activate`, commands and status providers, where no component reads
reactively; `events.on` remains the way to react there.

### `time()` during playback

`project.time()` and `transport.time()` change every frame while playing, so a
reader of either re-runs every frame. That is right for a playhead readout, and it
is how the editor's own time display works. For anything heavier, read less often:

```svelte
<script lang="ts">
  import type { PanelProps } from 'powermove';

  let { api }: PanelProps = $props();
  // Whole seconds: the $derived re-runs each frame, the text changes once a second.
  const seconds = $derived(Math.floor(api!.project.time()));
  // Four times a second while playing; exact while paused or scrubbing.
  let shown = $state(0);
  $effect(() => {
    if (!api!.project.playing()) { shown = api!.project.time(); return; }
    const timer = setInterval(() => { shown = api!.project.time(); }, 250);
    return () => clearInterval(timer);
  });
</script>

<p>{seconds}s · {api!.util.tc(shown)}</p>
```

While playing, the effect reads only `playing()`; the interval's `time()` read is
outside any reactive context, so the component does no per-frame work at all.

### Editor and sandbox differences

| | Editor (local, full-access) | Sandbox (Store) |
|---|---|---|
| `project.get()` | the live project | a Promise of a frozen snapshot |
| `project.latest()` | the live project, the same object after each change | the newest pulled snapshot; `undefined` until the first |
| time, playing, revision, selection | kernel state as it changes | the document's pushed state, updated at most once per flush |
| `theme.active()`, `theme.scheme()` | kernel state | the host's theme push |
| `svelte/reactivity/window`, viewport `MediaQuery` | the app window | the panel's own frame |
| Module-level state | one module for all of the extension's panels | one module copy per open panel; share through `api.storage` or `api.events` |

Transitions, motion and `prefersReducedMotion` behave the same in both.

## The API (apiVersion 1–3)

Full types: `api.ts` (next to this file in the agent API pack). Summary:

### Sandbox API (apiVersion 3)

In a Store sandbox, these methods return Promises. Await them even when the
in-realm type in `api.ts` shows a synchronous result: `api.commands.run`,
`api.project.get/apply/select/setTime/play/pause/undo/redo/snapshot`,
`api.transport.setTime/play/pause/toggle/step`, `api.assets.get`,
`api.storage.get/set/delete`, `api.media.getImportDefaults`, `api.ui.icon`,
`api.panels.isOpen` (your own panel ids only) and `api.extensions.list`.
`apiVersion` 1 and 2 code gets the same Promises; the Sandbox check reports
code that uses one of their results without awaiting it. Methods already typed as asynchronous, such as `api.assets.pick/import/importUrl/readText` and `api.ui.confirm/openExternal`, remain asynchronous.
`api.project.revision/selection/time/playing/latest`, `api.transport.time/playing`
and `api.theme.active/scheme` stay synchronous, and are reactive in components.
Outside a component, react to events:

```ts
api.events.on('project:changed', async () => {
  const project = await api.project.get();
  render(project.layers);
});
```

| Sandbox-safe | Trusted-only (`permissions: ["full-access"]`) |
| --- | --- |
| `effects`, `transitions`, `layers`, `theme`, `keybindings`, `commands`, `palette`, `menus`, `status`, `panels` | `anim`, `model`, `groups`, `history`, `edit`, `inspector`, `render`, `uiState` |
| `assets`, `project`, `transport` time and controls, `storage`, `events`, `vars`, `util`, `ease`, pure `space3d` helpers | `selection` live graph helpers, `dnd`, `workspace`, `services`, `host`; live `space3d` methods |
| `media.registerImportDefaults/getImportDefaults`, `ui.toast/confirm/openExternal/icon/copy`, `extensions.list`, `log`, `onDispose` | `media.importFiles/assets/audio/fonts`, `ui.controls/modal/menu/drag/gesture/mount` |

Sandboxed panels render their `component` or `build` content in a separate
view iframe. `panels.header`, `panels.moveSlot`, and `panels.library.render`
are unavailable there; the host owns the panel chrome. A toast's `action` and
`onDismiss` run in the document that raised it, so a toast with either, or
with a `key`, closes when that document does (its panel closes or reloads, or
the extension is turned off). A `key` replaces only the extension's own
toasts. Declare `network`,
`clipboard`, `assets`, or `project:write` (with `apiVersion: 3`)
when using their corresponding capabilities. `full-access` installs run with the
in-realm API after the person installing the extension accepts the trust dialog.

- **panels** — `register({ id, title, component?, build?, size, min, flush, noscroll, headless })`, `open(id, dock?)` or `open(id, { dock, index })`, `close`, `isOpen`, `refresh`, `list`.
  `component` is a Svelte 5 component receiving `{ panelId, spec, api }` (see [Writing UI with Svelte 5](#writing-ui-with-svelte-5)). `build(body)` is the imperative alternative.
- **commands** — `register({ id, label, category, run, when? })`, `run(id, …args)`, `has`, `list`. Commands appear in the palette (⌘K) unless `when()` returns false. `when` may return a Promise (a Store extension's always does, over the port): the palette asks it on every open, and every other reader, `list()` included, gets a synchronous `when()` that returns its last answer (`true` before the first), so `!c.when || c.when()` filters.
- **keybindings** — `bind({ key, command, args?, inFields?, looseModifiers?, repeat?, priority? })`. Chords: `cmd+shift+k`, `space`, `shift+f9`, `alt+up`. Lower priority runs first; return `false` from the command to pass through. Repeated browser keydowns are ignored by default; set `repeat: true` only for continuous, repeat-safe actions such as frame stepping or nudging. Suppressed repeats do not prevent the browser's default behavior.
- **effects** — `register({ id, label, group, params, frag, passes?, keepOrig?, backdrop? })`.
  `group` is the category in the FX browser. Reuse an existing family: Blur & Sharpen, Light & Shadow, Color, Stylize, Distort or Generate. Add a new group only when none plausibly fits; never one per effect or a near-duplicate of an existing name.
  Write only the body of `main()`. Available: `v_st` (uv), `u_tex`, `u_res`, `u_texel`, `u_time`, `u_pass`, helpers `src() luma() noise() fbm() hash() rgb2hsv() hsv2rgb()`. Each param `k` is a uniform `u_<k>` (float, or vec3 for `type:'color'`). Output `o` (vec4).
  Set `backdrop: true` when the shader needs the already-composited pixels below
  the layer; the host binds those pixels as `u_backdrop` on every effect pass.
  Params are keyframable automatically and appear in the Effects browser + inspector.
- **transitions** — `register({ id, label, params, frag })`. Inputs `u_from` (frame so far), `u_to` (incoming layer), `u_prog` 0→1. Output `o`. Applied on a layer via its `transition` property (inspector or `set_layer` command with `{ transition: { type, dur, p } }`).
- **layers** — `register({ id, label, version, params, defaults?, renderer })` adds a programmable renderer with structured project instances. Fragment renderers use `{ kind:'fragment', fragment }`; mesh renderers use `{ kind:'mesh', assetField:'assetId' }` and resolve a durable OBJ model id from layer data. Projects store only the definition id, version, JSON data, and keyframe channels—not renderer code or expanded vertex arrays. Missing definitions/assets keep their data and show a placeholder.
- **assets** — `pick({ accept, multiple? })`, `import(file, { layerDefinition? })`, `importUrl(url)` (see [Opening links and importing from a URL](#opening-links-and-importing-from-a-url)), `get(id)`, and `readText(id)`. Imported files live in Powermove's durable media store and are embedded when the `.pmv` is saved. Use an asset id in structured layer data instead of storing binary or large text in the project JSON.
- **theme** — `register({ id, name, scheme, tokens, darkTokens?, css?, rootAttributes? })`, `activate(id)`, `active()`, `scheme()` (both reactive in components). Tokens are CSS custom properties (see "Theme tokens"). `css` may restyle anything.
- **palette** — `registerProvider(query => entries[])`. The provider may return a Promise; its entries appear when it settles, if the palette still shows that query.
- **menus** — `contribute(location, ctx => items[])`, `collect(location, ctx?)` (in a Store sandbox, only your own contributions); locations: `panel:context`, `layer:context`, `timeline:context`, `viewer:context`. The function runs on every open with that open's `ctx`; it may return a Promise, which the menu waits for at most 100 ms. `gather(location, ctx)` resolves every contribution for one open, for `ui.menu`; `collect` returns only the ones that answer synchronously and never asks a Store extension's. Titlebar extension shortcuts are retired; registered panels appear in the panel Library automatically.
- **status** — `register({ id, text: () => string|null, side?, onClick? })` for the status bar.
- **project** — `get()`, `latest()`, `revision()`, `apply(commands, meta?)`, `selection()`, `select()`, `time()`, `setTime()`, `play/pause/playing`, `undo/redo`, `snapshot(t?, maxWidth?)`. `latest()`, `revision()`, `selection()`, `time()` and `playing()` are reactive in components.
  `apply` takes the typed edit commands (`set_property`, `replace_keyframes`, `set_easing`, `set_expression`, `set_content`, `set_layer`, `set_composition`, `add_layer`, `delete_layers`, `reorder_layer`, `add_effect`, `remove_effect`, `set_effect`, `set_scene_parameter`, `add_marker`, `create_section`, `update_section`, `transform_layers`). Every apply is one undo step, validated, lock-aware.
- **anim** — channel evaluation, property/keyframe edits, easing, expression errors, animation versioning, and 2D transform matrices.
- **model** — property/keyframe/layer/project factories, model schema tables, current composition, layer lookups, `cloneLayer(layer)`, and `normalizeFill(value, fallback?)`.
- **selection** — live selection reads, mutation with legacy events/invalidation, selected-key resolution, and key-selection mode.
- **groups** — hierarchy queries, selection expansion, stack normalization, and pose-preserving reparenting.
- **transport** — time and playback (`time()` and `playing()` are reactive in components), stepping, quality/performance, preview resolution, and render/UI invalidation.
- **history** — raw transaction begin/commit/cancel, undo/redo, external entries, and transaction-aware selection history.
- **edit** — validated one-shot edits, gesture transactions, dispatch, cancel/rollback, and structural mutation.
- **media** — timing, file import, asset-to-layer commands, waveform drawing, runtime assets, and font loading.
- **media import defaults** — `registerImportDefaults({ anchor: { x: 0.5, y: 0.5 } })` sets normalized anchors for future image, video and SVG imports and asset-to-timeline additions. `getImportDefaults()` reads the active override. Registration is owned by the mod; unloading restores the prior default. Existing layers/keyframes and audio/3D origins are untouched.
- **inspector** — `registerSection({ id, title, after: 'content' | 'transform' | 'effects', when?, build })` adds controls to Properties. `build(target, { layerIds })` returns a disposer or cleanup function. Sections rebuild on selection changes, and disappear when the mod unloads. Use `sections()` to inspect active contributions.
- **render** — WebGL bounds/picking/setup and `gl.compileError(key)`, raster access, offscreen frame rendering, and snapshots.
- **uiState** — layer/FX disclosure, key handles, timeline reveal state, and shader metadata via `getShaderMeta(layer)` / `setShaderMeta(layer, patch)`.
- **ui** — API-backed controls, overlays, menus, pointer drag, parent picking, shader editor opening, edit/history-backed `gesture` construction, and `openExternal(url)` for https links.
- **dnd** — canonical asset/FX MIME payloads, drag detection/parsing, live media drag state, and FX drop application.
- **workspace** — active workspace mutation plus indexed panel add/move/hide/restore/refresh operations.
- **util** — numeric interpolation/snapping, timecode, ids, and colour conversion.
- **ease** — easing preset lookup and handle-name matching.
- **space3d** — PM-bound 3D transforms, perspective planes, projection, inversion, and containment.
- **services** — LIFO typed runtime service registration; disposing an override restores the previous implementation.
- **storage** — per-extension `get/set/delete` (persisted).
- **events / on** — `project:changed`, `selection`, `time`, `transport`, `fonts` (complete family list), `layout`, `theme:changed`, `frame:rendered`, `extension:loaded/unloaded`.
  An extension's own events use names without `:`: sandboxed, `emit('pinned', value)` reaches `on('pinned')` in every document of that extension only, and a name containing `:` is refused.
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
Read project and time values reactively within the mounted component (see
[Reactive API reads](#reactive-api-reads)) so they update without rebuilding the
section on every keystroke.

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

**Add a panel (Svelte)** — for manifest id `counter`, `Counter.svelte` + `api.panels.register({ id:'counter.panel', title:'Counter', component: Counter, size: 160 })`, then `api.panels.open('counter.panel','right')`. Write the component with runes and reactive API reads; see [Writing UI with Svelte 5](#writing-ui-with-svelte-5).

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

## Native panel design

Unless the user's prompt explicitly asks for a different style, every new or
modified panel must look like a built-in Powermove panel: same layout, controls,
spacing, type and states. A generated panel that looks hand-made is a defect,
just like a build error. An explicit user style request overrides this default
only for the surface it names.

**Reuse before you build.** Open the built-in panel closest to the job with
`get_panel_state`/`capture_panel` and copy its structure. Let
`api.panels.register` supply the frame, header, docking and scrolling. Wherever
an existing component covers the need, use it: `api.ui.controls` (`Row`,
`Section`, `TextField`, `NumField`, `SelectField`, `ToggleField`, `ColorField`,
`FillField`, `FontField`), `api.ui.menu`, `api.ui.modal`, `api.ui.icon` and
`api.ui.toast`. Hand-write a control only when no existing component fits or the
user asks for it. Sandboxed Store panels cannot use the trusted-only components.
They must still reproduce the native control's look and behavior with theme
tokens, not invent a new one.

**Match the native look.**
- Use the theme tokens for color, radius, type, row and control height
  (`--row-h`, `--ctl-h`, `--fs-md`, `--r-base`, `--f-ui`). Do not hardcode
  colors or sizes, and do not add a palette of your own.
- Lay out label and control rows like Properties. Labels sit in `--tx-2`;
  values and controls align to the same column.
- Buttons are flat tonal fills or transparent. Do not add outlines, bevels,
  gradients or raised shadows. A choice among a few options is a segmented
  control on a sunken track, with the selected segment shown as a tonal fill.
  Never use separate outlined pills or an accent-colored ring for it.
- Group content with spacing and `--bg-panel-2`/`--bg-sunken` fills, not
  hairline dividers, nested cards or a second title bar.
- Leave out helper captions, attribution footers, status sentences and
  implementation details. Report results in the content itself or with
  `api.ui.toast`. Put recovery actions inside the error surface they act on.
- Inherit light and dark themes, and keep hover, selected, disabled and keyboard
  focus states visible.
- Keep controls source-connected and undoable.

Before you finish, capture the panel next to a built-in panel at a normal and a
narrow width and fix every visible difference. See
[the interface language](design-language.md).

## Theme tokens (subset; the full set is `@powermove/tokens/tokens.css`)

`--accent --bg-window --bg-panel --bg-panel-2 --bg-sunken --bg-field --tx --tx-2 --line --r-base --f-ui --f-mono --row-h --ctl-h --fs-md --dur-2 --ease`

## Variables

An extension that needs an API key, account id or similar value declares it and
reads it at runtime. It never carries the value in its source.

- **Declare** each value in `manifest.json` with `apiVersion: 2`:
  `"vars": [{ "key": "OPENAI_API_KEY", "label": "OpenAI API key", "secret": true }]`.
  `hint` is optional; `secret` masks the field. Users can save any subset, including
  no values. Legacy `required` flags are ignored and never block activation.
  Handle missing values at runtime: support alternative credentials or fallback
  behavior where possible, and explain which value to enter in Set Up when an
  operation actually needs it.
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
- **Shared realm.** Local and full-access extensions run in the editor's renderer,
  which holds every extension's values, so any of them can read a value delivered to
  another. A sandboxed Store extension receives only its own. Treat values as
  belonging to the Powermove profile.
- `powermove serve` hosts do not support variables yet.

## Rules the kernel enforces

- Errors in `activate` → extension is disabled with the message shown in Mods; the app keeps running.
- Two runtime errors within 10 s → auto-disabled.
- A sandboxed extension that stops answering for 8 s, or whose process crashes → its process is ended and it is turned off.
- A sandboxed extension that has not activated within 10 s → not loaded; if it also stopped answering (it is spinning), its process is ended and it is turned off instead, with one notice.
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
