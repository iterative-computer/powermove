# Studio render lane

## 2026-11-04 — native renderer port (in progress)

Behaviour reference: main `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`;
new Studio source: `b81b94c663d2d4b380ba14a322fa2f6b14b769df`.
The base already contains the earlier Rust Studio import and an environment-opt-in
`pm-render::scene3d::StudioPass`. This lane preserves those consumers and brings
across the newer guide/variance shaders and native scene extraction.

`pm_studio::extract::SceneSource` is the core/render boundary. It returns an
immutable evaluated `SceneInput` for composition ID, revision, exact engine ticks
and dimensions. Mesh instances carry stable layer IDs, immutable shared geometry,
material slots and affine object-to-world transforms. `Extractor` caches BLASes
per immutable geometry, rebuilds world-space TLAS bounds after motion and emits
the shader's inverse, inverse-transpose and forward matrices. The source's
material/map/normal/tangent/light buffer layouts are retained. Missing resources
and invalid geometry fail visibly. The core lane is not present on the remote
at this checkpoint; no speculative document/kernel adapter is claimed.

`pm_studio::service::StudioService` is a worker-owned facade with Fast (requires
an attached real rasterizer), Quality Live (half-size, 1 sample, 2 bounces),
Quality Refine (full-size, accumulated samples) and seeded Final (fixed samples,
8 bounces, optional guided or OIDN denoise). Per-layer textures use the source's
premultiplied display sRGB SDR contract; the compositor must perform its normal
working-space conversion. No HDR preservation claim is made for this SDR path.
Final never silently substitutes Quality or Fast.

Metal is selected on macOS; Vulkan is selected elsewhere. Both execute the same
WGSL compute BVH. Existing single-instance flattened geometry retains its optional
hardware ray-query implementation. Extracted local-space instances explicitly
use the compute BVH: the older hardware builder cannot represent their transforms.

### Picking rules

- N1/N2: retain full-scene transport for reflections between objects; masks are
  applied only when presenting each layer, not while tracing.
- N3: retain model-space geometry and exact affine transforms, inverse-transpose
  normals, stable IDs and complete resource invalidation; refuse unresolved maps.
- N3: deterministic frame salts follow the source's wrapping `salt * 4099`;
  Final resets every request and guide rays use the same frame parameters.
- N4/N6 and prune rule: leave old renderer/extension consumers and project data
  intact; no legacy schema or document is rewritten by this renderer.
- N7/N8: plain Rust scene/provider and texture interfaces; no Node, browser, DOM
  or main-thread JavaScript dependency. Synchronous GPU waits belong on workers.
- N3/honesty: optional OIDN failures and missing Fast adapters are errors, never
  optimistic success flags or blank output.

### Open integration and verification

The private `pm-render` Studio pass has no typed provider/settings setter.
The owner was asked to authorize a minimal hook outside lane ownership; until
that is answered, replacing the compositor/export path is blocked. The legacy
`PM_STUDIO` path remains unchanged and is not claimed to consume `SceneSource`.

No UI changes or design choices are made here. Swatches use the source's
material-scene recipe. Any eventual Studio controls must use Opus's specs and
review; this lane does not claim UI acceptance.

Linux checks are Vulkan/llvmpipe, not Metal evidence. Still required on macOS:
Metal shader compilation/execution, hardware-query equivalence, full compositor
masks/effects/opacity/blends, application service/export-child integration,
source-sized exports, real product scenes, performance on base Apple silicon,
GPU scheduling/cancellation, packaging/signing and Opus review of visual output.

## 2026-11-04 — source-aligned models and portable qualification

Merged integration/design head `78294c4c9`. The controlling UI specification is
[3d-studio.md](design/surfaces/3d-studio.md). `status::LightKind` uses **Softbox,
Spotlight, Bulb, Sun**, round-tripping **area, spot, point, sun** unchanged.
`Progress` and `Failure` contain the exact approved capsule/banner/export copy;
Live and Fast emit no capsule. Delays, announcements and transitions remain host
responsibilities. The two messages claiming a Fast fallback are explicitly for
use **after** a Fast adapter actually succeeds. They are not emitted by an
unattached renderer. No §12 proposal, explode/place-on human control or new
interaction has been implemented. No DESIGN-TODO(opus) is introduced: this lane
adds no control, layout or new copy. Rendered content remains provisional pending
Opus's visual review.

Shared immutable meshes now share one GPU BLAS as well as one CPU build. The
instance record holds material and layer overrides; zero retains the existing
per-triangle contract for earlier callers. Geometry buffers remain resident when
only transforms, camera, lights, materials or screens change. CPU/GPU upload
counters are inspectable; tests require one geometry upload across resource edits.
The SAH builder caps depth at 20, retaining **every** triangle in a larger leaf
when necessary, so neither fixed shader stack can overflow. Non-finite world
bounds and oversized storage buffers are explicit failures.

The reflection witness places an emissive card **behind the camera** and requires
its reflection on another layer's metal sphere; the card itself must have no
primary coverage. Its initial assertion expected pure-red output at 4× emission
and failed: the observed red reflection was `[253,143,143]`, which is the specified
Neutral tone mapper's highlight desaturation. The fixture now emits 0.5, below
Neutral's 0.76 shoulder, retaining the original pure-red/channel/coverage
assertions. No renderer tolerance or missed reflection was hidden.

`engine/hosts/pm-studio-host` is an engine-workspace member. It is a synthetic,
headless integration witness, **not the application's video-export replacement**:

```sh
cd engine
cargo +1.96.1 run --locked -p pm-studio-host -- hero /ABSOLUTE/NEW_OUTPUT 320 180 11
```

It writes per-layer PNGs and a JSON manifest mapping stable synthetic IDs to
filenames, with ticks, samples, frame salt, adapter and actual hardware-RT use.
The destination must be new; existing output is refused. Readback accepts any
row width using aligned staging rows; PNGs unpremultiply the compositor textures.
The integration test decodes every PNG, requires actual covered product pixels,
and proves a second export preserves the original manifest.

### Optional OIDN licence and packaging

The `oidn` feature (service forwarding feature `studio-oidn`) is **off by default**.
It links Intel Open Image Denoise **2.x**, under **Apache-2.0**. The tested binary
is the upstream `oidn-2.5.1.x86_64.linux.tar.gz` release. Its unmodified licence is
vendored at [OIDN-2.5.1-LICENSE.txt](../../engine/crates/pm-studio/licenses/OIDN-2.5.1-LICENSE.txt).
No OIDN binary, downloaded package, training data or system tool is committed.
`OIDN_DIR` selects an installed package; the build script never downloads code
or invokes a package manager. Non-macOS feature builds require it explicitly;
macOS also accepts the existing Apple-silicon Homebrew prefix for development.

Final's native guide/variance filter is always available. Selecting OIDN in a
build without it returns an error before extraction/GPU work. A requested OIDN
failure propagates to the job. The old `finish_with` API now fails explicitly;
new hosts should use `try_finish_with`/`try_present_with` to receive the error.
Input dimensions, strides and guide lengths are checked before entering C.

A shipping OIDN-enabled Mac bundle must vendor `libOpenImageDenoise` and its
complete chosen-device/runtime dependency closure under `Contents/Frameworks`
for **each supported architecture**, preserve required device libraries, and
include OIDN's licence plus upstream `third-party-programs*.txt` notices for
exactly the shipped distribution (including oneTBB and any DPC++/SYCL runtime).
Use `@rpath` install names and `@executable_path/../Frameworks`; audit with
`otool -L`, rewrite Homebrew absolute load paths, then sign nested libraries before
the app and notarize. The witness/service binary build helpers add that bundle rpath as well as the
local installation rpath for development, forwarding `DEP_PM_STUDIO_OIDN_LIB`
metadata from the library. The final native-app binary must also supply its
bundle rpath: a dependency rlib cannot install an executable runtime search path. No shipping binary/rpath/signing check was
possible on Linux. Do not enable the feature in a release before the clean-Mac,
both-architecture, no-Homebrew launch and denoised-export checks pass.

### Remaining delivery blockers

- The core branch became available at the final fetch: read-only input
  `1ffa79bda4137ef7872d6688f681b70a592e65c4` supplies `pm-scene3d`, schema,
  transactional edits and evaluated JSON assembly, but its handoff explicitly
  leaves imported geometry adapters/renderer integration open. It has not been
  merged wholesale into this lane. The trait is ready for that scene adapter; project migration/imports, exact
  evaluated visibility/material groups, screen compositions and source snapshots
  still need that adapter. This lane does not change or discard project data.
- The application compositor's Studio pass is private in **unowned** `pm-render`.
  It needs a typed setter/provider hook, and its frame planner/executor must
  consume these layer textures using the existing masks/effects/opacity/blends
  path. Minimal out-of-lane permission was requested but not received. The
  `pm-engine-service::studio` facade is additive, not installed production wiring.
- Export-child setup must reconstruct the same scene source/resources, select
  Final using the camera's samples, partition samples across existing shutter
  times and composite those results before encoding. The source itself divides
  samples by shutter count; motion blur belongs to the outer compositor.
  Orthographic/oversized Fast fallback requires a real attached rasterizer.
  General errors must not silently fall back during Final export.
- Live temporal reprojection, automatic idle/refine scheduling, bounded
  preparation/progress delivery, cancellation/admission between GPU slices and
  multi-view/device-loss recovery remain application-integration work. The
  synchronous portable facade is not a UI-thread task or a certified latency gate.
- Instanced Metal hardware-query acceleration is not implemented; the Metal
  compute BVH is the honest backend for these scenes. SDR `RGBA8` presentation
  follows the source; float/HDR compositor/export preservation is still open.

### Qualification checkpoint — 2026-11-04

Merged base: `78294c4c93cdb2a3a18aef8004fc42f9e5e420a9`. Backend: Vulkan
llvmpipe, LLVM 19.1.7. Rust: **1.96.1**. OIDN: **2.5.1**. Default suite:
**28 passed / 0 failed / 0 ignored**. OIDN-enabled suite:
**31 passed / 0 failed / 0 ignored** (the same 28 plus three OIDN checks). Both include the headless host's two export/CLI tests.
Default and OIDN all-target Clippy pass with `-D warnings`; engine packages and
the touched engine-service package pass formatting checks. Both workspace
lockfiles contain only the needed additive resolution changes; no bulk update.

Run from the repository root after sourcing the environment. Set `OIDN_DIR` to
the installed upstream OIDN 2.x directory for the optional commands:

```sh
cd engine
cargo +1.96.1 test --locked -p pm-studio -p pm-studio-host
cargo +1.96.1 test --locked -p pm-studio -p pm-studio-host --features oidn
cargo +1.96.1 clippy --locked -p pm-studio -p pm-studio-host --all-targets --no-deps -- -D warnings
cargo +1.96.1 clippy --locked -p pm-studio -p pm-studio-host --all-targets --features oidn --no-deps -- -D warnings
cargo +1.96.1 fmt -p pm-studio -p pm-studio-host --check
cd ../apps/native
cargo +1.96.1 fmt -p pm-engine-service --check
```

An initial OIDN-enabled CLI test exposed a **real runtime library lookup failure**:
a library crate's link arguments do not propagate its rpath into dependent
executables. The fix declares Cargo `links` metadata and forwards the selected
OIDN directory into the host/service binary build helpers. Replays now pass
without adding OIDN to `LD_LIBRARY_PATH`. The app's final bundle closure remains
a Mac gate, as described above.

This required check is **blocked**, not passed:

```sh
cd apps/native
cargo +1.96.1 test --locked -p pm-engine-service --no-default-features
```

It stops before running tests because the existing `objc2` dependency explicitly
refuses non-Apple platforms. No GNUstep substitute, skipped test, empty image or
capability flag was used to turn that into a pass. The additive facade/build
wiring needs the real service test/Clippy replay on a Mac. The portable witness
is not evidence of native-app composition or encoded video delivery.

**Handoff state: BLOCKED**, pending authorized production compositor hooks and
the core/resource adapter. This branch deliberately does not emit a MERGE-READY
finish signal for an unfinished production integration. Read the concrete
remaining-delivery list above before resuming.

### Colour correctness repair — 2026-11-04

Review found that the earlier Rust import uploaded the map array as
`Rgba8UnormSrgb`, although `trace.wgsl` explicitly decodes color/emission maps.
That double-decoded mid-tones and incorrectly decoded data maps (normals/ORM).
The source's `core/studio/gpu.ts` uses raw `rgba8unorm`. All native map-array
uploads now use `Rgba8Unorm`, leaving the single intended sRGB decode in WGSL
and leaving normal/ORM values raw. N3 selects authored color/data semantics;
N2 rejects retaining the older import's darkening bug.

A new real-GPU regression renders an authored 8-bit grey screen and the same
scene with its independently calculated linear emission. It failed before the
repair and now requires **identical RGBA bytes for every pixel in every layer**.
No tolerance was introduced. The qualification counts above include this test.

## 2026-11-05 — Mac production hook (partial)

`Renderer::set_studio_source` now installs a typed immutable scene provider and
settings. `Render3d::Studio` carries composition, pinned revision, exact ticks,
output dimensions, Final/preview intent and the actual outer shutter count.
The native service attaches `DocumentSource` on its render worker and reconstructs
it in the windowless export child from the same document and media paths.
`pm-kernel::commands::scene3d::resolve_scene` supplies evaluated transforms,
materials, camera and lights. Procedural/explicit mesh geometry retains immutable
identity across provider replacement; the extractor and GPU BLAS cache survive.

Layer outputs enter the normal compositor as premultiplied encoded SDR textures,
with the FBO row convention applied during presentation. Masks, opacity, blends
and supported effects use the existing compositor. Final selects the authored
sample budget, rounded up per shutter with the source's four-sample minimum,
eight bounces and a deterministic seed/time salt. It cannot silently select Fast.
The export child composites and encodes these outputs through `pm-export`.

Headless Mac regressions cover the native kernel/service/compositor boundary,
an emissive card behind the camera reflected in a metal sphere, layer opacity,
repeated Final pixels, and two matching decoded shutter-enabled PNG exports.
These are offscreen Metal tests, not app-window captures or visual acceptance.
Work counters are emitted by the synthetic integration test; no performance
baseline or acceptance threshold changes here.

This is **not full Studio closure**. Imported model parts/materials, material
maps/slots/live screens and rounded-box geometry still require adapters and fail
explicitly rather than drawing placeholders. Fast, depth/id, view overrides,
full depth-set/barrier/background/catcher presentation, cancellation between GPU
slices and float/HDR Studio output remain unqualified. No panels, Blender,
release, packaging or physical-display acceptance is added by this hook.

## Imported resource adapter — 2026-11-05

`DocumentSource` now loads worker-owned model bytes through `pm-scene3d::render::Resource`.
OBJ and glTF/GLB use the core decoder and normalization; GLB assemblies retain
part transforms, shared meshes, source physical materials, and embedded maps.
The native CAD importer's converted GLB uses this same resource path. Geometry
identity hashes complete mesh data, so replacing a document source does not
invalidate an unchanged mesh's BLAS. Material extraction is linear in part count.
Owned content-addressed/`.bin` cache paths select their decoder from the saved
asset format (historical name as fallback), not the renamed display label. Only
the approved media path supplies bytes; saved names never grant file access.

Gaussian PLY/.splat resources take a separate native EWA raster pass: projected
covariance, stable depth sorting, source Gaussian cutoff and premultiplied layer
compositing. They never become substitute surface meshes or enter reflections.

Headless Metal tests in engine-service `tests/studio.rs`:

- `metal_imported_multi_object_reflection_preview_final_and_export` verifies two
  imported parts sharing geometry, source metal material, an off-camera card's
  reflection, exact fixed-seed repeat, preview, service output and two matching
  decoded shutter-enabled PNG exports from the windowless child.
- `metal_imported_splats_keep_gaussian_pixels_layer_order_and_export` verifies
  Gaussian center/falloff, exact repeat, preview, service parity, two matching
  decoded child PNG exports and zero surface traces for a splat-only composition.
- `metal_imported_step_keeps_xde_parts_preview_final_and_child_export`, enabled
  by `pm-engine-service/native-cad-tests`, generates actual STEP with the native
  OCCT writer, converts it through XDE, and verifies part/face materials,
  nonempty Metal pixels, preview, fixed-seed Final repeat, service parity and
  two matching decoded child PNG exports. It requires the real helper and
  fixture generator; absent tools fail the test.

**The imported model/CAD/splat headless preview/export gate is closed by these
passing tests.** All six Studio tests pass on Metal, including `.bin` cache
resources. Native OCCT 7.9.3 additionally passes all four decoder tests and the
three kernel STEP/IGES/BREP import/Undo/save/reopen tests. The helper and fixture
generator were built and development-signed on Mac; distribution packaging and
release signing remain separate gates.

This does not close full Studio acceptance. Authored material-map slots and live
screens, complete depth/barrier/catcher handling, Fast, cancellation between GPU
slices, float/HDR presentation and physical/visual acceptance remain open.
