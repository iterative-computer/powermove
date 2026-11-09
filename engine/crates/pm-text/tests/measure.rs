//! Uses a repository font on every OS. No platform font registry or renderer.
#[cfg(feature = "level2")]
use pm_text::level2::{Level2, StyleSpan};
use pm_text::{
    compat::{Compat, TextContent},
    font::*,
    measure::*,
};

fn font() -> FontBytes {
    FontBytes::new(include_bytes!("fixtures/Geist-Variable.ttf").to_vec())
}
fn content(text: &str) -> TextContent {
    TextContent {
        text: text.into(),
        font: Some("Geist".into()),
        size: Some(40.0),
        weight: Some(400.0),
        ..Default::default()
    }
}
#[cfg(feature = "level2")]
fn engine() -> Level2 {
    let mut e = Level2::new();
    assert!(e.add_font("geist", font(), "Geist") > 0);
    e.set_fallback_families(&["Geist".into()]);
    e
}
struct ReadyFont(FaceInfo);
impl ReadyFont {
    fn new() -> Self {
        let data = font();
        Self(FaceInfo {
            facts: FaceFacts::of(data.as_slice(), 0),
            data,
            index: 0,
            blob: "geist".into(),
            coords: vec![],
            synthetic_bold: false,
            synthetic_italic: false,
            custom: true,
            web: None,
            synthesized_hyphens: [None; 2],
            postscript_name: "Geist".into(),
            typeface_key: "geist".into(),
            fallback_key: None,
        })
    }
}
impl FontResolver for ReadyFont {
    fn face(&self, _: FaceId) -> &FaceInfo {
        &self.0
    }
    fn family(&mut self, _: &str, _: &FontDesc) -> Option<FaceId> {
        Some(0)
    }
    fn standard(&mut self, _: &FontDesc) -> Option<FaceId> {
        Some(0)
    }
    fn fallback_for_char(&mut self, _: &FontDesc, _: u32, _: FaceId, _: bool) -> Option<FaceId> {
        None
    }
    fn last_resort(&mut self, _: &FontDesc) -> Option<FaceId> {
        Some(0)
    }
    fn variable_face(&mut self, _: &str) -> Option<FaceId> {
        Some(0)
    }
}
#[test]
fn compatibility_measurement_uses_the_existing_authored_layout() {
    let mut e = Compat::new(ReadyFont::new());
    let d = content("Hello motion\nWorld");
    let caret = e.caret_layout(&d);
    let units = e.glyph_layout(&d);
    let actual = measure_compat(&mut e, &d).unwrap();
    assert_eq!(actual.level, LayoutLevel::Compat1);
    assert_eq!(actual.lines.len(), caret.lines.len());
    assert_eq!(actual.units, units);
    for (a, b) in actual.lines.iter().zip(&caret.lines) {
        assert_eq!(a.bounds.x0, b.x);
        assert_eq!(a.bounds.width(), b.width);
        assert_eq!(a.baseline, b.baseline);
        assert_eq!(a.caret, b.boundaries);
    }
    assert_eq!(actual.ink, InkBounds::NotMeasured);
}
#[test]
fn trait_is_usable_by_a_headless_test_double() {
    struct Double(TextMeasurement);
    impl TextMeasure for Double {
        fn measure_text(&mut self, _: &TextContent) -> Result<TextMeasurement, MeasureError> {
            Ok(self.0.clone())
        }
    }
    let mut font = Compat::new(ReadyFont::new());
    let d = content("hi");
    let m = font.measure_text(&d).unwrap();
    let mut provider: Box<dyn TextMeasure> = Box::new(Double(m.clone()));
    assert_eq!(provider.measure_text(&d).unwrap(), m);
}
#[test]
fn invalid_numbers_are_reported_before_shaping() {
    let mut e = Compat::new(ReadyFont::new());
    let mut d = content("hi");
    d.size = Some(f64::NAN);
    assert_eq!(e.measure_text(&d), Err(MeasureError::InvalidValue("size")));
    d.size = Some(40.0);
    d.axes = vec![("wdth".into(), f64::INFINITY)];
    assert_eq!(
        e.measure_text(&d),
        Err(MeasureError::InvalidValue("fontAxis"))
    );
}
#[cfg(feature = "level2")]
mod native {
    use super::*;
    #[test]
    fn font_backed_widths_and_per_line_metrics_equal_the_shaper() {
        let mut e = engine();
        let d = content("Hello\nMotion");
        let shaped = e.layout(&d);
        let m = measure_level2(&mut e, &d, &[]).unwrap();
        assert_eq!(m.lines.len(), 2);
        assert_eq!(m.level, LayoutLevel::Native2);
        for (a, b) in m.lines.iter().zip(shaped.lines) {
            assert_eq!(a.bounds.width(), b.width);
            assert_eq!(a.baseline, b.baseline);
            assert_eq!(a.font_ascent, Some(b.ascent));
            assert_eq!(a.font_descent, Some(b.descent));
            assert_eq!(a.bounds.height(), b.height);
        }
        assert_eq!(m.units, shaped.units);
        assert!(m.layout_bounds.width() > 40.0);
    }
    #[test]
    fn paragraph_boxes_and_alignment_are_separate_from_content_bounds() {
        for (align, x) in [("left", 0.0), ("center", -100.0), ("right", -200.0)] {
            let mut d = content("Hi");
            d.paragraph = Some(true);
            d.box_width = Some(200.0);
            d.box_height = Some(120.0);
            d.align = Some(align.into());
            let m = engine().measure_text(&d).unwrap();
            let b = m.paragraph_box.unwrap();
            assert_eq!(b.x0, x);
            assert_eq!(b.width(), 200.0);
            assert_eq!(b.height(), 120.0);
            assert!(m.layout_bounds.width() < b.width());
            assert_eq!(
                m.layout_bounds.center()[0],
                match align {
                    "left" => m.layout_bounds.width() / 2.0,
                    "right" => -m.layout_bounds.width() / 2.0,
                    _ => 0.0,
                }
            );
        }
    }
    #[test]
    fn soft_break_carets_choose_upstream_or_downstream_explicitly() {
        let mut d = content("one two three four");
        d.paragraph = Some(true);
        d.box_width = Some(100.0);
        let m = engine().measure_text(&d).unwrap();
        assert!(m.lines.len() > 1);
        let offset = m.lines[1].start;
        let up = m.caret(offset, CaretAffinity::Upstream).unwrap();
        let down = m.caret(offset, CaretAffinity::Downstream).unwrap();
        assert!(up.line < down.line);
        assert_ne!(up.top, down.top);
        assert_eq!(m.source_length, d.text.encode_utf16().count());
    }
    #[test]
    fn grapheme_carets_do_not_cut_combining_marks_or_ligatures() {
        let d = content("a\u{301} office");
        let m = engine().measure_text(&d).unwrap();
        assert!(m.carets(1).is_empty());
        assert!(!m.carets(2).is_empty());
        assert_eq!(m.units.characters[0].text, "a\u{301}");
        for line in &m.lines {
            for c in &line.caret {
                let nearest = m
                    .nearest_caret(c.x, (line.bounds.y0 + line.bounds.y1) / 2.0)
                    .unwrap();
                assert!((nearest.x - c.x).abs() < 1e-6);
            }
        }
        assert!(m.nearest_caret(f64::NAN, 0.0).is_none());
        assert!(m.carets(100).is_empty());
    }
    #[test]
    fn blank_text_keeps_real_line_metrics_and_one_caret() {
        let m = engine().measure_text(&content("")).unwrap();
        assert_eq!(m.lines.len(), 1);
        assert_eq!(m.source_length, 0);
        assert_eq!(m.lines[0].caret.len(), 1);
        assert!(m.lines[0].font_ascent.unwrap() > 0.0);
        assert!(m.layout_bounds.height() > 0.0);
        assert!(m.units.characters.is_empty());
        let m = engine().measure_text(&content("\n\n")).unwrap();
        assert_eq!(m.lines.len(), 3);
    }
    #[test]
    fn styles_and_variable_axes_participate_in_font_measurement() {
        let mut e = engine();
        let d = content("Motion design");
        let base = e.measure_text(&d).unwrap();
        let mut larger = d.clone();
        larger.size = Some(80.0);
        let m = measure_level2(
            &mut e,
            &d,
            &[StyleSpan {
                start: 0,
                end: 6,
                content: larger,
            }],
        )
        .unwrap();
        assert!(m.layout_bounds.width() > base.layout_bounds.width());
        let mut bold = d.clone();
        bold.axes = vec![("wght".into(), 900.0)];
        let b = e.measure_text(&bold).unwrap();
        assert_ne!(base.layout_bounds.width(), b.layout_bounds.width());
    }
    #[test]
    fn missing_fonts_and_uncovered_glyphs_are_explicit_failures() {
        assert_eq!(
            Level2::new().measure_text(&content("hello")),
            Err(MeasureError::NoFonts)
        );
        assert!(matches!(
            engine().measure_text(&content("\u{10ffff}")),
            Err(MeasureError::MissingGlyphs(_))
        ));
    }
    #[test]
    fn measurement_json_round_trip_is_lossless() {
        let m = engine().measure_text(&content("Motion\nDesign")).unwrap();
        assert_eq!(
            serde_json::from_str::<TextMeasurement>(&serde_json::to_string(&m).unwrap()).unwrap(),
            m
        );
    }
    #[cfg(feature = "ink")]
    #[test]
    fn font_ink_is_available_without_a_renderer_and_is_not_an_advance_box() {
        let bytes = font();
        let mut e = engine();
        let d = content("j  ");
        let m = measure_level2_with_ink(&mut e, &d, &[], |key| (key == "geist").then_some(&bytes))
            .unwrap();
        let InkBounds::Bounds(ink) = m.ink else {
            panic!("visible font ink")
        };
        assert!(ink.width() > 0.0);
        assert!(ink.height() > 0.0);
        assert_ne!(ink, m.layout_bounds);
        let blank =
            measure_level2_with_ink(&mut e, &content("   "), &[], |_| Some(&bytes)).unwrap();
        assert_eq!(blank.ink, InkBounds::Empty);
        assert!(matches!(
            measure_level2_with_ink(&mut e, &d, &[], |_| None),
            Err(MeasureError::MissingFace(_))
        ));
    }
}

#[test]
fn compatibility_source_offsets_cannot_place_a_caret_inside_a_surrogate_pair() {
    let mut e = Compat::new(ReadyFont::new());
    let d = content("A😀B");
    let m = e.measure_text(&d).unwrap();
    assert_eq!(m.source_length, 4);
    assert!(m.carets(2).is_empty());
    assert!(!m.carets(1).is_empty());
    assert!(!m.carets(3).is_empty());
}
#[cfg(feature = "level2")]
#[test]
fn native_carets_follow_visual_rtl_order_while_source_offsets_stay_logical() {
    let m = engine()
        .measure_text(&content("\u{202e}abc\u{202c}"))
        .unwrap();
    assert!(m.lines[0].caret.windows(2).any(|p| p[1].x < p[0].x));
    assert!(
        m.lines[0]
            .caret
            .windows(2)
            .all(|p| p[0].index <= p[1].index)
    );
}
