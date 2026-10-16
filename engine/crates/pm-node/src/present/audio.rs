//! A viewer's audio playback (plan sections 15 and 16; contract 6): the
//! master clock of the presenter.
//!
//! One control thread per viewer owns a `pm_audio::playback::Playback` (the
//! output device through `cpal`) and its own mirror of the document, fed
//! with the same updates the render thread gets. The render thread never
//! blocks on audio: it sends controls and reads the clock through a
//! [`ClockHandle`]. The device opens at the first Play, or earlier once
//! warmed ([`AudioCtl::Warm`]): while paused, when the composition has
//! something to hear and no control arrived for [`WARM_IDLE`] (today's
//! `requestContextWarmup` opens its AudioContext on idle the same way), so
//! the first Play does not wait for the device to open. A new revision
//! reaches the mixer at most every [`PROJECT_INTERVAL`] while edits stream
//! in (pm-audio glides from what played into the new mix).
//!
//! When no output device opens, [`AudioLink::failed`] says so and the
//! presenter runs its line on the host clock instead.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pm_audio::playback::{ClockHandle, Playback, PlaybackConfig};
use pm_audio::{MixFormat, SourceCache};
use pm_engine::document::DocumentUpdate;
use pm_protocol::clock::{SeekMode, TransportCommand};
use pm_protocol::common::{PlaybackRate, TickRange, Ticks};
use serde::Serialize;

use super::document::MediaMap;

/// The shortest interval between two mixes of new revisions.
pub const PROJECT_INTERVAL: Duration = Duration::from_millis(30);

/// How long the controls must rest before a warmed thread checks whether
/// the composition is audible and opens the device.
pub const WARM_IDLE: Duration = Duration::from_millis(100);

/// Decoded audio kept per viewer.
const SOURCE_BUDGET: usize = 256 * 1024 * 1024;
const DECODE_WORKERS: usize = 2;

pub enum AudioCtl {
    Document(DocumentUpdate),
    Media(MediaMap),
    Composition(String),
    /// Seek to `from` (ticks) and play at `rate`.
    Play {
        from: i64,
        rate: PlaybackRate,
    },
    Pause,
    /// Open the device ahead of the first Play once the composition is
    /// audible and the controls rest.
    Warm,
    /// A seek while playing.
    Seek(i64),
    SetLoop(Option<TickRange>),
    Shutdown,
}

/// What the audio thread reports (the engine's status).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    pub device: Option<String>,
    pub device_rate: Option<u32>,
    pub latency_us: Option<u32>,
    pub error: Option<String>,
    pub projects: u64,
    pub underruns: u64,
    pub starved_renders: u64,
    pub problems: Vec<String>,
}

pub struct AudioLink {
    tx: Sender<AudioCtl>,
    clock: Arc<Mutex<Option<ClockHandle>>>,
    status: Arc<Mutex<AudioStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioLink {
    /// Starts the control thread with the view's document and media.
    pub fn start(
        doc: pm_model::Document,
        composition: String,
        media: MediaMap,
    ) -> Result<AudioLink, String> {
        let (tx, rx) = channel();
        let clock = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(AudioStatus::default()));
        let (thread_clock, thread_status) = (clock.clone(), status.clone());
        let thread = pm_engine::qos::spawn_engine_thread(
            pm_protocol::error::EngineProcess::Interactive,
            "pm-viewer-audio",
            move || {
                Control {
                    rx,
                    clock: thread_clock,
                    status: thread_status,
                    doc,
                    composition,
                    media,
                    playback: None,
                    dirty: true,
                    broken: false,
                    loop_range: None,
                    last_project: None,
                    warm: false,
                    warm_due: None,
                    warmed: false,
                }
                .run()
            },
        )
        .map_err(|e| format!("could not start the audio thread: {e}"))?;
        Ok(AudioLink {
            tx,
            clock,
            status,
            thread: Some(thread),
        })
    }

    pub fn send(&self, ctl: AudioCtl) {
        let _ = self.tx.send(ctl);
    }

    /// The device clock, once the device opened.
    pub fn clock(&self) -> Option<ClockHandle> {
        self.clock.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// No output device could be opened.
    pub fn failed(&self) -> bool {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .error
            .is_some()
    }

    pub fn status(&self) -> AudioStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for AudioLink {
    fn drop(&mut self) {
        let _ = self.tx.send(AudioCtl::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Control {
    rx: Receiver<AudioCtl>,
    clock: Arc<Mutex<Option<ClockHandle>>>,
    status: Arc<Mutex<AudioStatus>>,
    doc: pm_model::Document,
    composition: String,
    media: MediaMap,
    playback: Option<Playback>,
    /// The mixer plays an older revision, media map, or composition.
    dirty: bool,
    /// An update did not apply; wait for a resync.
    broken: bool,
    loop_range: Option<TickRange>,
    last_project: Option<Instant>,
    /// Asked to open the device ahead of Play ([`AudioCtl::Warm`]).
    warm: bool,
    /// When to check, once the controls rested ([`WARM_IDLE`]).
    warm_due: Option<Instant>,
    /// The device opened ahead of Play and the mixer has no project yet.
    warmed: bool,
}

impl Control {
    fn run(mut self) {
        loop {
            // A device opened ahead of Play gets the project at Play, with
            // the media the engine has resolved by then (as before).
            let feeding = self.playback.is_some() && !self.warmed;
            let timeout = match (feeding, self.dirty, self.last_project) {
                (true, true, Some(last)) => PROJECT_INTERVAL.saturating_sub(last.elapsed()),
                (true, true, None) => Duration::ZERO,
                _ if self.playback.is_none() && self.warm_due.is_some() => self
                    .warm_due
                    .map_or(Duration::ZERO, |due| due.saturating_duration_since(Instant::now())),
                _ => Duration::from_secs(3600),
            };
            let first = match self.rx.recv_timeout(timeout) {
                Ok(ctl) => Some(ctl),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let mut controls: Vec<AudioCtl> = first.into_iter().collect();
            while let Ok(ctl) = self.rx.try_recv() {
                controls.push(ctl);
            }
            let rested = controls.is_empty();
            for ctl in controls {
                if !self.apply(ctl) {
                    self.shutdown();
                    return;
                }
            }
            if self.playback.is_none() && self.warm {
                if !rested || self.warm_due.is_none() {
                    // A control came: check once they rest again.
                    self.warm_due = Some(Instant::now() + WARM_IDLE);
                } else if self.warm_due.is_some_and(|due| Instant::now() >= due) {
                    self.warm_due = None;
                    self.warm_if_audible();
                }
            }
            let due = self
                .last_project
                .is_none_or(|last| last.elapsed() >= PROJECT_INTERVAL);
            if self.playback.is_some() && !self.warmed && self.dirty && due {
                self.set_project();
            }
            self.report();
        }
        self.shutdown();
    }

    fn shutdown(&mut self) {
        *self.clock.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(mut playback) = self.playback.take() {
            playback.shutdown();
        }
    }

    /// Returns false on shutdown.
    fn apply(&mut self, ctl: AudioCtl) -> bool {
        match ctl {
            AudioCtl::Shutdown => return false,
            AudioCtl::Document(update) => {
                let replace = matches!(update, DocumentUpdate::Replace { .. });
                if self.broken && !replace {
                    return true;
                }
                self.broken = update.apply_to(&mut self.doc).is_err();
                self.dirty = true;
            }
            AudioCtl::Media(media) => {
                self.media = media;
                self.dirty = true;
            }
            AudioCtl::Composition(composition) => {
                if composition != self.composition {
                    self.composition = composition;
                    self.dirty = true;
                }
            }
            AudioCtl::SetLoop(range) => {
                self.loop_range = range;
                self.command(TransportCommand::SetLoop { range });
            }
            AudioCtl::Pause => self.command(TransportCommand::Pause),
            AudioCtl::Warm => self.warm = true,
            AudioCtl::Seek(at) => self.command(TransportCommand::Seek {
                time: Ticks::new(at.max(0)).unwrap_or(Ticks::ZERO),
                mode: SeekMode::Exact,
            }),
            AudioCtl::Play { from, rate } => {
                if self.playback.is_none() && !self.open() {
                    return true;
                }
                self.warmed = false;
                if self.dirty {
                    self.set_project();
                }
                self.command(TransportCommand::SetLoop {
                    range: self.loop_range,
                });
                self.command(TransportCommand::Seek {
                    time: Ticks::new(from.max(0)).unwrap_or(Ticks::ZERO),
                    mode: SeekMode::Exact,
                });
                self.command(TransportCommand::Play { rate });
            }
        }
        true
    }

    /// Opens the device when the composition has something to hear (the mixer
    /// gets the project at Play). Until it does, the next control checks
    /// again.
    fn warm_if_audible(&mut self) {
        if self.playback.is_some() || self.broken || self.doc.root().is_none() {
            return;
        }
        let mut ev = pm_eval::Evaluator::new();
        if pm_audio::graph::audible(&self.doc, &self.composition, &mut ev).unwrap_or(false)
            && self.open()
        {
            self.warmed = true;
        }
    }

    fn command(&mut self, command: TransportCommand) {
        let Some(playback) = &self.playback else {
            return;
        };
        if let Err(error) = playback.command(command) {
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            status.problems.push(format!("transport: {error}"));
            if status.problems.len() > 16 {
                status.problems.remove(0);
            }
        }
    }

    /// Opens the output device. False (and a reported error) when none opens.
    fn open(&mut self) -> bool {
        let rate = pm_audio::mixer::project_sample_rate(&self.doc);
        let cache = SourceCache::new(SOURCE_BUDGET, DECODE_WORKERS);
        match Playback::start(MixFormat::stereo(rate), cache, PlaybackConfig::default()) {
            Ok(playback) => {
                playback.set_evaluator(|| {
                    pm_eval::Evaluator::new()
                        .with_expressions(Arc::new(pm_render::expr::PmExprEngine))
                });
                let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
                status.device = playback.device().map(|d| d.name.clone());
                status.device_rate = Some(playback.device_rate());
                status.latency_us = playback.device().map(|d| d.latency_us());
                status.error = None;
                drop(status);
                *self.clock.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(playback.clock_handle());
                self.playback = Some(playback);
                self.dirty = true;
                true
            }
            Err(error) => {
                self.status.lock().unwrap_or_else(|e| e.into_inner()).error =
                    Some(format!("no audio output: {error}"));
                false
            }
        }
    }

    fn set_project(&mut self) {
        let Some(playback) = &self.playback else {
            return;
        };
        self.last_project = Some(Instant::now());
        if self.broken || self.doc.root().is_none() {
            return;
        }
        self.dirty = false;
        let doc = Arc::new(self.doc.clone());
        match playback.set_project(doc, &self.composition, &self.media) {
            Ok(problems) => {
                let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
                status.projects += 1;
                for (asset, path, message) in problems {
                    status.problems.push(format!(
                        "{asset} ({}): {message}",
                        PathBuf::from(path).display()
                    ));
                }
                let excess = status.problems.len().saturating_sub(16);
                status.problems.drain(..excess);
            }
            Err(error) => {
                let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
                status.problems.push(format!("mix: {error}"));
                let excess = status.problems.len().saturating_sub(16);
                status.problems.drain(..excess);
            }
        }
    }

    fn report(&self) {
        if let Some(playback) = &self.playback {
            let stats = playback.stats();
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            status.underruns = stats.underruns;
            status.starved_renders = stats.starved_renders;
        }
    }
}
