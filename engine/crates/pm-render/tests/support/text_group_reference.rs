//! Independent f64 composition of plain text groups and precomps.
//!
//! Text scenes supply authored drawing inputs to `text_reference`; this
//! module composes their completed layer images. A group keeps its own
//! transparent accumulator, so its opacity is applied once after its
//! children have blended. Mixed text and other content needs a whole-frame
//! reference and is reported as a gap. A text layer's built-in effects run
//! through `pm_reference::effects` after its masks (the plan's order: masks,
//! effects, matte, composite); its active transition is the caller's
//! (`LayerFacts::transition`), which holds the stacks it reads.

use pm_protocol::color::{BlendMode, Precision};
use pm_reference::compare::Class;
use pm_reference::effects::Open;
use pm_reference::composite::{self, Blend};
use pm_render::composite::Node;
use pm_render::plan::{Background, CompPlan, Content, Draw, Source};

use super::text_reference::{self, TextReference};

fn reference(width: u32, height: u32) -> TextReference {
    let pixels = width as usize * height as usize;
    TextReference {
        rgba: vec![0.0; pixels * 4],
        classes: vec![Class::Interior; pixels],
        image_pixels: 0,
        unreferenced: 0,
        open: vec![None; pixels],
    }
}

/// What a plan's layer node does not carry: its built-in effect chain and
/// its active transition, evaluated from the document as
/// `support::reference` evaluates them.
pub struct LayerFacts<'a> {
    pub document: &'a pm_model::Document,
    pub composition: &'a str,
    pub time: pm_protocol::common::Ticks,
    pub effects: &'a pm_render::effects::Effects,
}

pub use super::reference::Transition;

impl LayerFacts<'_> {
    fn layer<T>(
        &self,
        id: &str,
        f: impl for<'s, 'd> FnOnce(&mut pm_eval::Scope<'s, 'd>, &'d pm_model::Node, f64) -> Result<T, String>,
    ) -> Result<T, String> {
        let comp = pm_eval::Comp::resolve(self.document, self.composition)
            .map_err(|e| format!("{e:?}"))?;
        let seconds = comp.seconds_at(self.time.get());
        let mut evaluator = pm_eval::Evaluator::new()
            .with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
        let mut scope = pm_eval::Scope::new(&mut evaluator, &comp, &comp);
        let layer = comp
            .layers()
            .iter()
            .find(|l| pm_render::js::layer_id(l) == id)
            .ok_or("text layer effects or transition outside the composition's own layers")?;
        f(&mut scope, layer, seconds)
    }

    /// The layer's built-in chain, `None` when a step is not one the
    /// reference models.
    fn chain(&self, id: &str) -> Result<Option<super::reference::Chain>, String> {
        self.layer(id, |scope, layer, seconds| {
            super::reference::effect_chain(scope, self.effects, layer, seconds)
        })
    }

    /// The layer's active built-in transition.
    pub fn transition(&self, id: &str) -> Result<Option<Transition>, String> {
        self.layer(id, |scope, layer, seconds| {
            super::reference::layer_transition(scope, self.effects, layer, seconds)?
                .transpose()
                .map_err(|why| format!("text layer {why} the reference does not model"))
        })
    }

    pub fn seconds(&self) -> Result<f64, String> {
        let comp = pm_eval::Comp::resolve(self.document, self.composition)
            .map_err(|e| format!("{e:?}"))?;
        Ok(comp.seconds_at(self.time.get()))
    }

    pub fn precision(&self) -> Result<Precision, String> {
        let root = self.document.root().ok_or("no document")?;
        Ok(pm_render::composite::color::project_color(root)?.precision)
    }
}

/// The layer's chain over its frame-sized image (`support::reference::
/// apply_effects`). Resampling effects class every pixel they reach as
/// resampled; a value the formula leaves open is resampled too.
fn apply_chain(
    image: &mut TextReference,
    chain: &super::reference::Chain,
    width: u32,
    height: u32,
    facts: &LayerFacts<'_>,
) -> Result<(), String> {
    if image.unreferenced > 0 {
        return Err("unreferenced text under effects".into());
    }
    let (w, h) = (width as usize, height as usize);
    let input: Vec<([f64; 4], bool)> = image
        .rgba
        .chunks_exact(4)
        .map(|p| ([p[0], p[1], p[2], p[3]], false))
        .collect();
    let (out, open, resampled) = super::reference::apply_effects(
        chain,
        &input,
        w,
        h,
        facts.seconds()?,
        facts.precision()?,
    )
    .ok_or("text layer effect leaves values open that a later effect transforms")?;
    for (i, before) in input.iter().enumerate() {
        let after = out.at(i % w, i / w);
        let reached = after.iter().chain(&before.0).any(|v| *v != 0.0);
        if open[i].is_some() || (resampled && reached) {
            image.classes[i] = Class::Resampled;
        }
        image.rgba[i * 4..][..4].copy_from_slice(&after);
    }
    image.open = open;
    refresh_counts(image);
    Ok(())
}

fn blend(mode: BlendMode) -> Blend {
    match mode {
        BlendMode::Normal => Blend::Normal,
        BlendMode::Add => Blend::LinearDodge,
        BlendMode::Screen => Blend::Screen,
        BlendMode::Multiply => Blend::Multiply,
        BlendMode::Overlay => Blend::Overlay,
        BlendMode::SoftLight => Blend::SoftLightPhotoshop,
        BlendMode::Difference => Blend::Difference,
        BlendMode::Lighten => Blend::Lighten,
        BlendMode::Darken => Blend::Darken,
    }
}

/// The blend to use when placing this draw over an existing frame. Group
/// children are already blended in `render_draw`'s returned source image.
pub fn blend_mode(draw: &Draw) -> Blend {
    match &draw.content {
        Content::Composite(node) => match node.as_ref() {
            Node::Layer(layer) => blend(layer.blend),
            Node::Adjustment(adjustment) => blend(adjustment.blend),
        },
        _ => Blend::Normal,
    }
}

fn combined_class(a: Class, b: Class) -> Class {
    if a == Class::Unreferenced || b == Class::Unreferenced {
        Class::Unreferenced
    } else if a == Class::Resampled || b == Class::Resampled {
        Class::Resampled
    } else if a == Class::Edge || b == Class::Edge {
        Class::Edge
    } else {
        Class::Interior
    }
}

fn refresh_counts(image: &mut TextReference) {
    image.image_pixels = image
        .classes
        .iter()
        .filter(|&&class| class == Class::Resampled)
        .count();
    image.unreferenced = image
        .classes
        .iter()
        .filter(|&&class| class == Class::Unreferenced)
        .count();
}

fn over(dst: &mut TextReference, src: &TextReference, mode: Blend) -> Result<(), String> {
    if src.open.iter().chain(&dst.open).any(Option::is_some) {
        return Err("open effect values inside a text group".into());
    }
    for (i, (out, source)) in dst
        .rgba
        .chunks_exact_mut(4)
        .zip(src.rgba.chunks_exact(4))
        .enumerate()
    {
        let class = combined_class(dst.classes[i], src.classes[i]);
        dst.classes[i] = class;
        if class == Class::Unreferenced {
            out.fill(f64::NAN);
            continue;
        }
        let result = composite::blend(
            mode,
            [source[0], source[1], source[2], source[3]],
            [out[0], out[1], out[2], out[3]],
        );
        out.copy_from_slice(&result);
    }
    refresh_counts(dst);
    Ok(())
}

/// What one store into the working format may lose at `v`: half an 8-bit
/// code (rounded), or a binary16 or binary32 step (truncated).
fn step(v: f64, precision: Precision) -> f64 {
    let (bits, min) = match precision {
        Precision::Unorm8 => return 0.5 / 255.0,
        Precision::Float32 => (23, -126),
        _ => (10, -14),
    };
    let exponent = if v == 0.0 { min } else { (v.abs().log2().floor() as i32).max(min) };
    2f64.powi(exponent - bits)
}

/// Additive blending (`ONE, ONE`): the motion-blur accumulation. `stored`
/// sums, per channel, the format steps its stores may have lost.
fn add(
    dst: &mut TextReference,
    src: &TextReference,
    stored: &mut [f64],
    precision: Precision,
) -> Result<(), String> {
    if src.open.iter().any(Option::is_some) {
        return Err("open effect values in a motion-blurred text sample".into());
    }
    for (i, (out, source)) in dst
        .rgba
        .chunks_exact_mut(4)
        .zip(src.rgba.chunks_exact(4))
        .enumerate()
    {
        dst.classes[i] = combined_class(dst.classes[i], src.classes[i]);
        for (k, (o, s)) in out.iter_mut().zip(source).enumerate() {
            if *s != 0.0 {
                *o += s;
                stored[i * 4 + k] += step(*o, precision);
            }
        }
    }
    refresh_counts(dst);
    Ok(())
}

fn opacity(image: &mut TextReference, alpha: f64) -> Result<(), String> {
    if !alpha.is_finite() {
        return Err("non-finite text group opacity".into());
    }
    for pixel in image.rgba.chunks_exact_mut(4) {
        for channel in pixel {
            *channel *= alpha;
        }
    }
    let scale = |c: [f64; 4]| c.map(|v| v * alpha);
    for open in image.open.iter_mut().flatten() {
        *open = match open {
            Open::Points(points) => Open::Points(points.iter().map(|c| scale(*c)).collect()),
            Open::Range(lo, hi) => Open::Range(scale(*lo), scale(*hi)),
        };
    }
    Ok(())
}

fn contains_text(content: &Content) -> bool {
    match content {
        Content::Solid(_) => false,
        Content::Placed(placed) => contains_text(&placed.content),
        Content::Texture { source, .. } => match source {
            Source::Text(_) => true,
            Source::Comp(comp) | Source::Accumulate(comp) => {
                comp.draws.iter().any(|draw| contains_text(&draw.content))
            }
            _ => false,
        },
        Content::Composite(node) => match node.as_ref() {
            Node::Layer(layer) => layer
                .content
                .iter()
                .any(|draw| contains_text(&draw.content)),
            Node::Adjustment(_) => false,
        },
    }
}

fn render_draws(
    draws: &[Draw],
    width: u32,
    height: u32,
    facts: &LayerFacts<'_>,
) -> Result<Option<TextReference>, String> {
    let mut result = reference(width, height);
    let (mut has_text, mut has_other) = (false, false);
    for draw in draws {
        match draw_at(draw, width, height, facts, false)? {
            Some(source) => {
                if has_other {
                    return Err("text and nontext share a group or precomp".into());
                }
                has_text = true;
                over(&mut result, &source, blend_mode(draw))?;
            }
            None => {
                if has_text {
                    return Err("text and nontext share a group or precomp".into());
                }
                has_other = true;
            }
        }
    }
    Ok(has_text.then_some(result))
}

fn full_pixel_quad(draw: &Draw, width: u32, height: u32, uv: [f64; 4]) -> bool {
    let full = [width as f64, 0.0, 0.0, height as f64, 0.0, 0.0];
    draw.matrix
        .iter()
        .zip(full)
        .all(|(a, b)| (*a - b).abs() <= 1e-9)
        && uv == [0.0, 0.0, 1.0, 1.0]
}

fn render_comp(
    comp: &CompPlan,
    width: u32,
    height: u32,
    facts: &LayerFacts<'_>,
) -> Result<Option<TextReference>, String> {
    let result = render_draws(&comp.draws, comp.width, comp.height, facts)?;
    if result.is_some() {
        if (comp.width, comp.height) != (width, height) {
            return Err("resampled text precomp".into());
        }
        if !matches!(&comp.background, Background::Transparent) {
            return Err("text precomp has nontransparent background".into());
        }
    }
    Ok(result)
}

/// Compose a plain text draw into a premultiplied Standard-domain source.
/// `None` means the draw contains no text; unsupported text composition is an
/// explicit error so a gate cannot silently compare it as ordinary content.
/// A top-level text layer's transition is left to the caller
/// ([`LayerFacts::transition`]).
pub fn render_draw(
    draw: &Draw,
    width: u32,
    height: u32,
    facts: &LayerFacts<'_>,
) -> Result<Option<TextReference>, String> {
    draw_at(draw, width, height, facts, true)
}

fn draw_at(
    draw: &Draw,
    width: u32,
    height: u32,
    facts: &LayerFacts<'_>,
    top: bool,
) -> Result<Option<TextReference>, String> {
    match &draw.content {
        Content::Solid(_) => Ok(None),
        Content::Placed(placed) => {
            if contains_text(&placed.content) {
                Err("projected or resampled text".into())
            } else {
                Ok(None)
            }
        }
        Content::Texture { source, uv, alpha } => {
            let mut result = match source {
                Source::Text(plan) => {
                    if (plan.scene.width, plan.scene.height) != (width, height) {
                        return Err("text scene was not in output device pixels".into());
                    }
                    Some(text_reference::render(&plan.scene))
                }
                Source::Comp(comp) => {
                    let result = render_comp(comp, width, height, facts)?;
                    if result.is_some() && !full_pixel_quad(draw, width, height, *uv) {
                        return Err("placed or resampled text precomp".into());
                    }
                    result
                }
                // Motion blur (`crate::motion`): the samples, each at its
                // share of the opacity, summed with additive blending into a
                // transparent target in this pixel space.
                Source::Accumulate(comp) => {
                    if !comp.draws.iter().any(|d| contains_text(&d.content)) {
                        None
                    } else if (comp.width, comp.height) != (width, height)
                        || !full_pixel_quad(draw, width, height, *uv)
                    {
                        return Err("resampled motion-blurred text".into());
                    } else {
                        let mut sum = reference(width, height);
                        let mut stored = vec![0.0; sum.rgba.len()];
                        // The accumulation target is the frame's working
                        // format (`pm_gpu::target::working_format`).
                        let precision = facts.precision()?;
                        for sample in &comp.draws {
                            let image = draw_at(sample, width, height, facts, false)?
                                .ok_or("text and nontext share a motion-blurred group")?;
                            add(&mut sum, &image, &mut stored, precision)?;
                        }
                        // Each additive store may land up to a step of the
                        // target's format off (Metal's render-target
                        // conversion truncates): the sum is open by the steps
                        // its stores took.
                        for (i, open) in sum.open.iter_mut().enumerate() {
                            let e = &stored[i * 4..][..4];
                            if e.iter().any(|v| *v > 0.0) {
                                let v = &sum.rgba[i * 4..][..4];
                                *open = Some(Open::Range(
                                    std::array::from_fn(|k| v[k] - e[k]),
                                    std::array::from_fn(|k| v[k] + e[k]),
                                ));
                            }
                        }
                        Some(sum)
                    }
                }
                _ => None,
            };
            if let Some(image) = &mut result {
                opacity(image, *alpha)?;
            }
            Ok(result)
        }
        Content::Composite(node) => match node.as_ref() {
            Node::Layer(layer) => {
                let mut result = render_draws(&layer.content, width, height, facts)?;
                if let Some(image) = &mut result {
                    if layer.matte.is_some() {
                        return Err("text layer matte".into());
                    }
                    if layer.transition.is_some() && !top {
                        return Err("text layer transition inside a group or precomp".into());
                    }
                    if let Some(masks) = &layer.masks {
                        super::text_mask_reference::apply(
                            masks,
                            image,
                            width as usize,
                            height as usize,
                        )?;
                    }
                    if layer.effects.is_some() {
                        let chain = facts
                            .chain(&layer.layer)?
                            .ok_or("text layer effects the reference does not model")?;
                        apply_chain(image, &chain, width, height, facts)?;
                    }
                    opacity(image, layer.alpha)?;
                }
                Ok(result)
            }
            Node::Adjustment(_) => Ok(None),
        },
    }
}
