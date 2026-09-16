# Web animation export

Choose **Export → Code → Export code**. Code, Video, Images, and Project are visible choices at the top of the dialog. Code export explains the JavaScript player, React component, and agent handoff, and hides video encoding settings. It packages the full composition, independent of the work area. Users continue authoring visually; there is no project code editor.

The ZIP contains a versioned `scene.json`, a standalone ES-module `player.js` with TypeScript declarations, used media, available web fonts, an interactive browser preview, a React integration component, and integration instructions. Serve the extracted folder over HTTP. Nothing in the exported animation requires Electron, the Powermove app, an account, or a CDN.

To hand the implementation to a coding agent, attach the entire ZIP and paste `HANDOFF_PROMPT.txt`, adding the destination app and desired placement/trigger. `AGENT_HANDOFF.md` guides framework integration and destination testing. `handoff.json` provides the composition dimensions, duration, exact parameter and root text-layer IDs, generated rendering dependencies, warnings, and suggested frame-comparison times. It does not automatically send the project to an agent or overwrite the destination app's instructions.

```js
import { createPlayer } from './animation/player.js';

const player = await createPlayer({
  canvas: document.querySelector('canvas'),
  scene: new URL('./animation/scene.json', import.meta.url),
  transparent: true,
  onError: console.error,
});

button.onclick = () => player.play();
await player.seek(0.5);
// On unmount:
player.destroy();
```

`play`, `pause`, `seek` (seconds), and `loop` control playback. `setParameter` updates existing composition parameters referenced by expressions. `setText` overrides a root text layer by its stable ID. Every player owns a private scene copy. Canvas CSS controls display size; the drawing buffer follows that size and screen pixel density, within GPU limits. Vector layers are rasterized again at the resulting resolution, including after resizing or browser zoom. The React example owns a separate canvas per effect lifecycle, including Strict Mode cleanup.

## Rendering and generated effects

The player reuses the editor's animation evaluator, rasterizer, compositor, audio engine, and shader generators. It retains layers, keyframes, easing, expressions, paths, masks, parent transforms, nested compositions, effects, transitions, and shader source as structured data. It does not flatten the animation into frames.

Used effect, transition, and extension-layer definitions are captured from the active kernel registry. Generated effects therefore carry their shader bodies and parameter definitions into the package; the player registers those definitions before rendering. Raw shader layers preserve their code and channels, and the player reconstructs uniform metadata without an inspector. Generated editor panels and arbitrary extension JavaScript are not part of the playback runtime.

## Compatibility

- Requires a browser with WebGL2 and ES modules. This is a web/canvas integration, not a native iOS/Android renderer or per-layer React DOM output.
- Original media is bundled; playback depends on browser codec support. Desktop-only video transcoding is unavailable in the exported player.
- Fonts accessible through the editor's CSS font-face rules are bundled. System fonts must be provided by the host app; the preview and README list missing font families. Their absence can change text layout. Variable-font rendering uses bundled font bytes without requesting local-font access.
- Audio playback may require a user gesture. The runtime supports audio disabling through `audio: false`.
- Missing rendering definitions, unsupported expressions, missing media, and composition cycles produce errors. Runtime shader failures are surfaced instead of silently dropping the failing effect.
- Export size has no application quota. The current classic ZIP writer requires archives below 4 GiB and fewer than 65,535 entries; larger archives require ZIP64 support. Packaging currently retains media and the archive in memory, so available memory is a practical constraint.
- Project notes, edit history, workspace state, unused media, and local asset storage paths are excluded. The `.pmv` save format is unchanged.

## Implementation

`src/renderer/src/player/export-web.ts` freezes and packages source. `player.ts` creates isolated rendering engines and controls their lifetime. `scripts/player-bundle.ts` uses esbuild through a Vite virtual module to embed the matching standalone runtime in each app build; it adds no production dependency. The export format and existing save bridge integrate through `legacy/core/exporter.ts`.

`e2e/export-code.spec.ts` compares editor frames with an independent Chromium browser, validates ZIP integrity, generated effects/layers/shaders/transitions, nested compositions, bundled images and video, audio scheduling, transparency, playback, runtime updates, and native save/cancel behavior. `e2e/export-dialog.spec.ts` covers the format selector and field visibility. Unit tests cover source ownership, metadata exclusion, dependency errors, archive paths, packaged variable-font loading/cleanup, and format settings.
