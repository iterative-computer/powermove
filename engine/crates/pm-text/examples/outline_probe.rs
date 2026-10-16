//! Prints a glyph's outline before and after the profile's expansion: element
//! counts, bounds, and (with `DUMP=1`) every element. A debugging aid for
//! fonts whose expansion misbehaves.
//!
//! cargo run -p pm-text --features render --example outline_probe -- <font file> <char> <size px> <expansion px> [face index]
fn main() {
    use pm_text::render::outline::{Coords, FaceRef, GlyphStyle, Outlines};
    let args: Vec<String> = std::env::args().collect();
    let data = std::fs::read(&args[1]).unwrap();
    let ch = args[2].chars().next().unwrap();
    let size: f32 = args[3].parse().unwrap();
    let ex: f64 = args[4].parse().unwrap();
    let index: u32 = args.get(5).map(|s| s.parse().unwrap()).unwrap_or(0);
    let gid = {
        use read_fonts::TableProvider;
        read_fonts::FontRef::from_index(&data, index).unwrap().cmap().unwrap().map_codepoint(ch as u32).unwrap().to_u32()
    };
    let face = FaceRef { data: &data, index, id: 1 };
    let mut o = Outlines::new();
    for e in [0.0, ex] {
        let out = o.get(face, Coords::User(&[]), gid, GlyphStyle { size, synthetic_bold: (0.0, 0.0), skew: 0.0, expand: (e, e) }).unwrap();
        let els = out.path.elements();
        let nan = els.iter().any(|el| format!("{el:?}").contains("NaN") || format!("{el:?}").contains("inf"));
        let moves = els.iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
        let closes = els.iter().filter(|e| matches!(e, kurbo::PathEl::ClosePath)).count();
        println!("expand {e}: {} elements, {moves} moves, {closes} closes, nan {nan}, bounds {:?}", els.len(), out.bounds);
        if std::env::var("DUMP").is_ok() {
            for el in els {
                println!("  {el:?}");
            }
        }
    }
}
