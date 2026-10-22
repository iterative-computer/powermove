//! Bool, int, uint and float uniforms in the operators and conversions GLSL
//! ES 3.00 has: every program's verdict is WebGL's (ANGLE's), captured from
//! the app's renderer (`tests/fixtures/uniform-types.json`). An extension
//! layer's toggle is a `uniform bool` in today's program (the compositor's
//! `extensionLayerFragment`); comparing it with a float (`u_toggle > 0.5`)
//! does not compile in WebGL, so today's layer draws nothing and the engine
//! refuses it the same way, while every form WebGL accepts translates.

#[test]
fn uniform_types_compile_exactly_where_webgl_compiles_them() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/uniform-types.json")).unwrap();
    let template = fixture["template"].as_str().unwrap();
    let probes = fixture["probes"].as_array().unwrap();
    // The combinations: bool against float in relational, equality and
    // arithmetic operators; bool as a condition, negated, in logic, in ?:,
    // converted, and as mix's selector; int, uint and float likewise.
    assert!(probes.len() >= 30);
    let mut disagree = Vec::new();
    let (mut drawn, mut refused) = (0, 0);
    for probe in probes {
        let id = probe["id"].as_str().unwrap();
        let frag = template.replace("@STATEMENT@", probe["statement"].as_str().unwrap());
        let result = pm_glsl::cache::prepare_isolated(&frag, 0, pm_glsl::cache::DEFAULT_DEADLINE)
            .and_then(|p| p.finish().map(|_| ()));
        let agrees = match (probe["webgl"].as_str().unwrap(), &result) {
            ("drawn", Ok(())) => {
                drawn += 1;
                true
            }
            // Refused where WebGL's compile and link rules are reproduced.
            ("refused", Err(f)) if f.stage.webgl_refuses() => {
                refused += 1;
                true
            }
            _ => false,
        };
        if !agrees {
            disagree.push(format!(
                "{id}: WebGL {}, engine {}",
                probe["webgl"],
                match &result {
                    Ok(()) => "translates".to_owned(),
                    Err(f) => format!("refuses at {}: {}", f.stage.name(), f.message),
                }
            ));
        }
    }
    assert!(disagree.is_empty(), "{disagree:#?}");
    // Both kinds are covered.
    assert!(drawn >= 15 && refused >= 10, "{drawn} drawn, {refused} refused");
}

#[test]
fn a_toggle_compared_with_a_float_is_refused_with_webgls_message() {
    // The pattern of the private setup's layer (a `uniform bool` toggle
    // tested with `> 0.5`): WebGL says "wrong operand types"; so does the
    // engine, at the WebGL stage, never at translation.
    let frag = "#version 300 es\nprecision highp float;\nin vec2 v_uv;\nout vec4 o;\nuniform bool u_swap;\nvoid main() {\n  bool invert = true != (u_swap > 0.5);\n  o = vec4(float(invert), v_uv, 1.0);\n}\n";
    let failure = pm_glsl::cache::prepare_isolated(frag, 0, pm_glsl::cache::DEFAULT_DEADLINE)
        .and_then(|p| p.finish().map(|_| ()))
        .expect_err("WebGL refuses it");
    assert!(failure.stage.webgl_refuses(), "{failure:?}");
    assert!(failure.message.contains("wrong operand types"), "{}", failure.message);
}
