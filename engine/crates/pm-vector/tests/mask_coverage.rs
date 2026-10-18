//! Path mask coverage canvases against today's (`tools/mask-ref`;
//! docs/engine/composite.md "Path masks"), with the blur run by the CPU
//! model of Skia's GPU passes (`BlurPlan::run_cpu`). The item rasters must
//! match exactly where nothing is blurred; blurred cases report how close
//! the model is (pm-render's GPU executor carries the bar).
#![cfg(feature = "render")]

mod mask_cases;

use pm_vector::LayerMasks;
use pm_vector::canvas2d::filter::{BlurPlan, Image, Rect};
use pm_vector::canvas2d::mask;

fn coverage_cpu(c: &mask_cases::Case, fill: &mut std::time::Duration) -> Vec<u8> {
    let LayerMasks::Path(masks) = LayerMasks::new(&c.masks) else { panic!("{}: not a path route", c.id) };
    let mut cov = mask::start(&masks, c.width, c.height);
    for m in &masks.masks {
        let started = std::time::Instant::now();
        let item = mask::item(m, c.world, c.width, c.height).unwrap();
        *fill += started.elapsed();
        let op = mask::Composite::from(m.mode);
        match BlurPlan::new(c.width, c.height, mask::sigma(m, c.world)) {
            None => mask::composite(&mut cov, &item.alpha, op),
            Some(plan) => {
                let img = Image { rect: Rect::ltrb(0, 0, c.width as i32, c.height as i32), px: item.alpha };
                let v = plan.run_cpu(&img, c.width, c.height);
                for (d, s) in cov.iter_mut().zip(v) {
                    let df = f32::from(*d) / 255.0;
                    let r = match op {
                        mask::Composite::SourceOver => s + df * (1.0 - s),
                        mask::Composite::DestinationOut => df * (1.0 - s),
                    };
                    *d = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            }
        }
    }
    cov
}

#[test]
fn coverage_model_against_chromium() {
    let Some(dir) = mask_cases::capture_dir() else {
        eprintln!("note: no masks-* capture (run tools/mask-ref); not measured");
        return;
    };
    eprintln!("capture {}", dir.display());
    let only = std::env::var("PM_MASK_ONLY").ok();
    let mut sharp_failures = Vec::new();
    for c in mask_cases::cases() {
        if only.as_deref().is_some_and(|o| !c.id.starts_with(o)) {
            continue;
        }
        if c.width * c.height > 2_100_000 {
            continue;
        }
        let mut fill = std::time::Duration::ZERO;
        let ours = coverage_cpu(&c, &mut fill);
        let (_, _, reference) = mask_cases::read_coverage(&dir.join(format!("{}.png", c.id)));
        let (over1, max, any) = mask_cases::diff(&ours, &reference);
        let blurred = c.masks.iter().any(|m| m.feather * (c.world[0].hypot(c.world[1])) / 2.0 > 0.03);
        eprintln!(
            "{:<32} off {:>6} >1 {:>6} max {:>3}  item fill {:>7.1} ms{}",
            c.id,
            any,
            over1,
            max,
            fill.as_secs_f64() * 1000.0,
            if blurred { "" } else { "  (sharp)" }
        );
        if !blurred && max > 0 {
            sharp_failures.push(c.id.clone());
        }
    }
    assert!(sharp_failures.is_empty(), "sharp coverage differs: {sharp_failures:?}");
}
