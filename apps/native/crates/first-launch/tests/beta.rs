mod support;
use anyhow::Result;
use pm_distribution::{
    update_distribution::{Architecture, Channel, Installer, Recovery, Release, Stage},
    update_handoff::{DeltaReport, HandoffStatus, LaunchSource},
};
use pm_first_launch::{
    integration::{Coordinator, Evidence, UpdateSchedule},
    sparkle::{Driver, Enclosure, Sparkle},
    store_preparation::{self as preparation, Store},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use support::*;

#[derive(Default)]
struct Proof {
    tr: bool,
    verified: usize,
    observations: usize,
    ready: bool,
    changed: bool,
    force_secret_gap: bool,
}
impl Evidence for Proof {
    fn electron_quiescent(&mut self) -> Result<bool> {
        Ok(true)
    }
    fn verify_transition_receipt(&mut self) -> Result<bool> {
        self.verified += 1;
        Ok(self.tr)
    }
    fn after_delta(&mut self) -> Result<DeltaReport> {
        self.observations += 1;
        Ok(DeltaReport {
            complete: true,
            six_owner_domains: true,
            state_complete: true,
            secrets_complete: !self.force_secret_gap,
            restore_verified: self.ready,
            source_generation: 2,
            checkpoint_generation: if self.changed { 1 } else { 2 },
        })
    }
}
#[test]
fn direct_skipped_and_verified_tr_all_run_real_import_and_final_delta() {
    for (source, tr) in [
        (LaunchSource::DirectDmg, false),
        (LaunchSource::TransitionRelease, false),
        (LaunchSource::TransitionRelease, true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let config = profile(&temp.path().canonicalize().unwrap());
        let before = files(&config.electron_profile);
        let mut proof = Proof {
            tr,
            ready: true,
            ..Proof::default()
        };
        let mut host = Coordinator {
            config: &config,
            evidence: &mut proof,
            events: |_| Ok(()),
        };
        let result = host.first_run(source).unwrap();
        assert_eq!(result.status, HandoffStatus::Ready);
        assert_eq!(result.reuse_transition_evidence, tr);
        assert_eq!(proof.observations, 1);
        assert_eq!(
            proof.verified,
            usize::from(source == LaunchSource::TransitionRelease)
        );
        assert_eq!(before, files(&config.electron_profile));
        assert!(config.native_profile.join("first-launch-deltas").is_dir());
    }
}
#[test]
fn missing_restore_proof_or_changed_generation_never_allows_cutover() {
    for changed in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let config = profile(&temp.path().canonicalize().unwrap());
        let mut proof = Proof {
            ready: changed,
            changed,
            ..Proof::default()
        };
        let mut host = Coordinator {
            config: &config,
            evidence: &mut proof,
            events: |_| Ok(()),
        };
        assert_eq!(
            host.first_run(LaunchSource::DirectDmg).unwrap().status,
            HandoffStatus::Blocked
        );
    }
}
#[test]
fn electron_waits_before_any_tr_or_profile_work() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile(&temp.path().canonicalize().unwrap());
    std::os::unix::fs::symlink(
        format!("localhost-{}", std::process::id()),
        config.electron_profile.join("SingletonLock"),
    )
    .unwrap();
    let mut proof = Proof {
        tr: true,
        ready: true,
        ..Proof::default()
    };
    let mut host = Coordinator {
        config: &config,
        evidence: &mut proof,
        events: |_| Ok(()),
    };
    assert_eq!(
        host.first_run(LaunchSource::TransitionRelease)
            .unwrap()
            .status,
        HandoffStatus::WaitingForElectron
    );
    assert_eq!(proof.verified, 0);
    assert_eq!(proof.observations, 0);
    assert!(!config.native_profile.exists());
}
#[test]
fn final_delta_observes_import_after_bootstrap_publication() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile(&temp.path().canonicalize().unwrap());
    let mut proof = Proof {
        ready: true,
        ..Proof::default()
    };
    let mut first = true;
    let mut host = Coordinator {
        config: &config,
        evidence: &mut proof,
        events: |event| {
            if event == pm_first_launch::Event::Complete && first {
                first = false;
                media_db(&config.electron_profile, true);
            }
            Ok(())
        },
    };
    assert_eq!(
        host.first_run(LaunchSource::DirectDmg).unwrap().status,
        HandoffStatus::Ready
    );
    let migration: serde_json::Value =
        read(&config.native_profile.join("Media Store/migration.json"));
    assert_eq!(migration["passes"].as_object().unwrap().len(), 2);
}
#[test]
fn polling_uses_existing_thirty_second_and_four_hour_cadence() {
    let mut schedule = UpdateSchedule::started(100);
    assert!(!schedule.due(129));
    assert!(schedule.due(130));
    assert!(!schedule.due(131));
    assert!(!schedule.due(14529));
    assert!(schedule.due(14530));
    assert!(!schedule.due(14530));
}
fn release() -> Release {
    Release {
        schema: 1,
        channel: Channel::Beta,
        version: "2.0.0-beta.2".into(),
        architecture: Architecture::Arm64,
        bundle_id: pm_first_launch::BETA_BUNDLE_ID.into(),
        minimum_macos: 14,
        archive_url: "https://example.invalid/native.zip".into(),
        size: 9,
        sha256: format!("{:x}", Sha256::digest(b"synthetic")),
        sparkle_signature: "synthetic-signature".into(),
        expires_unix: u64::MAX,
    }
}
struct Backend {
    enclosure: Enclosure,
    staged: usize,
    installed: usize,
    failure: bool,
}
impl Sparkle for Backend {
    fn selected_enclosure(&mut self, _: &str) -> Result<Enclosure> {
        Ok(self.enclosure.clone())
    }
    fn stage_verified(&mut self, _: &str, _: &Path, _: &Release) -> Result<String> {
        self.staged += 1;
        if self.failure {
            anyhow::bail!("synthetic interrupted extraction");
        }
        Ok("opaque-ticket".into())
    }
    fn recover(&mut self, _: &str, _: &Stage) -> Result<Recovery> {
        Ok(Recovery::Pending)
    }
    fn install_on_normal_quit(&mut self, _: &str, _: &str) -> Result<()> {
        self.installed += 1;
        Ok(())
    }
}
fn driver(r: &Release) -> Driver<Backend> {
    Driver {
        sparkle: Backend {
            enclosure: Enclosure {
                version: r.version.clone(),
                archive_url: r.archive_url.clone(),
                size: r.size,
                signature: r.sparkle_signature.clone(),
            },
            staged: 0,
            installed: 0,
            failure: false,
        },
    }
}
#[test]
fn sparkle_binds_every_enclosure_field_before_any_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive.zip");
    fs::write(&archive, b"synthetic").unwrap();
    let release = release();
    for field in 0..4 {
        let mut driver = driver(&release);
        match field {
            0 => driver.sparkle.enclosure.version.push('x'),
            1 => driver.sparkle.enclosure.archive_url.push('x'),
            2 => driver.sparkle.enclosure.size += 1,
            _ => driver.sparkle.enclosure.signature.push('x'),
        }
        assert!(driver.stage(&"a".repeat(64), &archive, &release).is_err());
        assert_eq!(driver.sparkle.staged, 0);
    }
}
#[test]
fn sparkle_rechecks_bytes_and_retains_interrupted_install_input() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("archive.zip");
    let release = release();
    let mut driver = driver(&release);
    for bytes in [b"wrongbyte".as_slice(), b"too short", b"far too long"] {
        fs::write(&archive, bytes).unwrap();
        assert!(driver.stage(&"a".repeat(64), &archive, &release).is_err());
        assert_eq!(driver.sparkle.staged, 0);
    }
    fs::write(&archive, b"synthetic").unwrap();
    driver.sparkle.failure = true;
    assert!(driver.stage(&"a".repeat(64), &archive, &release).is_err());
    assert_eq!(fs::read(&archive).unwrap(), b"synthetic");
    assert_eq!(
        driver
            .recover(
                &"a".repeat(64),
                &Stage::Staging {
                    file: "archive.zip".into()
                }
            )
            .unwrap(),
        Recovery::Pending
    );
    driver.sparkle.failure = false;
    let ticket = driver.stage(&"a".repeat(64), &archive, &release).unwrap();
    assert_eq!(driver.sparkle.installed, 0);
    driver.request_install(&"a".repeat(64), &ticket).unwrap();
    assert_eq!(driver.sparkle.installed, 1);
}
#[test]
fn merge_input_orders_preserve_unknown_members_nulls_tombstones_and_conflicts() {
    let base: Store = serde_json::from_value(json!({"ext.synthetic":{"deleted":1,"null":2,"shared":0,"opaque":{"future":[1,2]}},"theme":"base","future":{"opaque":true}})).unwrap();
    let native: Store = serde_json::from_value(json!({"ext.synthetic":{"null":null,"shared":1,"opaque":{"future":[1,2]}},"theme":"native","future":{"opaque":true},"projectHistory.new":{"unknown":42,"index":-1,"entries":[]}})).unwrap();
    let electron: Store = serde_json::from_value(json!({"ext.synthetic":{"deleted":1,"null":2,"shared":2,"opaque":{"future":[1,2]},"new":3},"theme":"base","future":{"opaque":true}})).unwrap();
    for (current, incoming) in [(&native, &electron), (&electron, &native)] {
        let result = preparation::merge(&base, current, incoming);
        assert!(result.values["ext.synthetic"].get("deleted").is_none());
        assert!(result.values["ext.synthetic"]["null"].is_null());
        assert_eq!(result.values["ext.synthetic"]["new"], 3);
        assert_eq!(result.values["theme"], "native");
        assert_eq!(result.values["future"], base["future"]);
        assert_eq!(
            result.values["projectHistory.new"],
            native["projectHistory.new"]
        );
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].path, ["ext.synthetic", "shared"]);
    }
}
#[test]
fn delete_against_edit_and_new_null_are_visible_conflicts() {
    let base: Store = serde_json::from_value(json!({"theme":"old"})).unwrap();
    let current = Store::new();
    let incoming: Store = serde_json::from_value(json!({"theme":null})).unwrap();
    let plan = preparation::merge(&base, &current, &incoming);
    assert!(plan.values.is_empty());
    assert_eq!(plan.conflicts[0].current, None);
    assert_eq!(plan.conflicts[0].incoming, Some(json!(null)));
}
#[test]
fn interrupted_restore_save_retains_attempt_and_publishes_current_history() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let destination = root.join("Restore");
    let current: Store = serde_json::from_value(json!({"theme":"current","ext.synthetic":{"unknown":[1,2]},"projectHistory.new":{"version":1,"unknown":42,"index":-1,"entries":[]},"future":{"unknown":true}})).unwrap();
    assert!(
        preparation::prepare_store(
            &destination,
            &Store::new(),
            &current,
            &Store::new(),
            |_| anyhow::bail!("synthetic interrupted save")
        )
        .is_err()
    );
    assert!(!destination.exists());
    let attempt = files(&root.join("Restore.preparing"));
    let result =
        preparation::prepare_store(&destination, &Store::new(), &current, &Store::new(), |_| {
            Ok(())
        })
        .unwrap();
    assert_eq!(
        files(&root.join("Restore.preparing.archive-0000000000000000")),
        attempt
    );
    assert_eq!(result.values, current);
    let history: serde_json::Value = read(&destination.join("store/projectHistory.new.json"));
    assert_eq!(history["unknown"], 42);
    assert!(destination.join("restore-inputs.json").is_file());
    assert!(destination.join("restore-plan.json").is_file());
    assert!(
        preparation::prepare_store(
            &destination,
            &Store::new(),
            &current,
            &Store::new(),
            |_| Ok(())
        )
        .is_err()
    );
}
#[test]
fn repeated_delta_and_failed_capture_archive_every_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile(&temp.path().canonicalize().unwrap());
    pm_first_launch::run(&config, |_| Ok(())).unwrap();
    pm_first_launch::run(&config, |_| Ok(())).unwrap();
    pm_first_launch::run(&config, |_| Ok(())).unwrap();
    assert!(
        config
            .native_profile
            .join("first-launch-deltas/pending.archive-0000000000000000/checkpoint.json")
            .is_file()
    );
    let capture = config.native_profile.join("retained.capture");
    fs::create_dir(&capture).unwrap();
    fs::write(capture.join("partial"), b"retained").unwrap();
    pm_first_launch::Checkpoint::capture(
        &config.electron_profile,
        &config.native_profile.join("retained"),
    )
    .unwrap();
    assert_eq!(
        fs::read(
            config
                .native_profile
                .join("retained.capture.archive-0000000000000000/partial")
        )
        .unwrap(),
        b"retained"
    );
}

#[test]
fn concurrent_restore_retry_and_foreign_stage_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let destination = root.join("Restore.beta");
    let values: Store = serde_json::from_value(json!({"theme":"current"})).unwrap();
    preparation::prepare_store(&destination, &Store::new(), &values, &Store::new(), |_| {
        let error =
            preparation::prepare_store(&destination, &Store::new(), &values, &Store::new(), |_| {
                Ok(())
            })
            .unwrap_err();
        assert!(format!("{error:#}").contains("already running"));
        Ok(())
    })
    .unwrap();
    let foreign = root.join("Other.preparing");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("opaque"), b"foreign").unwrap();
    assert!(
        preparation::prepare_store(
            &root.join("Other"),
            &Store::new(),
            &values,
            &Store::new(),
            |_| Ok(())
        )
        .is_err()
    );
    assert_eq!(fs::read(foreign.join("opaque")).unwrap(), b"foreign");
    assert!(!root.join("Other").exists());
}

#[test]
fn restore_and_sparkle_refuse_symlink_destinations_and_archives() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let destination = root.join("Restore");
    std::os::unix::fs::symlink(root.join("missing"), &destination).unwrap();
    assert!(
        preparation::prepare_store(
            &destination,
            &Store::new(),
            &Store::new(),
            &Store::new(),
            |_| Ok(())
        )
        .is_err()
    );
    assert!(
        fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let archive = root.join("original.zip");
    fs::write(&archive, b"synthetic").unwrap();
    let link = root.join("linked.zip");
    std::os::unix::fs::symlink(&archive, &link).unwrap();
    let release = release();
    let mut driver = driver(&release);
    assert!(driver.stage(&"a".repeat(64), &link, &release).is_err());
    assert_eq!(driver.sparkle.staged, 0);
}

#[test]
fn declined_consent_is_not_reopened_by_an_incomplete_update_proof() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile(&temp.path().canonicalize().unwrap());
    write(
        &config.electron_profile.join("agent-provider.json"),
        &json!({"secret":"djEw"}),
    );
    let mut proof = Proof {
        force_secret_gap: true,
        ..Proof::default()
    };
    let mut host = Coordinator {
        config: &config,
        evidence: &mut proof,
        events: |_| Ok(()),
    };
    assert_eq!(
        host.first_run(LaunchSource::DirectDmg).unwrap().status,
        HandoffStatus::NeedsSecretConsent
    );
    pm_first_launch::skip_secret_consent(&config, |_| Ok(())).unwrap();
    let mut host = Coordinator {
        config: &config,
        evidence: &mut proof,
        events: |event| {
            assert_ne!(event, pm_first_launch::Event::SecretPromptRequired);
            Ok(())
        },
    };
    assert_eq!(
        host.first_run(LaunchSource::DirectDmg).unwrap().status,
        HandoffStatus::Blocked
    );
}
