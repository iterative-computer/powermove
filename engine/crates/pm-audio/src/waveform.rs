//! Waveform data from a streaming decode: the legacy timeline envelope and
//! a min/max pyramid for zoomable drawing.
//!
//! * [`legacy_peaks`] is `peakEnvelope` of `legacy/core/audio.ts`: 100
//!   buckets per second of source, each the largest magnitude over every
//!   channel, bucket `b` spanning frames `floor(b × frames / length)` to the
//!   next bucket's start. `api.assets.get(id).peaks` keeps this shape.
//! * [`Waveform`] keeps the minimum and maximum per channel per bucket and
//!   reduces by 4 per level, so a view at any zoom reads one level.
//!
//! Both read the file once, in chunks; nothing holds the decoded file.

use std::path::Path;

use pm_media::audio::AudioDecoder;
use serde::Serialize;

use crate::error::{AudioError, Result};

/// Legacy envelope rate.
pub const PEAKS_PER_SECOND: f64 = 100.0;

/// `peakEnvelope(buffer)` for the best audio stream of a file.
pub fn legacy_peaks(path: &Path) -> Result<Vec<f32>> {
    let mut d = AudioDecoder::open(path, None)?;
    let info = d.info().clone();
    let frames = match info.length {
        Some(n) if n > 0 => n,
        _ => count(&mut d)?,
    }
    .max(1);
    d.seek(0)?;
    let duration = (frames as f64 / info.sample_rate as f64).max(0.001);
    let length = (frames)
        .min((duration * PEAKS_PER_SECOND).ceil() as u64)
        .max(1);
    let mut peaks = vec![0f32; length as usize];
    let bounds = |b: u64| -> u64 { b * frames / length };
    let mut bucket = 0u64;
    let mut next = if length > 1 {
        (bounds(1)).max(1)
    } else {
        frames
    };
    let mut at = 0u64;
    let mut buf = vec![Vec::new(); info.channels()];
    loop {
        for c in &mut buf {
            c.clear();
        }
        let n = d.read(&mut buf, 1 << 15)?;
        if n == 0 {
            break;
        }
        for i in 0..n {
            if at >= frames {
                break;
            }
            while at >= next && bucket + 1 < length {
                bucket += 1;
                next = if bucket == length - 1 {
                    frames
                } else {
                    bounds(bucket + 1).max(bounds(bucket) + 1)
                };
            }
            let mut m = 0f32;
            for c in &buf {
                m = m.max(c[i].abs());
            }
            let p = &mut peaks[bucket as usize];
            *p = p.max(m);
            at += 1;
        }
    }
    Ok(peaks)
}

fn count(d: &mut AudioDecoder) -> Result<u64> {
    let mut buf = vec![Vec::new(); d.info().channels()];
    let mut total = 0u64;
    loop {
        for c in &mut buf {
            c.clear();
        }
        let n = d.read(&mut buf, 1 << 16)?;
        if n == 0 {
            return Ok(total);
        }
        total += n as u64;
    }
}

/// Minimum and maximum per channel per bucket, at several resolutions.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waveform {
    pub sample_rate: u32,
    pub channels: usize,
    /// Program samples decoded.
    pub frames: u64,
    pub levels: Vec<Level>,
}

/// One resolution: bucket `b` of channel `c` is `[min, max]` at
/// `data[b × channels + c]`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    pub samples_per_bucket: u64,
    pub data: Vec<[f32; 2]>,
}

impl Waveform {
    /// Decodes `path` and builds levels from `samples_per_bucket` (the
    /// finest) up, each 4 times coarser, until a level has at most 64
    /// buckets.
    pub fn compute(path: &Path, samples_per_bucket: u64) -> Result<Waveform> {
        if samples_per_bucket == 0 {
            return Err(AudioError::invalid("samples per bucket must be positive"));
        }
        let mut d = AudioDecoder::open(path, None)?;
        let channels = d.info().channels();
        let sample_rate = d.info().sample_rate;
        let mut data = Vec::new();
        let mut cur = vec![[f32::INFINITY, f32::NEG_INFINITY]; channels];
        let mut filled = 0u64;
        let mut frames = 0u64;
        let mut buf = vec![Vec::new(); channels];
        loop {
            for c in &mut buf {
                c.clear();
            }
            let n = d.read(&mut buf, 1 << 15)?;
            if n == 0 {
                break;
            }
            for i in 0..n {
                for (e, c) in cur.iter_mut().zip(&buf) {
                    e[0] = e[0].min(c[i]);
                    e[1] = e[1].max(c[i]);
                }
                filled += 1;
                if filled == samples_per_bucket {
                    data.extend_from_slice(&cur);
                    cur.iter_mut()
                        .for_each(|e| *e = [f32::INFINITY, f32::NEG_INFINITY]);
                    filled = 0;
                }
            }
            frames += n as u64;
        }
        if filled > 0 {
            data.extend_from_slice(&cur);
        }
        let mut levels = vec![Level {
            samples_per_bucket,
            data,
        }];
        while levels.last().unwrap().data.len() / channels.max(1) > 64 {
            let next = levels.last().unwrap().reduce(channels, 4);
            levels.push(next);
        }
        Ok(Waveform {
            sample_rate,
            channels,
            frames,
            levels,
        })
    }

    /// The finest level whose buckets span at least `samples` samples.
    pub fn level_for(&self, samples: u64) -> &Level {
        self.levels
            .iter()
            .find(|l| l.samples_per_bucket >= samples)
            .unwrap_or_else(|| self.levels.last().unwrap())
    }
}

impl Level {
    fn reduce(&self, channels: usize, factor: usize) -> Level {
        let buckets = self.data.len() / channels.max(1);
        let mut data = Vec::with_capacity(buckets.div_ceil(factor) * channels);
        for start in (0..buckets).step_by(factor) {
            for c in 0..channels {
                let mut e = [f32::INFINITY, f32::NEG_INFINITY];
                for b in start..(start + factor).min(buckets) {
                    let v = self.data[b * channels + c];
                    e[0] = e[0].min(v[0]);
                    e[1] = e[1].max(v[1]);
                }
                data.push(e);
            }
        }
        Level {
            samples_per_bucket: self.samples_per_bucket * factor as u64,
            data,
        }
    }
}
