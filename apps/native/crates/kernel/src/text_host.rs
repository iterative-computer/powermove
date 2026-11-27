//! Text measurement for commands. The kernel owns no fonts; the application
//! attaches a host, and a headless kernel refuses what needs type measured.
use pm_eval::text_geom::{MeasuredLine, TextMeasure};
use pm_text::compat::{GlyphLayout, TextContent};
use std::{fmt, sync::Arc};

/// One font-registry view per command. Measuring may wait on the registry, so
/// only the actor's own planning uses it, never a caller's thread.
pub trait TextHost: Send + Sync {
    fn measure(&self) -> Box<dyn TextMeasure>;
    /// A registry failure met since the last call, so a command never writes
    /// geometry measured with fonts that did not answer.
    fn take_failure(&self) -> Option<String>;
}

#[derive(Clone)]
pub struct SharedTextHost(pub Arc<dyn TextHost>);
impl fmt::Debug for SharedTextHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedTextHost")
    }
}

/// For edit kernels running on layers without text.
pub(crate) struct NoFonts;
impl TextMeasure for NoFonts {
    fn measure_lines(&mut self, _: &TextContent) -> Vec<MeasuredLine> {
        Vec::new()
    }
    fn plain_width(&mut self, _: &TextContent, _: &str) -> f64 {
        0.0
    }
    fn glyph_layout(&mut self, _: &TextContent) -> GlyphLayout {
        GlyphLayout::default()
    }
    fn spaced_width(&mut self, _: &TextContent, _: &str) -> f64 {
        0.0
    }
}
