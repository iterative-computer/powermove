//! Viewer transform gizmos. The engine draws them in the frame's own pass, so
//! they register with the picture at any zoom; this element owns hit testing and
//! runs each drag as one kernel gesture on a worker, never through V8.
use gpui::{prelude::*, *};
use pm_engine_service::OverlayLine;
use pm_kernel::{
    Ack, Call, Caller, Command, GestureCall, KernelHandle, PropertyCommand, QueryMirror, Reply,
    Request, SelectionCall, WriteBody, commands::properties::SetProperty,
};
use pm_model::Value;
use std::{collections::VecDeque, rc::Rc, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Rotate,
    Anchor,
    Other,
}
/// The composition's placement inside the viewer, in window points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFrame {
    pub origin: Point<Pixels>,
    pub scale: f64,
    pub size: [f64; 2],
}
pub trait Host: 'static {
    fn time(&self, cx: &App) -> f64;
    fn playing(&self, cx: &App) -> bool;
    fn frame(&self, bounds: Bounds<Pixels>, cx: &App) -> Option<ViewFrame>;
    fn tool(&self, cx: &App) -> Tool;
    /// Publish engine-drawn lines, normalized to the composition frame.
    fn overlay(&self, lines: Vec<OverlayLine>, cx: &mut App);
    fn report(&self, message: String, cx: &mut App);
}
/// One selected layer's pose at the current time, in composition pixels.
#[derive(Clone, Debug)]
pub struct Pose {
    pub id: String,
    pub locked: bool,
    /// Images and video keep their picture's aspect, as in the shipped viewer.
    keeps_aspect: bool,
    world: [f64; 6],
    /// Rotation, skew and scale of the layer alone.
    linear: [f64; 4],
    /// Maps composition deltas into the layer's parent space.
    to_parent: [f64; 4],
    /// Layer-space box: x0, y0, x1, y1.
    bounds: [f64; 4],
    pub corners: [[f64; 2]; 4],
    /// Where the anchor lands in the composition.
    pub anchor: [f64; 2],
    pub local_anchor: [f64; 2],
    pub position: [f64; 2],
    pub scale: [f64; 2],
    pub rotation: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Body,
    Handle(usize),
    Rotate,
    Anchor,
}
struct Drag {
    hit: Hit,
    origin: Point<Pixels>,
    active: bool,
    poses: Arc<Vec<Pose>>,
    time: f64,
    /// A press on the selection that never moves is a click: it picks.
    click: Option<Press>,
}
#[derive(Clone, Copy)]
struct Press {
    at: [f64; 2],
    shift: bool,
    deep: bool,
    time: f64,
}
enum Job {
    Begin(&'static str),
    Step(Vec<Command>),
    Finish(bool),
}
pub struct GizmoSurface {
    kernel: KernelHandle,
    project: String,
    tokens: pm_design::Tokens,
    host: Rc<dyn Host>,
    poses: Arc<Vec<Pose>>,
    bounds: Bounds<Pixels>,
    drag: Option<Drag>,
    queue: VecDeque<Job>,
    gesture: Option<u64>,
    busy: bool,
    latest: u64,
    /// Press waiting on a pick; the drag starts on its answer if still held.
    picking: Option<Point<Pixels>>,
    picks: usize,
}
const HANDLES: [(f64, f64); 8] = [
    (0., 0.),
    (0.5, 0.),
    (1., 0.),
    (1., 0.5),
    (1., 1.),
    (0.5, 1.),
    (0., 1.),
    (0., 0.5),
];
/// Stem above top-centre, points (viewer-chrome.md §Gizmos).
const STEM: f64 = 18.;
/// The Anchor tool grabs only near the anchor (viewer.ts startAnchorTool).
const ANCHOR_REACH: f64 = 12.;
/// Smallest scale a handle drag reaches, percent (viewer.ts calculateResize).
const MIN_SCALE: f64 = 0.1;
fn transform(m: &[f64; 6], p: [f64; 2]) -> [f64; 2] {
    [
        m[0] * p[0] + m[2] * p[1] + m[4],
        m[1] * p[0] + m[3] * p[1] + m[5],
    ]
}
fn inverse_linear(m: [f64; 4]) -> Option<[f64; 4]> {
    let det = m[0] * m[3] - m[1] * m[2];
    (det.abs() > 1e-12).then(|| [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det])
}
fn inverse_point(m: &[f64; 6], p: [f64; 2]) -> Option<[f64; 2]> {
    let inverse = inverse_linear([m[0], m[1], m[2], m[3]])?;
    Some(apply(inverse, [p[0] - m[4], p[1] - m[5]]))
}
fn round(value: f64, digits: i32) -> f64 {
    let unit = 10f64.powi(digits);
    (value * unit).round() / unit
}
fn apply(m: [f64; 4], v: [f64; 2]) -> [f64; 2] {
    [m[0] * v[0] + m[2] * v[1], m[1] * v[0] + m[3] * v[1]]
}
fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
impl Pose {
    fn local(&self, u: f64, v: f64) -> [f64; 2] {
        let b = self.bounds;
        [b[0] + (b[2] - b[0]) * u, b[1] + (b[3] - b[1]) * v]
    }
    fn handle(&self, index: usize) -> [f64; 2] {
        let (u, v) = HANDLES[index];
        let top = lerp(self.corners[0], self.corners[1], u);
        let bottom = lerp(self.corners[3], self.corners[2], u);
        lerp(top, bottom, v)
    }
    /// Above the top-centre handle, along the layer's own up axis.
    fn rotate_handle(&self, scale: f64) -> [f64; 2] {
        let top = self.handle(1);
        let centre = lerp(self.handle(1), self.handle(5), 0.5);
        let (dx, dy) = (top[0] - centre[0], top[1] - centre[1]);
        let length = dx.hypot(dy).max(1e-6);
        let stem = STEM / scale;
        [top[0] + dx / length * stem, top[1] + dy / length * stem]
    }
}
fn read_at(p: &pm_kernel::Published, time: f64) -> Option<Vec<Pose>> {
    let root = p.snapshot.root()?;
    let layers: std::collections::HashMap<String, (bool, String)> =
        pm_kernel::model::list_layers(root, None)
            .into_iter()
            .map(|l| (l.id, (l.effectively_locked, l.kind)))
            .collect();
    let mut mirror = QueryMirror::new(p);
    let mut poses = vec![];
    for id in p.selection.layers() {
        let q = mirror.queries();
        let (Ok(world), Ok(local), Ok(Some(bounds))) = (
            q.world_matrix(None, id, time),
            q.local_matrix(None, id, time),
            q.bounds(None, id, time),
        ) else {
            continue;
        };
        // A layer collapsed to nothing has no box to grab; the others keep theirs.
        let Some(inverse_local) = inverse_linear([local[0], local[1], local[2], local[3]]) else {
            continue;
        };
        // parent = world * local⁻¹; deltas go through its inverse.
        let world_linear = [world[0], world[1], world[2], world[3]];
        let parent = [
            world_linear[0] * inverse_local[0] + world_linear[2] * inverse_local[1],
            world_linear[1] * inverse_local[0] + world_linear[3] * inverse_local[1],
            world_linear[0] * inverse_local[2] + world_linear[2] * inverse_local[3],
            world_linear[1] * inverse_local[2] + world_linear[3] * inverse_local[3],
        ];
        let mut number = |channel: &str| {
            mirror
                .queries()
                .evaluate(None, id, channel, time)
                .ok()
                .and_then(|v| v.as_f64())
        };
        let Some(to_parent) = inverse_linear(parent) else {
            continue;
        };
        let (Some(px), Some(py), Some(sx), Some(sy)) = (
            number("position.x"),
            number("position.y"),
            number("scale.x"),
            number("scale.y"),
        ) else {
            continue;
        };
        let local_anchor = [
            number("anchor.x").unwrap_or(0.),
            number("anchor.y").unwrap_or(0.),
        ];
        let (locked, kind) = layers.get(id).map_or((true, ""), |(l, k)| (*l, k.as_str()));
        poses.push(Pose {
            id: id.clone(),
            locked,
            keeps_aspect: matches!(kind, "image" | "video"),
            world,
            linear: [local[0], local[1], local[2], local[3]],
            to_parent,
            bounds: [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
            corners: [
                transform(&world, [bounds.x0, bounds.y0]),
                transform(&world, [bounds.x1, bounds.y0]),
                transform(&world, [bounds.x1, bounds.y1]),
                transform(&world, [bounds.x0, bounds.y1]),
            ],
            anchor: transform(&world, local_anchor),
            local_anchor,
            position: [px, py],
            scale: [sx, sy],
            rotation: number("rotation").unwrap_or(0.),
        });
    }
    Some(poses)
}
impl GizmoSurface {
    pub fn new(
        kernel: KernelHandle,
        project: String,
        tokens: pm_design::Tokens,
        host: Rc<dyn Host>,
    ) -> Self {
        Self {
            kernel,
            project,
            tokens,
            host,
            poses: Default::default(),
            bounds: Bounds::default(),
            drag: None,
            queue: VecDeque::new(),
            gesture: None,
            busy: false,
            latest: 0,
            picking: None,
            picks: 0,
        }
    }
    /// Selection, document or playhead moved: reread the pose off the main
    /// thread. While playing the engine owns the frame and gizmos stay down.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.host.playing(cx) {
            self.poses = Default::default();
            self.host.overlay(vec![], cx);
            return;
        }
        self.latest += 1;
        let ticket = self.latest;
        let (kernel, project, time) = (
            self.kernel.clone(),
            self.project.clone(),
            self.host.time(cx),
        );
        cx.spawn(async move |this, cx| {
            let poses = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    read_at(&p, time)
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.latest == ticket {
                    s.poses = Arc::new(poses.unwrap_or_default());
                    s.draw(cx);
                }
            });
        })
        .detach();
    }
    fn draw(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return;
        };
        self.host
            .overlay(overlay(&self.poses, frame, &self.tokens), cx);
    }
    fn to_comp(&self, position: Point<Pixels>, frame: ViewFrame) -> [f64; 2] {
        let at = position - frame.origin;
        [
            f32::from(at.x) as f64 / frame.scale,
            f32::from(at.y) as f64 / frame.scale,
        ]
    }
    /// A selection holding a locked layer owns its geometry but does not move.
    fn transformable(&self) -> bool {
        !self.poses.is_empty() && self.poses.iter().all(|p| !p.locked)
    }
    fn handle_at(&self, at: [f64; 2], scale: f64) -> Option<Hit> {
        let [pose] = self.poses.as_slice() else {
            return None;
        };
        if pose.locked {
            return None;
        }
        let distance = |p: [f64; 2]| (p[0] - at[0]).hypot(p[1] - at[1]);
        let reach = self.tokens.metrics.hit as f64 / 2. / scale;
        if distance(pose.rotate_handle(scale)) <= reach {
            return Some(Hit::Rotate);
        }
        // Small boxes overlap: the handle really under the pointer wins.
        (0..8)
            .map(|i| (i, distance(pose.handle(i))))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| Hit::Handle(i))
    }
    fn on_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return;
        };
        let at = self.to_comp(event.position, frame);
        let time = self.host.time(cx);
        let start = |hit, click| Drag {
            hit,
            origin: event.position,
            active: false,
            poses: self.poses.clone(),
            time,
            click,
        };
        let deep = event.modifiers.platform || event.modifiers.control;
        let press = Press {
            at,
            shift: event.modifiers.shift,
            deep,
            time,
        };
        let drag = match self.host.tool(cx) {
            Tool::Other => return,
            Tool::Rotate => {
                (self.poses.len() == 1 && self.transformable()).then(|| start(Hit::Rotate, None))
            }
            Tool::Anchor => match self.poses.as_slice() {
                [pose]
                    if !pose.locked
                        && (pose.anchor[0] - at[0]).hypot(pose.anchor[1] - at[1])
                            <= ANCHOR_REACH / frame.scale.max(0.02) =>
                {
                    Some(start(Hit::Anchor, None))
                }
                _ => None,
            },
            Tool::Select => {
                let inside_selection = self.poses.iter().any(|p| inside(&p.corners, at));
                if let Some(hit) = self.handle_at(at, frame.scale) {
                    Some(start(hit, None))
                } else if inside_selection && self.transformable() {
                    Some(start(Hit::Body, Some(press)))
                } else if inside_selection {
                    None
                } else {
                    self.picking = Some(event.position);
                    self.pick(press, cx);
                    None
                }
            }
        };
        self.drag = drag;
        cx.stop_propagation();
    }
    /// Asks the kernel what is under the pointer and applies the shipped click
    /// rules, off the main thread. The answer arrives while the press may be held.
    fn pick(&mut self, press: Press, cx: &mut Context<Self>) {
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        self.picks += 1;
        cx.spawn(async move |this, cx| {
            let poses = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    let target = pick_target(&p, press)?;
                    let selected = p.selection.layers();
                    let ids = match &target {
                        Some(id) if press.shift && selected.contains(id) => {
                            selected.iter().filter(|s| *s != id).cloned().collect()
                        }
                        Some(id) => vec![id.clone()],
                        None if press.shift => return None,
                        None => vec![],
                    };
                    let add =
                        press.shift && target.is_some() && !selected.contains(target.as_ref()?);
                    let selection = SelectionCall::Layers { ids, add };
                    let request = Request::new(Caller::native(), selection);
                    kernel.call(&project, request).await?.ok()?;
                    let p = kernel.published(&project)?;
                    Some((read_at(&p, press.time)?, target.filter(|_| !press.shift)))
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.picks -= 1;
                let held = s.picking.take();
                let Some((poses, target)) = poses else {
                    return;
                };
                s.poses = Arc::new(poses);
                s.draw(cx);
                if let Some(origin) = held.filter(|_| target.is_some() && s.transformable()) {
                    s.drag = Some(Drag {
                        hit: Hit::Body,
                        origin,
                        active: false,
                        poses: s.poses.clone(),
                        time: press.time,
                        click: None,
                    });
                }
            });
        })
        .detach();
    }
    fn on_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let threshold = self.tokens.metrics.drag_threshold;
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return;
        };
        let at = self.to_comp(event.position, frame);
        let origin = self.drag.as_ref().map(|d| self.to_comp(d.origin, frame));
        let Some((drag, origin)) = self.drag.as_mut().zip(origin) else {
            return;
        };
        let moved = event.position - drag.origin;
        if !drag.active {
            if f32::from(moved.x).hypot(f32::from(moved.y)) < threshold {
                return;
            }
            drag.active = true;
            self.queue.push_back(Job::Begin(match drag.hit {
                Hit::Body => "Move",
                Hit::Handle(_) => "Scale",
                Hit::Rotate => "Rotate",
                Hit::Anchor => "Move Anchor Point",
            }));
        }
        let mods = Mods {
            shift: event.modifiers.shift,
            alt: event.modifiers.alt,
            snap: event.modifiers.platform || event.modifiers.control,
        };
        let commands = commands(drag, origin, at, mods);
        self.queue.push_back(Job::Step(commands));
        self.pump(cx);
    }
    fn on_up(&mut self, commit: bool, cx: &mut Context<Self>) {
        self.picking = None;
        let Some(drag) = self.drag.take() else {
            return;
        };
        if drag.active {
            self.queue.push_back(Job::Finish(commit));
            self.pump(cx);
        } else if let Some(press) = drag.click.filter(|_| commit) {
            self.pick(press, cx);
        }
    }
    pub fn cancel(&mut self, cx: &mut Context<Self>) -> bool {
        if self.drag.is_none() {
            return false;
        }
        self.on_up(false, cx);
        true
    }
    pub fn gesture_active(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.active)
    }
    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(mut job) = self.queue.pop_front() else {
            return;
        };
        while matches!(job, Job::Step(_)) && matches!(self.queue.front(), Some(Job::Step(_))) {
            job = self.queue.pop_front().unwrap();
        }
        self.busy = true;
        let (kernel, project, gesture) = (self.kernel.clone(), self.project.clone(), self.gesture);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let call: Call = match (job, gesture) {
                        (Job::Begin(label), _) => GestureCall::Begin {
                            label: label.into(),
                        }
                        .into(),
                        (Job::Step(commands), Some(gesture)) => GestureCall::Step {
                            gesture,
                            body: WriteBody::Commands(commands),
                            replace: true,
                        }
                        .into(),
                        (Job::Finish(true), Some(gesture)) => {
                            GestureCall::Commit { gesture }.into()
                        }
                        (Job::Finish(false), Some(gesture)) => {
                            GestureCall::Cancel { gesture }.into()
                        }
                        _ => return Err::<Ack, String>("Gesture was refused".into()),
                    };
                    kernel
                        .call(&project, Request::new(Caller::native(), call))
                        .await
                        .ok_or("Kernel closed")?
                        .map_err(|e| format!("{e:?}"))
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                match result {
                    Ok(ack) => match ack.reply {
                        Reply::Gesture(id) => s.gesture = Some(id),
                        Reply::Throw(error) => s.host.report(error.to_string(), cx),
                        _ => {}
                    },
                    Err(error) => s.host.report(error, cx),
                }
                if s.queue.is_empty() && s.drag.is_none() {
                    s.gesture = None;
                }
                s.pump(cx);
            });
        })
        .detach();
    }
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        !self.busy && self.queue.is_empty() && self.picks == 0
    }
    #[cfg(test)]
    pub fn poses(&self) -> Arc<Vec<Pose>> {
        self.poses.clone()
    }
    #[cfg(test)]
    pub fn points(&self) -> Option<([f64; 2], [f64; 2], [f64; 2])> {
        let pose = self.poses.first()?;
        Some((
            lerp(pose.corners[0], pose.corners[2], 0.5),
            pose.handle(4),
            pose.rotate_handle(1.),
        ))
    }
}
fn inside(corners: &[[f64; 2]; 4], p: [f64; 2]) -> bool {
    let mut sign = 0.;
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross != 0. {
            if sign != 0. && cross * sign < 0. {
                return false;
            }
            sign = cross;
        }
    }
    true
}
fn set(id: &str, channel: &str, value: f64, time: f64) -> Command {
    PropertyCommand::Set(SetProperty::new(id, channel, Value::Number(value), time)).into()
}
#[derive(Clone, Copy)]
struct Mods {
    shift: bool,
    alt: bool,
    /// Command or Control.
    snap: bool,
}
fn commands(drag: &Drag, origin: [f64; 2], at: [f64; 2], mods: Mods) -> Vec<Command> {
    let mut delta = [at[0] - origin[0], at[1] - origin[1]];
    if mods.shift && matches!(drag.hit, Hit::Body) {
        // Shift locks the dominant axis.
        let axis = usize::from(delta[0].abs() > delta[1].abs());
        delta[axis] = 0.;
    }
    let t = drag.time;
    let mut out = vec![];
    for pose in drag.poses.iter().filter(|p| !p.locked) {
        let id = &pose.id;
        match drag.hit {
            Hit::Body => {
                let d = apply(pose.to_parent, delta);
                out.push(set(id, "position.x", pose.position[0] + d[0], t));
                out.push(set(id, "position.y", pose.position[1] + d[1], t));
            }
            Hit::Rotate => {
                let angle = |p: [f64; 2]| {
                    let v = apply(
                        pose.to_parent,
                        [p[0] - pose.anchor[0], p[1] - pose.anchor[1]],
                    );
                    v[1].atan2(v[0])
                };
                let turn = (angle(at) - angle(origin)).to_degrees();
                let mut degrees = pose.rotation + (turn + 180.).rem_euclid(360.) - 180.;
                if mods.snap {
                    degrees = (degrees / 45.).round() * 45.;
                } else if mods.shift {
                    degrees = (degrees / 15.).round() * 15.;
                }
                out.push(set(id, "rotation", round(degrees, 2), t));
            }
            Hit::Anchor => {
                let Some(to_local) = inverse_linear(pose.linear) else {
                    continue;
                };
                let moved = apply(pose.to_parent, delta);
                let mut anchor = apply(to_local, moved);
                anchor = [
                    pose.local_anchor[0] + anchor[0],
                    pose.local_anchor[1] + anchor[1],
                ];
                if mods.snap {
                    // Command lands on the box's corners, edges and centre.
                    let nearest = |value: f64, low: f64, high: f64| {
                        [low, (low + high) / 2., high]
                            .into_iter()
                            .min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs()))
                            .unwrap_or(value)
                    };
                    let b = pose.bounds;
                    anchor = [
                        nearest(anchor[0], b[0], b[2]),
                        nearest(anchor[1], b[1], b[3]),
                    ];
                }
                out.push(set(id, "anchor.x", round(anchor[0], 3), t));
                out.push(set(id, "anchor.y", round(anchor[1], 3), t));
                if !mods.alt {
                    // Position compensates so the picture stays put; Option lets it go.
                    let shift = apply(
                        pose.linear,
                        [
                            anchor[0] - pose.local_anchor[0],
                            anchor[1] - pose.local_anchor[1],
                        ],
                    );
                    out.push(set(
                        id,
                        "position.x",
                        round(pose.position[0] + shift[0], 3),
                        t,
                    ));
                    out.push(set(
                        id,
                        "position.y",
                        round(pose.position[1] + shift[1], 3),
                        t,
                    ));
                }
            }
            Hit::Handle(index) => {
                out.extend(resize(pose, index, origin, at, mods, t));
            }
        }
    }
    out
}
/// Port of `calculateResize`: the opposite handle stays fixed, Option scales
/// from the centre and Shift keeps the aspect. Scale is the rightmost term of
/// the layer's matrix, so the position correction has a closed form.
fn resize(
    pose: &Pose,
    index: usize,
    origin: [f64; 2],
    at: [f64; 2],
    mods: Mods,
    t: f64,
) -> Vec<Command> {
    let (Some(start), Some(now)) = (
        inverse_point(&pose.world, origin),
        inverse_point(&pose.world, at),
    ) else {
        return vec![];
    };
    let (u, v) = HANDLES[index];
    let (active_x, active_y) = (u != 0.5, v != 0.5);
    let handle = pose.local(u, v);
    let pivot = if mods.alt {
        pose.local(0.5, 0.5)
    } else {
        pose.local(
            if active_x { 1. - u } else { 0.5 },
            if active_y { 1. - v } else { 0.5 },
        )
    };
    let grabbed = [
        now[0] - (start[0] - handle[0]),
        now[1] - (start[1] - handle[1]),
    ];
    let (dx, dy) = (handle[0] - pivot[0], handle[1] - pivot[1]);
    let factor = |active: bool, span: f64, grabbed: f64, pivot: f64| {
        if active && span.abs() > 1e-9 {
            (grabbed - pivot) / span
        } else {
            1.
        }
    };
    let mut fx = factor(active_x, dx, grabbed[0], pivot[0]);
    let mut fy = factor(active_y, dy, grabbed[1], pivot[1]);
    if mods.shift || pose.keeps_aspect {
        let uniform = if active_x && active_y {
            // Project onto the corner's diagonal: continuous through shrink and growth.
            ((grabbed[0] - pivot[0]) * dx + (grabbed[1] - pivot[1]) * dy)
                / (dx * dx + dy * dy).max(1e-9)
        } else if active_x {
            fx
        } else {
            fy
        };
        fx = uniform;
        fy = uniform;
    }
    let scale_x = (pose.scale[0] * fx).max(MIN_SCALE);
    let scale_y = (pose.scale[1] * fy).max(MIN_SCALE);
    let (kept_x, kept_y) = (scale_x / pose.scale[0], scale_y / pose.scale[1]);
    let fixed = [
        pivot[0] - pose.local_anchor[0],
        pivot[1] - pose.local_anchor[1],
    ];
    let correction = apply(
        pose.linear,
        [fixed[0] * (1. - kept_x), fixed[1] * (1. - kept_y)],
    );
    vec![
        set(&pose.id, "scale.x", round(scale_x, 3), t),
        set(&pose.id, "scale.y", round(scale_y, 3), t),
        set(
            &pose.id,
            "position.x",
            round(pose.position[0] + correction[0], 3),
            t,
        ),
        set(
            &pose.id,
            "position.y",
            round(pose.position[1] + correction[1], 3),
            t,
        ),
    ]
}
/// The top layer under a press. Groups the selection has not entered answer
/// for their members; Command reaches the leaf (viewer.ts resolvePickTarget).
fn pick_target(p: &pm_kernel::Published, press: Press) -> Option<Option<String>> {
    let root = p.snapshot.root()?;
    let hit = QueryMirror::new(p)
        .queries()
        .pick(
            None,
            press.at[0],
            press.at[1],
            press.time,
            &Default::default(),
        )
        .ok()?;
    let Some(id) = hit.layer_id else {
        return Some(None);
    };
    if press.deep {
        return Some(Some(id));
    }
    let layers = pm_kernel::model::list_layers(root, None);
    let group_of = |id: &str| {
        layers
            .iter()
            .find(|l| l.id == id)
            .and_then(|l| l.group.clone())
    };
    let chain = |from: &str| {
        let mut out = vec![];
        let mut at = group_of(from);
        while let Some(group) = at {
            at = group_of(&group);
            out.push(group);
        }
        out
    };
    let entered: Vec<String> = p.selection.layers().iter().flat_map(|l| chain(l)).collect();
    let outermost = chain(&id).into_iter().rev().find(|g| !entered.contains(g));
    Some(Some(outermost.unwrap_or(id)))
}
/// Accent box, handles and anchor as engine lines. Sizes follow the display,
/// never the composition zoom.
fn overlay(poses: &[Pose], frame: ViewFrame, t: &pm_design::Tokens) -> Vec<OverlayLine> {
    let accent = t.colors.accent;
    let channel = |shift: u32| ((accent >> shift) & 255) as f32 / 255.;
    let color = [channel(16), channel(8), channel(0), 1.];
    let [w, h] = frame.size;
    let n = |p: [f64; 2]| [(p[0] / w) as f32, (p[1] / h) as f32];
    let mut lines = vec![];
    let mut line = |a: [f64; 2], b: [f64; 2]| {
        let (a, b) = (n(a), n(b));
        lines.push(OverlayLine {
            ends: [a[0], a[1], b[0], b[1]],
            color,
        });
    };
    let half = (t.metrics.keyframe as f64 - 2.) / 2. / frame.scale;
    let square = |p: [f64; 2], line: &mut dyn FnMut([f64; 2], [f64; 2])| {
        let c = [
            [p[0] - half, p[1] - half],
            [p[0] + half, p[1] - half],
            [p[0] + half, p[1] + half],
            [p[0] - half, p[1] + half],
        ];
        for i in 0..4 {
            line(c[i], c[(i + 1) % 4]);
        }
    };
    for pose in poses {
        for i in 0..4 {
            line(pose.corners[i], pose.corners[(i + 1) % 4]);
        }
    }
    if let [pose] = poses
        && !pose.locked
    {
        for i in 0..8 {
            square(pose.handle(i), &mut line);
        }
        line(pose.handle(1), pose.rotate_handle(frame.scale));
        square(pose.rotate_handle(frame.scale), &mut line);
        let a = pose.anchor;
        line([a[0] - half, a[1]], [a[0] + half, a[1]]);
        line([a[0], a[1] - half], [a[0], a[1] + half]);
    }
    lines
}
impl Render for GizmoSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        div()
            .id("viewer-gizmos")
            .size_full()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, event: &MouseDownEvent, _, cx| s.on_down(event, cx)),
            )
            .child(
                canvas(
                    {
                        let this = this.clone();
                        move |bounds, _, cx| {
                            let _ = this.update(cx, |s, _| s.bounds = bounds);
                        }
                    },
                    move |_, _, window, _| {
                        let moved = this.clone();
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = moved.update(cx, |s, cx| s.on_move(e, cx));
                            }
                        });
                        let released = this.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = released.update(cx, |s, cx| s.on_up(true, cx));
                            }
                        });
                    },
                )
                .size_full(),
            )
    }
}
