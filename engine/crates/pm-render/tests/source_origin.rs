//! What an asset's source times select of its file's picture.
//!
//! With `timing: 2` a clip's time 0 is the start of its file's presentation
//! (its first presented video frame unless a soundtrack starts more than a
//! frame earlier), whatever timestamp the container gives it. Transport streams start around 1.4 s, program
//! streams and fragmented MP4 at smaller offsets; none of that may show as
//! a blank head or cost the tail. Distinct grey frames prove which source
//! frame each composition frame shows, in the viewer and in export.
//!
//! Without the member the asset is an older project's: every trim, speed
//! change and remap key shows the frame the earlier app showed for it
//! (measured in that app's decoder): the frame whose container time covers
//! the source time, the first frame held before its own time, and a sample
//! straddling a head-trimming edit left out.

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

/// The layer outlasts the source so the final frame must hold. `timing` is
/// the asset record's member.
fn document(trim: f64, speed: f64, timing: Option<u64>) -> serde_json::Value {
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
    if let Some(timing) = timing {
        doc["assets"]["clip"]["timing"] = json!(timing);
    }
    doc
}

const VERSIONED: Option<u64> = Some(pm_media::clip::ASSET_TIMING);

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
            let doc = load(&document(trim, speed, VERSIONED));
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
        let doc = load(&document(0.0, 1.0, VERSIONED));
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

fn boxes(buf: &[u8], from: usize, to: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut out = vec![];
    let mut at = from;
    while at + 8 <= to {
        let size = u32::from_be_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
        assert!(
            size >= 8 && at + size <= to,
            "a plain 32-bit box is expected"
        );
        out.push((buf[at + 4..at + 8].try_into().unwrap(), at, size));
        at += size;
    }
    out
}

fn wrap(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

/// Rewrites the video track of an MP4 whose `moov` is its last box: `edits`
/// replaces the edit list (`(media time or -1, milliseconds, rate 1 or 0
/// for a dwell)` entries), `offset` gives every sample that composition
/// offset.
fn retime_track(
    src: &std::path::Path,
    dst: &std::path::Path,
    edits: &[(i32, u32, u16)],
    offset: i32,
) {
    fn rebuild(
        buf: &[u8],
        at: usize,
        size: usize,
        video: bool,
        (edits, movie): (&[(i32, u32, u16)], u32),
        offset: i32,
    ) -> Vec<u8> {
        let mut out = vec![];
        for (kind, at, size) in boxes(buf, at + 8, at + size) {
            let raw = &buf[at..at + size];
            match &kind {
                b"trak" => {
                    // The handler: `vide` after MP4's zero field or QuickTime's
                    // `mhlr` component type.
                    let video = raw
                        .windows(8)
                        .any(|w| &w[4..] == b"vide" && (w[..4] == [0; 4] || &w[..4] == b"mhlr"));
                    out.extend(wrap(
                        &kind,
                        &rebuild(buf, at, size, video, (edits, movie), offset),
                    ));
                }
                b"mdia" | b"minf" | b"stbl" if video => {
                    out.extend(wrap(
                        &kind,
                        &rebuild(buf, at, size, video, (edits, movie), offset),
                    ));
                }
                b"edts" | b"ctts" if video => {}
                b"tkhd" if video && !edits.is_empty() => {
                    out.extend_from_slice(raw);
                    let mut elst = vec![0, 0, 0, 0];
                    elst.extend((edits.len() as u32).to_be_bytes());
                    for (media_time, duration, rate) in edits {
                        let duration = (u64::from(*duration) * u64::from(movie) / 1000) as u32;
                        elst.extend(duration.to_be_bytes());
                        elst.extend(media_time.to_be_bytes());
                        elst.extend(rate.to_be_bytes());
                        elst.extend([0, 0]);
                    }
                    out.extend(wrap(b"edts", &wrap(b"elst", &elst)));
                }
                b"stts" if video && offset != 0 => {
                    out.extend_from_slice(raw);
                    let entries = u32::from_be_bytes(raw[12..16].try_into().unwrap()) as usize;
                    let samples: u32 = (0..entries)
                        .map(|i| {
                            u32::from_be_bytes(raw[16 + 8 * i..20 + 8 * i].try_into().unwrap())
                        })
                        .sum();
                    let mut ctts = vec![1, 0, 0, 0, 0, 0, 0, 1];
                    ctts.extend(samples.to_be_bytes());
                    ctts.extend(offset.to_be_bytes());
                    out.extend(wrap(b"ctts", &ctts));
                }
                _ => out.extend_from_slice(raw),
            }
        }
        out
    }
    let buf = std::fs::read(src).unwrap();
    let (kind, at, size) = *boxes(&buf, 0, buf.len()).last().unwrap();
    assert_eq!(&kind, b"moov", "the fixture's moov must be its last box");
    let mut out = buf[..at].to_vec();
    // The movie timescale (version 0 `mvhd`).
    let mvhd = boxes(&buf, at + 8, at + size)
        .into_iter()
        .find(|b| &b.0 == b"mvhd")
        .expect("mvhd")
        .1;
    let movie = u32::from_be_bytes(buf[mvhd + 20..mvhd + 24].try_into().unwrap());
    out.extend(wrap(
        b"moov",
        &rebuild(&buf, at, size, false, (edits, movie), offset),
    ));
    std::fs::write(dst, out).unwrap();
}

/// An older project (no `timing` on the asset) keeps the frame each of its
/// edits selected in the earlier app, for every way a file starts its
/// picture away from timestamp zero.
#[test]
fn an_older_project_shows_the_frames_its_edits_selected() {
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-legacy-timing-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let h264: &[&str] = &["-c:v", "h264_videotoolbox", "-allow_sw", "1", "-b:v", "8M"];
    let plain: &[&str] = &["-c:v", "mpeg4", "-q:v", "2", "-g", "6", "-bf", "0"];
    let reordered: &[&str] = &["-c:v", "mpeg4", "-q:v", "2", "-g", "6", "-bf", "2"];
    let vp9: &[&str] = &["-c:v", "libvpx-vp9", "-crf", "12", "-b:v", "0", "-g", "6"];
    let frame = 1.0 / f64::from(FPS);
    // 12800 units a second, 512 a frame.
    let base = dir.0.join("zero.mp4");
    steps_clip(&base, plain, &[]);
    steps_clip(&dir.0.join("camera.ts"), h264, &[]);
    steps_clip(
        &dir.0.join("fragmented.mp4"),
        reordered,
        &["-movflags", "frag_keyframe+empty_moov+default_base_moof"],
    );
    // A converted camera file: the picture one frame after the file's start.
    steps_clip(
        &dir.0.join("converted.webm"),
        vp9,
        &["-output_ts_offset", "0.04"],
    );
    retime_track(
        &base,
        &dir.0.join("empty-edit.mp4"),
        &[(-1, 400, 1), (0, 800, 1)],
        0,
    );
    retime_track(&base, &dir.0.join("negative.mp4"), &[], -256);
    // The edit starts half way through frame 2.
    retime_track(&base, &dir.0.join("straddle.mp4"), &[(1280, 700, 1)], 0);
    retime_track(
        &base,
        &dir.0.join("empty-then-straddle.mp4"),
        &[(-1, 400, 1), (1280, 700, 1)],
        0,
    );
    // (file, the first frame shown, the source time at which it starts:
    // `None` reads it from the container's first timestamp)
    let cases: [(&str, usize, Option<f64>); 8] = [
        ("zero.mp4", 0, Some(0.0)),
        ("camera.ts", 0, None),
        ("fragmented.mp4", 0, None),
        ("converted.webm", 0, Some(0.04)),
        ("empty-edit.mp4", 0, Some(0.4)),
        ("negative.mp4", 0, Some(-0.02)),
        ("straddle.mp4", 3, Some(0.0)),
        ("empty-then-straddle.mp4", 3, Some(0.4)),
    ];
    let mut preview = Renderer::headless().unwrap();
    let mut export = Renderer::headless().unwrap();
    for (name, head, start) in cases {
        let path = dir.0.join(name);
        let source = pm_media::decode::service::MediaSource::open(&path)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let start = start.unwrap_or_else(|| {
            let tb = source.index.timebase;
            let start = source.index.frames[0].pts as f64 * tb.num() as f64 / tb.den() as f64;
            assert!(start > 0.0, "{name}: the fixture must start late ({start})");
            start
        });
        // Frame k is on screen from `start + (k - head) / fps`; before the
        // first one the earlier app showed that first frame.
        let legacy = |u: f64| {
            let k = ((u - start) / frame + 1e-4).floor().max(0.0) as usize + head;
            Some(k.min(FRAMES - 1))
        };
        let remap = json!({ "v": 0.3, "expr": null, "kf": [
            { "t": 0.0, "v": 0.3, "i": "a", "eo": [0.333, 0.0], "ei": [0.667, 1.0], "hold": true },
            { "t": 0.42, "v": 0.62, "i": "b", "eo": [0.333, 0.0], "ei": [0.667, 1.0], "hold": true },
        ]});
        let edits: [(&str, f64, f64, bool); 4] = [
            ("whole clip", 0.0, 1.0, false),
            ("trimmed", 5.0 / 25.0 + 0.01, 1.0, false),
            ("double speed", 2.0 / 25.0 + 0.01, 2.0, false),
            ("remap keys", 0.0, 1.0, true),
        ];
        for (what, trim, speed, remapped) in edits {
            let mut value = document(trim, speed, None);
            if remapped {
                value["layers"][0]["d"]["timeRemap"] = json!(true);
                value["layers"][0]["d"]["sourceTime"] = remap.clone();
            }
            let expected: Vec<Option<usize>> = (0..FRAMES + 2)
                .map(|n| {
                    let t = n as f64 / 25.0;
                    legacy(match remapped {
                        true if t < 0.42 => 0.3,
                        true => 0.62,
                        false => trim + t * speed,
                    })
                })
                .collect();
            let doc = load(&value);
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

/// With `timing: 2` a late picture is blank until its first frame, as
/// authored, when the sound starts more than a frame before it; a picture
/// within a frame of the sound opens the clip.
#[test]
fn a_clip_starts_where_its_presentation_starts() {
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-presentation-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let mut preview = Renderer::headless().unwrap();
    let mut export = Renderer::headless().unwrap();
    // Silence and picture both from zero; an empty edit then delays the
    // picture exactly.
    let with_sound: &[&str] = &[
        "-f",
        "lavfi",
        "-t",
        "1.2",
        "-i",
        "anullsrc=r=48000:cl=mono",
        "-c:v",
        "mpeg4",
        "-q:v",
        "2",
        "-g",
        "6",
        "-bf",
        "0",
        "-c:a",
        "aac",
    ];
    let base = dir.0.join("together.mp4");
    steps_clip(&base, with_sound, &[]);
    // (file, milliseconds the picture starts after the sound, blank frames)
    for (name, late, blank) in [
        ("five-frames-late.mp4", 200u32, 5usize),
        ("twelve-ms-late.mp4", 12, 0),
        ("one-frame-late.mp4", 40, 0),
    ] {
        let path = dir.0.join(name);
        retime_track(&base, &path, &[(-1, late, 1), (0, 800, 1)], 0);
        let source = pm_media::decode::service::MediaSource::open(&path).unwrap();
        let tb = source.index.timebase;
        let start = source.index.frames[0].pts as f64 * tb.num() as f64 / tb.den() as f64;
        assert!(
            (start - f64::from(late) / 1e3).abs() < 1e-3,
            "{name}: the picture starts at {start}"
        );
        let expected: Vec<Option<usize>> = (0..FRAMES + 2)
            .map(|n| n.checked_sub(blank).map(|k| k.min(FRAMES - 1)))
            .collect();
        let doc = load(&document(0.0, 1.0, VERSIONED));
        for (renderer, kind) in [
            (&mut preview, OutputKind::Preview),
            (&mut export, OutputKind::Final),
        ] {
            assert_eq!(
                shown(renderer, &doc, kind, &path),
                expected,
                "{name}, {kind:?}"
            );
        }
    }
}

/// A clip is as long as its presentation: a dwell lasts its segment, an
/// edit ending part way through a sample ends there, and a straddled head
/// starts at the edit. The imported length, the source's length and the
/// last frame a layer of that length reaches agree.
#[test]
fn a_clip_is_as_long_as_its_presentation() {
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-clip-length-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let plain: &[&str] = &["-c:v", "mpeg4", "-q:v", "2", "-g", "6", "-bf", "0"];
    let base = dir.0.join("zero.mp4");
    steps_clip(&base, plain, &[]);
    // Frames 0..9, then frame 10 held for two seconds.
    retime_track(
        &base,
        &dir.0.join("dwell.mp4"),
        &[(0, 400, 1), (5120, 2000, 0)],
        0,
    );
    // The edit ends half way through frame 12.
    retime_track(&base, &dir.0.join("clipped.mp4"), &[(0, 500, 1)], 0);
    // The edit starts half way through frame 2, which shows for 20 ms.
    retime_track(&base, &dir.0.join("straddle.mp4"), &[(1280, 700, 1)], 0);
    let mut renderer = Renderer::headless().unwrap();
    // (file, seconds, frames shown at composition frames 0, 1 and the last)
    for (name, seconds, first, second, last) in [
        ("zero.mp4", 0.8, 0usize, 1usize, 19usize),
        ("dwell.mp4", 2.4, 0, 1, 10),
        ("clipped.mp4", 0.5, 0, 1, 12),
        ("straddle.mp4", 0.7, 2, 3, 19),
    ] {
        let path = dir.0.join(name);
        let facts = pm_media::facts::media_facts(&path, None).unwrap();
        let length = facts.duration.unwrap();
        assert!(
            (length - seconds).abs() < 1e-6,
            "{name}: imported as {length} s"
        );
        let source = pm_media::decode::service::MediaSource::open(&path).unwrap();
        let ticks = source.duration() as f64 / pm_time::TICKS_PER_SECOND as f64;
        assert!(
            (ticks - seconds).abs() < 1e-6,
            "{name}: the source says {ticks} s"
        );
        // A layer exactly as long as the clip, in a composition that long.
        let frames = (seconds * f64::from(FPS)).round() as usize;
        let mut value = document(0.0, 1.0, VERSIONED);
        value["layers"][0]["dur"] = json!(seconds);
        value["dur"] = json!(seconds);
        let doc = load(&value);
        let mut resolver = MapResolver::default();
        resolver.paths.insert("clip".into(), path.clone());
        let request = RenderRequest {
            output_kind: OutputKind::Final,
            export: true,
            ..RenderRequest::at_scale(Size::new(SIDE, SIDE), 1.0, Precision::Float16)
        };
        let mut at = |n: usize| {
            let frame = renderer
                .render(&doc, "Ptest-root", ticks_of_frame(n), &request, &resolver)
                .expect("render");
            let rgba = renderer.read_rgba(&frame).unwrap();
            renderer.recycle(frame);
            let center = ((SIDE / 2) * SIDE + SIDE / 2) as usize * 4;
            (rgba[center + 3] >= 0.5)
                .then(|| ((f64::from(rgba[center]) * 255.0 - 20.0) / 10.0).round() as usize)
        };
        assert_eq!(
            [at(0), at(1), at(frames - 1)],
            [Some(first), Some(second), Some(last)],
            "{name}: frames 0, 1 and {}",
            frames - 1
        );
    }
}

fn ticks_of_frame(n: usize) -> pm_protocol::common::Ticks {
    ticks(n as f64 / f64::from(FPS))
}

/// The frame a composition at time `t` shows from the clip (`None`:
/// nothing drawn), read from the center pixel's level.
fn timeline_frame(
    renderer: &mut Renderer,
    document: &pm_model::Document,
    path: &std::path::Path,
    t: f64,
) -> Option<usize> {
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.to_path_buf());
    let request = RenderRequest {
        output_kind: OutputKind::Preview,
        ..RenderRequest::at_scale(Size::new(SIDE, SIDE), 1.0, Precision::Float16)
    };
    let frame = renderer
        .render(document, "Ptest-root", ticks(t), &request, &resolver)
        .expect("render");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let rgba = renderer.read_rgba(&frame).unwrap();
    renderer.recycle(frame);
    let center = ((SIDE / 2) * SIDE + SIDE / 2) as usize * 4;
    (rgba[center + 3] >= 0.5).then(|| nearest_frame(f64::from(rgba[center]) * 255.0))
}

/// The source frame whose grey level `code` (0..255) is.
fn nearest_frame(code: f64) -> usize {
    let (i, error) = (0..FRAMES)
        .map(|i| (i, (code - level(i)).abs()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    assert!(error < 6.0, "code {code:.1}");
    i
}

/// The source monitor, contact sheets and posters show, for a source time,
/// the frame the asset's own layers show at it: an older asset reads its
/// file as the earlier app did (the first frame held before its time), a
/// new one by its presentation, on the monitor's coded planes and on the
/// preview reader alike.
#[test]
fn source_views_show_the_frame_the_timeline_shows() {
    let dir =
        Clean(std::env::temp_dir().join(format!("pm-render-source-views-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let plain: &[&str] = &["-c:v", "mpeg4", "-q:v", "2", "-g", "6", "-bf", "0"];
    let base = dir.0.join("zero.mp4");
    steps_clip(&base, plain, &[]);
    // A picture that starts 0.4 s into the file, with no sound.
    retime_track(
        &base,
        &dir.0.join("empty-edit.mp4"),
        &[(-1, 400, 1), (0, 800, 1)],
        0,
    );
    // Sound from zero, the picture 0.2 s later.
    let with_sound: Vec<&str> = [
        "-f",
        "lavfi",
        "-t",
        "1.2",
        "-i",
        "anullsrc=r=48000:cl=mono",
        "-c:a",
        "aac",
    ]
    .into_iter()
    .chain(plain.iter().copied())
    .collect();
    let together = dir.0.join("together.mp4");
    steps_clip(&together, &with_sound, &[]);
    retime_track(
        &together,
        &dir.0.join("late-picture.mp4"),
        &[(-1, 200, 1), (0, 800, 1)],
        0,
    );
    let mut timeline = Renderer::headless().unwrap();
    let mut monitor = Renderer::headless().unwrap();
    for name in ["empty-edit.mp4", "late-picture.mp4"] {
        let path = dir.0.join(name);
        for timing in [None, VERSIONED] {
            let clip = pm_media::clip::ClipTiming::of_asset(timing.map(|t| t as f64));
            let doc = load(&document(0.0, 1.0, timing));
            let mut reader = pm_media::preview::PreviewReader::open(&path)
                .unwrap()
                .with_timing(clip);
            for t in [0.0, 0.1, 0.2, 0.3, 0.44, 0.6, 0.76, 1.1] {
                let shown = timeline_frame(&mut timeline, &doc, &path, t);
                let planes = monitor
                    .source_video_planes(&path, ticks(t), clip, false)
                    .map(|planes| {
                        let (size, bytes, _) = planes.expect("presentable planes");
                        let luma = f64::from(
                            bytes[(size.height / 2 * size.width + size.width / 2) as usize],
                        );
                        nearest_frame((luma - 16.0) * 255.0 / 219.0)
                    })
                    .ok();
                let previewed = reader.frame_at(t).ok().map(|frame| {
                    let decoded = frame.decoded();
                    let (width, height) = (decoded.format.width, decoded.format.height);
                    assert_eq!(decoded.format.bits, 8);
                    // SAFETY: the luma plane holds `height` rows of `stride` bytes.
                    let luma = decoded
                        .with_planes(|planes| unsafe {
                            *planes[0].ptr.add(height / 2 * planes[0].stride + width / 2)
                        })
                        .unwrap();
                    nearest_frame((f64::from(luma) - 16.0) * 255.0 / 219.0)
                });
                assert_eq!(
                    (planes, previewed),
                    (shown, shown),
                    "{name}, timing {timing:?}, source time {t}"
                );
            }
        }
    }
}
