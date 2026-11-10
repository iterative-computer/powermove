//! Generated panels (S102): a workspace's `custom` sections, drawn natively.
//! A control binds as v1's `sourceBinding` (`core/generated-bindings.ts`):
//! the composition (its fields and background fill), a layer's content,
//! properties, transform or layer fields (`$selection` is the first selected
//! layer), or a local `stateKey` kept in the workspace record. Numbers, text
//! and colours are typed in a small sheet, a select opens its options as a
//! menu, toggles and buttons act on click. Each document edit is one Undo
//! step named for its control; a `stateKey` edit is tool state, no Undo step
//! (v1's `local` edit mode). A sheet or menu edits the layer its control
//! targeted when it opened. A binding V2 can't write yet shows disabled with why.
use crate::session::Controller;
use gpui::{prelude::*, *};
use objc2_app_kit::NSAccessibility;
use pm_kernel::{Call, Caller, HistoryCall, Request, WriteBody};
use pm_shell::workspace::{self, Write};
use serde_json::{Value, json};

/// The section `id` of the window's workspace, and that record.
fn section(v: &Controller, id: &str) -> Option<(Value, usize, Value)> {
    let store = v.workspace_store();
    let records = store
        .project
        .iter()
        .filter(|p| workspace::usable(p))
        .chain(store.offered());
    for record in records {
        if let Some((i, s)) = record["custom"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .find(|(_, s)| s["id"] == id)
        {
            return Some((record.clone(), i, s.clone()));
        }
    }
    None
}

pub fn panel(
    v: &mut Controller,
    id: &str,
    rect: pm_shell::layout::Rect,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    let (_, _, def) = section(v, id)?;
    Some(render(v, id, &def, rect, cx))
}

/// What a control reads and writes.
enum Bind {
    Composition(String),
    Fill(String),
    Layer { target: String, path: String },
    Local(String),
    None(&'static str),
}
fn bind(control: &Value) -> Bind {
    let target = control["target"]
        .as_str()
        .or_else(|| control["binding"]["target"].as_str())
        .filter(|s| !s.is_empty());
    let path = control["path"]
        .as_str()
        .or_else(|| control["binding"]["path"].as_str())
        .filter(|s| !s.is_empty());
    match (target, path) {
        (Some("$composition" | "composition"), Some(path)) => {
            match path.strip_prefix("composition.background.") {
                Some(key) => Bind::Fill(key.into()),
                None => Bind::Composition(path.trim_start_matches("composition.").into()),
            }
        }
        (Some(target), Some(path)) => Bind::Layer {
            target: target.into(),
            path: path.into(),
        },
        _ => match control["stateKey"].as_str().filter(|s| !s.is_empty()) {
            Some(key) => Bind::Local(key.into()),
            None => Bind::None("Scene parameters can’t be edited here yet."),
        },
    }
}

fn json_of(node: Option<&pm_model::doc::Node>) -> Value {
    node.map(|n| {
        serde_json::from_str(&pm_model::json::to_canonical_string(&n.to_value()))
            .unwrap_or_default()
    })
    .unwrap_or_default()
}
/// The open composition's node (`comps/<compId>`, else the root).
fn root(v: &Controller) -> Option<&pm_model::doc::Node> {
    root_of(v.published.as_ref()?)
}
fn root_of(p: &pm_kernel::Published) -> Option<&pm_model::doc::Node> {
    let root = p.snapshot.root()?;
    let comp = root.get("compId").map(|n| json_of(Some(n)));
    comp.as_ref()
        .and_then(Value::as_str)
        .and_then(|id| root.get("comps")?.get(id))
        .or(Some(root))
}
/// The composition's background fill as v1's `normalizeFill` reads it.
fn fill(v: &Controller) -> Value {
    let root = root(v);
    let bg = json_of(root.and_then(|r| r.get("bg")));
    let bg = bg.as_str().unwrap_or("#000000");
    let raw = json_of(root.and_then(|r| r.get("backgroundFill")));
    let mut f = if raw.is_object() {
        raw
    } else {
        json!({"type": "solid", "stops": [{"color": bg, "position": 0}]})
    };
    if f["stops"].as_array().is_none_or(|s| s.is_empty()) {
        f["stops"] = json!([{"color": bg, "position": 0}]);
    }
    if f["type"].as_str().is_none() {
        f["type"] = "solid".into();
    }
    f
}
/// The layer a control targets: `$selection` is the first selected one, any
/// other target an id, else a name.
fn layer(v: &Controller, target: &str) -> Option<(String, Value)> {
    find_layer(v.published.as_ref()?, target, None)
}
/// The layer a control targets in `p`. `bound` is the layer a sheet or menu
/// bound when it opened: that layer, whatever is selected now. Otherwise
/// `$selection` is the first selected layer, any other target an id, else
/// a name.
fn find_layer(
    p: &pm_kernel::Published,
    target: &str,
    bound: Option<&str>,
) -> Option<(String, Value)> {
    let id = match (bound, target) {
        (Some(id), _) => Some(id.to_owned()),
        (None, "$selection" | "selection") => Some(p.selection.primary()?.to_owned()),
        _ => None,
    };
    let layers = root_of(p)?.get("layers")?;
    let mut i = 0;
    while let Some(node) = layers.at(i) {
        i += 1;
        let l = json_of(Some(node));
        let matches = match &id {
            Some(id) => l["id"] == id.as_str(),
            None => l["id"] == target || l["name"] == target,
        };
        if matches {
            return Some((l["id"].as_str()?.to_owned(), l));
        }
    }
    None
}
const LAYER_KEYS: [(&str, &str); 4] = [
    ("visible", "on"),
    ("locked", "lock"),
    ("duration", "dur"),
    ("motionBlur", "mblur"),
];
/// The control's current value, else its `def`; `Err` says why it can't be edited.
fn value(v: &Controller, def: &Value, control: &Value) -> Result<Value, &'static str> {
    let fallback = control["def"].clone();
    let or = |x: &Value| {
        if x.is_null() {
            fallback.clone()
        } else {
            x.clone()
        }
    };
    match bind(control) {
        Bind::Fill(key) => {
            let f = fill(v);
            let stops = &f["stops"];
            Ok(match key.as_str() {
                "type" => f["type"].clone(),
                "startColor" => or(&stops[0]["color"]),
                "endColor" => or(if stops[1]["color"].is_null() {
                    &stops[0]["color"]
                } else {
                    &stops[1]["color"]
                }),
                "angle" => or(&f["angle"]),
                "midpoint" => {
                    if stops[1]["position"].is_null() {
                        json!(100)
                    } else {
                        stops[1]["position"].clone()
                    }
                }
                _ => fallback,
            })
        }
        Bind::Composition(key) => {
            let key = match key.as_str() {
                "width" => "w",
                "height" => "h",
                "duration" => "dur",
                "background" => "bg",
                other => other,
            };
            Ok(or(&json_of(root(v).and_then(|r| r.get(key)))))
        }
        Bind::Layer { target, path } => {
            let Some((_, l)) = layer(v, &target) else {
                return Err("This control’s layer isn’t in the composition.");
            };
            if let Some(key) = path.strip_prefix("content.") {
                return Ok(or(&l["d"][key]));
            }
            if let Some(key) = path.strip_prefix("layer.") {
                let key = LAYER_KEYS
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map_or(key, |(_, s)| s);
                return Ok(or(&l[key]));
            }
            match path
                .strip_prefix("properties.")
                .or_else(|| path.strip_prefix("transform."))
            {
                Some(channel) => Ok(or(&l["p"][channel]["v"])),
                None => Err("This control’s property isn’t available."),
            }
        }
        Bind::Local(key) => Ok(or(&def["state"][key])),
        Bind::None(why) => Err(why),
    }
}

/// v1's background-field write: `type` switches the fill; any stop field
/// makes a real two-stop gradient first.
fn set_fill(mut f: Value, key: &str, next: Value) -> Value {
    if key == "type" {
        let t = next.as_str().unwrap_or("solid");
        f["type"] = if ["solid", "linear", "radial", "none"].contains(&t) {
            t
        } else {
            "solid"
        }
        .into();
        return f;
    }
    if !matches!(f["type"].as_str(), Some("linear" | "radial")) {
        f["type"] = "linear".into();
    }
    if let Some(stops) = f["stops"].as_array_mut()
        && stops.len() < 2
    {
        let first = stops.first().map_or(Value::Null, |s| s["color"].clone());
        stops.push(json!({"color": first, "position": 100}));
    }
    match key {
        "startColor" => f["stops"][0]["color"] = next,
        "endColor" => f["stops"][1]["color"] = next,
        "angle" => f["angle"] = next,
        "midpoint" => f["stops"][1]["position"] = next,
        _ => {}
    }
    f
}

/// The edit of layer `id`'s `path` (`content.*`, `layer.*`, `properties.*`,
/// `transform.*`) to `next`.
fn layer_command(
    id: String,
    path: &str,
    next: &Value,
    time: f64,
) -> Result<Option<pm_kernel::Command>, &'static str> {
    use pm_kernel::commands::{layers::LayerPatch, properties::SetProperty};
    let model = |v: &Value| pm_model::json::parse(&v.to_string()).ok();
    if let Some(key) = path.strip_prefix("content.") {
        let mut patch = pm_model::Map::new();
        if let Some(v) = model(next) {
            patch.insert(key, v);
        }
        return Ok(Some(
            pm_kernel::ContentCommand::Set {
                layer_id: id,
                patch,
                time,
            }
            .into(),
        ));
    }
    if let Some(key) = path.strip_prefix("layer.") {
        let mut patch = LayerPatch::default();
        match key {
            "visible" => patch.visible = next.as_bool(),
            "locked" => patch.locked = next.as_bool(),
            "name" => patch.name = next.as_str().map(str::to_owned),
            "motionBlur" => patch.motion_blur = next.as_bool(),
            "duration" => patch.duration = next.as_f64(),
            "from" => patch.from = next.as_f64(),
            _ => return Err("This control’s property isn’t available."),
        }
        return Ok(Some(
            pm_kernel::LayerCommand::Set {
                layer_id: id,
                patch,
            }
            .into(),
        ));
    }
    Ok(path
        .strip_prefix("properties.")
        .or_else(|| path.strip_prefix("transform."))
        .and_then(|channel| {
            let value = model(next)?;
            let mut set = SetProperty::new(id, channel, value, time);
            set.preserve_hand_edits = false;
            Some(pm_kernel::PropertyCommand::Set(set).into())
        }))
}
/// A `stateKey` edit: local tool state in the workspace record (v1's
/// `section.state`). As v1's `local` edit mode, it never touches the
/// document, so it is no Undo step.
fn local_write(
    store: &workspace::Store,
    record: &Value,
    si: usize,
    key: &str,
    next: Value,
) -> Write {
    let mut after = record.clone();
    after["custom"][si]["state"][key] = next;
    let own = store.project.as_ref().and_then(|p| workspace::record_id(p))
        == workspace::record_id(record);
    if own {
        Write::Active {
            before: store.project.clone().unwrap_or_default(),
            after,
        }
    } else {
        Write::Upsert(after)
    }
}
/// The layer a sheet or menu for `control` edits, bound as it opens.
fn bound_layer(v: &Controller, control: &Value) -> Option<String> {
    match bind(control) {
        Bind::Layer { target, .. } => layer(v, &target).map(|(id, _)| id),
        _ => None,
    }
}

impl Controller {
    /// Writes `next` through control `index` of generated panel `panel`.
    pub fn generated_set(
        &mut self,
        panel: &str,
        index: usize,
        next: Value,
        cx: &mut Context<Self>,
    ) {
        self.generated_set_on(panel, index, next, None, cx);
    }
    /// `generated_set` for a sheet or menu: a layer control writes `bound`,
    /// the layer it targeted when it opened.
    fn generated_set_on(
        &mut self,
        panel: &str,
        index: usize,
        next: Value,
        bound: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some((record, si, def)) = section(self, panel) else {
            return;
        };
        let Some(control) = def["controls"].get(index).cloned() else {
            return;
        };
        let label = control["label"].as_str().unwrap_or("Control").to_owned();
        let next = match pm_services::surface_backing::generated::edit_value(&control, next) {
            Ok(n) => n,
            Err(e) => return self.note(e.diagnostic().to_string(), cx),
        };
        let model = |v: &Value| pm_model::json::parse(&v.to_string()).ok();
        let time = self.time();
        use pm_kernel::commands::comps::CompPatch;
        let command: Option<pm_kernel::Command> = match bind(&control) {
            Bind::Fill(key) => {
                let f = set_fill(fill(self), &key, next);
                model(&f).map(|fill| {
                    pm_kernel::CompCommand::Set {
                        patch: Box::new(CompPatch {
                            background_fill: Some(fill),
                            ..Default::default()
                        }),
                    }
                    .into()
                })
            }
            Bind::Composition(key) => {
                let mut patch = CompPatch::default();
                match key.as_str() {
                    "name" => patch.name = next.as_str().map(str::to_owned),
                    "width" => patch.width = next.as_f64(),
                    "height" => patch.height = next.as_f64(),
                    "fps" => patch.fps = next.as_f64(),
                    "duration" => patch.duration = next.as_f64(),
                    "shutter" => patch.shutter = next.as_f64(),
                    "background" => patch.background = next.as_str().map(str::to_owned),
                    "backgroundFill" => patch.background_fill = model(&next),
                    _ => return self.note("This control’s property isn’t available.".into(), cx),
                }
                Some(
                    pm_kernel::CompCommand::Set {
                        patch: Box::new(patch),
                    }
                    .into(),
                )
            }
            Bind::Layer { target, path } => {
                let found = self
                    .published
                    .as_ref()
                    .and_then(|p| find_layer(p, &target, bound.as_deref()));
                let Some((id, _)) = found else {
                    return self.note("This control’s layer isn’t in the composition.".into(), cx);
                };
                match layer_command(id, &path, &next, time) {
                    Ok(command) => command,
                    Err(why) => return self.note(why.into(), cx),
                }
            }
            Bind::Local(key) => {
                let write = local_write(&self.workspace_store(), &record, si, &key, next);
                self.write_workspace(write);
                return;
            }
            Bind::None(why) => return self.note(why.into(), cx),
        };
        let (Some(command), Some(session)) = (command, &self.session) else {
            return;
        };
        let call = Call::Transact {
            label: Some(label),
            expected_revision: None,
            body: WriteBody::Commands(vec![command]),
        };
        let pending = self
            .kernel
            .call(&session.project, Request::new(Caller::native(), call));
        cx.spawn(async move |this, cx| {
            if let Some(Ok(ack)) = pending.await
                && let pm_kernel::Reply::Throw(e) = ack.reply
            {
                let _ = this.update(cx, |v, cx| v.note(e.to_string(), cx));
            }
        })
        .detach();
    }
    /// A generated button: undo, reset its local state, run a command, or
    /// say what the agent can do.
    pub fn generated_press(&mut self, panel: &str, index: usize, cx: &mut Context<Self>) {
        let Some((record, si, def)) = section(self, panel) else {
            return;
        };
        let control = &def["controls"][index];
        use pm_services::surface_backing::generated::{ButtonEffect, button_effect};
        match button_effect(control, &def["state"]) {
            Ok(ButtonEffect::Undo) => self.call(Call::History(HistoryCall::Undo), cx),
            Ok(ButtonEffect::Reset) => {
                let mut after = record.clone();
                for c in def["controls"].as_array().into_iter().flatten() {
                    if let Some(key) = c["stateKey"].as_str() {
                        after["custom"][si]["state"][key] = c["def"].clone();
                    }
                }
                self.write_workspace(Write::Upsert(after));
            }
            Ok(ButtonEffect::Command(cmd)) => self.host.borrow().send_command(&cmd, vec![]),
            Ok(ButtonEffect::AgentPrompt(_)) => {
                self.note("Ask the Powermove agent to change this section".into(), cx)
            }
            Ok(ButtonEffect::None) => {}
            Ok(ButtonEffect::Tool { .. } | ButtonEffect::Commands(_)) | Err(_) => {
                self.note(
                    format!(
                        "{} isn’t available in this version yet.",
                        control["label"].as_str().unwrap_or("This tool")
                    ),
                    cx,
                );
            }
        }
    }
    /// Types a number, text or colour for control `index` in a sheet.
    fn generated_edit(
        &mut self,
        panel: String,
        index: usize,
        current: String,
        cx: &mut Context<Self>,
    ) {
        let Some((_, _, def)) = section(self, &panel) else {
            return;
        };
        let control = def["controls"][index].clone();
        // The layer it edits is the one targeted now, not when it's submitted.
        let bound = bound_layer(self, &control);
        let label = control["label"].as_str().unwrap_or("Value").to_owned();
        let numeric = matches!(control["type"].as_str(), Some("slider" | "number"));
        let range = match (control["min"].as_f64(), control["max"].as_f64()) {
            (Some(a), Some(b)) => format!("From {a} to {b}."),
            _ if control["type"] == "color" => "A colour as #RRGGBB.".into(),
            _ => String::new(),
        };
        use crate::alert::{Alert, Button};
        let alert = Alert::new(
            label.clone(),
            range,
            vec![
                Button::new("Set", true).default(),
                Button::new("Cancel", false).cancel(),
            ],
        );
        let mtm = objc2::MainThreadMarker::new().expect("main thread");
        let field = objc2_app_kit::NSTextField::textFieldWithString(
            &objc2_foundation::NSString::from_str(&current),
            mtm,
        );
        field.setFrame(objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(0., 0.),
            objc2_foundation::NSSize::new(220., 24.),
        ));
        field.setAccessibilityLabel(Some(&objc2_foundation::NSString::from_str(&label)));
        let read = field.clone();
        let answer = alert.sheet_with(&self.host.borrow().window.clone(), |a| {
            a.setAccessoryView(Some(&field));
            a.window().setInitialFirstResponder(Some(&field));
        });
        cx.spawn(async move |this, cx| {
            if answer.recv().await != Ok(Some(true)) {
                return;
            }
            // The field editor may still hold the typing: commit it first.
            read.validateEditing();
            let text = read.stringValue().to_string();
            let next = if numeric {
                match text.trim().parse::<f64>() {
                    Ok(n) if n.is_finite() => json!(n),
                    _ => {
                        let _ =
                            this.update(cx, |v, cx| v.note(format!("{label} needs a number."), cx));
                        return;
                    }
                }
            } else {
                json!(text)
            };
            let _ = this.update(cx, |v, cx| {
                v.generated_set_on(&panel, index, next, bound, cx)
            });
        })
        .detach();
    }
}

fn shown(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(0.);
            if f.fract() == 0. {
                format!("{}", f as i64)
            } else {
                format!("{:.2}", f).trim_end_matches('0').to_owned()
            }
        }
        Value::Bool(b) => if *b { "On" } else { "Off" }.into(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
fn option_label(o: &Value) -> (Value, String) {
    match o.get("v") {
        Some(v) => (
            v.clone(),
            o["label"].as_str().map_or_else(|| shown(v), str::to_owned),
        ),
        None => (o.clone(), shown(o)),
    }
}
fn readout(v: &Controller, source: &str) -> String {
    let Some(p) = &v.published else {
        return "No project".into();
    };
    let (layers, keys) = (p.selection.layers().len(), p.selection.keys().len());
    match source {
        "selection.count" => layers.to_string(),
        "keyframes.count" => keys.to_string(),
        "keyframes.summary" if keys == 0 => "No keyframes selected".into(),
        "keyframes.summary" => format!("{keys} keyframes selected"),
        _ => format!("{layers} selected"),
    }
}

fn render(
    v: &mut Controller,
    id: &str,
    def: &Value,
    rect: pm_shell::layout::Rect,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let t = v.host.borrow().tokens.clone();
    let (c, m, ty) = (&t.colors, &t.metrics, &t.typography);
    let Some(controls) = def["controls"].as_array() else {
        let title = def["title"].as_str().unwrap_or("Controls");
        return pm_shell::view::empty_state(
            &t,
            &format!("{title} Can’t Be Shown"),
            "Its definition isn’t valid.",
        )
        .size_full()
        .into_any_element();
    };
    let primaries = controls
        .iter()
        .filter(|c| c["type"] == "button" && c["primary"] == true)
        .count();
    let mut body = div()
        .id(SharedString::from(format!("generated-{id}")))
        .flex()
        .flex_col()
        .w_full()
        .h(px((rect.h - m.header).max(0.)))
        .p(px(m.gap))
        .gap(px(m.gap / 2.))
        .overflow_y_scroll()
        .text_size(px(ty.body));
    if let Some(note) = def["note"].as_str().filter(|n| !n.is_empty()) {
        body = body.child(
            div()
                .text_size(px(ty.minimum))
                .text_color(rgb(c.muted))
                .child(note.to_owned()),
        );
    }
    let field = |eid: String, label: String, text: String, enabled: bool| {
        div()
            .id(SharedString::from(eid))
            .role(Role::Button)
            .aria_label(label)
            .flex_none()
            .min_w(px(m.control * 2.))
            .h(px(m.control))
            .px(px(m.inset))
            .flex()
            .items_center()
            .rounded(px(m.control_radius))
            .bg(rgb(c.field))
            .when(enabled, |d| d.hover_style(|s| s.bg(rgb(c.hover))))
            .text_color(rgb(if enabled { c.text } else { c.muted }))
            .child(text)
    };
    for (index, control) in controls.iter().enumerate() {
        let kind = control["type"].as_str().unwrap_or("");
        let label = control["label"].as_str().unwrap_or("").to_owned();
        let panel = id.to_owned();
        let eid = format!("generated-{id}-{index}");
        let row = div()
            .flex()
            .items_center()
            .gap(px(m.gap))
            .min_h(px(m.row))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(c.secondary))
                    .child(label.clone()),
            );
        let row = match kind {
            "note" => {
                body = body.child(
                    div()
                        .text_size(px(ty.minimum))
                        .text_color(rgb(c.muted))
                        .child(control["text"].as_str().unwrap_or(&label).to_owned()),
                );
                continue;
            }
            "readout" => row.child(
                div()
                    .text_color(rgb(c.secondary))
                    .child(readout(v, control["source"].as_str().unwrap_or(""))),
            ),
            "button" => {
                let accent = control["primary"] == true && primaries == 1;
                body = body.child(
                    div()
                        .id(SharedString::from(eid))
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .h(px(m.control))
                        .px(px(m.inset))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(m.control_radius))
                        .bg(rgb(if accent { c.accent } else { c.field }))
                        .hover_style(|s| s.opacity(0.85))
                        .text_color(rgb(if accent { c.accent_text } else { c.text }))
                        .child(label)
                        .on_click(
                            cx.listener(move |v, _, _, cx| v.generated_press(&panel, index, cx)),
                        ),
                );
                continue;
            }
            _ => match value(v, def, control) {
                Err(why) => row.child(
                    field(eid, label.clone(), "—".into(), false)
                        .tooltip(pm_shell::view::tooltip(&t, why.to_owned())),
                ),
                Ok(current) => match kind {
                    "toggle" => {
                        let on = current == true;
                        row.child(
                            field(
                                eid,
                                label.clone(),
                                if on { "On" } else { "Off" }.into(),
                                true,
                            )
                            .on_click(cx.listener(
                                move |v, _, _, cx| v.generated_set(&panel, index, json!(!on), cx),
                            )),
                        )
                    }
                    "select" => {
                        let options: Vec<(Value, String)> = control["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(option_label)
                            .collect();
                        let text = options
                            .iter()
                            .find(|(v, _)| *v == current)
                            .map_or_else(|| shown(&current), |(_, l)| l.clone());
                        row.child(field(eid, label.clone(), text, true).on_click(cx.listener(
                            move |v, e: &ClickEvent, _, _| {
                                v.generated_menu_layer =
                                    bound_layer(v, &control_at(v, &panel, index));
                                let entries: Vec<_> = options
                                    .iter()
                                    .enumerate()
                                    .map(|(o, (value, l))| {
                                        pm_shell::MenuEntry::item(
                                            l.clone(),
                                            format!("generatedPick:{o}:{index}:{panel}"),
                                        )
                                        .checked(*value == current)
                                    })
                                    .collect();
                                let at = e.position();
                                v.host.borrow().context_menu(
                                    &entries,
                                    f32::from(at.x),
                                    f32::from(at.y),
                                );
                            },
                        )))
                    }
                    "slider" | "number" | "text" | "color" => {
                        let text = shown(&current);
                        let mut f = field(eid, label.clone(), text.clone(), true);
                        if kind == "color"
                            && let Some(hex) = text
                                .strip_prefix('#')
                                .and_then(|h| u32::from_str_radix(h.get(..6)?, 16).ok())
                        {
                            f = f.child(
                                div()
                                    .ml(px(m.gap))
                                    .size(px(ty.reading))
                                    .rounded(px(m.inner_radius))
                                    .bg(rgb(hex)),
                            );
                        }
                        row.child(f.on_click(cx.listener(move |v, _, _, cx| {
                            v.generated_edit(panel.clone(), index, text.clone(), cx)
                        })))
                    }
                    _ => row.child(field(eid, label.clone(), shown(&current), false).tooltip(
                        pm_shell::view::tooltip(&t, "This control can’t be edited here yet."),
                    )),
                },
            },
        };
        body = body.child(row);
    }
    body.into_any_element()
}

/// Control `index` of generated panel `panel`.
fn control_at(v: &Controller, panel: &str, index: usize) -> Value {
    section(v, panel).map_or(Value::Null, |(_, _, def)| def["controls"][index].clone())
}
/// `generatedPick:<option>:<control>:<panel>` from a select's menu.
pub fn pick(v: &mut Controller, command: &str, cx: &mut Context<Controller>) -> bool {
    let Some(rest) = command.strip_prefix("generatedPick:") else {
        return false;
    };
    let mut parts = rest.splitn(3, ':');
    let (Some(o), Some(i), Some(panel)) = (parts.next(), parts.next(), parts.next()) else {
        return true;
    };
    let (Ok(o), Ok(i)) = (o.parse::<usize>(), i.parse::<usize>()) else {
        return true;
    };
    if let Some((_, _, def)) = section(v, panel)
        && let Some(option) = def["controls"][i]["options"].get(o)
    {
        let value = option_label(option).0;
        let bound = v.generated_menu_layer.take();
        v.generated_set_on(panel, i, value, bound, cx);
    }
    true
}

#[cfg(test)]
mod tests {
    // Not a glob: gpui's own `test` attribute would shadow the standard one.
    use super::{Bind, bind, find_layer, layer_command, local_write, set_fill, workspace};
    use pm_kernel::{
        Call, Caller, HistoryCall, KernelActor, KernelConfig, KernelHandle, OpenSource, Request,
        SelectionCall, WorkspaceCall, WriteBody,
    };
    use serde_json::{Value, json};

    const PROJECT: &str = "generated";
    fn open() -> (KernelActor, KernelHandle) {
        let actor = KernelActor::spawn(KernelConfig::default());
        let kernel = actor.handle();
        let project = pm_model::json::parse(&crate::v4_view_tests::project_json(2)).unwrap();
        kernel
            .open(PROJECT, OpenSource::Untitled(project))
            .wait()
            .unwrap()
            .unwrap();
        (actor, kernel)
    }
    fn call(kernel: &KernelHandle, call: impl Into<Call>) -> pm_kernel::Reply {
        kernel
            .call(PROJECT, Request::new(Caller::native(), call.into()))
            .wait()
            .unwrap()
            .unwrap()
            .reply
    }
    fn select(kernel: &KernelHandle, id: &str) {
        call(
            kernel,
            SelectionCall::Layers {
                ids: vec![id.into()],
                add: false,
            },
        );
    }
    fn opacity(kernel: &KernelHandle, id: &str) -> f64 {
        let p = kernel.published(PROJECT).unwrap();
        find_layer(&p, id, None).unwrap().1["p"]["opacity"]["v"]
            .as_f64()
            .unwrap()
    }
    fn set(kernel: &KernelHandle, id: String, value: f64) {
        let command = layer_command(id, "properties.opacity", &json!(value), 0.)
            .unwrap()
            .unwrap();
        call(
            kernel,
            Call::Transact {
                label: Some("Opacity".into()),
                expected_revision: None,
                body: WriteBody::Commands(vec![command]),
            },
        );
    }

    #[test]
    fn a_sheet_writes_the_layer_selected_when_it_opened() {
        let (_actor, kernel) = open();
        select(&kernel, "l0");
        // The sheet opens on l0 (`bound_layer`)...
        let p = kernel.published(PROJECT).unwrap();
        let bound = find_layer(&p, "$selection", None).map(|(id, _)| id);
        assert_eq!(bound.as_deref(), Some("l0"));
        // ...the person selects l1 while it's open, then presses Set.
        select(&kernel, "l1");
        let p = kernel.published(PROJECT).unwrap();
        let (id, _) = find_layer(&p, "$selection", bound.as_deref()).unwrap();
        set(&kernel, id, 40.);
        assert_eq!(
            (opacity(&kernel, "l0"), opacity(&kernel, "l1")),
            (40., 100.)
        );
        // A layer deleted meanwhile is not silently swapped for another.
        assert!(find_layer(&p, "$selection", Some("gone")).is_none());
    }

    #[test]
    fn local_state_is_tool_state_no_undo_step_as_v1() {
        let (_actor, kernel) = open();
        let mut record = workspace::preset("gradient").unwrap();
        record["custom"][0]["state"] = json!({});
        let model = |v: &Value| pm_model::json::parse(&v.to_string()).unwrap();
        let expected = kernel.published(PROJECT).unwrap().workspace.clone();
        call(
            &kernel,
            WorkspaceCall::Replace {
                expected,
                next: model(&record),
            },
        );
        set(&kernel, "l0".into(), 40.);
        let store = workspace::Store {
            records: vec![],
            current: None,
            default: None,
            project: Some(record.clone()),
            project_id: Some(PROJECT.into()),
        };
        // As the panel sends it for a window with a project.
        let workspace::Write::Active { before, after } =
            local_write(&store, &record, 0, "amount", json!(7))
        else {
            panic!("the project's own workspace is written in place");
        };
        call(
            &kernel,
            WorkspaceCall::Replace {
                expected: model(&before),
                next: model(&after),
            },
        );
        let state = |k: &KernelHandle| {
            let ws = &k.published(PROJECT).unwrap().workspace;
            serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(ws)).unwrap()
                ["custom"][0]["state"]["amount"]
                .clone()
        };
        assert_eq!(state(&kernel), json!(7));
        // Undo undoes the document edit; the tool setting stays.
        call(&kernel, Call::History(HistoryCall::Undo));
        assert_eq!(opacity(&kernel, "l0"), 100.);
        assert_eq!(state(&kernel), json!(7));
    }

    #[test]
    fn controls_bind_as_v1_source_bindings() {
        let b = |c: Value| match bind(&c) {
            Bind::Fill(k) => format!("fill {k}"),
            Bind::Composition(k) => format!("comp {k}"),
            Bind::Layer { target, path } => format!("layer {target} {path}"),
            Bind::Local(k) => format!("local {k}"),
            Bind::None(_) => "none".into(),
        };
        let gradient = workspace::preset("gradient").unwrap()["custom"][0].clone();
        let kinds: Vec<_> = gradient["controls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| b(c.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                "fill type",
                "fill startColor",
                "fill endColor",
                "fill angle",
                "fill midpoint"
            ]
        );
        assert_eq!(
            b(json!({"target": "composition", "path": "composition.fps"})),
            "comp fps"
        );
        assert_eq!(
            b(json!({"binding": {"target": "$selection", "path": "properties.opacity"}})),
            "layer $selection properties.opacity"
        );
        assert_eq!(b(json!({"stateKey": "amount"})), "local amount");
        assert_eq!(b(json!({"param": "Speed"})), "none");
    }

    #[test]
    fn a_stop_field_makes_a_two_stop_gradient_and_type_switches_it() {
        let solid = json!({"type": "solid", "stops": [{"color": "#112233", "position": 0}]});
        let f = set_fill(solid.clone(), "endColor", json!("#FFFFFF"));
        assert_eq!(f["type"], "linear");
        assert_eq!(f["stops"][0]["color"], "#112233");
        assert_eq!(f["stops"][1], json!({"color": "#FFFFFF", "position": 100}));
        let f = set_fill(f, "midpoint", json!(40));
        assert_eq!(f["stops"][1]["position"], 40);
        assert_eq!(
            set_fill(f.clone(), "type", json!("radial"))["type"],
            "radial"
        );
        assert_eq!(set_fill(f, "type", json!("bogus"))["type"], "solid");
    }
}
