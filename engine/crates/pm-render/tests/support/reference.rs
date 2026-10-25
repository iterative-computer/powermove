//! Reference frames of documents for the quality gates
//! (docs/engine/gate-policy.md).
//!
//! Authored values (keyframes, world matrices, anchors, opacity, blend
//! names, colors, fills) are evaluated by `pm-eval`, whose own gates keep
//! them exact. Coverage, compositing and color math come from
//! `pm-reference`, never from the engine's renderer. The reading of the
//! document is shared with the engine (`plan::derived_scale`,
//! `normalize_fill`, `project_color`, `blend_mode`, `has_masks`,
//! `vector::level2`): a bug there would be in both, which the interior
//! check against today's capture still catches.
//!
//! Modelled so far, in projects that composite in E: the composition
//! background (solid, linear and radial gradients, none); 2D solids
//! (radius included) and shape layers (shapes and editable paths: fills and
//! strokes); SVG images (independent f64 paths, gradients, groups and clips
//! on a 16× grid; unsupported constructs use separate resvg 32× samples,
//! whose RGBA8 paint quantization remains a reference limitation); affine
//! raster stills at mip level zero; motion blur of solids, shapes and SVG
//! images; parametric and path masks (`mask_reference`); and the authored
//! group, precomp and track-matte compositing graph (opacity and blend
//! applied once to each finished boundary image). Each child source is
//! painted from its own layers into f64 pixels before the parent samples it.
//! Other content leaves its footprint unreferenced (NaN), or the whole frame
//! when that footprint cannot be bounded. The comparator judges unreferenced
//! pixels against today's capture until the lane that owns that content
//! models it here (gate-policy.md, "Who converts what").

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pm_eval::node;
use pm_eval::{Comp, Evaluator, Scope};
use pm_model::{Document, Node};
use pm_protocol::color::{BlendMode, Precision, WorkingEncoding};
use pm_reference::compare::{Class, classify};
use pm_reference::composite::{Blend, Rgba, blend, scale};
use pm_reference::coverage::{FillRule, fill};
use pm_render::AssetResolver;
use pm_vector::kurbo::{Affine, Point};

use super::sampling_reference::Bitmap;

#[path = "svg_f64.rs"]
mod svg_f64;

pub const PATH_SAMPLES: usize = 16;

/// A proof variant with only one open-composition layer drawing. The other
/// layers remain as null transform parents, so authored world transforms do
/// not change. Effects, mattes, and transitions belong to later lanes; the
/// layer's content, opacity, masks and motion channels remain authored. The
/// float project depth exposes native geometry instead of RGBA8 quantization.
pub fn isolated_own_layer(document: &Document, index: usize) -> Document {
    use pm_model::Value;
    let mut root = document.to_value().expect("project root");
    let Value::Object(fields) = &mut root else {
        panic!("project root is not an object");
    };
    fields.insert("colorDepth", Value::Number(32.0));
    let Value::Array(layers) = fields.get_mut("layers").expect("open composition layers") else {
        panic!("open composition layers are not an array");
    };
    isolate_layer_list(layers, index);
    Document::new(Some(root), document.revision())
}

/// Isolate one native source within a nested composition, at that
/// composition's own geometry and evaluated source time. Parent precomp
/// placement and resampling remain a separate compositing question.
pub fn isolated_own_layer_in_comp(document: &Document, comp_id: &str, index: usize) -> Document {
    use pm_model::Value;
    let mut root = document.to_value().expect("project root");
    let Value::Object(fields) = &mut root else {
        panic!("project root is not an object");
    };
    fields.insert("colorDepth", Value::Number(32.0));
    let Value::Object(comps) = fields.get_mut("comps").expect("project compositions") else {
        panic!("project compositions are not an object");
    };
    let Value::Object(comp) = comps.get_mut(comp_id).expect("nested composition") else {
        panic!("nested composition is not an object");
    };
    let Value::Array(layers) = comp.get_mut("layers").expect("nested composition layers") else {
        panic!("nested composition layers are not an array");
    };
    isolate_layer_list(layers, index);
    Document::new(Some(root), document.revision())
}

fn isolate_layer_list(layers: &mut [pm_model::Value], index: usize) {
    use pm_model::Value;
    assert!(index < layers.len());
    for (i, layer) in layers.iter_mut().enumerate() {
        let Value::Object(fields) = layer else {
            panic!("layer is not an object");
        };
        if i == index {
            fields.insert("fx", Value::Array(Vec::new()));
            fields.insert("transitionIn", Value::Null);
            fields.insert("transitionOut", Value::Null);
            fields.insert("matteSource", Value::Null);
            fields.insert("blend", Value::String("normal".into()));
            fields.insert("group", Value::Null);
        } else {
            fields.insert("type", Value::String("null".into()));
            fields.insert("group", Value::Null);
            fields.insert("matteSource", Value::Null);
            fields.insert("solo", Value::Bool(false));
        }
    }
}

/// The intrinsic solid source before a 2.5D plane projects it. Content,
/// opacity, masks and motion channels remain authored. Its 3D placement and
/// 2D pose are deliberately canonical: those belong to the projection gate.
pub fn isolated_local_solid(document: &Document, index: usize) -> Document {
    use pm_model::Value;
    let mut root = isolated_own_layer(document, index)
        .to_value()
        .expect("isolated project");
    let Value::Object(root_fields) = &mut root else {
        panic!("project root is not an object");
    };
    let Value::Array(layers) = root_fields.get_mut("layers").expect("open layers") else {
        panic!("open layers are not an array");
    };
    let Value::Object(layer) = &mut layers[index] else {
        panic!("solid layer is not an object");
    };
    assert_eq!(layer.get("type").and_then(Value::as_str), Some("solid"));
    layer.insert("threeD", Value::Bool(false));
    let Value::Object(properties) = layer.get_mut("p").expect("solid properties") else {
        panic!("solid properties are not an object");
    };
    for (name, value) in [
        ("position.x", 0.0),
        ("position.y", 0.0),
        ("anchor.x", 0.0),
        ("anchor.y", 0.0),
        ("scale.x", 100.0),
        ("scale.y", 100.0),
        ("rotation", 0.0),
        ("skew", 0.0),
    ] {
        let Value::Object(channel) = properties.get_mut(name).expect("pose channel") else {
            panic!("pose channel is not a property");
        };
        channel.insert("v", Value::Number(value));
        channel.insert("kf", Value::Array(Vec::new()));
        channel.insert("expr", Value::Null);
    }
    Document::new(Some(root), document.revision())
}

#[derive(Clone, Copy)]
struct Depth {
    precomp: usize,
    matte: usize,
}

/// A reference frame.
#[derive(Clone)]
pub struct Reference {
    pub width: usize,
    pub height: usize,
    /// Premultiplied RGBA in the working encoding; NaN where not modelled.
    pub rgba: Vec<f64>,
    /// Pixels where modelled content is partially covered.
    pub edge: Vec<bool>,
    /// Pixels read from a modelled decoded source through a declared filter.
    pub resampled: Vec<bool>,
    /// What the frame holds that is not modelled (empty when nothing).
    pub unmodelled: Vec<String>,
    /// An unmodelled layer's footprint could not be bounded: the whole
    /// frame is unreferenced.
    pub whole_frame: bool,
}

impl Reference {
    /// Pixel classes: edges grown by 2 pixels; interior elsewhere.
    pub fn classes(&self) -> Vec<Class> {
        classify(&self.edge, &self.resampled, self.width, self.height, 2)
    }

    /// The capture's opaque sRGB byte boundary: over black, independently
    /// gamut-mapped in f64. Unknown source pixels stay explicitly unknown.
    pub fn opaque(&self) -> Vec<f64> {
        self.rgba
            .chunks_exact(4)
            .flat_map(|p| {
                if p.iter().any(|v| v.is_nan()) {
                    return [f64::NAN; 4];
                }
                let rgb = pm_reference::color::css_gamut_map_srgb([p[0], p[1], p[2]]);
                [rgb[0], rgb[1], rgb[2], 1.0]
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
fn gradient(
    fill: &pm_render::plan::Fill,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
) -> Option<[f64; 3]> {
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


/// The source reader is a boundary, not a pixel oracle for the renderer:
/// it supplies decoded texels; all placement, filtering and compositing
/// below are independent f64 calculations.
fn decoded_bitmap(path: &Path) -> Result<Option<Bitmap>, String> {
    let pool = pm_media::pool::FramePool::new(0);
    let (still, failed) =
        pm_render::media::level1_bitmap(path, &pool).map_err(|e| format!("bitmap source {e:?}"))?;
    if failed || still.bit_depth != 8 || !still.frame.format.rgba_premultiplied {
        return Ok(None);
    }
    let (width, height) = (still.frame.format.width, still.frame.format.height);
    let bytes = still
        .frame
        .with_planes(|planes| {
            let plane = planes.first()?;
            if planes.len() != 1 || plane.desc.format != pm_media::frame::PlaneFormat::Rgba8 {
                return None;
            }
            let mut bytes = Vec::with_capacity(width * height * 4);
            for y in 0..height {
                // The plane view owns `height` rows of `stride` bytes.
                bytes.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(plane.ptr.add(y * plane.stride), width * 4)
                });
            }
            Some(bytes)
        })
        .map_err(|e| format!("bitmap planes {e}"))?;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    Ok(Some(Bitmap {
        width,
        height,
        rgba: bytes.into_iter().map(|v| f64::from(v) / 255.0).collect(),
    }))
}

/// The authored fit window in decoded-source coordinates.
fn image_uv(fit: &str, source: (f64, f64), box_size: (f64, f64)) -> Option<[f64; 4]> {
    if ![source.0, source.1, box_size.0, box_size.1]
        .iter()
        .all(|v| v.is_finite() && *v > 0.0)
    {
        return None;
    }
    let source_ratio = source.0 / source.1;
    let box_ratio = box_size.0 / box_size.1;
    Some(match fit {
        "cover" if source_ratio > box_ratio => {
            let span = box_ratio / source_ratio;
            [(1.0 - span) / 2.0, 0.0, span, 1.0]
        }
        "cover" => {
            let span = source_ratio / box_ratio;
            [0.0, (1.0 - span) / 2.0, 1.0, span]
        }
        "contain" if source_ratio > box_ratio => {
            let span = source_ratio / box_ratio;
            [0.0, (1.0 - span) / 2.0, 1.0, span]
        }
        "contain" => {
            let span = box_ratio / source_ratio;
            [(1.0 - span) / 2.0, 0.0, span, 1.0]
        }
        _ => [0.0, 0.0, 1.0, 1.0],
    })
}

/// What a reference pass reads besides the composition being drawn.
struct Ctx<'a, 'd> {
    root: &'d Node,
    assets: &'a dyn AssetResolver,
    effects: &'a pm_render::effects::Effects,
    /// Motion-blur samples per blurred layer (1: none).
    samples: u32,
    /// 8-bit project storage holds 0…1 only.
    unorm8: bool,
    bitmaps: HashMap<PathBuf, Option<Bitmap>>,
}

impl Ctx<'_, '_> {
    fn clamp(&self, c: Rgba) -> Rgba {
        if self.unorm8 {
            c.map(|v| v.clamp(0.0, 1.0))
        } else {
            c
        }
    }
}

fn surface(w: usize, h: usize) -> Reference {
    Reference {
        width: w,
        height: h,
        rgba: vec![0.0; w * h * 4],
        edge: vec![false; w * h],
        resampled: vec![false; w * h],
        unmodelled: Vec::new(),
        whole_frame: false,
    }
}

fn note(out: &mut Reference, why: impl Into<String>) {
    let why = why.into();
    if !out.unmodelled.iter().any(|u| *u == why) {
        out.unmodelled.push(why);
    }
}

/// Track-matte strength from the independently painted, premultiplied
/// source. The authored luma mode uses encoded Rec.709 coefficients.
fn matte_strength(matte: &Option<(Reference, bool, bool)>, index: usize) -> (f64, bool) {
    let Some((surface, luma, inverted)) = matte else {
        return (1.0, false);
    };
    let px = &surface.rgba[index * 4..][..4];
    let value = if *luma {
        pm_reference::composite::luma709([px[0], px[1], px[2]])
    } else {
        px[3]
    };
    (
        if *inverted { 1.0 - value } else { value },
        surface.edge[index] || surface.resampled[index],
    )
}

fn render_matte<'d>(
    ctx: &mut Ctx<'_, 'd>,
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    seconds: f64,
    scale_xy: (f64, f64),
    size: (usize, usize),
    depth: Depth,
) -> Result<Option<(Reference, bool, bool)>, String> {
    if depth.matte >= pm_render::composite::MATTE_MAX_DEPTH as usize {
        return Ok(None);
    }
    let Some(source) = pm_render::composite::build::matte_source(scope.layers(), layer) else {
        return Ok(None);
    };
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let (luma, inverted) =
        pm_render::composite::build::matte_mode(scope, layer, seconds).map_err(fail)?;
    let mut matte = surface(size.0, size.1);
    draw_pass(
        ctx,
        scope,
        seconds,
        scale_xy,
        None,
        Some(source),
        &mut matte,
        Depth {
            matte: depth.matte + 1,
            ..depth
        },
    )?;
    Ok(Some((matte, luma, inverted)))
}

/// The reference frame of `composition` at `time`, `width × height`
/// output pixels at the legacy derived scale, with or without the
/// background (transparent captures draw none), with `samples` motion-blur
/// samples per blurred layer (1: none). `effects`: the definitions the
/// engine draws with (for effect padding). `assets` resolves image sources.
#[allow(clippy::too_many_arguments)]
pub fn render(
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    width: u32,
    height: u32,
    background: bool,
    samples: u32,
    effects: &pm_render::effects::Effects,
    assets: &dyn AssetResolver,
) -> Reference {
    let mut out = surface(width as usize, height as usize);
    if let Err(why) = draw(
        document,
        composition,
        time,
        &mut out,
        background,
        samples,
        effects,
        assets,
    ) {
        out.unmodelled.push(why);
        out.rgba.fill(f64::NAN);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw(
    document: &Document,
    composition: &str,
    time: pm_protocol::common::Ticks,
    out: &mut Reference,
    background: bool,
    samples: u32,
    effects: &pm_render::effects::Effects,
    assets: &dyn AssetResolver,
) -> Result<(), String> {
    let root = document.root().ok_or("no document")?;
    let color = pm_render::composite::color::project_color(root)?;
    if color.working() != WorkingEncoding::Standard {
        return Err("linear-light compositing".into());
    }
    let comp = Comp::resolve(document, composition).map_err(|e| format!("{e:?}"))?;
    let seconds = comp.seconds_at(time.get());
    let mut ctx = Ctx {
        root,
        assets,
        effects,
        samples,
        unorm8: color.precision == Precision::Unorm8,
        bitmaps: HashMap::new(),
    };
    draw_comp(
        &mut ctx,
        &comp,
        &comp,
        seconds,
        out,
        background,
        Depth {
            precomp: 0,
            matte: 0,
        },
    )
}

fn draw_comp<'d>(
    ctx: &mut Ctx<'_, 'd>,
    comp: &Comp<'d>,
    project: &Comp<'d>,
    seconds: f64,
    out: &mut Reference,
    background: bool,
    depth: Depth,
) -> Result<(), String> {
    let (w, h) = (out.width, out.height);
    if background {
        let fill =
            pm_render::plan::normalize_fill(comp.member("backgroundFill"), comp.member("bg"));
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
                out.rgba[(y * w + x) * 4..][..4].copy_from_slice(&ctx.clamp(c));
            }
        }
    }
    let scale = pm_render::plan::derived_scale(comp, w as u32, h as u32);
    // Expressions evaluate as the engine evaluates them (pm-eval with pm-expr).
    let mut ev =
        Evaluator::new().with_expressions(std::sync::Arc::new(pm_render::expr::PmExprEngine));
    let mut scope = Scope::new(&mut ev, comp, project);
    draw_pass(ctx, &mut scope, seconds, scale, None, None, out, depth)
}

/// The scaled world matrix (layer pixels before the size anchor's offset
/// to output pixels) and the device matrix (after it).
fn device_of<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    (sx, sy): (f64, f64),
) -> Result<(kurbo::Affine, kurbo::Affine), String> {
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let world = scope.world_matrix(layer, time).map_err(fail)?;
    let offset = scope.size_anchor_offset(layer, time).map_err(fail)?;
    let scaled = kurbo::Affine::new([
        world[0] * sx,
        world[1] * sy,
        world[2] * sx,
        world[3] * sy,
        world[4] * sx,
        world[5] * sy,
    ]);
    Ok((
        scaled,
        scaled * kurbo::Affine::translate((offset.x, offset.y)),
    ))
}

/// Composites a finished layer (or group) image `px` (premultiplied, with
/// its partial-coverage marks) through opacity, mask and matte.
#[allow(clippy::too_many_arguments)]
fn composite_layer(
    ctx: &Ctx<'_, '_>,
    out: &mut Reference,
    px: &[(Rgba, bool)],
    resampled: Option<&[bool]>,
    opacity: f64,
    mode: Blend,
    mask: &Option<(Vec<f64>, Vec<bool>)>,
    matte: &Option<(Reference, bool, bool)>,
) {
    for (i, (color, edge)) in px.iter().enumerate() {
        let (mask_value, mask_edge) = mask
            .as_ref()
            .map_or((1.0, false), |(coverage, band)| (coverage[i], band[i]));
        let (matte_value, matte_edge) = matte_strength(matte, i);
        let source = scale(*color, opacity * mask_value * matte_value);
        if source[3] == 0.0 && !edge && !source[0].is_nan() {
            continue;
        }
        if mode == Blend::Normal && source[3] >= 1.0 - 1e-12 {
            out.edge[i] = false;
            out.resampled[i] = false;
        }
        out.edge[i] |= *edge || mask_edge || matte_edge;
        if let Some(r) = resampled {
            out.resampled[i] |= r[i];
        }
        let at = &mut out.rgba[i * 4..][..4];
        let dst = [at[0], at[1], at[2], at[3]];
        at.copy_from_slice(&ctx.clamp(blend(mode, source, dst)));
    }
}

/// Why this reference does not model a leaf layer (`None`: it does).
fn unmodelled_leaf<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    ty: &str,
    seconds: f64,
) -> Result<Option<&'static str>, String> {
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let flag = |k: &str| node::truthy(node::member(Some(layer), k));
    Ok(if !matches!(ty, "solid" | "image" | "shape" | "precomp") {
        Some("other layer types")
    } else if flag("threeD") || scope.is_3d_layer(layer).map_err(fail)? {
        Some("2.5D planes")
    } else if scope
        .active_transition(layer, seconds, &|_| true)
        .map_err(fail)?
        .is_some()
    {
        Some("transitions")
    } else if scope.has_renderable_effects(layer, seconds).map_err(fail)? {
        Some("effects")
    } else if flag("mblur") && ty == "precomp" {
        Some("motion-blurred precomps")
    } else {
        None
    })
}

/// A compositing pass, with pure transform groups flattened by pm-eval's
/// authored group index. Boundary groups and precomps have their own
/// transparent f64 surface; their opacity and blend are applied once to the
/// finished image.
#[allow(clippy::too_many_arguments)]
fn draw_pass<'d>(
    ctx: &mut Ctx<'_, 'd>,
    scope: &mut Scope<'_, 'd>,
    seconds: f64,
    scale_xy: (f64, f64),
    parent: Option<&'d Node>,
    explicit: Option<&'d Node>,
    out: &mut Reference,
    depth: Depth,
) -> Result<(), String> {
    let (w, h) = (out.width, out.height);
    let (sx, sy) = scale_xy;
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let all: Vec<&Node> = scope.layers().iter().collect();
    let solo = all
        .iter()
        .any(|l| node::truthy(node::member(Some(l), "solo")));
    let parent_id = parent.map(|layer| node::get(layer, b"id").to_js());
    let mut boundary = |s: &mut Scope<'_, 'd>, layer: &'d Node| {
        s.group_creates_compositing_boundary(layer, seconds, true, &|_| true)
    };
    let layers = if let Some(layer) = explicit {
        vec![layer]
    } else {
        scope
            .compositing_layers(&all, parent_id.as_ref(), &mut boundary)
            .map_err(fail)?
    };
    let matte_sources: Vec<_> = all
        .iter()
        .filter_map(|layer| node::member(Some(layer), "matteSource"))
        .map(|id| node::to_js_string(Some(id)))
        .collect();
    // Motion blur (C-K4): a blurred layer is the average of its content at
    // `t + ((s + ½)/n − ½) · shutter / fps`, masked at the center time.
    let blur = (ctx.samples > 1).then(|| {
        let fps = pm_render::motion::level_fps(scope.comp);
        let shutter = pm_render::motion::shutter_frames(scope.comp.node()) / fps;
        (0..ctx.samples)
            .map(|s| ((f64::from(s) + 0.5) / f64::from(ctx.samples) - 0.5) * shutter)
            .collect::<Vec<f64>>()
    });
    // Bottom (last) to top (first), with today's skip rules.
    for &layer in layers.iter().rev() {
        let ty = pm_render::js::member_string(layer, "type").unwrap_or_default();
        let flag = |k: &str| node::truthy(node::member(Some(layer), k));
        if explicit.is_none()
            && matte_sources
                .iter()
                .any(|id| *id == node::to_js_string(node::member(Some(layer), "id")))
        {
            continue;
        }
        if matches!(ty.as_str(), "null" | "audio") {
            continue;
        }
        if explicit.is_none() && solo && !flag("solo") {
            let within_solo_group = scope
                .group_ancestors(layer)
                .map_err(fail)?
                .iter()
                .any(|group| node::truthy(node::member(Some(group), "solo")));
            let contains_solo = if ty == "group" {
                let mut found = false;
                for candidate in all
                    .iter()
                    .filter(|candidate| node::truthy(node::member(Some(candidate), "solo")))
                {
                    found |= scope
                        .group_ancestors(candidate)
                        .map_err(fail)?
                        .iter()
                        .any(|group| std::ptr::eq(*group, layer));
                }
                found
            } else {
                false
            };
            if !within_solo_group && !contains_solo {
                continue;
            }
        }
        let active = if explicit.is_some() {
            scope.active_forced_on(layer, seconds)
        } else {
            scope.active(layer, seconds)
        };
        if !active.map_err(fail)? {
            continue;
        }
        let opacity = scope.num(layer, "opacity", seconds).map_err(fail)? / 100.0;
        let opacity = if opacity.is_nan() {
            opacity
        } else {
            opacity.clamp(0.0, 1.0)
        };
        if opacity <= 0.001 {
            continue;
        }
        let blurred = blur.as_ref().filter(|_| {
            scope
                .evaluated(
                    layer,
                    node::member(Some(layer), "mblur"),
                    seconds,
                    "l.mblur",
                )
                .is_ok_and(|v| v.truthy())
        });
        let times: Vec<f64> = match blurred {
            Some(offsets) => offsets.iter().map(|o| seconds + o).collect(),
            None => vec![seconds],
        };
        if ty == "adjustment" {
            if scope.has_renderable_effects(layer, seconds).map_err(fail)? {
                note(out, "effects");
                footprint(scope, ctx.effects, layer, &ty, &times, seconds, scale_xy, out)?;
            }
            continue;
        }
        let why = if ty == "group" {
            if flag("threeD") || scope.is_3d_layer(layer).map_err(fail)? {
                note(out, "2.5D planes");
                footprint(scope, ctx.effects, layer, &ty, &times, seconds, scale_xy, out)?;
                continue;
            }
            (blurred.is_some()
                || scope.has_renderable_effects(layer, seconds).map_err(fail)?
                || scope
                    .active_transition(layer, seconds, &|_| true)
                    .map_err(fail)?
                    .is_some())
            .then_some("groups with effects, motion blur or transitions")
        } else {
            unmodelled_leaf(scope, layer, &ty, seconds)?
        };
        if let Some(why) = why {
            note(out, why);
            footprint(scope, ctx.effects, layer, &ty, &times, seconds, scale_xy, out)?;
            continue;
        }
        if ty == "precomp" && depth.precomp >= pm_render::plan::PRECOMP_MAX_DEPTH as usize {
            continue;
        }
        let (scaled, device) = device_of(scope, layer, seconds, scale_xy)?;
        let mask = super::mask_reference::layer_mask(scope, layer, seconds, scaled, w, h)?;
        let matte = if flag("matteSource") {
            render_matte(ctx, scope, layer, seconds, scale_xy, (w, h), depth)?
        } else {
            None
        };
        let mode = reference_blend(
            pm_render::composite::build::blend_mode(scope, layer, seconds).map_err(fail)?,
        );
        if ty == "group" {
            let mut child = surface(w, h);
            draw_pass(
                ctx,
                scope,
                seconds,
                scale_xy,
                Some(layer),
                None,
                &mut child,
                depth,
            )?;
            let px: Vec<(Rgba, bool)> = (0..w * h)
                .map(|i| {
                    (
                        [
                            child.rgba[4 * i],
                            child.rgba[4 * i + 1],
                            child.rgba[4 * i + 2],
                            child.rgba[4 * i + 3],
                        ],
                        child.edge[i],
                    )
                })
                .collect();
            composite_layer(
                ctx,
                out,
                &px,
                Some(&child.resampled),
                opacity,
                mode,
                &mask,
                &matte,
            );
            for why in child.unmodelled {
                note(out, why);
            }
            out.whole_frame |= child.whole_frame;
            continue;
        }
        let d = scope
            .ev
            .resolve_content(scope.comp, layer, seconds)
            .map_err(fail)?;
        if ty == "solid" || ty == "shape" {
            let px = layer_pixels(&times, |t| {
                content_pixels(scope, layer, &ty, t, scale_xy, w, h)
            })?;
            composite_layer(ctx, out, &px, None, opacity, mode, &mask, &matte);
            continue;
        }
        if ty == "image"
            && let Some(svg) = svg_source(ctx.root, &d, ctx.assets)?
        {
            let px = layer_pixels(&times, |t| {
                svg_pixels(scope, layer, t, scale_xy, w, h, &svg)
            })?;
            composite_layer(ctx, out, &px, None, opacity, mode, &mask, &matte);
            continue;
        }
        // Image and precomp sources: decoded (or independently painted) f64
        // texels, sampled at mip level zero through the layer's quad.
        let default_size = |key: &str, fallback: usize| {
            if ty == "precomp" {
                node::to_number(scope.comp.member(key))
            } else {
                fallback as f64
            }
        };
        let (fw, fh) = (default_size("w", w), default_size("h", h));
        let sw = pm_render::js::or_number(pm_render::js::get(&d, "w"), fw);
        let sh = pm_render::js::or_number(pm_render::js::get(&d, "h"), fh);
        let (sw, sh) = if ty == "precomp" {
            (sw.max(1.0), sh.max(1.0))
        } else {
            (sw, sh)
        };
        // Image quads are centered on the layer origin (ax=w/2, ay=h/2);
        // a precomp's box starts at its local origin.
        let rect = if ty == "precomp" {
            kurbo::Rect::new(0.0, 0.0, sw, sh)
        } else {
            kurbo::Rect::new(-sw / 2.0, -sh / 2.0, sw / 2.0, sh / 2.0)
        };
        let precomp = if ty == "precomp" {
            // The child is painted from its own authored layers into f64
            // texels; the parent then filters them. No native framebuffer
            // enters the expected image, and the child's unmodelled pixels
            // stay unknown where the parent samples them.
            let id = pm_render::js::string(pm_render::js::get(&d, "comp"));
            let sub = Comp::in_root(ctx.root, &id, scope.comp.revision())
                .map_err(|e| format!("precomp {id}: {e:?}"))?;
            let target_w = (sw * sx).round().max(2.0);
            let target_h = (sh * sy).round().max(2.0);
            if !target_w.is_finite()
                || !target_h.is_finite()
                || target_w > 16384.0
                || target_h > 16384.0
            {
                note(out, "precomp targets outside reference bounds");
                unreference(out, &(device * kurbo::Shape::to_path(&rect, 1e-9)), 1);
                continue;
            }
            let child_time = scope.source_time(layer, seconds, None).map_err(fail)?;
            let child_time = pm_render::retime::keep_frame_rate(layer, &sub, child_time);
            let mut child = surface(target_w as usize, target_h as usize);
            draw_comp(
                ctx,
                &sub,
                scope.project,
                child_time,
                &mut child,
                false,
                Depth {
                    precomp: depth.precomp + 1,
                    ..depth
                },
            )?;
            for why in &child.unmodelled {
                note(out, format!("precomp {id}: {why}"));
            }
            Some(Bitmap {
                width: child.width,
                height: child.height,
                rgba: child.rgba,
            })
        } else {
            None
        };
        let coverage = fill(
            &(device * kurbo::Shape::to_path(&rect, 1e-9)),
            FillRule::NonZero,
            PATH_SAMPLES,
            w,
            h,
        );
        let asset_id = pm_render::js::string(pm_render::js::get(&d, "asset"));
        let record = ctx.root.get("assets").and_then(|v| v.get(&asset_id));
        if precomp.is_none()
            && let Some(path) = ctx.assets.path(&asset_id, record)
        {
            ctx.bitmaps
                .entry(path.clone())
                .or_insert_with(|| decoded_bitmap(&path).ok().flatten());
        }
        let bitmap = precomp.as_ref().or_else(|| {
            ctx.assets
                .path(&asset_id, record)
                .and_then(|path| ctx.bitmaps.get(&path)?.as_ref())
        });
        let [a, b, c, dd, e, f] = device.as_coeffs();
        let det = a * dd - b * c;
        let natural = record.map(|r| {
            (
                node::to_number(node::member(Some(r), "w")),
                node::to_number(node::member(Some(r), "h")),
            )
        });
        let fit = pm_render::js::string(pm_render::js::get(&d, "fit"));
        let model = bitmap.filter(|_| blurred.is_none()).and_then(|bitmap| {
            let source = natural.unwrap_or((bitmap.width as f64, bitmap.height as f64));
            let uv = if ty == "precomp" {
                Some([0.0, 0.0, 1.0, 1.0])
            } else {
                image_uv(&fit, source, (sw, sh))
            }?;
            let du = uv[2] * bitmap.width as f64 / sw;
            let dv = uv[3] * bitmap.height as f64 / sh;
            let dx = (du * dd / det, -dv * b / det);
            let dy = (-du * c / det, dv * a / det);
            (det.is_finite()
                && det.abs() > 1e-12
                && Bitmap::mip_zero(dx, dy)
                && (ty == "precomp"
                    || record.is_some_and(|r| pm_render::footage::interpretation(r).is_none()))
                && !pm_render::sampling::is_level2(layer))
            .then_some((bitmap, uv, dx, dy))
        });
        let Some((bitmap, uv, dx, dy)) = model else {
            note(
                out,
                format!("{ty} sources outside the f64 mip-zero model (resampled content)"),
            );
            unreference(out, &(device * kurbo::Shape::to_path(&rect, 1e-9)), 1);
            continue;
        };
        let centered = if ty == "precomp" { 0.0 } else { 0.5 };
        let mut px = vec![([0.0; 4], false); w * h];
        let mut resampled = vec![false; w * h];
        for y in coverage.y0..coverage.y0 + coverage.height {
            for x in coverage.x0..coverage.x0 + coverage.width {
                let k = coverage.at(x, y);
                if k <= 0.0 {
                    continue;
                }
                let p = (x as f64 + 0.5 - e, y as f64 + 0.5 - f);
                let local_x = (dd * p.0 - c * p.1) / det;
                let local_y = (-b * p.0 + a * p.1) / det;
                let u = (uv[0] + uv[2] * (local_x / sw + centered)).clamp(0.0, 1.0)
                    * bitmap.width as f64;
                let v = (uv[1] + uv[3] * (local_y / sh + centered)).clamp(0.0, 1.0)
                    * bitmap.height as f64;
                let index = y * w + x;
                px[index] = (
                    scale(bitmap.sample((u, v), dx, dy), k),
                    k < 1.0 - 1e-12,
                );
                resampled[index] = true;
            }
        }
        composite_layer(ctx, out, &px, Some(&resampled), opacity, mode, &mask, &matte);
    }
    Ok(())
}

/// A layer's own pixels: one draw at `times[0]`, or the average of every
/// motion-blur sample (partial wherever the samples differ).
fn layer_pixels(
    times: &[f64],
    mut draw: impl FnMut(f64) -> Result<Vec<(Rgba, bool)>, String>,
) -> Result<Vec<(Rgba, bool)>, String> {
    let mut px: Option<Vec<(Rgba, bool)>> = None;
    for &t in times {
        let sample = draw(t)?;
        let acc = px.get_or_insert_with(|| vec![([0.0; 4], false); sample.len()]);
        for (acc, (color, edge)) in acc.iter_mut().zip(sample) {
            for (sum, value) in acc.0.iter_mut().zip(color) {
                *sum += value / times.len() as f64;
            }
            acc.1 |= edge;
        }
    }
    let mut px = px.unwrap_or_default();
    if times.len() > 1 {
        for p in &mut px {
            p.1 |= p.0[3] > 0.0 && p.0[3] < 1.0 - 1e-12;
        }
    }
    Ok(px)
}

struct SvgSource {
    tree: usvg::Tree,
    /// Natural image size saved with the asset, used by the authored fit.
    natural: (f64, f64),
}

/// SVG image fit in authored UV coordinates. Kept in the reference so the
/// renderer's image planning cannot determine both sides of the comparison.
fn svg_fit_window(fit: &str, sw: f64, sh: f64, bw: f64, bh: f64) -> [f64; 4] {
    let source_aspect = sw / sh;
    let box_aspect = bw / bh;
    match fit {
        "cover" if source_aspect > box_aspect => {
            let size = box_aspect / source_aspect;
            [(1.0 - size) / 2.0, 0.0, size, 1.0]
        }
        "cover" => {
            let size = source_aspect / box_aspect;
            [0.0, (1.0 - size) / 2.0, 1.0, size]
        }
        "contain" if source_aspect > box_aspect => {
            let size = source_aspect / box_aspect;
            [0.0, (1.0 - size) / 2.0, 1.0, size]
        }
        "contain" => {
            let size = box_aspect / source_aspect;
            [(1.0 - size) / 2.0, 0.0, size, 1.0]
        }
        _ => [0.0, 0.0, 1.0, 1.0],
    }
}

fn svg_source(
    root: &Node,
    content: &pm_eval::Content,
    assets: &dyn AssetResolver,
) -> Result<Option<SvgSource>, String> {
    let id = pm_render::js::string(pm_render::js::get(content, "asset"));
    let Some(record) = node::member(node::member(Some(root), "assets"), &id) else {
        return Ok(None);
    };
    let format = node::string(node::member(Some(record), "format"))
        .map(|s| s.to_string_lossy().to_ascii_lowercase());
    if format.as_deref() != Some("svg") {
        return Ok(None);
    }
    let path = assets
        .path(&id, Some(record))
        .ok_or("SVG reference asset is offline")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("SVG reference asset: {e}"))?;
    let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default())
        .map_err(|e| format!("SVG reference parse: {e}"))?;
    let n = |key: &str| {
        pm_render::js::or_number(&node::js_value_opt(node::member(Some(record), key)), 1.0)
    };
    Ok(Some(SvgSource {
        tree,
        natural: (n("w"), n("h")),
    }))
}

/// Direct usvg geometry feeds the independent f64 path/gradient/group/clip
/// painter. Unsupported constructs use separate resvg 32× RGBA8 samples
/// averaged in f64, with a half-CV paint quantization limitation. Placement,
/// fit, crop and layer compositing are independent f64 math in both paths.
#[allow(clippy::too_many_arguments)]
fn svg_pixels<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    (sx, sy): (f64, f64),
    w: usize,
    h: usize,
    source: &SvgSource,
) -> Result<Vec<(Rgba, bool)>, String> {
    const RATE: usize = 32;
    let rate = RATE;
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let world = scope.world_matrix(layer, time).map_err(fail)?;
    let offset = scope.size_anchor_offset(layer, time).map_err(fail)?;
    let device = Affine::new([
        world[0] * sx,
        world[1] * sy,
        world[2] * sx,
        world[3] * sy,
        world[4] * sx,
        world[5] * sy,
    ]) * Affine::translate((offset.x, offset.y));
    let content = scope
        .ev
        .resolve_content(scope.comp, layer, time)
        .map_err(fail)?;
    let bw = pm_render::js::or_number(pm_render::js::get(&content, "w"), w as f64);
    let bh = pm_render::js::or_number(pm_render::js::get(&content, "h"), h as f64);
    let fit = pm_render::js::string(pm_render::js::get(&content, "fit"));
    let [u0, v0, us, vs] = svg_fit_window(&fit, source.natural.0, source.natural.1, bw, bh);
    let sw = f64::from(source.tree.size().width());
    let sh = f64::from(source.tree.size().height());
    if !(bw > 0.0 && bh > 0.0 && us > 0.0 && vs > 0.0 && sw > 0.0 && sh > 0.0) {
        return Ok(vec![([0.0; 4], false); w * h]);
    }
    let to_layer = Affine::translate((-bw / 2.0 - u0 * bw / us, -bh / 2.0 - v0 * bh / vs))
        * Affine::scale_non_uniform(bw / (us * sw), bh / (vs * sh));
    let fine = Affine::scale(rate as f64) * device * to_layer;
    let clip = [
        Point::new((u0 * sw).max(0.0), (v0 * sh).max(0.0)),
        Point::new(((u0 + us) * sw).min(sw), (v0 * sh).max(0.0)),
        Point::new(((u0 + us) * sw).min(sw), ((v0 + vs) * sh).min(sh)),
        Point::new((u0 * sw).max(0.0), ((v0 + vs) * sh).min(sh)),
    ]
    .map(|p| fine * p);
    let mut out = vec![([0.0; 4], false); w * h];
    let bounds = |axis: usize, extent: usize| {
        let min = clip
            .iter()
            .map(|p| [p.x, p.y][axis])
            .fold(f64::INFINITY, f64::min);
        let max = clip
            .iter()
            .map(|p| [p.x, p.y][axis])
            .fold(f64::NEG_INFINITY, f64::max);
        (
            ((min / rate as f64).floor() - 1.0).clamp(0.0, extent as f64) as usize,
            ((max / rate as f64).ceil() + 1.0).clamp(0.0, extent as f64) as usize,
        )
    };
    let (left, right) = bounds(0, w);
    let (top, bottom) = bounds(1, h);
    if left >= right || top >= bottom {
        return Ok(out);
    }
    let crop_device = clip.map(|point| Point::new(point.x / rate as f64, point.y / rate as f64));
    if let Some(pixels) = svg_f64::paint(
        &source.tree,
        device * to_layer,
        &crop_device,
        (left, top, right, bottom),
        w,
        h,
    ) {
        return Ok(pixels);
    }
    let fw = (right - left)
        .checked_mul(rate)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("SVG reference width overflow")?;
    let fh = (bottom - top)
        .checked_mul(rate)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or("SVG reference height overflow")?;
    let mut pixmap = tiny_skia::Pixmap::new(fw, fh).ok_or("SVG reference pixmap allocation")?;
    let shifted = Affine::translate((-((left * rate) as f64), -((top * rate) as f64))) * fine;
    let c = shifted.as_coeffs().map(|value| value as f32);
    resvg::render(
        &source.tree,
        tiny_skia::Transform::from_row(c[0], c[1], c[2], c[3], c[4], c[5]),
        &mut pixmap.as_mut(),
    );
    let mut path = tiny_skia::PathBuilder::new();
    for (i, point) in clip.iter().enumerate() {
        let x = point.x - (left * rate) as f64;
        let y = point.y - (top * rate) as f64;
        if i == 0 {
            path.move_to(x as f32, y as f32);
        } else {
            path.line_to(x as f32, y as f32);
        }
    }
    path.close();
    let path = path.finish().ok_or("SVG reference crop path")?;
    let mut mask = tiny_skia::Mask::new(fw, fh).ok_or("SVG reference crop mask")?;
    mask.fill_path(
        &path,
        tiny_skia::FillRule::Winding,
        true,
        tiny_skia::Transform::identity(),
    );
    pixmap.apply_mask(&mask);
    let raster = pixmap.take();
    let fine_width = fw as usize;
    let divisor = 255.0 * (rate * rate) as f64;
    for y in 0..bottom - top {
        for x in 0..right - left {
            let dst = &mut out[(top + y) * w + left + x];
            let mut low = [u8::MAX; 4];
            let mut high = [u8::MIN; 4];
            for fy in 0..rate {
                for fx in 0..rate {
                    let at = ((y * rate + fy) * fine_width + x * rate + fx) * 4;
                    for (channel, (sum, byte)) in
                        dst.0.iter_mut().zip(&raster[at..at + 4]).enumerate()
                    {
                        *sum += f64::from(*byte) / divisor;
                        low[channel] = low[channel].min(*byte);
                        high[channel] = high[channel].max(*byte);
                    }
                }
            }
            // A hard internal contour may leave the final image opaque.
            // Fine-cell colour variation still identifies its coverage edge.
            dst.1 = (dst.0[3] > 1e-8 && dst.0[3] < 1.0 - 1e-8)
                || (0..4).any(|c| high[c].saturating_sub(low[c]) > 2);
        }
    }
    Ok(out)
}

/// A solid or shape layer's own pixels at `time` (premultiplied, before
/// opacity and masks), and where they are partially covered.
fn content_pixels<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    ty: &str,
    time: f64,
    (sx, sy): (f64, f64),
    w: usize,
    h: usize,
) -> Result<Vec<(Rgba, bool)>, String> {
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let world = scope.world_matrix(layer, time).map_err(fail)?;
    let offset = scope.size_anchor_offset(layer, time).map_err(fail)?;
    let device = kurbo::Affine::new([
        world[0] * sx,
        world[1] * sy,
        world[2] * sx,
        world[3] * sy,
        world[4] * sx,
        world[5] * sy,
    ]) * kurbo::Affine::translate((offset.x, offset.y));
    let d = scope
        .ev
        .resolve_content(scope.comp, layer, time)
        .map_err(fail)?;
    if ty == "shape" {
        // The authored geometry and paint (pm-vector's, gated exactly by its
        // own tests), covered and composited here.
        let has_paths = node::array(node::member(node::member(Some(layer), "d"), "paths"))
            .is_some_and(|p| !p.is_empty());
        let scene = if has_paths {
            let (_, colors, paths) = pm_render::vector::paths(scope, layer, time).map_err(fail)?;
            pm_render::vector::paths_scene(&paths, &colors, &device.as_coeffs())
        } else {
            let (content, colors) = pm_render::vector::shape_content(&d);
            match pm_render::vector::shape_scene(&content, &colors, &device.as_coeffs()) {
                Some(scene) => scene,
                None => return Ok(vec![([0.0; 4], false); w * h]),
            }
        };
        return scene_pixels(&scene, w, h);
    }
    let sw = pm_render::js::or_number(pm_render::js::get(&d, "w"), w as f64);
    let sh = pm_render::js::or_number(pm_render::js::get(&d, "h"), h as f64);
    let paint =
        hex(&pm_render::js::string(pm_render::js::get(&d, "color"))).ok_or("solid color")?;
    // `radius` rounds the corners, at most half of either side
    // (SOLID-RADIUS-1: the inspector edits it).
    let radius = pm_render::js::get(&d, "radius").to_number();
    let radius = if radius > 0.0 {
        radius.min(sw.abs() / 2.0).min(sh.abs() / 2.0)
    } else {
        0.0
    };
    let outline = kurbo::Shape::to_path(&kurbo::RoundedRect::new(0.0, 0.0, sw, sh, radius), 1e-9);
    let coverage = fill(&(device * outline), FillRule::NonZero, PATH_SAMPLES, w, h);
    let src: Rgba = [
        paint[0] * paint[3],
        paint[1] * paint[3],
        paint[2] * paint[3],
        paint[3],
    ];
    let mut out = vec![([0.0; 4], false); w * h];
    for y in coverage.y0..coverage.y0 + coverage.height {
        for x in coverage.x0..coverage.x0 + coverage.width {
            let k = coverage.at(x, y);
            if k > 0.0 {
                out[y * w + x] = (scale(src, k), k < 1.0 - 1e-12);
            }
        }
    }
    Ok(out)
}

/// Marks the pixels `outline` (output pixels) touches, grown by `grow`
/// pixels, unreferenced.
fn unreference(out: &mut Reference, outline: &kurbo::BezPath, grow: usize) {
    let (w, h) = (out.width, out.height);
    let c = fill(outline, FillRule::NonZero, 1, w, h);
    let mut inside = vec![false; w * h];
    for y in c.y0..c.y0 + c.height {
        for x in c.x0..c.x0 + c.width {
            inside[y * w + x] = c.at(x, y) > 0.0;
        }
    }
    for (i, _) in pm_reference::compare::dilate(&inside, w, h, grow)
        .iter()
        .enumerate()
        .filter(|(_, g)| **g)
    {
        out.rgba[i * 4..][..4].fill(f64::NAN);
    }
}

/// An unmodelled layer's footprint: the device-aligned bounds of its full
/// input texture for effects (including transparent rotated corners), otherwise its painted content
/// for solids and shapes, at every time it is drawn. Effect read padding and
/// pixelate's output block reach are in output pixels, with two more pixels
/// for raster/edge filtering. It is
/// unreferenced. A footprint this cannot bound (groups, precomps, 2.5D
/// planes, transitions, which move content) is the whole frame.
#[allow(clippy::too_many_arguments)]
fn footprint<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &pm_render::effects::Effects,
    layer: &'d Node,
    ty: &str,
    times: &[f64],
    seconds: f64,
    (sx, sy): (f64, f64),
    out: &mut Reference,
) -> Result<(), String> {
    let fail = |e: pm_eval::JsThrow| format!("evaluation: {e:?}");
    let (w, h) = (out.width, out.height);
    let content = matches!(ty, "solid" | "shape");
    let boxed = matches!(ty, "image" | "shader" | "extension");
    let bounded = (content || boxed)
        && !node::truthy(node::member(Some(layer), "group"))
        && !scope.is_3d_layer(layer).map_err(fail)?
        && scope
            .active_transition(layer, seconds, &|_| true)
            .map_err(fail)?
            .is_none();
    let steps =
        pm_render::effects::plan::effect_steps(scope, effects, layer, seconds).map_err(fail)?;
    let (effectful, padding, output_reach) = match steps {
        Ok(steps) => {
            // Pixelate samples the center of each device-space block and
            // paints that sample across the whole block. Its declared
            // viewportPadding is zero because it needs no source outside
            // the input texture; that does not bound where its output lands.
            // One block width per pass bounds the spread for any grid phase.
            let output_reach = steps
                .iter()
                .filter(|step| step.definition == "pixelate")
                .map(|step| {
                    let size = step
                        .params
                        .iter()
                        .find_map(|param| (param.key == "size").then_some(param.value));
                    let block = match size {
                        Some(pm_render::effects::plan::Binding::Float(value)) => value.max(1.0),
                        _ => 1.0,
                    };
                    block.ceil().clamp(0.0, f64::from(u32::MAX / 4)) as u32
                })
                .fold(0_u32, u32::saturating_add);
            (
                !steps.is_empty(),
                pm_render::effects::plan::chain_padding(&steps),
                output_reach,
            )
        }
        Err(_) => (true, pm_render::composite::Padding::Full, 0),
    };
    if !bounded || padding == pm_render::composite::Padding::Full {
        out.rgba.fill(f64::NAN);
        out.whole_frame = true;
        return Ok(());
    }
    let mut covered = vec![false; w * h];
    let pm_render::composite::Padding::Pixels(padding) = padding else {
        unreachable!()
    };
    let grow = padding.saturating_add(output_reach).saturating_add(2) as usize;
    for &t in times {
        let world = scope.world_matrix(layer, t).map_err(fail)?;
        let offset = scope.size_anchor_offset(layer, t).map_err(fail)?;
        let device = kurbo::Affine::new([
            world[0] * sx,
            world[1] * sy,
            world[2] * sx,
            world[3] * sy,
            world[4] * sx,
            world[5] * sy,
        ]) * kurbo::Affine::translate((offset.x, offset.y));
        if content && !effectful {
            for (i, (px, edge)) in content_pixels(scope, layer, ty, t, (sx, sy), w, h)?
                .into_iter()
                .enumerate()
            {
                covered[i] |= px[3] > 0.0 || edge;
            }
        } else {
            let d = scope
                .ev
                .resolve_content(scope.comp, layer, t)
                .map_err(fail)?;
            let rect = match ty {
                "solid" => {
                    let bw = pm_render::js::or_number(pm_render::js::get(&d, "w"), w as f64);
                    let bh = pm_render::js::or_number(pm_render::js::get(&d, "h"), h as f64);
                    kurbo::Rect::new(0.0, 0.0, bw, bh)
                }
                "shape" => {
                    let has_paths =
                        node::array(node::member(node::member(Some(layer), "d"), "paths"))
                            .is_some_and(|paths| !paths.is_empty());
                    if has_paths {
                        let (_, _, paths) =
                            pm_render::vector::paths(scope, layer, t).map_err(fail)?;
                        let b = paths.bounds;
                        kurbo::Rect::new(b.x0, b.y0, b.x1, b.y1)
                    } else {
                        let (shape, _) = pm_render::vector::shape_content(&d);
                        let raster = pm_vector::shape::ShapeRaster::new(&shape, 1.0);
                        kurbo::Rect::new(
                            -raster.w / 2.0,
                            -raster.h / 2.0,
                            raster.w / 2.0,
                            raster.h / 2.0,
                        )
                    }
                }
                _ => {
                    let bw = pm_render::js::or_number(pm_render::js::get(&d, "w"), w as f64);
                    let bh = pm_render::js::or_number(pm_render::js::get(&d, "h"), h as f64);
                    kurbo::Rect::new(-bw / 2.0, -bh / 2.0, bw / 2.0, bh / 2.0)
                }
            };
            // Native effects run on an axis-aligned device viewport texture.
            // Its transparent corner pixels can be painted by a later effect;
            // a rotated polygon would incorrectly leave those pixels modelled.
            let rect = if effectful {
                device.transform_rect_bbox(rect)
            } else {
                rect
            };
            let outline = if effectful {
                kurbo::Shape::to_path(&rect, 1e-9)
            } else {
                device * kurbo::Shape::to_path(&rect, 1e-9)
            };
            let f = fill(&outline, FillRule::NonZero, 1, w, h);
            for y in f.y0..f.y0 + f.height {
                for x in f.x0..f.x0 + f.width {
                    covered[y * w + x] |= f.at(x, y) > 0.0;
                }
            }
        }
    }
    for (i, g) in pm_reference::compare::dilate(&covered, w, h, grow)
        .into_iter()
        .enumerate()
    {
        if g {
            out.rgba[i * 4..][..4].fill(f64::NAN);
        }
    }
    Ok(())
}

/// Device-pixel tolerance of the reference's stroke outlines.
const STROKE_TOLERANCE: f64 = 1e-4;

/// A vector layer's own pixels (premultiplied, source-over item by item in
/// order) over a `w × h` frame, and where any item is partially covered.
/// Fills are covered exactly; strokes are the exact region their joins and
/// caps define (`pm_reference::stroke`), covered the same way.
pub fn scene_pixels(
    scene: &pm_vector::render::scene::CoverageScene,
    w: usize,
    h: usize,
) -> Result<Vec<(Rgba, bool)>, String> {
    use pm_vector::render::scene::{FillRule as Rule, ItemStyle};
    let mut out = vec![([0.0; 4], false); w * h];
    for item in &scene.items {
        let (device, rule, cells) = match &item.style {
            ItemStyle::Fill(rule) => (
                item.transform * item.path.clone(),
                match rule {
                    Rule::NonZero => FillRule::NonZero,
                    Rule::EvenOdd => FillRule::EvenOdd,
                },
                PATH_SAMPLES,
            ),
            ItemStyle::Stroke(stroke) => {
                let c = item.transform.as_coeffs();
                let scale = c[0].hypot(c[1]).max(c[2].hypot(c[3])).max(1e-12);
                let region =
                    pm_reference::stroke::outline(&item.path, stroke, STROKE_TOLERANCE / scale)
                        .ok_or("invalid stroke parameters")?;
                // Overlapping pieces at the edge count within a cell: finer
                // cells bound that error (1/n² of a pixel per cell).
                let cells = if region.folds {
                    4 * PATH_SAMPLES
                } else {
                    PATH_SAMPLES
                };
                (item.transform * region.path, FillRule::NonZero, cells)
            }
        };
        let coverage = fill(&device, rule, cells, w, h);
        // A contour can coincide with a pixel boundary: neither adjoining
        // pixel then has partial coverage, although both touch the edge.
        // Mark the occupancy transition before the comparator grows it by
        // two pixels. This also covers an item's boundary hidden beneath a
        // later, fully opaque item of another color.
        let x0 = coverage.x0.saturating_sub(1);
        let y0 = coverage.y0.saturating_sub(1);
        let x1 = (coverage.x0 + coverage.width + 1).min(w);
        let y1 = (coverage.y0 + coverage.height + 1).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let here = coverage.at(x, y) > 0.0;
                let boundary = (x > 0 && (coverage.at(x - 1, y) > 0.0) != here)
                    || (x + 1 < w && (coverage.at(x + 1, y) > 0.0) != here)
                    || (y > 0 && (coverage.at(x, y - 1) > 0.0) != here)
                    || (y + 1 < h && (coverage.at(x, y + 1) > 0.0) != here);
                out[y * w + x].1 |= boundary;
            }
        }
        let paint = item.premultiplied().map(f64::from);
        for y in coverage.y0..coverage.y0 + coverage.height {
            for x in coverage.x0..coverage.x0 + coverage.width {
                let k = coverage.at(x, y);
                if k <= 0.0 {
                    continue;
                }
                let (px, edge) = &mut out[y * w + x];
                *edge |= k < 1.0 - 1e-12;
                *px = blend(Blend::Normal, scale(paint, k), *px);
            }
        }
    }
    Ok(out)
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
        let f = renderer
            .render(document, composition, time, &r, resolver)
            .unwrap();
        renderer.read_rgba(&f).unwrap()
    };
    (frame(&preview), frame(&export))
}

#[cfg(test)]
mod tests {
    use super::super::doc;
    use super::*;
    use serde_json::json;

    fn authored_graph_frame(mut project: serde_json::Value, width: u32, height: u32) -> Reference {
        project["colorDepth"] = json!(32);
        let loaded = pm_model::json::parse(&project.to_string()).expect("authored graph JSON");
        let document = Document::new(Some(loaded), 1);
        render(
            &document,
            "Ptest-root",
            super::super::ticks(0.0),
            width,
            height,
            false,
            1,
            &pm_render::effects::Effects::new("reference tests"),
            &pm_render::MapResolver::default(),
        )
    }

    fn pixel(frame: &Reference, x: usize, y: usize) -> [f64; 4] {
        let i = (y * frame.width + x) * 4;
        frame.rgba[i..i + 4].try_into().expect("RGBA pixel")
    }

    fn assert_pixel(actual: [f64; 4], expected: [f64; 4]) {
        for (a, e) in actual.into_iter().zip(expected) {
            assert!((a - e).abs() < 1e-12, "{actual:?} != {expected:?}");
        }
    }

    fn solid(id: &str, color: &str, width: u32, height: u32, x: f64, y: f64) -> serde_json::Value {
        doc::layer(
            id,
            "solid",
            json!({ "color": color, "w": width, "h": height }),
            x,
            y,
            1.0,
        )
    }

    #[test]
    fn graph_reference_composites_group_once_before_opacity_and_blend() {
        // Top green completely covers lower red within the group. A 50%
        // multiply group over gray therefore has no red from the child.
        let mut top = solid("top", "#00FF00", 4, 4, 0.0, 0.0);
        top["group"] = json!("g");
        let mut lower = solid("lower", "#FF0000", 4, 4, 0.0, 0.0);
        lower["group"] = json!("g");
        let mut group = doc::layer("g", "group", json!({}), 0.0, 0.0, 1.0);
        group["blend"] = json!("multiply");
        doc::set(&mut group, "opacity", json!(50));
        let back = solid("back", "#808080", 4, 4, 0.0, 0.0);
        let frame = authored_graph_frame(
            doc::project(4, 4, 1.0, doc::no_fill(), vec![top, lower, group, back]),
            4,
            4,
        );
        assert_eq!(frame.modelled(), 1.0, "{:?}", frame.unmodelled);
        assert_pixel(
            pixel(&frame, 2, 2),
            [64.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0, 1.0],
        );
    }

    #[test]
    fn graph_reference_paints_child_before_parent_precomp_blend() {
        let top = solid("top", "#00FF00", 4, 4, 0.0, 0.0);
        let lower = solid("lower", "#FF0000", 4, 4, 0.0, 0.0);
        let mut nested = doc::layer(
            "nested",
            "precomp",
            json!({ "comp": "C001", "w": 4, "h": 4 }),
            0.0,
            0.0,
            1.0,
        );
        nested["blend"] = json!("multiply");
        doc::set(&mut nested, "opacity", json!(50));
        let back = solid("back", "#808080", 4, 4, 0.0, 0.0);
        let mut project = doc::project(4, 4, 1.0, doc::no_fill(), vec![nested, back]);
        doc::composition(&mut project, "C001", 4, 4, 1.0, vec![top, lower]);
        let frame = authored_graph_frame(project, 4, 4);
        assert_eq!(frame.modelled(), 1.0, "{:?}", frame.unmodelled);
        // Opaque green hides red in C001; the parent applies multiply at 50%.
        assert_pixel(
            pixel(&frame, 2, 2),
            [64.0 / 255.0, 128.0 / 255.0, 64.0 / 255.0, 1.0],
        );
    }

    #[test]
    fn graph_reference_matte_source_uses_fractional_coverage_and_is_hidden() {
        // The source spans x=.5..2.5: [half, full, half, empty] coverage.
        // It is green with 128/255 alpha, so encoded Rec.709 luma is
        // 0.7152 times its premultiplied alpha.
        for (mode, luma, inverted) in [
            ("alpha", false, false),
            ("alpha-inverted", false, true),
            ("luma", true, false),
            ("luma-inverted", true, true),
        ] {
            let mut target = solid("target", "#FF0000", 4, 2, 0.0, 0.0);
            target["matteSource"] = json!("matte");
            target["matteMode"] = doc::ch(json!(mode));
            let matte = solid("matte", "#00FF0080", 2, 2, 0.5, 0.0);
            let frame = authored_graph_frame(
                doc::project(4, 2, 1.0, doc::no_fill(), vec![target, matte]),
                4,
                2,
            );
            assert_eq!(frame.modelled(), 1.0, "{mode}: {:?}", frame.unmodelled);
            for (x, coverage) in [0.5, 1.0, 0.5, 0.0].into_iter().enumerate() {
                let source = coverage * (128.0 / 255.0) * if luma { 0.7152 } else { 1.0 };
                let alpha = if inverted { 1.0 - source } else { source };
                assert_pixel(pixel(&frame, x, 0), [alpha, 0.0, 0.0, alpha]);
            }
        }
    }

    #[test]
    fn fit_windows_preserve_authored_cover_and_contain_geometry() {
        assert_eq!(
            image_uv("cover", (2.0, 1.0), (1.0, 1.0)),
            Some([0.25, 0.0, 0.5, 1.0])
        );
        assert_eq!(
            image_uv("contain", (2.0, 1.0), (1.0, 1.0)),
            Some([0.0, -0.5, 1.0, 2.0])
        );
        assert_eq!(
            image_uv("fill", (2.0, 1.0), (1.0, 1.0)),
            Some([0.0, 0.0, 1.0, 1.0])
        );
    }
}

#[cfg(test)]
mod svg_sampling_tests {
    /// A fractional rounded contour, nested clip, gradient, and group
    /// opacity exercise the same resvg storage and f64 accumulation as the
    /// corpus reference, without reading any engine-rendered pixels.
    #[test]
    fn supersampled_rgba8_reference_converges() {
        const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48">
          <defs><linearGradient id="g"><stop stop-color="#23a7f4"/><stop offset="1" stop-color="#ec4760"/></linearGradient>
          <clipPath id="c"><circle cx="25.37" cy="23.19" r="18.23"/></clipPath></defs>
          <g opacity="0.73" clip-path="url(#c)"><rect x="4.31" y="5.17" width="38.41" height="35.86" rx="6.47" fill="url(#g)"/></g>
        </svg>"##;
        let tree = usvg::Tree::from_data(SVG.as_bytes(), &usvg::Options::default()).unwrap();
        let raster = |rate: usize| {
            let side = (48 * rate) as u32;
            let mut pixmap = tiny_skia::Pixmap::new(side, side).unwrap();
            resvg::render(
                &tree,
                tiny_skia::Transform::from_scale(rate as f32, rate as f32),
                &mut pixmap.as_mut(),
            );
            let data = pixmap.data();
            let mut pixels = vec![[0.0_f64; 4]; 48 * 48];
            for y in 0..48 {
                for x in 0..48 {
                    for sy in 0..rate {
                        for sx in 0..rate {
                            let index = ((y * rate + sy) * 48 * rate + x * rate + sx) * 4;
                            for c in 0..4 {
                                pixels[y * 48 + x][c] +=
                                    f64::from(data[index + c]) / (255.0 * (rate * rate) as f64);
                            }
                        }
                    }
                }
            }
            pixels
        };
        let (at8, at16, at32) = (raster(8), raster(16), raster(32));
        let delta = |a: &[[f64; 4]], b: &[[f64; 4]]| {
            let mut max: f64 = 0.0;
            let mut sum = 0.0;
            for (a, b) in a.iter().zip(b) {
                for c in 0..4 {
                    let cv = (a[c] - b[c]).abs() * 255.0;
                    max = max.max(cv);
                    sum += cv;
                }
            }
            (max, sum / (a.len() * 4) as f64)
        };
        let coarse = delta(&at8, &at16);
        let fine = delta(&at16, &at32);
        assert!(fine.1 < coarse.1, "8→16 {coarse:?}, 16→32 {fine:?}");
        assert!(fine.1 <= 0.05, "16→32 mean CV {fine:?}");
        // Each RGBA8 fine sample introduces at most 0.5 CV per channel;
        // averaging samples in f64 does not accumulate that storage bound.
        // A contour still moves across individual sample cells, so its
        // pointwise convergence is bounded separately from storage error.
        assert!(
            fine.0 < coarse.0 && fine.0 <= 2.0,
            "8→16 {coarse:?}, 16→32 {fine:?}"
        );
    }
}
