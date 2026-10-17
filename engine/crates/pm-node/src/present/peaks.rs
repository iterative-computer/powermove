//! Waveform peaks for the renderer's timeline and asset views (plan
//! section 16, "engine waveforms"; appendix C: timeline waveforms come from
//! the engine). Today's engine computes `peakEnvelope` from a whole-file
//! Web Audio decode under a 128 MiB budget, so long files get no waveform;
//! here the envelope comes from a streaming decode of any length
//! (`pm_audio::waveform::legacy_peaks`, the same buckets), on a worker
//! thread, kept per file, size, and modification time.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::SystemTime;

use pm_engine::Action;
use pm_protocol::clock::{AudioMessage, AudioPeaks, MAX_PEAKS};
use pm_protocol::codec;
use pm_protocol::envelope::{Envelope, Event, Message};

use super::render::{Key, Sink};

type FileKey = (PathBuf, u64, Option<SystemTime>);
/// Envelopes by file: (peaks, duration in seconds).
type Kept = Mutex<VecDeque<(FileKey, Arc<(Vec<f32>, f64)>)>>;

struct Job {
    key: Key,
    asset: String,
    path: Option<PathBuf>,
}

/// Computes peaks one file at a time; answers every request.
pub struct PeakWorker {
    tx: Option<Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

/// Envelopes kept (a few MB at most for hours of audio).
const KEPT: usize = 64;

impl PeakWorker {
    pub fn new(sink: Sink) -> PeakWorker {
        let (tx, rx) = channel::<Job>();
        let thread = pm_engine::qos::spawn_engine_thread(
            pm_protocol::error::EngineProcess::Interactive,
            "pm-viewer-peaks",
            move || {
                let kept: Kept = Mutex::default();
                let mut pending: VecDeque<Job> = VecDeque::new();
                loop {
                    let job = match pending.pop_front() {
                        Some(j) => j,
                        None => match rx.recv() {
                            Ok(j) => j,
                            Err(_) => return,
                        },
                    };
                    // Drop duplicates queued meanwhile (a timeline redraw
                    // asks again).
                    while let Ok(j) = rx.try_recv() {
                        if !pending
                            .iter()
                            .chain(std::iter::once(&job))
                            .any(|p| p.key == j.key && p.asset == j.asset && p.path == j.path)
                        {
                            pending.push_back(j);
                        }
                    }
                    let answer = answer(&job, &kept);
                    let envelope = Envelope::new(Message::Event {
                        body: Event::Audio(AudioMessage::Peaks(answer)),
                    })
                    .with_session(job.key.1);
                    if let Ok(bytes) = codec::encode(&envelope) {
                        sink(vec![Action::Send {
                            port: job.key.0,
                            bytes,
                        }]);
                    }
                }
            },
        )
        .ok();
        PeakWorker {
            tx: Some(tx),
            thread,
        }
    }

    /// Asks for an asset's peaks (`path`: where the session's media map
    /// resolved it, `None` when offline).
    pub fn request(&self, key: Key, asset: String, path: Option<PathBuf>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job { key, asset, path });
        }
    }
}

impl Drop for PeakWorker {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn answer(job: &Job, kept: &Kept) -> AudioPeaks {
    let none = |error: String| AudioPeaks {
        asset: job.asset.clone(),
        peaks_per_second: pm_audio::waveform::PEAKS_PER_SECOND,
        duration: 0.0,
        peaks: Vec::new(),
        error: Some(error),
    };
    let Some(path) = &job.path else {
        return none("the media is offline".into());
    };
    let Ok(meta) = std::fs::metadata(path) else {
        return none("the media is offline".into());
    };
    let file: FileKey = (path.clone(), meta.len(), meta.modified().ok());
    let hit = kept
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, _)| *k == file)
        .map(|(_, v)| v.clone());
    let data = match hit {
        Some(d) => d,
        None => {
            let duration = pm_media::audio::AudioDecoder::open(path, None)
                .ok()
                .and_then(|d| d.info().duration());
            match pm_audio::waveform::legacy_peaks(path) {
                Ok(mut peaks) => {
                    peaks.truncate(MAX_PEAKS);
                    let duration = duration
                        .unwrap_or(peaks.len() as f64 / pm_audio::waveform::PEAKS_PER_SECOND);
                    let d = Arc::new((peaks, duration));
                    let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
                    if kept.len() >= KEPT {
                        kept.pop_front();
                    }
                    kept.push_back((file, d.clone()));
                    d
                }
                Err(error) => return none(error.to_string()),
            }
        }
    };
    AudioPeaks {
        asset: job.asset.clone(),
        peaks_per_second: pm_audio::waveform::PEAKS_PER_SECOND,
        duration: data.1,
        peaks: data.0.clone(),
        error: None,
    }
}
