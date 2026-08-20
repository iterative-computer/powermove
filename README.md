# Powermove

Powermove is an AI-native motion and video editor for macOS. Its interface is a versioned document: sections can be split, stacked, tabbed, duplicated, floated, overlaid, moved, previewed, undone, and generated from a prompt without flattening the underlying project.

## What works

- WebGL composition preview, editable layers, keyframes, effects, shaders, media, timeline, inspector, export, and takes
- Recursive workspace manifests instead of fixed left/center/right docks
- Tabs, horizontal and vertical splits, floating panels, overlays, native pop-outs, and linked viewer/timeline instances
- Generated control sections with safe bindings to project, selection, and scene parameters
- Reversible generated-workspace previews plus interface-specific undo and redo
- ChatGPT subscription generation through the signed-in local Codex client; account tokens never enter the web interface
- Legacy workspace migration and guards against collapsed or unreachable sections

## Build

Requirements: macOS 13 or newer and Apple Command Line Tools.

```sh
npm test
npm run build:mac
```

The app is created at `build/Powermove.app`. To replace the installed app:

```sh
./scripts/build-macos-app.sh --install
```

The ChatGPT subscription provider requires the Codex CLI to be installed and signed in with ChatGPT.

## Architecture

- `js/core/workspace-schema.js` is the pure, testable workspace document model.
- `js/ui/layout.js` renders that document recursively and owns panel placement.
- `js/core/workspace.js` owns workspace state, previews, history, and generated controls.
- `js/agent/` exposes bounded editing tools and provider bridges.
- `native/main.swift` packages the editor as a lightweight WKWebView macOS app and brokers native-only capabilities.

Large projects stay responsive by keeping project state semantic, rendering the composition on the GPU, limiting DOM work to visible editor surfaces, and treating secondary viewers/timelines as linked mirrors rather than duplicate engines.
