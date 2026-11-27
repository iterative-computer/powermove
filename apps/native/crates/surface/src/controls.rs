//! Headless control semantics, independent of AppKit widgets and visual design.
use crate::{
    Id, Surface,
    binding::{BindingSink, Bindings},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Draft(Id, String),
    Blur(Id),
    Invalidate,
}
#[derive(Clone, Debug)]
struct Draft {
    initial: String,
    text: String,
    text_control: bool,
    binding: String,
}
#[derive(Default)]
pub struct Controls {
    pub bindings: Bindings,
    drafts: BTreeMap<Id, Draft>,
}
impl Controls {
    pub fn focus(
        &mut self,
        surface: &Surface,
        id: Id,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<Effect>, String> {
        if self.drafts.contains_key(&id) {
            return Ok(vec![]);
        }
        let node = surface.node(id).ok_or("missing control")?;
        if node.props.get("disabled").and_then(Value::as_bool) == Some(true) {
            return Err("disabled control".into());
        }
        let text_control = node.props.get("control").and_then(Value::as_str) == Some("text");
        let binding = node
            .props
            .get("binding")
            .and_then(Value::as_str)
            .ok_or("missing binding")?
            .to_owned();
        let value = node.props.get("value").cloned().unwrap_or(Value::Null);
        let text = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string());
        if text_control {
            self.bindings.begin(surface, id, sink)?;
        }
        self.drafts.insert(
            id,
            Draft {
                initial: text.clone(),
                text: text.clone(),
                text_control,
                binding,
            },
        );
        Ok(vec![Effect::Draft(id, text)])
    }
    pub fn input(
        &mut self,
        id: Id,
        text: String,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<Effect>, String> {
        let draft = self.drafts.get_mut(&id).ok_or("control is not editing")?;
        if draft.text_control {
            self.bindings.write(id, Value::String(text.clone()), sink)?;
        }
        draft.text = text;
        Ok(vec![Effect::Invalidate])
    }
    /// Numeric typing commits finite changes without scrub clamping (W073 B567).
    pub fn finish(
        &mut self,
        surface: &Surface,
        id: Id,
        cancel: bool,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<Effect>, String> {
        let Some(draft) = self.drafts.get(&id).cloned() else {
            return Ok(vec![]);
        };
        if draft.text_control {
            self.bindings.finish(id, cancel, sink)?;
        } else if !cancel && draft.text.trim() != draft.initial {
            let current = surface
                .node(id)
                .and_then(|n| n.props.get("value"))
                .and_then(Value::as_f64)
                .ok_or("numeric value required")?;
            if let Some(value) = numeric_draft(&draft.text, current) {
                self.bindings.once(surface, id, json!(value), sink)?;
            }
        }
        self.drafts.remove(&id);
        Ok(vec![Effect::Blur(id), Effect::Invalidate])
    }
    pub fn numeric_key(
        &mut self,
        surface: &Surface,
        id: Id,
        key: &str,
        shift: bool,
        sink: &mut impl BindingSink,
    ) -> Result<Vec<Effect>, String> {
        if key == "Escape" {
            return self.finish(surface, id, true, sink);
        }
        if key == "Enter" {
            return self.finish(surface, id, false, sink);
        }
        let node = surface.node(id).ok_or("missing numeric control")?;
        let step = node
            .props
            .get("step")
            .and_then(Value::as_f64)
            .filter(|s| *s > 0.)
            .unwrap_or(1.);
        let current = self
            .drafts
            .get(&id)
            .and_then(|d| {
                let current = node.props.get("value").and_then(Value::as_f64)?;
                numeric_draft(&d.text, current)
            })
            .or_else(|| node.props.get("value").and_then(Value::as_f64))
            .ok_or("numeric value required")?;
        let min = node
            .props
            .get("min")
            .and_then(Value::as_f64)
            .unwrap_or(-f64::MAX);
        let max = node
            .props
            .get("max")
            .and_then(Value::as_f64)
            .unwrap_or(f64::MAX);
        if min > max {
            return Err("invalid bounds".into());
        }
        let next = match key {
            "ArrowUp" | "ArrowRight" => current + step * if shift { 10. } else { 1. },
            "ArrowDown" | "ArrowLeft" => current - step * if shift { 10. } else { 1. },
            "Home" => min,
            "End" => max,
            _ => return Ok(vec![]),
        }
        .clamp(min, max);
        if !next.is_finite() {
            return Err("non-finite numeric result".into());
        }
        if let Some(draft) = self.drafts.get_mut(&id) {
            draft.text = next.to_string();
            return Ok(vec![Effect::Draft(id, draft.text.clone())]);
        }
        if next != current {
            self.bindings.once(surface, id, json!(next), sink)?;
        }
        Ok(vec![Effect::Invalidate])
    }
    pub fn toggle(
        &mut self,
        surface: &Surface,
        id: Id,
        key: &str,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        let node = surface.node(id).ok_or("missing toggle")?;
        let current = node
            .props
            .get("value")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mixed = node
            .props
            .get("mixed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let next = match key {
            "ArrowLeft" | "ArrowUp" => false,
            "ArrowRight" | "ArrowDown" => true,
            " " | "Enter" => mixed || !current,
            _ => return Ok(()),
        };
        if mixed || next != current {
            self.bindings.once(surface, id, json!(next), sink)?;
        }
        Ok(())
    }
    pub fn select(
        &mut self,
        surface: &Surface,
        id: Id,
        index: usize,
        declined: bool,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        if declined {
            return Ok(());
        } // command-like option: do not write document value.
        let node = surface.node(id).ok_or("missing select")?;
        let option = node
            .props
            .get("options")
            .and_then(Value::as_array)
            .and_then(|o| o.get(index))
            .ok_or("missing option")?;
        if option.get("disabled").and_then(Value::as_bool) == Some(true) {
            return Err("disabled option".into());
        }
        let value = if option.is_string() {
            option
        } else {
            option.get("v").ok_or("invalid option")?
        };
        self.bindings.once(surface, id, value.clone(), sink)
    }
    pub fn reconcile(
        &mut self,
        surface: &Surface,
        sink: &mut impl BindingSink,
    ) -> Result<(), String> {
        self.bindings.reconcile(surface, sink)?;
        self.drafts.retain(|id, draft| {
            surface.node(*id).is_some_and(|n| {
                n.props.get("disabled").and_then(Value::as_bool) != Some(true)
                    && n.props.get("binding").and_then(Value::as_str)
                        == Some(draft.binding.as_str())
            })
        });
        Ok(())
    }
}
/// Tiny arithmetic parser. No evaluation of JS or arbitrary source; bounded recursion/input.
pub fn numeric_draft(raw: &str, current: f64) -> Option<f64> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 4096 || !current.is_finite() {
        return None;
    }
    let relative = raw.starts_with(['+', '*', '/']) || raw.starts_with("-=");
    let expression = if relative {
        format!(
            "{current}{}",
            raw.replacen("+=", "+", 1).replacen("-=", "-", 1)
        )
    } else {
        raw.into()
    };
    let mut parser = Parser {
        bytes: expression.as_bytes(),
        at: 0,
        depth: 0,
    };
    let result = parser.expression()?;
    parser.space();
    if parser.at != parser.bytes.len() || !result.is_finite() {
        return None;
    }
    let scaled = result * 10_000.;
    Some(if scaled.is_finite() {
        scaled.round() / 10_000.
    } else {
        result
    })
}
struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn expression(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        loop {
            self.space();
            match self.bytes.get(self.at) {
                Some(b'+') => {
                    self.at += 1;
                    v += self.product()?;
                }
                Some(b'-') => {
                    self.at += 1;
                    v -= self.product()?;
                }
                _ => return v.is_finite().then_some(v),
            }
        }
    }
    fn product(&mut self) -> Option<f64> {
        let mut v = self.primary()?;
        loop {
            self.space();
            match self.bytes.get(self.at) {
                Some(b'*') => {
                    self.at += 1;
                    v *= self.primary()?;
                }
                Some(b'/') => {
                    self.at += 1;
                    v /= self.primary()?;
                }
                _ => return v.is_finite().then_some(v),
            }
        }
    }
    fn primary(&mut self) -> Option<f64> {
        self.space();
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let sign = match self.bytes.get(self.at) {
            Some(b'+') => {
                self.at += 1;
                1.
            }
            Some(b'-') => {
                self.at += 1;
                -1.
            }
            _ => 1.,
        };
        self.space();
        let v = if self.bytes.get(self.at) == Some(&b'(') {
            self.at += 1;
            let v = self.expression()?;
            self.space();
            if self.bytes.get(self.at) != Some(&b')') {
                return None;
            }
            self.at += 1;
            v
        } else if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
            self.primary()?
        } else {
            let start = self.at;
            while self
                .bytes
                .get(self.at)
                .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
            {
                self.at += 1;
            }
            if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
                self.at += 1;
                if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                    self.at += 1;
                }
                while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
            }
            std::str::from_utf8(&self.bytes[start..self.at])
                .ok()?
                .parse::<f64>()
                .ok()?
        };
        self.depth -= 1;
        (sign * v).is_finite().then_some(sign * v)
    }
}
