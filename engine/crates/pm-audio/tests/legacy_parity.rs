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

/// A captured media path, or the same file in this checkout: captures
/// record repository fixtures by the absolute path of the worktree that made
/// them (`apps/desktop/e2e/fixtures/…`), which may be gone; media the capture
/// generated lives in its own `media/` directory.
fn resolve_media(capture: &Path, file: &Path) -> PathBuf {
    if file.is_file() {
        return file.to_path_buf();
    }
    let Some(name) = file.file_name() else {
        return file.to_path_buf();
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures");
    [capture.join("media").join(name), fixtures.join(name)]
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| file.to_path_buf())
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
    /// The residual with in-points rounded to whole samples as today's export does.
    whole_sample_db: Option<f64>,
    signal_db: f64,
    clicks_ours: Vec<i64>,
    clicks_legacy: Vec<i64>,
    /// The residual its ceiling guards (whole mix, masked, low band, or
    /// adjusted; see [`ceiling`]).
    guarded_db: f64,
}

enum Ceiling {
    /// The whole mix.
    Mix(f64),
    /// The mix outside spans (seconds) where a decided fix changes it.
    Outside(f64, &'static [(f64, f64)]),
}

/// Each project's residual ceiling (dBFS). Today's export model
/// (`pm_audio::today`, decisions AUDIO-5 and AUDIO-6) makes the mix
/// bit-identical to today's (−240 dBFS here) wherever both decode the same
/// samples and the bus stays stereo.
fn ceiling(name: &str) -> Ceiling {
    match name {
        // Lossy decoders: FFmpeg revisions round differently from
        // Chromium's (AAC, MP3, Opus, the H.264 file's AAC soundtrack).
        "corpus-media-audio" => Ceiling::Mix(-90.0),
        "embedded" => Ceiling::Mix(-105.0),
        "mono" => Ceiling::Mix(-150.0),
        // Today's bus sums its clips in the order of a pointer hash set
        // (`AudioSummingJunction::outputs_`), so where three or more clips
        // feed one bus channel at once its own captures differ by an ulp
        // from run to run (−149 dBFS between two captures of
        // `corpus-media-audio`): that spread is the ceiling there.
        "wide-quad" => Ceiling::Mix(-145.0),
        // Decision LAYOUT-1: a 7.1 clip folds by BS.775 instead of keeping
        // only its first two channels; the mix differs from today by design
        // while it is connected (1 s to 2 s), and only there.
        "wide-71" => Ceiling::Outside(-145.0, &[(1.0, 2.01)]),
        n if n.starts_with("real-") => Ceiling::Mix(-240.0),
        _ => Ceiling::Mix(-240.0),
    }
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
                    media.insert(id, resolve_media(&dir, &PathBuf::from(file)));
                }
            }
        }
        let rate = 48_000u32;
        let expected_len = (dur * rate as f64).round() as usize;
        let (ours, problems) = render(loaded.clone(), &comp, &media, rate, expected_len);
        if !problems.is_empty() {
            failures.push(format!("{name}: media problems {problems:?}"));
        }
        // Today's export starts each clip on a whole sample of the 48 kHz
        // decode (`AudioBufferSourceNode.start` rounds the offset), so a
        // fractional in-point plays up to half a sample early or late. The
        // same document with in-points rounded that way isolates the rest
        // of the chain.
        let mut quantized = loaded;
        let whole_sample = quantize_in_points(&mut quantized, rate as f64)
            .then(|| render(quantized, &comp, &media, rate, expected_len).0);
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
        // Where the residual sits: 100 ms windows, worst first, per channel.
        let win = rate as usize / 10;
        let mut windows: Vec<(f64, usize, f64, f64)> = (0..n.div_ceil(win))
            .map(|w| {
                let (a, b) = (w * win, ((w + 1) * win).min(n));
                let mut e = [0f64; 2];
                let mut sg = 0f64;
                for c in 0..2 {
                    let l = &legacy[c.min(lc - 1)];
                    for i in a..b {
                        e[c] += (ours[c][i] as f64 - l[i] as f64).powi(2);
                        sg += (l[i] as f64).powi(2);
                    }
                }
                let k = (b - a) as f64;
                (
                    db(((e[0] + e[1]) / (2.0 * k)).sqrt()),
                    a,
                    db((e[0] / k).sqrt()) - db((e[1] / k).sqrt()),
                    db((sg / (2.0 * k)).sqrt()),
                )
            })
            .collect();
        windows.sort_by(|x, y| y.0.total_cmp(&x.0));
        if std::env::var("PM_AUDIO_PARITY_DETAIL").is_ok() {
            for (e, a, lr, sg) in windows.iter().take(6) {
                eprintln!(
                    "  {name} @ {:.1} s: residual {e:.1} dBFS (L − R {lr:+.1} dB), signal {sg:.1} dBFS",
                    *a as f64 / rate as f64
                );
            }
            let mut planar = ours.clone();
            for c in &mut planar {
                c.truncate(n);
            }
            common::write_wav(&pdir.join("ours.wav"), rate, &planar);
            if let Some(q) = &whole_sample {
                let mut planar = q.to_vec();
                for c in &mut planar {
                    c.truncate(n);
                }
                common::write_wav(&pdir.join("ours-whole-sample.wav"), rate, &planar);
            }
        }
        let rms = (sq / (2 * n) as f64).sqrt();
        let whole_sample_db = whole_sample.as_ref().map(|q| {
            let mut e = 0f64;
            for c in 0..2 {
                let l = &legacy[c.min(lc - 1)];
                for i in 0..n {
                    e += (q[c][i] as f64 - l[i] as f64).powi(2);
                }
            }
            db((e / (2 * n) as f64).sqrt())
        });
        let signal = (sig / (2 * n) as f64).sqrt();
        let mut m = Measured {
            name: name.clone(),
            legacy_len: legacy[0].len(),
            ours_len: ours[0].len(),
            expected_len,
            peak_diff: peak,
            rms_diff_db: db(rms),
            whole_sample_db,
            signal_db: db(signal),
            clicks_ours: vec![],
            clicks_legacy: vec![],
            guarded_db: 0.0,
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
        // Residual ceilings: what is left once the allowed differences
        // (docs/engine/pm-audio.md) are accounted for, as regression guards.
        let (residual, ceiling) = match ceiling(&name) {
            Ceiling::Mix(db) => (m.rms_diff_db, db),
            Ceiling::Outside(ceil, spans) => {
                let mut e = 0f64;
                let mut k = 0usize;
                for i in 0..n {
                    let t = i as f64 / rate as f64;
                    if spans.iter().any(|(a, b)| t >= *a && t < *b) {
                        continue;
                    }
                    for c in 0..2 {
                        let l = &legacy[c.min(lc - 1)];
                        e += (ours[c][i] as f64 - l[i] as f64).powi(2);
                    }
                    k += 2;
                }
                (db((e / k.max(1) as f64).sqrt()), ceil)
            }
        };
        m.guarded_db = residual;
        if residual > ceiling {
            failures.push(format!(
                "{name}: residual {residual:.1} dBFS above its {ceiling:.0} dBFS ceiling"
            ));
        }
        results.push(m);
    }
    eprintln!("capture {}", dir.display());
    eprintln!(
        "project              | legacy n | ours n (= round) | signal dBFS | residual rms dBFS | residual peak | whole-sample in-points | guarded dBFS | clicks ours / legacy (samples from frame start)"
    );
    for m in &results {
        eprintln!(
            "{:<20} | {:>8} | {:>8} ({:>6}) | {:>11.1} | {:>17.1} | {:>13.6} | {:>22} | {:>12.1} | {:?} / {:?}",
            m.name,
            m.legacy_len,
            m.ours_len,
            m.expected_len,
            m.signal_db,
            m.rms_diff_db,
            m.peak_diff,
            m.whole_sample_db
                .map_or("(none fractional)".to_string(), |d| format!("{d:.1} dBFS")),
            m.guarded_db,
            m.clicks_ours,
            m.clicks_legacy
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Media the graph could not open: (layer, asset, reason).
type Problems = Vec<(String, String, String)>;

/// Renders a loaded document's open composition, `[0, len)`, as stereo.
fn render(
    loaded: Value,
    comp: &str,
    media: &HashMap<String, PathBuf>,
    rate: u32,
    len: usize,
) -> ([Vec<f32>; 2], Problems) {
    let doc = Document::new(Some(loaded), 1);
    let cache = SourceCache::new(128 << 20, 0);
    let media = media.clone();
    let resolver = move |id: &str| media.get(id).cloned();
    let graph = MixGraph::build(&doc, comp, &mut Evaluator::new(), &cache, &resolver).unwrap();
    let problems = graph.problems.clone();
    let mut mixer = Mixer::new(
        MixFormat::stereo(rate),
        Arc::new(doc),
        Arc::new(graph),
        Evaluator::new(),
        cache,
    );
    let mut ours = [Vec::new(), Vec::new()];
    offline::render(&mut mixer, 0, len as i64, 8192, |_, inter| {
        for (i, v) in inter.iter().enumerate() {
            ours[i % 2].push(*v);
        }
        Ok(())
    })
    .unwrap();
    (ours, problems)
}

/// Rounds every static `trim` of a sounding layer to a whole sample at
/// `rate`, as today's export plays it. Returns whether any changed.
fn quantize_in_points(doc: &mut Value, rate: f64) -> bool {
    fn layers(list: Option<&mut Value>, rate: f64) -> bool {
        let Some(Value::Array(list)) = list else {
            return false;
        };
        let mut changed = false;
        for layer in list {
            let Value::Object(l) = layer else { continue };
            let sounding = match l.get("type") {
                Some(Value::String(t)) => matches!(t.to_string_lossy().as_str(), "audio" | "video"),
                _ => false,
            };
            if !sounding {
                continue;
            }
            if let Some(Value::Object(d)) = l.get_mut("d")
                && let Some(Value::Number(trim)) = d.get_mut("trim")
            {
                let q = (*trim * rate).round() / rate;
                if (q - *trim).abs() * rate > 1e-6 {
                    *trim = q;
                    changed = true;
                }
            }
        }
        changed
    }
    let Value::Object(root) = doc else {
        return false;
    };
    let mut changed = layers(root.get_mut("layers"), rate);
    if let Some(Value::Object(comps)) = root.get_mut("comps") {
        for (_, c) in comps.iter_mut() {
            if let Value::Object(c) = c {
                changed |= layers(c.get_mut("layers"), rate);
            }
        }
    }
    changed
}
