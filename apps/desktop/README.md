# Powermove desktop (`@powermove/desktop`)

The Electron editor. This is the only supported application; the earlier root-page/WKWebView implementation and its separate test oracle were retired after parity coverage moved to Vitest. For the repository overview and shared packages, see the [root README](../../README.md).

## Project files

Use **Command+S** to save an editable `.pmv` project and **Shift+Command+S** for Save As. Subsequent saves update the same file, keep the previous version in a hidden backup store, and keep unsaved changes separate from local recovery. **Command+O** reopens projects with their embedded media. See [saving and recovery](docs/project-files.md) for details and the current file-size limit.

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
with a toggle. The agent stages mods in an isolated workspace, guided by
[`docs/EXTENSIONS.md`](docs/EXTENSIONS.md) and the typed API pack. Powermove validates and promotes
reported changes, retains the prior live version for recovery, and reloads the
mod in place.

For interface requests, the agent is prompted to announce its target panel or
new-panel insertion point before editing. A validated public placement message
shows a click-through loading ghost there until the run finishes, fails, or is
stopped. This temporary overlay does not edit the project or saved layout and
does not reopen the app; scene-only requests do not show it.

## Architecture

- `src/main/` owns the Electron lifecycle, windows, menus, storage, native file operations, the private `app://powermove` protocol, and native Codex/Claude Code processes.
- `src/preload/` exposes a narrow typed IPC bridge. The renderer is context-isolated and has no Node.js access.
- `src/shared/` defines IPC contracts, limits, guards, and the shared edit vocabulary.
- `src/renderer/` is a Svelte 5 runes shell over the typed editor core: project state, editing commands, layout, panels, playback, WebGL composition, and export.
- `css/app.css` holds the shared app patterns; the design tokens come from the workspace package `@powermove/tokens` (`packages/tokens/tokens.css` at the repo root), imported as `@powermove/tokens/tokens.css`.
- Alignment haptics come from the `@powermove/macos-haptics` workspace package (`packages/macos-haptics`), a native addon built during `bun install`.
- Autonomous agent requests run through the provider's native harness. A run-scoped local MCP bridge exposes fresh project state, panel layout, real frame rendering, typed edits, and rollback while keeping the renderer as the sole owner of project state and Undo.
- Production assets load from `app://powermove` under CSP. Generated JavaScript runs in a separate sandboxed host with an opaque origin, no network or native bridge, bounded inputs, and validated command output.

Project expressions run through a bounded parser/interpreter, so the privileged
editor document does not permit `unsafe-eval`. Generated JavaScript remains in
its separate opaque-origin sandbox with no network or native bridge.

## Agent tools

Agents can delegate tasks to Codex, Claude, or a connected API/local model,
including models from another provider. Ask naturally, for example: “Have
Claude review the animation while Codex builds it.” Connect each provider in
Settings first. Tasks appear inside the conversation with their model, progress,
result, and a Stop control. Stopping the parent also stops its child tasks.

The app-owned `orchestrator_capabilities`, `delegate_task`, `task_status`, and
`task_cancel` tools use the same model catalog as the composer. Delegation can
run asynchronously or wait up to 90 seconds; a wait timeout keeps the task alive.
Async results automatically resume the parent after its turn. Children inherit
the project, planning mode, access and approval setting, receive the supplied brief rather than
the parent's chat history, and have isolated provider sessions and workspaces.
Nested work remains active until its descendants finish. Task IDs, retry keys,
and results are saved locally; unfinished tasks become interrupted after restart.
Each run permits 32 tasks, eight active children, and four levels of nesting.
Use a new delegation call and retry key for each new review round.

Use `task_send` to send follow-up instructions to an active subagent or change
its provider, model, or supported reasoning level. Direction uses native live
steering when available; otherwise the current turn is interrupted and resumed
in the same task and workspace. Live project edits stay in the parent's Undo
transaction. Completed task results stay immutable.

Agents can also start ordinary conversations with `thread_launch`, inspect them
with `thread_list` and `thread_read`, send direction/settings with `thread_send`,
and stop their current runs with `thread_cancel`. For example: “Start a separate
Claude thread to review the timing, watch it, and use its feedback here.” These
threads appear in the normal picker, keep their own conversation, model choice,
isolated workspace and Undo transaction, and keep working when the parent stops.
New threads inherit planning/project access and approval settings; a one-run Computer grant is not
copied. Thread controls are scoped to the current project and require its tab.

`thread_watch` binds to one run and automatically resumes the watcher with its
result after the current turn. Waiting can time out without cancelling the work;
`thread_unwatch` removes monitoring without stopping that thread. Reading a
terminal run acknowledges its delivery. A later turn needs another watch, watch
cycles are rejected, and active watches end when the parent stops or Powermove
closes. Launch/message retry keys are stored with the thread history.

Codex and Claude agents can use all enabled user-scoped MCP servers configured
for that provider, alongside Powermove's built-in tools. Registrations are read
again on each message; there is no Powermove-specific external-tool allowlist.
Codex reads `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`). Claude reads
`~/.claude.json` (or `$CLAUDE_CONFIG_DIR/.claude.json`). Configure Claude servers
with `claude mcp add --scope user` to make them available across projects.

The `powermove` server name is reserved for the live editor connection. Disabled
servers stay disabled. Provider-level tool restrictions and authentication still
apply; remote services may need their own login in Powermove's provider runtime.
User skills, plugins, hooks, rules, custom commands, and agent definitions are
available through the provider's normal resource discovery. Powermove shares the
resource folders and imports their settings without sharing login or session
files. Existing private resource folders are backed up before migration.
Resource configuration refreshes when the provider runtime starts; restart
Powermove after changing plugin settings. Project-local resources are discovered
from the agent workspace, not from unrelated repositories.
Existing Editor, Project, and Computer file-access modes continue to apply to
built-in tools. External MCP servers run with their configured access, which may
include files or services beyond the project workspace.

## Development

Requirements: macOS on Apple Silicon and bun 1.3 or newer (the repo pins `bun@1.3.14`). Xcode command line tools are needed to build the native haptics addon. Agent features additionally require the Codex CLI or Claude Code to be installed and signed in.

Install once from the repository root, then run scripts either from the root or from this directory:

```sh
bun install                 # from the repo root; installs every workspace

# from apps/desktop
bun run dev                 # Electron + renderer hot reload
bun run build               # production bundles in out/
bun run test                # Vitest suite
bun run test:e2e            # Playwright Electron coverage
bun run dist:mac            # arm64 DMG, ZIP, and app in dist/
```

From the repository root the same lanes are `bun run dev`, `bun run build`, `bun run test`, `bun run test:e2e`, and `bun run dist:mac`; `bun run --cwd apps/desktop <script>` runs any desktop script from the root.

`bun run typecheck` runs Svelte and TypeScript checks. `bun run preview` launches the built application. `bun run lint:boundaries` checks the kernel/extension import boundaries.

Run Vitest through the package script (`bun run test`), not `bunx vitest`: `bunx vitest` runs Vitest under bun's runtime instead of Node and skews a calibrated performance test. To pass files or flags to Vitest, append them after `--`, for example `bun run test -- src/main/updates.test.ts`.

macOS packaging is currently ad-hoc signed for development distribution; see the [release notes](docs/release.md) before sharing builds.

## Retired escape hatches

- There is no root `index.html` or standalone classic-script application.
- There is no Swift/WKWebView launcher or `build:mac` shell-script path.
- There is no separate Node `tests/` oracle; `vitest run` is authoritative.
- The renderer cannot bypass preload to reach Node, files, or Codex directly.
- Generated scripts cannot run in the application document or return unvalidated mutations.

## Delivery status

- Unsupported MOV/ProRes video is converted automatically to a durable H.264
  editing proxy while the original media stays embedded in the editable project.
- Containers Chromium cannot open — MKV, AVI, WMV, FLV, MPEG-PS/TS, MXF and the
  rest — convert on import through the same proxy path.
- Export includes playable H.264/AAC MP4, frame-exact WebM/VP9/Opus,
  realtime WebM, still PNG, and PNG-sequence delivery.
- Local builds remain ad-hoc signed. `bun run dist:release` is the guarded
  Developer ID, hardened-runtime, notarization, and verification lane; it exits
  before building unless the required certificate and Apple credentials exist.

## Design and migration notes

The panel Library lists every registered panel and offers **New panel** and
panel-specific refinement. Panel focus shares the model/reasoning options row;
selection boxes carry all intersecting panels into the agent request.

Scale axes can be linked in Properties. Animation caches are refreshed for each
frame, and selection handles extend across the viewer workspace. Save confirms
the native file write; autosave waits for storage and restores the last active
project. The animation performance baseline now measures actual frame changes
(the old baseline accidentally reused frozen transforms).

- [Design language](../../docs/design.md)
- [Electron scaffold](docs/scaffold.md)
- [Agent testing without visible windows](docs/background-testing.md)
- [App icon](docs/app-icon.md)
- [Phase 0 platform decisions](docs/phase0-decisions.md)
- [Phase 3b conversion pattern](docs/phase3b-pattern.md)
- [macOS release process](docs/release.md)
- [Phase 6 deletion manifest](docs/phase6-deletions.md)
- [e2e media fixtures](e2e/fixtures/README.md)

## Supported media

| Kind | Imports directly | Converted on import |
| --- | --- | --- |
| Still image | PNG, JPEG, WebP, AVIF, BMP, ICO, SVG | TIFF, HEIC/HEIF |
| Animated image | — | GIF, APNG, animated WebP, animated AVIF |
| Video | MP4, MOV, M4V, WebM | MKV, AVI, WMV, ASF, FLV, MPG/MPEG, M2V, TS/M2TS/MTS, 3GP/3G2, MXF, OGV, DV, VOB |
| Audio | WAV, MP3, M4A, AAC, OGG/OGA, Opus, WEBA, FLAC, AIF/AIFF | — |
| Model | OBJ | — |

A MOV or MP4 holding a codec Chromium refuses (notably ProRes) also converts,
after the decode attempt fails. Conversion happens in the main process with the
bundled FFmpeg; HEIC and HEIF go through macOS `sips`, which reads the HEIF
variants FFmpeg 6 cannot. Powermove stores the converted media in the project,
so a saved `.pmv` never depends on the original file.

## Animated images

An animated GIF, APNG, WebP or AVIF imports as a video clip rather than a still,
with the normal trim, speed, undo, playback, and export controls. Chromium
decodes the frames — it is the only decoder here that reads all four containers —
and the frames encode into the same transparent VP9 clip numbered image
sequences produce, so transparency and frame order survive.

Each frame is held for its own delay. When the delays share a divisor, the clip
plays at exactly that rate; when they do not, the frames are oversampled onto a
bounded rate instead, which keeps the total duration right. A still saved in one
of these containers still imports as an image. Animations above 2,400 frames are
rejected rather than encoded.

## Image sequences

Select numbered PNG, JPEG, WebP, or BMP frames through Import media
(Command+I) or drag them into the editor. Choose **Import sequence** and set the
source frame rate (1–240 fps, defaulting to the composition rate). Choose
**Individual images** to keep the stills separate. **Import image sequence…**
is also available in the File menu and command palette.

Frames must share a filename prefix, extension, and dimensions, with no duplicate
frame numbers. Missing frame numbers are listed before import. Continue to play
the available frames consecutively, choose **Reimport…** to select files again
while keeping your frame rate, or cancel. Selection order and zero padding do not
matter. The sequence becomes one video clip with transparency, source duration, and the
normal trim, speed, undo, playback, and export controls. Powermove stores the
converted clip in project media, so saving a `.pmv` embeds it and reopening does
not require the original frame files.
