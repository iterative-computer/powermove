//! Pure gesture math, shared by inspector and timeline readouts.
pub fn round(n: f64, places: i32) -> f64 {
    let m = 10_f64.powi(places);
    (n * m + 0.5).floor() / m
}
#[derive(Clone, Debug)]
pub struct NumberOptions {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub speed: f64,
    pub precision: Option<usize>,
    pub unit: String,
}
impl Default for NumberOptions {
    fn default() -> Self {
        Self {
            min: f64::NEG_INFINITY,
            max: f64::INFINITY,
            step: 1.,
            speed: 0.5,
            precision: None,
            unit: String::new(),
        }
    }
}
impl NumberOptions {
    // Preserve the old zero-option defaults.
    pub fn step(&self) -> f64 {
        if self.step == 0. { 1. } else { self.step }
    }
    pub fn speed(&self) -> f64 {
        if self.speed == 0. { 0.5 } else { self.speed }
    }
    pub fn clamp(&self, n: f64) -> f64 {
        round(n.max(self.min).min(self.max), 4)
    }
    pub fn format(&self, n: f64) -> String {
        let p = self
            .precision
            .unwrap_or(if self.step != 0. && self.step < 1. {
                2
            } else if n.abs() < 10. {
                1
            } else {
                0
            })
            .min(12);
        let mut s = fixed_decimal(n, p);
        if p > 0 {
            s = s.trim_end_matches('0').trim_end_matches('.').to_string();
        }
        s + &self.unit
    }
    pub fn scrub(&self, start: f64, dx: f64, dy: f64, shift: bool, alt: bool) -> f64 {
        round(
            self.clamp(start + (dx - dy) * self.step() * self.speed() * multiplier(shift, alt)),
            3,
        )
    }
}
pub fn multiplier(shift: bool, alt: bool) -> f64 {
    if shift {
        10.
    } else if alt {
        0.1
    } else {
        1.
    }
}
pub fn past_threshold(dx: f64, dy: f64) -> bool {
    dx.hypot(dy) >= 3.
}
pub fn snap(n: f64, min: f64, max: f64, step: f64) -> f64 {
    round(min + ((n - min) / step).round() * step, 6)
        .max(min)
        .min(max)
}
pub fn point_step(authored: Option<f64>, min: f64, max: f64) -> f64 {
    authored
        .filter(|s| s.is_finite() && *s > 0.)
        .unwrap_or_else(|| if max != min { (max - min) / 100. } else { 1. })
}
pub fn dial_value(degrees: f64, current: f64, min: f64, max: f64) -> f64 {
    let mut n = degrees + ((current - degrees) / 360.).round() * 360.;
    if n < min {
        n += ((min - n) / 360.).ceil() * 360.;
    }
    if n > max {
        n -= ((n - max) / 360.).ceil() * 360.;
    }
    n
}
/// Capture once at gesture start, so moving through zero cannot lose the ratio.
#[derive(Clone, Copy, Debug)]
pub struct LinkedScale {
    pub linked: bool,
    pub animated: bool,
    other: f64,
    ratio: f64,
}
impl LinkedScale {
    pub fn new(primary: f64, other: f64, linked: bool, animated: bool) -> Self {
        Self {
            linked,
            animated,
            other,
            ratio: if primary.abs() > 1e-8 {
                other / primary
            } else {
                1.
            },
        }
    }
    pub fn values(&self, primary: f64) -> (f64, Option<f64>) {
        (
            primary,
            (self.linked || self.animated).then_some(if self.linked {
                primary * self.ratio
            } else {
                self.other
            }),
        )
    }
}
/// No history dependency: consumers map these phases to their document transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum EditEvent<T> {
    Begin,
    Change(T),
    Commit(T),
    Cancel(T),
}
#[derive(Clone, Debug)]
pub struct Gesture<T> {
    before: Option<T>,
    changed: bool,
}
impl<T> Default for Gesture<T> {
    fn default() -> Self {
        Self {
            before: None,
            changed: false,
        }
    }
}
impl<T: Clone + PartialEq> Gesture<T> {
    pub fn active(&self) -> bool {
        self.before.is_some()
    }
    pub fn begin(&mut self, value: &T) -> bool {
        if self.active() {
            false
        } else {
            self.before = Some(value.clone());
            self.changed = false;
            true
        }
    }
    pub fn write(&mut self, value: &T) {
        self.changed |= self.before.as_ref().is_some_and(|v| v != value);
    }
    pub fn finish(&mut self, value: &T) -> Option<EditEvent<T>> {
        self.before.take().map(|before| {
            if self.changed {
                EditEvent::Commit(value.clone())
            } else {
                EditEvent::Cancel(before)
            }
        })
    }
    pub fn cancel(&mut self) -> Option<EditEvent<T>> {
        self.before.take().map(EditEvent::Cancel)
    }
}
/// Horizontal intent stays latched until the 300 ms idle commit.
#[derive(Default, Debug)]
pub struct WheelScrub {
    pub active: bool,
    pub last_ms: f64,
    travel: f64,
    last_haptic: Option<f64>,
}
impl WheelScrub {
    pub fn delta(&mut self, dx: f64, dy: f64, control: bool, now: f64) -> Option<f64> {
        if control || (!self.active && (dx == 0. || dx.abs() <= dy.abs())) {
            return None;
        }
        if dx != 0. {
            self.active = true;
            self.last_ms = now;
        }
        Some(dx * 0.25)
    }
    pub fn haptic(&mut self, delta: f64, changed: bool, now: f64) -> bool {
        if !changed {
            self.travel = 0.;
            return false;
        }
        self.travel += delta.abs();
        if self.travel >= 6. && self.last_haptic.is_none_or(|t| now - t >= 100.) {
            self.travel %= 6.;
            self.last_haptic = Some(now);
            true
        } else {
            false
        }
    }
    pub fn expired(&self, now: f64) -> bool {
        self.active && now - self.last_ms >= 300.
    }
    pub fn finish(&mut self) {
        self.active = false;
        self.travel = 0.;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrub_modifiers_and_bounds() {
        let o = NumberOptions {
            min: -10.,
            max: 20.,
            ..Default::default()
        };
        assert_eq!(o.scrub(0., 4., -2., false, false), 3.);
        assert_eq!(o.scrub(0., 4., 0., true, true), 20.);
        assert_eq!(o.scrub(0., 4., 0., false, true), 0.2);
        assert!(!past_threshold(2., 0.));
        assert!(past_threshold(3., 0.));
        assert_eq!(
            NumberOptions {
                step: 0.,
                speed: 0.,
                ..o
            }
            .scrub(0., 4., 0., false, false),
            2.
        );
    }
    #[test]
    fn formatting() {
        let mut o = NumberOptions::default();
        o.unit = "°".into();
        assert_eq!(o.format(2.), "2°");
        assert_eq!(o.format(2.25), "2.3°");
        o.precision = Some(3);
        assert_eq!(o.format(2.25), "2.25°");
    }
    #[test]
    fn linked_through_zero_and_unlinked_animation() {
        let l = LinkedScale::new(100., 50., true, true);
        assert_eq!(l.values(0.), (0., Some(0.)));
        assert_eq!(l.values(200.), (200., Some(100.)));
        assert_eq!(
            LinkedScale::new(100., 50., false, true).values(180.),
            (180., Some(50.))
        );
        assert_eq!(
            LinkedScale::new(0., 50., true, false).values(20.),
            (20., Some(20.))
        );
    }
    #[test]
    fn transaction_cancel_and_single_commit() {
        let mut g = Gesture::default();
        assert!(g.begin(&10));
        assert!(!g.begin(&20));
        g.write(&12);
        g.write(&13);
        assert_eq!(g.finish(&13), Some(EditEvent::Commit(13)));
        assert_eq!(g.finish(&13), None);
        g.begin(&13);
        g.write(&30);
        assert_eq!(g.cancel(), Some(EditEvent::Cancel(13)));
        g.begin(&13);
        assert_eq!(g.finish(&13), Some(EditEvent::Cancel(13)));
    }
    #[test]
    fn trackpad_intent_idle_and_haptics() {
        let mut w = WheelScrub::default();
        assert_eq!(w.delta(0., 20., false, 0.), None);
        assert_eq!(w.delta(24., 1., false, 0.), Some(6.));
        assert!(w.haptic(6., true, 0.));
        assert!(!w.haptic(6., true, 20.));
        assert_eq!(w.delta(0., 30., false, 100.), Some(0.));
        assert!(!w.expired(299.));
        assert!(w.expired(300.));
        assert!(!w.haptic(20., false, 200.));
        w.finish();
        assert!(!w.active);
    }
    #[test]
    fn normalized_point_and_dial() {
        assert_eq!(point_step(None, 0., 1.), 0.01);
        assert_eq!(snap(0.32, 0.1, 1., 0.2), 0.3);
        assert_eq!(dial_value(-90., 250., 0., 360.), 270.);
    }
}

/// Playback rendering reads at 15 Hz; edits and paused scrubbing read immediately.
/// The host owns the playback clock and invalidation cadence.
#[derive(Clone, Debug)]
pub struct PlaybackReadout<T> {
    sample: Option<T>,
    last_ms: f64,
}
impl<T> Default for PlaybackReadout<T> {
    fn default() -> Self {
        Self {
            sample: None,
            last_ms: f64::NEG_INFINITY,
        }
    }
}
impl<T: Clone> PlaybackReadout<T> {
    pub fn read(
        &mut self,
        now_ms: f64,
        playing: bool,
        editing: bool,
        live: impl FnOnce() -> T,
    ) -> T {
        if !playing
            || editing
            || self.sample.is_none()
            || now_ms - self.last_ms >= 1000. / 15.
            || now_ms < self.last_ms
        {
            self.sample = Some(live());
            self.last_ms = now_ms;
        }
        self.sample.as_ref().unwrap().clone()
    }
}

/// ECMAScript toFixed rounds an exact binary halfway case away from zero.
/// Compute the scaled binary fraction exactly, avoiding ties-to-even formatting.
pub fn fixed_decimal(n: f64, places: usize) -> String {
    if !n.is_finite() || n.abs() >= 1e21 {
        return n.to_string();
    }
    let places = places.min(12);
    let bits = n.abs().to_bits();
    let raw_exp = ((bits >> 52) & 0x7ff) as i32;
    let mantissa = if raw_exp == 0 {
        bits & ((1u64 << 52) - 1)
    } else {
        (bits & ((1u64 << 52) - 1)) | (1u64 << 52)
    };
    let exponent = if raw_exp == 0 {
        -1074
    } else {
        raw_exp - 1023 - 52
    };
    let scaled = mantissa as u128 * 5u128.pow(places as u32);
    let shift = exponent + places as i32;
    let integer = if shift >= 0 {
        let Some(value) = scaled.checked_shl(shift as u32) else {
            return format!("{n:.places$}");
        };
        value
    } else {
        let denominator = (-shift) as u32;
        if denominator >= 128 {
            0
        } else {
            let q = scaled >> denominator;
            let remainder = scaled - (q << denominator);
            q + u128::from(remainder >= (1u128 << (denominator - 1)))
        }
    };
    let sign = if n < 0. { "-" } else { "" };
    if places == 0 {
        return format!("{sign}{integer}");
    }
    let base = 10u128.pow(places as u32);
    format!("{sign}{}.{:0places$}", integer / base, integer % base)
}
