//! The first-run film's cue sheet: every beat of the stage is a time on this
//! sheet (scene in-points, waves, the sound), and the stage reads nothing
//! else for timing. Pure data, so the film is tested without a screen.
//!
//! Music: `CueSheet::music` names the licensed track's slot; the default is
//! silent. The sheet is cut for a 22 s piece at 100 BPM in 4/4 (one bar is
//! 2.4 s); `retimed` scales it to another tempo.
use pm_wave::WaveType;

/// One picture on the stage, in film order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scene {
    /// The desktop dims under a black veil; nothing else yet.
    HouseLights,
    /// Crop marks draw out from the centre, the timecode starts, one line.
    TitleCard,
    /// Light gathers and the mark resolves out of it.
    Mark,
    /// The engine's reels, each a composition rendered live.
    Reel(Reel),
    /// The calm card with one primary action; the first-run steps live here.
    Card,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reel {
    Cut,
    Ask,
    Grade,
    Depth,
}
impl Reel {
    pub const ALL: [Reel; 4] = [Reel::Cut, Reel::Ask, Reel::Grade, Reel::Depth];
}
/// How a scene leaves for the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    /// Light falls and rises again: the picture fades through black.
    Dip { seconds: f64 },
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
    /// OWNER: a licensed track goes here (22 s at 100 BPM, 4/4, with a bed
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
                cue(1.4, TitleCard, Some(Dip { seconds: 0.6 }), "Powermove"),
                cue(4.4, Mark, Some(Wave(WaveType::Center)), "Powermove"),
                cue(
                    7.6,
                    Reel(self::Reel::Cut),
                    Some(Wave(WaveType::Vertical)),
                    "Cut on the beat",
                ),
                cue(
                    11.2,
                    Reel(self::Reel::Ask),
                    Some(Wave(WaveType::Horizontal)),
                    "Ask for an effect",
                ),
                cue(
                    14.8,
                    Reel(self::Reel::Grade),
                    Some(Wave(WaveType::Vertical)),
                    "Grade the picture",
                ),
                cue(
                    18.4,
                    Reel(self::Reel::Depth),
                    Some(Wave(WaveType::Center)),
                    "Move in depth",
                ),
                cue(22.0, Card, None, "Welcome to Powermove"),
            ],
            sounds: vec![Sound {
                at: 4.4,
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
            Transition::Dip { seconds } => seconds,
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
            if let Some(Transition::Dip { seconds }) = &mut cue.out {
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
        assert_eq!(sheet.length(), 22.);
        for reel in Reel::ALL {
            assert!(sheet.start_of(Scene::Reel(reel)).is_some());
        }
        // Every transition fits inside its scene.
        for pair in sheet.cues.windows(2) {
            if let Some(out) = pair[0].out {
                assert!(sheet.span(out) < pair[1].at - pair[0].at);
            }
        }
    }
    #[test]
    fn positions_report_the_transition_running_out_of_a_scene() {
        let sheet = CueSheet::default();
        let p = sheet.at(9.0);
        assert_eq!(p.scene, Scene::Reel(Reel::Cut));
        assert!(p.leaving.is_none());
        // The Cut reel's wave starts 1.1 s before the Ask reel's cue.
        let p = sheet.at(11.2 - 0.55);
        let (out, progress) = p.leaving.unwrap();
        assert_eq!(out, Transition::Wave(WaveType::Vertical));
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
        assert!((fast.length() - 22. * 100. / 120.).abs() < 1e-9);
        assert!((fast.sounds[0].at - 4.4 * 100. / 120.).abs() < 1e-9);
        assert_eq!(fast.cues.len(), sheet.cues.len());
    }
    #[test]
    fn sounds_fire_once_between_two_frames() {
        let sheet = CueSheet::default();
        assert_eq!(sheet.sounds_between(4.39, 4.41).count(), 1);
        assert_eq!(sheet.sounds_between(4.41, 4.42).count(), 0);
    }
}
