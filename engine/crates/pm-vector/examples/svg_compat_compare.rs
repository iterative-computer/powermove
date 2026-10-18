//! Chromium SVG bucket comparison. Evidence stays outside the repository.
#[path = "../tests/canvas_cases/mod.rs"]
mod canvas_cases;
use pm_vector::render::svg::SvgDocument;
fn write(path: &std::path::Path, w: u32, h: u32, pixels: &[u8]) {
    let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(pixels).unwrap();
}
fn main() {
    let out = std::path::PathBuf::from(std::env::var("PM_MEDIA_EVIDENCE").expect("evidence directory outside repository"));
    std::fs::create_dir_all(&out).unwrap();
    let capture = canvas_cases::capture_dir().expect("Chromium capture required");
    let mut rows = Vec::new();
    let mut fonts = pm_text::compat::Compat::new(pm_text::registry::CoreTextRegistry::new());
    for c in canvas_cases::cases() {
        if c["kind"] != "svg" {
            continue;
        }
        let id = c["id"].as_str().unwrap();
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        let (w, h, reference) = canvas_cases::read_png(&capture.join(format!("{id}.png")));
        match doc.raster_compat_with_text(w, h, &mut fonts) {
            Err(e) => {
                eprintln!("{id}: {e}");
                rows.push(serde_json::json!({"case":id,"error":e.to_string(),"pass":false}));
            }
            Ok(bitmap) => {
                let mut max = 0;
                let mut over1 = 0;
                for (a, b) in bitmap.rgba.chunks_exact(4).zip(reference.chunks_exact(4)) {
                    let d = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                    max = max.max(d);
                    over1 += usize::from(d > 1);
                }
                let pass = max <= 4 && over1 * 1000 <= (w * h) as usize;
                eprintln!("{id}: max {max}, over1 {over1}/{} pass={pass}", w * h);
                write(&out.join(format!("{id}-native.png")), w, h, &bitmap.rgba);
                let mut pair = vec![0u8; (w * h * 8) as usize];
                for y in 0..h as usize {
                    for (side, bytes) in [&reference, &bitmap.rgba].iter().enumerate() {
                        let dst = (y * w as usize * 2 + side * w as usize) * 4;
                        pair[dst..dst + w as usize * 4].copy_from_slice(&bytes[y * w as usize * 4..(y + 1) * w as usize * 4]);
                    }
                }
                write(&out.join(format!("{id}-legacy-left-native-right.png")), w * 2, h, &pair);
                rows.push(serde_json::json!({"case":id,"max_cv":max,"pixels_over1":over1,"pixels":w*h,"pass":pass}));
            }
        }
    }
    std::fs::write(out.join("metrics.json"), serde_json::to_string_pretty(&rows).unwrap()).unwrap();
    if std::env::args().any(|a| a == "--expect-parity") {
        assert_eq!(rows.len(), 27);
        assert!(rows.iter().all(|r| r["pass"] == true), "SVG class fails; see metrics");
    }
}
