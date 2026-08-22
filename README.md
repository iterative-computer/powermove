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
- `js/core/editing.js` is the shared, typed source-edit boundary used by canvas gestures, inspectors, the timeline, agents, and generated controls.
- `js/assistant/harness.js` runs bounded composition observation, source edits, rendered review, repairs, and checkpoints.
- `js/assistant/spatial.js` owns Ripple selection, generated-section proposals, preview, and final user approval.
- Shake opens its floating agent prompt immediately; `Command-Shift-K` opens the same prompt as a keyboard fallback. Circling remains optional context.
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

Supported layer binding paths are `properties.*`, `content.*`, and `layer.*`. Composition background controls use `target: "$composition"` with `composition.background.*` paths. Generated sections reject disconnected controls instead of creating inert parameters. Generated buttons can submit a `commands` array. Those controls call the same atomic transactions as direct manipulation and agent source edits, so undo, validation, hand-edit preservation, revision checks, and provenance stay consistent.

A single prompt can also create a complete workspace manifest with multiple docks, built-in panels, and generated sections. The manifest is bounded and normalized before preview: unknown panels are removed, Composition remains reachable, generated sections are placed into a dock, and every non-button control must resolve to real editable source.

For composition work, the harness observes semantic project state and real rendered frames, proposes only typed source edits, creates a rollback checkpoint, applies with a revision check, visually reviews the rendered result, performs at most two bounded repair passes, and finishes with explicit **Keep change** or **Undo change** controls.
