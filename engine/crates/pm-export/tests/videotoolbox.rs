//! VideoToolbox + AVAssetWriter exports end to end (macOS): encode, mux, verify, move
//! into place, then an independent read of the delivered file with Powermove's
//! `ffprobe` when it is available (`PM_FFMPEG_DIR` or the shared build).

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use pm_export::mp4box::Movie;
use pm_export::verify::{self, Expected};
use pm_export::*;
use pm_time::{Rate, TimecodeFormat};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-vt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Premultiplied test frame: moving hue gradient, a grey ramp, bars, and (with
/// `alpha`) a radial alpha falling to zero at the corners.
fn picture(w: u32, h: u32, frame: u32, alpha: bool) -> Vec<f32> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (u, t) = (x as f32 / (w - 1) as f32, y as f32 / (h - 1) as f32);
            let c = if t < 0.25 {
                [u, u, u]
            } else if t < 0.5 {
                let bars = [
                    [0.75, 0.75, 0.75],
                    [0.75, 0.75, 0.0],
                    [0.0, 0.75, 0.75],
                    [0.0, 0.75, 0.0],
                    [0.75, 0.0, 0.75],
                    [0.75, 0.0, 0.0],
                    [0.0, 0.0, 0.75],
                ];
                bars[((u * 7.0) as usize).min(6)]
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
    ExportSpec {
        width: 320,
        height: 180,
        rate: Rate::FPS_23_976,
        frames: 12,
        codec,
        container,
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Default,
        audio: None,
        timecode: None,
        destination: dir.join(format!("{name}.{}", container.extension())),
        overwrite: Overwrite::Fail,
        mode: None,
    }
}

/// Runs an export with the test picture and one frame's worth of audio per frame.
fn run(spec: ExportSpec) -> Result<Delivered, Error> {
    let mut job = export(spec.clone())?;
    let timing = spec.timing().unwrap();
    let mut audio_at = 0usize;
    for f in 0..spec.frames as u32 {
        let px = picture(spec.width, spec.height, f, spec.alpha != AlphaOutput::None);
        job.push_frame(&WorkingFrame {
            width: spec.width,
            height: spec.height,
            rgba: &px,
        })?;
        if let Some(a) = spec.audio {
            let end = ((f as i64 + 1) * timing.frame_duration * a.sample_rate as i64
                / timing.timescale as i64) as usize;
            job.push_audio(&tone(
                end - audio_at,
                a.channels as usize,
                audio_at,
                a.sample_rate as f32,
            ))?;
            audio_at = end;
        }
    }
    // Nothing is visible at the destination until the export finishes.
    if spec.overwrite == Overwrite::Fail {
        assert!(!spec.destination.exists());
    }
    let done = job.finish()?;
    let r = &done.report;
    eprintln!(
        "MEASURE {} {}x{} {} frames: max block error {:.3} CV at {} bits, hardware encoder {:?}",
        r.codec,
        spec.width,
        spec.height,
        r.frames,
        r.max_pixel_error_cv(),
        r.depth,
        r.hardware_encoder
    );
    Ok(done)
}

fn ffprobe() -> Option<PathBuf> {
    let dir = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        });
    let p = dir.join("bin/ffprobe");
    p.exists().then_some(p)
}

/// `ffprobe` stream fields as key=value lines, or None without a build.
fn probe(file: &Path, entries: &str) -> Option<String> {
    let ffprobe = ffprobe()?;
    let out = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_entries",
            entries,
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(String::from_utf8(out.stdout).unwrap())
}

#[test]
fn prores_422_hq_with_pcm_audio_and_drop_frame_timecode() {
    let dir = scratch("hq");
    let mut s = spec(
        &dir,
        "Master",
        Codec::ProRes(ProResProfile::Hq),
        Container::Mov,
    );
    s.rate = Rate::FPS_29_97;
    s.frames = 30;
    s.audio = Some(AudioSpec::default_for(s.container, 48_000, 2));
    let df = TimecodeFormat::drop_frame(Rate::FPS_29_97).unwrap();
    s.timecode = Some(TimecodeSpec {
        start_frame: df.parse("01:00:00;00").unwrap(),
        format: df,
        reel: Some("A001".into()),
    });
    let done = run(s.clone()).unwrap();
    assert_eq!(done.path, s.destination);
    let r = &done.report;
    eprintln!("{r:#?}");
    assert_eq!(
        (
            r.codec.as_str(),
            r.profile.as_str(),
            r.pixel_format.as_str(),
            r.depth
        ),
        ("apch", "ProRes 422 HQ", "4:2:2", 10)
    );
    assert_eq!((r.cicp, r.full_range, r.alpha), ((1, 13, 1), false, false));
    assert_eq!(
        (r.frames, r.movie_timescale, r.media_timescale, r.duration),
        (30, 30_000, 30_000, 30_030)
    );
    let audio = r.audio.as_ref().unwrap();
    assert_eq!(
        (
            audio.codec.as_str(),
            audio.channels,
            audio.timescale,
            audio.samples
        ),
        ("lpcm", 2, 48_000, 48_048)
    );
    let tc = r.timecode.as_ref().unwrap();
    assert_eq!(
        (
            tc.start_frame,
            tc.drop_frame,
            tc.frames_per_second,
            tc.reel.as_deref()
        ),
        (107_892, true, 30, Some("A001"))
    );
    assert_eq!(r.pixels.len(), 27);
    assert!(r.max_pixel_error_cv() <= verify::pixel_tolerance(s.codec));
    if let Some(p) = probe(
        &done.path,
        "stream=codec_name,profile,pix_fmt,color_primaries,color_transfer,color_space,color_range,nb_read_frames,codec_tag_string:stream_tags=timecode",
    ) {
        for want in [
            "profile=HQ",
            "pix_fmt=yuv422p10le",
            "color_transfer=iec61966-2-1",
            "color_range=tv",
            "nb_read_frames=30",
            "TAG:timecode=01:00:00;00",
        ] {
            assert!(p.contains(want), "ffprobe lacks {want}:\n{p}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prores_4444_with_straight_alpha_from_y416() {
    let dir = scratch("4444a");
    let mut s = spec(
        &dir,
        "Alpha",
        Codec::ProRes(ProResProfile::P4444),
        Container::Mov,
    );
    s.alpha = AlphaOutput::Straight;
    s.timecode = Some(TimecodeSpec {
        start_frame: 0,
        format: TimecodeFormat::for_rate(s.rate),
        reel: None,
    });
    let done = run(s.clone()).unwrap();
    let r = &done.report;
    eprintln!("{r:#?}");
    assert_eq!(
        (r.codec.as_str(), r.pixel_format.as_str(), r.depth, r.alpha),
        ("ap4h", "4:4:4:4", 12, true)
    );
    assert!(!r.timecode.as_ref().unwrap().drop_frame);
    // Alpha survives: blocks at the corners are transparent, the centre opaque.
    let alphas: Vec<f64> = r
        .pixels
        .iter()
        .filter(|p| p.frame == 0)
        .map(|p| p.decoded[3])
        .collect();
    assert!(
        alphas.iter().any(|&a| a < 0.01) && alphas.iter().any(|&a| a > 0.9),
        "{alphas:?}"
    );
    if let Some(p) = probe(&done.path, "stream=codec_tag_string,pix_fmt,nb_read_frames") {
        assert!(
            p.contains("pix_fmt=yuva444p12le") && p.contains("codec_tag_string=ap4h"),
            "{p}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prores_4444_xq_opaque() {
    let dir = scratch("xq");
    let s = spec(
        &dir,
        "Opaque",
        Codec::ProRes(ProResProfile::P4444Xq),
        Container::Mov,
    );
    let done = run(s).unwrap();
    let r = &done.report;
    eprintln!("{r:#?}");
    assert_eq!(
        (r.codec.as_str(), r.pixel_format.as_str(), r.alpha),
        ("ap4x", "4:4:4", false)
    );
    // No alpha channel is coded in the frames either.
    assert_eq!(coded_alpha(&done.path), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn opaque_prores_4444_codes_no_alpha_channel() {
    let dir = scratch("4444o");
    let s = spec(
        &dir,
        "Opaque",
        Codec::ProRes(ProResProfile::P4444),
        Container::Mov,
    );
    let done = run(s).unwrap();
    assert_eq!((done.report.depth, done.report.alpha), (12, false));
    assert_eq!(coded_alpha(&done.path), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

/// The `alpha_channel_type` of the first ProRes frame's header (0 none, 1
/// 8-bit, 2 16-bit).
fn coded_alpha(path: &Path) -> u8 {
    let data = std::fs::read(path).unwrap();
    let at = data
        .windows(4)
        .position(|w| w == b"icpf")
        .expect("a ProRes frame");
    // icpf, then the frame header: size (2), version (2), creator (4),
    // width (2), height (2), flags (2), reserved, primaries, transfer,
    // matrix, then source pixel format and alpha type in one byte.
    data[at + 4 + 17] & 0x0f
}

#[test]
fn h264_mp4_with_aac_and_quicktime_tags() {
    let dir = scratch("h264");
    let mut s = spec(
        &dir,
        "Web",
        Codec::H264 {
            bitrate_bps: 16_000_000,
        },
        Container::Mp4,
    );
    s.rate = Rate::FPS_30;
    s.frames = 45;
    s.audio = Some(AudioSpec::default_for(s.container, 48_000, 2));
    let done = run(s.clone()).unwrap();
    let r = &done.report;
    eprintln!("{r:#?}");
    assert_eq!(
        (r.codec.as_str(), r.pixel_format.as_str(), r.depth),
        ("avc1", "4:2:0", 8)
    );
    assert!(r.profile.starts_with("H.264 profile 100"), "{}", r.profile);
    assert_eq!((r.cicp, r.full_range), ((1, 13, 1), false));
    assert_eq!(
        (r.frames, r.movie_timescale, r.duration),
        (45, 15_360, 23_040)
    );
    let audio = r.audio.as_ref().unwrap();
    assert_eq!(audio.codec, "mp4a");
    assert!(
        audio.samples.abs_diff(72_000) <= 3,
        "{} AAC samples",
        audio.samples
    );
    // MP4 for the web: the movie header precedes the media data.
    let bytes = std::fs::read(&done.path).unwrap();
    let pos = |k: &[u8]| bytes.windows(4).position(|w| w == k).unwrap();
    assert!(pos(b"moov") < pos(b"mdat"), "not optimized for network use");
    if let Some(p) = probe(
        &done.path,
        "stream=codec_name,profile,pix_fmt,color_primaries,color_transfer,color_space,color_range,nb_read_frames",
    ) {
        for want in [
            "codec_name=h264",
            "profile=High",
            "pix_fmt=yuv420p",
            "color_primaries=bt709",
            "color_transfer=iec61966-2-1",
            "color_space=bt709",
            "color_range=tv",
            "nb_read_frames=45",
            "codec_name=aac",
        ] {
            assert!(p.contains(want), "ffprobe lacks {want}:\n{p}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn broadcast_tags_are_1_1_1() {
    let dir = scratch("broadcast");
    let mut s = spec(
        &dir,
        "Broadcast",
        Codec::ProRes(ProResProfile::Lt),
        Container::Mov,
    );
    s.tags = ColorTags::Rec709Broadcast;
    s.rate = Rate::FPS_25;
    let done = run(s).unwrap();
    assert_eq!(
        (
            done.report.codec.as_str(),
            done.report.cicp,
            done.report.media_timescale
        ),
        ("apcs", (1, 1, 1), 12_800)
    );
    if let Some(p) = probe(&done.path, "stream=color_transfer,r_frame_rate") {
        assert!(
            p.contains("color_transfer=bt709") && p.contains("r_frame_rate=25/1"),
            "{p}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn every_prores_flavor_encodes() {
    let dir = scratch("flavors");
    for (p, fourcc) in [
        (ProResProfile::Proxy, "apco"),
        (ProResProfile::Standard, "apcn"),
    ] {
        let mut s = spec(&dir, fourcc, Codec::ProRes(p), Container::Mov);
        s.frames = 2;
        assert_eq!(run(s).unwrap().report.codec, fourcc);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timescales_for_ntsc_rates_keep_exact_frame_times() {
    let dir = scratch("ntsc");
    for rate in [Rate::FPS_23_976, Rate::FPS_59_94, Rate::FPS_119_88] {
        let mut s = spec(
            &dir,
            &format!("r{}", rate.num()),
            Codec::ProRes(ProResProfile::Proxy),
            Container::Mov,
        );
        s.rate = rate;
        s.frames = 3;
        let r = run(s).unwrap().report;
        assert_eq!(
            (
                r.movie_timescale as i64,
                r.media_timescale as i64,
                r.duration
            ),
            (rate.num(), rate.num(), 3003)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn nothing_appears_at_the_destination_unless_the_export_completes() {
    let dir = scratch("atomic");
    // Cancelled half way: no destination, no temporary file.
    let s = spec(
        &dir,
        "Cancelled",
        Codec::H264 {
            bitrate_bps: 4_000_000,
        },
        Container::Mp4,
    );
    let mut job = export(s.clone()).unwrap();
    let px = picture(s.width, s.height, 0, false);
    for _ in 0..4 {
        job.push_frame(&WorkingFrame {
            width: s.width,
            height: s.height,
            rgba: &px,
        })
        .unwrap();
    }
    job.cancel();
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "cancel leaves nothing behind: {left:?}");
    // An incomplete export fails and leaves nothing.
    let mut job = export(s.clone()).unwrap();
    job.push_frame(&WorkingFrame {
        width: s.width,
        height: s.height,
        rgba: &px,
    })
    .unwrap();
    assert!(job.finish().is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    // An existing file is never replaced under Overwrite::Fail, and is under Replace.
    std::fs::write(&s.destination, b"keep").unwrap();
    assert!(matches!(export(s.clone()), Err(Error::Exists(_))));
    let mut replace = s.clone();
    replace.overwrite = Overwrite::Replace;
    replace.mode = Some(0o600);
    let done = run(replace).unwrap();
    assert_ne!(std::fs::read(&done.path).unwrap(), b"keep");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&done.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        left,
        vec![std::ffi::OsString::from("Cancelled.mp4")],
        "only the delivered file remains"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

/// Verification catches a file that is not what was asked for.
#[test]
fn verification_rejects_mismatches() {
    let dir = scratch("verify");
    let mut s = spec(
        &dir,
        "Probe",
        Codec::ProRes(ProResProfile::Hq),
        Container::Mov,
    );
    s.frames = 4;
    let done = run(s.clone()).unwrap();
    let movie = Movie::read(&done.path).unwrap();
    let timing = s.timing().unwrap();
    let format = s.input_format();
    let decoded: Vec<_> = verify::sampled_frames(s.frames)
        .into_iter()
        .map(|f| {
            (
                f,
                pm_export::mac::reader::decode_frame(&done.path, f, timing, format).unwrap(),
            )
        })
        .collect();
    let blocks: Vec<_> = decoded
        .iter()
        .flat_map(|(f, d)| verify::block_means(d, *f))
        .collect();
    let base = Expected::new(&s, timing, 0, blocks.clone());
    assert!(verify::check(&base, &movie, &decoded).is_ok());
    let mut wrong = s.clone();
    wrong.tags = ColorTags::Rec709Broadcast;
    let e = verify::check(
        &Expected::new(&wrong, timing, 0, blocks.clone()),
        &movie,
        &decoded,
    )
    .unwrap_err();
    assert!(e.to_string().contains("colr 1-13-1, expected 1-1-1"), "{e}");
    let mut wrong = s.clone();
    wrong.frames = 5;
    assert!(
        verify::check(
            &Expected::new(&wrong, timing, 0, blocks.clone()),
            &movie,
            &decoded
        )
        .unwrap_err()
        .to_string()
        .contains("4 video frames, expected 5")
    );
    let mut wrong = s.clone();
    wrong.codec = Codec::ProRes(ProResProfile::Standard);
    assert!(
        verify::check(
            &Expected::new(&wrong, timing, 0, blocks.clone()),
            &movie,
            &decoded
        )
        .is_err()
    );
    // A block whose decoded mean is far from what the encoder was handed.
    let mut shifted = blocks.clone();
    shifted[4].mean[0] += 0.05;
    let e = verify::check(&Expected::new(&s, timing, 0, shifted), &movie, &decoded).unwrap_err();
    assert!(e.to_string().contains("differs by"), "{e}");
    let mut with_audio = s.clone();
    with_audio.audio = Some(AudioSpec::default_for(with_audio.container, 48_000, 2));
    assert!(
        verify::check(
            &Expected::new(&with_audio, timing, 100, blocks),
            &movie,
            &decoded
        )
        .unwrap_err()
        .to_string()
        .contains("audio track missing")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn mp4_cannot_carry_timecode() {
    let dir = scratch("mp4tc");
    let mut s = spec(
        &dir,
        "Tc",
        Codec::H264 {
            bitrate_bps: 4_000_000,
        },
        Container::Mp4,
    );
    s.timecode = Some(TimecodeSpec {
        start_frame: 0,
        format: TimecodeFormat::for_rate(s.rate),
        reel: None,
    });
    match export(s) {
        Err(Error::InvalidSpec(m)) => assert!(m.contains("timecode"), "{m}"),
        Err(e) => panic!("{e}"),
        Ok(job) => {
            // AVAssetWriter accepted the track: then it must verify like MOV.
            job.cancel();
            panic!("MP4 accepted a timecode track; extend the MP4 tests to verify it");
        }
    }
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Audio may arrive before the first frame or after the last; the muxer queues it.
#[test]
fn audio_before_and_after_video_still_interleaves() {
    let dir = scratch("audio-order");
    for (i, audio_first) in [true, false].into_iter().enumerate() {
        let mut s = spec(
            &dir,
            &format!("Order{i}"),
            Codec::ProRes(ProResProfile::Proxy),
            Container::Mov,
        );
        s.rate = Rate::FPS_24;
        s.frames = 48;
        s.audio = Some(AudioSpec::default_for(s.container, 48_000, 1));
        let mut job = export(s.clone()).unwrap();
        let audio = tone(96_000, 1, 0, 48_000.0);
        if audio_first {
            job.push_audio(&audio).unwrap();
        }
        for f in 0..48 {
            let px = picture(s.width, s.height, f, false);
            job.push_frame(&WorkingFrame {
                width: s.width,
                height: s.height,
                rgba: &px,
            })
            .unwrap();
        }
        if !audio_first {
            job.push_audio(&audio).unwrap();
        }
        let r = job.finish().unwrap().report;
        let a = r.audio.unwrap();
        assert_eq!(
            (a.channels, a.samples),
            (1, 96_000),
            "audio first: {audio_first}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hevc_main_and_main10_in_mp4_and_mov() {
    let dir = scratch("hevc");
    for (ten_bit, container) in [
        (false, Container::Mp4),
        (true, Container::Mp4),
        (false, Container::Mov),
        (true, Container::Mov),
    ] {
        let mut s = spec(
            &dir,
            &format!("Hevc{}{:?}", if ten_bit { 10 } else { 8 }, container),
            Codec::Hevc {
                bitrate_bps: 16_000_000,
                ten_bit,
            },
            container,
        );
        s.audio = Some(AudioSpec::default_for(container, 48_000, 2));
        if container == Container::Mov {
            s.timecode = Some(TimecodeSpec {
                start_frame: 86_400,
                format: TimecodeFormat::for_rate(s.rate),
                reel: Some("A001".into()),
            });
        }
        let done = run(s.clone()).unwrap();
        let r = &done.report;
        eprintln!("{r:#?}");
        assert_eq!(
            (r.codec.as_str(), r.pixel_format.as_str(), r.depth, r.alpha),
            ("hvc1", "4:2:0", if ten_bit { 10 } else { 8 }, false)
        );
        assert_eq!(r.cicp, (1, 13, 1));
        assert!(!r.full_range);
        let audio = r.audio.as_ref().unwrap();
        let want = if container == Container::Mov {
            "lpcm"
        } else {
            "mp4a"
        };
        assert_eq!(audio.codec, want);
        if let Some(p) = probe(
            &done.path,
            "stream=codec_name,profile,pix_fmt,color_transfer",
        ) {
            let pix = if ten_bit { "yuv420p10le" } else { "yuv420p" };
            assert!(
                p.contains("codec_name=hevc") && p.contains(&format!("pix_fmt={pix}")),
                "{p}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hevc_with_alpha_is_8_bit_main_with_an_alpha_layer() {
    let dir = scratch("hevc-alpha");
    for alpha in [AlphaOutput::Straight, AlphaOutput::Premultiplied] {
        let mut s = spec(
            &dir,
            &format!("HevcAlpha{alpha:?}"),
            Codec::Hevc {
                bitrate_bps: 16_000_000,
                ten_bit: false,
            },
            Container::Mov,
        );
        s.alpha = alpha;
        let done = run(s).unwrap();
        let r = &done.report;
        let movie = Movie::read(&done.path).unwrap();
        let visual = movie.track(b"vide").unwrap().visual.clone().unwrap();
        eprintln!("ALMO {alpha:?} {:?}", visual.alpha_mode);
        assert_eq!(
            (r.codec.as_str(), r.pixel_format.as_str(), r.depth, r.alpha),
            ("hvc1", "4:2:0:4", 8, true)
        );
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
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pcm_16_24_and_float_audio_in_mov() {
    let dir = scratch("pcm");
    for codec in [
        AudioCodec::Pcm16,
        AudioCodec::Pcm24,
        AudioCodec::PcmF32,
        AudioCodec::Aac,
    ] {
        let mut s = spec(
            &dir,
            &format!("Pcm{codec:?}"),
            Codec::ProRes(ProResProfile::Lt),
            Container::Mov,
        );
        s.audio = Some(AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec,
        });
        let done = run(s.clone()).unwrap();
        let a = done.report.audio.clone().unwrap();
        eprintln!("{codec:?}: {a:?}");
        let (bits, float) = match codec {
            AudioCodec::Pcm16 => (16, false),
            AudioCodec::Pcm24 => (24, false),
            AudioCodec::PcmF32 => (32, true),
            _ => (a.bits_per_channel, a.float),
        };
        assert_eq!((a.bits_per_channel, a.float), (bits, float), "{codec:?}");
        // Frames × 1001/24000 × 48000 = 2002 samples per frame, after priming.
        assert_eq!(a.samples, 12 * 2002, "{codec:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn h264_in_mov_carries_timecode_and_pcm() {
    let dir = scratch("h264mov");
    let mut s = spec(
        &dir,
        "H264Mov",
        Codec::H264 {
            bitrate_bps: 4_000_000,
        },
        Container::Mov,
    );
    s.rate = Rate::FPS_29_97;
    s.audio = Some(AudioSpec::default_for(Container::Mov, 48_000, 2));
    let ndf = TimecodeFormat::non_drop(s.rate);
    s.timecode = Some(TimecodeSpec {
        start_frame: ndf.parse("01:00:00:00").unwrap(),
        format: ndf,
        reel: Some("Reel 7".into()),
    });
    let done = run(s.clone()).unwrap();
    let r = &done.report;
    let tc = r.timecode.as_ref().unwrap();
    assert_eq!((tc.start_frame, tc.drop_frame), (108_000, false));
    assert_eq!(tc.reel.as_deref(), Some("Reel 7"));
    assert_eq!(r.codec, "avc1");
    std::fs::remove_dir_all(dir).unwrap();
}
