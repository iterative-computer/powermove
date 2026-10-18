//! Prints the level 1 gate verdict of every SVG canvas case at its size and
//! over its asset's bucket ladder (docs/engine/vector.md, "What level 1
//! draws").
//!
//! `--cases <file.json>` checks a custom case list (the
//! `canvas-ref/run.ts --cases` format) instead of the fixture cases.
#[path = "../tests/canvas_cases/mod.rs"]
mod canvas_cases;
use pm_vector::render::svg::SvgDocument;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cases = match args.iter().position(|a| a == "--cases") {
        Some(i) => {
            let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[i + 1]).unwrap()).unwrap();
            doc["cases"].as_array().unwrap().clone()
        }
        None => canvas_cases::cases(),
    };
    for c in cases {
        if c["kind"] != "svg" {
            continue;
        }
        let id = c["id"].as_str().unwrap();
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        let (w, h) = (c["width"].as_u64().unwrap() as u32, c["height"].as_u64().unwrap() as u32);
        let at = doc.compat_supported_at(&[(w, h)]);
        let (sw, sh) = doc.size();
        let t = std::time::Instant::now();
        let ladder = doc.compat_supported(sw, sh);
        let n = pm_vector::render::svg::svg_bucket_ladder(sw, sh, 8192.0).len();
        println!("{id:24} at-size {:60} ladder({n} buckets, {:?}) {}", format!("{at:?}"), t.elapsed(), match &ladder { Ok(()) => "Ok".into(), Err(e) => e.0.clone() });
    }
}
