use pm_fx_defs::audio::{Binding, Definition, Parameter, Processor, parse};
use serde_json::{Value, json};
use std::collections::BTreeMap;
fn definition(graph: Value) -> Value {
    json!({"id":"test.fx","label":"Test","group":"Audio","params":[],"graph":graph})
}
fn gain() -> Value {
    definition(json!([{"type":"gain","gainDb":{"value":0}}]))
}
#[test]
fn typed_parameters_and_bindings_round_trip() {
    let value = json!({"id":"test.fx","label":"Test","group":"Audio","params":[
        {"type":"number","k":"gain","label":"Gain","min":-24,"max":24,"unit":"dB","def":0},
        {"type":"toggle","k":"on","label":"On","def":true},
        {"type":"select","k":"mode","label":"Mode","options":["lowPass","highPass"],"def":"lowPass"}
    ],"graph":[
        {"type":"gain","gainDb":{"param":"gain"}},
        {"type":"gate","enabled":{"param":"on"}},
        {"type":"biquad","mode":{"param":"mode"},"frequency":{"value":1000},"q":{"value":0.7},"gainDb":{"value":0}}
    ]});
    let valid = parse(value.clone()).unwrap();
    assert_eq!(
        parse(serde_json::to_value(valid.definition()).unwrap()).unwrap(),
        valid
    );
    valid
        .resolve(&BTreeMap::from([
            ("gain".into(), json!(6)),
            ("on".into(), json!(false)),
            ("mode".into(), json!("highPass")),
        ]))
        .unwrap();
    for (key, value) in [
        ("gain", json!(25)),
        ("on", json!(1)),
        ("mode", json!("notch")),
        ("unknown", json!(1)),
    ] {
        let error = valid
            .resolve(&BTreeMap::from([(key.into(), value)]))
            .unwrap_err();
        assert!(error.to_string().contains(key));
    }
}
#[test]
fn every_processor_setting_rejects_values_outside_its_domain_by_field() {
    for (node, fields) in [
        (
            json!({"type":"gain","gainDb":{"value":0}}),
            vec![("gainDb", -96.0, 24.0)],
        ),
        (
            json!({"type":"biquad","mode":{"value":"lowPass"},"frequency":{"value":1000},"q":{"value":0.7},"gainDb":{"value":0}}),
            vec![
                ("frequency", 10.0, 20000.0),
                ("q", 0.1, 20.0),
                ("gainDb", -24.0, 24.0),
            ],
        ),
        (
            json!({"type":"compressor","threshold":{"value":-18},"ratio":{"value":4},"attack":{"value":0.01},"release":{"value":0.1},"knee":{"value":6},"makeup":{"value":0}}),
            vec![
                ("threshold", -96.0, 0.0),
                ("ratio", 1.0, 100.0),
                ("attack", 0.0001, 1.0),
                ("release", 0.001, 10.0),
                ("knee", 0.0, 24.0),
                ("makeup", 0.0, 24.0),
            ],
        ),
        (
            json!({"type":"limiter","ceiling":{"value":-1},"release":{"value":0.1}}),
            vec![("ceiling", -96.0, 0.0), ("release", 0.001, 10.0)],
        ),
        (
            json!({"type":"delay","time":{"value":0.1},"feedback":{"value":0.5},"mix":{"value":0.5},"maxTime":1}),
            vec![
                ("time", 1.0 / 8000.0, 1.0),
                ("feedback", -0.95, 0.95),
                ("mix", 0.0, 1.0),
            ],
        ),
    ] {
        parse(definition(json!([node.clone()]))).unwrap();
        for (key, lo, hi) in fields {
            for invalid in [lo - 1.0, hi + 1.0] {
                let mut bad = node.clone();
                bad[key]["value"] = json!(invalid);
                let e = parse(definition(json!([bad]))).unwrap_err().to_string();
                assert!(e.contains(&format!("graph[0].{key}")), "{e}");
            }
        }
    }
}
#[test]
fn malformed_metadata_references_and_memory_limits_have_named_errors() {
    let mut cases = vec![];
    for (key, value) in [
        ("id", json!("")),
        ("label", json!("")),
        ("group", json!("")),
        ("params", json!(false)),
        ("graph", json!([])),
        ("extra", json!(1)),
    ] {
        let mut bad = gain();
        bad[key] = value;
        cases.push((bad, key));
    }
    let mut bad = gain();
    bad["graph"][0]["gainDb"] = json!({"param":"missing"});
    cases.push((bad, "gainDb"));
    let mut bad = gain();
    bad["graph"][0]["gainDb"] = json!({"value":0,"param":"gain"});
    cases.push((bad, "gainDb"));
    let mut bad = gain();
    bad["graph"] = Value::Array(vec![json!({"type":"gain","gainDb":{"value":0}}); 33]);
    cases.push((bad, "graph"));
    for max_time in [0.0, 2.1] {
        cases.push((definition(json!([{"type":"delay","time":{"value":0.1},"feedback":{"value":0},"mix":{"value":1},"maxTime":max_time}])),"maxTime"));
    }
    cases.push((definition(json!([
        {"type":"delay","time":{"value":1},"feedback":{"value":0},"mix":{"value":1},"maxTime":2},
        {"type":"delay","time":{"value":1},"feedback":{"value":0},"mix":{"value":1},"maxTime":2}
    ])),"maxTime"));
    let mut bad = gain();
    bad["params"] =
        json!([{"type":"number","k":"gain","label":"Gain","min":-96,"max":25,"unit":"dB","def":0}]);
    bad["graph"][0]["gainDb"] = json!({"param":"gain"});
    cases.push((bad, "gainDb.param.max"));
    for (bad, field) in cases {
        let error = parse(bad).unwrap_err().to_string();
        assert!(error.contains(field), "{field}: {error}");
    }
}
#[test]
fn nonfinite_native_descriptions_never_validate() {
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let def = Definition {
            id: "test.fx".into(),
            label: "Test".into(),
            group: "Audio".into(),
            params: vec![],
            graph: vec![Processor::Gain {
                gain_db: Binding::Constant { value: v },
            }],
        };
        assert!(def.validate().unwrap_err().to_string().contains("gainDb"));
        let def = Definition {
            id: "test.fx".into(),
            label: "Test".into(),
            group: "Audio".into(),
            params: vec![Parameter::Number {
                k: "gain".into(),
                label: "Gain".into(),
                min: v,
                max: 24.0,
                unit: "dB".into(),
                def: 0.0,
            }],
            graph: vec![Processor::Gain {
                gain_db: Binding::Parameter {
                    param: "gain".into(),
                },
            }],
        };
        assert!(def.validate().unwrap_err().to_string().contains("min"));
    }
}
#[test]
fn random_descriptions_never_panic_or_accept_out_of_range_gains() {
    // Reproducible mutation fuzzing, no dependency or platform entropy.
    let mut seed = 0x34ac6b81u64;
    for _ in 0..20_000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let v = (seed as i64 as f64 / i64::MAX as f64) * 1000.0;
        let mut value = gain();
        value["graph"][0]["gainDb"]["value"] = json!(v);
        let accepted = std::panic::catch_unwind(|| parse(value))
            .expect("validator panicked")
            .is_ok();
        assert_eq!(accepted, (-96.0..=24.0).contains(&v));
        let junk = match seed % 8 {
            0 => Value::Null,
            1 => json!(true),
            2 => json!(seed),
            3 => json!([]),
            4 => json!({"graph":false}),
            5 => json!({"params":[v]}),
            6 => json!({"type":"delay","maxTime":v}),
            _ => {
                json!({"id":"test","label":"Test","group":"Audio","params":[],"graph":[{"type":"gain","gainDb":v}]})
            }
        };
        assert!(
            std::panic::catch_unwind(|| parse(junk))
                .expect("validator panicked")
                .is_err()
        );
    }
}
