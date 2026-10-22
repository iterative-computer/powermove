//! Section 14.4 fidelity bar on a 16-bit chart: a deep RGB video original
//! becomes a 12-bit ProRes 4444 proxy whose patches stay within ΔE2000 mean
//! 0.5 / max 1.0 and whose full frame stays within p99 3.0, compared with
//! the original reduced by the generator's own resampler (Lanczos) at the
//! same size.
#![cfg(target_os = "macos")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use pm_protocol::priority::{ProxyFormat, ProxySize};
use pm_proxy::{GenerationControl, GenerationRequest, ProxyStore, generate_with_disk_probe};

#[allow(dead_code)]
#[path = "../../../spikes/s2-precision/src/color.rs"]
mod color;

const WIDTH: usize = 640;
const HEIGHT: usize = 400;
const COLUMNS: usize = 8;
const ROWS: usize = 5;

/// 40 patches: a Macbeth-like set, a grey ramp, primaries and secondaries,
/// each offset by a few 16-bit codes so 8-bit sources could not express them.
const PATCHES: [[u8; 3]; 40] = [
    [115, 82, 68], [194, 150, 130], [98, 122, 157], [87, 108, 67], [133, 128, 177],
    [103, 189, 170], [214, 126, 44], [80, 91, 166], [193, 90, 99], [94, 60, 108],
    [157, 188, 64], [224, 163, 46], [56, 61, 150], [70, 148, 73], [175, 54, 60],
    [231, 199, 31], [187, 86, 149], [8, 133, 161], [243, 243, 242], [200, 200, 200],
    [160, 160, 160], [122, 122, 121], [85, 85, 85], [52, 52, 52], [0, 0, 0],
    [8, 8, 8], [16, 16, 16], [24, 24, 24], [32, 32, 32], [64, 64, 64],
    [128, 128, 128], [192, 192, 192], [255, 255, 255], [255, 0, 0], [0, 255, 0],
    [0, 0, 255], [0, 255, 255], [255, 0, 255], [255, 255, 0], [40, 80, 160],
];

fn chart_rgb48() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(WIDTH * HEIGHT * 6);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let i = (y / (HEIGHT / ROWS)) * COLUMNS + x / (WIDTH / COLUMNS);
            let offset = (i as i32 % 11) - 5;
            for code in PATCHES[i] {
                let value = (i32::from(code) * 257 + offset).clamp(0, 65535) as u16;
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    bytes
}

fn lab(rgb: [f64; 3]) -> [f64; 3] {
    color::xyz_to_lab(color::lin709_to_xyz(rgb.map(color::e_decode)), color::D65)
}

fn rgb48(ffmpeg: &Path, input: &Path, filter: Option<&str>) -> Vec<u8> {
    let mut command = Command::new(ffmpeg);
    command.args(["-v", "error", "-i"]).arg(input).args(["-frames:v", "1"]);
    if let Some(filter) = filter {
        command.args(["-vf", filter]);
    }
    let out = command
        .args(["-f", "rawvideo", "-pix_fmt", "rgb48le", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> [f64; 3] {
    let at = (y * width + x) * 6;
    [0, 2, 4].map(|c| f64::from(u16::from_le_bytes([bytes[at + c], bytes[at + c + 1]])) / 65535.0)
}

#[test]
fn sixteen_bit_chart_proxy_meets_the_delta_e_bar() {
    let codecs = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    let (ffmpeg, ffprobe) = (codecs.join("bin/ffmpeg"), codecs.join("bin/ffprobe"));
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("chart.rgb48");
    fs::write(&raw, chart_rgb48()).unwrap();
    // A lossless 16-bit RGB movie (PNG in MOV), tagged BT.709/sRGB-like
    // display encoding, exactly as a deep RGB master would arrive.
    let source = dir.path().join("chart.mov");
    let built = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb48le", "-video_size"])
        .arg(format!("{WIDTH}x{HEIGHT}"))
        .args(["-framerate", "24000/1001", "-stream_loop", "5", "-i"])
        .arg(&raw)
        .args([
            "-c:v", "png", "-pix_fmt", "rgb48be", "-color_primaries", "bt709",
            "-color_trc", "bt709", "-colorspace", "rgb",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let source_hash = blake3::hash(&fs::read(&source).unwrap()).to_hex().to_string();
    let store = ProxyStore::open(dir.path().join("generated")).unwrap();
    let cancel = AtomicBool::new(false);
    let never = || false;
    let on_pause = |_| {};
    let quota = |_: &Path| Ok((40 << 30, 100 << 30));
    let generated = generate_with_disk_probe(
        GenerationRequest {
            source: &source,
            source_hash: &source_hash,
            format: ProxyFormat::ProResProxy,
            size: ProxySize::Half,
            ffmpeg: &ffmpeg,
            ffprobe: &ffprobe,
            store: &store,
            control: GenerationControl { cancel: &cancel, pause: &never, on_pause: &on_pause },
        },
        &quota,
    )
    .unwrap();
    let recipe = &generated.stored.recipe;
    assert_eq!(recipe.codec(), "prores_4444");
    assert_eq!(recipe.source_depth, 16);
    let (w, h) = (recipe.width as usize, recipe.height as usize);
    assert_eq!((w, h), (WIDTH / 2, HEIGHT / 2));

    let proxy = rgb48(&ffmpeg, &generated.stored.path, None);
    let reference = rgb48(&ffmpeg, &source, Some(&format!("scale={w}:{h}:flags=lanczos")));
    assert_eq!(proxy.len(), w * h * 6);
    assert_eq!(reference.len(), proxy.len());

    // Patch interiors (the middle half of each patch, away from the
    // resampler's edge support), against the patch's own 16-bit value.
    let (pw, ph) = (w / COLUMNS, h / ROWS);
    let mut patch_means = Vec::new();
    let mut patch_max = 0.0_f64;
    for (i, patch) in PATCHES.iter().enumerate() {
        let offset = (i as i32 % 11) - 5;
        let original = patch.map(|code| f64::from((i32::from(code) * 257 + offset).clamp(0, 65535)) / 65535.0);
        let (x0, y0) = ((i % COLUMNS) * pw, (i / COLUMNS) * ph);
        let (mut sum, mut n) = (0.0, 0.0);
        for y in y0 + ph / 4..y0 + ph * 3 / 4 {
            for x in x0 + pw / 4..x0 + pw * 3 / 4 {
                let delta = color::delta_e2000(lab(original), lab(pixel(&proxy, w, x, y)));
                patch_max = patch_max.max(delta);
                sum += delta;
                n += 1.0;
            }
        }
        patch_means.push(sum / n);
    }
    let patch_mean = patch_means.iter().sum::<f64>() / patch_means.len() as f64;
    // Every pixel, including patch edges, at the matched scale/resampler.
    let mut full: Vec<f64> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| color::delta_e2000(lab(pixel(&reference, w, x, y)), lab(pixel(&proxy, w, x, y))))
        .collect();
    full.sort_by(f64::total_cmp);
    let p99 = full[(full.len() * 99) / 100];
    let full_max = *full.last().unwrap();
    eprintln!(
        "16-bit chart → ProRes 4444 proxy: patch ΔE2000 mean {patch_mean:.4}, max {patch_max:.4}; \
         full frame p99 {p99:.4}, max {full_max:.4}"
    );
    assert!(patch_mean <= 0.5, "patch mean ΔE2000 {patch_mean}");
    assert!(patch_max <= 1.0, "patch max ΔE2000 {patch_max}");
    assert!(p99 <= 3.0, "full-frame p99 ΔE2000 {p99}");
}
