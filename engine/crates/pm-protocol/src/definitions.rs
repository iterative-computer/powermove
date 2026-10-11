//! Definition snapshots and lifecycle (contract 10, section 11).
//!
//! The kernel pushes a snapshot of each registered effect, transition, and
//! extension-layer definition to every session: `{id, version, renderer
//! (verbatim), glsl?, params, flags}` with UI-only fields stripped. Snapshots
//! follow the winning registration per id (last registered wins; disposing
//! restores the previous one), so the engine only ever sees winners. Updates
//! carry a generation; the engine applies them in order and reports compile
//! failures per definition.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{DefinitionId, Seq};
use crate::error::EngineError;
use crate::validate::{ensure, ensure_unique, Validate, Violation};
use crate::version::ContractId;

/// What a definition registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DefinitionKind {
    Effect,
    Transition,
    Layer,
}

/// Where the winning registration came from; decides the limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum DefinitionOrigin {
    /// A built-in extension.
    BuiltIn,
    /// A local fork of a built-in registering under the built-in id.
    Fork,
    /// A local or agent-published extension running in the editor realm.
    Local,
    /// A Store extension in its sandbox (untrusted GLSL).
    Store,
}

impl DefinitionOrigin {
    pub const fn sandboxed(self) -> bool {
        matches!(self, DefinitionOrigin::Store)
    }
}

/// An extension layer's renderer, verbatim from the registration
/// (`kernel/sandbox-schemas.ts` `layers.renderer`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LayerRenderer {
    /// A GLSL `void main()` body (the user's text; the wrapped source is in `glsl`).
    Fragment { fragment: String },
    /// A mesh: `asset_field` names the key in layer data holding the model asset id.
    #[serde(rename_all = "camelCase")]
    Mesh { asset_field: String },
    /// The legacy kernel-owned scene renderer.
    #[serde(rename = "scene3d")]
    #[cfg_attr(feature = "ts", ts(rename = "scene3d"))]
    Scene3d,
    /// A 3D world member.
    #[serde(rename = "layer3d")]
    #[cfg_attr(feature = "ts", ts(rename = "layer3d"))]
    Layer3d { role: Layer3dRole },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Layer3dRole {
    Object,
    Light,
    Camera,
}

/// One parameter (UI fields stripped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ParamSpec {
    Number { key: String, default: f64, min: f64, max: f64, step: Option<f64> },
    /// `#rrggbb`.
    Color { key: String, default: String },
    Toggle { key: String, default: bool },
}

impl ParamSpec {
    pub fn key(&self) -> &str {
        match self {
            ParamSpec::Number { key, .. } | ParamSpec::Color { key, .. } | ParamSpec::Toggle { key, .. } => key,
        }
    }
}

/// Kernel-wrapped GLSL ES 3.00 ready for the engine pipeline (section 11:
/// wrapping uses the existing kernel generators, which stay TypeScript).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WrappedGlsl {
    pub source: String,
    /// Lines the wrapper added before the user's first line; errors subtract it.
    pub body_line_offset: u32,
    /// The user's body length in UTF-16 code units (what the kernel limits).
    pub body_utf16_len: u32,
    /// The user's body length in UTF-8 bytes (in-realm extension layers are limited in bytes).
    pub body_utf8_len: u32,
}

/// Effect and transition flags (`kernel/api.ts` `EffectDefinition`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DefinitionFlags {
    /// Ping-pong passes, 1 to 8.
    pub passes: u8,
    pub keep_orig: bool,
    pub backdrop: bool,
    pub raw_shader: bool,
    pub viewport_safe: bool,
    pub viewport_padding: Vec<String>,
    /// Values above 1.0 pass through unless an effect declares `clamp: true`.
    pub clamp: bool,
}

impl Default for DefinitionFlags {
    fn default() -> Self {
        Self { passes: 1, keep_orig: false, backdrop: false, raw_shader: false, viewport_safe: false, viewport_padding: vec![], clamp: false }
    }
}

/// A registered definition as the engine sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DefinitionSnapshot {
    pub id: DefinitionId,
    pub kind: DefinitionKind,
    /// Extension-layer `version`; 0 for effects and transitions.
    pub version: u32,
    pub origin: DefinitionOrigin,
    /// Extension layers only.
    pub renderer: Option<LayerRenderer>,
    /// Effects, transitions, and fragment layers.
    pub glsl: Option<WrappedGlsl>,
    pub params: Vec<ParamSpec>,
    pub flags: DefinitionFlags,
}

/// Limits from section 3 and `kernel/sandbox-schemas.ts`.
pub mod limits {
    pub const EFFECT_PARAMS: usize = 32;
    pub const EFFECT_MIN_PASSES: u8 = 1;
    pub const EFFECT_MAX_PASSES: u8 = 8;
    /// Effect and transition fragments, in UTF-16 code units.
    pub const EFFECT_FRAGMENT_UTF16: u32 = 65_536;
    pub const LAYER_PARAMS_IN_REALM: usize = 48;
    pub const LAYER_FRAGMENT_UTF8_IN_REALM: u32 = 96_000;
    pub const LAYER_PARAMS_STORE: usize = 32;
    pub const LAYER_FRAGMENT_UTF16_STORE: u32 = 65_536;
    /// Wrapped source ceiling: the user's body plus the kernel preamble.
    pub const WRAPPED_SOURCE_BYTES: usize = 512 * 1024;
}

fn valid_param_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some('a'..='z')) && chars.all(|c| c.is_ascii_alphanumeric())
}

fn valid_hex_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

impl Validate for DefinitionSnapshot {
    fn validate(&self) -> Result<(), Violation> {
        use limits::*;
        const C: ContractId = ContractId::Definitions;
        ensure(self.id.is_valid() && self.id.0.len() <= 128, C, || format!("definition id {:?} must be 1 to 128 bytes", self.id))?;
        ensure_unique(self.params.iter().map(ParamSpec::key), C, "parameter key")?;
        for param in &self.params {
            ensure(valid_param_key(param.key()), C, || format!("parameter key {:?} must match ^[a-z][a-zA-Z0-9]*$", param.key()))?;
            match param {
                ParamSpec::Number { default, min, max, step, .. } => {
                    ensure([*default, *min, *max].iter().all(|v| v.is_finite()) && min <= max, C, || format!("parameter {:?} has a bad range", param.key()))?;
                    ensure(step.is_none_or(|s| s.is_finite() && s > 0.0), C, || format!("parameter {:?} step must be positive", param.key()))?;
                }
                ParamSpec::Color { default, .. } => ensure(valid_hex_color(default), C, || format!("parameter {:?} default must be #rrggbb", param.key()))?,
                ParamSpec::Toggle { .. } => {}
            }
        }
        if let Some(glsl) = &self.glsl {
            ensure(glsl.source.len() <= WRAPPED_SOURCE_BYTES, C, || "wrapped GLSL exceeds the source ceiling".into())?;
            ensure(glsl.body_utf16_len > 0, C, || "empty shader body".into())?;
        }
        match self.kind {
            DefinitionKind::Effect | DefinitionKind::Transition => {
                ensure(self.renderer.is_none(), C, || "effects and transitions have no layer renderer".into())?;
                let glsl = self.glsl.as_ref().ok_or_else(|| Violation::new(C, "effects and transitions carry GLSL"))?;
                ensure(self.params.len() <= EFFECT_PARAMS, C, || format!("at most {EFFECT_PARAMS} parameters"))?;
                ensure(glsl.body_utf16_len <= EFFECT_FRAGMENT_UTF16, C, || format!("fragment exceeds {EFFECT_FRAGMENT_UTF16} characters"))?;
                ensure((EFFECT_MIN_PASSES..=EFFECT_MAX_PASSES).contains(&self.flags.passes), C, || {
                    format!("passes must be {EFFECT_MIN_PASSES} to {EFFECT_MAX_PASSES}, got {}", self.flags.passes)
                })?;
                if self.kind == DefinitionKind::Transition {
                    ensure(self.flags.passes == 1 && !self.flags.keep_orig && !self.flags.backdrop, C, || {
                        "transitions are single-pass with no u_orig or u_backdrop".into()
                    })?;
                }
                ensure(self.flags.viewport_padding.iter().all(|k| self.params.iter().any(|p| matches!(p, ParamSpec::Number { key, .. } if key == k))), C, || {
                    "viewport padding names numeric parameter keys".into()
                })?;
            }
            DefinitionKind::Layer => {
                let renderer = self.renderer.as_ref().ok_or_else(|| Violation::new(C, "extension layers carry a renderer"))?;
                let max_params = if self.origin.sandboxed() { LAYER_PARAMS_STORE } else { LAYER_PARAMS_IN_REALM };
                ensure(self.params.len() <= max_params, C, || format!("at most {max_params} parameters for this origin"))?;
                ensure(self.flags == DefinitionFlags::default(), C, || "layer definitions carry default flags".into())?;
                match renderer {
                    LayerRenderer::Fragment { fragment } => {
                        let glsl = self.glsl.as_ref().ok_or_else(|| Violation::new(C, "fragment layers carry wrapped GLSL"))?;
                        ensure(glsl.body_utf8_len as usize == fragment.len(), C, || "wrapped body length does not match the fragment".into())?;
                        if self.origin.sandboxed() {
                            ensure(glsl.body_utf16_len <= LAYER_FRAGMENT_UTF16_STORE, C, || format!("Store layer fragments are at most {LAYER_FRAGMENT_UTF16_STORE} characters"))?;
                        } else {
                            ensure(glsl.body_utf8_len <= LAYER_FRAGMENT_UTF8_IN_REALM, C, || format!("layer fragments are at most {LAYER_FRAGMENT_UTF8_IN_REALM} UTF-8 bytes"))?;
                        }
                    }
                    LayerRenderer::Mesh { asset_field } => {
                        ensure(!asset_field.is_empty() && asset_field.len() <= 128, C, || "asset field must be 1 to 128 bytes".into())?;
                        ensure(self.glsl.is_none(), C, || "mesh layers carry no GLSL".into())?;
                    }
                    LayerRenderer::Scene3d | LayerRenderer::Layer3d { .. } => {
                        ensure(self.glsl.is_none(), C, || "3D layers carry no GLSL".into())?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Built-in ids with id-based 3D rules (section 11). When a fork registers
/// one of these with a different renderer kind, the snapshot's kind wins.
pub const BUILTIN_3D_IDS: [&str; 4] = ["powermove.3d.object", "powermove.3d.light", "powermove.3d.camera", "powermove.3d.scene"];

/// A removal: no registration remains for the id (the engine marks it
/// missing: effects are skipped with their data kept, layers show the
/// checkerboard placeholder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DefinitionRemoval {
    pub id: DefinitionId,
    pub kind: DefinitionKind,
}

/// Pushed on every register, dispose, and hot reload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DefinitionsUpdate {
    /// Increases by one per update on a session; `full` resets the set.
    pub generation: Seq,
    pub full: bool,
    pub upserts: Vec<DefinitionSnapshot>,
    pub removals: Vec<DefinitionRemoval>,
}

impl Validate for DefinitionsUpdate {
    fn validate(&self) -> Result<(), Violation> {
        const C: ContractId = ContractId::Definitions;
        self.upserts.validate()?;
        ensure_unique(self.upserts.iter().map(|d| (d.kind, &d.id)).chain(self.removals.iter().map(|r| (r.kind, &r.id))), C, "definition in one update")?;
        ensure(!self.full || self.removals.is_empty(), C, || "a full update has no removals".into())
    }
}

/// The engine's answer to an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DefinitionsApplied {
    pub generation: Seq,
    /// Compile or translation failures; each stays per definition.
    pub failures: Vec<EngineError>,
}

/// The snapshots an export job or Collect Project embeds.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DefinitionSetSnapshot {
    pub generation: Seq,
    pub definitions: Vec<DefinitionSnapshot>,
    /// Ids already marked missing (pm-cli fails only on ids neither here nor in `definitions`).
    pub missing: Vec<DefinitionId>,
}

/// Engine-side definition set: applies updates in generation order.
#[derive(Debug, Clone, Default)]
pub struct DefinitionSet {
    generation: Option<Seq>,
    definitions: std::collections::BTreeMap<(DefinitionKind, DefinitionId), DefinitionSnapshot>,
    missing: std::collections::BTreeSet<(DefinitionKind, DefinitionId)>,
}

/// An update that cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("update generation {got} does not follow {current:?}; the host must send a full update")]
    Gap { got: Seq, current: Option<Seq> },
    #[error(transparent)]
    Invalid(#[from] Violation),
}

impl DefinitionSet {
    pub fn generation(&self) -> Option<Seq> {
        self.generation
    }

    pub fn get(&self, kind: DefinitionKind, id: &DefinitionId) -> Option<&DefinitionSnapshot> {
        self.definitions.get(&(kind, id.clone()))
    }

    pub fn is_missing(&self, kind: DefinitionKind, id: &DefinitionId) -> bool {
        self.missing.contains(&(kind, id.clone()))
    }

    /// Applies an update. Deltas must follow the current generation by one;
    /// a full update replaces everything at any generation greater than the current.
    pub fn apply(&mut self, update: &DefinitionsUpdate) -> Result<(), UpdateError> {
        update.validate()?;
        let follows = match self.generation {
            None => update.full,
            Some(current) if update.full => update.generation > current,
            Some(current) => current.next() == Some(update.generation),
        };
        if !follows {
            return Err(UpdateError::Gap { got: update.generation, current: self.generation });
        }
        if update.full {
            for key in self.definitions.keys() {
                if !update.upserts.iter().any(|d| (d.kind, &d.id) == (key.0, &key.1)) {
                    self.missing.insert(key.clone());
                }
            }
            self.definitions.clear();
        }
        for snapshot in &update.upserts {
            let key = (snapshot.kind, snapshot.id.clone());
            self.missing.remove(&key);
            self.definitions.insert(key, snapshot.clone());
        }
        for removal in &update.removals {
            let key = (removal.kind, removal.id.clone());
            self.definitions.remove(&key);
            self.missing.insert(key);
        }
        self.generation = Some(update.generation);
        Ok(())
    }

    pub fn snapshot(&self) -> DefinitionSetSnapshot {
        DefinitionSetSnapshot {
            generation: self.generation.unwrap_or(Seq::ZERO),
            definitions: self.definitions.values().cloned().collect(),
            missing: self.missing.iter().map(|(_, id)| id.clone()).collect(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn effect(id: &str) -> DefinitionSnapshot {
        let body = "o = src(v_uv) * u_amount;";
        DefinitionSnapshot {
            id: id.into(),
            kind: DefinitionKind::Effect,
            version: 0,
            origin: DefinitionOrigin::BuiltIn,
            renderer: None,
            glsl: Some(WrappedGlsl {
                source: format!("#version 300 es\nprecision highp float;\nuniform float u_amount;\nvoid main(){{\n{body}\n}}\n"),
                body_line_offset: 4,
                body_utf16_len: body.encode_utf16().count() as u32,
                body_utf8_len: body.len() as u32,
            }),
            params: vec![ParamSpec::Number { key: "amount".into(), default: 1.0, min: 0.0, max: 2.0, step: Some(0.01) }],
            flags: DefinitionFlags::default(),
        }
    }

    fn seq(n: u64) -> Seq {
        Seq::new(n).unwrap()
    }

    #[test]
    fn effect_limits() {
        assert!(effect("fx.tint").validate().is_ok());
        let mut fx = effect("fx.tint");
        fx.flags.passes = 9;
        assert!(fx.validate().is_err());
        let mut fx = effect("fx.tint");
        fx.params = (0..33).map(|i| ParamSpec::Toggle { key: format!("p{i}"), default: false }).collect();
        assert!(fx.validate().is_err());
        let mut fx = effect("fx.tint");
        fx.params.push(ParamSpec::Toggle { key: "amount".into(), default: true });
        assert!(fx.validate().is_err(), "duplicate keys");
        let mut fx = effect("fx.tint");
        fx.params[0] = ParamSpec::Number { key: "Amount".into(), default: 1.0, min: 0.0, max: 1.0, step: None };
        assert!(fx.validate().is_err(), "keys start lowercase");
        let mut fx = effect("fx.tint");
        fx.flags.viewport_padding = vec!["missing".into()];
        assert!(fx.validate().is_err());
    }

    #[test]
    fn store_layers_have_tighter_limits() {
        let fragment = "fragColor = vec4(uv, 0.5, 1.0);".to_string();
        let layer = DefinitionSnapshot {
            id: "store.layer".into(),
            kind: DefinitionKind::Layer,
            version: 1,
            origin: DefinitionOrigin::Local,
            renderer: Some(LayerRenderer::Fragment { fragment: fragment.clone() }),
            glsl: Some(WrappedGlsl { source: fragment.clone(), body_line_offset: 0, body_utf16_len: fragment.len() as u32, body_utf8_len: fragment.len() as u32 }),
            params: (0..40).map(|i| ParamSpec::Number { key: format!("p{i}"), default: 0.0, min: 0.0, max: 1.0, step: None }).collect(),
            flags: DefinitionFlags::default(),
        };
        assert!(layer.validate().is_ok(), "48 params in realm");
        let store = DefinitionSnapshot { origin: DefinitionOrigin::Store, ..layer };
        assert!(store.validate().is_err(), "32 params for Store layers");
    }

    #[test]
    fn renderer_encodes_like_the_kernel_schema() {
        let mesh = LayerRenderer::Mesh { asset_field: "model".into() };
        let json = serde_json::to_value(&mesh).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "mesh", "assetField": "model" }));
        let role = serde_json::to_value(LayerRenderer::Layer3d { role: Layer3dRole::Light }).unwrap();
        assert_eq!(role, serde_json::json!({ "kind": "layer3d", "role": "light" }));
        assert_eq!(serde_json::to_value(LayerRenderer::Scene3d).unwrap(), serde_json::json!({ "kind": "scene3d" }));
    }

    #[test]
    fn set_applies_generations_in_order() {
        let mut set = DefinitionSet::default();
        let delta = |generation, upserts, removals| DefinitionsUpdate { generation: seq(generation), full: false, upserts, removals };
        assert!(matches!(set.apply(&delta(1, vec![effect("a")], vec![])), Err(UpdateError::Gap { .. })), "first update must be full");
        set.apply(&DefinitionsUpdate { generation: seq(1), full: true, upserts: vec![effect("a"), effect("b")], removals: vec![] }).unwrap();
        set.apply(&delta(2, vec![], vec![DefinitionRemoval { id: "b".into(), kind: DefinitionKind::Effect }])).unwrap();
        assert!(set.is_missing(DefinitionKind::Effect, &"b".into()));
        assert!(matches!(set.apply(&delta(4, vec![], vec![])), Err(UpdateError::Gap { .. })));
        set.apply(&delta(3, vec![effect("b")], vec![])).unwrap();
        assert!(!set.is_missing(DefinitionKind::Effect, &"b".into()), "re-registering restores it");
        set.apply(&DefinitionsUpdate { generation: seq(10), full: true, upserts: vec![effect("c")], removals: vec![] }).unwrap();
        let snapshot = set.snapshot();
        assert_eq!(snapshot.definitions.len(), 1);
        assert_eq!(snapshot.missing.len(), 2);
        assert_eq!(set.generation(), Some(seq(10)));
    }
}
