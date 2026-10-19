//! 3D picks and bounds through the module's engine: model assets on
//! demand (`{"models": [ids]}`), the editor camera a request carries, and
//! the missing-model box.

use pm_model::Value;
use pm_model::json::parse;
use pm_wasm::Engine;
use pm_wasm::geometry::ModelKind;

fn ch(v: f64) -> String {
    format!(r#"{{"v":{v},"kf":[],"expr":null}}"#)
}

/// A 3D object layer with an OBJ model asset at the origin, a solid below.
fn project() -> String {
    let p = format!(
        r#"{{"position.x":{0},"position.y":{0},"position.z":{0},"rotation.x":{0},"rotation.y":{0},"rotation":{0},"scale.x":{1},"scale.y":{1},"scale.z":{1},"opacity":{1}}}"#,
        ch(0.0),
        ch(100.0)
    );
    format!(
        r##"{{"id":"P","name":"T","compId":"C","revision":0,"params":{{}},
          "assets":{{"m":{{"id":"m","kind":"model","format":"obj","name":"m.obj"}}}},
          "comps":{{"C":{{"id":"C","name":"Main","w":640,"h":360,"fps":30,"dur":10,"layers":[
            {{"id":"model","type":"extension","name":"Model","from":0,"dur":10,"on":true,"threeD":true,"parent":null,"p":{p},
              "d":{{"definition":"powermove.3d.object","data":{{"object":{{"source":{{"assetId":"m"}},"material":{{"p":{{"opacity":{o}}},"doubleSided":true}}}}}}}}}},
            {{"id":"below","type":"solid","name":"Below","from":0,"dur":10,"on":true,"parent":null,"p":{p},"d":{{"w":40,"h":40,"color":"#fff"}}}}
          ]}}}}}}"##,
        o = ch(1.0)
    )
}

const OBJ: &str = "v -2 -2 0\nv 2 -2 0\nv 2 2 0\nv -2 2 0\nf 1 2 3\nf 1 3 4\n";

fn q(e: &mut Engine, request: &str) -> Value {
    let reply = e.query(request.as_bytes());
    parse(&reply.text).unwrap_or_else(|err| panic!("{err}: {}", reply.text))
}

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

#[test]
fn model_assets_are_asked_for_once_then_picked() {
    let mut e = Engine::new();
    e.resync(project().as_bytes(), 1).unwrap();
    let pick = r#"{"op":"pick","comp":"C","open":"C","x":320,"y":180,"t":0,"includeLocked":false,"localView":null}"#;
    let first = q(&mut e, pick);
    let wanted = get(&first, "models")
        .and_then(Value::as_array)
        .expect("asks for the model");
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[0].as_str(), Some("m"));
    e.set_model("m", ModelKind::Obj, OBJ.as_bytes()).unwrap();
    // The quad faces +Z; the studio camera at (3, 2, 6) sees its front.
    let hit = q(&mut e, pick);
    assert_eq!(
        get(&hit, "ok")
            .and_then(Value::as_array)
            .map(|a| a[0].as_f64()),
        Some(Some(0.0)),
        "{hit:?}"
    );
    assert_eq!(e.model_ids(), vec!["m".to_owned()]);
    // Bounds of the object in composition pixels.
    let b = q(
        &mut e,
        r#"{"op":"bounds","comp":"C","open":"C","layer":0,"t":0}"#,
    );
    let b = get(&b, "ok").unwrap();
    let n = |k: &str| get(b, k).and_then(Value::as_f64).unwrap();
    assert!(
        n("x0") >= 0.0 && n("x1") <= 640.0 && n("x1") > n("x0"),
        "{b:?}"
    );
    // A missing asset draws the orange box: a smaller target, still hit at the center.
    e.set_model("m", ModelKind::Missing, &[]).unwrap();
    let hit = q(&mut e, pick);
    assert_eq!(
        get(&hit, "ok")
            .and_then(Value::as_array)
            .map(|a| a[0].as_f64()),
        Some(Some(0.0))
    );
    let corner = r#"{"op":"pick","comp":"C","open":"C","x":20,"y":20,"t":0,"includeLocked":false,"localView":null}"#;
    let below = q(&mut e, corner);
    assert_eq!(
        get(&below, "ok")
            .and_then(Value::as_array)
            .map(|a| a[0].as_f64()),
        Some(Some(1.0))
    );
    // An OBJ that does not parse (today's importer refuses it too) is kept
    // as missing, and the reason is named.
    let err = e
        .set_model("m", ModelKind::Obj, b"# nothing\n")
        .unwrap_err();
    assert!(err.starts_with("OBJ model m:"), "{err}");
    let hit = q(&mut e, pick);
    assert_eq!(
        get(&hit, "ok")
            .and_then(Value::as_array)
            .map(|a| a[0].as_f64()),
        Some(Some(0.0))
    );
}

/// A model three.js holds that this engine cannot load (a glTF `pm-geom3d`
/// rejects, an `object3d` without its source bytes) is a named failure for
/// every question about the scene: never the missing-model box.
#[test]
fn models_the_engine_cannot_load_fail_by_name() {
    let mut e = Engine::new();
    e.resync(project().as_bytes(), 1).unwrap();
    let pick = r#"{"op":"pick","comp":"C","open":"C","x":320,"y":180,"t":0,"includeLocked":false,"localView":null}"#;
    let bounds = r#"{"op":"bounds","comp":"C","open":"C","layer":0,"t":0}"#;
    let boxes = r#"{"op":"objectBoxes3d","comp":"C","open":"C","ids":["model"],"t":0}"#;
    let err = e
        .set_model("m", ModelKind::Gltf, b"not a glTF file")
        .unwrap_err();
    assert!(err.contains("cannot load the glTF model m"), "{err}");
    for request in [pick, bounds, boxes] {
        let reply = q(&mut e, request);
        let why = get(&reply, "unsupported").and_then(Value::as_str);
        assert!(
            why.is_some_and(|w| w.contains("cannot load the glTF model m")),
            "{request}: {reply:?}"
        );
    }
    e.set_model("m", ModelKind::Unloadable, b"no source bytes")
        .unwrap_err();
    let reply = q(&mut e, pick);
    assert_eq!(
        get(&reply, "unsupported").and_then(Value::as_str),
        Some("the engine cannot load the 3D model m: no source bytes")
    );
    // A loadable model answers again.
    e.set_model("m", ModelKind::Obj, OBJ.as_bytes()).unwrap();
    assert!(get(&q(&mut e, pick), "ok").is_some());
}

#[test]
fn the_editor_camera_decides_what_a_pick_sees() {
    let mut e = Engine::new();
    e.resync(project().as_bytes(), 1).unwrap();
    e.set_model("m", ModelKind::Obj, OBJ.as_bytes()).unwrap();
    // An editor camera looking at the quad from behind: a front-sided
    // material would miss, a double-sided one (as stored) hits.
    let world = [
        -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -8.0, 1.0,
    ];
    let aspect: f64 = 640.0 / 360.0;
    let (fov, near, far): (f64, f64, f64) = (50.0, 0.01, 1000.0);
    let top = near * (std::f64::consts::PI / 180.0 * 0.5 * fov).tan();
    let (h, w) = (2.0 * top, 2.0 * top * aspect);
    let projection = [
        2.0 * near / w,
        0.0,
        0.0,
        0.0,
        0.0,
        2.0 * near / h,
        0.0,
        0.0,
        0.0,
        0.0,
        -(far + near) / (far - near),
        -1.0,
        0.0,
        0.0,
        -2.0 * far * near / (far - near),
        0.0,
    ];
    let inv = |m: &[f64; 16]| pm_geom3d::gltf::math::invert(m);
    let camera = format!(
        r#"{{"world":{world:?},"view":{view:?},"projection":{projection:?},"projectionInverse":{pi:?},"orthographic":false,"fov":{fov},"zoom":1,"near":{near},"far":{far}}}"#,
        view = inv(&world),
        pi = inv(&projection)
    );
    let pick = format!(
        r#"{{"op":"pick","comp":"C","open":"C","x":320,"y":180,"t":0,"includeLocked":false,"localView":null,"camera":{camera}}}"#
    );
    let hit = q(&mut e, &pick);
    assert_eq!(
        get(&hit, "ok")
            .and_then(Value::as_array)
            .map(|a| a[0].as_f64()),
        Some(Some(0.0)),
        "{hit:?}"
    );
    // Local View without the model: the solid is not under the center either.
    let local = pick.replace(r#""localView":null"#, r#""localView":["other"]"#);
    let none = q(&mut e, &local);
    assert!(matches!(get(&none, "ok"), Some(Value::Null)), "{none:?}");
}
