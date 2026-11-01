use crate::Rasterizer;
use kurbo::{BezPath, PathEl};
use pm_recorder::{
    Canvas, Color, Composite, Error, FillRule, MAX_PIXELS, Paint, Result, SourceKey,
};

/// Device-space source alpha BEFORE the final canvas clip. Include any source
/// globalAlpha, image alpha, paint alpha and geometric coverage exactly once.
/// The explicit origin preserves off-canvas casters that cast onto the bitmap.
#[derive(Debug, Clone)]
pub struct AlphaMask {
    width: u32,
    height: u32,
    origin: [i32; 2],
    values: Vec<f32>,
}
impl AlphaMask {
    pub fn new(width: u32, height: u32, origin: [i32; 2], values: Vec<f32>) -> Result<Self> {
        let n = u64::from(width) * u64::from(height);
        if n > MAX_PIXELS {
            return Err(Error::Capacity);
        }
        if width == 0 || height == 0 || values.len() as u64 != n {
            return Err(Error::IndexSize);
        }
        if values
            .iter()
            .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err(Error::Range);
        }
        Ok(Self {
            width,
            height,
            origin,
            values,
        })
    }
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn origin(&self) -> [i32; 2] {
        self.origin
    }
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Gaussian sigma = shadowBlur/2 (HTML Canvas). A normalized, symmetric
    /// discrete Gaussian truncated beyond 5 sigma. Omitted mass < 5.8e-7.
    /// No 8-bit intermediate mask; offset filtering is premultiplied alpha.
    pub fn blurred(&self, blur: f64) -> Result<Self> {
        if !blur.is_finite() || blur < 0. {
            return Err(Error::Range);
        }
        if blur == 0. {
            return Ok(self.clone());
        }
        let sigma = blur / 2.;
        if sigma == 0. {
            return Ok(self.clone());
        }
        let radius = (5. * sigma).ceil();
        if radius > i32::MAX as f64 {
            return Err(Error::Capacity);
        }
        let r = radius as u32;
        let w = self
            .width
            .checked_add(r.checked_mul(2).ok_or(Error::Capacity)?)
            .ok_or(Error::Capacity)?;
        let h = self.height.checked_add(r * 2).ok_or(Error::Capacity)?;
        let n = u64::from(w) * u64::from(h);
        // Bound work as well as memory; never silently shrink the blur.
        if n > MAX_PIXELS {
            return Err(Error::Capacity);
        }
        let taps = u64::from(r) * 2 + 1;
        let work = u64::from(w) * u64::from(self.height) * taps + n * taps;
        if n > MAX_PIXELS || work > 100_000_000 {
            return Err(Error::Capacity);
        }
        let ox = i64::from(self.origin[0]) - i64::from(r);
        let oy = i64::from(self.origin[1]) - i64::from(r);
        let origin = [
            i32::try_from(ox).map_err(|_| Error::Capacity)?,
            i32::try_from(oy).map_err(|_| Error::Capacity)?,
        ];
        let mut kernel: Vec<f64> = (0..taps)
            .map(|i| {
                let x = i as f64 - f64::from(r);
                libm::exp(-0.5 * (x / sigma).powi(2))
            })
            .collect();
        let sum: f64 = kernel.iter().sum();
        for k in &mut kernel {
            *k /= sum;
        }
        let mut tmp = vec![0.; w as usize * self.height as usize];
        for y in 0..self.height {
            for x in 0..w {
                let mut v = 0.;
                for (k, weight) in kernel.iter().enumerate() {
                    let sx = i64::from(x) - k as i64;
                    if sx >= 0 && sx < i64::from(self.width) {
                        v += f64::from(self.values[y as usize * self.width as usize + sx as usize])
                            * weight;
                    }
                }
                tmp[y as usize * w as usize + x as usize] = v;
            }
        }
        let mut values = vec![0.; n as usize];
        for y in 0..h {
            for x in 0..w {
                let mut v = 0.;
                for (k, weight) in kernel.iter().enumerate() {
                    let sy = i64::from(y) - k as i64;
                    if sy >= 0 && sy < i64::from(self.height) {
                        v += tmp[sy as usize * w as usize + x as usize] * weight;
                    }
                }
                values[y as usize * w as usize + x as usize] = v as f32;
            }
        }
        Ok(Self {
            width: w,
            height: h,
            origin,
            values,
        })
    }
    fn sample(&self, x: f64, y: f64) -> f32 {
        let x = x - f64::from(self.origin[0]);
        let y = y - f64::from(self.origin[1]);
        if x < -1. || y < -1. || x >= f64::from(self.width) || y >= f64::from(self.height) {
            return 0.;
        }
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = ((x - x.floor()) as f32, (y - y.floor()) as f32);
        let get = |x: i64, y: i64| {
            if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
                0.
            } else {
                self.values[y as usize * self.width as usize + x as usize]
            }
        };
        (get(ix, iy) * (1. - fx) + get(ix + 1, iy) * fx) * (1. - fy)
            + (get(ix, iy + 1) * (1. - fx) + get(ix + 1, iy + 1) * fx) * fy
    }
}
#[derive(Debug, Clone)]
pub struct Clip {
    /// Device-space geometry; the caller captures the construction CTM.
    pub path: BezPath,
    pub rule: FillRule,
}
#[derive(Debug, Clone, Copy)]
pub struct Shadow {
    pub color: Color,
    /// Device-space offset; deliberately unaffected by the current transform.
    pub offset: [f64; 2],
    pub blur: f64,
}
impl Rasterizer {
    /// Composite only the shadow; replay the original command AFTER this call.
    /// Source mask must include off-canvas, pre-clip source coverage. It is not
    /// derivable from the current opaque recorder DrawCommand (integration gate).
    /// Returned version counts one logical effect operation, not internal spans.
    pub fn draw_shadow(
        &mut self,
        expected: SourceKey,
        mask: &AlphaMask,
        shadow: Shadow,
        composite: &str,
        clips: &[Clip],
    ) -> Result<SourceKey> {
        self.check(expected)?;
        let next = self.key.version.checked_add(1).ok_or(Error::Capacity)?;
        let color = shadow.color;
        if shadow.offset.iter().any(|v| !v.is_finite())
            || [color.r, color.g, color.b, color.a]
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err(Error::Range);
        }
        let operation = Composite::parse(composite).ok_or(Error::Syntax)?;
        if clips
            .iter()
            .any(|c| c.path.elements().iter().any(|e| !e.is_finite()))
        {
            return Err(Error::Range);
        }
        let mask = mask.blurred(shadow.blur)?;
        // Zero-sized recorder is a command builder only. Compact retires each
        // consumed span with no extra backing/rasterization; float destination
        // remains in the production CpuRasterizer throughout.
        let mut builder = Canvas::new(0, 0, 0)?;
        builder.set_composite(composite);
        for clip in clips {
            builder.begin_path();
            for el in clip.path.iter() {
                match el {
                    PathEl::MoveTo(p) => builder.move_to(p.x, p.y),
                    PathEl::LineTo(p) => builder.line_to(p.x, p.y),
                    PathEl::QuadTo(c, p) => builder.quadratic_curve_to(c, p),
                    PathEl::CurveTo(a, b, p) => builder.bezier_curve_to(a, b, p),
                    PathEl::ClosePath => builder.close_path(),
                }
            }
            builder.clip(clip.rule);
        }
        let unbounded = matches!(
            operation,
            Composite::Copy
                | Composite::SourceIn
                | Composite::SourceOut
                | Composite::DestinationIn
                | Composite::DestinationAtop
        );
        let bounds = |origin: i32, offset: f64, length: u32, limit: u32| {
            if unbounded {
                (0, limit)
            } else {
                (
                    (f64::from(origin) + offset - 1.)
                        .floor()
                        .clamp(0., f64::from(limit)) as u32,
                    (f64::from(origin) + offset + f64::from(length))
                        .ceil()
                        .clamp(0., f64::from(limit)) as u32,
                )
            }
        };
        let (x0, x1) = bounds(mask.origin[0], shadow.offset[0], mask.width, self.width);
        let (y0, y1) = bounds(mask.origin[1], shadow.offset[1], mask.height, self.height);
        for y in y0..y1 {
            let alpha = |x: u32| {
                mask.sample(
                    f64::from(x) - shadow.offset[0],
                    f64::from(y) - shadow.offset[1],
                ) * color.a
            };
            let mut x = x0;
            while x < x1 {
                let a = alpha(x);
                let mut end = x + 1;
                while end < x1 && alpha(end) == a {
                    end += 1;
                }
                if a != 0. || unbounded {
                    builder.save();
                    // Clip each run independently so unbounded operators apply
                    // transparent source once to every pixel inside the clip.
                    builder.begin_path();
                    builder.rect(f64::from(x), f64::from(y), f64::from(end - x), 1.);
                    builder.clip(FillRule::NonZero);
                    builder.set_fill(Paint::Solid(Color { a, ..color }));
                    builder.fill_rect(f64::from(x), f64::from(y), f64::from(end - x), 1.)?;
                    self.surface.replay(builder.commands())?;
                    builder.compact()?;
                    builder.restore();
                }
                x = end;
            }
        }
        self.key.version = next;
        Ok(self.key)
    }
}
