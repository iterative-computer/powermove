//! GPUI RGB frame element. Add overlay children after `frame_element` in a
//! relative, clipped parent to keep all content in GPUI's ordinary scene order.
use core_foundation::base::TCFType;
use core_video::{
    pixel_buffer::CVPixelBuffer, pixel_buffer_io_surface::CVPixelBufferCreateWithIOSurface,
};
use gpui::{
    App, Bounds, ContentMask, Context, IntoElement, Pixels, Render, Window, canvas, div, point,
    prelude::*, px, size,
};
pub use pm_mac_engine_host::FrameHandle;
use std::sync::Arc;

/// Zoom is relative to fit; pan is in logical GPUI points. The renderer's
/// physical dimensions only determine aspect ratio and pixel density.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub zoom: f32,
    pub pan: [f32; 2],
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: [0.0; 2],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin: [f32; 2],
    pub size: [f32; 2],
    pub scale: f32,
}
impl Placement {
    /// All output values are logical points. `retina` only snaps the origin;
    /// GPUI applies its backing scale when painting the Metal scene.
    pub fn fit(
        bounds: [f32; 4],
        source: [u32; 2],
        viewport: Viewport,
        retina: f32,
    ) -> Option<Self> {
        let [x, y, w, h] = bounds;
        if source.contains(&0)
            || !bounds.iter().all(|v| v.is_finite())
            || w <= 0.0
            || h <= 0.0
            || !viewport.zoom.is_finite()
            || viewport.zoom <= 0.0
            || !viewport.pan.iter().all(|v| v.is_finite())
            || !retina.is_finite()
            || retina <= 0.0
        {
            return None;
        }
        let scale = (w / source[0] as f32).min(h / source[1] as f32) * viewport.zoom;
        let size = [source[0] as f32 * scale, source[1] as f32 * scale];
        if !scale.is_finite() || !size.iter().all(|v| v.is_finite()) {
            return None;
        }
        let snap = |v: f32| (v * retina).round() / retina;
        Some(Self {
            origin: [
                snap(x + (w - size[0]) * 0.5 + viewport.pan[0]),
                snap(y + (h - size[1]) * 0.5 + viewport.pan[1]),
            ],
            size,
            scale,
        })
    }
    pub fn view_to_pixel(&self, p: [f32; 2]) -> [f32; 2] {
        [
            (p[0] - self.origin[0]) / self.scale,
            (p[1] - self.origin[1]) / self.scale,
        ]
    }
}

#[derive(Debug)]
struct Consumer(FrameHandle);
impl gpui::SurfaceCompletion for Consumer {
    fn metal_registry_id(&self) -> Option<u64> {
        Some(self.0.metal_registry_id)
    }
    fn gpu_completed(&self) {
        self.0.consumer_completed();
    }
}

/// Wraps the retained IOSurface without copying or mapping its pixels. The
/// CoreVideo create rule hands us +1, transferred into GPUI's CF wrapper. The
/// CVPixelBuffer retains the IOSurface independently; the completion lease
/// separately prevents the pool from rewriting it while a consumer samples it.
pub fn pixel_buffer(frame: &FrameHandle) -> Result<CVPixelBuffer, i32> {
    let io = &*frame.surface.iosurface.0;
    let mut buffer = std::ptr::null_mut();
    // SAFETY: Both binding families describe the same C IOSurfaceRef ABI. `io`
    // remains retained for the entire call. No ownership is transferred to CV.
    let status = unsafe {
        CVPixelBufferCreateWithIOSurface(
            std::ptr::null(),
            std::ptr::from_ref(io).cast_mut().cast(),
            std::ptr::null(),
            &mut buffer,
        )
    };
    if status != 0 {
        return Err(status);
    }
    // SAFETY: successful Create returned +1; the wrapper releases exactly once.
    Ok(unsafe { CVPixelBuffer::wrap_under_create_rule(buffer) })
}

/// A styled GPUI element, fitted/clipped to its layout bounds. Later siblings
/// can paint handles, guides, captions, etc. above these pixels.
pub fn frame_element(frame: FrameHandle, viewport: Viewport) -> impl IntoElement + Styled {
    let buffer = pixel_buffer(&frame);
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
            let Some(place) = Placement::fit(
                [
                    bounds.origin.x.into(),
                    bounds.origin.y.into(),
                    bounds.size.width.into(),
                    bounds.size.height.into(),
                ],
                [frame.size.width, frame.size.height],
                viewport,
                window.scale_factor(),
            ) else {
                return;
            };
            match buffer {
                Ok(buffer) => {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        window.paint_surface_with_completion(
                            Bounds::new(
                                point(px(place.origin[0]), px(place.origin[1])),
                                size(px(place.size[0]), px(place.size[1])),
                            ),
                            buffer,
                            Some(Arc::new(Consumer(frame))),
                        );
                    });
                }
                Err(code) => eprintln!("CVPixelBufferCreateWithIOSurface failed: {code}"),
            }
        },
    )
}

/// Minimal panel view for shell integration. The shell owns the host and feeds
/// its latest immutable frame here; notifications stay on GPUI's main thread.
pub struct Viewer {
    pub viewport: Viewport,
    frame: Option<FrameHandle>,
}
impl Default for Viewer {
    fn default() -> Self {
        Self {
            viewport: Viewport::default(),
            frame: None,
        }
    }
}
impl Viewer {
    pub fn set_frame(&mut self, frame: FrameHandle, cx: &mut Context<Self>) {
        self.frame = Some(frame);
        cx.notify();
    }
    pub fn frame(&self) -> Option<&FrameHandle> {
        self.frame.as_ref()
    }
}
impl Render for Viewer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(gpui::rgb(0x141416))
            .when_some(self.frame.clone(), |el, frame| {
                el.child(frame_element(frame, self.viewport).size_full())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_letterboxes_in_logical_points_on_retina() {
        let p = Placement::fit(
            [10.0, 20.0, 800.0, 600.0],
            [1920, 1080],
            Viewport::default(),
            2.0,
        )
        .unwrap();
        assert_eq!(p.origin, [10.0, 95.0]);
        assert_eq!(p.size, [800.0, 450.0]);
        assert_eq!(p.view_to_pixel([410.0, 320.0]), [960.0, 540.0]);
    }
    #[test]
    fn zoom_pan_and_backing_pixel_snap() {
        let p = Placement::fit(
            [0.0, 0.0, 800.0, 600.0],
            [800, 600],
            Viewport {
                zoom: 2.0,
                pan: [10.2, -8.3],
            },
            2.0,
        )
        .unwrap();
        assert_eq!(p.origin, [-390.0, -308.5]);
        assert_eq!(p.size, [1600.0, 1200.0]);
    }
    #[test]
    fn invalid_geometry_is_not_painted() {
        assert!(Placement::fit([0.0; 4], [1920, 1080], Viewport::default(), 2.0).is_none());
        assert!(
            Placement::fit(
                [0.0, 0.0, 800.0, 600.0],
                [0, 1080],
                Viewport::default(),
                2.0
            )
            .is_none()
        );
    }
}
