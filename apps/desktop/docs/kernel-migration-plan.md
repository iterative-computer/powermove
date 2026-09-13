# Kernel migration: built-ins off the legacy `PM` registry

Status: phases 0-4 complete (2026-09-12). Owner: orchestrator (Fable). Implementers: Sol (codex).
Every phase leaves the branch green on the full gate battery (section 7).

## 1. Why

`docs/extensions-design.md` phase 9 moved timeline, viewer, and inspector into
`src/extensions/*` but left their internals on `api.host.pm`, the untyped legacy
registry. Measured on 2026-09-12:

| Extension | `PM.*` references | typed `api.*` references |
|---|---:|---:|
| timeline | 467 | 2 |
| viewer | 286 | 2 |
| inspector | 221 | 32 |

Consequences: the built-ins are not real examples of the public API, agents that
copy them learn unstable globals, the typed API is missing the contracts real
extensions need (drag payloads, dock placement, evaluation, gestures), and the
viewer does not compile through the forkable pipeline at all (boundary escape +
css parse error in `compiler.test.ts`).

The coupling is bidirectional. Legacy calls back into extension-owned members:
`PM.TL` (graph, scrollY, scrollT, pps, keySelectionActive, frameView), `PM.Viewer`
(pan, zoom, showControls, shown, preview, layout, fit, deferNavigationRender),
`PM.Inspector.refresh`, `PM.syncShaderUniforms`, and `PM.tool/toolShape/setTool`.
`legacy/ui/shortcuts.ts` imports timeline modules directly. (An earlier draft
listed a timeline override of `PM.allProps`; verified in Phase 2, the timeline
only reads it.)

## 2. End state

- `src/extensions/{timeline,viewer,inspector,toolbar,keymap-default}` contain zero
  references to `api.host.pm` or `api.host.state`. The boundary lint enforces it.
- Every built-in compiles through `compileExtension` (the forkable pipeline).
- Legacy never references an extension-owned member. Extensions publish typed
  services through the kernel; legacy consumes them null-safely.
- `api.ts` gains the namespaces in section 4. `apiVersion` stays 1 (additive).
- `docs/EXTENSIONS.md` documents the new namespaces. A sample extension
  demonstrates asset import + timeline drop through the typed API only.
- `api.host.pm` remains for user extensions but is documented as deprecated.

Out of scope this round: renaming or splitting `src/renderer/src/legacy`. That is
cosmetic once the boundary is real and can follow separately.

## 3. Phases

### Phase 0: green baseline
Main is red: 18 failing unit tests, 16 typecheck errors, 4 boundary violations,
4 of 7 `e2e/extensions.spec.ts` cases failing. Fix or explicitly account for each.
Boundary violations and the viewer compile case move to Phase 1 (they need API
additions); everything else must be green here.

### Phase 1: freeze contracts
Add the namespaces in section 4 to `kernel/api.ts` with adapters in
`kernel/install.ts` over the existing legacy modules. Unit tests per adapter.
Resolve the four boundary escapes by exposing `space-3d`, `controls/gesture`,
and timeline property helpers through the API. Viewer css fix in the compiler.
No extension code changes yet except the minimal import swaps that close the
boundary violations. After this phase `api.ts` is frozen for the fan-out.

### Phase 2: invert the reverse seams
Legacy stops touching `PM.TL`, `PM.Viewer`, `PM.Inspector`,
`PM.syncShaderUniforms`, `PM.tool*`. It reads them through
`kernel.services.get(...)` with typed interfaces and null-safe defaults.
Extensions register the services. `shortcuts.ts` drops its timeline imports.
Gate adds: `rg "PM\.(TL|Viewer|Inspector|syncShaderUniforms|setTool|toolShape)\b" src/renderer` returns 0.

### Phase 3: migrate the built-ins (parallel, disjoint folders)
- 3a timeline, 3b viewer, 3c inspector, 3d toolbar + keymap-default.
Each rewrites `PM.*` to `api.*`, ends at zero `host.pm`/`host.state` in its
folder, compiles standalone, ports every test. Missing API discovered here is
reported back, never worked around with the escape hatch. Contracts are frozen;
gaps become a Phase 3.5 API patch owned by the orchestrator.

### Phase 4: enforce and teach
Boundary lint forbids `host.pm` and `host.state` in `src/extensions`.
`compiler.test.ts` compiles every built-in. `docs/EXTENSIONS.md` gains the new
namespaces. Sample `media-browser` extension: search a mocked source, import an
asset, drag onto the timeline, undo, using typed API only. Api pack ships it.

## 4. Contract additions to `PowermoveAPI` (apiVersion 1, additive)

Member lists come from measured usage in the three built-ins and legacy
back-references. Names below are final unless the orchestrator amends this file.

| Namespace | Members | Backed by |
|---|---|---|
| `api.anim` | `ev, evP, active, findProp, allProps, hasKeyAt, setKey, setKeyOn, removeKey, applyEaseTo, wouldCycle, resolveContent, expressionErrors, version(), touch, worldMatrix, localMatrix, transformParentMatrix, mul` | `legacy/core/anim.ts` |
| `api.model` | `P, CH, KF, BLENDS, TYPE_META, MASK_SHAPES, mkLayer, mkMask, mkProject, layerDefinition, curComp, layer(id) (was L), byName` | `legacy/core/model.ts`, `kernel/install.ts` |
| `api.selection` | `get(), layers(), first(), keys(), chan(), set(partial), select(ids, add), resolveSelectedKeys, keySelectionActive get/set` | `legacy/core/model.ts`, `selection.ts` |
| `api.groups` | `ancestors, transformRoots, span, expand, normalizeStack, moveToGroup` | `legacy/core/layer-groups.ts` |
| `api.transport` | `time(), setTime(t, opt?), play, pause, toggle, playing(), step, quality get/set, perf, invalidate(what?), previewResolution get/set` | `legacy/core/engine.ts`, `util.ts` |
| `api.history` | `do, begin, commit, cancel, undo, redo, external, selection` | `legacy/core/history.ts` |
| `api.edit` | `apply, begin, commit, cancel, dispatch, mutate` | `legacy/core/editing.ts` |
| `api.media` | `timing.{isTimed, rate, earliestStart}, importFiles, commandForAsset, audio.drawWaveform, assets (raster asset store: get/add/kind), fonts` | `media.ts`, `app.ts`, `shortcuts.ts`, `audio.ts`, `raster.ts`, `fonts.ts` |
| `api.render` | `gl.{bounds, pick, init, resize, previewViewport, context}, raster, renderFrameTo, snapshot` | `legacy/gl/*`, `engine.ts` |
| `api.uiState` | `getLayerCollapsed, setLayerCollapsed, getKeyHandles, setKeyHandles, getFxOpen, setFxOpen, getReveal, setReveal, setShaderMeta` | `legacy/core/ui-state.ts` |
| `api.ui` (additions) | `drag, closeMenus, showLayerMenu, showParentMenu, beginParentPick, openShaderEditor, gesture (EditGesture class)` | `util.ts`, overlays, `layer-menu.ts`, `parent-pickwhip.ts`, `controls/gesture.ts` |
| `api.dnd` | `ASSET_MIME, FX_MIME, startAssetDrag(dt, payload), mediaDrag get/set, hasAssetDrag, hasFileDrag, hasMediaDrag, readAssetDrag, hasFxDrag, readFxDrag, applyFxDrop` with `AssetDragPayload {id,name,kind,dur?}` and `FxDragPayload {kind,id,label}` | `fx/drop.ts`, `panels/install.ts`, `AssetsPanel.svelte` |
| `api.workspace` | `current(), mutate(fn), hasPanel, addPanel(id, dock, index?), movePanel(id, dock, index), removePanel, hidePanel, restorePanel, refresh` | `legacy/core/workspace.ts`, `layout/model.ts` |
| `api.panels.open` | gains optional `{ dock, index }` | same |
| `api.util` | `round, clamp, lerp, snapF, tc, parseTc, uid, hex2rgb, rgb2hex` | `util.ts` |
| `api.ease` | `nameOf, PRESETS` | `easing.ts` |
| `api.space3d` | `CHANNELS_3D, local3D, parent3D, world3D, is3DLayer, perspectiveAmount, planeMatrix, projectPoint, inversePlane, planeContains` (PM parameter removed; adapters bind it) | `legacy/core/space-3d.ts` |
| `api.services` | `register<T>(name, impl): Disposable`, `get<T>(name): T \| null` with declared interfaces `TimelineService`, `ViewerService`, `InspectorService`, `ToolService`, `ShaderHooks` | new, `kernel/services.ts` |
| `api.commands` (additions) | timeline registers `timeline.revealProperty:*` and `timeline.adjacentKeyframe:{prev,next}` commands so keymap-default and legacy shortcuts bind by id | timeline |

Service interfaces (consumed by legacy through the kernel, provided by extensions):

- `TimelineService`: `graph, cv, pps, scrollY, scrollT, keySelectionActive, frameView(), reveal(layer, keys: string[])`.
- `ViewerService`: `pan, zoom, fit, shown, showControls, preview, layout(panOnly?), stage, ov, attach, worldBounds, snapshotSnapCandidates, deferNavigationRender`.
- `InspectorService`: `refresh, syncs, focusText, copySelectedEffects, pasteCopiedEffects, body`.
- `ToolService`: `tool, toolShape, setTool`.
- `ShaderHooks`: `syncShaderUniforms`.

Legacy reads every service through a null-safe getter. Nothing in legacy may
assume a service exists (the e2e "timeline off and on" case depends on this).

Signature sheet used for Phase 1: `/tmp/luna-sigs-report.md` (Luna, 2026-09-12), derived from the code; the code wins on any conflict.

## 5. File ownership

| Phase | May edit | May not edit |
|---|---|---|
| 0 | failing tests and the code they cover; typecheck offenders; e2e extensions spec and what it exercises | `kernel/api.ts`, extension folders except for test-only fixes |
| 1 | `kernel/api.ts`, `kernel/install.ts`, new `kernel/services.ts`, `kernel/*.test.ts`, `main/extensions/compiler.ts`, the four boundary-escape import sites | everything else in extensions |
| 2 | `src/renderer/src/legacy/**`, `kernel/services.ts` wiring, extension `index.ts` files (service registration only) | extension internals |
| 3a | `src/extensions/timeline/**` | everything else |
| 3b | `src/extensions/viewer/**` | everything else |
| 3c | `src/extensions/inspector/**` | everything else |
| 3d | `src/extensions/toolbar/**`, `src/extensions/keymap-default/**` | everything else |
| 4 | `scripts/check-boundaries.mjs`, `compiler.test.ts`, `docs/EXTENSIONS.md`, new sample extension | extension internals |

Shared files (`api.ts`, `install.ts`, `services.ts`) belong to the orchestrator
after Phase 1. Implementers report gaps; they do not patch shared files.

## 6. Traps

- Test runner: `bun run test` (Vitest under Node). Never `bunx vitest`.
- `scale.test.ts` is a calibrated performance test and can flake; a failure
  there is reported, not "fixed" by loosening thresholds.
- The compiler's boundary plugin rejects any import leaving the extension folder,
  including `../../renderer/...`. Svelte and css inside the folder are fine.
- Legacy modules install into `PM` in a fixed order in `legacy/bootstrap.ts`;
  extensions boot after. Services may be absent during early boot.
- `PM.allProps` is overwritten by the timeline at activation. Phase 2 replaces
  that with a kernel-side registration; legacy must keep working when it is
  absent (falls back to `anim.ts` implementation).
- The e2e helper builds once into `out/`; a stale build hides renderer changes.
  Run `bun run build` before e2e when renderer code changed.
- No `git add` or `git commit` from implementers. The orchestrator stages.

## 7. Gate battery (orchestrator runs, unsandboxed)

```
bun run test                      # 0 failed (scale.test flake noted separately)
bun run typecheck                 # 0 errors
bun run lint:boundaries           # 0 violations
bun run test -- src/main/extensions/compiler.test.ts   # every built-in compiles
bun run build && bun run test:e2e -- e2e/extensions.spec.ts   # 7/7
rg -c "host\.pm|host\.state" src/extensions   # phase 3+: 0
rg "PM\.(TL|Viewer|Inspector|syncShaderUniforms|setTool|toolShape)\b" src/renderer/src/legacy  # phase 2+: 0
```

Plus one non-runner gate per phase: a real `bun run build` and a hidden-renderer
boot through the e2e helper that opens timeline, viewer, and inspector.

## 8. Results

Phase 4 removed the remaining legacy-registry references from the built-ins:

| Extension | Before | After |
|---|---:|---:|
| timeline | 467 | 0 |
| viewer | 286 | 0 |
| inspector | 221 | 0 |
