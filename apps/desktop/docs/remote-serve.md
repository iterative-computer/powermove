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

The host binds `0.0.0.0:4747` by default and prints a URL with a token. The token is generated once, kept at `<profile>/serve-token` (mode 600), and exchanged for an `HttpOnly; Secure; SameSite=Strict` cookie on first visit. Every HTTP request and the WebSocket upgrade require the cookie.

The host serves https with a self-signed certificate minted by the machine's `openssl` (`src/server/tls.ts`), listing localhost and every interface address; it is re-minted when the address list changes. Browsers only expose WebCodecs, OPFS, the clipboard, `crypto.subtle` and `crypto.randomUUID` on a secure origin, and a LAN or tailnet IP over plain http is not one, so this is not optional. The browser warns once per origin. For a trusted certificate, run `--http` behind `tailscale serve --bg https+insecure://localhost:4747` (or any TLS-terminating proxy) and use the URL it prints.

Restored project media is staged in memory on a remote client (`restoreProjectFileStream`): current Chromium keeps IndexedDB blobs as references to the OPFS staging file, which is deleted after the write, so the desktop app's disk staging leaves the media unreadable in a browser.

## Sessions, engine, runs (2026-09-19)

- **Sessions** (`src/server/sessions.ts`): the host holds the authoritative document per open project. Tabs diff their document against the last synced copy on every change signal (`src/renderer/src/host/remote-sync.ts`) and send leaf patches; the host applies, sequences, forwards, and persists to the project slot. A synced document that brings new assets triggers the same asset restore a project open gets. Claims never bounce between tabs; a tab opened without a project mirrors the most recent one.
- **Media** (`src/renderer/src/host/remote-media.ts`): the media store is wrapped at install; imports are mirrored to `<profile>/Media Store` (content-addressed), a miss is filled from the host over `/__powermove/media`.
- **Fonts** (`src/server/fonts.ts`, `src/renderer/src/host/remote-fonts.ts`): the host lists fontconfig families plus a `<profile>/Fonts` store. The tab unions host families into the picker, registers host faces it lacks as `FontFace`s from `/__powermove/fonts/file`, and when the project uses a family the host lacks, uploads that family's faces (Local Font Access `blob()`), once. `POWERMOVE_FONTCONFIG=0` ignores the host's system fonts.
- **Engine** (`src/engine/main.ts`, built by `engine.vite.config.ts` for Node): the renderer's core under happy-dom, connected as a client of kind `engine`. It joins sessions and answers the agent's document tools. No browser, no GPU.
- **Runs** (`src/server/runs.ts`): `codex:run` is intercepted so `event.sender` is a `RunOwner` that lives until the host stops, buffers events, forwards them to attached tabs, routes document tools to the engine and display tools to a live tab. A tab lists runs for its project and attaches with replay; the assistant (`resumeHostRuns` in spatial.ts) puts a thread still waiting on an answer back into the run.
- **Install** (`src/server/install.ts`): `powermove install|uninstall|status|logs` write and manage a systemd user unit or launchd agent that runs `serve` with the same flags; the log lives in the profile.
- `scripts/fake-model.mjs` is a scripted OpenAI-compatible model for exercising runs without a provider.

## Building and publishing

```sh
bun run --cwd packages/cli build     # electron-vite build + esbuild server → packages/cli/dist
node packages/cli/bin/powermove.mjs serve --host 127.0.0.1
```

`packages/cli` is the `powermove` npm package (`npx powermove@latest serve`). Its `prepack` runs the same build; `dist/` is not committed. The package depends on `esbuild`, `svelte` (extension compiler), `ffmpeg-static`, `ws`, and the two agent runtimes whose optional dependencies pull the right platform binary.

## Development

`bun run dev` in `apps/desktop` still serves the renderer to Electron from Vite; the bridge sees `window.powermove` from the preload and does nothing. To iterate on the host, rebuild with `node scripts/build-serve.mjs --out /tmp/pm-serve` and run `packages/cli/bin/powermove.mjs` against it with `POWERMOVE_USER_DATA` pointing at a scratch profile.
