//! In-memory paint and input on TestPlatform. The canvas probes read the
//! same frame and content mask used when staging an AppKit slot.
use super::*;
use gpui::{AppContext, HeadlessAppContext, TouchPhase, size};
use pm_design::{Accessibility, Scheme};
use std::{cell::Cell, sync::Arc};

const PANELS: [&str; 5] = ["ext.logos", "fxbrowser", "easing", "ext.pixel", "ext.notes"];
type Probe = (String, Bounds<Pixels>, Bounds<Pixels>);

struct DockFixture(Rc<RefCell<Vec<Probe>>>);

impl crate::integration::Integration for DockFixture {
    fn observe(&self, _: &mut Context<Editor>) {}
    fn command(&self, _: &Command, _: &mut Window, _: &mut App) -> bool {
        false
    }
    fn panel(&self, id: &str, rect: Rect, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        let index = PANELS.iter().position(|panel| *panel == id)?;
        let id = id.to_owned();
        let label = [
            "Commons Logos",
            "Search effects    ▾ All",
            "Apply Curve",
            "Mirage screen",
            "Motion Notes",
        ][index];
        let probes = self.0.clone();
        let header = Tokens::new(Scheme::Dark, Accessibility::default())
            .metrics
            .header;
        Some(
            div()
                .relative()
                .h(px((rect.h - header).max(0.)))
                .w_full()
                .child(
                    div()
                        .absolute()
                        .top(px(6.))
                        .left(px(10.))
                        .w(px(155.))
                        .h(px(24.))
                        .rounded(px(4.))
                        .bg(hsla(0.59, 0.45, 0.46, 1.0))
                        .text_color(rgb(0xffffff))
                        .child(label)
                        .child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, window, _| {
                                    probes.borrow_mut().push((
                                        id.clone(),
                                        bounds,
                                        window.content_mask().bounds,
                                    ));
                                },
                            )
                            .absolute()
                            .inset_0(),
                        ),
                )
                .into_any_element(),
        )
    }
    fn header(
        &self,
        id: &str,
        _: Rect,
        tabbed: bool,
        _: &mut Window,
        _: &mut App,
    ) -> Option<AnyElement> {
        if id != "agent" || !tabbed {
            return None;
        }
        let probes = self.0.clone();
        Some(
            div()
                .flex()
                .h_full()
                .children([("Threads", 94.), ("+", 26.)].map(|(label, width)| {
                    let probes = probes.clone();
                    div()
                        .relative()
                        .flex_none()
                        .w(px(width))
                        .h_full()
                        .child(label)
                        .child(
                            canvas(
                                |_, _, _| (),
                                move |bounds, _, window, _| {
                                    probes.borrow_mut().push((
                                        label.into(),
                                        bounds,
                                        window.content_mask().bounds,
                                    ));
                                },
                            )
                            .absolute()
                            .inset_0(),
                        )
                }))
                .into_any_element(),
        )
    }
    fn contributed_panels(&self, _: &App) -> Vec<(String, String)> {
        [
            ("ext.logos", "Commons Logos"),
            ("ext.pixel", "Pixel comparison"),
            ("ext.notes", "Motion Notes"),
        ]
        .map(|(id, title)| (id.into(), title.into()))
        .into()
    }
    fn project_id(&self, _: &App) -> Option<String> {
        None
    }
    fn status(&self, _: &App) -> String {
        String::new()
    }
    fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
}

#[gpui::test]
fn narrow_agent_stack_keeps_both_header_controls_inside_the_panel(cx: &mut gpui::TestAppContext) {
    let probes = Rc::new(RefCell::new(Vec::new()));
    let recorded = probes.clone();
    let (view, cx) = cx.add_window_view(|_, cx| {
        let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
        let mut editor = Editor::new(tokens, None, cx);
        editor.integration = Some(Rc::new(DockFixture(recorded)));
        let right = editor
            .layout
            .docks
            .iter_mut()
            .find(|dock| dock.id == "right")
            .unwrap();
        right.size = Some(220.);
        right.groups = vec![layout::Group {
            tabs: vec!["agent".into(), "project".into()],
            active: 0,
            size: None,
            collapsed: false,
        }];
        editor
    });
    cx.run_until_parked();
    let panel = view.read_with(cx, |editor, _| editor.panel_rect("agent").unwrap());
    assert!(panel.w <= 250.);
    for label in ["Threads", "+"] {
        let frames = probes.borrow();
        let (_, frame, clip) = frames.iter().rev().find(|(id, _, _)| id == label).unwrap();
        assert!(frame.size.width > px(0.));
        assert!(frame.right() <= px(panel.x + panel.w));
        assert!(frame.right() <= clip.right());
    }
}

#[::core::prelude::v1::test]
fn dock_render_follows_wheel_and_momentum() {
    let mut header_overlaps = Vec::new();
    for (theme, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
        let probes = Rc::new(RefCell::new(Vec::new()));
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
        let tokens = Tokens::new(scheme, Accessibility::default());
        let header = tokens.metrics.header;
        let window = cx
            .open_window(size(px(1000.), px(600.)), |_, cx| {
                cx.new(|cx| {
                    let mut editor = Editor::new(tokens, None, cx);
                    editor.integration = Some(Rc::new(DockFixture(probes.clone())));
                    let right = editor
                        .layout
                        .docks
                        .iter_mut()
                        .find(|dock| dock.id == "right")
                        .unwrap();
                    right.groups = PANELS
                        .iter()
                        .map(|id| layout::Group {
                            tabs: vec![(*id).to_owned()],
                            active: 0,
                            size: Some(245.),
                            collapsed: false,
                        })
                        .collect();
                    editor
                })
            })
            .unwrap();
        cx.run_until_parked();
        let start = window
            .update(&mut cx, |e, _, _| e.panel_rect("fxbrowser").unwrap())
            .unwrap();
        for (frame, (delta, touch_phase, is_momentum)) in [
            (0., TouchPhase::Started, false),
            (-16., TouchPhase::Moved, false),
            (-44., TouchPhase::Moved, false),
            (-85., TouchPhase::Moved, true),
            (-180., TouchPhase::Moved, true),
            (-300., TouchPhase::Moved, true),
            (-300., TouchPhase::Ended, true),
            (195., TouchPhase::Moved, false),
            (1000., TouchPhase::Ended, false),
        ]
        .into_iter()
        .enumerate()
        {
            probes.borrow_mut().clear();
            cx.update_window(window.into(), |_, w, cx| {
                if delta != 0. {
                    w.dispatch_event(
                        ScrollWheelEvent {
                            position: point(px(start.x + 80.), px(180.)),
                            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(delta))),
                            modifiers: Default::default(),
                            touch_phase,
                            is_momentum,
                        }
                        .to_platform_input(),
                        cx,
                    );
                }
                w.refresh();
            })
            .unwrap();
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).unwrap();
            let rects = window
                .update(&mut cx, |editor, _, _| {
                    PANELS.map(|id| {
                        (
                            id,
                            editor.panel_rect(id).unwrap(),
                            editor.panel_header_rect(id).unwrap(),
                        )
                    })
                })
                .unwrap();
            let paints = probes.borrow();
            assert!(!paints.is_empty());
            for (id, control, mask) in &*paints {
                let (_, panel, heading) = rects.iter().find(|(panel, _, _)| *panel == id).unwrap();
                assert_eq!(
                    control.origin,
                    point(px(panel.x + 10.), px(panel.y + header + 6.)),
                    "{theme} frame {frame}: {id}"
                );
                let visible = control.intersect(mask);
                if visible.size.height > px(0.) && visible.top() < px(heading.y + heading.h) {
                    header_overlaps.push(format!("{theme} frame {frame}: {id}"));
                }
                if control.bottom() <= px(0.) || control.top() >= px(600.) {
                    assert!(
                        visible.size.height <= px(0.),
                        "{id} staged outside the window"
                    );
                }
            }
            let (_, effects, _) = rects[1];
            if frame == 2 {
                assert_eq!(effects.y, start.y - 60.);
            }
            if let Some(dir) = std::env::var_os("PM_DOCK_RENDER_CAPTURE") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let scale = image.width() / 1000;
                let x = (start.x.round() as u32).saturating_sub(4) * scale;
                image::imageops::crop_imm(&image, x, 0, image.width() - x, image.height())
                    .to_image()
                    .save(dir.join(format!("{theme}-{frame:02}.png")))
                    .unwrap();
            }
        }
    }
    assert!(
        header_overlaps.is_empty(),
        "control paints over a sticky header: {header_overlaps:?}"
    );
}

struct BodyFadeProbe {
    scroll: ScrollHandle,
    fade: Rc<Cell<[f64; 2]>>,
}

impl Render for BodyFadeProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let fade = self.fade.clone();
        div()
            .id("scrolling-panel-body")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .scroll_edge_fade_y(px(pm_design::DOCK_SCROLL_FADE))
            .child(
                div().w_full().h(px(300.)).child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, w, _| {
                            fade.set(crate::layer_stage::vertical_fade(w.content_mask()))
                        },
                    )
                    .absolute()
                    .inset_0(),
                ),
            )
    }
}

#[::core::prelude::v1::test]
fn dock_render_body_fades_only_edges_with_more_content() {
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
    let scroll = ScrollHandle::new();
    let fade = Rc::new(Cell::new([0., 0.]));
    let window = cx
        .open_window(size(px(160.), px(100.)), |_, cx| {
            cx.new(|_| BodyFadeProbe {
                scroll: scroll.clone(),
                fade: fade.clone(),
            })
        })
        .unwrap();
    for (offset, expected) in [(0., [0., 28.]), (56., [28., 28.]), (200., [28., 0.])] {
        window
            .update(&mut cx, |_, w, _| {
                scroll.set_offset(point(px(0.), px(-offset)));
                w.refresh();
            })
            .unwrap();
        cx.run_until_parked();
        cx.capture_screenshot(window.into()).unwrap();
        assert_eq!(fade.get(), expected);
    }
}

/// The last row of a panel's own scrolling content is anchored to the body's
/// bottom. When the group stood taller than its dock the row painted under
/// the status bar's position and the panel's bottom edge was cut; bounded to
/// the column it ends above the status bar, corners and inset intact.
struct BottomRowFixture(Rc<RefCell<Vec<Probe>>>);

const LIST_EXTENSIONS: [(&str, &str); 4] = [
    ("background-remover.panel", "Remove Background"),
    ("pexels", "Pexels"),
    ("split-text.panel", "Split Text"),
    ("motion-notes", "Motion Notes"),
];

impl crate::integration::Integration for BottomRowFixture {
    fn observe(&self, _: &mut Context<Editor>) {}
    fn command(&self, _: &Command, _: &mut Window, _: &mut App) -> bool {
        false
    }
    fn panel(&self, id: &str, rect: Rect, _: &mut Window, _: &mut App) -> Option<AnyElement> {
        LIST_EXTENSIONS
            .iter()
            .any(|(panel, _)| *panel == id)
            .then(|| {
                let probes = self.0.clone();
                let header = Tokens::new(Scheme::Dark, Accessibility::default())
                    .metrics
                    .header;
                let id = id.to_owned();
                div()
                    .relative()
                    .h(px((rect.h - header).max(0.)))
                    .w_full()
                    .child(
                        div()
                            .absolute()
                            .bottom(px(6.))
                            .left(px(10.))
                            .w(px(155.))
                            .h(px(24.))
                            .rounded(px(4.))
                            .bg(hsla(0.59, 0.45, 0.46, 1.0))
                            .text_color(rgb(0xffffff))
                            .child("Last row")
                            .child(
                                canvas(
                                    |_, _, _| (),
                                    move |bounds, _, window, _| {
                                        probes.borrow_mut().push((
                                            id.clone(),
                                            bounds,
                                            window.content_mask().bounds,
                                        ));
                                    },
                                )
                                .absolute()
                                .inset_0(),
                            ),
                    )
                    .into_any_element()
            })
    }
    fn contributed_panels(&self, _: &App) -> Vec<(String, String)> {
        LIST_EXTENSIONS
            .map(|(id, title)| (id.into(), title.into()))
            .into()
    }
    fn project_id(&self, _: &App) -> Option<String> {
        None
    }
    fn status(&self, _: &App) -> String {
        "Playback paused · 1920×1080 · 30 fps · Default".into()
    }
    fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
}

#[::core::prelude::v1::test]
fn dock_render_bounds_a_tall_extension_body_above_the_status_bar() {
    let mut failures = Vec::new();
    for (theme, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
        for (tag, window_h, saved_height) in [("tall", 760., 2400.), ("short", 430., 900.)] {
            let probes = Rc::new(RefCell::new(Vec::new()));
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
            let tokens = Tokens::new(scheme, Accessibility::default());
            let m = tokens.metrics;
            let window = cx
                .open_window(size(px(1000.), px(window_h)), |_, cx| {
                    cx.new(|cx| {
                        let mut editor = Editor::new(tokens, None, cx);
                        editor.integration = Some(Rc::new(BottomRowFixture(probes.clone())));
                        let right = editor
                            .layout
                            .docks
                            .iter_mut()
                            .find(|dock| dock.id == "right")
                            .unwrap();
                        right.groups = vec![layout::Group {
                            tabs: vec![LIST_EXTENSIONS[0].0.to_owned()],
                            active: 0,
                            size: Some(saved_height),
                            collapsed: false,
                        }];
                        editor
                    })
                })
                .unwrap();
            cx.run_until_parked();
            // The shipped long-list extensions share this geometry.
            for (panel_id, _) in LIST_EXTENSIONS {
                window
                    .update(&mut cx, |editor, _, cx| {
                        let right = editor
                            .layout
                            .docks
                            .iter_mut()
                            .find(|dock| dock.id == "right")
                            .unwrap();
                        right.groups[0].tabs[0] = panel_id.to_owned();
                        cx.notify();
                    })
                    .unwrap();
                cx.run_until_parked();
                let image = cx.capture_screenshot(window.into()).unwrap();
                let panel = window
                    .update(&mut cx, |editor, _, _| editor.panel_rect(panel_id).unwrap())
                    .unwrap();
                if let Some(dir) = std::env::var_os("PM_DOCK_RENDER_CAPTURE") {
                    let dir = std::path::PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    let scale = image.width() / 1000;
                    let x = (panel.x.round() as u32).saturating_sub(4) * scale;
                    let y = (window_h as u32 - 140).min(image.height()) * scale;
                    image::imageops::crop_imm(&image, x, y, image.width() - x, image.height() - y)
                        .to_image()
                        .save(dir.join(format!("{theme}-{tag}-{panel_id}.png")))
                        .unwrap();
                }
                let dock_bottom = window_h - m.status - m.dock_gap;
                let status_top = window_h - m.status;
                if panel.y + panel.h > dock_bottom + 0.01 {
                    failures.push(format!(
                        "{theme}/{tag}/{panel_id}: panel bottom {} below the column viewport {dock_bottom}",
                        panel.y + panel.h,
                    ));
                }
                let body_bottom = panel.y + panel.h - m.border.max(m.splitter_reach);
                if body_bottom > status_top - m.inset + 0.01 {
                    failures.push(format!(
                        "{theme}/{tag}/{panel_id}: content bottom {body_bottom} inside the status inset {}",
                        status_top - m.inset,
                    ));
                }
                let row = probes
                    .borrow()
                    .iter()
                    .filter(|(id, _, _)| id == panel_id)
                    .last()
                    .map(|(_, bounds, mask)| (*bounds, *mask));
                if !row.is_some_and(|(bounds, mask)| {
                    bounds.intersect(&mask).size.height == bounds.size.height
                }) {
                    failures.push(format!("{theme}/{tag}/{panel_id}: the last row is clipped"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
