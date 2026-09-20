# Powermove

The video editor that rewrites itself around your work.

[![Powermove launch video](assets/brand/launch.gif)](https://www.youtube.com/watch?v=r2t1dhxHjkQ)

[Watch the launch video](https://www.youtube.com/watch?v=r2t1dhxHjkQ) · [trypowermove.com](https://trypowermove.com)

Powermove is an iterative video editor for macOS. Ask an agent to add or fork panels, effects, and workflows, then load them into the running app. If it is wrong, steer it and try again. Every change can be undone.

The kernel is intentionally tiny. Timeline, effects, inspector, the UI, all of it lives in extensions on top. The Powermove agent writes extensions the same way we built the rest of the editor, through a typed, undoable tool layer that can inspect, render, and transactionally edit the live project.

This repository is a bun workspace monorepo containing the desktop editor, the marketing website, and the packages they share.

## Get Powermove

Download the Mac app from [GitHub Releases](https://github.com/iterative-computer/powermove/releases/latest) or [trypowermove.com](https://trypowermove.com). Apple Silicon.

### Run it on another machine

If you also want Powermove on a box you leave on, a Linux server or a spare Mac, the `powermove-cli` package runs the same editor as a host you use from a browser on any device. The agents run on the host, so a request keeps going after you close your laptop, and edits, media and fonts stay in sync across every device with the project open.

```sh
npx powermove-cli@latest serve      # try it once, nothing installed
```

To keep it running as a service and reach it over Tailscale, see [packages/cli/README.md](packages/cli/README.md). Architecture in [apps/desktop/docs/remote-serve.md](apps/desktop/docs/remote-serve.md).

## Repository layout

```text
apps/
  desktop/            @powermove/desktop   Electron editor (Svelte 5, electron-vite, Vitest, Playwright)
  www/                @powermove/www       SvelteKit marketing site (adapter-static)
packages/
  tokens/             @powermove/tokens    tokens.css, the design tokens shared by desktop and website
  player/             @powermove/player    browser bundle of the web player and SVG player, built from the desktop sources
  macos-haptics/      @powermove/macos-haptics   native macOS alignment-haptics addon
  cli/                powermove            `npx powermove serve`: the editor host for any Linux or macOS box, used from a browser
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
bun run serve          # build the remote host and run it here; open the printed URL in a browser
```

To run a script in one workspace, use `bun run --cwd apps/desktop <script>` (or `cd` into the workspace). Run the desktop gates with `bun run test`, not `bunx vitest`: the latter runs Vitest under bun's runtime and skews a calibrated performance test.

## Further reading

- [Desktop editor](apps/desktop/README.md): architecture, kernel and extensions, project files, development, and release notes.
- [Website](apps/www/README.md): the SvelteKit marketing site.
- [Design language](docs/design.md): the implementation guide for every Powermove interface.
- [macOS release process](apps/desktop/docs/release.md): signing, notarization, and the beta workflow.
- [Remote host](apps/desktop/docs/remote-serve.md): how `powermove serve` runs the editor without Electron and what changes in a browser.

## License

Powermove is free software, licensed under the [GNU General Public License v3.0 or later](LICENSE).
