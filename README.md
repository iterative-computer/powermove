# Powermove

Powermove is an AI-native motion and video editor for macOS. It combines a GPU-backed editing engine with a dockable Svelte interface and an agent that can propose typed, undoable project edits through the locally installed Codex CLI.

The refactored Electron application is the only supported app. The earlier root-page/WKWebView implementation and its separate test oracle were retired after parity coverage moved to Vitest.

## Architecture

- `src/main/` owns the Electron lifecycle, windows, menus, storage, native file operations, the private `app://powermove` protocol, and Codex CLI processes.
- `src/preload/` exposes a narrow typed IPC bridge. The renderer is context-isolated and has no Node.js access.
- `src/shared/` defines IPC contracts, limits, guards, and the shared edit vocabulary.
- `src/renderer/` is a Svelte 5 runes shell over the typed editor core: project state, editing commands, layout, panels, playback, WebGL composition, and export.
- Agent requests run through the Codex CLI in the main process. Agent-authored project changes return through the same validated, revision-checked, undoable edit boundary as direct manipulation.
- Production assets load from `app://powermove` under CSP. Generated JavaScript runs in a separate sandboxed host with an opaque origin, no network or native bridge, bounded inputs, and validated command output.

The application CSP still permits `unsafe-eval` for the legacy expression evaluator. Removing it depends on the post-parity expression-interpreter work described below; the generated-script host remains separately sandboxed.

## Development

Requirements: macOS, Node.js 22 or newer, and npm. Agent features additionally require the Codex CLI to be installed and signed in.

```sh
npm ci
npm run dev       # Electron + renderer hot reload
npm run build     # production bundles in out/
npm run test      # Vitest suite
npm run test:e2e  # Playwright Electron coverage
npm run dist:mac  # arm64 DMG, ZIP, and app in dist/
```

`npm run typecheck` runs Svelte and TypeScript checks. `npm run preview` launches the built application. macOS packaging is currently ad-hoc signed for development distribution; see the release notes before sharing builds.

## Retired escape hatches

- There is no root `index.html` or standalone classic-script application.
- There is no Swift/WKWebView launcher or `build:mac` shell-script path.
- There is no separate Node `tests/` oracle; `vitest run` is authoritative.
- The renderer cannot bypass preload to reach Node, files, or Codex directly.
- Generated scripts cannot run in the application document or return unvalidated mutations.

## Post-parity backlog

- Replace the expression evaluator with an interpreter and remove application-level `unsafe-eval`.
- Add a supported ProRes import/transcode path.
- Add H.264/MP4 export; current delivery is WebM/VP9/Opus plus still/PNG-sequence paths.
- Restore safe panel pop-outs where the new shell intentionally omits them.
- Add Developer ID signing, hardened runtime, and notarization for public distribution.

## Design and migration notes

- [Electron scaffold](docs/scaffold.md)
- [Phase 0 platform decisions](docs/phase0-decisions.md)
- [Phase 3b conversion pattern](docs/phase3b-pattern.md)
- [macOS release process](docs/release.md)
- [Phase 6 deletion manifest](docs/phase6-deletions.md)
