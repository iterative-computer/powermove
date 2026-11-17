//! Speed/Duration… (Premiere's Clip ▸ Speed/Duration): a selected video or
//! audio clip's speed as a percentage or its duration (either sets the
//! other), Reverse Speed, Maintain Audio Pitch, and Ripple Edit, Shifting
//! Trailing Clips (on in the Tracks timeline, where the clip keeps its track).
//! v1 had only the Properties Speed field and Time-Reverse Layer.
use crate::{
    dialogs::{DialogId, Field, NativePiece, Surface, WindowId},
    layer_commands::Plan,
    session::Controller,
    sheets::{OpenSheet, from_surface},
};
use gpui::{prelude::*, *};
use pm_kernel::commands::timing::{RetimeMode, TimingCommand};
use pm_model::json::Value;
use std::rc::Rc;

pub const SPEED_ERROR: &str = "Enter a speed from 5 % to 10000 %.";
pub const REMAPPED: &str =
    "The clip is time remapped or its speed is animated: change it in Properties.";

/// The clip the sheet opens on and what it starts from.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub layer_id: String,
    pub name: String,
    /// Constant speed (1 = 100 %), or `None` when remapped or animated.
    pub speed: Option<f64>,
    pub duration: f64,
    pub keep_pitch: bool,
    pub ripple: bool,
}

/// What the person typed.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    pub speed: String,
    pub duration: String,
    pub reverse: bool,
    pub keep_pitch: bool,
    pub ripple: bool,
}

impl Draft {
    /// The first selected unlocked video or audio clip.
    pub fn of(published: &pm_kernel::Published, tracks: bool) -> Option<Self> {
        let root = published.snapshot.root()?;
        let selected = published.selection.layers();
        let info = pm_kernel::model::list_layers(root, None)
            .into_iter()
            .find(|l| {
                selected.contains(&l.id)
                    && matches!(l.kind.as_str(), "video" | "audio")
                    && !l.effectively_locked
            })?;
        let loc = pm_kernel::model::find_layer(root, &info.id)?;
        let layer = pm_kernel::model::layer_at(root, &loc)?.to_value();
        let d = layer.get("d");
        let field = |k: &str| d.and_then(|d| d.get(k));
        let constant = |v: Option<&Value>| match v {
            None | Some(Value::Null) => Some(1.0),
            Some(Value::Number(n)) => Some(*n),
            Some(v) => (v
                .get("kf")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
                && !v.get("expr").is_some_and(Value::is_truthy))
            .then(|| v.get("v").and_then(Value::as_f64))
            .flatten(),
        };
        let remapped = field("timeRemap").is_some_and(|r| match r {
            Value::Bool(b) => *b,
            v => v.get("v").is_some_and(Value::is_truthy),
        });
        Some(Self {
            layer_id: info.id.clone(),
            name: info.name.clone(),
            speed: (!remapped).then(|| constant(field("speed"))).flatten(),
            duration: info.out_time - info.in_time,
            keep_pitch: field("pitch").and_then(Value::as_str) != Some("follow"),
            ripple: tracks,
        })
    }

    pub fn surface(&self, id: DialogId, owner: WindowId) -> Surface {
        let percent = self.speed.map_or(String::new(), |s| trim(s * 100.));
        let mut speed = Field::number("speed", "Speed", vec![percent], "%");
        let mut duration = Field::number("duration", "Duration", vec![trim(self.duration)], "s");
        speed.enabled = self.speed.is_some();
        duration.enabled = self.speed.is_some();
        if self.speed.is_none() {
            speed.tooltip = Some(REMAPPED);
        }
        let checkbox = |id: &str, label: &str, on: bool| {
            Field::new(id, label, NativePiece::Checkbox, vec![on.to_string()])
        };
        let mut s = Surface::sheet(
            id,
            owner,
            "Speed/Duration",
            vec![
                speed,
                duration,
                checkbox("reverse", "Reverse Speed", false),
                checkbox("pitch", "Maintain Audio Pitch", self.keep_pitch),
                checkbox(
                    "ripple",
                    "Ripple Edit, Shifting Trailing Clips",
                    self.ripple,
                ),
            ],
            "Apply",
        );
        s.line = Some(self.name.clone());
        s
    }

    /// The edit the sheet asks for, or an error to show; `None` when nothing
    /// changed. `in_tracks` wraps a speed change made while the timeline
    /// shows tracks, so the clip keeps its track.
    pub fn plan(
        &self,
        fields: &Fields,
        seeded: &Fields,
        time: f64,
        in_tracks: impl Fn(TimingCommand) -> TimingCommand,
    ) -> Result<Option<Plan>, &'static str> {
        let mut commands = vec![];
        if let Some(current) = self.speed {
            let speed = if fields.duration != seeded.duration {
                let d: f64 = fields.duration.trim().parse().map_err(|_| SPEED_ERROR)?;
                (d > 0.)
                    .then(|| current * self.duration / d)
                    .ok_or(SPEED_ERROR)?
            } else {
                let p: f64 = fields.speed.trim().parse().map_err(|_| SPEED_ERROR)?;
                p / 100.
            };
            if !(0.05..=100.).contains(&speed) {
                return Err(SPEED_ERROR);
            }
            let changed = (speed - current).abs() > 1e-9 || fields.keep_pitch != seeded.keep_pitch;
            if changed {
                let command = TimingCommand::SetSpeed {
                    layer_id: self.layer_id.clone(),
                    speed,
                    ripple: fields.ripple,
                    keep_pitch: (fields.keep_pitch != seeded.keep_pitch)
                        .then_some(fields.keep_pitch),
                };
                commands.push(
                    if fields.ripple {
                        in_tracks(command)
                    } else {
                        command
                    }
                    .into(),
                );
            }
        }
        if fields.reverse {
            commands.push(
                TimingCommand::Retime {
                    layer_ids: vec![self.layer_id.clone()],
                    mode: RetimeMode::Reverse,
                    time,
                }
                .into(),
            );
        }
        Ok((!commands.is_empty()).then(|| Plan {
            label: "Speed/Duration".into(),
            commands,
            select: None,
            count_keys: None,
        }))
    }
}

/// A number as a field shows it: up to three decimals, no trailing zeros.
fn trim(n: f64) -> String {
    let s = format!("{n:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

impl Fields {
    fn read(text: impl Fn(&str) -> String) -> Self {
        Self {
            speed: text("speed"),
            duration: text("duration"),
            reverse: text("reverse") == "true",
            keep_pitch: text("pitch") == "true",
            ripple: text("ripple") == "true",
        }
    }
}

impl Controller {
    /// Speed/Duration… on the selected clip.
    pub fn speed_sheet(&mut self, cx: &mut Context<Self>) {
        let (Some(session), Some(published)) = (self.session.clone(), self.published.clone())
        else {
            return;
        };
        let tracks = crate::layer_commands::shows_tracks(&published);
        let Some(draft) = Draft::of(&published, tracks) else {
            self.message = "Select a video or audio clip first.".into();
            cx.notify();
            return;
        };
        let id = DialogId(format!("speed:{}", session.project));
        let owner = WindowId(session.project.clone());
        let surface = draft.surface(id, owner);
        let tokens = self.host.borrow().tokens.clone();
        let open = Rc::new(OpenSheet::present(
            from_surface(&surface, &tokens),
            &self.host.borrow().window,
            surface.first_selected.as_deref(),
        ));
        let actions = open.actions.clone();
        let seeded = Fields::read(|id| open.text(id));
        let time = self.time();
        cx.spawn(async move |this, cx| {
            while let Ok(action) = actions.recv().await {
                match action.as_str() {
                    "cancel" => {
                        open.close();
                        return;
                    }
                    "apply" => {
                        let fields = Fields::read(|id| open.text(id));
                        let planned = draft.plan(&fields, &seeded, time, |c| {
                            crate::layer_commands::in_tracks(&published, c)
                        });
                        open.close();
                        let _ = this.update(cx, |v, cx| match planned {
                            Ok(Some(plan)) => v.run_layer_plan(plan, cx),
                            Ok(None) => {}
                            Err(message) => {
                                v.message = message.into();
                                cx.notify();
                            }
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
    use super::{Draft, Fields, Plan, SPEED_ERROR};
    use crate::dialogs::{DialogId, WindowId};
    use pm_kernel::commands::timing::{RetimeMode, TimingCommand};

    fn draft() -> Draft {
        Draft {
            layer_id: "b".into(),
            name: "B".into(),
            speed: Some(1.0),
            duration: 4.0,
            keep_pitch: true,
            ripple: false,
        }
    }
    fn fields(speed: &str, duration: &str) -> Fields {
        Fields {
            speed: speed.into(),
            duration: duration.into(),
            reverse: false,
            keep_pitch: true,
            ripple: false,
        }
    }
    fn speed_of(plan: &Plan) -> f64 {
        match &plan.commands[0] {
            pm_kernel::Command::Timing(TimingCommand::SetSpeed { speed, .. }) => *speed,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_speed_or_a_duration_sets_the_other() {
        let d = draft();
        let seeded = fields("100", "4");
        let by_speed = d
            .plan(&fields("200", "4"), &seeded, 0., |c| c)
            .unwrap()
            .unwrap();
        assert_eq!(speed_of(&by_speed), 2.0);
        assert_eq!(by_speed.label, "Speed/Duration");
        let by_duration = d
            .plan(&fields("100", "8"), &seeded, 0., |c| c)
            .unwrap()
            .unwrap();
        assert_eq!(speed_of(&by_duration), 0.5);
        assert!(matches!(d.plan(&seeded, &seeded, 0., |c| c), Ok(None)));
        assert!(matches!(
            d.plan(&fields("1", "4"), &seeded, 0., |c| c),
            Err(SPEED_ERROR)
        ));
    }

    #[test]
    fn ripple_in_tracks_wraps_the_speed_change() {
        let d = draft();
        let seeded = fields("100", "4");
        let mut typed = fields("50", "4");
        typed.ripple = true;
        let plan = d
            .plan(&typed, &seeded, 0., |c| TimingCommand::InTracks {
                lanes: vec![],
                command: Box::new(c),
            })
            .unwrap()
            .unwrap();
        assert!(matches!(
            &plan.commands[0],
            pm_kernel::Command::Timing(TimingCommand::InTracks { .. })
        ));
    }

    #[test]
    fn a_remapped_clip_offers_reverse_only() {
        let mut d = draft();
        d.speed = None;
        let s = d.surface(DialogId("s".into()), WindowId("w".into()));
        assert!(!s.fields[0].enabled && !s.fields[1].enabled);
        let mut typed = fields("", "4");
        typed.reverse = true;
        let plan = d
            .plan(&typed, &fields("", "4"), 0., |c| c)
            .unwrap()
            .unwrap();
        assert!(matches!(
            &plan.commands[0],
            pm_kernel::Command::Timing(TimingCommand::Retime {
                mode: RetimeMode::Reverse,
                ..
            })
        ));
    }
}
