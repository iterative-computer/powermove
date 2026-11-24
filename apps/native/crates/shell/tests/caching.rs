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

struct TextHeader(Rc<Cell<usize>>);
impl Render for TextHeader {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.set(self.0.get() + 1);
        div().size_full().child("Threads")
    }
}

struct WorkspaceHost {
    header: Entity<TextHeader>,
}
impl Render for WorkspaceHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut style = StyleRefinement::default();
        style.size.width = Some(px(120.).into());
        style.size.height = Some(px(28.).into());
        div()
            .size_full()
            .child("Workspace")
            .child(self.header.clone().cached(style))
    }
}

struct DragShell {
    workspace: Entity<WorkspaceHost>,
    dragging: bool,
    chip: &'static str,
}
impl Render for DragShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let workspace = if self.dragging {
            self.workspace
                .clone()
                .cached(div().size_full().style().clone())
                .into_any_element()
        } else {
            self.workspace.clone().into_any_element()
        };
        div()
            .size_full()
            .child(self.chip)
            .child(workspace)
    }
}

#[test]
fn un_caching_an_outer_view_does_not_reuse_stale_inner_line_layouts() {
    objc2::rc::autoreleasepool(|_| {
        let header_renders = Rc::new(Cell::new(0));
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
            .open_window(size(px(200.), px(120.)), |_, cx| {
                let header = cx.new(|_| TextHeader(header_renders.clone()));
                let workspace = cx.new(|_| WorkspaceHost { header });
                cx.new(|_| DragShell {
                    workspace,
                    dragging: false,
                    chip: "Idle",
                })
            })
            .unwrap();
        cx.run_until_parked();
        let _ = cx.capture_screenshot(window.into()).unwrap();

        // Start drag: outer view becomes cached and renders once with extra text ahead of it.
        window
            .update(&mut cx, |shell, _, cx| {
                shell.dragging = true;
                shell.chip = "Dragging Threads panel";
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let _ = cx.capture_screenshot(window.into()).unwrap();

        // Continue drag: outer view hits its cache, keeping inner ViewElementState alive
        // while shifting LineLayoutCache indices.
        window
            .update(&mut cx, |shell, _, cx| {
                shell.chip = "Move";
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let _ = cx.capture_screenshot(window.into()).unwrap();

        // Drop panel: outer view stops caching and re-renders directly.
        // Previously this panicked in LineLayoutCache::reuse_layouts with slice_index_fail.
        let before_drop = header_renders.get();
        window
            .update(&mut cx, |shell, _, cx| {
                shell.dragging = false;
                shell.chip = "";
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let _ = cx.capture_screenshot(window.into()).unwrap();
        assert!(
            header_renders.get() > before_drop,
            "inner cached view must refresh after an enclosing cached ancestor reused its state"
        );
    });
}

struct Label(&'static str, Rc<Cell<usize>>);
impl Render for Label {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.1.set(self.1.get() + 1);
        div().size_full().child(self.0)
    }
}

/// Registers a font in its prepaint, once asked: between its cached
/// neighbours' prepaints, with no text shaped in between.
struct FontShell {
    before: Entity<Label>,
    load: Rc<Cell<bool>>,
    after: Entity<Label>,
}
impl Render for FontShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut style = StyleRefinement::default();
        style.size.width = Some(px(120.).into());
        style.size.height = Some(px(20.).into());
        let load = self.load.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(["One", "Two", "Three", "Four"])
            .child(self.before.clone().cached(style.clone()))
            .child(
                gpui::canvas(
                    move |_, _, cx| {
                        if load.replace(false) {
                            let font = include_bytes!(
                                "../../../vendor/gpui-ce/crates/gpui/test-fixtures/IBMPlexSans-Regular.ttf"
                            );
                            cx.text_system()
                                .add_fonts(vec![std::borrow::Cow::Borrowed(&font[..])])
                                .unwrap();
                        }
                    },
                    |_, _, _, _| {},
                )
                .w(px(10.))
                .h(px(10.)),
            )
            .child(self.after.clone().cached(style))
            .child("Tail")
    }
}

/// w83 review P0-4: a font registered between two cached views clears the
/// layout cache mid-frame; the next frames must not replay a range recorded
/// before that clear (it panicked slicing the layout list), and a stable
/// redraw after it draws the same pixels.
#[test]
fn a_font_registered_between_cached_views_never_replays_a_cleared_range() {
    objc2::rc::autoreleasepool(|_| {
        let renders = Rc::new(Cell::new(0));
        let load = Rc::new(Cell::new(false));
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
            .open_window(size(px(200.), px(200.)), |_, cx| {
                let before = cx.new(|_| Label("Before", renders.clone()));
                let after = cx.new(|_| Label("After", renders.clone()));
                cx.new(|_| FontShell {
                    before,
                    load: load.clone(),
                    after,
                })
            })
            .unwrap();
        let redraw = |cx: &mut HeadlessAppContext| {
            window.update(cx, |_, _, cx| cx.notify()).unwrap();
            cx.run_until_parked();
            cx.capture_screenshot(window.into()).unwrap()
        };
        redraw(&mut cx);
        redraw(&mut cx);
        load.set(true);
        let _ = redraw(&mut cx);
        assert!(!load.get(), "the font was registered in prepaint");
        let settled = redraw(&mut cx);
        for _ in 0..4 {
            assert_eq!(redraw(&mut cx), settled, "stable geometry draws the same pixels");
        }
        let cached = renders.get();
        redraw(&mut cx);
        assert_eq!(renders.get(), cached, "the cached views replay again once settled");
    });
}
