//! Nonvisual content behaviors. Services supply font/media facts; Linux tests
//! supply synthetic facts. Inspection never writes data.
use crate::view::{array, number, object, text};
use crate::{Binding, Inspector, InspectorError, PropertyId};
use pm_kernel::commands::properties::{KeyMode, SetProperty};
use pm_kernel::{Caller, ContentCommand, PropertyCommand, Request, TimingCommand};
use pm_model::{Map, Value};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq)]
pub struct FontAxis {
    pub tag: String,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FontFace {
    pub weight: f64,
    pub italic: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontFacts {
    pub axes: Vec<FontAxis>,
    pub faces: Vec<FontFace>,
    pub readable: bool,
}
/// Platform font enumeration/inspection is outside this model.
pub trait FontProvider {
    fn inspect(&self, family: &str, refresh: bool) -> Result<FontFacts, InspectorError>;
}
#[derive(Clone, Debug, PartialEq)]
pub struct AxisState {
    pub axis: FontAxis,
    pub value: Value,
    pub saved: bool,
}
pub use pm_kernel::commands::timing::RetimeMode as RemapMode;

pub fn nearest_face(faces: &[FontFace], weight: f64, italic: bool) -> Option<&FontFace> {
    let same_slope = faces.iter().any(|f| f.italic == italic);
    faces
        .iter()
        .filter(|f| !same_slope || f.italic == italic)
        .min_by(|a, b| {
            (a.weight - weight)
                .abs()
                .total_cmp(&(b.weight - weight).abs())
        })
}
pub fn valid_axis_tag(tag: &str) -> bool {
    tag.len() == 4 && tag.bytes().all(|b| (0x20..=0x7e).contains(&b))
}
pub fn smoothing_detent(value: f64) -> Result<f64, InspectorError> {
    if !value.is_finite() {
        return Err(InspectorError::InvalidInput);
    }
    Ok(if (value - 60.0).abs() <= 2.0 {
        60.0
    } else {
        value.clamp(0.0, 100.0)
    })
}
/// Cycle-safe tag behavior, including the shipped maximum nesting depth.
pub fn precomp_carries_audio(comp: &str, comps: &Value) -> bool {
    fn visit(id: &str, comps: &Value, depth: usize, seen: &HashSet<String>) -> bool {
        if id.is_empty() || seen.contains(id) || depth > 8 {
            return false;
        }
        let Some(comp) = comps.get(id) else {
            return false;
        };
        let mut seen = seen.clone();
        seen.insert(id.into());
        array(comp.get("layers")).iter().any(|layer| {
            let Some(d) = layer.get("d") else {
                return false;
            };
            match text(layer, "type") {
                "audio" => !text(d, "asset").is_empty(),
                "video" => {
                    d.get("embeddedAudio").and_then(Value::as_bool) == Some(true)
                        && !text(d, "asset").is_empty()
                }
                "precomp" => visit(text(d, "comp"), comps, depth + 1, &seen),
                _ => false,
            }
        })
    }
    visit(comp, comps, 0, &HashSet::new())
}
impl Inspector {
    pub fn set_content_patch(
        &mut self,
        caller: Caller,
        layer: &str,
        patch: &Map,
        time: f64,
    ) -> Result<Request, InspectorError> {
        let bindings = patch
            .keys()
            .map(|k| Binding::Content {
                layer: layer.into(),
                field: k.to_string_lossy(),
            })
            .collect::<Vec<_>>();
        let values = patch.values().cloned().collect::<Vec<_>>();
        self.set_many(caller, &bindings, &values, time)
    }
    pub fn independent_corners(
        &mut self,
        caller: Caller,
        layer: &str,
        on: bool,
        time: f64,
    ) -> Result<Request, InspectorError> {
        if text(&self.layer(layer)?, "type") != "shape" {
            return Err(InspectorError::InvalidInput);
        }
        let mut patch = Map::new();
        patch.insert("independentCorners", on.into());
        if on {
            let radius = self
                .binding_state(
                    &Binding::Content {
                        layer: layer.into(),
                        field: "radius".into(),
                    },
                    time,
                )?
                .value
                .as_f64()
                .unwrap_or(0.0);
            for field in ["radiusTL", "radiusTR", "radiusBL", "radiusBR"] {
                patch.insert(field, radius.into());
            }
        }
        // Seeding is absolute even when a selection includes different radii.
        let mut commands = Vec::new();
        let primary = self.layer(layer)?;
        let selected = if self.primary() == Some(layer) {
            self.selected.clone()
        } else {
            vec![layer.into()]
        };
        for target in selected {
            if self.editable(&target)
                && text(&self.layer(&target)?, "type") == text(&primary, "type")
            {
                commands.push(
                    ContentCommand::Set {
                        layer_id: target,
                        patch: patch.clone(),
                        time,
                    }
                    .into(),
                );
            }
        }
        self.transaction(caller, commands)
    }
    pub fn font_axes(
        &mut self,
        layer: &str,
        facts: &FontFacts,
        time: f64,
    ) -> Result<Vec<AxisState>, InspectorError> {
        if facts.axes.iter().any(|a| {
            !a.default.is_finite()
                || a.min.is_some_and(|v| !v.is_finite())
                || a.max.is_some_and(|v| !v.is_finite())
                || matches!((a.min,a.max),(Some(min),Some(max))if min>max)
        }) {
            return Err(InspectorError::InvalidInput);
        }
        let source = self.layer(layer)?;
        if text(&source, "type") != "text" {
            return Err(InspectorError::InvalidInput);
        }
        let mut axes = facts
            .axes
            .iter()
            .filter(|a| valid_axis_tag(&a.tag))
            .cloned()
            .map(|a| (a, false))
            .collect::<Vec<_>>();
        if let Some(d) = source.get("d").and_then(Value::as_object) {
            for (key, prop) in d.iter() {
                let key = key.to_string_lossy();
                if let Some(tag) = key.strip_prefix("fontAxis.")
                    && valid_axis_tag(tag)
                    && prop.get("kf").is_some()
                    && !axes.iter().any(|(a, _)| a.tag == tag)
                {
                    axes.push((
                        FontAxis {
                            tag: tag.into(),
                            default: number(prop, "v", 0.0),
                            min: None,
                            max: None,
                        },
                        true,
                    ));
                }
            }
        }
        let mut out = Vec::new();
        for (axis, saved) in axes {
            let id = PropertyId::new(layer, format!("c.fontAxis.{}", axis.tag));
            let value = if self.property(&id)?.is_some() {
                self.evaluate(&id, time)?
            } else {
                let initial = if axis.tag == "wght" {
                    self.binding_state(
                        &Binding::Content {
                            layer: layer.into(),
                            field: "weight".into(),
                        },
                        time,
                    )?
                    .value
                    .as_f64()
                    .filter(|w| *w != 0.0)
                    .unwrap_or(axis.default)
                    .clamp(
                        axis.min.unwrap_or(f64::NEG_INFINITY),
                        axis.max.unwrap_or(f64::INFINITY),
                    )
                } else {
                    axis.default
                };
                Value::Number(initial)
            };
            out.push(AxisState { axis, value, saved });
        }
        Ok(out)
    }
    pub fn set_axis(
        &mut self,
        caller: Caller,
        layer: &str,
        axis: &FontAxis,
        value: f64,
        time: f64,
        keyframe: bool,
    ) -> Result<Request, InspectorError> {
        if !valid_axis_tag(&axis.tag) || !value.is_finite() || !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        let field = format!("fontAxis.{}", axis.tag);
        let source = self.layer(layer)?;
        if text(&source, "type") != "text" {
            return Err(InspectorError::InvalidInput);
        }
        let binding = Binding::Content {
            layer: layer.into(),
            field: field.clone(),
        };
        let targets = self.binding_targets(&binding)?;
        let mut commands = Vec::new();
        for target in targets {
            let id = target.layer_id().expect("content");
            if self
                .layer(id)?
                .get("d")
                .and_then(|d| d.get(&field))
                .is_none()
            {
                let mut patch = Map::new();
                patch.insert(
                    field.as_str(),
                    object([
                        ("v", axis.default.into()),
                        ("kf", Value::Array(vec![])),
                        ("expr", Value::Null),
                    ]),
                );
                commands.push(
                    ContentCommand::Set {
                        layer_id: id.into(),
                        patch,
                        time,
                    }
                    .into(),
                );
            }
            let mut c = SetProperty::new(id, format!("c.{field}"), value, time);
            c.mode = if keyframe {
                KeyMode::Keyframe
            } else {
                KeyMode::Auto
            };
            c.preserve_hand_edits = false;
            commands.push(PropertyCommand::Set(c).into());
        }
        self.transaction(caller, commands)
    }
    /// Import happens asynchronously in the host. Accept only a registered
    /// asset of the requested kind; cancellation calls no model mutation.
    pub fn imported_source(
        &mut self,
        caller: Caller,
        layer: &str,
        kind: &str,
        asset: &str,
        time: f64,
    ) -> Result<Request, InspectorError> {
        if !["image", "video"].contains(&kind) || text(&self.layer(layer)?, "type") != kind {
            return Err(InspectorError::InvalidInput);
        }
        let root = self
            .published
            .snapshot
            .root()
            .ok_or(InspectorError::InvalidInput)?;
        let actual = root
            .get("assets")
            .and_then(|a| a.get(asset))
            .map(|a| a.to_value())
            .ok_or(InspectorError::InvalidInput)?;
        if text(&actual, "kind") != kind {
            return Err(InspectorError::InvalidInput);
        }
        self.set(
            caller,
            Binding::Content {
                layer: layer.into(),
                field: "asset".into(),
            },
            asset.into(),
            time,
        )
    }
    /// The Time buttons (`retiming.ts`): one kernel step, as the menu and
    /// the agent run it.
    pub fn remap(
        &mut self,
        caller: Caller,
        layer: &str,
        mode: RemapMode,
        time: f64,
    ) -> Result<Request, InspectorError> {
        let source = self.layer(layer)?;
        if !["video", "precomp"].contains(&text(&source, "type")) || !time.is_finite() {
            return Err(InspectorError::InvalidInput);
        }
        self.transaction(
            caller,
            vec![
                TimingCommand::Retime {
                    layer_ids: vec![layer.into()],
                    mode,
                    time,
                }
                .into(),
            ],
        )
    }
}
