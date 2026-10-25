//! Independent f64 interpretation of an authored COLR paint program.
//! Paths use pm-reference's 32-cell coverage; no production COLR pixels or
//! production scan converter enter the expected image.

use kurbo::{Affine, BezPath, Point};
use pm_protocol::color::WorkingEncoding;
use pm_reference::composite::{Blend, Operator, blend, porter_duff};
use pm_reference::coverage::{FillRule, fill};
use pm_text::render::colr::{CompositeMode, Extend, Fill, Op, Program};

fn working_color(color: [f32; 4], working: WorkingEncoding) -> [f64; 4] {
    let mut out = color.map(f64::from);
    if working == WorkingEncoding::LinearLight {
        for channel in &mut out[..3] {
            *channel = pm_reference::color::Transfer::Srgb.to_linear(*channel);
        }
    }
    out
}

fn stops_at(
    stops: &[(f32, [f32; 4])],
    t: f64,
    extend: Extend,
    working: WorkingEncoding,
) -> [f64; 4] {
    let t = match extend {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => t.rem_euclid(1.0),
        Extend::Reflect => {
            let v = t.rem_euclid(2.0);
            if v > 1.0 { 2.0 - v } else { v }
        }
        _ => return [0.0; 4],
    };
    let Some((first, first_color)) = stops.first() else {
        return [0.0; 4];
    };
    if t < f64::from(*first) {
        return working_color(*first_color, working);
    }
    for pair in stops.windows(2) {
        let [(a, ca), (b, cb)] = pair else {
            unreachable!()
        };
        if t < f64::from(*b) {
            let u = (t - f64::from(*a)) / f64::from(*b - *a);
            let ca = working_color(*ca, working);
            let cb = working_color(*cb, working);
            return std::array::from_fn(|i| ca[i] + (cb[i] - ca[i]) * u);
        }
    }
    working_color(stops.last().unwrap().1, working)
}

fn brush_at(fill: &Fill, p: Point, working: WorkingEncoding) -> [f64; 4] {
    match fill {
        Fill::Solid(c) => working_color(*c, working),
        Fill::Linear(p0, p1, stops, extend) => {
            let axis = *p1 - *p0;
            let denominator = axis.hypot2();
            let t = if denominator > 0.0 {
                (p - *p0).dot(axis) / denominator
            } else {
                0.0
            };
            stops_at(stops, t, *extend, working)
        }
        Fill::Radial(c0, r0, c1, r1, stops, extend) => {
            let q = p - *c0;
            let d = *c1 - *c0;
            let dr = f64::from(r1 - r0);
            let a = d.hypot2() - dr * dr;
            let b = -2.0 * (q.dot(d) + f64::from(*r0) * dr);
            let c = q.hypot2() - f64::from(*r0).powi(2);
            let t = if a.abs() < 1e-12 {
                -c / b
            } else {
                let discriminant = b * b - 4.0 * a * c;
                if discriminant < 0.0 {
                    return [0.0; 4];
                }
                [
                    (-b - discriminant.sqrt()) / (2.0 * a),
                    (-b + discriminant.sqrt()) / (2.0 * a),
                ]
                .into_iter()
                .filter(|t| f64::from(*r0) + t * dr >= 0.0)
                .reduce(f64::max)
                .unwrap_or(f64::NAN)
            };
            if t.is_finite() {
                stops_at(stops, t, *extend, working)
            } else {
                [0.0; 4]
            }
        }
        Fill::Sweep(center, start, end, stops, extend) => {
            let q = p - *center;
            let angle = q.y.atan2(q.x).to_degrees().rem_euclid(360.0);
            let t = (angle - f64::from(*start)) / f64::from(end - start);
            stops_at(stops, t, *extend, working)
        }
    }
}

fn mask(path: &BezPath, left: f64, top: f64, w: usize, h: usize, samples: usize) -> Vec<f64> {
    let local = Affine::translate((-left, -top)) * path.clone();
    let coverage = fill(&local, FillRule::NonZero, samples, w, h);
    (0..w * h).map(|i| coverage.at(i % w, i / w)).collect()
}

fn over(dst: &mut [f64], color: [f64; 4], coverage: f64) {
    let alpha = (color[3] * coverage).clamp(0.0, 1.0);
    let src = [color[0] * alpha, color[1] * alpha, color[2] * alpha, alpha];
    let backdrop = [dst[0], dst[1], dst[2], dst[3]];
    dst.copy_from_slice(&porter_duff(Operator::SourceOver, src, backdrop));
}

fn hsl_composite(source: [f64; 4], backdrop: [f64; 4], mode: CompositeMode) -> [f64; 4] {
    let (sa, ba) = (source[3], backdrop[3]);
    let cs: [f64; 3] = std::array::from_fn(|i| if sa > 0.0 { source[i] / sa } else { 0.0 });
    let cb: [f64; 3] = std::array::from_fn(|i| if ba > 0.0 { backdrop[i] / ba } else { 0.0 });
    let lum = |c: [f64; 3]| 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    let sat = |c: [f64; 3]| {
        c.into_iter().fold(f64::NEG_INFINITY, f64::max)
            - c.into_iter().fold(f64::INFINITY, f64::min)
    };
    let set_lum = |mut c: [f64; 3], l: f64| {
        let delta = l - lum(c);
        c = c.map(|v| v + delta);
        let n = c.into_iter().fold(f64::INFINITY, f64::min);
        let x = c.into_iter().fold(f64::NEG_INFINITY, f64::max);
        if n < 0.0 {
            c = c.map(|v| l + (v - l) * l / (l - n));
        }
        if x > 1.0 {
            c = c.map(|v| l + (v - l) * (1.0 - l) / (x - l));
        }
        c
    };
    let set_sat = |c: [f64; 3], value: f64| {
        let min = c.into_iter().fold(f64::INFINITY, f64::min);
        let range = sat(c);
        c.map(|v| {
            if range > 0.0 {
                (v - min) * value / range
            } else {
                0.0
            }
        })
    };
    let mixed = match mode {
        CompositeMode::HslHue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        CompositeMode::HslSaturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        CompositeMode::HslColor => set_lum(cs, lum(cb)),
        CompositeMode::HslLuminosity => set_lum(cb, lum(cs)),
        _ => unreachable!(),
    };
    [
        source[0] * (1.0 - ba) + backdrop[0] * (1.0 - sa) + sa * ba * mixed[0],
        source[1] * (1.0 - ba) + backdrop[1] * (1.0 - sa) + sa * ba * mixed[1],
        source[2] * (1.0 - ba) + backdrop[2] * (1.0 - sa) + sa * ba * mixed[2],
        sa + ba - sa * ba,
    ]
}

fn composite(source: [f64; 4], backdrop: [f64; 4], mode: CompositeMode) -> [f64; 4] {
    use CompositeMode::*;
    let value = match mode {
        Clear => porter_duff(Operator::Clear, source, backdrop),
        Src => porter_duff(Operator::Copy, source, backdrop),
        Dest => porter_duff(Operator::Destination, source, backdrop),
        SrcOver => porter_duff(Operator::SourceOver, source, backdrop),
        DestOver => porter_duff(Operator::DestinationOver, source, backdrop),
        SrcIn => porter_duff(Operator::SourceIn, source, backdrop),
        DestIn => porter_duff(Operator::DestinationIn, source, backdrop),
        SrcOut => porter_duff(Operator::SourceOut, source, backdrop),
        DestOut => porter_duff(Operator::DestinationOut, source, backdrop),
        SrcAtop => porter_duff(Operator::SourceAtop, source, backdrop),
        DestAtop => porter_duff(Operator::DestinationAtop, source, backdrop),
        Xor => porter_duff(Operator::Xor, source, backdrop),
        Plus => porter_duff(Operator::Lighter, source, backdrop),
        Multiply => blend(Blend::Multiply, source, backdrop),
        Screen => blend(Blend::Screen, source, backdrop),
        Overlay => blend(Blend::Overlay, source, backdrop),
        Darken => blend(Blend::Darken, source, backdrop),
        Lighten => blend(Blend::Lighten, source, backdrop),
        ColorDodge => blend(Blend::ColorDodge, source, backdrop),
        ColorBurn => blend(Blend::ColorBurn, source, backdrop),
        HardLight => blend(Blend::HardLight, source, backdrop),
        SoftLight => blend(Blend::SoftLight, source, backdrop),
        Difference => blend(Blend::Difference, source, backdrop),
        Exclusion => blend(Blend::Exclusion, source, backdrop),
        HslHue | HslSaturation | HslColor | HslLuminosity => hsl_composite(source, backdrop, mode),
        Unknown => panic!("unvalidated COLR composite mode"),
    };
    value.map(|v| v.clamp(0.0, 1.0))
}

/// Premultiplied pixels in the requested compositing domain. Each path is
/// independently integrated before its authored clip/layer compositing.
pub fn raster(program: &Program, working: WorkingEncoding, samples: usize) -> Vec<f64> {
    let (w, h) = (program.width as usize, program.height as usize);
    let (left, top) = (program.bounds.x0, program.bounds.y0);
    let mut image = vec![0.0; w * h * 4];
    let mut clips: Vec<Vec<f64>> = Vec::new();
    let mut layers: Vec<(Vec<f64>, CompositeMode)> = Vec::new();
    for op in program.ops() {
        match op {
            Op::Clip(path) => {
                let mut next = path
                    .as_ref()
                    .map_or_else(|| vec![0.0; w * h], |p| mask(p, left, top, w, h, samples));
                if let Some(previous) = clips.last() {
                    for (a, b) in next.iter_mut().zip(previous) {
                        *a *= *b;
                    }
                }
                clips.push(next);
            }
            Op::Unclip => {
                clips.pop().expect("validated COLR clips");
            }
            Op::Fill(brush, inverse) => {
                for (i, pixel) in image.chunks_exact_mut(4).enumerate() {
                    let coverage = clips.last().map_or(1.0, |m| m[i]);
                    if coverage <= 0.0 {
                        continue;
                    }
                    let p = *inverse
                        * Point::new(left + (i % w) as f64 + 0.5, top + (i / w) as f64 + 0.5);
                    over(pixel, brush_at(brush, p, working), coverage);
                }
            }
            Op::SolidGlyph(path, color) => {
                let coverage = mask(path, left, top, w, h, samples);
                let color = working_color(*color, working);
                for (i, pixel) in image.chunks_exact_mut(4).enumerate() {
                    let clip = clips.last().map_or(1.0, |m| m[i]);
                    if coverage[i] * clip > 0.0 {
                        over(pixel, color, coverage[i] * clip);
                    }
                }
            }
            Op::Layer(mode) => {
                layers.push((std::mem::replace(&mut image, vec![0.0; w * h * 4]), *mode))
            }
            Op::Unlayer => {
                let (mut backdrop, mode) = layers.pop().expect("validated COLR layers");
                for (dst, src) in backdrop.chunks_exact_mut(4).zip(image.chunks_exact(4)) {
                    let previous = [dst[0], dst[1], dst[2], dst[3]];
                    dst.copy_from_slice(&composite(src.try_into().unwrap(), previous, mode));
                }
                image = backdrop;
            }
        }
    }
    assert!(
        clips.is_empty() && layers.is_empty(),
        "validated COLR stacks"
    );
    image
}
