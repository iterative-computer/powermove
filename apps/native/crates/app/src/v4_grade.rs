//! The grading controls a panel shows (kit `color-wheel` and `curve-editor`):
//! drawn by `pm_shell::grade`, driven by the pointer through the panel's
//! grouped binding, one kernel gesture per drag (Escape cancels it, as a
//! number field's scrub).
use super::{ScrubDrag, ScrubOp, SurfaceView};
use crate::v4_compile::Kit;
use gpui::{
    Bounds, Context, IntoElement, MouseDownEvent, MouseMoveEvent, Pixels, Point, Styled, Window,
    canvas, point, px,
};
use pm_shell::grade::{self, Curve, Graph, Look};
use pm_surface::Id;
use serde_json::json;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// Where each grading control was last drawn, for the next press.
pub(super) type Drawn = Rc<RefCell<HashMap<Id, Bounds<Pixels>>>>;

/// A drag on a grading control.
pub(super) enum GradeDrag {
    Wheel {
        /// The puck when the press began, and how far it travels per point.
        from: (f64, f64),
        reach: f32,
        /// The channels when the press began: their brightness is kept.
        keep: [f64; 3],
        neutral: f64,
        range: [f64; 2],
        /// What the wheel shows now (ahead of the kernel's answer).
        live: [f64; 3],
    },
    Curve {
        graph: Graph,
        /// The parameters when the press began (slots past the points keep
        /// what they held).
        rest: Vec<f64>,
        original: Curve,
        point: usize,
        /// The point's value where the press began.
        from: (f64, f64),
        /// Where the pointer pressed.
        at: Point<Pixels>,
        /// The curve as it stands now, and whether the point is off the
        /// graph (it goes on release).
        live: Curve,
        leaving: bool,
    },
}

/// The graph inside a curve editor's bounds: inset so points on its edges
/// show whole.
pub(super) fn graph(bounds: Bounds<Pixels>, t: &pm_design::Tokens) -> Graph {
    let inset = px(t.metrics.keyframe / 2. + t.metrics.border);
    Graph::new(Bounds::new(
        bounds.origin + point(inset, inset),
        gpui::size(
            (bounds.size.width - inset * 2.).max(px(1.)),
            (bounds.size.height - inset * 2.).max(px(1.)),
        ),
    ))
}

impl SurfaceView {
    /// The drawing of a grading control, showing a drag ahead of the kernel.
    pub(super) fn grade_canvas(
        &self,
        id: Id,
        kit: &Kit,
        disabled: bool,
    ) -> Option<impl IntoElement> {
        let tokens = self.table.tokens.clone();
        let drawn = self.grade_drawn.clone();
        let drag = self
            .scrub
            .as_ref()
            .filter(|d| d.id == id)
            .and_then(|d| d.grade.as_ref());
        let look = Look {
            disabled,
            active: drag.is_some(),
        };
        let paint: Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut gpui::App)> = match (kit, drag) {
            (Kit::Wheel { rgb, neutral, .. }, drag) => {
                let rgb = match drag {
                    Some(GradeDrag::Wheel { live, .. }) => *live,
                    _ => *rgb,
                };
                let puck = grade::puck(rgb, *neutral);
                Box::new(move |b, window, cx| {
                    grade::paint_wheel(b, puck, look, &tokens, window, cx)
                })
            }
            (Kit::Curve { values, channel }, drag) => {
                let (curve, held, leaving) = match drag {
                    Some(GradeDrag::Curve {
                        live,
                        point,
                        leaving,
                        ..
                    }) => (live.clone(), Some(*point), *leaving),
                    _ => (Curve::from_values(values), None, false),
                };
                let channel = *channel;
                Box::new(move |b, window, _| {
                    let g = graph(b, &tokens);
                    grade::paint_curve(g, &curve, channel, held, leaving, look, &tokens, window)
                })
            }
            _ => return None,
        };
        Some(
            canvas(
                move |bounds, _, _| {
                    drawn.borrow_mut().insert(id, bounds);
                },
                move |bounds, _, window, cx| paint(bounds, window, cx),
            )
            .absolute()
            .inset_0(),
        )
    }

    /// A press on a grading control: a wheel's puck moves from where it is;
    /// a curve's point under the pointer moves, or a press on the graph adds
    /// one there. A double-click returns the control to neutral.
    pub(super) fn grade_press(&mut self, id: Id, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(node) = self.prepared.node(id) else {
            return;
        };
        let Some(binding) = node.binding.clone().filter(|_| !node.local_change) else {
            return;
        };
        if node.disabled {
            return;
        }
        let Some(bounds) = self.grade_drawn.borrow().get(&id).copied() else {
            return;
        };
        let t = self.table.tokens.clone();
        let kit = node.kit.clone();
        if event.click_count == 2 {
            let neutral = match &kit {
                Kit::Wheel { neutral, .. } => json!([neutral, neutral, neutral]),
                Kit::Curve { values, .. } => json!(
                    Curve {
                        points: vec![(0., 0.), (1., 1.)]
                    }
                    .values(values)
                ),
                _ => return,
            };
            self.scrub = None;
            self.scrub_ops.push_back(ScrubOp::Begin);
            self.scrub_ops.push_back(ScrubOp::Value(binding, neutral));
            self.scrub_ops.push_back(ScrubOp::Finish(true));
            self.scrub_pump(cx);
            return;
        }
        let drag = match kit {
            Kit::Wheel {
                rgb,
                neutral,
                range,
            } => {
                let d = t
                    .metrics
                    .grade_wheel
                    .min(f32::from(bounds.size.width))
                    .min(f32::from(bounds.size.height));
                GradeDrag::Wheel {
                    from: grade::puck(rgb, neutral),
                    reach: (d / 2. - t.metrics.keyframe / 2.).max(1.),
                    keep: rgb,
                    neutral,
                    range,
                    live: rgb,
                }
            }
            Kit::Curve { values, .. } => {
                let g = graph(bounds, &t);
                let original = Curve::from_values(&values);
                let hit = t.metrics.hit / 2.;
                let nearest = original
                    .points
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let at = g.to_point((p.0.clamp(0., 1.), p.1.clamp(0., 1.)));
                        (
                            i,
                            f32::from((at - event.position).magnitude() as f32 * px(1.)),
                        )
                    })
                    .filter(|(_, d)| *d <= hit)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(i, _)| i);
                let (mut live, point, added) = match nearest {
                    Some(i) => (original.clone(), i, false),
                    None if original.points.len() < grade::CURVE_POINTS
                        && g.area.contains(&event.position) =>
                    {
                        let (x, y) = g.to_value(event.position);
                        let mut c = original.clone();
                        c.points.push((x.clamp(0., 1.), y.clamp(0., 1.)));
                        let i = c.points.len() - 1;
                        (c, i, true)
                    }
                    None => return,
                };
                let from = live.points[point];
                if added {
                    live.points[point] = (precise(from.0), precise(from.1));
                }
                let drag = GradeDrag::Curve {
                    graph: g,
                    rest: values.clone(),
                    original: if added { live.clone() } else { original },
                    point,
                    from: live.points[point],
                    at: event.position,
                    live,
                    leaving: false,
                };
                if added {
                    // The new point is the press's own edit: it is made now.
                    let GradeDrag::Curve { live, rest, .. } = &drag else {
                        unreachable!()
                    };
                    self.scrub_ops.push_back(ScrubOp::Begin);
                    self.scrub_ops
                        .push_back(ScrubOp::Value(binding.clone(), json!(live.values(rest))));
                    self.scrub = Some(ScrubDrag {
                        id,
                        start: event.position,
                        number: Default::default(),
                        binding: Some(binding),
                        active: true,
                        grade: Some(drag),
                    });
                    self.scrub_pump(cx);
                    cx.notify();
                    return;
                }
                drag
            }
            _ => return,
        };
        self.scrub = Some(ScrubDrag {
            id,
            start: event.position,
            number: Default::default(),
            binding: Some(binding),
            active: false,
            grade: Some(drag),
        });
    }

    /// The pointer moved during a press on a grading control.
    pub(super) fn grade_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let threshold = self.table.tokens.metrics.drag_threshold;
        let off = self.table.tokens.metrics.hit;
        let Some(drag) = &mut self.scrub else {
            return;
        };
        let delta = event.position - drag.start;
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if !drag.active {
            if dx.hypot(dy) < threshold {
                return;
            }
            drag.active = true;
            self.scrub_ops.push_back(ScrubOp::Begin);
        }
        let fine = if event.modifiers.alt { 0.1 } else { 1. };
        let Some(binding) = drag.binding.clone() else {
            return;
        };
        let value = match drag.grade.as_mut() {
            Some(GradeDrag::Wheel {
                from,
                reach,
                keep,
                neutral,
                range,
                live,
            }) => {
                let puck = grade::unit((
                    from.0 + (dx / *reach) as f64 * fine,
                    from.1 - (dy / *reach) as f64 * fine,
                ));
                *live = grade::channels(puck, *keep, *neutral, *range);
                json!(live)
            }
            Some(GradeDrag::Curve {
                graph,
                rest,
                original,
                point,
                from,
                at,
                live,
                leaving,
            }) => {
                let a = graph.area;
                let outside = event.position.x < a.left() - px(off)
                    || event.position.x > a.right() + px(off)
                    || event.position.y < a.top() - px(off)
                    || event.position.y > a.bottom() + px(off);
                *leaving = outside && original.points.len() > 2;
                let mut next = original.clone();
                if *leaving {
                    next.points.remove(*point);
                } else {
                    let here = graph.to_value(event.position);
                    let pressed = graph.to_value(*at);
                    let x = from.0 + (here.0 - pressed.0) * fine;
                    let y = from.1 + (here.1 - pressed.1) * fine;
                    // A point keeps its place among its neighbours.
                    let mine = original.points[*point].0;
                    let others = original
                        .points
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i != point)
                        .map(|(_, p)| p.0);
                    let lo = others.clone().filter(|o| *o <= mine).fold(0f64, f64::max);
                    let hi = others.filter(|o| *o > mine).fold(1f64, f64::min);
                    let gap = 0.002;
                    let lo = if original.points.iter().any(|p| p.0 < mine) {
                        lo + gap
                    } else {
                        0.
                    };
                    let hi = if original.points.iter().any(|p| p.0 > mine) {
                        hi - gap
                    } else {
                        1.
                    };
                    next.points[*point] =
                        (precise(x.clamp(lo, hi.max(lo))), precise(y.clamp(0., 1.)));
                }
                *live = next;
                json!(live.values(rest))
            }
            None => return,
        };
        self.scrub_ops.push_back(ScrubOp::Value(binding, value));
        self.scrub_pump(cx);
        cx.notify();
    }
}

fn precise(v: f64) -> f64 {
    (v * 1000.).round() / 1000.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn the_graph_is_inset_by_half_a_point() {
        let t = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
        let b = Bounds::new(point(px(10.), px(20.)), gpui::size(px(200.), px(100.)));
        let g = graph(b, &t);
        let inset = t.metrics.keyframe / 2. + t.metrics.border;
        assert_eq!(
            g.to_point((0., 1.)),
            point(px(10. + inset), px(20. + inset))
        );
        let back = g.to_value(g.to_point((0.25, 0.75)));
        assert!((back.0 - 0.25).abs() < 1e-6 && (back.1 - 0.75).abs() < 1e-6);
    }
}
