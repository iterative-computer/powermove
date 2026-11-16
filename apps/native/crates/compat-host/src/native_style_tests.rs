use super::*;
use pm_design::{Accessibility, Scheme, Tokens};
#[test]
fn golden_stylesheet() {
    let l = Tokens::new(Scheme::Light, Accessibility::default());
    let d = Tokens::new(Scheme::Dark, Accessibility::default());
    let css = stylesheet(&l, &d);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../design/tests/pm-native.snapshot.css");
    if std::env::var_os("PM_UPDATE_NATIVE_CSS_SNAPSHOT").is_some() {
        std::fs::write(&path, &css).unwrap();
    }
    assert_eq!(css, std::fs::read_to_string(path).unwrap());
}
#[test]
fn every_token_matches_each_resolved_scheme() {
    let l = Tokens::new(Scheme::Light, Accessibility::default());
    let d = Tokens::new(Scheme::Dark, Accessibility::default());
    let css = stylesheet(&l, &d);
    let (light, dark) = css.split_once(":root[data-pm-scheme=dark]").unwrap();
    for (sheet, t) in [
        (light, &l),
        (dark.split_once("[data-pm-style=none]").unwrap().0, &d),
    ] {
        assert!(
            sheet.contains(&format!("--pm-ground:#{:06x};", t.colors.panel)),
            "colors.panel"
        );
        assert!(
            sheet.contains(&format!("--pm-ground-2:#{:06x};", t.colors.panel_secondary)),
            "colors.panel_secondary"
        );
        assert!(
            sheet.contains(&format!("--pm-window:#{:06x};", t.colors.window)),
            "colors.window"
        );
        assert!(
            sheet.contains(&format!("--pm-fill:#{:06x};", t.colors.fill)),
            "colors.fill"
        );
        assert!(
            sheet.contains(&format!("--pm-fill-strong:#{:06x};", t.colors.fill_strong)),
            "colors.fill_strong"
        );
        assert!(
            sheet.contains(&format!("--pm-hover:#{:06x};", t.colors.hover)),
            "colors.hover"
        );
        assert!(
            sheet.contains(&format!("--pm-hairline:#{:06x};", t.colors.hairline)),
            "colors.hairline"
        );
        assert!(
            sheet.contains(&format!("--pm-separator:#{:06x};", t.colors.separator)),
            "colors.separator"
        );
        assert!(
            sheet.contains(&format!("--pm-text:#{:06x};", t.colors.text)),
            "colors.text"
        );
        assert!(
            sheet.contains(&format!("--pm-text-2:#{:06x};", t.colors.secondary)),
            "colors.secondary"
        );
        assert!(
            sheet.contains(&format!("--pm-text-3:#{:06x};", t.colors.muted)),
            "colors.muted"
        );
        assert!(
            sheet.contains(&format!("--pm-accent:#{:06x};", t.colors.accent)),
            "colors.accent"
        );
        assert!(
            sheet.contains(&format!("--pm-accent-text:#{:06x};", t.colors.accent_text)),
            "colors.accent_text"
        );
        assert!(
            sheet.contains(&format!("--pm-accent-wash:#{:06x};", t.colors.accent_wash)),
            "colors.accent_wash"
        );
        assert!(
            sheet.contains(&format!(
                "--pm-selected-fill:#{:06x};",
                t.colors.selected_fill
            )),
            "colors.selected_fill"
        );
        assert!(
            sheet.contains(&format!(
                "--pm-selected-text:#{:06x};",
                t.colors.selected_text
            )),
            "colors.selected_text"
        );
        assert!(
            sheet.contains(&format!("--pm-focus:#{:06x};", t.colors.focus)),
            "colors.focus"
        );
        assert!(
            sheet.contains(&format!("--pm-danger:#{:06x};", t.colors.danger)),
            "colors.danger"
        );
        assert!(
            sheet.contains(&format!("--pm-warning:#{:06x};", t.colors.warning)),
            "colors.warning"
        );
        assert!(
            sheet.contains(&format!("--pm-success:#{:06x};", t.colors.success)),
            "colors.success"
        );
        assert!(
            sheet.contains(&format!("--pm-fs-caption:{}px;", t.typography.minimum)),
            "typography.minimum"
        );
        assert!(
            sheet.contains(&format!("--pm-fs-body:{}px;", t.typography.body)),
            "typography.body"
        );
        assert!(
            sheet.contains(&format!("--pm-fs-title:{}px;", t.typography.title)),
            "typography.title"
        );
        assert!(
            sheet.contains(&format!("--pm-fs-heading:{}px;", t.typography.heading)),
            "typography.heading"
        );
        assert!(
            sheet.contains(&format!("--pm-r-control:{}px;", t.metrics.control_radius)),
            "metrics.control_radius"
        );
        assert!(
            sheet.contains(&format!("--pm-r-card:{}px;", t.radius.md)),
            "radius.md"
        );
        assert!(
            sheet.contains(&format!("--pm-r-capsule:{}px;", t.metrics.capsule_radius)),
            "metrics.capsule_radius"
        );
        assert!(
            sheet.contains(&format!("--pm-row-h:{}px;", t.metrics.row)),
            "metrics.row"
        );
        assert!(
            sheet.contains(&format!("--pm-ctl-h:{}px;", t.metrics.control)),
            "metrics.control"
        );
        assert!(
            sheet.contains(&format!("--pm-header-h:{}px;", t.metrics.header)),
            "metrics.header"
        );
        assert!(
            sheet.contains(&format!("--pm-thumb-min:{}px;", t.metrics.thumbnail_min)),
            "metrics.thumbnail_min"
        );
        assert!(
            sheet.contains(&format!("--pm-gap:{}px;", t.metrics.gap)),
            "metrics.gap"
        );
        assert!(
            sheet.contains(&format!("--pm-inset:{}px;", t.metrics.inset)),
            "metrics.inset"
        );
        assert!(
            sheet.contains(&format!("--pm-space-xs:{}px;", t.space.xs)),
            "space.xs"
        );
        assert!(
            sheet.contains(&format!("--pm-space-sm:{}px;", t.space.sm)),
            "space.sm"
        );
        assert!(
            sheet.contains(&format!("--pm-space-md:{}px;", t.space.md)),
            "space.md"
        );
        assert!(
            sheet.contains(&format!("--pm-space-lg:{}px;", t.space.lg)),
            "space.lg"
        );
        assert!(
            sheet.contains(&format!("--pm-space-xl:{}px;", t.space.xl)),
            "space.xl"
        );
        assert!(
            sheet.contains(&format!(
                "--pm-weight-regular:{};",
                t.typography.regular_weight
            )),
            "typography.regular_weight"
        );
        assert!(
            sheet.contains(&format!(
                "--pm-weight-medium:{};",
                t.typography.medium_weight
            )),
            "typography.medium_weight"
        );
        assert!(
            sheet.contains(&format!(
                "--pm-weight-semibold:{};",
                t.typography.semibold_weight
            )),
            "typography.semibold_weight"
        );
        let (_, width, offset) = t.focus_ring();
        assert!(sheet.contains(&format!(
            "--pm-focus-width:{width}px;--pm-focus-offset:{offset}px;"
        )));
        assert!(sheet.contains(&format!("--pm-dur:{}ms;", t.motion.interaction * 1000.)));
        assert!(sheet.contains(&format!(
            "--pm-media-badge:rgb({} {} {} / {});",
            (t.colors.media_badge >> 16) & 255,
            (t.colors.media_badge >> 8) & 255,
            t.colors.media_badge & 255,
            t.metrics.media_badge_opacity
        )));
        for name in [
            "pm-font",
            "pm-font-mono",
            "pm-media-badge",
            "pm-dur",
            "pm-ease",
            "pm-focus-width",
            "pm-focus-offset",
            "bg-panel",
            "bg-panel-2",
            "bg-field",
            "line",
            "tx",
            "tx-2",
            "accent",
            "r-base",
            "ctl-h",
            "row-h",
            "fs-md",
            "f-ui",
        ] {
            assert!(sheet.contains(&format!("--{name}:")), "{name}");
        }
    }
}
#[test]
fn patched_theme_and_accessibility() {
    let base = Tokens::new(Scheme::Dark, Accessibility::default());
    let mut patched = base.clone();
    patched
        .apply_patch([("colors.panel", 0x123456 as f64), ("metrics.gap", 9.)])
        .unwrap();
    let css = stylesheet(&patched, &patched);
    assert!(css.contains("--pm-ground:#123456;"));
    assert!(css.contains("--pm-gap:9px;"));
    let accessible = Tokens::new(
        Scheme::Dark,
        Accessibility {
            increase_contrast: true,
            reduce_motion: true,
            ..Default::default()
        },
    );
    let css = stylesheet(&accessible, &accessible);
    assert_ne!(css, stylesheet(&base, &base));
    assert!(css.contains("--pm-dur:0ms;"));
    assert!(css.contains(&format!(
        "--pm-separator:#{:06x};",
        accessible.colors.separator
    )));
}

#[test]
fn opt_out_reset_has_only_valid_unique_custom_properties() {
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    let css = stylesheet(&t, &t);
    let reset = css
        .split_once(":where([data-pm-style=none]){")
        .unwrap()
        .1
        .split('}')
        .next()
        .unwrap();
    let mut names = std::collections::BTreeSet::new();
    for declaration in reset.split(';').filter(|s| !s.is_empty()) {
        let (name, value) = declaration.split_once(':').unwrap();
        assert!(name.starts_with("--"));
        assert!(
            name[2..]
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        );
        assert_eq!(value, "initial");
        assert!(names.insert(name));
    }
    assert_eq!(names.len(), 72);
}

#[test]
fn native_backing_matches_panel_over_window_over_every_backdrop() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for solid in [false, true] {
            let t = Tokens::new(
                scheme,
                Accessibility {
                    reduce_transparency: solid,
                    ..Default::default()
                },
            );
            let backing = panel_backing(&t);
            let (window, window_alpha) = t.ground();
            for backdrop in [0., 0.2, 0.5, 1.] {
                for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                    let window_ink = f64::from((window >> shift) & 255) / 255.;
                    let window_pixel = window_ink * f64::from(window_alpha)
                        + backdrop * (1. - f64::from(window_alpha));
                    let panel_ink = f64::from((t.colors.panel >> shift) & 255) / 255.;
                    let native_pixel = panel_ink * f64::from(t.translucency.panel)
                        + window_pixel * (1. - f64::from(t.translucency.panel));
                    let web_pixel = backing[channel] * backing[3] + backdrop * (1. - backing[3]);
                    assert!(
                        (native_pixel - web_pixel).abs() < 1e-9,
                        "{scheme:?}, solid={solid}, backdrop={backdrop}, channel={channel}"
                    );
                }
            }
        }
    }
}

/// The field look (fill, height, padding) is for text fields: a checkbox
/// in an extension panel stays the system checkbox, not a filled box.
#[test]
fn system_toggles_and_sliders_never_take_the_field_look() {
    let l = Tokens::new(Scheme::Light, Accessibility::default());
    let d = Tokens::new(Scheme::Dark, Accessibility::default());
    let css = stylesheet(&l, &d);
    let field = css.find("min-height: var(--pm-ctl-h);").unwrap();
    let rule = &css[css[..field].rfind('}').unwrap()..field];
    let input = rule
        .find("input:not(")
        .expect("the field rule names its inputs");
    for kind in ["checkbox", "radio", "range", "color", "file"] {
        assert!(
            rule[input..].contains(&format!("[type=\"{kind}\"]")),
            "{kind}"
        );
    }
    assert!(
        !rule.contains("  input:where("),
        "every input took the field look"
    );
}

/// Every v1 token the extensions on a migrated profile style with resolves,
/// in both schemes, and is reset by a `none` opt-out like the rest.
#[test]
fn v1_extension_tokens_resolve() {
    let l = Tokens::new(Scheme::Light, Accessibility::default());
    let d = Tokens::new(Scheme::Dark, Accessibility::default());
    let css = stylesheet(&l, &d);
    let dark = css.find(":root[data-pm-scheme=dark]{").unwrap();
    let none = css.find(":where([data-pm-style=none]){").unwrap();
    for name in [
        "--bg-panel",
        "--bg-panel-2",
        "--bg-sunken",
        "--bg-window",
        "--bg-field",
        "--bg-hover",
        "--line",
        "--tx",
        "--tx-2",
        "--tx-3",
        "--accent",
        "--on-accent",
        "--danger",
        "--warning",
        "--f-ui",
        "--f-mono",
        "--fs-xs",
        "--fs-sm",
        "--fs-md",
        "--r-base",
        "--row-h",
        "--ctl-h",
        "--dur-2",
        "--ease",
    ] {
        let set = format!("{{{name}:");
        let set_after = format!(";{name}:");
        let light = &css[..dark];
        let dark_part = &css[dark..none];
        for part in [light, dark_part] {
            assert!(part.contains(&set) || part.contains(&set_after), "{name}");
        }
        assert!(css[none..].contains(&format!("{name}:initial;")), "{name}");
    }
}
