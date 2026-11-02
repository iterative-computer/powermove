use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use pm_kernel::model;
use pm_kernel::{
    Ack, Caller, KernelFailure, KernelHandle, Pending, Published, QueryError, QueryMirror, Request,
};
use pm_model::{JsString, Map, Node, Value};

use crate::{Binding, EffectClipboard, EffectSelection};

#[derive(Clone, Debug, PartialEq)]
pub enum InspectorError {
    MissingLayer(String),
    MissingProperty(PropertyId),
    MissingEffect(String),
    MissingDefinition(String),
    InvalidInput,
    EmptyEdit,
    Query(QueryError),
}
impl From<QueryError> for InspectorError {
    fn from(error: QueryError) -> Self {
        Self::Query(error)
    }
}
impl std::fmt::Display for InspectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for InspectorError {}

/// Stable structural address; opaque strings are never split on delimiters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PropertyId {
    pub layer: String,
    pub path: String,
}
impl PropertyId {
    pub fn new(layer: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            layer: layer.into(),
            path: path.into(),
        }
    }
    /// Lossless agent-facing address, including punctuation and Unicode in ids.
    pub fn value(&self) -> Value {
        object([
            ("layer", self.layer.clone().into()),
            ("path", self.path.clone().into()),
        ])
    }
    pub fn from_value(value: &Value) -> Result<Self, InspectorError> {
        Ok(Self::new(
            value
                .get("layer")
                .and_then(Value::as_str)
                .ok_or(InspectorError::InvalidInput)?,
            value
                .get("path")
                .and_then(Value::as_str)
                .ok_or(InspectorError::InvalidInput)?,
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldState {
    pub value: Value,
    pub mixed: bool,
    pub editable_targets: Vec<PropertyId>,
    pub animated: bool,
    pub key_at_playhead: bool,
    pub expression: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyRow {
    pub id: PropertyId,
    pub state: FieldState,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RowWindow {
    pub total: usize,
    pub range: Range<usize>,
    pub rows: Vec<PropertyRow>,
}

/// The structural mirror is independent of playback's readout clock.
pub struct Inspector {
    pub(crate) published: Arc<Published>,
    pub(crate) mirror: QueryMirror,
    pub(crate) selected: Vec<String>,
    pub(crate) definitions: HashMap<String, crate::EffectDefinition>,
    pub(crate) disclosure: HashMap<(String, String), bool>,
    pub(crate) property_ids: HashMap<String, Vec<PropertyId>>,
    pub effects: EffectSelection,
    pub clipboard: EffectClipboard,
    pub scroll: ScrollMemory,
    pub clock: ReadoutClock,
}
impl Inspector {
    pub fn new(published: Arc<Published>, clipboard: EffectClipboard) -> Self {
        let mirror = QueryMirror::new(&published);
        let mut this = Self {
            published,
            mirror,
            selected: Vec::new(),
            definitions: HashMap::new(),
            disclosure: HashMap::new(),
            property_ids: HashMap::new(),
            effects: EffectSelection::default(),
            clipboard,
            scroll: ScrollMemory::default(),
            clock: ReadoutClock::default(),
        };
        this.refresh_selection();
        this
    }
    /// Call after an acknowledgement or a kernel event. Same-revision selection
    /// changes are adopted too; document replacement clears transient addresses.
    pub fn sync(&mut self, published: Arc<Published>) {
        let replaced = self.published.snapshot.doc_id != published.snapshot.doc_id;
        let changed = replaced || self.published.revision() != published.revision();
        self.mirror.sync(&published);
        self.published = published;
        if changed {
            self.property_ids.clear();
        }
        if replaced {
            self.disclosure.clear();
            self.effects = EffectSelection::default();
            self.scroll = ScrollMemory::default();
        }
        self.refresh_selection();
        if let Some(layer) = self.effects.layer.clone() {
            let ids = self.effect_ids(&layer).unwrap_or_default();
            self.effects.retain(&ids);
        }
    }
    /// Immutable native-host identity; no document mirror crosses the v4 boundary.
    pub fn document_id(&self) -> u64 {
        self.published.snapshot.doc_id
    }
    pub fn effect_definitions(
        &self,
    ) -> &std::collections::HashMap<String, crate::EffectDefinition> {
        &self.definitions
    }
    pub fn revision(&self) -> u64 {
        self.published.revision()
    }
    pub fn project_id(&self) -> &str {
        &self.published.project
    }
    pub fn selected_layers(&self) -> &[String] {
        &self.selected
    }
    pub fn primary(&self) -> Option<&str> {
        self.selected.first().map(String::as_str)
    }
    pub fn composition_mode(&self) -> bool {
        self.selected.is_empty()
    }
    pub fn register_effect(&mut self, definition: crate::EffectDefinition) {
        self.definitions
            .insert(definition.effect_type.clone(), definition);
    }
    pub fn remove_definition(&mut self, effect_type: &str) {
        self.definitions.remove(effect_type);
    }
    /// Host/agent chooses the caller. MCP never acquires Interface privileges.
    pub fn submit(
        &self,
        handle: &KernelHandle,
        request: Request,
    ) -> Pending<Result<Ack, KernelFailure>> {
        handle.call(self.project_id(), request)
    }
    pub fn binding_state(
        &mut self,
        binding: &Binding,
        time: f64,
    ) -> Result<FieldState, InspectorError> {
        if !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        let targets = self.binding_targets(binding)?;
        let value = self.read_binding(binding, time)?;
        let mut values = Vec::new();
        for target in &targets {
            values.push(self.read_binding(target, time)?);
        }
        let ids = targets
            .iter()
            .filter_map(Binding::property_id)
            .collect::<Vec<_>>();
        let primary = binding.property_id();
        let prop = primary
            .as_ref()
            .map(|id| self.property(id))
            .transpose()?
            .flatten();
        let animated = prop
            .as_ref()
            .is_some_and(|p| !array(p.get("kf")).is_empty());
        let expression = prop
            .as_ref()
            .and_then(|p| p.get("expr"))
            .and_then(Value::as_str)
            .filter(|e| !e.is_empty())
            .map(str::to_owned);
        let at = if let Some(id) = &primary {
            self.key_at(id, time)?.is_some()
        } else {
            false
        };
        Ok(FieldState {
            value,
            mixed: values
                .first()
                .is_some_and(|first| values.iter().any(|v| !same_mixed_value(v, first))),
            editable_targets: ids,
            animated,
            key_at_playhead: at,
            expression,
        })
    }
    /// Structural enumeration once per revision. Values are evaluated only for
    /// the requested ordinal window, even for 60,000+ structured properties.
    pub fn property_window(
        &mut self,
        layer: &str,
        range: Range<usize>,
        time: f64,
    ) -> Result<RowWindow, InspectorError> {
        if !self.property_ids.contains_key(layer) {
            self.layer_node(layer)?;
            let records = pm_eval::address::all_props(
                self.layer_node(layer)?,
                &pm_eval::address::NoDefinitions,
            )
            .map_err(eval_error)?;
            let ids = records
                .into_iter()
                .map(|r| PropertyId::new(layer, r.key.to_string_lossy()))
                .collect();
            self.property_ids.insert(layer.to_owned(), ids);
        }
        let ids = &self.property_ids[layer];
        let total = ids.len();
        let start = range.start.min(total);
        let end = range.end.max(start).min(total);
        let visible = ids[start..end].to_vec();
        let rows = visible
            .into_iter()
            .map(|id| {
                let state = self.binding_state(&Binding::Property(id.clone()), time)?;
                Ok(PropertyRow { id, state })
            })
            .collect::<Result<_, InspectorError>>()?;
        Ok(RowWindow {
            total,
            range: start..end,
            rows,
        })
    }
    pub(crate) fn layer_node(&self, id: &str) -> Result<&Node, InspectorError> {
        let root = self
            .published
            .snapshot
            .root()
            .ok_or_else(|| InspectorError::MissingLayer(id.into()))?;
        let loc =
            model::find_layer(root, id).ok_or_else(|| InspectorError::MissingLayer(id.into()))?;
        model::layer_at(root, &loc).ok_or_else(|| InspectorError::MissingLayer(id.into()))
    }
    pub fn layer(&self, id: &str) -> Result<Value, InspectorError> {
        Ok(self.layer_node(id)?.to_value())
    }
    pub fn editable(&self, id: &str) -> bool {
        self.published
            .snapshot
            .root()
            .and_then(|root| model::find_layer(root, id).map(|loc| !model::is_locked(root, &loc)))
            .unwrap_or(false)
    }
    pub(crate) fn refresh_selection(&mut self) {
        let Some(root) = self.published.snapshot.root() else {
            self.selected.clear();
            return;
        };
        let ids: HashSet<&str> = self
            .published
            .selection
            .layers()
            .iter()
            .map(String::as_str)
            .collect();
        self.selected = self
            .published
            .selection
            .layers()
            .iter()
            .filter(|id| {
                let Some(loc) = model::find_layer(root, id) else {
                    return false;
                };
                let Some(layer) = model::layer_at(root, &loc) else {
                    return false;
                };
                let Some(comp) = model::comp_node(root, loc.comp_id.as_deref()) else {
                    return false;
                };
                !model::group_ancestors(comp, layer).iter().any(|g| {
                    g.get("id")
                        .map(Node::to_value)
                        .and_then(|v| v.as_str().map(|s| ids.contains(s)))
                        .unwrap_or(false)
                })
            })
            .cloned()
            .collect();
    }
    pub(crate) fn query(
        &mut self,
        op: &str,
        layer: &str,
        key: Option<&str>,
        prop: Option<Value>,
        time: f64,
    ) -> Result<Value, InspectorError> {
        let root = self
            .published
            .snapshot
            .root()
            .ok_or_else(|| InspectorError::MissingLayer(layer.into()))?;
        let loc = model::find_layer(root, layer)
            .ok_or_else(|| InspectorError::MissingLayer(layer.into()))?;
        let mut request = object([
            ("op", op.into()),
            ("layer", (loc.index as f64).into()),
            ("t", time.into()),
            ("comp", loc.comp_id.unwrap_or_default().into()),
        ]);
        let map = request.as_object_mut().expect("object");
        if let Some(key) = key {
            map.insert("key", key.into());
        }
        if let Some(prop) = prop {
            map.insert("prop", prop);
        }
        Ok(self.mirror.queries().query(&request)?)
    }
    pub(crate) fn property(&mut self, id: &PropertyId) -> Result<Option<Value>, InspectorError> {
        let got = pm_eval::address::find_prop(
            self.layer_node(&id.layer)?,
            &JsString::from(id.path.as_str()),
        )
        .map_err(eval_error)?;
        Ok(got.node().map(Node::to_value))
    }
    pub(crate) fn evaluate(&mut self, id: &PropertyId, time: f64) -> Result<Value, InspectorError> {
        if let Some(prop) = self.property(id)? {
            return self.query("evP", &id.layer, Some(&id.path), Some(prop), time);
        }
        let layer = self.layer(&id.layer)?;
        let value = if let Some(k) = id.path.strip_prefix("c.") {
            layer.get("d").and_then(|d| d.get(k))
        } else if let Some(k) = id.path.strip_prefix("l.") {
            layer.get(k)
        } else if let Some(fx) = id.path.strip_suffix(".$enabled") {
            array(layer.get("fx"))
                .iter()
                .find(|f| text(f, "id") == fx)
                .and_then(|f| f.get("on"))
        } else if let Some(m) = id.path.strip_prefix("m.") {
            m.split_once('.').and_then(|(mid, k)| {
                array(layer.get("masks"))
                    .iter()
                    .find(|m| text(m, "id") == mid)
                    .and_then(|m| m.get(k))
            })
        } else {
            None
        };
        value
            .cloned()
            .ok_or_else(|| InspectorError::MissingProperty(id.clone()))
    }
    pub(crate) fn key_at(
        &mut self,
        id: &PropertyId,
        time: f64,
    ) -> Result<Option<Value>, InspectorError> {
        let Some(prop) = self.property(id)? else {
            return Ok(None);
        };
        let local = time
            - self
                .layer_node(&id.layer)?
                .get("from")
                .map(Node::to_value)
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
        Ok(array(prop.get("kf"))
            .iter()
            .find(|k| (number(k, "t", 0.0) - local).abs() < 0.5 / self.fps())
            .cloned())
    }
    pub(crate) fn fps(&self) -> f64 {
        self.published
            .snapshot
            .root()
            .and_then(|r| r.get("fps"))
            .map(Node::to_value)
            .and_then(|v| v.as_f64())
            .filter(|n| *n > 0.0)
            .unwrap_or(30.0)
    }
    pub(crate) fn composition(&self) -> Value {
        self.published
            .snapshot
            .root()
            .and_then(|root| model::comp_node(root, model::open_comp_id(root).as_deref()))
            .map(Node::to_value)
            .unwrap_or(Value::Null)
    }
    pub fn select_channel(&self, caller: Caller, path: Option<String>) -> Request {
        Request::new(caller, pm_kernel::SelectionCall::Channel(path))
    }
}
pub(crate) fn array(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
pub(crate) fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
pub(crate) fn number(v: &Value, key: &str, fallback: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(fallback)
}
pub(crate) fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut m = Map::new();
    for (k, v) in entries {
        m.insert(k, v);
    }
    Value::Object(m)
}

/// Semantic scroll anchors leave pixels, row heights and layout to the host.
#[derive(Clone, Debug, Default)]
pub struct ScrollMemory {
    positions: HashMap<Vec<String>, (String, f64)>,
}
impl ScrollMemory {
    pub fn remember(
        &mut self,
        selection: Vec<String>,
        row: String,
        fraction: f64,
    ) -> Result<(), InspectorError> {
        if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
            return Err(InspectorError::InvalidInput);
        }
        self.positions.insert(selection, (row, fraction));
        Ok(())
    }
    pub fn restore(&self, selection: &[String]) -> Option<&(String, f64)> {
        self.positions.get(selection)
    }
}
/// Shipped 15 Hz cadence, exact pause and input refresh (context.ts:101–118).
#[derive(Clone, Debug, Default)]
pub struct ReadoutClock {
    last: Option<f64>,
    pub time: f64,
}
impl ReadoutClock {
    pub fn update(&mut self, now_ms: f64, time: f64, playing: bool, force: bool) -> bool {
        if !now_ms.is_finite() || !time.is_finite() {
            return false;
        }
        if force
            || !playing
            || self
                .last
                .is_none_or(|last| now_ms < last || now_ms - last >= 1000.0 / 15.0)
        {
            self.last = Some(now_ms);
            self.time = time;
            true
        } else {
            false
        }
    }
}

fn eval_error(error: pm_eval::js_sem::JsThrow) -> InspectorError {
    InspectorError::Query(QueryError::Thrown {
        kind: error.kind.to_string(),
        message: error.message,
    })
}

// Object.is semantics of the shipped mixed-value check. Structured values from
// distinct targets are distinct objects; do not conflate signed zero or NaN.
fn same_mixed_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
        }
        (Value::Array(_), Value::Array(_)) | (Value::Object(_), Value::Object(_)) => false,
        _ => a == b,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_value_identity_matches_tag_numbers_and_objects() {
        assert!(same_mixed_value(
            &Value::Number(f64::NAN),
            &Value::Number(f64::NAN)
        ));
        assert!(!same_mixed_value(&Value::Number(-0.0), &Value::Number(0.0)));
        assert!(same_mixed_value(&Value::from("x"), &Value::from("x")));
        assert!(!same_mixed_value(
            &Value::Array(vec![]),
            &Value::Array(vec![])
        ));
    }
}
