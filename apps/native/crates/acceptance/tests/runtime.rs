use pm_acceptance::{
    catalog::Catalog,
    runner::{Filter, Status, run, run_with_runtime},
    runtime::{CASES, NativeRuntime, RuntimeHook},
};
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
struct Backing {
    bundle: bool,
    fail: bool,
    calls: Mutex<Vec<String>>,
}
impl RuntimeHook for Backing {
    fn has_compat_bundle(&self) -> bool {
        self.bundle
    }
    fn check(&self, id: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(id.into());
        if self.fail {
            Err("injected backing failure".into())
        } else {
            Ok(())
        }
    }
}
#[test]
fn absent_backings_are_portable_blockers_and_filters_do_not_call_them() {
    let c = Catalog::embedded().unwrap();
    for &(id, workflow, _, _) in CASES {
        let filter = Filter {
            workflow: Some(workflow.into()),
            case: Some(id.into()),
        };
        let r = run(&c, &filter, &[]).unwrap();
        assert_eq!(r.counts(), (0, 0, 1));
        assert!(!r.cases[0].needs_macos);
        let b = Backing {
            bundle: true,
            fail: false,
            calls: Mutex::default(),
        };
        let r = run_with_runtime(&c, &filter, &[], Some(&b)).unwrap();
        assert_eq!(r.counts(), (1, 0, 0));
        assert_eq!(*b.calls.lock().unwrap(), vec![id]);
    }
}
#[test]
fn missing_bundle_is_blocked_and_backing_errors_are_failures() {
    let c = Catalog::embedded().unwrap();
    let f = Filter {
        workflow: None,
        case: Some("W037.compat-services".into()),
    };
    let b = Backing {
        bundle: false,
        fail: true,
        calls: Mutex::default(),
    };
    let r = run_with_runtime(&c, &f, &[], Some(&b)).unwrap();
    assert_eq!(r.counts(), (0, 0, 1));
    assert!(b.calls.lock().unwrap().is_empty());
    let b = Backing {
        bundle: true,
        fail: true,
        calls: Mutex::default(),
    };
    let r = run_with_runtime(&c, &f, &[], Some(&b)).unwrap();
    assert_eq!(r.cases[0].status, Status::Fail);
    assert!(r.cases[0].blocked_by.is_none());
}
#[cfg(unix)]
#[test]
fn process_backing_checks_exit_json_and_reaps_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = pm_acceptance::fixture::Scratch::new().unwrap();
    let executable = scratch.0.join("synthetic-backing");
    for (body, ok) in [
        ("printf '%s' '{\"ok\":true}'", true),
        ("printf '%s' '{\"ok\":false}'", false),
        ("printf broken", false),
        ("exit 7", false),
        ("head -c 2048 /dev/zero", false),
    ] {
        std::fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let b = NativeRuntime {
            executable: executable.clone(),
            compat_bundle: None,
        };
        assert_eq!(b.check("synthetic").is_ok(), ok, "{body}");
    }
    std::fs::write(&executable, "#!/bin/sh\nexec sleep 10\n").unwrap();
    let b = NativeRuntime {
        executable,
        compat_bundle: None,
    };
    let start = Instant::now();
    assert!(
        b.check_timeout("synthetic", Duration::from_millis(25))
            .unwrap_err()
            .contains("timed out")
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    let b = NativeRuntime {
        executable: PathBuf::from("/no/such/acceptance-backing"),
        compat_bundle: None,
    };
    assert!(b.check("synthetic").is_err());
}
#[test]
fn pending_inputs_remain_schema_valid_after_catalog_promotions() {
    for spec in pm_mcp::catalog::tools() {
        let name = spec["name"].as_str().unwrap();
        if pm_mcp::catalog::IMPLEMENTED.contains(&name) {
            continue;
        }
        let args = pm_acceptance::scenarios::pending_arguments(&spec["inputSchema"]).unwrap();
        pm_mcp::catalog::validate(name, &args).unwrap();
    }
    assert!(
        pm_acceptance::scenarios::pending_arguments(
            &serde_json::json!({"type":"string","pattern":"new-contract"})
        )
        .is_err()
    );
}
