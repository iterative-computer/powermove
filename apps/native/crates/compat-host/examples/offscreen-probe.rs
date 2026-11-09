//! Real WebKit integration of the production page without activation, input
//! synthesis or screen access. Requires a logged-in WindowServer session,
//! including a locked session. Argument: the built page bundle.
use block2::RcBlock;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::*;
use objc2_foundation::*;
use objc2_web_kit::WKWebView;
use pm_compat_host::*;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    rc::Rc,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Handler {
    ready: Cell<u64>,
}
impl SyncHandler for Handler {
    fn event(&self, e: HostEvent) {
        match e {
            HostEvent::Ready { generation } => self.ready.set(generation),
            HostEvent::Failure(reason) => panic!("page failure: {reason}"),
            _ => (),
        }
    }
    fn remote_dom(&self, _: Value) -> pm_compat_host::Result<()> {
        Ok(())
    }
}
fn pump() {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
    if let Some(e) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
        NSEventMask::Any,
        Some(&NSDate::dateWithTimeIntervalSinceNow(0.003)),
        unsafe { NSDefaultRunLoopMode },
        true,
    ) {
        app.sendEvent(&e);
    }
    app.updateWindows();
}
fn until(mut condition: impl FnMut() -> bool) {
    let t = Instant::now();
    while !condition() {
        assert!(t.elapsed() < Duration::from_secs(10), "offscreen timeout");
        pump();
    }
}
fn eval(web: &WKWebView, code: &str) -> Value {
    let out = Rc::new(RefCell::new(None));
    let result = out.clone();
    let block = RcBlock::new(move |value: *mut AnyObject, error: *mut NSError| {
        assert!(error.is_null(), "probe JS failed");
        let text: Retained<NSString> = unsafe { msg_send![value, description] };
        *result.borrow_mut() = Some(serde_json::from_str::<Value>(&text.to_string()).unwrap());
    });
    unsafe {
        web.evaluateJavaScript_completionHandler(
            &NSString::from_str(&format!("JSON.stringify((()=>{{{code}}})())")),
            Some(&block),
        );
    }
    until(|| out.borrow().is_some());
    out.take().unwrap()
}
fn content_pid(web: &WKWebView) -> i32 {
    unsafe { msg_send![web, _webProcessIdentifier] }
}
fn slot() -> PanelSlot {
    let r = Rect::new(30., 30., 300., 250.);
    PanelSlot { bounds: r, clip: r }
}
fn host(
    mtm: MainThreadMarker,
    trust: Trust,
    stores: &mut StoreDataStores,
    bundle: &str,
) -> (MacDomHost, Retained<NSWindow>) {
    let handler = Rc::new(Handler::default());
    let mut host = MacDomHost::new_remote_dom(mtm, trust, handler.clone(), stores, bundle).unwrap();
    host.create(PanelSpec {
        id: 1,
        label: "Offscreen fixture".into(),
    })
    .unwrap();
    host.attach(1, slot()).unwrap();
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(800., 500.)),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setContentView(Some(&host.accessibility_view()));
    host.present(Rect::new(0., 0., 800., 500.)).unwrap();
    host.begin_background_job(false).unwrap();
    until(|| handler.ready.get() == 1);
    (host, window)
}
fn main() {
    objc2::rc::autoreleasepool(|_| run());
}
fn run() {
    let mtm = MainThreadMarker::new().unwrap();
    let bundle = std::fs::read_to_string(std::env::args().nth(1).expect("page bundle required"))
        .expect("readable page bundle");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let mut stores = StoreDataStores::default();
    let trust = |id: &str| Trust::Store {
        extension: id.into(),
        permissions: BTreeSet::new(),
    };
    let (mut a, wa) = host(mtm, trust("fixture-a"), &mut stores, &bundle);
    let (b, wb) = host(mtm, trust("fixture-b"), &mut stores, &bundle);
    let ds_a = unsafe { a.web_view().configuration().websiteDataStore() };
    let ds_b = unsafe { b.web_view().configuration().websiteDataStore() };
    assert!(!std::ptr::eq(&*ds_a, &*ds_b));
    let (pid_a, pid_b) = (content_pid(a.web_view()), content_pid(b.web_view()));
    assert!(pid_a > 0 && pid_b > 0 && pid_a != pid_b);
    assert_eq!(
        eval(
            a.web_view(),
            "let storage=false;try{localStorage.setItem('synthetic','x')}catch(_){storage=true}return {origin:location.origin,storage};",
        ),
        serde_json::json!({"origin":"null","storage":true})
    );
    println!("OFFSCREEN PASS opaque origin, storage denial, Store data-store/PID isolation");
    assert_eq!(
        eval(
            a.web_view(),
            "const s=document.createElement('script');s.textContent='window.extensionCodeRan=true';document.body.append(s);return window.extensionCodeRan===undefined;"
        ),
        true
    );
    eval(
        a.web_view(),
        "location.href='https://example.invalid/blocked';return true;",
    );
    pump();
    assert_eq!(eval(a.web_view(), "return location.href;"), "about:blank");
    assert_eq!(
        eval(a.web_view(), "return window.open('about:blank')===null;"),
        true
    );
    println!("OFFSCREEN PASS CSP script rejection, navigation denial and Store popout denial");
    let hidden = |a: &MacDomHost| eval(a.web_view(), "return document.documentElement.hidden;");
    a.detach(1).unwrap();
    until(|| hidden(&a) == true);
    a.attach(1, slot()).unwrap();
    until(|| hidden(&a) == false);
    a.destroy(1).unwrap();
    assert_eq!(
        a.apply(1, 1, serde_json::json!({})),
        Err(pm_compat_host::Error::UnknownPanel)
    );
    println!("OFFSCREEN PASS detach, reattach and destroy");
    let view = a.accessibility_view();
    let original_parent = unsafe { view.superview() }.unwrap();
    let original_frame = view.frame();
    a.end_background_job();
    assert!(std::ptr::eq(
        &*unsafe { view.superview() }.unwrap(),
        &*original_parent
    ));
    a.begin_background_job(false).unwrap();
    a.begin_background_job(true).unwrap(); // A later nested job can require parking.
    a.end_background_job();
    a.end_background_job();
    assert!(std::ptr::eq(
        &*unsafe { view.superview() }.unwrap(),
        &*original_parent
    ));
    assert_eq!(view.frame(), original_frame);
    drop(a);
    drop(b);
    wa.close();
    wb.close();
    println!(
        "OFFSCREEN PASS activity nesting, parking and teardown; no foreground/typing/visual claim"
    );
}
