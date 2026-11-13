//! Viewer interaction for Studio 3D layers (3d-studio.md §2, §4–6): Free View
//! navigation, click-to-select by picking, move/rotate/scale handles and the
//! Light tool. Handles are drawn by the host from the worker's viewport model,
//! as engine overlay lines, on the frame they're pressed. Every drag is one
//! kernel gesture; navigation never edits the document or Undo.
use super::{GizmoSurface, Job, ViewFrame, set};
use gpui::*;
use pm_engine_service::{OverlayLine, studio::FreeView};
use pm_kernel::{Caller, Command, Request, Scene3dCommand, SelectionCall};
use pm_panel_studio3d::viewport::{Eye, Item, Scene};
use pm_scene3d::navigation::{
    Lens, Pose as View, Vec3, add, cross, dolly, dot, length, mul, normalize, orbit, pan, sub,
};
use pm_scene3d::schema::Role;
use std::sync::Arc;

/// The viewer's 3D tool (UI state `studioTool`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Move,
    Rotate,
    Scale,
    Orbit,
    Pan,
    Zoom,
    Light,
}
impl Tool {
    pub fn parse(s: Option<&str>) -> Self {
        match s {
            Some("rotate") => Self::Rotate,
            Some("scale") => Self::Scale,
            Some("orbit") => Self::Orbit,
            Some("pan") => Self::Pan,
            Some("zoom") => Self::Zoom,
            Some("light") => Self::Light,
            _ => Self::Move,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Handle {
    Axis(usize),
    /// Inside the object: move in the view plane.
    Plane,
    /// The square between two axes: move in that plane (index of its normal).
    Pair(usize),
    Ring(usize),
    Uniform,
    /// A light's marker (index into the scene's items): pressing selects the
    /// light and dragging moves it in the view plane, as Electron's markers.
    /// `true` while pinned to the visible edge, where it only selects.
    Marker(usize, bool),
    /// A selected light's aim point (◆): dragging moves its target.
    Target(usize),
}
/// A light marker's radius, in points.
const MARKER: f64 = 9.5;
/// Where a light's marker sits in composition pixels, and the outward
/// direction when it is pinned to the edge of the visible area `vis`
/// (`[x0, y0, x1, y1]`) because the light is outside it or behind the eye.
fn marker_place(
    eye: &Eye,
    p: Vec3,
    vis: [f64; 4],
    margin: f64,
) -> Option<([f64; 2], Option<[f64; 2]>)> {
    let inner = [
        vis[0] + margin,
        vis[1] + margin,
        vis[2] - margin,
        vis[3] - margin,
    ];
    if inner[2] <= inner[0] || inner[3] <= inner[1] {
        return None;
    }
    let c = [(inner[0] + inner[2]) / 2., (inner[1] + inner[3]) / 2.];
    let dir = match eye.project(p) {
        Some((q, _))
            if q[0] >= inner[0] && q[0] <= inner[2] && q[1] >= inner[1] && q[1] <= inner[3] =>
        {
            return Some((q, None));
        }
        Some((q, _)) => [q[0] - c[0], q[1] - c[1]],
        None => {
            let (r, u) = eye.right_up();
            let v = sub(p, eye.pose.position);
            [dot(v, r), -dot(v, u)]
        }
    };
    let l = dir[0].hypot(dir[1]);
    let d = if l < 1e-9 {
        [0., 1.]
    } else {
        [dir[0] / l, dir[1] / l]
    };
    let (hw, hh) = ((inner[2] - inner[0]) / 2., (inner[3] - inner[1]) / 2.);
    let t = (hw / d[0].abs().max(1e-9)).min(hh / d[1].abs().max(1e-9));
    Some(([c[0] + d[0] * t, c[1] + d[1] * t], Some(d)))
}
fn circle(
    c: [f64; 2],
    r: f64,
    color: [f32; 4],
    push: &mut dyn FnMut([f64; 2], [f64; 2], [f32; 4]),
) {
    const N: usize = 24;
    let at = |k: usize| {
        let a = k as f64 / N as f64 * std::f64::consts::TAU;
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    };
    for k in 0..N {
        push(at(k), at(k + 1), color);
    }
}
fn dashed(
    a: [f64; 2],
    b: [f64; 2],
    dash: f64,
    color: [f32; 4],
    push: &mut dyn FnMut([f64; 2], [f64; 2], [f32; 4]),
) {
    let l = (b[0] - a[0]).hypot(b[1] - a[1]);
    let n = (l / dash.max(1e-6)).floor().max(1.) as usize;
    for k in (0..n).step_by(2) {
        let (s, e) = (k as f64 / n as f64, ((k + 1) as f64 / n as f64).min(1.));
        push(
            [a[0] + (b[0] - a[0]) * s, a[1] + (b[1] - a[1]) * s],
            [a[0] + (b[0] - a[0]) * e, a[1] + (b[1] - a[1]) * e],
            color,
        );
    }
}
/// A light's emitter frame, as the renderer builds it: `dir` toward the
/// target, `side` level with the floor, `up` across both.
fn emitter_basis(pivot: Vec3, target: Vec3) -> (Vec3, Vec3, Vec3) {
    let dir = normalize(sub(target, pivot));
    let mut side = cross(dir, [0., 1., 0.]);
    if length(side) < 1e-3 {
        side = [1., 0., 0.];
    }
    let side = normalize(side);
    (dir, side, cross(dir, side))
}
/// `v` turned `deg` degrees about the unit axis `k` (Rodrigues).
fn turn(v: Vec3, k: Vec3, deg: f64) -> Vec3 {
    let (s, c) = deg.to_radians().sin_cos();
    add(
        add(mul(v, c), mul(cross(k, v), s)),
        mul(k, dot(k, v) * (1. - c)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Nav {
    Orbit,
    Pan,
    Dolly,
}
struct NavDrag {
    mode: Nav,
    start: View,
    origin: Point<Pixels>,
}
struct Drag {
    handle: Handle,
    origin: [f64; 2],
    screen: Point<Pixels>,
    eye: Eye,
    pivot: Vec3,
    /// World length of one handle (axis length), for scaling deltas.
    length: f64,
    items: Vec<Item>,
    active: bool,
    time: f64,
    /// How far a move has carried the pivot, so the handle follows.
    offset: Vec3,
    /// The handle axes when the press began (scene or object).
    basis: [Vec3; 3],
}
struct LightOrbit {
    id: String,
    start: View,
    origin: Point<Pixels>,
    active: bool,
    time: f64,
}
#[derive(Default)]
pub struct State {
    pub scene: Option<Arc<Scene>>,
    /// The 3D layers in the selection.
    pub selected: Vec<String>,
    pub tool: Tool,
    /// Looking through Free View rather than the shot (UI state `studioLook`).
    pub free_look: bool,
    pub free: Option<View>,
    /// Handles follow the object's own axes (UI state `studioAxes`).
    pub object_axes: bool,
    /// The Light panel's kind to add and whether Add is armed (UI state
    /// `studioLightKind`, `studioLightAdd`).
    pub light_kind: String,
    pub light_add: bool,
    /// How a click aims the selected light (UI state `studioLightAim`):
    /// `reflect` puts its highlight there, `direct` faces it there, `orbit`
    /// drags it around what it lights.
    pub light_aim: String,
    light_orbit: Option<LightOrbit>,
    /// The handle under the pointer, drawn in text colour (§5).
    pub(super) hover: Option<Handle>,
    nav: Option<NavDrag>,
    drag: Option<Drag>,
}
fn rgb(c: u32) -> [f32; 4] {
    let ch = |s: u32| ((c >> s) & 255) as f32 / 255.;
    [ch(16), ch(8), ch(0), 1.]
}
const AXES: [Vec3; 3] = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
fn dist_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-12 {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0., 1.)
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}
/// A scene-space move as the layer's position channels: scene units for
/// Studio layers; pixels with y and z flipped for extruded text and shapes.
fn channel_delta(item: &Item, w: Vec3) -> Vec3 {
    match item.pixels {
        Some(u) if u > 0. => [w[0] / u, -w[1] / u, -w[2] / u],
        _ => w,
    }
}
/// The layer's own axes: the columns of Rz·Ry·Rx, the order the
/// evaluator composes `rotation`, `rotation.y` and `rotation.x`.
pub(super) fn object_basis(rotation: Vec3) -> [Vec3; 3] {
    let (x, y, z) = (
        rotation[0].to_radians(),
        rotation[1].to_radians(),
        rotation[2].to_radians(),
    );
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    // R = Rz * Ry * Rx; column i is where axis i points.
    let m = [
        [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
        [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
        [-sy, cy * sx, cy * cx],
    ];
    std::array::from_fn(|c| [m[0][c], m[1][c], m[2][c]])
}
/// Points around the ring perpendicular to `basis[axis]`, radius `r`.
fn ring(pivot: Vec3, basis: &[Vec3; 3], axis: usize, r: f64, n: usize) -> Vec<Vec3> {
    let (u, v) = (basis[(axis + 1) % 3], basis[(axis + 2) % 3]);
    (0..=n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            add(pivot, add(mul(u, r * a.cos()), mul(v, r * a.sin())))
        })
        .collect()
}
impl State {
    pub fn active(&self) -> bool {
        self.scene.is_some()
    }
    pub fn busy(&self) -> bool {
        self.nav.is_some() || self.drag.is_some() || self.light_orbit.is_some()
    }
    fn view(&self) -> Option<View> {
        let scene = self.scene.as_ref()?;
        Some(if self.free_look {
            self.free.unwrap_or(scene.shot)
        } else {
            scene.shot
        })
    }
    fn eye(&self) -> Option<Eye> {
        let scene = self.scene.as_ref()?;
        Some(scene.eye(self.view()))
    }
    /// Unlocked selected items the current tool can move.
    fn movable(&self) -> Vec<Item> {
        let Some(scene) = &self.scene else {
            return vec![];
        };
        scene
            .items
            .iter()
            .filter(|i| self.selected.contains(&i.id) && !i.locked)
            .filter(|i| match (i.role, i.light.as_deref(), self.tool) {
                // A Sun only turns; a Bulb only moves (3d-studio.md §2).
                (Role::Light, Some("sun"), Tool::Move | Tool::Scale) => false,
                (Role::Light, Some("point"), Tool::Rotate | Tool::Scale) => false,
                (Role::Light, _, Tool::Scale) => false,
                _ => true,
            })
            .cloned()
            .collect()
    }
    /// Handle axes: the scene's, or with Object Axes the first selected
    /// layer's own.
    pub(super) fn basis(&self) -> [Vec3; 3] {
        match self.movable().first() {
            Some(item) if self.object_axes => object_basis(item.rotation),
            _ => AXES,
        }
    }
    fn pivot(items: &[Item]) -> Option<Vec3> {
        if items.is_empty() {
            return None;
        }
        let sum = items.iter().fold([0.; 3], |a, i| add(a, i.pivot));
        Some(mul(sum, 1. / items.len() as f64))
    }
}

impl GizmoSurface {
    /// The viewer worker sees this viewer's Free View, or the shot.
    pub(super) fn studio_push_view(&mut self, cx: &mut Context<Self>) {
        let view = if self.studio.free_look {
            self.studio.free.map(|v| FreeView {
                eye: v.position,
                target: v.target,
                zoom: v.zoom,
            })
        } else {
            None
        };
        self.host.studio_view(view, cx);
    }
    /// Writes a viewer UI key (`studioLook`, `studioTool`): not the document.
    pub(super) fn studio_ui(&self, key: &'static str, value: &'static str, cx: &mut Context<Self>) {
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        cx.background_executor()
            .spawn(async move {
                let Some(p) = kernel.published(&project) else {
                    return;
                };
                if p.ui_state.viewer.get(key).and_then(pm_model::Value::as_str) == Some(value) {
                    return;
                }
                let mut ui = p.ui_state.clone();
                if let Some(v) = ui.viewer.as_object_mut() {
                    v.insert(key, pm_model::Value::from(value));
                }
                let state = kernel.state();
                let _ = kernel
                    .state_transaction(pm_kernel::state::StateTransaction {
                        expected_revision: Some(state.revision),
                        ops: vec![pm_kernel::state::StateOp::Ui { project, next: ui }],
                    })
                    .await;
            })
            .detach();
    }
    /// Switch to Free View, starting from the shot.
    fn studio_go_free(&mut self, cx: &mut Context<Self>) {
        if !self.studio.free_look {
            self.studio.free_look = true;
            if self.studio.free.is_none() {
                self.studio.free = self.studio.scene.as_ref().map(|s| s.shot);
            }
            self.studio_ui("studioLook", "free", cx);
        }
    }
    /// A viewer 3D setting from View ▸ 3D (`look`, `tool`, `axes`, `views`,
    /// `tier`) or `frame`, as the viewer bar's controls write it.
    pub fn studio_setting(&mut self, group: &str, value: &str, cx: &mut Context<Self>) {
        const LOOKS: &[&str] = &["camera", "free"];
        const TOOLS: &[&str] = &["move", "rotate", "scale", "orbit", "pan", "zoom", "light"];
        const AXES_: &[&str] = &["scene", "object"];
        const VIEWS: &[&str] = &["1", "2", "4"];
        const TIERS: &[&str] = &["live", "refine"];
        let (key, allowed) = match group {
            "frame" => return self.studio_frame(cx),
            "look" => ("studioLook", LOOKS),
            "tool" => ("studioTool", TOOLS),
            "axes" => ("studioAxes", AXES_),
            "views" => ("studioViews", VIEWS),
            "tier" => ("studioTier", TIERS),
            _ => return,
        };
        if let Some(v) = allowed.iter().find(|a| **a == value) {
            if key == "studioLook" && *v == "free" {
                self.studio_go_free(cx);
                self.studio_push_view(cx);
                return;
            }
            self.studio_ui(key, v, cx);
        }
    }
    /// Looking through Free View in a 3D composition: the viewer fills its
    /// whole body with the editor view.
    pub fn studio_free(&self) -> bool {
        self.studio.active() && self.studio.free_look
    }
    /// Where a light's marker and its ◆ target sit, in composition pixels,
    /// with the whole composition in view (tests drive them as a person would).
    #[cfg(test)]
    pub(crate) fn studio_light_points(&self, id: &str) -> Option<([f64; 2], Option<[f64; 2]>)> {
        let scene = self.studio.scene.as_ref()?;
        let eye = self.studio.eye()?;
        let item = scene.items.iter().find(|i| i.id == id)?;
        let (m, _) = marker_place(
            &eye,
            item.pivot,
            [0., 0., scene.width, scene.height],
            MARKER + 6.,
        )?;
        Some((m, item.target.and_then(|t| eye.project(t)).map(|p| p.0)))
    }
    /// The editor view's pose while looking around (tests).
    #[cfg(test)]
    pub(crate) fn studio_free_pose(&self) -> Option<View> {
        self.studio.free_look.then_some(self.studio.free).flatten()
    }
    /// Whether the composition shows Studio 3D layers (F frames them).
    pub fn studio_active(&self) -> bool {
        self.studio.active()
    }
    /// Frame (F): Free View fits the selection, or everything.
    pub fn studio_frame(&mut self, cx: &mut Context<Self>) {
        let Some(scene) = self.studio.scene.clone() else {
            return;
        };
        let from = self.studio.view().unwrap_or(scene.shot);
        if let Some(pose) = pm_panel_studio3d::viewport::framed(&scene, from, &self.studio.selected)
        {
            self.studio_go_free(cx);
            self.studio.free = Some(pose);
            self.studio_push_view(cx);
            self.draw(cx);
        }
    }
    /// Lines for the 3D selection and its handles, in normalized frame space.
    pub(super) fn studio_lines(&self, frame: ViewFrame) -> Vec<OverlayLine> {
        let mut lines = vec![];
        let (Some(scene), Some(eye)) = (&self.studio.scene, self.studio.eye()) else {
            return lines;
        };
        let [w, h] = frame.size;
        let t = &self.tokens;
        let mut push = |a: [f64; 2], b: [f64; 2], color: [f32; 4]| {
            lines.push(OverlayLine {
                ends: [a[0] / w, a[1] / h, b[0] / w, b[1] / h].map(|v| v as f32),
                color,
            });
        };
        let accent = rgb(t.colors.accent);
        // During a drag only the active handle stays (§5), with its guide.
        let drag = self.studio.drag.as_ref().filter(|d| d.active);
        for item in scene
            .items
            .iter()
            .filter(|i| self.studio.selected.contains(&i.id) && i.role != Role::Light)
        {
            if drag.is_some() {
                break;
            }
            let b = item.bounds;
            let corner = |n: usize| -> Vec3 {
                std::array::from_fn(|i| {
                    if n & (1 << i) == 0 {
                        b.min[i]
                    } else {
                        b.max[i]
                    }
                })
            };
            for (a, c) in [
                (0, 1),
                (2, 3),
                (4, 5),
                (6, 7),
                (0, 2),
                (1, 3),
                (4, 6),
                (5, 7),
                (0, 4),
                (1, 5),
                (2, 6),
                (3, 7),
            ] {
                if let (Some((pa, _)), Some((pb, _))) =
                    (eye.project(corner(a)), eye.project(corner(c)))
                {
                    push(pa, pb, accent);
                }
            }
        }
        // The shot camera, while looking around: its frustum and the frame it
        // renders, so the editor view shows where the shot is.
        if self.studio.free_look
            && let Lens::Perspective { fov, aspect, .. } = scene.lens
        {
            let shot = scene.shot;
            let mut ink = rgb(t.colors.secondary);
            ink[3] = 0.8;
            let (_, r, down) = emitter_basis(shot.position, shot.target);
            let d = length(sub(shot.target, shot.position)).max(0.5);
            let hh = d * (fov.to_radians() / 2.).tan() / shot.zoom.max(1e-6);
            let hw = hh * aspect;
            let corner = |x: f64, y: f64| add(shot.target, add(mul(r, x * hw), mul(down, y * hh)));
            let corners = [
                corner(-1., -1.),
                corner(1., -1.),
                corner(1., 1.),
                corner(-1., 1.),
            ];
            let at = eye.project(shot.position).map(|p| p.0);
            let shown: Vec<_> = corners
                .iter()
                .map(|c| eye.project(*c).map(|p| p.0))
                .collect();
            for k in 0..4 {
                if let (Some(a), Some(b)) = (shown[k], shown[(k + 1) % 4]) {
                    dashed(a, b, 6. / frame.scale, ink, &mut push);
                }
                if let (Some(a), Some(b)) = (at, shown[k]) {
                    push(a, b, ink);
                }
            }
            if let Some(a) = at {
                let s = 7. / frame.scale;
                for (p, q) in [
                    ([-s, -s], [s, -s]),
                    ([s, -s], [s, s]),
                    ([s, s], [-s, s]),
                    ([-s, s], [-s, -s]),
                ] {
                    push([a[0] + p[0], a[1] + p[1]], [a[0] + q[0], a[1] + q[1]], ink);
                }
            }
        }
        // Lights, always (as Electron's markers): a marker at each light,
        // pinned to the visible edge when it is outside or behind; a selected
        // light adds its emitter or cone, its aim line and its ◆ target.
        {
            let ink = rgb(t.colors.text);
            let shade = [0., 0., 0., 0.5];
            let pt = 1. / frame.scale;
            let vis = self.studio_visible(frame);
            for (index, item) in scene.items.iter().enumerate() {
                if item.role != Role::Light {
                    continue;
                }
                let sel = self.studio.selected.contains(&item.id);
                let kind = item.light.as_deref().unwrap_or("area");
                if sel
                    && kind != "point"
                    && let Some(target) = item.target
                {
                    let (_, side, up) = emitter_basis(item.pivot, target);
                    if kind == "area" && item.shape[0] > 0. {
                        let (w, h) = (item.shape[0] / 2., item.shape[1] / 2.);
                        let q: Vec<_> = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)]
                            .iter()
                            .map(|(x, y)| {
                                eye.project(add(item.pivot, add(mul(side, x * w), mul(up, y * h))))
                                    .map(|p| p.0)
                            })
                            .collect();
                        for k in 0..4 {
                            if let (Some(a), Some(b)) = (q[k], q[(k + 1) % 4]) {
                                push(a, b, accent);
                            }
                        }
                    }
                    if kind == "spot" && item.shape[2] > 0. {
                        let d = length(sub(target, item.pivot));
                        let rad = d * item.shape[2].clamp(1., 89.).to_radians().tan();
                        let rim = |k: usize| {
                            let a = k as f64 / 24. * std::f64::consts::TAU;
                            add(
                                target,
                                add(mul(side, rad * a.cos()), mul(up, rad * a.sin())),
                            )
                        };
                        for k in 0..24 {
                            if let (Some((a, _)), Some((b, _))) =
                                (eye.project(rim(k)), eye.project(rim(k + 1)))
                            {
                                push(a, b, accent);
                            }
                        }
                        for k in [0, 6, 12, 18] {
                            if let (Some((a, _)), Some((b, _))) =
                                (eye.project(item.pivot), eye.project(rim(k)))
                            {
                                push(a, b, accent);
                            }
                        }
                    }
                    if let (Some((a, _)), Some((b, _))) =
                        (eye.project(item.pivot), eye.project(target))
                    {
                        dashed(a, b, 5. * pt, accent, &mut push);
                        let color = if self.studio.hover == Some(Handle::Target(index)) {
                            ink
                        } else {
                            accent
                        };
                        let s = 6. * pt;
                        let dm = [
                            [b[0], b[1] - s],
                            [b[0] + s, b[1]],
                            [b[0], b[1] + s],
                            [b[0] - s, b[1]],
                        ];
                        for k in 0..4 {
                            push(dm[k], dm[(k + 1) % 4], color);
                        }
                        let s = 2. * pt;
                        push([b[0] - s, b[1]], [b[0] + s, b[1]], color);
                    }
                }
                let Some((m, edge)) = marker_place(&eye, item.pivot, vis, (MARKER + 6.) * pt)
                else {
                    continue;
                };
                let r = MARKER * pt;
                circle(m, r + pt, shade, &mut push);
                circle(m, r, ink, &mut push);
                if sel {
                    circle(m, r + 3.5 * pt, accent, &mut push);
                    circle(m, r + 4.5 * pt, accent, &mut push);
                }
                let g = |x: f64, y: f64| [m[0] + x * pt, m[1] + y * pt];
                let strokes: &[((f64, f64), (f64, f64))] = match kind {
                    "spot" => &[
                        ((-2., -5.), (2., -5.)),
                        ((2., -5.), (5., 4.)),
                        ((5., 4.), (-5., 4.)),
                        ((-5., 4.), (-2., -5.)),
                    ],
                    "point" => {
                        circle(m, 3.5 * pt, ink, &mut push);
                        &[]
                    }
                    "sun" => {
                        circle(m, 3. * pt, ink, &mut push);
                        for k in 0..8 {
                            let a = k as f64 / 8. * std::f64::consts::TAU;
                            push(
                                g(5. * a.cos(), 5. * a.sin()),
                                g(7. * a.cos(), 7. * a.sin()),
                                ink,
                            );
                        }
                        &[]
                    }
                    _ => &[
                        ((-5., -3.8), (5., -3.8)),
                        ((5., -3.8), (5., 3.8)),
                        ((5., 3.8), (-5., 3.8)),
                        ((-5., 3.8), (-5., -3.8)),
                        ((-1.7, -3.8), (-1.7, 3.8)),
                        ((1.7, -3.8), (1.7, 3.8)),
                    ],
                };
                for (a, b) in strokes {
                    push(g(a.0, a.1), g(b.0, b.1), ink);
                }
                if let Some(d) = edge {
                    // Pinned: a chevron points the way to the light.
                    let tip = [m[0] + d[0] * (r + 7. * pt), m[1] + d[1] * (r + 7. * pt)];
                    let back = [tip[0] - d[0] * 4. * pt, tip[1] - d[1] * 4. * pt];
                    let n = [-d[1] * 4. * pt, d[0] * 4. * pt];
                    push(tip, [back[0] + n[0], back[1] + n[1]], ink);
                    push(tip, [back[0] - n[0], back[1] - n[1]], ink);
                }
            }
        }
        let items = self.studio.movable();
        let Some(pivot) = State::pivot(&items) else {
            return lines;
        };
        let pivot = drag.map_or(pivot, |d| add(d.pivot, d.offset));
        if matches!(
            self.studio.tool,
            Tool::Orbit | Tool::Pan | Tool::Zoom | Tool::Light
        ) {
            return lines;
        }
        let Some((c, _)) = eye.project(pivot) else {
            return lines;
        };
        let axis_px = t.metrics.rotation_stem as f64 * 4. / frame.scale;
        let len = axis_px * eye.units_per_pixel(pivot);
        let colors = [
            t.colors.scope_red,
            t.colors.scope_green,
            t.colors.scope_blue,
        ]
        .map(rgb);
        let half = t.metrics.gizmo_handle as f64 / 2. / frame.scale;
        let square = |p: [f64; 2], color, push: &mut dyn FnMut([f64; 2], [f64; 2], [f32; 4])| {
            let q = [
                [p[0] - half, p[1] - half],
                [p[0] + half, p[1] - half],
                [p[0] + half, p[1] + half],
                [p[0] - half, p[1] + half],
            ];
            for i in 0..4 {
                push(q[i], q[(i + 1) % 4], color);
            }
        };
        let active = drag.map(|d| d.handle);
        let basis = drag.map_or_else(|| self.studio.basis(), |d| d.basis);
        let ink = rgb(t.colors.text);
        let hover = self.studio.hover.filter(|_| drag.is_none());
        let tint = |h: Handle, c: [f32; 4]| if hover == Some(h) { ink } else { c };
        // Each axis carries its letter beyond the tip, so colour isn't the
        // only cue (§5), drawn as strokes at the minimum type size.
        let letter = |i: usize, tip: [f64; 2], from: [f64; 2], color, push: &mut dyn FnMut([f64; 2], [f64; 2], [f32; 4])| {
            let (dx, dy) = (tip[0] - from[0], tip[1] - from[1]);
            let l = dx.hypot(dy).max(1e-6);
            let g = t.typography.minimum as f64 / frame.scale;
            let o = [tip[0] + dx / l * g * 1.2, tip[1] + dy / l * g * 1.2];
            let (hw, hh) = (g * 0.3, g * 0.4);
            let p = |x: f64, y: f64| [o[0] + x * hw, o[1] + y * hh];
            match i {
                0 => {
                    push(p(-1., -1.), p(1., 1.), color);
                    push(p(1., -1.), p(-1., 1.), color);
                }
                1 => {
                    push(p(-1., -1.), p(0., 0.), color);
                    push(p(1., -1.), p(0., 0.), color);
                    push(p(0., 0.), p(0., 1.), color);
                }
                _ => {
                    push(p(-1., -1.), p(1., -1.), color);
                    push(p(1., -1.), p(-1., 1.), color);
                    push(p(-1., 1.), p(1., 1.), color);
                }
            }
        };
        match self.studio.tool {
            Tool::Move | Tool::Scale => {
                if self.studio.tool == Tool::Move {
                    // Plane squares between each pair of axes, in the colour
                    // of the axis they hold still, at pasteboard opacity.
                    for k in 0..3 {
                        if active.is_some_and(|h| h != Handle::Pair(k)) {
                            continue;
                        }
                        let (a, b) = (basis[(k + 1) % 3], basis[(k + 2) % 3]);
                        let corner = |u: f64, v: f64| {
                            eye.project(add(pivot, add(mul(a, len * u), mul(b, len * v))))
                                .map(|p| p.0)
                        };
                        let q = [
                            corner(0.22, 0.22),
                            corner(0.42, 0.22),
                            corner(0.42, 0.42),
                            corner(0.22, 0.42),
                        ];
                        let mut color = tint(Handle::Pair(k), colors[k]);
                        if hover != Some(Handle::Pair(k)) {
                            color[3] = t.metrics.pasteboard_opacity;
                        }
                        if q.iter().all(Option::is_some) {
                            for e in 0..4 {
                                push(q[e].unwrap(), q[(e + 1) % 4].unwrap(), color);
                            }
                        }
                    }
                }
                for (i, axis) in basis.iter().enumerate() {
                    if active.is_some_and(|h| h != Handle::Axis(i)) {
                        continue;
                    }
                    let Some((tip, _)) = eye.project(add(pivot, mul(*axis, len))) else {
                        continue;
                    };
                    let color = tint(Handle::Axis(i), colors[i]);
                    push(c, tip, color);
                    letter(i, tip, c, color, &mut push);
                    if self.studio.tool == Tool::Move {
                        // Arrowhead: two short strokes back from the tip.
                        let (dx, dy) = (tip[0] - c[0], tip[1] - c[1]);
                        let l = dx.hypot(dy).max(1e-6);
                        let (ux, uy) = (dx / l, dy / l);
                        let s = half * 2.;
                        push(
                            tip,
                            [
                                tip[0] - ux * s - uy * s * 0.6,
                                tip[1] - uy * s + ux * s * 0.6,
                            ],
                            color,
                        );
                        push(
                            tip,
                            [
                                tip[0] - ux * s + uy * s * 0.6,
                                tip[1] - uy * s - ux * s * 0.6,
                            ],
                            color,
                        );
                    } else {
                        square(tip, color, &mut push);
                    }
                }
                if self.studio.tool == Tool::Scale && active.is_none_or(|h| h == Handle::Uniform) {
                    square(c, tint(Handle::Uniform, accent), &mut push);
                }
            }
            Tool::Rotate => {
                let pivot_depth = eye.project(pivot).map_or(0., |p| p.1);
                for i in 0..3 {
                    if active.is_some_and(|h| h != Handle::Ring(i)) {
                        continue;
                    }
                    let pts = ring(pivot, &basis, i, len, 28);
                    let color = tint(Handle::Ring(i), colors[i]);
                    for k in 0..pts.len() - 1 {
                        let (Some((a, za)), Some((b, zb))) =
                            (eye.project(pts[k]), eye.project(pts[k + 1]))
                        else {
                            continue;
                        };
                        // The back half hides, except the ring being dragged.
                        if active.is_none() && (za + zb) / 2. > pivot_depth + len * 0.05 {
                            continue;
                        }
                        push(a, b, color);
                    }
                    // The letter sits beyond the ring's front-most point on
                    // the screen-right side.
                    if active.is_none()
                        && let Some((p, _)) = eye.project(add(pivot, mul(basis[(i + 1) % 3], len)))
                    {
                        letter(i, p, c, color, &mut push);
                    }
                }
            }
            _ => {}
        }
        lines
    }
    /// The part of the composition the viewer shows, in composition pixels
    /// (`[x0, y0, x1, y1]`): all of it at Fit, less when the editor view
    /// covers the panel or the image is zoomed past it.
    pub(super) fn studio_visible(&self, frame: ViewFrame) -> [f64; 4] {
        let b = self.bounds;
        if f32::from(b.size.width) <= 0. || f32::from(b.size.height) <= 0. {
            return [0., 0., frame.size[0], frame.size[1]];
        }
        let x0 = f32::from(b.origin.x - frame.origin.x) as f64 / frame.scale;
        let y0 = f32::from(b.origin.y - frame.origin.y) as f64 / frame.scale;
        let x1 = x0 + f32::from(b.size.width) as f64 / frame.scale;
        let y1 = y0 + f32::from(b.size.height) as f64 / frame.scale;
        [
            x0.max(0.),
            y0.max(0.),
            x1.min(frame.size[0]),
            y1.min(frame.size[1]),
        ]
    }
    /// A light's ◆ target or marker under the point. They come before the
    /// transform handles, as Electron's markers do.
    fn studio_light_handle(&self, at: [f64; 2], frame: ViewFrame) -> Option<Handle> {
        if !matches!(self.studio.tool, Tool::Move | Tool::Rotate | Tool::Scale) {
            return None;
        }
        let scene = self.studio.scene.as_ref()?;
        let eye = self.studio.eye()?;
        let pt = 1. / frame.scale;
        let lights = || {
            scene
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| i.role == Role::Light && !i.locked)
        };
        for (index, item) in lights() {
            if self.studio.selected.contains(&item.id)
                && item.light.as_deref() != Some("point")
                && let Some((p, _)) = item.target.and_then(|t| eye.project(t))
                && (at[0] - p[0]).hypot(at[1] - p[1]) <= 8. * pt
            {
                return Some(Handle::Target(index));
            }
        }
        let vis = self.studio_visible(frame);
        let mut best: Option<(f64, Handle)> = None;
        for (index, item) in lights() {
            let Some((m, edge)) = marker_place(&eye, item.pivot, vis, (MARKER + 6.) * pt) else {
                continue;
            };
            let d = (at[0] - m[0]).hypot(at[1] - m[1]);
            // Selected markers win a tie, then the nearest.
            let rank = d - if self.studio.selected.contains(&item.id) {
                4. * pt
            } else {
                0.
            };
            if d <= (MARKER + 3.) * pt && best.is_none_or(|(b, _)| rank < b) {
                best = Some((rank, Handle::Marker(index, edge.is_some())));
            }
        }
        best.map(|(_, h)| h)
    }
    /// Which handle is under a composition point.
    fn studio_handle(&self, at: [f64; 2], frame: ViewFrame) -> Option<Handle> {
        if let Some(h) = self.studio_light_handle(at, frame) {
            return Some(h);
        }
        let eye = self.studio.eye()?;
        let items = self.studio.movable();
        let pivot = State::pivot(&items)?;
        let (c, _) = eye.project(pivot)?;
        let reach = self.tokens.metrics.hit as f64 / 2. / frame.scale;
        let axis_px = self.tokens.metrics.rotation_stem as f64 * 4. / frame.scale;
        let len = axis_px * eye.units_per_pixel(pivot);
        let mut best: Option<(f64, Handle)> = None;
        let mut take = |d: f64, h| {
            if d <= reach && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, h));
            }
        };
        let basis = self.studio.basis();
        match self.studio.tool {
            Tool::Move | Tool::Scale => {
                if self.studio.tool == Tool::Scale {
                    take((at[0] - c[0]).hypot(at[1] - c[1]) * 0.9, Handle::Uniform);
                }
                if self.studio.tool == Tool::Move {
                    for k in 0..3 {
                        let (a, b) = (basis[(k + 1) % 3], basis[(k + 2) % 3]);
                        if let Some((m, _)) =
                            eye.project(add(pivot, add(mul(a, len * 0.32), mul(b, len * 0.32))))
                        {
                            // Inside its square counts as on it.
                            take((at[0] - m[0]).hypot(at[1] - m[1]) * 0.8, Handle::Pair(k));
                        }
                    }
                }
                for (i, axis) in basis.iter().enumerate() {
                    if let Some((tip, _)) = eye.project(add(pivot, mul(*axis, len))) {
                        take(dist_segment(at, c, tip), Handle::Axis(i));
                    }
                }
            }
            Tool::Rotate => {
                for i in 0..3 {
                    let pts = ring(pivot, &basis, i, len, 28);
                    for k in 0..pts.len() - 1 {
                        if let (Some((a, _)), Some((b, _))) =
                            (eye.project(pts[k]), eye.project(pts[k + 1]))
                        {
                            take(dist_segment(at, a, b), Handle::Ring(i));
                        }
                    }
                }
            }
            _ => {}
        }
        best.map(|(_, h)| h).or_else(|| {
            // Inside a selected object's projected box: move in the view plane.
            (self.studio.tool == Tool::Move
                && items.iter().any(|i| {
                    let ray = eye.ray(at);
                    let only = Scene {
                        items: vec![i.clone()],
                        ..(**self.studio.scene.as_ref().unwrap()).clone()
                    };
                    only.pick(&ray).is_some()
                }))
            .then_some(Handle::Plane)
        })
    }
    /// A press in the viewer while the composition has 3D layers. `true`
    /// when the 3D layer took it.
    /// `view_tool` is the window's Hand (`Some(false)`) or Zoom (`Some(true)`)
    /// tool: in a 3D composition they move the editor view, never the shot.
    pub(super) fn studio_down(
        &mut self,
        event: &MouseDownEvent,
        at: [f64; 2],
        frame: ViewFrame,
        view_tool: Option<bool>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.studio.active() {
            return false;
        }
        let middle = event.button == MouseButton::Middle;
        let alt = event.modifiers.alt;
        let nav = if middle || alt || view_tool == Some(false) {
            Some(if event.modifiers.shift {
                Nav::Pan
            } else {
                Nav::Orbit
            })
        } else if view_tool == Some(true) {
            Some(Nav::Dolly)
        } else {
            match self.studio.tool {
                Tool::Orbit => Some(Nav::Orbit),
                Tool::Pan => Some(Nav::Pan),
                Tool::Zoom => Some(Nav::Dolly),
                _ => None,
            }
        };
        if let Some(mode) = nav {
            self.studio_go_free(cx);
            let start = self
                .studio
                .view()
                .unwrap_or(self.studio.scene.as_ref().unwrap().shot);
            self.studio.nav = Some(NavDrag {
                mode,
                start,
                origin: event.position,
            });
            self.cursor = if mode == Nav::Dolly {
                CursorStyle::ResizeUpDown
            } else {
                CursorStyle::ClosedHand
            };
            cx.notify();
            return true;
        }
        let Some(eye) = self.studio.eye() else {
            return false;
        };
        if self.studio.tool == Tool::Light && self.studio.light_aim == "orbit" {
            // Orbit: drag the selected light around what it lights.
            let light = self.studio.scene.as_ref().and_then(|s| {
                s.items
                    .iter()
                    .find(|i| {
                        i.role == Role::Light && self.studio.selected.contains(&i.id) && !i.locked
                    })
                    .cloned()
            });
            if let Some(light) = light
                && let Some(target) = light.target
            {
                self.studio.light_orbit = Some(LightOrbit {
                    id: light.id,
                    start: View {
                        position: light.pivot,
                        target,
                        zoom: 1.,
                    },
                    origin: event.position,
                    active: false,
                    time: self.host.time(cx),
                });
                self.cursor = CursorStyle::ClosedHand;
                cx.notify();
                return true;
            }
            self.host.report("Select a light to orbit it.".into(), cx);
            return true;
        }
        if self.studio.tool == Tool::Light {
            // The engine's surface (models, terrain) with its true normal;
            // the box pick when no frame can answer.
            let uv = [
                (at[0] / frame.size[0]) as f32,
                (at[1] / frame.size[1]) as f32,
            ];
            if let Some(rx) = self.host.pick_surface(uv, cx) {
                cx.spawn(async move |this, cx| {
                    let reply = rx.recv().await;
                    let _ = this.update(cx, |s, cx| match reply {
                        Ok(Ok(reply)) => match reply.hit {
                            Some(h) => {
                                let f = |v: [f32; 3]| v.map(f64::from);
                                s.studio_light_at(eye, f(h.point), normalize(f(h.normal)), cx)
                            }
                            None => s
                                .host
                                .report("Click a model to put light there.".into(), cx),
                        },
                        _ => s.studio_light(eye, at, cx),
                    });
                })
                .detach();
            } else {
                self.studio_light(eye, at, cx);
            }
            return true;
        }
        if let Some(handle) = self.studio_handle(at, frame) {
            let (handle, items) = match handle {
                Handle::Marker(index, _) | Handle::Target(index) => {
                    let Some(item) = self
                        .studio
                        .scene
                        .as_ref()
                        .and_then(|s| s.items.get(index).cloned())
                    else {
                        return true;
                    };
                    let shift = event.modifiers.shift;
                    if shift || !self.studio.selected.contains(&item.id) {
                        self.studio_select(item.id.clone(), shift, cx);
                    }
                    // ⇧ only toggles the light; a marker pinned to the edge
                    // only selects (its light is out of view).
                    if shift || matches!(handle, Handle::Marker(_, true)) {
                        return true;
                    }
                    let handle = if matches!(handle, Handle::Marker(..)) {
                        Handle::Plane
                    } else {
                        handle
                    };
                    (handle, vec![item])
                }
                h => (h, self.studio.movable()),
            };
            let pivot = State::pivot(&items).unwrap_or([0.; 3]);
            let axis_px = self.tokens.metrics.rotation_stem as f64 * 4. / frame.scale;
            self.studio.drag = Some(Drag {
                handle,
                origin: at,
                screen: event.position,
                eye,
                pivot,
                length: axis_px * eye.units_per_pixel(pivot),
                items,
                active: false,
                time: self.host.time(cx),
                offset: [0.; 3],
                basis: self.studio.basis(),
            });
            self.studio.hover = None;
            return true;
        }
        // Click to select. Lights have no surface, so their markers are
        // tested here; everything else asks the engine's surface query, with
        // the box pick as the fallback while no frame can answer.
        let scene = self.studio.scene.clone().unwrap();
        let shift = event.modifiers.shift;
        let lights = Scene {
            items: scene
                .items
                .iter()
                .filter(|i| i.role == Role::Light)
                .cloned()
                .collect(),
            ..(*scene).clone()
        };
        if let Some(hit) = lights.pick(&eye.ray(at)).filter(|h| !locked(&scene, &h.id)) {
            self.studio_select(hit.id, shift, cx);
            return true;
        }
        // A double-click on an imported model picks the part under the
        // pointer (viewer.ts dblclick → scene3d.pickPart).
        let part = event.click_count == 2 && !shift;
        let uv = [
            (at[0] / frame.size[0]) as f32,
            (at[1] / frame.size[1]) as f32,
        ];
        if let Some(rx) = self.host.pick_surface(uv, cx) {
            cx.spawn(async move |this, cx| {
                let reply = rx.recv().await;
                let _ = this.update(cx, |s, cx| {
                    let hit = match reply {
                        Ok(Ok(reply)) => Ok(reply.hit.map(|h| h.layer)),
                        // A stale frame: the box pick answers instead.
                        _ => Err(s.studio.scene.clone().and_then(|sc| sc.pick(&eye.ray(at)))),
                    };
                    // A locked layer isn't picked (the click goes through it).
                    let scene = s.studio.scene.clone();
                    let unlocked = |id: String| {
                        (!scene.as_ref().is_some_and(|sc| locked(sc, &id))).then_some(id)
                    };
                    match hit {
                        Ok(Some(layer)) => match unlocked(layer) {
                            Some(layer) if part => s.studio_part(layer, eye, at, cx),
                            Some(layer) => s.studio_select(layer, shift, cx),
                            None if !shift => {
                                s.queue.push_back(Job::Select(None));
                                s.pump(cx);
                            }
                            None => {}
                        },
                        Ok(None) if !shift => {
                            s.queue.push_back(Job::Select(None));
                            s.pump(cx);
                        }
                        Ok(None) => {}
                        Err(hit) => {
                            if let Some(id) = hit.and_then(|h| unlocked(h.id)) {
                                if part {
                                    s.studio_part(id, eye, at, cx);
                                } else {
                                    s.studio_select(id, shift, cx);
                                }
                            }
                        }
                    }
                });
            })
            .detach();
            return true;
        }
        let Some(hit) = scene.pick(&eye.ray(at)).filter(|h| !locked(&scene, &h.id)) else {
            return false;
        };
        if part {
            self.studio_part(hit.id, eye, at, cx);
        } else {
            self.studio_select(hit.id, shift, cx);
        }
        true
    }
    /// Double-click on a 3D layer: an imported model's part under the
    /// pointer becomes the focus of Properties (its material slot opens and
    /// is marked), with the model the only selected layer. Anything else, or
    /// a miss on the model's triangles, is an ordinary click.
    fn studio_part(&mut self, id: String, eye: Eye, at: [f64; 2], cx: &mut Context<Self>) {
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        let assets = self.host.scene3d_assets(cx);
        let time = self.host.time(cx);
        let ray = eye.ray(at);
        let layer = id.clone();
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move {
                    let p = kernel.published(&project)?;
                    let assets = assets?;
                    pm_panel_studio3d::viewport::pick_part(
                        &p.snapshot,
                        time,
                        &layer,
                        &ray,
                        &*assets,
                    )
                    .ok()
                    .flatten()
                })
                .await;
            let _ = this.update(cx, |s, cx| match picked {
                Some(hit) => {
                    if s.studio.selected != [id.clone()] {
                        s.studio_select(id.clone(), false, cx);
                    }
                    s.host.focus_part(&id, Some(hit.slot as u64), cx);
                }
                None => s.studio_select(id, false, cx),
            });
        })
        .detach();
    }
    /// Select a layer the pointer found; ⇧ adds or removes it.
    fn studio_select(&mut self, id: String, shift: bool, cx: &mut Context<Self>) {
        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        let ids = if shift && self.studio.selected.contains(&id) {
            self.studio
                .selected
                .iter()
                .filter(|s| **s != id)
                .cloned()
                .collect()
        } else {
            vec![id.clone()]
        };
        let add = shift && !self.studio.selected.contains(&id);
        cx.background_executor()
            .spawn(async move {
                kernel
                    .call(
                        &project,
                        Request::new(Caller::native(), SelectionCall::Layers { ids, add }),
                    )
                    .await;
            })
            .detach();
    }
    /// Light tool: aim the selected light at the point, or add a Softbox
    /// whose highlight lands there (Reflect, §6).
    fn studio_light(&mut self, eye: Eye, at: [f64; 2], cx: &mut Context<Self>) {
        let Some(scene) = self.studio.scene.clone() else {
            return;
        };
        let models = Scene {
            items: scene
                .items
                .iter()
                .filter(|i| i.role == Role::Object)
                .cloned()
                .collect(),
            ..(*scene).clone()
        };
        let Some(hit) = models.pick(&eye.ray(at)) else {
            self.host
                .report("Click a model to put light there.".into(), cx);
            return;
        };
        self.studio_light_at(eye, hit.point, hit.normal, cx);
    }
    fn studio_light_at(&mut self, eye: Eye, point: Vec3, normal: Vec3, cx: &mut Context<Self>) {
        let Some(scene) = self.studio.scene.clone() else {
            return;
        };
        struct At {
            point: Vec3,
            normal: Vec3,
        }
        let hit = At { point, normal };
        let viewer = eye.pose.position;
        let time = self.host.time(cx);
        // Add armed adds even with a light selected; otherwise the selected
        // light is aimed.
        let light = scene
            .items
            .iter()
            .find(|i| i.role == Role::Light && self.studio.selected.contains(&i.id) && !i.locked)
            .filter(|_| !self.studio.light_add);
        let kind = match self.studio.light_kind.as_str() {
            k @ ("spot" | "point" | "sun") => k,
            _ => "area",
        };
        let name = match kind {
            "spot" => "Spotlight",
            "point" => "Bulb",
            "sun" => "Sun",
            _ => "Softbox",
        };
        let comp = None;
        // A mirror placement can land under the ground (a side face seen from
        // above); a light there lights nothing, so it stays above the floor.
        // Values are rounded so the transform wells read cleanly.
        let place = |p: Vec3| -> Vec3 {
            let r = |v: f64| (v * 100.).round() / 100.;
            [r(p[0]), r(p[1].max(hit.point[1].max(0.) + 0.5)), r(p[2])]
        };
        let (label, args, select) = if let Some(light) = light {
            let distance = length(sub(light.pivot, light.target.unwrap_or(hit.point))).max(1.);
            // Direct faces the light at the point, along the surface; Reflect
            // places it so its highlight lands there.
            let placed = if self.studio.light_aim == "direct" {
                Ok(add(hit.point, mul(normalize(hit.normal), distance)))
            } else {
                pm_scene3d::light_tool::highlight_placement(hit.point, hit.normal, viewer, distance)
            };
            let Ok(p) = placed else {
                return;
            };
            let p = place(p);
            (
                "Aim Light",
                serde_json::json!({"operation":"update_light","target":light.id,"patch":{"p":{
                    "x":p[0],"y":p[1],"z":p[2],
                    "targetX":(hit.point[0]*100.).round()/100.,"targetY":(hit.point[1]*100.).round()/100.,"targetZ":(hit.point[2]*100.).round()/100.}}}),
                None,
            )
        } else {
            let distance = (length(sub(viewer, hit.point)) * 0.6).clamp(1.5, 12.);
            let Ok(p) = pm_scene3d::light_tool::highlight_placement(
                hit.point, hit.normal, viewer, distance,
            ) else {
                return;
            };
            let p = place(p);
            let id = pm_kernel::new_id("L");
            let size = ((distance * 0.6) * 10.).round() / 10.;
            let mut light = serde_json::json!({"type":kind,"p":{
                "x":p[0],"y":p[1],"z":p[2],
                "targetX":(hit.point[0]*100.).round()/100.,"targetY":(hit.point[1]*100.).round()/100.,"targetZ":(hit.point[2]*100.).round()/100.}});
            if kind == "area" {
                light["p"]["width"] = serde_json::json!(size);
                light["p"]["height"] = serde_json::json!(((size * 0.62) * 10.).round() / 10.);
                light["p"]["intensity"] = serde_json::json!(6);
            }
            if self.studio.light_add {
                // Add is one click.
                self.studio_ui("studioLightAdd", "off", cx);
            }
            (
                match kind {
                    "spot" => "Add Spotlight",
                    "point" => "Add Bulb",
                    "sun" => "Add Sun",
                    _ => "Add Softbox",
                },
                serde_json::json!({"operation":"add_light","id":id,"name":name,"light":light}),
                Some(id),
            )
        };
        let command: Command = Scene3dCommand::Apply {
            comp,
            args: pm_scene3d::to_model(&args),
            selection: vec![],
            time,
            autokey: false,
        }
        .into();
        self.queue
            .push_back(Job::Transact(label, vec![command], select));
        self.pump(cx);
    }
    /// Pointer motion while a 3D press is held. `true` when it was ours.
    pub(super) fn studio_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        if let Some(lo) = self.studio.light_orbit.as_mut() {
            let d = event.position - lo.origin;
            let (dx, dy) = (f32::from(d.x) as f64, f32::from(d.y) as f64);
            if !lo.active {
                if dx.hypot(dy) < self.tokens.metrics.drag_threshold as f64 {
                    return true;
                }
                lo.active = true;
                self.queue.push_back(Job::Begin("Orbit Light"));
            }
            let fine = if event.modifiers.shift { 0.1 } else { 1. };
            let pose = orbit(lo.start, dx * fine, dy * fine);
            let r = |v: f64| (v * 100.).round() / 100.;
            let command: Command = Scene3dCommand::Apply {
                comp: None,
                args: pm_scene3d::to_model(&serde_json::json!({"operation":"update_light","target":lo.id,
                    "patch":{"p":{"x":r(pose.position[0]),"y":r(pose.position[1]),"z":r(pose.position[2])}}})),
                selection: vec![],
                time: lo.time,
                autokey: false,
            }
            .into();
            self.queue.push_back(Job::Step(vec![command], true));
            self.pump(cx);
            return true;
        }
        if let Some(nav) = &self.studio.nav {
            let (Some(scene), Some(frame)) =
                (self.studio.scene.clone(), self.host.frame(self.bounds, cx))
            else {
                return true;
            };
            let d = event.position - nav.origin;
            let (dx, dy) = (f32::from(d.x) as f64, f32::from(d.y) as f64);
            let fine = if event.modifiers.shift && nav.mode != Nav::Pan {
                0.1
            } else {
                1.
            };
            let eye = scene.eye(Some(nav.start));
            let pose = match nav.mode {
                Nav::Orbit => orbit(nav.start, dx * fine, dy * fine),
                Nav::Pan => {
                    let (right, up) = eye.right_up();
                    let height = frame.size[1] * frame.scale;
                    pan(nav.start, dx, dy, scene.lens, height, right, up)
                }
                Nav::Dolly => dolly(nav.start, dy * fine, scene.lens),
            };
            self.studio.free = Some(pose);
            self.studio_push_view(cx);
            self.draw(cx);
            return true;
        }
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            return self.studio.drag.is_some();
        };
        let Some(drag) = self.studio.drag.as_mut() else {
            return false;
        };
        let moved = event.position - drag.screen;
        if !drag.active {
            if f32::from(moved.x).hypot(f32::from(moved.y)) < self.tokens.metrics.drag_threshold {
                return true;
            }
            drag.active = true;
            let plural = drag.items.len() > 1;
            let lights = drag.items.iter().all(|i| i.role == Role::Light);
            let scale = self.studio.tool == Tool::Scale;
            self.queue
                .push_back(Job::Begin(match (drag.handle, plural) {
                    (Handle::Target(_), _) => "Aim Light",
                    (Handle::Ring(_), _) if lights => "Aim Light",
                    (Handle::Plane | Handle::Axis(_) | Handle::Pair(_), false)
                        if lights && !scale =>
                    {
                        "Move Light"
                    }
                    (Handle::Plane | Handle::Axis(_) | Handle::Pair(_), true)
                        if lights && !scale =>
                    {
                        "Move Lights"
                    }
                    (Handle::Ring(_), false) => "Rotate 3D Layer",
                    (Handle::Ring(_), true) => "Rotate 3D Layers",
                    (Handle::Uniform, false) | (Handle::Axis(_), false)
                        if self.studio.tool == Tool::Scale =>
                    {
                        "Scale 3D Layer"
                    }
                    (Handle::Uniform, true) | (Handle::Axis(_), true)
                        if self.studio.tool == Tool::Scale =>
                    {
                        "Scale 3D Layers"
                    }
                    (_, false) => "Move 3D Layer",
                    (_, true) => "Move 3D Layers",
                }));
        }
        let at = [
            f32::from(event.position.x - frame.origin.x) as f64 / frame.scale,
            f32::from(event.position.y - frame.origin.y) as f64 / frame.scale,
        ];
        let fine = if event.modifiers.shift { 0.1 } else { 1. };
        let snap = event.modifiers.control || event.modifiers.platform;
        let d = [
            (at[0] - drag.origin[0]) * fine,
            (at[1] - drag.origin[1]) * fine,
        ];
        let eye = drag.eye;
        let tool = self.studio.tool;
        let mut commands = vec![];
        let mut offset = [0.; 3];
        let basis = drag.basis;
        let screen_axis = |i: usize| -> Option<[f64; 2]> {
            let (c, _) = eye.project(drag.pivot)?;
            let (t, _) = eye.project(add(drag.pivot, mul(basis[i], drag.length)))?;
            Some([t[0] - c[0], t[1] - c[1]])
        };
        let along = |i: usize| -> f64 {
            screen_axis(i).map_or(0., |s| {
                let l2 = (s[0] * s[0] + s[1] * s[1]).max(1e-9);
                (d[0] * s[0] + d[1] * s[1]) / l2
            })
        };
        // ⌥ while moving a light carries its target along (Electron), so it
        // keeps aiming the same way.
        let carry = event.modifiers.alt;
        let aim_by = |item: &Item, w: Vec3, commands: &mut Vec<Command>| {
            if let Some(aim) = item.aim {
                for (a, key) in ["light.targetX", "light.targetY", "light.targetZ"]
                    .iter()
                    .enumerate()
                {
                    commands.push(set(&item.id, key, aim[a] + w[a], drag.time));
                }
            }
        };
        for item in &drag.items {
            let id = item.id.as_str();
            let light = item.role == Role::Light;
            match (drag.handle, tool) {
                (Handle::Target(_), _) => {
                    // The aim point slides in the view plane through it.
                    let Some(target) = item.target else {
                        continue;
                    };
                    let upp = eye.units_per_pixel(target);
                    let (r, u) = eye.right_up();
                    let mut w = add(mul(r, d[0] * upp), mul(u, -d[1] * upp));
                    if snap {
                        w = w.map(f64::round);
                    }
                    aim_by(item, w, &mut commands);
                }
                (Handle::Ring(i), _) if light => {
                    // Rotating a light swings its aim about its position: the
                    // renderer aims lights by their target, not their rotation.
                    let (Some(target), Some((c, _))) = (item.target, eye.project(drag.pivot))
                    else {
                        continue;
                    };
                    let a = [drag.origin[0] - c[0], drag.origin[1] - c[1]];
                    let b = [at[0] - c[0], at[1] - c[1]];
                    let mut deg = (a[0] * b[1] - a[1] * b[0])
                        .atan2(a[0] * b[0] + a[1] * b[1])
                        .to_degrees()
                        * fine;
                    let f = normalize(sub(eye.pose.target, eye.pose.position));
                    if dot(basis[i], f) < 0. {
                        deg = -deg;
                    }
                    if snap {
                        deg = (deg / 15.).round() * 15.;
                    }
                    let v = sub(target, item.pivot);
                    let w = sub(add(item.pivot, turn(v, basis[i], -deg)), target);
                    aim_by(item, w, &mut commands);
                }
                (Handle::Axis(i), Tool::Move) => {
                    let mut t = along(i) * drag.length;
                    if snap {
                        t = t.round();
                    }
                    let w = mul(basis[i], t);
                    let c = channel_delta(item, w);
                    for (a, key) in ["position.x", "position.y", "position.z"]
                        .iter()
                        .enumerate()
                    {
                        // An axis the move doesn't touch keeps its keyframes.
                        if w[a].abs() > 1e-9 || a == i {
                            commands.push(set(id, key, item.position[a] + c[a], drag.time));
                        }
                    }
                    if light && carry {
                        aim_by(item, w, &mut commands);
                    }
                    offset = w;
                }
                (Handle::Pair(k), _) => {
                    // Where the pointer's ray meets the plane through the
                    // pivot that holds axis k still.
                    let n = basis[k];
                    let meet = |p: [f64; 2]| -> Option<Vec3> {
                        let ray = eye.ray(p);
                        let den = dot(ray.dir, n);
                        if den.abs() < 1e-6 {
                            return None;
                        }
                        let s = dot(sub(drag.pivot, ray.origin), n) / den;
                        Some(add(ray.origin, mul(ray.dir, s)))
                    };
                    let now = [drag.origin[0] + d[0], drag.origin[1] + d[1]];
                    let (Some(a), Some(b)) = (meet(drag.origin), meet(now)) else {
                        continue;
                    };
                    let mut w = sub(b, a);
                    if snap {
                        w = w.map(f64::round);
                    }
                    let c = channel_delta(item, w);
                    for (a, key) in ["position.x", "position.y", "position.z"]
                        .iter()
                        .enumerate()
                    {
                        commands.push(set(id, key, item.position[a] + c[a], drag.time));
                    }
                    if light && carry {
                        aim_by(item, w, &mut commands);
                    }
                    offset = w;
                }
                (Handle::Plane, _) => {
                    let upp = eye.units_per_pixel(drag.pivot);
                    let (r, u) = eye.right_up();
                    let mut w = add(mul(r, d[0] * upp), mul(u, -d[1] * upp));
                    if snap {
                        w = w.map(f64::round);
                    }
                    let c = channel_delta(item, w);
                    for (i, key) in ["position.x", "position.y", "position.z"]
                        .iter()
                        .enumerate()
                    {
                        commands.push(set(id, key, item.position[i] + c[i], drag.time));
                    }
                    if light && carry {
                        aim_by(item, w, &mut commands);
                    }
                    offset = w;
                }
                (Handle::Ring(i), _) => {
                    let Some((c, _)) = eye.project(drag.pivot) else {
                        continue;
                    };
                    let a = [drag.origin[0] - c[0], drag.origin[1] - c[1]];
                    let b = [at[0] - c[0], at[1] - c[1]];
                    let mut deg = (a[0] * b[1] - a[1] * b[0])
                        .atan2(a[0] * b[0] + a[1] * b[1])
                        .to_degrees()
                        * fine;
                    // Screen angles turn the other way when the axis faces us.
                    let f = normalize(sub(eye.pose.target, eye.pose.position));
                    if dot(basis[i], f) < 0. {
                        deg = -deg;
                    }
                    if snap {
                        deg = (deg / 15.).round() * 15.;
                    }
                    // Composition space is the scene turned 180° about X:
                    // Y and Z turn the other way in its channels.
                    if item.pixels.is_some() && i > 0 {
                        deg = -deg;
                    }
                    let key = ["rotation.x", "rotation.y", "rotation"][i];
                    commands.push(set(id, key, item.rotation[i] - deg, drag.time));
                }
                (Handle::Axis(i), Tool::Scale) => {
                    let mut k = (1. + along(i)).max(0.01);
                    if snap {
                        k = (k * 10.).round() / 10.;
                    }
                    let key = ["scale.x", "scale.y", "scale.z"][i];
                    commands.push(set(id, key, item.scale[i] * k, drag.time));
                }
                (Handle::Uniform, _) => {
                    let mut k = ((d[0] - d[1]) * 0.01).exp();
                    if snap {
                        k = (k * 10.).round() / 10.;
                    }
                    for (i, key) in ["scale.x", "scale.y", "scale.z"].iter().enumerate() {
                        commands.push(set(id, key, item.scale[i] * k.max(0.01), drag.time));
                    }
                }
                _ => {}
            }
        }
        if let Some(drag) = self.studio.drag.as_mut() {
            drag.offset = offset;
        }
        self.queue.push_back(Job::Step(commands, true));
        self.pump(cx);
        self.draw(cx);
        true
    }
    /// Release. `true` when a 3D press was held.
    pub(super) fn studio_up(&mut self, commit: bool, cx: &mut Context<Self>) -> bool {
        if let Some(orbit) = self.studio.light_orbit.take() {
            if orbit.active {
                self.queue.push_back(Job::Finish(commit));
                self.pump(cx);
            }
            self.cursor = CursorStyle::Arrow;
            cx.notify();
            return true;
        }
        if let Some(nav) = self.studio.nav.take() {
            if !commit {
                // Esc returns the view to where the gesture began.
                self.studio.free = Some(nav.start);
                self.studio_push_view(cx);
            }
            self.cursor = CursorStyle::Arrow;
            cx.notify();
            self.draw(cx);
            return true;
        }
        let Some(drag) = self.studio.drag.take() else {
            return false;
        };
        if drag.active {
            self.queue.push_back(Job::Finish(commit));
            self.pump(cx);
        }
        self.draw(cx);
        true
    }
    /// Scrolling in a 3D composition moves the editor view, never the shot:
    /// a trackpad's two-finger scroll orbits, ⇧ pans, and ⌥, ⌘ or a mouse
    /// wheel dollies. The first one leaves the camera for Free View.
    pub(super) fn studio_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        line: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scene) = self.studio.scene.clone() else {
            return false;
        };
        let d = event.delta.pixel_delta(line);
        let (dx, dy) = (f32::from(d.x) as f64, f32::from(d.y) as f64);
        let m = event.modifiers;
        let wheel = matches!(event.delta, ScrollDelta::Lines(_));
        let frame = self.host.frame(self.bounds, cx);
        self.studio_go_free(cx);
        let start = self.studio.view().unwrap_or(scene.shot);
        self.studio.free = Some(
            if wheel || m.alt || m.platform || m.control || self.studio.tool == Tool::Zoom {
                let fine = if m.shift && !wheel { 0.1 } else { 1. };
                dolly(start, -dy * fine, scene.lens)
            } else if m.shift {
                let Some(frame) = frame else {
                    return true;
                };
                let (right, up) = scene.eye(Some(start)).right_up();
                pan(
                    start,
                    dx,
                    dy,
                    scene.lens,
                    frame.size[1] * frame.scale,
                    right,
                    up,
                )
            } else {
                orbit(start, dx, dy)
            },
        );
        self.studio_push_view(cx);
        self.draw(cx);
        true
    }
    /// Pinching in a 3D composition dollies the editor view.
    pub(super) fn studio_pinch(&mut self, delta: f64, cx: &mut Context<Self>) -> bool {
        let Some(scene) = self.studio.scene.clone() else {
            return false;
        };
        self.studio_go_free(cx);
        let start = self.studio.view().unwrap_or(scene.shot);
        // Pinching out moves closer; a positive dolly moves away.
        self.studio.free = Some(dolly(start, -delta * 400., scene.lens));
        self.studio_push_view(cx);
        self.draw(cx);
        true
    }
    /// Tracks the handle under the pointer; `true` when it changed, so the
    /// overlay redraws with that handle in text colour.
    pub(super) fn studio_hover(&mut self, at: [f64; 2], frame: ViewFrame) -> bool {
        let next = if self.studio.active() && !self.studio.busy() {
            self.studio_handle(at, frame).filter(|h| *h != Handle::Plane)
        } else {
            None
        };
        let changed = next != self.studio.hover;
        self.studio.hover = next;
        changed
    }
    /// The cursor promises what a press does over the 3D view.
    pub(super) fn studio_cursor(&self, at: [f64; 2], frame: ViewFrame) -> Option<CursorStyle> {
        if !self.studio.active() {
            return None;
        }
        Some(match self.studio.tool {
            Tool::Orbit | Tool::Pan => CursorStyle::OpenHand,
            Tool::Zoom => CursorStyle::ResizeUpDown,
            Tool::Light => {
                let eye = self.studio.eye()?;
                let scene = self.studio.scene.as_ref()?;
                let hit = scene.pick(&eye.ray(at)).filter(|h| {
                    scene
                        .items
                        .iter()
                        .any(|i| i.id == h.id && i.role == Role::Object)
                });
                if hit.is_some() {
                    CursorStyle::Crosshair
                } else {
                    CursorStyle::Arrow
                }
            }
            _ => match self.studio_handle(at, frame) {
                Some(Handle::Plane | Handle::Marker(_, false)) => CursorStyle::OpenHand,
                Some(Handle::Target(_)) => CursorStyle::Crosshair,
                Some(_) => CursorStyle::PointingHand,
                None => return None,
            },
        })
    }
}
/// A 3D item the viewer must not pick: locked, or not in the scene.
fn locked(scene: &Scene, id: &str) -> bool {
    scene.items.iter().any(|i| i.id == id && i.locked)
}
#[cfg(test)]
mod light_marker_tests {
    use super::{Eye, View, length, marker_place, turn};
    use pm_scene3d::navigation::Lens;
    fn eye() -> Eye {
        Eye {
            pose: View {
                position: [0., 0., 5.],
                target: [0., 0., 0.],
                zoom: 1.,
            },
            lens: Lens::Perspective {
                fov: 40.,
                aspect: 16. / 9.,
                near: 0.01,
                far: 100.,
            },
            width: 640.,
            height: 360.,
        }
    }
    #[test]
    fn a_light_in_view_sits_where_it_projects() {
        let (m, edge) = marker_place(&eye(), [0., 0., 0.], [0., 0., 640., 360.], 10.).unwrap();
        assert!((m[0] - 320.).abs() < 1e-6 && (m[1] - 180.).abs() < 1e-6);
        assert!(edge.is_none());
    }
    #[test]
    fn a_light_outside_or_behind_is_pinned_to_the_visible_edge_pointing_at_it() {
        let vis = [0., 0., 640., 360.];
        // Far to the right: pinned at the right edge, pointing right.
        let (m, edge) = marker_place(&eye(), [50., 0., 0.], vis, 10.).unwrap();
        assert!((m[0] - 630.).abs() < 1e-6, "{m:?}");
        assert!(edge.unwrap()[0] > 0.9);
        // Behind the eye and up: still pinned, pointing up.
        let (m, edge) = marker_place(&eye(), [0., 5., 9.], vis, 10.).unwrap();
        assert!((m[1] - 10.).abs() < 1e-6, "{m:?}");
        assert!(edge.unwrap()[1] < -0.9);
        // A narrower visible area (the editor view covering the panel) pins inside it.
        let (m, _) = marker_place(&eye(), [50., 0., 0.], [100., 0., 500., 360.], 10.).unwrap();
        assert!((m[0] - 490.).abs() < 1e-6, "{m:?}");
    }
    #[test]
    fn rotating_a_lights_aim_keeps_its_distance() {
        let v = [0., 0., -3.];
        let w = turn(v, [0., 1., 0.], 90.);
        assert!((length(w) - 3.).abs() < 1e-9);
        assert!((w[0] + 3.).abs() < 1e-9 && w[2].abs() < 1e-9, "{w:?}");
    }
}
