//! Media-card geometry for a native painter. All measurements are the Opus
//! media-browser recipe; native font metrics drive middle truncation.
use pm_design::{Tokens, panels};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    fn outset(self, n: f64) -> Self {
        Self {
            x: self.x - n,
            y: self.y - n,
            width: self.width + 2. * n,
            height: self.height + 2. * n,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    pub role: &'static str,
    pub bounds: Rect,
    pub radius: f64,
    pub color: u32,
    pub opacity: f64,
    pub stroke: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CardRecipe {
    pub thumbnail: Mark,
    pub badges: Vec<Mark>,
    pub actions: Vec<Mark>,
    pub selection: Option<Mark>,
    pub focus: Option<Mark>,
    pub contrast: Option<Mark>,
    pub name_padding: f64,
    pub badge_inset: f64,
    pub status_dot: Mark,
    pub status_dot_keyline: (u32, f64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Duration,
    Status,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidGeometry,
}
/// Shared panel spec §6 supplies the minimum; native theme overrides may
/// provide a resolved value. The column formula is media-browser §2.
pub fn columns(width: f64, minimum: Option<f64>, t: &Tokens) -> Result<usize, Error> {
    let min = minimum.unwrap_or(t.metrics.thumbnail_min as f64);
    if !width.is_finite() || width < 0. || !min.is_finite() || min <= 0. {
        return Err(Error::InvalidGeometry);
    }
    Ok(
        ((width - t.metrics.inset as f64) / (min + t.metrics.inset as f64))
            .floor()
            .max(1.) as usize,
    )
}
impl CardRecipe {
    /// Badge/action counts are semantic facts, not invented actions. The host
    /// measures capsule text before calling; all rectangles remain inspectable.
    pub fn new(
        origin: (f64, f64),
        width: f64,
        selected: bool,
        focused: bool,
        badge_widths: &[(Badge, f64)],
        action_count: usize,
        t: &Tokens,
    ) -> Result<Self, Error> {
        let (x, y) = origin;
        if ![x, y, width].iter().all(|v| v.is_finite())
            || width <= 0.
            || badge_widths.iter().any(|(_, w)| !w.is_finite() || *w < 0.)
            || action_count > 4
        {
            return Err(Error::InvalidGeometry);
        }
        let m = t.metrics;
        let c = &t.colors;
        let rect = Rect {
            x,
            y,
            width,
            height: width * 9. / 16.,
        };
        let mark = |role, bounds, radius, color, opacity, stroke| Mark {
            role,
            bounds,
            radius,
            color,
            opacity,
            stroke,
        };
        let ring = if t.accessibility.increase_contrast {
            3.
        } else {
            2.
        };
        let inset = m.gap as f64 - 1.;
        // System font metrics own the actual capsule height (minimum text +
        // approved capsule padding); do not choose one here.
        let badges = badge_widths
            .iter()
            .map(|(badge, w)| {
                mark(
                    if *badge == Badge::Duration {
                        "duration"
                    } else {
                        "status"
                    },
                    Rect {
                        x: x + inset,
                        y: if *badge == Badge::Duration {
                            y + inset
                        } else {
                            y + rect.height - inset
                        },
                        width: *w,
                        height: 0.,
                    },
                    m.capsule_radius as f64,
                    c.media_badge,
                    panels::MEDIA_BADGE_ALPHA,
                    0.,
                )
            })
            .collect();
        let actions = (0..action_count)
            .map(|i| {
                mark(
                    "action",
                    Rect {
                        x: x + width - inset - (action_count - i) as f64 * m.hit as f64,
                        y: y + rect.height - inset - m.hit as f64,
                        width: m.hit as f64,
                        height: m.hit as f64,
                    },
                    m.hit as f64 / 2.,
                    c.media_badge,
                    panels::MEDIA_BADGE_ALPHA,
                    0.,
                )
            })
            .collect();
        Ok(Self {
            thumbnail: mark("thumbnail", rect, m.control_radius as f64, c.field, 1., 0.),
            badges,
            actions,
            selection: selected.then(|| {
                mark(
                    "selection",
                    rect.outset(ring / 2.),
                    m.control_radius as f64 + 2.,
                    c.accent,
                    1.,
                    ring,
                )
            }),
            focus: focused.then(|| {
                mark(
                    "focus",
                    rect.outset(if selected { ring + 2. } else { 2. }),
                    m.control_radius as f64 + 2.,
                    c.focus,
                    1.,
                    2.,
                )
            }),
            contrast: t.accessibility.increase_contrast.then(|| {
                mark(
                    "contrast",
                    rect,
                    m.control_radius as f64,
                    c.separator,
                    1.,
                    1.,
                )
            }),
            name_padding: m.gap as f64 - 2.,
            badge_inset: inset,
            status_dot_keyline: (c.media_badge, panels::MEDIA_BADGE_ALPHA),
            status_dot: mark(
                "open-composition",
                Rect {
                    x: x + width - inset - m.status_dot as f64,
                    y: y + inset,
                    width: m.status_dot as f64,
                    height: m.status_dot as f64,
                },
                m.status_dot as f64 / 2.,
                c.accent,
                1.,
                1.,
            ),
        })
    }
    /// Capsule heights/widths are measured by native text, then placed at the
    /// specified corners. A zero-size placeholder never masquerades as paint.
    pub fn measure_badges(&mut self, heights: &[f64]) -> Result<(), Error> {
        if heights.len() != self.badges.len() || heights.iter().any(|h| !h.is_finite() || *h <= 0.)
        {
            return Err(Error::InvalidGeometry);
        }
        for (badge, h) in self.badges.iter_mut().zip(heights) {
            badge.bounds.height = *h;
            if badge.role == "status" {
                badge.bounds.y -= h;
            }
        }
        Ok(())
    }
}
/// Native shaper supplies grapheme boundaries and widths. The extension is
/// retained in full when it fits; a very narrow well returns only the ellipsis.
/// No byte slicing of Unicode and no guessed glyph widths.
pub trait TextMetrics {
    fn width(&self, text: &str) -> f64;
    fn boundaries(&self, text: &str) -> Vec<usize>;
}
pub fn middle_truncate(text: &str, width: f64, metrics: &dyn TextMetrics) -> String {
    if width <= 0. || !width.is_finite() {
        return String::new();
    }
    if metrics.width(text) <= width {
        return text.into();
    }
    let ellipsis = "…";
    if metrics.width(ellipsis) > width {
        return String::new();
    }
    let boundaries = metrics.boundaries(text);
    if boundaries.first() != Some(&0)
        || boundaries.last() != Some(&text.len())
        || boundaries.windows(2).any(|w| w[0] >= w[1])
        || boundaries.iter().any(|i| !text.is_char_boundary(*i))
    {
        return ellipsis.into();
    }
    let extension = text.rfind('.').filter(|i| *i > 0).unwrap_or(text.len());
    let extension_index = boundaries
        .partition_point(|i| *i < extension)
        .min(boundaries.len() - 1);
    let count = boundaries.len() - 1;
    let extension_count = count - extension_index;
    for kept in (extension_count..count).rev() {
        let stem = kept - extension_count;
        let prefix = stem.div_ceil(2);
        let suffix = count - (extension_count + stem / 2);
        let candidate = format!(
            "{}{ellipsis}{}",
            &text[..boundaries[prefix]],
            &text[boundaries[suffix]..]
        );
        if metrics.width(&candidate) <= width {
            return candidate;
        }
    }
    ellipsis.into()
}
