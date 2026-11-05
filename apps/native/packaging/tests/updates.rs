use anyhow::{Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use pm_distribution::{Ed25519Verifier, HttpsTransport, update_distribution::*, update_handoff::*};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs,
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
};
use tempfile::TempDir;
use url::Url;

const NOW: u64 = 1_800_000_000;
fn ready() -> Eligibility {
    Eligibility {
        packaged_macos: true,
        macos_major: 14,
        installed_identity_verified: true,
        transition_or_native_receipt_verified: true,
        owner_approved: true,
        source_generation: 42,
        checkpoint_generation: 42,
        final_delta_complete: true,
        all_six_owner_domains: true,
        state_and_secrets_complete: true,
        installer_recovery_verified: true,
    }
}
struct Gate(VecDeque<Eligibility>);
impl Gate {
    fn open() -> Self {
        Self(VecDeque::from([ready()]))
    }
}
impl Readiness for Gate {
    fn now_unix(&mut self) -> Result<u64> {
        Ok(NOW)
    }
    fn current(&mut self) -> Result<Eligibility> {
        Ok(if self.0.len() > 1 {
            self.0.pop_front().unwrap()
        } else {
            *self.0.front().unwrap()
        })
    }
}
struct Fixture {
    config: UpdateConfig,
    release: Release,
    key: SigningKey,
    archive: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        // Deterministic synthetic signing material, generated only at test runtime.
        let key =
            SigningKey::from_bytes(&Sha256::digest(b"synthetic distribution test signer").into());
        let archive = b"synthetic ZIP bytes; the fake installer never extracts these".to_vec();
        Self {
            config: UpdateConfig {
                version: 1,
                enabled: true,
                feed_url: Some("https://updates.invalid/native/policy.json".into()),
                public_key: Some(STANDARD.encode(key.verifying_key().as_bytes())),
                channel: Channel::Cutover,
            },
            release: Release {
                schema: 1,
                channel: Channel::Cutover,
                version: "2.0.0".into(),
                sparkle_version: None,
                architecture: Architecture::Arm64,
                bundle_id: PRODUCTION_ID.into(),
                minimum_macos: 14,
                archive_url: "https://updates.invalid/native/app.zip".into(),
                size: archive.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&archive)),
                sparkle_signature: STANDARD.encode(key.sign(&archive).to_bytes()),
                expires_unix: NOW + 600,
            },
            key,
            archive,
        }
    }
    fn verifier(&self) -> Ed25519Verifier {
        Ed25519Verifier::from_base64(self.config.public_key.as_ref().unwrap()).unwrap()
    }
    fn policy(&self) -> Vec<u8> {
        let payload = serde_json::to_string(&self.release).unwrap();
        let mut message = SIGNING_DOMAIN.to_vec();
        message.extend_from_slice(payload.as_bytes());
        serde_json::to_vec(&SignedPolicy {
            payload,
            signature: STANDARD.encode(self.key.sign(&message).to_bytes()),
        })
        .unwrap()
    }
    fn offer(&self) -> Offer {
        let mut feed = LocalFeed::new([self.policy()]);
        let Check::Available(offer) = check(
            &self.config,
            ready(),
            "1.2.0",
            Architecture::Arm64,
            NOW,
            &mut feed,
            &self.verifier(),
        )
        .unwrap() else {
            panic!("expected update");
        };
        offer
    }
}
/// Each request crosses an actual loopback HTTP socket. The production HTTPS
/// client remains strict; this transport maps only the fixed synthetic host.
struct LocalFeed {
    responses: VecDeque<Vec<u8>>,
    calls: usize,
    fail_read: bool,
}
impl LocalFeed {
    fn new(responses: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            responses: responses.into_iter().collect(),
            calls: 0,
            fail_read: false,
        }
    }
}
impl Transport for LocalFeed {
    fn open(&mut self, url: &Url) -> Result<Box<dyn Read>> {
        assert_eq!(url.host_str(), Some("updates.invalid"));
        self.calls += 1;
        let body = self.responses.pop_front().expect("unexpected network call");
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let path = url.path().to_owned();
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            connection
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                connection.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() <= 2048);
            }
            assert!(
                String::from_utf8_lossy(&request).starts_with(&format!("GET {path} HTTP/1.1\r\n"))
            );
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            connection.write_all(&body).unwrap();
        });
        let mut connection = TcpStream::connect(address)?;
        write!(
            connection,
            "GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            url.path()
        )?;
        let mut response = Vec::new();
        connection.read_to_end(&mut response)?;
        server.join().unwrap();
        let start = response.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
        let body = response[start..].to_vec();
        if self.fail_read {
            Ok(Box::new(BrokenReader {
                bytes: Cursor::new(body),
                delivered: false,
            }))
        } else {
            Ok(Box::new(Cursor::new(body)))
        }
    }
}
struct BrokenReader {
    bytes: Cursor<Vec<u8>>,
    delivered: bool,
}
impl Read for BrokenReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.delivered {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "synthetic interruption",
            ));
        }
        self.delivered = true;
        self.bytes.read(&mut buffer[..3])
    }
}
#[derive(Default)]
struct FakeInstaller {
    stage_calls: usize,
    install_calls: usize,
    recover_calls: usize,
    low_space: bool,
    stage_failure: bool,
    install_failure: bool,
    installed: bool,
    refuse_signature: bool,
}
impl Installer for FakeInstaller {
    fn available_space(&mut self, _: &Path) -> Result<u64> {
        Ok(if self.low_space { 0 } else { u64::MAX })
    }
    fn stage(&mut self, _: &str, _: &Path, _: &Release) -> Result<String> {
        self.stage_calls += 1;
        if self.stage_failure || self.refuse_signature {
            bail!("synthetic Sparkle staging/signature failure");
        }
        Ok("sparkle-ticket".into())
    }
    fn recover(&mut self, _: &str, _: &Stage) -> Result<Recovery> {
        self.recover_calls += 1;
        Ok(if self.installed {
            Recovery::Installed
        } else {
            Recovery::Staged {
                ticket: "sparkle-ticket".into(),
            }
        })
    }
    fn request_install(&mut self, _: &str, _: &str) -> Result<()> {
        self.install_calls += 1;
        if self.install_failure {
            bail!("synthetic interruption after install request");
        }
        Ok(())
    }
}
struct Barrier(bool);
impl QuitBarrier for Barrier {
    fn ready(&mut self) -> Result<bool> {
        Ok(self.0)
    }
}
fn store() -> (TempDir, StagingStore) {
    let temp = tempfile::tempdir().unwrap();
    let store = StagingStore::open(&temp.path().canonicalize().unwrap().join("update")).unwrap();
    (temp, store)
}
fn stage(fixture: &Fixture, store: &StagingStore, installer: &mut FakeInstaller) -> Result<()> {
    store.download_and_stage(
        &fixture.offer(),
        &mut Gate::open(),
        NOW,
        &mut LocalFeed::new([fixture.archive.clone()]),
        &fixture.verifier(),
        installer,
    )
}

#[test]
fn local_signed_feed_and_archive_stage_then_normal_quit() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    assert_eq!(installer.stage_calls, 1);
    assert_eq!(installer.install_calls, 0);
    assert!(matches!(
        store.status().unwrap(),
        Some((_, Stage::Staged { .. }))
    ));
    assert!(
        store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .unwrap()
    );
    assert_eq!(installer.install_calls, 1);
    installer.installed = true;
    assert_eq!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW + 900)
            .unwrap(),
        Some(Stage::Completed)
    );
}
#[test]
fn every_closed_gate_prevents_contacting_the_native_feed() {
    let f = Fixture::new();
    let mut variants = Vec::new();
    macro_rules! closed {
        ($field:ident) => {{
            let mut e = ready();
            e.$field = false;
            variants.push(e);
        }};
    }
    closed!(packaged_macos);
    closed!(installed_identity_verified);
    closed!(transition_or_native_receipt_verified);
    closed!(owner_approved);
    closed!(final_delta_complete);
    closed!(all_six_owner_domains);
    closed!(state_and_secrets_complete);
    closed!(installer_recovery_verified);
    let mut old = ready();
    old.macos_major = 13;
    variants.push(old);
    let mut stale = ready();
    stale.source_generation += 1;
    variants.push(stale);
    let mut feed = LocalFeed::new([]);
    for eligibility in variants {
        assert!(matches!(
            check(
                &f.config,
                eligibility,
                "1.2.0",
                Architecture::Arm64,
                NOW,
                &mut feed,
                &f.verifier()
            )
            .unwrap(),
            Check::Gated
        ));
    }
    assert_eq!(feed.calls, 0);
}
#[test]
fn disabled_bundled_config_has_no_endpoint_or_key_and_never_contacts_network() {
    let config: UpdateConfig = serde_json::from_str(include_str!("../update-config.json")).unwrap();
    assert!(!config.enabled);
    assert!(config.feed_url.is_none());
    assert!(config.public_key.is_none());
    let mut feed = LocalFeed::new([]);
    assert!(matches!(
        check(
            &config,
            ready(),
            "1.2.0",
            Architecture::Arm64,
            NOW,
            &mut feed,
            &Fixture::new().verifier()
        )
        .unwrap(),
        Check::Gated
    ));
}
#[test]
fn bad_signature_wrong_key_and_modified_signed_bytes_are_rejected() {
    let f = Fixture::new();
    let mut bad: SignedPolicy = serde_json::from_slice(&f.policy()).unwrap();
    bad.payload.push(' ');
    let wrong = SigningKey::from_bytes(&Sha256::digest(b"different synthetic signer").into());
    let wrong =
        Ed25519Verifier::from_base64(&STANDARD.encode(wrong.verifying_key().as_bytes())).unwrap();
    let mut feed = LocalFeed::new([f.policy(), serde_json::to_vec(&bad).unwrap()]);
    assert!(
        check(
            &f.config,
            ready(),
            "1.2.0",
            Architecture::Arm64,
            NOW,
            &mut feed,
            &wrong
        )
        .is_err()
    );
    assert!(
        check(
            &f.config,
            ready(),
            "1.2.0",
            Architecture::Arm64,
            NOW,
            &mut feed,
            &f.verifier()
        )
        .is_err()
    );
}
#[test]
fn weak_malformed_and_wrong_length_public_keys_fail() {
    for key in [
        "not-base64".to_owned(),
        STANDARD.encode([0; 31]),
        STANDARD.encode([0; 32]),
    ] {
        assert!(Ed25519Verifier::from_base64(&key).is_err());
    }
}
#[test]
fn target_identity_schema_expiry_and_digest_are_validated_after_signature() {
    let mut f = Fixture::new();
    for field in 0..8 {
        f.release = Fixture::new().release;
        match field {
            0 => f.release.architecture = Architecture::X86_64,
            1 => f.release.channel = Channel::Beta,
            2 => f.release.bundle_id = "com.untrusted.app".into(),
            3 => f.release.schema = 9,
            4 => f.release.expires_unix = NOW,
            5 => f.release.sha256 = "x".repeat(64),
            6 => f.release.sparkle_signature.clear(),
            _ => f.release.size = u64::MAX,
        }
        assert!(
            check(
                &f.config,
                ready(),
                "1.2.0",
                Architecture::Arm64,
                NOW,
                &mut LocalFeed::new([f.policy()]),
                &f.verifier()
            )
            .is_err()
        );
    }
}
#[test]
fn stable_rejects_prerelease_downgrade_and_unsupported_macos() {
    let mut f = Fixture::new();
    for (version, floor) in [
        ("1.0.0", 14),
        ("1.2.0", 14),
        ("2.0.0-beta.1", 14),
        ("2.0.0", 26),
    ] {
        f.release.version = version.into();
        f.release.minimum_macos = floor;
        assert!(matches!(
            check(
                &f.config,
                ready(),
                "1.2.0",
                Architecture::Arm64,
                NOW,
                &mut LocalFeed::new([f.policy()]),
                &f.verifier()
            )
            .unwrap(),
            Check::Current
        ));
    }
}
#[test]
fn beta_is_isolated_and_allows_beta_updates() {
    let mut f = Fixture::new();
    f.config.channel = Channel::Beta;
    f.release.channel = Channel::Beta;
    f.release.bundle_id = BETA_ID.into();
    f.release.version = "2.0.0-beta.2".into();
    assert!(matches!(
        check(
            &f.config,
            ready(),
            "2.0.0-beta.1",
            Architecture::Arm64,
            NOW,
            &mut LocalFeed::new([f.policy()]),
            &f.verifier()
        )
        .unwrap(),
        Check::Available(_)
    ));
}
#[test]
fn malicious_config_and_archive_urls_are_rejected() {
    let mut f = Fixture::new();
    for url in [
        "http://updates.invalid/a",
        "https://name:pass@updates.invalid/a",
        "https://updates.invalid/a?q=1",
        "https://updates.invalid/a#x",
        "file:///tmp/app",
    ] {
        f.config.feed_url = Some(url.into());
        assert!(
            check(
                &f.config,
                ready(),
                "1.2.0",
                Architecture::Arm64,
                NOW,
                &mut LocalFeed::new([]),
                &f.verifier()
            )
            .is_err()
        );
        f.release.archive_url = url.into();
        f.config.feed_url = Fixture::new().config.feed_url;
        assert!(
            check(
                &f.config,
                ready(),
                "1.2.0",
                Architecture::Arm64,
                NOW,
                &mut LocalFeed::new([f.policy()]),
                &f.verifier()
            )
            .is_err()
        );
    }
    f.config.feed_url = Some("https://updates.invalid/latest-mac.yml".into());
    assert!(
        check(
            &f.config,
            ready(),
            "1.2.0",
            Architecture::Arm64,
            NOW,
            &mut LocalFeed::new([]),
            &f.verifier()
        )
        .is_err()
    );
}
#[test]
fn oversized_and_malformed_policy_fail_before_staging() {
    let f = Fixture::new();
    for bytes in [
        vec![b'x'; 128 * 1024 + 1],
        b"{}".to_vec(),
        b"{not json}".to_vec(),
    ] {
        assert!(
            check(
                &f.config,
                ready(),
                "1.2.0",
                Architecture::Arm64,
                NOW,
                &mut LocalFeed::new([bytes]),
                &f.verifier()
            )
            .is_err()
        );
    }
}
#[test]
fn production_transport_does_not_allow_http_even_for_loopback() {
    let mut transport = HttpsTransport::new().unwrap();
    assert!(
        transport
            .open(&Url::parse("http://127.0.0.1:1/policy").unwrap())
            .is_err()
    );
}
#[test]
fn bad_checksum_truncated_and_oversized_archives_preserve_bytes_without_staging() {
    let f = Fixture::new();
    for bytes in [
        b"wrong bytes".to_vec(),
        vec![0; f.archive.len()],
        vec![0; f.archive.len() + 1],
    ] {
        let (_temp, store) = store();
        let mut installer = FakeInstaller::default();
        assert!(
            store
                .download_and_stage(
                    &f.offer(),
                    &mut Gate::open(),
                    NOW,
                    &mut LocalFeed::new([bytes]),
                    &f.verifier(),
                    &mut installer
                )
                .is_err()
        );
        assert_eq!(installer.stage_calls, 0);
        assert!(matches!(
            store.status().unwrap(),
            Some((_, Stage::Downloading))
        ));
        assert!(fs::read_dir(store.root()).unwrap().any(|e| {
            e.unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "part")
        }));
    }
}
#[test]
fn interrupted_download_restarts_into_a_new_retained_attempt() {
    let f = Fixture::new();
    let (temp, store) = store();
    let mut installer = FakeInstaller::default();
    let mut feed = LocalFeed::new([f.archive.clone()]);
    feed.fail_read = true;
    assert!(
        store
            .download_and_stage(
                &f.offer(),
                &mut Gate::open(),
                NOW,
                &mut feed,
                &f.verifier(),
                &mut installer
            )
            .is_err()
    );
    drop(store);
    let store = StagingStore::open(&temp.path().canonicalize().unwrap().join("update")).unwrap();
    assert_eq!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
            .unwrap(),
        Some(Stage::Downloading)
    );
    stage(&f, &store, &mut installer).unwrap();
    let parts: Vec<_> = fs::read_dir(store.root())
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().is_some_and(|e| e == "part")).then_some(p)
        })
        .collect();
    assert_eq!(parts.len(), 2);
    assert!(parts.iter().any(|p| fs::read(p).unwrap().len() == 3));
}
#[test]
fn low_disk_does_not_contact_archive_or_create_a_receipt() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut feed = LocalFeed::new([]);
    let mut installer = FakeInstaller {
        low_space: true,
        ..Default::default()
    };
    assert!(
        store
            .download_and_stage(
                &f.offer(),
                &mut Gate::open(),
                NOW,
                &mut feed,
                &f.verifier(),
                &mut installer
            )
            .is_err()
    );
    assert!(store.status().unwrap().is_none());
    assert_eq!(feed.calls, 0);
}
#[test]
fn migration_change_during_download_blocks_staging_and_recovery_can_resume_later() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    let mut stale = ready();
    stale.source_generation += 1;
    let mut gate = Gate(VecDeque::from([ready(), stale]));
    assert!(
        store
            .download_and_stage(
                &f.offer(),
                &mut gate,
                NOW,
                &mut LocalFeed::new([f.archive.clone()]),
                &f.verifier(),
                &mut installer
            )
            .is_err()
    );
    assert_eq!(installer.stage_calls, 0);
    assert!(matches!(
        store.status().unwrap(),
        Some((_, Stage::Downloaded { .. }))
    ));
    assert!(
        store
            .recover(&f.verifier(), &mut installer, &mut gate, NOW)
            .is_err()
    );
    store
        .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
        .unwrap();
    assert_eq!(installer.stage_calls, 1);
}
#[test]
fn stage_interruption_uses_idempotent_installer_recovery_without_duplicate_stage() {
    let f = Fixture::new();
    let (temp, store) = store();
    let mut installer = FakeInstaller {
        stage_failure: true,
        ..Default::default()
    };
    assert!(stage(&f, &store, &mut installer).is_err());
    assert_eq!(installer.stage_calls, 1);
    drop(store);
    let store = StagingStore::open(&temp.path().canonicalize().unwrap().join("update")).unwrap();
    store
        .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
        .unwrap();
    assert_eq!(installer.stage_calls, 1);
    assert_eq!(installer.recover_calls, 1);
    assert!(matches!(
        store.status().unwrap(),
        Some((_, Stage::Staged { .. }))
    ));
    stage(&f, &store, &mut installer).unwrap();
    assert_eq!(installer.stage_calls, 1);
}
#[test]
fn installer_signature_rejection_never_requests_install() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller {
        refuse_signature: true,
        ..Default::default()
    };
    assert!(stage(&f, &store, &mut installer).is_err());
    assert!(
        store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .is_err()
    );
    assert_eq!(installer.install_calls, 0);
}
#[test]
fn cancelled_quit_and_changed_generation_after_barrier_leave_staged_update() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    assert!(
        !store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(false),
                &mut installer
            )
            .unwrap()
    );
    let mut stale = ready();
    stale.source_generation += 1;
    let mut gate = Gate(VecDeque::from([ready(), stale]));
    assert!(
        store
            .request_install(
                &mut gate,
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .is_err()
    );
    assert_eq!(installer.install_calls, 0);
    assert!(matches!(
        store.status().unwrap(),
        Some((_, Stage::Staged { .. }))
    ));
}
#[test]
fn failure_after_durable_install_request_recovers_without_another_quit() {
    let f = Fixture::new();
    let (temp, store) = store();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    installer.install_failure = true;
    assert!(
        store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .is_err()
    );
    drop(store);
    let store = StagingStore::open(&temp.path().canonicalize().unwrap().join("update")).unwrap();
    installer.installed = true;
    assert_eq!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW + 1000)
            .unwrap(),
        Some(Stage::Completed)
    );
    assert_eq!(installer.install_calls, 1);
}
#[test]
fn tampered_archive_is_rehashed_before_install_and_recovery() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    let Some((_, Stage::Staged { file, .. })) = store.status().unwrap() else {
        panic!()
    };
    fs::write(store.root().join(file), vec![0; f.archive.len()]).unwrap();
    assert!(
        store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .is_err()
    );
    assert!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
            .is_err()
    );
    assert_eq!(installer.install_calls, 0);
}
#[test]
fn process_lock_excludes_another_updater_and_releases_on_drop() {
    let (temp, store) = store();
    assert!(StagingStore::open(store.root()).is_err());
    drop(store);
    StagingStore::open(&temp.path().canonicalize().unwrap().join("update")).unwrap();
}
#[test]
fn torn_unpublished_receipt_is_retained_and_committed_corruption_fails_closed() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let pending = store.root().join("receipt-00000000-1.pending");
    fs::write(&pending, b"{torn").unwrap();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    assert_eq!(fs::read(pending).unwrap(), b"{torn");
    fs::write(store.root().join("receipt-00000000.json"), b"{torn").unwrap();
    assert!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
            .is_err()
    );
}
#[test]
fn persisted_policy_signature_and_receipt_sequence_are_checked() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    let path = fs::read_dir(store.root())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .max()
        .unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["offer"]["id"] = "wrong".into();
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
            .is_err()
    );
    fs::remove_file(store.root().join("receipt-00000000.json")).unwrap();
    assert!(store.status().is_err());
}
#[test]
#[cfg(unix)]
fn symlink_staging_lock_and_archive_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().canonicalize().unwrap().join("real");
    fs::create_dir(&real).unwrap();
    let linked = temp.path().canonicalize().unwrap().join("linked");
    symlink(&real, &linked).unwrap();
    assert!(StagingStore::open(&linked).is_err());
    symlink(
        temp.path().canonicalize().unwrap().join("absent"),
        real.join("update.lock"),
    )
    .unwrap();
    assert!(StagingStore::open(&real).is_err());
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    let Some((_, Stage::Staged { file, .. })) = store.status().unwrap() else {
        panic!()
    };
    let archive = store.root().join(file);
    fs::remove_file(&archive).unwrap();
    symlink(temp.path().canonicalize().unwrap().join("absent"), archive).unwrap();
    assert!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
            .is_err()
    );
}

struct HandoffFake {
    quiescent: bool,
    verified: bool,
    delta: DeltaReport,
    delta_calls: usize,
    verify_calls: usize,
}
impl HandoffFake {
    fn ready() -> Self {
        Self {
            quiescent: true,
            verified: true,
            delta: DeltaReport {
                complete: true,
                six_owner_domains: true,
                state_complete: true,
                secrets_complete: true,
                installer_recovery_verified: true,
                source_generation: 42,
                checkpoint_generation: 42,
            },
            delta_calls: 0,
            verify_calls: 0,
        }
    }
}
impl HandoffHost for HandoffFake {
    fn electron_quiescent(&mut self) -> Result<bool> {
        Ok(self.quiescent)
    }
    fn verify_transition_receipt(&mut self) -> Result<bool> {
        self.verify_calls += 1;
        Ok(self.verified)
    }
    fn final_delta(&mut self, reuse_transition_evidence: bool) -> Result<DeltaReport> {
        assert!(!reuse_transition_evidence || self.verified);
        self.delta_calls += 1;
        Ok(self.delta)
    }
}
#[test]
fn dmg_and_transition_first_launch_always_run_final_delta() {
    for source in [LaunchSource::DirectDmg, LaunchSource::TransitionRelease] {
        let mut host = HandoffFake::ready();
        let result = first_run(source, &mut host).unwrap();
        assert_eq!(result.status, HandoffStatus::Ready);
        assert_eq!(host.delta_calls, 1);
        assert_eq!(
            result.reuse_transition_evidence,
            source == LaunchSource::TransitionRelease
        );
        assert_eq!(
            host.verify_calls,
            usize::from(source == LaunchSource::TransitionRelease)
        );
    }
}
#[test]
fn no_handoff_or_delta_access_while_electron_is_running() {
    let mut host = HandoffFake::ready();
    host.quiescent = false;
    assert_eq!(
        first_run(LaunchSource::TransitionRelease, &mut host)
            .unwrap()
            .status,
        HandoffStatus::WaitingForElectron
    );
    assert_eq!(host.verify_calls + host.delta_calls, 0);
}
#[test]
fn invalid_tr_evidence_falls_back_to_direct_delta_without_inheriting_readiness() {
    let mut host = HandoffFake::ready();
    host.verified = false;
    host.delta.complete = false;
    let handoff = first_run(LaunchSource::TransitionRelease, &mut host).unwrap();
    assert!(!handoff.reuse_transition_evidence);
    assert_eq!(handoff.status, HandoffStatus::NeedsRecovery);
    assert_eq!(host.delta_calls, 1);
}
#[test]
fn incomplete_secret_handoff_requires_consent_and_never_queries_keychain_here() {
    let mut host = HandoffFake::ready();
    host.delta.secrets_complete = false;
    assert_eq!(
        first_run(LaunchSource::DirectDmg, &mut host)
            .unwrap()
            .status,
        HandoffStatus::NeedsSecretConsent
    );
}
#[test]
fn missing_owner_state_restore_or_stale_delta_blocks_library_open() {
    for field in 0..4 {
        let mut host = HandoffFake::ready();
        match field {
            0 => host.delta.six_owner_domains = false,
            1 => host.delta.state_complete = false,
            2 => host.delta.installer_recovery_verified = false,
            _ => host.delta.source_generation += 1,
        }
        assert_eq!(
            first_run(LaunchSource::DirectDmg, &mut host)
                .unwrap()
                .status,
            HandoffStatus::Blocked
        );
    }
}

#[test]
fn recovery_of_an_unperformed_install_request_requires_a_new_quit_barrier() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    stage(&f, &store, &mut installer).unwrap();
    installer.install_failure = true;
    assert!(
        store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .is_err()
    );
    assert!(matches!(
        store
            .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
            .unwrap(),
        Some(Stage::Staged { .. })
    ));
    assert!(
        !store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(false),
                &mut installer
            )
            .unwrap()
    );
    assert_eq!(installer.install_calls, 1);
    installer.install_failure = false;
    assert!(
        store
            .request_install(
                &mut Gate::open(),
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .unwrap()
    );
    assert_eq!(installer.install_calls, 2);
}
#[test]
fn expired_policy_blocks_new_stage_and_install_but_not_installer_recovery() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    let mut feed = LocalFeed::new([]);
    assert!(
        store
            .download_and_stage(
                &f.offer(),
                &mut Gate::open(),
                NOW + 900,
                &mut feed,
                &f.verifier(),
                &mut installer
            )
            .is_err()
    );
    assert_eq!(feed.calls, 0);
    stage(&f, &store, &mut installer).unwrap();
    assert!(
        store
            .request_install(
                &mut Gate::open(),
                NOW + 900,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .is_err()
    );
    store
        .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW + 900)
        .unwrap();
    assert_eq!(installer.recover_calls, 1);
    assert_eq!(installer.install_calls, 0);
}
#[test]
fn production_client_uses_only_the_configured_key_and_validates_disabled_schema() {
    let mut f = Fixture::new();
    f.config.public_key = Some("invalid key".into());
    assert!(pm_distribution::UpdateClient::new(f.config.clone()).is_err());
    f.config.enabled = false;
    f.config.version = 9;
    assert!(pm_distribution::UpdateClient::new(f.config).is_err());
    let config: UpdateConfig = serde_json::from_str(include_str!("../update-config.json")).unwrap();
    let mut client = pm_distribution::UpdateClient::new(config).unwrap();
    assert!(matches!(
        client
            .check(ready(), "1.2.0", Architecture::Arm64, NOW)
            .unwrap(),
        Check::Gated
    ));
    assert!(client.verifier().is_err());
}

struct ExpiringGate;
impl Readiness for ExpiringGate {
    fn current(&mut self) -> Result<Eligibility> {
        Ok(ready())
    }
    fn now_unix(&mut self) -> Result<u64> {
        Ok(NOW + 900)
    }
}
#[test]
fn policy_expiring_during_network_or_quit_does_not_stage_or_install() {
    let f = Fixture::new();
    let (_temp, store) = store();
    let mut installer = FakeInstaller::default();
    assert!(
        store
            .download_and_stage(
                &f.offer(),
                &mut ExpiringGate,
                NOW,
                &mut LocalFeed::new([f.archive.clone()]),
                &f.verifier(),
                &mut installer
            )
            .is_err()
    );
    assert_eq!(installer.stage_calls, 0);
    store
        .recover(&f.verifier(), &mut installer, &mut Gate::open(), NOW)
        .unwrap();
    assert!(
        store
            .request_install(
                &mut ExpiringGate,
                NOW,
                &f.verifier(),
                &mut Barrier(true),
                &mut installer
            )
            .is_err()
    );
    assert_eq!(installer.install_calls, 0);
}

#[derive(Default)]
struct AsyncBinding {
    calls: Vec<&'static str>,
}
impl pm_update_policy::sparkle::Binding for AsyncBinding {
    fn check(&mut self, _: &Release) -> Result<()> {
        self.calls.push("check");
        Ok(())
    }
    fn download(&mut self) -> Result<()> {
        self.calls.push("download");
        Ok(())
    }
    fn install(&mut self) -> Result<()> {
        self.calls.push("install");
        Ok(())
    }
    fn cancel(&mut self) -> Result<()> {
        self.calls.push("cancel");
        Ok(())
    }
}
struct Guard(bool);
impl pm_update_policy::sparkle::TerminationGuard for Guard {
    fn verify_candidate(&mut self, _: &Path, _: &Release) -> Result<bool> {
        Ok(true)
    }
    fn protect(&mut self) -> Result<()> {
        Ok(())
    }
    fn protected(&self) -> bool {
        self.0
    }
}
struct NoCandidate;
impl pm_update_policy::sparkle::TerminationGuard for NoCandidate {
    fn verify_candidate(&mut self, _: &Path, _: &Release) -> Result<bool> {
        Ok(false)
    }
    fn protect(&mut self) -> Result<()> {
        Ok(())
    }
    fn protected(&self) -> bool {
        true
    }
}
struct SignedProof(bool);
impl pm_update_policy::sparkle::InstalledProof for SignedProof {
    fn matches(&mut self, _: &Release) -> Result<bool> {
        Ok(self.0)
    }
}

#[test]
fn asynchronous_sparkle_requires_protected_quit_and_signed_recovery_not_cached_tickets() {
    use pm_update_policy::sparkle::{Event, Phase, Session, recover};
    let f = Fixture::new();
    let archive_key = pm_update_policy::sparkle::ArchiveKey::from_base64(
        &STANDARD.encode(f.key.verifying_key().as_bytes()),
    )
    .unwrap();
    let offer = f.offer();
    let t = TempDir::new().unwrap();
    let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
    let mut gate = Gate::open();
    let mut binding = AsyncBinding::default();
    let file = store
        .download_verified(
            &offer,
            &mut gate,
            NOW,
            &mut LocalFeed::new([f.archive.clone()]),
            &f.verifier(),
            u64::MAX,
        )
        .unwrap();
    assert!(
        Session::begin(
            &store,
            &offer,
            pm_update_policy::sparkle::ReferenceArchive {
                file: file.clone(),
                key: &archive_key
            },
            Guard(false),
            &mut gate,
            &f.verifier(),
            &mut binding
        )
        .is_err()
    );
    assert!(
        Session::begin(
            &store,
            &offer,
            pm_update_policy::sparkle::ReferenceArchive {
                file: file.clone(),
                key: &archive_key
            },
            NoCandidate,
            &mut gate,
            &f.verifier(),
            &mut binding
        )
        .is_err()
    );
    assert!(binding.calls.is_empty());
    let mut session = Session::begin(
        &store,
        &offer,
        pm_update_policy::sparkle::ReferenceArchive {
            file,
            key: &archive_key,
        },
        Guard(true),
        &mut gate,
        &f.verifier(),
        &mut binding,
    )
    .unwrap();
    assert_eq!(session.phase(), Phase::Checking);
    session
        .event(Event::Found, &mut gate, &f.verifier(), &mut binding)
        .unwrap();
    session
        .event(Event::Extracting, &mut gate, &f.verifier(), &mut binding)
        .unwrap();
    assert!(
        !session
            .allow_termination(&mut gate, &f.verifier(), &mut Barrier(true), &mut binding)
            .unwrap()
    );
    session
        .event(Event::Ready, &mut gate, &f.verifier(), &mut binding)
        .unwrap();
    assert!(
        !session
            .allow_termination(&mut gate, &f.verifier(), &mut Barrier(false), &mut binding)
            .unwrap()
    );
    assert_eq!(binding.calls, ["check", "download"]);
    assert!(
        session
            .allow_termination(&mut gate, &f.verifier(), &mut Barrier(true), &mut binding)
            .unwrap()
    );
    session
        .event(Event::Installed, &mut gate, &f.verifier(), &mut binding)
        .unwrap();
    assert!(matches!(
        store.status().unwrap().unwrap().1,
        Stage::Sparkle {
            phase: Phase::InstallRequested,
            ..
        }
    ));
    drop(session);
    drop(store);
    let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
    assert_eq!(
        recover(&store, &offer, &f.verifier(), &mut SignedProof(false)).unwrap(),
        Recovery::Pending
    );
    assert_eq!(
        recover(&store, &offer, &f.verifier(), &mut SignedProof(true)).unwrap(),
        Recovery::Installed
    );
    assert!(matches!(
        store.status().unwrap().unwrap().1,
        Stage::Completed
    ));
}

#[test]
fn async_sparkle_rejects_stale_generations_and_tampering_before_callbacks() {
    use pm_update_policy::sparkle::{Event, Phase, Session};
    for tamper in [false, true] {
        let f = Fixture::new();
        let archive_key = pm_update_policy::sparkle::ArchiveKey::from_base64(
            &STANDARD.encode(f.key.verifying_key().as_bytes()),
        )
        .unwrap();
        let offer = f.offer();
        let t = TempDir::new().unwrap();
        let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
        let mut gate = Gate::open();
        let mut binding = AsyncBinding::default();
        let file = store
            .download_verified(
                &offer,
                &mut gate,
                NOW,
                &mut LocalFeed::new([f.archive.clone()]),
                &f.verifier(),
                u64::MAX,
            )
            .unwrap();
        let mut session = Session::begin(
            &store,
            &offer,
            pm_update_policy::sparkle::ReferenceArchive {
                file: file.clone(),
                key: &archive_key,
            },
            Guard(true),
            &mut gate,
            &f.verifier(),
            &mut binding,
        )
        .unwrap();
        if tamper {
            fs::write(store.root().join(file), b"changed").unwrap();
        } else {
            let mut changed = ready();
            changed.source_generation += 1;
            gate.0 = [changed].into();
        }
        assert!(
            session
                .event(Event::Found, &mut gate, &f.verifier(), &mut binding)
                .is_err()
        );
        assert!(!binding.calls.contains(&"download"));
        if !tamper {
            assert_eq!(session.phase(), Phase::Aborted);
            assert!(binding.calls.contains(&"cancel"));
        }
    }
}

#[test]
fn async_policy_hash_cannot_bind_different_archive_bytes_to_a_valid_sparkle_signature() {
    use pm_update_policy::sparkle::{ArchiveKey, ReferenceArchive, Session};
    let mut f = Fixture::new();
    // Valid independent policy signature and SHA-256 for bytes A, but Sparkle's
    // valid archive signature still covers bytes B. This MUST fail before check.
    f.archive = b"different synthetic archive bytes".to_vec();
    f.release.size = f.archive.len() as u64;
    f.release.sha256 = format!("{:x}", Sha256::digest(&f.archive));
    let offer = f.offer();
    let t = TempDir::new().unwrap();
    let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
    let mut gate = Gate::open();
    let mut binding = AsyncBinding::default();
    let file = store
        .download_verified(
            &offer,
            &mut gate,
            NOW,
            &mut LocalFeed::new([f.archive.clone()]),
            &f.verifier(),
            u64::MAX,
        )
        .unwrap();
    let key = ArchiveKey::from_base64(&STANDARD.encode(f.key.verifying_key().as_bytes())).unwrap();
    assert!(
        Session::begin(
            &store,
            &offer,
            ReferenceArchive { file, key: &key },
            Guard(true),
            &mut gate,
            &f.verifier(),
            &mut binding
        )
        .is_err()
    );
    assert!(binding.calls.is_empty());
}

#[test]
fn asynchronous_cancel_keeps_termination_veto_until_native_confirmation() {
    use pm_update_policy::sparkle::{ArchiveKey, Event, Phase, ReferenceArchive, Session};
    let f = Fixture::new();
    let offer = f.offer();
    let t = TempDir::new().unwrap();
    let store = StagingStore::open(&t.path().canonicalize().unwrap()).unwrap();
    let mut gate = Gate::open();
    let mut binding = AsyncBinding::default();
    let file = store
        .download_verified(
            &offer,
            &mut gate,
            NOW,
            &mut LocalFeed::new([f.archive.clone()]),
            &f.verifier(),
            u64::MAX,
        )
        .unwrap();
    let key = ArchiveKey::from_base64(&STANDARD.encode(f.key.verifying_key().as_bytes())).unwrap();
    let mut session = Session::begin(
        &store,
        &offer,
        ReferenceArchive { file, key: &key },
        Guard(true),
        &mut gate,
        &f.verifier(),
        &mut binding,
    )
    .unwrap();
    session.cancel(&mut binding).unwrap();
    assert_eq!(session.phase(), Phase::Aborted);
    assert!(
        !session
            .allow_termination(&mut gate, &f.verifier(), &mut Barrier(true), &mut binding)
            .unwrap()
    );
    session
        .event(Event::Cancelled, &mut gate, &f.verifier(), &mut binding)
        .unwrap();
    assert_eq!(session.phase(), Phase::Cancelled);
    assert!(
        session
            .allow_termination(&mut gate, &f.verifier(), &mut Barrier(true), &mut binding)
            .unwrap()
    );
    assert!(!binding.calls.contains(&"install"));
}

#[test]
fn sparkle_build_number_is_independent_of_display_semver_and_legacy_policies_stay_readable() {
    let mut f = Fixture::new();
    assert_eq!(f.offer().release().sparkle_version(), "2.0.0");
    f.release.sparkle_version = Some("42.3".into());
    let offer = f.offer();
    assert_eq!(offer.release().version, "2.0.0");
    assert_eq!(offer.release().sparkle_version(), "42.3");
    for build in ["", "0", "01", "2.0.0-beta.1", "1.2.3.4", "1..2"] {
        f.release.sparkle_version = Some(build.into());
        let mut feed = LocalFeed::new([f.policy()]);
        assert!(
            check(
                &f.config,
                ready(),
                "1.2.0",
                Architecture::Arm64,
                NOW,
                &mut feed,
                &f.verifier()
            )
            .is_err(),
            "{build}"
        );
    }
}
