mod pro_fixture;
use pm_kernel::commands::pro_edit::{Edge, ProEditCommand};
use pro_fixture::*;
#[test]
fn current_main_media_timing_body_matches_native_authored_timing_at_three_rates() {
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/pro_main_reference.mjs"
        ))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reference: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        reference["reference"],
        "c7f46a05641b23944d3692c9d80f9677c7e1ef1d"
    );
    assert_eq!(
        reference["legacyBodyUnchanged"], true,
        "A06 baseline changed: re-audit instead of assuming tag identity"
    );
    assert_eq!(reference["fixtures"].as_array().unwrap().len(), 12);
    for case in reference["fixtures"].as_array().unwrap() {
        let mut s = store(vec![case["before"].clone()]);
        let delta = case["delta"].as_f64().unwrap();
        let command = match case["operation"].as_str().unwrap() {
            "move" => ProEditCommand::Move {
                layer_ids: ids(&["clip"]),
                delta,
                linked: false,
            },
            "slip" => ProEditCommand::Slip {
                layer_ids: ids(&["clip"]),
                delta,
                linked: false,
            },
            "trim-in" => ProEditCommand::Trim {
                layer_ids: ids(&["clip"]),
                edge: Edge::In,
                delta,
                linked: false,
            },
            "trim-out" => ProEditCommand::Trim {
                layer_ids: ids(&["clip"]),
                edge: Edge::Out,
                delta,
                linked: false,
            },
            _ => panic!(),
        };
        run(&mut s, command);
        let actual = layer(&s, "clip");
        let expected = pm_model::json::parse(&case["after"].to_string()).unwrap();
        for path in [
            &["from"][..],
            &["dur"],
            &["d", "trim", "v"],
            &["d", "speed"],
            &["p", "position.x", "kf", "0", "t"],
            &["p", "position.x", "kf", "1", "t"],
        ] {
            assert_eq!(
                n(&actual, path),
                n(&expected, path),
                "{} {} {path:?}",
                case["operation"],
                case["rate"]
            );
        }
        assert_eq!(actual.get("extensionData"), expected.get("extensionData"));
        assert_eq!(
            actual.get("d").unwrap().get("opaque"),
            expected.get("d").unwrap().get("opaque")
        );
    }
}
