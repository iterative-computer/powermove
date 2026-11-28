//! The intro's picture at a time:
//! 1. Starts zoomed right into the top crown of the sculpted Powermove playhead
//!    and zooms smoothly out while rotating +36° into a live Powermove agent cursor.
//! 2. Roams organically across the user's dimmed desktop like an agent cursor,
//!    with subtle luminous tap ripples and a soft trailing glow.
//! 3. Docks cleanly onto the wordmark's horizontal playhead rail, rotates -36°
//!    back to upright 0°, drops the orange vertical stem, and scrubs STRICTLY
//!    HORIZONTALLY left-to-right across the wordmark to reveal "Powermove" and
//!    lock into the official wordmark (wordmark only — no app icon, and
//!    `Light.mark` fades to 0 as the crisp vector head settles so there is no
//!    white fringe around the playhead).
use super::cue::{CueSheet, smooth};
use super::light::Light;
use crate::brand::{PLAYHEAD_HEAD_CX, PLAYHEAD_HEAD_CY, PLAYHEAD_HEAD_H, WORDMARK_VB_H, WORDMARK_VB_W};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use pm_wave::bezier_ease;

/// The resting wordmark's viewBox height as a fraction of the stage's height.
pub const WORDMARK_HEIGHT: f64 = 0.21;
/// Optical vertical centre of the "Powermove" letterforms on the stage.
const WORDMARK_OPTICAL_Y: f64 = 0.52;
/// Optical vertical centre of the letters inside `wordmark.svg`'s 115.20 viewBox.
const LETTERS_CY_VB: f64 = 68.5;
/// Resting playhead-head height as a fraction of stage height (exact ratio of
/// the playhead head inside `wordmark.svg`).
pub const REST_HEIGHT: f64 = WORDMARK_HEIGHT * (PLAYHEAD_HEAD_H as f64 / WORDMARK_VB_H as f64);
/// Light reach for the ambient studio pool & halo around the wordmark.
const LIGHT_REACH: f64 = 0.20;
/// Starting zoom multiplier on the top of the playhead head: fills the screen
/// with the sculpted crown before exponentially zooming out into the cursor.
const LAUNCH_SCALE: f64 = 32.0;
/// Cursor scale multiplier relative to the resting playhead head while roaming
/// across the desktop (slightly bolder so it reads heroically on a dark stage).
const CURSOR_SCALE: f64 = 1.06;
/// Clockwise rotation (radians) that turns the sculpted playhead head into an
/// up-left pointing agent cursor (+36° aligns its left spine dead vertical,
/// points the sculpted apex at ~11 o'clock, and places the concave notch at the
/// bottom-right tail).
const CURSOR_ANGLE: f64 = 36.0_f64.to_radians();
/// Top edge of the zoomed-in playhead head before creep and at launch
/// (stays clear below the credit while "Iterative Computer presents" holds).
const CREEP_TOP: f64 = 1.04;
const LAUNCH_TOP: f64 = 0.62;
/// While white-hot the outline is softened in mark units so the bloom leads
/// the silhouette before resolving cleanly to the playhead head.
const HOT_SOFTNESS: f64 = 4.0;
/// Top and bottom of the playhead head in mark units from its origin.
const TOP: f64 = -56.0;
const BOTTOM: f64 = 56.0;
/// Pointed cursor apex in mark units (from SVG upper-left corner `(302, 166)`
/// centred on `(495, 334)` — becomes the 11 o'clock pointer tip at +36°).
const TIP_LX: f64 = (302.0 - 495.0) * (112.0 / 368.0);
const TIP_LY: f64 = (166.0 - 334.0) * (112.0 / 368.0);
/// Starting viewBox X of the playhead head on the horizontal rail (just left of 'P').
const RAIL_START_VB: f64 = 4.0;
/// Horizontal offset from `PLAYHEAD_HEAD_CX` to the vertical stem's centre in `wordmark.svg`.
const STEM_OFFSET_VB: f64 = 1.83;
/// The warm white of the stage's type.
pub const INK: u32 = 0xede6dc;

// Choreography timeline (all within `CueSheet::default().hold = 6.55s`).
// Credit holds comfortably through ~3.05s while the top of the playhead rises
// quietly underneath, then the camera zooms out into the agile agent cursor.
const T_ZOOM_START: f64 = 2.45;
const T_ZOOM_END: f64 = 3.35;
const T_LEG1_START: f64 = 3.35;
const T_LEG1_END: f64 = 3.84;
const T_TAP1: f64 = 3.82;
const T_LEG2_START: f64 = 3.89;
const T_LEG2_END: f64 = 4.38;
const T_TAP2: f64 = 4.36;
const T_LEG3_START: f64 = 4.42;
const T_DOCK: f64 = 4.88;
const T_UNROTATE_START: f64 = 4.82;
const T_UNROTATE_END: f64 = 5.08;
/// Moment the vertical orange playhead stem begins dropping; from here onward
/// the full playhead is strictly upright, 1× scale, and moves ONLY horizontally.
pub const T_STEM_START: f64 = 5.08;
const T_STEM_END: f64 = 5.28;
const T_SCRUB_START: f64 = 5.28;
const T_SCRUB_END: f64 = 6.08;

/// The playhead / agent cursor at `t`: its origin (fractions of the stage),
/// its scale (fraction of the stage's height per mark unit), and clockwise
/// rotation in radians (`0` = upright playhead; `CURSOR_ANGLE` = agent cursor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkAt {
    pub x: f64,
    pub y: f64,
    pub unit: f64,
    pub angle: f64,
}
impl MarkAt {
    pub fn top(&self) -> f64 {
        self.y + TOP * self.unit
    }
    pub fn bottom(&self) -> f64 {
        self.y + BOTTOM * self.unit
    }
    pub fn height(&self) -> f64 {
        112. * self.unit
    }
    /// Position of the pointed tip in stage points for a stage of `area`.
    pub fn tip_pt(&self, area: Size<Pixels>) -> (f32, f32) {
        let w = f64::from(f32::from(area.width));
        let h = f64::from(f32::from(area.height));
        let (ca, sa) = (self.angle.cos(), self.angle.sin());
        let sx = TIP_LX * ca - TIP_LY * sa;
        let sy = TIP_LX * sa + TIP_LY * ca;
        (
            (self.x * w + sx * self.unit * h) as f32,
            (self.y * h + sy * self.unit * h) as f32,
        )
    }
}

/// Exact resting rectangle of the wordmark `(left, top, width, height)` in
/// stage points for a stage of `area`.
pub fn wordmark_rect(area: Size<Pixels>) -> (f32, f32, f32, f32) {
    let w = f32::from(area.width);
    let h = f32::from(area.height);
    let wm_h = WORDMARK_HEIGHT as f32 * h;
    let wm_w = wm_h * (WORDMARK_VB_W / WORDMARK_VB_H);
    let left = (w - wm_w) / 2.;
    let top = WORDMARK_OPTICAL_Y as f32 * h - (LETTERS_CY_VB as f32 / WORDMARK_VB_H) * wm_h;
    (left, top, wm_w, wm_h)
}

/// Resting playhead head pose for a stage with `aspect = width / height`.
pub fn rest_for_aspect(aspect: f64) -> MarkAt {
    let aspect = aspect.max(0.5);
    let dx_vb = PLAYHEAD_HEAD_CX as f64 - WORDMARK_VB_W as f64 / 2.;
    let dy_vb = PLAYHEAD_HEAD_CY as f64 - LETTERS_CY_VB;
    MarkAt {
        x: 0.5 + (dx_vb / WORDMARK_VB_H as f64) * WORDMARK_HEIGHT / aspect,
        y: WORDMARK_OPTICAL_Y + (dy_vb / WORDMARK_VB_H as f64) * WORDMARK_HEIGHT,
        unit: REST_HEIGHT / 112.,
        angle: 0.,
    }
}

pub fn rest() -> MarkAt {
    rest_for_aspect(16. / 10.)
}

/// Stage X fraction for a given viewBox X inside `wordmark.svg`.
fn stage_x_for_vb(vb_x: f64, aspect: f64) -> f64 {
    let aspect = aspect.max(0.5);
    let dx_vb = vb_x - WORDMARK_VB_W as f64 / 2.;
    0.5 + (dx_vb / WORDMARK_VB_H as f64) * WORDMARK_HEIGHT / aspect
}

/// Playhead head viewBox X while on the horizontal rail (`t >= T_DOCK`).
fn rail_vb_x(t: f64) -> f64 {
    if t <= T_SCRUB_START {
        RAIL_START_VB
    } else if t >= T_SCRUB_END {
        PLAYHEAD_HEAD_CX as f64
    } else {
        let u = ((t - T_SCRUB_START) / (T_SCRUB_END - T_SCRUB_START)).clamp(0., 1.);
        let ease = bezier_ease([0.22, 1.0, 0.36, 1.0], u);
        RAIL_START_VB + (PLAYHEAD_HEAD_CX as f64 - RAIL_START_VB) * ease
    }
}

/// Cubic Bezier evaluation for 2D cursor trajectories.
fn cubic2(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let n = 1. - t;
    let x = n * n * n * p0.0
        + 3. * n * n * t * p1.0
        + 3. * n * t * t * p2.0
        + t * t * t * p3.0;
    let y = n * n * n * p0.1
        + 3. * n * n * t * p1.1
        + 3. * n * t * t * p2.1
        + t * t * t * p3.1;
    (x, y)
}

/// Smooth cursor tap scale pulse (a crisp subtle click dip at `t_tap`).
fn tap_dip(t: f64, t_tap: f64) -> f64 {
    let dt = t - t_tap;
    if !(0. ..=0.16).contains(&dt) {
        return 1.;
    }
    let s = (dt / 0.16 * std::f64::consts::PI).sin();
    1. - 0.11 * s * s
}

/// Raw cursor/playhead position `(x, y)` before velocity-banking derivation.
fn raw_pos(t: f64, rest: MarkAt, aspect: f64) -> (f64, f64) {
    let rail_x = stage_x_for_vb(RAIL_START_VB, aspect);
    let w0 = (0.50, 0.50);
    let w1 = (0.23, 0.27);
    let w2 = (0.78, 0.30);
    let dock = (rail_x, rest.y);

    if t < T_ZOOM_START {
        let launch_unit = rest.unit * LAUNCH_SCALE;
        let launch_y = LAUNCH_TOP - TOP * launch_unit;
        let creep = smooth(((t - 0.65) / (T_ZOOM_START - 0.65)).clamp(0., 1.));
        let y = launch_y + (CREEP_TOP - LAUNCH_TOP) * (1. - creep);
        (0.5, y)
    } else if t < T_ZOOM_END {
        let launch_unit = rest.unit * LAUNCH_SCALE;
        let launch_y = LAUNCH_TOP - TOP * launch_unit;
        let u = ((t - T_ZOOM_START) / (T_ZOOM_END - T_ZOOM_START)).clamp(0., 1.);
        let ease = bezier_ease([0.22, 0.95, 0.32, 1.0], u);
        (0.5, launch_y + (w0.1 - launch_y) * ease)
    } else if t < T_LEG1_END {
        let u = ((t - T_LEG1_START) / (T_LEG1_END - T_LEG1_START)).clamp(0., 1.);
        let e = bezier_ease([0.42, 0.0, 0.22, 1.0], u);
        cubic2(w0, (0.43, 0.36), (0.31, 0.24), w1, e)
    } else if t < T_LEG2_START {
        w1
    } else if t < T_LEG2_END {
        let u = ((t - T_LEG2_START) / (T_LEG2_END - T_LEG2_START)).clamp(0., 1.);
        let e = bezier_ease([0.40, 0.0, 0.20, 1.0], u);
        cubic2(w1, (0.42, 0.19), (0.62, 0.40), w2, e)
    } else if t < T_LEG3_START {
        w2
    } else if t < T_DOCK {
        let u = ((t - T_LEG3_START) / (T_DOCK - T_LEG3_START)).clamp(0., 1.);
        let e = bezier_ease([0.38, 0.0, 0.18, 1.0], u);
        cubic2(w2, (0.68, 0.65), (rail_x + 0.08, rest.y + 0.16), dock, e)
    } else {
        // Strictly horizontal rail from dock -> resting wordmark lockup!
        (stage_x_for_vb(rail_vb_x(t), aspect), rest.y)
    }
}

pub fn mark_at_aspect(t: f64, _s: &CueSheet, motion: bool, aspect: f64) -> MarkAt {
    let rest = rest_for_aspect(aspect);
    if !motion {
        return rest;
    }
    let (x, y) = raw_pos(t, rest, aspect);

    // Scale: 32× zoomed into top crown -> 1.06× agile agent cursor -> 1.00× locked playhead.
    let size = if t < T_ZOOM_START {
        LAUNCH_SCALE
    } else if t < T_ZOOM_END {
        let u = ((t - T_ZOOM_START) / (T_ZOOM_END - T_ZOOM_START)).clamp(0., 1.);
        let ease = bezier_ease([0.20, 0.96, 0.30, 1.0], u);
        CURSOR_SCALE * (LAUNCH_SCALE / CURSOR_SCALE).powf(1. - ease)
    } else if t < T_UNROTATE_START {
        CURSOR_SCALE * tap_dip(t, T_TAP1) * tap_dip(t, T_TAP2)
    } else if t < T_UNROTATE_END {
        let u = smooth(((t - T_UNROTATE_START) / (T_UNROTATE_END - T_UNROTATE_START)).clamp(0., 1.));
        CURSOR_SCALE + (1.0 - CURSOR_SCALE) * u
    } else {
        1.0
    };

    // Angle: upright (0) while zoomed into top crown -> +36° agent cursor while
    // roaming (with subtle velocity banking) -> back to exact 0° upright playhead
    // before the stem drops and horizontal scrubbing begins.
    let angle = if t < 2.82 {
        0.0
    } else if t < T_ZOOM_END + 0.05 {
        let u = smooth(((t - 2.82) / (T_ZOOM_END + 0.05 - 2.82)).clamp(0., 1.));
        CURSOR_ANGLE * u
    } else if t < T_UNROTATE_START {
        let (x_prev, _) = raw_pos((t - 0.016).max(T_ZOOM_END), rest, aspect);
        let (x_next, _) = raw_pos((t + 0.016).min(T_DOCK), rest, aspect);
        let vx = (x_next - x_prev) / 0.032;
        let bank = (vx * 0.14).clamp(-0.12, 0.12);
        CURSOR_ANGLE + bank
    } else if t < T_UNROTATE_END {
        let u = smooth(((t - T_UNROTATE_START) / (T_UNROTATE_END - T_UNROTATE_START)).clamp(0., 1.));
        CURSOR_ANGLE * (1. - u)
    } else {
        0.0
    };

    MarkAt {
        x,
        y,
        unit: rest.unit * size,
        angle,
    }
}

pub fn mark_at(t: f64, s: &CueSheet, motion: bool) -> MarkAt {
    mark_at_aspect(t, s, motion, 16. / 10.)
}

/// Alpha of the crisp `#F0580A` vector playhead head (`wordmark_head`).
fn vector_head_alpha(t: f64, s: &CueSheet, motion: bool) -> f64 {
    if motion {
        smooth(((t - T_STEM_START) / 0.20).clamp(0., 1.))
    } else {
        smooth(((t - s.credit.2.at) / 1.2).clamp(0., 1.))
    }
}

/// The light on the stage at `t`, for a stage `size` device pixels with
/// `density` pixels per point.
pub fn light_at(t: f64, s: &CueSheet, motion: bool, size: (f32, f32), density: f32) -> Light {
    let aspect = (f64::from(size.0) / f64::from(size.1).max(1.)).max(0.5);
    let m = mark_at_aspect(t, s, motion, aspect);
    let h = f64::from(size.1);
    let creep = smooth(((t - 0.72) / 1.65).clamp(0., 1.));
    // White-hot during the zoomed-in playhead crown, cooling cleanly into
    // luminous orange right as it becomes the roaming agent cursor.
    let cool = smooth(((t - 2.55) / 0.78).clamp(0., 1.));
    // Only the cursor glows — and only at first: its warm bloom fades cleanly
    // to 0 before the playhead docks into the wordmark, and there is no wide
    // ambient floor pool under the wordmark.
    let cursor_glow_out = 1. - smooth(((t - 4.35) / 0.55).clamp(0., 1.));
    let head_a = vector_head_alpha(t, s, motion);
    let (exposure, bloom, pool) = if motion {
        let launch_flare = smooth(((t - 2.40) / 0.42).clamp(0., 1.))
            * (1. - smooth(((t - 2.90) / 0.60).clamp(0., 1.)));
        (
            1. + 26. * (1. - cool),
            creep * (0.42 + 0.50 * launch_flare) * cursor_glow_out,
            0.,
        )
    } else {
        (1., 0., 0.)
    };
    let soft = if motion {
        HOT_SOFTNESS * m.unit * h * (1. - cool)
    } else {
        0.
    };
    // Fade the Metal mark body cleanly to 0 as the crisp `#F0580A` vector
    // `wordmark_head` takes over so no white rim peeks out around the playhead.
    let mark_in = if motion {
        smooth(creep / 0.16) * (1. - head_a)
    } else {
        0.
    };
    Light {
        size: [size.0, size.1],
        centre: [(m.x * f64::from(size.0)) as f32, (m.y * h) as f32],
        scale: (m.unit * h) as f32,
        exposure: exposure as f32,
        bloom: bloom as f32,
        pool: pool as f32,
        mark: mark_in as f32,
        time: if motion { t as f32 } else { 0. },
        density,
        reach: (LIGHT_REACH * h) as f32,
        soft: soft as f32,
        iris: if motion {
            smooth(((t - 2.3) / 1.2).clamp(0., 1.)) as f32
        } else {
            1.
        },
        angle: m.angle as f32,
        pad: 0.,
        domain: [0.; 4],
    }
}

/// The credit: Iterative Computer's own wordmark, "presents" under it in
/// Fraunces Italic, holding clearly over the dimmed desktop before fading out
/// as the agent cursor launches across the screen.
pub fn credit(t: f64, _s: &CueSheet, area: Size<Pixels>, scale: f32) -> AnyElement {
    let out = 1. - smooth(((t - 3.05) / 0.65).clamp(0., 1.));
    let first = smooth(((t - 0.55) / 0.75).clamp(0., 1.)) * out;
    let then = smooth(((t - 0.95) / 0.55).clamp(0., 1.)) * out;
    if first <= 0. {
        return div().into_any_element();
    }
    let h = f32::from(area.height);
    let cap = h * 0.0185;
    let unit = cap / 1400.;
    let baseline = h * 0.44 + cap / 2.;
    let piece = |svg: Credit, top: f32, unit: f32, alpha: f64| {
        let (vw, vh, above) = svg.frame();
        let (w, ht) = (vw * unit, vh * unit);
        let image = credit_image(svg, (ht * scale).round() as u32, (w * scale).round() as u32);
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(top - above * unit))
            .flex()
            .justify_center()
            .opacity(alpha.clamp(0., 1.) as f32)
            .children(image.map(|image| img(ImageSource::Render(image)).w(px(w)).h(px(ht))))
    };
    let small = unit * 0.62;
    div()
        .id("stage-credit")
        .debug_selector(|| "stage-credit".into())
        .absolute()
        .inset_0()
        .child(piece(Credit::Wordmark, baseline, unit, 0.9 * first))
        .child(piece(
            Credit::Presents,
            baseline + 1.7 * cap,
            small,
            0.58 * then,
        ))
        .into_any_element()
}

/// The credit's two outlines.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Credit {
    Wordmark,
    Presents,
}
impl Credit {
    fn svg(self) -> &'static str {
        match self {
            Self::Wordmark => include_str!("../../assets/credit/iterative-computer.svg"),
            Self::Presents => include_str!("../../assets/credit/presents.svg"),
        }
    }
    /// Its view box's width and height and the baseline's depth from its
    /// top, in outline units.
    fn frame(self) -> (f32, f32, f32) {
        match self {
            Self::Wordmark => (14559.5, 1967.5, 1459.),
            Self::Presents => (7681., 1910., 1400.),
        }
    }
}

/// `svg` drawn `w` × `h` device pixels in the stage's ink, cached.
fn credit_image(svg: Credit, h: u32, w: u32) -> Option<std::sync::Arc<RenderImage>> {
    use resvg::{tiny_skia, usvg};
    use std::{cell::RefCell, collections::HashMap, sync::Arc};
    thread_local! {
        static CACHE: RefCell<HashMap<(Credit, u32), Option<Arc<RenderImage>>>> = RefCell::default();
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&(svg, h)).cloned()) {
        return hit;
    }
    let draw = || {
        let tree = usvg::Tree::from_str(svg.svg(), &usvg::Options::default()).ok()?;
        let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1))?;
        let size = tree.size();
        let transform =
            tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        let (r, g, b) = ((INK >> 16) as u8, (INK >> 8) as u8, INK as u8);
        let pixels = pixmap
            .pixels()
            .iter()
            .flat_map(|p| [b, g, r, p.alpha()])
            .collect();
        let buffer = image::RgbaImage::from_raw(w.max(1), h.max(1), pixels)?;
        Some(Arc::new(RenderImage::new([image::Frame::new(buffer)])))
    };
    let image = draw();
    CACHE.with(|c| c.borrow_mut().insert((svg, h), image.clone()));
    image
}

/// Subtle agent-cursor visual cues (delicate luminous waypoint ripples at the
/// cursor tip and a faint trailing glow) while the cursor roams across the
/// dimmed desktop, plus the official Powermove wordmark reveal as the upright
/// full playhead scrubs strictly horizontally into lockup.
pub fn words(
    t: f64,
    s: &CueSheet,
    area: Size<Pixels>,
    _tokens: &Tokens,
    scale: f32,
    motion: bool,
) -> AnyElement {
    if t >= s.length() {
        return div().into_any_element();
    }
    let w = f32::from(area.width);
    let h = f32::from(area.height);
    let aspect = (f64::from(w) / f64::from(h).max(1.)).max(0.5);
    let (left, top, wm_w, wm_h) = wordmark_rect(area);

    let mut root = div()
        .id("stage-words")
        .debug_selector(|| "stage-words".into())
        .absolute()
        .inset_0();

    // 1. Agent cursor trail & waypoint tap ripples while roaming (`3.32..5.05s`).
    if motion && (3.32..5.05).contains(&t) {
        let cur = mark_at_aspect(t, s, true, aspect);
        let cur_x = (cur.x * f64::from(w)) as f32;
        let cur_y = (cur.y * f64::from(h)) as f32;
        let trail_fade = smooth(((t - 3.32) / 0.20).clamp(0., 1.))
            * (1. - smooth(((t - 4.72) / 0.22).clamp(0., 1.)));
        if trail_fade > 0.01 {
            for k in 1..=7 {
                let ts = (t - k as f64 * 0.022).max(T_ZOOM_END);
                let prev = mark_at_aspect(ts, s, true, aspect);
                let cx = (prev.x * f64::from(w)) as f32;
                let cy = (prev.y * f64::from(h)) as f32;
                let dist = ((cx - cur_x).powi(2) + (cy - cur_y).powi(2)).sqrt();
                if dist < 24. {
                    continue;
                }
                let frac = 1. - k as f32 / 8.0;
                let r = 1.8 + 3.4 * frac;
                let alpha = (0.26 * frac * frac * trail_fade as f32).clamp(0., 1.);
                root = root.child(
                    div()
                        .absolute()
                        .left(px(cx - r))
                        .top(px(cy - r))
                        .w(px(r * 2.))
                        .h(px(r * 2.))
                        .rounded_full()
                        .bg(rgba((0xff7828u32 << 8) | (alpha * 255.).round() as u32)),
                );
            }
        }
        // Soft expanding luminous tap ripples right at the cursor's pointed tip.
        for t_tap in [T_TAP1, T_TAP2] {
            let dt = t - t_tap;
            if (0.0..0.48).contains(&dt) {
                let p = bezier_ease([0.16, 1.0, 0.3, 1.0], dt / 0.48) as f32;
                let alpha = ((1. - (dt / 0.48) as f32).powf(1.6) * 0.55).clamp(0., 1.);
                let radius = 4. + 26. * p;
                let at_tap = mark_at_aspect(t_tap, s, true, aspect);
                let (tx, ty) = at_tap.tip_pt(area);
                let ring_col = rgba((0xff8a3du32 << 8) | (alpha * 255.).round() as u32);
                root = root.child(
                    div()
                        .absolute()
                        .left(px(tx - radius))
                        .top(px(ty - radius))
                        .w(px(radius * 2.))
                        .h(px(radius * 2.))
                        .rounded_full()
                        .border_2()
                        .border_color(ring_col),
                );
            }
        }
    }

    // 2. Full playhead (upright `#F0580A` vector head + dropping vertical stem)
    //    scrubbing STRICTLY HORIZONTALLY to reveal "Powermove" (`t >= T_STEM_START`).
    let head_a = vector_head_alpha(t, s, motion);
    let (stem_p, playhead_x_vb, right_text_a) = if motion {
        let stem = smooth(((t - T_STEM_START) / (T_STEM_END - T_STEM_START)).clamp(0., 1.));
        let px_vb = rail_vb_x(t);
        let right_a = smooth(((t - 5.52) / (T_SCRUB_END - 5.52)).clamp(0., 1.));
        (stem, px_vb, right_a)
    } else {
        let up = smooth(((t - s.credit.2.at) / 1.2).clamp(0., 1.));
        let text = smooth(((t - s.credit.2.at - 0.35) / 1.1).clamp(0., 1.));
        (up, PLAYHEAD_HEAD_CX as f64, text)
    };

    if head_a <= 0. && stem_p <= 0. && right_text_a <= 0. {
        return root.into_any_element();
    }

    // Horizontal translation of the playhead relative to its resting wordmark lockup
    // (`0.0` once docked at `PLAYHEAD_HEAD_CX`; strictly horizontal, zero Y offset!).
    let dx_px = ((playhead_x_vb - PLAYHEAD_HEAD_CX as f64) / WORDMARK_VB_H as f64) as f32 * wm_h;
    let stem_x_px = (((playhead_x_vb + STEM_OFFSET_VB) / WORDMARK_VB_H as f64) as f32 * wm_h)
        .clamp(0., wm_w);
    let stem_top = (40.5 / WORDMARK_VB_H) * wm_h;
    let stem_full = ((93.0 - 40.5) / WORDMARK_VB_H) * wm_h;
    let stem_clip_h = stem_top + stem_full * stem_p as f32;
    let left_text_a = if motion {
        smooth(((t - T_SCRUB_START) / 0.14).clamp(0., 1.))
    } else {
        right_text_a
    };

    let wm_box = div()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(wm_w))
        .h(px(wm_h))
        // Letters swept open behind the horizontally scrubbing playhead (`0 .. stem_x_px`).
        .when(left_text_a > 0. && stem_x_px > 0.5, |d| {
            d.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(stem_x_px))
                    .h(px(wm_h))
                    .overflow_hidden()
                    .opacity(left_text_a.clamp(0., 1.) as f32)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .w(px(wm_w))
                            .h(px(wm_h))
                            .child(crate::brand::wordmark_text(wm_h, scale)),
                    ),
            )
        })
        // Remaining right half of "Powermove" (`stem_x_px .. wm_w`) illuminating cleanly
        // as the playhead glides into its lockup on 'm' — disjoint clip avoids double alpha.
        .when(right_text_a > 0. && wm_w - stem_x_px > 0.5, |d| {
            d.child(
                div()
                    .absolute()
                    .left(px(stem_x_px))
                    .top_0()
                    .w(px(wm_w - stem_x_px))
                    .h(px(wm_h))
                    .overflow_hidden()
                    .opacity(right_text_a.clamp(0., 1.) as f32)
                    .child(
                        div()
                            .absolute()
                            .left(px(-stem_x_px))
                            .top_0()
                            .w(px(wm_w))
                            .h(px(wm_h))
                            .child(crate::brand::wordmark_text(wm_h, scale)),
                    ),
            )
        })
        // Orange vertical playhead stem + motion trail (strictly horizontal `dx_px`, `top = 0`).
        .when(stem_p > 0., |d| {
            d.child(
                div()
                    .absolute()
                    .left(px(dx_px))
                    .top_0()
                    .w(px(wm_w))
                    .h(px(stem_clip_h))
                    .overflow_hidden()
                    .opacity(stem_p.clamp(0., 1.) as f32)
                    .child(crate::brand::wordmark_stem(wm_h, scale)),
            )
        })
        // Crisp `#F0580A` vector playhead head riding strictly horizontally (`dx_px`, `top = 0`).
        // With `Light.mark == 0` once settled, there is zero white rim around the head.
        .when(head_a > 0., |d| {
            d.child(
                div()
                    .absolute()
                    .left(px(dx_px))
                    .top_0()
                    .w(px(wm_w))
                    .h(px(wm_h))
                    .opacity(head_a.clamp(0., 1.) as f32)
                    .child(crate::brand::wordmark_head(wm_h, scale)),
            )
        });

    root.child(wm_box).into_any_element()
}

#[cfg(test)]
mod tests {
    /// Once the playhead stem begins dropping (`T_STEM_START`), the full
    /// playhead stays strictly upright at 1× scale and moves ONLY horizontally,
    /// and `Light.mark` fades to 0 once settled so no white fringe peeks out.
    #[test]
    fn the_full_playhead_moves_strictly_horizontally() {
        let s = super::CueSheet::default();
        let aspect = 16. / 10.;
        let rest = super::rest_for_aspect(aspect);
        let mut prev_x = f64::NEG_INFINITY;
        for step in 0..=200 {
            let t = super::T_STEM_START + (6.60 - super::T_STEM_START) * (step as f64 / 200.);
            let m = super::mark_at_aspect(t, &s, true, aspect);
            assert!(
                (m.y - rest.y).abs() < 1e-9,
                "t={t:.3}: full playhead moved vertically: y={} != {}",
                m.y,
                rest.y
            );
            assert!(
                m.angle.abs() < 1e-9,
                "t={t:.3}: full playhead rotated: angle={}",
                m.angle
            );
            assert!(
                (m.unit - rest.unit).abs() < 1e-9,
                "t={t:.3}: full playhead scaled: unit={} != {}",
                m.unit,
                rest.unit
            );
            assert!(
                m.x >= prev_x - 1e-9,
                "t={t:.3}: full playhead went backwards: {} < {}",
                m.x,
                prev_x
            );
            prev_x = m.x;
        }
        let end = super::mark_at_aspect(6.55, &s, true, aspect);
        assert!((end.x - rest.x).abs() < 1e-9);
        let end_light = super::light_at(6.5, &s, true, (1440., 900.), 2.);
        assert_eq!(end_light.mark, 0.);
        assert_eq!(end_light.bloom, 0.);
        assert_eq!(end_light.pool, 0.);
    }

    /// The light pass on the GPU, read back: a lit, opaque orange mark with
    /// nothing around it but its light (clear: the desktop's dim shows),
    /// with and without motion (a uniform layout mismatch paints it flat).
    #[test]
    fn the_light_pass_draws_a_lit_mark_on_a_clear_ground() {
        let s = super::CueSheet::default();
        let device = metal::Device::system_default().unwrap();
        let mut r = super::super::light::LightRenderer::new(&device).unwrap();
        for motion in [true, false] {
            let mut l = super::light_at(3.6, &s, motion, (302., 196.), 0.2);
            if !motion {
                l.mark = 1.;
            }
            let tex = r.render(l).unwrap();
            let t = &tex.texture;
            let d = metal::TextureDescriptor::new();
            d.set_width(t.width());
            d.set_height(t.height());
            d.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
            d.set_storage_mode(metal::MTLStorageMode::Shared);
            let out = device.new_texture(&d);
            let q = device.new_command_queue();
            let cmd = q.new_command_buffer();
            cmd.encode_wait_for_event(&tex.event, tex.value);
            let b = cmd.new_blit_command_encoder();
            b.copy_from_texture(
                t,
                0,
                0,
                metal::MTLOrigin { x: 0, y: 0, z: 0 },
                metal::MTLSize {
                    width: t.width(),
                    height: t.height(),
                    depth: 1,
                },
                &out,
                0,
                0,
                metal::MTLOrigin { x: 0, y: 0, z: 0 },
            );
            b.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
            let mut px = vec![0u8; (t.width() * t.height() * 4) as usize];
            out.get_bytes(
                px.as_mut_ptr().cast(),
                t.width() * 4,
                metal::MTLRegion::new_2d(0, 0, t.width(), t.height()),
                0,
            );
            let at = |x: u64, y: u64| {
                let i = ((y * t.width() + x) * 4) as usize;
                [px[i + 2], px[i + 1], px[i], px[i + 3]]
            };
            let (top, centre) = (
                at(151, 5),
                at(l.centre[0].round() as u64, l.centre[1].round() as u64),
            );
            // Away from the mark: no ground, only a little of its light.
            assert_eq!(top[3], 0, "{motion}: clear around the mark: {top:?}");
            assert!(
                top[..3].iter().all(|v| *v < 90),
                "{motion}: only light around the mark: {top:?}"
            );
            // The mark: lit vibrant Powermove orange and opaque.
            assert_eq!(centre[3], 255, "{motion}: the mark covers: {centre:?}");
            assert!(
                centre[0] > 200 && centre[1] > 110 && centre[2] > 40,
                "{motion}: the mark is lit: {centre:?}"
            );
        }
    }
}
