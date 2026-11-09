//! Real speech through the real runtime and a real model. Ignored by default:
//! it needs the runtime folder (`PM_TRANSCRIBE_RUNTIME`: libtranscribe.dylib,
//! ggml, contract.json) and a downloaded model (`PM_TRANSCRIBE_MODEL`, a
//! catalog GGUF). Run it with `--ignored`; without those it fails.
use pm_transcribe::runtime::Runtime;
use std::{path::PathBuf, process::Command, sync::atomic::AtomicBool};

fn samples(text: &str) -> Vec<f32> {
    let out = std::env::temp_dir().join(format!("pm-transcribe-say-{}.wav", std::process::id()));
    let ok = Command::new("/usr/bin/say")
        .args(["--file-format=WAVE", "--data-format=LEF32@16000", "-o"])
        .arg(&out)
        .arg(text)
        .status()
        .unwrap()
        .success();
    assert!(ok, "say failed");
    let bytes = std::fs::read(&out).unwrap();
    std::fs::remove_file(&out).unwrap();
    // The `data` chunk of a float WAVE file.
    let at = bytes.windows(4).position(|w| w == b"data").unwrap();
    bytes[at + 8..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
#[test]
#[ignore = "needs PM_TRANSCRIBE_RUNTIME and PM_TRANSCRIBE_MODEL (a runtime folder and a downloaded model)"]
fn spoken_words_come_back_with_their_times() {
    let runtime = PathBuf::from(
        std::env::var_os("PM_TRANSCRIBE_RUNTIME").expect("set PM_TRANSCRIBE_RUNTIME"),
    );
    let model =
        PathBuf::from(std::env::var_os("PM_TRANSCRIBE_MODEL").expect("set PM_TRANSCRIBE_MODEL"));
    let runtime = Runtime::load(&runtime).unwrap();
    let mut model = runtime.model(&model, 4).unwrap();
    let audio = samples("The quick brown fox jumps over the lazy dog.");
    let result = model.run(&audio, None, &AtomicBool::new(false)).unwrap();
    eprintln!("heard: {:?} ({} words)", result.text, result.words.len());
    let text = result.text.to_lowercase();
    assert!(text.contains("quick brown fox"), "{result:?}");
    assert!(result.words.len() >= 8, "{result:?}");
    let duration_ms = (audio.len() * 1000 / 16_000) as i64;
    for pair in result.words.windows(2) {
        assert!(pair[0].t0_ms <= pair[1].t0_ms, "{result:?}");
    }
    assert!(
        result.words.iter().all(|w| w.t1_ms <= duration_ms + 200),
        "{result:?}"
    );
    // A raised flag stops the run.
    let cancelled = model.run(&audio, None, &AtomicBool::new(true));
    assert!(
        matches!(cancelled, Err(pm_transcribe::runtime::Error::Cancelled)),
        "{cancelled:?}"
    );
}
