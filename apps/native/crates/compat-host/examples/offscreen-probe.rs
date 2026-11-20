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

fn has_fixture_fill(png: &[u8], fill: &str) -> bool {
    let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(png)).unwrap();
    if rep.bitsPerSample() != 8 || rep.samplesPerPixel() != 4 {
        return false;
    }
    // Sample the raw RGBA PNG. NSColor conversion uses the display profile
    // and can change the values of a panel's exact CSS color.
    let mut rgba = [0; 4];
    unsafe {
        rep.getPixel_atX_y(
            std::ptr::NonNull::new(rgba.as_mut_ptr()).unwrap(),
            rep.pixelsWide() / 2,
            rep.pixelsHigh() / 2,
        );
    }
    let color = usize::from_str_radix(fill.trim_start_matches('#'), 16).unwrap();
    rgba == [(color >> 16) & 255, (color >> 8) & 255, color & 255, 255]
}

fn shows_panel_pixels(png: &[u8]) -> bool {
    let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(png)).unwrap();
    let space = NSColorSpace::sRGBColorSpace();
    // Look for the fixture's green action, which the backing ground cannot
    // paint, whether these pixels came from live WebKit or its cached image.
    (0..rep.pixelsHigh()).step_by(4).any(|y| {
        (0..rep.pixelsWide()).step_by(4).any(|x| {
            rep.colorAtX_y(x, y)
                .and_then(|color| color.colorUsingColorSpace(&space))
                .is_some_and(|color| {
                    color.greenComponent() > color.redComponent() + 0.2
                        && color.greenComponent() > color.blueComponent() + 0.07
                })
        })
    })
}

fn verify_native_origin(host: &mut MacDomHost, mtm: MainThreadMarker) {
    let size = NSSize::new(800., 500.);
    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(800., 656.)),
    );
    let anchor = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0., 156.), size),
    );
    root.addSubview(&anchor);
    host.install_below(&root, &anchor);
    if std::env::var_os("PM_DOCK_STACK_LEGACY").is_none() {
        host.enable_panel_renderers(&root, &anchor);
    }
    host.present(Rect::new(0., 0., size.width, size.height))
        .unwrap();
    let actual = host.accessibility_view().frame();
    let expected = anchor.convertRect_toView(anchor.bounds(), Some(&root));
    let dy = actual.origin.y - expected.origin.y;
    eprintln!("native origin: actual={actual:?} expected={expected:?} delta_y={dy}");
    assert_eq!(
        actual, expected,
        "native paint must use the editor view's actual origin"
    );
    host.accessibility_view().removeFromSuperview();
}

/// Drive the same placement, clipping and AppKit present calls the editor uses.
fn verify_divider_resize(host: &mut MacDomHost, name: &str, dir: &std::path::Path) -> usize {
    let dark = name == "dark";
    let background = if dark { "#232222" } else { "#faf9f7" };
    let foreground = if dark { "#f5f4f2" } else { "#232222" };
    eval(
        host.web_view(),
        &format!(
            "document.body.innerHTML='<section data-pm-contribution=pexels style=\"background:{background};color:{foreground};font:13px -apple-system;padding:12px\"><p>Photos and videos provided by Pexels</p><p style=\"background:#238662;color:white;display:inline-block;padding:8px\">Search Pexels</p><p>Photos &nbsp; Videos &nbsp; Orientation</p></section>';return true;"
        ),
    );
    let viewport = Rect::new(0., 0., 800., 500.);
    let id = "pexels".to_owned();
    let held = BTreeSet::from([id.clone()]);
    let released = BTreeSet::new();
    let frame = |host: &mut MacDomHost, width: f64, height: f64, freeze: &BTreeSet<String>| {
        let target = Rect::new(780. - width, 64., width, height);
        host.place_panels(vec![(id.clone(), target)], freeze)
            .unwrap();
        host.set_regions_with_transparent_overlays(vec![target], vec![])
            .unwrap();
        host.set_panel_content_regions(&[(id.clone(), target, target)], None)
            .unwrap();
        host.set_bodies(vec![(id.clone(), target)]);
        host.attach(
            1,
            PanelSlot {
                bounds: viewport,
                clip: viewport,
            },
        )
        .unwrap();
        host.present(viewport).unwrap();
        target
    };
    // A parked WKWebView has no display link. Give the legacy page's
    // rAF-coalescing code a tick without putting any window on screen.
    eval(
        host.web_view(),
        "window.requestAnimationFrame = f => setTimeout(() => f(performance.now()), 8); return true;",
    );
    let start = frame(host, 360., 230., &released);
    until(|| host.panel_layout_applied(&id, start));
    frame(host, 360., 230., &released);

    let snap = |host: &MacDomHost, rect: Rect| {
        let ticket = host.capture_rect_png(rect).unwrap();
        let mut result = None;
        until(|| {
            result = ticket.try_take();
            result.is_some()
        });
        result.unwrap().unwrap()
    };
    let idle_png = snap(host, start);
    assert!(
        shows_panel_pixels(&idle_png),
        "the real WebKit fixture must paint before the drag"
    );
    std::fs::write(dir.join(format!("{name}-web-idle.png")), idle_png).unwrap();
    host.forget_panel_snapshot(&id);
    let mut blanks = 0;
    let mut frames = vec![];
    for (i, (width, height)) in [
        (395., 230.),
        (440., 230.),
        (330., 230.),
        (260., 230.),
        (210., 230.),
        (210., 168.),
    ]
    .into_iter()
    .enumerate()
    {
        let target = frame(host, width, height, &held);
        // Pick a point inside the overlap of all widths as well as the target.
        let (live, snapshot) = host.presented_panel_probe(&id, 735., 110.);
        let capture = if let Some(bytes) = snapshot {
            Some(("snapshot", bytes))
        } else if live {
            Some(("live", snap(host, target)))
        } else {
            None
        };
        let source = if let Some((source, bytes)) = capture {
            std::fs::write(dir.join(format!("{name}-web-{i}.png")), &bytes).unwrap();
            if !shows_panel_pixels(&bytes) {
                blanks += 1;
            }
            source
        } else {
            blanks += 1;
            "blank"
        };
        frames.push(serde_json::json!({"w":width,"h":height,"source":source}));
        pump();
    }
    let released_at = Instant::now();
    let target = frame(host, 210., 168., &released);
    let (live, snapshot) = host.presented_panel_probe(&id, 735., 110.);
    if !live && snapshot.is_none() {
        blanks += 1;
    }
    until(|| host.panel_layout_applied(&id, target));
    frame(host, 210., 168., &released);
    println!(
        "{name}: WebKit layout acknowledged {:.1} ms after release",
        released_at.elapsed().as_secs_f64() * 1000.
    );
    assert!(
        host.presented_panel_probe(&id, 735., 110.).0,
        "release must show live WebKit"
    );
    let settled = snap(host, target);
    assert!(
        shows_panel_pixels(&settled),
        "the live WebKit pixels must return on release"
    );
    std::fs::write(dir.join(format!("{name}-web-released.png")), settled).unwrap();
    std::fs::write(
        dir.join(format!("{name}-web-frames.json")),
        serde_json::to_vec(&frames).unwrap(),
    )
    .unwrap();
    blanks
}

fn verify_isolated_divider(
    page: &mut MacDomHost,
    window: &NSWindow,
    name: &str,
    dir: &std::path::Path,
) {
    let mtm = MainThreadMarker::new().unwrap();
    let viewport = NSSize::new(800., 500.);
    let parent = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, viewport));
    let editor = NSView::initWithFrame(NSView::alloc(mtm), parent.bounds());
    parent.addSubview(&editor);
    window.setContentView(Some(&parent));
    page.install_panel_below(&parent, &editor);
    let dark = name == "dark";
    let (ground, text) = if dark {
        ("#232222", "#f5f4f2")
    } else {
        ("#faf9f7", "#232222")
    };
    eval(
        page.web_view(),
        &format!(
            "document.body.innerHTML='<section data-pm-contribution=pexels style=\"background:{ground};color:{text};font:13px -apple-system;padding:12px\"><p>Photos and videos provided by Pexels</p><p style=\"background:#238662;color:white;display:inline-block;padding:8px\">Search Pexels</p><p>Photos &nbsp; Videos &nbsp; Orientation</p></section>';return true;"
        ),
    );
    let start = Rect::new(420., 64., 360., 230.);
    until(|| {
        page.present_panel_view("pexels", start, start).unwrap();
        page.panel_is_attached()
    });
    let snapshot = |page: &MacDomHost| {
        let ticket = page.capture_png(page.generation()).unwrap();
        let mut bytes = None;
        until(|| {
            bytes = ticket.try_take();
            bytes.is_some()
        });
        bytes.unwrap().unwrap()
    };
    std::fs::write(dir.join(format!("{name}-web-idle.png")), snapshot(page)).unwrap();
    let mut frames = Vec::new();
    for (i, (width, height)) in [
        (395., 230.),
        (440., 230.),
        (330., 230.),
        (260., 230.),
        (210., 230.),
        (210., 168.),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::new(780. - width, 64., width, height);
        page.present_panel_view("pexels", rect, rect).unwrap();
        assert_eq!(page.web_view().frame().size, NSSize::new(width, height));
        let bytes = snapshot(page);
        assert!(
            shows_panel_pixels(&bytes),
            "native resize lost its painted content at frame {i}"
        );
        frames.push(serde_json::json!({"w":width,"h":height,"source":"live"}));
        std::fs::write(dir.join(format!("{name}-web-{i}.png")), bytes).unwrap();
    }
    std::fs::write(
        dir.join(format!("{name}-web-frames.json")),
        serde_json::to_vec(&frames).unwrap(),
    )
    .unwrap();
    println!("{name}: six isolated WebKit resize frames painted in their exact view bounds");
}

/// Every visible body has a different pixel label. The field is an actual
/// AppKit search control in its clip; the other bodies are real WebKit views.
fn verify_isolated_stack(
    mtm: MainThreadMarker,
    stores: &mut StoreDataStores,
    bundle: &str,
    dir: &std::path::Path,
    theme: &str,
    legacy: bool,
) {
    let viewport = NSSize::new(800., 680.);
    let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, viewport));
    let editor = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0., 156.), NSSize::new(800., 524.)),
    );
    root.addSubview(&editor);
    let labels = [
        ("notes", "Motion Notes", "#2960a5"),
        ("pexels", "Pexels", "#238662"),
        ("split", "Split Text", "#ab6945"),
        ("voiceover", "Voiceover", "#6f4eaa"),
        ("pong", "Pong", "#b2314e"),
        ("easing", "Easing", "#967323"),
        ("anchor", "Anchor Point", "#357d95"),
    ];
    let effects = NSSearchField::initWithFrame(NSSearchField::alloc(mtm), NSRect::ZERO);
    effects.setPlaceholderString(Some(&NSString::from_str("Search effects and presets")));
    let appearance = NSAppearance::appearanceNamed(if theme == "dark" {
        unsafe { NSAppearanceNameDarkAqua }
    } else {
        unsafe { NSAppearanceNameAqua }
    });
    effects.setAppearance(appearance.as_deref());
    let clip = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
    clip.setWantsLayer(true);
    clip.layer().unwrap().setMasksToBounds(true);
    clip.addSubview(&effects);
    root.addSubview_positioned_relativeTo(&clip, NSWindowOrderingMode::Above, Some(&editor));
    let trust = Trust::Store {
        extension: "stack-fixture".into(),
        permissions: BTreeSet::new(),
    };
    let (mut host, window) = host(mtm, trust, stores, bundle);
    window.setContentSize(viewport);
    window.setContentView(Some(&root));
    host.install_below(&root, &editor);
    if legacy {
        eval(
            host.web_view(),
            "window.requestAnimationFrame = f => setTimeout(() => f(performance.now()), 8); return true;",
        );
    } else {
        host.enable_panel_renderers(&root, &editor);
    }
    let light = pm_design::Tokens::new(pm_design::Scheme::Light, Default::default());
    let dark = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
    let css = pm_compat_host::native_style::stylesheet(&light, &dark);
    host.update_renderer_style(
        &css,
        if theme == "dark" {
            dark.scheme
        } else {
            light.scheme
        },
    );
    let mut ops = Vec::new();
    for (index, (id, title, fill)) in labels
        .into_iter()
        .chain([("mirage", "Hidden Mirage", "#ff00ff")])
        .enumerate()
    {
        let section = 6 + index as u64 * 2;
        let text = section + 1;
        ops.extend([
            serde_json::json!({"op":"create","id":section,"kind":"element","name":"section","namespace":null}),
            serde_json::json!({"op":"attribute","id":section,"name":"data-pm-contribution","namespace":null,"value":id}),
            serde_json::json!({"op":"attribute","id":section,"name":"style","namespace":null,
                "value":format!("background:{fill}!important;color:white;font:14px -apple-system;padding:9px;visibility:visible")}),
            serde_json::json!({"op":"create","id":text,"kind":"text","name":"","namespace":null}),
            serde_json::json!({"op":"text","id":text,"value":title}),
            serde_json::json!({"op":"insert","parent":section,"id":text,"before":null}),
            serde_json::json!({"op":"insert","parent":4,"id":section,"before":null}),
        ]);
    }
    host.apply(1, 1, serde_json::json!({
        "version":1,"session":{"panel":"fixture","generation":1},"seq":1,"caused_by":null,"ops":ops
    })).unwrap();
    for (frame, (order, width, shown)) in [
        ([0, 1, 2, 3, 4, 5], 300., true),  // rest
        ([0, 1, 2, 3, 4, 5], 300., true),  // pickup
        ([0, 1, 2, 3, 4, 5], 300., true),  // first target
        ([0, 1, 2, 3, 4, 5], 300., true),  // second target
        ([1, 2, 0, 3, 4, 5], 300., true),  // committed drop
        ([1, 2, 0, 3, 4, 5], 300., true),  // cancel
        ([1, 2, 0, 3, 4, 5], 240., true),  // divider
        ([1, 2, 0, 3, 4, 5], 240., false), // workspace change
        ([1, 2, 0, 3, 4, 5], 240., true),  // panel show
        ([1, 2, 6, 3, 4, 5], 240., true),  // tab select
        ([1, 2, 0, 3, 4, 5], 240., true),  // back to previous tab
    ]
    .into_iter()
    .enumerate()
    {
        let field_rect = NSRect::new(
            NSPoint::new(780. - width, 156. + 74.),
            NSSize::new(width, 38.),
        );
        clip.setFrame(field_rect);
        effects.setFrame(NSRect::new(
            NSPoint::new(8., 5.),
            NSSize::new(width - 16., 28.),
        ));
        clip.setHidden(!shown);
        let field = effects.convertRect_toView(effects.bounds(), Some(&root));
        assert_eq!(field.origin.y, field_rect.origin.y + 5.);
        assert!(clip.layer().unwrap().masksToBounds());
        assert!(field.origin.y + field.size.height <= field_rect.origin.y + field_rect.size.height);
        if frame == 4 {
            let bounds = effects.bounds();
            let bitmap = effects
                .bitmapImageRepForCachingDisplayInRect(bounds)
                .unwrap();
            effects.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
            let png = unsafe {
                bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }
            .unwrap();
            std::fs::write(dir.join(format!("{theme}-stack-effects.png")), png.to_vec()).unwrap();
        }
        let poses: Vec<_> = shown
            .then(|| {
                order
                    .iter()
                    .enumerate()
                    .map(|(row, index)| {
                        let id = labels[*index].0;
                        let rect = Rect::new(780. - width, 22. + row as f64 * 63., width, 42.);
                        (id.to_owned(), rect, rect)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if legacy {
            let body = Rect::new(0., 0., 800., 524.);
            host.place_panels(
                poses
                    .iter()
                    .map(|(id, rect, _)| (id.clone(), *rect))
                    .collect(),
                &BTreeSet::new(),
            )
            .unwrap();
            host.set_regions_with_transparent_overlays(
                poses.iter().map(|(_, _, clip)| *clip).collect(),
                vec![],
            )
            .unwrap();
            host.set_panel_content_regions(&poses, None).unwrap();
            host.present(body).unwrap();
            until(|| host.panel_layouts_settled());
            host.set_panel_content_regions(&poses, None).unwrap();
            host.present(body).unwrap();
            if frame == 4 {
                let native = host.accessibility_view().frame();
                let ticket = host.capture_png(host.generation()).unwrap();
                let mut image = None;
                until(|| {
                    image = ticket.try_take();
                    image.is_some()
                });
                std::fs::write(
                    dir.join(format!("{theme}-stack-shared.png")),
                    image.unwrap().unwrap(),
                )
                .unwrap();
                let rows: Vec<_> = poses
                    .iter()
                    .map(|(id, rect, _)| (id, rect.x, rect.y, rect.w, rect.h))
                    .collect();
                std::fs::write(dir.join(format!("{theme}-stack-shared.json")),
                    serde_json::to_vec(&serde_json::json!({
                        "rows": rows, "frame_y": viewport.height - native.origin.y - native.size.height
                    })).unwrap()).unwrap();
            }
            continue;
        }
        let retired: Vec<_> = labels
            .iter()
            .filter_map(|(id, ..)| {
                (!poses.iter().any(|(visible, _, _)| visible == id))
                    .then(|| host.panel_probe(id).map(MacDomHost::accessibility_view))
                    .flatten()
            })
            .collect();
        let warm = host.warm_panels();
        let started = Instant::now();
        host.present_panel_renderers(&poses);
        assert!(retired.iter().all(|view| view.window().is_none()));
        assert!(
            host.panel_probe("mirage").is_none(),
            "hidden root has no renderer"
        );
        if !shown {
            assert!(labels.iter().all(|(id, ..)| host.panel_probe(id).is_none()));
            assert_eq!(host.warm_panels().len(), 2);
            continue;
        }
        for (id, _) in warm
            .iter()
            .filter(|(id, _)| poses.iter().any(|(shown, _, _)| shown == id))
        {
            assert!(
                host.panel_probe(id)
                    .is_some_and(MacDomHost::panel_is_attached),
                "warm re-show must paint on its first frame: {id}"
            );
        }
        until(|| {
            host.service_panel_renderers();
            host.present_panel_renderers(&poses);
            poses.iter().all(|(id, _, _)| {
                host.panel_probe(id)
                    .is_some_and(MacDomHost::panel_is_attached)
            })
        });
        if [0, 8, 9, 10].contains(&frame) {
            println!(
                "{theme} frame {frame}: stack ready after {:.1} ms",
                started.elapsed().as_secs_f64() * 1000.
            );
        }
        for (row, index) in order.into_iter().enumerate() {
            let (id, _, fill) = labels[index];
            let page = host.panel_probe(id).unwrap();
            let rect = poses[row].1;
            let exact = NSRect::new(
                NSPoint::new(rect.x, viewport.height - rect.y - rect.h),
                NSSize::new(rect.w, rect.h),
            );
            let view = page.accessibility_view();
            let aperture = unsafe { view.superview() }.unwrap();
            assert_eq!(aperture.frame(), exact, "{id}, frame {frame}: native clip");
            assert!(aperture.layer().unwrap().masksToBounds());
            assert_eq!(view.frame(), NSRect::new(NSPoint::ZERO, exact.size));
            assert_eq!(
                page.web_view().frame(),
                NSRect::new(NSPoint::ZERO, exact.size)
            );
            assert_eq!(
                eval(
                    page.web_view(),
                    "return [document.documentElement.dataset.pmScheme, document.querySelector('[data-pm-contribution=mirage]').style.display, ...[...document.querySelectorAll('section[data-pm-contribution]')].filter(node => node.style.display === 'block').map(node => [node.getAttribute('data-pm-contribution'),node.getBoundingClientRect().y])];"
                ),
                serde_json::json!([theme, "none", [id, 0]])
            );
            let ticket = page.capture_png(page.generation()).unwrap();
            let mut bytes = None;
            until(|| {
                bytes = ticket.try_take();
                bytes.is_some()
            });
            let bytes = bytes.unwrap().unwrap();
            std::fs::write(
                dir.join(format!("{theme}-stack-{frame}-{row}-{id}.png")),
                &bytes,
            )
            .unwrap();
            assert!(
                has_fixture_fill(&bytes, fill),
                "{id}, frame {frame}: foreign or empty pixels"
            );
        }
    }
    host.present_panel_renderers(&[]);
    host.end_background_job();
    drop(host);
    window.close();
    println!(
        "{theme}: native clip bounds verified for six distinct views over eleven input states"
    );
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
    if let Some(dir) = std::env::var_os("PM_DOCK_NATIVE_RESIZE_CAPTURE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["light", "dark"] {
            let (mut page, window) = host(mtm, trust(name), &mut stores, &bundle);
            verify_isolated_divider(&mut page, &window, name, &dir);
            page.detach_panel_view();
            page.end_background_job();
            drop(page);
            window.close();
        }
        return;
    }
    if let Some(dir) = std::env::var_os("PM_ISOLATED_STACK_CAPTURE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for theme in ["light", "dark"] {
            verify_isolated_stack(
                mtm,
                &mut stores,
                &bundle,
                &dir,
                theme,
                std::env::var_os("PM_DOCK_STACK_LEGACY").is_some(),
            );
        }
        return;
    }
    if std::env::var_os("PM_DOCK_NATIVE_ORIGIN").is_some() {
        let (mut page, window) = host(mtm, trust("native-origin"), &mut stores, &bundle);
        verify_native_origin(&mut page, mtm);
        page.end_background_job();
        drop(page);
        window.close();
        return;
    }
    if let Some(dir) = std::env::var_os("PM_DOCK_TABS_WEB_CAPTURE") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut blanks = 0;
        for name in ["light", "dark"] {
            let (mut page, window) = host(mtm, trust(name), &mut stores, &bundle);
            page.set_fill(Some(if name == "dark" {
                [0.14, 0.13, 0.13, 1.]
            } else {
                [0.98, 0.98, 0.97, 1.]
            }));
            blanks += verify_divider_resize(&mut page, name, &dir);
            page.end_background_job();
            drop(page);
            window.close();
        }
        assert_eq!(blanks, 0, "WebKit frames with an empty native presentation");
        println!("12 WebKit resize frames retained content in the native slot");
        return;
    }
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
