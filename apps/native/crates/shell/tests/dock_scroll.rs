#![cfg(target_os = "macos")]
use gpui::{
    AppContext, Context, HeadlessAppContext, InputEvent, IntoElement, Render, ScrollDelta,
    ScrollHandle, ScrollWheelEvent, Window, div, point, prelude::*, px, size,
};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    Command,
    layout::{Group, Layout},
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
                is_momentum: false,
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
fn nested_wheel_crossing_an_edge_hands_off_only_the_remainder() {
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
        window
            .update(&mut cx, |v, _, cx| {
                v.inner.set_offset(point(px(0.), px(-280.)));
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, w, cx| {
            w.dispatch_event(
                ScrollWheelEvent {
                    position: point(px(25.), px(25.)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(-50.))),
                    modifiers: Default::default(),
                    touch_phase: Default::default(),
                    is_momentum: false,
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, _| {
                assert_eq!(v.inner.offset().y, px(-300.));
                assert_eq!(
                    v.outer.offset().y,
                    px(-30.),
                    "20 inner + 30 outer = one 50px wheel event"
                );
            })
            .unwrap();
        // A redraw with no new input must retain both offsets.
        cx.capture_screenshot(window.into()).unwrap();
        window
            .update(&mut cx, |v, _, _| {
                assert_eq!(v.inner.offset().y, px(-300.));
                assert_eq!(v.outer.offset().y, px(-30.));
            })
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
                (panel.x + 4., panel.y + 80.), // panel padding
                (panel.x + 80., panel.y + 8.), // panel header
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
                    is_momentum: false,
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
                assert!(
                    (after - (before - 30.)).abs() < 0.01,
                    "one-for-one wheel distance"
                );
                for _ in 0..8 {
                    cx.update_window(window.into(), |_, w, cx| {
                        w.dispatch_event(
                            ScrollWheelEvent {
                                position: point(px(x), px(y)),
                                delta: ScrollDelta::Pixels(point(px(0.), px(-0.25))),
                                modifiers: Default::default(),
                                touch_phase: Default::default(),
                                is_momentum: true,
                            }
                            .to_platform_input(),
                            cx,
                        );
                    })
                    .unwrap();
                    cx.run_until_parked();
                }
                cx.capture_screenshot(window.into()).unwrap();
                window
                    .update(&mut cx, |v, _, _| {
                        assert!(
                            (v.panel_rect("inspector").unwrap().y - (before - 32.)).abs() < 0.01,
                            "fractional momentum survives redraw without jump-back"
                        );
                    })
                    .unwrap();
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
                is_momentum: false,
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

#[test]
fn sticky_column_header_pins_and_is_pushed_headlessly() {
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
        let top = window
            .update(&mut cx, |e, _, _| e.panel_rect("inspector").unwrap().y)
            .unwrap();
        window
            .update(&mut cx, |e, _, cx| {
                assert!(e.scroll_dock("right", 100.));
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |e, _, _| {
                assert_eq!(e.panel_header_rect("inspector").unwrap().y, top);
                assert_eq!(e.panel_rect("inspector").unwrap().y, top - 100.);
            })
            .unwrap();
        let by = window
            .update(&mut cx, |e, _, _| {
                let next = e.panel_rect("easing").unwrap().y;
                next - top
                    - Tokens::new(Scheme::Dark, Accessibility::default())
                        .metrics
                        .header
                        / 2.
            })
            .unwrap();
        window
            .update(&mut cx, |e, _, cx| {
                assert!(e.scroll_dock("right", by));
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |e, _, _| {
                let h = Tokens::new(Scheme::Dark, Accessibility::default())
                    .metrics
                    .header;
                assert_eq!(
                    e.panel_header_rect("inspector").unwrap().y,
                    e.panel_header_rect("easing").unwrap().y - h
                );
            })
            .unwrap();
    });
}

#[test]
fn sticky_header_click_switches_tabs_headlessly() {
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
                    let right = e.layout.docks.iter().position(|d| d.id == "right").unwrap();
                    let left = e.layout.docks.iter().position(|d| d.id == "left").unwrap();
                    e.layout.docks[left].groups = e.layout.docks[right].groups.clone();
                    e.layout.docks[left].groups[0].tabs =
                        vec!["assets".into(), "layer-effects".into()];
                    e.layout.docks[left].groups[0].active = 1;
                    e.layout.docks[right].groups.truncate(1);
                    e
                })
            })
            .unwrap();
        cx.run_until_parked();
        let top = window
            .update(&mut cx, |e, _, _| e.panel_rect("layer-effects").unwrap())
            .unwrap();
        window
            .update(&mut cx, |e, _, cx| {
                e.scroll_dock("left", 100.);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let position = point(px(top.x + 45.), px(top.y + 14.));
        cx.update_window(window.into(), |_, w, cx| {
            w.dispatch_event(
                gpui::MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    click_count: 1,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            w.dispatch_event(
                gpui::MouseUpEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    click_count: 1,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |e, _, _| {
                let (d, g, _) = e.layout.find("assets").unwrap();
                assert_eq!(e.layout.docks[d].groups[g].active(), "assets");
            })
            .unwrap();
    });
}

#[test]
fn sticky_header_light_and_dark_crops() {
    let Ok(dir) = std::env::var("PM_DOCK_CAPTURE_DIR") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for (name, scheme) in [("light", Scheme::Light), ("dark", Scheme::Dark)] {
        objc2::rc::autoreleasepool(|_| {
            let mut cx = context();
            let window = cx.open_window(size(px(1000.), px(600.)), |_, cx| {
                cx.new(|cx| {
                    let mut e = Editor::new(Tokens::new(scheme, Accessibility::default()), None, cx);
                    tall(&mut e);
                    e
                })
            }).unwrap();
            cx.run_until_parked();
            let panel = window.update(&mut cx, |e, _, _| e.panel_rect("inspector").unwrap()).unwrap();
            let header = Tokens::new(scheme, Accessibility::default()).metrics.header;
            for state in ["top", "pinned", "push", "next"] {
                let by = match state {
                    "pinned" => 100.,
                    "push" => window.update(&mut cx, |e, _, _| {
                        e.panel_rect("easing").unwrap().y - panel.y - header / 2.
                    }).unwrap(),
                    "next" => header,
                    _ => 0.,
                };
                window.update(&mut cx, |e, _, cx| { e.scroll_dock("right", by); cx.notify(); }).unwrap();
                cx.run_until_parked();
                let image = cx.capture_screenshot(window.into()).unwrap();
                let scale = image.width() as f32 / 1000.;
                image::imageops::crop_imm(&image, (panel.x * scale) as u32, 0,
                    (panel.w * scale) as u32, image.height()).to_image()
                    .save(std::path::Path::new(&dir).join(format!("{name}-sticky-{state}.png"))).unwrap();
            }
        });
    }
}

/// Nothing of a scrolling dock shows under the toolbar, and the toolbar is
/// the chrome's one tone: the same ground as the status bar (the title bar
/// paints no band of its own, so a light window has two surface tones).
#[test]
fn toolbar_stays_ground_above_scrolling_dock() {
    let mut mismatches = Vec::new();
    for (name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
        objc2::rc::autoreleasepool(|_| {
            let tokens = Tokens::new(scheme, Accessibility::default());
            let mut cx = context();
            let window = cx
                .open_window(size(px(1000.), px(600.)), |_, cx| {
                    cx.new(|cx| {
                        let mut editor = Editor::new(tokens, None, cx);
                        tall(&mut editor);
                        editor
                    })
                })
                .unwrap();
            cx.run_until_parked();
            let mut first = None;
            for (position, by) in [("top", 0.), ("middle", 350.), ("bottom", 10000.)] {
                window
                    .update(&mut cx, |v, _, cx| {
                        v.scroll_dock("right", by);
                        cx.notify();
                    })
                    .unwrap();
                cx.run_until_parked();
                let image = cx.capture_screenshot(window.into()).unwrap();
                let scale = image.width() / 1000;
                if let Ok(dir) = std::env::var("PM_DOCK_CAPTURE_DIR") {
                    std::fs::create_dir_all(&dir).unwrap();
                    image
                        .save(
                            std::path::Path::new(&dir)
                                .join(format!("{name}-toolbar-{position}.png")),
                        )
                        .unwrap();
                }
                let toolbar = image.get_pixel(950 * scale, 8 * scale).0;
                let ground = image.get_pixel(500 * scale, 592 * scale).0;
                if toolbar != ground {
                    mismatches.push(format!(
                        "{name}:{position}: toolbar {toolbar:?} != ground {ground:?}"
                    ));
                }
                if *first.get_or_insert(toolbar) != toolbar {
                    mismatches.push(format!(
                        "{name}:{position}: the dock shows through {toolbar:?}"
                    ));
                }
            }
        });
    }
    assert!(
        mismatches.is_empty(),
        "toolbar must be the ground: {mismatches:?}"
    );
}

#[test]
fn library_show_expands_selects_and_reveals_existing_tab() {
    objc2::rc::autoreleasepool(|_| {
        let mut cx = context();
        let window = cx
            .open_window(size(px(1000.), px(600.)), |_, cx| {
                cx.new(|cx| {
                    let mut editor = Editor::new(
                        Tokens::new(
                            Scheme::Dark,
                            Accessibility {
                                reduce_motion: true,
                                ..Default::default()
                            },
                        ),
                        None,
                        cx,
                    );
                    tall(&mut editor);
                    let (di, gi, _) = editor.layout.find("notes").unwrap();
                    editor.layout.docks[di].collapsed = true;
                    editor.layout.docks[di].groups[gi].collapsed = true;
                    editor
                })
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, cx| {
                assert_eq!(v.show_library_panel("notes", cx).unwrap(), "right");
                let (di, gi, ti) = v.layout.find("notes").unwrap();
                assert!(!v.layout.docks[di].collapsed);
                assert!(!v.layout.docks[di].groups[gi].collapsed);
                assert_eq!(v.layout.docks[di].groups[gi].active, ti);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, cx| {
                v.show_library_panel("fxbrowser", cx).unwrap();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, _| {
                let last = v.panel_rect("fxbrowser").unwrap();
                assert!(
                    last.y >= 0. && last.y + last.h <= 600.,
                    "the last panel is visible after Show: {last:?}"
                );
            })
            .unwrap();
    });
}
