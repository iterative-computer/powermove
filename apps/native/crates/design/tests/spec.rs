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
    (
        "colors.selected_secondary",
        "Metadata on a selected row (muted fails there)",
    ),
    ("colors.selected_danger", "Danger words on a selected row"),
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
    (
        "colors.viewer_guide",
        "Ruler guides over the viewer's picture",
    ),
    ("colors.viewer_grid", "The viewer's grid over the picture"),
    ("colors.warning", "Warnings, missing-but-recoverable states"),
    ("colors.success", "Completed, online, rendered"),
    (
        "colors.scope_red",
        "Red channel scope trace on the dark viewer",
    ),
    (
        "colors.scope_green",
        "Green channel scope trace on the dark viewer",
    ),
    (
        "colors.scope_blue",
        "Blue channel scope trace on the dark viewer",
    ),
    (
        "colors.channel_red",
        "Red channel ink on panels (curves, channel values)",
    ),
    (
        "colors.channel_green",
        "Green channel ink on panels (curves, channel values)",
    ),
    (
        "colors.channel_blue",
        "Blue channel ink on panels (curves, channel values)",
    ),
    (
        "colors.media_badge",
        "Badge fill over media (duration, offline); always dark",
    ),
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
    (
        "metrics.titlebar",
        "Title bar band of a plain panel window; content starts below it",
    ),
    ("metrics.toolbar_title_width", "Document title slot width"),
    ("metrics.toolbar_title_line", "Document title line height"),
    (
        "metrics.control",
        "Standalone control height (buttons, popups)",
    ),
    ("metrics.field", "Value well height inside a row"),
    (
        "metrics.value_column",
        "Least width of an inspector row's value column",
    ),
    (
        "metrics.label_column",
        "Most an inspector row's label takes before its value",
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
    (
        "metrics.message_inset",
        "Leading room beside a person's message in the Agent conversation",
    ),
    ("metrics.channel_strip", "Mixer channel strip width"),
    ("metrics.fader_knob", "Mixer fader knob height"),
    ("metrics.welcome_width", "Welcome window width"),
    ("metrics.welcome_height", "Welcome window height"),
    (
        "metrics.mark_bracket",
        "Width of In and Out brackets on rulers and scrubbers",
    ),
    ("metrics.sync_badge", "Linked-clip sync badge height"),
    (
        "metrics.viewer_3d_labels",
        "Minimum viewer width for labels in the 3D toolbar",
    ),
    (
        "metrics.viewer_3d_collapse",
        "Viewer width below which the 3D toolbar becomes one pull-down",
    ),
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
    ("metrics.viewer_ruler", "The viewer's ruler bands"),
    ("metrics.grade_wheel", "A colour wheel's diameter"),
    ("metrics.curve_editor", "The curve editor's graph height"),
    ("metrics.layer_column", "Timeline layer list column width"),
    ("metrics.clip_radius", "Timeline clip bar corner radius"),
    ("metrics.playhead", "Playhead line width"),
    ("metrics.symbol_toolbar", "SF Symbol size in toolbar items"),
    ("metrics.symbol_panel", "SF Symbol size in panel headers"),
    ("metrics.symbol_inline", "SF Symbol size inside rows"),
    (
        "metrics.symbol_action",
        "SF Symbol size of a surface's one primary action glyph",
    ),
    ("metrics.hit", "Minimum pointer target"),
    ("metrics.popover_width", "Default popover width"),
    (
        "metrics.popover_radius",
        "Corner of popovers Powermove frames itself (agent popover)",
    ),
    ("metrics.sheet_width", "Default sheet width"),
    ("metrics.agent_width", "Default agent panel width"),
    (
        "metrics.agent_height",
        "Agent popover height when the window has room",
    ),
    (
        "metrics.agent_gap",
        "Space between turns in the Agent conversation",
    ),
    ("metrics.tree_indent", "Indent per outline level"),
    (
        "metrics.kind_badge",
        "Layer-kind badge square in the timeline and Project list",
    ),
    (
        "metrics.thumbnail_min",
        "Minimum Project grid thumbnail width",
    ),
    (
        "metrics.sheet_inset",
        "Content inset inside sheets and Settings panes",
    ),
    (
        "metrics.sheet_value_column",
        "Shared value column in sheets",
    ),
    (
        "metrics.sheet_width_wide",
        "Width of wide sheets: Export, Expression",
    ),
    (
        "metrics.composer_max_lines",
        "Agent composer grows to this many lines, then scrolls",
    ),
    ("metrics.group_radius", "Settings grouped-form card corner"),
    ("metrics.form_row", "Minimum Settings row height"),
    (
        "metrics.section_gap",
        "Space above each Settings section after the first",
    ),
    (
        "metrics.gizmo_handle",
        "Transform handle and path vertex square",
    ),
    (
        "metrics.gizmo_keyline",
        "Gizmo stroke and keyline width over the image",
    ),
    ("metrics.rotation_stem", "Rotation handle stem length"),
    ("metrics.anchor_mark", "Anchor point crosshair circle"),
    ("metrics.tangent_end", "Bézier tangent end circle"),
    ("metrics.checker", "Transparency checkerboard square"),
    (
        "metrics.pasteboard_opacity",
        "Opacity of selected layers drawn outside the frame",
    ),
    (
        "metrics.status_dot",
        "Attention dot and drop insertion-line end circle",
    ),
    (
        "metrics.text_padding",
        "Vertical text padding in bubbles, the composer and code blocks",
    ),
    ("metrics.drop_stroke", "Drop-target inner stroke"),
    (
        "metrics.drag_opacity",
        "Opacity of a drag source left in place",
    ),
    (
        "metrics.media_badge_opacity",
        "Opacity of the media badge fill",
    ),
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
        "Emphasis weight of panel titles and section headings (medium)",
    ),
    (
        "typography.display_weight",
        "Weight of Heading and Display, the only semibold text",
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
    (
        "motion.wheel_rest",
        "Pointer rest before the scroll wheel edits a hovered value (s)",
    ),
    (
        "motion.overlay_idle",
        "Still time before the Focus overlay capsule fades (s)",
    ),
    (
        "motion.tooltip_delay",
        "Delay before a tooltip appears for hover or keyboard focus (s)",
    ),
    (
        "motion.tooltip_warm",
        "Time window for an immediate tooltip swap to a neighbouring control (s)",
    ),
    (
        "motion.note_hold",
        "Time a status note holds before the status line returns (s)",
    ),
    (
        "translucency.window",
        "Window tint over the behind-window blur (1 = solid, Reduce Transparency)",
    ),
    (
        "translucency.panel",
        "Shared panel material over the window tint (1 = solid)",
    ),
    (
        "translucency.ground",
        "Window tint over the behind-window blur, darker than `window` so panels step above the composite",
    ),
    ("translucency.edge", "Panel hairline edge in `text` ink"),
    (
        "translucency.line",
        "Header, column and status dividers in `text` ink",
    ),
    ("translucency.tick_minor", "Minor ruler tick, as `text` ink"),
    (
        "translucency.tick_major",
        "Labelled ruler tick, as `text` ink",
    ),
    (
        "translucency.grid",
        "Time grid under the timeline tracks, as `text` ink",
    ),
    (
        "translucency.offline_hatch",
        "Media Offline hatch on timeline clips, as `danger` ink",
    ),
    (
        "translucency.hover",
        "Hover wash and soft fills, as `text` ink",
    ),
    (
        "translucency.selected",
        "Neutral on state of a tab or segment, as `text` ink",
    ),
    ("translucency.pressed", "Pressed wash, as `text` ink"),
    ("translucency.faint", "Third text step, as `text` ink"),
    (
        "translucency.glass_tint",
        "Window-colour tint on system glass that carries labels",
    ),
    (
        "translucency.glass_menu",
        "Floating colour under a palette's or menu's rows on system glass",
    ),
    (
        "colors.faint",
        "Placeholders, shortcut hints, disabled labels (derived)",
    ),
    (
        "colors.raised",
        "Raised neutral surface: selected segment or tab, neutral button, card (derived)",
    ),
    (
        "metrics.dock_gap",
        "Space between docked panels and to the window edge",
    ),
    ("metrics.sidebar", "Sidebar width of Home and Settings"),
    ("typography.heading_tracking", "Tracking of headings (pt)"),
    (
        "typography.display_tracking",
        "Tracking of screen titles (pt)",
    ),
    (
        "space.hair",
        "Spacing scale: pill inset, icon-to-edge nudge",
    ),
    ("space.xs", "Spacing scale: icon to label, stacked text"),
    ("space.sm", "Spacing scale: between related controls"),
    ("space.md", "Spacing scale: between panels, control padding"),
    ("space.lg", "Spacing scale: panel and card inset"),
    ("space.xl", "Spacing scale: between groups"),
    ("space.xxl", "Spacing scale: screen margins"),
    ("space.section", "Spacing scale: between sections"),
    ("radius.base", "Radius scale base"),
    ("radius.xs", "Checkbox, kind badge (base)"),
    (
        "radius.sm",
        "Wells, segments, menu rows: the smallest control corner",
    ),
    ("radius.md", "Buttons, fields, tabs, pill tracks"),
    ("radius.lg", "Panels, cards, menus"),
    ("radius.xl", "Popovers, sheets, the palette"),
    ("radius.window", "The window's own corner (system-owned)"),
    (
        "radius.smoothing",
        "Continuous-corner smoothing of `lg` and larger surfaces",
    ),
    (
        "colors.fill",
        "Soft neutral fill: segmented tracks, search fields (derived)",
    ),
    (
        "colors.fill_strong",
        "Selected tab or segment over a raised track, never the accent (derived)",
    ),
    ("metrics.tab", "Text tab and full-height segmented control"),
    (
        "metrics.sheet_body_max",
        "Tallest sheet option body before it scrolls",
    ),
    (
        "metrics.segment_radius",
        "Segment and tab corner inside a pill (concentric)",
    ),
    ("colors.hairline", "Opaque control and card edge (derived)"),
    (
        "metrics.pill_radius",
        "Segmented track and tab-strip corner",
    ),
    (
        "metrics.pill_inset",
        "Track-to-segment padding and segment gap",
    ),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn flatten(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(map) => {
            // Sorted, so the table is identical whether or not another crate in the
            // build turns on serde_json's `preserve_order` feature.
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in entries {
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
    for group in [
        "colors",
        "metrics",
        "space",
        "radius",
        "typography",
        "motion",
        "translucency",
    ] {
        flatten(group, &v[group], &mut out);
    }
    out
}

fn show(key: &str, v: &Value) -> String {
    if key.starts_with("colors.") || key == "translucency.ground" {
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
        for (what, ink) in [
            ("metadata", c.selected_secondary),
            ("danger words", c.selected_danger),
        ] {
            assert!(
                contrast(ink, c.selected_fill) >= 4.5,
                "{name}: {what} on a selected row"
            );
        }
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
fn neutrals_are_one_ink_over_a_ground() {
    for t in variants() {
        let c = &t.colors;
        // No neutral carries more chroma than its appearance's ink and grounds.
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
            c.faint,
            c.raised,
            c.selected_fill,
            c.fill,
            c.fill_strong,
            c.hairline,
            c.focus,
        ] {
            let (_, chroma, _) = pm_design::color::to_oklch(v);
            assert!(chroma < 0.016, "#{v:06x} is tinted ({chroma:.3})");
        }
        // The text steps descend toward the panel: text, secondary, muted, faint.
        let steps = [c.text, c.secondary, c.muted, c.faint].map(|v| contrast(v, c.panel));
        assert!(steps.windows(2).all(|w| w[0] > w[1]), "{steps:?}");
        assert!(steps[3] >= 2., "faint still reads as text");
    }
    // Dark is warm stone, light is cool paper: the identity v1 established.
    let hue = |scheme| pm_design::color::to_oklch(Colors::of(scheme).panel).2;
    assert!((30. ..90.).contains(&hue(Scheme::Dark)));
    assert!((250. ..300.).contains(&hue(Scheme::Light)));
    assert_eq!(pm_design::color::oklch(1., 0., 0.), 0xffffff);
    assert_eq!(pm_design::color::oklch(0., 0., 0.), 0x000000);
}

#[test]
fn concentric_radii_and_grid() {
    for t in variants() {
        let m = t.metrics;
        // Concentric radii apply only inside an outer corner's curve. Panel
        // content sits `inset` in, clear of the panel corner, so controls keep
        // their own radius and are never rounder than the panel.
        assert!(m.inset >= m.panel_radius, "controls clear the panel corner");
        assert!(m.control_radius <= m.panel_radius, "control nests in panel");
        assert!(m.inner_radius <= m.control_radius - 2.);
        let r = t.radius;
        assert_eq!(
            (r.xs, r.sm, r.md, r.lg, r.xl),
            (r.base, r.base * 1.5, r.base * 2., r.base * 3., r.base * 4.),
            "one scale from one base"
        );
        assert_eq!(r.nested(r.lg, t.space.xs), r.md, "child = parent − inset");
        assert_eq!(
            r.nested(r.md, t.space.md),
            r.sm,
            "never below the smallest corner"
        );
        assert!(m.dock_gap <= m.inset);
        assert_eq!(
            m.segment_radius,
            m.pill_radius - m.pill_inset,
            "segments are concentric"
        );
        assert_eq!(m.tab, m.control, "a tab strip lines up with push buttons");
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
    assert!(
        contrast(t.colors.separator, t.colors.panel) >= 3.,
        "a patched separator is raised back to 3:1"
    );
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

#[test]
fn every_patchable_field_reads_back() {
    let mut t = Tokens::new(Scheme::Light, Accessibility::default());
    for name in Tokens::PATCHABLE {
        assert!(t.field(name).is_some(), "{name} has no getter");
    }
    assert_eq!(t.field("metrics.gap"), Some(t.metrics.gap as f64));
    assert_eq!(t.field("colors.nope"), None);
    t.apply_patch([("colors.accent", 0x123456 as f64)]).unwrap();
    assert_eq!(t.field("colors.accent"), Some(0x123456 as f64));
}

#[test]
fn surface_tokens_match_opus_spec_in_every_appearance() {
    let doc = std::fs::read_to_string(root().join("docs/native/design/surfaces/README.md"))
        .expect("surface spec");
    let section = doc
        .split("## 7. New tokens requested by these specs")
        .nth(1)
        .expect("new surface tokens section");
    let requested: Vec<_> = section
        .lines()
        .filter(|line| line.starts_with("| `"))
        .map(|line| {
            let mut cells = line.split('|').skip(1).map(str::trim);
            let name = cells.next().unwrap().trim_matches('`');
            let value = cells.next().unwrap();
            (name, value)
        })
        .collect();
    assert_eq!(requested.len(), 13, "review new surface token requests");
    for scheme in [Scheme::Light, Scheme::Dark] {
        for increase_contrast in [false, true] {
            for reduce_transparency in [false, true] {
                for reduce_motion in [false, true] {
                    let mut t = Tokens::new(
                        scheme,
                        Accessibility {
                            increase_contrast,
                            reduce_transparency,
                            reduce_motion,
                        },
                    );
                    let values = leaves(&t);
                    for (name, expected) in &requested {
                        let (_, value) = values
                            .iter()
                            .find(|(key, _)| key == name)
                            .unwrap_or_else(|| panic!("missing surface token {name}"));
                        assert_eq!(show(name, value), *expected, "{name}: {scheme:?}");
                        let before = t.clone();
                        assert!(
                            t.apply_patch([(*name, 0.)]).is_err(),
                            "surface tokens are not theme-patchable: {name}"
                        );
                        assert_eq!(t, before, "a rejected patch changes nothing");
                    }
                }
            }
        }
    }
}

#[test]
fn scope_traces_meet_contrast_on_the_specified_dark_viewer() {
    // colour.md and language.md: scope plots use dark-scheme tokens even in
    // light appearance. The data colours are not chrome or text on light panels.
    for increase_contrast in [false, true] {
        let c = Tokens::new(
            Scheme::Dark,
            Accessibility {
                increase_contrast,
                ..Default::default()
            },
        )
        .colors;
        for trace in [c.scope_red, c.scope_green, c.scope_blue] {
            assert!(contrast(trace, c.viewer) >= 3., "scope trace #{trace:06X}");
        }
    }
}

/// A channel's ink reads as text on a panel in both appearances (the Color
/// panel's channel values are caption-sized numbers).
#[test]
fn channel_inks_read_on_panels_in_both_appearances() {
    for scheme in [Scheme::Dark, Scheme::Light] {
        let c = Tokens::new(scheme, Default::default()).colors;
        for ink in [c.channel_red, c.channel_green, c.channel_blue] {
            assert!(
                contrast(ink, c.panel) >= 4.5,
                "{scheme:?} channel ink #{ink:06X} on #{:06X}: {}",
                c.panel,
                contrast(ink, c.panel)
            );
        }
    }
}
