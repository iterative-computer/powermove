//! Chromium's accelerated Canvas2D, emulated for `compat` level 1
//! (docs/engine/vector.md).

pub mod analytic;
pub mod blinksvg;
pub mod blur;
pub mod dash;
pub mod gradient;
pub mod blink;
pub mod context;
pub mod convex;
pub mod m44;
pub mod skrrect;
pub mod msaa;
pub mod paint;
pub mod shape;
pub mod skpath;
pub mod stroke;
pub mod svg;
pub mod svgrender;
pub mod tess;
pub mod svg_text;

/// A diagnostic switch of the SVG replay (`PM_SVG_DEBUG`, `PM_SVG_PROBE`,
/// `PM_SVG_MESH`, `PM_SVG_STRADDLE`, `PM_SVG_S0_RULE`): read from the
/// environment only in builds with the `svg-diagnostics` feature, so
/// production rendering never prints, writes files, or changes output
/// because of the environment.
#[cfg(feature = "render")]
pub(crate) fn diagnostic(name: &str) -> Option<String> {
    #[cfg(feature = "svg-diagnostics")]
    {
        std::env::var(name).ok()
    }
    #[cfg(not(feature = "svg-diagnostics"))]
    {
        let _ = name;
        None
    }
}
