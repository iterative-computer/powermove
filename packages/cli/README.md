# powermove

Run the [Powermove](https://powermove.app) host on one machine and use the editor from a browser on another. The agents (ChatGPT via Codex, Claude) run where the host runs, so a Linux box can keep working on your project while your laptop is closed.

```sh
npx powermove@latest serve
```

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

## What lives where

- Projects, extensions, agent workspaces and history live in the profile on the host.
- Files you drop into the editor are uploaded to the host.
- Exports are written on the host and also downloaded by the browser.
- Nothing is sent anywhere else. There is no telemetry.

## Requirements

Node 22 or newer. Linux and macOS.
