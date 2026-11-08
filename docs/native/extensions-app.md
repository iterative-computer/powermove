# Extensions in the app

Status of the running app's extension support. Measured results live in
[extcheck-p2.md](extcheck-p2.md); the realm internals in
[compat-realm.md](compat-realm.md).

## Path

`pm_compat_runner::host::ExtensionHost` is created per project window by
`Controller::start_extensions`. One owner thread does everything that can wait:

1. **Discover**: immediate children of the shipped app's extensions folder, then
   the native profile's. Later roots win. A manifest error stays a visible record.
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
only a composition switch or a whole-root write sends a document. `transport.rs`
keeps `PM.time` a lock and a copy; the app's clock owner pushes time and receives
seek, play and pause.

## DOM panels

The realm has one document, so the window has one WebKit page below GPUI. A
contributed panel's body is transparent in GPUI (`Integration::native_panel`), the
page shows through and takes pointer events there, and the realm positions the
panel's root at the shell's rectangle. Recorder canvases reach the page as immutable
PNG resources. Inspector sections share a contributed `extension-properties` panel,
because the v4 Inspector has no DOM slot.

## Failure

A failed compile, throw or export leaves one record with its reason. A task that
runs for eight seconds is terminated and the realm is rebuilt without the extension
that was activating, which stays disabled in state; two restarts in one session stop
the realm until the project reopens. The rebuild is a new writer generation: the old
one is sealed (`RealmFailed`), and its page is replaced and its open panels reopened.
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
- A v4 extension adds a panel or replaces a whole built-in panel. It cannot yet
  slot into part of another panel (a viewer overlay or timeline column without a
  fork), register commands, menus or keybindings, or add toolbar items; v1–3
  extensions can do the latter through the compat realm.
- Each window reserves 64 MiB per isolate for up to 64 isolates; idle isolates are
  not parked yet.
- Store panels have no recorder canvases, and `ui.*` from a helper is not attached.
