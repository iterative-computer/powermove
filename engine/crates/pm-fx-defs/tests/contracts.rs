use pm_fx_defs::{catalog::*, inputs::*, spatial::*};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn oracle() -> Value {
    serde_json::from_str(include_str!("main-values.json")).unwrap()
}
fn number(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

#[test]
fn spatial_plans_equal_values_executed_from_main() {
    let f = oracle();
    for row in f["sampling"].as_array().unwrap() {
        let a = &row["args"];
        assert_eq!(
            sample_lod(
                number(&a[0]),
                number(&a[1]),
                number(&a[2]),
                [number(&a[3][0]), number(&a[3][1])],
                a[4].as_u64().unwrap() as u32,
                a[5].as_u64().unwrap() as u32
            )
            .unwrap(),
            row["value"].as_u64().unwrap() as u32
        );
    }
    for row in f["grids"].as_array().unwrap() {
        let a = &row["args"];
        assert_eq!(
            json!(grid_size(number(&a[0]), number(&a[1]), a[2].as_u64().unwrap() as u32).unwrap()),
            row["value"]
        );
    }
    for row in f["mips"].as_array().unwrap() {
        let a = &row["args"];
        assert_eq!(
            mip_bytes(
                a[0].as_u64().unwrap() as u32,
                a[1].as_u64().unwrap() as u32,
                a[2].as_u64().unwrap() as u8,
                a[3].as_u64().unwrap() as u32
            )
            .unwrap(),
            row["value"].as_u64().unwrap()
        );
    }
}
#[test]
fn typed_input_validation_equals_main_including_null_and_utf16_boundaries() {
    for row in oracle()["inputs"].as_array().unwrap() {
        let result = validate_inputs(&row["args"]);
        if row["error"] == true {
            assert!(result.is_err(), "{}", row["args"]);
        } else {
            assert_eq!(
                serde_json::to_value(result.unwrap()).unwrap(),
                row["value"],
                "{}",
                row["args"]
            );
        }
    }
}
#[test]
fn temporal_limits_and_subframe_times_equal_main() {
    for row in oracle()["temporal"].as_array().unwrap() {
        let a = &row["args"];
        let result = temporal_times(
            number(&a[0]),
            number(&a[1]),
            number(&a[2]),
            a[3].as_u64().unwrap() as u32,
        );
        if row["error"] == true {
            assert!(result.is_err(), "{a}");
        } else {
            assert_eq!(json!(result.unwrap()), row["value"], "{a}");
        }
    }
}
#[test]
fn shutter_times_preserve_negative_shutter_order_and_exact_fps() {
    for row in oracle()["motionBlur"].as_array().unwrap() {
        let a = &row["args"];
        let result = motion_blur_times(
            number(&a[0]),
            number(&a[1]),
            a[2]["mbSamples"].as_f64(),
            a[2]["shutter"].as_f64(),
        )
        .unwrap();
        assert_eq!(
            result,
            serde_json::from_value::<Vec<f64>>(row["value"].clone()).unwrap(),
            "{a}"
        );
    }
    assert!(motion_blur_times(f64::NAN, 24., None, None).is_err());
}
#[test]
fn every_current_parameter_default_equals_main_at_source_sizes() {
    // Main's revision of each effect: the revisions made here keep main's
    // as history, and say what they change (`catalog::revise`).
    let pinned: Value = serde_json::from_str(include_str!("../src/main-effects.json")).unwrap();
    let main_version = |id: &str| {
        let effect = pinned["effects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap();
        effect_version(effect.get("version")).unwrap()
    };
    for row in oracle()["defaults"].as_array().unwrap() {
        let id = row["id"].as_str().unwrap();
        let def = main_catalog().get(id, main_version(id)).unwrap();
        let param = def.params.iter().find(|p| p.k == row["key"]).unwrap();
        assert_json_number_equivalent(
            parameter_default(
                param,
                number(&row["size"]["width"]),
                number(&row["size"]["height"]),
            ),
            row["value"].clone(),
        );
    }
}
#[test]
fn versions_equal_main_without_coercing_authored_values() {
    for row in oracle()["versions"].as_array().unwrap() {
        let result = effect_version(Some(&row["args"]));
        if row["value"].is_null() {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), row["value"].as_u64().unwrap());
        }
    }
}
#[test]
fn remap_is_static_and_does_not_rewrite_path_or_asset_ids() {
    let original = json!({"map":{"layerId":"L","stage":"masks","space":"source"},"path":{"kind":"path","layerId":"L","pathId":"L"},"lut":{"kind":"asset","assetId":"L"},"empty":{"layerId":null,"stage":"source"}});
    let mut refs = validate_inputs(&original).unwrap();
    remap_layers(&mut refs, &BTreeMap::from([("L".into(), "copy".into())]));
    let mut expected = original.clone();
    expected["map"]["layerId"] = json!("copy");
    expected["path"]["layerId"] = json!("copy");
    assert_eq!(serde_json::to_value(&refs).unwrap(), expected);
    remap_layers(&mut refs, &BTreeMap::from([("copy".into(), "L".into())]));
    assert_eq!(serde_json::to_value(&refs).unwrap(), original);
}
#[test]
fn exact_request_keys_do_not_alias_stages_spaces_prefixes_or_subframes() {
    let request = |stage, space, time, prefix| {
        Request::new("owner", stage, space, Some(time), prefix).unwrap()
    };
    let base = request(Stage::Effects, Space::Composition, 1.25, None);
    for other in [
        request(Stage::Source, Space::Composition, 1.25, None),
        request(Stage::Effects, Space::Source, 1.25, None),
        request(Stage::Effects, Space::Composition, 1.2500000000000002, None),
        request(Stage::Effects, Space::Composition, 1.25, Some(0)),
    ] {
        assert_ne!(base, other);
    }
    assert_eq!(
        request(Stage::Effects, Space::Source, -0., None),
        request(Stage::Effects, Space::Source, 0., None)
    );
    assert!(
        Request::new(
            "x",
            Stage::Effects,
            Space::Source,
            Some(f64::INFINITY),
            None
        )
        .is_err()
    );
}
#[test]
fn budgets_and_nonfinite_contracts_fail_before_allocation() {
    assert!(assert_input_budget(4, 100, 1000).is_ok());
    assert!(assert_input_budget(4, 100, 999).is_err());
    assert!(assert_input_budget(usize::MAX, u64::MAX, u64::MAX).is_err());
    assert!(sample_lod(1., 1., 0., [1., 1.], 10, 10).is_err());
    assert!(sample_lod(1., 1., 32., [f64::NAN, 1.], 10, 10).is_err());
    assert_eq!(sample_lod(f64::NAN, 1., 32., [1., 1.], 10, 10).unwrap(), 0);
    assert_eq!(grid_size(f64::NAN, f64::INFINITY, 4096).unwrap(), [1, 1]);
    assert!(mip_bytes(u32::MAX, u32::MAX, 32, 31).is_err());
    assert!(temporal_times(f64::INFINITY, 0., 0., 32).is_err());
    assert!(temporal_times(1., f64::NAN, 0., 32).is_err());
    assert_eq!(
        kernel_scale(Some([0., -3., 2., 0., 99., 99.]), [0.5, 2.]),
        [6., 1.]
    );
}
#[test]
fn source_bounds_preserve_off_comp_padding_and_temporal_hulls() {
    let source = Extent {
        x: -100.,
        y: 2000.,
        width: 20.,
        height: 10.,
    };
    let padded = source.pad(12.).unwrap();
    assert_eq!(
        padded,
        Extent {
            x: -112.,
            y: 1988.,
            width: 44.,
            height: 34.
        }
    );
    assert_eq!(
        padded
            .union(Extent {
                x: 0.,
                y: 0.,
                width: 10.,
                height: 10.
            })
            .unwrap(),
        Extent {
            x: -112.,
            y: 0.,
            width: 122.,
            height: 2022.
        }
    );
    assert!(source.pad(f64::INFINITY).is_err());
}

fn assert_json_number_equivalent(actual: Value, expected: Value) {
    if actual.is_number() && expected.is_number() {
        assert_eq!(actual.as_f64(), expected.as_f64());
    } else {
        assert_eq!(actual, expected);
    }
}
