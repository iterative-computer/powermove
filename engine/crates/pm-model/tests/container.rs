//! The read-only project file readers against `decodeProjectContainer`
//! (tests/vectors/container-vectors.json and tests/fixtures/containers, both
//! written by tests/vectors/generate-container-vectors.ts with the app's own
//! encoders and decoder).

use std::path::PathBuf;

use pm_model::container::{
    Container, ContainerError, ContainerFormat, ReadOptions, read_container, read_container_bytes,
};
use pm_model::hash::{canonical_hash, sha256_hex};
use pm_model::json::{Value, parse};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/containers")
        .join(name)
}

fn vectors() -> Vec<Value> {
    parse(include_str!("vectors/container-vectors.json"))
        .unwrap()
        .get("files")
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn text(v: &Value) -> String {
    v.as_js_string().unwrap().to_string_lossy()
}

fn format_name(format: ContainerFormat) -> &'static str {
    match format {
        ContainerFormat::Json => "json",
        ContainerFormat::Pmv3 => "pmv3",
        ContainerFormat::Pmv4 => "pmv4",
    }
}

fn error_name(error: &ContainerError) -> String {
    match error {
        ContainerError::Json(_) => "json".to_owned(),
        ContainerError::Invalid(message) => (*message).to_owned(),
        ContainerError::Io(message) => format!("io: {message}"),
    }
}

fn check_ok(name: &str, bytes: &[u8], container: &Container, expected: &Value) {
    assert_eq!(
        format_name(container.format),
        text(expected.get("format").unwrap()),
        "{name}: format"
    );
    assert_eq!(
        canonical_hash(&container.document),
        text(expected.get("documentHash").unwrap()),
        "{name}: document"
    );
    let media = expected.get("media").unwrap().as_array().unwrap();
    assert_eq!(container.media.len(), media.len(), "{name}: media count");
    for (record, want) in container.media.iter().zip(media) {
        assert_eq!(
            record.id.to_string_lossy(),
            text(want.get("id").unwrap()),
            "{name}"
        );
        assert_eq!(
            record.media_type.to_string_lossy(),
            text(want.get("type").unwrap()),
            "{name}"
        );
        assert_eq!(
            record.offset as f64,
            want.get("offset").unwrap().as_f64().unwrap(),
            "{name}: offset"
        );
        assert_eq!(
            record.length as f64,
            want.get("length").unwrap().as_f64().unwrap(),
            "{name}: length"
        );
        let revision = want.get("revision").unwrap();
        assert_eq!(
            record.revision.as_ref().map(|r| r.to_string_lossy()),
            (!revision.is_null()).then(|| text(revision)),
            "{name}: revision"
        );
        let sha = want.get("sha256").unwrap();
        assert_eq!(
            record.sha256.clone(),
            (!sha.is_null()).then(|| text(sha)),
            "{name}: sha256"
        );
        let data = container.media_bytes(bytes, record).unwrap();
        assert_eq!(
            sha256_hex(data),
            text(want.get("dataSha256").unwrap()),
            "{name}: data"
        );
    }
}

#[test]
fn every_container_matches_decode_project_container() {
    let vectors = vectors();
    assert_eq!(vectors.len(), 39);
    for entry in &vectors {
        let name = text(entry.get("name").unwrap());
        let bytes = std::fs::read(fixture(&name)).unwrap();
        let result = read_container_bytes(&bytes, ReadOptions::default());
        match (entry.get("ok"), entry.get("error"), result) {
            (Some(expected), None, Ok(container)) => check_ok(&name, &bytes, &container, expected),
            (None, Some(expected), Err(error)) => {
                assert_eq!(error_name(&error), text(expected), "{name}")
            }
            (_, _, result) => panic!("{name}: expected {entry:?}, got {result:?}"),
        }
    }
}

#[test]
fn files_read_through_random_access_match_memory() {
    for entry in vectors() {
        let name = text(entry.get("name").unwrap());
        let path = fixture(&name);
        let bytes = std::fs::read(&path).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let from_file = read_container(&file, ReadOptions::default());
        let from_memory = read_container_bytes(&bytes, ReadOptions::default());
        assert_eq!(from_file, from_memory, "{name}");
    }
}

#[test]
fn media_checksums_can_be_deferred() {
    // Without verify_media the corrupt media still opens; verifying it fails.
    let bytes = std::fs::read(fixture("pmv4-media-corrupt.pmv")).unwrap();
    assert_eq!(
        read_container_bytes(&bytes, ReadOptions::default()),
        Err(ContainerError::Invalid(
            "The project content checksum does not match."
        ))
    );
    let container = read_container_bytes(
        &bytes,
        ReadOptions {
            verify_media: false,
        },
    )
    .unwrap();
    let results: Vec<_> = container
        .media
        .iter()
        .map(|record| container.verify_media(bytes.as_slice(), record).is_ok())
        .collect();
    assert_eq!(results, [false, true]);
    // A good file verifies every record on demand too.
    let bytes = std::fs::read(fixture("pmv4.pmv")).unwrap();
    let container = read_container_bytes(
        &bytes,
        ReadOptions {
            verify_media: false,
        },
    )
    .unwrap();
    for record in &container.media {
        container.verify_media(bytes.as_slice(), record).unwrap();
    }
}

#[test]
fn project_is_proj_or_the_bare_document() {
    let saved = read_container_bytes(
        &std::fs::read(fixture("pmv4.pmv")).unwrap(),
        ReadOptions::default(),
    )
    .unwrap();
    assert_eq!(
        saved.project().get("name").and_then(Value::as_str),
        Some("Container test")
    );
    assert!(saved.document.get("history").is_some());
    let bare = read_container_bytes(
        &std::fs::read(fixture("pmv4-bare-project.pmv")).unwrap(),
        ReadOptions::default(),
    )
    .unwrap();
    assert_eq!(bare.project(), &bare.document);
    // Lone surrogates written by JSON.stringify survive the reader.
    let text = saved.project().get("text").unwrap().as_js_string().unwrap();
    assert!(text.as_str().is_none());
    assert_eq!(
        pm_model::json::to_canonical_string(saved.project().get("text").unwrap()),
        "\"lone \\ud800 surrogate and 😀\""
    );
}
