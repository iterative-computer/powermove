//! Linux conformance, independently decoded through FFmpeg. All inputs are
//! generated here. No personal media or captured project data is used.
use pm_export::*;
use pm_time::{Rate, TimecodeFormat};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

// Optional external codec builds vary across developer machines. Missing
// capabilities are explicit skips, never disguised verification successes.
fn codecs_available(encoders: &[&str], decoders: &[&str]) -> bool {
    if !Command::new("ffprobe")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP software delivery: ffprobe is unavailable");
        return false;
    }
    for (flag, required) in [("-encoders", encoders), ("-decoders", decoders)] {
        let Ok(output) = Command::new("ffmpeg").args(["-hide_banner", flag]).output() else {
            eprintln!("SKIP software delivery: ffmpeg is unavailable");
            return false;
        };
        let listing = String::from_utf8_lossy(&output.stdout);
        for name in required {
            if !output.status.success()
                || !listing
                    .lines()
                    .any(|line| line.split_whitespace().nth(1) == Some(*name))
            {
                eprintln!("SKIP software delivery: ffmpeg {flag} lacks {name}");
                return false;
            }
        }
    }
    true
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "pm-software-{}-{}",
            std::process::id(),
            ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn spec(dir: &Path, codec: Codec, container: Container) -> ExportSpec {
    ExportSpec {
        width: 96,
        height: 64,
        rate: Rate::FPS_29_97,
        frames: 6,
        codec,
        container,
        tags: if matches!(codec, Codec::Png { .. }) {
            ColorTags::SrgbImage
        } else {
            ColorTags::Rec709QuickTime
        },
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: dir.join(format!("out.{}", container.extension())),
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
    }
}
fn picture(frame: u64, alpha: bool) -> Vec<f32> {
    (0..96 * 64)
        .flat_map(|i| {
            let x = i % 96;
            let a = if alpha && frame > 0 {
                match x / 32 {
                    0 => 0.0,
                    1 => 0.5,
                    _ => 1.0,
                }
            } else {
                1.0
            };
            let c = if x < 32 {
                [0.2, 0.3, 0.5]
            } else if x < 64 {
                [0.7, 0.5, 0.2]
            } else {
                [0.35, 0.6, 0.4]
            };
            [c[0] * a, c[1] * a, c[2] * a, a]
        })
        .collect()
}
fn portable_export(s: ExportSpec) -> Export {
    if matches!(s.codec, Codec::Png { .. }) {
        pm_export::export(s.clone()).unwrap()
    } else {
        Pipeline::new(
            s.clone(),
            OutputSink::Software(Box::new(software::SoftwareSink::new(&s).unwrap())),
        )
        .unwrap()
    }
}

fn write(s: ExportSpec) -> Delivered {
    let mut job = portable_export(s.clone());
    for i in 0..s.frames {
        let rgba = picture(i, s.alpha != AlphaOutput::None);
        job.push_frame(&WorkingFrame {
            width: s.width,
            height: s.height,
            rgba: &rgba,
        })
        .unwrap();
    }
    if let Some(a) = s.audio {
        let n = ((s.frames as f64 * s.rate.den() as f64 / s.rate.num() as f64)
            * f64::from(a.sample_rate))
        .round() as usize;
        job.push_audio(&tone(n, a.channels, a.sample_rate)).unwrap();
    }
    job.finish().unwrap()
}
fn tone(n: usize, ch: u16, rate: u32) -> Vec<f32> {
    (0..n)
        .flat_map(|i| {
            (0..ch).map(move |c| {
                0.25 * (std::f32::consts::TAU * (440.0 + 110.0 * f32::from(c)) * i as f32
                    / rate as f32)
                    .sin()
            })
        })
        .collect()
}
fn decode(path: &Path, pix: &str) -> Vec<u8> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-an", "-threads", "1", "-f", "rawvideo", "-pix_fmt", pix, "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
#[test]
fn every_prores_profile_and_alpha_survives_an_opaque_first_frame() {
    if !codecs_available(&["prores_ks"], &["prores"]) {
        return;
    }
    for p in [
        ProResProfile::Proxy,
        ProResProfile::Lt,
        ProResProfile::Standard,
        ProResProfile::Hq,
        ProResProfile::P4444,
        ProResProfile::P4444Xq,
    ] {
        for mode in [
            AlphaOutput::None,
            AlphaOutput::Straight,
            AlphaOutput::Premultiplied,
        ]
        .into_iter()
        .filter(|a| *a == AlphaOutput::None || p.is_4444())
        {
            let alpha = mode != AlphaOutput::None;
            let t = Temp::new();
            let mut s = spec(&t.0, Codec::ProRes(p), Container::Mov);
            s.alpha = mode;
            let d = write(s);
            assert_eq!(d.report.frames, 6);
            assert_eq!(d.report.alpha, alpha);
            assert!(d.report.max_pixel_error_cv() <= 4.0);
            if alpha {
                let rgba = decode(&d.path, "rgba64le");
                let pixel = |f: usize, x: usize| {
                    let i = (f * 96 * 64 + x) * 8;
                    u16::from_le_bytes([rgba[i + 6], rgba[i + 7]])
                };
                assert!((65535 - i32::from(pixel(0, 16))).abs() <= 16);
                assert_eq!(pixel(1, 16), 0);
                assert!((i32::from(pixel(1, 48)) - 32768).abs() < 258);
            }
        }
    }
}
#[test]
fn h264_quality_tiers_and_hevc_depths() {
    if !codecs_available(&["libx264", "libx265"], &["h264", "hevc"]) {
        return;
    }
    for codec in [
        Codec::H264 {
            bitrate_bps: 4_000_000,
        },
        Codec::H264 {
            bitrate_bps: 16_000_000,
        },
        Codec::H264 {
            bitrate_bps: 40_000_000,
        },
        Codec::Hevc {
            bitrate_bps: 16_000_000,
            ten_bit: false,
        },
        Codec::Hevc {
            bitrate_bps: 16_000_000,
            ten_bit: true,
        },
    ] {
        let t = Temp::new();
        let mut s = spec(&t.0, codec, Container::Mp4);
        s.rate = Rate::FPS_23_976;
        let d = write(s);
        assert_eq!(d.report.frames, 6);
        assert_eq!(d.report.depth, codec.depth());
    }
}
#[test]
fn pq_and_hlg_are_encoded_and_tagged_as_bt2100() {
    if !codecs_available(&["libx265"], &["hevc"]) {
        return;
    }
    for tags in [
        ColorTags::Rec2100Pq { white_nits: 203 },
        ColorTags::Rec2100Hlg {
            white_nits: 203,
            peak_nits: 1000,
        },
    ] {
        let t = Temp::new();
        let mut s = spec(
            &t.0,
            Codec::Hevc {
                bitrate_bps: 16_000_000,
                ten_bit: true,
            },
            Container::Mp4,
        );
        s.tags = tags;
        s.working = WorkingEncoding::Linear;
        let d = write(s);
        assert_eq!(d.report.cicp, tags.cicp());
        assert_eq!(d.report.depth, 10);
    }
}
#[test]
fn all_audio_codings_preserve_rate_channels_length_and_tone() {
    if !codecs_available(
        &["prores_ks", "aac", "pcm_s16le", "pcm_s24le", "pcm_f32le"],
        &[],
    ) {
        return;
    }
    for codec in [
        AudioCodec::Pcm16,
        AudioCodec::Pcm24,
        AudioCodec::PcmF32,
        AudioCodec::Aac,
    ] {
        for (sample_rate, channels) in [(44100, 1), (48000, 2)] {
            let t = Temp::new();
            let mut s = spec(&t.0, Codec::ProRes(ProResProfile::Hq), Container::Mov);
            let a = AudioSpec {
                sample_rate,
                channels,
                codec,
            };
            s.audio = Some(a);
            let n = (s.frames as f64 * s.rate.den() as f64 / s.rate.num() as f64
                * f64::from(sample_rate))
            .round() as usize;
            let d = write(s);
            let r = d.report.audio.unwrap();
            assert_eq!(r.channels, u32::from(channels));
            assert!(r.samples.abs_diff(n as u64) <= 1);
            let out = Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(d.path)
                .args(["-map", "0:a:0", "-f", "f32le", "pipe:1"])
                .output()
                .unwrap();
            assert!(out.status.success());
            let got: Vec<f32> = out
                .stdout
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let want = tone(n, channels, sample_rate);
            let rms = (got
                .iter()
                .zip(&want)
                .map(|(a, b)| f64::from(a - b).powi(2))
                .sum::<f64>()
                / want.len() as f64)
                .sqrt();
            if codec == AudioCodec::Aac {
                // AAC's first and final transform blocks contain startup/flush
                // transients. Judge the steady signal between them; the full
                // program, including both ends, still has its RMS bound below.
                let margin = 1024 * usize::from(channels);
                let end = want.len() - margin;
                assert!(end > margin);
                let steady = (got[margin..end]
                    .iter()
                    .zip(&want[margin..end])
                    .map(|(a, b)| f64::from(a - b).powi(2))
                    .sum::<f64>()
                    / (end - margin) as f64)
                    .sqrt();
                assert!(
                    steady < 0.003,
                    "AAC {sample_rate} Hz {channels} channels steady-state RMS {steady}"
                );
            }
            assert!(
                rms < if codec == AudioCodec::Aac {
                    0.02
                } else if codec == AudioCodec::PcmF32 {
                    1e-12
                } else if codec == AudioCodec::Pcm24 {
                    1e-7
                } else {
                    0.00004
                },
                "{codec:?} {sample_rate} Hz {channels} channels RMS {rms}"
            );
        }
    }
}
#[test]
fn webm_vp8_vp9_alpha_and_opus() {
    if !codecs_available(
        &["libvpx", "libvpx-vp9", "libopus"],
        &["libvpx", "libvpx-vp9"],
    ) {
        return;
    }
    for codec in [
        Codec::Vp8 {
            bitrate_bps: 4_000_000,
        },
        Codec::Vp9 {
            bitrate_bps: 4_000_000,
        },
    ] {
        for alpha in [false, true] {
            let t = Temp::new();
            let mut s = spec(&t.0, codec, Container::Webm);
            if alpha {
                s.alpha = AlphaOutput::Straight;
            }
            s.audio = Some(AudioSpec {
                sample_rate: 48000,
                channels: 2,
                codec: AudioCodec::Opus,
            });
            // Exercise the portable backend even when built with engine integration.
            let sink = software::SoftwareSink::new(&s).unwrap();
            let mut job = Pipeline::new(s.clone(), sink).unwrap();
            for i in 0..s.frames {
                job.push_frame(&WorkingFrame {
                    width: 96,
                    height: 64,
                    rgba: &picture(i, alpha),
                })
                .unwrap();
            }
            let n = (s.frames as f64 * s.rate.den() as f64 / s.rate.num() as f64 * 48000.0).round()
                as usize;
            job.push_audio(&tone(n, 2, 48000)).unwrap();
            let d = job.finish().unwrap();
            assert_eq!(d.report.alpha, alpha);
            assert_eq!(d.report.frames, 6);
        }
    }
}
#[test]
fn png_depths_still_sequence_and_alpha() {
    for sixteen_bit in [false, true] {
        for alpha in [false, true] {
            for container in [Container::PngStill, Container::PngSequence] {
                let t = Temp::new();
                let mut s = spec(&t.0, Codec::Png { sixteen_bit }, container);
                s.frames = if container == Container::PngStill {
                    1
                } else {
                    3
                };
                if alpha {
                    s.alpha = AlphaOutput::Straight;
                }
                let d = write(s);
                assert_eq!(d.report.depth, if sixteen_bit { 16 } else { 8 });
                assert_eq!(d.report.alpha, alpha);
                assert_eq!(d.report.max_pixel_error_cv(), 0.0);
            }
        }
    }
}
#[test]
fn cancellation_drop_and_spawn_failure_preserve_destination_and_clean_work() {
    if !codecs_available(&["libx264"], &[]) {
        return;
    }
    let t = Temp::new();
    let mut s = spec(
        &t.0,
        Codec::H264 {
            bitrate_bps: 4_000_000,
        },
        Container::Mp4,
    );
    std::fs::write(&s.destination, b"original").unwrap();
    s.overwrite = Overwrite::Replace;
    let mut job = portable_export(s.clone());
    for _ in 0..s.frames {
        job.push_frame(&WorkingFrame {
            width: 96,
            height: 64,
            rgba: &picture(0, false),
        })
        .unwrap();
    }
    assert!(matches!(
        job.finish_with_cancel(&AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
    assert_eq!(std::fs::read(&s.destination).unwrap(), b"original");
    drop(portable_export(s.clone()));
    let tools = software::SoftwareTools {
        ffmpeg: t.0.join("missing-ffmpeg"),
        ffprobe: "ffprobe".into(),
    };
    assert!(software::SoftwareSink::with_tools(&s, tools).is_err());
    assert_eq!(std::fs::read_dir(&t.0).unwrap().count(), 1);
}
#[test]
fn unsupported_hevc_alpha_and_eight_bit_hdr_fail_before_work() {
    let t = Temp::new();
    let mut s = spec(
        &t.0,
        Codec::Hevc {
            bitrate_bps: 4_000_000,
            ten_bit: false,
        },
        Container::Mov,
    );
    s.alpha = AlphaOutput::Straight;
    assert!(software::SoftwareSink::new(&s).is_err());
    s.alpha = AlphaOutput::None;
    s.tags = ColorTags::Rec2100Pq { white_nits: 203 };
    assert!(s.validate().is_err());
    assert_eq!(std::fs::read_dir(&t.0).unwrap().count(), 0);
}
#[test]
fn mov_timecode_is_carried_to_delivery() {
    if !codecs_available(&["prores_ks"], &["prores"]) {
        return;
    }
    let t = Temp::new();
    let mut s = spec(&t.0, Codec::ProRes(ProResProfile::Hq), Container::Mov);
    let format = TimecodeFormat::drop_frame(s.rate).unwrap();
    s.timecode = Some(TimecodeSpec {
        start_frame: format.parse("01:00:00;00").unwrap(),
        format,
        reel: Some("A001".into()),
    });
    let d = write(s);
    assert_eq!(d.report.timecode.unwrap().start_frame, 107892);
}
#[test]
fn shipped_v120_mp4_and_prores_decode_comparison() {
    if !codecs_available(&["prores_ks", "libx264"], &["prores", "h264"]) {
        return;
    }
    for prores in [false, true] {
        let t = Temp::new();
        let s = spec(
            &t.0,
            if prores {
                Codec::ProRes(ProResProfile::P4444)
            } else {
                Codec::H264 {
                    bitrate_bps: 16_000_000,
                }
            },
            if prores {
                Container::Mov
            } else {
                Container::Mp4
            },
        );
        let legacy = t.0.join(if prores { "v120.mov" } else { "v120.mp4" });
        // Exact encoderArgs from v1.2.0 render-encoder.ts; synthetic RGBA8 is
        // supplied at its canvas boundary. Pixel comparison allows the engine's
        // higher precision and explicit chroma-siting conversion.
        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-v",
            "error",
            "-f",
            "rawvideo",
            "-pixel_format",
            "rgba",
            "-video_size",
            "96x64",
            "-framerate",
            "30000/1001",
            "-i",
            "pipe:0",
            "-an",
            "-vf",
            "scale=in_range=full:out_range=limited:out_color_matrix=bt709",
            "-threads",
            "2",
        ]);
        if prores {
            cmd.args([
                "-c:v",
                "prores_ks",
                "-profile:v",
                "4",
                "-pix_fmt",
                "yuv444p10le",
                "-alpha_bits",
                "16",
            ]);
        } else {
            cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-b:v", "16M"]);
        }
        cmd.args([
            "-color_primaries",
            "bt709",
            "-color_trc",
            "iec61966-2-1",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
            "-movflags",
            "+faststart",
            "-n",
        ])
        .arg(&legacy)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let mut pipe = child.stdin.take().unwrap();
        for i in 0..s.frames {
            let bytes: Vec<u8> = picture(i, false)
                .iter()
                .map(|v| (v * 255.0).round() as u8)
                .collect();
            pipe.write_all(&bytes).unwrap();
        }
        drop(pipe);
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let native = write(s);
        let a = decode(&legacy, "rgba");
        let b = decode(&native.path, "rgba");
        assert_eq!(a.len(), b.len());
        let rms = (a
            .iter()
            .zip(&b)
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
            .sum::<f64>()
            / a.len() as f64)
            .sqrt();
        assert!(rms < 3.0, "ProRes {prores}: decoded RMS {rms} CV");
    }
}

#[test]
fn hdr_neutral_highlights_decode_to_standard_signal_values() {
    if !codecs_available(&["libx265"], &["hevc"]) {
        return;
    }
    for (tags, levels, signals) in [
        (
            ColorTags::Rec2100Pq { white_nits: 100 },
            [1.0, 10.0, 40.0],
            [0.5080784215, 0.7518270962, 0.9025723933],
        ),
        (
            ColorTags::Rec2100Hlg {
                white_nits: 203,
                peak_nits: 1000,
            },
            [0.0, 1.0, 1000.0 / 203.0],
            [0.0, 0.75, 1.0],
        ),
    ] {
        let t = Temp::new();
        let mut s = spec(
            &t.0,
            Codec::Hevc {
                bitrate_bps: 16_000_000,
                ten_bit: true,
            },
            Container::Mp4,
        );
        s.tags = tags;
        s.working = WorkingEncoding::Linear;
        s.frames = 2;
        let mut job = Pipeline::new(s.clone(), software::SoftwareSink::new(&s).unwrap()).unwrap();
        let rgba: Vec<f32> = (0..96 * 64)
            .flat_map(|i| {
                let v = levels[(i % 96) / 32];
                [v, v, v, 1.0]
            })
            .collect();
        for _ in 0..s.frames {
            job.push_frame(&WorkingFrame {
                width: 96,
                height: 64,
                rgba: &rgba,
            })
            .unwrap();
        }
        let d = job.finish().unwrap();
        let bytes = decode(&d.path, "p010le");
        for (x, signal) in [16, 48, 80].into_iter().zip(signals) {
            let i = (16 * 96 + x) * 2;
            let code = u16::from_le_bytes([bytes[i], bytes[i + 1]]) >> 6;
            assert!(
                (f64::from(code) - (64.0 + 876.0 * signal)).abs() < 2.0,
                "{tags:?}: code {code} signal {signal}"
            );
        }
    }
}

#[test]
fn every_supported_rate_has_exact_mov_mp4_timing() {
    if !codecs_available(&["libx264"], &[]) {
        return;
    }
    for rate in pm_time::SUPPORTED_RATES {
        let t = Temp::new();
        let mut s = spec(
            &t.0,
            Codec::H264 {
                bitrate_bps: 4_000_000,
            },
            Container::Mp4,
        );
        s.rate = rate;
        s.frames = 2;
        let timing = s.timing().unwrap();
        let d = write(s);
        assert_eq!(d.report.media_timescale, timing.timescale as u32);
        assert_eq!(d.report.duration, timing.duration(2) as u64);
    }
}

#[test]
fn cancelling_png_before_commit_preserves_existing_output() {
    let t = Temp::new();
    let mut s = spec(&t.0, Codec::Png { sixteen_bit: false }, Container::PngStill);
    s.frames = 1;
    s.overwrite = Overwrite::Replace;
    std::fs::write(&s.destination, b"original").unwrap();
    let mut job = export(s.clone()).unwrap();
    job.push_frame(&WorkingFrame {
        width: 96,
        height: 64,
        rgba: &picture(0, false),
    })
    .unwrap();
    assert!(matches!(
        job.finish_with_cancel(&AtomicBool::new(true)),
        Err(Error::Cancelled)
    ));
    assert_eq!(std::fs::read(&s.destination).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(&t.0).unwrap().count(), 1);
}

#[test]
fn probe_failure_never_publishes_or_replaces_user_output() {
    if !codecs_available(&["libx264"], &[]) {
        return;
    }
    let t = Temp::new();
    let mut s = spec(
        &t.0,
        Codec::H264 {
            bitrate_bps: 4_000_000,
        },
        Container::Mp4,
    );
    s.overwrite = Overwrite::Replace;
    std::fs::write(&s.destination, b"original").unwrap();
    let tools = software::SoftwareTools {
        ffmpeg: "ffmpeg".into(),
        ffprobe: t.0.join("missing-probe"),
    };
    let mut job = Pipeline::new(
        s.clone(),
        software::SoftwareSink::with_tools(&s, tools).unwrap(),
    )
    .unwrap();
    for _ in 0..s.frames {
        job.push_frame(&WorkingFrame {
            width: 96,
            height: 64,
            rgba: &picture(0, false),
        })
        .unwrap();
    }
    assert!(job.finish().is_err());
    assert_eq!(std::fs::read(&s.destination).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(&t.0).unwrap().count(), 1);
}
