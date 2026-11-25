//! The intro's picture at a time, from the cue sheet: where the mark is and
//! how it is lit (`light::Light`), the credit, and the wordmark. Geometry
//! follows Dia's measured disc on its curves, normalised to the mark: it
//! starts 2.5 times its resting size (Dia's ×5.96) with its top at the
//! bottom edge, and comes to rest at the centre of the screen.
use super::cue::{CueSheet, smooth};
use super::light::Light;
use gpui::{prelude::*, *};
use pm_design::{TEXT_FONT, Tokens};

/// The mark's resting height, a fraction of the screen's height (Dia's
/// disc is 0.275; the mark's three arms carry less light than a disc).
pub const REST_HEIGHT: f64 = 0.235;
/// Its resting centre (Dia's disc rests at 0.4997).
pub const REST_Y: f64 = 0.5;
/// At launch the mark is 2.5 times its resting size (Dia's disc is ×5.96:
/// at that size the mark's stem reads as a spike, not as light) with its apex
/// at the bottom edge; before it, only its glow crests the edge, like a
/// horizon, as it creeps up from below (Dia's creep curve).
const LAUNCH_SCALE: f64 = 2.5;
/// While white-hot the outline is this soft, in mark units, so the bloom
/// carries the silhouette and the thin apex never shows as a white spike.
const HOT_SOFTNESS: f64 = 15.;
const LAUNCH_TOP: f64 = 1.0;
const CREEP_TOP: f64 = 1.09;
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
    let launch_unit = rest.unit * LAUNCH_SCALE;
    let launch_y = LAUNCH_TOP - TOP * launch_unit;
    let crept = launch_y + (CREEP_TOP - LAUNCH_TOP) * (1. - s.creep.at(t));
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
            creep * (0.75 + 0.75 * flare - 0.35 * settled),
            1.0 * creep * (1. - s.fall.at(t)) + 0.9 * flare,
            creep * (0.3 + 0.7 * s.rise.at(t)),
        )
    } else {
        // Still: the mark fades in where it rests as the credit goes.
        let up = smooth((t - s.credit.2.at) / 1.2);
        (1., 0.4 * up, 0., up)
    };
    // The light leads the shape while it is white-hot: a soft outline that
    // resolves as the exposure comes down.
    let soft = if motion {
        HOT_SOFTNESS * m.unit * h * (1. - settled)
    } else {
        0.
    };
    // The room's light: Dia's room light going down, then down hard to
    // near-black as the mark rises, so the mark is the only light (Dia's
    // room drops to deep navy).
    let atmosphere = if motion {
        (1. - 0.57 * s.dim.at(t)) * (1. - 0.93 * smooth(1.6 * s.rise.at(t)))
    } else {
        // Still: the room goes down as the mark fades in.
        0.3 * (1. - 0.85 * smooth((t - s.credit.2.at) / 1.2))
    };
    Light {
        size: [size.0, size.1],
        centre: [(m.x * f64::from(size.0)) as f32, (m.y * h) as f32],
        scale: (m.unit * h) as f32,
        exposure: exposure as f32,
        bloom: bloom as f32,
        edge: edge as f32,
        pool: pool as f32,
        // The body comes up with its glow as the creep begins: before it,
        // the swollen white-hot outline would show at the edge unlit.
        mark: if motion {
            smooth(creep / 0.12) as f32
        } else {
            smooth((t - s.credit.2.at) / 1.2) as f32
        },
        // Reduce Motion: nothing drifts.
        time: if motion { t as f32 } else { 0. },
        density,
        reach: (REST_HEIGHT * h) as f32,
        atmosphere: atmosphere as f32,
        ground: [0.; 3],
        soft: soft as f32,
        iris: if motion {
            smooth((t - 5.0) / 1.4) as f32
        } else {
            1.
        },
        field: 1.,
        domain: [0.; 4],
    }
}

fn ink(alpha: f64) -> Rgba {
    let mut c = Rgba::from(rgb(INK));
    c.alpha = alpha.clamp(0., 1.) as f32;
    c
}

/// The credit: Iterative Computer's own wordmark, "presents" under it in
/// Fraunces Italic (the wordmark's family), a step quieter; where Dia sets
/// its own. Both are drawn from their outlines (`assets/credit`) at the
/// stage's device resolution.
pub fn credit(t: f64, s: &CueSheet, area: Size<Pixels>, scale: f32) -> AnyElement {
    let out = 1. - s.credit.2.at(t);
    let first = s.credit.0.at(t) * out;
    let then = s.credit.1.at(t) * out;
    if first <= 0. {
        return div().into_any_element();
    }
    let h = f32::from(area.height);
    // Points per unit of the outlines (both 2000 to the em): the wordmark's
    // capitals (1400 units) stand 0.0185 of the height, centred where Dia's
    // first line is (0.466); "presents" at 0.62 of its size, its baseline
    // 1.7 cap heights under the wordmark's.
    let cap = h * 0.0185;
    let unit = cap / 1400.;
    let baseline = h * 0.466 + cap / 2.;
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
    /// The light pass on the GPU, read back: a dim warm room around a lit
    /// mark, with and without motion (a uniform layout mismatch paints it a
    /// flat red).
    #[test]
    fn the_light_pass_draws_a_warm_room_and_a_lit_mark() {
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
            // The room is its dim warm field, never the flat red of a
            // uniform layout mismatch (green near zero).
            assert!(
                top.iter().all(|v| *v < 170),
                "{motion}: the room is dim: {top:?}"
            );
            assert!(
                u32::from(top[1]) * 5 > u32::from(top[0]),
                "{motion}: not flat red: {top:?}"
            );
            assert!(
                centre.iter().all(|v| *v > 170),
                "{motion}: the mark is lit: {centre:?}"
            );
        }
    }
}
