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
    macos::hidden_moves(mtm);
    macos::regions(mtm);
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

    /// P-SLOT-REGION: slots in a scrolling region sit in its container at
    /// their content position; a step moves the container only (one view,
    /// whatever the number of slots), in the slot transaction, and each slot
    /// lands on the window position its row is drawn at. Nested regions nest
    /// their containers. `PM_REGION_SLOTS=<n>` sets how many slots the timing
    /// uses (default 300) and prints the per-step cost both ways.
    pub fn regions(mtm: MainThreadMarker) {
        use gpui::{Bounds, SlotRegionGeometry, point, px, size};
        use gpui_macos::slot_region::probe as regions;
        let rect =
            |x: f64, y: f64, w: f64, h: f64| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect(-20000., -20000., 400., 600.),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        let content = window.contentView().expect("a content view");
        let gpui = NSView::initWithFrame(NSView::alloc(mtm), content.bounds());
        content.addSubview(&gpui);
        let button = || -> Retained<NSView> {
            unsafe {
                let raw: *mut AnyObject = msg_send![objc2::class!(NSButton), alloc];
                let raw: *mut NSView = msg_send![raw, initWithFrame: rect(0., 0., 80., 20.)];
                Retained::from_raw(raw).expect("a button")
            }
        };
        // Where a slot shows, in GPUI's top-down window points.
        let shown = |slot: &NativeControlSlot| {
            let w = slot.wrapper();
            let r = w.convertRect_toView(w.bounds(), Some(&gpui));
            let h = gpui.bounds().size.height;
            (r.origin.x, h - r.origin.y - r.size.height)
        };
        let region = |id, parent, frame: (f32, f32, f32, f32), origin_y: f32| SlotRegionGeometry {
            id,
            parent,
            frame: Bounds::new(point(px(frame.0), px(frame.1)), size(px(frame.2), px(frame.3))),
            origin: point(px(0.), px(origin_y)),
            fade: [px(0.), px(0.)],
        };
        let open = rect(-1.0e6, -1.0e6, 2.0e6, 2.0e6);
        let count: usize = std::env::var("PM_REGION_SLOTS").ok().and_then(|v| v.parse().ok()).unwrap_or(300);
        let slots: Vec<_> = (0..count).map(|_| NativeControlSlot::new(&gpui, button())).collect();
        // Region 1: a panel body at (0, 50), 200 x 300, scrolled to the top.
        let mut offset = 0f32;
        let stage = |slots: &[NativeControlSlot]| {
            for (i, slot) in slots.iter().enumerate() {
                slot.stage_in_region(1, rect(10., 40. * i as f64, 80., 20.), open);
            }
        };
        regions::set(&gpui, &[region(1, None, (0., 50., 200., 300.), offset)]);
        stage(&slots);
        probe::painted(&gpui);
        assert!(probe::place(&gpui), "the first frame shows them");
        let clip = regions::clip(&gpui, 1).expect("one container for the region");
        for (i, slot) in slots.iter().enumerate().take(12) {
            assert_eq!(shown(slot), (10., 50. + 40. * i as f64), "slot {i} at rest");
            let parent = unsafe { slot.wrapper().superview() }.expect("in the window");
            assert!(parent.isDescendantOf(&clip), "slot {i} is in its region's container");
        }
        // Wheel steps: the slots are kept as painted; only the region moves.
        let frames: Vec<_> = slots.iter().map(|s| s.wrapper().frame()).collect();
        for dy in [-12f32, -30., 17., -25., -333.] {
            offset += dy;
            regions::set(&gpui, &[region(1, None, (0., 50., 200., 300.), offset)]);
            for slot in &slots {
                slot.keep();
            }
            probe::painted(&gpui);
            assert!(probe::pending(&gpui), "a moving region is a move, known before");
            assert!(probe::place(&gpui), "a moving region presents with the transaction");
            for (i, slot) in slots.iter().enumerate().take(12) {
                let y = 50. + 40. * i as f64 + f64::from(offset);
                assert_eq!(shown(slot), (10., y), "slot {i} after a step of {dy}");
            }
            let same = slots.iter().zip(&frames).all(|(s, f)| s.wrapper().frame() == *f);
            assert!(same, "no slot's own frame changed: the step moved one view");
            // The next frame, at the same offset (another view drew).
            for slot in &slots {
                slot.keep();
            }
            probe::painted(&gpui);
            assert!(!probe::pending(&gpui) && !probe::place(&gpui), "a frame that does not scroll moves nothing");
        }
        // Nested: region 2 sits at (0, 80) in region 1's content, 200 x 100.
        let inner = NativeControlSlot::new(&gpui, button());
        let (mut outer_y, mut inner_y) = (0f32, 0f32);
        let set = |outer_y, inner_y| {
            regions::set(
                &gpui,
                &[
                    region(1, None, (0., 50., 200., 300.), outer_y),
                    region(2, Some(1), (0., 80., 200., 100.), inner_y),
                ],
            )
        };
        for (outer_step, inner_step) in [(0., 0.), (-20., 0.), (0., -15.), (7., -9.)] {
            outer_y += outer_step;
            inner_y += inner_step;
            set(outer_y, inner_y);
            inner.stage_in_region(2, rect(10., 60., 80., 20.), open);
            for slot in &slots {
                slot.keep();
            }
            probe::painted(&gpui);
            probe::place(&gpui);
            let y = 50. + f64::from(outer_y) + 80. + f64::from(inner_y) + 60.;
            assert_eq!(shown(&inner), (10., y), "nested slot after ({outer_step}, {inner_step})");
        }
        let inner_clip = regions::clip(&gpui, 2).expect("a container for the inner region");
        assert!(inner_clip.isDescendantOf(&clip), "the inner container is in the outer's content");
        // A frame that stages no slot of a region hides its container.
        regions::set(&gpui, &[region(1, None, (0., 50., 200., 300.), outer_y)]);
        probe::painted(&gpui);
        assert!(probe::place(&gpui));
        assert!(clip.isHidden(), "a region with no slot shown hides");
        // Per-step cost with `count` slots: the region's step, against every
        // slot moved by its own frame (the path before P-SLOT-REGION).
        let mut region_ms = Vec::new();
        let mut each_ms = Vec::new();
        for step in 0..60 {
            let y = -(step as f32) * 7.;
            regions::set(&gpui, &[region(1, None, (0., 50., 200., 300.), y)]);
            stage(&slots);
            probe::painted(&gpui);
            let at = Instant::now();
            probe::place(&gpui);
            region_ms.push(at.elapsed().as_secs_f64() * 1e3);
        }
        for step in 0..61 {
            let y = -(step as f32) * 7.;
            for (i, slot) in slots.iter().enumerate() {
                slot.stage(rect(10., 50. + 40. * i as f64 + f64::from(y), 80., 20.), rect(0., 50., 200., 300.));
            }
            probe::painted(&gpui);
            let at = Instant::now();
            probe::place(&gpui);
            // The first takes every slot out of the container.
            if step > 0 {
                each_ms.push(at.elapsed().as_secs_f64() * 1e3);
            }
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        println!(
            "slot regions: {count} slots, per step p50 {:.3} ms moving the region, {:.3} ms moving each slot",
            median(&mut region_ms),
            median(&mut each_ms)
        );
        drop(slots);
        drop(inner);
        window.close();
    }

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
