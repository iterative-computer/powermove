//! The first-run stage drawn offscreen with the real engine and the app's
//! fonts (`--stage-capture <folder>`, development and soak builds): stills
//! of the intro, one recording of the whole run, the room's frame costs and
//! the run's pacing at 120 Hz. No window reaches the screen: a headless GPUI window and the
//! engine's offscreen frames. The font registry answers on this, the main,
//! thread.
use crate::stage_pictures::EnginePictures;
use gpui::{prelude::*, *};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    stage::{CueSheet, Phase, Stage},
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
    let cx = HeadlessAppContext::with_platform(
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
/// The stage with the intro (`intro` false: the window at once). `exact`
/// waits for each room picture (recordings); otherwise frames paint the
/// newest one, as the window does.
pub(crate) fn open(
    cx: &mut HeadlessAppContext,
    tokens: Tokens,
    intro: bool,
    exact: bool,
) -> WindowHandle<Stage> {
    cx.open_window(size(px(ROOM.0), px(ROOM.1)), move |_, cx| {
        let mut room = pictures(ROOM);
        if !exact {
            room.exact = None;
        }
        let stage = cx.new(|cx| {
            let mut stage = Stage::new(
                tokens.clone(),
                CueSheet::default(),
                Box::new(room),
                intro,
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
/// Stills of every scene and wave, one recording of the whole run (raw
/// frames encoded by ffmpeg), and the pictures' costs, into `out`.
/// `stills_only` leaves out the recording and the timing.
/// Stills of the intro, one recording of the whole run (raw frames encoded
/// by ffmpeg) with a step and the hand-off, the room's costs and the
/// pacing, into `out`.
pub fn run(out: std::path::PathBuf, stills_only: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    if !stills_only {
        time_the_room();
        pacing(&out)?;
    }
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut cx = headless();
    let window = open(&mut cx, tokens.clone(), true, true);
    let sheet = CueSheet::default();
    let stills: Vec<(&str, f64)> = vec![
        ("0-dim", 0.8),
        ("1-credit", 2.2),
        ("2-mark-rising", sheet.rise.0 + 0.45),
        ("3-mark-settled", sheet.rise.1 + 0.3),
        ("4-light-spreads", sheet.hold),
        ("5-mid-wave", sheet.window + sheet.wave_seconds * 0.5),
        ("6-window", sheet.length() + 0.5),
    ];
    let fps = if stills_only { 20. } else { 30. };
    let end = sheet.length() + 3.;
    let first = frame_at(&mut cx, window, 0.);
    let (w, h) = (first.width(), first.height());
    let mut ffmpeg = (!stills_only).then(|| {
        std::process::Command::new("ffmpeg")
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
            .expect("ffmpeg encodes the recording")
    });
    let mut stdin = ffmpeg.as_mut().map(|f| f.stdin.take().unwrap());
    let mut n = 0;
    let mut stepped = false;
    let mut handed = false;
    let mut next_still = 0;
    loop {
        let t = f64::from(n) / fps;
        if t > end || stills_only && next_still >= stills.len() {
            break;
        }
        if !stills_only && t >= sheet.length() + 1. && !stepped {
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
        if !stills_only && t >= end - 1.5 && !handed {
            handed = true;
            cx.update_window(window.into(), |view, _, cx| {
                let stage = view.downcast::<Stage>().unwrap();
                stage.update(cx, |s, cx| {
                    s.clock = Some(t);
                    s.hand_off(None, |_, _| {}, cx);
                });
            })
            .unwrap();
        }
        let image = frame_at(&mut cx, window, t);
        while next_still < stills.len() && stills[next_still].1 <= t + 0.5 / fps {
            image.save(out.join(format!("{}.png", stills[next_still].0)))?;
            next_still += 1;
        }
        if let Some(stdin) = stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(image.as_raw())?;
        }
        n += 1;
    }
    drop(stdin);
    if let Some(mut ffmpeg) = ffmpeg {
        anyhow::ensure!(ffmpeg.wait()?.success(), "ffmpeg failed");
    }
    eprintln!("{n} frames recorded");
    Ok(())
}

/// The room through the stage's path (no GPUI): what a frame costs from
/// the request to the GPU's end, on the engine's worker.
fn time_the_room() {
    let mut room = pictures(ROOM);
    room.measure = true;
    let px_ = ((ROOM.0 * 2. * 0.5) as u32, (ROOM.1 * 2. * 0.5) as u32);
    use pm_shell::stage::Pictures;
    for i in 0..90 {
        let _ = room.picture(f64::from(i) / 24., px_);
    }
    let mut costs: Vec<f64> = room.measured.drain(..).skip(5).collect();
    costs.sort_by(f64::total_cmp);
    if let Some(max) = costs.last() {
        eprintln!(
            "room {}x{}: request to GPU end p50 {:.2} ms p95 {:.2} ms max {:.2} ms (worker, not the main thread)",
            px_.0,
            px_.1,
            costs[costs.len() / 2],
            costs[costs.len() * 95 / 100],
            max
        );
    }
}

/// The run at 120 Hz as the window runs it: the room is not waited for (a
/// frame paints the newest one), and each frame's main-thread work
/// (layout, paint, the hold's GPU snapshots, the wave's preparation and
/// encoding) is timed, then the GPU is waited for so the next frame starts
/// clean. Offscreen: this is the work per frame against the 8.33 ms
/// interval, not a measure of presentation.
pub fn pacing(out: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out)?;
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut cx = headless();
    let window = open(&mut cx, tokens, true, false);
    let sheet = CueSheet::default();
    // Warm: the room's engine starts and the window draws once.
    for t in [0., 0.1] {
        frame_at(&mut cx, window, t);
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
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
        rows.push((t, sheet.phase(t), ms));
        n += 1;
        let spare = (1. / 120.) - started.elapsed().as_secs_f64();
        if spare > 0. {
            std::thread::sleep(std::time::Duration::from_secs_f64(spare));
        }
    }
    let mut report = String::from("t,phase,ms\n");
    for (t, phase, ms) in &rows {
        report.push_str(&format!("{t:.4},{phase:?},{ms:.3}\n"));
    }
    std::fs::write(out.join("pacing.csv"), report)?;
    let mut by: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for (_, phase, ms) in &rows {
        by.entry(format!("{phase:?}")).or_default().push(*ms);
    }
    by.insert("all".into(), rows.iter().map(|r| r.2).collect());
    let worst = rows.iter().fold(
        (0., 0.),
        |w: (f64, f64), r| if r.2 > w.1 { (r.0, r.2) } else { w },
    );
    for (phase, mut v) in by {
        v.sort_by(f64::total_cmp);
        let over = v.iter().filter(|ms| **ms > 1000. / 120.).count();
        eprintln!(
            "pacing {phase:<8} frames {:>5}  p50 {:>6.2} ms  p95 {:>6.2}  p99 {:>6.2}  max {:>7.2}  over 8.33 ms: {over}",
            v.len(),
            v[v.len() / 2],
            v[v.len() * 95 / 100],
            v[v.len() * 99 / 100],
            v[v.len() - 1]
        );
    }
    eprintln!(
        "pacing worst frame at t={:.3} s: {:.2} ms",
        worst.0, worst.1
    );
    let _ = Phase::Window;
    Ok(())
}
