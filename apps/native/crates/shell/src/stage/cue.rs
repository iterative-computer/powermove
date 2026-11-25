//! The first-run intro's cue sheet: every beat of the stage is a time here,
//! and the stage reads nothing else for timing. Pure data, so the intro is
//! tested without a screen. About six and a half seconds from launch to an
//! interactive Welcome window.
//!
//! Music: `CueSheet::music` names a licensed track's slot; the default is
//! silent. `retimed` scales every beat to another tempo.

/// What the stage shows at a time, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The desktop dims under a black veil.
    Dim,
    /// The credit line, quiet, near the top.
    Credit,
    /// The mark rises from below the bottom edge into the centre.
    Mark,
    /// The mark's light softens and spreads; the picture holds still for
    /// the wave to picture it.
    Spread,
    /// The wave draws the Welcome window out of the light.
    Wave,
    /// The Welcome window, interactive.
    Window,
}
/// A sound the intro plays at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sound {
    pub at: f64,
    pub id: &'static str,
}
/// The licensed track's slot: none is bundled, and the intro is silent
/// without one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Music {
    /// A file in the app's resources (`Contents/Resources/<file>`).
    pub file: &'static str,
    pub bpm: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct CueSheet {
    /// The veil is down (the house lights are out).
    pub dimmed: f64,
    /// The credit: fades in, its second line in, fades out.
    pub credit: (f64, f64, f64),
    /// Light first shows at the bottom edge.
    pub edge_light: f64,
    /// The mark rises from below the edge, and settles in the centre.
    pub rise: (f64, f64),
    /// The wordmark settles under the mark.
    pub words: f64,
    /// The mark's light softens and spreads.
    pub spread: f64,
    /// Everything holds still from here so the wave's pictures are taken
    /// before it starts, never on the frame that needs them.
    pub hold: f64,
    /// The wave starts drawing the Welcome window.
    pub window: f64,
    pub wave_seconds: f64,
    pub sounds: Vec<Sound>,
    /// OWNER: a licensed track goes here; `None` is the silent default.
    pub music: Option<Music>,
    pub bpm: f64,
}
impl Default for CueSheet {
    fn default() -> Self {
        Self {
            dimmed: 1.2,
            credit: (1.0, 1.6, 3.8),
            edge_light: 1.7,
            rise: (2.8, 4.3),
            words: 3.9,
            spread: 4.8,
            hold: 5.25,
            window: 5.4,
            wave_seconds: 1.1,
            sounds: vec![Sound {
                at: 2.8,
                id: "chime",
            }],
            music: None,
            bpm: 100.,
        }
    }
}
impl CueSheet {
    /// When the Welcome window is interactive.
    pub fn length(&self) -> f64 {
        self.window + self.wave_seconds
    }
    pub fn phase(&self, t: f64) -> Phase {
        if t < self.credit.0 {
            Phase::Dim
        } else if t < self.rise.0 {
            Phase::Credit
        } else if t < self.spread {
            Phase::Mark
        } else if t < self.window {
            Phase::Spread
        } else if t < self.length() {
            Phase::Wave
        } else {
            Phase::Window
        }
    }
    /// What VoiceOver reads for a phase. OWNER-COPY: the credit.
    pub fn title(&self, phase: Phase) -> &'static str {
        match phase {
            Phase::Dim => "Powermove",
            Phase::Credit => "[Presenter] presents",
            Phase::Mark | Phase::Spread => "Powermove",
            Phase::Wave | Phase::Window => "Welcome to Powermove",
        }
    }
    /// The same intro at another tempo: every time scales by the ratio.
    pub fn retimed(&self, bpm: f64) -> Self {
        let k = self.bpm / bpm.max(1.);
        let mut s = self.clone();
        for v in [
            &mut s.dimmed,
            &mut s.credit.0,
            &mut s.credit.1,
            &mut s.credit.2,
            &mut s.edge_light,
            &mut s.rise.0,
            &mut s.rise.1,
            &mut s.words,
            &mut s.spread,
            &mut s.hold,
            &mut s.window,
            &mut s.wave_seconds,
        ] {
            *v *= k;
        }
        for sound in &mut s.sounds {
            sound.at *= k;
        }
        s.bpm = bpm;
        s
    }
    /// Sounds whose time falls in `(from, to]`.
    pub fn sounds_between(&self, from: f64, to: f64) -> impl Iterator<Item = &Sound> {
        self.sounds
            .iter()
            .filter(move |s| s.at > from && s.at <= to)
    }
}

pub fn smooth(x: f64) -> f64 {
    let x = x.clamp(0., 1.);
    x * x * (3. - 2. * x)
}
pub fn ease_out(x: f64) -> f64 {
    let x = x.clamp(0., 1.);
    1. - (1. - x).powi(3)
}
/// 0 to 1 over `(from, to)` seconds, smoothed.
pub fn span(t: f64, from: f64, to: f64) -> f64 {
    smooth((t - from) / (to - from))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_intro_runs_in_order_to_an_interactive_window() {
        let s = CueSheet::default();
        let order = [
            s.credit.0,
            s.edge_light,
            s.rise.0,
            s.words,
            s.rise.1,
            s.spread,
            s.hold,
            s.window,
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        assert!(
            s.credit.2 > s.rise.0 && s.credit.2 < s.rise.1,
            "the credit goes as the mark rises"
        );
        assert!((6.0..=8.0).contains(&s.length()), "interactive in 6–8 s");
        assert_eq!(s.phase(0.5), Phase::Dim);
        assert_eq!(s.phase(3.0), Phase::Mark);
        assert_eq!(s.phase(5.3), Phase::Spread);
        assert_eq!(s.phase(5.9), Phase::Wave);
        assert_eq!(s.phase(9.), Phase::Window);
    }
    #[test]
    fn retiming_scales_every_beat_and_keeps_the_silent_default() {
        let s = CueSheet::default();
        assert!(s.music.is_none());
        let fast = s.retimed(120.);
        assert!((fast.length() - s.length() * 100. / 120.).abs() < 1e-9);
        assert!((fast.sounds[0].at - s.sounds[0].at * 100. / 120.).abs() < 1e-9);
    }
    #[test]
    fn sounds_fire_once_between_two_frames() {
        let s = CueSheet::default();
        assert_eq!(s.sounds_between(2.79, 2.81).count(), 1);
        assert_eq!(s.sounds_between(2.81, 2.82).count(), 0);
    }
}
