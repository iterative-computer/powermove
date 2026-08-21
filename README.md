# Powermove

Powermove is an AI-native motion and video editor for macOS. This main branch keeps the original compact editing interface and adds ChatGPT subscription access through the signed-in local Codex client.

## What works

- WebGL composition preview, editable layers, keyframes, effects, shaders, media, timeline, inspector, export, and takes
- Prompt-driven composition, motion, shader, and workspace edits
- Compact docked workspaces with editable panels and custom parameter controls
- ChatGPT subscription generation through the signed-in local Codex client; account tokens never enter the web interface

### Production branch additions (`production`)

- **Precomps** — nested compositions with scoped evaluation (⌘⇧C), recursion-capped rendering, comp-aware parenting/expressions/params
- **Layer masks** — animatable rect/ellipse masks with feather, add/subtract modes, applied before effects (AE semantics)
- **Delivery** — Opus audio mixdown muxed into WebM; transparent-background PNG sequence/still export via FBO readback
- **Robustness** — full project-load sanitization (corrupt files degrade instead of NaN-ing the renderer), cycle-safe parenting, audio-node lifecycle fixes, video speed desync fix
- **Performance** — per-frame hierarchy memoization (1000-layer scene: ~17s → ~77ms per second of playback)
- **Diagnostics** — uncaught errors/rejections captured with real stacks to stderr; native boot self-check

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
- `js/agent/` exposes bounded editing tools and provider bridges.
- `native/main.swift` packages the editor as a lightweight WKWebView macOS app and brokers native-only capabilities.

Large projects stay responsive by keeping project state semantic and rendering the composition on the GPU. The earlier recursive-layout architecture is preserved on the `experimental-architecture` branch for future work; it is not the interface shipped by this branch.
