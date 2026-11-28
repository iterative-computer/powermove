//! What a `hang` says about the moment the main thread stopped: the last
//! command, whether playback ran, the front surface and the project. The
//! main thread only stores atomics and swaps a value under a lock; the
//! watchdog reads them when a hang begins (never waiting on a lock), and the
//! telemetry thread counts the project's layers, so nothing here waits on the
//! hung thread.
use crate::ProjectFacts;
use crate::events::{Hang, Surface, count_bucket};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};

static LAST_COMMAND: Mutex<&'static str> = Mutex::new("none");
static PLAYING: AtomicBool = AtomicBool::new(false);
static EDITOR: AtomicU8 = AtomicU8::new(0);
static SETTINGS_KEY: AtomicBool = AtomicBool::new(false);
static PROJECT: Mutex<Option<ProjectFacts>> = Mutex::new(None);

/// A person ran this command (its `command_invoked` id).
pub fn note_command(id: &'static str) {
    if let Ok(mut last) = LAST_COMMAND.lock() {
        *last = id;
    }
}

pub fn note_playing(playing: bool) {
    PLAYING.store(playing, Relaxed);
}

/// What an editor window shows (a project, Home, the Store page).
pub fn note_surface(surface: Surface) {
    let index = Surface::ALL.iter().position(|s| *s == surface).unwrap_or(0);
    EDITOR.store(index as u8, Relaxed);
}

/// The Settings window became key (true) or another window did (false).
pub fn note_settings_key(key: bool) {
    SETTINGS_KEY.store(key, Relaxed);
}

fn surface() -> Surface {
    if SETTINGS_KEY.load(Relaxed) {
        return Surface::Settings;
    }
    Surface::ALL
        .get(EDITOR.load(Relaxed) as usize)
        .copied()
        .unwrap_or_default()
}

pub fn note_project(facts: Option<ProjectFacts>) {
    if let Ok(mut project) = PROJECT.lock() {
        *project = facts;
    }
}

/// Read by the watchdog as the hang begins. `played_recently`: a played
/// frame reached the viewer just before the main thread stopped.
pub struct Context {
    last_command: &'static str,
    playing: bool,
    surface: Surface,
    main_thread_answered: bool,
    project: Option<ProjectFacts>,
}

impl Context {
    /// `main_thread_answered`: the main queue's own heartbeat is fresh
    /// ([`crate::main_thread::since_queue_beat_ms`]).
    pub fn now(played_recently: bool, main_thread_answered: bool) -> Context {
        Context {
            // The main thread holds these locks only for a store; one it
            // holds now (the hang began inside it) reads as unknown.
            last_command: LAST_COMMAND.try_lock().map_or("unknown", |c| *c),
            playing: PLAYING.load(Relaxed) || played_recently,
            surface: surface(),
            main_thread_answered,
            project: PROJECT.try_lock().ok().and_then(|p| p.clone()),
        }
    }

    /// The event, its project counted here (on the telemetry thread).
    pub fn hang(&self, duration: &'static str) -> Hang {
        let facts = self.project.as_ref().and_then(|count| count());
        Hang {
            duration,
            last_command: self.last_command,
            playing: self.playing,
            surface: self.surface,
            main_thread_answered: self.main_thread_answered,
            layer_count: facts.map(|(layers, _)| count_bucket(layers)),
            has_3d: facts.map(|(_, has_3d)| has_3d),
        }
    }
}

#[cfg(test)]
pub fn reset() {
    note_command("none");
    note_playing(false);
    note_surface(Surface::Other);
    note_settings_key(false);
    note_project(None);
}
