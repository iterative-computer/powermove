//! What happens inside each scene, as functions of the scene's own time:
//! the engine's compositions are keyed from these, and the stage's furniture
//! (the clip strip, the composer, the readouts) reads the same values, so
//! the picture and its instruments never drift apart.
use super::cue::Reel;

/// How long each scene holds before its transition, as cut on the sheet.
pub const REEL_SECONDS: f64 = 3.6;
/// Cut: three shots of two beats each at 100 BPM (1.2 s).
pub const SHOT_SECONDS: f64 = 1.2;
pub const SHOTS: usize = 3;
/// Ask: the request is typed, the agent answers, the effect builds.
pub const ASK_PROMPT: &str = "Make the title glow";
pub const ASK_TYPED: (f64, f64) = (0.15, 1.05);
pub const ASK_BUILDING: f64 = 1.2;
pub const ASK_BUILT: f64 = 2.2;

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

/// The shot showing at `t` in the Cut reel, and how far into it.
pub fn shot(t: f64) -> (usize, f64) {
    let t = t.clamp(0., REEL_SECONDS - 1e-9);
    let index = ((t / SHOT_SECONDS) as usize).min(SHOTS - 1);
    (index, t - index as f64 * SHOT_SECONDS)
}
/// How many characters of the request show at `t`.
pub fn typed(t: f64) -> usize {
    let n = ASK_PROMPT.chars().count();
    let (a, b) = ASK_TYPED;
    (((t - a) / (b - a)).clamp(0., 1.) * n as f64).round() as usize
}
/// The glow the agent builds: (intensity, radius) of the Glow effect.
pub fn glow(t: f64) -> (f64, f64) {
    let k = span(t, ASK_BUILDING, ASK_BUILT);
    (k * 160., 30. + k * 90.)
}
/// The Grade reel's look, as the Color effect's (temperature, saturation,
/// contrast, exposure) over a flat, ungraded shot.
pub const GRADE: [f64; 4] = [32., 60., 24., 0.1];
/// How much of the shot the grade covers at `t`: a split sweeping across.
pub fn split(t: f64) -> f64 {
    span(t, 0.35, 2.9)
}
/// The camera's orbit in the Depth reel, in degrees around the subject.
pub fn orbit(t: f64) -> f64 {
    -28. + 56. * smooth(t / REEL_SECONDS)
}

/// What the caption under the screen says.
pub fn caption(reel: Reel) -> &'static str {
    // OWNER-COPY: one plain line per reel.
    match reel {
        Reel::Cut => "Cut on the beat.",
        Reel::Ask => "Ask your agent for an effect.",
        Reel::Grade => "Grade in real colour.",
        Reel::Depth => "Move through 3D.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn beats_land_inside_their_reel() {
        assert_eq!(shot(0.).0, 0);
        assert_eq!(shot(1.3).0, 1);
        assert_eq!(shot(REEL_SECONDS).0, SHOTS - 1);
        assert_eq!(typed(0.), 0);
        assert_eq!(typed(2.), ASK_PROMPT.chars().count());
        assert_eq!(glow(0.).0, 0.);
        assert_eq!(glow(3.).0, 160.);
        assert!(split(0.) == 0. && split(3.) == 1.);
        assert!(orbit(0.) < 0. && orbit(REEL_SECONDS) > 0.);
    }
}
