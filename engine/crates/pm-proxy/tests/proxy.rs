use std::fs;
use std::io::Write;
use std::sync::atomic::AtomicBool;

use pm_protocol::priority::{ProxyFormat, ProxySize};
use pm_proxy::{
    AutomaticInput, DecodeMeasurement, GenerationControl, GenerationRequest, MachineProfile,
    MediaAvailability, MediaUse, PlaybackMoment, ProxyRecipe, ProxyStore, Representation,
    RepresentationIntent, ResolutionIntent, choose_representation, disk_guard, generate,
    source_for,
};
use tempfile::{NamedTempFile, tempdir};

#[test]
fn recipe_key_and_store_survive_relink_and_reopen() {
    let temp = tempdir().unwrap();
    let store = ProxyStore::open(temp.path()).unwrap();
    let recipe = ProxyRecipe::new(
        ProxyFormat::ProResProxy,
        ProxySize::Automatic,
        1920,
        1080,
        10,
        false,
    )
    .unwrap();
    let hash = blake3::hash(b"source bytes").to_hex().to_string();
    let mut movie = NamedTempFile::new_in(temp.path()).unwrap();
    movie.write_all(b"validated movie bytes").unwrap();
    let saved = store.commit(&hash, &recipe, movie).unwrap();
    assert_eq!(
        store.lookup(&hash, &recipe).unwrap().unwrap().path,
        saved.path
    );
    assert_eq!(
        store
            .lookup(&hash.to_uppercase(), &recipe)
            .unwrap()
            .unwrap()
            .path,
        saved.path
    );
    let mut obsolete_recipe = recipe.clone();
    obsolete_recipe.version = 1;
    let mut obsolete_media = NamedTempFile::new_in(temp.path()).unwrap();
    obsolete_media.write_all(b"old encoding algorithm").unwrap();
    store
        .commit(&hash, &obsolete_recipe, obsolete_media)
        .unwrap();
    let reopened = ProxyStore::open(temp.path()).unwrap();
    let relinked = reopened
        .find_for_source(&hash, None, None)
        .unwrap()
        .unwrap();
    assert_eq!(relinked.blake3, saved.blake3);
    assert!(
        reopened
            .find_for_source(&hash.to_uppercase(), None, None)
            .unwrap()
            .is_some()
    );
    fs::write(&saved.path, b"corrupt").unwrap();
    assert!(reopened.lookup(&hash, &recipe).is_err());
    assert!(
        reopened
            .find_for_source(&hash, None, None)
            .unwrap()
            .is_none()
    );
    reopened.remove(&hash, &recipe).unwrap();
    assert!(reopened.lookup(&hash, &recipe).unwrap().is_none());
}

#[test]
fn disk_guard_uses_larger_of_20_gib_and_ten_percent() {
    let gib = 1u64 << 30;
    assert!(!disk_guard(19 * gib, 100 * gib));
    assert!(disk_guard(20 * gib, 100 * gib));
    assert!(!disk_guard(29 * gib, 300 * gib));
    assert!(disk_guard(30 * gib, 300 * gib));
}

#[test]
fn deep_source_recipe_uses_4444_without_discarding_depth_fact() {
    let recipe =
        ProxyRecipe::new(ProxyFormat::Hevc10, ProxySize::Half, 101, 57, 16, false).unwrap();
    assert_eq!(recipe.source_depth, 16);
    assert_eq!(recipe.codec(), "prores_4444");
    assert_eq!(recipe.width % 2, 0);
    assert_eq!(recipe.height % 2, 0);
}

#[test]
fn policy_respects_boundaries_offline_originals_and_export_safety() {
    let profile = MachineProfile {
        gpu_frame_p95_ms: 4.0,
        gpu_headroom_ms: 12.667,
        disk_write_mib_s: 500.0,
        on_battery: false,
    };
    let mut input = AutomaticInput {
        intent: RepresentationIntent::Automatic,
        resolution: ResolutionIntent::Automatic,
        moment: PlaybackMoment::Start,
        availability: MediaAvailability::Online,
        current: Representation::Original,
        proxy_ready: true,
        reduced_half_available: false,
        reduced_quarter_available: false,
        decode: DecodeMeasurement {
            original_p95_ms: 60.0,
            reduced_half_p95_ms: None,
            reduced_quarter_p95_ms: None,
            proxy_p95_ms: Some(4.0),
        },
        profile,
        fps: 30.0,
        viewer_size: (960, 540),
        source_size: (3840, 2160),
    };
    assert_eq!(
        choose_representation(input).representation,
        Some(Representation::Proxy)
    );
    input.reduced_half_available = true;
    input.decode.reduced_half_p95_ms = Some(5.0);
    assert_eq!(
        choose_representation(input).representation,
        Some(Representation::ReducedHalf)
    );
    input.reduced_quarter_available = true;
    input.decode.reduced_quarter_p95_ms = Some(3.0);
    assert_eq!(
        choose_representation(input).representation,
        Some(Representation::ReducedQuarter)
    );
    // A measured original that fits is the exact source even on a small
    // canvas; the decoder's reduced picture has different texels.
    input.decode.original_p95_ms = 5.0;
    assert_eq!(
        choose_representation(input).representation,
        Some(Representation::Original)
    );
    input.decode.original_p95_ms = 60.0;
    input.reduced_half_available = false;
    input.reduced_quarter_available = false;
    input.moment = PlaybackMoment::Continuing;
    input.current = Representation::Original;
    assert_eq!(
        choose_representation(input).representation,
        Some(Representation::Original)
    );
    input.moment = PlaybackMoment::Paused;
    assert_eq!(
        choose_representation(input).representation,
        Some(Representation::Original)
    );
    input.resolution = ResolutionIntent::Half;
    input.current = Representation::ReducedQuarter;
    assert_eq!(
        choose_representation(input).representation,
        Some(Representation::Original)
    );
    input.intent = RepresentationIntent::Originals;
    input.availability = MediaAvailability::OriginalOfflineProxyAvailable;
    assert_eq!(choose_representation(input).representation, None);
    input.intent = RepresentationIntent::Automatic;
    assert_eq!(choose_representation(input).representation, None);
    assert_eq!(choose_representation(input).offline, MediaAvailability::OriginalOfflineProxyAvailable);
    input.moment = PlaybackMoment::Start;
    assert_eq!(choose_representation(input).representation, Some(Representation::Proxy));
    input.moment = PlaybackMoment::Paused;
    input.intent = RepresentationIntent::Proxies;
    assert_eq!(choose_representation(input).representation, Some(Representation::Proxy));
    assert!(source_for(MediaUse::Export, Representation::Proxy, false).is_err());
    assert_eq!(
        source_for(MediaUse::Analysis, Representation::Proxy, true).unwrap(),
        Representation::Original
    );
    assert_eq!(
        source_for(MediaUse::DraftExport, Representation::Proxy, false).unwrap(),
        Representation::Proxy
    );
}

#[cfg(target_os = "macos")]
#[test]
fn synthetic_video_with_pcm_audio_generates_validated_prores_proxy() {
    use std::path::PathBuf;
    use std::process::Command;

    let codec_dir = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
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
            "testsrc2=size=128x72:rate=24000/1001:duration=0.5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=400:sample_rate=48000:duration=0.5",
            "-c:v",
            "prores_videotoolbox",
            "-profile:v",
            "proxy",
            "-pix_fmt",
            "p216le",
            "-c:a",
            "pcm_s24le",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-timecode",
            "01:00:00:00",
            "-metadata",
            "reel_name=test_reel",
            "-movflags",
            "use_metadata_tags",
            "-shortest",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let source_tags = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(&source)
        .output()
        .unwrap();
    let source_tags: serde_json::Value = serde_json::from_slice(&source_tags.stdout).unwrap();
    assert_eq!(source_tags["streams"][0]["tags"]["timecode"], "01:00:00:00");
    assert_eq!(source_tags["format"]["tags"]["reel_name"], "test_reel");
    let hash = blake3::hash(&fs::read(&source).unwrap())
        .to_hex()
        .to_string();
    let store = ProxyStore::open(temp.path().join("store")).unwrap();
    let cancel = AtomicBool::new(false);
    for format in [
        ProxyFormat::ProResProxy,
        ProxyFormat::ProResLt,
        ProxyFormat::Hevc10,
    ] {
        let request = || GenerationRequest {
            source: &source,
            source_hash: &hash,
            format,
            size: ProxySize::Half,
            ffmpeg: &ffmpeg,
            ffprobe: &ffprobe,
            store: &store,
            control: GenerationControl {
                cancel: &cancel,
                pause: &|| false,
                on_pause: &|_| {},
            },
        };
        let result = generate(request()).unwrap();
        assert!(!result.reused);
        assert!(result.stored.byte_size > 0);
        let again = generate(request()).unwrap();
        assert!(again.reused);
        assert_eq!(again.stored.path, result.stored.path);
    }
    let replaced = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=128x72:r=24000/1001:d=0.5",
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
        replaced.status.success(),
        "{}",
        String::from_utf8_lossy(&replaced.stderr)
    );
    let stale = generate(GenerationRequest {
        source: &source,
        source_hash: &hash,
        format: ProxyFormat::ProResProxy,
        size: ProxySize::Half,
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        store: &store,
        control: GenerationControl {
            cancel: &cancel,
            pause: &|| false,
            on_pause: &|_| {},
        },
    });
    assert!(matches!(stale, Err(pm_proxy::ProxyError::InvalidMedia(_))));
}

#[cfg(target_os = "macos")]
#[test]
fn alpha_source_forces_prores_4444_even_with_hevc_preference() {
    use std::path::PathBuf;
    use std::process::Command;

    let codec_dir = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    let ffmpeg = codec_dir.join("bin/ffmpeg");
    let ffprobe = codec_dir.join("bin/ffprobe");
    let temp = tempdir().unwrap();
    let source = temp.path().join("alpha.mov");
    let built = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red@0.5:s=128x72:r=24:d=0.5,format=yuva444p10le",
            "-c:v",
            "prores_videotoolbox",
            "-profile:v",
            "4444",
            "-pix_fmt",
            "ayuv64le",
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
    let output = generate(GenerationRequest {
        source: &source,
        source_hash: &hash,
        format: ProxyFormat::Hevc10,
        size: ProxySize::Half,
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        store: &store,
        control: GenerationControl {
            cancel: &cancel,
            pause: &|| false,
            on_pause: &|_| {},
        },
    })
    .unwrap();
    assert_eq!(output.stored.recipe.codec(), "prores_4444");
    assert!(output.stored.recipe.source_has_alpha);
}

#[cfg(target_os = "macos")]
#[test]
fn float64_pcm_source_keeps_its_coding_and_samples() {
    use std::path::PathBuf;
    use std::process::Command;

    let codec_dir = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    let ffmpeg = codec_dir.join("bin/ffmpeg");
    let ffprobe = codec_dir.join("bin/ffprobe");
    let temp = tempdir().unwrap();
    let source = temp.path().join("float.mov");
    let built = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x72:rate=24:duration=0.5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=917:sample_rate=48000:duration=0.5",
            "-c:v",
            "prores_videotoolbox",
            "-profile:v",
            "proxy",
            "-pix_fmt",
            "p216le",
            "-c:a",
            "pcm_f64le",
            "-shortest",
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
    let output = generate(GenerationRequest {
        source: &source,
        source_hash: &hash,
        format: ProxyFormat::ProResProxy,
        size: ProxySize::Half,
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        store: &store,
        control: GenerationControl {
            cancel: &cancel,
            pause: &|| false,
            on_pause: &|_| {},
        },
    })
    .unwrap();
    let probed = Command::new(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "json",
        ])
        .arg(&output.stored.path)
        .output()
        .unwrap();
    let probed: serde_json::Value = serde_json::from_slice(&probed.stdout).unwrap();
    assert!(
        probed["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["codec_name"] == "pcm_f64le")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn variable_frame_intervals_keep_exact_pts() {
    use std::path::PathBuf;
    use std::process::Command;

    let codec_dir = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    let ffmpeg = codec_dir.join("bin/ffmpeg");
    let ffprobe = codec_dir.join("bin/ffprobe");
    let temp = tempdir().unwrap();
    let source = temp.path().join("variable.mov");
    let built = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x72:rate=10:duration=0.7",
            "-vf",
            r"setpts=PTS+gte(N\,3)",
            "-fps_mode",
            "passthrough",
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
    let output = generate(GenerationRequest {
        source: &source,
        source_hash: &hash,
        format: ProxyFormat::ProResProxy,
        size: ProxySize::Half,
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        store: &store,
        control: GenerationControl {
            cancel: &cancel,
            pause: &|| false,
            on_pause: &|_| {},
        },
    })
    .unwrap();
    assert!(output.stored.byte_size > 0);
}

#[cfg(target_os = "macos")]
#[test]
fn odd_source_dimensions_retain_display_aspect() {
    use std::path::PathBuf;
    use std::process::Command;

    let codec_dir = PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh"));
    let ffmpeg = codec_dir.join("bin/ffmpeg");
    let ffprobe = codec_dir.join("bin/ffprobe");
    let temp = tempdir().unwrap();
    let source = temp.path().join("odd.mov");
    let built = Command::new(&ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=641x359:rate=12:duration=0.25",
            "-c:v",
            "png",
            "-pix_fmt",
            "rgb24",
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
    let output = generate(GenerationRequest {
        source: &source,
        source_hash: &hash,
        format: ProxyFormat::ProResProxy,
        size: ProxySize::Half,
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        store: &store,
        control: GenerationControl {
            cancel: &cancel,
            pause: &|| false,
            on_pause: &|_| {},
        },
    })
    .unwrap();
    assert_eq!(output.stored.recipe.width % 2, 0);
    assert_eq!(output.stored.recipe.height % 2, 0);
}
