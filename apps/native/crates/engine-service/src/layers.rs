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
        let _: () = msg_send![p,setWantsExtendedDynamicRangeContent:true];
        let _: () = msg_send![p,setMaximumDrawableCount:2_u64];
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
    };
    output.validate()?;
    Ok((
        output,
        screen.maximumPotentialExtendedDynamicRangeColorComponentValue(),
    ))
}
