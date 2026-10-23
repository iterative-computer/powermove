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
//!
//! J/K/L, play around, and audio scrub (section 15) go to pm-audio's
//! transport; the mixer's monitor solo follows the renderer's mixer panel,
//! and the levels of what is heard go back to it about 30 times a second
//! while playing ([`LEVELS_INTERVAL`]).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pm_audio::playback::{ClockHandle, CommandReceipt, Playback, PlaybackConfig};
use pm_audio::{MixFormat, SourceCache};
use pm_engine::document::DocumentUpdate;
use pm_protocol::clock::{AudioLevels, SeekMode, TransportCommand, TransportState};
use pm_protocol::common::{PlaybackRate, TickRange, Ticks};
use serde::Serialize;

use super::document::MediaMap;

/// The shortest interval between two mixes of new revisions.
pub const PROJECT_INTERVAL: Duration = Duration::from_millis(30);

/// Levels go to the renderer this often while playing (the mixer panel
/// draws at the display rate and steps its own ballistics in between).
pub const LEVELS_INTERVAL: Duration = Duration::from_millis(33);

/// Bound queued transport answers when a producer stalls behind an earlier command.
const MAX_PENDING_COMMANDS: usize = 64;

/// Where the levels of what is heard go (the renderer's port).
pub type LevelsSink = Box<dyn Fn(AudioLevels) + Send>;
/// How long the controls must rest before a warmed thread checks whether
/// the composition is audible and opens the device.
pub const WARM_IDLE: Duration = Duration::from_millis(100);

/// Decoded audio kept per viewer.
const SOURCE_BUDGET: usize = 128 * 1024 * 1024;
const DECODE_WORKERS: usize = 2;

// The audio control thread is ordinary-priority. Keep these clock reads and
// the diagnostic write out of pm-audio's real-time producer and callback.
fn control_profile_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("PM_AUDIO_PROFILE").is_some())
}

fn play_receive_stamp(ctl: &AudioCtl) -> Option<u64> {
    (control_profile_enabled()
        && matches!(
            ctl,
            AudioCtl::Play { .. } | AudioCtl::Transport(TransportCommand::Play { .. })
        ))
    .then(pm_audio::clock::host_now_us)
}

#[derive(Clone, Copy, Default)]
struct ProjectTiming {
    start_us: u64,
    clone_done_us: u64,
    build_send_done_us: u64,
    status_done_us: u64,
}

pub enum AudioCtl {
    Document(DocumentUpdate),
    MemoryPressure,
    /// Transport forwarded at presenter ingress, before render/document work.
    Transport(TransportCommand),
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
    /// A scrub seek while paused: a grain at `ticks` when audio scrub is on.
    Scrub(i64),
    /// Play around `at` (ticks): from `pre` before to `post` after, then
    /// back to `at`.
    PlayAround {
        at: i64,
        pre: Ticks,
        post: Ticks,
    },
    AudioScrub(bool),
    /// The mixer panel's monitor solo (strip ids; `None`: every strip).
    Solo(Option<HashSet<String>>),
    SetLoop(Option<TickRange>),
    Shutdown,
}

/// What the audio thread reports (the engine's status).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    /// Presenter ingress, before document, surface, and DSP preparation work.
    pub ingress_request_us: Option<u64>,
    pub device: Option<String>,
    pub device_rate: Option<u32>,
    pub latency_us: Option<u32>,
    pub error: Option<String>,
    pub projects: u64,
    pub underruns: u64,
    /// Output device cycles that overran their real-time deadline.
    pub device_overloads: u64,
    pub device_callbacks: u64,
    pub starved_renders: u64,
    pub problems: Vec<String>,
    pub start_timing: Option<pm_audio::playback::StartTiming>,
    pub prepared_starts: Vec<i64>,
}

/// FIFO ingress shared with the presenter. It holds senders, never render
/// state; the audio producer remains the sole owner of the playback clock.
#[derive(Clone, Default)]
pub struct AudioIngress(Arc<Mutex<BTreeMap<super::Key, Endpoint>>>);

struct Endpoint {
    tx: Sender<AudioCtl>,
    clock: Arc<Mutex<Option<ClockHandle>>>,
    status: Arc<Mutex<AudioStatus>>,
    enabled: bool,
    sent_revision: u64,
    pending_revision: u64,
    last_play: Option<(PlaybackRate, Option<u32>)>,
}

#[derive(Clone, Copy, Debug)]
pub struct ForwardedAudio {
    pub before: Option<u32>,
}

impl AudioIngress {
    pub fn register(&self, key: super::Key, link: &AudioLink, enabled: bool, revision: u64) {
        let mut endpoints = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(endpoint) = endpoints.get_mut(&key) {
            endpoint.enabled = enabled;
        } else {
            endpoints.insert(
                key,
                Endpoint {
                    tx: link.tx.clone(),
                    clock: link.clock.clone(),
                    status: link.status.clone(),
                    enabled,
                    sent_revision: revision,
                    pending_revision: revision,
                    last_play: None,
                },
            );
        }
    }
    /// A document sent through the render fallback has now reached audio.
    pub fn document_sent(&self, key: super::Key, revision: u64) {
        if let Some(endpoint) = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&key)
        {
            endpoint.sent_revision = endpoint.sent_revision.max(revision);
        }
    }
    pub fn forget(&self, key: super::Key) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
    }
    pub fn transport(&self, key: super::Key, command: TransportCommand) -> Option<ForwardedAudio> {
        if matches!(command, TransportCommand::Step { .. }) {
            return None;
        }
        let mut endpoints = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let endpoint = endpoints
            .get_mut(&key)
            .filter(|e| e.enabled && e.sent_revision >= e.pending_revision)?;
        let sample = endpoint
            .clock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(ClockHandle::latest);
        let before = sample.as_ref().map(|s| s.epoch);
        match command {
            TransportCommand::Play { rate } => {
                if endpoint.last_play.is_some_and(|(r, epoch)| {
                    r == rate
                        && (epoch == before
                            || sample
                                .as_ref()
                                .is_some_and(|s| s.state != TransportState::Stopped))
                }) {
                    return Some(ForwardedAudio { before });
                }
                endpoint.last_play = Some((rate, before));
            }
            TransportCommand::Pause => endpoint.last_play = None,
            _ => {}
        }
        if matches!(
            command,
            TransportCommand::Play { .. } | TransportCommand::PlayAround { .. }
        ) {
            endpoint
                .status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .ingress_request_us = Some(pm_audio::clock::host_now_us());
        }
        endpoint.tx.send(AudioCtl::Transport(command)).ok()?;
        Some(ForwardedAudio { before })
    }
    pub fn document(&self, key: super::Key, update: &DocumentUpdate) -> bool {
        let mut endpoints = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(endpoint) = endpoints.get_mut(&key) else {
            return false;
        };
        let revision = update.revision().get();
        if revision <= endpoint.sent_revision {
            return true;
        }
        // A link can appear while older render-thread document commands are
        // queued. Do not let fast controls overtake those fallback updates.
        if revision != endpoint.sent_revision + 1
            || endpoint.pending_revision > endpoint.sent_revision
        {
            endpoint.pending_revision = endpoint.pending_revision.max(revision);
            return false;
        }
        if endpoint
            .tx
            .send(AudioCtl::Document(update.clone()))
            .is_err()
        {
            return false;
        }
        endpoint.sent_revision = revision;
        true
    }

    #[cfg(test)]
    pub(super) fn test_endpoint(&self, key: super::Key) -> Receiver<AudioCtl> {
        let (tx, rx) = channel();
        self.0.lock().unwrap().insert(
            key,
            Endpoint {
                tx,
                clock: Arc::default(),
                status: Arc::default(),
                enabled: true,
                sent_revision: 0,
                pending_revision: 0,
                last_play: None,
            },
        );
        rx
    }
}

pub struct AudioLink {
    tx: Sender<AudioCtl>,
    clock: Arc<Mutex<Option<ClockHandle>>>,
    status: Arc<Mutex<AudioStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioLink {
    /// Starts the control thread with the view's document and media;
    /// `levels` receives the levels of what is heard while playing.
    pub fn start(
        doc: pm_model::Document,
        composition: String,
        media: MediaMap,
        levels: LevelsSink,
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
                    doc: Arc::new(doc),
                    composition,
                    media,
                    playback: None,
                    pending_commands: VecDeque::new(),
                    dirty: true,
                    broken: false,
                    loop_range: None,
                    last_project: None,
                    audio_scrub: false,
                    solo: None,
                    levels,
                    last_levels: None,
                    levels_running: false,
                    watch_until: None,
                    warm: false,
                    warm_due: None,
                    paused_at: 0,
                    transport_running: false,
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
        if matches!(&ctl, AudioCtl::Play { .. } | AudioCtl::PlayAround { .. }) {
            self.status
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .ingress_request_us = Some(pm_audio::clock::host_now_us());
        }
        let sent_us = (control_profile_enabled() && matches!(&ctl, AudioCtl::Play { .. }))
            .then(pm_audio::clock::host_now_us);
        let _ = self.tx.send(ctl);
        if let Some(sent_us) = sent_us {
            eprintln!("pm-audio-node-send sent_us={sent_us}");
        }
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

struct PendingCommand {
    receipt: CommandReceipt,
    pause: bool,
}

struct Control {
    rx: Receiver<AudioCtl>,
    clock: Arc<Mutex<Option<ClockHandle>>>,
    status: Arc<Mutex<AudioStatus>>,
    /// Shared with the mixer after a project goes out; unshared (copied)
    /// while the controls rest, so neither an edit nor a Play waits for the
    /// copy ([`Control::unshare_document`]).
    doc: Arc<pm_model::Document>,
    composition: String,
    media: MediaMap,
    playback: Option<Playback>,
    /// Individually answered Pause, SetLoop, and exact Seek commands. Their
    /// FIFO application is guaranteed by the producer queue even across
    /// separate control-thread receive batches.
    pending_commands: VecDeque<PendingCommand>,
    /// The mixer plays an older revision, media map, or composition.
    dirty: bool,
    /// An update did not apply; wait for a resync.
    broken: bool,
    loop_range: Option<TickRange>,
    last_project: Option<Instant>,
    audio_scrub: bool,
    solo: Option<HashSet<String>>,
    levels: LevelsSink,
    last_levels: Option<Instant>,
    /// Levels went out while playing; one silent message follows the stop.
    levels_running: bool,
    /// After a play, a scrub, or a play around, the device starts within the
    /// pre-roll and the output latency: watch for its levels until then.
    watch_until: Option<Instant>,
    /// Asked to open the device ahead of Play ([`AudioCtl::Warm`]).
    warm: bool,
    /// When to check, once the controls rested ([`WARM_IDLE`]).
    warm_due: Option<Instant>,
    /// Latest paused playhead, including seeks before the device opens.
    paused_at: i64,
    transport_running: bool,
}

impl Control {
    fn run(mut self) {
        loop {
            // Feed warmed devices while paused too: their mixers decode and
            // pre-roll at the playhead before the first Play arrives.
            let feeding = self.playback.is_some();
            let timeout = match (feeding, self.dirty, self.last_project) {
                (true, true, Some(last)) => PROJECT_INTERVAL.saturating_sub(last.elapsed()),
                (true, true, None) => Duration::ZERO,
                _ if self.playback.is_none() && self.warm_due.is_some() => {
                    self.warm_due.map_or(Duration::ZERO, |due| {
                        due.saturating_duration_since(Instant::now())
                    })
                }
                // Paused preparation and device health continue changing
                // without controls or levels; keep diagnostics current too.
                _ if feeding => LEVELS_INTERVAL,
                _ => Duration::from_secs(3600),
            };
            // Levels while the device plays (or is about to).
            let watching = self.watch_until.is_some_and(|w| Instant::now() < w);
            let timeout = if self.running() || self.levels_running || watching {
                timeout.min(self.last_levels.map_or(Duration::ZERO, |l| {
                    LEVELS_INTERVAL.saturating_sub(l.elapsed())
                }))
            } else {
                timeout
            };
            let first = match self.rx.recv_timeout(timeout) {
                Ok(ctl) => Some(ctl),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let mut controls: Vec<(AudioCtl, Option<u64>)> = first
                .into_iter()
                .map(|ctl| {
                    let received_us = play_receive_stamp(&ctl);
                    (ctl, received_us)
                })
                .collect();
            while let Ok(ctl) = self.rx.try_recv() {
                let received_us = play_receive_stamp(&ctl);
                controls.push((ctl, received_us));
            }
            let rested = controls.is_empty();
            let starting = controls.iter().any(|(c, _)| {
                matches!(
                    c,
                    AudioCtl::Play { .. }
                        | AudioCtl::PlayAround { .. }
                        | AudioCtl::Transport(
                            TransportCommand::Play { .. } | TransportCommand::PlayAround { .. }
                        )
                )
            });
            let _priority = starting.then(pm_audio::playback::StartPriority::new);
            for (ctl, received_us) in controls {
                if !self.apply(ctl, received_us) {
                    self.shutdown();
                    return;
                }
            }
            self.drain_pending_commands();
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
            if self.playback.is_some() && self.dirty && due {
                let _ = self.set_project();
            }
            self.send_levels();
            self.report();
            if rested || !starting {
                self.unshare_document();
            }
        }
        self.shutdown();
    }

    /// Copies the document the mixer shares, between controls, so the next
    /// edit applies in place: a long project's copy takes milliseconds,
    /// which an edit and the Play right after it would otherwise wait for.
    fn unshare_document(&mut self) {
        if Arc::strong_count(&self.doc) > 1 {
            Arc::make_mut(&mut self.doc);
        }
    }

    /// Whether the device plays a run (or a grain).
    fn running(&self) -> bool {
        self.playback
            .as_ref()
            .and_then(|p| p.clock())
            .is_some_and(|c| c.state != TransportState::Stopped)
    }

    /// The levels of what is heard, at most every [`LEVELS_INTERVAL`]; once
    /// silent after a run stops.
    fn send_levels(&mut self) {
        let Some(playback) = &self.playback else {
            return;
        };
        if self
            .last_levels
            .is_some_and(|l| l.elapsed() < LEVELS_INTERVAL)
        {
            return;
        }
        let running = self.running();
        // A queued Pause has not necessarily reached the producer yet. Do
        // not publish another running meter frame while its ACK is pending.
        if running && self.pending_commands.iter().any(|pending| pending.pause) {
            return;
        }
        if !running && !self.levels_running {
            return;
        }
        let Some(meters) = playback.meters() else {
            return;
        };
        let meters = if running { meters } else { meters.silent() };
        (self.levels)(meters.to_levels(playback.format().sample_rate));
        self.last_levels = Some(Instant::now());
        self.levels_running = running;
    }

    fn shutdown(&mut self) {
        *self.clock.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(mut playback) = self.playback.take() {
            playback.shutdown();
        }
        // Shutdown is queued after the receipts, so every earlier command has
        // either answered or its sender disconnected by the time the join ends.
        self.drain_pending_commands();
    }

    /// Returns false on shutdown.
    fn apply(&mut self, ctl: AudioCtl, received_us: Option<u64>) -> bool {
        match ctl {
            AudioCtl::Transport(command) => {
                match command {
                    TransportCommand::Play { .. } | TransportCommand::PlayAround { .. } => {
                        if self.playback.is_none() && !self.open() {
                            return true;
                        }
                        if self.dirty {
                            let _ = self.set_project();
                        }
                        // Seek/pause/loop controls have already reached the
                        // producer through this same FIFO. Play its own current
                        // playhead rather than a delayed render-thread estimate.
                        self.transport_running = true;
                        self.commands(&[
                            TransportCommand::SetLoop {
                                range: self.loop_range,
                            },
                            command,
                        ]);
                    }
                    TransportCommand::Seek {
                        time,
                        mode: SeekMode::Scrub,
                    } => {
                        let seek = if self.transport_running {
                            AudioCtl::Seek(time.get())
                        } else {
                            AudioCtl::Scrub(time.get())
                        };
                        return self.apply(seek, received_us);
                    }
                    TransportCommand::Pause => return self.apply(AudioCtl::Pause, received_us),
                    TransportCommand::Seek { time, .. } => {
                        return self.apply(AudioCtl::Seek(time.get()), received_us);
                    }
                    TransportCommand::SetLoop { range } => {
                        return self.apply(AudioCtl::SetLoop(range), received_us);
                    }
                    TransportCommand::AudioScrub { enabled } => {
                        return self.apply(AudioCtl::AudioScrub(enabled), received_us);
                    }
                    _ => self.enqueue_simple_command(command),
                }
            }
            AudioCtl::Shutdown => return false,
            AudioCtl::MemoryPressure => {
                if let Some(playback) = &self.playback {
                    playback.memory_pressure();
                }
            }
            AudioCtl::Document(update) => {
                let replace = matches!(update, DocumentUpdate::Replace { .. });
                if self.broken && !replace {
                    return true;
                }
                self.broken = update.apply_to(Arc::make_mut(&mut self.doc)).is_err();
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
                self.enqueue_simple_command(TransportCommand::SetLoop { range });
            }
            AudioCtl::Pause => {
                self.transport_running = false;
                self.enqueue_simple_command(TransportCommand::Pause);
            }
            AudioCtl::Warm => self.warm = true,
            AudioCtl::Seek(at) => {
                self.paused_at = at.max(0);
                self.enqueue_simple_command(TransportCommand::Seek {
                    time: Ticks::new(self.paused_at).unwrap_or(Ticks::ZERO),
                    mode: SeekMode::Exact,
                });
            }
            AudioCtl::Scrub(at) => {
                self.paused_at = at.max(0);
                if !self.audio_scrub {
                    // Paused timeline moves must still reach the warm mixer.
                    // This is an ordinary exact seek, so preserve FIFO without
                    // waiting for its ACK before the following Play is sent.
                    self.enqueue_simple_command(TransportCommand::Seek {
                        time: Ticks::new(at.max(0)).unwrap_or(Ticks::ZERO),
                        mode: SeekMode::Exact,
                    });
                    return true;
                }
                if self.playback.is_none() && !self.open() {
                    return true;
                }
                if self.dirty {
                    let _ = self.set_project();
                }
                self.command(TransportCommand::Seek {
                    time: Ticks::new(at.max(0)).unwrap_or(Ticks::ZERO),
                    mode: SeekMode::Scrub,
                });
            }
            AudioCtl::PlayAround { at, pre, post } => {
                self.transport_running = true;
                if self.playback.is_none() && !self.open() {
                    return true;
                }
                if self.dirty {
                    let _ = self.set_project();
                }
                self.command(TransportCommand::Seek {
                    time: Ticks::new(at.max(0)).unwrap_or(Ticks::ZERO),
                    mode: SeekMode::Exact,
                });
                self.command(TransportCommand::PlayAround {
                    pre_roll: pre,
                    post_roll: post,
                });
            }
            AudioCtl::AudioScrub(enabled) => {
                self.audio_scrub = enabled;
                self.command(TransportCommand::AudioScrub { enabled });
            }
            AudioCtl::Solo(solo) => {
                self.solo = solo.clone();
                if let Some(playback) = &self.playback {
                    playback.set_monitor_solo(solo);
                }
            }
            AudioCtl::Play { from, rate } => {
                self.transport_running = true;
                let profiled = received_us.is_some();
                let begin_us = profiled.then(pm_audio::clock::host_now_us).unwrap_or(0);
                let opening = self.playback.is_none();
                if self.playback.is_none() && !self.open() {
                    return true;
                }
                let open_done_us = profiled.then(pm_audio::clock::host_now_us).unwrap_or(0);
                self.paused_at = from.max(0);
                let dirty = self.dirty;
                let project = dirty.then(|| self.set_project()).flatten();
                let dispatch_us = profiled.then(pm_audio::clock::host_now_us).unwrap_or(0);
                self.commands(&[
                    TransportCommand::SetLoop {
                        range: self.loop_range,
                    },
                    TransportCommand::Seek {
                        time: Ticks::new(from.max(0)).unwrap_or(Ticks::ZERO),
                        mode: SeekMode::Exact,
                    },
                    TransportCommand::Play { rate },
                ]);
                if let Some(received_us) = received_us {
                    let ack_us = pm_audio::clock::host_now_us();
                    let project = project.unwrap_or_default();
                    eprintln!(
                        "pm-audio-node-play received_us={received_us} begin_us={begin_us} open_done_us={open_done_us} project_start_us={} clone_done_us={} build_send_done_us={} status_done_us={} dispatch_us={dispatch_us} ack_us={ack_us} dirty={dirty} opened={opening}",
                        project.start_us,
                        project.clone_done_us,
                        project.build_send_done_us,
                        project.status_done_us,
                    );
                }
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
            let _ = self.set_project();
            self.command(TransportCommand::SetLoop {
                range: self.loop_range,
            });
            self.command(TransportCommand::Seek {
                time: Ticks::new(self.paused_at).unwrap_or(Ticks::ZERO),
                mode: SeekMode::Exact,
            });
        }
    }

    fn command(&mut self, command: TransportCommand) {
        self.commands(&[command]);
    }

    fn enqueue_simple_command(&mut self, command: TransportCommand) {
        let Some(playback) = &self.playback else {
            return;
        };
        let pause = matches!(command, TransportCommand::Pause);
        if self.pending_commands.len() == MAX_PENDING_COMMANDS {
            let result = self
                .pending_commands
                .pop_front()
                .expect("bounded receipts")
                .receipt
                .wait();
            self.record_transport_result(result);
        }
        match playback.enqueue_command(command) {
            Ok(receipt) => self
                .pending_commands
                .push_back(PendingCommand { receipt, pause }),
            Err(error) => {
                self.drain_pending_commands();
                self.record_transport_result(Err(error));
            }
        }
        self.drain_pending_commands();
    }

    fn drain_pending_commands(&mut self) {
        while let Some(result) = self
            .pending_commands
            .front()
            .and_then(|pending| pending.receipt.try_result())
        {
            self.pending_commands.pop_front();
            self.record_transport_result(result);
        }
    }

    fn record_transport_result(&self, result: pm_audio::Result<()>) {
        if let Err(error) = result {
            let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
            let problem = format!("transport: {error}");
            if !status.problems.contains(&problem) {
                status.problems.push(problem);
                if status.problems.len() > 16 {
                    status.problems.remove(0);
                }
            }
        }
    }

    fn commands(&mut self, commands: &[TransportCommand]) {
        let Some(playback) = &self.playback else {
            return;
        };
        if commands.iter().any(|command| {
            matches!(
                command,
                TransportCommand::Play { .. }
                    | TransportCommand::PlayAround { .. }
                    | TransportCommand::Seek {
                        mode: SeekMode::Scrub,
                        ..
                    }
            )
        }) {
            self.watch_until = Some(Instant::now() + Duration::from_millis(1500));
        }
        let result = playback.commands(commands.iter().copied());
        // This ACK is a FIFO barrier for all earlier queued simple controls;
        // report their individual failures before this command's failure.
        self.drain_pending_commands();
        self.record_transport_result(result);
    }

    /// Opens the output device. False (and a reported error) when none opens.
    fn open(&mut self) -> bool {
        let rate = pm_audio::mixer::project_sample_rate(&self.doc);
        let cache = SourceCache::new(SOURCE_BUDGET, DECODE_WORKERS);
        match Playback::start(MixFormat::stereo(rate), cache, PlaybackConfig::default()) {
            Ok(playback) => {
                playback.set_standard_evaluator(|| {
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
                playback.set_monitor_solo(self.solo.clone());
                let _ = playback.command(TransportCommand::AudioScrub {
                    enabled: self.audio_scrub,
                });
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

    fn set_project(&mut self) -> Option<ProjectTiming> {
        let Some(playback) = &self.playback else {
            return None;
        };
        let profiled = control_profile_enabled();
        let mut timing = ProjectTiming {
            start_us: profiled.then(pm_audio::clock::host_now_us).unwrap_or(0),
            ..ProjectTiming::default()
        };
        self.last_project = Some(Instant::now());
        if self.broken || self.doc.root().is_none() {
            return profiled.then_some(timing);
        }
        self.dirty = false;
        let doc = self.doc.clone();
        timing.clone_done_us = profiled.then(pm_audio::clock::host_now_us).unwrap_or(0);
        let result = playback.set_project(doc, &self.composition, &self.media);
        timing.build_send_done_us = profiled.then(pm_audio::clock::host_now_us).unwrap_or(0);
        match result {
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
        timing.status_done_us = profiled.then(pm_audio::clock::host_now_us).unwrap_or(0);
        profiled.then_some(timing)
    }

    fn report(&self) {
        if let Some(playback) = &self.playback {
            report_playback(&self.status, playback);
        }
    }
}

/// Copies a playback's counters and new render errors into the status
/// (every [`LEVELS_INTERVAL`]). A message already listed is not listed
/// again, so a recurring error never pushes the others out of the 16.
fn report_playback(status: &Mutex<AudioStatus>, playback: &Playback) {
    let stats = playback.stats();
    let errors = playback.take_errors();
    let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
    status.underruns = stats.underruns;
    status.device_overloads = stats.device_overloads;
    status.device_callbacks = stats.device_callbacks;
    status.starved_renders = stats.starved_renders;
    status.start_timing = playback.start_timing();
    status.prepared_starts = playback.prepared_starts();
    for error in errors {
        if !status.problems.contains(&error) {
            status.problems.push(error);
        }
    }
    let excess = status.problems.len().saturating_sub(16);
    status.problems.drain(..excess);
}

#[cfg(test)]
mod tests {
    #[test]
    fn ingress_keeps_audio_documents_and_transport_fifo_without_a_render_thread() {
        use pm_protocol::common::{Revision, SessionId};
        let key = (pm_protocol::host::PortId(1), SessionId::new(1).unwrap());
        let ingress = super::AudioIngress::default();
        let rx = ingress.test_endpoint(key);
        let update = |revision| pm_engine::document::DocumentUpdate::Replace {
            root: None,
            revision: Revision::new(revision).unwrap(),
        };
        assert!(ingress.document(key, &update(1)));
        assert!(
            ingress
                .transport(
                    key,
                    TransportCommand::Play {
                        rate: PlaybackRate::NORMAL
                    }
                )
                .is_some()
        );
        assert!(
            ingress
                .transport(
                    key,
                    TransportCommand::Play {
                        rate: PlaybackRate::NORMAL
                    }
                )
                .is_some()
        );
        assert!(matches!(rx.recv().unwrap(), super::AudioCtl::Document(_)));
        assert!(matches!(
            rx.recv().unwrap(),
            super::AudioCtl::Transport(TransportCommand::Play { .. })
        ));
        assert!(
            rx.try_recv().is_err(),
            "duplicate Play must not restart preparation"
        );
        // A link appeared with an older document still in the render queue.
        // No newer fast control may jump ahead of that fallback update.
        assert!(!ingress.document(key, &update(3)));
        assert!(ingress.transport(key, TransportCommand::Pause).is_none());
        ingress.document_sent(key, 2);
        assert!(ingress.transport(key, TransportCommand::Pause).is_none());
        ingress.document_sent(key, 3);
        assert!(ingress.transport(key, TransportCommand::Pause).is_some());
        ingress.forget(key);
        assert!(ingress.transport(key, TransportCommand::Pause).is_none());
    }

    use std::path::Path;

    use pm_audio::offline::{PcmFormat, WavWriter};
    use pm_model::json::parse;

    use super::*;

    fn write_wav(path: &Path, rate: u32, planar: &[Vec<f32>]) {
        let frames = planar[0].len();
        let inter: Vec<f32> = (0..frames)
            .flat_map(|i| planar.iter().map(move |c| c[i]))
            .collect();
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

    fn clip(id: &str, asset: &str) -> String {
        format!(
            r#"{{"id":"{id}","type":"audio","name":"{id}","from":0,"dur":1,"on":true,"solo":false,"group":null,"parent":null,"p":{{}},"fx":[],"masks":[],"d":{{"asset":"{asset}","gain":1,"trim":0,"fadeIn":0,"fadeOut":0}}}}"#
        )
    }

    #[test]
    fn queued_controls_report_ordered_errors_and_shutdown_drains() {
        struct ShutdownCache(Arc<SourceCache>);
        impl Drop for ShutdownCache {
            fn drop(&mut self) {
                self.0.shutdown();
            }
        }

        let cache = SourceCache::new(1 << 20, 0);
        let _decode_workers = ShutdownCache(cache.clone());
        let (playback, _side) = Playback::start_headless(
            MixFormat::stereo(48_000),
            cache,
            PlaybackConfig::default(),
            48_000,
            2,
            0,
        )
        .unwrap();
        let (_tx, rx) = channel();
        let status = Arc::new(Mutex::new(AudioStatus {
            problems: vec!["media: missing".into()],
            ..AudioStatus::default()
        }));
        let mut control = Control {
            rx,
            clock: Arc::new(Mutex::new(None)),
            status: status.clone(),
            doc: Arc::new(pm_model::Document::new(None, 1)),
            composition: "C".into(),
            media: MediaMap::default(),
            playback: Some(playback),
            pending_commands: VecDeque::new(),
            dirty: false,
            broken: false,
            loop_range: None,
            last_project: None,
            audio_scrub: false,
            solo: None,
            levels: Box::new(|_| {}),
            last_levels: None,
            levels_running: false,
            watch_until: None,
            warm: false,
            warm_due: None,
            paused_at: 0,
            transport_running: false,
        };
        let invalid_loop = TransportCommand::SetLoop {
            range: Some(TickRange {
                start: Ticks::ZERO,
                end: Ticks::ZERO,
            }),
        };
        control.enqueue_simple_command(invalid_loop);
        // The frontend requests a scrub for a paused timeline move even
        // with audible scrubbing disabled. It must enqueue the exact seek
        // without waiting for its own or the earlier control's receipt.
        assert!(control.apply(AudioCtl::Scrub(Ticks::from_whole_seconds(1).get()), None));
        assert_eq!(control.paused_at, Ticks::from_whole_seconds(1).get());
        control.enqueue_simple_command(invalid_loop);
        control.commands(&[TransportCommand::Play {
            rate: PlaybackRate { num: 3, den: 1 },
        }]);

        assert!(control.pending_commands.is_empty());
        let problems = status.lock().unwrap().problems.clone();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(problems[0], "media: missing");
        assert!(problems[1].contains("loop range must be non-empty"));
        assert!(problems[2].contains("playback at 3/1"));
        assert_eq!(
            control
                .playback
                .as_ref()
                .unwrap()
                .clock()
                .unwrap()
                .media_time,
            Ticks::from_whole_seconds(1),
        );

        // Shutdown joins the producer before draining even a receipt that
        // was deliberately left in the queue without another control poll.
        status.lock().unwrap().problems.clear();
        let receipt = control
            .playback
            .as_ref()
            .unwrap()
            .enqueue_command(invalid_loop)
            .unwrap();
        control.pending_commands.push_back(PendingCommand {
            receipt,
            pause: false,
        });
        control.shutdown();
        assert!(control.pending_commands.is_empty());
        let problems = status.lock().unwrap().problems.clone();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("loop range must be non-empty"));
    }

    /// The status polls every 33 ms while a bad block fails each segment
    /// and a stuck decoder drops its voice (A3): each message is listed
    /// once, and a problem listed before (a media resolve) stays.
    #[test]
    fn recurring_render_errors_are_listed_once() {
        struct ShutdownCache(Arc<SourceCache>);
        impl Drop for ShutdownCache {
            fn drop(&mut self) {
                self.0.shutdown();
            }
        }

        let rate = 48_000u32;
        let directory = tempfile::Builder::new()
            .prefix("pm-node-audio-problems-")
            .tempdir()
            .unwrap();
        let dir = directory.path();
        let (good_file, bad_file, late_file) = (
            dir.join("good.wav"),
            dir.join("bad.wav"),
            dir.join("late.wav"),
        );
        let n = rate as usize;
        write_wav(&good_file, rate, &[vec![0.25; n], vec![0.0; n]]);
        write_wav(&bad_file, rate, &[vec![0.3; n], vec![0.3; n]]);
        write_wav(&late_file, rate, &[vec![0.0; n], vec![0.2; n]]);
        let cache = SourceCache::new(64 << 20, 2);
        let _decode_workers = ShutdownCache(cache.clone());
        let good = cache.open(&good_file, None).unwrap();
        cache.block(&good, 0, true).unwrap();
        let bad = cache.open(&bad_file, None).unwrap();
        cache.set_decode_failing(&bad, true);
        let late = cache.open(&late_file, None).unwrap();
        cache.set_decode_blocked(&late, true);
        let (playback, mut side) = Playback::start_headless(
            MixFormat::stereo(rate),
            cache.clone(),
            PlaybackConfig {
                preroll_ms: 0,
                preroll_wait_ms: 0,
                ..PlaybackConfig::default()
            },
            rate,
            2,
            5_000,
        )
        .unwrap();
        let text = format!(
            r#"{{"id":"P","compId":"C","name":"T","w":320,"h":180,"fps":30,"dur":1,"layers":[{},{},{}],"comps":{{}},"assets":{{}},"params":{{}}}}"#,
            clip("good", "g"),
            clip("bad", "b"),
            clip("late", "s"),
        );
        let doc = pm_model::Document::new(Some(parse(&text).unwrap()), 1);
        let resolver = |id: &str| {
            Some(match id {
                "g" => good_file.clone(),
                "b" => bad_file.clone(),
                _ => late_file.clone(),
            })
        };
        assert!(
            playback
                .set_project(Arc::new(doc), "C", &resolver)
                .unwrap()
                .is_empty()
        );
        let status = Mutex::new(AudioStatus {
            problems: vec!["m (/media/m.wav): file not found".into()],
            ..AudioStatus::default()
        });
        playback
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .unwrap();
        let host_start = pm_audio::clock::host_now_us();
        let mut out = vec![0.0; 256 * 2];
        let mut frame = 0u64;
        let mut heard = false;
        // Half a second of device time past the 120 ms bound, one status
        // report per buffer (far more often than the real 33 ms).
        while frame < rate as u64 / 2 {
            side.callback(&mut out, 2, host_start + frame * 1_000_000 / rate as u64);
            heard |= out.chunks_exact(2).any(|p| p[0] == 0.25);
            frame += 256;
            report_playback(&status, &playback);
            std::thread::sleep(Duration::from_millis(1));
        }
        cache.set_decode_blocked(&late, false);
        let status = status.into_inner().unwrap();
        assert!(heard, "the good voice plays on");
        assert!(status.starved_renders > 0);
        let problems = &status.problems;
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(problems[0], "m (/media/m.wav): file not found");
        assert_eq!(
            problems.iter().filter(|p| p.starts_with("bad:")).count(),
            1,
            "{problems:?}"
        );
        assert_eq!(
            problems
                .iter()
                .filter(|p| p.contains("exceeded 120 ms") && p.ends_with(": late"))
                .count(),
            1,
            "{problems:?}"
        );
        drop(playback);
    }
}
