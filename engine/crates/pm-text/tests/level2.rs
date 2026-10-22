//! Level 2 layout on Parley: breaking, font-metric baselines, alignment,
//! fallback from the manifest, carets, and identity between full fonts and
//! metrics-and-shaping subsets. Uses fonts that ship with macOS.

#![cfg(all(target_os = "macos", feature = "level2"))]

use pm_text::compat::{TextContent, graphemes};
use pm_text::font::FontBytes;
use pm_text::level2::Level2;
use pm_text::registry::{FallbackManifest, ScriptFallback};
use pm_text::subset::metrics_subset;
use read_fonts::{FileRef, FontRef, TableProvider};

const FONTS: &[(&str, &str)] = &[
    ("Georgia", "/System/Library/Fonts/Supplemental/Georgia.ttf"),
    (
        "Georgia",
        "/System/Library/Fonts/Supplemental/Georgia Bold.ttf",
    ),
    (
        "Georgia",
        "/System/Library/Fonts/Supplemental/Georgia Italic.ttf",
    ),
    ("System Font", "/System/Library/Fonts/SFNS.ttf"),
    (
        "Hiragino Sans GB",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ),
    (
        "Apple Color Emoji",
        "/System/Library/Fonts/Apple Color Emoji.ttc",
    ),
    ("Geeza Pro", "/System/Library/Fonts/GeezaPro.ttc"),
];

fn manifest() -> FallbackManifest {
    FallbackManifest {
        base_family: "System Font".into(),
        scripts: vec![
            ScriptFallback {
                script: "Hani".into(),
                language: None,
                families: vec!["Hiragino Sans GB".into()],
            },
            ScriptFallback {
                script: "Arab".into(),
                language: None,
                families: vec!["Geeza Pro".into()],
            },
        ],
        emoji: Some("Apple Color Emoji".into()),
        last_resort: None,
        faces: Vec::new(),
    }
}

/// Every face of every font, full or reduced to its subset.
fn engine(subsets: bool) -> Level2 {
    let mut l2 = Level2::new();
    for (family, path) in FONTS {
        let data = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let faces = FileRef::new(&data).unwrap().fonts().count();
        for index in 0..faces as u32 {
            let key = format!("{path}#{index}");
            let bytes = if subsets {
                metrics_subset(&data, index).unwrap().0
            } else if faces == 1 {
                data.clone()
            } else {
                // One face of a collection as its own font with every table, as subsets are.
                single_face(&data, index)
            };
            assert!(
                l2.add_font(&key, FontBytes::new(bytes), family) > 0,
                "{key}"
            );
        }
    }
    l2.set_manifest(&manifest());
    l2
}

fn single_face(data: &[u8], index: u32) -> Vec<u8> {
    let font = FileRef::new(data)
        .unwrap()
        .fonts()
        .nth(index as usize)
        .unwrap()
        .unwrap();
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

fn d(text: &str, font: &str, size: f64, box_width: f64) -> TextContent {
    TextContent {
        text: text.into(),
        font: Some(font.into()),
        size: Some(size),
        weight: Some(400.0),
        box_width: Some(box_width),
        paragraph: Some(box_width > 0.0),
        ..Default::default()
    }
}

#[test]
fn wraps_at_word_boundaries_within_the_box() {
    let mut l2 = engine(false);
    let c = d(
        "Motion design is the art of bringing graphics to life.",
        "Georgia",
        24.0,
        200.0,
    );
    let out = l2.layout(&c);
    assert!(out.lines.len() >= 3, "{} lines", out.lines.len());
    for l in &out.lines {
        assert!(
            l.width <= 200.0 + 1e-3,
            "line {:?} is {} px",
            l.text,
            l.width
        );
        assert!(
            !l.text.starts_with(' '),
            "line starts with a space: {:?}",
            l.text
        );
        // Breaks fall between words.
        let next = c
            .text
            .encode_utf16()
            .nth(l.end)
            .map(|u| u == u16::from(b' '))
            .unwrap_or(true);
        assert!(
            next || l.end == c.text.encode_utf16().count() || l.text.ends_with(' '),
            "line {:?} breaks inside a word",
            l.text
        );
    }
    assert_eq!(out.missing_glyphs, 0);
}

#[test]
fn long_words_break_at_graphemes_only() {
    let mut l2 = engine(false);
    let family = "👨‍👩‍👧‍👦";
    let text = format!("Supercalifragilistic{family}{family}{family}expialidocious");
    let out = l2.layout(&d(&text, "Georgia", 32.0, 90.0));
    assert!(out.lines.len() > 2);
    let units: Vec<u16> = text.encode_utf16().collect();
    let bounds: std::collections::BTreeSet<usize> = graphemes(&units)
        .iter()
        .map(|r| r.start)
        .chain([units.len()])
        .collect();
    for l in &out.lines {
        assert!(
            bounds.contains(&l.start) && bounds.contains(&l.end),
            "line {}..{} splits a grapheme",
            l.start,
            l.end
        );
    }
}

#[test]
fn baselines_come_from_font_metrics() {
    let mut l2 = engine(false);
    let mut c = d("One\nTwo\n\nFour", "Georgia", 40.0, 0.0);
    c.leading = Some(1.5);
    let out = l2.layout(&c);
    assert_eq!(out.lines.len(), 4);
    let data = std::fs::read(FONTS[0].1).unwrap();
    let font = FontRef::new(&data).unwrap();
    let upem = font.head().unwrap().units_per_em() as f64;
    let hhea = font.hhea().unwrap();
    let (asc, desc) = (
        hhea.ascender().to_i16() as f64 * 40.0 / upem,
        -hhea.descender().to_i16() as f64 * 40.0 / upem,
    );
    for (row, l) in out.lines.iter().enumerate() {
        assert!((l.height - 60.0).abs() < 1e-3, "line height {}", l.height);
        assert!((l.y - row as f64 * 60.0).abs() < 1e-3);
        if !l.text.is_empty() {
            // CSS half-leading: the baseline sits half the leading plus the ascent below the top.
            let expected = l.y + (60.0 - (asc + desc)) / 2.0 + asc;
            assert!(
                (l.baseline - expected).abs() < 0.05,
                "row {row}: baseline {} vs {expected}",
                l.baseline
            );
        }
    }
}

#[test]
fn range_typography_participates_in_shaping_wrapping_and_utf16_sources() {
    use pm_text::level2::StyleSpan;
    let mut full = engine(false);
    let mut subsets = engine(true);
    let content = d(
        "Office e\u{301} typography wraps here",
        "Georgia",
        24.0,
        200.0,
    );
    let mut large = content.clone();
    large.size = Some(60.0);
    large.weight = Some(700.0);
    let spans = [StyleSpan {
        start: 0,
        end: 6,
        content: large,
    }];
    let plain = full.layout(&content);
    let styled = full.layout_styled(&content, &spans);
    assert!(styled.lines[0].height > plain.lines[0].height);
    assert!(
        styled.lines[0].end < plain.lines[0].end,
        "range font metrics change wrapping"
    );
    assert!(styled.runs.iter().any(|r| r.size == 60.0));
    assert!(styled.runs.iter().any(|r| r.size == 24.0));
    assert_eq!(
        styled,
        subsets.layout_styled(&content, &spans),
        "full fonts and WASM subsets shape identically"
    );
    let starts: std::collections::BTreeSet<_> =
        graphemes(&content.text.encode_utf16().collect::<Vec<_>>())
            .into_iter()
            .map(|r| r.start)
            .collect();
    for run in &styled.runs {
        assert_eq!(run.glyphs.len(), run.sources.len());
        assert!(run.sources.iter().all(|s| starts.contains(s)));
    }
}

/// Every glyph's source, in visual order, across all runs of a line.
fn sources_by_line(out: &pm_text::level2::Level2Layout) -> Vec<Vec<usize>> {
    let mut lines = vec![Vec::new(); out.lines.len()];
    for run in &out.runs {
        assert_eq!(run.glyphs.len(), run.sources.len(), "glyph/source map");
        lines[run.line].extend(run.sources.iter().copied());
    }
    lines
}

/// Parley cuts one shaped run into several glyph runs where only a
/// non-shaping style changes (line height). Each glyph run must carry the
/// sources of its own glyphs, not the whole shaped run's from the start.
#[test]
fn glyph_runs_split_by_line_height_or_tracking_keep_their_own_sources() {
    use pm_text::level2::StyleSpan;
    let mut l2 = engine(false);
    let text = "Hello world again";
    let base = d(text, "System Font", 40.0, 0.0);
    let plain = l2.layout(&base);
    assert_eq!(plain.runs.len(), 1, "one run without styles");
    let expected = sources_by_line(&plain);
    assert_eq!(
        expected[0],
        (0..17).collect::<Vec<_>>(),
        "one glyph per letter"
    );
    for (name, change) in [
        (
            "leading",
            Box::new(|c: &mut TextContent| c.leading = Some(2.0)) as Box<dyn Fn(&mut TextContent)>,
        ),
        (
            "tracking",
            Box::new(|c: &mut TextContent| c.tracking = Some(6.0)),
        ),
        ("restated base", Box::new(|_: &mut TextContent| {})),
    ] {
        let mut styled = base.clone();
        change(&mut styled);
        let spans = [StyleSpan {
            start: 6,
            end: 11,
            content: styled,
        }];
        let out = l2.layout_styled(&base, &spans);
        assert_eq!(
            out.missing_glyphs, 0,
            "{name}: notdef counted once per cluster"
        );
        let total: usize = out.runs.iter().map(|r| r.glyphs.len()).sum();
        assert_eq!(total, 17, "{name}");
        assert_eq!(
            sources_by_line(&out),
            expected,
            "{name}: sources follow the glyphs"
        );
        // Each run's glyphs are the ones at its sources (same ids as unstyled).
        let ids: Vec<u32> = out
            .runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(|g| g.id))
            .collect();
        let plain_ids: Vec<u32> = plain.runs[0].glyphs.iter().map(|g| g.id).collect();
        assert_eq!(ids, plain_ids, "{name}");
        if name == "leading" {
            assert!(out.runs.len() >= 3, "line height splits the glyph run");
            let world: Vec<_> = out
                .runs
                .iter()
                .filter(|r| r.sources.iter().all(|s| (6..11).contains(s)))
                .collect();
            assert_eq!(world.len(), 1, "one glyph run holds exactly 'world'");
            assert_eq!(world[0].sources, (6..11).collect::<Vec<_>>());
        }
        if name == "restated base" {
            assert_eq!(out, plain, "a span equal to the base changes nothing");
        }
    }
    // A missing glyph inside a split run is counted once.
    let tofu = d("ab\u{0B95}cd", "Georgia", 40.0, 0.0);
    let mut tall = tofu.clone();
    tall.leading = Some(3.0);
    let spans = [StyleSpan {
        start: 1,
        end: 4,
        content: tall,
    }];
    let once = l2.layout(&tofu).missing_glyphs;
    assert_eq!(l2.layout_styled(&tofu, &spans).missing_glyphs, once);
}

/// Right-to-left glyph runs split by line height keep their visual-order
/// sources, and a ligature's glyph belongs to its first grapheme.
#[test]
fn split_rtl_runs_and_ligatures_keep_sources() {
    use pm_text::level2::StyleSpan;
    let mut l2 = engine(false);
    let arabic = d(
        "\u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064A}\u{0629}",
        "Geeza Pro",
        40.0,
        0.0,
    );
    let plain = l2.layout(&arabic);
    let mut tall = arabic.clone();
    tall.leading = Some(2.5);
    let spans = [StyleSpan {
        start: 2,
        end: 5,
        content: tall,
    }];
    let split = l2.layout_styled(&arabic, &spans);
    assert!(
        split.runs.len() > plain.runs.len(),
        "line height splits the run"
    );
    assert_eq!(sources_by_line(&split), sources_by_line(&plain));
    // Lam-alef is a mandatory ligature: one glyph for two graphemes.
    let ligature = d("\u{0628}\u{0644}\u{0627}\u{0628}", "Geeza Pro", 40.0, 0.0);
    let out = l2.layout(&ligature);
    let sources = &sources_by_line(&out)[0];
    assert_eq!(sources.len(), 3, "lam-alef ligates: {sources:?}");
    assert!(
        sources.contains(&1),
        "the ligature takes its first grapheme's source"
    );
    assert!(
        !sources.contains(&2),
        "the continuation draws nothing of its own"
    );
}

#[test]
fn alignment_places_lines_around_the_origin() {
    let mut l2 = engine(false);
    for (align, want) in [("left", 0.0), ("center", -0.5), ("right", -1.0)] {
        let mut c = d("Powermove", "Georgia", 48.0, 0.0);
        c.align = Some(align.into());
        let out = l2.layout(&c);
        let l = &out.lines[0];
        assert!(
            (l.x - want * l.width).abs() < 1e-3,
            "{align}: x {} width {}",
            l.x,
            l.width
        );
        let first = out.runs[0].glyphs[0].x;
        assert!(
            (first - l.x).abs() < 1e-3,
            "{align}: first glyph at {first}, line at {}",
            l.x
        );
    }
}

#[test]
fn falls_back_through_the_manifest() {
    let mut l2 = engine(false);
    let out = l2.layout(&d("Edit 编辑 مرحبا 👋🏽", "Georgia", 32.0, 0.0));
    assert_eq!(out.missing_glyphs, 0);
    let fonts: std::collections::BTreeSet<&str> =
        out.runs.iter().map(|r| r.font.as_str()).collect();
    assert!(fonts.iter().any(|f| f.contains("Georgia.ttf")), "{fonts:?}");
    assert!(
        fonts.iter().any(|f| f.contains("Hiragino Sans GB")),
        "{fonts:?}"
    );
    assert!(fonts.iter().any(|f| f.contains("GeezaPro")), "{fonts:?}");
    assert!(
        fonts.iter().any(|f| f.contains("Apple Color Emoji")),
        "{fonts:?}"
    );
}

#[test]
fn caret_boundaries_follow_graphemes() {
    let mut l2 = engine(false);
    let text = "Ae\u{301} 👍🏿x";
    let out = l2.layout(&d(text, "Georgia", 30.0, 0.0));
    let caret = &out.caret[0];
    let units: Vec<u16> = text.encode_utf16().collect();
    assert_eq!(caret.len(), graphemes(&units).len() + 1);
    assert!(
        caret
            .windows(2)
            .all(|w| w[1].x > w[0].x && w[1].index > w[0].index),
        "{caret:?}"
    );
    assert_eq!(caret.last().unwrap().index, units.len());
    assert!((caret.last().unwrap().x - out.lines[0].width).abs() < 1e-3);
    // Units: four non-space graphemes in two words.
    assert_eq!(out.units.characters.len(), 4);
    assert_eq!(out.units.words.len(), 2);
}

#[test]
fn variable_axes_and_optical_size_apply() {
    let mut l2 = engine(false);
    let mut light = d("Powermove", "System Font", 40.0, 0.0);
    light.axes = vec![("wght".into(), 200.0)];
    let mut heavy = light.clone();
    heavy.axes = vec![("wght".into(), 900.0)];
    let (a, b) = (l2.layout(&light), l2.layout(&heavy));
    assert!(
        b.lines[0].width > a.lines[0].width + 1.0,
        "{} vs {}",
        a.lines[0].width,
        b.lines[0].width
    );
    assert_ne!(a.runs[0].coords, b.runs[0].coords);
}

#[test]
fn box_height_keeps_whole_lines() {
    let mut l2 = engine(false);
    let mut c = d(
        "one two three four five six seven eight nine ten",
        "Georgia",
        20.0,
        80.0,
    );
    c.box_height = Some(50.0);
    let out = l2.layout(&c);
    assert_eq!(
        out.lines.len(),
        2,
        "{:?}",
        out.lines.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
}

#[test]
fn subsets_lay_out_identically_to_full_fonts() {
    let (mut full, mut sub) = (engine(false), engine(true));
    for c in [
        d(
            "Motion design is the art of bringing graphics to life. Edit 编辑 مرحبا 👋🏽",
            "Georgia",
            28.0,
            260.0,
        ),
        d(
            "AVATAR Tower WAVE office affinity",
            "System Font",
            64.0,
            0.0,
        ),
        {
            let mut v = d("AVATAR Tower", "System Font", 30.0, 0.0);
            v.axes = vec![("wght".into(), 650.0), ("wdth".into(), 80.0)];
            v
        },
        {
            let mut v = d("Bold italic text", "Georgia", 22.0, 0.0);
            v.weight = Some(700.0);
            v.italic = Some(true);
            v.tracking = Some(2.0);
            v
        },
    ] {
        assert_eq!(full.layout(&c), sub.layout(&c), "{:?}", c.text);
    }
}
