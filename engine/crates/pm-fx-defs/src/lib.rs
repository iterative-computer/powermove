//! Native, inspectable definitions from immutable tagged and main registries.
//! `builtins()` retains v1.2.0; `catalog::main_catalog()` is explicitly separate.
//!
//! This is data, not a second contribution registry: callers supply the active
//! extension definition when an extension overrides a built-in (N4/N6). No JS,
//! shader compiler, renderer, or platform APIs are needed for defaults/Reset.

pub mod catalog;
pub mod graph;
pub mod inputs;
pub mod lut;
pub mod spatial;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::OnceLock};

/// Complete parameter metadata. Unknown presentation fields survive serde.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub k: String,
    pub label: String,
    pub def: Value,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<serde_json::Number>,
    #[serde(flatten)]
    pub presentation: Map<String, Value>,
}

impl Parameter {
    pub fn is_reference(&self) -> bool {
        matches!(self.kind.as_deref(), Some("layer" | "path" | "asset"))
    }
}

/// Effect or transition definition; shader source and presentation are retained
/// verbatim as extension-authored data. Optional flags remain absent when absent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub params: Vec<Parameter>,
    pub frag: String,
    #[serde(flatten)]
    pub metadata: Map<String, Value>,
}

impl Definition {
    /// Values for `add_effect` / Reset, in parameter-key order. Never invent a
    /// default for an unavailable extension definition.
    pub fn defaults(&self) -> BTreeMap<String, Value> {
        self.params
            .iter()
            .filter(|p| !p.is_reference())
            .map(|p| (p.k.clone(), p.def.clone()))
            .collect()
    }

    /// Fresh property channels. Each call owns its arrays; mutation cannot alter
    /// the schema or another layer. `expr:null` is the shipped PM.P contract.
    pub fn channels(&self) -> BTreeMap<String, PropertyChannel> {
        self.params
            .iter()
            .filter(|p| !p.is_reference())
            .map(|p| (p.k.clone(), PropertyChannel::new(p.def.clone())))
            .collect()
    }

    /// Typed reference creation defaults belong in Effect.inputs, outside
    /// animation data. Every invocation owns its sentinels.
    pub fn input_defaults(&self) -> inputs::Inputs {
        self.params
            .iter()
            .filter_map(|p| {
                inputs::empty_reference(p.kind.as_deref()?).map(|input| (p.k.clone(), input))
            })
            .collect()
    }

    pub fn default_for(&self, key: &str) -> Option<&Value> {
        self.params.iter().find(|p| p.k == key).map(|p| &p.def)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyChannel {
    pub v: Value,
    pub kf: Vec<Value>,
    pub expr: Value,
}

impl PropertyChannel {
    pub fn new(v: Value) -> Self {
        Self {
            v,
            kf: Vec::new(),
            expr: Value::Null,
        }
    }
}

/// Composition-dependent defaults must be resolved against the owning comp,
/// never the active UI comp or a hard-coded 1920×1080 frame.
#[derive(Clone, Copy, Debug)]
pub struct DefaultContext {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChannelDefault {
    Literal { value: Value },
    Width,
    Height,
    HalfWidth,
    HalfHeight,
}

impl ChannelDefault {
    pub fn resolve(&self, context: DefaultContext) -> Option<Value> {
        match self {
            Self::Literal { value } => Some(value.clone()),
            Self::Width => context
                .width
                .is_finite()
                .then(|| Value::from(context.width)),
            Self::Height => context
                .height
                .is_finite()
                .then(|| Value::from(context.height)),
            Self::HalfWidth => context
                .width
                .is_finite()
                .then(|| Value::from(context.width / 2.0)),
            Self::HalfHeight => context
                .height
                .is_finite()
                .then(|| Value::from(context.height / 2.0)),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Builtins {
    pub tag: String,
    pub commit: String,
    /// Immutable source blob hashes, for auditing/regeneration.
    pub blobs: BTreeMap<String, String>,
    pub effects: Vec<Definition>,
    pub transitions: Vec<Definition>,
    /// PM.CH metadata, including units, bounds and step sizes.
    pub channels: BTreeMap<String, Value>,
    /// Transform paths and `c.` content paths, by layer type.
    pub layer_defaults: BTreeMap<String, BTreeMap<String, ChannelDefault>>,
    /// Explicit v1.2.0 inspector transform Reset defaults, which differ
    /// from constructor defaults for a few layer types.
    pub reset_defaults: BTreeMap<String, BTreeMap<String, ChannelDefault>>,
}

impl Builtins {
    pub fn effect(&self, id: &str) -> Option<&Definition> {
        self.effects.iter().find(|e| e.id == id)
    }

    pub fn transition(&self, id: &str) -> Option<&Definition> {
        self.transitions.iter().find(|e| e.id == id)
    }

    /// The shipped transform Reset contract. For content/effect/transition
    /// channels, use the active definition's explicit default instead; if
    /// none exists, clear keys/expression and retain the static value.
    pub fn reset_default(
        &self,
        layer_type: &str,
        key: &str,
        context: DefaultContext,
    ) -> Option<Value> {
        self.reset_defaults
            .get(layer_type)?
            .get(key)?
            .resolve(context)
    }

    pub fn channel_default(
        &self,
        layer_type: &str,
        key: &str,
        context: DefaultContext,
    ) -> Option<Value> {
        self.layer_defaults
            .get(layer_type)?
            .get(key)?
            .resolve(context)
    }
}

pub fn builtins() -> &'static Builtins {
    static BUILTINS: OnceLock<Builtins> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        serde_json::from_str(include_str!("builtins.json")).expect("generated built-in definitions")
    })
}
