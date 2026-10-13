//! Offline mixes against today's export audio (`PM.Audio.renderOffline`,
//! captured by `tools/legacy-mix.ts` from the built legacy app).
//!
//! The capture lives outside the repository. The test reads the capture
//! named by `PM_AUDIO_PARITY` (a capture directory), or the newest `audio-*`
//! capture under `~/Library/Application Support/Powermove Engine Corpus/`,
//! and passes with a note when there is none.
//!
//! Bars (section 2 "Audio", section 25): a click at frame N lands at sample
//! floor(N × sr × den / num) ± 1, and the export has round(duration × sr)
//! samples. Residuals against the legacy mix are reported as measured; each
//! project states what may differ (docs/engine/pm-audio.md).

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_audio::offline::{self, read_wav};
use pm_audio::{MixFormat, MixGraph, Mixer, SourceCache};
use pm_eval::Evaluator;
use pm_model::Document;
use pm_model::json::{Value, parse};

fn capture_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PM_AUDIO_PARITY") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").ok()?;
    let root = Path::new(&home).join("Library/Application Support/Powermove Engine Corpus");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("audio-"))
                && p.join("manifest.json").is_file()
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => m.get(key),
        _ => None,
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    match v {
        Some(Value::Number(n)) => Some(*n),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

fn array(v: Option<&Value>) -> &[Value] {
    match v {
        Some(Value::Array(a)) => a,
        _ => &[],
    }
}

struct Measured {
    name: String,
    legacy_len: usize,
    ours_len: usize,
    expected_len: usize,
    peak_diff: f64,
    rms_diff_db: f64,
    signal_db: f64,
    clicks_ours: Vec<i64>,
    clicks_legacy: Vec<i64>,
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

#[test]
fn offline_mixes_match_todays_export_audio() {
    let Some(dir) = capture_dir() else {
        eprintln!("no audio parity capture (run tools/legacy-mix.ts); skipping");
        return;
    };
    let manifest = parse(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for p in array(get(&manifest, "projects")) {
        let name = text(get(p, "name")).unwrap();
        if let Some(e) = text(get(p, "error")) {
            failures.push(format!("{name}: the legacy capture failed: {e}"));
            continue;
        }
        let dur = num(get(p, "dur")).unwrap();
        let pdir = dir.join("projects").join(&name);
        let loaded = parse(&std::fs::read_to_string(pdir.join("loaded.json")).unwrap()).unwrap();
        let comp = text(get(&loaded, "compId"))
            .or_else(|| text(get(&loaded, "id")))
            .unwrap();
        let mut media: HashMap<String, PathBuf> = HashMap::new();
        if let Some(Value::Object(m)) = get(p, "media") {
            for (_, info) in m.iter() {
                if let (Some(id), Some(file)) =
                    (text(get(info, "assetId")), text(get(info, "file")))
                {
                    media.insert(id, PathBuf::from(file));
                }
            }
        }
        let doc = Document::new(Some(loaded), 1);
        let cache = SourceCache::new(128 << 20, 0);
        let resolver = move |id: &str| media.get(id).cloned();
        let graph = MixGraph::build(&doc, &comp, &mut Evaluator::new(), &cache, &resolver).unwrap();
        if !graph.problems.is_empty() {
            failures.push(format!("{name}: media problems {:?}", graph.problems));
        }
        let rate = 48_000u32;
        let mut mixer = Mixer::new(
            MixFormat::stereo(rate),
            Arc::new(doc),
            Arc::new(graph),
            Evaluator::new(),
            cache,
        );
        let expected_len = (dur * rate as f64).round() as usize;
        let mut ours = [Vec::new(), Vec::new()];
        offline::render(&mut mixer, 0, expected_len as i64, 8192, |_, inter| {
            for (i, v) in inter.iter().enumerate() {
                ours[i % 2].push(*v);
            }
            Ok(())
        })
        .unwrap();
        let (_, channels, legacy_rate, legacy) =
            read_wav(&std::fs::read(pdir.join("legacy.wav")).unwrap()).unwrap();
        assert_eq!(legacy_rate, rate);
        let lc = channels as usize;
        let legacy: Vec<Vec<f32>> = (0..lc)
            .map(|c| legacy.iter().skip(c).step_by(lc).copied().collect())
            .collect();
        let n = ours[0].len().min(legacy[0].len());
        let (mut peak, mut sq, mut sig) = (0f64, 0f64, 0f64);
        for c in 0..2 {
            let l = &legacy[c.min(lc - 1)];
            for i in 0..n {
                let d = ours[c][i] as f64 - l[i] as f64;
                peak = peak.max(d.abs());
                sq += d * d;
                sig += (l[i] as f64).powi(2);
            }
        }
        let rms = (sq / (2 * n) as f64).sqrt();
        let signal = (sig / (2 * n) as f64).sqrt();
        let mut m = Measured {
            name: name.clone(),
            legacy_len: legacy[0].len(),
            ours_len: ours[0].len(),
            expected_len,
            peak_diff: peak,
            rms_diff_db: db(rms),
            signal_db: db(signal),
            clicks_ours: vec![],
            clicks_legacy: vec![],
        };
        // Clicks: each spec's frames, placed through the clip's offset.
        for spec in array(get(p, "clicks")) {
            let fps = array(get(spec, "fps"));
            let (fnum, fden) = (
                num(fps.first()).unwrap() as i64,
                num(fps.get(1)).unwrap() as i64,
            );
            let from = num(get(spec, "from")).unwrap();
            let trim = num(get(spec, "trim")).unwrap();
            for f in array(get(spec, "frames")) {
                let frame = num(Some(f)).unwrap() as i64;
                // floor(N × sr × den / num) in the source, then the clip offset.
                let source_sample =
                    (frame as i128 * rate as i128 * fden as i128).div_euclid(fnum as i128) as i64;
                let expected = source_sample + ((from - trim) * rate as f64).round() as i64;
                let find = |x: &[f32]| -> i64 {
                    let lo = (expected - 48).max(0) as usize;
                    let hi = ((expected + 48) as usize).min(x.len());
                    let (i, _) = x[lo..hi].iter().enumerate().fold((0, 0f32), |b, (i, v)| {
                        if v.abs() > b.1 { (i, v.abs()) } else { b }
                    });
                    (lo + i) as i64 - expected
                };
                m.clicks_ours.push(find(&ours[0]));
                m.clicks_legacy.push(find(&legacy[0]));
            }
        }
        if m.ours_len != m.expected_len {
            failures.push(format!(
                "{name}: {} samples, expected round(duration × sr) = {}",
                m.ours_len, m.expected_len
            ));
        }
        if m.clicks_ours.iter().any(|d| d.abs() > 1) {
            failures.push(format!(
                "{name}: clicks off frame starts by {:?} samples",
                m.clicks_ours
            ));
        }
        results.push(m);
    }
    eprintln!("capture {}", dir.display());
    eprintln!(
        "project              | legacy n | ours n (= round) | signal dBFS | residual rms dBFS | residual peak | clicks ours / legacy (samples from frame start)"
    );
    for m in &results {
        eprintln!(
            "{:<20} | {:>8} | {:>8} ({:>6}) | {:>11.1} | {:>17.1} | {:>13.6} | {:?} / {:?}",
            m.name,
            m.legacy_len,
            m.ours_len,
            m.expected_len,
            m.signal_db,
            m.rms_diff_db,
            m.peak_diff,
            m.clicks_ours,
            m.clicks_legacy
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
