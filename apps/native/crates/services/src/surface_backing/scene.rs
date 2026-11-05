//! S098. Source intent: layers-3d/{scene-model,SceneField,LayerInspector}.
//! Structural command planning is separate from capability admission. The current
//! kernel Scene3dCommand is NotImplemented: do not infer support from this model.
use super::{Session, invalid, unavailable};
use crate::Result;
use serde_json::{Value, json};

pub const PRIMITIVES: &[&str] = &[
    "box",
    "sphere",
    "plane",
    "cylinder",
    "cone",
    "torus",
    "capsule",
    "icosahedron",
];
pub const MAP_SLOTS: &[&str] = &[
    "color",
    "normal",
    "roughness",
    "metalness",
    "emissive",
    "ao",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Number,
    Color,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub path: String,
    pub kind: FieldKind,
    pub step: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
}
fn number(path: &str, step: f64, min: Option<f64>, max: Option<f64>) -> Field {
    Field {
        path: path.into(),
        kind: FieldKind::Number,
        step,
        min,
        max,
    }
}
fn color(path: &str) -> Field {
    Field {
        path: path.into(),
        kind: FieldKind::Color,
        step: 0.,
        min: None,
        max: None,
    }
}
fn target(prefix: &str) -> Vec<Field> {
    ["targetX", "targetY", "targetZ"]
        .map(|key| number(&format!("{prefix}.{key}"), 0.01, None, None))
        .to_vec()
}
pub fn fields(data: &Value) -> Result<Vec<Field>> {
    if data.get("object").is_some_and(Value::is_object) {
        if data["object"]["useSourceMaterials"] == true {
            return Ok(Vec::new());
        }
        return Ok(vec![
            color("m.color"),
            number("m.roughness", 0.01, Some(0.02), Some(1.)),
            number("m.metalness", 0.01, Some(0.), Some(1.)),
            color("m.emissive"),
            number("m.emissiveIntensity", 0.05, Some(0.), None),
            number("m.opacity", 0.01, Some(0.), Some(1.)),
        ]);
    }
    if data.get("light").is_some_and(Value::is_object) {
        let kind = data["light"]["type"]
            .as_str()
            .ok_or_else(|| invalid("missing light type"))?;
        if !["sun", "point", "spot"].contains(&kind) {
            return Err(unavailable("unsupported light type retained"));
        }
        let mut out = vec![
            color("light.color"),
            number("light.intensity", 0.05, Some(0.), None),
        ];
        if kind != "point" {
            out.extend(target("light"));
        }
        if kind != "sun" {
            out.extend([
                number("light.distance", 0.1, Some(0.), None),
                number("light.decay", 0.05, Some(0.), None),
            ]);
        }
        if kind == "spot" {
            out.extend([
                number("light.angle", 1., Some(1.), Some(89.)),
                number("light.penumbra", 0.01, Some(0.), Some(1.)),
            ]);
        }
        return Ok(out);
    }
    if data.get("camera").is_some_and(Value::is_object) {
        let mut out = target("camera");
        out.push(match data["camera"]["projection"].as_str() {
            Some("perspective") => number("camera.fov", 0.5, Some(1.), Some(150.)),
            Some("orthographic") => number("camera.zoom", 0.01, Some(0.01), None),
            _ => return Err(unavailable("unsupported camera projection retained")),
        });
        out.extend([
            number("camera.near", 0.01, Some(0.001), None),
            number("camera.far", 1., Some(0.01), None),
            number("environment.ambient", 0.01, Some(0.), None),
            color("environment.ambientColor"),
            number("environment.exposure", 0.01, Some(0.), None),
        ]);
        return Ok(out);
    }
    Err(unavailable("unsupported scene payload retained"))
}
pub fn field_command(
    layer_id: &str,
    field: &Field,
    value: Value,
    time: f64,
    keyframe: bool,
) -> Result<Value> {
    if layer_id.is_empty() || !time.is_finite() {
        return Err(invalid("invalid scene edit identity or time"));
    }
    if field.kind == FieldKind::Number {
        let n = value
            .as_f64()
            .ok_or_else(|| invalid("invalid scene number"))?;
        if field.min.is_some_and(|min| n < min) || field.max.is_some_and(|max| n > max) {
            return Err(invalid("scene number outside authored domain"));
        }
    }
    Ok(
        json!({"type":"set_property","target":layer_id,"path":field.path,"value":value,"time":time,"mode":if keyframe {"keyframe"} else {"auto"},"preserveHandEdits":false}),
    )
}
#[derive(Clone, Debug, PartialEq)]
pub struct SceneEdit {
    pub session: Session,
    pub revision: u64,
    pub layer_id: String,
    pub operation: Value,
}
pub trait SceneHost {
    /// Defaults to unavailable. Implement only after actual kernel commands and
    /// asset admission are wired; a model/trait is not an optimistic capability flag.
    fn apply(&mut self, _edit: &SceneEdit) -> Result<u64> {
        Err(unavailable("3D kernel editing not installed"))
    }
    fn import_model(&mut self, _session: &Session, _asset_id: &str) -> Result<u64> {
        Err(unavailable("3D model importer not installed"))
    }
}
/// Editor camera is ephemeral; storing it never mutates the authored render camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Navigation {
    pub editor_camera: Value,
    pub before_gesture: Option<Value>,
}
impl Navigation {
    pub fn begin(&mut self) -> Result<()> {
        if self.before_gesture.is_some() {
            return Err(invalid("navigation gesture active"));
        }
        self.before_gesture = Some(self.editor_camera.clone());
        Ok(())
    }
    pub fn cancel(&mut self) {
        if let Some(before) = self.before_gesture.take() {
            self.editor_camera = before;
        }
    }
    pub fn commit(&mut self) {
        self.before_gesture = None;
    }
}
