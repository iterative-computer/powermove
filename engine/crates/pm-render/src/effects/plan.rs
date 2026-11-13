//! The effect pass and transition nodes of a layer at one frame (plan
//! section 8, "Render graph"), as `legacy/gl/compositor.ts` decides them:
//! `hasRenderableEffects`, `runEffects`' skip rules and `effectParamValue`,
//! `activeTransition`, `runTransition`'s parameter reads, and `setParam`'s
//! value conversion.

use pm_eval::node;
use pm_eval::{JsResult, JsValue, Scope};
use pm_model::{JsString, Node};
use pm_protocol::definitions::{DefinitionKind, DefinitionSnapshot, ParamSpec};

use super::{Effects, Status};
use crate::capability::Feature;
use crate::composite::Padding;

/// A parameter value as the compositor binds it (`setParam`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Binding {
    /// Numbers (`Number(value) || 0`) and toggles (`value ? 1 : 0`):
    /// `uniform1f`.
    Float(f64),
    /// Colors (`PM.hex2rgb(String(value))`): `uniform3fv`.
    Color([f64; 3]),
}

/// One parameter of a step: its key, whether it is a color (positional
/// alias `u_c<i>` rather than `u_p<i>`), and its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub key: String,
    pub color: bool,
    pub value: Binding,
}

/// One effect pass node: an effect instance's passes.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectStep {
    pub owner: Option<std::sync::Arc<Owner>>,
    pub source_domain: Option<(pm_fx_defs::spatial::Extent, pm_fx_defs::spatial::Extent)>,
    pub spatial: Option<Spatial>,
    pub temporal: bool,
    pub references: Vec<super::resources::Reference>,
    pub assets: Vec<(String, std::sync::Arc<super::resources::Asset>)>,
    /// `fx.id`.
    pub instance: String,
    /// `fx.type`, the definition id.
    pub definition: String,
    pub passes: u8,
    pub keep_orig: bool,
    pub backdrop: bool,
    /// Values above 1.0 pass through unless the definition sets `clamp`.
    pub clamp: bool,
    pub viewport_safe: bool,
    /// Pixels the effect samples outside a viewport: the sum of its
    /// `viewportPadding` parameter values.
    pub padding: f64,
    pub params: Vec<Param>,
    /// Output pixels per composition pixel where this step draws. A step
    /// without an owner binding sees composition pixels (`u_res`,
    /// `u_texel`, `v_px`), so its pixel-sized parameters draw at the size
    /// the 1:1 export draws them at any viewer zoom or playback scale.
    pub density: [f64; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatial {
    pub lod: u32,
    pub grid: Option<[u32; 2]>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Owner {
    pub valid: bool,
    pub uv: [f32; 9],
    pub size: [f64; 2],
    pub bounds: [f64; 4],
    pub pixel_scale: [f64; 2],
}

/// A transition node.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionStep {
    pub definition: String,
    /// `edge === 'in'`: from the stack below to the stack with the layer.
    pub edge_in: bool,
    pub progress: f64,
    pub params: Vec<Param>,
    /// Output pixels per composition pixel where it draws: a transition sees
    /// composition pixels as an effect does (`EffectStep::density`).
    pub density: [f64; 2],
}

/* ── setParam's value conversion ─────────────────────────────────────── */

fn js_whitespace(u: u16) -> bool {
    matches!(
        u,
        0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20 | 0xa0 | 0x1680 | 0x2000
            ..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff
    )
}

/// `parseInt(text, 16)`, NaN as `None`.
fn parse_int_hex(units: &[u16]) -> Option<f64> {
    let mut i = 0;
    while i < units.len() && js_whitespace(units[i]) {
        i += 1;
    }
    let mut sign = 1.0;
    if i < units.len() && (units[i] == u16::from(b'-') || units[i] == u16::from(b'+')) {
        if units[i] == u16::from(b'-') {
            sign = -1.0;
        }
        i += 1;
    }
    if i + 1 < units.len()
        && units[i] == u16::from(b'0')
        && (units[i + 1] == u16::from(b'x') || units[i + 1] == u16::from(b'X'))
    {
        i += 2;
    }
    let mut value = 0.0f64;
    let mut any = false;
    while i < units.len() {
        let d = match units[i] {
            u @ 0x30..=0x39 => u - 0x30,
            u @ 0x61..=0x66 => u - 0x61 + 10,
            u @ 0x41..=0x46 => u - 0x41 + 10,
            _ => break,
        };
        value = value * 16.0 + f64::from(d);
        any = true;
        i += 1;
    }
    any.then_some(sign * value)
}

/// `PM.hex2rgb(String(value))` (`legacy/core/util.ts`): the first `#`
/// removed, 3 or 4 digits doubled, channels read by position with
/// `parseInt(…, 16) || 0`, divided by 255.
pub fn hex2rgb(text: &JsString) -> [f64; 3] {
    let mut units: Vec<u16> = text.utf16_units().collect();
    if let Some(hash) = units.iter().position(|u| *u == u16::from(b'#')) {
        units.remove(hash);
    }
    if units.len() == 3 || units.len() == 4 {
        units = units.iter().flat_map(|u| [*u, *u]).collect();
    }
    let channel = |at: usize| {
        let end = (at + 2).min(units.len());
        let slice = if at < units.len() {
            &units[at..end]
        } else {
            &[][..]
        };
        let v = parse_int_hex(slice).unwrap_or(f64::NAN);
        (if v.is_nan() || v == 0.0 { 0.0 } else { v }) / 255.0
    };
    [channel(0), channel(2), channel(4)]
}

/// `setParam(p, pd, i, value)`'s value.
pub fn binding(spec: &ParamSpec, value: &JsValue) -> Binding {
    match spec {
        ParamSpec::Color { .. } => Binding::Color(hex2rgb(&value.to_js_string())),
        ParamSpec::Toggle { .. } => Binding::Float(if value.truthy() { 1.0 } else { 0.0 }),
        ParamSpec::Number { .. } => {
            let n = value.to_number();
            Binding::Float(if n.is_nan() || n == 0.0 { 0.0 } else { n })
        }
    }
}

/// A parameter's default as the value `effectParamValue` falls back to.
fn default_value(spec: &ParamSpec) -> JsValue {
    match spec {
        ParamSpec::Number { default, .. } => JsValue::Number(*default),
        ParamSpec::Color { default, .. } => JsValue::from(default.as_str()),
        ParamSpec::Toggle { default, .. } => JsValue::Bool(*default),
    }
}

fn param(spec: &ParamSpec, value: &JsValue) -> Param {
    Param {
        key: spec.key().to_owned(),
        color: matches!(spec, ParamSpec::Color { .. }),
        value: binding(spec, value),
    }
}

/// An effect pass node from a definition and the instance's parameter
/// values in definition order (`None`: the definition's default, as
/// `effectParamValue` falls back for a missing property or a nullish value).
pub fn effect_step(
    def: &DefinitionSnapshot,
    instance: &str,
    values: &[Option<JsValue>],
) -> EffectStep {
    let params: Vec<Param> = def
        .params
        .iter()
        .enumerate()
        .map(|(i, spec)| match values.get(i).cloned().flatten() {
            Some(v) => param(spec, &v),
            None => param(spec, &default_value(spec)),
        })
        .collect();
    let padding = def
        .flags
        .viewport_padding
        .iter()
        .filter_map(|k| params.iter().find(|p| &p.key == k))
        .map(|p| match p.value {
            Binding::Float(v) => v.abs(),
            Binding::Color(_) => 0.0,
        })
        .sum();
    EffectStep {
        owner: None,
        source_domain: None,
        spatial: None,
        temporal: false,
        references: vec![],
        assets: vec![],
        instance: instance.to_owned(),
        definition: def.id.as_str().to_owned(),
        passes: def.flags.passes,
        keep_orig: def.flags.keep_orig,
        backdrop: def.flags.backdrop,
        clamp: def.flags.clamp,
        viewport_safe: def.flags.viewport_safe,
        padding,
        params,
        density: [1., 1.],
    }
}

/// A transition node. `values` in definition order; `None` is the
/// definition's default (`prop ? PM.evP(…) : pd.def`: a property that
/// evaluates to null binds as null, not as the default).
pub fn transition_step(
    def: &DefinitionSnapshot,
    edge_in: bool,
    progress: f64,
    values: &[Option<JsValue>],
) -> TransitionStep {
    TransitionStep {
        definition: def.id.as_str().to_owned(),
        edge_in,
        progress,
        params: def
            .params
            .iter()
            .enumerate()
            .map(|(i, spec)| match values.get(i).cloned().flatten() {
                Some(v) => param(spec, &v),
                None => param(spec, &default_value(spec)),
            })
            .collect(),
        density: [1., 1.],
    }
}

/* ── the layer's staging ─────────────────────────────────────────────── */

/// Whether a transition type has a registration (`PM.transitionDef`), for
/// `activeTransition` and the group boundary rule. An id the engine knows
/// nothing about (no snapshot, not marked missing) counts as registered so
/// the frame reaches [`staging`], which refuses it instead of guessing.
pub fn transition_known(effects: &Effects, ty: &JsString) -> bool {
    let id = ty.to_string_lossy();
    effects.get(DefinitionKind::Transition, &id).is_some()
        || !effects.is_missing(DefinitionKind::Transition, &id)
}

/// `runEffects`' effect pass nodes for `layer` at `time`: every enabled
/// effect that is not marked missing, whose definition is registered and
/// draws, and that the watchdog has not turned off, in order. `Err` names
/// what the engine cannot draw (an unknown definition, one that needs the
/// fallback runner).
pub fn effect_steps<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &Effects,
    layer: &'d Node,
    time: f64,
) -> JsResult<Result<Vec<EffectStep>, Feature>> {
    effect_steps_scaled(scope, effects, layer, time, (1., 1.))
}

pub fn effect_steps_scaled<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &Effects,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
) -> JsResult<Result<Vec<EffectStep>, Feature>> {
    effect_steps_prefix(scope, effects, layer, time, scale, None)
}
pub(crate) fn effect_steps_prefix<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &Effects,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    prefix: Option<usize>,
) -> JsResult<Result<Vec<EffectStep>, Feature>> {
    effect_steps_raster(scope, effects, layer, time, scale, prefix, None, None)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn effect_steps_raster<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &Effects,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    prefix: Option<usize>,
    raster: Option<crate::composite::effects::SourceRaster>,
    mut measure: Option<&mut dyn pm_eval::text_geom::TextMeasure>,
) -> JsResult<Result<Vec<EffectStep>, Feature>> {
    let mut steps = Vec::new();
    let mut refused = None;
    let composition = scope.comp.id().to_string_lossy();
    for (fx_index, fx) in node::array_or_empty(node::member(Some(layer), "fx"))?
        .iter()
        .enumerate()
        .take(prefix.unwrap_or(usize::MAX))
    {
        let fx_id = node::to_js_string(node::member(Some(fx), "id")).to_string_lossy();
        let on = node::member_checked(Some(fx), "on")?;
        if !scope
            .evaluated(layer, on, time, &format!("{fx_id}.$enabled"))?
            .truthy()
            || node::truthy(node::member(Some(fx), "missing"))
        {
            continue;
        }
        let ty = node::to_js_string(node::member(Some(fx), "type")).to_string_lossy();
        let mut refuse = |feature: Feature| {
            if effects.bypass_missing {
                effects.report_bypassed(composition.clone(), layer, &ty, &feature);
            } else {
                refused.get_or_insert(feature);
            }
        };
        // No provider is installed: the effect is skipped with its data kept,
        // when the kernel marked it missing or for viewer frames (After
        // Effects' missing-effect bypass). Exports refuse instead.
        if effects.get(DefinitionKind::Effect, &ty).is_none()
            && (effects.bypass_missing || effects.is_missing(DefinitionKind::Effect, &ty))
        {
            continue;
        }
        let ty = match effects.saved_key(&ty, node::member(Some(fx), "version")) {
            Ok(key) => key,
            // A saved revision this build has no definition for (a newer
            // app saved it): bypassed like a missing provider in viewer
            // frames and named by `missing_effects`; exports refuse.
            Err(_) if effects.bypass_missing => continue,
            Err(message) => {
                refuse(Feature::Definition(message));
                continue;
            }
        };
        let Some(def) = effects.get(DefinitionKind::Effect, &ty) else {
            if !effects.is_missing(DefinitionKind::Effect, &ty) {
                refuse(Feature::Definition(format!("effect {ty}")));
            }
            continue;
        };
        match effects.status(DefinitionKind::Effect, &ty) {
            Some(Status::Ready) => {}
            // Drawn by the fallback runner when one is attached
            // (`super::fallback`), else routed.
            Some(Status::NeedsFallback(_)) if effects.fallback.runner => {}
            Some(Status::NeedsFallback(_)) => {
                refuse(Feature::ShaderFallback(ty.clone()));
                continue;
            }
            // Refused programs render nothing today: skipped.
            Some(Status::Refused(failure)) => {
                if effects.bypass_missing {
                    refuse(Feature::Definition(format!("effect {ty}: {failure}")));
                }
                continue;
            }
            None => continue,
        }
        if effects.watchdog.is_disabled(DefinitionKind::Effect, &ty) {
            continue;
        }
        // `effectParamValue`: no property (falsy) or a nullish value falls
        // back to the definition's default.
        let p = node::member(Some(fx), "p");
        let mut values = Vec::with_capacity(def.params.len());
        for spec in &def.params {
            let value = match node::member(p, spec.key()).filter(|n| node::truthy(Some(n))) {
                None => None,
                Some(prop) => Some(scope.channel_p(layer, Some(prop), time, spec.key())?),
            };
            values.push(value.filter(|v| !matches!(v, JsValue::Undefined | JsValue::Null)));
        }
        let mut step = effect_step(def, &fx_id, &values);
        step.density = [scale.0.abs(), scale.1.abs()];
        if raster.is_some() && super::versions::main_definition(&step.definition).is_none() {
            refuse(Feature::Definition(
                "Effect has no source processing contract".into(),
            ));
            continue;
        }
        if effects.versioned_builtins
            && def.glsl.as_ref().is_some_and(|g| {
                g.source.contains("u_layerUV") || g.source.contains("u_temporalOwnerUV")
            })
        {
            match raster.map_or_else(
                || owner(scope, layer, time, scale, &mut measure),
                |r| Ok(Ok(r.owner())),
            )? {
                Ok(owner) => step.owner = Some(std::sync::Arc::new(owner)),
                Err(f) => {
                    refuse(f);
                    continue;
                }
            }
        }
        if let Some(definition) = super::versions::main_definition(&step.definition) {
            let inputs = node::member(Some(fx), "inputs")
                .map(|n| {
                    serde_json::from_str::<serde_json::Value>(&pm_model::json::to_canonical_string(
                        &n.to_value(),
                    ))
                })
                .transpose();
            let inputs = match inputs {
                Ok(Some(v)) => pm_fx_defs::inputs::validate_inputs(&v).map_err(|e| e.to_string()),
                Ok(None) => Ok(definition.input_defaults()),
                Err(e) => Err(e.to_string()),
            };
            let inputs = match inputs {
                Ok(v) => v,
                Err(e) => {
                    refuse(Feature::Definition(e));
                    continue;
                }
            };
            let mut failed = false;
            for param in definition.params.iter().filter(|p| p.is_reference()) {
                if let Some(input) = inputs.get(&param.k) {
                    let valid = matches!(
                        (param.kind.as_deref(), input),
                        (Some("layer"), pm_fx_defs::inputs::EffectInput::Layer(_))
                            | (Some("path"), pm_fx_defs::inputs::EffectInput::Path(_))
                            | (Some("asset"), pm_fx_defs::inputs::EffectInput::Asset(_))
                    );
                    if !valid {
                        refuse(Feature::Definition(format!(
                            "Effect reference kind does not match {}",
                            param.k
                        )));
                        failed = true;
                        continue;
                    }
                }
                match inputs.get(&param.k) {
                    Some(pm_fx_defs::inputs::EffectInput::Asset(a)) => {
                        if let Some(id) = &a.asset_id {
                            let lut = param.presentation.get("assetKind").and_then(|v| v.as_str())
                                == Some("lut");
                            match effects.assets.prepared.get(&(id.clone(), lut)) {
                                Some(Ok(asset)) => {
                                    step.assets.push((param.k.clone(), asset.clone()))
                                }
                                Some(Err(e)) => {
                                    refuse(Feature::EffectAsset(e.clone()));
                                    failed = true;
                                }
                                None => {
                                    refuse(Feature::Definition(format!(
                                        "Effect asset not prepared: {id}"
                                    )));
                                    failed = true;
                                }
                            }
                        }
                    }
                    Some(pm_fx_defs::inputs::EffectInput::Layer(l)) if l.layer_id.is_some() => {
                        let request = pm_fx_defs::inputs::Request::new(
                            l.layer_id.as_ref().unwrap(),
                            l.stage,
                            l.space.unwrap_or_default(),
                            Some(time),
                            None,
                        )
                        .unwrap();
                        step.references.push(super::resources::Reference {
                            effect: step.instance.clone(),
                            name: param.k.clone(),
                            request,
                        });
                    }
                    Some(pm_fx_defs::inputs::EffectInput::Path(p)) if p.layer_id.is_some() => {
                        let provider = scope
                            .layers()
                            .iter()
                            .find(|l| crate::js::layer_id(l) == *p.layer_id.as_ref().unwrap());
                        if let Some(provider) = provider {
                            let size = scope.comp_size();
                            match super::brush::raster(
                                scope,
                                provider,
                                layer,
                                p.path_id.as_ref().unwrap(),
                                time,
                                &step,
                                (
                                    (size.0 * scale.0.abs()).max(1.) as u32,
                                    (size.1 * scale.1.abs()).max(1.) as u32,
                                ),
                                scale,
                            )? {
                                Ok(asset) => step.assets.push((param.k.clone(), asset)),
                                Err(e) => {
                                    refuse(Feature::Definition(e));
                                    failed = true;
                                }
                            }
                        } else {
                            refuse(Feature::Definition("Missing Stroke path provider".into()));
                            failed = true;
                        }
                    }
                    _ => {}
                }
            }
            if failed {
                continue;
            }
        }
        if let Some(definition) = super::versions::main_definition(&step.definition) {
            let value = |key: &str| {
                step.params
                    .iter()
                    .find(|p| p.key == key)
                    .map_or(0., |p| match p.value {
                        Binding::Float(v) => v,
                        _ => 0.,
                    })
            };
            let size = scope.comp_size();
            let w = (size.0 * scale.0.abs()).max(1.) as u32;
            let h = (size.1 * scale.1.abs()).max(1.) as u32;
            let sampling = definition.metadata.get("sampling");
            let grid = definition.metadata.get("grid");
            let layer_kernel = sampling
                .and_then(|s| s.get("space"))
                .and_then(|s| s.as_str())
                == Some("layer");
            let kernel_scale = if layer_kernel {
                step.owner.as_ref().map_or(step.density, |o| o.pixel_scale)
            } else {
                step.density
            };
            let space_scale = if layer_kernel {
                kernel_scale[0].hypot(kernel_scale[1])
            } else {
                scale.0.abs().max(scale.1.abs())
            };
            // Main's viewer adds fixed support and scales authored padding by
            // its declared factor and the actual composition/layer density.
            step.padding = (step.padding
                * definition
                    .metadata
                    .get("viewportPaddingFactor")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.)
                + definition
                    .metadata
                    .get("viewportPaddingPixels")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.))
                * space_scale;
            if sampling.is_some() || grid.is_some() {
                let lod = if let Some(sampling) = sampling {
                    pm_fx_defs::spatial::sample_lod(
                        value(sampling["radius"].as_str().unwrap()),
                        sampling["support"].as_f64().unwrap(),
                        sampling["maxRadius"].as_f64().unwrap(),
                        kernel_scale,
                        w,
                        h,
                    )
                    .unwrap()
                } else {
                    w.max(h).ilog2()
                };
                step.spatial = Some(Spatial {
                    lod,
                    grid: grid.map(|g| {
                        pm_fx_defs::spatial::grid_size(
                            value(g["columns"].as_str().unwrap()),
                            value(g["rows"].as_str().unwrap()),
                            2048,
                        )
                        .unwrap()
                    }),
                });
            }
        }
        if let Some(definition) = super::versions::main_definition(&step.definition)
            && let Some(temporal) = definition.metadata.get("temporal")
        {
            let value = |key: &str| {
                step.params
                    .iter()
                    .find(|p| p.key == key)
                    .map_or(0., |p| match p.value {
                        Binding::Float(v) => v,
                        _ => 0.,
                    })
            };
            let times = pm_fx_defs::inputs::temporal_times(
                value(temporal["count"].as_str().unwrap()),
                value(temporal["step"].as_str().unwrap()),
                time,
                temporal["maxSamples"].as_u64().unwrap() as u32,
            );
            let times = match times {
                Ok(t) => t,
                Err(e) => {
                    refuse(Feature::Definition(e.to_string()));
                    continue;
                }
            };
            step.temporal = true;
            for at in std::iter::once(time).chain(times) {
                let request = pm_fx_defs::inputs::Request::new(
                    crate::js::layer_id(layer),
                    pm_fx_defs::inputs::Stage::Effects,
                    pm_fx_defs::inputs::Space::Composition,
                    Some(at),
                    Some(fx_index),
                )
                .unwrap();
                step.references.push(super::resources::Reference {
                    effect: step.instance.clone(),
                    name: "temporal".into(),
                    request,
                });
            }
        }
        // Main definitions scale their padding by the density above; any
        // other step reaches its authored padding in composition pixels.
        if super::versions::main_definition(&step.definition).is_none() && step.owner.is_none() {
            step.padding *= step.density[0].max(step.density[1]);
        }
        steps.push(step);
    }
    Ok(match refused {
        Some(f) => Err(f),
        None => Ok(steps),
    })
}

/// The transition node of `layer` at `time` (`activeTransition`,
/// `runTransition`'s parameter reads): `Ok(None)` when none is active or
/// its program does not draw (today the layer then composites without it).
pub fn transition_at<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &Effects,
    layer: &'d Node,
    time: f64,
) -> JsResult<Result<Option<TransitionStep>, Feature>> {
    let known = |ty: &JsString| transition_known(effects, ty);
    let Some(active) = scope.active_transition(layer, time, &known)? else {
        return Ok(Ok(None));
    };
    let ty = node::to_js_string(node::member(Some(active.transition), "type")).to_string_lossy();
    let Some(def) = effects.get(DefinitionKind::Transition, &ty) else {
        return Ok(Err(Feature::Definition(format!("transition {ty}"))));
    };
    match effects.status(DefinitionKind::Transition, &ty) {
        Some(Status::NeedsFallback(_)) if !effects.fallback.runner => {
            return Ok(Err(Feature::ShaderFallback(ty)));
        }
        Some(Status::Ready | Status::NeedsFallback(_))
            if !effects
                .watchdog
                .is_disabled(DefinitionKind::Transition, &ty) => {}
        // A program that does not link: `runTransition` returns null and
        // the layer composites without the transition.
        _ => return Ok(Ok(None)),
    }
    // `runTransition`: `prop ? PM.evP(L, prop, T, pd.k) : pd.def`.
    let p = node::member(Some(active.transition), "p");
    let mut values = Vec::with_capacity(def.params.len());
    for spec in &def.params {
        values.push(
            match node::member(p, spec.key()).filter(|n| node::truthy(Some(n))) {
                None => None,
                Some(prop) => Some(scope.channel_p(layer, Some(prop), time, spec.key())?),
            },
        );
    }
    Ok(Ok(Some(transition_step(
        def,
        active.edge_in,
        active.progress,
        &values,
    ))))
}

/// How far a chain reads beyond each output pixel: a viewport-safe effect
/// its `viewportPadding` values, any other the whole frame; chained reads
/// add up.
pub fn chain_padding(steps: &[EffectStep]) -> Padding {
    padding_with(steps, 0)
}

/// Beyond its declared spread, each pass of a viewport-safe effect may read
/// one texel of a grid down to 1/4 resolution: a bilinear tap, an area
/// downsample or an upsample of an intermediate target. A declared spread
/// is the kernel's reach; these footprints add up over the passes.
const PASS_SUPPORT: u32 = 4;

/// The padding a region's pass renders a chain over so the region equals
/// the whole frame: [`chain_padding`] plus each pass's sampling support.
pub fn region_padding(steps: &[EffectStep]) -> Padding {
    padding_with(steps, PASS_SUPPORT)
}

fn padding_with(steps: &[EffectStep], support: u32) -> Padding {
    steps.iter().fold(Padding::NONE, |acc, step| {
        acc.then(if step.viewport_safe {
            let spread = step.padding.ceil().clamp(0.0, f64::from(u32::MAX / 4)) as u32;
            Padding::Pixels(spread.saturating_add(support * u32::from(step.passes.max(1))))
        } else {
            Padding::Full
        })
    })
}

/// Electron's `effectLayerCoordinates`: GL.bounds, inverse projected plane,
/// then the composition fallback (valid only for adjustment layers).
pub(crate) fn owner<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    measure: &mut Option<&mut dyn pm_eval::text_geom::TextMeasure>,
) -> JsResult<Result<Owner, Feature>> {
    let bounds = scope.bounds_with_measure(layer, time, measure, &mut pm_eval::bounds::NoHost)?;
    let world = scope.world_matrix(layer, time)?;
    let plane = scope.plane_matrix(layer, time, false)?;
    let mut scaled = plane;
    for i in 0..3 {
        scaled[i * 3] *= scale.0;
        scaled[i * 3 + 1] *= scale.1;
    }
    if let (Some(b), Some(inv)) = (bounds, pm_eval::space3d::inverse_plane(&scaled))
        && b.w > 0.
        && b.h > 0.
    {
        let mut uv = [0.; 9];
        for i in 0..3 {
            let [x, y, w] = [inv[i * 3], inv[i * 3 + 1], inv[i * 3 + 2]];
            uv[i * 3] = ((x - b.x0 * w) / b.w) as f32;
            uv[i * 3 + 1] = ((y - b.y0 * w) / b.h) as f32;
            uv[i * 3 + 2] = w as f32;
        }
        let owner = Owner {
            valid: true,
            uv,
            size: [b.w, b.h],
            bounds: [b.x0, b.y0, b.w, b.h],
            pixel_scale: pm_fx_defs::spatial::kernel_scale(Some(world), [scale.0, scale.1]),
        };
        if owner
            .uv
            .iter()
            .chain(&super::resources::from_uv(&owner))
            .all(|v| v.is_finite())
        {
            return Ok(Ok(owner));
        }
    }
    let (w, h) = scope.comp_size();
    let size = [(w * scale.0).max(1.), (h * scale.1).max(1.)];
    Ok(Ok(Owner {
        valid: node::is_str(node::member(Some(layer), "type"), "adjustment"),
        uv: [
            (1. / size[0]) as f32,
            0.,
            0.,
            0.,
            (1. / size[1]) as f32,
            0.,
            0.,
            0.,
            1.,
        ],
        size,
        bounds: [0., 0., size[0], size[1]],
        pixel_scale: pm_fx_defs::spatial::kernel_scale(Some(world), [scale.0, scale.1]),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_group_and_adjustment_use_electrons_composition_fallback() {
        let root = pm_model::json::parse(
            &serde_json::json!({
                "id":"p","compId":"c","w":320,"h":180,"fps":30,"dur":1,"comps":{},
                "layers":[fixture_layer("group","group",serde_json::json!({}),"blur",0.,0.),
                    fixture_layer("grade","adjustment",serde_json::json!({}),"blur",0.,0.)]
            })
            .to_string(),
        )
        .unwrap();
        let doc = pm_model::Document::new(Some(pm_model::canonical::canonical_project(root)), 1);
        let comp = pm_eval::Comp::resolve(&doc, "c").unwrap();
        let mut ev = pm_eval::Evaluator::new();
        let mut scope = Scope::new(&mut ev, &comp, &comp);
        for layer in scope.layers() {
            let owner = owner(&mut scope, layer, 0., (0.5, 0.5), &mut None)
                .unwrap()
                .unwrap();
            assert_eq!(owner.size, [160., 90.]);
            assert_eq!(owner.bounds, [0., 0., 160., 90.]);
            assert_eq!(owner.valid, crate::js::layer_type(layer) == "adjustment");
            assert_eq!(owner.uv, [1. / 160., 0., 0., 0., 1. / 90., 0., 0., 0., 1.]);
        }
    }

    fn fixture_layer(
        id: &str,
        ty: &str,
        d: serde_json::Value,
        effect: &str,
        x: f64,
        y: f64,
    ) -> serde_json::Value {
        let mut p = serde_json::Map::new();
        for (key, value) in [
            ("position.x", x),
            ("position.y", y),
            ("anchor.x", 0.),
            ("anchor.y", 0.),
            ("scale.x", 100.),
            ("scale.y", 100.),
            ("rotation", 0.),
            ("opacity", 100.),
        ] {
            p.insert(
                key.into(),
                serde_json::json!({"v":value,"kf":[],"expr":null}),
            );
        }
        serde_json::json!({"id":id,"type":ty,"on":true,"from":0,"dur":2,"p":p,"d":d,
            "fx":[{"id":"f","type":effect,"version":2,"on":true,"p":{}}]})
    }

    struct MeasuredText;
    impl pm_eval::text_geom::TextMeasure for MeasuredText {
        fn measure_lines(
            &mut self,
            _: &pm_text::compat::TextContent,
        ) -> Vec<pm_eval::text_geom::MeasuredLine> {
            vec![pm_eval::text_geom::MeasuredLine {
                text: "Test".into(),
                width: 40.,
                ink: [-2., -15., 39., 4.],
                anchor_ink: [-2., -15., 39., 4.],
            }]
        }
        fn plain_width(&mut self, _: &pm_text::compat::TextContent, _: &str) -> f64 {
            40.
        }
        fn spaced_width(&mut self, _: &pm_text::compat::TextContent, _: &str) -> f64 {
            40.
        }
        fn glyph_layout(
            &mut self,
            _: &pm_text::compat::TextContent,
        ) -> pm_text::compat::GlyphLayout {
            unreachable!("no animators")
        }
    }

    #[test]
    fn owner_reading_effects_use_electron_text_group_adjustment_and_path_rects() {
        use serde_json::json;
        let mut group = fixture_layer("group", "group", json!({}), "blur", 100., 50.);
        group["fx"][0]["p"] = json!({"amount":{"v":8}});
        let mut child = fixture_layer("child", "solid", json!({"w":80,"h":40}), "blur", 10., 20.);
        child["group"] = json!("group");
        child["fx"] = json!([]);
        let mut other = child.clone();
        other["id"] = json!("other");
        other["d"] = json!({"w":10,"h":20});
        other["p"]["position.x"]["v"] = json!(-20);
        other["p"]["position.y"]["v"] = json!(60);
        let mut hidden = child.clone();
        hidden["id"] = json!("hidden");
        hidden["on"] = json!(false);
        hidden["d"] = json!({"w":999,"h":999});
        let path = json!({"id":"rect","type":"path","p":{"x":0,"y":0,"rotation":0,"scaleX":100,"scaleY":100,"closed":true,"fillEnabled":true,"fillOpacity":100,"strokeWidth":4,"strokeOpacity":100,"trimStart":0,"trimEnd":100,"trimOffset":0,"copies":1,"repeatX":0,"repeatY":0,"repeatRotation":0},"vertices":[
            {"id":"a","p":{"x":10,"y":20}},{"id":"b","p":{"x":50,"y":20}},{"id":"c","p":{"x":50,"y":60}},{"id":"d","p":{"x":10,"y":60}}]});
        let layers = vec![
            fixture_layer(
                "text",
                "text",
                json!({"text":"Test","size":20,"leading":1,"align":"left"}),
                "shadow",
                100.,
                50.,
            ),
            group,
            child,
            other,
            hidden,
            fixture_layer("adjustment", "adjustment", json!({}), "blur", 999., 999.),
            fixture_layer("path", "shape", json!({"paths":[path]}), "blur", 100., 50.),
        ];
        let root=pm_model::json::parse(&json!({"id":"p","compId":"c","w":320,"h":180,"fps":30,"dur":2,"layers":layers,"comps":{}}).to_string()).unwrap();
        let doc = pm_model::Document::new(Some(root), 1);
        let comp = pm_eval::Comp::resolve(&doc, "c").unwrap();
        let mut ev = pm_eval::Evaluator::new();
        let mut scope = Scope::new(&mut ev, &comp, &comp);
        let mut effects = Effects::new("owner-unit");
        let mut defs = super::super::versions::builtin_snapshot();
        defs.definitions.retain(|d| {
            [
                "shadow",
                "blur",
                "pm.history.main.shadow.2",
                "pm.history.main.blur.2",
            ]
            .contains(&d.id.as_str())
        });
        assert!(effects.set_snapshot(&defs).unwrap().is_empty());
        effects.versioned_builtins = true;
        // Electron GL.bounds: text ink + selection margin (3 px here),
        // active group member union, composition fallback, path vertices
        // padded by strokeWidth + 4. These are deliberately not sizeBounds.
        for (id, expected) in [
            ("text", [-5., -1.6, 47., 25.]),
            ("group", [-20., 20., 110., 60.]),
            ("adjustment", [0., 0., 160., 90.]),
            ("path", [2., 12., 56., 56.]),
        ] {
            let layer = scope
                .layers()
                .iter()
                .find(|l| crate::js::layer_id(l) == id)
                .unwrap();
            let steps = effect_steps_raster(
                &mut scope,
                &effects,
                layer,
                0.,
                (0.5, 0.5),
                None,
                None,
                Some(&mut MeasuredText),
            )
            .unwrap()
            .unwrap();
            assert_eq!(steps.len(), 1, "{id}");
            let owner = steps[0].owner.as_ref().unwrap();
            assert!(owner.valid, "{id}");
            for (a, b) in owner.bounds.into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-9, "{id}: {:?}", owner.bounds);
            }
            // A known composition point maps to its authored local fraction.
            if id != "adjustment" {
                let q = pm_eval::space3d::project_point(
                    &owner.uv.map(f64::from),
                    50. + expected[0] * 0.5,
                    25. + expected[1] * 0.5,
                );
                assert!(q.0.abs() < 1e-6 && q.1.abs() < 1e-6, "{id}: {q:?}");
            }
        }
    }

    #[test]
    fn projected_owner_and_size_anchor_follow_electrons_gl_bounds() {
        use serde_json::json;
        let mut layer = fixture_layer(
            "plane",
            "shape",
            json!({"w":80,"h":40,"stroke":6,"sizeAnchorBounds":{"x0":-80,"y0":-40,"x1":80,"y1":40}}),
            "blur",
            100.,
            50.,
        );
        layer["threeD"] = json!(true);
        layer["p"]["rotation.y"] = json!({"v":35});
        let root=pm_model::json::parse(&json!({"id":"p","compId":"c","w":320,"h":180,"fps":30,"dur":2,"layers":[layer],"comps":{}}).to_string()).unwrap();
        let doc = pm_model::Document::new(Some(root), 1);
        let comp = pm_eval::Comp::resolve(&doc, "c").unwrap();
        let mut ev = pm_eval::Evaluator::new();
        let mut scope = Scope::new(&mut ev, &comp, &comp);
        let layer = &scope.layers()[0];
        let owner = owner(&mut scope, layer, 0., (0.5, 0.5), &mut None)
            .unwrap()
            .unwrap();
        assert!(owner.valid);
        assert!(
            owner.uv[2].abs() + owner.uv[5].abs() > 0.,
            "perspective must survive"
        );
        let bounds = scope
            .bounds_with_measure(layer, 0., &mut None, &mut pm_eval::bounds::NoHost)
            .unwrap()
            .unwrap();
        let plane = scope.plane_matrix(layer, 0., false).unwrap();
        for (u, v) in [(0., 0.), (0.5, 0.5), (1., 1.)] {
            let point = pm_eval::space3d::project_point(
                &plane,
                bounds.x0 + u * bounds.w,
                bounds.y0 + v * bounds.h,
            );
            let actual = pm_eval::space3d::project_point(
                &owner.uv.map(f64::from),
                point.0 * 0.5,
                point.1 * 0.5,
            );
            assert!(
                (actual.0 - u).abs() < 1e-6 && (actual.1 - v).abs() < 1e-6,
                "{actual:?}"
            );
        }
    }

    fn rgb(s: &str) -> [f64; 3] {
        hex2rgb(&JsString::from(s))
    }

    #[test]
    fn hex2rgb_reads_channels_like_the_legacy_parser() {
        assert_eq!(rgb("#FF0080"), [1.0, 0.0, 128.0 / 255.0]);
        assert_eq!(rgb("#f08"), [1.0, 0.0, 136.0 / 255.0]);
        assert_eq!(rgb("#f08c"), [1.0, 0.0, 136.0 / 255.0], "alpha ignored");
        assert_eq!(rgb("#11223344"), [17.0 / 255.0, 34.0 / 255.0, 51.0 / 255.0]);
        assert_eq!(
            rgb("zz8000"),
            [0.0, 128.0 / 255.0, 0.0],
            "bad channels are 0"
        );
        assert_eq!(rgb("null"), [0.0, 0.0, 0.0]);
        assert_eq!(rgb("#1"), [1.0 / 255.0, 0.0, 0.0]);
        assert_eq!(
            rgb("-1ff00"),
            [-1.0 / 255.0, 1.0, 0.0],
            "parseInt takes a sign"
        );
        assert_eq!(
            rgb("1g2233"),
            [1.0 / 255.0, 34.0 / 255.0, 51.0 / 255.0],
            "stops at a non-digit"
        );
    }

    #[test]
    fn set_param_converts_values() {
        let number = ParamSpec::Number {
            key: "a".into(),
            default: 3.0,
            min: 0.0,
            max: 9.0,
            step: None,
        };
        let toggle = ParamSpec::Toggle {
            key: "t".into(),
            default: false,
        };
        assert_eq!(
            binding(&number, &JsValue::from("12.5")),
            Binding::Float(12.5)
        );
        assert_eq!(
            binding(&number, &JsValue::from("nope")),
            Binding::Float(0.0)
        );
        assert_eq!(
            binding(&number, &JsValue::Number(-0.0)),
            Binding::Float(0.0)
        );
        assert_eq!(binding(&number, &JsValue::Bool(true)), Binding::Float(1.0));
        assert_eq!(binding(&toggle, &JsValue::from("")), Binding::Float(0.0));
        assert_eq!(binding(&toggle, &JsValue::Number(2.0)), Binding::Float(1.0));
    }
}
