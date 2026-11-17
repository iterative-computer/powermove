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

/// Finest buckets per second of media: a clip zoomed in to a few frames
/// still has real data under each column; coarser zooms read the pyramid
/// built from it (`pm_panel_timeline::waveform::Levels`).
pub const RATE: f64 = 400.;

type Levels = Arc<pm_panel_timeline::waveform::Levels>;

enum State {
    Reading(Vec<async_channel::Sender<()>>),
    Ready(Levels),
    Failed,
}
static CACHE: Mutex<Option<HashMap<Key, State>>> = Mutex::new(None);

/// What a timeline can draw for `key` now: the envelope, `Some(None)` while
/// it is read (or wanted), `None` when the clip has no readable audio.
pub fn lookup(key: &Key) -> Option<Option<Levels>> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    match cache.as_ref().and_then(|c| c.get(key)) {
        Some(State::Ready(levels)) => Some(Some(levels.clone())),
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
    let state = match levels(key) {
        Some(levels) => State::Ready(levels),
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

/// The peak and RMS of the heard streams per ~1/`RATE` s of clip time, as
/// a pyramid for every zoom.
fn levels(key: &Key) -> Option<Levels> {
    let l = pm_audio::waveform::clip_levels(&key.path, &key.streams, key.timing, RATE).ok()?;
    Some(Arc::new(pm_panel_timeline::waveform::Levels::new(
        l.rate, l.channels, &l.peak, &l.rms,
    )))
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
        // (record, the loudest bucket: clip seconds × 400)
        for (text, at) in [
            (
                format!(r#"{{"kind":"audio","timing":2,"audioStreams":{entries}}}"#),
                300,
            ),
            (
                format!(r#"{{"kind":"audio","audioStreams":{entries}}}"#),
                200,
            ),
            (r#"{"kind":"audio"}"#.to_owned(), 40),
        ] {
            let doc = record(&text);
            let key = Key::of(path.clone(), doc.root());
            let _ = read(&key).recv_blocking();
            let levels = lookup(&key).unwrap().unwrap();
            assert!((levels.rate - RATE).abs() < 1e-9);
            let loudest = levels.combined.peak[0]
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(loudest, at, "{text}");
        }
    }
}
