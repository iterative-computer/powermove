//! What the shader UI is told about shader and extension layers (contract
//! 7 `FailureReport`, plan section 11: `uiState.getShaderMeta` and the
//! compile errors "filled from engine render events at the frame's
//! revision").
//!
//! Today the compositor's `program()` fills `GL.errors` and
//! `setShaderMeta({shaderKey})` when it draws a shader layer, and a missing
//! extension layer sets `missing: true`. With the engine presenting, the
//! WebGL compositor draws nothing, so the engine reports both:
//!
//! - a shader layer whose code WebGL refuses too (`Verdict::Refused`):
//!   `ShaderCompile`, layer scope, with the diagnostics in the author's
//!   lines (the header and the joining newline removed);
//! - an extension layer whose definition is marked missing (it draws the
//!   checkerboard): `DefinitionMissing`, layer scope.
//!
//! These are statements about the document at a revision, not about one
//! frame's draws: [`layer_reports`] checks every shader and extension layer
//! of the composition and the compositions it nests, drawn at the frame's
//! time or not, so the Shader panel shows a hidden layer's error too. The
//! interactive presenter sends the whole list at every revision it
//! presents (and what changed when only the definitions change), before
//! the frame; the app treats a layer report older than the frame it is
//! showing as cleared (`kernel/engine-definitions.ts`).
//!
//! Extension fragment programs that do not compile are definition failures
//! (`DefinitionsApplied.failures`, definition scope), as the effects lane
//! reports them; programs only the fallback runner draws compile in WebGL
//! and are not errors for the Shader panel (the presenter logs them).

use std::collections::BTreeSet;

use pm_eval::node;
use pm_model::{Document, Node};
use pm_protocol::definitions::DefinitionKind;
use pm_protocol::error::{EngineError, ErrorCode, ErrorDetail, ErrorScope, Retry};

use super::translate::{self, Verdict};
use crate::capability::{composition_node, precomp_target};
use crate::effects::Effects;
use crate::js;

/// The layer reports of `composition` and every composition it nests, in
/// stack order, one per layer. `definitions` are the renderer's (`None`:
/// extension layers route and nothing is said about them);
/// `mesh_definitions` names mesh layer definitions the 3D pass draws
/// without a snapshot (the built-in OBJ model layer).
pub fn layer_reports(
    document: &Document,
    composition: &str,
    definitions: Option<&Effects>,
    is_mesh: &dyn Fn(&str) -> bool,
) -> Vec<EngineError> {
    let Some(root) = document.root() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut reported = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![composition.to_owned()];
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(comp) = composition_node(root, &id) else {
            continue;
        };
        let layers = node::array(node::member(Some(comp), "layers")).unwrap_or(&[]);
        for layer in layers {
            let layer_id = js::layer_id(layer);
            if layer_id.is_empty() || reported.contains(&layer_id) {
                continue;
            }
            let report = match js::layer_type(layer).as_str() {
                "shader" => shader_compile(layer, &layer_id),
                "extension" => definitions.and_then(|e| missing(layer, &layer_id, e, is_mesh)),
                "precomp" => {
                    if let Some((nested, _)) = precomp_target(root, layer) {
                        stack.push(nested);
                    }
                    None
                }
                _ => None,
            };
            if let Some(report) = report {
                reported.insert(layer_id);
                out.push(report);
            }
        }
    }
    out
}

fn name(layer: &Node, id: &str) -> String {
    node::string(node::member(Some(layer), "name"))
        .map(|n| n.to_string_lossy())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| id.to_owned())
}

/// `ShaderCompile` for a shader layer whose code WebGL refuses too.
fn shader_compile(layer: &Node, id: &str) -> Option<EngineError> {
    // `PM.SHADER_HEADER + '\n' + d.code`, as the plan wraps it (a missing
    // code is the text "undefined").
    let code = node::member(node::member(Some(layer), "d"), "code");
    let code = node::js_value_opt(code).to_js_string().to_string_lossy();
    let (source, line_offset) = super::shader_layer_program(&code);
    let Verdict::Refused(failure) = translate::verdict(&source, line_offset).1 else {
        return None;
    };
    let message = crate::effects::truncate(format!(
        "Shader layer {} does not compile ({}: {}).",
        name(layer, id),
        failure.stage.name(),
        failure.message.lines().next().unwrap_or_default()
    ));
    Some(
        EngineError::new(
            ErrorCode::ShaderCompile,
            ErrorScope::Layer { layer: id.into() },
            message,
        )
        .with_retry(Retry::AfterUserAction)
        .with_detail(ErrorDetail::ShaderCompile {
            diagnostics: crate::effects::diagnostics(&failure),
        }),
    )
}

/// `DefinitionMissing` for an extension layer drawn as the checkerboard
/// (`crate::shader_layers::plan::extension_layer`).
fn missing(
    layer: &Node,
    id: &str,
    effects: &Effects,
    is_mesh: &dyn Fn(&str) -> bool,
) -> Option<EngineError> {
    let definition = node::member(node::member(Some(layer), "d"), "definition")
        .map(|d| node::js_value_opt(Some(d)).to_js_string().to_string_lossy())
        .unwrap_or_default();
    // 3D roles and legacy scene layers are the 3D pass's; known mesh
    // definitions draw their model.
    if pm_eval::parenting::Role3d::of(layer).is_some()
        || definition == crate::scene3d::legacy::SCENE3D_DEFINITION
        || is_mesh(&definition)
        || effects.get(DefinitionKind::Layer, &definition).is_some()
        || !effects.is_missing(DefinitionKind::Layer, &definition)
    {
        return None;
    }
    let message = crate::effects::truncate(format!(
        "Extension layer {} needs {}, which is not installed; it shows a placeholder.",
        name(layer, id),
        if definition.is_empty() {
            "an extension".to_owned()
        } else {
            format!("the extension that defines {definition}")
        }
    ));
    Some(EngineError::new(
        ErrorCode::DefinitionMissing,
        ErrorScope::Layer { layer: id.into() },
        message,
    ))
}
