//! Section 25's video class against its FFmpeg reference, measured for the
//! Phase 3a gate (docs/engine/phase3a-gate.md): "the engine within 1 CV of an
//! FFmpeg plus zimg reference using the same chroma filter". It decides the
//! corpus videos the legacy capture cannot (`media/converted-mkv`,
//! `media/prores422-mov`, `media/prores4444-alpha-mov` show a playback
//! proxy there) and the opaque fixtures beside them.
//!
//! Each e2e fixture video is drawn alone by the renderer at its native size
//! (one layer, `contain`, centered, so every output pixel is one source pixel)
//! at 32-bit, and compared with frame n decoded by FFmpeg and converted by
//! zimg to 16-bit RGB with the matrix and range the engine reads from the file
//! (`Renderer::video_color`) and its chroma siting. zimg applies one filter to
//! both axes; the engine upsamples 4:2:2 linearly (zimg `bilinear`, the same
//! filter) and 4:2:0 with Catmull-Rom across and linear between rows
//! (pm-color `chroma.rs`), so 4:2:0 sources are compared against zimg's
//! Catmull-Rom (`bicubic`, b = 0, c = 0.5) on both axes and their vertical
//! chroma edges carry that difference: their row is reported, the class is
//! judged on 4:4:4 and 4:2:2 sources only. Alpha sources compare color where
//! the reference is opaque and alpha everywhere.
//!
//! A measurement of the gate, not a regression test:
//! `cargo test --release -p pm-cli --test phase3a_video_class -- --ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::process::Command;

use pm_protocol::color::{ChromaSiting, MatrixCoefficients, Precision, Range, Subsampling};
use pm_protocol::common::{Size, Ticks};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn ffmpeg() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(|d| PathBuf::from(d).join("bin/ffmpeg"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn document(w: u32, h: u32, fps: f64, dur: f64) -> pm_model::Document {
    let p = json!({
        "perspective": ch(json!(50)), "position.z": ch(json!(0)), "anchor.z": ch(json!(0)),
        "scale.z": ch(json!(100)), "rotation.x": ch(json!(0)), "rotation.y": ch(json!(0)),
        "orientation.x": ch(json!(0)), "orientation.y": ch(json!(0)), "orientation.z": ch(json!(0)),
        "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)),
        "position.x": ch(json!(f64::from(w) / 2.0)), "position.y": ch(json!(f64::from(h) / 2.0)),
        "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
        "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
    });
    let layer = json!({
        "id": "v", "type": "video", "name": "v", "from": 0, "dur": dur, "on": true,
        "lock": false, "shy": false, "collapsed": true, "threeD": false, "solo": false,
        "color": "#3B62E8", "blend": "normal", "mblur": false, "parent": null, "group": null,
        "matteSource": null, "p": p, "fx": [], "transitionIn": null, "transitionOut": null,
        "masks": [], "d": { "asset": "clip", "fit": "contain", "trim": 0, "speed": 1,
        "embeddedAudio": false, "w": w, "h": h }, "locked_intent": {}
    });
    let doc = json!({
        "id": "Pvideo", "name": "video", "w": w, "h": h, "fps": fps, "dur": dur, "bg": "#000000",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "Pvideo-root", "compName": "video", "markers": [], "work": [0, dur], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer], "comps": {},
        "assets": { "clip": { "id": "clip", "name": "clip", "kind": "video", "w": w, "h": h,
            "dur": dur, "fps": fps, "persisted": true } }
    });
    pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
}

/// Frame `n` decoded by FFmpeg and converted by zimg to 16-bit RGB(A),
/// interleaved, with the engine's reading of the file.
fn reference(
    path: &Path,
    n: u32,
    matrix: &str,
    range: &str,
    chromal: &str,
    filter: &str,
    alpha: bool,
) -> Vec<u16> {
    let vf = format!(
        "select=eq(n\\,{n}),zscale=matrixin={matrix}:rangein={range}:range=full:chromal={chromal}:filter={filter}:dither=none,format={}",
        if alpha { "gbrap16le" } else { "gbrp16le" }
    );
    let out = Command::new(ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vf", &vf, "-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .expect("ffmpeg runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let planar: Vec<u16> = out
        .stdout
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    let planes = if alpha { 4 } else { 3 };
    let px = planar.len() / planes;
    // Planes G, B, R (, A).
    (0..px)
        .flat_map(|i| {
            [
                planar[2 * px + i],
                planar[i],
                planar[px + i],
                if alpha { planar[3 * px + i] } else { 65535 },
            ]
        })
        .collect()
}

struct Row {
    name: String,
    kind: &'static str,
    frames: u32,
    color_px: usize,
    max_cv: f64,
    over_one: usize,
    alpha_max_cv: f64,
}

#[test]
#[ignore = "gate measurement (docs/engine/phase3a-gate.md); run with --ignored"]
fn video_class_against_ffmpeg_and_zimg() {
    let fixtures = repo().join("apps/desktop/e2e/fixtures");
    let mut r = Renderer::headless().expect("a Metal device");
    let mut rows = Vec::new();
    for name in [
        "prores422.mov",
        "prores4444-alpha.mov",
        "converted.mkv",
        "h264-aac.mov",
        "h264-aac.mp4",
        "hevc.mov",
        "hevc.mp4",
        "vp8-opus.webm",
        "vp9-opus.webm",
        "av1.mp4",
    ] {
        let path = fixtures.join(name);
        let color = match r.video_color(&path) {
            Ok(c) => c,
            Err(e) => {
                println!("{name}: the engine does not read it ({e})");
                continue;
            }
        };
        // The fixtures are 160 × 90 at 30 fps.
        let (w, h, fps) = (160u32, 90u32, 30.0);
        let matrix = match color.matrix {
            MatrixCoefficients::Bt709 => "709",
            MatrixCoefficients::Bt601 => "170m",
            MatrixCoefficients::Bt2020Ncl => "2020_ncl",
            MatrixCoefficients::Identity => "input",
        };
        let range = if color.range == Range::Full {
            "full"
        } else {
            "limited"
        };
        let chromal = match color.siting {
            ChromaSiting::Left => "left",
            ChromaSiting::Center => "center",
            ChromaSiting::TopLeft => "topleft",
            ChromaSiting::Top => "top",
            ChromaSiting::BottomLeft => "bottomleft",
            ChromaSiting::Bottom => "bottom",
        };
        let (kind, filter) = match color.subsampling {
            Subsampling::S444 => ("4:4:4", "bilinear"),
            Subsampling::S422 => ("4:2:2", "bilinear"),
            _ => ("4:2:0", "bicubic:param_a=0:param_b=0.5"),
        };
        let alpha = name.contains("alpha");
        let doc = document(w, h, fps, 1.0);
        let mut resolver = MapResolver::default();
        resolver.paths.insert("clip".into(), path.clone());
        let mut row = Row {
            name: name.into(),
            kind,
            frames: 0,
            color_px: 0,
            max_cv: 0.0,
            over_one: 0,
            alpha_max_cv: 0.0,
        };
        for n in [0u32, 7, 15, 23] {
            let t = (f64::from(n) + 0.5) / fps;
            let request = RenderRequest::at_scale(Size::new(w, h), 1.0, Precision::Float32);
            let ticks = Ticks::new((t * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap();
            let frame = match r.render(&doc, "Pvideo-root", ticks, &request, &resolver) {
                Ok(f) if f.failures.is_empty() => f,
                Ok(f) => panic!("{name}: {:?}", f.failures),
                Err(e) => {
                    println!("{name}: not rendered ({e})");
                    break;
                }
            };
            let ours = r.read_rgba(&frame).expect("readback");
            let reference = reference(&path, n, matrix, range, chromal, filter, alpha);
            assert_eq!(reference.len(), ours.len(), "{name}: frame size");
            row.frames += 1;
            for (o, q) in ours.chunks_exact(4).zip(reference.chunks_exact(4)) {
                let ra = f64::from(q[3]) / 65535.0;
                let oa = f64::from(o[3]);
                row.alpha_max_cv = row.alpha_max_cv.max((oa - ra).abs() * 255.0);
                if ra < 1.0 {
                    continue;
                }
                row.color_px += 1;
                let mut worst: f64 = 0.0;
                for c in 0..3 {
                    // The engine's output is premultiplied; opaque here.
                    let d = (f64::from(o[c]) - f64::from(q[c]) / 65535.0).abs() * 255.0;
                    worst = worst.max(d);
                }
                row.max_cv = row.max_cv.max(worst);
                if worst > 1.0 {
                    row.over_one += 1;
                }
            }
        }
        println!(
            "{:<22} {} {:?} {:?} {:?}: {} frames, {} opaque px, max {:.3} CV, {} px over 1 CV{}",
            row.name,
            row.kind,
            color.matrix,
            color.range,
            color.siting,
            row.frames,
            row.color_px,
            row.max_cv,
            row.over_one,
            if alpha {
                format!(", alpha max {:.3} CV", row.alpha_max_cv)
            } else {
                String::new()
            }
        );
        rows.push(row);
    }
    let judged: Vec<&Row> = rows.iter().filter(|r| r.kind != "4:2:0").collect();
    let worst = judged
        .iter()
        .map(|r| r.max_cv.max(r.alpha_max_cv))
        .fold(0.0, f64::max);
    println!("4:4:4 and 4:2:2 sources: worst {worst:.3} CV (bar 1)");
    assert!(
        !judged.is_empty() && worst <= 1.0,
        "the video class against FFmpeg plus zimg: {worst:.3} CV"
    );
}
