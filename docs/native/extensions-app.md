# Extensions in the app

Status of the running app's extension support. Measured results live in
[extcheck-p2.md](extcheck-p2.md); the realm internals in
[compat-realm.md](compat-realm.md).

## Path

`pm_compat_runner::host::ExtensionHost` is created per project window by
`Controller::start_extensions`. One owner thread does everything that can wait:

1. **Discover**: immediate children of `extensions-shared` (the shipped app's
   folder as first launch migrated it, kept current by each launch's delta pass),
   then the native profile's `extensions`. Later roots win. The shipped app's live
   folder is never read. A manifest error stays a visible record.
2. **Compile**: `compile.rs` applies the frozen compiler's source boundary, strips
   TypeScript with oxc, compiles Svelte in a compiler realm and caches graphs by
   source digest. There is no bundler: V8 links the graph as real ES modules.
3. **Activate**: `js/compat/src/loader.ts` plans and activates with the reference
   loader's order, timeouts and auto-disable. v1 to 3 share one realm; Store
   installs each get a confined helper; v4 extensions get their own realm.
4. **Publish**: contributions go to kernel registries, so Effects, Properties,
   MCP and layer commands see them as they see built-ins, and engine definitions go
   to every viewer and export through `ResourceSnapshot`.
5. **Follow**: kernel publications update the realm mirror, Settings toggles and
   on-disk edits reload, and native undo reaches external closures through the
   realm mailbox.

The main thread only enqueues and reads published copies (N8).

## Kernel channel

`adapter.rs` serves the realm from `KernelHandle`. Reads carry facts only. A write
publishes the stored change's operations mapped onto the realized open composition;
only a composition switch or a whole-root write sends a document. The first
document is serialized before the realm starts and parsed with `JSON.parse`
(a pro project can be hundreds of megabytes).

`PM.proj` is a view over the realm's mirror that records which records were
written; a boundary diffs and settles only those subtrees. Records an extension
builds and inserts are compared slot by slot, since it may keep writing through
its own reference. Replacing the whole project falls back to whole-tree diffs. `transport.rs`
keeps `PM.time` a lock and a copy; the app's clock owner pushes time and receives
seek, play and pause.

## DOM panels

The realm has one document, so the window has one WebKit page below GPUI. A
contributed panel's body is transparent in GPUI (`Integration::native_panel`, and
the shell's window fill leaves it out), the page shows through and takes pointer
events there, and the realm positions the panel's root at the shell's rectangle.
The root stays hidden until placed; the extension builds into a body inside it,
which it may style. The page carries the compat theme edition for the window's
appearance, so `--tx`, `--bg-field` and the rest resolve as in the shipped app. Recorder canvases reach the page as immutable
PNG resources. Inspector sections share a contributed `extension-properties` panel,
because the v4 Inspector has no DOM slot.

## Failure

A failed compile, throw or export leaves one record with its reason. Each
extension activates in its own realm task. A task that runs for eight seconds gets
the D11 sheet ("isn't responding — Wait / Stop"), named for the extension that was
activating or whose command is running; the task keeps running meanwhile. Stop
terminates it and rebuilds the realm; a hang during activation also turns that
extension off in state. Two restarts in one session stop the realm until the project
reopens. The rebuild is a new writer generation: the old one is sealed
(`RealmFailed`), and its page is replaced and its open panels reopened. A panel whose
extension reloads is parked (`panel:park`) and returns to its tab.

The trusted realm queues fetches that do not fit its external-memory budget
(`Limits::queue_fetches`) and always leaves room for one synchronous engine reply,
so a panel that starts dozens of requests cannot starve the kernel channel.
A fatal V8 process abort is not contained:
the trusted realm is in-process, by decision 5.

## Native answers

Menus, sheets and links belong to AppKit, so the realm asks and carries on. A
request reaches `Controller::extension_request` on the main thread and its answer
returns through the host's inbox, resolving the realm's promise or running the menu
item's closure. `ui.confirm`, `ui.menu`, `ui.openExternal`, `panels.open` and
`panel:context` contributions take this path. A tab menu waits 150 ms for
contributions, then opens without them.

`api.storage` is the kernel's `ext.<id>` settings, the keys the shipped app wrote,
so values survive a restart and are read by a later window.

A panel whose definition disappears leaves the layout and returns if the id is
registered again, as in the shipped app. The kernel's workspace record is not
touched: native layout lives in the shell.

## Store

Each Store extension gets a confined helper and its own WebKit document. Panels are
built in the helper against a remote DOM; only batches reach the page. Settings
toggles start and stop the helper; a rescan restarts only helpers whose source moved.
A Store fetch reaches public HTTPS hosts only, across redirects, and drops credentials
on a cross-origin redirect. The palette's Install Extension from Store appears only
once a Store key is pinned; until the Store signs releases there is none (see
[store-host.md](store-host.md)).

## Agent authoring

An agent changes the app through extensions, as on main. `start_session` copies
every extension folder (later roots win) into a run-private stage and writes the
API pack (`js/v4/api-pack`) to `api/`. `publish_extensions` checks the declared
changes against a snapshot of the stage, scans text for credentials, then
`extension_authoring::check` compiles each one with the native compiler and
activates it on a synthetic project: v1–3 in a trusted realm, v4 in a panel
isolate, a fork in place of its built-in. Promotion into the profile's folder
goes through a journal in `<profile>/agent-change-history/<project>` that a crash
rolls back at the next start; `rescan_all` reloads every window at once. The
palette's **Undo Extension Change** restores the newest change set.

Built-in panels mount through `panels/<id>/mount.tsx` and the `BuiltinEnv`
contract (`src/builtin.ts`). The build writes each built-in's source as an
extension tree (`scripts/builtin-sources.ts`, shared imports become
`powermove/*`), which the app writes once to `<profile>/builtin-extensions/`.
`fork_builtin_extension` copies one with a `.forked-from` base;
`stage_fork_rebase` merges a newer built-in line by line. A v4 extension whose
`replaces` names a built-in runs in that panel's realm (`Builtins::replace`) with
the built-in's routes; removing it restores the built-in. A reloaded v4
extension gets the previous version's `api.hot.data`. A panel published while a
window is open is shown in it.

`pm-mcp/src/extensions` holds the transaction, scanner and three-way merge;
`app/src/extension_authoring_tests.rs` drives the agent flow headless.

## Known limits

- Images do not decode in the realm (`Image.decode`, `naturalWidth`, `drawImage` of an
  image), so checks such as Commons Logos' transparency test fail.
- `PM.proj` hands out views: a project record read back is a view (its identity is
  stable), not the stored object. `structuredClone` accepts views; other natives that
  refuse proxies would not.

- `render.raster`, `renderFrameTo` and `snapshot` have no engine backend, and raster
  overrides do not reach the viewer.
- `api.services` holds only what extensions register; the built-in `viewer`,
  `timeline`, `inspector`, `tool` and `shaderHooks` services do not exist natively.
- `ui.modal` and `PM.Fonts` throw `Unavailable`.
- `layer:context`, `timeline:context` and `viewer:context` have nowhere to appear,
  because those native surfaces have no context menus. `titlebar:right` has no host.
- Contributed layer types render and label their timeline row, but their parameters
  have no Properties rows until per-kind controls are designed.
- Layout reads in the realm are cached from the last page report (`remote-dom.md`).
- A v4 extension adds a panel, replaces a whole built-in panel, and registers
  palette commands with shortcuts (`commands.register`). It cannot yet slot into
  part of another panel (a viewer overlay or timeline column without a fork), add
  menus or toolbar items; v1–3 extensions can add menus through the compat realm.
- Each window reserves 64 MiB per isolate for up to 64 isolates, so many windows
  can together reserve past the shared 4 GB cage; idle isolates are not parked.
- Store panels have no recorder canvases, and `ui.*` from a helper is not attached.

## v4 parity with v1–3 (2026-11-08)

What a v1–3 extension could change and where v4 stands, without forking:

| v1–3 could | v4 now |
|---|---|
| Commands, keybindings, panels | `commands.register`, `view()` / `mount(env)` |
| Themes: tokens and CSS (`theme.register`, `theme.activate`) | `themes.register` / `themes.activate` (tokens). v1–3 CSS themes reach native chrome through the translator; surface rules beyond tokens aren't applied natively yet. |
| Context menus (`menus.contribute`) | `menus.contribute` for panel, layer, timeline and viewer; `titlebar:right` not yet |
| Viewer overlays (drawing on `ViewerService.ov` or the stage DOM) | `overlays.viewer` (display only) |
| Timeline columns and painting (`TimelineService.cv`, gutter DOM) | Open: needs v4 outline columns and P-TEXTURE painters (TL-PAINT-1 ≤ 2 ms for 1,000 clips) |
| Titlebar/toolbar items (`titlebar:right`, titlebar DOM) | Open: needs an extension slot in the window toolbar |

Agent flows (publish → visible → revert) run with `scripts/agent-flows.py`.
