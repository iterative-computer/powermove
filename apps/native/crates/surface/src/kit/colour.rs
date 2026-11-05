//! Live colour previews are one binding transaction; opening never rewrites colour.
use crate::{
    Id, Surface,
    binding::{BindingSink, Bindings},
};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct ColourEdit {
    anchor: Id,
    binding: String,
    active: bool,
    closed: bool,
}
impl ColourEdit {
    pub fn open(surface: &Surface, anchor: Id) -> Result<Self, String> {
        let n = surface.node(anchor).ok_or("missing colour anchor")?;
        if n.props.get("control").and_then(Value::as_str) != Some("color")
            || n.props.get("disabled").and_then(Value::as_bool) == Some(true)
        {
            return Err("not an enabled colour well".into());
        }
        Ok(Self {
            anchor,
            binding: n
                .props
                .get("binding")
                .and_then(Value::as_str)
                .ok_or("missing colour binding")?
                .into(),
            active: false,
            closed: false,
        })
    }
    pub fn preview(
        &mut self,
        surface: &Surface,
        colour: &str,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        if self.closed {
            return Err("colour editor already closed".into());
        }
        if !surface.node(self.anchor).is_some_and(|n| {
            n.props.get("binding").and_then(Value::as_str) == Some(&self.binding)
                && n.props.get("control").and_then(Value::as_str) == Some("color")
                && n.props.get("disabled").and_then(Value::as_bool) != Some(true)
        }) {
            self.close(true, bindings, sink)?;
            return Err("retired colour descriptor".into());
        }
        if !self.active {
            bindings.begin(surface, self.anchor, sink)?;
            self.active = true;
        }
        bindings.write(self.anchor, json!(colour), sink)
    }
    /// Escape cancels; outside dismissal commits. Queue failures are retryable.
    pub fn close(
        &mut self,
        cancel: bool,
        bindings: &mut Bindings,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        if self.closed {
            return Ok(());
        }
        if self.active {
            bindings.finish(self.anchor, cancel, sink)?;
        }
        self.active = false;
        self.closed = true;
        Ok(())
    }
}
