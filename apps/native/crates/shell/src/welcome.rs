//! The Welcome window's content, as data: first-launch gate (N001–N003) and
//! onboarding (S107–S110) share one frame. Headless, so flows test without a screen.

#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// 0 to 1.
    Determinate(f64),
    Indeterminate,
}
/// A Settings-style row in the gate's card: title left, value right, and a
/// progress bar under the row that is running.
#[derive(Clone, Debug, PartialEq)]
pub struct StepRow {
    pub title: String,
    pub value: String,
    pub progress: Option<Progress>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Danger,
    Note,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub symbol: &'static str,
    pub severity: Severity,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub leading: bool,
}
impl Button {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            enabled: true,
            leading: false,
        }
    }
    pub fn leading(mut self) -> Self {
        self.leading = true;
        self
    }
}
/// One mutually exclusive option: a card in a row of cards or a grouped-form row.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    /// Illustration rows on a card, drawn with timeline clip recipes.
    pub lines: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Stable surface id, for inspection and tests.
    pub surface: String,
    /// Plays the first-run animation inside the window.
    pub animate: bool,
    pub back: Option<String>,
    pub eyebrow: Option<String>,
    pub title: String,
    pub line: String,
    pub status: Option<String>,
    pub spinner: bool,
    /// The card under the line: copy steps, or the folders macOS protects.
    pub steps: Vec<StepRow>,
    pub rows: Vec<Row>,
    pub choices: Vec<Choice>,
    pub chosen: Option<String>,
    pub cards: bool,
    pub details: Option<String>,
    pub error: Option<String>,
    pub note: Option<String>,
    pub buttons: Vec<Button>,
    pub default: Option<String>,
}
impl Frame {
    /// Return activates this. Disabled buttons never do.
    pub fn default_action(&self) -> Option<&str> {
        let id = self.default.as_deref()?;
        self.buttons
            .iter()
            .any(|b| b.id == id && b.enabled)
            .then_some(id)
    }
    /// ←/→ step through the options and stop at the ends.
    pub fn stepped_choice(&self, forward: bool) -> Option<&str> {
        let at = self
            .choices
            .iter()
            .position(|c| Some(&c.id) == self.chosen.as_ref())?;
        let next = if forward {
            at.checked_add(1)?
        } else {
            at.checked_sub(1)?
        };
        self.choices.get(next).map(|c| c.id.as_str())
    }
}

/// v1.2.0's scene: duration in seconds and 1920 × 1080 stage.
pub const ANIMATION_SECONDS: f64 = 9.766666666666667;
const STAGE: [f64; 2] = [1920., 1080.];

/// The Powermove mark: anchors with incoming and outgoing tangents, centred.
pub const MARK: [[f64; 6]; 12] = [
    [-37.1115, 48.544, 21.9931, -6.7673, 0., 0.],
    [-65.30264, 57.218, 0., 0., -3.92762, 1.208],
    [-69.38743, 50.648, -2.8211, 2.988, 0., 0.],
    [-42.2946, 21.9531, 0., 0., 14.7007, -15.5703],
    [-12.4301, -33.6651, -4.8582, 20.8553, 0., 0.],
    [-7.4817, -54.90751, 0., 0., 0.9693, -4.16062],
    [0.3251, -54.83898, -0.8961, -4.17699, 0., 0.],
    [3.6448, -39.3633, 0., 0., 5.2402, 24.4279],
    [39.924, 24.2092, -18.366, -16.9375, 0., 0.],
    [68.329, 50.405, 0., 0., 3.089, 2.849],
    [64.539, 57.198, 4.047, 1.132, 0., 0.],
    [30.516, 47.677, 0., 0., -22.1595, -6.2013],
];

#[derive(Clone, Copy)]
struct Key {
    t: f64,
    v: f64,
    /// Outgoing and incoming tangent as (speed, influence %); `None` is linear.
    out: Option<(f64, f64)>,
    into: Option<(f64, f64)>,
}
const fn key(t: f64, v: f64) -> Key {
    Key {
        t,
        v,
        out: None,
        into: None,
    }
}
/// Keyframe value with the editor's speed/influence Bézier interpolation.
fn value(keys: &[Key], t: f64) -> f64 {
    let first = keys[0];
    if t <= first.t {
        return first.v;
    }
    for pair in keys.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t > b.t {
            continue;
        }
        let span = b.t - a.t;
        let (out_speed, out_influence) = a.out.unwrap_or(((b.v - a.v) / span, 100. / 3.));
        let (in_speed, in_influence) = b.into.unwrap_or(((b.v - a.v) / span, 100. / 3.));
        let (x1, x2) = (out_influence / 100., 1. - in_influence / 100.);
        let y1 = a.v + out_speed * span * x1;
        let y2 = b.v - in_speed * span * (1. - x2);
        let curve = |u: f64, p0: f64, p1: f64, p2: f64, p3: f64| {
            let n = 1. - u;
            n * n * n * p0 + 3. * n * n * u * p1 + 3. * n * u * u * p2 + u * u * u * p3
        };
        let target = (t - a.t) / span;
        let (mut low, mut high) = (0., 1.);
        for _ in 0..40 {
            let mid: f64 = (low + high) / 2.;
            if curve(mid, 0., x1, x2, 1.) < target {
                low = mid;
            } else {
                high = mid;
            }
        }
        return curve((low + high) / 2., a.v, y1, y2, b.v);
    }
    keys[keys.len() - 1].v
}

const LARGE_SCALE: [Key; 2] = [key(0., 2160.953), key(3.8333333333333335, 1654.11295)];
const LARGE_Y: [Key; 2] = [key(0., 579.440047), key(3.8333333333333335, 905.315047)];
const LARGE_GLOW: [Key; 4] = [
    Key {
        t: 0.,
        v: 0.,
        out: Some((210.5263157894737, 33.33333333333333)),
        into: None,
    },
    key(1.9, 400.),
    key(3.1333333333333333, 400.),
    key(3.8333333333333335, 0.),
];
const SMALL_SCALE: [Key; 2] = [
    Key {
        t: -0.8333333333333334,
        v: 225.93325,
        out: Some((-126.4870505136986, 0.72)),
        into: None,
    },
    Key {
        t: 2.466666666666667,
        v: 167.152,
        out: None,
        into: Some((-0.42757320205479565, 100.)),
    },
];
const SMALL_GLOW: [Key; 2] = [key(3.6, 400.), key(4.433333333333334, 0.)];
const SECOND_LAYER_START: f64 = 3.848800000143237;
const LARGE_POSITION_X: f64 = 1029.4375;
const SMALL_POSITION: [f64; 2] = [960.717, 540.492];

/// What the window draws at time `t`, on the 1920 × 1080 stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkState {
    pub center: [f64; 2],
    /// Stage points per mark unit.
    pub scale: f64,
    /// Glow strength, 0 to 1.
    pub glow: f64,
}
pub fn mark_at(t: f64) -> Option<MarkState> {
    if t >= ANIMATION_SECONDS {
        return None;
    }
    Some(if t < SECOND_LAYER_START {
        MarkState {
            center: [LARGE_POSITION_X, value(&LARGE_Y, t)],
            scale: value(&LARGE_SCALE, t) / 100.,
            glow: value(&LARGE_GLOW, t) / 400.,
        }
    } else {
        let local = t - SECOND_LAYER_START;
        MarkState {
            center: SMALL_POSITION,
            scale: value(&SMALL_SCALE, local) / 100.,
            glow: value(&SMALL_GLOW, t) / 400.,
        }
    })
}
/// Stage to window: scaled to fit, centred and clipped by the content area.
pub fn fit_stage(window: [f64; 2]) -> (f64, [f64; 2]) {
    let scale = (window[0] / STAGE[0]).min(window[1] / STAGE[1]);
    (
        scale,
        [
            (window[0] - STAGE[0] * scale) / 2.,
            (window[1] - STAGE[1] * scale) / 2.,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_matches_its_authored_keys() {
        let open = mark_at(0.).unwrap();
        assert_eq!(open.scale, 21.60953);
        assert_eq!(open.glow, 0.);
        let bloom = mark_at(2.5).unwrap();
        assert_eq!(bloom.glow, 1.);
        let fading = mark_at(3.48).unwrap();
        assert!((fading.glow - (3.8333333333333335 - 3.48) / 0.7).abs() < 1e-3);
        assert!(mark_at(3.8).unwrap().center[1] > 880.);
        // The cut lands on the small mark, already easing, with the glow still lit.
        let cut = mark_at(SECOND_LAYER_START + 0.01).unwrap();
        assert_eq!(cut.center, SMALL_POSITION);
        assert!(cut.glow > 0.3 && cut.glow < 1.);
        assert_eq!(mark_at(4.5).unwrap().glow, 0.);
        assert!(mark_at(ANIMATION_SECONDS).is_none());
    }
    #[test]
    fn the_small_mark_settles_without_overshoot() {
        let mut last = f64::MAX;
        for frame in 0..60 {
            let scale = mark_at(SECOND_LAYER_START + frame as f64 / 10.)
                .unwrap()
                .scale;
            assert!(scale <= last + 1e-9, "scale rose at {frame}");
            last = scale;
        }
        assert!((last - 1.67152).abs() < 1e-3);
    }
    #[test]
    fn the_stage_fits_the_window_and_stays_centred() {
        let (scale, origin) = fit_stage([900., 640.]);
        assert!((scale - 900. / 1920.).abs() < 1e-9);
        assert_eq!(origin[0], 0.);
        assert!((origin[1] - (640. - 1080. * scale) / 2.).abs() < 1e-9);
    }
    #[test]
    fn return_never_activates_a_disabled_default() {
        let mut frame = Frame {
            buttons: vec![Button::new("continue", "Continue")],
            default: Some("continue".into()),
            ..Frame::default()
        };
        assert_eq!(frame.default_action(), Some("continue"));
        frame.buttons[0].enabled = false;
        assert_eq!(frame.default_action(), None);
    }
    #[test]
    fn arrows_step_through_choices_and_stop_at_the_ends() {
        let choice = |id: &str| Choice {
            id: id.into(),
            label: id.into(),
            detail: None,
            lines: vec![],
        };
        let frame = Frame {
            choices: vec![choice("layers"), choice("tracks")],
            chosen: Some("layers".into()),
            ..Frame::default()
        };
        assert_eq!(frame.stepped_choice(true), Some("tracks"));
        assert_eq!(frame.stepped_choice(false), None);
    }
}
