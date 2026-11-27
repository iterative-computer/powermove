//! Independent f64 layer-mask formulas over the text reference.

use kurbo::Affine;
use pm_reference::compare::{Class, dilate};
use pm_reference::coverage::{FillRule, fill};
use pm_reference::gaussian::{Edge, blur};
use pm_render::composite::Masks;
use pm_vector::{MaskMode, MaskShape};

use super::text_reference::TextReference;

/// Apply authored parametric masks or exact path coverage and Gaussian feather.
/// No engine mask pixels or sampling code enter the reference.
pub fn apply(
    masks: &Masks,
    text: &mut TextReference,
    width: usize,
    height: usize,
) -> Result<(), String> {
    let mut alpha = vec![0.0; width * height];
    if let Some(path) = &masks.path {
        if (path.width as usize, path.height as usize) != (width, height)
            || masks.inverse != [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        {
            return Err("projected path masks".into());
        }
        alpha.fill(if path.masks.start_covered { 1.0 } else { 0.0 });
        let world = Affine::new(path.world);
        for mask in &path.masks.masks {
            let sigma = mask.feather * path.world[0].hypot(path.world[1]) / 2.0;
            let pad = (8.0 * sigma).ceil() as usize;
            let (w, h) = (width + 2 * pad, height + 2 * pad);
            let outline =
                Affine::translate((pad as f64, pad as f64)) * world * mask.transform * &mask.path;
            let covered = fill(&outline, FillRule::NonZero, 32, w, h);
            let mut pixels = vec![0.0; w * h];
            for y in covered.y0..covered.y0 + covered.height {
                for x in covered.x0..covered.x0 + covered.width {
                    pixels[y * w + x] = covered.at(x, y);
                }
            }
            if sigma > 0.0 {
                pixels = blur(&pixels, w, h, 1, (sigma, sigma), Edge::Transparent);
            }
            for y in 0..height {
                for x in 0..width {
                    let a = pixels[(y + pad) * w + x + pad];
                    let d = &mut alpha[y * width + x];
                    *d = match mask.mode {
                        MaskMode::Add => a + *d * (1.0 - a),
                        MaskMode::Subtract => *d * (1.0 - a),
                    };
                }
            }
        }
    } else {
        let inv = masks.inverse;
        for y in 0..height {
            for x in 0..width {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let z = inv[2] * px + inv[5] * py + inv[8];
                if !z.is_finite() || z == 0.0 {
                    return Err("singular mask projection".into());
                }
                let local = (
                    (inv[0] * px + inv[3] * py + inv[6]) / z,
                    (inv[1] * px + inv[4] * py + inv[7]) / z,
                );
                let mut a: f64 = if masks.masks.has_add { 0.0 } else { 1.0 };
                for mask in masks.masks.masks.iter().take(8) {
                    if mask.size.0 < 0.01 || mask.size.1 < 0.01 {
                        continue;
                    }
                    let (mut dx, mut dy) = (local.0 - mask.center.x, local.1 - mask.center.y);
                    if mask.rotation.abs() > 1e-4 {
                        let (s, c) = mask.rotation.sin_cos();
                        (dx, dy) = (c * dx + s * dy, -s * dx + c * dy);
                    }
                    let half = (
                        (mask.size.0 * 0.5).max(0.001),
                        (mask.size.1 * 0.5).max(0.001),
                    );
                    let distance = match mask.shape {
                        MaskShape::Rect => {
                            let d = (dx.abs() - half.0, dy.abs() - half.1);
                            d.0.max(0.0).hypot(d.1.max(0.0)) + d.0.max(d.1).min(0.0)
                        }
                        MaskShape::Ellipse => {
                            ((dx / half.0).hypot(dy / half.1) - 1.0) * half.0.min(half.1)
                        }
                    };
                    let covered = if mask.feather > 0.01 {
                        let t = ((distance + mask.feather * 0.5) / mask.feather).clamp(0.0, 1.0);
                        1.0 - t * t * (3.0 - 2.0 * t)
                    } else {
                        if distance <= 0.0 { 1.0 } else { 0.0 }
                    };
                    a = match mask.mode {
                        MaskMode::Add => a.max(covered),
                        MaskMode::Subtract => a.min(1.0 - covered),
                    };
                }
                alpha[y * width + x] = a;
            }
        }
    }
    let edge: Vec<bool> = alpha
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            (a > 0.0 && a < 1.0)
                || (i % width > 0 && a != alpha[i - 1])
                || (i >= width && a != alpha[i - width])
        })
        .collect();
    let edge = dilate(&edge, width, height, 2);
    for (i, &mask) in alpha.iter().enumerate() {
        if mask == 0.0 {
            text.rgba[i * 4..i * 4 + 4].fill(0.0);
            text.classes[i] = if edge[i] {
                Class::Edge
            } else {
                Class::Interior
            };
        } else {
            for c in &mut text.rgba[i * 4..i * 4 + 4] {
                *c *= mask;
            }
            if edge[i] && text.classes[i] == Class::Interior {
                text.classes[i] = Class::Edge;
            }
        }
    }
    text.unreferenced = text
        .classes
        .iter()
        .filter(|&&c| c == Class::Unreferenced)
        .count();
    Ok(())
}
