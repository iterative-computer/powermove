//! SVG's resolved horizontal text spans through pm-text. No usvg glyph
//! outlines enter the bitmap. The host owns the resolver and font lifetime.
use super::{shape::Bitmap, skpath::Mat};
use crate::render::svg::SvgError;
use pm_text::render::{
    bitmap::Bitmaps,
    outline::Outlines,
    profile::Profile,
    scene::{Painter, TextScene},
};
use pm_text::{
    compat::{Compat, TextContent},
    font::FontResolver,
};

pub struct TextRenderer<'a, R> {
    compat: &'a mut Compat<R>,
    outlines: Outlines,
    bitmaps: Bitmaps,
}
impl<'a, R: FontResolver> TextRenderer<'a, R> {
    pub fn new(compat: &'a mut Compat<R>) -> Self {
        Self {
            compat,
            outlines: Outlines::new(),
            bitmaps: Bitmaps::new(),
        }
    }
    pub fn raster(&mut self, text: &usvg::Text, m: Mat, width: u32, height: u32) -> Result<Bitmap, SvgError> {
        if text.writing_mode() != usvg::WritingMode::LeftToRight
            || text.dx().iter().chain(text.dy()).chain(text.rotate()).any(|v| *v != 0.0)
        {
            return Err(SvgError(
                "SVG per-character positioning/vertical text needs a pm-text placement adapter".into(),
            ));
        }
        let matrix = kurbo::Affine::new([m.sx, m.ky, m.kx, m.sy, m.tx, m.ty].map(f64::from));
        let mut scene = TextScene {
            width,
            height,
            items: Vec::new(),
        };
        let mut pen = (0.0, 0.0);
        for chunk in text.chunks() {
            if !matches!(chunk.text_flow(), usvg::TextFlow::Linear) {
                return Err(SvgError("SVG text-on-path placement is not matched".into()));
            }
            pen = (chunk.x().map(f64::from).unwrap_or(pen.0), chunk.y().map(f64::from).unwrap_or(pen.1));
            let mut spans = Vec::new();
            let mut advance = 0.0;
            for span in chunk.spans() {
                let decoration = span.decoration();
                if span.small_caps()
                    || !span.apply_kerning()
                    || !matches!(
                        span.dominant_baseline(),
                        usvg::DominantBaseline::Auto | usvg::DominantBaseline::Alphabetic
                    )
                    || !matches!(
                        span.alignment_baseline(),
                        usvg::AlignmentBaseline::Auto | usvg::AlignmentBaseline::Baseline | usvg::AlignmentBaseline::Alphabetic
                    )
                    || span
                        .baseline_shift()
                        .iter()
                        .any(|v| !matches!(v, usvg::BaselineShift::Baseline | usvg::BaselineShift::Number(0.0)))
                    || decoration.underline().is_some()
                    || decoration.overline().is_some()
                    || decoration.line_through().is_some()
                {
                    return Err(SvgError("SVG text feature/baseline/decoration is not matched".into()));
                }
                if span.stroke().is_some()
                    || span.word_spacing() != 0.0
                    || span.text_length().is_some()
                    || span.font().stretch() != usvg::FontStretch::Normal
                {
                    return Err(SvgError("SVG text stroke/spacing/stretch is not matched".into()));
                }
                let family = span
                    .font()
                    .families()
                    .iter()
                    .map(|f| match f {
                        usvg::FontFamily::Named(s) => s.clone(),
                        _ => f.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let content = TextContent {
                    font: Some(family),
                    size: Some(f64::from(span.font_size().get())),
                    weight: Some(f64::from(span.font().weight())),
                    italic: Some(span.font().style() != usvg::FontStyle::Normal),
                    axes: span
                        .font()
                        .variations()
                        .iter()
                        .map(|v| (String::from_utf8_lossy(&v.tag).to_string(), f64::from(v.value)))
                        .collect(),
                    ..Default::default()
                };
                let (runs, w) = self
                    .compat
                    .text_runs(&content, &chunk.text()[span.start()..span.end()], f64::from(span.letter_spacing()));
                spans.push((span, runs, advance));
                advance += w;
            }
            let anchor = match chunk.anchor() {
                usvg::TextAnchor::Start => 0.0,
                usvg::TextAnchor::Middle => advance * 0.5,
                usvg::TextAnchor::End => advance,
            };
            for (span, runs, x) in spans {
                if !span.is_visible() {
                    continue;
                }
                if let Some(fill) = span.fill() {
                    let usvg::Paint::Color(c) = fill.paint() else {
                        return Err(SvgError("SVG text paint server is not matched".into()));
                    };
                    let color = [f64::from(c.red) / 255.0, f64::from(c.green) / 255.0, f64::from(c.blue) / 255.0, 1.0];
                    let mut painter = Painter {
                        outlines: &mut self.outlines,
                        bitmaps: &mut self.bitmaps,
                        masks: None,
                        viewport: None,
                        profile: Profile::CANVAS2D,
                        unsupported: Vec::new(),
                    };
                    let runs = pm_text::render::runs(self.compat.resolver(), &runs);
                    painter.fill_text(
                        &mut scene.items,
                        &runs,
                        (pen.0 + x - anchor, pen.1),
                        matrix,
                        color,
                        f64::from(fill.opacity().get()),
                        0.0,
                    );
                    if !painter.unsupported.is_empty() {
                        return Err(SvgError(format!("SVG text: {:?}", painter.unsupported)));
                    }
                }
            }
            pen.0 += advance;
        }
        Ok(Bitmap {
            width,
            height,
            rgba: pm_text::render::paint::render_cpu(&scene).to_rgba8(),
        })
    }
}
