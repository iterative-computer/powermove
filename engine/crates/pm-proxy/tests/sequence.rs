use std::fs;
use std::path::Path;

use pm_proxy::{SequenceFrame, SequenceManifest, SequenceRate};

#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../../../spikes/s2-precision/src/color.rs"]
mod color;

#[test]
fn aggregate_identity_uses_rate_and_ordered_bytes_not_paths() {
    let hash_a = "a".repeat(64);
    let hash_b = "b".repeat(64);
    let one = SequenceManifest {
        powermove_sequence: 1,
        rate: SequenceRate {
            num: 24000,
            den: 1001,
        },
        frames: vec![
            SequenceFrame {
                path: "/first/a.png".into(),
                hash: hash_a.clone(),
            },
            SequenceFrame {
                path: "/first/b.png".into(),
                hash: hash_b.clone(),
            },
        ],
    };
    let mut relocated = one.clone();
    relocated.frames[0].path = "/second/a.png".into();
    relocated.frames[0].hash = hash_a.to_ascii_uppercase();
    assert_eq!(
        one.content_hash().unwrap(),
        relocated.content_hash().unwrap()
    );
    relocated.frames.reverse();
    assert_ne!(
        one.content_hash().unwrap(),
        relocated.content_hash().unwrap()
    );
    relocated = one.clone();
    relocated.rate.den = 1000;
    assert_ne!(
        one.content_hash().unwrap(),
        relocated.content_hash().unwrap()
    );
    relocated = one.clone();
    relocated.frames[0].path = "relative.png".into();
    assert!(relocated.content_hash().is_err());
}

#[cfg(target_os = "macos")]
mod real {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::AtomicBool;

    use pm_protocol::priority::{ProxyFormat, ProxySize};
    use pm_proxy::{
        GenerationControl, GenerationRequest, ProxyError, ProxyStore, generate_with_disk_probe,
    };

    const PATCHES: [[u8; 3]; 40] = [
        [115, 82, 68],
        [194, 150, 130],
        [98, 122, 157],
        [87, 108, 67],
        [133, 128, 177],
        [103, 189, 170],
        [214, 126, 44],
        [80, 91, 166],
        [193, 90, 99],
        [94, 60, 108],
        [157, 188, 64],
        [224, 163, 46],
        [56, 61, 150],
        [70, 148, 73],
        [175, 54, 60],
        [231, 199, 31],
        [187, 86, 149],
        [8, 133, 161],
        [243, 243, 242],
        [200, 200, 200],
        [160, 160, 160],
        [122, 122, 121],
        [85, 85, 85],
        [52, 52, 52],
        [0, 0, 0],
        [3, 3, 3],
        [8, 8, 8],
        [16, 16, 16],
        [24, 24, 24],
        [32, 32, 32],
        [64, 64, 64],
        [128, 128, 128],
        [192, 192, 192],
        [255, 255, 255],
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [0, 255, 255],
        [255, 0, 255],
        [255, 255, 0],
    ];
    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 200;

    fn chart(path: &Path) -> Vec<[u16; 3]> {
        let mut values = Vec::new();
        for (i, patch) in PATCHES.iter().enumerate() {
            let offset = (i as i32 % 11) - 5;
            values.push(patch.map(|code| (i32::from(code) * 257 + offset).clamp(0, 65535) as u16));
        }
        let mut bytes = Vec::with_capacity((WIDTH * HEIGHT * 8) as usize);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let i = (y / 40 * 8 + x / 40) as usize;
                for code in values[i] {
                    bytes.extend_from_slice(&code.to_be_bytes());
                }
                bytes.extend_from_slice(&u16::MAX.to_be_bytes());
            }
        }
        let file = fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&bytes).unwrap();
        values
    }

    fn lab(rgb: [f64; 3]) -> [f64; 3] {
        color::xyz_to_lab(color::lin709_to_xyz(rgb.map(color::e_decode)), color::D65)
    }

    #[test]
    fn sixteen_bit_original_frames_make_exact_rate_4444_and_measured_chart() {
        let dir = tempfile::tempdir().unwrap();
        let ffmpeg_dir = std::path::PathBuf::from(
            std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"),
        );
        let ffmpeg = ffmpeg_dir.join("bin/ffmpeg");
        let ffprobe = ffmpeg_dir.join("bin/ffprobe");
        let frames: Vec<_> = (0..3)
            .map(|n| {
                let path = dir.path().join(format!("original-{n}.png"));
                chart(&path);
                let hash = blake3::hash(&fs::read(&path).unwrap()).to_hex().to_string();
                SequenceFrame { path, hash }
            })
            .collect();
        let expected = chart(&frames[0].path);
        let manifest = SequenceManifest {
            powermove_sequence: 1,
            rate: SequenceRate {
                num: 24000,
                den: 1001,
            },
            frames,
        };
        let source_hash = manifest.content_hash().unwrap();
        let source = dir.path().join("original-sequence.json");
        fs::write(&source, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let store = ProxyStore::open(dir.path().join("generated")).unwrap();
        let cancel = AtomicBool::new(false);
        let never_pause = || false;
        let on_pause = |_| {};
        let make_request = || GenerationRequest {
            source: &source,
            source_hash: &source_hash,
            format: ProxyFormat::ProResProxy,
            size: ProxySize::Half,
            ffmpeg: &ffmpeg,
            ffprobe: &ffprobe,
            store: &store,
            control: GenerationControl {
                cancel: &cancel,
                pause: &never_pause,
                on_pause: &on_pause,
            },
        };
        // A deterministic quota keeps this codec test independent of the
        // developer's currently shared build volume. The disk guard itself
        // is exercised by the dedicated pause/resume tests.
        let quota = |_: &Path| Ok((40 << 30, 100 << 30));
        let first = generate_with_disk_probe(make_request(), &quota).unwrap();
        assert!(!first.reused);
        assert_eq!(first.stored.recipe.codec(), "prores_4444");
        assert!(first.stored.recipe.source_has_alpha);
        assert_eq!(first.stored.recipe.source_depth, 16);
        assert_eq!(first.stored.recipe.width, WIDTH / 2);
        assert_eq!(first.stored.recipe.height, HEIGHT / 2);
        assert!(
            generate_with_disk_probe(make_request(), &quota)
                .unwrap()
                .reused
        );

        let decoded = Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&first.stored.path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba64le",
                "-",
            ])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        let width = first.stored.recipe.width as usize;
        let height = first.stored.recipe.height as usize;
        assert_eq!(decoded.stdout.len(), width * height * 8);
        let mut max_delta_e = 0.0_f64;
        for i in 0..40 {
            let x = (i % 8) * (width / 8) + width / 16;
            let y = (i / 8) * (height / 5) + height / 10;
            let offset = (y * width + x) * 8;
            let actual = [0, 2, 4].map(|channel| {
                u16::from_le_bytes(
                    decoded.stdout[offset + channel..offset + channel + 2]
                        .try_into()
                        .unwrap(),
                ) as f64
                    / 65535.0
            });
            let original = expected[i].map(|code| f64::from(code) / 65535.0);
            max_delta_e = max_delta_e.max(color::delta_e2000(lab(original), lab(actual)));
        }
        eprintln!("original 16-bit PNG → ProRes 4444 chart max ΔE2000 {max_delta_e:.4}");
        assert!(max_delta_e < 0.5, "max ΔE2000 {max_delta_e}");

        fs::write(&manifest.frames[0].path, b"changed").unwrap();
        assert!(matches!(
            generate_with_disk_probe(make_request(), &quota),
            Err(ProxyError::InvalidMedia(_))
        ));
    }

    #[test]
    fn sequence_color_tags_follow_p3_and_preserve_exr_highlights_and_other_icc() {
        let dir = tempfile::tempdir().unwrap();
        let ffmpeg_dir = std::path::PathBuf::from(
            std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"),
        );
        let ffmpeg = ffmpeg_dir.join("bin/ffmpeg");
        let ffprobe = ffmpeg_dir.join("bin/ffprobe");
        let source_png = dir.path().join("base.png");
        chart(&source_png);
        let p3 = dir.path().join("p3.png");
        let adobe = dir.path().join("adobe.png");
        for (profile, output) in [
            ("/System/Library/ColorSync/Profiles/Display P3.icc", &p3),
            (
                "/System/Library/ColorSync/Profiles/AdobeRGB1998.icc",
                &adobe,
            ),
        ] {
            let status = Command::new("sips")
                .args(["-m", profile])
                .arg(&source_png)
                .args(["--out"])
                .arg(output)
                .output()
                .unwrap();
            assert!(
                status.status.success(),
                "{}",
                String::from_utf8_lossy(&status.stderr)
            );
        }
        let exr = dir.path().join("highlight.exr");
        let raw = dir.path().join("highlight.gbrpf32le");
        let mut bytes = Vec::new();
        // gbrpf32le's planar order is G, B, R. One channel is four times
        // scene reference white, which an SDR/sRGB conversion would clip.
        for channel in [0.5_f32, 0.1, 4.0] {
            for _ in 0..(WIDTH * HEIGHT) {
                bytes.extend_from_slice(&channel.to_le_bytes());
            }
        }
        fs::write(&raw, bytes).unwrap();
        let status = Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pixel_format",
                "gbrpf32le",
                "-video_size",
                "320x200",
                "-framerate",
                "24",
                "-i",
            ])
            .arg(&raw)
            .args(["-frames:v", "1", "-c:v", "exr"])
            .arg(&exr)
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "{}",
            String::from_utf8_lossy(&status.stderr)
        );

        let quota = |_: &Path| Ok((40 << 30, 100 << 30));
        for (name, frame, expected_primaries, expected_transfer) in [
            ("p3", &p3, "smpte432", "iec61966-2-1"),
            ("adobe", &adobe, "bt2020", "smpte2084"),
            ("exr", &exr, "bt2020", "smpte2084"),
        ] {
            eprintln!("checking {name} source color and proxy tags");
            let manifest = SequenceManifest {
                powermove_sequence: 1,
                rate: SequenceRate {
                    num: 24000,
                    den: 1001,
                },
                frames: vec![SequenceFrame {
                    path: frame.to_path_buf(),
                    hash: blake3::hash(&fs::read(frame).unwrap()).to_hex().to_string(),
                }],
            };
            let hash = manifest.content_hash().unwrap();
            let source = dir.path().join(format!("{name}.json"));
            fs::write(&source, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let store = ProxyStore::open(dir.path().join(format!("store-{name}"))).unwrap();
            let cancel = AtomicBool::new(false);
            let output = generate_with_disk_probe(
                GenerationRequest {
                    source: &source,
                    source_hash: &hash,
                    format: ProxyFormat::ProResProxy,
                    size: ProxySize::Half,
                    ffmpeg: &ffmpeg,
                    ffprobe: &ffprobe,
                    store: &store,
                    control: GenerationControl {
                        cancel: &cancel,
                        pause: &|| false,
                        on_pause: &|_| {},
                    },
                },
                &quota,
            )
            .unwrap();
            let probe = Command::new(&ffprobe)
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=color_primaries,color_transfer,nb_frames,r_frame_rate",
                    "-of",
                    "json",
                ])
                .arg(&output.stored.path)
                .output()
                .unwrap();
            assert!(
                probe.status.success(),
                "{}",
                String::from_utf8_lossy(&probe.stderr)
            );
            let stream: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
            let stream = &stream["streams"][0];
            assert_eq!(
                stream["color_primaries"], expected_primaries,
                "{name}: {stream}"
            );
            assert_eq!(
                stream["color_transfer"], expected_transfer,
                "{name}: {stream}"
            );
            assert_eq!(stream["r_frame_rate"], "24000/1001");
            if name == "exr" {
                let decoded = Command::new(&ffmpeg)
                    .args(["-v", "error", "-i"])
                    .arg(&output.stored.path)
                    .args([
                        "-frames:v",
                        "1",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        "rgba64le",
                        "-",
                    ])
                    .output()
                    .unwrap();
                assert!(
                    decoded.status.success(),
                    "{}",
                    String::from_utf8_lossy(&decoded.stderr)
                );
                let red = f64::from(u16::from_le_bytes(decoded.stdout[0..2].try_into().unwrap()))
                    / 65535.0;
                // PQ(100 nits), the scene 1.0 reference white, is ~0.508.
                // A linear EXR red of 4.0 must survive the codec above it.
                assert!(
                    red > 0.56,
                    "EXR highlight clipped to SDR white: PQ red {red}"
                );
            }
        }
    }
}
