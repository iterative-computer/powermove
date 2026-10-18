//! Dumps the paint record and the draws of one SVG case (diagnostics).
#[path = "../tests/canvas_cases/mod.rs"]
mod canvas_cases;
use pm_vector::render::svg::SvgDocument;
fn main() {
    let want = std::env::args().nth(1).unwrap_or_else(|| "svg-opacity-64x64".into());
    for c in canvas_cases::cases() {
        if c["id"] != want.as_str() {
            continue;
        }
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        let (w, h) = (c["width"].as_u64().unwrap() as u32, c["height"].as_u64().unwrap() as u32);

        match doc.raster_compat(w, h) {
            Ok(b) => {
                let n = b.rgba.chunks(4).filter(|p| p[3] != 0).count();
                println!("{want}: {}x{} opaque-ish pixels {n}", b.width, b.height);
            }
            Err(e) => println!("{want}: error {e}"),
        }
    }
}
