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
- Store panels have no recorder canvases, and `ui.*` from a helper is not attached.
