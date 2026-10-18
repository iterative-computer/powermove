//! Chromium 152 upload hashes captured through the shipped app's importer.
//! Failed JPEG uploads instead pin the incomplete texture seen in actual
//! legacy exports; failed uploads are not counted as upload comparisons.
use pm_media::{image, pool::FramePool};
use pm_render::media::{ProbeIssue, probe_image};
use sha2::{Digest, Sha256};
use std::path::Path;

#[test]
fn remaining_still_classes_keep_chromiums_pixels_and_failures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stills2");
    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join("chromium.json")).unwrap()).unwrap();
    let pool = FramePool::new(0);
    for row in rows {
        let name = row["file"].as_str().unwrap();
        let path = root.join(name);
        if row["outcome"] == "rejected" {
            assert!(
                image::compat::read_tiff_source(&path, &pool).is_err(),
                "{name}: the app rejects this import"
            );
            assert!(
                matches!(probe_image(&path), Err(ProbeIssue::Unreadable(_))),
                "{name}"
            );
            continue;
        }
        assert_eq!(probe_image(&path), Ok(()), "{name}");
        let source = if name.ends_with(".tiff") {
            image::compat::read_tiff_source(&path, &pool).unwrap()
        } else {
            image::read(&path, &pool).unwrap()
        };
        let bitmap = if name.ends_with(".avif") {
            image::avif_compat::read(&path, &source, &pool).unwrap()
        } else {
            image::compat::bitmap_at(&path, &source, &pool).unwrap()
        };
        let rgba = bitmap
            .frame
            .with_planes(|p| {
                (0..p[0].desc.height)
                    .flat_map(|y| unsafe {
                        std::slice::from_raw_parts(
                            p[0].ptr.add(y * p[0].stride),
                            p[0].desc.width * 4,
                        )
                    })
                    .copied()
                    .collect::<Vec<_>>()
            })
            .unwrap();
        if row["outcome"] == "incomplete-texture" {
            assert!(
                source.bit_depth > 8
                    || image::jpeg::info(&std::fs::read(&path).unwrap())
                        .unwrap()
                        .arithmetic,
                "{name}: retain native source precision"
            );
            assert!(rgba.chunks_exact(4).all(|p| p == [0, 0, 0, 255]), "{name}");
        } else {
            assert_eq!(
                format!("{:x}", Sha256::digest(&rgba)),
                row["sha256"].as_str().unwrap(),
                "{name}"
            );
        }
    }
}
