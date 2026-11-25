//! Manual developer gallery. Never compiled into the release app.
#[cfg(all(target_os = "macos", debug_assertions))]
mod gallery {
    use gpui::{prelude::*, *};
    use pm_shell::wave::*;
    use std::rc::Rc;
    struct Gallery {
        waves: Vec<Entity<WaveState>>,
        sources: Vec<WaveSource>,
    }
    impl Render for Gallery {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let mut row = div()
                .flex()
                .gap_4()
                .p_4()
                .bg(rgb(0x101822))
                .text_color(rgb(0xffffff));
            for (index, name) in ["Vertical", "Horizontal", "Center"].into_iter().enumerate() {
                let handle = self.waves[index].clone();
                let source = self.sources[index].clone();
                let play = handle.clone();
                let reverse = handle.clone();
                let reset = handle.clone();
                let button = div()
                    .id(("play", index))
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(rgb(0xe69541))
                    .cursor_pointer()
                    .child("Play")
                    .on_click(move |_, w, c| play.update(c, |s, _| s.play(w)));
                let trigger = match index {
                    0 => div()
                        .absolute()
                        .bottom_3()
                        .left(px(140.))
                        .child(source.track(button)),
                    1 => div()
                        .absolute()
                        .left_2()
                        .top(px(70.))
                        .child(source.track(button.w(px(44.)).h(px(110.)))),
                    _ => div()
                        .absolute()
                        .left(px(145.))
                        .top(px(95.))
                        .child(source.track(button.w(px(60.)).h(px(60.)).rounded_full())),
                };
                let controls = div()
                    .absolute()
                    .bottom_3()
                    .right_3()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id(("reverse", index))
                            .px_3()
                            .py_2()
                            .bg(rgb(0x34445c))
                            .rounded_lg()
                            .cursor_pointer()
                            .child("Reverse")
                            .on_click(move |_, w, c| reverse.update(c, |s, _| s.reverse(w))),
                    )
                    .child(
                        div()
                            .id(("reset", index))
                            .px_3()
                            .py_2()
                            .bg(rgb(0x34445c))
                            .rounded_lg()
                            .cursor_pointer()
                            .child("Reset")
                            .on_click(move |_, w, c| reset.update(c, |s, _| s.reset(w))),
                    );
                let from = div()
                    .size_full()
                    .bg(rgb(0x142034))
                    .p_5()
                    .child("From view")
                    .child(div().mt_4().w_24().h_16().rounded_lg().bg(rgb(0xf4a042)))
                    .child(div().mt_3().w_40().h_8().bg(rgb(0x3ab2cf)));
                let to = div()
                    .size_full()
                    .bg(rgb(0x8253b7))
                    .p_5()
                    .child("Revealed view")
                    .child(div().mt_4().w_32().h_24().rounded_full().bg(rgb(0x75c6cf)));
                row = row.child(
                    div().flex().flex_col().gap_3().child(name).child(
                        DistortionWave::new(handle, from)
                            .revealed_content(to)
                            .overlay(div().relative().size_full().child(trigger).child(controls))
                            .w(px(350.))
                            .h(px(250.)),
                    ),
                );
            }
            row
        }
    }
    pub fn run() {
        Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false))).run(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1140.), px(330.)),
                        cx,
                    ))),
                    ..WindowOptions::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let sources = (0..3).map(|_| WaveSource::default()).collect::<Vec<_>>();
                        let waves = [WaveType::Vertical, WaveType::Horizontal, WaveType::Center]
                            .into_iter()
                            .enumerate()
                            .map(|(i, kind)| {
                                let style = WaveStyle::new(kind);
                                cx.new(|_| WaveState::new(style, sources[i].clone()).unwrap())
                            })
                            .collect();
                        Gallery { waves, sources }
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
    }
}
#[cfg(all(target_os = "macos", debug_assertions))]
fn main() {
    gallery::run();
}
#[cfg(not(all(target_os = "macos", debug_assertions)))]
fn main() {
    eprintln!("The distortion wave gallery is available only in macOS debug builds.");
}
