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
                let u =
                    |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
                [u(p[0]), u(p[1]), u(p[2]), a]
            }
        })
        .collect()
}

include!("main_thread_fonts.rs");

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
    let mut add =
        |registry: &pm_text::registry::CoreTextRegistry, family: &str, blob: &str, index: u32| {
            if let Some(bytes) = registry.blob(blob) {
                let data = single_face(bytes.as_slice(), index);
                l2.add_font(
                    &format!("{blob}#{index}"),
                    pm_text::font::FontBytes::new(data),
                    family,
                );
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
    let Some(map) = root.as_object_mut() else {
        return out;
    };
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
            if let Some(a) = c
                .as_object_mut()
                .and_then(|c| c.get_mut("layers"))
                .and_then(|l| l.as_array_mut())
            {
                out.push(a);
            }
        }
    }
    out
}

fn is_text_layer(l: &pm_model::Value) -> bool {
    matches!(
        l.as_object()
            .and_then(|o| o.get("type"))
            .and_then(|t| t.as_str()),
        Some("text" | "captions")
    )
}

/// Whether any composition of the document holds text or captions layers.
pub fn has_text(doc: &pm_model::Document) -> bool {
    let Some(mut root) = doc.to_value() else {
        return false;
    };
    layer_lists(&mut root)
        .iter()
        .any(|l| l.iter().any(is_text_layer))
}

/// The document with every text and captions layer at opacity 0 (drawn by
/// nothing, every other layer unchanged).
pub fn without_text(doc: &pm_model::Document) -> pm_model::Document {
    let mut root = doc.to_value().expect("a document");
    for list in layer_lists(&mut root) {
        for l in list.iter_mut().filter(|l| is_text_layer(l)) {
            let Some(p) = l
                .as_object_mut()
                .and_then(|o| o.get_mut("p"))
                .and_then(|p| p.as_object_mut())
            else {
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

/// The corpus ids the native level 2 semantic gate judges
/// (`tests/text_level2_semantics.rs`, over the shared fixture).
pub fn semantic_gate_ids() -> std::collections::BTreeSet<String> {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../../../corpus/fixtures/text-level2.json"))
            .expect("the level 2 corpus fixture");
    cases
        .iter()
        .map(|c| c["id"].as_str().expect("a case id").to_owned())
        .collect()
}

/// Level 2 text documents a legacy capture holds. Today's engine draws them
/// at level 1, so the capture is no level 2 reference: a parity gate routes
/// them here instead of comparing pixels, and every routed project must be
/// one the semantic gate judges. Routing is counted, printed and asserted;
/// nothing is skipped silently.
#[derive(Default)]
pub struct Level2Routes {
    gate: std::collections::BTreeSet<String>,
    /// Routed project label to its level 2 text layer ids.
    pub routed: std::collections::BTreeMap<String, Vec<String>>,
    /// Routed projects the semantic gate does not judge.
    uncovered: Vec<String>,
}

impl Level2Routes {
    pub fn new() -> Self {
        Self::with_gate(semantic_gate_ids())
    }

    pub fn with_gate(gate: std::collections::BTreeSet<String>) -> Self {
        Self {
            gate,
            ..Self::default()
        }
    }

    /// Whether the project's document holds level 2 text (recorded under
    /// `label`); `name` is its corpus id.
    pub fn route(&mut self, label: &str, name: &str, doc: &pm_model::Document) -> bool {
        let layers = pm_eval::text_level2::level2_text_layers(doc);
        if layers.is_empty() {
            return false;
        }
        if !self.gate.contains(name) {
            self.uncovered.push(label.to_owned());
        }
        self.routed.insert(label.to_owned(), layers);
        true
    }

    /// One failure per routed project the semantic gate does not judge.
    pub fn failures(&self) -> Vec<String> {
        self.uncovered
            .iter()
            .map(|label| {
                format!(
                    "{label}: level 2 text with no legacy reference, and the semantic gate (tests/text_level2_semantics.rs) does not judge it"
                )
            })
            .collect()
    }

    pub fn report(&self) {
        println!(
            "\nlevel 2 text routed to the semantic gate (no legacy reference): {} projects",
            self.routed.len()
        );
        for (label, layers) in &self.routed {
            let covered = !self.uncovered.contains(label);
            println!(
                "  {label:<44} layers {}{}",
                layers.join(", "),
                if covered { "" } else { "  NOT JUDGED" }
            );
        }
    }
}
