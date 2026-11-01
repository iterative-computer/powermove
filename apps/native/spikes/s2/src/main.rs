mod contract;
mod design;
mod glass;
mod native;
mod onscreen;
mod slot_view;
mod verify;
use gpui::{prelude::*, *};
use std::{cell::RefCell, rc::Rc};
struct Demo {
    host: Rc<RefCell<native::Host>>,
    fixture: Option<Rc<RefCell<onscreen::Fixture>>>,
}
impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let host = self.host.clone();
        let fixture = self.fixture.clone();
        if fixture.as_ref().is_some_and(|f| f.borrow().frames < 650) {
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
                        if let Some(fixture) = &fixture {
                            fixture.borrow_mut().paint(w, h, window);
                        }
                        let _ = cx;
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
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
    Application::with_platform(platform).run(move |cx| {
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
                let demo = cx.new(|_| Demo { host, fixture });
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
