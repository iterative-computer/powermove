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
            let frame = self.rate.frame_of(
                (s.clamp(0., self.duration) * pm_protocol::common::TICKS_PER_SECOND as f64) as i64,
            );
            self.rate.frame_start(frame).unwrap_or(0) as f64
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
        self.skimming.on = !self.skimming.on;
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
            timeline.update(cx, |t, cx| t.skimming_changed(cx));
        }
        crate::viewer2::update_skim_checks(self.skimming.on, self.skimming.audio);
        cx.notify();
    }
}

/// Whether `cx`'s controller has skimming on (the timeline's delegate asks).
pub fn on(controller: &gpui::WeakEntity<Controller>, cx: &App) -> bool {
    controller.upgrade().is_some_and(|c| c.read(cx).skimming.on)
}
