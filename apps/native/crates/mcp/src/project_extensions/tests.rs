use super::*;

#[test]
fn references_include_nested_render_definitions_only() {
    let project = pm_model::Document::new(Some(model_value(json!({
        "compId":"root",
        "layers":[{"id":"parent","type":"precomp","d":{"comp":"child"}}],
        "comps":{"child":{"layers":[
            {"id":"nested","type":"extension","d":{"definition":"toy.layer","panel":"ignore.panel"},
             "fx":[{"type":"blur"},{"type":"toy.tint"}],"transitionOut":{"type":"toy.fade"}}
        ]}}
    })).unwrap()), 1);
    let refs = references(project.root().unwrap());
    assert_eq!(refs.len(), 4);
    let owners = BTreeMap::new();
    let builtins = BTreeSet::from([("effect".into(), "blur".into())]);
    let bundle = Bundle::collect(
        refs,
        &BTreeMap::new(),
        &owners,
        &builtins,
        &BTreeSet::new(),
        true,
    );
    assert_eq!(bundle.entries.len(), 1);
    assert_eq!(bundle.entries[0].id, "toy");
    assert_eq!(bundle.entries[0].items.len(), 3);
    assert_eq!(bundle.entries[0].status, Status::Unavailable);
}

#[test]
fn package_verification_covers_every_file() {
    let entry = Entry {
        id: "toy".into(),
        name: "Toy".into(),
        version: "1.0.0".into(),
        author: "user".into(),
        source: "local".into(),
        store_id: None,
        hash: None,
        bytes: 0,
        items: vec![],
        status: Status::Included,
    };
    let mut files = Files::from([
        (
            "manifest.json".into(),
            br#"{"id":"toy","version":"1.0.0"}"#.to_vec(),
        ),
        ("index.ts".into(), b"export default function () {}".to_vec()),
    ]);
    let hash = package_hash(&files);
    let bytes: usize = files.values().map(Vec::len).sum();
    let mut extra = pm_model::json::Map::new();
    extra.insert(
        MANIFEST,
        model_value(json!({"version":1,"entries":[Entry {
            hash: Some(hash), bytes, ..entry
        }]}))
        .unwrap(),
    );
    let write = |extra: &mut pm_model::json::Map, files: &Files| {
        let encoded: BTreeMap<_, _> = files
            .iter()
            .map(|(path, data)| (path, STANDARD.encode(data)))
            .collect();
        extra.insert(FILES, model_value(json!({"toy":encoded})).unwrap());
    };
    write(&mut extra, &files);
    assert!(Bundle::read(&extra).unwrap().files("toy").is_some());
    files.get_mut("index.ts").unwrap()[0] ^= 1;
    write(&mut extra, &files);
    let bad = Bundle::read(&extra).unwrap();
    assert_eq!(bad.entries[0].status, Status::Invalid);
    assert!(bad.files("toy").is_none());
}
