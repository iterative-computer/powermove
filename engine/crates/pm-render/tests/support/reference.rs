//! Reference frames of documents for the quality gates
//! (docs/engine/gate-policy.md).
//!
//! Authored values (keyframes, world matrices, anchors, opacity, blend
//! names, colors, fills) are evaluated by `pm-eval`, whose own gates keep
//! them exact. Coverage, compositing and color math come from
//! `pm-reference`, never from the engine's renderer.
//!
//! Modelled so far: the composition background (solid, linear and radial
//! gradients, none) and 2D solid layers (world matrix, size anchor,
//! opacity, blend mode, level 2 radius) in projects that composite in E.
//! Image layers leave their box unreferenced (NaN); any other content
//! leaves the whole frame unreferenced. The comparator judges unreferenced
//! pixels against today's capture until the lane that owns that content
//! models it here (gate-policy.md, "Who converts what").

use pm_eval::node;
use pm_eval::{Comp, Evaluator, Scope};
use pm_model::{Document, Node};
use pm_protocol::color::{BlendMode, Precision, WorkingEncoding};
use pm_reference::compare::{Class, classify};
use pm_reference::composite::{Blend, Rgba, blend, scale};
use pm_reference::coverage::{FillRule, fill};

/// Coverage cells per pixel side for paths and solids.
pub const PATH_SAMPLES: usize = 16;

/// A reference frame.
pub struct Reference {
    pub width: usize,
    pub height: usize,
    /// Premultiplied RGBA in the working encoding; NaN where not modelled.
    pub rgba: Vec<f64>,
    /// Pixels where modelled content is partially covered.
    pub edge: Vec<bool>,
    /// Why the frame is not modelled (empty when it is).
    pub unmodelled: Vec<String>,
}

impl Reference {
    /// Pixel classes: edges grown by 2 pixels; interior elsewhere.
    pub fn classes(&self) -> Vec<Class> {
        let none = vec![false; self.edge.len()];
        classify(&self.edge, &none, self.width, self.height, 2)
    }

    /// The capture's opaque view: color over black clamped to 0…1, alpha 1.
    pub fn opaque(&self) -> Vec<f64> {
        self.rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = if p[3].is_nan() { f64::NAN } else { 1.0 };
                [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0), p[2].clamp(0.0, 1.0), a]
            })
            .collect()
    }

    /// The share of pixels the reference models.
    pub fn modelled(&self) -> f64 {
        let n = self.rgba.chunks_exact(4).filter(|p| !p[3].is_nan()).count();
        n as f64 / (self.width * self.height).max(1) as f64
    }

    pub fn alpha(&self) -> Vec<f64> {
        self.rgba.chunks_exact(4).map(|p| p[3]).collect()
    }
}

/// `#RRGGBB` or `#RRGGBBAA` as straight E values; `None` for anything the
/// reference does not read.
fn hex(s: &str) -> Option<[f64; 4]> {
    let d = s.strip_prefix('#')?;
    if !(d.len() == 6 || d.len() == 8) || !d.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = |i: usize| f64::from(u8::from_str_radix(&d[i..i + 2], 16).unwrap()) / 255.0;
    Some([c(0), c(2), c(4), if d.len() == 8 { c(6) } else { 1.0 }])
}

/// The background fill (`FRAG_BACKGROUND_FILL`, the authored formula): `t`
/// along the angle (or the radius × 1.4142) from the pixel center with y up,
/// colors mixed stop by stop with `smoothstep` (a hard step where two stops
/// coincide), in E.
fn gradient(fill: &pm_render::plan::Fill, x: usize, y: usize, w: usize, h: usize) -> Option<[f64; 3]> {
    let stops: Vec<([f64; 4], f64)> = fill
        .stops
        .iter()
        .map(|(c, pos)| hex(c).map(|c| (c, pos / 100.0)))
        .collect::<Option<_>>()?;
    let p = [
        (x as f64 + 0.5) / w as f64 - 0.5,
        (h as f64 - (y as f64 + 0.5)) / h as f64 - 0.5,
    ];
    // 1.4142 is the formula's literal, not √2.
    #[allow(clippy::approx_constant)]
    let t = if fill.kind == "radial" {
        (p[0].hypot(p[1]) * 1.4142).clamp(0.0, 1.0)
    } else {
        let a = fill.angle.to_radians();
        (p[0] * a.cos() + p[1] * a.sin() + 0.5).clamp(0.0, 1.0)
    };
    let mut color = [stops[0].0[0], stops[0].0[1], stops[0].0[2]];
    for pair in stops.windows(2) {
        let ((_, e0), (right, e1)) = (pair[0], pair[1]);
        let amount = if e1 <= e0 {
            if t >= e0 { 1.0 } else { 0.0 }
        } else {
            let k = ((t - e0) / (e1 - e0)).clamp(0.0, 1.0);
            k * k * (3.0 - 2.0 * k)
        };
        for i in 0..3 {
            color[i] += (right[i] - color[i]) * amount;
        }
    }
    Some(color)
}

fn reference_blend(mode: BlendMode) -> Blend {
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

/// The reference frame of `composition` at `time`, `width × height`
/// output pixels at the legacy derived scale, with or without the
/// background (transparent captures draw none).
pub fn render(
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    width: u32,
    height: u32,
    background: bool,
) -> Reference {
    let (w, h) = (width as usize, height as usize);
    let mut out = Reference {
        width: w,
        height: h,
        rgba: vec![0.0; w * h * 4],
        edge: vec![false; w * h],
        unmodelled: Vec::new(),
    };
    if let Err(why) = draw(document, composition, time, &mut out, background) {
        out.unmodelled.push(why);
        out.rgba.fill(f64::NAN);
    }
    out
}

fn draw(
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    out: &mut Reference,
    background: bool,
) -> Result<(), String> {
    let (w, h) = (out.width, out.height);
    let root = document.root().ok_or("no document")?;
    let color = pm_render::composite::color::project_color(root)?;
    if color.working() != WorkingEncoding::Standard {
        return Err("linear-light compositing".into());
    }
    // 8-bit storage holds 0…1 only.
    let clamp = |c: Rgba| -> Rgba {
        if color.precision == Precision::Unorm8 {
            c.map(|v| v.clamp(0.0, 1.0))
        } else {
            c
        }
    };
    let comp = Comp::resolve(document, composition).map_err(|e| format!("{e:?}"))?;
    let seconds = comp.seconds_at(time.get());
    if background {
        let fill = pm_render::plan::normalize_fill(comp.member("backgroundFill"), comp.member("bg"));
        for y in 0..h {
            for x in 0..w {
                let c: Rgba = match fill.kind.as_str() {
                    "none" => [0.0; 4],
                    "solid" => {
                        let c = hex(&fill.stops[0].0).ok_or("background color")?;
                        [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
                    }
                    _ => {
                        let c = gradient(&fill, x, y, w, h).ok_or("background stops")?;
                        [c[0], c[1], c[2], 1.0]
                    }
                };
                out.rgba[(y * w + x) * 4..][..4].copy_from_slice(&clamp(c));
            }
        }
    }
    let (sx, sy) = pm_render::plan::derived_scale(&comp, w as u32, h as u32);
    let mut ev = Evaluator::new();
    let mut scope = Scope::new(&mut ev, &comp, &comp);
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let layers: Vec<&Node> = comp.layers().iter().collect();
    let solo = layers.iter().any(|l| node::truthy(node::member(Some(l), "solo")));
    // Bottom (last) to top (first), with today's skip rules.
    for &layer in layers.iter().rev() {
        let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
        let flag = |k: &str| node::truthy(node::member(Some(layer), k));
        if flag("group") || ty == "group" || ty == "precomp" {
            return Err("groups and precomps".into());
        }
        if (solo && !flag("solo")) || matches!(ty.as_str(), "null" | "audio") {
            continue;
        }
        if !scope.active(layer, seconds).map_err(fail)? {
            continue;
        }
        let opacity = scope.num(layer, "opacity", seconds).map_err(fail)? / 100.0;
        let opacity = if opacity.is_nan() { opacity } else { opacity.clamp(0.0, 1.0) };
        if opacity <= 0.001 {
            continue;
        }
        if !matches!(ty.as_str(), "solid" | "image") {
            return Err(format!("{ty} layers"));
        }
        if flag("matteSource")
            || flag("mblur")
            || flag("threeD")
            || pm_render::composite::build::has_masks(&mut scope, layer, seconds).map_err(fail)?
            || scope.has_renderable_effects(layer, seconds).map_err(fail)?
            || scope.active_transition(layer, seconds, &|_| true).map_err(fail)?.is_some()
            || scope.is_3d_layer(layer).map_err(fail)?
        {
            return Err("mattes, motion blur, 3D, masks, effects or transitions".into());
        }
        let d = scope.ev.resolve_content(scope.comp, layer, seconds).map_err(fail)?;
        let sw = pm_render::js::or_number(pm_render::js::get(&d, "w"), w as f64);
        let sh = pm_render::js::or_number(pm_render::js::get(&d, "h"), h as f64);
        let world = scope.world_matrix(layer, seconds).map_err(fail)?;
        let offset = scope.size_anchor_offset(layer, seconds).map_err(fail)?;
        let device = kurbo::Affine::new([
            world[0] * sx,
            world[1] * sy,
            world[2] * sx,
            world[3] * sy,
            world[4] * sx,
            world[5] * sy,
        ]) * kurbo::Affine::translate((offset.x, offset.y));
        if ty == "image" {
            // Resampled content no lane models yet: its box (centered on
            // the anchor), grown by a pixel, is unreferenced.
            let rect = kurbo::Rect::new(-sw / 2.0, -sh / 2.0, sw / 2.0, sh / 2.0);
            let box_ = fill(&(device * kurbo::Shape::to_path(&rect, 1e-9)), FillRule::NonZero, 1, w, h);
            let mut inside = vec![false; w * h];
            for y in box_.y0..box_.y0 + box_.height {
                for x in box_.x0..box_.x0 + box_.width {
                    inside[y * w + x] = box_.at(x, y) > 0.0;
                }
            }
            let grown = pm_reference::compare::dilate(&inside, w, h, 1);
            for (i, _) in grown.iter().enumerate().filter(|(_, g)| **g) {
                out.rgba[i * 4..][..4].fill(f64::NAN);
            }
            continue;
        }
        let paint = hex(&pm_render::js::string(pm_render::js::get(&d, "color"))).ok_or("solid color")?;
        // Level 2 honors `radius` (at most half of either side); level 1
        // ignores it, as shipped (decision SOLID-RADIUS-1 is open).
        let radius = pm_render::js::get(&d, "radius").to_number();
        let radius = if pm_render::vector::level2(layer) && radius > 0.0 {
            radius.min(sw.abs() / 2.0).min(sh.abs() / 2.0)
        } else {
            0.0
        };
        let outline = kurbo::Shape::to_path(&kurbo::RoundedRect::new(0.0, 0.0, sw, sh, radius), 1e-9);
        let coverage = fill(&(device * outline), FillRule::NonZero, PATH_SAMPLES, w, h);
        let mode = reference_blend(
            pm_render::composite::build::blend_mode(&mut scope, layer, seconds).map_err(fail)?,
        );
        let a = paint[3] * opacity;
        let src: Rgba = [paint[0] * a, paint[1] * a, paint[2] * a, a];
        for y in coverage.y0..coverage.y0 + coverage.height {
            for x in coverage.x0..coverage.x0 + coverage.width {
                let k = coverage.at(x, y);
                if k <= 0.0 {
                    continue;
                }
                if k < 1.0 - 1e-12 {
                    out.edge[y * w + x] = true;
                }
                let px = &mut out.rgba[(y * w + x) * 4..][..4];
                let dst = [px[0], px[1], px[2], px[3]];
                px.copy_from_slice(&clamp(blend(mode, scale(src, k), dst)));
            }
        }
    }
    Ok(())
}

/// Preview == export (gate-policy.md): the frame the paused viewer asks for
/// and the frame an export asks for, from contract 2 requests, with the
/// same motion-blur samples (sample counts per output kind are a setting,
/// not a pipeline difference).
pub fn preview_and_export(
    renderer: &mut pm_render::Renderer,
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    size: pm_protocol::common::Size,
    motion_blur: pm_render::motion::MotionBlur,
    resolver: &pm_render::MapResolver,
) -> (Vec<f32>, Vec<f32>) {
    use pm_protocol::color::{OutputColor, ViewerColorSpace};
    use pm_protocol::frame::{
        FramePurpose, FrameRequest, MediaPreference, MotionBlurKind, PreviewResolution,
    };
    use pm_protocol::priority::PriorityClass;
    let export = FrameRequest {
        composition: pm_protocol::common::CompositionId::new(composition),
        time,
        purpose: FramePurpose::Export,
        priority: PriorityClass::UserJob,
        output_size: size,
        region: None,
        scale: 1.0,
        resolution: PreviewResolution::Full,
        media: MediaPreference::Originals,
        color: OutputColor::SrgbImage,
        precision: Precision::Float16,
        motion_blur: MotionBlurKind::Export,
        view: None,
        draft_3d: false,
        draft_render: false,
        depth_id: false,
        overlay: false,
    };
    let preview = FrameRequest {
        purpose: FramePurpose::Viewer,
        priority: PriorityClass::Interactive,
        color: OutputColor::Viewer {
            space: ViewerColorSpace::Srgb,
        },
        motion_blur: MotionBlurKind::PreviewPaused,
        ..export.clone()
    };
    let mut frame = |request: &FrameRequest| {
        let mut r = pm_render::RenderRequest::from_frame_request(request).unwrap();
        // The legacy scale of the composition, as the corpus renders it.
        r.scale = None;
        r.motion_blur = motion_blur;
        let f = renderer.render(document, composition, time, &r, resolver).unwrap();
        renderer.read_rgba(&f).unwrap()
    };
    (frame(&preview), frame(&export))
}
