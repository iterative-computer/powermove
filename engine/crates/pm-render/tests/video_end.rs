//! The last frames of a video clip (Phase 2 gate D1): today's
//! `videoPlaybackTime(asset, time, endPadding)` keeps a source time at
//! least `endPadding` before the clip's end. The viewer's compositor seeks
//! with 0.04 s; exports and agent stills run `prepareFrame` first, which
//! passes `1 / PM.proj.fps`, so they keep every frame of a clip that ends
//! with the composition. `RenderRequest::video_end` carries the output kind;
//! these tests render a 20-frame H.264 clip of 20 distinct levels (the clip
//! `e2e/export-video-frames.spec.ts` makes) in a project of its length at
//! 10, 24, 25, 30, and 60 fps and read back which source frame each output
//! frame shows, against today's rule for each kind.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::frame::FramePurpose;
use pm_render::{MapResolver, RenderRequest, Renderer, VideoEnd};
use serde_json::json;
use support::doc::*;
use support::ticks;

const FRAMES: usize = 20;
const SIDE: u32 = 64;

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
        .args(["-i", "pipe:0", "-c:v", "h264_videotoolbox", "-allow_sw", "1"])
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

/// The project the spec builds: `fps`, 20 frames long, one video layer
/// over the whole frame (centered: the anchor is the layer's center) for
/// the whole composition.
fn document(fps: u32) -> pm_model::Document {
    let dur = FRAMES as f64 / f64::from(fps);
    let v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": SIDE, "h": SIDE, "trim": 0, "speed": 1 }),
        f64::from(SIDE) / 2.0,
        f64::from(SIDE) / 2.0,
        dur,
    );
    let mut doc = project(SIDE, SIDE, dur, no_fill(), vec![v]);
    doc["fps"] = json!(fps);
    asset(&mut doc, "clip", "video", "mp4", SIDE, SIDE, dur);
    doc["assets"]["clip"]["fps"] = json!(fps);
    load(&doc)
}

/// The source frame each composition frame shows, read from the center
/// pixel's level.
fn shown(
    renderer: &mut Renderer,
    fps: u32,
    video_end: VideoEnd,
    path: &std::path::Path,
) -> Vec<usize> {
    let doc = document(fps);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    let request = RenderRequest {
        video_end,
        ..RenderRequest::at_scale(Size::new(SIDE, SIDE), 1.0, Precision::Float16)
    };
    (0..FRAMES)
        .map(|n| {
            let t = n as f64 / f64::from(fps);
            let frame = renderer
                .render(&doc, "Ptest-root", ticks(t), &request, &resolver)
                .expect("render");
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let rgba = renderer.read_rgba(&frame).unwrap();
            let center = ((SIDE / 2) * SIDE + SIDE / 2) as usize * 4;
            let code = f64::from(rgba[center]) * 255.0;
            let (i, error) = (0..FRAMES)
                .map(|i| (i, (code - level(i)).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(error < 5.0, "{fps} fps frame {n}: code {code:.1}");
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
fn exports_keep_the_last_frames_and_the_viewer_pads_like_today() {
    let dir = std::env::temp_dir().join(format!("pm-render-video-end-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let every: Vec<usize> = (0..FRAMES).collect();
    for fps in [10, 24, 25, 30, 60] {
        let path = dir.join(format!("steps-{fps}.mp4"));
        steps_clip(&path, fps);
        // Exports and agent stills: `prepareFrame`'s `1 / fps`.
        let final_frames = shown(&mut renderer, fps, VideoEnd::Final, &path);
        assert_eq!(
            final_frames,
            today(fps, 1.0 / f64::from(fps)),
            "{fps} fps, final"
        );
        assert_eq!(final_frames, every, "{fps} fps: an export keeps every frame");
        // The viewer: the compositor's 0.04 s.
        let preview = shown(&mut renderer, fps, VideoEnd::Preview, &path);
        assert_eq!(preview, today(fps, 0.04), "{fps} fps, preview");
    }
    std::fs::remove_dir_all(&dir).ok();
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
    assert_eq!(VideoEnd::of(FramePurpose::Export), VideoEnd::Final);
    assert_eq!(VideoEnd::of(FramePurpose::AgentCapture), VideoEnd::Final);
    assert_eq!(VideoEnd::of(FramePurpose::Viewer), VideoEnd::Preview);
    assert_eq!(VideoEnd::of(FramePurpose::Scrub), VideoEnd::Preview);
    assert_eq!(VideoEnd::of(FramePurpose::Thumbnail), VideoEnd::Preview);
    let sized = RenderRequest::sized(Size::new(SIDE, SIDE), Precision::Float16);
    assert_eq!(sized.video_end, VideoEnd::Preview);
}
