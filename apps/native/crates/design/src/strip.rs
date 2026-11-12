//! The timeline's drawn materials: layer and clip strips, the playhead and the
//! trim grip. A strip is a slab of tinted glass on the solid timeline canvas:
//! a deep, saturated top that opens into a luminous lower half, a 1 px rim of
//! light in its own hue (brightest along the top edge and round the corners)
//! instead of an outline, continuous corners about a quarter of its height,
//! and the ruler's grid reading through the body. Hover and selection only
//! brighten and whiten the rim. The same slab is used in both appearances: a
//! clip is an object with its own colour, not a tint of the panel. It is
//! drawn by the timeline renderer (hundreds on screen while scrolling), never
//! a system view.
use crate::{
    Scheme, Tokens,
    color::{mix, oklch, to_oklch},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StripState {
    Rest,
    /// The pointer is over the strip: the rim brightens toward white.
    Hover,
    /// Selected: a full white rim.
    Selected,
    /// Being moved or trimmed: lifted (drop shadow) and more translucent.
    Dragging,
    /// Muted, hidden or offline: chroma and opacity drop.
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    /// Body gradient, top to bottom: deep and saturated, then luminous.
    pub top: u32,
    pub bottom: u32,
    /// Body opacity over the canvas.
    pub opacity: f32,
    /// Light gathering in the lower half, as (colour, alpha at the bottom
    /// edge); it fades to nothing at mid height.
    pub glow: (u32, f32),
    /// 1 px ring just inside the edge, as (colour, alpha).
    pub rim: (u32, f32),
    /// The ring along the top edge and round the top corners: brighter.
    pub rim_top: (u32, f32),
    /// Width of the ring in points: 1, or 2 on a selected strip so it reads
    /// at a glance in a dense timeline.
    pub rim_width: f32,
    /// The ruler's grid where it passes through the body, as (colour, alpha).
    pub grid: (u32, f32),
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
    /// Corner of a strip `height` points tall: about a quarter of its height.
    pub fn strip_radius(&self, height: f32) -> f32 {
        (height * 0.27).clamp(self.metrics.inner_radius, self.metrics.panel_radius)
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
        let coloured = chroma >= 0.012;
        let chroma = match (coloured, state) {
            (false, _) => 0.,
            (true, StripState::Disabled) => 0.04,
            (true, _) => 0.15,
        };
        if self.accessibility.increase_contrast {
            let body = oklch(0.42, chroma * 0.7, hue);
            let selected = state == StripState::Selected;
            return Strip {
                top: body,
                bottom: body,
                opacity: 1.,
                glow: (0, 0.),
                rim: (0xffffff, if selected { 1. } else { 0.6 }),
                rim_top: (0xffffff, if selected { 1. } else { 0.6 }),
                rim_width: if selected { 2. } else { 1. },
                grid: (0xffffff, 0.),
                shadow: (0, 0., 0., 0.),
                foreground: 0xffffff,
                primary: 0xffffff,
            };
        }
        // A light canvas shows through the body, so the slab is a step deeper
        // there to keep its labels at 4.5:1.
        let (top, bottom, opacity, faded) = if dark {
            (0.33, 0.52, 0.9, 0.55)
        } else {
            (0.3, 0.45, 0.94, 0.85)
        };
        let light = oklch(0.9, chroma * 0.55, hue);
        let (rim, side, edge) = match state {
            StripState::Rest => (light, 0.2, 0.5),
            StripState::Hover => (mix(light, 0xffffff, 0.6), 0.5, 0.85),
            StripState::Selected => (0xffffff, 0.92, 1.),
            StripState::Dragging => (mix(light, 0xffffff, 0.6), 0.6, 0.9),
            StripState::Disabled => (light, 0.1, 0.2),
        };
        let disabled = state == StripState::Disabled;
        let selected = state == StripState::Selected;
        // A selected strip is a step brighter as well as white-rimmed.
        let (top, bottom) = if selected {
            (top + 0.03, bottom + 0.02)
        } else {
            (top, bottom)
        };
        let body = oklch(bottom, chroma * 0.92, hue);
        let opacity_now = match state {
            StripState::Dragging if dark => opacity - 0.06,
            StripState::Disabled => faded,
            _ => opacity,
        };
        // Labels are the palest tint of the strip's own hue that still holds
        // 4.5:1 on the body as it lands on the canvas: as much hue as the
        // contrast floor allows, per colour.
        let landed = mix(self.colors.panel, body, opacity_now);
        let foreground = (0..=14)
            .map(|step| oklch(0.86 + step as f32 * 0.01, chroma * 0.45, hue))
            .find(|label| crate::color::contrast(*label, landed) >= 4.6)
            .unwrap_or(0xffffff);
        Strip {
            // More chroma at the top, where the slab is deepest.
            top: oklch(top, chroma, hue),
            bottom: body,
            opacity: opacity_now,
            glow: (
                oklch(0.82, chroma * 0.8, hue),
                if disabled { 0. } else { 0.16 },
            ),
            rim: (rim, side),
            rim_top: (rim, edge),
            rim_width: if selected { 2. } else { 1. },
            grid: (light, if disabled { 0.05 } else { 0.13 }),
            shadow: if state == StripState::Dragging {
                (0x000000, if dark { 0.45 } else { 0.22 }, 6., 16.)
            } else {
                (0, 0., 0., 0.)
            },
            foreground,
            primary: oklch(0.8, chroma * 0.62, hue),
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
