//! Layer relations and cross-panel service requests, addressed only by id.
use crate::view::{object, text};
use crate::{Inspector, InspectorError, PropertyId, ServiceIntent};
use pm_kernel::commands::layers::{LayerPatch, ParentChange};
use pm_kernel::{Caller, LayerCommand, Request, SelectionCall};
use pm_model::Value;

#[derive(Clone, Debug)]
pub struct ParentPick {
    document: u64,
    targets: Vec<String>,
    time: f64,
}
impl Inspector {
    fn parenting_layers(&self, layer: &str) -> Result<Vec<String>, InspectorError> {
        self.layer_node(layer)?;
        let ids = if self.published.selection.contains(layer) {
            self.published.selection.layers().to_vec()
        } else {
            vec![layer.into()]
        };
        Ok(ids
            .into_iter()
            .filter(|id| self.layer(id).is_ok_and(|l| text(&l, "type") != "audio"))
            .collect())
    }
    fn relation_query(
        &mut self,
        op: &str,
        layer: &str,
        key: &str,
        value: Value,
    ) -> Result<bool, InspectorError> {
        let root = self
            .published
            .snapshot
            .root()
            .ok_or(InspectorError::InvalidInput)?;
        let loc = pm_kernel::model::find_layer(root, layer)
            .ok_or_else(|| InspectorError::MissingLayer(layer.into()))?;
        let mut request = object([
            ("op", op.into()),
            ("layer", (loc.index as f64).into()),
            ("comp", loc.comp_id.unwrap_or_default().into()),
        ]);
        request.as_object_mut().expect("object").insert(key, value);
        self.mirror
            .queries()
            .query(&request)?
            .as_bool()
            .ok_or(InspectorError::InvalidInput)
    }
    pub fn parent_candidates(&mut self, layer: &str) -> Result<Vec<String>, InspectorError> {
        let children = self.parenting_layers(layer)?;
        let root = self
            .published
            .snapshot
            .root()
            .ok_or(InspectorError::InvalidInput)?;
        let ids = pm_kernel::model::list_layers(root, None)
            .into_iter()
            .map(|l| l.id)
            .collect::<Vec<_>>();
        let mut out = Vec::new();
        for id in ids {
            if text(&self.layer(&id)?, "type") == "audio" || children.contains(&id) {
                continue;
            }
            let mut cyclic = false;
            for child in &children {
                if self.relation_query("wouldCycle", child, "parent", id.clone().into())? {
                    cyclic = true;
                    break;
                }
            }
            if !cyclic {
                out.push(id);
            }
        }
        Ok(out)
    }
    pub fn matte_candidates(&mut self, layer: &str) -> Result<Vec<String>, InspectorError> {
        let root = self
            .published
            .snapshot
            .root()
            .ok_or(InspectorError::InvalidInput)?;
        let ids = pm_kernel::model::list_layers(root, None)
            .into_iter()
            .map(|l| l.id)
            .collect::<Vec<_>>();
        let mut out = Vec::new();
        for id in ids {
            if self.relation_query("validMatteSource", layer, "id", id.clone().into())? {
                out.push(id);
            }
        }
        Ok(out)
    }
    /// Captures the exact raw selection (including selected members), matching
    /// LayerOptions parenting, which intentionally differs from shared edits.
    pub fn begin_parent_pick(&self, layer: &str, time: f64) -> Result<ParentPick, InspectorError> {
        if !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        Ok(ParentPick {
            document: self.published.snapshot.doc_id,
            targets: self.parenting_layers(layer)?,
            time,
        })
    }
    /// Escape/cancel returns no request and changes neither document nor history.
    pub fn finish_parent_pick(
        &self,
        caller: Caller,
        pick: ParentPick,
        parent: Option<String>,
        cancel: bool,
    ) -> Result<Option<Request>, InspectorError> {
        if cancel {
            return Ok(None);
        }
        if pick.document != self.published.snapshot.doc_id {
            return Err(InspectorError::InvalidInput);
        }
        let commands = pick
            .targets
            .into_iter()
            .map(|id| {
                LayerCommand::Set {
                    layer_id: id,
                    patch: LayerPatch {
                        parent: Some(ParentChange {
                            parent: parent.clone(),
                            time: pick.time,
                        }),
                        ..Default::default()
                    },
                }
                .into()
            })
            .collect();
        self.transaction(caller, commands).map(Some)
    }
    pub fn graph_intents(
        &self,
        id: PropertyId,
        paired_scale: bool,
        open: bool,
    ) -> Result<Vec<ServiceIntent>, InspectorError> {
        self.layer_node(&id.layer)?;
        let mut paths = vec![id.clone()];
        if paired_scale && id.path == "scale.x" {
            paths.push(PropertyId::new(&id.layer, "scale.y"));
        }
        let mut intents = vec![if open {
            ServiceIntent::OpenGraph(id.clone())
        } else {
            ServiceIntent::FocusGraph(id)
        }];
        if open {
            intents.push(ServiceIntent::Reveal(paths));
        }
        Ok(intents)
    }
    pub fn select_group_contents(
        &self,
        caller: Caller,
        group: &str,
    ) -> Result<Request, InspectorError> {
        let source = self.layer_node(group)?;
        if source
            .get("type")
            .map(|n| n.to_value())
            .and_then(|v| v.as_str().map(str::to_owned))
            .as_deref()
            != Some("group")
        {
            return Err(InspectorError::InvalidInput);
        }
        let root = self
            .published
            .snapshot
            .root()
            .ok_or(InspectorError::InvalidInput)?;
        let comp =
            pm_kernel::model::comp_node(root, pm_kernel::model::open_comp_id(root).as_deref())
                .ok_or(InspectorError::InvalidInput)?;
        let ids = pm_kernel::model::layers_of(comp)
            .filter(|layer| {
                pm_kernel::model::group_ancestors(comp, layer)
                    .iter()
                    .any(|g| g.get("id").map(|n| n.to_value()) == Some(group.into()))
            })
            .filter_map(|layer| {
                layer
                    .get("id")
                    .map(|n| n.to_value())
                    .and_then(|v| v.as_str().map(str::to_owned))
            })
            .collect();
        Ok(Request::new(
            caller,
            SelectionCall::Layers { ids, add: false },
        ))
    }
    pub fn effect_parameters(
        &self,
        layer: &str,
        effect: &str,
    ) -> Result<Vec<(PropertyId, crate::ParamDefinition)>, InspectorError> {
        let fx = self.effect(layer, effect)?;
        let Some(definition) = self.definitions.get(text(&fx, "type")) else {
            return Err(InspectorError::MissingDefinition(text(&fx, "type").into()));
        };
        Ok(definition
            .params
            .iter()
            .filter(|p| {
                fx.get("p")
                    .and_then(|m| m.get(&p.key))
                    .is_some_and(|v| !v.is_null())
            })
            .cloned()
            .map(|p| (PropertyId::new(layer, format!("{effect}.{}", p.key)), p))
            .collect())
    }
}
