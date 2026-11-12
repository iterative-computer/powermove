//! The timeline's drawn materials: layer and clip strips, the playhead and the
//! trim grip. A strip is a slab of tinted glass on the solid timeline canvas:
//! a saturated body in the layer's own hue, lighter toward the bottom, a 1 px
//! rim of light inside the edge instead of an outline, large corners, and
//! enough translucency that the time grid reads through it. It is drawn by the
//! timeline renderer (hundreds on screen while scrolling), never a system view.
use crate::{
    Scheme, Tokens,
    color::{mix, oklch, to_oklch},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StripState {
    Rest,
    /// The pointer is over the strip: the rim brightens toward white.
    Hover,
    /// Selected: a full white rim and a slightly brighter body.
    Selected,
    /// Being moved or trimmed: lifted (drop shadow) and more translucent.
    Dragging,
    /// Muted, hidden or offline: chroma and opacity drop.
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    /// Body gradient, top to bottom (the lower half is the more luminous).
    pub top: u32,
    pub bottom: u32,
    /// Body opacity over the canvas: the grid under it stays faintly visible.
    pub opacity: f32,
    /// 1 px ring just inside the edge, as (colour, alpha).
    pub rim: (u32, f32),
    /// The ring's top edge, brighter: light catching the slab.
    pub rim_top: (u32, f32),
    /// Drop shadow under a dragged strip as (colour, alpha, y, blur); alpha 0 = none.
    pub shadow: (u32, f32, f32, f32),
    /// Names and glyphs on the strip: a pale tint of its own hue.
    pub foreground: u32,
    /// Waveforms and marks: the hue again, between body and foreground.
    pub primary: u32,
}

/// The playhead: a 1 px line with a round head in the ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playhead {
    pub color: u32,
    /// A soft seam of the canvas colour either side, so the line reads over
    /// strips of any hue, as (colour, alpha).
    pub halo: (u32, f32),
    pub line: f32,
    pub head: f32,
}

/// The trim grip: a short rounded pill inside the hovered strip's near edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrimGrip {
    pub color: u32,
    pub width: f32,
    /// Share of the strip's height.
    pub height: f32,
    /// Distance from the strip's edge to the pill.
    pub inset: f32,
}

impl Tokens {
    /// Corner of a strip `height` points tall: a fifth of its height, never
    /// below the smallest radius of the scale.
    pub fn strip_radius(&self, height: f32) -> f32 {
        (height * 0.22).clamp(self.metrics.inner_radius, self.metrics.panel_radius)
    }
    /// Even space between neighbouring strips, so each reads as its own object.
    pub fn strip_gap(&self) -> f32 {
        self.metrics.border * 2.
    }
    /// The strip material for a layer colour (`colors.labels.*`) in `state`.
    pub fn strip(&self, fill: u32, state: StripState) -> Strip {
        let dark = self.scheme == Scheme::Dark;
        let (_, chroma, hue) = to_oklch(fill);
        // Neutral labels stay neutral; coloured ones are lifted to one chroma.
        let chroma = if chroma < 0.012 {
            0.
        } else if state == StripState::Disabled {
            0.035
        } else {
            0.125
        };
        if self.accessibility.increase_contrast {
            let body = oklch(if dark { 0.42 } else { 0.8 }, chroma * 0.8, hue);
            return Strip {
                top: body,
                bottom: body,
                opacity: 1.,
                rim: (
                    self.colors.separator,
                    if state == StripState::Selected {
                        0.
                    } else {
                        1.
                    },
                ),
                rim_top: (self.colors.text, f32::from(state == StripState::Selected)),
                shadow: (0, 0., 0., 0.),
                foreground: self.colors.text,
                primary: self.colors.text,
            };
        }
        // Lightness of the body's top and bottom, the labels and the marks.
        let (top, bottom, label, mark) = if dark {
            (0.4, 0.5, 0.94, 0.8)
        } else {
            (0.78, 0.86, 0.3, 0.46)
        };
        let lift = if state == StripState::Selected {
            if dark { 0.035 } else { -0.03 }
        } else {
            0.
        };
        let rim = oklch(if dark { 0.9 } else { 0.99 }, chroma * 0.5, hue);
        let (rim_alpha, top_alpha) = match state {
            StripState::Rest => (0.16, 0.34),
            StripState::Hover => (0.4, 0.62),
            StripState::Selected => (0.9, 1.),
            StripState::Dragging => (0.55, 0.8),
            StripState::Disabled => (0.1, 0.18),
        };
        let white = matches!(
            state,
            StripState::Hover | StripState::Selected | StripState::Dragging
        );
        let rim = if white { mix(rim, 0xffffff, 0.7) } else { rim };
        Strip {
            top: oklch(top + lift, chroma, hue),
            bottom: oklch(bottom + lift, chroma * 1.08, hue),
            opacity: match state {
                StripState::Dragging => 0.78,
                StripState::Disabled => 0.5,
                _ => 0.9,
            },
            rim: (rim, rim_alpha),
            rim_top: (rim, top_alpha),
            shadow: if state == StripState::Dragging {
                (0x000000, if dark { 0.45 } else { 0.18 }, 6., 16.)
            } else {
                (0, 0., 0., 0.)
            },
            foreground: oklch(label, chroma * if dark { 0.35 } else { 0.6 }, hue),
            primary: oklch(mark, chroma * if dark { 0.7 } else { 0.9 }, hue),
        }
    }
    pub fn playhead(&self) -> Playhead {
        Playhead {
            color: self.colors.text,
            halo: (self.colors.panel, 0.6),
            line: self.metrics.border,
            head: 9.,
        }
    }
    pub fn trim_grip(&self) -> TrimGrip {
        TrimGrip {
            color: 0xffffff,
            width: 3.,
            height: 0.6,
            inset: 4.,
        }
    }
}
