//! PM-478: an export ended by force (a quit signal's deadline, a crash) runs
//! no drop, so its partial file stays beside the destination. Each temporary
//! is recorded in the app's ledger before it exists; the next launch removes
//! exactly what the ledger names and nothing else. Its own process: the
//! ledger is process-wide.
use pm_export::atomic::{AtomicOutput, clean_ledger, set_ledger};
use pm_export::settings::Overwrite;
use std::fs;

fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pm-temp-ledger-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// One test: the ledger is process-wide, and the cases run one after another.
#[test]
fn the_ledger_names_exactly_what_a_forced_end_left() {
    a_forced_end_leaves_only_what_the_next_launch_removes();
    a_finished_or_cancelled_export_leaves_no_entry();
    a_running_export_elsewhere_is_left_alone();
}

fn a_forced_end_leaves_only_what_the_next_launch_removes() {
    let (out, ledger) = (dir("out"), dir("ledger"));
    set_ledger(Some(ledger.clone()));
    // An export in progress, then the process ends with no drop.
    let export = AtomicOutput::new(&out.join("Film.mov"), Overwrite::Fail, None).unwrap();
    let partial = export.temp_path().to_path_buf();
    fs::write(&partial, vec![7u8; 4096]).unwrap();
    assert_eq!(fs::read_dir(&ledger).unwrap().count(), 1, "recorded before it was written");
    std::mem::forget(export);
    // Someone else's file that looks the same, never recorded.
    let lookalike = out.join(".Other.0123456789abcdef.partial.mov");
    fs::write(&lookalike, b"keep").unwrap();
    // A PNG sequence's temporary folder, also left.
    let frames = AtomicOutput::folder(&out.join("Frames"), Overwrite::Fail, None).unwrap();
    let folder = frames.temp_path().to_path_buf();
    fs::write(folder.join("Frames_0001.png"), b"png").unwrap();
    std::mem::forget(frames);
    set_ledger(None);
    // The next launch.
    let mut removed = clean_ledger(&ledger);
    removed.sort();
    let mut expected = vec![partial.clone(), folder.clone()];
    expected.sort();
    assert_eq!(removed, expected);
    assert!(!partial.exists() && !folder.exists());
    assert!(lookalike.exists(), "an unrecorded file is never touched");
    assert_eq!(fs::read_dir(&ledger).unwrap().count(), 0, "entries go with them");
}

/// Another instance's export still at work (its process alive) is left.
fn a_running_export_elsewhere_is_left_alone() {
    let (out, ledger) = (dir("alive"), dir("alive-ledger"));
    let partial = out.join(".Live.0123456789abcdef.partial.mov");
    fs::write(&partial, b"live").unwrap();
    let alive = std::os::unix::process::parent_id();
    fs::write(ledger.join("x.temp"), format!("file\n{}\n{alive}\n", partial.display())).unwrap();
    assert!(clean_ledger(&ledger).is_empty());
    assert!(partial.exists());
}

fn a_finished_or_cancelled_export_leaves_no_entry() {
    let (out, ledger) = (dir("done"), dir("done-ledger"));
    set_ledger(Some(ledger.clone()));
    let done = AtomicOutput::new(&out.join("Done.mov"), Overwrite::Fail, None).unwrap();
    fs::write(done.temp_path(), b"x").unwrap();
    done.commit().unwrap();
    let cancelled = AtomicOutput::new(&out.join("Cancelled.mov"), Overwrite::Fail, None).unwrap();
    fs::write(cancelled.temp_path(), b"x").unwrap();
    drop(cancelled);
    set_ledger(None);
    assert_eq!(fs::read_dir(&ledger).unwrap().count(), 0);
    assert!(out.join("Done.mov").exists());
}
