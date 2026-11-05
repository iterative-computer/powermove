#![cfg(target_os = "macos")]
//! P-120 regression: parent invalidation reuses a cached subtree without rendering it again.
use gpui::{
    AppContext, Context, Entity, HeadlessAppContext, IntoElement, Render, StyleRefinement, Window,
    div, prelude::*, px, rgb, size,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
struct StaticView(Rc<Cell<usize>>);
impl Render for StaticView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.set(self.0.get() + 1);
        div().size_full().bg(rgb(0x339966))
    }
}
struct Parent {
    child: Entity<StaticView>,
}
impl Render for Parent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut style = StyleRefinement::default();
        style.size.width = Some(px(80.).into());
        style.size.height = Some(px(80.).into());
        div().size_full().child(self.child.clone().cached(style))
    }
}
#[test]
fn unchanged_subtree_is_reused_and_notification_invalidates_it() {
    objc2::rc::autoreleasepool(|_| {
        let count = Rc::new(Cell::new(0));
        let mut cx = HeadlessAppContext::with_platform(
            Arc::new(gpui_parley::ParleyTextSystem::new(
                gpui_parley::SystemFonts::Load,
            )),
            Arc::new(()),
            || {
                Some(Box::new(
                    gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                ))
            },
        );
        let window = cx
            .open_window(size(px(100.), px(100.)), |_, cx| {
                cx.new(|cx| Parent {
                    child: cx.new(|_| StaticView(count.clone())),
                })
            })
            .unwrap();
        cx.run_until_parked();
        let first = cx.capture_screenshot(window.into()).unwrap();
        let initial = count.get();
        assert!(initial > 0);
        for _ in 0..12 {
            window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
            cx.run_until_parked();
            assert_eq!(cx.capture_screenshot(window.into()).unwrap(), first);
        }
        assert_eq!(
            count.get(),
            initial,
            "parent frames must not re-render an unchanged view"
        );
        window
            .update(&mut cx, |parent, _, cx| {
                parent.child.update(cx, |_, cx| cx.notify())
            })
            .unwrap();
        cx.run_until_parked();
        let _ = cx.capture_screenshot(window.into()).unwrap();
        assert!(
            count.get() > initial,
            "a child notification must invalidate its cache"
        );
    });
}
