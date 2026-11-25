//! The stage's own drawing over the engine's room: the credit, and the mark
//! rising from below the bottom edge into the centre in its own light.
use super::cue::{CueSheet, ease_out, smooth, span};
use crate::welcome;
use gpui::{prelude::*, *};
use pm_design::{TEXT_FONT, Tokens};

/// Tessellated outlines, keyed by the geometry they were made for.
type Outlines = (Vec<u32>, Vec<Option<Path<Pixels>>>);
thread_local! {
    /// The mark's outlines for the last geometry (`mark`).
    static OUTLINES: std::cell::RefCell<Outlines> = std::cell::RefCell::default();
}

/// The light on the stage: a warm white, never pure.
pub(super) const LIGHT: u32 = 0xf2_ee_e6;
/// The mark's light: the accent warmed toward the stage's white.
pub(super) fn glow(tokens: &Tokens) -> u32 {
    pm_design::mix(tokens.colors.accent, LIGHT, 0.55)
}

fn ink(alpha: f64) -> Rgba {
    let mut c = Rgba::from(rgb(LIGHT));
    c.alpha = alpha.clamp(0., 1.) as f32;
    c
}

/// The credit near the top: the presenter in fine spaced capitals, then
/// "presents" under it, quiet.
pub fn credit(t: f64, sheet: &CueSheet, tokens: &Tokens) -> AnyElement {
    let (enter, second, leave) = sheet.credit;
    let out = 1. - span(t, leave - 0.3, leave);
    let first = span(t, enter, enter + 0.7) * out;
    let then = span(t, second, second + 0.6) * out;
    if first <= 0. {
        return div().into_any_element();
    }
    let size = tokens.typography.body;
    div()
        .id("stage-credit")
        .debug_selector(|| "stage-credit".into())
        .absolute()
        .left_0()
        .right_0()
        .top(relative(0.36))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(size * 0.5))
        .font_family(TEXT_FONT)
        // OWNER-COPY: the presenter's name.
        .child(
            div()
                .text_size(px(size * 1.05))
                .text_color(ink(0.78 * first))
                .child("[PRESENTER]"),
        )
        .child(
            div()
                .text_size(px(size * 0.72))
                .text_color(ink(0.6 * then))
                .child("PRESENTS"),
        )
        .into_any_element()
}

/// Where the mark is at `t`: its centre (fractions of the stage) and its
/// height (fraction of the stage's height).
pub fn mark_at(t: f64, sheet: &CueSheet, motion: bool) -> (f64, f64, f64) {
    let (rise, settled) = sheet.rise;
    let p = ((t - rise) / (settled - rise)).clamp(0., 1.);
    let rest = (0.5, 0.44, 0.16);
    if !motion {
        return rest;
    }
    // Large below the bottom edge, rising fast and shrinking as it slows
    // into the centre.
    let y = 1.32 + (rest.1 - 1.32) * ease_out(p);
    let size = 0.62 + (rest.2 - 0.62) * ease_out(p.powf(0.8));
    (rest.0, y, size)
}

/// The mark in its light, and the wordmark under it.
pub fn mark(t: f64, sheet: &CueSheet, tokens: &Tokens, motion: bool) -> AnyElement {
    let tint = Rgba::from(rgb(glow(tokens)));
    let (cx, cy, size) = mark_at(t, sheet, motion);
    let shown = if motion {
        span(t, sheet.rise.0 - 0.1, sheet.rise.0 + 0.2)
    } else {
        span(t, sheet.rise.0, sheet.rise.1)
    };
    // Bright as it rises, calmer once settled, then softening and spreading
    // for the window.
    let settled = span(t, sheet.rise.1 - 0.4, sheet.rise.1 + 0.4);
    let soften = span(t, sheet.spread, sheet.hold);
    let bloom = shown * (1. - 0.55 * settled) * (1. + 0.6 * soften);
    let reach = 0.7 + 1.4 * soften;
    let body = shown * (1. - 0.35 * soften);
    let words = span(t, sheet.words, sheet.words + 0.7) * (1. - 0.6 * soften);
    let figure = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (w, h) = (f64::from(bounds.size.width), f64::from(bounds.size.height));
            // The mark's outline spans about 112 units high.
            let k = h * size / 112.;
            let (ox, oy) = (w * cx, h * cy);
            let outline = |grow: f64| {
                let at = |x: f64, y: f64| {
                    bounds.origin
                        + point(
                            px((ox + x * grow * k) as f32),
                            px((oy + y * grow * k) as f32),
                        )
                };
                let m = &welcome::MARK;
                let mut builder = PathBuilder::fill();
                builder.move_to(at(m[0][0], m[0][1]));
                for i in 0..m.len() {
                    let (a, b) = (m[i], m[(i + 1) % m.len()]);
                    builder.cubic_bezier_to(
                        at(b[0], b[1]),
                        at(a[0] + a[4], a[1] + a[5]),
                        at(b[0] + b[2], b[1] + b[3]),
                    );
                }
                builder.close();
                builder.build()
            };
            // The light: the mark's own shape, larger and fainter outward.
            const STEPS: usize = 16;
            // The outlines are tessellated once per geometry: while the
            // mark holds still (and the wave pictures it) they are reused.
            let key = [
                bounds.origin.x,
                bounds.origin.y,
                bounds.size.width,
                bounds.size.height,
            ]
            .map(|v| f32::from(v).to_bits())
            .into_iter()
            .chain([cx, cy, size, reach].map(|v| (v as f32).to_bits()))
            .collect::<Vec<_>>();
            OUTLINES.with_borrow_mut(|cache| {
                if cache.0 != key {
                    let paths = (0..=STEPS)
                        .map(|step| outline(1. + reach * step as f64 / STEPS as f64).ok())
                        .collect();
                    *cache = (key, paths);
                }
                let paths = &cache.1;
                if bloom > 0.01 {
                    for step in (1..=STEPS).rev() {
                        let r = step as f64 / STEPS as f64;
                        if let Some(path) = paths[step].clone() {
                            let mut c = tint;
                            c.alpha = (bloom * 0.1 * smooth(1. - r)) as f32;
                            window.paint_path(path, c);
                        }
                    }
                }
                if body > 0.
                    && let Some(path) = paths[0].clone()
                {
                    window.paint_path(path, ink(body));
                }
            });
        },
    )
    .absolute()
    .inset_0();
    div()
        .id("stage-mark")
        .debug_selector(|| "stage-mark".into())
        .absolute()
        .inset_0()
        .child(figure)
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(relative((cy + size * 0.62) as f32))
                .flex()
                .justify_center()
                .font_family(TEXT_FONT)
                .font_weight(FontWeight(tokens.typography.display_weight))
                .text_size(px(tokens.typography.display * 1.35))
                .text_color(ink(0.94 * words))
                .child("Powermove"),
        )
        .into_any_element()
}
