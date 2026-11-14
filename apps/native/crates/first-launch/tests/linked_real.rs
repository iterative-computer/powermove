use pm_first_launch::{Config, Outcome};
use pm_kernel::{
    Kernel, KernelConfig, LibraryStore,
    persistence::DiskFormat,
    project_io::MediaSource,
    session::{SessionSource, ShellCall},
};
#[test]
#[ignore = "requires explicit local real-data paths; never runs in CI"]
fn real_linked_launch() {
    let source =
        std::path::PathBuf::from(std::env::var_os("PM_LINK_SOURCE").expect("PM_LINK_SOURCE"));
    let native =
        std::path::PathBuf::from(std::env::var_os("PM_LINK_NATIVE").expect("PM_LINK_NATIVE"));
    assert!(!native.exists());
    let config = Config {
        electron_profile: source,
        native_profile: native.clone(),
    };
    let start = std::time::Instant::now();
    let mut media_start = None;
    let mut media_seconds = 0.0;
    let mut progress = |event| {
        if matches!(
            event,
            pm_first_launch::Event::Started(pm_first_launch::Step::Media)
        ) {
            media_start = Some(std::time::Instant::now());
        }
        if let pm_first_launch::Event::Progress {
            step: pm_first_launch::Step::Media,
            done,
            total,
        } = event
            && done == total
            && total > 0
            && let Some(begin) = media_start.take()
        {
            media_seconds += begin.elapsed().as_secs_f64();
        }

        if matches!(
            event,
            pm_first_launch::Event::Started(_) | pm_first_launch::Event::Published(_)
        ) {
            println!("{event:?} at {:.3}s", start.elapsed().as_secs_f64());
        }
        Ok(())
    };
    let mut outcome = pm_first_launch::run(&config, &mut progress).unwrap();
    loop {
        outcome = match outcome {
            Outcome::NeedsSecretConsent { .. } => {
                pm_first_launch::skip_secret_consent(&config, &mut progress).unwrap()
            }
            Outcome::NeedsFolderAccess { .. } => pm_first_launch::run_with_folder_access(
                &config,
                pm_first_launch::source_access::Choice::Skip,
                &mut progress,
            )
            .unwrap(),
            _ => break,
        };
    }
    drop(progress);
    println!(
        "wall={:?} media_publication_seconds={media_seconds:.3} outcome={outcome:?}",
        start.elapsed()
    );
    assert!(matches!(outcome, Outcome::Complete { .. }));
    let summary = pm_native_migrate::linked_summary(&native.join("Media Store")).unwrap();
    println!(
        "items={} bytes={} missing={}",
        summary.total_items, summary.total_bytes, summary.missing
    );
    copy_mode_offline_audit(&config, &outcome);
    let store = LibraryStore::open(&native);
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel
        .shell(ShellCall::AttachLibrary {
            profile: native.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .unwrap();
    let mut hydrated = 0;
    let mut resolved = 0;
    for (index, row) in store.projects().unwrap().into_iter().enumerate() {
        let session = format!("check-{index}");
        kernel
            .shell(ShellCall::Open {
                id: session.clone(),
                source: SessionSource::Library {
                    profile: native.clone(),
                    project: row.id,
                },
                electron_profile: None,
            })
            .unwrap();
        let media = kernel.project(&session).unwrap().store().media();
        let mut count = 0;
        for entry in media.iter() {
            if let MediaSource::File { path, .. } = &entry.source {
                assert!(path.is_file());
                count += 1;
            }
        }
        if count > 0 {
            hydrated += 1;
            resolved += count;
        }
        if hydrated >= 5 {
            break;
        }
    }
    println!("hydrated_projects={hydrated} resolved_files={resolved}");
    assert!(hydrated >= 3);
    drop(kernel);
    fn unseal(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::symlink_metadata(path).unwrap().is_dir() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
            for entry in std::fs::read_dir(path).unwrap() {
                unseal(&entry.unwrap().path());
            }
        }
    }
    unseal(&native);
    std::fs::remove_dir_all(native).unwrap();
}

mod support;
#[test]
fn first_launch_links_live_blobs_and_kernel_resolves_them() {
    let temp = tempfile::tempdir().unwrap();
    let config = support::profile(&temp.path().canonicalize().unwrap());
    for family in ["backups", "recovery", "transcription-cache"] {
        std::fs::create_dir_all(config.electron_profile.join(family)).unwrap();
        std::fs::write(
            config.electron_profile.join(family).join("excluded"),
            b"synthetic excluded payload",
        )
        .unwrap();
    }
    for i in 0..40 {
        let path = config.electron_profile.join("Agent State/metadata");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join(format!("{i}.json")), b"{}").unwrap();
    }
    let before = support::files(&config.electron_profile);
    assert!(matches!(
        pm_first_launch::run(&config, |_| Ok(())).unwrap(),
        Outcome::Complete {
            blocked_owners: 0,
            ..
        }
    ));
    assert_eq!(before, support::files(&config.electron_profile));
    let cp = support::checkpoint(&config);
    assert!(cp.chromium_payloads.is_empty());
    for family in [
        "Agent Workspaces",
        "backups",
        "recovery",
        "transcription-cache",
    ] {
        assert!(!cp.files.keys().any(|p| p.starts_with(family)));
    }
    let summary =
        pm_native_migrate::linked_summary(&config.native_profile.join("Media Store")).unwrap();
    assert!(summary.total_items > 0);
    assert_eq!(summary.missing, 0);
    assert_eq!(
        std::fs::read_dir(config.native_profile.join("Media Store/objects"))
            .unwrap()
            .count(),
        0
    );
    let mut kernel = Kernel::new(KernelConfig::default());
    kernel
        .shell(ShellCall::AttachLibrary {
            profile: config.native_profile.clone(),
            format: DiskFormat::V1_2,
            electron_profile: None,
        })
        .unwrap();
    kernel
        .shell(ShellCall::Open {
            id: "check".into(),
            source: SessionSource::Library {
                profile: config.native_profile.clone(),
                project: cp.projects["p"].clone(),
            },
            electron_profile: None,
        })
        .unwrap();
    let media = kernel.project("check").unwrap().store().media();
    assert!(media.iter().any(|entry| matches!(&entry.source, MediaSource::File { path, .. } if path.starts_with(&config.electron_profile) && path.is_file())));
}

// Code-based baseline audit: the pre-link importer could only bring an original
// source matching its recorded identity, an eligible disk index entry, or a
// captured IndexedDB payload (including its virtual empty-payload case).
fn copy_mode_offline_audit(config: &Config, outcome: &Outcome) {
    use pm_native_migrate::{
        migration::{Pass, Status},
        snapshot::Snapshot,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Read;
    let read = |path: &std::path::Path| {
        serde_json::from_reader::<_, serde_json::Value>(std::fs::File::open(path).unwrap()).unwrap()
    };
    let native = &config.native_profile;
    let cp_root = native.join("first-launch-checkpoint");
    let cp: pm_first_launch::Checkpoint =
        serde_json::from_value(read(&cp_root.join("checkpoint.json"))).unwrap();
    let state: pm_first_launch::State =
        serde_json::from_value(read(&native.join("migration.json"))).unwrap();
    let media = read(&native.join("Media Store/migration.json"));
    let id = format!(
        "{}:{}",
        state.latest_pass.as_ref().unwrap(),
        state.latest_pass_base.as_ref().unwrap()
    );
    let pass: Pass = serde_json::from_value(media["passes"][&id].clone()).unwrap();
    let batches = cp
        .indexeddb
        .iter()
        .map(|p| {
            let snapshot = Snapshot::open(&cp_root.join(p)).unwrap();
            let records = pm_native_migrate::indexeddb::read(&snapshot)
                .unwrap()
                .records;
            (snapshot, records)
        })
        .collect::<Vec<_>>();
    let disks = pm_native_migrate::library_media::read_link_index(
        &cp_root.join("profile/Media Store"),
        &cp.source.join("Media Store"),
    )
    .unwrap();
    let mut lent = BTreeMap::<String, BTreeSet<String>>::new();
    let (mut checked, mut copy_available, mut original, mut disk, mut blob, mut empty) =
        (0, 0, 0, 0, 0, 0);
    for entry in pass
        .closure
        .iter()
        .filter(|e| e.status == Status::AlreadyOffline)
    {
        let r = &entry.reference;
        if let Some(copy) = state.receipts.get(&r.owner) {
            let ids = lent
                .entry(copy.native.clone())
                .or_insert_with(|| pm_kernel::embedded_media::lent(native, &copy.native));
            if r.legacy_id.as_ref().is_some_and(|id| ids.contains(id)) {
                continue;
            }
        }
        checked += 1;
        let named = |key: &str| key == r.key || r.legacy_id.as_deref() == Some(key);
        let indexed = disks.iter().any(|d| named(&d.key));
        let disk_record = disks.iter().find(|d| d.key == r.key).or_else(|| {
            r.legacy_id
                .as_ref()
                .and_then(|id| disks.iter().find(|d| &d.key == id))
        });
        let from_disk = disk_record
            .filter(|d| r.revision.as_ref().is_none_or(|rev| rev == &d.revision))
            .is_some_and(|d| d.open_verified().is_ok());
        let mut seen = BTreeSet::new();
        let mut candidates = Vec::new();
        for (snapshot, records) in &batches {
            for record in records.iter().filter(|d| named(&d.key)) {
                if seen.insert((record.database, record.store, record.key.clone())) {
                    candidates.push((snapshot, record));
                }
            }
        }
        if candidates.iter().any(|(_, record)| record.key == r.key) {
            candidates.retain(|(_, record)| record.key == r.key);
        }
        candidates.retain(|(_, record)| {
            r.revision
                .as_deref()
                .is_none_or(|rev| Some(rev) == record.revision())
        });
        assert!(
            candidates.len() <= 1,
            "baseline importer would refuse ambiguous media"
        );
        let from_empty = candidates
            .iter()
            .any(|(_, record)| record.payload().size == 0);
        let from_blob = candidates.iter().any(|(snapshot, record)| {
            snapshot
                .linked_blob_path(record.database, record.payload().number)
                .unwrap()
                .is_some()
        });
        let from_original = !indexed
            && !r.unreached
            && r.source_path.as_ref().is_some_and(|path| {
                if !pm_native_migrate::library_media::is_resident(path).unwrap_or(false) {
                    return false;
                }
                if let Some(expected) = &r.original_blake3 {
                    let Ok(mut file) = std::fs::File::open(path) else {
                        return false;
                    };
                    let mut hash = blake3::Hasher::new();
                    let mut buffer = vec![0; 1024 * 1024];
                    loop {
                        let Ok(n) = file.read(&mut buffer) else {
                            return false;
                        };
                        if n == 0 {
                            break;
                        }
                        hash.update(&buffer[..n]);
                    }
                    hash.finalize().to_hex().as_str() == expected.as_str()
                } else if let Some(expected) = &r.fingerprint {
                    pm_native_migrate::migration::sampled_fingerprint(path)
                        .is_ok_and(|fp| fp == *expected)
                } else {
                    false
                }
            });
        original += usize::from(from_original);
        disk += usize::from(from_disk);
        blob += usize::from(from_blob);
        empty += usize::from(from_empty);
        copy_available += usize::from(from_original || from_disk || from_blob || from_empty);
    }
    let Outcome::Complete {
        already_offline, ..
    } = outcome
    else {
        panic!("incomplete baseline audit");
    };
    println!(
        "copy_baseline_audit link_offline={checked} copy_offline={} copy_available={copy_available} original={original} disk={disk} blob={blob} empty={empty}",
        checked - copy_available
    );
    assert_eq!(checked, *already_offline);
    assert_eq!(
        copy_available, 0,
        "link mode lost media the copying importer could bring over"
    );
}
