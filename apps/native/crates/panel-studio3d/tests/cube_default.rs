use pm_kernel::{Call, Caller, KernelActor, KernelConfig, OpenSource, Reply, Request, WriteBody};
use pm_scene3d::{from_model, to_model};
use serde_json::{Value, json};

fn fixture(layers: Value) -> KernelActor {
    let a = KernelActor::spawn(KernelConfig::default());
    a.handle()
        .open(
            "p",
            OpenSource::Untitled(to_model(&json!({"id":"p","compId":"c","w":640,"h":360,"fps":30,"dur":10,"layers":layers,"assets":{},"comps":{}}))),
        )
        .wait()
        .unwrap()
        .unwrap();
    a
}
fn add_cube(a: &KernelActor) {
    let published = a.handle().published("p").unwrap();
    let (label, commands, _) =
        pm_panel_studio3d::properties::add_layer(&published, 0., "box", "L1").unwrap();
    let ack = a
        .handle()
        .call(
            "p",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some(label),
                    body: WriteBody::Commands(commands),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
}
fn layers(a: &KernelActor) -> Vec<Value> {
    let root = from_model(
        &a.handle()
            .published("p")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
    )
    .unwrap();
    root["comps"]["c"]["layers"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| root["layers"].as_array().cloned().unwrap_or_default())
}

fn published_layers(a: &KernelActor) -> usize {
    a.handle().published("p").unwrap().history.entries
}
fn names(layers: &[Value]) -> Vec<String> {
    layers
        .iter()
        .map(|l| l["name"].as_str().unwrap_or_default().to_owned())
        .collect()
}
fn is_light(l: &Value) -> bool {
    l["d"]["definition"] == "powermove.3d.light"
}
fn is_world(l: &Value) -> bool {
    let d = &l["d"]["data"];
    d.get("world").is_some() || d["object"].get("world").is_some()
}
fn is_floor(l: &Value) -> bool {
    l["name"] == "Floor" || l["d"]["data"]["object"]["source"]["primitive"] == "plane"
}
fn v(l: &Value, k: &str) -> f64 {
    l["p"][k]["v"].as_f64().unwrap_or(0.)
}
/// What Layer ▸ Add 3D Layer ▸ Cube may add to a composition with no 3D yet:
/// the cube, and at most one key light. No floor, no world.
fn assert_only_cube_and_key(before: &[Value], after: &[Value]) {
    let added: Vec<&Value> = after
        .iter()
        .filter(|l| !before.iter().any(|b| b["id"] == l["id"]))
        .collect();
    let all = names(after);
    assert!(!after.iter().any(is_floor), "no floor: {all:?}");
    assert!(!after.iter().any(is_world), "no world backdrop: {all:?}");
    let cubes = added
        .iter()
        .filter(|l| l["d"]["data"]["object"]["source"]["primitive"] == "box")
        .count();
    let lights = added.iter().filter(|l| is_light(l)).count();
    assert_eq!(cubes, 1, "one cube: {all:?}");
    assert!(lights <= 1, "at most a key light: {all:?}");
    assert_eq!(added.len(), cubes + lights, "nothing else: {all:?}");
    // The 2D layers keep their order, below what was added.
    let kept: Vec<&Value> = after
        .iter()
        .filter(|l| before.iter().any(|b| b["id"] == l["id"]))
        .collect();
    assert_eq!(
        kept.iter().map(|l| &l["id"]).collect::<Vec<_>>(),
        before.iter().map(|l| &l["id"]).collect::<Vec<_>>()
    );
    assert_eq!(
        after[after.len() - before.len()..]
            .iter()
            .map(|l| &l["id"])
            .collect::<Vec<_>>(),
        before.iter().map(|l| &l["id"]).collect::<Vec<_>>(),
        "what was added sits above the 2D layers"
    );
}
fn two_d() -> Value {
    json!([
        {"id":"top","type":"solid","d":{"w":64,"h":64}},
        {"id":"bottom","type":"solid","d":{"w":640,"h":360}}
    ])
}

#[test]
fn a_cube_in_a_2d_composition_adds_only_the_cube_and_a_key_light_in_one_undo() {
    let a = fixture(two_d());
    let before = layers(&a);
    let history = published_layers(&a);
    add_cube(&a);
    let after = layers(&a);
    assert_only_cube_and_key(&before, &after);
    assert_eq!(published_layers(&a), history + 1, "one undo step");
    let ack = a
        .handle()
        .call(
            "p",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
    assert_eq!(
        names(&layers(&a)),
        names(&before),
        "undo removes everything added"
    );
}

#[test]
fn a_cube_in_an_empty_composition_has_no_floor_or_world() {
    let a = fixture(json!([]));
    add_cube(&a);
    assert_only_cube_and_key(&[], &layers(&a));
}

/// The default camera with no camera layer: at (0, 0, 6), aimed at the
/// origin, 45° vertical field of view (pm_scene3d's camera defaults).
fn projected_extent(cube: &Value) -> (f64, [f64; 2]) {
    let eye = [0., 0., 6.];
    let norm = |a: [f64; 3]| {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        a.map(|x| x / l)
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let f = norm([-eye[0], -eye[1], -eye[2]]);
    let r = norm(cross(f, [0., 1., 0.]));
    let u = cross(r, f);
    let t = 22.5f64.to_radians().tan();
    let source = &cube["d"]["data"]["object"]["source"];
    let n = |k: &str| source["parameters"][k].as_f64();
    let shape = pm_geom3d::primitives::Primitive::from_parameters(
        source["primitive"].as_str().unwrap(),
        &pm_geom3d::primitives::Parameters {
            width: n("width"),
            height: n("height"),
            depth: n("depth"),
            radius: n("radius"),
            radius_top: n("radiusTop"),
            radius_bottom: n("radiusBottom"),
            tube: n("tube"),
            segments: n("segments"),
            detail: n("detail"),
        },
    )
    .unwrap();
    let positions = shape.geometry().positions;
    let scale = [v(cube, "scale.x"), v(cube, "scale.y"), v(cube, "scale.z")].map(|s| s / 100.);
    let (rx, ry, rz) = (
        (v(cube, "rotation.x") + v(cube, "orientation.x")).to_radians(),
        (v(cube, "rotation.y") + v(cube, "orientation.y")).to_radians(),
        (v(cube, "rotation") + v(cube, "orientation.z")).to_radians(),
    );
    let at = [
        v(cube, "position.x"),
        v(cube, "position.y"),
        v(cube, "position.z"),
    ];
    let (mut xs, mut ys) = (vec![], vec![]);
    for v in positions.chunks_exact(3) {
        let mut p = [0usize, 1, 2].map(|k| v[k] as f64 * scale[k]);
        // X, then Y, then Z rotation.
        p = [
            p[0],
            p[1] * rx.cos() - p[2] * rx.sin(),
            p[1] * rx.sin() + p[2] * rx.cos(),
        ];
        p = [
            p[0] * ry.cos() + p[2] * ry.sin(),
            p[1],
            -p[0] * ry.sin() + p[2] * ry.cos(),
        ];
        p = [
            p[0] * rz.cos() - p[1] * rz.sin(),
            p[0] * rz.sin() + p[1] * rz.cos(),
            p[2],
        ];
        let d = [
            p[0] + at[0] - eye[0],
            p[1] + at[1] - eye[1],
            p[2] + at[2] - eye[2],
        ];
        let z = dot(d, f);
        xs.push(dot(d, r) / z / t);
        ys.push(dot(d, u) / z / t);
    }
    let span = |v: &[f64]| {
        let (lo, hi) = v
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        (hi - lo, (hi + lo) / 2.)
    };
    let (w, cx) = span(&xs);
    let (h, cy) = span(&ys);
    // Normalised device units span 2 per frame height.
    (w.max(h) / 2., [cx / 2., cy / 2.])
}

#[test]
fn the_cube_is_readable_centred_and_turned_three_quarters() {
    let a = fixture(two_d());
    add_cube(&a);
    let after = layers(&a);
    let cube = after
        .iter()
        .find(|l| l["d"]["data"]["object"]["source"]["primitive"] == "box")
        .unwrap();
    let (extent, centre) = projected_extent(cube);
    assert!(
        (0.35..=0.40).contains(&extent),
        "largest projected dimension {:.1}% of the frame height",
        extent * 100.
    );
    assert!(
        centre[0].abs() < 0.05 && centre[1].abs() < 0.05,
        "centred: {centre:?}"
    );
    // Seen from the straight-on default camera at (0, 0, 6), the cube starts
    // unrotated (0°), facing the viewer cleanly.
    let turn = (v(cube, "rotation.y") + v(cube, "orientation.y")).rem_euclid(90.);
    assert!(turn.abs() < 1e-6, "expected straight-on cube, got {turn:.1}°");
}

#[test]
fn a_cube_in_a_lit_3d_composition_adds_only_the_cube() {
    let a = fixture(two_d());
    add_cube(&a);
    let before = layers(&a);
    let published = a.handle().published("p").unwrap();
    let (label, commands, _) =
        pm_panel_studio3d::properties::add_layer(&published, 0., "box", "L2").unwrap();
    let ack = a
        .handle()
        .call(
            "p",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some(label),
                    body: WriteBody::Commands(commands),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
    let after = layers(&a);
    assert_eq!(after.len(), before.len() + 1, "{:?}", names(&after));
}

/// Every menu shape, added first to a 2D composition, spans about the same
/// share of the frame as the cube, centred.
#[test]
fn every_menu_shape_added_first_is_readable_and_centred() {
    for kind in [
        "box",
        "sphere",
        "plane",
        "cylinder",
        "cone",
        "torus",
        "capsule",
        "icosahedron",
    ] {
        let a = fixture(two_d());
        let before = layers(&a);
        let published = a.handle().published("p").unwrap();
        let (label, commands, _) =
            pm_panel_studio3d::properties::add_layer(&published, 0., kind, "L1").unwrap();
        let ack = a
            .handle()
            .call(
                "p",
                Request::new(
                    Caller::native(),
                    Call::Transact {
                        label: Some(label),
                        body: WriteBody::Commands(commands),
                        expected_revision: None,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert!(!matches!(ack.reply, Reply::Throw(_)), "{kind}: {:?}", ack.reply);
        let after = layers(&a);
        let shape = after.iter().find(|l| l["id"] == "L1").unwrap();
        let (extent, centre) = projected_extent(shape);
        assert!(
            (0.35..=0.40).contains(&extent),
            "{kind}: largest projected dimension {:.1}% of the frame height",
            extent * 100.
        );
        assert!(
            centre[0].abs() < 0.05 && centre[1].abs() < 0.05,
            "{kind}: centred: {centre:?}"
        );
        assert!(!after.iter().any(is_world), "{kind}: no world");
        assert_eq!(after.len(), before.len() + 2, "{kind}: the shape and its key");
    }
}
