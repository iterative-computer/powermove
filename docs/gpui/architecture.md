# Powermove for Mac: the GPUI + AppKit rewrite

Status: draft, 2026-10-31. Branch `feat/gpui-mac` (from `new-3d-system-native`,
which carries the Rust engine). Code: `apps/mac`.

## In one paragraph

Powermove today is a web page in a browser shell (Electron): Svelte draws the
UI and a TypeScript/WebGL engine draws the pictures. The rewrite turns it into
a real Mac app. **GPUI** (Zed's Rust UI framework, which draws with the GPU
like a game) draws the dense editing surfaces: viewer, timeline, inspector,
layers. **AppKit** (Apple's own toolkit) draws the window chrome in Liquid
Glass: toolbar, menus, sheets, alerts, file pickers and the Settings window.
The pictures come from the **Rust engine** already on this branch, linked
into the app directly. The goal is **1:1 parity**: every feature in
`parity-a-shell.md` (755 items) and `parity-b-editor.md` (637 items) works
the same in the new app before it replaces the old one.

## Why hybrid

| Need | GPUI | AppKit |
|---|---|---|
| Thousands of timeline rows, 120 Hz drags, custom hit-testing | yes (GPU-drawn, Rust) | slow to customize |
| Liquid Glass that refracts content, system toolbar, menus, sheets, Settings, VoiceOver defaults | no | yes, automatically |
| One codebase portable to Windows later | yes (GPUI has DirectX) | Mac only, kept thin |

So AppKit is a thin, Mac-only skin, and everything that holds state or logic
lives in portable Rust.

## How the glass works (proven in the spike)

GPUI owns one view inside the window. `apps/mac/src/glass.rs` adds AppKit views
around it:

- **Toolbar:** a real `NSToolbar`; macOS groups its items into glass capsules.
- **Behind glass:** `NSGlassEffectView`s under GPUI's view. GPUI leaves those
  rectangles transparent and draws its controls on top, so the sidebar and
  inspector sit on real glass.
- **Floating glass:** `NSGlassEffectView`s above GPUI's view, holding native
  controls (the viewer's zoom HUD). They refract the GPUI pixels underneath.
- GPUI reports each region's rectangle while it lays out a frame, on the main
  thread, so glass and content move together.

HIG rules we follow: glass is for navigation and controls, never for the
content itself (the viewer and timeline stay opaque); floating panels are
inset from the window edge with concentric corners; standard controls,
SF Symbols, system accent colors and appearance; Reduce Transparency and
Increase Contrast swap glass for solid fills automatically.

## Process model

```
Powermove (one Rust process)
├─ main thread: AppKit + GPUI (UI only, never waits on the engine)
├─ document core: pm-model document, edit commands, undo, selection
├─ interactive engine threads: evaluate, render to IOSurface, playback, audio
├─ media threads: decode, thumbnails, waveforms
└─ children
   ├─ job engine (same binary, `--engine-job`): exports, agent renders, proxies
   ├─ extension host (JavaScriptCore): runs extension scripts, crash-isolated
   └─ agent runtimes already external today: `codex app-server`, `claude`
```

Engine API details, what the engine already renders natively, and which
TypeScript must be ported: `engine-boundary.md`.

## Code layout

`apps/mac` is its own Cargo workspace (so engine-only builds never compile
GPUI); it links engine crates by path. Panels are separate crates so lanes can
work in parallel without touching each other's files:

```
apps/mac/
  src/main.rs            app entry, windows, menus
  crates/
    glass/               AppKit bridge (toolbar, glass regions, sheets, alerts, panels)
    kit/                 GPUI component kit = src/renderer/src/controls 1:1
    core/                app state: document, commands, undo, selection, keymap, settings
    panel-viewer/        viewer, tools, gizmos, overlays
    panel-timeline/      timeline, keyframes, graph editor
    panel-inspector/     inspector, effect controls, value dialogs
    panel-project/       layers/project panel, library, import
    panel-effects/       effects & presets browser, easing
    panel-agent/         agent conversations
    panel-media/         mixer, captions, transcription
    export/              export dialog, render queue
    shell/               welcome/projects, settings window, docking/workspaces
    extensions/          v4 extension host + v1–3 migration
    mcp/                 external MCP server + CLI bridge
```

## Extensions

Decision (user, 2026-10-31): keeping today's Svelte extensions running is
**not** required if it is costly. The plan:

1. **v4 native extensions.** A manifest, scripts in TypeScript/JavaScript run by
   JavaScriptCore (built into macOS, nothing to bundle), and panels declared
   from the native component kit (`kit`), so they look and behave like
   built-in panels. GLSL effects, transitions and shader layers keep today's
   contract unchanged (the engine already compiles it), so most effect
   extensions carry over untouched.
2. **Agent-driven migration.** `docs/extensions/migrating-to-v4.md` plus an
   MCP tool (`migrate_extension`) and agent skill: a user tells their agent
   "convert my extensions", the agent reads the v1–3 source, writes v4, and
   verifies it in the app. Svelte panel UIs are the only part that needs
   rewriting.
3. A web-view compatibility host for v1–3 panels is built only if a spike shows
   it is cheap; otherwise migration is the path.

## Parity process

- The inventories are the checklist. Each lane checks off items as they land,
  with a test or a screenshot as evidence.
- The old e2e specs (`apps/desktop/e2e` on `main`) describe expected behavior;
  lanes port the relevant ones to GPUI `TestAppContext` tests.
- Cutover happens only when both inventories are fully checked and an audit
  pass confirms it. Until then the Electron app keeps shipping.

## Lanes and order

0. **Spine** (first, one owner): crate layout, `core` (document via the
   engine, commands, undo, selection, keymap, menu router), engine frames in
   the viewer, open/save `.pmv`.
1. **Wave 1:** `kit`; timeline; inspector; project/layers/library.
2. **Wave 2:** viewer tools and gizmos; effects/easing/graph; export and render
   queue; shell (welcome, projects, Settings, docking); agent panel.
3. **Wave 3:** extensions v4 + migration; MCP + CLI; mixer/captions/
   transcription; cloud and Store; 3D viewport; accessibility and localization.
4. **Cutover:** parity audit, packaging, signing, updates, then retire Electron.

Waves run at most three agents at once (usage budget, and the disk has about
5 GB free), sharing one Cargo target directory (`~/Developer/pm-target`).
