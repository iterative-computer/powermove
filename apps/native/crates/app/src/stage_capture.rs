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
    stage::{Change, CueSheet, Picture, Pictures, Scene, Stage},
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
        crate::stage_pictures::film(),
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
/// The stage with the film: `film` false shows the card alone.
pub(crate) fn open(cx: &mut HeadlessAppContext, tokens: Tokens, film: bool) -> WindowHandle<Stage> {
    cx.open_window(size(px(ROOM.0), px(ROOM.1)), move |_, cx| {
        let shared =
            crate::demo_editor::SharedPictures(Rc::new(std::cell::RefCell::new(pictures(ROOM))));
        let demo: Option<Box<dyn pm_shell::stage::Demo>> = film.then(|| {
            Box::new(crate::demo_editor::FilmDemo::start(
                crate::stage_pictures::film(),
                tokens.clone(),
                shared.clone(),
            )) as Box<dyn pm_shell::stage::Demo>
        });
        let stage = cx.new(|cx| {
            let mut stage = Stage::new(
                tokens.clone(),
                CueSheet::default(),
                Box::new(shared),
                demo,
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
        stage.update(cx, |s, cx| {
            s.clock = Some(t);
            s.settle(cx);
        });
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
        pacing(&out)?;
    }
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut cx = headless();
    let window = open(&mut cx, tokens.clone(), true);
    let sheet = CueSheet::default();
    // Named stills along the run, in film order (a beat's change happens
    // across frames, so the film is never jumped through).
    let mut stills: Vec<(String, f64)> = vec![
        ("00-house-lights".into(), 1.0),
        ("01-mark".into(), 3.0),
        ("02-fold".into(), 3.7),
        ("03-composer".into(), 4.6),
        ("04-editor-arrives".into(), 5.3),
    ];
    for (i, change) in Change::ALL.iter().enumerate() {
        let at = sheet.start_of(Scene::Beat(*change)).unwrap();
        for (k, (what, dt)) in [
            ("typing", 0.9),
            ("working", 1.7),
            ("wave", 2.4),
            ("landed", 3.4),
        ]
        .into_iter()
        .enumerate()
        {
            stills.push((format!("{}{k}-{change:?}-{what}", i + 1), at + dt));
        }
    }
    stills.push(("9-card".into(), sheet.length() + 1.5));
    let fps = if stills_only { 10. } else { 30. };
    let end = sheet.length() + 2.5;
    let first = frame_at(&mut cx, window, 0.);
    let (w, h) = (first.width(), first.height());
    let mut ffmpeg = (!stills_only).then(|| {
        std::process::Command::new("ffmpeg")
            .args([
                "-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s",
            ])
            .arg(format!("{w}x{h}"))
            .args([
                "-r", "30", "-i", "-", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18",
            ])
            .arg(out.join("stage-run.mp4"))
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("ffmpeg encodes the recording")
    });
    let mut stdin = ffmpeg.as_mut().map(|f| f.stdin.take().unwrap());
    let mut costs = Vec::new();
    let mut n = 0;
    let mut stepped = false;
    let mut next_still = 0;
    loop {
        let t = n as f64 / fps;
        if t > end + 3. {
            break;
        }
        if !stills_only && t >= sheet.length() + 1.2 && !stepped {
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
        if !stills_only && (t - end).abs() < 0.5 / fps {
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
        while next_still < stills.len() && stills[next_still].1 <= t + 0.5 / fps {
            image.save(out.join(format!("{}.png", stills[next_still].0)))?;
            next_still += 1;
        }
        if let Some(stdin) = stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(image.as_raw())?;
        }
        n += 1;
        if stills_only && next_still >= stills.len() {
            break;
        }
    }
    drop(stdin);
    if let Some(mut ffmpeg) = ffmpeg {
        anyhow::ensure!(ffmpeg.wait()?.success(), "ffmpeg failed");
    }
    costs.sort_by(f64::total_cmp);
    eprintln!(
        "{n} frames; offscreen frame incl. exact waits and readback: p50 {:.1} ms, p95 {:.1} ms",
        costs[costs.len() / 2],
        costs[costs.len() * 95 / 100],
    );
    Ok(())
}

/// The engine pictures each reel through the stage's path (no GPUI): what
/// a frame costs to encode and commit on the worker.
fn time_the_stage_pictures() {
    let mut pictures = pictures(ROOM);
    pictures.measure = true;
    let room_px = ((ROOM.0 * 2. * 0.5) as u32, (ROOM.1 * 2. * 0.5) as u32);
    let what = [(Picture::Backdrop, room_px), (Picture::Viewer, (1280, 720))];
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

/// The film at 120 Hz as the window runs it: pictures are not waited for
/// (a frame paints the newest one), and each frame's main-thread work
/// (layout, paint, the frame's own captures) is timed. Offscreen: no
/// display presents, so this is the work per frame against the 8.33 ms
/// interval, not a measure of presentation.
fn pacing(out: &std::path::Path) -> anyhow::Result<()> {
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut cx = headless();
    let window = open(&mut cx, tokens, true);
    cx.update_window(window.into(), |view, _, cx| {
        let stage = view.downcast::<Stage>().unwrap();
        stage.update(cx, |s, cx| s.settle(cx));
    })
    .unwrap();
    let sheet = CueSheet::default();
    let end = sheet.length() + 1.;
    let mut rows = Vec::new();
    let mut n = 0u32;
    loop {
        let t = f64::from(n) / 120.;
        if t > end {
            break;
        }
        let started = Instant::now();
        cx.update_window(window.into(), |view, w, cx| {
            let stage = view.downcast::<Stage>().unwrap();
            stage.update(cx, |s, _| s.clock = Some(t));
            w.refresh();
            w.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let ms = started.elapsed().as_secs_f64() * 1e3;
        let scene = format!("{:?}", sheet.at(t).scene);
        rows.push((t, scene, ms));
        n += 1;
        // Real time between frames, so the engine and the agent's tools
        // keep their own pace beside it.
        let spare = (1. / 120.) - started.elapsed().as_secs_f64();
        if spare > 0. {
            std::thread::sleep(std::time::Duration::from_secs_f64(spare));
        }
    }
    let mut report = String::from("t,scene,ms\n");
    for (t, scene, ms) in &rows {
        report.push_str(&format!("{t:.4},{scene},{ms:.2}\n"));
    }
    std::fs::write(out.join("pacing.csv"), report)?;
    let mut by: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for (_, scene, ms) in &rows {
        by.entry(scene.split('(').next().unwrap_or("").to_owned())
            .or_default()
            .push(*ms);
    }
    by.insert("all".into(), rows.iter().map(|r| r.2).collect());
    for (scene, mut v) in by {
        v.sort_by(f64::total_cmp);
        let over = v.iter().filter(|ms| **ms > 1000. / 120.).count();
        eprintln!(
            "pacing {scene:<12} frames {:>5}  p50 {:>6.2} ms  p95 {:>6.2}  p99 {:>6.2}  max {:>7.2}  over 8.33 ms: {over}",
            v.len(),
            v[v.len() / 2],
            v[v.len() * 95 / 100],
            v[v.len() * 99 / 100],
            v[v.len() - 1]
        );
    }
    Ok(())
}
