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
use crate::capability::{Feature, Unsupported};
use crate::plan::CompPlan;

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
}

/// A transition node.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionStep {
    pub definition: String,
    /// `edge === 'in'`: from the stack below to the stack with the layer.
    pub edge_in: bool,
    pub progress: f64,
    pub params: Vec<Param>,
}

/// A layer drawn into its own buffer (`lf`), processed, then composited:
/// today's path for layers with effects or an active transition.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    /// The layer's pixels: its draws, or a group's members, into a
    /// transparent target the size of the level.
    pub content: CompPlan,
    pub effects: Vec<EffectStep>,
    pub transition: Option<TransitionStep>,
    /// Opacity of the final composite: a group's opacity (`compositeAlpha`);
    /// 1 for other layers, whose opacity is in their draws.
    pub alpha: f64,
    /// `u_time`: the level's time in seconds.
    pub time: f64,
}

/// What a layer needs at this frame.
pub enum Staging {
    /// The fast path: no renderable effect and no active transition.
    None,
    Stage {
        effects: Vec<EffectStep>,
        transition: Option<TransitionStep>,
    },
    /// Something here cannot be drawn (it was added to `unsupported`).
    Refused,
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
        instance: instance.to_owned(),
        definition: def.id.as_str().to_owned(),
        passes: def.flags.passes,
        keep_orig: def.flags.keep_orig,
        backdrop: def.flags.backdrop,
        clamp: def.flags.clamp,
        viewport_safe: def.flags.viewport_safe,
        padding,
        params,
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

/// The staging of `layer` at `time` (`renderProject`'s choice between the
/// fast path and the layer buffer). Adjustment layers are not staged here.
pub fn staging<'d>(
    scope: &mut Scope<'_, 'd>,
    effects: &Effects,
    comp_id: &str,
    layer: &'d Node,
    time: f64,
    unsupported: &mut Vec<Unsupported>,
) -> JsResult<Staging> {
    let mut refused = false;
    let mut refuse = |feature: Feature, out: &mut Vec<Unsupported>| {
        out.push(Unsupported::layer(comp_id, layer, feature));
        refused = true;
    };
    let has_fx = scope.has_renderable_effects(layer, time)?;
    let mut steps = Vec::new();
    if has_fx {
        // `runEffects`: every enabled effect that is not marked missing and
        // whose definition is registered, in order.
        for fx in node::array_or_empty(node::member(Some(layer), "fx"))? {
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
            let Some(def) = effects.get(DefinitionKind::Effect, &ty) else {
                if !effects.is_missing(DefinitionKind::Effect, &ty) {
                    refuse(Feature::Definition(format!("effect {ty}")), unsupported);
                }
                continue;
            };
            match effects.status(DefinitionKind::Effect, &ty) {
                Some(Status::Ready) => {}
                Some(Status::NeedsFallback(_)) => {
                    refuse(Feature::ShaderFallback(ty.clone()), unsupported);
                    continue;
                }
                // Refused programs render nothing today: skipped.
                Some(Status::Refused(_)) | None => continue,
            }
            if effects.watchdog.is_disabled(DefinitionKind::Effect, &ty) {
                continue;
            }
            // `effectParamValue`: no property (falsy) or a nullish value
            // falls back to the definition's default.
            let p = node::member(Some(fx), "p");
            let mut values = Vec::with_capacity(def.params.len());
            for spec in &def.params {
                let value = match node::member(p, spec.key()).filter(|n| node::truthy(Some(n))) {
                    None => None,
                    Some(prop) => Some(scope.channel_p(layer, Some(prop), time, spec.key())?),
                };
                values.push(value.filter(|v| !matches!(v, JsValue::Undefined | JsValue::Null)));
            }
            steps.push(effect_step(def, &fx_id, &values));
        }
    }
    let known = |ty: &JsString| transition_known(effects, ty);
    let active = scope.active_transition(layer, time, &known)?;
    let mut transition = None;
    if let Some(active) = &active {
        let ty =
            node::to_js_string(node::member(Some(active.transition), "type")).to_string_lossy();
        match effects.get(DefinitionKind::Transition, &ty) {
            None => refuse(Feature::Definition(format!("transition {ty}")), unsupported),
            Some(def) => match effects.status(DefinitionKind::Transition, &ty) {
                Some(Status::NeedsFallback(_)) => {
                    refuse(Feature::ShaderFallback(ty.clone()), unsupported)
                }
                Some(Status::Ready)
                    if !effects
                        .watchdog
                        .is_disabled(DefinitionKind::Transition, &ty) =>
                {
                    // `runTransition`: `prop ? PM.evP(L, prop, T, pd.k) : pd.def`.
                    let p = node::member(Some(active.transition), "p");
                    let mut values = Vec::with_capacity(def.params.len());
                    for spec in &def.params {
                        values.push(
                            match node::member(p, spec.key()).filter(|n| node::truthy(Some(n))) {
                                None => None,
                                Some(prop) => {
                                    Some(scope.channel_p(layer, Some(prop), time, spec.key())?)
                                }
                            },
                        );
                    }
                    transition = Some(transition_step(
                        def,
                        active.edge_in,
                        active.progress,
                        &values,
                    ));
                }
                // A program that does not link: `runTransition` returns
                // null and the layer composites without the transition (on
                // the layer-buffer path).
                _ => {}
            },
        }
    }
    if refused {
        return Ok(Staging::Refused);
    }
    if !has_fx && active.is_none() {
        return Ok(Staging::None);
    }
    Ok(Staging::Stage {
        effects: steps,
        transition,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
