#![cfg(target_os = "macos")]
use gpui::{
    AppContext, Context, HeadlessAppContext, InputEvent, IntoElement, Render, ScrollDelta,
    ScrollHandle, ScrollWheelEvent, Window, div, point, prelude::*, px, size,
};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    layout::{Group, Layout},
    native::Command,
    view::Editor,
};
use std::sync::Arc;
fn context() -> HeadlessAppContext {
    HeadlessAppContext::with_platform(
        Arc::new(gpui_parley::ParleyTextSystem::new(
            gpui_parley::SystemFonts::Load,
        )),
        Arc::new(()),
        || {
            Some(Box::new(
                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    )
}
fn tall(editor: &mut Editor) {
    editor.layout = Layout::preset("design");
    let right = editor
        .layout
        .docks
        .iter_mut()
        .find(|d| d.id == "right")
        .unwrap();
    right.groups = [
        vec!["inspector", "notes"],
        vec!["easing"],
        vec!["fxbrowser"],
    ]
    .into_iter()
    .map(|tabs| Group {
        tabs: tabs.into_iter().map(str::to_owned).collect(),
        active: 0,
        size: Some(400.),
        collapsed: false,
    })
    .collect();
}
#[test]
fn overflowing_dock_reaches_last_panel_and_captures_three_positions() {
    objc2::rc::autoreleasepool(|_| {
        let mut cx = context();
        let window = cx
            .open_window(size(px(1000.), px(600.)), |_, cx| {
                cx.new(|cx| {
                    let mut editor = Editor::new(
                        Tokens::new(Scheme::Dark, Accessibility::default()),
                        None,
                        cx,
                    );
                    tall(&mut editor);
                    editor
                })
            })
            .unwrap();
        cx.run_until_parked();
        let dir = std::path::PathBuf::from(
            std::env::var("PM_DOCK_CAPTURE_DIR").unwrap_or_else(|_| "/tmp/pm-dock-scroll".into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        for (name, by) in [("top", 0.), ("middle", 350.), ("bottom", 10000.)] {
            window
                .update(&mut cx, |editor, _, cx| {
                    if by > 0. {
                        assert!(editor.scroll_dock("right", by));
                    }
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(dir.join(format!("dark-right-{name}.png")))
                .unwrap();
        }
        window
            .update(&mut cx, |editor, _, _| {
                assert!(!editor.scroll_dock("right", 100.));
                let last = editor.panel_rect("fxbrowser").unwrap();
                assert!(last.y >= 0. && last.y + last.h <= 600.);
            })
            .unwrap();
    });
}
#[test]
fn short_dock_does_not_scroll() {
    objc2::rc::autoreleasepool(|_| {
        let mut cx = context();
        let window = cx
            .open_window(size(px(1400.), px(1600.)), |_, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Tokens::new(Scheme::Dark, Accessibility::default()),
                        None,
                        cx,
                    )
                })
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |editor, _, _| {
                assert!(!editor.scroll_dock("right", 100.));
            })
            .unwrap();
    });
}
#[test]
fn hosted_keyboard_focus_reveals_panel() {
    objc2::rc::autoreleasepool(|_| {
        let mut cx = context();
        let window = cx
            .open_window(size(px(1000.), px(600.)), |_, cx| {
                cx.new(|cx| {
                    let mut e = Editor::new(
                        Tokens::new(Scheme::Dark, Accessibility::default()),
                        None,
                        cx,
                    );
                    tall(&mut e);
                    e
                })
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |editor, w, cx| {
                editor.command(
                    Command {
                        name: "dockReveal:fxbrowser".into(),
                        args: vec![0.into(), 24.into()],
                        token: None,
                    },
                    w,
                    cx,
                )
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |editor, _, _| {
                assert!(editor.scroll_dock("right", -1.))
            })
            .unwrap();
    });
}
struct Nested {
    inner: ScrollHandle,
    outer: ScrollHandle,
}
impl Render for Nested {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("outer")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.outer)
            .child(
                div()
                    .id("inner")
                    .w_full()
                    .h(px(100.))
                    .overflow_y_scroll()
                    .track_scroll(&self.inner)
                    .child(div().h(px(400.)).w_full()),
            )
            .child(div().h(px(500.)).w_full())
    }
}
#[test]
fn nested_wheel_hands_off_at_both_edges() {
    objc2::rc::autoreleasepool(|_| {
        let mut cx = context();
        let window = cx
            .open_window(size(px(200.), px(200.)), |_, cx| {
                cx.new(|_| Nested {
                    inner: ScrollHandle::new(),
                    outer: ScrollHandle::new(),
                })
            })
            .unwrap();
        cx.run_until_parked();
        let wheel = |dy| {
            ScrollWheelEvent {
                position: point(px(25.), px(25.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                modifiers: Default::default(),
                touch_phase: Default::default(),
            }
            .to_platform_input()
        };
        window
            .update(&mut cx, |_, w, cx| {
                w.dispatch_event(wheel(-50.), cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, cx| {
                assert_eq!(v.inner.offset().y, px(-50.));
                assert_eq!(v.outer.offset().y, px(0.));
                v.inner.scroll_to_bottom();
                cx.notify();
            })
            .unwrap();
        cx.capture_screenshot(window.into()).unwrap();
        window
            .update(&mut cx, |_, w, cx| {
                w.dispatch_event(wheel(-50.), cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, cx| {
                assert_eq!(v.outer.offset().y, px(-50.));
                v.inner.set_offset(point(px(0.), px(0.)));
                cx.notify();
            })
            .unwrap();
        cx.capture_screenshot(window.into()).unwrap();
        window
            .update(&mut cx, |_, w, cx| {
                w.dispatch_event(wheel(25.), cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, _| assert_eq!(v.outer.offset().y, px(-25.)))
            .unwrap();
    });
}

#[test]
fn native_keyboard_panel_navigation_reveals_last_panel() {
    objc2::rc::autoreleasepool(|_| {
        let mut cx = context();
        let window = cx
            .open_window(size(px(1000.), px(600.)), |_, cx| {
                cx.new(|cx| {
                    let mut editor = Editor::new(
                        Tokens::new(Scheme::Dark, Accessibility::default()),
                        None,
                        cx,
                    );
                    tall(&mut editor);
                    editor
                })
            })
            .unwrap();
        cx.run_until_parked();
        // Move through the actual native panel focus handles, one rendered frame each.
        for _ in 0..8 {
            window
                .update(&mut cx, |editor, w, cx| {
                    editor.command(
                        Command {
                            name: "shellPanelNext".into(),
                            args: vec![],
                            token: None,
                        },
                        w,
                        cx,
                    )
                })
                .unwrap();
            cx.run_until_parked();
        }
        window
            .update(&mut cx, |editor, _, _| {
                // The traversal reached the lower right groups, so the column moved.
                assert!(editor.scroll_dock("right", -1.));
            })
            .unwrap();
    });
}
