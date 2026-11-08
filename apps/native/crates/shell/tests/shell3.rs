#![cfg(target_os = "macos")]
//! Shell 3 (design review must-fixes): drop zones, tab-menu state, empty
//! states, Workspace vocabulary, theme patches, document subtitle and SF Symbols.
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{layout::*, menu, symbol, theme};
use std::sync::Arc;

fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, Accessibility::default())
}
const BOUNDS: Rect = Rect {
    x: 6.,
    y: 58.,
    w: 1428.,
    h: 810.,
};

#[test]
fn drop_previews_cover_only_the_drop_zone() {
    let l = Layout::preset("design");
    let t = tokens();
    let g = l.geometry(BOUNDS, &t);
    // Edges: the outer quarter of the docking area, never a whole dock.
    let left = l
        .drop_target(&g, BOUNDS, BOUNDS.x + 2., BOUNDS.y + 300., &t)
        .unwrap();
    assert_eq!(left.target, DropTarget::Edge(Edge::Left));
    assert_eq!(
        left.rect,
        Rect {
            w: BOUNDS.w * EDGE_ZONE,
            ..BOUNDS
        }
    );
    let right = l
        .drop_target(&g, BOUNDS, BOUNDS.x + BOUNDS.w - 2., BOUNDS.y + 300., &t)
        .unwrap();
    assert_eq!(right.target, DropTarget::Edge(Edge::Right));
    assert_eq!(right.rect.x + right.rect.w, BOUNDS.x + BOUNDS.w);
    assert_eq!(right.rect.w, BOUNDS.w * EDGE_ZONE);
    let bottom = l
        .drop_target(&g, BOUNDS, 600., BOUNDS.y + BOUNDS.h - 2., &t)
        .unwrap();
    assert_eq!(bottom.target, DropTarget::Edge(Edge::Bottom));
    assert_eq!(bottom.rect.h, BOUNDS.h * EDGE_ZONE);
    assert_eq!(bottom.rect.y + bottom.rect.h, BOUNDS.y + BOUNDS.h);
    // Every preview stays inside the docking area and is smaller than it.
    for p in [left, right, bottom] {
        assert!(p.rect.w * p.rect.h < BOUNDS.w * BOUNDS.h * 0.3);
    }
    // Viewer offers only above/below; no tab target.
    let viewer = g
        .panels
        .iter()
        .find(|p| l.docks[p.dock].groups[p.group].active() == "viewer")
        .unwrap()
        .rect;
    let tab = l
        .drop_target(&g, BOUNDS, viewer.x + 40., viewer.y + 4., &t)
        .unwrap();
    assert!(matches!(tab.target, DropTarget::Split { .. }));
    assert!(tab.rect.h < viewer.h / 2.);
    let half = l
        .drop_target(&g, BOUNDS, viewer.x + 40., viewer.y + viewer.h * 0.7, &t)
        .unwrap();
    assert!(half.rect.h < viewer.h / 2. + 0.5);
}

#[test]
fn tab_menu_checks_the_current_dock_and_says_workspace() {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(gpui_parley::ParleyTextSystem::new(
            gpui_parley::SystemFonts::Load,
        )),
        Arc::new(()),
        || None,
    );
    let t = tokens();
    let window = cx
        .open_window(size(px(1440.), px(900.)), |_, cx| {
            cx.new(|cx| {
                let mut v = pm_shell::view::Editor::new(t, None, cx);
                v.layout = Layout::preset("design");
                v
            })
        })
        .unwrap();
    window
        .update(&mut cx, |v, _, cx| {
            let entries = v.tab_menu_entries("inspector");
            let checked: Vec<_> = entries.iter().filter(|e| e.checked).collect();
            assert_eq!(checked.len(), 1);
            assert_eq!(checked[0].label, "Dock Right");
            assert!(entries.iter().any(|e| e.label == "Save Workspace…"));
            assert!(entries.iter().any(|e| e.label == "Reset Workspace"));
            assert!(entries.iter().all(|e| !e.label.contains("Layout")));
            // Moving the panel moves the checkmark.
            assert!(v.run_shell_command("panel:dock:inspector:left", cx));
            let entries = v.tab_menu_entries("inspector");
            let checked: Vec<_> = entries.iter().filter(|e| e.checked).collect();
            assert_eq!(checked[0].label, "Dock Left");
            // The viewer cannot be closed.
            let viewer = v.tab_menu_entries("viewer");
            assert!(viewer.iter().all(|e| !e.label.starts_with("Close")));
            // It sits in the centre column, which is not a dock choice.
            assert_eq!(viewer.iter().filter(|e| e.checked).count(), 0);
        })
        .unwrap();
}

#[test]
fn empty_states_are_user_language_never_panel_ids() {
    for id in PANELS {
        let (heading, line, action) = empty_state(id);
        for text in [heading, line] {
            for word in [
                "TEMPORARY",
                "PHASE",
                "fxbrowser",
                "layer-effects",
                "gradient-controls",
                "assets",
                "perf",
                "isolated",
                "placeholder",
            ] {
                assert!(!text.contains(word), "{id}: {text}");
            }
            assert_ne!(text, text.to_uppercase(), "no all-caps");
        }
        // Heading in Title Case, one sentence ending with a full stop.
        assert!(
            heading
                .split(' ')
                .all(|w| w.chars().next().unwrap().is_uppercase()),
            "{heading}"
        );
        assert!(line.ends_with('.'), "{line}");
        if let Some((label, command)) = action {
            assert!(label.ends_with('…'), "{label} asks for more input");
            assert!(!command.is_empty());
        }
    }
    assert_eq!(empty_state("assets").0, "No Media");
    assert_eq!(empty_state("inspector").0, "No Selection");
}

#[test]
fn one_workspace_vocabulary_in_menus_and_errors() {
    let labels: Vec<String> = menu::menus()
        .into_iter()
        .flat_map(|(_, items)| items)
        .map(|i| i.label)
        .collect();
    assert!(labels.iter().any(|l| l == "Workspace"));
    assert!(labels.iter().all(|l| !l.contains("Layout")), "{labels:?}");
    assert!(!menu::SUBMENUS.contains(&"layouts"));
    for bad in ["", "   ", &"x".repeat(LAYOUT_NAME_MAX + 1), "a\u{7}b"] {
        let e = layout_name(bad).unwrap_err().to_string();
        assert!(e.contains("orkspace"), "{e}");
    }
}

#[test]
fn theme_patch_applies_on_every_token_build_and_keeps_contrast_floors() {
    let patch =
        theme::parse(r#"{"nativePatch":{"colors.accent":1193046,"metrics.gap":4}}"#).unwrap();
    assert_eq!(patch.len(), 2);
    assert_eq!(
        theme::parse(r#"{"colors.accent":1193046}"#).unwrap().len(),
        1
    );
    assert!(theme::parse(r#"{"nativePatch":{"colors.accent":"orange"}}"#).is_err());
    assert!(theme::parse("[1]").is_err());
    theme::set(patch).unwrap();
    for scheme in [Scheme::Light, Scheme::Dark] {
        let t = theme::tokens(scheme, Accessibility::default());
        assert_eq!(t.colors.accent, 0x123456);
        assert_eq!(t.metrics.gap, 4.);
    }
    // Increase Contrast is re-applied after the theme.
    let hc = Accessibility {
        increase_contrast: true,
        ..Default::default()
    };
    assert_eq!(
        theme::tokens(Scheme::Dark, hc).metrics.border,
        Tokens::new(Scheme::Dark, hc).metrics.border
    );
    // A rejected patch leaves the current theme in place.
    assert!(theme::set(vec![("colors.nope".into(), 1.)]).is_err());
    assert_eq!(
        theme::tokens(Scheme::Dark, Accessibility::default())
            .colors
            .accent,
        0x123456
    );
    // environment.nativeTokens lists every patchable field with its value.
    let t = theme::tokens(Scheme::Dark, Accessibility::default());
    let env = theme::native_tokens(&t);
    assert_eq!(env.len(), Tokens::PATCHABLE.len());
    assert_eq!(env["metrics.gap"], serde_json::json!(4.0));
    theme::clear();
    assert_eq!(
        theme::tokens(Scheme::Dark, Accessibility::default()),
        tokens()
    );
}

#[test]
fn document_subtitle_shows_saving_then_edited_then_nothing() {
    assert_eq!(pm_shell::document_subtitle(true, true), "Saving…");
    assert_eq!(pm_shell::document_subtitle(false, true), "Saving…");
    assert_eq!(pm_shell::document_subtitle(true, false), "Edited");
    assert_eq!(pm_shell::document_subtitle(false, false), "");
}

#[test]
fn sf_symbols_rasterize_tinted_for_every_kind_and_header_glyph() {
    objc2::rc::autoreleasepool(|_| {
        let red = 0xff2000;
        let image = symbol::rasterize("plus", 13., symbol::Weight::Medium, red, 2.).unwrap();
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (26, 26));
        let bytes = image.as_bytes(0).unwrap();
        // BGRA, straight alpha: opaque pixels carry the token colour.
        let opaque: Vec<_> = bytes.chunks_exact(4).filter(|p| p[3] == 255).collect();
        assert!(!opaque.is_empty(), "plus drew nothing");
        assert!(
            opaque
                .iter()
                .all(|p| p[2] > 0xf0 && p[1] < 0x30 && p[0] < 0x10),
            "{:?}",
            opaque[0]
        );
        // Cached: the same request returns the same image.
        let again = symbol::rasterize("plus", 13., symbol::Weight::Medium, red, 2.).unwrap();
        assert!(Arc::ptr_eq(&image, &again));
        for kind in [
            "text",
            "shape",
            "video",
            "image",
            "sequence",
            "audio",
            "adjustment",
            "null",
            "camera",
            "group",
            "precomp",
            "solid",
            "light",
            "unknown",
        ] {
            let name = symbol::kind_symbol(kind);
            assert!(
                symbol::rasterize(name, 11., symbol::Weight::Regular, 0xffffff, 2.).is_some(),
                "{kind} → {name} is not an SF Symbol"
            );
        }
        for name in [
            "xmark",
            "chevron.up.chevron.down",
            "play.fill",
            "pause.fill",
        ] {
            assert!(
                symbol::rasterize(name, 11., symbol::Weight::Medium, 0, 2.).is_some(),
                "{name}"
            );
        }
        assert!(symbol::rasterize("no.such.symbol", 11., symbol::Weight::Regular, 0, 2.).is_none());
    });
}

#[test]
fn panel_headers_share_the_32_pt_header_metric() {
    assert_eq!(tokens().metrics.header, 32.);
    assert_eq!(
        Tokens::new(Scheme::Light, Accessibility::default())
            .metrics
            .header,
        32.
    );
}

#[test]
fn a_parked_panel_comes_back_where_it_was() {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(gpui_parley::ParleyTextSystem::new(
            gpui_parley::SystemFonts::Load,
        )),
        Arc::new(()),
        || None,
    );
    let t = tokens();
    let window = cx
        .open_window(size(px(1440.), px(900.)), |_, cx| {
            cx.new(|cx| {
                let mut v = pm_shell::view::Editor::new(t, None, cx);
                v.layout = Layout::preset("design");
                v
            })
        })
        .unwrap();
    window
        .update(&mut cx, |v, _, cx| {
            // An extension reloading takes its panel away for a moment.
            assert!(v.run_shell_command("panel:dock:inspector:left", cx));
            let place = v.layout.find("inspector").unwrap();
            assert!(v.run_shell_command("panel:park:inspector", cx));
            assert!(v.layout.find("inspector").is_none());
            assert!(v.run_shell_command("panel:show:inspector", cx));
            assert_eq!(v.layout.find("inspector"), Some(place));
            // A tab among others returns to its index in that stack.
            let shared = v.layout.find("layer-effects").unwrap();
            assert!(v.layout.docks[shared.0].groups[shared.1].tabs.len() > 1);
            assert!(v.run_shell_command("panel:park:layer-effects", cx));
            assert!(v.run_shell_command("panel:show:layer-effects", cx));
            assert_eq!(v.layout.find("layer-effects"), Some(shared));
            // A plain close forgets the place: show uses the panel's home.
            assert!(v.run_shell_command("panel:park:inspector", cx));
            assert!(v.run_shell_command("panel:close:inspector", cx));
            assert!(v.run_shell_command("panel:show:inspector", cx));
            assert_ne!(v.layout.find("inspector").map(|p| p.0), Some(place.0));
        })
        .unwrap();
}
