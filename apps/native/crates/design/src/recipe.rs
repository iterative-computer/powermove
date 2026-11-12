//! Recipes every surface draws from: text styles, edges and elevation.
//! Edges are hairline shadows, never borders: a border changes a control's
//! box and doubles up where two surfaces meet; a 0.5 pt spread ring does neither.
use crate::{Scheme, Tokens};

/// A text style of the scale: size, weight, tracking (pt) and leading (×size).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: f32,
    pub tracking: f32,
    pub leading: f32,
    /// Tabular figures (`tnum`): timecode, counts, numeric fields.
    pub tabular: bool,
}

/// The type scale's roles. One role per job; a surface never picks a size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Text {
    /// Metadata, counts, shortcut hints, section labels (11).
    Caption,
    /// Rows, fields, buttons, menus (12).
    Body,
    /// The same size at medium weight: the name in a row, a button label.
    Label,
    /// Panel and window titles (13, medium).
    Title,
    /// Running prose (13, loose leading).
    Reading,
    /// Sheet and section headings (15, semibold).
    Heading,
    /// A screen's title: Home, Settings (24, semibold, tight).
    Display,
    /// Timecode and numeric readouts (13, medium, tabular).
    Timecode,
}

/// One shadow layer in points. A ring is `blur 0, spread 0.5`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: u32,
    pub alpha: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    pub inset: bool,
}
impl Shadow {
    const fn ring(color: u32, alpha: f32, spread: f32) -> Self {
        Self {
            color,
            alpha,
            y: 0.,
            blur: 0.,
            spread,
            inset: false,
        }
    }
    const fn drop(color: u32, alpha: f32, y: f32, blur: f32) -> Self {
        Self {
            color,
            alpha,
            y,
            blur,
            spread: 0.,
            inset: false,
        }
    }
    /// A hairline of light just inside the top edge.
    const fn highlight(alpha: f32) -> Self {
        Self {
            color: 0xffffff,
            alpha,
            y: 0.5,
            blur: 0.,
            spread: 0.,
            inset: true,
        }
    }
}

/// How far a surface sits above its ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Elevation {
    /// A quiet well or track: a hairline only.
    Well,
    /// The one raised element of a group: the selected segment or tab, a
    /// neutral push button, a card. Fill is `colors.raised`.
    Raised,
    /// A docked panel on the window ground.
    Panel,
    /// Menus, popovers, tooltips, toasts.
    Menu,
    /// Sheets, the palette, dialogs.
    Sheet,
}

impl Tokens {
    pub fn text(&self, role: Text) -> TextStyle {
        let ty = &self.typography;
        let style = |size, weight, tracking, leading| TextStyle {
            size,
            weight,
            tracking,
            leading,
            tabular: false,
        };
        match role {
            Text::Caption => style(ty.minimum, ty.regular_weight, 0., ty.dense_leading),
            Text::Body => style(ty.body, ty.regular_weight, 0., ty.dense_leading),
            Text::Label => style(ty.label, ty.medium_weight, 0., ty.dense_leading),
            Text::Title => style(ty.title, ty.medium_weight, 0., ty.dense_leading),
            Text::Reading => style(ty.reading, ty.regular_weight, 0., ty.reading_leading),
            Text::Heading => style(
                ty.heading,
                ty.semibold_weight,
                ty.heading_tracking,
                ty.dense_leading,
            ),
            Text::Display => style(ty.display, ty.semibold_weight, ty.display_tracking, 1.15),
            Text::Timecode => TextStyle {
                tabular: true,
                ..style(ty.timecode, ty.medium_weight, 0., ty.dense_leading)
            },
        }
    }
    /// The shadow layers of an elevation, outermost last. Increase Contrast
    /// replaces soft rings with a solid `separator` ring at `metrics.border / 2`.
    pub fn elevation(&self, elevation: Elevation) -> Vec<Shadow> {
        let dark = self.scheme == Scheme::Dark;
        let (ink, edge) = (self.colors.text, self.translucency.edge);
        let ring = if self.accessibility.increase_contrast {
            Shadow::ring(self.colors.separator, 1., self.metrics.border / 2.)
        } else if dark {
            // Dark surfaces part from their ground with a dark seam; the light
            // comes from the highlight inside the top edge.
            Shadow::ring(0x000000, 0.42, 0.5)
        } else {
            Shadow::ring(ink, edge, 0.5)
        };
        let shade = if dark { 0x000000 } else { ink };
        match elevation {
            Elevation::Well => vec![Shadow::ring(ink, edge, 0.5)],
            Elevation::Raised if dark => vec![
                Shadow::highlight(0.09),
                ring,
                Shadow::drop(shade, 0.28, 1., 2.),
            ],
            Elevation::Raised => vec![ring, Shadow::drop(shade, 0.07, 1., 2.)],
            Elevation::Panel if dark => vec![Shadow::highlight(0.045), ring],
            Elevation::Panel => vec![ring, Shadow::drop(shade, 0.04, 1., 3.)],
            Elevation::Menu => vec![
                Shadow::ring(ink, if dark { 0.12 } else { 0.1 }, 0.5),
                Shadow::drop(shade, if dark { 0.45 } else { 0.14 }, 8., 24.),
            ],
            Elevation::Sheet => vec![
                Shadow::ring(ink, if dark { 0.12 } else { 0.1 }, 0.5),
                Shadow::drop(shade, if dark { 0.6 } else { 0.2 }, 20., 48.),
            ],
        }
    }
    /// Keyboard focus ring as (colour, width, gap): neutral ink, outside the
    /// control, shown for keyboard focus only. Never the accent.
    pub fn focus_ring(&self) -> (u32, f32, f32) {
        (self.colors.focus, self.metrics.border * 2., 1.)
    }
}
