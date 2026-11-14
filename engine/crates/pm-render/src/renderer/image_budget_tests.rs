//! Source snapshots are frames of the media cache: a renderer drawing one
//! distinct still after another keeps only the snapshot being drawn's still
//! over a small budget.

use super::*;
use pm_protocol::common::Size;
use pm_protocol::snapshot::SourceFit;

#[test]
fn source_snapshots_keep_only_the_still_being_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_image_cache_budget(1);
    let mut first = None;
    for i in 0..6u8 {
        let path = dir.path().join(format!("snapshot-{i}.png"));
        let rgba: Vec<u8> = (0..128u32 * 128)
            .flat_map(|p| [i.wrapping_mul(41), (p % 128) as u8, (p / 128) as u8, 255])
            .collect();
        let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 128, 128);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
        renderer
            .source_rgba8(
                &path,
                Ticks::new(0).unwrap(),
                Size::new(64, 64),
                SourceFit::Contain,
                None,
                false,
            )
            .unwrap();
        let held = renderer.image_cache_bytes();
        assert!(held > 0, "the snapshot's own still is kept");
        assert_eq!(
            held,
            *first.get_or_insert(held),
            "snapshot {i}: earlier snapshots' stills were kept beside its own"
        );
    }
}
