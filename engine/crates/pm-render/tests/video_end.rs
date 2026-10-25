//! Source timestamps select video frames on every output surface. Browser
//! seek padding must never hide the last frame or make preview differ from
//! export. Distinct grey frames prove selection at matching and mixed
//! source/composition rates, in direct layers and nested compositions.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::frame::FramePurpose;
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;
use support::ticks;

const FRAMES: usize = 20;
const SIDE: u32 = 64;

struct Clean(std::path::PathBuf);

impl Drop for Clean {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Source frame `i` is the grey level `20 + 10 i` (the spec's levels).
fn level(i: usize) -> f64 {
    20.0 + 10.0 * i as f64
}

fn ffmpeg() -> std::path::PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg")
}

/// A 20-frame 64 × 64 H.264 clip at `fps`, frame `i` at [`level`].
fn steps_clip(path: &std::path::Path, fps: u32) {
    use std::io::Write;
    let frame = (SIDE * SIDE * 3) as usize;
    let mut raw = Vec::with_capacity(FRAMES * frame);
    for i in 0..FRAMES {
        raw.extend(std::iter::repeat_n(level(i) as u8, frame));
    }
    let mut child = std::process::Command::new(ffmpeg())
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", &format!("{SIDE}x{SIDE}"), "-r", &fps.to_string()])
        .args([
            "-i",
            "pipe:0",
            "-c:v",
            "h264_videotoolbox",
            "-allow_sw",
            "1",
        ])
        .args(["-b:v", "8M", "-pix_fmt", "yuv420p"])
        .args(["-color_primaries", "bt709", "-color_trc", "bt709"])
        .args(["-colorspace", "bt709"])
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("ffmpeg (PM_FFMPEG_DIR)");
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    assert!(child.wait().unwrap().success());
}

/// The layer outlasts the source so the final frame must hold.
fn document(source_fps: u32, comp_fps: u32, nested: bool) -> serde_json::Value {
    let source_dur = FRAMES as f64 / f64::from(source_fps);
    let dur = source_dur + 1.0;
    let v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": SIDE, "h": SIDE, "trim": 0, "speed": 1 }),
        f64::from(SIDE) / 2.0,
        f64::from(SIDE) / 2.0,
        dur,
    );
    let mut doc = if nested {
        let pre = layer(
            "pre",
            "precomp",
            json!({"comp":"inner","w":SIDE,"h":SIDE}),
            0.0,
            0.0,
            dur,
        );
        let mut doc = project(SIDE, SIDE, dur, no_fill(), vec![pre]);
        composition(&mut doc, "inner", SIDE, SIDE, dur, vec![v]);
        doc["comps"]["inner"]["fps"] = json!(25);
        doc
    } else {
        project(SIDE, SIDE, dur, no_fill(), vec![v])
    };
    doc["fps"] = json!(comp_fps);
    asset(&mut doc, "clip", "video", "mp4", SIDE, SIDE, source_dur);
    doc["assets"]["clip"]["fps"] = json!(source_fps);
    doc
}

/// The source frame each composition frame shows, read from the center
/// pixel's level.
fn shown(
    renderer: &mut Renderer,
    source_fps: u32,
    document: &pm_model::Document,
    output_kind: OutputKind,
    path: &std::path::Path,
) -> Vec<usize> {
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    let request = RenderRequest {
        output_kind,
        export: output_kind == OutputKind::Final,
        ..RenderRequest::at_scale(Size::new(SIDE, SIDE), 1.0, Precision::Float16)
    };
    (0..FRAMES + 2)
        .map(|n| {
            let t = n as f64 / f64::from(source_fps);
            let frame = renderer
                .render(document, "Ptest-root", ticks(t), &request, &resolver)
                .expect("render");
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let rgba = renderer.read_rgba(&frame).unwrap();
            renderer.recycle(frame);
            let center = ((SIDE / 2) * SIDE + SIDE / 2) as usize * 4;
            let code = f64::from(rgba[center]) * 255.0;
            let (i, error) = (0..FRAMES)
                .map(|i| (i, (code - level(i)).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(error < 5.0, "{source_fps} fps frame {n}: code {code:.1}");
            i
        })
        .collect()
}

/// Today's answer, from `video-timing.ts`: the source time
/// `max(0, min(max(0, dur − endPadding), t))`, seeked on Chromium's
/// microsecond clock (`ceil(at · 1e6 − 1e-7) / 1e6`), showing the last
/// source frame that starts at or before it.
fn today(fps: u32, padding: f64) -> Vec<usize> {
    let rate = f64::from(fps);
    let dur = FRAMES as f64 / rate;
    let micro = |s: f64| 0f64.max((s * 1e6 - 1e-7).ceil() / 1e6);
    (0..FRAMES)
        .map(|n| {
            let t = n as f64 / rate;
            let at = 0f64.max((dur - padding).max(0.0).min(t));
            let seek = micro(at);
            (0..FRAMES)
                .rev()
                .find(|&k| micro(k as f64 / rate) <= seek)
                .unwrap()
        })
        .collect()
}

#[test]
fn every_surface_keeps_last_frames_at_matching_and_mixed_rates() {
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-video-end-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    // Separate surfaces cannot answer from each other's converted-frame cache.
    let mut preview = Renderer::headless().unwrap();
    let mut export = Renderer::headless().unwrap();
    let expected: Vec<usize> = (0..FRAMES + 2).map(|n| n.min(FRAMES - 1)).collect();
    for source_fps in [10, 24, 25, 30, 60, 120] {
        let path = dir.0.join(format!("steps-{source_fps}.mp4"));
        steps_clip(&path, source_fps);
        let mut rates = vec![source_fps, 10, 120];
        rates.sort_unstable();
        rates.dedup();
        for comp_fps in rates {
            for nested in [false, true] {
                let doc = load(&document(source_fps, comp_fps, nested));
                for (renderer, kind) in [
                    (&mut preview, OutputKind::Preview),
                    (&mut export, OutputKind::Final),
                ] {
                    assert_eq!(
                        shown(renderer, source_fps, &doc, kind, &path),
                        expected,
                        "{source_fps} fps source, {comp_fps} fps comp, nested={nested}, {kind:?}",
                    );
                }
            }
        }
        if source_fps == 60 {
            for nested in [false, true] {
                for (name, trim, speed, remap) in [
                    ("trimmed double speed", 5.0 / 60.0, 2.0, None),
                    ("reverse from final frame", 19.0 / 60.0, -1.0, None),
                    ("remapped final frame", 0.0, 0.0, Some(19.0 / 60.0)),
                ] {
                    let mut value = document(source_fps, 30, nested);
                    let video = if nested {
                        &mut value["comps"]["inner"]["layers"][0]
                    } else {
                        &mut value["layers"][0]
                    };
                    video["d"]["trim"] = json!(trim);
                    video["d"]["speed"] = json!(speed);
                    if let Some(t) = remap {
                        video["d"]["timeRemap"] = json!(true);
                        video["d"]["sourceTime"] = ch(json!(t));
                    }
                    let expected: Vec<usize> = (0..FRAMES + 2)
                        .map(|n| {
                            let source = remap.unwrap_or(trim + n as f64 / 60.0 * speed);
                            ((source * 60.0 + 1e-6).floor().max(0.0) as usize).min(FRAMES - 1)
                        })
                        .collect();
                    let doc = load(&value);
                    for (renderer, kind) in [
                        (&mut preview, OutputKind::Preview),
                        (&mut export, OutputKind::Final),
                    ] {
                        assert_eq!(
                            shown(renderer, source_fps, &doc, kind, &path),
                            expected,
                            "{name}, nested={nested}, {kind:?}"
                        );
                    }
                }
            }
        }
        let old = today(source_fps, 0.04);
        let mut errors: Vec<_> = old
            .iter()
            .enumerate()
            .map(|(n, &frame)| n.abs_diff(frame))
            .collect();
        errors.sort_unstable();
        eprintln!(
            "VIDEO-END {source_fps} fps: old max={} p99.9={} mean={:.3}; native=0,0,0 source frames",
            errors[FRAMES - 1],
            errors[FRAMES - 1],
            errors.iter().sum::<usize>() as f64 / FRAMES as f64,
        );
    }
}

/// A held decoded source reuses its filtered texels. A new decoded frame,
/// including one uploaded into recycled texture storage, invalidates them.
#[test]
fn minified_video_reuses_only_the_same_source_revision() {
    use pm_perf::Counter;
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-video-mips-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("steps.mp4");
    steps_clip(&path, 24);
    let doc = load(&document(24, 24, false));
    let mut assets = MapResolver::default();
    assets.paths.insert("clip".into(), path);
    let mut renderer = Renderer::headless().unwrap();
    let request = RenderRequest {
        sampling: pm_render::sampling::Sampling::Playback,
        ..RenderRequest::at_scale(Size::new(8, 8), 0.125, Precision::Float16)
    };
    for source in [0, 1, 2, 8, 19] {
        let time = source as f64 / 24.0;
        let before = pm_perf::snapshot();
        let frame = renderer
            .render(&doc, "Ptest-root", ticks(time), &request, &assets)
            .unwrap();
        let allocations = pm_perf::snapshot()
            .since(&before)
            .get(Counter::PyramidAllocs);
        if source > 0 {
            assert_eq!(allocations, 0, "source {source} should reuse an idle tail");
        }
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let actual = renderer.read_rgba(&frame).unwrap();
        renderer.recycle(frame);
        for px in actual.chunks_exact(4) {
            assert!(
                (f64::from(px[0]) * 255.0 - level(source)).abs() < 5.0,
                "source {source}: {px:?}"
            );
        }
        let before = pm_perf::snapshot();
        let held = renderer
            .render(&doc, "Ptest-root", ticks(time + 0.01), &request, &assets)
            .unwrap();
        let work = pm_perf::snapshot().since(&before);
        assert!(held.failures.is_empty(), "{:?}", held.failures);
        assert_eq!(
            renderer.read_rgba(&held).unwrap(),
            actual,
            "source {source}"
        );
        renderer.recycle(held);
        assert!(
            work.get(Counter::RenderPasses) <= 2,
            "held source {source} regenerated mip texels: {} passes",
            work.get(Counter::RenderPasses)
        );
        assert_eq!(work.get(Counter::PyramidAllocs), 0);
    }
}

#[test]
fn todays_padding_hides_the_last_frames_only_in_the_viewer_above_25_fps() {
    // The legacy rule itself, as the gate measured it: 0.04 s hides the
    // last frame at 30 fps and the last two at 60 fps; 1 / fps hides none.
    let last = |fps: u32, padding: f64| today(fps, padding)[FRAMES - 1];
    for fps in [10, 24, 25] {
        assert_eq!(last(fps, 0.04), FRAMES - 1);
    }
    assert_eq!(today(30, 0.04)[FRAMES - 2..], [18, 18]);
    assert_eq!(today(60, 0.04)[FRAMES - 3..], [17, 17, 17]);
    for fps in [10, 24, 25, 30, 60] {
        assert_eq!(last(fps, 1.0 / f64::from(fps)), FRAMES - 1);
    }
}

#[test]
fn output_kinds_follow_the_frame_purpose() {
    assert_eq!(OutputKind::of(FramePurpose::Export), OutputKind::Final);
    assert_eq!(
        OutputKind::of(FramePurpose::AgentCapture),
        OutputKind::Final
    );
    assert_eq!(OutputKind::of(FramePurpose::Viewer), OutputKind::Preview);
    assert_eq!(OutputKind::of(FramePurpose::Scrub), OutputKind::Preview);
    assert_eq!(OutputKind::of(FramePurpose::Thumbnail), OutputKind::Preview);
    let sized = RenderRequest::sized(Size::new(SIDE, SIDE), Precision::Float16);
    assert_eq!(sized.output_kind, OutputKind::Preview);
}
