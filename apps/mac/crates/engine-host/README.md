# Native viewer engine host

This is an ordinary Rust library over `pm-gpu`, `pm-render`, `pm-model` and
`pm-node`'s **feature-disabled** Rust font helpers. No napi, Node runtime,
WASM module, browser, interprocess bridge or CPU presentation upload is used.

## Integration

1. On AppKit main: `MainThreadFonts::new(wake)`; retain it for the session.
   `service_mut()` exposes the engine's existing project-font API.
2. Start `EngineHost::start(fonts.broker())`. Initialization is asynchronous.
3. Send `RenderRequest { document: Arc<Document>, composition, time: Ticks,
   size: Size, assets: Arc<dyn AssetResolver> }`. Canonical document snapshots
   and real asset resolution belong to core. `size` is physical pixels.
4. Await coalesced `updates().recv()` and take `latest()`. Also inspect
   `last_error()`; errors preserve the last good frame. Never wait on the
   renderer from the UI. Pump `MainThreadFonts::pump()` on main when its wake
   fires. The standalone demo uses an 8 ms main-thread timer for pumping.
5. Hand the frame to `pm-mac-panel-viewer::frame_element` or `Viewer::set_frame`.
   Its GPUI lease calls `FrameHandle::consumer_completed` from Metal completion.

`FrameHandle` carries `surface: Arc<ExportedSurface>`, `revision`, `time`,
`size`, pool `generation`, `slot`, monotonic `request_id`, producer
`metal_registry_id`, sRGB `color_space`, `requested_at` and `ready_at`.
`display_latency()` records first consumer GPU completion, not scanout.

The worker owns the renderer, caches and working targets. It installs
`all_builtin_definitions` and `RenderFonts` and refuses failed font/layer renders.
Core installs extension definitions, media interpretation and project fonts
before requesting those features; those wider application services remain
integration work. This first facade is intentionally SDR and composition-fit.

## Pixels and lifetime

`render_into` draws a size-matched project-precision target. The output is
premultiplied E-encoded RGB; a GPU pass removes premultiplication before the
engine's `DisplayTransform::encode` (Standard → sRGB BGRA8) applies display
conversion/dither once and premultiplies its result. This prevents double
alpha. Nothing maps or reads pixels in the presentation path.

A three-slot ring holds exported surfaces. The worker publishes only after
producer GPU completion, waiting on its own thread. Only the newest request
may publish; pending requests collapse into one. The ring may rewrite a slot
only when its Arc is the sole owner. UI handles, retained GPUI scenes and
Metal completion blocks all own leases. Busy pools leave the last frame on
screen; a timed worker wait notices lease release without blocking AppKit.
On resize, only free slots are recreated, so allocations stay bounded at
three exported surfaces. Old-size frames remain valid through their fences.
Drop cancels pending work and lets the worker retire off-main. Dropping the
main-thread font service closes its outstanding asks; dropping one host does
not close a broker shared by other hosts.

The worker matches GPUI 0.3.8's Metal device-selection policy. It first tries
`Gpu::new`, then enumerates Metal adapters if needed and selects one whose
registry ID matches GPUI's preferred device (including integrated Intel GPUs).
GPUI additionally checks the actual consuming renderer's registry ID before
sampling. No cross-device coherence is assumed. This machine's Apple Silicon
path is validated; the multi-GPU fallback needs hardware coverage.

## Validation

Use the shared target and engine environment; check free disk before builds:

```
source engine/env.sh
export CARGO_TARGET_DIR=~/Developer/pm-target
cd apps/mac
cargo test -p pm-mac-engine-host -p pm-mac-panel-viewer
cargo build -p pm-mac-panel-viewer --examples
```

The surface integration test has its own main-thread harness for CoreText.
Diagnostic IOSurface readback is confined to that test. It checks an actual
PMV4 with embedded image, real text ink, shape/background channels, alpha,
held ring slots, resize generations, coalescing and last-good error behavior.

The panel-viewer's `rgb_surface` test additionally passes these real surfaces
through GPUI Apple's offscreen Metal renderer. It verifies RGB import, clipping,
overlay z-order, later animation ticks, premultiplied source-over, and both
repeated and asynchronous consuming-command-buffer completion. This is
GPU-backed automation; it does not certify window activation, live keyboard
input, display ColorSync, physical scanout or on-screen Retina behavior.
