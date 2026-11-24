//! The stage's furniture, drawn over the engine's pictures: the slate, the
//! mark arriving out of light, and each reel's screen with the instrument
//! under it (the clip strip, the agent's composer, the grade's readout, the
//! camera's). Values come from `beats`, as the compositions' keys do.
use super::{beats, cue::Reel};
use crate::kit::tabular;
use crate::welcome;
use gpui::{prelude::*, *};
use pm_design::{TEXT_FONT, Tokens};
use std::sync::Arc;

/// The projector's light on the screen: a warm white, never pure.
const LIGHT: u32 = 0xf2_ee_e6;

fn ink(alpha: f64) -> Rgba {
    let mut c = Rgba::from(rgb(LIGHT));
    c.alpha = alpha.clamp(0., 1.) as f32;
    c
}

/// The title card: the slate of the first take. Crop marks of a 2.39:1
/// frame draw out from the centre, its fields fade in, and the timecode
/// runs from zero.
pub fn slate(local: f64, t: &Tokens, motion: bool) -> AnyElement {
    let marks = beats::span(local, 0., 0.9);
    let fields = beats::span(local, 0.35, 1.15);
    let reach = if motion {
        0.82 + 0.18 * beats::ease_out(local / 1.1)
    } else {
        1.
    };
    let frames = (local.max(0.) * 24.) as u64;
    let timecode = format!(
        "{:02}:{:02}:{:02}:{:02}",
        frames / (24 * 3600),
        frames / (24 * 60) % 60,
        frames / 24 % 60,
        frames % 24
    );
    let corners = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let fw = w * 0.7 * reach as f32;
            let fh = fw / 2.39;
            let (cx, cy) = (w / 2., h / 2.);
            let arm = (w * 0.018).max(18.);
            let stroke = 1.5;
            let colour = ink(0.55 * marks);
            for (sx, sy) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                let x = cx + sx * fw / 2.;
                let y = cy + sy * fh / 2.;
                let at = |dx: f32, dy: f32| bounds.origin + point(px(x + dx), px(y + dy));
                let mut path = PathBuilder::stroke(px(stroke));
                path.move_to(at(-sx * arm, 0.));
                path.line_to(at(0., 0.));
                path.line_to(at(0., -sy * arm));
                if let Ok(path) = path.build() {
                    window.paint_path(path, colour);
                }
            }
        },
    )
    .absolute()
    .inset_0();
    let label = |text: &'static str| {
        div()
            .text_size(px(t.typography.title))
            .text_color(ink(0.42 * fields))
            .child(text)
    };
    let value = |text: &'static str| {
        div()
            .text_size(px(t.typography.title * 1.25))
            .text_color(ink(0.8 * fields))
            .child(text)
    };
    let field = |l: &'static str, v: &'static str| {
        div()
            .flex()
            .flex_col()
            .gap(px(t.space.xs))
            .child(label(l))
            .child(value(v))
    };
    // OWNER-COPY: the slate's fields.
    let fields_row = div()
        .flex()
        .gap(px(t.space.xxl * 2.))
        .font_features(tabular())
        .child(field("PRODUCTION", "POWERMOVE"))
        .child(field("SCENE", "1"))
        .child(field("TAKE", "1"))
        .child(field("ROLL", "A001"));
    div()
        .id("stage-slate")
        .debug_selector(|| "stage-slate".into())
        .absolute()
        .inset_0()
        .child(corners)
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(t.space.xxl))
                .child(
                    div()
                        .font_features(tabular())
                        .text_size(px(t.typography.display * 2.4))
                        .text_color(ink(0.92 * fields))
                        .child(timecode),
                )
                .child(fields_row),
        )
        .into_any_element()
}

/// The mark arriving out of light: its bloom gathers first, the mark
/// resolves out of it and settles, then the wordmark.
pub fn mark(local: f64, t: &Tokens, motion: bool) -> AnyElement {
    let accent = t.colors.accent;
    let bloom = beats::span(local, 0., 0.9) * (1. - 0.82 * beats::span(local, 1.0, 2.4));
    let body = beats::span(local, 0.45, 1.35);
    let settle = if motion {
        1.22 - 0.22 * beats::ease_out((local - 0.3) / 1.6)
    } else {
        1.
    };
    let words = beats::span(local, 1.25, 2.05);
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

/// A reel on its screen, with the caption and the reel's instrument under it.
pub fn reel(
    reel: Reel,
    local: f64,
    screen: Bounds<Pixels>,
    picture: Option<Arc<NativeTexture>>,
    t: &Tokens,
    window: &mut Window,
) -> AnyElement {
    let enter = beats::span(local, 0., 0.5);
    let radius = px(t.radius.sm);
    let rim = ink(0.08);
    let picture_layer = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let mask = ContentMask {
                bounds,
                shape: bounds,
                corner_radii: Corners::all(radius),
                ..ContentMask::default()
            };
            window.with_content_mask(Some(mask), |window| {
                window.paint_quad(PaintQuad::new(bounds).background(rgb(0x050506)));
                if let Some(picture) = picture {
                    window.paint_texture(bounds, picture);
                }
            });
            window.paint_quad(
                PaintQuad::new(bounds)
                    .corner_radii(Corners::all(radius))
                    .border_widths(px(1.))
                    .border_color(rim),
            );
        },
    )
    .absolute()
    .left(screen.origin.x)
    .top(screen.origin.y)
    .w(screen.size.width)
    .h(screen.size.height);
    let under = screen.origin.y + screen.size.height + px(t.space.xl);
    let caption = div()
        .absolute()
        .left(screen.origin.x)
        .top(under)
        .w(screen.size.width)
        .flex()
        .items_start()
        .justify_between()
        .gap(px(t.space.xl))
        .opacity(enter as f32)
        .child(
            div()
                .font_family(TEXT_FONT)
                .text_size(px(t.typography.title * 1.3))
                .text_color(ink(0.9))
                .child(beats::caption(reel)),
        )
        .child(instrument(reel, local, screen.size.width, t, window));
    div()
        .id(("stage-reel", reel as usize))
        .debug_selector(move || format!("stage-reel-{reel:?}"))
        .absolute()
        .inset_0()
        .child(picture_layer)
        .child(caption)
        .into_any_element()
}

fn instrument(
    reel: Reel,
    local: f64,
    width: Pixels,
    t: &Tokens,
    window: &mut Window,
) -> AnyElement {
    match reel {
        Reel::Cut => strip(local, width * 0.5, t).into_any_element(),
        Reel::Ask => composer(local, t, window).into_any_element(),
        Reel::Grade => {
            let [temperature, saturation, contrast, _] = beats::GRADE;
            readout(
                &[
                    ("Temperature", temperature),
                    ("Saturation", saturation),
                    ("Contrast", contrast),
                ],
                t,
            )
        }
        Reel::Depth => readout(&[("Orbit", beats::orbit(local))], t),
    }
}

/// The Cut reel's clips end to end, the one under the playhead lit, the
/// playhead crossing them.
fn strip(local: f64, width: Pixels, t: &Tokens) -> impl IntoElement {
    let (active, _) = beats::shot(local);
    let at = (local / beats::REEL_SECONDS).clamp(0., 1.) as f32;
    let radius = t.radius.xs;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let gap = px(3.);
            let n = beats::SHOTS as f32;
            let clip_w = (bounds.size.width - gap * (n - 1.)) / n;
            for i in 0..beats::SHOTS {
                let x = bounds.origin.x + (clip_w + gap) * i as f32;
                let b = Bounds::new(point(x, bounds.origin.y), size(clip_w, bounds.size.height));
                let lit = i == active;
                window.paint_quad(
                    PaintQuad::new(b)
                        .corner_radii(Corners::all(px(radius)))
                        .background(ink(if lit { 0.22 } else { 0.08 }))
                        .border_widths(px(1.))
                        .border_color(ink(if lit { 0.45 } else { 0.14 })),
                );
            }
            let x = bounds.origin.x + bounds.size.width * at;
            window.paint_quad(
                PaintQuad::new(Bounds::new(
                    point(x - px(1.), bounds.origin.y - px(4.)),
                    size(px(2.), bounds.size.height + px(8.)),
                ))
                .background(ink(0.95)),
            );
        },
    )
    .flex_none()
    .w(width)
    .h(px(t.metrics.property_row))
}

/// The agent's composer: the request typed, then what the agent did.
fn composer(local: f64, t: &Tokens, window: &mut Window) -> impl IntoElement {
    let typed: String = beats::ASK_PROMPT
        .chars()
        .take(beats::typed(local))
        .collect();
    let caret = local < beats::ASK_BUILDING && (local * 2.).fract() < 0.6;
    let status = if local >= beats::ASK_BUILT {
        Some(("checkmark.circle.fill", "Glow added"))
    } else if local >= beats::ASK_BUILDING {
        Some(("sparkles", "Adding Glow…"))
    } else {
        None
    };
    let mut field = div()
        .flex()
        .items_center()
        .gap(px(t.space.sm))
        .px(px(t.space.lg))
        .h(px(t.metrics.form_row))
        .rounded(px(t.metrics.form_row / 2.))
        .bg(ink(0.07))
        .border_1()
        .border_color(ink(0.12))
        .text_size(px(t.typography.body))
        .text_color(ink(0.88))
        .child(typed);
    if caret {
        field = field.child(div().w(px(1.5)).h(px(t.typography.body * 1.2)).bg(ink(0.9)));
    }
    let mut row = div()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(t.space.sm))
        .child(field);
    if let Some((symbol, text)) = status {
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap(px(t.space.xs))
                .text_size(px(t.typography.body))
                .text_color(ink(0.6))
                .child(crate::symbol::symbol(
                    symbol,
                    t.typography.body,
                    crate::symbol::Weight::Medium,
                    pm_design::mix(LIGHT, 0, 0.35),
                    window,
                ))
                .child(text),
        );
    }
    row
}

/// Values and their names, the figures tabular so they don't jitter.
fn readout(values: &[(&'static str, f64)], t: &Tokens) -> AnyElement {
    let mut row = div().flex().gap(px(t.space.xl));
    for (name, value) in values {
        row = row.child(
            div()
                .flex()
                .flex_col()
                .items_end()
                .gap(px(t.space.xs))
                .child(
                    div()
                        .text_size(px(t.typography.body))
                        .text_color(ink(0.45))
                        .child(*name),
                )
                .child(
                    div()
                        .font_features(tabular())
                        .text_size(px(t.typography.title))
                        .text_color(ink(0.88))
                        .child(format!("{value:+.0}")),
                ),
        );
    }
    row.into_any_element()
}
