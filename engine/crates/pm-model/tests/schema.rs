//! The typed schema is a lossless lens: every project fixture in the
//! repository decodes and encodes back to an identical value (numbers by bit
//! pattern), unknown members survive at every level, and key order never
//! matters.
//!
//! Fixtures covered:
//!
//! * the 35 synthetic corpus projects (`tests/fixtures/corpus`);
//! * the onboarding scene (`apps/desktop/src/renderer/public/onboarding/scene.json`)
//!   and the www hero scene (`apps/www/static/hero/scene.json`);
//! * every JSON, PMV3, and PMV4 file in `tests/fixtures/containers` that
//!   `decodeProjectContainer` accepts (written by the app's own encoders);
//! * the synthetic projects in `tests/vectors/canonical-vectors.json`;
//! * the Blender material sample in `apps/desktop/docs/SCENE3D.md`.

use std::path::{Path, PathBuf};

use pm_model::canonical::{canonical_project, project_of};
use pm_model::container::{ReadOptions, read_container_bytes};
use pm_model::hash::canonical_hash;
use pm_model::json::{JsString, Map, Value, parse, to_canonical_string};
use pm_model::schema::{
    BlendMode, BlenderMaterial, ChannelValue, Codec, Composition, LayerType, Project, Prop,
    SavedDocument, WebScene, mismatches,
};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo() -> PathBuf {
    manifest().join("../../..")
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    parse(&text).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()))
}

fn sorted_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == extension))
        .collect();
    files.sort();
    files
}

/// `T::decode(v)?.encode()` is identical to `v`.
fn assert_lossless<T: Codec>(name: &str, value: &Value) -> T {
    let typed = T::decode(value.clone()).unwrap_or_else(|_| panic!("{name}: did not decode"));
    let back = typed.encode();
    assert!(
        back.identical(value),
        "{name}: round trip differs\n{}\n{}",
        to_canonical_string(value),
        to_canonical_string(&back)
    );
    typed
}

/// Every project fixture in the repository, by name, as stored.
fn project_fixtures() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for path in sorted_files(&manifest().join("tests/fixtures/corpus"), "json") {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        out.push((
            format!("corpus/{name}"),
            project_of(&read_json(&path)).clone(),
        ));
    }
    for rel in [
        "apps/desktop/src/renderer/public/onboarding/scene.json",
        "apps/www/static/hero/scene.json",
    ] {
        let scene = read_json(&repo().join(rel));
        out.push((rel.to_owned(), scene.get("project").unwrap().clone()));
    }
    for path in sorted_files(&manifest().join("tests/fixtures/containers"), "pmv") {
        let bytes = std::fs::read(&path).unwrap();
        if let Ok(container) = read_container_bytes(&bytes, ReadOptions::default()) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let project = project_of(&container.document).clone();
            if project.as_object().is_some() {
                out.push((format!("containers/{name}"), project));
            }
        }
    }
    let vectors = parse(include_str!("vectors/canonical-vectors.json")).unwrap();
    for entry in vectors.get("projects").unwrap().as_array().unwrap() {
        if let Some(Value::String(source)) = entry.get("project") {
            let name = entry.get("name").unwrap().as_str().unwrap().to_owned();
            out.push((
                format!("vectors/{name}"),
                parse(&source.to_string_lossy()).unwrap(),
            ));
        }
    }
    out
}

#[test]
fn fixture_inventory_is_complete() {
    let fixtures = project_fixtures();
    let count = |prefix: &str| {
        fixtures
            .iter()
            .filter(|(n, _)| n.starts_with(prefix))
            .count()
    };
    assert_eq!(count("corpus/"), 35);
    assert_eq!(count("apps/"), 2);
    assert!(count("containers/") >= 8, "{}", count("containers/"));
    assert!(count("vectors/") >= 2);
}

#[test]
fn every_project_fixture_round_trips_through_the_typed_schema() {
    for (name, project) in project_fixtures() {
        assert_lossless::<Project>(&name, &project);
        // The canonical form's compositions decode uniformly too.
        let canonical = canonical_project(project.clone());
        let typed = assert_lossless::<Project>(&format!("{name} (canonical)"), &canonical);
        if let Some(comps) = &typed.comps {
            for (id, comp) in comps {
                let encoded = comp.encode();
                assert_lossless::<Composition>(&format!("{name} comps[{id:?}]"), &encoded);
            }
        }
    }
}

#[test]
fn whole_files_round_trip() {
    for path in sorted_files(&manifest().join("tests/fixtures/containers"), "pmv") {
        let bytes = std::fs::read(&path).unwrap();
        if let Ok(container) = read_container_bytes(&bytes, ReadOptions::default())
            && container.document.get("proj").is_some()
        {
            assert_lossless::<SavedDocument>(&path.display().to_string(), &container.document);
        }
    }
    for rel in [
        "apps/desktop/src/renderer/public/onboarding/scene.json",
        "apps/www/static/hero/scene.json",
    ] {
        let scene = read_json(&repo().join(rel));
        let typed = assert_lossless::<WebScene>(rel, &scene);
        assert_eq!(typed.format.unwrap().as_str(), Some("powermove-web"));
    }
}

/// Paths below the corpus whose shape differs from the TypeScript. A new
/// entry here is a schema gap to fix, not an expected result.
#[test]
fn corpus_fits_the_typed_schema() {
    let mut report = Vec::new();
    for (name, project) in project_fixtures() {
        let typed = Project::decode(project).unwrap();
        for path in mismatches(&typed) {
            report.push(format!("{name}: {path}"));
        }
    }
    // The two synthetic canonical vectors are malformed on purpose (a
    // one-element work range, a non-object composition entry).
    assert_eq!(
        report,
        [
            "vectors/synthetic: comps.C3.work",
            "vectors/legacy-unnormalized: comps",
        ],
        "shape mismatches:\n{}",
        report.join("\n")
    );
}

#[test]
fn typed_views_read_real_content() {
    let project = read_json(&manifest().join("tests/fixtures/corpus/compositing-blend-modes.json"));
    let project = Project::decode(project_of(&project).clone()).unwrap();
    let layers = project.layers.as_ref().unwrap();
    assert!(!layers.is_empty());
    let modes: Vec<&BlendMode> = layers
        .iter()
        .filter_map(|l| l.blend.as_ref()?.as_static())
        .collect();
    assert!(modes.contains(&&BlendMode::Multiply), "{modes:?}");
    assert!(layers.iter().all(|l| l.layer_type.is_some()));

    let captions =
        read_json(&manifest().join("tests/fixtures/corpus/captions-box-highlight-pop.json"));
    let captions = Project::decode(project_of(&captions).clone()).unwrap();
    let layer = captions
        .layers
        .as_ref()
        .unwrap()
        .iter()
        .find(|l| l.layer_type == Some(LayerType::Captions))
        .expect("a captions layer");
    let content = layer.d.as_ref().unwrap();
    assert!(!content.cues.as_ref().unwrap().is_empty());
    assert!(content.style.as_ref().unwrap().highlight.is_some());

    let gltf = read_json(&manifest().join("tests/fixtures/corpus/3d-gltf.json"));
    let gltf = Project::decode(project_of(&gltf).clone()).unwrap();
    let has_object = gltf
        .layers
        .iter()
        .chain(
            gltf.comps
                .iter()
                .flat_map(|c| c.values().filter_map(|c| c.layers.as_ref())),
        )
        .flatten()
        .any(|l| {
            l.d.as_ref()
                .and_then(|d| d.data.as_ref())
                .and_then(|data| data.object.as_ref())
                .and_then(|o| o.source.as_ref())
                .is_some_and(|s| s.asset_id.is_some())
        });
    assert!(has_object, "a glTF object layer");
}

const EVERYTHING: &str = r##"{
  "id": "P", "name": "Unknowns", "w": 1920, "h": 1080, "fps": 29.97, "dur": 10,
  "fpsRational": [30000, 1001], "minReaderVersion": 3, "requiresEngine": "native",
  "color": {"linear": true, "future": [1, 2]}, "media": {"proxies": "auto"},
  "audioSampleRate": 48000, "motionBlurSamples": {"opaque": 16, "alpha": 20},
  "futureRoot": {"deep": [{"x": -0}]},
  "work": [0, 10],
  "markers": [{"t": 0.1, "name": "m", "color": "#f00"}],
  "assets": {"A": {"id": "A", "kind": "video", "storage": "linked", "link": {"path": "/x"},
                   "interpret": {"alpha": "premultiplied"}, "somethingNew": 1}},
  "layers": [{
    "id": "L", "type": "video", "name": "v", "from": 0.30000000000000004, "dur": 5e-324,
    "on": true, "lock": false, "shy": false, "collapsed": false, "color": "#fff",
    "blend": "hardmix", "mblur": false, "parent": null, "fx": [], "masks": [],
    "locked_intent": {}, "compat": 2, "futureLayerField": {"a": 1},
    "p": {"opacity": {"v": 100, "kf": [{"t": 1.7976931348623157e308, "v": 1, "i": "linear", "newKeyField": true}], "expr": null, "chanExtra": 1}},
    "d": {"asset": "A", "trim": 0, "speed": 1, "pitch": "preserve", "alwaysOriginal": true,
          "keepFrameRate": true, "keepRendered": {"key": "abc"}, "unknownContent": "\ud800"}
  }, {
    "id": "M", "type": "hologram", "name": "future layer type", "d": {"beam": 3}
  }],
  "comps": {"C2": {"id": "C2", "name": "child", "fps": 23.976, "layers": [], "compExtra": null}}
}"##;

#[test]
fn unknown_members_and_new_values_survive() {
    let value = parse(EVERYTHING).unwrap();
    let project = assert_lossless::<Project>("everything", &value);
    assert_eq!(project.fps_rational(), Some([30000, 1001]));
    assert!(project.extra.contains_key("futureRoot"));
    let layers = project.layers.as_ref().unwrap();
    assert_eq!(
        layers[0].blend.as_ref().unwrap().as_static(),
        Some(&BlendMode::Other(JsString::from("hardmix")))
    );
    assert_eq!(
        layers[1].layer_type,
        Some(LayerType::Other(JsString::from("hologram")))
    );
    let kf = &layers[0].p.as_ref().unwrap()[&JsString::from("opacity")]
        .kf
        .as_ref()
        .unwrap()[0];
    assert_eq!(kf.t, Some(f64::MAX));
    assert!(kf.extra.contains_key("newKeyField"));
    assert_eq!(layers[0].dur.unwrap().to_bits(), 1);
    // Unknown enumerated values are reported, not dropped.
    let report = mismatches(&project);
    assert!(
        report.iter().any(|p| p.starts_with("layers[0].blend")),
        "{report:?}"
    );
    assert!(
        report.iter().any(|p| p.starts_with("layers[1].type")),
        "{report:?}"
    );
}

#[test]
fn shape_mismatches_are_kept_raw_and_reported() {
    let text = r##"{"w": "1920", "layers": [{"id": "L", "from": "soon", "p": {"opacity": 5},
        "d": {"color": {"v": "#fff", "kf": "nope", "expr": null}}}], "work": [0, 1, 2]}"##;
    let value = parse(text).unwrap();
    let project = assert_lossless::<Project>("mismatch", &value);
    assert_eq!(project.w, None);
    assert_eq!(project.extra.get("w").unwrap().as_str(), Some("1920"));
    assert_eq!(project.work, None);
    let mut report = mismatches(&project);
    report.sort();
    assert_eq!(
        report,
        [
            "layers[0].d.color.kf",
            "layers[0].from",
            "layers[0].p",
            "w",
            "work",
        ]
    );
}

#[test]
fn key_order_is_irrelevant() {
    for (name, project) in project_fixtures().into_iter().take(12) {
        let reversed = reverse_keys(&parse(&reverse_text(&project)).unwrap());
        let a = Project::decode(project.clone()).unwrap();
        let b = Project::decode(reversed).unwrap();
        assert_eq!(a, b, "{name}");
        assert_eq!(
            canonical_hash(&a.encode()),
            canonical_hash(&project),
            "{name}"
        );
    }
}

/// The value serialized with every object's keys in reverse order (a
/// different text for the same JSON value).
fn reverse_text(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let members: Vec<String> = map
                .iter()
                .rev()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        to_canonical_string(&Value::String(k.clone())),
                        reverse_text(v)
                    )
                })
                .collect();
            format!("{{{}}}", members.join(","))
        }
        Value::Array(items) => {
            format!(
                "[{}]",
                items.iter().map(reverse_text).collect::<Vec<_>>().join(",")
            )
        }
        other => to_canonical_string(other),
    }
}

fn reverse_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), reverse_keys(v)))
                .collect::<Map>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(reverse_keys).collect()),
        other => other.clone(),
    }
}

#[test]
fn animated_content_fields_decode_as_channels() {
    let text = r##"{"id": "L", "type": "text", "d": {"size": {"v": 48, "kf": [{"t": 0, "v": 24}], "expr": null},
        "text": "hi", "fontAxis": {"wght": 400, "wdth": {"v": 100, "kf": [], "expr": "time"}}}}"##;
    let layer = assert_lossless::<pm_model::schema::Layer>("text", &parse(text).unwrap());
    let d = layer.d.unwrap();
    let size = d.size.unwrap();
    let channel = size.as_channel().unwrap();
    assert_eq!(
        channel.v.as_ref().and_then(ChannelValue::as_f64),
        Some(48.0)
    );
    assert_eq!(d.text.unwrap().as_static().unwrap().as_str(), Some("hi"));
    let axis = d.font_axis.unwrap();
    assert!(matches!(axis[&JsString::from("wght")], Prop::Static(v) if v == 400.0));
    assert!(axis[&JsString::from("wdth")].as_channel().is_some());
}

const MATERIAL_SAMPLE: &str = r##"{
  "id":"product_paint",
  "name":"Product paint",
  "graph":{
    "nodes":[
      {"id":"noise","type":"ShaderNodeTexNoise","inputs":{"Scale":5,"Detail":2}},
      {"id":"bump","type":"ShaderNodeBump","inputs":{"Strength":0.2,"Distance":0.1}},
      {"id":"surface","type":"ShaderNodeBsdfPrincipled","inputs":{"Base Color":"#FF6B30","Roughness":0.3,"Metallic":0.15}},
      {"id":"output","type":"ShaderNodeOutputMaterial"}
    ],
    "links":[
      {"from":"noise","output":"Fac","to":"bump","input":"Height"},
      {"from":"bump","output":"Normal","to":"surface","input":"Normal"},
      {"from":"surface","output":"BSDF","to":"output","input":"Surface"}
    ]
  },
  "inputs":[
    {"id":"scale","label":"Pattern scale","kind":"number","node":"noise","socket":"Scale","min":0.01,"max":100},
    {"id":"color","label":"Color","kind":"color","node":"surface","socket":"Base Color"}
  ],
  "p":{"scale":{"v":5,"kf":[],"expr":null},"color":{"v":"#FF6B30","kf":[],"expr":null}}
}
"##;

#[test]
fn scene3d_doc_material_sample_decodes() {
    // The Blender material sample from apps/desktop/docs/SCENE3D.md before
    // main retired Blender (2b1fa379a) and rewrote that page without it; the
    // schema stays while the engine keeps its Blender crates.
    let value = parse(MATERIAL_SAMPLE).unwrap();
    let material = assert_lossless::<BlenderMaterial>("SCENE3D.md material", &value);
    assert!(
        mismatches(&material).is_empty(),
        "{:?}",
        mismatches(&material)
    );
    let graph = material.graph.unwrap();
    assert!(!graph.nodes.unwrap().is_empty());
    assert!(!graph.links.unwrap().is_empty());
}
