//! Standalone AppKit host for S5. The timer never calls or waits on JavaScript.
mod kernel;
mod lifecycle;
mod pixels;
mod s5b;
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
    NSData, NSDate, NSDefaultRunLoopMode, NSDictionary, NSError, NSHTTPURLResponse, NSObject,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer, NSURL, NSURLRequest,
    NSURLResponse,
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
            let url = unsafe { task.request() }.URL().unwrap();
            let path = url.path().map(|p|p.to_string()).unwrap_or_default();
            if path.starts_with("/s5b/") {
                let (mime, bytes) = self.ivars().s5b.request(&path);
                scheme_reply_arc(task, bytes, mime, path.contains("/stream/"));
            } else { scheme_reply(task,b"{\"ok\":true}"); }
        }
        #[unsafe(method(webView:stopURLSchemeTask:))]
        unsafe fn stop(&self, _web: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            self.ivars().cancelled_tasks.lock().unwrap().insert(task as *const _ as usize);
        }
    }
);
fn scheme_reply(task: &ProtocolObject<dyn WKURLSchemeTask>, bytes: &[u8]) {
    scheme_reply_mime(task, bytes, "application/json");
}
fn scheme_reply_mime(task: &ProtocolObject<dyn WKURLSchemeTask>, bytes: &[u8], mime: &str) {
    let data = NSData::with_bytes(bytes);
    scheme_reply_data(task, &[data], mime);
}
fn scheme_reply_arc(
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    bytes: Arc<Vec<u8>>,
    mime: &str,
    stream: bool,
) {
    // Each NSData retains the immutable allocation through its deallocator block.
    // This removes the UI-process NSData copy; WebKit IPC/decode may still copy.
    let size = if stream {
        64 * 1024
    } else {
        bytes.len().max(1)
    };
    let data: Vec<_> = bytes
        .chunks(size)
        .map(|chunk| {
            let keep = bytes.clone();
            let release = RcBlock::new(move |_: std::ptr::NonNull<std::ffi::c_void>, _: usize| {
                std::hint::black_box(&keep);
            });
            unsafe {
                NSData::initWithBytesNoCopy_length_deallocator(
                    NSData::alloc(),
                    std::ptr::NonNull::new(chunk.as_ptr().cast_mut().cast()).unwrap(),
                    chunk.len(),
                    Some(&release),
                )
            }
        })
        .collect();
    scheme_reply_data(task, &data, mime);
}
fn scheme_reply_data(
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    data: &[Retained<NSData>],
    mime: &str,
) {
    let length: usize = data.iter().map(|d| d.len()).sum();
    let response = unsafe {
        NSURLResponse::initWithURL_MIMEType_expectedContentLength_textEncodingName(
            NSURLResponse::alloc(),
            &task.request().URL().unwrap(),
            Some(&NSString::from_str(mime)),
            length as isize,
            Some(&NSString::from_str("utf-8")),
        )
    };
    let image_response = if mime.starts_with("image/") {
        let keys = [
            NSString::from_str("Content-Type"),
            NSString::from_str("Access-Control-Allow-Origin"),
            NSString::from_str("Cross-Origin-Resource-Policy"),
            NSString::from_str("Cache-Control"),
        ];
        let vals = [
            NSString::from_str(mime),
            NSString::from_str("*"),
            NSString::from_str("cross-origin"),
            NSString::from_str("no-store"),
        ];
        let headers = NSDictionary::from_slices(
            &keys.iter().map(|x| &**x).collect::<Vec<_>>(),
            &vals.iter().map(|x| &**x).collect::<Vec<_>>(),
        );
        Some(unsafe {
            NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
                NSHTTPURLResponse::alloc(),
                &task.request().URL().unwrap(),
                200,
                None,
                Some(&headers),
            )
            .unwrap()
        })
    } else {
        None
    };
    unsafe {
        task.didReceiveResponse(image_response.as_deref().map(|r| &**r).unwrap_or(&response));
        for chunk in data {
            task.didReceiveData(chunk);
        }
        task.didFinish();
    }
}
fn main() {
    if std::env::args().any(|arg| arg == "--kernel") {
        for row in kernel::traces() {
            println!("{row}");
        }
        return;
    }
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
    app.finishLaunching();
    // A visible accessory window alone is not a foreground comparator.
    let headless = std::env::var("S5_HEADLESS").as_deref() == Ok("1");
    if headless {
        window.setFrameOrigin(NSPoint::new(-20000., -20000.));
        window.setCanHide(false);
        if std::env::var("S5_VISIBLE_HOST").as_deref() == Ok("1") {
            window.orderFront(None);
        }
    } else {
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        // Activation is asynchronous. Do not start measuring a merely ordered-front
        // window: establish the actual AppKit foreground baseline first.
        let activation_started = Instant::now();
        while (!app.isActive() || !window.isKeyWindow())
            && activation_started.elapsed() < Duration::from_secs(2)
        {
            if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                NSEventMask::Any,
                Some(&NSDate::dateWithTimeIntervalSinceNow(0.01)),
                unsafe { NSDefaultRunLoopMode },
                true,
            ) {
                app.sendEvent(&event);
            }
            app.updateWindows();
        }
        if !app.isActive() || !window.isKeyWindow() {
            state.record(json!({"test":"foreground_activation_failed","app_active":app.isActive(),"window_key":window.isKeyWindow()}));
            lifecycle.finish(&state);
            std::process::exit(2);
        }
    } // foreground-only guard; headless runs never activate the app
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
    state.record(json!({"test":"foreground_state","headless":headless,"app_active":app.isActive(),"window_key":window.isKeyWindow(),"window_visible":window.isVisible(),"minimised":window.isMiniaturized()}));
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
            && !case.starts_with("s5b-")
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
    let needs_snapshot = matches!(case.as_str(), "http" | "scheme" | "prompt" | "app");
    let success = state.done.load(Ordering::SeqCst)
        && state.failures.load(Ordering::SeqCst) == 0
        && (!needs_snapshot || snapshot_done.load(Ordering::SeqCst));
    state.record(json!({"test":"host_finished","page_done":success,"pings":state.pings.load(Ordering::SeqCst),"snapshot_done":snapshot_done.load(Ordering::SeqCst),"elapsed_s":started.elapsed().as_secs_f64()}));
    window.orderOut(None);
    if !success {
        std::process::exit(2);
    }
}
