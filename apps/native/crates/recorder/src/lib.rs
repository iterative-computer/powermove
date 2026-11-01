//! Headless Canvas2D command recording, versioned pixels, and analytic CPU paint.
//! See docs/native/recorder.md for the binding boundary and conformance limits.
#![forbid(unsafe_code)]
mod composite;
mod coverage;
mod image;
mod paint;
mod path;
mod raster;
mod text;

pub use composite::Composite;
pub use image::{ImageData, Source, SourceKey, Sources};
pub use kurbo::{Affine, Cap, Join, Point, Rect, Stroke, Vec2};
use kurbo::{BezPath, Shape};
use paint::FrozenPaint;
pub use paint::{Gradient, GradientKind, Paint, Pattern, Repetition};
pub use path::Path;
use pm_vector::render::scene::{CoverageItem, ItemStyle};
pub use pm_vector::render::{color::CanvasColor as Color, scene::FillRule};
pub use raster::CpuRasterizer;
use std::sync::Arc;
pub use text::{
    Direction, Font, TextAlign, TextBaseline, TextEngine, TextLayout, TextMetrics, TextProvider,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    IndexSize,
    Range,
    Syntax,
    NotSupported,
    Capacity,
    MissingSource,
    VersionConflict,
    MissingFont,
    Poisoned,
    NeedsPromotion(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct DrawingState {
    pub transform: Affine,
    pub fill: Paint,
    pub stroke: Paint,
    pub line: Stroke,
    pub alpha: f32,
    pub composite: Composite,
    pub font: Font,
    pub text_align: TextAlign,
    pub text_baseline: TextBaseline,
    pub direction: Direction,
    pub image_smoothing: bool,
    clips: Arc<Vec<Clip>>,
}
#[derive(Debug, Clone)]
struct Clip {
    path: BezPath,
    rule: FillRule,
}
impl Default for DrawingState {
    fn default() -> Self {
        Self {
            transform: Affine::IDENTITY,
            fill: Paint::default(),
            stroke: Paint::default(),
            line: Stroke::new(1.)
                .with_miter_limit(10.)
                .with_caps(Cap::Butt)
                .with_join(Join::Miter),
            alpha: 1.,
            composite: Composite::SourceOver,
            font: Font::default(),
            text_align: TextAlign::Start,
            text_baseline: TextBaseline::Alphabetic,
            direction: Direction::Ltr,
            image_smoothing: true,
            clips: Arc::new(Vec::new()),
        }
    }
}
#[derive(Debug, Clone)]
pub struct DrawCommand {
    path: BezPath,
    style: ItemStyle,
    transform: Affine,
    paint: FrozenPaint,
    alpha: f32,
    composite: Composite,
    clips: Arc<Vec<Clip>>,
    clear: bool,
}
#[derive(Debug, Clone)]
pub enum Command {
    Draw(Box<DrawCommand>),
    Put {
        image: Arc<ImageData>,
        dx: i32,
        dy: i32,
        dirty: [i32; 4],
    },
}
impl DrawCommand {
    pub fn path(&self) -> &BezPath {
        &self.path
    }
    pub fn transform(&self) -> Affine {
        self.transform
    }
    pub fn style(&self) -> &ItemStyle {
        &self.style
    }
    pub fn composite(&self) -> Composite {
        self.composite
    }
    pub fn global_alpha(&self) -> f32 {
        self.alpha
    }
    pub fn clip_count(&self) -> usize {
        self.clips.len()
    }
}
impl Command {
    pub fn source_key(&self) -> Option<SourceKey> {
        match self {
            Self::Draw(d) => match &d.paint {
                FrozenPaint::Image { source, .. } => Some(source.key),
                _ => None,
            },
            Self::Put { .. } => None,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Draw(d) if d.clear => "clear",
            Self::Draw(_) => "draw",
            Self::Put { .. } => "put-image-data",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAttributes {
    pub alpha: bool,
    pub color_space: &'static str,
    pub color_type: &'static str,
}
/// The target must complete synchronously, retaining its own copy when necessary.
/// It must preserve the JS canvas/context wrappers; this hook never replaces them.
pub trait PromotionTarget {
    fn present(&mut self, snapshot: &PromotionSnapshot) -> Result<()>;
}
#[derive(Debug, Clone)]
pub struct PromotionSnapshot {
    pub key: SourceKey,
    pub pixels: Arc<ImageData>,
    pub state: DrawingState,
    pub path: Path,
    pub saved_states: Vec<DrawingState>,
}

pub struct Canvas {
    id: u64,
    version: u64,
    width: u32,
    height: u32,
    state: DrawingState,
    stack: Vec<DrawingState>,
    path: Path,
    commands: Vec<Command>,
    surface: CpuRasterizer,
    rendered: usize,
    promoted: Option<SourceKey>,
}
impl Canvas {
    pub fn new(id: u64, width: u32, height: u32) -> Result<Self> {
        let surface = CpuRasterizer::new(width, height)?;
        Ok(Self {
            id,
            version: 0,
            width,
            height,
            state: DrawingState::default(),
            stack: Vec::new(),
            path: Path::default(),
            commands: Vec::new(),
            surface,
            rendered: 0,
            promoted: None,
        })
    }
    pub fn key(&self) -> SourceKey {
        SourceKey {
            id: self.id,
            version: self.version,
        }
    }
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn state(&self) -> &DrawingState {
        &self.state
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }
    pub fn context_attributes(&self) -> ContextAttributes {
        ContextAttributes {
            alpha: true,
            color_space: "srgb",
            color_type: "unorm8",
        }
    }
    pub fn is_context_lost(&self) -> bool {
        false
    }
    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
    }
    pub fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.state = s;
        }
    }
    pub fn set_fill(&mut self, p: Paint) {
        self.state.fill = p;
    }
    pub fn set_stroke(&mut self, p: Paint) {
        self.state.stroke = p;
    }
    pub fn set_fill_color(&mut self, s: &str) {
        if let Some(c) = pm_vector::render::color::parse(s) {
            self.set_fill(Paint::Solid(c));
        }
    }
    pub fn set_stroke_color(&mut self, s: &str) {
        if let Some(c) = pm_vector::render::color::parse(s) {
            self.set_stroke(Paint::Solid(c));
        }
    }
    pub fn set_global_alpha(&mut self, a: f64) {
        if (0. ..=1.).contains(&a) {
            self.state.alpha = a as f32;
        }
    }
    pub fn set_composite(&mut self, s: &str) {
        if let Some(c) = Composite::parse(s) {
            self.state.composite = c;
        }
    }
    pub fn set_line_width(&mut self, w: f64) {
        if w.is_finite() && w > 0. {
            self.state.line.width = w;
        }
    }
    pub fn set_line_cap(&mut self, c: Cap) {
        self.state.line.start_cap = c;
        self.state.line.end_cap = c;
    }
    pub fn set_line_join(&mut self, j: Join) {
        self.state.line.join = j;
    }
    pub fn set_miter_limit(&mut self, m: f64) {
        if m.is_finite() && m > 0. {
            self.state.line.miter_limit = m;
        }
    }
    pub fn set_line_dash(&mut self, d: &[f64]) {
        if d.iter().any(|v| !v.is_finite() || *v < 0.) {
            return;
        }
        self.state.line.dash_pattern.clear();
        self.state.line.dash_pattern.extend_from_slice(d);
        if d.len() % 2 == 1 {
            self.state.line.dash_pattern.extend_from_slice(d);
        }
        if self.state.line.dash_pattern.iter().all(|v| *v == 0.) {
            self.state.line.dash_pattern.clear();
        }
    }
    pub fn set_line_dash_offset(&mut self, d: f64) {
        if d.is_finite() {
            self.state.line.dash_offset = d;
        }
    }
    pub fn set_font(&mut self, s: &str) -> Result<()> {
        match Font::parse(s, 16., 16.) {
            Ok(f) => {
                self.state.font = f;
                Ok(())
            }
            Err(Error::Syntax) => Ok(()),
            Err(e) => Err(e),
        }
    }
    pub fn set_text_align(&mut self, a: TextAlign) {
        self.state.text_align = a;
    }
    pub fn set_text_baseline(&mut self, b: TextBaseline) {
        self.state.text_baseline = b;
    }
    pub fn set_direction(&mut self, d: Direction) {
        self.state.direction = d;
    }
    pub fn set_image_smoothing(&mut self, s: bool) {
        self.state.image_smoothing = s;
    }
    pub fn set_transform(&mut self, m: Affine) {
        if m.as_coeffs().iter().all(|v| v.is_finite()) {
            self.state.transform = m;
        }
    }
    pub fn transform(&mut self, m: Affine) {
        if m.as_coeffs().iter().all(|v| v.is_finite()) {
            self.set_transform(self.state.transform * m);
        }
    }
    pub fn reset_transform(&mut self) {
        self.state.transform = Affine::IDENTITY;
    }
    pub fn translate(&mut self, x: f64, y: f64) {
        self.transform(Affine::translate((x, y)));
    }
    pub fn scale(&mut self, x: f64, y: f64) {
        self.transform(Affine::scale_non_uniform(x, y));
    }
    pub fn rotate(&mut self, r: f64) {
        if r.is_finite() {
            let (s, c) = (libm::sin(r), libm::cos(r));
            self.transform(Affine::new([c, s, -s, c, 0., 0.]));
        }
    }
    pub fn begin_path(&mut self) {
        self.path.clear();
    }
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.path.move_to(self.state.transform, x, y);
    }
    pub fn line_to(&mut self, x: f64, y: f64) {
        self.path.line_to(self.state.transform, x, y);
    }
    pub fn close_path(&mut self) {
        self.path.close();
    }
    pub fn quadratic_curve_to(&mut self, c: Point, p: Point) {
        self.path.quadratic_curve_to(self.state.transform, c, p);
    }
    pub fn bezier_curve_to(&mut self, a: Point, b: Point, p: Point) {
        self.path.bezier_curve_to(self.state.transform, a, b, p);
    }
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.path
            .rect(self.state.transform, Rect::new(x, y, x + w, y + h));
    }
    pub fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: &[Vec2]) -> Result<()> {
        self.path.round_rect(self.state.transform, x, y, w, h, r)
    }
    pub fn arc(&mut self, center: Point, r: f64, start: f64, end: f64, ccw: bool) -> Result<()> {
        self.ellipse(center, Vec2::new(r, r), 0., start, end, ccw)
    }
    pub fn ellipse(
        &mut self,
        center: Point,
        radii: Vec2,
        rotation: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> Result<()> {
        self.path.ellipse(
            self.state.transform,
            center,
            radii,
            rotation,
            start,
            end,
            ccw,
        )
    }
    pub fn arc_to(&mut self, a: Point, b: Point, r: f64) -> Result<()> {
        self.path.arc_to(self.state.transform, a, b, r)
    }
    pub fn clip(&mut self, rule: FillRule) {
        Arc::make_mut(&mut self.state.clips).push(Clip {
            path: self.path.data.clone(),
            rule,
        });
    }
    pub fn is_point_in_path(&self, p: Point, rule: FillRule) -> bool {
        self.path.contains(p, rule)
    }
    pub fn is_point_in_stroke(&self, p: Point) -> bool {
        if !p.is_finite() {
            return false;
        }
        let item = self.stroke_item(self.path.data.clone());
        item.is_some_and(|(path, transform)| {
            CoverageItem {
                path,
                transform,
                style: ItemStyle::Stroke(self.state.line.clone()),
                color: [1.; 4],
            }
            .device_outline()
            .winding(p)
                != 0
        })
    }
    pub fn fill(&mut self, rule: FillRule) -> Result<()> {
        self.draw(
            self.path.data.clone(),
            ItemStyle::Fill(rule),
            Affine::IDENTITY,
            self.state.fill.clone(),
            false,
        )
    }
    pub fn stroke(&mut self) -> Result<()> {
        if let Some((path, m)) = self.stroke_item(self.path.data.clone()) {
            self.draw(
                path,
                ItemStyle::Stroke(self.state.line.clone()),
                m,
                self.state.stroke.clone(),
                false,
            )
        } else {
            Ok(())
        }
    }
    fn stroke_item(&self, path: BezPath) -> Option<(BezPath, Affine)> {
        let m = self.state.transform;
        (m.determinant() != 0.).then(|| (m.inverse() * path, m))
    }
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<()> {
        self.rect_draw(x, y, w, h, false, false)
    }
    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<()> {
        self.rect_draw(x, y, w, h, true, false)
    }
    pub fn clear_rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> Result<()> {
        self.rect_draw(x, y, w, h, false, true)
    }
    fn rect_draw(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        stroke: bool,
        clear: bool,
    ) -> Result<()> {
        if !path::finite(&[x, y, w, h, x + w, y + h]) || (!stroke && (w == 0. || h == 0.)) {
            return Ok(());
        }
        let mut p = Path::default();
        if stroke && (w == 0. || h == 0.) {
            if w == 0. && h == 0. {
                return Ok(());
            }
            p.move_to(Affine::IDENTITY, x, y);
            p.line_to(Affine::IDENTITY, x + w, y + h);
        } else {
            p.rect(Affine::IDENTITY, Rect::new(x, y, x + w, y + h));
        }
        self.draw(
            p.data,
            if stroke {
                ItemStyle::Stroke(self.state.line.clone())
            } else {
                ItemStyle::Fill(FillRule::NonZero)
            },
            self.state.transform,
            if stroke {
                self.state.stroke.clone()
            } else {
                self.state.fill.clone()
            },
            clear,
        )
    }
    fn draw(
        &mut self,
        path: BezPath,
        style: ItemStyle,
        transform: Affine,
        paint: Paint,
        clear: bool,
    ) -> Result<()> {
        if path.elements().is_empty() {
            return Ok(());
        }
        let paint = if clear {
            FrozenPaint::Solid([0.; 4])
        } else {
            paint.freeze(self.state.transform, self.state.image_smoothing)?
        };
        self.record(Command::Draw(Box::new(DrawCommand {
            path,
            style,
            transform,
            paint,
            alpha: self.state.alpha,
            composite: self.state.composite,
            clips: self.state.clips.clone(),
            clear,
        })))
    }
    fn record(&mut self, c: Command) -> Result<()> {
        // Bound an unflushed batch; flushing retains the exact float backing.
        if self.commands.len() >= 65_536 {
            return Err(Error::Capacity);
        }
        let version = self.version.checked_add(1).ok_or(Error::Capacity)?;
        self.commands.push(c);
        self.version = version;
        Ok(())
    }
    /// drawImage's 3/5/9-argument forms share this normalized source/destination primitive.
    /// Negative sizes extend in the opposite direction and never flip the pixels.
    pub fn draw_image(&mut self, source: Source, src: Rect, dst: Rect) -> Result<()> {
        if !path::finite(&[
            src.x0, src.y0, src.x1, src.y1, dst.x0, dst.y0, dst.x1, dst.y1,
        ]) {
            return Ok(());
        }
        let src = src.abs();
        let dst = dst.abs();
        if src.width() == 0. || src.height() == 0. || dst.width() == 0. || dst.height() == 0. {
            return Ok(());
        }
        let bounded = src.intersect(Rect::new(
            0.,
            0.,
            source.pixels.width() as f64,
            source.pixels.height() as f64,
        ));
        if bounded.width() <= 0. || bounded.height() <= 0. {
            return Ok(());
        }
        let local = Affine::translate((dst.x0, dst.y0))
            * Affine::scale_non_uniform(dst.width() / src.width(), dst.height() / src.height())
            * Affine::translate((-src.x0, -src.y0));
        let m = self.state.transform * local;
        if m.determinant() == 0. {
            return Ok(());
        }
        let command = DrawCommand {
            path: bounded.to_path(1e-4),
            style: ItemStyle::Fill(FillRule::NonZero),
            transform: m,
            paint: FrozenPaint::Image {
                source,
                to_source: m.inverse(),
                repetition: Repetition::NoRepeat,
                smooth: self.state.image_smoothing,
                extend: true,
                crop: bounded,
            },
            alpha: self.state.alpha,
            composite: self.state.composite,
            clips: self.state.clips.clone(),
            clear: false,
        };
        self.record(Command::Draw(Box::new(command)))
    }
    pub fn measure_text(
        &self,
        text: &[u16],
        provider: &mut impl TextProvider,
    ) -> Result<TextMetrics> {
        Ok(provider
            .layout(&self.state.font, text, self.state.direction)?
            .metrics(
                self.state.text_align,
                self.state.text_baseline,
                self.state.direction,
            ))
    }
    pub fn fill_text(
        &mut self,
        text: &[u16],
        x: f64,
        y: f64,
        max_width: Option<f64>,
        provider: &mut impl TextProvider,
    ) -> Result<()> {
        self.draw_text(text, Point::new(x, y), max_width, provider, false)
    }
    pub fn stroke_text(
        &mut self,
        text: &[u16],
        x: f64,
        y: f64,
        max_width: Option<f64>,
        provider: &mut impl TextProvider,
    ) -> Result<()> {
        self.draw_text(text, Point::new(x, y), max_width, provider, true)
    }
    fn draw_text(
        &mut self,
        text: &[u16],
        origin: Point,
        max_width: Option<f64>,
        provider: &mut impl TextProvider,
        stroke: bool,
    ) -> Result<()> {
        if !origin.is_finite() || max_width.is_some_and(|w| !w.is_finite() || w <= 0.) {
            return Ok(());
        }
        let layout = provider.layout(&self.state.font, text, self.state.direction)?;
        let (x, y) = layout.anchor(
            self.state.text_align,
            self.state.text_baseline,
            self.state.direction,
        );
        let scale = max_width.map_or(1., |w| {
            if layout.width > 0. {
                (w / layout.width).min(1.)
            } else {
                1.
            }
        });
        let local = Affine::translate((origin.x, origin.y))
            * Affine::scale_non_uniform(scale, 1.)
            * Affine::translate((-x, y));
        // maxWidth condenses glyphs, not the stroke pen. Stroke after condensation.
        let path = local * layout.path;
        self.draw(
            path,
            if stroke {
                ItemStyle::Stroke(self.state.line.clone())
            } else {
                ItemStyle::Fill(FillRule::NonZero)
            },
            self.state.transform,
            if stroke {
                self.state.stroke.clone()
            } else {
                self.state.fill.clone()
            },
            false,
        )
    }
    pub fn put_image_data(
        &mut self,
        image: ImageData,
        dx: i32,
        dy: i32,
        dirty: Option<[i32; 4]>,
    ) -> Result<()> {
        let dirty = dirty.unwrap_or([0, 0, image.width() as i32, image.height() as i32]);
        self.record(Command::Put {
            image: Arc::new(image),
            dx,
            dy,
            dirty,
        })
    }
    pub fn flush(&mut self) -> Result<()> {
        for command in &self.commands[self.rendered..] {
            self.surface.execute(command)?;
            self.rendered += 1;
        }
        Ok(())
    }
    pub fn get_image_data(&mut self, sx: i32, sy: i32, sw: i32, sh: i32) -> Result<ImageData> {
        if sw == 0 || sh == 0 {
            return Err(Error::IndexSize);
        }
        let (x, y) = (
            i64::from(sx) + i64::from(sw.min(0)),
            i64::from(sy) + i64::from(sh.min(0)),
        );
        let mut data = ImageData::new(sw.unsigned_abs(), sh.unsigned_abs())?;
        self.flush()?;
        self.surface.read_into(x, y, &mut data);
        Ok(data)
    }
    pub fn snapshot(&mut self) -> Result<Source> {
        // ImageData forbids zero size; zero-sized canvases have no drawable source.
        let pixels = Arc::new(self.get_image_data(0, 0, self.width as i32, self.height as i32)?);
        Ok(Source {
            key: self.key(),
            pixels,
        })
    }
    /// Retire consumed commands only; source versions held by other recorders remain alive.
    pub fn compact(&mut self) -> Result<()> {
        self.flush()?;
        self.commands.clear();
        self.rendered = 0;
        Ok(())
    }
    pub fn reset(&mut self) -> Result<()> {
        self.resize(self.width, self.height)
    }
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let surface = CpuRasterizer::new(width, height)?;
        let version = self.version.checked_add(1).ok_or(Error::Capacity)?;
        self.surface = surface;
        self.width = width;
        self.height = height;
        self.version = version;
        self.state = DrawingState::default();
        self.stack.clear();
        self.path.clear();
        self.commands.clear();
        self.rendered = 0;
        self.promoted = None;
        Ok(())
    }
    pub fn promote(&mut self, target: &mut impl PromotionTarget) -> Result<SourceKey> {
        let source = self.snapshot()?;
        let snapshot = PromotionSnapshot {
            key: source.key,
            pixels: source.pixels,
            state: self.state.clone(),
            path: self.path.clone(),
            saved_states: self.stack.clone(),
        };
        target.present(&snapshot)?;
        self.promoted = Some(snapshot.key);
        Ok(snapshot.key)
    }
    /// A cloned canvas retains dimensions but starts with default state and empty pixels.
    pub fn clone_blank(&self, new_id: u64) -> Result<Self> {
        Self::new(new_id, self.width, self.height)
    }
    /// OffscreenCanvas transfer consumes the bitmap while retaining drawing state/path.
    pub fn transfer_to_image_bitmap(&mut self) -> Result<Source> {
        let source = self.snapshot()?;
        let version = self.version.checked_add(1).ok_or(Error::Capacity)?;
        let surface = CpuRasterizer::new(self.width, self.height)?;
        self.surface = surface;
        self.commands.clear();
        self.rendered = 0;
        self.version = version;
        Ok(source)
    }
    /// Commit edits from a promoted backing only if no native writes intervened.
    /// A stale page may never overwrite a newer authored version.
    pub fn import_promoted(&mut self, expected: SourceKey, image: ImageData) -> Result<()> {
        if self.key() != expected || self.promoted != Some(expected) {
            return Err(Error::VersionConflict);
        }
        if (image.width(), image.height()) != self.size() {
            return Err(Error::IndexSize);
        }
        self.put_image_data(image, 0, 0, None)
    }
    pub fn promoted_version(&self) -> Option<SourceKey> {
        self.promoted
    }
}
