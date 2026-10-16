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

/// HE-AAC (AAC with SBR) from macOS's `afconvert`, in M4A and remuxed to
/// Matroska; our LGPL FFmpeg build has no HE-AAC encoder. `None` where
/// `afconvert` is missing (not macOS).
fn he_aac(stem: &str, source: &str) -> Option<(PathBuf, PathBuf)> {
    let files = afconvert_aac(stem, source, "aach", "64000", &["mkv"])?;
    Some((files[0].clone(), files[1].clone()))
}

/// `source` encoded by `afconvert -f m4af -d <format> -b <bitrate>` (`aach`
/// HE-AAC, `aacp` HE-AAC v2), then the M4A remuxed with `-c copy` to each
/// of `remux` (extensions). Returns the M4A first. `None` where `afconvert`
/// is missing (not macOS).
fn afconvert_aac(
    stem: &str,
    source: &str,
    format: &str,
    bitrate: &str,
    remux: &[&str],
) -> Option<Vec<PathBuf>> {
    let wav = generated(&format!("{stem}.wav"), source, &["-c:a", "pcm_s16le"]);
    let m4a = wav.with_extension("m4a");
    if !m4a.exists() {
        let status = Command::new("afconvert")
            .args(["-f", "m4af", "-d", format, "-b", bitrate])
            .arg(&wav)
            .arg(&m4a)
            .status()
            .ok()?;
        assert!(status.success(), "afconvert could not encode {format}");
    }
    let mut out = vec![m4a.clone()];
    for ext in remux {
        let file = wav.with_extension(ext);
        if !file.exists() {
            let status = Command::new(ffmpeg())
                .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(&m4a)
                .args(["-c", "copy"])
                .arg(&file)
                .status()
                .expect("run ffmpeg");
            assert!(status.success(), "ffmpeg could not remux {format} to {ext}");
        }
        out.push(file);
    }
    Some(out)
}

/// The largest difference between `out` and `want` from sample `at` on, over
/// every channel.
fn max_diff(out: &[Vec<f32>], want: &[Vec<f32>], at: usize) -> f32 {
    out.iter().zip(want).fold(0f32, |m, (o, w)| {
        o.iter()
            .zip(&w[at..])
            .fold(m, |m, (a, b)| m.max((a - b).abs()))
    })
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

/// Adds `delta` (90 kHz ticks) to the PTS (and DTS) of every PES packet of
/// an MPEG-TS file whose PTS is at least `from`, so the stream's timestamps
/// jump forward (a gap) or back (an overlap) there. Returns the packets
/// moved.
fn shift_ts_timestamps(src: &Path, dst: &Path, from: i64, delta: i64) -> usize {
    fn read(b: &[u8], o: usize) -> i64 {
        (((b[o] >> 1) & 7) as i64) << 30
            | (b[o + 1] as i64) << 22
            | ((b[o + 2] >> 1) as i64) << 15
            | (b[o + 3] as i64) << 7
            | (b[o + 4] >> 1) as i64
    }
    fn write(b: &mut [u8], o: usize, v: i64) {
        b[o] = (b[o] & 0xF1) | ((v >> 29) & 0x0E) as u8;
        b[o + 1] = (v >> 22) as u8;
        b[o + 2] = ((v >> 14) & 0xFE) as u8 | 1;
        b[o + 3] = (v >> 7) as u8;
        b[o + 4] = ((v << 1) & 0xFE) as u8 | 1;
    }
    let mut b = std::fs::read(src).unwrap();
    let mut moved = 0;
    for i in (0..b.len()).step_by(188) {
        // Sync byte, payload unit start.
        if b[i] != 0x47 || b[i + 1] & 0x40 == 0 {
            continue;
        }
        let control = (b[i + 3] >> 4) & 3;
        if control & 1 == 0 {
            continue;
        }
        let q = i
            + 4
            + if control & 2 != 0 {
                1 + b[i + 4] as usize
            } else {
                0
            };
        let id = b[q + 3];
        if b[q..q + 3] != [0, 0, 1] || !(id == 0xBD || (0xC0..=0xEF).contains(&id)) {
            continue;
        }
        let flags = b[q + 7] >> 6;
        if flags & 2 == 0 {
            continue;
        }
        let pts = read(&b, q + 9);
        if pts >= from {
            write(&mut b, q + 9, pts + delta);
            if flags == 3 {
                let dts = read(&b, q + 14);
                write(&mut b, q + 14, dts + delta);
            }
            moved += 1;
        }
    }
    std::fs::write(dst, b).unwrap();
    moved
}

/// Timestamps that jump inside a stream with a sample-exact timebase
/// (MPEG-TS, 90 kHz, here SMPTE 302M 24-bit PCM carrying a ramp whose value
/// is its sample index). A jump forward is a gap: silence, then the media
/// at its timestamp. A jump back is an overlap: the samples before what
/// was already returned are dropped (beyond a one-sample tolerance), and
/// the rest sits at its timestamp. Either way every sample after the jump
/// is where its timestamp puts it, so a sequential decode and a seek into
/// the stream after the jump agree.
#[test]
fn gaps_are_silence_and_overlaps_are_dropped() {
    let ramp = generated(
        "ramp302.ts",
        "aevalsrc=exprs='n/8388608|n/8388608':s=48000:d=4",
        &["-c:a", "s302m", "-strict", "-2"],
    );
    let dir = ramp.parent().unwrap().to_owned();
    // The source: program sample n holds n / 2^23 (exact in 24 bits and f32).
    let (d, pcm) = decode_all(&ramp);
    assert!(d.info().sample_exact_timestamps);
    let index = |v: f32| (v as f64 * 8_388_608.0).round() as i64;
    assert!(
        pcm[0][..48_000]
            .iter()
            .enumerate()
            .all(|(i, v)| index(*v) == i as i64)
    );
    // Packets are 1,024 samples (1,920 ticks); move every packet from the
    // 51st on, which starts at program sample 51,200.
    let start_pts = 126_000; // the muxer's default 1.4 s offset
    let at = 50 * 1_024i64;
    for (name, shift) in [("gap.ts", 1_000i64), ("overlap.ts", -600)] {
        let file = dir.join(name);
        let moved = shift_ts_timestamps(
            &ramp,
            &file,
            start_pts + 50 * 1_920,
            shift * 90_000 / 48_000,
        );
        assert!(moved > 100, "{name}: {moved} packets moved");
        let (_, pcm) = decode_all(&file);
        let got = &pcm[0];
        // Before the jump: the source, unchanged.
        assert!(
            (0..at).all(|i| index(got[i as usize]) == i),
            "{name}: before the jump"
        );
        // After it: sample k is source sample k − shift (a gap of silence
        // first; an overlap's first 600 samples dropped).
        let from = at + shift.max(0);
        if shift > 0 {
            assert!(
                got[at as usize..from as usize].iter().all(|v| *v == 0.0),
                "{name}: the gap is silence"
            );
        }
        let wrong = (from..from + 100_000)
            .filter(|&k| index(got[k as usize]) != k - shift)
            .count();
        assert_eq!(
            wrong, 0,
            "{name}: samples after the jump off their timestamps"
        );
        // A seek past the jump lands on the same sample as the sequential
        // decode.
        let mut s = AudioDecoder::open(&file, None).unwrap();
        for target in [from as u64 + 5_000, 150_000] {
            s.seek_anchored(target).unwrap();
            let mut out = vec![Vec::new(); 2];
            s.read(&mut out, 4_096).unwrap();
            assert_eq!(
                index(out[0][0]),
                target as i64 - shift,
                "{name}: seek to {target}"
            );
            assert_eq!(&out[0][..], &got[target as usize..target as usize + 4_096]);
        }
    }
}

/// Matroska and WebM (1 ms timebases) with every codec they commonly carry:
/// a seek starts at a packet the demux-only scan placed and counts from
/// there, so it lands on the samples of the sequential decode (exactly for
/// lossless codecs, converged within the pre-roll for lossy ones) and costs
/// the same wherever it lands: the packets it decodes do not grow with the
/// seek distance. ALAC in Matroska has no parser and no fixed frame size:
/// its seeks decode from the start, and reads in order continue the running
/// decode (each packet decoded once). HE-AAC's codec delay (4,224 samples)
/// is longer than its 2,048-sample packets: a fresh decoder's first frame
/// is the third packet's, and placing it as the anchor's put every seek one
/// frame early.
#[test]
fn coarse_timebase_seeks_start_near_their_target() {
    let source = "aevalsrc=exprs='0.3*sin(2*PI*440*t)+0.2*sin(2*PI*3001*t)|0.3*sin(2*PI*660*t)':s=48000:d=40";
    let mut files = vec![];
    for (name, args, lossless) in [
        ("c-opus.webm", &["-c:a", "libopus"][..], false),
        (
            "c-vorbis.mkv",
            &["-c:a", "vorbis", "-strict", "-2"][..],
            false,
        ),
        ("c-aac.mkv", &["-c:a", "aac"][..], false),
        ("c-aac44.mkv", &["-c:a", "aac", "-ar", "44100"][..], false),
        (
            "c-nopns-aac44.mkv",
            &["-c:a", "aac", "-aac_pns", "0", "-ar", "44100"][..],
            false,
        ),
        ("c-ac3.mkv", &["-c:a", "ac3"][..], false),
        ("c-eac3.mkv", &["-c:a", "eac3"][..], false),
        ("c-mp2.mkv", &["-c:a", "mp2"][..], false),
        ("c-flac.mkv", &["-c:a", "flac"][..], true),
        ("c-pcm.mkv", &["-c:a", "pcm_s24le"][..], true),
        ("c-alac.mkv", &["-c:a", "alac"][..], true),
    ] {
        files.push((name.to_string(), generated(name, source, args), lossless));
    }
    match he_aac("c-aac-he", source) {
        Some((m4a, mkv)) => {
            files.push(("c-aac-he.m4a".into(), m4a, false));
            files.push(("c-aac-he.mkv".into(), mkv, false));
        }
        None => eprintln!("afconvert is missing: HE-AAC not covered"),
    }
    // HE-AAC v2: a fresh decoder has no stereo until a Parametric Stereo
    // header, which Apple's encoder repeats about every 1.8 s.
    match afconvert_aac("c-aac-ps", source, "aacp", "32000", &["mkv"]) {
        Some(ps) => {
            files.push(("c-aac-ps.m4a".into(), ps[0].clone(), false));
            files.push(("c-aac-ps.mkv".into(), ps[1].clone(), false));
        }
        None => eprintln!("afconvert is missing: HE-AAC v2 not covered"),
    }
    for (name, file, lossless) in files {
        let name = name.as_str();
        let (d, pcm) = decode_all(&file);
        assert!(!d.info().sample_exact_timestamps, "{name}");
        let total = pcm[0].len() as u64;
        let rate = d.info().sample_rate as u64;
        let preroll = d.info().preroll;
        let mut s = AudioDecoder::open(&file, None).unwrap();
        let mut worst = 0f32;
        let mut worst_packets = 0;
        let mut costs = vec![];
        let targets = [
            30 * rate + 7,
            preroll + 1,
            5_000,
            rate + 123,
            10 * rate + 4_321,
            total - 3_000,
            20 * rate + 1,
        ];
        for &t in &targets {
            let before = s.packets_decoded();
            s.seek_anchored(t).unwrap();
            let mut out = vec![Vec::new(); pcm.len()];
            let n = s.read(&mut out, 2_048).unwrap();
            assert_eq!(n as u64, 2_048.min(total - t), "{name}: read at {t}");
            if lossless {
                for (o, p) in out.iter().zip(&pcm) {
                    assert_eq!(
                        &o[..],
                        &p[t as usize..t as usize + n],
                        "{name}: seek to {t}"
                    );
                }
            }
            worst = worst.max(max_diff(&out, &pcm, t as usize));
            let cost = s.packets_decoded() - before;
            costs.push(cost);
            if t > 2 * rate {
                worst_packets = worst_packets.max(cost);
            }
        }
        // Lossy decoders converge within the pre-roll; an off-by-a-frame
        // placement would differ by the signal's full swing (about 1), and
        // HE-AAC v2 from a decoder that has not seen a Parametric Stereo
        // header yet by the signal's level (0.3). AAC's noise substitution
        // is random by design (its generator runs across frames): up to
        // 7.2e-3 where the tone stops at the end. The same AAC encoded
        // without it seeks onto the sequential decode exactly.
        let bound = if name.starts_with("c-aac") {
            1e-2
        } else {
            1e-4
        };
        assert!(worst < bound, "{name}: seek differs by {worst}");
        if name != "c-alac.mkv" {
            // Pre-roll, anchor spacing, the 2,048 samples read, and the
            // codec's frame: a few dozen packets, never the 30 s before
            // (HE-AAC v2's 4 s pre-roll is 94 packets of 2,048 samples).
            let packet_bound = 60 + preroll / 2_048;
            assert!(
                worst_packets <= packet_bound,
                "{name}: a seek decoded {worst_packets} packets ({costs:?})"
            );
            // Blocks read in order (as the source cache decodes them, each
            // from its own anchor) cost the stream once plus a pre-roll and
            // an anchor spacing per block: linear in the length. Read in
            // reverse, every block starts a fresh decoder far from the last
            // one; either way a block is the sequential decode's.
            let block = 65_536u64;
            let blocks = total.div_ceil(block);
            for reverse in [false, true] {
                let mut s = AudioDecoder::open(&file, None).unwrap();
                for k in 0..blocks {
                    let k = if reverse { blocks - 1 - k } else { k };
                    s.seek_anchored(k * block).unwrap();
                    let mut out = vec![Vec::new(); pcm.len()];
                    s.read(&mut out, block as usize).unwrap();
                    let diff = max_diff(&out, &pcm, (k * block) as usize);
                    assert!(
                        diff < bound,
                        "{name}: block {k} differs by {diff} (reverse {reverse})"
                    );
                }
                if !reverse {
                    let once = d.packets_decoded();
                    let packets = s.packets_decoded();
                    assert!(
                        packets <= once + blocks * (45 + preroll / 2_048),
                        "{name}: {blocks} blocks in order decoded {packets} packets (the stream has {once})"
                    );
                }
            }
        } else {
            // Reads in order continue the running decode.
            let mut s = AudioDecoder::open(&file, None).unwrap();
            for k in 0..(total / 65_536) {
                s.seek_anchored(k * 65_536).unwrap();
                let mut out = vec![Vec::new(); pcm.len()];
                s.read(&mut out, 65_536).unwrap();
            }
            let packets = s.packets_decoded();
            assert!(
                packets <= total / 4_096 + 4,
                "{name}: in-order reads decoded {packets} packets"
            );
        }
        eprintln!("{name}: seeks within {worst:e} of the sequential decode; packets {costs:?}");
    }
}

/// HE-AAC v2 (AAC with SBR and Parametric Stereo) from `afconvert -d aacp`
/// at 32, 44.1 and 48 kHz, in M4A and Matroska (coarse timebases, seeks from
/// scanned anchors) and in MP4 and ADTS (placed by timestamps): every
/// 65,536-sample block read in reverse, each from a fresh decoder, matches
/// the sequential decode on both channels. A fresh decoder applies no
/// stereo until a frame carries a PS header (Apple's encoder repeats it
/// about every 1.8 s), so with AAC's 4,096-sample pre-roll almost every
/// block differed by about 0.3 (the signal's level) for its first 1.2 to
/// 1.9 s.
#[test]
fn he_aac_v2_blocks_match_the_sequential_decode() {
    let mut covered = 0;
    for rate in [32_000, 44_100, 48_000] {
        let source =
            format!("aevalsrc=exprs='0.3*sin(2*PI*440*t)|0.3*sin(2*PI*660*t)':s={rate}:d=40");
        let Some(files) = afconvert_aac(
            &format!("ps-{rate}"),
            &source,
            "aacp",
            "32000",
            &["mkv", "mp4", "aac"],
        ) else {
            eprintln!("afconvert is missing: HE-AAC v2 not covered");
            return;
        };
        for file in files {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let (d, pcm) = decode_all(&file);
            let info = d.info();
            assert_eq!(info.sample_rate, rate, "{name}");
            assert_eq!(info.channels(), 2, "{name}");
            assert!(info.parametric_stereo, "{name}: PS not detected");
            assert_eq!(info.preroll, 4 * rate as u64, "{name}");
            let total = pcm[0].len() as u64;
            let block = 65_536u64;
            let blocks = total.div_ceil(block);
            let mut s = AudioDecoder::open(&file, None).unwrap();
            let mut worst = 0f32;
            let mut worst_cost = 0;
            for k in (0..blocks).rev() {
                let before = s.packets_decoded();
                s.seek_anchored(k * block).unwrap();
                let mut out = vec![Vec::new(); 2];
                s.read(&mut out, block as usize).unwrap();
                assert_eq!(out[0].len() as u64, block.min(total - k * block), "{name}");
                let diff = max_diff(&out, &pcm, (k * block) as usize);
                assert!(diff < 1e-3, "{name}: block {k} differs by {diff}");
                worst = worst.max(diff);
                worst_cost = worst_cost.max(s.packets_decoded() - before);
            }
            // The 4 s pre-roll, an anchor spacing and the block: 2,048
            // samples a packet.
            let bound = (4 * rate as u64 + 4_096 + block) / 2_048 + 8;
            assert!(
                worst_cost <= bound,
                "{name}: a block decoded {worst_cost} packets"
            );
            eprintln!(
                "{name}: {blocks} blocks in reverse within {worst:e} of the sequential decode, \
                 at most {worst_cost} packets each"
            );
            covered += 1;
        }
    }
    assert_eq!(covered, 12);
    // HE-AAC v1 keeps AAC's short pre-roll.
    let source = "aevalsrc=exprs='0.3*sin(2*PI*440*t)|0.3*sin(2*PI*660*t)':s=48000:d=5";
    if let Some((m4a, _)) = he_aac("he-v1-short", source) {
        let d = AudioDecoder::open(&m4a, None).unwrap();
        assert!(!d.info().parametric_stereo);
        assert_eq!(d.info().preroll, 4_096);
    }
}

/// HE-AAC's codec delay (4,224 samples) is longer than AAC's pre-roll
/// (4,096): a seek that started at the last anchor at least the pre-roll
/// before its target put its first sample after the target for 128 of every
/// 4,096 targets, and every such seek decoded the stream from the start
/// (and, before, stopped anchored seeks on the stream for good). Every
/// target across two anchor spacings now starts from its anchor.
#[test]
fn he_aac_seeks_inside_the_codec_delay_window() {
    let source = "aevalsrc=exprs='0.3*sin(2*PI*440*t)+0.2*sin(2*PI*3001*t)|0.3*sin(2*PI*660*t)':s=48000:d=40";
    let Some((m4a, mkv)) = he_aac("c-aac-he", source) else {
        eprintln!("afconvert is missing: HE-AAC not covered");
        return;
    };
    for file in [m4a, mkv] {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let (_, pcm) = decode_all(&file);
        let mut s = AudioDecoder::open(&file, None).unwrap();
        let mut worst = 0f32;
        let mut worst_cost = 0;
        let mut seeks = 0;
        for t in (958_464u64..958_464 + 8_192).step_by(7) {
            let before = s.packets_decoded();
            s.seek_anchored(t).unwrap();
            let mut out = vec![Vec::new(); 2];
            s.read(&mut out, 256).unwrap();
            worst = worst.max(max_diff(&out, &pcm, t as usize));
            let cost = s.packets_decoded() - before;
            assert!(cost <= 12, "{name}: a seek to {t} decoded {cost} packets");
            worst_cost = worst_cost.max(cost);
            seeks += 1;
        }
        assert!(worst < 1e-2, "{name}: seeks differ by {worst}");
        let before = s.packets_decoded();
        s.seek_anchored(1_440_000).unwrap();
        let mut out = vec![Vec::new(); 2];
        s.read(&mut out, 256).unwrap();
        let cost = s.packets_decoded() - before;
        assert!(cost <= 12, "{name}: a later seek decoded {cost} packets");
        eprintln!(
            "{name}: {seeks} seeks within {worst:e}, at most {worst_cost} packets each; \
             the next seek {cost}"
        );
    }
}
