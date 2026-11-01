//! Standalone AppKit host for S5. The timer never calls or waits on JavaScript.
mod lifecycle;
mod pixels;
mod server;
use block2::{DynBlock, RcBlock};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSImage, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    NSData, NSDate, NSDefaultRunLoopMode, NSError, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString, NSTimer, NSURL, NSURLRequest, NSURLResponse,
};
use objc2_web_kit::{
    WKFrameInfo, WKNavigation, WKNavigationDelegate, WKUIDelegate, WKURLSchemeHandler,
    WKURLSchemeTask, WKWebView, WKWebViewConfiguration,
};
use serde_json::json;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Arc<server::State>]
    struct Bridge;
    unsafe impl NSObjectProtocol for Bridge {}
    unsafe impl WKNavigationDelegate for Bridge {
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        unsafe fn terminated(&self,_web:&WKWebView){self.ivars().terminated.store(true,Ordering::SeqCst);self.ivars().record(json!({"test":"content_process_terminated_callback"}));}

        #[unsafe(method(webView:didFinishNavigation:))]
        unsafe fn navigated(&self, _web: &WKWebView, _nav: Option<&WKNavigation>) {
            println!("{}", json!({"test":"navigation_finished"}));
        }
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        unsafe fn failed(&self, _web: &WKWebView, _nav: Option<&WKNavigation>, error: &NSError) {
            println!(
                "{}",
                json!({"test":"navigation_failed","code":error.code(),"domain":error.domain().to_string()})
            );
        }
    }
    unsafe impl WKUIDelegate for Bridge {
        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        unsafe fn prompt(
            &self,
            _web: &WKWebView,
            prompt: &NSString,
            _default: Option<&NSString>,
            _frame: &WKFrameInfo,
            reply: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            if prompt.to_string() == "s5:hold" {
                let reply=reply.copy();let state=self.ivars().clone();state.blocked.store(true,Ordering::SeqCst);
                let callback=RcBlock::new(move |_timer:std::ptr::NonNull<NSTimer>| {
                    state.blocked.store(false,Ordering::SeqCst);
                    let answer=NSString::from_str("released");
                    reply.call((Retained::as_ptr(&answer).cast_mut(),));
                });
                unsafe {NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.3,false,&callback);}
                return;
            }
            let answer = if prompt.to_string() == "s5:ping" {
                Some(NSString::from_str("{\"ok\":true}"))
            } else {
                None
            };
            reply.call((answer
                .as_ref()
                .map_or(std::ptr::null_mut(), |s| Retained::as_ptr(s).cast_mut()),));
        }
    }
    unsafe impl WKURLSchemeHandler for Bridge {
        #[unsafe(method(webView:startURLSchemeTask:))]
        unsafe fn start(&self, _web: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            if unsafe {task.request()}.URL().unwrap().path().is_some_and(|p|p.to_string()=="/hold") {
                let state=self.ivars().clone();state.blocked.store(true,Ordering::SeqCst);
                // The timer retains the task. stop marks it canceled; never call a stopped task.
                let task=unsafe {Retained::retain(task as *const _ as *mut ProtocolObject<dyn WKURLSchemeTask>).unwrap()};
                let key=Retained::as_ptr(&task) as usize;
                let callback=RcBlock::new(move |_timer:std::ptr::NonNull<NSTimer>| {
                    state.blocked.store(false,Ordering::SeqCst);
                    if !state.cancelled_tasks.lock().unwrap().remove(&key) {scheme_reply(&task,b"released");}
                });
                unsafe {NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.3,false,&callback);}
                return;
            }
            scheme_reply(task,b"{\"ok\":true}");
        }
        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn stop(&self, _web: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            self.ivars().cancelled_tasks.lock().unwrap().insert(task as *const _ as usize);
        }
    }
);
fn scheme_reply(task: &ProtocolObject<dyn WKURLSchemeTask>, bytes: &[u8]) {
    let data = NSData::with_bytes(bytes);
    let response = unsafe {
        NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(
            NSURLResponse::alloc(),
            &task.request().URL().unwrap(),
            Some(&NSString::from_str("application/json")),
            data.len() as isize,
            Some(&NSString::from_str("utf-8")),
        )
    };
    unsafe {
        task.didReceiveResponse(&response);
        task.didReceiveData(&data);
        task.didFinish();
    }
}
fn main() {
    objc2::rc::autoreleasepool(|_| run());
}
fn run() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let state = server::start();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let allocated = Bridge::alloc(mtm).set_ivars(state.clone());
    let bridge: Retained<Bridge> = unsafe { msg_send![super(allocated), init] };
    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    unsafe {
        config.setURLSchemeHandler_forURLScheme(
            Some(ProtocolObject::from_ref(&*bridge)),
            &NSString::from_str("s5probe"),
        );
    }
    let case = std::env::var("S5_CASE").unwrap_or_else(|_| "http".into());
    let mut lifecycle = lifecycle::Controller::new(case.clone(), &config, &state);
    let rect = NSRect::new(NSPoint::new(80., 80.), NSSize::new(960., 640.));
    let web =
        unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), rect, &config) };
    unsafe {
        web.setUIDelegate(Some(ProtocolObject::from_ref(&*bridge)));
        web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*bridge)));
    }
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    window.setTitle(&NSString::from_str("Powermove S5 synthetic spike"));
    window.setContentView(Some(&web));
    window.orderFront(None);
    app.finishLaunching();
    let url = NSURL::URLWithString(&NSString::from_str(&format!(
        "http://127.0.0.1:{}/{}/",
        state.port, state.token
    )))
    .unwrap();
    unsafe {
        web.loadRequest(&NSURLRequest::requestWithURL(&url));
    }
    let work: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(vec![]));
    let intervals: Rc<RefCell<Vec<f64>>> = Rc::new(RefCell::new(vec![]));
    let last = Rc::new(RefCell::new(Instant::now()));
    let samples = work.clone();
    let gaps = intervals.clone();
    let prev = last.clone();
    let scrub_view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    let timer_block = RcBlock::new(move |_timer: std::ptr::NonNull<NSTimer>| {
        let t = Instant::now();
        gaps.borrow_mut()
            .push(t.duration_since(*prev.borrow()).as_secs_f64() * 1000.);
        *prev.borrow_mut() = t;
        // 5,000 clip transforms + hit bounds + 1ms minimum CPU slice, then native geometry.
        let mut checksum = 0.;
        let mut tick = 0.;
        while t.elapsed() < Duration::from_millis(1) {
            for i in 0..5000 {
                let x = (i as f64 * 0.17 + tick).sin();
                checksum += x * x;
            }
            tick += 0.01;
        }
        std::hint::black_box(checksum);
        scrub_view.setFrame(NSRect::new(
            NSPoint::new(checksum % 900., 0.),
            NSSize::new(1., 600.),
        ));
        samples.borrow_mut().push(t.elapsed().as_secs_f64() * 1000.);
    });
    let timer = unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(1. / 120., true, &timer_block)
    };
    let snapshot_done = Arc::new(AtomicBool::new(false));
    let snapshot_started = Instant::now();
    let mut probed = false;
    let mut diagnosed = false;
    let started = Instant::now();
    println!("{}", json!({"test":"host_started"}));
    while !state.done.load(Ordering::SeqCst)
        && started.elapsed()
            < Duration::from_secs(if case.starts_with("lifecycle-") {
                160
            } else {
                90
            })
    {
        lifecycle.tick(mtm, &app, &window, &web, &state);
        if !diagnosed
            && !case.starts_with("kill-")
            && !case.starts_with("lifecycle-")
            && started.elapsed() > Duration::from_secs(3)
        {
            diagnosed = true;
            let diagnostic = RcBlock::new(|value: *mut AnyObject, error: *mut NSError| {
                let value = if value.is_null() {
                    String::from("null")
                } else {
                    unsafe {
                        let desc: Retained<NSString> = msg_send![value, description];
                        desc.to_string()
                    }
                };
                println!(
                    "{}",
                    json!({"test":"bootstrap_diagnostic","value":value,"error":!error.is_null()})
                );
            });
            unsafe {
                web.evaluateJavaScript_completionHandler(&NSString::from_str("JSON.stringify({state:document.readyState,scripts:document.scripts.length,channels:typeof channels,body:document.body.innerText.slice(0,80)})"),Some(&diagnostic));
            }
        }
        if !probed && state.blocked.load(Ordering::SeqCst) {
            probed = true;
            state.record(json!({"test":"snapshot_requested_while_blocked"}));
            let done = snapshot_done.clone();
            let s = state.clone();
            let callback = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                s.record(json!({"test":"snapshot_completion","image":!image.is_null(),"error":!error.is_null(),"since_host_start_ms":snapshot_started.elapsed().as_secs_f64()*1000.}));
                done.store(true, Ordering::SeqCst);
            });
            unsafe {
                web.takeSnapshotWithConfiguration_completionHandler(None, &callback);
            }
            // Closest safe locked-session AX proxy: enumerate native container children.
            // It does not force synchronous WebContent accessibility IPC.
            let ax_start = Instant::now();
            let children: Option<Retained<AnyObject>> =
                unsafe { msg_send![&*window, accessibilityChildren] };
            state.record(json!({"test":"ax_container_proxy","children":children.is_some(),"elapsed_ms":ax_start.elapsed().as_secs_f64()*1000.}));
        }
        if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
            NSEventMask::Any,
            Some(&NSDate::dateWithTimeIntervalSinceNow(0.002)),
            unsafe { NSDefaultRunLoopMode },
            true,
        ) {
            app.sendEvent(&event);
        }
        app.updateWindows();
    }
    lifecycle.tick(mtm, &app, &window, &web, &state);
    lifecycle.finish(&state);
    timer.invalidate();
    state.record(json!({"test":"scrub_work", "stats":server::stats(work.borrow().clone()),"intervals":server::stats(intervals.borrow().clone())}));
    for (key, name) in [
        ("poll_ack_ms", "app_to_page_long_poll_ack_upper_bound"),
        ("ws_ack_ms", "app_to_page_websocket_ack_upper_bound"),
    ] {
        let values = state
            .results
            .lock()
            .unwrap()
            .iter()
            .filter_map(|v| v[key].as_f64())
            .collect();
        state.record(json!({"test":name,"stats":server::stats(values)}));
    }
    let success = state.done.load(Ordering::SeqCst) && state.failures.load(Ordering::SeqCst) == 0;
    state.record(json!({"test":"host_finished","page_done":success,"pings":state.pings.load(Ordering::SeqCst),"snapshot_done":snapshot_done.load(Ordering::SeqCst),"elapsed_s":started.elapsed().as_secs_f64()}));
    window.orderOut(None);
    if !success {
        std::process::exit(2);
    }
}
