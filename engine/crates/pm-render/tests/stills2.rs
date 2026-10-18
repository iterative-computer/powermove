//! Chromium 152 upload hashes captured through the shipped app's importer.
//! Failed JPEG uploads instead pin the incomplete texture seen in actual
//! legacy exports; failed uploads are not counted as upload comparisons.
mod support;
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

/// Level 2 layers read the native-depth source, which reads neither PNG
/// cICP/cHRM nor HDR transfers, and ImageIO converts CMYK and other models
/// to sRGB. The classes measured only for Chromium's bitmap therefore stay
/// named for level 2, in the capability check and at execution, while the
/// same files draw at level 1.
#[test]
fn level_two_keeps_the_native_reader_classes() {
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::capability::Feature;
    use pm_render::media::{StillReader, probe_still};
    use pm_render::{RenderError, RenderRequest, Renderer};
    use serde_json::json;
    use support::doc::*;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stills2");
    let mut renderer = Renderer::headless().unwrap();
    let mut request = RenderRequest::at_scale(Size::new(32, 24), 1.0, Precision::Float32);
    request.background = false;
    for name in [
        "pq16.png",
        "hlg8-alpha.png",
        "chrm16.png",
        "chrm8.png",
        "pq-metadata.png",
        "hlg-metadata.png",
        "plain-metadata.png",
        "hdr-pq.avif",
        "hdr-hlg.avif",
        "bt2020-sdr.avif",
        "cmyk-noicc.jpg",
        "lossless8.jpg",
        "lossless12.jpg",
        "arithmetic8.jpg",
        "tiff-cmyka.tiff",
        "tiff-tiled.tiff",
        "tiff-ycbcr-jpeg.tiff",
        "tiff-planar16.tiff",
        "tiff-f16-pred3.tiff",
        "tiff-f32-lzw-pred3.tiff",
    ] {
        let path = root.join(name);
        let native = probe_still(&path, StillReader::Native);
        assert!(
            matches!(native, Err(ProbeIssue::Unsupported(Feature::Media(_)))),
            "{name}: {native:?}"
        );
        assert_eq!(probe_still(&path, StillReader::Chromium), Ok(()), "{name}");
        let ext = name.rsplit_once('.').unwrap().1;
        let mut value = project(
            32,
            24,
            1.0,
            no_fill(),
            vec![layer(
                "still",
                "image",
                json!({"asset":"clip","w":32,"h":24}),
                16.0,
                12.0,
                1.0,
            )],
        );
        asset(&mut value, "clip", "image", ext, 32, 24, 1.0);
        let mut resolver = pm_render::MapResolver::default();
        resolver.paths.insert("clip".into(), path.clone());
        for level in [1, 2] {
            value["layers"][0]["compat"] = json!(level);
            let document = load(&value);
            let gated = renderer.unsupported_with_media(&document, "Ptest-root", &resolver);
            let rendered = renderer.render(
                &document,
                "Ptest-root",
                support::ticks(0.0),
                &request,
                &resolver,
            );
            if level == 1 {
                assert!(gated.is_empty(), "{name} level 1: {gated:?}");
                let frame = rendered.unwrap_or_else(|e| panic!("{name} level 1: {e}"));
                assert!(frame.failures.is_empty(), "{name}: {:?}", frame.failures);
            } else {
                assert!(
                    gated
                        .iter()
                        .any(|u| matches!(u.feature, Feature::Media(_))
                            && u.layer.as_deref() == Some("still")),
                    "{name} level 2 capability: {gated:?}"
                );
                // Rendering without the capability check names it too:
                // never PQ/HLG code values or CMYK drawn as sRGB.
                match rendered {
                    Err(RenderError::Unsupported(u)) => assert!(
                        u.iter().any(|u| matches!(u.feature, Feature::Media(_))),
                        "{name}: {u:?}"
                    ),
                    Err(e) => panic!("{name} level 2: {e}"),
                    Ok(_) => panic!("{name} level 2 drew a class its reader cannot read"),
                }
            }
        }
    }
}
