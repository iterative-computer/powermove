//! Viewer controls declared by effects, including local/agent registrations.
use super::*;
use pm_fx_defs::handles::{Handle, Mapping, PointBinding};
use std::collections::BTreeMap;
#[derive(Clone)]
pub(super) struct Control {
    layer: String,
    prefix: String,
    declaration: Handle,
    world: [f64; 6],
    values: BTreeMap<String, f64>,
}
pub(super) struct Drag {
    index: usize,
    part: usize,
    original: Control,
    time: f64,
    pub active: bool,
}
impl Control {
    fn point(&self, p: &PointBinding) -> Option<[f64; 2]> {
        Some([*self.values.get(&p.x)?, *self.values.get(&p.y)?])
    }
    fn points(&self) -> Vec<[f64; 2]> {
        let mut p: Vec<_> = self
            .declaration
            .points()
            .into_iter()
            .filter_map(|p| self.point(p))
            .collect();
        if let Some(c) = p.first().copied() {
            match &self.declaration {
                Handle::Angle { angle, .. } => {
                    let a = self.values.get(angle).copied().unwrap_or(0.).to_radians();
                    p.push([c[0] + 60. * a.cos(), c[1] + 60. * a.sin()]);
                }
                Handle::Radius { radius, .. } => {
                    p.push([c[0] + self.values.get(radius).copied().unwrap_or(0.), c[1]])
                }
                _ => (),
            }
        }
        p.into_iter().map(|p| transform(&self.world, p)).collect()
    }
}
impl GizmoSurface {
    pub(super) fn effects_reread(&mut self, cx: &mut Context<Self>) {
        // A user gesture owns its starting transform and local preview. A late
        // publication (including an agent edit) must not pull the pointer back.
        if self.effect_drag.is_some() {
            return;
        }
        self.effect_controls.clear();
        if self.host.playing(cx) {
            cx.notify();
            return;
        }
        let Some(p) = self.kernel.published(&self.project) else {
            return;
        };
        let Some(root) = p.snapshot.root() else {
            return;
        };
        let time = self.host.time(cx);
        let state = self.kernel.state();
        let mut mirror = QueryMirror::new(&p);
        for id in p.selection.layers() {
            let Some(loc) = pm_kernel::model::find_layer(root, id) else {
                continue;
            };
            let Some(layer) = pm_kernel::model::layer_at(root, &loc) else {
                continue;
            };
            let layer = layer.to_value();
            if pm_kernel::model::list_layers(root, None)
                .iter()
                .any(|l| l.id == *id && l.effectively_locked)
            {
                continue;
            }
            if !mirror.queries().active(None, id, time).unwrap_or(false) {
                continue;
            }
            let Ok(world) = mirror.queries().world_matrix(None, id, time) else {
                continue;
            };
            let mut controls = vec![];
            let mut add = |prefix: String, declarations: Vec<Handle>| {
                for declaration in declarations {
                    let mut keys: Vec<String> = declaration
                        .points()
                        .into_iter()
                        .flat_map(|p| [p.x.clone(), p.y.clone()])
                        .collect();
                    match &declaration {
                        Handle::Angle { angle, .. } => keys.push(angle.clone()),
                        Handle::Radius { radius, .. } => keys.push(radius.clone()),
                        Handle::Line {
                            radial: Some(k), ..
                        } => keys.push(k.clone()),
                        _ => (),
                    }
                    let values = keys
                        .into_iter()
                        .filter_map(|k| {
                            mirror
                                .queries()
                                .evaluate(None, id, &format!("{prefix}{k}"), time)
                                .ok()
                                .and_then(|v| v.as_f64())
                                .map(|v| (k, v))
                        })
                        .collect();
                    let control = Control {
                        layer: id.clone(),
                        prefix: prefix.clone(),
                        declaration,
                        world,
                        values,
                    };
                    if control.points().len() >= control.declaration.points().len() {
                        controls.push(control);
                    }
                }
            };
            if let Some(fx) = layer.get("fx").and_then(Value::as_array) {
                for effect in fx {
                    let Some(eid) = effect.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if !self
                        .host
                        .effect_visible(id, eid, cx)
                        .unwrap_or(effect.get("open").and_then(Value::as_bool) != Some(false))
                    {
                        continue;
                    }
                    let Some(kind) = effect.get("type").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(registration) = state.provider("effects", kind) else {
                        continue;
                    };
                    let Ok(definition) = serde_json::from_str::<pm_fx_defs::Definition>(
                        &pm_model::json::to_canonical_string(&registration.definition),
                    ) else {
                        continue;
                    };
                    // Resolve the saved revision, never bind today's keys to an old implementation.
                    let catalog = pm_fx_defs::catalog::EffectCatalog::new(vec![definition]);
                    let Ok(catalog) = catalog else { continue };
                    let raw: serde_json::Value =
                        serde_json::from_str(&pm_model::json::to_canonical_string(effect))
                            .unwrap_or_default();
                    let Ok(definition) = catalog.instance(kind, raw.get("version")) else {
                        continue;
                    };
                    if let Ok(handles) = pm_fx_defs::handles::parse(
                        definition.metadata.get("viewerHandles"),
                        &definition.params,
                    ) {
                        add(format!("{eid}."), handles);
                    }
                }
            }
            // Shader-layer declarations live beside uniforms. They use exactly
            // the same bindings and gesture path as registered effects.
            if layer.get("type").and_then(Value::as_str) == Some("shader") {
                if let Some(raw) = layer.get("d").and_then(|d| d.get("viewerHandles")) {
                    if let Ok(handles) = serde_json::from_str::<Vec<Handle>>(
                        &pm_model::json::to_canonical_string(raw),
                    ) {
                        add("u.".into(), handles);
                    }
                }
            }
            self.effect_controls.extend(controls);
        }
        cx.notify();
        self.draw(cx);
    }
    #[cfg(test)]
    pub fn effect_points(&self) -> Vec<[f64; 2]> {
        self.effect_controls
            .iter()
            .flat_map(Control::points)
            .collect()
    }
    pub(super) fn effects_hit(&self, at: [f64; 2], scale: f64) -> Option<(usize, usize)> {
        self.effect_controls
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, c)| {
                c.points()
                    .iter()
                    .enumerate()
                    .find(|(_, p)| (p[0] - at[0]).hypot(p[1] - at[1]) * scale <= 9.)
                    .map(|(j, _)| (i, j))
            })
    }
    pub(super) fn effects_down(
        &mut self,
        event: &MouseDownEvent,
        frame: ViewFrame,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.controls || self.host.playing(cx) {
            return false;
        }
        let Some((index, part)) =
            self.effects_hit(self.to_comp(event.position, frame), frame.scale)
        else {
            return false;
        };
        self.effect_drag = Some(Drag {
            index,
            part,
            original: self.effect_controls[index].clone(),
            time: self.host.time(cx),
            active: false,
        });
        self.set_cursor(CursorStyle::Crosshair, cx);
        true
    }
    pub(super) fn effects_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let Some(mut drag) = self.effect_drag.take() else {
            return false;
        };
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            self.effect_drag = Some(drag);
            return true;
        };
        let Some(mut at) = Mapping(drag.original.world).local(self.to_comp(event.position, frame))
        else {
            self.effect_drag = Some(drag);
            return true;
        };
        let original = &drag.original;
        let bindings = original.declaration.points();
        if event.modifiers.shift && bindings.len() == 2 {
            if let Some(other) = original.point(bindings[1 - drag.part]) {
                at = pm_fx_defs::handles::constrain(at, other);
            }
        }
        let mut updates = vec![];
        match &original.declaration {
            Handle::Angle { center, angle } if drag.part == 1 => {
                let c = original.point(center).unwrap();
                let mut a = (at[1] - c[1]).atan2(at[0] - c[0]).to_degrees();
                if event.modifiers.shift {
                    a = (a / 45.).round() * 45.;
                }
                updates.push((angle.clone(), a));
            }
            Handle::Radius { center, radius } if drag.part == 1 => {
                let c = original.point(center).unwrap();
                updates.push((radius.clone(), (at[0] - c[0]).hypot(at[1] - c[1])));
            }
            _ => {
                if let Some(b) = bindings.get(drag.part) {
                    updates.push((b.x.clone(), at[0]));
                    updates.push((b.y.clone(), at[1]));
                }
            }
        }
        if !drag.active {
            drag.active = true;
            self.queue.push_back(Job::Begin("Edit effect handle"));
        }
        let commands = updates
            .iter()
            .map(|(k, v)| {
                let mut set = SetProperty::new(
                    &original.layer,
                    format!("{}{k}", original.prefix),
                    Value::Number(*v),
                    drag.time,
                );
                set.preserve_hand_edits = false;
                set.mark_intent = true;
                Command::Property(PropertyCommand::Set(set))
            })
            .collect();
        for (k, v) in updates {
            self.effect_controls[drag.index].values.insert(k, v);
        }
        self.effect_drag = Some(drag);
        self.queue.push_back(Job::Step(commands, true));
        self.pump(cx);
        self.draw(cx);
        cx.notify();
        true
    }
    pub(super) fn effects_lines(&self, frame: ViewFrame) -> Vec<OverlayLine> {
        let mut lines = vec![];
        let mut push = |a: [f64; 2], b: [f64; 2], color| {
            lines.push(OverlayLine {
                ends: [
                    a[0] / frame.size[0],
                    a[1] / frame.size[1],
                    b[0] / frame.size[0],
                    b[1] / frame.size[1],
                ]
                .map(|n| n as f32),
                color,
            })
        };
        for c in &self.effect_controls {
            let p = c.points();
            if p.len() < 2 {
                continue;
            }
            if matches!(c.declaration, Handle::Rect { .. }) {
                let local: Vec<_> = c
                    .declaration
                    .points()
                    .iter()
                    .filter_map(|p| c.point(p))
                    .collect();
                let a = local[0];
                let b = local[1];
                let corners = [a, [b[0], a[1]], b, [a[0], b[1]]].map(|p| transform(&c.world, p));
                for i in 0..4 {
                    push(corners[i], corners[(i + 1) % 4], [1.; 4]);
                }
            } else {
                push(p[0], p[1], [1.; 4]);
            }
            if let Handle::Line {
                radial: Some(k), ..
            } = &c.declaration
            {
                if c.values.get(k).copied().unwrap_or(0.) > 0.5 {
                    let a = c.point(c.declaration.points()[0]).unwrap();
                    let b = c.point(c.declaration.points()[1]).unwrap();
                    let r = (a[0] - b[0]).hypot(a[1] - b[1]);
                    let circle: Vec<_> = (0..=64)
                        .map(|i| {
                            let t = i as f64 * std::f64::consts::TAU / 64.;
                            transform(&c.world, [a[0] + r * t.cos(), a[1] + r * t.sin()])
                        })
                        .collect();
                    for pair in circle.windows(2) {
                        push(pair[0], pair[1], [1., 1., 1., 0.6]);
                    }
                }
            }
        }
        lines
    }
    pub(super) fn effects_paint(&self, frame: ViewFrame, window: &mut Window, cx: &App) {
        if !self.controls || self.host.playing(cx) || self.text_session().is_some() {
            return;
        }
        for line in self.effects_lines(frame) {
            let [x0, y0, x1, y1] = line.ends;
            let at = |x: f32, y: f32| {
                point(
                    frame.origin.x + px(x * frame.size[0] as f32 * frame.scale as f32),
                    frame.origin.y + px(y * frame.size[1] as f32 * frame.scale as f32),
                )
            };
            for (width, color) in [(3., rgb(0x252525)), (1., rgb(0xffffff))] {
                let mut path = PathBuilder::stroke(px(width));
                path.move_to(at(x0, y0));
                path.line_to(at(x1, y1));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        }
        for c in &self.effect_controls {
            for p in c.points() {
                let at = point(
                    frame.origin.x + px((p[0] * frame.scale) as f32),
                    frame.origin.y + px((p[1] * frame.scale) as f32),
                );
                let bounds = Bounds::new(at - point(px(5.), px(5.)), size(px(10.), px(10.)));
                window.paint_quad(
                    PaintQuad::new(bounds)
                        .corner_radii(px(5.))
                        .background(rgb(0xffffff))
                        .border_widths(1f32)
                        .border_color(rgb(0x252525)),
                );
            }
        }
    }
}

#[cfg(test)]
mod captures {
    use super::*;
    struct CaptureHost;
    impl Host for CaptureHost {
        fn time(&self, _: &App) -> f64 {
            0.
        }
        fn playing(&self, _: &App) -> bool {
            false
        }
        fn frame(&self, _: Bounds<Pixels>, _: &App) -> Option<ViewFrame> {
            Some(ViewFrame {
                origin: point(px(0.), px(0.)),
                scale: 1.,
                size: [640., 360.],
            })
        }
        fn tool(&self, _: &App) -> Tool {
            Tool::Select
        }
        fn overlay(
            &self,
            _: Vec<OverlayLine>,
            _: Option<crate::gizmo_layer::Placement>,
            _: &mut App,
        ) {
        }
        fn navigate(&self, _: Navigate, _: &mut App) {}
        fn layer_menu(&self, _: Option<String>, _: &mut Window, _: &mut App) {}
        fn report(&self, _: String, _: &mut App) {}
    }
    struct Scene {
        gizmo: Entity<GizmoSurface>,
        radial: bool,
    }
    impl Render for Scene {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let gizmo = self.gizmo.clone();
            let radial = self.radial;
            div().size_full().child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, cx| {
                        // Synthetic gradient scene; controls are the production viewer painter.
                        for y in (0..360).step_by(4) {
                            for x in (0..640).step_by(4) {
                                let t = if radial {
                                    ((x as f32 - 160.).hypot(y as f32 - 100.) / 300.).clamp(0., 1.)
                                } else {
                                    ((x as f32 - 160.) / 300.).clamp(0., 1.)
                                };
                                let n = (t * 255.) as u32;
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(px(x as f32), px(y as f32)),
                                        size(px(4.), px(4.)),
                                    ),
                                    rgb(n * 0x010101),
                                ));
                            }
                        }
                        gizmo.read(cx).effects_paint(
                            ViewFrame {
                                origin: point(px(0.), px(0.)),
                                scale: 1.,
                                size: [640., 360.],
                            },
                            window,
                            cx,
                        );
                    },
                )
                .size_full(),
            )
        }
    }
    #[::core::prelude::v1::test]
    #[ignore = "writes headless viewer-control review captures"]
    fn capture_gradient_handles() {
        let dir = std::env::var("PM_GRADIENT_CAPTURE").expect("PM_GRADIENT_CAPTURE");
        std::fs::create_dir_all(&dir).unwrap();
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
        for radial in [false, true] {
            let mut cx = HeadlessAppContext::with_platform(
                Arc::new(gpui_parley::ParleyTextSystem::new(
                    gpui_parley::SystemFonts::Load,
                )),
                Arc::new(()),
                || {
                    Some(Box::new(
                        gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                    ))
                },
            );
            let kernel = actor.handle();
            let window = cx
                .open_window(size(px(640.), px(360.)), |_, cx| {
                    let gizmo = cx.new(|_| {
                        let mut g = GizmoSurface::new(
                            kernel,
                            "capture".into(),
                            pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()),
                            Rc::new(CaptureHost),
                        );
                        g.effect_controls.push(Control {
                            layer: "demo".into(),
                            prefix: "ramp.".into(),
                            declaration: Handle::Line {
                                start: PointBinding {
                                    x: "x".into(),
                                    y: "y".into(),
                                },
                                end: PointBinding {
                                    x: "ex".into(),
                                    y: "ey".into(),
                                },
                                stops: None,
                                radial: Some("shape".into()),
                            },
                            world: [1., 0., 0., 1., 0., 0.],
                            values: BTreeMap::from([
                                ("x".into(), 160.),
                                ("y".into(), 100.),
                                ("ex".into(), 460.),
                                ("ey".into(), 100.),
                                ("shape".into(), if radial { 1. } else { 0. }),
                            ]),
                        });
                        g
                    });
                    cx.new(|_| Scene { gizmo, radial })
                })
                .unwrap();
            cx.run_until_parked();
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(format!(
                    "{dir}/{}.png",
                    if radial { "radial" } else { "linear" }
                ))
                .unwrap();
        }
    }
}
