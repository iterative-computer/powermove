//! On-screen presentation with presentation timestamps.
//!
//! A borderless, non-activating, click-through window hosts a `CAMetalLayer` with a 4K
//! drawable. The app runs with the accessory activation policy (no Dock icon, no menu
//! bar, never becomes active), and the window is ordered front without activating, so
//! it never takes focus. Frames rendered by wgpu into the drawable's texture are
//! presented with a raw Metal command buffer on the same queue (ordered after the
//! frame), and each drawable's `presentedTime` is collected from its presented handler:
//! the host time the frame actually reached the glass, or 0 if the compositor skipped
//! it.

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSFloatingWindowLevel, NSScreen, NSWindow,
    NSWindowCollectionBehavior, NSWindowOcclusionState, NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
use objc2_metal::{MTLCommandBuffer, MTLCommandQueue, MTLDrawable, MTLPixelFormat, MTLTextureType};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Where the window goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// A small window in the bottom-left corner of the main screen.
    Corner,
    /// Entirely outside every screen (the compositor may then skip presentation).
    Offscreen,
}

impl Placement {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "corner" => Self::Corner,
            "offscreen" => Self::Offscreen,
            _ => return None,
        })
    }
}

/// Window and layer; lives on the main thread.
pub struct Window {
    window: Retained<NSWindow>,
    pub layer: Retained<CAMetalLayer>,
}

impl Window {
    /// Whether AppKit reports any part of the window as visible on screen.
    pub fn visible(&self) -> bool {
        self.window.occlusionState().contains(NSWindowOcclusionState::Visible)
    }
}

/// Create the window and its layer. `device` must be the Metal device of the queue
/// that will present.
pub fn create_window(
    mtm: MainThreadMarker,
    device: &ProtocolObject<dyn objc2_metal::MTLDevice>,
    width: u32,
    height: u32,
    placement: Placement,
    points: (f64, f64),
) -> Window {
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let origin = match placement {
        Placement::Corner => {
            let f = NSScreen::mainScreen(mtm).map(|s| s.visibleFrame()).unwrap_or_default();
            NSPoint::new(f.origin.x, f.origin.y)
        }
        Placement::Offscreen => NSPoint::new(-30000.0, -30000.0),
    };
    let rect = NSRect::new(origin, NSSize::new(points.0, points.1));
    // SAFETY: standard AppKit initialisation on the main thread.
    let window = unsafe {
        let w = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        );
        w.setReleasedWhenClosed(false);
        w
    };
    window.setIgnoresMouseEvents(true);
    if placement == Placement::Corner {
        // Above normal windows and on every Space (including a full-screen app's), so
        // the compositor actually shows it; still click-through and never key.
        window.setLevel(NSFloatingWindowLevel);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
    }
    window.setHasShadow(false);
    window.setOpaque(true);
    let layer = CAMetalLayer::new();
    layer.setDevice(Some(device));
    layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    layer.setFramebufferOnly(true);
    layer.setDrawableSize(objc2_core_foundation::CGSize::new(width as f64, height as f64));
    layer.setMaximumDrawableCount(3);
    layer.setDisplaySyncEnabled(true);
    layer.setAllowsNextDrawableTimeout(true);
    let view = window.contentView().expect("content view");
    view.setWantsLayer(true);
    view.setLayer(Some(&layer));
    if placement == Placement::Offscreen {
        // Borderless windows may sit outside the screens; keep AppKit from moving it.
        window.setFrame_display(rect, false);
    }
    window.orderFrontRegardless();
    // Let AppKit and Core Animation commit the window before the first frame.
    pump(mtm, 0.2);
    Window { window, layer }
}

/// Run the main run loop for `seconds`.
pub fn pump(_mtm: MainThreadMarker, seconds: f64) {
    let rl = NSRunLoop::mainRunLoop();
    let until = NSDate::dateWithTimeIntervalSinceNow(seconds);
    rl.runUntilDate(&until);
}

/// Pump the main run loop until `done` is set.
pub fn pump_until(mtm: MainThreadMarker, done: &AtomicBool) {
    while !done.load(Ordering::Acquire) {
        pump(mtm, 0.05);
    }
}

/// Render-thread side: acquires drawables, wraps them for wgpu, presents them.
pub struct Presenter {
    layer: Retained<CAMetalLayer>,
    width: u32,
    height: u32,
    /// Presented host time (ns) per frame index; 0 = skipped by the compositor.
    pub presented: Arc<Mutex<Vec<(u64, u64)>>>,
}

// SAFETY: CAMetalLayer's nextDrawable and drawable presentation are thread-safe; the
// layer is retained by the window, which outlives the presenter (main joins the render
// thread before returning).
unsafe impl Send for Presenter {}

pub struct Frame {
    drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>,
    pub texture: wgpu::Texture,
}

impl Presenter {
    pub fn new(layer: Retained<CAMetalLayer>, width: u32, height: u32) -> Self {
        Self {
            layer,
            width,
            height,
            presented: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Acquire the next drawable (blocks while all drawables are in use, up to 1 s).
    pub fn acquire(&self, device: &wgpu::Device) -> Option<Frame> {
        let drawable = objc2::rc::autoreleasepool(|_| self.layer.nextDrawable())?;
        let raw = drawable.texture();
        // SAFETY: the drawable's texture is a 2D BGRA8 texture of the layer's drawable
        // size, created on the same MTLDevice as `device`; the drawable is retained
        // in `Frame` for as long as the wgpu texture is used.
        let texture = unsafe {
            let hal = wgpu::hal::metal::Device::texture_from_raw(
                raw,
                wgpu::TextureFormat::Bgra8Unorm,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent {
                    width: self.width,
                    height: self.height,
                    depth: 1,
                },
                None,
            );
            device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                hal,
                &wgpu::TextureDescriptor {
                    label: Some("drawable"),
                    size: wgpu::Extent3d {
                        width: self.width,
                        height: self.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::TextureUses::UNINITIALIZED,
            )
        };
        Some(Frame { drawable, texture })
    }

    /// Present `frame` (already submitted on `queue`) and record its presented time
    /// under index `n`.
    pub fn present(&self, queue: &wgpu::Queue, frame: Frame, n: u64) {
        let log = self.presented.clone();
        let block = RcBlock::new(move |d: NonNull<ProtocolObject<dyn MTLDrawable>>| {
            // SAFETY: the drawable is valid for the duration of the handler.
            let t = unsafe { d.as_ref() }.presentedTime();
            let ns = if t > 0.0 { (t * 1e9) as u64 } else { 0 };
            log.lock().unwrap().push((n, ns));
        });
        let drawable: &ProtocolObject<dyn MTLDrawable> = ProtocolObject::from_ref(&*frame.drawable);
        // SAFETY: the raw queue is the queue wgpu submitted the frame on, so this
        // command buffer is ordered after it; the block is copied by Metal.
        unsafe {
            drawable.addPresentedHandler(RcBlock::as_ptr(&block));
            let hal = queue.as_hal::<wgpu::hal::api::Metal>().expect("Metal queue");
            let cb = hal.as_raw().commandBuffer().expect("command buffer");
            cb.presentDrawable(drawable);
            cb.commit();
        }
        drop(frame.texture);
    }
}

/// The marker is only available on the main thread.
pub fn main_thread() -> Option<MainThreadMarker> {
    MainThreadMarker::new()
}
