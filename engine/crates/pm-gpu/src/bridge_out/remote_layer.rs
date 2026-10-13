//! The C′ prototype (plan section 18 "Fallbacks"; amendments 5 and 18): the
//! engine presents playback frames outside Chromium's compositor through a
//! remote Core Animation layer.
//!
//! The interactive engine makes a `CAContext` on its own WindowServer
//! connection with a `CAMetalLayer` as its layer, and main hosts the
//! context's id with a `CALayerHost` in the viewer's window. Frames already
//! rendered into a pool surface are copied into the layer's next drawable
//! and presented at their display time with `presentDrawable:atTime:`; Core
//! Animation reports when each one actually reached the display
//! (`addPresentedHandler`, `presentedTime`).
//!
//! **Private API.** `CAContext`, `CALayerHost`, and `CGSMainConnectionID`
//! are not public macOS API. Chromium has used exactly this pair to move its
//! GPU process's layers into the browser process since macOS 10.9, which is
//! why it keeps working across releases, but Apple documents none of it and
//! may change it without notice; App Store review rejects it (Powermove
//! ships outside the store). The risk evaluation is in
//! docs/engine/bridge.md ("C′ prototype"). This module is the prototype that
//! evaluation measured; nothing uses it unless the presenter is started with
//! `POWERMOVE_BRIDGE_REMOTE_LAYER=1`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGColorSpace;
use objc2_foundation::NSDictionary;
use objc2_metal::{
    MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice,
    MTLDrawable, MTLPixelFormat, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer, CATransaction};
use pm_protocol::color::ViewerColorSpace;
use pm_protocol::common::Size;
use pm_protocol::interop::SurfaceFormat;

use super::surface::SharedIoSurface;

unsafe extern "C" {
    /// The process's main WindowServer connection (SkyLight, re-exported by
    /// CoreGraphics). Private.
    fn CGSMainConnectionID() -> u32;
}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {}

/// One drawable's fate: presented at a host time, or dropped by Core
/// Animation (`presentedTime` 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Presented {
    pub seq: u64,
    /// Host µs the drawable was asked to show at.
    pub asked_us: i64,
    /// Host µs it reached the display, or `None` if it never did.
    pub presented_us: Option<i64>,
}

/// A `CAContext` with a `CAMetalLayer`, presenting copies of pool surfaces.
pub struct RemoteLayer {
    context: Retained<AnyObject>,
    layer: Retained<CAMetalLayer>,
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    size: Size,
    format: SurfaceFormat,
    /// Metal textures over pool surfaces, by IOSurface id.
    sources: HashMap<u32, Retained<ProtocolObject<dyn MTLTexture>>>,
    presented: Arc<Mutex<Vec<Presented>>>,
}

// SAFETY: the layer, context, device, and queue are used from the one
// render thread that owns this value; Core Animation and Metal objects may
// be used from any single thread with explicit transactions.
unsafe impl Send for RemoteLayer {}

#[derive(Debug, thiserror::Error)]
pub enum RemoteLayerError {
    #[error("this process has no WindowServer connection")]
    NoWindowServer,
    #[error("{0} is unavailable")]
    Missing(&'static str),
    #[error("{0}")]
    Metal(&'static str),
}

fn metal_format(format: SurfaceFormat) -> Option<MTLPixelFormat> {
    match format {
        SurfaceFormat::Bgra8Unorm => Some(MTLPixelFormat::BGRA8Unorm),
        SurfaceFormat::Rgba16Float => Some(MTLPixelFormat::RGBA16Float),
        _ => None,
    }
}

fn color_space(
    space: ViewerColorSpace,
    extended: bool,
) -> Option<objc2_core_foundation::CFRetained<CGColorSpace>> {
    // SAFETY: the name constants are exported by CoreGraphics.
    let name = unsafe {
        match (space, extended) {
            (ViewerColorSpace::DisplayP3, true) => {
                objc2_core_graphics::kCGColorSpaceExtendedDisplayP3
            }
            (ViewerColorSpace::DisplayP3, false) => objc2_core_graphics::kCGColorSpaceDisplayP3,
            (_, true) => objc2_core_graphics::kCGColorSpaceExtendedSRGB,
            (_, false) => objc2_core_graphics::kCGColorSpaceSRGB,
        }
    };
    CGColorSpace::with_name(Some(name))
}

/// The refresh rate to ask the display for while content at `content_fps`
/// plays (plan section 15: a multiple of the content rate): the largest
/// multiple not above `max_hz`, or `max_hz` when the content is faster.
/// 24 → 120, 25 → 100, 29.97 → 119.88, 30 → 120, 50 → 100, 60 → 120 on a
/// 120 Hz ProMotion panel.
pub fn preferred_refresh(content_fps: f64, max_hz: f64) -> f64 {
    let valid = content_fps.is_finite() && max_hz.is_finite() && content_fps > 0.0 && max_hz > 0.0;
    if !valid || content_fps >= max_hz {
        return max_hz;
    }
    (max_hz / content_fps + 1e-9).floor() * content_fps
}

objc2::define_class!(
    /// The display link's target: the link only carries the rate request.
    #[unsafe(super(objc2_foundation::NSObject))]
    struct RateTarget;

    impl RateTarget {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &AnyObject) {
            // SAFETY: CADisplayLink's timestamp properties.
            let (now, target): (f64, f64) = unsafe { (msg_send![link, timestamp], msg_send![link, targetTimestamp]) };
            if let Ok(mut ticks) = TICKS.lock()
                && ticks.len() < MAX_TICKS
            {
                ticks.push((now, target));
            }
        }
    }
);

/// Display link callbacks seen (`timestamp`, `targetTimestamp`, seconds on
/// the host clock), for the refresh-rate probe; capped.
static TICKS: Mutex<Vec<(f64, f64)>> = Mutex::new(Vec::new());
const MAX_TICKS: usize = 20_000;

/// The display link callbacks recorded so far, emptied: when the display
/// actually refreshed while a [`RatePreference`] lived.
pub fn take_display_ticks() -> Vec<(f64, f64)> {
    std::mem::take(&mut *TICKS.lock().unwrap_or_else(|e| e.into_inner()))
}

/// A display link that asks the main display for a refresh rate while it
/// lives (`-[NSScreen displayLinkWithTarget:selector:]` and
/// `preferredFrameRateRange`, macOS 14). It runs on its own run-loop
/// thread; its callbacks do nothing.
pub struct RatePreference {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The rate asked for (0 when only observing).
    pub hz: f64,
}

impl RatePreference {
    /// Asks for `hz` (exactly: minimum = maximum = preferred).
    pub fn request(hz: f64) -> Result<Self, RemoteLayerError> {
        Self::start(Some(hz))
    }

    /// A display link that asks for nothing (it only observes refreshes).
    pub fn observe() -> Result<Self, RemoteLayerError> {
        Self::start(None)
    }

    fn start(want: Option<f64>) -> Result<Self, RemoteLayerError> {
        let hz = want.unwrap_or(0.0);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), RemoteLayerError>>();
        let thread = std::thread::Builder::new()
            .name("pm-remote-layer-rate".into())
            .spawn(move || {
                let screen_class = match AnyClass::get(c"NSScreen") {
                    Some(c) => c,
                    None => {
                        let _ = tx.send(Err(RemoteLayerError::Missing("NSScreen")));
                        return;
                    }
                };
                // SAFETY: class methods and properties of AppKit and Core
                // Animation, used on this thread only.
                unsafe {
                    let screen: Option<Retained<AnyObject>> = msg_send![screen_class, mainScreen];
                    let Some(screen) = screen else {
                        let _ = tx.send(Err(RemoteLayerError::Missing("a main screen")));
                        return;
                    };
                    let target: Retained<RateTarget> = msg_send![<RateTarget as objc2::ClassType>::class(), new];
                    let link: Option<Retained<AnyObject>> = msg_send![&*screen, displayLinkWithTarget: &*target, selector: objc2::sel!(tick:)];
                    let Some(link) = link else {
                        let _ = tx.send(Err(RemoteLayerError::Missing("a display link")));
                        return;
                    };
                    if want.is_some() {
                        let range = objc2_quartz_core::CAFrameRateRange {
                            minimum: hz as f32,
                            maximum: hz as f32,
                            preferred: hz as f32,
                        };
                        let _: () = msg_send![&*link, setPreferredFrameRateRange: range];
                    }
                    let run_loop: Retained<AnyObject> = msg_send![AnyClass::get(c"NSRunLoop").expect("NSRunLoop"), currentRunLoop];
                    let _: () = msg_send![&*link, addToRunLoop: &*run_loop, forMode: objc2_foundation::NSRunLoopCommonModes];
                    let _ = tx.send(Ok(()));
                    while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                        objc2_core_foundation::CFRunLoop::run_in_mode(objc2_core_foundation::kCFRunLoopDefaultMode, 0.25, false);
                    }
                    let _: () = msg_send![&*link, invalidate];
                }
            })
            .map_err(|_| RemoteLayerError::Missing("a thread"))?;
        match rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                thread: Some(thread),
                hz,
            }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(RemoteLayerError::Missing("the display link thread")),
        }
    }
}

impl Drop for RatePreference {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The Metal device under a wgpu device (the presenter's), for the layer.
pub fn raw_metal_device(device: &wgpu::Device) -> Option<Retained<ProtocolObject<dyn MTLDevice>>> {
    // SAFETY: the hal device is only read to clone its retained MTLDevice.
    let hal = unsafe { device.as_hal::<wgpu_hal::api::Metal>() }?;
    Some(hal.raw_device().clone())
}

impl RemoteLayer {
    /// Makes the context and its layer for frames of `size` device pixels
    /// at `scale` device pixels per point. Call on the render thread.
    pub fn new(
        device: Retained<ProtocolObject<dyn MTLDevice>>,
        size: Size,
        scale: f64,
        format: SurfaceFormat,
        space: ViewerColorSpace,
    ) -> Result<Self, RemoteLayerError> {
        let pixel_format =
            metal_format(format).ok_or(RemoteLayerError::Metal("not a viewer format"))?;
        // SAFETY: a plain C function with no arguments.
        let connection = unsafe { CGSMainConnectionID() };
        if connection == 0 {
            return Err(RemoteLayerError::NoWindowServer);
        }
        let class = AnyClass::get(c"CAContext").ok_or(RemoteLayerError::Missing("CAContext"))?;
        let queue = device
            .newCommandQueue()
            .ok_or(RemoteLayerError::Metal("newCommandQueue failed"))?;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&device));
        layer.setPixelFormat(pixel_format);
        layer.setFramebufferOnly(false);
        layer.setDrawableSize(CGSize::new(f64::from(size.width), f64::from(size.height)));
        layer.setContentsScale(scale);
        layer.setFrame(CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(
                f64::from(size.width) / scale,
                f64::from(size.height) / scale,
            ),
        ));
        layer.setOpaque(true);
        layer.setMaximumDrawableCount(3);
        layer.setAllowsNextDrawableTimeout(true);
        let extended = format == SurfaceFormat::Rgba16Float;
        layer.setWantsExtendedDynamicRangeContent(extended);
        layer.setColorspace(color_space(space, extended).as_deref());
        let options = NSDictionary::<AnyObject, AnyObject>::new();
        // SAFETY: `+[CAContext contextWithCGSConnection:options:]` takes the
        // connection id and an options dictionary and returns an autoreleased
        // context (Chromium's ui/accelerated_widget_mac).
        let context: Option<Retained<AnyObject>> =
            unsafe { msg_send![class, contextWithCGSConnection: connection, options: &*options] };
        let Some(context) = context else {
            CATransaction::commit();
            return Err(RemoteLayerError::Missing("a CAContext for this connection"));
        };
        // SAFETY: `setLayer:` takes a CALayer.
        let _: () = unsafe { msg_send![&*context, setLayer: &*layer] };
        CATransaction::commit();
        CATransaction::flush();
        Ok(Self {
            context,
            layer,
            device,
            queue,
            size,
            format,
            sources: HashMap::new(),
            presented: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// The id main's `CALayerHost` shows (`-[CAContext contextId]`).
    pub fn context_id(&self) -> u32 {
        // SAFETY: a readonly uint32 property.
        unsafe { msg_send![&*self.context, contextId] }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn format(&self) -> SurfaceFormat {
        self.format
    }

    fn source(
        &mut self,
        surface: &SharedIoSurface,
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, RemoteLayerError> {
        let id = surface.0.id();
        if let Some(texture) = self.sources.get(&id) {
            return Ok(texture.clone());
        }
        let pixel_format =
            metal_format(self.format).ok_or(RemoteLayerError::Metal("not a viewer format"))?;
        // SAFETY: a 2D, single-mip descriptor of the pool's format and size.
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                pixel_format,
                self.size.width as usize,
                self.size.height as usize,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Shared);
        let texture = self
            .device
            .newTextureWithDescriptor_iosurface_plane(&desc, &surface.0, 0)
            .ok_or(RemoteLayerError::Metal(
                "newTextureWithDescriptor:iosurface:plane: failed",
            ))?;
        self.sources.insert(id, texture.clone());
        Ok(texture)
    }

    /// Copies `surface` (a pool surface of this layer's size and format,
    /// whose GPU work completed) into the next drawable and presents it at
    /// host µs `at_us`. Blocks up to a second when all three drawables are
    /// queued. Returns when the copy is committed; its fate arrives later
    /// in [`Self::take_presented`].
    pub fn present(
        &mut self,
        surface: &SharedIoSurface,
        seq: u64,
        at_us: i64,
    ) -> Result<(), RemoteLayerError> {
        let source = self.source(surface)?;
        let drawable = self
            .layer
            .nextDrawable()
            .ok_or(RemoteLayerError::Metal("no drawable within a second"))?;
        let target = drawable.texture();
        let buffer = self
            .queue
            .commandBuffer()
            .ok_or(RemoteLayerError::Metal("commandBuffer failed"))?;
        let blit = buffer
            .blitCommandEncoder()
            .ok_or(RemoteLayerError::Metal("blitCommandEncoder failed"))?;
        // SAFETY: equal sizes and pixel formats (both from this layer's spec).
        unsafe { blit.copyFromTexture_toTexture(&source, &target) };
        blit.endEncoding();
        let presented = self.presented.clone();
        let handler = RcBlock::new(
            move |drawable: std::ptr::NonNull<ProtocolObject<dyn MTLDrawable>>| {
                // SAFETY: Metal passes the live drawable to its presented handler.
                let time = unsafe { drawable.as_ref() }.presentedTime();
                let presented_us = (time > 0.0).then(|| (time * 1e6).round() as i64);
                if let Ok(mut list) = presented.lock() {
                    list.push(Presented {
                        seq,
                        asked_us: at_us,
                        presented_us,
                    });
                }
            },
        );
        let as_drawable: &ProtocolObject<dyn MTLDrawable> = ProtocolObject::from_ref(&*drawable);
        // SAFETY: the handler is retained by Metal until it runs.
        unsafe { as_drawable.addPresentedHandler(RcBlock::as_ptr(&handler)) };
        // Host µs and CACurrentMediaTime share mach_absolute_time's base.
        buffer.presentDrawable_atTime(as_drawable, at_us as f64 / 1e6);
        buffer.commit();
        // The pool surface is free again once the copy ran.
        buffer.waitUntilCompleted();
        Ok(())
    }

    /// Presentation results so far, emptied.
    pub fn take_presented(&self) -> Vec<Presented> {
        std::mem::take(&mut *self.presented.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Forgets the textures of a retired pool.
    pub fn forget_sources(&mut self) {
        self.sources.clear();
    }
}

impl Drop for RemoteLayer {
    fn drop(&mut self) {
        CATransaction::begin();
        // SAFETY: detaching the layer from the context before both go.
        let _: () = unsafe { msg_send![&*self.context, setLayer: std::ptr::null::<AnyObject>()] };
        CATransaction::commit();
        CATransaction::flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_out::surface::ExportedSurface;
    use pm_protocol::interop::SurfaceRequest;

    #[test]
    fn the_preferred_refresh_is_a_multiple_of_the_content_rate() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        assert!(close(preferred_refresh(24.0, 120.0), 120.0));
        assert!(close(preferred_refresh(25.0, 120.0), 100.0));
        assert!(close(
            preferred_refresh(30_000.0 / 1001.0, 120.0),
            120_000.0 / 1001.0
        ));
        assert!(close(preferred_refresh(30.0, 120.0), 120.0));
        assert!(close(preferred_refresh(50.0, 120.0), 100.0));
        assert!(close(preferred_refresh(60.0, 120.0), 120.0));
        assert!(close(preferred_refresh(144.0, 120.0), 120.0));
        assert!(close(preferred_refresh(25.0, 60.0), 50.0));
    }

    /// A context with a layer exists, has an id main can host, and takes
    /// copies of pool surfaces with a presentation time. Nothing hosts the
    /// context here, so nothing reaches a display.
    #[test]
    fn a_remote_layer_has_a_context_id_and_takes_pool_surfaces() {
        let (device, _queue, _) = crate::bridge_out::metal_device("remote layer test").unwrap();
        let raw = raw_metal_device(&device).expect("a Metal device");
        let size = Size::new(64, 32);
        let mut layer = match RemoteLayer::new(
            raw,
            size,
            2.0,
            SurfaceFormat::Bgra8Unorm,
            ViewerColorSpace::Srgb,
        ) {
            Ok(layer) => layer,
            // A process without a WindowServer session (a headless runner).
            Err(RemoteLayerError::NoWindowServer) => {
                eprintln!("skipped: no WindowServer connection");
                return;
            }
            Err(error) => panic!("{error}"),
        };
        assert_ne!(layer.context_id(), 0);
        eprintln!("remote layer context {}", layer.context_id());
        assert_eq!(
            (layer.size(), layer.format()),
            (size, SurfaceFormat::Bgra8Unorm)
        );
        let surface = ExportedSurface::create(
            &device,
            &SurfaceRequest {
                size,
                format: SurfaceFormat::Bgra8Unorm,
            },
        )
        .unwrap();
        let now = pm_protocol_now_us();
        layer.present(&surface.iosurface, 1, now + 20_000).unwrap();
        layer.present(&surface.iosurface, 2, now + 40_000).unwrap();
        // One Metal texture per pool surface.
        assert_eq!(layer.sources.len(), 1);
    }

    fn pm_protocol_now_us() -> i64 {
        // CACurrentMediaTime's base (mach_absolute_time) in µs.
        (objc2_quartz_core::CACurrentMediaTime() * 1e6) as i64
    }
}
