//! Independent f64 composition of plain text groups and precomps.
//!
//! Text scenes supply authored drawing inputs to `text_reference`; this
//! module composes their completed layer images. A group keeps its own
//! transparent accumulator, so its opacity is applied once after its
//! children have blended. Mixed text and other content needs a whole-frame
//! reference and is reported as a gap.

use pm_protocol::color::BlendMode;
use pm_reference::compare::Class;
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
    }
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

fn over(dst: &mut TextReference, src: &TextReference, mode: Blend) {
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

fn render_draws(draws: &[Draw], width: u32, height: u32) -> Result<Option<TextReference>, String> {
    let mut result = reference(width, height);
    let (mut has_text, mut has_other) = (false, false);
    for draw in draws {
        match render_draw(draw, width, height)? {
            Some(source) => {
                if has_other {
                    return Err("text and nontext share a group or precomp".into());
                }
                has_text = true;
                over(&mut result, &source, blend_mode(draw));
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

fn render_comp(comp: &CompPlan, width: u32, height: u32) -> Result<Option<TextReference>, String> {
    let result = render_draws(&comp.draws, comp.width, comp.height)?;
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
pub fn render_draw(draw: &Draw, width: u32, height: u32) -> Result<Option<TextReference>, String> {
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
                    let result = render_comp(comp, width, height)?;
                    if result.is_some() && !full_pixel_quad(draw, width, height, *uv) {
                        return Err("placed or resampled text precomp".into());
                    }
                    result
                }
                Source::Accumulate(comp) => {
                    if comp.draws.iter().any(|d| contains_text(&d.content)) {
                        return Err("motion-blurred text group".into());
                    }
                    None
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
                let mut result = render_draws(&layer.content, width, height)?;
                if let Some(image) = &mut result {
                    if layer.effects.is_some()
                        || layer.matte.is_some()
                        || layer.transition.is_some()
                    {
                        return Err("text layer effects, matte, or transition".into());
                    }
                    if let Some(masks) = &layer.masks {
                        super::text_mask_reference::apply(
                            masks,
                            image,
                            width as usize,
                            height as usize,
                        )?;
                    }
                    opacity(image, layer.alpha)?;
                }
                Ok(result)
            }
            Node::Adjustment(_) => Ok(None),
        },
    }
}
