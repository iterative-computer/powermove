//! W079: kernel-side shuttle facts. Pure transitions, separate from document
//! undo. A host stores the result in UiState.transport via StateTransaction.
use super::pro_support::{invalid, number};
use crate::CommandError;
use pm_model::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShuttleRate {
    Reverse2,
    Reverse1,
    Stopped,
    Forward1,
    Forward2,
}
impl ShuttleRate {
    pub fn factor(self) -> i8 {
        match self {
            Self::Reverse2 => -2,
            Self::Reverse1 => -1,
            Self::Stopped => 0,
            Self::Forward1 => 1,
            Self::Forward2 => 2,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayRange {
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShuttleState {
    pub available: bool,
    pub time: f64,
    pub rate: ShuttleRate,
    pub range: Option<PlayRange>,
    pub fps: f64,
    pub return_time: Option<f64>,
}
impl Default for ShuttleState {
    fn default() -> Self {
        Self {
            available: false,
            time: 0.0,
            rate: ShuttleRate::Stopped,
            range: None,
            fps: 30.0,
            return_time: None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShuttleInput {
    J,
    K,
    L,
    /// Explicit reverse start, including paused state (Control-J).
    Reverse,
    /// Control-Shift-Space. Host chooses the play-around extent in seconds.
    PlayAround {
        before: f64,
        after: f64,
        duration: f64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShuttleOutcome {
    Declined,
    Handled(ShuttleState),
}
impl ShuttleState {
    /// J/K/L decline when the clock is unavailable or paused; the registry
    /// may then run the shipped binding exactly once. Reverse/around may start
    /// from paused. A throw is an error, never a request to replay another key.
    pub fn dispatch(self, input: ShuttleInput) -> Result<ShuttleOutcome, CommandError> {
        if !self.available
            || self.rate == ShuttleRate::Stopped
                && matches!(input, ShuttleInput::J | ShuttleInput::K | ShuttleInput::L)
        {
            return Ok(ShuttleOutcome::Declined);
        }
        self.validate()?;
        let mut next = self;
        match input {
            ShuttleInput::J => {
                next.rate = if matches!(self.rate, ShuttleRate::Reverse1 | ShuttleRate::Reverse2) {
                    ShuttleRate::Reverse2
                } else {
                    ShuttleRate::Reverse1
                };
                next.range = None;
                next.return_time = None;
            }
            ShuttleInput::L => {
                next.rate = if matches!(self.rate, ShuttleRate::Forward1 | ShuttleRate::Forward2) {
                    ShuttleRate::Forward2
                } else {
                    ShuttleRate::Forward1
                };
                next.range = None;
                next.return_time = None;
            }
            ShuttleInput::K => {
                next.rate = ShuttleRate::Stopped;
                next.time = number(self.time * self.fps)?.round() / self.fps;
                next.range = None;
                next.return_time = None;
            }
            ShuttleInput::Reverse => {
                next.rate = ShuttleRate::Reverse1;
                next.range = None;
                next.return_time = None;
            }
            ShuttleInput::PlayAround {
                before,
                after,
                duration,
            } => {
                if number(before)? < 0.0
                    || number(after)? < 0.0
                    || number(duration)? <= 0.0
                    || self.time > duration
                {
                    return Err(invalid("Invalid play-around range."));
                }
                let range = PlayRange {
                    start: (self.time - before).max(0.0),
                    end: (self.time + after).min(duration),
                };
                if range.end <= range.start {
                    return Err(invalid("Play-around range must have positive duration."));
                }
                next.time = range.start;
                next.rate = ShuttleRate::Forward1;
                next.range = Some(range);
                next.return_time = Some(self.time);
            }
        }
        Ok(ShuttleOutcome::Handled(next))
    }
    /// Consume the engine's observed clock; stop at boundaries and return to
    /// the original frame after play-around. This does not synthesize a clock.
    pub fn observe_clock(self, time: f64, duration: f64) -> Result<Self, CommandError> {
        self.validate()?;
        if number(duration)? <= 0.0 {
            return Err(invalid("Transport duration must be positive."));
        }
        number(time)?;
        let bounds = self.range.unwrap_or(PlayRange {
            start: 0.0,
            end: duration,
        });
        if bounds.end > duration || self.return_time.is_some_and(|t| t > duration) {
            return Err(invalid("Transport range exceeds the composition."));
        }
        let mut next = self;
        next.time = time.clamp(bounds.start, bounds.end);
        let stopped = self.rate.factor() < 0 && time <= bounds.start
            || self.rate.factor() > 0 && time >= bounds.end;
        if stopped {
            next.rate = ShuttleRate::Stopped;
            next.time = self.return_time.unwrap_or(next.time);
            next.range = None;
            next.return_time = None;
        }
        Ok(next)
    }
    /// Read kernel UI facts. Runtime capability and composition FPS are supplied
    /// by the host, so reopening old sessions never asserts clock availability.
    pub fn from_transport(v: &Value, available: bool, fps: f64) -> Result<Self, CommandError> {
        let time = number(v.get("time").and_then(Value::as_f64).unwrap_or(0.0))?;
        if time < 0.0 || number(fps)? <= 0.0 {
            return Err(invalid("Invalid transport time or frame rate."));
        }
        let playing = v.get("playing").and_then(Value::as_bool) == Some(true);
        let rate = if !playing {
            ShuttleRate::Stopped
        } else {
            match v.get("rate").and_then(Value::as_f64).unwrap_or(1.0) {
                -2.0 => ShuttleRate::Reverse2,
                -1.0 => ShuttleRate::Reverse1,
                1.0 => ShuttleRate::Forward1,
                2.0 => ShuttleRate::Forward2,
                _ => return Err(invalid("Unsupported shuttle rate.")),
            }
        };
        let range = match v.get("playRange") {
            None | Some(Value::Null) => None,
            Some(r) => {
                let start = number(
                    r.get("start")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| invalid("Missing play range start."))?,
                )?;
                let end = number(
                    r.get("end")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| invalid("Missing play range end."))?,
                )?;
                if start < 0.0 || end <= start {
                    return Err(invalid("Invalid play range."));
                }
                Some(PlayRange { start, end })
            }
        };
        let return_time = v
            .get("returnTime")
            .filter(|v| *v != &Value::Null)
            .map(|v| number(v.as_f64().ok_or_else(|| invalid("Invalid return time."))?))
            .transpose()?;
        if return_time.is_some_and(|t| t < 0.0) {
            return Err(invalid("Invalid return time."));
        }
        let state = Self {
            available,
            time,
            rate,
            range,
            fps,
            return_time,
        };
        state.validate()?;
        Ok(state)
    }
    fn validate(self) -> Result<(), CommandError> {
        if number(self.time)? < 0.0 || number(self.fps)? <= 0.0 {
            return Err(invalid("Invalid transport time or frame rate."));
        }
        if let Some(r) = self.range
            && (number(r.start)? < 0.0 || number(r.end)? <= r.start)
        {
            return Err(invalid("Invalid play range."));
        }
        if let Some(t) = self.return_time
            && number(t)? < 0.0
        {
            return Err(invalid("Invalid return time."));
        }
        Ok(())
    }
    /// Merge facts into the existing transport object, retaining opaque fields.
    /// `available` is a runtime capability, never persisted as project data.
    pub fn transport_value(self, existing: &Value) -> Result<Value, CommandError> {
        self.validate()?;
        let mut m = existing.as_object().cloned().unwrap_or_default();
        m.insert("time", Value::Number(self.time));
        m.insert("playing", Value::Bool(self.rate != ShuttleRate::Stopped));
        m.insert("rate", Value::Number(f64::from(self.rate.factor())));
        m.insert(
            "returnTime",
            self.return_time.map(Value::Number).unwrap_or(Value::Null),
        );
        m.insert(
            "playRange",
            self.range
                .map(|r| {
                    let mut m = Map::new();
                    m.insert("start", Value::Number(r.start));
                    m.insert("end", Value::Number(r.end));
                    Value::Object(m)
                })
                .unwrap_or(Value::Null),
        );
        Ok(Value::Object(m))
    }
}
