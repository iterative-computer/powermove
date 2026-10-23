/// The CoreText registry's level 1 measure, marked `Send` for the
/// renderer: tests that use it run without the harness, on the main thread,
/// and never move it to another thread (AppKit's font manager is
/// main-thread only). Hosts give the renderer a `Send` resolver (recorded
/// registry answers over full fonts).
#[cfg(target_os = "macos")]
pub struct MainThreadFonts(
    pub pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>,
    pub Option<pm_render::text::Level2Fonts>,
);

// SAFETY: see the type's documentation; the value stays on the main thread.
#[cfg(target_os = "macos")]
unsafe impl Send for MainThreadFonts {}

#[cfg(target_os = "macos")]
impl pm_render::text::TextFonts for MainThreadFonts {
    fn measure(&mut self) -> &mut dyn pm_eval::text_geom::TextMeasure {
        &mut self.0
    }
    fn line_runs(&mut self, d: &pm_text::compat::TextContent) -> Vec<pm_text::compat::LineRuns> {
        self.0.compat().line_runs(d)
    }
    fn text_runs(
        &mut self,
        d: &pm_text::compat::TextContent,
        text: &str,
        tracking: f64,
    ) -> Vec<pm_text::compat::GlyphRun> {
        self.0.compat().text_runs(d, text, tracking).0
    }
    fn face(&mut self, id: pm_text::font::FaceId) -> &pm_text::font::FaceInfo {
        use pm_text::font::FontResolver;
        self.0.compat().resolver().face(id)
    }
    fn em_advance(&mut self, face: pm_text::font::FaceId, size: f32) -> Option<f32> {
        use pm_text::font::FontResolver;
        self.0.compat().resolver_mut().em_advance(face, size)
    }
    fn level2(&mut self) -> Option<&mut pm_render::text::Level2Fonts> {
        self.1.as_mut()
    }
}
