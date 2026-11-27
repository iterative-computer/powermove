//! 3D material maps read the level 1 still-compatibility bitmap, texel for
//! texel as today's app stores them.
//!
//! `fixtures/stills2/three-maps.json` holds what Electron 44 (Chromium 152)
//! stored for each fixture through the shipped importer and three.js r180's
//! map upload (`e2e/tools/threed-maps-capture.ts`: `texStorage2D` of
//! `log2(max(w, h)) + 1` levels, `texSubImage2D` of the asset's bitmap,
//! read back from a framebuffer), as SHA-256 of level 0 (identical in the
//! sRGB and linear formats) and of level 1 after `generateMipmap` per
//! format. `upload-failed` rows are the deep and arithmetic JPEGs whose
//! upload fails `INVALID_VALUE` and leaves the storage zeroed; `rejected`
//! rows fail import (no map).
mod support;

use pm_render::scene3d::textures::{MapError, Textures};
use std::path::Path;

/// A map is level 1 content, read from the same Chromium bitmap as an
/// image layer: for every still fixture, the map's verdict is the image
/// layer's (`probe_image`), feature for feature, so a map is never the
/// reason a project an image layer of the same file draws in is refused;
/// and every still an image layer draws decodes to the one RGBA8 plane a
/// map uploads (`decode`'s layout check is an internal assertion).
#[test]
fn map_verdicts_are_the_image_layer_verdicts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stills2");
    let mut names: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                matches!(
                    x,
                    "png"
                        | "jpg"
                        | "jpeg"
                        | "tiff"
                        | "tif"
                        | "avif"
                        | "webp"
                        | "gif"
                        | "bmp"
                        | "heic"
                )
            })
        })
        .collect();
    names.sort();
    let (mut drawn, mut unsupported, mut unreadable) = (0, 0, 0);
    for path in &names {
        let name = path.file_name().unwrap().to_string_lossy();
        let layer = pm_render::media::probe_image(path);
        let map = Textures::probe(path);
        match (&layer, &map) {
            (Ok(()), Ok(())) => drawn += 1,
            (Err(pm_render::media::ProbeIssue::Unsupported(a)), Err(MapError::Unsupported(b))) => {
                assert_eq!(a, b, "{name}");
                unsupported += 1;
            }
            (Err(pm_render::media::ProbeIssue::Unreadable(_)), Err(MapError::Failed(_))) => {
                unreadable += 1;
            }
            _ => panic!("{name}: image layer {layer:?}, map {map:?}"),
        }
    }
    eprintln!("map verdicts: {drawn} drawn, {unsupported} unsupported, {unreadable} unreadable");
    assert!(drawn >= 100, "{drawn}");
}
