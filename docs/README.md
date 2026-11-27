# Powermove docs

Design and architecture notes for Powermove V2, the native Rust app. Start with
the [repository README](../README.md) for building and contributing. Paths in
these docs are relative to the repository root unless they are links.

## Architecture

- [Native engine: plan and design](rust-engine.md): the plan the Rust engine
  was built from; its Electron-host sections are historical.
- [pm-kernel](native/kernel.md): the single-writer kernel actor, transactions
  and history. [Document API](gpui/core-api.md) for panels.
- [Native shell](native/shell.md), [app integration](native/integration.md)
  and [application services](native/services.md).
- [Engine service](native/engine-service.md) and the
  [native engine boundary](gpui/engine-boundary.md).
- [Project file guard](native/file-guard.md) and
  [first launch from Powermove 1](native/first-launch.md).
- [Telemetry](telemetry.md): what the app collects.

## Engine

- Contracts: [RFCs](engine/rfcs/README.md) and the Phase 0
  [spike reports](engine/spikes/README.md).
- Correctness: [gate policy](engine/gate-policy.md),
  [quality ledger](engine/quality-ledger.md) (where the engine deliberately
  differs from Powermove 1), [parity corpus](engine/corpus.md).
- Evaluation: [pm-eval](engine/pm-eval.md),
  [layers](engine/pm-eval-layers.md), [pm-edit](engine/pm-edit.md),
  [expressions](engine/expressions.md), [document storage](engine/document-memory.md),
  [write path and pm-wasm](engine/write-path.md).
- Rendering: [pm-render](engine/pm-render.md), [compositing](engine/composite.md),
  [effects and GLSL](engine/effects.md), [shaders](engine/shaders.md),
  [motion](engine/motion.md), [vector](engine/vector.md),
  [text](engine/text.md) and [text rendering](engine/text-render.md).
- Colour: [project colour depth](engine/color-depth.md),
  [8-bit export](engine/color-depth-export.md), [HDR and camera log](engine/hdr.md),
  [video reading](engine/video-parity.md).
- 3D: [scene renderer](engine/scene3d.md), [glTF](engine/gltf.md),
  [Blender](engine/blender.md), [Studio redesign](3d-studio-redesign.md).
- Media: [pm-media](engine/media.md), [input formats](engine/formats.md),
  [media store](engine/media-store.md), [long-GOP scrub](engine/scrub.md).
- Audio and export: [pm-audio](engine/pm-audio.md), [export](engine/export.md),
  [delivery and FFmpeg](engine/delivery.md), [WebM and Opus](engine/export-opus.md).
- Performance: [memory](engine/memory.md), [work counters](engine/perfbench.md),
  [latency notes](engine/perf.md).

## Native app

- Agents: [agents](native/agents.md), [MCP tools](native/mcp.md),
  [Aero inference API](native/aero-inference.md), [Aero eval](native/aero-eval.md).
- Extensions: [v4 API](native/v4-api.md), [UI kit](native/ui-kit.md),
  [extensions in the app](native/extensions-app.md),
  [Store view host](native/store-host.md).
- Powermove 1 compatibility: [compatibility realm](native/compat-realm.md),
  [WebKit host](native/compat-host.md), [remote DOM](native/remote-dom.md),
  [V8 runtime](native/runtime-v8.md), [recorder canvas](native/recorder.md),
  [themes](native/themes.md) and the [theme inventory](native/t1-themes.md);
  [extension compatibility in the engine](engine/extensions-compat.md).
- Panels models: [agent](native/panels/agentpanel.md),
  [inspector](native/panels/inspector.md), [media](native/panels/media.md),
  [timeline](native/panels/timeline.md), [viewer chrome](native/panels/viewerchrome.md).
- Editing and media: [pro editing](native/pro-editing.md),
  [pro media and colour](native/pro-media.md), [Studio document](native/scene3d.md),
  [Studio commands](native/studio3d-command-contract.md).
- Tools: [headless CLI](native/cli.md), [behaviour traces](native/traces.md).
- Parity with Powermove 1: [acceptance harness](native/acceptance.md),
  [workflows](native/w1-workflows.md), [keymap](native/w1-keymap.md),
  [e2e triage](native/e2e-triage.csv), [surface inventory](native/surfaces.md),
  [main delta](native/main-delta.md). Tests read several of these files.

## Design

- [Design language](design.md) and the native
  [design system](native/design/language.md) with its [tokens](native/design/tokens.md).
- [Panel specs](native/design/panels/README.md) and
  [surface specs](native/design/surfaces/README.md);
  [Media Offline](native/design/media-offline.md).
- Mockups: [editor](native/design/editor-mockup-v3.html),
  [panels](native/design/panels-mockup.html).

## Release

- [Releasing Powermove V2](native/release.md).
