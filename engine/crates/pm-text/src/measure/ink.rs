//! Same unhinted font-ink contract as pm-eval::text_measure; no painter.
use skrifa::instance::Size;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, GlyphId, MetadataProvider};
pub(super) fn glyph_ink(
    data: &crate::font::FontBytes,
    run: &crate::level2::Run,
    glyph: u32,
) -> Result<Option<[f64; 4]>, super::MeasureError> {
    let skew = -(f64::from(run.skew) * core::f64::consts::PI / 180.0).tan();
    // Color glyphs: the level 2 painter's strike or COLR program placement.
    if let Some(r) = crate::render::ink::drawn_color_glyph_ink(
        crate::render::outline::FaceRef {
            data: data.as_slice(),
            index: 0,
            id: data.id(),
        },
        crate::render::outline::Coords::Normalized(&run.coords),
        glyph,
        run.size,
        skew,
        None,
    ) {
        return Ok(r.map(|r| [r.x0, r.y0, r.x1, r.y1]));
    }
    let font = FontRef::new(data.as_slice())
        .map_err(|_| super::MeasureError::InvalidFont(run.font.clone()))?;
    let coords: Vec<_> = run
        .coords
        .iter()
        .map(|v| skrifa::instance::NormalizedCoord::from_bits(*v))
        .collect();
    let mut pen = ControlBox {
        b: [0.0; 4],
        any: false,
    };
    if let Some(outline) = font.outline_glyphs().get(GlyphId::new(glyph)) {
        outline
            .draw(
                DrawSettings::unhinted(Size::new(run.size), coords.as_slice()),
                &mut pen,
            )
            .map_err(|_| super::MeasureError::InvalidFont(run.font.clone()))?;
    }
    if !pen.any {
        // Nothing is drawn for a glyph with neither outline nor color source.
        return Ok(None);
    }
    let mut ink = [
        f64::from(pen.b[0]),
        -f64::from(pen.b[3]),
        f64::from(pen.b[2]),
        -f64::from(pen.b[1]),
    ];
    if run.synthetic_bold {
        // The CoreText profile's synthetic face is an outline dilation.
        let x = f64::from(run.size) * 0.00875;
        let y = x * 0.925;
        ink = [ink[0] - x, ink[1] - y, ink[2] + x, ink[3] + y];
    }
    let (a, b) = (skew * ink[1], skew * ink[3]);
    ink[0] += a.min(b);
    ink[2] += a.max(b);
    Ok(Some(ink))
}

struct ControlBox {
    b: [f32; 4],
    any: bool,
}

impl ControlBox {
    fn point(&mut self, x: f32, y: f32) {
        if !self.any {
            self.b = [x, y, x, y];
            self.any = true;
            return;
        }
        self.b[0] = self.b[0].min(x);
        self.b[1] = self.b[1].min(y);
        self.b[2] = self.b[2].max(x);
        self.b[3] = self.b[3].max(y);
    }
}

impl OutlinePen for ControlBox {
    fn move_to(&mut self, x: f32, y: f32) {
        self.point(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.point(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.point(cx0, cy0);
        self.point(x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.point(cx0, cy0);
        self.point(cx1, cy1);
        self.point(x, y);
    }
    fn close(&mut self) {}
}
