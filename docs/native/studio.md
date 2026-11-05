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
