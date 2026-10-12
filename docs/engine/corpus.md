# Parity corpus

The parity corpus is what the legacy TypeScript engine produces today, captured so the
native engine can be compared against it (plan section 25, appendix B, decision 13).
The tooling lives in `engine/corpus/`; captured data never enters the repository.

- **Generator** (`src/generate.ts`, `src/projects/`): synthetic project documents, one or
  more per appendix B feature group, plus the media they need that the e2e fixtures do not
  provide.
- **Capture** (`src/capture.ts`): opens every project in the built legacy app and writes
  frames, float frames, evaluation dumps, and Blender snapshots at chosen times; also the
  12 pixel-hash scenarios of `e2e/rendering-performance.spec.ts` and Jude's real project.
- **Verify** (`src/verify.ts`): checks a capture against its manifest.

Captures are written to `~/Library/Application Support/Powermove Engine Corpus/<capture-id>/`.
The capture refuses to write inside the repository.

## Commands

Run from the repository root after `bun install` (bun 1.3.14). Node 24 must be on `PATH`
(the Playwright Electron driver runs under Node; see "Electron bridge" below).

```sh
bun run --cwd engine/corpus check          # validate every document; writes nothing
bun run --cwd engine/corpus test           # unit tests
bun run --cwd engine/corpus typecheck
bun engine/corpus/src/generate.ts --out /tmp/corpus   # documents and generated media only

bun run --cwd apps/desktop build           # the capture drives apps/desktop/out
bun engine/corpus/src/capture.ts           # full capture (about 4 minutes on an M4 Max)
bun engine/corpus/src/verify.ts "<capture dir>"
```

Capture options:

| Option | Meaning |
|---|---|
| `--id <id>` | Capture directory name (default `v0-<UTC stamp>-<commit>`); an existing directory is never reused |
| `--root <dir>` | Parent directory (default the Application Support path above) |
| `--label <text>` | Recorded in the manifest (default `v0`) |
| `--only <prefix,...>` | Only synthetic projects whose id starts with a prefix |
| `--skip-synthetic`, `--skip-scenarios`, `--skip-real` | Leave out a part |
| `--real <file.pmv>` | Real project (default `~/Documents/Powermove ad.pmv`) |
| `--real-step <s>` | Sample spacing for the real project (default 2 s, plus the last frame) |
| `--no-eval` | Skip evaluation dumps |
| `--restart-every <n>` | Relaunch the app every n projects (default 30) |

## How the capture drives the app

The app is launched the way the e2e helper does (`apps/desktop/e2e/helpers/app.ts`):
Playwright's Electron driver, `POWERMOVE_BACKGROUND_TEST=1` (hidden window that never
takes focus), an isolated temporary profile (`POWERMOVE_USER_DATA`), DevTools off, and
native dialogs substituted. Only one app instance runs at a time.

**Electron bridge.** Bun cannot host `_electron.launch` (it attaches to Electron's
inspector and never resolves). `src/electron-bridge.mts` runs the driver under Node and
talks to the Bun tool over IPC. Page functions travel as source text and run through
CDP, which the renderer's CSP does not restrict. Closing skips the editor's close
decision (the profile is disposable) and kills Electron if it has not exited after 20 s.

**Opening a project.** The generator writes documents with media named `media:<file>`.
The capture opens the document with the app's `pm-open-project` event, imports each
referenced file through `PM.assets.add` (fixture media from `apps/desktop/e2e/fixtures`,
generated media from the capture's `generated/media`), rewrites the references to the
asset ids the app assigned, reopens the rewritten document, and waits for media to load
and for three consecutive identical renders (`settle`). `loaded.json` is the project as
the app holds it after loading. Differences between the written document and the
loaded one are recorded per project (`hydrateIssues`; keyframe id reassignments, which
the loader does to effect parameters on every open, are counted apart as
`keyIdRewrites`).

**Rendering a sample** (`src/page.ts`, `capture`), with `PM.Export.busy` held as the
exporter holds it and `PM.quality = 1`:

1. `PM.prepareFrame(t)` (video frames, Blender frames). A throw is recorded as
   `prepareError` and the sample is still rendered, as the compositor draws it.
2. Pending GL errors are drained (recorded as warnings) so the errors recorded for the
   sample belong to its own renders.
3. Opaque: `PM.GL.renderToPixels(t, w, h, {mblur: true, mbSamples: 16, shutter, opaque:
   true, topDownOpaque: true})`, the export path's opaque capture.
4. Transparent projects (`alpha`): `renderToPixels(t, w, h, {transparent: true, mblur:
   true, mbSamples: 20, shutter})`. The float readback inside it (`gl.readPixels` with
   `FLOAT`) is captured, and the 8-bit result is converted to straight alpha exactly as
   the exporter's `alphaFrame` does.
5. Evaluation (below), with `PM.time = t`.

**Legacy helpers.** Some functions the compositor uses are not on `PM`. The capture
bundles them from the same commit's source and loads them beside the app
(`src/inject.ts`): `text-animation.ts` (`textAnimationState`, `animatedGlyphs`),
`retiming.ts` (`sourceTime`), and `frame-preparation.ts` (`blenderFrameRequests`; its
renderer and video imports are stubbed). They take `PM` as an argument and share no
module state with the app.

## Layout

```text
<capture-id>/
  manifest.json
  capture.log
  generated/                       the generator's output for this capture
    index.json                     projects (covers, samples, size, media, document hash)
    projects/<id>/document.json
    media/                         corpus-model.glb, corpus-materials.glb, corpus-gem.obj, corpus-art.svg
  projects/<group>/<name>/
    document.json                  as generated (media: references)
    loaded.json                    as the app holds it (asset ids, defaults filled in)
    project.json                   the project record (also in the manifest)
    frames/t<time>.png             8-bit RGBA, opaque
    frames/t<time>.alpha.png       8-bit straight alpha (transparent projects)
    frames/t<time>.rgba32f         float RGBA (transparent projects)
    frames/t<time>.rgba32f.json    its header
    eval/t<time>.json              evaluation dump
    blender/t<time>.json           Blender snapshots (projects with 3D objects)
    blender/assets/<sha256><ext>   asset bytes the snapshots reference
  scenarios/rendering-performance/
    <label>.rgba8, <label>.png, hashes.json, scenarios.json
  real/<slug>/                     the real project, same per-sample layout
```

`<time>` is the composition time in seconds with six decimals (`t1.250000`).

## Formats

**PNG frames** are 8-bit RGBA, top-down rows, with `tEXt` chunks naming the source and
the alpha convention. Opaque frames are the export path's opaque capture (alpha 255,
background composited over black where the background is `none`).

**Float frames** (`.rgba32f`) are raw little-endian IEEE 754 binary32, 4 channels per
pixel, top-down rows, no padding. The JSON header beside each:

| Field | Value |
|---|---|
| `format`, `version` | `powermove-corpus-rgba32f`, 1 |
| `width`, `height`, `channels` | Pixel size; 4 |
| `sampleType`, `endianness`, `rowOrder` | `f32`, `little`, `top-down` |
| `alpha` | `premultiplied` |
| `sourcePrecision` | `binary16`: the legacy accumulator is an RGBA16F texture, so every value is exact in binary16 |
| `encoding` | Display encoding E, Rec.709 primaries, premultiplied, unclamped (values above 1 occur, for example with glow) |
| `source` | The `renderToPixels` call |
| `time` | Composition time |
| `dataFile`, `dataBytes`, `dataSha256` | The data file and its size and hash |

`src/output.ts` `readFloat` reads one back and checks the hash.

**Evaluation dumps** (`format: powermove-corpus-eval`, version 1) hold, per composition
and recursively for every active precomp at its source time:

- per layer: every channel from `PM.allProps` evaluated through `PM.ev`/`PM.evP`
  (transform, content, effect, transition, mask, animator, range style, path, and
  shader-uniform channels), expression errors, active state, evaluated layer fields
  (blend, motion blur, visibility, matte mode, matte source, parent, group, 3D, solo),
  resolved content, local and world matrices, world opacity, the 3D plane matrix,
  `GL.bounds`, source time for video, precomp, and audio layers;
- text: `PM.textLayout`, text raster geometry, and for animated or range-styled text the
  animator weights, range styles, and every posed glyph (matrix relative to the glyph's
  baseline origin, opacity, blur, color, tracking, resolved style);
- captions: caption bounds; shapes with paths: each path's values and group matrix;
- `scene3d`: the scene description (`scene3d.describe()`) when the project has 3D layers;
- `shaderMeta`: the parsed shader metadata per shader and extension layer.

Non-finite numbers are written as strings (`"NaN"`, `"Infinity"`); `-0` is written as 0.

**Blender snapshots** (`format: powermove-corpus-blender`, version 1). The capture never
runs Blender. The main process's `blender:run` IPC handler is replaced by a recorder that
keeps each job and answers with an empty render. After a project's frames, Blender
output (`render3d`) is turned on in memory for every composition holding 3D objects,
with the project's `blender` settings if it names any, and for each sample:

- `final`: `PM.prepareFrame(t)` with its default options (composition size; 10 shutter
  samples when a layer has motion blur, the RAM-preview count rather than export's 16 or
  20, see appendix A item 4). One snapshot per entry of `blenderFrameRequests`, which
  labels each with its composition, time, and size.
- `preview`: `scene3d.renderFrame()` (the paused rendered preview) at t, labelled by the
  same walk `renderBlenderPreview` does; `view` records the viewport mode.

Each snapshot is `renderSnapshot`'s output as Blender would receive it, with typed arrays
written as JSON arrays and asset bytes stored content-addressed under `blender/assets/`.
`mismatch` is set (and requests dropped) if jobs and requests do not pair one to one.

**Scenarios.** `scenarios/rendering-performance/` reproduces the spec's twelve captures
statement for statement up to its timing loop. `<label>.rgba8` is the exact
`renderToPixels` result (RGBA8, bottom-up rows, premultiplied), whose SHA-256 is the
spec's hash; `hashes.json` has the spec's evidence shape; the PNG is a top-down,
straight-alpha view of it.

**Manifest** (`format: powermove-engine-corpus`, version 1): capture id and label, start
and finish times, engine commit, branch, dirty flag, and a content hash of
`apps/desktop/out`, app and Electron versions, GPU, machine, the render settings above,
whether the shader presets the generator extracted from `legacy/gl/shaders.ts` match
`PM.SHADER_PRESETS`, the generated corpus, totals, every project record with every
sample's files (path, size, SHA-256), and the scenarios.

## Synthetic projects

`generated/index.json` is the authoritative list; `bun run --cwd engine/corpus check`
prints it. 131 projects as of this writing:

| Appendix B group | Projects |
|---|---|
| Layers | `layers/solid`, `layers/svg-scales`, `layers/null-parenting`, `groups/solo-and-hidden` |
| Shader and extension layers | the five shader presets, `layers/shader-binding-quirks`, `layers/studio-cube`, `layers/obj-model`, `layers/extension-missing` |
| Media | one project per e2e fixture video and image (`media/*`) |
| Audio | `media/audio` (all five audio fixtures, embedded audio, keyed gain) |
| Shapes and paths | `shapes/primitives`, `shapes/editable-paths`, `shapes/svg-import-paths` |
| Text | `text/properties`, `text/variable-axes`, `text/range-styles`, one project per animator preset |
| Captions | four styled variants and one per caption preset |
| Animation | `animation/interpolation`, `animation/expressions` |
| Transforms | `animation/transforms`, `space/layers-25d`, `space/groups-25d` |
| Composition | metadata, four frame rates, five backgrounds, nested and cyclic precomps, multiple compositions |
| Compositing | nine blend modes (opaque and over transparency), adjustment layers, motion blur at two shutters, groups |
| Masks and mattes | parametric, more than eight, path masks, four matte modes, nested and keyed mattes, a matte cycle |
| Effects and transitions | each of the 16 effects, each of the 4 transitions, an extension effect (the Typewriter fixture extension), missing definitions |
| Retiming | retimed precomps, retimed video |
| 3D | primitives, lathe and extrude, every light type, orthographic and animated cameras, motion blur, transparency and double-sided, depth-set barriers, studio defaults, glTF (skinning, two clips), glTF materials (every extension r180 loads without a decoder, alpha modes, maps, morphs, points and lines, instancing, embedded lights), mesh slots and maps, missing model, legacy scene, nested 3D precomps, solo and hidden |

Documents are deterministic (fixed ids and timestamps; two runs produce identical bytes)
and valid: each is a fixed point of `sanitizeProject` (`core/validate/project.ts`), every
3D layer's content is a fixed point of the scene schema, effect and transition parameters
match the registered definitions, and keyframe ids are unique. Extension and deliberately
missing effects are checked against their own declarations, since `sanitizeProject` keeps
only built-in effects while the app's loader keeps the others. Definitions (effects,
transitions, Studio Cube, OBJ model, animator presets, caption presets, blend and matte
modes, layer types, render settings) are imported from the desktop source, not copied.

Projects that install a fixture extension run last, in their own app sessions, so
extension hooks never touch other projects' frames.

## Real project

`~/Documents/Powermove ad.pmv` (decision 13) is copied into the temporary profile and
opened through the app's Open command, so the original is never written; its hash is
checked before and after. It is sampled every 2 s and at its last frame. Its captured
data stays under the capture directory like everything else.

## Not covered yet

- Blender-rendered surfaces, recipes (Rocket, Staircase, 3D Text, mesh recipes with
  modifiers), Blender shader graphs, and EEVEE or Cycles output: these need Blender and
  a pinned build (plan section 25).
- Final Blender snapshots use `PM.prepareFrame`'s defaults; export-path snapshots (export
  size, 16 or 20 shutter samples) need the exporter's own `prepareFrame` call.
- Variable-frame-rate MP4, coarse-timebase sources, and embedded fonts: no e2e fixture
  provides them.
- Audio is not rendered; evaluation dumps record the audio channels.
- Chromium color measurement for tagged and HDR sources (section 25, Phase 0) is a
  separate task.
