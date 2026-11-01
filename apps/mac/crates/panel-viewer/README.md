# Viewer panel

`frame_element(FrameHandle, Viewport) -> impl IntoElement + Styled` displays
an immutable engine frame in any GPUI layout. Set its size using normal GPUI
styling. It fits/zooms/pans inside its own bounds, clips to the intersection
with its parent's content mask, and participates in ordinary GPUI z-order.
Add overlay children after it in a relative parent.

`Viewer::default()` is a minimal GPUI view; feed it with
`Viewer::set_frame(frame, cx)` and change `viewer.viewport` as needed.
`Viewer::frame()` exposes the displayed revision/time for overlay alignment.
It intentionally contains no document authority or render scheduler.

`Viewport { zoom, pan: [x,y] }`: zoom 1 means fit; pan is logical points.
`Placement::fit([x,y,w,h], [source_w,source_h], viewport, retina)` returns
logical origin/size/scale; `view_to_pixel` maps overlay pointer coordinates to
source pixels. Retina scale snaps origins to backing pixels. GPUI alone
performs logical→physical scaling when it records the Metal scene. A shell
can request more engine pixels as the backing scale/zoom changes; stretching
an existing frame while a sharper replacement arrives is safe.

`pixel_buffer(&FrameHandle)` wraps the IOSurface through
`CVPixelBufferCreateWithIOSurface` with no mapping/copy. It audits the transfer
between objc2's retained IOSurface and GPUI's CoreFoundation wrapper: borrow
the IOSurface pointer, adopt the returned CVPixelBuffer's create-rule +1.
The pixel buffer retains the IOSurface. The separate completion lease keeps
the ring slot immutable through scene retirement and Metal completion.

## Demo

```
source engine/env.sh
export CARGO_TARGET_DIR=~/Developer/pm-target
cd apps/mac
cargo build -p pm-mac-panel-viewer --examples
~/Developer/pm-target/debug/examples/frames [optional-project.pmv]
```

Left/right arrows scrub one rational-rate frame; Shift steps ten. The overlay
badge is a normal GPUI child above the surface. Title/log latency measures
request→first consuming GPU completion, not physical scanout. Last good pixels
stay on screen if a new request fails; the bottom line exposes the error.
The demo loads the file before opening its window. A production shell should
load on its document worker, supply the immutable snapshot/assets, and use the
font wake callback to schedule main-thread pumping.

`fixtures/frames.pmv` is a small real PMV4 project, containing an original
embedded PNG color-bar still, an animated shape using the engine's
`shape-cases.json` rect definition, and the Georgia text from the corpus's
`text-level2/range-position` fixture. It is a synthetic rendering fixture,
not user data or a screenshot. PMV4 document/index/media hashes are verified
on read. It is four seconds at 30 fps, 640×360. Other files can be passed as
an argument; this demo resolves embedded media and explicit linked sourcePath
assets, but does not replace core's catalog, font import, or save envelope.

SDR sRGB BGRA8 is supported. The patch can sample RGBA16F but EDR/P3 output,
ROI/canvas parity at extreme zoom, production transport, media decode retry,
gizmos and accessibility are later integration work. No AppKit Metal subview,
NV12 conversion or per-frame CPU readback is used.

## Automated proof without an unlocked screen

`cargo test -p pm-mac-engine-host -p pm-mac-panel-viewer` runs the main-thread
engine fixture test, placement tests, and `tests/rgb_surface.rs`. The last test
uses GPUI Apple's existing `new_headless` / `render_scene_to_image` APIs with
real engine-host frames, not CPU image uploads. Pixel readback is restricted to
these diagnostics. It checks packed BGRA import, colors, clipping, overlays,
animation, alpha blending and repeated/asynchronous consumer completion.
The normal demo continues to use the platform's ordinary frame loop; it does
not force rendering while macOS marks its window occluded or screen-locked.
Live keyboard, resizing, display color management and screenshots need an
unlocked Mac and remain separate acceptance evidence.
