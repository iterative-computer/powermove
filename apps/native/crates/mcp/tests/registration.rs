use pm_mcp::{
    Result,
    registration::{Action, ClaudeCli, Consent, Registration, RegistrationConsent, Review},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
const EXE: &str = "/Applications/Powermove.app/Contents/MacOS/Powermove";
const CLIENTS: &[(&str, &str)] = &[
    ("claude-code", ".claude.json"),
    ("codex", ".codex/config.toml"),
    (
        "claude-desktop",
        "Library/Application Support/Claude/claude_desktop_config.json",
    ),
    ("gemini-cli", ".gemini/settings.json"),
    ("cursor", ".cursor/mcp.json"),
    ("vscode", "Library/Application Support/Code/User/mcp.json"),
    ("windsurf", ".codeium/windsurf/mcp_config.json"),
    ("lm-studio", ".lmstudio/mcp.json"),
    ("opencode", ".config/opencode/opencode.json"),
];
struct Fixture {
    root: PathBuf,
    service: Registration,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(pm_kernel::new_id("native-registration"));
        for d in ["home", "profile", "applications"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let root = root.canonicalize().unwrap();
        let service = Registration::new(
            root.join("home"),
            root.join("profile"),
            EXE.into(),
            root.join("applications"),
        )
        .unwrap();
        Self { root, service }
    }
    fn path(&self, id: &str) -> PathBuf {
        self.root
            .join("home")
            .join(CLIENTS.iter().find(|(name, _)| *name == id).unwrap().1)
    }
    fn write(&self, id: &str, text: &[u8]) {
        let path = self.path(id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn oracle(&self, action: &str, id: &str) -> Value {
        let mut child = Command::new("node")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/registration-pairs.mjs"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                json!({"root":self.root,"action":action,"client":id,"executable":EXE})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Ask {
    allow: bool,
    calls: AtomicUsize,
}
impl RegistrationConsent for Ask {
    fn ask(&self, _: &Review) -> Result<Consent> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(if self.allow {
            Consent::Approved
        } else {
            Consent::Denied
        })
    }
}
fn approved() -> Ask {
    Ask {
        allow: true,
        calls: AtomicUsize::new(0),
    }
}
#[test]
fn all_nine_inventory_statuses_match_pinned_main_without_writes() {
    let f = Fixture::new();
    let expected = f.oracle("inspect", "");
    assert_eq!(f.service.inspect()["clients"], expected["result"]);
    for (id, _) in CLIENTS {
        f.write(
            id,
            if *id == "codex" {
                b"# Synthetic comment\n"
            } else {
                b"{}\n"
            },
        );
    }
    assert_eq!(
        f.service.inspect()["clients"],
        f.oracle("inspect", "")["result"]
    );
    assert!(!f.root.join("profile/mcp").exists());
}
#[test]
fn all_native_json_formats_connect_disconnect_and_archive_original_bytes() {
    for (id, _) in CLIENTS
        .iter()
        .filter(|(id, _)| !["codex", "claude-code"].contains(id))
    {
        let f = Fixture::new();
        let before = b"{\"synthetic\":{\"keep\":true}}\n";
        f.write(id, before);
        let expected = f.oracle("connect", id);
        let expected_file: Value = serde_json::from_slice(&fs::read(f.path(id)).unwrap()).unwrap();
        f.write(id, before);
        let plan = f.service.prepare(id, Action::Connect).unwrap();
        let ask = approved();
        let result = f.service.apply(plan, Some(&ask)).unwrap();
        assert_eq!(result.next, expected["result"]);
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(f.path(id)).unwrap()).unwrap(),
            expected_file
        );
        assert_eq!(fs::read(result.archive.unwrap()).unwrap(), before);
        assert_eq!(
            ask.calls.load(Ordering::SeqCst),
            usize::from(*id == "claude-desktop")
        );
        let connected = fs::read(f.path(id)).unwrap();
        f.oracle("disconnect", id);
        let disconnected: Value = serde_json::from_slice(&fs::read(f.path(id)).unwrap()).unwrap();
        f.write(id, &connected);
        let result = f
            .service
            .apply(f.service.prepare(id, Action::Disconnect).unwrap(), None)
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(f.path(id)).unwrap()).unwrap(),
            disconnected
        );
        assert_eq!(fs::read(result.archive.unwrap()).unwrap(), connected);
        if *id == "claude-desktop" {
            assert_eq!(
                serde_json::from_slice::<Value>(
                    &fs::read(f.root.join("profile/mcp/claude-desktop-registration.json")).unwrap()
                )
                .unwrap(),
                json!({"registered":true})
            );
        }
    }
}
#[test]
fn claude_desktop_missing_or_denied_consent_changes_nothing() {
    let f = Fixture::new();
    let denied = Ask {
        allow: false,
        calls: AtomicUsize::new(0),
    };
    assert!(
        f.service
            .apply(
                f.service
                    .prepare("claude-desktop", Action::Connect)
                    .unwrap(),
                None
            )
            .is_err()
    );
    assert!(
        f.service
            .apply(
                f.service
                    .prepare("claude-desktop", Action::Connect)
                    .unwrap(),
                Some(&denied)
            )
            .is_err()
    );
    assert_eq!(denied.calls.load(Ordering::SeqCst), 1);
    assert!(!f.path("claude-desktop").exists());
    assert!(!f.root.join("home/Library").exists());
    assert!(!f.root.join("profile/mcp").exists());
}
#[test]
fn handwritten_entries_and_json_comments_are_preserved() {
    let f = Fixture::new();
    for id in ["cursor", "vscode", "opencode", "claude-desktop"] {
        let key = if id == "vscode" {
            "servers"
        } else if id == "opencode" {
            "mcp"
        } else {
            "mcpServers"
        };
        let before =
            json!({key:{"powermove":{"command":"handwritten","args":["keep"]}}}).to_string();
        f.write(id, before.as_bytes());
        let result = f
            .service
            .apply(f.service.prepare(id, Action::Connect).unwrap(), None)
            .unwrap();
        assert!(!result.changed);
        assert_eq!(result.next, "Already set up.");
        assert!(f.service.prepare(id, Action::Disconnect).is_err());
        assert_eq!(fs::read_to_string(f.path(id)).unwrap(), before);
    }
    f.write("vscode", b"// Keep this comment\n{\"servers\":{}}\n");
    let expected = f.oracle("connect", "vscode");
    let error = f.service.prepare("vscode", Action::Connect).err().unwrap();
    assert_eq!(error.message, expected["error"]);
    assert!(
        fs::read_to_string(f.path("vscode"))
            .unwrap()
            .starts_with("// Keep")
    );
}
#[test]
fn codex_toml_retains_comments_other_tables_and_custom_server_ownership() {
    let f = Fixture::new();
    let before = b"# Keep original settings\nmodel = \"synthetic\"\n\n[other]\nflag = true\n";
    f.write("codex", before);
    let expected = f.oracle("connect", "codex");
    let expected_text = fs::read(f.path("codex")).unwrap();
    f.write("codex", before);
    let result = f
        .service
        .apply(f.service.prepare("codex", Action::Connect).unwrap(), None)
        .unwrap();
    assert_eq!(result.next, expected["result"]);
    assert_eq!(fs::read(f.path("codex")).unwrap(), expected_text);
    assert_eq!(fs::read(result.archive.unwrap()).unwrap(), before);
    let connected = fs::read(f.path("codex")).unwrap();
    f.oracle("disconnect", "codex");
    let disconnected = fs::read(f.path("codex")).unwrap();
    f.write("codex", &connected);
    f.service
        .apply(
            f.service.prepare("codex", Action::Disconnect).unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(fs::read(f.path("codex")).unwrap(), disconnected);
    for custom in [
        "[mcp_servers.powermove]\ncommand = \"custom\"\nargs = []\n",
        "mcp_servers.powermove.command = \"custom\"\n",
        "[mcp_servers]\npowermove = { command = \"custom\" }\n",
    ] {
        f.write("codex", custom.as_bytes());
        assert!(
            !f.service
                .prepare("codex", Action::Connect)
                .unwrap()
                .review()
                .changes
        );
        assert!(f.service.prepare("codex", Action::Disconnect).is_err());
        assert_eq!(fs::read_to_string(f.path("codex")).unwrap(), custom);
    }
}
#[test]
fn changes_during_consent_refuse_stale_publication() {
    struct Steering {
        path: PathBuf,
    }
    impl RegistrationConsent for Steering {
        fn ask(&self, _: &Review) -> Result<Consent> {
            fs::write(&self.path, b"{\"foreign\":true}\n")?;
            Ok(Consent::Approved)
        }
    }
    let f = Fixture::new();
    f.write("claude-desktop", b"{}\n");
    let plan = f
        .service
        .prepare("claude-desktop", Action::Connect)
        .unwrap();
    let result = f.service.apply(
        plan,
        Some(&Steering {
            path: f.path("claude-desktop"),
        }),
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read(f.path("claude-desktop")).unwrap(),
        b"{\"foreign\":true}\n"
    );
    assert!(!f.root.join("profile/mcp").exists());
}
#[test]
fn cli_registration_uses_native_adapter_and_exact_scope_argv() {
    struct Cli(Mutex<Vec<Vec<String>>>);
    impl ClaudeCli for Cli {
        fn run_mcp(&self, args: &[String]) -> Result<()> {
            self.0.lock().unwrap().push(args.to_vec());
            Ok(())
        }
    }
    let mut f = Fixture::new();
    f.write("claude-code", b"{}\n");
    assert!(
        f.service
            .apply(
                f.service.prepare("claude-code", Action::Connect).unwrap(),
                None
            )
            .is_err()
    );
    let cli = Arc::new(Cli(Mutex::new(vec![])));
    f.service.claude_cli = Some(cli.clone());
    let expected = f.oracle("connect", "claude-code");
    f.service
        .apply(
            f.service.prepare("claude-code", Action::Connect).unwrap(),
            None,
        )
        .unwrap();
    assert_eq!(
        json!(*cli.0.lock().unwrap()),
        json!(
            expected["calls"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| json!(a.as_array().unwrap().iter().skip(1).collect::<Vec<_>>()))
                .collect::<Vec<_>>()
        )
    );
}
#[cfg(unix)]
#[test]
fn linked_settings_and_ancestors_are_never_overwritten() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = f.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("mcp.json"), b"{\"preserve\":true}\n").unwrap();
    symlink(&outside, f.root.join("home/.cursor")).unwrap();
    assert!(f.service.prepare("cursor", Action::Connect).is_err());
    assert_eq!(
        fs::read(outside.join("mcp.json")).unwrap(),
        b"{\"preserve\":true}\n"
    );
}
#[test]
fn reviewed_change_cannot_be_applied_by_a_different_native_host() {
    let f = Fixture::new();
    let other = Fixture::new();
    let plan = f.service.prepare("cursor", Action::Connect).unwrap();
    assert!(other.service.apply(plan, None).is_err());
    assert!(!f.path("cursor").exists());
    assert!(!other.path("cursor").exists());
}
#[cfg(unix)]
#[test]
fn archives_are_private_and_existing_settings_permissions_survive() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.write("cursor", b"{}\n");
    fs::set_permissions(f.path("cursor"), fs::Permissions::from_mode(0o640)).unwrap();
    let result = f
        .service
        .apply(f.service.prepare("cursor", Action::Connect).unwrap(), None)
        .unwrap();
    assert_eq!(
        fs::metadata(f.path("cursor")).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::metadata(result.archive.unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn native_bundle_executable_remains_owned_after_connect_and_can_disconnect() {
    let mut f = Fixture::new();
    f.service = Registration::new(
        f.root.join("home"),
        f.root.join("profile"),
        "/Applications/Powermove Native.app/Contents/MacOS/powermove-native".into(),
        f.root.join("applications"),
    )
    .unwrap();
    f.service
        .apply(f.service.prepare("cursor", Action::Connect).unwrap(), None)
        .unwrap();
    let status = f.service.inspect()["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "cursor")
        .unwrap()
        .clone();
    assert_eq!(status["state"], "connected");
    assert_eq!(status["custom"], false);
    f.service
        .apply(
            f.service.prepare("cursor", Action::Disconnect).unwrap(),
            None,
        )
        .unwrap();
    let after: Value = serde_json::from_slice(&fs::read(f.path("cursor")).unwrap()).unwrap();
    assert!(after["mcpServers"].get("powermove").is_none());
}
