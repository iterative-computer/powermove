//! Native scrub gesture: one transaction; authored typing remains Controls' path.
use crate::{
    Id, Surface,
    binding::{BindingSink, Bindings, Scrub, scrub},
};
use pm_design::Tokens;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrubEffect {
    Pending,
    CapturePointer,
    Value(f64),
    EditText,
    ReleasePointer,
}
#[derive(Debug)]
pub struct NumericScrub {
    id: Id,
    binding: String,
    origin: (f64, f64),
    start: f64,
    step: f64,
    speed: f64,
    bounds: (f64, f64),
    threshold: f64,
    active: bool,
    capture_requested: bool,
    finished: bool,
    last: Option<f64>,
}
impl NumericScrub {
    pub fn new(
        surface: &Surface,
        id: Id,
        origin: (f64, f64),
        tokens: &Tokens,
    ) -> Result<Self, String> {
        let node = surface.node(id).ok_or("missing numeric target")?;
        if node.props.get("control").and_then(Value::as_str) != Some("number")
            || node.props.get("disabled").and_then(Value::as_bool) == Some(true)
        {
            return Err("not an enabled numeric control".into());
        }
        let number = |key: &str, default| {
            node.props
                .get(key)
                .and_then(Value::as_f64)
                .unwrap_or(default)
        };
        let start = number("value", f64::NAN);
        let step = number("step", 1.);
        let speed = number("speed", 0.5); // W073 / language.md §9 behavior, not a visual value.
        let bounds = (number("min", -f64::MAX), number("max", f64::MAX));
        let threshold = f64::from(tokens.metrics.drag_threshold);
        if ![
            origin.0, origin.1, start, step, speed, bounds.0, bounds.1, threshold,
        ]
        .iter()
        .all(|v| v.is_finite())
            || step <= 0.
            || speed <= 0.
            || threshold < 0.
            || bounds.0 > bounds.1
        {
            return Err("invalid numeric descriptor".into());
        }
        Ok(Self {
            id,
            binding: node
                .props
                .get("binding")
                .and_then(Value::as_str)
                .ok_or("missing numeric binding")?
                .into(),
            origin,
            start,
            step,
            speed,
            bounds,
            threshold,
            active: false,
            capture_requested: false,
            finished: false,
            last: None,
        })
    }
    pub fn sample(
        &mut self,
        surface: &Surface,
        position: (f64, f64),
        shift: bool,
        alt: bool,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<ScrubEffect>, String> {
        if self.finished {
            return Err("numeric gesture already finished".into());
        }
        if !surface.node(self.id).is_some_and(|n| {
            n.props.get("binding").and_then(Value::as_str) == Some(&self.binding)
                && n.props.get("disabled").and_then(Value::as_bool) != Some(true)
        }) {
            return Err("retired numeric descriptor; cancel original gesture".into());
        }
        let delta = (position.0 - self.origin.0, position.1 - self.origin.1);
        let value = scrub(Scrub {
            start: self.start,
            delta,
            step: self.step,
            speed: self.speed,
            shift,
            alt,
            bounds: self.bounds,
        })?;
        if !self.active && delta.0.hypot(delta.1) < self.threshold {
            return Ok(vec![ScrubEffect::Pending]);
        }
        let mut effects = vec![];
        if !self.active {
            bindings.begin(surface, self.id, sink)?;
            self.active = true;
        }
        if self.last != Some(value) {
            bindings.write(self.id, json!(value), sink)?;
            self.last = Some(value);
            effects.push(ScrubEffect::Value(value));
        }
        if !self.capture_requested {
            self.capture_requested = true;
            effects.insert(0, ScrubEffect::CapturePointer);
        }
        Ok(effects)
    }
    /// Host executes pointer effects without a JS round trip. Failed queue writes
    /// leave the gesture retryable. On retire/disable use cancel before reconcile.
    pub fn finish(
        &mut self,
        cancel: bool,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<ScrubEffect>, String> {
        if self.finished {
            return Ok(vec![]);
        }
        if self.active {
            bindings.finish(self.id, cancel, sink)?;
            self.active = false;
            self.finished = true;
            Ok(vec![ScrubEffect::ReleasePointer])
        } else {
            self.finished = true;
            Ok(if cancel {
                vec![]
            } else {
                vec![ScrubEffect::EditText]
            })
        }
    }
}
/// Electron horizontal-scrub.ts: pixel gain, dominant-axis admission and 300ms rest.
/// Values accumulate without rounding so fractional trackpad samples are preserved.
#[derive(Debug)]
pub struct HorizontalScrub {
    pub current: f64,
    pub active: bool,
}
impl HorizontalScrub {
    pub fn new(value: f64) -> Self {
        Self {
            current: value,
            active: false,
        }
    }
    pub fn accepts(&self, dx: f64, dy: f64, control: bool) -> bool {
        !control
            && dx.is_finite()
            && dy.is_finite()
            && (self.active || (dx != 0. && dx.abs() > dy.abs()))
    }
    pub fn sample(
        &mut self,
        dx: f64,
        step: f64,
        speed: f64,
        shift: bool,
        alt: bool,
        bounds: (f64, f64),
    ) -> f64 {
        let multiplier = if shift {
            10.
        } else if alt {
            0.1
        } else {
            1.
        };
        self.active = true;
        self.current =
            (self.current + dx * 0.25 * step * speed * multiplier).clamp(bounds.0, bounds.1);
        let scaled = self.current * 10_000.;
        if scaled.is_finite() {
            scaled.round() / 10_000.
        } else {
            self.current
        }
    }
}
pub const HORIZONTAL_SCRUB_REST: std::time::Duration = std::time::Duration::from_millis(300);

#[cfg(test)]
mod horizontal_scrub_tests {
    use super::*;
    #[test]
    fn electron_pixel_gain_modifiers_and_fractional_accumulation() {
        for (shift, alt, expected) in [
            (false, false, 3.),
            (true, false, 30.),
            (false, true, 0.3),
            (true, true, 30.),
        ] {
            let mut w = HorizontalScrub::new(0.);
            assert_eq!(w.sample(24., 1., 0.5, shift, alt, (-100., 100.)), expected);
        }
        let mut w = HorizontalScrub::new(0.);
        for _ in 0..100 {
            w.sample(0.001, 1., 0.5, false, false, (-100., 100.));
        }
        assert!((w.current - 0.0125).abs() < 1e-10);
    }
    #[test]
    fn vertical_scroll_hands_off_until_horizontal_intent() {
        let mut w = HorizontalScrub::new(0.);
        assert!(!w.accepts(0., 20., false));
        assert!(!w.accepts(2., 20., false));
        assert!(!w.accepts(20., 20., false));
        assert!(!w.accepts(20., 0., true));
        assert!(w.accepts(20., 2., false));
        w.sample(20., 1., 0.5, false, false, (-100., 100.));
        assert!(w.accepts(0., 20., false));
    }
    #[test]
    fn clamp_accumulator_before_reversing_and_keep_angles_unwrapped() {
        let mut w = HorizontalScrub::new(99.);
        assert_eq!(w.sample(100., 1., 0.5, false, false, (0., 100.)), 100.);
        assert_eq!(w.sample(-8., 1., 0.5, false, false, (0., 100.)), 99.);
        let mut angle = HorizontalScrub::new(359.);
        assert_eq!(
            angle.sample(24., 1., 0.5, false, false, (-f64::MAX, f64::MAX)),
            362.
        );
    }
}

/// Incremental, axis-locked pointer travel. Native relative motion is immune
/// to cursor warps and window moves; absolute positions are a headless fallback.
#[derive(Debug)]
pub struct NumericMotion {
    previous: (f64, f64),
    travel: (f64, f64),
    horizontal: Option<bool>,
    current: f64,
}
impl NumericMotion {
    pub fn new(position: (f64, f64), value: f64) -> Self {
        Self {
            previous: position,
            travel: (0., 0.),
            horizontal: None,
            current: value,
        }
    }
    pub fn sample(
        &mut self,
        position: (f64, f64),
        relative: Option<(f64, f64)>,
        threshold: f64,
        step: f64,
        speed: f64,
        shift: bool,
        alt: bool,
        bounds: (f64, f64),
    ) -> Option<f64> {
        let delta =
            relative.unwrap_or((position.0 - self.previous.0, position.1 - self.previous.1));
        self.previous = position;
        if ![delta.0, delta.1].iter().all(|n| n.is_finite()) {
            return None;
        }
        self.travel.0 += delta.0;
        self.travel.1 += delta.1;
        let distance = if let Some(horizontal) = self.horizontal {
            if horizontal { delta.0 } else { -delta.1 }
        } else {
            if self.travel.0.hypot(self.travel.1) < threshold {
                return None;
            }
            let horizontal = self.travel.0.abs() >= self.travel.1.abs();
            self.horizontal = Some(horizontal);
            if horizontal {
                self.travel.0
            } else {
                -self.travel.1
            }
        };
        let multiplier = if shift {
            10.
        } else if alt {
            0.1
        } else {
            1.
        };
        self.current =
            (self.current + distance * step * speed * multiplier).clamp(bounds.0, bounds.1);
        let scaled = self.current * 1000.;
        Some(if scaled.is_finite() {
            scaled.round() / 1000.
        } else {
            self.current
        })
    }
}
#[cfg(test)]
mod numeric_motion_tests {
    use super::*;
    #[test]
    fn both_position_fields_are_monotonic_through_warps_in_both_directions() {
        for start in [-1419.72, 1703.42] {
            for direction in [-1., 1.] {
                let mut motion = NumericMotion::new((800., 400.), start);
                let mut previous = start;
                // Absolute cursor warps back across the window. Only hardware
                // travel counts, including a zero-motion synthetic warp event.
                for (x, dx) in [
                    (810., 10.),
                    (820., 10.),
                    (200., 0.),
                    (210., 10.),
                    (220., 10.),
                    (100., 0.),
                ] {
                    let next = motion
                        .sample(
                            (x, 400.),
                            Some((dx * direction, 0.)),
                            3.,
                            1.,
                            0.5,
                            false,
                            false,
                            (-1e6, 1e6),
                        )
                        .unwrap();
                    assert!((next - previous) * direction >= 0.);
                    previous = next;
                }
                assert!((previous - (start + 20. * direction)).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn perpendicular_jitter_and_modifier_changes_never_rebase_the_value() {
        let mut motion = NumericMotion::new((0., 0.), 0.);
        assert_eq!(
            motion.sample((4., 1.), None, 3., 1., 0.5, false, false, (-100., 100.)),
            Some(2.)
        );
        assert_eq!(
            motion.sample((6., 8.), None, 3., 1., 0.5, false, false, (-100., 100.)),
            Some(3.)
        );
        assert_eq!(
            motion.sample((8., 8.), None, 3., 1., 0.5, false, true, (-100., 100.)),
            Some(3.1)
        );
    }
}
