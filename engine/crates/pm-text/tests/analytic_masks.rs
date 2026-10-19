//! Public, synthetic A8 scan-conversion references from pinned Skia m152.
//! No system fonts or private rendering corpus are needed for this gate.
#![cfg(feature = "cg-masks")]

use pm_skia_path::PathBuilder;
use serde_json::Value;

#[test]
fn analytic_masks_match_skia_152_byte_for_byte() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/skia-analytic-masks.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let p = path(case);
        let pixels = pm_text::render::skaaa::fill_path_a8(
            &p,
            case["width"].as_u64().unwrap() as u32,
            case["height"].as_u64().unwrap() as u32,
        );
        let hash = pixels
            .into_iter()
            .fold(14_695_981_039_346_656_037u64, |h, v| {
                (h ^ u64::from(v)).wrapping_mul(1_099_511_628_211)
            });
        assert_eq!(hash, case["hash"].as_u64().unwrap(), "{}", case["id"]);
    }
}

fn path(case: &Value) -> pm_skia_path::Path {
    let mut p = PathBuilder::new();
    for op in case["ops"].as_array().unwrap() {
        let v = |i: usize| op[i].as_f64().unwrap() as f32;
        match op[0].as_str().unwrap() {
            "M" => p.move_to(v(1), v(2)),
            "L" => p.line_to(v(1), v(2)),
            "Q" => p.quad_to(v(1), v(2), v(3), v(4)),
            "K" => p.conic_to(v(1), v(2), v(3), v(4), v(5)),
            "C" => p.cubic_to(v(1), v(2), v(3), v(4), v(5), v(6)),
            "Z" => p.close(),
            _ => panic!("unknown oracle path verb"),
        }
    }
    p.finish().unwrap()
}

#[test]
fn n32_spans_match_skia_152_byte_for_byte() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/skia-analytic-masks.json")).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let p = path(case);
        let (w, h) = (
            case["width"].as_u64().unwrap() as u32,
            case["height"].as_u64().unwrap() as u32,
        );
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        for color in fixture["solidColors"].as_array().unwrap() {
            let rgba = std::array::from_fn(|i| color[i].as_u64().unwrap() as f32 / 255.);
            pm_text::render::skaaa::fill_path_solid(&p, w, h, rgba, &mut pixels);
        }
        let hash = pixels
            .into_iter()
            .fold(14_695_981_039_346_656_037u64, |h, v| {
                (h ^ u64::from(v)).wrapping_mul(1_099_511_628_211)
            });
        assert_eq!(hash, case["rgbaHash"].as_u64().unwrap(), "{}", case["id"]);
    }
}
