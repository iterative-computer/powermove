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
    let mut outcome = pm_first_launch::run(&config, |_| Ok(())).unwrap();
    loop {
        outcome = match outcome {
            Outcome::NeedsSecretConsent { .. } => {
                pm_first_launch::skip_secret_consent(&config, |_| Ok(())).unwrap()
            }
            Outcome::NeedsFolderAccess { .. } => pm_first_launch::run_with_folder_access(
                &config,
                pm_first_launch::source_access::Choice::Skip,
                |_| Ok(()),
            )
            .unwrap(),
            _ => break,
        };
    }
    println!("wall={:?} outcome={outcome:?}", start.elapsed());
    assert!(matches!(outcome, Outcome::Complete { .. }));
    let summary = pm_native_migrate::linked_summary(&native.join("Media Store")).unwrap();
    println!(
        "items={} bytes={} missing={}",
        summary.total_items, summary.total_bytes, summary.missing
    );
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
    for row in store.projects().unwrap() {
        kernel
            .shell(ShellCall::Open {
                id: "check".into(),
                source: SessionSource::Library {
                    profile: native.clone(),
                    project: row.id,
                },
                electron_profile: None,
            })
            .unwrap();
        let media = kernel.project("check").unwrap().store().media();
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
