//! 3D content (`core/scene3d/schema.ts`, `core/scene3d/layers.ts`).
//!
//! 3D layers are extension layers whose `d.data` holds `object`, `light`, or
//! `camera` and `environment` ([`super::ExtensionData`]); identity, hierarchy,
//! and transforms live on the layer itself. Legacy scene layers store a whole
//! [`Scene3D`] under `d.data.scene`. Channel maps (`p`) accept constants or
//! channels, as the zod `channel()` helper does.

use super::{
    BlenderMaterial, BlenderSource, ChannelValue, JsMap, Nullable, Prop, record, string_enum,
};
use crate::json::JsString;

/// A 3D channel map: names to constants or channels.
pub type Channels3D = JsMap<Prop<ChannelValue>>;

record! {
    /// `primitiveParams`.
    pub struct PrimitiveParams {
        width: f64 = "width",
        height: f64 = "height",
        depth: f64 = "depth",
        radius: f64 = "radius",
        radius_top: f64 = "radiusTop",
        radius_bottom: f64 = "radiusBottom",
        tube: f64 = "tube",
        segments: f64 = "segments",
        detail: f64 = "detail",
    }
}

record! {
    /// A material face range of an inline mesh.
    pub struct MeshGroup {
        start: f64 = "start",
        count: f64 = "count",
        material_index: f64 = "materialIndex",
    }
}

record! {
    /// An inline triangle mesh.
    pub struct Mesh {
        positions: Vec<f64> = "positions",
        indices: Vec<f64> = "indices",
        groups: Vec<MeshGroup> = "groups",
        normals: Vec<f64> = "normals",
        uvs: Vec<f64> = "uvs",
    }
}

string_enum! {
    /// Built-in primitives.
    pub enum Primitive {
        Box = "box",
        Sphere = "sphere",
        Plane = "plane",
        Cylinder = "cylinder",
        Cone = "cone",
        Torus = "torus",
        Capsule = "capsule",
        Icosahedron = "icosahedron",
    }
}

record! {
    /// `sourceSchema`, a union decided by which member is present:
    /// `primitive` (+ `parameters`), `assetId`, `mesh`, `lathe` (+ `segments`),
    /// or `extrude` (+ `depth`, `bevel`).
    pub struct Source {
        primitive: Primitive = "primitive",
        parameters: PrimitiveParams = "parameters",
        asset_id: JsString = "assetId",
        mesh: Mesh = "mesh",
        lathe: Vec<[f64; 2]> = "lathe",
        segments: f64 = "segments",
        extrude: Vec<[f64; 2]> = "extrude",
        depth: f64 = "depth",
        bevel: f64 = "bevel",
    }
}

record! {
    /// Texture placement of a material.
    pub struct Placement {
        scale_x: f64 = "scaleX",
        scale_y: f64 = "scaleY",
        offset_x: f64 = "offsetX",
        offset_y: f64 = "offsetY",
        rotation: f64 = "rotation",
    }
}

record! {
    /// Texture maps by role (asset ids).
    pub struct MaterialMaps {
        color: JsString = "color",
        normal: JsString = "normal",
        roughness: JsString = "roughness",
        metalness: JsString = "metalness",
        emissive: JsString = "emissive",
        ao: JsString = "ao",
    }
}

record! {
    /// `materialSchema`; channels `color roughness metalness emissive
    /// emissiveIntensity opacity`.
    pub struct Material {
        shader: BlenderMaterial = "shader",
        placement: Placement = "placement",
        p: Channels3D = "p",
        maps: MaterialMaps = "maps",
        double_sided: bool = "doubleSided",
    }
}

record! {
    /// A material slot.
    pub struct MaterialSlot {
        id: JsString = "id",
        name: JsString = "name",
        material: Material = "material",
    }
}

record! {
    /// Which generated model part an object is.
    pub struct Generation {
        group_id: JsString = "groupId",
        part_id: JsString = "partId",
    }
}

record! {
    /// `objectSchema`. On a 3D object layer (`d.data.object`) the identity
    /// members (`id`, `name`, `parent`, `p`) are absent.
    pub struct SceneObject {
        id: JsString = "id",
        name: JsString = "name",
        parent: Nullable<JsString> = "parent",
        source: Source = "source",
        /// Channels `x y z rx ry rz sx sy sz visible`.
        p: Channels3D = "p",
        material: Material = "material",
        use_source_materials: bool = "useSourceMaterials",
        blender: BlenderSource = "blender",
        generation: Generation = "generation",
        slots: Vec<MaterialSlot> = "slots",
        cast_shadow: bool = "castShadow",
        receive_shadow: bool = "receiveShadow",
    }
}

/// `d.data.object` of a 3D object layer.
pub type ObjectContent = SceneObject;

string_enum! {
    /// Light types.
    pub enum LightType {
        Sun = "sun",
        Point = "point",
        Spot = "spot",
        Area = "area",
    }
}

record! {
    /// `lightSchema`. On a light layer (`d.data.light`) only `type`, `p` (without
    /// `x y z visible`), and `castShadow` are stored.
    pub struct SceneLight {
        id: JsString = "id",
        name: JsString = "name",
        light_type: LightType = "type",
        /// Channels `x y z targetX targetY targetZ color intensity distance decay
        /// angle penumbra width height visible`.
        p: Channels3D = "p",
        cast_shadow: bool = "castShadow",
    }
}

/// `d.data.light` of a light layer.
pub type LightContent = SceneLight;

string_enum! {
    /// Camera projections.
    pub enum Projection {
        Perspective = "perspective",
        Orthographic = "orthographic",
    }
}

record! {
    /// `cameraSchema`; channels `x y z targetX targetY targetZ fov zoom near far`
    /// (`x y z` absent on camera layers).
    pub struct Camera {
        projection: Projection = "projection",
        p: Channels3D = "p",
    }
}

/// `d.data.camera` of a camera layer.
pub type CameraContent = Camera;

record! {
    /// `environmentSchema`; channels `ambient ambientColor exposure`.
    pub struct Environment {
        p: Channels3D = "p",
        background: Nullable<JsString> = "background",
        shadows: bool = "shadows",
    }
}

record! {
    /// A legacy scene layer's whole scene.
    pub struct Scene3D {
        version: f64 = "version",
        objects: Vec<SceneObject> = "objects",
        lights: Vec<SceneLight> = "lights",
        camera: Camera = "camera",
        environment: Environment = "environment",
    }
}
