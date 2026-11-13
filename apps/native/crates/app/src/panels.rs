//! Shared native geometry and library navigation, retained by the v4 host.
use crate::session::Controller;
use gpui::{prelude::*, *};
use pm_shell::layout::Rect;

/// The Scopes panel beside the viewer (Window ▸ Panels ▸ Scopes).
pub fn scopes(v: &mut Controller, id: &str, cx: &mut Context<Controller>) -> Option<AnyElement> {
    (id == "scopes").then(|| v.scopes_panel(cx))
}

/// While the first project is read, and until each panel has mounted for it,
/// the panels it fills say so instead of offering to open one.
pub fn opening(v: &Controller, id: &str, bounds: Rect) -> Option<AnyElement> {
    let name = v.opening_name.as_ref()?;
    // The viewer stays opening while the project waits for effects its
    // extensions are still loading: it never shows them bypassed.
    let waiting = id == "viewer" && v.extensions.settle_after.is_some();
    if !matches!(id, "assets" | "viewer" | "timeline")
        || (v.session.is_some()
            && (v.settling.is_none() || v.builtins.ready.contains(id))
            && !waiting)
    {
        return None;
    }
    let t = v.host.borrow().tokens.clone();
    Some(
        pm_shell::view::empty_state(
            &t,
            &format!("Opening “{name}”…"),
            "It appears here when it’s ready.",
        )
        .id(SharedString::from(format!("opening-{id}")))
        .role(Role::Status)
        .aria_label(format!("Opening {name}"))
        .h(px((bounds.h - t.metrics.header).max(0.)))
        .into_any_element(),
    )
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

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// Status bar copy: document state, then counts. No developer words.
/// Status line trailing facts (language §14 numbers): `1920×1080 · 29.97 fps`.
pub fn format_facts(dimensions: [f64; 2], fps: f64) -> String {
    let fps = if (fps - fps.round()).abs() < 1e-6 {
        format!("{}", fps.round())
    } else {
        format!("{fps:.2}")
    };
    format!(
        "{}×{} · {fps} fps",
        dimensions[0].round(),
        dimensions[1].round()
    )
}
/// `1 file`, `3 files`.
pub fn file_count(count: usize) -> String {
    if count == 1 {
        "1 file".into()
    } else {
        format!("{count} files")
    }
}
fn refused(f: &pm_first_launch::WaitingFolder) -> bool {
    f.outcome == pm_first_launch::source_access::Outcome::Denied
}
/// Whether macOS can still ask about any of `folders` (one it refused it
/// won't ask about again).
pub fn askable(folders: &[pm_first_launch::WaitingFolder]) -> bool {
    folders.iter().any(|f| !refused(f))
}
/// The explanation before Powermove opens the folders (Settings › Media).
pub fn folder_access_prompt(folders: &[pm_first_launch::WaitingFolder]) -> String {
    let named = |refused_ones: bool| {
        folders
            .iter()
            .filter(|f| refused(f) == refused_ones)
            .map(|f| format!("{} ({})", f.name, file_count(f.files)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut text = Vec::new();
    if askable(folders) {
        text.push(format!(
            "Some originals are in folders macOS protects: {}. When you continue, macOS asks once for each folder; allow it so Powermove can bring the files in.",
            named(false)
        ));
    }
    if folders.iter().any(refused) {
        text.push(format!(
            "macOS refused {}. If you've since allowed Powermove in System Settings › Privacy & Security › Files and Folders, continuing checks again; otherwise those files stay offline.",
            named(true)
        ));
    }
    text.push("Your original app and files stay unchanged.".into());
    text.join(" ")
}
/// The status line's offline files, as a person counts them; empty for none.
pub fn offline_summary(count: usize) -> String {
    match count {
        0 => String::new(),
        1 => "1 file offline".into(),
        n => format!("{n} files offline"),
    }
}
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
        CHANNELS, channel_label, display_value, file_count, fit_scale, folder_access_prompt,
        kind_name, needs_frame_outline, offline_summary, parse_background, status_line, zoom_title,
    };

    #[test]
    fn format_facts_read_like_the_status_line() {
        assert_eq!(
            super::format_facts([1920., 1080.], 30.),
            "1920×1080 · 30 fps"
        );
        assert_eq!(
            super::format_facts([3840., 2160.], 30000. / 1001.),
            "3840×2160 · 29.97 fps"
        );
    }
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
    fn kind_names_read_as_words() {
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
    fn the_folder_prompt_names_each_folder_and_what_macos_will_ask() {
        assert_eq!(file_count(1), "1 file");
        let folder = |name: &str, files| pm_first_launch::WaitingFolder {
            path: format!("/x/{name}").into(),
            name: name.into(),
            files,
            outcome: pm_first_launch::source_access::Outcome::Pending,
        };
        let text = folder_access_prompt(&[folder("Downloads", 3), folder("Desktop", 1)]);
        assert!(
            text.contains("Downloads (3 files), Desktop (1 file)"),
            "{text}"
        );
        assert!(text.contains("macOS asks once for each folder"), "{text}");
    }

    #[test]
    fn offline_files_read_as_a_person_counts_them() {
        assert_eq!(offline_summary(0), "");
        assert_eq!(offline_summary(1), "1 file offline");
        assert_eq!(offline_summary(2), "2 files offline");
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
