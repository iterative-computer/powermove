//! Replays a Chromium SVG `<img>` probe against `svg::broken_image`.
//!
//! Usage: `svg_broken_compare <dir>`, where `<dir>` holds `<case>.svg`
//! files and `chromium.json` mapping each case to `{ "ok": bool }` (whether
//! Chromium's `<img>` loaded it). Exits non-zero when a document Chromium
//! paints is classified broken (a blank layer that should paint), the one
//! outcome that would be a visible difference. Chromium-broken documents
//! the engine leaves undecided stay explicitly unsupported and are listed.

use pm_media::image::svg::broken_image;
use std::path::Path;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: svg_broken_compare <dir>");
    let dir = Path::new(&dir);
    let chromium: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("chromium.json")).expect("chromium.json"))
            .expect("chromium.json");
    let (mut agree, mut undecided, mut wrong) = (0, Vec::new(), Vec::new());
    for (name, result) in chromium.as_object().expect("an object of cases") {
        let data = std::fs::read(dir.join(format!("{name}.svg"))).expect("case file");
        let chromium_broken = !result["ok"].as_bool().expect("ok");
        match (chromium_broken, broken_image(&data)) {
            (a, b) if a == b => agree += 1,
            (true, false) => undecided.push(name.clone()),
            _ => wrong.push(name.clone()),
        }
    }
    println!("agree {agree}");
    println!(
        "undecided (Chromium broken, engine gated) {}: {undecided:?}",
        undecided.len()
    );
    println!("painted but classified broken {}: {wrong:?}", wrong.len());
    if !wrong.is_empty() {
        std::process::exit(1);
    }
}
