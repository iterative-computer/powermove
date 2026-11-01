//! Keeps pm-design, the design spec and the theme pipeline in step.
//!
//! - `docs/native/design/tokens.md` carries a generated table between markers.
//!   It must equal what these tokens produce. Regenerate with
//!   `PM_UPDATE_DESIGN_SNAPSHOT=1 cargo test -p pm-design`.
//! - Every token needs a role sentence below; a new field without one fails.
//! - The contrast and material rules in `language.md` are asserted here.
//! - The JS theme pipeline may only target fields that `Tokens::PATCHABLE` lists.
use pm_design::*;
use serde_json::Value;
use std::path::PathBuf;

const ROLES: &[(&str, &str)] = &[
    (
        "colors.window",
        "Window ground behind panels, toolbar band and status bar",
    ),
    (
        "colors.panel",
        "Panel material: every docked work surface and sheet content",
    ),
    (
        "colors.panel_secondary",
        "Recessed band inside a panel: timeline layer column, ruler",
    ),
    (
        "colors.viewer",
        "Viewer surround; the darkest neutral so footage reads true",
    ),
    (
        "colors.field",
        "Value wells, text fields, segmented control track",
    ),
    (
        "colors.floating",
        "Solid fallback for floating surfaces; lifted above panel",
    ),
    (
        "colors.hover",
        "Pointer hover fill on rows and borderless buttons",
    ),
    ("colors.separator", "Hairline separators and panel edges"),
    ("colors.text", "Primary text and glyphs"),
    ("colors.secondary", "Row labels, secondary text"),
    ("colors.muted", "Captions, metadata, placeholder, units"),
    (
        "colors.accent",
        "Powermove orange: primary button, playhead, selection marks",
    ),
    ("colors.selected_text", "Text and glyphs on an accent fill"),
    (
        "colors.selected_fill",
        "Neutral selected row / segment fill",
    ),
    ("colors.focus", "Keyboard focus ring"),
    (
        "colors.accent_text",
        "Accent as text or a thin mark on panels (4.5:1)",
    ),
    (
        "colors.accent_wash",
        "Opaque accent tint: drop targets, active guide bed",
    ),
    ("colors.danger", "Destructive and error text, offline media"),
    ("colors.warning", "Warnings, missing-but-recoverable states"),
    ("colors.success", "Completed, online, rendered"),
    (
        "colors.labels.blue",
        "Label: default for text and shape layers",
    ),
    (
        "colors.labels.violet",
        "Label: adjustment and effect layers",
    ),
    ("colors.labels.orange", "Label: generated / solid layers"),
    ("colors.labels.red", "Label: user colour"),
    ("colors.labels.yellow", "Label: user colour"),
    ("colors.labels.green", "Label: audio"),
    ("colors.labels.teal", "Label: video and image media"),
    ("colors.labels.gray", "Label: guides, nulls, cameras"),
    ("metrics.gap", "Gap between panels and to the window edge"),
    ("metrics.inset", "Content inset inside a panel"),
    ("metrics.panel_radius", "Panel corner radius"),
    (
        "metrics.control_radius",
        "Control radius inside a panel (panel radius − gap)",
    ),
    (
        "metrics.inner_radius",
        "Control nested in a control (segment in a track)",
    ),
    (
        "metrics.capsule_radius",
        "Toolbar capsule radius (height ÷ 2)",
    ),
    (
        "metrics.header",
        "Panel header, viewer bar and timeline header height",
    ),
    ("metrics.row", "Inspector and list row height"),
    ("metrics.status", "Status bar height"),
    ("metrics.toolbar", "Unified toolbar height"),
    ("metrics.toolbar_title_width", "Document title slot width"),
    ("metrics.toolbar_title_line", "Document title line height"),
    (
        "metrics.control",
        "Standalone control height (buttons, popups)",
    ),
    ("metrics.field", "Value well height inside a row"),
    (
        "metrics.value_column",
        "Inspector value column width shared by every row",
    ),
    ("metrics.keyframe", "Keyframe diamond size"),
    (
        "metrics.border",
        "Separator thickness (Increase Contrast doubles it)",
    ),
    ("metrics.min_panel", "Smallest panel extent"),
    ("metrics.min_center", "Smallest viewer extent"),
    ("metrics.keyboard_step", "Splitter arrow-key step"),
    (
        "metrics.drag_threshold",
        "Pointer travel before a drag starts",
    ),
    ("metrics.window_width", "Default window width"),
    ("metrics.window_height", "Default window height"),
    ("metrics.min_width", "Minimum window width"),
    ("metrics.min_height", "Minimum window height"),
    ("metrics.palette_width", "Command palette width"),
    ("metrics.palette_height", "Command palette maximum height"),
    ("metrics.field_menu_width", "Field menu width"),
    ("metrics.field_menu_height", "Field menu maximum height"),
    ("metrics.settings_width", "Settings window width"),
    ("metrics.settings_height", "Settings window height"),
    ("metrics.settings_sidebar", "Settings sidebar width"),
    ("metrics.menu_inset", "Menu and palette row inset"),
    ("metrics.menu_row", "Palette row height"),
    ("metrics.track_row", "Timeline layer row"),
    ("metrics.property_row", "Timeline property sub-row"),
    ("metrics.ruler", "Timeline ruler height"),
    ("metrics.layer_column", "Timeline layer list column width"),
    ("metrics.playhead", "Playhead line width"),
    ("metrics.symbol_toolbar", "SF Symbol size in toolbar items"),
    ("metrics.symbol_panel", "SF Symbol size in panel headers"),
    ("metrics.symbol_inline", "SF Symbol size inside rows"),
    ("metrics.hit", "Minimum pointer target"),
    ("metrics.popover_width", "Default popover width"),
    ("metrics.sheet_width", "Default sheet width"),
    ("metrics.agent_width", "Default agent panel width"),
    (
        "typography.minimum",
        "Smallest text anywhere: captions, ruler, status bar",
    ),
    (
        "typography.body",
        "Dense UI text: values, list rows, menus in panels",
    ),
    ("typography.label", "Inspector row labels"),
    (
        "typography.title",
        "Panel titles, section headings, window title (semibold)",
    ),
    ("typography.display", "Empty-state and onboarding display"),
    (
        "typography.reading",
        "Prose: agent replies, sheet explanations, Settings",
    ),
    (
        "typography.heading",
        "Sheet and Settings pane headings (semibold)",
    ),
    (
        "typography.timecode",
        "Transport timecode (medium, tabular figures)",
    ),
    ("typography.regular_weight", "Regular weight"),
    (
        "typography.medium_weight",
        "Medium weight: tabs, timecode, values being edited",
    ),
    (
        "typography.semibold_weight",
        "Semibold weight: titles, headings, primary buttons",
    ),
    (
        "typography.dense_leading",
        "Line height multiplier for dense UI",
    ),
    (
        "typography.reading_leading",
        "Line height multiplier for prose",
    ),
    ("typography.text_optical_size", "Optical size for text"),
    (
        "typography.display_optical_size",
        "Optical size for display",
    ),
    ("motion.enter", "Spring response for appearing surfaces (s)"),
    ("motion.bounce", "Spring bounce for appearing surfaces"),
    ("motion.exit", "Fade-out for dismissed surfaces (s)"),
    ("motion.reduced", "Cross-fade used under Reduce Motion (s)"),
    (
        "motion.start_scale",
        "Scale an appearing surface starts from",
    ),
    (
        "motion.interaction",
        "Hover, press and selection feedback (s)",
    ),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&key, v, out)
            }
        }
        _ => out.push((prefix.to_string(), v.clone())),
    }
}

fn leaves(t: &Tokens) -> Vec<(String, Value)> {
    let v = serde_json::to_value(t).unwrap();
    let mut out = Vec::new();
    for group in ["colors", "metrics", "typography", "motion"] {
        flatten(group, &v[group], &mut out);
    }
    out
}

fn show(key: &str, v: &Value) -> String {
    if key.starts_with("colors.") {
        format!("`#{:06X}`", v.as_u64().unwrap())
    } else {
        let n = v.as_f64().unwrap();
        let n = (n * 1000.).round() / 1000.;
        format!("{n}")
    }
}

fn variants() -> [Tokens; 4] {
    let ic = Accessibility {
        increase_contrast: true,
        ..Default::default()
    };
    [
        Tokens::new(Scheme::Light, Accessibility::default()),
        Tokens::new(Scheme::Dark, Accessibility::default()),
        Tokens::new(Scheme::Light, ic),
        Tokens::new(Scheme::Dark, ic),
    ]
}

fn table() -> String {
    let all = variants().map(|t| leaves(&t));
    let mut s = String::from(
        "| Token | Role | Light | Dark | Light, Increase Contrast | Dark, Increase Contrast |\n|---|---|---|---|---|---|\n",
    );
    for (i, (key, _)) in all[0].iter().enumerate() {
        let role = ROLES
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("{key} has no role sentence in tests/spec.rs"))
            .1;
        let cells: Vec<_> = all.iter().map(|l| show(key, &l[i].1)).collect();
        s += &format!("| `{key}` | {role} | {} |\n", cells.join(" | "));
    }
    s
}

#[test]
fn token_table_matches_spec_doc() {
    let path = root().join("docs/native/design/tokens.md");
    let doc = std::fs::read_to_string(&path).expect("docs/native/design/tokens.md");
    let (begin, end) = ("<!-- tokens:begin -->\n", "<!-- tokens:end -->");
    let a = doc.find(begin).expect("begin marker") + begin.len();
    let b = doc.find(end).expect("end marker");
    let generated = table();
    if std::env::var_os("PM_UPDATE_DESIGN_SNAPSHOT").is_some() {
        let next = format!("{}{}{}", &doc[..a], generated, &doc[b..]);
        std::fs::write(&path, next).unwrap();
        return;
    }
    assert_eq!(
        doc[a..b],
        generated,
        "tokens.md is stale: run PM_UPDATE_DESIGN_SNAPSHOT=1 cargo test -p pm-design"
    );
    for (key, _) in ROLES {
        assert!(
            leaves(&variants()[0]).iter().any(|(k, _)| k == key),
            "role for a token that no longer exists: {key}"
        );
    }
}

fn lum(c: u32) -> f64 {
    let ch = |s: u32| {
        let v = ((c >> s) & 255) as f64 / 255.;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
}
fn contrast(a: u32, b: u32) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn contrast_rules_from_language() {
    for t in variants() {
        let c = &t.colors;
        let ic = t.accessibility.increase_contrast;
        let name = format!("{:?} ic={ic}", t.scheme);
        for (bg_name, bg) in [
            ("panel", c.panel),
            ("field", c.field),
            ("floating", c.floating),
            ("window", c.window),
        ] {
            let text_floor = if ic { 7. } else { 4.5 };
            assert!(contrast(c.text, bg) >= 7., "{name}: text on {bg_name}");
            assert!(
                contrast(c.secondary, bg) >= text_floor,
                "{name}: secondary on {bg_name}"
            );
            assert!(contrast(c.muted, bg) >= 4.5, "{name}: muted on {bg_name}");
        }
        for (what, fg) in [
            ("accent_text", c.accent_text),
            ("danger", c.danger),
            ("warning", c.warning),
            ("success", c.success),
        ] {
            assert!(contrast(fg, c.panel) >= 4.5, "{name}: {what} on panel");
            assert!(contrast(fg, c.field) >= 4.3, "{name}: {what} on field");
        }
        assert!(
            contrast(c.selected_text, c.accent) >= 3.0,
            "{name}: label on accent fill (bold 13 pt = large text)"
        );
        assert!(
            contrast(c.accent, c.panel) >= 3.0,
            "{name}: accent mark on panel"
        );
        assert!(
            contrast(c.text, c.selected_fill) >= 7.,
            "{name}: selected row"
        );
        for (label, fill) in Labels::NAMES.iter().zip(c.labels.all()) {
            assert!(
                contrast(c.text, fill) >= 7.,
                "{name}: text on {label} label"
            );
        }
        if ic {
            assert!(
                contrast(c.separator, c.panel) >= 3.,
                "{name}: IC separators"
            );
            assert_eq!(t.metrics.border, 2.);
        }
        // Elevation: floating surfaces sit above panels, panels above the window,
        // and the viewer is the deepest neutral in dark.
        if t.scheme == Scheme::Dark {
            assert!(lum(c.floating) > lum(c.panel) && lum(c.panel) > lum(c.window));
            assert!(lum(c.viewer) <= lum(c.window));
            assert!(lum(c.field) > lum(c.panel));
        } else {
            assert!(lum(c.floating) >= lum(c.panel) && lum(c.panel) > lum(c.window));
        }
    }
}

#[test]
fn dark_is_neutral_grey() {
    let c = Tokens::new(Scheme::Dark, Accessibility::default()).colors;
    for v in [
        c.window,
        c.panel,
        c.panel_secondary,
        c.viewer,
        c.field,
        c.floating,
        c.hover,
        c.separator,
        c.text,
        c.secondary,
        c.muted,
        c.selected_fill,
    ] {
        let (r, g, b) = ((v >> 16) & 255, (v >> 8) & 255, v & 255);
        assert!(
            r == g && g == b,
            "#{v:06x} is tinted; dark neutrals are pure grey"
        );
    }
}

#[test]
fn concentric_radii_and_grid() {
    for t in variants() {
        let m = t.metrics;
        assert_eq!(
            m.control_radius,
            m.panel_radius - m.gap,
            "control nests in panel"
        );
        assert!(m.inner_radius <= m.control_radius - 2.);
        assert_eq!(
            m.capsule_radius * 2.,
            34.,
            "toolbar capsules are 34 pt tall"
        );
        assert!(m.field < m.row && m.control <= m.row);
        assert!(m.hit >= 24.);
        for v in [
            m.gap,
            m.inset,
            m.header,
            m.row,
            m.toolbar,
            m.track_row,
            m.ruler,
        ] {
            assert_eq!(v % 2., 0., "{v} is off the 2 pt grid");
        }
        let ty = t.typography;
        assert!(ty.minimum >= 11.);
        assert!(ty.reading > ty.body && ty.heading > ty.title && ty.display > ty.heading);
    }
}

#[test]
fn materials_follow_hig() {
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    for s in [
        Surface::Toolbar,
        Surface::FloatingBar,
        Surface::Popover,
        Surface::ViewerOverlay,
        Surface::Sheet,
        Surface::Menu,
        Surface::Palette,
    ] {
        assert_eq!(t.material(s, true), Material::Glass, "{s:?}");
    }
    for s in [
        Surface::Viewer,
        Surface::Timeline,
        Surface::InspectorRow,
        Surface::List,
        Surface::Panel,
        Surface::AgentPanel,
        Surface::StatusBar,
    ] {
        assert_eq!(
            t.material(s, true),
            Material::Solid,
            "{s:?} holds dense content"
        );
    }
}

#[test]
fn theme_patch_cannot_beat_increase_contrast() {
    let mut t = Tokens::new(
        Scheme::Dark,
        Accessibility {
            increase_contrast: true,
            ..Default::default()
        },
    );
    t.apply_patch([
        ("colors.separator", 0x222222 as f64),
        ("metrics.border", 0.5),
        ("colors.accent", 0x123456 as f64),
    ])
    .unwrap();
    assert_eq!(t.colors.separator, 0x888888);
    assert_eq!(t.metrics.border, 2.);
    assert_eq!(t.colors.accent, 0x123456);
    let before = t.clone();
    assert!(t.apply_patch([("colors.accent", 1.5)]).is_err());
    assert!(t.apply_patch([("colors.nope", 1.)]).is_err());
    assert!(t.apply_patch([("metrics.gap", f64::NAN)]).is_err());
    assert_eq!(t, before, "a failed patch changes nothing");
}

#[test]
fn theme_pipeline_targets_exist() {
    let src = std::fs::read_to_string(root().join("apps/native/js/themes/tokens.mjs")).unwrap();
    let a = src.find("const native = {").expect("native map");
    let b = a + src[a..].find("\n};").unwrap();
    let mut seen = 0;
    for part in src[a..b].split('"') {
        if part.contains('.')
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
        {
            assert!(
                Tokens::PATCHABLE.contains(&part),
                "themes/tokens.mjs targets {part}, which pm-design cannot patch"
            );
            seen += 1;
        }
    }
    assert!(seen >= 20, "parsed only {seen} targets");
}
