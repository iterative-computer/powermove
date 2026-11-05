//! Shared native geometry and library navigation, retained by the v4 host.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_shell::layout::Rect;

pub fn library(
    v: &mut Controller,
    id: &str,
    bounds: Rect,
    cx: &mut Context<Controller>,
) -> Option<AnyElement> {
    if id != "assets" || (v.session.is_some() && !v.show_library) {
        return None;
    }
    let t = v.host.borrow().tokens.clone();
    let mut body = div()
        .id("project-library")
        .h(px((bounds.h - t.metrics.header).max(0.)))
        .w_full()
        .p(px(t.metrics.inset))
        .flex()
        .flex_col()
        .overflow_y_scroll();
    for (index, (id, name)) in v.recent.clone().into_iter().enumerate() {
        body = body.child(
            div()
                .id(index)
                .role(Role::Button)
                .aria_label(name.clone())
                .tab_index(0)
                .h(px(t.metrics.row))
                .child(name)
                .on_click(cx.listener(move |v, _, _, cx| v.open_library(id.clone(), cx))),
        );
    }
    if v.recent.is_empty() {
        body = body.justify_center().child(pm_shell::view::empty_state(
            &t,
            "No Recent Projects",
            "Choose File ▸ Open… to open a project.",
        ));
    }
    Some(body.into_any_element())
}
impl Controller {
    pub fn playing(&self) -> bool {
        self.viewer.as_ref().is_some_and(|v| {
            v.status()
                .clock
                .is_some_and(|c| c.state != pm_protocol::clock::TransportState::Stopped)
        })
    }
}
/// Compatibility formatting retained by the native geometry checks.
#[cfg(test)]
const CHANNELS: [&str; 3] = ["opacity", "position.x", "position.y"];

/// Title Case label without its unit (units belong to values).
#[cfg(test)]
pub fn channel_label(channel: &str) -> &'static str {
    match channel {
        "opacity" => "Opacity",
        "position.x" => "Position X",
        "position.y" => "Position Y",
        _ => "Value",
    }
}

/// Value with its unit at the property's display precision: percentages to
/// one decimal, pixels to two, trailing zeros trimmed (`100 %`, `12.5 px`).
#[cfg(test)]
pub fn display_value(channel: &str, value: f64) -> String {
    let (decimals, unit) = match channel {
        "opacity" => (1, "%"),
        _ => (2, "px"),
    };
    let mut text = format!("{value:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    }
    if text == "-0" {
        text = "0".into();
    }
    format!("{text} {unit}")
}

/// Zoom as a whole percentage (`54 %`).
#[cfg(test)]
pub fn percent(zoom: f64) -> String {
    format!("{} %", (zoom * 100.).round() as i64)
}

/// The zoom pop-up title: `Fit (54 %)` or `100 %`.
#[cfg(test)]
pub fn zoom_title(zoom: Option<f64>, fit: f64) -> String {
    match zoom {
        None => format!("Fit ({})", percent(fit)),
        Some(z) => percent(z),
    }
}

/// Fit scale for the body below the viewer bar (the shell's header): the
/// image is centred there with `margin` on every side.
pub fn fit_scale(body_w: f32, body_h: f32, comp: [f64; 2], margin: f32) -> f64 {
    let w = (body_w - margin * 2.).max(1.) as f64 / comp[0].max(1.);
    let h = (body_h - margin * 2.).max(1.) as f64 / comp[1].max(1.);
    w.min(h).max(0.01)
}

/// Clip label colour by layer kind (tokens.md, `colors.labels.*`).
pub fn label_color(kind: &str, labels: &pm_design::Labels) -> u32 {
    match kind {
        "text" | "shape" => labels.blue,
        "video" | "image" | "sequence" => labels.teal,
        "audio" => labels.green,
        "solid" => labels.orange,
        "adjustment" | "effect" => labels.violet,
        _ => labels.gray,
    }
}

/// Display name of a layer kind (`Video`, `Text`, …).
#[cfg(test)]
pub fn kind_name(kind: &str) -> String {
    match kind {
        "precomp" => "Composition".into(),
        "" => "Layer".into(),
        k => {
            let mut c = k.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    }
}

/// `a` moved towards `b` by `t` (sRGB, per channel).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |v: u32, s: u32| ((v >> s) & 255) as f32;
    [16, 8, 0].iter().fold(0, |acc, &s| {
        let v = ch(a, s) + (ch(b, s) - ch(a, s)) * t;
        acc | ((v.round().clamp(0., 255.) as u32) << s)
    })
}

/// The comp frame gets a separator outline when its background is
/// transparent or too close to the viewer surround to show its bounds.
pub fn needs_frame_outline(background: Option<u32>, surround: u32) -> bool {
    background.is_none_or(|bg| {
        [16, 8, 0]
            .iter()
            .map(|s| (((bg >> s) & 255) as i32 - ((surround >> s) & 255) as i32).abs())
            .max()
            .unwrap_or(0)
            <= 24
    })
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`; translucent colours count as transparent.
pub fn parse_background(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#')?;
    let expand: String = if hex.len() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect()
    } else {
        hex.to_owned()
    };
    let v = u32::from_str_radix(expand.get(..6)?, 16).ok()?;
    match expand.len() {
        6 => Some(v),
        8 if expand.get(6..8)? == "ff" || expand.get(6..8)? == "FF" => Some(v),
        _ => None,
    }
}

/// Major ruler interval in frames: the smallest "nice" step at least
/// `min_px` wide.
pub fn ruler_step(fps: f64, px_per_second: f64, min_px: f64) -> f64 {
    let fps = fps.max(1.);
    let frames = [1., 2., 5., 10.];
    let seconds = [
        1., 2., 5., 10., 15., 30., 60., 120., 300., 600., 1800., 3600.,
    ];
    frames
        .into_iter()
        .chain(seconds.into_iter().map(|s| s * fps))
        .find(|f| f / fps * px_per_second >= min_px)
        .unwrap_or(3600. * fps)
}

/// Ruler label for a frame: `0:05` on whole seconds, else `0:05:12`.
pub fn ruler_label(frame: f64, fps: f64) -> String {
    let fps = fps.max(1.).round();
    let frame = frame.round();
    let total = (frame / fps).floor() as i64;
    let rest = (frame - total as f64 * fps) as i64;
    let (m, s) = (total / 60, total % 60);
    if rest == 0 {
        format!("{m}:{s:02}")
    } else {
        format!("{m}:{s:02}:{rest:02}")
    }
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// Status bar copy: document state, then counts. No developer words.
pub fn status_line(dirty: bool, saved_path: bool, layers: usize, keyframes: usize) -> String {
    let counts = format!(
        "{} · {}",
        plural(layers, "layer"),
        plural(keyframes, "keyframe")
    );
    match (dirty, saved_path) {
        (true, _) => format!("Edited · {counts}"),
        (false, true) => format!("Saved · {counts}"),
        (false, false) => counts,
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's `test` attribute macro would shadow `#[test]`.
    use super::{
        CHANNELS, channel_label, display_value, fit_scale, kind_name, label_color, mix,
        needs_frame_outline, parse_background, ruler_label, ruler_step, status_line, zoom_title,
    };

    #[test]
    fn values_carry_units_at_display_precision() {
        assert_eq!(display_value("opacity", 100.), "100 %");
        assert_eq!(display_value("opacity", 50.26), "50.3 %");
        assert_eq!(display_value("position.x", 0.), "0 px");
        assert_eq!(display_value("position.x", -0.001), "0 px");
        assert_eq!(display_value("position.y", 12.5), "12.5 px");
        assert_eq!(display_value("position.y", -3.456), "-3.46 px");
        assert_eq!(channel_label("opacity"), "Opacity");
        assert!(CHANNELS.iter().all(|c| !channel_label(c).contains('%')));
    }

    #[test]
    fn zoom_titles_and_fit_keep_the_image_clear_of_the_bar() {
        assert_eq!(zoom_title(None, 0.5414), "Fit (54 %)");
        assert_eq!(zoom_title(Some(1.), 0.5), "100 %");
        // 1000×600 body below the bar, 12 pt margin, 1920×1080 comp.
        let fit = fit_scale(1000., 600., [1920., 1080.], 12.);
        let image_h = 1080. * fit;
        let top = (600. - image_h) / 2.;
        assert!(top >= 12. - 1e-9, "image top {top} touches the viewer bar");
        assert!(1920. * fit <= 1000. - 24. + 1e-9);
    }

    #[test]
    fn clips_use_label_colours_never_the_accent() {
        for (scheme, increase_contrast) in [
            (pm_design::Scheme::Light, false),
            (pm_design::Scheme::Dark, false),
            (pm_design::Scheme::Light, true),
            (pm_design::Scheme::Dark, true),
        ] {
            let accessibility = pm_design::Accessibility {
                reduce_transparency: false,
                increase_contrast,
                reduce_motion: false,
            };
            let c = pm_design::Tokens::new(scheme, accessibility).colors;
            for kind in [
                "text", "shape", "video", "image", "audio", "solid", "null", "group",
            ] {
                let fill = label_color(kind, &c.labels);
                assert_ne!(fill, c.accent, "{kind}");
                assert_ne!(mix(fill, c.text, 0.12), c.accent, "{kind}");
            }
            assert_eq!(label_color("text", &c.labels), c.labels.blue);
            assert_eq!(label_color("video", &c.labels), c.labels.teal);
        }
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
        assert_eq!(kind_name("video"), "Video");
        assert_eq!(kind_name("precomp"), "Composition");
    }

    #[test]
    fn frame_outline_shows_when_the_comp_cannot_be_told_from_the_surround() {
        assert!(needs_frame_outline(None, 0x0e0e0e));
        assert!(needs_frame_outline(parse_background("#000000"), 0x0e0e0e));
        assert!(needs_frame_outline(parse_background("#1C1C21"), 0x0e0e0e));
        assert!(!needs_frame_outline(parse_background("#ffffff"), 0x0e0e0e));
        assert!(needs_frame_outline(parse_background("#ffffff80"), 0x0e0e0e));
        assert_eq!(parse_background("#abc"), Some(0xaabbcc));
        assert_eq!(parse_background("transparent"), None);
    }

    #[test]
    fn ruler_steps_are_readable_and_labels_are_short() {
        assert_eq!(ruler_step(30., 300., 72.), 10.);
        assert_eq!(ruler_step(30., 60., 72.), 60.);
        assert_eq!(ruler_label(0., 30.), "0:00");
        assert_eq!(ruler_label(150., 30.), "0:05");
        assert_eq!(ruler_label(160., 30.), "0:05:10");
        assert_eq!(ruler_label(30. * 75., 30.), "1:15");
    }

    #[test]
    fn status_line_has_no_developer_words() {
        assert_eq!(
            status_line(false, true, 2, 0),
            "Saved · 2 layers · 0 keyframes"
        );
        assert_eq!(
            status_line(true, true, 1, 1),
            "Edited · 1 layer · 1 keyframe"
        );
        assert_eq!(status_line(false, false, 3, 2), "3 layers · 2 keyframes");
        for s in [
            status_line(true, false, 0, 0),
            status_line(false, true, 5, 9),
        ] {
            assert!(!s.contains("Writable") && !s.contains("Opened"), "{s}");
        }
    }
}
