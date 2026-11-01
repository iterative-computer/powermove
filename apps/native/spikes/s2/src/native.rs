//! AppKit slot host. Placement runs only from the patched GPUI present callback.
use crate::contract::{Band, Clip, Frame, Rect, Slot, Target, Transform};
use foreign_types::{ForeignType, ForeignTypeRef};
use metal::{MTLClearColor, MTLLoadAction, MTLPixelFormat, MTLStoreAction};
use objc2::{MainThreadMarker, MainThreadOnly, class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSTextField, NSView, NSWindow, NSWindowOrderingMode,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::RefCell, rc::Rc};

pub struct Host {
    pub frame: Frame,
    content: Retained<NSView>,
    gpui: Retained<NSView>,
    window: Retained<NSWindow>,
    viewer: Retained<crate::slot_view::SlotView>,
    web: Retained<crate::slot_view::SlotView>,
    field: Retained<NSTextField>,
    layer: metal::MetalLayer,
    queue: metal::CommandQueue,
    pattern: metal::RenderPipelineState,
    menu: Retained<NSWindow>,
    menu_open: bool,
    registration: Option<gpui_macos::composition::CompositionRegistration>,
}
fn nsrect(r: Rect, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(r.x, h - r.y - r.h), NSSize::new(r.w, r.h))
}
fn slot(id: u64, r: Rect, band: Band) -> Slot {
    Slot {
        id,
        band,
        bounds: Rect::new(0., 0., r.w, r.h),
        clip: Clip::Rect(Rect::new(0., 0., r.w, r.h)),
        transform: Transform::translation(r.x, r.y),
    }
}
impl Host {
    pub fn install(window: &gpui::Window, device: &metal::Device) -> Rc<RefCell<Self>> {
        let mtm = MainThreadMarker::new().unwrap();
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            panic!("macOS only")
        };
        let gpui: Retained<NSView> =
            unsafe { Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        let native_window = gpui.window().unwrap();
        let content = native_window.contentView().unwrap();
        let viewer = crate::slot_view::SlotView::new(mtm);
        viewer.setWantsLayer(true);
        let layer = metal::MetalLayer::new();
        layer.set_device(device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        layer.set_opaque(true);
        layer.set_presents_with_transaction(true);
        unsafe {
            let _: () = msg_send![&*viewer,setLayer:layer.as_ptr() as *mut AnyObject];
        }
        content.addSubview_positioned_relativeTo(&viewer, NSWindowOrderingMode::Below, Some(&gpui));
        // No navigation, network, project data, or scripts loaded from disk.
        let web_content: Retained<NSView> = unsafe {
            let p: *mut NSView = msg_send![class!(WKWebView), alloc];
            let p: *mut NSView =
                msg_send![p,initWithFrame:NSRect::new(NSPoint::ZERO,NSSize::new(280.,400.))];
            Retained::from_raw(p).unwrap()
        };
        let html = NSString::from_str(
            "<meta name='color-scheme' content='dark'><style>body{background:transparent;color:#eee;font:14px -apple-system;margin:0}#panel{position:fixed;right:12px;top:64px;width:248px;padding:16px;height:calc(100vh - 284px);background:#23252a}button,input{font:inherit}#overlay{position:fixed;right:12px;top:304px;width:320px;height:60px;background:#4a3e75}input{width:90%}</style><section id='panel'><h3>Compat panel</h3><input placeholder='WebKit text input'><p>Resize to reflow this native WKWebView.</p><button onclick='document.querySelector(\"#overlay\").hidden=!document.querySelector(\"#overlay\").hidden'>Toggle overlay</button></section><div id='overlay'>Compat overlay hole<br><button onclick='this.textContent=\"Clicked\"'>Click inside WebKit</button></div>",
        );
        unsafe {
            let _: *mut AnyObject = msg_send![&*web_content,loadHTMLString:&*html,baseURL:std::ptr::null::<AnyObject>()];
        }
        web_content.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let web = crate::slot_view::SlotView::new(mtm);
        // Autoresizing is relative to the parent's initial size. A zero-size
        // wrapper offsets fixed DOM content outside the visible clip.
        web.setFrame(web_content.frame());
        web.addSubview(&web_content);
        content.addSubview_positioned_relativeTo(&web, NSWindowOrderingMode::Below, Some(&gpui));
        let field = NSTextField::initWithFrame(NSTextField::alloc(mtm), NSRect::ZERO);
        field.setPlaceholderString(Some(&NSString::from_str("Native name · IME / Services")));
        field.setEditable(true);
        field.setSelectable(true);
        content.addSubview_positioned_relativeTo(&field, NSWindowOrderingMode::Above, Some(&gpui));
        let menu = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::ZERO, NSSize::new(300., 230.)),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe {
            menu.setReleasedWhenClosed(false);
        }
        menu.setOpaque(false);
        menu.setBackgroundColor(Some(&NSColor::clearColor()));
        menu.setHasShadow(true);
        let menu_content = material(mtm, NSRect::new(NSPoint::ZERO, NSSize::new(300., 230.)));
        let search = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(12., 184.), NSSize::new(276., 28.)),
        );
        search.setPlaceholderString(Some(&NSString::from_str("Search fields…")));
        material_contents(&menu_content).addSubview(&search);
        for (i, name) in ["Position", "Scale", "Rotation", "Opacity"]
            .iter()
            .enumerate()
        {
            let label = NSTextField::labelWithString(&NSString::from_str(name), mtm);
            label.setFrame(NSRect::new(
                NSPoint::new(22., 148. - i as f64 * 32.),
                NSSize::new(250., 26.),
            ));
            material_contents(&menu_content).addSubview(&label);
        }
        menu.setContentView(Some(&menu_content));
        let library=device.new_library_with_source("#include <metal_stdlib>\nusing namespace metal;\nvertex float4 s2v(uint i [[vertex_id]]) {float2 p[3]={float2(-1,-1),float2(3,-1),float2(-1,3)};return float4(p[i],0,1);}\nfragment float4 s2f(float4 p [[position]]) {float c=fmod(floor(p.x/48)+floor(p.y/48),2.0);return float4(0.06+c*0.025,0.12+c*0.035,0.19+c*0.045,1);}",&metal::CompileOptions::new()).expect("viewer pattern shader");
        let descriptor = metal::RenderPipelineDescriptor::new();
        descriptor.set_vertex_function(Some(&library.get_function("s2v", None).unwrap()));
        descriptor.set_fragment_function(Some(&library.get_function("s2f", None).unwrap()));
        descriptor
            .color_attachments()
            .object_at(0)
            .unwrap()
            .set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        let pattern = device.new_render_pipeline_state(&descriptor).unwrap();
        let host = Rc::new(RefCell::new(Self {
            frame: Frame::default(),
            content,
            gpui: gpui.clone(),
            window: native_window,
            viewer,
            web,
            field,
            layer,
            queue: device.new_command_queue(),
            pattern,
            menu,
            menu_open: false,
            registration: None,
        }));
        let weak = Rc::downgrade(&host);
        let hit = weak.clone();
        let reg = gpui_macos::composition::register(
            &gpui,
            Rc::new(move || {
                if let Some(h) = weak.upgrade() {
                    h.borrow_mut().present();
                }
            }),
            Rc::new(move |p| {
                hit.upgrade()
                    .is_none_or(|h| h.borrow().frame.hit([p.x, p.y]) == Target::Gpui)
            }),
        );
        host.borrow_mut().registration = Some(reg);
        host
    }
    pub fn publish(&mut self, w: f64, h: f64) {
        let top = crate::glass::toolbar_inset().max(52.);
        let right = Rect::new(
            (w - 292.).max(300.),
            top + 12.,
            280.,
            (h - top - 220.).max(100.),
        );
        let viewer = Rect::new(
            204.,
            top + 12.,
            (w - 510.).max(80.),
            (h - top - 220.).max(100.),
        );
        let field = Rect::new(214., top + 22., 250., 26.);
        let transport = Rect::new((w / 2. - 110.).max(205.), h - 246., 220., 44.);
        let timeline = Rect::new(0., h - 190., w, 190.);
        // Publication is atomic: no visible intermediate list while laying out.
        self.frame = Frame {
            revision: self.frame.revision + 1,
            slots: vec![
                slot(1, viewer, Band::Below),
                Slot {
                    id: 2,
                    band: Band::Below,
                    bounds: Rect::new(0., 0., w, h),
                    clip: Clip::Union(vec![right, Rect::new(right.x - 40., 304., 320., 60.)]),
                    transform: Transform::translation(0., 0.),
                },
                slot(3, field, Band::Above),
                slot(4, transport, Band::Above),
            ],
            interactive: vec![Rect::new(0., top, 195., h - top), timeline],
            occluding: vec![Rect::new(0., 0., w, top)],
            // Fixed DOM rectangle in this synthetic fixture; S5 must publish actual rectangles.
            compat_overlays: vec![Rect::new(right.x - 40., 304., 320., 60.)],
            children: vec![],
            accessibility: vec![
                Target::Gpui,
                Target::Slot(1),
                Target::Slot(3),
                Target::Slot(2),
                Target::Slot(4),
            ],
        };
    }
    fn present(&mut self) {
        let height = self.content.bounds().size.height;
        for s in &self.frame.slots {
            let [_, _, _, _, x, y] = s.transform.0;
            let r = Rect::new(x, y, s.bounds.w, s.bounds.h);
            match s.id {
                1 => {
                    self.viewer.setFrame(nsrect(r, height));
                    self.viewer.clip(&s.clip);
                }
                2 => {
                    self.web.setFrame(nsrect(r, height));
                    self.web.clip(&s.clip);
                }
                3 => self.field.setFrame(nsrect(r, height)),
                _ => {}
            }
        }
        let r = self.frame.slots.iter().find(|s| s.id == 4).unwrap();
        let [_, _, _, _, x, y] = r.transform.0;
        crate::glass::place(
            "transport",
            crate::glass::Rect {
                x,
                y,
                w: r.bounds.w,
                h: r.bounds.h,
            },
            22.,
            crate::glass::Layer::Floating,
            crate::glass::Contents::Buttons(&[
                (
                    "backward.end.fill",
                    "Previous",
                    crate::glass::Action::ZoomOut,
                ),
                ("play.fill", "Play", crate::glass::Action::PlayPause),
                (
                    "slider.horizontal.3",
                    "Field menu",
                    crate::glass::Action::ZoomIn,
                ),
            ]),
        );
        crate::glass::finish_frame();
        let scale = self.window.backingScaleFactor();
        let size = self.viewer.bounds().size;
        self.layer
            .set_drawable_size(core_graphics::geometry::CGSize::new(
                (size.width * scale).max(1.),
                (size.height * scale).max(1.),
            ));
        if let Some(d) = self.layer.next_drawable() {
            if std::env::var_os("S2_PACING").is_some() {
                let handler = block::ConcreteBlock::new(|drawable: &metal::DrawableRef| {
                    println!("S2_VIEWER_PRESENT {:.9}", drawable.presented_time());
                })
                .copy();
                d.add_presented_handler(&handler);
            }
            let pass = metal::RenderPassDescriptor::new();
            let c = pass.color_attachments().object_at(0).unwrap();
            c.set_texture(Some(d.texture()));
            c.set_load_action(MTLLoadAction::Clear);
            c.set_store_action(MTLStoreAction::Store);
            c.set_clear_color(MTLClearColor::new(0.055, 0.11, 0.18, 1.));
            let cmd = self.queue.new_command_buffer();
            let enc = cmd.new_render_command_encoder(pass);
            enc.set_render_pipeline_state(&self.pattern);
            enc.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 3);
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_scheduled();
            d.present();
        }
    }
    pub fn toggle_menu(&mut self) {
        unsafe {
            let _: () = msg_send![class!(NSObject),cancelPreviousPerformRequestsWithTarget:&*self.menu,selector:objc2::sel!(orderOut:),object:std::ptr::null::<AnyObject>()];
        }
        if self.menu_open {
            let duration = animate_menu(&self.menu, false);
            unsafe {
                let _: () = msg_send![&*self.menu,performSelector:objc2::sel!(orderOut:),withObject:std::ptr::null::<AnyObject>(),afterDelay:duration];
            }
        } else {
            let parent = self.window.frame();
            self.menu.setFrameOrigin(NSPoint::new(
                parent.origin.x + parent.size.width / 2. - 150.,
                parent.origin.y + 210.,
            ));
            unsafe {
                self.window
                    .addChildWindow_ordered(&self.menu, NSWindowOrderingMode::Above);
            }
            self.menu.orderFront(None);
            animate_menu(&self.menu, true);
        }
        self.menu_open = !self.menu_open;
    }
}
/// AppKit material, falling back below macOS 26. Adapted from the prior glass crate.
pub fn material(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSView> {
    if objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
    {
        let view = objc2_app_kit::NSBox::initWithFrame(objc2_app_kit::NSBox::alloc(mtm), frame);
        view.setBoxType(objc2_app_kit::NSBoxType::Custom);
        view.setFillColor(&NSColor::windowBackgroundColor());
        view.setBorderWidth(
            if objc2_app_kit::NSWorkspace::sharedWorkspace()
                .accessibilityDisplayShouldIncreaseContrast()
            {
                2.
            } else {
                1.
            },
        );
        Retained::into_super(view)
    } else if objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some() {
        let view = objc2_app_kit::NSGlassEffectView::initWithFrame(
            objc2_app_kit::NSGlassEffectView::alloc(mtm),
            frame,
        );
        view.setCornerRadius(16.);
        let content = NSView::initWithFrame(NSView::alloc(mtm), frame);
        content.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        view.setContentView(Some(&content));
        Retained::into_super(view)
    } else {
        let view = objc2_app_kit::NSVisualEffectView::initWithFrame(
            objc2_app_kit::NSVisualEffectView::alloc(mtm),
            frame,
        );
        view.setMaterial(objc2_app_kit::NSVisualEffectMaterial::Popover);
        view.setState(objc2_app_kit::NSVisualEffectState::Active);
        Retained::into_super(view)
    }
}
pub fn material_contents(view: &NSView) -> Retained<NSView> {
    unsafe {
        let glass = objc2::runtime::AnyClass::get(c"NSGlassEffectView")
            .is_some_and(|class| msg_send![view, isKindOfClass:class]);
        if glass {
            msg_send![view, contentView]
        } else {
            Retained::retain(view as *const NSView as *mut NSView).unwrap()
        }
    }
}
fn animate_menu(window: &NSWindow, enter: bool) -> f64 {
    let reduce =
        objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
    let motion = crate::contract::menu_motion(reduce);
    let duration = if enter { motion.enter } else { motion.exit };
    if let Some(v) = window.contentView() {
        v.setWantsLayer(true);
        if let Some(layer) = v.layer() {
            unsafe {
                let presentation = layer.presentationLayer();
                let from_alpha = presentation
                    .as_ref()
                    .map_or(if enter { 0. } else { 1. }, |p| p.opacity() as f64);
                let animation: *mut AnyObject = msg_send![class!(CABasicAnimation),animationWithKeyPath:&*NSString::from_str("opacity")];
                let from: *mut AnyObject = msg_send![class!(NSNumber),numberWithDouble:from_alpha];
                let to: *mut AnyObject =
                    msg_send![class!(NSNumber),numberWithDouble:if enter{1.}else{0.}];
                let _: () = msg_send![animation,setFromValue:from];
                let _: () = msg_send![animation,setToValue:to];
                let _: () = msg_send![animation,setDuration:duration];
                layer.setOpacity(if enter { 1. } else { 0. });
                let _: () = msg_send![&*layer,addAnimation:animation,forKey:&*NSString::from_str("pm.menu.opacity")];
                if enter && !reduce {
                    let allocated: *mut AnyObject = msg_send![class!(CASpringAnimation), alloc];
                    let spring: *mut AnyObject = msg_send![allocated,initWithPerceptualDuration:motion.enter,bounce:motion.bounce];
                    let spring = Retained::<AnyObject>::from_raw(spring).unwrap();
                    let key = NSString::from_str("transform.scale");
                    let _: () = msg_send![&*spring,setKeyPath:&*key];
                    let current: *mut AnyObject = if let Some(p) = &presentation {
                        msg_send![&**p,valueForKeyPath:&*key]
                    } else {
                        std::ptr::null_mut()
                    };
                    let from: *mut AnyObject = if current.is_null() {
                        msg_send![class!(NSNumber),numberWithDouble:motion.start_scale]
                    } else {
                        current
                    };
                    let to: *mut AnyObject = msg_send![class!(NSNumber),numberWithDouble:1.];
                    let _: () = msg_send![&*spring,setFromValue:from];
                    let _: () = msg_send![&*spring,setToValue:to];
                    let settling: f64 = msg_send![&*spring, settlingDuration];
                    println!(
                        "S2_MENU enter={} perceptual_s={} bounce={} settling_s={}",
                        enter, motion.enter, motion.bounce, settling
                    );
                    let _: () = msg_send![&*spring,setDuration:settling];
                    let _: () = msg_send![&*layer,addAnimation:&*spring,forKey:&*NSString::from_str("pm.menu.scale")];
                }
            }
        }
    }
    duration
}
#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}
