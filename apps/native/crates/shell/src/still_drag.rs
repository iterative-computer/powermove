use super::{Editor, Gesture, TAB_SPACING, positioned, tint, title_width};
use crate::{
    drag,
    kit::Recipes as _,
    layout::{DropTarget, Geometry, Rect},
};
use gpui::prelude::*;
use gpui::{App, Div, FontWeight, PathBuilder, SharedString, Window, canvas, div, point, px, rgb};
use pm_design::{Elevation, Scheme, Tokens};
use std::time::Instant;

const MORPH: f32 = 0.12;
const INSET: f32 = 2.;
const STROKE: f32 = 1.5;
const LINE_WIDTH: f32 = 2.;

#[derive(Clone, Copy, Default)]
pub(super) struct Cue {
    goal: Option<(Rect, bool)>,
    from: Option<(Rect, Instant)>,
}

impl Cue {
    pub(super) fn update(&mut self, goal: Option<(Rect, bool)>, animate: bool, t: &Tokens) {
        if self.goal == goal {
            return;
        }
        self.from =
            if animate && goal.is_some_and(|(_, tab)| !tab) && !t.accessibility.reduce_motion {
                self.shown(t)
                    .filter(|(_, tab, _)| !tab)
                    .map(|(r, _, _)| (r, drag::now()))
            } else {
                None
            };
        self.goal = goal;
    }

    fn shown(self, t: &Tokens) -> Option<(Rect, bool, bool)> {
        let (mut to, tab) = self.goal?;
        let mut moving = false;
        if let Some((from, since)) = self.from {
            let p = drag::progress(since, MORPH, t);
            let mix = |a, b| a + (b - a) * p;
            to = Rect {
                x: mix(from.x, to.x),
                y: mix(from.y, to.y),
                w: mix(from.w, to.w),
                h: mix(from.h, to.h),
            };
            moving = p < 1.;
        }
        Some((to, tab, moving))
    }
}

/// The marker is based on the drawn strip, including its active tab and
/// overflow, not on a speculative strip with the lifted tab inserted.
pub(super) fn tab_line(target: DropTarget, zones: &Geometry, t: &Tokens) -> Option<Rect> {
    let DropTarget::Tab { dock, group, index } = target else {
        return None;
    };
    let p = zones
        .panels
        .iter()
        .find(|p| (p.dock, p.group) == (dock, group))?;
    let header = zones.header_rect(p, t);
    let x = zones
        .tabs
        .get(&(dock, group))
        .and_then(|spans| {
            let before = spans.iter().rfind(|(i, ..)| *i < index);
            let after = spans.iter().find(|(i, ..)| *i >= index);
            match (before, after) {
                (Some((_, _, r)), Some((_, l, _))) => Some((r + l) / 2.),
                (Some((_, _, r)), None) => Some(r + TAB_SPACING / 2.),
                (None, Some((_, l, _))) => Some(l - TAB_SPACING / 2.),
                (None, None) => None,
            }
        })
        .unwrap_or(header.x + INSET);
    let half = LINE_WIDTH / 2.;
    let left = header.x + INSET;
    let right = (header.x + header.w - INSET - LINE_WIDTH).max(left);
    let height = t.metrics.hit.min(header.h);
    let line = Rect {
        x: (x - half).clamp(left, right),
        y: header.y + (header.h - height) / 2. + INSET,
        w: LINE_WIDTH,
        h: (height - 2. * INSET).max(0.),
    };
    Some(
        zones
            .side_docks
            .get(&dock)
            .map_or(line, |dock| line.intersection(*dock)),
    )
}

pub(super) fn slot(rect: Rect) -> Rect {
    Rect {
        x: rect.x + INSET,
        y: rect.y + INSET,
        w: (rect.w - 2. * INSET).max(0.),
        h: (rect.h - 2. * INSET).max(0.),
    }
}

#[derive(Clone)]
pub(super) struct Chip {
    label: SharedString,
    width: f32,
}

impl Chip {
    pub(super) fn new(label: String, t: &Tokens, w: &Window) -> Self {
        let m = &t.metrics;
        let width = title_width(&label, t.typography.title, w) + 3. * m.gap + m.symbol_inline + 4.;
        Self {
            label: label.into(),
            width,
        }
    }
}

// Use GPUI primitives: a cold SF Symbol would rasterize synchronously on lift.
fn chip_icon(size: f32, color: u32, rejected: bool) -> Div {
    let icon = div().relative().size(px(size)).flex_none();
    let stroke = tint(color, 1.);
    if rejected {
        let pad = (size * 0.18).round();
        icon.child(
            div()
                .absolute()
                .top(px(pad))
                .bottom(px(pad))
                .left(px(pad))
                .right(px(pad))
                .rounded_full()
                .border(px(1.))
                .border_color(stroke)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, (), w, _| {
                            let mut path = PathBuilder::stroke(px(1.));
                            path.move_to(
                                bounds.origin
                                    + point(bounds.size.width * 0.2, bounds.size.height * 0.2),
                            );
                            path.line_to(
                                bounds.origin
                                    + point(bounds.size.width * 0.8, bounds.size.height * 0.8),
                            );
                            if let Ok(path) = path.build() {
                                w.paint_path(path, stroke);
                            }
                        },
                    )
                    .absolute()
                    .inset_0(),
                ),
        )
    } else {
        let card = |x, y| {
            div()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(size * 0.57))
                .h(px(size * 0.45))
                .rounded(px(1.5))
                .border(px(1.))
                .border_color(stroke)
        };
        icon.child(card(size * 0.13, size * 0.16))
            .child(card(size * 0.30, size * 0.34).bg(tint(color, 0.18)))
    }
}

impl Editor {
    pub(super) fn drag_overlay(&mut self, w: &mut Window, cx: &App) -> Option<Div> {
        if !self.dragging_panel() && self.settle.is_none() {
            return None;
        }
        let t = &self.tokens;
        let m = &t.metrics;
        let c = &t.colors;
        let mut overlay = div().absolute().top_0().left_0().size_full();
        let mut unprepared = None;
        if let Some(Gesture::Panel(d)) = &self.gesture
            && d.moved
            && d.chip.is_none()
        {
            unprepared = Some(Chip::new(
                self.moving_name(&d.original, &d.id, d.stack, &self.panels(cx)),
                t,
                w,
            ));
        }
        let (chip, at, alpha, rejected) = if let Some(Gesture::Panel(d)) = &mut self.gesture
            && d.moved
        {
            if d.chip.is_none() {
                d.chip = unprepared;
            }
            if let Some((r, tab, moving)) = d.cue.shown(t) {
                if moving {
                    w.request_animation_frame();
                }
                if r.w > 0. && r.h > 0. {
                    overlay = overlay.child(
                        positioned(r)
                            .id("drop-preview")
                            .when(tab, |d| d.rounded(px(LINE_WIDTH / 2.)).bg(rgb(c.accent)))
                            .when(!tab, |d| {
                                let fill = if t.scheme == Scheme::Dark {
                                    tint(pm_design::color::mix(c.floating, c.accent, 0.10), 0.28)
                                } else {
                                    tint(c.accent, 0.08)
                                };
                                d.rounded(px(m.panel_radius))
                                    .border(px(STROKE))
                                    .border_color(rgb(c.accent))
                                    .bg(fill)
                            }),
                    );
                }
            }
            let alpha = drag::progress(d.lifted.unwrap_or_else(drag::now), drag::LIFT, t);
            if alpha < 1. {
                w.request_animation_frame();
            }
            let o = drag::CHIP_OFFSET;
            (
                d.chip.clone()?,
                (f32::from(d.at.x) + o, f32::from(d.at.y) + o),
                alpha,
                d.preview.is_none(),
            )
        } else {
            let s = self.settle.as_ref()?;
            let p = drag::progress(s.since, drag::SETTLE, t);
            if p >= 1. {
                self.settle = None;
                return None;
            }
            w.request_animation_frame();
            (
                s.chip.clone(),
                (
                    s.from.0 + (s.to.0 - s.from.0) * p,
                    s.from.1 + (s.to.1 - s.from.1) * p,
                ),
                1. - p,
                false,
            )
        };
        let viewport = w.viewport_size();
        let window = (f32::from(viewport.width), f32::from(viewport.height));
        let width = chip.width.min((window.0 - 2. * drag::CHIP_MARGIN).max(0.));
        let (x, y) = drag::chip_origin(at, (width, m.hit), window);
        Some(
            overlay.child(
                positioned(Rect {
                    x,
                    y,
                    w: width,
                    h: m.hit,
                })
                .id("drag-ghost")
                .px(px(m.gap + 2.))
                .gap(px(m.gap))
                .flex()
                .items_center()
                .rounded(px(m.control_radius))
                .bg(rgb(c.floating))
                .elevated(t, Elevation::Menu)
                .opacity(alpha * if rejected { 0.78 } else { 1. })
                .font_family(pm_design::TEXT_FONT)
                .text_color(rgb(if rejected { c.muted } else { c.text }))
                .text_size(px(t.typography.title))
                .font_weight(FontWeight(t.typography.semibold_weight))
                .child(chip_icon(m.symbol_inline, c.muted, rejected))
                .child(div().min_w(px(0.)).truncate().child(chip.label)),
            ),
        )
    }
}
