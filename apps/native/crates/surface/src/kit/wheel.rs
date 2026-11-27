//! Rest-gated numeric wheel editing (panels/README §5.2). All times are monotonic.
use super::{DesignTokenTable, WHEEL_REST_TOKEN};
use crate::{
    Id, Surface,
    binding::{BindingSink, Bindings},
};
use pm_design::Tokens;
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelDevice {
    Discrete,
    Trackpad,
}
#[derive(Clone, Copy, Debug)]
pub struct WheelSample {
    pub now: Duration,
    /// Positive increments. The native adapter converts its device units to notches.
    pub notches: f64,
    pub device: WheelDevice,
    pub shift: bool,
    pub alt: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelDisposition {
    Scroll,
    Edited(f64),
}
#[derive(Clone, Debug)]
pub struct NumericWheel {
    id: Id,
    binding: String,
    rest: Duration,
    threshold: f64,
    origin: (f64, f64),
    hovered_since: Duration,
    last_scroll: Duration,
    last_event: Option<Duration>,
    last_value: Option<f64>,
    active: bool,
    present: bool,
}
impl NumericWheel {
    pub fn new(
        surface: &Surface,
        id: Id,
        t: &Tokens,
        table: &DesignTokenTable,
        now: Duration,
        position: (f64, f64),
    ) -> Result<Self, String> {
        let node = surface.node(id).ok_or("missing numeric target")?;
        if node.props.get("control").and_then(Value::as_str) != Some("number")
            || node.props.get("disabled").and_then(Value::as_bool) == Some(true)
            || ![position.0, position.1, f64::from(t.metrics.drag_threshold)]
                .iter()
                .all(|n| n.is_finite())
            || t.metrics.drag_threshold < 0.
        {
            return Err("not an enabled numeric control".into());
        }
        let seconds = table
            .metric(WHEEL_REST_TOKEN)
            .ok_or("missing motion.wheel_rest")?;
        let rest = Duration::try_from_secs_f64(seconds).map_err(|_| "invalid motion.wheel_rest")?;
        Ok(Self {
            id,
            binding: node
                .props
                .get("binding")
                .and_then(Value::as_str)
                .ok_or("missing numeric binding")?
                .into(),
            rest,
            threshold: f64::from(t.metrics.drag_threshold),
            origin: position,
            hovered_since: now,
            last_scroll: now,
            last_event: None,
            last_value: None,
            active: false,
            present: true,
        })
    }
    pub fn pointer(&mut self, now: Duration, position: (f64, f64)) {
        if ![position.0, position.1].iter().all(|n| n.is_finite()) {
            self.present = false;
            return;
        }
        if !self.present
            || (position.0 - self.origin.0).hypot(position.1 - self.origin.1) > self.threshold
        {
            self.origin = position;
            self.hovered_since = now;
        }
        self.present = true;
    }
    /// Call for actual container scroll, including momentum and programmatic scroll.
    pub fn container_scrolled(&mut self, now: Duration) {
        self.last_scroll = now;
    }
    pub fn armed(&self, now: Duration) -> bool {
        self.present
            && now.saturating_sub(self.hovered_since) >= self.rest
            && now.saturating_sub(self.last_scroll) >= self.rest
    }
    fn current<'a>(&self, surface: &'a Surface) -> Option<&'a crate::Node> {
        surface.node(self.id).filter(|n| {
            n.props.get("binding").and_then(Value::as_str) == Some(&self.binding)
                && n.props.get("control").and_then(Value::as_str) == Some("number")
                && n.props.get("disabled").and_then(Value::as_bool) != Some(true)
        })
    }
    pub fn sample(
        &mut self,
        surface: &Surface,
        event: WheelSample,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<WheelDisposition, String> {
        if self.current(surface).is_none() {
            self.finish(true, bindings, sink)?;
            self.present = false;
            return Ok(WheelDisposition::Scroll);
        }
        self.tick(event.now, bindings, sink)?;
        if !self.armed(event.now)
            || (event.device == WheelDevice::Trackpad && !event.alt)
            || !event.notches.is_finite()
            || event.notches == 0.
        {
            return Ok(WheelDisposition::Scroll);
        }
        let node = self
            .current(surface)
            .expect("checked current numeric descriptor");
        let number = |key, default| {
            node.props
                .get(key)
                .and_then(Value::as_f64)
                .unwrap_or(default)
        };
        let start = self.last_value.unwrap_or_else(|| number("value", f64::NAN));
        let step = number("step", 1.);
        let min = number("min", -f64::MAX);
        let max = number("max", f64::MAX);
        if ![start, step, min, max].iter().all(|v| v.is_finite()) || step <= 0. || min > max {
            return Err("invalid numeric wheel descriptor".into());
        }
        // Option gates a trackpad but does not reduce its step. Shift wins.
        let multiplier = if event.shift {
            10.
        } else if event.alt && event.device == WheelDevice::Discrete {
            0.1
        } else {
            1.
        };
        let next = start + event.notches * step * multiplier;
        if !next.is_finite() {
            return Err("non-finite wheel value".into());
        }
        let next = next.clamp(min, max);
        if next == start {
            if self.active {
                self.last_event = Some(event.now);
            }
            return Ok(WheelDisposition::Edited(next));
        }
        if !self.active {
            bindings.begin(surface, self.id, sink)?;
            self.active = true;
        }
        self.last_event = Some(event.now);
        bindings.write(self.id, json!(next), sink)?;
        // Publish local state only after the queue accepted the edit. A failed
        // begin/write/commit remains retryable and never duplicates a delta.
        self.last_value = Some(next);
        self.last_event = Some(event.now);
        Ok(WheelDisposition::Edited(next))
    }
    /// Schedule for wheel_rest after the last accepted event. Exactly one undo.
    pub fn tick(
        &mut self,
        now: Duration,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<bool, String> {
        if self.active
            && self
                .last_event
                .is_some_and(|last| now.saturating_sub(last) >= self.rest)
        {
            self.finish(self.last_value.is_none(), bindings, sink)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub fn leave(
        &mut self,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        self.finish(false, bindings, sink)?;
        self.present = false;
        Ok(())
    }
    /// On destroy/disable/rebind cancel before `Bindings::reconcile`.
    pub fn finish(
        &mut self,
        cancel: bool,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        if self.active {
            bindings.finish(self.id, cancel, sink)?;
        }
        self.active = false;
        self.last_event = None;
        self.last_value = None;
        Ok(())
    }
}
