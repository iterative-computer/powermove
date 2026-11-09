mod support;
use pm_first_launch::store_preparation::{self as preparation, Store};
use serde_json::json;
use std::fs;
use support::*;

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
fn a_repeated_delta_keeps_no_copy_and_a_failed_capture_is_archived() {
    let temp = tempfile::tempdir().unwrap();
    let config = profile(&temp.path().canonicalize().unwrap());
    pm_first_launch::run(&config, |_| Ok(())).unwrap();
    pm_first_launch::run(&config, |_| Ok(())).unwrap();
    pm_first_launch::run(&config, |_| Ok(())).unwrap();
    // Nothing changed, so nothing is owed: a capture that adds nothing can't pile up.
    assert_eq!(
        fs::read_dir(config.native_profile.join("first-launch-deltas"))
            .unwrap()
            .count(),
        0
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
fn restore_refuses_symlink_destinations() {
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
}
