//! The first-run intro's cue sheet: every beat of the stage is a time here,
//! and the stage reads nothing else for timing. The times and curves are
//! Dia's opening measured frame by frame (60 fps) and fitted: near-critically
//! damped springs for the mark's rise and scale, long cubic fades for the
//! credit and the light (lane notes, w101). Pure data, so the intro is
//! tested without a screen.
//!
//! Music: `CueSheet::music` names a licensed track's slot; the default is
//! silent. `retimed` scales every beat to another tempo.
use pm_wave::{WaveStyle, WaveType, bezier_ease};

/// What the stage shows at a time, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// The desktop dims; the mark's light shows at the bottom edge.
    Dim,
    /// The credit, quiet, in the upper middle.
    Credit,
    /// The mark rises from below the bottom edge into the centre.
    Mark,
    /// The mark at rest, breathing; the wave's pictures are taken.
    Rest,
    /// The wave draws the Welcome window out of the mark's light.
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
/// A cubic fade: from `at`, over `seconds`, on `curve` (CSS cubic-bezier).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub at: f64,
    pub seconds: f64,
    pub curve: [f64; 4],
}
impl Fade {
    pub fn at(&self, t: f64) -> f64 {
        bezier_ease(self.curve, (t - self.at) / self.seconds)
    }
}
/// A damped spring released at `at`: 0 → 1 with angular frequency `omega`
/// (rad/s) and damping ratio `zeta`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub at: f64,
    pub omega: f64,
    pub zeta: f64,
}
impl Spring {
    pub fn at(&self, t: f64) -> f64 {
        let tau = (t - self.at).max(0.);
        let (w, z) = (self.omega, self.zeta);
        if (z - 1.).abs() < 1e-6 {
            return 1. - (1. + w * tau) * (-w * tau).exp();
        }
        if z < 1. {
            let wd = w * (1. - z * z).sqrt();
            1. - (-z * w * tau).exp() * ((wd * tau).cos() + z * w / wd * (wd * tau).sin())
        } else {
            let r = (z * z - 1.).sqrt();
            let (r1, r2) = (-w * (z - r), -w * (z + r));
            1. - (r2 * (r1 * tau).exp() - r1 * (r2 * tau).exp()) / (r2 - r1)
        }
    }
    /// When it is within `within` (a fraction) of rest for good.
    pub fn settled(&self, within: f64) -> f64 {
        let mut t = self.at + 12.;
        while t > self.at && (1. - self.at(t)).abs() < within {
            t -= 1. / 120.;
        }
        t
    }
}

/// The black veil's opacity over the desktop at its darkest, and as the
/// window arrives (it lifts a touch; there is no further dim). Dia's,
/// measured on the screen the way macOS blends a window (display-encoded:
/// 1 − value under the veil / value before it): 0.741, then 0.675. Arc's
/// peaks at 0.871 and lifts away once its window is up.
pub const DIM_PEAK: f64 = 0.741;
pub const DIM_WINDOW: f64 = 0.675;

#[derive(Clone, Debug, PartialEq)]
pub struct CueSheet {
    /// The desktop dims: a black veil fading in over the real desktop, on
    /// Dia's measured curve (to `DIM_PEAK`).
    pub dim: Spring,
    /// The veil lifts a touch as the window arrives (to `DIM_WINDOW`).
    pub lift: Fade,
    /// The hand-off: the veil fades out from the step's start (seconds from
    /// the hand-off) as the window eases away (Dia's measured fade-out).
    pub dim_out: Fade,
    /// The mark's top shows at the bottom edge and creeps up.
    pub creep: Fade,
    /// The credit's first line, its second, and both going.
    pub credit: (Fade, Fade, Fade),
    /// The mark's centre rises; its scale comes down (they start apart).
    pub rise: Spring,
    pub shrink: Spring,
    /// The light at the bottom edge flares as the mark launches, then falls.
    pub flare: Fade,
    pub fall: Fade,
    /// The white-hot body comes down to its gradient.
    pub exposure: Fade,
    /// The wordmark, uncovered as the mark passes, rides up into place.
    pub words: Fade,
    /// Everything holds still from here so the wave's pictures are taken
    /// before it starts, never on the frame that needs them.
    pub hold: f64,
    /// The wave starts drawing the Welcome window.
    pub window: f64,
    /// The wave's own length (its style's duration, unchanged).
    pub wave_seconds: f64,
    /// The fraction of the wave after which nothing of it shows: the window
    /// is live from there.
    pub wave_visible: f64,
    pub sounds: Vec<Sound>,
    /// OWNER: a licensed track goes here; `None` is the silent default.
    pub music: Option<Music>,
    pub bpm: f64,
}
/// The wave the window arrives with: the original's Vertical defaults.
pub fn wave_style() -> WaveStyle {
    WaveStyle::new(WaveType::Vertical)
}
impl Default for CueSheet {
    fn default() -> Self {
        let wave = wave_style();
        Self {
            // Fitted to Dia's veil (30 fps, rms 0.0025 of opacity).
            dim: Spring {
                at: 0.52,
                omega: 2.55,
                zeta: 1.10,
            },
            // Dia's veil lifts from 7.05 s for 0.45 s, as its window comes.
            lift: Fade {
                at: 7.05,
                seconds: 0.45,
                curve: [0.42, 0., 0.58, 1.],
            },
            // Dia's veil goes in 1.35 s, fast first (rms 0.006 of opacity).
            dim_out: Fade {
                at: 0.,
                seconds: 1.35,
                curve: [0.3, 0.4, 0.1, 1.0],
            },
            creep: Fade {
                at: 0.72,
                seconds: 2.40,
                curve: [0.13, 0.19, 0.29, 0.92],
            },
            credit: (
                Fade {
                    at: 0.91,
                    seconds: 2.11,
                    curve: [0.35, 0.53, 0.41, 0.85],
                },
                Fade {
                    at: 1.62,
                    seconds: 0.92,
                    curve: [0.27, -0.29, 0.59, 1.07],
                },
                Fade {
                    at: 3.08,
                    seconds: 1.26,
                    curve: [0.24, 0.06, 0., -0.07],
                },
            ),
            rise: Spring {
                at: 3.07,
                omega: 2.81,
                zeta: 1.075,
            },
            shrink: Spring {
                at: 3.03,
                omega: 2.02,
                zeta: 0.976,
            },
            flare: Fade {
                at: 3.08,
                seconds: 0.68,
                curve: [0.27, 0.12, 0.19, 1.13],
            },
            fall: Fade {
                at: 3.72,
                seconds: 1.24,
                curve: [0.75, 0.65, 0., 0.94],
            },
            exposure: Fade {
                at: 3.90,
                seconds: 1.53,
                curve: [0.32, 0.79, 0.70, 0.95],
            },
            words: Fade {
                at: 5.30,
                seconds: 1.68,
                curve: [0.30, 0.92, 0.56, 0.91],
            },
            hold: 6.55,
            window: 6.7,
            wave_seconds: wave.duration(),
            // Measured: the original's rim and hairline are gone at 0.72 of
            // its run (mean difference to the window 0.17/255).
            wave_visible: 0.72,
            sounds: vec![Sound {
                at: 3.07,
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
        self.window + self.wave_seconds * self.wave_visible
    }
    pub fn phase(&self, t: f64) -> Phase {
        if t < self.credit.0.at {
            Phase::Dim
        } else if t < self.shrink.at {
            Phase::Credit
        } else if t < self.hold {
            Phase::Mark
        } else if t < self.window {
            Phase::Rest
        } else if t < self.length() {
            Phase::Wave
        } else {
            Phase::Window
        }
    }
    /// What VoiceOver reads for a phase.
    pub fn title(&self, phase: Phase) -> &'static str {
        match phase {
            Phase::Dim => "Powermove",
            Phase::Credit => "Iterative Computer presents",
            Phase::Mark | Phase::Rest => "Powermove",
            Phase::Wave | Phase::Window => "Welcome to Powermove",
        }
    }
    /// The same intro at another tempo: every time scales by the ratio.
    pub fn retimed(&self, bpm: f64) -> Self {
        let k = self.bpm / bpm.max(1.);
        let mut s = self.clone();
        for f in [
            &mut s.lift,
            &mut s.creep,
            &mut s.credit.0,
            &mut s.credit.1,
            &mut s.credit.2,
            &mut s.flare,
            &mut s.fall,
            &mut s.exposure,
            &mut s.words,
        ] {
            f.at *= k;
            f.seconds *= k;
        }
        for spring in [&mut s.dim, &mut s.rise, &mut s.shrink] {
            spring.at *= k;
            spring.omega /= k;
        }
        for v in [&mut s.hold, &mut s.window, &mut s.wave_seconds] {
            *v *= k;
        }
        for sound in &mut s.sounds {
            sound.at *= k;
        }
        s.bpm = bpm;
        s
    }
    /// The veil's opacity `opened` seconds after the stage opened, at the
    /// intro's time `t` (a Skip moves `t`, never the dim's own fade-in).
    pub fn veil(&self, opened: f64, t: f64) -> f64 {
        DIM_PEAK * self.dim.at(opened) - (DIM_PEAK - DIM_WINDOW) * self.lift.at(t)
    }
    /// The veil's share left `since` seconds into the hand-off (1 to 0).
    pub fn veil_left(&self, since: f64) -> f64 {
        1. - self.dim_out.at(since)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_intro_runs_in_order_to_an_interactive_window() {
        let s = CueSheet::default();
        let order = [
            s.dim.at,
            s.creep.at,
            s.credit.0.at,
            s.credit.1.at,
            s.shrink.at,
            s.rise.at,
            s.exposure.at,
            s.words.at,
            s.hold,
            s.window,
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        assert!(
            s.rise.settled(0.01) < s.hold,
            "the mark rests before the wave's pictures are taken"
        );
        assert!((7.0..=10.0).contains(&s.length()), "{}", s.length());
        assert_eq!(s.phase(0.5), Phase::Dim);
        assert_eq!(s.phase(3.5), Phase::Mark);
        assert_eq!(s.phase(6.6), Phase::Rest);
        assert_eq!(s.phase(7.0), Phase::Wave);
        assert_eq!(s.phase(12.), Phase::Window);
    }
    #[test]
    fn the_springs_match_dias_measured_rise() {
        // Dia's disc centre (fractions of the height) at measured times.
        let s = CueSheet::default();
        let y = |t: f64| 1.781 + (0.4997 - 1.781) * s.rise.at(t);
        for (t, measured) in [
            (3.3, 1.6129),
            (3.6, 1.2467),
            (4.0, 0.8790),
            (4.5, 0.6522),
            (5.0, 0.5605),
        ] {
            assert!(
                (y(t) - measured).abs() < 0.03,
                "{t}: {} vs {measured}",
                y(t)
            );
        }
    }
    #[test]
    fn the_veil_follows_dias_measured_dim() {
        // Dia's veil opacity, measured at 30 fps on the desktop it covers.
        let s = CueSheet::default();
        for (t, measured) in [
            (0.5, 0.0),
            (1.0, 0.249),
            (1.5, 0.502),
            (2.0, 0.628),
            (3.0, 0.723),
        ] {
            let v = s.veil(t, t);
            assert!((v - measured).abs() < 0.03, "{t}: {v} vs {measured}");
        }
        assert!((s.veil(30., 30.) - DIM_WINDOW).abs() < 1e-3);
        assert!(s.veil(6.9, 6.9) > s.veil(30., 30.), "lifts, never deepens");
        // Out: half gone in about 0.25 s, all of it by 1.35 s.
        assert!(
            (s.veil_left(0.25) - 0.5).abs() < 0.12,
            "{}",
            s.veil_left(0.25)
        );
        assert_eq!(s.veil_left(1.35), 0.);
    }
    #[test]
    fn retiming_scales_every_beat_and_keeps_the_silent_default() {
        let s = CueSheet::default();
        assert!(s.music.is_none());
        let fast = s.retimed(120.);
        assert!((fast.length() - s.length() * 100. / 120.).abs() < 1e-9);
        assert!((fast.rise.at(fast.rise.at + 0.5) - s.rise.at(s.rise.at + 0.6)).abs() < 1e-9);
    }
    #[test]
    fn sounds_fire_once_between_two_frames() {
        let s = CueSheet::default();
        assert_eq!(s.sounds_between(3.06, 3.08).count(), 1);
        assert_eq!(s.sounds_between(3.08, 3.09).count(), 0);
    }
}
