//! Today's own FFmpeg for AAC and Opus (`src/chromium.rs`). Its own test
//! binary: the library is located once per process.

/// Electron's own FFmpeg (`POWERMOVE_CHROMIUM_FFMPEG`, else the checkout's
/// Electron), as the app's engine processes find it already loaded.
fn chromium_ffmpeg() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("POWERMOVE_CHROMIUM_FFMPEG") {
        return Some(p.into());
    }
    let modules =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../node_modules/.bun");
    std::fs::read_dir(modules).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        let lib = e.path().join("node_modules/electron/dist/Electron.app/Contents/Frameworks/Electron Framework.framework/Versions/A/Libraries/libffmpeg.dylib");
        (name.starts_with("electron@") && lib.is_file()).then_some(lib)
    })
}

/// AAC and Opus decode through today's own FFmpeg, bit for bit: the
/// SHA-256 of each fixture's planar decode equals today's `decodeAudioData`
/// output captured from the app (Electron 44), with blocks decoded last
/// first so the decoder restarts and serves blocks it passed.
#[test]
fn aac_and_opus_decode_as_today() {
    use sha2::{Digest, Sha256};
    let Some(lib) = chromium_ffmpeg() else {
        eprintln!("no Electron libffmpeg in this checkout; skipping");
        return;
    };
    // SAFETY: set before any thread opens a source in this process.
    unsafe { std::env::set_var("POWERMOVE_CHROMIUM_FFMPEG", &lib) };
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures");
    const TODAY_AAC: &str = "3846f36f30ed0abb7df89dbb3385b87647cd4735ec5cd5035aa1e0ae21659556";
    for (file, want) in [
        ("tone.m4a", TODAY_AAC),
        ("h264-aac.mp4", TODAY_AAC),
        ("h264-aac.mov", TODAY_AAC),
        (
            "tone.opus",
            "73246459b0048228c26e9d78a05c688b338f6e737ef7917d30c000e5a5bd23db",
        ),
    ] {
        let cache = pm_audio::SourceCache::new(64 << 20, 0);
        cache.set_blocking_lengths(true);
        let source = cache.open(&fixtures.join(file), None).unwrap();
        assert!(
            pm_audio::chromium::library().is_some(),
            "today's decoder loads"
        );
        let n = source.length().unwrap() as usize;
        let mut planar = vec![vec![0f32; n]; source.channels()];
        let b = pm_audio::source::BLOCK;
        for k in (0..n.div_ceil(b)).rev() {
            let (lo, m) = (k * b, (n - k * b).min(b));
            let mut views: Vec<&mut [f32]> =
                planar.iter_mut().map(|c| &mut c[lo..lo + m]).collect();
            cache.read(&source, lo as i64, m, &mut views, true).unwrap();
        }
        let mut h = Sha256::new();
        for c in &planar {
            for v in c {
                h.update(v.to_le_bytes());
            }
        }
        let got: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, want, "{file}");
        cache.shutdown();
    }
}

/// Playback never waits for today's length: the first open takes the
/// container's length and counts today's on a background thread; a later
/// open of the same file takes today's.
#[test]
fn todays_length_is_counted_off_the_opening_thread() {
    let Some(lib) = chromium_ffmpeg() else {
        eprintln!("no Electron libffmpeg in this checkout; skipping");
        return;
    };
    // SAFETY: set before any thread opens a source in this process.
    unsafe { std::env::set_var("POWERMOVE_CHROMIUM_FFMPEG", &lib) };
    let dir = tempfile::tempdir().unwrap();
    // A copy: the per-file memo must not already know it.
    let file = dir.path().join("tone-copy.opus");
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../apps/desktop/e2e/fixtures");
    std::fs::copy(fixtures.join("tone.opus"), &file).unwrap();
    let cache = pm_audio::SourceCache::new(16 << 20, 0);
    let first = cache.open(&file, None).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while pm_audio::chromium::known_length(&file).is_none() && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(pm_audio::chromium::known_length(&file), Some(96_000));
    let again = cache.open(&file, None).unwrap();
    assert_eq!(again.length(), Some(96_000), "today's length once counted");
    let _ = first;
    cache.shutdown();
}
