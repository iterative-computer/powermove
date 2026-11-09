#[path = "golden/fonts.rs"]
mod fonts;
#[path = "golden/scenes.rs"]
mod scenes;
use std::{fs::File, io::BufReader, path::PathBuf};
fn compare(name: &str) {
    let actual = scenes::render(name);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = if root.ends_with("isolated") {
        root.parent().unwrap().to_owned()
    } else {
        root
    };
    let fixture = root.join("tests/golden").join(format!("{name}.png"));
    let decoder = png::Decoder::new(BufReader::new(
        File::open(fixture).unwrap(),
    ));
    let mut reader = decoder.read_info().unwrap();
    let mut expected = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut expected).unwrap();
    expected.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!((actual.width(), actual.height()), (info.width, info.height));
    let mut worst = 0;
    let mut bad = Vec::new();
    let mut error_sum = 0u64;
    for (i, (a, b)) in actual
        .bytes()
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        // Compare premultiplied color: undefined straight RGB at alpha zero
        // cannot turn a sub-byte coverage difference into a 255-channel error.
        let aa = f64::from(a[3]) / 255.;
        let ba = f64::from(b[3]) / 255.;
        let boundary = |pixels: &[u8]| {
            let x = i % info.width as usize;
            let y = i / info.width as usize;
            [
                (x.checked_sub(1), Some(y)),
                (Some(x + 1), Some(y)),
                (Some(x), y.checked_sub(1)),
                (Some(x), Some(y + 1)),
            ]
            .into_iter()
            .any(|(x, y)| {
                let other = match (x, y) {
                    (Some(x), Some(y)) if x < info.width as usize && y < info.height as usize => {
                        pixels[(y * info.width as usize + x) * 4 + 3]
                    }
                    _ => 0,
                };
                other != pixels[i * 4 + 3]
            })
        };
        let edge = ((0 < a[3] && a[3] < 255) || (0 < b[3] && b[3] < 255))
            && (boundary(actual.bytes()) || boundary(&expected));
        let tolerance = match name {
            // The browser reference is a bounded visual comparison, not the
            // numerical oracle for AA or its 1/16-phase image interpolator.
            "images" if i % info.width as usize >= 17 && i / (info.width as usize) < 13 => 20,
            "images" => 1,
            _ if name.starts_with("analytic-") => 1,
            "composites" | "shadow-composites" => 3,
            "linear" | "radial" | "conic" => 2,
            "strokes" if edge => 32,
            "arcs" if edge => 64,
            "roundrect" if edge => 48,
            "text" if edge => 16,
            _ if edge => 80,
            _ => 1,
        };
        for j in 0..4 {
            let av = if j == 3 {
                a[j] as f64
            } else {
                a[j] as f64 * aa
            };
            let bv = if j == 3 {
                b[j] as f64
            } else {
                b[j] as f64 * ba
            };
            let delta = (av - bv).abs().ceil() as u32;
            worst = worst.max(delta);
            error_sum += u64::from(delta);
            if delta > tolerance && bad.len() < 12 {
                bad.push((
                    i % info.width as usize,
                    i / info.width as usize,
                    j,
                    a.to_vec(),
                    b.to_vec(),
                    delta,
                    tolerance,
                ));
            }
        }
    }
    eprintln!(
        "{name}: max premultiplied channel delta {worst}; mean {:.4}",
        error_sum as f64 / actual.bytes().len() as f64
    );
    let mean = error_sum as f64 / actual.bytes().len() as f64;
    assert!(mean <= 1.5, "{name}: mean premultiplied error {mean} > 1.5");
    assert!(bad.is_empty(), "{name}: {bad:?}");
}
macro_rules! golden { ($($name:ident => $scene:literal),* $(,)?) => {$ (#[test] fn $name(){compare($scene);})*}; }
golden! {
    analytic_bezier_area=>"analytic-beziers", analytic_rounded_corner_area=>"analytic-roundrect", analytic_rotated_ellipse_area=>"analytic-ellipse",
    analytic_disk_area=>"analytic-circle", analytic_bilinear_taps=>"analytic-bilinear", analytic_fractional_rect=>"analytic-rect",
    shaped_text_via_font_provider=>"text",
    rectangles_alpha_clear=>"rects", paths_beziers_winding=>"curves", arcs_ellipses_tangents=>"arcs",
    roundrect_signed_radii=>"roundrect", caps_joins_dashes=>"strokes", ctm_and_clip=>"clip-transform",
    linear_gradient=>"linear", radial_gradient=>"radial", conic_gradient=>"conic",
    all_pattern_repetitions=>"patterns", images_and_dirty_readback=>"images", all_26_composites=>"composites",
}
