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
        for (frame, (delta, touch_phase)) in [
            (0., TouchPhase::Started),
            (-16., TouchPhase::Moved),
            (-44., TouchPhase::Moved),
            (-85., TouchPhase::Moved),
            (-180., TouchPhase::Moved),
            (-300., TouchPhase::Moved),
            (-300., TouchPhase::Ended),
            (195., TouchPhase::Moved),
            (1000., TouchPhase::Ended),
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
