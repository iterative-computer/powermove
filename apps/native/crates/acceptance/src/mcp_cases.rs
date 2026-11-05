//! Complete portable MCP contracts, separately from shell and pixel e2e gates.
use crate::fixture::{Fixture, layer, tool_error};
use serde_json::json;
type Check = Result<(), String>;
pub fn union() -> Check {
    let f = Fixture::new()?;
    let actual = f.session.tools();
    assert_eq!(actual, json!(pm_mcp::catalog::tools()));
    let names = actual
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names.len(), actual.as_array().unwrap().len());
    for subset in ["external", "inspection", "app"] {
        // The frozen external union is represented by tools itself; allowlist
        // keys present in the production catalog must reference exposed tools.
        if let Some(list) = pm_mcp::catalog::catalog()[subset].as_array() {
            for name in list {
                assert!(names.contains(name.as_str().unwrap()));
            }
        }
    }
    for name in pm_mcp::catalog::IMPLEMENTED {
        assert!(names.contains(name));
    }
    Ok(())
}
pub fn timeline() -> Check {
    let mut f = Fixture::new()?;
    f.edit(json!([
      {"type":"add_layer","id":"parent","layerType":"solid","duration":12},
      {"type":"add_layer","id":"clip","layerType":"video","from":2,"duration":3,"parent":"parent","content":{"asset":"synthetic-media","trim":1,"speed":2,"embeddedAudio":true}},
      {"type":"set_layer","target":"clip","patch":{"solo":true,"shy":true,"locked":true}}
    ]))?;
    f.call("select_layers", json!({"layerIds":["clip"]}))?;
    let before = f.document()?;
    let all = f.call("get_timeline", json!({}))?;
    let d = before["layers"].as_array().unwrap();
    let strips = all["strips"].as_array().unwrap();
    assert_eq!(
        strips.iter().map(|l| &l["id"]).collect::<Vec<_>>(),
        d.iter().map(|l| &l["id"]).collect::<Vec<_>>()
    );
    assert_eq!(all["selection"], json!(["clip"]));
    assert_eq!(all["fps"], 30);
    let clip = strips.iter().find(|l| l["id"] == "clip").unwrap();
    for (key, value) in [
        ("from", json!(2)),
        ("end", json!(5)),
        ("duration", json!(3)),
        ("startFrame", json!(60)),
        ("endFrame", json!(150)),
        ("sourceIn", json!(1)),
        ("sourceOut", json!(7)),
        ("speed", json!(2)),
        ("parent", json!("parent")),
        ("locked", json!(true)),
        ("solo", json!(true)),
        ("shy", json!(true)),
        ("visible", json!(true)),
        ("assetId", json!("synthetic-media")),
        ("embeddedAudio", json!(true)),
        ("animatedTiming", json!(false)),
    ] {
        assert_eq!(clip[key], value, "{key}");
    }
    let first = f.call("get_timeline", json!({"limit":1}))?;
    assert_eq!(first["nextOffset"], 1);
    let second = f.call("get_timeline", json!({"limit":1,"offset":1}))?;
    assert!(second["nextOffset"].is_null());
    assert_eq!(first["strips"][0], strips[0]);
    assert_eq!(second["strips"][0], strips[1]);
    assert_eq!(f.document()?, before);
    Ok(())
}
pub fn layout() -> Check {
    let workspace = json!({"id":"synthetic-layout","name":"Synthetic saved layout","layout":{"docks":[{"id":"left","size":240,"panels":[{"id":"missing-provider","collapsed":true},{"id":"authored-title","title":"Authored title","flex":true,"size":180}]}]}});
    let mut f = Fixture::configured(|profile| {
        let writer = pm_kernel::persistence::LibraryWriter::open(
            profile,
            pm_kernel::persistence::DiskFormat::V1_2,
            None,
        )
        .map_err(|e| format!("{e:?}"))?;
        for (key, value) in [
            ("workspace", json!("synthetic-layout")),
            ("workspaces", json!([workspace])),
        ] {
            writer
                .set(key, pm_model::json::parse(&value.to_string()).unwrap())
                .map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())
    })?;
    let before = f.document()?;
    let revision = f.revision();
    let expected = json!({"workspace":{"id":"synthetic-layout","name":"Synthetic saved layout"},"registeredPanels":[],"docks":[{"id":"left","size":240,"panels":[{"id":"missing-provider","title":"missing-provider","collapsed":true,"flex":false,"size":null},{"id":"authored-title","title":"Authored title","collapsed":false,"flex":true,"size":180}]}]});
    assert_eq!(f.call("get_panel_layout", json!({}))?, expected);
    assert_eq!(f.revision(), revision);
    assert_eq!(f.document()?, before);
    // Pinned main's cleaner discards undeclared fields before this read.
    assert_eq!(
        f.call("get_panel_layout", json!({"unexpected":true}))?,
        expected
    );
    assert_eq!(f.revision(), revision);
    assert_eq!(f.document()?, before);
    f.call("finish_session", json!({"commit":true}))?;
    f.call(
        "start_session",
        json!({"context":"app","prompt":"Synthetic layout inspection"}),
    )?;
    assert_eq!(f.call("get_panel_layout", json!({}))?, expected);
    Ok(())
}
pub fn manage() -> Check {
    let mut f = Fixture::new()?;
    f.add("kept")?;
    f.call(
        "manage_project",
        json!({"operation":"rename","name":"Renamed"}),
    )?;
    f.call("manage_project", json!({"operation":"save"}))?;
    f.call("finish_session", json!({"commit":true}))?;
    f.call(
        "start_session",
        json!({"context":"app","prompt":"Synthetic library"}),
    )?;
    let created = f.call(
        "manage_project",
        json!({"operation":"create","name":"New independent project"}),
    )?;
    let new_id = created["projectId"].clone();
    let copy = f.call(
        "manage_project",
        json!({"operation":"duplicate","projectId":f.id,"name":"Retained copy"}),
    )?;
    let copy_id = copy["projectId"].clone();
    assert_ne!(copy_id, json!(f.id));
    assert_ne!(new_id, copy_id);
    f.call(
        "manage_project",
        json!({"operation":"trash","projectId":copy_id}),
    )?;
    let listed = f.call("manage_project", json!({"operation":"list"}))?;
    assert!(
        listed["trash"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == copy_id)
    );
    f.call(
        "manage_project",
        json!({"operation":"restore","projectId":copy_id}),
    )?;
    f.call(
        "manage_project",
        json!({"operation":"open","projectId":copy_id}),
    )?;
    let doc = f
        .host
        .lock()
        .unwrap()
        .read_document(copy_id.as_str().unwrap())
        .map_err(|e| e.to_string())?;
    assert_eq!(layer(&doc, "kept")["id"], "kept");
    f.call(
        "manage_project",
        json!({"operation":"open","projectId":f.id}),
    )?;
    assert_eq!(layer(&f.document()?, "kept")["id"], "kept");
    Ok(())
}

/// Frozen kernel/glsl.ts admission, through the actual MCP body. No compilation
/// or rendered pixels are implied by the tool's own definition-only result.
pub fn validate_effect() -> Check {
    let mut f = Fixture::new()?;
    let before = f.document()?;
    let revision = f.revision();
    let definition = json!({"id":"Synthetic.effect","label":"Synthetic","group":"Synthetic","params":[{"k":"amount","def":1}],"frag":"outColor=vec4(1.);"});
    let expected = json!({"valid":true,"id":"Synthetic.effect","parameterCount":1,"note":"Definition validation only. This does not register the effect or compile/render its shader."});
    assert_eq!(
        f.call("validate_effect", json!({"definition":definition}))?,
        expected
    );
    for (key, value, diagnostic) in [
        ("id", json!(""), "effect: invalid id \"\""),
        (
            "label",
            json!(""),
            "effect \"Synthetic.effect\": \"label\" is required",
        ),
        (
            "group",
            json!(null),
            "effect \"Synthetic.effect\": \"group\" is required",
        ),
        (
            "backdrop",
            json!(1),
            "effect \"Synthetic.effect\": \"backdrop\" must be a boolean",
        ),
        (
            "passes",
            json!(9),
            "effect \"Synthetic.effect\": \"passes\" must be an integer 1..8",
        ),
        (
            "params",
            json!([{"k":"a"},{"k":"a"}]),
            "effect \"Synthetic.effect\": duplicate param key \"a\"",
        ),
        (
            "frag",
            json!(""),
            "effect \"Synthetic.effect\": \"frag\" must be a non-empty string",
        ),
    ] {
        let mut invalid = definition.clone();
        invalid[key] = value;
        let error = tool_error(
            &mut f.session,
            "validate_effect",
            json!({"definition":invalid}),
        )?;
        assert_eq!(error["code"], "invalid_arguments");
        assert_eq!(error["message"], diagnostic);
    }
    let mut invalid = definition.clone();
    invalid["label"] = json!("");
    invalid["frag"] = json!("");
    assert_eq!(
        tool_error(
            &mut f.session,
            "validate_effect",
            json!({"definition":invalid})
        )?["message"],
        "effect \"Synthetic.effect\": \"label\" is required"
    );
    for (count, valid) in [(32768, true), (32769, false)] {
        let mut boundary = definition.clone();
        boundary["frag"] = json!("😀".repeat(count));
        if valid {
            assert_eq!(
                f.call("validate_effect", json!({"definition":boundary}))?,
                expected
            );
        } else {
            assert_eq!(
                tool_error(
                    &mut f.session,
                    "validate_effect",
                    json!({"definition":boundary})
                )?["message"],
                "effect \"Synthetic.effect\": shader is too large (65538 > 65536 bytes)"
            );
        }
    }
    assert_eq!(f.document()?, before);
    assert_eq!(f.revision(), revision);
    Ok(())
}
