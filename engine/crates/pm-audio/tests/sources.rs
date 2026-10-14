//! Sources without a length limit, the export render to PCM files, and
//! waveform data.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use common::*;
use pm_audio::offline::{self, PcmFormat, WavWriter, read_wav, sample_range};
use pm_audio::source::BLOCK;
use pm_audio::waveform::{Waveform, legacy_peaks};
use pm_audio::{MixFormat, SourceCache};
use pm_protocol::common::TICKS_PER_SECOND;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures")
}

/// Forty minutes of AAC, far past the legacy decode budget (128 MiB, about
/// 5.8 minutes of stereo float): a render near the end lands on the right
/// samples and the cache stays within its budget.
#[test]
fn long_sources_stream_without_a_limit() {
    let dir = temp_dir("long");
    let file = dir.join("long.m4a");
    if !file.exists() {
        let status = Command::new(ffmpeg())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
            ])
            .arg("aevalsrc=0.125*sin(2*PI*250*t):s=48000:d=2400")
            .args(["-ac", "2", "-c:a", "aac", "-b:a", "64k"])
            .arg(&file)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let budget = 16 << 20;
    let cache = SourceCache::new(budget, 0);
    let doc = project(
        30.0,
        2400.0,
        &[audio("A", "l", 0.0, 2400.0, "", "")],
        &[],
        "",
    );
    let mut rig = Rig::new(&[("l", &file)]);
    rig.cache = cache.clone();
    let mut m = rig.mixer(doc, "C", MixFormat::stereo(48_000));
    let source = m.graph().voices[0].source.clone();
    assert_eq!(source.length(), Some(48_000 * 2400));
    // 39 min 59 s, against FFmpeg's own accurate seek and decode of the file.
    let start = 48_000i64 * 2399;
    let out = rig.render(&mut m, start, start + 4800, 1024);
    let raw = dir.join("region.f32");
    let status = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&file)
        .args([
            "-ss",
            "2399",
            "-t",
            "0.1",
            "-af",
            "pan=mono|c0=c0",
            "-f",
            "f32le",
        ])
        .arg(&raw)
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&raw).unwrap();
    let reference: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(reference.len(), 4800);
    let mut best = (f64::MAX, 0i64);
    for shift in -32i64..=32 {
        let mut e = 0f64;
        for (i, &v) in out[0].iter().enumerate().take(4700).skip(64) {
            let j = (i as i64 + shift) as usize;
            e = e.max((v as f64 - reference[j] as f64).abs());
        }
        if e < best.0 {
            best = (e, shift);
        }
    }
    let worst = best.0;
    assert_eq!(
        best.1, 0,
        "the render is {} samples off forty minutes in",
        best.1
    );
    assert!(worst < 1e-3, "worst difference {worst} forty minutes in");
    // Hop around the file.
    for t in [1200i64, 10, 2300, 600] {
        let s = 48_000 * t;
        rig.render(&mut m, s, s + 2048, 2048);
    }
    let stats = cache.stats();
    assert!(
        stats.bytes <= budget + (1 << 20),
        "cache holds {} bytes",
        stats.bytes
    );
    eprintln!("40 min AAC: max |render − ffmpeg decode| {worst:e} at 39:59; cache {stats:?}");
}

/// Ten minutes of WebM/Opus and of Matroska/FLAC: 1 ms timebases, so the
/// decoder positions them by counting samples rather than by timestamps.
/// Each block costs the same wherever it sits in the file, read in order
/// (playback, export) or cold (a seek near the end): decoding them is
/// linear in the length, not quadratic. A cold block equals the block read
/// in order.
#[test]
fn long_coarse_timebase_sources_decode_in_linear_time() {
    let dir = temp_dir("coarse");
    let tone = "aevalsrc=exprs='0.3*sin(2*PI*440*t)+0.1*sin(2*PI*5003*t)|0.3*sin(2*PI*660*t)':s=48000:d=600";
    for (name, args, bound) in [
        (
            "long.webm",
            &["-c:a", "libopus", "-b:a", "128k"][..],
            1e-4f32,
        ),
        ("long.mkv", &["-c:a", "flac"][..], 0.0),
    ] {
        let file = dir.join(name);
        generate(&file, tone, args);
        let cache = SourceCache::new(8 << 20, 0);
        let source = cache.open(&file, None).unwrap();
        let blocks = (600 * 48_000u64).div_ceil(BLOCK as u64);
        let mut packets = Vec::new();
        let mut micros = Vec::new();
        let mut kept = Vec::new();
        for k in 0..blocks {
            let before = source.packets_decoded();
            let t = Instant::now();
            let b = cache.block(&source, k, true).unwrap().unwrap();
            micros.push(t.elapsed().as_micros() as u64);
            packets.push(source.packets_decoded() - before);
            if k == 5 || k == blocks - 3 {
                kept.push((k, b));
            }
        }
        // The first block has no pre-roll; every other costs the block, the
        // pre-roll, and at most an anchor spacing (Opus: about 69 + 25 + 5
        // packets), the same near the end as near the start.
        let first = packets[1];
        let worst = *packets[1..].iter().max().unwrap();
        let flat = first + first / 10 + 2;
        assert!(
            worst <= flat,
            "{name}: blocks in order cost {first} to {worst} packets"
        );
        let cold = SourceCache::new(8 << 20, 0);
        let cold_source = cold.open(&file, None).unwrap();
        for (k, b) in &kept {
            let before = cold_source.packets_decoded();
            let t = Instant::now();
            let c = cold.block(&cold_source, *k, true).unwrap().unwrap();
            let us = t.elapsed().as_micros();
            let cost = cold_source.packets_decoded() - before;
            assert!(
                cost <= flat,
                "{name}: cold block {k} decoded {cost} packets"
            );
            for (x, y) in c.channels.iter().zip(&b.channels) {
                let diff = x.iter().zip(y).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
                assert!(diff <= bound, "{name}: cold block {k} differs by {diff}");
            }
            eprintln!("{name}: cold block {k} decoded {cost} packets in {us} µs");
        }
        let at = |k: usize| micros[k.min(micros.len() - 1)];
        eprintln!(
            "{name}: {blocks} blocks in order, {first} to {worst} packets each; \
             block 0 {} µs, 100 {} µs, 200 {} µs, 400 {} µs; total {} ms",
            at(0),
            at(100),
            at(200),
            at(400),
            micros.iter().sum::<u64>() / 1000
        );
    }
}

/// The export path: a tick range renders to `round(duration × sr)` frames
/// as 24-bit, float, and dithered 16-bit WAV.
#[test]
fn export_renders_pcm_files() {
    let dir = temp_dir("export");
    let file = dir.join("tone.wav");
    write_wav(
        &file,
        48_000,
        &[
            tone(48_000, 3.0, 440.0, 0.5),
            tone(48_000, 3.0, 660.0, 0.25),
        ],
    );
    let doc = project(
        29.97,
        3.0,
        &[audio("A", "t", 0.0, 3.0, r#","fadeIn":0.1"#, "")],
        &[],
        "",
    );
    let rig = Rig::new(&[("t", &file)]);
    let end = pm_time::Rate::FPS_29_97.frame_start(75).unwrap();
    let (a, b) = sample_range(0, end, 48_000);
    let expected_frames = (end as f64 / TICKS_PER_SECOND as f64 * 48_000.0).round() as u64;
    let mut reference = None;
    for format in [PcmFormat::F32, PcmFormat::S24, PcmFormat::S16] {
        let mut m = rig.mixer(doc.clone(), "C", MixFormat::stereo(48_000));
        let mut w = WavWriter::new(std::io::Cursor::new(Vec::new()), format, 2, 48_000).unwrap();
        let summary =
            offline::render(&mut m, a, b, 4096, |first, inter| w.write(first, inter)).unwrap();
        let (cur, frames, clipped) = w.finish().unwrap();
        assert_eq!(frames, expected_frames);
        assert_eq!(summary.frames, expected_frames);
        assert_eq!(clipped, 0);
        let (f, ch, rate, data) = read_wav(&cur.into_inner()).unwrap();
        assert_eq!(
            (f, ch, rate, data.len() as u64),
            (format, 2, 48_000, expected_frames * 2)
        );
        match format {
            PcmFormat::F32 => reference = Some(data),
            _ => {
                let r = reference.as_ref().unwrap();
                let lsb = if format == PcmFormat::S24 {
                    1.0 / 8_388_608.0
                } else {
                    1.0 / 32_768.0
                };
                let worst = r
                    .iter()
                    .zip(&data)
                    .map(|(x, y)| (x - y).abs())
                    .fold(0f32, f32::max);
                // 24-bit rounds to half an LSB; 16-bit adds ±1 LSB of TPDF dither.
                let bound = if format == PcmFormat::S24 {
                    0.5 * lsb + 1e-9
                } else {
                    1.5 * lsb + 1e-9
                };
                assert!(worst <= bound, "{format:?}: {worst} vs {bound}");
            }
        }
    }
}

#[test]
fn waveforms() {
    // The legacy envelope: 100 buckets a second, max |sample| over channels.
    let peaks = legacy_peaks(&fixtures().join("tone.wav")).unwrap();
    assert_eq!(peaks.len(), 150);
    assert!(
        peaks.iter().all(|&p| (p - 0.25).abs() < 0.01),
        "{:?}",
        &peaks[..4]
    );
    let peaks = legacy_peaks(&fixtures().join("tone.m4a")).unwrap();
    assert_eq!(peaks.len(), 200);
    // The pyramid: min and max per channel; each level 4× coarser.
    let w = Waveform::compute(&fixtures().join("tone.flac"), 256).unwrap();
    assert_eq!(w.frames, 96_000);
    assert_eq!(w.channels, 2);
    assert_eq!(w.levels[0].data.len(), 96_000usize.div_ceil(256) * 2);
    assert!(w.levels.len() >= 3);
    for l in &w.levels {
        for e in &l.data {
            assert!(e[0] >= -0.26 && e[1] <= 0.26 && e[0] <= e[1]);
        }
    }
    assert_eq!(w.level_for(1000).samples_per_bucket, 1024);
    let _ = Arc::new(());
}
