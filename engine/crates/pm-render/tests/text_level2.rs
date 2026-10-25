//! Level 2 layout glyphs at exact device positions against independent
//! 32-cell f64 outline coverage. Semantic bounds, picks, caret and animator
//! invariants are in text_level2_semantics.rs.

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use kurbo::Affine;
    use pm_protocol::color::Precision;
    use pm_reference::compare::{Class, Stats, geometry, same_geometry};
    use pm_render::Renderer;
    use pm_text::compat::TextContent;
    use pm_text::render::bitmap::Bitmaps;
    use pm_text::render::outline::{Coords, FaceRef, Outlines};
    use pm_text::render::scene::{Painter, Run, TextScene};
    use serde_json::json;

    let families = ["SF Pro Display", "Georgia", "Helvetica Neue", "Avenir Next"];
    let mut registry = pm_text::registry::CoreTextRegistry::new();
    let mut fonts = support::text::level2_fonts(&mut registry, &families);
    let mut renderer = Renderer::headless().expect("Metal device");
    let (mut outlines, mut bitmaps) = (Outlines::new(), Bitmaps::new());
    let mut failures = Vec::new();
    let mut count = 0;
    for family in families {
        for (size, scale, align, color, weight, italic, opsz) in [
            (14.0, 1.0, "left", "#FFFFFF", 500, false, None),
            (32.0, 1.0, "center", "#FFFFFF", 500, false, None),
            (24.0, 1.5, "left", "#FFD43B", 700, true, Some(18.0)),
            (72.0, 0.75, "right", "#FFFFFF", 900, false, None),
            (18.0, 2.0, "center", "#000000", 400, true, Some(12.0)),
        ] {
            let mut d = json!({
                "text": "Level two: Quick brown fox, 0123",
                "font": family, "weight": weight, "size": size,
                "tracking": 0, "leading": 1.2, "align": align,
                "italic": italic, "color": color,
                "boxWidth": 0, "boxHeight": 0
            });
            if let Some(axis) = opsz {
                d["fontAxis.opsz"] = json!(axis);
            }
            let content = TextContent::from_json(&d);
            let layout = fonts.engine.layout(&content);
            let faces: Vec<_> = layout
                .runs
                .iter()
                .map(|r| fonts.face(&r.font).unwrap().clone())
                .collect();
            let mut scene = TextScene {
                width: 640,
                height: 360,
                items: Vec::new(),
            };
            let mut painter = Painter {
                outlines: &mut outlines,
                bitmaps: &mut bitmaps,
                unsupported: Vec::new(),
            };
            let paint = pm_text::render::color::parse_or(color, [0.0, 0.0, 0.0, 1.0]);
            let placement = Affine::translate((320.25, 140.6)) * Affine::scale(scale);
            for (run, face) in layout.runs.iter().zip(&faces) {
                let draw = Run {
                    face: FaceRef {
                        data: face.as_slice(),
                        index: 0,
                        id: face.id(),
                    },
                    size: run.size,
                    coords: Coords::Normalized(&run.coords),
                    synthetic_bold: run.synthetic_bold,
                    synthetic_italic: false,
                    oblique: f64::from(run.skew),
                    glyphs: &run.glyphs,
                    bitmap_size: None,
                };
                painter.fill_text(
                    &mut scene.items,
                    &[draw],
                    (0.0, 0.0),
                    placement,
                    paint,
                    1.0,
                    0.0,
                );
            }
            assert!(
                painter.unsupported.is_empty(),
                "case {count}: {:?}",
                painter.unsupported
            );
            let ideal = support::text_reference::render(&scene);
            let diagnostic_scene = (count < 5
                && std::env::var_os("PM_TEXT_LEVEL2_ORACLE").is_some())
            .then(|| scene.clone());
            let actual = renderer
                .paint_text_scene(scene, Precision::Float32)
                .unwrap();
            if let Some(scene) = diagnostic_scene {
                let (pixel, difference) = actual
                    .chunks_exact(4)
                    .zip(ideal.rgba.chunks_exact(4))
                    .enumerate()
                    .map(|(i, (a, r))| {
                        (
                            i,
                            (0..4)
                                .map(|c| (f64::from(a[c]) - r[c]).abs() * 255.0)
                                .fold(0.0, f64::max),
                        )
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                let (x, y) = (pixel % 640, pixel / 640);
                println!(
                    "case {count}: worst pixel ({x},{y}) {difference:.3} CV, alpha engine {:.6}, reference {:.6}",
                    actual[pixel * 4 + 3],
                    ideal.rgba[pixel * 4 + 3]
                );
                for (item, path, area32, winding256) in
                    support::text_reference::point_oracle(&scene, x, y)
                {
                    println!(
                        "case {count}: oracle item {item} path {path} area32 {area32:.6} winding256 {winding256:.6} delta {:.3} CV",
                        (area32 - winding256).abs() * 255.0
                    );
                }
            }
            let errors: Vec<f64> = actual
                .chunks_exact(4)
                .zip(ideal.rgba.chunks_exact(4))
                .zip(&ideal.classes)
                .filter(|(_, class)| **class != Class::Unreferenced)
                .map(|((a, r), _)| {
                    (0..4)
                        .map(|c| (f64::from(a[c]) - r[c]).abs() * 255.0)
                        .fold(0.0, f64::max)
                })
                .collect();
            let stats = Stats::of(errors);
            println!(
                "case {count}: engine vs exact outline {stats}; unmodeled image gap {} px",
                ideal.unreferenced
            );
            if stats.max > 1.0 {
                failures.push(format!("case {count}: {stats}"));
            }
            if ideal.unreferenced == 0 {
                let ea: Vec<f64> = actual.chunks_exact(4).map(|p| f64::from(p[3])).collect();
                let ra: Vec<f64> = ideal.rgba.chunks_exact(4).map(|p| p[3]).collect();
                if let Err(e) = same_geometry(&geometry(&ea, 640), &geometry(&ra, 640), 1e-3) {
                    failures.push(format!("case {count}: {e}"));
                }
            }
            count += 1;
        }
    }
    assert_eq!(count, 20);
    assert!(
        failures.is_empty(),
        "level 2 exact outline failures: {failures:#?}"
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
