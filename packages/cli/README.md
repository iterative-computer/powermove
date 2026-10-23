# powermove-cli

The package is `powermove-cli` (npm refused `powermove` as too close to an unrelated package); the command it installs is `powermove`.

The [Powermove](https://powermove.app) desktop app is the way to use Powermove. This package is for the other case: running the Powermove host on a machine you leave on, and using the editor from a browser on any device. The agents run on the host, edits made anywhere reach every open tab, and a run you started keeps going after you close the lid.

Try it once, nothing installed:

```sh
npx powermove-cli@latest serve
```

Keep it running on a box:

```sh
npx powermove-cli@latest install   # copies the host under ~/.powermove and registers a service (systemd on Linux, launchd on macOS)
powermove status                   # running? and the address to open
powermove logs                     # the host log
powermove uninstall
```

No `sudo`: everything lands in your home folder and the service runs as you. Run `install` again to update. The `powermove` command is placed in `~/.local/bin` when that folder exists; otherwise use `npx powermove-cli <command>`.

On Linux, `loginctl enable-linger $USER` keeps the service up while you are logged out.

A browser tab that loses its connection to the host (Wi-Fi blip, closed lid, phone changing networks) reconnects on its own; edits made in the meantime are sent when it is back, and a running agent turn is picked up where its stream left off.

Open the printed URL. Over [Tailscale](https://tailscale.com), use the `100.x` address; on a LAN, the local IP. The URL carries a one-time token that the browser remembers.

The certificate is self-signed, so the browser asks once whether to proceed. For a trusted one:

```sh
npx powermove-cli@latest serve --http
tailscale serve --bg https+insecure://localhost:4747
```

and open the `https://<machine>.<tailnet>.ts.net` URL Tailscale prints, with `?token=` from the first command.

## Signing in to an agent

Codex and Claude sign in with a browser flow that returns to `localhost` on the host machine. Either:

- run `codex login` or `claude auth login` in a terminal on the host once, or
- forward the callback port when you connect: `ssh -L 1455:localhost:1455 host`, then use Settings › Agents in the browser.

## Updates

The host is published to npm with every Powermove release, at the same version as the Mac app, through npm trusted publishing from the release workflow. It checks the registry and the editor shows the same update notice the desktop app does. Pressing it updates a global install in place and restarts the service; an `npx` run or a source checkout is handed the command to run instead.

## Options

```
--port <n>        Port to listen on (default 4747)
--host <addr>     Interface to bind (default 0.0.0.0; use 127.0.0.1 to stay local)
--user-data <dir> Profile directory (default ~/.powermove)
--exports <dir>   Where Save… writes on the host (default ~/Powermove)
--token <value>   Access token (default: generated once and kept in the profile)
--http            Plain http instead of self-signed https (only behind a TLS proxy)
```

## How it works

- The host keeps the document. Every tab on a project sees the same document; an edit on one device appears on the others, and each device keeps its own undo.
- Media you import is stored on the host, so a second device opening the project has the footage.
- Fonts travel too. The font picker lists your machine's fonts and the host's. When a text layer uses a font the host does not have, the browser sends that family's files up once; any device that lacks the font renders it from the host.
- Agent runs are owned by the host and served by its document engine, the editor's core running in Node. Close the tab and the run finishes anyway; open a tab later and the thread shows the result. Tools that need pixels (panel capture, panel interaction) wait for an open tab.
- Projects, extensions, agent workspaces and history live in the profile on the host (`~/.powermove`).
- Exports are written on the host and also downloaded by the browser.
- Nothing is sent anywhere else. There is no telemetry.

## MCP for agents

The CLI also provides a stdio MCP server. Run `powermove mcp-install-browser` once, then configure your agent to launch `powermove mcp`. It starts a hidden editor, so agents can create a project, import media, animate and edit, review frames, and export video without opening the app. Its default profile is `~/.powermove-headless`; exports go to `~/Powermove`.

Use `--user-data` and `--exports` for other folders. `mcp --connect --user-data <profile>` connects to an already running app or host. See [the MCP guide](https://github.com/iterative-computer/powermove/blob/main/apps/desktop/docs/MCP.md) for client configuration, tool workflow and capability limits.

## Native renders

```sh
powermove render project.pmv /any/output/folder/movie.mov --preset master
powermove render project.pmv frame.png --format still --param 'title="Hello"'
```

The second argument is the destination, including paths outside the exports folder.
Render options are the native `pm-cli render` options: definition snapshots
(`--definitions`), media overrides, fonts via the host registry, composition, range,
rate, size, presets and repeatable `--param NAME=JSON` values. Parameter overrides
use the player's numeric bounds, toggle, color and select validation. Pinned and not-yet-verified level 1 user GLSL programs run through the
on-demand Chromium ANGLE runner, using the desktop profile's persisted probe
namespace and pins. A missing browser is named `chromium-unavailable` when a
program needs it. The profile's latest published definition snapshot is used
unless `--definitions` supplies another set. Existing outputs require `--force`. Native process exit codes reach the caller.

Level 1 SVG images also start Chromium on demand. Both `powermove render` and
bundled `pm-cli` draw the app's SVG raster buckets through the hidden host's
`decodeSvg` and `SvgRasterCanvas` module. A bare `pm-cli` without the helper
fails with `svg-chromium-unavailable` when a frame draws an SVG. Passing
`--approximate-svg` permits resvg only when no helper is available; this prints
an approximate SVG notice and counts those draws in the summary's
`approximateSvg`. It does not provide the app's Chromium pixels.

A source checkout can set `POWERMOVE_PM_CLI` to its built binary. A packaged build
looks under `dist/native/<platform>-<arch>/pm-cli` and names a missing runtime as
`native-renderer-unavailable`.

Maintainers assemble each macOS architecture from its matching Rust artifacts and
verified LGPL FFmpeg build before building the package:

```sh
bun run build:native --arch arm64 --artifacts ../../engine/target/release --ffmpeg /path/to/lgpl-build --node /path/to/clean-node22
# Repeat with x64 artifacts, x64 FFmpeg, and x64 Node 22.
bun run build
```

Assembly relocates dependent dylibs, applies ad-hoc signatures, and load-tests the
addons plus `pm-cli` with Node 22. It writes ignored `prebuilds/`; the package build
copies those into `dist/native`. This does not create a release or publish.
Both architecture gates are required before distributing native CLI support.

On macOS, `mcp` with Use New Engine enabled embeds host services, supervised
native engines and one document worker per project. Chromium starts on demand
for workspace/panel tools and pinned or unverified user shaders. Missing
Chromium returns `chromium-unavailable` when first needed. The native runtime
must match the bundled protocol; missing addons are `native-runtime-unavailable`.
`render` and direct bundled `pm-cli` use the desktop profile’s shader pins and
the same on-demand Chromium service for ANGLE and level 1 SVG images. Set `POWERMOVE_USER_DATA` to select another
profile. See [native hidden-host integration](../../docs/engine/hidden-host.md)
for compatibility and validation status.

## Requirements

Node 22 or newer. Linux and macOS.
