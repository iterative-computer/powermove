//! The colour commands through a real kernel, down to rendered pixels.
use super::*;
use pm_kernel::{
    KernelActor, KernelConfig, KernelHandle,
    session::{SessionHandle, SessionSource, ShellCall, ShellReply},
};
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;

fn channel(value: f64) -> serde_json::Value {
    json!({ "v": value, "kf": [], "expr": null })
}

fn layer(id: &str, ty: &str, d: serde_json::Value) -> serde_json::Value {
    let p: serde_json::Map<_, _> = [
        ("position.x", 0.0),
        ("position.y", 0.0),
        ("position.z", 0.0),
        ("anchor.x", 0.0),
        ("anchor.y", 0.0),
        ("anchor.z", 0.0),
        ("scale.x", 100.0),
        ("scale.y", 100.0),
        ("scale.z", 100.0),
        ("rotation", 0.0),
        ("rotation.x", 0.0),
        ("rotation.y", 0.0),
        ("orientation.x", 0.0),
        ("orientation.y", 0.0),
        ("orientation.z", 0.0),
        ("opacity", 100.0),
        ("skew", 0.0),
        ("perspective", 50.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), channel(v)))
    .collect();
    json!({
        "id": id, "type": ty, "name": id, "from": 0, "dur": 1, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": p, "fx": [], "transitionIn": null, "transitionOut": null, "masks": [], "d": d
    })
}

fn fixture() -> (tempfile::TempDir, KernelActor, SessionHandle) {
    let dir = tempfile::tempdir().unwrap();
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path().join("journals")),
        ..Default::default()
    });
    let k = actor.handle();
    crate::mcp::attach(&k, dir.path()).unwrap();
    let solid = layer(
        "solid",
        "solid",
        json!({ "color": "#408040", "w": 16, "h": 16 }),
    );
    let mut clip = layer("clip", "video", json!({ "asset": "A", "w": 16, "h": 16 }));
    clip["on"] = json!(false);
    let project = json!({
        "id": "test", "name": "Base", "compId": "c", "w": 16, "h": 16, "fps": 30, "dur": 1,
        "bg": "#000000", "layers": [solid, clip], "comps": {},
        "assets": { "A": { "id": "A", "name": "A", "kind": "video" } }
    });
    let ShellReply::Opened { session, .. } = k
        .shell(ShellCall::Open {
            id: "test".into(),
            source: SessionSource::New(pm_model::json::parse(&project.to_string()).unwrap()),
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap()
    else {
        panic!("project opens")
    };
    (dir, actor, session)
}

fn published(k: &KernelHandle) -> Arc<Published> {
    k.published("test").unwrap()
}

#[test]
fn panel_settings_choose_what_the_engine_counts() {
    let config = scope_config(true, true);
    assert_eq!(config.signal, ScopeSignal::Pq);
    assert_eq!(config.matrix, ScopeMatrix::Rec2020);
    let config = scope_config(false, false);
    assert_eq!(config.signal, ScopeSignal::Display);
    assert_eq!(config.matrix, ScopeMatrix::Rec709);
    let more = more_menu(false, false, true);
    assert_eq!(more.len(), 5);
    assert_eq!(more[0]["checked"], true);
    assert_eq!(more[1]["checked"], false);
    assert_eq!(more[4]["disabled"], true);
    assert_eq!(more[4]["tooltip"], "This project has no values above SDR.");
}

#[test]
fn every_camera_reading_is_one_the_engine_parses() {
    for log in CAMERA_LOGS {
        let member = serde_json::json!({ "transfer": log.transfer, "primaries": log.primaries });
        let (reading, issues) = pm_media::interpret::Interpretation::from_json(&member);
        assert!(issues.is_empty(), "{}: {issues:?}", log.label);
        assert!(reading.transfer.is_some() && reading.primaries.is_some());
    }
}

#[test]
fn camera_log_is_one_undoable_interpretation_that_keeps_other_overrides() {
    let (_dir, actor, _session) = fixture();
    let k = actor.handle();
    let p = published(&k);
    let log = CAMERA_LOGS.iter().find(|l| l.transfer == "logC4").copied();
    let command = interpret_command(&p, "clip", log).unwrap();
    k.call_blocking("test", transact(&p, "Interpret Footage", vec![command]))
        .unwrap();
    let p = published(&k);
    let assets = p.snapshot.root().unwrap().get("assets").unwrap();
    let interpret = assets
        .get("A")
        .unwrap()
        .get("interpret")
        .unwrap()
        .to_value();
    assert_eq!(
        interpret.get("transfer").and_then(Value::as_str),
        Some("logC4")
    );
    assert_eq!(
        interpret.get("primaries").and_then(Value::as_str),
        Some("awgGen4")
    );
    let reading = pm_render::footage::interpretation(assets.get("A").unwrap()).unwrap();
    assert_eq!(reading.transfer, Some(pm_protocol::color::Transfer::LogC4));
    let command = interpret_command(&p, "clip", None).unwrap();
    k.call_blocking("test", transact(&p, "Interpret Footage", vec![command]))
        .unwrap();
    let p = published(&k);
    let assets = p.snapshot.root().unwrap().get("assets").unwrap();
    assert!(assets.get("A").unwrap().get("interpret").is_none());
    assert!(interpret_command(&p, "solid", log).is_err());
    actor.shutdown();
}

#[test]
fn applying_a_cube_reaches_rendered_pixels_through_the_kernel() {
    let (dir, actor, _session) = fixture();
    let k = actor.handle();
    let p = published(&k);
    let source = dir.path().join("Invert.cube");
    std::fs::write(
        &source,
        "LUT_3D_SIZE 2\n1 1 1\n0 1 1\n1 0 1\n0 0 1\n1 1 0\n0 1 0\n1 0 0\n0 0 0\n",
    )
    .unwrap();
    let retained = retain_lut(dir.path(), &source).unwrap();
    assert_ne!(retained, source);
    std::fs::remove_file(&source).unwrap();
    let commands = lut_commands(&p, "solid", "grade", "Invert.cube", &retained, &none()).unwrap();
    k.call_blocking("test", transact(&p, "Apply LUT", commands))
        .unwrap();
    let p = published(&k);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("grade".into(), retained);
    let mut renderer = Renderer::headless().unwrap();
    renderer
        .set_main_effects_for_document(&p.snapshot.document)
        .unwrap();
    let frame = renderer
        .render(
            &p.snapshot.document,
            &p.snapshot.open_comp().unwrap(),
            Ticks::ZERO,
            &RenderRequest::sized(Size::new(16, 16), Precision::Float32),
            &resolver,
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    let expected = [0x40, 0x80, 0x40].map(|v: u32| 1.0 - v as f32 / 255.0);
    for c in 0..3 {
        assert!((pixels[c] - expected[c]).abs() < 1e-3, "{:?}", &pixels[..4]);
    }
    actor.shutdown();
}

#[test]
fn the_clean_aperture_switches_on_and_off_and_keeps_other_overrides() {
    let (_dir, actor, _session) = fixture();
    let k = actor.handle();
    let apply = |command| {
        k.call_blocking(
            "test",
            transact(&published(&k), "Interpret Footage", vec![command]),
        )
        .unwrap();
    };
    let reading = || {
        let p = published(&k);
        let root = p.snapshot.root().unwrap();
        root.get("assets")
            .and_then(|assets| assets.get("A"))
            .and_then(pm_render::footage::interpretation)
            .unwrap_or_default()
    };
    apply(interpret_command(&published(&k), "clip", Some(CAMERA_LOGS[0])).unwrap());
    apply(clean_aperture_command(&published(&k), "clip").unwrap());
    assert_eq!(reading().clean_aperture, Some(true));
    assert!(reading().transfer.is_some(), "the log reading stays");
    apply(clean_aperture_command(&published(&k), "clip").unwrap());
    assert_eq!(reading().clean_aperture, None);
    assert!(reading().transfer.is_some(), "the log reading stays");
    assert!(clean_aperture_command(&published(&k), "solid").is_err());
    actor.shutdown();
}

#[test]
fn a_cube_fills_the_effects_browsers_empty_lut_effect() {
    use pm_kernel::commands::effects::{AddEffect, EffectCommand, saved_builtin_definition};
    let (dir, actor, _session) = fixture();
    let k = actor.handle();
    let p = published(&k);
    let definition = saved_builtin_definition(
        &p.snapshot.document,
        "applyColorLut",
        Some(&Value::from(2)),
        0.,
        0.,
    )
    .unwrap();
    let add = EffectCommand::Add(Box::new(AddEffect {
        layer_id: "solid".into(),
        effect_id: "browser".into(),
        definition,
        parameters: Map::new(),
        inputs: None,
        open: true,
        enabled: true,
        enabled_animation: None,
        index: None,
    }));
    k.call_blocking("test", transact(&p, "Add Effect", vec![add.into()]))
        .unwrap();
    let p = published(&k);
    let path = dir.path().join("Grade.cube");
    let commands = lut_commands(&p, "solid", "grade", "Grade.cube", &path, &none()).unwrap();
    k.call_blocking("test", transact(&p, "Apply LUT", commands))
        .unwrap();
    let p = published(&k);
    let root = p.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, "solid").unwrap();
    let effects = pm_kernel::model::layer_at(root, &at)
        .unwrap()
        .get("fx")
        .unwrap()
        .to_value();
    let effects = effects.as_array().unwrap();
    assert_eq!(effects.len(), 1, "the empty effect takes the file");
    let asset = effects[0].get("inputs").unwrap().get("lut").unwrap();
    assert_eq!(asset.get("assetId").unwrap().as_str(), Some("grade"));
    actor.shutdown();
}

fn none() -> std::collections::BTreeSet<String> {
    Default::default()
}

/// The layer's `fx`, as saved.
fn effects_of(k: &KernelHandle, layer: &str) -> Vec<serde_json::Value> {
    let p = published(k);
    let root = p.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, layer).unwrap();
    let effects = pm_kernel::model::layer_at(root, &at)
        .unwrap()
        .get("fx")
        .unwrap()
        .to_value();
    serde_json::from_str(&pm_model::json::to_canonical_string(&effects)).unwrap()
}

#[test]
fn applying_a_lut_relinks_the_one_whose_file_is_missing() {
    let (dir, actor, _session) = fixture();
    let k = actor.handle();
    let apply = |asset: &str, name: &str, missing: &std::collections::BTreeSet<String>| {
        let p = published(&k);
        let path = dir.path().join(name);
        let commands = lut_commands(&p, "solid", asset, name, &path, missing).unwrap();
        k.call_blocking("test", transact(&p, "Apply LUT", commands))
            .unwrap();
    };
    apply("first", "Look.cube", &none());
    // The file went away: the next LUT takes the same effect, and its format
    // follows the new file.
    apply("second", "Look.3dl", &["first".to_owned()].into());
    let effects = effects_of(&k, "solid");
    assert_eq!(effects.len(), 1, "the relink adds no second effect");
    assert_eq!(effects[0]["inputs"]["lut"]["assetId"], "second");
    let p = published(&k);
    let format = p.snapshot.root().unwrap().get("assets").unwrap();
    assert_eq!(
        format
            .get("second")
            .unwrap()
            .get("format")
            .unwrap()
            .to_value()
            .as_str(),
        Some("3dl")
    );
    // A LUT that still reads is left alone: another one stacks.
    apply("third", "Other.cube", &none());
    assert_eq!(effects_of(&k, "solid").len(), 2);
    actor.shutdown();
}

#[test]
fn a_cube_over_the_size_limit_is_refused_at_import() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Huge.cube");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(pm_color::lut::MAX_FILE_BYTES + 1).unwrap();
    assert_eq!(
        retain_lut(dir.path(), &path).unwrap_err(),
        "This LUT is too large."
    );
}

#[test]
fn a_malformed_cube_never_reaches_the_kernel() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Broken.cube");
    std::fs::write(&path, "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
    assert!(retain_lut(dir.path(), &path).is_err());
}
