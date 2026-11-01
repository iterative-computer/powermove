//! P-LAYER host adapted from S2; rect-only clips, translation-only slots.
use crate::{
    policy::Output,
    worker::{Host, Update},
};
use foreign_types::ForeignType;
use objc2::{
    MainThreadMarker, MainThreadOnly, class, define_class, msg_send, rc::Retained,
    runtime::AnyObject,
};
use objc2_app_kit::{NSScreen, NSView, NSWindow, NSWindowOrderingMode};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::RefCell, rc::Rc};
// Timecode and playhead are visual chrome. Their above-slot view must not
// swallow mouse events intended for the GPUI scrub strip underneath it.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMS3PassiveOverlay"]
    struct PassiveOverlay;
    impl PassiveOverlay {
        #[unsafe(method_id(hitTest:))]
        fn hit(&self, _: NSPoint) -> Option<Retained<NSView>> { None }
    }
);
pub struct Native {
    _pacer: crate::pacing::Pacer,
    pub playback_ready: bool,
    pub layer_updates: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub clock: Option<std::sync::mpsc::Receiver<crate::pacing::Tick>>,
    window: Retained<NSWindow>,
    view: Retained<NSView>,
    above: Retained<PassiveOverlay>,
    timecode: Retained<AnyObject>,
    playhead: Retained<AnyObject>,
    pub layer: metal::MetalLayer,
    pub host: Option<Host>,
    registration: Option<gpui_macos::composition::CompositionRegistration>,
    width: f64,
    height: f64,
    simulated: bool,
    last_output: Option<Output>,
}
pub fn screen_output(window: &NSWindow, width: u32, height: u32) -> Output {
    let screen = window
        .screen()
        .unwrap_or_else(|| NSScreen::mainScreen(MainThreadMarker::new().unwrap()).unwrap());
    let headroom = screen.maximumExtendedDynamicRangeColorComponentValue();
    let n = screen.deviceDescription();
    let key = NSString::from_str("NSScreenNumber");
    let display = unsafe {
        let number: *mut AnyObject = msg_send![&*n,objectForKey:&*key];
        let v: u32 = msg_send![number, unsignedIntValue];
        v
    };
    Output {
        width,
        height,
        display,
        headroom,
    }
}
impl Native {
    pub fn install(window: &gpui::Window, device: &metal::Device, hz: f64) -> Rc<RefCell<Self>> {
        let mtm = MainThreadMarker::new().unwrap();
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            panic!("macOS")
        };
        let gpui: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        let native = gpui.window().unwrap();
        let content = native.contentView().unwrap();
        let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        view.setWantsLayer(true);
        let layer = metal::MetalLayer::new();
        layer.set_device(device);
        layer.set_pixel_format(metal::MTLPixelFormat::RGBA16Float);
        layer.set_opaque(true);
        layer.set_framebuffer_only(false);
        layer.set_presents_with_transaction(false);
        unsafe {
            let p = layer.as_ptr() as *mut AnyObject;
            let _: () = msg_send![p,setWantsExtendedDynamicRangeContent:true];
            let _: () = msg_send![p,setMaximumDrawableCount:3_u64];
            let _: () = msg_send![p,setAllowsNextDrawableTimeout:true];
            let name = objc2_core_graphics::kCGColorSpaceExtendedLinearDisplayP3;
            let space = objc2_core_graphics::CGColorSpace::with_name(Some(name)).unwrap();
            let _: () = msg_send![p,setColorspace:&*space];
            let _: () = msg_send![&*view,setLayer:p];
        }
        content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(&gpui));
        let allocated = PassiveOverlay::alloc(mtm).set_ivars(());
        let above: Retained<PassiveOverlay> =
            unsafe { msg_send![super(allocated), initWithFrame:NSRect::ZERO] };
        above.setWantsLayer(true);
        content.addSubview_positioned_relativeTo(&above, NSWindowOrderingMode::Above, Some(&gpui));
        let (timecode, playhead) = unsafe {
            let t: *mut AnyObject = msg_send![class!(CATextLayer), new];
            let t = Retained::from_raw(t).unwrap();
            let p: *mut AnyObject = msg_send![class!(CALayer), new];
            let p = Retained::from_raw(p).unwrap();
            let _: () = msg_send![&*t,setFontSize:14_f64];
            let _: () = msg_send![&*t,setContentsScale:2_f64];
            let c = objc2_app_kit::NSColor::whiteColor().CGColor();
            let _: () = msg_send![&*p,setBackgroundColor:&*c];
            let _: () = msg_send![&*above.layer().unwrap(),addSublayer:&*t];
            let _: () = msg_send![&*above.layer().unwrap(),addSublayer:&*p];
            (t, p)
        };
        let (pacer, clock) = crate::pacing::Pacer::new(&view, hz);
        let this = Rc::new(RefCell::new(Self {
            _pacer: pacer,
            playback_ready: false,
            layer_updates: Default::default(),
            clock: Some(clock),
            window: native,
            view,
            above,
            timecode,
            playhead,
            layer,
            host: None,
            registration: None,
            width: 0.,
            height: 0.,
            simulated: false,
            last_output: None,
        }));
        let weak = Rc::downgrade(&this);
        let reg = gpui_macos::composition::register(
            &gpui,
            Rc::new(move || {
                if let Some(h) = weak.upgrade() {
                    h.borrow_mut().place();
                }
            }),
            Rc::new(|_| true),
        );
        this.borrow_mut().registration = Some(reg);
        this
    }
    pub fn publish(&mut self, w: f64, h: f64) {
        self.width = w;
        self.height = h;
        if let Some(h) = &self.host {
            // GPUI publishes locked geometry; it is consumed inside the engine pass.
            h.control.lock().unwrap().rect = [0.1, 0.1, 0.9, 0.9];
            h.renders.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
    }
    pub fn place(&mut self) {
        if let Some(host) = &self.host {
            host.presents
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
        let w = (self.width - 48.).max(32.);
        let h = (self.height - 150.).max(32.);
        self.view
            .setFrame(NSRect::new(NSPoint::new(24., 94.), NSSize::new(w, h)));
        self.view.layer().unwrap().setMasksToBounds(true);
        self.above
            .setFrame(NSRect::new(NSPoint::new(24., 30.), NSSize::new(w, 50.)));
        let scale = self.window.backingScaleFactor();
        self.layer
            .set_drawable_size(core_graphics_types::geometry::CGSize::new(
                (w * scale).round(),
                (h * scale).round(),
            ));
        self.layer.set_contents_scale(scale);
        self.poll_output();
    }
    pub fn poll_output(&mut self) {
        let size = self.layer.drawable_size();
        let mut output = screen_output(&self.window, size.width as u32, size.height as u32);
        if self.simulated {
            output.headroom = 1.;
            output.display = u32::MAX;
        }
        if self.last_output != Some(output) {
            eprintln!("S3 output {output:?}; previous {:?}", self.last_output);
            self.last_output = Some(output);
        }
        if let Some(host) = &self.host {
            host.control.lock().unwrap().policy.configure(output);
        }
    }
    pub fn update(&mut self, update: &Update) {
        self.playback_ready |= !update.finished;
        self.poll_output();
        self.layer_updates
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        unsafe {
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction),setDisableActions:true];
            let frames = (update.time * 120.).floor() as u64;
            let s = NSString::from_str(&format!(
                "00:{:02}:{:02}:{:03}",
                frames / 7200,
                frames / 120 % 60,
                frames % 120
            ));
            let _: () = msg_send![&*self.timecode,setString:&*s];
            let _: () = msg_send![&*self.timecode,setFrame:NSRect::new(NSPoint::new(0.,24.),NSSize::new(220.,24.))];
            let x = update.time / 2.5 * (self.width - 48.).max(1.);
            let _: () = msg_send![&*self.playhead,setFrame:NSRect::new(NSPoint::new(x,0.),NSSize::new(2.,20.))];
            let _: () = msg_send![class!(CATransaction), commit];
        }
    }
    pub fn simulate(&mut self) {
        self.simulated = !self.simulated;
        self.poll_output();
    }
    pub fn resize(&self) {
        let f = self.window.frame();
        self.window
            .setFrame_display(NSRect::new(f.origin, NSSize::new(720., 520.)), true);
    }
    pub fn output(&self) -> Output {
        self.last_output.unwrap()
    }
    pub fn seek(&mut self, time: f64) {
        self.seek_at(time, objc2_quartz_core::CACurrentMediaTime(), false);
    }
    fn seek_at(&mut self, time: f64, input: f64, appkit: bool) {
        if let Some(host) = &self.host {
            let mut c = host.control.lock().unwrap();
            let revision = c.policy.stamp.revision;
            let stamp = c.policy.request(revision);
            c.appkit_inputs += u64::from(appkit);
            c.seek = Some((time, input, stamp));
        }
    }
    pub fn seek_input(&mut self, x: f64) {
        let time = ((x - 24.) / (self.width - 48.)).clamp(0., 1.) * 2.5;
        // Capture the NSEvent origin BEFORE publishing the request. The worker
        // must never consume a request with a provisional enqueue timestamp.
        let app = objc2_app_kit::NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        let input = app.currentEvent().map_or_else(
            || objc2_quartz_core::CACurrentMediaTime(),
            |e| e.timestamp(),
        );
        self.seek_at(time, input, true);
    }
    pub fn identity(&self) {
        eprintln!(
            "S3 window_id={} displays={} max_hz={} current_headroom={} potential_headroom={} pixel_format={:?}",
            self.window.windowNumber(),
            NSScreen::screens(MainThreadMarker::new().unwrap()).len(),
            self.window.screen().unwrap().maximumFramesPerSecond(),
            self.window
                .screen()
                .unwrap()
                .maximumExtendedDynamicRangeColorComponentValue(),
            self.window
                .screen()
                .unwrap()
                .maximumPotentialExtendedDynamicRangeColorComponentValue(),
            self.layer.pixel_format()
        );
    }
}
