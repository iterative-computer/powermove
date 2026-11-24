//! The first-run stage drawn offscreen with the real engine and the app's
//! fonts (`--stage-capture <folder>`, development builds): a still of every
//! scene and wave, one recording of the whole run, and the pictures' frame
//! costs. No window reaches the screen: a headless GPUI window and the
//! engine's offscreen frames. The font registry answers on this, the main,
//! thread.
use crate::stage_pictures::EnginePictures;
use gpui::{prelude::*, *};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    stage::{CueSheet, Picture, Pictures, Reel, Stage},
    welcome::{Button, Frame},
    welcome_view::WelcomeView,
};
use std::{rc::Rc, sync::Arc, time::Instant};

/// The 14-inch MacBook Pro's display, in points.
pub(crate) const ROOM: (f32, f32) = (1512., 982.);

/// The app's fonts, answered on the main thread; without a registry (a test
/// thread) every ask is refused and text draws nothing.
fn fonts() -> (pm_render::text::host::RenderFonts, Box<dyn FnMut()>) {
    let broker = pm_text::host::FontBroker::new();
    broker.set_wake(Arc::new(|| {}));
    let fonts = pm_render::text::host::RenderFonts::new(broker.clone());
    match pm_text::host::FontService::new() {
        Ok(mut service) => {
            broker.set_generation(service.generation());
            let pump = Box::new(move || pm_text::host::serve(&mut service, broker.take_asks()));
            (fonts, pump)
        }
        Err(_) => {
            broker.close();
            (fonts, Box::new(|| {}))
        }
    }
}
pub(crate) fn pictures(room: (f32, f32)) -> EnginePictures {
    let (fonts, pump) = fonts();
    let mut pictures = EnginePictures::start(
        fonts,
        Some(std::env::temp_dir().join("pm-stage-shader-cache")),
        (f64::from(room.0), f64::from(room.1)),
    );
    pictures.exact = Some(pump);
    pictures
}
pub(crate) fn welcome_frame() -> Frame {
    Frame {
        surface: "S107".into(),
        animate: true,
        title: "Welcome to\nPowermove.".into(),
        centred: true,
        buttons: vec![Button::new("begin", "Begin")],
        default: Some("begin".into()),
        ..Frame::default()
    }
}
pub(crate) fn timeline_frame() -> Frame {
    Frame {
        surface: "S108".into(),
        back: Some("Back to Welcome".into()),
        title: "Pick Your Timeline.".into(),
        line: "Work the way you already do. You can switch anytime from the timeline panel.".into(),
        buttons: vec![Button::new("continue", "Continue")],
        default: Some("continue".into()),
        ..Frame::default()
    }
}
pub(crate) fn headless() -> HeadlessAppContext {
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
    cx.allow_parking();
    cx
}
pub(crate) fn open(cx: &mut HeadlessAppContext, tokens: Tokens) -> WindowHandle<Stage> {
    cx.open_window(size(px(ROOM.0), px(ROOM.1)), move |_, cx| {
        let pictures = pictures(ROOM);
        let stage = cx.new(|cx| {
            let mut stage = Stage::new(
                tokens.clone(),
                CueSheet::default(),
                Box::new(pictures),
                true,
                cx,
            );
            stage.clock = Some(0.);
            stage
        });
        let card = cx.new(|cx| {
            let mut card = WelcomeView::new(tokens, welcome_frame(), Rc::new(|_, _| {}), cx);
            card.stage = true;
            card
        });
        stage.update(cx, |s, cx| s.set_card(card, cx));
        stage
    })
    .unwrap()
}
pub(crate) fn frame_at(
    cx: &mut HeadlessAppContext,
    window: WindowHandle<Stage>,
    t: f64,
) -> image::RgbaImage {
    cx.update_window(window.into(), |view, w, cx| {
        let stage = view.downcast::<Stage>().unwrap();
        stage.update(cx, |s, _| s.clock = Some(t));
        w.refresh();
        w.draw(cx).clear(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.capture_screenshot(window.into()).unwrap()
}
pub(crate) fn luma(image: &image::RgbaImage, x: f32, y: f32) -> f32 {
    let p = image.get_pixel(
        ((x * image.width() as f32) as u32).min(image.width() - 1),
        ((y * image.height() as f32) as u32).min(image.height() - 1),
    );
    (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])) / 255.
}
pub(crate) fn mean(image: &image::RgbaImage) -> f32 {
    let mut sum = 0.;
    for p in image.pixels() {
        sum += f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]);
    }
    sum / (image.width() * image.height() * 3) as f32 / 255.
}

/// Stills of every scene and wave, one recording of the whole run (raw
/// frames encoded by ffmpeg), and the pictures' costs, into `out`.
/// `stills_only` leaves out the recording and the timing.
pub fn run(out: std::path::PathBuf, stills_only: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    if !stills_only {
        time_the_stage_pictures();
    }
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut cx = headless();
    let window = open(&mut cx, tokens.clone());
    let sheet = CueSheet::default();
    // One still per scene, mid-scene, and mid-wave for each wave.
    let mut stills = vec![
        ("0-house-lights", 1.0),
        ("1-title-card", 3.2),
        ("2-mark", 6.4),
    ];
    for (i, reel) in Reel::ALL.iter().enumerate() {
        let at = sheet.start_of(pm_shell::stage::Scene::Reel(*reel)).unwrap();
        stills.push((
            ["3-reel-cut", "4-reel-ask", "5-reel-grade", "6-reel-depth"][i],
            at + 2.2,
        ));
        stills.push((
            [
                "w1-wave-into-cut",
                "w2-wave-into-ask",
                "w3-wave-into-grade",
                "w4-wave-into-depth",
            ][i],
            at - 0.45,
        ));
    }
    stills.push(("w5-wave-into-card", sheet.length() - 0.45));
    stills.push(("7-card", sheet.length() + 1.5));
    for (name, t) in &stills {
        frame_at(&mut cx, window, t - 1. / 60.);
        let image = frame_at(&mut cx, window, *t);
        image.save(out.join(format!("{name}.png"))).unwrap();
    }
    if stills_only {
        return Ok(());
    }
    // The whole run at 30 fps, then a step on the card and the hand-off.
    let fps = 30.;
    let end = sheet.length() + 2.5;
    let first = frame_at(&mut cx, window, 0.);
    let (w, h) = (first.width(), first.height());
    let mut ffmpeg = std::process::Command::new("ffmpeg")
        .args([
            "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s",
        ])
        .arg(format!("{w}x{h}"))
        .args([
            "-r", "30", "-i", "-", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "16",
        ])
        .arg(out.join("stage-run.mp4"))
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("ffmpeg encodes the recording");
    let mut stdin = ffmpeg.stdin.take().unwrap();
    let mut costs = Vec::new();
    let mut n = 0;
    let mut stepped = false;
    loop {
        let t = n as f64 / fps;
        if t > end + 3. {
            break;
        }
        if t >= sheet.length() + 1.2 && !stepped {
            stepped = true;
            cx.update_window(window.into(), |view, _, cx| {
                let stage = view.downcast::<Stage>().unwrap();
                let card = cx.new(|cx| {
                    let mut card =
                        WelcomeView::new(tokens.clone(), timeline_frame(), Rc::new(|_, _| {}), cx);
                    card.stage = true;
                    card
                });
                stage.update(cx, |s, cx| {
                    s.clock = Some(t);
                    s.set_card(card, cx);
                });
            })
            .unwrap();
        }
        if (t - end).abs() < 0.5 / fps {
            cx.update_window(window.into(), |view, _, cx| {
                let stage = view.downcast::<Stage>().unwrap();
                stage.update(cx, |s, cx| {
                    s.clock = Some(t);
                    s.hand_off(None, |_, _| {}, cx);
                });
            })
            .unwrap();
        }
        let started = Instant::now();
        let image = frame_at(&mut cx, window, t);
        costs.push(started.elapsed().as_secs_f64() * 1e3);
        use std::io::Write;
        stdin.write_all(image.as_raw()).unwrap();
        n += 1;
    }
    drop(stdin);
    assert!(ffmpeg.wait().unwrap().success());
    costs.sort_by(f64::total_cmp);
    eprintln!(
        "recorded {n} frames; offscreen frame incl. exact engine wait and readback: p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        costs[costs.len() / 2],
        costs[costs.len() * 95 / 100],
        costs[costs.len() - 1]
    );
    Ok(())
}

/// The engine pictures each reel through the stage's path (no GPUI): what
/// a frame costs to encode and commit on the worker.
fn time_the_stage_pictures() {
    let mut pictures = pictures(ROOM);
    pictures.measure = true;
    let screen = Stage::screen(size(px(ROOM.0), px(ROOM.1)));
    let reel_px = (
        (f32::from(screen.size.width) * 2.) as u32,
        (f32::from(screen.size.height) * 2.) as u32,
    );
    let room_px = ((ROOM.0 * 2. * 0.5) as u32, (ROOM.1 * 2. * 0.5) as u32);
    let what: Vec<(Picture, (u32, u32))> = std::iter::once((Picture::Backdrop, room_px))
        .chain(Reel::ALL.iter().map(|r| (Picture::Reel(*r), reel_px)))
        .collect();
    for (picture, px_) in what {
        for i in 0..90 {
            let _ = pictures.picture(picture, i as f64 / 120. * 3., px_);
        }
        let mut costs: Vec<f64> = pictures
            .measured
            .drain(..)
            .filter(|(p, _)| *p == picture)
            .map(|(_, ms)| ms)
            .skip(5)
            .collect();
        costs.sort_by(f64::total_cmp);
        if costs.is_empty() {
            continue;
        }
        eprintln!(
            "{picture:?} {}x{}: request to GPU end p50 {:.2} ms p95 {:.2} ms max {:.2} ms ({} frames)",
            px_.0,
            px_.1,
            costs[costs.len() / 2],
            costs[costs.len() * 95 / 100],
            costs[costs.len() - 1],
            costs.len()
        );
    }
    assert!(pictures.failure().is_none(), "{:?}", pictures.failure());
}
