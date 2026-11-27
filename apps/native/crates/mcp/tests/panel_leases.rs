use pm_mcp::{Automation, McpSession, SurfaceHook};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
struct Capture(Mutex<Vec<Value>>);
impl SurfaceHook for Capture {
    fn call(&self, _: &str, _: Option<&str>, args: &Value) -> pm_mcp::Result<Value> {
        self.0.lock().unwrap().push(args.clone());
        Ok(json!({}))
    }
}
#[test]
fn p2_external_connections_have_distinct_trusted_session_leases() {
    let root = std::env::temp_dir().join(pm_kernel::new_id("panel-lease-test"));
    let mut host = Automation::new(root.clone(), root.join("exports")).unwrap();
    let capture = Arc::new(Capture(Mutex::new(vec![])));
    host.surfaces = Some(capture.clone());
    let host = Arc::new(Mutex::new(host));
    let mut a = McpSession::new(host.clone());
    let mut b = McpSession::new(host);
    for s in [&mut a, &mut b] {
        s.dispatch(
            "start_session",
            &json!({"prompt":"Synthetic panels","access":"project","context":"app"}),
        )
        .unwrap();
        s.dispatch(
            "panel_begin",
            &json!({"title":"Synthetic", "placement":{"tab_in":"inspector"}}),
        )
        .unwrap();
        s.dispatch(
            "panel_build",
            &json!({"build_id":"synthetic", "event":"cancel"}),
        )
        .unwrap();
    }
    let calls = capture.0.lock().unwrap();
    let owner = calls[0]["_panel_session"]
        .as_str()
        .expect("every external session needs a lease");
    assert_eq!(calls[1]["_panel_session"], owner);
    assert_ne!(calls[2]["_panel_session"], owner);
    let a_id = owner.to_owned();
    let b_id = calls[2]["_panel_session"].as_str().unwrap().to_owned();
    assert!(pm_mcp::panel_lease::live(&a_id));
    assert!(pm_mcp::panel_lease::live(&b_id));
    drop(calls);
    a.finish(true).unwrap();
    assert!(!pm_mcp::panel_lease::live(&a_id));
    assert!(pm_mcp::panel_lease::live(&b_id));
    a.dispatch(
        "start_session",
        &json!({"prompt":"New session","access":"project","context":"app"}),
    )
    .unwrap();
    a.dispatch("panel_begin", &json!({"title":"New", "placement":{}}))
        .unwrap();
    let new_id = capture.0.lock().unwrap().last().unwrap()["_panel_session"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(a_id, new_id);
    assert!(pm_mcp::panel_lease::live(&new_id));
    a.close().unwrap();
    assert!(!pm_mcp::panel_lease::live(&new_id));
    drop(b);
    assert!(!pm_mcp::panel_lease::live(&b_id));
    let _ = std::fs::remove_dir_all(root);
}
