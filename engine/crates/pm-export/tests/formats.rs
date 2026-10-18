//! Conformance of the formats written outside AVAssetWriter (plan section 17,
//! "Formats at cutover"; section 25, codec and export conformance): WebM with
//! VP9 and VP8 (with and without alpha) and Opus through FFmpeg, and PNG
//! sequences and stills at 8 and 16 bits. Each export is verified by the
//! exporter itself (frame count, timestamps, tags, range, alpha, audio samples,
//! sampled pixels) and then probed independently with Powermove's `ffprobe`.

use std::path::{Path, PathBuf};
use std::process::Command;

use pm_export::*;
use pm_time::Rate;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-fmt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Premultiplied test frame: a moving gradient, bars, a grey ramp, and (with
/// `alpha`) a radial alpha falling to zero at the corners.
fn picture(w: u32, h: u32, frame: u32, alpha: bool) -> Vec<f32> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (u, t) = (x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32);
            let c = if t < 0.3 {
                [u, u, u]
            } else if t < 0.6 {
                let bars = [
                    [0.75, 0.75, 0.75],
                    [0.75, 0.75, 0.0],
                    [0.0, 0.75, 0.75],
                    [0.0, 0.75, 0.0],
                    [0.75, 0.0, 0.75],
                ];
                bars[((u * 5.0) as usize).min(4)]
            } else {
                let p = (u + frame as f32 * 0.02).fract();
                [p, 1.0 - p, 0.5 * t]
            };
            let a = if alpha {
                let (dx, dy) = (u - 0.5, t - 0.5);
                (1.0 - (dx * dx + dy * dy).sqrt() * 1.6).clamp(0.0, 1.0)
            } else {
                1.0
            };
            v.extend([c[0] * a, c[1] * a, c[2] * a, a]);
        }
    }
    v
}

fn tone(frames: usize, channels: usize, start: usize, rate: f32) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            (0..channels).map(move |c| {
                0.25 * ((start + i) as f32 * (440.0 + 110.0 * c as f32) * std::f32::consts::TAU
                    / rate)
                    .sin()
            })
        })
        .collect()
}

fn spec(dir: &Path, name: &str, codec: Codec, container: Container) -> ExportSpec {
    let image = matches!(codec, Codec::Png { .. });
    ExportSpec {
        width: 320,
        height: 180,
        rate: Rate::FPS_23_976,
        frames: 30,
        codec,
        container,
        tags: if image {
            ColorTags::SrgbImage
        } else {
            ColorTags::Rec709QuickTime
        },
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Default,
        audio: None,
        timecode: None,
        destination: if container == Container::PngSequence {
            dir.join(name)
        } else {
            dir.join(format!("{name}.{}", container.extension()))
        },
        overwrite: Overwrite::Fail,
        mode: None,
    }
}

/// Exports the test picture with one frame's worth of audio per frame.
fn run(spec: ExportSpec) -> Result<Delivered, Error> {
    let mut job = export(spec.clone())?;
    let mut audio_at = 0usize;
    for f in 0..spec.frames as u32 {
        let px = picture(spec.width, spec.height, f, spec.alpha != AlphaOutput::None);
        job.push_frame(&WorkingFrame {
            width: spec.width,
            height: spec.height,
            rgba: &px,
        })?;
        if let Some(a) = spec.audio {
            let end = ((f as u64 + 1) * a.sample_rate as u64 * spec.rate.den() as u64
                / spec.rate.num() as u64) as usize;
            job.push_audio(&tone(
                end - audio_at,
                a.channels as usize,
                audio_at,
                a.sample_rate as f32,
            ))?;
            audio_at = end;
        }
    }
    assert!(
        !spec.destination.exists(),
        "nothing before the export finishes"
    );
    let done = job.finish()?;
    let r = &done.report;
    eprintln!(
        "MEASURE {} {} {}x{} {} frames: max block error {:.3} CV, {} bytes",
        r.codec,
        r.pixel_format,
        spec.width,
        spec.height,
        r.frames,
        r.max_pixel_error_cv(),
        r.bytes
    );
    Ok(done)
}

fn ffprobe() -> Option<PathBuf> {
    let dir = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("Library/Caches/powermove-engine/ffmpeg/out"))
        })?;
    let p = dir.join("bin/ffprobe");
    p.is_file().then_some(p)
}

fn probe(file: &Path, entries: &str, count: bool) -> Option<String> {
    let mut cmd = Command::new(ffprobe()?);
    cmd.args(["-v", "error"]);
    if count {
        cmd.arg("-count_frames");
    }
    let out = cmd
        .args(["-show_entries", entries, "-of", "default=nw=1"])
        .arg(file)
        .output()
        .ok()?;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn webm_vp9_with_opus_is_frame_exact_and_tagged() {
    let dir = scratch("vp9");
    let mut s = spec(
        &dir,
        "Vp9",
        Codec::Vp9 {
            bitrate_bps: 4_000_000,
        },
        Container::Webm,
    );
    s.audio = Some(AudioSpec::default_for(Container::Webm, 48_000, 2));
    let done = run(s.clone()).unwrap();
    let r = &done.report;
    assert_eq!(
        (r.codec.as_str(), r.pixel_format.as_str(), r.frames, r.alpha),
        ("vp09", "4:2:0", 30, false)
    );
    assert_eq!(r.cicp, (1, 13, 1));
    // round(30 × 1001/24000 × 48000) = 60060.
    assert_eq!(r.audio.as_ref().unwrap().samples, 60_060);
    if let Some(p) = probe(
        &done.path,
        "stream=codec_name,pix_fmt,color_range,color_transfer,color_primaries,color_space,chroma_location,nb_read_frames,r_frame_rate",
        true,
    ) {
        for want in [
            "codec_name=vp9",
            "pix_fmt=yuv420p",
            "color_range=tv",
            "color_transfer=iec61966-2-1",
            "color_primaries=bt709",
            "color_space=bt709",
            "chroma_location=left",
            "nb_read_frames=30",
            "r_frame_rate=24000/1001",
            "codec_name=opus",
        ] {
            assert!(p.contains(want), "{want} in {p}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn webm_vp9_and_vp8_with_alpha() {
    let dir = scratch("alpha");
    for codec in [
        Codec::Vp9 {
            bitrate_bps: 8_000_000,
        },
        Codec::Vp8 {
            bitrate_bps: 8_000_000,
        },
    ] {
        let mut s = spec(&dir, &codec.name(), codec, Container::Webm);
        s.alpha = AlphaOutput::Straight;
        s.tags = ColorTags::Rec709Broadcast;
        let done = run(s).unwrap();
        let r = &done.report;
        assert_eq!(
            (r.pixel_format.as_str(), r.alpha, r.frames),
            ("4:2:0:4", true, 30)
        );
        assert_eq!(r.cicp, (1, 1, 1));
        let alphas: Vec<f64> = r
            .pixels
            .iter()
            .filter(|p| p.frame == 0)
            .map(|p| p.decoded[3])
            .collect();
        assert!(
            alphas.iter().any(|&a| a < 0.02) && alphas.iter().any(|&a| a > 0.9),
            "{alphas:?}"
        );
        if let Some(p) = probe(&done.path, "stream_tags=alpha_mode", false) {
            assert!(p.to_ascii_lowercase().contains("alpha_mode=1"), "{p}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn webm_vp8_opaque_at_29_97() {
    let dir = scratch("vp8");
    let mut s = spec(
        &dir,
        "Vp8",
        Codec::Vp8 {
            bitrate_bps: 4_000_000,
        },
        Container::Webm,
    );
    s.rate = Rate::FPS_29_97;
    s.frames = 45;
    s.audio = Some(AudioSpec::default_for(Container::Webm, 48_000, 1));
    let done = run(s).unwrap();
    let r = &done.report;
    assert_eq!((r.codec.as_str(), r.frames), ("vp08", 45));
    // round(45 × 1001/30000 × 48000) = 72072.
    assert_eq!(r.audio.as_ref().unwrap().samples, 72_072);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn png_sequences_and_stills_at_8_and_16_bits() {
    let dir = scratch("png");
    for (sixteen_bit, alpha) in [(false, false), (false, true), (true, false), (true, true)] {
        let name = format!(
            "Seq{}{}",
            if sixteen_bit { 16 } else { 8 },
            if alpha { "A" } else { "" }
        );
        let mut s = spec(
            &dir,
            &name,
            Codec::Png { sixteen_bit },
            Container::PngSequence,
        );
        s.width = 33;
        s.height = 17;
        s.frames = 12;
        if alpha {
            s.alpha = AlphaOutput::Straight;
        }
        let done = run(s.clone()).unwrap();
        assert!(done.path.is_dir());
        let mut files: Vec<String> = std::fs::read_dir(&done.path)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        assert_eq!(files.len(), 12);
        assert_eq!(files[0], format!("{name}_00000.png"));
        assert_eq!(files[11], format!("{name}_00011.png"));
        let (info, decoded) = pm_export::png::read(&done.path.join(&files[3]), true).unwrap();
        assert_eq!(
            (info.width, info.height, info.bit_depth, info.alpha),
            (33, 17, if sixteen_bit { 16 } else { 8 }, alpha)
        );
        assert!(info.srgb);
        assert_eq!(info.cicp, Some((1, 13, 0, 1)));
        // Lossless: the file equals a fresh conversion of the same frame.
        let px = picture(33, 17, 3, alpha);
        let params = pm_export::convert::ConvertParams {
            legacy_png: false,
            working: WorkingEncoding::Standard,
            alpha: s.alpha,
            dither: !sixteen_bit,
            frame: 3,
        };
        let mut want = pm_export::convert::convert(
            &WorkingFrame {
                width: 33,
                height: 17,
                rgba: &px,
            },
            s.input_format(),
            &params,
        )
        .unwrap();
        if !alpha {
            match &mut want.planes[0].data {
                pm_export::convert::PlaneData::U8(v) => {
                    v.chunks_exact_mut(4).for_each(|p| p[3] = 255)
                }
                pm_export::convert::PlaneData::U16(v) => {
                    v.chunks_exact_mut(4).for_each(|p| p[3] = 65535)
                }
            }
        }
        assert_eq!(decoded.unwrap(), want, "{name}");
    }
    // A still, Rec.709-tagged.
    let mut s = spec(
        &dir,
        "Still",
        Codec::Png { sixteen_bit: true },
        Container::PngStill,
    );
    s.frames = 1;
    s.tags = ColorTags::Rec709Image;
    let done = run(s).unwrap();
    assert!(done.path.is_file());
    let (info, _) = pm_export::png::read(&done.path, false).unwrap();
    assert_eq!((info.cicp, info.srgb), (Some((1, 1, 0, 1)), false));
    // An existing sequence folder is never replaced under the fail policy.
    let s = spec(
        &dir,
        "Seq8",
        Codec::Png { sixteen_bit: false },
        Container::PngSequence,
    );
    assert!(matches!(export(s), Err(Error::Exists(_))));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_dropped_webm_or_png_export_leaves_nothing() {
    let dir = scratch("drop");
    let s = spec(
        &dir,
        "Drop",
        Codec::Vp9 {
            bitrate_bps: 4_000_000,
        },
        Container::Webm,
    );
    let mut job = export(s.clone()).unwrap();
    let px = picture(320, 180, 0, false);
    job.push_frame(&WorkingFrame {
        width: 320,
        height: 180,
        rgba: &px,
    })
    .unwrap();
    job.cancel();
    let s = spec(
        &dir,
        "DropSeq",
        Codec::Png { sixteen_bit: false },
        Container::PngSequence,
    );
    let mut job = export(s).unwrap();
    job.push_frame(&WorkingFrame {
        width: 320,
        height: 180,
        rgba: &px,
    })
    .unwrap();
    drop(job);
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "{left:?}");
    std::fs::remove_dir_all(dir).unwrap();
}
