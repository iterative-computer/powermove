//! Every action the Color panel offers a person has an agent route with the
//! same result (`js/v4/panels/color/contract.ts` ACTIONS, whose routes the
//! panel's own test checks against this catalog): each is run here on a real
//! project through the commands an agent sends.
use pm_mcp::Automation;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct Fixture {
    root: PathBuf,
    host: Arc<Mutex<Automation>>,
    id: String,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("pm-mcp-grade"));
        let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
        let id = host
            .create_project(&json!({"name":"Synthetic grade"}))
            .unwrap()["projectId"]
            .as_str()
            .unwrap()
            .into();
        let f = Self {
            root,
            host: Arc::new(Mutex::new(host)),
            id,
        };
        f.agent(vec![
            json!({"type":"add_layer","id":"clip","layerType":"solid",
            "name":"Synthetic solid"}),
        ]);
        f
    }
    /// Plans `commands` as `apply_commands` does, with the app's seeded
    /// registries, and applies them; returns each command's result data.
    fn agent(&self, commands: Vec<Value>) -> Vec<Value> {
        let state = pm_kernel::state::StateSnapshot::seeded();
        let mut h = self.host.lock().unwrap();
        let project = h.read_document(&self.id).unwrap();
        let batch =
            pm_mcp::commands::plan_with_definitions(&project, &[], &commands, 0., Some(&state))
                .unwrap();
        let revision = h.revision(&self.id).unwrap();
        h.apply_commands(
            &self.id,
            batch.commands,
            "Grade",
            pm_kernel::Origin::Agent,
            revision,
            907,
        )
        .unwrap();
        batch.results.iter().map(|r| r["data"].clone()).collect()
    }
    fn fx(&self) -> Vec<Value> {
        let doc = self.host.lock().unwrap().read_document(&self.id).unwrap();
        doc["layers"][0]["fx"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn every_color_panel_action_has_an_agent_route() {
    let f = Fixture::new();
    // Add Color Wheels.
    let wheels = f.agent(vec![
        json!({"type":"add_effect","target":"clip","effect":"colorWheels"}),
    ])[0]["effectId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(f.fx()[0]["type"], "colorWheels");
    // Set a value; key one at the playhead.
    f.agent(vec![
        json!({"type":"set_property","target":"clip","path":format!("{wheels}.gainMaster"),
            "value":1.25,"mode":"static"}),
        json!({"type":"replace_keyframes","target":"clip","path":format!("{wheels}.liftMaster"),
            "keyframes":[{"time":0,"value":0},{"time":1,"value":0.1}]}),
    ]);
    let p = &f.fx()[0]["p"];
    assert_eq!(p["gainMaster"]["v"], 1.25);
    assert_eq!(p["liftMaster"]["kf"].as_array().unwrap().len(), 2);
    // Reset one control to its default.
    f.agent(vec![json!({"type":"set_property","target":"clip",
        "path":format!("{wheels}.gainMaster"),"value":1,"mode":"static"})]);
    assert_eq!(f.fx()[0]["p"]["gainMaster"]["v"], 1);
    // Turn the tool off.
    f.agent(vec![
        json!({"type":"set_effect","target":"clip","effect":wheels,
        "patch":{"enabled":false}}),
    ]);
    assert_eq!(f.fx()[0]["on"], false);
    // Add a Red curve with its points.
    let curve = f.agent(vec![
        json!({"type":"add_effect","target":"clip","effect":"colorCurves",
        "parameters":{"channel":1,"count":3,"x1":0.5,"y1":0.6,"x2":1,"y2":1}}),
    ])[0]["effectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let added = f.fx();
    assert_eq!(added[1]["type"], "colorCurves");
    assert_eq!(added[1]["p"]["channel"]["v"], 1);
    assert_eq!(added[1]["p"]["y1"]["v"], 0.6);
    // Load a LUT: an imported LUT asset (`import_media`, tested in the app)
    // on Apply Color LUT, applied on ACEScct.
    f.agent(vec![json!({"type":"add_effect","target":"clip","effect":"applyColorLut",
        "inputs":{"lut":{"kind":"asset","assetId":"lut-a"}},"parameters":{"inputSpace":10,"outputSpace":10}})]);
    let lut = &f.fx()[2];
    assert_eq!(lut["type"], "applyColorLut");
    assert_eq!(lut["version"], 2);
    assert_eq!(lut["inputs"]["lut"]["assetId"], "lut-a");
    assert_eq!(lut["p"]["inputSpace"]["v"], 10);
    // Copy and paste a grade: read it back, add it with its values.
    let read = f.fx()[0].clone();
    let values: serde_json::Map<String, Value> = read["p"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(_, v)| v["kf"].as_array().is_none_or(|k| k.is_empty()))
        .map(|(k, v)| (k.clone(), v["v"].clone()))
        .collect();
    f.agent(vec![
        json!({"type":"add_effect","target":"clip","effect":"colorWheels",
        "parameters":values}),
    ]);
    assert_eq!(
        f.fx()[3]["p"]["gainMaster"]["v"],
        read["p"]["gainMaster"]["v"]
    );
    // Remove a tool.
    f.agent(vec![
        json!({"type":"remove_effect","target":"clip","effect":curve}),
    ]);
    assert!(f.fx().iter().all(|fx| fx["type"] != "colorCurves"));
}

/// Relinking a LUT, as Load LUT… does for a person: the existing Apply Color
/// LUT takes the new asset in place, keeping its identity, encodings and its
/// animated Amount, in one Undo step.
#[test]
fn an_agent_relinks_the_lut_of_an_existing_effect() {
    let f = Fixture::new();
    let lut = f.agent(vec![
        json!({"type":"add_effect","target":"clip","effect":"applyColorLut",
        "inputs":{"lut":{"kind":"asset","assetId":"gone"}},
        "parameters":{"inputSpace":10,"outputSpace":4}}),
    ])[0]["effectId"]
        .as_str()
        .unwrap()
        .to_owned();
    f.agent(vec![
        json!({"type":"replace_keyframes","target":"clip","path":format!("{lut}.amount"),
            "keyframes":[{"time":0,"value":20},{"time":1,"value":80}]}),
    ]);
    let before = f.fx()[0].clone();
    let result = f.agent(vec![
        json!({"type":"set_effect","target":"clip","effect":lut,
        "patch":{"inputs":{"lut":{"kind":"asset","assetId":"replacement"}}}}),
    ]);
    assert_eq!(result[0]["effectId"], lut.as_str());
    let after = f.fx();
    assert_eq!(after.len(), 1, "relinked in place, not added");
    let relinked = &after[0];
    assert_eq!(relinked["id"], lut.as_str());
    assert_eq!(relinked["inputs"]["lut"]["assetId"], "replacement");
    assert_eq!(relinked["p"], before["p"], "values and keys kept");
    assert_eq!(relinked["p"]["amount"]["kf"].as_array().unwrap().len(), 2);
    assert_eq!(relinked["p"]["inputSpace"]["v"], 10);
    // A reference that is not a reference is refused by the kernel's check.
    let project = f.host.lock().unwrap().read_document(&f.id).unwrap();
    let state = pm_kernel::state::StateSnapshot::seeded();
    let bad = pm_mcp::commands::plan_with_definitions(
        &project,
        &[],
        &[json!({"type":"set_effect","target":"clip","effect":lut,
            "patch":{"inputs":{"lut":{"kind":"asset","assetId":""}}}})],
        0.,
        Some(&state),
    )
    .and_then(|batch| {
        let mut h = f.host.lock().unwrap();
        let revision = h.revision(&f.id).unwrap();
        h.apply_commands(
            &f.id,
            batch.commands,
            "Grade",
            pm_kernel::Origin::Agent,
            revision,
            907,
        )
    });
    assert!(bad.is_err(), "an empty asset id is refused");
    assert_eq!(f.fx()[0]["inputs"]["lut"]["assetId"], "replacement");
    // One Undo returns the old link.
    f.host
        .lock()
        .unwrap()
        .undo(&f.id, pm_kernel::Origin::Agent, 907)
        .unwrap();
    assert_eq!(f.fx()[0], before);
}

/// What an agent writes to a grading tool stays within what its controls
/// offer: a number within the range, one of the options, a whole count. Each
/// route an agent writes by is refused with a sentence, and nothing changes.
#[test]
fn an_agent_writes_grading_values_the_controls_can_hold() {
    let f = Fixture::new();
    let refused = |commands: Vec<Value>| -> String {
        let project = f.host.lock().unwrap().read_document(&f.id).unwrap();
        let state = pm_kernel::state::StateSnapshot::seeded();
        match pm_mcp::commands::plan_with_definitions(&project, &[], &commands, 0., Some(&state)) {
            Ok(_) => panic!("{commands:?} was accepted"),
            Err(e) => format!("{e:?}"),
        }
    };
    let add = |p: Value, effect: &str| json!({"type":"add_effect","target":"clip","effect":effect,"parameters":p});
    let message = refused(vec![add(json!({"exposure":1024}), "colorWheels")]);
    assert!(
        message.contains("Exposure takes values from -8 to 8, not 1024."),
        "{message}"
    );
    // Representable in f64, an infinity at GPU precision.
    refused(vec![add(json!({"gainMaster":1e39}), "colorWheels")]);
    // A channel's keys too.
    refused(vec![add(
        json!({"gainMaster":{"v":1,"kf":[{"t":0,"v":1},{"t":1,"v":9}],"expr":null}}),
        "colorWheels",
    )]);
    let message = refused(vec![add(json!({"count":2.5}), "colorCurves")]);
    assert!(message.contains("whole number"), "{message}");
    let message = refused(vec![add(json!({"channel":7}), "colorCurves")]);
    assert!(message.contains("one of"), "{message}");
    refused(vec![add(json!({"inputSpace":99}), "applyColorLut")]);

    let wheels = f.agent(vec![add(
        json!({"exposure":2,"gainMaster":1.5}),
        "colorWheels",
    )])[0]["effectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = format!("{wheels}.exposure");
    refused(vec![
        json!({"type":"set_property","target":"clip","path":path,"value":9,"mode":"static"}),
    ]);
    refused(vec![
        json!({"type":"replace_keyframes","target":"clip","path":path,
        "keyframes":[{"time":0,"value":0},{"time":1,"value":-1e39}]}),
    ]);
    f.agent(vec![
        json!({"type":"set_property","target":"clip","path":path,"value":-8,"mode":"static"}),
        json!({"type":"replace_keyframes","target":"clip","path":format!("{wheels}.saturation"),
            "keyframes":[{"time":0,"value":0},{"time":1,"value":200}]}),
    ]);
    let fx = f.fx();
    assert_eq!(fx[0]["p"]["exposure"]["v"], -8);
    assert_eq!(fx.len(), 1, "nothing else was added");
    // Other effects keep main's free values.
    f.agent(vec![add(json!({"amount":5000}), "blur")]);
    assert_eq!(f.fx()[1]["p"]["amount"]["v"], 5000);
}

/// An agent adds a Color Space Transform as the Colour panel's Add does, with
/// its conversion and method, and is refused a method that does not exist.
#[test]
fn an_agent_adds_a_color_space_transform() {
    let f = Fixture::new();
    let id = f.agent(vec![json!({"type":"add_effect","target":"clip","effect":"colorSpaceTransform",
        "parameters":{"inputGamut":6,"inputTransfer":10,"toneMapping":2,"sourcePeak":4000,"gamutMapping":2}})])
        [0]["effectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let fx = &f.fx()[0];
    assert_eq!(fx["type"], "colorSpaceTransform");
    assert_eq!(fx["p"]["inputTransfer"]["v"], 10);
    assert_eq!(fx["p"]["sourcePeak"]["v"], 4000);
    let project = f.host.lock().unwrap().read_document(&f.id).unwrap();
    let state = pm_kernel::state::StateSnapshot::seeded();
    let refused = pm_mcp::commands::plan_with_definitions(
        &project,
        &[],
        &[
            json!({"type":"set_property","target":"clip","path":format!("{id}.toneMapping"),"value":9,"mode":"static"}),
        ],
        0.,
        Some(&state),
    );
    assert!(format!("{:?}", refused.err()).contains("one of"));
}
