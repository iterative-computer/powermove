//! Text class scoring (plan section 25): SSIM over ink windows.

#![allow(dead_code)]

/// `(ink SSIM, full SSIM, ink ratio)`: `pm_render::parity::text_ssim`.
pub fn ink_ssim(reference: &[u8], test: &[u8], w: usize, h: usize) -> (f64, f64, f64) {
    pm_render::parity::text_ssim(reference, test, w, h)
}

/// Premultiplied RGBA8 to straight (for viewing), or as it is.
pub fn png_rgba(px: &[u8], unpremultiply: bool) -> Vec<u8> {
    if !unpremultiply {
        return px.to_vec();
    }
    px.chunks_exact(4)
        .flat_map(|p| {
            let a = p[3];
            if a == 0 {
                [0, 0, 0, 0]
            } else {
                let u = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
                [u(p[0]), u(p[1]), u(p[2]), a]
            }
        })
        .collect()
}

/// The CoreText registry's level 1 measure, marked `Send` for the
/// renderer: tests that use it run without the harness, on the main thread,
/// and never move it to another thread (AppKit's font manager is
/// main-thread only). Hosts give the renderer a `Send` resolver (recorded
/// registry answers over full fonts).
#[cfg(target_os = "macos")]
pub struct MainThreadFonts(pub pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>);

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
}
