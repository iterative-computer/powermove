//! The first-run film's cue sheet: every beat of the stage is a time on this
//! sheet (scene in-points, waves, the sound), and the stage reads nothing
//! else for timing. Pure data, so the film is tested without a screen.
//!
//! Music: `CueSheet::music` names the licensed track's slot; the default is
//! silent. The sheet is cut for a 20 s piece at 100 BPM in 4/4 (one bar is
//! 2.4 s; each change takes a bar and a half); `retimed` scales it.
use pm_wave::WaveType;

/// One picture on the stage, in film order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scene {
    /// The desktop dims under a black veil; nothing else yet.
    HouseLights,
    /// Light gathers and the mark resolves out of it.
    Mark,
    /// The mark folds into the agent's composer, alone on the stage.
    Composer,
    /// The real editor, and one change asked for in the composer: typed,
    /// sent, and carried onto the surface it changes by the wave.
    Beat(Change),
    /// The calm card with one primary action; the first-run steps live here.
    Card,
}
/// The changes the film asks the agent for, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    GlassTimeline,
    LightLeak,
    BeatMarkers,
    ClassicLayout,
}
impl Change {
    pub const ALL: [Change; 4] = [
        Change::GlassTimeline,
        Change::LightLeak,
        Change::BeatMarkers,
        Change::ClassicLayout,
    ];
    /// What is typed into the composer. OWNER-COPY: the prompts.
    pub fn prompt(self) -> &'static str {
        match self {
            Change::GlassTimeline => "Make the timeline feel like glass",
            Change::LightLeak => "Write me a light-leak transition",
            Change::BeatMarkers => "Build me a beat-markers panel",
            Change::ClassicLayout => "Put my tools where After Effects had them",
        }
    }
    /// How many changes come before this one.
    pub fn index(self) -> usize {
        Change::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }
}
/// How a scene leaves for the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    /// The mark folds into the composer over `seconds` (drawn, no wave:
    /// one object changing shape).
    Fold { seconds: f64 },
    /// The distortion wave across the whole picture, from the frozen
    /// outgoing frame into the first frame of the next scene.
    Wave(WaveType),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    pub at: f64,
    pub scene: Scene,
    /// What carries this scene into the next one (the card's own steps use
    /// `CueSheet::step_wave`).
    pub out: Option<Transition>,
    /// What VoiceOver reads when the scene starts.
    pub title: &'static str,
}
/// A sound the film plays at a time (the mark's chime).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sound {
    pub at: f64,
    pub id: &'static str,
}
/// The licensed track's slot: none is bundled yet, and the film is silent
/// without one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Music {
    /// A file in the app's resources (`Contents/Resources/<file>`).
    pub file: &'static str,
    pub bpm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CueSheet {
    pub cues: Vec<Cue>,
    pub sounds: Vec<Sound>,
    /// OWNER: a licensed track goes here (20 s at 100 BPM, 4/4, with a bed
    /// that can loop under the card). `None` is the silent default.
    pub music: Option<Music>,
    /// The tempo the times below are cut to.
    pub bpm: f64,
    /// How long a wave takes from the cue that starts it.
    pub wave_seconds: f64,
}
/// The default wave between two steps on the card, and into the app.
pub const STEP_WAVE: WaveType = WaveType::Center;

impl Default for CueSheet {
    fn default() -> Self {
        use Scene::*;
        use Transition::*;
        let cue = |at, scene, out, title| Cue {
            at,
            scene,
            out,
            title,
        };
        Self {
            cues: vec![
                cue(0.0, HouseLights, None, "Powermove"),
                cue(1.4, Mark, Some(Fold { seconds: 0.8 }), "Powermove"),
                cue(4.0, Composer, Some(Wave(WaveType::Center)), "Your agent"),
                cue(
                    5.6,
                    Beat(Change::GlassTimeline),
                    None,
                    Change::GlassTimeline.prompt(),
                ),
                cue(
                    9.2,
                    Beat(Change::LightLeak),
                    None,
                    Change::LightLeak.prompt(),
                ),
                cue(
                    12.8,
                    Beat(Change::BeatMarkers),
                    None,
                    Change::BeatMarkers.prompt(),
                ),
                cue(
                    16.4,
                    Beat(Change::ClassicLayout),
                    Some(Wave(WaveType::Center)),
                    Change::ClassicLayout.prompt(),
                ),
                cue(20.0, Card, None, "Welcome to Powermove"),
            ],
            sounds: vec![Sound {
                at: 1.4,
                id: "chime",
            }],
            music: None,
            bpm: 100.,
            wave_seconds: 1.1,
        }
    }
}
/// Where the film is at a time: the scene showing, how far into it, and the
/// transition running out of it (with its 0–1 progress), if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub index: usize,
    pub scene: Scene,
    /// Seconds since the scene's cue.
    pub local: f64,
    /// Seconds the scene holds before the next cue (infinite for the last).
    pub length: f64,
    pub leaving: Option<(Transition, f64)>,
}
impl CueSheet {
    pub fn length(&self) -> f64 {
        self.cues.last().map_or(0., |c| c.at)
    }
    /// The cue in force at `t` (seconds from the start of the film).
    pub fn at(&self, t: f64) -> Position {
        let t = t.max(0.);
        let index = self.cues.iter().rposition(|c| c.at <= t).unwrap_or(0);
        let cue = self.cues[index];
        let next = self.cues.get(index + 1).map(|n| n.at);
        let length = next.map_or(f64::INFINITY, |n| n - cue.at);
        let local = t - cue.at;
        let leaving = cue.out.and_then(|out| {
            let span = self.span(out);
            let start = length - span;
            (local >= start && length.is_finite())
                .then(|| (out, ((local - start) / span).clamp(0., 1.)))
        });
        Position {
            index,
            scene: cue.scene,
            local,
            length,
            leaving,
        }
    }
    /// How long a transition runs before the next cue.
    pub fn span(&self, out: Transition) -> f64 {
        match out {
            Transition::Fold { seconds } => seconds,
            Transition::Wave(_) => self.wave_seconds,
        }
    }
    /// When a scene starts (the Skip target is the card's).
    pub fn start_of(&self, scene: Scene) -> Option<f64> {
        self.cues.iter().find(|c| c.scene == scene).map(|c| c.at)
    }
    /// The same film cut to another tempo: every time scales by the ratio.
    pub fn retimed(&self, bpm: f64) -> Self {
        let k = self.bpm / bpm.max(1.);
        let mut sheet = self.clone();
        for cue in &mut sheet.cues {
            cue.at *= k;
            if let Some(Transition::Fold { seconds }) = &mut cue.out {
                *seconds *= k;
            }
        }
        for sound in &mut sheet.sounds {
            sound.at *= k;
        }
        sheet.wave_seconds *= k;
        sheet.bpm = bpm;
        sheet
    }
    /// Sounds whose time falls in `(from, to]`.
    pub fn sounds_between(&self, from: f64, to: f64) -> impl Iterator<Item = &Sound> {
        self.sounds
            .iter()
            .filter(move |s| s.at > from && s.at <= to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_sheet_runs_in_order_and_ends_on_the_card() {
        let sheet = CueSheet::default();
        assert!(sheet.cues.windows(2).all(|w| w[0].at < w[1].at));
        assert_eq!(sheet.cues.last().unwrap().scene, Scene::Card);
        assert_eq!(sheet.length(), 20.);
        for change in Change::ALL {
            assert!(sheet.start_of(Scene::Beat(change)).is_some());
        }
        for pair in sheet.cues.windows(2) {
            if let Some(out) = pair[0].out {
                assert!(sheet.span(out) < pair[1].at - pair[0].at);
            }
        }
    }
    #[test]
    fn positions_report_the_transition_running_out_of_a_scene() {
        let sheet = CueSheet::default();
        let p = sheet.at(4.5);
        assert_eq!(p.scene, Scene::Composer);
        assert!(p.leaving.is_none());
        let (out, progress) = sheet.at(5.6 - 0.55).leaving.unwrap();
        assert_eq!(out, Transition::Wave(WaveType::Center));
        assert!((progress - 0.5).abs() < 1e-9);
        let p = sheet.at(100.);
        assert_eq!(p.scene, Scene::Card);
        assert!(p.length.is_infinite() && p.leaving.is_none());
        assert_eq!(sheet.at(-1.).scene, Scene::HouseLights);
    }
    #[test]
    fn retiming_scales_every_beat_and_keeps_the_silent_default() {
        let sheet = CueSheet::default();
        assert!(sheet.music.is_none());
        let fast = sheet.retimed(120.);
        assert!((fast.length() - 20. * 100. / 120.).abs() < 1e-9);
        assert!((fast.sounds[0].at - 1.4 * 100. / 120.).abs() < 1e-9);
    }
    #[test]
    fn sounds_fire_once_between_two_frames() {
        let sheet = CueSheet::default();
        assert_eq!(sheet.sounds_between(1.39, 1.41).count(), 1);
        assert_eq!(sheet.sounds_between(1.41, 1.42).count(), 0);
    }
}
