//! Explicit opt-in captures through TestPlatform and a headless Metal renderer.
//! No NSWindow, app bundle, user profile, or OS input participates.
use crate::{
    timeline_surface::TimelineSurface,
    v4_view::{Natives, SurfaceView},
    v4_view_tests::{PROJECT, publication, rig_project},
};
use gpui::{
    AppContext, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, point, prelude::*, px,
    size,
};
use pm_kernel::{Call, Caller, Request, SelectionCall, WriteBody};
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
#[ignore = "Opt-in headless visual review; requires PM_G2_CAPTURE_DIR"]
fn g2_transition_crops() {
    let output = std::path::PathBuf::from(
        std::env::var_os("PM_G2_CAPTURE_DIR").expect("capture output directory"),
    );
    std::fs::create_dir_all(&output).unwrap();
    let text = Arc::new(gpui_parley::ParleyTextSystem::new(
        gpui_parley::SystemFonts::Load,
    ));
    let mut app = HeadlessAppContext::with_platform(text, Arc::new(()), || {
        Some(Box::new(
            gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
        ))
    });
    app.run_until_parked();
    for (scheme, name) in [
        (pm_design::Scheme::Light, "light"),
        (pm_design::Scheme::Dark, "dark"),
    ] {
        let mut project: serde_json::Value =
            serde_json::from_str(&crate::v4_view_tests::project_json(2)).unwrap();
        project["layers"][0]["from"] = serde_json::json!(0.);
        project["layers"][1]["from"] = serde_json::json!(2.);
        for layer in project["layers"].as_array_mut().unwrap() {
            layer["solo"] = serde_json::json!(false);
            layer["track"] = serde_json::json!(0);
        }
        for i in 0..2 {
            let source = format!("source-{i}");
            project["layers"][i]["type"] = serde_json::json!("precomp");
            project["layers"][i]["d"] =
                serde_json::json!({"comp":source,"w":640,"h":360,"trim":if i == 0 {0.} else {0.3}});
            project["comps"][&source] = serde_json::json!({"id":source,"w":640,"h":360,"fps":30,"dur":if i == 0 {2.3} else {4.},"layers":[],"backgroundFill":{"type":"none","angle":0,"stops":[]}});
        }
        let rig = rig_project("timeline", &project.to_string());
        let first = publication(&rig);
        let tokens = pm_design::Tokens::new(scheme, Default::default());
        let natives = Natives::default();
        let outline = natives.outline.clone();
        let recorder = Rc::new(crate::timeline_tests::Recorder::default());
        let delegate = Rc::new(crate::timeline_tests::Recording(recorder));
        let (window, surface) = app.update(|cx| {
            let surface = cx.new(|cx| {
                TimelineSurface::new(
                    rig.kernel.clone(),
                    PROJECT.into(),
                    tokens.clone(),
                    delegate,
                    outline.clone(),
                    Default::default(),
                    cx,
                )
            });
            natives
                .views
                .borrow_mut()
                .insert("timeline".into(), surface.clone().into());
            let handle = cx
                .open_window(
                    WindowOptions::new()
                        .show(false)
                        .focus(false)
                        .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(1100.), px(360.)),
                        )))),
                    |window, cx| {
                        let view = cx.new(|cx| {
                            SurfaceView::new(
                                rig.runtime.clone(),
                                first,
                                tokens,
                                natives,
                                window,
                                cx,
                            )
                        });
                        outline.bind(view.downgrade());
                        view
                    },
                )
                .unwrap();
            surface.update(cx, |s, cx| {
                s.set_view(Some(220.), Some(0.), Some(0.), cx);
                s.command("timeline.mode:tracks", 0., None, false, cx);
            });
            (handle, surface)
        });
        for phase in [
            "before",
            "unselected",
            "selected",
            "shortened",
            "zero",
            "restored",
        ] {
            if phase == "unselected" {
                let published = rig.kernel.published(PROJECT).unwrap();
                let commands = pm_kernel::commands::transition_edges::plan(
                    published.snapshot.root().unwrap(),
                    "l0",
                    pm_kernel::commands::effects::Edge::Out,
                    pm_kernel::commands::transition_edges::builtin_spec("crossfade"),
                    true,
                )
                .unwrap();
                let ack = rig
                    .kernel
                    .call(
                        PROJECT,
                        Request::new(
                            Caller::native(),
                            Call::Transact {
                                label: Some("Add Transition".into()),
                                expected_revision: Some(published.revision()),
                                body: WriteBody::Commands(commands),
                            },
                        ),
                    )
                    .wait()
                    .unwrap()
                    .unwrap();
                assert!(
                    !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
                    "{:?}",
                    ack.reply
                );
            } else if phase == "selected" {
                rig.kernel
                    .call(
                        PROJECT,
                        Request::new(
                            Caller::native(),
                            SelectionCall::Layers {
                                ids: vec!["l1".into()],
                                add: false,
                            },
                        ),
                    )
                    .wait()
                    .unwrap()
                    .unwrap();
            }
            if matches!(phase, "shortened" | "zero") {
                use pm_kernel::commands::timing::{TimeEdge, TimingCommand};
                let mut commands = vec![
                    TimingCommand::Roll {
                        layer_id: "l0".into(),
                        edge: TimeEdge::Out,
                        time: if phase == "zero" { 2.3 } else { 2.2 },
                    }
                    .into(),
                ];
                if phase == "zero" {
                    commands.push(
                        pm_kernel::PropertyCommand::Set(
                            pm_kernel::commands::properties::SetProperty::new(
                                "l1", "c.trim", 0., 2.3,
                            ),
                        )
                        .into(),
                    );
                }
                capture_transact(&rig, commands);
            } else if phase == "restored" {
                for _ in 0..2 {
                    let ack = rig
                        .kernel
                        .call(
                            PROJECT,
                            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
                        )
                        .wait()
                        .unwrap()
                        .unwrap();
                    assert!(
                        !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
                        "{:?}",
                        ack.reply
                    );
                }
            }
            if phase == "selected" {
                app.update(|cx| {
                    surface.update(cx, |surface, cx| {
                        surface.set_transition_selection(Some(("l1".into(), "in".into())), cx);
                    })
                });
            }
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                app.run_until_parked();
                let ready = app.update(|cx| {
                    surface.update(cx, |s, cx| s.published(cx));
                    gpui::AnyWindowHandle::from(window)
                        .update(cx, |_, window, cx| {
                            window.refresh();
                            window.draw(cx).clear(cx);
                        })
                        .unwrap();
                    surface.read(cx).frame().is_some_and(|frame| {
                        surface.read(cx).toggles().0
                            && match phase {
                                "before" => {
                                    !frame.ops.iter().any(|op| op.id.starts_with("transition:"))
                                }
                                "selected" => frame
                                    .ops
                                    .iter()
                                    .any(|op| op.id.starts_with("transition-selection:")),
                                "shortened" | "restored" | "zero" => frame.ops.iter().any(|op| {
                                    op.id.starts_with("transition:")
                                        && op.detail["effectiveDuration"].as_f64().is_some_and(
                                            |duration| {
                                                (duration
                                                    - if phase == "shortened" {
                                                        0.2
                                                    } else if phase == "zero" {
                                                        0.
                                                    } else {
                                                        0.5
                                                    })
                                                .abs()
                                                    < 1e-6
                                            },
                                        )
                                }),
                                _ => frame.ops.iter().any(|op| op.id.starts_with("transition:")),
                            }
                    })
                });
                if ready {
                    break;
                }
                assert!(Instant::now() < deadline, "capture did not settle");
                std::thread::sleep(Duration::from_millis(10));
            }
            if phase != "before" {
                app.update(|cx| {
                    let frame = surface.read(cx).frame().unwrap();
                    for capsule in frame
                        .ops
                        .iter()
                        .filter(|op| op.id.starts_with("transition:"))
                    {
                        for name in frame
                            .ops
                            .iter()
                            .filter(|op| op.id.starts_with("clip-content:") && op.shape == "text")
                        {
                            let a = capsule.bounds;
                            let b = name.bounds;
                            assert!(
                                b.width == 0. || b.x + b.width <= a.x || b.x >= a.x + a.width,
                                "clip names must stay clear of the visible transition capsule"
                            );
                        }
                        let clip = frame.ops.iter().find(|op| op.id == "clip:l1").unwrap();
                        assert_ne!(
                            capsule.color, clip.color,
                            "the capsule uses the bar's contrasting accent tint"
                        );
                    }
                    for selection in frame
                        .ops
                        .iter()
                        .filter(|op| op.id.starts_with("transition-selection:"))
                    {
                        assert_eq!(
                            selection.shape, "ring",
                            "selection must outline rather than fill the capsule"
                        );
                        assert_eq!(selection.stroke, 2.);
                        assert!(
                            frame
                                .ops
                                .iter()
                                .filter(|op| op.id.starts_with("clip:"))
                                .all(|op| op.detail["selected"] != true),
                            "transition selection must not look like clip selection"
                        );
                    }
                });
            }
            for (_, ticket) in rig.runtime.publish() {
                let ticket = ticket.unwrap();
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    if let Some(result) = ticket.try_take() {
                        result.unwrap();
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "Timeline publication did not settle"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            for publication in rig.runtime.take_publications() {
                app.update(|cx| {
                    window
                        .update(cx, |view, _, cx| view.publish(publication, cx))
                        .unwrap()
                });
            }
            app.run_until_parked();
            app.update(|cx| {
                gpui::AnyWindowHandle::from(window)
                    .update(cx, |_, window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                        let image = window.render_to_image().unwrap();
                        let frame = surface.read(cx).frame().unwrap();
                        assert_capsule_pixels(&frame, image.width(), |x, y| {
                            image.get_pixel(x, y).0
                        });
                        image
                            .save(output.join(format!("{name}-{phase}.png")))
                            .unwrap();
                    })
                    .unwrap()
            });
        }
        // Leaving the capsule restores the owner's explicit clip selection.
        app.update(|cx| {
            surface.update(cx, |s, cx| s.set_transition_selection(None, cx));
            gpui::AnyWindowHandle::from(window)
                .update(cx, |_, window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                    let frame = surface.read(cx).frame().unwrap();
                    assert!(
                        frame
                            .ops
                            .iter()
                            .any(|op| op.id == "clip:l1" && op.detail["selected"] == true)
                    );
                    assert!(
                        !frame
                            .ops
                            .iter()
                            .any(|op| op.id.starts_with("transition-selection:"))
                    );
                    let image = window.render_to_image().unwrap();
                    assert_capsule_pixels(&frame, image.width(), |x, y| image.get_pixel(x, y).0);
                })
                .unwrap();
            surface.update(cx, |s, cx| {
                s.set_transition_selection(Some(("l1".into(), "in".into())), cx)
            });
        });
        // Explicitly selected clips keep their rings, below the capsule.
        rig.kernel
            .call(
                PROJECT,
                Request::new(
                    Caller::native(),
                    SelectionCall::Layers {
                        ids: vec!["l0".into(), "l1".into()],
                        add: false,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            app.run_until_parked();
            let ready = app.update(|cx| {
                surface.update(cx, |s, cx| s.published(cx));
                gpui::AnyWindowHandle::from(window)
                    .update(cx, |_, window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                        let frame = surface.read(cx).frame().unwrap();
                        let ready = frame
                            .ops
                            .iter()
                            .filter(|op| {
                                op.id.starts_with("clip:") && op.detail["selected"] == true
                            })
                            .count()
                            == 2;
                        if ready {
                            let image = window.render_to_image().unwrap();
                            assert_capsule_pixels(&frame, image.width(), |x, y| {
                                image.get_pixel(x, y).0
                            });
                        }
                        ready
                    })
                    .unwrap()
            });
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "explicit clip selection did not settle"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        app.update(|cx| {
            gpui::AnyWindowHandle::from(window)
                .update(cx, |_, window, _| window.remove_window())
                .unwrap()
        });
        // Effects uses the same v4 browser and winning native registry as the app.
        for baseline in [true, false] {
            let mut fx = rig_project("timeline", &project.to_string());
            if baseline {
                let ops = fx
                    .kernel
                    .state()
                    .registries
                    .get("transitions")
                    .into_iter()
                    .flatten()
                    .map(|entry| pm_kernel::state::StateOp::Dispose {
                        token: entry.token.clone(),
                    })
                    .collect();
                fx.kernel
                    .state_transaction(pm_kernel::state::StateTransaction {
                        expected_revision: None,
                        ops,
                    })
                    .wait()
                    .unwrap()
                    .unwrap();
            }
            let ticket = Arc::get_mut(&mut fx.runtime)
                .expect("unshared capture runtime")
                .mount("fxbrowser")
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                if let Some(result) = ticket.try_take() {
                    result.unwrap();
                    break;
                }
                assert!(Instant::now() < deadline, "Effects mount did not settle");
                std::thread::sleep(Duration::from_millis(5));
            }

            let mut first = fx
                .runtime
                .take_publications()
                .into_iter()
                .rev()
                .find(|p| p.panel == "fxbrowser")
                .expect("Effects publication");
            assert_eq!(
                first
                    .surface
                    .nodes()
                    .values()
                    .any(|node| node.text == "Crossfade"),
                !baseline,
                "before uses the pre-G2 registry input; after shows Transitions"
            );
            if !baseline {
                // Use the actual category selection rather than seeding
                // storage: the built-in extension's legacy dotted storage
                // key is rejected by the kernel's current key validator.
                let category = first
                    .surface
                    .nodes()
                    .values()
                    .find(|node| {
                        node.props.get("label").and_then(serde_json::Value::as_str)
                            == Some("Transitions")
                    })
                    .expect("Transitions category")
                    .id;
                let ticket = fx
                    .runtime
                    .event(
                        "fxbrowser",
                        first.generation,
                        first.surface.revision(),
                        category,
                        "click",
                        pm_surface::events::EventDetail::default(),
                    )
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    if let Some(result) = ticket.try_take() {
                        result.unwrap();
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "Transitions category did not settle"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                first = fx
                    .runtime
                    .take_publications()
                    .into_iter()
                    .rev()
                    .find(|publication| publication.panel == "fxbrowser")
                    .expect("selected Transitions publication");
                for name in ["Crossfade", "Wipe", "Slide", "Zoom"] {
                    assert!(first.surface.nodes().values().any(|node| node.text == name));
                }
            }
            let tokens = pm_design::Tokens::new(scheme, Default::default());
            let window = app.update(|cx| {
                cx.open_window(
                    WindowOptions::new()
                        .show(false)
                        .focus(false)
                        .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(420.), px(650.)),
                        )))),
                    |window, cx| {
                        let view = cx.new(|cx| {
                            SurfaceView::new(
                                fx.runtime.clone(),
                                first,
                                tokens.clone(),
                                Natives::default(),
                                window,
                                cx,
                            )
                        });
                        cx.new(|_| PanelCapture {
                            view,
                            tokens,
                            title: if baseline {
                                "Before · pre-G2 registry input"
                            } else {
                                "After · Transitions"
                            },
                        })
                    },
                )
                .unwrap()
            });
            app.run_until_parked();
            app.update(|cx| {
                gpui::AnyWindowHandle::from(window)
                    .update(cx, |_, window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                        window
                            .render_to_image()
                            .unwrap()
                            .save(output.join(format!(
                                "{name}-effects-{}.png",
                                if baseline { "before" } else { "after" }
                            )))
                            .unwrap();
                        window.remove_window();
                    })
                    .unwrap()
            });
            drop(fx);
        }
        for phase in ["before", "after", "shortened", "zero"] {
            let source = if phase == "before" {
                project.to_string()
            } else {
                pm_model::json::to_canonical_string(
                    &rig.kernel
                        .published(PROJECT)
                        .unwrap()
                        .snapshot
                        .root()
                        .unwrap()
                        .to_value(),
                )
            };
            let properties = rig_project("inspector", &source);
            properties
                .kernel
                .call(
                    PROJECT,
                    Request::new(
                        Caller::native(),
                        SelectionCall::Layers {
                            ids: vec!["l1".into()],
                            add: false,
                        },
                    ),
                )
                .wait()
                .unwrap()
                .unwrap();
            if matches!(phase, "shortened" | "zero") {
                use pm_kernel::commands::timing::{TimeEdge, TimingCommand};
                let mut commands = vec![
                    TimingCommand::Roll {
                        layer_id: "l0".into(),
                        edge: TimeEdge::Out,
                        time: if phase == "zero" { 2.3 } else { 2.2 },
                    }
                    .into(),
                ];
                if phase == "zero" {
                    commands.push(
                        pm_kernel::PropertyCommand::Set(
                            pm_kernel::commands::properties::SetProperty::new(
                                "l1", "c.trim", 0., 2.3,
                            ),
                        )
                        .into(),
                    );
                }
                capture_transact(&properties, commands);
            }
            for (_, ticket) in properties.runtime.publish() {
                let ticket = ticket.unwrap();
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    if let Some(result) = ticket.try_take() {
                        result.unwrap();
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "Properties publication did not settle"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let first = publication(&properties);
            let tokens = pm_design::Tokens::new(scheme, Default::default());
            let window = app.update(|cx| {
                cx.open_window(
                    WindowOptions::new()
                        .show(false)
                        .focus(false)
                        .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                            point(px(0.), px(0.)),
                            size(px(420.), px(850.)),
                        )))),
                    |window, cx| {
                        let view = cx.new(|cx| {
                            SurfaceView::new(
                                properties.runtime.clone(),
                                first,
                                tokens.clone(),
                                Natives::default(),
                                window,
                                cx,
                            )
                        });
                        cx.new(|_| PanelCapture {
                            view,
                            tokens,
                            title: "Properties",
                        })
                    },
                )
                .unwrap()
            });
            app.run_until_parked();
            app.update(|cx| {
                gpui::AnyWindowHandle::from(window)
                    .update(cx, |_, window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                        let image = window.render_to_image().unwrap();
                        let ground = image.get_pixel(0, image.height() - 1).0;
                        let panel = pm_design::Tokens::new(scheme, Default::default())
                            .colors
                            .panel;
                        for (actual, expected) in ground[..3].iter().zip([
                            (panel >> 16) as u8,
                            (panel >> 8) as u8,
                            panel as u8,
                        ]) {
                            assert!(
                                actual.abs_diff(expected) <= 1,
                                "Properties capture must include its themed panel ground"
                            );
                        }
                        assert_eq!(ground[3], 255);
                        image
                            .save(output.join(format!("{name}-properties-{phase}.png")))
                            .unwrap();
                        window.remove_window();
                    })
                    .unwrap()
            });
        }
    }
}

/// Pixel regression: the label's upper padding is uniform across the cut,
/// so neither cut-line nor clip-ring colours may survive through its rect.
/// A zero-duration marker has no label: check its central patch instead.
fn assert_capsule_pixels(
    frame: &pm_panel_timeline::render::Frame,
    width: u32,
    pixel: impl Fn(u32, u32) -> [u8; 4],
) {
    let scale = width as f64 / frame.viewport.width;
    for capsule in frame
        .ops
        .iter()
        .filter(|op| op.id.starts_with("transition:"))
    {
        let b = capsule.bounds;
        let ground = frame
            .ops
            .iter()
            .position(|op| op.id == capsule.id.replacen("transition:", "transition-ground:", 1))
            .unwrap();
        assert_eq!(frame.ops[ground].opacity, 1.);
        assert!(
            frame
                .ops
                .iter()
                .enumerate()
                .filter(|(_, op)| op.id.starts_with("clip:"))
                .all(|(index, _)| index < ground),
            "capsule must draw above every clip edge and selected ring"
        );
        let cut = frame
            .ops
            .iter()
            .find(|op| op.id == format!("clip:{}", capsule.detail["layer"].as_str().unwrap()))
            .unwrap()
            .detail["from"]
            .as_f64()
            .unwrap();
        let cut_x =
            frame.viewport.layer_column + (cut - frame.viewport.scroll_time) * frame.viewport.pps;
        let y = if b.width >= 66. {
            b.y + 2.5
        } else {
            b.y + b.height / 2.
        };
        let y = (y * scale).round() as u32;
        let center = (cut_x * scale).round() as i32;
        let radius = (3. * scale) as i32;
        let reference = pixel((center - radius) as u32, y);
        for x in center - radius..=center + radius {
            let actual = pixel(x as u32, y);
            for channel in 0..3 {
                assert!(
                    actual[channel].abs_diff(reference[channel]) <= 1,
                    "cut-line or clip-ring colour leaked into capsule label rect at {x},{y}: {actual:?} vs {reference:?}"
                );
            }
        }
    }
}

fn capture_transact(rig: &crate::v4_view_tests::Rig, commands: Vec<pm_kernel::Command>) {
    let ack = rig
        .kernel
        .call(
            PROJECT,
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Edit Clip".into()),
                    expected_revision: None,
                    body: WriteBody::Commands(commands),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(
        !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
        "{:?}",
        ack.reply
    );
}

/// The comparison baseline is actual native UI with pre-G2 registry input,
/// clearly labelled; it is not a capture of an older application binary.
struct PanelCapture {
    view: gpui::Entity<SurfaceView>,
    tokens: pm_design::Tokens,
    title: &'static str,
}
impl gpui::Render for PanelCapture {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .bg(gpui::rgb(self.tokens.colors.panel))
            .child(
                gpui::div()
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .text_size(px(11.))
                    .text_color(gpui::rgb(self.tokens.colors.accent))
                    .child(self.title),
            )
            .child(gpui::div().flex_1().min_h_0().child(self.view.clone()))
    }
}
