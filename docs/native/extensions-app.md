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
that was activating; two restarts in one session stop the realm until the project
reopens. Kernel edits were sealed by then. A fatal V8 process abort is not contained:
the trusted realm is in-process, by decision 5.

## Known limits

- Raster overrides do not reach the viewer: `render.raster`, `renderFrameTo` and
  `snapshot` have no engine backend, so `services.get('raster')` wrappers run
  only when called directly.
- `api.services` holds only what extensions register; the built-in `viewer`,
  `timeline`, `inspector`, `tool` and `shaderHooks` services do not exist natively.
- Layout reads in the realm are cached from the last page report (`remote-dom.md`).
- Menu contributions are collected but no native context menu shows them.
- Store views have no WebKit host.
