//! Platform-neutral native input/drive envelope. No callbacks or main-thread waits.
use crate::{
    Id, Surface,
    interaction::{EventPlan, Interaction},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift_key: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_key: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_key: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctrl_key: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub button: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buttons: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub click_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<(usize, usize)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composition: Option<(usize, usize)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drag: Option<DragData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<(usize, usize)>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DragData {
    pub types: Vec<String>,
    pub data: BTreeMap<String, Value>,
    pub effect: DragEffect,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DragEffect {
    None,
    Copy,
    Move,
    Link,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeEvent {
    pub generation: u64,
    pub plan: EventPlan,
    pub r#type: String,
    pub detail: EventDetail,
}
impl Interaction {
    /// Called by an agent or AppKit adapter before enqueueing JS. Rejections change no input.
    #[allow(clippy::too_many_arguments)]
    pub fn drive(
        &mut self,
        surface: &mut Surface,
        generation: u64,
        expected_generation: u64,
        revision: u64,
        target: Id,
        event: &str,
        detail: EventDetail,
    ) -> Result<NativeEvent, String> {
        if generation == 0 || generation != expected_generation || revision != surface.revision() {
            return Err("stale native drive".into());
        }
        if [
            detail.pointer_id,
            detail.button,
            detail.buttons,
            detail.click_count,
        ]
        .into_iter()
        .flatten()
        .any(|v| v > 9_007_199_254_740_991)
        {
            return Err("unsafe event integer".into());
        }
        if [detail.x, detail.y, detail.delta_x, detail.delta_y]
            .into_iter()
            .flatten()
            .any(|v| !v.is_finite())
        {
            return Err("nonfinite event coordinate".into());
        }
        if [detail.range, detail.selection, detail.composition]
            .into_iter()
            .flatten()
            .any(|(a, b)| a > b || b > 9_007_199_254_740_991)
            || (detail.composition.is_some() && detail.composing != Some(true))
        {
            return Err("invalid event range/composition".into());
        }
        let plan = self.plan(
            surface,
            target,
            detail.pointer_id,
            event,
            detail.key.as_deref(),
        )?;
        if let Some(value) = &detail.value
            && surface.node(plan.target).is_some_and(|n| n.input.is_some())
            && !(event == "change"
                && value.is_number()
                && surface.node(plan.target).is_some_and(|n| {
                    n.props.get("control").and_then(Value::as_str) == Some("number")
                }))
        {
            let value = value.as_str().ok_or("input observation must be string")?;
            let len = value.encode_utf16().count();
            surface.observe_input_composition(
                plan.target,
                value.to_owned(),
                detail.selection.unwrap_or((len, len)),
                detail.composing.unwrap_or(false),
                detail.composition,
            )?;
        }
        if matches!(event, "pointerup" | "pointercancel")
            && let Some(pointer) = detail.pointer_id
        {
            self.release(pointer);
        }
        Ok(NativeEvent {
            generation,
            plan,
            r#type: event.into(),
            detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commit, Op};
    fn input() -> Surface {
        let mut surface = Surface::default();
        surface
            .apply(Commit {
                base_revision: 0,
                ops: vec![
                    Op::Create {
                        id: 1,
                        r#type: "input".into(),
                    },
                    Op::InsertBefore {
                        parent: 0,
                        id: 1,
                        before: None,
                    },
                ],
            })
            .unwrap();
        surface
    }
    #[test]
    fn utf16_composition_and_echo_are_exact() {
        let mut surface = input();
        let mut interaction = Interaction::default();
        let detail = EventDetail {
            value: Some(Value::String("A日本🙂B".into())),
            selection: Some((3, 5)),
            composing: Some(true),
            composition: Some((1, 3)),
            meta_key: Some(true),
            ..Default::default()
        };
        let event = interaction
            .drive(&mut surface, 4, 4, 1, 1, "input", detail.clone())
            .unwrap();
        assert_eq!(event.detail, detail);
        let before = surface.node(1).unwrap().input.clone();
        surface.apply_json(r#"{"baseRevision":1,"ops":[{"op":"setProps","id":1,"patch":{"value":"A日本🙂B"}}]}"#).unwrap();
        assert_eq!(surface.node(1).unwrap().input, before);
        assert_eq!(surface.revision(), 2);
    }
    #[test]
    fn invalid_range_and_stale_generation_are_atomic() {
        let mut surface = input();
        let before = surface.nodes().clone();
        let mut interaction = Interaction::default();
        for (generation, selection, composition) in [
            (2, (0, 0), None),
            (1, (0, 4), None),
            (1, (0, 0), Some((2, 1))),
        ] {
            let detail = EventDetail {
                value: Some(Value::String("🙂".into())),
                selection: Some(selection),
                composing: Some(true),
                composition,
                ..Default::default()
            };
            assert!(
                interaction
                    .drive(&mut surface, generation, 1, 1, 1, "input", detail)
                    .is_err()
            );
            assert_eq!(surface.nodes(), &before);
        }
    }
    #[test]
    fn pointer_capture_routes_and_releases_typed_drag() {
        let mut surface = input();
        let mut interaction = Interaction::default();
        interaction.capture(&surface, 7, 1).unwrap();
        let detail: EventDetail = serde_json::from_str(r#"{"pointerId":7,"x":1.25,"y":-3,"button":0,"buttons":1,"clickCount":2,"ctrlKey":true,"drag":{"types":["effect"],"data":{"effect":"test"},"effect":"copy"}}"#).unwrap();
        let mut malformed = detail.clone();
        malformed.selection = Some((2, 1));
        assert!(
            interaction
                .drive(&mut surface, 1, 1, 1, 0, "pointercancel", malformed)
                .is_err()
        );
        assert_eq!(interaction.captured(7), Some(1));
        let event = interaction
            .drive(&mut surface, 1, 1, 1, 0, "pointerup", detail)
            .unwrap();
        assert_eq!(event.plan.target, 1);
        assert_eq!(interaction.captured(7), None);
        assert!(serde_json::from_str::<EventDetail>(r#"{"unknown":1}"#).is_err());
    }
}
