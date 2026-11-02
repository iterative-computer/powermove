//! Input routes use shipped command IDs, modifiers and repeat/field rules.
//! v1.2.0 keymap.ts:10–20, 63–90, 109–151; viewer.ts:1594–1622;
//! source-preview.ts:270–291. Resolver priority/extension override stays shell-owned.
use crate::*;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub meta: bool,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct KeyEvent<'a> {
    pub key: &'a str,
    pub modifiers: Modifiers,
    pub repeat: bool,
    pub in_field: bool,
    pub cancelled: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyContext {
    pub pointer_over: bool,
    pub source_active: bool,
    pub source_slider: bool,
    pub active_drag: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub enum KeyRoute {
    Declined,
    CancelDrag,
    Action(Action),
    Command {
        id: &'static str,
        args: Vec<Value>,
        temporary_hand: bool,
    },
}
impl KeyRoute {
    pub fn action(self) -> Option<Action> {
        match self {
            Self::Action(a) => Some(a),
            Self::Command { id, args, .. } => Some(Action::Host(HostAction::Command {
                id: id.into(),
                args,
            })),
            _ => None,
        }
    }
}
use pm_model::Value;
pub fn resolve_key(event: KeyEvent<'_>, context: KeyContext) -> KeyRoute {
    if event.cancelled {
        return KeyRoute::Declined;
    }
    let key = event.key;
    let m = event.modifiers;
    if key == "escape" && context.active_drag {
        return KeyRoute::CancelDrag;
    }
    if key == "escape" && context.source_active {
        return KeyRoute::Action(Action::Host(HostAction::Source(SourceCommand::Close)));
    }
    if context.source_slider {
        let command = match key {
            "left" => Some(SourceCommand::StepSeconds(-5.0)),
            "right" => Some(SourceCommand::StepSeconds(5.0)),
            "home" => Some(SourceCommand::Start),
            "end" => Some(SourceCommand::End),
            _ => None,
        };
        if let Some(command) = command {
            return KeyRoute::Action(Action::Host(HostAction::Source(command)));
        }
    }
    if event.in_field {
        return KeyRoute::Declined;
    }
    let command = |id, args, temporary_hand| KeyRoute::Command {
        id,
        args,
        temporary_hand,
    };
    if key == "space" {
        return if event.repeat {
            KeyRoute::Declined
        } else {
            command("play", vec![], context.pointer_over)
        };
    }
    // Strict primary-modifier bindings have exactly one primary modifier.
    if (m.meta || m.control) && !(m.meta && m.control) && !m.alt {
        let route = match (key, m.shift) {
            ("=", false) | ("+", true) => Some(("zoomIn", vec![], false)),
            ("-", false) => Some(("zoomOut", vec![], false)),
            ("0", false) => Some(("fitComposition", vec![], false)),
            ("1", false) => Some(("actualSize", vec![], false)),
            ("h", true) => Some(("toggleLayerControls", vec![], false)),
            ("left", true) => Some(("stepFrames", vec![Value::Number(-10.0)], true)),
            ("right", true) => Some(("stepFrames", vec![Value::Number(10.0)], true)),
            ("left", false) => Some((
                if m.meta {
                    "prevFrame"
                } else {
                    "timeline.adjacentKeyframe:prev"
                },
                vec![],
                m.meta,
            )),
            ("right", false) => Some((
                if m.meta {
                    "nextFrame"
                } else {
                    "timeline.adjacentKeyframe:next"
                },
                vec![],
                m.meta,
            )),
            _ => None,
        };
        if let Some((id, args, repeatable)) = route {
            return if event.repeat && !repeatable {
                KeyRoute::Declined
            } else {
                command(id, args, false)
            };
        }
    }
    if m.meta || m.control || m.alt {
        return KeyRoute::Declined;
    }
    let route = match (key, m.shift) {
        ("home", false) => ("gotoStart", vec![], false),
        ("end", false) => ("gotoEnd", vec![], false),
        ("home", true) => ("gotoWorkIn", vec![], false),
        ("end", true) => ("gotoWorkOut", vec![], false),
        ("pageup", false) => ("prevFrame", vec![], true),
        ("pagedown", false) => ("nextFrame", vec![], true),
        ("pageup", true) => ("stepFrames", vec![Value::Number(-10.0)], true),
        ("pagedown", true) => ("stepFrames", vec![Value::Number(10.0)], true),
        ("j", false) => ("prevVisibleEvent", vec![], false),
        ("k", false) => ("nextVisibleEvent", vec![], false),
        ("j", true) => ("timeline.adjacentKeyframe:prev", vec![], false),
        ("k", true) => ("timeline.adjacentKeyframe:next", vec![], false),
        ("i", false) => ("gotoLayerIn", vec![], false),
        ("o", false) => ("gotoLayerOut", vec![], false),
        ("b", _) => ("workIn", vec![], false),
        ("n", _) => ("workOut", vec![], false),
        (".", false) => ("zoomIn", vec![], false),
        (",", false) => ("zoomOut", vec![], false),
        ("/", false) => ("actualSize", vec![], false),
        ("/", true) | ("?", true) => ("fitComposition", vec![], false),
        ("enter", false) => ("text.editSelected", vec![], false),
        _ => return KeyRoute::Declined,
    };
    if event.repeat && !route.2 {
        KeyRoute::Declined
    } else {
        command(route.0, route.1, false)
    }
}
/// Map shipped command IDs into model actions after shell registry arbitration.
/// Unsupported commands retain their exact IDs for the owning panel/service.
pub fn command_action(id: &str, args: &[Value], viewport: Viewport) -> Result<Action, Error> {
    let no_args = || {
        if args.is_empty() {
            Ok(())
        } else {
            Err(Error::Invalid("command arguments"))
        }
    };
    let transport = match id {
        "play" => Some(TransportAction::Toggle),
        "gotoStart" => Some(TransportAction::Start),
        "gotoEnd" => Some(TransportAction::End),
        "gotoWorkIn" => Some(TransportAction::WorkIn),
        "gotoWorkOut" => Some(TransportAction::WorkOut),
        "prevFrame" => Some(TransportAction::Step(-1)),
        "nextFrame" => Some(TransportAction::Step(1)),
        _ => None,
    };
    if let Some(action) = transport {
        no_args()?;
        return Ok(Action::Transport(action));
    }
    Ok(match id {
        "stepFrames" => {
            if args.len() != 1 {
                return Err(Error::Invalid("frame count"));
            }
            let n = args[0].as_f64().ok_or(Error::Invalid("frame count"))?;
            if !n.is_finite()
                || n == 0.0
                || n.fract() != 0.0
                || n < f64::from(i32::MIN)
                || n > f64::from(i32::MAX)
            {
                return Err(Error::Invalid("frame count"));
            }
            Action::Transport(TransportAction::Step(n as i32))
        }
        "zoomIn" => {
            no_args()?;
            Action::ZoomBy(1.25, viewport)
        }
        "zoomOut" => {
            no_args()?;
            Action::ZoomBy(0.8, viewport)
        }
        "actualSize" => {
            no_args()?;
            Action::SetZoom(1.0)
        }
        "fitComposition" => {
            no_args()?;
            Action::Fit
        }
        "toggleLayerControls" => {
            no_args()?;
            Action::ToggleControls
        }
        "workIn" => {
            no_args()?;
            Action::WorkIn
        }
        "workOut" => {
            no_args()?;
            Action::WorkOut
        }
        _ => Action::Host(HostAction::Command {
            id: id.into(),
            args: args.to_vec(),
        }),
    })
}
/// Ephemeral hand exposure does not consume Space's global play command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerState {
    pub over: bool,
    pub temporary_hand: bool,
}
impl PointerState {
    pub fn key_down(&mut self, event: KeyEvent<'_>) {
        if event.key == "space" && self.over && !event.repeat && !event.in_field && !event.cancelled
        {
            self.temporary_hand = true;
        }
    }
    pub fn key_up(&mut self, key: &str) {
        if key == "space" {
            self.temporary_hand = false;
        }
    }
    /// Lost focus/capture cannot leave the Hand tool stuck.
    pub fn blur(&mut self) {
        self.temporary_hand = false;
        self.over = false;
    }
}
