//! Blender materials, render settings, and generated models
//! (`shared/blender.ts`).

use super::{Channel, ChannelValue, Codec, JsMap, record, string_enum};
use crate::json::{JsString, Value};

/// A shader socket: a name or an index.
#[derive(Clone, Debug, PartialEq)]
pub enum Socket {
    Name(JsString),
    Index(f64),
}

impl Codec for Socket {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::String(s) => Ok(Socket::Name(s)),
            Value::Number(n) => Ok(Socket::Index(n)),
            other => Err(other),
        }
    }

    fn encode(&self) -> Value {
        match self {
            Socket::Name(s) => Value::String(s.clone()),
            Socket::Index(n) => Value::Number(*n),
        }
    }
}

record! {
    /// One color-ramp stop.
    pub struct RampStop {
        position: f64 = "position",
        color: JsString = "color",
    }
}

record! {
    /// A `ShaderNodeValToRGB` ramp.
    pub struct ColorRamp {
        interpolation: JsString = "interpolation",
        stops: Vec<RampStop> = "stops",
    }
}

record! {
    /// `nodeProperties`.
    pub struct NodeProperties {
        operation: JsString = "operation",
        blend_type: JsString = "blend_type",
        noise_dimensions: JsString = "noise_dimensions",
        voronoi_dimensions: JsString = "voronoi_dimensions",
        distance: JsString = "distance",
        feature: JsString = "feature",
        wave_type: JsString = "wave_type",
        bands_direction: JsString = "bands_direction",
        rings_direction: JsString = "rings_direction",
        gradient_type: JsString = "gradient_type",
        interpolation: JsString = "interpolation",
        extension: JsString = "extension",
        vector_type: JsString = "vector_type",
        space: JsString = "space",
        distribution: JsString = "distribution",
        use_clamp: bool = "use_clamp",
    }
}

record! {
    /// A shader node (`type` is one of `SHADER_NODES`).
    pub struct ShaderNode {
        id: JsString = "id",
        node_type: JsString = "type",
        /// Socket values: numbers, booleans, `#rrggbb`, or 2 to 4 numbers.
        inputs: JsMap<ChannelValue> = "inputs",
        properties: NodeProperties = "properties",
        image: JsString = "image",
        color_space: JsString = "colorSpace",
        ramp: ColorRamp = "ramp",
    }
}

record! {
    /// A link between node sockets.
    pub struct ShaderLink {
        from: JsString = "from",
        output: Socket = "output",
        to: JsString = "to",
        input: Socket = "input",
    }
}

record! {
    /// `shaderGraphSchema`.
    pub struct ShaderGraph {
        nodes: Vec<ShaderNode> = "nodes",
        links: Vec<ShaderLink> = "links",
    }
}

record! {
    /// An exposed material input (`shaderInputSchema`).
    pub struct ShaderInput {
        id: JsString = "id",
        label: JsString = "label",
        /// `number | color | toggle`.
        kind: JsString = "kind",
        node: JsString = "node",
        socket: Socket = "socket",
        min: f64 = "min",
        max: f64 = "max",
        step: f64 = "step",
    }
}

record! {
    /// A material taken from a `.blend` asset.
    pub struct BlenderMaterialSource {
        asset_id: JsString = "assetId",
        material: JsString = "material",
    }
}

record! {
    /// `blenderMaterialSchema`.
    pub struct BlenderMaterial {
        id: JsString = "id",
        name: JsString = "name",
        preset: JsString = "preset",
        source: BlenderMaterialSource = "source",
        graph: ShaderGraph = "graph",
        inputs: Vec<ShaderInput> = "inputs",
        p: JsMap<Channel> = "p",
    }
}

string_enum! {
    /// Blender render engines.
    pub enum RenderEngine {
        Eevee = "eevee",
        Cycles = "cycles",
    }
}

record! {
    /// `render3DSchema`: a composition's Blender render settings (`render3d`).
    pub struct Render3D {
        enabled: bool = "enabled",
        engine: RenderEngine = "engine",
        samples: f64 = "samples",
        preview_samples: f64 = "previewSamples",
        preview_scale: f64 = "previewScale",
        denoise: bool = "denoise",
        /// `auto | cpu | gpu`.
        device: JsString = "device",
    }
}

record! {
    /// `blenderSourceSchema`: an object inside a `.blend` asset.
    pub struct BlenderSource {
        asset_id: JsString = "assetId",
        object: JsString = "object",
    }
}

record! {
    /// One modifier (`modifierSchema`, discriminated by `type`): bevel
    /// (`width segments`), subdivision (`levels`), solidify (`thickness`),
    /// array (`count offset`), mirror (`axes`).
    pub struct Modifier {
        modifier_type: JsString = "type",
        width: f64 = "width",
        segments: f64 = "segments",
        levels: f64 = "levels",
        thickness: f64 = "thickness",
        count: f64 = "count",
        offset: [f64; 3] = "offset",
        axes: [bool; 3] = "axes",
    }
}

record! {
    /// An inline recipe mesh.
    pub struct RecipeMesh {
        positions: Vec<f64> = "positions",
        indices: Vec<f64> = "indices",
    }
}

/// A recipe parameter: a number or a string.
#[derive(Clone, Debug, PartialEq)]
pub enum RecipeParameter {
    Number(f64),
    String(JsString),
}

impl Codec for RecipeParameter {
    fn decode(value: Value) -> Result<Self, Value> {
        match value {
            Value::Number(n) => Ok(RecipeParameter::Number(n)),
            Value::String(s) => Ok(RecipeParameter::String(s)),
            other => Err(other),
        }
    }

    fn encode(&self) -> Value {
        match self {
            RecipeParameter::Number(n) => Value::Number(*n),
            RecipeParameter::String(s) => Value::String(s.clone()),
        }
    }
}

record! {
    /// `recipeSchema` (`kind`: `rocket staircase text lathe extrude mesh`).
    pub struct Recipe {
        kind: JsString = "kind",
        parameters: JsMap<RecipeParameter> = "parameters",
        profile: Vec<[f64; 2]> = "profile",
        mesh: RecipeMesh = "mesh",
        modifiers: Vec<Modifier> = "modifiers",
    }
}

record! {
    /// A generated part and its base transform.
    pub struct ModelPart {
        layer_id: JsString = "layerId",
        base: JsMap<f64> = "base",
    }
}

record! {
    /// `generatedModelSchema`: a model group's `d.modeling`.
    pub struct GeneratedModel {
        version: f64 = "version",
        recipe: Recipe = "recipe",
        p: JsMap<Channel> = "p",
        parts: JsMap<ModelPart> = "parts",
        retired: Vec<JsString> = "retired",
        source_asset_id: JsString = "sourceAssetId",
    }
}
