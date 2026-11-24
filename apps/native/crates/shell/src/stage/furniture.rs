//! The stage's own drawing: the mark arriving out of light, then folding
//! away into the composer.
use super::beats;
use crate::welcome;
use gpui::{prelude::*, *};
use pm_design::{TEXT_FONT, Tokens};

/// The projector's light on the screen: a warm white, never pure.
const LIGHT: u32 = 0xf2_ee_e6;

fn ink(alpha: f64) -> Rgba {
    let mut c = Rgba::from(rgb(LIGHT));
    c.alpha = alpha.clamp(0., 1.) as f32;
    c
}

/// The mark arriving out of light: its bloom gathers first, the mark
/// resolves out of it and settles, then the wordmark.
/// `fold` (0 to 1) shrinks it into the composer's place.
pub fn mark(local: f64, fold: f64, t: &Tokens, motion: bool) -> AnyElement {
    let accent = t.colors.accent;
    let bloom = beats::span(local, 0., 0.9) * (1. - 0.82 * beats::span(local, 1.0, 2.4));
    let body = beats::span(local, 0.45, 1.35);
    let settle = if motion {
        1.22 - 0.22 * beats::ease_out((local - 0.3) / 1.6)
    } else {
        1.
    };
    // Folding: the mark shrinks to the composer's size as its wordmark
    // goes, then hands over to the composer drawn in its place.
    let settle = settle * (1. - 0.82 * beats::smooth(fold));
    let body = body * (1. - beats::span(fold, 0.6, 1.));
    let bloom = bloom * (1. - beats::smooth(fold));
    let words = beats::span(local, 1.25, 2.05) * (1. - beats::span(fold, 0., 0.4));
    let rise = if motion {
        (1. - beats::ease_out((local - 1.25) / 0.8)) * 10.
    } else {
        0.
    };
    let figure = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (w, h) = (f64::from(bounds.size.width), f64::from(bounds.size.height));
            // The mark's outline spans about 138 units; it stands at a
            // sixth of the display's height.
            let k = (h / 6.) / 112. * settle;
            let (cx, cy) = (w / 2., h / 2. - h * 0.04);
            let at = |x: f64, y: f64| {
                bounds.origin + point(px((cx + x * k) as f32), px((cy + y * k) as f32))
            };
            // `grow` scales the outline about the mark's centre.
            let outline = |grow: f64| {
                let at = |x: f64, y: f64| at(x * grow, y * grow);
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
            // The bloom: the mark's own shape, larger and fainter outward,
            // so the light follows its form instead of radiating spikes.
            const STEPS: usize = 18;
            if bloom > 0.01 {
                let tint = Rgba::from(rgb(pm_design::mix(accent, LIGHT, 0.5)));
                for step in (1..=STEPS).rev() {
                    let reach = step as f64 / STEPS as f64;
                    if let Ok(path) = outline(1. + 0.9 * reach) {
                        let mut c = tint;
                        c.alpha = (bloom * 0.11 * (1. - reach).powf(0.6)) as f32;
                        window.paint_path(path, c);
                    }
                }
            }
            if body > 0.
                && let Ok(path) = outline(1.)
            {
                window.paint_path(path, ink(body));
            }
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
                .top(relative(0.64))
                .mt(px(rise as f32))
                .flex()
                .justify_center()
                .font_family(TEXT_FONT)
                .font_weight(FontWeight(t.typography.display_weight))
                .text_size(px(t.typography.display * 1.5))
                .text_color(ink(0.94 * words))
                .child("Powermove"),
        )
        .into_any_element()
}
