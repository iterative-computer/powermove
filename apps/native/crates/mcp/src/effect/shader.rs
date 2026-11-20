//! Rust wrapper for the headless MCP tool. The preamble is shared with
//! `compat/src/frozen/host/glsl.js`; the parity test guards the assembly rules.
use serde::Deserialize;
use serde_json::Value;
use std::{fmt::Write, sync::LazyLock};

const PRELUDE_TS: &str =
    include_str!("../../../../js/compat/src/frozen/reference/legacy/gl/shader-prelude.ts");
static PRELUDE: LazyLock<String> = LazyLock::new(|| {
    let literal = PRELUDE_TS
        .lines()
        .find_map(|line| line.strip_prefix("export const GLSL_PRE = "))
        .and_then(|line| line.strip_suffix(';'))
        .expect("frozen effect prelude");
    serde_json::from_str(literal).expect("GLSL_PRE is a JSON-compatible string")
});
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EffectPreamble {
    scale: String,
    backdrop: String,
    viewer_handles: String,
}

static EXTENSIONS: LazyLock<EffectPreamble> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../js/compat/src/effect-shader-preamble.json"
    ))
    .expect("shared effect preamble declarations")
});

pub(super) fn wrap(def: &Value, params: &[Value], body: &str) -> (String, u32) {
    if super::truthy(&def["rawShader"]) {
        return (body.to_owned(), 0);
    }
    let mut prelude = (*PRELUDE).clone();
    prelude.push('\n');
    prelude.push_str(&EXTENSIONS.scale);
    if def["viewerHandles"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        prelude.push('\n');
        prelude.push_str(&EXTENSIONS.viewer_handles);
        prelude.push('\n');
    }
    if def["backdrop"] == true {
        prelude.push('\n');
        prelude.push_str(&EXTENSIONS.backdrop);
    }

    let mut declarations = String::new();
    let mut aliases = String::new();
    for (i, param) in params.iter().enumerate() {
        let key = param["k"].as_str().expect("validated effect parameter key");
        match param["type"].as_str() {
            Some("gradientStops") => {
                gradient_declarations(&mut declarations, key);
                declarations.push('\n');
            }
            Some("color") => {
                writeln!(declarations, "uniform vec3 u_{key};").unwrap();
                writeln!(aliases, "#define u_c{i} u_{key}").unwrap();
            }
            _ => {
                writeln!(declarations, "uniform float u_{key};").unwrap();
                writeln!(aliases, "#define u_p{i} u_{key}").unwrap();
            }
        }
    }
    declarations.push_str("uniform int u_pass;\n");
    if def["keepOrig"] == true {
        declarations.push_str("uniform sampler2D u_orig;\n");
    }
    declarations.push_str(&aliases);
    let mut source = format!("{prelude}\n{declarations}void main(){{\n");
    let offset = source.lines().count() as u32;
    write!(source, "{body}\n}}\n").unwrap();
    (source, offset)
}

// Keep in sync with gradientDeclarations in compat/src/viewer-handles.ts.
fn gradient_declarations(out: &mut String, key: &str) {
    writeln!(out, "uniform float u_{key}Count;").unwrap();
    for i in 0..16 {
        writeln!(out, "uniform float u_{key}P{i}; uniform vec4 u_{key}C{i};").unwrap();
    }
    writeln!(
        out,
        "vec4 pm_{key}(float t){{vec4 c=u_{key}C0; float p=u_{key}P0;"
    )
    .unwrap();
    for i in 1..16 {
        writeln!(out, "if(u_{key}Count>{i}.5){{if(t<=u_{key}P{i}) return mix(c,u_{key}C{i},clamp((t-p)/max(u_{key}P{i}-p,1e-8),0.,1.));c=u_{key}C{i};p=u_{key}P{i};}}").unwrap();
    }
    out.push_str("return c;}\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::process::Command;

    #[test]
    fn compile_tests_mcp_wrap_matches_the_runtime_byte_for_byte() {
        let settings = [
            json!({}),
            json!({"params": [{"k": "amount"}, {"k": "color", "type": "color"},
                {"k": "stops", "type": "gradientStops"}, {"k": "enabled", "type": "toggle"}],
                "passes": 3, "keepOrig": true, "backdrop": true, "viewerHandles": [{}]}),
            json!({"frag": "]", "backdrop": true}),
            json!({"frag": "}", "viewerHandles": [{}]}),
            json!({"rawShader": true, "backdrop": true, "frag": "#version 300 es\n"}),
        ];
        let definitions: Vec<_> = settings
            .into_iter()
            .map(|options| {
                let mut def = json!({"id": "sample.fx", "label": "Sample", "group": "Test",
                    "params": [], "frag": "\no = vec4(1.);"});
                def.as_object_mut()
                    .unwrap()
                    .extend(options.as_object().unwrap().clone());
                def
            })
            .collect();
        let script = format!(
            r#"import {{ definitionSnapshots }} from "./src/definitions";
const entries = {definitions}.map(item => ({{id:item.id, ownerId:"test", item}}));
const snapshots = definitionSnapshots({{ effects: {{entries: () => entries}},
  transitions: {{entries: () => []}}, layerTypes: {{entries: () => []}} }});
console.log(JSON.stringify(snapshots.map(s => s.glsl)));
"#,
            definitions = json!(definitions)
        );
        let output = Command::new("bun")
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../js/compat"))
            .args(["-e", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let snapshots: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(snapshots.len(), definitions.len());
        for (def, glsl) in definitions.iter().zip(snapshots) {
            let (source, offset) = wrap(
                def,
                def["params"].as_array().unwrap(),
                def["frag"].as_str().unwrap(),
            );
            assert_eq!(glsl["source"], source, "{def}");
            assert_eq!(glsl["bodyLineOffset"], offset, "{def}");
        }
    }
}
