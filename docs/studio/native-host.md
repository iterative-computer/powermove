# Native Studio in Electron

Studio runs in `pm-studio-host`, an isolated Rust process. Electron owns the
window, controls, scene evaluation and compositing. No Blender is needed.
The integration implements Lane A's `StudioBackend` contract; the factory is
registered by `core/studio-native/install.ts` before editor boot. The viewer's
backend dispatch lives on `studio-3d`; these changes deliberately do not merge
that branch's document model or panels into `studio-3d-native`.

## Rendering and transport

- **Live:** half-resolution, one sample, three-bounce path tracing with Metal
  hardware ray queries. Direct lighting uses bounded weighted reservoirs:
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

Apple M5, 16 GB, macOS 27.3, Electron 44.0.0, 1920 × 1080 output. Measured
2026-11-09 while another development lane was active. Live traces 960 × 540.
These are measured wall times, not GPU timestamp-query estimates.

Initial pooled shared-texture run, thirty camera/time edits per scene:

| Scene | Input → pixels median / p95 | Host median | Refine 16 spp + OIDN |
|---|---:|---:|---:|
| hero | 10.3 / 10.6 ms | 8.1 ms | 1.19 s |
| chrome | 14.8 / 15.1 ms | 11.9 ms | 1.40 s |

The timer starts before a camera edit, includes the scene patch, render,
interprocess transfer and canvas draw, and ends after a pixel read forces
completion. It excludes display scanout/vsync. Host times include submission
and waiting for GPU completion. The initial frame took 1.77 s for hero and
0.19 s for chrome in that run; shader/driver and shared-image caches affect
startup substantially. A previous first import took 3.4 s.

Before the shared-texture path, a binary-frame run with the native temporal
filter measured 10.9 / 12.8 ms input-to-pixels median/p95 for hero and
14.9 / 17.9 ms for chrome. This is not a controlled MetalFX transport comparison;
the corpus script can run identical settings with `STUDIO_NO_SHARED_TEXTURE=1`.
The standalone host reached roughly 1.45–1.48 GiB resident memory after Live
and Refine. This is process RSS, not total app peak memory.

At 960 × 540 and 64 samples, with denoising off in both engines, mean absolute
native/WebGPU RGB differences were 0.0043 code values for hero and 0.814 for
chrome (out of 255). The latter includes different photon batch sizes and
hardware intersection rounding. This verifies the shared look, not equivalence
to Blender Cycles. Full corpus measurements and limits are recorded below as
validation progresses.

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
only the top-level hierarchy and instance data.

`prepared` is the fast path for Rust's `Prepared` layout already produced by
the app's shared scene extractor. It supplies positions, triangle attributes,
BLAS/TLAS nodes, material data and lights/instances as binary ranges.
`preparedPatch` sends only the TLAS prefix, materials, lights/instances and
view. A changed layout or layer list sends a new full upload. Texture arrays
are uploaded separately; subsequent edits replace only changed layers.

Other operations are `textures`, `screen`, `render`, `pick`, `bounds`,
`releaseFrame`, `snapshot` (corpus validation) and `shutdown`. Time is evaluated
by the desktop host, passed with edits/render requests, and seeds deterministic
Final sampling. Per-layer output retains other layers' occlusion/reflections.
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
CARGO_TARGET_DIR=~/Developer/pm-target nice -n 10 ~/.cargo/bin/cargo test --manifest-path engine/Cargo.toml -p pm-studio -p pm-studio-host --features oidn,metalfx
bun run --cwd apps/desktop test -- src/main/studio-native/manager.test.ts src/renderer/src/core/studio-native/backend.test.ts src/renderer/src/core/studio-native/recovery.test.ts
python3 apps/desktop/scripts/studio-native-smoke.py
node apps/desktop/scripts/studio-native-electron.mjs
```

The app build compiles the host and N-API addon, copies OIDN and its dependency
closure, rewrites library references/rpaths, includes licenses, and signs the
local binaries ad hoc. `electron-builder.yml` copies `resources/studio` into
`Contents/Resources/studio`. Rust, Xcode tools, Node headers and OIDN 2.x are
build prerequisites (`OIDN_DIR` and `NODE_INCLUDE_DIR` can select installations).
Dev resolves the built resource directory or the shared Cargo target directory;
`POWERMOVE_STUDIO_HOST` can select an explicit development binary.

`PM_STUDIO_SCENES`, `PM_STUDIO_FRAMES`, and `PM_STUDIO_SHOTS` configure the
Electron corpus script. `STUDIO_NO_RT`, `STUDIO_NO_RIS`, `STUDIO_NO_METALFX` and
`STUDIO_NO_SHARED_TEXTURE` exercise fallbacks. Screenshots and raw measurement
logs belong only in `/tmp/pm-shots/studio3d/laneB/`, never Git.

## Limits and remaining gates

- The full corpus 60 fps gate and general environment/Cycles quality gate are
  not yet established by the initial two-scene run.
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
