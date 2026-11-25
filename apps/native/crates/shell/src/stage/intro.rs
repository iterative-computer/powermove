//! The intro's picture at a time, from the cue sheet: where the mark is and
//! how it is lit (`light::Light`), the credit, and the wordmark. Geometry is
//! Dia's measured disc normalised to the mark: it starts 5.96 times its
//! resting size with its top at the bottom edge, and comes to rest at the
//! centre of the screen.
use super::cue::{CueSheet, smooth};
use super::light::Light;
use gpui::{prelude::*, *};
use pm_design::{TEXT_FONT, Tokens};

/// The mark's resting height, a fraction of the screen's height (Dia's
/// disc is 0.275; the mark's three arms carry less light than a disc).
pub const REST_HEIGHT: f64 = 0.235;
/// Its resting centre (Dia's disc rests at 0.4997).
pub const REST_Y: f64 = 0.5;
/// Dia's disc at launch: ×5.96 its resting size, top at 0.961 of the height,
/// having crept up from the edge (1.0).
const LAUNCH_SCALE: f64 = 5.96;
const LAUNCH_TOP: f64 = 0.961;
/// The outline's top (the tip's curve, above its anchors at −54.9) and
/// bottom, in mark units from its origin.
const TOP: f64 = -58.0;
const BOTTOM: f64 = 57.2;
/// The notch under the stem, where the wordmark is uncovered first.
const NOTCH: f64 = 42.;
/// The warm white of the stage's type.
pub const INK: u32 = 0xede6dc;

/// The mark at `t`: its origin (fractions of the stage) and its scale
/// (fraction of the stage's height per mark unit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkAt {
    pub x: f64,
    pub y: f64,
    pub unit: f64,
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
}
pub fn rest() -> MarkAt {
    MarkAt {
        x: 0.5,
        y: REST_Y,
        unit: REST_HEIGHT / 112.,
    }
}
pub fn mark_at(t: f64, s: &CueSheet, motion: bool) -> MarkAt {
    let rest = rest();
    if !motion {
        return rest;
    }
    let size = LAUNCH_SCALE + (1. - LAUNCH_SCALE) * s.shrink.at(t);
    let unit = rest.unit * size;
    // At launch the top sits where Dia's did; before it, it crept up from
    // the edge.
    let launch_unit = rest.unit * LAUNCH_SCALE;
    let launch_y = LAUNCH_TOP - TOP * launch_unit;
    let crept = launch_y + (1. - LAUNCH_TOP) * (1. - s.creep.at(t));
    let y = crept + (rest.y - launch_y) * s.rise.at(t);
    MarkAt { x: 0.5, y, unit }
}

/// The light on the stage at `t`, for a stage `size` device pixels with
/// `density` pixels per point.
pub fn light_at(t: f64, s: &CueSheet, motion: bool, size: (f32, f32), density: f32) -> Light {
    let m = mark_at(t, s, motion);
    let h = f64::from(size.1);
    let creep = s.creep.at(t);
    let flare = s.flare.at(t) * (1. - s.fall.at(t));
    let settled = s.exposure.at(t);
    let (exposure, bloom, edge, pool) = if motion {
        (
            1. + 39. * (1. - settled),
            creep * (0.55 + 0.75 * flare - 0.15 * settled),
            0.55 * creep * (1. - s.fall.at(t)) + 0.9 * flare,
            creep * (0.3 + 0.7 * s.rise.at(t)),
        )
    } else {
        // Still: the mark fades in where it rests as the credit goes.
        let up = smooth((t - s.credit.2.at) / 1.2);
        (1., 0.4 * up, 0., up)
    };
    Light {
        size: [size.0, size.1],
        centre: [(m.x * f64::from(size.0)) as f32, (m.y * h) as f32],
        scale: (m.unit * h) as f32,
        exposure: exposure as f32,
        bloom: bloom as f32,
        edge: edge as f32,
        pool: pool as f32,
        mark: if motion {
            1.
        } else {
            smooth((t - s.credit.2.at) / 1.2) as f32
        },
        time: t as f32,
        density,
        reach: (REST_HEIGHT * h) as f32,
        // Dia's room keeps 43 % of its light once the house lights are down.
        atmosphere: if motion {
            (1. - 0.57 * s.dim.at(t)) as f32
        } else {
            0.43
        },
        ground: [0.; 3],
        pad: [0.; 3],
        domain: [0.; 4],
    }
}

fn ink(alpha: f64) -> Rgba {
    let mut c = Rgba::from(rgb(INK));
    c.alpha = alpha.clamp(0., 1.) as f32;
    c
}

/// The credit: "ITERATIVE COMPUTER" in spaced capitals, "presents" under
/// it in a light italic, a step quieter; where Dia sets its own. (System
/// families only: a family GPUI cannot resolve stops the app.)
pub fn credit(t: f64, s: &CueSheet, area: Size<Pixels>) -> AnyElement {
    let out = 1. - s.credit.2.at(t);
    let first = s.credit.0.at(t) * out;
    let then = s.credit.1.at(t) * out;
    if first <= 0. {
        return div().into_any_element();
    }
    let h = f32::from(area.height);
    // Cap heights: Dia's line 0.0175 of the height; ours a touch smaller
    // and tracked wide. SF's capitals are 0.705 of the size.
    let size = h * 0.0165 / 0.705;
    let line = |top: f32, child: Div| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(top))
            .flex()
            .justify_center()
            .child(child)
    };
    div()
        .id("stage-credit")
        .debug_selector(|| "stage-credit".into())
        .absolute()
        .inset_0()
        .child(line(
            h * 0.466 - size * 0.62,
            div()
                .font_family(TEXT_FONT)
                .font_weight(FontWeight(430.))
                .text_size(px(size))
                .letter_spacing(px(size * 0.26))
                .text_color(ink(0.9 * first))
                .child("ITERATIVE COMPUTER"),
        ))
        .child(line(
            h * 0.507 - size * 0.6,
            div()
                .font_family(TEXT_FONT)
                .italic()
                .font_weight(FontWeight::LIGHT)
                .text_size(px(size * 1.08))
                .letter_spacing(px(size * 0.04))
                .text_color(ink(0.6 * then))
                .child("presents"),
        ))
        .into_any_element()
}

/// The wordmark under the mark: uncovered as the mark passes it, then
/// riding up into place with it (Dia's 4.5 % of the height).
pub fn words(
    t: f64,
    s: &CueSheet,
    area: Size<Pixels>,
    tokens: &Tokens,
    motion: bool,
) -> AnyElement {
    let h = f32::from(area.height);
    let rest = rest();
    let size = h * 0.030 / 0.705;
    let centre = rest.bottom() + 0.05;
    let (y, shown) = if motion {
        let rise = 0.046 * (1. - s.words.at(t));
        let mark = mark_at(t, s, true);
        let top = centre + rise - 0.015;
        // Shown as the mark's base clears its top.
        let clear = ((top - (mark.y + NOTCH * mark.unit)) / 0.03).clamp(0., 1.);
        (centre + rise, smooth(clear))
    } else {
        (centre, smooth((t - s.credit.2.at - 0.3) / 1.2))
    };
    if shown <= 0. {
        return div().into_any_element();
    }
    div()
        .id("stage-words")
        .debug_selector(|| "stage-words".into())
        .absolute()
        .left_0()
        .right_0()
        .top(px(y as f32 * h - size * 0.62))
        .flex()
        .justify_center()
        .font_family(TEXT_FONT)
        .font_weight(FontWeight(tokens.typography.display_weight))
        .text_size(px(size))
        .letter_spacing(px(-size * 0.012))
        .text_color(ink(0.95 * shown))
        .child("Powermove")
        .into_any_element()
}
#[cfg(test)]
mod tests {
    /// The light pass on the GPU, read back: a dark room around a lit mark,
    /// with and without motion (a uniform layout mismatch paints it red).
    #[test]
    fn the_light_pass_draws_a_dark_room_and_a_lit_mark() {
        let s = super::CueSheet::default();
        let device = metal::Device::system_default().unwrap();
        let mut r = super::super::light::LightRenderer::new(&device).unwrap();
        for motion in [true, false] {
            let l = super::light_at(6.5, &s, motion, (302., 196.), 0.2);
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
                [px[i + 2], px[i + 1], px[i]]
            };
            let (top, centre) = (at(151, 5), at(151, 92));
            assert!(
                top.iter().all(|v| *v < 90),
                "{motion}: the room is dark: {top:?}"
            );
            assert!(
                top[0].abs_diff(top[2]) < 30,
                "{motion}: and not tinted: {top:?}"
            );
            assert!(
                centre.iter().all(|v| *v > 170),
                "{motion}: the mark is lit: {centre:?}"
            );
        }
    }
}
