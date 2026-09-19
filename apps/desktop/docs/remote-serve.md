# Remote host: `powermove serve`

The desktop app is one Electron process: a sandboxed renderer that only reaches native capability through the frozen IPC contract in `src/shared/ipc.ts`, and a main process whose modules register against `IpcMain` and answer through `event.sender.send(...)`. `powermove serve` keeps those main modules as they are and swaps the two Electron-specific pieces around them:

| Desktop | Remote host |
| --- | --- |
| `BrowserWindow` + preload (`src/preload/index.ts`) | A browser tab + `src/renderer/src/host/web-bridge.ts`, which installs `window.powermove` over one WebSocket before the engines boot |
| `ipcMain` | `WebIpcMain` in `src/server/clients.ts`; each socket is a `RemoteClient` that stands in for a `WebContents`, with a `RemoteWindow` for `BrowserWindow.fromWebContents` |
| `electron` module | `src/server/electron-stub.ts`, aliased at build time (`scripts/build-serve.mjs`). Dialogs are answered by the browser (`ask` frames) or resolved to `~/Powermove`; `safeStorage` is AES-GCM with a key file in the profile |
| `app://powermove` | `node:http` static server for `out/renderer`, `/ext/<id>/bundle.js` from the extension registry, `/__powermove/download` for exports |

Messages are framed by `src/shared/wire.ts`: a JSON header with every `Uint8Array` pulled into a trailing blob table, so images, project bytes and media chunks cross without base64. `sendSync` channels (store boot snapshot, window initial project) become async `sync` requests the bridge awaits before importing `legacy/bootstrap`.

## What behaves differently in a browser

- **Files** dropped into the editor are uploaded to `<profile>/Remote Uploads` first, then converted by ffmpeg on the host like any local file. Open Project… uploads into `<profile>/Remote Projects`.
- **Save…** writes to `~/Powermove` on the host (deduplicated names). Exports that are not `.pmv` are also downloaded by the browser. Projects opened from an upload save back to their uploaded path.
- **Confirmations** are `window.confirm`. Three-button sheets (Save / Cancel / Don't Save) map OK to the primary button; the middle option is not reachable.
- **Windows**: one browser tab is one window. "Open in new window" is `window.open('/?project=<id>')`; the new tab tells the host which project it was opened for in the WebSocket URL.
- **No** native menus (the overlay Menu falls back to its own listbox), haptics, theme source, window capture, auto-update, or system fonts beyond what the browser exposes.
- **Agents** run on the host with the platform binaries from `@openai/codex` and `@anthropic-ai/claude-code`. Sign-in flows return to `localhost` on the host, so either sign in there once (`codex login`, `claude auth login`) or forward port 1455 over SSH.

## Access

The host binds `0.0.0.0:4747` by default and prints a URL with a token. The token is generated once, kept at `<profile>/serve-token` (mode 600), and exchanged for an `HttpOnly; SameSite=Strict` cookie on first visit. Every HTTP request and the WebSocket upgrade require the cookie. There is no TLS; use Tailscale or a LAN you trust, or put it behind a reverse proxy that terminates TLS.

## Building and publishing

```sh
bun run --cwd packages/cli build     # electron-vite build + esbuild server → packages/cli/dist
node packages/cli/bin/powermove.mjs serve --host 127.0.0.1
```

`packages/cli` is the `powermove` npm package (`npx powermove@latest serve`). Its `prepack` runs the same build; `dist/` is not committed. The package depends on `esbuild`, `svelte` (extension compiler), `ffmpeg-static`, `ws`, and the two agent runtimes whose optional dependencies pull the right platform binary.

## Development

`bun run dev` in `apps/desktop` still serves the renderer to Electron from Vite; the bridge sees `window.powermove` from the preload and does nothing. To iterate on the host, rebuild with `node scripts/build-serve.mjs --out /tmp/pm-serve` and run `packages/cli/bin/powermove.mjs` against it with `POWERMOVE_USER_DATA` pointing at a scratch profile.
