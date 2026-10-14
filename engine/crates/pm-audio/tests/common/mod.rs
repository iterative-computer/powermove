//! Shared helpers: documents, generated media, offline renders.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_audio::offline::{self, PcmFormat, WavWriter};
use pm_audio::{MixFormat, MixGraph, Mixer, SourceCache};
use pm_eval::Evaluator;
use pm_model::Document;
use pm_model::json::parse;

pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-audio-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Our FFmpeg build's command-line tool (`PM_FFMPEG_DIR`, see engine/env.sh).
pub fn ffmpeg() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg")
}

/// Encodes a `lavfi` source (`aevalsrc=…`) into `path` with `args`, unless
/// the file is already there.
pub fn generate(path: &Path, source: &str, args: &[&str]) {
    if path.exists() {
        return;
    }
    let status = std::process::Command::new(ffmpeg())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(source)
        .args(args)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg failed for {}", path.display());
}

/// Writes a 32-bit float WAV from planar channels.
pub fn write_wav(path: &Path, rate: u32, planar: &[Vec<f32>]) {
    let frames = planar[0].len();
    let mut inter = Vec::with_capacity(frames * planar.len());
    for i in 0..frames {
        for c in planar {
            inter.push(c[i]);
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut w = WavWriter::new(
        std::io::BufWriter::new(file),
        PcmFormat::F32,
        planar.len() as u16,
        rate,
    )
    .unwrap();
    w.write(0, &inter).unwrap();
    w.finish().unwrap();
}

pub fn clicks(len: usize, at: &[usize], amp: f32) -> Vec<f32> {
    let mut x = vec![0f32; len];
    for &i in at {
        x[i] = amp;
    }
    x
}

pub fn tone(rate: u32, seconds: f64, hz: f64, amp: f64) -> Vec<f32> {
    (0..(rate as f64 * seconds).round() as usize)
        .map(|i| (amp * (std::f64::consts::TAU * hz * i as f64 / rate as f64).sin()) as f32)
        .collect()
}

pub fn ch(v: &str) -> String {
    format!(r#"{{"v":{v},"kf":[],"expr":null}}"#)
}

/// A keyframed channel with linear keys `(t, v)`.
pub fn keys(points: &[(f64, f64)]) -> String {
    let kf: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, (t, v))| {
            format!(
                r#"{{"t":{t},"v":{v},"i":"k{i}","inInterp":"linear","outInterp":"linear","inEase":{{"speed":0,"influence":16.666666666666668}},"outEase":{{"speed":0,"influence":16.666666666666668}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"v":{},"kf":[{}],"expr":null}}"#,
        points[0].1,
        kf.join(",")
    )
}

/// A held boolean channel switching at the given times.
pub fn bool_keys(points: &[(f64, bool)]) -> String {
    let kf: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(i, (t, v))| {
            format!(
                r#"{{"t":{t},"v":{v},"i":"b{i}","inInterp":"hold","outInterp":"hold","inEase":{{"speed":0,"influence":16.666666666666668}},"outEase":{{"speed":0,"influence":16.666666666666668}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"v":{},"kf":[{}],"expr":null}}"#,
        points[0].1,
        kf.join(",")
    )
}

pub struct LayerSpec<'a> {
    pub id: &'a str,
    pub ty: &'a str,
    pub from: f64,
    pub dur: f64,
    pub d: String,
    pub extra: String,
}

pub fn layer(id: &str, ty: &str, from: f64, dur: f64, d: &str, extra: &str) -> String {
    format!(
        r#"{{"id":"{id}","type":"{ty}","name":"{id}","from":{from},"dur":{dur},"on":true,"solo":false,"group":null,"parent":null,"p":{{}},"fx":[],"masks":[],"d":{d}{extra}}}"#
    )
}

pub fn audio(id: &str, asset: &str, from: f64, dur: f64, d_extra: &str, extra: &str) -> String {
    let d = format!(r#"{{"asset":"{asset}","gain":1,"trim":0,"fadeIn":0,"fadeOut":0{d_extra}}}"#);
    layer(id, "audio", from, dur, &d, extra)
}

/// A stored-form project document (`PM.proj`): the open composition `C` at
/// the root, nested compositions in `comps`.
pub fn project(
    fps: f64,
    dur: f64,
    layers: &[String],
    comps: &[(String, String)],
    extra: &str,
) -> Document {
    let comps: Vec<String> = comps
        .iter()
        .map(|(id, c)| format!(r#""{id}":{c}"#))
        .collect();
    let text = format!(
        r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":{fps},"dur":{dur},"layers":[{}],"comps":{{{}}},"assets":{{}},"params":{{}}{extra}}}"#,
        layers.join(","),
        comps.join(",")
    );
    Document::new(Some(parse(&text).unwrap()), 1)
}

pub fn comp(fps: f64, dur: f64, layers: &[String], extra: &str) -> String {
    format!(
        r#"{{"id":"X","name":"N","w":320,"h":180,"fps":{fps},"dur":{dur},"layers":[{}]{extra}}}"#,
        layers.join(",")
    )
}

pub struct Rig {
    pub cache: Arc<SourceCache>,
    pub media: HashMap<String, PathBuf>,
}

impl Rig {
    pub fn new(media: &[(&str, &Path)]) -> Rig {
        Rig {
            cache: SourceCache::new(64 << 20, 0),
            media: media
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_path_buf()))
                .collect(),
        }
    }

    pub fn graph(&self, doc: &Document, comp: &str) -> MixGraph {
        let media = self.media.clone();
        let resolver = move |id: &str| media.get(id).cloned();
        let mut ev = Evaluator::new();
        MixGraph::build(doc, comp, &mut ev, &self.cache, &resolver).unwrap()
    }

    pub fn mixer(&self, doc: Document, comp: &str, format: MixFormat) -> Mixer {
        let graph = Arc::new(self.graph(&doc, comp));
        Mixer::new(
            format,
            Arc::new(doc),
            graph,
            Evaluator::new(),
            self.cache.clone(),
        )
    }

    /// Renders samples `[start, end)` as planar channels, in blocks.
    pub fn render(&self, mixer: &mut Mixer, start: i64, end: i64, block: usize) -> Vec<Vec<f32>> {
        let channels = mixer.format().channels();
        let mut out = vec![Vec::new(); channels];
        let summary = offline::render(mixer, start, end, block, |_, inter| {
            for (i, v) in inter.iter().enumerate() {
                out[i % channels].push(*v);
            }
            Ok(())
        })
        .unwrap();
        assert!(
            summary.errors.is_empty(),
            "render errors: {:?}",
            summary.errors
        );
        out
    }
}

/// Indexes of samples whose magnitude exceeds `threshold` and is a local
/// maximum (a click's peak).
pub fn peaks(x: &[f32], threshold: f32) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < x.len() {
        if x[i].abs() > threshold {
            let mut best = i;
            let mut j = i;
            while j < x.len() && j < i + 64 {
                if x[j].abs() > x[best].abs() {
                    best = j;
                }
                j += 1;
            }
            out.push(best);
            i = best + 64;
        } else {
            i += 1;
        }
    }
    out
}
