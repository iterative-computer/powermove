use pm_mcp::effect;
use serde_json::{Value, json};
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
fn oracle(d: &Value) -> Value {
    let mut child = Command::new("node")
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/service-pairs.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"tool":"validate_effect","arguments":{"definition":d}})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn base() -> Value {
    json!({"id":"Synthetic.effect","label":"Synthetic","group":"Synthetic","params":[{"k":"amount","def":1}],"frag":"outColor=vec4(1.);"})
}
fn compare(d: Value) {
    let expected = oracle(&d);
    match effect::validate(&d) {
        Ok(value) => assert_eq!(value, expected["result"], "{d}"),
        Err(e) => assert_eq!(e.message, expected["error"], "{d}"),
    }
}
#[test]
fn typed_inputs_contextual_defaults_collisions_and_limits_match_main() {
    for params in [
        json!([{"k":"source","type":"layer"}]),
        json!([{"k":"path","type":"path"}]),
        json!([{"k":"lut","type":"asset","assetKind":"lut"}]),
        json!([{"k":"number","defaultFrom":{"dimension":"width","factor":0.5}}]),
        json!([{"k":"number","defaultFrom":{"dimension":"bad","factor":1}}]),
        json!([{"k":"color","type":"color","defaultFrom":{"dimension":"width","factor":1}}]),
        json!([{"k":"x","type":"bad"}]),
        json!([{"k":"source","type":"layer"},{"k":"sourceUV","def":1}]),
        json!([{"k":"layerUV","def":1}]),
        json!(
            (0..5)
                .map(|i| json!({"k":format!("source{i}"),"type":"layer"}))
                .collect::<Vec<_>>()
        ),
    ] {
        let mut d = base();
        d["params"] = params;
        compare(d);
    }
    for coordinates in [
        json!("layer"),
        json!("composition"),
        Value::Null,
        json!(true),
    ] {
        let mut d = base();
        d["coordinates"] = coordinates;
        compare(d);
    }
}
#[test]
fn sampling_temporal_source_grid_padding_and_path_contracts_match_main() {
    for (key, values) in [
        (
            "sampling",
            vec![
                json!({"radius":"amount","support":2,"maxRadius":64}),
                json!({"radius":"amount","support":0,"maxRadius":1}),
                json!({"radius":"missing","support":1,"maxRadius":1}),
                json!({"radius":"amount","support":2,"maxRadius":64,"firstPass":1}),
            ],
        ),
        (
            "temporal",
            vec![
                json!({"step":"amount","count":"amount","maxSamples":32}),
                json!({"step":"amount","count":"amount","maxSamples":33}),
            ],
        ),
        (
            "sourceProcessing",
            vec![
                json!({"padding":["amount"],"factor":1,"pixels":4096}),
                json!({"geometry":["missing"]}),
                json!({"padding":"amount"}),
                json!({"when":"amount"}),
                json!({"factor":9}),
            ],
        ),
        (
            "grid",
            vec![
                json!({"columns":"amount","rows":"amount"}),
                json!({"columns":"amount","rows":"missing"}),
            ],
        ),
        ("viewportPaddingFactor", vec![json!(0), json!(8), json!(9)]),
        (
            "viewportPaddingPixels",
            vec![json!(-1), json!(0), json!(4096), json!(4097)],
        ),
    ] {
        for value in values {
            let mut d = base();
            if key == "temporal" {
                d["rawShader"] = json!(true);
            }
            if key == "grid" {
                d["passes"] = json!(2);
            }
            d[key] = value;
            compare(d);
        }
    }
    for p in [
        json!({"k":"path","type":"path","pathWidth":"missing"}),
        json!({"k":"asset","type":"asset","assetKind":"video"}),
        json!({"k":"path","type":"path","pathBrush":{}}),
    ] {
        let mut d = base();
        d["params"].as_array_mut().unwrap().push(p);
        compare(d);
    }
}
#[test]
fn historical_revisions_and_parameter_actions_match_main() {
    for version in [
        Value::Null,
        json!(1),
        json!(0),
        json!(1.5),
        json!(9_007_199_254_740_992u64),
    ] {
        let mut d = base();
        d["version"] = version;
        compare(d);
    }
    let mut old = base();
    old["version"] = json!(1);
    let mut current = base();
    current["version"] = json!(2);
    current["legacy"] = json!([old]);
    compare(current.clone());
    current["legacy"][0]["version"] = json!(2);
    compare(current.clone());
    current["legacy"] = json!([base(), base()]);
    compare(current);
    for actions in [
        json!([]),
        json!([{"kind":"swap","label":"Swap","keys":["a","b"]}]),
        json!([{"kind":"swap","label":"Swap","keys":["a","a"]}]),
        json!([{"kind":"other","label":"Swap","keys":["a","b"]}]),
    ] {
        let mut d = base();
        d["params"] = json!([{"k":"a","type":"color"},{"k":"b","type":"color"}]);
        d["actions"] = actions;
        compare(d);
    }
}
