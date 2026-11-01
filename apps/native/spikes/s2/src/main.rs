mod contract;
mod design;
mod glass;
mod native;
mod onscreen;
mod p120;
mod slot_view;
mod verify;
use gpui::{prelude::*, *};
use std::{cell::RefCell, rc::Rc};
struct Demo {
    host: Rc<RefCell<native::Host>>,
    fixture: Option<Rc<RefCell<onscreen::Fixture>>>,
    focus: FocusHandle,
    tiles: Option<Entity<onscreen::TileView>>,
    scroll: UniformListScrollHandle,
    frame: usize,
    text: Vec<Entity<pm_shell::text::TextField>>,
}
impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame += 1;
        let timeline = std::env::var_os("S2_TIMELINE").is_some();
        if timeline {
            self.scroll
                .scroll_to_item_strict((self.frame * 7) % 4990, ScrollStrategy::Top);
        }
        if timeline && self.frame == 7400 {
            println!("S2_TIMELINE frames=7400 rows=5000 scroll_step=7");
        }
        let host = self.host.clone();
        let fixture = self.fixture.clone();
        if (timeline && self.frame < 7400)
            || fixture.as_ref().is_some_and(|f| f.borrow().frames < 7400)
        {
            let weak = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
            window.request_animation_frame();
        }
        div()
            .size_full()
            .text_color(rgb(design::TEXT))
            .font_family(".AppleSystemUIFont")
            .child(
                canvas(
                    |_, _, _| (),
                    move |b, _, window, cx| {
                        let w = f32::from(b.size.width);
                        let h = f32::from(b.size.height);
                        host.borrow_mut().publish(w as f64, h as f64);
                        // Solid work surfaces; the viewer and compat areas remain transparent.
                        for (r, c) in [
                            (
                                contract::Rect::new(0., 52., 195., h as f64 - 52.),
                                design::SIDEBAR,
                            ),
                            (
                                contract::Rect::new(0., h as f64 - 190., w as f64, 190.),
                                design::TIMELINE,
                            ),
                        ] {
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(px(r.x as f32), px(r.y as f32)),
                                    size(px(r.w as f32), px(r.h as f32)),
                                ),
                                rgb(c),
                            ));
                        }
                        let _ = cx;
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(self.tiles.clone().map(|tile| {
                if std::env::var_os("S2_BASELINE").is_some() {
                    tile.into_any_element()
                } else {
                    tile.cached({
                        let mut style = StyleRefinement::default();
                        style.size.width = Some(relative(1.).into());
                        style.size.height = Some(relative(1.).into());
                        style.position = Some(Position::Absolute);
                        style
                    })
                    .into_any_element()
                }
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |b, _, w, _| {
                        if let Some(f) = &fixture {
                            f.borrow_mut().paint(
                                f32::from(b.size.width),
                                f32::from(b.size.height),
                                w,
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(self.text.iter().enumerate().map(|(i, field)| {
                div()
                    .absolute()
                    .left(px(214.))
                    .top(px(124. + i as f32 * 106.))
                    .w(px(250.))
                    .child(field.clone())
            }))
            .child(
                div()
                    .absolute()
                    .id("navigation")
                    .role(Role::Group)
                    .accessibility_id("s2-navigation")
                    .aria_label("Navigation")
                    .track_focus(&self.focus)
                    .left_4()
                    .top(px(92.))
                    .w(px(160.))
                    .child("POWERMOVE")
                    .child(div().mt_4().child("Media"))
                    .child(div().mt_2().child("Composition"))
                    .child(div().mt_2().child("Effects")),
            )
            .child(
                div()
                    .absolute()
                    .left_4()
                    .id("timeline-heading")
                    .role(Role::Group)
                    .accessibility_id("s2-timeline")
                    .aria_label("Timeline")
                    .bottom(px(144.))
                    .child("Timeline · 5,000 virtual rows"),
            )
            .child(
                uniform_list("timeline", 5000, |range, _, _| {
                    range
                        .map(|i| {
                            div()
                                .h(px(24.))
                                .px_3()
                                .border_b_1()
                                .border_color(rgb(design::SEPARATOR))
                                .child(format!("Track {:04}  ━━━━━━━  clip  ━━━━━━━", i + 1))
                        })
                        .collect()
                })
                .track_scroll(&self.scroll)
                .absolute()
                .left_0()
                .bottom_0()
                .w_full()
                .h(px(130.)),
            )
    }
}
fn main() -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--verify") {
        return verify::run();
    }
    let device =
        metal::Device::system_default().ok_or_else(|| anyhow::anyhow!("Metal unavailable"))?;
    let platform = Rc::new(gpui_macos::MacPlatform::new_with_metal_device(
        false,
        device.clone(),
    ));
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    println!(
        "S2_SETTINGS reduce_transparency={} increase_contrast={}",
        workspace.accessibilityDisplayShouldReduceTransparency(),
        workspace.accessibilityDisplayShouldIncreaseContrast()
    );
    Application::with_platform(platform)
        .with_accessibility_forced()
        .run(move |cx| {
            native::install_text_menu();
            cx.set_window_appearance(Some(
                if std::env::var("S2_APPEARANCE").as_deref() == Ok("light") {
                    WindowAppearance::Light
                } else {
                    WindowAppearance::Dark
                },
            ));
            cx.open_window(
                WindowOptions::new()
                    .macos_window_background(MacosWindowBackground::Transparent)
                    .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                        point(px(80.), px(80.)),
                        size(px(1180.), px(760.)),
                    )))),
                move |window, cx| {
                    window.set_window_title("Powermove Native · S2");
                    let actions = glass::install(window);
                    let host = native::Host::install(window, &device);
                    if std::env::var_os("S2_MENU").is_some() {
                        host.borrow_mut().toggle_menu();
                    }
                    // Event-driven: native controls wake GPUI, with no idle timer.
                    let fixture = std::env::var_os("S2_PACING")
                        .map(|_| Rc::new(RefCell::new(onscreen::Fixture::new(&device))));
                    let tiles = fixture.as_ref().map(|f| {
                        cx.new(|_| onscreen::TileView {
                            fixture: f.clone(),
                            renders: 0,
                        })
                    });
                    let text = if std::env::var_os("S2_TEXT").is_some() {
                        [
                            (
                                gpui_macos::native_text::TextKind::Multiline,
                                "Caption",
                                "Caption / composer: spell check, IME, Services",
                            ),
                            (
                                gpui_macos::native_text::TextKind::DenseNumber,
                                "Scale",
                                "100",
                            ),
                            (
                                gpui_macos::native_text::TextKind::DenseText,
                                "Layer name",
                                "Layer name",
                            ),
                        ]
                        .into_iter()
                        .map(|(kind, label, value)| {
                            cx.new(|cx| {
                                pm_shell::text::TextField::new(
                                    kind,
                                    label,
                                    value,
                                    pm_design::Tokens::new(
                                        pm_design::Scheme::Dark,
                                        pm_design::Accessibility::default(),
                                    ),
                                    window,
                                    cx,
                                )
                            })
                        })
                        .collect()
                    } else {
                        Vec::new()
                    };
                    let demo = cx.new(|cx| Demo {
                        host,
                        fixture,
                        tiles,
                        text,
                        frame: 0,
                        scroll: UniformListScrollHandle::new(),
                        focus: cx.focus_handle(),
                    });
                    let weak = demo.downgrade();
                    cx.spawn(async move |cx| {
                        while let Ok(action) = actions.recv().await {
                            if weak
                                .update(cx, |demo, cx| {
                                    if action == glass::Action::ZoomIn {
                                        demo.host.borrow_mut().toggle_menu();
                                    }
                                    cx.notify();
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                    demo
                },
            )
            .unwrap();
            cx.activate(true);
        });
    Ok(())
}
