# Hidden MCP host integration status

The macOS native hidden host is incomplete. `server/mcp.ts` still embeds the
Chromium production editor through `serve`; it does not embed the per-project
`DocumentHostPool` or a supervised `pm-node` job engine. That path must not be
mistaken for native delivery.

`hidden-engine-policy.ts` reads the persisted `useNewEngine` setting using the
store's key mapping and the launch-time environment override. On macOS an enabled
profile fails with `native-hidden-host-unavailable` before the profile lock or
Chromium startup. An unreadable setting fails with
`native-engine-setting-unreadable`. Linux retains its Chromium host. A missing
Chromium launch is named `chromium-unavailable`. Native desktop connections remain
available through `mcp --connect`.

`RunHub` now chooses a tab belonging to the run's pinned project for DOM tools and
for document tools when no worker is available. It never uses another project's
first tab. `set_panel_layout` is a DOM tool. `get_workspace_state` requires the
production tab: a document worker's null workspace and empty panel registry are
not an equivalent answer. Without that tab the call fails, rather than returning
invented layout or panel data.

## Required integration before native hidden delivery

- Embed `DocumentHostPoolApi` in the CLI host with profile-store, project-session,
  extension, transcription and host-media services. Preserve each project's Undo.
- Drive `pm-node` in a child using the desktop supervisor, crash attribution,
  backoff, host services, full fonts and acknowledged winning definition sets.
- Import and replace through the media store and engine probes without a file
  input. Worker `media.ts` currently provides availability, not import decoding.
- Start Chromium on demand for DOM tools, compatibility raster/SVG inputs and
  pinned or unverified level 1 user GLSL. Persisted per-machine shader pins must
  reach this host and the CLI; native translation alone does not verify parity.
- Pin each Chromium tab to its session project and present CPU frames from
  `pm-node`. Return the production hidden editor's persisted workspace/layout,
  viewport and complete built-in/user panel registry, byte for byte.
- Gate desktop/hidden tool document bytes, pinned-program export pixels,
  publish-then-render, video import, on-demand panels, missing Chromium, FFmpeg
  verification, launchd and SSH without a GUI session. These native hidden gates
  are not passed by the Chromium-only smoke test.

Native artifact assembly is in `packages/cli/scripts/assemble-native.mjs`. It
checks architecture, relocates dylibs, includes FFmpeg license/provenance files,
requires Node 22 for addon loading, and launches `pm-cli`. The package build copies
ignored `packages/cli/prebuilds` into `dist/native`. Both darwin-arm64 and darwin-x64
need their own matching LGPL FFmpeg artifacts and clean Node 22 load test.

`pm-cli render` now calls `Renderer::require_shader_reference`. The shared effect,
transition and shader-layer gates consult a configured parity service even when no
runner is attached. Unknown and pinned programs then fail by name, rather than
using an unverified native frame. The CLI still needs the desktop environment/pin
store integration and the on-demand ANGLE runner to export those projects.
