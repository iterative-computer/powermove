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
fn restored_sources_decode_lazily_and_count_length_on_first_read() {
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
    assert_eq!(
        pm_audio::chromium::resident_bytes_of(&file),
        None,
        "metadata open must not decode"
    );
    read_block(&cache, &first, 0, first.length().unwrap() as usize);
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

/// A stereo AAC file of `seconds` (a chirp over noise, so every block
/// differs) in `dir`, encoded by our FFmpeg.
fn long_aac(dir: &std::path::Path, seconds: u32, name: &str) -> std::path::PathBuf {
    let file = dir.join(name);
    let ffmpeg = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg");
    let status = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!(
            "aevalsrc=0.3*sin(2*PI*(200+t*3)*t)+0.05*(random(0)-0.5)|0.3*sin(2*PI*(300+t*2)*t):s=48000:d={seconds}"
        ))
        .args(["-c:a", "aac", "-b:a", "192k"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success(), "ffmpeg failed");
    file
}

fn read_block(
    cache: &pm_audio::SourceCache,
    source: &std::sync::Arc<pm_audio::source::Source>,
    k: usize,
    n: usize,
) -> Vec<Vec<f32>> {
    let b = pm_audio::source::BLOCK;
    let m = (n - k * b).min(b);
    let mut planar = vec![vec![0f32; m]; source.channels()];
    let mut views: Vec<&mut [f32]> = planar.iter_mut().map(|c| &mut c[..]).collect();
    cache
        .read(source, (k * b) as i64, m, &mut views, true)
        .unwrap();
    planar
}

/// Issue A8 (review round 9): AAC is the common video soundtrack, and an
/// unprepared seek on it must not wait for a decode from the file's start.
/// Today decodes the whole file once (`decodeAudioData`) and plays the
/// buffer; here every open of a file reads its one shared whole decode, so
/// a seek anywhere (forward, or far back) is a copy once the decode has
/// passed it, the samples equal one sequential decode bit for bit, and the
/// memory is the decoded samples (never the file's bytes), freed with the
/// last open.
#[test]
fn seeks_read_the_shared_whole_decode() {
    let Some(lib) = chromium_ffmpeg() else {
        eprintln!("no Electron libffmpeg in this checkout; skipping");
        return;
    };
    // SAFETY: set before any thread opens a source in this process.
    unsafe { std::env::set_var("POWERMOVE_CHROMIUM_FFMPEG", &lib) };
    let dir = tempfile::tempdir().unwrap();
    // Five minutes: within today's 128 MiB budget, so today plays it.
    let file = long_aac(dir.path(), 300, "five-minutes.m4a");
    let b = pm_audio::source::BLOCK;

    // The sequential decode, read in order.
    let opened = std::time::Instant::now();
    let mut seq = pm_audio::chromium::ChromiumDecoder::open(&file, 2)
        .unwrap()
        .expect("today's decoder");
    let n = seq.count_frames().unwrap() as usize;
    eprintln!("whole decode from open: {:?}", opened.elapsed());
    assert!(n > 299 * 48_000, "{n}");
    let mut reference: Vec<Vec<f32>> = (0..2).map(|_| Vec::with_capacity(n)).collect();
    seq.seek_anchored(0).unwrap();
    while seq.read(&mut reference, b).unwrap() > 0 {}
    assert_eq!(reference[0].len(), n);

    // Memory: the decoded samples in whole blocks, not the file.
    let resident = pm_audio::chromium::resident_bytes_of(&file).unwrap();
    assert_eq!(
        resident, 0,
        "shared PCM is on disk; blocks alone consume the RAM budget"
    );
    let file_bytes = std::fs::metadata(&file).unwrap().len() as usize;
    eprintln!("resident {resident} B for a {file_bytes} B file of {n} frames");

    // A cold cache (small budget, so blocks are evicted and read again):
    // the last block first, then far back, then forward.
    let cache = pm_audio::SourceCache::new(4 * b * 2 * 4, 0);
    cache.set_blocking_lengths(true);
    let source = cache.open(&file, None).unwrap();
    assert_eq!(source.length(), Some(n as u64), "today's length");
    let blocks = n.div_ceil(b);
    let at = |seconds: usize| seconds * 48_000 / b;
    let order = [
        blocks - 1,
        at(240),
        at(200),
        0,
        at(120),
        at(280),
        at(10),
        blocks - 2,
    ];
    let mut worst = std::time::Duration::ZERO;
    for &k in &order {
        let started = std::time::Instant::now();
        let got = read_block(&cache, &source, k, n);
        worst = worst.max(started.elapsed());
        for (c, ch) in got.iter().enumerate() {
            let want = &reference[c][k * b..k * b + ch.len()];
            assert!(
                ch.iter().zip(want).all(|(a, w)| a.to_bits() == w.to_bits()),
                "block {k} channel {c} equals the sequential decode"
            );
        }
    }
    eprintln!("slowest cold block read: {worst:?}");
    // A3's 120 ms bound, with a wide margin for a loaded machine: a copy.
    assert!(worst < std::time::Duration::from_millis(40), "{worst:?}");
    cache.shutdown();
    drop(source);
    drop(cache);
    drop(seq);
    assert_eq!(
        pm_audio::chromium::resident_bytes_of(&file),
        None,
        "freed with the last open"
    );
}

/// A file longer than today plays (its decoded buffer is past today's
/// 128 MiB budget, so today refuses it) decodes with Powermove's random-
/// access decoder: no whole decode is held for it.
#[test]
fn files_today_refuses_keep_random_access() {
    let Some(lib) = chromium_ffmpeg() else {
        eprintln!("no Electron libffmpeg in this checkout; skipping");
        return;
    };
    // SAFETY: set before any thread opens a source in this process.
    unsafe { std::env::set_var("POWERMOVE_CHROMIUM_FFMPEG", &lib) };
    let dir = tempfile::tempdir().unwrap();
    let file = long_aac(dir.path(), 420, "seven-minutes.m4a");
    let cache = pm_audio::SourceCache::new(16 << 20, 0);
    let source = cache.open(&file, None).unwrap();
    assert!(!pm_audio::chromium::plays_today(&source.info));
    assert_eq!(pm_audio::chromium::resident_bytes_of(&file), None);
    let n = source.length().unwrap() as usize;
    let b = pm_audio::source::BLOCK;
    let started = std::time::Instant::now();
    let got = read_block(&cache, &source, (n / b) - 1, n);
    eprintln!("cold read near the end: {:?}", started.elapsed());
    assert!(got[0].iter().any(|v| *v != 0.0));
    assert_eq!(pm_audio::chromium::resident_bytes_of(&file), None);
    cache.shutdown();
}

/// The presenter uses one cache for every file in its mix. Opening a restored
/// project must be lazy, and ten decodes together must obey the RAM budget.
#[test]
fn total_decode_memory_across_files_obeys_the_presenter_budget() {
    let Some(lib) = chromium_ffmpeg() else { return };
    // SAFETY: all tests set the same library path before its OnceLock is read.
    unsafe { std::env::set_var("POWERMOVE_CHROMIUM_FFMPEG", &lib) };
    let dir = tempfile::tempdir().unwrap();
    let encoded = long_aac(dir.path(), 45, "seed.m4a");
    let budget = 128 << 20;
    let cache = pm_audio::SourceCache::new(budget, 0);
    let files: Vec<_> = (0..10)
        .map(|i| {
            let file = dir.path().join(format!("source-{i}.m4a"));
            std::fs::copy(&encoded, &file).unwrap();
            file
        })
        .collect();
    let sources: Vec<_> = files.iter().map(|p| cache.open(p, None).unwrap()).collect();
    for file in &files {
        assert_eq!(
            pm_audio::chromium::resident_bytes_of(file),
            None,
            "restored metadata is lazy"
        );
    }
    let mut total_decoded = 0;
    for source in &sources {
        let n = source.length().unwrap() as usize;
        for block in 0..n.div_ceil(pm_audio::source::BLOCK) {
            let _ = read_block(&cache, source, block, n);
        }
        total_decoded += n * source.channels() * 4;
        assert!(
            cache.stats().bytes <= budget,
            "aggregate block LRU: {:?}",
            cache.stats()
        );
    }
    assert!(
        total_decoded > budget,
        "the test must exceed the budget across files"
    );
    assert_eq!(
        pm_audio::chromium::resident_bytes(),
        0,
        "no hidden whole-file sample allocation"
    );
    assert!(cache.stats().evicted_blocks > 0);
    // An inactive old block was evicted; the newest remains warm.
    let before = cache.stats();
    cache.block(&sources[0], 0, true).unwrap();
    assert!(cache.stats().misses > before.misses);
    let pinned = cache
        .block(sources.last().unwrap(), 0, true)
        .unwrap()
        .unwrap();
    cache.memory_pressure();
    assert!(cache.stats().bytes > 0, "an in-use block stays pinned");
    drop(pinned);
    cache.memory_pressure();
    assert_eq!(cache.stats().bytes, 0);
    drop(sources);
    cache.shutdown();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while files
        .iter()
        .any(|p| pm_audio::chromium::resident_bytes_of(p).is_some())
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the last mix releases every spool"
        );
        std::thread::yield_now();
    }
}
