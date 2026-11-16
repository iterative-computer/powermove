//! Viewer controls declared by effects, including local/agent registrations.
use super::*;
use pm_fx_defs::handles::{Handle, Mapping, PointBinding, Stop};
use std::collections::BTreeMap;
#[derive(Clone)]
pub(super) struct Control {
    layer: String,
    prefix: String,
    declaration: Handle,
    world: [f64; 6],
    values: BTreeMap<String, f64>,
    stops: Vec<Stop>,
}
pub(super) struct Drag {
    index: usize,
    part: usize,
    original: Control,
    time: f64,
    pub active: bool,
    pub removed: bool,
    color_channel: Option<usize>,
}
impl Control {
    fn stop_key(&self) -> Option<&str> {
        if let Handle::Line { stops: Some(k), .. } = &self.declaration {
            Some(k)
        } else {
            None
        }
    }
    fn stop_point(&self, index: usize) -> Option<[f64; 2]> {
        let p = self.points();
        let t = self.stops.get(index)?.position;
        Some([
            p[0][0] + (p[1][0] - p[0][0]) * t,
            p[0][1] + (p[1][1] - p[0][1]) * t,
        ])
    }
    fn stop_command(&self, time: f64) -> Option<Command> {
        let json = serde_json::to_string(&self.stops).ok()?;
        let mut set = SetProperty::new(
            &self.layer,
            format!("{}{}", self.prefix, self.stop_key()?),
            pm_model::json::parse(&json).ok()?,
            time,
        );
        set.preserve_hand_edits = false;
        set.mark_intent = true;
        Some(Command::Property(PropertyCommand::Set(set)))
    }

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
fn stop_rgba(color: [f64; 4]) -> Rgba {
    rgba(color.into_iter().fold(0u32, |v, c| {
        (v << 8) | ((c.clamp(0., 1.) * 255.).round() as u32)
    }))
}

impl GizmoSurface {
    pub(super) fn effects_reread(&mut self, cx: &mut Context<Self>) {
        // A user gesture owns its starting transform and local preview. A late
        // publication (including an agent edit) must not pull the pointer back.
        if self.effect_drag.is_some() {
            return;
        }
        let selection = self.effect_stop.and_then(|(i, j)| {
            self.effect_controls
                .get(i)
                .map(|c| (c.layer.clone(), c.prefix.clone(), c.declaration.clone(), j))
        });
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
        let layers = pm_kernel::model::list_layers(root, None);
        for id in p.selection.layers() {
            let Some(loc) = pm_kernel::model::find_layer(root, id) else {
                continue;
            };
            let Some(layer) = pm_kernel::model::layer_at(root, &loc) else {
                continue;
            };
            let layer = layer.to_value();
            if layers.iter().any(|l| l.id == *id && l.effectively_locked) {
                continue;
            }
            if !mirror.queries().active(None, id, time).unwrap_or(false) {
                continue;
            }
            let Ok(world) = mirror.queries().world_matrix(None, id, time) else {
                continue;
            };
            // Never wait for the font registry on the UI thread. Text bounds
            // come from the pose worker; its completion rereads these controls.
            let Some(origin) = mirror
                .queries()
                .bounds(None, id, time)
                .ok()
                .flatten()
                .map(|b| [b.x0, b.y0])
                .or_else(|| {
                    self.poses
                        .iter()
                        .find(|p| p.id == *id)
                        .map(|p| [p.bounds[0], p.bounds[1]])
                })
            else {
                continue;
            };
            let world = pm_fx_defs::handles::source_mapping(world, origin).0;
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
                                .evaluate_property(None, id, &format!("{prefix}{k}"), time)
                                .ok()
                                .and_then(|v| v.as_f64())
                                .map(|v| (k, v))
                        })
                        .collect();
                    let stops = if let Handle::Line { stops: Some(k), .. } = &declaration {
                        mirror
                            .queries()
                            .evaluate_property(None, id, &format!("{prefix}{k}"), time)
                            .ok()
                            .and_then(|v| {
                                serde_json::from_str::<serde_json::Value>(
                                    &pm_model::json::to_canonical_string(&v),
                                )
                                .ok()
                            })
                            .and_then(|v| pm_fx_defs::handles::validate_stops(&v).ok())
                            .unwrap_or_default()
                    } else {
                        vec![]
                    };
                    let control = Control {
                        layer: id.clone(),
                        prefix: prefix.clone(),
                        declaration,
                        world,
                        values,
                        stops,
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
            if layer.get("type").and_then(Value::as_str) == Some("extension") {
                if let Some(registration) = layer
                    .get("d")
                    .and_then(|d| d.get("definition"))
                    .and_then(Value::as_str)
                    .and_then(|id| state.provider("layerTypes", id))
                {
                    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(
                        &pm_model::json::to_canonical_string(&registration.definition),
                    ) {
                        if let Ok(params) = serde_json::from_value::<Vec<pm_fx_defs::Parameter>>(
                            raw["params"].clone(),
                        ) {
                            if let Ok(handles) =
                                pm_fx_defs::handles::parse(raw.get("viewerHandles"), &params)
                            {
                                add("x.".into(), handles);
                            }
                        }
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
        self.effect_stop = selection.and_then(|(layer, prefix, declaration, j)| {
            self.effect_controls
                .iter()
                .position(|c| {
                    c.layer == layer
                        && c.prefix == prefix
                        && c.declaration == declaration
                        && j < c.stops.len()
                })
                .map(|i| (i, j))
        });
        if self.effect_picker.is_some() {
            self.effect_picker = self.effect_stop;
            if self.effect_picker.is_none() {
                self.effect_colour = None;
            }
        }
        cx.notify();
    }
    #[cfg(test)]
    pub fn effect_points(&self) -> Vec<[f64; 2]> {
        self.effect_controls
            .iter()
            .flat_map(Control::points)
            .collect()
    }
    fn effects_stop_hit(&self, at: [f64; 2], scale: f64) -> Option<(usize, usize)> {
        self.effect_controls
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, c)| {
                c.stops.iter().enumerate().find_map(|(j, _)| {
                    let p = c.stop_point(j)?;
                    ((p[0] - at[0]).abs() * scale <= 8. && (p[1] - at[1]).abs() * scale <= 8.)
                        .then_some((i, j))
                })
            })
    }
    pub(super) fn effects_hit(&self, at: [f64; 2], scale: f64) -> Option<(usize, usize)> {
        // The small white endpoint centres move the line. The coloured square
        // surrounding a coincident stop still selects and moves that stop.
        let endpoint = self
            .effect_controls
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, c)| {
                c.points()
                    .iter()
                    .enumerate()
                    .find(|(_, p)| (p[0] - at[0]).hypot(p[1] - at[1]) * scale <= 4.)
                    .map(|(j, _)| (i, j))
            });
        endpoint
            .or_else(|| self.effects_stop_hit(at, scale).map(|(i, j)| (i, j + 2)))
            .or_else(|| {
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
            })
    }
    fn effects_line_hit(&self, at: [f64; 2], scale: f64) -> Option<(usize, f64)> {
        self.effect_controls
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, c)| {
                if c.stop_key().is_none() || c.stops.len() < 2 {
                    return None;
                }
                let p = c.points();
                let (t, _) = pm_fx_defs::handles::project(at, p[0], p[1]);
                let q = [
                    p[0][0] + t * (p[1][0] - p[0][0]),
                    p[0][1] + t * (p[1][1] - p[0][1]),
                ];
                ((q[0] - at[0]).hypot(q[1] - at[1]) * scale <= 5.).then_some((i, t))
            })
    }
    pub(super) fn effects_down(
        &mut self,
        event: &MouseDownEvent,
        frame: ViewFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.controls || self.host.playing(cx) {
            return false;
        }
        let at = self.to_comp(event.position, frame);
        if self.effect_colour.is_some() {
            if self
                .effects_picker_bounds(frame)
                .is_some_and(|b| b.contains(&event.position))
            {
                return true;
            }
            self.effects_finish_colour(true, cx);
        }
        let double = event.click_count >= 2;
        let hit = if double {
            self.effects_stop_hit(at, frame.scale)
                .map(|(i, j)| (i, j + 2))
                .or_else(|| self.effects_hit(at, frame.scale))
        } else {
            self.effects_hit(at, frame.scale)
        };
        if let Some((index, part)) = hit {
            if let Some(focus) = &self.focus {
                window.focus(focus, cx);
            }
            if part >= 2 {
                self.effect_stop = Some((index, part - 2));
                if double {
                    self.effects_open_colour(index, part - 2, window, cx);
                    cx.notify();
                    return true;
                }
            } else {
                self.effect_stop = None;
                self.effect_picker = None;
            }
            self.effect_drag = Some(Drag {
                index,
                part,
                original: self.effect_controls[index].clone(),
                time: self.host.time(cx),
                active: false,
                removed: false,
                color_channel: None,
            });
            self.set_cursor(CursorStyle::Crosshair, cx);
            cx.notify();
            return true;
        }
        if let Some((index, t)) = self.effects_line_hit(at, frame.scale) {
            let c = &mut self.effect_controls[index];
            if c.stops.len() >= pm_fx_defs::handles::MAX_STOPS {
                return true;
            }
            let j = pm_fx_defs::handles::add_stop(&mut c.stops, t);
            if let Some(command) = c.stop_command(self.host.time(cx)) {
                self.queue
                    .push_back(Job::Transact("Add gradient stop", vec![command], None));
            }
            self.effect_stop = Some((index, j));
            if let Some(focus) = &self.focus {
                window.focus(focus, cx);
            }
            self.pump(cx);
            cx.notify();
            return true;
        }
        self.effect_stop = None;
        false
    }
    pub(super) fn effects_delete(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.controls || self.host.playing(cx) || self.text_session().is_some() {
            return false;
        }
        let Some((i, j)) = self.effect_stop else {
            return false;
        };
        if self.effect_colour.is_some() {
            return false;
        }
        if self.effect_drag.is_some() {
            return true;
        }
        let Some(c) = self.effect_controls.get_mut(i) else {
            return false;
        };
        if pm_fx_defs::handles::delete_stop(&mut c.stops, j) {
            if let Some(command) = c.stop_command(self.host.time(cx)) {
                self.queue
                    .push_back(Job::Transact("Delete gradient stop", vec![command], None));
                self.pump(cx);
            }
            self.effect_stop = None;
            self.effect_picker = None;
            cx.notify();
        }
        true
    }
    pub(super) fn effects_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        if self.effect_drag.is_some() && self.host.playing(cx) {
            if self.effect_colour.is_some() {
                self.effects_finish_colour(false, cx);
            } else {
                self.on_up(false, None, cx);
            }
            return true;
        }
        if self
            .effect_drag
            .as_ref()
            .is_some_and(|d| d.color_channel.is_some())
        {
            return true;
        }
        let Some(mut drag) = self.effect_drag.take() else {
            return false;
        };
        let Some(frame) = self.host.frame(self.bounds, cx) else {
            self.effect_drag = Some(drag);
            return true;
        };
        if drag.part >= 2 {
            let points = drag.original.points();
            let at = self.to_comp(event.position, frame);
            let screen = |p: [f64; 2]| p.map(|v| v * frame.scale);
            let (stops, removed) = pm_fx_defs::handles::drag_stop(
                &drag.original.stops,
                drag.part - 2,
                screen(at),
                screen(points[0]),
                screen(points[1]),
            );
            drag.removed = removed;
            self.effect_controls[drag.index].stops = stops;
            if !drag.active {
                drag.active = true;
                self.queue.push_back(Job::Begin("Move gradient stop"));
            }
            if let Some(command) = self.effect_controls[drag.index].stop_command(drag.time) {
                self.queue.push_back(Job::Step(vec![command], true));
            }
            self.effect_drag = Some(drag);
            self.pump(cx);
            cx.notify();
            return true;
        }
        let original = &drag.original;
        let bindings = original.declaration.points();
        let mut comp = self.to_comp(event.position, frame);
        if event.modifiers.shift && matches!(original.declaration, Handle::Line { .. }) {
            comp = pm_fx_defs::handles::constrain(comp, original.points()[1 - drag.part]);
        }
        let Some(mut at) = Mapping(original.world).local(comp) else {
            self.effect_drag = Some(drag);
            return true;
        };
        if event.modifiers.shift && matches!(original.declaration, Handle::Rect { .. }) {
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
    pub(super) fn effects_paint(&self, frame: ViewFrame, window: &mut Window, cx: &mut App) {
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
        for (i, c) in self.effect_controls.iter().enumerate() {
            for (j, stop) in c.stops.iter().enumerate() {
                let Some(p) = c.stop_point(j) else { continue };
                let at = point(
                    frame.origin.x + px((p[0] * frame.scale) as f32),
                    frame.origin.y + px((p[1] * frame.scale) as f32),
                );
                if self.effect_stop == Some((i, j)) {
                    window.paint_quad(
                        PaintQuad::new(Bounds::new(
                            at - point(px(8.), px(8.)),
                            size(px(16.), px(16.)),
                        ))
                        .corner_radii(px(4.))
                        .background(rgb(self.tokens.colors.accent)),
                    );
                }
                let b = Bounds::new(at - point(px(6.), px(6.)), size(px(12.), px(12.)));
                window.paint_quad(
                    PaintQuad::new(b)
                        .corner_radii(px(3.))
                        .background(rgb(0x888888)),
                );
                window.paint_quad(
                    PaintQuad::new(b)
                        .corner_radii(px(3.))
                        .background(stop_rgba(stop.color))
                        .border_widths(1f32)
                        .border_color(rgb(0xffffff)),
                );
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

impl GizmoSurface {
    #[cfg(test)]
    pub fn effect_gesture_open(&self) -> bool {
        self.effect_drag.as_ref().is_some_and(|d| d.active) && self.gesture.is_some()
    }
    #[cfg(test)]
    pub fn effect_stop_values(&self) -> Vec<Stop> {
        self.effect_controls
            .first()
            .map(|c| c.stops.clone())
            .unwrap_or_default()
    }
    #[cfg(test)]
    pub fn effect_picker_rect(&self, cx: &App) -> Option<Bounds<Pixels>> {
        self.effects_picker_bounds(self.host.frame(self.bounds, cx)?)
    }
    pub(super) fn effects_hover(&self, at: [f64; 2], frame: ViewFrame) -> bool {
        self.effects_hit(at, frame.scale).is_some()
            || self.effects_line_hit(at, frame.scale).is_some()
            || self.effects_picker_bounds(frame).is_some_and(|b| {
                b.contains(&point(
                    frame.origin.x + px((at[0] * frame.scale) as f32),
                    frame.origin.y + px((at[1] * frame.scale) as f32),
                ))
            })
    }
    fn effects_picker_bounds(&self, frame: ViewFrame) -> Option<Bounds<Pixels>> {
        let (i, j) = self.effect_picker?;
        let p = self.effect_controls.get(i)?.stop_point(j)?;
        let anchor = [
            f64::from(f32::from(frame.origin.x)) + p[0] * frame.scale,
            f64::from(f32::from(frame.origin.y)) + p[1] * frame.scale,
        ];
        let view = if self.bounds.size.width > px(0.) {
            [
                f64::from(f32::from(self.bounds.origin.x)),
                f64::from(f32::from(self.bounds.origin.y)),
                f64::from(f32::from(self.bounds.size.width)),
                f64::from(f32::from(self.bounds.size.height)),
            ]
        } else {
            [0., 0., 640., 360.]
        };
        let w = f64::from(self.tokens.metrics.color_picker_width);
        let h = 320.;
        Some(Bounds::new(
            point(
                px(anchor[0].clamp(view[0], (view[0] + view[2] - w).max(view[0])) as f32),
                px((anchor[1] + 18.).min((view[1] + view[3] - h).max(view[1])) as f32),
            ),
            size(px(w as f32), px(h as f32)),
        ))
    }
    pub(super) fn effects_open_colour(
        &mut self,
        i: usize,
        j: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::{
            colour_model::Color,
            colour_picker::{Edit, Picker},
        };
        self.effect_picker = Some((i, j));
        let color = Color(self.effect_controls[i].stops[j].color.map(|v| v as f32));
        let tokens = self.tokens.clone();
        let picker = cx.new(|cx| Picker::new(color, tokens, window, None, cx));
        let subscription = cx.subscribe(&picker, |g, entity, event, cx| match event {
            Edit::Begin => {
                if let Some((i, j)) = g.effect_picker {
                    g.effect_drag = Some(Drag {
                        index: i,
                        part: j + 2,
                        original: g.effect_controls[i].clone(),
                        time: g.host.time(cx),
                        active: true,
                        removed: false,
                        color_channel: Some(0),
                    });
                    g.queue.push_back(Job::Begin("Change gradient stop colour"));
                    g.pump(cx);
                }
            }
            Edit::Value(_) => {
                if let Some((i, j)) = g.effect_picker {
                    g.effect_controls[i].stops[j].color = entity.read(cx).color.0.map(f64::from);
                    if let Some(command) = g.effect_controls[i].stop_command(g.host.time(cx)) {
                        g.queue.push_back(Job::Step(vec![command], true));
                        g.pump(cx);
                    }
                    cx.notify();
                }
            }
            Edit::Finish(commit) => g.effects_finish_colour(*commit, cx),
            Edit::Sample => (),
        });
        self.effect_colour = Some((picker, subscription));
        cx.notify();
    }
    pub(super) fn effects_finish_colour(&mut self, commit: bool, cx: &mut Context<Self>) {
        self.effect_colour = None;
        self.effect_picker = None;
        if self.effect_drag.take().is_some_and(|d| d.active) {
            self.queue.push_back(Job::Finish(commit));
            self.pump(cx);
        }
        self.effects_reread(cx);
        cx.notify();
    }
    pub(super) fn effects_picker_element(&self, cx: &App) -> Option<AnyElement> {
        let (picker, _) = self.effect_colour.as_ref()?;
        let b = self.effects_picker_bounds(self.host.frame(self.bounds, cx)?)?;
        Some(
            deferred(
                anchored()
                    .position(b.origin)
                    .snap_to_window()
                    .child(picker.clone()),
            )
            .priority(3)
            .into_any_element(),
        )
    }
    #[cfg(test)]
    pub fn effect_colour_entity(&self) -> Option<Entity<crate::colour_picker::Picker>> {
        self.effect_colour.as_ref().map(|(p, _)| p.clone())
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
        image: Arc<Image>,
    }
    impl Render for Scene {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let gizmo = self.gizmo.clone();

            div()
                .size_full()
                .relative()
                .child(
                    img(ImageSource::Image(self.image.clone()))
                        .absolute()
                        .size_full(),
                )
                .children(self.gizmo.read(cx).effects_picker_element(cx))
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, cx| {
                            gizmo.update(cx, |g, cx| {
                                g.effects_paint(
                                    ViewFrame {
                                        origin: point(px(0.), px(0.)),
                                        scale: 1.,
                                        size: [640., 360.],
                                    },
                                    window,
                                    cx,
                                )
                            });
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
        for radial in [false, true] {
            let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
            let ch = |v: serde_json::Value| serde_json::json!({"v":v,"kf":[]});
            let source = serde_json::json!({"id":"capture-project","compId":"capture-comp","w":640,"h":360,"fps":30,"dur":2,"bg":"#000000","assets":{},"comps":{},"layers":[{
            "id":"demo","type":"solid","name":"Gradient Ramp","from":0,"dur":2,"on":true,"p":{"position.x":ch(0.into()),"position.y":ch(0.into()),"scale.x":ch(100.into()),"scale.y":ch(100.into()),"rotation":ch(0.into()),"opacity":ch(100.into())},"d":{"w":640,"h":360,"color":"#FFFFFF"},"fx":[{"id":"ramp","type":"gradient","version":3,"on":true,"open":true,"p":{
                "startX":ch(160.into()),"startY":ch(100.into()),"endX":ch(460.into()),"endY":ch(100.into()),"rampShape":ch((if radial{1}else{0}).into()),"stops":ch(serde_json::json!([{"position":0,"color":[0,0,0,1]},{"position":0.5,"color":[0.5,0.5,0.5,1]},{"position":1,"color":[1,1,1,1]}]))
            }}]}]});
            let kernel = actor.handle();
            kernel
                .open(
                    "capture",
                    pm_kernel::OpenSource::Untitled(
                        pm_model::json::parse(&source.to_string()).unwrap(),
                    ),
                )
                .wait()
                .unwrap()
                .unwrap();
            kernel
                .call(
                    "capture",
                    Request::new(
                        Caller::native(),
                        SelectionCall::Layers {
                            ids: vec!["demo".into()],
                            add: false,
                        },
                    ),
                )
                .wait()
                .unwrap()
                .unwrap();
            let p = kernel.published("capture").unwrap();
            let mut renderer = pm_render::Renderer::headless().unwrap();
            renderer
                .set_main_effects_for_document(&p.snapshot.document)
                .unwrap();
            let rendered = renderer
                .render(
                    &p.snapshot.document,
                    &p.snapshot.render_comp(),
                    pm_protocol::common::Ticks::ZERO,
                    &pm_render::RenderRequest::sized(
                        pm_protocol::common::Size::new(640, 360),
                        pm_protocol::color::Precision::Float16,
                    ),
                    &pm_render::MapResolver::default(),
                )
                .unwrap();
            assert!(rendered.failures.is_empty(), "{:?}", rendered.failures);
            let pixels: Vec<u8> = renderer
                .read_rgba(&rendered)
                .unwrap()
                .into_iter()
                .map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
                .collect();
            let mut bytes = vec![];
            {
                let mut png = png::Encoder::new(&mut bytes, 640, 360);
                png.set_color(png::ColorType::Rgba);
                png.set_depth(png::BitDepth::Eight);
                png.write_header()
                    .unwrap()
                    .write_image_data(&pixels)
                    .unwrap();
            }
            let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));

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
                .open_window(size(px(640.), px(360.)), |window, cx| {
                    let gizmo = cx.new(|cx| {
                        let mut g = GizmoSurface::new(
                            kernel,
                            "capture".into(),
                            pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()),
                            Rc::new(CaptureHost),
                        );
                        // Capture controls resolved from the real selected layer and
                        // its evaluated parameters, through the production reader.
                        g.effects_reread(cx);
                        assert_eq!(g.effect_points(), vec![[160., 100.], [460., 100.]]);
                        assert_eq!(g.effect_stop_values().len(), 3);
                        g.effect_stop = Some((0, 1));
                        if radial {
                            g.effects_open_colour(0, 1, window, cx);
                        }
                        g
                    });
                    cx.new(|_| Scene { gizmo, image })
                })
                .unwrap();
            for _ in 0..20 {
                cx.run_until_parked();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
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
