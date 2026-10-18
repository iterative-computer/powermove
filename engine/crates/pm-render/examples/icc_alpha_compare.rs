//! Section 25 alpha-class comparison against the legacy float accumulator.
//! Unlike PNG round trips, this measures premultiplied working values before
//! the encoders' byte quantization and unpremultiplication.
#[path = "../tests/support/doc.rs"]
mod doc;
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;

fn main() {
    let root = std::path::PathBuf::from(
        std::env::var_os("PM_MEDIA_EVIDENCE").expect("private ICC capture directory"),
    );
    let mut renderer = Renderer::headless().unwrap();
    let mut rows = Vec::new();
    for class in ["icc-opaque", "icc-alpha"] {
        for name in ["p3.png", "p3.tiff", "adobe.png", "adobe.tiff"] {
            let dir = root.join(class);
            let value: serde_json::Value = serde_json::from_slice(
                &std::fs::read(dir.join(format!("{name}.document.json"))).unwrap(),
            )
            .unwrap();
            let mut resolver = MapResolver::default();
            for id in value["assets"].as_object().unwrap().keys() {
                resolver.paths.insert(id.clone(), dir.join(name));
            }
            let document = doc::load(&value);
            let composition = value["compId"].as_str().unwrap();
            assert!(
                renderer
                    .unsupported_with_media(&document, composition, &resolver)
                    .is_empty()
            );
            let mut request = RenderRequest::at_scale(Size::new(96, 64), 1.0, Precision::Float32);
            request.background = false;
            let frame = renderer
                .render(
                    &document,
                    composition,
                    pm_protocol::common::Ticks::ZERO,
                    &request,
                    &resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let native = renderer.read_rgba(&frame).unwrap();
            let bytes = std::fs::read(dir.join(format!("{name}.rgba32f"))).unwrap();
            let reference: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
                .collect();
            assert_eq!(native.len(), reference.len());
            let max = native
                .iter()
                .zip(&reference)
                .map(|(a, b)| f64::from((a - b).abs()) * 255.0)
                .fold(0.0f64, f64::max);
            let alpha = native
                .chunks_exact(4)
                .zip(reference.chunks_exact(4))
                .map(|(a, b)| f64::from((a[3] - b[3]).abs()) * 255.0)
                .fold(0.0f64, f64::max);
            eprintln!("{class}/{name}: premultiplied float max{max:.6} CV alpha{alpha:.6}");
            let mut pair = vec![0u8; 96 * 64 * 8];
            for y in 0..64 {
                for (side, src) in [&reference, &native].iter().enumerate() {
                    for x in 0..96 {
                        let i = (y * 192 + side * 96 + x) * 4;
                        for c in 0..3 {
                            pair[i + c] =
                                (src[(y * 96 + x) * 4 + c].clamp(0.0, 1.0) * 255.0).round() as u8;
                        }
                        pair[i + 3] = 255;
                    }
                }
            }
            let mut encoder = png::Encoder::new(
                std::fs::File::create(
                    dir.join(format!("{name}-float-legacy-left-native-right.png")),
                )
                .unwrap(),
                192,
                64,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pair)
                .unwrap();
            rows.push(
                json!({"class":class,"file":name,"max_cv":max,"alpha_cv":alpha,"pass":max<=1.0}),
            );
        }
    }
    std::fs::write(
        root.join("icc-float-metrics.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    assert!(
        rows.iter().all(|r| r["pass"] == true),
        "ICC alpha class fails: see metrics"
    );
}
