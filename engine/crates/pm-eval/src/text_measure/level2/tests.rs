use super::*;
use crate::text_geom::TextMeasure;
use crate::text_measure::CompatMeasure;
use crate::text_measure::test_fonts::{Fonts, collection, font};

fn content(text: &str, family: &str, weight: f64) -> TextContent {
    TextContent {
        text: text.into(),
        font: Some(family.into()),
        weight: Some(weight),
        size: Some(20.0),
        ..Default::default()
    }
}

fn copy(fonts: &Fonts) -> Fonts {
    Fonts {
        faces: fonts.faces.clone(),
        families: fonts.families.clone(),
        bold_from: fonts.bold_from,
    }
}

/// What a fresh collection (today's per-request `Level2::new()`) lays out.
fn fresh(fonts: &Fonts, d: &TextContent, spans: &[StyleSpan]) -> Level2Layout {
    let mut compat = Compat::new(copy(fonts));
    shape(&mut compat, &mut Level2Cache::default(), d, spans)
        .expect("fresh layout")
        .layout
}

/// Every bound holds, and Parley references only cached faces.
fn assert_bounded(cache: &Level2Cache) {
    assert!(cache.faces.len() <= cache.face_limit_entries);
    assert!(cache.faces.bytes() <= cache.face_limit_bytes);
    assert!(cache.sets.len() <= SET_LIMIT_ENTRIES);
    assert!(cache.layouts.len() <= LAYOUT_LIMIT_ENTRIES);
    assert!(cache.layout_bytes <= LAYOUT_LIMIT_BYTES);
    assert_eq!(
        cache.layout_bytes,
        cache.layouts.iter().map(|e| e.bytes).sum::<usize>()
    );
    let cached: HashSet<u64> = cache.faces.values().map(|f| f.font.id()).collect();
    let signatures = cache
        .sets
        .keys()
        .chain(cache.active.iter())
        .chain(cache.layouts.iter().map(|e| &e.signature));
    for signature in signatures {
        for r in &signature.registrations {
            assert!(cached.contains(&r.font), "{} outlived its face", r.key);
        }
    }
    if let Some(active) = &cache.active {
        assert!(cache.sets.keys().any(|s| s == active));
    }
}

fn run_fonts(layout: &Level2Layout) -> Vec<&str> {
    let mut keys: Vec<&str> = layout.runs.iter().map(|r| r.font.as_str()).collect();
    keys.dedup();
    keys
}

#[test]
fn collections_and_layouts_are_reused_across_requests() {
    let mut fonts = Fonts::new();
    fonts.add("Sans", 400, "sans", FontBytes::new(font(400, 500, 400)), 0);
    fonts.add("Mono", 400, "mono", FontBytes::new(font(400, 600, 300)), 0);
    let a = content("Hello world", "Sans", 400.0);
    let b = content("abc", "Mono", 400.0);
    let (want_a, want_b) = (fresh(&fonts, &a, &[]), fresh(&fonts, &b, &[]));
    let mut compat = Compat::new(fonts);
    let mut cache = Level2Cache::default();
    for _ in 0..3 {
        let got = shape(&mut compat, &mut cache, &a, &[]).unwrap().layout;
        assert_eq!(got, want_a);
        let got = shape(&mut compat, &mut cache, &b, &[]).unwrap().layout;
        assert_eq!(got, want_b);
        assert_bounded(&cache);
    }
    let c = cache.counters;
    assert_eq!((c.face_splits, c.face_hits), (2, 4));
    // Each collection is built and registered once, then swapped back in.
    assert_eq!((c.set_builds, c.registrations, c.set_hits), (2, 2, 4));
    assert_eq!((c.layouts, c.layout_hits), (2, 4));
    assert_eq!(run_fonts(&want_a), ["sans#0"]);
    // Text that changes reuses the collection and shapes again.
    let edited = content("Hello world!", "Sans", 400.0);
    let got = shape(&mut compat, &mut cache, &edited, &[]).unwrap().layout;
    assert_eq!(got, fresh(&copy(compat.resolver()), &edited, &[]));
    assert_eq!((cache.counters.set_builds, cache.counters.layouts), (2, 3));
}

#[test]
fn mixed_weights_of_one_family_alternate_without_stale_faces() {
    let mut fonts = Fonts::new();
    fonts.add(
        "Serif",
        400,
        "serif-r",
        FontBytes::new(font(400, 500, 400)),
        0,
    );
    fonts.add(
        "Serif",
        700,
        "serif-b",
        FontBytes::new(font(700, 560, 480)),
        0,
    );
    // A weight 450 request resolves the bold face. CSS matching in a
    // collection that still held the regular face would choose regular.
    fonts.bold_from = 450.0;
    let mixed = content("Hello world", "Serif", 400.0);
    let spans = [StyleSpan {
        start: 6,
        end: 11,
        content: content("Hello world", "Serif", 700.0),
    }];
    let bold = content("Hello world", "Serif", 450.0);
    let want_mixed = fresh(&fonts, &mixed, &spans);
    let want_bold = fresh(&fonts, &bold, &[]);
    assert_eq!(run_fonts(&want_mixed), ["serif-r#0", "serif-b#0"]);
    assert_eq!(run_fonts(&want_bold), ["serif-b#0"]);
    let mut compat = Compat::new(fonts);
    let mut cache = Level2Cache::default();
    for _ in 0..4 {
        let got = shape(&mut compat, &mut cache, &mixed, &spans)
            .unwrap()
            .layout;
        assert_eq!(got, want_mixed);
        let got = shape(&mut compat, &mut cache, &bold, &[]).unwrap().layout;
        assert_eq!(got, want_bold);
        assert_bounded(&cache);
    }
    // One collection per signature: both weights under "Serif", then the
    // bold face alone. Alternating never re-registers.
    let c = cache.counters;
    assert_eq!((c.set_builds, c.registrations, c.set_evictions), (2, 3, 0));
    assert_eq!((c.face_splits, c.face_evictions), (2, 0));
    assert_eq!((c.layouts, c.layout_hits), (2, 6));
}

#[test]
fn least_recently_used_faces_leave_every_collection_and_layout() {
    let mut fonts = Fonts::new();
    for i in 0..5u16 {
        let name = format!("F{i}");
        fonts.add(
            &name,
            400,
            &name,
            FontBytes::new(font(400, 500 + i, 400)),
            0,
        );
    }
    let d = |i: usize| content("abc", &format!("F{i}"), 400.0);
    let identity = |compat: &Compat<Fonts>, i: usize| (compat.resolver().faces[i].data.id(), 0);
    let mut compat = Compat::new(fonts);
    let mut cache = Level2Cache {
        face_limit_entries: 3,
        ..Default::default()
    };
    for i in [0, 1, 2, 0, 3] {
        shape(&mut compat, &mut cache, &d(i), &[]).unwrap();
        assert_bounded(&cache);
    }
    // F1 was the least recently used face: F0 was used again since.
    let kept: Vec<bool> = (0..4)
        .map(|i| cache.faces.keys().any(|k| *k == identity(&compat, i)))
        .collect();
    assert_eq!(kept, [true, false, true, true]);
    assert_eq!((cache.counters.face_evictions, cache.sets.len()), (1, 3));
    assert_eq!(cache.layouts.len(), 3);

    // A byte ceiling of two faces keeps the two most recent.
    let size = font(400, 500, 400).len();
    cache.face_limit_entries = FACE_LIMIT_ENTRIES;
    cache.face_limit_bytes = 2 * size + 1;
    shape(&mut compat, &mut cache, &d(4), &[]).unwrap();
    assert_bounded(&cache);
    let kept: Vec<bool> = (0..5)
        .map(|i| cache.faces.keys().any(|k| *k == identity(&compat, i)))
        .collect();
    assert_eq!(kept, [false, false, false, true, true]);
    assert_eq!(cache.sets.len(), 2);

    // A request whose own faces exceed the ceiling still lays out; the
    // cache keeps nothing of it once it returns, Parley included.
    cache.face_limit_bytes = size - 1;
    let shaped = shape(&mut compat, &mut cache, &d(0), &[]).unwrap();
    assert_eq!(run_fonts(&shaped.layout), ["F0#0"]);
    assert!(shaped.faces.contains_key("F0#0"));
    assert_bounded(&cache);
    assert_eq!(
        (cache.faces.len(), cache.sets.len(), cache.layouts.len()),
        (0, 0, 0)
    );
    assert!(cache.active.is_none());
    assert_eq!(cache.engine.families().count(), 0);
}

#[test]
fn new_bytes_under_a_registry_key_replace_faces_layouts_and_glyph_boxes() {
    let mut fonts = Fonts::new();
    fonts.add("Sans", 400, "sans", FontBytes::new(font(400, 500, 400)), 0);
    let mut measure = CompatMeasure::new(fonts);
    let d = content("Hello", "Sans", 400.0);
    let first = measure.level2(&d, &[]).unwrap();
    let run = &first.layout.runs[0];
    let data = &first.faces["sans#0"];
    let ink = measure.level2_ink(data, run, run.glyphs[0].id);
    // The glyph box is measured once.
    assert_eq!(measure.level2_ink(data, run, run.glyphs[0].id), ink);
    let stats = measure.level2_cache_stats();
    assert_eq!((stats.glyph_misses, stats.glyph_hits), (1, 1));
    assert!(stats.glyph_bytes > 0 && stats.glyph_bytes <= stats.glyph_limit_bytes);

    // The same registry key now serves other bytes (a replaced font, a
    // grown replay subset).
    measure.compat().resolver_mut().faces[0].data = FontBytes::new(font(400, 700, 600));
    let second = measure.level2(&d, &[]).unwrap();
    assert_eq!(second.layout.width, first.layout.width * 700.0 / 500.0);
    let run = &second.layout.runs[0];
    let data = &second.faces["sans#0"];
    assert_ne!(data.id(), first.faces["sans#0"].id());
    assert_ne!(measure.level2_ink(data, run, run.glyphs[0].id), ink);
    let stats = measure.level2_cache_stats();
    assert_eq!((stats.face_entries, stats.face_evictions), (1, 1));
    assert_eq!((stats.set_entries, stats.layout_entries), (1, 1));
    assert_eq!((stats.layouts, stats.glyph_misses), (2, 2));
    assert!(stats.retained_bytes <= stats.limit_bytes);
}

#[test]
fn collection_files_are_split_into_counted_copies() {
    let ttc = FontBytes::new(collection(&[(400, 500, 400), (700, 560, 480)]));
    let mut fonts = Fonts::new();
    fonts.add("Serif", 400, "serif", ttc.clone(), 0);
    fonts.add("Serif", 700, "serif", ttc.clone(), 1);
    let mut measure = CompatMeasure::new(fonts);
    let d = content("Hello", "Serif", 700.0);
    let shaped = measure.level2(&d, &[]).unwrap();
    assert_eq!(run_fonts(&shaped.layout), ["serif#1"]);
    assert!((shaped.layout.width - 5.0 * 560.0 * 20.0 / 1000.0).abs() < 1e-3);
    let face = &shaped.faces["serif#1"];
    assert!(matches!(
        skrifa::raw::FileRef::new(face.as_slice()),
        Ok(skrifa::raw::FileRef::Font(_))
    ));
    let stats = measure.level2_cache_stats();
    assert_eq!(stats.face_copied_bytes, face.as_slice().len());
    assert_eq!(stats.face_bytes, face.as_slice().len());
}

#[test]
fn layout_charges_count_every_span_text() {
    let text = "x".repeat(4000);
    let d = content(&text, "Sans", 400.0);
    let spans: Vec<StyleSpan> = (0..8)
        .map(|i| StyleSpan {
            start: i * 500,
            end: i * 500 + 500,
            content: content(&text, "Sans", 400.0),
        })
        .collect();
    let entry = LayoutEntry {
        signature: Signature {
            registrations: Vec::new(),
            fallback: Vec::new(),
        },
        content: d,
        spans,
        layout: Level2Layout::default(),
        bytes: 0,
    };
    // Nine copies of the text: the content and each span's.
    assert!(layout_entry_bytes(&entry) >= 9 * 4000);
}

#[test]
fn hot_face_family_aliases_and_collection_keys_are_bounded() {
    let mut fonts = Fonts::new();
    fonts.add("Sans", 400, "sans", FontBytes::new(font(400, 500, 400)), 0);
    let mut compat = Compat::new(fonts);
    let mut cache = Level2Cache::default();
    shape(&mut compat, &mut cache, &content("abc", "Sans", 400.0), &[]).unwrap();
    let identity = (compat.resolver().faces[0].data.id(), 0);
    for i in 0..100 {
        cache.blob(identity, &format!("family-{i}"));
    }
    let face = cache.faces.peek(&identity).unwrap();
    assert_eq!(face.blobs.len(), FACE_ALIAS_LIMIT_ENTRIES);
    assert!(face.blobs.iter().map(|(a, _)| a.capacity()).sum::<usize>() <= FACE_ALIAS_LIMIT_BYTES);
    let hot = cache.blob(identity, "family-99").id();
    let huge = "x".repeat(FACE_ALIAS_LIMIT_BYTES + 1);
    cache.blob(identity, &huge);
    assert_eq!(cache.blob(identity, "family-99").id(), hot);
    assert!(
        cache
            .faces
            .peek(&identity)
            .unwrap()
            .blobs
            .iter()
            .all(|(a, _)| a != &huge)
    );

    let mut signature = cache.active.clone().unwrap();
    signature.fallback.push("x".repeat(SET_KEY_LIMIT_BYTES));
    cache.activate(&signature);
    cache.trim();
    assert!(cache.sets.bytes() <= SET_KEY_LIMIT_BYTES);
    assert!(cache.active.is_none());
    assert_eq!(cache.engine.families().count(), 0);
    assert_bounded(&cache);
}
