//! Video into a pixel effect, a blend mode and masks. The video layer's
//! frame (the one reading, proven by tests/video_reading.rs) is stored as a
//! 16-bit still; the same effect, screen blend and masks over the video and
//! over that still must draw the same pixels. Stills through these stages
//! are covered by the pixel corpus, so video reaches effects, blending and
//! masking exactly as any other layer does.

#[path = "support/doc.rs"]
mod doc;

use doc::ch;
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_render::{MapResolver, OutputKind, RenderRequest, Renderer};
use serde_json::{Value, json};
use std::path::PathBuf;

const W: u32 = 256;
const H: u32 = 144;

fn clip() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video/h264-709-limited.mp4")
}

fn render(renderer: &mut Renderer, doc: &Value, assets: &MapResolver) -> Vec<f32> {
    let request = RenderRequest {
        export: true,
        output_kind: OutputKind::Final,
        ..RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32)
    };
    let tick = Ticks::new(pm_time::TICKS_PER_SECOND * 5 / 8).unwrap();
    let frame = renderer
        .render(&doc::load(doc), "Ptest-root", tick, &request, assets)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

/// `kind` ("video" or "image") with a colour effect, screen blend and two
/// masks, over a solid and a gradient.
fn stack(kind: &str, format: &str) -> Value {
    let mut source = doc::layer(
        "s",
        kind,
        json!({"asset":"clip","fit":"stretch","w":W,"h":H,"trim":0,"speed":1}),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        2.0,
    );
    source["compat"] = json!(2);
    source["blend"] = json!("screen");
    source["fx"] = json!([{ "id": "e", "type": "color", "on": true,
        "p": { "exposure": ch(json!(0.4)), "contrast": ch(json!(20)), "saturation": ch(json!(-30)) } }]);
    source["masks"] = json!([
        { "id": "m1", "shape": "ellipse", "mode": "add", "on": true,
          "p": { "x": ch(json!(0)), "y": ch(json!(0)), "w": ch(json!(220)), "h": ch(json!(120)),
                 "rotation": ch(json!(10)), "feather": ch(json!(10)) } },
        { "id": "m2", "shape": "rect", "mode": "subtract", "on": true,
          "p": { "x": ch(json!(40)), "y": ch(json!(10)), "w": ch(json!(50)), "h": ch(json!(30)),
                 "rotation": ch(json!(0)), "feather": ch(json!(0)) } }
    ]);
    let under = doc::layer(
        "u",
        "solid",
        json!({ "color": "#3050A0", "w": 140, "h": 90 }),
        90.0,
        60.0,
        2.0,
    );
    let mut p = doc::project(
        W,
        H,
        2.0,
        json!({"type":"linear","angle":30,"stops":[
            {"id":"a","color":"#182132","position":0},{"id":"b","color":"#C06A40","position":100}]}),
        vec![under, source],
    );
    p["colorDepth"] = json!(32);
    doc::asset(&mut p, "clip", kind, format, W, H, 2.0);
    p
}

#[test]
fn video_reaches_effects_blends_and_masks_as_a_still_does() {
    let mut renderer = Renderer::headless().unwrap();
    assert!(
        renderer
            .set_definitions(&pm_render::builtin_definitions())
            .unwrap()
            .is_empty()
    );
    // The video layer's own frame, alone.
    let mut alone = doc::project(
        W,
        H,
        2.0,
        doc::no_fill(),
        vec![doc::layer(
            "v",
            "video",
            json!({"asset":"clip","fit":"stretch","w":W,"h":H,"trim":0,"speed":1}),
            f64::from(W) / 2.0,
            f64::from(H) / 2.0,
            2.0,
        )],
    );
    alone["colorDepth"] = json!(32);
    doc::asset(&mut alone, "clip", "video", "mp4", W, H, 2.0);
    let mut video_assets = MapResolver::default();
    video_assets.paths.insert("clip".into(), clip());
    let frame = render(&mut renderer, &alone, &video_assets);

    // As a 16-bit still (straight, opaque, untagged: the sRGB encoding).
    let dir = std::env::temp_dir().join(format!("pm-render-video-fx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let still = dir.join("frame.png");
    let mut info = png::Info::with_size(W, H);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Sixteen;
    let mut writer = png::Encoder::with_info(
        std::io::BufWriter::new(std::fs::File::create(&still).unwrap()),
        info,
    )
    .unwrap()
    .write_header()
    .unwrap();
    let bytes: Vec<u8> = frame
        .iter()
        .flat_map(|v| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes())
        .collect();
    writer.write_image_data(&bytes).unwrap();
    writer.finish().unwrap();
    // Pixels a 0..1 still can carry exactly.
    let in_range: Vec<bool> = frame
        .chunks_exact(4)
        .map(|p| p[..3].iter().all(|v| (0.0..=1.0).contains(v)))
        .collect();

    let over_video = render(&mut renderer, &stack("video", "mp4"), &video_assets);
    let mut still_assets = MapResolver::default();
    still_assets.paths.insert("clip".into(), still);
    let over_still = render(&mut renderer, &stack("image", "png"), &still_assets);

    let (mut worst, mut compared) = (0.0f32, 0);
    for (i, ok) in in_range.iter().enumerate() {
        if !ok {
            continue;
        }
        compared += 1;
        let (a, b) = (&over_video[i * 4..i * 4 + 4], &over_still[i * 4..i * 4 + 4]);
        for c in 0..4 {
            worst = worst.max((a[c] - b[c]).abs());
        }
    }
    assert!(compared > (W * H) as usize / 2, "{compared}");
    // A 16-bit store (half a step: 7.6e-6) through the effect's gain.
    assert!(
        worst < 1e-4,
        "video vs still through effect, blend, masks: {worst}"
    );
    // The stack changed the picture: the effect, blend and masks all act.
    let changed = over_video
        .chunks_exact(4)
        .zip(frame.chunks_exact(4))
        .filter(|(a, b)| (a[0] - b[0]).abs() > 0.05)
        .count();
    assert!(changed > (W * H) as usize / 4, "{changed}");
}
