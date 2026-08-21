# Powermove

Powermove is an AI-native motion and video editor for macOS. This main branch keeps the original compact editing interface and adds ChatGPT subscription access through the signed-in local Codex client.

## What works

- WebGL composition preview, editable layers, keyframes, effects, shaders, media, timeline, inspector, export, and takes
- Prompt-driven composition, motion, shader, and workspace edits
- Compact docked workspaces with editable panels and custom parameter controls
- ChatGPT subscription generation through the signed-in local Codex client; account tokens never enter the web interface

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

- `js/ui/layout.js` renders the compact docked workspace.
- `js/core/workspace.js` owns workspace state and custom controls.
- `js/core/editing.js` is the shared, typed source-edit boundary used by canvas gestures, inspectors, the timeline, agents, and generated controls.
- `js/agent/` exposes bounded editing tools and provider bridges.
- `native/main.swift` packages the editor as a lightweight WKWebView macOS app and brokers native-only capabilities.

Large projects stay responsive by keeping project state semantic and rendering the composition on the GPU. The earlier recursive-layout architecture is preserved on the `experimental-architecture` branch for future work; it is not the interface shipped by this branch.

Generated interfaces never own a second copy of composition state. A custom control can bind to a scene parameter as before, or directly to editable source:

```json
{
  "type": "slider",
  "label": "Selected opacity",
  "target": "$selection",
  "path": "properties.opacity",
  "min": 0,
  "max": 100,
  "def": 100
}
```

Supported binding paths are `properties.*`, `content.*`, and `layer.*`. Generated buttons can submit a `commands` array. Those controls call the same atomic transactions as direct manipulation and agent `edit_source` calls, so undo, validation, hand-edit preservation, revision checks, and provenance stay consistent.
