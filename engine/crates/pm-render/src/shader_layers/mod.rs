//! Shader layers and extension fragment layers through the GLSL pipeline
//! (plan sections 10 and 11; amendment 9; decision 19). Design notes:
//! `docs/engine/shaders.md`.
//!
//! - **Shader layers** (`type: 'shader'`): `PM.SHADER_HEADER + '\n' +
//!   d.code`, drawn into a target of the layer's size at output scale
//!   (`min(4096, max(2, round(w · s)))`, the level 1 cap), with the
//!   compositor's uniforms (`iResolution`, layer-local `iTime`,
//!   `iGlobalTime`, `iProgress`, `iFrame` at the open composition's rate,
//!   `iMouse` = (0, 0)) and `d.uniforms` bound through the `@param`
//!   definitions [`parse_uniforms`] reads, with today's quirks: a single
//!   float never sets a `vec2`, `vec4`, or `int`, so those stay 0
//!   (`compositor.ts:1024-1030`).
//! - **Extension fragment layers** (Studio Cube included): the kernel's
//!   wrapped program from the definition snapshot (contract 10), the same
//!   uniforms, and `u_<param>` per parameter (colors `uniform3f`, toggles
//!   `uniform1i` on `uniform bool`, numbers `uniform1f`). A layer whose
//!   definition is marked missing draws the checkerboard placeholder.
//! - **The fallback runner** (decision 19, [`fallback`]): a program WebGL
//!   draws and the native pipeline cannot translate is rendered by a
//!   minimal WebGL2 runner in the renderer and uploaded like the `raster`
//!   override; without a runner (headless hosts) it is not
//!   headless-renderable and routes.
//!
//! Programs draw in GL row order like every pm-render target, as the
//! compositor's `grab` buffers, and are sampled through the layer's
//! transform as a `fromFbo` quad with its anchor at the origin.

pub mod exec;
pub mod fallback;
pub mod plan;
pub mod reports;
pub mod translate;
pub mod uniforms;

use std::sync::OnceLock;

use pm_protocol::definitions::DefinitionSnapshot;

pub use fallback::{
    Bands, FallbackBand, FallbackFailure, FallbackHost, FallbackJob, FallbackPixels, FallbackRequest,
    FallbackStore, FallbackUnit,
};
pub use plan::{ProgramRef, ShaderSource};
pub use reports::layer_reports;
pub use uniforms::{Control, UniformDef, UniformType, parse_float, parse_uniforms};

/// What the engine embeds of the app's shader-layer runtime
/// (`builtins.json`, written by `tools/shaders/builtins.ts`; the vitest
/// `kernel/shader-layer-builtins.test.ts` fails when it drifts).
#[derive(Debug, serde::Deserialize)]
pub struct Builtins {
    /// `PM.SHADER_HEADER`.
    pub header: String,
    /// The missing-layer checkerboard body (`compositor.ts`).
    pub missing: String,
    /// `PM.SHADER_TEMPLATE`.
    pub template: String,
    /// `PM.SHADER_PRESETS`, in their order.
    pub presets: Vec<Preset>,
    /// The built-in extension layer definitions (Studio Cube, the OBJ model
    /// layer) as the kernel snapshots them.
    pub layers: Vec<DefinitionSnapshot>,
}

#[derive(Debug, serde::Deserialize)]
pub struct Preset {
    pub name: String,
    pub code: String,
}

/// The embedded runtime.
pub fn builtins() -> &'static Builtins {
    static B: OnceLock<Builtins> = OnceLock::new();
    B.get_or_init(|| {
        serde_json::from_str(include_str!("builtins.json"))
            .expect("shader_layers/builtins.json is the shader-layer runtime")
    })
}

/// `PM.SHADER_HEADER`.
pub fn header() -> &'static str {
    &builtins().header
}

/// The program the compositor compiles for shader-layer code:
/// `PM.SHADER_HEADER + '\n' + code`, and the number of lines before the
/// code's first line (for diagnostics in the author's lines).
pub fn shader_layer_program(code: &str) -> (String, u32) {
    let header = header();
    let offset = header.matches('\n').count() as u32 + 1;
    (format!("{header}\n{code}"), offset)
}

/// The program of an extension layer whose definition is missing (or not a
/// fragment renderer that draws): the header and the checkerboard.
pub fn missing_layer_program() -> (String, u32) {
    shader_layer_program(&builtins().missing)
}

/// The built-in extension layer definitions (Studio Cube, the OBJ model
/// layer) as the kernel snapshots them.
pub fn builtin_layer_definitions() -> Vec<DefinitionSnapshot> {
    builtins().layers.clone()
}

/// [`crate::builtin_definitions`] (the 16 effects and 4 transitions) plus
/// [`builtin_layer_definitions`]: every built-in definition, for hosts
/// without a kernel (`pm-cli`, tests).
pub fn all_builtin_definitions() -> pm_protocol::definitions::DefinitionSetSnapshot {
    let mut set = crate::effects::builtin_definitions();
    set.definitions.extend(builtin_layer_definitions());
    set
}

/// The mesh layer definitions of a definition set (`renderer.kind ===
/// 'mesh'`, contract 10): `assetField` and every parameter's default, as
/// `meshExtensionQuad`'s `extensionParam` falls back to them.
pub fn mesh_definitions(
    effects: &crate::effects::Effects,
) -> std::collections::HashMap<String, crate::scene3d::legacy::MeshDefinition> {
    use pm_model::json::Value;
    use pm_protocol::definitions::{DefinitionKind, LayerRenderer, ParamSpec};
    effects
        .snapshot()
        .definitions
        .into_iter()
        .filter(|d| d.kind == DefinitionKind::Layer)
        .filter_map(|d| match &d.renderer {
            Some(LayerRenderer::Mesh { asset_field }) => Some((
                d.id.as_str().to_owned(),
                crate::scene3d::legacy::MeshDefinition {
                    asset_field: asset_field.clone(),
                    defaults: d
                        .params
                        .iter()
                        .map(|p| {
                            let v = match p {
                                ParamSpec::Number { default, .. } => Value::Number(*default),
                                ParamSpec::Color { default, .. } => {
                                    Value::String(default.as_str().into())
                                }
                                ParamSpec::Toggle { default, .. } => Value::Bool(*default),
                            };
                            (p.key().to_owned(), v)
                        })
                        .collect(),
                },
            )),
            _ => None,
        })
        .collect()
}

/// The built-in Studio Cube definition id.
pub const STUDIO_CUBE: &str = "powermove.3d.studio-cube";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runtime_is_embedded() {
        let b = builtins();
        assert!(b.header.starts_with("#version 300 es\n"));
        assert!(b.header.contains("uniform int iFrame;"));
        assert_eq!(b.presets.len(), 5);
        assert_eq!(
            b.presets[0].code, b.template,
            "Aurora Field is the template"
        );
        assert!(b.layers.iter().any(|l| l.id.as_str() == STUDIO_CUBE));
    }

    #[test]
    fn programs_put_the_code_after_the_header() {
        let (source, offset) = shader_layer_program("void main(){ fragColor = vec4(1.0); }");
        let line = source.lines().nth(offset as usize).unwrap();
        assert_eq!(line, "void main(){ fragColor = vec4(1.0); }");
    }
}
