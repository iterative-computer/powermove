//! Thin v4-host adapter. All document work still uses Inspector and the actor.
//!
//! The application owns generation routing, presentation descriptors, menu/font
//! providers, actual acknowledgements and publication. A handle is a capability
//! allocated here, never a parsed JS property path. Keep one adapter per realm.
use std::collections::HashMap;

use crate::{
    Binding, EditSession, FieldState, Inspector, InspectorError, PropertyAction, ServiceIntent,
    Value,
};
use pm_kernel::{Caller, Request};

#[derive(Clone, Debug)]
struct Entry {
    bindings: Vec<Binding>,
    selection: std::sync::Arc<[String]>,
    document: u64,
}
#[derive(Default)]
pub struct V4Adapter {
    next: u64,
    entries: HashMap<String, Entry>,
    selection: Option<(u64, std::sync::Arc<[String]>)>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum V4Action {
    Diamond,
    RemoveAnimation,
    AddKey,
    Expression(Option<String>),
    Reset(Option<Value>),
    LinkScale,
    ShowGraph,
}
#[derive(Debug)]
pub enum V4Plan {
    Request(Request),
    Service(Vec<ServiceIntent>),
}
impl V4Adapter {
    /// Register an opaque immutable address set. New selection uses fresh handles;
    /// late control events cannot follow a reused semantic row id to another layer.
    pub fn register(
        &mut self,
        model: &Inspector,
        bindings: Vec<Binding>,
    ) -> Result<String, InspectorError> {
        if bindings.is_empty() {
            return Err(InspectorError::InvalidInput);
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or(InspectorError::InvalidInput)?;
        let handle = format!("inspector:{}", self.next);
        if !self
            .selection
            .as_ref()
            .is_some_and(|(document, selection)| {
                *document == model.document_id() && selection.as_ref() == model.selected_layers()
            })
        {
            self.selection = Some((model.document_id(), model.selected_layers().into()));
        }
        let selection = self
            .selection
            .as_ref()
            .ok_or(InspectorError::InvalidInput)?
            .1
            .clone();
        self.entries.insert(
            handle.clone(),
            Entry {
                bindings,
                selection,
                document: model.published.snapshot.doc_id,
            },
        );
        Ok(handle)
    }
    pub fn dispose(&mut self, handle: &str) {
        self.entries.remove(handle);
    }
    pub fn retire(&mut self) {
        self.entries.clear();
        self.selection = None;
    }
    fn entry(&self, model: &Inspector, handle: &str) -> Result<&Entry, InspectorError> {
        let entry = self
            .entries
            .get(handle)
            .ok_or(InspectorError::InvalidInput)?;
        if entry.selection.as_ref() != model.selected_layers()
            || entry.document != model.published.snapshot.doc_id
        {
            return Err(InspectorError::InvalidInput);
        }
        Ok(entry)
    }
    pub fn read(
        &self,
        model: &mut Inspector,
        handle: &str,
        time: f64,
    ) -> Result<Vec<FieldState>, InspectorError> {
        let bindings = self.entry(model, handle)?.bindings.clone();
        bindings
            .iter()
            .map(|binding| model.binding_state(binding, time))
            .collect()
    }
    /// A typed value is absolute; the native live driver uses begin/step below
    /// for from-start deltas. Neither path fans out edits in JS.
    pub fn set(
        &self,
        model: &mut Inspector,
        caller: Caller,
        handle: &str,
        values: &[Value],
        time: f64,
    ) -> Result<Request, InspectorError> {
        let bindings = self.entry(model, handle)?.bindings.clone();
        model.set_many(caller, &bindings, values, time)
    }
    pub fn begin(
        &self,
        model: &mut Inspector,
        caller: Caller,
        handle: &str,
        time: f64,
    ) -> Result<(EditSession, Request), InspectorError> {
        let bindings = self.entry(model, handle)?.bindings.clone();
        model.begin_edit(caller, &bindings, time)
    }
    pub fn action(
        &self,
        model: &mut Inspector,
        caller: Caller,
        handle: &str,
        action: V4Action,
        time: f64,
    ) -> Result<V4Plan, InspectorError> {
        let bindings = self.entry(model, handle)?.bindings.clone();
        let mut ids = bindings
            .iter()
            .filter_map(Binding::property_id)
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return Err(InspectorError::InvalidInput);
        }
        if action == V4Action::ShowGraph {
            return Ok(V4Plan::Service(model.graph_intents(
                ids[0].clone(),
                bindings.len() > 1,
                true,
            )?));
        }
        if action == V4Action::LinkScale {
            if !ids
                .iter()
                .all(|id| matches!(id.path.as_str(), "scale.x" | "scale.y"))
            {
                return Err(InspectorError::InvalidInput);
            }
            let layer = &ids[0].layer;
            let linked = model
                .layer(layer)?
                .get("scaleLinked")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            return model
                .set(
                    caller,
                    Binding::Layer {
                        layer: layer.clone(),
                        field: "scaleLinked".into(),
                    },
                    (!linked).into(),
                    time,
                )
                .map(V4Plan::Request);
        }
        let property_action = match action {
            V4Action::Diamond => PropertyAction::Diamond,
            V4Action::AddKey => PropertyAction::AddKey,
            V4Action::RemoveAnimation => {
                // Stopwatch on a static property would ADD animation. Option
                // click and the explicit menu action must only remove it.
                let mut animated = Vec::new();
                for binding in &bindings {
                    for target in model.binding_targets(binding)? {
                        if let Some(id) = target.property_id()
                            && model
                                .property(&id)?
                                .is_some_and(|prop| !crate::view::array(prop.get("kf")).is_empty())
                        {
                            animated.push(id);
                        }
                    }
                }
                if animated.is_empty() {
                    return Err(InspectorError::EmptyEdit);
                }
                // Make the model's disable decision see animated members even
                // when the primary member is static. Its target set deduplicates.
                ids.extend(animated);
                PropertyAction::Stopwatch
            }
            V4Action::Expression(value) => PropertyAction::Expression(value),
            V4Action::Reset(value) => PropertyAction::Reset(value),
            V4Action::ShowGraph | V4Action::LinkScale => unreachable!(),
        };
        model
            .property_action(caller, &ids, property_action, time)
            .map(V4Plan::Request)
    }
}
