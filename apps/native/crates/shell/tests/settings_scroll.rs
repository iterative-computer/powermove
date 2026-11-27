//! Native wheel routing in the Extensions form, in a window that is never shown.
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSButton, NSEvent,
        NSScrollView, NSSwitch, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_core_graphics::{CGEvent, CGScrollEventUnit};
    use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize, ns_string};

    define_class!(
        #[unsafe(super(NSView))]
        #[thread_kind = MainThreadOnly]
        #[name = "PMSettingsScrollTestDocument"]
        struct Document;

        impl Document {
            #[unsafe(method(isFlipped))]
            fn flipped(&self) -> bool { true }
        }
    );

    let mtm = MainThreadMarker::new().expect("the test binary keeps the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let frame = rect(0., 0., 400., 200.);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    let page = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
    page.setHasVerticalScroller(true);
    window.setContentView(Some(&page));
    let document = |frame| -> Retained<Document> {
        unsafe { msg_send![super(Document::alloc(mtm).set_ivars(())), initWithFrame: frame] }
    };
    let width = page.contentSize().width;
    let outer = document(rect(0., 0., width, page.contentSize().height + 300.));
    page.setAutomaticallyAdjustsContentInsets(false);
    page.setDocumentView(Some(&outer));
    let list = pm_shell::settings_window::extension_list_scroll(mtm);
    let list_width = width - 60.;
    list.setFrame(rect(30., 50., list_width, 100.));
    list.setAutomaticallyAdjustsContentInsets(false);
    list.setHasVerticalScroller(false);
    outer.addSubview(&list);
    let rows = document(rect(0., 0., list_width, 100.));
    list.setDocumentView(Some(&rows));
    let details = NSButton::new(mtm);
    details.setTitle(ns_string!("Details"));
    let enabled = NSSwitch::new(mtm);
    let controls: [&NSView; 2] = [&details, &enabled];
    for (i, control) in controls.into_iter().enumerate() {
        control.setFrame(rect(20. + i as f64 * 100., 15., 80., 24.));
        rows.addSubview(control);
    }

    let page_clip = page.contentView();
    let list_clip = list.contentView();
    let wheel = |view: &NSView, lines| {
        let cg = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 1, lines, 0, 0)
            .expect("a mouse wheel event");
        let event = NSEvent::eventWithCGEvent(&cg).expect("an AppKit wheel event");
        view.scrollWheel(&event);
        // AppKit animates the line-wheel event on the run loop.
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.5));
    };

    // A gutter, the list background, and the real controls all drive the
    // page when the list fits. The controls still win native hit testing.
    for (location, target) in [
        ("gutter", &*outer as &NSView),
        ("list background", &*rows),
        ("Details button", &*details),
        ("enable switch", &*enabled),
    ] {
        page_clip.scrollToPoint(NSPoint::ZERO);
        wheel(target, -3);
        assert!(
            page_clip.bounds().origin.y > 0.,
            "the page should scroll over the {location} (page {:?}, list {:?})",
            page_clip.bounds(),
            list_clip.bounds(),
        );
        assert_eq!(list_clip.bounds().origin.y, 0.);
    }
    page_clip.scrollToPoint(NSPoint::ZERO);
    for control in controls {
        let bounds = control.bounds();
        let center = NSPoint::new(bounds.size.width / 2., bounds.size.height / 2.);
        let point = control.convertPoint_toView(center, Some(&page_clip));
        let hit = outer.hitTest(point).expect("a hit over the control");
        assert!(hit.isDescendantOf(control), "the control receives the hit");
    }

    // A virtualized list uses its own scroll range and hands the wheel
    // back to the page when it reaches the end.
    rows.setFrameSize(NSSize::new(list_width, 600.));
    list.setHasVerticalScroller(true);
    wheel(&rows, -3);
    assert!(list_clip.bounds().origin.y > 0.);
    assert_eq!(page_clip.bounds().origin.y, 0.);
    list_clip.scrollToPoint(NSPoint::new(0., 600. - list_clip.bounds().size.height));
    wheel(&rows, -3);
    let offset = page_clip.bounds().origin.y;
    assert!(offset > 0.);
    list_clip.scrollToPoint(NSPoint::ZERO);
    wheel(&rows, 3);
    assert!(page_clip.bounds().origin.y < offset);
    window.close();
    println!("settings_scroll: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
