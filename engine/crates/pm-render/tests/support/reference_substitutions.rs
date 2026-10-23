//! Explicit owner decisions changing a reference, never its comparison bound.
//! The original sample path stays available to historical diagnostics.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{Sample, repo_root, sha256_hex};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Substitution {
    id: String,
    owner_decision: String,
    reason: String,
    original: FileRecord,
    corrected: FileRecord,
    evidence: Vec<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRecord {
    file: PathBuf,
    bytes: u64,
    sha256: String,
}

fn checked_file(capture: &Path, record: &FileRecord) -> Result<(PathBuf, Vec<u8>), String> {
    if record.file.is_absolute() || record.file.as_os_str().is_empty() {
        return Err("reference file must be a nonempty relative path".into());
    }
    let path = capture
        .join(&record.file)
        .canonicalize()
        .map_err(|error| format!("reference file {}: {error}", record.file.display()))?;
    if !path.starts_with(capture) || !path.is_file() {
        return Err(format!(
            "reference file escapes capture or is not a file: {}",
            path.display()
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() as u64 != record.bytes || sha256_hex(&bytes) != record.sha256 {
        return Err(format!(
            "reference size or SHA-256 mismatch: {}",
            path.display()
        ));
    }
    Ok((path, bytes))
}

fn rgba(bytes: &[u8], path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if (reader.info().width, reader.info().height) != (480, 270) {
        return Err(format!(
            "COMPCLOSE-1 reference must be 480x270: {}",
            path.display()
        ));
    }
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("PNG has no output size")?
    ];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!(
            "reference must be an 8-bit RGBA PNG: {}",
            path.display()
        ));
    }
    pixels.truncate(info.buffer_size());
    Ok((info.width, info.height, pixels))
}

/// Returns the original pixels unless this exact sample carries a valid,
/// owner-authorized COMPCLOSE-1 substitution. Present invalid metadata errors;
/// it never falls back to either image or changes a comparison bound.
pub fn opaque_reference(
    capture: &Path,
    project_name: &str,
    sample: &Sample,
) -> Result<(u32, u32, Vec<u8>), String> {
    let Some(raw) = &sample.reference_substitution else {
        return Ok(super::read_png(&sample.png));
    };
    let metadata: Substitution =
        serde_json::from_str(&pm_model::json::write::to_canonical_string(raw))
            .map_err(|error| format!("invalid referenceSubstitution: {error}"))?;
    if metadata.id != "COMPCLOSE-1"
        || metadata.owner_decision != "2026-10-19"
        || project_name != "motion/blur-video"
        || ![0.5f64.to_bits(), 1.2f64.to_bits()].contains(&sample.time.to_bits())
        || (sample.width, sample.height) != (480, 270)
        || metadata.reason.trim().is_empty()
        || metadata.evidence.is_empty()
    {
        return Err("referenceSubstitution is outside the exact COMPCLOSE-1 owner decision".into());
    }
    let capture = capture
        .canonicalize()
        .map_err(|error| format!("capture: {error}"))?;
    let (original_path, original) = checked_file(&capture, &metadata.original)?;
    let sample_path = sample
        .png
        .canonicalize()
        .map_err(|error| format!("original sample: {error}"))?;
    if original_path != sample_path {
        return Err("referenceSubstitution.original does not match Sample.png".into());
    }
    // A nested worktree lives inside the public checkout on this machine.
    // Exclude both that worktree and any containing checkout from evidence.
    let repository = repo_root();
    let roots: Vec<_> = repository
        .ancestors()
        .filter(|path| *path == repository.as_path() || path.join(".git").is_dir())
        .collect();
    for evidence in &metadata.evidence {
        if !evidence.is_absolute() {
            return Err("reference substitution evidence must be absolute".into());
        }
        let canonical = evidence
            .canonicalize()
            .map_err(|error| format!("evidence {}: {error}", evidence.display()))?;
        if !canonical.is_file()
            || roots
                .iter()
                .any(|root| evidence.starts_with(root) || canonical.starts_with(root))
        {
            return Err(format!(
                "evidence must be an existing file outside the repository: {}",
                evidence.display()
            ));
        }
        // Evidence travels with the capture; a temporary or other outside
        // file would vanish and turn this gate into a later failure.
        if !canonical.starts_with(&capture) {
            return Err(format!(
                "evidence must be inside the capture: {}",
                evidence.display()
            ));
        }
    }
    // Decode the same bytes whose hashes were checked; no second file read
    // can substitute different pixels between verification and comparison.
    rgba(&original, &original_path)?;
    let (corrected_path, corrected) = checked_file(&capture, &metadata.corrected)?;
    let pixels = rgba(&corrected, &corrected_path)?;
    eprintln!(
        "reference substitution {} decision={} project={} t={} reason={:?}; original={} sha256={}; corrected={} sha256={}",
        metadata.id,
        metadata.owner_decision,
        project_name,
        sample.time,
        metadata.reason,
        original_path.display(),
        metadata.original.sha256,
        corrected_path.display(),
        metadata.corrected.sha256,
    );
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Own every fixture path from creation onward, including on panics.
    /// No additional dev dependency or corpus files are needed.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "pm-reference-substitution-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        _root: TempDir,
        capture: PathBuf,
        metadata: serde_json::Value,
    }

    fn record(file: &str, bytes: &[u8]) -> serde_json::Value {
        json!({"file": file, "bytes": bytes.len(), "sha256": sha256_hex(bytes)})
    }

    impl Fixture {
        fn new() -> Self {
            let root = TempDir::new();
            let capture = root.0.join("capture");
            std::fs::create_dir_all(capture.join("frames")).unwrap();
            let original = capture.join("frames/original.png");
            let corrected = capture.join("frames/corrected.png");
            super::super::write_png(&original, 480, 270, &[255, 0, 0, 255].repeat(480 * 270));
            super::super::write_png(&corrected, 480, 270, &[0, 255, 0, 255].repeat(480 * 270));
            let evidence = capture.join("evidence.json");
            std::fs::write(&evidence, b"{\"ownerDecision\":\"2026-10-19\"}").unwrap();
            let metadata = json!({
                "id": "COMPCLOSE-1", "ownerDecision": "2026-10-19",
                "reason": "Owner-authorized corrected app capture of the center video frame",
                "original": record("frames/original.png", &std::fs::read(original).unwrap()),
                "corrected": record("frames/corrected.png", &std::fs::read(corrected).unwrap()),
                "evidence": [evidence],
            });
            Self {
                _root: root,
                capture,
                metadata,
            }
        }

        fn sample(&self) -> Sample {
            Sample {
                time: 0.5,
                width: 480,
                height: 270,
                png: self.capture.join("frames/original.png"),
                reference_substitution: Some(
                    pm_model::json::parse(&self.metadata.to_string()).unwrap(),
                ),
                float: None,
                blur: Vec::new(),
            }
        }

        fn rejects(&self) {
            assert!(opaque_reference(&self.capture, "motion/blur-video", &self.sample()).is_err());
        }
    }

    #[test]
    fn authorized_samples_use_corrected_pixels_and_keep_original_path() {
        let fixture = Fixture::new();
        for time in [0.5, 1.2] {
            let mut sample = fixture.sample();
            sample.time = time;
            let original_path = sample.png.clone();
            let (width, height, pixels) =
                opaque_reference(&fixture.capture, "motion/blur-video", &sample).unwrap();
            assert_eq!((width, height), (480, 270));
            assert_eq!(pixels, [0, 255, 0, 255].repeat(480 * 270));
            assert_eq!(sample.png, original_path);
            assert_eq!(
                super::super::read_png(&sample.png).2,
                [255, 0, 0, 255].repeat(480 * 270)
            );
        }
        let mut original = fixture.sample();
        original.reference_substitution = None;
        assert_eq!(
            opaque_reference(&fixture.capture, "motion/blur-video", &original)
                .unwrap()
                .2,
            [255, 0, 0, 255].repeat(480 * 270)
        );
    }

    #[test]
    fn substitution_rejects_other_project_time_id_date_and_schema() {
        let mut fixture = Fixture::new();
        assert!(opaque_reference(&fixture.capture, "other/case", &fixture.sample()).is_err());
        for time in [0.0, 0.6, 1.2000000000000002, f64::NAN] {
            let mut sample = fixture.sample();
            sample.time = time;
            assert!(opaque_reference(&fixture.capture, "motion/blur-video", &sample).is_err());
        }
        let mut sample = fixture.sample();
        sample.width = 479;
        assert!(opaque_reference(&fixture.capture, "motion/blur-video", &sample).is_err());
        for (field, value) in [
            ("id", json!("OTHER")),
            ("ownerDecision", json!("2026-10-20")),
            ("reason", json!(" \n ")),
            ("evidence", json!([])),
            ("unknown", json!(true)),
        ] {
            let valid = fixture.metadata.clone();
            fixture.metadata[field] = value;
            fixture.rejects();
            fixture.metadata = valid;
        }
        let mut sample = fixture.sample();
        sample.reference_substitution = Some(pm_model::Value::Null);
        assert!(opaque_reference(&fixture.capture, "motion/blur-video", &sample).is_err());
    }

    #[test]
    fn substitution_rejects_tampered_missing_and_mismatched_original_files() {
        for kind in ["original", "corrected"] {
            let fixture = Fixture::new();
            let path = fixture.capture.join(format!("frames/{kind}.png"));
            let valid = std::fs::read(&path).unwrap();
            let mut tampered = valid.clone();
            tampered[0] ^= 1;
            std::fs::write(&path, tampered).unwrap();
            fixture.rejects();
            std::fs::remove_file(&path).unwrap();
            fixture.rejects();
        }
        let mut fixture = Fixture::new();
        fixture.metadata["original"] = fixture.metadata["corrected"].clone();
        fixture.rejects();
        let mut fixture = Fixture::new();
        fixture.metadata["corrected"]["bytes"] = json!(1);
        fixture.rejects();
        let mut fixture = Fixture::new();
        fixture.metadata["corrected"]["sha256"] = json!("0".repeat(64));
        fixture.rejects();
    }

    #[test]
    fn substitution_rejects_wrong_image_dimensions_even_with_valid_hash() {
        let mut fixture = Fixture::new();
        let path = fixture.capture.join("frames/corrected.png");
        super::super::write_png(&path, 479, 270, &[0, 255, 0, 255].repeat(479 * 270));
        fixture.metadata["corrected"] =
            record("frames/corrected.png", &std::fs::read(path).unwrap());
        fixture.rejects();
    }

    #[test]
    fn substitution_rejects_relative_absolute_and_symlink_path_escapes() {
        let mut fixture = Fixture::new();
        let bytes = std::fs::read(fixture.capture.join("frames/corrected.png")).unwrap();
        let outside = fixture._root.0.join("outside.png");
        std::fs::write(&outside, &bytes).unwrap();
        fixture.metadata["corrected"] = record("../outside.png", &bytes);
        fixture.rejects();
        fixture.metadata["corrected"]["file"] = json!(outside);
        fixture.rejects();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, fixture.capture.join("frames/escape.png"))
                .unwrap();
            fixture.metadata["corrected"] = record("frames/escape.png", &bytes);
            fixture.rejects();
        }
    }

    #[test]
    fn substitution_rejects_missing_relative_outside_and_repository_evidence() {
        let mut fixture = Fixture::new();
        let outside = fixture._root.0.join("comparison.json");
        std::fs::write(&outside, b"{}").unwrap();
        for evidence in [
            json!(fixture.capture.join("missing.json")),
            json!(outside),
            json!("relative.json"),
            json!(repo_root().join("engine/Cargo.toml")),
        ] {
            fixture.metadata["evidence"] = json!([evidence]);
            fixture.rejects();
        }
    }
}
