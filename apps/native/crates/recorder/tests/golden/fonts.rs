//! Linux font-provider double using only the recorder's generated geometric font.
use pm_recorder::TextEngine;
use pm_text::font::{FaceFacts, FaceInfo, FontBytes, FontDesc, FontResolver};
pub struct Fonts {
    info: FaceInfo,
}
pub fn engine() -> TextEngine<Fonts> {
    let data = include_bytes!("../fixtures/recorder-test.ttf").to_vec();
    let facts = FaceFacts::of(&data, 0);
    TextEngine::new(Fonts {
        info: FaceInfo {
            data: FontBytes::new(data),
            index: 0,
            blob: "synthetic".into(),
            coords: vec![],
            synthetic_bold: false,
            synthetic_italic: false,
            custom: false,
            web: None,
            synthesized_hyphens: [None; 2],
            postscript_name: "RecorderTest".into(),
            typeface_key: "synthetic".into(),
            fallback_key: None,
            facts,
        },
    })
}
impl FontResolver for Fonts {
    fn face(&self, _: u32) -> &FaceInfo {
        &self.info
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
