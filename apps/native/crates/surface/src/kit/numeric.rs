//! Native scrub gesture: one transaction; authored typing remains Controls' path.
use crate::{
    Id, Surface,
    binding::{BindingSink, Bindings, Scrub, scrub},
};
use pm_design::Tokens;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrubEffect {
    Pending,
    CapturePointer,
    Value(f64),
    EditText,
    ReleasePointer,
}
#[derive(Debug)]
pub struct NumericScrub {
    id: Id,
    binding: String,
    origin: (f64, f64),
    start: f64,
    step: f64,
    speed: f64,
    bounds: (f64, f64),
    threshold: f64,
    active: bool,
    capture_requested: bool,
    finished: bool,
    last: Option<f64>,
}
impl NumericScrub {
    pub fn new(
        surface: &Surface,
        id: Id,
        origin: (f64, f64),
        tokens: &Tokens,
    ) -> Result<Self, String> {
        let node = surface.node(id).ok_or("missing numeric target")?;
        if node.props.get("control").and_then(Value::as_str) != Some("number")
            || node.props.get("disabled").and_then(Value::as_bool) == Some(true)
        {
            return Err("not an enabled numeric control".into());
        }
        let number = |key: &str, default| {
            node.props
                .get(key)
                .and_then(Value::as_f64)
                .unwrap_or(default)
        };
        let start = number("value", f64::NAN);
        let step = number("step", 1.);
        let speed = number("speed", 0.5); // W073 / language.md §9 behavior, not a visual value.
        let bounds = (number("min", -f64::MAX), number("max", f64::MAX));
        let threshold = f64::from(tokens.metrics.drag_threshold);
        if ![
            origin.0, origin.1, start, step, speed, bounds.0, bounds.1, threshold,
        ]
        .iter()
        .all(|v| v.is_finite())
            || step <= 0.
            || speed <= 0.
            || threshold < 0.
            || bounds.0 > bounds.1
        {
            return Err("invalid numeric descriptor".into());
        }
        Ok(Self {
            id,
            binding: node
                .props
                .get("binding")
                .and_then(Value::as_str)
                .ok_or("missing numeric binding")?
                .into(),
            origin,
            start,
            step,
            speed,
            bounds,
            threshold,
            active: false,
            capture_requested: false,
            finished: false,
            last: None,
        })
    }
    pub fn sample(
        &mut self,
        surface: &Surface,
        position: (f64, f64),
        shift: bool,
        alt: bool,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<ScrubEffect>, String> {
        if self.finished {
            return Err("numeric gesture already finished".into());
        }
        if !surface.node(self.id).is_some_and(|n| {
            n.props.get("binding").and_then(Value::as_str) == Some(&self.binding)
                && n.props.get("disabled").and_then(Value::as_bool) != Some(true)
        }) {
            return Err("retired numeric descriptor; cancel original gesture".into());
        }
        let delta = (position.0 - self.origin.0, position.1 - self.origin.1);
        let value = scrub(Scrub {
            start: self.start,
            delta,
            step: self.step,
            speed: self.speed,
            shift,
            alt,
            bounds: self.bounds,
        })?;
        if !self.active && delta.0.hypot(delta.1) < self.threshold {
            return Ok(vec![ScrubEffect::Pending]);
        }
        let mut effects = vec![];
        if !self.active {
            bindings.begin(surface, self.id, sink)?;
            self.active = true;
        }
        if self.last != Some(value) {
            bindings.write(self.id, json!(value), sink)?;
            self.last = Some(value);
            effects.push(ScrubEffect::Value(value));
        }
        if !self.capture_requested {
            self.capture_requested = true;
            effects.insert(0, ScrubEffect::CapturePointer);
        }
        Ok(effects)
    }
    /// Host executes pointer effects without a JS round trip. Failed queue writes
    /// leave the gesture retryable. On retire/disable use cancel before reconcile.
    pub fn finish(
        &mut self,
        cancel: bool,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<ScrubEffect>, String> {
        if self.finished {
            return Ok(vec![]);
        }
        if self.active {
            bindings.finish(self.id, cancel, sink)?;
            self.active = false;
            self.finished = true;
            Ok(vec![ScrubEffect::ReleasePointer])
        } else {
            self.finished = true;
            Ok(if cancel {
                vec![]
            } else {
                vec![ScrubEffect::EditText]
            })
        }
    }
}
// DESIGN-TODO(opus): wheel-rest timing/affordance has no token/spec threshold.
// Wheel changes stay unbound until approved; scrolling past a field cannot edit it.
