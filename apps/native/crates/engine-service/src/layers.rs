//! Main-thread CALayer adapters, independent of GPUI repaint.
use anyhow::{Result, ensure};
use foreign_types::ForeignTypeRef;
use objc2::{MainThreadMarker, class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// Configure a caller-owned below-slot. Resize/placement remains the shell's
/// CATransaction responsibility. Call Viewer::configure in that transaction.
pub fn configure_layer(
    _: MainThreadMarker,
    layer: &metal::MetalLayerRef,
    device: &metal::DeviceRef,
) -> Result<()> {
    layer.set_device(device);
    layer.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
    layer.set_opaque(true);
    layer.set_framebuffer_only(false);
    layer.set_presents_with_transaction(false);
    unsafe {
        let p = layer.as_ptr() as *mut AnyObject;
        // Diagnostics: PM_FIX_LAYER_EDR=0 asks for SDR composition;
        // PM_FIX_LAYER_DRAWABLES=N changes the drawable queue depth.
        let edr = pm_media::decode::diag::knob("PM_FIX_LAYER_EDR") != Some(0);
        let _: () = msg_send![p,setWantsExtendedDynamicRangeContent:edr];
        let drawables = pm_media::decode::diag::knob("PM_FIX_LAYER_DRAWABLES")
            .filter(|n| (2..=3).contains(n))
            .unwrap_or(3);
        let _: () = msg_send![p,setMaximumDrawableCount:drawables];
        let _: () = msg_send![p,setAllowsNextDrawableTimeout:true];
        let space = objc2_core_graphics::CGColorSpace::with_name(Some(
            objc2_core_graphics::kCGColorSpaceExtendedLinearDisplayP3,
        ))
        .ok_or_else(|| anyhow::anyhow!("extended-linear P3 unavailable"))?;
        let _: () = msg_send![p,setColorspace:&*space];
    }
    Ok(())
}
/// The shell attaches these to an above-slot CALayer. !Send: updates require
/// MainThreadMarker; no per-frame GPUI entity invalidation is needed.
pub struct PlaybackLayers {
    pub timecode: Retained<AnyObject>,
    pub playhead: Retained<AnyObject>,
    pub meters: [Retained<AnyObject>; 2],
    _main: MainThreadMarker,
}
impl PlaybackLayers {
    pub fn new(main: MainThreadMarker) -> Self {
        unsafe {
            let timecode = Retained::from_raw(msg_send![class!(CATextLayer), new]).unwrap();
            let playhead = Retained::from_raw(msg_send![class!(CALayer), new]).unwrap();
            let meters = std::array::from_fn(|_| {
                Retained::from_raw(msg_send![class!(CALayer), new]).unwrap()
            });
            let _: () = msg_send![&*timecode,setFontSize:14_f64];
            Self {
                timecode,
                playhead,
                meters,
                _main: main,
            }
        }
    }
    pub fn update(
        &self,
        _: MainThreadMarker,
        clock: &pm_protocol::clock::ClockSample,
        at: pm_protocol::common::HostMicros,
        rate: pm_time::Rate,
        fraction: f64,
        levels: [f64; 2],
    ) -> Result<()> {
        ensure!(
            fraction.is_finite() && levels.iter().all(|v| v.is_finite()),
            "invalid layer feed"
        );
        let ticks = clock.media_time_at(at).get();
        let label = pm_time::timecode::TimecodeFormat::for_rate(rate).format(rate.frame_of(ticks));
        unsafe {
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction),setDisableActions:true];
            let _: () = msg_send![&*self.timecode,setString:&*NSString::from_str(&label)];
            let _: () = msg_send![&*self.playhead,setFrame:NSRect::new(NSPoint::new(fraction,0.),NSSize::new(1.,1.))];
            for (layer, level) in self.meters.iter().zip(levels) {
                let _: () = msg_send![&**layer,setFrame:NSRect::new(NSPoint::ZERO,NSSize::new(1.,level.clamp(0.,1.)))];
            }
            let _: () = msg_send![class!(CATransaction), commit];
        }
        Ok(())
    }
}

/// Read on screen-change / EDR notifications; publishing a changed Output
/// retires all old-size/display/headroom submissions for that viewer.
pub fn screen_output(
    _: MainThreadMarker,
    screen: &objc2_app_kit::NSScreen,
    width: u32,
    height: u32,
) -> Result<(crate::Output, f64)> {
    let descriptions = screen.deviceDescription();
    let key = NSString::from_str("NSScreenNumber");
    let display = unsafe {
        let number: *mut AnyObject = msg_send![&*descriptions,objectForKey:&*key];
        let value: u32 = msg_send![number, unsignedIntValue];
        value
    };
    let output = crate::Output {
        width,
        height,
        display,
        headroom: screen.maximumExtendedDynamicRangeColorComponentValue(),
        crop: None,
    };
    output.validate()?;
    Ok((
        output,
        screen.maximumPotentialExtendedDynamicRangeColorComponentValue(),
    ))
}

use objc2::{DefinedClass, MainThreadOnly, define_class, sel};
use objc2_foundation::{NSObject, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};
use std::rc::Rc;
struct DisplayState {
    callback: Rc<dyn Fn(f64, f64)>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMEngineDisplayTarget"]
    #[ivars = DisplayState]
    struct DisplayTarget;
    impl DisplayTarget {
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            (self.ivars().callback)(link.targetTimestamp(), link.targetTimestamp() - link.timestamp());
        }
    }
);
/// View-bound phase source. Pause on stopped status; resume on command/frame
/// notification. Retain for the window lifetime; invalidates before target drop.
pub struct DisplayLink {
    link: Option<Retained<CADisplayLink>>,
    _target: Retained<DisplayTarget>,
}
impl DisplayLink {
    pub fn new(view: &objc2_app_kit::NSView, hz: f64, callback: Rc<dyn Fn(f64, f64)>) -> Self {
        let this = DisplayTarget::alloc(MainThreadMarker::new().unwrap())
            .set_ivars(DisplayState { callback });
        let target: Retained<DisplayTarget> = unsafe { msg_send![super(this), init] };
        let available: bool =
            unsafe { msg_send![view, respondsToSelector:sel!(displayLinkWithTarget:selector:)] };
        if !available {
            return Self {
                link: None,
                _target: target,
            };
        }
        let link = unsafe { view.displayLinkWithTarget_selector(&target, sel!(tick:)) };
        link.setPreferredFrameRateRange(CAFrameRateRange::new(hz as f32, hz as f32, hz as f32));
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
        }
        link.setPaused(true);
        Self {
            link: Some(link),
            _target: target,
        }
    }
    pub fn set_paused(&self, paused: bool) {
        if let Some(link) = &self.link {
            link.setPaused(paused);
        }
    }
}
impl Drop for DisplayLink {
    fn drop(&mut self) {
        if let Some(link) = &self.link {
            link.invalidate();
        }
    }
}

struct PlacementState {
    window: Retained<objc2_app_kit::NSWindow>,
    layer: metal::MetalLayer,
    publish: Rc<dyn Fn(crate::Output)>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMEnginePlacementObserver"]
    #[ivars = PlacementState]
    struct PlacementObserver;
    impl PlacementObserver {
        #[unsafe(method(screenChanged:))]
        fn screen_changed(&self, _: Option<&objc2_foundation::NSNotification>) { self.sync(); }
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(&self, _: Option<&NSString>, _: Option<&AnyObject>, _: Option<&AnyObject>, _: *mut std::ffi::c_void) {
            self.sync();
        }
    }
);
impl PlacementObserver {
    fn sync(&self) {
        let s = self.ivars();
        let size = s.layer.drawable_size();
        if let Some(screen) = s.window.screen()
            && let Ok((output, _)) = screen_output(
                MainThreadMarker::new().unwrap(),
                &screen,
                size.width.round().max(1.) as u32,
                size.height.round().max(1.) as u32,
            )
        {
            (s.publish)(output);
        }
    }
}
/// Synchronous KVO adapter for the shell-owned layer. drawableSize publication
/// calls Viewer::configure before the shell's CATransaction commits. It does
/// not move the layer or replace the shell's composition registration.
pub struct OutputBinding {
    observer: Retained<PlacementObserver>,
}
impl OutputBinding {
    pub fn new(
        window: &objc2_app_kit::NSWindow,
        layer: metal::MetalLayer,
        publish: Rc<dyn Fn(crate::Output)>,
    ) -> Self {
        let this =
            PlacementObserver::alloc(MainThreadMarker::new().unwrap()).set_ivars(PlacementState {
                window: unsafe {
                    Retained::retain(window as *const _ as *mut objc2_app_kit::NSWindow).unwrap()
                },
                layer,
                publish,
            });
        let observer: Retained<PlacementObserver> = unsafe { msg_send![super(this), init] };
        unsafe {
            let p = observer.ivars().layer.as_ptr() as *mut AnyObject;
            let _: () = msg_send![p, addObserver:&*observer, forKeyPath:&*NSString::from_str("drawableSize"), options:0_usize, context:std::ptr::null_mut::<std::ffi::c_void>()];
            for key in ["screen", "backingScaleFactor"] {
                let _: () = msg_send![window, addObserver:&*observer, forKeyPath:&*NSString::from_str(key), options:0_usize, context:std::ptr::null_mut::<std::ffi::c_void>()];
            }
        }
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter()
                .addObserver_selector_name_object(
                    &observer,
                    sel!(screenChanged:),
                    Some(&NSString::from_str(
                        "NSApplicationDidChangeScreenParametersNotification",
                    )),
                    None,
                );
            // NSWindow.screen is not documented as KVO-compliant: a window
            // dragged to another display (EDR headroom, display id) is only
            // reliably reported by these window notifications.
            for name in [
                "NSWindowDidChangeScreenNotification",
                "NSWindowDidChangeBackingPropertiesNotification",
            ] {
                objc2_foundation::NSNotificationCenter::defaultCenter()
                    .addObserver_selector_name_object(
                        &observer,
                        sel!(screenChanged:),
                        Some(&NSString::from_str(name)),
                        Some(AsRef::<AnyObject>::as_ref(window)),
                    );
            }
        }
        Self { observer }
    }
    pub fn sync(&self) {
        self.observer.sync();
    }
}
impl Drop for OutputBinding {
    fn drop(&mut self) {
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter().removeObserver(&self.observer);
            let s = self.observer.ivars();
            let _: () = msg_send![s.layer.as_ptr() as *mut AnyObject, removeObserver:&*self.observer, forKeyPath:&*NSString::from_str("drawableSize")];
            for key in ["screen", "backingScaleFactor"] {
                let _: () = msg_send![&*s.window, removeObserver:&*self.observer, forKeyPath:&*NSString::from_str(key)];
            }
        }
    }
}

pub(crate) fn mastering_space(
    _: MainThreadMarker,
    layer: &metal::MetalLayerRef,
    mode: crate::color::Mastering,
) -> Result<()> {
    let name = unsafe {
        match mode {
            crate::color::Mastering::LinearP3 | crate::color::Mastering::Edr => {
                objc2_core_graphics::kCGColorSpaceExtendedLinearDisplayP3
            }
            crate::color::Mastering::Pq { .. } => objc2_core_graphics::kCGColorSpaceITUR_2100_PQ,
            crate::color::Mastering::Hlg { .. } => objc2_core_graphics::kCGColorSpaceITUR_2100_HLG,
        }
    };
    let space = objc2_core_graphics::CGColorSpace::with_name(Some(name))
        .ok_or_else(|| anyhow::anyhow!("mastering color space unavailable"))?;
    unsafe {
        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions:true];
        let _: () = msg_send![layer.as_ptr() as *mut AnyObject, setColorspace:&*space];
        let _: () = msg_send![class!(CATransaction), commit];
    }
    Ok(())
}
