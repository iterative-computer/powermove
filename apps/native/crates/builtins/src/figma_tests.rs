//! Today's `clipboard.test.ts` and `convert.test.ts`. Archives are synthetic
//! designs encoded with the real Figma schema by today's encoder (fixtures);
//! planner cases build node changes directly, as today's tests do.
use super::*;
use pm_kernel::{Command, DocumentStore, LayerCommand, Origin};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/figma/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
/// `archiveHtml`: the clipboard HTML Figma writes around the archive.
fn archive(schema: &[u8], message: &[u8]) -> String {
    let mut bytes = b"fig-kiwi".to_vec();
    bytes.extend(15u32.to_le_bytes());
    for chunk in [schema, message] {
        bytes.extend((chunk.len() as u32).to_le_bytes());
        bytes.extend(chunk);
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        r#"<meta charset="utf-8"><span data-metadata="<!--(figmeta)e30=(/figmeta)-->"></span><span data-buffer="<!--(figma){encoded}(/figma)-->"></span>"#
    )
}
fn html(message: &str) -> String {
    archive(&fixture("schema.deflate"), &fixture(message))
}

fn o(pairs: Vec<(&str, K)>) -> K {
    K::Obj(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}
fn k(v: f64) -> K {
    K::Num(v)
}
fn t(v: &str) -> K {
    K::Str(v.into())
}
fn guid(local: f64) -> K {
    o(vec![("sessionID", k(7.)), ("localID", k(local))])
}
fn matrix(m: [f64; 6]) -> K {
    o(vec![
        ("m00", k(m[0])),
        ("m01", k(m[1])),
        ("m02", k(m[2])),
        ("m10", k(m[3])),
        ("m11", k(m[4])),
        ("m12", k(m[5])),
    ])
}
fn solid(r: f64, g: f64, b: f64) -> K {
    o(vec![
        ("type", t("SOLID")),
        (
            "color",
            o(vec![("r", k(r)), ("g", k(g)), ("b", k(b)), ("a", k(1.))]),
        ),
        ("opacity", k(1.)),
        ("visible", K::Bool(true)),
    ])
}
/// `figmaNode`: a 100 × 60 orange rectangle unless patched.
fn node(local: f64, kind: &str, patch: Vec<(&str, K)>) -> K {
    let mut m = match o(vec![
        ("guid", guid(local)),
        ("phase", t("CREATED")),
        ("type", t(kind)),
        ("name", t(&format!("Layer {local}"))),
        ("size", o(vec![("x", k(100.)), ("y", k(60.))])),
        ("transform", matrix([1., 0., 0., 0., 1., 0.])),
        ("fillPaints", K::List(vec![solid(1., 0.25, 0.)])),
    ]) {
        K::Obj(m) => m,
        _ => unreachable!(),
    };
    for (key, value) in patch {
        m.insert(key.into(), value);
    }
    K::Obj(m)
}
fn child(of: f64, position: &str) -> K {
    o(vec![("guid", guid(of)), ("position", t(position))])
}
const TARGET: Target = Target {
    w: 800.,
    h: 600.,
    fps: 30.,
    dur: 10.,
};
fn plan_of(nodes: Vec<K>, blobs: Vec<Vec<u8>>, at: f64) -> Result<Plan, String> {
    plan(&Selection { nodes, blobs }, TARGET, at, &["Arial".into()])
}
fn prop(layer: &BlockLayer, key: &str) -> f64 {
    layer
        .add
        .properties
        .get(key)
        .and_then(Value::as_f64)
        .unwrap()
}
fn content<'a>(layer: &'a BlockLayer, key: &str) -> &'a Value {
    layer.add.content.get(key).unwrap()
}
/// `pathBlob`: M 0 0 L 100 0 L 0 60 Z.
fn path_blob() -> Vec<u8> {
    let mut bytes = vec![];
    for (op, x, y) in [(1u8, 0f32, 0f32), (2, 100., 0.), (2, 0., 60.)] {
        bytes.push(op);
        bytes.extend(x.to_le_bytes());
        bytes.extend(y.to_le_bytes());
    }
    bytes.push(0);
    bytes
}

#[test]
fn deflate_and_zstd_archives_read_through_their_embedded_schema() {
    let card = decode(&html("card.deflate")).unwrap();
    assert_eq!(card.nodes.len(), 3);
    assert_eq!(text(&card.nodes[0], "type"), Some("GROUP"));
    assert_eq!(num(&card.nodes[0], "opacity"), Some(0.5));
    assert_eq!(transform(&card.nodes[0]), [0., 1., -1., 0., 1200., 500.]);
    assert_eq!(position(&card.nodes[2]), "\"");
    let vector = decode(&html("vector.zstd")).unwrap();
    assert_eq!(text(&vector.nodes[0], "name"), Some("Flèche → 日本語"));
    assert_eq!(vector.blobs, [path_blob()]);
}

#[test]
fn missing_or_malformed_data_is_refused_without_reading_html_as_code() {
    for html in [
        "",
        "<p>normal HTML</p>",
        "<!--(figma)bad!(/figma)-->",
        "<!--(figma)AA==(/figma)-->",
    ] {
        assert!(decode(html).is_err(), "{html}");
    }
    let mut bytes = base64::engine::general_purpose::STANDARD
        .decode(
            html("card.deflate")
                .split("(figma)")
                .nth(1)
                .unwrap()
                .split("(/figma)")
                .next()
                .unwrap(),
        )
        .unwrap();
    bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    let truncated = format!(
        "(figma){}(/figma)",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    assert!(decode(&truncated).unwrap_err().contains("incomplete"));
}

#[test]
fn limits_hold_before_allocating() {
    let many = archive(&fixture("many.schema.deflate"), &fixture("many.deflate"));
    assert_eq!(
        decode(&many).unwrap_err(),
        "Copy up to 5,000 Figma layers at a time."
    );
    let lengths = archive(&fixture("array.schema.deflate"), &fixture("array.deflate"));
    assert!(decode(&lengths).unwrap_err().contains("array length"));
    let deflate = |bytes: &[u8]| {
        use std::io::Write;
        let mut e = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    };
    let schema = archive(&deflate(&[1, 65]), &deflate(&[0]));
    assert!(
        decode(&schema).unwrap_err().contains("schema"),
        "{:?}",
        decode(&schema)
    );
    let window = archive(
        &fixture("array.schema.deflate"),
        &[0x28, 0xb5, 0x2f, 0xfd, 0, 0xff],
    );
    assert!(decode(&window).unwrap_err().contains("too large"));
}

#[test]
fn nested_rotated_groups_keep_stacking_local_poses_and_fill_alpha() {
    let selection = decode(&html("card.deflate")).unwrap();
    let plan = plan(&selection, TARGET, 3., &[]).unwrap();
    assert_eq!(plan.issues, []);
    let names: Vec<_> = plan
        .layers
        .iter()
        .map(|l| l.add.name.clone().unwrap())
        .collect();
    assert_eq!(names, ["Card", "Top", "Bottom"]);
    let (group, top, bottom) = (&plan.layers[0], &plan.layers[1], &plan.layers[2]);
    assert_eq!(plan.roots, std::slice::from_ref(&group.add.id));
    assert_eq!(top.group.as_deref(), Some(group.add.id.as_str()));
    assert_eq!(bottom.group.as_deref(), Some(group.add.id.as_str()));
    assert_eq!(content(top, "shape").as_str(), Some("ellipse"));
    assert_eq!(content(bottom, "color").as_str(), Some("#FF4000FF"));
    assert_eq!((bottom.add.from, bottom.add.duration), (Some(3.), Some(7.)));
    // The selection is centred: its 100 × 200 bounds start at (350, 200).
    assert_eq!(
        (prop(group, "position.x"), prop(group, "position.y")),
        (450., 200.)
    );
    assert_eq!(prop(group, "rotation"), 90.);
    assert_eq!(prop(group, "opacity"), 50.);
    // Primitive shapes centre on their origin; Figma's is the top-left corner.
    assert_eq!(
        (prop(bottom, "anchor.x"), prop(bottom, "anchor.y")),
        (-50., -30.)
    );
    assert_eq!(
        (prop(top, "position.x"), prop(top, "position.y")),
        (100., 40.)
    );
    assert!((prop(bottom, "opacity") - 80.).abs() < 1e-4);
    assert_eq!((plan.width, plan.height), (100., 200.));

    // One step into a document, with names and group-local values kept.
    let mut store = DocumentStore::new_untitled(
        pm_model::json::parse(r##"{"id":"P","name":"T","compId":"C","w":800,"h":600,"fps":30,"dur":10,"bg":"#000000","assets":{},"layers":[]}"##).unwrap(),
    );
    store
        .execute(
            Command::Layer(LayerCommand::InsertBlock {
                label: "Paste Figma layers".into(),
                layers: plan.layers.clone(),
            }),
            Origin::Interface,
        )
        .unwrap();
    let layers = store.layers();
    assert_eq!(
        layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        ["Card", "Top", "Bottom"]
    );
}

#[test]
fn frame_backgrounds_sit_beneath_children() {
    let selection = decode(&html("frame.deflate")).unwrap();
    let plan = plan(&selection, TARGET, 2., &[]).unwrap();
    assert_eq!(plan.issues, []);
    let kinds: Vec<_> = plan
        .layers
        .iter()
        .map(|l| l.add.layer_type.as_str())
        .collect();
    assert_eq!(kinds, ["group", "shape", "shape"]);
    let background = &plan.layers[2];
    assert_eq!(background.add.name.as_deref(), Some("Layer 1 background"));
    assert_eq!(prop(background, "opacity"), 100.);
    assert_eq!(
        plan.layers[0]
            .add
            .properties
            .get("opacity")
            .and_then(Value::as_f64),
        Some(50.)
    );
}

#[test]
fn typography_keeps_the_baseline_alignment_and_spacing() {
    let selection = decode(&html("text.deflate")).unwrap();
    let plan = plan(&selection, TARGET, 3., &["Arial".into()]).unwrap();
    assert_eq!(plan.issues, []);
    let layer = &plan.layers[0];
    assert_eq!(layer.add.layer_type, "text");
    assert_eq!(content(layer, "text").as_str(), Some("Hello\nWorld"));
    assert_eq!(content(layer, "font").as_str(), Some("Arial"));
    for (key, value) in [
        ("size", 20.),
        ("weight", 600.),
        ("tracking", 2.),
        ("leading", 1.5),
        ("boxWidth", 100.),
    ] {
        assert_eq!(content(layer, key).as_f64(), Some(value), "{key}");
    }
    assert_eq!(content(layer, "italic").as_bool(), Some(true));
    assert_eq!(content(layer, "align").as_str(), Some("center"));
    assert_eq!(prop(layer, "position.x"), 400.);
    assert!((prop(layer, "position.y") - 275.6).abs() < 1e-6);
}

#[test]
fn vector_commands_become_editable_vertices_without_moving() {
    let selection = decode(&html("vector.zstd")).unwrap();
    let plan = plan(&selection, TARGET, 3., &[]).unwrap();
    assert_eq!(plan.issues, []);
    let path = &content(&plan.layers[0], "paths").as_array().unwrap()[0];
    let v = |vertex: &Value, key: &str| {
        vertex
            .get("p")
            .unwrap()
            .get(key)
            .unwrap()
            .get("v")
            .unwrap()
            .as_f64()
            .unwrap()
    };
    let points: Vec<_> = path
        .get("vertices")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (v(p, "x"), v(p, "y")))
        .collect();
    assert_eq!(points, [(0., 0.), (100., 0.), (0., 60.)]);
    assert_eq!(
        path.get("p")
            .unwrap()
            .get("closed")
            .unwrap()
            .get("v")
            .unwrap()
            .as_bool(),
        Some(true)
    );
    // Editable paths keep authored coordinates: no centring anchor.
    assert_eq!(prop(&plan.layers[0], "anchor.x"), 0.);
}

#[test]
fn unsupported_features_ask_for_the_fallback_before_anything_applies() {
    let font = |family: &str| {
        o(vec![
            ("family", t(family)),
            ("style", t("Regular")),
            ("postscript", t("")),
        ])
    };
    let cases: Vec<(&str, Vec<(&str, K)>)> = vec![
        ("INSTANCE", vec![]),
        (
            "RECTANGLE",
            vec![(
                "effects",
                K::List(vec![o(vec![("type", t("DROP_SHADOW"))])]),
            )],
        ),
        ("RECTANGLE", vec![("mask", K::Bool(true))]),
        (
            "RECTANGLE",
            vec![(
                "fillPaints",
                K::List(vec![o(vec![("type", t("GRADIENT_LINEAR"))])]),
            )],
        ),
        (
            "RECTANGLE",
            vec![
                ("strokePaints", K::List(vec![o(vec![("type", t("SOLID"))])])),
                ("strokeAlign", t("INSIDE")),
            ],
        ),
        ("TEXT", vec![("fontName", font("Missing font"))]),
        (
            "TEXT",
            vec![(
                "textData",
                o(vec![
                    ("characters", t("Mixed")),
                    ("characterStyleIDs", K::List(vec![k(0.), k(1.)])),
                ]),
            )],
        ),
        (
            "VECTOR",
            vec![(
                "fillGeometry",
                K::List(vec![o(vec![("commandsBlob", k(9.))])]),
            )],
        ),
    ];
    for (kind, patch) in cases {
        let label = format!("{kind} {patch:?}");
        let plan = plan_of(vec![node(1., kind, patch)], vec![], 0.).unwrap();
        assert!(!plan.issues.is_empty(), "{label}");
    }
    let shadow = plan_of(
        vec![node(
            1.,
            "RECTANGLE",
            vec![(
                "effects",
                K::List(vec![o(vec![("type", t("DROP_SHADOW"))])]),
            )],
        )],
        vec![],
        0.,
    )
    .unwrap();
    assert_eq!(reasons(None, &shadow.issues), "“Layer 1”: effects");
    assert_eq!(
        reasons(Some("Unsupported Figma format"), &[]),
        "Unsupported Figma format"
    );
}

#[test]
fn cycles_are_refused_instead_of_dropping_layers() {
    let nodes = vec![
        node(1., "GROUP", vec![("parentIndex", child(2., "!"))]),
        node(2., "GROUP", vec![("parentIndex", child(1., "!"))]),
    ];
    assert!(plan_of(nodes, vec![], 0.).is_err());
}

#[test]
fn only_an_inline_png_data_uri_is_a_preview() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z/D/PwAHggJ/Pchr2QAAAABJRU5ErkJggg==";
    let bytes = inline_png(&format!(
        r#"<span></span><IMG alt="x" SRC="data:image/png;base64,{png}">"#
    ))
    .unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
    assert_eq!(inline_png(r#"<img src="https://example.com/a.png">"#), None);
    assert_eq!(inline_png("<p>no image</p>"), None);
    assert_eq!(style_weight("Extra Light"), 200.);
    assert_eq!(style_weight("SemiBold"), 600.);
    assert_eq!(style_weight("Bold Italic"), 700.);
    assert_eq!(style_weight("Regular"), 400.);
}
