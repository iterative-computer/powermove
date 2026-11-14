//! The timeline's drawn materials: layer and clip strips, the playhead and the
//! trim grip. A strip is a slab of tinted glass on the solid timeline canvas,
//! derived from the layer's hue for the appearance it stands in.
//!
//! On the dark canvas (`Tokens::strip`): a deep, saturated top that opens
//! smoothly into a luminous foot, a 1 px rim of light in its own hue inside
//! the edge instead of an outline, light falling on the top edge that thins
//! out round the top corners (brightest along the top and corners, never a
//! band with a visible end), and pale names. Hover and selection brighten
//! and whiten the rim.
//!
//! On the light canvas (`Tokens::strip_light`): a pale tint of the hue, a
//! step deeper at the top and lighter in the foot, a 1 px rim in a deeper
//! shade of the hue with white light along the top, and names in a dark
//! shade of the hue. Hover deepens the rim; selection is a 2 px ring in the
//! hue's deepest shade on a body a step deeper. A dark slab on a near-white
//! canvas reads as a weight, not as glass, so nothing here is shared with
//! the dark recipe but the hue.
//!
//! Both have continuous corners about a quarter of the strip's height and
//! the ruler's grid reading through the body. Strips are drawn by the
//! timeline renderer (hundreds on screen while scrolling), never a system
//! view.
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
    /// Body gradient, top to bottom: deep and saturated, then luminous. One
    /// smooth ramp over the whole height: no band starts or stops inside it.
    pub top: u32,
    pub bottom: u32,
    /// The body where a name holds the least contrast: on the dark canvas
    /// its lightest part under a name (the foot of the descenders), on the
    /// light canvas its deepest (the top). Labels hold their contrast here.
    pub under_label: u32,
    /// Body opacity over the canvas: 1 (a solid slab, the canvas's grid
    /// does not run through it) except a dragged strip, which is lifted.
    pub opacity: f32,
    /// The rim: a ring of light just inside the edge, even all round, as
    /// (colour, alpha), `ring_width` points wide.
    pub ring: (u32, f32),
    pub ring_width: f32,
    /// Light falling on the top edge, as (colour, alpha): a line inside the
    /// top edge that thins out round the top corners and is gone by the
    /// straight sides, so the rim is brightest along the top and corners
    /// with nothing cut off.
    pub light: (u32, f32),
    /// The ruler's grid where it passes through the body, as (colour, alpha
    /// at the strip's top edge): a top-lit line that fades to nothing
    /// `GRID_FADE` of the way down.
    pub grid: (u32, f32),
    /// Drop shadow under a dragged strip as (colour, alpha, y, blur); alpha 0 = none.
    pub shadow: (u32, f32, f32, f32),
    /// Names and glyphs on the strip, in its own hue: a pale tint on the
    /// dark canvas, a dark shade on the light one.
    pub foreground: u32,
    /// Waveforms and marks: the hue again, between body and foreground.
    pub primary: u32,
}

/// The playhead, as v1 (Electron) draws it: a 1 px accent line under an
/// accent pin (flat top, pointed foot) resting on the foot of the ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playhead {
    pub color: u32,
    /// A soft seam of the canvas colour either side, so the line reads over
    /// strips of any hue, as (colour, alpha).
    pub halo: (u32, f32),
    pub line: f32,
    /// The pin's width and height (pt).
    pub head: f32,
    /// The gap between the pin's point and the foot of the ruler (pt).
    pub head_foot: f32,
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

/// How far down a strip its name's descenders reach (a share of its height).
const LABEL_FOOT: f32 = 0.78;
/// How far down a strip the ruler's grid line reaches before it has faded
/// out: about half its height (the owner's reference).
pub const GRID_FADE: f32 = 0.5;

/// The light canvas's strip, in OKLCH lightness unless said otherwise.
struct LightStrip {
    /// The body: its top, a step deeper, and its foot.
    top: f32,
    bottom: f32,
    /// The hue's chroma in the body (moderate: a tint, not a paint).
    chroma: f32,
    /// How much of the canvas a disabled body covers.
    faded: f32,
    /// The rim's shade, the selected ring's, a name's lightest shade, and
    /// the shade of waveforms and marks.
    rim: f32,
    ring: f32,
    label: f32,
    mark: f32,
    /// How much deeper a selected body is.
    selected_step: f32,
}
const LIGHT: LightStrip = LightStrip {
    top: 0.85,
    bottom: 0.915,
    chroma: 0.09,
    faded: 0.5,
    rim: 0.6,
    ring: 0.42,
    label: 0.38,
    mark: 0.54,
    selected_step: 0.045,
};

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
        let selected = state == StripState::Selected;
        if self.accessibility.increase_contrast {
            let (body, ink) = if dark {
                (oklch(0.42, chroma * 0.7, hue), 0xffffff)
            } else {
                // Increase Contrast on the light canvas: a flat pale body
                // outlined and named in the ink.
                (oklch(0.88, chroma * 0.5, hue), self.colors.text)
            };
            let edge = if selected {
                1.
            } else if dark {
                0.6
            } else {
                0.7
            };
            return Strip {
                top: body,
                bottom: body,
                under_label: body,
                opacity: 1.,
                ring: (ink, edge),
                ring_width: if selected { 2. } else { 1. },
                light: (ink, 0.),
                grid: (ink, 0.),
                shadow: (0, 0., 0., 0.),
                foreground: ink,
                primary: ink,
            };
        }
        if !dark {
            return self.strip_light(hue, coloured, state);
        }
        let disabled = state == StripState::Disabled;
        let opacity = match state {
            StripState::Dragging => 0.88,
            StripState::Disabled => 0.55,
            _ => 1.,
        };
        // A selected strip is a step brighter as well as white-rimmed.
        let lift = if selected { 0.03 } else { 0. };
        let deep = oklch(0.4 + lift, chroma, hue);
        // The foot is as luminous as the hue allows while a name, which
        // stands on the deeper upper band, still holds 4.5:1 at the foot of
        // its descenders (`LABEL_FOOT` of the way down).
        let under = |bottom: u32| mix(deep, bottom, LABEL_FOOT);
        let landed = |c: u32| mix(self.colors.panel, c, opacity);
        let body = (0..=16)
            .map(|step| oklch(0.6 + lift - step as f32 * 0.01, chroma * 0.9, hue))
            .find(|bottom| crate::color::contrast(0xffffff, landed(under(*bottom))) >= 4.7)
            .unwrap_or_else(|| oklch(0.44 + lift, chroma * 0.9, hue));
        let under_label = under(body);
        let foreground = (0..=14)
            .map(|step| oklch(0.86 + step as f32 * 0.01, chroma * 0.7, hue))
            .find(|label| crate::color::contrast(*label, landed(under_label)) >= 4.6)
            .unwrap_or(0xffffff);
        // The rim's light is the palest tint of the hue; under the pointer
        // it brightens toward white, selected it is white.
        let tint = oklch(0.9, chroma * 0.55, hue);
        let whiter = mix(tint, 0xffffff, 0.6);
        let (ring, light) = match state {
            StripState::Rest => ((tint, 0.16), (tint, 0.5)),
            StripState::Hover => ((whiter, 0.42), (whiter, 0.8)),
            StripState::Selected => ((0xffffff, 0.92), (0xffffff, 0.)),
            StripState::Dragging => ((whiter, 0.42), (whiter, 0.8)),
            StripState::Disabled => ((tint, 0.08), (tint, 0.2)),
        };
        // A disabled strip is the faded slab as it lands on the canvas,
        // still solid: the grid fades inside it as in any other.
        let solid = |c: u32| if disabled { landed(c) } else { c };
        Strip {
            // More chroma at the top, where the slab is deepest.
            top: solid(deep),
            bottom: solid(body),
            under_label: solid(under_label),
            opacity: if disabled { 1. } else { opacity },
            ring,
            ring_width: if selected { 2. } else { 1. },
            light,
            grid: (tint, if disabled { 0.1 } else { 0.3 }),
            shadow: if state == StripState::Dragging {
                (0x000000, 0.45, 6., 16.)
            } else {
                (0, 0., 0., 0.)
            },
            foreground,
            // Marks (waveforms) stand on the foot, the body's lightest part:
            // the palest step of the hue that still reads there.
            primary: (0..=12)
                .map(|step| oklch(0.8 + step as f32 * 0.01, chroma * 0.62, hue))
                .find(|mark| crate::color::contrast(*mark, solid(body)) >= 2.5)
                .unwrap_or(0xffffff),
        }
    }
    /// The strip on the light canvas (see the module's note).
    fn strip_light(&self, hue: f32, coloured: bool, state: StripState) -> Strip {
        let l = &LIGHT;
        let disabled = state == StripState::Disabled;
        let selected = state == StripState::Selected;
        let chroma = match (coloured, disabled) {
            (false, _) => 0.,
            (true, true) => l.chroma * 0.3,
            (true, false) => l.chroma,
        };
        // A selected strip is a step deeper and fuller as well as ringed.
        let (deeper, fuller) = if selected {
            (l.selected_step, 1.2)
        } else {
            (0., 1.)
        };
        let deep = oklch(l.top - deeper, chroma * fuller, hue);
        let body = oklch(l.bottom - deeper, chroma * fuller * 0.85, hue);
        let opacity = match state {
            StripState::Dragging => 0.88,
            StripState::Disabled => l.faded,
            _ => 1.,
        };
        // The rim: a deeper shade of the hue round the slab, white light
        // along its top. A neutral strip takes the ink's own grey.
        let edge = oklch(l.rim, chroma * 1.3, hue);
        let ring = oklch(l.ring, (chroma * 1.6).min(0.17), hue);
        let (ring, light) = match state {
            StripState::Rest => ((edge, 0.36), (0xffffff, 0.85)),
            StripState::Hover => ((edge, 0.68), (0xffffff, 1.)),
            StripState::Selected => ((ring, 1.), (0xffffff, 0.)),
            StripState::Dragging => ((edge, 0.6), (0xffffff, 1.)),
            StripState::Disabled => ((edge, 0.16), (0xffffff, 0.4)),
        };
        // Names are the hue's dark shade, as light as still holds 4.5:1 on
        // the deepest part of the body as it lands on the canvas.
        let landed = mix(self.colors.panel, deep, opacity);
        let foreground = (0..=12)
            .map(|step| oklch(l.label - step as f32 * 0.02, (chroma * 1.3).min(0.14), hue))
            .find(|label| crate::color::contrast(*label, landed) >= 4.6)
            .unwrap_or(self.colors.text);
        let solid = |c: u32| {
            if disabled {
                mix(self.colors.panel, c, opacity)
            } else {
                c
            }
        };
        Strip {
            top: solid(deep),
            bottom: solid(body),
            under_label: solid(deep),
            opacity: if disabled { 1. } else { opacity },
            ring,
            ring_width: if selected { 2. } else { 1. },
            light,
            grid: (edge, if disabled { 0.1 } else { 0.3 }),
            shadow: if state == StripState::Dragging {
                (0x000000, 0.22, 6., 16.)
            } else {
                (0, 0., 0., 0.)
            },
            foreground,
            primary: oklch(l.mark, (chroma * 1.4).min(0.15), hue),
        }
    }
    pub fn playhead(&self) -> Playhead {
        Playhead {
            // Powermove orange, as v1: the ruler's one bright mark.
            color: self.colors.accent,
            halo: (self.colors.panel, 0.6),
            line: self.metrics.border,
            head: 11.,
            head_foot: 1.,
        }
    }
    pub fn trim_grip(&self) -> TrimGrip {
        TrimGrip {
            // White on the dark canvas's deep strips, the ink on the light
            // canvas's pale ones.
            color: if self.scheme == Scheme::Dark {
                0xffffff
            } else {
                self.colors.text
            },
            width: 3.,
            height: 0.6,
            inset: 4.,
        }
    }
}
