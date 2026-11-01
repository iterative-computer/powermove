# GPUI 0.3.8 RGB surface patch

Pinned upstream: `gpui-pre` / `gpui-pre-apple` **0.3.8**, the published
Zed snapshot `279fe070bb389b79652e52065b2f001edcc0b11b`. Apache-2.0 licenses
are included beside each crate. Copied only build/runtime sources, manifests,
resources and licenses; upstream examples, docs, test assets and lockfiles
are omitted. Cargo's root `[patch.crates-io]` selects these local sources.
No engine files or registry sources are modified.

Changes (search for `SurfaceCompletion`, `rgb_surface_fragment`):

* `gpui-pre/src/scene.rs`: optional retained `SurfaceCompletion` on a
  `PaintSurface`; its `gpu_completed` callback is invoked for each consuming
  Metal submission. Its `metal_registry_id` identifies the producing device.
* `gpui-pre/src/window.rs`: additive `paint_surface_with_completion` method.
  Existing `paint_surface` and `surface(CVPixelBuffer)` remain compatible.
* `gpui-pre-apple/src/metal_renderer.rs`: import packed BGRA8 (`BGRA`) and
  RGBA16Float (`RGhA`) as one plane through CoreVideo's Metal texture cache;
  preserve NV12 support. RGB gets its own premultiplied source-over pipeline
  (ONE / ONE_MINUS_SRC_ALPHA for both RGB and alpha). Validate producer
  Metal registry ID against the actual consumer device before sampling.
  Explicitly tag the BGRA8 CAMetalLayer as sRGB for ColorSync.
  The manifest adds objc2-core-graphics for that colorspace tag.
* `gpui-pre-apple/src/shaders.metal`: RGB sampling using the existing vertex
  positioning, top-down UVs, filtering, and clipping. No CPU pixel path.
* The consuming command buffer retains CVPixelBuffers, CVMetalTexture wrappers
  and completion leases until its Metal completed handler runs. Scene copies
  also retain the lease: a paused scene may be submitted again. Releasing a
  UI reference, returning from paint, or Metal scheduling is not completion.

RGB input contract: already display-encoded sRGB, **premultiplied** alpha,
top-down rows. GPUI's existing BGRA8 layer remains SDR. RGBA16F import is a
foundation only: it does not enable HDR/EDR, linear-layer output, or P3
presentation. Those require explicit layer/display colorspace work. The first
host implementation deliberately emits BGRA8 sRGB. Never convert graphics to
NV12 or use per-frame readback/upload to work around this contract.

`gpu_completed` means sampling has finished, **not** physical scanout. It can
run on a Metal thread, can run repeatedly for a retained frame, and should
remain short/nonblocking. Implementations must retain their pool lease. A
slot becomes writable only after every scene/UI/command-buffer lease is gone.
A command buffer abandoned before submission releases its closures normally;
no completion signal is promised for an unsubmitted buffer.

Upstream intent: propose the additive lifetime hook and packed RGB pipeline to
gpui-pre/GPUI. Keep panel-specific pooling, timing and project state outside the
fork. When upgrading, diff these four files against the published 0.3.8 sources,
port the small changes, and rerun native surface/alpha/resize and window proofs.
