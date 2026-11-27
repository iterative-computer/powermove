use super::{Editor, Gesture, TAB_SPACING, positioned, tint, title_width};
use crate::{
    drag,
    kit::Recipes as _,
    layout::{DropTarget, Geometry, Rect},
};
use gpui::prelude::*;
use gpui::{App, Div, FontWeight, PathBuilder, SharedString, Window, canvas, div, point, px, rgb};
use pm_design::{Elevation, Scheme, Tokens};

const INSET: f32 = 2.;
const STROKE: f32 = 1.5;
const LINE_WIDTH: f32 = 2.;

/// The marker uses the tab positions in the current header.
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
        if !self.dragging_panel() {
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
        let Some(Gesture::Panel(d)) = &mut self.gesture else {
            return None;
        };
        if d.chip.is_none() {
            d.chip = unprepared;
        }
        if let Some(preview) = d.preview {
            let tab = d.live && matches!(preview.target, DropTarget::Tab { .. });
            let r = if tab { preview.rect } else { slot(preview.rect) };
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
        let chip = d.chip.clone()?;
        let o = drag::CHIP_OFFSET;
        let at = (f32::from(d.at.x) + o, f32::from(d.at.y) + o);
        let rejected = d.preview.is_none();
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
