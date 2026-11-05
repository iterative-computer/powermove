//! Evaluated engine geometry in logical display points, never re-laid-out text.
use super::Accessibility;
use pm_design::{Scheme, Tokens};
use std::collections::BTreeSet;

pub const LOCKED: &str = "Layer is locked";
pub const OVERSET: &str = "More text than fits";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub fn plus(self, p: Self) -> Self {
        Self {
            x: self.x + p.x,
            y: self.y + p.y,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Vertex {
    pub id: String,
    /// Position in authored path pixels, used in VoiceOver (not display pixels).
    pub position: Point,
    pub incoming: Point,
    pub outgoing: Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Select,
    Draw,
    Add,
    Delete,
    Close,
    Convert,
}
impl Cursor {
    /// None means the system arrow. Pen template images use their tip as hotspot.
    pub fn symbol(self) -> Option<&'static str> {
        match self {
            Self::Select => None,
            Self::Draw => Some("pencil.tip"),
            Self::Add => Some("pencil.tip.crop.circle.badge.plus"),
            Self::Delete => Some("pencil.tip.crop.circle.badge.minus"),
            Self::Close => Some("pencil.tip.crop.circle.badge.arrow.forward"),
            Self::Convert => Some("arrow.triangle.branch"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VertexPlan {
    pub vertex: Vertex,
    pub side: f32,
    pub fill: u32,
    pub stroke: u32,
    pub stroke_width: f32,
    pub display_position: Point,
    pub display_incoming: Point,
    pub display_outgoing: Point,
    pub show_tangents: bool,
    pub close_ring: Option<f32>,
    pub ax: Accessibility,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PathPlan {
    pub path_id: String,
    pub closed: bool,
    pub scheme: Scheme,
    pub stroke: u32,
    pub stroke_width: f32,
    pub vertices: Vec<VertexPlan>,
    pub tangent_stroke: u32,
    pub tangent_fill: u32,
    pub tangent_width: f32,
    pub tangent_diameter: f32,
    pub rubber_band: Option<(Point, Point)>,
    pub rubber_band_colour: u32,
    pub rubber_band_dash: [f32; 2],
    pub banner: Option<&'static str>,
    pub banner_seconds: f64,
    pub announcement_priority: &'static str,
}
#[derive(Clone, Debug, Default)]
pub struct PathState {
    pub id: String,
    pub vertices: Vec<Vertex>,
    pub selected: BTreeSet<String>,
    pub hovered: Option<String>,
    pub dragging: Option<String>,
    pub closed: bool,
    pub drawing: bool,
    pub pointer: Option<Point>,
    pub can_close: bool,
    pub locked: bool,
    pub controls_visible: bool,
    /// Already resolved from the mask's label token, never a vertex-type colour.
    pub mask_label_colour: Option<u32>,
    /// Combined engine layer/path-to-display transform, including zoom.
    /// None is identity; the view never recomputes engine transforms.
    pub to_display: Option<[f64; 6]>,
}
impl PathState {
    pub fn plan(&self, tokens: &Tokens) -> PathPlan {
        let dark = if tokens.scheme == Scheme::Dark {
            tokens.clone()
        } else {
            Tokens::new(Scheme::Dark, tokens.accessibility)
        };
        let c = &dark.colors;
        let m = &tokens.metrics;
        let mut plan = PathPlan {
            path_id: self.id.clone(),
            closed: self.closed,
            scheme: Scheme::Dark,
            stroke: self.mask_label_colour.unwrap_or(c.accent),
            stroke_width: if tokens.accessibility.increase_contrast {
                2.0
            } else {
                1.0
            },
            vertices: vec![],
            tangent_stroke: c.secondary,
            tangent_fill: c.floating,
            tangent_width: m.gizmo_keyline,
            tangent_diameter: m.tangent_end,
            rubber_band: None,
            rubber_band_colour: c.accent,
            rubber_band_dash: [4.0, 3.0],
            banner: self.locked.then_some(LOCKED),
            banner_seconds: 4.0,
            announcement_priority: "medium",
        };
        if self.locked || !self.controls_visible {
            return plan;
        }
        let count = self.vertices.len();
        for (i, vertex) in self.vertices.iter().enumerate() {
            let selected = self.selected.contains(&vertex.id);
            let neighbour = (i > 0 && self.selected.contains(&self.vertices[i - 1].id))
                || (i + 1 < count && self.selected.contains(&self.vertices[i + 1].id))
                || (self.closed
                    && count > 1
                    && ((i == 0 && self.selected.contains(&self.vertices[count - 1].id))
                        || (i == count - 1 && self.selected.contains(&self.vertices[0].id))));
            plan.vertices.push(VertexPlan {
                vertex: vertex.clone(),
                side: m.gizmo_handle
                    + if self.hovered.as_ref() == Some(&vertex.id) {
                        2.0
                    } else {
                        0.0
                    },
                fill: if selected { c.accent } else { c.floating },
                stroke: if selected { c.selected_text } else { c.accent },
                stroke_width: m.gizmo_keyline,
                display_position: self.display(vertex.position),
                display_incoming: self.display(vertex.position.plus(vertex.incoming)),
                display_outgoing: self.display(vertex.position.plus(vertex.outgoing)),
                show_tangents: selected || neighbour,
                close_ring: (i == 0 && self.drawing && self.can_close).then_some(m.keyframe),
                ax: Accessibility {
                    id: format!("path/{}/vertex/{}", self.id, vertex.id),
                    role: "AXButton",
                    label: format!(
                        "Point {} of {}, {}, {} pixels",
                        i + 1,
                        count,
                        vertex.position.x,
                        vertex.position.y
                    ),
                    selected: Some(selected),
                    enabled: true,
                },
            });
        }
        if self.drawing && !self.closed {
            plan.rubber_band = self
                .vertices
                .last()
                .zip(self.pointer)
                .map(|(v, p)| (self.display(v.position), self.display(p)));
        }
        plan
    }
    fn display(&self, p: Point) -> Point {
        let m = self.to_display.unwrap_or([1., 0., 0., 1., 0., 0.]);
        Point {
            x: m[0] * p.x + m[2] * p.y + m[4],
            y: m[1] * p.x + m[3] * p.y + m[5],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Vertex,
    Incoming,
    Outgoing,
    New,
}
/// Tagged path-editing.ts's from-start drag. Option breaks the opposite handle;
/// an Option vertex drag edits the outgoing tangent. Shift adds no new snapping.
pub fn drag(start: &Vertex, handle: Handle, delta: Point, option: bool) -> Vertex {
    let mut next = start.clone();
    if handle == Handle::Vertex && !option {
        next.position = start.position.plus(delta);
    } else {
        let (original, changed, opposite) = if handle == Handle::Incoming {
            (start.incoming, &mut next.incoming, &mut next.outgoing)
        } else {
            (start.outgoing, &mut next.outgoing, &mut next.incoming)
        };
        *changed = original.plus(delta);
        if !option {
            *opposite = Point {
                x: -changed.x,
                y: -changed.y,
            };
        }
    }
    next
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitAction {
    Close,
    Delete,
    Drag,
}
pub struct Hit {
    pub index: usize,
    pub count: usize,
    pub active: bool,
    pub point: bool,
    pub clicks: u8,
    pub option: bool,
    pub command: bool,
}
pub fn hit_action(hit: Hit) -> HitAction {
    if hit.clicks > 1
        || (hit.index == 0
            && hit.point
            && hit.count >= 3
            && hit.active
            && !hit.option
            && !hit.command)
    {
        HitAction::Close
    } else if hit.command {
        HitAction::Delete
    } else {
        HitAction::Drag
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextOverlayPlan {
    pub editor: Accessibility,
    pub bounds_stroke: u32,
    pub bounds_width: f32,
    pub dashed_bounds: bool,
    pub selection_colour: u32,
    pub selection_alpha: f32,
    pub system_caret: bool,
    pub system_marked_text: bool,
    pub overset: Option<(f32, u32, &'static str)>,
}
pub fn text_overlay(layer: &str, name: &str, overset: bool, tokens: &Tokens) -> TextOverlayPlan {
    let dark = if tokens.scheme == Scheme::Dark {
        tokens.clone()
    } else {
        Tokens::new(Scheme::Dark, tokens.accessibility)
    };
    TextOverlayPlan {
        editor: Accessibility {
            id: format!("text/{layer}"),
            role: "AXTextArea",
            label: name.into(),
            selected: None,
            enabled: true,
        },
        bounds_stroke: dark.colors.secondary,
        bounds_width: 1.0,
        dashed_bounds: true,
        selection_colour: dark.colors.accent,
        selection_alpha: 0.3,
        system_caret: true,
        system_marked_text: true,
        overset: overset.then_some((tokens.metrics.keyframe, dark.colors.warning, OVERSET)),
    }
}
