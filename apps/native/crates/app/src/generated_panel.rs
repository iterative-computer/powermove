//! Generated panels (S102): a workspace's `custom` sections, drawn natively.
//! A control binds as v1's `sourceBinding` (`core/generated-bindings.ts`):
//! the composition (its fields and background fill), a layer's content,
//! properties, transform or layer fields (`$selection` is the first selected
//! layer), or a local `stateKey` kept in the workspace record. Numbers, text
//! and colours are typed in a small sheet, a select opens its options as a
//! menu, toggles and buttons act on click. Rows are the inspector's: the
//! label in its column and a value well (a toggle as Off and On segments, a
//! slider filled to its place), a gradient's colours as one ramp above
//! them. Each document edit is one Undo step named for its control; a
//! `stateKey` edit is tool state, no Undo step (v1's `local` edit mode). A
//! sheet or menu edits the layer its control targeted when it opened. A
//! binding V2 can't write yet shows disabled with why.
#[path = "panel_build.rs"]
pub(crate) mod panel_build;

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
    window: &Window,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    let (_, _, def) = section(v, id)?;
    if let Some(state) = v.panel_builds.get_mut(id) { state.tick(); }
    Some(render(v, id, &def, rect, window, cx))
}

/// Generated definitions arrive through the kernel workspace publication,
/// including agent-authored workspace edits. Compare authored controls rather
/// than local state values so editing a finished slider never restarts a build.
pub(crate) fn publication(
    builds: &mut std::collections::HashMap<String, panel_build::State>,
    before: &Value,
    after: &Value,
) {
    let sections = after["custom"].as_array().cloned().unwrap_or_default();
    builds.retain(|id, state| state.explicit || sections.iter().any(|s| s["id"].as_str() == Some(id)));
    for section in &sections {
        let Some(id) = section["id"].as_str() else { continue; };
        if builds.get(id).is_some_and(|state| state.explicit) { continue; }
        let previous = before["custom"].as_array().and_then(|sections| sections.iter().find(|s| s["id"] == id));
        let controls = section["controls"].as_array().cloned().unwrap_or_default();
        let changed: Vec<_> = controls.iter().enumerate().filter_map(|(index, control)| {
            let old = previous.and_then(|s| s["controls"].get(index));
            (old != Some(control)).then_some(index)
        }).collect();
        if !changed.is_empty() || (previous.is_none() && controls.is_empty()) {
            builds.entry(id.to_owned()).or_default().published(changed, section["building"] != true);
        } else if section["building"] != true {
            if let Some(state) = builds.get_mut(id) { state.complete = true; }
        }
    }
}

/// The canvas owns no hit targets or focus handles. Every primitive is clipped
/// to the reserved body, so the viewer and neighbouring panels stay untouched.
fn build_surface(
    state: panel_build::State,
    rect: pm_shell::layout::Rect,
    t: &pm_design::Tokens,
    playing: bool,
    fraction: f32,
    _window: &Window,
    cx: &mut Context<Controller>,
) -> AnyElement {
    let entity = cx.entity().downgrade();
    if state.animating(std::time::Instant::now(),t.accessibility.reduce_motion) {
    cx.spawn(async move |_, cx| {
        cx.background_executor().timer(std::time::Duration::from_millis(if playing { 33 } else { 16 })).await;
        let _ = entity.update(cx, |_, cx| cx.notify());
    }).detach();
    }
    let age = state.started.elapsed().as_secs_f32();
    let elapsed = state.changed.elapsed().as_secs_f32();
    build_frame(state, rect, t, playing, age, elapsed, fraction, false)
}

/// Fixed event-relative times let headless captures exercise the live painter.
fn build_frame(
    state: panel_build::State,
    rect: pm_shell::layout::Rect,
    t: &pm_design::Tokens,
    playing: bool,
    age: f32,
    elapsed: f32,
    fraction: f32,
    ink_only: bool,
) -> AnyElement {
    let reduced = t.accessibility.reduce_motion;
    let height = (rect.h - t.metrics.header).max(0.);
    let fade = match state.stage {
        panel_build::Stage::Ready => 1.-pm_design::dots::resolve(elapsed),
        panel_build::Stage::Erase => 1.-panel_build::fade(elapsed,0.24),
        _ => panel_build::fade(age,0.24),
    };
    let (accent,text,muted)=(t.colors.accent,t.colors.text,t.colors.muted);
    let radius=t.radius.lg;
    let dim=panel_build::playback(playing).0;
    let _ = fraction;
    div().absolute().inset_0().h(px(height)).overflow_hidden().rounded(px(t.radius.lg))
        .child(canvas(|_,_,_|(),move |bounds,_,window,_| {
            if ink_only || reduced || fade<=0. { return; }
            let size=[f32::from(bounds.size.width),f32::from(bounds.size.height)];
            let opening=panel_build::fade(age,0.48);
            window.with_content_mask(Some(ContentMask{bounds,fade_out:Default::default()}), |window| {
                for dot in pm_design::dots::aurora(size,age,accent,text,muted) {
                    if !inside_ground(dot.center,size,radius) {continue;}
                    // The thin initial row expands into a sparse-to-dense slot field.
                    if dot.center[1]>3.+(size[1]-3.)*opening {continue;}
                    if pm_design::dots::hash(dot.center)>opening {continue;}
                    let p=bounds.origin+point(px(dot.center[0]-dot.radius),px(dot.center[1]-dot.radius));
                    window.paint_quad(PaintQuad::new(Bounds::new(p,gpui::size(px(dot.radius*2.),px(dot.radius*2.))))
                        .corner_radii(px(dot.radius)).background(pm_shell::kit::ink(dot.color,dot.alpha*fade*dim)));
                }
            });
        }).size_full()).into_any_element()
}

const BUILD_ROW: f32 = 32.;
const BUILD_HEADER: f32 = 42.;
const BUILD_INSET: f32 = 16.;

fn slot_surface(t: &pm_design::Tokens, build: Option<&panel_build::State>) -> u32 {
    let _=build;
    t.colors.panel
}

fn inside_ground(point: [f32;2], size: [f32;2], radius: f32) -> bool {
    let r=radius.min(size[0].min(size[1])*0.5);
    let dx=point[0]-point[0].clamp(r,size[0]-r);
    let dy=point[1]-point[1].clamp(r,size[1]-r);
    dx*dx+dy*dy <= r*r
}

fn ground(t: &pm_design::Tokens, build: Option<&panel_build::State>) -> Div {
    let resolve=build.map_or(1.,|b|b.resolve(b.now()));
    if resolve>=1. || t.accessibility.reduce_motion {return div().absolute().inset_0().rounded(px(t.radius.lg)).bg(rgb(t.colors.panel));}
    let color=t.colors.panel;
    let radius=t.radius.lg;
    div().absolute().inset_0().child(canvas(|_,_,_|(),move |bounds,_,window,_| {
        window.with_content_mask(Some(ContentMask{bounds,fade_out:Default::default()}),|window| {
            if resolve>0. {window.paint_quad(PaintQuad::new(bounds).background(pm_shell::kit::ink(color,resolve)));}
            let dimensions=[f32::from(bounds.size.width),f32::from(bounds.size.height)];
            for dot in pm_design::dots::matrix(dimensions,4.,|_|[1.;4]) {
                if !inside_ground(dot.center,dimensions,radius) {continue;}
                let p=bounds.origin+point(px(dot.center[0]-dot.radius),px(dot.center[1]-dot.radius));
                window.paint_quad(PaintQuad::new(Bounds::new(p,size(px(dot.radius*2.),px(dot.radius*2.))))
                    .corner_radii(px(dot.radius)).background(pm_shell::kit::ink(color,1.-resolve)));
            }
        });
    }).size_full())
}

fn panel_title(def: &Value, t: &pm_design::Tokens) -> Div {
    div().h(px(BUILD_HEADER)).flex_none().px(px(BUILD_INSET)).flex().items_center()
        .text_size(px(14.)).font_weight(FontWeight::MEDIUM).text_color(rgb(t.colors.text))
        .child(def["title"].as_str().unwrap_or("Controls").to_owned())
}

/// The actual generated slider's label, track, value fill and numeric text.
/// Sharing this element prevents capture-only stand-ins from drifting from UI.
fn slider_control(control: &Value, current: &Value, t: &pm_design::Tokens) -> Div {
    let fraction = control_fraction(control, current);
    let text = format!("{}{}", shown(current), control["unit"].as_str().unwrap_or(""));
    div().h(px(BUILD_ROW)).flex_none().px(px(BUILD_INSET)).child(
        div().relative().size_full().text_size(px(12.)).text_color(rgb(t.colors.secondary))
            .child(div().absolute().left_0().top_0().h_full().w(relative(0.34)).flex().items_center()
                .truncate().child(control["label"].as_str().unwrap_or("").to_owned()))
            .child(div().absolute().left(relative(0.38)).top(px(15.)).w(relative(0.44)).h(px(2.))
                .rounded(px(1.)).bg(pm_shell::kit::ink(t.colors.text, 0.14))
                .child(div().h_full().w(relative(fraction)).rounded(px(1.)).bg(rgb(t.colors.text))))
            .child(div().absolute().left(relative(0.87)).top_0().h_full().w(relative(0.13)).flex()
                .items_center().justify_end().text_size(px(11.)).text_color(rgb(t.colors.text))
                .child(div().flex_none().whitespace_nowrap()
                    .font_family(pm_design::TEXT_FONT).font_features(pm_shell::kit::tabular())
                    .letter_spacing(px(0.)).child(text))))
}

fn control_fraction(control: &Value, current: &Value) -> f32 {
    match (control["min"].as_f64(), control["max"].as_f64(), current.as_f64()) {
        (Some(lo), Some(hi), Some(value)) if hi > lo => ((value-lo)/(hi-lo)).clamp(0., 1.) as f32,
        _ => 0.5,
    }
}

pub(crate) fn control_opacity(build: Option<&panel_build::State>, index: usize, reduced: bool) -> f32 {
    let Some(build)=build else {return 1.;};
    if build.stage==panel_build::Stage::Ready {return 1.;}
    build.committed.get(&index).map_or(0.,|at| if reduced {panel_build::reveal(build.now().saturating_duration_since(*at).as_secs_f32(),true)} else {1.})
}
pub(crate) fn dot_filter(build: Option<&panel_build::State>, index: Option<usize>, t: &pm_design::Tokens) -> Vec<Filter> {
    let Some(build)=build else {return vec![];};
    if t.accessibility.reduce_motion {return vec![];}
    let now=build.now();
    let detail=index.or(Some(0)).and_then(|i|build.committed.get(&i)).map_or(0.6,|at|pm_design::dots::detail(now.saturating_duration_since(*at).as_secs_f32()));
    vec![Filter::DotMatrix{detail,resolve:build.resolve(now),accent:t.colors.accent}]
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

/// The control a gradient ramp's end edits: v1's `startColor` / `endColor`
/// (a colour control named so by its state key, parameter or label).
fn ramp_end(controls: &[Value], end: &str) -> Option<usize> {
    controls.iter().position(|c| {
        let name: String = [&c["stateKey"], &c["param"], &c["label"]]
            .into_iter()
            .find_map(Value::as_str)
            .unwrap_or("")
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        c["type"] == "color" && name.eq_ignore_ascii_case(end)
    })
}

fn render(
    v: &mut Controller,
    id: &str,
    def: &Value,
    rect: pm_shell::layout::Rect,
    window: &Window,
    cx: &mut Context<Controller>,
) -> AnyElement {
    use pm_shell::kit::ink;
    let t = v.host.borrow().tokens.clone();
    let (c, m, ty, tr) = (&t.colors, &t.metrics, &t.typography, &t.translucency);
    let build = v.panel_builds.get(id).cloned();
    let overlay = build.as_ref().filter(|b| b.stage != panel_build::Stage::Ready || b.changed.elapsed().as_secs_f32() < 0.7)
        .map(|b| {
            let control = &def["controls"][b.row];
            let current = value(v, def, control).unwrap_or_else(|_| control["def"].clone());
            let fraction = control_fraction(control, &current);
            (build_surface(b.clone(), rect, &t, v.running(), fraction, window, cx),
             build_frame(b.clone(), rect, &t, v.running(), b.started.elapsed().as_secs_f32(), b.changed.elapsed().as_secs_f32(), fraction, true))
        });
    let (under, over) = match overlay { Some((under,over)) => (Some(under),Some(over)), None => (None,None) };
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
        .relative().rounded(px(t.radius.lg))
        .w_full()
        .h(px((rect.h - m.header).max(0.)))
        .overflow_y_scroll()
        .text_size(px(ty.body))
        .child(ground(&t,build.as_ref()))
        .children(under)
        .child(panel_title(def, &t).filter(dot_filter(build.as_ref(),None,&t)));
    if let Some(note) = def["note"].as_str().filter(|n| !n.is_empty()) {
        body = body.child(
            div()
                .px(px(m.inset))
                .pb(px(m.gap))
                .text_size(px(ty.label))
                .text_color(rgb(c.muted))
                .child(note.to_owned()),
        );
    }
    // The inspector's row: the label in its column, the value well taking
    // the rest (the kit's InspectorRow and value well).
    let row = |label: &str| {
        div()
            .flex()
            .items_center()
            .gap(px(m.gap))
            .h(px(m.row))
            .flex_none()
            .px(px(m.inset))
            .child(
                div()
                    .w(px(m.label_column))
                    .flex_shrink(1.)
                    .min_w(px(0.))
                    .truncate()
                    .text_size(px(ty.label))
                    .text_color(rgb(c.secondary))
                    .child(label.to_owned()),
            )
    };
    let well = |eid: String, label: String, enabled: bool| {
        div()
            .id(SharedString::from(eid))
            .role(Role::Button)
            .aria_label(label)
            .relative()
            .flex_1()
            .min_w(px(m.value_column.min(m.control * 2.)))
            .h(px(m.field))
            .px(px(m.gap))
            .flex()
            .items_center()
            .gap(px(m.gap))
            .overflow_hidden()
            .rounded(px(m.control_radius))
            .bg(ink(c.text, tr.hover))
            .text_color(rgb(if enabled { c.text } else { c.muted }))
    };
    // A gradient's two colours as one ramp above their rows (v1's RampField);
    // each half opens its colour.
    if let (Some(start), Some(end)) = (
        ramp_end(controls, "startcolor"),
        ramp_end(controls, "endcolor"),
    ) && let (Ok(a), Ok(b)) = (
        value(v, def, &controls[start]),
        value(v, def, &controls[end]),
    ) {
        let colour = |x: &Value| {
            crate::v4_compile::parse_hex(x.as_str().unwrap_or("")).unwrap_or(c.text)
        };
        let (from, to) = (colour(&a), colour(&b));
        let half = |eid: &str, index: usize, text: Value| {
            let panel = id.to_owned();
            let text = shown(&text);
            div()
                .id(SharedString::from(format!("generated-{id}-{eid}")))
                .role(Role::Button)
                .aria_label(controls[index]["label"].as_str().unwrap_or("").to_owned())
                .flex_1()
                .h_full()
                .on_click(cx.listener(move |v, _, _, cx| {
                    v.generated_edit(panel.clone(), index, text.clone(), cx)
                }))
        };
        body = body.child(
            div().px(px(m.inset)).pb(px(m.gap)).child(
                div()
                    .flex()
                    .h(px(m.control))
                    .rounded(px(m.control_radius))
                    .border_1()
                    .border_color(ink(c.text, tr.edge))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(rgb(from), 0.),
                        linear_color_stop(rgb(to), 1.),
                    ))
                    .child(half("ramp-start", start, a))
                    .child(half("ramp-end", end, b)),
            ),
        );
    }
    let chevron = |window: &Window| {
        pm_shell::symbol::symbol(
            "chevron.up.chevron.down",
            m.symbol_inline,
            pm_shell::symbol::Weight::Semibold,
            c.secondary,
            window,
        )
    };
    for (index, control) in controls.iter().enumerate() {
        let kind = control["type"].as_str().unwrap_or("");
        let label = control["label"].as_str().unwrap_or("").to_owned();
        let panel = id.to_owned();
        let eid = format!("generated-{id}-{index}");
        let element = match kind {
            "note" => div()
                .px(px(m.inset))
                .py(px(m.gap / 2.))
                .text_size(px(ty.label))
                .text_color(rgb(c.muted))
                .child(control["text"].as_str().unwrap_or(&label).to_owned()),
            "readout" => row(&label).child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(rgb(c.text))
                    .child(readout(v, control["source"].as_str().unwrap_or(""))),
            ),
            "slider" => {
                let current = value(v, def, control).unwrap_or_else(|_| control["def"].clone());
                let text = shown(&current);
                div().child(slider_control(control, &current, &t).id(SharedString::from(eid))
                    .role(Role::Slider).aria_label(label.clone())
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.generated_edit(panel.clone(), index, text.clone(), cx)
                    })))
            }
            "button" => {
                let accent = control["primary"] == true && primaries == 1;
                div().px(px(m.inset)).py(px(m.gap / 2.)).child(
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
                        .bg(if accent {
                            rgb(c.accent)
                        } else {
                            ink(c.text, tr.hover)
                        })
                        .text_color(rgb(if accent { c.accent_text } else { c.text }))
                        .child(label)
                        .on_click(
                            cx.listener(move |v, _, _, cx| v.generated_press(&panel, index, cx)),
                        ),
                )
            }
            _ => match value(v, def, control) {
                Err(why) => row(&label).child(
                    well(eid, label.clone(), false)
                        .child("—")
                        .tooltip(pm_shell::view::tooltip(&t, why.to_owned())),
                ),
                Ok(current) => row(&label).child(match kind {
                    // Off and On as two segments (v1's ToggleField): the
                    // state reads as a word.
                    "toggle" => {
                        let on = current == true;
                        let segment = |name: &'static str, is: bool| {
                            let panel = panel.clone();
                            div()
                                .id(SharedString::from(format!("{eid}-{name}")))
                                .role(Role::RadioButton)
                                .aria_label(format!("{label} {name}"))
                                .aria_toggled(if on == is {
                                    gpui::Toggled::True
                                } else {
                                    gpui::Toggled::False
                                })
                                .flex_1()
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(m.inner_radius))
                                .when(on == is, |d| {
                                    d.bg(ink(c.text, tr.selected)).text_color(rgb(c.text))
                                })
                                .when(on != is, |d| d.text_color(rgb(c.secondary)))
                                .child(name)
                                .on_click(cx.listener(move |v, _, _, cx| {
                                    if on != is {
                                        v.generated_set(&panel, index, json!(is), cx)
                                    }
                                }))
                        };
                        well(eid.clone(), label.clone(), true)
                            .px(px(2.))
                            .gap(px(2.))
                            .child(segment("Off", false))
                            .child(segment("On", true))
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
                        well(eid, label.clone(), true)
                            .child(div().flex_1().min_w(px(0.)).truncate().child(text))
                            .child(chevron(window))
                            .on_click(cx.listener(move |v, e: &ClickEvent, _, _| {
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
                            }))
                    }
                    "slider" | "number" | "text" | "color" => {
                        let text = shown(&current);
                        let mut f = well(eid, label.clone(), true);
                        if kind == "slider"
                            && let (Some(lo), Some(hi), Some(x)) = (
                                control["min"].as_f64(),
                                control["max"].as_f64(),
                                current.as_f64(),
                            )
                            && hi > lo
                        {
                            // Filled to its place in the range (v1's
                            // filled slider), the number over it.
                            let fraction = ((x - lo) / (hi - lo)).clamp(0., 1.) as f32;
                            f = f.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .h_full()
                                    .w(relative(fraction))
                                    .rounded(px(m.control_radius))
                                    .bg(ink(c.text, tr.hover)),
                            );
                        }
                        if kind == "color"
                            && let Some(hex) = crate::v4_compile::parse_hex(&text)
                        {
                            f = f.child(
                                div()
                                    .flex_none()
                                    .size(px(m.field - 8.))
                                    .rounded(px(m.inner_radius / 2.))
                                    .border_1()
                                    .border_color(ink(c.text, tr.edge))
                                    .bg(rgb(hex)),
                            );
                        }
                        let unit = control["unit"].as_str().unwrap_or("").to_owned();
                        f.child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .font_features(pm_shell::kit::tabular())
                                .child(format!("{text}{unit}")),
                        )
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.generated_edit(panel.clone(), index, text.clone(), cx)
                        }))
                    }
                    _ => well(eid, label.clone(), false)
                        .child(shown(&current))
                        .tooltip(pm_shell::view::tooltip(
                            &t,
                            "This control can’t be edited here yet.",
                        )),
                }),
            },
        };
        body = body.child(div().flex_none().w_full().child(element).filter(dot_filter(build.as_ref(),Some(index),&t)).opacity(control_opacity(build.as_ref(), index, t.accessibility.reduce_motion)));
    }
    if let Some(overlay) = over { body = body.child(overlay); }
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

    #[test]
    fn panel_build_publications_ignore_state_edits() {
        let mut builds = std::collections::HashMap::new();
        let before = json!({"custom":[]});
        let after = json!({"custom":[{"id":"new", "controls":[{"type":"slider"},{"type":"number"}]}]});
        super::publication(&mut builds, &before, &after);
        assert_eq!(builds["new"].pending.len(), 2);
        assert!(builds["new"].complete);
        let mut edited = after.clone();
        edited["custom"][0]["state"] = json!({"amount":5});
        super::publication(&mut builds, &after, &edited);
        assert_eq!(builds["new"].pending.len(), 2);
        super::publication(&mut builds, &after, &before);
        assert!(builds.is_empty());
    }

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
            trashed: vec![],
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

#[cfg(test)]
mod panel_build_capture {
    use super::{build_frame, panel_build, panel_title, slider_control, control_opacity};
    use gpui::{AppContext, HeadlessAppContext, IntoElement, ParentElement, Render, Styled, px, size};
    use pm_design::{Accessibility, Scheme, Tokens};
    use serde_json::json;
    use std::sync::Arc;

    struct Frame { state: panel_build::State, tokens: Tokens, age: f32, elapsed: f32, definition: serde_json::Value }
    impl Render for Frame {
        fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let mut body = gpui::div().relative().opacity(if self.state.stage == panel_build::Stage::Erase { 1.-panel_build::fade(self.elapsed,0.36) } else { 1. }).w(px(300.)).h(px(260.)).flex().flex_col().overflow_hidden().rounded(px(self.tokens.radius.lg)).child(super::ground(&self.tokens,Some(&self.state)))
                .font_family(pm_design::TEXT_FONT);
            let rect = pm_shell::layout::Rect { x: 0., y: 0., w: 300., h: 260. + self.tokens.metrics.header };
            let fraction = super::control_fraction(&self.definition["controls"][self.state.row.min(4)], &self.definition["controls"][self.state.row.min(4)]["def"]);
            body = body.child(build_frame(self.state.clone(), rect, &self.tokens, false, self.age, self.elapsed, fraction, false));
            body = body.child(panel_title(&self.definition, &self.tokens).filter(super::dot_filter(Some(&self.state),None,&self.tokens)));
            for (index, control) in self.definition["controls"].as_array().unwrap().iter().enumerate() {
                body = body.child(slider_control(control, &control["def"], &self.tokens)
                    .filter(super::dot_filter(Some(&self.state),Some(index),&self.tokens)).opacity(control_opacity(Some(&self.state), index, self.tokens.accessibility.reduce_motion)));
            }
            body = body.child(build_frame(self.state.clone(), rect, &self.tokens, false, self.age, self.elapsed, fraction, true));
            let body=body.opacity(if self.state.stage==panel_build::Stage::Erase {1.-panel_build::fade(self.elapsed,0.24)}else{1.});
            gpui::div().size_full().p(px(16.)).bg(gpui::rgb(if self.tokens.scheme == Scheme::Light { 0xececec } else { 0x151515 })).child(body)

        }
    }

    #[test]
    #[ignore = "writes PNGs; set PM_PANEL_CAPTURE"]
    fn capture_panel_build_key_frames() {
        let dir = std::env::var("PM_PANEL_CAPTURE").expect("PM_PANEL_CAPTURE");
        std::fs::create_dir_all(&dir).unwrap();
        let mut cases=Vec::new();
        for scheme in [Scheme::Dark,Scheme::Light] {
            for (label,stage,row,elapsed) in [
                ("opening",panel_build::Stage::Wake,0,0.12),
                ("field",panel_build::Stage::Sketch,0,0.42),
                ("coarse",panel_build::Stage::Ink,0,0.35),
                ("fine",panel_build::Stage::Ink,1,0.82),
                ("waiting",panel_build::Stage::Rest,4,2.),
                ("resolving",panel_build::Stage::Ready,4,0.35),
                ("ready",panel_build::Stage::Ready,4,0.8),
                ("cancel",panel_build::Stage::Erase,4,0.12),
            ] {cases.push((format!("{:?}-{label}",scheme),stage,row,elapsed,scheme,false));}
        }
        for frame in 0..=90 {
            let time=frame as f32/15.;
            let (stage,row,elapsed)=if time<0.48 {(panel_build::Stage::Wake,0,time)}
                else if time<4.98 {let p=(time-0.48)/0.9;(panel_build::Stage::Ink,(p as usize).min(4),(p.fract()*0.9))}
                else {(panel_build::Stage::Ready,4,time-4.98)};
            cases.push((format!("frame-{frame:03}"),stage,row,elapsed,Scheme::Dark,false));
        }
        cases.push(("reduced-motion".into(),panel_build::Stage::Ink,1,0.2,Scheme::Dark,true));
        for (name, stage, row, elapsed, scheme, reduced) in cases {
            objc2::rc::autoreleasepool(|_| {
                let mut cx = HeadlessAppContext::with_platform(
                    Arc::new(gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load).with_automatic_optical_sizing()),
                    Arc::new(()), || Some(Box::new(gpui_apple::metal_renderer::MetalHeadlessRenderer::new())),
                );
                let mut state = panel_build::State::default();
                state.explicit = true;
                state.planned.extend(0..5);
                state.pending.push_back(0);
                state.changed -= std::time::Duration::from_millis(241);
                state.tick();
                assert_eq!(state.stage, panel_build::Stage::Ink);
                state.stage = stage;
                state.row = row;
                state.changed = std::time::Instant::now() - std::time::Duration::from_secs_f32(elapsed);
                state.pending = ((row+1)..5).collect();
                let count = if matches!(stage,panel_build::Stage::Ready|panel_build::Stage::Erase) { 5 } else if stage == panel_build::Stage::Rest { row+1 } else { row };
                for index in 0..count { state.committed.insert(index, std::time::Instant::now() - std::time::Duration::from_secs(2)); }
                if stage == panel_build::Stage::Ink { state.committed.insert(row, std::time::Instant::now() - std::time::Duration::from_secs_f32(elapsed)); }
                state.sampled_at=Some(std::time::Instant::now());
                let definition = json!({"title":"Colour grade","controls":[
                    {"type":"slider","label":"Exposure","def":0.35,"min":-1,"max":1},
                    {"type":"slider","label":"Contrast","def":12,"min":0,"max":30},
                    {"type":"slider","label":"Saturation","def":108,"min":0,"max":135},
                    {"type":"slider","label":"Warmth","def":5600,"min":2000,"max":8000},
                    {"type":"slider","label":"Tint","def":-4,"min":-10,"max":20}]});
                let tokens = Tokens::new(scheme, Accessibility { reduce_motion: reduced, ..Default::default() });
                let window = cx.open_window(size(px(332.), px(292.)), |_, cx| cx.new(|_| Frame { state, tokens, age: name.strip_prefix("frame-").and_then(|n|n.parse::<f32>().ok()).map_or(if stage == panel_build::Stage::Wake {elapsed}else{1.},|n|n/15.), elapsed, definition })).unwrap();
                for _ in 0..4 { cx.run_until_parked(); }
                cx.update_window(window.into(), |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                cx.run_until_parked();
                let image = cx.capture_screenshot(window.into()).unwrap();
                assert_eq!(image.width(), 664, "capture must be 2x");
                assert_eq!(image.height(), 584);
                assert!(image.pixels().any(|p| p.0[0] != 0 || p.0[1] != 0 || p.0[2] != 0), "empty {name}");
                image.save(format!("{dir}/{name}.png")).unwrap();
                println!("CAPTURE {dir}/{name}.png");
            });
        }
    }
}

/// A pre-authoring slot; the target can be either a generated or compat panel.
#[derive(Clone)]
pub(crate) struct AgentBuild {
    pub title: String,
    pub planned: Vec<Value>,
    pub target: Option<String>,
    pub finishing: bool,
    pub owner: Option<String>,
    pub snapshot: Option<(usize,std::sync::Arc<RenderImage>)>,
}

pub(crate) fn agent_slot(v: &mut Controller, id: &str, rect: pm_shell::layout::Rect, window: &mut Window, cx: &mut Context<Controller>) -> Option<AnyElement> {
    let build = v.agent_panel_builds.get(id)?.clone();
    if build.owner.as_deref().is_some_and(|run| !pm_agents::launch::grants::run_live(run)) {
        if let Some(state) = v.panel_builds.get_mut(id) {
            if state.stage != panel_build::Stage::Erase { state.event("cancel", state.row); }
            if state.changed.elapsed().as_secs_f32() >= 0.36 {
                let id = id.to_owned();
                let owner = cx.weak_entity();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |v, cx| {
                    if let Some(shell) = v.shell.clone().and_then(|shell| shell.upgrade()) {
                        shell.update(cx, |e, cx| { e.release_agent_panel(&id); cx.notify(); });
                    }
                    if let Some(build) = v.agent_panel_builds.remove(&id) {
                        if let Some(target) = build.target {
                            if let Some(page)=v.extensions.page_of(&target){page.borrow_mut().finish_panel_build(&target);}
                            v.panel_builds.remove(&target);
                        }
                    }
                    v.panel_builds.remove(&id); cx.notify();
                    });
                });
            }
        }
    }
    let mut content = None;
    let mut generated = false;
    let mut native_tree=false;
    let mut web_capture=None;
    if let Some(target) = &build.target {
        content = v.extension_panel(target, rect, cx);
        if content.is_none() { content=v.builtin_panel(target,rect,window,cx); native_tree=content.is_some(); }
        let rendered = if v.extensions.owns_panel(target) { v.extension_build_rendered(target) }
            else if content.is_some() { true }
            else { section(v, target).is_some() };
        let rendered=if v.extensions.owns_panel(target) {
            if let Some(page)=v.extensions.page_of(target) {
                let state=v.panel_builds.get(id).cloned().unwrap_or_default();
                let resolve=state.resolve(state.now());
                let ready=(state.stage==panel_build::Stage::Ready).then_some(resolve);
                let captured=build.snapshot.is_some();
                let (_,live)=pm_design::dots::snapshot_visibility(captured,ready);
                let revision=state.committed.values().max().map_or(0,|at|at.saturating_duration_since(state.started).as_nanos() as u64);
                let mut capture_done=true;
                if resolve>=1. {page.borrow_mut().finish_panel_build(target);}
                else {
                    let (done,image)=page.borrow_mut().panel_build_snapshot(target,revision,live);
                    capture_done=done;
                    if done && image.is_none(){v.agent_panel_builds.get_mut(id).unwrap().snapshot=None;}
                    if let Some((source,image))=image {
                        let key=(&*image as *const objc2_app_kit::NSImage) as usize;
                        let cached=v.agent_panel_builds.get_mut(id).unwrap();
                        if cached.snapshot.as_ref().map(|s|s.0)!=Some(key) {
                            cached.snapshot=pm_shell::native_control::snapshot_image(&image).map(|image|(key,image));
                        }
                        web_capture=cached.snapshot.as_ref().map(|(_,image)|(source,image.clone()));
                    }
                    content=None;
                    if !done && web_capture.is_none() {return Some(div().relative().size_full().child(build_surface(state,rect,&v.host.borrow().tokens.clone(),v.running(),0.5,window,cx)).into_any_element());}
                }
                rendered && capture_done
            } else {rendered}
        } else {rendered};
        if rendered && build.finishing {
            if let Some(state) = v.panel_builds.get_mut(id) { state.complete = true; }
        }
        if content.is_none() {
            if let Some((_, _, def)) = section(v, target) {
                generated = true;
                if let Some(state) = v.panel_builds.get(id).cloned() { v.panel_builds.insert(target.clone(), state); }
                content = Some(render(v, target, &def, rect, window, cx));
            }
        }
    }
    let state = v.panel_builds.get_mut(id)?;
    state.tick();
    let state = state.clone();
    if generated && state.stage != panel_build::Stage::Erase { return content; }
    if state.stage == panel_build::Stage::Ready && state.changed.elapsed().as_secs_f32() >= 0.7 { return content; }
    let tokens = v.host.borrow().tokens.clone();
    let overlay = build_surface(state.clone(), rect, &tokens, v.running(), 0.5, window, cx);
    let root_alpha=if state.stage==panel_build::Stage::Erase {1.-panel_build::fade(state.changed.elapsed().as_secs_f32(),0.36)}else{1.};
    let mut body=div().relative().size_full().overflow_hidden().opacity(root_alpha).bg(rgb(tokens.colors.panel)).child(overlay);
    if let Some((source,image))=web_capture {
        body=body.child(div().absolute().inset_0().overflow_hidden().filter(dot_filter(Some(&state),None,&tokens))
            .child(img(image).w(px(source.w as f32)).h(px(source.h as f32))));
    } else if let Some(content)=content {
        body=body.child(div().size_full().child(content).filter(if native_tree{vec![]}else{dot_filter(Some(&state),None,&tokens)}));
    } else if build.target.is_none() {
        body=body.child(div().h(px(BUILD_HEADER)).px(px(16.)).filter(dot_filter(Some(&state),None,&tokens)).child(build.title.clone()));
        for (row,control) in build.planned.iter().enumerate() {
            body=body.child(div().h(px(BUILD_ROW)).px(px(16.))
                .opacity(control_opacity(Some(&state),row,tokens.accessibility.reduce_motion))
                .filter(dot_filter(Some(&state),Some(row),&tokens))
                .child(control["label"].as_str().unwrap_or_default().to_owned()));
        }
    }
    Some(body.into_any_element())
}

/// Extension realms publish a subtree atomically; sample that same subtree.
pub(crate) fn extension_build(v: &mut Controller, id: &str, content: AnyElement, rect: pm_shell::layout::Rect, window: &Window, cx: &mut Context<Controller>) -> AnyElement {
    if !v.extensions.surfaces.contains_key(id) { return content; }
    let t=v.host.borrow().tokens.clone();
    let Some(state)=v.panel_builds.get_mut(id) else {return content;};
    state.tick(); let state=state.clone();
    if state.stage==panel_build::Stage::Ready && state.changed.elapsed().as_secs_f32()>=0.7 {return content;}
    let field=build_surface(state.clone(),rect,&t,v.running(),0.5,window,cx);
    div().relative().size_full().child(field)
        .child(div().size_full().child(content)
            .opacity(control_opacity(Some(&state),0,t.accessibility.reduce_motion))).into_any_element()
}
