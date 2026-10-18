//! Bezier path mask coverage canvases on the GPU against today's
//! (`pm-vector/tools/mask-ref`; docs/engine/composite.md "Path masks").
//!
//! The references are captured from today's `applyPathMasks` in the app's
//! Electron (the coverage canvas as the apply pass samples it); the test
//! reads `PM_MASK_REF` or the newest complete `masks-*` capture and passes
//! with a note when there is none. Bar: no pixel off by more than 1 code
//! value.
#![cfg(target_os = "macos")]

#[path = "../../pm-vector/tests/mask_cases/mod.rs"]
mod mask_cases;

use pm_render::composite::path_masks::{PathCoverage, coverage_bytes};
use pm_vector::LayerMasks;

#[test]
fn path_mask_coverage_matches_chromium() {
    let Some(dir) = mask_cases::capture_dir() else {
        eprintln!("note: no masks-* capture (run pm-vector/tools/mask-ref); not measured");
        return;
    };
    eprintln!("capture {}", dir.display());
    let gpu = pm_gpu::Gpu::new("path mask test").expect("a Metal device");
    let only = std::env::var("PM_MASK_ONLY").ok();
    let mut failures = Vec::new();
    let (mut total_off, mut cases) = (0usize, 0usize);
    for c in mask_cases::cases() {
        if only.as_deref().is_some_and(|o| !c.id.starts_with(o)) {
            continue;
        }
        let LayerMasks::Path(masks) = LayerMasks::new(&c.masks) else { panic!("{}: not a path route", c.id) };
        let cov = PathCoverage { width: c.width, height: c.height, world: c.world, masks };
        let ours = coverage_bytes(&gpu.device, &gpu.queue, &cov).unwrap();
        let (_, _, reference) = mask_cases::read_coverage(&dir.join(format!("{}.png", c.id)));
        let (over1, max, any) = mask_cases::diff(&ours, &reference);
        total_off += any;
        cases += 1;
        eprintln!("{:<32} off {:>6} >1 {:>4} max {:>3}", c.id, any, over1, max);
        if let Ok(out) = std::env::var("PM_MASK_DIFFS") {
            let out = std::path::Path::new(&out);
            std::fs::create_dir_all(out).unwrap();
            let save = |name: &str, px: &[u8]| {
                let f = std::fs::File::create(out.join(format!("{}-{name}.png", c.id))).unwrap();
                let mut e = png::Encoder::new(std::io::BufWriter::new(f), c.width, c.height);
                e.set_color(png::ColorType::Grayscale);
                e.set_depth(png::BitDepth::Eight);
                e.write_header().unwrap().write_image_data(px).unwrap();
            };
            if max > 1 {
                save("engine", &ours);
                save("legacy", &reference);
                let d: Vec<u8> = ours.iter().zip(&reference).map(|(a, b)| (a.abs_diff(*b)).saturating_mul(40)).collect();
                save("diff40", &d);
            }
        }
        if max > 1 {
            failures.push(format!("{} ({over1} px > 1, max {max})", c.id));
        }
    }
    eprintln!("{cases} cases, {total_off} pixels off by 1");
    assert!(failures.is_empty(), "coverage differs by more than 1: {failures:?}");
}
