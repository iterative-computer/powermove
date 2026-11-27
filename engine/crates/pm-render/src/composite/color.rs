//! The project's color settings (Settings › Project › Color; section 9,
//! "Project color settings"; contract 4 `ProjectColor`).
//!
//! The document stores them at the root as `colorDepth: 8 | 16 | 32` plus
//! `color: { mode, linearBlending, blendDomain, masteringPeak, hdrTransfer }`.
//! Mode `hdr` keeps BT.2100 light in the working space; it needs float
//! storage, so depth 8 reads as 16 there. `hdrTransfer` (`pq`, the default,
//! or `hlg`) is what an HDR project delivers; together with the mode it is
//! the project's Output.
//! The old `color.precision` is read only when depth is absent. An absent or
//! `null` field, and any absent key, is decision 1's default (Standard,
//! linear blending off, 16-bit float, HDR-safe blend formulas), which is
//! what migrated projects open with. A value this build does not know is
//! refused rather than guessed (the capability check routes it).

use pm_eval::node;
use pm_model::{Node, NodeKind};
use pm_protocol::color::{
    BlendDomain, ColorMode, HdrTransfer, Precision, ProjectColor, WorkingEncoding,
};

/// The color settings a frame renders with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSettings {
    pub working: WorkingEncoding,
    pub blend_domain: BlendDomain,
    /// The project's authoritative intermediate storage precision.
    pub precision: Precision,
    /// Working light goes past SDR white ([`ColorMode::Hdr`]).
    pub hdr: bool,
}

impl ColorSettings {
    pub const STANDARD: ColorSettings = ColorSettings::of(&ProjectColor::DEFAULT);

    pub const fn of(project: &ProjectColor) -> ColorSettings {
        ColorSettings {
            working: project.working(),
            blend_domain: project.blend_domain,
            precision: project.precision,
            hdr: matches!(project.mode, ColorMode::Hdr),
        }
    }

    /// Whether intermediates are linear light.
    pub fn linear(&self) -> bool {
        self.working == WorkingEncoding::LinearLight
    }
}

/// The project's `color` field, or why it cannot be read.
pub fn project_color(root: &Node) -> Result<ProjectColor, String> {
    let mut out = ProjectColor::DEFAULT;
    let color = node::member(Some(root), "color");
    let map = match color.map(Node::kind) {
        None | Some(NodeKind::Null) => None,
        Some(NodeKind::Object(map)) => Some(map),
        _ => return Err("the project color setting is not an object".into()),
    };
    for (key, value) in map.into_iter().flat_map(|map| map.iter()) {
        let key = key.to_string_lossy();
        let text = node::string(Some(value)).map(|s| s.to_string_lossy());
        match (key.as_str(), text.as_deref(), value.kind()) {
            ("mode", Some("standard"), _) => out.mode = ColorMode::Standard,
            ("mode", Some("hdr"), _) => out.mode = ColorMode::Hdr,
            ("masteringPeak", _, NodeKind::Number(n)) if (203.0..=10000.0).contains(n) => {
                out.mastering_peak = *n as u16
            }
            ("hdrTransfer", Some("pq"), _) => out.hdr_transfer = HdrTransfer::Pq,
            ("hdrTransfer", Some("hlg"), _) => out.hdr_transfer = HdrTransfer::Hlg,
            ("linearBlending", _, NodeKind::Bool(b)) => out.linear_blending = *b,
            ("precision", Some("float16"), _) => out.precision = Precision::Float16,
            ("precision", Some("float32"), _) => out.precision = Precision::Float32,
            ("blendDomain", Some("hdrSafe"), _) => out.blend_domain = BlendDomain::HdrSafe,
            ("blendDomain", Some("displayEncoded"), _) => {
                out.blend_domain = BlendDomain::DisplayEncoded
            }
            _ => return Err(format!("project color setting {key} has an unknown value")),
        }
    }
    // Main's setting wins over the engine's retired Precision field. A missing
    // depth retains old native projects' precision until the file is migrated.
    if let Some(depth) = node::member(Some(root), "colorDepth") {
        out.precision = match depth.kind() {
            NodeKind::Number(n) if *n == 8.0 => Precision::Unorm8,
            NodeKind::Number(n) if *n == 32.0 => Precision::Float32,
            _ => Precision::Float16, // normalizeColorDepth in the shipped app
        };
    }
    if out.mode == ColorMode::Hdr && out.precision == Precision::Unorm8 {
        out.precision = Precision::Float16;
    }
    Ok(out)
}

/// A premultiplied E-encoded color (hex solids, backgrounds) in the
/// working encoding.
pub fn premultiplied_to_working(rgba: [f64; 4], working: WorkingEncoding) -> [f64; 4] {
    match working {
        WorkingEncoding::Standard => rgba,
        WorkingEncoding::LinearLight => pm_color::linear::premultiplied_e_to_linear(rgba),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(json: &str) -> Node {
        Node::from_value(pm_model::json::parse(json).unwrap(), 0)
    }

    #[test]
    fn absent_and_partial_settings_are_the_default() {
        assert_eq!(project_color(&root("{}")), Ok(ProjectColor::DEFAULT));
        assert_eq!(
            project_color(&root(r#"{"color":null}"#)),
            Ok(ProjectColor::DEFAULT)
        );
        let linear = project_color(&root(r#"{"color":{"linearBlending":true}}"#)).unwrap();
        assert!(linear.linear_blending);
        assert_eq!(linear.precision, Precision::Float16);
        let all = project_color(&root(
            r#"{"color":{"mode":"standard","linearBlending":false,"precision":"float32","blendDomain":"displayEncoded"}}"#,
        ))
        .unwrap();
        assert_eq!(all.precision, Precision::Float32);
        assert_eq!(all.blend_domain, BlendDomain::DisplayEncoded);
    }

    #[test]
    fn the_output_is_sdr_unless_the_project_is_hdr_and_pq_unless_it_says_hlg() {
        use pm_protocol::color::ProjectOutput;
        for (json, output) in [
            (r#"{}"#, ProjectOutput::Sdr),
            (r#"{"color":{"hdrTransfer":"hlg"}}"#, ProjectOutput::Sdr),
            (r#"{"color":{"mode":"hdr"}}"#, ProjectOutput::Pq),
            (
                r#"{"color":{"mode":"hdr","hdrTransfer":"pq"}}"#,
                ProjectOutput::Pq,
            ),
            (
                r#"{"color":{"mode":"hdr","hdrTransfer":"hlg"}}"#,
                ProjectOutput::Hlg,
            ),
        ] {
            assert_eq!(
                project_color(&root(json)).unwrap().output(),
                output,
                "{json}"
            );
        }
        assert!(project_color(&root(r#"{"color":{"hdrTransfer":"slog3"}}"#)).is_err());
    }

    #[test]
    fn color_depth_is_authoritative_and_legacy_precision_migrates() {
        for (depth, precision) in [
            (8, Precision::Unorm8),
            (16, Precision::Float16),
            (32, Precision::Float32),
        ] {
            let json = format!(r#"{{"colorDepth":{depth},"color":{{"precision":"float32"}}}}"#);
            assert_eq!(project_color(&root(&json)).unwrap().precision, precision);
        }
        for json in [
            r#"{}"#,
            r#"{"colorDepth":"32"}"#,
            r#"{"colorDepth":64}"#,
            r#"{"colorDepth":null}"#,
        ] {
            assert_eq!(
                project_color(&root(json)).unwrap().precision,
                Precision::Float16
            );
        }
        assert_eq!(
            project_color(&root(r#"{"color":{"precision":"float32"}}"#))
                .unwrap()
                .precision,
            Precision::Float32
        );
    }

    #[test]
    fn unknown_settings_are_refused() {
        for bad in [
            r#"{"color":{"mode":"aces"}}"#,
            r#"{"color":{"precision":"float64"}}"#,
            r#"{"color":{"linearBlending":"yes"}}"#,
            r#"{"color":{"gamut":"p3"}}"#,
            r#"{"color":true}"#,
        ] {
            assert!(project_color(&root(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn colors_convert_to_linear_light() {
        let c = premultiplied_to_working([0.5, 0.25, 0.0, 0.5], WorkingEncoding::LinearLight);
        assert!(
            (c[0] - 0.5).abs() < 1e-12,
            "white at 50% stays 0.5 premultiplied"
        );
        assert!((c[1] - pm_color::transfer::display_decode(0.5) * 0.5).abs() < 1e-12);
        assert_eq!(
            premultiplied_to_working([0.5, 0.25, 0.0, 0.5], WorkingEncoding::Standard),
            [0.5, 0.25, 0.0, 0.5]
        );
    }
}
