//! View-only navigation. Source: v1.2.0 viewer.ts:179–254, 1000–1086,
//! 1566–1583, 1766–1776, 1886–1920. No drawing or layout policy.
use crate::{Error, finite};

pub const ZOOM_PRESETS: &[f64] = &[0.05, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
/// The range navigation reaches. Stored values may sit lower, down to 0.02.
pub const ZOOM_MIN: f64 = 0.05;
pub const ZOOM_MAX: f64 = 8.0;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub fn checked(self) -> Result<Self, Error> {
        finite(self.x)?;
        finite(self.y)?;
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}
impl Size {
    pub fn checked(self) -> Result<Self, Error> {
        if finite(self.width)? <= 0.0 || finite(self.height)? <= 0.0 {
            return Err(Error::Invalid("extent"));
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}
impl Rect {
    pub fn between(a: Point, b: Point) -> Self {
        Self {
            min: Point {
                x: a.x.min(b.x),
                y: a.y.min(b.y),
            },
            max: Point {
                x: a.x.max(b.x),
                y: a.y.max(b.y),
            },
        }
    }
    pub fn width(self) -> f64 {
        self.max.x - self.min.x
    }
    pub fn height(self) -> f64 {
        self.max.y - self.min.y
    }
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
}
/// Host supplies usable geometry and fit inset from approved design tokens.
/// DESIGN-TODO(opus): fit inset and zoom-marquee threshold; no native values chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub stage: Size,
    pub composition: Size,
    pub fit_inset: f64,
}
impl Viewport {
    pub fn checked(self) -> Result<Self, Error> {
        self.stage.checked()?;
        self.composition.checked()?;
        if finite(self.fit_inset)? < 0.0
            || self.fit_inset * 2.0 >= self.stage.width.min(self.stage.height)
        {
            return Err(Error::Invalid("fit inset"));
        }
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Navigation {
    pub fit: bool,
    pub zoom: f64,
    pub pan: Point,
}
impl Default for Navigation {
    fn default() -> Self {
        Self {
            fit: true,
            zoom: 1.0,
            pan: Point::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomChoice {
    Fit,
    Preset(f64),
    Custom(f64),
}
impl Navigation {
    pub fn checked(self) -> Result<Self, Error> {
        if !(0.02..=8.0).contains(&finite(self.zoom)?) {
            return Err(Error::Invalid("zoom"));
        }
        self.pan.checked()?;
        Ok(self)
    }
    pub fn shown(self, viewport: Viewport) -> Result<f64, Error> {
        self.checked()?;
        viewport.checked()?;
        Ok(if self.fit {
            ((viewport.stage.width - 2.0 * viewport.fit_inset) / viewport.composition.width)
                .min(
                    (viewport.stage.height - 2.0 * viewport.fit_inset)
                        / viewport.composition.height,
                )
                .clamp(0.02, 8.0)
        } else {
            self.zoom.clamp(0.02, 8.0)
        })
    }
    pub fn choice(self, viewport: Viewport) -> Result<ZoomChoice, Error> {
        let shown = self.shown(viewport)?;
        Ok(if self.fit {
            ZoomChoice::Fit
        } else if ZOOM_PRESETS.contains(&shown) {
            ZoomChoice::Preset(shown)
        } else {
            ZoomChoice::Custom(shown)
        })
    }
    pub fn origin(self, viewport: Viewport) -> Result<Point, Error> {
        let zoom = self.shown(viewport)?;
        let pan = if self.fit { Point::default() } else { self.pan };
        Point {
            x: (viewport.stage.width - viewport.composition.width * zoom) / 2.0 + pan.x,
            y: (viewport.stage.height - viewport.composition.height * zoom) / 2.0 + pan.y,
        }
        .checked()
    }
    pub fn composition_point(self, viewport: Viewport, pointer: Point) -> Result<Point, Error> {
        pointer.checked()?;
        let origin = self.origin(viewport)?;
        let zoom = self.shown(viewport)?;
        Point {
            x: (pointer.x - origin.x) / zoom,
            y: (pointer.y - origin.y) / zoom,
        }
        .checked()
    }
    pub fn set_zoom(self, zoom: f64) -> Result<Self, Error> {
        finite(zoom)?;
        self.checked()?;
        Ok(Self {
            fit: false,
            zoom: zoom.clamp(ZOOM_MIN, ZOOM_MAX),
            pan: if self.fit { Point::default() } else { self.pan },
        })
    }
    pub fn fit(self) -> Self {
        Self {
            fit: true,
            pan: Point::default(),
            ..self
        }
    }
    pub fn zoom_at(self, viewport: Viewport, pointer: Point, factor: f64) -> Result<Self, Error> {
        if finite(factor)? <= 0.0 {
            return Err(Error::Invalid("zoom factor"));
        }
        let point = self.composition_point(viewport, pointer)?;
        self.place_point(viewport, pointer, point, self.shown(viewport)? * factor)
    }
    fn place_point(
        self,
        viewport: Viewport,
        pointer: Point,
        point: Point,
        zoom: f64,
    ) -> Result<Self, Error> {
        viewport.checked()?;
        pointer.checked()?;
        point.checked()?;
        finite(zoom)?;
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        let pan = Point {
            x: pointer.x
                - viewport.stage.width / 2.0
                - (point.x - viewport.composition.width / 2.0) * zoom,
            y: pointer.y
                - viewport.stage.height / 2.0
                - (point.y - viewport.composition.height / 2.0) * zoom,
        }
        .checked()?;
        Ok(Self {
            fit: false,
            zoom,
            pan,
        })
    }
    pub fn pan_by(self, viewport: Viewport, delta: Point) -> Result<Self, Error> {
        delta.checked()?;
        let shown = self.shown(viewport)?;
        let start = if self.fit { Point::default() } else { self.pan };
        let pan = Point {
            x: start.x + delta.x,
            y: start.y + delta.y,
        }
        .checked()?;
        Ok(Self {
            fit: false,
            zoom: if self.fit { shown } else { self.zoom },
            pan,
        })
    }
    pub fn out_of_view(self, viewport: Viewport, source_active: bool) -> Result<bool, Error> {
        let origin = self.origin(viewport)?;
        let zoom = self.shown(viewport)?;
        Ok(!self.fit
            && !source_active
            && (origin.x + viewport.composition.width * zoom <= 0.0
                || origin.x >= viewport.stage.width
                || origin.y + viewport.composition.height * zoom <= 0.0
                || origin.y >= viewport.stage.height))
    }
    /// Native host classifies continuous scroll, discrete wheel, and magnify.
    /// Chromium wheelDelta heuristics are deliberately not part of this API.
    pub fn scroll(self, viewport: Viewport, pointer: Point, input: Scroll) -> Result<Self, Error> {
        match input {
            Scroll::Pan { delta, shift } => {
                let delta = if shift && delta.x == 0.0 {
                    Point {
                        x: -delta.y,
                        y: 0.0,
                    }
                } else {
                    Point {
                        x: -delta.x,
                        y: -delta.y,
                    }
                };
                self.pan_by(viewport, delta)
            }
            Scroll::ZoomPixels(delta) => self.zoom_at(
                viewport,
                pointer,
                (-finite(delta)? * 0.006).exp().clamp(0.5, 2.0),
            ),
            Scroll::Magnify(factor) => self.zoom_at(viewport, pointer, factor),
        }
    }
    /// `moved` comes from the host's approved drag recognizer. `minimum` is
    /// likewise supplied, not a new layout/interaction choice by this model.
    pub fn zoom_release(
        self,
        viewport: Viewport,
        start: Point,
        end: Point,
        moved: bool,
        alt: bool,
        minimum: f64,
    ) -> Result<Self, Error> {
        start.checked()?;
        end.checked()?;
        if finite(minimum)? < 0.0 {
            return Err(Error::Invalid("marquee minimum"));
        }
        let rect = Rect::between(start, end);
        if !moved || alt || rect.width() < minimum || rect.height() < minimum {
            return self.zoom_at(viewport, end, if alt { 0.8 } else { 1.25 });
        }
        let rect = Rect::between(
            self.composition_point(viewport, start)?,
            self.composition_point(viewport, end)?,
        );
        let zoom = ((viewport.stage.width - 2.0 * viewport.fit_inset) / rect.width().max(0.01))
            .min((viewport.stage.height - 2.0 * viewport.fit_inset) / rect.height().max(0.01));
        self.place_point(
            viewport,
            Point {
                x: viewport.stage.width / 2.0,
                y: viewport.stage.height / 2.0,
            },
            Point {
                x: (rect.min.x + rect.max.x) / 2.0,
                y: (rect.min.y + rect.max.y) / 2.0,
            },
            zoom,
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    Pan { delta: Point, shift: bool },
    ZoomPixels(f64),
    Magnify(f64),
}
/// From-start pan; live Shift doubles speed. Cancellation restores the start
/// only if the caller's state revision still matches (see Controller).
#[derive(Clone, Copy, Debug)]
pub struct PanDrag {
    pub start: Navigation,
}
impl PanDrag {
    pub fn update(
        self,
        viewport: Viewport,
        delta: Point,
        shift: bool,
    ) -> Result<Navigation, Error> {
        let speed = if shift { 2.0 } else { 1.0 };
        self.start.pan_by(
            viewport,
            Point {
                x: delta.x * speed,
                y: delta.y * speed,
            },
        )
    }
}
