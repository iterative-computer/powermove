//! Powermove P-SLOT-REGION: one clipping container per scrolling region that
//! holds hosted views (`gpui::slot_region`). A region's slots are children
//! of its container's content view, placed in the region's content
//! coordinates; a scroll moves only the content view's origin, in the same
//! slot transaction as the frame that draws the scrolled content. Nested
//! regions get nested containers. A container shows only while one of its
//! slots (or of its nested regions' slots) is placed.
use gpui::SlotRegionGeometry;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{NSResponder, NSView, NSWindowOrderingMode};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

define_class!(
    /// A region's content view: top-down coordinates, and no pointer event of
    /// its own (a press between its slots reaches GPUI's view).
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSlotRegionContent"]
    struct RegionContent;
    unsafe impl NSObjectProtocol for RegionContent {}
    impl RegionContent {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            let hit: *mut NSView = unsafe { msg_send![super(self), hitTest: point] };
            if std::ptr::eq(hit, self as *const Self as *const NSView) { std::ptr::null_mut() } else { hit }
        }
    }
);
define_class!(
    /// A region's clip (with its edge fades as the layer's mask), passing
    /// presses through as its content view does.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMSlotRegionClip"]
    struct RegionClip;
    unsafe impl NSObjectProtocol for RegionClip {}
    impl RegionClip {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            let hit: *mut NSView = unsafe { msg_send![super(self), hitTest: point] };
            if std::ptr::eq(hit, self as *const Self as *const NSView) { std::ptr::null_mut() } else { hit }
        }
    }
);
macro_rules! region_view {
    ($class:ty, $mtm:expr) => {{
        let this = <$class>::alloc($mtm).set_ivars(());
        let view: Retained<$class> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        view.setWantsLayer(true);
        view
    }};
}
/// Where a paint put a slot: its bounds and clip, in GPUI's top-down window
/// points, or in region `region`'s content coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub bounds: NSRect,
    pub clip: NSRect,
    pub region: Option<usize>,
}

/// What shows of region `region` as the last draw placed it, in its own
/// content coordinates: its clip, cut by every region around it (each at its
/// own offset). `None` when the last draw made no such region.
pub(crate) fn viewport(gpui: &NSView, region: usize) -> Option<NSRect> {
    let view = gpui as *const NSView as usize;
    HOSTS.with_borrow(|hosts| viewport_in(&hosts.get(&view)?.targets, region))
}
fn viewport_in(targets: &HashMap<usize, SlotRegionGeometry>, id: usize) -> Option<NSRect> {
    let t = targets.get(&id)?;
    let a = applied(t);
    let own = NSRect::new(NSPoint::new(-a.origin.x, -a.origin.y), a.frame.size);
    let Some(parent) = t.parent else {
        return Some(own);
    };
    // The parent's view, moved into this region's content coordinates.
    let p = viewport_in(targets, parent)?;
    let p = NSRect::new(
        NSPoint::new(
            p.origin.x - a.frame.origin.x - a.origin.x,
            p.origin.y - a.frame.origin.y - a.origin.y,
        ),
        p.size,
    );
    Some(intersect(own, p))
}
pub(crate) fn intersect(a: NSRect, b: NSRect) -> NSRect {
    let x = a.origin.x.max(b.origin.x);
    let y = a.origin.y.max(b.origin.y);
    let w = (a.origin.x + a.size.width).min(b.origin.x + b.size.width) - x;
    let h = (a.origin.y + a.size.height).min(b.origin.y + b.size.height) - y;
    NSRect::new(NSPoint::new(x, y), NSSize::new(w.max(0.), h.max(0.)))
}
/// Whether `rect` (content coordinates of `region`) shows any of itself.
pub(crate) fn shows(gpui: &NSView, region: usize, rect: NSRect) -> bool {
    viewport(gpui, region).is_some_and(|v| {
        let r = intersect(v, rect);
        r.size.width > 0. && r.size.height > 0.
    })
}

/// Converts `rect` (in `region`'s content coordinates) into top-down GPUI
/// window coordinates, clipped by every enclosing region's viewport.
pub(crate) fn window_rect(gpui: &NSView, region: usize, rect: NSRect) -> Option<NSRect> {
    let view = gpui as *const NSView as usize;
    HOSTS.with_borrow(|hosts| {
        let targets = &hosts.get(&view)?.targets;
        let mut r = rect;
        let mut current = Some(region);
        while let Some(id) = current {
            let t = targets.get(&id)?;
            let a = applied(t);
            r = NSRect::new(
                NSPoint::new(
                    r.origin.x + a.frame.origin.x + a.origin.x,
                    r.origin.y + a.frame.origin.y + a.origin.y,
                ),
                r.size,
            );
            r = intersect(r, a.frame);
            if r.size.width <= 0. || r.size.height <= 0. {
                return None;
            }
            current = t.parent;
        }
        Some(r)
    })
}

/// The content view of region `region` for a slot: placed by this present
/// (`apply`), or only as it stands (`None` when not made yet).
pub(crate) fn content_view(gpui: &NSView, region: usize, apply: bool) -> Option<Retained<NSView>> {
    if apply {
        return content(gpui, region);
    }
    let view = gpui as *const NSView as usize;
    HOSTS.with_borrow(|hosts| {
        hosts
            .get(&view)?
            .containers
            .get(&region)
            .filter(|c| !c.clip.isHidden())
            .map(|c| c.content.clone().into_super())
    })
}

/// Whether a slot's wrapper is somewhere other than where it belongs: in
/// `parent` (a region's content view), or in GPUI's view's parent.
pub(crate) fn misplaced(gpui: &NSView, wrapper: &NSView, parent: Option<&NSView>) -> bool {
    let Some(current) = (unsafe { wrapper.superview() }) else {
        return false;
    };
    // Held where it is for an edit in progress (`reparent`): not a move.
    if holds_first_responder(wrapper) {
        return false;
    }
    match parent {
        Some(parent) => !std::ptr::eq(&*current, parent),
        None => unsafe { gpui.superview() }.is_some_and(|home| !std::ptr::eq(&*current, &*home)),
    }
}

/// Puts a slot's wrapper in `parent` (a region's content view), or in GPUI's
/// view's parent when `parent` is `None` and it sits elsewhere, and says
/// where its `frame` (in that destination's coordinates) goes. A view moves
/// between parents of a window by being added to the new one: AppKit keeps an
/// edit in progress then (`removeFromSuperview` would end it). A wrapper that
/// holds the window's first responder (a field being edited) stays in its
/// parent until the edit ends, at the same place on screen: its frame comes
/// back converted into that parent. Returns whether it changed parent.
///
/// P-SLOT-LAZY: a wrapper not in the window yet joins it only when `shown`
/// (its region's view shows it); until then it stays out, its frame kept.
pub(crate) fn reparent(
    gpui: &NSView,
    wrapper: &NSView,
    parent: Option<&NSView>,
    frame: NSRect,
    shown: bool,
) -> (bool, NSRect) {
    let current = unsafe { wrapper.superview() };
    let home = unsafe { gpui.superview() };
    let Some(target) = parent.or(home.as_deref()) else {
        return (false, frame);
    };
    match current {
        Some(current) if std::ptr::eq(&*current, target) => (false, frame),
        Some(current) if holds_first_responder(wrapper) => {
            (false, target.convertRect_toView(frame, Some(&current)))
        }
        // A wrapper not in the window yet joins it where its paint shows it
        // (outside a region, `attach` decides when).
        None if parent.is_none() || !shown => (false, frame),
        _ => {
            adopt(target, wrapper, parent.is_none().then_some(gpui));
            (true, frame)
        }
    }
}

/// Adds `view` to `parent`, above GPUI's view when `above` is it.
fn adopt(parent: &NSView, view: &NSView, above: Option<&NSView>) {
    match above {
        Some(gpui) => parent.addSubview_positioned_relativeTo(
            view,
            NSWindowOrderingMode::Above,
            Some(gpui),
        ),
        None => parent.addSubview(view),
    }
}

/// Whether the window's first responder is `view` or inside it (a field's
/// editor is a subview of its field).
pub(crate) fn holds_first_responder(view: &NSView) -> bool {
    let Some(responder) = view.window().and_then(|w| w.firstResponder()) else {
        return false;
    };
    responder
        .downcast::<NSView>()
        .is_ok_and(|responder| responder.isDescendantOf(view))
}

/// A content view's size: its slots stay inside it for hit testing at any
/// offset a panel reaches.
const CONTENT_EXTENT: f64 = 1.0e5;

#[derive(Clone, Copy, PartialEq)]
struct Applied {
    frame: NSRect,
    origin: NSPoint,
    fade: [f64; 2],
}
struct Container {
    /// Unique for the process: a slot settled in a container that was
    /// dropped is placed again in its successor.
    serial: u64,
    clip: Retained<RegionClip>,
    content: Retained<RegionContent>,
    parent: Option<usize>,
    applied: Option<Applied>,
    fade: crate::scroll_fade::ScrollFade,
}
#[derive(Default)]
struct Host {
    targets: HashMap<usize, SlotRegionGeometry>,
    containers: HashMap<usize, Container>,
    /// Containers placed by this present (a slot of theirs is shown).
    used: HashSet<usize>,
    moved: bool,
}
thread_local! {
    static HOSTS: RefCell<HashMap<usize, Host>> = RefCell::new(HashMap::new());
    static SERIAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The serial of region `region`'s container, if made.
pub(crate) fn serial(gpui: &NSView, region: usize) -> Option<u64> {
    let view = gpui as *const NSView as usize;
    HOSTS.with_borrow(|hosts| Some(hosts.get(&view)?.containers.get(&region)?.serial))
}

/// The regions GPUI's last draw made for the view keyed `view`.
pub(crate) fn set(view: usize, regions: &[SlotRegionGeometry]) {
    HOSTS.with_borrow_mut(|hosts| {
        let host = hosts.entry(view).or_default();
        host.targets.clear();
        host.targets.extend(regions.iter().map(|r| (r.id, *r)));
    });
}

fn ns(b: gpui::Bounds<gpui::Pixels>) -> NSRect {
    NSRect::new(
        NSPoint::new(f32::from(b.origin.x).into(), f32::from(b.origin.y).into()),
        NSSize::new(f32::from(b.size.width).into(), f32::from(b.size.height).into()),
    )
}
fn applied(target: &SlotRegionGeometry) -> Applied {
    Applied {
        frame: ns(target.frame),
        origin: NSPoint::new(
            f32::from(target.origin.x).into(),
            f32::from(target.origin.y).into(),
        ),
        fade: target.fade.map(|f| f32::from(f).into()),
    }
}

/// The content view a slot of region `region` goes into, placing the
/// region's containers (outermost first) the first time this present asks.
/// `None` when the last draw made no such region: the slot is hidden.
pub(crate) fn content(gpui: &NSView, region: usize) -> Option<Retained<NSView>> {
    let view = gpui as *const NSView as usize;
    HOSTS.with_borrow_mut(|hosts| place(hosts.get_mut(&view)?, gpui, region))
}

fn place(host: &mut Host, gpui: &NSView, id: usize) -> Option<Retained<NSView>> {
    if host.used.contains(&id) {
        return host.containers.get(&id).map(|c| c.content.clone().into_super());
    }
    let target = *host.targets.get(&id)?;
    let parent_content = match target.parent {
        Some(parent) => Some(place(host, gpui, parent)?),
        None => None,
    };
    let mtm = MainThreadMarker::new().expect("slot regions are main-thread");
    let container = host.containers.entry(id).or_insert_with(|| {
        let clip = region_view!(RegionClip, mtm);
        if let Some(layer) = clip.layer() {
            layer.setMasksToBounds(true);
        }
        let content = region_view!(RegionContent, mtm);
        content.setFrame(NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(CONTENT_EXTENT, CONTENT_EXTENT),
        ));
        clip.addSubview(&content);
        Container {
            serial: SERIAL.with(|s| {
                s.set(s.get() + 1);
                s.get()
            }),
            clip,
            content,
            parent: None,
            applied: None,
            fade: Default::default(),
        }
    });
    // In its parent region's content, or GPUI's view's parent (P-EMBED).
    let superview = unsafe { container.clip.superview() };
    let wanted = parent_content.or_else(|| unsafe { gpui.superview() })?;
    if superview.as_deref().is_none_or(|s| !std::ptr::eq(s, &*wanted))
        || container.parent != target.parent
    {
        // Added, not removed first: an edit inside keeps going (see `reparent`).
        adopt(&wanted, &container.clip, target.parent.is_none().then_some(gpui));
        container.parent = target.parent;
        container.applied = None;
        host.moved = true;
    }
    let next = applied(&target);
    if container.applied != Some(next) {
        let frame = match target.parent {
            Some(_) => next.frame,
            None => crate::composition::hosted_rect(gpui, next.frame),
        };
        if container.clip.frame() != frame {
            container.clip.setFrame(frame);
        }
        // The clip runs up from its bottom edge; the content view's top-left
        // sits at the region's content origin.
        let origin = NSPoint::new(
            next.origin.x,
            next.frame.size.height - next.origin.y - CONTENT_EXTENT,
        );
        if container.content.frame().origin != origin {
            container.content.setFrameOrigin(origin);
        }
        let bounds = NSRect::new(NSPoint::new(0., 0.), next.frame.size);
        container.fade.stage(next.fade);
        container.fade.place(&container.clip, bounds, bounds, true);
        container.applied = Some(next);
        host.moved = true;
    }
    if container.clip.isHidden() {
        container.clip.setHidden(false);
        host.moved = true;
    }
    host.used.insert(id);
    Some(container.content.clone().into_super())
}

/// Ends a present: containers with no slot shown hide, those whose region is
/// gone go. Returns whether any container moved, showed or hid.
pub(crate) fn finish(view: usize) -> bool {
    HOSTS.with_borrow_mut(|hosts| {
        let Some(host) = hosts.get_mut(&view) else {
            return false;
        };
        let used = std::mem::take(&mut host.used);
        let targets = &host.targets;
        let mut moved = std::mem::take(&mut host.moved);
        host.containers.retain(|id, container| {
            // A field being edited inside stays shown until the edit ends
            // (hiding its ancestor would end it): it is held there by
            // `reparent` while its row moves elsewhere.
            if used.contains(id) || holds_first_responder(&container.clip) {
                return true;
            }
            if !container.clip.isHidden() {
                container.clip.setHidden(true);
                moved = true;
            }
            if targets.contains_key(id) {
                return true;
            }
            container.clip.removeFromSuperview();
            false
        });
        moved
    })
}

/// Whether placing the containers as the last draw asks would move one
/// (nothing is touched). A region whose container is not made yet is found
/// by its slot (its view is not in that container yet).
pub(crate) fn pending(view: usize) -> bool {
    HOSTS.with_borrow(|hosts| {
        hosts.get(&view).is_some_and(|host| {
            host.containers.iter().any(|(id, c)| {
                !c.clip.isHidden()
                    && host
                        .targets
                        .get(id)
                        .is_some_and(|t| c.applied != Some(applied(t)))
            })
        })
    })
}

/// The window closes (its views go with it; nothing is touched here, as the
/// window's lock is held).
pub(crate) fn forget(view: usize) {
    HOSTS.with_borrow_mut(|hosts| hosts.remove(&view));
}

/// Screen-free probes for contract tests outside this crate.
#[doc(hidden)]
pub mod probe {
    /// Hands `regions` to the view as a draw would.
    pub fn set(view: &objc2_app_kit::NSView, regions: &[gpui::SlotRegionGeometry]) {
        super::set(view as *const _ as usize, regions);
    }
    /// The container's clip view of `region`, if made.
    pub fn clip(view: &objc2_app_kit::NSView, region: usize) -> Option<objc2::rc::Retained<objc2_app_kit::NSView>> {
        super::HOSTS.with_borrow(|hosts| {
            hosts
                .get(&(view as *const _ as usize))?
                .containers
                .get(&region)
                .map(|c| c.clip.clone().into_super())
        })
    }
}
