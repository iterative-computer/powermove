use pm_color::effect_transforms::*;
fn profile(v: u64) -> Profile {
    match v {
        0 => Profile::Srgb,
        1 => Profile::DisplayP3,
        2 => Profile::Rec2020Gamma24,
        _ => panic!("unknown profile"),
    }
}
#[test]
fn profiles_and_transfers_match_values_executed_from_main() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../pm-fx-defs/tests/main-values.json")).unwrap();
    let mut worst: f64 = 0.;
    for row in fixture["profiles"].as_array().unwrap() {
        let a = &row["args"];
        let rgb = std::array::from_fn(|i| a[0][i].as_f64().unwrap());
        let actual = convert_profile(
            rgb,
            profile(a[1].as_u64().unwrap()),
            profile(a[2].as_u64().unwrap()),
            a[3].as_bool().unwrap(),
            a[4].as_bool().unwrap(),
        );
        for (i, value) in actual.iter().enumerate() {
            let error = (*value - row["value"][i].as_f64().unwrap()).abs();
            worst = worst.max(error);
            assert!(error <= 1e-12, "{a}: {error}");
        }
    }
    for row in fixture["transfers"].as_array().unwrap() {
        let a = &row["args"];
        let value = a[0].as_f64().unwrap();
        let p = profile(a[1].as_u64().unwrap());
        for (key, actual) in [
            ("encode", encode_transfer(value, p)),
            ("decode", decode_transfer(value, p)),
        ] {
            let error = (actual - row[key].as_f64().unwrap()).abs();
            worst = worst.max(error);
            assert!(error <= 1e-12, "{key} {a}: {error}");
        }
    }
    eprintln!(
        "main f64 transform oracle: 216 profiles + 60 transfer results; maximum absolute error {worst:e}"
    );
}
#[test]
fn legacy_preview_import_treatment_matches_main_without_altering_original_tags() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../pm-fx-defs/tests/main-values.json")).unwrap();
    for row in fixture["video"].as_array().unwrap() {
        let a = &row["args"];
        let expected = match row["value"].as_str() {
            Some("tonemap") => Some(VideoColorTreatment::ToneMap),
            Some("preserve") => Some(VideoColorTreatment::PreserveCodeValues),
            None => None,
            _ => panic!("unknown treatment"),
        };
        assert_eq!(
            video_color_treatment(
                a["transfer"].as_str(),
                a["primaries"].as_str(),
                a["matrix"].as_str()
            ),
            expected,
            "{a}"
        );
    }
}
#[test]
fn compander_round_trip_preserves_signed_hdr_and_partial_alpha() {
    for alpha in [0.001, 0.25, 1.] {
        for gain in [0.001, 1., 4., 65504.] {
            for gamma in [0.1, 1., 2.2, 10.] {
                let original = [-0.25 * alpha, 2. * alpha, 8. * alpha, alpha];
                let compressed = compander_pixel(original, CompanderMode::Compress, gain, gamma);
                assert!(compressed[0] < 0.);
                let expanded = compander_pixel(compressed, CompanderMode::Expand, gain, gamma);
                for i in 0..4 {
                    assert!(
                        (expanded[i] - original[i]).abs() <= 1e-12,
                        "{gain} {gamma}: {expanded:?}"
                    );
                }
            }
        }
    }
    for alpha in [0., -1.] {
        let original = [1., -2., 8., alpha];
        assert_eq!(
            compander_pixel(original, CompanderMode::Compress, 4., 2.2),
            original
        );
    }
}
#[test]
fn clipping_is_explicit_amount_is_bounded_and_zero_alpha_light_survives() {
    let original = [-0.25, 2., 8., 0.5];
    let transform = ProfileTransform {
        input: Profile::Srgb,
        output: Profile::Srgb,
        linear_input: false,
        linear_output: false,
        clip: false,
        amount: 100.,
    };
    assert_eq!(transform.pixel(original), original);
    assert_eq!(
        ProfileTransform {
            clip: true,
            ..transform
        }
        .pixel(original),
        [0., 0.5, 0.5, 0.5]
    );
    assert_eq!(
        ProfileTransform {
            clip: true,
            amount: 0.,
            ..transform
        }
        .pixel(original),
        original
    );
    for alpha in [0., -1.] {
        let original = [1., -2., 8., alpha];
        assert_eq!(
            ProfileTransform {
                output: Profile::DisplayP3,
                clip: true,
                ..transform
            }
            .pixel(original),
            original
        );
    }
}
