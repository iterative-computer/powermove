//! The session folder as a store (Sol's m2 items 1 and 4): a torn or
//! uncertain write is cut before the next append, so a reopen after it and a
//! second reopen after the next append agree on a valid journal; concurrent
//! and interrupted writers of one image never publish bytes that miss its
//! name. Store-only: no core needed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use pm_aero_core::journal::SessionFolder;

const CHILD: &str = "AERO_STORE_CHILD";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aero-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A record of events `first..=last`, each with a text field.
fn events(first: u64, last: u64, text: &str) -> String {
    let parts: Vec<String> = (first..=last)
        .map(|seq| format!(r#"{{"seq":{seq},"type":"turn_progress","text":"{text} {seq}"}}"#))
        .collect();
    format!("[{}]", parts.join(","))
}

/// The seqs a stored journal holds, oldest first (it must be a JSON array of
/// events with seqs).
fn seqs(journal_json: &str) -> Vec<u64> {
    if journal_json.is_empty() {
        return Vec::new();
    }
    let value: serde_json::Value = serde_json::from_str(journal_json)
        .unwrap_or_else(|error| panic!("the stored journal is not JSON ({error}): {journal_json}"));
    value
        .as_array()
        .expect("an array")
        .iter()
        .map(|event| event["seq"].as_u64().expect("a seq"))
        .collect()
}

fn tear(dir: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("events"))
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
}

/// A folder with a checkpoint at seq 3 and a non-empty tail (4..=5), then
/// a torn record for seq 6 (`torn`), as a crash mid-write leaves it.
fn torn_folder(name: &str, torn: &[u8]) -> PathBuf {
    let dir = scratch(name);
    {
        let folder = SessionFolder::open(&dir).expect("open");
        folder.append(1, 2, &events(1, 2, "a")).unwrap();
        folder.append(3, 3, &events(3, 3, "b")).unwrap();
    }
    {
        // Reopened, the stored head is the one the next runtime opens from.
        let folder = SessionFolder::open(&dir).expect("reopen");
        assert!(folder.save_checkpoint(3, b"snapshot").unwrap());
        folder.append(4, 5, &events(4, 5, "c")).unwrap();
    }
    tear(&dir, torn);
    dir
}

fn torn_then_appended_reopens_twice(name: &str, torn: &[u8]) {
    let dir = torn_folder(name, torn);
    {
        let folder = SessionFolder::open(&dir).expect("a torn tail opens");
        let stored = SessionFolder::load(&dir).expect("load");
        assert_eq!(stored.last_seq, 5, "the torn record was never acknowledged");
        assert_eq!(stored.checkpoint, b"snapshot");
        assert_eq!(seqs(&stored.journal_json), vec![4, 5]);
        // The next append takes the torn record's seq.
        folder
            .append(6, 6, &events(6, 6, "d"))
            .expect("append after the tear");
    }
    let folder = SessionFolder::open(&dir).expect("second reopen");
    let stored = SessionFolder::load(&dir).expect("load again");
    assert_eq!(stored.last_seq, 6);
    assert_eq!(
        seqs(&stored.journal_json),
        vec![4, 5, 6],
        "{}",
        stored.journal_json
    );
    assert!(stored.journal_json.contains("d 6"));
    // Every line on disk is whole: no fragment of the torn write is left.
    let text = String::from_utf8(std::fs::read(dir.join("events")).unwrap()).expect("UTF-8");
    assert_eq!(text.lines().count(), 4, "{text}");
    folder
        .append(7, 7, &events(7, 7, "e"))
        .expect("and appends go on");
    drop(folder);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_torn_record_is_cut_before_the_next_append_and_both_reopens_agree() {
    if std::env::var(CHILD).is_ok() {
        return;
    }
    // A record cut mid-JSON, and one cut inside a UTF-8 sequence ("é").
    torn_then_appended_reopens_twice("ascii", b"6 6 [{\"seq\":6,\"te");
    torn_then_appended_reopens_twice(
        "utf8",
        "6 6 [{\"seq\":6,\"text\":\"caf\u{e9}"
            .as_bytes()
            .split_last()
            .unwrap()
            .1,
    );
}

#[test]
fn a_whole_record_that_is_not_valid_refuses_the_folder() {
    if std::env::var(CHILD).is_ok() {
        return;
    }
    // A whole line that is not the next record (a seq gap), and one whose
    // events do not match its seqs: corruption, never silently dropped.
    for (name, line) in [
        ("gap", "7 7 [{\"seq\":7}]\n"),
        ("mismatch", "6 6 [{\"seq\":9}]\n"),
        ("garbage", "6 6 [{\"seq\":6,\"te6 6 [{\"seq\":6}]\n"),
    ] {
        let dir = torn_folder(name, line.as_bytes());
        // StoreError::Corrupt, matched by its name.
        let refused = SessionFolder::open(&dir)
            .err()
            .map(|error| format!("{error:?}"));
        assert!(
            refused.as_deref().is_some_and(|e| e.starts_with("Corrupt")),
            "{name}: {refused:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The child for the uncertain-write test: a file-size limit makes the next
/// append's write stop part-way (a partial write_all), the limit is lifted,
/// and the same append is tried again.
fn uncertain_child(dir: PathBuf) {
    let folder = SessionFolder::open(&dir).expect("open");
    folder.append(1, 1, &events(1, 1, "a")).unwrap();
    let len = std::fs::metadata(dir.join("events")).unwrap().len();
    // SAFETY: plain libc calls on this process: ignore SIGXFSZ (the write
    // then fails with EFBIG) and cap the soft file-size limit.
    let limit = unsafe {
        libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
        let mut limit: libc::rlimit = std::mem::zeroed();
        libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit);
        let capped = libc::rlimit {
            rlim_cur: len + 12,
            rlim_max: limit.rlim_max,
        };
        libc::setrlimit(libc::RLIMIT_FSIZE, &capped);
        limit
    };
    let first = folder.append(2, 2, &events(2, 2, "a long record cut part-way"));
    // SAFETY: as above; the original soft limit is within the hard one.
    unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &limit) };
    assert!(first.is_err(), "the capped write failed");
    // Retried on the same folder: an uncertain write stops the folder (it
    // never appends after a record that may be on disk in part).
    let again = folder.append(2, 2, &events(2, 2, "retried"));
    std::fs::write(
        dir.join("RESULT"),
        if again.is_err() {
            "stopped"
        } else {
            "appended"
        },
    )
    .unwrap();
}

#[test]
fn an_uncertain_write_stops_the_folder_and_the_reopen_is_valid() {
    if let Ok(dir) = std::env::var(CHILD) {
        if dir.ends_with("-uncertain") {
            uncertain_child(PathBuf::from(dir));
        }
        return;
    }
    let dir = std::env::temp_dir().join(format!("aero-store-{}-uncertain", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "an_uncertain_write_stops_the_folder_and_the_reopen_is_valid",
            "--test-threads=1",
        ])
        .env(CHILD, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run the child");
    assert!(status.success(), "the child failed");
    assert_eq!(
        std::fs::read_to_string(dir.join("RESULT")).unwrap(),
        "stopped"
    );
    let folder = SessionFolder::open(&dir).expect("the folder reopens");
    let stored = SessionFolder::load(&dir).expect("load");
    assert_eq!(stored.last_seq, 1, "{}", stored.journal_json);
    assert_eq!(seqs(&stored.journal_json), vec![1]);
    folder
        .append(2, 2, &events(2, 2, "after the reopen"))
        .expect("append after the reopen");
    drop(folder);
    let stored = SessionFolder::load(&dir).expect("load");
    assert_eq!(seqs(&stored.journal_json), vec![1, 2]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_and_interrupted_writers_never_publish_a_torn_image() {
    if std::env::var(CHILD).is_ok() {
        return;
    }
    let dir = scratch("images");
    let folder = Arc::new(SessionFolder::open(&dir).expect("open"));
    let mut bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/pixel.png"
    ))
    .unwrap();
    bytes.resize(8 << 20, 0x5a);
    for index in 0u64..30 {
        let len = bytes.len();
        bytes[len - 8..].copy_from_slice(&index.to_le_bytes());
        let image = Arc::new(bytes.clone());
        let expected = pm_aero_core::image_name(&image);
        // Writers of the same image started 1 ms apart, across the first
        // one's sync: some are still writing when it has published.
        let (done, first) = std::sync::mpsc::channel();
        let writers: Vec<_> = (0..16u64)
            .map(|k| {
                let (folder, image, done) = (folder.clone(), image.clone(), done.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(k));
                    if let Ok(name) = folder.put_image(&image) {
                        let _ = done.send(name);
                    }
                })
            })
            .collect();
        drop(done);
        // The moment one writer says the image is stored the journal may
        // name it, and a crash then (the other writers interrupted) leaves
        // the file as it is now: it must already be the image.
        let name = first.recv().expect("one writer stored it");
        assert_eq!(name, expected);
        let now = SessionFolder::image(&dir, &name).expect("published");
        assert!(
            pm_aero_core::image_name(&now) == name,
            "image {index}: the published file's {} bytes miss its name while other writers run",
            now.len()
        );
        for writer in writers {
            let _ = writer.join();
        }
        let after = SessionFolder::image(&dir, &name).expect("published");
        assert_eq!(
            pm_aero_core::image_name(&after),
            name,
            "image {index} after every writer"
        );
        let _ = std::fs::remove_file(dir.join("images").join(&name));
    }
    drop(folder);
    let _ = std::fs::remove_dir_all(&dir);
}
