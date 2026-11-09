//! Audio peak envelopes for timeline waveforms: each file is read once per
//! process, off the main thread, and shared by every window that shows it.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Envelope buckets per second of media (the MCP waveform tool's resolution).
pub const RATE: f64 = 100.;

enum State {
    Reading(Vec<async_channel::Sender<()>>),
    /// The envelope and its buckets per second.
    Ready(Arc<[f32]>, f64),
    Failed,
}
static CACHE: Mutex<Option<HashMap<PathBuf, State>>> = Mutex::new(None);

/// What a timeline can draw for `path` now: the envelope, `Some(None)` while
/// it is read (or wanted), `None` when the file has no readable audio.
pub fn lookup(path: &Path) -> Option<Option<(Arc<[f32]>, f64)>> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match cache.as_ref().and_then(|c| c.get(path)) {
        Some(State::Ready(peaks, rate)) => Some(Some((peaks.clone(), *rate))),
        Some(State::Failed) => None,
        Some(State::Reading(_)) | None => Some(None),
    }
}

/// Reads `path`'s envelope unless it is known or already being read; the
/// returned receiver closes when the envelope (or a failure) is known.
/// Blocking: call it from a background task.
pub fn read(path: &Path) -> async_channel::Receiver<()> {
    let (done, wait) = async_channel::bounded(1);
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(HashMap::new);
        match cache.get_mut(path) {
            Some(State::Reading(waiters)) => {
                waiters.push(done);
                return wait;
            }
            Some(_) => return wait,
            None => {
                cache.insert(path.to_owned(), State::Reading(vec![]));
            }
        }
    }
    let state = match envelope(path) {
        Some((peaks, rate)) => State::Ready(peaks, rate),
        None => State::Failed,
    };
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(State::Reading(waiters)) = cache
        .get_or_insert_with(HashMap::new)
        .insert(path.to_owned(), state)
    {
        drop(waiters);
    }
    drop(done);
    wait
}

/// The largest absolute sample of any channel per ~1/`RATE` s, 0–1, and the
/// exact buckets per second.
fn envelope(path: &Path) -> Option<(Arc<[f32]>, f64)> {
    let rate = pm_media::probe::probe_timing(path)
        .ok()?
        .streams
        .iter()
        .find_map(|stream| stream.audio.as_ref().map(|audio| audio.sample_rate))?;
    let per_peak = ((rate as f64 / RATE).round() as usize).max(1);
    let peaks = pm_media::probe::peaks::peaks(path, None, per_peak, 0).ok()?;
    let channels = peaks.channels.max(1);
    let exact = peaks.sample_rate as f64 / per_peak as f64;
    Some((
        peaks
            .data
            .chunks(channels)
            .map(|bucket| {
                bucket
                    .iter()
                    .map(|[low, high]| low.abs().max(high.abs()))
                    .fold(0f32, f32::max)
                    .min(1.)
            })
            .collect(),
        exact,
    ))
}
