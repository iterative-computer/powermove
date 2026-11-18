#![cfg(target_os = "macos")]
//! Headless recording of panel dragging: the real shell tree on the
//! headless Metal renderer, a scripted pointer dispatched through the
//! window as mouse events, every frame at 60 fps piped to ffmpeg. No window
//! opens and no OS input is sent.
//!
//! `PM_PANEL_DRAG_DIR=<dir> cargo test -p pm-shell --test panel_drag_recording -- --ignored`
//! writes `recording-light.mp4`, `recording-dark.mp4` and `contact-sheet.png`.
use gpui::{
    AppContext, HeadlessAppContext, InputEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, point, px, size,
};
use image::{Rgba, RgbaImage, imageops};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    layout::{Group, Layout, Rect},
    view::Editor,
};
use std::{
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
};

const FPS: f32 = 60.;

/// One step of the script.
enum Step {
    /// Glide the pointer to a place worked out from the window as it is
    /// when the step begins, over this many seconds.
    Move(f32, Box<dyn Fn(&Editor) -> (f32, f32)>),
    Press,
    Release,
    Escape,
    Wait(f32),
    /// Keep this frame for the contact sheet.
    Mark,
}

fn middle(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2., r.y + r.h / 2.)
}
fn tab(id: &'static str) -> Box<dyn Fn(&Editor) -> (f32, f32)> {
    Box::new(move |v| middle(v.tab_rect(id).expect("a tab")))
}
/// A point in the panel showing `id`, as fractions of its rect.
fn within(id: &'static str, fx: f32, fy: f32) -> Box<dyn Fn(&Editor) -> (f32, f32)> {
    Box::new(move |v| {
        let r = v.panel_rect(id).expect("a panel");
        (r.x + r.w * fx, r.y + r.h * fy)
    })
}

fn script() -> Vec<Step> {
    use Step::*;
    vec![
        Wait(0.6),
        // 1. Reorder within a tab group: Notes to the front of its strip.
        Move(0.5, tab("notes")),
        Wait(0.15),
        Press,
        Move(
            1.1,
            Box::new(|v| {
                let r = v.tab_rect("fxbrowser").expect("a tab");
                (r.x + r.w * 0.2, r.y + r.h / 2.)
            }),
        ),
        Wait(0.25),
        Mark,
        Wait(0.2),
        Release,
        Wait(0.5),
        Mark,
        Wait(0.3),
        // 2. Another position in the column: the whole stack, by its
        // header, above Properties.
        Move(
            0.5,
            Box::new(|v| {
                let r = v.panel_rect("notes").expect("a panel");
                (r.x + r.w * 0.72, r.y + 16.)
            }),
        ),
        Wait(0.15),
        Press,
        Move(1.0, within("inspector", 0.5, 0.14)),
        Wait(0.3),
        Mark,
        Wait(0.3),
        Release,
        Wait(0.5),
        Mark,
        Wait(0.3),
        // 3. A cancel: Project out under the viewer, then Escape.
        Move(0.6, tab("assets")),
        Wait(0.15),
        Press,
        Move(1.0, within("viewer", 0.5, 0.85)),
        Wait(0.3),
        Mark,
        Wait(0.4),
        Escape,
        Wait(0.5),
        Mark,
        Wait(0.4),
        // 4. Into another group: Layer Effects joins the right-hand stack.
        Move(0.5, tab("layer-effects")),
        Wait(0.15),
        Press,
        Move(
            1.2,
            Box::new(|v| {
                let r = v.tab_rect("fxbrowser").expect("a tab");
                (r.x + r.w * 0.9, r.y + r.h / 2.)
            }),
        ),
        Wait(0.3),
        Mark,
        Wait(0.3),
        Release,
        Wait(0.6),
        Mark,
        Wait(0.6),
    ]
}

/// The pointer, drawn into the frame (the headless renderer has none): the
/// standard arrow, black with a white edge, its tip at `at`.
fn draw_pointer(image: &mut RgbaImage, at: (f32, f32), _pressed: bool, scale: f32) {
    // The arrow's outline in points, tip at the origin.
    const ARROW: [(f32, f32); 7] = [
        (0., 0.),
        (0., 16.),
        (3.7, 12.6),
        (6.3, 18.4),
        (8.7, 17.3),
        (6.1, 11.7),
        (11.2, 11.7),
    ];
    let edge = 1.1;
    for py in (-2. * scale) as i32..(21. * scale) as i32 {
        for px_ in (-2. * scale) as i32..(14. * scale) as i32 {
            let (x, y) = (px_ as f32 / scale, py as f32 / scale);
            let (mut inside, mut nearest) = (false, f32::MAX);
            for i in 0..ARROW.len() {
                let (a, b) = (ARROW[i], ARROW[(i + 1) % ARROW.len()]);
                if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                    inside = !inside;
                }
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let t = (((x - a.0) * dx + (y - a.1) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
                nearest = nearest.min((x - a.0 - t * dx).hypot(y - a.1 - t * dy));
            }
            let ink = if nearest <= edge {
                [255., 255., 255.]
            } else if inside {
                [0., 0., 0.]
            } else {
                continue;
            };
            let (ix, iy) = ((at.0 * scale) as i32 + px_, (at.1 * scale) as i32 + py);
            if ix < 0 || iy < 0 || ix >= image.width() as i32 || iy >= image.height() as i32 {
                continue;
            }
            // Soften the outer edge.
            let alpha = if inside { 1. } else { ((edge - nearest) * scale).clamp(0., 1.) };
            let pixel = image.get_pixel_mut(ix as u32, iy as u32);
            for c in 0..3 {
                pixel.0[c] = (pixel.0[c] as f32 * (1. - alpha) + ink[c] * alpha) as u8;
            }
        }
    }
}

fn record(scheme: Scheme, name: &str, dir: &std::path::Path) -> Vec<RgbaImage> {
    let tokens = Tokens::new(scheme, Accessibility::default());
    let m = tokens.metrics;
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(
            gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                .with_automatic_optical_sizing(),
        ),
        Arc::new(()),
        || {
            Some(Box::new(
                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    );
    let window = cx
        .open_window(size(px(m.window_width), px(m.window_height)), |_, cx| {
            cx.new(|cx| {
                let mut v = Editor::new(tokens, None, cx);
                v.layout = Layout::preset("design");
                // Three tabs in the right-hand stack, so a reorder shows
                // the neighbours making room.
                let right = v.layout.docks.iter_mut().find(|d| d.id == "right").unwrap();
                right.groups = [vec!["inspector"], vec!["fxbrowser", "easing", "notes"]]
                    .into_iter()
                    .map(|tabs| Group {
                        // The stack is sized, so showing another of its tabs
                        // re-lays nothing; Properties takes the rest.
                        size: (tabs.len() > 1).then_some(360.),
                        tabs: tabs.into_iter().map(str::to_owned).collect(),
                        active: 0,
                        collapsed: false,
                    })
                    .collect();
                v
            })
        })
        .unwrap();
    cx.run_until_parked();
    pm_shell::drag::set_frame_time(Some(std::time::Instant::now()));
    let first = cx.capture_screenshot(window.into()).unwrap();
    let (w, h) = (first.width(), first.height());
    let scale = w as f32 / m.window_width;
    let mut ffmpeg = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pixel_format", "rgba"])
        .args(["-video_size", &format!("{w}x{h}"), "-framerate", "60", "-i", "pipe:0"])
        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "15"])
        .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
        .arg(dir.join(format!("recording-{name}.mp4")))
        .stdin(Stdio::piped())
        .spawn()
        .expect("ffmpeg");
    let mut pipe = ffmpeg.stdin.take().unwrap();

    let mut pointer = (m.window_width * 0.55, m.window_height * 0.45);
    let mut pressed = false;
    let mut marks = vec![];
    let mut frames = 0;
    // One frame: a sixtieth of a second passes, the window draws, the
    // pointer is drawn on top, the picture goes to the encoder.
    let mut frame = |cx: &mut HeadlessAppContext, pointer: (f32, f32), pressed: bool| {
        pm_shell::drag::advance_frame_time(1. / FPS);
        cx.advance_clock(std::time::Duration::from_secs_f32(1. / FPS));
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.run_until_parked();
        let mut image = cx.capture_screenshot(window.into()).unwrap();
        draw_pointer(&mut image, pointer, pressed, scale);
        pipe.write_all(image.as_raw()).unwrap();
        frames += 1;
        image
    };
    let moved = |cx: &mut HeadlessAppContext, to: (f32, f32), pressed: bool| {
        cx.update_window(window.into(), |_, w, cx| {
            w.dispatch_event(
                MouseMoveEvent {
                    position: point(px(to.0), px(to.1)),
                    pressed_button: pressed.then_some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
    };
    let mut last = first;
    for step in script() {
        match step {
            Step::Move(seconds, to) => {
                let to = window.update(&mut cx, |v, _, _| to(v)).unwrap();
                let from = pointer;
                let count = (seconds * FPS).round().max(1.) as usize;
                for i in 1..=count {
                    // Ease in and out, as a hand moves.
                    let t = i as f32 / count as f32;
                    let e = t * t * (3. - 2. * t);
                    pointer = (from.0 + (to.0 - from.0) * e, from.1 + (to.1 - from.1) * e);
                    moved(&mut cx, pointer, pressed);
                    last = frame(&mut cx, pointer, pressed);
                }
            }
            Step::Press => {
                pressed = true;
                cx.update_window(window.into(), |_, w, cx| {
                    w.dispatch_event(
                        MouseDownEvent {
                            button: MouseButton::Left,
                            position: point(px(pointer.0), px(pointer.1)),
                            click_count: 1,
                            ..Default::default()
                        }
                        .to_platform_input(),
                        cx,
                    );
                })
                .unwrap();
                last = frame(&mut cx, pointer, pressed);
            }
            Step::Release => {
                assert!(
                    window.update(&mut cx, |v, _, _| v.dragging_panel()).unwrap(),
                    "the script was dragging a panel"
                );
                pressed = false;
                cx.update_window(window.into(), |_, w, cx| {
                    w.dispatch_event(
                        MouseUpEvent {
                            button: MouseButton::Left,
                            position: point(px(pointer.0), px(pointer.1)),
                            click_count: 1,
                            ..Default::default()
                        }
                        .to_platform_input(),
                        cx,
                    );
                })
                .unwrap();
                last = frame(&mut cx, pointer, pressed);
            }
            Step::Escape => {
                // What the Escape key runs (the headless window has no key
                // focus to send it through).
                window
                    .update(&mut cx, |v, _, cx| {
                        assert!(v.dragging_panel(), "the script was dragging a panel");
                        v.cancel_gesture(cx)
                    })
                    .unwrap();
                pressed = false;
                last = frame(&mut cx, pointer, pressed);
            }
            Step::Wait(seconds) => {
                for _ in 0..(seconds * FPS).round() as usize {
                    last = frame(&mut cx, pointer, pressed);
                }
            }
            Step::Mark => marks.push(imageops::resize(
                &last,
                w / 4,
                h / 4,
                imageops::FilterType::Triangle,
            )),
        }
    }
    drop(frame);
    drop(pipe);
    assert!(ffmpeg.wait().unwrap().success(), "ffmpeg encodes the frames");
    pm_shell::drag::set_frame_time(None);
    eprintln!("{name}: {frames} frames, {:.1} s, {w}x{h}", frames as f32 / FPS);
    marks
}

#[test]
#[ignore = "writes the panel-drag recording; set PM_PANEL_DRAG_DIR"]
fn record_panel_dragging() {
    let dir = std::path::PathBuf::from(std::env::var("PM_PANEL_DRAG_DIR").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    objc2::rc::autoreleasepool(|_| {
        let rows = [
            record(Scheme::Light, "light", &dir),
            record(Scheme::Dark, "dark", &dir),
        ];
        // Contact sheet: the marked frames, light above dark.
        let (tw, th) = rows[0][0].dimensions();
        let columns = 4;
        let per = rows[0].len().div_ceil(columns) as u32;
        let gap = 8;
        let mut sheet = RgbaImage::from_pixel(
            columns as u32 * (tw + gap) + gap,
            2 * per * (th + gap) + gap,
            Rgba([32, 32, 34, 255]),
        );
        for (r, marks) in rows.iter().enumerate() {
            for (i, mark) in marks.iter().enumerate() {
                let x = gap + (i % columns) as u32 * (tw + gap);
                let y = gap + (r as u32 * per + (i / columns) as u32) * (th + gap);
                imageops::replace(&mut sheet, mark, x as i64, y as i64);
            }
        }
        sheet.save(dir.join("contact-sheet.png")).unwrap();
    });
}
