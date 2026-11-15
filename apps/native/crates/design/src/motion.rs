//! The one motion system: substepped, damped springs with velocity.
//!
//! Every animated value in the app (hover washes, a gliding selection, a
//! surface entering, a row sliding to a new place) is an [`Animated`] driven by
//! a [`Preset`]. Retargeting keeps the current value and velocity, so motion is
//! always interruptible. Four substeps per frame keep the same constants stable
//! at 60 and 120 Hz. A value at rest reports `false` from [`Animated::step`],
//! and its owner stops requesting frames: nothing ticks while nothing moves.
//!
//! Reduce Motion: place, scale and pop presets snap; opacity presets become
//! the `motion.reduced` cross-fade (a critically damped spring that settles in
//! that time). See [`crate::Tokens::spring`].
use std::time::Instant;

/// Stiffness `k` and damping `c` of a unit-mass spring. Critical damping is
/// `2√k`; below it the value overshoots once (what a pop wants). A zero
/// stiffness snaps straight to the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
}
impl Spring {
    /// Jump to the target on the next step.
    pub const SNAP: Self = Self::new(0., 0.);
    pub const fn new(stiffness: f32, damping: f32) -> Self {
        Self { stiffness, damping }
    }
    /// A spring from a perceptual duration (s) and bounce (0 = critically
    /// damped), the way `motion.enter`/`motion.bounce` are written.
    pub fn perceptual(duration: f64, bounce: f64) -> Self {
        let omega = std::f64::consts::TAU / duration.max(1e-3);
        Self::new((omega * omega) as f32, (2. * omega * (1. - bounce)) as f32)
    }
    /// A critically damped spring that has visibly arrived (within 0.3 %)
    /// after `seconds`.
    pub fn settling(seconds: f64) -> Self {
        let omega = 8. / seconds.max(1e-3);
        Self::new((omega * omega) as f32, (2. * omega) as f32)
    }
    pub fn snaps(self) -> bool {
        self.stiffness <= 0.
    }
}

/// Named springs. Values are tuned by feel at 60 and 120 Hz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    /// Places and sizes: rows sliding, a selection gliding between tabs or
    /// segments, panels resizing. Slightly underdamped (k 420, c 31).
    Layout,
    /// A row or control appearing (k 260, c 30).
    Show,
    /// Hover wash arriving under the pointer: quick (k 1100, c 66).
    HoverIn,
    /// Hover wash leaving after the pointer left the control: slow (k 300, c 35).
    HoverOut,
    /// The wash the pointer just left for a neighbour: gone in about 80 ms
    /// (k 2500, c 100), so two washes never read as one smear.
    HoverSwitch,
    /// An icon or check confirming an action: one overshoot (k 600, c 15).
    Pop,
    /// A surface (popover, menu, palette, sheet content) entering:
    /// `motion.enter` with `motion.bounce`, scale from `motion.start_scale`.
    Enter,
    /// A surface leaving: no bounce, settles in `motion.exit`.
    Exit,
}
impl Preset {
    /// The preset's spring with full motion.
    pub fn spring(self, motion: &crate::Motion) -> Spring {
        match self {
            Self::Layout => Spring::new(420., 31.),
            Self::Show => Spring::new(260., 30.),
            Self::HoverIn => Spring::new(1100., 66.),
            Self::HoverOut => Spring::new(300., 35.),
            Self::HoverSwitch => Spring::new(2500., 100.),
            Self::Pop => Spring::new(600., 15.),
            Self::Enter => Spring::perceptual(motion.enter, motion.bounce),
            Self::Exit => Spring::settling(motion.exit),
        }
    }
    /// Presets that move or scale something (as opposed to fading it).
    pub fn moves(self) -> bool {
        matches!(self, Self::Layout | Self::Pop | Self::Enter)
    }
}

/// `N` values animated together (x/y/width/height of a glider, one opacity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animated<const N: usize = 1> {
    pub value: [f32; N],
    pub velocity: [f32; N],
    pub target: [f32; N],
    pub spring: Spring,
    /// Distance from the target that counts as there (velocity: ×10).
    pub precision: f32,
}
/// Rest precision for 0–1 values (opacity, mix, scale).
pub const UNIT_PRECISION: f32 = 0.002;
/// Rest precision for values in points (positions, sizes).
pub const POINT_PRECISION: f32 = 0.05;
const SUBSTEPS: usize = 4;
/// The longest step the integration takes at once; a longer frame is
/// stepped in pieces of this, so the spring stays stable.
const MAX_STEP: f32 = 1. / 30.;
/// The most one frame advances a value: a slow frame moves it by the time
/// it took (as Core Animation does), so motion ends on time and a heavy
/// surface isn't drawn again for every 1/120 s of an animation (Settings'
/// page switches, owner feedback S1); a long stall catches up only this far.
const MAX_FRAME: f32 = 0.25;

impl<const N: usize> Animated<N> {
    pub fn new(value: [f32; N], spring: Spring, precision: f32) -> Self {
        Self {
            value,
            velocity: [0.; N],
            target: value,
            spring,
            precision,
        }
    }
    /// Head for `target` from wherever the value is now, keeping its velocity.
    pub fn animate_to(&mut self, target: [f32; N], spring: Spring) {
        self.target = target;
        self.spring = spring;
        if spring.snaps() {
            self.snap(target);
        }
    }
    /// Jump to `value` and rest there.
    pub fn snap(&mut self, value: [f32; N]) {
        self.value = value;
        self.target = value;
        self.velocity = [0.; N];
    }
    pub fn moving(&self) -> bool {
        self.value != self.target || self.velocity.iter().any(|v| *v != 0.)
    }
    /// Advance by `dt` seconds. Returns whether the value still moves.
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.moving() {
            return false;
        }
        if self.spring.snaps() {
            self.snap(self.target);
            return false;
        }
        let dt = dt.clamp(0., MAX_FRAME);
        let pieces = (dt / MAX_STEP).ceil().max(1.) as usize;
        let h = dt / (pieces * SUBSTEPS) as f32;
        let Spring { stiffness, damping } = self.spring;
        let mut moving = false;
        for i in 0..N {
            for _ in 0..pieces * SUBSTEPS {
                self.velocity[i] +=
                    (stiffness * (self.target[i] - self.value[i]) - damping * self.velocity[i]) * h;
                self.value[i] += self.velocity[i] * h;
            }
            if (self.value[i] - self.target[i]).abs() < self.precision
                && self.velocity[i].abs() < self.precision * 10.
            {
                self.value[i] = self.target[i];
                self.velocity[i] = 0.;
            } else {
                moving = true;
            }
        }
        moving
    }
}
impl Animated<1> {
    /// A 0–1 value (opacity, mix) at rest at `value`.
    pub fn unit(value: f32) -> Self {
        Self::new([value], Spring::SNAP, UNIT_PRECISION)
    }
    pub fn get(&self) -> f32 {
        self.value[0]
    }
    pub fn to(&mut self, target: f32, spring: Spring) {
        self.animate_to([target], spring);
    }
}

/// Frame clock for one animating owner: the time since its last frame, and a
/// reset when everything rests so the next start never sees a long gap.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    last: Option<Instant>,
}
impl Clock {
    /// Seconds since the previous tick (0 on the first tick after rest).
    pub fn tick(&mut self, now: Instant) -> f32 {
        let dt = self
            .last
            .map_or(0., |last| now.saturating_duration_since(last).as_secs_f32());
        self.last = Some(now);
        dt
    }
    pub fn rest(&mut self) {
        self.last = None;
    }
}

/// Closed-form damped oscillator (any mass) for handing velocity between
/// Core Animation springs (`CASpringAnimation` on AppKit popups): sample where
/// the previous spring is, then start the next one from there.
#[derive(Clone, Copy, Debug)]
pub struct Analytic {
    pub from: f64,
    pub velocity: f64,
    pub target: f64,
    pub mass: f64,
    pub stiffness: f64,
    pub damping: f64,
}
impl Analytic {
    pub fn sample(self, seconds: f64) -> (f64, f64) {
        let alpha = self.damping / (2. * self.mass);
        let omega2 = self.stiffness / self.mass;
        let x = self.from - self.target;
        let d = omega2 - alpha * alpha;
        if d > f64::EPSILON {
            let w = d.sqrt();
            let b = (self.velocity + alpha * x) / w;
            let (s, c) = (w * seconds).sin_cos();
            let decay = (-alpha * seconds).exp();
            let y = x * c + b * s;
            let v = -alpha * y - x * w * s + b * w * c;
            (self.target + decay * y, decay * v)
        } else if d.abs() <= f64::EPSILON {
            let b = self.velocity + alpha * x;
            let decay = (-alpha * seconds).exp();
            let y = x + b * seconds;
            (self.target + decay * y, decay * (b - alpha * y))
        } else {
            let root = (-d).sqrt();
            let r1 = -alpha + root;
            let r2 = -alpha - root;
            let a = (self.velocity - r2 * x) / (r1 - r2);
            let b = x - a;
            let (e1, e2) = ((r1 * seconds).exp(), (r2 * seconds).exp());
            (self.target + a * e1 + b * e2, a * r1 * e1 + b * r2 * e2)
        }
    }
}
