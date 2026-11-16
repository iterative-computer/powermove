//! Web panel tokens, generated from the same resolved tokens as native controls.
use pm_design::Tokens;
use std::fmt::Write;

/// The same source-over stack the shell paints: panel tint over window tint,
/// then the shared behind-window material. WebKit lives below GPUI's window
/// tint, so omitting that first tint would expose a different panel ground.
pub fn panel_backing(t: &Tokens) -> [f64; 4] {
    let (window, window_alpha) = t.ground();
    let window_alpha = f64::from(window_alpha);
    let panel_alpha = f64::from(t.translucency.panel);
    let alpha = panel_alpha + window_alpha * (1. - panel_alpha);
    if alpha == 0. {
        return [0.; 4];
    }
    let channel = |shift: u32| {
        let panel = f64::from((t.colors.panel >> shift) & 255) / 255.;
        let window = f64::from((window >> shift) & 255) / 255.;
        (panel * panel_alpha + window * window_alpha * (1. - panel_alpha)) / alpha
    };
    [channel(16), channel(8), channel(0), alpha]
}

pub fn stylesheet(light: &Tokens, dark: &Tokens) -> String {
    let mut css = String::new();
    for (selector, t) in [(":root", light), (":root[data-pm-scheme=dark]", dark)] {
        write!(css, "{selector}{{").unwrap();
        write!(css, "--pm-ground:#{:06x};", t.colors.panel).unwrap();
        write!(css, "--pm-ground-2:#{:06x};", t.colors.panel_secondary).unwrap();
        write!(css, "--pm-window:#{:06x};", t.colors.window).unwrap();
        write!(css, "--pm-fill:#{:06x};", t.colors.fill).unwrap();
        write!(css, "--pm-fill-strong:#{:06x};", t.colors.fill_strong).unwrap();
        write!(css, "--pm-hover:#{:06x};", t.colors.hover).unwrap();
        write!(css, "--pm-hairline:#{:06x};", t.colors.hairline).unwrap();
        write!(css, "--pm-separator:#{:06x};", t.colors.separator).unwrap();
        write!(css, "--pm-text:#{:06x};", t.colors.text).unwrap();
        write!(css, "--pm-text-2:#{:06x};", t.colors.secondary).unwrap();
        write!(css, "--pm-text-3:#{:06x};", t.colors.muted).unwrap();
        write!(css, "--pm-accent:#{:06x};", t.colors.accent).unwrap();
        write!(css, "--pm-accent-text:#{:06x};", t.colors.accent_text).unwrap();
        write!(css, "--pm-accent-wash:#{:06x};", t.colors.accent_wash).unwrap();
        write!(css, "--pm-selected-fill:#{:06x};", t.colors.selected_fill).unwrap();
        write!(css, "--pm-selected-text:#{:06x};", t.colors.selected_text).unwrap();
        write!(css, "--pm-focus:#{:06x};", t.colors.focus).unwrap();
        write!(css, "--pm-danger:#{:06x};", t.colors.danger).unwrap();
        write!(css, "--pm-warning:#{:06x};", t.colors.warning).unwrap();
        write!(css, "--pm-success:#{:06x};", t.colors.success).unwrap();
        write!(css, "--pm-fs-caption:{}px;", t.typography.minimum).unwrap();
        write!(css, "--pm-fs-body:{}px;", t.typography.body).unwrap();
        write!(css, "--pm-fs-title:{}px;", t.typography.title).unwrap();
        write!(css, "--pm-fs-heading:{}px;", t.typography.heading).unwrap();
        write!(css, "--pm-r-control:{}px;", t.metrics.control_radius).unwrap();
        write!(css, "--pm-r-card:{}px;", t.radius.md).unwrap();
        write!(css, "--pm-r-capsule:{}px;", t.metrics.capsule_radius).unwrap();
        write!(css, "--pm-row-h:{}px;", t.metrics.row).unwrap();
        write!(css, "--pm-ctl-h:{}px;", t.metrics.control).unwrap();
        write!(css, "--pm-header-h:{}px;", t.metrics.header).unwrap();
        write!(css, "--pm-thumb-min:{}px;", t.metrics.thumbnail_min).unwrap();
        write!(css, "--pm-gap:{}px;", t.metrics.gap).unwrap();
        write!(css, "--pm-inset:{}px;", t.metrics.inset).unwrap();
        write!(css, "--pm-space-xs:{}px;", t.space.xs).unwrap();
        write!(css, "--pm-space-sm:{}px;", t.space.sm).unwrap();
        write!(css, "--pm-space-md:{}px;", t.space.md).unwrap();
        write!(css, "--pm-space-lg:{}px;", t.space.lg).unwrap();
        write!(css, "--pm-space-xl:{}px;", t.space.xl).unwrap();
        write!(css, "--pm-weight-regular:{};", t.typography.regular_weight).unwrap();
        write!(css, "--pm-weight-medium:{};", t.typography.medium_weight).unwrap();
        write!(
            css,
            "--pm-weight-semibold:{};",
            t.typography.semibold_weight
        )
        .unwrap();
        let (_, width, offset) = t.focus_ring();
        write!(
            css,
            "--pm-focus-width:{width}px;--pm-focus-offset:{offset}px;--pm-dur:{}ms;",
            if t.accessibility.reduce_motion {
                0.
            } else {
                t.motion.interaction * 1000.
            }
        )
        .unwrap();
        write!(
            css,
            "--pm-media-badge:rgb({} {} {} / {});",
            (t.colors.media_badge >> 16) & 255,
            (t.colors.media_badge >> 8) & 255,
            t.colors.media_badge & 255,
            t.metrics.media_badge_opacity
        )
        .unwrap();
        css.push_str("--pm-font:-apple-system,system-ui;--pm-font-mono:ui-monospace,'SF Mono';--pm-ease:cubic-bezier(.22,1,.36,1);");
        css.push_str("--bg-panel:var(--pm-ground);");
        css.push_str("--bg-panel-2:var(--pm-ground-2);");
        css.push_str("--bg-sunken:var(--pm-ground-2);");
        css.push_str("--bg-window:var(--pm-window);");
        css.push_str("--bg-field:var(--pm-fill);");
        css.push_str("--line:var(--pm-hairline);");
        css.push_str("--tx:var(--pm-text);");
        css.push_str("--tx-2:var(--pm-text-2);");
        css.push_str("--accent:var(--pm-accent);");
        css.push_str("--f-ui:var(--pm-font);");
        css.push_str("--f-mono:var(--pm-font-mono);");
        css.push_str("--fs-md:var(--pm-fs-body);");
        css.push_str("--r-base:var(--pm-r-control);");
        css.push_str("--row-h:var(--pm-row-h);");
        css.push_str("--ctl-h:var(--pm-ctl-h);");
        css.push_str("--dur-2:var(--pm-dur);");
        css.push_str("--ease:var(--pm-ease);");
        css.push('}');
    }
    // `none` on a contribution also stops inherited host variables.
    css.push_str(":where([data-pm-style=none]){");
    let token_names: std::collections::BTreeSet<String> = css
        .split("--")
        .skip(1)
        .filter_map(|part| part.split_once(':').map(|(name, _)| name.to_owned()))
        .filter(|name| name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .collect();
    for name in token_names {
        write!(css, "--{name}:initial;").unwrap();
    }
    css.push('}');
    css.push_str(include_str!("pm-native.css"));
    css
}

#[cfg(test)]
#[path = "native_style_tests.rs"]
mod tests;
