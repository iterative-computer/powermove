//! Bounded synthetic proof in a real GPUI P-LAYER slot. Screenshots stay in /tmp.
use gpui::{prelude::*, *};
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
use objc2_foundation::{NSPoint, NSString, NSTimer};
use pm_compat_host::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

/// The in-loop 35 s check runs on the AppKit timer, so a stalled main thread
/// or a run loop stuck in another mode would skip it. This thread enforces
/// the bound independently and keeps a stack sample of the stall in /tmp.
const LIMIT: Duration = Duration::from_secs(35);
const DONE: u32 = u32::MAX;
fn watchdog(stage: Arc<AtomicU32>) {
    std::thread::spawn(move || {
        std::thread::sleep(LIMIT + Duration::from_secs(10));
        let at = stage.load(Ordering::Acquire);
        if at == DONE {
            return;
        }
        println!("SMOKE watchdog_timeout stage={at}: main thread missed the 35 s bound");
        let _ = std::fs::create_dir_all("/tmp/pm-shots");
        let _ = std::process::Command::new("/usr/bin/sample")
            .args([
                &std::process::id().to_string(),
                "2",
                "-file",
                "/tmp/pm-shots/compat-host-stall.txt",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        std::process::exit(3);
    });
}

#[derive(Default)]
struct Inbox(RefCell<Vec<HostEvent>>, Cell<bool>);
impl SyncHandler for Inbox {
    fn call(&self, _: &RequestContext, method: &str, args: Value) -> pm_compat_host::Result<Value> {
        if method == "echo" {
            if args["loop"] == true {
                self.1.set(true);
                println!("SMOKE real_infinite_loop_started");
            }
            Ok(args)
        } else {
            Err(pm_compat_host::Error::Forbidden)
        }
    }
    fn event(&self, event: HostEvent) {
        self.0.borrow_mut().push(event);
    }
    fn prepare_reset(&self, generation: u64) -> pm_compat_host::Result<()> {
        println!("SMOKE sealed_generation={generation}");
        Ok(())
    }
}
struct Demo {
    host: Rc<RefCell<MacDomHost>>,
    gpui: Retained<NSView>,
    registration: Option<gpui_macos::composition::CompositionRegistration>,
    timer: Retained<NSTimer>,
    done: Rc<Cell<bool>>,
}
impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.done.get() {
            self.timer.invalidate();
            self.registration.take();
            cx.quit();
        }
        let weak = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = weak.update(cx, |_, cx| cx.notify());
        });
        window.request_animation_frame();
        let host = self.host.clone();
        let gpui = self.gpui.clone();
        div()
            .size_full()
            .text_color(rgb(0xeeeeee))
            .font_family(".AppleSystemUIFont")
            .child(
                canvas(
                    |_, _, _| (),
                    move |b, _, window, _| {
                        let w = f32::from(b.size.width) as f64;
                        let h = f32::from(b.size.height) as f64;
                        // Geometry is staged by the shell, then committed at presentation.
                        host.borrow_mut()
                            .attach(
                                1,
                                PanelSlot {
                                    id: 10,
                                    bounds: Rect::new(w - 330., 70., 300., h - 150.),
                                    clip: Rect::new(w - 330., 70., 300., h - 150.),
                                    reading_order: 1,
                                },
                            )
                            .unwrap();
                        let _ = &gpui;
                        // The page sits below GPUI: a compat overlay is visible
                        // only where the shell leaves its hole transparent.
                        let holes = host.borrow().composition().overlay_holes;
                        let mut quads = vec![
                            (Rect::new(0., 0., w, 60.), 0x25262a),
                            (Rect::new(0., 60., 180., h - 60.), 0x202125),
                            (Rect::new(180., h - 120., w - 180., 120.), 0x292a2e),
                        ];
                        for r in holes.iter().fold(
                            vec![Rect::new(180., 60., w - 510., h - 180.)],
                            |rs, hole| rs.into_iter().flat_map(|r| subtract(r, hole)).collect(),
                        ) {
                            quads.push((r, 0x111216));
                        }
                        for (r, color) in quads {
                            let mut quad = PaintQuad::new(Bounds::new(
                                point(px(r.x as f32), px(r.y as f32)),
                                size(px(r.w as f32), px(r.h as f32)),
                            ));
                            quad.background = rgb(color).into();
                            window.paint_quad(quad);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top(px(18.))
                    .left(px(24.))
                    .child("Powermove Native · Compat host smoke"),
            )
            .child(
                div()
                    .absolute()
                    .top(px(90.))
                    .left(px(24.))
                    .child("Native shell"),
            )
            .child(
                div()
                    .absolute()
                    .bottom(px(50.))
                    .left(px(220.))
                    .child("Native timeline · WebKit clips to panels and overlays"),
            )
    }
}
/// `r` minus `hole`, as up to four non-overlapping rectangles.
fn subtract(r: Rect, hole: &Rect) -> Vec<Rect> {
    let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.w, r.y + r.h);
    let (hx0, hy0) = (hole.x.max(x0), hole.y.max(y0));
    let (hx1, hy1) = ((hole.x + hole.w).min(x1), (hole.y + hole.h).min(y1));
    if hx0 >= hx1 || hy0 >= hy1 {
        return vec![r];
    }
    [
        Rect::new(x0, y0, r.w, hy0 - y0),
        Rect::new(x0, hy1, r.w, y1 - hy1),
        Rect::new(x0, hy0, hx0 - x0, hy1 - hy0),
        Rect::new(hx1, hy0, x1 - hx1, hy1 - hy0),
    ]
    .into_iter()
    .filter(|q| q.w > 0. && q.h > 0.)
    .collect()
}
fn fixture() -> Value {
    json!([
        ["create", 1, "h2"],
        ["text", 1, "Remote DOM panel"],
        ["append", 0, 1],
        [
            "attr",
            0,
            "style",
            "background:#25262a;color:#eeeeee;padding:16px;box-sizing:border-box"
        ],
        ["create", 2, "input"],
        ["attr", 2, "aria-label", "Compat name"],
        ["attr", 2, "placeholder", "Type here"],
        ["attr", 2, "style", "font:16px -apple-system;width:240px"],
        ["attr", 2, "spellcheck", "false"],
        ["attr", 2, "autocorrect", "off"],
        ["attr", 2, "autocapitalize", "off"],
        ["attr", 2, "autocomplete", "off"],
        ["append", 0, 2],
        ["create", 3, "output"],
        ["text", 3, "Waiting for input"],
        ["attr", 3, "style", "display:block;margin:16px 0"],
        ["append", 0, 3],
        ["create", 4, "button"],
        ["text", 4, "Open popout"],
        ["attr", 4, "data-pm-popout", "true"],
        [
            "attr",
            4,
            "style",
            "position:absolute;left:16px;top:200px;font:16px -apple-system;height:32px"
        ],
        ["append", 0, 4],
        ["create", 5, "div"],
        ["attr", 5, "data-pm-overlay", "true"],
        [
            "attr",
            5,
            "style",
            "position:fixed;left:430px;top:310px;width:280px;height:80px;background:#594683;color:white;padding:12px;box-sizing:border-box"
        ],
        ["append", 0, 5],
        ["create", 6, "button"],
        ["text", 6, "Overlay hole · click"],
        ["attr", 6, "style", "font:16px -apple-system"],
        ["append", 5, 6]
    ])
}
fn key(window: &objc2_app_kit::NSWindow, text: &str, code: u16) {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
        let event=NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(kind,NSPoint::ZERO,NSEventModifierFlags::empty(),0.,window.windowNumber(),None,&NSString::from_str(text),&NSString::from_str(text),false,code).unwrap();
        app.sendEvent(&event);
    }
}
fn click(window: &objc2_app_kit::NSWindow, x: f64, y: f64) {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
        let event=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(kind,NSPoint::new(x,window.contentView().unwrap().bounds().size.height-y),NSEventModifierFlags::empty(),0.,window.windowNumber(),None,1,1,1.).unwrap();
        app.sendEvent(&event);
    }
}
fn shot(window: &objc2_app_kit::NSWindow, name: &str) {
    let _ = std::fs::create_dir_all("/tmp/pm-shots");
    let result = std::process::Command::new("/usr/sbin/screencapture")
        .args([
            "-x",
            "-o",
            "-l",
            &window.windowNumber().to_string(),
            &format!("/tmp/pm-shots/compat-host-{name}.png"),
        ])
        .status();
    assert!(result.is_ok_and(|s| s.success()), "window capture failed");
}
fn main() {
    let device = metal::Device::system_default().expect("Metal");
    let platform = Rc::new(gpui_macos::MacPlatform::new_with_metal_device(
        false, device,
    ));
    let progress = Arc::new(AtomicU32::new(0));
    watchdog(progress.clone());
    let reached = progress.clone();
    Application::with_platform(platform).with_accessibility_forced().run(move|cx|{
        cx.set_window_appearance(Some(WindowAppearance::Dark));
        cx.open_window(WindowOptions::new().macos_window_background(MacosWindowBackground::Transparent).window_bounds(Some(WindowBounds::Windowed(Bounds::new(point(px(100.),px(100.)),size(px(1000.),px(680.)))))),move|window,cx|{
            window.set_window_title("Powermove Native · Compat host smoke");
            let mtm=MainThreadMarker::new().unwrap();
            let RawWindowHandle::AppKit(handle)=window.window_handle().unwrap().as_raw() else{panic!("Mac only")};
            let native:Retained<NSView>=unsafe{Retained::retain(handle.ns_view.as_ptr().cast()).unwrap()};
            let w=native.window().unwrap();let content=w.contentView().unwrap();
            let inbox=Rc::new(Inbox::default());let mut stores=StoreDataStores::default();
            let mut host=MacDomHost::new(mtm,Trust::Trusted,inbox.clone(),&mut stores,BTreeMap::new()).unwrap();
            host.create(PanelSpec{id:1,label:"Compat panel".into()}).unwrap();host.install_below(&content,&native);
            let host=Rc::new(RefCell::new(host));let h=host.clone();let view=native.clone();
            let accepts=host.clone();
            let registration=gpui_macos::composition::register(&native,Rc::new(move||{
                let bounds=view.bounds();h.borrow_mut().present(Rect::new(0.,0.,bounds.size.width,bounds.size.height)).unwrap();
                gpui_macos::composition::publish_accessibility(&view,vec![gpui_macos::composition::AccessibilityItem::RemainingGpui,h.borrow().accessibility_item()]);
            }),Rc::new(move|p|{accepts.borrow().composition().accepts_gpui(p.x,p.y,&[Rect::new(0.,0.,1000.,60.),Rect::new(0.,60.,180.,620.),Rect::new(180.,560.,820.,120.)])}));
            let done=Rc::new(Cell::new(false));let finish=done.clone();let timer_host=host.clone();
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);
            w.makeKeyAndOrderFront(None);
            #[allow(deprecated)] app.activateIgnoringOtherApps(true);
            // Test-only fault injection for the bounds themselves:
            // `stall` blocks the main thread (watchdog, exit 3);
            // `stuck` never advances past typing (driver timeout, exit 2).
            let fault=std::env::var("PM_COMPAT_SMOKE_FAULT").unwrap_or_default();
            let started=Instant::now();let mut stage=0u32;let mut phase=Instant::now();let mut typed=false;let mut overlay=false;let mut opened=false;let mut recovered=false;let mut input_value=String::new();
            let driver=Rc::new(RefCell::new(move||{
                let mut h=timer_host.borrow_mut();h.tick();
                for e in inbox.0.take(){match e {
                    HostEvent::Ready{generation}=>{h.apply(generation,1,fixture()).unwrap();h.focus(1,Some(2)).unwrap();println!("SMOKE ready_generation={generation}");},
                    HostEvent::Dom{payload,..}=>{
                        if payload["type"]!="input" {println!("SMOKE dom type={} node={}",payload["type"],payload["node"]);}
                        if payload["type"]=="input" {input_value=payload["value"].as_str().unwrap().into();let g=h.generation();h.apply(g,1,json!([["text",3,format!("Value: {input_value}")]])).unwrap();typed=input_value=="powermove";}
                        if payload["type"]=="click" && payload["node"]==6 {overlay=true;let g=h.generation();h.apply(g,1,json!([["text",6,"Overlay clicked"]])).unwrap();}
                    },HostEvent::PopoutOpened=>{opened=true;println!("SMOKE popout_created");},
                    HostEvent::Recovered{generation,..}=>{recovered=true;println!("SMOKE recovery_generation={generation}");},
                    HostEvent::Failure(code)=>{println!("SMOKE failure={code}");shot(&w,"failure");std::process::exit(2);},_=>{}
                }}
                progress.store(stage,Ordering::Release);
                if started.elapsed()>LIMIT{println!("SMOKE timeout stage={stage} typed={typed} overlay={overlay} popout={opened} recovered={recovered}");shot(&w,"failure");std::process::exit(2)}
                if finish.get(){return}
                match stage {
                    0 if phase.elapsed()>Duration::from_secs(2) && w.isKeyWindow() && NSApplication::sharedApplication(mtm).isActive()=>{println!("SMOKE native_foreground active={} key={} visible={}",NSApplication::sharedApplication(mtm).isActive(),w.isKeyWindow(),w.isVisible());
                        assert!(w.isKeyWindow());assert!(NSApplication::sharedApplication(mtm).isActive());println!("SMOKE foreground=true responder={}",h.owns_first_responder());drop(h);for (c,code) in [("p",35),("o",31),("w",13),("e",14),("r",15),("m",46),("o",31),("v",9),("e",14)]{key(&w,c,code);}stage=1;phase=Instant::now();},
                    1 if fault=="stall" => {println!("SMOKE fault=stall: blocking the main thread");std::thread::sleep(Duration::from_secs(120));},
                    1 if fault=="stuck" => {},
                    1 if typed && phase.elapsed()>Duration::from_millis(500) && !h.composition().overlay_holes.is_empty()=>{println!("SMOKE typed={input_value} overlay_rects={}",h.composition().overlay_holes.len());shot(&w,"typing");drop(h);click(&w,500.,333.);stage=2;phase=Instant::now();},
                    2 if overlay && phase.elapsed()>Duration::from_millis(500)=>{println!("SMOKE overlay_hit=true");shot(&w,"overlay");drop(h);click(&w,750.,286.);stage=3;phase=Instant::now();},
                    3 if opened && phase.elapsed()>Duration::from_secs(1)=>{println!("SMOKE popout_count={}",h.popout_count());shot(&w,"popout-parent");if let Some(children)=w.childWindows(){for child in children.to_vec(){shot(&child,"popout");}}
                        // A deliberately requested real infinite-loop fixture. This
                        // one eval is test injection only, never host transport.
                        unsafe{h.web_view().evaluateJavaScript_completionHandler(&NSString::from_str("window.pmCompatHost.call(1,'echo',{loop:true});while(true){}"),None);}stage=4;phase=Instant::now();},
                    4 if inbox.1.get() && phase.elapsed()>Duration::from_secs(1)=>{h.stop_and_recover().unwrap();stage=5;phase=Instant::now();},
                    5 if recovered && phase.elapsed()>Duration::from_secs(1)=>{progress.store(DONE,Ordering::Release);shot(&w,"recovered");h.begin_background_job(false).unwrap();h.begin_background_job(false).unwrap();h.end_background_job();h.end_background_job();println!("SMOKE PASS typing overlay popout recovery activity");finish.set(true);},_=>{}
                }
            }));
            let block=block2::RcBlock::new(move|_:std::ptr::NonNull<NSTimer>|(driver.borrow_mut())());
            let timer=unsafe{NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.02,true,&block)};
            unsafe{objc2_foundation::NSRunLoop::currentRunLoop().addTimer_forMode(&timer,objc2_foundation::NSRunLoopCommonModes)};
            cx.new(|_|Demo{host,gpui:native,registration:Some(registration),timer,done})
        }).unwrap();cx.activate(true);
    });
    // Reached only if the run loop returns without GPUI terminating the app.
    reached.store(DONE, Ordering::Release);
}
