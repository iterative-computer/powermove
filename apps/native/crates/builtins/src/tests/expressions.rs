//! A Properties row's expression: Add/Edit Expression… and Show in Graph
//! Editor hand off to the app, the row lists the expression, and its field
//! writes every channel of the row.
use super::*;

fn backend(actor: &KernelActor) -> data::Backend {
    select_layer(&actor.handle(), "a");
    let identity = pm_surface::host_api::Identity {
        realm: "inspector".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    data::Backend::new(actor.handle(), identity, "inspector", false).unwrap()
}
fn row(snapshot: &Value, id: &str) -> Value {
    snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap())
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("no row {id}"))
        .clone()
}
fn act(backend: &mut data::Backend, row: &str, action: &str) -> Vec<(String, Value)> {
    backend.native.platform.clear();
    backend
        .native_command(
            "panel.inspector",
            &json!({"type":"row-action","row":row,"action":action}),
        )
        .unwrap();
    std::mem::take(&mut backend.native.platform)
}
fn expression(kernel: &pm_kernel::KernelHandle, channel: &str) -> Option<String> {
    let published = kernel.published("synthetic").unwrap();
    let root = published.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, "a").unwrap();
    let layer = pm_kernel::model::layer_at(root, &at).unwrap().to_value();
    layer
        .get("p")?
        .get(channel)?
        .get("expr")?
        .as_str()
        .map(str::to_owned)
}
fn run(kernel: &pm_kernel::KernelHandle, request: Request) {
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(
        !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
        "{:?}",
        ack.reply
    );
}

#[test]
fn add_expression_asks_the_app_for_its_editor_with_the_rows_channels() {
    let actor = actor();
    let mut backend = backend(&actor);
    let snapshot = backend.inspector_snapshot().unwrap();
    assert!(row(&snapshot, "position")["expression"].is_null());
    let asked = act(&mut backend, "position", "edit-expression");
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].0, "command");
    assert_eq!(asked[0].1["name"], "properties.expression");
    let input = &asked[0].1["args"][0];
    assert_eq!(input["layer"], "Headline");
    assert_eq!(input["property"], "Position");
    assert_eq!(input["source"], Value::Null, "a new expression");
    assert_eq!(
        input["targets"],
        json!([["a", "position.x"], ["a", "position.y"]])
    );
    // Nothing is written until the editor applies.
    assert_eq!(expression(&actor.handle(), "position.x"), None);
    actor.shutdown();
}

#[test]
fn a_row_lists_its_expression_and_the_field_rewrites_and_removes_it() {
    let actor = actor();
    let kernel = actor.handle();
    let mut backend = backend(&actor);
    let set = |channel: &str| pm_kernel::Call::Transact {
        label: Some("Set Expression".into()),
        expected_revision: None,
        body: pm_kernel::WriteBody::Commands(vec![
            pm_kernel::commands::properties::PropertyCommand::SetExpression {
                layer_id: "a".into(),
                channel: channel.into(),
                expression: Some(properties::STARTING_EXPRESSION.into()),
            }
            .into(),
        ]),
    };
    for channel in ["position.x", "position.y"] {
        run(&kernel, Request::new(Caller::native(), set(channel)));
    }
    let snapshot = backend.inspector_snapshot().unwrap();
    let listed = row(&snapshot, "position")["expression"].clone();
    assert_eq!(listed["source"], "value + wiggle(2, 20)");
    assert_eq!(listed["error"], Value::Null);
    assert!(row(&snapshot, "opacity")["expression"].is_null());
    // Edit Expression… opens on what the property has.
    let asked = act(&mut backend, "position", "edit-expression");
    assert_eq!(asked[0].1["args"][0]["source"], "value + wiggle(2, 20)");

    let handle = listed["binding"]["handle"].as_str().unwrap().to_owned();
    let model = backend.model.as_mut().unwrap();
    let write = |properties: &mut properties::Properties,
                 model: &mut pm_panel_inspector::Inspector,
                 text: &str| {
        properties.write(model, Caller::native(), &handle, text.into(), 0.)
    };
    // The kernel refuses a source the parser refuses; nothing changes.
    let refused = write(&mut backend.native.properties, model, "value +").unwrap();
    let outcome = kernel.call("synthetic", refused).wait().unwrap();
    assert!(
        !matches!(&outcome, Ok(ack) if !matches!(ack.reply, pm_kernel::Reply::Throw(_))),
        "{outcome:?}"
    );
    assert_eq!(
        expression(&kernel, "position.x").as_deref(),
        Some("value + wiggle(2, 20)")
    );
    run(
        &kernel,
        write(&mut backend.native.properties, model, "t * 10").unwrap(),
    );
    for channel in ["position.x", "position.y"] {
        assert_eq!(expression(&kernel, channel).as_deref(), Some("t * 10"));
    }
    let snapshot = backend.inspector_snapshot().unwrap();
    assert_eq!(row(&snapshot, "position")["expression"]["source"], "t * 10");
    let model = backend.model.as_mut().unwrap();
    run(
        &kernel,
        write(&mut backend.native.properties, model, "").unwrap(),
    );
    assert_eq!(expression(&kernel, "position.x"), None);
    assert_eq!(expression(&kernel, "position.y"), None);
    let snapshot = backend.inspector_snapshot().unwrap();
    assert!(row(&snapshot, "position")["expression"].is_null());
    actor.shutdown();
}

#[test]
fn show_in_graph_editor_asks_the_timeline_for_the_rows_property() {
    let actor = actor();
    let mut backend = backend(&actor);
    backend.inspector_snapshot().unwrap();
    assert_eq!(
        act(&mut backend, "opacity", "show-graph"),
        [(
            "command".to_owned(),
            json!({"name":"timeline.showGraph","args":["a","opacity",["opacity"]]})
        )]
    );
    assert_eq!(
        act(&mut backend, "position", "show-graph"),
        [(
            "command".to_owned(),
            json!({"name":"timeline.showGraph","args":["a","position.x",["position.x"]]})
        )]
    );
    actor.shutdown();
}
