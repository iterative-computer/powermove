//! What happens inside each scene, as functions of the scene's own time.
//! A beat: the request is typed into the composer, sent, the agent works,
//! and the change lands on its surface under the wave.
use super::cue::Change;

/// How long each beat holds (a bar and a half at 100 BPM).
pub const BEAT_SECONDS: f64 = 3.6;
/// The request is typed over this span of the beat.
pub const TYPED: (f64, f64) = (0.15, 1.2);
/// Sent (the composer's send turns to working).
pub const SENT: f64 = 1.3;
/// The change is applied and its wave starts; the wave takes the sheet's
/// `wave_seconds`.
pub const LANDS: f64 = 1.9;

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
/// How many characters of the request show at `t` into its beat.
pub fn typed(change: Change, t: f64) -> usize {
    let n = change.prompt().chars().count();
    let (a, b) = TYPED;
    (((t - a) / (b - a)).clamp(0., 1.) * n as f64).round() as usize
}
/// What the composer shows at `t` into a beat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Composing {
    Typing(usize),
    Working,
    Done,
}
pub fn composing(change: Change, t: f64) -> Composing {
    if t < SENT {
        Composing::Typing(typed(change, t))
    } else if t < LANDS + 0.6 {
        Composing::Working
    } else {
        Composing::Done
    }
}
/// The final line on the card. OWNER-COPY: the closing line.
pub const CLOSING: &str = "[Closing line — owner's words]";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_beat_types_sends_and_lands_in_order() {
        let change = Change::GlassTimeline;
        assert_eq!(composing(change, 0.), Composing::Typing(0));
        assert_eq!(
            composing(change, SENT - 0.01),
            Composing::Typing(change.prompt().chars().count())
        );
        assert_eq!(composing(change, SENT + 0.1), Composing::Working);
        assert_eq!(composing(change, BEAT_SECONDS - 0.1), Composing::Done);
        assert!(TYPED.1 < SENT && SENT < LANDS && LANDS + 1.1 < BEAT_SECONDS);
    }
}
