<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/powermove-light.svg">
  <img src="assets/brand/powermove-dark.svg" alt="Powermove" width="88">
</picture>

# Powermove

**Shape your video editor.**

A native motion and video editor for macOS that you reshape by asking.<br>
Connect an agent, describe the change, and it lands in the running app as an undoable edit.

[Website](https://trypowermove.com) · [Download for Mac](https://github.com/iterative-computer/powermove/releases/latest) · [Discord](https://discord.gg/6ebptZF3Ch) · [Launch video](https://www.youtube.com/watch?v=r2t1dhxHjkQ)

[![macOS on Apple silicon](https://img.shields.io/badge/macOS-Apple%20silicon-111111?style=flat-square&logo=apple)](https://trypowermove.com)
[![Rust 1.96.1](https://img.shields.io/badge/Rust-1.96.1-111111?style=flat-square&logo=rust)](apps/native/rust-toolchain.toml)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-111111?style=flat-square)](LICENSE)

</div>

<br>

<p align="center">
  <img src="assets/readme/editor.jpg" alt="The Powermove editor with a project called Summer in Lisbon: a graded sunset over Lisbon with the title in the viewer, a Layers timeline with captions, title keyframes, lower thirds and shots, a Grade layer's effects, and an Aero agent turn that warmed the sunset and added a glow to the title." width="100%">
</p>

<br>

## Welcome to malleable software

Most editors ship a fixed feature set and a plugin SDK for the brave. Powermove
inverts that. It is a real motion editor, with an agent that adds what it's
missing.

- **An editor you can rewrite.** Connect a coding agent and do basically anything:
  make extensions, restyle the interface, or generate effects and shaders.
- **A deliberately tiny kernel.** Everything above it is an extension, including
  the parts we wrote. The agent builds new ones through the same typed, undoable
  tool layer.
- **Native all the way down.** Powermove V2 is written in Rust: a GPUI interface
  in native macOS windows, a single-writer kernel, and a GPU engine on Metal.
  No Electron and no Chromium.
- **Every change is reversible.** Agent edits go through the same transactions and
  history as your own, and **Undo Extension Change** takes back a new or changed
  extension.

```text
 ┌─────────────────────────────────────────────────────────────────────────┐
 │  Your mods                Yours to add. Same API as everything below    │
 ├─────────────────────────────────────────────────────────────────────────┤
 │  Panels                   Timeline, inspector, agent, media             │
 ├─────────────────────────────────────────────────────────────────────────┤
 │  Effects and transitions  Blur, grain, whip pan, dissolve               │
 ├─────────────────────────────────────────────────────────────────────────┤
 │  Theme and keymap         Your look, your shortcuts                     │
 ╞═════════════════════════════════════════════════════════════════════════╡
 │  Kernel                   Project store, typed edits, undo, compositor  │
 └─────────────────────────────────────────────────────────────────────────┘
```

## A tour

| | |
| --- | --- |
| **A real motion editor** | Layers, keyframes, curves and effects, all on a timeline, with the agent in a panel beside them. |
| **Pro editing** | Ripple trims and deletes, slip, slide, split and shuttle, with keymap presets for After Effects (the default), Premiere Pro, Final Cut Pro and DaVinci Resolve. |
| **Effects** | GLSL effects and transitions you can fork, or have the agent write new ones from a description. |
| **Media** | FFmpeg (our own LGPL build) and VideoToolbox decoding, proxies and a render cache. |
| **Captions** | Speech to text that runs on your Mac. |
| **3D** | A 3D Studio for scenes inside your compositions. |
| **Export** | MP4, ProRes and WebM, rendered in a separate process so the editor stays responsive. |
| **Only what you need** | Turn extensions off in Settings when you don't need them, and switch them back on whenever you do. |

Coming from Powermove 1? On first launch the app copies your library and extensions
across. Your original profile and projects are only ever read.

## Agents

Powermove works with three agents. Each one edits your project through the same
tool layer, renders frames to check its own work, and leaves every change undoable.

| Agent | What it is | Sign-in |
| --- | --- | --- |
| **Aero** | Powermove's own agent, on Powermove inference. Pick a model and effort per thread. | Your Powermove account, with Aerolite |
| **Claude Code** | Anthropic's coding agent, bundled with the app. | Your own Claude Code subscription |
| **Codex** | OpenAI's coding agent, bundled with the app. | Your own Codex subscription |

Claude Code and Codex keep their own sign-ins and keys.

### Plans

**The editor is free.** Plans add our inference, and soon the cloud.

| | Free | Aerolite | Aero *(coming soon)* |
| --- | --- | --- | --- |
| **Price** | $0 | $9.99 per month | $19.99 per month |
| **For** | The whole editor, with your own agent | Aero, our agent, with inference included | Editors who lean on Aero every day |
| **Includes** | Every tool, effect and export · extensions from the Store, or built by the agent · bring your Claude Code or Codex subscription | Everything in Free, plus 1,000 credits every month · review links *(soon)* · cloud projects *(soon)* | Everything in Aerolite, plus 2,500 credits every month · early access · priority support · more cloud storage *(soon)* |

Prices are in US dollars. Credits pay for Aero's inference and don't roll over.
The details are in [docs/native/aero-inference.md](docs/native/aero-inference.md).

## Extensions and the Store

An extension is a folder with a manifest and a TSX entry point:

```json
{ "id": "hello-panel", "name": "Hello", "version": "1.0.0", "apiVersion": 4,
  "entry": "index.tsx", "author": "agent" }
```

Panels are written in JSX against the `powermove-v4` API, run in their own V8
isolate, and draw natively. When an agent publishes a change, Powermove checks
it against the files, scans for credentials, compiles each extension and its
shaders, and runs it once in a throwaway runtime. Only then does it go live, in
every open window, within a second.

The **Store** is where extensions are shared. Store extensions run in a separate,
sandboxed helper process with their own storage, apart from the editor and from
each other. Extensions written for Powermove 1 run in a compatibility realm.

- Authoring guide: [apps/native/js/v4/api-pack/EXTENSIONS.md](apps/native/js/v4/api-pack/EXTENSIONS.md)
- A sample to start from: [apps/native/js/v4/api-pack/samples/hello-panel](apps/native/js/v4/api-pack/samples/hello-panel)
- The API: [docs/native/v4-api.md](docs/native/v4-api.md)

## Architecture

```mermaid
flowchart TB
  subgraph app["Powermove.app"]
    direction TB
    shell["<b>Shell</b><br/>GPUI views in native macOS windows"]
    panels["<b>Built-in panels</b><br/>timeline · viewer · inspector · effects<br/>export · agent · color · easing · project"]
    mods["<b>Your extensions</b><br/>v4 API, plus Powermove 1 compatibility"]
    v8["<b>Extension runtime</b><br/>V8 isolates, sandbox enabled"]
    agents["<b>Agents</b><br/>Aero · Claude Code · Codex"]
    mcp["<b>Tool layer</b><br/>typed, undoable MCP tools"]
    kernel["<b>Kernel</b><br/>single writer: document, transactions,<br/>history, journal, library"]
    engine["<b>Engine</b><br/>render on Metal · media · audio · text · 3D · export"]
  end
  store["<b>Store helper</b><br/>sandboxed process per Store extension"]
  cloud["<b>Powermove Cloud</b><br/>accounts · Store registry · billing · Aero inference"]

  shell --> panels
  panels --> v8
  mods --> v8
  v8 --> kernel
  agents --> mcp --> kernel
  kernel --> engine
  mods -.-> store
  store -.-> cloud
  agents -.->|Aero| cloud
```

The kernel is the only writer to a project. Panels, extensions and agents all
reach it through typed transactions, which is why every edit, from a drag on the
timeline to an agent's batch of changes, shares one history and one undo.

Further reading: [native engine design](docs/rust-engine.md) ·
[kernel](docs/native/kernel.md) · [shell](docs/native/shell.md) ·
[MCP tools](docs/native/mcp.md) · [design language](docs/design.md) ·
[all docs](docs/README.md)

## Repository layout

```text
apps/
  native/        The macOS app: a Rust workspace (crates/), the JS that runs inside it (js/),
                 vendored GPUI, taffy and Aero's libfx core (vendor/), bundle and packaging
  cloud/         Powermove Cloud: a Hono Worker on Cloudflare for accounts, the Store,
                 billing (Stripe) and Aero inference
  www/           trypowermove.com, a prerendered SvelteKit site
  admin/         The admin panel, SvelteKit on Cloudflare Workers
engine/          The native engine, a second Rust workspace: render, media, audio, text,
                 color, export, 3D, plus a pinned LGPL FFmpeg build (third_party/)
packages/
  registry/      The Store contract shared by the app and the cloud: manifest, snapshots, wire schemas
  tokens/        Design tokens shared by the website and the admin panel
  player/        The web player bundle the website animates its hero with
  macos-haptics/ Native macOS haptics addon
tools/           Verification tooling: performance gates, visual parity, extension checks
docs/            Design language, engine and native architecture notes, telemetry
```

<details>
<summary><b>Inside <code>apps/native/crates</code></b></summary>

| Area | Crates |
| --- | --- |
| App and interface | `app` (the `powermove-native` binary), `shell`, `design`, `surface`, `first-launch`, `update-policy` |
| Project model | `kernel`, `migrate`, `services` |
| Panels | `builtins`, `panel-timeline`, `panel-inspector`, `panel-media`, `panel-viewerchrome`, `panel-agentpanel`, `panel-studio3d` |
| Engine bridge | `engine-service`, `promedia`, `scene3d`, `studio3d-engine`, `transcribe` |
| Extensions | `runtime-v8`, `compat-runner`, `compat-host`, `remote-dom`, `store-host`, `recorder` |
| Agents and automation | `agents`, `aero-core`, `aero-eval`, `mcp` |
| Quality and telemetry | `acceptance`, `telemetry` |

</details>

## Build from source

Powermove builds on **macOS 14 or later on Apple silicon**. You'll need:

- Xcode command line tools: `xcode-select --install`
- [rustup](https://rustup.rs). Each Rust workspace pins **Rust 1.96.1** in
  `rust-toolchain.toml`, and rustup installs it on first use.
- [bun](https://bun.sh) **1.3.14**, the only package manager in the repo. The Rust
  build also runs it to bundle the built-in panels.
- The tools for the pinned media libraries: `brew install cmake ninja pkg-config autoconf automake libtool xz python`

**1. Set up the environment and JavaScript dependencies**

```sh
source engine/env.sh     # rustup first on PATH, build settings, PM_FFMPEG_DIR
export CARGO_TARGET_DIR="$PWD/target"   # one target folder, so helpers land beside the app

bun install --frozen-lockfile
for d in v4 compat remote-dom; do (cd "apps/native/js/$d" && bun install); done
```

**2. Build the pinned dependencies (once)**

```sh
# Powermove's LGPL-only FFmpeg, reproducible from checksummed sources
PM_FFMPEG_OUT="$PM_FFMPEG_DIR" engine/third_party/ffmpeg/build.sh

# The pinned Zig for Aero's core. Without it, Aero builds as a stub that reports itself unavailable.
apps/native/vendor/fx/install-zig.sh
```

The extension runtime links rusty_v8 152.2.0 with V8's sandbox enabled. Build that
archive with the [`Native V8 sandbox spike`](.github/workflows/native-v8.yml)
workflow, then point the build at its artifact:

```sh
export RUSTY_V8_ARCHIVE=/path/to/librusty_v8.a
export RUSTY_V8_SRC_BINDING_PATH=/path/to/src_binding.rs
```

**3. Build the app and its helpers**

The first build also compiles pinned dav1d, OpenEXR and JPEG libraries into a cache
under `~/Library/Caches/powermove-engine`.

```sh
(cd apps/native && cargo build --locked -p pm-native-app)
(cd apps/native && cargo build --locked -p pm-runtime-v8 --features macos-sandbox --bin pm-store-helper)
(cd engine && cargo build --locked -p pm-glsl --bin pm-glsl-compiler)
```

`pm-glsl-compiler` compiles effect shaders, and `pm-store-helper` hosts Store
extensions. Both must sit next to the app's executable.

**4. Run it**

```sh
apps/native/scripts/bundle-dev.sh "$CARGO_TARGET_DIR/debug/powermove-native" build/dev
build/dev/Powermove.app/Contents/MacOS/powermove-native --profile "$HOME/Powermove Dev"
```

`bundle-dev.sh` wraps the binary in a development `Powermove.app` with the helpers,
the bundle resources and your installed `claude` and `codex` CLIs. A development
build runs only on a profile folder you give it, so it never opens your real library.

### Tests and the other apps

```sh
(cd apps/native && cargo test --locked -p pm-kernel)   # any native crate
(cd apps/native/js/v4 && bun test)                     # the extension API and built-in panels
bun run test                                           # every JavaScript workspace
bun run typecheck

bun run dev:www                                        # the website on http://localhost:5173
(cd apps/cloud && bun run local:billing)               # the cloud, Stripe and AI stand-ins, run locally
```

The cloud's local stack needs Docker. Its [README](apps/cloud/README.md) covers
the database, migrations and the local Stripe and AI stand-ins. The
[admin panel](apps/admin/README.md) runs against the same stack.

## Contributing

Issues and pull requests are welcome. For anything large, open an issue or come
talk in [Discord](https://discord.gg/6ebptZF3Ch) first.

- Work on a branch (`feat/…`, `fix/…`), never on `main`. Pull requests are merged,
  not squashed, so make each commit a step that stands on its own.
- Run the checks for what you touched: `cargo fmt`, `cargo clippy` and
  `cargo test` in the Rust workspaces, `bun run typecheck` and `bun run test` for
  JavaScript.
- Interface work follows the [design language](docs/design.md). Nested corners stay
  concentric: `Outer R = Inner R + Padding`.
- Read [AGENTS.md](AGENTS.md) before your first commit. It is short, and it applies
  to people as much as to agents.

## Security

This repository is public. Never commit credentials, session cookies or tokens,
private keys, database URLs, user data or private reports. Configuration that
must stay secret lives in Worker and GitHub secrets, not in Git.

Turn on the secret-scanning hook once per clone:

```sh
git config core.hooksPath .githooks
```

It runs [gitleaks](https://github.com/gitleaks/gitleaks) on every commit. Run it
by hand with `gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow`.
If a real secret is ever exposed, revoke or rotate it: deleting the file does not
remove it from history.

**Reporting a vulnerability:** please email
[hello@iterative.computer](mailto:hello@iterative.computer) with "Security" in the
subject, instead of opening a public issue.

## Privacy

Powermove sends anonymous usage, crash, hang and performance reports so we can fix
what breaks. Turn it off in **Settings › Privacy**. Every event and property it can
send is listed in one file; [docs/telemetry.md](docs/telemetry.md) explains what is
collected and what never is.

## License

Powermove is free software: the app, the engine, the cloud, the website and the
packages are licensed under the
[GNU Affero General Public License v3.0 or later](LICENSE). Vendored third-party
code keeps its own license, in the `LICENSE` and `POWERMOVE.md` files beside it.

<br>

<div align="center">
<sub>Made by <a href="https://iterative.computer">Iterative Computer</a> · <a href="https://trypowermove.com">trypowermove.com</a></sub>
</div>
