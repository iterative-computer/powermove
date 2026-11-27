# @powermove/player

The Powermove web player, bundled for browsers. It is built with esbuild from
the Electron app's player sources (`apps/desktop/src/renderer/src/player/`) at
the commit pinned in `build.mjs`, read through `git archive` (the Electron app
ships from main and is not on this branch). This package holds no runtime
source of its own.

## Entry points

- `@powermove/player/player`: `createPlayer` (WebGL2 canvas player) and
  `createEngine` (scene evaluation without a canvas).
- `@powermove/player/svg-player`: `createSvgPlayer`, an SVG drawing backend
  over the same engine, for scenes made of text, solids, shapes and groups.

## Building

```sh
bun run build   # from packages/player
```

Writes `dist/player.js`, `dist/svg-player.js` and copies the hand-written
declarations from `types/` to `dist/*.d.ts`. `dist/` and the extracted
sources (`.v1-src/`) are generated and gitignored. Change the pinned commit to
pick up different player sources.

## Consumers

- `apps/www`: animates its hero with `createSvgPlayer` and a `scene.json`
  exported from the desktop app.
