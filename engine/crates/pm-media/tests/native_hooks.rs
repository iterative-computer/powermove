//! Same-source B-frame and edit-list qualification; all pixels/PTS pass through
//! the existing sequential/random-access oracle, for every available backend.
mod common;
use common::*;
use pm_media::vt::DecodePurpose;

#[test]
fn b_frames_and_trimmed_mp4_keep_exact_sequential_frames_and_timestamps() {
    let original = fixtures_dir().join("h264-aac.mov");
    let source = pm_media::decode::service::MediaSource::open(&original).unwrap();
    assert!(
        source.index.samples.windows(2).any(|s| s[0].cts > s[1].cts),
        "fixture must contain actual presentation reordering"
    );
    let dir = tempfile::tempdir().unwrap();
    let trimmed = dir.path().join("trimmed-bframes.mp4");
    let result = std::process::Command::new(ffmpeg_bin())
        .args(["-v", "error", "-ss", "0.1", "-i"])
        .arg(&original)
        .args([
            "-t",
            "0.8",
            "-map",
            "0:v:0",
            "-c",
            "copy",
            "-avoid_negative_ts",
            "disabled",
            "-use_editlist",
            "1",
        ])
        .arg(&trimmed)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let edited = pm_media::decode::service::MediaSource::open(&trimmed).unwrap();
    assert!(
        edited.index.stats.edit_list.is_some(),
        "remux must contain an edit list"
    );
    for purpose in [DecodePurpose::Display, DecodePurpose::Export] {
        for file in [&original, &trimmed] {
            let report = run_file(file, None, purpose);
            let failures = check(&report);
            assert!(failures.is_empty(), "{}", failures.join("\n"));
            assert!(report.runs.iter().all(|run| run.frames > 0));
            eprintln!("{}", summary(&report));
        }
    }
}
