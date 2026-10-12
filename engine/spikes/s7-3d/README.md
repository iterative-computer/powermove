# Spike S7: 3D

Plan sections 7, 8, 9, 17, and 25; report in `docs/engine/spikes/S7.md`.

Its own Cargo workspace (pinned Rust through `engine/env.sh`) plus bun scripts. All
outputs go to `out/` (ignored).

```sh
source ../../env.sh && export RUSTC_WRAPPER=kache CARGO_BUILD_JOBS=4
cargo build --release
```

## 1. three.js r180 references

```sh
bun ref/corpus.ts --big    # textures, glTF models, scenes, pick plans → out/corpus
bun ref/render.ts          # per-layer and full renders, WebGL2 traces, masks → out/ref
bun ref/picks.ts           # production picks and three.js raycasts → out/picks
```

`ref/page.ts` runs the production `SceneRuntime` and `loadGltf` from
`apps/desktop/src/renderer/src/core/scene3d/` in headless Chromium (ANGLE on Metal),
renders each 3D layer as the compositor does (shared depth prepass, background on the
bottom layer), records every WebGL2 call (`ref/recorder.ts`), and writes the
classification masks: object ids, per-output no-MSAA renders (MSAA edges), and shadow
penumbrae. `--fs-tail '<glsl>' --fs-match <text>` appends a probe to matching fragment
shaders in the recorded trace.

## 2. wgpu renderer

```sh
./target/release/s7-replay out/ref out/replay [--hand] [scene ...]
./target/release/s7-compare out/ref out/replay          # section 2 3D bar
./patches/build-math-mode.sh                            # tools with a Metal math-mode switch
S7_MATH=safe ./target/math/release/s7-replay out/ref out/safe
./target/release/s7-compile-bench out/ref               # GLSL pipeline cost per program
```

`crates/s7-glsl` is the S3-style pipeline for three.js programs (glslang → SPIR-V
rewrites → naga). `crates/s7-replay` replays the recorded command streams on wgpu
(Metal); `--hand` swaps the standard material for the hand-written WGSL in
`src/hand.rs`. Debug tools: `s7-zoom`, `s7-pixels`, `s7-diffs`, and the `s7-glsl`
examples (`pp`, `io`, `wgsl`, `dump`).

## 3. pm-geom3d prototype

```sh
./target/release/s7-picks .                              # native parity and timing
cargo build --release --lib -p s7-geom3d --target wasm32-unknown-unknown
node bench/picks.mjs target/wasm32-unknown-unknown/release/s7_geom3d.wasm
S7_FULL_SKIN=1 node bench/picks.mjs ...                  # re-skin + refit path instead of lazy
cargo run --release -p s7-geom3d --example images -- out/corpus/assets/webp-avif.glb out/ref/gltf-webp-avif 392,263148
cargo run --release -p s7-geom3d --example snapshot_geometry -- out/corpus/assets/skinned.glb out/blender/gltf-skinned/pinned-eevee/snapshot.json rig out/picks/gltf-skinned.json 0.7
```

## 4. Blender

Blender 4.5.14 LTS (official arm64 build, SHA-256 checked) in `/Applications/Blender.app`.

```sh
bun blender/run.ts --variant pinned --engine eevee standard-sun orthographic gltf-webp-avif gltf-skinned
bun blender/run.ts --variant pinned --engine cycles --samples 16 [--device auto] ...
bun blender/run.ts --variant today --engine eevee ...
bun blender/run.ts --variant pinned --no-stamp --engine eevee ...
Blender --background --factory-startup --python-exit-code 1 --python blender/agx_check.py -- out/blender/agx eevee
./target/release/s7-blender-check agx out/blender/agx eevee
./target/release/s7-blender-check batch out/blender/standard-sun/pinned-eevee
./target/release/s7-blender-check orient out/blender/gltf-webp-avif/pinned-eevee/batched/cards.png out/ref/gltf-webp-avif/layer-cards.png
./target/release/s7-blender-check ortho out/blender/orthographic/pinned-eevee/batched out/ref/orthographic box ball ring cone
```

`blender/run.ts` runs the shipped worker (`apps/desktop/src/main/blender-worker.ts`) with
`BlenderWorker`'s flags and request protocol; snapshots come from `blenderSnapshot` in
`ref/page.ts`, a port of `renderSnapshot` over the production runtime.
