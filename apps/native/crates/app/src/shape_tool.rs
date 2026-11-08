//! The Shape tool (Q): a drag in the viewer draws a rectangle (viewer.ts
//! `startShape`/`createShape`). With an unlocked, maskable layer selected
//! the box becomes a mask on it; otherwise a new shape layer.
use pm_kernel::{
    Command, LayerCommand, PropertyCommand, Published, QueryMirror,
    commands::{
        layers::AddLayer,
        masks::{MaskCommand, MaskShape},
        properties::SetProperty,
    },
    new_id,
};
use pm_model::{Map, Value};

/// A drawn box in composition pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}
impl ShapeBox {
    /// `shapeBoxFromDrag`: Shift draws a square, Option draws from the centre.
    pub fn from_drag(start: [f64; 2], pointer: [f64; 2], from_center: bool, square: bool) -> Self {
        let (mut dx, mut dy) = (pointer[0] - start[0], pointer[1] - start[1]);
        if square {
            let size = dx.abs().max(dy.abs());
            dx = size.copysign(if dx < 0. { -1. } else { 1. });
            dy = size.copysign(if dy < 0. { -1. } else { 1. });
        }
        let near = if from_center {
            [start[0] - dx, start[1] - dy]
        } else {
            start
        };
        let far = [start[0] + dx, start[1] + dy];
        Self {
            x0: near[0].min(far[0]),
            y0: near[1].min(far[1]),
            x1: near[0].max(far[0]),
            y1: near[1].max(far[1]),
        }
    }
    pub fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    pub fn corners(&self) -> [[f64; 2]; 4] {
        [
            [self.x0, self.y0],
            [self.x1, self.y0],
            [self.x1, self.y1],
            [self.x0, self.y1],
        ]
    }
}
/// What the drawn box writes: the undo label, the commands and the layer to
/// select afterwards (a new shape layer; a mask keeps the selection).
pub struct ShapePlan {
    pub label: &'static str,
    pub commands: Vec<Command>,
    pub select: Option<String>,
}
/// `None` for a box too small to draw (under half a pixel either way).
pub fn plan(published: &Published, shape: ShapeBox, time: f64) -> Option<ShapePlan> {
    if shape.w() < 0.5 || shape.h() < 0.5 {
        return None;
    }
    if let Some(plan) = mask_plan(published, shape, time) {
        return Some(plan);
    }
    let id = new_id("L");
    let mut content = Map::new();
    content.insert("shape", Value::from("rect"));
    content.insert("w", Value::Number(100.));
    content.insert("h", Value::Number(100.));
    content.insert("radius", Value::Number(0.));
    // The source shape is 100 px, so Scale in percent is the drawn size.
    let mut properties = Map::new();
    properties.insert("position.x", Value::Number((shape.x0 + shape.x1) / 2.));
    properties.insert("position.y", Value::Number((shape.y0 + shape.y1) / 2.));
    properties.insert("scale.x", Value::Number(shape.w()));
    properties.insert("scale.y", Value::Number(shape.h()));
    let add = AddLayer {
        id: id.clone(),
        layer_type: "shape".into(),
        comp: None,
        name: Some("Rect".into()),
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
    Some(ShapePlan {
        label: "Draw rect",
        commands: vec![LayerCommand::Add(Box::new(add)).into()],
        select: Some(id),
    })
}
/// The first selected layer takes the box as a mask when it can hold one.
fn mask_plan(published: &Published, shape: ShapeBox, time: f64) -> Option<ShapePlan> {
    let layer = published.selection.layers().first()?.clone();
    let root = published.snapshot.root()?;
    let info = pm_kernel::model::list_layers(root, None)
        .into_iter()
        .find(|l| l.id == layer)?;
    if info.effectively_locked || matches!(info.kind.as_str(), "shape" | "audio") {
        return None;
    }
    let world = QueryMirror::new(published)
        .queries()
        .world_matrix(None, &layer, time)
        .ok()?;
    let first = crate::gizmo::inverse_point(&world, [shape.x0, shape.y0])?;
    let second = crate::gizmo::inverse_point(&world, [shape.x1, shape.y1])?;
    let mask = new_id("M");
    let mut commands: Vec<Command> = vec![
        MaskCommand::Add {
            layer_id: layer.clone(),
            mask_id: mask.clone(),
            shape: MaskShape::Rect,
        }
        .into(),
    ];
    for (field, value) in [
        ("x", (first[0] + second[0]) / 2.),
        ("y", (first[1] + second[1]) / 2.),
        ("w", (second[0] - first[0]).abs()),
        ("h", (second[1] - first[1]).abs()),
        ("feather", 0.),
    ] {
        let mut set = SetProperty::new(
            &layer,
            format!("m.{mask}.{field}"),
            Value::Number(value),
            time,
        );
        set.preserve_hand_edits = false;
        commands.push(PropertyCommand::Set(set).into());
    }
    Some(ShapePlan {
        label: "Draw mask",
        commands,
        select: None,
    })
}
