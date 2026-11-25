//! What an agent reads and drives in a v4 panel (get_panel_state,
//! interact_panel): the compiled tree's visible text and its controls, in tree
//! order, as main's panel tools read a panel's DOM.
use super::SurfaceView;
use crate::v4_compile::NodeView;
use gpui::{Context, Pixels, Point, Styled, canvas, prelude::*};
use pm_surface::{Id, events::EventDetail, kit::Component};
use serde_json::{Value, json};

/// One observed control. `id` stays in the app; agents see only refs.
pub struct Control {
    pub id: Id,
    pub fact: Value,
}
pub struct Target {
    pub protected: bool,
    pub editable: bool,
}
pub struct Reading {
    pub text: String,
    pub controls: Vec<Control>,
}
const TEXT_LIMIT: usize = 12_000;
const CONTROL_LIMIT: usize = 250;

fn clip(text: &str, limit: usize) -> String {
    text.trim().chars().take(limit).collect()
}
fn role_name(view: &NodeView) -> String {
    match view.component {
        Some(Component::TextField | Component::NumberField) => "textbox".into(),
        Some(Component::PopUpButton) => "select".into(),
        Some(Component::Checkbox) => "checkbox".into(),
        Some(Component::Toggle | Component::Switch) => "switch".into(),
        Some(Component::Segment) => "tab".into(),
        Some(Component::ListItem | Component::TreeItem) => "option".into(),
        Some(Component::DisclosureButton | Component::DisclosureGroup) => "summary".into(),
        Some(Component::SecureField) => "password".into(),
        _ => view.ax.as_ref().and_then(|ax| ax.role).map_or_else(
            || "button".into(),
            |role| format!("{role:?}").to_lowercase(),
        ),
    }
}

impl SurfaceView {
    fn visible(&self, view: &NodeView) -> bool {
        let mut next = Some(view);
        while let Some(node) = next {
            if node.hidden {
                return false;
            }
            next = node.parent.and_then(|id| self.prepared.node(id));
        }
        true
    }
    fn descendant_text(&self, id: Id, out: &mut String) {
        let Some(view) = self.prepared.node(id) else {
            return;
        };
        if view.hidden {
            return;
        }
        if let Some(text) = &view.text {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(text);
        }
        for child in &view.children {
            self.descendant_text(*child, out);
        }
    }
    /// A bounds probe for an agent-relevant node, only while probing.
    pub(super) fn probe(&self, id: Id) -> Option<gpui::AnyElement> {
        let view = self.prepared.node(id)?;
        let relevant = self.is_control(view)
            || view.component == Some(Component::SecureField)
            || !self.prop(id, "href").is_null();
        if !self.probing || !relevant {
            return None;
        }
        let targets = self.targets.clone();
        Some(
            canvas(
                move |bounds, _, _| {
                    targets.borrow_mut().insert(id, bounds);
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0()
            .into_any_element(),
        )
    }
    /// Turns bounds recording on for the next draw (and off after it).
    pub fn set_probing(&mut self, on: bool) {
        if on {
            self.targets.borrow_mut().clear();
        }
        self.probing = on;
    }
    /// What lies under a window point: the innermost recorded control, and
    /// whether it (or a link around it) is protected, and whether it edits text.
    pub fn agent_target(&self, at: Point<Pixels>) -> Option<Target> {
        let targets = self.targets.borrow();
        let (id, _) =
            targets
                .iter()
                .filter(|(_, b)| b.contains(&at))
                .min_by(|(_, a), (_, b)| {
                    (f32::from(a.size.width) * f32::from(a.size.height))
                        .total_cmp(&(f32::from(b.size.width) * f32::from(b.size.height)))
                })?;
        let view = self.prepared.node(*id)?;
        let mut linked = false;
        let mut next = Some(view);
        while let Some(node) = next {
            linked |= !self.prop(node.id, "href").is_null();
            next = node.parent.and_then(|p| self.prepared.node(p));
        }
        Some(Target {
            protected: view.disabled || view.component == Some(Component::SecureField) || linked,
            editable: matches!(
                view.component,
                Some(Component::TextField | Component::NumberField)
            ) || view.number.is_some(),
        })
    }
    fn is_control(&self, view: &NodeView) -> bool {
        !view.is_plain_text
            && view.native.is_none()
            && (view.events.click
                || view.events.double_click
                || view.binding.is_some()
                || view.number.is_some()
                || matches!(
                    view.component,
                    Some(
                        Component::TextField
                            | Component::NumberField
                            | Component::PopUpButton
                            | Component::Checkbox
                            | Component::Toggle
                            | Component::Switch
                            | Component::Segment
                            | Component::PushButton
                            | Component::DisclosureButton
                    )
                ))
    }
    fn prop(&self, id: Id, key: &str) -> Value {
        self.publication
            .surface
            .node(id)
            .and_then(|node| node.props.get(key).cloned())
            .unwrap_or(Value::Null)
    }
    fn options(&self, id: Id) -> Vec<Value> {
        self.offered_rows(id)
            .as_ref()
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .map(|option| {
                        let value = option_value(option);
                        json!({"value":option_text(value),
                            "label":option.get("label").and_then(Value::as_str).map_or_else(|| option_text(value), str::to_owned),
                            "disabled":option["disabled"] == true})
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    /// The visible text and controls, as main's `readPanel` reports them.
    pub fn agent_read(&self) -> Reading {
        let mut text = String::new();
        let mut controls = Vec::new();
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            let Some(view) = self.prepared.node(id) else {
                continue;
            };
            if view.hidden {
                continue;
            }
            if let Some(t) = &view.text
                && text.len() < TEXT_LIMIT
            {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
            if controls.len() < CONTROL_LIMIT
                && self.is_control(view)
                && view.component != Some(Component::SecureField)
            {
                controls.push(self.control(view));
            }
            stack.extend(view.children.iter().rev());
        }
        Reading {
            text: clip(&text, TEXT_LIMIT),
            controls,
        }
    }
    fn control(&self, view: &NodeView) -> Control {
        let ax = view.ax.as_ref();
        let label = ax
            .and_then(|ax| ax.label.clone())
            .or_else(|| view.title.clone())
            .map(|s| s.to_string())
            .or_else(|| {
                self.prop(view.id, "placeholder")
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| {
                let mut out = String::new();
                self.descendant_text(view.id, &mut out);
                out
            });
        let mut fact = json!({"role":role_name(view),"label":clip(&label, 300),
            "disabled":view.disabled || ax.is_some_and(|ax| ax.disabled)});
        let field = matches!(
            view.component,
            Some(Component::TextField | Component::NumberField | Component::PopUpButton)
        ) || view.number.is_some();
        if field {
            let value = match (view.number, ax.and_then(|ax| ax.value.clone())) {
                (Some(number), _) => number.text(number.value),
                (None, Some(value)) => value.to_string(),
                (None, None) => option_text(&self.prop(view.id, "value")),
            };
            fact["value"] = json!(clip(&value, 2000));
        }
        if view.component == Some(Component::PopUpButton) {
            fact["options"] = json!(self.options(view.id));
        }
        if let Some(checked) = self
            .prop(view.id, "checked")
            .as_bool()
            .or_else(|| ax.and_then(|ax| ax.selected))
            .filter(|_| {
                matches!(
                    view.component,
                    Some(Component::Checkbox | Component::Toggle | Component::Switch)
                )
            })
        {
            fact["checked"] = json!(checked);
        }
        Control { id: view.id, fact }
    }
    /// A person typing: the field shows the text, `input` listeners see it,
    /// then the edit commits (main sets the value, then input and change).
    fn fill(&mut self, id: Id, text: &str, value: Value, cx: &mut Context<Self>) {
        if let Some(field) = self.fields.get(&id).cloned() {
            field.update(cx, |f, cx| f.set_value(text, cx));
        }
        let listeners = self
            .publication
            .surface
            .node(id)
            .map(|n| {
                (
                    n.listeners.contains_key("input"),
                    n.listeners.contains_key("change"),
                )
            })
            .unwrap_or_default();
        if listeners.0 {
            let detail = EventDetail {
                value: Some(json!(text)),
                ..Default::default()
            };
            self.send(id, "input", detail, cx);
        }
        // A bound field also keeps the typed text in its model, as it does
        // while a person edits, so the field shows what was filled.
        if listeners.1
            && let Some(binding) = self.prop(id, "binding").as_str().map(str::to_owned)
        {
            let ticket = self
                .runtime
                .write(&self.publication.panel, &binding, value.clone());
            self.run(ticket, cx);
        }
        self.change(id, value, cx);
    }
    pub fn agent_has(&self, id: Id) -> bool {
        self.prepared
            .node(id)
            .is_some_and(|view| self.visible(view) && self.is_control(view))
    }
    /// interact_panel's click, fill, select and press, with main's refusals.
    pub fn agent_act(
        &mut self,
        id: Id,
        args: &Value,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let view = self
            .prepared
            .node(id)
            .ok_or("The control reference is stale. Read get_panel_state again.")?;
        if view.disabled || view.component == Some(Component::SecureField) {
            return Err("This control is protected or disabled.".into());
        }
        let component = view.component;
        let number = view.number;
        let text = |key: &str| args[key].as_str().map(str::to_owned);
        match args["action"].as_str() {
            // A click lands where a person's would; pop-ups and colour wells
            // open native pickers, which the select action replaces.
            Some("click") => match component {
                Some(Component::PopUpButton | Component::ColourWell) => {
                    return Err(
                        "Use select for pop-up menus; colour wells need computer_use_panel.".into(),
                    );
                }
                _ => self.activate(id, EventDetail::default(), window, cx),
            },
            Some("fill") => {
                let value = text("value")
                    .filter(|v| v.chars().count() <= 10_000)
                    .ok_or("Supply text up to 10000 characters.")?;
                match (component, number) {
                    (_, Some(_)) | (Some(Component::NumberField), _) => {
                        let n = value
                            .trim()
                            .parse::<f64>()
                            .map_err(|_| "Choose an editable text or numeric field.")?;
                        self.fill(id, &value, json!(n), cx);
                    }
                    (Some(Component::TextField), _) => self.fill(id, &value, json!(value), cx),
                    _ => return Err("Choose an editable text or numeric field.".into()),
                }
            }
            Some("select") => {
                let wanted = text("value").unwrap_or_default();
                let offered = self.offered_rows(id);
                let options = offered.as_ref().and_then(Value::as_array);
                let chosen = options
                    .into_iter()
                    .flatten()
                    .filter(|o| o["disabled"] != true)
                    .map(option_value)
                    .find(|v| option_text(v) == wanted)
                    .cloned()
                    .filter(|_| component == Some(Component::PopUpButton))
                    .ok_or("Choose an enabled option value from get_panel_state.")?;
                self.change(id, chosen, cx);
            }
            Some("press") => {
                if !matches!(
                    component,
                    Some(Component::TextField | Component::NumberField | Component::PopUpButton)
                ) {
                    return Err(
                        "Use click for buttons; key presses require a panel text or select field."
                            .into(),
                    );
                }
                let key = text("key").filter(|k| {
                    [
                        "Enter",
                        "Escape",
                        "ArrowUp",
                        "ArrowDown",
                        "ArrowLeft",
                        "ArrowRight",
                        " ",
                    ]
                    .contains(&k.as_str())
                });
                let key = key.ok_or("Unsupported panel key.")?;
                for event in ["keydown", "keyup"] {
                    let detail = EventDetail {
                        key: Some(key.clone()),
                        ..Default::default()
                    };
                    self.send(id, event, detail, cx);
                }
            }
            _ => return Err("Unknown panel action.".into()),
        }
        Ok(())
    }
}
/// Kit options are `{value|v, label}` objects or bare values.
fn option_value(option: &Value) -> &Value {
    option
        .get("value")
        .or_else(|| option.get("v"))
        .unwrap_or(option)
}
fn option_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
