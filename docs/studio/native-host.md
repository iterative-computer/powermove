# Native Studio in Electron

Studio runs in `pm-studio-host`, an isolated Rust process. Electron owns the
window, controls, scene evaluation and compositing. No Blender is needed.
The integration implements Lane A's `StudioBackend` contract; the factory is
registered by `core/studio-native/install.ts` before editor boot. The combined
`studio-3d` branch retains main's document model/panels and Lane A's current
backend dispatch, extractor, compositor and final-frame preparation.

## Rendering and transport

- **Live:** bounded tracing resolution, one sample, three-bounce path tracing
  with Metal hardware ray queries. Direct lighting uses bounded weighted reservoirs:
  two fresh candidates (four above eight lights), one temporal reservoir and
  one spatial neighbor, with one final visibility test. Short-path indirect
  light is reused by the temporal denoiser. This is not a full ReSTIR GI solver.
- **macOS 26+:** `MTLFXTemporalDenoisedScaler` receives linear color, depth,
  camera motion, diffuse/specular albedo, view normals, roughness, jitter and
  camera matrices, then denoises and upscales to the output size. Object,
  material and lighting changes reset history; camera movement retains it.
- **Refine/Final:** the same tracer accumulates at output resolution. At sixteen
  samples and above, Open Image Denoise filters color and shadow-catcher
  shading with albedo/normal guides. The bundled OIDN build uses its CPU device.
- **Portable fallbacks:** WGSL BVH traversal without ray queries, temporal
  reprojection plus edge-aware à-trous filtering without MetalFX, binary RGBA
  frames without the surface addon, and the in-page WebGPU backend after a
  process or transport failure.

MetalFX and the Metal handle borrowing live in `gpu/metalfx.rs` and
`native/metalfx.mm`. Reservoir sampling has its own WGSL/module. No Metal object
or raw pointer is exposed to the editor page.

Electron 44 provides experimental
[`sharedTexture.importSharedTexture` and `sendSharedTexture`](https://github.com/electron/electron/blob/v44.0.0/docs/api/shared-texture.md).
Its [macOS handle is a process-local IOSurfaceRef](https://github.com/electron/electron/blob/v44.0.0/docs/api/structures/shared-texture-handle.md),
not an integer surface ID or a pointer that can be copied from Rust.
Electron's [implementation notes](https://github.com/electron/electron/blob/v44.0.0/shell/common/api/shared_texture/README.md)
require transferring a Mach right before import. Offscreen shared textures
export Chromium's own frames; they do not import Studio's frames.

The chosen path is:

1. Studio packs the finished output on the GPU and blits it into one of three
   IOSurfaces.
2. A private Mach message transfers an IOSurface right to a small stable N-API
   addon in main. The addon resolves it to a local reference.
3. Electron imports and transfers the texture to a sandboxed preload receiver.
4. The renderer draws the managed VideoFrame into its canvas. There is no CPU
   frame readback or pixel upload on this path. There are GPU copies.
5. `allReferencesReleased` releases the local reference and returns the surface
   slot to the host. A busy pool or unavailable API uses binary frames.

The receiver follows Electron's
[managed preload example](https://github.com/electron/electron/blob/v44.0.0/spec/fixtures/api/shared-texture/managed/preload.js).
The MetalFX output is initialized once through wgpu before Metal writes it;
otherwise wgpu's first sampled read would clear an externally written texture.

## Measurements

Apple M5, 16 GB, macOS 27.3, Electron 44.0.0, 1920 × 1080 output.
Measured 2026-11-09. These are wall times, not GPU timestamp estimates.
The shared-texture path uses Metal ray queries, direct-light reservoirs and
MetalFX. Refine uses sixteen samples and OIDN at full output resolution.

Full product corpus, 45 camera/time edits per scene, 960 × 540 Live tracing:

| Scene | Host median | Input → pixels median / p95 | Refine + OIDN |
|---|---:|---:|---:|
| hero | 8.26 ms | 10.4 / 10.8 ms | 1.30 s |
| edge | 7.25 ms | 9.5 / 10.0 ms | 1.09 s |
| macro | 12.45 ms | 14.6 / 15.0 ms | 1.41 s |
| back | 10.94 ms | 13.1 / 13.5 ms | 1.21 s |
| chrome | 11.52 ms | 14.3 / 14.9 ms | 1.37 s |
| materials | 10.93 ms | 15.1 / 15.4 ms | 1.28 s |
| caustics | 9.39 ms | 12.5 / 13.0 ms | 1.14 s |
| wet | 12.73 ms | 14.8 / 15.3 ms | 1.39 s |

The timer starts before a camera/time edit, includes the small scene patch,
render, interprocess transfer and canvas draw, and ends after a pixel read
forces completion. This includes diagnostic synchronization/readback and
excludes display scanout/vsync. It is a hidden Electron integration harness,
not the full editor compositor. Host times include GPU submission and waits.
All eight scenes met a 16.7 ms p95 in this run. That is not a sustained full-app
60 fps guarantee.

A later 60-edit repeat during a Cargo check and concurrent Electron rendering
measured **10.8–21.9 ms median**, up to **32.4 ms p95**, and **1.29–2.07 s**
Refine. The target is not maintained under all concurrent workloads. No other
lane or user application was stopped for these measurements.

Peak native-process RSS, read with `getrusage` after Live, 1080p Refine and
960 × 540 Final, was **1452–1505 MiB (1.42–1.47 GiB)** across the product
corpus. This excludes Electron and is not a total unified-memory footprint.
Steady first-hit queries took roughly **0.5–0.9 ms** round trip. A first query
also compiles its pipeline. Cold first frames varied from approximately
**0.17 s to 3.4 s** with driver/shader/shared-image cache state.

Sequential runs with identical scene, size and sampling settings compared
transport costs (background machine load was not locked):

| Transport | Hero input → pixels median / p95 | Chrome input → pixels median / p95 |
|---|---:|---:|
| IOSurface + sharedTexture | 10.4 / 10.8 ms | 14.3 / 14.9 ms |
| Binary RGBA + canvas upload | 16.1 / 22.7 ms | 19.8 / 21.2 ms |

Both transports used MetalFX. Shared textures saved about 5–6 ms at the median
in this pair. The production shared-texture path has no CPU frame readback;
the benchmark's final pixel read is only a completion check.

At 960 × 540, sixty-four samples and denoising disabled in both engines,
all eight native/WebGPU comparisons passed. Mean absolute RGB differences
were at most **0.814 code values out of 255**. Different photon batch sizes and
hardware intersection rounding can contribute to the larger differences in caustic/glass scenes.
This establishes shared-renderer parity, not Blender Cycles equivalence.

Additional low-detail environment fixtures exercise scene size and many lights:

| Fixture | Instances / lights | Live input | Input → pixels median / p95 | Refine |
|---|---:|---:|---:|---:|
| Room and tabletop | 21 / 64 | 768 × 432 | 16.5 / 34.5 ms | 1.95 s |
| 144 simple trees and ground | 289 / 1 | 768 × 432 | 14.4 / 17.0 ms | 1.43 s |
| Room, orthographic camera | 21 / 64 | 768 × 432 | 17.1 / 23.2 ms | 2.11 s |

These render and pick correctly and match WebGPU. They do not establish
photorealistic environment quality or universal 60 fps. The room's 64 finite
point lights exercise bounded direct-light resampling. The forest fixture has
5,772 triangles; it is not a production foliage/large-world stress test.

Live normally traces at 50% of output dimensions; scenes above 128 instances
or 32 lights start at 40%. `STUDIO_LIVE_SCALE` fixes a diagnostic scale between
one third and one half. An optional `STUDIO_ADAPTIVE_LIVE=1` budget reduces
resolution after sustained slow frames and recovers slowly. Its 120-edit
experiment improved steady timings, but recreating a target/MetalFX scaler
caused **0.8–1.57 s pauses**. It is **disabled by default** until resizing can
retain the scaler and pipelines. Refine/Final always use the requested size.

The integration script also killed the live native process deliberately.
Scene replay into WebGPU produced verified pixels in **38–52 ms** in two runs.
The same script verifies a populated → empty scene transition, including the
change from hardware queries to portable guide traversal.

## Scene and process protocol

stdin/stdout carry framed messages. stderr is reserved for diagnostics:

```
4 bytes  "PMS1"
u32 LE   JSON byte length
u32 LE   binary byte length
JSON     { id, op, ... }
binary   raw little-endian arrays or RGBA8 pixels
```

Responses contain `{id, ok, result}` or `{id, ok:false, error}` plus optional
binary data. JSON is limited to 4 MiB and each binary packet to 256 MiB.
`hello` reports the protocol and exact GPU buffer strides, so stale packaged
hosts cannot silently interpret a newer material layout.

`scene` accepts the serialized `pm_studio::scene::Scene`. Its meshes can be
empty JSON placeholders plus binary `meshes` attachments identifying an object
and byte ranges for positions, normals, UVs and indices. `patch` replaces
camera/material/light data or supplies absolute column-major object transforms.
Base meshes, per-mesh BVHs and native BLAS remain resident; dragging updates
only the top-level hierarchy and instance data. Camera-only edits retain both
levels of acceleration structures.

`prepared` is the fast path for Rust's `Prepared` layout already produced by
the app's shared scene extractor. It supplies positions, triangle attributes,
BLAS/TLAS nodes, material data and lights/instances as binary ranges.
`preparedPatch` sends only the TLAS prefix, materials, lights/instances and
view and instance part identities. A changed layout or layer list sends a new
full upload. Invalid cyclic/deep trees and stale geometry patches are rejected
before GPU traversal. Texture arrays are uploaded separately; subsequent edits
replace only changed layers.

Other operations are `textures`, `screen`, `render`, `pick`, `bounds`,
`releaseFrame`, `snapshot` (corpus validation) and `shutdown`. GPU picks return
the actual first visible layer, part, world point and shading normal (including
normal maps); before a target exists a CPU triangle query is available. Bounds
are calculated from cached local boxes and current transforms, optionally
filtered by layer. Time is evaluated by the desktop host, passed with edits
and render requests, and seeds deterministic Final sampling. Per-layer output retains other layers' occlusion/reflections.
Multiple layer requests reuse the same finished trace.

The manager bounds pending requests, times out a stuck process, rejects work
on exit, and starts a new process/generation on the next connection. The
recovering backend replays its scene into WebGPU when native work fails.
Navigating, closing or crashing an editor releases its sessions and surfaces.
There are at most two native sessions per editor to bound memory.

## Build and verification

From the repository root:

```sh
bun install --frozen-lockfile
bun run --cwd apps/desktop build
CARGO_TARGET_DIR=~/Developer/pm-target CARGO_BUILD_JOBS=2 nice -n 10 ~/.cargo/bin/cargo test --manifest-path engine/Cargo.toml -p pm-studio -p pm-studio-host --features oidn,metalfx
bun run --cwd apps/desktop test -- src/main/studio-native/manager.test.ts src/renderer/src/core/studio-native/backend.test.ts src/renderer/src/core/studio-native/recovery.test.ts
python3 apps/desktop/scripts/studio-native-smoke.py
node apps/desktop/scripts/studio-native-electron.mjs
PM_STUDIO_SCENES=hero PM_STUDIO_RECOVERY=1 PM_STUDIO_EMPTY=1 node apps/desktop/scripts/studio-native-electron.mjs
```

The app build compiles the host and N-API addon, copies OIDN and its dependency
closure, rewrites library references/rpaths, includes licenses, and signs the
local binaries ad hoc. `electron-builder.yml` copies `resources/studio` into
`Contents/Resources/studio`. Rust, Xcode tools, Node headers and OIDN 2.x are
build prerequisites (`OIDN_DIR` and `NODE_INCLUDE_DIR` can select installations).
Dev resolves the built resource directory or the shared Cargo target directory;
`POWERMOVE_STUDIO_HOST` can select an explicit development binary.

`PM_STUDIO_SCENES`, `PM_STUDIO_FRAMES`, and `PM_STUDIO_SHOTS` configure the
Electron corpus script. Additional fixtures are `room`, `forest` and
`room-ortho`. `PM_STUDIO_RECOVERY=1` kills the owned test process to verify
WebGPU recovery; `PM_STUDIO_EMPTY=1` checks empty-scene transitions.
`STUDIO_NO_RT`, `STUDIO_NO_RIS`, `STUDIO_NO_METALFX` and
`STUDIO_NO_SHARED_TEXTURE` exercise fallbacks. Screenshots and raw measurement
logs belong only in `/tmp/pm-shots/studio3d/laneB/`, never Git.

Validation completed: Rust library/host/GPU tests; TypeScript manager, binary
adapter and fallback tests (22 Rust/GPU and 17 targeted TypeScript tests in
total); full desktop typecheck (zero errors, two existing
Svelte warnings); desktop build; local `electron-builder --dir --mac --arm64
--publish never`; relocated bundled-host pixel smoke; real Electron shared
textures (also with the packaged host/addon), all eight corpus comparisons,
environment/orthographic fixtures,
crash recovery and empty-scene transitions. A no-default-features Cargo check
and a hardware-RT/MetalFX-disabled rendering smoke passed. Release signing,
notarization and publication were not run.

## Limits and remaining gates

- The product corpus meets the measured frame-time target in the lower-load
  run above. Consistent full-editor 60 fps, concurrent-load performance and
  environment/Cycles quality remain open gates. No Cycles comparison was made.
- Live's first frame is noisy; temporal history improves it over subsequent
  frames. Glass, mirror edges and floor shadows still show artifacts before
  Refine. MetalFX is not used to judge Final/export parity.
- Object movement resets denoiser history; camera motion has reprojection.
  Per-object motion-vector reprojection is not implemented.
- A single packet and a single storage binding bound very large scenes.
  There is no out-of-core mesh/texture paging or mesh LOD in this host.
- Output is SDR RGBA8, up to 4096 pixels on an axis and 8,847,360 output pixels.
- Metal BLAS builds are submitted and completed separately. Batched builds
  dropped the intermediate glass-sphere BLAS on this M5/SDK; a hardware/BVH
  intersection regression test covers the fix. Transform drags reuse BLAS.

## Combined editor integration

The editor requests native Live at output resolution; the host controls tracing
resolution and MetalFX upscaling. Overlapping superseded depth sets release
sessions immediately, while opaque layers share a job and finished trace.
Prepared stills/exports retain async images until composition readback. Capture
readiness tracks only that capture's requested images and temporarily defers
viewer paints. Nested 3D compositions inherit Final quality. Unprepared
synchronous readbacks use Studio's WebGPU implementation.

The editor integration test verifies native pixels, actual Electron shared
texture transfers and repeated layer additions. The Final-quality regression
checks a 3D shot nested in a 2D composition against its configured sample goal.
Development launches resolve the built runtime beside the desktop project so
its surface addon is found before the shared Cargo binary fallback.
