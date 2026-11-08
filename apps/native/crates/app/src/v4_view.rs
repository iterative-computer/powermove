//! GPUI host for accepted pm-surface trees. Publications compile on a worker;
//! render reads compiled facts only and never waits on JS, the kernel or disk.
//! Ordinary controls deliver events to JS; native surfaces own their own input.
use crate::v4_compile::{Kit, NodeView, Prepared, TokenTable};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, Publication};
use pm_shell::text::{TextEvent, TextField, TextKind};
use pm_surface::{
    Id,
    events::{DragData, DragEffect, EventDetail},
    kit::Component,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

/// Native surfaces by the opaque `surface` lease a `canvas`/`frame` node mounts.
#[path = "v4_agent.rs"]
pub mod agent;

#[derive(Clone, Default)]
pub struct Natives {
    pub views: Rc<RefCell<BTreeMap<String, AnyView>>>,
    pub outline: Rc<Outline>,
}
/// Scroll and column geometry the timeline surface shares with its JSX outline.
/// The outline row window stays JS; its position follows native scroll at once.
#[derive(Default)]
pub struct Outline {
    scroll_y: Cell<f32>,
    pub column: Cell<f32>,
    scene: RefCell<Option<Arc<pm_panel_timeline::render::Scene>>>,
    view: RefCell<Option<WeakEntity<SurfaceView>>>,
    surface: RefCell<Option<WeakEntity<crate::timeline_surface::TimelineSurface>>>,
}
impl Outline {
    pub fn bind_surface(&self, surface: WeakEntity<crate::timeline_surface::TimelineSurface>) {
        *self.surface.borrow_mut() = Some(surface);
    }
    /// A timeline value readout drags as one kernel gesture on the surface.
    fn scrub_property(&self, layer: &str, path: &str, at: Point<Pixels>, cx: &mut App) -> bool {
        self.surface
            .borrow()
            .as_ref()
            .and_then(|s| s.upgrade())
            .is_some_and(|s| {
                s.update(cx, |s, _| s.press_property(layer.into(), path.into(), at));
                true
            })
    }
    pub fn set_scene(&self, scene: Option<Arc<pm_panel_timeline::render::Scene>>) {
        *self.scene.borrow_mut() = scene;
    }
    pub fn bind(&self, view: WeakEntity<SurfaceView>) {
        *self.view.borrow_mut() = Some(view);
    }
    fn offset(&self, first: &str) -> f32 {
        let top = self
            .scene
            .borrow()
            .as_ref()
            .and_then(|s| s.scroll_for(first))
            .unwrap_or(0.) as f32;
        top - self.scroll_y.get()
    }
    pub fn scrolled(&self, scroll_y: f32, column: f32, rows: Option<(usize, usize)>, cx: &mut App) {
        let moved = self.scroll_y.get() != scroll_y || self.column.get() != column;
        self.scroll_y.set(scroll_y);
        self.column.set(column);
        if let Some(view) = self.view.borrow().as_ref().and_then(|v| v.upgrade()) {
            view.update(cx, |v, cx| {
                if moved {
                    cx.notify();
                }
                if let Some(rows) = rows {
                    v.outline_rows(rows, cx);
                }
            });
        }
    }
}
/// The only pointer the host models. Multi-touch reaches panels as gestures.
const POINTER: u64 = 1;

/// A press on a number field: a click edits, a drag scrubs (language.md §9).
struct ScrubDrag {
    id: Id,
    start: Point<Pixels>,
    number: crate::v4_compile::NumberFacts,
    binding: Option<SharedString>,
    active: bool,
}
enum ScrubOp {
    Begin,
    Value(SharedString, f64),
    Finish(bool),
}
#[derive(Clone, Copy)]
struct Capture {
    target: Id,
    button: u64,
}
pub struct SurfaceView {
    pub runtime: Arc<Builtins>,
    pub publication: Publication,
    table: Arc<TokenTable>,
    prepared: Rc<Prepared>,
    window: AnyWindowHandle,
    appkit: bool,
    natives: Natives,
    fields: BTreeMap<Id, Entity<TextField>>,
    controls: BTreeMap<Id, Entity<crate::v4_control::NativeControl>>,
    scrolls: BTreeMap<Id, ScrollHandle>,
    leases: BTreeMap<Id, Option<SharedString>>,
    option_sets: BTreeMap<Id, Value>,
    focus: BTreeMap<Id, FocusHandle>,
    subscriptions: BTreeMap<Id, Vec<Subscription>>,
    origin: Rc<Cell<Point<Pixels>>>,
    anchors: Rc<RefCell<HashMap<Id, Bounds<Pixels>>>>,
    capture: Option<Capture>,
    scrub: Option<ScrubDrag>,
    scrub_ops: VecDeque<ScrubOp>,
    scrub_gesture: Option<u64>,
    scrub_busy: bool,
    scrub_shown: Option<(Id, f64)>,
    move_busy: bool,
    queued_move: Option<(Id, EventDetail)>,
    latest: u64,
    requested: Option<(usize, usize)>,
    #[cfg(test)]
    pub renders: usize,
    #[cfg(test)]
    pub log: Vec<(Id, String, EventDetail)>,
    pub error: Option<String>,
}
impl SurfaceView {
    pub fn new(
        runtime: Arc<Builtins>,
        publication: Publication,
        tokens: pm_design::Tokens,
        natives: Natives,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        use raw_window_handle::HasWindowHandle;
        let table = Arc::new(TokenTable::new(tokens));
        let prepared = Rc::new(Prepared::new(
            &publication.panel,
            publication.surface.clone(),
            &table,
        ));
        let mut this = Self {
            runtime,
            publication,
            table,
            prepared,
            window: (*window).window_handle(),
            appkit: HasWindowHandle::window_handle(window).is_ok(),
            natives,
            fields: BTreeMap::new(),
            controls: BTreeMap::new(),
            scrolls: BTreeMap::new(),
            leases: BTreeMap::new(),
            option_sets: BTreeMap::new(),
            focus: BTreeMap::new(),
            subscriptions: BTreeMap::new(),
            origin: Rc::new(Cell::new(Point::default())),
            anchors: Default::default(),
            capture: None,
            scrub: None,
            scrub_ops: VecDeque::new(),
            scrub_gesture: None,
            scrub_busy: false,
            scrub_shown: None,
            move_busy: false,
            queued_move: None,
            latest: 0,
            requested: None,
            #[cfg(test)]
            renders: 0,
            #[cfg(test)]
            log: vec![],
            error: None,
        };
        this.reconcile(window, cx);
        this
    }
    /// A reopened panel window keeps its view; later publications install there.
    pub fn attach(&mut self, window: AnyWindowHandle) {
        self.window = window;
    }
    pub fn tokens(&self) -> &pm_design::Tokens {
        &self.table.tokens
    }
    /// Compile off the main thread; only the newest publication installs.
    pub fn publish(&mut self, publication: Publication, cx: &mut Context<Self>) {
        self.compile(publication, self.table.clone(), cx);
    }
    pub fn retheme(&mut self, tokens: pm_design::Tokens, cx: &mut Context<Self>) {
        let table = Arc::new(TokenTable::new(tokens));
        self.compile(self.publication.clone(), table, cx);
    }
    fn compile(
        &mut self,
        publication: Publication,
        table: Arc<TokenTable>,
        cx: &mut Context<Self>,
    ) {
        self.latest += 1;
        let ticket = self.latest;
        let window = self.window;
        cx.spawn(async move |this, cx| {
            let surface = publication.surface.clone();
            let panel = publication.panel.clone();
            let worker = table.clone();
            let prepared = cx
                .background_executor()
                .spawn(async move { Prepared::new(&panel, surface, &worker) })
                .await;
            let _ = cx.update_window(window, |_, window, cx| {
                this.update(cx, |v, cx| {
                    if v.latest == ticket {
                        if v.publication.generation != publication.generation {
                            v.restart();
                        }
                        v.table = table;
                        v.publication = publication;
                        v.prepared = Rc::new(prepared);
                        v.reconcile(window, cx);
                        cx.notify();
                    }
                })
            });
        })
        .detach();
    }
    /// A new generation (an extension reloaded, a fork swapped in) reuses node
    /// ids, so nothing bound to the old tree may carry over.
    fn restart(&mut self) {
        self.controls.clear();
        self.fields.clear();
        self.scrolls.clear();
        self.leases.clear();
        self.option_sets.clear();
        self.focus.clear();
        self.subscriptions.clear();
        self.anchors.borrow_mut().clear();
        self.capture = None;
        self.scrub = None;
        self.queued_move = None;
        self.error = None;
    }
    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let prepared = self.prepared.clone();
        let live = |id: &Id| prepared.nodes.contains_key(id);
        self.controls.retain(|id, _| live(id));
        self.scrolls.retain(|id, _| live(id));
        self.subscriptions.retain(|id, _| live(id));
        self.fields.retain(|id, _| live(id));
        self.focus.retain(|id, _| live(id));
        self.leases.retain(|id, _| live(id));
        self.option_sets.retain(|id, _| live(id));
        if self.capture.is_some_and(|c| !live(&c.target)) {
            self.capture = None;
        }
        let surface = self.publication.surface.clone();
        for view in prepared.nodes.values() {
            let id = view.id;
            let Some(node) = surface.node(id) else {
                continue;
            };
            let lease = view.binding.clone();
            let options = node.props.get("options").cloned().unwrap_or(Value::Null);
            if self.leases.get(&id) != Some(&lease) || self.option_sets.get(&id) != Some(&options) {
                self.subscriptions.remove(&id);
                self.fields.remove(&id);
                self.controls.remove(&id);
                self.leases.insert(id, lease.clone());
                self.option_sets.insert(id, options);
            }
            if view.scroll_y || view.scroll_x {
                self.scrolls.entry(id).or_default();
            }
            let kind = self
                .appkit
                .then(|| control_kind(&self.publication.panel, node))
                .flatten();
            if let Some(kind) = kind {
                self.mount_control(kind, view, node, lease, window, cx);
                continue;
            }
            if !view.disabled
                && (view.component.is_some_and(|c| c.interactive())
                    || view.tab_index.is_some_and(|v| v >= 0))
            {
                self.focus.entry(id).or_insert_with(|| cx.focus_handle());
            }
            if !self.appkit
                || !matches!(
                    view.component,
                    Some(Component::NumberField | Component::TextField)
                )
                || view.disabled
            {
                continue;
            }
            self.mount_field(view, node, lease, window, cx);
        }
    }
    fn mount_control(
        &mut self,
        kind: crate::v4_control::Kind,
        view: &NodeView,
        node: &pm_surface::Node,
        lease: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.id;
        let value = node.props.get("value").cloned().unwrap_or(Value::Null);
        if let Some(control) = self.controls.get(&id) {
            let options = node
                .props
                .get("options")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            control.update(cx, |v, cx| v.update(&value, options, view.disabled, cx));
            return;
        }
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
        let tokens = self.table.tokens.clone();
        let disabled = view.disabled;
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
                &tokens,
                window,
                cx,
            )
        });
        let generation = self.publication.generation;
        self.subscriptions.entry(id).or_default().push(cx.subscribe(
            &control,
            move |v, _, value, cx| {
                if v.publication.generation != generation || v.leases.get(&id) != Some(&lease) {
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
    fn mount_field(
        &mut self,
        view: &NodeView,
        node: &pm_surface::Node,
        lease: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.id;
        // Number fields read at their display precision, as when scrubbed.
        let value = match (view.number, node.props.get("value")) {
            (Some(number), Some(value)) if value.is_number() => {
                number.text(value.as_f64().unwrap_or(number.value))
            }
            (_, value) => value.map(value_text).unwrap_or_default(),
        };
        let placeholder = node
            .props
            .get("metadata")
            .and_then(|m| m.get("placeholder"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if let Some(field) = self.fields.get(&id) {
            let tokens = self.table.tokens.clone();
            field.update(cx, |f, cx| {
                f.set_tokens(tokens);
                f.set_placeholder(&placeholder);
                f.set_value(&value, cx);
            });
            return;
        }
        let number = view.component == Some(Component::NumberField);
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
        let tokens = self.table.tokens.clone();
        let field = cx.new(|cx| {
            let mut field = TextField::new(kind, label, &value, tokens, window, cx);
            field.set_placeholder(&placeholder);
            field
        });
        self.subscriptions.entry(id).or_default().push(cx.subscribe(
            &field,
            move |this, _, event, cx| {
                if this.prepared.node(id).and_then(|n| n.binding.clone()) != lease {
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
                    TextEvent::Cancelled => {
                        if this.prepared.node(id).is_some_and(|n| n.cancel_change) {
                            this.change(id, json!(""), cx);
                        }
                    }
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
    /// Ask JS for the row window that covers native scroll, when it left the
    /// published one. Rows already overscan a viewport on each side.
    pub fn outline_rows(&mut self, rows: (usize, usize), cx: &mut Context<Self>) {
        let Some(node) = self
            .prepared
            .nodes
            .values()
            .find(|n| n.outline_window.is_some())
        else {
            return;
        };
        let (id, (first, count)) = (node.id, node.outline_window.unwrap());
        if rows.1 > rows.0
            && (rows.0 < first || rows.1 > first + count)
            && self.requested != Some(rows)
        {
            self.requested = Some(rows);
            self.send(
                id,
                "rangechange",
                EventDetail {
                    range: Some(rows),
                    ..Default::default()
                },
                cx,
            );
        }
    }
    #[cfg(test)]
    pub fn change_semantic(&mut self, semantic: &str, value: Value, cx: &mut Context<Self>) {
        let id = self.prepared.semantics[semantic];
        self.change(id, value, cx);
    }
    #[cfg(test)]
    pub fn id_of_text(&self, text: &str) -> Option<Id> {
        let surface = &self.publication.surface;
        let parent = |n: &NodeView| n.parent;
        self.prepared
            .nodes
            .values()
            .find(|n| n.text.as_deref() == Some(text))
            .map(|n| {
                parent(n)
                    .filter(|p| surface.node(*p).is_some())
                    .unwrap_or(n.id)
            })
    }
    #[cfg(test)]
    pub fn semantic_ids(&self) -> Vec<String> {
        self.prepared.semantics.keys().cloned().collect()
    }
    #[cfg(test)]
    pub fn click_semantic(&mut self, semantic: &str, cx: &mut Context<Self>) {
        let id = self.prepared.semantics[semantic];
        self.send(id, "click", EventDetail::default(), cx);
    }
    #[cfg(test)]
    pub fn cancellable_field(&self) -> Option<Id> {
        self.prepared
            .nodes
            .values()
            .find(|n| n.cancel_change)
            .map(|n| n.id)
    }
    #[cfg(test)]
    pub fn first_number(&self) -> Option<String> {
        let mut numbers: Vec<_> = self
            .prepared
            .nodes
            .values()
            .filter(|n| n.number.is_some() && n.binding.is_some() && !n.disabled)
            .collect();
        numbers.sort_by_key(|n| n.id);
        numbers.first().map(|n| {
            n.semantic
                .as_ref()
                .map_or_else(|| format!("n{}", n.id), |s| s.to_string())
        })
    }
    #[cfg(test)]
    pub fn semantic_id_starting(&self, prefix: &str) -> Option<String> {
        self.prepared
            .semantics
            .keys()
            .find(|k| k.starts_with(prefix))
            .cloned()
    }
    pub fn has_header(&self) -> bool {
        self.prepared.header.is_some()
    }
    pub fn scroll_semantic(&mut self, semantic: &str, cx: &mut Context<Self>) {
        if let Some(id) = self.prepared.semantics.get(semantic)
            && let Some(node) = self.prepared.node(*id)
            && let Some(parent) = node.parent.and_then(|id| self.prepared.node(id))
            && let Some(scroll) = self.scrolls.get(&parent.id)
            && let Some(index) = parent.children.iter().position(|id| *id == node.id)
        {
            scroll.scroll_to_top_of_item(index);
            cx.notify();
        }
    }
    pub fn focus_semantic(&self, semantic: &str, window: &mut Window, cx: &mut App) {
        if let Some(id) = self.prepared.semantics.get(semantic) {
            if let Some(field) = self.fields.get(id) {
                field.update(cx, |f, cx| f.begin_editing(cx));
            } else if let Some(focus) = self.focus.get(id) {
                focus.focus(window, cx);
            }
        }
    }
    /// Re-stage AppKit slots whose cached paint did not run this frame.
    pub fn keep_native_slots(&self, cx: &App) {
        for control in self.controls.values() {
            control.read(cx).keep();
        }
        for field in self.fields.values() {
            field.read(cx).keep();
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
        #[cfg(test)]
        self.log.push((id, event.into(), detail.clone()));
        let runtime = self.runtime.clone();
        let p = self.publication.clone();
        let event = event.to_owned();
        let semantic = self.prepared.node(id).and_then(|n| n.semantic.clone());
        // Native planning can read model state. Keep its locks off the main thread.
        cx.spawn(async move |this, cx| {
            let ticket = cx
                .background_executor()
                .spawn(async move {
                    runtime.activate(
                        &p.panel,
                        p.generation,
                        p.surface.revision(),
                        id,
                        semantic.as_deref(),
                        &event,
                        detail,
                    )
                })
                .await;
            let _ = this.update(cx, |v, cx| v.run(ticket, cx));
        })
        .detach();
    }
    /// Pointer stream events address the current tree: a publication landing
    /// mid-drag must not lose the release. Moves coalesce to one in flight.
    fn pointer(
        &mut self,
        id: Id,
        event: &'static str,
        detail: EventDetail,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        self.log.push((id, event.into(), detail.clone()));
        if event == "pointermove" {
            if self.move_busy {
                self.queued_move = Some((id, detail));
                return;
            }
            self.move_busy = true;
        }
        let runtime = self.runtime.clone();
        let p = self.publication.clone();
        cx.spawn(async move |this, cx| {
            let ticket = cx
                .background_executor()
                .spawn(
                    async move { runtime.pointer_event(&p.panel, p.generation, id, event, detail) },
                )
                .await;
            let result = match ticket {
                Ok(ticket) => ticket.await.map(|_| ()),
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |v, cx| {
                if let Err(error) = result {
                    v.error = Some(error.to_string());
                }
                if event == "pointermove" {
                    v.move_busy = false;
                    if let Some((id, detail)) = v.queued_move.take() {
                        v.pointer(id, "pointermove", detail, cx);
                    }
                }
            });
        })
        .detach();
    }
    pub fn change(&mut self, id: Id, value: Value, cx: &mut Context<Self>) {
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
    fn activate(
        &mut self,
        id: Id,
        detail: EventDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            Some(Component::PopUpButton) if self.appkit => {
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
            _ => self.send(id, "click", detail, cx),
        }
    }
    fn detail(
        &self,
        position: Point<Pixels>,
        button: Option<u64>,
        buttons: u64,
        clicks: usize,
        modifiers: Modifiers,
    ) -> EventDetail {
        let origin = self.origin.get();
        EventDetail {
            pointer_id: Some(POINTER),
            x: Some(f32::from(position.x - origin.x) as f64),
            y: Some(f32::from(position.y - origin.y) as f64),
            button,
            buttons: Some(buttons),
            click_count: Some(clicks as u64),
            ..modified(modifiers)
        }
    }
    fn press(&mut self, id: Id, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let button = button_code(event.button);
        self.capture = Some(Capture { target: id, button });
        let detail = self.detail(
            event.position,
            Some(button),
            1 << button,
            event.click_count,
            event.modifiers,
        );
        self.pointer(id, "pointerdown", detail, cx);
    }
    fn scrub_press(&mut self, id: Id, event: &MouseDownEvent, window: &mut Window) {
        let Some(node) = self.prepared.node(id) else {
            return;
        };
        let Some(number) = node.number else {
            return;
        };
        // A press must not focus (and so open) the text field before a drag decides.
        window.prevent_default();
        self.scrub = Some(ScrubDrag {
            id,
            start: event.position,
            number,
            binding: node.binding.clone().filter(|_| !node.local_change),
            active: false,
        });
    }
    fn scrub_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let threshold = self.table.tokens.metrics.drag_threshold;
        let Some(drag) = &mut self.scrub else {
            return;
        };
        let delta = event.position - drag.start;
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if !drag.active {
            if dx.hypot(dy) < threshold {
                return;
            }
            drag.active = true;
            if drag.binding.is_some() {
                self.scrub_ops.push_back(ScrubOp::Begin);
            }
        }
        let n = drag.number;
        let Ok(value) = pm_surface::binding::scrub(pm_surface::binding::Scrub {
            start: n.value,
            delta: (dx as f64, dy as f64),
            step: n.step,
            speed: n.speed,
            shift: event.modifiers.shift,
            alt: event.modifiers.alt,
            bounds: (n.min, n.max),
        }) else {
            return;
        };
        let id = drag.id;
        self.scrub_shown = Some((id, value));
        match drag.binding.clone() {
            Some(binding) => {
                self.scrub_ops.push_back(ScrubOp::Value(binding, value));
                self.scrub_pump(cx);
            }
            None => self.change(id, json!(value), cx),
        }
        if let Some(field) = self.fields.get(&id) {
            let text = n.text(value);
            field.update(cx, |f, cx| f.set_value(&text, cx));
        }
        cx.notify();
    }
    fn scrub_release(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(drag) = self.scrub.take() else {
            return;
        };
        if drag.active {
            if drag.binding.is_some() {
                self.scrub_ops.push_back(ScrubOp::Finish(commit));
                self.scrub_pump(cx);
            }
        } else if let Some(field) = self.fields.get(&drag.id) {
            field.update(cx, |f, cx| f.begin_editing(cx));
        }
    }
    /// Escape during a scrub restores the value through the kernel gesture.
    pub fn cancel_gesture(&mut self, cx: &mut Context<Self>) -> bool {
        if self.scrub.is_none() {
            return false;
        }
        self.scrub_release(false, cx);
        true
    }
    fn scrub_pump(&mut self, cx: &mut Context<Self>) {
        if self.scrub_busy {
            return;
        }
        let Some(mut op) = self.scrub_ops.pop_front() else {
            return;
        };
        // Only the newest value matters while the kernel is busy.
        while matches!(op, ScrubOp::Value(..))
            && matches!(self.scrub_ops.front(), Some(ScrubOp::Value(..)))
        {
            op = self.scrub_ops.pop_front().unwrap();
        }
        self.scrub_busy = true;
        let runtime = self.runtime.clone();
        let panel = self.publication.panel.clone();
        let gesture = self.scrub_gesture;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let pending = match (op, gesture) {
                        (ScrubOp::Begin, _) => runtime.scrub_begin(&panel),
                        (ScrubOp::Value(handle, value), Some(gesture)) => {
                            runtime.scrub_step(&panel, &handle, gesture, json!(value))
                        }
                        (ScrubOp::Finish(commit), Some(gesture)) => {
                            runtime.scrub_finish(&panel, gesture, commit)
                        }
                        _ => return Err("Scrub gesture was refused".to_owned()),
                    }
                    .map_err(|e| e.to_string())?;
                    pending
                        .await
                        .ok_or("Kernel closed")?
                        .map_err(|e| format!("{e:?}"))
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.scrub_busy = false;
                match result {
                    Ok(ack) => match ack.reply {
                        pm_kernel::Reply::Gesture(id) => v.scrub_gesture = Some(id),
                        pm_kernel::Reply::Throw(error) => {
                            v.error = Some(error.to_string());
                        }
                        _ => {}
                    },
                    Err(error) => v.error = Some(error),
                }
                if v.scrub_ops.is_empty() && v.scrub.is_none() {
                    v.scrub_gesture = None;
                    v.scrub_shown = None;
                }
                v.scrub_pump(cx);
            });
        })
        .detach();
    }
    /// Capture-phase window listeners: a pressed pointer keeps its target until
    /// release, wherever it travels, like AppKit drag tracking.
    fn tracking(cx: &mut Context<Self>, origin: Rc<Cell<Point<Pixels>>>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        canvas(
            move |bounds, _, _| origin.set(bounds.origin),
            move |_, _, window, _| {
                let moved = weak.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let _ = moved.update(cx, |v, cx| {
                        if v.scrub.is_some() {
                            cx.stop_propagation();
                            v.scrub_move(event, cx);
                            return;
                        }
                        let Some(capture) = v.capture else {
                            return;
                        };
                        cx.stop_propagation();
                        if v.prepared
                            .node(capture.target)
                            .is_some_and(|n| n.events.pointer_move)
                        {
                            let detail = v.detail(
                                event.position,
                                None,
                                1 << capture.button,
                                0,
                                event.modifiers,
                            );
                            v.pointer(capture.target, "pointermove", detail, cx);
                        }
                    });
                });
                let released = weak.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let _ = released.update(cx, |v, cx| {
                        v.scrub_release(true, cx);
                        let Some(capture) = v.capture.take() else {
                            return;
                        };
                        if v.prepared
                            .node(capture.target)
                            .is_some_and(|n| n.events.pointer_up)
                        {
                            let detail = v.detail(
                                event.position,
                                Some(capture.button),
                                0,
                                event.click_count,
                                event.modifiers,
                            );
                            v.pointer(capture.target, "pointerup", detail, cx);
                        }
                    });
                });
            },
        )
        .absolute()
        .inset_0()
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
        let Some(node) = self.prepared.node(id) else {
            return div().into_any_element();
        };
        if node.hidden {
            return div().into_any_element();
        }
        if let Some(control) = self.controls.get(&id) {
            return div().flex_none().child(control.clone()).into_any_element();
        }
        if let Some(field) = self.fields.get(&id) {
            return self.field_element(node, field, cx);
        }
        let mut element = div().id(("v4", id));
        *element.style() = node.style.clone();
        if node.outline_column && self.natives.outline.column.get() > 0. {
            element = element.w(px(self.natives.outline.column.get()));
        }
        if let Some(first) = &node.outline_first {
            element = element
                .absolute()
                .left(px(0.))
                .right(px(0.))
                .top(px(self.natives.outline.offset(first)));
        }
        #[cfg(test)]
        {
            let name = node
                .semantic
                .as_ref()
                .map_or_else(|| format!("n{id}"), |s| s.to_string());
            element = element.debug_selector(|| name);
        }
        if let Some(scroll) = self.scrolls.get(&id) {
            element = element.flex_1().min_h(px(0.)).track_scroll(scroll);
            if node.scroll_y {
                element = element.overflow_y_scroll();
            }
            if node.scroll_x {
                element = element.overflow_x_scroll();
            }
        }
        if let Some(geometry) = node.virtual_window {
            element = element.on_scroll_wheel(cx.listener(move |_, _, window, cx| {
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
        element = self.accessible(element, node);
        if let Some(focus) = self.focus.get(&id) {
            let t = &self.table.tokens;
            let (ring, color) = (px(t.metrics.border * 2.), rgb(t.colors.focus));
            element = element
                .track_focus(focus)
                .tab_index(node.tab_index.unwrap_or(0))
                .focus_visible_style(move |s| s.ring(ring).ring_color(color));
        }
        element = self.interactive(element, node, cx);
        element = self.decorate(element, node, window);
        let mut children: Vec<AnyElement> = vec![];
        if let Some(geometry) = node.virtual_window {
            let (before, _) = geometry.spacers();
            children.push(div().flex_none().h(px(before as f32)).into_any_element());
        }
        for child in &node.children {
            match self.prepared.node(*child) {
                Some(c) if c.is_plain_text => {
                    if let Some(text) = &c.text {
                        children.push(text.clone().into_any_element());
                    }
                }
                _ => children.push(self.element(*child, window, cx, skip)),
            }
        }
        if let Some(geometry) = node.virtual_window {
            let (_, after) = geometry.spacers();
            children.push(div().flex_none().h(px(after as f32)).into_any_element());
        }
        if let Some(surface) = &node.native
            && let Some(view) = self.natives.views.borrow().get(surface.as_ref())
        {
            children.push(
                view.clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element(),
            );
        }
        let element = element.children(children);
        self.place(element.into_any_element(), node, window, cx)
    }
    /// Anchored kit surfaces escape the panel clip and stack above it.
    fn place(
        &self,
        element: AnyElement,
        node: &NodeView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Kit::Anchored {
            anchor,
            point: at,
            dismiss,
        } = node.kit
        else {
            return element;
        };
        let t = &self.table.tokens;
        let origin = self.origin.get();
        let position = if node.component == Some(Component::SheetAnchor) {
            let width = window.viewport_size().width;
            point(
                (width - px(t.metrics.sheet_width)) / 2.,
                px(t.metrics.toolbar),
            )
        } else if let Some(bounds) = anchor.and_then(|id| self.anchors.borrow().get(&id).copied()) {
            bounds.bottom_left()
        } else if let Some((x, y)) = at {
            origin + point(px(x), px(y))
        } else {
            origin
        };
        let id = node.id;
        let mut wrapper = div().occlude().child(element);
        if dismiss {
            wrapper = wrapper.on_mouse_down_out(cx.listener(move |v, _, _, cx| {
                v.send(
                    id,
                    "keydown",
                    EventDetail {
                        key: Some("Escape".into()),
                        ..Default::default()
                    },
                    cx,
                );
            }));
        }
        deferred(
            anchored()
                .position(position)
                .snap_to_window()
                .child(wrapper),
        )
        .priority(1)
        .into_any_element()
    }
    fn accessible(&self, mut element: Stateful<Div>, node: &NodeView) -> Stateful<Div> {
        if let Some(ax) = &node.ax {
            if let Some(role) = ax.role {
                element = element.role(role);
            }
            if let Some(label) = &ax.label {
                element = element.aria_label(label.clone());
            }
            if let Some(value) = &ax.value {
                element = element.aria_value(value.clone());
            }
            if ax.disabled {
                element = element.aria_disabled(true);
            }
            if let Some(selected) = ax.selected {
                element = element.aria_selected(selected);
            }
            if let Some(expanded) = ax.expanded {
                element = element.aria_expanded(expanded);
            }
        }
        if let Some(semantic) = &node.semantic {
            element =
                element.accessibility_id(format!("v4.{}.{}", self.publication.panel, semantic));
        }
        if let Some(count) = node.row_count {
            element = element.aria_row_count(count);
        }
        element
    }
    fn interactive(
        &self,
        mut element: Stateful<Div>,
        node: &NodeView,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = node.id;
        let events = node.events;
        let disabled = node.disabled;
        let clickable = events.click
            || matches!(
                node.component,
                Some(
                    Component::PopUpButton
                        | Component::Checkbox
                        | Component::Toggle
                        | Component::ColourWell
                )
            );
        if !disabled && (clickable || events.double_click) {
            let hover = rgb(self.table.tokens.colors.hover);
            if clickable {
                element = element.hover_style(move |s| s.bg(hover));
            }
            element = element.on_click(cx.listener(move |v, event: &ClickEvent, window, cx| {
                cx.stop_propagation();
                let position = event.position();
                let detail = v.detail(position, Some(0), 0, event.click_count(), event.modifiers());
                if clickable {
                    v.activate(id, detail.clone(), window, cx);
                }
                if events.double_click && event.click_count() == 2 {
                    v.send(id, "doubleclick", detail, cx);
                }
            }));
        }
        if disabled {
            return element;
        }
        // Finder drops reach the Project panel's own targets; the window's
        // drop handler places anything landing elsewhere at the playhead.
        if events.drop && self.publication.panel == "assets" {
            let target = rgb(self.table.tokens.colors.selected_fill);
            element = element
                .on_drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
                .on_drop(cx.listener(move |v, paths: &ExternalPaths, window, cx| {
                    let detail = file_drop(paths.paths(), window.modifiers().alt);
                    v.send(id, "drop", detail, cx);
                }));
        }
        if node.number.is_some() {
            element = element.cursor_col_resize().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, event: &MouseDownEvent, window, cx| {
                    v.scrub_press(id, event, window);
                    cx.stop_propagation();
                }),
            );
        }
        if let Some((layer, path)) = node.timeline_scrub.clone() {
            let outline = self.natives.outline.clone();
            element = element.cursor_col_resize().on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, _, cx| {
                    if outline.scrub_property(&layer, &path, event.position, cx) {
                        cx.stop_propagation();
                    }
                },
            );
        }
        if events.pointer() {
            element = element.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                    v.press(id, event, cx);
                    cx.stop_propagation();
                }),
            );
        }
        // Panels open their menus from a secondary press, the way they did in Electron.
        if events.pointer_down {
            element = element.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let detail = v.detail(event.position, Some(2), 2, 1, event.modifiers);
                    v.pointer(id, "pointerdown", detail, cx);
                }),
            );
        }
        if events.context_menu {
            element = element.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let detail = v.detail(event.position, Some(2), 4, 1, event.modifiers);
                    v.send(id, "contextmenu", detail, cx);
                }),
            );
        }
        if events.hover {
            element = element.on_hover(cx.listener(move |v, hovered: &bool, _, cx| {
                let event = if *hovered {
                    "pointerenter"
                } else {
                    "pointerleave"
                };
                v.pointer(id, event, EventDetail::default(), cx);
            }));
        }
        if events.wheel {
            element = element.on_scroll_wheel(cx.listener(
                move |v, event: &ScrollWheelEvent, window, cx| {
                    cx.stop_propagation();
                    let delta = event.delta.pixel_delta(window.line_height());
                    let detail = EventDetail {
                        delta_x: Some(-f32::from(delta.x) as f64),
                        delta_y: Some(-f32::from(delta.y) as f64),
                        ..v.detail(event.position, None, 0, 0, event.modifiers)
                    };
                    v.pointer(id, "wheel", detail, cx);
                },
            ));
            // Browser pinch convention: a control-modified wheel, so one handler zooms.
            element = element.on_pinch(cx.listener(move |v, event: &PinchEvent, _, cx| {
                cx.stop_propagation();
                let detail = EventDetail {
                    delta_y: Some(-f64::from(event.delta) * 100.),
                    ctrl_key: Some(true),
                    ..v.detail(event.position, None, 0, 0, event.modifiers)
                };
                v.pointer(id, "wheel", detail, cx);
            }));
        }
        if node.parent == Some(0) && self.prepared.has_keydown {
            element =
                element.on_key_down(cx.listener(move |v, event: &KeyDownEvent, window, cx| {
                    v.key_down(id, event, window, cx);
                }));
        }
        if let Some(title) = node.title.clone() {
            let tokens = self.table.tokens.clone();
            element = element.tooltip(move |_, cx| {
                let (title, tokens) = (title.clone(), tokens.clone());
                cx.new(|_| Tooltip { title, tokens }).into()
            });
        }
        element
    }
    fn key_down(
        &mut self,
        root: Id,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let id = self
            .focus
            .iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(id, _)| *id)
            .unwrap_or(root);
        if pm_surface::interaction::Interaction::default()
            .plan(&self.publication.surface, id, None, "keydown", Some(key))
            .is_ok_and(|p| p.prevent_default)
        {
            window.prevent_default();
            cx.stop_propagation();
        }
        self.send(
            id,
            "keydown",
            EventDetail {
                key: Some(key.into()),
                repeat: Some(event.is_held),
                ..modified(event.keystroke.modifiers)
            },
            cx,
        );
    }
    /// Kit furniture that is not expressible as a generic style.
    fn decorate(
        &self,
        mut element: Stateful<Div>,
        node: &NodeView,
        window: &Window,
    ) -> Stateful<Div> {
        let t = &self.table.tokens;
        if self.prepared.anchors.contains(&node.id) {
            let (id, anchors) = (node.id, self.anchors.clone());
            element = element.child(
                canvas(
                    move |bounds, _, _| {
                        anchors.borrow_mut().insert(id, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        }
        match &node.kit {
            Kit::Check(on) => {
                element = element.child(pm_shell::symbol::symbol(
                    if *on {
                        "checkmark.square.fill"
                    } else {
                        "square"
                    },
                    t.metrics.symbol_panel,
                    pm_shell::symbol::Weight::Regular,
                    t.colors.secondary,
                    window,
                ));
            }
            Kit::Swatch { color, keyline } => {
                element = element
                    .w(px(t.metrics.field))
                    .border(px(t.metrics.border))
                    .border_color(rgb(*keyline));
                if let Some(color) = color {
                    element = element.bg(rgb(*color));
                }
            }
            Kit::Progress(value) => {
                let track = div()
                    .h(px(t.metrics.inner_radius))
                    .w_full()
                    .rounded(px(t.metrics.inner_radius / 2.))
                    .overflow_hidden()
                    .bg(rgb(t.colors.field));
                let track = match value {
                    Some(value) => {
                        track.child(div().h_full().w(relative(*value)).bg(rgb(t.colors.accent)))
                    }
                    None => track.child(
                        div().size_full().bg(rgb(t.colors.muted)).with_animation(
                            "progress",
                            Animation::new(std::time::Duration::from_secs(1))
                                .repeat()
                                .with_easing(pulsating_between(0.3, 1.)),
                            |this, delta| this.opacity(delta),
                        ),
                    ),
                };
                element = element.child(track);
            }
            Kit::Gizmo {
                position,
                size,
                fill,
                stroke,
                keyline,
                round,
            } => {
                let half = size / 2.;
                element = element
                    .absolute()
                    .left(px(position.0 - half))
                    .top(px(position.1 - half))
                    .size(px(*size))
                    .bg(rgb(*fill))
                    .border(px(*keyline))
                    .border_color(rgb(*stroke));
                if *round {
                    element = element.rounded_full();
                }
            }
            Kit::Banner { symbol } => {
                if let Some((name, size, color)) = symbol {
                    element = element.child(pm_shell::symbol::symbol(
                        name,
                        *size,
                        pm_shell::symbol::Weight::Regular,
                        *color,
                        window,
                    ));
                }
            }
            Kit::Disclosure(open) => {
                element = element.child(pm_shell::symbol::symbol(
                    if *open {
                        "chevron.down"
                    } else {
                        "chevron.right"
                    },
                    t.metrics.symbol_inline,
                    pm_shell::symbol::Weight::Regular,
                    t.colors.secondary,
                    window,
                ));
            }
            Kit::Secure | Kit::Anchored { .. } | Kit::None => {}
        }
        if let Some(symbol) = &node.symbol {
            let ink = if node.disabled {
                t.colors.muted
            } else {
                t.colors.secondary
            };
            element = element.child(pm_shell::symbol::symbol(
                symbol,
                node.symbol_size,
                pm_shell::symbol::Weight::Regular,
                ink,
                window,
            ));
        }
        if let Some(text) = &node.text {
            element = element.child(text.clone());
        }
        if let Some(number) = node.number {
            let shown = self
                .scrub_shown
                .filter(|(id, _)| *id == node.id)
                .map_or(number.value, |(_, value)| value);
            element = element.child(number.text(shown));
        }
        element
    }
    fn field_element(
        &self,
        node: &NodeView,
        field: &Entity<TextField>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = node.id;
        // A field grows along a row (value wells); in a column it spans the
        // width at its own height instead of stretching downwards.
        let in_row = node
            .parent
            .and_then(|p| self.prepared.node(p))
            .is_some_and(|p| p.style.flex_direction == Some(FlexDirection::Row));
        let mut wrapper = div()
            .id(("field", id))
            .when(in_row, |d| d.flex_1())
            .when(!in_row, |d| d.flex_none().w_full())
            .min_w(px(0.))
            .child(field.clone());
        if let Some(ax) = &node.ax {
            if let Some(role) = ax.role {
                wrapper = wrapper.role(role);
            }
            wrapper = wrapper.aria_label(ax.label.clone().unwrap_or_default());
            if let Some(value) = &ax.value {
                wrapper = wrapper.aria_value(value.clone());
            }
        }
        if let Some(focus) = self.focus.get(&id) {
            wrapper = wrapper.track_focus(focus).tab_index(0);
        }
        if node.number.is_some() {
            wrapper = wrapper
                .cursor_col_resize()
                .capture_any_mouse_down(cx.listener(
                    move |v, event: &MouseDownEvent, window, cx| {
                        if event.button == MouseButton::Left {
                            v.scrub_press(id, event, window);
                            cx.stop_propagation();
                        }
                    },
                ));
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
        if let Some(number) = node.number {
            wrapper = wrapper
                .aria_numeric_value(number.value)
                .aria_numeric_value_step(number.step);
            for (action, direction) in [
                (AccessibleAction::Increment, 1.),
                (AccessibleAction::Decrement, -1.),
            ] {
                let weak = cx.entity().downgrade();
                wrapper = wrapper.on_a11y_action(action, move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| {
                        if let Some(n) = v.prepared.node(id).and_then(|n| n.number) {
                            let next = (n.value + n.step * direction).clamp(n.min, n.max);
                            v.change(id, json!(next), cx);
                        }
                    });
                });
            }
        }
        let field = field.clone();
        wrapper = wrapper.on_a11y_action(AccessibleAction::Focus, move |_, _, cx| {
            field.update(cx, |f, cx| f.begin_editing(cx));
        });
        wrapper.into_any_element()
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
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let t = &self.table.tokens;
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .text_color(rgb(t.colors.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .child(self.element(0, window, cx, self.prepared.header))
            .child(Self::tracking(cx, self.origin.clone()))
    }
}
struct Tooltip {
    title: SharedString,
    tokens: pm_design::Tokens,
}
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = &self.tokens;
        div()
            .px(px(t.metrics.gap))
            .py(px(t.metrics.inner_radius))
            .bg(rgb(t.colors.floating))
            .rounded(px(t.metrics.inner_radius))
            .text_size(px(t.typography.minimum))
            .text_color(rgb(t.colors.text))
            .child(self.title.clone())
    }
}
fn control_kind(panel: &str, node: &pm_surface::Node) -> Option<crate::v4_control::Kind> {
    use crate::v4_control::Kind;
    match Component::of(node) {
        Some(Component::PopUpButton) => Some(Kind::Popup),
        Some(Component::Checkbox | Component::Toggle) => Some(Kind::Checkbox),
        Some(Component::Switch) => Some(Kind::Switch),
        Some(Component::PushButton) => Some(Kind::Push),
        Some(Component::ToolbarItem)
            if panel == "export"
                && node
                    .props
                    .get("metadata")
                    .and_then(|m| m.get("export"))
                    .and_then(|m| m.get("nativeControl"))
                    .and_then(Value::as_str)
                    == Some("NSButton") =>
        {
            Some(Kind::Push)
        }
        _ => None,
    }
}
fn modified(modifiers: Modifiers) -> EventDetail {
    EventDetail {
        shift_key: Some(modifiers.shift),
        alt_key: Some(modifiers.alt),
        meta_key: Some(modifiers.platform),
        ctrl_key: Some(modifiers.control),
        ..Default::default()
    }
}
fn button_code(button: MouseButton) -> u64 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        _ => 3,
    }
}
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub struct HeaderView {
    view: Entity<SurfaceView>,
    _observe: Subscription,
}
impl HeaderView {
    pub fn new(view: Entity<SurfaceView>, cx: &mut Context<Self>) -> Self {
        let _observe = cx.observe(&view, |_, _, cx| cx.notify());
        Self { view, _observe }
    }
}
impl Render for HeaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.view.update(cx, |v, cx| match v.prepared.header {
            Some(id) => v.element(id, window, cx, None),
            None => div().into_any_element(),
        })
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
