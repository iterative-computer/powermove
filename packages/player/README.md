# @powermove/player

The Powermove web player, bundled for browsers. It is built from the desktop
app's player sources (`apps/desktop/src/renderer/src/player/`) with esbuild;
this package holds no runtime source of its own.

## Entry points

- `@powermove/player/player`: `createPlayer` (WebGL2 canvas player) and
  `createEngine` (scene evaluation without a canvas).
- `@powermove/player/svg-player`: `createSvgPlayer`, an SVG drawing backend
  over the same engine, for scenes made of text, solids, shapes and groups.
- `@powermove/player/build`: `bundlePlayerEntry(entryPath)` and `entries`, the
  single home of the esbuild options. The desktop app's
  `virtual:powermove-player` module (the runtime embedded into web exports)
  calls this so both outputs are byte-identical builds.

## Building

```sh
bun run build   # from packages/player
```

Writes `dist/player.js`, `dist/svg-player.js` and copies the hand-written
declarations from `types/` to `dist/*.d.ts`. `dist/` is generated and
gitignored; rebuild after changing anything under the desktop player sources.

## Consumers

- `apps/desktop`: imports `@powermove/player/build` from
  `scripts/player-bundle.ts` to embed the player text into exported zips.
- `apps/website`: animates its hero with `createSvgPlayer` and a `scene.json`
  exported from the desktop app.
