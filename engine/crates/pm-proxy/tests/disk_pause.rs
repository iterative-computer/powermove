#![cfg(target_os = "macos")]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use pm_protocol::priority::{ProxyFormat, ProxySize};
use pm_proxy::{
    GenerationControl, GenerationRequest, PauseReason, ProxyStore, generate_with_disk_probe,
};
use tempfile::tempdir;

#[test]
fn low_disk_during_transcode_pauses_then_resumes_the_same_generation() {
    let codec_dir =
        std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    let ffmpeg = codec_dir.join("bin/ffmpeg");
    let ffprobe = codec_dir.join("bin/ffprobe");
    let temp = tempdir().unwrap();
    let source = temp.path().join("source.mov");
    let built = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24:duration=2",
            "-c:v",
            "prores_videotoolbox",
            "-profile:v",
            "proxy",
            "-pix_fmt",
            "p216le",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let hash = blake3::hash(&fs::read(&source).unwrap())
        .to_hex()
        .to_string();
    let store = ProxyStore::open(temp.path().join("store")).unwrap();
    let cancel = AtomicBool::new(false);
    let calls = AtomicUsize::new(0);
    let resume = AtomicBool::new(false);
    let (status_tx, status_rx) = mpsc::channel();
    let gib = 1u64 << 30;

    std::thread::scope(|scope| {
        // The first check admits the transcode. The next check is inside its
        // child loop and reports a full disk until this test releases it.
        let disk_space = |_: &Path| {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            Ok((
                if call == 0 || resume.load(Ordering::SeqCst) {
                    40 * gib
                } else {
                    19 * gib
                },
                100 * gib,
            ))
        };
        let on_pause = |reason| {
            let _ = status_tx.send(reason);
        };
        let source_ref = &source;
        let hash_ref = &hash;
        let ffmpeg_ref = &ffmpeg;
        let ffprobe_ref = &ffprobe;
        let store_ref = &store;
        let cancel_ref = &cancel;
        let worker = scope.spawn(move || {
            generate_with_disk_probe(
                GenerationRequest {
                    source: source_ref,
                    source_hash: hash_ref,
                    format: ProxyFormat::ProResProxy,
                    size: ProxySize::Half,
                    ffmpeg: ffmpeg_ref,
                    ffprobe: ffprobe_ref,
                    store: store_ref,
                    control: GenerationControl {
                        cancel: cancel_ref,
                        pause: &|| false,
                        on_pause: &on_pause,
                    },
                },
                &disk_space,
            )
        });

        let paused = status_rx.recv_timeout(Duration::from_secs(15));
        // Release even on a failed assertion, so scoped thread cleanup cannot
        // wait forever and the tempfile guard always removes the test files.
        resume.store(true, Ordering::SeqCst);
        let generated = worker.join().unwrap().unwrap();
        assert_eq!(paused.unwrap(), Some(PauseReason::LowDisk));
        assert_eq!(
            status_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            None
        );
        assert!(calls.load(Ordering::SeqCst) >= 3);
        assert!(!generated.reused);
        assert!(generated.stored.path.is_file());
        assert!(generated.stored.byte_size > 0);
        assert!(
            store
                .find_for_source(&hash, Some(ProxyFormat::ProResProxy), Some(ProxySize::Half))
                .unwrap()
                .is_some()
        );
    });
}
