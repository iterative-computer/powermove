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
            let image = cx.capture_screenshot(window.into()).unwrap();
            // Inspect the otherwise empty right edge of each visible panel body.
            // A column thumb would change these pixels while scrolling.
            let metrics = Tokens::new(Scheme::Dark, Accessibility::default()).metrics;
            let scale = image.width() / 1000;
            let x = image.width() - (metrics.dock_gap as u32 + 4) * scale;
            let panels = window
                .update(&mut cx, |editor, _, _| {
                    ["inspector", "easing", "fxbrowser"].map(|id| editor.panel_rect(id).unwrap())
                })
                .unwrap();
            for y in (100..550).step_by(20) {
                if panels
                    .iter()
                    .any(|p| y as f32 > p.y + metrics.header + 20. && (y as f32) < p.y + p.h - 20.)
                {
                    assert_eq!(
                        image.get_pixel(x, y * scale),
                        image.get_pixel(x - 8 * scale, y * scale),
                        "no column scrollbar thumb at {name}:{y}"
                    );
                }
            }
            image
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
    observed: usize,
    inner: ScrollHandle,
    outer: ScrollHandle,
}
impl Render for Nested {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .on_scroll_wheel(cx.listener(|v, _, _, _| v.observed += 1))
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
                    observed: 0,
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
        cx.update_window(window.into(), |_, w, cx| {
            w.dispatch_event(wheel(-50.), cx);
        })
        .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, cx| {
                assert_eq!(v.observed, 1, "inner wheel observers still run");
                assert_eq!(v.inner.offset().y, px(-50.));
                assert_eq!(v.outer.offset().y, px(0.));
                v.inner.scroll_to_bottom();
                cx.notify();
            })
            .unwrap();
        cx.capture_screenshot(window.into()).unwrap();
        cx.update_window(window.into(), |_, w, cx| {
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
        cx.update_window(window.into(), |_, w, cx| {
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
        cx.update_window(window.into(), |_, window, _| window.activate())
            .unwrap();
        cx.run_until_parked();
        // Move through the actual native panel focus handles, one rendered frame each.
        let mut reached_last = false;
        for _ in 0..20 {
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
            reached_last = window
                .update(&mut cx, |editor, _, _| {
                    let last = editor.panel_rect("fxbrowser").unwrap();
                    last.y + last.h <= 600.
                })
                .unwrap();
            if reached_last {
                break;
            }
        }
        assert!(reached_last, "keyboard traversal reveals the final panel");
        window
            .update(&mut cx, |editor, _, _| {
                // The traversal reached the lower right groups, so the column moved.
                assert!(editor.scroll_dock("right", -1.));
            })
            .unwrap();
    });
}

#[test]
fn normal_height_content_floors_capture_before_and_after() {
    objc2::rc::autoreleasepool(|_| {
        let mut cx = context();
        let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
        let header = tokens.metrics.header;
        let window = cx
            .open_window(size(px(1248.), px(705.)), |_, cx| {
                cx.new(|cx| {
                    let mut editor = Editor::new(tokens, None, cx);
                    editor.layout = Layout::preset("design");
                    let right = editor
                        .layout
                        .docks
                        .iter_mut()
                        .find(|d| d.id == "right")
                        .unwrap();
                    right.groups = ["inspector", "easing", "fxbrowser"]
                        .into_iter()
                        .map(|id| Group {
                            tabs: vec![id.into()],
                            active: 0,
                            size: None,

                            collapsed: false,
                        })
                        .collect();
                    editor
                })
            })
            .unwrap();
        cx.run_until_parked();
        let dir = std::path::PathBuf::from(
            std::env::var("PM_DOCK_CAPTURE_DIR").unwrap_or_else(|_| "/tmp/pm-dock-scroll".into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let mut records = vec![];
        // The before case recreates .15's 200-point-only sizing, where panel
        // content demand was not connected to the column. The layout and its
        // saved sizes stay identical in both captures.
        for (name, natural) in [
            ("before", [200., 200., 200.]),
            ("after", [600., 360., 2000.]),
        ] {
            window
                .update(&mut cx, |editor, w, cx| {
                    for (id, height) in ["inspector", "easing", "fxbrowser"]
                        .into_iter()
                        .zip(natural)
                    {
                        editor.command(
                            Command {
                                name: format!("dockContent:{id}"),
                                args: vec![serde_json::json!(height - header)],
                                token: None,
                            },
                            w,
                            cx,
                        );
                    }
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(dir.join(format!("dark-705-{name}.png")))
                .unwrap();
            let (layout, heights, moves) = window
                .update(&mut cx, |editor, _, _| {
                    let heights: Vec<_> = ["inspector", "easing", "fxbrowser"]
                        .into_iter()
                        .map(|id| (id, editor.panel_rect(id).unwrap().h))
                        .collect();
                    (
                        editor.layout.clone(),
                        heights,
                        editor.scroll_dock("right", 10000.),
                    )
                })
                .unwrap();
            assert!(
                layout
                    .docks
                    .iter()
                    .find(|d| d.id == "right")
                    .unwrap()
                    .groups
                    .iter()
                    .all(|g| g.size.is_none()),
                "automatic measurement never changes saved sizing"
            );
            assert_eq!(moves, name == "after");
            assert!(heights.iter().all(|(_, height)| *height >= 200.));
            records.push(serde_json::json!({"case":name,"window":[1248,705],"natural":natural,"assigned":heights,"column_overflows":moves}));
        }
        window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();
        cx.capture_screenshot(window.into())
            .unwrap()
            .save(dir.join("dark-705-after-bottom.png"))
            .unwrap();
        std::fs::write(
            dir.join("normal-height-metrics.json"),
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .unwrap();
    });
}

#[test]
fn wheel_over_dock_edge_and_panel_gap_scrolls_the_column() {
    for side in ["left", "right"] {
        objc2::rc::autoreleasepool(|_| {
            let mut cx = context();
            let window = cx
                .open_window(size(px(1000.), px(900.)), |_, cx| {
                    cx.new(|cx| {
                        let mut editor = Editor::new(
                            Tokens::new(Scheme::Dark, Accessibility::default()),
                            None,
                            cx,
                        );
                        tall(&mut editor);
                        if side == "left" {
                            let left = editor
                                .layout
                                .docks
                                .iter()
                                .position(|d| d.id == "left")
                                .unwrap();
                            let right = editor
                                .layout
                                .docks
                                .iter()
                                .position(|d| d.id == "right")
                                .unwrap();
                            let groups = editor.layout.docks[left].groups.clone();
                            editor.layout.docks[left].groups =
                                editor.layout.docks[right].groups.clone();
                            editor.layout.docks[right].groups = groups;
                        }
                        editor
                    })
                })
                .unwrap();
            cx.run_until_parked();
            let panel = window
                .update(&mut cx, |editor, _, _| {
                    editor.panel_rect("inspector").unwrap()
                })
                .unwrap();
            let gap = Tokens::new(Scheme::Dark, Accessibility::default())
                .metrics
                .dock_gap;
            let edge = if side == "left" {
                panel.x + panel.w + gap / 2.
            } else {
                panel.x - gap / 2.
            };
            for (x, y) in [
                (edge, panel.y + 80.),
                (panel.x + 30., panel.y + panel.h + gap / 2.),
            ] {
                window
                    .update(&mut cx, |editor, _, cx| {
                        editor.scroll_dock(side, -10000.);
                        cx.notify();
                    })
                    .unwrap();
                cx.run_until_parked();
                let before = window
                    .update(&mut cx, |editor, _, _| {
                        editor.panel_rect("inspector").unwrap().y
                    })
                    .unwrap();
                let event = ScrollWheelEvent {
                    position: point(px(x), px(y)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
                    modifiers: Default::default(),
                    touch_phase: Default::default(),
                }
                .to_platform_input();
                cx.update_window(window.into(), |_, w, cx| {
                    w.dispatch_event(event, cx);
                })
                .unwrap();
                cx.run_until_parked();
                let after = window
                    .update(&mut cx, |editor, _, _| {
                        editor.panel_rect("inspector").unwrap().y
                    })
                    .unwrap();
                assert!(
                    after < before,
                    "wheel over edge or gap moves the whole column at {x},{y}"
                );
            }
            window
                .update(&mut cx, |editor, _, cx| {
                    editor.scroll_dock(side, 10000.);
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            let before = window
                .update(&mut cx, |editor, _, _| {
                    editor.panel_rect("inspector").unwrap().y
                })
                .unwrap();
            let event = ScrollWheelEvent {
                position: point(px(edge), px(panel.y + 80.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
                modifiers: Default::default(),
                touch_phase: Default::default(),
            }
            .to_platform_input();
            let result = cx
                .update_window(window.into(), |_, w, cx| w.dispatch_event(event, cx))
                .unwrap();
            cx.run_until_parked();
            assert!(
                !result.propagate,
                "the gutter keeps the wheel at the column limit"
            );
            let after = window
                .update(&mut cx, |editor, _, _| {
                    editor.panel_rect("inspector").unwrap().y
                })
                .unwrap();
            assert_eq!(after, before);
            if let Ok(dir) = std::env::var("PM_DOCK_CAPTURE_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                cx.capture_screenshot(window.into())
                    .unwrap()
                    .save(std::path::Path::new(&dir).join(format!("{side}-edge-scroll.png")))
                    .unwrap();
            }
        });
    }
}
