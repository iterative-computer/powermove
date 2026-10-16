//! Text class scoring (plan section 25): SSIM over ink windows.

#![allow(dead_code)]

/// `(ink SSIM, full SSIM, ink ratio)`: `pm_render::parity::text_ssim`.
pub fn ink_ssim(reference: &[u8], test: &[u8], w: usize, h: usize) -> (f64, f64, f64) {
    pm_render::parity::text_ssim(reference, test, w, h)
}

/// Premultiplied RGBA8 to straight (for viewing), or as it is.
pub fn png_rgba(px: &[u8], unpremultiply: bool) -> Vec<u8> {
    if !unpremultiply {
        return px.to_vec();
    }
    px.chunks_exact(4)
        .flat_map(|p| {
            let a = p[3];
            if a == 0 {
                [0, 0, 0, 0]
            } else {
                let u = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
                [u(p[0]), u(p[1]), u(p[2]), a]
            }
        })
        .collect()
}

/// The CoreText registry's level 1 measure, marked `Send` for the
/// renderer: tests that use it run without the harness, on the main thread,
/// and never move it to another thread (AppKit's font manager is
/// main-thread only). Hosts give the renderer a `Send` resolver (recorded
/// registry answers over full fonts).
#[cfg(target_os = "macos")]
pub struct MainThreadFonts(
    pub pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>,
    pub Option<pm_render::text::Level2Fonts>,
);

// SAFETY: see the type's documentation; the value stays on the main thread.
#[cfg(target_os = "macos")]
unsafe impl Send for MainThreadFonts {}

#[cfg(target_os = "macos")]
impl pm_render::text::TextFonts for MainThreadFonts {
    fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
        &mut self.0
    }
    fn line_runs(&mut self, d: &pm_text::compat::TextContent) -> Vec<pm_text::compat::LineRuns> {
        self.0.compat().line_runs(d)
    }
    fn text_runs(
        &mut self,
        d: &pm_text::compat::TextContent,
        text: &str,
        tracking: f64,
    ) -> Vec<pm_text::compat::GlyphRun> {
        self.0.compat().text_runs(d, text, tracking).0
    }
    fn face(&mut self, id: pm_text::font::FaceId) -> &pm_text::font::FaceInfo {
        use pm_text::font::FontResolver;
        self.0.compat().resolver().face(id)
    }
    fn em_advance(&mut self, face: pm_text::font::FaceId, size: f32) -> Option<f32> {
        use pm_text::font::FontResolver;
        self.0.compat().resolver_mut().em_advance(face, size)
    }
    fn level2(&mut self) -> Option<&mut pm_render::text::Level2Fonts> {
        self.1.as_mut()
    }
}

/// One face of a font file as its own font (collections split).
#[cfg(target_os = "macos")]
fn single_face(data: &[u8], index: u32) -> Vec<u8> {
    let file = read_fonts::FileRef::new(data).unwrap();
    if matches!(file, read_fonts::FileRef::Font(_)) {
        return data.to_vec();
    }
    let font = file.fonts().nth(index as usize).unwrap().unwrap();
    let mut tables: Vec<(u32, Vec<u8>)> = font
        .table_directory()
        .table_records()
        .iter()
        .filter_map(|r| {
            Some((
                u32::from_be_bytes(r.tag().to_be_bytes()),
                font.table_data(r.tag())?.as_bytes().to_vec(),
            ))
        })
        .collect();
    pm_text::sfnt::build_sfnt(&mut tables)
}

/// Level 2 fonts from the CoreText registry: the `system-ui` fallback
/// manifest's faces and every face of `families`, full fonts.
#[cfg(target_os = "macos")]
pub fn level2_fonts(
    registry: &mut pm_text::registry::CoreTextRegistry,
    families: &[&str],
) -> pm_render::text::Level2Fonts {
    let manifest = registry.fallback_manifest("system-ui");
    let mut l2 = pm_render::text::Level2Fonts::default();
    let mut add = |registry: &pm_text::registry::CoreTextRegistry, family: &str, blob: &str, index: u32| {
        if let Some(bytes) = registry.blob(blob) {
            let data = single_face(bytes.as_slice(), index);
            l2.add_font(&format!("{blob}#{index}"), pm_text::font::FontBytes::new(data), family);
        }
    };
    for f in &manifest.faces {
        add(registry, &f.family, &f.source.blob, f.source.index);
    }
    for family in families {
        for source in registry.family_faces(family) {
            add(registry, family, &source.blob, source.index);
        }
    }
    l2.set_manifest(&manifest);
    l2
}

fn layer_lists(root: &mut pm_model::Value) -> Vec<&mut Vec<pm_model::Value>> {
    let mut out = Vec::new();
    let Some(map) = root.as_object_mut() else { return out };
    let mut comps = None;
    for (k, v) in map.iter_mut() {
        match k.to_string_lossy().as_str() {
            "layers" => {
                if let Some(a) = v.as_array_mut() {
                    out.push(a);
                }
            }
            "comps" => comps = Some(v),
            _ => {}
        }
    }
    if let Some(comps) = comps.and_then(|c| c.as_object_mut()) {
        for (_, c) in comps.iter_mut() {
            if let Some(a) = c.as_object_mut().and_then(|c| c.get_mut("layers")).and_then(|l| l.as_array_mut()) {
                out.push(a);
            }
        }
    }
    out
}

fn is_text_layer(l: &pm_model::Value) -> bool {
    matches!(
        l.as_object().and_then(|o| o.get("type")).and_then(|t| t.as_str()),
        Some("text" | "captions")
    )
}

/// Whether any composition of the document holds text or captions layers.
pub fn has_text(doc: &pm_model::Document) -> bool {
    let Some(mut root) = doc.to_value() else { return false };
    layer_lists(&mut root).iter().any(|l| l.iter().any(is_text_layer))
}

/// The document with every text and captions layer at opacity 0 (drawn by
/// nothing, every other layer unchanged).
pub fn without_text(doc: &pm_model::Document) -> pm_model::Document {
    let mut root = doc.to_value().expect("a document");
    for list in layer_lists(&mut root) {
        for l in list.iter_mut().filter(|l| is_text_layer(l)) {
            let Some(p) = l.as_object_mut().and_then(|o| o.get_mut("p")).and_then(|p| p.as_object_mut()) else {
                continue;
            };
            let mut ch = pm_model::json::Map::new();
            ch.insert("v", pm_model::Value::Number(0.0));
            ch.insert("kf", pm_model::Value::Array(Vec::new()));
            ch.insert("expr", pm_model::Value::Null);
            p.insert("opacity", pm_model::Value::Object(ch));
        }
    }
    pm_model::Document::new(Some(root), doc.revision() + 1)
}
