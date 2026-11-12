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
    /// The body where it is lightest under a name: at the foot of the
    /// name's descenders. Labels hold their contrast against this.
    pub under_label: u32,
    /// Body opacity over the canvas.
    pub opacity: f32,
    /// Light gathering in the foot, as (colour, alpha at the bottom edge);
    /// it fades to nothing `GLOW_FROM` of the way down, below a name.
    pub glow: (u32, f32),
    /// 1 px ring just inside the edge, as (colour, alpha).
    pub rim: (u32, f32),
    /// The ring along the top edge and round the top corners: brighter.
    pub rim_top: (u32, f32),
    /// Width of the ring in points: 1, or 2 on a selected strip so it reads
    /// at a glance in a dense timeline.
    pub rim_width: f32,
    /// How far inside the edge the ring runs. A selected strip on a light
    /// canvas keeps a band of its own colour outside the white ring, which
    /// would otherwise vanish into the canvas.
    pub rim_inset: f32,
    /// The ruler's grid where it passes through the body, as (colour, alpha
    /// at the strip's top edge): a top-lit line that fades to nothing
    /// `GRID_FADE` of the way down.
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

/// How far down a strip its name's descenders reach, and where the glow of
/// the lower half begins (both as a share of the strip's height).
const LABEL_FOOT: f32 = 0.78;
pub const GLOW_FROM: f32 = 0.6;
/// The most light the foot of a strip gathers (the glow's alpha at its bottom edge).
const GLOW_MOST: f32 = 0.46;
/// How far down a strip the ruler's grid line reaches before it has faded out.
pub const GRID_FADE: f32 = 0.55;

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
                under_label: body,
                opacity: 1.,
                glow: (0, 0.),
                rim: (0xffffff, if selected { 1. } else { 0.6 }),
                rim_top: (0xffffff, if selected { 1. } else { 0.6 }),
                rim_width: if selected { 2. } else { 1. },
                rim_inset: if selected && !dark { 2. } else { 0. },
                grid: (0xffffff, 0.),
                shadow: (0, 0., 0., 0.),
                foreground: 0xffffff,
                primary: 0xffffff,
            };
        }
        // A light canvas shows through the body, so the slab is a step deeper
        // there to keep its labels at 4.5:1.
        let (top, bottom, opacity, faded) = if dark {
            (0.39, 0.52, 0.92, 0.55)
        } else {
            (0.33, 0.46, 0.95, 0.85)
        };
        let light = oklch(0.9, chroma * 0.55, hue);
        let (rim, side, edge) = match state {
            StripState::Rest => (light, 0.13, 0.5),
            StripState::Hover => (mix(light, 0xffffff, 0.6), 0.5, 0.85),
            StripState::Selected => (0xffffff, 0.92, 1.),
            StripState::Dragging => (mix(light, 0xffffff, 0.6), 0.6, 0.9),
            StripState::Disabled => (light, 0.07, 0.2),
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
        let deep = oklch(top, chroma, hue);
        // The light gathers below the name, so the foot can be luminous
        // while the band a name stands on stays deep.
        let glow = oklch(0.84, chroma * 0.8, hue);
        let opacity_now = match state {
            StripState::Dragging if dark => opacity - 0.06,
            StripState::Disabled => faded,
            _ => opacity,
        };
        // Labels are the palest tint of the strip's own hue that still holds
        // 4.5:1 on the body as it lands on the canvas: as much hue as the
        // contrast floor allows, per colour. A name stands on the deeper
        // upper band, so the floor is held at its foot (`LABEL_FOOT` of the
        // way down), where the body under it is lightest.
        let under = |glow_alpha: f32| {
            mix(
                mix(deep, body, LABEL_FOOT),
                glow,
                glow_alpha * ((LABEL_FOOT - GLOW_FROM) / (1. - GLOW_FROM)).max(0.),
            )
        };
        // The foot is as luminous as the hue allows: the glow steps down
        // from `GLOW_MOST` until a label can still hold its contrast on the
        // body under a name (a pale layer colour takes less).
        let glow_alpha = if disabled {
            0.
        } else {
            (0..=12)
                .map(|step| GLOW_MOST - step as f32 * 0.02)
                .find(|alpha| {
                    let landed = mix(self.colors.panel, under(*alpha), opacity_now);
                    crate::color::contrast(0xffffff, landed) >= 4.6
                })
                .unwrap_or(GLOW_MOST - 0.24)
        };
        let under_label = under(glow_alpha);
        let landed = mix(self.colors.panel, under_label, opacity_now);
        let foreground = (0..=14)
            .map(|step| oklch(0.86 + step as f32 * 0.01, chroma * 0.7, hue))
            .find(|label| crate::color::contrast(*label, landed) >= 4.6)
            .unwrap_or(0xffffff);
        Strip {
            // More chroma at the top, where the slab is deepest.
            top: deep,
            bottom: body,
            under_label,
            opacity: opacity_now,
            glow: (glow, glow_alpha),
            rim: (rim, side),
            rim_top: (rim, edge),
            rim_width: if selected { 2. } else { 1. },
            rim_inset: if selected && !dark { 2. } else { 0. },
            grid: (light, if disabled { 0.1 } else { 0.34 }),
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
