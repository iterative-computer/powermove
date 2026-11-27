//! Where a delivered file's sound starts, not only how long it lasts.
//!
//! A composition carries two short chirps at known samples. A work area
//! that starts one second in is exported through the job engine to H.264
//! (MP4, AAC) and ProRes 422 HQ (MOV, 24-bit PCM), and the file's sound is
//! decoded by FFmpeg and by Apple's decoder (`afconvert`): each chirp must
//! sit at its sample (the encoder's priming is taken out by the edit list,
//! not left in front of the sound), nothing may sound before the first
//! one, and the decoded sound must last the frames' span.
//!
//! The encoder itself is driven at 44.1, 48 and 96 kHz the same way (the
//! app's job always mixes at 48 kHz).

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::convert::WorkingFrame;
use pm_export::mp4box::Movie;
use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, ExportFormat, ExportSettings, ProResProfile as WireProRes,
    QualityTier,
};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::json;
use support::doc::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-onset-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 20 ms chirp (500 Hz to 4 kHz) under a raised-cosine window, half scale.
fn chirp(rate: u32) -> Vec<f32> {
    let n = (rate / 50) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            let sweep = 500.0 * t + (4000.0 - 500.0) * t * t / (2.0 * n as f64 / f64::from(rate));
            let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            (0.5 * window * (std::f64::consts::TAU * sweep).sin()) as f32
        })
        .collect()
}

/// `samples` of silence (mono) with a chirp starting at each of `at`.
fn signal(rate: u32, samples: usize, at: &[usize]) -> Vec<f32> {
    let mut out = vec![0f32; samples];
    for &start in at {
        for (i, v) in chirp(rate).into_iter().enumerate() {
            out[start + i] = v;
        }
    }
    out
}

fn write_wav(path: &Path, rate: u32, mono: &[f32]) {
    let data: Vec<u8> = mono
        .iter()
        .flat_map(|&v| {
            let s = (f64::from(v) * 32767.0).round() as i16;
            [s.to_le_bytes(), s.to_le_bytes()].concat()
        })
        .collect();
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend(b"RIFF");
    wav.extend((36 + data.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((data.len() as u32).to_le_bytes());
    wav.extend(data);
    std::fs::write(path, wav).unwrap();
}

/// The file's first audio channel as a player hears it, decoded by FFmpeg
/// (edit lists applied) at the stream's own rate.
fn ffmpeg_decode(path: &Path) -> Vec<f32> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-vn", "-map", "0:a:0", "-f", "f32le", "-ac", "2", "pipe:1"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
        .chunks_exact(8)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// The same through AudioToolbox (`afconvert`), or `None` where the tool
/// cannot open the file.
fn apple_decode(path: &Path, scratch: &Path) -> Option<Vec<f32>> {
    let wav = scratch.join(format!(
        "{}.apple.wav",
        path.file_name().unwrap().to_string_lossy()
    ));
    let out = Command::new("/usr/bin/afconvert")
        .args(["-f", "WAVE", "-d", "LEF32"])
        .arg(path)
        .arg(&wav)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!(
            "NOTE afconvert cannot read {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return None;
    }
    let bytes = std::fs::read(&wav).ok()?;
    // The `data` chunk of the float WAVE it wrote: interleaved f32.
    let mut at = 12;
    let (mut channels, mut data) = (2usize, None);
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        match &bytes[at..at + 4] {
            b"fmt " => {
                channels = u16::from_le_bytes([bytes[at + 10], bytes[at + 11]]) as usize;
            }
            b"data" => {
                data = Some(&bytes[at + 8..(at + 8 + size).min(bytes.len())]);
                break;
            }
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    Some(
        data?
            .chunks_exact(4 * channels)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
    )
}

/// How many samples late (positive) the chirp expected at `at` sits in
/// `got`: the lag of the largest correlation within ±4,096 samples (an
/// encoder's priming left in front is 2,112, a missing packet 1,024).
fn lag(got: &[f32], rate: u32, at: usize) -> i64 {
    let burst = chirp(rate);
    let score = |lag: i64| -> f64 {
        burst
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let k = at as i64 + lag + i as i64;
                let g = if k >= 0 && (k as usize) < got.len() {
                    got[k as usize]
                } else {
                    0.0
                };
                f64::from(b) * f64::from(g)
            })
            .sum()
    };
    (-4096..=4096)
        .map(|l| (l, score(l)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
        .0
}

struct Heard {
    /// Samples each chirp sits from where it should.
    lags: Vec<i64>,
    /// The loudest sample before the first chirp's neighbourhood.
    before: f32,
    /// The chirps' peak.
    peak: f32,
    samples: usize,
}

fn hear(got: &[f32], rate: u32, at: &[usize]) -> Heard {
    let quiet = at[0].saturating_sub(2048).min(got.len());
    Heard {
        lags: at.iter().map(|&a| lag(got, rate, a)).collect(),
        before: got[..quiet].iter().fold(0.0, |m, v| m.max(v.abs())),
        peak: got.iter().fold(0.0, |m, v| m.max(v.abs())),
        samples: got.len(),
    }
}

/// Checks one decoder's reading of a file against where the chirps should
/// be and how long the sound should last (`slack` samples either way).
fn check(what: &str, got: &[f32], rate: u32, at: &[usize], samples: usize, slack: usize) {
    let h = hear(got, rate, at);
    eprintln!(
        "MEASURE {what}: chirps {:?} samples from their place, loudest before the first {:.5}, peak {:.3}, {} samples decoded of {samples}",
        h.lags, h.before, h.peak, h.samples
    );
    assert!(
        h.lags.iter().all(|l| *l == 0),
        "{what}: the sound is shifted by {:?} samples",
        h.lags
    );
    assert!(
        h.before < 0.01,
        "{what}: {} sounds before the first chirp",
        h.before
    );
    assert!(h.peak > 0.3, "{what}: the chirps are missing ({})", h.peak);
    assert!(
        h.samples.abs_diff(samples) <= slack,
        "{what}: {} samples decoded, {samples} expected",
        h.samples
    );
    // Whatever a decoder returns past the sound's end is silence.
    let surplus = got[samples.min(got.len())..]
        .iter()
        .fold(0f32, |m, v| m.max(v.abs()));
    assert!(surplus < 0.001, "{what}: {surplus} sounds after the end");
}

/// How far a decoder's sample count may be from the sound's length. Apple's
/// decoder and PCM are exact. FFmpeg applies an AAC track's edit list at the
/// start to the sample, but ends on a packet boundary: up to one packet
/// (1,024 samples) of the encoder's padding follows.
fn slack(aac: bool, ffmpeg: bool) -> usize {
    if aac && ffmpeg { 1024 } else { 1 }
}

/// The audio track's edit list and codec delay as the container states it.
fn edit(path: &Path) -> String {
    let movie = Movie::read(path).unwrap();
    let track = movie.track(b"soun").unwrap();
    format!("edits {:?}", track.edits)
}

#[test]
fn a_work_area_export_starts_its_sound_at_the_work_area() {
    let dir = scratch("job");
    let rate = 48_000u32;
    let dur = 4.0;
    // Chirps at 1.25 s and 2.4 s of the composition.
    let at = [60_000usize, 115_200];
    let clip = dir.join("chirps.wav");
    write_wav(&clip, rate, &signal(rate, (dur * 48_000.0) as usize, &at));
    let formats = [
        ("h264-aac", ExportFormat::H264, "mp4"),
        (
            "prores-pcm",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            "mov",
        ),
    ];
    for (num, den) in [(25u32, 1u32), (30000, 1001)] {
        for (name, format, ext) in formats {
            let fps = f64::from(num) / f64::from(den);
            let mut doc = project(
                64,
                36,
                dur,
                solid_fill("#14161C"),
                vec![audio("sound", "chirps", 0.0, dur)],
            );
            doc["fps"] = json!(fps);
            asset(&mut doc, "chirps", "audio", "wav", 0, 0, dur);
            let mut media = MediaPaths::default();
            media.insert("chirps", clip.clone());
            let destination = dir.join(format!("{name}-{num}.{ext}"));
            let job = ExportJob {
                document: Arc::new(load(&doc)),
                composition: "Ptest-root".into(),
                settings: ExportSettings {
                    format,
                    alpha: WireAlpha::None,
                    quality: QualityTier::Draft,
                    size: Size::new(64, 36),
                    frame_rate: FrameRate::new(num, den),
                    // The work area: 1.0 s to 2.6 s.
                    range: TickRange {
                        start: Ticks::new(TICKS_PER_SECOND).unwrap(),
                        end: Ticks::new(TICKS_PER_SECOND * 26 / 10).unwrap(),
                    },
                    audio: true,
                    audio_index_playhead: None,
                    motion_blur: false,
                    color: OutputColor::Rec709QuickTime {
                        candidate: QuickTimeTag::Cicp1_13_1,
                    },
                    precision: Precision::Float16,
                    dither: false,
                    draft_from_proxies: false,
                    delivery: Default::default(),
                },
                media: Arc::new(media),
                destination: destination.clone(),
                overwrite: Overwrite::Fail,
                mode: None,
                seed: 1,
            };
            let plan = plan_export(&job).unwrap();
            let first = plan.timeline.samples(0, rate).0;
            let samples = export_audio_samples(&plan.timeline, rate) as usize;
            let mut renderer = Renderer::headless().unwrap();
            run_export(
                &mut renderer,
                &job,
                &Unrestricted,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_or_else(|e| panic!("{name} at {fps}: {e}"));
            // Where the chirps sit in the file: the work area's start is
            // its first sample.
            let placed: Vec<usize> = at.iter().map(|a| a - first as usize).collect();
            let what = format!("{name} at {fps:.3} fps");
            eprintln!(
                "MEASURE {what}: work area from sample {first}, {samples} samples; {}",
                edit(&destination)
            );
            let aac = ext == "mp4";
            check(
                &format!("{what}, FFmpeg"),
                &ffmpeg_decode(&destination),
                rate,
                &placed,
                samples,
                slack(aac, true),
            );
            if let Some(got) = apple_decode(&destination, &dir) {
                let what = format!("{what}, Apple");
                check(&what, &got, rate, &placed, samples, slack(aac, false));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_encoder_places_sound_at_its_sample_at_every_rate() {
    let dir = scratch("rates");
    for rate in [44_100u32, 48_000, 96_000] {
        for (name, codec, video, container) in [
            (
                "aac",
                AudioCodec::Aac,
                Codec::H264 {
                    bitrate_bps: 4_000_000,
                },
                Container::Mp4,
            ),
            (
                "pcm24",
                AudioCodec::Pcm24,
                Codec::ProRes(ProResProfile::Hq),
                Container::Mov,
            ),
        ] {
            let frames = 50u64;
            let samples = (frames * u64::from(rate) / 25) as usize;
            // The AAC encoder takes no rate above 48 kHz: asked for one, the
            // export says so before anything is written.
            let unsupported = codec == AudioCodec::Aac && rate > 48_000;
            // Chirps a quarter and three quarters of the way through.
            let at = [samples / 4, samples * 3 / 4];
            let destination = dir.join(format!("{name}-{rate}.{}", container.extension()));
            let started = pm_export::export(ExportSpec {
                width: 64,
                height: 36,
                rate: Rate::new(25, 1).unwrap(),
                frames,
                codec: video,
                container,
                tags: ColorTags::Rec709QuickTime,
                alpha: AlphaOutput::None,
                working: WorkingEncoding::Standard,
                dither: Dither::Off,
                audio: Some(AudioSpec {
                    sample_rate: rate,
                    channels: 2,
                    codec,
                }),
                timecode: None,
                destination: destination.clone(),
                overwrite: Overwrite::Replace,
                mode: None,
            });
            if unsupported {
                let why = started.err().map(|e| e.to_string()).unwrap_or_default();
                eprintln!("MEASURE {name} at {rate} Hz: refused ({why})");
                assert!(why.contains("AAC audio is at most 48 kHz"), "{why}");
                continue;
            }
            let mut job = started.unwrap_or_else(|e| panic!("{name} at {rate} Hz: {e}"));
            let rgba = vec![0.25f32; 64 * 36 * 4];
            for _ in 0..frames {
                job.push_frame(&WorkingFrame {
                    width: 64,
                    height: 36,
                    rgba: &rgba,
                })
                .unwrap();
            }
            let interleaved: Vec<f32> = signal(rate, samples, &at)
                .into_iter()
                .flat_map(|v| [v, v])
                .collect();
            job.push_audio(&interleaved).unwrap();
            job.finish().unwrap();
            let what = format!("{name} at {rate} Hz");
            eprintln!("MEASURE {what}: {}", edit(&destination));
            let aac = codec == AudioCodec::Aac;
            check(
                &format!("{what}, FFmpeg"),
                &ffmpeg_decode(&destination),
                rate,
                &at,
                samples,
                slack(aac, true),
            );
            if let Some(got) = apple_decode(&destination, &dir) {
                let what = format!("{what}, Apple");
                check(&what, &got, rate, &at, samples, slack(aac, false));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sound over full scale is counted and reported: an integer PCM or AAC
/// track clips it, a 32-bit float track keeps it, and the person exporting
/// is told how much and which.
#[test]
fn sound_over_full_scale_is_counted_and_reported() {
    let dir = scratch("overs");
    let rate = 48_000u32;
    let dur = 1.0;
    // A 440 Hz tone at 0.8 of full scale.
    let tone: Vec<f32> = (0..rate)
        .map(|i| (0.8 * (std::f64::consts::TAU * 440.0 * f64::from(i) / 48_000.0).sin()) as f32)
        .collect();
    let clip = dir.join("tone.wav");
    write_wav(&clip, rate, &tone);
    use pm_protocol::priority::{AudioCoding, DeliveryOptions};
    for (gain, loud, coding) in [
        (1.0, false, None),
        (2.0, true, None),
        (2.0, true, Some(AudioCoding::Pcm16)),
        (2.0, true, Some(AudioCoding::PcmFloat)),
    ] {
        let float = coding == Some(AudioCoding::PcmFloat);
        let mut layer = audio("sound", "tone", 0.0, dur);
        layer["d"]["gain"] = json!(gain);
        let mut doc = project(64, 36, dur, solid_fill("#14161C"), vec![layer]);
        doc["fps"] = json!(25);
        asset(&mut doc, "tone", "audio", "wav", 0, 0, dur);
        let mut media = MediaPaths::default();
        media.insert("tone", clip.clone());
        let destination = dir.join(format!("gain-{gain}-{coding:?}.mov"));
        let job = ExportJob {
            document: Arc::new(load(&doc)),
            composition: "Ptest-root".into(),
            settings: ExportSettings {
                format: ExportFormat::ProRes {
                    profile: WireProRes::Hq,
                },
                alpha: WireAlpha::None,
                quality: QualityTier::Draft,
                size: Size::new(64, 36),
                frame_rate: FrameRate::new(25, 1),
                range: TickRange {
                    start: Ticks::ZERO,
                    end: Ticks::new(TICKS_PER_SECOND).unwrap(),
                },
                audio: true,
                audio_index_playhead: None,
                motion_blur: false,
                color: OutputColor::Rec709QuickTime {
                    candidate: QuickTimeTag::Cicp1_13_1,
                },
                precision: Precision::Float16,
                dither: false,
                draft_from_proxies: false,
                delivery: DeliveryOptions {
                    audio_coding: coding,
                    ..Default::default()
                },
            },
            media: Arc::new(media),
            destination: destination.clone(),
            overwrite: Overwrite::Fail,
            mode: None,
            seed: 1,
        };
        let mut renderer = Renderer::headless().unwrap();
        let mut last = None;
        let delivered = run_export(
            &mut renderer,
            &job,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |p| last = p.mix,
        )
        .unwrap();
        let mix = delivered.report.mix.expect("the mix's levels");
        let decoded = ffmpeg_decode(&destination);
        let ceiling = decoded.iter().fold(0f32, |m, v| m.max(v.abs()));
        eprintln!(
            "MEASURE tone at 0.8 with gain {gain} as {coding:?}: mix peak {:.3} ({:+.1} dB), {} samples over full scale; the file's own peak {ceiling:.3}; said to the person: {:?}",
            mix.peak,
            mix.peak_db().unwrap(),
            mix.overs,
            mix.warning()
        );
        assert_eq!(last, Some(mix), "the last progress carries the whole mix");
        assert!(
            (f64::from(mix.peak) - 0.8 * gain).abs() < 0.01,
            "{}",
            mix.peak
        );
        assert_eq!(mix.overs > 0, loud);
        assert_eq!(mix.warning().is_some(), loud);
        assert_eq!(mix.kept, float);
        if float {
            // The float track holds what is over: nothing was clipped.
            assert!((f64::from(ceiling) - 0.8 * gain).abs() < 0.01, "{ceiling}");
            let said = mix.warning().unwrap();
            assert!(
                said.starts_with("Audio over full scale: ")
                    && said.ends_with("peak +4.1 dB (kept in the 32-bit float track).")
                    && !said.contains("clipped"),
                "{said}"
            );
        } else {
            // An integer track cannot hold what is over: it is clipped there.
            assert!(ceiling <= 1.0 && (ceiling > 0.99) == loud, "{ceiling}");
            if loud {
                let said = mix.warning().unwrap();
                assert!(
                    said.starts_with("Audio clipped: ") && said.ends_with("peak +4.1 dB."),
                    "{said}"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
