//! Scrollable viewport for the project and page tabs in the toolbar.
use super::{TAB_CLEAR, TAB_GAP, TAB_LEADING, TAB_TOOLS, mask_stop};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{NSClipView, NSColor, NSEvent, NSScrollElasticity, NSScrollView, NSView};
use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CAGradientLayer, CATransaction};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

const EDGE_FADE: f64 = 18.;
const DRAG_EDGE: f64 = 24.;
const DRAG_STEP: f64 = 10.;
const WHEEL_STEP: f64 = 18.;

#[derive(Clone, Copy, Debug)]
pub(super) struct TabLayout {
    pub document: f64,
    pub viewport: f64,
    pub menu: bool,
}

impl TabLayout {
    pub fn room(window: f64, tools: f64, leading: f64, control: f64) -> f64 {
        (window / 2.
            - tools.max(TAB_TOOLS) / 2.
            - TAB_CLEAR
            - leading.max(TAB_LEADING)
            - 2. * (control + TAB_GAP))
            .max(0.)
            .floor()
    }

    pub fn new(widths: impl Iterator<Item = f64>, room: f64, control: f64) -> Self {
        let document = (widths.map(|width| width + TAB_GAP).sum::<f64>() - TAB_GAP).max(0.);
        let menu = document > room && room > control + TAB_GAP;
        let viewport = if menu { room - control - TAB_GAP } else { room };
        Self {
            document,
            viewport: viewport.min(document),
            menu,
        }
    }

    fn limit(self) -> f64 {
        (self.document - self.viewport).max(0.)
    }

    fn fade(self, offset: f64) -> (f64, f64) {
        let edge = EDGE_FADE.min(self.viewport / 2.);
        (
            offset.clamp(0., edge),
            (self.limit() - offset).clamp(0., edge),
        )
    }

    fn reveal(self, offset: f64, tab: NSRect) -> f64 {
        let left = tab.origin.x;
        let right = left + tab.size.width;
        let inset = EDGE_FADE.min(((self.viewport - tab.size.width) / 2.).max(0.));
        let offset = if left < offset + inset {
            left - inset
        } else if right > offset + self.viewport - inset {
            right - self.viewport + inset
        } else {
            offset
        };
        offset.clamp(0., self.limit())
    }

    fn drag_step(self, x: f64) -> f64 {
        let edge = DRAG_EDGE.min(self.viewport / 2.);
        if edge <= 0. || self.limit() <= 0. {
            return 0.;
        }
        if x < edge {
            -DRAG_STEP * ((edge - x) / edge).clamp(0., 1.)
        } else {
            DRAG_STEP * ((x - (self.viewport - edge)) / edge).clamp(0., 1.)
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ScrollPosition {
    x: f64,
    active: Option<isize>,
}
thread_local! {
    // The selected NSWindow changes when a tab is clicked; its strip
    // continues from the same horizontal offset as the outgoing window.
    static POSITIONS: RefCell<HashMap<isize, ScrollPosition>> = RefCell::default();
}

pub(super) struct ScrollState {
    fade: Retained<CAGradientLayer>,
    last_fade: Cell<Option<(NSSize, f64, f64)>>,
    layout: TabLayout,
    group: Cell<Option<isize>>,
    active: Option<(isize, NSRect)>,
}

define_class!(
    #[unsafe(super(NSScrollView))]
    #[thread_kind = MainThreadOnly]
    #[name = "PMShellTabScroll"]
    #[ivars = ScrollState]
    pub(super) struct TabScroll;

    impl TabScroll {
        #[unsafe(method(reflectScrolledClipView:))]
        fn reflect(&self, clip: &NSClipView) {
            unsafe { let _: () = msg_send![super(self), reflectScrolledClipView: clip]; }
            self.update_fade(clip);
            if let Some(group) = self.ivars().group.get()
                && self.window().is_some_and(|w| {
                    w.tabGroup().is_none_or(|g| g.selectedWindow().as_deref() == Some(&*w))
                })
            {
                POSITIONS.with_borrow_mut(|positions| {
                    positions.entry(group).or_default().x = clip.bounds().origin.x;
                });
            }
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            if event.hasPreciseScrollingDeltas() || self.ivars().layout.limit() == 0. {
                // Trackpad phases, direction and momentum belong to AppKit.
                unsafe { let _: () = msg_send![super(self), scrollWheel: event]; }
            } else {
                // A mouse with one wheel can scroll this horizontal strip.
                let x = event.scrollingDeltaX();
                let delta = if x == 0. { event.scrollingDeltaY() } else { x };
                self.scroll_to(self.offset() - WHEEL_STEP * delta);
            }
        }
    }
);

impl TabScroll {
    pub fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        layout: TabLayout,
        document: &NSView,
        active: Option<(isize, NSRect)>,
    ) -> Retained<Self> {
        let fade = CAGradientLayer::new();
        fade.setStartPoint(NSPoint::new(0., 0.5));
        fade.setEndPoint(NSPoint::new(1., 0.5));
        let scroll: Retained<Self> = unsafe {
            msg_send![
                super(Self::alloc(mtm).set_ivars(ScrollState {
                    fade,
                    last_fade: Cell::new(None),
                    layout,
                    group: Cell::new(None),
                    active,
                })),
                initWithFrame: frame
            ]
        };
        scroll.setDrawsBackground(false);
        scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        scroll.setHasHorizontalScroller(false);
        scroll.setHasVerticalScroller(false);
        scroll.setHorizontalScrollElasticity(NSScrollElasticity::None);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        scroll.setDocumentView(Some(document));
        scroll.setWantsLayer(true);
        if let Some(layer) = scroll.layer() {
            layer.setMasksToBounds(true);
        }
        scroll.update_fade(&scroll.contentView());
        scroll
    }

    fn update_fade(&self, clip: &NSClipView) {
        let bounds = self.bounds();
        let (left, right) = self.ivars().layout.fade(clip.bounds().origin.x);
        let Some(layer) = self.layer() else { return };
        let fade_geometry = (bounds.size, left, right);
        if self.ivars().last_fade.replace(Some(fade_geometry)) == Some(fade_geometry) {
            return;
        }
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        let fade = &self.ivars().fade;
        if left > 0. || right > 0. {
            let solid = mask_stop(&NSColor::blackColor());
            let clear = mask_stop(&NSColor::clearColor());
            let near = if left > 0. {
                clear.clone()
            } else {
                solid.clone()
            };
            let far = if right > 0. { clear } else { solid.clone() };
            fade.setFrame(NSRect::new(NSPoint::ZERO, bounds.size));
            unsafe {
                fade.setColors(Some(&NSArray::from_retained_slice(&[
                    near,
                    solid.clone(),
                    solid,
                    far,
                ])));
                fade.setLocations(Some(&NSArray::from_retained_slice(&[
                    NSNumber::new_f64(0.),
                    NSNumber::new_f64(left / bounds.size.width),
                    NSNumber::new_f64(1. - right / bounds.size.width),
                    NSNumber::new_f64(1.),
                ])));
                layer.setMask(Some(fade));
            }
        } else {
            unsafe { layer.setMask(None) };
        }
        CATransaction::commit();
    }

    pub fn offset(&self) -> f64 {
        self.contentView().bounds().origin.x
    }

    fn scroll_to(&self, x: f64) {
        let x = x.clamp(0., self.ivars().layout.limit());
        if x != self.offset() {
            let clip = self.contentView();
            clip.scrollToPoint(NSPoint::new(x, 0.));
            self.reflectScrolledClipView(&clip);
        }
    }

    pub fn restore(&self, group: isize, resized: bool) {
        let previous = POSITIONS
            .with_borrow(|positions| positions.get(&group).copied())
            .unwrap_or_default();
        let active = self.ivars().active;
        let id = active.map(|(id, _)| id);
        let offset = match active {
            Some((_, frame)) if previous.active != id || resized => {
                self.ivars().layout.reveal(previous.x, frame)
            }
            _ => previous.x,
        };
        self.ivars().group.set(Some(group));
        self.scroll_to(offset);
        POSITIONS.with_borrow_mut(|positions| {
            positions.insert(
                group,
                ScrollPosition {
                    x: self.offset(),
                    active: id,
                },
            );
        });
    }

    pub fn drag_step_at(&self, at: NSPoint) -> f64 {
        let local = self.convertPoint_fromView(at, None);
        let layout = self.ivars().layout;
        if !(-DRAG_EDGE..layout.viewport + DRAG_EDGE).contains(&local.x) {
            return 0.;
        }
        let by = layout.drag_step(local.x);
        let offset = self.offset();
        (offset + by).clamp(0., layout.limit()) - offset
    }

    pub fn drag_scroll(&self, by: f64) {
        self.scroll_to(self.offset() + by);
    }

    pub fn find(strip: &NSView) -> Option<Retained<Self>> {
        strip
            .subviews()
            .to_vec()
            .into_iter()
            .find_map(|v| v.downcast().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(x: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, 0.), NSSize::new(100., 28.))
    }

    #[test]
    fn three_tabs_fit_at_their_own_widths() {
        let room = TabLayout::room(1512., 300., 92., 28.);
        let layout = TabLayout::new([80., 100., 140.].into_iter(), room, 28.);
        assert_eq!(layout.viewport, 328.);
        assert_eq!(layout.document, layout.viewport);
        assert!(!layout.menu);
        assert_eq!(layout.fade(0.), (0., 0.));
    }

    #[test]
    fn twelve_tabs_fade_only_where_the_list_continues() {
        let room = TabLayout::room(1512., 300., 92., 28.);
        let layout = TabLayout::new([100.; 12].into_iter(), room, 28.);
        assert!(layout.menu);
        assert_eq!(
            layout.viewport + 28. + TAB_GAP,
            room,
            "leave room for the pinned menu"
        );
        assert_eq!(layout.document, 1244.);
        assert_eq!(layout.fade(0.), (0., EDGE_FADE));
        let offset = layout.reveal(0., tab(5. * 104.));
        assert_eq!(layout.fade(offset), (EDGE_FADE, EDGE_FADE));
        assert_eq!(layout.reveal(offset, tab(4. * 104.)), offset);
    }

    #[test]
    fn thirty_tabs_can_reach_the_last_active_tab_under_the_same_width_limit() {
        let room = TabLayout::room(1512., 300., 92., 28.);
        let layout = TabLayout::new([100.; 30].into_iter(), room, 28.);
        let offset = layout.reveal(0., tab(29. * 104.));
        assert_eq!(layout.document, 3116.);
        assert_eq!(offset + layout.viewport, layout.document);
        assert_eq!(layout.fade(offset), (EDGE_FADE, 0.));
        assert_eq!(layout.reveal(offset, tab(0.)), 0.);
        let far_edge = 92. + 2. * (28. + TAB_GAP) + layout.viewport + 28. + TAB_GAP;
        assert_eq!(far_edge, 1512. / 2. - 300. / 2. - TAB_CLEAR);
    }

    #[test]
    fn edge_drag_speed_ramps_toward_both_edges() {
        let layout = TabLayout::new([100.; 12].into_iter(), 360., 28.);
        assert_eq!(layout.drag_step(layout.viewport / 2.), 0.);
        assert_eq!(layout.drag_step(DRAG_EDGE / 2.), -DRAG_STEP / 2.);
        assert_eq!(
            layout.drag_step(layout.viewport - DRAG_EDGE / 2.),
            DRAG_STEP / 2.
        );
    }
}
