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
    Pose as View, Vec3, add, dolly, dot, length, mul, normalize, orbit, pan, sub,
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
enum Handle {
    Axis(usize),
    Plane,
    Ring(usize),
    Uniform,
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
/// Points around the ring perpendicular to `axis`, radius `r`.
fn ring(pivot: Vec3, axis: usize, r: f64, n: usize) -> Vec<Vec3> {
    let (u, v) = match axis {
        0 => ([0., 1., 0.], [0., 0., 1.]),
        1 => ([0., 0., 1.], [1., 0., 0.]),
        _ => ([1., 0., 0.], [0., 1., 0.]),
    };
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
        self.nav.is_some() || self.drag.is_some()
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
            .filter(|i| self.studio.selected.contains(&i.id))
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
        match self.studio.tool {
            Tool::Move | Tool::Scale => {
                for (i, axis) in AXES.iter().enumerate() {
                    if active.is_some_and(|h| h != Handle::Axis(i)) {
                        continue;
                    }
                    let Some((tip, _)) = eye.project(add(pivot, mul(*axis, len))) else {
                        continue;
                    };
                    push(c, tip, colors[i]);
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
                            colors[i],
                        );
                        push(
                            tip,
                            [
                                tip[0] - ux * s + uy * s * 0.6,
                                tip[1] - uy * s - ux * s * 0.6,
                            ],
                            colors[i],
                        );
                    } else {
                        square(tip, colors[i], &mut push);
                    }
                }
                if self.studio.tool == Tool::Scale && active.is_none_or(|h| h == Handle::Uniform) {
                    square(c, accent, &mut push);
                }
            }
            Tool::Rotate => {
                let pivot_depth = eye.project(pivot).map_or(0., |p| p.1);
                for i in 0..3 {
                    if active.is_some_and(|h| h != Handle::Ring(i)) {
                        continue;
                    }
                    let pts = ring(pivot, i, len, 28);
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
                        push(a, b, colors[i]);
                    }
                }
            }
            _ => {}
        }
        lines
    }
    /// Which handle is under a composition point.
    fn studio_handle(&self, at: [f64; 2], frame: ViewFrame) -> Option<Handle> {
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
        match self.studio.tool {
            Tool::Move | Tool::Scale => {
                if self.studio.tool == Tool::Scale {
                    take((at[0] - c[0]).hypot(at[1] - c[1]) * 0.9, Handle::Uniform);
                }
                for (i, axis) in AXES.iter().enumerate() {
                    if let Some((tip, _)) = eye.project(add(pivot, mul(*axis, len))) {
                        take(dist_segment(at, c, tip), Handle::Axis(i));
                    }
                }
            }
            Tool::Rotate => {
                for i in 0..3 {
                    let pts = ring(pivot, i, len, 28);
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
    pub(super) fn studio_down(
        &mut self,
        event: &MouseDownEvent,
        at: [f64; 2],
        frame: ViewFrame,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.studio.active() {
            return false;
        }
        let middle = event.button == MouseButton::Middle;
        let alt = event.modifiers.alt;
        let nav = if middle || alt {
            Some(if event.modifiers.shift {
                Nav::Pan
            } else {
                Nav::Orbit
            })
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
        if self.studio.tool == Tool::Light {
            self.studio_light(eye, at, cx);
            return true;
        }
        if let Some(handle) = self.studio_handle(at, frame) {
            let items = self.studio.movable();
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
            });
            return true;
        }
        // Click to select by picking; a miss falls through to 2D picking
        // (which clears the selection on empty canvas).
        let scene = self.studio.scene.clone().unwrap();
        let Some(hit) = scene.pick(&eye.ray(at)) else {
            return false;
        };

        let (kernel, project) = (self.kernel.clone(), self.project.clone());
        let add = event.modifiers.shift;
        let ids = if add && self.studio.selected.contains(&hit.id) {
            self.studio
                .selected
                .iter()
                .filter(|s| **s != hit.id)
                .cloned()
                .collect()
        } else {
            vec![hit.id.clone()]
        };
        let add = add && !self.studio.selected.contains(&hit.id);
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
        true
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
        let viewer = eye.pose.position;
        let time = self.host.time(cx);
        let light = scene
            .items
            .iter()
            .find(|i| i.role == Role::Light && self.studio.selected.contains(&i.id) && !i.locked);
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
            let Ok(p) = pm_scene3d::light_tool::highlight_placement(
                hit.point, hit.normal, viewer, distance,
            ) else {
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
            (
                "Add Softbox",
                serde_json::json!({"operation":"add_light","id":id,"name":"Softbox","light":{"type":"area","p":{
                    "x":p[0],"y":p[1],"z":p[2],
                    "targetX":(hit.point[0]*100.).round()/100.,"targetY":(hit.point[1]*100.).round()/100.,"targetZ":(hit.point[2]*100.).round()/100.,
                    "width":size,"height":((size*0.62)*10.).round()/10.,"intensity":6}}}),
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
            self.queue
                .push_back(Job::Begin(match (drag.handle, plural) {
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
        let screen_axis = |i: usize| -> Option<[f64; 2]> {
            let (c, _) = eye.project(drag.pivot)?;
            let (t, _) = eye.project(add(drag.pivot, mul(AXES[i], drag.length)))?;
            Some([t[0] - c[0], t[1] - c[1]])
        };
        let along = |i: usize| -> f64 {
            screen_axis(i).map_or(0., |s| {
                let l2 = (s[0] * s[0] + s[1] * s[1]).max(1e-9);
                (d[0] * s[0] + d[1] * s[1]) / l2
            })
        };
        for item in &drag.items {
            let id = item.id.as_str();
            match (drag.handle, tool) {
                (Handle::Axis(i), Tool::Move) => {
                    let mut t = along(i) * drag.length;
                    if snap {
                        t = t.round();
                    }
                    let key = ["position.x", "position.y", "position.z"][i];
                    commands.push(set(id, key, item.position[i] + t, drag.time));
                    offset = mul(AXES[i], t);
                }
                (Handle::Plane, _) => {
                    let upp = eye.units_per_pixel(drag.pivot);
                    let (r, u) = eye.right_up();
                    let mut w = add(mul(r, d[0] * upp), mul(u, -d[1] * upp));
                    if snap {
                        w = w.map(f64::round);
                    }
                    for (i, key) in ["position.x", "position.y", "position.z"]
                        .iter()
                        .enumerate()
                    {
                        commands.push(set(id, key, item.position[i] + w[i], drag.time));
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
                    if dot(AXES[i], f) < 0. {
                        deg = -deg;
                    }
                    if snap {
                        deg = (deg / 15.).round() * 15.;
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
    /// ⌥-scroll dollies anywhere; plain scroll dollies with the Zoom tool.
    pub(super) fn studio_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        line: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scene) = self.studio.scene.clone() else {
            return false;
        };
        if !(event.modifiers.alt || self.studio.tool == Tool::Zoom) {
            return false;
        }
        let dy = f32::from(event.delta.pixel_delta(line).y) as f64;
        self.studio_go_free(cx);
        let start = self.studio.view().unwrap_or(scene.shot);
        let fine = if event.modifiers.shift { 0.1 } else { 1. };
        self.studio.free = Some(dolly(start, -dy * fine, scene.lens));
        self.studio_push_view(cx);
        self.draw(cx);
        true
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
                Some(Handle::Plane) => CursorStyle::OpenHand,
                Some(_) => CursorStyle::PointingHand,
                None => return None,
            },
        })
    }
}
