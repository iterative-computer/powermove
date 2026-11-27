use pm_mcp::{
    registration::{Action, ClaudeCli, Registration},
    registration_cli::NativeClaudeCli,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
struct Fixture {
    root: PathBuf,
    cli: PathBuf,
    config: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("pm-registration-cli"));
        for name in ["home", "profile", "apps"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        let root = root.canonicalize().unwrap();
        let cli = root.join("native cli ; literal");
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/registration-cli.py"),
            &cli,
        )
        .unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let config = root.join("home/.claude.json");
        fs::write(&config, b"{\"foreign\":{\"keep\":true}}\n").unwrap();
        Self { root, cli, config }
    }
    fn adapter(&self, mode: &str) -> NativeClaudeCli {
        NativeClaudeCli::new(
            self.cli.clone(),
            self.root.clone(),
            BTreeMap::from([
                ("PATH".into(), std::env::var("PATH").unwrap()),
                ("PM_REGISTRATION_TEST_MODE".into(), mode.into()),
                (
                    "PM_REGISTRATION_TEST_CONFIG".into(),
                    self.config.to_str().unwrap().into(),
                ),
            ]),
        )
    }
    fn registration(&self, mode: &str) -> Registration {
        let mut service = Registration::new(
            self.root.join("home"),
            self.root.join("profile"),
            self.root
                .join("Powermove Native.app/Contents/MacOS/powermove-native"),
            self.root.join("apps"),
        )
        .unwrap();
        service.claude_cli = Some(Arc::new(self.adapter(mode)));
        service
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn real_native_cli_connect_readback_disconnect_and_archive_original() {
    let f = Fixture::new();
    let original = fs::read(&f.config).unwrap();
    let service = f.registration("write");
    let connected = service
        .apply(
            service.prepare("claude-code", Action::Connect).unwrap(),
            None,
        )
        .unwrap();
    assert!(connected.changed);
    assert_eq!(fs::read(connected.archive.unwrap()).unwrap(), original);
    let config: Value = serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
    assert_eq!(config["foreign"], json!({"keep":true}));
    assert_eq!(
        config["mcpServers"]["powermove"]["command"],
        json!(
            f.root
                .join("Powermove Native.app/Contents/MacOS/powermove-native")
        )
    );
    assert_eq!(service.inspect()["clients"][0]["state"], "connected");
    service
        .apply(
            service.prepare("claude-code", Action::Disconnect).unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(service.inspect()["clients"][0]["state"], "available");
}
#[test]
fn exit_success_without_config_publication_never_claims_registration() {
    let f = Fixture::new();
    let before = fs::read(&f.config).unwrap();
    let service = f.registration("no-write");
    let result = service.apply(
        service.prepare("claude-code", Action::Connect).unwrap(),
        None,
    );
    assert_eq!(
        result.err().unwrap().message,
        "Claude Code did not publish the requested Powermove connection."
    );
    assert_eq!(fs::read(&f.config).unwrap(), before);
}
#[test]
fn both_pipes_drain_and_original_cli_error_survives_cleanup() {
    let f = Fixture::new();
    let error = f.adapter("fail").run_mcp(&["add".into()]).unwrap_err();
    assert_eq!(error.message, "Original CLI error.");
}
#[test]
fn stalled_or_flooding_cli_is_bounded_and_reaped() {
    for mode in ["stall", "flood"] {
        let f = Fixture::new();
        let start = Instant::now();
        let error = f
            .adapter(mode)
            .with_timeout(if mode == "stall" {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(5)
            })
            .run_mcp(&["add".into()])
            .unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(8));
        assert!(error.message.contains(if mode == "stall" {
            "timed out"
        } else {
            "exceeded"
        }));
    }
}
