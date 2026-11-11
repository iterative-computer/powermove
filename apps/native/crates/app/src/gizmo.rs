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
use pm_panel_viewerchrome::{ZOOM_MAX, ZOOM_MIN};
use std::{cell::Cell, collections::VecDeque, rc::Rc, sync::Arc, time::Duration};
#[path = "marquee.rs"]
mod marquee;
#[path = "pen_tool.rs"]
pub mod pen;
#[path = "snap.rs"]
mod snap;
#[path = "gizmo3d.rs"]
pub mod studio3d;
#[path = "text_edit.rs"]
pub mod text;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Hand,
    Zoom,
    Rotate,
    Anchor,
    Shape,
    Pen,
    Text,
    Other,
}
/// Viewer navigation the pointer asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Navigate {
    Pan(Point<Pixels>),
    Zoom { factor: f64, at: Point<Pixels> },
}
/// How long pointer navigation rests before it is written. Every write publishes
/// to all panels, so a hot gesture only moves local state.
pub const NAVIGATION_IDLE: Duration = Duration::from_millis(150);
/// The viewer's zoom and pan. The pointer owns them from its first event until
/// their write lands: publications in between carry older values.
#[derive(Debug)]
pub struct ViewPlacement {
    /// `None` fits the slot.
    pub zoom: Option<f64>,
    /// Offset from centred, points.
    pub pan: [f64; 2],
    /// The composition's scale as shown, which is what pointers map through.
    pub scale: Cell<f64>,
    /// The viewer body the composition fits in, points.
    pub stage: [f64; 2],
    local: bool,
}
impl Default for ViewPlacement {
    fn default() -> Self {
        Self {
            zoom: None,
            pan: [0., 0.],
            scale: Cell::new(1.),
            stage: [640., 360.],
            local: false,
        }
    }
}
impl ViewPlacement {
    pub fn navigate(&mut self, change: Navigate, center: Point<Pixels>, limit: f64) {
        let (zoom, pan) = navigated(change, self.scale.get(), self.pan, center, limit);
        self.zoom = Some(zoom);
        self.pan = pan;
        self.scale.set(zoom);
        self.local = true;
    }
    /// An absolute placement from a command.
    pub fn place(&mut self, zoom: Option<f64>, pan: [f64; 2]) {
        self.zoom = zoom;
        self.pan = pan;
        if let Some(zoom) = zoom {
            self.scale.set(zoom);
        }
        self.local = true;
    }
    /// A publication's values, ignored while local ones are unwritten.
    pub fn adopt(&mut self, navigation: &pm_panel_viewerchrome::Navigation) {
        if !self.local {
            self.zoom = (!navigation.fit).then_some(navigation.zoom);
            self.pan = [navigation.pan.x, navigation.pan.y];
        }
    }
    pub fn written(&mut self) {
        self.local = false;
    }
}
/// The zoom and pan after a change: zooming keeps the point under the pointer.
/// `limit` is the highest zoom the display can show.
pub fn navigated(
    change: Navigate,
    zoom: f64,
    pan: [f64; 2],
    center: Point<Pixels>,
    limit: f64,
) -> (f64, [f64; 2]) {
    match change {
        Navigate::Pan(by) => (
            zoom,
            [
                pan[0] + f32::from(by.x) as f64,
                pan[1] + f32::from(by.y) as f64,
            ],
        ),
        Navigate::Zoom { factor, at } => {
            let next = (zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX.min(limit).max(ZOOM_MIN));
            let kept = 1. - next / zoom;
            let offset = |at: Pixels, center: Pixels, pan: f64| {
                pan + (f32::from(at) as f64 - f32::from(center) as f64 - pan) * kept
            };
            (
                next,
                [
                    offset(at.x, center.x, pan[0]),
                    offset(at.y, center.y, pan[1]),
                ],
            )
        }
    }
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
    fn navigate(&self, change: Navigate, cx: &mut App);
    /// The shared layer menu, on the selection the press made.
    /// The right-click menu over `layer`, or over empty canvas.
    fn layer_menu(&self, layer: Option<String>, window: &mut Window, cx: &mut App);
    fn report(&self, message: String, cx: &mut App);
    /// A drag step is on its way to the kernel; `at` is the oldest pointer
    /// event it carries (host seconds), for the viewer's input-to-photon trace.
    fn input(&self, _at: f64, _: &mut App) {}
    /// The Shape tool's variant.
    fn shape_variant(&self, _: &App) -> String {
        "rect".into()
    }
    /// Editing tools stop playback first.
    fn pause(&self, _: &mut App) {}
    /// A shell command (the tool returning to Select).
    fn command(&self, _: &str, _: &mut App) {}
    /// The native editor an on-canvas text session types into.
    fn text_input(&self, _: &str, _: &mut Window, _: &mut App) -> Option<Rc<dyn text::TextInput>> {
        None
    }
    /// The viewer's Free View for Studio 3D layers; `None` is the shot.
    fn studio_view(&self, _: Option<pm_engine_service::studio::FreeView>, _: &mut App) {}
    /// The engine's surface pick at normalized composition coordinates, on
    /// the frame on screen. `None` when there's no viewer frame to ask.
    fn pick_surface(
        &self,
        _: [f32; 2],
        _: &App,
    ) -> Option<async_channel::Receiver<Result<pm_engine_service::SurfaceReply, String>>> {
        None
    }
}
/// One selected layer's pose at the current time, in composition pixels.
#[derive(Clone, Debug)]
pub struct Pose {
    pub id: String,
    pub locked: bool,
    /// A selected parent or group carries this layer; moving both would double it.
    root: bool,
    /// Images and video keep their picture's aspect, as in the shipped viewer.
    keeps_aspect: bool,
    world: [f64; 6],
    /// Rotation, skew and scale of the layer alone.
    linear: [f64; 4],
    /// Maps composition deltas into the layer's parent space.
    to_parent: [f64; 4],
    /// Layer-space box: x0, y0, x1, y1.
    bounds: [f64; 4],
    /// A parametric shape resizes its content, not its scale.
    shape: Option<ShapeSize>,
    /// A text layer's font size: the common box resizes type by it.
    text_size: Option<f64>,
    has_skew: bool,
    pub corners: [[f64; 2]; 4],
    /// Where the anchor lands in the composition.
    pub anchor: [f64; 2],
    pub local_anchor: [f64; 2],
    pub position: [f64; 2],
    pub scale: [f64; 2],
    pub rotation: f64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShapeSize {
    width: f64,
    height: f64,
    stroke: f64,
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
    /// What a move snaps to, once read off the main thread.
    snaps: Option<Arc<Vec<[f64; 2]>>>,
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
    /// `true`: supersedes the previous step (absolute, from-start values).
    Step(Vec<Command>, bool),
    Finish(bool),
    /// A one-step edit outside any gesture, then the layer to select.
    Transact(&'static str, Vec<Command>, Option<String>),
    /// Select one layer, or nothing (not an edit).
    Select(Option<String>),
}
pub struct GizmoSurface {
    studio: studio3d::State,
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
    /// The oldest pointer event behind the queued drag steps (`Host::input`).
    step_input: Option<f64>,
    latest: u64,
    /// Press waiting on a pick; the drag starts on its answer if still held.
    picking: Option<Point<Pixels>>,
    picks: usize,
    controls: bool,
    panning: Option<Point<Pixels>>,
    cursor: CursorStyle,
    refreshing: bool,
    /// A Shape tool drag: where it started and the box drawn so far.
    sketch: Option<Sketch>,
    /// A Text tool press, until its release decides what it makes.
    text_sketch: Option<text::TextSketch>,
    /// The on-canvas text edit.
    text: Option<text::TextSession>,
    /// The Pen tool's view of the selected layer's paths.
    pen: Option<pen::PenPose>,
    pen_reading: u64,
    /// The layer and open path new clicks extend.
    pen_active: Option<(String, String)>,
    /// A pen read has landed since the tool was chosen.
    pen_ready: bool,
    /// A press that came before the read, replayed when it lands.
    pen_waiting: Option<(MouseDownEvent, [f64; 2], f64)>,
    /// Path and vertex whose tangents show.
    pen_selected: Option<(String, String)>,
    pen_drag: Option<pen::PenDrag>,
    pen_pointer: Option<[f64; 2]>,
    /// A Select-tool drag from empty canvas.
    marquee: Option<marquee::Marquee>,
    /// Snapping guides of the move in progress, composition points.
    guides: Vec<snap::Guide>,
}
struct Sketch {
    origin: Point<Pixels>,
    start: [f64; 2],
    shape: Option<crate::shape_tool::ShapeBox>,
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
pub(crate) fn inverse_point(m: &[f64; 6], p: [f64; 2]) -> Option<[f64; 2]> {
    let inverse = inverse_linear([m[0], m[1], m[2], m[3]])?;
    Some(apply(inverse, [p[0] - m[4], p[1] - m[5]]))
}
/// A composition-space step as the layer's parent sees it.
pub fn parent_delta(world: &[f64; 6], local: &[f64; 6], delta: [f64; 2]) -> Option<[f64; 2]> {
    Some(apply(parent_inverse(world, local)?, delta))
}
/// parent = world · local⁻¹; deltas go through its inverse.
fn parent_inverse(world: &[f64; 6], local: &[f64; 6]) -> Option<[f64; 4]> {
    let inverse_local = inverse_linear([local[0], local[1], local[2], local[3]])?;
    let w = [world[0], world[1], world[2], world[3]];
    inverse_linear([
        w[0] * inverse_local[0] + w[2] * inverse_local[1],
        w[1] * inverse_local[0] + w[3] * inverse_local[1],
        w[0] * inverse_local[2] + w[2] * inverse_local[3],
        w[1] * inverse_local[2] + w[3] * inverse_local[3],
    ])
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
    fn rotate_handle(&self, stem: f64) -> [f64; 2] {
        let top = self.handle(1);
        let centre = lerp(self.handle(1), self.handle(5), 0.5);
        let (dx, dy) = (top[0] - centre[0], top[1] - centre[1]);
        let length = dx.hypot(dy).max(1e-6);
        [top[0] + dx / length * stem, top[1] + dy / length * stem]
    }
    /// The resize cursor along a handle's direction from the box centre.
    fn cursor(&self, index: usize) -> CursorStyle {
        let centre = lerp(self.handle(0), self.handle(4), 0.5);
        let at = self.handle(index);
        let angle = (at[1] - centre[1]).atan2(at[0] - centre[0]).to_degrees();
        match ((angle + 22.5).rem_euclid(180.) / 45.) as usize {
            0 => CursorStyle::ResizeLeftRight,
            1 => CursorStyle::ResizeUpLeftDownRight,
            2 => CursorStyle::ResizeUpDown,
            _ => CursorStyle::ResizeUpRightDownLeft,
        }
    }
}
fn read_at(p: &pm_kernel::Published, time: f64) -> Option<Vec<Pose>> {
    let root = p.snapshot.root()?;
    let list = pm_kernel::model::list_layers(root, None);
    let roots = crate::layer_commands::transform_roots(&list, p.selection.layers());
    let layers: std::collections::HashMap<&str, (bool, &str)> = list
        .iter()
        .map(|l| (l.id.as_str(), (l.effectively_locked, l.kind.as_str())))
        .collect();
    let mut mirror = QueryMirror::new(p);
    let mut poses = vec![];
    for id in p.selection.layers() {
        let q = mirror.queries();
        // A selected layer the playhead is outside of has nothing on screen to
        // box (viewer.ts visibleSelectionLayers).
        if !q.active(None, id, time).unwrap_or(false) {
            continue;
        }
        let (Ok(world), Ok(local), Ok(Some(bounds))) = (
            q.world_matrix(None, id, time),
            q.local_matrix(None, id, time),
            // Text measures with the registry's fonts.
            crate::text_host::query(p, |q| q.bounds(None, id, time)),
        ) else {
            continue;
        };
        // A layer collapsed to nothing has no box to grab; the others keep theirs.
        let Some(to_parent) = parent_inverse(&world, &local) else {
            continue;
        };
        let mut number = |channel: &str| {
            mirror
                .queries()
                .evaluate(None, id, channel, time)
                .ok()
                .and_then(|v| v.as_f64())
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
        let (locked, kind) = layers.get(id.as_str()).copied().unwrap_or((true, ""));
        let parametric = kind == "shape"
            && pm_kernel::model::find_layer(root, id)
                .and_then(|at| pm_kernel::model::layer_at(root, &at))
                .and_then(|layer| layer.get("d"))
                .and_then(|d| d.get("paths"))
                .is_none_or(|paths| paths.to_value().as_array().is_none_or(|p| p.is_empty()));
        // Static content reads as stored; animated content is evaluated.
        let stored = |key: &str| {
            let d = pm_kernel::model::find_layer(root, id)
                .and_then(|at| pm_kernel::model::layer_at(root, &at))
                .and_then(|layer| layer.get("d"))?
                .to_value();
            let v = d.get(key)?.clone();
            let animated = v
                .get("kf")
                .and_then(Value::as_array)
                .is_some_and(|k| !k.is_empty());
            if animated {
                return None;
            }
            v.as_f64().or_else(|| v.get("v").and_then(Value::as_f64))
        };
        let mut content = |key: &str| stored(key).or_else(|| number(&format!("c.{key}")));
        let shape = if parametric {
            content("w")
                .zip(content("h"))
                .map(|(width, height)| ShapeSize {
                    width,
                    height,
                    stroke: content("stroke").unwrap_or(0.).max(0.),
                })
        } else {
            None
        };
        poses.push(Pose {
            id: id.clone(),
            locked,
            root: roots.contains(id.as_str()),
            keeps_aspect: matches!(kind, "image" | "video"),
            world,
            linear: [local[0], local[1], local[2], local[3]],
            to_parent,
            bounds: [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
            shape,
            text_size: if kind == "text" {
                content("size")
            } else {
                None
            },
            has_skew: number("skew").is_some(),
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
            studio: Default::default(),
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
            step_input: None,
            latest: 0,
            picking: None,
            picks: 0,
            controls: true,
            panning: None,
            cursor: CursorStyle::Arrow,
            refreshing: false,
            sketch: None,
            text_sketch: None,
            text: None,
            pen: None,
            pen_reading: 0,
            pen_active: None,
            pen_ready: false,
            pen_waiting: None,
            pen_selected: None,
            pen_drag: None,
            pen_pointer: None,
            marquee: None,
            guides: vec![],
        }
    }
    /// Selection, document or playhead moved. The Controller calls in during its
    /// own update and the host reads the Controller, which GPUI refuses while it
    /// is leased, so the work runs once that update ends.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.refreshing, true) {
            return;
        }
        let this = cx.entity();
        cx.defer(move |cx| {
            this.update(cx, |s, cx| {
                s.refreshing = false;
                s.reread(cx);
                s.text_reread(cx);
                s.pen_reread(cx);
            });
        });
    }
    /// Lines depend on the shown scale; repaint them from the current poses.
    pub fn redraw(&mut self, cx: &mut Context<Self>) {
        let this = cx.entity();
        cx.defer(move |cx| this.update(cx, |s, cx| s.draw(cx)));
    }
    /// Reread the pose off the main thread. While playing the engine owns the
    /// frame and gizmos stay down.
    fn reread(&mut self, cx: &mut Context<Self>) {
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
            let read = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    // 3D layers get 3D handles; their 2D boxes would mislead.
                    let scene = pm_panel_studio3d::viewport::scene(&p, time).ok();
                    let three_d: Vec<String> = scene
                        .as_ref()
                        .map(|s| s.items.iter().map(|i| i.id.clone()).collect())
                        .unwrap_or_default();
                    let mut poses = read_at(&p, time).unwrap_or_default();
                    // Extruded text and shapes are drawn by the 3D renderer too:
                    // their flat box would sit somewhere else.
                    poses.retain(|pose| {
                        !three_d.contains(&pose.id)
                            && !is_studio(&p, &pose.id)
                            && !is_extruded(&p, &pose.id)
                    });
                    let selected = p
                        .selection
                        .layers()
                        .iter()
                        .filter(|id| three_d.contains(id))
                        .cloned()
                        .collect::<Vec<_>>();
                    let ui = |k: &str| {
                        p.ui_state
                            .viewer
                            .get(k)
                            .and_then(pm_model::Value::as_str)
                            .map(str::to_owned)
                    };
                    Some((
                        poses,
                        scene,
                        selected,
                        ui("studioTool"),
                        ui("studioLook"),
                        ui("studioAxes"),
                        (ui("studioLightKind"), ui("studioLightAdd")),
                    ))
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.latest == ticket {
                    let (poses, scene, selected, tool, look, axes, (kind, add)) =
                        read.unwrap_or((vec![], None, vec![], None, None, None, (None, None)));
                    s.studio.object_axes = axes.as_deref() == Some("object");
                    s.studio.light_kind = kind.unwrap_or_default();
                    s.studio.light_add = add.as_deref() == Some("on");
                    s.poses = Arc::new(poses);
                    let had = s.studio.scene.is_some();
                    let free_look = look.as_deref() == Some("free");
                    let changed_look = free_look != s.studio.free_look;
                    s.studio.scene = scene.map(Arc::new);
                    s.studio.selected = selected;
                    s.studio.tool = studio3d::Tool::parse(tool.as_deref());
                    if !s.studio.busy() {
                        s.studio.free_look = free_look;
                    }
                    if changed_look || had != s.studio.scene.is_some() {
                        s.studio_push_view(cx);
                    }
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
        let editing = self.text_session().is_some();
        let poses: &[Pose] = if self.controls && !editing {
            &self.poses
        } else {
            &[]
        };
        let mut lines = overlay(poses, frame, &self.tokens);
        let drawn = self.sketch.as_ref().and_then(|s| s.shape);
        if let Some(shape) = drawn.or(self.text_sketch.as_ref().and_then(|s| s.shape)) {
            let [w, h] = frame.size;
            // The Ellipse variant previews its outline; the others their box.
            let c = if drawn.is_some() && self.host.shape_variant(cx) == "ellipse" {
                crate::shape_tool::ellipse_outline(shape, 32)
            } else {
                shape.corners().to_vec()
            };
            for i in 0..c.len() {
                let (a, b) = (c[i], c[(i + 1) % c.len()]);
                lines.push(OverlayLine {
                    ends: [a[0] / w, a[1] / h, b[0] / w, b[1] / h].map(|v| v as f32),
                    color: [1., 1., 1., 1.],
                });
            }
        }
        if self.controls && !editing {
            lines.extend(self.studio_lines(frame));
        }
        lines.truncate(pm_engine_service::OverlayGeometry::MAX_LINES);
        self.host.overlay(lines, cx);
    }
    fn to_comp(&self, position: Point<Pixels>, frame: ViewFrame) -> [f64; 2] {
        let at = position - frame.origin;
        [
            f32::from(at.x) as f64 / frame.scale,
            f32::from(at.y) as f64 / frame.scale,
        ]
    }
    fn stem(&self, scale: f64) -> f64 {
        self.tokens.metrics.rotation_stem as f64 / scale
    }
    /// The cursor promises what a press here does.
    fn hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return;
        };
        let at = self.to_comp(position, frame);
        let tool = self.host.tool(cx);
        let in_text = !matches!(tool, Tool::Hand | Tool::Zoom)
            && self
                .text_session()
                .is_some_and(|s| s.contains(at, frame.scale));
        // A 3D handle under the pointer lights up (3d-studio.md §5).
        if matches!(tool, Tool::Select) && self.studio_hover(at, frame) {
            self.draw(cx);
        }
        let cursor = match tool {
            _ if in_text => CursorStyle::IBeam,
            Tool::Hand => CursorStyle::OpenHand,
            Tool::Text => CursorStyle::IBeam,
            Tool::Anchor | Tool::Zoom | Tool::Shape | Tool::Pen => CursorStyle::Crosshair,
            Tool::Select if self.studio_cursor(at, frame).is_some() => {
                self.studio_cursor(at, frame).unwrap()
            }
            _ => match self.handle_at(at, frame.scale) {
                Some(Hit::Handle(index)) => self
                    .handle_pose()
                    .map_or(CursorStyle::Arrow, |p| p.cursor(index)),
                _ => CursorStyle::Arrow,
            },
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            cx.notify();
        }
    }
    /// A selection holding a locked layer owns its geometry but does not move.
    fn transformable(&self) -> bool {
        !self.poses.is_empty() && self.poses.iter().all(|p| !p.locked)
    }
    /// The box whose handles a press can take: the one layer's, or the
    /// common box of an unlocked multiple selection.
    fn handle_pose(&self) -> Option<Pose> {
        match self.poses.as_slice() {
            [pose] => Some(pose.clone()),
            poses => group_pose(poses),
        }
    }
    fn handle_at(&self, at: [f64; 2], scale: f64) -> Option<Hit> {
        let pose = self.handle_pose()?;
        if pose.locked || !self.controls {
            return None;
        }
        let distance = |p: [f64; 2]| (p[0] - at[0]).hypot(p[1] - at[1]);
        let reach = self.tokens.metrics.hit as f64 / 2. / scale;
        if distance(pose.rotate_handle(self.stem(scale))) <= reach {
            return Some(Hit::Rotate);
        }
        // Small boxes overlap: the handle really under the pointer wins.
        (0..8)
            .map(|i| (i, distance(pose.handle(i))))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| Hit::Handle(i))
    }
    fn on_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return;
        };
        let at = self.to_comp(event.position, frame);
        let time = self.host.time(cx);
        let mut tool = self.host.tool(cx);
        // An open text edit owns presses on its text; any other press commits
        // it and then acts as a Select-tool click.
        if self.text_session().is_some() && !matches!(tool, Tool::Hand | Tool::Zoom) {
            if self.text_press(event, at, frame.scale) {
                cx.notify();
                return;
            }
            self.finish_text(false, cx);
            tool = Tool::Select;
        }
        if (tool == Tool::Select || event.modifiers.alt && self.studio.active())
            && self.studio_down(event, at, frame, cx)
        {
            return;
        }
        let start = |hit, click| Drag {
            hit,
            origin: event.position,
            active: false,
            poses: self.poses.clone(),
            time,
            click,
            snaps: None,
        };
        let deep = event.modifiers.platform || event.modifiers.control;
        let press = Press {
            at,
            shift: event.modifiers.shift,
            deep,
            time,
        };
        let drag = match tool {
            Tool::Other => return,
            Tool::Pen => {
                self.pen_down(event, at, frame.scale, cx);
                return;
            }
            Tool::Text => {
                self.text_sketch = Some(text::TextSketch {
                    origin: event.position,
                    start: at,
                    shift: event.modifiers.shift,
                    shape: None,
                });
                return;
            }
            Tool::Hand => {
                self.panning = Some(event.position);
                return;
            }
            Tool::Shape => {
                self.sketch = Some(Sketch {
                    origin: event.position,
                    start: at,
                    shape: None,
                });
                return;
            }
            Tool::Zoom => {
                let factor = if event.modifiers.alt { 0.8 } else { 1.25 };
                let at = event.position;
                self.host.navigate(Navigate::Zoom { factor, at }, cx);
                self.draw(cx);
                return;
            }
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
            Tool::Select if event.click_count == 2 => {
                self.text_double_click(at, window, cx);
                None
            }
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
    }
    /// Secondary press: selects what is under the pointer, locked layers
    /// included, then opens the shared layer menu on it.
    fn on_context(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return;
        };
        let at = self.to_comp(event.position, frame);
        cx.stop_propagation();
        if self.text_context(at, frame.scale) {
            return;
        }
        self.finish_text(false, cx);
        let press = Press {
            at,
            shift: false,
            deep: false,
            time: self.host.time(cx),
        };
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        let handle = window.window_handle();
        self.picks += 1;
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    let options = pm_kernel::PickOptions {
                        include_locked: true,
                    };
                    let hit = crate::text_host::query(&p, |q| {
                        q.pick(None, press.at[0], press.at[1], press.time, &options)
                    })
                    .ok()
                    .and_then(|hit| hit.layer_id);
                    // Empty canvas: the viewer's own menu, which only extensions fill.
                    let Some(hit) = hit else {
                        return Some((None, None));
                    };
                    if !p.selection.layers().contains(&hit) {
                        let ids = vec![hit.clone()];
                        let selection = SelectionCall::Layers { ids, add: false };
                        let request = Request::new(Caller::native(), selection);
                        kernel.call(&project, request).await?.ok()?;
                    }
                    let p = kernel.published(&project)?;
                    Some((Some(hit), read_at(&p, press.time)))
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.picks -= 1;
                let Some((hit, poses)) = picked else {
                    return;
                };
                if let Some(poses) = poses {
                    s.poses = Arc::new(poses);
                    s.draw(cx);
                }
                let host = s.host.clone();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| host.layer_menu(hit, window, cx));
                });
            });
        })
        .detach();
    }
    /// Asks the kernel what is under the pointer and applies the shipped click
    /// rules, off the main thread. The answer arrives while the press may be held.
    fn pick(&mut self, press: Press, cx: &mut Context<Self>) {
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        // The 2D picker can't measure Studio 3D layers; their own picking ran
        // first, so a 2D failure over a 3D scene is empty canvas.
        let studio = self.studio.active();
        self.picks += 1;
        cx.spawn(async move |this, cx| {
            let poses = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    let target = match pick_target(&p, press) {
                        Some(target) => target,
                        None if studio => None,
                        None => return None,
                    };
                    let selected = p.selection.layers();
                    let ids = match &target {
                        Some(id) if press.shift && selected.contains(id) => {
                            selected.iter().filter(|s| *s != id).cloned().collect()
                        }
                        Some(id) => vec![id.clone()],
                        // Empty canvas keeps the selection until the release
                        // decides (a click clears it, a marquee replaces it,
                        // Escape leaves it).
                        None => return Some((read_at(&p, press.time)?, None, true)),
                    };
                    let add =
                        press.shift && target.is_some() && !selected.contains(target.as_ref()?);
                    let selection = SelectionCall::Layers { ids, add };
                    let request = Request::new(Caller::native(), selection);
                    kernel.call(&project, request).await?.ok()?;
                    let p = kernel.published(&project)?;
                    let empty = target.is_none();
                    Some((
                        read_at(&p, press.time)?,
                        target.filter(|_| !press.shift),
                        empty,
                    ))
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.picks -= 1;
                let held = s.picking.take();
                let Some((poses, target, empty)) = poses else {
                    return;
                };
                s.poses = Arc::new(poses);
                s.draw(cx);
                // A press on empty canvas that is still held draws a marquee;
                // one already released was a click that clears.
                if empty && held.is_none() && !press.shift {
                    s.queue.push_back(Job::Select(None));
                    s.pump(cx);
                }
                if let Some(origin) = held.filter(|_| empty) {
                    s.marquee = Some(marquee::Marquee {
                        origin,
                        start: press.at,
                        shift: press.shift,
                        shape: None,
                    });
                }
                if let Some(origin) = held.filter(|_| target.is_some() && s.transformable()) {
                    s.drag = Some(Drag {
                        hit: Hit::Body,
                        origin,
                        active: false,
                        poses: s.poses.clone(),
                        time: press.time,
                        click: None,
                        snaps: None,
                    });
                }
            });
        })
        .detach();
    }
    fn on_wheel(&mut self, event: &ScrollWheelEvent, line: Pixels, cx: &mut Context<Self>) {
        if self.studio_wheel(event, line, cx) {
            return;
        }
        let delta = event.delta.pixel_delta(line);
        let m = event.modifiers;
        let (dx, dy) = (f32::from(delta.x) as f64, f32::from(delta.y) as f64);
        if m.control || m.platform || matches!(event.delta, ScrollDelta::Lines(_)) {
            // Responsive, reversible, bounded for coarse wheels (viewer.ts).
            let factor = (dy * 0.006).exp().clamp(0.5, 2.);
            let at = event.position;
            self.host.navigate(Navigate::Zoom { factor, at }, cx);
            self.draw(cx);
        } else if m.shift {
            // A plain two-finger scroll leaves the view where it is.
            let by = if dx == 0. {
                point(dy, 0.)
            } else {
                point(dx, dy)
            };
            let by = point(px(by.x as f32), px(by.y as f32));
            self.host.navigate(Navigate::Pan(by), cx);
        }
    }
    fn on_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.studio_move(event, cx) {
            return;
        }
        if let Some(last) = self.panning {
            self.panning = Some(event.position);
            self.host.navigate(Navigate::Pan(event.position - last), cx);
            return;
        }
        if self.sketch.is_some() || self.text_sketch.is_some() {
            self.sketch_move(event, cx);
            return;
        }
        if self.marquee_move(event, cx) {
            return;
        }
        if let Some(frame) = self.host.frame(self.bounds, cx) {
            let at = self.to_comp(event.position, frame);
            if self.text_drag(at) {
                cx.notify();
                return;
            }
            if self.pen_move(event, at, cx) {
                return;
            }
        }
        if self.drag.is_none() {
            if self.bounds.contains(&event.position) {
                self.hover(event.position, cx);
            }
            return;
        }
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
            if drag.hit == Hit::Body {
                let ids: Vec<String> = drag.poses.iter().map(|p| p.id.clone()).collect();
                let (kernel, project, time) =
                    (self.kernel.clone(), self.project.clone(), drag.time);
                let started = drag.origin;
                self.picks += 1;
                cx.spawn(async move |this, cx| {
                    let points = cx
                        .background_executor()
                        .spawn(
                            async move { snap::points(&*kernel.published(&project)?, &ids, time) },
                        )
                        .await;
                    let _ = this.update(cx, |s, _| {
                        s.picks -= 1;
                        if let (Some(drag), Some(points)) = (s.drag.as_mut(), points)
                            && drag.origin == started
                        {
                            drag.snaps = Some(Arc::new(points));
                        }
                    });
                })
                .detach();
            }
            self.queue.push_back(Job::Begin(match drag.hit {
                Hit::Body => "Move",
                Hit::Handle(_) if drag.poses.iter().any(|p| p.shape.is_some()) => "Resize Shape",
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
        // Moves align with the composition and other layers; Command
        // bypasses, and a Shift-locked axis stays put.
        let mut offset = [0., 0.];
        let mut guides = vec![];
        if drag.hit == Hit::Body
            && !mods.snap
            && let Some(points) = drag.snaps.clone()
        {
            let delta = locked_delta(origin, at, mods.shift);
            let axes = [!mods.shift || delta[0] != 0., !mods.shift || delta[1] != 0.];
            let moving = snap::box_points(&drag.poses, delta);
            let result = snap::snap(
                &moving,
                &points,
                snap::DISTANCE / frame.scale.max(0.02),
                axes,
            );
            offset = result.0;
            guides = result.1;
        }
        let commands = commands(drag, origin, at, mods, offset);
        // The alignment feedback marks catching something new, not each
        // step that stays caught.
        let caught = |g: &[snap::Guide]| g.iter().map(|(_, to)| *to).collect::<Vec<_>>();
        if caught(&guides) != caught(&self.guides) && !guides.is_empty() {
            snap::haptic();
        }
        if guides != self.guides {
            self.guides = guides;
            cx.notify();
        }
        self.queue.push_back(Job::Step(commands, true));
        // The viewer trace's input time; untraced, no timestamp is read.
        if pm_engine_service::trace::enabled() {
            self.step_input
                .get_or_insert_with(crate::session::Controller::event_time);
        }
        self.pump(cx);
    }
    fn on_up(&mut self, commit: bool, window: Option<&mut Window>, cx: &mut Context<Self>) {
        self.picking = None;
        self.panning = None;
        if self.studio_up(commit, cx) {
            return;
        }
        if !self.guides.is_empty() {
            self.guides.clear();
            cx.notify();
        }
        if self.text_release() || self.pen_up(commit, cx) || self.marquee_up(commit, cx) {
            return;
        }
        if let Some(sketch) = self.text_sketch.take() {
            self.draw(cx);
            if let (true, Some(window)) = (commit, window) {
                self.text_tool_release(sketch, window, cx);
            }
            return;
        }
        if let Some(sketch) = self.sketch.take() {
            if let Some(shape) = sketch.shape.filter(|_| commit) {
                self.create_shape(shape, cx);
            }
            self.draw(cx);
            return;
        }
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
        if self.finish_text(false, cx) {
            return true;
        }
        if self.pen_up(false, cx) || self.marquee_up(false, cx) {
            return true;
        }
        if self.studio.busy() {
            return self.studio_up(false, cx);
        }
        if self.drag.is_none() && self.sketch.is_none() && self.text_sketch.is_none() {
            return false;
        }
        self.on_up(false, None, cx);
        true
    }
    /// The box follows the pointer once it passes the drag threshold;
    /// Shift draws a square, Option from the centre.
    fn sketch_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return;
        };
        let at = self.to_comp(event.position, frame);
        let threshold = self.tokens.metrics.drag_threshold;
        let m = event.modifiers;
        let (origin, start, shape, square) = if let Some(s) = self.sketch.as_mut() {
            (s.origin, s.start, &mut s.shape, m.shift)
        } else if let Some(s) = self.text_sketch.as_mut() {
            // A text box keeps its proportions free.
            (s.origin, s.start, &mut s.shape, false)
        } else {
            return;
        };
        let moved = event.position - origin;
        if shape.is_none() && f32::from(moved.x).hypot(f32::from(moved.y)) < threshold {
            return;
        }
        *shape = Some(crate::shape_tool::ShapeBox::from_drag(
            start, at, m.alt, square,
        ));
        self.draw(cx);
    }
    /// One undo step, planned and written off the main thread; a new shape
    /// layer is selected, as in the shipped viewer.
    fn create_shape(&mut self, shape: crate::shape_tool::ShapeBox, cx: &mut Context<Self>) {
        let (kernel, project, time) = (
            self.kernel.clone(),
            self.project.clone(),
            self.host.time(cx),
        );
        let variant = self.host.shape_variant(cx);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let published = kernel.published(&project).ok_or("No open project")?;
                    let Some(plan) = crate::shape_tool::plan(&published, shape, time, &variant)
                    else {
                        return Ok(());
                    };
                    let call = Call::Transact {
                        label: Some(plan.label.into()),
                        body: WriteBody::Commands(plan.commands),
                        expected_revision: Some(published.revision()),
                    };
                    let ack = kernel
                        .call(&project, Request::new(Caller::native(), call))
                        .await
                        .ok_or("Kernel closed")?
                        .map_err(|e| format!("{e:?}"))?;
                    if let Reply::Throw(error) = ack.reply {
                        return Err(error.to_string());
                    }
                    if let Some(id) = plan.select {
                        let select = SelectionCall::Layers {
                            ids: vec![id],
                            add: false,
                        };
                        kernel
                            .call(&project, Request::new(Caller::native(), select))
                            .await;
                    }
                    Ok::<(), String>(())
                })
                .await;
            if let Err(error) = result {
                let _ = this.update(cx, |s, cx| s.host.report(error, cx));
            }
        })
        .detach();
    }
    /// View ▸ Show/Hide Layer Controls: boxes off, handles disarmed.
    pub fn toggle_controls(&mut self, cx: &mut Context<Self>) {
        self.controls = !self.controls;
        self.redraw(cx);
    }
    pub fn controls(&self) -> bool {
        self.controls
    }
    #[cfg(test)]
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
        // Superseding steps coalesce; a step that accumulates never drops.
        while matches!(job, Job::Step(_, true))
            && matches!(self.queue.front(), Some(Job::Step(_, true)))
        {
            job = self.queue.pop_front().unwrap();
        }
        self.busy = true;
        if matches!(job, Job::Step(..))
            && let Some(at) = self.step_input.take()
        {
            self.host.input(at, cx);
        }
        let (kernel, project, gesture) = (self.kernel.clone(), self.project.clone(), self.gesture);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let call: Call = match (job, gesture) {
                        (Job::Select(id), _) => {
                            let ids = id.into_iter().collect();
                            let select = SelectionCall::Layers { ids, add: false };
                            return kernel
                                .call(&project, Request::new(Caller::native(), select))
                                .await
                                .ok_or("Kernel closed")?
                                .map_err(|e| format!("{e:?}"));
                        }
                        (Job::Transact(label, commands, select), _) => {
                            let call = Call::Transact {
                                label: Some(label.into()),
                                body: WriteBody::Commands(commands),
                                expected_revision: None,
                            };
                            let ack = kernel
                                .call(&project, Request::new(Caller::native(), call))
                                .await
                                .ok_or("Kernel closed")?
                                .map_err(|e| format!("{e:?}"))?;
                            if let (Some(id), false) =
                                (select, matches!(ack.reply, Reply::Throw(_)))
                            {
                                let select = SelectionCall::Layers {
                                    ids: vec![id],
                                    add: false,
                                };
                                kernel
                                    .call(&project, Request::new(Caller::native(), select))
                                    .await;
                            }
                            return Ok(ack);
                        }
                        (Job::Begin(label), _) => GestureCall::Begin {
                            label: label.into(),
                        }
                        .into(),
                        (Job::Step(commands, replace), Some(gesture)) => GestureCall::Step {
                            gesture,
                            body: WriteBody::Commands(commands),
                            replace,
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
                if s.queue.is_empty()
                    && s.drag.is_none()
                    && s.pen_drag.is_none()
                    && !s.studio.busy()
                {
                    s.gesture = None;
                }
                s.pump(cx);
            });
        })
        .detach();
    }
    /// The common box's rotate handle and centre.
    #[cfg(test)]
    pub fn group_points(&self) -> Option<([f64; 2], [f64; 2], [f64; 2])> {
        let g = group_pose(&self.poses)?;
        Some((g.rotate_handle(self.stem(1.)), g.anchor, g.handle(4)))
    }
    #[cfg(test)]
    pub fn pen_paths(&self) -> Vec<usize> {
        self.pen.as_ref().map_or(vec![], |p| {
            p.targets.iter().map(|t| t.vertices.len()).collect()
        })
    }
    #[cfg(test)]
    pub fn pen_vertices(&self) -> usize {
        self.pen
            .as_ref()
            .map_or(0, |p| p.targets.iter().map(|t| t.vertices.len()).sum())
    }
    /// The text session, including one still finishing.
    pub fn text_open(&self) -> Option<&text::TextSession> {
        self.text.as_ref()
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
            pose.rotate_handle(self.stem(1.)),
        ))
    }
}
/// The common box of an unlocked multiple selection, as a pose whose
/// handles the shared scale and rotate drags use.
fn group_pose(poses: &[Pose]) -> Option<Pose> {
    if poses.len() < 2 || poses.iter().any(|p| p.locked) {
        return None;
    }
    let points = poses.iter().flat_map(|p| p.corners);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for [x, y] in points {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let centre = [(x0 + x1) / 2., (y0 + y1) / 2.];
    Some(Pose {
        id: String::new(),
        locked: false,
        root: false,
        keeps_aspect: false,
        world: [1., 0., 0., 1., 0., 0.],
        linear: [1., 0., 0., 1.],
        to_parent: [1., 0., 0., 1.],
        bounds: [x0, y0, x1, y1],
        shape: None,
        text_size: None,
        has_skew: false,
        corners: [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
        anchor: centre,
        local_anchor: centre,
        position: centre,
        scale: [100., 100.],
        rotation: 0.,
    })
}
/// A layer's local rotation, scale (as factors) and skew (degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Decomposed {
    pub rotation: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub skew: f64,
}
fn compose_linear(t: Decomposed) -> [f64; 4] {
    let (sin, cos) = t.rotation.to_radians().sin_cos();
    let skew = t.skew.to_radians().tan();
    [
        cos * t.scale_x,
        sin * t.scale_x,
        t.scale_y * (cos * skew - sin),
        t.scale_y * (sin * skew + cos),
    ]
}
/// The local rotation, scale and skew giving `desired` under `parent`
/// (viewer.ts `solveLocalTransformForWorldLinear`); a mirrored layer keeps
/// its mirror and the rotation stays near the one it had.
pub(crate) fn solve_local(
    parent: Option<[f64; 4]>,
    desired: [f64; 4],
    preferred_scale_x: f64,
    preferred_rotation: f64,
) -> Option<Decomposed> {
    let local = match parent {
        Some(p) => {
            let inverse = inverse_linear(p)?;
            let column = |x: f64, y: f64| apply(inverse, [x, y]);
            let (a, b) = (
                column(desired[0], desired[1]),
                column(desired[2], desired[3]),
            );
            [a[0], a[1], b[0], b[1]]
        }
        None => desired,
    };
    let scale_x = local[0].hypot(local[1]);
    if !scale_x.is_finite() || scale_x < 1e-8 {
        return None;
    }
    let (cos, sin) = (local[0] / scale_x, local[1] / scale_x);
    let skew_scale_y = cos * local[2] + sin * local[3];
    let scale_y = -sin * local[2] + cos * local[3];
    if !scale_y.is_finite() || scale_y.abs() < 1e-8 {
        return None;
    }
    let mut skew = skew_scale_y.atan2(scale_y);
    if skew > std::f64::consts::FRAC_PI_2 {
        skew -= std::f64::consts::PI;
    } else if skew < -std::f64::consts::FRAC_PI_2 {
        skew += std::f64::consts::PI;
    }
    let mut rotation = local[1].atan2(local[0]).to_degrees();
    let (mut sx, mut sy) = (scale_x, scale_y);
    if preferred_scale_x < 0. {
        rotation += 180.;
        sx = -sx;
        sy = -sy;
    }
    rotation += ((preferred_rotation - rotation) / 360.).round() * 360.;
    let out = Decomposed {
        rotation,
        scale_x: sx,
        scale_y: sy,
        skew: skew.to_degrees(),
    };
    let check = compose_linear(out);
    let magnitude = local.iter().fold(1f64, |m, v| m.max(v.abs()));
    check
        .iter()
        .zip(local)
        .all(|(c, l)| (c - l).abs() <= magnitude * 1e-7)
        .then_some(out)
}
/// Resize or rotate the selected root layers about the common box
/// (viewer.ts common transform). Resizing is proportional: type by its size,
/// parametric shapes by their size, others by a signed scale (a mirrored
/// layer stays mirrored), and each anchor follows the pivot (the opposite
/// handle, the centre with Option). Rotating carries each anchor about the
/// box centre and solves the layer's own rotation, scale and skew for the
/// turned world matrix under its parent (Shift: 15° steps, Command: 45°).
fn group_commands(drag: &Drag, origin: [f64; 2], at: [f64; 2], mods: Mods) -> Vec<Command> {
    let Some(group) = group_pose(&drag.poses) else {
        return vec![];
    };
    let t = drag.time;
    let centre = group.anchor;
    let mut out = vec![];
    let carry = |pose: &Pose, moved: [f64; 2], out: &mut Vec<Command>| {
        let d = apply(
            pose.to_parent,
            [moved[0] - pose.anchor[0], moved[1] - pose.anchor[1]],
        );
        out.push(set(
            &pose.id,
            "position.x",
            round(pose.position[0] + d[0], 3),
            t,
        ));
        out.push(set(
            &pose.id,
            "position.y",
            round(pose.position[1] + d[1], 3),
            t,
        ));
    };
    let roots = drag.poses.iter().filter(|p| p.root && !p.locked);
    match drag.hit {
        Hit::Handle(index) => {
            let (u, v) = HANDLES[index];
            let (active_x, active_y) = (u != 0.5, v != 0.5);
            let handle = group.handle(index);
            let pivot = if mods.alt {
                centre
            } else {
                group.local(
                    if active_x { 1. - u } else { 0.5 },
                    if active_y { 1. - v } else { 0.5 },
                )
            };
            let grabbed = [
                at[0] - (origin[0] - handle[0]),
                at[1] - (origin[1] - handle[1]),
            ];
            let (dx, dy) = (handle[0] - pivot[0], handle[1] - pivot[1]);
            let f = if active_x && active_y {
                ((grabbed[0] - pivot[0]) * dx + (grabbed[1] - pivot[1]) * dy)
                    / (dx * dx + dy * dy).max(1e-9)
            } else if active_x {
                (grabbed[0] - pivot[0]) / dx.abs().max(1e-9).copysign(dx)
            } else {
                (grabbed[1] - pivot[1]) / dy.abs().max(1e-9).copysign(dy)
            };
            // calculateResize's floor: the box never turns inside out.
            let f = f.max(0.01);
            for pose in roots {
                if let Some(size) = pose.text_size {
                    out.push(set(&pose.id, "c.size", round((size * f).max(4.), 2), t));
                } else if let Some(shape) = pose.shape {
                    let side = |v: f64| round(((v + shape.stroke) * f - shape.stroke).max(1.), 3);
                    out.push(set(&pose.id, "c.w", side(shape.width), t));
                    out.push(set(&pose.id, "c.h", side(shape.height), t));
                } else {
                    out.push(set(&pose.id, "scale.x", round(pose.scale[0] * f, 3), t));
                    out.push(set(&pose.id, "scale.y", round(pose.scale[1] * f, 3), t));
                }
                let moved = [
                    pivot[0] + (pose.anchor[0] - pivot[0]) * f,
                    pivot[1] + (pose.anchor[1] - pivot[1]) * f,
                ];
                carry(pose, moved, &mut out);
            }
        }
        Hit::Rotate => {
            let angle = |p: [f64; 2]| (p[1] - centre[1]).atan2(p[0] - centre[0]);
            let mut turn =
                ((angle(at) - angle(origin)).to_degrees() + 180.).rem_euclid(360.) - 180.;
            if mods.snap {
                turn = (turn / 45.).round() * 45.;
            } else if mods.shift {
                turn = (turn / 15.).round() * 15.;
            }
            let (s, c) = turn.to_radians().sin_cos();
            for pose in roots {
                let r = [pose.anchor[0] - centre[0], pose.anchor[1] - centre[1]];
                let moved = [
                    centre[0] + c * r[0] - s * r[1],
                    centre[1] + s * r[0] + c * r[1],
                ];
                carry(pose, moved, &mut out);
                let parent = inverse_linear(pose.to_parent).filter(|p| {
                    p.iter()
                        .zip([1., 0., 0., 1.])
                        .any(|(a, b)| (a - b).abs() > 1e-9)
                });
                let Some(parent) = parent else {
                    // No parent: turning the world turns only the rotation.
                    out.push(set(&pose.id, "rotation", round(pose.rotation + turn, 3), t));
                    continue;
                };
                let w = [pose.world[0], pose.world[1], pose.world[2], pose.world[3]];
                let desired = [
                    c * w[0] - s * w[1],
                    s * w[0] + c * w[1],
                    c * w[2] - s * w[3],
                    s * w[2] + c * w[3],
                ];
                let Some(local) =
                    solve_local(Some(parent), desired, pose.scale[0] / 100., pose.rotation)
                else {
                    continue;
                };
                out.push(set(&pose.id, "rotation", round(local.rotation, 4), t));
                out.push(set(&pose.id, "scale.x", round(local.scale_x * 100., 4), t));
                out.push(set(&pose.id, "scale.y", round(local.scale_y * 100., 4), t));
                // A layer saved without a skew channel takes none it doesn't need.
                if pose.has_skew || local.skew.abs() > 1e-6 {
                    out.push(set(&pose.id, "skew", round(local.skew, 4), t));
                }
            }
        }
        _ => {}
    }
    out
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
    // A viewer edit is the artist's hand: it overrides earlier hand edits and
    // protects its value from agents, as in the shipped viewer.
    let mut set = SetProperty::new(id, channel, Value::Number(value), time);
    set.preserve_hand_edits = false;
    set.mark_intent = true;
    PropertyCommand::Set(set).into()
}
#[derive(Clone, Copy)]
struct Mods {
    shift: bool,
    alt: bool,
    /// Command or Control.
    snap: bool,
}
/// Shift locks a move's dominant axis.
fn locked_delta(origin: [f64; 2], at: [f64; 2], shift: bool) -> [f64; 2] {
    let mut delta = [at[0] - origin[0], at[1] - origin[1]];
    if shift {
        let axis = usize::from(delta[0].abs() > delta[1].abs());
        delta[axis] = 0.;
    }
    delta
}
fn commands(
    drag: &Drag,
    origin: [f64; 2],
    at: [f64; 2],
    mods: Mods,
    snap: [f64; 2],
) -> Vec<Command> {
    let delta = if matches!(drag.hit, Hit::Body) {
        let d = locked_delta(origin, at, mods.shift);
        [d[0] + snap[0], d[1] + snap[1]]
    } else {
        [at[0] - origin[0], at[1] - origin[1]]
    };
    if drag.poses.len() > 1 && matches!(drag.hit, Hit::Handle(_) | Hit::Rotate) {
        return group_commands(drag, origin, at, mods);
    }
    let t = drag.time;
    let mut out = vec![];
    let moves_roots = matches!(drag.hit, Hit::Body);
    for pose in drag
        .poses
        .iter()
        .filter(|p| !p.locked && (p.root || !moves_roots))
    {
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
    if let Some(shape) = pose.shape {
        return resize_shape(pose, shape, pivot, [fx, fy], t);
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
/// Selection bounds carry half the stroke on each side, and a shape's box grows
/// about its own centre, so the fixed point follows its place inside that box.
fn resize_shape(
    pose: &Pose,
    shape: ShapeSize,
    pivot: [f64; 2],
    f: [f64; 2],
    t: f64,
) -> Vec<Command> {
    let [x0, y0, x1, y1] = pose.bounds;
    let (span_x, span_y) = (x1 - x0, y1 - y0);
    let width = (span_x * f[0] - shape.stroke).max(1.);
    let height = (span_y * f[1] - shape.stroke).max(1.);
    let centre = pose.local(0.5, 0.5);
    let moved = [
        (pivot[0] - centre[0]) * (1. - (width + shape.stroke) / span_x.max(1e-9)),
        (pivot[1] - centre[1]) * (1. - (height + shape.stroke) / span_y.max(1e-9)),
    ];
    let correction = apply(pose.linear, moved);
    vec![
        set(&pose.id, "c.w", round(width, 3), t),
        set(&pose.id, "c.h", round(height, 3), t),
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
    let hit = crate::text_host::query(p, |q| {
        q.pick(
            None,
            press.at[0],
            press.at[1],
            press.time,
            &Default::default(),
        )
    })
    .ok()?;
    let Some(id) = hit.layer_id else {
        return Some(None);
    };
    // A 3D layer draws over the whole frame; its own picking ran first, so
    // a 2D hit on one is empty canvas.
    if is_studio(p, &id) {
        return Some(None);
    }
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
    let half = t.metrics.gizmo_handle as f64 / 2. / frame.scale;
    let stem = t.metrics.rotation_stem as f64 / frame.scale;
    // A circle as an octagon of engine lines (the rotation knob, the anchor ring).
    let ring = |p: [f64; 2], r: f64, line: &mut dyn FnMut([f64; 2], [f64; 2])| {
        let at = |i: usize| {
            let a = i as f64 * std::f64::consts::FRAC_PI_4 + std::f64::consts::FRAC_PI_8;
            [p[0] + r * a.cos(), p[1] + r * a.sin()]
        };
        for i in 0..8 {
            line(at(i), at(i + 1));
        }
    };
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
    // The box a press can take comes first: a huge selection over the
    // engine's line budget loses its last per-layer outlines, never handles.
    let common = (poses.len() > 1).then(|| group_pose(poses)).flatten();
    if let Some(group) = &common {
        for i in 0..4 {
            line(group.corners[i], group.corners[(i + 1) % 4]);
        }
        for i in 0..8 {
            square(group.handle(i), &mut line);
        }
        line(group.handle(1), group.rotate_handle(stem));
        ring(group.rotate_handle(stem), half, &mut line);
    }
    if let [pose] = poses
        && !pose.locked
    {
        for i in 0..8 {
            square(pose.handle(i), &mut line);
        }
        // Rotation is a round knob, so it never reads as a scale handle.
        line(pose.handle(1), pose.rotate_handle(stem));
        ring(pose.rotate_handle(stem), half, &mut line);
        // The anchor point: a ring with a cross through it (After Effects, v1).
        let a = pose.anchor;
        let reach = t.metrics.anchor_mark as f64 / 2. / frame.scale;
        ring(a, reach * 0.6, &mut line);
        line([a[0] - reach, a[1]], [a[0] + reach, a[1]]);
        line([a[0], a[1] - reach], [a[0], a[1] + reach]);
    }
    for pose in poses {
        for i in 0..4 {
            line(pose.corners[i], pose.corners[(i + 1) % 4]);
        }
    }
    lines
}
impl Render for GizmoSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        div()
            .id("viewer-gizmos")
            .size_full()
            .cursor(self.cursor)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, event: &MouseDownEvent, window, cx| s.on_down(event, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|s, event: &MouseDownEvent, _, cx| {
                    if let Some(frame) = s.host.frame(s.bounds, cx) {
                        let at = s.to_comp(event.position, frame);
                        s.studio_down(event, at, frame, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|s, event: &MouseDownEvent, window, cx| {
                    s.on_context(event, window, cx);
                }),
            )
            .on_scroll_wheel(cx.listener(|s, event: &ScrollWheelEvent, window, cx| {
                s.on_wheel(event, window.line_height(), cx);
            }))
            .on_pinch(cx.listener(|s, event: &PinchEvent, _, cx| {
                let (factor, at) = (1. + event.delta as f64, event.position);
                s.host.navigate(Navigate::Zoom { factor, at }, cx);
                s.draw(cx);
            }))
            .child(
                canvas(
                    {
                        let this = this.clone();
                        move |bounds, _, cx| {
                            let _ = this.update(cx, |s, _| s.bounds = bounds);
                        }
                    },
                    move |_, _, window, cx| {
                        let _ = this.update(cx, |s, cx| {
                            if let Some(frame) = s.host.frame(s.bounds, cx) {
                                s.pen_paint(frame, window);
                                s.marquee_paint(frame, window);
                                s.guides_paint(frame, window);
                                s.text_paint(frame, window);
                            }
                        });
                        let moved = this.clone();
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = moved.update(cx, |s, cx| s.on_move(e, cx));
                            }
                        });
                        let released = this.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ =
                                    released.update(cx, |s, cx| s.on_up(true, Some(window), cx));
                            }
                        });
                    },
                )
                .size_full(),
            )
    }
}

/// A Studio 3D layer (model, light, camera) by its definition.
/// A text or shape layer the 3D renderer draws with depth.
pub(crate) fn is_extruded(p: &pm_kernel::Published, id: &str) -> bool {
    p.snapshot.root().is_some_and(|root| {
        pm_kernel::model::find_layer(root, id)
            .and_then(|at| pm_kernel::model::layer_at(root, &at))
            .map(|l| pm_scene3d::from_model(&l.to_value()).unwrap_or_default())
            .is_some_and(|l| pm_scene3d::extrusion::enabled(&l))
    })
}
pub(crate) fn is_studio(p: &pm_kernel::Published, id: &str) -> bool {
    p.snapshot.root().is_some_and(|root| {
        pm_kernel::model::find_layer(root, id)
            .and_then(|at| pm_kernel::model::layer_at(root, &at))
            .and_then(|l| l.get("d"))
            .and_then(|d| d.get("definition"))
            .and_then(|d| d.to_value().as_str().map(str::to_owned))
            .is_some_and(|d| d.starts_with("powermove.3d."))
    })
}
