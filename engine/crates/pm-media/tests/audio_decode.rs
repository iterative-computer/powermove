//! Streaming audio decode (`pm_media::audio`): priming, program length,
//! channel layouts, and sample-exact seeks on the e2e fixtures and on files
//! generated with Powermove's FFmpeg build.

use pm_media::audio::{AudioDecoder, Speaker};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures")
}

fn ffmpeg() -> PathBuf {
    let dir = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        });
    dir.join("bin/ffmpeg")
}

/// Generates a file with `ffmpeg -f lavfi -i <source> <args> <out>` once per test run.
fn generated(name: &str, source: &str, args: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-media-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join(name);
    if !out.exists() {
        let status = Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                source,
            ])
            .args(args)
            .arg(&out)
            .status()
            .expect("run ffmpeg");
        assert!(status.success(), "ffmpeg could not generate {name}");
    }
    out
}

fn decode_all(path: &Path) -> (AudioDecoder, Vec<Vec<f32>>) {
    let mut d = AudioDecoder::open(path, None).unwrap();
    let mut out = vec![Vec::new(); d.info().channels()];
    loop {
        let n = d.read(&mut out, 4096).unwrap();
        if n == 0 {
            break;
        }
    }
    (d, out)
}

/// The fixtures are 440 Hz sines at 48 kHz (`gen.sh`: `sine=frequency=440,volume=2`,
/// amplitude 0.25), so program sample 0 is the first sample of the sine.
#[test]
fn fixtures_start_on_the_first_program_sample() {
    for (file, tolerance, length) in [
        ("tone.wav", 1e-4, 72_000u64),
        ("tone.flac", 1e-4, 96_000),
        ("tone.m4a", 0.02, 96_000),
        ("tone.mp3", 0.02, 96_000),
        ("tone.opus", 0.02, 96_000),
        ("h264-aac.mp4", 0.02, 96_000),
    ] {
        let (d, pcm) = decode_all(&fixtures().join(file));
        let info = d.info().clone();
        assert_eq!(info.sample_rate, 48_000, "{file}");
        assert_eq!(
            info.speakers,
            vec![Speaker::FrontLeft, Speaker::FrontRight],
            "{file}"
        );
        assert_eq!(info.length, Some(length), "{file}: metadata length");
        let decoded = pcm[0].len() as u64;
        assert!(
            decoded.abs_diff(length) <= 1,
            "{file}: decoded {decoded} samples, metadata says {length}"
        );
        // Correlate against the reference sine around the start (skipping the
        // codec's own start-up transient) to find the timeline offset.
        let reference =
            |n: i64| 0.25 * (2.0 * std::f64::consts::PI * 440.0 * n as f64 / 48_000.0).sin();
        let window = 2000..6000usize;
        let mut best = (f64::MAX, 0i64);
        for shift in -64i64..=64 {
            let err: f64 = window
                .clone()
                .map(|i| (pcm[0][i] as f64 - reference(i as i64 + shift)).powi(2))
                .sum::<f64>()
                / window.len() as f64;
            if err < best.0 {
                best = (err, shift);
            }
        }
        assert_eq!(
            best.1, 0,
            "{file}: the program starts {} samples late (priming {})",
            best.1, info.priming
        );
        assert!(
            best.0.sqrt() < tolerance,
            "{file}: rms error {} against the sine",
            best.0.sqrt()
        );
    }
}

#[test]
fn priming_is_reported() {
    let m4a = AudioDecoder::open(&fixtures().join("tone.m4a"), None).unwrap();
    assert_eq!(m4a.info().priming, 1024);
    let mp3 = AudioDecoder::open(&fixtures().join("tone.mp3"), None).unwrap();
    assert_eq!(mp3.info().priming, 1105);
    let opus = AudioDecoder::open(&fixtures().join("tone.opus"), None).unwrap();
    assert_eq!(opus.info().priming, 312);
    let wav = AudioDecoder::open(&fixtures().join("tone.wav"), None).unwrap();
    assert_eq!(wav.info().priming, 0);
}

/// Section 25 "Seeking" for audio: random access to sample N equals the
/// sequential decode of sample N, exactly for lossless codecs and PCM, and
/// for lossy codecs once the decoder state has converged within the pre-roll.
/// `None` as the tolerance: the codec substitutes random noise (AAC PNS), so
/// only the alignment is checked (the best match over ±64 samples is at 0).
#[test]
fn seeks_land_on_the_exact_sample() {
    let flac44 = generated(
        "noise-44k.flac",
        "anoisesrc=color=white:sample_rate=44100:amplitude=0.5:duration=20:seed=7",
        &["-ac", "2", "-c:a", "flac"],
    );
    let aac44 = generated(
        "noise-44k.m4a",
        "anoisesrc=color=pink:sample_rate=44100:amplitude=0.5:duration=20:seed=7",
        &["-ac", "2", "-c:a", "aac", "-b:a", "192k"],
    );
    let cases: Vec<(PathBuf, Option<f32>)> = vec![
        (fixtures().join("tone.wav"), Some(0.0)),
        (fixtures().join("tone.flac"), Some(0.0)),
        (flac44, Some(0.0)),
        (fixtures().join("tone.m4a"), Some(1e-4)),
        (fixtures().join("h264-aac.mp4"), Some(1e-4)),
        (aac44, None),
        (fixtures().join("tone.mp3"), Some(1e-4)),
        (fixtures().join("tone.opus"), Some(2e-3)),
    ];
    for (path, tolerance) in cases {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let (_, all) = decode_all(&path);
        let total = all[0].len() as u64;
        let mut d = AudioDecoder::open(&path, None).unwrap();
        let channels = d.info().channels();
        // Backwards, forwards, near the start, near the end, and a short hop.
        let mut targets = vec![
            total / 2,
            total / 7,
            1,
            0,
            total - 3000,
            total / 3,
            total / 3 + 5000,
            4321,
        ];
        targets.retain(|&t| t + 2048 <= total);
        let mut worst = 0f32;
        for t in targets {
            d.seek(t).unwrap();
            assert_eq!(d.position(), t);
            let mut out = vec![Vec::new(); channels];
            let n = d.read(&mut out, 2048).unwrap();
            assert_eq!(n, 2048, "{name}: short read at {t}");
            for c in 0..channels {
                for i in 0..n {
                    worst = worst.max((out[c][i] - all[c][t as usize + i]).abs());
                }
            }
            let mut best = (f64::MAX, 0i64);
            for shift in -64i64..=64 {
                let Some(start) = (t as i64 + shift)
                    .try_into()
                    .ok()
                    .filter(|s: &usize| s + n <= all[0].len())
                else {
                    continue;
                };
                let err: f64 = (0..n)
                    .map(|i| (out[0][i] as f64 - all[0][start + i] as f64).powi(2))
                    .sum();
                if err < best.0 {
                    best = (err, shift);
                }
            }
            assert_eq!(
                best.1, 0,
                "{name}: the seek to {t} is off by {} samples",
                best.1
            );
        }
        if let Some(tolerance) = tolerance {
            assert!(
                worst <= tolerance,
                "{name}: seek differs from the sequential decode by {worst}"
            );
        }
        eprintln!("{name}: seek vs sequential max |diff| {worst:e}");
    }
}

/// Every seek starts from a fresh decoder, so decoding the same target twice
/// gives the same samples whatever was decoded in between.
#[test]
fn seeks_are_deterministic() {
    let aac = generated(
        "noise-44k-det.m4a",
        "anoisesrc=color=pink:sample_rate=44100:amplitude=0.5:duration=8:seed=3",
        &["-ac", "2", "-c:a", "aac", "-b:a", "128k"],
    );
    let mut d = AudioDecoder::open(&aac, None).unwrap();
    let read_at = |d: &mut AudioDecoder, t: u64| {
        d.seek(t).unwrap();
        let mut out = vec![Vec::new(); 2];
        d.read(&mut out, 4096).unwrap();
        out
    };
    let first = read_at(&mut d, 150_000);
    let _ = read_at(&mut d, 20_000);
    let _ = read_at(&mut d, 300_000);
    assert_eq!(read_at(&mut d, 150_000), first);
}

#[test]
fn layouts_and_rates() {
    let mono = generated(
        "mono-22k.wav",
        "sine=frequency=1000:sample_rate=22050:duration=1",
        &["-c:a", "pcm_s24le"],
    );
    let d = AudioDecoder::open(&mono, None).unwrap();
    assert_eq!(d.info().sample_rate, 22_050);
    assert_eq!(d.info().speakers, vec![Speaker::FrontCenter]);
    assert_eq!(d.info().length, Some(22_050));
    let surround = generated(
        "surround.flac",
        "sine=frequency=500:sample_rate=48000:duration=1",
        &[
            "-af",
            "pan=5.1(side)|FL=c0|FR=c0|FC=c0|LFE=c0|SL=c0|SR=c0",
            "-c:a",
            "flac",
        ],
    );
    let d = AudioDecoder::open(&surround, None).unwrap();
    assert_eq!(
        d.info().speakers,
        vec![
            Speaker::FrontLeft,
            Speaker::FrontRight,
            Speaker::FrontCenter,
            Speaker::LowFrequency,
            Speaker::SideLeft,
            Speaker::SideRight
        ]
    );
    // A long file streams without holding it: read in chunks and count.
    let long = generated(
        "long.flac",
        "sine=frequency=100:sample_rate=48000:duration=600",
        &["-c:a", "flac"],
    );
    let mut d = AudioDecoder::open(&long, None).unwrap();
    let mut total = 0u64;
    let mut buf = vec![Vec::new(); 1];
    loop {
        buf[0].clear();
        let n = d.read(&mut buf, 1 << 16).unwrap();
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    assert_eq!(total, 48_000 * 600);
    assert_eq!(d.info().length, Some(48_000 * 600));
}

/// Section 13's audio formats decode to their full program length.
#[test]
fn container_formats() {
    for (name, args) in [
        ("t.aiff", &["-c:a", "pcm_s16be"][..]),
        ("t.caf", &["-c:a", "pcm_s24le"][..]),
        ("t-alac.m4a", &["-c:a", "alac"][..]),
        ("t.ogg", &["-c:a", "libopus"][..]),
        ("t-pcm.mov", &["-c:a", "pcm_s24le"][..]),
        ("t.ac3", &["-c:a", "ac3"][..]),
        ("t.mkv", &["-c:a", "flac"][..]),
    ] {
        let file = generated(
            name,
            "sine=frequency=500:sample_rate=48000:duration=3",
            args,
        );
        let (d, pcm) = decode_all(&file);
        let n = pcm[0].len() as i64;
        assert!(
            (n - 144_000).abs() <= 1536,
            "{name}: {n} samples ({:?})",
            d.info().length
        );
        assert!(
            d.info()
                .length
                .is_some_and(|l| (l as i64 - n).abs() <= 1536),
            "{name}: metadata {:?}, decoded {n}",
            d.info().length
        );
        eprintln!(
            "{name}: {n} samples, metadata {:?}, priming {}",
            d.info().length,
            d.info().priming
        );
    }
}

/// WebM's millisecond timebase: positioned by counting; the length comes
/// from the container and the Opus pre-skip is removed.
#[test]
fn webm_opus() {
    let (d, pcm) = decode_all(&fixtures().join("vp9-opus.webm"));
    let n = pcm[0].len() as i64;
    assert!(!d.info().sample_exact_timestamps);
    assert_eq!(d.info().priming, 312);
    let length = d.info().length.unwrap() as i64;
    eprintln!("vp9-opus.webm: decoded {n}, metadata {length}");
    assert!((n - length).abs() <= 960, "decoded {n}, metadata {length}");
    // A seek decodes forward from the start and lands on the same samples.
    let mut s = AudioDecoder::open(&fixtures().join("vp9-opus.webm"), None).unwrap();
    s.seek(30_000).unwrap();
    let mut out = vec![Vec::new(); 2];
    s.read(&mut out, 1000).unwrap();
    assert_eq!(&out[0][..], &pcm[0][30_000..31_000]);
}
