mod support;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, write::GzEncoder};
use pm_agents::runtime::{self, Arch, Pin, Probe, Registry, Runtime};
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::Path,
};
use support::Temp;
fn tar(name: &str, kind: u8, bytes: &[u8]) -> Vec<u8> {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    h[100..108].copy_from_slice(b"0000755\0");
    let size = format!("{:011o}\0", bytes.len());
    h[124..136].copy_from_slice(size.as_bytes());
    h[148..156].fill(b' ');
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    let sum = h.iter().map(|v| *v as u64).sum::<u64>();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    let mut tar = h.to_vec();
    tar.extend(bytes);
    tar.resize(tar.len().div_ceil(512) * 512, 0);
    tar.extend([0u8; 1024]);
    let mut gz = GzEncoder::new(vec![], Compression::default());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}
struct Fake {
    metadata: Value,
    archive: Vec<u8>,
    requests: Vec<String>,
}
impl Registry for Fake {
    fn metadata(&mut self, url: &str) -> io::Result<Value> {
        self.requests.push(url.into());
        Ok(self.metadata.clone())
    }
    fn download(&mut self, url: &str) -> io::Result<Box<dyn Read>> {
        self.requests.push(url.into());
        Ok(Box::new(Cursor::new(self.archive.clone())))
    }
}
struct Version(&'static str);
impl Probe for Version {
    fn version(&mut self, binary: &Path) -> io::Result<String> {
        assert!(fs::metadata(binary)?.is_file());
        Ok(self.0.into())
    }
}
fn fake(provider: Runtime, name: &str, kind: u8) -> (Pin, Fake) {
    let archive = tar(name, kind, b"synthetic native executable fixture");
    let integrity = format!("sha512-{}", STANDARD.encode(Sha512::digest(&archive)));
    let p = Pin {
        provider,
        arch: Arch::Arm64,
        version: "1.2.3".into(),
        integrity: integrity.clone(),
    };
    let f = Fake {
        metadata: json!({"version":p.package_version(),"dist":{"tarball":"https://registry.npmjs.org/synthetic.tgz","integrity":integrity}}),
        archive,
        requests: vec![],
    };
    (p, f)
}
#[test]
fn pinned_package_selection_matches_bundled_public_lock() {
    for provider in [Runtime::Codex, Runtime::Claude] {
        for arch in [Arch::Arm64, Arch::X64] {
            let p = runtime::bundled_pin(provider, arch);
            assert!(
                p.registry_url()
                    .unwrap()
                    .starts_with("https://registry.npmjs.org/")
            );
            assert!(!p.registry_url().unwrap().contains("latest"));
            assert_eq!(
                STANDARD
                    .decode(p.integrity.strip_prefix("sha512-").unwrap())
                    .unwrap()
                    .len(),
                64
            );
        }
    }
}
#[test]
fn installer_verifies_then_atomically_activates_and_keeps_previous_versions() {
    let t = Temp::new();
    let root = t.0.join("runtimes");
    let (p, mut f) = fake(Runtime::Claude, "package/claude", b'0');
    let old = runtime::install(
        &root,
        "app-1",
        &p,
        &mut f,
        &mut Version("1.2.3 (Claude Code)"),
    )
    .unwrap();
    let next = runtime::install(&root, "app-1", &p, &mut f, &mut Version("1.2.3")).unwrap();
    assert_ne!(old, next);
    assert!(old.exists());
    assert_eq!(
        runtime::candidate(&root, Runtime::Claude, "app-1").unwrap(),
        Some(next)
    );
    assert_eq!(
        runtime::candidate(&root, Runtime::Claude, "app-2").unwrap(),
        None
    );
    assert!(f.requests.iter().all(|url| !url.contains("latest")));
}
#[test]
fn corrupt_download_cannot_replace_active_runtime() {
    let t = Temp::new();
    let root = t.0.join("runtimes");
    let (p, mut f) = fake(Runtime::Claude, "package/claude", b'0');
    let old = runtime::install(&root, "app", &p, &mut f, &mut Version("1.2.3")).unwrap();
    f.archive.push(1);
    assert!(runtime::install(&root, "app", &p, &mut f, &mut Version("1.2.3")).is_err());
    assert_eq!(
        runtime::candidate(&root, Runtime::Claude, "app").unwrap(),
        Some(old)
    );
}
#[test]
fn registry_checksum_substitution_is_rejected() {
    let t = Temp::new();
    let (p, mut f) = fake(Runtime::Claude, "package/claude", b'0');
    f.metadata["dist"]["integrity"] = json!("sha512-substituted");
    assert!(runtime::install(&t.0, "app", &p, &mut f, &mut Version("1.2.3")).is_err());
    assert_eq!(f.requests.len(), 1);
}
#[test]
fn registry_redirect_and_wrong_version_are_rejected() {
    for (key, value) in [
        ("tarball", "https://untrusted.invalid/file"),
        ("version", "1.2.4"),
    ] {
        let t = Temp::new();
        let (p, mut f) = fake(Runtime::Claude, "package/claude", b'0');
        if key == "version" {
            f.metadata[key] = json!(value);
        } else {
            f.metadata["dist"][key] = json!(value);
        }
        assert!(runtime::install(&t.0, "app", &p, &mut f, &mut Version("1.2.3")).is_err());
    }
}
#[test]
fn archive_traversal_and_linked_binary_are_rejected() {
    for (name, kind) in [("package/../claude", b'0'), ("package/claude", b'2')] {
        let t = Temp::new();
        let (p, mut f) = fake(Runtime::Claude, name, kind);
        assert!(runtime::install(&t.0, "app", &p, &mut f, &mut Version("1.2.3")).is_err());
        assert!(!t.0.join("claude").exists());
    }
}
#[test]
fn executable_version_mismatch_does_not_activate() {
    let t = Temp::new();
    let (p, mut f) = fake(Runtime::Claude, "package/claude", b'0');
    assert!(runtime::install(&t.0, "app", &p, &mut f, &mut Version("9.9.9")).is_err());
    assert!(!t.0.join("claude").exists());
}
#[test]
fn codex_extracts_native_executable_without_node_launcher() {
    let t = Temp::new();
    let (p, mut f) = fake(
        Runtime::Codex,
        "package/vendor/aarch64-apple-darwin/bin/codex",
        b'0',
    );
    let binary =
        runtime::install(&t.0, "app", &p, &mut f, &mut Version("codex-cli 1.2.3")).unwrap();
    assert!(binary.ends_with("bin/codex"));
    assert_eq!(
        fs::read(binary).unwrap(),
        b"synthetic native executable fixture"
    );
    assert!(!t.0.join("bin/codex.js").exists());
}
#[test]
fn runtime_root_cannot_be_a_symlink() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let outside = t.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let link = t.0.join("linked");
    symlink(&outside, &link).unwrap();
    let (p, mut f) = fake(Runtime::Claude, "package/claude", b'0');
    assert!(runtime::install(&link, "app", &p, &mut f, &mut Version("1.2.3")).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}
#[test]
fn version_comparison_matches_shipped_numeric_contract() {
    for (a, b, want) in [
        ("2.1.280", "2.1.251", true),
        ("0.159.1-darwin-arm64", "0.159.1", false),
        ("1.2", "1.2.0", false),
        ("1.2.1", "1.2.0", true),
    ] {
        assert_eq!(runtime::newer(a, b), want);
    }
}
#[test]
fn up_to_date_runtime_performs_no_registry_or_auth_access() {
    let t = Temp::new();
    let (p, mut f) = fake(Runtime::Claude, "package/claude", b'0');
    assert!(
        runtime::install_if_newer(&t.0, "app", &p, "1.2.3", &mut f, &mut Version("1.2.3"))
            .unwrap()
            .is_none()
    );
    assert!(f.requests.is_empty());
    assert_eq!(fs::read_dir(&t.0).unwrap().count(), 0);
}
#[test]
fn codex_update_retains_helper_binary_and_resource_tree() {
    let t = Temp::new();
    let (p, mut f) = fake(
        Runtime::Codex,
        "package/vendor/aarch64-apple-darwin/bin/codex",
        b'0',
    );
    let mut body = vec![];
    for (name, data) in [
        (
            "package/vendor/aarch64-apple-darwin/bin/codex",
            b"main".as_slice(),
        ),
        (
            "package/vendor/aarch64-apple-darwin/bin/codex-code-mode-host",
            b"native helper".as_slice(),
        ),
        (
            "package/vendor/aarch64-apple-darwin/codex-path/rg",
            b"native search".as_slice(),
        ),
        (
            "package/vendor/aarch64-apple-darwin/codex-resources/runtime.json",
            b"{\"version\":1}".as_slice(),
        ),
    ] {
        let archive = tar(name, b'0', data);
        let mut raw = vec![];
        flate2::read::GzDecoder::new(archive.as_slice())
            .read_to_end(&mut raw)
            .unwrap();
        raw.truncate(raw.len() - 1024);
        body.extend(raw);
    }
    body.extend([0; 1024]);
    let mut gz = GzEncoder::new(vec![], Compression::default());
    gz.write_all(&body).unwrap();
    f.archive = gz.finish().unwrap();
    let mut p = p;
    p.integrity = format!("sha512-{}", STANDARD.encode(Sha512::digest(&f.archive)));
    f.metadata["dist"]["integrity"] = json!(p.integrity);
    let binary = runtime::install(&t.0, "app", &p, &mut f, &mut Version("1.2.3")).unwrap();
    let root = binary.parent().unwrap().parent().unwrap();
    assert_eq!(
        fs::read(root.join("bin/codex-code-mode-host")).unwrap(),
        b"native helper"
    );
    assert_eq!(
        fs::read(root.join("codex-path/rg")).unwrap(),
        b"native search"
    );
    assert!(root.join("codex-resources/runtime.json").is_file());
}
