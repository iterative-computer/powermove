//! The first-run stage drawn offscreen (`--stage-capture <folder>`,
//! development and soak builds): stills of the intro, frames every quarter
//! second for the side-by-side with Dia, one recording of the whole run at
//! full size, and the run's pacing at 120 Hz. No window reaches the screen:
//! a headless GPUI window drawing with Metal.
use gpui::{prelude::*, *};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    stage::{CueSheet, Stage},
    welcome::{Button, Frame},
    welcome_view::WelcomeView,
};
use std::{rc::Rc, sync::Arc, time::Instant};

/// The 14-inch MacBook Pro's display, in points.
pub(crate) const ROOM: (f32, f32) = (1512., 982.);

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
/// The stage with the intro (`intro` false: the window at once). `exact`
/// waits for the light's renderer (recordings); otherwise a frame before it
/// is ready paints no room, as the window does.
pub(crate) fn open(
    cx: &mut HeadlessAppContext,
    tokens: Tokens,
    intro: bool,
    exact: bool,
) -> WindowHandle<Stage> {
    cx.open_window(size(px(ROOM.0), px(ROOM.1)), move |_, cx| {
        let stage = cx.new(|cx| {
            let mut stage = Stage::new(tokens.clone(), CueSheet::default(), intro, cx);
            stage.clock = Some(0.);
            stage.wait = exact;
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
/// A step's card on the stage.
fn step_card(cx: &mut App, tokens: &Tokens) -> Entity<WelcomeView> {
    cx.new(|cx| {
        let mut card = WelcomeView::new(tokens.clone(), timeline_frame(), Rc::new(|_, _| {}), cx);
        card.stage = true;
        card
    })
}
/// Stills of the intro, frames every quarter second over 0–6 s (the
/// side-by-side with Dia), and one recording of the whole run at full size
/// and 60 fps with a step and the hand-off, into `out`; `stills_only`
/// leaves out the frames and the recording. `reduced` runs with Reduce
/// Motion.
pub fn run(out: std::path::PathBuf, stills_only: bool, reduced: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(&out)?;
    let accessibility = Accessibility {
        reduce_motion: reduced,
        ..Accessibility::default()
    };
    let tokens = Tokens::new(Scheme::Dark, accessibility);
    let mut cx = headless();
    let window = open(&mut cx, tokens.clone(), true, true);
    let sheet = CueSheet::default();
    let wave = |p: f64| sheet.window + sheet.wave_seconds * p;
    let stills: Vec<(&str, f64)> = vec![
        ("1-credit", 2.6),
        ("2-mid-rise", 3.6),
        ("3-rest", sheet.hold - 0.05),
        ("4-mid-wave", wave(0.12)),
        ("4b-wave", wave(0.2)),
        ("5-window", sheet.length() + 0.3),
    ];
    let step_at = sheet.length() + 1.;
    let hand_off_at = step_at + 1.4;
    let end = if stills_only { sheet.length() + 0.4 } else { hand_off_at + 1.2 };
    let fps = 60.;
    let first = frame_at(&mut cx, window, 0.);
    let (w, h) = (first.width(), first.height());
    let mut ffmpeg = (!stills_only).then(|| {
        std::fs::create_dir_all(out.join("quarter")).ok();
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s"])
            .arg(format!("{w}x{h}"))
            .args([
                "-r", "60", "-i", "-", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "14",
            ])
            .arg(out.join("intro.mp4"))
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("ffmpeg encodes the recording")
    });
    let mut stdin = ffmpeg.as_mut().map(|f| f.stdin.take().unwrap());
    let (mut stepped, mut handed) = (false, false);
    let mut next_still = 0;
    let mut n = 0u32;
    loop {
        let t = f64::from(n) / fps;
        if t > end {
            break;
        }
        // Stills only: jump between the stills (the springs and the wave
        // are functions of the clock).
        if stills_only && next_still < stills.len() && t + 0.5 / fps < stills[next_still].1 - 1. / fps {
            n = (stills[next_still].1 * fps).round() as u32;
            continue;
        }
        if !stills_only && t >= step_at && !stepped {
            stepped = true;
            cx.update_window(window.into(), |view, _, cx| {
                let stage = view.downcast::<Stage>().unwrap();
                let card = step_card(cx, &tokens);
                stage.update(cx, |s, cx| {
                    s.clock = Some(t);
                    s.set_card(card, cx);
                });
            })
            .unwrap();
        }
        if !stills_only && t >= hand_off_at && !handed {
            handed = true;
            cx.update_window(window.into(), |view, _, cx| {
                let stage = view.downcast::<Stage>().unwrap();
                stage.update(cx, |s, cx| {
                    s.clock = Some(t);
                    s.hand_off(|_, _| {}, cx);
                });
            })
            .unwrap();
        }
        let image = frame_at(&mut cx, window, t);
        while next_still < stills.len() && stills[next_still].1 <= t + 0.5 / fps {
            image.save(out.join(format!("{}.png", stills[next_still].0)))?;
            next_still += 1;
        }
        if !stills_only && n.is_multiple_of(15) && t <= 6.0 + 1e-9 {
            image.save(out.join("quarter").join(format!("{:05.2}.png", t)))?;
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
    eprintln!("{n} frames at {w}x{h}");
    Ok(())
}

/// The run at 120 Hz as the window runs it: the room is not waited for (a
/// frame paints the newest one), and each frame's main-thread work
/// (layout, paint, the hold's GPU snapshots, the wave's preparation and
/// encoding) is timed, then the GPU is waited for so the next frame starts
/// clean. Offscreen: this is the work per frame against the 8.33 ms
/// interval, not a measure of presentation.
pub fn pacing(out: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out)?;
    let mut cx = headless();
    let window = open(
        &mut cx,
        Tokens::new(Scheme::Dark, Accessibility::default()),
        true,
        false,
    );
    let sheet = CueSheet::default();
    // Warm: the shaders compile on their workers and the window draws once.
    for t in [0., 0.1] {
        frame_at(&mut cx, window, t);
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    // The intro, the window's wave, a step in the window, and the hand-off.
    let step_at = sheet.length() + 0.6;
    let hand_off_at = step_at + 1.2;
    let end = hand_off_at + 1.2;
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut rows = Vec::new();
    let mut n = 0u32;
    let (mut stepped, mut handed) = (false, false);
    loop {
        let t = f64::from(n) / 120.;
        if t > end {
            break;
        }
        let started = Instant::now();
        let cpu = thread_cpu();
        if t >= step_at && !stepped {
            stepped = true;
            cx.update_window(window.into(), |view, _, cx| {
                let stage = view.downcast::<Stage>().unwrap();
                let card = step_card(cx, &tokens);
                stage.update(cx, |s, cx| {
                    s.clock = Some(t);
                    s.set_card(card, cx);
                });
            })
            .unwrap();
        }
        if t >= hand_off_at && !handed {
            handed = true;
            cx.update_window(window.into(), |view, _, cx| {
                let stage = view.downcast::<Stage>().unwrap();
                stage.update(cx, |s, cx| {
                    s.clock = Some(t);
                    s.hand_off(|_, _| {}, cx);
                });
            })
            .unwrap();
        }
        cx.update_window(window.into(), |view, w, cx| {
            let stage = view.downcast::<Stage>().unwrap();
            stage.update(cx, |s, _| s.clock = Some(t));
            w.refresh();
            w.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
        let ms = started.elapsed().as_secs_f64() * 1e3;
        let cpu = (thread_cpu() - cpu) * 1e3;
        let phase = if t >= hand_off_at {
            "HandOff".to_owned()
        } else if t >= step_at {
            "Step".to_owned()
        } else {
            format!("{:?}", sheet.phase(t))
        };
        rows.push((t, phase, ms, cpu));
        n += 1;
        let spare = (1. / 120.) - started.elapsed().as_secs_f64();
        if spare > 0. {
            std::thread::sleep(std::time::Duration::from_secs_f64(spare));
        }
    }
    let mut report = String::from("t,phase,ms,cpu_ms\n");
    for (t, phase, ms, cpu) in &rows {
        report.push_str(&format!("{t:.4},{phase},{ms:.3},{cpu:.3}\n"));
    }
    std::fs::write(out.join("pacing.csv"), report)?;
    // Wall time includes the shared machine's preemption; the main
    // thread's own CPU time is the work itself.
    for (label, pick) in [("wall", 2usize), ("cpu", 3)] {
        let value = |r: &(f64, String, f64, f64)| if pick == 2 { r.2 } else { r.3 };
        let mut by: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
        for r in &rows {
            by.entry(r.1.clone()).or_default().push(value(r));
        }
        by.insert("all".into(), rows.iter().map(value).collect());
        let worst = rows.iter().fold(
            (0., 0.),
            |w: (f64, f64), r| {
                if value(r) > w.1 { (r.0, value(r)) } else { w }
            },
        );
        for (phase, mut v) in by {
            v.sort_by(f64::total_cmp);
            let over = v.iter().filter(|ms| **ms > 1000. / 120.).count();
            eprintln!(
                "pacing {label:<4} {phase:<8} frames {:>5}  p50 {:>6.2} ms  p95 {:>6.2}  p99 {:>6.2}  max {:>7.2}  over 8.33 ms: {over}",
                v.len(),
                v[v.len() / 2],
                v[v.len() * 95 / 100],
                v[v.len() * 99 / 100],
                v[v.len() - 1]
            );
        }
        eprintln!(
            "pacing {label:<4} worst frame at t={:.3} s: {:.2} ms",
            worst.0, worst.1
        );
    }
    Ok(())
}

/// The calling thread's CPU time, in seconds.
fn thread_cpu() -> f64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid out-pointer for the call.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 * 1e-9
}
