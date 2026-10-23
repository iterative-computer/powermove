//! The project's color settings (Settings › Project › Color; section 9,
//! "Project color settings"; contract 4 `ProjectColor`).
//!
//! The document stores them at the root as
//! `color: { mode, linearBlending, precision, blendDomain }`. An absent or
//! `null` field, and any absent key, is decision 1's default (Standard,
//! linear blending off, 16-bit float, HDR-safe blend formulas), which is
//! what migrated projects open with. A value this build does not know is
//! refused rather than guessed (the capability check routes it).

use pm_eval::node;
use pm_model::{Node, NodeKind};
use pm_protocol::color::{BlendDomain, ColorMode, Precision, ProjectColor, WorkingEncoding};

/// The color settings a frame renders with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorSettings {
    pub working: WorkingEncoding,
    pub blend_domain: BlendDomain,
    /// The project's intermediate precision (a floor for every render of
    /// the project).
    pub precision: Precision,
}

impl ColorSettings {
    pub const STANDARD: ColorSettings = ColorSettings::of(&ProjectColor::DEFAULT);

    pub const fn of(project: &ProjectColor) -> ColorSettings {
        ColorSettings {
            working: project.working(),
            blend_domain: project.blend_domain,
            precision: project.precision,
        }
    }

    /// Whether intermediates are linear light.
    pub fn linear(&self) -> bool {
        self.working == WorkingEncoding::LinearLight
    }
}

/// The project's `color` field, or why it cannot be read.
pub fn project_color(root: &Node) -> Result<ProjectColor, String> {
    let Some(color) = node::member(Some(root), "color") else {
        return Ok(ProjectColor::DEFAULT);
    };
    let map = match color.kind() {
        NodeKind::Null => return Ok(ProjectColor::DEFAULT),
        NodeKind::Object(map) => map,
        _ => return Err("the project color setting is not an object".into()),
    };
    let mut out = ProjectColor::DEFAULT;
    for (key, value) in map.iter() {
        let key = key.to_string_lossy();
        let text = node::string(Some(value)).map(|s| s.to_string_lossy());
        match (key.as_str(), text.as_deref(), value.kind()) {
            ("mode", Some("standard"), _) => out.mode = ColorMode::Standard,
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
    Ok(out)
}

/// Whether the project asks for 32-bit float intermediates through the
/// current app's Settings › Project › Color depth (`colorDepth`: 8, 16, or
/// 32 bits per channel; absent or anything else is 16, `normalizeColorDepth`).
/// The current compositor falls back to 16 where the GPU lacks float
/// rendering (`availableColorDepth`); the renderer does the same.
pub fn color_depth_float32(root: &Node) -> bool {
    matches!(node::member(Some(root), "colorDepth").map(Node::kind), Some(NodeKind::Number(n)) if *n == 32.0)
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
    fn color_depth_32_asks_for_float32() {
        assert!(color_depth_float32(&root(r#"{"colorDepth":32}"#)));
        for other in [
            r#"{}"#,
            r#"{"colorDepth":16}"#,
            r#"{"colorDepth":8}"#,
            r#"{"colorDepth":"32"}"#,
        ] {
            assert!(!color_depth_float32(&root(other)), "{other}");
        }
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
