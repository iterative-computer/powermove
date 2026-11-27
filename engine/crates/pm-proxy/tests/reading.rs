//! A proxy is read exactly as its original: the original's resolved colour
//! reading (tags where the file has them, the engine's defaults for the
//! original's size where it has none) is written into the proxy, and the
//! picture each decodes to agrees.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use pm_protocol::color::{ColorDescription, MatrixCoefficients, Range};
use pm_protocol::priority::{ProxyFormat, ProxySize};
use pm_proxy::{GenerationControl, GenerationRequest, ProxyStore, generate_with_disk_probe};

fn tools() -> (PathBuf, PathBuf) {
    let dir = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    (dir.join("bin/ffmpeg"), dir.join("bin/ffprobe"))
}

fn run(cmd: &mut Command) -> Vec<u8> {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// Left half 75% green, right half 75% red: the colours the audit saw move
/// (0,191,0 read with the wrong matrix becomes 15,224,6).
fn make_source(ffmpeg: &Path, path: &Path, (w, h): (u32, u32), codec: &[&str]) {
    let half = format!("{}x{h}", w / 2);
    run(Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(["-f", "lavfi", "-i"])
        .arg(format!("color=c=0x00BF00:s={half}:r=24:d=0.25"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("color=c=0xBF0000:s={half}:r=24:d=0.25"))
        .args(["-filter_complex", "hstack"])
        .args(codec)
        .arg(path));
}

fn reading(path: &Path) -> ColorDescription {
    pm_media::probe::probe(path)
        .unwrap()
        .probe
        .video
        .unwrap()
        .color
        .interpreted
}

/// Decodes the first frame to 16-bit RGB with `desc`'s matrix and range,
/// as the engine's source pass reads it, and returns the centre of each
/// half (0..1), averaged over a block.
fn patches(ffmpeg: &Path, path: &Path, desc: &ColorDescription) -> [[f64; 3]; 2] {
    let probed = pm_media::probe::probe(path).unwrap().probe.video.unwrap();
    let (w, h) = (probed.codec.width as usize, probed.codec.height as usize);
    let filter = if probed.codec.is_rgb() {
        "format=rgb48le".to_owned()
    } else {
        let matrix = match desc.matrix {
            MatrixCoefficients::Bt709 => "bt709",
            MatrixCoefficients::Bt601 => "bt601",
            MatrixCoefficients::Bt2020Ncl => "bt2020",
            m => panic!("matrix {m:?}"),
        };
        let range = match desc.range {
            Range::Full => "full",
            Range::Limited => "tv",
        };
        format!("scale=in_color_matrix={matrix}:in_range={range}:out_range=full,format=rgb48le")
    };
    let raw = run(Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &filter, "-f", "rawvideo", "-"]));
    assert_eq!(raw.len(), w * h * 6);
    // The mean of a block (a paletted source is dithered).
    let at = |cx: usize| {
        let (bw, bh) = (w / 8, h / 8);
        let mut sum = [0.0; 3];
        for y in h / 2 - bh / 2..h / 2 + bh / 2 {
            for x in cx - bw / 2..cx + bw / 2 {
                let i = (y * w + x) * 6;
                for (c, s) in sum.iter_mut().enumerate() {
                    *s += f64::from(u16::from_le_bytes([raw[i + 2 * c], raw[i + 2 * c + 1]]));
                }
            }
        }
        sum.map(|s| s / 65535.0 / (bw * (bh / 2 * 2)) as f64)
    };
    [at(w / 4), at(3 * w / 4)]
}

#[test]
fn a_proxy_is_read_with_its_originals_reading() {
    let (ffmpeg, ffprobe) = tools();
    let temp = tempfile::tempdir().unwrap();
    let store = ProxyStore::open(temp.path().join("store")).unwrap();
    let cancel = AtomicBool::new(false);
    for (name, size, codec) in [
        // Untagged Y′CbCr HD: the half-size proxy is "SD" by size.
        (
            "untagged-720p.mkv",
            (1280, 720),
            &["-c:v", "ffv1", "-pix_fmt", "yuv422p10le"][..],
        ),
        (
            "untagged-1080p.mkv",
            (1920, 1080),
            &["-c:v", "ffv1", "-pix_fmt", "yuv422p10le"][..],
        ),
        // Untagged MJPEG (full range by the engine's rule).
        (
            "mjpeg-720p.mov",
            (1280, 720),
            &["-c:v", "mjpeg", "-q:v", "1", "-pix_fmt", "yuvj422p"][..],
        ),
        // QuickTime Animation with alpha, and paletted video: R′G′B′.
        (
            "animation-720p.mov",
            (1280, 720),
            &["-c:v", "qtrle", "-pix_fmt", "argb"][..],
        ),
        (
            "pal8-720p.mov",
            (1280, 720),
            &["-c:v", "png", "-pix_fmt", "pal8"][..],
        ),
    ] {
        let source = temp.path().join(name);
        make_source(&ffmpeg, &source, size, codec);
        let hash = blake3::hash(&std::fs::read(&source).unwrap())
            .to_hex()
            .to_string();
        let proxy = generate_with_disk_probe(
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
                    on_progress: &|_| {},
                },
            },
            &|_| Ok((40 << 30, 100 << 30)),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let path = &proxy.stored.path;
        let (a, b) = (reading(&source), reading(path));
        assert_eq!(
            (a.primaries, a.transfer),
            (b.primaries, b.transfer),
            "{name}"
        );
        let rgb = pm_media::probe::probe(&source)
            .unwrap()
            .probe
            .video
            .unwrap()
            .codec
            .is_rgb();
        if rgb {
            assert_eq!(
                (a.matrix, a.range),
                (MatrixCoefficients::Identity, Range::Full),
                "{name}"
            );
            assert_eq!(
                (b.matrix, b.range),
                (MatrixCoefficients::Bt709, Range::Limited),
                "{name}"
            );
        } else {
            // ProRes is limited range: full-range codes are rescaled.
            assert_eq!((a.matrix, b.range), (b.matrix, Range::Limited), "{name}");
        }
        let (original, proxied) = (patches(&ffmpeg, &source, &a), patches(&ffmpeg, path, &b));
        let worst = original
            .iter()
            .flatten()
            .zip(proxied.iter().flatten())
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max);
        eprintln!("{name}: proxy {original:?} -> {proxied:?}, worst {worst:.4}");
        assert!(worst < 0.01, "{name}: {original:?} vs {proxied:?}");
    }
}
