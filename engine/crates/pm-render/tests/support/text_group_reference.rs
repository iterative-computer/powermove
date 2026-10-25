//! Independent f64 composition of text groups, precomps and 2.5D planes.
//!
//! Text scenes supply authored drawing inputs to `text_reference`; this
//! module composes their completed layer images. A group keeps its own
//! transparent accumulator, so its opacity is applied once after its
//! children have blended. A precomp is painted at its own size and a
//! perspective plane's source at its raster size, then read through the
//! draw's placement with the declared Final filter (`sampling_reference`,
//! Lanczos-3 at mip level zero, along the exact Jacobian). Mixed text and
//! other content needs a whole-frame reference and is reported as a gap.

use pm_protocol::color::BlendMode;
use pm_reference::compare::Class;
use pm_reference::composite::{self, Blend};
use pm_render::composite::Node;
use pm_render::plan::{Background, CompPlan, Content, Draw, Source};

use super::sampling_reference::Bitmap;
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

/// Unit-quad coordinates of an output position and their exact
/// derivatives along output x and y.
type UnitMap<'a> = &'a dyn Fn(f64, f64) -> Option<((f64, f64), (f64, f64), (f64, f64))>;

/// Reads `source` (premultiplied f64, `sw × sh`, top-down) into `width ×
/// height` output pixels: `to_unit` maps an output position to the draw's
/// unit-quad coordinates and `outline` is the quad in output pixels. The
/// window `uv` selects the source part the quad shows. As the declared
/// Final filter reads: Lanczos-3 along the exact Jacobian at mip level zero
/// (a minifying placement is a gap), at the window position clamped to the
/// content, times the quad's coverage.
#[allow(clippy::too_many_arguments)]
fn resample(
    source: &TextReference,
    (sw, sh): (u32, u32),
    uv: [f64; 4],
    to_unit: UnitMap<'_>,
    outline: &kurbo::BezPath,
    width: u32,
    height: u32,
) -> Result<TextReference, String> {
    let (w, h) = (width as usize, height as usize);
    let bitmap = Bitmap {
        width: sw as usize,
        height: sh as usize,
        rgba: source.rgba.clone(),
    };

    let coverage = pm_reference::coverage::fill(
        outline,
        pm_reference::coverage::FillRule::NonZero,
        super::reference::PATH_SAMPLES,
        w,
        h,
    );
    let mut out = reference(width, height);
    let mut edge = vec![false; w * h];
    let mut sampled = vec![false; w * h];
    let mut unknown = vec![false; w * h];
    for y in coverage.y0..coverage.y0 + coverage.height {
        for x in coverage.x0..coverage.x0 + coverage.width {
            let k = coverage.at(x, y);
            if k <= 0.0 {
                continue;
            }
            let Some(((s, t), ds, dt)) = to_unit(x as f64 + 0.5, y as f64 + 0.5) else {
                return Err("degenerate text placement".into());
            };
            let (tw, th) = (f64::from(sw), f64::from(sh));
            let at = (
                (uv[0] + uv[2] * s).clamp(0.0, 1.0) * tw,
                (uv[1] + uv[3] * t).clamp(0.0, 1.0) * th,
            );
            let dx = (ds.0 * uv[2] * tw, ds.1 * uv[3] * th);
            let dy = (dt.0 * uv[2] * tw, dt.1 * uv[3] * th);
            if !Bitmap::mip_zero(dx, dy) {
                return Err("minified text source".into());
            }
            let i = y * w + x;
            let value = bitmap.sample(at, dx, dy);
            if value.iter().any(|v| v.is_nan()) {
                unknown[i] = true;
                continue;
            }
            out.rgba[i * 4..][..4].copy_from_slice(&value.map(|v| v * k));
            sampled[i] = true;
            edge[i] = k < 1.0 - 1e-12;
        }
    }
    out.classes = pm_reference::compare::classify(&edge, &sampled, w, h, 2);
    for (i, unknown) in unknown.into_iter().enumerate() {
        if unknown {
            out.classes[i] = Class::Unreferenced;
            out.rgba[i * 4..][..4].fill(f64::NAN);
        }
    }
    refresh_counts(&mut out);
    Ok(out)
}

/// An inner source image at its own size, with its size, UV window and
/// opacity.
type SourceImage = (TextReference, (u32, u32), [f64; 4], f64);

/// A draw's inner source image at its own size: a text scene or a
/// precomp. `None` when it holds no text.
fn source_image(content: &Content) -> Result<Option<SourceImage>, String> {
    let Content::Texture { source, uv, alpha } = content else {
        return Ok(None);
    };
    Ok(match source {
        Source::Text(plan) => {
            // The quad shows the plan's crop of its scene.
            let full = text_reference::render(&plan.scene);
            let [x0, y0, x1, y1] = plan.crop.map(|v| v as usize);
            let (cw, ch) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
            let stride = plan.scene.width as usize;
            let mut image = reference(cw as u32, ch as u32);
            for y in 0..ch {
                for x in 0..cw {
                    let (from, to) = ((y0 + y) * stride + x0 + x, y * cw + x);
                    image.rgba[to * 4..][..4].copy_from_slice(&full.rgba[from * 4..][..4]);
                    image.classes[to] = full.classes[from];
                }
            }
            refresh_counts(&mut image);
            Some((image, (cw as u32, ch as u32), *uv, *alpha))
        }
        Source::Comp(comp) => render_comp(comp)?
            .map(|image| (image, (comp.width, comp.height), *uv, *alpha)),
        _ => None,
    })
}

fn full_pixel_quad(draw: &Draw, width: u32, height: u32, uv: [f64; 4]) -> bool {
    let full = [width as f64, 0.0, 0.0, height as f64, 0.0, 0.0];
    draw.matrix
        .iter()
        .zip(full)
        .all(|(a, b)| (*a - b).abs() <= 1e-9)
        && uv == [0.0, 0.0, 1.0, 1.0]
}

/// A precomp or group's text image at its own size.
fn render_comp(comp: &CompPlan) -> Result<Option<TextReference>, String> {
    let result = render_draws(&comp.draws, comp.width, comp.height)?;
    if result.is_some() && !matches!(&comp.background, Background::Transparent) {
        return Err("text precomp has nontransparent background".into());
    }
    Ok(result)
}

/// Compose a plain text draw into a premultiplied Standard-domain source.
/// `None` means the draw contains no text; unsupported text composition is an
/// explicit error so a gate cannot silently compare it as ordinary content.
pub fn render_draw(draw: &Draw, width: u32, height: u32) -> Result<Option<TextReference>, String> {
    match &draw.content {
        Content::Solid(_) => Ok(None),
        // A 2D raster drawn through the shared sampler (every raster layer
        // carries one): only a full-pixel quad reads its texels one to one,
        // which `Source::Comp` checks below.
        Content::Placed(placed) if placed.plane.is_none() => render_draw(
            &Draw {
                content: placed.content.clone(),
                ..draw.clone()
            },
            width,
            height,
        ),
        Content::Placed(placed) => {
            let Some(plane) = placed.plane else {
                unreachable!("handled above")
            };
            let Some((image, size, uv, alpha)) = source_image(&placed.content)? else {
                return Ok(None);
            };
            // The homography maps the unit quad to output pixels
            // (column-major).
            let inverse = invert3(plane).ok_or("singular text plane")?;
            let to_unit = |x: f64, y: f64| {
                let p = [
                    inverse[0] * x + inverse[3] * y + inverse[6],
                    inverse[1] * x + inverse[4] * y + inverse[7],
                    inverse[2] * x + inverse[5] * y + inverse[8],
                ];
                if p[2] <= 1e-12 {
                    return None;
                }
                let q = (p[0] / p[2], p[1] / p[2]);
                // ∂q/∂x = (inv₀ − q·inv₀.z) / w, likewise for y.
                let along = |c: usize| {
                    (
                        (inverse[c] - q.0 * inverse[c + 2]) / p[2],
                        (inverse[c + 1] - q.1 * inverse[c + 2]) / p[2],
                    )
                };
                Some((q, along(0), along(3)))
            };
            let corner = |s: f64, t: f64| {
                let w = plane[2] * s + plane[5] * t + plane[8];
                kurbo::Point::new(
                    (plane[0] * s + plane[3] * t + plane[6]) / w,
                    (plane[1] * s + plane[4] * t + plane[7]) / w,
                )
            };
            let mut outline = kurbo::BezPath::new();
            outline.move_to(corner(0.0, 0.0));
            outline.line_to(corner(1.0, 0.0));
            outline.line_to(corner(1.0, 1.0));
            outline.line_to(corner(0.0, 1.0));
            outline.close_path();
            let mut result = resample(&image, size, uv, &to_unit, &outline, width, height)?;
            opacity(&mut result, alpha)?;
            Ok(Some(result))
        }
        Content::Texture { source, uv, alpha } => {
            let mut result = match source {
                Source::Text(plan) => {
                    if (plan.scene.width, plan.scene.height) != (width, height) {
                        return Err("text scene was not in output device pixels".into());
                    }
                    Some(text_reference::render(&plan.scene))
                }
                Source::Comp(comp) => match render_comp(comp)? {
                    Some(image)
                        if (comp.width, comp.height) == (width, height)
                            && full_pixel_quad(draw, width, height, *uv) =>
                    {
                        Some(image)
                    }
                    Some(image) => {
                        let m = draw.matrix;
                        let det = m[0] * m[3] - m[1] * m[2];
                        if det.abs() < 1e-12 || !det.is_finite() {
                            return Err("singular text precomp placement".into());
                        }
                        let to_unit = |x: f64, y: f64| {
                            let (px, py) = (x - m[4], y - m[5]);
                            Some((
                                ((m[3] * px - m[2] * py) / det, (-m[1] * px + m[0] * py) / det),
                                (m[3] / det, -m[1] / det),
                                (-m[2] / det, m[0] / det),
                            ))
                        };
                        let quad = kurbo::Shape::to_path(&kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), 1e-9);
                        let outline = kurbo::Affine::new(m) * quad;
                        Some(resample(
                            &image,
                            (comp.width, comp.height),
                            *uv,
                            &to_unit,
                            &outline,
                            width,
                            height,
                        )?)
                    }
                    None => None,
                },
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

fn invert3(m: [f64; 9]) -> Option<[f64; 9]> {
    let det = m[0] * (m[4] * m[8] - m[7] * m[5]) - m[3] * (m[1] * m[8] - m[7] * m[2])
        + m[6] * (m[1] * m[5] - m[4] * m[2]);
    if det.abs() < 1e-18 || !det.is_finite() {
        return None;
    }
    let inv = [
        m[4] * m[8] - m[7] * m[5],
        m[7] * m[2] - m[1] * m[8],
        m[1] * m[5] - m[4] * m[2],
        m[6] * m[5] - m[3] * m[8],
        m[0] * m[8] - m[6] * m[2],
        m[3] * m[2] - m[0] * m[5],
        m[3] * m[7] - m[6] * m[4],
        m[6] * m[1] - m[0] * m[7],
        m[0] * m[4] - m[3] * m[1],
    ];
    Some(inv.map(|v| v / det))
}
