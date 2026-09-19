# powermove

The [Powermove](https://powermove.app) desktop app is the way to use Powermove. This package is for the other case: running the Powermove host on a machine you leave on, and using the editor from a browser on any device. The agents run on the host, edits made anywhere reach every open tab, and a run you started keeps going after you close the lid.

Try it once, nothing installed:

```sh
npx powermove@latest serve
```

Keep it running on a box:

```sh
npm i -g powermove
powermove install          # systemd user service on Linux, launchd agent on macOS
powermove status           # running? and the address to open
powermove logs             # the host log
powermove uninstall
```

On Linux, `loginctl enable-linger $USER` keeps the service up while you are logged out.

Open the printed URL. Over [Tailscale](https://tailscale.com), use the `100.x` address; on a LAN, the local IP. The URL carries a one-time token that the browser remembers.

The certificate is self-signed, so the browser asks once whether to proceed. For a trusted one:

```sh
npx powermove@latest serve --http
tailscale serve --bg https+insecure://localhost:4747
```

and open the `https://<machine>.<tailnet>.ts.net` URL Tailscale prints, with `?token=` from the first command.

## Signing in to an agent

Codex and Claude sign in with a browser flow that returns to `localhost` on the host machine. Either:

- run `codex login` or `claude auth login` in a terminal on the host once, or
- forward the callback port when you connect: `ssh -L 1455:localhost:1455 host`, then use Settings › Agents in the browser.

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
- Agent runs are owned by the host and served by its document engine, the editor's core running in Node. Close the tab and the run finishes anyway; open a tab later and the thread shows the result. Tools that need pixels (panel capture, panel interaction) wait for an open tab.
- Projects, extensions, agent workspaces and history live in the profile on the host (`~/.powermove`).
- Exports are written on the host and also downloaded by the browser.
- Nothing is sent anywhere else. There is no telemetry.

## Requirements

Node 22 or newer. Linux and macOS.
