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
fn verify_native_style(host: &MacDomHost) {
    use pm_design::{Accessibility, Scheme, Tokens};
    let light = Tokens::new(Scheme::Light, Accessibility::default());
    let dark = Tokens::new(Scheme::Dark, Accessibility::default());
    let rgb = |c: u32| format!("rgb({}, {}, {})", (c >> 16) & 255, (c >> 8) & 255, c & 255);
    for t in [&light, &dark] {
        host.update_native_style(&light, &dark, t.scheme).unwrap();
        let scheme = if t.scheme == Scheme::Dark {
            "dark"
        } else {
            "light"
        };
        until(|| {
            eval(
                host.web_view(),
                "return document.documentElement.dataset.pmScheme;",
            ) == scheme
        });
        let value = eval(
            host.web_view(),
            "const s=getComputedStyle(document.body);return [s.fontSize,s.fontFamily,s.backgroundColor,s.color,getComputedStyle(document.documentElement).colorScheme];",
        );
        assert_eq!(value[0], format!("{}px", t.typography.body));
        assert!(value[1].as_str().unwrap().contains("-apple-system"));
        assert_eq!(value[2], "rgba(0, 0, 0, 0)");
        assert_eq!(value[3], rgb(t.colors.text));
        assert_eq!(value[4], scheme);
        assert_eq!(
            eval(
                host.web_view(),
                "document.body.innerHTML='<button id=plain class=pm-button>Plain</button><div class=pm-segmented><button aria-pressed=true>Selected</button></div>';const s=getComputedStyle(document.querySelector('#plain'));return [s.borderTopWidth,s.backgroundColor,s.minHeight];"
            ),
            serde_json::json!([
                "0px",
                rgb(t.colors.fill),
                format!("{}px", t.metrics.control)
            ])
        );
        assert_eq!(
            eval(
                host.web_view(),
                "return getComputedStyle(document.querySelector('[aria-pressed]')).backgroundColor;"
            ),
            rgb(t.colors.fill_strong)
        );
        assert_eq!(
            eval(
                host.web_view(),
                "const style=document.createElement('style');style.id='author';style.textContent='button{background:rgb(1,2,3);font-size:19px;border:3px solid red;border-radius:17px}';document.head.append(style);const s=getComputedStyle(document.querySelector('#plain'));return [s.backgroundColor,s.fontSize,s.borderTopWidth,s.borderRadius];"
            ),
            serde_json::json!(["rgb(1, 2, 3)", "19px", "3px", "17px"])
        );
        eval(
            host.web_view(),
            "document.getElementById('author').remove();document.documentElement.dataset.pmStyle='tokens-only';return true;",
        );
        until(|| {
            eval(
                host.web_view(),
                "return !document.getElementById('pm-native').textContent.includes('@layer pm-base');",
            ) == true
        });
        assert_eq!(
            eval(
                host.web_view(),
                "return getComputedStyle(document.documentElement).getPropertyValue('--pm-ground').trim();"
            ),
            format!("#{:06x}", t.colors.panel)
        );
        eval(
            host.web_view(),
            "document.documentElement.dataset.pmStyle='none';return true;",
        );
        until(|| {
            eval(
                host.web_view(),
                "return document.getElementById('pm-native').textContent.length;",
            ) == 0
        });
        assert_eq!(
            eval(
                host.web_view(),
                "return getComputedStyle(document.documentElement).getPropertyValue('--pm-ground').trim();"
            ),
            ""
        );
        eval(
            host.web_view(),
            "delete document.documentElement.dataset.pmStyle;return true;",
        );
        until(|| {
            eval(
                host.web_view(),
                "return document.getElementById('pm-native').textContent.includes('@layer pm-base');",
            ) == true
        });
        assert_eq!(
            eval(
                host.web_view(),
                "document.body.innerHTML='<section data-pm-style=none><button>None</button></section><section data-pm-style=tokens-only><button>Tokens</button></section><button id=full>Full</button>';return [...document.querySelectorAll('button')].map(x=>getComputedStyle(x).borderTopWidth);"
            ),
            serde_json::json!(["2px", "2px", "0px"])
        );
        assert_eq!(
            eval(
                host.web_view(),
                "return getComputedStyle(document.querySelector('[data-pm-style=none]')).getPropertyValue('--pm-ground').trim();"
            ),
            ""
        );
        let metrics = eval(
            host.web_view(),
            "document.body.innerHTML='<div class=pm-toolbar><input type=search class=pm-search placeholder=Search></div><div class=pm-grid><button class=pm-card><div class=pm-card-media>Image</div><span class=pm-card-title>Title</span><span class=pm-card-meta>Meta</span></button></div><button id=focus>Focus</button>';const style=x=>getComputedStyle(document.querySelector(x));return [style('.pm-grid').gap,style('.pm-card-media').borderRadius,style('.pm-card-title').fontSize,style('.pm-card-meta').fontSize,getComputedStyle(document.body,'::-webkit-scrollbar').width,String(document.querySelector('.pm-toolbar').getBoundingClientRect().height)+'px'];",
        );
        assert_eq!(
            metrics,
            serde_json::json!([
                format!("{}px", t.metrics.gap),
                format!("{}px", t.radius.md),
                format!("{}px", t.typography.body),
                format!("{}px", t.typography.minimum),
                "10px",
                format!("{}px", t.metrics.header)
            ])
        );
        if let Some(dir) = std::env::var_os("PM_NATIVE_STYLE_CAPTURE") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            let ticket = host.capture_png(host.generation()).unwrap();
            let mut captured = None;
            until(|| {
                captured = ticket.try_take();
                captured.is_some()
            });
            std::fs::write(
                dir.join(format!("kit-controls-{scheme}.png")),
                captured.unwrap().unwrap(),
            )
            .unwrap();
        }
        // This unordered, activation-prohibited AppKit window cannot own DOM
        // focus. Real Tab navigation and focus-visible are checked without a
        // desktop window by tests/native-style-keyboard.mjs.
        assert_eq!(
            eval(
                host.web_view(),
                "document.documentElement.classList.add('pm-scrolling');return getComputedStyle(document.body,'::-webkit-scrollbar-thumb').backgroundColor !== 'rgba(0, 0, 0, 0)';"
            ),
            true
        );
        println!(
            "OFFSCREEN STYLE PASS {scheme}: defaults, native controls, neutral selection, author cascade, document and scoped opt-outs, live scheme"
        );
    }
    let mut patched = dark.clone();
    patched
        .apply_patch([("colors.panel", 0x123456 as f64)])
        .unwrap();
    host.update_native_style(&light, &patched, Scheme::Dark)
        .unwrap();
    eval(
        host.web_view(),
        "document.body.innerHTML='<div id=ground style=background:var(--pm-ground)>Ground</div>';return true;",
    );
    until(|| {
        eval(
            host.web_view(),
            "return getComputedStyle(document.getElementById('ground')).backgroundColor;",
        ) == "rgb(18, 52, 86)"
    });
    let accessible = Tokens::new(
        Scheme::Dark,
        Accessibility {
            increase_contrast: true,
            reduce_motion: true,
            ..Default::default()
        },
    );
    host.update_native_style(&light, &accessible, Scheme::Dark)
        .unwrap();
    until(|| {
        eval(
            host.web_view(),
            "return getComputedStyle(document.documentElement).getPropertyValue('--pm-dur').trim();",
        ) == "0ms"
    });
    assert_eq!(
        eval(
            host.web_view(),
            "return getComputedStyle(document.documentElement).getPropertyValue('--pm-separator').trim();"
        ),
        format!("#{:06x}", accessible.colors.separator)
    );
    println!("OFFSCREEN STYLE PASS theme patch, Increase Contrast, Reduce Motion without reload");
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
    verify_native_style(&a);
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
