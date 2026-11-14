//! HAP, HAP Alpha and HAP Q import: the bundled FFmpeg's HAP decoder reads
//! each flavour, and the pixels pm-media hands the renderer are FFmpeg's own
//! decode of the file, alpha included. The files are synthetic
//! (`engine/fixtures/hap/make.swift`): no FFmpeg build encodes HAP.

mod common;

use common::ffmpeg_bin;
use pm_media::decode::service::{DecodeService, MediaSource};
use pm_media::frame::Layout;
use pm_media::vt::DecodePurpose;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/hap")
        .join(name)
}

/// FFmpeg's RGBA of every frame, 0..1.
fn ffmpeg_rgba(path: &Path) -> Vec<Vec<[f64; 4]>> {
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let frame = 64 * 32 * 4;
    out.stdout
        .chunks_exact(frame)
        .map(|f| {
            f.chunks_exact(4)
                .map(|p| [0, 1, 2, 3].map(|c| f64::from(p[c]) / 255.))
                .collect()
        })
        .collect()
}

#[test]
fn hap_hap_alpha_and_hap_q_decode_to_ffmpegs_pixels() {
    for (name, tag, alpha) in [
        ("hap1.mov", "Hap1", false),
        ("hap5.mov", "Hap5", true),
        ("hapy.mov", "HapY", false),
    ] {
        let path = fixture(name);
        let probe = pm_media::probe::probe(&path).unwrap().probe;
        let video = probe.video.as_ref().expect("a video stream");
        assert_eq!(
            (video.codec.codec.as_str(), video.codec.codec_tag.as_deref()),
            ("hap", Some(tag)),
            "{name}"
        );
        assert_eq!((video.width, video.height), (64, 32), "{name}");
        let want = ffmpeg_rgba(&path);
        assert_eq!(want.len(), 2, "{name}: FFmpeg's frames");
        // Every block column differs, so a wrong block order or channel shows.
        for frame in &want {
            assert!(
                (0..16).all(|b| frame[4 * b] != frame[4 * b + 4]),
                "{name}: flat frame"
            );
        }
        let source = MediaSource::open(&path).unwrap();
        let service = DecodeService::new(Default::default());
        let mut session = service.session(&source, 1, DecodePurpose::Export).unwrap();
        for (n, want) in want.iter().enumerate() {
            let frame = session.frame(n).unwrap();
            let format = &frame.format;
            assert!(
                matches!(format.layout, Layout::Rgba | Layout::PlanarRgb),
                "{name}: {:?}",
                format.layout
            );
            assert_eq!(format.has_alpha(), alpha, "{name}");
            // RGB layouts: [G, A] at luma positions, [B, R] at chroma ones.
            let (ga, br) = frame.reference().unwrap();
            let mut worst = 0f64;
            for (i, w) in want.iter().enumerate() {
                let got = [br[i][1], ga[i][0], br[i][0], ga[i][1]].map(f64::from);
                for c in 0..4 {
                    worst = worst.max((got[c] - w[c]).abs());
                }
            }
            assert!(
                worst < 0.5 / 255.,
                "{name} frame {n}: {:.1} codes from FFmpeg",
                worst * 255.
            );
        }
    }
}
