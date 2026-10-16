//! Text class scoring (plan section 25): SSIM over ink windows.

#![allow(dead_code)]

/// A separable 11-tap Gaussian (σ = 1.5), clamped at the edges, as SSIM's
/// window.
fn window_mean(src: &[f64], w: usize, h: usize) -> Vec<f64> {
    let sigma = 1.5f64;
    let k: Vec<f64> = (-5i32..=5)
        .map(|i| (-f64::from(i * i) / (2.0 * sigma * sigma)).exp())
        .collect();
    let s: f64 = k.iter().sum();
    let k: Vec<f64> = k.iter().map(|v| v / s).collect();
    let mut tmp = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let sx = (x as i64 + i as i64 - 5).clamp(0, w as i64 - 1) as usize;
                acc += kv * src[y * w + sx];
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let sy = (y as i64 + i as i64 - 5).clamp(0, h as i64 - 1) as usize;
                acc += kv * tmp[sy * w + x];
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// `(ink SSIM, full SSIM, ink ratio)` of `test` against `reference`
/// (premultiplied RGBA8). Ink windows: local mean alpha above 1/512 in
/// either image. Ink ratio: Σα(test) / Σα(reference).
pub fn ink_ssim(reference: &[u8], test: &[u8], w: usize, h: usize) -> (f64, f64, f64) {
    let map = pm_render::parity::ssim_map(reference, test, w, h);
    let alpha = |img: &[u8]| -> Vec<f64> { img.chunks_exact(4).map(|p| f64::from(p[3]) / 255.0).collect() };
    let (ar, at) = (alpha(reference), alpha(test));
    let (mr, mt) = (window_mean(&ar, w, h), window_mean(&at, w, h));
    let (mut sum, mut n) = (0.0, 0usize);
    for i in 0..w * h {
        if mr[i] > 1.0 / 512.0 || mt[i] > 1.0 / 512.0 {
            sum += map[i];
            n += 1;
        }
    }
    let full = map.iter().sum::<f64>() / map.len().max(1) as f64;
    let ink = if n == 0 { full } else { sum / n as f64 };
    let ra: f64 = ar.iter().sum();
    let ta: f64 = at.iter().sum();
    let ratio = if ra > 0.0 { ta / ra } else if ta > 0.0 { f64::INFINITY } else { 1.0 };
    (ink, full, ratio)
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
}
