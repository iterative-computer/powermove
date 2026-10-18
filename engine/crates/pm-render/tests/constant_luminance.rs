//! Matrix 10 must survive probing and decode nonlinearly, rather than
//! treating BT.2020 constant luminance as the matrix-9 NCL transform.
mod support;

use pm_protocol::color::{MatrixCoefficients, Precision};
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use std::io::Write;
use std::process::{Command, Stdio};
use support::doc::*;

#[test]
fn matrix_10_probe_and_gpu_decode_match_bt2020_table_4() {
    let dir = std::env::temp_dir().join(format!("pm-cl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("constant-luminance.mkv");
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .expect("source engine/env.sh")
        .join("bin/ffmpeg");
    let patches: [[u16; 3]; 8] = [
        [64, 512, 512],
        [940, 512, 512],
        [502, 512, 512],
        [502, 600, 600],
        [502, 424, 424],
        [502, 600, 424],
        [502, 424, 600],
        [250, 530, 500],
    ];
    let mut bytes = Vec::new();
    for component in [0, 1, 2] {
        for _y in 0..8 {
            for x in 0..32 {
                bytes.extend_from_slice(&(patches[x / 4][component] * 4).to_le_bytes());
            }
        }
    }
    let mut process = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pixel_format",
            "yuv444p12le",
            "-video_size",
            "32x8",
            "-framerate",
            "1",
            "-colorspace",
            "bt2020c",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "bt2020-12",
            "-color_range",
            "tv",
            "-i",
            "pipe:0",
            "-frames:v",
            "2",
            "-c:v",
            "ffv1",
            "-level",
            "3",
            "-colorspace",
            "bt2020c",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "bt2020-12",
            "-color_range",
            "tv",
        ])
        .arg(&path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(&bytes.repeat(2))
        .unwrap();
    assert!(process.wait().unwrap().success());
    let probe = pm_media::probe::probe(&path).unwrap().probe;
    let color = probe.video.unwrap().color;
    assert_eq!(color.tagged.matrix, Some(10));
    assert_eq!(color.interpreted.matrix, MatrixCoefficients::Bt2020Cl);
    assert_eq!(pm_color::cicp::matrix_code(color.interpreted.matrix), 10);
    assert!(pm_color::ycbcr::Matrix::of(color.interpreted.matrix).is_none());
    let mut doc = project(
        32,
        8,
        1.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({"asset":"clip","w":32,"h":8,"fit":"stretch"}),
            16.0,
            4.0,
            1.0,
        )],
    );
    asset(&mut doc, "clip", "video", "mkv", 32, 8, 1.0);
    // Isolate Y'cC'bcC'rc decoding from wide-gamut conversion, which still
    // has its own legacy parity gate on an un-interpreted level-1 asset.
    doc["assets"]["clip"]["interpret"] = json!({"primaries":"bt709"});
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path);
    let mut renderer = Renderer::headless().unwrap();
    let frame = renderer
        .render(
            &load(&doc),
            "Ptest-root",
            support::ticks(0.0),
            &RenderRequest::at_scale(Size::new(32, 8), 1.0, Precision::Float32),
            &resolver,
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    for (i, codes) in patches.into_iter().enumerate() {
        let ycc = [
            (f64::from(codes[0]) - 64.0) / 876.0,
            (f64::from(codes[1]) - 512.0) / 896.0,
            (f64::from(codes[2]) - 512.0) / 896.0,
        ];
        let expected = pm_color::ycbcr::bt2020_cl_to_rgb(ycc);
        let got = &pixels[(3 * 32 + i * 4 + 1) * 4..][..4];
        for c in 0..3 {
            assert!(
                (f64::from(got[c]) - expected[c]).abs() < 3e-5,
                "patch {i} channel {c}: {got:?} != {expected:?}"
            );
        }
        assert_eq!(got[3], 1.0);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
