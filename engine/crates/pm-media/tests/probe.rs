//! Probe (section 13): priming, color interpretation (section 9), field
//! order, nominal rate, variable rate, timecode, UMID, decode cost, poster,
//! filmstrip seed, peaks, and the content hash.

mod common;

use common::*;
use pm_media::probe::timing::PrimingSource;
use pm_media::probe::{AlphaSource, CostClass, FieldOrder, probe};
use pm_media::time::RateMethod;
use pm_protocol::color::{MatrixCoefficients, Primaries, Range};
use std::path::{Path, PathBuf};
use std::process::Command;

fn audio_priming(path: &Path) -> Option<(i64, PrimingSource)> {
    let p = probe(path).unwrap().probe;
    let a = p.streams.iter().find_map(|s| s.audio.as_ref()).expect("an audio stream");
    a.priming.map(|p| (p.samples, p.source))
}

fn tmp_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("pm-media-probe-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn priming_from_fixtures() {
    let f = fixtures_dir();
    // FFmpeg's MP3 writer stores LAME's 576-sample delay; +529 decoder delay.
    assert_eq!(audio_priming(&f.join("tone.mp3")), Some((576 + 529, PrimingSource::LameHeader)));
    // FFmpeg's AAC encoder: 1024 in the edit list.
    assert_eq!(audio_priming(&f.join("tone.m4a")), Some((1024, PrimingSource::EditList)));
    assert_eq!(audio_priming(&f.join("h264-aac.mov")), Some((1024, PrimingSource::EditList)));
    // Opus pre-skip.
    assert_eq!(audio_priming(&f.join("tone.opus")).map(|p| p.1), Some(PrimingSource::Codec));
    assert_eq!(audio_priming(&f.join("tone.flac")), None);
}

/// Apple's encoder: 2112 in an edit list (AudioToolbox through FFmpeg) or
/// in iTunSMPB (`afconvert`, which writes no edit list).
#[test]
fn apple_aac_priming() {
    let dir = tmp_dir();
    let wav = dir.join("sine.wav");
    let ok = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=44100", "-t", "1"])
        .arg(&wav)
        .status()
        .unwrap()
        .success();
    assert!(ok, "ffmpeg could not write a WAV");
    let m4a = dir.join("afconvert.m4a");
    let ok = Command::new("afconvert").args(["-f", "m4af", "-d", "aac"]).arg(&wav).arg(&m4a).status().unwrap().success();
    assert!(ok, "afconvert failed");
    let p = probe(&m4a).unwrap().probe;
    let pr = p.streams.iter().find_map(|s| s.audio.as_ref()).unwrap().priming.unwrap();
    assert_eq!((pr.samples, pr.source), (2112, PrimingSource::ITunSmpb));
    assert!(pr.padding.is_some());
}

#[test]
fn color_interpretation_follows_section_9() {
    let f = fixtures_dir();
    // Untagged SD-height video: BT.601, 525-line primaries, limited range.
    let p = probe(&f.join("hevc.mp4")).unwrap().probe;
    let c = &p.video.unwrap().color;
    assert_eq!((c.interpreted.primaries, c.interpreted.matrix, c.interpreted.range), (Primaries::Bt601_525, MatrixCoefficients::Bt601, Range::Limited));
    assert!(c.defaults_applied.iter().any(|d| d.contains("untagged matrix")));
    // ProRes 4444 with alpha.
    let p = probe(&f.join("prores4444-alpha.mov")).unwrap().probe;
    assert_eq!(p.video.unwrap().alpha, Some(AlphaSource::PixelFormat));
    // HEVC with an alpha layer.
    let p = probe(&f.join("hevc-alpha.mov")).unwrap().probe;
    assert_eq!(p.video.unwrap().alpha, Some(AlphaSource::HevcAlphaLayer));
}

#[test]
#[ignore]
fn corpus_probe_facts() {
    let c = corpus_dir();
    let v = |name: &str| probe(&c.join(name)).unwrap_or_else(|e| panic!("{name}: {e}")).probe;
    assert_eq!(audio_priming(&c.join("sine-aac-at.m4a")), Some((2112, PrimingSource::EditList)));
    assert_eq!(audio_priming(&c.join("sine-lame.mp3")), Some((1105, PrimingSource::LameHeader)));
    // HD untagged: BT.709; MJPEG reads as full range.
    let x = v("mxf-xdcam422-1080i-tff.mxf");
    let xv = x.video.as_ref().unwrap();
    assert_eq!(xv.color.interpreted.matrix, MatrixCoefficients::Bt709);
    assert_eq!(xv.fields.order, FieldOrder::TopFirst);
    assert!(x.umid.is_some(), "MXF material package UMID");
    assert_eq!(x.timecode.as_ref().map(|t| t.frame), Some(Some(0)));
    assert_eq!(v("mjpeg.avi").video.unwrap().color.interpreted.range, Range::Full);
    // DV: 525-line primaries by height, bottom field first from the bitstream.
    let dv = v("dv-ntsc.dv");
    let dvv = dv.video.as_ref().unwrap();
    assert_eq!(dvv.color.interpreted.primaries, Primaries::Bt601_525);
    assert_eq!(dvv.fields.order, FieldOrder::BottomFirst);
    assert_eq!(v("dv-pal.avi").video.unwrap().color.interpreted.primaries, Primaries::Bt601_625);
    // Nominal rates (S6 table) and how they were decided.
    for (name, rate, method) in [
        ("h264-422-10.mov", (25, 1), RateMethod::Median),
        ("hevc-main10.mov", (24000, 1001), RateMethod::MedianThenMean),
        ("webm-1ms-23976.webm", (24000, 1001), RateMethod::MedianThenMean),
        ("ts-90k-2997.ts", (30000, 1001), RateMethod::Median),
        ("mov-600-prores-2997.mov", (30000, 1001), RateMethod::MedianThenMean),
    ] {
        let n = v(name).video.unwrap().nominal_rate.unwrap();
        assert_eq!(((n.rate.num(), n.rate.den()), n.method), (rate, method), "{name}");
    }
    // Variable frame rate (every fifth frame dropped) is detected.
    let vfr = v("vfr-h264.mp4").video.unwrap();
    assert!(vfr.variable_rate);
    assert!(vfr.max_frame_duration > vfr.min_frame_duration);
    assert!(!v("h264-bpyramid.mp4").video.unwrap().variable_rate);
    // Decode cost, poster, filmstrip.
    let u = v("uhd-hevc-main10.mov").video.unwrap();
    assert!(u.decode.hardware);
    assert_eq!(v("prores422hq-vt.mov").video.unwrap().decode.class, CostClass::Light);
    let b = v("h264-bpyramid.mp4").video.unwrap();
    assert!(b.decode.max_gop >= 48 && !b.decode.intra_only);
    assert!(b.poster_frame >= b.frames / 10);
    assert!(!b.filmstrip_seed.is_empty() && b.filmstrip_seed.iter().all(|&n| n < b.frames));
}

#[test]
fn timecode_with_drop_frame() {
    let dir = tmp_dir();
    let out = dir.join("tc.mov");
    let ok = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=30000/1001", "-frames:v", "10"])
        .args(["-c:v", "mjpeg", "-timecode", "01:00:00;00"])
        .arg(&out)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let p = probe(&out).unwrap().probe;
    let tc = p.timecode.expect("timecode");
    assert!(tc.drop_frame);
    // 01:00:00;00 at 29.97 drop-frame is frame 107,892.
    assert_eq!(tc.frame, Some(107_892));
}

#[test]
fn waveform_peaks() {
    let f = fixtures_dir().join("tone.wav");
    let pk = pm_media::probe::peaks::peaks(&f, None, 480, 0).unwrap();
    assert_eq!(pk.channels, 2);
    assert_eq!(pk.data.len() as u64, pk.samples.div_ceil(480) * 2);
    let max = pk.data.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
    let min = pk.data.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
    assert!(max > 0.05 && max <= 1.0 && min < -0.05 && min >= -1.0, "{min} {max}");
    // Skipping priming shortens the decoded program audio by exactly that much.
    let skipped = pm_media::probe::peaks::peaks(&f, None, 480, 1000).unwrap();
    assert_eq!(skipped.skipped, 1000);
    assert_eq!(skipped.samples + 1000, pk.samples);
}

#[test]
fn content_hash_and_fingerprint() {
    let f = fixtures_dir().join("h264-aac.mp4");
    let fp = pm_media::hash::sampled_fingerprint(&f).unwrap();
    assert!(fp.starts_with("v2:"));
    let never = std::sync::atomic::AtomicBool::new(false);
    let settle = pm_media::hash::Settle { quiet: std::time::Duration::ZERO, poll: std::time::Duration::from_millis(10) };
    let h = pm_media::hash::hash_file(&f, settle, &never, &|_| {}).unwrap();
    assert_eq!(h.blake3, *blake3::hash(&std::fs::read(&f).unwrap()).as_bytes());
}
