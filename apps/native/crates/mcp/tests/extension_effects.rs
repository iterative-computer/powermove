//! A published extension is validated separately from its live project realm.
//! MCP must load that realm before looking up an extension effect.
use pm_mcp::{Automation, CaptureRequest, EngineHook, McpSession, Result};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct Loader {
    kernel: pm_kernel::KernelHandle,
    automation: std::sync::Weak<Mutex<Automation>>,
}
impl EngineHook for Loader {
    fn capture(&self, _: &CaptureRequest) -> Result<Value> {
        panic!("adding an effect must not render")
    }
    fn prepare_definitions(&self, _: &str, _: u64, _: u64) -> Result<()> {
        // Loading may call back into automation; never hold its mutex here.
        let automation = self.automation.upgrade().unwrap();
        assert!(automation.try_lock().is_ok());
        use pm_kernel::state::{Registration, StateOp, StateTransaction};
        let owner = pm_kernel::Writer {
            realm: 9001,
            generation: 1,
        };
        self.kernel
            .state_transaction(StateTransaction {
                expected_revision: None,
                ops: vec![StateOp::Register {
                registry: "effects".into(),
                entry: Registration {
                    token: "probe-effect".into(), owner, id: "cps-probe.fx".into(),
                    definition: pm_model::json::parse(&json!({
                        "id":"cps-probe.fx", "label":"Probe", "group":"Test",
                        "params":[{"k":"amount","label":"Amount","min":0,"max":1,"def":0.5}],
                        "frag":"o = src(v_uv) * u_amount;"
                    }).to_string()).unwrap(),
                },
            }],
            })
            .wait()
            .unwrap()
            .unwrap();
        Ok(())
    }
}
fn call(session: &mut McpSession, name: &str, args: Value) -> Value {
    let result = session.call(name, args);
    assert_eq!(result["isError"], false, "{name}: {result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}
#[test]
fn add_effect_loads_the_live_registry_before_planning() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("effect-realm"));
    std::fs::create_dir_all(&root).unwrap();
    let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
    actor
        .handle()
        .shell(pm_kernel::session::ShellCall::AttachLibrary {
            profile: root.clone(),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    let mut automation = Automation::attached(
        pm_mcp::app_host::Host::new(actor.handle(), root.clone()),
        root.join("exports"),
    );
    let id = automation
        .create_project(&json!({"name":"Extension effect"}))
        .unwrap()["projectId"]
        .as_str()
        .unwrap()
        .to_owned();
    let host = Arc::new(Mutex::new(automation));
    host.lock().unwrap().engine = Some(Arc::new(Loader {
        kernel: actor.handle(),
        automation: Arc::downgrade(&host),
    }));
    assert!(
        actor
            .handle()
            .state()
            .provider("effects", "cps-probe.fx")
            .is_none()
    );
    let mut session = McpSession::new(host.clone());
    call(
        &mut session,
        "start_session",
        json!({"prompt":"Synthetic probe"}),
    );
    call(
        &mut session,
        "apply_commands",
        json!({"commands":[
            {"type":"add_layer","id":"plate","layerType":"solid"}
        ]}),
    );
    call(
        &mut session,
        "apply_commands",
        json!({"commands":[
            {"type":"add_effect","target":"plate","effect":"cps-probe.fx","parameters":{"amount":0.75}}
        ]}),
    );
    let document = host.lock().unwrap().read_document(&id).unwrap();
    let effect = &document["layers"][0]["fx"][0];
    assert_eq!(effect["type"], "cps-probe.fx");
    assert_eq!(effect["p"]["amount"]["v"], 0.75);
    call(&mut session, "finish_session", json!({"commit":true}));
    drop(session);
    drop(host);
    actor.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}
