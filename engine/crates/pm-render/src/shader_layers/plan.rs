//! What a shader layer or an extension fragment layer draws at one frame:
//! its program, its target size, and the compositor's uniform calls in issue
//! order (`compositor.ts` `nativeContentQuad`, the `shader` and `extension`
//! branches).

use std::sync::Arc;

use pm_eval::node;
use pm_eval::{Content, JsResult, JsValue, Scope};
use pm_glsl::Key;
use pm_model::{JsString, Node};
use pm_protocol::definitions::{DefinitionKind, LayerRenderer, ParamSpec};

use super::fallback::FallbackRequest;
use super::translate::{self, Translated, Verdict};
use super::uniforms::{Control, parse_uniforms};
use crate::capability::Feature;
use crate::effects::program::Call;
use crate::effects::{Effects, Status};
use crate::js;

/// The program a shader source runs.
#[derive(Debug, Clone)]
pub enum ProgramRef {
    /// Shader-layer code or the missing-layer checkerboard, translated by
    /// `pm_glsl` (cached by source). `source` is the wrapped program, for a
    /// fallback request if the device refuses the translation.
    Code {
        key: Key,
        prepared: Translated,
        source: Arc<str>,
    },
    /// An extension fragment layer's program in the renderer's definitions.
    Definition { id: String, key: Option<Key> },
    /// A program only the decision 19 fallback runner can draw.
    Fallback(Arc<FallbackRequest>),
}

/// A shader layer's or extension fragment layer's content at one frame.
#[derive(Debug, Clone)]
pub struct ShaderSource {
    pub layer: String,
    pub program: ProgramRef,
    /// The target (`grab(targetW, targetH)`), in output pixels.
    pub width: u32,
    pub height: u32,
    /// `setU`/`setI` calls in issue order (a later call overwrites).
    pub calls: Vec<(String, Call)>,
}

impl ShaderSource {
    /// Identifies the program and every input of its pixels (plan equality
    /// and render cache keys, section 8).
    pub fn content_key(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"pm-render shader source 1\0");
        match &self.program {
            ProgramRef::Code { key, .. } => {
                h.update([0]);
                h.update(key.0);
            }
            ProgramRef::Definition { id, key } => {
                h.update([1]);
                h.update(id.as_bytes());
                h.update([0]);
                if let Some(k) = key {
                    h.update(k.0);
                }
            }
            ProgramRef::Fallback(r) => {
                h.update([2]);
                h.update(r.key.as_bytes());
            }
        }
        h.update(self.width.to_le_bytes());
        h.update(self.height.to_le_bytes());
        h.update(calls_bytes(&self.calls));
        h.finalize().into()
    }
}

impl PartialEq for ShaderSource {
    fn eq(&self, other: &Self) -> bool {
        self.layer == other.layer && self.content_key() == other.content_key()
    }
}

/// The calls as bytes (content keys, fallback keys).
pub(crate) fn calls_bytes(calls: &[(String, Call)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, call) in calls {
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        match call {
            Call::F(v) => {
                out.push(b'f');
                out.push(v.len() as u8);
                v.iter()
                    .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
            }
            Call::I(i) => {
                out.push(b'i');
                out.extend_from_slice(&i.to_le_bytes());
            }
            Call::M3(m) => {
                out.push(b'm');
                m.iter()
                    .for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
            }
        }
    }
    out
}

/// What the plan builder does with a shader or extension fragment layer.
#[derive(Debug)]
pub(crate) enum Planned {
    /// Drawn as a `w` × `h` quad (composition pixels) through the layer's
    /// transform, anchored at the origin, sampled as a `fromFbo` target.
    Draw {
        source: Box<ShaderSource>,
        w: f64,
        h: f64,
    },
    /// The program WebGL refuses too: today's `program()` returns null and
    /// the layer draws nothing; the diagnostics are the layer's failure.
    Refused(String),
    /// Something this renderer cannot draw (routes the composition).
    Unsupported(Feature),
}

/// WebIDL's `long` conversion of a JavaScript number (`uniform1i`).
fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let t = x.trunc();
    let m = t.rem_euclid(4_294_967_296.0);
    (if m >= 2_147_483_648.0 {
        m - 4_294_967_296.0
    } else {
        m
    }) as i32
}

fn f(values: &[f64]) -> Call {
    Call::F(values.iter().map(|v| *v as f32).collect())
}

/// The target size: `w = max(1, Number(d.w || comp.w || W))` and
/// `targetW = min(4096, max(2, round(w · sx)))`, both axes. A `compat`
/// level 2 layer renders at the density it is drawn at (never below the
/// output scale) without today's 4,096 px cap, up to the level 2 limit, and
/// is drawn by the level 2 resampler (section 10: level 1 keeps the cap).
fn size<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    d: &Content,
    output: (u32, u32),
    scale: (f64, f64),
    time: f64,
) -> JsResult<(f64, f64, u32, u32)> {
    let (cw, ch) = scope.comp_size();
    let (w, tw) = crate::scene3d::legacy::target_size(
        js::get(d, "w").to_number(),
        cw,
        f64::from(output.0),
        scale.0,
    );
    let (h, th) = crate::scene3d::legacy::target_size(
        js::get(d, "h").to_number(),
        ch,
        f64::from(output.1),
        scale.1,
    );
    if crate::sampling::is_level2(layer) {
        let (dx, dy) = crate::sampling::precomp_density(scope, layer, time, scale, w, h)?;
        let side = |size: f64, s: f64, d: f64| {
            js::min(
                crate::sampling::MAX_TARGET,
                js::max(2.0, js::round(size * js::max(s, d))),
            )
        };
        return Ok((
            w,
            h,
            side(w, scale.0, dx) as u32,
            side(h, scale.1, dy) as u32,
        ));
    }
    Ok((w, h, tw as u32, th as u32))
}

/// The calls both layer kinds make before their own uniforms:
/// `u_m`, `u_res`, `u_uv` (the vertex shader), `iResolution`, `iTime`
/// (`T - L.from`), `iGlobalTime` (`T`), `iProgress`
/// (`clamp((T - L.from) / max(L.dur, 1e-4), 0, 1)`), `iFrame`
/// (`uniform1i(Math.round(T * PM.proj.fps))`, the open composition's rate),
/// `iMouse` (0, 0).
fn base_calls(
    scope: &Scope<'_, '_>,
    layer: &Node,
    time: f64,
    tw: u32,
    th: u32,
) -> Vec<(String, Call)> {
    let (w, h) = (f64::from(tw), f64::from(th));
    let from = node::to_number(node::member(Some(layer), "from"));
    let dur = node::to_number(node::member(Some(layer), "dur"));
    let fps = node::to_number(scope.project.member("fps"));
    let progress = js::clamp((time - from) / js::max(dur, 1e-4), 0.0, 1.0);
    vec![
        (
            "u_m".into(),
            Call::M3([w as f32, 0.0, 0.0, 0.0, h as f32, 0.0, 0.0, 0.0, 1.0]),
        ),
        ("u_res".into(), f(&[w, h])),
        ("u_uv".into(), f(&[0.0, 0.0, 1.0, 1.0])),
        ("iResolution".into(), f(&[w, h])),
        ("iTime".into(), f(&[time - from])),
        ("iGlobalTime".into(), f(&[time])),
        ("iProgress".into(), f(&[progress])),
        ("iFrame".into(), Call::I(to_int32(js::round(time * fps)))),
        ("iMouse".into(), f(&[0.0, 0.0])),
    ]
}

/// `Number(value) || 0`.
fn number_or_zero(v: &JsValue) -> f64 {
    let n = v.to_number();
    if n.is_nan() || n == 0.0 { 0.0 } else { n }
}

fn color_call(v: &JsValue) -> Call {
    f(&crate::effects::plan::hex2rgb(&v.to_js_string()))
}

/// Translates a wrapped program and turns its verdict into a plan.
#[allow(clippy::too_many_arguments)]
fn program_plan(
    layer: &Node,
    what: &str,
    source: String,
    line_offset: u32,
    calls: Vec<(String, Call)>,
    (w, h, tw, th): (f64, f64, u32, u32),
    fallback_runner: bool,
) -> Planned {
    let (key, verdict) = translate::verdict(&source, line_offset);
    let program = match verdict {
        Verdict::Ready(prepared) => ProgramRef::Code {
            key,
            prepared,
            source: source.into(),
        },
        Verdict::Refused(failure) => {
            return Planned::Refused(format!(
                "the {what} does not compile ({}: {})",
                failure.stage.name(),
                failure.message.lines().next().unwrap_or_default()
            ));
        }
        Verdict::NeedsFallback(failure) => {
            if !fallback_runner {
                return Planned::Unsupported(Feature::ShaderFallback(what.to_owned()));
            }
            ProgramRef::Fallback(Arc::new(super::fallback::request(
                js::layer_id(layer),
                source,
                tw,
                th,
                &calls,
                &failure,
            )))
        }
    };
    Planned::Draw {
        source: Box::new(ShaderSource {
            layer: js::layer_id(layer),
            program,
            width: tw,
            height: th,
            calls,
        }),
        w,
        h,
    }
}

/// A shader layer at `time` (comp seconds of its composition).
pub(crate) fn shader_layer<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    d: &Content,
    time: f64,
    output: (u32, u32),
    scale: (f64, f64),
    fallback_runner: bool,
) -> JsResult<Planned> {
    let sized = size(scope, layer, d, output, scale, time)?;
    let code_value = js::get(d, "code");
    // `PM.SHADER_HEADER + '\n' + d.code` (string concatenation, so a missing
    // code is the text "undefined" and fails to compile, as today).
    let code = code_value.to_js_string().to_string_lossy();
    let (source, line_offset) = super::shader_layer_program(&code);
    let mut calls = base_calls(scope, layer, time, sized.2, sized.3);
    // `uniformDefsFor(codeKey, d.code)`: `PM.parseUniforms(d.code || '')`.
    let defs = if code_value.truthy() {
        parse_uniforms(&code)
    } else {
        Vec::new()
    };
    let uniforms = node::member(node::member(Some(layer), "d"), "uniforms");
    for name in node::for_in_keys(uniforms) {
        let got = uniforms
            .map(|u| node::get(u, name.as_bytes()))
            .unwrap_or(node::Got::Undefined);
        let key = name.to_string_lossy();
        let value = scope.channel_p(layer, got.node(), time, &key)?;
        let def = defs.iter().find(|u| u.name == key);
        let call = match def.map(|d| &d.control) {
            Some(Control::Color { .. }) => color_call(&value),
            Some(Control::Toggle { .. }) => Call::I(i32::from(value.truthy())),
            _ => f(&[number_or_zero(&value)]),
        };
        calls.push((key, call));
    }
    Ok(program_plan(
        layer,
        "shader layer",
        source,
        line_offset,
        calls,
        sized,
        fallback_runner,
    ))
}

/// The missing-definition checkerboard for `layer` at its extension size:
/// what the compositor draws for an extension layer with no fragment
/// renderer, which includes a legacy 3D scene layer whose scene does not
/// sync (`sceneRuntime` throws or has no scene; the compositor falls
/// through to this program).
#[allow(clippy::too_many_arguments)]
pub(crate) fn missing_placeholder<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    d: &Content,
    time: f64,
    output: (u32, u32),
    scale: (f64, f64),
    fallback_runner: bool,
) -> JsResult<Planned> {
    let sized = size(scope, layer, d, output, scale, time)?;
    let (source, line_offset) = super::missing_layer_program();
    let calls = base_calls(scope, layer, time, sized.2, sized.3);
    Ok(program_plan(
        layer,
        "missing-layer placeholder",
        source,
        line_offset,
        calls,
        sized,
        fallback_runner,
    ))
}

/// An extension layer whose definition is a fragment renderer, or whose
/// definition is marked missing (the checkerboard). `None` for other
/// renderer kinds (mesh, legacy scene, 3D roles) and when the renderer has
/// no definitions (routed as before).
#[allow(clippy::too_many_arguments)]
pub(crate) fn extension_layer<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    d: &Content,
    definition: &str,
    time: f64,
    output: (u32, u32),
    scale: (f64, f64),
    definitions: Option<&Effects>,
    fallback_runner: bool,
) -> JsResult<Option<Planned>> {
    let Some(effects) = definitions else {
        return Ok(None);
    };
    let sized = size(scope, layer, d, output, scale, time)?;
    let Some(snapshot) = effects.get(DefinitionKind::Layer, definition) else {
        if !effects.is_missing(DefinitionKind::Layer, definition) {
            return Ok(Some(Planned::Unsupported(Feature::Definition(format!(
                "layer {definition}"
            )))));
        }
        // `definition` undefined: the header and the checkerboard.
        let (source, line_offset) = super::missing_layer_program();
        let calls = base_calls(scope, layer, time, sized.2, sized.3);
        return Ok(Some(program_plan(
            layer,
            "missing-layer placeholder",
            source,
            line_offset,
            calls,
            sized,
            fallback_runner,
        )));
    };
    if !matches!(snapshot.renderer, Some(LayerRenderer::Fragment { .. })) {
        return Ok(None);
    }
    let mut calls = base_calls(scope, layer, time, sized.2, sized.3);
    let params = node::member(node::member(Some(layer), "d"), "params");
    for spec in &snapshot.params {
        let k = spec.key();
        // `d.params?.[param.k]`, then `property ? PM.evP(…) : param.def`.
        let property = node::member(params, k).filter(|p| node::truthy(Some(p)));
        let value = match property {
            Some(p) => scope.channel_p(layer, Some(p), time, k)?,
            None => match spec {
                ParamSpec::Number { default, .. } => JsValue::Number(*default),
                ParamSpec::Color { default, .. } => JsValue::from(default.as_str()),
                ParamSpec::Toggle { default, .. } => JsValue::Bool(*default),
            },
        };
        let call = match spec {
            ParamSpec::Color { .. } => color_call(&value),
            ParamSpec::Toggle { .. } => Call::I(i32::from(value.truthy())),
            ParamSpec::Number { .. } => f(&[number_or_zero(&value)]),
        };
        calls.push((format!("u_{k}"), call));
    }
    let id = definition.to_owned();
    let program = match effects.status(DefinitionKind::Layer, definition) {
        Some(Status::Ready) => ProgramRef::Definition {
            key: effects.source_key(DefinitionKind::Layer, definition),
            id,
        },
        Some(Status::Refused(failure)) => {
            return Ok(Some(Planned::Refused(format!(
                "the shader of extension layer {definition} does not compile ({}: {})",
                failure.stage.name(),
                failure.message.lines().next().unwrap_or_default()
            ))));
        }
        Some(Status::NeedsFallback(failure)) => {
            if !fallback_runner {
                return Ok(Some(Planned::Unsupported(Feature::ShaderFallback(id))));
            }
            let source = snapshot
                .glsl
                .as_ref()
                .map(|g| g.source.clone())
                .unwrap_or_default();
            ProgramRef::Fallback(Arc::new(super::fallback::request(
                js::layer_id(layer),
                source,
                sized.2,
                sized.3,
                &calls,
                failure,
            )))
        }
        // A fragment snapshot without a program (contract 10 requires one).
        None => {
            return Ok(Some(Planned::Unsupported(Feature::Definition(format!(
                "layer {definition} (no program)"
            )))));
        }
    };
    Ok(Some(Planned::Draw {
        source: Box::new(ShaderSource {
            layer: js::layer_id(layer),
            program,
            width: sized.2,
            height: sized.3,
            calls,
        }),
        w: sized.0,
        h: sized.1,
    }))
}

/// The static verdict of a shader layer's code (capability routing):
/// `Some(feature)` when only the fallback runner can draw it.
pub(crate) fn shader_layer_feature(layer: &Node) -> Option<Feature> {
    // `d.code` as the plan reads it (shader layers store it as a plain
    // string; `resolveContent` declares no `code` channel).
    let code = node::member(node::member(Some(layer), "d"), "code");
    let text: JsString = node::js_value_opt(code).to_js_string();
    let (source, offset) = super::shader_layer_program(&text.to_string_lossy());
    match translate::verdict(&source, offset).1 {
        Verdict::NeedsFallback(_) => Some(Feature::ShaderFallback("shader layer".into())),
        _ => None,
    }
}

/// The static check of an extension layer that is not a 3D role, given the
/// renderer's definitions: `None` when it is drawn (a fragment program the
/// pipeline translates or WebGL refuses too, a mesh layer, a definition
/// marked missing), else what routes it.
pub(crate) fn extension_feature(effects: &Effects, definition: &str) -> Option<Feature> {
    let Some(snapshot) = effects.get(DefinitionKind::Layer, definition) else {
        return (!effects.is_missing(DefinitionKind::Layer, definition))
            .then(|| Feature::Definition(format!("layer {definition}")));
    };
    match &snapshot.renderer {
        Some(LayerRenderer::Fragment { .. }) => match effects
            .status(DefinitionKind::Layer, definition)
        {
            Some(Status::NeedsFallback(_)) => Some(Feature::ShaderFallback(definition.to_owned())),
            Some(_) => None,
            None => Some(Feature::Definition(format!(
                "layer {definition} (no program)"
            ))),
        },
        // The fixed mesh shading (`crate::scene3d::legacy`).
        Some(LayerRenderer::Mesh { .. }) => None,
        _ => Some(Feature::ExtensionDefinition(definition.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform1i_converts_like_webidl_long() {
        assert_eq!(to_int32(3.0), 3);
        assert_eq!(to_int32(-3.7), -3);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_int32(2_147_483_648.0), -2_147_483_648);
        assert_eq!(to_int32(4_294_967_297.0), 1);
    }
}
