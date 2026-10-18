//! Plan-time reading of a layer's compositing properties: blend mode,
//! masks, and track matte, each exactly as today's `renderProject` reads
//! them (`legacy/gl/compositor.ts`).

use pm_eval::node;
use pm_eval::{JsResult, JsThrow, JsValue, Scope};
use pm_model::Node;
use pm_protocol::color::BlendMode;
use pm_vector::mask::{LayerMasks, MaskMode, MaskShape, MaskValues};

use super::Masks;

/// `BLEND_ID[isProperty(L.blend) ? PM.evP(L, L.blend, T, 'l.blend') : L.blend] || 0`.
pub fn blend_mode<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
) -> JsResult<BlendMode> {
    let blend = scope.evaluated(layer, node::member(Some(layer), "blend"), time, "l.blend")?;
    Ok(match blend {
        JsValue::Undefined | JsValue::Null => BlendMode::Normal,
        v => pm_color::linear::blend_mode_from_name(&v.to_js_string().to_string_lossy()),
    })
}

/// `(L.masks || []).some(m => evaluatedValue(PM, L, m.on, T, 'm.<id>.on') !== false)`.
/// A truthy `masks` that is not an array throws, as `.some` does today.
pub fn has_masks<'d>(scope: &mut Scope<'_, 'd>, layer: &'d Node, time: f64) -> JsResult<bool> {
    let masks = node::member(Some(layer), "masks");
    if !node::truthy(masks) {
        return Ok(false);
    }
    let Some(masks) = node::array(masks) else {
        return Err(JsThrow::type_error(
            "(L.masks || []).some is not a function",
        ));
    };
    for m in masks {
        if mask_enabled(scope, layer, m, time)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn mask_enabled<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    m: &'d Node,
    time: f64,
) -> JsResult<bool> {
    let id = node::to_js_string(node::member_checked(Some(m), "id")?).to_string_lossy();
    let on = node::member_checked(Some(m), "on")?;
    Ok(scope.evaluated(layer, on, time, &format!("m.{id}.on"))? != JsValue::Bool(false))
}

/// What `applyMasks` does with a layer's masks.
#[derive(Debug, Clone, PartialEq)]
pub enum MaskRoute {
    /// No enabled mask with values, or a singular world matrix: the layer
    /// is left as it is.
    Unmasked,
    Parametric(Masks),
    /// Some enabled mask has a bezier path: every mask goes through the
    /// Canvas2D coverage route (`applyPathMasks`); `masks.path` is set.
    Path(Masks),
}

/// One enabled mask's values as `applyMasks` reads them
/// (`Number(PM.evP(L, m.p[key], T, key))`) or `applyPathMasks` does
/// (`Number(PM.evP(L, m.p[key], T, 'm.<id>.<key>')) || 0`, and
/// `pathValues(PM, L, m.path, T, 'mp.<id>')`), with the evaluated shape and
/// mode.
fn mask_values<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    m: &'d Node,
    time: f64,
    with_path: bool,
) -> JsResult<MaskValues> {
    let p = node::member(Some(m), "p");
    let id = node::to_js_string(node::member(Some(m), "id")).to_string_lossy();
    let num = |scope: &mut Scope<'_, 'd>, key: &str| -> JsResult<f64> {
        if !with_path {
            return Ok(scope.channel_p(layer, node::member(p, key), time, key)?.to_number());
        }
        let v = scope
            .channel_p(layer, node::member(p, key), time, &format!("m.{id}.{key}"))?
            .to_number();
        Ok(pm_vector::js::or0(v))
    };
    let x = num(scope, "x")?;
    let y = num(scope, "y")?;
    let mw = num(scope, "w")?;
    let mh = num(scope, "h")?;
    let rotation = num(scope, "rotation")?;
    let feather = num(scope, "feather")?;
    let shape = scope.evaluated(
        layer,
        node::member(Some(m), "shape"),
        time,
        &format!("m.{id}.shape"),
    )?;
    let mode = scope.evaluated(
        layer,
        node::member(Some(m), "mode"),
        time,
        &format!("m.{id}.mode"),
    )?;
    let path = match node::member(Some(m), "path") {
        Some(path) if with_path && node::truthy(Some(path)) => Some(
            scope
                .path_values(layer, path, time, &format!("mp.{id}"))?
                .path_values(),
        ),
        _ => None,
    };
    Ok(MaskValues {
        x,
        y,
        w: mw,
        h: mh,
        rotation,
        feather,
        shape: if shape == JsValue::from("ellipse") {
            MaskShape::Ellipse
        } else {
            MaskShape::Rect
        },
        mode: if mode == JsValue::from("subtract") {
            MaskMode::Subtract
        } else {
            MaskMode::Add
        },
        path,
    })
}

/// `applyMasks(L, T, src, W, H)`'s reading of the masks. `world` is the
/// layer's `scaledWorld`; `plane` is its projected plane scaled to output
/// pixels when the layer is 3D (`is3DLayer`); `width × height` is the
/// output (the path route's coverage canvas).
#[allow(clippy::too_many_arguments)]
pub fn masks<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    world: [f64; 6],
    plane: Option<[f64; 9]>,
    width: u32,
    height: u32,
) -> JsResult<MaskRoute> {
    let list = node::array(node::member(Some(layer), "masks")).unwrap_or(&[]);
    let mut enabled: Vec<&'d Node> = Vec::new();
    for m in list {
        // `m && evaluatedValue(...) !== false && m.p`
        if !node::truthy(Some(m)) {
            continue;
        }
        if mask_enabled(scope, layer, m, time)? && node::truthy(node::member(Some(m), "p")) {
            enabled.push(m);
        }
    }
    if enabled.is_empty() {
        return Ok(MaskRoute::Unmasked);
    }
    if enabled
        .iter()
        .any(|m| node::truthy(node::member(Some(m), "path")))
    {
        // `applyPathMasks`: every enabled mask, in order, with no limit.
        let mut values = Vec::with_capacity(enabled.len());
        for m in &enabled {
            values.push(mask_values(scope, layer, m, time, true)?);
        }
        let LayerMasks::Path(path) = LayerMasks::new(&values) else {
            return Ok(MaskRoute::Unmasked);
        };
        // The coverage canvas is sampled at each output pixel; a 3D layer
        // maps it through its inverse plane composed with the world
        // matrix (`u_covTransform`, columns).
        let mut transform = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        if let Some(h) = plane
            && let Some(inv) = pm_eval::space3d::inverse_plane(&h)
        {
            for i in 0..3 {
                let (x, y, w) = (inv[i * 3], inv[i * 3 + 1], inv[i * 3 + 2]);
                transform[i * 3] = world[0] * x + world[2] * y + world[4] * w;
                transform[i * 3 + 1] = world[1] * x + world[3] * y + world[5] * w;
                transform[i * 3 + 2] = w;
            }
        }
        return Ok(MaskRoute::Path(Masks {
            inverse: transform,
            masks: pm_vector::ParametricMasks {
                masks: Vec::new(),
                has_add: false,
            },
            path: Some(super::path_masks::PathCoverage {
                width,
                height,
                world,
                masks: path,
            }),
        }));
    }
    let det = world[0] * world[3] - world[1] * world[2];
    if !det.is_finite() || det.abs() < 1e-9 {
        return Ok(MaskRoute::Unmasked);
    }
    let w = world;
    let inv = [
        w[3] / det,
        -w[1] / det,
        -w[2] / det,
        w[0] / det,
        (-w[3] * w[4] + w[2] * w[5]) / det,
        (w[1] * w[4] - w[0] * w[5]) / det,
    ];
    let mut values = Vec::new();
    for m in enabled.iter().take(super::MASK_MAX) {
        values.push(mask_values(scope, layer, m, time, false)?);
    }
    let LayerMasks::Parametric(parametric) = LayerMasks::new(&values) else {
        return Ok(MaskRoute::Unmasked);
    };
    let mut inverse = [
        inv[0], inv[1], 0.0, inv[2], inv[3], 0.0, inv[4], inv[5], 1.0,
    ];
    if let Some(h) = plane
        && let Some(projected) = pm_eval::space3d::inverse_plane(&h)
    {
        inverse = projected;
    }
    Ok(MaskRoute::Parametric(Masks {
        inverse,
        masks: parametric,
        path: None,
    }))
}

/// A track matte's mode: `evaluatedValue(L.matteMode, 'l.matteMode') || 'alpha'`,
/// read with `startsWith('luma')` and `endsWith('inverted')` (which throw
/// on a value that is not a string).
pub fn matte_mode<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
) -> JsResult<(bool, bool)> {
    let mode = scope.evaluated(
        layer,
        node::member(Some(layer), "matteMode"),
        time,
        "l.matteMode",
    )?;
    let mode = if mode.truthy() {
        mode
    } else {
        JsValue::from("alpha")
    };
    match mode {
        JsValue::String(s) => {
            let s = s.to_string_lossy();
            Ok((s.starts_with("luma"), s.ends_with("inverted")))
        }
        _ => Err(JsThrow::type_error("mode.startsWith is not a function")),
    }
}

/// `proj.layers.find(l => l.id === L.matteSource)`.
pub fn matte_source<'d>(layers: &'d [Node], layer: &Node) -> Option<&'d Node> {
    let id = node::get(layer, b"matteSource").to_js();
    layers
        .iter()
        .find(|l| pm_eval::js_sem::strict_eq(&node::get(l, b"id").to_js(), &id))
}
