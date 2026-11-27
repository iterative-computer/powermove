//! The legacy easing library, ported from `legacy/core/easing.ts` (`PM.Ease`).
//!
//! * [`bezier`]: the cubic-bezier solver used for color keys and presets
//!   (Newton, then bisection, with the legacy constants: 6 Newton steps at
//!   1e-6, 24 bisection steps).
//! * [`PRESETS`], [`preset`], [`handles`], [`name_of`].
//! * [`SpringTables::spring`]: the spring table. The simulation runs in `f64` at a fixed
//!   1/480 s step and stores positions in a `Float32Array`, so every answer is
//!   an `f32` widened to `f64`, as in V8.
//! * [`spring_duration`].
//!
//! Operation order follows the TypeScript exactly (no fused multiply-add).

use std::collections::HashMap;
use std::sync::Arc;

use pm_model::Node;

use crate::node;

/// A unit-square cubic bezier `(x1, y1, x2, y2)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bezier {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    identity: bool,
    ax: f64,
    bx: f64,
    cx: f64,
    ay: f64,
    by: f64,
    cy: f64,
}

/// `Ease.bezier(x1, y1, x2, y2)`.
pub fn bezier(x1: f64, y1: f64, x2: f64, y2: f64) -> Bezier {
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    Bezier {
        x1,
        y1,
        x2,
        y2,
        identity: x1 == y1 && x2 == y2,
        ax,
        bx,
        cx,
        ay,
        by,
        cy,
    }
}

impl Bezier {
    pub fn control_points(&self) -> [f64; 4] {
        [self.x1, self.y1, self.x2, self.y2]
    }

    #[inline]
    fn sx(&self, t: f64) -> f64 {
        ((self.ax * t + self.bx) * t + self.cx) * t
    }

    #[inline]
    fn sy(&self, t: f64) -> f64 {
        ((self.ay * t + self.by) * t + self.cy) * t
    }

    #[inline]
    fn dx(&self, t: f64) -> f64 {
        (3.0 * self.ax * t + 2.0 * self.bx) * t + self.cx
    }

    /// The curve's value at progress `x`.
    pub fn eval(&self, x: f64) -> f64 {
        if self.identity {
            return x;
        }
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }
        let mut t = x;
        for _ in 0..6 {
            let d = self.sx(t) - x;
            if d.abs() < 1e-6 {
                return self.sy(t);
            }
            let g = self.dx(t);
            if g.abs() < 1e-6 {
                break;
            }
            t -= d / g;
        }
        let mut lo = 0.0;
        let mut hi = 1.0;
        t = x;
        for _ in 0..24 {
            let d = self.sx(t) - x;
            if d.abs() < 1e-6 {
                break;
            }
            if d > 0.0 {
                hi = t;
            } else {
                lo = t;
            }
            t = (lo + hi) / 2.0;
        }
        self.sy(t)
    }
}

/// The preset curves, in `Object.keys(PRESETS)` order.
pub const PRESETS: [(&str, [f64; 4]); 27] = [
    ("linear", [0.0, 0.0, 1.0, 1.0]),
    ("ease", [0.25, 0.1, 0.25, 1.0]),
    ("easeIn", [0.42, 0.0, 1.0, 1.0]),
    ("easeOut", [0.0, 0.0, 0.58, 1.0]),
    ("easeInOut", [0.42, 0.0, 0.58, 1.0]),
    ("quadIn", [0.11, 0.0, 0.5, 0.0]),
    ("quadOut", [0.5, 1.0, 0.89, 1.0]),
    ("quadInOut", [0.45, 0.0, 0.55, 1.0]),
    ("cubicIn", [0.32, 0.0, 0.67, 0.0]),
    ("cubicOut", [0.33, 1.0, 0.68, 1.0]),
    ("cubicInOut", [0.65, 0.0, 0.35, 1.0]),
    ("quartIn", [0.5, 0.0, 0.75, 0.0]),
    ("quartOut", [0.25, 1.0, 0.5, 1.0]),
    ("quartInOut", [0.76, 0.0, 0.24, 1.0]),
    ("expoIn", [0.7, 0.0, 0.84, 0.0]),
    ("expoOut", [0.16, 1.0, 0.3, 1.0]),
    ("expoInOut", [0.87, 0.0, 0.13, 1.0]),
    ("circIn", [0.55, 0.0, 1.0, 0.45]),
    ("circOut", [0.0, 0.55, 0.45, 1.0]),
    ("circInOut", [0.85, 0.0, 0.15, 1.0]),
    ("backIn", [0.36, 0.0, 0.66, -0.56]),
    ("backOut", [0.34, 1.56, 0.64, 1.0]),
    ("backInOut", [0.68, -0.6, 0.32, 1.6]),
    ("power", [0.62, 0.05, 0.0, 1.0]),
    ("swish", [0.8, 0.0, 0.2, 1.0]),
    ("snap", [0.9, 0.0, 0.1, 1.0]),
    ("glide", [0.16, 0.84, 0.24, 1.0]),
];

/// `PRESETS[name] || PRESETS.linear`.
pub fn preset(name: &str) -> [f64; 4] {
    PRESETS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(PRESETS[0].1, |(_, p)| *p)
}

/// `Ease.fn(name)`.
pub fn preset_curve(name: &str) -> Bezier {
    let p = preset(name);
    bezier(p[0], p[1], p[2], p[3])
}

/// `Ease.handles(name)`: `{ eo: [x1, y1], ei: [x2, y2] }`.
pub fn handles(name: &str) -> ([f64; 2], [f64; 2]) {
    let p = preset(name);
    ([p[0], p[1]], [p[2], p[3]])
}

/// `Ease.nameOf(eo, ei)`: the first preset within 0.02 on every coordinate,
/// or `"custom"`.
pub fn name_of(eo: [f64; 2], ei: [f64; 2]) -> &'static str {
    for (name, p) in PRESETS.iter() {
        if (p[0] - eo[0]).abs() < 0.02
            && (p[1] - eo[1]).abs() < 0.02
            && (p[2] - ei[0]).abs() < 0.02
            && (p[3] - ei[1]).abs() < 0.02
        {
            return name;
        }
    }
    "custom"
}

/// The spring simulation step, `1 / 480` s.
pub const SPRING_STEP: f64 = 1.0 / 480.0;

/// A spring's parameters after JavaScript's destructuring defaults and
/// numeric coercion: `{ damping = 18, stiffness = 180, mass = 1 }`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringConfig {
    pub damping: f64,
    pub stiffness: f64,
    pub mass: f64,
}

impl Default for SpringConfig {
    fn default() -> SpringConfig {
        SpringConfig {
            damping: 18.0,
            stiffness: 180.0,
            mass: 1.0,
        }
    }
}

impl SpringConfig {
    /// Reads a key's `spring` value. A member that is missing takes its
    /// default; any other member is coerced with `ToNumber` (as the
    /// arithmetic does). Primitives and arrays have none of the members.
    pub fn from_node(spring: Option<&Node>) -> SpringConfig {
        let read = |key: &str, default: f64| match node::member(spring, key) {
            None => default,
            Some(n) => node::to_number(Some(n)),
        };
        SpringConfig {
            damping: read("damping", 18.0),
            stiffness: read("stiffness", 180.0),
            mass: read("mass", 1.0),
        }
    }

    fn key(&self) -> (u64, u64, u64) {
        (
            self.damping.to_bits(),
            self.stiffness.to_bits(),
            self.mass.to_bits(),
        )
    }
}

/// The table length, `Math.ceil(6 / SDT)`.
pub fn spring_table_len() -> usize {
    (6.0 / SPRING_STEP).ceil() as usize
}

/// The spring's position table, shared by every track of the process that
/// uses the same spring (a segment's curve is made on its first use, away
/// from an evaluator's [`SpringTables`]).
pub fn shared_spring_table(cfg: SpringConfig) -> Arc<[f32]> {
    static TABLES: std::sync::OnceLock<std::sync::Mutex<SpringTables>> = std::sync::OnceLock::new();
    let mut tables = TABLES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Bounded: tables of springs no longer in use go (a track keeps its own).
    if tables.len() >= 256 {
        *tables = SpringTables::default();
    }
    tables.table(cfg)
}

/// The spring's position table (`Float32Array`).
pub fn spring_table(cfg: SpringConfig) -> Vec<f32> {
    let len = spring_table_len();
    let mut table = Vec::with_capacity(len);
    let mut x = 0.0f64;
    let mut v = 0.0f64;
    for _ in 0..len {
        let a = (-cfg.stiffness * (x - 1.0) - cfg.damping * v) / cfg.mass;
        v += a * SPRING_STEP;
        x += v * SPRING_STEP;
        table.push(x as f32);
    }
    table
}

/// `Math.min(a, b)`: NaN wins (Rust's `f64::min` ignores it).
#[inline]
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b {
        a
    } else {
        b
    }
}

/// Looks `t` up in a spring table: 0 for `t <= 0`; NaN where the legacy code
/// reads `undefined` (a NaN `t`).
pub fn spring_lookup(table: &[f32], t: f64) -> f64 {
    if t <= 0.0 {
        return 0.0;
    }
    let i = js_min((table.len() - 1) as f64, (t / SPRING_STEP).floor());
    if i.is_nan() {
        return f64::NAN;
    }
    f64::from(table[i as usize])
}

/// Spring tables cached by configuration (`springCache`).
///
/// The legacy cache is keyed by the string `damping|stiffness|mass` of the raw
/// values, so two configurations with the same string but different numbers
/// (`true` and `"true"`) share whichever table was built first. This cache is
/// keyed by the coerced numbers, which is what the arithmetic uses; the two
/// agree on every configuration a project can hold except that collision.
#[derive(Clone, Debug, Default)]
pub struct SpringTables {
    tables: HashMap<(u64, u64, u64), Arc<[f32]>>,
}

impl SpringTables {
    pub fn new() -> SpringTables {
        SpringTables::default()
    }

    pub fn table(&mut self, cfg: SpringConfig) -> Arc<[f32]> {
        self.tables
            .entry(cfg.key())
            .or_insert_with(|| spring_table(cfg).into())
            .clone()
    }

    /// `Ease.spring(t, cfg)`.
    pub fn spring(&mut self, t: f64, cfg: SpringConfig) -> f64 {
        if t <= 0.0 {
            return 0.0;
        }
        let table = self.table(cfg);
        spring_lookup(&table, t)
    }

    pub fn len(&self) -> usize {
        self.tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

/// `Ease.springDuration(cfg)`: the first step at which the spring is within
/// 0.002 of rest in position and velocity, or 10 s.
pub fn spring_duration(cfg: SpringConfig) -> f64 {
    let mut x = 0.0f64;
    let mut v = 0.0f64;
    let limit = 10.0 / SPRING_STEP;
    let mut i = 1.0f64;
    while i < limit {
        let a = (-cfg.stiffness * (x - 1.0) - cfg.damping * v) / cfg.mass;
        v += a * SPRING_STEP;
        x += v * SPRING_STEP;
        if (x - 1.0).abs() < 0.002 && v.abs() < 0.002 {
            return i * SPRING_STEP;
        }
        i += 1.0;
    }
    10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_clamping() {
        let linear = preset_curve("linear");
        assert_eq!(linear.eval(0.3), 0.3);
        assert_eq!(linear.eval(-1.0), -1.0, "the identity curve does not clamp");
        let ease = preset_curve("ease");
        assert_eq!(ease.eval(-1.0), 0.0);
        assert_eq!(ease.eval(2.0), 1.0);
        assert_eq!(preset("nope"), PRESETS[0].1);
    }

    #[test]
    fn spring_table_shape() {
        assert_eq!(spring_table_len(), 2880);
        let mut tables = SpringTables::new();
        let cfg = SpringConfig::default();
        assert_eq!(tables.spring(0.0, cfg), 0.0);
        let end = tables.spring(100.0, cfg);
        assert!((end - 1.0).abs() < 1e-3);
        assert!(tables.spring(f64::NAN, cfg).is_nan());
        assert_eq!(tables.len(), 1);
    }

    #[test]
    fn names() {
        assert_eq!(name_of([0.62, 0.05], [0.0, 1.0]), "power");
        assert_eq!(name_of([0.3, 0.3], [0.9, 0.1]), "custom");
    }
}
