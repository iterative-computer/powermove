//! Skimming, as Final Cut Pro does it: hovering over the timeline shows a
//! skimmer (a ghost playhead with its timecode) and the viewer shows the
//! frame under it, with audio grains following the pointer, while the
//! playhead stays where it is. Clicking moves the playhead to the skimmer;
//! Space plays from it. View ▸ Skimming (⌥S) and Audio Skimming (⌥⇧S)
//! turn them off; the choice is the person's, kept in the profile.
use crate::session::Controller;
use gpui::{App, Context};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skimming {
    pub on: bool,
    pub audio: bool,
    /// Where the skimmer is (seconds, on a frame), while the pointer is over
    /// the timeline.
    pub at: Option<f64>,
}
impl Default for Skimming {
    fn default() -> Self {
        Self {
            on: true,
            audio: true,
            at: None,
        }
    }
}

fn prefs_path(profile: &Path) -> PathBuf {
    profile.join("Viewer").join("skimming.json")
}
/// The person's saved choice (`None`: never chosen, the defaults apply).
pub fn load(profile: &Path) -> Option<(bool, bool)> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(prefs_path(profile)).ok()?).ok()?;
    Some((v["skimming"].as_bool()?, v["audioSkimming"].as_bool()?))
}
fn save(profile: PathBuf, on: bool, audio: bool) {
    std::thread::spawn(move || {
        let path = prefs_path(&profile);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(
            path,
            serde_json::json!({ "skimming": on, "audioSkimming": audio }).to_string(),
        );
    });
}

impl Controller {
    /// Reads the saved choice off the main thread and applies it.
    pub fn load_skimming(&mut self, cx: &mut Context<Self>) {
        let profile = self.profile.clone();
        let read = cx
            .background_executor()
            .spawn(async move { load(&profile) });
        cx.spawn(async move |this, cx| {
            if let Some((on, audio)) = read.await {
                let _ = this.update(cx, |c, cx| {
                    c.skimming.on = on;
                    c.skimming.audio = audio;
                    c.apply_skimming(cx);
                });
            }
        })
        .detach();
    }
    /// The skimmer moved to `seconds` (`None`: the pointer left the
    /// timeline). The viewer shows that frame; the playhead stays.
    pub fn skim(&mut self, seconds: Option<f64>, input: f64) {
        let seconds = seconds.filter(|_| self.skimming.on).map(|s| {
            crate::session::frame_start_at(self.rate, s.clamp(0., self.duration())).unwrap_or(0)
                as f64
                / pm_protocol::common::TICKS_PER_SECOND as f64
        });
        if self.skimming.at == seconds {
            return;
        }
        self.skimming.at = seconds;
        if let Some(v) = &self.viewer {
            let time = seconds.and_then(|s| {
                pm_protocol::common::Ticks::new(
                    (s * pm_protocol::common::TICKS_PER_SECOND as f64).round() as i64,
                )
                .ok()
            });
            let _ = v.skim(time, input);
        }
    }
    pub fn toggle_skimming(&mut self, cx: &mut Context<Self>) {
        self.set_skimming(!self.skimming.on, cx);
    }
    /// Settings › General › Timeline › Hover to Scrub, and View ▸ Skimming:
    /// one saved choice.
    pub fn set_skimming(&mut self, on: bool, cx: &mut Context<Self>) {
        self.skimming.on = on;
        save(self.profile.clone(), self.skimming.on, self.skimming.audio);
        self.apply_skimming(cx);
    }
    pub fn toggle_audio_skimming(&mut self, cx: &mut Context<Self>) {
        self.skimming.audio = !self.skimming.audio;
        save(self.profile.clone(), self.skimming.on, self.skimming.audio);
        self.apply_skimming(cx);
    }
    /// Pushes the toggles to the viewer, the timeline and the menu.
    pub fn apply_skimming(&mut self, cx: &mut Context<Self>) {
        if !self.skimming.on {
            self.skim(None, Self::event_time());
        }
        if let Some(v) = &self.viewer {
            let _ = v.skim_audio(self.skimming.on && self.skimming.audio);
        }
        if let Some(timeline) = self.builtins.timeline.clone() {
            // The surface calls back into this controller: after this update.
            cx.defer(move |cx| timeline.update(cx, |t, cx| t.skimming_changed(cx)));
        }
        crate::viewer2::update_skim_checks(self.skimming.on, self.skimming.audio);
        cx.notify();
    }
}

/// Whether `cx`'s controller has skimming on (the timeline's delegate asks).
pub fn on(controller: &gpui::WeakEntity<Controller>, cx: &App) -> bool {
    controller.upgrade().is_some_and(|c| c.read(cx).skimming.on)
}

thread_local! {
    /// A review sweep's pointer time (development builds), standing in for
    /// the NSEvent timestamp a real pointer move has.
    static REVIEW_INPUT: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}
/// The pointer time of the skim being handled.
pub fn input() -> f64 {
    REVIEW_INPUT
        .with(|r| r.get())
        .unwrap_or_else(Controller::event_time)
}
/// Development builds: `powermove://review/skim/<from>/<to>/<seconds>`
/// moves the pointer over the first editor's timeline from one time to
/// another at 60 Hz (through the timeline's own hover handler), rests 0.5 s
/// and leaves: background reviews cannot hover the window.
#[cfg(debug_assertions)]
pub fn review_sweep(editor: gpui::WeakEntity<Controller>, rest: &str, cx: &mut App) {
    let parts: Vec<f64> = rest.split('/').filter_map(|p| p.parse().ok()).collect();
    let [from, to, seconds] = parts[..] else {
        return;
    };
    let Some(timeline) = editor
        .upgrade()
        .and_then(|c| c.read(cx).builtins.timeline.clone())
    else {
        return;
    };
    let timeline = timeline.downgrade();
    cx.spawn(async move |cx| {
        let steps = (seconds * 60.).round().max(1.) as usize;
        let started = std::time::Instant::now();
        for i in 0..=steps {
            let due = started + std::time::Duration::from_secs_f64(i as f64 / 60.);
            let wait = due.saturating_duration_since(std::time::Instant::now());
            cx.background_executor().timer(wait).await;
            let at = from + (to - from) * i as f64 / steps as f64;
            let ok = cx.update(|cx| {
                REVIEW_INPUT.with(|r| r.set(Some(pm_engine_service::trace::now())));
                let ok = timeline
                    .update(cx, |t, cx| t.review_hover(Some(at), cx))
                    .is_ok();
                REVIEW_INPUT.with(|r| r.set(None));
                ok
            });
            if !ok {
                return;
            }
        }
        cx.background_executor()
            .timer(std::time::Duration::from_millis(500))
            .await;
        let _ = cx.update(|cx| timeline.update(cx, |t, cx| t.review_hover(None, cx)));
    })
    .detach();
}
