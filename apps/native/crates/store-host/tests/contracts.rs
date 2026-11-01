//! Synthetic public fixtures only. Signing seed below is deliberately public
//! test material and never used as a distribution/publisher credential.
use ed25519_dalek::{Signer, SigningKey};
use flate2::{Compression, write::GzEncoder};
use pm_store_host::{
    Error, Result,
    artifact::*,
    channel::*,
    identity::*,
    install::*,
    permission::{self, *},
    view::*,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Cursor};

fn hash(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn id(s: &str) -> ExtensionId {
    ExtensionId::new(s).unwrap()
}
fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}
fn trust() -> TrustStore {
    let mut t = TrustStore::default();
    t.trust(
        "publisher".into(),
        "test-key".into(),
        signing_key().verifying_key(),
    )
    .unwrap();
    t
}
fn pack(entries: &[(String, Vec<u8>, u8)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (path, data, kind) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_mode(0o644);
        h.set_size(data.len() as u64);
        h.set_entry_type(tar::EntryType::new(*kind));
        let raw = h.as_mut_bytes();
        raw[..100].fill(0);
        raw[..path.len()].copy_from_slice(path.as_bytes());
        if *kind == b'2' {
            h.set_link_name("target").unwrap();
        }
        h.set_cksum();
        tar.append(&h, Cursor::new(data)).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}
fn sign(s: ReleaseStatement) -> SignedRelease {
    let signature = signing_key()
        .sign(&s.signing_bytes().unwrap())
        .to_bytes()
        .to_vec();
    SignedRelease {
        statement: s,
        signature,
    }
}
fn from_entries(entries: &[(String, Vec<u8>, u8)]) -> (SignedRelease, Vec<u8>) {
    let bytes = pack(entries);
    let files = entries
        .iter()
        .map(|(p, d, _)| {
            (
                p.clone(),
                FileSeal {
                    sha256: hash(d),
                    bytes: d.len() as u64,
                },
            )
        })
        .collect();
    (
        sign(ReleaseStatement {
            format: 1,
            key_id: "test-key".into(),
            publisher: "publisher".into(),
            extension: id("sample-ext"),
            release_id: "release-1".into(),
            archive_sha256: hash(&bytes),
            files,
        }),
        bytes,
    )
}
fn entries(version: &str, perms: &[Permission]) -> Vec<(String, Vec<u8>, u8)> {
    vec![
        (
            "manifest.json".into(),
            serde_json::to_vec(&json!({"id":"sample-ext", "version":version,
        "apiVersion":3, "permissions":perms, "contributes":{"panels":[]}, "future":{"opaque":42}}))
            .unwrap(),
            b'0',
        ),
        ("index.js".into(), b"export default () => 42".to_vec(), b'0'),
    ]
}
fn fixture(version: &str, perms: &[Permission]) -> (SignedRelease, Vec<u8>) {
    from_entries(&entries(version, perms))
}
fn verified(version: &str, perms: &[Permission]) -> VerifiedRelease {
    let (s, b) = fixture(version, perms);
    verify(&s, &b, &trust(), Limits::default()).unwrap()
}
fn installed(perms: &[Permission]) -> (MemoryKernel, tempfile::TempDir, FileArtifacts) {
    let mut k = MemoryKernel::default();
    let dir = tempfile::tempdir().unwrap();
    let mut a = FileArtifacts::new(dir.path()).unwrap();
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("1.0.0", perms),
        None,
    )
    .unwrap();
    (k, dir, a)
}
fn state(k: &impl StoreKernel) -> ExtensionState {
    k.load(&id("sample-ext")).unwrap().unwrap()
}
fn grant(k: &mut impl StoreKernel, p: Permission) {
    let Admission::Prompt(prompt) = request(k, &id("sample-ext"), p).unwrap() else {
        panic!("expected prompt");
    };
    respond(k, &prompt, Decision::Allow).unwrap();
}

#[test]
fn identity_stable_and_profile_scoped() {
    let a = DataStoreIdentity::new("profile-a", id("sample-ext")).unwrap();
    assert_eq!(
        a,
        DataStoreIdentity::new("profile-a", id("sample-ext")).unwrap()
    );
    assert_ne!(
        a.key,
        DataStoreIdentity::new("profile-b", id("sample-ext"))
            .unwrap()
            .key
    );
    assert_ne!(
        a.key,
        DataStoreIdentity::new("profile-a", id("other-ext"))
            .unwrap()
            .key
    );
    assert_eq!(a.kernel_storage_key(), "ext.sample-ext");
    assert_eq!(a.webkit_uuid().unwrap().len(), 36);
}
#[test]
fn identity_rejects_unsafe_ids_including_deserialization() {
    for s in [
        "a",
        "../escape",
        "A-b",
        "abc_def",
        "a/b",
        "a.b",
        "a\0b",
        "-aa",
        "ünicode",
    ] {
        assert!(ExtensionId::new(s).is_err());
        assert!(serde_json::from_value::<ExtensionId>(json!(s)).is_err());
    }
    assert!(ExtensionId::new("a".repeat(65)).is_err());
    assert!(ExtensionId::new("a".repeat(64)).is_ok());
    assert!(DataStoreIdentity::new("", id("abc")).is_err());
}
#[test]
fn legacy_reader_uses_shipped_origin_vectors() {
    assert_eq!(
        LegacyImportPlan::v1_2_0(id("abc")).origin,
        "app://x-abc-xj4bnp4pahh6uqkbidpf3lrceo"
    );
    let p = LegacyImportPlan::v1_2_0(id("sandboxed-ext"));
    assert_eq!(p.origin, "app://x-sandboxed-ext-vkyvbqsl7rchbesnaeceq5ga5t");
    assert_eq!(p.json_storage_key, "ext.sandboxed-ext");
    assert!(p.snapshot_shared_profile && p.preserve_unknown_records);
    assert!(!p.import_cookies_into_renderer);
}
#[test]
fn legacy_slug_normalizes_repeated_hyphens() {
    assert!(
        LegacyImportPlan::v1_2_0(id("a---long--id"))
            .origin
            .starts_with("app://x-a-long-id-")
    );
    assert!(
        LegacyImportPlan::v1_2_0(id("abcdefghijklmnopqrs-tuvw"))
            .origin
            .starts_with("app://x-abcdefghijklmnopqrs-")
    );
}
#[test]
fn webkit_uuid_rejects_corrupt_keys_without_panicking() {
    let mut d = DataStoreIdentity::new("p", id("abc")).unwrap();
    for bad in ["x".repeat(64), "a".repeat(63), "é".repeat(32)] {
        d.key = bad;
        assert!(d.webkit_uuid().is_err());
    }
}
#[test]
fn verifies_signed_bytes_inventory_and_opaque_manifest() {
    let v = verified("1.0.0", &[Permission::Assets]);
    assert_eq!(v.file("index.js").unwrap(), b"export default () => 42");
    assert_eq!(v.manifest().extra["future"], json!({"opaque":42}));
    assert!(v.manifest().permissions.contains(&Permission::Assets));
}
#[test]
fn all_legacy_api_versions_are_admitted() {
    for api in 1..=4 {
        let mut e = entries("1.0.0", &[]);
        let mut m: Value = serde_json::from_slice(&e[0].1).unwrap();
        m["apiVersion"] = json!(api);
        e[0].1 = serde_json::to_vec(&m).unwrap();
        let (s, b) = from_entries(&e);
        assert_eq!(
            verify(&s, &b, &trust(), Limits::default())
                .unwrap()
                .manifest()
                .api_version,
            api
        );
    }
}
#[test]
fn rejects_tampered_archive() {
    let (s, mut b) = fixture("1.0.0", &[]);
    b[12] ^= 1;
    assert_eq!(
        verify(&s, &b, &trust(), Limits::default()).unwrap_err(),
        Error::Integrity("archive hash")
    );
}
#[test]
fn rejects_bad_signature_and_modified_metadata() {
    let (mut s, b) = fixture("1.0.0", &[]);
    s.statement.release_id = "attacker".into();
    assert!(verify(&s, &b, &trust(), Limits::default()).is_err());
    s.signature.clear();
    assert!(verify(&s, &b, &trust(), Limits::default()).is_err());
}
#[test]
fn unknown_revoked_and_other_publisher_keys_fail() {
    let (s, b) = fixture("1.0.0", &[]);
    let mut t = trust();
    t.revoke("publisher", "test-key");
    assert!(verify(&s, &b, &t, Limits::default()).is_err());
    t.trust(
        "other".into(),
        "test-key".into(),
        signing_key().verifying_key(),
    )
    .unwrap();
    assert!(verify(&s, &b, &t, Limits::default()).is_err());
}
#[test]
fn refuses_duplicate_trust_key_replacement() {
    let mut t = trust();
    assert!(
        t.trust(
            "publisher".into(),
            "test-key".into(),
            SigningKey::from_bytes(&[8; 32]).verifying_key()
        )
        .is_err()
    );
}
#[test]
fn rejects_wrong_file_hash_and_length() {
    let (s, b) = fixture("1.0.0", &[]);
    for wrong_size in [true, false] {
        let mut s = s.statement.clone();
        let seal = s.files.get_mut("index.js").unwrap();
        if wrong_size {
            seal.bytes += 1;
        } else {
            seal.sha256 = "a".repeat(64);
        }
        assert!(verify(&sign(s), &b, &trust(), Limits::default()).is_err());
    }
}
#[test]
fn rejects_missing_and_unlisted_files() {
    let (s, b) = fixture("1.0.0", &[]);
    let mut stmt = s.statement.clone();
    stmt.files.remove("index.js");
    assert!(verify(&sign(stmt), &b, &trust(), Limits::default()).is_err());
    let mut stmt = s.statement;
    stmt.files.insert(
        "missing.js".into(),
        FileSeal {
            sha256: hash(b""),
            bytes: 0,
        },
    );
    assert!(verify(&sign(stmt), &b, &trust(), Limits::default()).is_err());
}
#[test]
fn rejects_duplicate_archive_entries() {
    let mut e = entries("1.0.0", &[]);
    e.push(e[1].clone());
    let (s, b) = from_entries(&e);
    assert!(verify(&s, &b, &trust(), Limits::default()).is_err());
}
#[test]
fn rejects_traversal_hidden_absolute_and_windows_paths() {
    for path in [
        "../bad",
        "/absolute",
        "dir/../bad",
        "dir//bad",
        "dir/.hidden",
        "dir\\bad",
        "C:bad",
        "dir/line\nbreak",
    ] {
        let mut e = entries("1.0.0", &[]);
        e[1].0 = path.into();
        let (s, b) = from_entries(&e);
        assert!(
            verify(&s, &b, &trust(), Limits::default()).is_err(),
            "{path}"
        );
    }
}
#[test]
fn rejects_links_directories_and_special_entries() {
    for kind in [b'1', b'2', b'3', b'5', b'6'] {
        let mut e = entries("1.0.0", &[]);
        e[1].2 = kind;
        e[1].1.clear();
        let (s, b) = from_entries(&e);
        assert!(verify(&s, &b, &trust(), Limits::default()).is_err());
    }
}
#[test]
fn rejects_wrong_manifest_identity_and_invalid_permissions() {
    for m in [
        json!({"id":"other-ext","version":"1.0.0"}),
        json!({"id":"sample-ext","version":"1","permissions":["made-up"]}),
        json!({"id":"sample-ext","version":"1","apiVersion":99}),
        json!({"id":"sample-ext","version":""}),
    ] {
        let mut e = entries("1.0.0", &[]);
        e[0].1 = serde_json::to_vec(&m).unwrap();
        let (s, b) = from_entries(&e);
        assert!(verify(&s, &b, &trust(), Limits::default()).is_err());
    }
}
#[test]
fn rejects_missing_manifest_and_corrupt_json() {
    for e in [
        vec![("index.js".into(), vec![], b'0')],
        vec![("manifest.json".into(), b"{".to_vec(), b'0')],
    ] {
        let (s, b) = from_entries(&e);
        assert!(verify(&s, &b, &trust(), Limits::default()).is_err());
    }
}
#[test]
fn enforces_archive_expansion_inventory_and_manifest_budgets() {
    let (s, b) = fixture("1.0.0", &[]);
    for limits in [
        Limits {
            archive_bytes: 1,
            ..Limits::default()
        },
        Limits {
            expanded_bytes: 1,
            ..Limits::default()
        },
        Limits {
            files: 1,
            ..Limits::default()
        },
        Limits {
            manifest_bytes: 1,
            ..Limits::default()
        },
    ] {
        assert!(verify(&s, &b, &trust(), limits).is_err());
    }
}
#[test]
fn rejects_signed_decompression_bomb() {
    let (s, _) = fixture("1.0.0", &[]);
    let b = pack(&[("index.js".into(), vec![0; 100_000], b'0')]);
    let mut stmt = s.statement;
    stmt.archive_sha256 = hash(&b);
    let limits = Limits {
        expanded_bytes: 1000,
        files: 2,
        ..Limits::default()
    };
    assert_eq!(
        verify(&sign(stmt), &b, &trust(), limits).unwrap_err(),
        Error::Invalid("decompression budget")
    );
}
#[test]
fn gzip_checksum_corruption_is_not_ignored_after_tar_end() {
    let (s, mut b) = fixture("1.0.0", &[]);
    let n = b.len();
    b[n - 8] ^= 1;
    let mut stmt = s.statement;
    stmt.archive_sha256 = hash(&b);
    assert!(verify(&sign(stmt), &b, &trust(), Limits::default()).is_err());
}
#[test]
fn immutable_artifacts_are_verified_and_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let v = verified("1.0.0", &[]);
    let digest = v.statement().archive_sha256.clone();
    {
        let mut a = FileArtifacts::new(dir.path()).unwrap();
        a.preserve(&v).unwrap();
        a.preserve(&v).unwrap();
    }
    let a = FileArtifacts::new(dir.path()).unwrap();
    assert_eq!(hash(&a.read(&digest, 1 << 20).unwrap()), digest);
    assert_eq!(
        fs::read_dir(dir.path().join(".staging")).unwrap().count(),
        2
    );
}
#[test]
fn corrupt_existing_artifact_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = FileArtifacts::new(dir.path()).unwrap();
    let v = verified("1.0.0", &[]);
    let path = a.object_path(&v.statement().archive_sha256).unwrap();
    fs::write(&path, b"corrupt").unwrap();
    assert!(a.preserve(&v).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"corrupt");
}
#[test]
fn artifact_reads_refuse_unsafe_hashes_and_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = FileArtifacts::new(dir.path()).unwrap();
    let v = verified("1.0.0", &[]);
    assert!(a.object_path("../secret").is_err());
    a.preserve(&v).unwrap();
    assert!(a.read(&v.statement().archive_sha256, 1).is_err());
}
#[test]
fn install_update_uninstall_and_rollback_keep_identity_and_all_versions() {
    let (mut k, _dir, mut a) = installed(&[]);
    let original = state(&k).data_store;
    assert_eq!(
        install(
            &mut k,
            &mut a,
            "test-profile",
            &verified("2.0.0", &[]),
            Some(1)
        )
        .unwrap(),
        InstallOutcome::Activated { revision: 2 }
    );
    assert_eq!(state(&k).archives.len(), 1);
    assert_eq!(state(&k).archives[0].release.manifest.version, "1.0.0");
    assert_eq!(uninstall(&mut k, &id("sample-ext"), 2).unwrap(), 3);
    assert_eq!(state(&k).data_status, DataStoreStatus::Archived);
    assert!(state(&k).active.is_none());
    assert_eq!(restore(&mut k, &mut a, &id("sample-ext"), 3, 2).unwrap(), 4);
    let s = state(&k);
    assert_eq!(s.data_store, original);
    assert_eq!(s.archives.len(), 2);
    assert_eq!(s.active.unwrap().manifest.version, "1.0.0");
}
#[test]
fn failed_commit_leaves_active_and_preserves_candidate_bytes() {
    let (mut k, _dir, mut a) = installed(&[]);
    let before = state(&k);
    k.fail_next_commit = true;
    let v = verified("2.0.0", &[]);
    assert!(install(&mut k, &mut a, "test-profile", &v, Some(1)).is_err());
    assert_eq!(state(&k), before);
    assert!(a.read(&v.statement().archive_sha256, 1 << 20).is_ok());
}
#[test]
fn failed_artifact_write_leaves_kernel_unchanged() {
    struct Broken;
    impl ArtifactStore for Broken {
        fn preserve(&mut self, _: &VerifiedRelease) -> Result<()> {
            Err(Error::Io("disk full".into()))
        }
    }
    let (mut k, _dir, _a) = installed(&[]);
    let before = state(&k);
    assert!(
        install(
            &mut k,
            &mut Broken,
            "test-profile",
            &verified("2.0.0", &[]),
            Some(1)
        )
        .is_err()
    );
    assert_eq!(state(&k), before);
}
#[test]
fn stale_operations_cannot_overwrite_a_newer_install() {
    let (mut k, _dir, mut a) = installed(&[]);
    for expected in [None, Some(0), Some(2)] {
        assert_eq!(
            install(
                &mut k,
                &mut a,
                "test-profile",
                &verified("2.0.0", &[]),
                expected
            )
            .unwrap_err(),
            Error::Conflict
        );
    }
    assert_eq!(
        uninstall(&mut k, &id("sample-ext"), 0).unwrap_err(),
        Error::Conflict
    );
    assert!(restore(&mut k, &mut a, &id("sample-ext"), 1, 999).is_err());
    assert_eq!(state(&k).revision, 1);
}
#[test]
fn cannot_change_profile_or_publisher_through_update() {
    let (mut k, _dir, mut a) = installed(&[]);
    assert!(
        install(
            &mut k,
            &mut a,
            "wrong-profile",
            &verified("2.0.0", &[]),
            Some(1)
        )
        .is_err()
    );
    let (s, b) = fixture("2.0.0", &[]);
    let mut stmt = s.statement;
    stmt.publisher = "other".into();
    let mut t = trust();
    t.trust(
        "other".into(),
        "test-key".into(),
        signing_key().verifying_key(),
    )
    .unwrap();
    let v = verify(&sign(stmt), &b, &t, Limits::default()).unwrap();
    assert_eq!(
        install(&mut k, &mut a, "test-profile", &v, Some(1)).unwrap_err(),
        Error::Denied
    );
    uninstall(&mut k, &id("sample-ext"), 1).unwrap();
    assert_eq!(
        install(&mut k, &mut a, "test-profile", &v, Some(2)).unwrap_err(),
        Error::Denied
    );
}
#[test]
fn local_edits_keep_running_and_remote_candidate_stays_alongside() {
    let (mut k, _dir, mut a) = installed(&[]);
    let mut s = state(&k);
    s.revision = 2;
    s.active.as_mut().unwrap().local_overlay = Some("preserved-overlay-id".into());
    k.compare_exchange(&id("sample-ext"), Some(1), &s).unwrap();
    assert_eq!(
        install(
            &mut k,
            &mut a,
            "test-profile",
            &verified("2.0.0", &[]),
            Some(2)
        )
        .unwrap(),
        InstallOutcome::ConflictPreserved {
            revision: 3,
            archive: 3
        }
    );
    let s = state(&k);
    assert_eq!(s.active.unwrap().manifest.version, "1.0.0");
    assert_eq!(s.archives[0].reason, ArchiveReason::ConflictCandidate);
    restore(&mut k, &mut a, &id("sample-ext"), 3, 3).unwrap();
    assert!(
        state(&k)
            .archives
            .iter()
            .any(|a| a.release.local_overlay.as_deref() == Some("preserved-overlay-id"))
    );
}
#[test]
fn reinstall_reuses_archived_data_and_decisions() {
    let (mut k, _dir, mut a) = installed(&[Permission::Assets]);
    grant(&mut k, Permission::Assets);
    let before = state(&k);
    uninstall(&mut k, &id("sample-ext"), 2).unwrap();
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2.0.0", &[Permission::Assets]),
        Some(3),
    )
    .unwrap();
    assert_eq!(state(&k).data_store, before.data_store);
    assert_eq!(state(&k).permissions, before.permissions);
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Assets).unwrap(),
        Admission::Granted
    );
}
#[test]
fn rollback_never_drops_the_replaced_version() {
    let (mut k, _dir, mut a) = installed(&[]);
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2.0.0", &[]),
        Some(1),
    )
    .unwrap();
    restore(&mut k, &mut a, &id("sample-ext"), 2, 2).unwrap();
    assert_eq!(state(&k).archives.len(), 2);
    assert_eq!(state(&k).archives[1].release.manifest.version, "2.0.0");
    restore(&mut k, &mut a, &id("sample-ext"), 3, 3).unwrap();
    assert_eq!(state(&k).active.unwrap().manifest.version, "2.0.0");
}
#[test]
fn failed_uninstall_and_rollback_are_atomic() {
    let (mut k, _dir, mut a) = installed(&[]);
    k.fail_next_commit = true;
    assert!(uninstall(&mut k, &id("sample-ext"), 1).is_err());
    assert!(state(&k).active.is_some());
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2.0.0", &[]),
        Some(1),
    )
    .unwrap();
    let before = state(&k);
    k.fail_next_commit = true;
    assert!(restore(&mut k, &mut a, &id("sample-ext"), 2, 2).is_err());
    assert_eq!(state(&k), before);
}
#[test]
fn kernel_double_checks_revision_and_identity() {
    let (mut k, _dir, _a) = installed(&[]);
    let mut s = state(&k);
    s.revision = 2;
    s.data_store = DataStoreIdentity::new("p", id("other-ext")).unwrap();
    assert!(k.compare_exchange(&id("sample-ext"), Some(1), &s).is_err());
    assert_eq!(state(&k).revision, 1);
}

#[test]
fn undeclared_permissions_cannot_be_prompted_or_granted() {
    let (k, _dir, _a) = installed(&[]);
    for p in [
        Permission::Network,
        Permission::Assets,
        Permission::Clipboard,
        Permission::ProjectWrite,
        Permission::FullAccess,
    ] {
        assert_eq!(
            request(&k, &id("sample-ext"), p).unwrap(),
            Admission::Denied
        );
    }
}
#[test]
fn denied_decision_is_persisted_and_does_not_prompt_again() {
    let (mut k, _dir, _a) = installed(&[Permission::Network]);
    let Admission::Prompt(p) = request(&k, &id("sample-ext"), Permission::Network).unwrap() else {
        panic!()
    };
    respond(&mut k, &p, Decision::Deny).unwrap();
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Network).unwrap(),
        Admission::Denied
    );
    assert_eq!(
        respond(&mut k, &p, Decision::Allow).unwrap_err(),
        Error::Stale
    );
}
#[test]
fn prompt_is_bound_to_identity_release_permission_and_revision() {
    let (mut k, _dir, mut a) = installed(&[Permission::Network, Permission::Assets]);
    let Admission::Prompt(p) = request(&k, &id("sample-ext"), Permission::Network).unwrap() else {
        panic!()
    };
    let mut forged = p.clone();
    forged.permissions = vec![Permission::Assets];
    assert_eq!(
        respond(&mut k, &forged, Decision::Allow).unwrap_err(),
        Error::Stale
    );
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2.0.0", &[Permission::Network]),
        Some(1),
    )
    .unwrap();
    assert_eq!(
        respond(&mut k, &p, Decision::Allow).unwrap_err(),
        Error::Stale
    );
}
#[test]
fn failed_permission_commit_never_issues_grant() {
    let (mut k, _dir, _a) = installed(&[Permission::Assets]);
    let Admission::Prompt(p) = request(&k, &id("sample-ext"), Permission::Assets).unwrap() else {
        panic!()
    };
    k.fail_next_commit = true;
    assert!(respond(&mut k, &p, Decision::Allow).is_err());
    assert!(matches!(
        request(&k, &id("sample-ext"), Permission::Assets).unwrap(),
        Admission::Prompt(_)
    ));
}
#[test]
fn install_consent_groups_all_undecided_permissions_in_shipped_order() {
    let (mut k, _dir, _a) = installed(&PERMISSION_ORDER);
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    assert_eq!(prompt.permissions, PERMISSION_ORDER);
    assert!(prompt.requires_trusted_realm);
    for permission in PERMISSION_ORDER {
        assert_eq!(
            request(&k, &id("sample-ext"), permission).unwrap(),
            Admission::Prompt(prompt.clone())
        );
    }
    let decisions = prompt
        .permissions
        .iter()
        .map(|p| {
            (
                *p,
                if *p == Permission::FullAccess {
                    Decision::Deny
                } else {
                    Decision::Allow
                },
            )
        })
        .collect();
    respond_all(&mut k, &prompt, &decisions).unwrap();
    assert_eq!(state(&k).revision, prompt.revision + 1);
    assert_eq!(state(&k).permissions.len(), 5);
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::FullAccess).unwrap(),
        Admission::Denied
    );
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Network).unwrap(),
        Admission::Granted
    );
    assert_eq!(respond_all(&mut k, &prompt, &decisions), Err(Error::Stale));
}

#[test]
fn grouped_consent_is_atomic_and_requires_exact_response_coverage() {
    let (mut k, _dir, _a) = installed(&[Permission::Network, Permission::Assets]);
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    let before = state(&k);
    for decisions in [
        std::collections::BTreeMap::new(),
        [(Permission::Assets, Decision::Allow)].into(),
        [
            (Permission::Assets, Decision::Allow),
            (Permission::Clipboard, Decision::Allow),
        ]
        .into(),
        [
            (Permission::Assets, Decision::Allow),
            (Permission::Network, Decision::Allow),
            (Permission::Clipboard, Decision::Allow),
        ]
        .into(),
    ] {
        assert_eq!(
            respond_all(&mut k, &prompt, &decisions),
            Err(Error::Invalid("permission response coverage"))
        );
        assert_eq!(state(&k), before);
    }
    k.fail_next_commit = true;
    assert!(respond(&mut k, &prompt, Decision::Allow).is_err());
    assert_eq!(state(&k), before);
    respond(&mut k, &prompt, Decision::Allow).unwrap();
    assert_eq!(state(&k).revision, 2);
    assert_eq!(state(&k).permissions.len(), 2);
}

#[test]
fn update_prompts_only_for_new_undecided_permissions_and_preserves_denials() {
    let (mut k, _dir, mut a) = installed(&[Permission::Assets, Permission::Clipboard]);
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Assets).unwrap()
    else {
        panic!()
    };
    respond_all(
        &mut k,
        &prompt,
        &[
            (Permission::Assets, Decision::Allow),
            (Permission::Clipboard, Decision::Deny),
        ]
        .into(),
    )
    .unwrap();
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified(
            "2",
            &[
                Permission::Assets,
                Permission::Clipboard,
                Permission::Network,
            ],
        ),
        Some(2),
    )
    .unwrap();
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    assert_eq!(prompt.permissions, vec![Permission::Network]);
    respond(&mut k, &prompt, Decision::Allow).unwrap();
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Clipboard).unwrap(),
        Admission::Denied
    );
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Assets).unwrap(),
        Admission::Granted
    );
}

fn display_release(
    reason: Option<&str>,
    icon: Option<&str>,
    verified_publisher: bool,
) -> VerifiedRelease {
    let mut files = entries("1", &[Permission::Network, Permission::Assets]);
    let mut manifest: Value = serde_json::from_slice(&files[0].1).unwrap();
    manifest["name"] = json!("Glass Blur");
    manifest["publisher_verified"] = json!(!verified_publisher);
    manifest["publisher_name"] = json!("Manifest cannot choose the publisher name");
    if let Some(reason) = reason {
        manifest["reason"] = json!(reason);
    }
    if let Some(icon) = icon {
        manifest["icon"] = json!(icon);
    }
    files[0].1 = serde_json::to_vec(&manifest).unwrap();
    files.push(("icon.png".into(), b"synthetic icon fixture".to_vec(), b'0'));
    let (signed, bytes) = from_entries(&files);
    let mut pins = TrustStore::default();
    pins.trust_with_identity(
        "publisher".into(),
        "test-key".into(),
        signing_key().verifying_key(),
        PublisherIdentity {
            name: "Example Publisher".into(),
            verified: verified_publisher,
        },
    )
    .unwrap();
    verify(&signed, &bytes, &pins, Limits::default()).unwrap()
}

#[test]
fn prompt_display_data_comes_from_signed_manifest_and_host_publisher_identity() {
    for verified_publisher in [false, true] {
        let release = display_release(
            Some("Load images for your project."),
            Some("icon.png"),
            verified_publisher,
        );
        let (mut k, _dir, mut a) = installed(&[]);
        install(&mut k, &mut a, "test-profile", &release, Some(1)).unwrap();
        let Admission::Prompt(prompt) =
            request(&k, &id("sample-ext"), Permission::Network).unwrap()
        else {
            panic!()
        };
        assert_eq!(prompt.extension_name, "Glass Blur");
        assert_eq!(prompt.publisher_name, "Example Publisher");
        assert_eq!(prompt.publisher_verified, verified_publisher);
        assert_eq!(prompt.icon.as_deref(), Some("icon.png"));
        assert_eq!(
            prompt.reason.as_deref(),
            Some("Load images for your project.")
        );
        assert_eq!(
            release.manifest().extra["publisher_verified"],
            json!(!verified_publisher)
        );
        let shown = prompt.presentation();
        assert_eq!(shown.icon_symbol, None);
        if verified_publisher {
            assert_eq!(shown.publisher_line, "From Example Publisher");
            assert_eq!(shown.publisher_badge, Some(VERIFIED_PUBLISHER_SYMBOL));
        } else {
            assert_eq!(shown.publisher_line, "From Example Publisher · Unverified");
            assert_eq!(shown.publisher_badge, None);
        }
        let json = serde_json::to_vec(&prompt).unwrap();
        assert_eq!(
            serde_json::from_slice::<PermissionRequest>(&json).unwrap(),
            *prompt
        );
    }
}

#[test]
fn optional_purpose_is_bounded_plain_text_without_truncating_authored_content() {
    let reason = "🖼".repeat(140);
    assert_eq!(
        display_release(Some(&reason), None, false)
            .manifest()
            .reason
            .as_deref(),
        Some(reason.as_str())
    );
    assert_eq!(display_release(None, None, false).manifest().reason, None);
    // Angle brackets/Markdown stay literal text, never a renderer instruction.
    let text = "Use <images> and **fonts**.";
    assert_eq!(
        display_release(Some(text), None, false)
            .manifest()
            .reason
            .as_deref(),
        Some(text)
    );
    for reason in [
        "x".repeat(141),
        "🖼".repeat(141),
        "line\nbreak".into(),
        "tab\there".into(),
        "null\0here".into(),
    ] {
        let mut files = entries("1", &[Permission::Network]);
        let mut manifest: Value = serde_json::from_slice(&files[0].1).unwrap();
        manifest["reason"] = json!(reason);
        files[0].1 = serde_json::to_vec(&manifest).unwrap();
        let (signed, bytes) = from_entries(&files);
        assert_eq!(
            verify(&signed, &bytes, &trust(), Limits::default()).unwrap_err(),
            Error::Invalid("permission reason")
        );
    }
}

#[test]
fn icon_must_be_an_exact_asset_in_the_verified_bundle() {
    for icon in [
        "missing.png",
        "../icon.png",
        "/icon.png",
        "https://example.com/icon.png",
        "icon.png/../index.js",
    ] {
        let mut files = entries("1", &[]);
        let mut manifest: Value = serde_json::from_slice(&files[0].1).unwrap();
        manifest["icon"] = json!(icon);
        files[0].1 = serde_json::to_vec(&manifest).unwrap();
        let (signed, bytes) = from_entries(&files);
        assert_eq!(
            verify(&signed, &bytes, &trust(), Limits::default()).unwrap_err(),
            Error::Integrity("manifest icon")
        );
    }
    assert!(
        display_release(None, Some("icon.png"), false)
            .file("icon.png")
            .is_some()
    );
}

#[test]
fn forged_display_data_and_presentation_class_cannot_authorize_consent() {
    let release = display_release(Some("Load images."), Some("icon.png"), true);
    let (mut k, _dir, mut a) = installed(&[]);
    install(&mut k, &mut a, "test-profile", &release, Some(1)).unwrap();
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    let before = state(&k);
    let mut changes = vec![prompt.clone(); 7];
    changes[0].extension_name = "Different name".into();
    changes[1].publisher_name = "Different publisher".into();
    changes[2].publisher_verified = false;
    changes[3].icon = None;
    changes[4].reason = None;
    changes[5].requires_trusted_realm = true;
    changes[6].id = "forged".into();
    for forged in changes {
        assert_eq!(respond(&mut k, &forged, Decision::Allow), Err(Error::Stale));
        assert_eq!(state(&k), before);
    }
}

#[test]
fn permission_copy_matches_every_verbatim_review_row_and_shipped_store_label() {
    let review = include_str!("../../../../../docs/native/design/reviews/na-cloud-storehost.md");
    assert_eq!(PERMISSION_COPY.map(|row| row.permission), PERMISSION_ORDER);
    for row in PERMISSION_COPY {
        let permission = serde_json::to_value(row.permission).unwrap();
        assert!(review.contains(&format!(
            "| `{}` | `{}` | {} | {} |",
            permission.as_str().unwrap(),
            row.symbol,
            row.name,
            row.detail
        )));
    }
    assert_eq!(
        PERMISSION_COPY.map(|row| row.store_label),
        [
            "Needs full access to Powermove",
            "Edits your project",
            "Imports files",
            "Uses the network",
            "Uses the clipboard",
        ]
    );
    assert_eq!(SETTINGS_ALLOW_LABEL, "Allow");
}

#[test]
fn isolated_permissions_specify_a_window_modal_sheet_and_accent_allow_default() {
    let release = display_release(None, None, false);
    let (mut k, _dir, mut a) = installed(&[]);
    install(&mut k, &mut a, "test-profile", &release, Some(1)).unwrap();
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    let sheet = prompt.presentation();
    assert_eq!(sheet.surface, PermissionSurface::WindowModalSheet);
    assert_eq!(sheet.title, "Allow “Glass Blur” To:");
    assert_eq!(sheet.informative_text, None);
    assert_eq!(sheet.rows, vec![PERMISSION_COPY[2], PERMISSION_COPY[3]]);
    assert_eq!(sheet.buttons.map(|b| b.label), ["Don’t Allow", "Allow"]);
    assert_eq!(
        sheet.buttons.map(|b| b.decision),
        [Decision::Deny, Decision::Allow]
    );
    assert_eq!(
        sheet.buttons[sheet.default_button].style,
        PermissionButtonStyle::Accent
    );
    assert_eq!(sheet.buttons[sheet.cancel_button].decision, Decision::Deny);
    assert_eq!(sheet.accessibility_identifier, prompt.id);
}

#[test]
fn full_access_specifies_a_warning_nsalert_with_exact_copy_and_cancel_default() {
    let mut files = entries("1", &PERMISSION_ORDER);
    let mut manifest: Value = serde_json::from_slice(&files[0].1).unwrap();
    manifest["name"] = json!("Glass Blur");
    files[0].1 = serde_json::to_vec(&manifest).unwrap();
    let (signed, bytes) = from_entries(&files);
    let release = verify(&signed, &bytes, &trust(), Limits::default()).unwrap();
    let (mut k, _dir, mut a) = installed(&[]);
    install(&mut k, &mut a, "test-profile", &release, Some(1)).unwrap();
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    assert!(prompt.requires_trusted_realm);
    let alert = prompt.presentation();
    assert_eq!(alert.surface, PermissionSurface::NsAlertWarning);
    assert_eq!(alert.title, "Give Glass Blur full access to Powermove?");
    assert_eq!(
        alert.informative_text,
        Some(
            "It will run with the same access as the app: your projects, files you open, the network, and other extensions’ values. Only trust extensions from people you know. You can revoke this from the Library."
        )
    );
    assert_eq!(alert.rows, PERMISSION_COPY);
    assert_eq!(alert.buttons.map(|b| b.label), ["Trust", "Cancel"]);
    assert_eq!(
        alert.buttons.map(|b| b.style),
        [PermissionButtonStyle::System; 2]
    );
    assert_eq!(alert.default_button, 1);
    assert_eq!(alert.cancel_button, 1);
    respond(
        &mut k,
        &prompt,
        alert.buttons[alert.default_button].decision,
    )
    .unwrap();
    assert!(
        stored_decisions(&k, &id("sample-ext"))
            .unwrap()
            .iter()
            .all(|row| row.decision == Decision::Deny)
    );
}

#[test]
fn older_displayless_records_are_preserved_without_using_ids_as_labels() {
    for permission in [Permission::Network, Permission::FullAccess] {
        let (k, _dir, _a) = installed(&[permission]);
        let mut legacy = serde_json::to_value(state(&k)).unwrap();
        legacy["active"]
            .as_object_mut()
            .unwrap()
            .remove("publisher_identity");
        legacy["active"]["manifest"]
            .as_object_mut()
            .unwrap()
            .remove("name");
        let restored: ExtensionState = serde_json::from_value(legacy).unwrap();
        let active = restored.active.as_ref().unwrap();
        assert_eq!(active.publisher_identity, PublisherIdentity::default());
        assert!(active.manifest.name.is_empty());
        assert_eq!(active.manifest.extra["future"]["opaque"], json!(42));
        let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), permission).unwrap() else {
            panic!()
        };
        assert!(prompt.extension_name.is_empty());
        assert!(prompt.publisher_name.is_empty());
        assert!(!prompt.publisher_verified);
        assert_eq!(prompt.icon, None);
        let shown = prompt.presentation();
        let expected = if permission == Permission::FullAccess {
            NAMELESS_ALERT_TITLE
        } else {
            NAMELESS_SHEET_TITLE
        };
        assert_eq!(shown.title, expected);
        assert_eq!(shown.publisher_line, "From an unidentified developer");
        assert_eq!(shown.publisher_badge, None);
        assert_eq!(shown.icon_symbol, Some("puzzlepiece.extension"));
        for label in [&shown.title, &shown.publisher_line] {
            assert!(!label.contains("sample-ext"), "{label}");
        }
    }
}

#[test]
fn settings_inspection_retains_denials_and_marks_removed_or_old_publisher_decisions_inactive() {
    let (mut k, _dir, mut a) = installed(&[Permission::Assets, Permission::Network]);
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    respond_all(
        &mut k,
        &prompt,
        &[
            (Permission::Assets, Decision::Allow),
            (Permission::Network, Decision::Deny),
        ]
        .into(),
    )
    .unwrap();
    let rows = stored_decisions(&k, &id("sample-ext")).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (r.permission, r.decision, r.active))
            .collect::<Vec<_>>(),
        vec![
            (Permission::Assets, Decision::Allow, true),
            (Permission::Network, Decision::Deny, true),
        ]
    );
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2", &[Permission::Network, Permission::Clipboard]),
        Some(2),
    )
    .unwrap();
    let mut state = state(&k);
    state.revision += 1;
    state.permissions.insert(
        Permission::Clipboard,
        PermissionDecision {
            publisher: "old-publisher".into(),
            decision: Decision::Allow,
        },
    );
    k.compare_exchange(&id("sample-ext"), Some(3), &state)
        .unwrap();
    let rows = stored_decisions(&k, &id("sample-ext")).unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| (r.permission, r.active))
            .collect::<Vec<_>>(),
        vec![
            (Permission::Assets, false),
            (Permission::Network, true),
            (Permission::Clipboard, false)
        ]
    );
    permission::revoke(&mut k, &id("sample-ext"), 4, Permission::Network).unwrap();
    assert_eq!(
        stored_decisions(&k, &id("sample-ext")).unwrap()[1].decision,
        Decision::Deny
    );
    uninstall(&mut k, &id("sample-ext"), 5).unwrap();
    assert!(
        stored_decisions(&k, &id("sample-ext"))
            .unwrap()
            .iter()
            .all(|r| !r.active)
    );
    assert_eq!(stored_decisions(&k, &id("sample-ext")).unwrap().len(), 3);
}

#[test]
fn stale_responses_cover_uninstall_missing_install_and_cas_races() {
    struct CasRace(MemoryKernel);
    impl StoreKernel for CasRace {
        fn load(&self, id: &ExtensionId) -> Result<Option<ExtensionState>> {
            self.0.load(id)
        }
        fn compare_exchange(
            &mut self,
            _: &ExtensionId,
            _: Option<u64>,
            _: &ExtensionState,
        ) -> Result<()> {
            Err(Error::Conflict)
        }
    }
    let (mut k, _dir, _a) = installed(&[Permission::Network]);
    let Admission::Prompt(prompt) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    let before = state(&k);
    let mut race = CasRace(k);
    assert_eq!(
        respond(&mut race, &prompt, Decision::Allow),
        Err(Error::Stale)
    );
    assert_eq!(state(&race), before);
    k = race.0;
    uninstall(&mut k, &id("sample-ext"), 1).unwrap();
    assert_eq!(respond(&mut k, &prompt, Decision::Allow), Err(Error::Stale));
    assert_eq!(
        respond(&mut MemoryKernel::default(), &prompt, Decision::Allow),
        Err(Error::Stale)
    );
}

#[test]
fn accessibility_identifier_is_stable_across_requests_and_changes_with_the_binding() {
    let (mut k, _dir, mut a) = installed(&[Permission::Network, Permission::Assets]);
    let Admission::Prompt(first) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    let Admission::Prompt(second) = request(&k, &id("sample-ext"), Permission::Assets).unwrap()
    else {
        panic!()
    };
    assert_eq!(first.id, second.id);
    assert_eq!(first.id.len(), 64);
    assert_eq!(
        first.presentation().accessibility_identifier,
        second.presentation().accessibility_identifier
    );
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2", &[Permission::Network, Permission::Assets]),
        Some(1),
    )
    .unwrap();
    let Admission::Prompt(updated) = request(&k, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    assert_ne!(first.id, updated.id);
    assert_eq!(updated.presentation().accessibility_identifier, updated.id);
}
#[test]
fn full_access_routes_to_trusted_realm_without_bypassing_store_policy() {
    let (mut k, _dir, _a) = installed(&[Permission::FullAccess]);
    let Admission::Prompt(p) = request(&k, &id("sample-ext"), Permission::FullAccess).unwrap()
    else {
        panic!()
    };
    assert!(p.requires_trusted_realm);
    respond(&mut k, &p, Decision::Allow).unwrap();
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::FullAccess).unwrap(),
        Admission::RequiresTrustedRealm
    );
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Network).unwrap(),
        Admission::Denied
    );
}
#[test]
fn method_policy_denies_unknown_methods_and_requires_every_capability() {
    let (mut k, _dir, _a) = installed(&[Permission::Network, Permission::Assets]);
    let mut p = MethodPolicy::default();
    p.register(
        "assets.importUrl".into(),
        BTreeSet::from([Permission::Assets, Permission::Network]),
    )
    .unwrap();
    assert!(
        p.authorize(&k, &id("sample-ext"), "assets.importUrl")
            .is_err()
    );
    grant(&mut k, Permission::Assets);
    p.authorize(&k, &id("sample-ext"), "assets.importUrl")
        .unwrap();
    permission::revoke(&mut k, &id("sample-ext"), 2, Permission::Network).unwrap();
    assert!(
        p.authorize(&k, &id("sample-ext"), "assets.importUrl")
            .is_err()
    );
    assert!(p.authorize(&k, &id("sample-ext"), "unknown").is_err());
    assert!(
        p.register("assets.importUrl".into(), BTreeSet::new())
            .is_err()
    );
    assert!(
        p.register(
            "everything".into(),
            BTreeSet::from([Permission::FullAccess])
        )
        .is_err()
    );
    uninstall(&mut k, &id("sample-ext"), 3).unwrap();
    assert!(
        p.authorize(&k, &id("sample-ext"), "assets.importUrl")
            .is_err()
    );
}
#[test]
fn revocation_survives_updates_and_closes_network_csp() {
    let (mut k, _dir, mut a) = installed(&[Permission::Network]);
    grant(&mut k, Permission::Network);
    permission::revoke(&mut k, &id("sample-ext"), 2, Permission::Network).unwrap();
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2.0.0", &[Permission::Network]),
        Some(3),
    )
    .unwrap();
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Network).unwrap(),
        Admission::Denied
    );
    assert!(
        !renderer_csp(&k, &id("sample-ext"), &"a".repeat(64))
            .unwrap()
            .contains("https:")
    );
    assert!(permission::revoke(&mut k, &id("sample-ext"), 2, Permission::Network).is_err());
}
#[test]
fn csp_only_grants_network_to_passive_resources_after_consent() {
    let (mut k, _dir, _a) = installed(&[Permission::Network]);
    let nonce = "a".repeat(64);
    let before = renderer_csp(&k, &id("sample-ext"), &nonce).unwrap();
    assert!(!before.contains("https:"));
    grant(&mut k, Permission::Network);
    let after = renderer_csp(&k, &id("sample-ext"), &nonce).unwrap();
    assert!(after.contains("https:"));
    for directive in after.split(';').map(str::trim) {
        if directive.starts_with("connect-src") || directive.starts_with("script-src") {
            assert!(!directive.contains("https:"));
        }
    }
    for forbidden in ["unsafe-eval", "http:", "wss:", "script-src 'self'"] {
        assert!(!after.contains(forbidden));
    }
    for required in [
        "worker-src 'none'",
        "frame-src 'none'",
        "object-src 'none'",
        "base-uri 'none'",
        "form-action 'none'",
    ] {
        assert!(after.contains(required));
    }
}
#[test]
fn csp_refuses_nonce_injection_and_uninstalled_identity() {
    let (mut k, _dir, _a) = installed(&[]);
    for nonce in ["x' https:", "", &"g".repeat(64), &"a".repeat(129)] {
        assert!(renderer_csp(&k, &id("sample-ext"), nonce).is_err());
    }
    uninstall(&mut k, &id("sample-ext"), 1).unwrap();
    assert!(renderer_csp(&k, &id("sample-ext"), &"a".repeat(64)).is_err());
}
#[test]
fn added_permission_needs_new_consent_and_removed_permission_is_inactive() {
    let (mut k, _dir, mut a) = installed(&[Permission::Assets]);
    grant(&mut k, Permission::Assets);
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2.0.0", &[Permission::Network]),
        Some(2),
    )
    .unwrap();
    assert_eq!(
        request(&k, &id("sample-ext"), Permission::Assets).unwrap(),
        Admission::Denied
    );
    assert!(matches!(
        request(&k, &id("sample-ext"), Permission::Network).unwrap(),
        Admission::Prompt(_)
    ));
    assert!(state(&k).permissions.contains_key(&Permission::Assets));
}

/// Durable trait double: exercises actual state/decision serialization across
/// process-like reopening, without claiming pm-kernel journal integration.
struct DiskKernel {
    root: std::path::PathBuf,
}
impl StoreKernel for DiskKernel {
    fn load(&self, id: &ExtensionId) -> Result<Option<ExtensionState>> {
        let path = self.root.join(format!("{}.json", id.as_str()));
        match fs::read(path) {
            Ok(b) => Ok(Some(serde_json::from_slice(&b).unwrap())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn compare_exchange(
        &mut self,
        id: &ExtensionId,
        expected: Option<u64>,
        next: &ExtensionState,
    ) -> Result<()> {
        if self.load(id)?.map(|s| s.revision) != expected {
            return Err(Error::Conflict);
        }
        let staged = self.root.join("next");
        fs::write(&staged, serde_json::to_vec(next).unwrap())?;
        fs::File::open(&staged)?.sync_all()?;
        fs::rename(staged, self.root.join(format!("{}.json", id.as_str())))?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(())
    }
}
#[test]
fn permission_and_archive_decisions_survive_durable_kernel_reopen() {
    let root = tempfile::tempdir().unwrap();
    let blobs = tempfile::tempdir().unwrap();
    let mut k = DiskKernel {
        root: root.path().into(),
    };
    let mut a = FileArtifacts::new(blobs.path()).unwrap();
    install(
        &mut k,
        &mut a,
        "profile",
        &verified("1.0.0", &[Permission::Assets]),
        None,
    )
    .unwrap();
    grant(&mut k, Permission::Assets);
    let identity = state(&k).data_store;
    drop(k);
    let mut reopened = DiskKernel {
        root: root.path().into(),
    };
    assert_eq!(
        request(&reopened, &id("sample-ext"), Permission::Assets).unwrap(),
        Admission::Granted
    );
    uninstall(&mut reopened, &id("sample-ext"), 2).unwrap();
    drop(reopened);
    let reopened = DiskKernel {
        root: root.path().into(),
    };
    let s = state(&reopened);
    assert!(s.active.is_none());
    assert_eq!(s.archives.len(), 1);
    assert_eq!(s.data_store, identity);
    assert!(!s.permissions.is_empty());
}

#[test]
fn grouped_display_metadata_and_mixed_decisions_survive_durable_reopen() {
    let root = tempfile::tempdir().unwrap();
    let blobs = tempfile::tempdir().unwrap();
    let mut kernel = DiskKernel {
        root: root.path().into(),
    };
    let mut artifacts = FileArtifacts::new(blobs.path()).unwrap();
    let release = display_release(Some("Load images."), Some("icon.png"), true);
    install(&mut kernel, &mut artifacts, "profile", &release, None).unwrap();
    let Admission::Prompt(prompt) =
        request(&kernel, &id("sample-ext"), Permission::Network).unwrap()
    else {
        panic!()
    };
    respond_all(
        &mut kernel,
        &prompt,
        &[
            (Permission::Assets, Decision::Allow),
            (Permission::Network, Decision::Deny),
        ]
        .into(),
    )
    .unwrap();
    drop(kernel);
    let reopened = DiskKernel {
        root: root.path().into(),
    };
    let persisted = state(&reopened);
    assert_eq!(persisted.revision, 2);
    let active = persisted.active.unwrap();
    assert_eq!(active.manifest.name, prompt.extension_name);
    assert_eq!(active.manifest.reason, prompt.reason);
    assert_eq!(active.manifest.icon, prompt.icon);
    assert_eq!(
        active.publisher_identity,
        PublisherIdentity {
            name: prompt.publisher_name,
            verified: true
        }
    );
    assert_eq!(
        stored_decisions(&reopened, &id("sample-ext"))
            .unwrap()
            .iter()
            .map(|row| (row.permission, row.decision, row.active))
            .collect::<Vec<_>>(),
        vec![
            (Permission::Assets, Decision::Allow, true),
            (Permission::Network, Decision::Deny, true)
        ]
    );
}

#[test]
fn rollback_uses_current_host_publisher_metadata_and_preserves_the_archive() {
    let (mut kernel, _dir, mut artifacts) = installed(&[]);
    let release = display_release(None, Some("icon.png"), true);
    install(
        &mut kernel,
        &mut artifacts,
        "test-profile",
        &release,
        Some(1),
    )
    .unwrap();
    let target = state(&kernel).active.unwrap();
    install(
        &mut kernel,
        &mut artifacts,
        "test-profile",
        &verified("2", &[Permission::Network]),
        Some(2),
    )
    .unwrap();
    let mut pins = TrustStore::default();
    pins.trust_with_identity(
        "publisher".into(),
        "test-key".into(),
        signing_key().verifying_key(),
        PublisherIdentity {
            name: "Current Publisher Name".into(),
            verified: false,
        },
    )
    .unwrap();
    let bytes = artifacts
        .read(&target.bundle, Limits::default().archive_bytes)
        .unwrap();
    let reverified = verify(&target.proof, &bytes, &pins, Limits::default()).unwrap();
    rollback(
        &mut kernel,
        &mut artifacts,
        &reverified,
        &id("sample-ext"),
        3,
        3,
    )
    .unwrap();
    let restored = state(&kernel);
    let active = restored.active.unwrap();
    assert_eq!(active.publisher_identity.name, "Current Publisher Name");
    assert!(!active.publisher_identity.verified);
    assert!(
        restored
            .archives
            .iter()
            .find(|archive| archive.id == 3)
            .unwrap()
            .release
            .publisher_identity
            .verified
    );
}

fn envelope(seq: u64) -> Envelope {
    Envelope {
        version: 1,
        extension: id("sample-ext"),
        view: "panel-1".into(),
        generation: 100,
        sequence: seq,
        from: Endpoint::View,
        to: Endpoint::Realm,
        message: Message::Input {
            node: 1,
            event: "click".into(),
            data: json!({"x":1}),
        },
    }
}
fn channel(limits: ChannelLimits) -> Channel {
    Channel::new(id("sample-ext"), "panel-1".into(), 100, limits).unwrap()
}
fn admit(c: &mut Channel, m: &Envelope) -> Result<Receipt> {
    c.admit(m.from, &serde_json::to_vec(m).unwrap())
}
#[test]
fn channel_orders_messages_and_deduplicates_before_and_after_completion() {
    let mut c = channel(ChannelLimits::default());
    let m = envelope(1);
    assert_eq!(admit(&mut c, &m).unwrap(), Receipt::Queued);
    assert_eq!(
        admit(&mut c, &m).unwrap(),
        Receipt::Duplicate { completed: false }
    );
    c.complete(Endpoint::View, 1).unwrap();
    assert_eq!(
        admit(&mut c, &m).unwrap(),
        Receipt::Duplicate { completed: true }
    );
    assert_eq!(c.queued(), 0);
}
#[test]
fn channel_rejects_stale_generation_extension_and_view() {
    let mut c = channel(ChannelLimits::default());
    for field in 0..3 {
        let mut m = envelope(1);
        match field {
            0 => m.generation = 99,
            1 => m.extension = id("other-ext"),
            _ => m.view = "other-view".into(),
        };
        assert_eq!(admit(&mut c, &m).unwrap_err(), Error::Stale);
    }
    assert_eq!(c.queued(), 0);
}
#[test]
fn channel_rejects_sender_spoofing_and_view_to_kernel_calls() {
    let mut c = channel(ChannelLimits::default());
    let mut m = envelope(1);
    assert_eq!(
        c.admit(Endpoint::Realm, &serde_json::to_vec(&m).unwrap())
            .unwrap_err(),
        Error::Denied
    );
    m.to = Endpoint::Kernel;
    m.message = Message::ServiceCall {
        request: 1,
        method: "project.write".into(),
        args: json!({}),
    };
    assert_eq!(admit(&mut c, &m).unwrap_err(), Error::Denied);
}
#[test]
fn channel_accepts_all_defined_directions() {
    let mut c = channel(ChannelLimits::default());
    let mut m = envelope(1);
    m.message = Message::Ready {
        origin: "null".into(),
    };
    admit(&mut c, &m).unwrap();
    m.sequence = 2;
    m.message = Message::PaintAck { revision: 3 };
    admit(&mut c, &m).unwrap();
    m.sequence = 3;
    m.message = Message::Failure {
        code: "paint_failed".into(),
    };
    admit(&mut c, &m).unwrap();
    m.sequence = 1;
    m.from = Endpoint::Realm;
    m.to = Endpoint::View;
    m.message = Message::DomBatch {
        revision: 3,
        operations: json!([]),
    };
    admit(&mut c, &m).unwrap();
    m.sequence = 2;
    m.message = Message::Snapshot {
        revision: 3,
        document: json!({}),
    };
    admit(&mut c, &m).unwrap();
    m.sequence = 3;
    m.to = Endpoint::Kernel;
    m.message = Message::ServiceCall {
        request: 1,
        method: "project.read".into(),
        args: json!([]),
    };
    admit(&mut c, &m).unwrap();
    m.sequence = 1;
    m.from = Endpoint::Kernel;
    m.to = Endpoint::Realm;
    m.message = Message::ServiceReply {
        request: 1,
        result: WireResult::Ok { value: json!(42) },
    };
    admit(&mut c, &m).unwrap();
    m.sequence = 2;
    m.message = Message::KernelEvent {
        revision: 4,
        event: "changed".into(),
        data: json!({}),
    };
    admit(&mut c, &m).unwrap();
    assert_eq!(c.queued(), 8);
}
#[test]
fn channel_requires_opaque_origin_and_nonempty_event_names() {
    let mut c = channel(ChannelLimits::default());
    let mut m = envelope(1);
    for message in [
        Message::Ready {
            origin: "app://forged".into(),
        },
        Message::Input {
            node: 0,
            event: "click".into(),
            data: Value::Null,
        },
        Message::Input {
            node: 1,
            event: "".into(),
            data: Value::Null,
        },
    ] {
        m.message = message;
        assert_eq!(admit(&mut c, &m).unwrap_err(), Error::Denied);
    }
}
#[test]
fn channel_backpressure_never_consumes_sequence_or_drops_old_work() {
    let mut c = channel(ChannelLimits {
        queued_messages: 1,
        ..ChannelLimits::default()
    });
    admit(&mut c, &envelope(1)).unwrap();
    assert_eq!(
        admit(&mut c, &envelope(2)).unwrap_err(),
        Error::Backpressure
    );
    assert_eq!(c.front().unwrap().sequence, 1);
    c.complete(Endpoint::View, 1).unwrap();
    admit(&mut c, &envelope(2)).unwrap();
}
#[test]
fn channel_enforces_byte_budgets_and_strict_schema() {
    let mut c = channel(ChannelLimits {
        message_bytes: 10,
        ..ChannelLimits::default()
    });
    assert!(admit(&mut c, &envelope(1)).is_err());
    let mut c = channel(ChannelLimits {
        queued_bytes: 10,
        ..ChannelLimits::default()
    });
    assert_eq!(
        admit(&mut c, &envelope(1)).unwrap_err(),
        Error::Backpressure
    );
    let mut c = channel(ChannelLimits::default());
    let mut v = serde_json::to_value(envelope(1)).unwrap();
    v["surprise"] = json!(true);
    assert!(
        c.admit(Endpoint::View, &serde_json::to_vec(&v).unwrap())
            .is_err()
    );
    assert!(c.admit(Endpoint::View, b"{").is_err());
}
#[test]
fn channel_refuses_gaps_zero_and_out_of_order_completion() {
    let mut c = channel(ChannelLimits::default());
    assert!(admit(&mut c, &envelope(0)).is_err());
    assert!(admit(&mut c, &envelope(2)).is_err());
    admit(&mut c, &envelope(1)).unwrap();
    admit(&mut c, &envelope(2)).unwrap();
    assert!(c.complete(Endpoint::View, 2).is_err());
    assert_eq!(c.queued(), 2);
    c.complete(Endpoint::View, 1).unwrap();
    c.complete(Endpoint::View, 2).unwrap();
}
#[test]
fn channel_rejects_version_and_invalid_config() {
    let mut c = channel(ChannelLimits::default());
    let mut m = envelope(1);
    m.version = 2;
    assert!(admit(&mut c, &m).is_err());
    assert!(Channel::new(id("sample-ext"), "".into(), 100, ChannelLimits::default()).is_err());
    assert!(Channel::new(id("sample-ext"), "p".into(), 0, ChannelLimits::default()).is_err());
}

fn token(extension: &str, generation: u64) -> ViewToken {
    ViewToken {
        data_store: DataStoreIdentity::new("p", id(extension)).unwrap(),
        view: "panel".into(),
        generation,
    }
}
fn view() -> View {
    View::start(token("sample-ext", 100), 0, RecoveryPolicy::default())
        .unwrap()
        .0
}
fn fail_and_stop(v: &mut View, reason: Failure, now: u64) -> Action {
    let g = v.token().generation;
    assert!(matches!(
        v.fail(g, reason, now).unwrap(),
        Action::Seal { .. }
    ));
    assert!(matches!(v.sealed(g, now).unwrap(), Action::Stop { .. }));
    v.stopped(g, now).unwrap()
}
#[test]
fn view_ready_requests_authoritative_snapshot() {
    let mut v = view();
    assert_eq!(v.status(), ViewStatus::Starting);
    assert!(matches!(
        v.ready(100, 1).unwrap(),
        Action::RestoreSnapshot { .. }
    ));
    assert_eq!(v.status(), ViewStatus::Ready);
    assert!(v.ready(100, 2).is_err());
}
#[test]
fn view_crash_requires_seal_stop_exit_and_delay_before_recreation() {
    let mut v = view();
    v.ready(100, 1).unwrap();
    assert!(matches!(
        v.fail(100, pm_store_host::view::Failure::Crash, 10)
            .unwrap(),
        Action::Seal { .. }
    ));
    assert!(v.stopped(100, 10).is_err());
    assert!(v.restart(101, 10).is_err());
    v.sealed(100, 10).unwrap();
    assert!(matches!(
        v.stopped(100, 11).unwrap(),
        Action::ScheduleRestart {
            not_before_ms: 261,
            ..
        }
    ));
    assert_eq!(v.restart(101, 260).unwrap_err(), Error::Backpressure);
    let store = v.token().data_store.clone();
    assert!(matches!(
        v.restart(101, 261).unwrap(),
        Action::Create { .. }
    ));
    assert_eq!(v.token().data_store, store);
    assert!(v.ready(100, 262).is_err());
    v.ready(101, 262).unwrap();
}
#[test]
fn crash_loop_disables_after_bounded_restarts_and_can_be_explicitly_reenabled() {
    let mut v = view();
    let mut now = 1;
    for n in 0..3 {
        assert!(matches!(
            fail_and_stop(&mut v, pm_store_host::view::Failure::Crash, now),
            Action::ScheduleRestart { .. }
        ));
        now += 1000;
        v.restart(101 + n, now).unwrap();
        now += 1;
    }
    assert!(matches!(
        fail_and_stop(&mut v, pm_store_host::view::Failure::Crash, now),
        Action::Disabled { .. }
    ));
    assert_eq!(v.status(), ViewStatus::Disabled);
    v.restart(104, now + 1).unwrap();
    assert_eq!(v.status(), ViewStatus::Starting);
}
#[test]
fn failure_window_expires_without_erasing_user_data() {
    let mut v = view();
    fail_and_stop(&mut v, pm_store_host::view::Failure::Crash, 1);
    v.restart(101, 1000).unwrap();
    v.ready(101, 1001).unwrap();
    let action = fail_and_stop(&mut v, pm_store_host::view::Failure::Crash, 61_000);
    assert!(matches!(
        action,
        Action::ScheduleRestart {
            not_before_ms: 61_250,
            ..
        }
    ));
}
#[test]
fn user_stop_does_not_automatically_restart() {
    let mut v = view();
    assert!(matches!(
        fail_and_stop(&mut v, pm_store_host::view::Failure::UserStop, 1),
        Action::Disabled { .. }
    ));
    assert!(v.tick(100_000).unwrap().is_none());
    assert_eq!(v.status(), ViewStatus::Disabled);
}
#[test]
fn startup_timeout_uses_same_safe_recovery_path() {
    let mut v = view();
    assert!(v.tick(19_999).unwrap().is_none());
    assert!(matches!(v.tick(20_000).unwrap(), Some(Action::Seal { .. })));
    assert_eq!(v.status(), ViewStatus::Sealing);
}
#[test]
fn recovery_rejects_clock_reversal_and_stale_completion() {
    let mut v = view();
    v.ready(100, 5).unwrap();
    assert!(v.fail(100, pm_store_host::view::Failure::Crash, 4).is_err());
    assert_eq!(v.status(), ViewStatus::Ready);
    v.fail(100, pm_store_host::view::Failure::Crash, 6).unwrap();
    assert!(v.sealed(99, 6).is_err());
    v.sealed(100, 6).unwrap();
    assert!(v.stopped(99, 7).is_err());
}
#[test]
fn closing_cancels_restart_and_keeps_platform_store() {
    let mut v = view();
    let mut platform = MemoryPlatform::default();
    platform
        .enqueue(&Action::Create {
            token: v.token().clone(),
        })
        .unwrap();
    let key = v.token().data_store.key.clone();
    let handle = platform.store_handle(&key);
    fail_and_stop(&mut v, pm_store_host::view::Failure::Crash, 1);
    platform.enqueue(&v.close().unwrap()).unwrap();
    assert!(v.restart(101, 1000).is_err());
    assert_eq!(platform.store_handle(&key), handle);
}
#[test]
fn platform_double_separates_extensions_and_reuses_store_across_views_and_restarts() {
    let mut p = MemoryPlatform::default();
    let a = token("sample-ext", 100);
    let b = token("other-ext", 101);
    p.enqueue(&Action::Create { token: a.clone() }).unwrap();
    p.enqueue(&Action::Create { token: b.clone() }).unwrap();
    let h = p.store_handle(&a.data_store.key);
    assert_ne!(h, p.store_handle(&b.data_store.key));
    let mut replacement = a.clone();
    replacement.generation = 102;
    replacement.view = "panel-2".into();
    p.enqueue(&Action::Create { token: replacement }).unwrap();
    assert_eq!(p.store_handle(&a.data_store.key), h);
}
#[test]
fn platform_enqueue_failure_is_reported_without_losing_store_registry() {
    let mut p = MemoryPlatform::default();
    p.fail_next = true;
    assert!(
        p.enqueue(&Action::Create {
            token: token("sample-ext", 100)
        })
        .is_err()
    );
    assert!(p.actions.is_empty());
    p.enqueue(&Action::Create {
        token: token("sample-ext", 100),
    })
    .unwrap();
    assert_eq!(p.actions.len(), 1);
}
#[test]
fn zero_restart_budget_and_ready_deadline_are_explicit() {
    let (mut v, _) = View::start(
        token("sample-ext", 100),
        0,
        RecoveryPolicy {
            max_restarts: 0,
            ..RecoveryPolicy::default()
        },
    )
    .unwrap();
    assert!(v.ready(100, 20_000).is_err());
    assert!(matches!(
        fail_and_stop(&mut v, pm_store_host::view::Failure::ReadyTimeout, 20_000),
        Action::Disabled { .. }
    ));
}

fn restore(
    k: &mut impl StoreKernel,
    a: &mut impl ArtifactStore,
    ext: &ExtensionId,
    expected: u64,
    archive: u64,
) -> Result<u64> {
    let state = k.load(ext)?.ok_or(Error::NotInstalled)?;
    let target = state.archives.iter().find(|a| a.id == archive);
    let version = target
        .map(|a| a.release.manifest.version.as_str())
        .unwrap_or("1.0.0");
    let permissions: Vec<_> = target
        .map(|a| a.release.manifest.permissions.iter().copied().collect())
        .unwrap_or_default();
    pm_store_host::install::rollback(
        k,
        a,
        &verified(version, &permissions),
        ext,
        expected,
        archive,
    )
}

#[test]
fn channel_rejects_sequence_reuse_with_different_authored_payload() {
    let mut c = channel(ChannelLimits::default());
    let original = envelope(1);
    admit(&mut c, &original).unwrap();
    let mut changed = original.clone();
    changed.message = Message::PaintAck { revision: 123 };
    assert_eq!(admit(&mut c, &changed).unwrap_err(), Error::Conflict);
    c.complete(Endpoint::View, 1).unwrap();
    assert_eq!(admit(&mut c, &changed).unwrap_err(), Error::Conflict);
    assert_eq!(
        admit(&mut c, &original).unwrap(),
        Receipt::Duplicate { completed: true }
    );
}
#[test]
fn channel_replay_retention_is_bounded_and_old_retries_require_query() {
    let mut c = channel(ChannelLimits::default());
    for seq in 1..=1000 {
        admit(&mut c, &envelope(seq)).unwrap();
        c.complete(Endpoint::View, seq).unwrap();
    }
    assert_eq!(admit(&mut c, &envelope(1)).unwrap_err(), Error::Stale);
    assert_eq!(
        admit(&mut c, &envelope(1000)).unwrap(),
        Receipt::Duplicate { completed: true }
    );
    assert_eq!(c.queued(), 0);
}
#[test]
fn channel_duplicate_payload_comparison_ignores_json_whitespace() {
    let mut c = channel(ChannelLimits::default());
    let m = envelope(1);
    admit(&mut c, &m).unwrap();
    c.complete(Endpoint::View, 1).unwrap();
    assert_eq!(
        c.admit(Endpoint::View, &serde_json::to_vec_pretty(&m).unwrap())
            .unwrap(),
        Receipt::Duplicate { completed: true }
    );
}
#[test]
fn failed_stop_is_visible_and_cannot_restart_before_exit_is_confirmed() {
    let mut v = view();
    v.fail(100, pm_store_host::view::Failure::Hang, 1).unwrap();
    v.sealed(100, 1).unwrap();
    assert!(matches!(
        v.stop_failed(100).unwrap(),
        Action::Disabled {
            reason: pm_store_host::view::Failure::PlatformFailure,
            ..
        }
    ));
    assert_eq!(v.status(), ViewStatus::StopFailed);
    assert!(v.restart(101, 1000).is_err());
    assert!(matches!(
        v.stopped(100, 1001).unwrap(),
        Action::ScheduleRestart { .. }
    ));
    v.restart(101, 1251).unwrap();
}
#[test]
fn stop_exit_deadline_does_not_pretend_successful_recovery() {
    let mut v = view();
    v.fail(100, pm_store_host::view::Failure::Crash, 1).unwrap();
    v.sealed(100, 1).unwrap();
    assert!(v.tick(20_000).unwrap().is_none());
    assert!(matches!(
        v.tick(20_001).unwrap(),
        Some(Action::Disabled {
            reason: pm_store_host::view::Failure::PlatformFailure,
            ..
        })
    ));
    assert!(v.restart(101, 30_000).is_err());
}
#[test]
fn rollback_rejects_different_signed_release_and_corrupt_preserved_bytes() {
    let (mut k, _dir, mut a) = installed(&[]);
    install(
        &mut k,
        &mut a,
        "test-profile",
        &verified("2.0.0", &[]),
        Some(1),
    )
    .unwrap();
    let before = state(&k);
    assert_eq!(
        pm_store_host::install::rollback(
            &mut k,
            &mut a,
            &verified("9.0.0", &[]),
            &id("sample-ext"),
            2,
            2
        )
        .unwrap_err(),
        Error::Integrity("rollback target")
    );
    let path = a.object_path(&before.archives[0].release.bundle).unwrap();
    fs::write(path, b"damaged").unwrap();
    assert!(restore(&mut k, &mut a, &id("sample-ext"), 2, 2).is_err());
    assert_eq!(state(&k), before);
}

#[test]
fn stop_deadline_starts_after_kernel_sealing_completes() {
    let mut v = view();
    v.fail(100, pm_store_host::view::Failure::Crash, 1).unwrap();
    // A slow worker seal does not count against platform process-exit time.
    v.sealed(100, 30_000).unwrap();
    assert!(v.tick(49_999).unwrap().is_none());
    assert!(matches!(
        v.tick(50_000).unwrap(),
        Some(Action::Disabled { .. })
    ));
}
