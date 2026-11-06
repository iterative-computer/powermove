//! GPUI adapter for accepted pm-surface trees. Kit recipes and authored token
//! styles are resolved here; no JS, kernel waits or authored model copies run in render.
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, Publication};
use pm_shell::text::{TextEvent, TextField, TextKind};
use pm_surface::{
    Id,
    events::{DragData, DragEffect, EventDetail},
    kit::{Component, Panel},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

pub struct SurfaceView {
    pub runtime: Arc<Builtins>,
    pub publication: Publication,
    pub tokens: pm_design::Tokens,
    panel: Panel,
    fields: BTreeMap<Id, Entity<TextField>>,
    controls: BTreeMap<Id, Entity<crate::v4_control::NativeControl>>,
    scrolls: BTreeMap<Id, ScrollHandle>,
    leases: BTreeMap<Id, Option<String>>,
    option_sets: BTreeMap<Id, Value>,
    focus: BTreeMap<Id, FocusHandle>,
    subscriptions: BTreeMap<Id, Vec<Subscription>>,
    pub error: Option<String>,
}
impl SurfaceView {
    pub fn new(
        runtime: Arc<Builtins>,
        publication: Publication,
        tokens: pm_design::Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let panel = Panel::prepare(
            publication.surface.clone(),
            &tokens,
            &Default::default(),
            None,
            false,
        );
        let mut this = Self {
            runtime,
            publication,
            tokens,
            panel,
            fields: BTreeMap::new(),
            controls: BTreeMap::new(),
            scrolls: BTreeMap::new(),
            leases: BTreeMap::new(),
            option_sets: BTreeMap::new(),
            focus: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
            error: None,
        };
        this.reconcile(window, cx);
        this
    }
    pub fn publish(
        &mut self,
        publication: Publication,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.publication = publication;
        self.panel = Panel::prepare(
            self.publication.surface.clone(),
            &self.tokens,
            &Default::default(),
            None,
            false,
        );
        self.reconcile(window, cx);
        cx.notify();
    }
    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.controls
            .retain(|id, _| self.publication.surface.node(*id).is_some());
        self.scrolls
            .retain(|id, _| self.publication.surface.node(*id).is_some());
        self.subscriptions
            .retain(|id, _| self.publication.surface.node(*id).is_some());
        self.fields
            .retain(|id, _| self.publication.surface.node(*id).is_some());
        self.focus
            .retain(|id, _| self.publication.surface.node(*id).is_some());
        let nodes = self
            .publication
            .surface
            .nodes()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        self.leases
            .retain(|id, _| self.publication.surface.node(*id).is_some());
        self.option_sets
            .retain(|id, _| self.publication.surface.node(*id).is_some());
        for node in nodes {
            let lease = node
                .props
                .get("binding")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let options = node.props.get("options").cloned().unwrap_or(Value::Null);
            if self.leases.get(&node.id) != Some(&lease)
                || self.option_sets.get(&node.id) != Some(&options)
            {
                self.subscriptions.remove(&node.id);
                self.fields.remove(&node.id);
                self.controls.remove(&node.id);
                self.leases.insert(node.id, lease.clone());
                self.option_sets.insert(node.id, options);
            }
            if matches!(node.r#type.as_str(), "scroll" | "virtual-list") {
                self.scrolls.entry(node.id).or_default();
            }
            let kind = match Component::of(&node) {
                Some(Component::PopUpButton) => Some(crate::v4_control::Kind::Popup),
                Some(Component::Checkbox | Component::Toggle) => {
                    Some(crate::v4_control::Kind::Checkbox)
                }
                Some(Component::Switch) => Some(crate::v4_control::Kind::Switch),
                Some(Component::PushButton) => Some(crate::v4_control::Kind::Push),
                Some(Component::ToolbarItem)
                    if self.publication.panel == "export"
                        && node
                            .props
                            .get("metadata")
                            .and_then(|m| m.get("export"))
                            .and_then(|m| m.get("nativeControl"))
                            .and_then(Value::as_str)
                            == Some("NSButton") =>
                {
                    Some(crate::v4_control::Kind::Push)
                }
                _ => None,
            };
            if let Some(kind) = kind {
                let id = node.id;
                let value = node.props.get("value").cloned().unwrap_or(Value::Null);
                let disabled = node.props.get("disabled") == Some(&Value::Bool(true));
                if let Some(control) = self.controls.get(&id) {
                    control.update(cx, |v, cx| v.update(&value, disabled, cx));
                } else {
                    let label = node
                        .props
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let identifier = format!(
                        "v4.{}.{}",
                        self.publication.panel,
                        node.props
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or(label)
                    );
                    let options = node
                        .props
                        .get("options")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let button_role = node
                        .props
                        .get("metadata")
                        .and_then(|m| m.get("pmKit"))
                        .and_then(|m| m.get("buttonRole"))
                        .and_then(Value::as_str);
                    let primary = button_role == Some("default")
                        || node.props.get("attr").and_then(|a| a.get("default-button"))
                            == Some(&Value::Bool(true))
                        || node
                            .props
                            .get("metadata")
                            .and_then(|m| m.get("export"))
                            .and_then(|e| e.get("default"))
                            == Some(&Value::Bool(true));
                    let control = cx.new(|cx| {
                        crate::v4_control::NativeControl::new(
                            crate::v4_control::ControlSpec {
                                kind,
                                label,
                                identifier: &identifier,
                                value: &value,
                                options,
                                disabled,
                                default: primary,
                                destructive: button_role == Some("destructive"),
                            },
                            &self.tokens,
                            window,
                            cx,
                        )
                    });
                    let generation = self.publication.generation;
                    self.subscriptions.entry(id).or_default().push(cx.subscribe(
                        &control,
                        move |v, _, value, cx| {
                            if v.publication.generation != generation
                                || v.leases.get(&id) != Some(&lease)
                            {
                                return;
                            }
                            if matches!(kind, crate::v4_control::Kind::Push) {
                                v.send(id, "click", EventDetail::default(), cx);
                            } else {
                                v.change(id, value.clone(), cx);
                            }
                        },
                    ));
                    self.controls.insert(id, control);
                }
                continue;
            }
            if !disabled_node(&node)
                && (Component::of(&node).is_some_and(|c| c.interactive())
                    || node
                        .props
                        .get("tabIndex")
                        .and_then(Value::as_i64)
                        .is_some_and(|v| v >= 0))
            {
                self.focus
                    .entry(node.id)
                    .or_insert_with(|| cx.focus_handle());
            }
            if !matches!(
                Component::of(&node),
                Some(Component::NumberField | Component::TextField)
            ) || node.props.get("disabled") == Some(&Value::Bool(true))
            {
                continue;
            }
            let value = node.props.get("value").map(value_text).unwrap_or_default();
            if let Some(field) = self.fields.get(&node.id) {
                field.update(cx, |f, cx| {
                    f.set_tokens(self.tokens.clone());
                    f.set_value(&value, cx);
                });
                continue;
            }
            let number = Component::of(&node) == Some(Component::NumberField);
            let label = node
                .props
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("");
            let kind = if number {
                TextKind::DenseNumber
            } else if node.r#type == "textarea" {
                TextKind::Multiline
            } else {
                TextKind::DenseText
            };
            let tokens = self.tokens.clone();
            let id = node.id;
            let field = cx.new(|cx| TextField::new(kind, label, &value, tokens, window, cx));
            self.subscriptions.entry(id).or_default().push(cx.subscribe(
                &field,
                move |this, _, event, cx| {
                    if this
                        .publication
                        .surface
                        .node(id)
                        .and_then(|n| n.props.get("binding"))
                        .and_then(Value::as_str)
                        != lease.as_deref()
                    {
                        return;
                    }
                    match event {
                        TextEvent::Changed(value) => {
                            if this
                                .publication
                                .surface
                                .node(id)
                                .is_some_and(|n| n.listeners.contains_key("input"))
                            {
                                this.send(
                                    id,
                                    "input",
                                    EventDetail {
                                        value: Some(json!(value)),
                                        ..Default::default()
                                    },
                                    cx,
                                );
                            }
                        }
                        TextEvent::Committed(value) => {
                            let value = if number {
                                match value.parse::<f64>() {
                                    Ok(n) if n.is_finite() => json!(n),
                                    _ => {
                                        this.error = Some("Enter a finite number".into());
                                        cx.notify();
                                        return;
                                    }
                                }
                            } else {
                                json!(value)
                            };
                            this.change(id, value, cx);
                        }
                        TextEvent::Cancelled => {}
                    }
                },
            ));
            if let Some(focus) = self.focus.get(&id) {
                let subscription = cx.on_focus(focus, window, move |v, _, cx| {
                    if let Some(field) = v.fields.get(&id) {
                        field.update(cx, |f, cx| f.begin_editing(cx));
                    }
                });
                self.subscriptions.entry(id).or_default().push(subscription);
            }
            self.fields.insert(id, field);
        }
    }
    pub fn header_id(&self) -> Option<Id> {
        let root = self
            .publication
            .surface
            .node(0)?
            .children
            .first()
            .and_then(|id| self.publication.surface.node(*id))?;
        root.children.iter().copied().find(|id| {
            let node = self.publication.surface.node(*id).unwrap();
            matches!(
                node.props.get("id").and_then(Value::as_str),
                Some("timeline:header" | "viewer.bar" | "inspector:header")
            ) || matches!(
                self.publication.panel.as_str(),
                "assets" | "layer-effects" | "fxbrowser"
            ) && node
                .props
                .get("style")
                .and_then(|s| s.get("height"))
                .and_then(|h| h.get("token"))
                .and_then(Value::as_str)
                == Some("metrics.header")
        })
    }
    pub fn scroll_semantic(&mut self, semantic: &str, cx: &mut Context<Self>) {
        if let Some(node) = self
            .publication
            .surface
            .nodes()
            .values()
            .find(|n| n.props.get("id").and_then(Value::as_str) == Some(semantic))
            && let Some(parent) = node.parent.and_then(|id| self.publication.surface.node(id))
            && let Some(scroll) = self.scrolls.get(&parent.id)
            && let Some(index) = parent.children.iter().position(|id| *id == node.id)
        {
            scroll.scroll_to_top_of_item(index);
            cx.notify();
        }
    }
    pub fn focus_semantic(&self, semantic: &str, window: &mut Window, cx: &mut App) {
        if let Some(node) = self
            .publication
            .surface
            .nodes()
            .values()
            .find(|n| n.props.get("id").and_then(Value::as_str) == Some(semantic))
        {
            if let Some(field) = self.fields.get(&node.id) {
                field.update(cx, |f, cx| f.begin_editing(cx));
            } else if let Some(focus) = self.focus.get(&node.id) {
                focus.focus(window, cx);
            }
        }
    }
    fn run(
        &mut self,
        ticket: pm_runtime_v8::Result<pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>>,
        cx: &mut Context<Self>,
    ) {
        match ticket {
            Ok(ticket) => {
                cx.spawn(async move |this, cx| {
                    if let Err(error) = ticket.await {
                        let _ = this.update(cx, |v, cx| {
                            v.error = Some(error.to_string());
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
    pub fn send(&mut self, id: Id, event: &str, detail: EventDetail, cx: &mut Context<Self>) {
        let runtime = self.runtime.clone();
        let p = self.publication.clone();
        let event = event.to_owned();
        // Native planning can read model state. Keep its locks off the main thread.
        cx.spawn(async move |this, cx| {
            let ticket = cx
                .background_executor()
                .spawn(async move {
                    runtime.event(
                        &p.panel,
                        p.generation,
                        p.surface.revision(),
                        id,
                        &event,
                        detail,
                    )
                })
                .await;
            let _ = this.update(cx, |v, cx| v.run(ticket, cx));
        })
        .detach();
    }
    fn change(&mut self, id: Id, value: Value, cx: &mut Context<Self>) {
        let Some(node) = self.publication.surface.node(id) else {
            return;
        };
        if node.listeners.contains_key("change") {
            // A native text observation uses a string; native numeric bindings go
            // through the model lease, not a duplicate JS callback.
            self.send(
                id,
                "change",
                EventDetail {
                    value: Some(value),
                    ..Default::default()
                },
                cx,
            );
        } else if let Some(binding) = node.props.get("binding").and_then(Value::as_str) {
            let ticket = self.runtime.write(&self.publication.panel, binding, value);
            self.run(ticket, cx);
        }
    }
    fn activate(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.publication.surface.node(id).cloned() else {
            return;
        };
        if node.props.get("disabled") == Some(&Value::Bool(true)) {
            return;
        }
        match Component::of(&node) {
            Some(Component::Checkbox | Component::Toggle | Component::Switch) => self.change(
                id,
                json!(
                    !node
                        .props
                        .get("value")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                ),
                cx,
            ),
            Some(Component::PopUpButton) => {
                let options = node
                    .props
                    .get("options")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let owner = cx.entity().downgrade();
                crate::v4_native::menu(window, &options.clone(), cx, move |index, cx| {
                    if let Some(index) = index
                        && let Some(option) = options.get(index)
                    {
                        let value = option.get("v").unwrap_or(option).clone();
                        let _ = owner.update(cx, |v, cx| v.change(id, value, cx));
                    }
                });
            }
            Some(Component::ColourWell) => {
                let initial = node
                    .props
                    .get("value")
                    .and_then(Value::as_str)
                    .and_then(crate::sheets::hex_colour);
                let owner = cx.entity().downgrade();
                let binding = node.props.get("binding").cloned();
                crate::colour_panel::menu(window, initial, cx, move |colour, cx| {
                    let _ = owner.update(cx, |v, cx| {
                        // The panel can outlive the row: its lease must still be this one.
                        let current = v.publication.surface.node(id);
                        if current.and_then(|n| n.props.get("binding")) == binding.as_ref() {
                            v.change(id, json!(colour), cx);
                        }
                    });
                });
            }
            _ => self.send(id, "click", EventDetail::default(), cx),
        }
    }
    fn element(
        &self,
        id: Id,
        window: &mut Window,
        cx: &mut Context<Self>,
        skip: Option<Id>,
    ) -> AnyElement {
        if skip == Some(id) {
            return div().into_any_element();
        }
        let surface = &self.publication.surface;
        let Some(node) = surface.node(id) else {
            return div().into_any_element();
        };
        let mut style = surface.computed_style(id).unwrap_or_default();
        if self.publication.panel == "viewer" {
            style.remove("background");
            style.remove("backgroundColor");
        }
        if self.publication.panel == "assets"
            && Component::of(node) == Some(Component::SegmentedControl)
        {
            return div().into_any_element(); // The owning shell supplies the dock tabs.
        }
        if style.get("display").and_then(Value::as_str) == Some("none") {
            return div().into_any_element();
        }
        let semantic = node.props.get("id").and_then(Value::as_str);
        if semantic == Some("timeline:splitter") {
            return div().into_any_element();
        }
        if semantic == Some("timeline:ruler") {
            return self.timeline_ruler(window);
        }
        let native_row = node
            .props
            .get("metadata")
            .and_then(|m| m.get("nativeRow"))
            .and_then(Value::as_str)
            .and_then(|id| {
                self.publication
                    .timeline
                    .as_ref()
                    .and_then(|scene| scene.rows.get(id))
            });
        let disabled = node.props.get("disabled") == Some(&Value::Bool(true));
        if let Some(control) = self.controls.get(&id) {
            return div().flex_none().child(control.clone()).into_any_element();
        }
        if let Some(field) = self.fields.get(&id) {
            let ax = self
                .panel
                .accessibility
                .nodes
                .iter()
                .find(|(node_id, _)| node_id.0 == id)
                .map(|(_, n)| n);
            let mut wrapper = div()
                .id(("field", id))
                .flex_1()
                .min_w(px(0.))
                .child(field.clone());
            if let Some(ax) = ax {
                wrapper = wrapper
                    .role(ax.role())
                    .aria_label(ax.label().unwrap_or("").to_owned());
                if let Some(value) = ax.value() {
                    wrapper = wrapper.aria_value(value.to_owned());
                }
            }
            if let Some(focus) = self.focus.get(&id) {
                wrapper = wrapper.track_focus(focus).tab_index(0);
            }
            let weak = cx.entity().downgrade();
            wrapper = wrapper.on_a11y_action(AccessibleAction::SetValue, move |data, _, cx| {
                let value = match data {
                    Some(accesskit::ActionData::NumericValue(n)) if n.is_finite() => json!(n),
                    Some(accesskit::ActionData::Value(text)) => json!(text.to_string()),
                    _ => return,
                };
                let _ = weak.update(cx, |v, cx| v.change(id, value, cx));
            });
            if Component::of(node) == Some(Component::NumberField) {
                let value = node
                    .props
                    .get("value")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.);
                let step = node.props.get("step").and_then(Value::as_f64).unwrap_or(1.);
                wrapper = wrapper
                    .aria_numeric_value(value)
                    .aria_numeric_value_step(step);
                for (action, direction) in [
                    (AccessibleAction::Increment, 1.),
                    (AccessibleAction::Decrement, -1.),
                ] {
                    let weak = cx.entity().downgrade();
                    wrapper = wrapper.on_a11y_action(action, move |_, _, cx| {
                        let _ = weak.update(cx, |v, cx| {
                            if let Some(node) = v.publication.surface.node(id) {
                                let current = node
                                    .props
                                    .get("value")
                                    .and_then(Value::as_f64)
                                    .unwrap_or(value);
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
                                v.change(
                                    id,
                                    json!((current + step * direction).clamp(min, max)),
                                    cx,
                                );
                            }
                        });
                    });
                }
            }
            let field = field.clone();
            wrapper = wrapper.on_a11y_action(AccessibleAction::Focus, move |_, _, cx| {
                field.update(cx, |f, cx| f.begin_editing(cx));
            });
            return wrapper.into_any_element();
        }
        let mut element = div().id(("v4", id)).flex().flex_col().min_w(px(0.));
        if id == 0 || node.parent == Some(0) {
            element = element.size_full();
        }
        if let Some(recipe) = self.panel.components.get(&id) {
            match Component::of(node) {
                Some(Component::InspectorRow) => {
                    element = element
                        .flex_row()
                        .items_center()
                        .min_h(px(self.tokens.metrics.row));
                }
                Some(Component::ToolbarItem | Component::DisclosureButton) => {
                    element = element
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .min_w(px(recipe.hit))
                        .min_h(px(recipe.hit))
                        .rounded(px(recipe
                            .radius
                            .unwrap_or(self.tokens.metrics.control_radius)));
                }
                Some(Component::ListItem | Component::TreeItem) => {
                    element = element
                        .flex_row()
                        .items_center()
                        .min_h(px(self.tokens.metrics.row));
                }
                Some(Component::PopUpButton | Component::NumberField | Component::TextField) => {
                    element = element
                        .flex_row()
                        .items_center()
                        .px(px(self.tokens.metrics.gap))
                        .h(px(recipe
                            .visual_height
                            .unwrap_or(self.tokens.metrics.field)))
                        .bg(rgb(recipe.background))
                        .rounded(px(recipe
                            .radius
                            .unwrap_or(self.tokens.metrics.control_radius)));
                }
                Some(Component::SegmentedControl) => {
                    element = element
                        .flex_row()
                        .p(px(recipe.track_padding.unwrap_or(0.)))
                        .gap(px(0.))
                        .bg(rgb(recipe.background))
                        .rounded(px(recipe
                            .radius
                            .unwrap_or(self.tokens.metrics.control_radius)));
                }
                Some(Component::Segment) => {
                    element = element
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .px(px(self.tokens.metrics.gap))
                        .h(px(self.tokens.metrics.field));
                }
                Some(Component::FormGroup) => {
                    element = element
                        .bg(rgb(recipe.background))
                        .rounded(px(self.tokens.metrics.group_radius))
                        .overflow_hidden();
                }
                Some(Component::EmptyState) => {
                    element = element
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap(px(self.tokens.metrics.gap));
                }
                Some(Component::List | Component::Tree) => {
                    element = element.flex_1().overflow_y_scroll();
                }
                _ => {}
            }
        }
        element = styled(element, &style, &self.tokens);
        match node.props.get("data-pm-slot").and_then(Value::as_str) {
            Some("label") => element = element.flex_1(),
            Some("value") => {
                element = element
                    .flex_row()
                    .items_center()
                    .gap(px(self.tokens.metrics.gap))
            }
            _ => {}
        }
        if let Some(header) = self.header_id()
            && id == header
        {
            element = element.flex_none().w_full();
        }
        if let Some(scroll) = self.scrolls.get(&id) {
            element = element
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .track_scroll(scroll);
        }
        let virtual_window = if node.r#type == "virtual-list" {
            node.props
                .get("count")
                .and_then(Value::as_u64)
                .zip(node.props.get("range").and_then(Value::as_array))
                .zip(
                    node.props
                        .get("itemSize")
                        .and_then(Value::as_f64)
                        .or_else(|| {
                            (node
                                .props
                                .get("metadata")
                                .and_then(|m| m.get("rowHeightToken"))
                                .and_then(Value::as_str)
                                == Some("metrics.row"))
                            .then_some(self.tokens.metrics.row as f64)
                        }),
                )
                .and_then(|((count, range), size)| {
                    pm_builtins::virtualization::Window::new(
                        count as usize,
                        (
                            range.first()?.as_u64()? as usize,
                            range.get(1)?.as_u64()? as usize,
                        ),
                        size,
                    )
                    .ok()
                })
        } else {
            None
        };
        if let Some(geometry) = virtual_window {
            element = element
                .aria_row_count(geometry.count)
                .on_scroll_wheel(cx.listener(move |_, _, window, cx| {
                    cx.defer_in(window, move |v, _, cx| {
                        let Some(scroll) = v.scrolls.get(&id) else {
                            return;
                        };
                        let offset = -f32::from(scroll.offset().y) as f64;
                        let height = f32::from(scroll.bounds().size.height) as f64;
                        if let Ok(range) = geometry.visible(offset, height) {
                            v.send(
                                id,
                                "rangechange",
                                EventDetail {
                                    range: Some(range),
                                    ..Default::default()
                                },
                                cx,
                            );
                        }
                    });
                }));
        }
        if node.props.get("aria-selected") == Some(&Value::Bool(true))
            || node.props.get("state").and_then(|s| s.get("selected")) == Some(&Value::Bool(true))
        {
            element = element.bg(rgb(self.tokens.colors.selected_fill));
        }
        let ax = self
            .panel
            .accessibility
            .nodes
            .iter()
            .find(|(node_id, _)| node_id.0 == id)
            .map(|(_, n)| n);
        if let Some(ax) = ax {
            if ax.role() != Role::GenericContainer {
                element = element.role(ax.role());
            }
            if let Some(label) = ax.label() {
                element = element.aria_label(label.to_owned());
            }
            if let Some(value) = ax.value() {
                element = element.aria_value(value.to_owned());
            }
            if ax.is_disabled() {
                element = element.aria_disabled(true);
            }
            if let Some(selected) = ax.is_selected() {
                element = element.aria_selected(selected);
            }
            if let Some(expanded) = ax.is_expanded() {
                element = element.aria_expanded(expanded);
            }
        }
        if let Some(semantic) = node.props.get("id").and_then(Value::as_str) {
            element =
                element.accessibility_id(format!("v4.{}.{}", self.publication.panel, semantic));
        }
        if let Some(total) = node
            .props
            .get("metadata")
            .and_then(|m| m.get("totalRows").or_else(|| m.get("totalCount")))
            .and_then(Value::as_u64)
        {
            element = element.aria_row_count(total as usize);
        }
        if let Some(focus) = self.focus.get(&id) {
            element = element
                .track_focus(focus)
                .tab_index(
                    node.props
                        .get("tabIndex")
                        .and_then(Value::as_i64)
                        .unwrap_or(0) as isize,
                )
                .focus_visible_style(|s| {
                    s.ring(px(self.tokens.metrics.border * 2.))
                        .ring_color(rgb(self.tokens.colors.focus))
                });
        }
        let component = Component::of(node);
        let clickable = node.listeners.contains_key("click")
            || matches!(
                component,
                Some(
                    Component::PopUpButton
                        | Component::Checkbox
                        | Component::Toggle
                        | Component::ColourWell
                )
            );
        if !disabled && clickable {
            element = element
                .cursor_pointer()
                .hover_style(|s| s.bg(rgb(self.tokens.colors.hover)))
                .on_click(cx.listener(move |v, _, window, cx| {
                    cx.stop_propagation();
                    v.activate(id, window, cx);
                }));
        }
        // Finder drops reach the Project panel's own targets; the window's
        // drop handler places anything landing elsewhere at the playhead.
        if !disabled && node.listeners.contains_key("drop") && self.publication.panel == "assets" {
            let target = rgb(self.tokens.colors.selected_fill);
            element = element
                .on_drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
                .on_drop(cx.listener(move |v, paths: &ExternalPaths, window, cx| {
                    let detail = file_drop(paths.paths(), window.modifiers().alt);
                    v.send(id, "drop", detail, cx);
                }));
        }
        if node.parent == Some(0)
            && surface
                .nodes()
                .values()
                .any(|n| n.listeners.contains_key("keydown"))
        {
            element =
                element.on_key_down(cx.listener(move |v, event: &KeyDownEvent, window, cx| {
                    let key = match event.keystroke.key.as_str() {
                        "up" => "ArrowUp",
                        "down" => "ArrowDown",
                        "left" => "ArrowLeft",
                        "right" => "ArrowRight",
                        "enter" => "Enter",
                        "escape" => "Escape",
                        "space" => " ",
                        other => other,
                    };
                    let id = v
                        .focus
                        .iter()
                        .find(|(_, h)| h.is_focused(window))
                        .map(|(id, _)| *id)
                        .unwrap_or(id);
                    if pm_surface::interaction::Interaction::default()
                        .plan(&v.publication.surface, id, None, "keydown", Some(key))
                        .is_ok_and(|p| p.prevent_default)
                    {
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                    v.send(
                        id,
                        "keydown",
                        EventDetail {
                            key: Some(key.into()),
                            repeat: Some(event.is_held),
                            shift_key: Some(event.keystroke.modifiers.shift),
                            alt_key: Some(event.keystroke.modifiers.alt),
                            meta_key: Some(event.keystroke.modifiers.platform),
                            ctrl_key: Some(event.keystroke.modifiers.control),
                            ..Default::default()
                        },
                        cx,
                    );
                }));
        }
        if let Some(symbol) = node
            .props
            .get("metadata")
            .and_then(|m| m.get("symbol").or_else(|| m.get("requiredSymbol")))
            .or_else(|| node.props.get("attr").and_then(|m| m.get("symbol")))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            let ink = if disabled {
                self.tokens.colors.muted
            } else {
                self.tokens.colors.secondary
            };
            element = element.child(pm_shell::symbol::symbol(
                symbol,
                self.tokens.metrics.symbol_panel,
                pm_shell::symbol::Weight::Regular,
                ink,
                window,
            ));
        }
        if matches!(component, Some(Component::Checkbox | Component::Toggle)) {
            let on = node
                .props
                .get("value")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            element = element.child(pm_shell::symbol::symbol(
                if on {
                    "checkmark.square.fill"
                } else {
                    "square"
                },
                self.tokens.metrics.symbol_panel,
                pm_shell::symbol::Weight::Regular,
                self.tokens.colors.secondary,
                window,
            ));
        }
        if !node.text.is_empty() {
            element = element.child(node.text.clone());
        }
        let mut row_label = div()
            .flex()
            .items_center()
            .justify_between()
            .flex_none()
            .w(px(self.tokens.metrics.layer_column))
            .h_full()
            .px(px(self.tokens.metrics.inset))
            .bg(rgb(if native_row.is_some_and(|r| r.selected) {
                self.tokens.colors.selected_fill
            } else {
                self.tokens.colors.panel_secondary
            }))
            .border_r(px(self.tokens.metrics.border))
            .border_color(rgb(self.tokens.colors.separator));
        if let Some(geometry) = virtual_window {
            let (before, _) = geometry.spacers();
            element = element.child(div().flex_none().h(px(before as f32)));
        }
        for child in &node.children {
            let child_node = surface.node(*child).unwrap();
            if child_node.r#type == "text"
                && child_node.props.is_empty()
                && child_node.children.is_empty()
            {
                if native_row.is_some() {
                    row_label = row_label.child(child_node.text.clone());
                } else {
                    element = element.child(child_node.text.clone());
                }
            } else {
                if native_row.is_some() {
                    row_label = row_label.child(self.element(*child, window, cx, skip));
                } else {
                    element = element.child(self.element(*child, window, cx, skip));
                }
            }
        }
        if let Some(geometry) = virtual_window {
            let (_, after) = geometry.spacers();
            element = element.child(div().flex_none().h(px(after as f32)));
        }
        if let Some(row) = native_row {
            element = element
                .child(row_label)
                .child(self.timeline_track(row, window, cx));
        }
        element.into_any_element()
    }
    fn timeline_ruler(&self, _window: &mut Window) -> AnyElement {
        let Some(scene) = &self.publication.timeline else {
            return div().into_any_element();
        };
        let t = &self.tokens;
        let mut ruler = div()
            .relative()
            .flex_1()
            .h(px(t.metrics.ruler))
            .overflow_hidden();
        let step = crate::panels::ruler_step(scene.fps, 100., 72.);
        let last = (scene.duration * scene.fps / step).ceil().min(512.) as usize;
        for index in 0..=last {
            let frame = index as f64 * step;
            let fraction = (frame / scene.fps / scene.duration.max(0.001)) as f32;
            if fraction > 1. {
                continue;
            }
            ruler = ruler.child(
                div()
                    .absolute()
                    .left(relative(fraction))
                    .top(px(t.metrics.inner_radius))
                    .text_size(px(t.typography.minimum))
                    .text_color(rgb(t.colors.secondary))
                    .child(crate::panels::ruler_label(frame, scene.fps)),
            );
        }
        div()
            .id("native-timeline-ruler")
            .flex()
            .flex_row()
            .h(px(t.metrics.ruler))
            .w_full()
            .role(Role::Slider)
            .aria_label("Playhead")
            .aria_value(format!("{} seconds", scene.time))
            .child(
                div()
                    .flex_none()
                    .w(px(t.metrics.layer_column))
                    .h_full()
                    .bg(rgb(t.colors.panel_secondary))
                    .border_r(px(t.metrics.border))
                    .border_color(rgb(t.colors.separator)),
            )
            .child(ruler)
            .into_any_element()
    }
    fn timeline_track(
        &self,
        row: &pm_builtins::timeline_scene::Row,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = &self.tokens;
        let scene = self.publication.timeline.as_ref().unwrap();
        let duration = scene.duration.max(0.001);
        let height = if row.property {
            t.metrics.property_row
        } else {
            t.metrics.track_row
        };
        let mut track = div()
            .id(format!("native-track:{}", row.layer))
            .relative()
            .flex_1()
            .h(px(height))
            .overflow_hidden();
        if !row.property {
            let from = (row.from / duration).clamp(0., 1.) as f32;
            let span = (row.duration / duration).clamp(0., 1. - from as f64) as f32;
            let fill = crate::panels::label_color(&row.kind, &t.colors.labels);
            let fill = if row.selected {
                crate::panels::mix(
                    fill,
                    t.colors.text,
                    if t.scheme == pm_design::Scheme::Dark {
                        0.12
                    } else {
                        0.06
                    },
                )
            } else {
                fill
            };
            let clip = div()
                .absolute()
                .left(relative(from))
                .w(relative(span))
                .top(px(t.metrics.inner_radius))
                .h(px(height - 2. * t.metrics.inner_radius))
                .rounded(px(t.metrics.inner_radius))
                .bg(rgb(fill))
                .px(px(t.metrics.inset - t.metrics.inner_radius))
                .overflow_hidden()
                .flex()
                .items_center()
                .text_size(px(t.typography.body))
                .text_color(rgb(t.colors.text))
                .when(row.selected, |d| {
                    d.border(px(t.metrics.playhead))
                        .border_color(rgb(t.colors.text))
                })
                .child(row.name.clone());
            track = track.child(clip);
        }
        for key in &row.keys {
            let size = if row.property {
                t.metrics.keyframe
            } else {
                t.metrics.keyframe - 2.
            };
            let fraction = (key.time / duration) as f32;
            let layer = row.layer.clone();
            let channel = key.channel.clone();
            let key_id = key.id.clone();
            let selected = key.selected;
            let shape = if matches!(key.interpolation.as_str(), "ease" | "bezier") {
                "circle.fill"
            } else {
                "diamond.fill"
            };
            let color = if selected {
                t.colors.accent
            } else {
                t.colors.text
            };
            let button = div()
                .id(format!("key:{}", key.id))
                .absolute()
                .left(relative(fraction))
                .ml(px(-size / 2.))
                .top(px(if row.property {
                    (height - size) / 2.
                } else {
                    height - t.metrics.inner_radius - size / 2.
                }))
                .size(px(size))
                .role(Role::Button)
                .accessibility_id(format!("timeline.key.{}", key.id))
                .aria_label(format!(
                    "Keyframe at {:.3} seconds, {}{}",
                    key.time,
                    key.channel,
                    if selected { ", selected" } else { "" }
                ))
                .aria_selected(selected)
                .cursor_pointer()
                .child(pm_shell::symbol::symbol(
                    shape,
                    size,
                    pm_shell::symbol::Weight::Regular,
                    color,
                    window,
                ))
                .on_click(cx.listener(move |v, event: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let additive = event.modifiers().shift || event.modifiers().platform;
                    let ticket = v.runtime.timeline_key(&layer, &channel, &key_id, additive);
                    v.run(ticket, cx);
                }));
            track = track.child(button);
        }
        track.into_any_element()
    }
}
/// A file drop as the panel's JS reads it (`public.file-url`, as on AppKit).
fn file_drop(paths: &[std::path::PathBuf], alt: bool) -> EventDetail {
    const FILES: &str = "public.file-url";
    let files = paths
        .iter()
        .map(|path| json!(path.to_string_lossy()))
        .collect();
    EventDetail {
        alt_key: Some(alt),
        drag: Some(DragData {
            types: vec![FILES.into()],
            data: BTreeMap::from([(FILES.to_owned(), Value::Array(files))]),
            effect: DragEffect::Copy,
        }),
        ..Default::default()
    }
}

impl Render for SurfaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(rgb(self.tokens.colors.text))
            .text_size(px(self.tokens.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .child(self.element(0, window, cx, self.header_id()))
    }
}
fn disabled_node(node: &pm_surface::Node) -> bool {
    node.props.get("disabled") == Some(&Value::Bool(true))
}
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}
fn resolved(value: &Value, tokens: &Value) -> Value {
    if let Some(token) = value.get("token").and_then(Value::as_str) {
        token
            .split('.')
            .fold(tokens, |v, key| v.get(key).unwrap_or(&Value::Null))
            .clone()
    } else {
        value.clone()
    }
}
fn styled(mut e: Stateful<Div>, style: &pm_surface::Props, t: &pm_design::Tokens) -> Stateful<Div> {
    let tokens = serde_json::to_value(t).unwrap();
    if style.get("display").and_then(Value::as_str) == Some("flex")
        && !style.contains_key("flexDirection")
    {
        e = e.flex_row();
    }
    for (key, value) in style {
        let value = resolved(value, &tokens);
        let n = value.as_f64().map(|n| n as f32);
        let s = value.as_str();
        match key.as_str() {
            "flexDirection" => {
                e = if s == Some("row") {
                    e.flex_row()
                } else {
                    e.flex_col()
                };
            }
            "alignItems" => {
                e = match s {
                    Some("center") => e.items_center(),
                    Some("end") => e.items_end(),
                    _ => e,
                };
            }
            "justifyContent" => {
                e = match s {
                    Some("center") => e.justify_center(),
                    Some("end") => e.justify_end(),
                    Some("space-between") => e.justify_between(),
                    _ => e,
                };
            }
            "gap" => {
                if let Some(n) = n {
                    e = e.gap(px(n));
                }
            }
            "padding" => {
                if let Some(n) = n {
                    e = e.p(px(n));
                }
            }
            "paddingLeft" => {
                if let Some(n) = n {
                    e = e.pl(px(n));
                }
            }
            "paddingRight" => {
                if let Some(n) = n {
                    e = e.pr(px(n));
                }
            }
            "paddingTop" => {
                if let Some(n) = n {
                    e = e.pt(px(n));
                }
            }
            "paddingBottom" => {
                if let Some(n) = n {
                    e = e.pb(px(n));
                }
            }
            "marginTop" => {
                if let Some(n) = n {
                    e = e.mt(px(n));
                }
            }
            "marginBottom" => {
                if let Some(n) = n {
                    e = e.mb(px(n));
                }
            }
            "height" => {
                if let Some(n) = n {
                    e = e.h(px(n));
                }
            }
            "width" => {
                if let Some(n) = n {
                    e = e.w(px(n));
                } else if s == Some("100%") {
                    e = e.w_full();
                }
            }
            "minHeight" => {
                if let Some(n) = n {
                    e = e.min_h(px(n));
                }
            }
            "minWidth" => {
                if let Some(n) = n {
                    e = e.min_w(px(n));
                }
            }
            "maxHeight" => {
                if let Some(n) = n {
                    e = e.max_h(px(n));
                }
            }
            "maxWidth" => {
                if let Some(n) = n {
                    e = e.max_w(px(n));
                }
            }
            "flexGrow" => {
                if n.is_some_and(|n| n > 0.) {
                    e = e.flex_1();
                }
            }
            "borderRadius" => {
                if let Some(n) = n {
                    e = e.rounded(px(n));
                }
            }
            "fontSize" => {
                if let Some(n) = n {
                    e = e.text_size(px(n));
                }
            }
            "fontWeight" => {
                if let Some(n) = n {
                    e = e.font_weight(FontWeight(n));
                }
            }
            "color" => {
                if let Some(color) = value.as_u64() {
                    e = e.text_color(rgb(color as u32));
                }
            }
            "background" | "backgroundColor" => {
                if let Some(color) = value.as_u64() {
                    e = e.bg(rgb(color as u32));
                }
            }
            "overflow" | "overflowY" => {
                if s == Some("auto") || s == Some("scroll") {
                    e = e.overflow_y_scroll();
                } else if s == Some("hidden") {
                    e = e.overflow_hidden();
                }
            }
            "whiteSpace" if s == Some("nowrap") => {
                e = e.whitespace_nowrap();
            }
            _ => {}
        }
    }
    e
}

pub struct HeaderView(pub Entity<SurfaceView>);
impl Render for HeaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0
            .update(cx, |v, cx| {
                v.header_id().map(|id| v.element(id, window, cx, None))
            })
            .unwrap_or_else(|| div().into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::file_drop;
    use serde_json::json;

    #[test]
    fn a_file_drop_carries_paths_under_the_key_the_project_panel_reads() {
        let detail = file_drop(&["/a b/clip.mov".into(), "/c.png".into()], true);
        assert_eq!(detail.alt_key, Some(true));
        let drag = detail.drag.unwrap();
        assert_eq!(drag.types, ["public.file-url"]);
        assert_eq!(
            drag.data["public.file-url"],
            json!(["/a b/clip.mov", "/c.png"])
        );
    }
}
