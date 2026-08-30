# Powermove

Powermove is an AI-native motion and video editor for macOS. It combines a GPU-backed editing engine with a dockable Svelte interface and an agent that can propose typed, undoable project edits through the locally installed Codex CLI.

The refactored Electron application is the only supported app. The earlier root-page/WKWebView implementation and its separate test oracle were retired after parity coverage moved to Vitest.

## Project files

Use **Command+S** to save an editable `.pmv` project and **Shift+Command+S** for Save As. Subsequent saves update the same file, retain a `.pmv1` backup, and keep unsaved changes separate from local recovery. **Command+O** reopens projects with their embedded media. See [saving and recovery](docs/project-files.md) for details and the current file-size limit.

## Kernel and extensions

Powermove is a small kernel plus extensions. The kernel owns the project store,
the typed 19-command edit boundary with undo/revision checks, the WebGL
compositor (including two-input transitions), and typed registries for panels,
commands, keybindings, effects, transitions, themes, palette providers, menus,
and status items. Everything above the kernel — the default theme, keymap,
effects, transitions, toolbar, viewer, timeline, inspector, and Mods surface —
ships as built-in extensions in `src/extensions/*`, written against the same
`PowermoveAPI` (apiVersion 1) that user extensions use.

User extensions ("mods") live in `<userData>/extensions/<id>/` as a
`manifest.json` plus TypeScript/Svelte sources. The main process compiles them
with esbuild on change and serves the bundles over `app://powermove/ext/`;
the renderer kernel hot-loads them, contains their failures (an erroring mod is
auto-disabled with a Fix it / Turn off toast), and lets any built-in be layered
over or replaced (`replaces` in the manifest). The Mods panel lists everything
with a toggle. The agent writes mods directly into that directory through its
Codex workspace (`--add-dir`), guided by `docs/EXTENSIONS.md` and the typed API
pack; results report changed extension ids so the app reloads them in place.

For interface requests, the agent is prompted to announce its target panel or
new-panel insertion point before editing. A validated public placement message
shows a click-through loading ghost there until the run finishes, fails, or is
stopped. This temporary overlay does not edit the project or saved layout and
does not reopen the app; scene-only requests do not show it.

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
- Add Developer ID signing, hardened runtime, and notarization for public distribution.

## Design and migration notes

The panel Library lists every registered panel and offers **New panel** and
panel-specific refinement. Panel focus shares the model/reasoning options row;
selection boxes carry all intersecting panels into the agent request.

Scale axes can be linked in Properties. Animation caches are refreshed for each
frame, and selection handles extend across the viewer workspace. Save confirms
the native file write; autosave waits for storage and restores the last active
project. The animation performance baseline now measures actual frame changes
(the old baseline accidentally reused frozen transforms).

- [Electron scaffold](docs/scaffold.md)
- [Phase 0 platform decisions](docs/phase0-decisions.md)
- [Phase 3b conversion pattern](docs/phase3b-pattern.md)
- [macOS release process](docs/release.md)
- [Phase 6 deletion manifest](docs/phase6-deletions.md)
