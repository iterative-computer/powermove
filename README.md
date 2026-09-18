# Powermove

The video editor that rewrites itself around your work.

![Powermove launch video](assets/brand/launch.gif)

[Watch the launch video](assets/brand/launch.mp4) · [trypowermove.com](https://trypowermove.com)

Powermove is an iterative video editor for macOS. Ask an agent to add or fork panels, effects, and workflows, then load them into the running app. If it is wrong, steer it and try again. Every change can be undone.

The kernel is intentionally tiny. Timeline, effects, inspector, the UI, all of it lives in extensions on top. The Powermove agent writes extensions the same way we built the rest of the editor, through a typed, undoable tool layer that can inspect, render, and transactionally edit the live project.

This repository is a bun workspace monorepo containing the desktop editor, the marketing website, and the packages they share.

## Repository layout

```text
apps/
  desktop/            @powermove/desktop   Electron editor (Svelte 5, electron-vite, Vitest, Playwright)
  www/                @powermove/www       SvelteKit marketing site (adapter-static)
packages/
  tokens/             @powermove/tokens    tokens.css, the design tokens shared by desktop and website
  player/             @powermove/player    browser bundle of the web player and SVG player, built from the desktop sources
  macos-haptics/      @powermove/macos-haptics   native macOS alignment-haptics addon
assets/
  brand/              powermove-light.svg, powermove-dark.svg, powermove-light.png
docs/
  design.md           Powermove design language
```

Each workspace has its own `package.json`; the root `package.json` only holds workspace scripts and the bun toolchain pin.

## Requirements

- macOS on Apple Silicon.
- [bun](https://bun.sh) 1.3 or newer (the repo pins `bun@1.3.14`). bun is the only package manager; `bun.lock` is the only lockfile.
- Xcode command line tools, for building the `@powermove/macos-haptics` native addon during install.
- For agent features in the desktop app: the Codex CLI or Claude Code, installed and signed in.

## Quickstart

```sh
bun install            # installs every workspace and builds the native addon
bun run dev            # desktop editor: Electron + renderer hot reload
bun run dev:www        # website dev server
bun run build          # build every workspace
bun run test           # run every workspace's test script
bun run typecheck      # run every workspace's typecheck script
bun run test:e2e       # desktop Playwright Electron coverage
bun run dist:mac       # ad-hoc signed arm64 DMG, ZIP, and app in apps/desktop/dist/
bun run dist:release   # guarded Developer ID, notarized release lane
```

To run a script in one workspace, use `bun run --cwd apps/desktop <script>` (or `cd` into the workspace). Run the desktop gates with `bun run test`, not `bunx vitest`: the latter runs Vitest under bun's runtime and skews a calibrated performance test.

## Further reading

- [Desktop editor](apps/desktop/README.md): architecture, kernel and extensions, project files, development, and release notes.
- [Website](apps/www/README.md): the SvelteKit marketing site.
- [Design language](docs/design.md): the implementation guide for every Powermove interface.
- [macOS release process](apps/desktop/docs/release.md): signing, notarization, and the beta workflow.

## License

Powermove is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE).
