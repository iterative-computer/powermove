//! Real WebKit integration without activation, input synthesis or screen access.
//! Requires a logged-in WindowServer session, including a locked session.
use block2::RcBlock;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::*;
use objc2_foundation::*;
use objc2_web_kit::WKWebView;
use pm_compat_host::*;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Handler {
    events: RefCell<Vec<HostEvent>>,
    ready: Cell<u64>,
    sealed: Cell<bool>,
    loop_started: Cell<bool>,
}
impl SyncHandler for Handler {
    fn call(&self, c: &RequestContext, method: &str, args: Value) -> pm_compat_host::Result<Value> {
        assert_eq!(c.panel, 1);
        if method == "read" {
            if args["loop"] == true {
                self.loop_started.set(true);
            }
            Ok(args)
        } else {
            Err(pm_compat_host::Error::Forbidden)
        }
    }
    fn event(&self, e: HostEvent) {
        if let HostEvent::Ready { generation } = e {
            self.ready.set(generation);
        }
        self.events.borrow_mut().push(e);
    }
    fn prepare_reset(&self, _: u64) -> pm_compat_host::Result<()> {
        self.sealed.set(true);
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
fn host(
    mtm: MainThreadMarker,
    trust: Trust,
    stores: &mut StoreDataStores,
) -> (MacDomHost, Rc<Handler>, Retained<NSWindow>) {
    let handler = Rc::new(Handler::default());
    let mut host = MacDomHost::new(
        mtm,
        trust,
        handler.clone(),
        stores,
        BTreeMap::from([
            ("read".into(), "project:read".into()),
            ("write".into(), "project:write".into()),
        ]),
    )
    .unwrap();
    host.create(PanelSpec {
        id: 1,
        label: "Offscreen fixture".into(),
    })
    .unwrap();
    host.attach(
        1,
        PanelSlot {
            id: 1,
            bounds: Rect::new(30., 30., 300., 250.),
            clip: Rect::new(30., 30., 300., 250.),
            reading_order: 0,
        },
    )
    .unwrap();
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
    (host, handler, window)
}
fn main() {
    objc2::rc::autoreleasepool(|_| run());
}
fn run() {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let mut stores = StoreDataStores::default();
    let trust = |id: &str| Trust::Store {
        extension: id.into(),
        permissions: BTreeSet::from(["project:read".into()]),
    };
    let (mut a, handler, wa) = host(mtm, trust("fixture-a"), &mut stores);
    let (b, _, wb) = host(mtm, trust("fixture-b"), &mut stores);
    let ds_a = unsafe { a.web_view().configuration().websiteDataStore() };
    let ds_b = unsafe { b.web_view().configuration().websiteDataStore() };
    assert!(!std::ptr::eq(&*ds_a, &*ds_b));
    assert!(a.content_pid() > 0 && b.content_pid() > 0 && a.content_pid() != b.content_pid());
    let result = eval(
        a.web_view(),
        "let storage=false;try{localStorage.setItem('synthetic','x')}catch(_){storage=true}return {origin:location.origin,storage,read:pmCompatHost.call(1,'read',{n:7})};",
    );
    assert_eq!(
        result,
        json!({"origin":"null","storage":true,"read":{"n":7}})
    );
    assert_eq!(
        eval(
            a.web_view(),
            "let denied=0;for(const m of ['write','unknown']){try{pmCompatHost.call(1,m,{})}catch(_){denied++}}return denied;"
        ),
        2
    );
    println!(
        "OFFSCREEN PASS opaque origin, storage denial, Store data-store/PID isolation, scheme RPC and permission denial"
    );
    a.apply(
        1,
        1,
        json!([
            ["create", 1, "input"],
            ["attr", 1, "aria-label", "Synthetic input"],
            ["append", 0, 1],
            ["create", 2, "output"],
            ["text", 2, "Ready"],
            ["append", 0, 2],
            ["create", 3, "div"],
            ["attr", 3, "data-pm-overlay", "true"],
            [
                "attr",
                3,
                "style",
                "position:fixed;left:300px;top:200px;width:120px;height:60px"
            ],
            ["append", 0, 3]
        ]),
    )
    .unwrap();
    until(|| eval(a.web_view(), "return !!document.querySelector('output');") == true);
    eval(
        a.web_view(),
        "const n=document.querySelector('input');n.value='synthetic';n.dispatchEvent(new Event('input',{bubbles:true}));return true;",
    );
    assert!(handler.events.borrow().iter().any(|e|matches!(e,HostEvent::Dom{payload,..} if payload["type"]=="input" && payload["value"]=="synthetic")));
    a.apply(1, 1, json!([["text", 2, "Value: synthetic"]]))
        .unwrap();
    until(|| {
        eval(
            a.web_view(),
            "return document.querySelector('output').textContent;",
        ) == "Value: synthetic"
    });
    until(|| {
        pump();
        !a.composition().overlay_holes.is_empty()
    });
    assert_eq!(
        a.composition().overlay_holes,
        [Rect::new(300., 200., 120., 60.)]
    );
    println!("OFFSCREEN PASS page applier, synthetic input -> Rust -> DOM, real overlay reports");
    eval(
        a.web_view(),
        "window.addEventListener('keydown',e=>{if(e.key==='k')e.preventDefault()});const n=document.querySelector('input');for(const key of ['k','x','z'])n.dispatchEvent(new KeyboardEvent('keydown',{key,ctrlKey:true,bubbles:true,cancelable:true}));return true;",
    );
    until(|| {
        handler
            .events
            .borrow()
            .iter()
            .filter(|e| matches!(e, HostEvent::Key { .. }))
            .count()
            == 2
    });
    let keys: Vec<_> = handler
        .events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            HostEvent::Key { payload, .. } => Some(payload["key"].as_str().unwrap().to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(keys, ["x", "z"]);
    eval(
        a.web_view(),
        "document.querySelector('[data-pm-panel]').addEventListener('pm-host-event',e=>window.hostEventValue=e.detail);return true;",
    );
    a.send_event(1, 1, json!({"revision":7})).unwrap();
    until(|| eval(a.web_view(), "return window.hostEventValue??null;") == json!({"revision":7}));
    println!(
        "OFFSCREEN PASS DOM bubble-order/decline forwarding and app-to-page events (synthetic keys)"
    );

    assert_eq!(
        eval(
            a.web_view(),
            "let ran=false;const s=document.createElement('script');s.textContent='window.extensionCodeRan=true';document.body.append(s);return window.extensionCodeRan===undefined;"
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
    a.detach(1).unwrap();
    until(|| {
        eval(
            a.web_view(),
            "return document.querySelector('[data-pm-panel]').hidden;",
        ) == true
    });
    assert!(a.composition().visible_union.is_empty());
    a.attach(
        1,
        PanelSlot {
            id: 1,
            bounds: Rect::new(30., 30., 300., 250.),
            clip: Rect::new(30., 30., 300., 250.),
            reading_order: 0,
        },
    )
    .unwrap();
    until(|| {
        eval(
            a.web_view(),
            "return document.querySelector('[data-pm-panel]').hidden;",
        ) == false
    });
    a.destroy(1).unwrap();
    until(|| {
        eval(
            a.web_view(),
            "return document.querySelector('[data-pm-panel]')===null;",
        ) == true
    });
    assert_eq!(
        a.apply(1, 1, json!([])),
        Err(pm_compat_host::Error::UnknownPanel)
    );
    println!("OFFSCREEN PASS detach, reattach and destroy");
    a.create(PanelSpec {
        id: 1,
        label: "Recovery fixture".into(),
    })
    .unwrap();
    a.attach(
        1,
        PanelSlot {
            id: 1,
            bounds: Rect::new(30., 30., 300., 250.),
            clip: Rect::new(30., 30., 300., 250.),
            reading_order: 0,
        },
    )
    .unwrap();
    until(|| {
        eval(
            a.web_view(),
            "return !!document.querySelector('[data-pm-panel]');",
        ) == true
    });
    let old = a.content_pid();
    unsafe {
        a.web_view().evaluateJavaScript_completionHandler(
            &NSString::from_str("pmCompatHost.call(1,'read',{loop:true});while(true){}"),
            None,
        );
    }
    until(|| handler.loop_started.get());
    let started = Instant::now();
    assert_eq!(a.stop_and_recover().unwrap(), 2);
    assert!(handler.sealed.get());
    assert_eq!(
        a.apply(1, 1, json!([])),
        Err(pm_compat_host::Error::StaleGeneration)
    );
    until(|| {
        a.tick();
        handler.events.borrow().iter().any(|e|matches!(e,HostEvent::Recovered{old_pid,new_pid,generation:2} if *old_pid==old && *new_pid!=old))
    });
    println!(
        "OFFSCREEN PASS real infinite-loop reset, sealed generation, replacement ready and old PID exited; recovery_ms={:.2}",
        started.elapsed().as_secs_f64() * 1000.
    );
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
