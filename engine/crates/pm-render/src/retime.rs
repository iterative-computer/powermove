//! Retiming on the render side (plan section 5 rules 4, 5, and 7;
//! amendment 16; section 10, the precomp row).
//!
//! `pm-eval` answers a layer's source time (`sourceTime`: time remap,
//! constant speed and trim, variable speed integrated at the owning
//! composition's rate). The render graph turns it into what is drawn:
//!
//! - **Video.** The nonnegative source time selects the media frame by
//!   section 5 rule 7 in `pm-media` (the last frame with PTS ≤ t + q/2),
//!   holding the last frame beyond the source's end on every surface. With an
//!   Interpret Footage frame-rate override (`asset.interpret.frameRate`),
//!   frame n is shown at n / rate instead of at its PTS: the frame is the
//!   last n with n / rate ≤ t + q/2, q a quarter of the overridden frame
//!   duration, clamped to the file's frames.
//! - **Precomps.** The child composition renders at the layer's source
//!   time; with "Keep frame rate when nested" (`d.keepFrameRate`) the time
//!   is first quantized down to the start of the child's own frame
//!   (section 5 rule 4).

use pm_eval::Comp;
use pm_eval::node;
use pm_model::Node;
use pm_time::{Rate, TICKS_PER_SECOND, Tick};

/// The Interpret Footage frame-rate override of an asset record:
/// `interpret.frameRate` as `[num, den]` or a positive number (a float
/// within 0.02% of an NTSC or integer rate maps to it, as composition
/// rates do), or the sheet's `frameRateRational` beside its float. One
/// reading for the renderer and the sheet: `pm_media::interpret`.
pub fn rate_override(record: &Node) -> Option<Rate> {
    crate::footage::interpretation(record).and_then(|i| i.frame_rate)
}

/// The frame showing at source time `seconds` when frame n is shown at n /
/// `rate`: the last n with n / rate ≤ t + q/2, q = 1 / (4 · rate), so n =
/// ⌊t · rate + 1/8⌋ in exact arithmetic; `None` before the first frame.
/// The caller clamps to the file's last frame.
pub fn override_frame(seconds: f64, rate: Rate) -> Option<usize> {
    let ticks = (seconds * TICKS_PER_SECOND as f64).round();
    if !ticks.is_finite() {
        return None;
    }
    override_frame_at(ticks as Tick, rate)
}

/// [`override_frame`] at a tick.
pub fn override_frame_at(ticks: Tick, rate: Rate) -> Option<usize> {
    let t = i128::from(ticks);
    // n ≤ (t / TICKS + 1 / (8 · rate)) · rate
    //   = (8 · t · num + TICKS · den) / (8 · TICKS · den).
    let num = i128::from(rate.num());
    let den = i128::from(rate.den());
    let ticks_per_second = i128::from(TICKS_PER_SECOND);
    let top = 8 * t * num + ticks_per_second * den;
    if top < 0 {
        return None;
    }
    let n = top / (8 * ticks_per_second * den);
    usize::try_from(n).ok()
}

/// Section 5 rule 4: with `d.keepFrameRate`, the child's time quantized
/// down to the start of its own frame (`n / fps` of the child, the
/// document's own float, amendment 1). Without a valid child rate the time
/// is kept.
pub fn keep_frame_rate(layer: &Node, child: &Comp<'_>, time: f64) -> f64 {
    let keep = node::member(node::member(Some(layer), "d"), "keepFrameRate");
    if !node::truthy(keep) {
        return time;
    }
    quantize(child, time)
}

/// The start of the frame of `child` containing `time`.
pub fn quantize(child: &Comp<'_>, time: f64) -> f64 {
    let Some(rate) = child.rate() else {
        return time;
    };
    let Ok(t) = pm_time::ticks_from_seconds(time) else {
        return time;
    };
    let n = rate.rate.frame_of(t);
    match rate.rate.frame_start(n) {
        Ok(start) => rate.seconds_of(start as Tick),
        Err(_) => time,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Node {
        Node::from_value(pm_model::json::parse(s).unwrap(), 0)
    }

    #[test]
    fn rate_overrides_read_fractions_and_floats() {
        let r = |s: &str| rate_override(&parse(s)).map(|r| (r.num(), r.den()));
        assert_eq!(
            r(r#"{"interpret":{"frameRate":[24000,1001]}}"#),
            Some((24000, 1001))
        );
        assert_eq!(r(r#"{"interpret":{"frameRate":25}}"#), Some((25, 1)));
        assert_eq!(
            r(r#"{"interpret":{"frameRate":29.97}}"#),
            Some((30000, 1001))
        );
        assert_eq!(r(r#"{"interpret":{"frameRate":[0,1]}}"#), None);
        assert_eq!(r(r#"{"interpret":{"frameRate":[24.5,1]}}"#), None);
        assert_eq!(r(r#"{"interpret":{"frameRate":"24"}}"#), None);
        assert_eq!(r(r#"{"interpret":{}}"#), None);
        assert_eq!(r(r#"{}"#), None);
    }

    #[test]
    fn override_frames_follow_rule_seven() {
        let r24 = Rate::new(24, 1).unwrap();
        assert_eq!(override_frame(0.0, r24), Some(0));
        // Frame 1 starts at 1/24; a time a hair before it still shows it
        // (the q/2 tolerance), a quarter frame before shows frame 0.
        assert_eq!(override_frame(1.0 / 24.0 - 1e-9, r24), Some(1));
        assert_eq!(override_frame(1.0 / 24.0 - 0.25 / 24.0, r24), Some(0));
        assert_eq!(override_frame(10.0, r24), Some(240));
        assert_eq!(override_frame(-0.001, r24), Some(0));
        assert_eq!(override_frame(-0.01, r24), None);
        let ntsc = Rate::new(30000, 1001).unwrap();
        for n in [0usize, 1, 29, 30, 1799, 1800, 107_892] {
            let t = n as f64 * 1001.0 / 30000.0;
            assert_eq!(override_frame(t, ntsc), Some(n), "frame {n}");
        }
    }

    #[test]
    fn keep_frame_rate_quantizes_to_the_child_frame() {
        let d = pm_model::Document::new(
            Some(
                pm_model::json::parse(
                    r#"{"id":"P","compId":"C","fps":60,"layers":[],"comps":{"K":{"id":"K","fps":24,"layers":[]}}}"#,
                )
                .unwrap(),
            ),
            1,
        );
        let child = Comp::resolve(&d, "K").unwrap();
        let on = parse(r#"{"d":{"keepFrameRate":true}}"#);
        let off = parse(r#"{"d":{}}"#);
        assert_eq!(keep_frame_rate(&off, &child, 0.05), 0.05);
        assert_eq!(keep_frame_rate(&on, &child, 0.05), 1.0 / 24.0);
        assert_eq!(keep_frame_rate(&on, &child, 1.0 / 24.0), 1.0 / 24.0);
        assert_eq!(keep_frame_rate(&on, &child, 0.0), 0.0);
        assert_eq!(keep_frame_rate(&on, &child, -0.01), -1.0 / 24.0);
        let t = 10.0 + 0.5 / 24.0;
        assert_eq!(keep_frame_rate(&on, &child, t), 240.0 / 24.0);
    }
}
