//! S029 Keyframe Value, Keyframe Velocity and Scale Keyframes, on the selected keys.
use crate::{
    dialogs::{
        DialogId, MixedNumber, ScaleDraft, VelocityDraft, WindowId,
        kernel::{Context as Dialog, DialogError, PreparedEdit},
        keyframe_value,
    },
    session::Controller,
    sheets::{OpenSheet, from_surface},
};
use gpui::{prelude::*, *};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Value,
    Velocity,
    Scale,
}
impl Kind {
    pub fn of(command: &str) -> Option<Self> {
        match command {
            "keyframeValue" => Some(Self::Value),
            "keyframeVelocity" => Some(Self::Velocity),
            "scaleKeyframes" => Some(Self::Scale),
            _ => None,
        }
    }
}
/// A property's display name from its track id: `position.x` reads `Position X`.
pub fn property_label(track: &str) -> String {
    track
        .rsplit("p.")
        .next()
        .unwrap_or(track)
        .split(['.', '_'])
        .map(|word| {
            let mut letters = word.chars();
            letters.next().map_or(String::new(), |first| {
                first.to_uppercase().chain(letters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
/// Runs off main. `None` when the sheet has nothing to change.
fn prepare(
    context: &Dialog,
    kind: Kind,
    ids: &[String],
    fields: &Fields,
) -> Result<PreparedEdit, DialogError> {
    match kind {
        Kind::Value => context.values(ids, &MixedNumber::parsed_input(&fields.value)),
        Kind::Velocity => context.velocity(ids, &fields.velocity()),
        Kind::Scale => context.scale(
            ids,
            &ScaleDraft {
                time_percent: fields.time.parse().unwrap_or(f64::NAN),
                value_percent: fields.value.parse().unwrap_or(f64::NAN),
                ..ScaleDraft::default()
            },
        ),
    }
}
/// Everything the worker needs from a sheet, read on main.
pub struct Fields {
    value: String,
    time: String,
    incoming: [String; 2],
    outgoing: [String; 2],
}
impl Fields {
    fn read(open: &OpenSheet) -> Self {
        Self {
            value: open.text("value"),
            time: open.text("time"),
            incoming: [open.text("incomingSpeed"), open.text("incomingInfluence")],
            outgoing: [open.text("outgoingSpeed"), open.text("outgoingInfluence")],
        }
    }
    fn velocity(&self) -> VelocityDraft {
        VelocityDraft {
            incoming_speed: MixedNumber::parsed_input(&self.incoming[0]),
            incoming_influence: MixedNumber::parsed_input(&self.incoming[1]),
            outgoing_speed: MixedNumber::parsed_input(&self.outgoing[0]),
            outgoing_influence: MixedNumber::parsed_input(&self.outgoing[1]),
        }
    }
}
impl Controller {
    pub fn keyframe_sheet(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let (Some(session), Some(published)) = (self.session.clone(), self.published.clone())
        else {
            return;
        };
        let ids = published.selection.keys().to_vec();
        if ids.is_empty() {
            self.message = "Select keyframes first.".into();
            cx.notify();
            return;
        }
        let Ok(context) = Dialog::capture(&self.kernel, &session.project) else {
            return;
        };
        let keys = match context.keys(&ids) {
            Ok(keys) if !keys.is_empty() => keys,
            _ => return,
        };
        let id = DialogId(format!("keyframes:{}", session.project));
        let owner = WindowId(session.project.clone());
        let surface = match kind {
            Kind::Value => {
                let values: Vec<f64> = keys.iter().filter_map(|k| k.value.as_f64()).collect();
                keyframe_value(
                    id,
                    owner,
                    &property_label(&keys[0].track),
                    &format!("{:.2} s", keys[0].time),
                    keys.len(),
                    &[MixedNumber::new(&values)],
                    "",
                )
            }
            Kind::Velocity => {
                let number = |values: Vec<f64>| MixedNumber::new(&values);
                VelocityDraft {
                    incoming_speed: number(keys.iter().map(|k| k.incoming.speed).collect()),
                    incoming_influence: number(keys.iter().map(|k| k.incoming.influence).collect()),
                    outgoing_speed: number(keys.iter().map(|k| k.outgoing.speed).collect()),
                    outgoing_influence: number(keys.iter().map(|k| k.outgoing.influence).collect()),
                }
                .surface(id, owner, keys.len(), "/s")
            }
            Kind::Scale => ScaleDraft::default().surface(id, owner),
        };
        let tokens = self.host.borrow().tokens.clone();
        let open = OpenSheet::present(
            from_surface(&surface, &tokens),
            &self.host.borrow().window,
            surface.first_selected.as_deref(),
        );
        let (actions, kernel) = (open.actions.clone(), self.kernel.clone());
        let open = Rc::new(open);
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                match action.as_str() {
                    "cancel" => {
                        open.close();
                        return;
                    }
                    "apply" => {
                        let fields = Fields::read(&open);
                        let (context, kernel, ids) = (context.clone(), kernel.clone(), ids.clone());
                        let result = cx
                            .background_executor()
                            .spawn(async move {
                                let edit = prepare(&context, kind, &ids, &fields)?;
                                edit.submit(&kernel, 0)
                                    .wait()
                                    .ok_or(DialogError::Stale)?
                                    .map_err(|e| DialogError::Kernel(format!("{e:?}")))?;
                                Ok::<_, DialogError>(())
                            })
                            .await;
                        open.close();
                        let _ = this.update(cx, |v, cx| match result {
                            Ok(()) => v.refresh(cx),
                            Err(error) => v.report_dialog_error(error, cx),
                        });
                        return;
                    }
                    _ => {}
                }
            }
        })
        .detach();
    }
}
#[cfg(test)]
mod tests {
    use super::{Kind, property_label};
    #[test]
    fn commands_map_to_their_sheets() {
        assert_eq!(Kind::of("keyframeValue"), Some(Kind::Value));
        assert_eq!(Kind::of("keyframeVelocity"), Some(Kind::Velocity));
        assert_eq!(Kind::of("scaleKeyframes"), Some(Kind::Scale));
        assert_eq!(Kind::of("delete"), None);
    }
    #[test]
    fn property_labels_read_like_the_inspector() {
        assert_eq!(property_label("position.x"), "Position X");
        assert_eq!(property_label("p.opacity"), "Opacity");
    }
}
