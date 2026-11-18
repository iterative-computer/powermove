//! Audio peak envelopes for timeline waveforms: each clip reading of a file
//! (its streams and timing) is read once per process, off the main thread,
//! and shared by every window that shows it. The envelope is what the mix
//! plays, on the clip's own timeline.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A file as an asset's clips play it: the streams heard and how its times
/// are read.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub path: PathBuf,
    pub streams: Vec<Option<usize>>,
    pub timing: pm_media::clip::ClipTiming,
}
impl Key {
    /// `path` as the asset `record` plays it.
    pub fn of(path: PathBuf, record: Option<&pm_model::Node>) -> Key {
        Key {
            path,
            streams: pm_audio::graph::selected_streams(record),
            timing: pm_audio::graph::clip_timing(record),
        }
    }
}

/// Envelope buckets per second of media (the MCP waveform tool's resolution).
pub const RATE: f64 = 100.;

enum State {
    Reading(Vec<async_channel::Sender<()>>),
    /// The envelope and its buckets per second.
    Ready(Arc<[f32]>, f64, Arc<[Arc<[f32]>]>),
    Failed,
}
static CACHE: Mutex<Option<HashMap<Key, State>>> = Mutex::new(None);

/// What a timeline can draw for `key` now: the envelope, `Some(None)` while
/// it is read (or wanted), `None` when the clip has no readable audio.
pub fn lookup(key: &Key) -> Option<Option<(Arc<[f32]>, f64, Arc<[Arc<[f32]>]>)>> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match cache.as_ref().and_then(|c| c.get(key)) {
        Some(State::Ready(peaks, rate, pyramid)) => {
            Some(Some((peaks.clone(), *rate, pyramid.clone())))
        }
        Some(State::Failed) => None,
        Some(State::Reading(_)) | None => Some(None),
    }
}

/// Reads `key`'s envelope unless it is known or already being read; the
/// returned receiver closes when the envelope (or a failure) is known.
/// Blocking: call it from a background task.
pub fn read(key: &Key) -> async_channel::Receiver<()> {
    let (done, wait) = async_channel::bounded(1);
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(HashMap::new);
        match cache.get_mut(key) {
            Some(State::Reading(waiters)) => {
                waiters.push(done);
                return wait;
            }
            Some(_) => return wait,
            None => {
                cache.insert(key.clone(), State::Reading(vec![]));
            }
        }
    }
    let state = match envelope(key) {
        Some((peaks, rate)) => {
            let pyramid = pyramid(&peaks);
            State::Ready(peaks, rate, pyramid)
        }
        None => State::Failed,
    };
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(State::Reading(waiters)) = cache
        .get_or_insert_with(HashMap::new)
        .insert(key.clone(), state)
    {
        drop(waiters);
    }
    drop(done);
    wait
}

/// The largest absolute sample of any channel of any heard stream per
/// ~1/`RATE` s of clip time, 0–1, and the exact buckets per second.
fn envelope(key: &Key) -> Option<(Arc<[f32]>, f64)> {
    let (peaks, rate) =
        pm_audio::waveform::clip_envelope(&key.path, &key.streams, key.timing, RATE).ok()?;
    Some((peaks.into(), rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A timeline draws a clip's waveform as its asset plays it: only the
    /// streams heard, each where the asset's timing places it.
    #[test]
    fn a_clip_draws_the_streams_it_plays_where_it_plays_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("two.mov");
        let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var("HOME").unwrap())
                    .join("Library/Caches/powermove-engine/ffmpeg/out")
            })
            .join("bin/ffmpeg");
        // Stream 0 clicks at 0.1 s; stream 1, starting 0.25 s into the
        // file, clicks 0.5 s into its own sound.
        let status = std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "aevalsrc='0.9*eq(n\\,4800)':s=48000:d=1"])
            .args(["-itsoffset", "0.25", "-f", "lavfi"])
            .args(["-i", "aevalsrc='0.9*eq(n\\,24000)':s=48000:d=1"])
            .args(["-map", "0:a", "-map", "1:a", "-c:a", "pcm_s16le"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let record =
            |text: &str| pm_model::Document::new(Some(pm_model::json::parse(text).unwrap()), 1);
        let entries = r#"[{"index":0,"channels":1,"sampleRate":48000,"on":false},{"index":1,"channels":1,"sampleRate":48000,"on":true}]"#;
        // (record, the loudest bucket: clip seconds × 100)
        for (text, at) in [
            (
                format!(r#"{{"kind":"audio","timing":2,"audioStreams":{entries}}}"#),
                75,
            ),
            (
                format!(r#"{{"kind":"audio","audioStreams":{entries}}}"#),
                50,
            ),
            (r#"{"kind":"audio"}"#.to_owned(), 10),
        ] {
            let doc = record(&text);
            let key = Key::of(path.clone(), doc.root());
            let _ = read(&key).recv_blocking();
            let (peaks, rate, _) = lookup(&key).unwrap().unwrap();
            assert!((rate - RATE).abs() < 1e-9);
            let loudest = peaks
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(loudest, at, "{text}");
        }
    }
}

/// Successive min/max-magnitude summaries; performed only by the decode worker.
fn pyramid(peaks: &[f32]) -> Arc<[Arc<[f32]>]> {
    let mut levels: Vec<Arc<[f32]>> = Vec::new();
    let mut previous = peaks;
    while previous.len() > 1 {
        let next: Arc<[f32]> = previous
            .chunks(2)
            .map(|pair| pair.iter().copied().fold(0., f32::max))
            .collect();
        levels.push(next);
        previous = levels.last().unwrap();
    }
    levels.into()
}

#[cfg(test)]
mod pcm_tests {
    use super::*;
    #[test]
    fn peak_summary_matches_known_pcm_and_pyramid_preserves_extrema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known.wav");
        let samples: Vec<i16> = (0..960)
            .map(|i| if i < 480 { 8192 } else { -24576 })
            .collect();
        let bytes = (samples.len() * 2) as u32;
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + bytes).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&48000u32.to_le_bytes());
        wav.extend_from_slice(&96000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&bytes.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(&path, wav).unwrap();
        let (peaks, rate) = envelope(&Key::of(path, None)).unwrap();
        assert_eq!(rate, 100.);
        assert_eq!(peaks.len(), 2);
        assert!((peaks[0] - 0.25).abs() < 0.0001);
        assert!((peaks[1] - 0.75).abs() < 0.0001);
        let pyramid = pyramid(&peaks);
        assert_eq!(&*pyramid[0], &[0.75]);
    }
}

/// Static source timing for waveform drawing, without changing the shared timeline index.
pub fn layer_timing(root: &pm_model::Node, id: &str) -> Option<(f64, f64)> {
    let at = pm_kernel::model::find_layer(root, id)?;
    let comp = pm_kernel::model::comp_node(root, at.comp_id.as_deref())?;
    let layer = pm_kernel::model::layers_of(comp).nth(at.index)?;
    let number = |key: &str, default: f64| {
        layer
            .get("d")
            .and_then(|d| d.get(key))
            .and_then(|n| n.get("v").unwrap_or(n).to_value().as_f64())
            .filter(|v| v.is_finite())
            .unwrap_or(default)
    };
    Some((number("trim", 0.).max(0.), number("speed", 1.)))
}
#[cfg(test)]
mod timing_tests {
    use super::*;
    #[test]
    fn waveform_timing_reads_numeric_and_channel_values() {
        for content in [
            r#"{"trim":2,"speed":2}"#,
            r#"{"trim":{"v":2,"kf":[]},"speed":{"v":2,"kf":[]}}"#,
        ] {
            let layers = format!(r#"[{{"id":"a","type":"audio","d":{content}}}]"#);
            for project in [
                format!(r#"{{"layers":{layers}}}"#),
                format!(r#"{{"compId":"c","comps":{{"c":{{"layers":{layers}}}}}}}"#),
            ] {
                let doc =
                    pm_model::Document::new(Some(pm_model::json::parse(&project).unwrap()), 1);
                assert_eq!(layer_timing(doc.root().unwrap(), "a"), Some((2., 2.)));
            }
        }
    }
}
