//! A clip's time 0 is its first presented video frame, whatever timestamp
//! the container gives it. Transport streams start around 1.4 s, program
//! streams and fragmented MP4 at smaller offsets; none of that may show as
//! a blank head or cost the tail. Distinct grey frames prove which source
//! frame each composition frame shows, in the viewer and in export.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;
use support::ticks;

const FRAMES: usize = 20;
const SIDE: u32 = 64;
const FPS: u32 = 25;

struct Clean(std::path::PathBuf);

impl Drop for Clean {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Source frame `i` is the grey level `20 + 10 i`.
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

/// A 20-frame 64 × 64 clip, frame `i` at [`level`], muxed as `path`'s
/// extension says with `mux` options.
fn steps_clip(path: &std::path::Path, codec: &[&str], mux: &[&str]) {
    use std::io::Write;
    let frame = (SIDE * SIDE * 3) as usize;
    let mut raw = Vec::with_capacity(FRAMES * frame);
    for i in 0..FRAMES {
        raw.extend(std::iter::repeat_n(level(i) as u8, frame));
    }
    let mut child = std::process::Command::new(ffmpeg())
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24"])
        .args(["-s", &format!("{SIDE}x{SIDE}"), "-r", &FPS.to_string()])
        .args(["-i", "pipe:0"])
        .args(codec)
        .args(["-pix_fmt", "yuv420p"])
        .args(["-color_primaries", "bt709", "-color_trc", "bt709"])
        .args(["-colorspace", "bt709"])
        .args(mux)
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("ffmpeg (PM_FFMPEG_DIR)");
    child.stdin.take().unwrap().write_all(&raw).unwrap();
    assert!(child.wait().unwrap().success());
}

/// The layer outlasts the source so the final frame must hold.
fn document(trim: f64, speed: f64) -> serde_json::Value {
    let source_dur = FRAMES as f64 / f64::from(FPS);
    let dur = source_dur + 1.0;
    let v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": SIDE, "h": SIDE, "trim": trim, "speed": speed }),
        f64::from(SIDE) / 2.0,
        f64::from(SIDE) / 2.0,
        dur,
    );
    let mut doc = project(SIDE, SIDE, dur, no_fill(), vec![v]);
    doc["fps"] = json!(FPS);
    asset(&mut doc, "clip", "video", "mp4", SIDE, SIDE, source_dur);
    doc["assets"]["clip"]["fps"] = json!(FPS);
    doc
}

/// The source frame each composition frame shows (`None`: nothing drawn),
/// read from the center pixel's level.
fn shown(
    renderer: &mut Renderer,
    document: &pm_model::Document,
    output_kind: OutputKind,
    path: &std::path::Path,
) -> Vec<Option<usize>> {
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    let request = RenderRequest {
        output_kind,
        export: output_kind == OutputKind::Final,
        ..RenderRequest::at_scale(Size::new(SIDE, SIDE), 1.0, Precision::Float16)
    };
    (0..FRAMES + 2)
        .map(|n| {
            let t = n as f64 / f64::from(FPS);
            let frame = renderer
                .render(document, "Ptest-root", ticks(t), &request, &resolver)
                .expect("render");
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let rgba = renderer.read_rgba(&frame).unwrap();
            renderer.recycle(frame);
            let center = ((SIDE / 2) * SIDE + SIDE / 2) as usize * 4;
            if rgba[center + 3] < 0.5 {
                return None;
            }
            let code = f64::from(rgba[center]) * 255.0;
            let (i, error) = (0..FRAMES)
                .map(|i| (i, (code - level(i)).abs()))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(error < 5.0, "frame {n}: code {code:.1}");
            Some(i)
        })
        .collect()
}

#[test]
fn a_late_first_timestamp_is_clip_time_zero_on_every_surface() {
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-source-origin-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let h264: &[&str] = &["-c:v", "h264_videotoolbox", "-allow_sw", "1", "-b:v", "8M"];
    let mpeg4: &[&str] = &["-c:v", "mpeg4", "-q:v", "2", "-g", "6", "-bf", "2"];
    let mpeg2: &[&str] = &["-c:v", "mpeg2video", "-q:v", "2", "-g", "6", "-bf", "2"];
    let cases: [(&str, &[&str], &[&str]); 5] = [
        // A camera transport stream: FFmpeg's muxer starts at 1.4 s.
        ("camera.ts", h264, &[]),
        // The AVCHD wrapper (192-byte packets).
        (
            "camera.m2ts",
            h264,
            &["-f", "mpegts", "-mpegts_m2ts_mode", "1"],
        ),
        // MPEG-2 with B-frames: the first presented frame is not the first
        // timestamp the demuxer reads.
        ("broadcast.ts", mpeg2, &[]),
        // Fragmented MP4 with reordered frames: no edit list hides the
        // reorder delay, so the first frame is stamped after zero.
        (
            "fragmented.mp4",
            mpeg4,
            &["-movflags", "frag_keyframe+empty_moov+default_base_moof"],
        ),
        // A plain MP4 whose edit list delays the picture.
        ("delayed.mp4", h264, &["-output_ts_offset", "0.52"]),
    ];
    // Separate surfaces cannot answer from each other's converted-frame cache.
    let mut preview = Renderer::headless().unwrap();
    let mut export = Renderer::headless().unwrap();
    for (name, codec, mux) in cases {
        let path = dir.0.join(name);
        steps_clip(&path, codec, mux);
        let source = pm_media::decode::service::MediaSource::open(&path)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(source.index.frames.len(), FRAMES, "{name}");
        if name != "delayed.mp4" {
            assert!(
                source.index.frames[0].pts > 0,
                "{name}: the fixture must start after zero to prove anything"
            );
        }
        for (what, trim, speed) in [
            ("whole clip", 0.0, 1.0),
            ("trimmed", 5.0 / 25.0, 1.0),
            ("double speed", 2.0 / 25.0, 2.0),
            ("reverse from the final frame", 19.0 / 25.0, -1.0),
        ] {
            let expected: Vec<Option<usize>> = (0..FRAMES + 2)
                .map(|n| {
                    let source = trim + n as f64 / 25.0 * speed;
                    Some(((source * 25.0 + 1e-6).floor().max(0.0) as usize).min(FRAMES - 1))
                })
                .collect();
            let doc = load(&document(trim, speed));
            for (renderer, kind) in [
                (&mut preview, OutputKind::Preview),
                (&mut export, OutputKind::Final),
            ] {
                assert_eq!(
                    shown(renderer, &doc, kind, &path),
                    expected,
                    "{name}, {what}, {kind:?}"
                );
            }
        }
    }
}

/// A movie with a single frame is a clip like any other: it opens, shows
/// its frame at time 0 and holds it.
#[test]
fn a_one_frame_movie_shows_its_frame() {
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-one-frame-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let h264: &[&str] = &["-c:v", "h264_videotoolbox", "-allow_sw", "1", "-b:v", "8M"];
    let mut preview = Renderer::headless().unwrap();
    let mut export = Renderer::headless().unwrap();
    for (name, mux) in [
        ("one.mp4", &["-frames:v", "1"][..]),
        ("one.mov", &["-frames:v", "1"]),
        ("one.ts", &["-frames:v", "1"]),
    ] {
        let path = dir.0.join(name);
        steps_clip(&path, h264, mux);
        let source = pm_media::decode::service::MediaSource::open(&path)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(source.index.frames.len(), 1, "{name}");
        // MP4 and QuickTime give the frame a duration; a lone
        // transport-stream picture states no rate and still opens.
        if name != "one.ts" {
            assert_eq!(source.rate(), pm_time::Rate::FPS_25, "{name}");
        }
        let doc = load(&document(0.0, 1.0));
        for (renderer, kind) in [
            (&mut preview, OutputKind::Preview),
            (&mut export, OutputKind::Final),
        ] {
            assert_eq!(
                shown(renderer, &doc, kind, &path),
                vec![Some(0); FRAMES + 2],
                "{name}, {kind:?}"
            );
        }
    }
}
