//! Lazy Parley font collection from the existing registry/replay boundary.
//! Resolving the requested typography first uses the same font queries in
//! native and WASM hosts. Parley owns the final shaping and line breaking.

use pm_text::{
    compat::{Compat, TextContent},
    font::{FontBytes, FontDesc, FontResolver},
    level2::{Level2, StyleSpan},
};
use std::collections::{HashMap, HashSet};

pub(super) fn shape<R: FontResolver>(
    compat: &mut Compat<R>,
    split: &mut HashMap<(u64, u32), FontBytes>,
    d: &TextContent,
    spans: &[StyleSpan],
) -> Result<crate::text_level2::Shaped, String> {
    let mut engine = Level2::new();
    let mut faces = HashMap::new();
    let mut added = HashSet::new();
    let mut fallback = Vec::new();
    let mut typography = vec![d];
    for span in spans {
        if !typography.contains(&&span.content) {
            typography.push(&span.content);
        }
    }
    for content in typography {
        let family = content.font.as_deref().unwrap_or("system-ui");
        let desc = FontDesc {
            size: content.size() as f32,
            weight: content
                .weight
                .filter(|w| *w != 0.0 && !w.is_nan())
                .unwrap_or(500.0)
                .clamp(1.0, 1000.0) as f32,
            italic: content.italic.unwrap_or(false),
        };
        let variable = (!content.axes.is_empty())
            .then(|| compat.resolver_mut().variable_face(family))
            .flatten();
        let primary = variable
            .or_else(|| compat.resolver_mut().family(family, &desc))
            .or_else(|| compat.resolver_mut().family("system-ui", &desc))
            .or_else(|| compat.resolver_mut().standard(&desc));
        // Resolve fallback for all characters, not just those in the span:
        // fonts can affect shaping on either side of a script boundary.
        let runs = compat.text_runs(content, &d.text, 0.0).0;
        let ids = primary.into_iter().chain(runs.iter().map(|r| r.face));
        for id in ids {
            let info = compat.resolver().face(id);
            let key = format!("{}#{}", info.blob, info.index);
            if !faces.contains_key(&key) {
                let identity = (info.data.id(), info.index);
                let data = match split.get(&identity) {
                    Some(data) => data.clone(),
                    None => {
                        let data = single_face(&info.data, info.index)?;
                        split.insert(identity, data.clone());
                        data
                    }
                };
                faces.insert(key.clone(), data);
            }
            let alias = if Some(id) == primary {
                family.to_owned()
            } else {
                format!("pm-fallback-{key}")
            };
            if added.insert((key.clone(), alias.clone())) {
                engine.add_font(&key, faces[&key].clone(), &alias);
            }
            if !fallback.contains(&alias) {
                fallback.push(alias);
            }
        }
    }
    if faces.is_empty() && !d.text.is_empty() {
        return Err("level 2 font bytes unavailable".into());
    }
    engine.set_fallback_families(&fallback);
    let layout = engine.layout_styled(d, spans);
    if layout.missing_glyphs > 0 {
        return Err("level 2 text whose fonts are not loaded".into());
    }
    Ok(crate::text_level2::Shaped { layout, faces })
}

fn single_face(data: &FontBytes, index: u32) -> Result<FontBytes, String> {
    use skrifa::raw::FileRef;
    let file = FileRef::new(data.as_slice()).map_err(|e| format!("level 2 font: {e}"))?;
    if matches!(file, FileRef::Font(_)) {
        return Ok(data.clone());
    }
    let font = file
        .fonts()
        .nth(index as usize)
        .ok_or("level 2 font collection index")?
        .map_err(|e| e.to_string())?;
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
    Ok(FontBytes::new(pm_text::sfnt::build_sfnt(&mut tables)))
}
