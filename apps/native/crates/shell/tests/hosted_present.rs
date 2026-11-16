//! Frames that carry hosted AppKit views (vendor/gpui-ce POWERMOVE.md, P-120):
//! the main thread never waits for a drawable, a scene is never drawn into a
//! drawable of another size, a view staged by a paint that was never shown is
//! not placed over a later scene, and a pop-up's rows follow a publication
//! without destroying the items of an open menu, and the viewer's picture
//! shows in a window that opened on Home. Runs on the main thread
//! with activation prohibited and never orders a window on screen.
#[cfg(target_os = "macos")]
fn main() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let mtm = MainThreadMarker::new().expect("runs on the main thread (harness = false)");
    NSApplication::sharedApplication(mtm)
        .setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    macos::policy();
    macos::drawables();
    macos::paints(mtm);
    macos::rows();
    macos::host_rows();
    macos::viewer_slot(mtm);
    macos::pictures(mtm);
    macos::hidden_moves(mtm);
    println!("hosted_present: ok");
}
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod macos {
    use foreign_types::ForeignType;
    use gpui_apple::metal_renderer::{DrawableSource, new_window_layer};
    use gpui_macos::composition::{
        DIRECT_LINGER, LinkDrawable, Pace, Step, leaves_direct, probe, step,
    };
    use gpui_macos::native_control::NativeControlSlot;
    use objc2::{MainThreadMarker, MainThreadOnly, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{
        NSBackingStoreType, NSButton, NSMenuDidBeginTrackingNotification,
        NSMenuDidEndTrackingNotification, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSDate, NSNotificationCenter, NSPoint, NSRect, NSRunLoop, NSSize};
    use pm_shell::native_control::PopupRows;
    use serde_json::json;
    use std::time::{Duration, Instant};

    /// P-HIDDEN-MOVES: a hosted view hidden before and after a frame (a
    /// dense field at rest, a control scrolled out of its clip) follows its
    /// row without making the frame a synchronous present; the frame that
    /// shows it again does.
    pub fn hidden_moves(mtm: MainThreadMarker) {
        use gpui_macos::native_text::{NativeTextSlot, TextKind};
        let rect =
            |x: f64, y: f64, w: f64, h: f64| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(-20000., -20000., 400., 300.),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let content = window.contentView().expect("a content view");
        let gpui = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
        content.addSubview(&gpui);
        let clip = rect(0., 0., 400., 300.);
        // A dense number field at rest: drawn by GPUI, its editor hidden.
        let field = NativeTextSlot::new(&gpui, TextKind::DenseNumber, "Width", "128");
        field.stage(rect(10., 10., 90., 22.), clip);
        probe::painted(&gpui);
        probe::place(&gpui);
        for y in [14., 30., 80., 200.] {
            field.stage(rect(10., y, 90., 22.), clip);
            probe::painted(&gpui);
            assert!(!probe::pending(&gpui), "a field at rest moved at {y}");
            assert!(!probe::place(&gpui));
        }
        // Clicked: its editor shows where the row now is.
        field.begin_editing();
        field.stage(rect(10., 200., 90., 22.), clip);
        probe::painted(&gpui);
        assert!(probe::place(&gpui), "the editor shows");
        drop(field);
        // A control below its clip (scrolled out), moving on below it.
        let button: Retained<NSView> = unsafe {
            let raw: *mut AnyObject = msg_send![objc2::class!(NSButton), alloc];
            let raw: *mut NSView = msg_send![raw, initWithFrame: rect(0., 0., 80., 20.)];
            Retained::from_raw(raw).expect("a button")
        };
        let slot = NativeControlSlot::new(&gpui, button);
        slot.stage(rect(10., 20., 80., 20.), clip);
        probe::painted(&gpui);
        assert!(probe::place(&gpui), "shown");
        slot.stage(rect(10., 320., 80., 20.), clip);
        probe::painted(&gpui);
        assert!(probe::place(&gpui), "hidden as it leaves");
        for y in [340., 400., 700.] {
            slot.stage(rect(10., y, 80., 20.), clip);
            probe::painted(&gpui);
            assert!(!probe::place(&gpui), "out of its clip, moved at {y}");
        }
        slot.stage(rect(10., 100., 80., 20.), clip);
        probe::painted(&gpui);
        assert!(probe::place(&gpui), "back in");
        assert!(!slot.wrapper().isHidden());
        window.close();
    }

    /// P-SLOT-PICTURE: a control's picture is AppKit's own drawing of it,
    /// taken whether its slot was ever shown or not (a row that scrolls in
    /// while the pictures stand has never been shown), and hiding a slot
    /// leaves the next present nothing to move but the hide itself.
    /// `PM_PICTURE_DIR=<dir>` writes each picture there (PPM over grey).
    pub fn pictures(mtm: MainThreadMarker) {
        use objc2::runtime::AnyClass;
        let rect =
            |x: f64, y: f64, w: f64, h: f64| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(-20000., -20000., 400., 300.),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let dark = objc2_app_kit::NSAppearance::appearanceNamed(unsafe {
            objc2_app_kit::NSAppearanceNameDarkAqua
        });
        objc2_app_kit::NSAppearanceCustomization::setAppearance(&*window, dark.as_deref());
        let content = window.contentView().expect("a content view");
        let gpui = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
        content.addSubview(&gpui);
        let make = |class: &AnyClass, w: f64, h: f64| -> Retained<NSView> {
            unsafe {
                let raw: *mut AnyObject = msg_send![class, alloc];
                let raw: *mut NSView = msg_send![raw, initWithFrame: rect(0., 0., w, h)];
                Retained::from_raw(raw).expect("a control")
            }
        };
        let title = |s: &str| objc2_foundation::NSString::from_str(s);
        let checkbox = make(objc2::class!(NSButton), 80., 20.);
        let popup = make(objc2::class!(NSPopUpButton), 140., 24.);
        let segmented = make(objc2::class!(NSSegmentedControl), 150., 24.);
        let slider = make(objc2::class!(NSSlider), 140., 20.);
        let field = make(objc2::class!(NSTextField), 90., 22.);
        let switch = make(objc2::class!(NSSwitch), 40., 22.);
        unsafe {
            let _: () = msg_send![&*checkbox, setButtonType: 3usize];
            let _: () = msg_send![&*checkbox, setTitle: &*title("Extrude")];
            let _: () = msg_send![&*checkbox, setState: 1isize];
            let _: () = msg_send![&*popup, addItemWithTitle: &*title("Geist Width")];
            let _: () = msg_send![&*segmented, setSegmentCount: 3isize];
            for (i, l) in ["Left", "Centre", "Right"].iter().enumerate() {
                let _: () = msg_send![&*segmented, setLabel: &*title(l), forSegment: i as isize];
            }
            let _: () = msg_send![&*segmented, setSelectedSegment: 1isize];
            let _: () = msg_send![&*slider, setDoubleValue: 0.4f64];
            let _: () = msg_send![&*field, setStringValue: &*title("128 px")];
            let _: () = msg_send![&*switch, setState: 1isize];
        }
        let controls = [
            ("checkbox", checkbox),
            ("popup", popup),
            ("segmented", segmented),
            ("slider", slider),
            ("field", field),
            ("switch", switch),
        ];
        let clip = rect(0., 0., 400., 300.);
        for (i, (name, control)) in controls.into_iter().enumerate() {
            let size = control.frame().size;
            let slot = NativeControlSlot::new(&gpui, control);
            let mut shown_and_hidden = vec![];
            // Never shown: hidden and not yet in the window.
            assert!(slot.wrapper().isHidden());
            shown_and_hidden.push(("never-shown", slot.picture(size)));
            // Shown by a present.
            slot.stage(rect(10., 10. + 40. * i as f64, size.width, size.height), clip);
            probe::painted(&gpui);
            assert!(probe::place(&gpui));
            assert!(!slot.wrapper().isHidden());
            shown_and_hidden.push(("shown", slot.picture(size)));
            // Hidden for its picture, taken again (a value changed).
            slot.hide();
            probe::painted(&gpui);
            assert!(probe::place(&gpui));
            assert!(slot.wrapper().isHidden());
            shown_and_hidden.push(("hidden", slot.picture(size)));
            // The same picture whether the control was ever shown or not:
            // the window's appearance (dark here) and key state, never a
            // detached control's.
            let first = shown_and_hidden[0].1.as_ref().map(|p| &p.bgra);
            for (when, p) in &shown_and_hidden[1..] {
                assert!(p.as_ref().map(|p| &p.bgra) == first, "{name}: {when} differs from never-shown");
            }
            for (when, picture) in shown_and_hidden {
                let p = picture.unwrap_or_else(|| panic!("{name} {when}: no picture"));
                let (w, h) = (p.width as usize, p.height as usize);
                assert_eq!(p.bgra.len(), w * h * 4);
                // At the window's scale, the control's own size.
                assert!(w as f64 >= size.width && h as f64 >= size.height, "{name} {when}: {w}x{h}");
                let inked = p.bgra.chunks(4).filter(|px| px[3] > 0).count();
                let distinct: std::collections::HashSet<&[u8]> = p.bgra.chunks(4).collect();
                assert!(inked * 20 > w * h && distinct.len() > 4, "{name} {when}: blank ({inked} inked, {} colours)", distinct.len());
                if let Some(dir) = std::env::var_os("PM_PICTURE_DIR") {
                    let mut out = format!("P6 {w} {h} 255\n").into_bytes();
                    for px in p.bgra.chunks(4) {
                        let over = |c: u8| ((c as u16 * px[3] as u16 + 128 * (255 - px[3] as u16)) / 255) as u8;
                        out.extend_from_slice(&[over(px[2]), over(px[1]), over(px[0])]);
                    }
                    std::fs::write(std::path::Path::new(&dir).join(format!("{name}-{when}.ppm")), out).unwrap();
                }
            }
            // Kept hidden: nothing moves.
            slot.keep();
            probe::painted(&gpui);
            assert!(!probe::pending(&gpui), "{name}: a kept hidden slot moves nothing");
        }
        window.close();
    }

    /// The decision a draw makes before it touches a hosted view.
    pub fn policy() {
        use LinkDrawable::{Ready, Stale, Wait};
        // Under the link: its drawable, unless a hosted view would move (a
        // link's drawable is shown at the link's time, not the transaction's)
        // or the link keeps offering the size before a resize.
        assert_eq!(step(Pace::Link, Ready, false, false), Step::Link);
        assert_eq!(step(Pace::Link, Ready, true, false), Step::EnterDirect);
        assert_eq!(step(Pace::Link, Wait, false, false), Step::Defer);
        assert_eq!(step(Pace::Link, Wait, true, false), Step::Defer);
        assert_eq!(step(Pace::Link, Stale, false, false), Step::EnterDirect);
        assert_eq!(step(Pace::Link, Stale, true, false), Step::EnterDirect);
        // Direct: only ever into a drawable already in hand; without one the
        // scene and the views wait together.
        for link in [Ready, Wait, Stale] {
            for pending in [false, true] {
                assert_eq!(step(Pace::Direct, link, pending, true), Step::Direct);
                assert_eq!(step(Pace::Direct, link, pending, false), Step::Defer);
            }
        }
        // No decision draws into a wrong size or asks the layer on the main
        // thread: those would be steps this enum does not have.
        // One stay for a whole scroll: the window is direct until nothing
        // has moved for the linger, no frame waits and no drawable is on
        // its way (a link must not be made while the layer is being asked).
        let short = DIRECT_LINGER - Duration::from_millis(1);
        assert!(!leaves_direct(Duration::ZERO, false, true));
        assert!(!leaves_direct(short, false, true));
        assert!(leaves_direct(DIRECT_LINGER, false, true));
        assert!(!leaves_direct(DIRECT_LINGER, true, true));
        assert!(!leaves_direct(DIRECT_LINGER, false, false));
    }

    /// Drawables come from the layer on another thread; the main thread
    /// takes one that has arrived and has the right size, or none, at once.
    pub fn drawables() {
        let size = |w: f64, h: f64| core_graphics::geometry::CGSize::new(w, h);
        let layer = new_window_layer(false);
        layer.set_device(&metal::Device::system_default().expect("a Metal device"));
        layer.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        layer.set_drawable_size(size(64., 48.));
        let bounded: bool =
            unsafe { msg_send![layer.as_ptr() as *mut AnyObject, allowsNextDrawableTimeout] };
        assert!(bounded, "the layer gives up on a full pool");
        let mut source = DrawableSource::new(layer.clone());
        let prompt = |what: &str, at: Instant| {
            assert!(
                at.elapsed() < Duration::from_millis(100),
                "{what} waited {:?} on the main thread",
                at.elapsed()
            );
        };
        let arrives = |source: &mut DrawableSource, want: (u64, u64)| {
            let asked = Instant::now();
            loop {
                if let Some(drawable) = source.take(want) {
                    return drawable;
                }
                assert!(
                    asked.elapsed() < Duration::from_secs(5),
                    "no drawable arrived"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        // Nothing asked yet: none, at once, and one is on its way.
        let at = Instant::now();
        assert!(source.take((64, 48)).is_none());
        prompt("an empty take", at);
        let first = arrives(&mut source, (64, 48));
        assert_eq!(
            (first.texture().width(), first.texture().height()),
            (64, 48)
        );
        // One of the size before a resize is never handed out: it goes back
        // and one of the new size is asked for.
        source.request();
        let asked = Instant::now();
        while !source.idle() {
            assert!(asked.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        }
        layer.set_drawable_size(size(128., 96.));
        let at = Instant::now();
        assert!(source.take((128, 96)).is_none(), "a 64×48 one is in hand");
        prompt("a take of another size", at);
        let second = arrives(&mut source, (128, 96));
        assert_eq!(second.texture().width(), 128);
        // The layer's pool in use: the wait is the source's, not the caller's.
        let third = arrives(&mut source, (128, 96));
        let at = Instant::now();
        assert!(source.take((128, 96)).is_none());
        assert!(!source.idle(), "asked, and waiting off the main thread");
        prompt("a take from a full pool", at);
        drop(third);
        drop(arrives(&mut source, (128, 96)));
        // What it holds can be given back, so a link may own the layer again.
        source.request();
        let asked = Instant::now();
        while !source.idle() {
            assert!(asked.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        }
        source.release();
        assert!(source.idle());
        drop((first, second));
    }

    /// A control staged by a paint that never reached the screen is not
    /// placed over a later scene that left it out.
    pub fn paints(mtm: MainThreadMarker) {
        let rect =
            |x: f64, y: f64, w: f64, h: f64| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(-20000., -20000., 400., 300.),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let content = window.contentView().expect("a content view");
        let gpui = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
        content.addSubview(&gpui);
        let button: Retained<NSView> = Retained::into_super(Retained::into_super(
            NSButton::initWithFrame(NSButton::alloc(mtm), NSRect::ZERO),
        ));
        let slot = NativeControlSlot::new(&gpui, button);
        let (bounds, clip) = (rect(10., 20., 80., 24.), rect(0., 0., 400., 300.));
        // Paint A stages it. Asking whether a present would move it touches
        // nothing.
        slot.stage(bounds, clip);
        probe::painted(&gpui);
        assert!(probe::pending(&gpui));
        assert!(slot.wrapper().isHidden());
        // Not shown yet, it is not in the window: AppKit lays out none of a
        // long list's controls until a paint shows them (P-SLOT-LAZY).
        assert!(unsafe { slot.wrapper().superview() }.is_none());
        // A never reaches the screen (no drawable). Paint B leaves it out:
        // the frame that shows B does not show it.
        probe::painted(&gpui);
        assert!(!probe::pending(&gpui), "A's staging is not in B's scene");
        assert!(!probe::place(&gpui));
        assert!(slot.wrapper().isHidden());
        // Paint C stages it and is presented: it stands where C put it
        // (AppKit's y runs up from the bottom of the 300 pt content).
        slot.stage(bounds, clip);
        probe::painted(&gpui);
        assert!(probe::pending(&gpui));
        assert!(probe::place(&gpui));
        assert!(!slot.wrapper().isHidden());
        assert!(unsafe { slot.wrapper().superview() }.is_some_and(|v| v == content));
        assert_eq!(slot.wrapper().frame(), rect(10., 256., 80., 24.));
        // Paint D reuses a cached view: it keeps its place and nothing moves.
        slot.keep();
        probe::painted(&gpui);
        assert!(!probe::pending(&gpui));
        assert!(!probe::place(&gpui));
        assert!(!slot.wrapper().isHidden());
        // Two paints before one present (a draw nobody showed, then the one
        // that is shown): the later one decides. It moved the control.
        slot.stage(rect(10., 60., 80., 24.), clip);
        probe::painted(&gpui);
        slot.stage(rect(10., 100., 80., 24.), clip);
        probe::painted(&gpui);
        assert!(probe::place(&gpui));
        assert_eq!(slot.wrapper().frame(), rect(10., 176., 80., 24.));
        // Paint E leaves it out: the present hides it.
        probe::painted(&gpui);
        assert!(probe::pending(&gpui));
        assert!(probe::place(&gpui));
        assert!(slot.wrapper().isHidden());
        drop(slot);
        probe::forget(&gpui);
    }

    /// A pop-up whose rows the host lists (Parent: every layer) shows only
    /// its current row until its menu is about to show; then it lists them
    /// all with the current row still selected, and a choice is that row.
    pub fn host_rows() {
        let row = |v: serde_json::Value, label: &str| json!({ "v": v, "label": label });
        let shown = [row(json!(null), "None"), row(json!("b"), "Layer B")];
        let rows = PopupRows::new(&shown);
        rows.popup.selectItemAtIndex(1);
        let asked = std::rc::Rc::new(std::cell::Cell::new(0));
        let source = {
            let asked = asked.clone();
            std::rc::Rc::new(move || {
                asked.set(asked.get() + 1);
                let mut all = vec![json!({ "v": null, "label": "None" })];
                for id in ["a", "b", "c", "d"] {
                    all.push(json!({ "v": id, "label": format!("Layer {}", id.to_uppercase()) }));
                }
                all
            }) as pm_shell::native_control::RowSource
        };
        rows.fill_rows(Some(source));
        assert_eq!(
            rows.popup.numberOfItems(),
            2,
            "nothing is listed before the menu opens"
        );
        assert_eq!(asked.get(), 0);
        rows.menu_will_show();
        assert_eq!(asked.get(), 1);
        assert_eq!(rows.popup.numberOfItems(), 5);
        assert_eq!(
            rows.popup.indexOfSelectedItem(),
            2,
            "Layer B stays selected"
        );
        rows.popup.selectItemAtIndex(4);
        rows.act();
        assert_eq!(rows.chosen.try_recv().ok(), Some(json!("d")));
        // The next publication shows the closed button's rows again.
        rows.update(
            &[row(json!(null), "None"), row(json!("d"), "Layer D")],
            &json!("d"),
        );
        assert_eq!(rows.popup.numberOfItems(), 2);
        // A source that no longer offers the current row keeps it listed.
        let gone = std::rc::Rc::new(|| vec![json!({ "v": null, "label": "None" })])
            as pm_shell::native_control::RowSource;
        rows.popup.selectItemAtIndex(1);
        rows.fill_rows(Some(gone));
        rows.menu_will_show();
        assert_eq!(rows.popup.numberOfItems(), 2);
        assert_eq!(rows.popup.indexOfSelectedItem(), 1);
    }

    /// A pop-up's rows while its menu is open.
    pub fn rows() {
        let row = |v: serde_json::Value, label: &str| json!({ "v": v, "label": label });
        let options = [
            row(json!("fit"), "Fit (38 %)"),
            row(json!(0.5), "50 %"),
            row(json!(1), "100 %"),
        ];
        let rows = PopupRows::new(&options);
        let items = || -> Vec<usize> {
            (0..rows.popup.numberOfItems())
                .map(|i| Retained::as_ptr(&rows.popup.itemAtIndex(i).unwrap()) as usize)
                .collect()
        };
        let title = |i: isize| rows.popup.itemAtIndex(i).unwrap().title().to_string();
        let menu = rows.popup.menu().expect("a menu");
        let post = |name: &objc2_foundation::NSNotificationName| unsafe {
            NSNotificationCenter::defaultCenter().postNotificationName_object(name, Some(&menu));
        };
        let turn = || {
            NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
        };
        let shown = items();
        assert_eq!(shown.len(), 3);
        unsafe { post(NSMenuDidBeginTrackingNotification) };
        // The zoom row names the current zoom: a new title, the same items
        // (the one under the pointer stays the one under the pointer).
        let renamed = [
            row(json!("fit"), "Fit (40 %)"),
            row(json!(0.5), "50 %"),
            row(json!(1), "100 %"),
        ];
        rows.update(&renamed, &json!("fit"));
        assert_eq!(items(), shown);
        assert_eq!(title(0), "Fit (40 %)");
        // Other rows while the menu is open: its items stay, and choosing
        // one still means the row it showed.
        let longer = [
            row(json!("new"), "New"),
            row(json!("fit"), "Fit (40 %)"),
            row(json!(0.5), "50 %"),
            row(json!(1), "100 %"),
        ];
        rows.update(&longer, &json!(1));
        assert_eq!(items(), shown);
        rows.popup.selectItemAtIndex(1);
        unsafe { post(NSMenuDidEndTrackingNotification) };
        assert_eq!(
            items(),
            shown,
            "the rows stay until the action has gone out"
        );
        rows.act();
        assert_eq!(rows.chosen.try_recv().ok(), Some(json!(0.5)));
        // The next turn of the run loop shows the new rows, with the
        // publication's value selected.
        turn();
        assert_eq!(rows.popup.numberOfItems(), 4);
        assert_eq!(title(0), "New");
        assert_eq!(rows.popup.indexOfSelectedItem(), 3);
        rows.popup.selectItemAtIndex(0);
        rows.act();
        assert_eq!(rows.chosen.try_recv().ok(), Some(json!("new")));
        // A closed menu takes other rows at once.
        rows.update(&options, &json!("fit"));
        assert_eq!(rows.popup.numberOfItems(), 3);
        // Reopened before the pending rows were shown: they wait again.
        unsafe { post(NSMenuDidBeginTrackingNotification) };
        rows.update(&longer, &json!("fit"));
        unsafe { post(NSMenuDidEndTrackingNotification) };
        unsafe { post(NSMenuDidBeginTrackingNotification) };
        turn();
        assert_eq!(rows.popup.numberOfItems(), 3);
        unsafe { post(NSMenuDidEndTrackingNotification) };
        turn();
        assert_eq!(rows.popup.numberOfItems(), 4);
        // Two rows of one title are two rows.
        rows.update(&[row(json!(1), "Same"), row(json!(2), "Same")], &json!(2));
        assert_eq!(rows.popup.numberOfItems(), 2);
        rows.popup.selectItemAtIndex(1);
        rows.act();
        assert_eq!(rows.chosen.try_recv().ok(), Some(json!(2)));
    }

    /// A window that opens on Home hides its viewer slot before the first
    /// project frame gives it a transform. The hide reaches the Metal layer
    /// (still the view's backing layer); clipping then makes a root the
    /// view's layer, so showing the view alone left the picture hidden (an
    /// empty viewer and the viewer's ground). Showing the slot shows it.
    pub fn viewer_slot(mtm: MainThreadMarker) {
        use pm_shell::viewer_slot;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(400., 300.)),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let content = window.contentView().expect("a content view");
        let hidden = |layer: &metal::MetalLayer| -> bool {
            unsafe { msg_send![layer.as_ptr() as *mut AnyObject, isHidden] }
        };
        let slot = || {
            let view = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
            view.setWantsLayer(true);
            let layer = metal::MetalLayer::new();
            unsafe {
                let _: () = msg_send![&*view, setLayer: layer.as_ptr() as *mut AnyObject];
            }
            content.addSubview(&view);
            (view, layer)
        };
        // The trap, as AppKit has it: the view's hide stays on the layer.
        let (view, layer) = slot();
        view.setHidden(true);
        assert!(hidden(&layer), "hiding a layer-backed view hides its layer");
        viewer_slot::clip(&view, &layer, 0);
        view.setHidden(false);
        assert!(hidden(&layer), "the clipped picture stays hidden");
        // Home first, then a project: the picture shows.
        let (view, layer) = slot();
        viewer_slot::show(&view, &layer, false);
        viewer_slot::clip(&view, &layer, 0);
        viewer_slot::show(&view, &layer, true);
        assert!(!view.isHidden() && !hidden(&layer), "the picture shows");
        let root = view.layer().expect("the clipping root");
        let sub: *mut AnyObject =
            unsafe { msg_send![layer.as_ptr() as *mut AnyObject, superlayer] };
        assert_eq!(sub, Retained::as_ptr(&root) as *mut AnyObject);
        // And hidden again with Home.
        viewer_slot::show(&view, &layer, false);
        assert!(view.isHidden());
        window.close();
    }
}
