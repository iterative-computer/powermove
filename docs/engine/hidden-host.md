# Native hidden MCP host

On macOS, `powermove mcp` reads the launch-time Use New Engine setting from the
profile and selects `server/native/mcp.ts` when enabled. Host services run inside
the CLI process. `server/native/host.ts` embeds the production `DocumentHostPool`,
with one process, pm-wasm document, kernel, extension registry and Undo history
per project. The host persists worker edits before answering tools. Profile writes are
serialized, and shared project/trash indexes apply each worker’s changes against
the current host value, preserving projects created or renamed by other workers.
Workers refresh shared preferences before tools; profile snapshots omit unrelated
project documents and Undo histories. Committed preferences notify DOM clients.
Before the first join, project sessions replay the checkpoint’s ordered recovery
journal, matching `Projects.get`; recovery leaves the persisted checkpoint and
journal untouched. A native worker therefore receives the same recovered
document a browser brings to its session.

The interactive and job `pm-node` children use the desktop `NativeEngine`
supervisor, restart ledger, attribution, backoff, host capabilities, budgets,
font service and Blender supervision. `main/engine/engine-runtime.ts` is shared
with Electron's utility process; `server/native/ports.ts` transports its existing
port protocol over Node IPC. The native bridge supplies the same host clock and
shared pixel storage. Job definitions and CPU snapshot definitions have separate
owners and increasing generations. The interactive child receives a fresh,
authenticated pm-bridge rendezvous on every launch so its native document
presenter can serve headless CPU snapshots. It creates no viewer pools or
Chromium texture imports; a bridge failure restarts the child through the
existing supervisor.

## Media, jobs and persistence

A native DOM tab joins the worker document immediately but holds outgoing edits during extension boot. After definitions settle, it pulls the authoritative document again before enabling writes. Temporary hydration placeholders cannot mark a valid saved effect missing in the worker.

Agent imports run today's editor importer in the project's on-demand tab. The
tab uses the desktop's host media-store protocol: uploads finish only after the
host verifies their bytes, with the same content hashes, originals, conversions,
posters, SVG metadata and model records. The production agent harness retains
validation, cancellation, locked references and transactions. Its worker adapter
answers only after the tab's commit reaches the worker. Direct timeline imports
share today's hidden-host adapter: omitting `at` imports assets only; placement
uses registered import defaults and the editor's editable SVG and model branches.
Workers retain complete imported metadata and confirm availability through host
paths, including models. Existing CLI flat media copies are promoted into host
objects when first needed, without removing their source files.

Frames and all agent exports use `AgentJobs`, pinned to the worker's revision
and winning definitions. Contact-sheet deadlines cancel that batch alone and
return its completed cells with the remainder omitted. Save writes today's
PMV3 agent container, including available originals and Undo history; missing
media remains in the document and does not prevent saving. Planning waits for
pending imports, and `Projects.put` runs after the file is written. The packer
retains today's blob MIME types, frozen with each save's content bindings.
Project-specific run owners and media bindings keep jobs independent.

## Chromium and shader pins

`LazyNativeBrowser` starts Chromium only for DOM tools, a compatibility raster
request, or an ANGLE shader request. A DOM page waits for document and complete
extension boot, opens the session's project, then rejoins the authoritative
remote project session before accepting tools. This discards an empty or stale
local document restored by asynchronous tab boot, even when the project ID
already matches. The worker’s seed also replaces an older boot copy (including
recovered journal edits); a newer joined revision remains authoritative. Later
calls flush edits before answering. The setting-off host uses the same extension
readiness gate, so neither host exposes a partially registered panel list. Workspace
tools invoke the production dispatcher, retaining persisted layout, workspace,
viewport and the complete built-in/user panel registry. They do not return the
worker's reduced workspace answer. The page's compositor output is replaced by
CPU snapshots from `pm-node` over authenticated binary HTTP requests.

The desktop writes `desktop-environment.json` beside `Engine Cache/Shader Cache`.
Hidden engines and CLI renders read this namespace and the same `angle-parity`
pins. A hidden profile with its own desktop probe copy uses it; otherwise the
host reads this machine’s desktop store. `POWERMOVE_ENGINE_SHADER_CACHE` selects
an explicit cache for isolated hosts. A profile without desktop probes uses a fresh namespace: user programs
must pass through ANGLE before native verification.

Only the desktop writes that namespace. Its decisions compare native output
with Electron's WebGL runner; the hidden host and both CLI entry points compare
with Playwright's Chromium, which can be a different ANGLE revision or
SwiftShader under launchd or ssh. Their engines therefore open it read only
(`POWERMOVE_ENGINE_PARITY_STORE=read-only`, set by `cliProbeEnvironment` for
the hidden host and `powermove render`, and forced in code for every `pm-cli`
renderer). They honour the desktop's runner pins and native verifications, so
a desktop-verified program draws natively everywhere and a desktop-pinned one
uses the runner. A program the desktop has not decided is probed against the
host's own Chromium on first use, decided in memory for that process, and
leaves no `.native` or `.runner` marker that the desktop would trust. The
desktop still probes it against Electron. A different `STORE` value also
fails closed as read only. The browser module uses
`ShaderFallbackRunner`, with the desktop's F16 texture, uniform and watchdog
contract. `powermove render` starts an authenticated loopback service; direct
bundled `pm-cli` starts the same Node helper and closes it on stdin EOF.
The same service handles level 1 SVG images at `POST /svg`. It verifies the
SVG bytes against the planned content hash, then uses the hidden host page's
`decodeSvg` and `SvgRasterCanvas` to draw today's bucket. The engine receives
premultiplied, top-down RGBA8 and composites it natively (decision S1). Render
and scope install this Chromium raster source; `powermove render` and direct
bundled `pm-cli` use it without an approximation flag. A bare `pm-cli` with no
helper fails with `svg-chromium-unavailable` on a frame that draws an SVG and
does not publish an output. `--approximate-svg` opts into resvg only when no
helper is available, with the named approximate SVG notice and `approximateSvg`
draw count. A configured helper that cannot draw the raster still fails.
The desktop and hidden host also atomically persist their winning definitions in
`Engine Cache/definitions.json`. `powermove render` uses that set unless the
caller provides `--definitions`; direct `pm-cli` accepts the same snapshot.

`pm-cli render` and `pm-cli scope` share one renderer setup
(`pm-cli/src/render.rs` `headless_renderer`): the read-only probe store,
`require_shader_reference`, the helper's Chromium as the fallback host, the
Chromium SVG source and the `--definitions` snapshot (else the built-in set with
the same warning). Before this, scope set up only the SVG source, so with no
`POWERMOVE_ENGINE_PARITY_ENV` every user program counted as natively verified
and a desktop-pinned program was measured from native pixels. Without a helper,
both commands report a pinned, unverified or untranslatable user program as not
headless-renderable (exit 4) and write nothing.

Missing native addons are `native-runtime-unavailable`; missing Chromium is
`chromium-unavailable` at the first browser-dependent call. Corrupt persisted
probe data is `shader-probe-store-unreadable`. The enabled host has no blanket
refusal and no Chromium composition fallback. Linux and the setting-off macOS
host keep the previous behavior.

## Validation commands

Build the matching native wrappers, pm-wasm, desktop and CLI host before taking
the app lease. Hold `pm-electron-lock` only for app runs, with Playwright workers
set to one:

- `node apps/desktop/scripts/test-mcp.mjs --desktop`
- `node apps/desktop/scripts/test-mcp.mjs --native`
- `node apps/desktop/scripts/test-mcp.mjs --missing-browser`
- `node apps/desktop/scripts/test-native-hidden-parity.mjs`
- `node apps/desktop/scripts/test-native-hidden-services.mjs --launchd`
- `node apps/desktop/scripts/test-native-hidden-services.mjs --ssh`
- `node apps/desktop/scripts/test-native-import-contract.mjs` compares imported
  asset/layer bytes with the desktop and metadata with both hidden hosts. It
  covers SVG paths, animated GIF, BMP, OBJ, omitted placement and a registered
  import-default override. `--timeline-regression` is the focused placement gate.
- In `apps/desktop`, set `POWERMOVE_NATIVE_HOST_DIST` to the built CLI `dist`
  and run `bunx playwright test e2e/document-host-contract.spec.ts --config playwright.prebuilt.config.ts --workers=1`.
  Set `POWERMOVE_CONTRACT_ENGINE_SCRIPT` to an engine bundle built beforehand
  with `vite build -c engine.vite.config.ts --mode test` and a separate
  `POWERMOVE_ENGINE_OUT`. Its deterministic IDs and live-byte statistics are
  test-only. The contract compares complete no-window workspace bytes, including
  every desktop built-in and a persisted user panel.
- For shipping observation timing, use the same prebuilt config with
  `e2e/engine-agent-observation.spec.ts`, only when machine load is at most 8.
  `POWERMOVE_SHIPPING_ADDON_SHA256` binds the run to the assembled shipping addon;
  the measurement records its hash and start/peak one-minute load.
- `e2e/engine-svg.spec.ts`, test `powermove render draws level 1 SVG images`,
  compares desktop PNG exports against both CLI entry points at three animation
  times: at least 99.9% of pixels within 1 CV and none beyond 4. Build the desktop,
  CLI host and `pm-cli` first, then wrap only the Playwright command with
  `~/Library/Caches/powermove-engine/bin/pm-electron-lock` and use
  `--config playwright.prebuilt.config.ts --workers=1`.

The parity harness captures a real desktop runner pin, compares every decoded
PNG frame with the hidden export, and compares complete workspace JSON bytes
with today's hidden host, including a user panel. The export fixture has visible
color, verifies the recovered layer identity and requires the project’s ANGLE
runner, so a missing black composition cannot pass pixel parity. Before restart it saves a
named workspace with custom dock sizes and the user panel visible; both hidden
hosts must restore that complete layout and report equal viewport bytes. The
contract gate compares
answers and document bytes after every call, using only the existing deterministic
id normalization. Service tests create temporary user-domain launchd jobs and a
loopback-only SSH service with temporary keys outside the repository; they do
not change login or authorized-key settings.

The same parity fixture renders through both CLI entry points. A simultaneous
native verification must lose to the persisted runner pin; removing only that
pin must produce different native pixels. With every desktop decision removed,
first probes by both CLI entry points and by a hidden host on a fresh profile
copy must match the desktop's pixels and leave `angle-parity` empty. A missing
browser must fail by name. `pm-cli scope` runs on the same pinned project
with the forged native verification in place: its channel statistics must match
`pm-cli scope --frame` of the desktop's frame 0 within one 8-bit step (the PNG
holds bytes, scope measures the 32-bit render). With only the pin removed, the
native scope must deviate by more than that step. Without a helper it must exit
4 naming the effect, and with a missing browser it must fail with
`chromium-unavailable`; neither writes `scope.json`. The `scope_*` tests in
`cargo test -p pm-cli --test definitions` cover a shader layer and a runner-only
definition without a helper, and a shader layer through the production helper.
`cargo test -p pm-cli --test definitions`
(`a_cli_first_probe_leaves_no_decision_in_the_desktop_namespace`) covers an
exactly matching program, which a writable store would have recorded as
`.native`, and pm-render's `effects_fallback` and `effects::parity` tests cover
the read-only store directly. Without `POWERMOVE_ANGLE_HELPER` the helper
gates print a skip reason; gate runs set `POWERMOVE_REQUIRE_ANGLE_HELPER=1`.
The Rust definitions gate retains its original successful override and blue-pixel
assertions, with the production ANGLE helper enabled. Child-supervision tests
kill real pm-node children, check in-flight asset attribution and the two-crash
ledger, increasing backoff, per-project definition replay and recovery of both
engine roles. Build all test binaries before acquiring the app lease.

## Remaining integration gates

The current integration branch does not yet include the extension-compatibility
lane's raster override/context producers and corresponding snapshot events.
The host's on-demand compatibility relay is wired, but those projects require
that integration and parity validation; this lane does not claim their pixels
are proved. The import contract uses synthetic fixtures; wider media corpus
coverage remains with the media lanes. Desktop-encrypted extension variables and on-device captions are not
yet provided by this embedder.

## macOS artifacts

`packages/cli/scripts/assemble-native.mjs` consumes `pm-node-addon`,
`pm-bridge-addon` and `pm-cli` release artifacts, matching LGPL FFmpeg, and the
native image-decoder license directory. It checks architecture, relocates and
signs dylibs, resolves cross-linked Swift overlays to Apple's system runtime
without redistributing them, constructs/ticks/pumps `EngineHost` on clean Node 22,
and launches `pm-cli`. ARM64 and x64 require separate FFmpeg/native libraries; x64 loads under
Rosetta on an ARM64 machine. The FFmpeg cross build uses the same pinned sources,
component selection and LGPL policy as ARM64. These commands never publish.

See [the ownership inventory](agents-ownership.md) for every agent-lane edit
outside the original file ownership list and its justification, including
changes subsequently reverted.

The import contract freezes random WebM container IDs through a temporary FFmpeg wrapper shared by all three test hosts, analogous to the document contract’s clock and ID seam. The production encoder is unchanged. The comparison retains every asset and layer field, including `contentHash`; a separate test compares the wrapper’s decoded RGBA with today’s default encoder. All wrapper files and profiles are removed by the harness.
