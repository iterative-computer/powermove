use pm_kernel::{
    ContentCommand, DocumentStore, LayerCommand, Origin,
    commands::{layers::AddLayer, shader},
};
use pm_model::{Map, Value, json::parse};
fn add() -> AddLayer {
    AddLayer {
        id: "S".into(),
        layer_type: "shader".into(),
        comp: None,
        name: None,
        from: None,
        duration: None,
        content: Map::new(),
        properties: Map::new(),
        color: None,
        index: None,
        parent: None,
        blend: None,
        motion_blur: None,
        visible: None,
        shy: None,
        collapsed: None,
        time: 0.,
    }
}
fn layer(store: &DocumentStore) -> Value {
    store
        .root()
        .unwrap()
        .to_value()
        .get("layers")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .clone()
}

#[test]
fn shader_factory_uses_pinned_template_and_content_reconciliation_is_one_undo() {
    let mut store = DocumentStore::new_untitled(
        parse(r#"{"id":"P","w":640,"h":360,"fps":30,"dur":5,"layers":[],"comps":{}}"#).unwrap(),
    );
    let mut request = add();
    request.content=parse(r#"{"code":"uniform float uKeep; // @param 3\nuniform bool uRemoved; // @param true","uniforms":{"uKeep":{"v":8,"kf":[{"i":"key","t":1,"v":10}],"expr":"time","opaque":"\ud800"},"uRemoved":{"v":false}}}"#).unwrap().as_object().unwrap().clone();
    store
        .execute(
            LayerCommand::Add(Box::new(request)).into(),
            Origin::Interface,
        )
        .unwrap();
    let before = layer(&store);
    let keep = before
        .get("d")
        .unwrap()
        .get("uniforms")
        .unwrap()
        .get("uKeep")
        .cloned();
    let patch=parse(r##"{"code":"uniform float iTime; uniform float uKeep; // @param -2\nuniform vec3 uColor; // @param #abcdef\nuniform bool uOn; // @param true"}"##).unwrap().as_object().unwrap().clone();
    store
        .execute(
            ContentCommand::Set {
                layer_id: "S".into(),
                patch,
                time: 0.,
            }
            .into(),
            Origin::Interface,
        )
        .unwrap();
    let after = layer(&store);
    let uniforms = after.get("d").unwrap().get("uniforms").unwrap();
    assert_eq!(uniforms.get("uKeep"), keep.as_ref());
    assert!(uniforms.get("uRemoved").is_none());
    assert!(uniforms.get("iTime").is_none());
    assert_eq!(
        uniforms
            .get("uColor")
            .unwrap()
            .get("v")
            .and_then(Value::as_str),
        Some("#abcdef")
    );
    assert_eq!(
        uniforms.get("uOn").unwrap().get("v"),
        Some(&Value::Bool(true))
    );
    store.undo().unwrap();
    assert_eq!(layer(&store), before);
    store.redo().unwrap();
    assert_eq!(layer(&store), after);
    let mut template = DocumentStore::new_untitled(
        parse(r#"{"id":"P","w":640,"h":360,"fps":30,"dur":5,"layers":[],"comps":{}}"#).unwrap(),
    );
    template
        .execute(LayerCommand::Add(Box::new(add())).into(), Origin::Interface)
        .unwrap();
    let l = layer(&template);
    assert_eq!(
        l.get("d").unwrap().get("code").and_then(Value::as_str),
        Some(shader::TEMPLATE)
    );
    assert_eq!(
        l.get("d")
            .unwrap()
            .get("uniforms")
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        l.get("p").unwrap().get("position.x").unwrap().get("v"),
        Some(&Value::Number(0.))
    );
}
#[test]
fn inspector_annotations_keep_main_number_semantics_utf16_and_regex_quirks() {
    let code=parse(r##""// xuniform float uA; // @param 0x10 1 33\nuniform float uB; // @param 2bad\nuniform vec3 uColor; // @param #ab\ud800\nuniform bool uFlag;\n// @param true\nuniform float iFrame;""##).unwrap();
    let defs = shader::parameters(code.as_js_string().unwrap());
    assert_eq!(defs.len(), 4);
    assert_eq!(defs[0].get("name").and_then(Value::as_str), Some("uA"));
    assert_eq!(defs[0].get("def"), Some(&Value::Number(16.)));
    assert_eq!(defs[1].get("def"), Some(&Value::Number(0.)));
    assert_eq!(defs[1].get("max"), Some(&Value::Number(1.)));
    assert_eq!(defs[2].get("def"), parse(r##""#ab\ud800""##).ok().as_ref());
    assert_eq!(defs[3].get("def"), Some(&Value::Bool(true)));
}
