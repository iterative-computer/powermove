//! The `edit.*` requests: pm-edit's kernels over the mirror, their replies,
//! and the operations applied back the way the renderer applies them.

use pm_model::json::parse;
use pm_model::{Value, binary, ops};
use pm_wasm::Engine;

/// A layer with every 2D transform channel (as `PM.mkLayer` makes them),
/// with `values` overriding the defaults (`"name": v` or a whole channel
/// JSON).
fn layer(
    id: &str,
    kind: &str,
    from: f64,
    parent: &str,
    group: &str,
    values: &[(&str, &str)],
) -> String {
    let defaults = [
        ("anchor.x", "0"),
        ("anchor.y", "0"),
        ("position.x", "0"),
        ("position.y", "0"),
        ("scale.x", "100"),
        ("scale.y", "100"),
        ("rotation", "0"),
        ("skew", "0"),
        ("opacity", "100"),
    ];
    let channels: Vec<String> = defaults
        .iter()
        .map(|(name, v)| {
            let v = values
                .iter()
                .find(|(n, _)| n == name)
                .map_or(*v, |(_, v)| *v);
            if v.starts_with('{') {
                format!(r#""{name}":{v}"#)
            } else {
                format!(r#""{name}":{{"v":{v},"kf":[],"expr":null}}"#)
            }
        })
        .collect();
    format!(
        r##"{{"id":"{id}","type":"{kind}","name":"{id}","from":{from},"dur":10,"on":true,"parent":{parent},"group":{group},"p":{{{}}},"d":{{"color":"#ff0000","w":200,"h":100,"radius":0}},"fx":[],"masks":[]}}"##,
        channels.join(",")
    )
}

/// A parent chain (B under a rotating A, from 1 s) and a rotated, faded
/// group G holding M, on a 29.97 composition.
fn project() -> String {
    let layers = [
        layer(
            "A",
            "solid",
            0.0,
            "null",
            "null",
            &[
                ("position.x", "100"),
                ("position.y", "50"),
                ("scale.x", "150"),
                (
                    "rotation",
                    r#"{"v":0,"kf":[{"t":0,"v":0,"i":"k1"},{"t":2,"v":90,"i":"k2"}],"expr":null}"#,
                ),
            ],
        ),
        layer(
            "B",
            "solid",
            1.0,
            r#""A""#,
            "null",
            &[("position.x", "10"), ("position.y", "20")],
        ),
        layer(
            "G",
            "group",
            0.0,
            "null",
            "null",
            &[
                ("position.x", "300"),
                ("position.y", "200"),
                ("rotation", "30"),
                ("opacity", "50"),
            ],
        ),
        layer(
            "M",
            "solid",
            0.0,
            "null",
            r#""G""#,
            &[("position.x", "40"), ("opacity", "80")],
        ),
    ];
    format!(
        r#"{{"id":"P1","name":"Test","compId":"C1","revision":0,"assets":{{}},"params":{{}},"comps":{{"C1":{{"id":"C1","name":"Main","w":1920,"h":1080,"fps":29.97,"dur":2,"layers":[{}]}}}}}}"#,
        layers.join(",")
    )
}

fn engine() -> Engine {
    let mut e = Engine::new();
    e.resync(project().as_bytes(), 1).unwrap();
    e
}

fn q(e: &mut Engine, request: &str) -> Value {
    let reply = e.query(request.as_bytes());
    parse(&reply.text).unwrap_or_else(|err| panic!("{err}: {}", reply.text))
}

fn member<'v>(v: &'v Value, k: &str) -> &'v Value {
    v.as_object().and_then(|m| m.get(k)).unwrap_or_else(|| {
        panic!(
            "no {k} in {}",
            pm_model::json::write::to_canonical_string(v)
        )
    })
}

fn ok(e: &mut Engine, request: &str) -> Value {
    member(&q(e, request), "ok").clone()
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

/// Applies an edit reply's operations to the mirror as the renderer's own
/// write would reach it.
fn apply(e: &mut Engine, reply: &Value) {
    apply_with(e, reply, &[]);
}

/// [`apply`] plus the caller's own writes (leaf patches as JSON).
fn apply_with(e: &mut Engine, reply: &Value, own: &[&str]) {
    let mut list = ops::ops_from_value(member(reply, "ops")).unwrap();
    assert!(!list.is_empty());
    for patch in own {
        list.push(ops::Op::from_value(&parse(patch).unwrap()).unwrap());
    }
    let rev = e.revision();
    e.apply(&binary::encode(&list).unwrap(), rev, rev + 1)
        .unwrap();
}

fn close(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() <= 1e-9 * (1.0 + y.abs()), "{a:?} vs {b:?}");
    }
}

#[test]
fn reparenting_keeps_the_world_pose() {
    let mut e = engine();
    let t = 0.5;
    let before = nums(&ok(
        &mut e,
        &format!(r#"{{"op":"worldMatrix","comp":"C1","layer":1,"t":{t}}}"#),
    ));
    let reply = ok(
        &mut e,
        &format!(r#"{{"op":"edit.reparent","comp":"C1","layer":1,"parent":null,"t":{t}}}"#),
    );
    // The kernel never writes the mirror itself.
    assert_eq!(e.revision(), 1);
    assert_eq!(member(&reply, "next").as_f64(), Some(1.0));
    // The kernel keeps the pose; the `parent` member is the caller's write
    // (as `setLayer` writes it).
    apply_with(
        &mut e,
        &reply,
        &[r#"{"exists":true,"path":["comps","C1","layers",1,"parent"],"value":null}"#],
    );
    let after = nums(&ok(
        &mut e,
        &format!(r#"{{"op":"worldMatrix","comp":"C1","layer":1,"t":{t}}}"#),
    ));
    close(&after, &before);
}

#[test]
fn ungroup_bakes_keys_on_the_frame_grid() {
    let mut e = engine();
    // A is animated, so dissolving the rotated, faded group bakes M.
    let world = |e: &mut Engine, layer: usize, t: f64| {
        nums(&ok(
            e,
            &format!(r#"{{"op":"worldMatrix","comp":"C1","layer":{layer},"t":{t}}}"#),
        ))
    };
    let before: Vec<Vec<f64>> = (0..60)
        .map(|n| world(&mut e, 3, n as f64 / 29.97))
        .collect();
    let reply = ok(
        &mut e,
        r#"{"op":"edit.ungroup","comp":"C1","groups":[2],"t":0.25,"uid":{"base":"x","start":7}}"#,
    );
    assert_eq!(
        member(&reply, "ids")
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["M"]
    );
    assert!(member(&reply, "next").as_f64().unwrap() > 7.0);
    apply(&mut e, &reply);
    // The group is gone; M (now index 2) keeps its world pose at every frame.
    for (n, want) in before.iter().enumerate() {
        close(&world(&mut e, 2, n as f64 / 29.97), want);
    }
    // Created key times are the document's float frame grid (n / 29.97),
    // and created ids come from the request's id source.
    let doc = parse(&e.to_json()).unwrap();
    let m = &member(member(member(&doc, "comps"), "C1"), "layers")
        .as_array()
        .unwrap()[2];
    let keys = member(member(member(m, "p"), "position.x"), "kf")
        .as_array()
        .unwrap();
    let times: Vec<f64> = keys
        .iter()
        .map(|k| member(k, "t").as_f64().unwrap())
        .collect();
    for n in 0..60 {
        assert!(
            times.contains(&(n as f64 / 29.97)),
            "frame {n} in {times:?}"
        );
    }
    assert!(times.contains(&0.25));
    assert!(
        keys.iter()
            .all(|k| member(k, "i").as_str().unwrap().starts_with("kx"))
    );
    // M's opacity takes the group's.
    let opacity = member(member(member(m, "p"), "opacity"), "v").as_f64();
    assert_eq!(opacity, Some(40.0));
}

#[test]
fn align_answers_moves_and_throws_are_reported() {
    let mut e = engine();
    let reply = ok(
        &mut e,
        r#"{"op":"edit.align","comp":"C1","layers":[0,3],"mode":"left","t":0}"#,
    );
    let moves = member(&reply, "moves").as_array().unwrap();
    assert!(!moves.is_empty());
    for m in moves {
        assert!(member(m, "value").as_f64().unwrap().is_finite());
    }
    // Unknown layers and edits are throws, not panics.
    let bad = q(
        &mut e,
        r#"{"op":"edit.reparent","comp":"C1","layer":9,"parent":null,"t":0}"#,
    );
    assert!(bad.as_object().unwrap().get("throw").is_some());
    let unknown = q(&mut e, r#"{"op":"edit.nope","comp":"C1","t":0}"#);
    assert!(unknown.as_object().unwrap().get("throw").is_some());
    let no_comp = q(
        &mut e,
        r#"{"op":"edit.reparent","comp":"ZZ","layer":0,"t":0}"#,
    );
    assert!(no_comp.as_object().unwrap().get("throw").is_some());
}

/// A text layer whose `d.animators` is not an array: the legacy split throws
/// `(source.d.animators || []).map is not a function` after laying the text
/// out.
fn text_project(animators: &str) -> String {
    let text = layer("T", "text", 0.0, "null", "null", &[]).replace(
        r##""d":{"color":"#ff0000","w":200,"h":100,"radius":0}"##,
        &format!(
            r##""d":{{"text":"Hello world","size":40,"font":"Inter","weight":600,"align":"left","color":"#ffffff","animators":{animators}}}"##
        ),
    );
    format!(
        r#"{{"id":"P2","name":"Text","compId":"C1","revision":0,"assets":{{}},"params":{{}},"comps":{{"C1":{{"id":"C1","name":"Main","w":1920,"h":1080,"fps":30,"dur":2,"layers":[{text}]}}}}}}"#
    )
}

#[test]
fn a_throw_after_measuring_without_fonts_is_unanswered() {
    let mut e = Engine::new();
    e.resync(text_project("5").as_bytes(), 1).unwrap();
    // No fonts are served: the layout misses Inter, so the kernel's throw
    // (raised after the layout) is not reported; the renderer runs the
    // legacy split instead.
    let reply = q(
        &mut e,
        r#"{"op":"edit.splitText","comp":"C1","sources":[0],"mode":"words","t":0}"#,
    );
    let reply = reply.as_object().unwrap();
    assert!(reply.get("throw").is_none(), "{reply:?}");
    assert!(reply.get("fonts").is_some(), "{reply:?}");
    // The font anchor capture measures too.
    let capture = q(
        &mut e,
        r#"{"op":"edit.captureFontAnchor","comp":"C1","layer":0,"fields":["size"],"t":0}"#,
    );
    assert!(capture.as_object().unwrap().get("fonts").is_some());
    // A throw before any measurement is still the kernel's throw.
    let bad = q(
        &mut e,
        r#"{"op":"edit.splitText","comp":"C1","sources":[3],"mode":"words","t":0}"#,
    );
    assert!(bad.as_object().unwrap().get("throw").is_some());
}

/// A camera (keyframed, at 0) and a group holding a solid; with `looped`
/// the group is parented to the camera.
fn camera_project(looped: bool) -> String {
    let mut camera = layer(
        "CAM",
        "extension",
        0.0,
        "null",
        "null",
        &[(
            "position.x",
            r#"{"v":0,"kf":[{"t":0,"v":0,"i":"k1"},{"t":2,"v":400,"i":"k2"}],"expr":null}"#,
        )],
    );
    let target = |v: f64| format!(r#"{{"v":{v},"kf":[],"expr":null}}"#);
    camera = camera.replace(
        r##""d":{"color":"#ff0000","w":200,"h":100,"radius":0}"##,
        &format!(
            r#""threeD":true,"d":{{"definition":"powermove.3d.camera","data":{{"camera":{{"p":{{"targetX":{},"targetY":{},"targetZ":{}}}}}}}}}"#,
            target(10.0),
            target(20.0),
            target(30.0)
        ),
    );
    assert!(camera.contains("powermove.3d.camera"));
    let parent = if looped { r#""CAM""# } else { "null" };
    let layers = [
        camera,
        layer("G", "group", 0.0, parent, "null", &[("rotation", "30")]),
        layer("S", "solid", 0.0, "null", r#""G""#, &[("position.x", "40")]),
    ];
    format!(
        r#"{{"id":"P1","name":"Test","compId":"C1","revision":0,"assets":{{}},"params":{{}},"comps":{{"C1":{{"id":"C1","name":"Main","w":1920,"h":1080,"fps":29.97,"dur":2,"layers":[{}]}}}}}}"#,
        layers.join(",")
    )
}

#[test]
fn a_camera_reparented_into_a_space_that_moves_with_it_is_answered_natively() {
    let request = r#"{"op":"edit.reparent","comp":"C1","layer":0,"parent":2,"t":0.5}"#;
    // S sits in G, which hangs from the camera: the kernel reads the camera's
    // keyed channels as the legacy evaluation cache has them (from before the
    // pose write), as `preserveParentPose` does. The replay
    // (pm-edit/tests/edit_parity.rs, `selfSpace` cases) holds the values to
    // the legacy edit's.
    for looped in [true, false] {
        let mut e = Engine::new();
        e.resync(camera_project(looped).as_bytes(), 1).unwrap();
        let reply = ok(&mut e, request);
        assert!(
            !ops::ops_from_value(member(&reply, "ops"))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn baked_keys_settle_on_the_tick_grid_for_off_grid_starts() {
    // M starts off the 29.97 grid: its baked keys are written at the legacy
    // float times while the ungroup samples, then moved to their ticks.
    let mut e = Engine::new();
    e.resync(
        project()
            .replace(
                r#""id":"M","type":"solid","name":"M","from":0"#,
                r#""id":"M","type":"solid","name":"M","from":0.4333"#,
            )
            .as_bytes(),
        1,
    )
    .unwrap();
    let reply = ok(
        &mut e,
        r#"{"op":"edit.ungroup","comp":"C1","groups":[2],"t":0.25}"#,
    );
    apply(&mut e, &reply);
    let doc = parse(&e.to_json()).unwrap();
    let m = &member(member(member(&doc, "comps"), "C1"), "layers")
        .as_array()
        .unwrap()[2];
    assert_eq!(member(m, "from").as_f64(), Some(0.4333));
    let keys = member(member(member(m, "p"), "position.x"), "kf")
        .as_array()
        .unwrap();
    assert!(keys.len() > 60);
    const TICKS: f64 = 705_600_000.0;
    for k in keys {
        let t = member(k, "t").as_f64().unwrap();
        let ticks = t * TICKS;
        assert!(
            (ticks - ticks.round()).abs() < 1e-3,
            "{t} is not on the tick grid"
        );
    }
    // Frame 30 less the start, on 30000/1001 rather than the float grid.
    let frame = 30.0 * 1001.0 / 30000.0;
    let want = ((frame * TICKS).round() - (0.4333 * TICKS).round()) / TICKS;
    assert!(
        keys.iter().any(|k| member(k, "t").as_f64() == Some(want)),
        "{want}"
    );
}
