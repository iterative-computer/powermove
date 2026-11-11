//! TEXT-SMOOTH-1 on the recorder canvas: an extension's `fillText` draws
//! with today's Canvas2D weight, as the engine's text does (`pm-render`'s
//! `text_weight` gate). Today's app drew extension canvases through
//! Chromium's Canvas2D too, so the same measured stems hold: a Helvetica
//! "l" drawn with the system's Helvetica, white on black and black on white,
//! within 0.12 px of today's coverage width on average and 0.45 px at worst
//! (2026-11-11: see the printed table; exact coverage is 0.48 px off).
#![cfg(target_os = "macos")]

use pm_recorder::*;
use pm_text::font::{FaceFacts, FaceInfo, FontBytes, FontDesc, FontResolver};
use pm_text::render::smoothing::CANVAS2D_STEMS;
use read_fonts::{FileRef, TableProvider};

/// One system face for every request.
struct Face(FaceInfo);
impl FontResolver for Face {
    fn face(&self, _: u32) -> &FaceInfo {
        &self.0
    }
    fn family(&mut self, _: &str, _: &FontDesc) -> Option<u32> {
        Some(0)
    }
    fn standard(&mut self, _: &FontDesc) -> Option<u32> {
        Some(0)
    }
    fn fallback_for_char(&mut self, _: &FontDesc, _: u32, _: u32, _: bool) -> Option<u32> {
        Some(0)
    }
    fn last_resort(&mut self, _: &FontDesc) -> Option<u32> {
        Some(0)
    }
    fn variable_face(&mut self, _: &str) -> Option<u32> {
        None
    }
}

/// Helvetica's face `postscript` from the system collection.
fn helvetica(postscript: &str) -> TextEngine<Face> {
    let bytes = std::fs::read("/System/Library/Fonts/Helvetica.ttc").expect("system Helvetica");
    let FileRef::Collection(fonts) = FileRef::new(&bytes).unwrap() else {
        panic!("Helvetica.ttc is a collection");
    };
    let index = (0..fonts.len())
        .find(|&i| {
            let font = fonts.get(i).unwrap();
            let name = font.name().unwrap();
            name.name_record().iter().any(|r| {
                r.name_id() == read_fonts::types::NameId::POSTSCRIPT_NAME
                    && r.string(name.string_data())
                        .is_ok_and(|s| s.chars().eq(postscript.chars()))
            })
        })
        .expect("the face");
    let facts = FaceFacts::of(&bytes, index);
    TextEngine::new(Face(FaceInfo {
        data: FontBytes::new(bytes),
        index,
        blob: "system".into(),
        coords: Vec::new(),
        synthetic_bold: false,
        synthetic_italic: false,
        custom: false,
        web: None,
        synthesized_hyphens: [None; 2],
        postscript_name: postscript.into(),
        typeface_key: postscript.into(),
        fallback_key: None,
        facts,
    }))
}

/// The stem's coverage width on the ink's middle row, as today's probe
/// measures it: `Σ (L − background) / (paint − background)`.
fn stem(engine: &mut TextEngine<Face>, weight: u32, white: bool, size: f64) -> f64 {
    const W: u32 = 256;
    let (paint, background) = if white {
        ("#FFFFFF", "#000000")
    } else {
        ("#000000", "#FFFFFF")
    };
    let mut c = Canvas::new(1, W, W).unwrap();
    c.set_fill_color(background).unwrap();
    c.fill_rect(0., 0., W as f64, W as f64).unwrap();
    c.set_fill_color(paint).unwrap();
    c.set_font(&format!("{weight} {size}px Helvetica")).unwrap();
    c.set_text_align(TextAlign::Center);
    c.fill_text(&[u16::from(b'l')], 128., 128. + size * 0.35, None, engine)
        .unwrap();
    let px = c.get_image_data(0, 0, W as i32, W as i32).unwrap();
    let px = px.bytes();
    let cov = |x: u32, y: u32| {
        let i = ((y * W + x) * 4) as usize;
        let l = (0.2126 * f64::from(px[i])
            + 0.7152 * f64::from(px[i + 1])
            + 0.0722 * f64::from(px[i + 2]))
            / 255.;
        (if white { l } else { 1. - l }).clamp(0., 1.)
    };
    let rows: Vec<u32> = (0..W)
        .filter(|&y| (0..W).any(|x| cov(x, y) > 0.5))
        .collect();
    let y = rows[rows.len() / 2];
    (0..W).map(|x| cov(x, y)).sum()
}

#[test]
fn canvas_text_draws_with_todays_weight() {
    let mut regular = helvetica("Helvetica");
    let mut bold = helvetica("Helvetica-Bold");
    let mut failures = Vec::new();
    let mut sum = 0.;
    for &(weight, white, size, today) in CANVAS2D_STEMS {
        let engine = if weight >= 600 {
            &mut bold
        } else {
            &mut regular
        };
        let width = stem(engine, weight, white, size);
        println!(
            "{weight} {} {size:>3} px: today {today:.3}, canvas {width:.3}",
            if white { "white" } else { "black" }
        );
        sum += (width - today).abs();
        if (width - today).abs() > 0.45 {
            failures.push(format!(
                "{weight} {white} {size} px: {width:.3} against {today:.3}"
            ));
        }
    }
    let mean = sum / CANVAS2D_STEMS.len() as f64;
    println!("mean |width − today| {mean:.3} px");
    assert!(mean <= 0.12, "mean {mean:.3} px; {failures:?}");
    assert!(failures.is_empty(), "{failures:?}");
}
