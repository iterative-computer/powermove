//! The Pen tool (G): draws and edits Bézier paths (the shipped viewer's
//! `path-editing.ts`). On a shape layer a path joins its `d.paths`; on any
//! other maskable layer it becomes a mask; with neither selected a new shape
//! layer holds it. Each press is one undo step.
//!
//! - Click empty canvas: a vertex on the open path (a new path if none);
//!   dragging pulls out its tangents, mirrored unless Option is held.
//! - Drag a vertex: moves it (Option: pulls its out tangent). Drag a tangent
//!   end: moves it, mirroring the other unless Option is held.
//! - Click the first vertex of the open path (three or more vertices), or
//!   double-click any vertex: closes the path. Command-click: deletes it.
use super::{GizmoSurface, Job, inverse_point, transform};
use gpui::{prelude::*, *};
use pm_kernel::{
    Command, LayerCommand, MaskCommand, PropertyCommand,
    commands::{layers::AddLayer, properties::SetProperty},
    new_id,
};
use pm_model::{Map, Value};

/// Layers a path can mask (shape layers hold paths themselves).
const MASKABLE: [&str; 7] = [
    "shape", "text", "solid", "image", "video", "precomp", "shader",
];
/// Grab radius around vertices and tangent ends, display points.
const REACH: f64 = 8.;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vertex {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub in_x: f64,
    pub in_y: f64,
    pub out_x: f64,
    pub out_y: f64,
}
/// One editable path: a shape layer's (`g.<id>`) or a mask's (`mp.<id>`).
#[derive(Clone, Debug)]
pub struct Target {
    pub prefix: String,
    pub path: String,
    /// Path space → composition.
    pub matrix: [f64; 6],
    pub closed: bool,
    pub vertices: Vec<Vertex>,
}
#[derive(Clone, Debug)]
pub struct PenPose {
    pub layer: String,
    pub kind: String,
    /// Locked layers (or inside locked groups) take no path edits.
    pub locked: bool,
    /// Layer space → composition, for a new path's first vertex.
    pub world: [f64; 6],
    pub targets: Vec<Target>,
}
fn mul(a: &[f64; 6], b: &[f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn rotate_scale(rotation: f64, sx: f64, sy: f64, x: f64, y: f64) -> [f64; 6] {
    let (s, c) = rotation.to_radians().sin_cos();
    [c * sx, s * sx, -s * sy, c * sy, x, y]
}
fn static_value(node: Option<&Value>, key: &str) -> Option<Value> {
    let v = node?.get("p")?.get(key)?;
    Some(v.get("v").cloned().unwrap_or_else(|| v.clone()))
}
/// The selected layer's paths at `time`, evaluated (keyframes included).
pub fn read_pen(p: &pm_kernel::Published, time: f64) -> Option<PenPose> {
    let id = p.selection.layers().first()?.clone();
    let root = p.snapshot.root()?;
    let info = pm_kernel::model::list_layers(root, None)
        .into_iter()
        .find(|l| l.id == id)?;
    if info.effectively_locked {
        return Some(PenPose {
            layer: id,
            kind: info.kind,
            locked: true,
            world: [1., 0., 0., 1., 0., 0.],
            targets: vec![],
        });
    }
    let layer =
        pm_kernel::model::layer_at(root, &pm_kernel::model::find_layer(root, &id)?)?.to_value();
    let layer = &layer;
    let loc = pm_kernel::model::find_layer(root, &id)?;
    // Off main, through the project's font-fed mirror.
    let (world, targets) = crate::text_host::query(p, |q| {
        let world = q.world_matrix(None, &id, time)?;
        // Static channels read as stored; animated ones through `evP`, which
        // evaluates a structured channel (`g.…`, `mp.…`) with its keys.
        let mut number = |node: Option<&Value>, key: &str, fallback: f64| {
            let short = key.rsplit('.').next().unwrap_or(key);
            let Some(prop) = node.and_then(|n| n.get("p")).and_then(|p| p.get(short)) else {
                return fallback;
            };
            let animated = prop
                .get("kf")
                .and_then(Value::as_array)
                .is_some_and(|k| !k.is_empty())
                || prop
                    .get("expr")
                    .and_then(Value::as_str)
                    .is_some_and(|e| !e.is_empty());
            let stored = prop.get("v").and_then(Value::as_f64);
            if !animated {
                return stored.unwrap_or(fallback);
            }
            let mut request = Map::new();
            request.insert("op", Value::from("evP"));
            request.insert("layer", Value::Number(loc.index as f64));
            request.insert("t", Value::Number(time));
            request.insert("comp", Value::from(loc.comp_id.clone().unwrap_or_default()));
            request.insert("key", Value::from(key));
            request.insert("prop", prop.clone());
            q.query(&Value::Object(request))
                .ok()
                .and_then(|v| v.as_f64())
                .or(stored)
                .unwrap_or(fallback)
        };
        let list = |node: Option<&Value>, key: &str| -> Vec<Value> {
            node.and_then(|n| n.get(key))
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
        };
        let text = |node: &Value, key: &str| {
            node.get(key)
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        };
        let paths = list(layer.get("d"), "paths");
        let mut targets = vec![];
        let path_target =
            |path: &Value,
             prefix: String,
             base: [f64; 6],
             number: &mut dyn FnMut(Option<&Value>, &str, f64) -> f64| {
                let vertices = list(Some(path), "vertices")
                    .iter()
                    .map(|v| {
                        let vid = text(v, "id");
                        let key =
                            |k: &str, number: &mut dyn FnMut(Option<&Value>, &str, f64) -> f64| {
                                number(Some(v), &format!("{prefix}.v.{vid}.{k}"), 0.)
                            };
                        Vertex {
                            x: key("x", number),
                            y: key("y", number),
                            in_x: key("inX", number),
                            in_y: key("inY", number),
                            out_x: key("outX", number),
                            out_y: key("outY", number),
                            id: vid.clone(),
                        }
                    })
                    .collect();
                let local = rotate_scale(
                    number(Some(path), &format!("{prefix}.rotation"), 0.),
                    number(Some(path), &format!("{prefix}.scaleX"), 100.) / 100.,
                    number(Some(path), &format!("{prefix}.scaleY"), 100.) / 100.,
                    number(Some(path), &format!("{prefix}.x"), 0.),
                    number(Some(path), &format!("{prefix}.y"), 0.),
                );
                let closed = static_value(Some(path), "closed") == Some(Value::Bool(true));
                Target {
                    prefix: prefix.clone(),
                    path: text(path, "id"),
                    matrix: mul(&base, &local),
                    closed,
                    vertices,
                }
            };
        // Group transforms nest through `parent`.
        let group = |start: &Value, number: &mut dyn FnMut(Option<&Value>, &str, f64) -> f64| {
            let mut m = [1., 0., 0., 1., 0., 0.];
            let mut at = paths
                .iter()
                .find(|c| text(c, "id") == text(start, "parent"))
                .cloned();
            let mut seen = 0;
            while let Some(parent) = at.filter(|_| seen < 32) {
                let prefix = format!("g.{}", text(&parent, "id"));
                let local = rotate_scale(
                    number(Some(&parent), &format!("{prefix}.rotation"), 0.),
                    number(Some(&parent), &format!("{prefix}.scaleX"), 100.) / 100.,
                    number(Some(&parent), &format!("{prefix}.scaleY"), 100.) / 100.,
                    number(Some(&parent), &format!("{prefix}.x"), 0.),
                    number(Some(&parent), &format!("{prefix}.y"), 0.),
                );
                m = mul(&local, &m);
                at = paths
                    .iter()
                    .find(|c| text(c, "id") == text(&parent, "parent"))
                    .cloned();
                seen += 1;
            }
            m
        };
        for path in &paths {
            if path.get("vertices").is_none() {
                continue;
            }
            let base = mul(&world, &group(path, &mut number));
            let prefix = format!("g.{}", text(path, "id"));
            targets.push(path_target(path, prefix, base, &mut number));
        }
        for mask in list(Some(layer), "masks") {
            let Some(path) = mask.get("path").filter(|p| p.get("vertices").is_some()) else {
                continue;
            };
            let mid = text(&mask, "id");
            let key = |k: &str| format!("m.{mid}.{k}");
            let base = mul(
                &world,
                &rotate_scale(
                    number(Some(&mask), &key("rotation"), 0.),
                    1.,
                    1.,
                    number(Some(&mask), &key("x"), 0.),
                    number(Some(&mask), &key("y"), 0.),
                ),
            );
            let path = path.clone();
            targets.push(path_target(&path, format!("mp.{mid}"), base, &mut number));
        }
        Ok((world, targets))
    })
    .ok()?;
    Some(PenPose {
        layer: id,
        kind: info.kind,
        locked: false,
        world,
        targets,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Point,
    In,
    Out,
    /// A vertex this press just placed: a drag pulls out its tangents.
    New,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub target: usize,
    pub index: usize,
    pub side: Side,
}
/// The vertex or tangent end under a composition point.
pub fn hit(pose: &PenPose, at: [f64; 2], scale: f64) -> Option<Hit> {
    let mut found = None;
    for (t, target) in pose.targets.iter().enumerate() {
        let Some(p) = inverse_point(&target.matrix, at) else {
            continue;
        };
        let linear = target.matrix[0].hypot(target.matrix[1]).max(0.01);
        let radius = REACH / scale.max(0.02) / linear;
        for (index, v) in target.vertices.iter().enumerate() {
            for (side, x, y) in [
                (Side::In, v.x + v.in_x, v.y + v.in_y),
                (Side::Out, v.x + v.out_x, v.y + v.out_y),
                (Side::Point, v.x, v.y),
            ] {
                if (p[0] - x).hypot(p[1] - y) < radius {
                    found = Some(Hit {
                        target: t,
                        index,
                        side,
                    });
                }
            }
        }
    }
    found
}
/// A press-to-release Pen edit.
pub struct PenDrag {
    pub origin: Point<Pixels>,
    pub layer: String,
    pub prefix: String,
    pub vertex: String,
    pub matrix: [f64; 6],
    pub start: [f64; 2],
    pub values: Vertex,
    pub side: Side,
    pub time: f64,
    pub moved: bool,
    /// Steps sent so far; the first never supersedes the press's own step.
    pub steps: usize,
    /// A layer the press made, selected once the edit is kept.
    pub select: Option<String>,
}
fn set(layer: &str, channel: String, value: f64, time: f64) -> Command {
    let mut set = SetProperty::new(
        layer,
        channel,
        Value::Number((value * 1000.).round() / 1000.),
        time,
    );
    set.preserve_hand_edits = false;
    set.mark_intent = true;
    PropertyCommand::Set(set).into()
}
/// The drag's channel writes, absolute from the press (path-editing.ts).
pub fn drag_commands(d: &PenDrag, at: [f64; 2], alt: bool) -> Vec<Command> {
    let Some(p) = inverse_point(&d.matrix, at) else {
        return vec![];
    };
    let (dx, dy) = (p[0] - d.start[0], p[1] - d.start[1]);
    let v = &d.values;
    let key = |k: &str| format!("{}.v.{}.{k}", d.prefix, d.vertex);
    let mut out = vec![];
    if d.side == Side::Point && !alt {
        out.push(set(&d.layer, key("x"), v.x + dx, d.time));
        out.push(set(&d.layer, key("y"), v.y + dy, d.time));
        return out;
    }
    let (mine, other, base) = if d.side == Side::In {
        (("inX", "inY"), ("outX", "outY"), (v.in_x, v.in_y))
    } else {
        (("outX", "outY"), ("inX", "inY"), (v.out_x, v.out_y))
    };
    out.push(set(&d.layer, key(mine.0), base.0 + dx, d.time));
    out.push(set(&d.layer, key(mine.1), base.1 + dy, d.time));
    if !alt {
        out.push(set(&d.layer, key(other.0), -base.0 - dx, d.time));
        out.push(set(&d.layer, key(other.1), -base.1 - dy, d.time));
    }
    out
}

/// A box with a 1 pt edge.
fn fill_box(
    b: Bounds<Pixels>,
    radius: Pixels,
    fill: impl Into<Background>,
    edge: Rgba,
) -> PaintQuad {
    PaintQuad::new(b)
        .corner_radii(radius)
        .background(fill)
        .border_widths(1f32)
        .border_color(edge)
}
impl GizmoSurface {
    pub(super) fn pen_reread(&mut self, cx: &mut Context<Self>) {
        if self.host.tool(cx) != super::Tool::Pen || self.host.playing(cx) {
            self.pen_ready = false;
            self.pen_active = None;
            if self.pen.take().is_some() {
                cx.notify();
            }
            return;
        }
        self.pen_reading += 1;
        let ticket = self.pen_reading;
        // A read taken while the Pen's own steps are still queued (a new path
        // layer is selected only after its first vertex lands) may see the
        // old selection; it must not end the open path.
        let own_work = self.busy || !self.queue.is_empty();
        let (kernel, project, time) = (
            self.kernel.clone(),
            self.project.clone(),
            self.host.time(cx),
        );
        cx.spawn(async move |this, cx| {
            let pose = cx
                .background_executor()
                .spawn(async move { read_pen(kernel.published(&project)?.as_ref(), time) })
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.pen_reading == ticket {
                    // The open path lives on while its layer is the one
                    // selected (a new path layer's own selection keeps it).
                    let own_work = own_work || s.busy || !s.queue.is_empty();
                    if !own_work
                        && s.pen_active.as_ref().map(|(layer, _)| layer)
                            != pose.as_ref().map(|p| &p.layer)
                    {
                        s.pen_active = None;
                    }
                    s.pen = pose;
                    s.pen_ready = true;
                    cx.notify();
                    if let Some((event, at, scale)) = s.pen_waiting.take() {
                        s.pen_down(&event, at, scale, cx);
                    }
                }
            });
        })
        .detach();
    }
    /// A Pen press: close, delete, grab, or place a vertex.
    pub(super) fn pen_down(
        &mut self,
        event: &MouseDownEvent,
        at: [f64; 2],
        scale: f64,
        cx: &mut Context<Self>,
    ) {
        let time = self.host.time(cx);
        // The first press after choosing the tool may beat its read; it runs
        // once the read lands, never by querying here.
        if !self.pen_ready {
            self.pen_waiting = Some((event.clone(), at, scale));
            self.pen_reread(cx);
            return;
        }
        let pose = self.pen.clone();
        if pose.as_ref().is_some_and(|p| p.locked) {
            return;
        }
        let cmd = event.modifiers.platform || event.modifiers.control;
        if let Some(pose) = &pose
            && let Some(h) = hit(pose, at, scale)
        {
            let target = &pose.targets[h.target];
            let vertex = &target.vertices[h.index];
            let closes = event.click_count > 1
                || (h.index == 0
                    && h.side == Side::Point
                    && target.vertices.len() >= 3
                    && self.pen_active.as_ref().map(|(_, p)| p.as_str())
                        == Some(target.path.as_str())
                    && !event.modifiers.alt
                    && !cmd);
            if closes {
                self.pen_active = None;
                let close = MaskCommand::ClosePath {
                    layer_id: pose.layer.clone(),
                    path_id: target.path.clone(),
                };
                self.queue
                    .push_back(Job::Transact("Close path", vec![close.into()], None));
                self.pump(cx);
                return;
            }
            if cmd {
                let delete = MaskCommand::DeleteVertex {
                    layer_id: pose.layer.clone(),
                    path_id: target.path.clone(),
                    vertex_id: vertex.id.clone(),
                };
                self.queue.push_back(Job::Transact(
                    "Delete path vertex",
                    vec![delete.into()],
                    None,
                ));
                self.pump(cx);
                return;
            }
            let Some(start) = inverse_point(&target.matrix, at) else {
                return;
            };
            self.pen_selected = Some((target.path.clone(), vertex.id.clone()));
            self.queue.push_back(Job::Begin("Edit path vertex"));
            self.pen_drag = Some(PenDrag {
                origin: event.position,
                layer: pose.layer.clone(),
                prefix: target.prefix.clone(),
                vertex: vertex.id.clone(),
                matrix: target.matrix,
                start,
                values: vertex.clone(),
                side: h.side,
                time,
                moved: false,
                steps: 0,
                select: None,
            });
            self.pump(cx);
            return;
        }
        // A new vertex: on the open path, or a new path (and a new layer,
        // inside the same gesture so Undo or Escape leaves nothing).
        let mut commands: Vec<Command> = vec![];
        let selected = pose
            .as_ref()
            .filter(|p| MASKABLE.contains(&p.kind.as_str()));
        let mut select = None;
        let (layer, kind, world) = match selected {
            Some(p) => (p.layer.clone(), p.kind.clone(), p.world),
            None => {
                let id = new_id("L");
                let mut content = Map::new();
                content.insert("paths", Value::Array(vec![]));
                let mut properties = Map::new();
                properties.insert("position.x", Value::Number(0.));
                properties.insert("position.y", Value::Number(0.));
                let add = AddLayer {
                    id: id.clone(),
                    layer_type: "shape".into(),
                    comp: None,
                    name: Some("Path".into()),
                    from: None,
                    duration: None,
                    content,
                    properties,
                    color: None,
                    index: None,
                    parent: None,
                    blend: None,
                    motion_blur: None,
                    visible: None,
                    shy: None,
                    collapsed: None,
                    time,
                };
                commands.push(LayerCommand::Add(Box::new(add)).into());
                self.pen_active = None;
                select = Some(id.clone());
                (id, "shape".to_owned(), [1., 0., 0., 1., 0., 0.])
            }
        };
        let open = pose
            .as_ref()
            .filter(|p| p.layer == layer)
            .and_then(|p| {
                p.targets.iter().find(|t| {
                    self.pen_active.as_ref().map(|(_, p)| p.as_str()) == Some(t.path.as_str())
                        && !t.closed
                })
            })
            .cloned();
        let (path, prefix, matrix) = match open {
            Some(t) => (t.path, t.prefix, t.matrix),
            None => {
                let path = new_id("path");
                let mask = (kind != "shape").then(|| new_id("K"));
                commands.push(
                    MaskCommand::AddPath {
                        layer_id: layer.clone(),
                        path_id: path.clone(),
                        mask_id: mask.clone(),
                    }
                    .into(),
                );
                let prefix = match &mask {
                    Some(m) => format!("mp.{m}"),
                    None => format!("g.{path}"),
                };
                (path, prefix, world)
            }
        };
        let Some(p) = inverse_point(&matrix, at) else {
            return;
        };
        let vertex = new_id("v");
        commands.push(
            MaskCommand::AddVertex {
                layer_id: layer.clone(),
                path_id: path.clone(),
                vertex_id: vertex.clone(),
                x: (p[0] * 1000.).round() / 1000.,
                y: (p[1] * 1000.).round() / 1000.,
            }
            .into(),
        );
        self.pen_active = Some((layer.clone(), path.clone()));
        self.pen_selected = Some((path, vertex.clone()));
        self.queue.push_back(Job::Begin("Draw path vertex"));
        self.queue.push_back(Job::Step(commands, false));
        self.pen_drag = Some(PenDrag {
            origin: event.position,
            layer,
            prefix,
            vertex,
            matrix,
            start: p,
            values: Vertex {
                x: p[0],
                y: p[1],
                ..Default::default()
            },
            side: Side::New,
            time,
            moved: false,
            steps: 0,
            select,
        });
        self.pump(cx);
    }
    pub(super) fn pen_move(
        &mut self,
        event: &MouseMoveEvent,
        at: [f64; 2],
        cx: &mut Context<Self>,
    ) -> bool {
        let threshold = self.tokens.metrics.drag_threshold;
        let Some(d) = self.pen_drag.as_mut() else {
            if self.pen.is_some() {
                self.pen_pointer = Some(at);
                cx.notify();
            }
            return false;
        };
        let moved = event.position - d.origin;
        if !d.moved && f32::from(moved.x).hypot(f32::from(moved.y)) < threshold {
            return true;
        }
        d.moved = true;
        let commands = drag_commands(d, at, event.modifiers.alt);
        let replace = d.steps > 0;
        d.steps += 1;
        self.queue.push_back(Job::Step(commands, replace));
        self.pump(cx);
        true
    }
    pub(super) fn pen_up(&mut self, commit: bool, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.pen_drag.take() else {
            return false;
        };
        self.queue.push_back(Job::Finish(commit));
        if let Some(id) = drag.select.filter(|_| commit) {
            self.queue.push_back(Job::Select(Some(id)));
        }
        self.pump(cx);
        true
    }
    /// Paths, vertices, the selected vertex's tangents and the rubber band.
    pub(super) fn pen_paint(&self, frame: super::ViewFrame, window: &mut Window) {
        let Some(pose) = &self.pen else { return };
        let t = &self.tokens;
        let to_window = |m: &[f64; 6], p: [f64; 2]| {
            let c = transform(m, p);
            point(
                frame.origin.x + px((c[0] * frame.scale) as f32),
                frame.origin.y + px((c[1] * frame.scale) as f32),
            )
        };
        let accent = rgb(t.colors.accent);
        let floating = rgb(t.colors.floating);
        let secondary = rgb(t.colors.secondary);
        let side = t.metrics.gizmo_handle;
        let square = |c: Point<Pixels>, fill: Rgba, edge: Rgba, half: f32, window: &mut Window| {
            let b = Bounds::new(
                c - point(px(half), px(half)),
                size(px(half * 2.), px(half * 2.)),
            );
            window.paint_quad(fill_box(b, px(0.), fill, edge));
        };
        let circle = |c: Point<Pixels>, d: f32, window: &mut Window| {
            let b = Bounds::new(c - point(px(d / 2.), px(d / 2.)), size(px(d), px(d)));
            window.paint_quad(fill_box(b, px(d / 2.), floating, secondary));
        };
        for target in &pose.targets {
            let m = &target.matrix;
            let v = &target.vertices;
            if v.len() >= 2 {
                let mut path = PathBuilder::stroke(px(1.));
                path.move_to(to_window(m, [v[0].x, v[0].y]));
                let segments = if target.closed { v.len() } else { v.len() - 1 };
                for i in 0..segments {
                    let (a, b) = (&v[i], &v[(i + 1) % v.len()]);
                    path.cubic_bezier_to(
                        to_window(m, [b.x, b.y]),
                        to_window(m, [a.x + a.out_x, a.y + a.out_y]),
                        to_window(m, [b.x + b.in_x, b.y + b.in_y]),
                    );
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, accent);
                }
            }
            let selected = self
                .pen_selected
                .as_ref()
                .filter(|(p, _)| *p == target.path)
                .and_then(|(_, id)| v.iter().position(|x| &x.id == id));
            for (i, vertex) in v.iter().enumerate() {
                let near = selected.is_some_and(|s| {
                    s == i || (s + 1) % v.len() == i || (s + v.len() - 1) % v.len() == i
                });
                let at = to_window(m, [vertex.x, vertex.y]);
                if near {
                    for (hx, hy) in [(vertex.in_x, vertex.in_y), (vertex.out_x, vertex.out_y)] {
                        if hx == 0. && hy == 0. {
                            continue;
                        }
                        let end = to_window(m, [vertex.x + hx, vertex.y + hy]);
                        let mut line = PathBuilder::stroke(px(1.));
                        line.move_to(at);
                        line.line_to(end);
                        if let Ok(line) = line.build() {
                            window.paint_path(line, secondary);
                        }
                        circle(end, t.metrics.keyframe - 3., window);
                    }
                }
                let (fill, edge) = if selected == Some(i) {
                    (accent, rgb(t.colors.selected_text))
                } else {
                    (floating, accent)
                };
                square(at, fill, edge, side / 2., window);
            }
            // Drawing: the rubber band to the pointer, and a ring when the
            // first vertex would close the path.
            if self.pen_active.as_ref().map(|(_, p)| p.as_str()) == Some(target.path.as_str())
                && !target.closed
                && self.pen_drag.is_none()
                && let (Some(last), Some(pointer)) = (v.last(), self.pen_pointer)
            {
                let mut band = PathBuilder::stroke(px(1.)).dash_array(&[px(4.), px(3.)]);
                band.move_to(to_window(m, [last.x, last.y]));
                band.line_to(point(
                    frame.origin.x + px((pointer[0] * frame.scale) as f32),
                    frame.origin.y + px((pointer[1] * frame.scale) as f32),
                ));
                if let Ok(band) = band.build() {
                    window.paint_path(band, accent);
                }
                if v.len() >= 3
                    && let Some(local) = inverse_point(m, pointer)
                {
                    let linear = m[0].hypot(m[1]).max(0.01);
                    let first = &v[0];
                    if (local[0] - first.x).hypot(local[1] - first.y) < REACH / frame.scale / linear
                    {
                        let at = to_window(m, [first.x, first.y]);
                        let d = t.metrics.keyframe;
                        let b = Bounds::new(at - point(px(d / 2.), px(d / 2.)), size(px(d), px(d)));
                        window.paint_quad(fill_box(b, px(d / 2.), transparent_black(), accent));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PenDrag, PenPose, Side, Target, Vertex, drag_commands, hit};
    fn pose() -> PenPose {
        let vertex = |id: &str, x: f64| Vertex {
            id: id.into(),
            x,
            y: 10.,
            out_x: 20.,
            ..Default::default()
        };
        PenPose {
            layer: "l".into(),
            kind: "shape".into(),
            locked: false,
            world: [1., 0., 0., 1., 0., 0.],
            targets: vec![Target {
                prefix: "g.p".into(),
                path: "p".into(),
                matrix: [1., 0., 0., 1., 100., 0.],
                closed: false,
                vertices: vec![vertex("a", 0.), vertex("b", 100.)],
            }],
        }
    }
    #[test]
    fn hits_find_vertices_and_tangents_through_the_path_matrix() {
        let pose = pose();
        let h = hit(&pose, [200., 10.], 1.).unwrap();
        assert_eq!((h.index, h.side), (1, Side::Point));
        let h = hit(&pose, [120., 10.], 1.).unwrap();
        assert_eq!((h.index, h.side), (0, Side::Out));
        assert!(hit(&pose, [150., 80.], 1.).is_none());
        // Zoomed out, the same display reach covers more of the composition.
        assert!(hit(&pose, [205., 10.], 0.5).is_some());
    }
    #[test]
    fn tangent_drags_mirror_unless_option_is_held() {
        let d = PenDrag {
            origin: Default::default(),
            layer: "l".into(),
            prefix: "g.p".into(),
            vertex: "a".into(),
            matrix: [1., 0., 0., 1., 0., 0.],
            start: [0., 0.],
            values: Vertex::default(),
            side: Side::New,
            time: 0.,
            moved: true,
            steps: 0,
            select: None,
        };
        assert_eq!(drag_commands(&d, [5., 6.], false).len(), 4);
        assert_eq!(drag_commands(&d, [5., 6.], true).len(), 2);
        let point = PenDrag {
            side: Side::Point,
            ..d
        };
        assert_eq!(drag_commands(&point, [5., 6.], false).len(), 2);
    }
}
