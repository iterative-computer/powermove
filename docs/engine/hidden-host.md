# Native hidden MCP host

On macOS, `powermove mcp` reads the launch-time Use New Engine setting from the
profile and selects `server/native/mcp.ts` when enabled. Host services run inside
the CLI process. `server/native/host.ts` embeds the production `DocumentHostPool`,
with one process, pm-wasm document, kernel, extension registry and Undo history
per project. The host persists worker edits before answering tools.

The interactive and job `pm-node` children use the desktop `NativeEngine`
supervisor, restart ledger, attribution, backoff, host capabilities, budgets,
font service and Blender supervision. `main/engine/engine-runtime.ts` is shared
with Electron's utility process; `server/native/ports.ts` transports its existing
port protocol over Node IPC. The native bridge supplies the same host clock and
shared pixel storage. Job definitions and CPU snapshot definitions have separate
owners and increasing generations.

## Media, jobs and persistence

Agent imports stream into the host's immutable content store, use the desktop's
sampled fingerprint plus the full content hash, and ask the job engine to probe
the stored object. Numeric audio facts retain channel counts for named and custom
layouts. The production agent harness keeps validation, cancellation, locked
references and transactions; the worker import adapter commits the shared asset
identity and preserves replacement bytes for Undo. Workers resolve availability
through host media paths. Existing CLI flat media copies are promoted into host
objects when first needed, without removing their source files.

Frames and all agent exports use `AgentJobs`, pinned to the worker's revision
and winning definitions. Save uses the host-services PMV4 packer, including
original media and Undo history. Project-specific run owners and media bindings
keep an export independent of another project or tab.

## Chromium and shader pins

`LazyNativeBrowser` starts Chromium only for DOM tools, a compatibility raster
request, or an ANGLE shader request. A DOM page opens the session's project,
joins its remote project session and flushes edits before answering. Workspace
tools invoke the production dispatcher, retaining persisted layout, workspace,
viewport and the complete built-in/user panel registry. They do not return the
worker's reduced workspace answer. The page's compositor output is replaced by
CPU snapshots from `pm-node` over authenticated binary HTTP requests.

The desktop writes `desktop-environment.json` beside `Engine Cache/Shader Cache`.
Hidden engines and CLI renders read this namespace and the same `angle-parity`
pins. A profile without desktop probes uses a fresh namespace: user programs
must pass through ANGLE before native verification. The browser module uses
`ShaderFallbackRunner`, with the desktop's F16 texture, uniform and watchdog
contract. `powermove render` starts an authenticated loopback service; direct
bundled `pm-cli` starts the same Node helper and closes it on stdin EOF.

Missing native addons are `native-runtime-unavailable`; missing Chromium is
`chromium-unavailable` at the first browser-dependent call. Corrupt persisted
probe data is `shader-probe-store-unreadable`. The enabled host has no blanket
refusal and no Chromium composition fallback. Linux and the setting-off macOS
host keep the previous behavior.

## Validation commands

Build the matching native wrappers, pm-wasm and CLI host first. Run these under
`pm-electron-lock`, with Playwright workers set to one:

- `node apps/desktop/scripts/test-mcp.mjs --desktop`
- `node apps/desktop/scripts/test-mcp.mjs --native`
- `node apps/desktop/scripts/test-mcp.mjs --missing-browser`
- `node apps/desktop/scripts/test-native-hidden-parity.mjs`
- `node apps/desktop/scripts/test-native-hidden-services.mjs --launchd`
- `node apps/desktop/scripts/test-native-hidden-services.mjs --ssh`
- In `apps/desktop`, set `POWERMOVE_NATIVE_HOST_DIST` to the built CLI `dist`
  and run `bunx playwright test e2e/document-host-contract.spec.ts --workers=1`.

The parity harness captures a real desktop runner pin, compares every decoded
PNG frame with the hidden export, and compares complete workspace JSON bytes
with today's hidden host, including a user panel. The contract gate compares
answers and document bytes after every call, using only the existing deterministic
id normalization. Service tests create temporary user-domain launchd jobs and a
loopback-only SSH service with temporary keys outside the repository; they do
not change login or authorized-key settings.

## Remaining integration gates

The current integration branch does not yet include the extension-compatibility
lane's raster override/context producers and corresponding snapshot events.
The host's on-demand compatibility relay is wired, but those projects require
that integration and parity validation; this lane does not claim their pixels
are proved. Native importer format parity for animated images, SVG metadata and
models also needs the corresponding media/extension integrations and corpus
coverage. Desktop-encrypted extension variables and on-device captions are not
yet provided by this embedder.

## macOS artifacts

`packages/cli/scripts/assemble-native.mjs` consumes `pm-node-addon`,
`pm-bridge-addon` and `pm-cli` release artifacts, matching LGPL FFmpeg, and the
native image-decoder license directory. It checks architecture, relocates and
signs dylibs, constructs/ticks/pumps `EngineHost` on clean Node 22, and launches
`pm-cli`. ARM64 and x64 require separate FFmpeg/native libraries; x64 loads under
Rosetta on an ARM64 machine. The FFmpeg cross build uses the same pinned sources,
component selection and LGPL policy as ARM64. These commands never publish.
